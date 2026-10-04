"""Uma posse por chave durante cliente do cano, fila e recuperação."""
from __future__ import annotations

import asyncio
import contextvars
import copy
import errno
import json
import logging
import os
import sys
import re
import threading
import time
import uuid
from contextlib import asynccontextmanager, contextmanager
from dataclasses import dataclass, field
from enum import Enum
from pathlib import Path

from app import runtime_queue

_log = logging.getLogger("hangar.runtime")

_current = None
_lifecycle = contextvars.ContextVar("runtime_lifecycle", default=None)


def current():
    return _current


def ensure():
    global _current
    if _current is None:
        _current = RuntimeCoordinator()
    return _current


class Phase(Enum):
    Python = "python"
    PreparingRust = "preparing_rust"
    Rust = "rust"
    RecoveringPython = "recovering_python"


class WriterLease:
    def __init__(self, path):
        path = path if isinstance(path, Path) else Path(path)
        path.parent.mkdir(parents=True, exist_ok=True)
        fd = os.open(path, os.O_RDWR | os.O_CREAT, 0o600)
        self.file = os.fdopen(fd, "r+b", buffering=0)
        try:
            if sys.platform == "win32":
                import ctypes
                import msvcrt

                class Overlapped(ctypes.Structure):
                    _fields_ = [("internal", ctypes.c_size_t), ("internal_high", ctypes.c_size_t),
                                ("offset", ctypes.c_uint32), ("offset_high", ctypes.c_uint32),
                                ("event", ctypes.c_void_p)]

                lock = ctypes.WinDLL("kernel32", use_last_error=True).LockFileEx
                lock.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32,
                                 ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p]
                lock.restype = ctypes.c_int
                overlapped = Overlapped()
                if not lock(msvcrt.get_osfhandle(fd), 3, 0, 0xFFFFFFFF, 0xFFFFFFFF, ctypes.byref(overlapped)):
                    error = ctypes.get_last_error()
                    if error == 33:
                        raise BlockingIOError(errno.EAGAIN, "outro responsável possui a sessão")
                    raise ctypes.WinError(error)
            else:
                import fcntl
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BaseException:
            self.file.close()
            raise

    def close(self):
        self.file.close()

    @property
    def closed(self):
        return self.file.closed


@dataclass
class Binding:
    name: str
    key: str
    provider: str
    headless: bool
    meta: dict
    jsonl: str
    projection_dir: Path
    state_path: Path
    lock_path: Path
    generation: int

    def descriptor(self):
        return {"name": self.name, "key": self.key, "provider": self.provider,
                "headless": self.headless, "meta": copy.deepcopy(self.meta), "jsonl": self.jsonl,
                "projection_dir": str(self.projection_dir), "state_path": str(self.state_path),
                "lock_path": str(self.lock_path), "generation": self.generation}


@dataclass
class Slot:
    binding: Binding
    phase: Phase = Phase.RecoveringPython
    lease: WriterLease | None = None
    store: runtime_queue.QueueStore | None = None
    view: dict = field(default_factory=dict)
    carry: dict = field(default_factory=dict)
    active: int = 0
    frozen: bool = False
    guard: threading.Lock = field(default_factory=threading.Lock)
    lifecycle: asyncio.Lock = field(default_factory=asyncio.Lock)
    changed: asyncio.Event = field(default_factory=asyncio.Event)
    cache_valid: bool = False
    lifecycle_token: object | None = None
    terminal_serial: asyncio.Lock = field(default_factory=asyncio.Lock)
    # Geração em que o Rust falhou com a sessão: ela fica no Python até o backend reiniciar, que é
    # quando chega a versão com a correção.
    rust_refused: int | None = None
    adopt_failures: int = 0
    awaiting_identity: bool = False


def _clock():
    return {"monotonic_s": time.monotonic(), "epoch_s": time.time()}


def failure_reason(exc: BaseException) -> dict:
    """Tipo e motivo de uma falha do runtime para o diário. Só falhas do caminho Rust levam o
    detalhe: lá a mensagem é código e frase fixa. As do Python podem embutir texto da sessão."""
    from_rust = getattr(exc, "_hangar_rust", False) or type(exc).__name__ in {"RustOpError", "RustCacheInvalid"}
    plain = from_rust or getattr(exc, "safe_detail", False) or isinstance(exc, (TimeoutError, ConnectionError))
    return {"codigo": type(exc).__name__, "detalhe": str(exc)[:200] if plain else ""}


# Recusas do Rust que acontecem antes de qualquer efeito (nada escrito no cano nem na fila):
# repetir é seguro. Fora delas, repetir pode digitar a mesma mensagem duas vezes.
_PRE_EFFECT_CODES = frozenset({
    "command_kind", "command_fields", "descriptor_shape", "descriptor_binding", "cano_pid",
    "cano_token", "cano_address", "cano_version", "cano_connect", "cano_auth", "runtime_binding",
    "runtime_provider", "runtime_lease", "runtime_generation", "runtime_stopping", "control_kind",
    "queue_action", "terminal_facts", "receipt_scan"})
_TERMINAL_PRE_EFFECT_ERRORS = frozenset({"terminal_facts", "receipt_scan"})
# Respostas normais do Rust ao pedido (botão velho, sessão ocupada, entrada inválida): não são
# defeito, então não contam nem trocam de dono; só sobem como erro.
_ANSWER_CODES = frozenset({
    "claude_command", "codex_command", "lifecycle_required", "operation_reused", "input_text",
    "queue_busy", "queue_entry", "steer_unknown", "policy_refused"})
_RUST_TRIES = 4          # 3 tentativas, uma pausa e a última; depois a parte vai para o Python
_RETRY_PAUSE_S = 2.0     # cobre a volta do canal de eventos, que recompõe o estado sozinho


class RustCacheInvalid(RuntimeError):
    """O Python perdeu a cópia do estado da sessão no Rust; nada foi enviado."""


def _safe_to_repeat(exc: BaseException) -> bool:
    if isinstance(exc, RustCacheInvalid):
        return True
    status, code = getattr(exc, "status", None), getattr(exc, "code", "")
    return status in (400, 409, 413) or (status == 503 and code in _PRE_EFFECT_CODES)


