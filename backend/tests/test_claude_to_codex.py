import hashlib
import json
from pathlib import Path

import pytest

from app.claude_to_codex import (
    ConversionError,
    MAX_REQUEST_BYTES,
    convert_snapshot,
    reconstruct_chain,
    serialized_batches,
)


FIXTURES = Path(__file__).parent / "fixtures" / "claude_to_codex"


def fixture_path(name):
    return FIXTURES / name


def fixture_records(name):
    return [json.loads(line) for line in fixture_path(name).read_text().splitlines()]


def full_output_from_fixture():
    return "".join(f"line-{n:05d}|\n" for n in range(12000))


def save_records(tmp_path, records):
    path = tmp_path / "snapshot.jsonl"
    path.write_text("".join(json.dumps(row) + "\n" for row in records))
    return path


def historical_text(context):
    return "\n".join(part.get("text", "") for item in context.items
                     for part in item.get("content", []))


def test_reconstructs_originals_across_compaction():
    records = fixture_records("compacted_branch.jsonl")
    chain = reconstruct_chain(records)
    ids = [r["uuid"] for r in chain if r["type"] in {"user", "assistant"}]
    assert ids == ["u0", "a0", "u1", "a1"]
    assert all(not r.get("isCompactSummary") for r in chain)
    assert chain[1]["message"]["content"] == "resposta antiga"
    assert any(r["uuid"] == "instructions" for r in chain)


def test_explicit_leaf_is_prioritized_over_later_fork_and_duplicate():
    records = fixture_records("compacted_branch.jsonl")
    assert reconstruct_chain(records)[-1]["uuid"] == "a1"
    assert records[-2]["uuid"] == "rejected"


def test_fallback_uses_last_main_message_without_leaf_metadata():
    records = fixture_records("compacted_branch.jsonl")
    records = [r for r in records if r["type"] != "last-prompt"]
    assert [r["uuid"] for r in reconstruct_chain(records)] == ["u0", "rejected"]


def test_parent_version_must_precede_child():
    records = fixture_records("mixed_blocks.jsonl")
    duplicate = json.loads(json.dumps(records[0]))
    duplicate["message"]["content"][0]["text"] = "versão posterior"
    records.append(duplicate)
    records.append({"type": "last-prompt", "leafUuid": "mixed-after"})
    assert reconstruct_chain(records)[0]["message"]["content"][0]["text"] == "antes"


@pytest.mark.parametrize("fixture,code", [
    ("missing_parent.jsonl", "session_transfer_source_parent_missing"),
    ("cycle.jsonl", "session_transfer_source_cycle"),
])
def test_invalid_links_are_errors(fixture, code):
    with pytest.raises(ConversionError) as error:
        reconstruct_chain(fixture_records(fixture))
    assert error.value.code == code


@pytest.mark.parametrize("change,code", [
    ("missing", "session_transfer_source_leaf_invalid"),
    ("side", "session_transfer_unsupported_source_topology"),
    (None, "session_transfer_source_leaf_invalid"),
])
def test_invalid_explicit_leaf_never_falls_back(change, code):
    records = fixture_records("compacted_branch.jsonl")
    records.append({"type": "last-prompt", "leafUuid": change})
    with pytest.raises(ConversionError) as error:
        reconstruct_chain(records)
    assert error.value.code == code


def test_unknown_topology_and_missing_leaf_fail():
    records = fixture_records("mixed_blocks.jsonl")
    del records[0]["parentUuid"]
    with pytest.raises(ConversionError) as error:
        reconstruct_chain(records)
    assert error.value.code == "session_transfer_unsupported_source_topology"
    with pytest.raises(ConversionError) as error:
        reconstruct_chain([])
    assert error.value.code == "session_transfer_no_source_leaf"


