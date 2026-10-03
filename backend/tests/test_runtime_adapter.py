import asyncio
from types import SimpleNamespace

import pytest

from app import runtime_coordinator
from app.runtime_adapter import RuntimeAdapter, RuntimeView, install_adapter


class Coordinator:
    instance = "instance-test"

    def __init__(self):
        self.calls = []
        self.voice_clients = {}
        self.target = SimpleNamespace(phase=runtime_coordinator.Phase.Rust,
            binding=SimpleNamespace(key="key", generation=2, provider="codex", headless=True),
            view={"key":"key","generation":2,"revision":7,"channels":{},"view":{"alive":True,"ready":True,
                "thread_id":"thread-2","model":"test-model","effort":"high","deliverable":True,
                "public_state":{"session":"session","state":"idle","headless":True}}},
            cache_valid=True, changed=asyncio.Event())

    def managed_runtime(self, name):
        return name == "session"

    def slot(self, name):
        return self.target

    async def op(self, name, command, operation_id):
        self.calls.append((name, command, operation_id))
        return {"disposition":"accepted","payload":[{"model":"test-model"}]}


@pytest.fixture
def owner(monkeypatch):
    coordinator = Coordinator()
    monkeypatch.setattr(runtime_coordinator, "_current", coordinator)
    return coordinator


def test_owned_runtime_never_opens_legacy_reader(owner):
    class Adapter:
        async def ensure_running(self, name):
            raise AssertionError("leitor Legacy")

        async def list_models(self, name):
            raise AssertionError("RPC Legacy")

    install_adapter(Adapter, "codex")
    adapter = Adapter()
    assert isinstance(asyncio.run(adapter.ensure_running("session")), RuntimeView)
    assert asyncio.run(adapter.list_models("session")) == [{"model":"test-model"}]
    assert len(owner.calls) == 1


def test_getter_uses_same_generation(owner):
    facade = RuntimeAdapter("codex")
    assert facade.current_model("session")["model"] == "test-model"
    owner.target.view["generation"] = 1
    with pytest.raises(RuntimeError):
        facade.current_model("session")


def test_invalid_event_or_gap_requests_snapshot(owner):
    from app.runtime_adapter import apply_event
    before = dict(owner.target.view)
    event = {"key":"key","generation":2,"revision":9,"channel":"state", "data":{"state":"working"}}
    assert apply_event(owner.target, event) is False
    assert owner.target.cache_valid is False
    assert owner.target.view == before
    with pytest.raises(RuntimeError):
        asyncio.run(RuntimeAdapter("codex").list_models("session"))


def test_old_snapshot_preserves_newer_good_cache(owner):
    from app.runtime_adapter import apply_event
    data = dict(owner.target.view)
    data["revision"] = 6
    before = dict(owner.target.view)
    assert apply_event(owner.target, {"key":"key", "generation":2, "revision":6, "channel":"snapshot", "data":data})
    assert owner.target.view == before
    assert owner.target.cache_valid


def test_sync_lifecycle_does_not_touch_legacy_while_owned(owner):
    class Adapter:
        def close_sync(self, name):
            raise AssertionError("fechamento Legacy sem barreira")
        def rename(self, old, new):
            raise AssertionError("rename Legacy sem barreira")
    install_adapter(Adapter, "codex")
    adapter = Adapter()
    with pytest.raises(RuntimeError):
        adapter.close_sync("session")
    with pytest.raises(RuntimeError):
        adapter.rename("session", "new")


def test_sync_endpoint_uses_server_loop(owner):
    from app.runtime_adapter import run_sync
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        async def value():
            assert asyncio.get_running_loop() is owner.loop
            return 17
        assert await asyncio.to_thread(run_sync, value, owner.loop) == 17
        with pytest.raises(RuntimeError):
            run_sync(value, owner.loop)
    asyncio.run(scenario())


