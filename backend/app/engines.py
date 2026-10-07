"""Motores de modelo alternativos: trocar o MOTOR sem trocar o CARRO.

A sessão continua no MESMO ~/.claude (skills, hooks, plugins, CLAUDE.md, statusline) e no mesmo
transcript .jsonl que o app já lê. O que muda é um punhado de variáveis de ambiente NO PROCESSO
daquela sessão — nada em disco, nada na conta logada.

Medido em 26/07/2026 (claude 2.1.220), contra Kimi Code e OmniRoute reais:
  - ANTHROPIC_BASE_URL + ANTHROPIC_AUTH_TOKEN bastam: não pede login, não pede aprovação de key.
  - ANTHROPIC_API_KEY (o que os projetos parecidos usam) dispara o prompt "detectei uma API key" e
    grava customApiKeyResponses no ~/.claude.json GLOBAL. Por isso aqui é AUTH_TOKEN, nunca API_KEY.
  - MAX_CONTEXT_TOKENS move a janela; AUTO_COMPACT_WINDOW (o que a doc da Moonshot manda) é inerte.

STDLIB PURA de propósito: o wrapper de shell importa este módulo com o `python3` do sistema, fora do
venv do backend (ver scripts/hangar-engine). Um `from app.config import …` aqui puxaria pydantic e
quebraria o terminal, deixando só o app funcionando. Há teste que trava isso.
"""
import ipaddress
import json
import logging
import os
import re
import tempfile
import threading
from pathlib import Path

# Os helpers compartilhados com o wrapper também precisam ser stdlib-only.
from app import atomico
from typing import Any
from urllib.parse import urlparse

_log = logging.getLogger(__name__)

# Campos aceitos. Qualquer outro é descartado: o cliente não inventa campo (mesma regra do
# runtime_config.EDITAVEIS). Não há campo de "esforço": medido que o provedor pode ignorar o pedido
# do CC (no OmniRoute quem manda é o sufixo do id do modelo), então seria um controle que não controla.
_CAMPOS: dict[str, type] = {
    "label": str,
    "base_url": str,
    "api_key": str,
    "model": str,
    "subagent_model": str,   # -> CLAUDE_CODE_SUBAGENT_MODEL; vazio = mesmo modelo principal
    "context_window": int,   # -> CLAUDE_CODE_MAX_CONTEXT_TOKENS
    "vision": bool,          # informativo; vem do /v1/models do provedor
    "auth_via_api_key": bool,  # provedor que só aceita x-api-key (opencode zen); ver env_de
    # Capacidades do harness. TODAS são positivas ("true = ligado") mesmo quando a env var do Claude
    # Code é negativa (DISABLE_*): misturar as duas polaridades num formulário faz o usuário marcar
    # uma caixa pra desligar algo. env_de() faz a tradução; a UI só mostra capacidade.
    "tool_search": bool,                  # ver env_de
    "bundled_skills": bool,               # ver env_de
    "experimental_betas": bool,           # ver env_de
    "prompt_caching": bool,               # ver env_de
    "adaptive_thinking": bool,            # ver env_de
    "gateway_model_discovery": bool,      # ver env_de
    "fine_grained_tool_streaming": bool,  # ver env_de
    "auto_compact_window": int,           # -> CLAUDE_CODE_AUTO_COMPACT_WINDOW
    "max_output_tokens": int,             # -> CLAUDE_CODE_MAX_OUTPUT_TOKENS
}
_OBRIGATORIOS = ("base_url", "api_key", "model")

# Serializa read-modify-write: dois PUT ao mesmo tempo liam o mesmo estado e o último a gravar
# apagava o motor do outro. Protege só ESTE processo (o backend); o hangar-engine apenas lê.
_LOCK = threading.Lock()

_NOME_OK = re.compile(r"^[a-z0-9][a-z0-9_-]{0,31}$")
_PROIBIDO_NO_VALOR = "\n\r\x00"