class RuntimeCoordinator:
    def __init__(self, transport=None, legacy=None, peek=None):
        self.transport, self.legacy = transport, legacy
        self.peek = peek or self._peek
        self.instance = getattr(transport, "instance", None)
        self.slots: dict[str, Slot] = {}
        self.names: dict[str, str] = {}
        self.loop = None
        self.events_task = None
        self.refreshing = {}
        self.voice_clients = {}
        self.legacy_active = set()
        self.registration_locks = {}
        self.adoption_task = None
        self.rebindings = {}
        self.drains = {}

    async def prepare_session(self, name, provider):
        if self.legacy is None:
            return self.managed_runtime(name)
        async with self.registration_locks.setdefault(name, asyncio.Lock()):
            binding = await asyncio.to_thread(self.legacy.binding, name, provider)
            if binding is None:
                from app.runtime_terminal import outside_scope
                if self.managed_runtime(name) or provider == "claude" and not await asyncio.to_thread(outside_scope, name):
                    raise RuntimeError("vínculo gerenciado indisponível; escrita suspensa")
                return False
            if self.managed_queue(name) and self.slot(name).binding.key != binding.key:
                previous = self.slot(name)
                if previous.awaiting_identity:
                    # Registro em espera nunca teve posse, fila nem dono no Rust: não há o que soltar.
                    self.names.pop(name, None)
                else:
                    async with self.freeze(name):
                        if previous.phase != Phase.Python:
                            await self.detach(name, restore=False)
                        previous.lease.close()
                        previous.lease = None
                        previous.phase = Phase.RecoveringPython
                        self.names.pop(name, None)
            slot = self.slots.get(binding.key)
            if slot is None:
                slot = await asyncio.to_thread(self.register, binding)
            if slot is None or not self.managed_runtime(name):
                return False
            if slot.awaiting_identity:
                async with self.freeze(name):
                    from app.runtime_process import reconcile_startup
                    from app.runtime_terminal import validate_binding
                    await asyncio.to_thread(reconcile_startup, allow_current=True)
                    await asyncio.to_thread(validate_binding, binding.descriptor())
                    if slot.store is not None or slot.lease is not None or slot.phase != Phase.RecoveringPython:
                        raise RuntimeError("registro aguardando identidade já possui responsável")
                    await self._restore(slot, reconnect=False)
                    slot.awaiting_identity = False
            if slot.frozen or slot.phase not in {Phase.Python, Phase.Rust}:
                raise RuntimeError("sessão em transferência; aguarde a confirmação")
            agent_changed = binding.meta.get("terminal") and any(binding.meta.get(field) != slot.binding.meta.get(field)
                for field in ("agent_pid", "agent_birth"))
            if binding.jsonl != slot.binding.jsonl or agent_changed:
                async def changed():
                    return None
                field = "session_id" if binding.provider == "claude" else "thread_id"
                await self.change(name, changed, advance=bool(agent_changed) or binding.meta.get(field) != slot.binding.meta.get(field), reopen=False)
                binding = slot.binding
            if slot.phase == Phase.Python:
                with slot.guard:
                    slot.binding.meta = binding.meta
                    state = copy.deepcopy(slot.store.state)
                    state["runtime_state"]["_binding"] = slot.binding.descriptor()
                    slot.store._persist(state)
                if self.transport is not None and ((binding.meta.get("cano") or {}).get("versao") == 2
                        or binding.meta.get("terminal")) and slot.rust_refused is None:
                    await self.adopt(name)
            return True

    async def start_sessions(self, adapters):
        from app.runtime_process import reconcile_startup
        await asyncio.to_thread(reconcile_startup)
        from app.runtime_adapter import LegacyBridge
        self.loop = asyncio.get_running_loop()
        self.legacy = LegacyBridge(self, adapters)
        from app.pqueue import _queue_dir
        for path in (_queue_dir() / "runtime").glob("*.json"):
            state = await asyncio.to_thread(lambda: json.loads(path.read_bytes()))
            descriptor = state.get("runtime_state", {}).get("_binding")
            if descriptor and not descriptor.get("headless"):
                values = {**descriptor, "generation":state["generation"]}
                for field in ("projection_dir", "state_path", "lock_path"):
                    values[field] = Path(values[field])
                binding = Binding(**values)
                if binding.meta.get("terminal"):
                    from app.runtime_terminal import resolve_binding
                    fresh = await asyncio.to_thread(resolve_binding, binding.name, binding)
                    if fresh is None or fresh.key != binding.key:
                        self.slots[binding.key] = Slot(binding=binding)
                        if fresh is None:
                            self.slots[binding.key].awaiting_identity = True
                            self.names.setdefault(binding.name, binding.key)
                            from app import diag
                            diag.registrar("runtime.registration_failed", "erro", sessao=binding.name, codigo="terminal_binding")
                            continue
                    else:
                        fresh.generation += int(fresh.jsonl != binding.jsonl)
                        fresh.meta["terminal"]["generation"] = fresh.generation
                    binding = fresh
                await asyncio.to_thread(self.register, binding)
        from app.adapters.claude_headless import sessions as claude_sessions
        from app.adapters.codex import sessions as codex_sessions
        for provider, sessions in (("claude", claude_sessions), ("codex", codex_sessions)):
            for meta in await asyncio.to_thread(sessions.list_all):
                if meta.get("headless"):
                    try:
                        await self.prepare_session(meta["name"], provider)
                    except Exception as exc:
                        from app import diag
                        diag.registrar("runtime.registration_failed", "erro", sessao=meta["name"], **failure_reason(exc))

    async def native_receipt(self, message_id, status):
        for slot in tuple(self.slots.values()):
            if self.names.get(slot.binding.name) != slot.binding.key:
                continue
            state = await asyncio.to_thread(lambda: json.loads(slot.binding.state_path.read_bytes()))
            for operation_id, operation in state["operations"].items():
                if operation["payload"].get("kind") not in {"input", "steer"}:
                    continue
                root_id = operation.get("entry_id") or operation_id
                if slot.binding.meta.get("terminal"):
                    result = operation.get("result") or {}
                    delivery = result.get("payload") or {}
                    if (delivery.get("native") is not True or delivery.get("message_id") != message_id
                            or operation["payload"].get("payload", {}).get("_terminal_generation") != slot.binding.generation):
                        continue
                expected = str(uuid.uuid5(uuid.NAMESPACE_URL, "hangar:" + slot.binding.key + ":" + root_id))
                if message_id != expected:
                    continue
                disposition = "accepted" if status in {"delivered", "released", ""} else "rejected" if status in {"rejected", "refused"} else "unknown"
                await self.op(slot.binding.name, {"kind":"queue", "action":{"kind":"finish", "id":root_id,
                    "status":disposition, "result":{"operation_id":root_id, "disposition":disposition,
                        "payload":{"native_status":status}}}}, "native-receipt:" + message_id + ":" + status)
                if disposition == "accepted":
                    await self.op(slot.binding.name, {"kind":"confirm"}, uuid.uuid4().hex)
                return True
        return False

    def configure_transport(self, transport):
        if self.events_task is not None and not self.events_task.done():
            raise RuntimeError("leitor privado anterior ainda ativo")
        self.transport, self.instance = transport, transport.instance
        self.loop = asyncio.get_running_loop()
        self.events_task = self.loop.create_task(self._events(transport, transport.instance))
        if self.legacy is not None:
            async def adopt_registered():
                for slot in tuple(self.slots.values()):
                    if self.names.get(slot.binding.name) == slot.binding.key and self.managed_runtime(slot.binding.name):
                        try:
                            await self.prepare_session(slot.binding.name, slot.binding.provider)
                        except Exception as exc:
                            from app import diag
                            diag.registrar("runtime.adoption_failed", "erro", sessao=slot.binding.name, **failure_reason(exc))
            self.adoption_task = self.loop.create_task(adopt_registered())

    async def close_events(self):
        tasks = [task for task in [self.events_task, self.adoption_task, *self.refreshing.values(), *self.rebindings.values()] if task is not None]
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
        self.events_task = None
        self.refreshing.clear()
        for client in tuple(self.voice_clients.values()):
            client.fail(RuntimeError("runtime encerrado; chamada de voz invalidada"))

    async def _push_channels(self, slot):
        from app.adapters.preview_push import PushPreviewSource, fonte_ferramenta, fonte_pensamento
        for channel, data in (slot.view.get("channels") or {}).items():
            source = {"preview":PushPreviewSource.get, "thinking":fonte_pensamento, "tool":fonte_ferramenta}.get(channel)
            if source is not None:
                await source(slot.binding.name).push(data["text"])

    async def refresh_snapshot(self, name):
        from app.runtime_adapter import apply_event
        slot = self.slot(name)
        descriptor, instance = slot.binding.descriptor(), self.instance
        data = await self._rpc(descriptor, {"kind":"snapshot"}, uuid.uuid4().hex)
        if (self.instance != instance or slot.binding.generation != descriptor["generation"]
                or slot.phase not in {Phase.Rust, Phase.PreparingRust}):
            return False
        event = {"key":descriptor["key"], "generation":descriptor["generation"], "revision":data.get("revision"), "channel":"snapshot", "data":data}
        valid = apply_event(slot, event)
        if valid:
            await self._push_channels(slot)
        self._signal(slot)
        return valid

    def _refresh(self, slot):
        key = slot.binding.key
        if key in self.refreshing and not self.refreshing[key].done():
            return
        async def refresh():
            try:
                await self.refresh_snapshot(slot.binding.name)
            except Exception as exc:
                slot.cache_valid = False
                self._signal(slot)
                from app import diag
                diag.registrar("runtime.refresh_failed", "erro", sessao=slot.binding.name, **failure_reason(exc))
        self.refreshing[key] = asyncio.create_task(refresh())

    def request_drain(self, name, kind="drain"):
        slot = self.slots.get(self.names.get(name, ""))
        if slot is None or slot.binding.key in self.drains and not self.drains[slot.binding.key].done():
            return
        async def drain():
            try:
                await self.op(name, {"kind":kind}, uuid.uuid4().hex)
            except Exception as exc:
                slot.cache_valid = False
                self._signal(slot)
                from app import diag
                diag.registrar("runtime.drain_failed", "erro", sessao=name, **failure_reason(exc))
        self.drains[slot.binding.key] = self.loop.create_task(drain())

    async def _events(self, transport, instance):
        from app.runtime_adapter import apply_event
        delay = 0.25
        while self.transport is transport and self.instance == instance:
            try:
                async for event in transport.events():
                    if self.transport is not transport or self.instance != instance:
                        return
                    if not isinstance(event, dict) or not isinstance(event.get("key"), str):
                        raise ValueError("evento privado inválido")
                    slot = self.slots.get(event["key"])
                    if slot is None or slot.phase not in {Phase.Rust, Phase.PreparingRust}:
                        continue
                    if type(event.get("generation")) is int and event["generation"] != slot.binding.generation:
                        continue
                    previous = slot.view.get("revision", -1)
                    if not apply_event(slot, event):
                        slot.cache_valid = False
                        self._refresh(slot)
                    elif event.get("revision", -1) > previous or event.get("channel") == "snapshot":
                        if event["channel"] in {"preview", "thinking", "tool", "snapshot"}:
                            await self._push_channels(slot)
                        if event["channel"] in {"voice", "voice_target"}:
                            client = self.voice_clients.get((event["key"], event["data"].get("call_id")))
                            if client is not None:
                                client.receive(event["channel"], event["data"]["event"])
                    self._signal(slot)
                    if event.get("channel") in {"view", "snapshot"}:
                        conversation = (slot.view.get("view") or {}).get("conversation")
                        field = "session_id" if slot.binding.provider == "claude" else "thread_id"
                        if conversation and conversation != slot.binding.meta.get(field):
                            self._rebind(slot)
                    if slot.binding.meta.get("terminal") and slot.view.get("error"):
                        self.request_drain(slot.binding.name, "confirm" if slot.view["error"] == "receipt_scan" else "drain")
                    delay = 0.25
                raise RuntimeError("stream privado encerrado sem aviso")
            except asyncio.CancelledError:
                raise
            except Exception as exc:
                from app import diag
                diag.registrar("runtime.events_interrupted", "aviso", ms=int(delay * 1000), **failure_reason(exc))
                for slot in tuple(self.slots.values()):
                    if slot.phase == Phase.Rust:
                        slot.cache_valid = False
                        self._signal(slot)
                for client in tuple(self.voice_clients.values()):
                    client.fail(RuntimeError("stream privado interrompido; voz invalidada"))
            await asyncio.sleep(delay)
            delay = min(5.0, delay * 2)

    def _rebind(self, slot):
        key = slot.binding.key
        if key in self.rebindings and not self.rebindings[key].done():
            return
        slot.frozen = True
        async def rebind():
            async def changed():
                return None
            try:
                await self.change(slot.binding.name, changed)
            except Exception as exc:
                slot.cache_valid = False
                self._signal(slot)
                from app import diag
                diag.registrar("runtime.rebind_failed", "erro", sessao=slot.binding.name, **failure_reason(exc))
        self.rebindings[key] = asyncio.create_task(rebind())

    def register(self, binding: Binding):
        global _current
        from app.runtime_process import reconcile_startup
        reconcile_startup(allow_current=True)
        if binding.provider not in {"claude", "codex"} or not re.fullmatch(r"[A-Za-z0-9_-]{1,128}", binding.key):
            raise ValueError("binding inválido para o runtime")
        if old := self.slots.get(binding.key):
            with old.guard:
                if old.binding.lock_path != binding.lock_path or old.binding.state_path != binding.state_path:
                    raise ValueError("a chave não pode trocar os arquivos de posse")
                identity_changed = (old.binding.name != binding.name or old.binding.headless != binding.headless
                    or old.binding.provider != binding.provider or old.binding.jsonl != binding.jsonl)
                if identity_changed and (not old.frozen or old.active or old.phase != Phase.Python):
                    raise RuntimeError("mudança de binding exige parada e posse Python")
                if old.binding.generation != binding.generation:
                    if not old.frozen or old.active or old.phase != Phase.Python:
                        raise RuntimeError("mudança de vida exige a barreira de lifecycle")
                    if binding.generation <= old.binding.generation:
                        raise ValueError("a geração não pode voltar para uma vida antiga")
                    state = copy.deepcopy(old.store.state)
                    state["generation"] = binding.generation
                    old.store._persist(state)
                    old.store.ensure_projection()
                self.names.pop(old.binding.name, None)
                old.binding = copy.deepcopy(binding)
                self.names[binding.name] = binding.key
            return old
        terminal = binding.provider == "claude" and isinstance(binding.meta.get("terminal"), dict)
        if not binding.headless and not terminal and not binding.state_path.exists():
            return None
        lease = WriterLease(binding.lock_path)
        slot = Slot(binding=copy.deepcopy(binding), lease=lease)
        self.slots[binding.key], self.names[binding.name] = slot, binding.key
        _current = self
        runtime_queue.configure(self)
        try:
            projection = binding.projection_dir / f"{self._sanitize(binding.name)}.jsonl"
            rows = []
            if terminal:
                from app.runtime_terminal import validate_binding, import_legacy
                if self.legacy is not None:
                    validate_binding(binding.descriptor())
                if not binding.state_path.exists() and projection.exists():
                    rows = import_legacy(self, binding, projection)
            if not binding.state_path.exists() and projection.exists() and not terminal:
                for raw in projection.read_text(encoding="utf-8").splitlines():
                    row = json.loads(raw)
                    if not isinstance(row, dict):
                        raise ValueError("entrada Legacy da fila inválida")
                    rows.append(row)
            slot.store = runtime_queue.QueueStore(binding.state_path, binding.projection_dir,
                runtime_queue.initial_state(binding.key, binding.generation, binding.name, rows))
            if slot.store.state["generation"] != binding.generation:
                if binding.generation < slot.store.state["generation"]:
                    raise ValueError("a geração não pode voltar para uma vida antiga")
                state = copy.deepcopy(slot.store.state)
                state["generation"] = binding.generation
                if terminal:
                    state["runtime_state"] = {key:value for key,value in state["runtime_state"].items() if key == "terminal_write_barrier"}
                slot.store._persist(state)
            slot.store.exec(binding.generation, "recover:" + uuid.uuid4().hex, _clock(), {"kind": "recover"})
            state = copy.deepcopy(slot.store.state)
            state["runtime_state"]["_binding"] = binding.descriptor()
            slot.store._persist(state)
            slot.phase = Phase.Python
            return slot
        except BaseException:
            # O slot bloqueado permanece visível; não se importa JSONL por cima de estado inválido.
            lease.close()
            slot.lease = None
            raise

    @staticmethod
    def _sanitize(name):
        return re.sub(r"[^A-Za-z0-9_.-]", "-", name)

    def slot(self, name):
        key = self.names.get(name)
        if key is None:
            raise ValueError("sessão sem registro de runtime")
        return self.slots[key]

    def managed_runtime(self, name):
        key = self.names.get(name)
        return bool(key and (self.slots[key].binding.headless or
            self.slots[key].binding.provider == "claude" and (isinstance(self.slots[key].binding.meta.get("terminal"), dict)
                or self.slots[key].binding.meta.get("pending_terminal"))))

    def managed_queue(self, name):
        return name in self.names

    def legacy_allowed(self, key, generation):
        slot = self.slots.get(key)
        if slot is None:
            return True
        with slot.guard:
            return (slot.phase == Phase.Python and (not slot.frozen or self.in_lifecycle(slot)) and slot.binding.generation == generation
                    and slot.lease is not None and not slot.lease.closed)

    @staticmethod
    def in_lifecycle(slot):
        return slot.lifecycle_token is not None and _lifecycle.get() is slot.lifecycle_token

    def _signal(self, slot):
        if self.loop is not None and self.loop.is_running():
            self.loop.call_soon_threadsafe(slot.changed.set)

    @contextmanager
    def queue_gate(self, name):
        if not self.managed_queue(name):
            yield None
            return
        slot = self.slot(name)
        with slot.guard:
            if (slot.frozen and not self.in_lifecycle(slot)) or slot.phase not in {Phase.Python, Phase.Rust}:
                raise RuntimeError("sessão em transferência; aguarde a posse ser confirmada")
            if slot.phase == Phase.Python and (slot.lease is None or slot.lease.closed):
                raise RuntimeError("reserva sem posse da sessão")
            slot.active += 1
            route = (slot, slot.phase, slot.binding.descriptor())
        try:
            yield route
        finally:
            with slot.guard:
                slot.active -= 1
            self._signal(slot)

    def queue_rpc(self, route, call_id, clock, action):
        slot, phase, descriptor = route
        if phase == Phase.Python:
            if descriptor["meta"].get("terminal") and not self.in_lifecycle(slot):
                from app.runtime_terminal import _writer
                if _writer.get() is None:
                    from app.runtime_adapter import run_sync
                    return run_sync(lambda:self.op(descriptor["name"], {"kind":"queue", "action":action}, call_id), self.loop)
            # A rota já conta em `slot.active` (queue_gate): a posse não muda até ela sair, e a
            # gravação é serializada dentro do QueueStore, fora da trava que o laço de eventos usa.
            return slot.store.exec(descriptor["generation"], call_id, _clock(), action)
        if self.loop is None or not self.loop.is_running():
            raise RuntimeError("loop do runtime indisponível")
        try:
            running = asyncio.get_running_loop()
        except RuntimeError:
            running = None
        if running is self.loop:
            raise RuntimeError("fila síncrona chamada no loop do servidor")
        future = asyncio.run_coroutine_threadsafe(self._rpc(descriptor,
            {"kind": "queue", "action": action}, call_id), self.loop)
        try:
            return future.result(timeout=35)
        except BaseException:
            future.cancel()
            raise

    def commit_python_state(self, route, state):
        slot, phase, descriptor = route
        if phase != Phase.Python:
            raise RuntimeError("a fila pertence ao Rust")
        if state["owner_key"] != descriptor["key"] or state["generation"] != descriptor["generation"]:
            raise ValueError("estado não pertence à vida atual")
        with slot.store._lock:
            slot.store._persist(copy.deepcopy(state))
            slot.store.ensure_projection()

    async def shutdown(self):
        for slot in tuple(self.slots.values()):
            if self.names.get(slot.binding.name) != slot.binding.key:
                continue
            async with self._barrier(slot):
                with slot.guard:
                    slot.frozen = True
                await self._wait_active(slot)
                if slot.phase == Phase.Rust:
                    await self.detach(slot.binding.name)
                if self.legacy is not None:
                    await self.legacy.quiesce(slot.binding.descriptor())
        self.close_python_leases()

    async def _wait_active(self, slot):
        while True:
            with slot.guard:
                if slot.active == 0:
                    return
                slot.changed.clear()
            await slot.changed.wait()

    @asynccontextmanager
    async def _barrier(self, slot):
        self.loop = asyncio.get_running_loop()
        if self.in_lifecycle(slot):
            yield
        else:
            async with slot.lifecycle:
                slot.lifecycle_token = object()
                token = _lifecycle.set(slot.lifecycle_token)
                try:
                    yield
                finally:
                    slot.lifecycle_token = None
                    _lifecycle.reset(token)

    async def _rpc(self, descriptor, command, operation_id):
        if self.transport is None or not self.instance:
            raise RuntimeError("IPC do runtime indisponível")
        result = await self.transport.op(descriptor, command, operation_id, _clock())
        if descriptor["meta"].get("terminal") and command["kind"] == "drain":
            result = {**result, "sent":int((result.get("reply") or {}).get("disposition") == "accepted")}
        return result

    async def _hand_to_python(self, name, reason: str, exc: BaseException | None = None):
        """Passa só esta sessão para o Python até o backend reiniciar; o resto segue no Rust.
        O detach recupera a fila marcando o que estava em voo como incerto: nada é redigitado."""
        slot = self.slots.get(self.names.get(name, ""))
        if slot is None or slot.phase != Phase.Rust:
            return
        from app import diag
        cause = failure_reason(exc) if exc is not None else {}
        try:
            await self.detach(name)
        except Exception as err:
            diag.registrar("runtime.parte_para_python", "erro", sessao=name, etapa="detach falhou:" + reason,
                           **failure_reason(err))
            raise
        with slot.guard:
            slot.rust_refused = slot.binding.generation
        diag.registrar("runtime.parte_para_python", "erro", sessao=name, etapa=reason, **cause)

    async def _settle_rust(self, name):
        """Rust parado em erro não volta sozinho: devolve a sessão ao Python, que segue atendendo."""
        slot = self.slots.get(self.names.get(name, ""))
        if slot is None or slot.phase != Phase.Rust or slot.cache_valid:
            return
        try:
            await self.refresh_snapshot(name)
        except Exception as exc:
            from app import diag
            diag.registrar("runtime.refresh_failed", "erro", sessao=name, **failure_reason(exc))
            return          # sem resposta do Rust não dá para saber; quem decide é a contagem do op
        error = (slot.view or {}).get("error")
        if slot.phase != Phase.Rust or slot.cache_valid or not error:
            return
        if slot.binding.meta.get("terminal") and error in _TERMINAL_PRE_EFFECT_ERRORS:
            return
        await self._hand_to_python(name, "rust_em_erro:" + str(error)[:60])

    async def op(self, name, command, operation_id):
        """Falha do Rust antes de qualquer efeito: 3 tentativas, uma pausa e a última; aí só esta
        sessão vai para o Python e a mesma operação sai por ele. Falha que pode ter tido efeito não
        se repete (duplicaria a mensagem): a sessão vai para o Python e o erro sobe."""
        self.loop = asyncio.get_running_loop()
        if self.legacy is not None and self.managed_queue(name):
            slot = self.slot(name)
            if (slot.binding.meta.get("terminal") or slot.binding.meta.get("pending_terminal")) and command["kind"] != "queue" and not self.in_lifecycle(slot):
                await self.prepare_session(name, "claude")
        if (command["kind"] == "submit" and command["text"].split()[0:1] == ["/clear"]
                and self.managed_queue(name) and self.slot(name).binding.meta.get("terminal")
                and not self.in_lifecycle(self.slot(name))):
            async def clear():
                async with self.freeze(name):
                    result = await self.op(name, command, operation_id)
                    if result.get("disposition") in {"accepted", "unknown"}:
                        await self.op(name, {"kind":"queue", "action":{"kind":"clear"}}, uuid.uuid4().hex)
                    return result
            task = asyncio.create_task(clear())
            try:
                return await asyncio.shield(task)
            except asyncio.CancelledError:
                await task
                raise
        read_only = command.get("kind") in {"snapshot", "ensure_projection"}
        initial = self.slots.get(self.names.get(name, ""))
        terminal_identity = (initial.binding.key, initial.binding.generation) if initial and initial.binding.meta.get("terminal") else None
        failures = 0
        while True:
            current = self.slots.get(self.names.get(name, ""))
            if terminal_identity and (current is None or (current.binding.key, current.binding.generation) != terminal_identity):
                raise RuntimeError("vida terminal mudou durante a tentativa; entrada não repetida")
            if not read_only:
                await self._settle_rust(name)
            current = self.slots.get(self.names.get(name, ""))
            if terminal_identity and (current is None or (current.binding.key, current.binding.generation) != terminal_identity):
                raise RuntimeError("vida terminal mudou durante a tentativa; entrada não repetida")
            try:
                source = self.slots.get(self.names.get(name, ""))
                identity = (source.binding.key, source.binding.generation) if source and source.phase == Phase.Rust and source.binding.meta.get("terminal") else None
                result = await self._op_once(name, command, operation_id)
                outcome = result.get("reply", result) if isinstance(result, dict) else None
                if identity and isinstance(outcome, dict) and outcome.get("disposition") == "unknown":
                    from app import diag
                    from app.rust_server import RustOpError
                    failure = RustOpError("terminal_delivery_unknown: entrega não comprovada; não repetir", 503, "terminal_delivery_unknown")
                    diag.registrar("runtime.rust_delivery_failed", "erro", sessao=name, **failure_reason(failure))
                    current = self.slots.get(self.names.get(name, ""))
                    if current and (current.binding.key, current.binding.generation) == identity:
                        await self._hand_to_python(name, "entrega_incerta", failure)
                return result
            except Exception as exc:
                if not getattr(exc, "_hangar_rust", False) or getattr(exc, "code", "") in _ANSWER_CODES:
                    raise
                failures += 1
                from app import diag
                diag.registrar("runtime.rust_op_failed", "erro", sessao=name, etapa=str(command.get("kind")),
                               quantidade=failures, **failure_reason(exc))
                if not (read_only or _safe_to_repeat(exc)):
                    try:
                        await self._hand_to_python(name, "falha_com_efeito_possivel", exc)
                    except Exception:
                        pass        # o motivo do detach já foi para o diário; o erro original é o que sobe
                    raise
                if failures < _RUST_TRIES:
                    if failures == _RUST_TRIES - 1:
                        await asyncio.sleep(_RETRY_PAUSE_S)
                    continue
                await self._hand_to_python(name, "falhas_seguidas", exc)
                current = self.slots.get(self.names.get(name, ""))
                if terminal_identity and (current is None or (current.binding.key, current.binding.generation) != terminal_identity):
                    raise RuntimeError("vida terminal mudou durante a tentativa; entrada não repetida")
                return await self._op_once(name, command, operation_id)

    async def _op_once(self, name, command, operation_id):
        with self.queue_gate(name) as route:
            if route is None:
                raise RuntimeError("sessão sem responsável gerenciado")
            slot, phase, descriptor = route
            if phase == Phase.Rust:
                try:
                    maintenance = (slot.binding.meta.get("terminal") and command["kind"] in {"confirm", "drain"}
                        and slot.view.get("error") in _TERMINAL_PRE_EFFECT_ERRORS)
                    if command["kind"] not in {"snapshot", "ensure_projection"} and not slot.cache_valid and not maintenance:
                        raise RustCacheInvalid("estado do runtime indisponível; aguarde a reposição")
                    return await self._rpc(descriptor, command, operation_id)
                except Exception as exc:
                    exc._hangar_rust = True     # só falha do caminho Rust entra na contagem
                    raise
            if self.legacy is None:
                raise RuntimeError("serviço da reserva indisponível")
            if descriptor["meta"].get("terminal"):
                from app import runtime_terminal
                async with slot.terminal_serial:
                    if slot.binding.descriptor() != descriptor:
                        raise RuntimeError("binding mudou durante a espera")
                    await asyncio.to_thread(runtime_terminal.validate_binding, descriptor)
                    task = asyncio.create_task(self.legacy.op(descriptor, command, operation_id))
                    try:
                        return await asyncio.shield(task)
                    except asyncio.CancelledError:
                        await task
                        raise
            return await self.legacy.op(descriptor, command, operation_id)

    async def adopt(self, name):
        slot = self.slot(name)
        async with self._barrier(slot):
            if slot.phase == Phase.Rust:
                return True
            if slot.phase != Phase.Python or not self.managed_runtime(name):
                raise RuntimeError("sessão não está disponível para transferência")
            if slot.binding.meta.get("pending_terminal"):
                raise RuntimeError("vínculo terminal ainda não confirmado; transferência suspensa")
            descriptor = slot.binding.descriptor()
            if descriptor["meta"].get("terminal"):
                from app.runtime_terminal import validate_binding
                await asyncio.to_thread(validate_binding, descriptor)
            else:
                await self.peek(descriptor)
            with slot.guard:
                slot.phase = Phase.PreparingRust
            await self._wait_active(slot)
            released = False
            try:
                if self.legacy is None:
                    raise RuntimeError("serviço da reserva indisponível")
                slot.carry = await self.legacy.quiesce(descriptor)
                await self._wait_active(slot)
                json.dumps(slot.carry)
                with slot.guard:
                    slot.store.exec(descriptor["generation"], "quiesce:" + uuid.uuid4().hex, _clock(), {"kind": "recover"})
                    slot.lease.close()
                    slot.lease = None
                    released = True
                try:
                    ready = await self._rpc(descriptor, {"kind": "adopt", "descriptor": descriptor, "carry": slot.carry}, uuid.uuid4().hex)
                    if not (ready.get("ready") is True and ready.get("instance") == self.instance
                            and ready.get("key") == descriptor["key"] and ready.get("generation") == descriptor["generation"]):
                        raise RuntimeError("readiness não corresponde à vida atual")
                except Exception as exc:
                    exc._hangar_rust = True     # só falha do Rust conta para passar a sessão ao Python
                    raise
                with slot.guard:
                    slot.view = ready["state"]
                    slot.cache_valid = True
                    slot.phase = Phase.Rust
                    slot.adopt_failures = 0
                self._signal(slot)
                return True
            except BaseException as exc:
                with slot.guard:
                    slot.phase = Phase.RecoveringPython
                if released:
                    detach_error = None
                    try:
                        detached = await self._rpc(descriptor, {"kind": "detach"}, uuid.uuid4().hex)
                    except Exception as err:
                        detached, detach_error = {}, err
                        err._hangar_rust = True
                    # Sem confirmação, quem decide é o lock: se o Rust ainda segura a sessão, o
                    # _restore falha ao pegá-lo e o erro sobe; nunca dois donos.
                    if detached.get("detached") is not True:
                        from app import diag
                        diag.registrar("runtime.detach_unconfirmed", "erro", sessao=name,
                                       **(failure_reason(detach_error) if detach_error else {}))
                await self._restore(slot)
                if not isinstance(exc, Exception):
                    raise
                # Adotar ainda não entregou nada do usuário: a sessão segue no Python, sem erro na
                # tela, e a próxima ação tenta o Rust de novo até a quarta falha.
                from app import diag
                if not getattr(exc, "_hangar_rust", False):
                    # Falhou do lado do Python (preparar a passagem): não é defeito do Rust e não conta.
                    diag.registrar("runtime.adopt_failed_python", "erro", sessao=name, **failure_reason(exc))
                    return False
                with slot.guard:
                    slot.adopt_failures += 1
                    if slot.adopt_failures >= _RUST_TRIES:
                        slot.rust_refused = slot.binding.generation
                diag.registrar("runtime.adopt_refused", "erro", sessao=name, quantidade=slot.adopt_failures,
                               **failure_reason(exc))
                if slot.rust_refused is not None:
                    diag.registrar("runtime.parte_para_python", "erro", sessao=name, etapa="adocao_recusada",
                                   **failure_reason(exc))
                return False

    async def _restore(self, slot, *, reconnect=True):
        if slot.lease is None or slot.lease.closed:
            slot.lease = WriterLease(slot.binding.lock_path)
        slot.store = runtime_queue.QueueStore(slot.binding.state_path, slot.binding.projection_dir,
            runtime_queue.initial_state(slot.binding.key, slot.binding.generation, slot.binding.name, []))
        slot.store.exec(slot.binding.generation, "recover:" + uuid.uuid4().hex, _clock(), {"kind": "recover"})
        recovered_view = slot.store.state.get("runtime_state", {}).get("view") or {}
        slot.carry = {"runtime_state":copy.deepcopy(recovered_view)} if recovered_view else slot.carry
        if self.legacy is None:
            raise RuntimeError("serviço da reserva indisponível")
        try:
            ready = await self.legacy.reconnect(slot.binding.descriptor(), slot.carry) if reconnect else {"hydrated":True}
            if ready.get("hydrated") is not True:
                raise RuntimeError("reserva não restaurou o snapshot")
        except Exception as exc:
            # Com terminal, quem resolve é o `recover`: aposenta o registro e o próximo
            # prepare_session refaz a sessão do estado durável, com o pane que existir então.
            if slot.binding.meta.get("terminal"):
                raise
            # A trava e a fila já são do Python: sem cano para religar (morreu com a sessão), ela
            # fica estacionada e o próximo envio a sobe de novo, como o adapter antigo. Ficar em
            # "recuperando" recusava toda operação daquela sessão até o backend reiniciar.
            from app import diag
            diag.registrar("runtime.restore_sem_cliente", "erro", sessao=slot.binding.name, **failure_reason(exc))
        with slot.guard:
            slot.phase = Phase.Python

    async def detach(self, name, *, restore=True):
        slot = self.slot(name)
        async with self._barrier(slot):
            if slot.phase == Phase.Python:
                return
            with slot.guard:
                slot.phase = Phase.RecoveringPython
            await self._wait_active(slot)
            try:
                reply = await self._rpc(slot.binding.descriptor(), {"kind": "detach"}, uuid.uuid4().hex)
                if reply.get("detached") is not True:
                    raise RuntimeError("Rust não confirmou a liberação da sessão")
            except BaseException:
                # O Rust não soltou: ele continua dono, e a sessão não fica presa em transferência.
                with slot.guard:
                    slot.phase = Phase.Rust
                raise
            await self._restore(slot, reconnect=restore)

    async def recover(self, name, confirmed_dead: bool, containment=None):
        slot = self.slot(name)
        async with self._barrier(slot):
            alive = getattr(self.transport, "alive", False)
            alive = alive() if callable(alive) else alive
            if not confirmed_dead or alive:
                raise RuntimeError("morte do Rust não foi confirmada; a reserva permanece bloqueada")
            if slot.binding.meta.get("terminal"):
                proof = getattr(containment or self.transport, "containment_clean", None)
                if proof is None or proof() is not True:
                    raise RuntimeError("fim dos descendentes Rust não comprovado; escrita suspensa")
            with slot.guard:
                slot.phase = Phase.RecoveringPython
            await self._wait_active(slot)
            try:
                await self._restore(slot)
            except BaseException:
                # Rust morto e contido: o registro sai, e o próximo prepare_session refaz a sessão a
                # partir do estado durável. Sem isto o nome ficava preso em recuperação até o restart.
                with slot.guard:
                    try:
                        if slot.lease is not None:
                            slot.lease.close()
                    except Exception:
                        _log.warning("trava de escrita não fechou ao aposentar a sessão", exc_info=True)
                    slot.lease = None
                    if self.names.get(name) == slot.binding.key:
                        self.names.pop(name, None)
                    self.slots.pop(slot.binding.key, None)
                raise

    @asynccontextmanager
    async def freeze(self, name):
        slot = self.slot(name)
        async with self._barrier(slot):
            with slot.guard:
                slot.frozen = True
            try:
                await self._wait_active(slot)
                yield slot.binding
            finally:
                with slot.guard:
                    slot.frozen = False

    def close_python_leases(self):
        global _current
        for slot in self.slots.values():
            with slot.guard:
                if slot.active:
                    raise RuntimeError("persistência ainda em curso; posse conservada")
                if slot.lease is not None:
                    slot.lease.close()
        if _current is self:
            _current = None
        if runtime_queue._coordinator is self:
            runtime_queue.configure(None)

    async def retire_waiting(self, name):
        """Nome reaproveitado por uma vida nova: o registro que esperava a identidade antiga sai."""
        async with self.registration_locks.setdefault(name, asyncio.Lock()):
            key = self.names.get(name)
            if key is not None and self.slots[key].awaiting_identity:
                self.names.pop(name, None)

    async def change(self, name, action, *, new_name=None, advance=True, remove=False, reopen=True):
        if not self.managed_queue(name):
            return await action()
        slot = self.slot(name)
        if self.in_lifecycle(slot):
            return await action()
        if remove and slot.awaiting_identity:
            # Registro em espera nunca teve posse nem dono no Rust: fechar só solta o nome.
            await self.retire_waiting(name)
            return await action()

        async def perform():
            async with self.freeze(name):
                if slot.phase != Phase.Python:
                    await self.detach(name)
                if remove and self.legacy is not None:
                    # Fechar não escreve na conversa: basta esperar os escritores, mesmo com vínculo mudado.
                    await self.legacy.quiesce({**slot.binding.descriptor(), "removed":True})
                result = await action()
                await self._wait_active(slot)
                if remove:
                    if self.legacy is not None:
                        # Removida, a sessão não tem mais vínculo a conferir: só se esperam os escritores.
                        await self.legacy.quiesce({**slot.binding.descriptor(), "removed":True})
                    with slot.guard:
                        slot.lease.close()
                        slot.lease = None
                        self.names.pop(slot.binding.name, None)
                        self.slots.pop(slot.binding.key, None)
                    return result
                target_name = new_name or name
                binding = await asyncio.to_thread(self.legacy.binding, target_name, slot.binding.provider)
                if binding is None:
                    from app.runtime_terminal import pending_binding
                    binding = (await asyncio.to_thread(pending_binding, target_name, slot.binding)
                        if slot.binding.provider == "claude" and slot.binding.headless else None)
                    if binding is None:
                        binding = copy.deepcopy(slot.binding)
                        binding.name, binding.headless = target_name, False
                        binding.meta = {**binding.meta, "headless":False, "cano":None}
                if binding.key != slot.binding.key:
                    raise RuntimeError("mudança de modo não pode trocar a chave durável")
                if self.legacy is not None:
                    await self.legacy.quiesce(binding.descriptor())
                with slot.guard:
                    if slot.store.state["name"] != target_name:
                        slot.store.exec(slot.binding.generation, "rename:" + uuid.uuid4().hex, _clock(), {"kind":"rename", "name":target_name})
                    if slot.binding.meta.get("terminal") and binding.jsonl != slot.binding.jsonl:
                        slot.store.exec(slot.binding.generation, "clear:" + uuid.uuid4().hex, _clock(), {"kind":"clear"})
                    binding.generation = slot.binding.generation + int(advance)
                    if isinstance(binding.meta.get("terminal"), dict):
                        binding.meta["terminal"]["generation"] = binding.generation
                    state = copy.deepcopy(slot.store.state)
                    if advance:
                        state["runtime_state"] = {key:value for key,value in state["runtime_state"].items() if key == "terminal_write_barrier"}
                    state["runtime_state"]["_binding"] = binding.descriptor()
                    slot.store._persist(state)
                self.register(binding)
                slot.view, slot.cache_valid = {}, False
                self._signal(slot)
                return result

        task = asyncio.create_task(perform())
        try:
            result = await asyncio.shield(task)
        except asyncio.CancelledError:
            await task
            raise
        if reopen and not remove and self.managed_runtime(slot.binding.name):
            await self.prepare_session(slot.binding.name, slot.binding.provider)
        return result

    async def lifecycle_call(self, name, method, arguments):
        if self.legacy is None:
            raise RuntimeError("serviço administrativo indisponível")
        adapter = self.legacy.adapters[self.slot(name).binding.provider]
        import inspect
        original = inspect.unwrap(getattr(adapter, method))
        params = {key:value for key,value in arguments.items() if key not in {"self", "name", "old"}}
        async def action():
            if inspect.iscoroutinefunction(original):
                return await original(adapter, name, **params)
            return await asyncio.to_thread(original, adapter, name, **params)
        return await self.change(name, action, new_name=params.get("new") if method == "rename" else None,
            advance=method != "rename", remove=False, reopen=method not in {"close_sync", "parar"})

    @staticmethod
    async def _peek(descriptor):
        cano = descriptor["meta"].get("cano") or {}
        if cano.get("versao") != 2 or not cano.get("token"):
            raise RuntimeError("cano v1 permanece na reserva até reabertura natural")
        from app.adapters.claude_headless.cano import MAX_ENVELOPE, MAX_FRAME
        address = cano.get("escuta", "")
        if address.startswith("unix:"):
            opening = asyncio.open_unix_connection(address[5:], limit=MAX_ENVELOPE)
        elif address.startswith("tcp:"):
            host, port = address[4:].rsplit(":", 1)
            import ipaddress
            if not ipaddress.ip_address(host).is_loopback:
                raise ValueError("cano fora do loopback")
            opening = asyncio.open_connection(host, int(port), limit=MAX_ENVELOPE)
        else:
            raise ValueError("endereço do cano inválido")
        reader, writer = await asyncio.wait_for(opening, 10)
        try:
            writer.write(f"peek {cano['token']}\n".encode())
            await writer.drain()
            raw = await asyncio.wait_for(reader.readline(), 10)
            if not raw.endswith(b"\n") or len(raw) > MAX_FRAME + 1:
                raise ValueError("snapshot incompleto ou acima do teto")
            snapshot = json.loads(raw)
            if snapshot.get("type") != "cano_snapshot" or snapshot.get("versao") != 2:
                raise ValueError("snapshot incompatível")
            # O pid do snapshot é o do agente filho, não o do cano gravado no sidecar; quem prova
            # que é o cano certo é o token único por subida.
            if type(snapshot.get("pid")) is not int:
                raise ValueError("snapshot sem pid")
            for line in snapshot["pendentes"]:
                if not isinstance(json.loads(line), dict):
                    raise ValueError("pedido pendente inválido")
            return snapshot
        finally:
            writer.close()
            await writer.wait_closed()
