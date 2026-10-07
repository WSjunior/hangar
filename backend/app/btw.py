"""Pergunta lateral numa sessão Claude, com terminal (`/btw`) ou sem ele (fork da conversa).

Com terminal: o `/btw` do Claude Code responde num overlay da TUI e não grava a resposta em lugar
nenhum (só a pergunta vai pro `history.jsonl`). O que a TUI oferece é a tecla `c`, que copia a
resposta INTEIRA por OSC 52 — e o tmux guarda isso num buffer. É daí que a resposta sai:
sem corte na largura da janela e sem rolar o overlay. O overlay aceita o `/btw` no meio de um
turno, e o Esc que o fecha não interrompe o turno principal.

Sem terminal não há overlay, e a CLI recusa o comando (`/btw isn't available in this
environment`). Lá a pergunta vira um fork descartável da conversa — ver `perguntar_sem_terminal`.
"""

import json
import logging
import os
import subprocess
import threading
import time
import uuid
from pathlib import Path

from app import tmux
from app.adapters import CLAUDE_HEADLESS, get_adapter
from app.adapters.claude_headless import sessions as hl_sessions
from app.config import settings
from app.pqueue import _sanitize
from app.terminal_input import (_SETTLE, _esvaziar_composer_claude, _pane_tail, _send_lock,
                                _texto_composer_claude)

_log = logging.getLogger(__name__)

# Rodapé do overlay: `· Answering…` enquanto responde; `c to copy` só quando a resposta terminou.
_PRONTO = "c to copy"
_ABERTO = "Esc to close"
_COPIADO = "Copied to clipboard"
_POLL = 0.3
_PRAZO_ABRIR = 6.0
_PRAZO_COPIA = 3.0
_RODAPE_LINHAS = 3
MAX_HISTORICO = 50
# Buffers do tmux são do SERVIDOR, não do pane: dois `c` em sessões diferentes na mesma janela
# trocariam as respostas. O `_send_lock` é por sessão, então o trecho "c → achar → apagar" tem
# trava própria, global.
_COPIA_LOCK = threading.Lock()


class BtwError(RuntimeError):
    def __init__(self, status: int, code: str, detail: str):
        super().__init__(detail)
        self.status = status
        self.code = code
        self.detail = detail


def _rodape(name: str) -> str:
    return _pane_tail(tmux.capture_pane(name, lines=60), _RODAPE_LINHAS)


def _close_overlay(name: str) -> None:
    if not tmux.send_keys(name, "Escape"):
        raise BtwError(502, "erro_btw_overlay_aberto", "não consegui fechar o /btw no terminal")
    deadline = time.monotonic() + _PRAZO_ABRIR
    while True:
        time.sleep(_POLL)
        pane = tmux.capture_pane(name, lines=60)
        if pane.strip() and _ABERTO not in _pane_tail(pane, _RODAPE_LINHAS):
            return
        if time.monotonic() >= deadline:
            # Não repetir Esc: o diálogo pode já ter fechado e a tecla interromperia o turno.
            raise BtwError(502, "erro_btw_overlay_aberto", "não consegui confirmar o fechamento do /btw")


def _limpar_composer_as_cegas(name: str) -> None:
    """Às cegas: com a tela desalinhada a leitura não vê o resto, e o próximo envio normal sairia
    grudado nele. C-u num composer vazio não faz nada."""
    for _ in range(3):
        if not tmux.send_keys(name, "C-u"):
            _log.warning("btw de %r: o C-u da limpeza não chegou ao terminal; pode ter sobrado texto "
                         "no composer", name)


# No tmux os buffers são do servidor e `-t` é flag desconhecida nos comandos de buffer; no psmux são
# da SESSÃO, e sem `-t` o comando fala com outra. None = ainda não medido: decide o código de retorno.
_BUFFER_COM_ALVO: bool | None = None


