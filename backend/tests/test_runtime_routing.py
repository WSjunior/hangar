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

    async def prepare_session(self, name, provider, *, launch=False, engine_models=None):
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


def test_send_without_rust_answer_is_uncertain_not_failed(monkeypatch):
    owner = Owner("unknown")
    async def lost(name, command, operation_id):
        owner.calls.append((command, operation_id))
        return {"operation_id":operation_id, "disposition":"unknown", "payload":{"transport_lost":True}}
    owner.op = lost
    monkeypatch.setattr(runtime_coordinator, "_current", owner)
    result = asyncio.run(api._send_managed("session", "Olá", "claude", track_entry=True))
    assert result["ok"] and not result["delivered"] and result["uncertain"]
    assert result["error"] is None and result["entry_id"] == owner.calls[0][1]


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


# --- Sessão sem terminal nasce direto no Rust (dono único, Task 2) ---

SID = "11111111-1111-1111-1111-111111111111"


class Transport:
    """Rust falso: conta as operações; `open` pode demorar ou recusar com um código."""
    instance = "runtime"

    def __init__(self, fail=None, delay=0.0, view=None):
        self.ops, self.fail, self.delay, self.view = [], fail, delay, view or {}

    async def op(self, descriptor, command, operation_id, clock):
        from app.rust_server import RustOpError
        self.ops.append((command["kind"], descriptor))
        if command["kind"] == "open":
            await asyncio.sleep(self.delay)
            if self.fail:
                raise RustOpError(f"IPC recusou a operação (503: {self.fail})", 503, self.fail)
            view = {"alive":True, "initialized":False, "iniciando":True,
                    "public_state":{"session":descriptor["name"], "state":"working", "headless":True}, **self.view}
            return {"opened":True, "instance":self.instance, "key":descriptor["key"], "generation":descriptor["generation"],
                    "state":{"key":descriptor["key"], "generation":descriptor["generation"], "revision":1,
                             "view":view, "channels":{}, "error":None}}
        if command["kind"] == "submit":
            return {"operation_id":operation_id, "disposition":"deferred", "payload":{}}
        if command["kind"] == "drain":
            return {"sent":0}
        return {}

    def kinds(self):
        return [kind for kind, _ in self.ops]


@pytest.fixture
def birth(tmp_path, monkeypatch):
    from app import pqueue
    from app.adapters.claude_headless import adapter as A, sessions as S
    from app.adapters.claude_headless.adapter import ClaudeHeadlessAdapter
    from app.runtime_adapter import LegacyBridge
    monkeypatch.setattr(pqueue.settings, "projects_dir", tmp_path / "projects")
    monkeypatch.setattr(S, "_dir", lambda: tmp_path / "hl")
    monkeypatch.setattr(A.log_paths, "base", lambda: tmp_path / "logs")
    monkeypatch.setattr(A, "_dir_marcadores", lambda meta: tmp_path / "state")
    monkeypatch.setattr(A, "_esforco_padrao", lambda config_dir: None)
    monkeypatch.setattr(A, "_marca_config", lambda config_dir: "marca")
    S.save("s1", str(tmp_path), SID, model="haiku", permission_mode="manual")
    state = SimpleNamespace(launches=[], kills=[])
    async def launch(argv, *, cwd, env, key, log, tarefas=None):
        state.launches.append(argv)
        return {"pid":999_999_999, "escuta":"unix:" + str(tmp_path / "cano.sock"), "token":"t", "ts":1.0}, SimpleNamespace(pid=999_999_999)
    monkeypatch.setattr(A, "subir_cano_processo", launch)
    monkeypatch.setattr(A, "_matar_grupo", lambda pid, name: state.kills.append(pid))
    async def no_client(*args, **kwargs):
        pytest.fail("cliente Python aberto no cano de uma sessão migrada")
    monkeypatch.setattr(A, "conectar_cano", no_client)
    monkeypatch.setattr(ClaudeHeadlessAdapter, "_conectar", no_client)
    adapter = ClaudeHeadlessAdapter()
    def build(transport):
        owner = runtime_coordinator.RuntimeCoordinator(transport)
        owner.legacy = LegacyBridge(owner, {"claude":adapter})
        monkeypatch.setattr(runtime_coordinator, "_current", owner)
        return owner
    state.adapter, state.build, state.sessions = adapter, build, S
    yield state
    current = runtime_coordinator._current
    if current is not None:
        current.close_python_leases()


def test_new_headless_session_never_opens_python_client(birth):
    transport = Transport()
    owner = birth.build(transport)
    birth.adapter._subidas["s1"] = 3   # teto esgotado: acordar é ação do usuário e abre outra rodada
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        birth.adapter.acordar("s1")
        for _ in range(200):
            if transport.ops:
                break
            await asyncio.sleep(0.01)
        await asyncio.sleep(0.05)
    asyncio.run(scenario())
    assert transport.kinds() == ["open"]
    assert "s1" not in birth.adapter._subidas, "abrir zera o teto de subidas"
    cano = birth.sessions.load("s1")["cano"]
    assert cano["pid"] == 999_999_999 and cano["versao"] == 2
    assert transport.ops[0][1]["meta"]["cano"]["pid"] == 999_999_999
    assert len(birth.launches) == 1
    slot = owner.slot("s1")
    assert slot.phase == runtime_coordinator.Phase.Rust and slot.store is None and slot.lease is None


