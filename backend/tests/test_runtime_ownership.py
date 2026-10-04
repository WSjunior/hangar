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
        if command["kind"] == "open":
            self.lease = WriterLease(target["lock_path"])
            return {"opened": True, "instance": self.instance, "key": target["key"],
                    "generation": target["generation"], "state": {"alive": True}}
        if command["kind"] == "close":
            self.lease.close()
            self.lease = None
            return {"closed": True}
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
            if command["kind"] == "open":
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


def test_refused_adopt_goes_to_python_only_after_the_fourth_failure(tmp_path):
    async def flow():
        legacy, gateway = Legacy(), Gateway()
        calls = []
        original = gateway.op
        async def refuse(target, command, operation_id, clock):
            calls.append(command["kind"])
            if command["kind"] == "open":
                raise RuntimeError("IPC recusou a operação (503: cano_binding snapshot de outro cano)")
            return await original(target, command, operation_id, clock)
        gateway.op = refuse
        coordinator = RuntimeCoordinator(gateway, legacy, peek)
        coordinator.register(binding(tmp_path))
        for _ in range(3):
            assert await coordinator.adopt("session") is False
            # Cada recusa devolve a sessão ao Python, mas a próxima ação ainda tenta o Rust.
            assert coordinator.legacy_allowed("key", 1)
            assert coordinator.slot("session").rust_refused is None
        assert await coordinator.adopt("session") is False
        assert coordinator.slot("session").rust_refused == 1
        assert calls.count("open") == 4
        assert legacy.events == ["quiesce", "reconnect"] * 4
        coordinator.close_python_leases()
    asyncio.run(flow())


def test_unconfirmed_detach_never_makes_two_owners(tmp_path):
    async def flow():
        legacy, gateway = Legacy(), Gateway()
        original = gateway.op
        async def dead_actor(target, command, operation_id, clock):
            if command["kind"] == "open":
                await original(target, command, operation_id, clock)
                raise RuntimeError("IPC recusou a operação (503: runtime_initialize)")
            if command["kind"] == "close":
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
            if command["kind"] == "close":
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


def _python_ops(legacy):
    sent = []
    async def legacy_op(target, command, operation_id):
        sent.append(command["kind"])
        return {"accepted": True, "via": "python"}
    legacy.op = legacy_op
    return sent


@pytest.fixture
def no_pause(monkeypatch):
    from app import runtime_coordinator
    monkeypatch.setattr(runtime_coordinator, "_RETRY_PAUSE_S", 0)


def test_lost_state_without_answer_goes_to_python_after_four_tries(tmp_path, no_pause):
    async def flow():
        legacy, gateway = Legacy(), Gateway()
        sent = _python_ops(legacy)
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
        result = await coordinator.op("session", {"kind": "submit", "text": "Olá"}, "op-1")
        assert result["via"] == "python"
        assert sent == ["submit"]
        assert slot.rust_refused == 1
        coordinator.close_python_leases()
    asyncio.run(flow())


def _refusing(gateway, code, fail_times):
    from app.rust_server import RustOpError
    original, attempts = gateway.op, []
    async def op(target, command, operation_id, clock):
        if command["kind"] == "submit":
            attempts.append(operation_id)
            if len(attempts) <= fail_times:
                raise RustOpError(f"IPC recusou a operação (503: {code})", 503, code)
        return await original(target, command, operation_id, clock)
    gateway.op = op
    return attempts


def test_pre_effect_refusal_is_retried_and_stays_in_rust(tmp_path, no_pause):
    async def flow():
        legacy, gateway = Legacy(), Gateway()
        sent = _python_ops(legacy)
        attempts = _refusing(gateway, "cano_connect", fail_times=3)
        coordinator = RuntimeCoordinator(gateway, legacy, peek)
        slot = coordinator.register(binding(tmp_path))
        await coordinator.adopt("session")
        assert await coordinator.op("session", {"kind": "submit", "text": "Olá"}, "op-1") == {"accepted": True}
        assert attempts == ["op-1"] * 4
        assert sent == [] and slot.rust_refused is None
        gateway.lease.close()
    asyncio.run(flow())


def test_fourth_pre_effect_refusal_sends_through_python(tmp_path, no_pause):
    async def flow():
        legacy, gateway = Legacy(), Gateway()
        sent = _python_ops(legacy)
        attempts = _refusing(gateway, "cano_connect", fail_times=99)
        coordinator = RuntimeCoordinator(gateway, legacy, peek)
        slot = coordinator.register(binding(tmp_path))
        await coordinator.adopt("session")
        result = await coordinator.op("session", {"kind": "submit", "text": "Olá"}, "op-1")
        assert len(attempts) == 4
        assert result["via"] == "python" and sent == ["submit"]
        assert slot.rust_refused == 1
        # Depois de passar para o Python, a sessão não volta a tentar o Rust nesta vida do backend.
        await coordinator.op("session", {"kind": "submit", "text": "de novo"}, "op-2")
        assert len(attempts) == 4 and sent == ["submit", "submit"]
        coordinator.close_python_leases()
    asyncio.run(flow())


