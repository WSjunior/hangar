"""Coordenação real com processos/RPC simulados; não é prova do runtime nativo."""
import asyncio
from dataclasses import replace
import hashlib
import json
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import AsyncMock, Mock
import uuid

import pytest
from fastapi.testclient import TestClient
from pydantic import ValidationError

from app import api, conversation_transfer as store, pqueue, registry as registry_module
from app.adapters import CLAUDE_HEADLESS
from app.adapters.claude_headless import sessions as hs
from app.adapters.claude_headless.adapter import ClaudeHeadlessAdapter
from app.adapters.codex import sessions as cs, transfer as importer
from app.adapters.codex.adapter import CodexAdapter
from app.models import SessionInfo


@pytest.fixture
def scenario(tmp_path, monkeypatch):
    monkeypatch.setattr(store, "_base", lambda: tmp_path / "transfers")
    monkeypatch.setattr(hs, "_dir", lambda: tmp_path / "headless")
    monkeypatch.setattr(cs, "_dir", lambda: tmp_path / "codex-sidecars")
    monkeypatch.setattr(pqueue, "_queue_dir", lambda: tmp_path / "queue")
    (tmp_path / "queue").mkdir()
    sid = str(uuid.uuid4())
    source = tmp_path / ".claude" / "projects" / "project" / f"{sid}.jsonl"
    source.parent.mkdir(parents=True)
    source.write_text(json.dumps({"type": "user", "uuid": "u1", "parentUuid": None,
                                  "message": {"role": "user", "content": "contexto exclusivo"}}) + "\n")
    original = source.read_bytes()
    meta = hs.save("s", str(tmp_path), sid, config_dir=str(tmp_path / ".claude"),
                   key="source-key", permission_mode="manual")
    info = SessionInfo(name="s", cwd=str(tmp_path), jsonl=str(source), tracked=True,
                       provider="claude", headless=True, conta=f"claude:{tmp_path / '.claude'}")
    reg = registry_module.SessionRegistry()
    monkeypatch.setattr(reg, "list", lambda: [info])
    monkeypatch.setattr(reg, "_forget", Mock())
    monkeypatch.setattr(registry_module.tmux, "has_session", lambda name: False)
    monkeypatch.setattr(store, "session_life", lambda name: "k:source-key")
    hl, cx = ClaudeHeadlessAdapter(), CodexAdapter()
    live = SimpleNamespace(vivo=True, iniciando=False, in_progress=False, pending={}, question=None, sid=sid)
    hl._sessions["s"] = live
    hl.escolhas = lambda name: (None, None)
    resumed = []

    async def ensure(name, **kwargs):
        record = store.transfer_for_session(name)
        if record and record.phase == store.TransferPhase.RESTORING:
            resumed.append(name)
            live.vivo = True
        return live

    async def stop(name):
        live.vivo = False
    hl.ensure_running, hl.parar = ensure, stop
    hl.acordar = Mock(side_effect=AssertionError("não pode agendar nem drenar para provar restauração"))
    hl.send_prompt = AsyncMock(side_effect=AssertionError("não pode reenviar prompt"))
    adapter = lambda provider: hl if provider == CLAUDE_HEADLESS else cx
    monkeypatch.setattr("app.adapters.get_adapter", adapter)
    monkeypatch.setattr(api, "get_adapter", adapter)
    monkeypatch.setattr(api, "registry", reg)
    monkeypatch.setattr(api, "_invalidate_lists", Mock())
    monkeypatch.setattr(api, "session_life", lambda name: "k:source-key")
    monkeypatch.setattr(api.share_store, "set_life", Mock())
    monkeypatch.setattr(api.guest_users, "set_life", Mock())
    monkeypatch.setattr(api.settings, "auth_token", "test-secret")
    account = SimpleNamespace(id="chosen", home=tmp_path / ".codex-chosen")
    credential = f"codex:{account.home}"
    monkeypatch.setattr(store, "_resolve_target", lambda value: account if value == credential else
                        (_ for _ in ()).throw(store.TransferError("session_transfer_unknown_account")))
    monkeypatch.setattr(store, "_check_target", AsyncMock())
    thread_id = str(uuid.uuid4())
    rollout = tmp_path / f"rollout-test-{thread_id}.jsonl"
    failures = set()
    calls = []

    def fail(stage):
        calls.append(stage)
        if stage in failures:
            raise RuntimeError("conteúdo privado do erro não pode ser público")

    class Native:
        server_requests = {}
        async def request(self, method, params):
            fail(method)
            if method == "thread/loaded/list":
                return {"data": [thread_id]}
            if method == "config/read":
                return {"config": {"tool_output_token_limit": 5000}}
            if method == "thread/read":
                return {"thread": {"id": thread_id, "path": str(rollout), "cwd": str(tmp_path),
                        "model": "test-model", "reasoningEffort": "low", "status": {"type": "idle"}, "turns": []}}
            raise AssertionError(method)
        async def close(self):
            calls.append("close")

    async def prepare(account, cwd, context, model, effort, permission_mode, *, transfer_id, collaboration_mode):
        fail("inject")
        record = store.load_transfer(transfer_id)
        assert record.phase == store.TransferPhase.SOURCE_STOPPED
        assert not cs.list_all()
        raw = (json.dumps({"type": "session_meta", "payload": {"id": thread_id}}) + "\n").encode()
        rollout.write_bytes(raw)
        boundary = store.ImportBoundary(thread_id, str(rollout), len(raw), (), hashlib.sha256(raw).hexdigest())
        target = {"thread_id": thread_id, "rollout_path": str(rollout), "codex_home": str(account.home),
                  "codex_account": account.id, "tool_output_token_limit": 5000, "cwd": cwd,
                  "permission_mode": permission_mode, "transfer_id": transfer_id,
                  "model": "test-model", "effort": "low"}
        store.save_transfer(replace(record, destination_meta=target, boundary=boundary))
        fail("persistence")
        return importer.PreparedCodexThread(thread_id, str(rollout), "test-model", "low", collaboration_mode, 5000, boundary)

    async def adopt(name, prepared, target, *, terminal):
        fail("resume")
        assert not terminal
        cs.write_prepared(name, target["transfer_id"], target)
        cx._adoptions = {name: (target["transfer_id"], Native(), prepared.mode)}
        fail("adoption")

    def attach(name, client, thread_id, **kwargs):
        cx._sessions[name] = {"client": client, "thread_id": thread_id}

    monkeypatch.setattr(importer, "prepare_import", prepare)
    cx.adopt_imported, cx.attach = adopt, attach
    body = {"credential_id": credential, "source_life": "k:source-key", "source_jsonl": str(source),
            "model": "test-model", "effort": "low"}

    async def run():
        return await store.transfer_claude_to_codex(reg, "s", **body)

    return SimpleNamespace(reg=reg, hl=hl, cx=cx, source=source, original=original, info=info,
                           meta=meta, body=body, run=run, live=live, failures=failures, calls=calls,
                           resumed=resumed, account=account, credential=credential, thread_id=thread_id)


