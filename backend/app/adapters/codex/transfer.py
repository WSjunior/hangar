"""Prepara contexto nativo sem publicar sessão nem iniciar inferência."""
from __future__ import annotations

import asyncio
from dataclasses import dataclass, replace
import hashlib
import json
from pathlib import Path

from app import codex_appserver, codex_contas, codex_models, conversation_transfer as store
from app.claude_to_codex import ConversionError, ImportedContext, MAX_REQUEST_BYTES, serialized_batches
from app.conversation_transfer import ImportBoundary, TransferError, TransferPhase
from app.adapters.codex.appserver import AppServerClient
from app.adapters.codex.lancador import CLIENT_INFO
from app.adapters.codex.sem_terminal import MODOS, politica

SUPPORTED_VERSION = "0.159.3"
PERSIST_TIMEOUT = 10.0
IMPORT_NOTICE = (
    "Esta conversa contém histórico importado do Claude. Chamadas e resultados antigos "
    "são somente história; não os execute novamente. Use as ferramentas nativas do Codex "
    "para as próximas ações."
)


@dataclass(frozen=True)
class NativeModelLimits:
    usable_tokens: int
    auto_compact_tokens: int
    input_modalities: tuple[str, ...]
    context_tokens: int


@dataclass(frozen=True)
class PreparedCodexThread:
    thread_id: str
    rollout_path: str
    model: str
    effort: str | None
    mode: str
    tool_output_token_limit: int
    boundary: ImportBoundary


def _unknown() -> TransferError:
    return TransferError("session_transfer_model_capacity_unknown")


def _positive(value: object) -> int:
    if type(value) is not int or value <= 0:
        raise _unknown()
    return value


