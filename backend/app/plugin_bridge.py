"""Caminho NATIVO de entrada nas sessões Claude: o function-hook do plugin `hangar`.

O que muda em relação ao pane: o texto não é digitado. O plugin mantém um
long-poll aberto aqui e entrega por `$.prompt.submit`, a mesma chamada que o
engine faz para um prompt digitado. A fila durável, a poda e o claim continuam
sendo do `pqueue`/`terminal_input.drain` — aqui é só o transporte.

Fallback é por AUSÊNCIA, não por erro: sem um long-poll vivo para a sessão,
`aguardando()` é falso e o `drain` segue pelo caminho de tecla de sempre. Vale
para toda sessão que não é Claude, para a que nasceu antes do flag e para a
que carregou o plugin e ele morreu.

A identidade é o `CP_SESSION_NAME` que o pane já carrega, e o segredo é um
token por sessão cunhado no nascimento — o bearer do app NUNCA entra no
ambiente do pane.
"""
import asyncio
import hashlib
import hmac
import json
import logging
import os
import re
import secrets
import shutil
import subprocess
import threading
import time
from pathlib import Path

from fastapi import APIRouter, Depends, HTTPException, Request
from pydantic import BaseModel, Field

from app import atomico
from app.auth import require_loopback
from app.live_rate import live_rate

_log = logging.getLogger("hangar.plugin_bridge")

plugin_router = APIRouter(prefix="/api/plugin")

# Quanto o backend segura o long-poll antes de responder 204. Curto o bastante
# para a morte da sessão aparecer, longo o bastante para a espera não virar poll.
ESPERA_S = 25.0

_lock = threading.Lock()
_waiters: dict[str, asyncio.Queue] = {}
_loop: asyncio.AbstractEventLoop | None = None

# Dono do long-poll por sessão: (instância, modos declarados, última batida). Um segundo `claude` com
# o mesmo nome ou pane não pode tomar a fila do primeiro.
_donos: dict[str, tuple[str, set[str], float]] = {}


def declared_modes(name: str) -> set[str]:
    with _lock:
        dono = _donos.get(name)
    return set(dono[1]) if dono else set()


# Capacidade do CLI, não versão: o número seria um palpite sobre qual release ganhou a flag, e
# quem derruba a sessão é a flag desconhecida — `claude --plugin-dir` inexistente sai com erro e o
# pane morre no nascimento, com o `tmux new-session` ainda devolvendo 0 (o app reportaria sucesso).
# `--help` custa 0,20 s (medido) e responde direto; o cache evita pagar isso a cada sessão.
_TTL_CAPACIDADE_S = 600.0
_capacidade: tuple[float, bool] | None = None
# Mods ligados por padrão no CLI daqui em diante; a variável do acesso antecipado é ignorada.
MODS_BY_DEFAULT = (2, 1, 287)
PLUGIN_SRC = Path(__file__).resolve().parents[2] / "plugins" / "hangar"
_versao: tuple[float, tuple[int, ...] | None] | None = None


def cli_version() -> tuple[int, ...] | None:
    """`claude --version` como (2, 1, 287); None quando não dá para ler. Em cache, com prazo."""
    global _versao
    if _versao is not None and time.monotonic() - _versao[0] < _TTL_CAPACIDADE_S:
        return _versao[1]
    exe = shutil.which("claude")
    versao = None
    if exe:
        try:
            r = subprocess.run([exe, "--version"], capture_output=True, text=True, timeout=10,
                               encoding="utf-8", errors="replace")
            m = re.match(r"\s*(\d+)\.(\d+)\.(\d+)", r.stdout or "")
            versao = tuple(int(x) for x in m.groups()) if m else None
        except (OSError, subprocess.SubprocessError) as e:
            _log.warning("plugin: `claude --version` falhou: %r", e)
    _versao = (time.monotonic(), versao)
    return versao


def mods_by_default() -> bool:
    versao = cli_version()
    return versao is not None and versao >= MODS_BY_DEFAULT


def aceita_plugin_dir() -> bool:
    """O `claude` desta máquina conhece `--plugin-dir`? Em cache, com prazo."""
    global _capacidade
    if _capacidade is not None and time.monotonic() - _capacidade[0] < _TTL_CAPACIDADE_S:
        return _capacidade[1]
    exe = shutil.which("claude")
    ok = False
    if exe:
        try:
            r = subprocess.run([exe, "--help"], capture_output=True, text=True, timeout=10,
                               encoding="utf-8", errors="replace")
            ok = "--plugin-dir" in (r.stdout or "")
        except (OSError, subprocess.SubprocessError) as e:
            # Sonda que não respondeu vira NÃO: o preço de errar para o não é o caminho de sempre,
            # e o de errar para o sim é sessão que não nasce.
            _log.warning("plugin: `claude --help` falhou: %r", e)
    _capacidade = (time.monotonic(), ok)
    return ok


def esquecer_capacidade() -> None:
    """Descarta as sondas do CLI. Quem acabou de atualizar o `claude` precisa disto."""
    global _capacidade, _versao
    _capacidade = None
    _versao = None


def _ligado_de_verdade() -> bool:
    from app import runtime_config
    if not runtime_config.get("claude_function_hooks"):
        return False
    return mods_by_default() or aceita_plugin_dir()


def ligado() -> bool:
    """O caminho do plugin vale nas sessões Claude desta máquina?

    `claude_function_hooks` é o liga/desliga (nasce ligado). Com ele ligado, vale no CLI com mods por
    padrão (2.1.287+) ou no anterior que aceita `--plugin-dir` com a variável do acesso antecipado."""
    return _ligado_de_verdade()


def raizes_dos_plugins() -> list[str]:
    """`--plugin-dir` sempre, mesmo com o plugin na pasta de skills da conta.

    Só o plugin de `--plugin-dir` fica POR FORA dos instalados pelo marketplace na cadeia de hooks,
    e a faixa dos mods (`ui.ts`) só enxerga o que os plugins abaixo dele desenham. Com o mesmo nome
    nos dois lugares, o CLI carrega só o de `--plugin-dir`."""
    if not ligado():
        return []
    return [str(PLUGIN_SRC)]


def env_da_sessao(name: str) -> dict[str, str]:
    """O que o pane precisa para achar a ponte: endereço e o token DESTA sessão.

    O bearer do app não entra aqui — quem roda dentro do pane não é o app."""
    if not ligado():
        return {}
    from app.config import settings
    return {
        "HANGAR_PLUGIN_URL": f"http://127.0.0.1:{settings.port}/api/plugin",
        "HANGAR_PLUGIN_TOKEN": mint(name),
    }


