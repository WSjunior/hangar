"""Observação terminal interna, com vínculo explícito e reserva no chamador."""
from __future__ import annotations

import asyncio
from concurrent.futures import ThreadPoolExecutor
from contextvars import ContextVar
from contextlib import contextmanager
import ipaddress
import json
from http.client import HTTPException
import logging
import math
import sys
import threading
import time
import urllib.request
from uuid import uuid4

from app import diag, tmux

_log = logging.getLogger("hangar.terminal_observer")
TIMEOUT = 0.25
MAX_FAILURES = 3
MAX_BACKOFF = 30.0
_io_pool = ThreadPoolExecutor(max_workers=4, thread_name_prefix="hangar-terminal")
_io_slots = threading.BoundedSemaphore(4)
_failures = 0
_retry_at = 0.0
_backoff = 1.0
_fallback = False
MAX_BODY = 16 * 1024 * 1024
HEARTBEAT = 20.0
_config: tuple[str, str] | None = None
_generation = 0
_epochs: dict[str, int] = {}
_bindings: dict[str, tuple[str, str]] = {}
_analysis: dict[str, tuple[tuple, float, str, dict]] = {}
_warned: set[str] = set()
_current: ContextVar[Lease | None] = ContextVar("terminal_observer", default=None)
_STATES = {"idle", "working", "awaiting_input", "dead"}


def configure(address: str | None, secret: str | None) -> None:
    global _config, _generation, _failures, _retry_at, _backoff, _fallback
    _config = None
    _generation += 1
    _analysis.clear()
    _warned.clear()
    _failures, _retry_at, _backoff = 0, 0.0, 1.0
    _fallback = False
    if address is not None and secret:
        # O Supervisor fornece um IP literal: não resolvemos nomes nem usamos proxies.
        if not isinstance(address, str) or len(address) > 128:
            raise ValueError("invalid terminal address")
        host, port = address.rsplit(":", 1)
        ip = ipaddress.ip_address(host.strip("[]"))
        if not ip.is_loopback or not 0 < int(port) <= 65535:
            raise ValueError("terminal bridge requires loopback")
        host = f"[{ip}]" if ip.version == 6 else str(ip)
        _config = (f"{host}:{int(port)}", secret)


def forget(name: str) -> None:
    _epochs[name] = _epochs.get(name, 0) + 1
    _analysis.pop(name, None)


def _failure(code: str) -> None:
    global _failures, _retry_at, _backoff, _fallback
    now = time.monotonic()
    if now >= _retry_at:
        _failures += 1
        if _failures >= MAX_FAILURES:
            _retry_at = now + _backoff
            _backoff = min(_backoff * 2, MAX_BACKOFF)
    if not _fallback:
        _fallback = True
        diag.registrar("terminal_observer.fallback", "aviso", codigo=code)
    if code not in _warned:
        _warned.add(code)
        _log.warning("observação terminal usa reserva Python: %s", code)


def _success() -> None:
    global _failures, _retry_at, _backoff, _fallback
    _failures, _retry_at, _backoff = 0, 0.0, 1.0
    if _fallback:
        diag.registrar("terminal_observer.recovered", "ok", codigo="rust_available")
        _fallback = False
    _warned.clear()


def _available() -> bool:
    return _config is not None and sys.platform != "win32" and time.monotonic() >= _retry_at


class _IoBusy(Exception):
    pass


async def _io(fn, *args):
    if not _io_slots.acquire(blocking=False):
        raise _IoBusy()
    slots = _io_slots
    try:
        job = _io_pool.submit(fn, *args)
    except BaseException:
        slots.release()
        raise
    # Cancelar a espera não libera a vaga de um trabalho que ainda está executando.
    job.add_done_callback(lambda _: slots.release())
    return await asyncio.wait_for(asyncio.wrap_future(job), TIMEOUT)


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        # O segredo interno só pertence ao endereço configurado pelo Supervisor.
        return None


def _http(config: tuple[str, str], payload: dict) -> dict | None:
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), _NoRedirect())
    req = urllib.request.Request(f"http://{config[0]}/__hangar_server/terminal",
        data=json.dumps(payload, allow_nan=False).encode("utf-8"),
        headers={"Content-Type": "application/json", "x-hangar-internal": config[1]}, method="POST")
    with opener.open(req, timeout=TIMEOUT) as response:
        if response.status != 200:
            raise urllib.error.HTTPError(req.full_url, response.status, "terminal status", None, None)
        body = response.read(MAX_BODY + 1)
        if len(body) > MAX_BODY:
            return None
        result = json.loads(body.decode("utf-8"), parse_constant=lambda _: None)
        return result if isinstance(result, dict) else None