def test_failure_after_possible_effect_is_never_repeated(tmp_path, no_pause):
    async def flow():
        from app.rust_server import RustOpError
        legacy, gateway = Legacy(), Gateway()
        sent = _python_ops(legacy)
        attempts = _refusing(gateway, "queue_io", fail_times=99)
        coordinator = RuntimeCoordinator(gateway, legacy, peek)
        slot = coordinator.register(binding(tmp_path))
        await coordinator.adopt("session")
        with pytest.raises(RustOpError):
            await coordinator.op("session", {"kind": "submit", "text": "Olá"}, "op-1")
        # Uma tentativa só, nada reenviado pelo Python; a sessão já passou para ele.
        assert attempts == ["op-1"] and sent == []
        assert slot.rust_refused == 1 and coordinator.legacy_allowed("key", 1)
        coordinator.close_python_leases()
    asyncio.run(flow())


def test_normal_answer_from_rust_is_not_a_failure(tmp_path, no_pause):
    async def flow():
        from app.rust_server import RustOpError
        legacy, gateway = Legacy(), Gateway()
        sent = _python_ops(legacy)
        attempts = _refusing(gateway, "claude_command", fail_times=99)
        coordinator = RuntimeCoordinator(gateway, legacy, peek)
        slot = coordinator.register(binding(tmp_path))
        await coordinator.adopt("session")
        # Ex.: "nenhuma permissão pendente" de um botão velho: sobe como erro, a sessão fica no Rust.
        with pytest.raises(RustOpError):
            await coordinator.op("session", {"kind": "submit", "text": "Olá"}, "op-1")
        assert attempts == ["op-1"] and sent == []
        assert slot.rust_refused is None and not coordinator.legacy_allowed("key", 1)
        gateway.lease.close()
    asyncio.run(flow())


def test_slow_disk_write_does_not_block_the_event_loop(tmp_path, monkeypatch):
    import time
    from app import runtime_queue
    from app.runtime_adapter import LegacyIO

    async def flow():
        coordinator = RuntimeCoordinator(Gateway(), Legacy(), peek)
        slot = coordinator.register(binding(tmp_path))
        original = runtime_queue.QueueStore._atomic
        def slow(path, data):
            time.sleep(0.5)
            original(path, data)
        monkeypatch.setattr(runtime_queue.QueueStore, "_atomic", staticmethod(slow))
        write = asyncio.create_task(LegacyIO(coordinator)._exec("session", {"kind": "set_runtime_state", "state": {"a": 1}}))
        await asyncio.sleep(0.1)          # a gravação está no disco, numa thread
        started = time.monotonic()
        with slot.guard:                  # é o que o leitor de eventos faz em cada evento
            pass
        assert time.monotonic() - started < 0.1
        await write
        coordinator.close_python_leases()
    asyncio.run(flow())


def test_python_side_handoff_failure_does_not_count_against_rust(tmp_path):
    async def flow():
        legacy, gateway = Legacy(), Gateway()
        async def broken_quiesce(target):
            legacy.events.append("quiesce")
            raise ValueError("synthetic Python")
        legacy.quiesce = broken_quiesce
        coordinator = RuntimeCoordinator(gateway, legacy, peek)
        slot = coordinator.register(binding(tmp_path))
        for _ in range(5):
            assert await coordinator.adopt("session") is False
        assert slot.adopt_failures == 0 and slot.rust_refused is None
        coordinator.close_python_leases()
    asyncio.run(flow())


def test_dead_cano_after_rust_lets_go_parks_the_session_in_python(tmp_path):
    # O Rust soltou e a trava voltou: sem cano para religar, a sessão não pode ficar presa em
    # "recuperando" recusando tudo; ela fica no Python e sobe de novo no próximo envio.
    async def flow():
        legacy, gateway = Legacy(), Gateway()
        coordinator = RuntimeCoordinator(gateway, legacy, peek)
        coordinator.register(binding(tmp_path))
        await coordinator.adopt("session")
        async def dead(target, carry):
            raise RuntimeError("reserva não reconectou ao cano existente")
        legacy.reconnect = dead
        await coordinator.detach("session")
        assert coordinator.legacy_allowed("key", 1)
        coordinator.close_python_leases()
    asyncio.run(flow())