def mint(name: str) -> str:
    """Token desta sessão, para o `-e` do pane.

    DERIVADO, não sorteado: sorteado ele viveria só na memória do backend, e todo restart deixava
    a sessão viva batendo 403 para sempre — o envio caía no tmux (certo), mas o caminho nativo só
    voltava recriando a sessão (medido em 18/09/2026). O segredo do servidor é estável, então o
    valor se refaz igual depois do restart.

    Não é o bearer do app: é um HMAC dele, de mão única, e é ele que vai para o ambiente do pane.
    Sessão recriada com o MESMO nome recebe o mesmo token, o que é aceitável — quem responde por
    aquele nome é uma sessão só, e a anterior já morreu.
    """
    from app.config import settings
    segredo = (settings.auth_token or "hangar").encode()
    return hmac.new(segredo, f"plugin:{name}".encode(), hashlib.sha256).hexdigest()[:32]


def machine_key() -> str:
    """Chave desta máquina para o `/whoami`: derivada como o `mint`, e só abre aquela rota."""
    from app.config import settings
    segredo = (settings.auth_token or "hangar").encode()
    return hmac.new(segredo, b"plugin:machine", hashlib.sha256).hexdigest()[:32]


def machine_file(home: Path | None = None) -> Path:
    return (home or Path.home()) / ".hangar" / "plugin.json"


def _publish_address(home: Path | None = None) -> None:
    """Onde o plugin acha a ponte: sessão aberta fora do Hangar não recebe `HANGAR_PLUGIN_*`."""
    from app.config import settings
    alvo = machine_file(home)
    alvo.parent.mkdir(parents=True, exist_ok=True)
    tmp = alvo.with_name(alvo.name + ".tmp")
    tmp.write_text(json.dumps({"url": f"http://127.0.0.1:{settings.port}/api/plugin",
                               "chave": machine_key()}), encoding="utf-8")
    try:
        tmp.chmod(0o600)
    except OSError:
        pass
    atomico.substituir(tmp, alvo)


def plugin_dir_file(home: Path | None = None) -> Path:
    return (home or Path.home()) / ".hangar" / "plugin-dir"


def _publish_plugin_dir(home: Path | None = None) -> None:
    """Caminho do plugin para o wrapper do shell, que não sabe onde o repositório mora.

    Sessão aberta no terminal precisa do mesmo `--plugin-dir` das que o backend abre: só pela pasta
    de skills o plugin fica por dentro do marketplace e não enxerga a faixa dos mods. Sem
    `raizes_dos_plugins()` o arquivo sai, porque flag que o CLI não conhece mata a sessão ao nascer."""
    alvo = plugin_dir_file(home)
    raizes = raizes_dos_plugins()
    if not raizes:
        alvo.unlink(missing_ok=True)
        return
    alvo.parent.mkdir(parents=True, exist_ok=True)
    tmp = alvo.with_name(alvo.name + ".tmp")
    # Uma linha, texto puro: quem lê é shell (bash, zsh, fish, PowerShell), sem parser de JSON.
    tmp.write_text(raizes[0] + "\n", encoding="utf-8", newline="\n")
    atomico.substituir(tmp, alvo)


def _publish_for_machine(home: Path | None = None) -> None:
    _publish_address(home)
    _publish_plugin_dir(home)


# O conftest troca `publish_address`; o teste chega na implementação por `_publish_address` e
# `_publish_plugin_dir`.
publish_address = _publish_for_machine


def _socket_do_tmux() -> str | None:
    """Socket do servidor tmux do Hangar, ou None quando não dá para saber (aí não se compara)."""
    # No psmux `#{socket_path}` nunca é igual ao caminho do `TMUX` (medido): a comparação lá
    # recusaria toda sessão.
    if os.name == "nt":
        return None
    from app import tmux
    cp = tmux._run(["tmux", "list-sessions", "-F", "#{socket_path}"])
    if cp.returncode != 0:
        return None
    caminho = next(iter((cp.stdout or "").splitlines()), "").strip()
    return caminho if caminho and "#{" not in caminho else None


def _sessao_do_tmux(tmux_env: str) -> str | None:
    """`TMUX` é `socket,pid,id` no tmux: o id é o da sessão (`$N`), que dá o nome dela."""
    partes = (tmux_env or "").split(",")
    if len(partes) < 3 or not partes[2].strip():
        return None
    from app import tmux
    cp = tmux._run(["tmux", "list-sessions", "-F", "#{session_id} #{session_name}"])
    if cp.returncode != 0:
        return None
    alvo = "$" + partes[2].strip().lstrip("$")
    for linha in (cp.stdout or "").splitlines():
        sid, _, nome = linha.partition(" ")
        if sid == alvo:
            return nome
    return None


_PSMUX_PID = re.compile(r"psmux-(\d+)\b")


def _sessao_do_psmux(pid: str) -> str | None:
    """No psmux cada sessão tem servidor próprio, e o pid no `TMUX` é o dele (sobrevive ao rename)."""
    from app import tmux
    cp = tmux._run(["tmux", "list-sessions", "-F", "#{pid} #{session_name}"])
    if cp.returncode != 0:
        return None
    for linha in (cp.stdout or "").splitlines():
        spid, _, nome = linha.strip().partition(" ")
        if spid == pid:
            return nome
    return None


def esquecer(name: str) -> None:
    with _lock:
        _waiters.pop(name, None)
        _donos.pop(name, None)
        _estados.pop(name, None)
        _perguntas.pop(name, None)
        _batidas.pop(name, None)
        _fechadas.pop(name, None)
        _sugestoes.pop(name, None)
        _bands.pop(name, None)
        _confirmacoes.pop(name, None)
        _preenchido.pop(name, None)
        _pressed.pop(name, None)
        _cliques.pop(name, None)
    _eventos.pop(name, None)
    _band_wakers.pop(name, None)
    _press_wakers.pop(name, None)
    for chave in [c for c in list(_recusas) if c[0] == name]:
        _recusas.pop(chave, None)


def _confere(name: str, token: str) -> None:
    if not secrets.compare_digest(mint(name), token):
        raise HTTPException(403, detail="token do plugin invalido")