def _buffer_cmd(name: str, sub: str, *args: str):
    global _BUFFER_COM_ALVO
    if _BUFFER_COM_ALVO is not False:
        cp = tmux._run(["tmux", sub, "-t", f"={name}", *args])
        if _BUFFER_COM_ALVO or cp.returncode == 0:
            _BUFFER_COM_ALVO = True
            return cp
        # Só a recusa da FLAG desliga o `-t`, e pra sempre no processo. Qualquer outro rc≠0 (sessão
        # que caiu no meio, multiplexador fora) é falha desta chamada: gravar False aqui faria todo
        # buffer seguinte falar com a sessão padrão do psmux — lendo e apagando o clipboard de outra.
        stderr = cp.stderr if isinstance(cp.stderr, str) else ""
        if "unknown flag" not in stderr and "unknown option" not in stderr:
            return cp
        _BUFFER_COM_ALVO = False
    return tmux._run(["tmux", sub, *args])


def _buffers(name: str) -> list[str]:
    cp = _buffer_cmd(name, "list-buffers", "-F", "#{buffer_name}")
    return [b for b in (cp.stdout or "").split("\n") if b]


def _ler_buffer(name: str, nome: str) -> str:
    # O psmux ignora o `-b` (vazio com rc 0) e nem devolve o nome real no `-F`. Sem `-b` vem o mais
    # recente, que é o do `c` — o `_COPIA_LOCK` garante que ninguém copiou no meio.
    cp = _buffer_cmd(name, "show-buffer", "-b", nome)
    texto = cp.stdout or ""
    if texto.strip():
        return texto
    # Vazio com rc 0 é o psmux ignorando o `-b`. rc≠0 é falha de verdade (buffer sumiu): cair no
    # "mais recente" aqui, no tmux, devolveria o buffer de outra cópia como se fosse a resposta.
    if cp.returncode != 0:
        return ""
    return _buffer_cmd(name, "show-buffer").stdout or ""


def _apagar_buffers(name: str, novos: list[str], quantos_antes: int) -> None:
    for b in novos:
        _buffer_cmd(name, "delete-buffer", "-b", b)
    # psmux (aceitou o `-t`): o `-b` não apagou nada e o OSC 52 vira duas cópias; sem `-b` sai o
    # mais recente. No tmux o `-b` já apagou, e apagar por contagem levaria o buffer de uma cópia
    # manual feita na mesma janela — os buffers lá são do servidor inteiro.
    if _BUFFER_COM_ALVO is not True:
        return
    for _ in range(4):
        if len(_buffers(name)) <= quantos_antes:
            break
        _buffer_cmd(name, "delete-buffer")


def _resposta_do_pane(name: str, pergunta: str) -> str:
    """Plano B (multiplexador que não vira OSC 52 em buffer): o overlay como está na tela."""
    linhas = tmux.capture_pane(name, lines=200).rstrip("\n").split("\n")
    inicio = next((i for i in range(len(linhas) - 1, -1, -1)
                   if linhas[i].strip().startswith("/btw ")), None)
    fim = next((i for i in range(len(linhas) - 1, -1, -1) if _ABERTO in linhas[i]), None)
    if inicio is None or fim is None or fim <= inicio:
        return ""
    return "\n".join(l[6:] if l.startswith("      ") else l.strip() for l in linhas[inicio + 1:fim]).strip()


