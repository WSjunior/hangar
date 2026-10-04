import asyncio
import subprocess
import sys
from contextlib import contextmanager

import pytest

from app.runtime_coordinator import Binding, RuntimeCoordinator, WriterLease


def binding(tmp_path, headless=True):
    return Binding(name="session", key="key", provider="claude", headless=headless,
        meta={"key": "key", "cano": {"pid": 42, "escuta": "tcp:127.0.0.1:1", "token": "test", "versao": 2}},
        jsonl=str(tmp_path / "chat.jsonl"), projection_dir=tmp_path / "projection",
        state_path=tmp_path / "key.queue-state.json", lock_path=tmp_path / "key.lock", generation=1)


class Legacy:
    def __init__(self):
        self.events = []

    async def quiesce(self, target):
        self.events.append("quiesce")
        return {"runtime_state": {}, "pending": []}

    async def reconnect(self, target, carry):
        self.events.append("reconnect")
        return {"hydrated": True}


class Gateway:
    instance = "instance-1"

    def __init__(self):
        self.lease = None
        self.alive = True
        self.fail = False

    async def op(self, target, command, operation_id, clock):
        if self.fail:
            raise TimeoutError("synthetic IPC")
        if command["kind"] == "adopt":
            self.lease = WriterLease(target["lock_path"])
            return {"ready": True, "instance": self.instance, "key": target["key"],
                    "generation": target["generation"], "state": {"alive": True}}
        if command["kind"] == "detach":
            self.lease.close()
            self.lease = None
            return {"detached": True}
        return {"accepted": True}


async def peek(target):
    return {"type": "cano_snapshot", "versao": 2, "pendentes": [], "inflight": {}}


def test_two_processes_one_lease(tmp_path):
    path = tmp_path / "lease"
    lease = WriterLease(path)
    try:
        script = "from app.runtime_coordinator import WriterLease\nimport sys\ntry:\n WriterLease(sys.argv[1])\nexcept BlockingIOError:\n sys.exit(0)\nsys.exit(1)"
        result = subprocess.run([sys.executable, "-c", script, str(path)], timeout=5)
        assert result.returncode == 0
    finally:
        lease.close()


def test_failed_peek_keeps_python(tmp_path):
    async def fail(target):
        raise OSError("snapshot")
    legacy, gateway = Legacy(), Gateway()
    coordinator = RuntimeCoordinator(gateway, legacy, fail)
    coordinator.register(binding(tmp_path))
    with pytest.raises(OSError):
        asyncio.run(coordinator.adopt("session"))
    assert coordinator.legacy_allowed("key", 1)
    assert legacy.events == []
    coordinator.close_python_leases()


def test_quiesce_precedes_release_and_reserve_requires_detach(tmp_path):
    async def flow():
        legacy, gateway = Legacy(), Gateway()
        coordinator = RuntimeCoordinator(gateway, legacy, peek)
        coordinator.register(binding(tmp_path))
        assert await coordinator.adopt("session")
        assert legacy.events == ["quiesce"]
        assert not coordinator.legacy_allowed("key", 1)
        await coordinator.detach("session")
        assert coordinator.legacy_allowed("key", 1)
        assert legacy.events == ["quiesce", "reconnect"]
        coordinator.close_python_leases()
    asyncio.run(flow())


def test_timeout_is_not_fallback(tmp_path):
    async def flow():
        legacy, gateway = Legacy(), Gateway()
        coordinator = RuntimeCoordinator(gateway, legacy, peek)
        coordinator.register(binding(tmp_path))
        await coordinator.adopt("session")
        gateway.fail = True
        with pytest.raises(TimeoutError):
            await coordinator.op("session", {"kind": "submit", "text": "Olá"}, "op-1")
        with pytest.raises(RuntimeError):
            await coordinator.recover("session", confirmed_dead=True)
        assert legacy.events == ["quiesce"]
        assert not coordinator.legacy_allowed("key", 1)
        gateway.lease.close()
    asyncio.run(flow())


def test_managed_queue_survives_terminal_mode(tmp_path):
    async def flow():
        coordinator = RuntimeCoordinator()
        target = binding(tmp_path)
        coordinator.register(target)
        async with coordinator.freeze("session"):
            target.headless = False
            coordinator.register(target)
        assert coordinator.managed_queue("session")
        assert not coordinator.managed_runtime("session")
        coordinator.close_python_leases()
    asyncio.run(flow())


def test_late_ready_rejected(tmp_path):
    async def flow():
        legacy, gateway = Legacy(), Gateway()
        original = gateway.op
        async def stale(target, command, operation_id, clock):
            result = await original(target, command, operation_id, clock)
            if command["kind"] == "adopt":
                result["generation"] = 0
            return result
        gateway.op = stale
        coordinator = RuntimeCoordinator(gateway, legacy, peek)
        coordinator.register(binding(tmp_path))
        assert await coordinator.adopt("session") is False
        assert gateway.lease is None
        assert coordinator.legacy_allowed("key", 1)
        coordinator.close_python_leases()
    asyncio.run(flow())


def test_refused_adopt_stays_in_python_for_this_generation(tmp_path):
    async def flow():
        legacy, gateway = Legacy(), Gateway()
        calls = []
        original = gateway.op
        async def refuse(target, command, operation_id, clock):
            calls.append(command["kind"])
            if command["kind"] == "adopt":
                raise RuntimeError("IPC recusou a operação (503: cano_binding snapshot de outro cano)")
            return await original(target, command, operation_id, clock)
        gateway.op = refuse
        coordinator = RuntimeCoordinator(gateway, legacy, peek)
        coordinator.register(binding(tmp_path))
        assert await coordinator.adopt("session") is False
        assert legacy.events == ["quiesce", "reconnect"]
        assert coordinator.legacy_allowed("key", 1)
        assert coordinator.slot("session").rust_refused == 1
        coordinator.close_python_leases()
    asyncio.run(flow())