def aguardando(name: str) -> bool:
    """Há um long-poll ABERTO para esta sessão agora?

    É o gate do caminho feliz. Falso enquanto o plugin processa uma entrega
    anterior — e isso é de propósito: nesse instante ele não pode receber outra,
    e a fila espera o próximo poll em vez de cair para a tecla no meio do caminho.
    """
    with _lock:
        return name in _waiters


# Como o texto entra na sessão:
#  - `submit`: `$.prompt.submit`, sem tecla nenhuma. O engine carimba a origem
#    `plugin` e ENVOLVE a mensagem numa moldura ("The hangar plugin sent a
#    message: …") que nenhum hook consegue tirar — medido, o engine recusa.
#  - `fill`: `$.prompt.fill` põe o rascunho no composer e o Hangar manda SÓ o
#    Enter pelo tmux. A origem vira a do usuário, sem moldura, e o `send-keys`
#    deixa de digitar o texto — a parte que hoje fatia, espera e às vezes corta.
MODO_PADRAO = "fill"

# `user`: `$.prompt.submit({text, asUser: true})` entra como fala da pessoa, sem tecla nenhuma. Só com
# a sessão parada (no turno a promessa espera o fim), só se o plugin dono declarou o modo (plugin
# velho trataria como `submit` com moldura) e só com texto que o modo não estraga: `@arquivo` não é
# expandido, `!` é modo bash digitado e `/` é menu da TUI.
_MENCAO = re.compile(r"(^|\s)@\S")
PROVA_TRANSCRIPT_S = 10.0
# Sem confirmação nem prova (transcript ilegível ou texto repetido): nem entregue, nem livre p/ tecla.
INCERTO = "incerto"


def choose_mode(name: str, text: str) -> str:
    recente = estado_recente(name)
    if (mods_by_default() and "user" in declared_modes(name) and recente is not None
            and recente[0] == "idle" and not _MENCAO.search(text)
            and not text.lstrip().startswith(("!", "/"))):
        return "user"
    return MODO_PADRAO


def _linhas_do_usuario(jsonl: str | None) -> set[str] | None:
    if not jsonl:
        return None
    from app import pqueue
    return pqueue.committed_user_lines(jsonl)


def _prova_user(aviso: threading.Event, texto: str, jsonl: str | None, antes: set[str] | None):
    """Entrega `user`: vale o que vier primeiro, o aviso do plugin (que só chega depois dos hooks
    do UserPromptSubmit) ou o texto no transcript. Devolve "aviso", True (transcript), False ou
    `INCERTO`."""
    alvo = texto.strip()
    inicio = time.monotonic()
    limite = inicio + CONFIRMA_S + PROVA_TRANSCRIPT_S
    # A conferência é por conjunto: texto que já estava lá não prova esta entrega. Sem a foto de
    # antes, o transcript só vale depois do prazo do aviso, como era.
    repetido = antes is not None and alvo in antes
    transcript_desde = inicio + (CONFIRMA_S if antes is None else 0.0)
    lido: set[str] | None = None
    while True:
        agora = time.monotonic()
        if not repetido and agora >= transcript_desde:
            lido = _linhas_do_usuario(jsonl)
            if lido is not None and alvo in lido:
                return True
        if aviso.wait(max(0.0, min(0.15, limite - agora))):
            return "aviso"
        if time.monotonic() >= limite:
            # Repetido e legível também é incerto: o texto está lá, mas pode ser o de antes.
            if repetido or lido is None:
                return INCERTO
            # O texto pode ter chegado durante a última espera: sem reler, seria digitado de novo.
            lido = _linhas_do_usuario(jsonl)
            if lido is None:
                return INCERTO
            return alvo in lido

# Teto da espera pelo aviso de que o rascunho entrou. Passou disso, o Enter NÃO
# é enviado: apertar Enter num composer que não recebeu o texto submete o que
# estiver lá — ou nada.
CONFIRMA_S = 5.0

_confirmacoes: dict[str, threading.Event] = {}
_preenchido: dict[str, bool] = {}

# Último estado que o plugin anunciou, por sessão: (momento, estado, motivo).
_estados: dict[str, tuple[float, str, str | None]] = {}

# A frase que a TUI propõe depois do turno. Não há evento de DESCARTE — se a pessoa digita por
# cima, nada avisa —, então quem a apaga é o começo do turno seguinte.
_sugestoes: dict[str, str] = {}


def sugestao(name: str) -> str:
    """A sugestão viva desta sessão, ou string vazia."""
    with _lock:
        return _sugestoes.get(name, "")


# Faixa e painéis que os mods desenham: (versão, {"above", "columns", "panes"}, JSON do app). A
# versão é de todas as sessões juntas: o SSE só compara se mudou desde o que já mandou. O JSON é
# feito uma vez por mudança, não uma vez por conexão aberta.
_bands: dict[str, tuple[int, dict, str]] = {}
_band_seq = 0
_VAZIA: dict = {"above": None, "columns": None, "panes": []}
_VAZIA_JSON = json.dumps({"above": None, "panes": []})
# Quem espera algo da sessão: um Event por sessão, trocado a cada aviso, acorda todos de uma vez.
# Guardado com o loop que o criou: Event usado em outro loop levanta RuntimeError.
_band_wakers: dict[str, tuple[asyncio.AbstractEventLoop, asyncio.Event]] = {}
_press_wakers: dict[str, tuple[asyncio.AbstractEventLoop, asyncio.Event]] = {}


def _waker(tabela: dict, name: str) -> asyncio.Event:
    loop = asyncio.get_running_loop()
    guardado = tabela.get(name)
    if guardado is None or guardado[0] is not loop:
        guardado = tabela[name] = (loop, asyncio.Event())
    return guardado[1]


def _acordar_todos(tabela: dict, name: str) -> None:
    guardado = tabela.pop(name, None)
    if guardado is not None:
        guardado[1].set()


async def _esperar_ate(tabela: dict, name: str, achou, timeout: float):
    """Espera `achou()` dar verdadeiro, acordando a cada aviso da tabela, até `timeout`."""
    fim = time.monotonic() + timeout
    while True:
        valor = achou()
        if valor:
            return valor
        resta = fim - time.monotonic()
        if resta <= 0:
            return None
        try:
            await asyncio.wait_for(_waker(tabela, name).wait(), resta)
        except asyncio.TimeoutError:
            return achou() or None