def resolve_limits(raw_model: dict, effective_config: dict) -> NativeModelLimits:
    """Fórmulas do ModelInfo/with_config_overrides na versão conferida."""
    window = _positive(raw_model.get("context_window") if raw_model.get("context_window") is not None
                       else raw_model.get("max_context_window"))
    maximum = raw_model.get("max_context_window")
    if maximum is not None:
        maximum = _positive(maximum)
        if window > maximum:
            raise _unknown()
    configured = effective_config.get("model_context_window")
    if configured is not None:
        configured = _positive(configured)
        # Sem máximo conhecido, aumentar o valor local não prova capacidade do servidor.
        if maximum is None and configured > window:
            raise _unknown()
        window = min(configured, maximum) if maximum is not None else configured
    percent = _positive(raw_model.get("effective_context_window_percent"))
    if percent >= 100:
        raise _unknown()
    compact = effective_config.get("model_auto_compact_token_limit")
    if compact is None:
        compact = raw_model.get("auto_compact_token_limit")
    compact = min(_positive(compact), window * 9 // 10) if compact is not None else window * 9 // 10
    modalities = raw_model.get("input_modalities")
    if not isinstance(modalities, list) or not modalities or any(
            not isinstance(m, str) for m in modalities):
        raise _unknown()
    return NativeModelLimits(window * percent // 100, compact, tuple(modalities), window)


def _bytes(value: object) -> int:
    return len(json.dumps(value, ensure_ascii=False).encode("utf-8"))


def _image_count(value: object) -> int:
    if isinstance(value, dict):
        return int(value.get("type") == "input_image") + sum(_image_count(v) for v in value.values())
    return sum(_image_count(v) for v in value) if isinstance(value, (list, tuple)) else 0


async def _instruction_bytes(client: AppServerClient, started: dict, config: dict,
                             raw: dict, cwd: str, thread_id: str) -> int:
    # O app-server informa os arquivos realmente carregados; o teto de leitura não é conteúdo.
    paths = started.get("instructionSources")
    if not isinstance(paths, list):
        raise _unknown()
    model_messages = raw.get("model_messages") or {}
    if not isinstance(model_messages, dict):
        raise _unknown()
    template = model_messages.get("instructions_template") or raw.get("base_instructions")
    if not isinstance(template, str) or not template:
        raise _unknown()
    size = _bytes(model_messages or {"base_instructions": template})
    if config.get("model_instructions_file"):
        paths = [*paths, config["model_instructions_file"]]
    if any(not isinstance(path, str) or not Path(path).is_absolute() for path in paths):
        raise _unknown()
    for path in set(paths):
        try:
            size += len(await asyncio.to_thread(Path(path).read_bytes))
        except OSError as exc:
            raise _unknown() from exc
    size += _bytes({k: config.get(k) for k in ("instructions", "developer_instructions")})
    size += len(IMPORT_NOTICE.encode())
    skills = await client.request("skills/list", {"cwds": [cwd], "forceReload": True})
    entries = skills.get("data")
    if (not isinstance(entries, list) or any(not isinstance(e, dict) for e in entries)
            or not any(e.get("cwd") == cwd for e in entries)):
        raise _unknown()
    for entry in entries:
        if (entry.get("errors") or not isinstance(entry.get("skills"), list)
                or any(not isinstance(s, dict) for s in entry["skills"])):
            raise _unknown()
        size += _bytes([s for s in entry["skills"] if s.get("enabled")])
    cursor, seen = None, set()
    while True:
        params = {"threadId": thread_id}
        if cursor:
            params["cursor"] = cursor
        inventory = await client.request("mcpServerStatus/list", params)
        if not isinstance(inventory.get("data"), list):
            raise _unknown()
        for server in inventory["data"]:
            if (not isinstance(server, dict) or server.get("toolsError") or not isinstance(server.get("tools"), dict)
                    or server.get("runtimeStatus") not in {"connected", "disabled"}):
                raise _unknown()
            size += _bytes(server["tools"])
        cursor = inventory.get("nextCursor")
        if not cursor:
            break
        if not isinstance(cursor, str) or cursor in seen:
            raise _unknown()
        seen.add(cursor)
    return size


def check_budget(items: tuple[dict, ...], limits: NativeModelLimits, instructions_bytes: int) -> None:
    if "text" not in limits.input_modalities or (_image_count(items) and "image" not in limits.input_modalities):
        raise TransferError("session_transfer_model_media_unsupported")
    # Um token por byte serializado é uma estimativa conservadora para texto/JSON.
    # A reserva nativa cobre ferramentas internas/sistema/output; outra reserva igual dá continuação.
    continuation = limits.context_tokens - limits.usable_tokens
    # Imagem tem piso próprio; bytes de base64 pequeno não limitam o custo de visão.
    estimate = _bytes(items) + _image_count(items) * 10000 + instructions_bytes + continuation
    if estimate >= min(limits.usable_tokens, limits.auto_compact_tokens):
        raise TransferError("session_transfer_context_budget_exceeded")


def _identified_items(context: ImportedContext, transfer_id: str) -> tuple[dict, ...]:
    items = tuple({**item, "id": item.get("id") or
                   "import_" + hashlib.sha256(f"{transfer_id}:{index}".encode()).hexdigest()}
                  for index, item in enumerate(context.items))
    if len({item["id"] for item in items}) != len(items):
        raise TransferError("session_transfer_invalid_source_schema")
    return items


async def _inject(client: AppServerClient, thread_id: str, items: tuple[dict, ...]) -> None:
    pending = serialized_batches(items, MAX_REQUEST_BYTES)
    while pending:
        batch = pending.pop(0)
        params = {"threadId": thread_id, "items": batch}
        if len(client.request_bytes("thread/inject_items", params)) > MAX_REQUEST_BYTES:
            if len(batch) == 1:
                raise TransferError("session_transfer_source_item_too_large")
            middle = len(batch) // 2
            pending[0:0] = [batch[:middle], batch[middle:]]
            continue
        await client.request("thread/inject_items", params)


def _project(actual: object, expected: object) -> object:
    if isinstance(expected, dict) and isinstance(actual, dict):
        return {key: _project(actual.get(key), value) for key, value in expected.items()}
    if isinstance(expected, list) and isinstance(actual, list):
        if len(actual) != len(expected):
            return None
        return [_project(a, e) for a, e in zip(actual, expected)]
    return actual


def _persisted(path: Path, thread_id: str, expected: tuple[dict, ...]) -> ImportBoundary | None:
    try:
        raw = path.read_bytes()
    except FileNotFoundError:
        return None
    if not raw.endswith(b"\n"):
        return None
    rows = [json.loads(line) for line in raw.split(b"\n") if line]
    if not rows or rows[0].get("type") != "session_meta" or rows[0].get("payload", {}).get("id") != thread_id:
        raise TransferError("session_transfer_import_persistence_mismatch")
    actual = [row["payload"] for row in rows if row.get("type") == "response_item"]
    wanted = {item["id"] for item in expected}
    imported = [item for item in actual if item.get("id") in wanted]
    if len(imported) < len(expected):
        return None
    if len(imported) != len(expected) or any(_project(a, e) != e for a, e in zip(imported, expected)):
        raise TransferError("session_transfer_import_persistence_mismatch")
    # Não pode haver turno/compactação concorrente na preparação isolada.
    if any(row.get("type") == "compacted" or
           (row.get("type") == "event_msg" and row.get("payload", {}).get("type") in
            {"task_started", "user_message", "agent_message"}) for row in rows):
        raise TransferError("session_transfer_unexpected_import_turn")
    return ImportBoundary(thread_id, str(path), len(raw), tuple(item["id"] for item in expected),
                          hashlib.sha256(raw).hexdigest())


async def _wait_persisted(path: Path, thread_id: str, items: tuple[dict, ...]) -> ImportBoundary:
    deadline = asyncio.get_running_loop().time() + PERSIST_TIMEOUT
    while True:
        boundary = await asyncio.to_thread(_persisted, path, thread_id, items)
        if boundary:
            return boundary
        if asyncio.get_running_loop().time() >= deadline:
            raise TransferError("session_transfer_import_persistence_unconfirmed")
        await asyncio.sleep(0.05)


async def prepare_import(account: codex_contas.Account, cwd: str, context: ImportedContext,
                         model: str | None, effort: str | None, permission_mode: str,
                         *, transfer_id: str, collaboration_mode: str = "default") -> PreparedCodexThread:
    if collaboration_mode not in {"default", "plan"}:
        raise TransferError("session_transfer_invalid_mode")
    if permission_mode not in {name for name, *_ in MODOS}:
        raise TransferError("session_transfer_invalid_permission_mode")
    record = store.load_transfer(transfer_id)
    if (record is None or record.phase != TransferPhase.SOURCE_STOPPED or record.source is None
            or record.destination_meta is not None or record.source.digest != context.source_digest
            or record.origin_meta.get("cwd") != cwd
            or set(record.source.selected_uuids) != context.selected_uuids):
        raise TransferError("session_transfer_invalid_record")
    store.verify_source(record.source)
    items = _identified_items(context, transfer_id)
    # A conta permanece isolada; não há reconciliação, mudança de confiança ou config global.
    budget = max(1, context.max_output_bytes, *(_bytes(i.get("output")) for i in items
                   if i.get("type") == "function_call_output"))
    client = AppServerClient()
    try:
        await client.start(codex_home=account.home, cwd=cwd, tool_output_token_limit=budget,
                           session_name=record.name, session_key=record.origin_meta["key"])
        await client.request("initialize", {"clientInfo": CLIENT_INFO,
                                           "capabilities": {"experimentalApi": True}})
        version = await asyncio.to_thread(codex_appserver.versao)
        if version != SUPPORTED_VERSION:
            raise TransferError("session_transfer_codex_version_unsupported")
        config = (await client.request("config/read", {"cwd": cwd, "includeLayers": False})).get("config")
        if not isinstance(config, dict):
            raise _unknown()
        approval, sandbox = politica(permission_mode)
        params = {"cwd": cwd, "approvalPolicy": approval, "sandbox": sandbox,
                  "developerInstructions": "\n\n".join(filter(None, [config.get("developer_instructions"), IMPORT_NOTICE]))}
        if model is not None:
            params["model"] = model
        started = await client.request("thread/start", params)
        thread = started.get("thread") or {}
        thread_id, rollout = thread.get("id"), thread.get("path")
        if not isinstance(thread_id, str) or not thread_id or not isinstance(rollout, str) or not rollout:
            raise TransferError("session_transfer_invalid_native_thread")
        # Associar antes de qualquer outro RPC: uma falha posterior não deixa importação órfã.
        record = replace(record, destination_meta={"thread_id": thread_id, "rollout_path": rollout,
                         "codex_home": str(account.home.expanduser().absolute()), "codex_account": account.id,
                         "tool_output_token_limit": budget, "cwd": cwd, "permission_mode": permission_mode,
                         "transfer_id": transfer_id})
        store.save_transfer(record)
        effective_model = started.get("model")
        if not isinstance(effective_model, str) or not effective_model or (model is not None and model != effective_model):
            raise TransferError("session_transfer_native_settings_mismatch")
        if started.get("modelProvider") != (config.get("model_provider") or "openai"):
            raise TransferError("session_transfer_native_settings_mismatch")
        if effort is not None:
            await client.request("thread/settings/update", {"threadId": thread_id, "effort": effort})
        resolved = (await client.request("thread/read", {"threadId": thread_id, "includeTurns": False})).get("thread") or {}
        effective_effort = resolved.get("reasoningEffort")
        if resolved.get("model") != effective_model or (effort is not None and effective_effort != effort):
            raise TransferError("session_transfer_native_settings_mismatch")
        if collaboration_mode == "plan":
            await client.request("thread/settings/update", {"threadId": thread_id, "collaborationMode": {
                "mode": "plan", "settings": {"model": effective_model, "reasoning_effort": effective_effort,
                                              "developer_instructions": None}}})
        raw_model = await asyncio.to_thread(codex_models.raw_model, account.home, config, effective_model, version)
        limits = resolve_limits(raw_model, config)
        instructions = await _instruction_bytes(client, started, config, raw_model, cwd, thread_id)
        check_budget(items, limits, instructions)
        await _inject(client, thread_id, items)
        resolved = (await client.request("thread/read", {"threadId": thread_id, "includeTurns": False})).get("thread") or {}
        if (resolved.get("id") != thread_id or resolved.get("path") != rollout
                or resolved.get("model") != effective_model or resolved.get("reasoningEffort") != effective_effort):
            raise TransferError("session_transfer_native_settings_mismatch")
        boundary = await _wait_persisted(Path(rollout), thread_id, items)
        store.save_transfer(replace(record, destination_meta={**record.destination_meta,
                            "model": effective_model, "effort": effective_effort}, boundary=boundary))
        return PreparedCodexThread(thread_id, rollout, effective_model, effective_effort,
                                   collaboration_mode, budget, boundary)
    except codex_models.CodexRespostaInvalida:
        raise _unknown() from None
    except ConversionError as exc:
        raise TransferError(exc.code) from None
    except (TransferError, asyncio.CancelledError):
        raise
    except Exception:
        # __context__ retém a causa para inspeção privada, sem payload no texto público.
        error = TransferError("session_transfer_native_import_failed")
        raise error from None
    finally:
        await client.close()
