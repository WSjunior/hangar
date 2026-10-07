"""Ponte para a rota privada de páginas do hangar-server (Rust). Sem o Rust de pé, páginas não existem."""
import http.client
import json
import urllib.error
import urllib.request

_config: tuple[str, str] | None = None
_TIMEOUT = 30.0  # o Rust espera vaga no Chromium (fora do prazo) e mais 8 s de render
_MAX_RESPONSE = 256 * 1024
_opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))


class PagesBridgeError(Exception):
    def __init__(self, code: str, detail: str = ""):
        super().__init__(f"{code}: {detail}" if detail else code)
        self.code, self.detail = code, detail


def configure(address: str | None, secret: str | None) -> None:
    global _config
    _config = (address, secret) if address and secret else None


def publish(payload: dict) -> dict:
    config = _config
    if config is None:
        raise PagesBridgeError("erro_paginas_sem_servidor_rust", "páginas precisam do hangar-server de pé")
    req = urllib.request.Request(
        f"http://{config[0]}/__hangar_server/pages",
        data=json.dumps(payload, ensure_ascii=False).encode("utf-8"),
        headers={"content-type": "application/json", "x-hangar-internal": config[1]},
        method="POST")
    try:
        with _opener.open(req, timeout=_TIMEOUT) as response:
            body = response.read(_MAX_RESPONSE + 1)
        if len(body) > _MAX_RESPONSE:
            raise ValueError("resposta grande demais")
        value = json.loads(body)
    except (OSError, ValueError, urllib.error.URLError, http.client.HTTPException) as e:
        raise PagesBridgeError("erro_paginas_indisponivel", type(e).__name__) from e
    if not isinstance(value, dict) or value.get("ok") is not True:
        error = value.get("error") if isinstance(value, dict) and isinstance(value.get("error"), dict) else {}
        raise PagesBridgeError(str(error.get("code") or "erro_paginas_indisponivel"), str(error.get("detail") or ""))
    result = value.get("result")
    if not isinstance(result, dict):
        raise PagesBridgeError("erro_paginas_indisponivel", "resposta sem resultado")
    return result