def _guardar_faixa(name: str, above: dict | None, columns: int | None, panes: list[dict]) -> None:
    global _band_seq
    dados = {"above": above, "columns": columns, "panes": panes}
    with _lock:
        # Reenvio igual (o plugin reenvia de tempos em tempos) não vira evento para todo app aberto.
        if name in _bands and _bands[name][1] == dados:
            return
        _band_seq += 1
        _bands[name] = (_band_seq, dados, json.dumps({"above": above, "panes": panes}, ensure_ascii=False))
    _acordar_todos(_band_wakers, name)


def band(name: str) -> tuple[int, dict]:
    """A faixa e os painéis da sessão, como o app recebe, e a versão; versão 0 quando nada foi desenhado."""
    with _lock:
        versao, dados, _ = _bands.get(name, (0, _VAZIA, ""))
    return versao, {"above": dados["above"], "panes": dados["panes"]}


def band_json(name: str) -> str:
    """O mesmo de `band`, já em JSON, para o evento `plugin_ui`."""
    with _lock:
        return _bands[name][2] if name in _bands else _VAZIA_JSON


def band_columns(name: str) -> int | None:
    """Largura da coluna da conversa no pane, em células, como o plugin mediu."""
    with _lock:
        return _bands.get(name, (0, _VAZIA, ""))[1]["columns"]


def transcript_columns(name: str) -> int | None:
    """Onde cortar as linhas do pane para a prévia: só com painel ancorado ao lado da conversa."""
    with _lock:
        dados = _bands.get(name, (0, _VAZIA, ""))[1]
    return dados["columns"] if any(p.get("placement") == "dock" for p in dados["panes"]) else None


async def esperar_faixa(name: str, vista: int, timeout: float) -> int:
    """Dorme até a faixa sair da versão `vista`, ou até `timeout`; devolve a versão atual."""
    await _esperar_ate(_band_wakers, name, lambda: band(name)[0] != vista, timeout)
    return band(name)[0]


async def esperar_sem_painel(name: str, painel: str, timeout: float) -> bool:
    """O painel saiu da sessão (o plugin viu o `ui.close`) dentro de `timeout`?"""
    def fechou():
        return not any(p.get("id") == painel for p in band(name)[1]["panes"])
    return bool(await _esperar_ate(_band_wakers, name, fechou, timeout))


# Cliques que o plugin confirmou: o pedido de clique do app espera por eles.
_pressed: dict[str, list[tuple[float, str, str]]] = {}
# O clique do app em andamento, um por sessão: o que se espera (site, key, prazo), a tentativa, se o
# plugin já casou o press com ela e o efeito (texto copiado, URL aberta). Fica aberto até a resposta
# ir ao app; efeito que chega depois volta para o terminal.
_cliques: dict[str, dict] = {}


async def esperar_press(name: str, site: str, key: str, desde: float, timeout: float) -> bool:
    """O plugin confirmou o press de `key` no site depois de `desde`?"""
    def achou():
        with _lock:
            return any(t >= desde and r == site and k == key for t, r, k in _pressed.get(name, []))
    return bool(await _esperar_ate(_press_wakers, name, achou, timeout))


async def esperar_efeito(name: str, tentativa: str, timeout: float) -> tuple[str | None, str | None]:
    """O que o clique do app fez nesta tentativa: (texto copiado, URL aberta). Volta assim que um
    dos dois chega, ou vazio depois de `timeout`."""
    def achou():
        with _lock:
            clique = _cliques.get(name)
            if not clique or clique["tentativa"] != tentativa:
                return None
            efeito = (clique["copied"], clique["opened"])
        return efeito if any(efeito) else None
    return await _esperar_ate(_press_wakers, name, achou, timeout) or (None, None)


def esperar_clique_do_app(name: str, site: str, key: str, prazo: float) -> str:
    tentativa = secrets.token_hex(8)
    with _lock:
        _cliques[name] = {"site": site, "key": key, "prazo": time.monotonic() + prazo, "tentativa": tentativa,
                          "casado": False, "copied": None, "opened": None}
    return tentativa


def encerrar_clique_do_app(name: str, tentativa: str) -> None:
    with _lock:
        if _cliques.get(name, {}).get("tentativa") == tentativa:
            del _cliques[name]


def _do_app(name: str, site: str, key: str) -> str | None:
    """A tentativa do clique do app que este press é, uma vez só; None para press do terminal."""
    with _lock:
        clique = _cliques.get(name)
        if (not clique or clique["casado"] or (clique["site"], clique["key"]) != (site, key)
                or time.monotonic() > clique["prazo"]):
            return None
        clique["casado"] = True
        return clique["tentativa"]


def _guardar_efeito(campo: str, name: str, tentativa: str, valor: str) -> None:
    with _lock:
        clique = _cliques.get(name)
        if not clique or clique["tentativa"] != tentativa:
            # 409: o plugin segue com o `next` e a ação acontece no terminal, em vez de sumir.
            raise HTTPException(409, detail="clique do app já respondido")
        clique[campo] = valor
    _acordar_todos(_press_wakers, name)


# Trecho da âncora: o terminal corta a linha da faixa com reticências quando o pane é estreito.
_ANCHOR_CHARS = 16


def band_anchor(name: str) -> str | None:
    """O começo do primeiro texto legível da faixa, como ele aparece no pane.

    É por ele que a prévia sabe onde a conversa acaba: a faixa fica entre a última resposta e a
    caixa de digitar, e um mod que começa a linha com ● seria lido como prosa em andamento."""
    tree = band(name)[1]["above"]
    pilha: list = [tree]
    while pilha:
        no = pilha.pop()
        if isinstance(no, str):
            texto = no.strip()
            if sum(c.isalnum() for c in texto) >= 3:
                return texto[:_ANCHOR_CHARS]
        elif isinstance(no, dict):
            pilha.extend(reversed(no.get("children") or []))
            # Botão e link desenham o `label` antes dos filhos: uma faixa que começa por botão
            # começa por ele.
            rotulo = (no.get("props") or {}).get("label")
            if isinstance(rotulo, str):
                pilha.append(rotulo)
    return None

# Depois disso a leitura do pane volta a mandar sozinha. O plugin não repete estado — ele avisa
# transição —, então o prazo cobre uma sessão parada em `idle` por horas: o que expira aqui é a
# CONFIANÇA de que o plugin ainda está vivo, e quem a renova é a batida do long-poll.
VALIDADE_ESTADO_S = 90.0


