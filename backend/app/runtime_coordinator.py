"""Uma posse por chave durante cliente do cano, fila e recuperação."""
from __future__ import annotations

import asyncio
import contextvars
import copy
import errno
import json
import os
import re
import threading
import time
import uuid
from contextlib import asynccontextmanager, contextmanager
from dataclasses import dataclass, field
from enum import Enum
from pathlib import Path

from app import runtime_queue

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
        path = Path(path)
        path.parent.mkdir(parents=True, exist_ok=True)
        fd = os.open(path, os.O_RDWR | os.O_CREAT, 0o600)
        self.file = os.fdopen(fd, "r+b", buffering=0)
        try:
            if os.name == "nt":
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
    # Geração em que o Rust recusou adotar a sessão: ela fica no Python até a próxima vida.
    rust_refused: int | None = None


def _clock():
    return {"monotonic_s": time.monotonic(), "epoch_s": time.time()}


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

    async def prepare_session(self, name, provider):
        if self.legacy is None:
            return self.managed_runtime(name)
        async with self.registration_locks.setdefault(name, asyncio.Lock()):
            binding = await asyncio.to_thread(self.legacy.binding, name, provider)
            if binding is None:
                return False
            slot = self.slots.get(binding.key)
            if slot is None:
                slot = await asyncio.to_thread(self.register, binding)
            if slot is None or not slot.binding.headless:
                return False
            if slot.frozen or slot.phase not in {Phase.Python, Phase.Rust}:
                raise RuntimeError("sessão em transferência; aguarde a confirmação")
            if slot.phase == Phase.Rust and binding.jsonl != slot.binding.jsonl:
                async def changed():
                    return None
                field = "session_id" if binding.provider == "claude" else "thread_id"
                await self.change(name, changed, advance=binding.meta.get(field) != slot.binding.meta.get(field), reopen=False)
            if slot.phase == Phase.Python:
                with slot.guard:
                    slot.binding.meta = binding.meta
                if (self.transport is not None and (binding.meta.get("cano") or {}).get("versao") == 2
                        and slot.rust_refused != slot.binding.generation):
                    await self.adopt(name)
            return True

    async def start_sessions(self, adapters):
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
                await asyncio.to_thread(self.register, Binding(**values))
        from app.adapters.claude_headless import sessions as claude_sessions
        from app.adapters.codex import sessions as codex_sessions
        for provider, sessions in (("claude", claude_sessions), ("codex", codex_sessions)):
            for meta in await asyncio.to_thread(sessions.list_all):
                if meta.get("headless"):
                    try:
                        await self.prepare_session(meta["name"], provider)
                    except Exception as exc:
                        from app import diag
                        diag.registrar("runtime.registration_failed", "erro", sessao=meta["name"], codigo=type(exc).__name__)

    async def native_receipt(self, message_id, status):
        for slot in tuple(self.slots.values()):
            state = await asyncio.to_thread(lambda: json.loads(slot.binding.state_path.read_bytes()))
            for operation_id, operation in state["operations"].items():
                if operation["payload"].get("kind") not in {"input", "steer"}:
                    continue
                expected = str(uuid.uuid5(uuid.NAMESPACE_URL, "hangar:" + slot.binding.key + ":" + operation_id))
                if message_id != expected:
                    continue
                disposition = "accepted" if status in {"delivered", "released", ""} else "rejected" if status in {"rejected", "refused"} else "unknown"
                await self.op(slot.binding.name, {"kind":"queue", "action":{"kind":"finish", "id":operation_id,
                    "status":disposition, "result":{"operation_id":operation_id, "disposition":disposition,
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
                    if slot.binding.headless:
                        try:
                            await self.prepare_session(slot.binding.name, slot.binding.provider)
                        except Exception as exc:
                            from app import diag
                            diag.registrar("runtime.adoption_failed", "erro", sessao=slot.binding.name, codigo=type(exc).__name__)
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
            except Exception:
                slot.cache_valid = False
                self._signal(slot)
        self.refreshing[key] = asyncio.create_task(refresh())

    def request_drain(self, name):
        async def drain():
            try:
                await self.op(name, {"kind":"drain"}, uuid.uuid4().hex)
            except Exception:
                self.slot(name).cache_valid = False
                self._signal(self.slot(name))
        self.loop.create_task(drain())

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
                    delay = 0.25
                raise RuntimeError("stream privado encerrado sem aviso")
            except asyncio.CancelledError:
                raise
            except Exception:
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
                diag.registrar("runtime.rebind_failed", "erro", sessao=slot.binding.name, codigo=type(exc).__name__)
        self.rebindings[key] = asyncio.create_task(rebind())

    def register(self, binding: Binding):
        global _current
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
        if not binding.headless and not binding.state_path.exists():
            return None
        lease = WriterLease(binding.lock_path)
        slot = Slot(binding=copy.deepcopy(binding), lease=lease)
        self.slots[binding.key], self.names[binding.name] = slot, binding.key
        _current = self
        runtime_queue.configure(self)
        try:
            projection = binding.projection_dir / f"{self._sanitize(binding.name)}.jsonl"
            rows = []
            if not binding.state_path.exists() and projection.exists():
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
                slot.store._persist(state)
            slot.store.exec(binding.generation, "recover:" + uuid.uuid4().hex, _clock(), {"kind": "recover"})
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
        return bool(key and self.slots[key].binding.headless)

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
            with slot.guard:
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
        with slot.guard:
            if state["owner_key"] != descriptor["key"] or state["generation"] != descriptor["generation"]:
                raise ValueError("estado não pertence à vida atual")
            slot.store._persist(copy.deepcopy(state))
            slot.store.ensure_projection()

    async def shutdown(self):
        for slot in tuple(self.slots.values()):
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
        return await self.transport.op(descriptor, command, operation_id, _clock())

    async def op(self, name, command, operation_id):
        self.loop = asyncio.get_running_loop()
        with self.queue_gate(name) as route:
            if route is None:
                raise RuntimeError("sessão sem responsável gerenciado")
            slot, phase, descriptor = route
            if phase == Phase.Rust:
                if command["kind"] not in {"snapshot", "ensure_projection"} and not slot.cache_valid:
                    raise RuntimeError("estado do runtime indisponível; aguarde a reposição")
                return await self._rpc(descriptor, command, operation_id)
            if self.legacy is None:
                raise RuntimeError("serviço da reserva indisponível")
            return await self.legacy.op(descriptor, command, operation_id)

    async def adopt(self, name):
        slot = self.slot(name)
        async with self._barrier(slot):
            if slot.phase == Phase.Rust:
                return True
            if slot.phase != Phase.Python or not slot.binding.headless:
                raise RuntimeError("sessão não está disponível para transferência")
            descriptor = slot.binding.descriptor()
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
                ready = await self._rpc(descriptor, {"kind": "adopt", "descriptor": descriptor, "carry": slot.carry}, uuid.uuid4().hex)
                if not (ready.get("ready") is True and ready.get("instance") == self.instance
                        and ready.get("key") == descriptor["key"] and ready.get("generation") == descriptor["generation"]):
                    raise RuntimeError("readiness não corresponde à vida atual")
                with slot.guard:
                    slot.view = ready["state"]
                    slot.cache_valid = True
                    slot.phase = Phase.Rust
                self._signal(slot)
                return True
            except BaseException as exc:
                with slot.guard:
                    slot.phase = Phase.RecoveringPython
                if released:
                    try:
                        detached = await self._rpc(descriptor, {"kind": "detach"}, uuid.uuid4().hex)
                    except Exception:
                        detached = {}
                    # Sem confirmação, quem decide é o lock: se o Rust ainda segura a sessão, o
                    # _restore falha ao pegá-lo e o erro sobe; nunca dois donos.
                    if detached.get("detached") is not True:
                        from app import diag
                        diag.registrar("runtime.detach_unconfirmed", "erro", sessao=name)
                await self._restore(slot)
                if not isinstance(exc, Exception):
                    raise
                # Adotar ainda não entregou nada do usuário: a sessão segue no Python, sem erro na tela.
                with slot.guard:
                    slot.rust_refused = slot.binding.generation
                from app import diag
                # Só a recusa do IPC vai no detalhe: ela é código e frase fixa, nunca conversa.
                detalhe = str(exc)[:200] if str(exc).startswith("IPC recusou") else ""
                diag.registrar("runtime.adopt_refused", "erro", sessao=name, codigo=type(exc).__name__,
                               detalhe=detalhe)
                return False

    async def _restore(self, slot):
        if slot.lease is None or slot.lease.closed:
            slot.lease = WriterLease(slot.binding.lock_path)
        slot.store = runtime_queue.QueueStore(slot.binding.state_path, slot.binding.projection_dir,
            runtime_queue.initial_state(slot.binding.key, slot.binding.generation, slot.binding.name, []))
        slot.store.exec(slot.binding.generation, "recover:" + uuid.uuid4().hex, _clock(), {"kind": "recover"})
        recovered_view = slot.store.state.get("runtime_state", {}).get("view") or {}
        slot.carry = {"runtime_state":copy.deepcopy(recovered_view)} if recovered_view else slot.carry
        if self.legacy is None:
            raise RuntimeError("serviço da reserva indisponível")
        ready = await self.legacy.reconnect(slot.binding.descriptor(), slot.carry)
        if ready.get("hydrated") is not True:
            raise RuntimeError("reserva não restaurou o snapshot")
        with slot.guard:
            slot.phase = Phase.Python

    async def detach(self, name):
        slot = self.slot(name)
        async with self._barrier(slot):
            if slot.phase == Phase.Python:
                return
            with slot.guard:
                slot.phase = Phase.RecoveringPython
            await self._wait_active(slot)
            reply = await self._rpc(slot.binding.descriptor(), {"kind": "detach"}, uuid.uuid4().hex)
            if reply.get("detached") is not True:
                raise RuntimeError("Rust não confirmou a liberação da sessão")
            await self._restore(slot)

    async def recover(self, name, confirmed_dead: bool):
        slot = self.slot(name)
        async with self._barrier(slot):
            alive = getattr(self.transport, "alive", False)
            alive = alive() if callable(alive) else alive
            if not confirmed_dead or alive:
                raise RuntimeError("morte do Rust não foi confirmada; a reserva permanece bloqueada")
            with slot.guard:
                slot.phase = Phase.RecoveringPython
            await self._wait_active(slot)
            await self._restore(slot)

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

    async def change(self, name, action, *, new_name=None, advance=True, remove=False, reopen=True):
        if not self.managed_queue(name):
            return await action()
        slot = self.slot(name)
        if self.in_lifecycle(slot):
            return await action()

        async def perform():
            async with self.freeze(name):
                if slot.phase != Phase.Python:
                    await self.detach(name)
                if remove and self.legacy is not None:
                    await self.legacy.quiesce(slot.binding.descriptor())
                result = await action()
                await self._wait_active(slot)
                if remove:
                    if self.legacy is not None:
                        await self.legacy.quiesce(slot.binding.descriptor())
                    with slot.guard:
                        slot.lease.close()
                        slot.lease = None
                        self.names.pop(slot.binding.name, None)
                        self.slots.pop(slot.binding.key, None)
                    return result
                target_name = new_name or name
                binding = await asyncio.to_thread(self.legacy.binding, target_name, slot.binding.provider)
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
                    binding.generation = slot.binding.generation + int(advance)
                    state = copy.deepcopy(slot.store.state)
                    if advance:
                        state["runtime_state"] = {}
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
        if reopen and not remove and slot.binding.headless:
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