def test_legacy_body_is_unchanged():
    body = api.AccountMoveBody(config_dir="/example/.claude")
    assert body.config_dir == "/example/.claude" and body.credential_id is None


@pytest.mark.parametrize("body", [{}, {"config_dir": "a", "credential_id": "codex:b"},
    {"config_dir": "a", "model": "x"}, {"credential_id": "codex:a", "source_life": "k:a"},
    {"credential_id": "claude:a", "source_life": "k:a", "source_jsonl": "/x"}])
def test_account_route_rejects_ambiguous_or_incomplete_target(body):
    with pytest.raises(ValidationError):
        api.AccountMoveBody(**body)


def test_blank_overrides_are_absence(scenario):
    body = api.AccountMoveBody(**{**scenario.body, "model": "  ", "effort": ""})
    assert body.model is None and body.effort is None


@pytest.mark.parametrize("field,value", [("source_life", "k:other"), ("source_jsonl", "/not/authorized.jsonl")])
def test_post_rejects_stale_source_without_import(scenario, field, value):
    response = TestClient(api.app).post("/api/sessions/s/conta", json={**scenario.body, field: value},
                                       headers={"Authorization": "Bearer test-secret"})
    assert response.status_code == 409
    assert response.json()["detail"]["code"] == "session_transfer_source_changed"
    assert scenario.calls == [] and scenario.source.read_bytes() == scenario.original


