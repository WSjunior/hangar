"""Catálogo de modelos do Codex: a fonte da lista na tela de ABERTURA, onde ainda não há sessão.

Por que não é como nenhum dos outros três: o `~/.codex/config.toml` guarda só o modelo escolhido
(`model = "..."`), nunca a lista — então não há o caminho do Kimi. E não existe `codex
--list-models`, então também não há o caminho do Pi. O que existe é o `model/list` do app-server, e
medido em 30/08/2026 (codex-cli 0.151.0) ele responde no modo **stdio**, sem `--listen`, sem thread
aberta e sem sessão viva, em 0,78s. É a mesma fonte que a folha da sessão viva usa
(`CodexAdapter.list_models`), só que por um processo efêmero em vez do app-server do pane.

Os `efforts` de cada modelo vêm daqui e não de uma lista no código porque **variam por modelo**
(medido: `gpt-5.6-sol` aceita `ultra`, `gpt-5.6-luna` não; `gpt-5.5` também não aceita `max`) — a
mesma lição que o Pi já tinha ensinado.

Antes do app-server, a lista vem do `/codex/models` do backend do ChatGPT por HTTP (`_listar_http`),
a mesma rota de onde o app-server a tira; o processo efêmero fica de reserva. Ver "Cota e catálogo
do Codex por HTTP" em docs/decisoes/harnesses.md.

Cache pelo motivo do pi_catalog: a lista muda de mês em mês.
"""
import json
import copy
import hashlib
import logging
import time
import tomllib
import urllib.parse
from pathlib import Path

from app import codex_appserver
from app import codex_contas

# Reexportado porque a rota captura este erro por aqui, e ele não muda de significado no caminho
# do catálogo: "não achei o codex" continua sendo outra conversa que "o codex falhou".
CodexAusente = codex_appserver.CodexAusente
CodexIndisponivel = codex_appserver.CodexIndisponivel
CodexRecusado = codex_appserver.CodexRecusado
CodexRespostaInvalida = codex_appserver.CodexRespostaInvalida


class CodexLimitado(CodexIndisponivel):
    """O backend respondeu 429 ao catálogo."""

_log = logging.getLogger("hangar.codex_models")
_TTL = 600.0
_cache: dict[tuple[str, tuple], tuple[float, list[dict]]] = {}


def _cache_key(codex_home: str | Path | None) -> tuple[str, tuple]:
    path = Path(codex_home) if codex_home is not None else codex_contas.default_home()
    path = path.expanduser().resolve(strict=False)
    assinatura = []
    for nome in ("auth.json", "config.toml"):
        arquivo = path / nome
        try:
            assinatura.append((nome, hashlib.sha256(arquivo.read_bytes()).hexdigest()))
        except OSError:
            assinatura.append((nome, None))
    return str(path), tuple(assinatura)


def invalidar(codex_home: str | Path | None = None) -> None:
    global _cache
    if not isinstance(_cache, dict):
        _cache = {}
    if codex_home is None:
        _raw_catalogs.clear()
        _cache.clear()
        return
    raiz = _cache_key(codex_home)[0]
    for key in list(_raw_catalogs):
        if key[0][0] == raiz:
            _raw_catalogs.pop(key, None)
    for key in list(_cache):
        if key[0] == raiz:
            _cache.pop(key, None)