# Quantos SSE do app (lista ou conversa) estão abertos agora. Pedido de permissão só é segurado
# pelo plugin com alguém no app para responder.
_apps_abertos = 0


def app_entrou() -> None:
    global _apps_abertos
    with _lock:
        _apps_abertos += 1


def app_saiu() -> None:
    global _apps_abertos
    with _lock:
        _apps_abertos = max(0, _apps_abertos - 1)


def app_presente() -> bool:
    with _lock:
        return _apps_abertos > 0


def terminal_preso(name: str) -> bool:
    """Há cliente tmux preso nesta sessão (terminal de verdade ou o painel do app)?

    Na dúvida é SIM: errar para o sim só devolve o diálogo ao terminal, que é o de sempre; errar
    para o não esconderia o pedido de permissão de quem está olhando o terminal."""
    from app import tmux
    try:
        cp = tmux._run(["tmux", "list-clients", "-t", f"={name}", "-F", "#{client_tty}"])
    except Exception:
        return True
    if cp.returncode != 0:
        return True
    return bool((cp.stdout or b"").strip())


# Última batida do long-poll de entrada, por sessão: é o pulso que diz que o plugin está vivo.
_batidas: dict[str, float] = {}
# Um por sessão, criado por quem espera (o monitor de estado). Aviso do plugin acorda o monitor na
# hora, em vez de a transição esperar o próximo tique do pane.
_eventos: dict[str, asyncio.Event] = {}


def vivo(name: str) -> bool:
    """O plugin desta sessão bateu aqui há pouco?"""
    with _lock:
        if name in _waiters:
            return True
        quando = _batidas.get(name)
    return quando is not None and time.monotonic() - quando < ESPERA_S + 10


async def esperar_evento(name: str, timeout: float) -> None:
    """Dorme até o plugin avisar algo desta sessão, ou até `timeout`. Só no loop do servidor."""
    ev = _eventos.setdefault(name, asyncio.Event())
    try:
        await asyncio.wait_for(ev.wait(), timeout)
    except asyncio.TimeoutError:
        pass
    ev.clear()


def _acordar(name: str) -> None:
    ev = _eventos.get(name)
    if ev is not None:
        ev.set()


def estado_recente(name: str) -> tuple[str, str | None] | None:
    """O estado anunciado pelo plugin, se ainda válido. None = o pane que decida."""
    with _lock:
        hit = _estados.get(name)
        vivo = name in _waiters
    if hit is None:
        return None
    quando, estado, motivo = hit
    if not vivo and time.monotonic() - quando > VALIDADE_ESTADO_S:
        return None
    return estado, motivo


def entregar(name: str, texto: str, modo: str = MODO_PADRAO, jsonl: str | None = None):
    """Passa o texto ao long-poll da sessão. True = entregue; False = ninguém entregou (use o pane);
    `INCERTO` = pode ter entrado e não dá para provar (não digite; a reconciliação decide).

    Chamado de dentro do `drain`/`_send_one`, que rodam em thread: o Queue é do
    loop do FastAPI, então a entrega atravessa por `call_soon_threadsafe`.

    Roda inteiro sob o `_send_lock` da sessão: o Enter, a conferência e a limpeza tocam o mesmo
    composer que o `send_prompt`, e a confirmação do rascunho é UMA por sessão — duas entregas
    juntas roubariam o aviso uma da outra.
    """
    from app import terminal_input
    from app.conversation_transfer import transfer_active
    if transfer_active(name):
        return False
    with terminal_input._send_lock(name):
        if transfer_active(name):
            return False
        return _entregar(name, texto, modo, jsonl)


def _entregar(name: str, texto: str, modo: str, jsonl: str | None = None):
    with _lock:
        fila = _waiters.get(name)
        loop = _loop
    if fila is None or loop is None:
        return False
    aviso = threading.Event()
    if modo in ("fill", "user"):
        with _lock:
            _confirmacoes[name] = aviso
            _preenchido.pop(name, None)
    # A foto sai ANTES de o plugin receber o texto: depois, não dá para separar o novo do repetido.
    antes = _linhas_do_usuario(jsonl) if modo == "user" else None
    try:
        loop.call_soon_threadsafe(fila.put_nowait, {"text": texto, "modo": modo})
    except RuntimeError:
        # Loop morrendo (shutdown): não é entrega, e o caller tem o pane.
        return False
    if modo not in ("fill", "user"):
        return True
    if modo == "user":
        prova = _prova_user(aviso, texto, jsonl, antes)
        with _lock:
            ok = _preenchido.pop(name, False)
            _confirmacoes.pop(name, None)
        if prova == "aviso":
            return ok
        if prova is not True:
            _log.warning("plugin %s: envio sem confirmação nem prova no transcript — resultado=%s",
                         name, prova)
        return prova
    confirmou = aviso.wait(CONFIRMA_S)
    with _lock:
        ok = _preenchido.pop(name, False)
        _confirmacoes.pop(name, None)
    if not confirmou:
        _log.warning("plugin %s: rascunho sem confirmação em %.0fs — sem Enter", name, CONFIRMA_S)
    from app import terminal_input, tmux
    if not confirmou or not ok:
        # O rascunho pode ter entrado mesmo sem o aviso chegar. Quem assume daqui é o caminho de
        # tecla, que digitaria EM CIMA do resíduo e concatenaria as duas coisas — o mesmo estrago
        # que o tratamento de envio parcial existe pra evitar. Limpa antes de devolver.
        terminal_input._limpar_composer(name, texto, None)
        return False
    if not tmux.send_keys(name, "Enter"):
        terminal_input._limpar_composer(name, texto, None)
        return False
    # O Enter SAIR não é o mesmo que a mensagem ENTRAR: o composer pode ter perdido o foco, ou um
    # overlay pode ter subido entre o rascunho e a tecla. Sem esta conferência a entrada era marcada
    # como entregue com o texto parado no campo — mensagem perdida do ponto de vista da fila, que é
    # o pior defeito possível aqui. Mesma prova que o caminho de tecla já usava.
    if not terminal_input._submeteu(name, texto):
        _log.warning("plugin %s: Enter não submeteu — devolvendo pro caminho de tecla", name)
        terminal_input._limpar_composer(name, texto, None)
        return False
    return True