def test_preserves_mixed_order_and_full_tool_result():
    ctx = convert_snapshot(fixture_path("mixed_blocks.jsonl"))
    kinds = [item["type"] for item in ctx.items]
    assert kinds == ["message", "function_call", "function_call_output", "message"]
    assert ctx.items[0]["content"][0]["text"] == "antes"
    assert json.loads(ctx.items[1]["arguments"]) == {
        "file_path": "/tmp/example.txt", "offset": 17, "limit": 80,
    }
    assert ctx.items[1]["call_id"] == ctx.items[2]["call_id"] == "read-1"
    assert ctx.items[2]["output"] == full_output_from_fixture()
    assert ctx.items[3]["content"][0]["text"] == "depois"
    assert ctx.max_output_bytes == len(full_output_from_fixture().encode())
    assert ctx.source_digest == hashlib.sha256(fixture_path("mixed_blocks.jsonl").read_bytes()).hexdigest()
    assert ctx.selected_uuids == frozenset({"mixed-call", "mixed-result", "mixed-after"})
    assert ctx == convert_snapshot(fixture_path("mixed_blocks.jsonl"))


def test_interleaved_blocks_keep_order_with_parallel_tools(tmp_path):
    records = fixture_records("mixed_blocks.jsonl")
    records[0]["message"]["content"] += [
        {"type": "text", "text": "entre chamadas"},
        {"type": "tool_use", "id": "read-2", "name": "Read", "input": {"file_path": "/tmp/second"}},
    ]
    records[1]["message"]["content"] += [
        {"type": "text", "text": "entre resultados"},
        {"type": "tool_result", "tool_use_id": "read-2", "content": "segundo resultado"},
    ]
    ctx = convert_snapshot(save_records(tmp_path, records))
    assert [item["type"] for item in ctx.items] == [
        "message", "function_call", "message", "function_call",
        "function_call_output", "message", "function_call_output", "message",
    ]
    assert ctx.items[2]["content"][0]["text"] == "entre chamadas"
    assert ctx.items[5]["content"][0]["text"] == "entre resultados"
    assert ctx.items[6]["output"] == "segundo resultado"


def test_instructions_meta_and_origin_are_preserved_as_history():
    ctx = convert_snapshot(fixture_path("compacted_branch.jsonl"))
    text = historical_text(ctx)
    assert "regra hist" in text
    assert "meta preservada" in text and "isMeta" in text
    assert "catálogo ativo" in text and "Claude" in text
    assert "resumo que não entra" not in text
    assert "fork rejeitado" not in text and "duplicata posterior" not in text
    assert "instructions" in ctx.selected_uuids


def test_meta_marker_survives_text_flushed_before_tool(tmp_path):
    records = fixture_records("mixed_blocks.jsonl")
    records[0]["isMeta"] = True
    ctx = convert_snapshot(save_records(tmp_path, records))
    assert "isMeta" in ctx.items[0]["content"][0]["text"]
    assert ctx.items[0]["content"][1]["text"] == "antes"
    assert ctx.items[1]["type"] == "function_call"


@pytest.mark.parametrize("payload", [
    {"type": "hook_additional_context", "content": ["contexto artificial exclusivo"], "hookEvent": "Stop"},
    {"type": "environment", "snapshot": {"cwd": "/tmp/contexto artificial exclusivo"}},
    {"type": "model", "identity": "artificial", "text": "contexto artificial exclusivo"},
    {"type": "output_style_instructions", "style": {"instructions": "contexto artificial exclusivo"}},
    {"type": "deferred_tools_delta", "addedLines": ["contexto artificial exclusivo"], "addedNames": ["Read"]},
    {"type": "agent_listing_delta", "addedLines": ["contexto artificial exclusivo"], "addedTypes": ["worker"]},
    {"type": "mcp_instructions_delta", "addedBlocks": ["contexto artificial exclusivo"], "addedNames": ["example"]},
    {"type": "skill_listing", "content": "contexto artificial exclusivo", "names": ["example"], "skillCount": 1},
    {"type": "auto_mode", "bashFirstSteer": "contexto artificial exclusivo", "bypass": False},
    {"type": "total_tokens_reminder", "text": "contexto artificial exclusivo"},
    {"type": "output_style", "style": "contexto artificial exclusivo", "turnReminder": True},
    {"type": "session_context", "context": "contexto artificial exclusivo"},
    {"type": "date", "date": "2026-10-03", "changed": "contexto artificial exclusivo"},
    {"type": "prompt_snapshot", "systemPrompt": ["contexto artificial exclusivo"], "tools": [{"name": "Read"}]},
    {"type": "deferred_tools_record", "entries": [{"name": "Read", "description": "contexto artificial exclusivo"}]},
    {"type": "async_hook_response", "response": "contexto artificial exclusivo", "hookEvent": "Stop", "exitCode": 0},
    {"type": "queued_command", "prompt": [{"type": "text", "text": "contexto artificial exclusivo"}], "source_uuid": "old"},
    {"type": "diagnostics", "files": [{"path": "/tmp/file", "diagnostic": "contexto artificial exclusivo"}], "isNew": True},
    {"type": "silent_turn_reminder", "text": "contexto artificial exclusivo"},
    {"type": "edited_text_file", "filename": "/tmp/file", "snippet": "contexto artificial exclusivo"},
    {"type": "task_status", "taskId": "old", "status": "completed", "deltaSummary": "contexto artificial exclusivo"},
    {"type": "hook_success", "hookEvent": "Stop", "exitCode": 0, "stdout": "contexto artificial exclusivo"},
])
def test_known_context_attachment_never_becomes_active_tool(tmp_path, payload):
    records = fixture_records("compacted_branch.jsonl")
    records[4]["attachment"] = payload
    ctx = convert_snapshot(save_records(tmp_path, records))
    text = historical_text(ctx)
    assert payload["type"] in text and "contexto artificial exclusivo" in text
    assert all(item["type"] == "message" for item in ctx.items)