def caminho() -> Path:
    # Fixo em ~/.claude: motor é ortogonal a perfil. Derivar de CLAUDE_CONFIG_DIR faria o hangar-engine
    # de um terminal com perfil alternativo ver ZERO motores enquanto o app mostra dois.
    return Path(os.environ.get("CP_ENGINES_FILE") or (Path.home() / ".claude" / "engines.json"))


def _estado() -> tuple[dict[str, Any], bool]:
    """Lê o arquivo bruto. Devolve (dict, corrompido).

    Ausente é o estado NORMAL — ninguém configurou motor ainda — e fica calado (corrompido=False):
    logar aqui incomodaria em todo boot/tick do SSE de quem nunca usou a feature. Presente mas
    ilegível (JSON quebrado, ou não é um objeto) é ANORMAL: é o achado do review — um typo no
    hand-edit quebra o arquivo, listar() volta {} igual a "nunca configurou nada", o usuário re-adiciona
    um motor achando que é a primeira vez, e a gravação por cima apaga os outros motores e suas
    keys, calado. Loga aqui pra não repetir; quem grava (salvar/remover) usa o corrompido=True pra
    recusar a escrita.
    """
    p = caminho()
    try:
        texto = p.read_text(encoding="utf-8")
    except FileNotFoundError:
        return {}, False
    except OSError as e:
        _log.warning("engines.json (%s) não pôde ser lido: %s — tratando como vazio nesta leitura, "
                     "mas recusando sobrescrever até alguém corrigir ou mover o arquivo.", p, e)
        return {}, True
    try:
        d = json.loads(texto)
    except ValueError as e:  # inclui json.JSONDecodeError
        _log.warning("engines.json (%s) existe mas não é JSON válido (%s) — tratando como vazio "
                     "nesta leitura, mas recusando sobrescrever até alguém corrigir ou mover o "
                     "arquivo.", p, e)
        return {}, True
    if not isinstance(d, dict):
        _log.warning("engines.json (%s) não é um objeto JSON — tratando como vazio nesta leitura, "
                     "mas recusando sobrescrever até alguém corrigir ou mover o arquivo.", p)
        return {}, True
    return d, False


def listar() -> dict[str, dict[str, Any]]:
    """Motores gravados, com a api_key INTEIRA. Quem devolve ao cliente mascara na borda HTTP."""
    d, _corrompido = _estado()
    # O nome vira parte de um `$SHELL -c` (registry.py monta `hangar-engine --exec {nome} -- ...`).
    # salvar() já barra nome fora do padrão, mas o arquivo é hand-editável (0600, mas ainda assim);
    # pular o registro corrupto em vez de derrubar a lista inteira — um motor ruim não pode tirar
    # os outros do ar.
    return {nome: v for nome, v in d.items() if isinstance(nome, str) and _NOME_OK.match(nome)}


def arquivo_corrompido() -> bool:
    """True quando engines.json existe mas não pôde ser lido como JSON de motores.

    listar() devolve {} tanto pra isto quanto pra "nunca configurou nada" (de propósito — não pode
    derrubar sessão nem o tick do SSE por um hand-edit ruim). A API usa este sinal à parte pra tela
    parar de dizer "nenhum motor ainda" quando na verdade há um arquivo quebrado escondendo motores
    reais (o item 1 do review).
    """
    return _estado()[1]


def _host_local(host: str) -> bool:
    try:
        ip = ipaddress.ip_address(host)
    except ValueError:
        return host in ("localhost", "localhost.localdomain")
    return ip.is_loopback or ip.is_private