def test_unconfirmed_detach_never_makes_two_owners(tmp_path):
    async def flow():
        legacy, gateway = Legacy(), Gateway()
        original = gateway.op
        async def dead_actor(target, command, operation_id, clock):
            if command["kind"] == "adopt":
                await original(target, command, operation_id, clock)
                raise RuntimeError("IPC recusou a operação (503: runtime_initialize)")
            if command["kind"] == "detach":
                raise RuntimeError("IPC recusou a operação (503: runtime_closed)")
            return await original(target, command, operation_id, clock)
        gateway.op = dead_actor
        coordinator = RuntimeCoordinator(gateway, legacy, peek)
        coordinator.register(binding(tmp_path))
        # O Rust ainda segura o lock: a reserva não pode assumir.
        with pytest.raises(BlockingIOError):
            await coordinator.adopt("session")
        assert legacy.events == ["quiesce"]
        gateway.lease.close()
    asyncio.run(flow())


def test_detach_and_lock_before_reserve(tmp_path):
    async def flow():
        legacy, gateway = Legacy(), Gateway()
        coordinator = RuntimeCoordinator(gateway, legacy, peek)
        coordinator.register(binding(tmp_path))
        await coordinator.adopt("session")
        original = gateway.op
        interloper = []
        async def detach_then_other_owner(target, command, operation_id, clock):
            result = await original(target, command, operation_id, clock)
            if command["kind"] == "detach":
                interloper.append(WriterLease(target["lock_path"]))
            return result
        gateway.op = detach_then_other_owner
        try:
            with pytest.raises(BlockingIOError):
                await coordinator.detach("session")
            assert not coordinator.legacy_allowed("key", 1)
            assert legacy.events == ["quiesce"]
        finally:
            for lease in interloper:
                lease.close()
    asyncio.run(flow())


def test_recover_dispatch_as_unknown(tmp_path):
    async def flow():
        legacy, gateway = Legacy(), Gateway()
        coordinator = RuntimeCoordinator(gateway, legacy, peek)
        slot = coordinator.register(binding(tmp_path))
        slot.store.exec(1, "prepare", {"monotonic_s": 1, "epoch_s": 1},
                        {"kind": "prepare", "id": "op", "payload": {}, "entry_id": None})
        await coordinator.adopt("session")
        from app.runtime_queue import QueueStore, initial_state
        native_store = QueueStore(slot.binding.state_path, slot.binding.projection_dir,
                                  initial_state("key", 1, "session", []))
        native_store.exec(1, "dispatch", {"monotonic_s": 1, "epoch_s": 1},
                          {"kind": "begin_dispatch", "id": "op", "wire_id": "wire:1"})
        gateway.lease.close()
        gateway.alive = False
        await coordinator.recover("session", confirmed_dead=True)
        assert slot.store.state["operations"]["op"]["status"] == "unknown"
        assert coordinator.legacy_allowed("key", 1)
        coordinator.close_python_leases()
    asyncio.run(flow())


def test_rust_stuck_in_error_hands_session_back_to_python(tmp_path):
    async def flow():
        legacy, gateway = Legacy(), Gateway()
        sent = []
        async def legacy_op(target, command, operation_id):
            sent.append(command["kind"])
            return {"accepted": True}
        legacy.op = legacy_op
        original = gateway.op
        async def broken(target, command, operation_id, clock):
            if command["kind"] == "snapshot":
                return {"key": "key", "generation": 1, "revision": 5, "channels": {}, "error": "cano_closed",
                        "view": {"public_state": {"session": "session", "state": "idle", "headless": True}}}
            if command["kind"] == "submit":
                raise AssertionError("o Rust em erro não pode receber o envio")
            return await original(target, command, operation_id, clock)
        gateway.op = broken
        coordinator = RuntimeCoordinator(gateway, legacy, peek)
        slot = coordinator.register(binding(tmp_path))
        await coordinator.adopt("session")
        slot.cache_valid = False
        await coordinator.op("session", {"kind": "submit", "text": "Olá"}, "op-1")
        assert sent == ["submit"]
        assert legacy.events == ["quiesce", "reconnect"]
        assert slot.rust_refused == 1
        assert coordinator.legacy_allowed("key", 1)
        coordinator.close_python_leases()
    asyncio.run(flow())


def test_rust_without_answer_keeps_the_error(tmp_path):
    async def flow():
        legacy, gateway = Legacy(), Gateway()
        original = gateway.op
        async def silent(target, command, operation_id, clock):
            if command["kind"] == "snapshot":
                raise TimeoutError("synthetic IPC")
            return await original(target, command, operation_id, clock)
        gateway.op = silent
        coordinator = RuntimeCoordinator(gateway, legacy, peek)
        slot = coordinator.register(binding(tmp_path))
        await coordinator.adopt("session")
        slot.cache_valid = False
        with pytest.raises(RuntimeError, match="aguarde a reposição"):
            await coordinator.op("session", {"kind": "submit", "text": "Olá"}, "op-1")
        assert legacy.events == ["quiesce"]
        gateway.lease.close()
    asyncio.run(flow())
