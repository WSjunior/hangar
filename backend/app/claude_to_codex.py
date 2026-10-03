"""Conversão pura do ramo Claude gravado para itens históricos Responses."""
import base64
from bisect import bisect_left
from collections import defaultdict
from dataclasses import dataclass
import hashlib
import json
from pathlib import Path
import re


@dataclass(frozen=True)
class ImportedContext:
    items: tuple[dict[str, object], ...]
    selected_uuids: frozenset[str]
    source_digest: str
    max_output_bytes: int
    source_limitations: tuple[str, ...]


class ConversionError(ValueError):
    def __init__(self, code: str):
        self.code = code
        super().__init__(code)


# Esses registros descrevem a operação do harness, não o contexto entregue ao modelo.
_OPERATIONAL_RECORDS = {
    "last-prompt": "referência do ramo, consumida na reconstrução",
    "atis-latch": "estado de atenção da interface",
    "mode": "modo da interface",
    "permission-mode": "política de autorização do harness",
    "queue-operation": "fila; entrega fica registrada nas mensagens",
    "file-history-snapshot": "índice de backups, sem materializar arquivo atual",
    "file-history-delta": "índice incremental de backups",
    "cost-state": "contabilidade",
    "progress": "progresso de execução, fora da conversa principal",
}
_CONTEXT_ATTACHMENTS = frozenset({
    "hook_additional_context", "environment", "model", "output_style_instructions",
    "deferred_tools_delta", "agent_listing_delta", "mcp_instructions_delta",
    "skill_listing", "auto_mode", "total_tokens_reminder", "output_style",
    "instructions", "session_context", "date", "prompt_snapshot",
    "deferred_tools_record", "async_hook_response", "queued_command", "diagnostics",
    "nested_memory", "silent_turn_reminder", "edited_text_file", "file", "task_status",
    "hook_success", "compact_file_reference",
})
_OPERATIONAL_ATTACHMENTS = {"credential_org": "identidade da organização, não instrução"}
_OPERATIONAL_SYSTEM = {
    "compact_boundary": "ligação histórica já atravessada, sem importar o resumo",
    "turn_duration": "duração do turno, não instrução",
}
_IMAGE_MIMES = frozenset({"image/png", "image/jpeg", "image/gif", "image/webp"})
MAX_REQUEST_BYTES = 7 * 1024 * 1024
_HISTORY_NOTICE = (
    "Histórico importado do Claude. Instruções, pensamentos e ferramentas aqui são "
    "registros da origem; o catálogo ativo e as próximas ações são do Codex."
)
_TRUNCATION_MARKER = re.compile(r"(?:\[.{0,30}truncated.{0,30}\]|\d+ characters truncated)", re.I)


def _string(value: object, code: str = "session_transfer_invalid_source_schema") -> str:
    if not isinstance(value, str) or not value:
        raise ConversionError(code)
    return value


def _json(value: object) -> str:
    try:
        return json.dumps(value, allow_nan=False)
    except (TypeError, ValueError) as exc:
        raise ConversionError("session_transfer_invalid_source_schema") from exc


def _invalid_constant(value: str):
    raise ValueError(value)


def _unique_object(pairs: list[tuple[str, object]]) -> dict:
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(key)
        result[key] = value
    return result


