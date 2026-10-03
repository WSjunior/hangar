"""Importação sem turno; os envelopes precisam existir completos no arquivo da thread."""
import asyncio
import hashlib
import json
from pathlib import Path
import uuid

import pytest

from app import codex_contas, conversation_transfer as store
from app.claude_to_codex import convert_snapshot
from app.conversation_transfer import ConversationSource, TransferPhase, TransferRecord
from app.adapters.codex import transfer
from app.adapters.codex.appserver import AppServerClient
from app.adapters.codex import sessions


MODEL = {"slug": "test-model", "context_window": 1000000, "max_context_window": 1000000,
         "effective_context_window_percent": 95, "auto_compact_token_limit": 900000,
         "input_modalities": ["text", "image"], "model_messages": {"instructions_template": "native"}}


@pytest.fixture
def fake_client(tmp_path, monkeypatch):
    monkeypatch.setattr(store, "_base", lambda: tmp_path / "transfers")
    monkeypatch.setattr(sessions, "_dir", lambda: tmp_path / "sidecars")
    source = tmp_path / "claude.jsonl"
    records = [json.loads(line) for line in (Path(__file__).parent / "fixtures" / "claude_to_codex" /
                                            "mixed_blocks.jsonl").read_text().splitlines()]
    # Exercita também escapes no wire, sem inferir versão atual de arquivo.
    records[0]["message"]["content"][0]["text"] += " ação\u2028🙂"
    source.write_text("".join(json.dumps(row) + "\n" for row in records))
    context = convert_snapshot(source)
    record = TransferRecord(str(uuid.uuid4()), "session", "k:key", TransferPhase.SOURCE_STOPPED,
                            ConversationSource(str(source), "claude", context.source_digest,
                                               tuple(sorted(context.selected_uuids))),
                            {"name": "session", "key": "key", "cwd": str(tmp_path)}, None, None, None)
    store.save_transfer(record)

    class FakeClient(AppServerClient):
        def __init__(self):
            super().__init__()
            self.account = codex_contas.Account("test", tmp_path / "codex", False)
            self.record, self.context = record, context
            self.calls, self.injected = [], []
            self.path = tmp_path / "rollout.jsonl"
            self.thread_id = str(uuid.uuid4())
            self.effort = "medium"
            self.mode = "default"
            self.config = {}
            self.finished = False
            self.corrupt = None
            self.fail_method = None

        async def start(self, **kwargs):
            self.start_args = kwargs

        async def request(self, method, params, timeout=30):
            self._next_id += 1
            self.calls.append((method, params))
            if self.fail_method == method:
                raise ConnectionError("payload privado não deve aparecer")
            if method == "initialize":
                assert params["clientInfo"] == transfer.CLIENT_INFO
                return {}
            if method == "config/read":
                return {"config": self.config}
            if method == "thread/start":
                self.path.write_text(json.dumps({"type": "session_meta", "payload": {"id": self.thread_id}}) + "\n")
                return {"thread": self.thread(), "model": "test-model", "reasoningEffort": self.effort,
                        "modelProvider": "openai", "instructionSources": []}
            if method == "thread/settings/update":
                self.effort = params.get("effort", self.effort)
                self.mode = params.get("collaborationMode", {}).get("mode", self.mode)
                return {}
            if method == "thread/read":
                return {"thread": self.thread()}
            if method == "skills/list":
                return {"data": [{"cwd": str(tmp_path), "skills": [], "errors": []}]}
            if method == "mcpServerStatus/list":
                return {"data": [{"name": "hangar", "tools": {"send": {"inputSchema": {"type": "object"}}},
                                  "runtimeStatus": "connected", "toolsError": None}], "nextCursor": None}
            if method == "thread/inject_items":
                associated = store.transfer_for_thread(str(self.account.home), self.thread_id)
                assert associated.id == record.id and associated.boundary is None
                assert associated.destination_meta["tool_output_token_limit"] >= context.max_output_bytes
                self.injected.extend(params["items"])
                with self.path.open("ab") as output:
                    for item in params["items"]:
                        changed = dict(item)
                        if self.corrupt == "truncate" and item["type"] == "function_call_output":
                            changed["output"] = str(item["output"])[:4000]
                        if self.corrupt == "missing" and item == params["items"][-1]:
                            continue
                        line = json.dumps({"type": "response_item", "payload": changed}).encode()
                        output.write(line + (b"" if self.corrupt == "partial" else b"\n"))
                return {}
            raise AssertionError(method)

        def thread(self):
            return {"id": self.thread_id, "path": str(self.path), "model": "test-model",
                    "reasoningEffort": self.effort, "turns": [], "status": {"type": "idle"}}

        async def close(self):
            self.finished = True

    fake = FakeClient()
    monkeypatch.setattr(transfer, "AppServerClient", lambda: fake)
    monkeypatch.setattr(transfer.codex_appserver, "versao", lambda: "0.159.3")
    monkeypatch.setattr(transfer.codex_models, "raw_model", lambda *args: dict(MODEL))
    return fake