def perguntar(name: str, pergunta: str, timeout: float = 60.0) -> dict:
    """Digita `/btw <pergunta>`, espera a resposta, copia pelo `c`, fecha o overlay e devolve.

    Segura o `_send_lock` da sessão o tempo todo: um drain da fila digitando no meio do overlay
    cairia dentro dele. O custo é que um envio normal pra ESTA sessão espera a resposta — por
    isso o teto é curto (respostas normais levam segundos) e o overlay que some por fora aborta
    na hora, sem esperar o teto. Roda fora do pool de envio (`_send_thread`) de propósito: um
    worker de lá preso por um minuto derrubaria o envio de todas as sessões.
    Um Enter só — o `/btw` é imediato, e um 2º Enter num overlay já aberto seria tecla dentro dele.
    """
    pergunta = " ".join(pergunta.split())
    if not pergunta:
        raise BtwError(400, "erro_btw_vazia", "pergunta vazia")
    with _send_lock(name):
        # Overlay de uma pergunta anterior ainda fechando: digitar agora perdia a `/` e a TUI
        # desenhava o texto em cima da régua (medido ao vivo, pergunta logo depois de outra).
        fim_espera = time.monotonic() + _PRAZO_ABRIR
        while _ABERTO in _rodape(name):
            if time.monotonic() >= fim_espera:
                # Preso aberto (um /btw feito à mão no terminal): digitar agora cairia dentro dele.
                raise BtwError(409, "erro_btw_overlay_aberto",
                               "há um /btw aberto no terminal da sessão; feche antes de perguntar")
            time.sleep(_POLL)
        # O rascunho é APAGADO, como no envio normal — e o que sobra depois do C-u não é texto
        # digitado, então não é motivo pra recusar: recusar aqui perdia o rascunho E a pergunta.
        # Quem protege a conversa de um "<resíduo>/btw …" submetido como mensagem é a conferência
        # do composer antes do Enter, abaixo.
        _esvaziar_composer_claude(name)
        # Confere o composer ANTES do Enter: no Windows a `/` inicial sumiu e o Enter submeteu
        # "btw <pergunta>" como mensagem da conversa (o overlay nunca abriu). Uma segunda tentativa;
        # errado de novo, apaga e para sem Enter. Vazio ou ilegível seguem, como antes.
        for tentativa in range(2):
            if not tmux.send_keys(name, "/btw " + pergunta, literal=True):
                raise BtwError(502, "erro_btw_nao_digitou", "não consegui digitar o /btw no terminal da sessão")
            time.sleep(_SETTLE)
            digitado = _texto_composer_claude(name)
            if digitado is None:
                # Ilegível não confirma nada: com Enter às cegas a pergunta podia cair na conversa.
                time.sleep(_SETTLE)
                digitado = _texto_composer_claude(name)
                if digitado is None:
                    _log.warning("btw de %r: composer ilegível depois de digitar; parado sem Enter", name)
                    _limpar_composer_as_cegas(name)
                    raise BtwError(502, "erro_btw_composer_ilegivel",
                                   "não consegui conferir o /btw digitado; nada foi enviado pra conversa")
            if digitado == "" or digitado.startswith("/btw"):
                break
            _log.warning("btw de %r: o composer recebeu sem a barra (%d/2); apagando", name, tentativa + 1)
            _esvaziar_composer_claude(name)
        else:
            _limpar_composer_as_cegas(name)
            if "/btw" in digitado:
                # A barra chegou: o que estragou a linha foi o que já estava no composer e não sai
                # com C-u. Dizer "perdeu a /" aqui mandaria o usuário caçar o bug errado.
                raise BtwError(409, "erro_btw_composer_ocupado",
                               "sobrou texto no composer do terminal que não sai com Ctrl-U; "
                               "apague no terminal antes de perguntar")
            raise BtwError(502, "erro_btw_barra_perdida",
                           "o terminal perdeu a / do /btw; nada foi enviado pra conversa")
        if not tmux.send_keys(name, "Enter"):
            # Sem isto o Enter perdido só aparecia 6s depois como "não abriu", culpando o overlay.
            _limpar_composer_as_cegas(name)
            raise BtwError(502, "erro_btw_enter_nao_enviado",
                           "o Enter do /btw não chegou ao terminal; nada foi enviado pra conversa")

        inicio = time.monotonic()
        aberto = False
        while True:
            time.sleep(_POLL)
            rodape = _rodape(name)
            if _ABERTO in rodape:
                aberto = True
                if _PRONTO in rodape:
                    break
            elif aberto:
                # Alguém fechou o overlay por fora (Esc no terminal, interrupt do app). Sem overlay
                # não há resposta pra ler — e um Esc nosso agora cairia no turno principal.
                raise BtwError(409, "erro_btw_fechado", "o /btw foi fechado no terminal antes de eu ler a resposta")
            decorrido = time.monotonic() - inicio
            if not aberto and decorrido > _PRAZO_ABRIR:
                raise BtwError(409, "erro_btw_nao_abriu", "o /btw não abriu no terminal da sessão")
            if decorrido > timeout:
                _close_overlay(name)
                raise BtwError(504, "erro_btw_sem_resposta", "o /btw não respondeu a tempo")

        with _COPIA_LOCK:
            antes = _buffers(name)
            tmux.send_keys(name, "c")
            fim = time.monotonic() + _PRAZO_COPIA
            novos: list[str] = []
            while time.monotonic() < fim:
                time.sleep(_POLL)
                novos = [b for b in _buffers(name) if b not in antes]
                if novos or _COPIADO in _rodape(name):
                    break
            resposta = ""
            if novos:
                resposta = _ler_buffer(name, novos[0])
                _apagar_buffers(name, novos, len(antes))
        if resposta.strip():
            fonte = "buffer"
        else:
            resposta = _resposta_do_pane(name, pergunta)
            fonte = "pane"
            _log.warning("btw de %r: o buffer do OSC 52 não trouxe a resposta; resposta lida do pane", name)
        _close_overlay(name)
    resposta = resposta.rstrip("\n")
    if not resposta:
        raise BtwError(502, "erro_btw_ilegivel", "o /btw respondeu, mas não consegui ler a resposta")
    return {"question": pergunta, "answer": resposta, "fonte": fonte, "ts": time.time()}