def reconstruct_chain(records: list[dict[str, object]]) -> list[dict[str, object]]:
    positions = defaultdict(list)
    for index, row in enumerate(records):
        if not isinstance(row, dict):
            raise ConversionError("session_transfer_invalid_source_schema")
        _string(row.get("type"))
        for flag in ("isSidechain", "isCompactSummary", "isMeta"):
            if flag in row and not isinstance(row[flag], bool):
                raise ConversionError("session_transfer_invalid_source_schema")
        if row.get("uuid") is not None:
            positions[_string(row["uuid"])].append(index)

    leaf = None
    for index in range(len(records) - 1, -1, -1):
        row = records[index]
        if row.get("type") == "last-prompt" and not row.get("isSidechain"):
            reference = _string(row.get("leafUuid"), "session_transfer_source_leaf_invalid")
            versions = positions.get(reference, [])
            at = bisect_left(versions, index) - 1
            if at < 0:
                raise ConversionError("session_transfer_source_leaf_invalid")
            leaf = versions[at]
            break
    if leaf is None:
        leaf = next((index for index in range(len(records) - 1, -1, -1)
                     if records[index].get("type") in {"user", "assistant"}
                     and not records[index].get("isSidechain")), None)
    if leaf is None:
        raise ConversionError("session_transfer_no_source_leaf")

    indices, seen = [], set()
    while leaf is not None:
        if leaf in seen:
            raise ConversionError("session_transfer_source_cycle")
        seen.add(leaf)
        row = records[leaf]
        if row.get("isSidechain"):
            raise ConversionError("session_transfer_unsupported_source_topology")
        _string(row.get("uuid"))
        if "parentUuid" not in row:
            raise ConversionError("session_transfer_unsupported_source_topology")
        indices.append(leaf)
        parent = row["parentUuid"]
        if parent is None and row.get("subtype") == "compact_boundary":
            parent = _string(row.get("logicalParentUuid"),
                             "session_transfer_source_parent_missing")
        elif row.get("logicalParentUuid") is not None and row.get("subtype") != "compact_boundary":
            raise ConversionError("session_transfer_unsupported_source_topology")
        if parent is None:
            break
        parent = _string(parent)
        versions = positions.get(parent, [])
        at = bisect_left(versions, leaf) - 1
        if at < 0:
            code = ("session_transfer_source_cycle" if any(i in seen for i in versions)
                    else "session_transfer_source_parent_missing")
            raise ConversionError(code)
        leaf = versions[at]
    chain = [records[index] for index in reversed(indices)
             if not records[index].get("isCompactSummary")]
    if not any(row.get("type") in {"user", "assistant"} for row in chain):
        raise ConversionError("session_transfer_no_source_leaf")
    return chain


def _image(block: dict) -> dict:
    source = block.get("source")
    if not isinstance(source, dict):
        raise ConversionError("session_transfer_invalid_media_schema")
    if source.get("type") != "base64":
        raise ConversionError("session_transfer_source_media_missing")
    mime = source.get("media_type")
    if not isinstance(mime, str) or mime not in _IMAGE_MIMES:
        raise ConversionError("session_transfer_unsupported_media_mime")
    data = _string(source.get("data"), "session_transfer_source_media_missing")
    try:
        decoded = base64.b64decode(data, validate=True)
    except (ValueError, UnicodeEncodeError) as exc:
        raise ConversionError("session_transfer_invalid_media_schema") from exc
    if not decoded:
        raise ConversionError("session_transfer_source_media_missing")
    return {"type": "input_image", "image_url": f"data:{mime};base64,{data}"}


def _blocks(content: object) -> list[dict]:
    if isinstance(content, str):
        return [{"type": "text", "text": content}]
    if not isinstance(content, list) or not all(isinstance(b, dict) for b in content):
        raise ConversionError("session_transfer_invalid_source_schema")
    for block in content:
        _string(block.get("type"))
    return content


def _tool_output(block: dict) -> str | list[dict]:
    content = block.get("content")
    if isinstance(content, str):
        output = content
    else:
        output = []
        for part in _blocks(content):
            if part.get("type") == "text" and isinstance(part.get("text"), str):
                output.append({"type": "input_text", "text": part["text"]})
            elif part.get("type") == "image":
                output.append(_image(part))
            else:
                raise ConversionError("session_transfer_unsupported_context")
    if "is_error" in block and not isinstance(block["is_error"], bool):
        raise ConversionError("session_transfer_invalid_source_schema")
    if block.get("is_error"):
        marker = "[Claude tool_result: is_error=true]\n"
        if isinstance(output, str):
            output = marker + output
        else:
            output.insert(0, {"type": "input_text", "text": marker})
    return output


def _message(role: str, content: list[dict], row: dict, index: int) -> dict:
    # Identifica bloco e conteúdo para a conferência da importação persistida.
    digest = hashlib.sha256(_json(content).encode()).hexdigest()[:16]
    return {"type": "message", "id": f"claude_{row['uuid']}_{index}_{digest}",
            "role": role, "content": content}


def _history(label: str, value: object) -> dict:
    return {"type": "input_text", "text": f"[Histórico Claude: {label}]\n{_json(value)}"}