async def test_preserves_identity_original_and_native_response(scenario):
    result = await scenario.run()
    assert result == {"ok": True, "provider": "codex", "conta": scenario.credential,
                      "model": "test-model", "effort": "low", "transfer_id": result["transfer_id"]}
    record = store.load_transfer(result["transfer_id"])
    assert record.phase == store.TransferPhase.COMPLETE
    assert cs.load("s")["key"] == scenario.meta["key"] == record.origin_meta["key"]
    assert cs.load("s")["thread_id"] == scenario.thread_id
    assert scenario.source.read_bytes() == scenario.original and not hs.exists("s")
    assert not scenario.resumed and not scenario.hl.send_prompt.called and not scenario.hl.acordar.called


@pytest.mark.parametrize("stage", ["inject", "persistence", "resume", "adoption", "thread/read", "thread/loaded/list"])
async def test_faults_restore_original_without_replay(scenario, stage):
    scenario.failures.add(stage)
    with pytest.raises(store.TransferError) as exc:
        await scenario.run()
    assert exc.value.code == "session_transfer_failed"
    assert scenario.resumed == ["s"]
    assert hs.load("s")["session_id"] == scenario.meta["session_id"]
    assert not cs.exists("s") and scenario.source.read_bytes() == scenario.original
    record = store.transfer_for_session("s")
    assert record.phase == store.TransferPhase.ROLLED_BACK
    assert not scenario.hl.send_prompt.called and not scenario.hl.acordar.called


@pytest.mark.parametrize("delivered", [False, True])
async def test_real_guard_refuses_pending_and_unconfirmed_delivery(scenario, delivered):
    entry = pqueue.PromptQueue("s").append("guardar entrada", delivered=delivered)
    with pytest.raises(store.TransferError, match="session_transfer_queue_pending"):
        await scenario.run()
    assert pqueue.PromptQueue("s").load()[0]["id"] == entry["id"]
    assert scenario.live.vivo and not scenario.calls


@pytest.mark.parametrize("field,value", [("in_progress", True), ("pending", {"approval": 1}),
                                         ("question", {"question": 1}), ("iniciando", True)])
async def test_real_guard_refuses_active_work(scenario, field, value):
    setattr(scenario.live, field, value)
    with pytest.raises(store.TransferError, match="session_transfer_source_busy"):
        await scenario.run()
    assert scenario.live.vivo and not scenario.calls


async def test_dead_quota_source_with_trusted_transcript_can_transfer(scenario):
    scenario.live.vivo = False
    result = await scenario.run()
    assert result["ok"]


@pytest.mark.parametrize("mode,base,wanted", [("manual", None, "Ask for approval"),
    ("acceptEdits", None, "Approve for me"), ("bypassPermissions", None, "Full Access"),
    ("plan", "manual", "Ask for approval"), ("plan", "bypassPermissions", "Full Access")])
def test_permission_and_plan_are_separate(mode, base, wanted):
    assert store._map_permission({"permission_mode": mode, "previous_non_plan": base}) == (
        wanted, "plan" if mode == "plan" else "default")


@pytest.mark.parametrize("mode", [None, "auto", "dontAsk", "unknown", "plan"])
def test_unknown_permission_never_falls_back_to_full_access(mode):
    with pytest.raises(store.TransferError):
        store._map_permission({"permission_mode": mode})


async def test_restore_failed_is_durable_and_reload_recovers(scenario, monkeypatch):
    scenario.failures.add("inject")
    restore = scenario.reg.restore_transfer_source
    monkeypatch.setattr(scenario.reg, "restore_transfer_source", AsyncMock(side_effect=RuntimeError("private")))
    with pytest.raises(store.TransferError, match="session_transfer_restore_failed"):
        await scenario.run()
    record = store.transfer_for_session("s")
    assert record.phase == store.TransferPhase.RESTORE_FAILED and store.transfer_active("s")
    monkeypatch.setattr(scenario.reg, "restore_transfer_source", restore)
    result = await api.recarregar_sessao("s")
    assert result["provider"] == "claude"
    assert store.load_transfer(record.id).phase == store.TransferPhase.ROLLED_BACK
    assert scenario.resumed == ["s"]