async def prepare(fake, **kwargs):
    return await transfer.prepare_import(fake.account, fake.record.origin_meta["cwd"], fake.context,
                                         kwargs.pop("model", "test-model"), kwargs.pop("effort", "low"),
                                         "Full Access", transfer_id=fake.record.id, **kwargs)


async def test_native_import_preserves_long_output_and_does_not_start_turn(fake_client):
    fake = fake_client
    prepared = await prepare(fake)
    assert "turn/start" not in [method for method, _ in fake.calls]
    assert fake.finished and not sessions._dir().exists()
    assert fake.start_args["codex_home"] == fake.account.home
    assert fake.start_args["session_key"] == "key" and fake.start_args["session_name"] == "session"
    assert fake.start_args["tool_output_token_limit"] >= fake.context.max_output_bytes
    call = next(item for item in fake.injected if item["type"] == "function_call")
    assert call["name"] == "Read"
    assert json.loads(call["arguments"]) == {"file_path": "/tmp/example.txt", "offset": 17, "limit": 80}
    output = next(item for item in fake.injected if item["type"] == "function_call_output")
    original = next(item for item in fake.context.items if item["type"] == "function_call_output")
    assert output["output"] == original["output"] and len(output["output"]) == 144000
    assert any(transfer._image_count(item) for item in fake.injected)
    assert len(fake.injected) == len(fake.context.items)
    assert all(transfer._project(imported, original) == original
               for imported, original in zip(fake.injected, fake.context.items))
    rows = [json.loads(line) for line in fake.path.read_bytes().split(b"\n") if line]
    assert [row["payload"] for row in rows[1:]] == fake.injected
    assert prepared.boundary.min_offset == fake.path.stat().st_size
    assert prepared.boundary.prefix_digest == hashlib.sha256(fake.path.read_bytes()).hexdigest()
    assert prepared.boundary.imported_item_ids == tuple(item["id"] for item in fake.injected)
    assert store.load_transfer(fake.record.id).phase == TransferPhase.SOURCE_STOPPED
    assert store.load_transfer(fake.record.id).boundary == prepared.boundary


async def test_default_model_comes_from_native_start_and_plan_uses_settings(fake_client):
    prepared = await prepare(fake_client, model=None, effort=None, collaboration_mode="plan")
    assert prepared.model == "test-model" and prepared.effort == "medium" and prepared.mode == "plan"
    params = next(p for method, p in fake_client.calls if method == "thread/start")
    assert "model" not in params
    assert fake_client.mode == "plan"


@pytest.mark.parametrize("corruption,code", [
    ("truncate", "session_transfer_import_persistence_mismatch"),
    ("partial", "session_transfer_import_persistence_unconfirmed"),
    ("missing", "session_transfer_import_persistence_unconfirmed"),
])
async def test_persistence_requires_complete_content_and_physical_lf(fake_client, monkeypatch, corruption, code):
    fake_client.corrupt = corruption
    monkeypatch.setattr(transfer, "PERSIST_TIMEOUT", 0)
    with pytest.raises(transfer.TransferError) as error:
        await prepare(fake_client)
    assert error.value.code == code and fake_client.finished
    assert store.load_transfer(fake_client.record.id).boundary is None


async def test_failed_injection_keeps_association_without_publishing(fake_client):
    fake_client.fail_method = "thread/inject_items"
    with pytest.raises(transfer.TransferError) as error:
        await prepare(fake_client)
    assert "payload privado" not in str(error.value)
    record = store.load_transfer(fake_client.record.id)
    assert record.destination_meta["thread_id"] == fake_client.thread_id
    assert record.boundary is None and fake_client.finished
    assert not sessions._dir().exists()


async def test_unknown_capacity_fails_before_injection(fake_client, monkeypatch):
    monkeypatch.setattr(transfer.codex_models, "raw_model", lambda *a: {"slug": "test-model"})
    with pytest.raises(transfer.TransferError, match="capacity_unknown"):
        await prepare(fake_client)
    assert not fake_client.injected and fake_client.finished