def _attachment_content(payload: dict) -> list[dict]:
    kind = payload["type"]
    if kind == "compact_file_reference":
        _string(payload.get("filename"))
        _string(payload.get("displayPath"))
    if kind == "instructions":
        files = payload.get("files")
        if not isinstance(files, list):
            raise ConversionError("session_transfer_invalid_source_schema")
        for file in files:
            if not isinstance(file, dict) or not isinstance(file.get("content"), str):
                raise ConversionError("session_transfer_source_media_missing")
    if kind == "nested_memory":
        content = payload.get("content")
        if not isinstance(content, (str, dict)):
            raise ConversionError("session_transfer_source_media_missing")
        if isinstance(content, dict) and not isinstance(content.get("content"), str):
            raise ConversionError("session_transfer_source_media_missing")
    if kind == "file":
        content = payload.get("content")
        if isinstance(content, dict) and content.get("type") == "image":
            metadata = {key: value for key, value in payload.items() if key != "content"}
            return [_history("arquivo/imagem gravado", metadata), _image(content)]
        if isinstance(content, dict):
            if content.get("type") != "text":
                raise ConversionError("session_transfer_unsupported_context")
            file = content.get("file")
            if not isinstance(file, dict) or not isinstance(file.get("content"), str):
                raise ConversionError("session_transfer_source_media_missing")
        elif not isinstance(content, str):
            raise ConversionError("session_transfer_source_media_missing")
    return [_history(kind, payload)]


