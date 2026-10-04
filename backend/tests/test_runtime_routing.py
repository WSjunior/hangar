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
    from app.adapters.codex import sessions as codex_sessions
    monkeypatch.setattr(codex_sessions, "_dir", lambda: tmp_path / "codex-sessions")
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



# --- Administração da sessão sem terminal por close/open (dono único, Task 4) ---

class LockingTransport(Transport):
    """Rust falso que segura a trava da fila enquanto a sessão está aberta, como o real."""

    def __init__(self, **kwargs):
        super().__init__(**kwargs)
        self.lease, self.revision = None, 1

    async def op(self, descriptor, command, operation_id, clock):
        from app.runtime_coordinator import WriterLease
        kind = command["kind"]
        if kind not in {"close", "snapshot"}:
            result = await super().op(descriptor, command, operation_id, clock)
            if kind == "open":
                self.lease = WriterLease(descriptor["lock_path"])
            return result
        self.ops.append((kind, descriptor))
        if kind == "close":
            if self.lease is not None:
                self.lease.close()
                self.lease = None
            return {"closed":True}
        self.revision += 1
        view = {"alive":True, "initialized":True, "iniciando":False,
                "public_state":{"session":descriptor["name"], "state":"idle", "headless":True}, **self.view}
        return {"key":descriptor["key"], "generation":descriptor["generation"], "revision":self.revision,
                "view":view, "channels":{}, "error":None}


def _alive_cano(birth):
    import os
    birth.sessions.update("s1", cano={"pid":os.getpid(), "escuta":"unix:/tmp/vivo.sock", "token":"t", "ts":1.0, "versao":2})
    return os.getpid()


def _opened(birth, **view):
    pid = _alive_cano(birth)
    transport = LockingTransport(view=view)
    return birth.build(transport), transport, pid


def test_rename_keeps_session_in_rust_without_python_client(birth):
    import json
    owner, transport, _ = _opened(birth)
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        await owner.ensure_open("s1")
        async def rename():
            await asyncio.to_thread(birth.sessions.rename, "s1", "s2")
        await owner.change("s1", rename, new_name="s2", advance=False)
    asyncio.run(scenario())
    assert transport.kinds() == ["open", "snapshot", "close", "open"]
    assert transport.ops[3][1]["name"] == "s2" and transport.ops[3][1]["generation"] == 1
    slot = owner.slot("s2")
    assert slot.phase == runtime_coordinator.Phase.Rust and slot.lease is None and slot.store is None
    assert not owner.managed_queue("s1")
    assert json.loads(slot.binding.state_path.read_bytes())["name"] == "s2"
    assert birth.launches == []


def test_kill_closes_in_rust_and_stops_cano(birth):
    owner, transport, pid = _opened(birth)
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        await owner.ensure_open("s1")
        async def kill():
            meta = birth.sessions.load("s1")
            birth.sessions.delete("s1")
            await asyncio.to_thread(birth.adapter.close_sync, "s1", meta)
        await owner.change("s1", kill, remove=True)
    asyncio.run(scenario())
    assert transport.kinds() == ["open", "snapshot", "close"]
    assert birth.kills == [pid] and not owner.managed_queue("s1")
    assert transport.lease is None


def test_mode_switch_to_terminal_closes_rust_first(birth, monkeypatch):
    from app import runtime_terminal
    owner, transport, pid = _opened(birth)
    monkeypatch.setattr(runtime_terminal, "_collect", lambda name: None)
    monkeypatch.setattr(runtime_terminal, "_session_proof", lambda name: "pane-novo")
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        await owner.ensure_open("s1")
        async def to_terminal():
            assert transport.kinds()[-1] == "close" and transport.lease is None, "o Rust soltou antes da ação"
            meta = birth.sessions.load("s1")
            birth.sessions.delete("s1")
            await asyncio.to_thread(birth.adapter.close_sync, "s1", meta)
        await owner.change("s1", to_terminal)
    asyncio.run(scenario())
    slot = owner.slot("s1")
    assert transport.kinds() == ["open", "snapshot", "close"], "vínculo pendente não abre no Rust"
    assert slot.binding.meta["pending_terminal"] == "pane-novo" and not slot.binding.headless
    assert birth.kills == [pid]


def test_reload_reopens_in_rust(birth):
    owner, transport, pid = _opened(birth)
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        await owner.ensure_open("s1")
        await birth.adapter.recarregar("s1")
    asyncio.run(scenario())
    assert transport.kinds() == ["open", "snapshot", "close", "open"]
    assert birth.kills == [pid] and len(birth.launches) == 1, "mata pelo sidecar e relança uma vez"
    assert transport.ops[3][1]["meta"]["cano"]["pid"] == 999_999_999 and transport.ops[3][1]["generation"] == 2
    slot = owner.slot("s1")
    assert slot.phase == runtime_coordinator.Phase.Rust and slot.lease is None