@pytest.mark.parametrize("phase", ["source_stopped", "inject"])
async def test_http_cancellation_waits_until_operation_converges(scenario, monkeypatch, phase):
    entered, release = asyncio.Event(), asyncio.Event()
    target = scenario.reg if phase == "source_stopped" else importer
    attribute = "stop_transfer_source" if phase == "source_stopped" else "prepare_import"
    original = getattr(target, attribute)

    async def pause(*args, **kwargs):
        result = await original(*args, **kwargs)
        entered.set()
        await release.wait()
        return result

    monkeypatch.setattr(target, attribute, pause)
    request = asyncio.create_task(api.trocar_conta("s", api.AccountMoveBody(**scenario.body)))
    await entered.wait()
    request.cancel()
    await asyncio.sleep(0)
    assert not request.done() and "s" in api.share_api.changing_mode
    release.set()
    with pytest.raises(asyncio.CancelledError):
        await request
    record = store.transfer_for_thread(str(scenario.account.home), scenario.thread_id)
    assert record.phase == store.TransferPhase.COMPLETE
    assert "s" not in api.share_api.changing_mode


async def test_duplicate_operation_is_rejected_without_waiting(scenario):
    with store.session_operation("s"):
        with pytest.raises(store.TransferError, match="session_transfer_busy"):
            await scenario.run()
    assert not scenario.calls


async def test_terminal_send_lock_is_nonblocking(scenario):
    from app.terminal_input import _send_lock
    lock = _send_lock("s")
    lock.acquire()
    try:
        with pytest.raises(store.TransferError, match="session_transfer_busy"):
            await asyncio.wait_for(scenario.run(), 1)
    finally:
        lock.release()


async def test_transfer_rows_keep_shared_and_owner_without_physical_poll(scenario, monkeypatch):
    row = SessionInfo(name="s", transfer_id=str(uuid.uuid4()), transfer_phase="restore_failed")
    monkeypatch.setattr(registry_module.share_store, "active_sessions", lambda: {"s"})
    monkeypatch.setattr(registry_module.guest_users, "has_claims", lambda: True)
    monkeypatch.setattr(registry_module.guest_users, "owner_name", lambda name: "dono")
    monkeypatch.setattr(registry_module.tmux, "capture_pane", Mock(side_effect=AssertionError("não pode ler estado projetado")))
    result = await scenario.reg.list_with_state([row])
    assert result == [row] and row.shared and row.owner == "dono"


def test_prepared_runtime_is_not_discoverable(scenario):
    transfer_id = str(uuid.uuid4())
    meta = {"name": "s", "transfer_id": transfer_id, "thread_id": scenario.thread_id,
            "key": "source-key", "codex_home": str(scenario.account.home)}
    cs.write_prepared("s", transfer_id, meta)
    assert cs.load("s") is None and cs.list_all() == []
    with pytest.raises(ValueError):
        cs.write_prepared("s", transfer_id, {**meta, "key": "other"})
    assert cs.load_prepared("s", transfer_id) == meta


@pytest.mark.parametrize("provider,engine,tracked", [("codex", None, True), ("claude", "engine", True),
                                                     ("claude", None, False)])
async def test_wrong_provider_engine_or_untracked_source_is_refused(scenario, provider, engine, tracked):
    scenario.info.provider, scenario.info.engine, scenario.info.tracked = provider, engine, tracked
    with pytest.raises(store.TransferError, match="session_transfer_source_changed"):
        await scenario.run()
    assert scenario.live.vivo and not scenario.calls


async def test_rejection_after_preparing_does_not_reopen_source(scenario, monkeypatch):
    check = store._check_source_idle
    count = 0

    async def starts_work(*args, **kwargs):
        nonlocal count
        count += 1
        if count == 2:
            scenario.live.in_progress = True
        await check(*args, **kwargs)

    monkeypatch.setattr(store, "_check_source_idle", starts_work)
    with pytest.raises(store.TransferError, match="session_transfer_source_busy"):
        await scenario.run()
    record = store.transfer_for_session("s")
    assert record.phase == store.TransferPhase.REJECTED
    assert scenario.live.vivo and not scenario.resumed


async def test_snapshot_failure_restores_source(scenario, monkeypatch):
    monkeypatch.setattr(store, "capture_snapshot", Mock(side_effect=OSError("private")))
    with pytest.raises(store.TransferError, match="session_transfer_failed"):
        await scenario.run()
    assert scenario.resumed == ["s"] and scenario.source.read_bytes() == scenario.original
    assert store.transfer_for_session("s").phase == store.TransferPhase.ROLLED_BACK


