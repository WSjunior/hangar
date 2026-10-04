"""Instância local do CLIProxyAPI: endereço e chave lidos do config dele.

A chave nunca sai do servidor: a tela recebe só o endereço e os modelos, e o motor é gravado com
`use_cliproxy_key`, que pega a chave daqui. O navegador pode ser o celular.
"""
from pathlib import Path
from typing import Any

import yaml

_PORTA_PADRAO = 8317


def config_path() -> Path:
    return Path.home() / ".cli-proxy-api" / "config.yaml"


def normalize_base(url: str) -> str:
    url = url.strip().rstrip("/")
    return url[:-3].rstrip("/") if url.endswith("/v1") else url


def local() -> dict[str, str] | None:
    """`{base_url, api_key}` da instância desta máquina; None sem config.

    Config que existe e não serve (ilegível, sem api-keys) levanta ValueError: "não instalado" e
    "instalado sem chave" são respostas diferentes para quem configura.
    """
    try:
        texto = config_path().read_text(encoding="utf-8")
    except FileNotFoundError:
        return None
    except OSError as e:
        raise ValueError(f"{config_path()}: {e.strerror or e}") from None
    try:
        dados: Any = yaml.safe_load(texto)
    except yaml.YAMLError:
        raise ValueError(f"{config_path()}: YAML inválido") from None
    if not isinstance(dados, dict):
        return None
    lista = dados.get("api-keys")
    if lista is not None and not isinstance(lista, list):
        raise ValueError(f"{config_path()}: api-keys deve ser uma lista")
    chaves = [k.strip() for k in (lista or []) if isinstance(k, str) and k.strip()]
    if not chaves:
        raise ValueError(f"{config_path()}: sem api-keys")
    host = str(dados.get("host") or "").strip()
    # Escutar em todas as interfaces inclui o loopback, que é por onde este servidor fala com ele.
    if host in ("", "0.0.0.0", "::"):
        host = "127.0.0.1"
    if ":" in host and not host.startswith("["):
        host = f"[{host}]"
    try:
        porta = int(dados.get("port") or _PORTA_PADRAO)
    except (TypeError, ValueError):
        raise ValueError(f"{config_path()}: port inválida") from None
    if not 0 < porta < 65536:
        raise ValueError(f"{config_path()}: port inválida")
    tls = dados.get("tls")
    esquema = "https" if isinstance(tls, dict) and tls.get("enable") is True else "http"
    return {"base_url": f"{esquema}://{host}:{porta}", "api_key": chaves[0]}


def is_local_engine(cfg: dict) -> bool:
    inst = local()
    return bool(inst and normalize_base(str(cfg.get("base_url") or "")) == inst["base_url"])


def account_for_engine(cfg: dict, account: str, home: str | None = None) -> dict:
    from app.cliproxy_accounts import resolve
    inst = local()
    if not inst or normalize_base(str(cfg.get("base_url") or "")) != inst["base_url"]:
        raise ValueError("a conta ChatGPT só pode ser fixada no CLIProxyAPI local")
    entry = resolve(account, home=home)
    return {**entry, "home": entry["credential_id"].removeprefix("codex:"),
            "base_url": inst["base_url"]}


def validate_models(cfg: dict, model: str, account: dict, models: list[dict] | None = None) -> list[dict]:
    from app import engine_probe
    from app.cliproxy_accounts import base_model, models_for
    if models is None:
        try:
            models = engine_probe.listar_modelos(cfg["base_url"], cfg["api_key"])
        except (RuntimeError, ValueError) as exc:
            raise ValueError("CLIProxyAPI: catálogo indisponível: " + redact(str(exc), cfg["api_key"])) from None
    catalog = models_for(models, account["prefix"])
    available = {item["id"] for item in catalog}
    for label, selected in (("principal", model), ("dos subagentes", cfg.get("subagent_model") or model)):
        if base_model(selected, account["prefix"]) not in available:
            raise ValueError(f"CLIProxyAPI: modelo {label} indisponível nesta conta")
    return catalog


def engine_env(name: str, model: str | None, context_window: int | None, account_id: str, *,
               home: str | None = None, models: list[dict] | None = None,
               expected_base: str | None = None) -> dict[str, str]:
    from app import engines
    cfg = engines.listar().get(name)
    if not cfg:
        raise ValueError("CLIProxyAPI: motor indisponível")
    account = account_for_engine(cfg, account_id, home=home)
    if expected_base is not None and normalize_base(expected_base) != account["base_url"]:
        raise ValueError("CLIProxyAPI: endereço da conta fixa mudou")
    validate_models(cfg, model or cfg["model"], account, models)
    return engines.env_de(name, model, context_window, engine_account=account_id,
                          engine_account_home=account["home"], engine_account_base_url=account["base_url"])


def is_engine_model(model_id: object) -> bool:
    # gpt-image-* gera imagem e não responde /v1/messages: não serve de motor.
    return isinstance(model_id, str) and not model_id.startswith("gpt-image-")


def redact(texto: str, api_key: str) -> str:
    # O erro do proxy pode repetir a chave ("invalid key …"), e ele vai para a tela.
    return texto.replace(api_key, "***") if api_key else texto