def test_legacy_reserve_journals_controls(tmp_path, monkeypatch):
    from app.runtime_adapter import LegacyIO
    from app.runtime_coordinator import Binding, RuntimeCoordinator
    coordinator = RuntimeCoordinator()
    monkeypatch.setattr(runtime_coordinator, "_current", coordinator)
    slot = coordinator.register(Binding("session", "key", "claude", True,
        {"key":"key", "session_id":"sid"}, str(tmp_path / "chat.jsonl"), tmp_path / "projection",
        tmp_path / "state.json", tmp_path / "lease", 1))
    class Writer:
        def write(self, raw):
            import json
            envelope = json.loads(raw)
            assert slot.store.state["operations"][envelope["operation_id"]]["status"] == "dispatching"

        async def drain(self):
            raise OSError("partial write")
    async def scenario():
        io = LegacyIO(coordinator)
        endpoint = SimpleNamespace(runtime_acks={})
        with pytest.raises(RuntimeError):
            await io.write("session", endpoint, Writer(), {"type":"control_request", "request_id":"request",
                "request":{"subtype":"set_model", "model":"test-model"}}, 2)
        phases = [operation for operation in slot.store.state["operations"].values()
                  if operation["payload"].get("frame")]
        assert len(phases) == 1
        assert phases[0]["status"] == "unknown"
    try:
        asyncio.run(scenario())
    finally:
        coordinator.close_python_leases()


def test_voice_broker_virtual_client_does_not_construct_legacy_client(owner, monkeypatch):
    from app import codex_voice_broker
    monkeypatch.setattr(codex_voice_broker, "AppServerClient", lambda: (_ for _ in ()).throw(AssertionError("cliente paralelo")))
    client = SimpleNamespace(virtual=True)
    broker = codex_voice_broker.VoiceBroker(SimpleNamespace(), "session", {"runtime_voice":True, "client":client}, None)
    assert broker.client is client


def test_voice_stream_failure_invalidates_old_workers(owner):
    from app.runtime_adapter import NativeVoiceClient
    client = NativeVoiceClient(owner, "session", "call", asyncio.Queue())
    assert client.valid()
    client.fail(RuntimeError("gap"))
    assert not client.valid()


def test_steer_queue_uses_one_owner_command(owner):
    async def op(name, command, operation_id):
        owner.calls.append(command)
        return {"disposition":"accepted", "payload":{"ids":["entry"]}}
    owner.op = op
    result = asyncio.run(RuntimeAdapter("codex").dispatch("steer_queue", "session", {"entry_id":"entry"}))
    assert result == ["entry"]
    assert owner.calls == [{"kind":"control", "control":"steer_queue", "payload":{"entry_id":"entry"}}]


def test_codex_reserve_permission_uses_provider_signature(tmp_path, monkeypatch):
    from app.runtime_adapter import LegacyBridge
    from app.runtime_coordinator import Binding, RuntimeCoordinator
    coordinator = RuntimeCoordinator()
    monkeypatch.setattr(runtime_coordinator, "_current", coordinator)
    slot = coordinator.register(Binding("session", "key", "codex", True, {"key":"key", "thread_id":"thread"},
        str(tmp_path / "chat.jsonl"), tmp_path / "projection", tmp_path / "state", tmp_path / "lease", 1))
    calls = []
    class Adapter:
        async def set_permission_mode_sem_terminal(self, name, modo):
            calls.append((name, modo))
            return {"mode":modo}
    bridge = LegacyBridge(coordinator, {"codex":Adapter()})
    try:
        result = asyncio.run(bridge.op(slot.binding.descriptor(), {"kind":"control", "control":"set_permission_mode",
            "payload":{"mode":"Full Access"}}, "permission-op"))
        assert result["disposition"] == "accepted"
        assert calls == [("session", "Full Access")]
    finally:
        coordinator.close_python_leases()


def test_quiesce_waits_for_cleanup_persistence(tmp_path, monkeypatch):
    from app.runtime_coordinator import Binding, RuntimeCoordinator, WriterLease
    class Legacy:
        async def quiesce(self, descriptor):
            slot.active += 1
            async def finish():
                await asyncio.sleep(0)
                slot.active -= 1
                coordinator._signal(slot)
            asyncio.create_task(finish())
            return {"runtime_state":{}}
    class Gateway:
        instance = "instance"
        async def op(self, descriptor, command, operation_id, clock):
            assert slot.active == 0
            self.lease = WriterLease(descriptor["lock_path"])
            return {"ready":True, "instance":"instance", "key":"key", "generation":1, "state":{}}
    async def peek(descriptor):
        return {}
    gateway = Gateway()
    coordinator = RuntimeCoordinator(gateway, Legacy(), peek)
    monkeypatch.setattr(runtime_coordinator, "_current", coordinator)
    slot = coordinator.register(Binding("session", "key", "claude", True, {"key":"key"}, str(tmp_path / "chat"),
        tmp_path / "projection", tmp_path / "state", tmp_path / "lease", 1))
    try:
        asyncio.run(coordinator.adopt("session"))
    finally:
        gateway.lease.close()