def parse(result: dict) -> list[dict]:
    """A resposta do `model/list` no formato da tela. Estoura se não sobrar modelo nenhum."""
    if not isinstance(result, dict):
        raise CodexRespostaInvalida("codex app-server retornou catálogo inválido")
    data = result.get("data")
    if not isinstance(data, list):
        raise CodexRespostaInvalida("codex app-server retornou catálogo inválido")
    out: list[dict] = []
    for m in data:
        # `hidden` é o provedor dizendo "não ofereça este": oferecer faria a sessão nascer num id
        # que o plano do usuário não atende, e a falha só apareceria no primeiro turno.
        if not isinstance(m, dict) or m.get("hidden") or not m.get("model"):
            continue
        efforts = m.get("supportedReasoningEfforts") or []
        if not isinstance(efforts, list):
            raise CodexRespostaInvalida("codex app-server retornou esforços inválidos")
        out.append({
            "id": m["model"],
            "name": m.get("displayName") or m["model"],
            "desc": m.get("description") or "",
            "efforts": [e.get("reasoningEffort") for e in efforts
                        if isinstance(e, dict) and e.get("reasoningEffort")],
            # `default_effort` é o mesmo campo que o catálogo do Kimi já manda — a tela lê os dois
            # pelo mesmo `ModelOption`. O `isDefault` do provedor NÃO entra: quem decide o padrão
            # desta máquina é o `model` do `~/.codex/config.toml`, e mostrar o outro como "padrão"
            # apontaria pro modelo errado.
            "default_effort": m.get("defaultReasoningEffort"),
        })
    if not out:
        # Zero modelo com rc=0 é falha do provedor (login vencido, versão que mudou o schema), não
        # "seu plano não tem modelo". Levanta pra virar o 502 que a rota já sabe dar, e o caller
        # NÃO cacheia: senão o erro duraria 10 min depois de o Codex voltar.
        raise CodexRespostaInvalida("codex app-server nao devolveu modelo nenhum em model/list")
    return out


def _listar_http(raiz: Path) -> list[dict] | None:
    """O `/codex/models` do backend do ChatGPT, que é de onde o próprio app-server tira o
    `model/list`. None = este caminho não serve agora e o app-server responde no lugar."""
    try:
        config = tomllib.loads((raiz / "config.toml").read_text(encoding="utf-8"))
    except FileNotFoundError:
        config = {}
    except (OSError, ValueError):
        config = None
    try:
        # Outro provedor ou outro backend: a lista não é a do ChatGPT, e quem sabe montá-la é o CLI.
        if config is None or config.get("model_provider", "openai") != "openai" \
                or "chatgpt_base_url" in config:
            raise CodexIndisponivel("config fora do padrão")
        versao = codex_appserver.versao()
        status, corpo = codex_appserver.backend_get(
            f"/codex/models?client_version={urllib.parse.quote(versao)}", codex_home=raiz)
        if status == 429:
            raise CodexLimitado("http 429")
        brutos = corpo.get("models") if status == 200 and isinstance(corpo, dict) else None
        if not isinstance(brutos, list):
            raise CodexIndisponivel(f"http {status}")
        _raw_catalogs[(_cache_key(raiz), versao)] = (time.monotonic(), copy.deepcopy(brutos))
        data = []
        # O app-server ordena por `priority`; sem isso o primeiro da tela mudaria.
        for m in sorted((m for m in brutos if isinstance(m, dict)),
                        key=lambda m: m.get("priority") if isinstance(m.get("priority"), int) else 1 << 30):
            niveis = m.get("supported_reasoning_levels")
            if not isinstance(niveis, list):
                raise CodexIndisponivel("formato-desconhecido")
            data.append({
                "model": m.get("slug"), "displayName": m.get("display_name"),
                "description": m.get("description"),
                "hidden": m.get("visibility") != "list",
                "supportedReasoningEfforts": [{"reasoningEffort": n.get("effort")}
                                              for n in niveis if isinstance(n, dict)],
                "defaultReasoningEffort": m.get("default_reasoning_level"),
            })
        modelos = parse({"data": data})
    except (CodexIndisponivel, CodexRespostaInvalida) as e:
        if isinstance(e, CodexLimitado):
            raise
        # info e não debug: cair calado no app-server é a regressão que ninguém veria.
        _log.info("catalogo codex %s pelo app-server (http: %s)", raiz, e)
        return None
    _log.debug("catalogo codex %s por http", raiz)
    return modelos


