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
            view={"key":"key","generation":2,"revision":7,"view":{"alive":True,"ready":True,
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