def validar_base_url(url: str) -> str:
    p = urlparse(url)
    if p.scheme not in ("http", "https") or not p.hostname:
        raise ValueError("base_url: use uma URL http(s) completa")
    if p.scheme == "http" and not _host_local(p.hostname):
        # A api_key vai no header Authorization: em http para host público ela atravessa a rede em
        # claro. Loopback/rede privada segue liberado — é o caso do proxy tradutor local.
        raise ValueError("base_url: use https (http só para loopback ou rede privada)")
    limpo = url.rstrip("/")
    # `/v1` no fim é TIRADO: neste app base_url é a raiz do provedor, e quem acrescenta o dialeto é
    # quem consome — `engine_probe` monta `{base}/v1/models`, e o Claude Code monta `{base}/v1/...`
    # a partir de ANTHROPIC_BASE_URL. Colar a URL do jeito que o provedor documenta
    # ("https://api.kimi.com/coding/v1", que é o que está no config.toml do próprio Kimi) gerava
    # `/coding/v1/v1/models`: 404, zero modelo, e nenhuma pista do porquê. Aceitar as duas formas é
    # mais barato que ensinar a diferença em cada campo de URL da tela.
    return limpo[:-3].rstrip("/") if limpo.endswith("/v1") else limpo


def _normalizar(nome: str, dados: dict[str, Any]) -> dict[str, Any]:
    if not _NOME_OK.match(nome or ""):
        raise ValueError("nome: use minúsculas, números, '-' ou '_' (até 32 caracteres)")
    out: dict[str, Any] = {}
    for campo, tipo in _CAMPOS.items():
        if campo not in dados or dados[campo] is None:
            continue
        valor = dados[campo]
        if tipo is bool:
            if not isinstance(valor, bool):
                raise ValueError(f"{campo}: esperado true/false")
            out[campo] = valor
        elif tipo is int:
            if isinstance(valor, bool) or not isinstance(valor, (int, float, str)):
                raise ValueError(f"{campo}: esperado número")
            # Vazio LIMPA, como no ramo de texto logo abaixo: o campo sai do registro. Sem isto o
            # numérico não tinha valor de limpeza nenhum — `""` era "esperado número" e `0`, "deve
            # ser maior que zero" —, e apagar uma janela já gravada era impossível pela tela.
            if isinstance(valor, str) and not valor.strip():
                continue
            try:
                n = int(valor)
            except (TypeError, ValueError):
                raise ValueError(f"{campo}: esperado número") from None
            if n <= 0:
                raise ValueError(f"{campo}: deve ser maior que zero")
            out[campo] = n
        else:
            if not isinstance(valor, str):
                raise ValueError(f"{campo}: esperado texto")
            texto = valor.strip()
            # Uma linha por variável é CONTRATO com o shell: `hangar-engine --env` imprime CHAVE=VALOR e o
            # claude-engine dá export nisso. Um \n aqui exportaria variável arbitraria (PATH, LD_*).
            if any(c in texto for c in _PROIBIDO_NO_VALOR):
                raise ValueError(f"{campo}: sem quebra de linha nem caractere nulo")
            if texto:
                out[campo] = texto
    for campo in _OBRIGATORIOS:
        if not out.get(campo):
            raise ValueError(f"{campo}: obrigatório")
    out["base_url"] = validar_base_url(out["base_url"])
    out.setdefault("label", nome)
    return out


def _atual_ou_recusa() -> dict[str, dict[str, Any]]:
    # Chamado só de dentro do _LOCK por salvar()/remover(). Corrompido = recusa a escrita: perder
    # esta gravação é recuperável (usuário tenta de novo depois de corrigir o arquivo); perder o
    # arquivo inteiro sobrescrito com {} + um registro novo não é (o achado do item 1 do review).
    d, corrompido = _estado()
    if corrompido:
        raise ValueError(
            f"engines.json ({caminho()}) existe mas está corrompido (JSON inválido); corrija-o à "
            "mão ou mova-o antes de salvar/remover — gravar por cima agora apagaria os outros "
            "motores e as keys deles."
        )
    return {nome: v for nome, v in d.items() if isinstance(nome, str) and _NOME_OK.match(nome)}


def salvar(nome: str, dados: dict[str, Any]) -> dict[str, Any]:
    """Grava um motor. Campo desconhecido é descartado; inválido levanta ValueError."""
    registro = _normalizar(nome, dados)
    with _LOCK:
        atual = _atual_ou_recusa()
        atual[nome] = registro
        _gravar(atual)
    return registro