def test_account_switch_reopens_with_engine_models_and_waits_initialize(birth, monkeypatch):
    seen = []
    original = birth.adapter.launch_process
    async def launch(name, *, engine_models=None, launch=True):
        seen.append(engine_models)
        return await original(name, engine_models=engine_models, launch=launch)
    monkeypatch.setattr(birth.adapter, "launch_process", launch)
    owner, transport, pid = _opened(birth)
    models = [{"id":"modelo-da-conta"}]
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        await owner.ensure_open("s1")
        transport.view = {"iniciando":False, "public_state":{"session":"s1", "state":"idle", "headless":True,
            "problema":"headless_nao_subiu", "problema_detalhe":"initialize recusado: conta sem acesso"}}
        async def move():
            await birth.adapter.parar("s1")
            await asyncio.to_thread(birth.sessions.update, "s1", config_dir="/outra-conta")
            await birth.adapter.ensure_running("s1", require_initialize=True, engine_models=models)
        await owner.change("s1", move)
    with pytest.raises(RuntimeError, match="conta sem acesso"):
        asyncio.run(scenario())
    assert seen[-1] == models and transport.kinds() == ["open", "snapshot", "close", "open"]
    assert birth.kills == [pid] and len(birth.launches) == 1
    assert owner.slot("s1").phase == runtime_coordinator.Phase.Rust, "a recusa fica visível no Rust"


def test_switch_to_headless_opens_in_rust(birth, monkeypatch):
    # Regressão: terminal no Rust → sidecar sem terminal → abre sem terminal no Rust (a adoção de cano não existe mais).
    from app import runtime_terminal
    from app.runtime_coordinator import Binding
    owner = birth.build(LockingTransport())
    transport = owner.transport
    from app import pqueue
    directory, key = pqueue._queue_dir(), birth.sessions.load("s1")["key"]
    terminal = Binding("t1", key, "claude", False,
        {"key":key, "session_id":SID, "fingerprint":"vida",
         "terminal":{"name":"t1", "pane":"%1", "conversation":SID, "generation":1, "created":1,
                     "mux_argv":["tmux"], "windows":False, "clipboard_lock_path":None}},
        "", directory, directory / "runtime" / f"{key}.json", directory / "runtime" / f"{key}.lock", 1)
    monkeypatch.setattr(runtime_terminal, "_session_proof", lambda name: None)
    meta = birth.sessions.load("s1")
    birth.sessions.delete("s1")
    pane = {"binding":terminal}
    monkeypatch.setattr(runtime_terminal, "resolve_binding", lambda name, previous=None: pane["binding"])
    monkeypatch.setattr(runtime_terminal, "validate_binding", lambda descriptor: pane["binding"])
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        assert await owner.prepare_session("t1", "claude")
        assert owner.slot("t1").phase == runtime_coordinator.Phase.Rust
        async def to_headless():
            pane["binding"] = None
            birth.sessions.save("t1", meta["cwd"], SID, model="haiku", permission_mode="manual")
            birth.sessions.update("t1", key=terminal.key)
        await owner.change("t1", to_headless)
        await birth.adapter.ensure_running("t1", esperar_pronta=False)
    asyncio.run(scenario())
    assert transport.kinds() == ["open", "snapshot", "close", "open"]
    assert transport.ops[3][1]["headless"] is True and len(birth.launches) == 1
    assert owner.slot("t1").phase == runtime_coordinator.Phase.Rust


def test_transfer_source_idle_reads_runtime_view(birth):
    from app.conversation_transfer import TransferError, _check_source_idle
    owner, transport, _ = _opened(birth, in_progress=True)
    meta = {"headless":True, "jsonl":str(birth.adapter.transcript_path_de(birth.sessions.load("s1")))}
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        await owner.ensure_open("s1")
        await owner.refresh_snapshot("s1")
        async def busy():
            with pytest.raises(TransferError, match="session_transfer_source_busy"):
                await _check_source_idle(api.registry, "s1", meta)
        await owner.change("s1", busy)
        transport.view = {}
        await owner.refresh_snapshot("s1")
        async def idle():
            await _check_source_idle(api.registry, "s1", meta)
        await owner.change("s1", idle)
    asyncio.run(scenario())
    assert transport.kinds().count("close") == 2


def test_transfer_stops_source_without_python_client(birth):
    owner, transport, pid = _opened(birth)
    record = SimpleNamespace(name="s1", id="transfer-1", origin_meta={"headless":True})
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        await owner.ensure_open("s1")
        async def stop():
            await api.registry.stop_transfer_source(record, {"processes":{}, "original":birth.sessions.load("s1")})
        await owner.change("s1", stop)
    asyncio.run(scenario())
    assert birth.kills == [pid]
    assert transport.kinds() == ["open", "snapshot", "close"], "parada não reabre"
    assert birth.sessions.load("s1")["transfer_id"] == "transfer-1"