# ── Sem terminal: a pergunta é um fork descartável da conversa ────────────────────────────────
# Prazo generoso: aqui o modelo lê o contexto inteiro antes de responder, e a chamada não segura
# lock nenhum da sessão — quem espera é só quem perguntou.
_FORK_PRAZO = 180.0
# O fork é uma PERGUNTA, não um turno de trabalho: nada que aja sai daqui. As de leitura ficam —
# ajudam a responder e não mudam nada. Os dois caminhos mais curtos foram descartados na medição:
# `--permission-mode plan` barra tudo, mas a CLI emenda "quer que eu chame ExitPlanMode?" na
# resposta mesmo mandada calar, e `--disallowed-tools "*"` tira até a leitura — aí o modelo escreve
# chamadas de ferramenta inventadas no meio do texto. `--permission-mode default` NÃO serve de
# guarda: o allow-list da conta vale, e o Bash rodou sem perguntar nada.
# ponytail: lista de nomes, então ferramenta de ação NOVA entra permitida até alguém somar aqui.
_FORK_SEM_FERRAMENTA = ("Bash BashOutput KillShell Edit Write NotebookEdit Task TodoWrite "
                        "WebFetch WebSearch SlashCommand mcp__*")


def _argv_fork(meta: dict, sid: str, pergunta: str) -> list[str]:
    argv = ["claude", "-p", "--resume", meta["session_id"], "--fork-session", "--session-id", sid,
            "--disallowed-tools", _FORK_SEM_FERRAMENTA, "--output-format", "json"]
    if meta.get("model"):
        argv += ["--model", meta["model"]]
    if meta.get("engine"):
        pre = ["hangar-engine", "--exec", meta["engine"]]
        if meta.get("model"):
            pre += ["--model", meta["model"]]
            if meta.get("context_window"):
                pre += ["--context", str(meta["context_window"])]
        argv = pre + ["--"] + argv
    # `--` antes da pergunta: ela é texto de quem usa o app, e o parser da CLI lê o último argumento
    # como OPÇÃO quando ele casa com uma flag (medido: uma pergunta "--permission-mode" sai como
    # `error: option '--permission-mode <mode>' argument missing`). Sem isto dá pra ligar flag pelo
    # texto da pergunta — inclusive as que derrubam o `--disallowed-tools` logo acima.
    return argv + ["--", pergunta]


