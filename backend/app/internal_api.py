# backend/app/internal_api.py
"""Rotas internas que só o hangar-server, filho deste backend na mesma máquina, consome."""
import secrets

from fastapi import APIRouter, Depends, HTTPException, Request

from app.models import session_key

_LOOPBACK = {"127.0.0.1", "::1"}

# Só na memória: no os.environ ele iria para toda sessão que o backend sobe.
_secret: str | None = None


def set_secret(value: str | None) -> None:
    global _secret
    _secret = value or None


def require_internal(request: Request) -> None:
    # 404 em toda recusa: quem não é o hangar-server não descobre que a rota existe. O segredo nasce
    # a cada subida e o repasse do hangar-server manda o IP real no X-Forwarded-For, então quem
    # chega de fora pela porta pública cai no IP antes do segredo.
    secret = _secret
    ip = request.client.host if request.client else None
    given = request.headers.get("x-hangar-internal", "")
    if secret is None or ip not in _LOOPBACK or not secrets.compare_digest(given.encode(), secret.encode()):
        raise HTTPException(status_code=404)


def info_payload(name: str, provider: str, jsonl: str | None) -> dict:
    """O `InternalInfo` do Rust: a rota `info` e o evento `info` do side-events usam este mesmo."""
    from app.adapters import chave_de
    from app.pqueue import PromptQueue

    return {
        # Chave do adapter: o Claude sem terminal vem como "claude-headless".
        "provider": chave_de(name, provider),
        "jsonl": jsonl,
        "session_key": session_key(jsonl) if jsonl else "",
        # Tudo que o merged_history do Rust precisa além do transcript.
        "history": {"queue": str(PromptQueue(name).path)},
    }


router = APIRouter(prefix="/internal", dependencies=[Depends(require_internal)], include_in_schema=False)


@router.get("/sessions/{name}/info")
async def session_info(name: str) -> dict:
    # Import tardio: api.py importa este módulo no topo.
    from app import api

    info = await api._cached_info(name)
    if info is None:
        raise HTTPException(status_code=404)
    return info_payload(name, info.provider, info.jsonl)