def test_shutdown_leaves_canos_alive_and_touches_no_session(birth, monkeypatch):
    from app.runtime_adapter import LegacyBridge
    owner, transport, _ = _opened(birth)
    monkeypatch.setattr(LegacyBridge, "quiesce", lambda *args: pytest.fail("quiesce na parada"))
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        await owner.ensure_open("s1")
        await owner.shutdown()
    asyncio.run(scenario())
    assert transport.kinds() == ["open"] and birth.kills == []
    transport.lease.close()


# --- Modo do processo, restart e a guarda do cliente legado (dono único, Task 5) ---

def _queue_with_pending(birth, delivered=False):
    from app import pqueue, runtime_queue
    from app.runtime_coordinator import WriterLease
    key = birth.sessions.load("s1")["key"]
    directory = pqueue._queue_dir()
    lease = WriterLease(directory / "runtime" / f"{key}.lock")
    try:
        runtime_queue.QueueStore(directory / "runtime" / f"{key}.json", directory, runtime_queue.initial_state(
            key, 1, "s1", [{"id":"e1", "text":"oi", "delivered":delivered, "ts":1.0}]))
    finally:
        lease.close()


def _pending_owner(birth, transport=None):
    owner = birth.build(None)
    owner.mode = "pending"
    return owner


async def _enter(owner, transport):
    owner.loop = asyncio.get_running_loop()
    owner.configure_transport(transport)
    await owner.adoption_task
    return owner


def test_lifespan_with_rust_expected_opens_no_cano_client(birth, monkeypatch):
    _alive_cano(birth)
    owner = _pending_owner(birth)
    calls = []
    from app.adapters.claude_headless import adapter as A
    monkeypatch.setattr(A, "matar_orfaos", lambda: 0)     # a varredura real olha os processos da máquina
    monkeypatch.setattr(birth.adapter, "reconectar_todas", lambda: calls.append("reconectar"))
    monkeypatch.setattr(api, "_recover_pending_transfers", lambda: calls.append("transferencias"))
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        await owner.start_sessions({"claude":birth.adapter, "codex":SimpleNamespace(_sessions={})})
        await api._boot_sessions(owner)
    asyncio.run(scenario())
    assert not owner.managed_queue("s1"), "nada da sessão Claude sem terminal registrado antes do Rust"
    assert calls == [] and set(owner.mode_hooks) == {"rust", "python"}


def test_rust_up_opens_live_canos_in_rust(birth):
    _alive_cano(birth)
    owner = _pending_owner(birth)
    transport = LockingTransport()
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        await owner.start_sessions({"claude":birth.adapter, "codex":SimpleNamespace(_sessions={})})
        await _enter(owner, transport)
    asyncio.run(scenario())
    assert transport.kinds() == ["open"] and owner.mode == "rust"
    slot = owner.slot("s1")
    assert slot.phase == runtime_coordinator.Phase.Rust and slot.lease is None
    assert birth.launches == []
    transport.lease.close()


def test_dead_cano_with_pending_queue_is_relaunched_after_rust_up(birth):
    _queue_with_pending(birth)
    owner = _pending_owner(birth)
    transport = LockingTransport()
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        await owner.start_sessions({"claude":birth.adapter, "codex":SimpleNamespace(_sessions={})})
        await _enter(owner, transport)
    asyncio.run(scenario())
    assert transport.kinds() == ["open"] and len(birth.launches) == 1, "relança uma vez para entregar"
    transport.lease.close()


def test_dead_cano_without_pending_queue_stays_stopped_after_rust_up(birth):
    _queue_with_pending(birth, delivered=True)
    owner = _pending_owner(birth)
    transport = LockingTransport()
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        await owner.start_sessions({"claude":birth.adapter, "codex":SimpleNamespace(_sessions={})})
        await _enter(owner, transport)
    asyncio.run(scenario())
    assert transport.kinds() == [] and birth.launches == []


def test_crash_before_limit_keeps_sessions_out_of_python(birth, monkeypatch):
    from app.runtime_adapter import LegacyBridge
    owner, first, _ = _opened(birth)
    monkeypatch.setattr(LegacyBridge, "reconnect", lambda *args: pytest.fail("Python religou a sessão"))
    second = LockingTransport()
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        await owner.ensure_open("s1")
        first.lease.close()             # o Rust caiu: o sistema soltou a trava dele
        await owner.enter_pending()
        slot = owner.slot("s1")
        assert owner.mode == "pending" and slot.phase == runtime_coordinator.Phase.Rust and slot.lease is None
        await _enter(owner, second)
    asyncio.run(scenario())
    assert second.kinds() == ["open"] and owner.mode == "rust"
    assert owner.slot("s1").phase == runtime_coordinator.Phase.Rust and owner.slot("s1").cache_valid
    second.lease.close()