def tracked_session_id(name: str) -> str | None:
    """O uuid da conversa que o Hangar acompanha nesta sessão; None quando o vínculo é só palpite."""
    from app import tmux
    from app.api import registry
    from app.procinfo import _proc_children_map
    from app.registry import SessionRegistry
    # cwd do pane do agente, como no `list()`: no Windows a pasta do projeto sai dele, e um split
    # ativo noutra pasta apontaria para outro transcript.
    panes = tmux.list_panes_all().get(name)
    cwd = SessionRegistry._agent_pane(panes, _proc_children_map())["cwd"] if panes else ""
    jsonl, tracked = registry.resolve_tracked(name, cwd)
    return Path(jsonl).stem if jsonl and tracked else None


def _conversation_mismatch(name: str, session_id: str | None) -> str | None:
    """None quando quem chama é a conversa que o Hangar acompanha; senão o motivo da recusa.

    Pane, ambiente herdado e pid do psmux valem para QUALQUER `claude` aberto na sessão (split, janela
    nova); só a conversa separa o dono de um segundo processo. Plugin velho não manda o id e cai na tecla."""
    if not session_id:
        return "uuid-ausente"
    atual = tracked_session_id(name)
    if atual is None:
        return "uuid-desconhecido"
    return None if atual == session_id else "uuid-diferente"


# Última recusa logada por (sessão, instância): o plugin recusado volta a cada 30 s e o log só registra
# a mudança. Por instância porque os pulls aceitos do dono não podem apagar a marca do recusado.
_recusas: dict[tuple[str, str], str] = {}


class PullBody(BaseModel):
    sessao: str
    token: str
    instance: str = ""
    modos: list[str] = []
    # Último estado que o plugin viu: o `idle` da largada sai antes de existir ponte.
    estado: str | None = None
    # `$.session.id()` do plugin: a conversa que ele atende.
    session_id: str | None = None


class StateBody(BaseModel):
    sessao: str
    token: str
    estado: str
    cwd: str | None = None
    model: str | None = None
    motivo: str | None = None
    tool: str | None = None
    origin: str | None = None


class WhoamiBody(BaseModel):
    chave: str
    pane: str | None = None
    nome: str | None = None
    tmux: str | None = None
    session_id: str | None = None


@plugin_router.post("/whoami", dependencies=[Depends(require_loopback)])
async def whoami(body: WhoamiBody):
    """Sessão aberta fora do Hangar descobre nome e token. Só da própria máquina; no psmux resolve o
    pid do servidor da sessão (depois o nome); no tmux, pane único, o id da sessão e o nome sem pane."""
    if not secrets.compare_digest(machine_key(), body.chave):
        raise HTTPException(403, detail="chave do plugin invalida")
    nome, origem = await _whoami(body)
    if nome:
        recusa = await asyncio.to_thread(_conversation_mismatch, nome, body.session_id)
        if recusa:
            _log.info("plugin whoami recusado pane=%s sessao=%s uuid=%s origem=%s",
                      body.pane, nome, body.session_id, recusa)
            return {"sessao": None}
    _log.info("plugin whoami pane=%s sessao=%s origem=%s", body.pane, nome, origem)
    return {"sessao": nome, "token": mint(nome), "origem": origem} if nome else {"sessao": None}


async def _whoami(body: WhoamiBody) -> tuple[str | None, str]:
    # O interruptor desliga o caminho inteiro: sessão nenhuma descobre a ponte com ele desligado.
    if not await asyncio.to_thread(ligado):
        return None, "desligado"
    from app import quem_chama
    psmux = _PSMUX_PID.search((body.tmux or "").split(",")[0])
    if psmux:
        # Pane fica de fora: no psmux todo pane é `%1`, e com uma sessão só ele casaria com a errada.
        nome = await asyncio.to_thread(_sessao_do_psmux, psmux.group(1))
        if nome:
            return nome, "psmux-pid"
        # O plugin sempre manda pane, então o `CP_SESSION_NAME` do wrapper só é ouvido aqui.
        try:
            return await asyncio.to_thread(quem_chama.resolver, {quem_chama.CAB_NOME: body.nome or ""})
        except quem_chama.SessaoDesconhecida:
            return None, "psmux-pid"
    if body.tmux:
        meu = await asyncio.to_thread(_socket_do_tmux)
        if meu and body.tmux.split(",")[0] != meu:
            # Outro servidor tmux: o mesmo pane id lá é outra sessão, não uma do Hangar.
            return None, "outro-socket"
    if body.pane:
        try:
            return await asyncio.to_thread(quem_chama.resolver, {quem_chama.CAB_PANE: body.pane})
        except quem_chama.SessaoDesconhecida:
            return await asyncio.to_thread(_sessao_do_tmux, body.tmux or ""), "tmux"
    try:
        return await asyncio.to_thread(quem_chama.resolver, {quem_chama.CAB_NOME: body.nome or ""})
    except quem_chama.SessaoDesconhecida:
        return None, "nome"


@plugin_router.post("/pull")
async def pull(body: PullBody):
    """Long-poll do plugin. Sempre 200: com o texto, ou `{"text": null}` quando a janela fecha vazia.

    Sem `Depends(require_auth)`: quem chama é o pane, que não tem o bearer do
    app. O token por sessão é a credencial daqui.
    """
    _confere(body.sessao, body.token)
    recusa = await asyncio.to_thread(_conversation_mismatch, body.sessao, body.session_id)
    chave = (body.sessao, body.instance)
    if not recusa:
        _recusas.pop(chave, None)
    elif _recusas.get(chave) != recusa:
        _recusas[chave] = recusa
        _log.info("plugin pull recusado sessao=%s instance=%s uuid=%s origem=%s",
                  body.sessao, body.instance, body.session_id, recusa)
    if recusa:
        # 409 como o de dono: o plugin larga a ponte e tenta de novo depois — após um `/clear` os
        # dois ids voltam a bater e o dono se recupera sozinho.
        raise HTTPException(409, detail=f"conversa nao e a que o Hangar acompanha ({recusa})")
    agora = time.monotonic()
    with _lock:
        dono = _donos.get(body.sessao)
        if dono and dono[0] != body.instance and agora - dono[2] < ESPERA_S + 10:
            raise HTTPException(409, detail="outra instância do plugin já atende esta sessão")
        _donos[body.sessao] = (body.instance, set(body.modos) or {"fill"}, agora)
        # Só semeia: com entrada do `/state`, quem manda é ela.
        if body.estado and body.sessao not in _estados:
            _estados[body.sessao] = (agora, body.estado, None)
    global _loop
    fila: asyncio.Queue = asyncio.Queue()
    with _lock:
        _loop = asyncio.get_running_loop()
        _waiters[body.sessao] = fila
        _batidas[body.sessao] = time.monotonic()
    # `faixa`: o backend tem a faixa dos mods desta sessão? Reiniciado, não tem, e o plugin reenvia.
    try:
        entrega = await asyncio.wait_for(fila.get(), timeout=ESPERA_S)
    except asyncio.TimeoutError:
        return {"text": None, "faixa": body.sessao in _bands}
    finally:
        with _lock:
            # Só renova o próprio dono: um `esquecer` ou outra instância no meio não é desfeito.
            dono = _donos.get(body.sessao)
            if dono and dono[0] == body.instance:
                _donos[body.sessao] = (dono[0], dono[1], time.monotonic())
            _batidas[body.sessao] = time.monotonic()
            if _waiters.get(body.sessao) is fila:
                del _waiters[body.sessao]
    return {**entrega, "faixa": body.sessao in _bands}


