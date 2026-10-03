"""Serviços de dados do ator; nenhum cliente CLI ou autoridade paralela de fila."""
from __future__ import annotations

import asyncio
import json
import threading
import time
import uuid
from pathlib import Path
from types import SimpleNamespace

from app import diag, log_paths

_PATCH = {
    "claude": {"session_id", "cwd", "model", "effort", "permission_mode", "previous_non_plan", "context_window", "problema"},
    "codex": {"thread_id", "rollout_path", "cwd", "model", "effort", "mode", "skipped_async_questions", "problema"},
}
_unknown_guard = threading.Lock()
_unknown_counts = {}
_mutation_locks = {}
_quota_lock = threading.Lock()
_quota_cache = {}


async def execute(kind: str, payload: dict, metadata: dict) -> dict:
    task = asyncio.create_task(asyncio.to_thread(run, kind, payload, metadata))
    try:
        data = await asyncio.shield(task)
        return {"ok": True, "data": data}
    except asyncio.CancelledError:
        try:
            await asyncio.shield(task)
        finally:
            raise
    except Exception as exc:
        diag.registrar("runtime.policy_failed", "erro", codigo=type(exc).__name__)
        return {"ok": False, "error_type": type(exc).__name__}


def _quota(metadata):
    from app import cotas
    root = Path(metadata.get("config_dir") or Path.home() / ".claude").resolve()
    with _quota_lock:
        cached = _quota_cache.get(root)
        if cached is not None and time.monotonic() - cached[0] < 300:
            return cached[1]
        try:
            value = next((account.model_dump() for account in cotas.listar_cotas()
                if account.provedor == "claude" and ":" in account.id
                and Path(account.id.split(":", 1)[1]).resolve() == root), None)
        except Exception as exc:
            diag.registrar("runtime.quota_failed", "erro", codigo=type(exc).__name__)
            value = cached[1] if cached else None
        _quota_cache[root] = (time.monotonic(), value)
        return value


def _unknown(payload, metadata):
    from app.adapters.claude_headless.adapter import _MAX_DESCONHECIDOS_B, _TETO_DESCONHECIDOS
    kind = payload.get("kind")
    if not isinstance(kind, str) or len(kind) > 512 or not isinstance(payload.get("event"), dict):
        raise ValueError("evento privado inválido")
    with _unknown_guard:
        key = (metadata.get("key"), metadata.get("generation"), kind)
        if _unknown_counts.get(key, 0) >= _TETO_DESCONHECIDOS:
            return {"recorded": False, "reason": "type_limit"}
        directory = log_paths.base() / "privado"
        directory.mkdir(mode=0o700, parents=True, exist_ok=True)
        path = directory / ("claude-headless-desconhecidos.jsonl" if metadata["provider"] == "claude" else "codex-headless-desconhecidos.jsonl")
        if path.exists() and path.stat().st_size > _MAX_DESCONHECIDOS_B:
            return {"recorded": False, "reason": "file_limit"}
        line = json.dumps({"ts": time.time(), "sessao": metadata.get("name"), "tipo": kind, "evento": payload["event"]}, ensure_ascii=False)
        with path.open("a", encoding="utf-8") as stream:
            stream.write(line + "\n")
        _unknown_counts[key] = _unknown_counts.get(key, 0) + 1
    return {"recorded": True}


def native_message(payload, metadata):
    from app import api, uds_messaging
    text = payload.get("text")
    operation_id = metadata.get("operation_id")
    if not isinstance(text, str) or not isinstance(operation_id, str) or not operation_id:
        raise ValueError("recado sem operação registrada")
    sender, _ = uds_messaging.separar_prefixo(text)
    if sender is None:
        return {"outcome": "not_written", "reason": "ordinary_input"}
    endpoint = uds_messaging.socket_da_sessao(metadata.get("session_id", ""), metadata.get("config_dir"))
    if not endpoint:
        return {"outcome": "not_written", "reason": "no_socket"}
    mode = api._classe_modo(sender, metadata["name"], metadata.get("config_dir"))
    mid = str(uuid.uuid5(uuid.NAMESPACE_URL, "hangar:" + metadata["key"] + ":" + operation_id))
    validate = metadata.get("validate")
    if validate is None:
        raise RuntimeError("recado sem confirmação de posse")
    validate()
    try:
        uds_messaging.enviar(endpoint, text, sender, mode, msg_id=mid)
    except Exception:
        return {"outcome": "unknown", "msg_id": mid}
    return {"outcome": "written", "msg_id": mid}