async def _request(payload: dict) -> dict | None:
    config = _config
    generation = _generation
    if not _available():
        return None
    try:
        result = await _io(_http, config, payload)
        if generation != _generation:
            return None
        if result is None:
            _failure("invalid_http_response")
        elif payload.get("op") in ("acquire", "release"):
            if result != {}:
                _failure("invalid_http_response")
                return None
        return result
    except (OSError, ValueError, TimeoutError, HTTPException, _IoBusy) as exc:
        # Nem pane, segredo, URL ou mensagem de exceção entram no diário.
        if generation == _generation:
            if isinstance(exc, urllib.error.HTTPError):
                code = f"http_{exc.code}"
            elif isinstance(exc, TimeoutError) or (isinstance(exc, urllib.error.URLError)
                                                  and isinstance(exc.reason, TimeoutError)):
                code = "http_timeout"
            elif isinstance(exc, HTTPException):
                code = "http_protocol"
            elif isinstance(exc, _IoBusy):
                code = "io_busy"
            elif isinstance(exc, ValueError):
                code = "invalid_http_response"
            else:
                code = "http_connection"
            _failure(code)
        return None


def _strings(value) -> bool:
    return isinstance(value, list) and all(isinstance(v, str) for v in value)


def valid_analysis(value) -> bool:
    fields = {"state", "label", "question", "options", "spinner", "status_line", "overlay",
              "login", "limit_reset", "preview", "codex_menu"}
    if not isinstance(value, dict) or set(value) != fields or not isinstance(value["state"], str) or value["state"] not in _STATES:
        return False
    if any(value[k] is not None and not isinstance(value[k], str)
           for k in ("label", "question", "spinner", "status_line", "limit_reset")):
        return False
    if value["options"] is not None and not _strings(value["options"]):
        return False
    menu = value["codex_menu"]
    return (type(value["overlay"]) is bool and type(value["login"]) is bool
        and isinstance(value["preview"], str)
        and (menu is None or (isinstance(menu, dict) and set(menu) == {"question", "options"}
            and (menu["question"] is None or isinstance(menu["question"], str)) and _strings(menu["options"]))))


def valid_memory(value) -> bool:
    return (isinstance(value, dict) and set(value) == {"prev_spinner", "frozen", "no_spinner", "held_state", "held_label"}
        and isinstance(value["held_state"], str) and value["held_state"] in _STATES
        and all(type(value[k]) is int and 0 <= value[k] <= 0xffffffff for k in ("frozen", "no_spinner"))
        and all(value[k] is None or isinstance(value[k], str) for k in ("prev_spinner", "held_label")))


class Lease:
    def __init__(self, name, provider, binding_get):
        self.name, self.provider, self.binding_get = name, provider, binding_get
        self.consumer = uuid4().hex
        self.binding = None
        self.open = False
        self.remote_generation = None

    def identity(self):
        if not self.open or self.provider not in ("claude", "codex"):
            return None
        binding = self.binding_get()
        if not isinstance(binding, str) or not binding:
            return None
        if binding != self.binding:
            self.binding = binding
            _bindings[self.name] = (self.provider, binding)
            forget(self.name)
        if _bindings.get(self.name) != (self.provider, binding):
            return None
        return self.provider, binding, _epochs.get(self.name, 0), _generation

    async def __aenter__(self):
        try:
            await self.start()
        except BaseException:
            await self.close()
            raise
        self.token = _current.set(self)
        return self

    async def start(self):
        try:
            self.binding = self.binding_get()
            self.open = True
            if self.provider in ("claude", "codex") and isinstance(self.binding, str) and self.binding:
                if _bindings.get(self.name) != (self.provider, self.binding):
                    _bindings[self.name] = (self.provider, self.binding)
                    forget(self.name)
            await self.acquire()
        except Exception as exc:
            _failure(f"lease_start_{type(exc).__name__}")
            try:
                await self.close()
            except Exception as close_exc:
                _failure(f"lease_close_{type(close_exc).__name__}")

    async def __aexit__(self, *exc):
        _current.reset(self.token)
        await self.close()

    async def close(self):
        self.open = False
        remote_generation, self.remote_generation = self.remote_generation, None
        if remote_generation == _generation:
            await _request({"op": "release", "consumer": self.consumer})

    async def payload(self, op, started):
        identity = self.identity()
        if identity is None or not _available():
            return None
        try:
            target = await _io(tmux._pane_target, self.name)
        except (OSError, TimeoutError, _IoBusy) as exc:
            _failure(f"terminal_target_{type(exc).__name__}")
            return None
        if identity != self.identity():
            return None
        self.remote_generation = _generation
        return dict(op=op, consumer=self.consumer, name=self.name, provider=self.provider,
                    binding=identity[1], target=target, started=started, lines=200, colors=False, join=False)

    async def acquire(self):
        payload = await self.payload("acquire", time.monotonic())
        if payload is not None:
            await _request(payload)

    async def watch(self):
        while self.open:
            try:
                await self.acquire()
            except Exception as exc:
                # A falha não pode encerrar a renovação nem registrar conteúdo privado.
                _failure(f"lease_watch_{type(exc).__name__}")
            await asyncio.sleep(HEARTBEAT)