def test_first_message_right_after_create_is_delivered_once(birth, monkeypatch):
    transport = Transport(delay=0.3)
    owner = birth.build(transport)
    monkeypatch.setattr(api, "_enviar_nativo", lambda *args: pytest.fail("socket nativo direto"))
    monkeypatch.setattr(api.PromptQueue, "append", lambda *args, **kwargs: pytest.fail("fila Python"))
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        birth.adapter.acordar("s1")
        await asyncio.sleep(0.1)
        return await api._send_one_headless("s1", "Olá")
    result = asyncio.run(scenario())
    assert result["ok"] and not result["delivered"]
    assert transport.kinds() == ["open", "submit"], "abre uma vez; o Rust entrega depois do initialize"
    assert len(birth.launches) == 1


def test_open_connect_failure_kills_launched_cano(birth):
    owner = birth.build(Transport(fail="cano_connect"))
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        await owner.ensure_open("s1")
    with pytest.raises(Exception) as caught:
        asyncio.run(scenario())
    assert caught.value.code == "cano_connect"
    assert birth.kills == [999_999_999]
    assert birth.sessions.load("s1").get("cano") is None
    assert birth.adapter._subidas["s1"] == 1, "a falha conta no teto de subidas"
    assert not owner.managed_queue("s1"), "sem dono registrado depois da falha"
    assert birth.adapter._problemas["s1"][0] == "headless_nao_subiu", "a falha aparece na faixa"
    assert birth.sessions.load("s1")["problema"][1].startswith("cano_connect")


def test_never_relaunch_while_sidecar_pid_alive(birth):
    import os
    alive = {"pid":os.getpid(), "escuta":"unix:/tmp/vivo.sock", "token":"t", "ts":1.0, "versao":2}
    birth.sessions.update("s1", cano=alive)
    transport = Transport()
    owner = birth.build(transport)
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        await owner.ensure_open("s1")
    asyncio.run(scenario())
    assert birth.launches == []
    assert transport.ops[0][1]["meta"]["cano"]["pid"] == os.getpid()


def test_migrated_send_never_falls_to_legacy_path(birth, monkeypatch):
    owner = birth.build(Transport(fail="queue_io"))
    monkeypatch.setattr(api, "_enviar_nativo", lambda *args: pytest.fail("socket nativo direto"))
    monkeypatch.setattr(api.PromptQueue, "append", lambda *args, **kwargs: pytest.fail("fila Python"))
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        return await api._send_one_headless("s1", "Olá")
    result = asyncio.run(scenario())
    assert not result["ok"]
    assert "queue_io" in str(result["error"])
    assert birth.kills == [], "recusa que não é de conexão não mata o cano"


def test_wait_initialized_reports_cli_refusal(birth):
    view = {"iniciando":False, "public_state":{"session":"s1", "state":"idle", "headless":True,
            "problema":"headless_nao_subiu", "problema_detalhe":"initialize recusado: conta sem acesso"}}
    owner = birth.build(Transport(view=view))
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        await owner.ensure_open("s1", wait_initialized=True)
    with pytest.raises(RuntimeError, match="conta sem acesso"):
        asyncio.run(scenario())


def test_reads_and_stop_never_launch_a_process(birth):
    transport = Transport()
    owner = birth.build(transport)
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        assert await birth.adapter.deliverable("s1") is False
        await birth.adapter.parar("s1")
    asyncio.run(scenario())
    assert birth.launches == [] and transport.kinds() == []


def test_ensure_running_opens_in_rust_with_engine_models(birth, monkeypatch):
    seen = []
    original = birth.adapter.launch_process
    async def launch(name, *, engine_models=None, launch=True):
        seen.append(engine_models)
        return await original(name, engine_models=engine_models, launch=launch)
    monkeypatch.setattr(birth.adapter, "launch_process", launch)
    view = {"initialized":True, "iniciando":False}
    transport = Transport(view=view)
    owner = birth.build(transport)
    models = [{"id":"modelo-da-conta"}]
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        await birth.adapter.ensure_running("s1", require_initialize=True, engine_models=models)
    asyncio.run(scenario())
    assert seen == [models] and transport.kinds() == ["open"]


def test_python_registration_without_cano_moves_to_rust_on_send(birth, monkeypatch):
    transport = Transport()
    owner = birth.build(transport)
    monkeypatch.setattr(api, "_enviar_nativo", lambda *args: pytest.fail("socket nativo direto"))
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        await birth.adapter.deliverable("s1")      # leitura registra no Python, sem processo
        assert owner.slot("s1").phase == runtime_coordinator.Phase.Python
        return await api._send_one_headless("s1", "Olá")
    result = asyncio.run(scenario())
    assert result["ok"]
    slot = owner.slot("s1")
    assert slot.phase == runtime_coordinator.Phase.Rust and slot.lease is None
    assert transport.kinds() == ["open", "submit"]