class SuggestBody(BaseModel):
    sessao: str
    token: str
    texto: str
    mostrada: bool


@plugin_router.post("/suggest")
async def suggest(body: SuggestBody):
    """A frase que a TUI propõe depois do turno (Tab aceita, no terminal).

    Medição antes de virar recurso: só registra, para responder se ela é regerada
    todo turno e o que acontece quando é descartada."""
    _confere(body.sessao, body.token)
    with _lock:
        # `mostrada=False` é proposta que a TUI não pôs na caixa (diálogo aberto, headless): mostrar
        # no app o que nem o terminal mostrou seria inventar estado.
        _sugestoes[body.sessao] = body.texto if body.mostrada else ""
    return {"ok": True}


# O plugin já corta árvore acima de 256 KB; a folga cobre sessão e token no mesmo corpo.
MAX_BAND_BYTES = 300 * 1024


class BandPane(BaseModel):
    id: str = Field(min_length=1, max_length=64)
    title: str = ""
    placement: str = "inline"
    columns: int | None = None
    tree: dict | None = None


class BandBody(BaseModel):
    sessao: str
    token: str
    above: dict | None = None
    columns: int | None = None
    panes: list[BandPane] = Field(default_factory=list)


@plugin_router.post("/ui")
async def ui(body: BandBody, request: Request):
    """A faixa acima do prompt e os painéis, como o plugin os recebeu de todos os mods, só quando mudam.

    O Hangar não interpreta as árvores: elas vão como vieram para o app, que desenha os elementos do
    Claude Code sem saber de que mod vieram."""
    if int(request.headers.get("content-length") or 0) > MAX_BAND_BYTES:
        raise HTTPException(413, detail="faixa grande demais")
    _confere(body.sessao, body.token)
    _guardar_faixa(body.sessao, body.above, body.columns, [p.model_dump() for p in body.panes])
    return {"ok": True}


class PressBody(BaseModel):
    sessao: str
    token: str
    requestId: str = Field(min_length=1, max_length=64)
    element: str = Field(min_length=1, max_length=256)


@plugin_router.post("/pressed")
async def pressed(body: PressBody):
    """Um botão de mod foi pressionado no terminal: confirma o clique que o app pediu."""
    _confere(body.sessao, body.token)
    with _lock:
        fila = _pressed.setdefault(body.sessao, [])
        fila.append((time.monotonic(), body.requestId, body.element))
        del fila[:-20]
    _acordar_todos(_press_wakers, body.sessao)
    return {"ok": True}


class CopiedBody(BaseModel):
    sessao: str
    token: str
    attempt: str = Field(min_length=1, max_length=64)
    text: str = Field(max_length=65536)


@plugin_router.post("/copied")
async def copied(body: CopiedBody):
    """Um mod copiou um texto num clique do app: vai ao app, que copia no aparelho de quem clicou."""
    _confere(body.sessao, body.token)
    _guardar_efeito("copied", body.sessao, body.attempt, body.text)
    return {"ok": True}


@plugin_router.post("/press-start")
async def press_start(body: PressBody):
    """O press que começou no terminal é o clique que o app pediu? Responde sim uma vez só."""
    _confere(body.sessao, body.token)
    tentativa = _do_app(body.sessao, body.requestId, body.element)
    return {"fromApp": tentativa is not None, "attempt": tentativa}


class OpenedBody(BaseModel):
    sessao: str
    token: str
    attempt: str = Field(min_length=1, max_length=64)
    url: str = Field(max_length=8192)


@plugin_router.post("/opened")
async def opened(body: OpenedBody):
    """Um mod mandou abrir uma URL num clique do app: o app abre no aparelho de quem clicou."""
    _confere(body.sessao, body.token)
    if not re.match(r"^https?://", body.url, re.IGNORECASE):
        raise HTTPException(400, detail="só http(s)")
    _guardar_efeito("opened", body.sessao, body.attempt, body.url)
    return {"ok": True}


# Pergunta de múltipla escolha que o hook de `tool.call` segura: o diálogo do terminal e o app
# correm juntos, e a entrada sobrevive ao intervalo entre dois long-polls (só a fila troca).
_perguntas: dict[str, dict] = {}


def pergunta_pendente(name: str) -> dict | None:
    """A pergunta que o plugin segura agora (`id`, `questions`), ou None.

    Só vale com o long-poll batendo: hook que morreu não pode segurar a resposta do app."""
    with _lock:
        p = _perguntas.get(name)
        if p is None or time.monotonic() - p["visto"] > ESPERA_S + 10:
            return None
        return {"id": p["id"], "questions": p["questions"], "tool": p.get("tool"),
                "resumo": p.get("resumo")}


# Última pergunta fechada por sessão: (id, quem fechou). Resposta repetida (toque duplo, retry do
# cliente) para uma pergunta que o APP já fechou é entrega feita — cair na tecla a mandaria de novo.
_fechadas: dict[str, tuple[str, str]] = {}


