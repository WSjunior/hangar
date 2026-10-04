"""Falhas conhecidas do hangar-server no diário, sem conteúdo livre do processo filho."""
import json
import re

from fastapi import HTTPException, Request

from app import diag

MAX_BODY_BYTES = 1024
FAILURE_REASONS = {
    "no_scopes": "Escopos de custos indisponíveis.",
    "no_disk": "Índice de custos indisponível.",
    "reader_panic": "Falha no leitor de custos.",
    "sqlite": "Falha no índice de custos.",
    "json": "Falha ao preparar a resposta de custos.",
    "worker_join": "Falha no processamento de custos.",
    "info_unavailable": "Informações da sessão indisponíveis.",
    "io": "Falha na leitura dos dados de custos.",
    "non_finite": "Valor de custos ou uso inválido.",
}
PARTS = {"costs", "usage", "exchange_rate", "session_cost"}


def _unique_object(pairs: list[tuple[str, object]]) -> dict:
    body = {}
    for key, value in pairs:
        if key in body:
            raise ValueError("campo repetido")
        body[key] = value
    return body


def _validate_body(raw: bytearray) -> dict:
    body = json.loads(raw, object_pairs_hook=_unique_object)
    if not isinstance(body, dict):
        raise ValueError("objeto obrigatório")
    if set(body) not in ({"part", "code", "attempt", "transferred"},
                          {"part", "code", "attempt", "transferred", "session"}):
        raise ValueError("campos inválidos")
    part, code = body["part"], body["code"]
    if not isinstance(part, str) or part not in PARTS or not isinstance(code, str) or code not in FAILURE_REASONS:
        raise ValueError("causa inválida")
    if type(body["attempt"]) is not int or not 1 <= body["attempt"] <= 4 or type(body["transferred"]) is not bool:
        raise ValueError("tentativa inválida")
    if part == "session_cost":
        session = body.get("session")
        if not isinstance(session, str) or not re.fullmatch(r"[A-Za-z0-9._-]{1,64}", session):
            raise ValueError("sessão inválida")
    elif "session" in body:
        raise ValueError("sessão fora da parte")
    return body


async def record_rust_failure(request: Request) -> dict[str, bool]:
    """Handler do router interno: autenticação obrigatória na dependência `require_internal`."""
    raw = bytearray()
    async for chunk in request.stream():
        if len(raw) + len(chunk) > MAX_BODY_BYTES:
            raise HTTPException(413)
        raw.extend(chunk)
    try:
        body = _validate_body(raw)
    except (ValueError, TypeError, RecursionError):
        # A recusa não devolve os valores nem as mensagens do parser ao chamador.
        raise HTTPException(400) from None
    fields = {"codigo": body["code"], "detalhe": FAILURE_REASONS[body["code"]],
              "etapa": body["part"], "tentativa": body["attempt"]}
    if "session" in body:
        fields["sessao"] = body["session"]
    request_id = request.headers.get("x-hangar-req", "")
    if not re.fullmatch(r"[A-Za-z0-9_-]{1,32}", request_id):
        request_id = ""
    token = diag.req_atual.set(request_id)
    try:
        # O Rust envia a falha e a transferência separadamente e garante uma única transferência.
        if body["transferred"]:
            diag.registrar("rust.part_to_python", "aviso", **fields)
        else:
            diag.registrar("rust.part_failed", "erro", **fields)
    finally:
        diag.req_atual.reset(token)
    return {"ok": True}
