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
            return (slot.phase == Phase.Python and not slot.frozen and slot.binding.generation == generation
                    and slot.lease is not None and not slot.lease.closed)

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
            if slot.frozen or slot.phase not in {Phase.Python, Phase.Rust}:
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
        if _lifecycle.get() is slot:
            yield
        else:
            async with slot.lifecycle:
                token = _lifecycle.set(slot)
                try:
                    yield
                finally:
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
                    slot.phase = Phase.Rust
                return True
            except BaseException:
                with slot.guard:
                    slot.phase = Phase.RecoveringPython
                if released:
                    detached = await self._rpc(descriptor, {"kind": "detach"}, uuid.uuid4().hex)
                    if detached.get("detached") is not True:
                        raise RuntimeError("Rust não confirmou a liberação da sessão")
                await self._restore(slot)
                raise

    async def _restore(self, slot):
        if slot.lease is None or slot.lease.closed:
            slot.lease = WriterLease(slot.binding.lock_path)
        slot.store = runtime_queue.QueueStore(slot.binding.state_path, slot.binding.projection_dir,
            runtime_queue.initial_state(slot.binding.key, slot.binding.generation, slot.binding.name, []))
        slot.store.exec(slot.binding.generation, "recover:" + uuid.uuid4().hex, _clock(), {"kind": "recover"})
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
        for slot in self.slots.values():
            with slot.guard:
                if slot.active:
                    raise RuntimeError("persistência ainda em curso; posse conservada")
                if slot.lease is not None:
                    slot.lease.close()

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
            for line in snapshot["pendentes"]:
                if not isinstance(json.loads(line), dict):
                    raise ValueError("pedido pendente inválido")
            return snapshot
        finally:
            writer.close()
            await writer.wait_closed()