def test_send_during_crash_waits_and_repeats_same_operation(birth):
    owner, first, _ = _opened(birth)
    second = LockingTransport()
    first.alive = False
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        await owner.ensure_open("s1")
        owner.mode = "rust"
        original = first.op
        async def dropped(descriptor, command, operation_id, clock):
            if command["kind"] == "submit":
                first.lease.close()
                raise ConnectionResetError("Rust caiu no meio do envio")
            return await original(descriptor, command, operation_id, clock)
        first.op = dropped
        async def supervisor():
            await asyncio.sleep(0.1)
            await owner.enter_pending()
            await asyncio.sleep(0.1)
            await _enter(owner, second)
        task = asyncio.create_task(supervisor())
        result = await owner.op("s1", {"kind":"submit", "text":"Olá"}, "op-crash")
        await task
        return result
    result = asyncio.run(scenario())
    assert result["disposition"] == "deferred"
    assert [(kind, d) for kind, d in second.ops if kind == "submit"] and second.kinds() == ["open", "submit"]
    second.lease.close()


def test_pending_wait_has_ceiling(birth, monkeypatch):
    monkeypatch.setattr(runtime_coordinator, "PENDING_WAIT_S", 0.2)
    owner = _pending_owner(birth)
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        await owner.prepare_session("s1", "claude", launch=True)
    with pytest.raises(RuntimeError) as caught:
        asyncio.run(scenario())
    assert getattr(caught.value, "code", "") == "runtime_starting"


def test_third_crash_recovers_each_session_once(birth, monkeypatch):
    from app.runtime_adapter import LegacyBridge
    owner, transport, _ = _opened(birth)
    reconnected, hooks = [], []
    async def reconnect(self, descriptor, carry):
        reconnected.append(descriptor["name"])
        return {"hydrated":True}
    monkeypatch.setattr(LegacyBridge, "reconnect", reconnect)
    async def python_hook():
        hooks.append("python")
    owner.mode_hooks["python"] = python_hook
    transport.alive = False
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        await owner.ensure_open("s1")
        transport.lease.close()
        await owner.enter_pending()
        await owner.enter_python(SimpleNamespace(containment_clean=lambda: True))
    asyncio.run(scenario())
    assert reconnected == ["s1"] and hooks == ["python"] and owner.mode == "python"
    assert owner.slot("s1").phase == runtime_coordinator.Phase.Python


def test_transfer_recovery_waits_for_owner(birth):
    owner = _pending_owner(birth)
    ran = []
    async def recovery():
        ran.append(owner.mode)
    owner.mode_hooks["rust"] = recovery
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        await owner.start_sessions({"claude":birth.adapter, "codex":SimpleNamespace(_sessions={})})
        assert ran == []
        await _enter(owner, LockingTransport())
        await owner.enter_pending()
        await _enter(owner, LockingTransport())
    asyncio.run(scenario())
    assert ran == ["rust"], "uma vez, depois do Rust de pé"


def test_legacy_client_refused_while_rust_owns(birth):
    import inspect
    _alive_cano(birth)
    owner = birth.build(LockingTransport())
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        for mode in ("pending", "rust"):
            owner.mode = mode
            with pytest.raises(RuntimeError, match="cliente Python"):
                await inspect.unwrap(birth.adapter.ensure_running)(birth.adapter, "s1", so_reconectar=True)
    asyncio.run(scenario())


def test_codex_headless_stays_python_while_rust_owns(birth, monkeypatch):
    from app.adapters.codex import sessions as codex_sessions
    from app.runtime_adapter import LegacyBridge, assert_legacy
    meta = {"name":"c1", "key":"codexkey", "headless":True, "thread_id":"thread", "rollout_path":"/tmp/rollout.jsonl"}
    monkeypatch.setattr(codex_sessions, "load", lambda name: meta if name == "c1" else None)
    transport = LockingTransport()
    owner = birth.build(transport)
    owner.legacy = LegacyBridge(owner, {"claude":birth.adapter, "codex":SimpleNamespace(_sessions={})})
    owner.mode = "rust"
    async def scenario():
        owner.loop = asyncio.get_running_loop()
        assert await owner.prepare_session("c1", "codex", launch=True)
    asyncio.run(scenario())
    assert transport.kinds() == [] and owner.slot("c1").phase == runtime_coordinator.Phase.Python
    assert_legacy("c1")         # o cliente Python do Codex segue permitido