def listar(fresco: bool = False, *, codex_home: str | Path | None = None) -> list[dict]:
    key = _cache_key(codex_home)
    cached = _cache.get(key)
    if cached and not fresco and time.monotonic() - cached[0] < _TTL:
        return cached[1]
    try:
        modelos = _listar_http(Path(key[0]))
    except CodexLimitado:
        # O app-server bate no mesmo backend com o mesmo token: cair nele só renovaria o 429.
        # Com catálogo guardado, ele vale por mais um TTL em vez de insistir.
        if cached is None:
            raise
        _log.info("catalogo codex %s: http 429, mantendo o guardado", key[0])
        _cache[key] = (time.monotonic(), cached[1])
        return cached[1]
    if modelos is None:
        result = (codex_appserver.perguntar("model/list") if codex_home is None else
                  codex_appserver.perguntar("model/list", codex_home=Path(key[0])))
        modelos = parse(result)
    _cache[key] = (time.monotonic(), modelos)
    return modelos


def checar_escolha(model: str | None, effort: str | None, *,
                   codex_home: str | Path | None = None) -> None:
    """Recusa (ValueError) modelo fora do catálogo, ou nível que AQUELE modelo não lista.

    `model_args` só valida a FORMA do nível — não pode ter lista fechada, porque os níveis variam
    por modelo. Sem esta checagem, `--effort ultra` num `gpt-5.5` nasce a sessão e o binário
    **descarta o nível calado** (medido em 30/08/2026: ele não morre, segue com o dele), ou seja, o
    app reportaria sucesso sobre uma escolha que não valeu. Mesma doutrina do `engine_model_set`:
    recusar aqui em vez de deixar a falha aparecer só no turno.

    Nível sem modelo não é checável (o modelo então é o do `~/.codex/config.toml`, que este
    catálogo não diz qual é) e passa.
    """
    if model is None:
        return
    modelos = listar() if codex_home is None else listar(codex_home=codex_home)
    for m in modelos:
        if m["id"] == model:
            if effort is not None and effort not in m["efforts"]:
                raise ValueError(f"nivel fora do suporte de {model}: {effort!r} "
                                 f"(use um de {', '.join(m['efforts']) or 'nenhum'})")
            return
    raise ValueError(f"modelo fora do catalogo do Codex: {model}")


# O catálogo de UI perde campos necessários para provar capacidade.
_raw_catalogs: dict[tuple, tuple[float, list[dict]]] = {}


def raw_model(codex_home: str | Path, effective_config: dict, slug: str,
              version: str) -> dict:
    """Metadado completo, ligado à conta, à configuração efetiva e ao CLI."""
    root = Path(codex_home).expanduser().resolve(strict=False)
    local = effective_config.get("model_catalog_json")
    if local:
        path = Path(local)
        if not path.is_absolute():
            raise CodexRespostaInvalida("session_transfer_model_capacity_unknown")
        try:
            catalog = json.loads(path.read_text(encoding="utf-8"))
            models = catalog["models"]
        except (OSError, ValueError, KeyError, TypeError) as exc:
            raise CodexRespostaInvalida("session_transfer_model_capacity_unknown") from exc
    else:
        if (effective_config.get("model_provider") not in (None, "openai")
                or effective_config.get("chatgpt_base_url")
                or (effective_config.get("model_providers") or {}).get("openai")):
            raise CodexRespostaInvalida("session_transfer_model_capacity_unknown")
        key = (_cache_key(root), version)
        cached = _raw_catalogs.get(key)
        if cached and time.monotonic() - cached[0] < _TTL:
            models = cached[1]
        else:
            status, payload = codex_appserver.backend_get(
                f"/codex/models?client_version={urllib.parse.quote(version)}", codex_home=root)
            if status != 200 or not isinstance(payload, dict):
                raise CodexRespostaInvalida("session_transfer_model_capacity_unknown")
            models = payload.get("models")
            if isinstance(models, list):
                _raw_catalogs[key] = (time.monotonic(), copy.deepcopy(models))
    if not isinstance(models, list):
        raise CodexRespostaInvalida("session_transfer_model_capacity_unknown")
    matches = [m for m in models if isinstance(m, dict) and m.get("slug") == slug]
    if len(matches) != 1 or matches[0].get("used_fallback_model_metadata"):
        raise CodexRespostaInvalida("session_transfer_model_capacity_unknown")
    return copy.deepcopy(matches[0])