async def test_complete_commit_failure_rolls_back_published_codex(scenario, monkeypatch):
    save = store.save_transfer

    def fail_commit(record):
        if record.phase == store.TransferPhase.COMPLETE:
            raise OSError("disk")
        return save(record)

    monkeypatch.setattr(store, "save_transfer", fail_commit)
    with pytest.raises(store.TransferError):
        await scenario.run()
    assert not cs.exists("s") and hs.exists("s")
    assert store.transfer_for_session("s").phase == store.TransferPhase.ROLLED_BACK
    assert scenario.resumed == ["s"]


async def test_source_written_after_stop_cannot_publish(scenario, monkeypatch):
    adopt = scenario.cx.adopt_imported

    async def changed(*args, **kwargs):
        await adopt(*args, **kwargs)
        with scenario.source.open("ab") as source:
            source.write(b'{}\n')

    monkeypatch.setattr(scenario.cx, "adopt_imported", changed)
    with pytest.raises(store.TransferError, match="session_transfer_restore_failed"):
        await scenario.run()
    assert not cs.exists("s")
    assert store.transfer_for_session("s").phase == store.TransferPhase.RESTORE_FAILED


async def test_terminal_native_state_is_checked_even_when_list_says_idle(scenario, monkeypatch):
    scenario.info.headless = False
    scenario.live.vivo = False
    pane = {"pid": 901, "cwd": scenario.info.cwd}
    monkeypatch.setattr(scenario.reg, "_pane_of", lambda name: pane)
    monkeypatch.setattr(registry_module, "_pid_do_agente", lambda pid: 902)
    monkeypatch.setattr(registry_module, "provider_of_pane", lambda *args: "claude")
    monkeypatch.setattr(registry_module.procinfo, "_proc_children_map", lambda **kwargs: {})
    monkeypatch.setattr(registry_module.procinfo, "_engine_of", lambda pid: None)
    native_dir = Path(scenario.meta["config_dir"]) / "sessions"
    native_dir.mkdir()
    native = native_dir / "902.json"
    native.write_text(json.dumps({"sessionId": scenario.meta["session_id"], "pid": 902, "status": "busy"}))
    meta = {**scenario.meta, "jsonl": str(scenario.source), "headless": False, "pane_pid": 901}
    with pytest.raises(store.TransferError, match="session_transfer_source_busy"):
        await store._check_source_idle(scenario.reg, "s", meta)
    native.write_text(json.dumps({"sessionId": "changed", "pid": 902, "status": "idle"}))
    with pytest.raises(store.TransferError, match="session_transfer_source_changed"):
        await store._check_source_idle(scenario.reg, "s", meta)


async def test_ingress_and_controls_are_temporarily_unavailable(scenario, monkeypatch):
    record = store.TransferRecord(str(uuid.uuid4()), "s", "k:source-key", store.TransferPhase.PREPARING,
                                  None, {**scenario.meta, "jsonl": str(scenario.source)}, None, None, None)
    store.save_transfer(record)
    monkeypatch.setattr(api, "_send_native_available", Mock(side_effect=AssertionError("não pode enviar")))
    assert api._send_one("s", "guardar")["error"]["code"] == "session_transfer_busy"
    assert (await api._enviar("s", "guardar"))["error"]["code"] == "session_transfer_busy"
    assert api._enviar_nativo("s", "[de: peer] guardar") is None
    assert api.terminal.send_prompt("s", "guardar") == "deferred"
    assert not api.plugin_bridge.entregar("s", "guardar")
    assert pqueue.PromptQueue("s").load() == []
    client = TestClient(api.app)
    for method, path, body in [("post", "/api/sessions/s/rename", {"name": "new"}),
                               ("delete", "/api/sessions/s", None),
                               ("post", "/api/sessions/s/keys", {"key": "enter"})]:
        response = client.request(method, path, json=body, headers={"Authorization": "Bearer test-secret"})
        assert response.status_code == 409
        assert response.json()["detail"]["code"] == "session_transfer_busy"