def test_reserve_composite_waits_for_both_replies(tmp_path, monkeypatch):
    from app.runtime_adapter import LegacyIO, _legacy_operation
    from app.runtime_coordinator import Binding, RuntimeCoordinator
    coordinator = RuntimeCoordinator()
    monkeypatch.setattr(runtime_coordinator, "_current", coordinator)
    slot = coordinator.register(Binding("session", "key", "claude", True, {"key":"key"},
        str(tmp_path / "chat"), tmp_path / "projection", tmp_path / "state", tmp_path / "lease", 1))
    async def scenario():
        context = {"operation_id":"composite", "command":{"kind":"set_model", "payload":{"effort":"high"}}}
        token = _legacy_operation.set(context)
        coordinator.legacy_active.add("composite")
        io = LegacyIO(coordinator)
        try:
            for request_id, subtype in (("model", "set_model"), ("effort", "set_effort")):
                ticket = await io.prepare_wire("session", subtype,
                    {"type":"control_request", "request_id":request_id, "request":{"subtype":subtype}})
                endpoint = SimpleNamespace(runtime_tickets={(str, request_id):ticket})
                await io.reply("session", endpoint, {"type":"control_response", "response":{"request_id":request_id}})
                await io.finish_wire(ticket, "unknown")
                assert slot.store.state["operations"]["composite"]["status"] == "dispatching"
            await io.finish_call("session", context)
            assert slot.store.state["operations"]["composite"]["status"] == "accepted"
        finally:
            coordinator.legacy_active.discard("composite")
            _legacy_operation.reset(token)
    try:
        asyncio.run(scenario())
    finally:
        coordinator.close_python_leases()


def test_reserve_bootstrap_before_rollout(tmp_path, monkeypatch):
    from app.runtime_adapter import LegacyIO
    from app.runtime_coordinator import Binding, RuntimeCoordinator
    coordinator = RuntimeCoordinator()
    monkeypatch.setattr(runtime_coordinator, "_current", coordinator)
    slot = coordinator.register(Binding("session", "key", "codex", True, {"key":"key"}, "",
        tmp_path / "projection", tmp_path / "state", tmp_path / "lease", 1))
    try:
        ticket = asyncio.run(LegacyIO(coordinator).prepare_wire("session", "initialize",
            {"id":"bootstrap", "method":"initialize", "params":{}}))
        assert slot.store.state["operations"][ticket.phase_id]["status"] == "dispatching"
    finally:
        coordinator.close_python_leases()


def test_reply_without_ack_is_final(tmp_path, monkeypatch):
    from app.runtime_adapter import LegacyIO, _legacy_operation
    from app.runtime_coordinator import Binding, RuntimeCoordinator
    coordinator = RuntimeCoordinator()
    monkeypatch.setattr(runtime_coordinator, "_current", coordinator)
    slot = coordinator.register(Binding("session", "key", "codex", True, {"key":"key"},
        str(tmp_path / "chat"), tmp_path / "projection", tmp_path / "state", tmp_path / "lease", 1))
    async def scenario():
        io = LegacyIO(coordinator)
        endpoint = SimpleNamespace(runtime_acks={})
        context = {"operation_id":"control", "command":{"kind":"list_models", "payload":{}}}
        token = _legacy_operation.set(context)
        coordinator.legacy_active.add("control")
        class Writer:
            def write(self, raw):
                pass
            async def drain(self):
                await io.reply("session", endpoint, {"id":"request", "result":{"data":[]}})
        try:
            await asyncio.wait_for(io.write("session", endpoint, Writer(),
                {"id":"request", "method":"model/list", "params":{}}, 2), 2)
            await io.finish_call("session", context)
            assert slot.store.state["operations"]["control"]["status"] == "accepted"
            assert not endpoint.runtime_acks
        finally:
            coordinator.legacy_active.discard("control")
            _legacy_operation.reset(token)
    try:
        asyncio.run(scenario())
    finally:
        coordinator.close_python_leases()