def responder_pergunta(name: str, corpo: dict, id: str | None = None) -> bool:
    """Entrega a resposta do app ao hook. False = ele não pegou; quem chama cai na tecla.

    `id` é a pergunta que quem chama leu em `pergunta_pendente`: outra pergunta no lugar não recebe
    esta resposta."""
    with _lock:
        p = _perguntas.get(name)
        loop = _loop
        if p is None or loop is None or (id is not None and p["id"] != id):
            return id is not None and _fechadas.get(name) == (id, "app")
        aviso = p.get("aviso")
        primeiro = aviso is None
        fila = None
        if primeiro:
            aviso = p["aviso"] = threading.Event()
            fila = p.get("fila")
            if fila is None:
                p["resposta"] = corpo
    if fila is not None:
        try:
            loop.call_soon_threadsafe(fila.put_nowait, corpo)
        except RuntimeError:
            return False
    # Segunda resposta com a primeira em voo espera o MESMO aviso: uma pergunta, um veredito.
    pegou = aviso.wait(CONFIRMA_S)
    if primeiro:
        with _lock:
            p = _perguntas.get(name)
            if p is not None and p.get("aviso") is aviso:
                p.pop("aviso", None)
                p.pop("resposta", None)
    return pegou


class AskBody(BaseModel):
    sessao: str
    token: str
    id: str
    questions: list | None = None
    # Pedido de permissão: sem perguntas, só a ferramenta que pede.
    tool: str | None = None
    # Quanto o hook aceita esperar; sem isto vale a janela do long-poll.
    janela_ms: int | None = None
    # O que a ferramenta quer rodar (o comando do Bash, o arquivo do Edit), para o card do app.
    resumo: str | None = None


@plugin_router.post("/ask")
async def ask(body: AskBody):
    """Long-poll do hook do AskUserQuestion: 200 com a resposta do app, ou vazio na janela."""
    _confere(body.sessao, body.token)
    global _loop
    # Permissão só fica com o plugin enquanto há alguém no app E ninguém no terminal: segurar
    # esconde o diálogo do terminal. Reavaliado a cada poll do hook, então prender um terminal no
    # meio da espera devolve o diálogo a ele em poucos segundos.
    if body.id.startswith("perm:") and (
            not app_presente() or await asyncio.to_thread(terminal_preso, body.sessao)):
        with _lock:
            p = _perguntas.get(body.sessao)
            if p is not None and p["id"] == body.id:
                del _perguntas[body.sessao]
        _acordar(body.sessao)
        return {"soltar": True}
    fila: asyncio.Queue = asyncio.Queue()
    with _lock:
        _loop = asyncio.get_running_loop()
        p = _perguntas.get(body.sessao)
        if p is None or p["id"] != body.id:
            p = _perguntas[body.sessao] = {"id": body.id, "questions": body.questions or [],
                                           "tool": body.tool, "resumo": body.resumo}
        p["visto"] = time.monotonic()
        guardada = p.pop("resposta", None)
        if guardada is None:
            p["fila"] = fila
    _acordar(body.sessao)
    if guardada is not None:
        return guardada
    espera = min(ESPERA_S, body.janela_ms / 1000) if body.janela_ms else ESPERA_S
    try:
        return await asyncio.wait_for(fila.get(), timeout=espera)
    except asyncio.TimeoutError:
        return {"answers": None}
    finally:
        with _lock:
            p = _perguntas.get(body.sessao)
            if p is not None and p.get("fila") is fila:
                p.pop("fila", None)
                p["visto"] = time.monotonic()


class AskFimBody(BaseModel):
    sessao: str
    token: str
    id: str
    vencedor: str


@plugin_router.post("/ask-fim")
async def ask_fim(body: AskFimBody):
    """A pergunta fechou. `vencedor=app` é a prova de que a resposta do app valeu."""
    _confere(body.sessao, body.token)
    with _lock:
        p = _perguntas.get(body.sessao)
        if p is None or p["id"] != body.id:
            return {"ok": True}
        del _perguntas[body.sessao]
        _fechadas[body.sessao] = (body.id, body.vencedor)
        fila, aviso = p.get("fila"), p.get("aviso")
    if fila is not None:
        fila.put_nowait({"answers": None})
    if aviso is not None and body.vencedor == "app":
        aviso.set()
    _acordar(body.sessao)
    _log.info("plugin pergunta sessao=%s fechou por %s", body.sessao, body.vencedor)
    return {"ok": True}


class FilledBody(BaseModel):
    sessao: str
    token: str
    ok: bool


@plugin_router.post("/filled")
async def filled(body: FilledBody):
    """O plugin avisa que o rascunho entrou (ou não) no composer.

    É o que libera o Enter: sem esse aviso o Hangar não aperta tecla nenhuma."""
    _confere(body.sessao, body.token)
    with _lock:
        aviso = _confirmacoes.get(body.sessao)
        _preenchido[body.sessao] = body.ok
    if aviso is not None:
        aviso.set()
    return {"ok": True}


class SubmittedBody(BaseModel):
    sessao: str
    token: str
    ok: bool


@plugin_router.post("/submitted", dependencies=[Depends(require_loopback)])
async def submitted(body: SubmittedBody):
    """O plugin avisa se o `$.prompt.submit` do modo `user` foi aceito."""
    _confere(body.sessao, body.token)
    with _lock:
        aviso = _confirmacoes.get(body.sessao)
        _preenchido[body.sessao] = body.ok
    if aviso is not None:
        aviso.set()
    return {"ok": True}


@plugin_router.post("/state")
async def state(body: StateBody, request: Request):
    """Estado por EVENTO, sem `capture-pane`.

    Hoje só registra: quem manda no rótulo continua sendo o monitor de
    `state.py`, que atende os outros provedores também. Ligar as duas fontes é
    passo separado, e ele não pode nascer junto com a troca do caminho de entrada.
    """
    _confere(body.sessao, body.token)
    with _lock:
        _estados[body.sessao] = (time.monotonic(), body.estado, body.motivo)
        if body.estado == "working":
            # Turno novo aposenta a sugestão do anterior — é o que substitui o evento de descarte
            # que o engine não dá.
            _sugestoes.pop(body.sessao, None)
    _acordar(body.sessao)
    _log.debug("plugin estado sessao=%s estado=%s motivo=%s", body.sessao, body.estado, body.motivo)
    return {"ok": True}


class RateBody(BaseModel):
    sessao: str
    token: str
    tokens: int = Field(gt=0, le=1_000_000)
    seconds: float = Field(gt=0, le=3600, allow_inf_nan=False)
    session_id: str


@plugin_router.post("/rate")
async def rate(body: RateBody):
    """Velocidade de uma resposta, medida pelo `turn.step` do plugin no próprio processo."""
    _confere(body.sessao, body.token)
    live_rate(body.sessao).close(body.tokens, body.seconds, body.session_id)
    return {"ok": True}