def perguntar_sem_terminal(name: str, pergunta: str, timeout: float = _FORK_PRAZO) -> dict:
    """Responde no contexto da conversa sem entrar nela, onde não há TUI pra abrir o overlay.

    `--fork-session` é o que preserva a conversa: sem ele, `-p --resume` grava a pergunta e a
    resposta no `.jsonl` da própria sessão e elas aparecem no chat (medido). Com ele, o original
    não muda um byte e o que nasce é outro transcript, apagado no fim — senão ele apareceria na
    lista de conversas pra retomar. O processo é separado do cano, então o turno em voo não é
    tocado; em troca, o fork enxerga o transcript GRAVADO, não o que ainda está sendo escrito.
    """
    pergunta = " ".join(pergunta.split())
    if not pergunta:
        raise BtwError(400, "erro_btw_vazia", "pergunta vazia")
    meta = hl_sessions.load(name)
    if not meta or not meta.get("session_id"):
        raise BtwError(404, "erro_btw_sem_conversa", "esta sessão ainda não tem conversa pra perguntar")
    adapter = get_adapter(CLAUDE_HEADLESS)
    if not Path(adapter.transcript_path_de(meta)).exists():
        # Sessão que nasceu e nunca conversou: o `--resume` falharia com "No conversation found".
        raise BtwError(409, "erro_btw_sem_conversa", "esta sessão ainda não tem conversa pra perguntar")
    sid = str(uuid.uuid4())
    env = dict(os.environ)
    if meta.get("config_dir"):
        env["CLAUDE_CONFIG_DIR"] = meta["config_dir"]
    try:
        cp = subprocess.run(_argv_fork(meta, sid, pergunta), cwd=meta["cwd"], env=env,
                            capture_output=True, text=True, encoding="utf-8", errors="replace",
                            timeout=timeout)
    except subprocess.TimeoutExpired:
        raise BtwError(504, "erro_btw_sem_resposta", "a pergunta lateral não respondeu a tempo")
    except OSError as e:
        _log.warning("btw sem terminal de %r: não consegui rodar o claude (%s)", name, e.__class__.__name__)
        raise BtwError(502, "erro_btw_fork_falhou", "não consegui abrir a pergunta lateral")
    finally:
        try:
            Path(adapter.transcript_path(meta["cwd"], sid, meta.get("config_dir"))).unlink(missing_ok=True)
        except OSError as e:
            # Apagar é limpeza, não o resultado: no Windows um neto do motor ainda com o arquivo
            # aberto recusa o unlink, e deixar subir trocaria o erro real (o timeout) por um 500.
            _log.warning("btw sem terminal de %r: sobrou o transcript do fork (%s)", name, e.__class__.__name__)
    if cp.returncode != 0:
        # Saída do CLI fora do log de serviço: ela transcreve a conversa e pode carregar credencial.
        _log.warning("btw sem terminal de %r: claude saiu com %s", name, cp.returncode)
        raise BtwError(502, "erro_btw_fork_falhou", "a pergunta lateral falhou")
    try:
        d = json.loads(cp.stdout)
    except ValueError:
        d = None
    if isinstance(d, dict) and d.get("is_error"):
        # Falha do lado do modelo (cota, recusa) vem com rc=0 e o motivo no `result`. Chamar isso de
        # "não consegui ler" seria mentira — foi legível, e o que falhou foi a pergunta.
        _log.warning("btw sem terminal de %r: o claude devolveu erro (subtype=%s)", name, d.get("subtype"))
        raise BtwError(502, "erro_btw_fork_falhou", "a pergunta lateral falhou")
    resposta = (d.get("result") or "").strip() if isinstance(d, dict) else ""
    if not resposta:
        raise BtwError(502, "erro_btw_ilegivel", "a pergunta lateral respondeu, mas não consegui ler a resposta")
    return {"question": pergunta, "answer": resposta, "fonte": "fork", "ts": time.time()}


def _arquivo(name: str) -> Path:
    d = Path(settings.projects_dir).parent / ".hangar-btw"
    d.mkdir(parents=True, exist_ok=True)
    return d / f"{_sanitize(name)}.jsonl"


def _linhas(p: Path) -> list[str]:
    if not p.exists():
        return []
    return [l for l in p.read_text(encoding="utf-8").split("\n") if l.strip()]


def registrar(name: str, item: dict) -> None:
    p = _arquivo(name)
    linhas = _linhas(p) + [json.dumps(item, ensure_ascii=False)]
    p.write_text("\n".join(linhas[-MAX_HISTORICO:]) + "\n", encoding="utf-8")


def historico(name: str) -> list[dict]:
    itens = []
    for linha in _linhas(_arquivo(name)):
        try:
            itens.append(json.loads(linha))
        except ValueError:
            continue
    return itens[-MAX_HISTORICO:]

from app.runtime_terminal import wrap_driver as _wrap_terminal_driver
perguntar = _wrap_terminal_driver(perguntar, admin=True)