@pytest.mark.parametrize("reload", [False, True])
def test_confirmed_prompt_does_not_consume_next_echo(tmp_path, monkeypatch, reload):
    import json
    from app.runtime_adapter import LegacyBridge
    from app.runtime_receipt import ReceiptIndex
    from app.runtime_coordinator import Binding, RuntimeCoordinator
    from app.runtime_queue import QueueStore, initial_state
    path = tmp_path / "chat.jsonl"
    path.touch()
    coordinator = RuntimeCoordinator()
    monkeypatch.setattr(runtime_coordinator, "_current", coordinator)
    slot = coordinator.register(Binding("session", "key", "claude", True, {"key":"key", "session_id":"sid"},
        str(path), tmp_path / "projection", tmp_path / "state", tmp_path / "lease", 1))
    bridge = LegacyBridge(coordinator, {})
    sample = {"monotonic_s":1, "epoch_s":1800000000}
    async def scenario():
        for identifier in ("first", "second"):
            cursor = ReceiptIndex("claude", "sid").capture(path)
            slot.store.exec(1, identifier + ":append", sample, {"kind":"append", "text":"Olá", "delivered":False,
                "ts":None, "pre_transcript":False, "entry_id":identifier})
            slot.store.exec(1, identifier + ":prepare", sample, {"kind":"prepare", "id":identifier,
                "entry_id":identifier, "payload":{"kind":"input"}})
            slot.store.exec(1, identifier + ":cursor", sample, {"kind":"bind_dispatch", "id":identifier, "cursor":cursor})
            slot.store.exec(1, identifier + ":dispatch", sample, {"kind":"begin_dispatch", "id":identifier, "wire_id":identifier})
            with path.open("a") as stream:
                stream.write(json.dumps({"type":"user", "uuid":identifier, "message":{"role":"user", "content":"Olá"}}) + "\n")
            assert (await bridge.confirm(slot.binding.descriptor()))["confirmed"] == 1
            if reload:
                slot.store = QueueStore(slot.binding.state_path, slot.binding.projection_dir, initial_state("key", 1, "session", []))
        assert all(row["confirmed"] for row in slot.store.state["rows"])
        assert len(slot.store.state["used_occurrences"]) == 2
    try:
        asyncio.run(scenario())
    finally:
        coordinator.close_python_leases()


def test_codex_quiesce_preserves_async_questions(tmp_path, monkeypatch):
    from app.runtime_adapter import LegacyBridge
    from app.runtime_coordinator import Binding, RuntimeCoordinator
    from app.adapters.codex.async_questions import AsyncQuestions
    coordinator = RuntimeCoordinator()
    monkeypatch.setattr(runtime_coordinator, "_current", coordinator)
    slot = coordinator.register(Binding("session", "key", "codex", True, {"key":"key", "thread_id":"thread"},
        str(tmp_path / "chat"), tmp_path / "projection", tmp_path / "state", tmp_path / "lease", 1))
    questions = AsyncQuestions("thread")
    questions.observe({"id":"question", "type":"agentMessage", "delivery":"async",
        "questions":[{"title":"Qual opção?", "options":["A","B"]}]})
    class Client:
        tem_processo_proprio = False
        async def close(self, **kwargs):
            pass
    adapter = SimpleNamespace(_sessions={"session":{"client":Client(), "thread_id":"thread", "async_questions":questions}},
        _subscribers={}, _tmux_watchers={})
    try:
        carry = asyncio.run(LegacyBridge(coordinator, {"codex":adapter}).quiesce(slot.binding.descriptor()))
        assert carry["runtime_state"]["async_questions"] == list(questions._pending.items())
        assert carry["runtime_state"]["async_seen"] == sorted(questions._seen)
    finally:
        coordinator.close_python_leases()


def test_later_rollout_path_rebinds_same_conversation(tmp_path, monkeypatch):
    from app.runtime_coordinator import Binding, RuntimeCoordinator, Phase
    target = Binding("session", "key", "codex", True, {"key":"key", "thread_id":"thread"}, "",
        tmp_path / "projection", tmp_path / "state", tmp_path / "lease", 1)
    class Legacy:
        def binding(self, name, provider):
            import copy
            new = copy.deepcopy(target)
            new.jsonl = str(tmp_path / "rollout.jsonl")
            return new
    coordinator = RuntimeCoordinator(legacy=Legacy())
    monkeypatch.setattr(runtime_coordinator, "_current", coordinator)
    slot = coordinator.register(target)
    slot.phase = Phase.Rust
    calls = []
    async def change(name, action, **kwargs):
        calls.append(kwargs)
        slot.binding.jsonl = str(tmp_path / "rollout.jsonl")
        slot.phase = Phase.Python
    coordinator.change = change
    try:
        assert asyncio.run(coordinator.prepare_session("session", "codex"))
        assert calls == [{"advance":False, "reopen":False}]
        assert slot.binding.generation == 1
    finally:
        coordinator.close_python_leases()