@pytest.mark.parametrize("payload", [
    {"type": "file", "filename": "/tmp/old.txt", "content": {"type": "text", "file": {
        "filePath": "/tmp/old.txt", "content": "bytes antigos", "numLines": 1,
        "startLine": 1, "totalLines": 1}}},
    {"type": "nested_memory", "path": "/tmp/old.md", "content": {
        "path": "/tmp/old.md", "type": "project", "content": "bytes antigos",
        "contentDiffersFromDisk": True}},
])
def test_recorded_historical_file_bytes_are_preserved(tmp_path, payload):
    records = fixture_records("compacted_branch.jsonl")
    records[4]["attachment"] = payload
    ctx = convert_snapshot(save_records(tmp_path, records))
    assert "bytes antigos" in historical_text(ctx)
    assert "/tmp/old" in historical_text(ctx)


def test_image_inside_file_attachment_is_native(tmp_path):
    records = fixture_records("compacted_branch.jsonl")
    image = fixture_records("media_and_thinking.jsonl")[0]["message"]["content"][0]
    records[4]["attachment"] = {"type": "file", "filename": "/tmp/old.png", "content": image}
    ctx = convert_snapshot(save_records(tmp_path, records))
    images = [block for item in ctx.items for block in item.get("content", [])
              if block["type"] == "input_image"]
    assert len(images) == 1
    assert images[0]["image_url"].endswith(image["source"]["data"])


@pytest.mark.parametrize("payload", [
    {"type": "instructions", "files": [{"path": "/tmp/old.md"}]},
    {"type": "file", "filename": "/tmp/old.txt"},
    {"type": "file", "content": {"type": "text", "file": {"filePath": "/tmp/old.txt"}}},
    {"type": "nested_memory", "content": {"path": "/tmp/old.md"}},
])
def test_missing_attachment_bytes_do_not_become_empty_context(tmp_path, payload):
    records = fixture_records("compacted_branch.jsonl")
    records[4]["attachment"] = payload
    with pytest.raises(ConversionError) as error:
        convert_snapshot(save_records(tmp_path, records))
    assert error.value.code == "session_transfer_source_media_missing"


def test_operational_records_are_explicitly_ignored(tmp_path):
    records = fixture_records("compacted_branch.jsonl")
    records[4]["attachment"] = {"type": "credential_org", "organizationUuid": "operational-org"}
    for kind in ("atis-latch", "mode", "permission-mode", "queue-operation",
                 "file-history-snapshot", "file-history-delta", "cost-state", "progress"):
        records.append({"type": kind, "operational": "não faz parte do contexto"})
    ctx = convert_snapshot(save_records(tmp_path, records))
    assert "operational-org" not in historical_text(ctx)
    assert "parte do contexto" not in historical_text(ctx)