async def test_unsupported_media_fails_before_injection(fake_client, monkeypatch):
    monkeypatch.setattr(transfer.codex_models, "raw_model", lambda *a: {**MODEL, "input_modalities": ["text"]})
    with pytest.raises(transfer.TransferError, match="media_unsupported"):
        await prepare(fake_client)
    assert not fake_client.injected


def test_budget_respects_native_effective_and_compaction_limits():
    limits = transfer.resolve_limits({**MODEL, "context_window": 272000, "max_context_window": 400000,
                                      "auto_compact_token_limit": 250000}, {})
    assert limits.usable_tokens == 258400 and limits.auto_compact_tokens == 244800
    capped = transfer.resolve_limits(MODEL, {"model_context_window": 999999999})
    assert capped.context_tokens == MODEL["max_context_window"]


@pytest.mark.parametrize("field,value", [("context_window", False), ("max_context_window", -1),
                                         ("effective_context_window_percent", True),
                                         ("auto_compact_token_limit", 0), ("input_modalities", None)])
def test_invalid_native_capacity_is_not_an_overflow_claim(field, value):
    with pytest.raises(transfer.TransferError, match="capacity_unknown"):
        transfer.resolve_limits({**MODEL, field: value}, {})


async def test_batches_remeasure_real_wire_escapes_and_ids(monkeypatch):
    client = AppServerClient()
    calls = []
    async def request(method, params):
        assert len(client.request_bytes(method, params)) <= 600
        client._next_id += 1
        calls.append(params["items"])
        return {}
    client.request = request
    monkeypatch.setattr(transfer, "MAX_REQUEST_BYTES", 600)
    items = tuple({"type": "message", "id": str(i), "role": "user", "content": [
                  {"type": "input_text", "text": "🙂" * 12}]} for i in range(5))
    await transfer._inject(client, "thread", items)
    assert [item for batch in calls for item in batch] == list(items)
    assert len(calls) > 1


async def test_indivisible_real_frame_rejects_without_cut(monkeypatch):
    client = AppServerClient()
    monkeypatch.setattr(transfer, "MAX_REQUEST_BYTES", 350)
    item = {"type": "message", "content": [{"type": "input_text", "text": "🙂" * 25}]}
    with pytest.raises(transfer.TransferError, match="item_too_large"):
        await transfer._inject(client, "thread", (item,))


async def test_cancel_closes_preparation_without_publishing(fake_client):
    original = fake_client.request
    async def cancel(method, params, **kwargs):
        if method == "thread/inject_items":
            raise asyncio.CancelledError()
        return await original(method, params, **kwargs)
    fake_client.request = cancel
    with pytest.raises(asyncio.CancelledError):
        await prepare(fake_client)
    assert fake_client.finished and not sessions._dir().exists()
    assert store.load_transfer(fake_client.record.id).destination_meta["thread_id"] == fake_client.thread_id


def test_small_image_has_separate_native_budget():
    limits = transfer.NativeModelLimits(19000, 18000, ("text", "image"), 20000)
    images = ({"type": "message", "content": [{"type": "input_image", "image_url": "data:image/png;base64,eA=="}]},)
    transfer.check_budget(images, limits, 0)
    with pytest.raises(transfer.TransferError, match="budget_exceeded"):
        transfer.check_budget(images * 2, limits, 0)


async def test_instruction_cost_uses_loaded_files_not_maximum(fake_client, tmp_path):
    path = tmp_path / "AGENTS.md"
    path.write_text("regra curta")
    size = await transfer._instruction_bytes(fake_client, {"instructionSources": [str(path)]},
                                              {"project_doc_max_bytes": 99999999}, MODEL,
                                              str(tmp_path), fake_client.thread_id)
    assert len(path.read_bytes()) <= size < 10000


async def test_discovery_failure_is_unknown_capacity(fake_client):
    original = fake_client.request
    async def fail_discovery(method, params, **kwargs):
        if method == "mcpServerStatus/list":
            return {"data": [{"name": "broken", "tools": {}, "toolsError": "error",
                              "runtimeStatus": "failed"}]}
        return await original(method, params, **kwargs)
    fake_client.request = fail_discovery
    with pytest.raises(transfer.TransferError, match="capacity_unknown"):
        await prepare(fake_client)
    assert not fake_client.injected