def remover(nome: str) -> bool:
    with _LOCK:
        atual = _atual_ou_recusa()
        if nome not in atual:
            return False
        del atual[nome]
        _gravar(atual)
    return True


def _gravar(tudo: dict[str, Any]) -> None:
    # Escrita atômica (tmp + replace): um corte no meio não deixa JSON pela metade, que na próxima
    # leitura viraria "nenhum motor configurado" — perder a config inteira, calado.
    destino = caminho()
    destino.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=str(destino.parent), suffix=".tmp")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            json.dump(tudo, fh, ensure_ascii=False, indent=2)
        atomico.substituir(tmp, destino)
        try:
            os.chmod(destino, 0o600)
        except OSError:
            # Guarda a key do provedor; falha de chmod não desfaz a gravação (o valor já está lá).
            pass
    except BaseException:
        Path(tmp).unlink(missing_ok=True)
        raise


def _booleano(campo: str, valor: Any) -> bool | None:
    """Lê um campo bool do disco, ou estoura. None = ausente (cai no default de quem chama).

    Os testes de identidade (`is True` / `is False`) são o que dá o default correto para o campo
    AUSENTE, mas contra um arquivo hand-editado eles falham calados: `1 is not True` é verdadeiro,
    então `"tool_search": 1` — escrito por quem viu a convenção "1"/"0" das env vars ao lado — cai no
    ramo DESLIGADO, o oposto do pedido. Sem erro, sem log, e o efeito aparece turnos depois (um 400
    do provedor, ou uma skill estourando a janela). Mesma escolha do _inteiro_positivo: recusar o
    arquivo envenenado em vez de adivinhar a intenção.
    """
    if valor is None:
        return None
    if not isinstance(valor, bool):
        raise ValueError(f"{campo}: esperado true/false (recebido {type(valor).__name__})")
    return valor


def _inteiro_positivo(campo: str, valor: Any) -> int:
    """Normaliza um campo int na hora de virar env var, ou estoura.

    `_normalizar` já rejeita não-numérico e `<= 0` no SAVE, mas o engines.json é hand-editável: um
    valor negativo lá passava direto e virava `CLAUDE_CODE_MAX_CONTEXT_TOKENS=-1000` — o Claude Code
    não valida isso, então a sessão subia com uma janela absurda e a falha aparecia longe da causa.
    Falhar aqui é a mesma escolha que _PROIBIDO_NO_VALOR faz para os campos string: recusar o
    arquivo envenenado em vez de exportar um valor que ninguém mais confere.
    """
    try:
        n = int(valor)
    except (TypeError, ValueError):
        raise ValueError(f"{campo}: esperado número") from None
    if n <= 0:
        raise ValueError(f"{campo}: deve ser maior que zero")
    return n


def catalog_model(model: str) -> str:
    if re.fullmatch(r"gpt-\d[^/]*\[1m\]", model.rsplit("/", 1)[-1]):
        return model.removesuffix("[1m]")
    return model


def service_tier_env(service_tier: str, extra_body: str | None = None) -> dict[str, str]:
    """Escolha por execução, conservando os outros campos do corpo enviado ao motor."""
    if service_tier not in ("default", "priority"):
        raise ValueError("service_tier: use default ou priority")
    try:
        body = json.loads(extra_body) if extra_body is not None else {}
        json.dumps(body, allow_nan=False)
    except (TypeError, ValueError):
        raise ValueError("CLAUDE_CODE_EXTRA_BODY: JSON inválido") from None
    if not isinstance(body, dict):
        raise ValueError("CLAUDE_CODE_EXTRA_BODY: esperado um objeto JSON")
    body["service_tier"] = service_tier
    # O tradutor também aceita speed=fast, que reativaria a prioridade ao desligar.
    if service_tier == "default" and body.get("speed") == "fast":
        del body["speed"]
    return {"CP_ENGINE_SERVICE_TIER": service_tier,
            "CLAUDE_CODE_EXTRA_BODY": json.dumps(body, ensure_ascii=False, separators=(",", ":"))}