def lease(name, provider, binding_get) -> Lease:
    return Lease(name, provider, binding_get)


@contextmanager
def use(source):
    token = _current.set(source)
    try:
        yield
    finally:
        _current.reset(token)


def stamp(name: str) -> tuple:
    source = _current.get()
    identity = source.identity() if source is not None and source.name == name else None
    return (identity, _epochs.get(name, 0), _generation)


def retired(name: str) -> bool:
    source = _current.get()
    return (source is not None and source.open and source.name == name and source.provider in ("claude", "codex")
            and isinstance(source.binding, str) and bool(source.binding) and source.identity() is None)


async def capture(name: str, started: float) -> dict | None:
    source = _current.get()
    before = stamp(name)
    if source is None or source.name != name or type(started) not in (int, float) or not math.isfinite(started):
        return None
    payload = await source.payload("capture", started)
    if payload is None:
        return None
    result = await _request(payload)
    if before != stamp(name):
        return None
    if result is None:
        return None
    if not isinstance(result, dict) or set(result) != {"binding", "started", "text", "analysis"}:
        _failure("invalid_frame")
        return None
    if (result["binding"] != payload["binding"] or type(result["started"]) not in (int, float)
            or result["started"] != started or not isinstance(result["text"], str) or not valid_analysis(result["analysis"])):
        _failure("invalid_frame")
        return None
    _analysis[name] = (before, started, result["text"], result["analysis"])
    _success()
    return result


def frame_analysis(name: str, pane: str) -> dict | None:
    frame = _analysis.get(name)
    if frame is None or frame[0] != stamp(name) or frame[2] != pane:
        return None
    # A análise pertence também ao início do quadro no cache, nunca só ao texto.
    from app import state
    hit = state._frames.get(name)
    return frame[3] if hit is not None and hit == (frame[1], pane) else None


async def reduce(name: str, pane: str, memory: dict, facts: dict) -> dict | None:
    if not _available() or stamp(name)[0] is None:
        return None
    before = stamp(name)
    result = await _request(dict(op="reduce", pane=pane, memory=memory, facts=facts))
    if before != stamp(name):
        return None
    if result is None:
        return None
    if (not isinstance(result, dict) or set(result) != {"analysis", "memory", "diagnostic"}
            or not valid_analysis(result["analysis"]) or not valid_memory(result["memory"])):
        _failure("invalid_reducer")
        return None
    diagnostic = result["diagnostic"]
    if (not isinstance(diagnostic, dict) or set(diagnostic) != {"before_plugin", "plugin_applied"}
            or not isinstance(diagnostic["before_plugin"], str) or diagnostic["before_plugin"] not in _STATES
            or type(diagnostic["plugin_applied"]) is not bool):
        _failure("invalid_reducer")
        return None
    if (result["memory"]["held_state"] != result["analysis"]["state"]
            or result["memory"]["held_label"] != result["analysis"]["label"]
            or (diagnostic["plugin_applied"] and (facts["plugin_state"] not in ("working", "idle")
                or diagnostic["before_plugin"] not in ("working", "idle")))):
        _failure("invalid_reducer")
        return None
    _success()
    return result
