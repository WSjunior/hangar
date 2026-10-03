import asyncio
import json

import pytest

from app import runtime_coordinator, runtime_queue
from app.runtime_coordinator import Binding, RuntimeCoordinator, WriterLease
from app.runtime_queue import QueueStore, initial_state

CLOCK = {"monotonic_s":1.0, "epoch_s":1800000000.0}
POINTS = ("before_prepare", "after_prepare", "before_write", "partial_write", "after_write",
          "before_reply", "between_state_and_projection", "after_detach")


@pytest.mark.parametrize("point", POINTS)
def test_crash_boundary_keeps_intent_and_single_writer(tmp_path, monkeypatch, point):
    monkeypatch.setattr(runtime_coordinator, "_current", None)
    monkeypatch.setattr(runtime_queue, "_coordinator", None)
    lease_path = tmp_path / "key.lock"
    lease = WriterLease(lease_path)
    path, projection = tmp_path / "key.state", tmp_path / "projection"
    def open_store():
        return QueueStore(path, projection, initial_state("key", 1, "session", []))
    store = open_store()
    store.exec(1, "append", CLOCK, {"kind":"append", "text":"Olá\r\nação 🚀", "delivered":False,
        "ts":None, "pre_transcript":False, "entry_id":"entry"})
    try:
        with pytest.raises(BlockingIOError):
            WriterLease(lease_path)
        if point != "before_prepare":
            store.exec(1, "prepare", CLOCK, {"kind":"prepare", "id":"entry", "payload":{"kind":"input"}, "entry_id":"entry"})
        dispatched = point in {"partial_write", "after_write", "before_reply", "between_state_and_projection", "after_detach"}
        if dispatched:
            store.exec(1, "dispatch", CLOCK, {"kind":"begin_dispatch", "id":"entry", "wire_id":"wire"})
        if point == "between_state_and_projection":
            original = store.ensure_projection
            monkeypatch.setattr(store, "ensure_projection", lambda: (_ for _ in ()).throw(OSError("projection")))
            with pytest.raises(OSError):
                store.exec(1, "reply", CLOCK, {"kind":"finish", "id":"entry", "status":"accepted",
                    "result":{"operation_id":"entry", "disposition":"accepted", "payload":{}}})
            monkeypatch.setattr(store, "ensure_projection", original)
        lease.close()
        lease = WriterLease(lease_path)
        reserve = open_store()
        reserve.exec(1, "recover", CLOCK, {"kind":"recover"})
        assert reserve.state["owner_key"] == "key"
        assert reserve.state["rows"][0]["text"] == "Olá\r\nação 🚀"
        assert len(reserve.state["rows"]) == 1
        if dispatched and point != "between_state_and_projection":
            assert reserve.state["operations"]["entry"]["status"] == "unknown"
            with pytest.raises(ValueError):
                reserve.exec(1, "retry", CLOCK, {"kind":"begin_dispatch", "id":"entry", "wire_id":"retry"})
        if point == "between_state_and_projection":
            assert reserve.state["operations"]["entry"]["status"] == "accepted"
        with pytest.raises(BlockingIOError):
            WriterLease(lease_path)
    finally:
        lease.close()


def test_partial_wire_then_new_owner_uses_same_cano(tmp_path, monkeypatch):
    from app.runtime_adapter import LegacyIO
    from types import SimpleNamespace
    target = Binding("session", "key", "claude", True, {"key":"key", "cano":{"pid":42}},
        str(tmp_path / "chat"), tmp_path / "projection", tmp_path / "state", tmp_path / "lease", 1)
    seen = []
    class Legacy:
        async def reconnect(self, descriptor, carry):
            seen.append(descriptor["meta"]["cano"]["pid"])
            return {"hydrated":True}
    coordinator = RuntimeCoordinator(legacy=Legacy())
    monkeypatch.setattr(runtime_coordinator, "_current", coordinator)
    slot = coordinator.register(target)
    class Writer:
        def write(self, raw):
            seen.append("partial")
        async def drain(self):
            raise OSError("connection lost")
    async def scenario():
        with pytest.raises(RuntimeError):
            await LegacyIO(coordinator).write("session", SimpleNamespace(runtime_acks={}), Writer(),
                {"type":"user", "message":{"role":"user", "content":"Olá"}}, 2)
        slot.lease.close()
        slot.lease = None
        await coordinator.recover("session", confirmed_dead=True)
        assert any(operation["status"] == "unknown" for operation in slot.store.state["operations"].values())
    try:
        asyncio.run(scenario())
        assert seen == ["partial", 42]
    finally:
        coordinator.close_python_leases()
