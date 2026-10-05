"""Ponte privada de Git/arquivos. Falta de confirmação nunca repete uma escrita."""
import functools
import http.client
import inspect
import ipaddress
import json
import logging
import os
import threading
import urllib.error
import urllib.request

_log = logging.getLogger("hangar.workspace")
_config: tuple[str, str] | None = None
_slots = threading.BoundedSemaphore(8)
_MAX_RESPONSE = 32 * 1024 * 1024
# Teto do corpo que o Rust aceita na ponte (`MAX_BODY` em workspace_routes.rs).
_MAX_REQUEST = 4 * 1024 * 1024
# Erro da ponte com código: vira o envelope {code, params, msg} que o front traduz.
_MESSAGES = {
    "workspace_busy": "Git ocupado, tente em instantes.",
    "workspace_unavailable": "Git/arquivos indisponível: {motivo}",
    "workspace_request_too_large": "Pedido grande demais para o Git/arquivos.",
    "workspace_invalid_request": "Pedido inválido para o Git/arquivos.",
}


def _error(status: int, code: str, motivo: str) -> dict:
    return {"ok": False, "error": {"status": status, "code": code, "detail": motivo}}


def configure(address: str | None, secret: str | None) -> None:
    global _config
    _config = None
    if address is None or secret is None:
        return
    host, port = address.rsplit(":", 1)
    if not ipaddress.ip_address(host.strip("[]")).is_loopback or not 1 <= int(port) <= 65535:
        raise ValueError("invalid workspace address")
    _config = (address, secret)


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *_):
        return None


_opener = urllib.request.OpenerDirector()
for _handler in (urllib.request.ProxyHandler({}), urllib.request.HTTPHandler(),
                 urllib.request.HTTPDefaultErrorHandler(), urllib.request.HTTPErrorProcessor(), _NoRedirect()):
    _opener.add_handler(_handler)


def request(operation: str, arguments: dict, *, mutation: bool = False) -> dict | None:
    """`None` só quando o Python é dono do Git/arquivos: ponte desligada ou Rust fora do ar."""
    config = _config
    if config is None:
        return None
    try:
        data = json.dumps({"op": operation, "args": arguments}, ensure_ascii=False,
                          allow_nan=False, default=os.fspath).encode("utf-8")
    except (TypeError, ValueError):
        _failed(operation, False, "argumentos")
        return _error(500, "workspace_invalid_request", "argumentos")
    if len(data) > _MAX_REQUEST:
        _failed(operation, mutation, "pedido_grande")
        return _error(413, "workspace_request_too_large", "pedido grande demais")
    if not _slots.acquire(blocking=False):
        _failed(operation, mutation, "sem_vaga")
        return _error(503, "workspace_busy", "vagas cheias")
    try:
        req = urllib.request.Request(f"http://{config[0]}/__hangar_server/workspace", data=data,
            headers={"content-type": "application/json", "x-hangar-internal": config[1]}, method="POST")
        with _opener.open(req, timeout=160 if mutation else 50) as response:
            body = response.read(_MAX_RESPONSE + 1)
        if len(body) > _MAX_RESPONSE:
            raise ValueError("invalid workspace response")
        value = json.loads(body, parse_constant=lambda _: None)
        if not isinstance(value, dict) or type(value.get("ok")) is not bool:
            raise ValueError("invalid workspace response")
        if value["ok"] and "result" not in value:
            raise ValueError("missing workspace result")
        if not value["ok"] and (not isinstance(value.get("error"), dict)
                or type(value["error"].get("status")) is not int or "detail" not in value["error"]):
            raise ValueError("invalid workspace error")
        if not value["ok"] and value["error"].get("code") in _MESSAGES:
            _failed(operation, mutation, value["error"]["code"])
        return value
    except (OSError, ValueError, urllib.error.URLError, http.client.HTTPException) as e:
        refused = isinstance(getattr(e, "reason", None), ConnectionRefusedError)
        _failed(operation, mutation, "conexao_recusada" if refused else type(e).__name__)
        # Conexão recusada: o Rust está fora do ar e o pedido nem saiu; o Python assume a porta.
        if refused:
            return None
        if mutation:
            return {"ok": False, "error": {"status": 503, "code": "workspace_action_uncertain",
                "detail": "Não foi possível confirmar o resultado. Confira o estado antes de repetir."}}
        return _error(503, "workspace_unavailable", "sem resposta do Rust")
    finally:
        _slots.release()


def _failed(operation: str, mutation: bool, reason: str) -> None:
    from app import diag
    _log.warning("workspace bridge unavailable op=%s mutation=%s reason=%s", operation, mutation, reason)
    diag.registrar("workspace.ponte", "aviso", operacao_rust=operation, escrita=mutation, motivo=reason)


def text_rows(arguments: dict) -> dict:
    """Linhas da conversa já em memória vão como texto: o JSON da ponte não leva bytes."""
    if arguments.get("rows") is not None:
        arguments["rows"] = [row.decode("utf-8", errors="replace") for row in arguments["rows"]]
    return arguments


_MISSING = object()


def delegate(operation: str, exception, *, mutation=False, prepare=None, decode=None, quiet=_MISSING):
    """Mantém a assinatura; o corpo Python só roda com a ponte desligada.

    `quiet`: valor que a função original promete em vez de levantar (ex.: `git_summary` -> None).
    Falha da ponte devolve esse valor; o motivo já foi ao log e ao diário em `_failed`."""
    def decorate(original):
        signature = inspect.signature(original)
        @functools.wraps(original)
        def wrapped(*args, **kwargs):
            bound = signature.bind(*args, **kwargs)
            bound.apply_defaults()
            arguments = dict(bound.arguments)
            if prepare is not None:
                arguments = prepare(arguments)
            changing = mutation
            if operation == "git_action":
                changing = arguments["action"] not in ("status", "log")
            result = request(operation, arguments, mutation=changing)
            if result is None:
                return original(*args, **kwargs)
            if not result["ok"]:
                failure = result["error"]
                code = failure.get("code")
                if code in _MESSAGES and quiet is not _MISSING:
                    return quiet
                message = failure["detail"]
                if code in _MESSAGES:
                    message = _MESSAGES[code].format(motivo=failure["detail"])
                if operation in {"read_file", "read_at", "write_file", "write_at", "list_dir", "search", "resolver"}:
                    raise exception(failure["status"], code or "erro_arq_lista_falhou", message)
                # Texto no `detail`, como todo GitError/FsError; o código vai à parte para o handler da API.
                error = exception(failure["status"], message)
                error.code = code
                raise error
            value = result["result"]
            if decode is not None:
                return decode(value)
            if operation in {"head_info", "create_worktree", "cited_elsewhere"}:
                return tuple(value)
            return value
        return wrapped
    return decorate