def _read_settings(value: str, *, optional: bool = False, cwd: Path | None = None) -> dict:
    try:
        path = Path(value)
        if cwd is not None and not path.is_absolute():
            path = cwd / path
        text = value if value.lstrip().startswith("{") else path.read_text(encoding="utf-8")
        settings = json.loads(text)
    except FileNotFoundError:
        if optional:
            return {}
        raise ValueError("--settings: arquivo não encontrado") from None
    except (OSError, ValueError):
        raise ValueError("settings: arquivo ilegível ou JSON inválido") from None
    if not isinstance(settings, dict) or not isinstance(settings.get("env", {}), dict):
        raise ValueError("settings: esperado um objeto JSON com env como objeto")
    return settings


def _local_settings_root(cwd: Path) -> Path:
    # O Claude usa o local da raiz Git no POSIX, apenas quando a pasta é do usuário.
    if os.name == "nt":
        return cwd
    for root in (cwd, *cwd.parents):
        if not (root / ".git").exists():
            continue
        if root == Path.home().resolve():
            return cwd
        try:
            uid = os.geteuid()
            entries = [root, root / ".git"]
            if (root / ".claude").exists():
                entries.append(root / ".claude")
            return root if all(path.stat().st_uid == uid for path in entries) else cwd
        except OSError:
            return cwd
    return cwd


def service_tier_settings(cmd: list[str], env: dict[str, str], service_tier: str, *,
                          cwd: str | Path | None = None) -> tuple[list[str], dict]:
    """Resolve o mesmo corpo no pré-voo e no lançador, sem gravar nem mudar de pasta."""
    cwd = Path(cwd or Path.cwd()).resolve()
    explicit = {}
    sources = {"user", "project", "local"}
    args = [cmd[0]]
    i = 1
    while i < len(cmd):
        arg = cmd[i]
        if arg == "--":
            args += cmd[i:]
            break
        if arg == "--settings":
            if i + 1 >= len(cmd):
                raise ValueError("--settings precisa de um valor")
            explicit = _read_settings(cmd[i + 1], cwd=cwd)
            i += 2
        elif arg.startswith("--settings="):
            explicit = _read_settings(arg.split("=", 1)[1], cwd=cwd)
            i += 1
        elif arg == "--setting-sources":
            if i + 1 >= len(cmd):
                raise ValueError("--setting-sources precisa de um valor")
            sources = set(cmd[i + 1].split(","))
            args += cmd[i:i + 2]
            i += 2
        elif arg.startswith("--setting-sources="):
            sources = set(arg.split("=", 1)[1].split(","))
            args.append(arg)
            i += 1
        else:
            args.append(arg)
            i += 1
    config = Path(env.get("CLAUDE_CONFIG_DIR") or Path.home() / ".claude")
    project = cwd / ".claude"
    paths = []
    if "user" in sources:
        paths.append(config / "settings.json")
    if "project" in sources:
        paths.append(project / "settings.json")
    if "local" in sources:
        paths.append(project / "settings.local.json")
        local_root = _local_settings_root(cwd)
        if local_root != cwd:
            paths.append(local_root / ".claude" / "settings.local.json")
    extra = env.get("CLAUDE_CODE_EXTRA_BODY", "{}")
    for path in paths:
        source = _read_settings(str(path), optional=True, cwd=cwd)
        if "CLAUDE_CODE_EXTRA_BODY" in source.get("env", {}):
            extra = source["env"]["CLAUDE_CODE_EXTRA_BODY"]
    if "CLAUDE_CODE_EXTRA_BODY" in explicit.get("env", {}):
        extra = explicit["env"]["CLAUDE_CODE_EXTRA_BODY"]
    if not isinstance(extra, str):
        raise ValueError("CLAUDE_CODE_EXTRA_BODY: esperado texto JSON")
    selected = service_tier_env(service_tier, extra)
    explicit["env"] = {**explicit.get("env", {}), **selected}
    try:
        json.dumps(explicit, allow_nan=False)
    except (TypeError, ValueError):
        raise ValueError("settings: JSON inválido") from None
    env.update(selected)
    return args, explicit