def run(kind: str, payload: dict, metadata: dict) -> dict:
    if not isinstance(payload, dict) or not isinstance(metadata, dict):
        raise ValueError("serviço com dados inválidos")
    provider = metadata.get("provider")
    if provider not in _PATCH:
        raise ValueError("provedor fora do runtime")
    if kind == "prepare_prompt":
        text = payload.get("text")
        if not isinstance(text, str):
            raise ValueError("entrada sem texto")
        if provider == "claude":
            from app.adapters.claude_headless.adapter import _blocos_do_prompt
            from app import uds_messaging
            content, notices = _blocos_do_prompt(text)
            sender, _ = uds_messaging.separar_prefixo(text)
            return {"content": content, "notices": notices, "native_candidate":sender is not None}
        return {"input": [{"type": "text", "text": text}],
                "skill_name":text.lstrip().split()[0][1:] if text.lstrip().startswith("/") else None}
    if kind == "skill_catalog":
        from app.adapters.codex.chat_controls import skills_do_catalogo
        skills = skills_do_catalogo(payload["catalog"])
        return {"skill":next((skill for skill in skills if skill["name"] == payload.get("name")), None)}
    if kind == "answer_body":
        from app.adapters.claude_headless.adapter import respostas_do_app
        body, conversation = respostas_do_app(payload["questions"], payload["answers"])
        return {"body": body, "conversation": conversation}
    if kind == "format_status":
        if provider == "codex":
            from app.adapters.codex.adapter import format_status_line
            return {"status_line": format_status_line(payload.get("model"), payload.get("effort"), payload.get("token_usage"), payload.get("rate_limits"))}
        from app.adapters.claude_headless.adapter import ClaudeHeadlessAdapter, _hora_local
        quota = _quota(metadata)
        windows = [SimpleNamespace(**window) for window in (quota or {}).get("janelas", []) if not window.get("por_modelo")]
        data = SimpleNamespace(model=payload.get("model"), effort=payload.get("effort"), usage=payload.get("usage"),
            context_window=payload.get("context_window"), cost=payload.get("cost"), meta=metadata, janelas=windows)
        rate = payload.get("rate_limit_info") or {}
        return {"status_line": ClaudeHeadlessAdapter.status_line(None, data),
                "limit_reset": _hora_local(rate.get("resetsAt")) if rate.get("status") == "rejected" else None}
    if kind == "last_usage":
        from app.adapters.claude_headless.adapter import _uso_da_ultima_chamada
        path = Path(metadata["jsonl"])
        try:
            with path.open("rb"):
                pass
        except FileNotFoundError:
            return {"usage":None}
        return {"usage": _uso_da_ultima_chamada(str(path))}
    if kind == "reload_stamp":
        from app.adapters.claude_headless.adapter import _marca_config
        recorded = (metadata.get("cano") or {}).get("config_marca")
        return {"reason": "config" if recorded and _marca_config(metadata.get("config_dir")) != recorded else None}
    if kind == "quota":
        return {"account": _quota(metadata)}
    if kind == "native_message":
        return native_message(payload, metadata)
    if kind == "unknown_private":
        return _unknown(payload, metadata)
    if kind == "session.patch_meta":
        if set(payload) - _PATCH[provider]:
            raise ValueError("campo fora do catálogo do sidecar")
        validate = metadata.get("validate")
        if validate is None:
            raise RuntimeError("alteração sem confirmação de posse")
        with _unknown_guard:
            lock = _mutation_locks.setdefault(metadata["key"], threading.Lock())
        with lock:
            validate()
            state_path = metadata.get("state_path")
            if state_path:
                view = json.loads(Path(state_path).read_bytes())["runtime_state"].get("view") or {}
                for key, value in payload.items():
                    source = "conversation" if key == "session_id" else key
                    if source in view and view[source] != value:
                        return {"updated": False, "stale": True}
            if provider == "claude":
                from app.adapters.claude_headless import sessions
            else:
                from app.adapters.codex import sessions
            current = sessions.load(metadata["name"])
            if not current or current.get("key") != metadata["key"]:
                raise RuntimeError("sidecar de outra vida")
            updated = sessions.update(metadata["name"], **payload)
            if updated is None:
                raise RuntimeError("sidecar desapareceu durante a alteração")
        return {"updated": True}
    if kind == "session.marker":
        if provider != "claude" or payload:
            raise ValueError("marcador inválido")
        metadata["validate"]()
        from app.adapters.claude_headless import sessions
        sessions.marcar_troca(metadata["name"])
        return {"marked": True}
    if kind == "diag.error":
        code = payload.get("error_type")
        if not isinstance(code, str) or not code.isidentifier() or len(code) > 128:
            raise ValueError("tipo de falha inválido")
        diag.registrar("runtime.service_failed", "erro", codigo=code)
        return {"recorded": True}
    raise ValueError("serviço não permitido")