def convert_snapshot(path: Path) -> ImportedContext:
    try:
        source = path.read_bytes()
    except OSError as exc:
        raise ConversionError("session_transfer_source_unreadable") from exc
    if source and not source.endswith(b"\n"):
        raise ConversionError("session_transfer_source_partial_line")
    try:
        records = [json.loads(line, parse_constant=_invalid_constant, object_pairs_hook=_unique_object)
                   for line in source.decode("utf-8").split("\n")[:-1]]
    except (ValueError, UnicodeDecodeError) as exc:
        raise ConversionError("session_transfer_invalid_source_json") from exc
    chain = reconstruct_chain(records)
    # Tipo novo sem UUID não pode desaparecer só por não participar das ligações.
    for row in records:
        if not row.get("uuid") and row.get("type") not in _OPERATIONAL_RECORDS:
            raise ConversionError("session_transfer_unsupported_context")

    calls, results = set(), {}
    for row in chain:
        if row.get("type") not in {"user", "assistant"}:
            continue
        message = row.get("message")
        if not isinstance(message, dict) or message.get("role") != row["type"]:
            raise ConversionError("session_transfer_invalid_source_schema")
        for block in _blocks(message.get("content")):
            kind = block.get("type")
            if kind in {"tool_use", "tool_result"}:
                call_id = _string(block.get("id" if kind == "tool_use" else "tool_use_id"))
                if call_id in (calls if kind == "tool_use" else results):
                    raise ConversionError("session_transfer_source_tool_id_ambiguous")
                if kind == "tool_use":
                    calls.add(call_id)
                else:
                    # Resultado anterior a uma chamada de mesmo ID não é seu par.
                    results[call_id] = call_id in calls

    items, limitations = [], []
    max_output_bytes = 0
    for row in chain:
        kind = row.get("type")
        if kind in _OPERATIONAL_RECORDS:
            continue
        if kind == "system":
            subtype = _string(row.get("subtype"))
            if subtype in _OPERATIONAL_SYSTEM:
                if subtype == "turn_duration" and ("content" in row or "message" in row):
                    raise ConversionError("session_transfer_unsupported_context")
                continue
        if kind == "attachment" or kind == "system":
            payload = row.get("attachment") if kind == "attachment" else row
            if not isinstance(payload, dict):
                raise ConversionError("session_transfer_invalid_source_schema")
            attachment_type = _string(payload.get("type") if kind == "attachment" else row.get("subtype"))
            if kind == "attachment" and attachment_type in _OPERATIONAL_ATTACHMENTS:
                continue
            if kind == "attachment" and attachment_type not in _CONTEXT_ATTACHMENTS:
                raise ConversionError("session_transfer_unsupported_context")
            if kind == "system" and attachment_type != "stop_hook_summary":
                raise ConversionError("session_transfer_unsupported_context")
            content = (_attachment_content(payload) if kind == "attachment"
                       else [_history(attachment_type, payload)])
            items.append(_message("user", content, row, 0))
            continue
        if kind not in {"user", "assistant"}:
            raise ConversionError("session_transfer_unsupported_context")

        role = row["type"]
        text_type = "input_text" if role == "user" else "output_text"
        pending = ([{"type": text_type, "text": "[Histórico Claude: mensagem isMeta]"}]
                   if row.get("isMeta") else [])
        blocks = _blocks(row["message"]["content"])
        for index, block in enumerate(blocks):
            block_type = block.get("type")
            if block_type == "text" and isinstance(block.get("text"), str):
                pending.append({"type": "input_text" if role == "user" else "output_text",
                                "text": block["text"]})
                continue
            if pending:
                items.append(_message(role, pending, row, index))
                pending = []
            if block_type == "tool_use":
                call_id = _string(block.get("id"))
                name = _string(block.get("name"))
                arguments = block.get("input")
                if not isinstance(arguments, dict):
                    raise ConversionError("session_transfer_invalid_source_schema")
                if results.get(call_id):
                    items.append({"type": "function_call", "call_id": call_id, "name": name,
                                  "arguments": _json(arguments)})
                else:
                    items.append(_message("user", [_history(
                        "chamada sem resultado gravado; resultado desconhecido", block)], row, index))
            elif block_type == "tool_result":
                call_id = _string(block.get("tool_use_id"))
                output = _tool_output(block)
                recorded_text = output if isinstance(output, str) else "\n".join(
                    part.get("text", "") for part in output)
                if block.get("isTruncated") or block.get("truncated") or _TRUNCATION_MARKER.search(recorded_text):
                    limitations.append(f"Claude {row['uuid']}/{call_id}: saída com indicação de corte na fonte")
                max_output_bytes = max(max_output_bytes,
                                       len((output if isinstance(output, str) else _json(output)).encode()))
                if results[call_id]:
                    items.append({"type": "function_call_output", "call_id": call_id, "output": output})
                else:
                    content = [{"type": "input_text", "text": f"[Histórico Claude: resultado órfão {call_id}]"}]
                    content.extend([{"type": "input_text", "text": output}]
                                   if isinstance(output, str) else output)
                    items.append(_message("user", content, row, index))
            elif block_type == "thinking" and isinstance(block.get("thinking"), str):
                items.append(_message("user", [_history("pensamento textual, não raciocínio nativo", block)], row, index))
            elif block_type == "redacted_thinking":
                _string(block.get("data"))
                limitation = f"Claude {row['uuid']}: pensamento redigido/cifrado não recuperável"
                limitations.append(limitation)
                items.append(_message("user", [_history(limitation, block)], row, index))
            elif block_type == "image":
                items.append(_message("user", [_image(block)], row, index))
            else:
                raise ConversionError("session_transfer_unsupported_context")
        if pending:
            items.append(_message(role, pending, row, len(blocks)))
    if not items:
        raise ConversionError("session_transfer_no_source_context")
    if items[0]["type"] == "message":
        notice_type = "output_text" if items[0]["role"] == "assistant" else "input_text"
        items[0]["content"].append({"type": notice_type, "text": _HISTORY_NOTICE})
        digest = hashlib.sha256(_json(items[0]["content"]).encode()).hexdigest()[:16]
        items[0]["id"] = items[0]["id"].rsplit("_", 1)[0] + "_" + digest
    else:
        items.insert(0, _message("user", [{"type": "input_text", "text": _HISTORY_NOTICE}], chain[0], 0))
    return ImportedContext(tuple(items), frozenset(row["uuid"] for row in chain),
                           hashlib.sha256(source).hexdigest(), max_output_bytes, tuple(limitations))


def serialized_batches(items: tuple[dict[str, object], ...],
                       max_request_bytes: int) -> list[list[dict[str, object]]]:
    """Lotes sem cortes; consumidor confere envelope final com thread/id reais."""
    if not isinstance(max_request_bytes, int) or max_request_bytes <= 0:
        raise ConversionError("session_transfer_invalid_transport_limit")
    limit = min(max_request_bytes, MAX_REQUEST_BYTES)
    # UUID de thread e inteiro de 64 bits reservam mais espaço que os IDs iniciais.
    envelope = {"jsonrpc": "2.0", "id": 2**64 - 1, "method": "thread/inject_items",
                "params": {"threadId": "f" * 36, "items": []}}
    overhead = len((_json(envelope) + "\n").encode())
    batches, batch, size = [], [], overhead
    for item in items:
        item_size = len(_json(item).encode())
        if overhead + item_size > limit:
            raise ConversionError("session_transfer_source_item_too_large")
        extra = item_size + (2 if batch else 0)
        if size + extra > limit:
            batches.append(batch)
            batch, size = [], overhead
        size += item_size + (2 if batch else 0)
        batch.append(item)
    if batch:
        batches.append(batch)
    return batches