@pytest.mark.parametrize("headless", [False, True])
async def test_archived_thread_reapplies_budget_and_keeps_explicit_choices(scenario, monkeypatch, headless):
    result = await scenario.run()
    record = store.load_transfer(result["transfer_id"])
    cs.delete("s")
    scenario.account.is_default = False
    monkeypatch.setattr(registry_module.codex_contas, "resolve_account", lambda name: scenario.account)
    monkeypatch.setattr(registry_module, "_exigir_lancador_codex", Mock())
    monkeypatch.setattr(registry_module, "_encerrar_pares_externos", Mock())
    monkeypatch.setattr(registry_module, "ThenLink", Mock())
    monkeypatch.setattr(scenario.reg, "_clear_pair", Mock())
    monkeypatch.setattr(cs, "pretrust_cwd", Mock(side_effect=AssertionError("não alterar confiança")))
    calls = []

    def spawn(name, cwd, command, *args, **kwargs):
        meta = cs.load(name)
        assert meta["transfer_id"] == record.id and meta["tool_output_token_limit"] == 5000
        assert meta["model"] == "explicit-model" and meta["effort"] == "high"
        assert "--tool-output-token-limit 5000" in command and "explicit-model" in command
        calls.append(command)
        return True

    monkeypatch.setattr(registry_module.tmux, "new_session", spawn)
    scenario.reg.create("archive", scenario.info.cwd, provider="codex", codex_account=scenario.account.id,
                        resume_session_id=scenario.thread_id, transfer_id=record.id, tool_output_token_limit=5000,
                        model="explicit-model", effort="high", headless=headless)
    meta = cs.load("archive")
    assert meta["transfer_id"] == record.id and meta["tool_output_token_limit"] == 5000
    assert meta["model"] == "explicit-model" and meta["effort"] == "high"
    assert bool(calls) is not headless


def test_recycled_pid_never_receives_signal(scenario, monkeypatch):
    from app import procinfo
    record = store.TransferRecord(str(uuid.uuid4()), "s", "k:source-key", store.TransferPhase.SOURCE_STOPPED,
                                  None, scenario.meta, None, None, None)
    store.prepare_runtime(record)
    store._write_json(store._runtime_path(record), {"original": {}, "processes": {}, "import_pid": 9,
        "import_processes": {"9": {"started": 1, "command": "old"}}})
    monkeypatch.setattr(procinfo, "pid_vivo", lambda pid: True)
    monkeypatch.setattr(store, "_process_identity", lambda pid: {"started": 2, "command": "other"})
    kill = Mock(side_effect=AssertionError("não pode matar PID reciclado"))
    monkeypatch.setattr(store.os, "kill", kill)
    with pytest.raises(store.TransferError, match="session_transfer_process_identity_changed"):
        asyncio.run(store.stop_import_process(record))
    assert not kill.called