def env_de(nome: str, modelo: str | None = None, context_window: int | None = None,
           engine_account: str | None = None, *, engine_account_home: str | None = None,
           engine_account_base_url: str | None = None) -> dict[str, str]:
    """Variáveis de ambiente que fazem uma sessão rodar neste motor.

    `modelo`/`context_window` vêm da escolha da tela de abertura. Precisam entrar AQUI, e não só
    como `--model` na linha de comando, porque este env exporta o mesmo modelo em cinco chaves e a
    janela em outra: a flag ganha só de ANTHROPIC_MODEL, e o resto continuaria no modelo do motor.

    `subagent_model` configurado continua ganhando: quem o escreveu fez escolha de custo deliberada,
    e trocar o modelo principal não desfaz isso.

    KeyError no motor inexistente de propósito: env vazio faria a sessão subir na conta Anthropic
    ACHANDO que é o motor escolhido — o pior tipo de falha, a silenciosa.

    ValueError se qualquer valor contém caractere proibido (\\n, \\r, \\x00): contrato com o shell
    que `hangar-engine --env` exporta (uma linha por variável). Se engines.json foi hand-editado ou
    corrompido, rejeitamos em vez de silenciar.
    """
    e = listar()[nome]

    # Valida shell-safety: uma linha por variável é o contrato com o shell. Rejeita se o JSON
    # contém um valor que já violou a invariante (hand-edited file, corrupted write recovered by
    # another tool, etc). Reutiliza _PROIBIDO_NO_VALOR para uma única fonte de verdade.
    for campo in ("api_key", "model", "base_url", "subagent_model"):
        valor = e.get(campo, "")
        if isinstance(valor, str) and any(c in valor for c in _PROIBIDO_NO_VALOR):
            raise ValueError(f"{campo}: contém caractere proibido (quebra de linha ou nulo)")

    modelo_final = modelo or e["model"]
    if any(c in modelo_final for c in _PROIBIDO_NO_VALOR):
        # O modelo escolhido na abertura não passa pelo _normalizar (que só roda no SAVE): a
        # checagem de shell-safety tem que rodar aqui também — o valor vai pro `export` do wrapper.
        raise ValueError("model: contém caractere proibido (quebra de linha ou nulo)")
    account = None
    subagent = e.get("subagent_model") or modelo_final
    if engine_account is not None:
        from app import cliproxy_accounts
        if not engine_account_home or not engine_account_base_url:
            raise ValueError("CLIProxyAPI: conta fixa exige raiz Codex e endereço local validados")
        expected_base = validar_base_url(engine_account_base_url)
        if not _host_local(urlparse(expected_base).hostname or ""):
            raise ValueError("CLIProxyAPI: conta fixa exige um endereço local")
        if validar_base_url(e["base_url"]) != expected_base:
            raise ValueError("CLIProxyAPI: endereço do motor mudou; conta fixa não pode usar outro provedor")
        account = cliproxy_accounts.resolve(engine_account, home=engine_account_home)
        modelo_final = cliproxy_accounts.prefix_model(modelo_final, account["prefix"])
        subagent = cliproxy_accounts.prefix_model(subagent, account["prefix"])
    env = {
        # Marca lida do /proc/<pid>/environ para descobrir o motor de uma sessão viva (Task 5).
        "CP_ENGINE": nome,
        "ANTHROPIC_BASE_URL": e["base_url"],
        "ANTHROPIC_AUTH_TOKEN": e["api_key"],
        # Os 6 andam juntos: faltar um faz subagent/background falhar sem mensagem clara.
        "ANTHROPIC_MODEL": modelo_final,
        "ANTHROPIC_DEFAULT_OPUS_MODEL": modelo_final,
        "ANTHROPIC_DEFAULT_SONNET_MODEL": modelo_final,
        "ANTHROPIC_DEFAULT_HAIKU_MODEL": modelo_final,
        "ANTHROPIC_DEFAULT_FABLE_MODEL": modelo_final,
        # Subagentes fazem muita busca mecânica; um modelo mais barato aí é dinheiro de verdade.
        # Vazio (campo ausente) cai no mesmo modelo principal — nunca uma env var vazia.
        "CLAUDE_CODE_SUBAGENT_MODEL": subagent,
    }
    if account is not None:
        env["CP_ENGINE_ACCOUNT"] = account["account"]
        env["CP_ENGINE_CREDENTIAL_ID"] = account["credential_id"]
        env["CP_ENGINE_ACCOUNT_BASE_URL"] = expected_base
    if _booleano("auth_via_api_key", e.get("auth_via_api_key")) is True:
        # Provedor que lê a key SÓ em `x-api-key` e ignora `Authorization: Bearer`. opencode zen
        # (opencode.ai/zen/go) é um: medido em 06/08/2026, `Bearer` sozinho devolve o MESMO
        # "Missing API key." de uma requisição sem credencial, enquanto `x-api-key` errado devolve
        # "Invalid API key." — prova de que só o segundo header é lido.
        #
        # Por que header extra e não ANTHROPIC_API_KEY: essa var dispara o prompt "Do you want to
        # use this API key?" em sessão INTERATIVA (que é como o app abre), e recusar faz o CLI
        # descartar a key e voltar pro login OAuth — o cabeçalho vira "Claude Max" e o 401 volta,
        # agora por outra causa. AUTH_TOKEN nunca pergunta; o header cobre o que falta.
        env["ANTHROPIC_CUSTOM_HEADERS"] = f"x-api-key: {e['api_key']}"
    # MAX_CONTEXT_TOKENS, nao AUTO_COMPACT_WINDOW: medido nos dois provedores, a segunda nao move
    # a janela (o /context seguia em 200k) e a primeira move. Sem isto, um modelo de 256k/500k
    # compacta em ~167k — capacidade jogada fora, calado.
    #
    # Com modelo escolhido, a janela é a DELE — inclusive quando o MOTOR não configurou nenhuma
    # (context_window é opcional): o número vem do catálogo do provedor, e descartá-lo por causa de
    # um campo ausente do motor deixa a sessão compactando em ~167k com um modelo de 262k, com o
    # `--context` do prefixo aceito e ignorado, calado. Sem número pra ele, omitir — exportar a do
    # motor com outro modelo é exatamente o bug que esta variável existe pra corrigir.
    janela = context_window if modelo else e.get("context_window")
    if account is not None and janela is None:
        from app.cliproxy_accounts import base_model
        if base_model(modelo_final, account["prefix"]) == base_model(e["model"], account["prefix"]):
            janela = e.get("context_window")
    if catalog_model(modelo_final) != modelo_final:
        janela = 1_000_000
    if janela:
        env["CLAUDE_CODE_MAX_CONTEXT_TOKENS"] = str(_inteiro_positivo("context_window", janela))
    if _booleano("bundled_skills", e.get("bundled_skills")) is not True:
        # Default desligado por MEDIÇÃO: a skill empacotada `claude-api` não tem SKILL.md na raiz, e
        # invocá-la injeta os 64 arquivos dela de uma vez — medido em 847.630 chars / 206.553 tokens
        # num único turno (12% -> 67% da janela numa sessão gpt-5.6-sol de 372k). O gatilho dela é
        # amplo ("o prompt cita Claude/Anthropic em qualquer forma"), então um "qual modelo você é?"
        # basta. Na conta Anthropic isso passa (janela maior + poda server-side via
        # context_management, que nenhum provedor de terceiro implementa); num motor, mata a sessão.
        # Só as BUNDLED caem: plugin e ~/.claude/skills/ seguem intactas.
        env["CLAUDE_CODE_DISABLE_BUNDLED_SKILLS"] = "1"
    if _booleano("tool_search", e.get("tool_search")) is not True:
        # Default desligado por FAIL-SAFE, não por medição: a doc da Moonshot diz que o endpoint do
        # Kimi ainda não suporta Tool Search, e um erro no meio do turno é pior que uma ferramenta a
        # menos. Provedor que suportar liga com "tool_search": true.
        # Confirmado pela doc depois: o próprio Claude Code já desliga tool search quando
        # ANTHROPIC_BASE_URL aponta pra host não-first-party, "since most proxies do not forward
        # tool_reference blocks". Manter explícito não custa e vale pro gateway que se anuncia
        # first-party.
        env["ENABLE_TOOL_SEARCH"] = "false"
    if _booleano("experimental_betas", e.get("experimental_betas")) is not True:
        # Default desligado: campos beta que o CC manda sozinho (context_management, campos beta de
        # tool) fazem o upstream de terceiro devolver `400 Extra inputs are not permitted`. A doc do
        # gateway lista essa var como a remediação do lado do cliente.
        # ATENÇÃO à dependência: com isto ligado, tool search fica off e ENABLE_TOOL_SEARCH NÃO
        # consegue sobrepor — por isso a UI desabilita aquele toggle quando este está desmarcado.
        # NÃO cobre `output_config`/effort: a doc não lista remediação de cliente pra esse campo.
        env["CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS"] = "1"
        if _booleano("tool_search", e.get("tool_search")) is True:
            # A UI desabilita o toggle e explica, mas ela não é o único cliente: curl, um PWA velho
            # em cache ou uma automação salvam essa combinação e recebem 200. Sem esta linha, o
            # `GET /api/engines` mostra "tool_search: true" e ninguém descobre que está inerte.
            _log.warning("motor %r: tool_search=true não tem efeito com experimental_betas=false — "
                         "DISABLE_EXPERIMENTAL_BETAS mantém o tool search desligado e "
                         "ENABLE_TOOL_SEARCH não sobrepõe.", nome)
    if _booleano("prompt_caching", e.get("prompt_caching")) is False:
        # Default LIGADO (só desliga com false explícito): cache é economia, e num gateway ele já
        # degrada sozinho e calado — "if the gateway rejects the cache breakpoint, Claude Code
        # retries without it and leaves that block uncached for the rest of the conversation".
        # Desligar de propósito só faz sentido em provedor que cobra o cache mais caro que o miss.
        env["DISABLE_PROMPT_CACHING"] = "1"
    if _booleano("adaptive_thinking", e.get("adaptive_thinking")) is False:
        # Default LIGADO. Desligar thinking é destrutivo em alguns provedores: a doc da Kimi diz que
        # "disabling thinking routes both K3 and K2.7 Code to K2.6" — ou seja, rebaixa o modelo
        # calado. Só marque false se o upstream devolver 400 citando `thinking`/`adaptive`.
        env["CLAUDE_CODE_DISABLE_ADAPTIVE_THINKING"] = "1"
    if _booleano("gateway_model_discovery", e.get("gateway_model_discovery")) is True:
        # Opt-in: consulta /v1/models do gateway e popula o picker /model. Fora por padrão porque é
        # uma chamada extra a um endpoint que nem todo gateway expõe.
        env["CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY"] = "1"
    if _booleano("fine_grained_tool_streaming", e.get("fine_grained_tool_streaming")) is True:
        # Opt-in: o CC desliga por padrão atrás de base URL custom.
        env["CLAUDE_CODE_ENABLE_FINE_GRAINED_TOOL_STREAMING"] = "1"
    for campo, var in (("auto_compact_window", "CLAUDE_CODE_AUTO_COMPACT_WINDOW"),
                       ("max_output_tokens", "CLAUDE_CODE_MAX_OUTPUT_TOKENS")):
        if e.get(campo):
            # Sem valor = não inventa a var (o CC usa o default dele).
            env[var] = str(_inteiro_positivo(campo, e[campo]))
    return env