def test_unpaired_calls_and_orphan_outputs_are_explicit_history():
    ctx = convert_snapshot(fixture_path("unpaired_tools.jsonl"))
    assert all(item["type"] == "message" for item in ctx.items)
    text = historical_text(ctx)
    assert "resultado desconhecido" in text and "missing-output" in text
    assert "printf teste" in text
    assert "órfão missing-call" in text and "saída órfã integral" in text
    assert "is_error=true" in text
    assert "aborted" not in text


def test_orphan_before_same_id_call_does_not_form_false_pair(tmp_path):
    records = fixture_records("unpaired_tools.jsonl")
    records[0]["message"]["content"] = [{"type": "tool_result", "tool_use_id": "same", "content": "early"}]
    records[1]["message"]["content"] = [{"type": "tool_use", "id": "same", "name": "Read", "input": {}}]
    ctx = convert_snapshot(save_records(tmp_path, records))
    assert all(item["type"] == "message" for item in ctx.items)
    assert "resultado desconhecido" in historical_text(ctx)
    assert "early" in historical_text(ctx)


def test_duplicate_tool_ids_fail(tmp_path):
    records = fixture_records("mixed_blocks.jsonl")
    records[0]["message"]["content"].append(records[0]["message"]["content"][1])
    with pytest.raises(ConversionError) as error:
        convert_snapshot(save_records(tmp_path, records))
    assert error.value.code == "session_transfer_source_tool_id_ambiguous"


def test_existing_source_truncation_is_identified_without_new_cut(tmp_path):
    records = fixture_records("mixed_blocks.jsonl")
    result = records[1]["message"]["content"][0]
    result["content"] = "conteúdo gravado\n[...truncated...]"
    ctx = convert_snapshot(save_records(tmp_path, records))
    assert ctx.items[2]["output"] == result["content"]
    assert len(ctx.source_limitations) == 1
    assert "indicação de corte" in ctx.source_limitations[0]


def test_images_multimodal_error_and_thinking_keep_recorded_content():
    ctx = convert_snapshot(fixture_path("media_and_thinking.jsonl"))
    expected_image = fixture_records("media_and_thinking.jsonl")[0]["message"]["content"][0]["source"]
    url = f"data:{expected_image['media_type']};base64,{expected_image['data']}"
    assert ctx.items[0]["content"][0] == {"type": "input_image", "image_url": url}
    output = next(item["output"] for item in ctx.items if item["type"] == "function_call_output")
    assert output == [
        {"type": "input_text", "text": "[Claude tool_result: is_error=true]\n"},
        {"type": "input_text", "text": "texto completo"},
        {"type": "input_image", "image_url": url},
    ]
    text = historical_text(ctx)
    assert "pensamento gravado" in text and "assinatura-original" in text
    assert "conte" in text and "redigido/cifrado" in text
    assert all(item["type"] != "reasoning" for item in ctx.items)
    assert len(ctx.source_limitations) == 1


def test_does_not_silently_drop_unknown_context():
    with pytest.raises(ConversionError) as error:
        convert_snapshot(fixture_path("unknown_context.jsonl"))
    assert error.value.code == "session_transfer_unsupported_context"


@pytest.mark.parametrize("record", [
    {"type": "future_context", "content": "novo contexto"},
    {"type": "attachment", "attachment": {"type": "instructions", "content": "sem ligação"}},
])
def test_unlinked_unknown_context_cannot_disappear(tmp_path, record):
    records = fixture_records("mixed_blocks.jsonl") + [record]
    with pytest.raises(ConversionError) as error:
        convert_snapshot(save_records(tmp_path, records))
    assert error.value.code == "session_transfer_unsupported_context"


@pytest.mark.parametrize("raw,code", [
    (b'{"type":"user"}', "session_transfer_source_partial_line"),
    (b'{broken}\n', "session_transfer_invalid_source_json"),
    (b'\xff\n', "session_transfer_invalid_source_json"),
    (b'{"number":NaN}\n', "session_transfer_invalid_source_json"),
    (b'{"type":"user","type":"assistant"}\n', "session_transfer_invalid_source_json"),
    (b'[]\n', "session_transfer_invalid_source_schema"),
])
def test_snapshot_requires_complete_valid_jsonl(tmp_path, raw, code):
    path = tmp_path / "bad.jsonl"
    path.write_bytes(raw)
    with pytest.raises(ConversionError) as error:
        convert_snapshot(path)
    assert error.value.code == code