async def test_terminal_adoption_loads_same_thread_before_publishing(scenario, monkeypatch):
    from app.adapters.codex import adapter as adapter_module
    result = await scenario.run()
    record = store.load_transfer(result["transfer_id"])
    cs.delete("s")
    scenario.cx._sessions.clear()
    meta = {**record.destination_meta, "headless": False}
    prepared = importer.PreparedCodexThread(scenario.thread_id, record.boundary.rollout_path,
        "test-model", "low", "default", 5000, record.boundary)
    spawned = False
    requests = []

    class Client:
        endpoint = None
        server_requests = {}
        async def connect(self, endpoint):
            self.endpoint = endpoint
        async def close(self):
            pass
        async def request(self, method, params):
            requests.append(method)
            thread = {"id": scenario.thread_id, "path": record.boundary.rollout_path,
                      "cwd": scenario.info.cwd, "model": "test-model", "reasoningEffort": "low",
                      "turns": [], "status": {"type": "idle"}}
            if method in {"initialize", "thread/settings/update"}:
                return {}
            if method == "thread/loaded/list":
                return {"data": [scenario.thread_id]}
            if method == "thread/read":
                return {"thread": thread}
            if method == "config/read":
                return {"config": {"tool_output_token_limit": 5000}}
            if method == "thread/resume":
                assert "cwd" not in params
                return {"thread": thread, "approvalPolicy": "on-request", "sandbox": {"type": "readOnly"}}
            raise AssertionError(method)

    def spawn(name, cwd, command, **kwargs):
        nonlocal spawned
        spawned = True
        assert "--transfer-id" in command and record.id in command
        assert kwargs["env"]["CP_SESSION_KEY"] == record.origin_meta["key"]
        assert cs.load(name) is None
        cs.update_transfer_runtime(name, record.id, endpoint="ws://127.0.0.1:9999", app_pid=12, tui_pid=13)
        return True

    monkeypatch.setattr(adapter_module, "AppServerClient", Client)
    monkeypatch.setattr(adapter_module.codex_contas, "resolve_account", lambda name: scenario.account)
    monkeypatch.setattr(adapter_module, "pid_vivo", lambda pid: True)
    monkeypatch.setattr(registry_module, "_exigir_lancador_codex", Mock())
    monkeypatch.setattr(store, "_processes", lambda pid: {str(pid): {"started": 1, "command": "test"}})
    monkeypatch.setattr(registry_module.tmux, "has_session", lambda name: spawned)
    monkeypatch.setattr(registry_module.tmux, "pane_pid", lambda name: 11)
    monkeypatch.setattr(registry_module.tmux, "new_session", spawn)
    await CodexAdapter.adopt_imported(scenario.cx, "s", prepared, meta, terminal=True)
    assert cs.load("s") is None and cs.list_all() == []
    assert "thread/loaded/list" in requests and "thread/resume" in requests
    assert "turn/start" not in requests and "thread/start" not in requests
    await scenario.cx.publish_imported(record)
    assert cs.load("s")["thread_id"] == scenario.thread_id
    assert cs.load("s")["key"] == record.origin_meta["key"]


def test_crash_without_final_child_snapshot_never_claims_cleanup(scenario, monkeypatch):
    from app import procinfo
    record = store.TransferRecord(str(uuid.uuid4()), "s", "k:source-key", store.TransferPhase.SOURCE_STOPPED,
                                  None, scenario.meta, None, None, None)
    store.prepare_runtime(record)
    store._write_json(store._runtime_path(record), {"original": {}, "processes": {}, "import_pid": 9,
        "import_processes": {"9": {"started": 1, "command": "owned"}}})
    monkeypatch.setattr(procinfo, "pid_vivo", lambda pid: False)
    kill = Mock(side_effect=AssertionError("não há prova de filhos por nome"))
    monkeypatch.setattr(store.os, "kill", kill)
    with pytest.raises(store.TransferError, match="session_transfer_import_cleanup_unconfirmed"):
        asyncio.run(store.stop_import_process(record))
    assert not kill.called


@pytest.mark.parametrize("state,pct,code", [("expirada", 0, "session_transfer_login_required"),
    ("lida", 99, "session_transfer_account_full"), ("lida", 10, "session_transfer_invalid_model_choice")])
async def test_target_uses_codex_quota_and_validates_model_before_stop(tmp_path, monkeypatch, state, pct, code):
    from app import cotas, codex_models, codex_appserver
    account = SimpleNamespace(home=tmp_path / ".codex-target")
    monkeypatch.setattr(codex_appserver, "versao", lambda: importer.SUPPORTED_VERSION)
    reads = []
    monkeypatch.setattr(cotas, "_ler_codex", lambda home: reads.append(home) or
                        (state, [SimpleNamespace(pct=pct)], None))
    monkeypatch.setattr(cotas, "cotas_claude", Mock(side_effect=AssertionError("não é cota Claude")))
    monkeypatch.setattr(codex_models, "checar_escolha", Mock(side_effect=ValueError("private")))
    with pytest.raises(store.TransferError) as error:
        await store._check_target(account, "invalid", "low")
    assert error.value.code == code and reads == [account.home]


def test_credential_path_is_only_matched_to_registered_account(tmp_path, monkeypatch):
    from app import codex_contas
    listed = SimpleNamespace(id="known", home=tmp_path / ".codex-known")
    monkeypatch.setattr(codex_contas, "list_accounts", lambda: [listed])
    resolve = Mock(side_effect=AssertionError("caminho arbitrário não pode resolver conta"))
    monkeypatch.setattr(codex_contas, "resolve_account", resolve)
    with pytest.raises(store.TransferError, match="session_transfer_unknown_account"):
        store._resolve_target(f"codex:{tmp_path / 'arbitrary'}")
    assert not resolve.called