def test_v1_control_waits_for_cli_reply_without_inventing_ack(tmp_path, monkeypatch):
    from app.runtime_adapter import LegacyIO, _legacy_operation
    from app.runtime_coordinator import Binding, RuntimeCoordinator
    coordinator = RuntimeCoordinator()
    monkeypatch.setattr(runtime_coordinator, "_current", coordinator)
    slot = coordinator.register(Binding("session", "key", "codex", True, {"key":"key"},
        str(tmp_path / "chat"), tmp_path / "projection", tmp_path / "state", tmp_path / "lease", 1))
    async def scenario():
        io = LegacyIO(coordinator)
        endpoint = SimpleNamespace(runtime_acks={})
        context = {"operation_id":"v1-control", "command":{"kind":"list_models", "payload":{}}}
        token = _legacy_operation.set(context)
        coordinator.legacy_active.add("v1-control")
        writes = []
        class Writer:
            def write(self, raw):
                writes.append(raw)
            async def drain(self):
                pass
        try:
            ticket = await io.write("session", endpoint, Writer(), {"id":"request", "method":"model/list", "params":{}}, 1)
            assert slot.store.state["operations"][ticket.phase_id]["status"] == "unknown"
            await io.reply("session", endpoint, {"id":"request", "result":{"data":[]}})
            await io.finish_call("session", context)
            assert slot.store.state["operations"]["v1-control"]["status"] == "accepted"
            assert len(writes) == 1 and b"cano_input" not in writes[0]
        finally:
            coordinator.legacy_active.discard("v1-control")
            _legacy_operation.reset(token)
    try:
        asyncio.run(scenario())
    finally:
        coordinator.close_python_leases()


def test_v1_prompt_stays_unknown_until_transcript_proof(tmp_path, monkeypatch):
    import json
    from app.runtime_adapter import LegacyIO, LegacyBridge, _legacy_operation
    from app.runtime_coordinator import Binding, RuntimeCoordinator
    path = tmp_path / "chat.jsonl"
    path.touch()
    coordinator = RuntimeCoordinator()
    monkeypatch.setattr(runtime_coordinator, "_current", coordinator)
    slot = coordinator.register(Binding("session", "key", "claude", True, {"key":"key", "session_id":"sid"},
        str(path), tmp_path / "projection", tmp_path / "state", tmp_path / "lease", 1))
    async def scenario():
        io = LegacyIO(coordinator)
        await io._exec("session", {"kind":"append", "text":"Olá", "delivered":False, "ts":None,
            "pre_transcript":False, "entry_id":"entry"})
        context = {"operation_id":"entry", "entry_id":"entry", "command":{"kind":"input", "payload":{"text":"Olá"}}}
        token = _legacy_operation.set(context)
        coordinator.legacy_active.add("entry")
        writes = []
        class Writer:
            def write(self, raw):
                writes.append(raw)
            async def drain(self):
                pass
        try:
            await io.write("session", SimpleNamespace(runtime_acks={}), Writer(),
                {"type":"user", "message":{"role":"user", "content":"Olá"}}, 1)
            with pytest.raises(RuntimeError):
                await io.finish_call("session", context)
            assert slot.store.state["operations"]["entry"]["status"] == "unknown"
            with pytest.raises(ValueError):
                await io._exec("session", {"kind":"set_delivered", "entry_id":"entry", "value":False, "steered":False})
            path.write_text(json.dumps({"type":"user", "uuid":"echo", "message":{"role":"user", "content":"Olá"}}) + "\n")
            assert (await LegacyBridge(coordinator, {}).confirm(slot.binding.descriptor()))["confirmed"] == 1
            assert slot.store.state["rows"][0]["confirmed"]
            assert len(writes) == 1
        finally:
            coordinator.legacy_active.discard("entry")
            _legacy_operation.reset(token)
    try:
        asyncio.run(scenario())
    finally:
        coordinator.close_python_leases()
