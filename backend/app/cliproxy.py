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


def is_engine_model(model_id: object) -> bool:
    # gpt-image-* gera imagem e não responde /v1/messages: não serve de motor.
    return isinstance(model_id, str) and not model_id.startswith("gpt-image-")


def redact(texto: str, api_key: str) -> str:
    # O erro do proxy pode repetir a chave ("invalid key …"), e ele vai para a tela.
    return texto.replace(api_key, "***") if api_key else texto
