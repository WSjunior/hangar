import asyncio
from types import SimpleNamespace

import pytest

from app import api, runtime_coordinator, sse


class Owner:
    instance = "runtime"

    def __init__(self, disposition="accepted"):
        self.calls = []
        self.disposition = disposition
        self.loop = None

    def managed_runtime(self, name):
        return name == "session"

    def managed_queue(self, name):
        return self.managed_runtime(name)

    async def prepare_session(self, name, provider):
        return self.managed_runtime(name)

    async def op(self, name, command, operation_id):
        self.calls.append((command, operation_id))
        if command["kind"] == "drain":
            return {"sent":1}
        if command["kind"] == "confirm":
            return {"confirmed":1}
        return {"operation_id":operation_id, "disposition":self.disposition, "payload":{}}


@pytest.mark.parametrize("provider,path", [
    ("claude", "input"), ("codex", "input"), ("claude", "group"),
    ("claude", "pair"), ("codex", "peer"), ("claude", "guest"),
])
def test_all_send_producers_use_one_operation(monkeypatch, provider, path):
    owner = Owner()
    monkeypatch.setattr(runtime_coordinator, "_current", owner)
    monkeypatch.setattr(api, "_provider_of", lambda name: provider)
    monkeypatch.setattr(api, "_headless", lambda name: provider == "claude")
    monkeypatch.setattr(api, "_session_exists", lambda name: True)
    monkeypatch.setattr(api, "_enviar_nativo", lambda *args: pytest.fail("UDS antes do diário"))
    monkeypatch.setattr(api.PromptQueue, "append", lambda *args, **kwargs: pytest.fail("append fora do ator"))
    async def scenario():
        result = await api._enviar("session", "[de: sender] Olá" if path != "input" else "Olá")
        assert result["ok"] and result["delivered"]
    asyncio.run(scenario())
    assert len(owner.calls) == 1
    assert owner.calls[0][0]["kind"] == "submit"
    assert owner.calls[0][1]


def test_unknown_not_marked_unsent(monkeypatch):
    owner = Owner("unknown")
    monkeypatch.setattr(runtime_coordinator, "_current", owner)
    result = asyncio.run(api._send_managed("session", "Olá", "claude", track_entry=True))
    assert not result["ok"]
    assert result["entry_id"] == owner.calls[0][1]
    assert len(owner.calls) == 1


@pytest.mark.parametrize("source", ["hook", "drain_session", "turn_end", "codex_confirm"])
def test_background_producers_reach_owner(monkeypatch, source):
    owner = Owner()
    monkeypatch.setattr(runtime_coordinator, "_current", owner)
    monkeypatch.setattr(api, "_cached_info_sync", lambda name: SimpleNamespace(jsonl="chat", provider="claude"))
    monkeypatch.setattr(api.PromptQueue, "load", lambda *args: pytest.fail("confirmação por texto antigo"))
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        if source == "codex_confirm":
            await asyncio.to_thread(sse._confirm_codex_queue, "session", "chat")
        elif source == "turn_end":
            await asyncio.to_thread(api._confirm_and_drain, "session")
        elif source == "drain_session":
            await asyncio.to_thread(api._drain_session, "session")
        else:
            await asyncio.to_thread(api._drenar, "session", "chat", "claude")
    asyncio.run(scenario())
    assert [call[0]["kind"] for call in owner.calls] == (["confirm", "drain"] if source == "turn_end" else ["confirm"] if source == "codex_confirm" else ["drain"])


def test_terminal_and_other_providers_keep_legacy_route(monkeypatch):
    monkeypatch.setattr(runtime_coordinator, "_current", Owner())
    monkeypatch.setattr(api, "_headless", lambda name: False)
    monkeypatch.setattr(api, "drain", lambda *args: 3)
    assert api._drenar("terminal", "chat", "pi") == 3


def test_registered_v2_is_adopted_without_opening_python_reader(tmp_path, monkeypatch):
    from app.runtime_coordinator import Binding, RuntimeCoordinator, Phase
    target = Binding("session", "key", "claude", True, {"key":"key", "cano":{"versao":2}},
        str(tmp_path / "chat"), tmp_path / "projection", tmp_path / "state", tmp_path / "lease", 1)
    class Legacy:
        def binding(self, name, provider):
            return target
    owner = RuntimeCoordinator(SimpleNamespace(instance="runtime"), Legacy())
    monkeypatch.setattr(runtime_coordinator, "_current", owner)
    async def adopt(name):
        owner.slot(name).phase = Phase.Rust
        return True
    owner.adopt = adopt
    try:
        assert asyncio.run(owner.prepare_session("session", "claude"))
        assert owner.slot("session").phase == Phase.Rust
    finally:
        owner.close_python_leases()


def test_native_receipt_updates_registered_operation_only(tmp_path, monkeypatch):
    import json
    import uuid
    from app.runtime_coordinator import Binding, RuntimeCoordinator
    owner = RuntimeCoordinator()
    monkeypatch.setattr(runtime_coordinator, "_current", owner)
    slot = owner.register(Binding("session", "key", "claude", True, {"key":"key"},
        str(tmp_path / "chat"), tmp_path / "projection", tmp_path / "state", tmp_path / "lease", 1))
    slot.store.exec(1, "prepare", {"epoch_s":1, "monotonic_s":1},
        {"kind":"prepare", "id":"entry", "entry_id":None, "payload":{"kind":"input"}})
    calls = []
    async def op(name, command, operation_id):
        calls.append(command)
    owner.op = op
    mid = str(uuid.uuid5(uuid.NAMESPACE_URL, "hangar:key:entry"))
    try:
        assert asyncio.run(owner.native_receipt(mid, "rejected"))
        assert calls[0]["action"]["id"] == "entry"
        assert calls[0]["action"]["status"] == "rejected"
    finally:
        owner.close_python_leases()