@pytest.mark.parametrize("field,value,code", [
    ("media_type", "application/pdf", "session_transfer_unsupported_media_mime"),
    ("data", "not-base64", "session_transfer_invalid_media_schema"),
    ("data", "", "session_transfer_source_media_missing"),
    ("type", "url", "session_transfer_source_media_missing"),
    ("media_type", [], "session_transfer_unsupported_media_mime"),
])
def test_missing_or_invalid_media_is_error(tmp_path, field, value, code):
    records = fixture_records("media_and_thinking.jsonl")
    records[0]["message"]["content"][0]["source"][field] = value
    with pytest.raises(ConversionError) as error:
        convert_snapshot(save_records(tmp_path, records))
    assert error.value.code == code


def test_missing_historical_file_never_reads_current_disk(tmp_path):
    current = tmp_path / "current.txt"
    current.write_text("versão de agora")
    records = fixture_records("compacted_branch.jsonl")
    records[4]["attachment"] = {"type": "compact_file_reference", "filename": str(current)}
    with pytest.raises(ConversionError) as error:
        convert_snapshot(save_records(tmp_path, records))
    assert error.value.code == "session_transfer_source_media_missing"


@pytest.mark.parametrize("field,value", [
    ("isSidechain", "false"), ("isMeta", 1), ("isCompactSummary", []), ("type", []),
])
def test_invalid_record_schema_has_defined_error(tmp_path, field, value):
    records = fixture_records("mixed_blocks.jsonl")
    records[0][field] = value
    with pytest.raises(ConversionError) as error:
        convert_snapshot(save_records(tmp_path, records))
    assert error.value.code == "session_transfer_invalid_source_schema"


def artificial_items_with_unicode():
    return tuple({"type": "message", "role": "user", "content": [
        {"type": "input_text", "text": ("ação 😀 " * 20) + str(n)}
    ]} for n in range(8))


def large_indivisible_item():
    return {"type": "message", "role": "user", "content": [
        {"type": "input_text", "text": "x" * 10000}
    ]}


def request_bytes(batch):
    return len((json.dumps({"jsonrpc": "2.0", "id": 2**64 - 1,
                           "method": "thread/inject_items", "params": {
                               "threadId": "f" * 36, "items": batch}}) + "\n").encode())


def test_batching_keeps_all_items_and_rejects_indivisible_item():
    items = artificial_items_with_unicode()
    batches = serialized_batches(items, max_request_bytes=2048)
    assert [item for batch in batches for item in batch] == list(items)
    assert len(batches) > 1
    assert all(request_bytes(batch) <= 2048 for batch in batches)
    with pytest.raises(ConversionError) as error:
        serialized_batches((large_indivisible_item(),), max_request_bytes=512)
    assert error.value.code == "session_transfer_source_item_too_large"


def test_batching_measures_escaped_unicode_and_exact_envelope():
    item = artificial_items_with_unicode()[0]
    exact = request_bytes([item])
    assert serialized_batches((item,), exact) == [[item]]
    with pytest.raises(ConversionError):
        serialized_batches((item,), exact - 1)
    assert exact > len(json.dumps([item], ensure_ascii=False).encode())


def test_batching_media_and_empty_history():
    image = {"type": "input_image", "image_url": "data:image/png;base64," + "AAAA" * 80}
    items = artificial_items_with_unicode() + ({"type": "function_call_output", "call_id": "img",
                                               "output": [image]},)
    batches = serialized_batches(items, 2048)
    assert [item for batch in batches for item in batch] == list(items)
    assert all(request_bytes(batch) <= 2048 for batch in batches)
    assert serialized_batches((), 2048) == []
    with pytest.raises(ConversionError) as error:
        serialized_batches(items, 0)
    assert error.value.code == "session_transfer_invalid_transport_limit"


def test_batching_cannot_raise_confirmed_transport_ceiling():
    huge = {"type": "message", "role": "user", "content": [
        {"type": "input_text", "text": "x" * MAX_REQUEST_BYTES}
    ]}
    with pytest.raises(ConversionError) as error:
        serialized_batches((huge,), MAX_REQUEST_BYTES * 2)
    assert error.value.code == "session_transfer_source_item_too_large"
