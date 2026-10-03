"""Ponte terminal em memória; todos os pedidos usam transporte sintético."""
import asyncio
import copy
import sys
from unittest.mock import patch

import pytest

from app import state


def bridge():
    from app import terminal_observer
    return terminal_observer


def analysis():
    return dict(state="idle", label=None, question=None, options=None, spinner=None,
                status_line=None, overlay=False, login=False, limit_reset=None,
                preview="resposta Rust", codex_menu=None)


@pytest.fixture(autouse=True)
def clean_frames(monkeypatch):
    blocked = []
    def deny(*args, **kwargs):
        blocked.append(args)
        raise AssertionError("real tmux forbidden in terminal observer tests")
    monkeypatch.setattr(state.tmux, "_run", deny)
    monkeypatch.setattr(state.tmux, "RUN", deny)
    import threading
    from concurrent.futures import ThreadPoolExecutor
    from app import diag
    monkeypatch.setattr(diag, "registrar", lambda *args, **kwargs: None)
    pool = ThreadPoolExecutor(max_workers=4)
    monkeypatch.setattr(bridge(), "_io_pool", pool)
    monkeypatch.setattr(bridge(), "_io_slots", threading.BoundedSemaphore(4))
    state._frames.clear()
    state._frames_inflight.clear()
    monkeypatch.setattr(state.tmux, "capture_pane", lambda *args: "Python")
    monkeypatch.setattr(state.tmux, "sessao_existe", lambda *args: True)
    yield
    pool.shutdown(wait=True)
    assert not blocked, "a fallback swallowed a forbidden tmux call"
    try:
        bridge().configure(None, None)
    except ImportError:
        pass


def fake_http(calls, fail=False):
    async def request(payload):
        calls.append(payload)
        if fail:
            return None
        if payload["op"] == "capture":
            return dict(binding=payload["binding"], started=payload["started"],
                        text="● resposta Rust", analysis=analysis())
        return {}
    return request


def test_disabled_and_windows_never_request(monkeypatch):
    t = bridge()
    calls = []
    monkeypatch.setattr(t, "_request", fake_http(calls))
    async def run():
        async with t.lease("s", "claude", lambda: "thread"):
            assert await t.capture("s", 1.0) is None
        t.configure("127.0.0.1:12345", "secret")
        monkeypatch.setattr(sys, "platform", "win32")
        async with t.lease("s", "claude", lambda: "thread"):
            assert await t.capture("s", 1.0) is None
    asyncio.run(run())
    assert not calls


@pytest.mark.parametrize("fail", [False, True])
def test_shared_capture_prefers_bridge_and_keeps_python_reserve(monkeypatch, fail):
    t = bridge()
    calls = []
    t.configure("127.0.0.1:12345", "secret")
    monkeypatch.setattr(t, "_request", fake_http(calls, fail))
    monkeypatch.setattr(state.tmux, "_pane_target", lambda name: "%8")
    monkeypatch.setattr(state.tmux, "capture_pane", lambda name: "Python")
    async def run():
        async with t.lease("s", "claude", lambda: "thread"):
            assert await state.shared_capture("s", 0) == ("Python" if fail else "● resposta Rust")
    asyncio.run(run())
    assert calls[0]["op"] == "acquire"
    assert calls[-1]["op"] == "release"


@pytest.mark.parametrize("field,value", [("binding", "old"), ("started", 2.0),
    ("started", True), ("text", 3), ("analysis", {}), ("analysis", None)])
def test_invalid_capture_returns_absence(monkeypatch, field, value):
    t = bridge()
    t.configure("127.0.0.1:12345", "secret")
    monkeypatch.setattr(state.tmux, "_pane_target", lambda name: "%8")
    async def request(payload):
        if payload["op"] != "capture":
            return {}
        answer = dict(binding=payload["binding"], started=payload["started"], text="", analysis=analysis())
        answer[field] = value
        return answer
    monkeypatch.setattr(t, "_request", request)
    async def run():
        async with t.lease("s", "claude", lambda: "thread"):
            assert await t.capture("s", 1.0) is None
    asyncio.run(run())


def test_clear_and_binding_change_discard_inflight(monkeypatch):
    t = bridge()
    t.configure("127.0.0.1:12345", "secret")
    monkeypatch.setattr(state.tmux, "_pane_target", lambda name: "%8")
    current = ["a"]
    async def run():
        started = asyncio.Event()
        finish = asyncio.Event()
        async def request(payload):
            if payload["op"] != "capture":
                return {}
            started.set()
            await finish.wait()
            return dict(binding=payload["binding"], started=payload["started"], text="old", analysis=analysis())
        monkeypatch.setattr(t, "_request", request)
        async with t.lease("s", "claude", lambda: current[0]):
            task = asyncio.create_task(state.shared_capture("s", 0))
            await started.wait()
            current[0] = "b"
            state.forget_frame("s")
            finish.set()
            assert await task == ""
            assert "s" not in state._frames
            assert t.frame_analysis("s", "old") is None
    asyncio.run(run())


def test_generation_change_discards_inflight(monkeypatch):
    t = bridge()
    t.configure("127.0.0.1:12345", "secret")
    monkeypatch.setattr(state.tmux, "_pane_target", lambda name: "%8")
    async def request(payload):
        if payload["op"] == "capture":
            t.configure("127.0.0.1:23456", "new")
            return dict(binding=payload["binding"], started=payload["started"], text="old", analysis=analysis())
        return {}
    monkeypatch.setattr(t, "_request", request)
    async def run():
        async with t.lease("s", "claude", lambda: "thread"):
            assert await t.capture("s", 1.0) is None
    asyncio.run(run())


def test_claude_reducer_hands_memory_to_python_on_failure(monkeypatch):
    t = bridge()
    t.configure("127.0.0.1:12345", "secret")
    monkeypatch.setattr(state.tmux, "_pane_target", lambda name: "%8")
    pane = "✻ Thinking…\n❯ "
    monkeypatch.setattr(state.tmux, "capture_pane", lambda name: pane)
    monkeypatch.setattr(state, "pergunta_aberta", lambda sid: None)
    monkeypatch.setattr(state, "_sidecar_status", lambda sid: None)
    monkeypatch.setattr(state.plugin_bridge, "pergunta_pendente", lambda name: None)
    monkeypatch.setattr(state.plugin_bridge, "estado_recente", lambda name: None)
    monkeypatch.setattr(state.plugin_bridge, "vivo", lambda name: False)
    monkeypatch.setattr(state.hook_state, "get_state", lambda sid: None)
    monkeypatch.setattr(state.hook_state, "shells", lambda sid: [])
    from app.loop import LoopLink
    monkeypatch.setattr(LoopLink, "get", lambda self: None)
    memories = []
    async def request(payload):
        if payload["op"] != "reduce":
            return None
        memories.append(copy.deepcopy(payload["memory"]))
        if len(memories) > 1:
            return None
        a = analysis()
        a.update(state="working", label="Thinking…", spinner="✻ Thinking…")
        return dict(analysis=a, memory=dict(prev_spinner="✻ Thinking…", frozen=2, no_spinner=0,
            held_state="working", held_label="Thinking…"), diagnostic=dict(before_plugin="working", plugin_applied=False))
    monkeypatch.setattr(t, "_request", request)
    async def run():
        monitor = state.StateMonitor("s", poll=0, sid_get=lambda: "thread", provider="claude")
        stream = monitor.stream()
        try:
            assert (await anext(stream)).state == "working"
            assert (await asyncio.wait_for(anext(stream), 1)).state == "idle"
        finally:
            await stream.aclose()
    asyncio.run(run())
    assert memories[0]["frozen"] == 0
    assert memories[1]["frozen"] == 2


def test_preview_empty_sidecar_and_rust_pane_analysis(monkeypatch):
    from app import preview
    t = bridge()
    t.configure("127.0.0.1:12345", "secret")
    calls = []
    monkeypatch.setattr(t, "_request", fake_http(calls))
    monkeypatch.setattr(state.tmux, "_pane_target", lambda name: "%8")
    sidecar = [""]
    monkeypatch.setattr(preview, "read_sidecar", lambda stem: sidecar[0])
    async def run():
        broker = preview.PreviewBroker("s", "claude", lambda: "thread")
        source = broker.subscribe()
        try:
            assert await asyncio.wait_for(anext(source), 1) == ("", False, False)
            assert await asyncio.wait_for(anext(source), 1) == ("", True, True)
            assert not any(c["op"] == "capture" for c in calls)
            sidecar[0] = None
            assert await asyncio.wait_for(anext(source), 2) == ("resposta Rust", False, False)
        finally:
            await source.aclose()
            if broker._task is not None:
                await asyncio.gather(broker._task, return_exceptions=True)
    asyncio.run(run())


def test_supervisor_enables_only_after_health_and_clears_even_without_proc(monkeypatch):
    from app import rust_server
    t = bridge()
    calls = []
    original = t.configure
    def configure(address, secret):
        calls.append((address, secret))
        original(address, secret)
    monkeypatch.setattr(t, "configure", configure)
    class Process:
        stdin = None
        def poll(self):
            return None
    monkeypatch.setattr(rust_server, "_spawn", lambda *args: Process())
    def health(*args):
        assert t._config is None
        return {"ok": True, "protocol": rust_server.RUST_SERVER_PROTOCOL, "terminal_address": "127.0.0.1:12347"}
    monkeypatch.setattr(rust_server, "_health", health)
    monkeypatch.setattr(rust_server, "server_log_path", lambda: "/tmp/unused-test-log")
    supervisor = rust_server.Supervisor(None, "0.0.0.0", 12345, 12346, "owner", "", lambda: False)
    async def run():
        assert await supervisor._start() == "up"
        assert t._config[0] == "127.0.0.1:12347"
        supervisor.proc = None
        await supervisor.stop()
        assert t._config is None
    asyncio.run(run())
    assert calls[0] == (None, None)
    assert calls[-1] == (None, None)


def test_monitor_lease_does_not_leak_into_consumer_context(monkeypatch):
    t = bridge()
    monkeypatch.setattr(state, "shared_capture", lambda *args: asyncio.sleep(0, result="● hello"))
    from app.loop import LoopLink
    monkeypatch.setattr(LoopLink, "get", lambda self: None)
    async def run():
        source = state.StateMonitor("s", provider="claude", sid_get=lambda: "thread").stream()
        try:
            await anext(source)
            assert t.stamp("s")[0] is None
        finally:
            await source.aclose()
    asyncio.run(run())


@pytest.mark.parametrize("body", [b'\xff', b'not-json', b'[]', b'{}' + b' ' * 32])
def test_http_rejects_utf8_json_and_bounded_body(monkeypatch, body):
    import urllib.request
    t = bridge()
    monkeypatch.setattr(t, "MAX_BODY", 16)
    requested = []
    class Response:
        status = 200
        def __enter__(self): return self
        def __exit__(self, *args): pass
        def read(self, size):
            assert size == 17
            return body[:size]
    class Opener:
        def open(self, req, timeout):
            requested.append((req, timeout))
            assert req.get_header("X-hangar-internal") == "secret"
            return Response()
    monkeypatch.setattr(urllib.request, "build_opener", lambda *handlers: Opener())
    t.configure("127.0.0.1:12345", "secret")
    assert asyncio.run(t._request({"op":"release", "consumer":"c"})) is None
    assert len(requested) == 1


def test_unconfigured_windows_and_excluded_provider_skip_target_resolution(monkeypatch):
    t = bridge()
    blocked = []
    def target(name):
        blocked.append(name)
        raise AssertionError("target resolution forbidden")
    monkeypatch.setattr(state.tmux, "_pane_target", target)
    async def run():
        async with t.lease("s", "claude", lambda: "b"):
            assert await t.capture("s", 1) is None
        t.configure("127.0.0.1:12345", "secret")
        async with t.lease("s", "kimi", lambda: "b"):
            assert await t.capture("s", 1) is None
        monkeypatch.setattr(sys, "platform", "win32")
        async with t.lease("s", "claude", lambda: "b"):
            assert await t.capture("s", 1) is None
    asyncio.run(run())
    assert not blocked


def test_same_binding_consumers_share_inflight_but_old_binding_cannot_reclaim(monkeypatch):
    t = bridge()
    t.configure("127.0.0.1:12345", "secret")
    monkeypatch.setattr(state.tmux, "_pane_target", lambda name: "%8")
    calls = []
    async def request(payload):
        calls.append(payload)
        if payload["op"] == "capture":
            await asyncio.sleep(0.02)
            return dict(binding=payload["binding"], started=payload["started"], text="same", analysis=analysis())
        return {}
    monkeypatch.setattr(t, "_request", request)
    async def run():
        old = t.lease("s", "claude", lambda: "a")
        preview = t.lease("s", "claude", lambda: "a")
        await old.start()
        await preview.start()
        async def capture(source):
            with t.use(source):
                return await state.shared_capture("s", 0.5)
        assert await asyncio.gather(capture(old), capture(preview)) == ["same", "same"]
        assert len([c for c in calls if c["op"] == "capture"]) == 1
        replacement = t.lease("s", "claude", lambda: "b")
        await replacement.start()
        assert old.identity() is None
        with t.use(replacement):
            assert await state.shared_capture("s", 0.5) == "same"
            assert t.frame_analysis("s", "same") == analysis()
        await old.close()
        assert replacement.identity() is not None
        await preview.close()
        await replacement.close()
    asyncio.run(run())


def test_clear_with_identical_binding_and_text_still_discards_frame(monkeypatch):
    t = bridge()
    t.configure("127.0.0.1:12345", "secret")
    monkeypatch.setattr(state.tmux, "_pane_target", lambda name: "%8")
    calls = []
    async def run():
        entered, finish = asyncio.Event(), asyncio.Event()
        async def request(payload):
            calls.append(payload)
            if payload["op"] != "capture": return {}
            entered.set()
            await finish.wait()
            return dict(binding=payload["binding"], started=payload["started"], text="same", analysis=analysis())
        monkeypatch.setattr(t, "_request", request)
        async with t.lease("s", "claude", lambda: "b"):
            task = asyncio.create_task(state.shared_capture("s", 0))
            await entered.wait()
            state.forget_frame("s")
            finish.set()
            assert await task == ""
            assert "s" not in state._frames
            assert t.frame_analysis("s", "same") is None
    asyncio.run(run())


@pytest.mark.parametrize("address", [None, "198.51.100.1:8765", "localhost:8765", "127.0.0.1:0", "0.0.0.0:8765", "http://127.0.0.1:8765", 1])
def test_supervisor_bad_health_address_keeps_bridge_disabled(monkeypatch, address):
    from app import rust_server
    t = bridge()
    class Process:
        stdin = None
        def poll(self): return None
    monkeypatch.setattr(rust_server, "_spawn", lambda *args: Process())
    monkeypatch.setattr(rust_server, "server_log_path", lambda: "/tmp/unused-test-log")
    monkeypatch.setattr(rust_server, "_health", lambda *args: dict(ok=True, protocol=2, terminal_address=address))
    supervisor = rust_server.Supervisor(None, "0.0.0.0", 12345, 12346, "owner", "", lambda: False)
    assert asyncio.run(supervisor._start()) == "up"
    assert t._config is None


def test_retired_monitor_never_publishes_again(monkeypatch):
    t = bridge()
    from app.loop import LoopLink
    monkeypatch.setattr(LoopLink, "get", lambda self: None)
    monkeypatch.setattr(state, "pergunta_aberta", lambda sid: None)
    monkeypatch.setattr(state.hook_state, "get_state", lambda sid: None)
    monkeypatch.setattr(state.hook_state, "shells", lambda sid: [])
    monkeypatch.setattr(state.plugin_bridge, "vivo", lambda name: False)
    async def run():
        monitor = state.StateMonitor("s", poll=0, provider="claude", sid_get=lambda: "old").stream()
        assert (await anext(monitor)).state == "idle"
        replacement = t.lease("s", "claude", lambda: "new")
        await replacement.start()
        monkeypatch.setattr(state.tmux, "capture_pane", lambda name: "✻ Thinking…")
        with pytest.raises(TimeoutError):
            await asyncio.wait_for(anext(monitor), 0.05)
        await replacement.close()
        await monitor.aclose()
    asyncio.run(run())


def test_python_contract_generator_explicitly_disables_bridge(monkeypatch):
    import importlib.util
    from pathlib import Path
    t = bridge()
    t.configure("127.0.0.1:12345", "secret")
    blocked = []
    async def request(payload):
        blocked.append(payload)
        raise AssertionError("reference generator cannot use Rust")
    monkeypatch.setattr(t, "_request", request)
    path = Path(__file__).parent / "fixtures" / "contract" / "gen_terminal.py"
    spec = importlib.util.spec_from_file_location("terminal_reference", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    frames = [dict(pane="✻ Thinking…", facts={})] * 4
    result = asyncio.run(module.reference_sequence(frames))
    assert result[-1]["analysis"]["state"] == "idle"
    assert result[-1]["memory"]["frozen"] == 3
    assert not blocked


def test_sidecar_preview_producer_keeps_lease_alive_without_capture(monkeypatch):
    from app import preview
    t = bridge()
    t.configure("127.0.0.1:12345", "secret")
    monkeypatch.setattr(t, "HEARTBEAT", 0.01)
    monkeypatch.setattr(state.tmux, "_pane_target", lambda name: "%8")
    calls = []
    monkeypatch.setattr(t, "_request", fake_http(calls))
    monkeypatch.setattr(preview, "read_sidecar", lambda stem: "sidecar")
    async def run():
        broker = preview.PreviewBroker("s", "claude", lambda: "b")
        task = asyncio.create_task(broker._loop())
        await asyncio.sleep(0.04)
        task.cancel()
        await asyncio.gather(task, return_exceptions=True)
    asyncio.run(run())
    assert len([c for c in calls if c["op"] == "acquire"]) >= 2
    assert not any(c["op"] == "capture" for c in calls)
    assert calls[-1]["op"] == "release"


def test_bridge_error_is_visible_without_raw_response_or_secret(monkeypatch, caplog):
    t = bridge()
    t.configure("127.0.0.1:12345", "secret")
    def failure(*args):
        raise ValueError("private pane and secret")
    monkeypatch.setattr(t, "_http", failure)
    assert asyncio.run(t._request({"op":"release", "consumer":"c"})) is None
    assert any(r.levelname == "WARNING" for r in caplog.records)
    assert "private pane" not in caplog.text and "secret" not in caplog.text


@pytest.mark.parametrize("problem", ["memory-state", "plugin-without-fact"])
def test_reducer_rejects_inconsistent_complete_response_before_memory_update(monkeypatch, problem):
    t = bridge()
    t.configure("127.0.0.1:12345", "secret")
    monkeypatch.setattr(state.tmux, "_pane_target", lambda name: "%8")
    memory = dict(prev_spinner=None, frozen=2, no_spinner=3, held_state="idle", held_label=None)
    facts = dict(open_question=None, plugin_question=None, plugin_state=None, hook_state=None, hook_grace=8, status_line=None)
    response = dict(analysis=analysis(), memory=copy.deepcopy(memory), diagnostic=dict(before_plugin="idle", plugin_applied=False))
    if problem == "memory-state": response["memory"]["held_state"] = "working"
    else: response["diagnostic"]["plugin_applied"] = True
    async def request(payload): return response if payload["op"] == "reduce" else {}
    monkeypatch.setattr(t, "_request", request)
    async def run():
        async with t.lease("s", "claude", lambda: "b"):
            assert await t.reduce("s", "pane", memory, facts) is None
    asyncio.run(run())
    assert memory == dict(prev_spinner=None, frozen=2, no_spinner=3, held_state="idle", held_label=None)


def test_stdlib_bridge_performs_real_loopback_http_without_environment_proxy(monkeypatch):
    import json
    import threading
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
    t = bridge()
    calls = []
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            assert self.path == "/__hangar_server/terminal"
            assert self.headers["x-hangar-internal"] == "test-only"
            payload = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            calls.append(payload)
            result = {}
            if payload["op"] == "capture":
                result = dict(binding=payload["binding"], started=payload["started"], text="Rust", analysis=analysis())
            elif payload["op"] == "reduce":
                result = dict(analysis=analysis(), memory=payload["memory"], diagnostic=dict(before_plugin="idle", plugin_applied=False))
            body = json.dumps(result).encode()
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        def log_message(self, *args): pass
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    monkeypatch.setenv("HTTP_PROXY", "http://198.51.100.1:12345")
    monkeypatch.setenv("NO_PROXY", "")
    monkeypatch.setattr(state.tmux, "_pane_target", lambda name: "%8")
    t.configure(f"127.0.0.1:{server.server_port}", "test-only")
    async def run():
        async with t.lease("s", "claude", lambda: "b"):
            assert await state.shared_capture("s", 0) == "Rust"
            memory = dict(prev_spinner=None, frozen=0, no_spinner=0, held_state="idle", held_label=None)
            facts = dict(open_question=None, plugin_question=None, plugin_state=None, hook_state=None, hook_grace=8, status_line=None)
            assert (await t.reduce("s", "Rust", memory, facts))["analysis"] == analysis()
    try:
        asyncio.run(run())
    finally:
        server.shutdown()
        server.server_close()
        thread.join(1)
    assert [c["op"] for c in calls] == ["acquire", "capture", "reduce", "release"]


def test_invalid_plugin_label_keeps_python_behavior_and_facts_are_read_once(monkeypatch):
    from pydantic import ValidationError
    t = bridge()
    t.configure("127.0.0.1:12345", "secret")
    monkeypatch.setattr(state.tmux, "_pane_target", lambda name: "%8")
    counters = dict(question=0, plugin_question=0, plugin_state=0, hook=0, status=0)
    question = {"id":"q", "questions":[{"question":"choose", "options":[{"label":None}]}]}
    def collect(key, value):
        def get(*args):
            counters[key] += 1
            return value
        return get
    monkeypatch.setattr(state, "pergunta_aberta", collect("question", None))
    monkeypatch.setattr(state.plugin_bridge, "pergunta_pendente", collect("plugin_question", question))
    monkeypatch.setattr(state.plugin_bridge, "estado_recente", collect("plugin_state", ("working", None)))
    monkeypatch.setattr(state.hook_state, "get_state", collect("hook", None))
    monkeypatch.setattr(state.hook_state, "shells", lambda sid: [])
    monkeypatch.setattr(state, "_sidecar_status", collect("status", "sidecar"))
    from app.loop import LoopLink
    monkeypatch.setattr(LoopLink, "get", lambda self: None)
    seen = []
    async def request(payload):
        if payload["op"] == "reduce": seen.append(payload)
        return None
    monkeypatch.setattr(t, "_request", request)
    async def run():
        stream = state.StateMonitor("s", sid_get=lambda: "thread", provider="claude").stream()
        try:
            with pytest.raises(ValidationError, match="options.0"):
                await anext(stream)
        finally:
            await stream.aclose()
    asyncio.run(run())
    assert counters == dict(question=1, plugin_question=1, plugin_state=1, hook=1, status=1)
    assert seen[0]["facts"]["plugin_question"] == question


@pytest.mark.parametrize("kind", ["bad-status", "incomplete-read"])
def test_http_protocol_errors_use_reserve_without_raw_diagnostics(monkeypatch, caplog, kind):
    import http.client
    t = bridge()
    t.configure("127.0.0.1:12345", "test-only")
    def failure(*args):
        if kind == "bad-status": raise http.client.BadStatusLine("dummy-private")
        raise http.client.IncompleteRead(b"dummy-private", 100)
    monkeypatch.setattr(t, "_http", failure)
    assert asyncio.run(t._request({"op":"release", "consumer":"c"})) is None
    assert "http_protocol" in caplog.text
    assert "dummy-private" not in caplog.text and "test-only" not in caplog.text


@pytest.mark.parametrize("kind", ["bad-status", "incomplete-read"])
def test_http_protocol_failure_preserves_capture_and_preview_python_reserve(monkeypatch, kind):
    import http.client
    from app import preview
    t = bridge()
    t.configure("127.0.0.1:12345", "test-only")
    monkeypatch.setattr(state.tmux, "_pane_target", lambda name: "%8")
    monkeypatch.setattr(state.tmux, "capture_pane", lambda name: "● Python reserve\n")
    monkeypatch.setattr(preview, "read_sidecar", lambda stem: None)
    def http_response(config, payload):
        if payload["op"] == "capture":
            if kind == "bad-status": raise http.client.BadStatusLine("dummy-private")
            raise http.client.IncompleteRead(b"dummy-private", 100)
        return {}
    monkeypatch.setattr(t, "_http", http_response)
    async def run():
        async with t.lease("s", "claude", lambda: "b"):
            assert await state.shared_capture("s", 0) == "● Python reserve\n"
        state.forget_frame("s")
        broker = preview.PreviewBroker("s", "claude", lambda: "b")
        source = broker.subscribe()
        try:
            assert await anext(source) == ("", False, False)
            assert await asyncio.wait_for(anext(source), 1) == ("Python reserve", False, False)
        finally:
            task = broker._task
            await source.aclose()
            if task is not None:
                await asyncio.gather(task, return_exceptions=True)
    asyncio.run(run())


@pytest.mark.parametrize("kind", ["bad-status", "incomplete-read"])
def test_http_protocol_failure_hands_reducer_memory_to_python(monkeypatch, kind):
    import http.client
    t = bridge()
    t.configure("127.0.0.1:12345", "test-only")
    monkeypatch.setattr(state.tmux, "_pane_target", lambda name: "%8")
    monkeypatch.setattr(state.tmux, "capture_pane", lambda name: "✻ Thinking…\n❯ ")
    monkeypatch.setattr(state, "pergunta_aberta", lambda sid: None)
    monkeypatch.setattr(state, "_sidecar_status", lambda sid: None)
    monkeypatch.setattr(state.plugin_bridge, "pergunta_pendente", lambda name: None)
    monkeypatch.setattr(state.plugin_bridge, "estado_recente", lambda name: None)
    monkeypatch.setattr(state.plugin_bridge, "vivo", lambda name: False)
    monkeypatch.setattr(state.hook_state, "get_state", lambda sid: None)
    monkeypatch.setattr(state.hook_state, "shells", lambda sid: [])
    from app.loop import LoopLink
    monkeypatch.setattr(LoopLink, "get", lambda self: None)
    memories = []
    def http_response(config, payload):
        if payload["op"] != "reduce": return None if payload["op"] == "capture" else {}
        memories.append(copy.deepcopy(payload["memory"]))
        if len(memories) == 1:
            a = analysis()
            a.update(state="working", label="Thinking…", spinner="✻ Thinking…")
            return dict(analysis=a, memory=dict(prev_spinner="✻ Thinking…", frozen=2, no_spinner=0,
                held_state="working", held_label="Thinking…"), diagnostic=dict(before_plugin="working", plugin_applied=False))
        if kind == "bad-status": raise http.client.BadStatusLine("dummy-private")
        raise http.client.IncompleteRead(b"dummy-private", 100)
    monkeypatch.setattr(t, "_http", http_response)
    async def run():
        source = state.StateMonitor("s", poll=0, provider="claude", sid_get=lambda: "b").stream()
        try:
            assert (await anext(source)).state == "working"
            assert (await asyncio.wait_for(anext(source), 1)).state == "idle"
        finally:
            await source.aclose()
    asyncio.run(run())
    assert memories[1]["frozen"] == 2


@pytest.mark.parametrize("status", [301, 302, 303, 307, 308])
@pytest.mark.parametrize("location", ["http://198.51.100.1:12345/other", "http://127.0.0.1:12345/other"])
def test_terminal_bridge_refuses_every_redirect_before_second_request(monkeypatch, status, location):
    import email.message
    import io
    import urllib.request
    import urllib.response
    t = bridge()
    t.configure("127.0.0.1:12345", "test-only")
    calls = []
    class Transport(urllib.request.HTTPHandler):
        def http_open(self, req):
            calls.append((req.full_url, req.get_header("X-hangar-internal")))
            if len(calls) > 1:
                assert calls[-1][1] == "test-only"
                raise AssertionError("redirect must never reach another location")
            headers = email.message.Message()
            headers["Location"] = location
            response = urllib.response.addinfourl(io.BytesIO(b""), headers, req.full_url, status)
            response.msg = "Synthetic redirect"
            return response
    original = urllib.request.build_opener
    monkeypatch.setattr(urllib.request, "build_opener", lambda *handlers: original(*handlers, Transport()))
    assert asyncio.run(t._request({"op":"release", "consumer":"c"})) is None
    assert calls == [("http://127.0.0.1:12345/__hangar_server/terminal", "test-only")]


def test_http_reserve_does_not_swallow_request_cancellation(monkeypatch):
    import threading
    t = bridge()
    t.configure("127.0.0.1:12345", "test-only")
    release = threading.Event()
    async def run():
        entered = asyncio.Event()
        loop = asyncio.get_running_loop()
        def pending(*args):
            loop.call_soon_threadsafe(entered.set)
            assert release.wait(2)
            return {}
        monkeypatch.setattr(t, "_http", pending)
        task = asyncio.create_task(t._request({"op":"release", "consumer":"c"}))
        try:
            await asyncio.wait_for(entered.wait(), 1)
            task.cancel()
            with pytest.raises(asyncio.CancelledError):
                await task
            assert task.cancelled()
        finally:
            release.set()
    asyncio.run(run())


@pytest.mark.parametrize("enabled", [False, True])
def test_preview_lease_follows_replaced_getter_after_clear_without_restarting_subscriber(monkeypatch, enabled):
    from app import preview
    t = bridge()
    t.configure("127.0.0.1:12345" if enabled else None, "test-only" if enabled else None)
    monkeypatch.setattr(state.tmux, "_pane_target", lambda name: "%8")
    calls = []
    monkeypatch.setattr(t, "_request", fake_http(calls))
    monkeypatch.setattr(preview, "read_sidecar", lambda stem: f"preview-{stem}")
    old_stem, current_stem = ["a"], ["a"]
    old_getter = lambda: old_stem[0]
    current_getter = lambda: current_stem[0]
    name = "preview-current-getter"
    async def run():
        broker = preview.PreviewBroker.get(name, "claude", old_getter)
        subscriber = broker.subscribe()
        monitor = None
        task = None
        try:
            assert await anext(subscriber) == ("", False, False)
            assert await asyncio.wait_for(anext(subscriber), 1) == ("preview-a", True, True)
            task = broker._task
            assert preview.PreviewBroker.get(name, "claude", current_getter) is broker
            current_stem[0] = "b"
            assert old_getter() == "a"
            monitor = t.lease(name, "claude", current_getter)
            await monitor.start()
            broker.reset()
            assert await asyncio.wait_for(anext(subscriber), 1) == ("preview-b", True, True)
            assert broker._task is task and not task.done()
            assert broker._subs == 1
        finally:
            await subscriber.aclose()
            if task is not None:
                await asyncio.gather(task, return_exceptions=True)
            if monitor is not None:
                await monitor.close()
    asyncio.run(run())
    assert bool(calls) is enabled


@pytest.mark.parametrize("enabled", [False, True])
def test_delayed_sse_clear_keeps_preview_producer_and_discards_old_sidecar(monkeypatch, enabled):
    from app import preview
    import threading
    t = bridge()
    t.configure("127.0.0.1:12345" if enabled else None, "test-only" if enabled else None)
    monkeypatch.setattr(state.tmux, "_pane_target", lambda name: "%8")
    monkeypatch.setattr(t, "_request", fake_http([]))
    first, delayed = ["a"], ["a"]
    name = "preview-delayed-clear"
    release = threading.Event()
    reads = [0]
    async def run():
        entered = asyncio.Event()
        loop = asyncio.get_running_loop()
        def sidecar(stem):
            reads[0] += 1
            if reads[0] == 2:
                loop.call_soon_threadsafe(entered.set)
                assert release.wait(2)
                return "discarded-old-sidecar"
            return f"preview-{stem}"
        monkeypatch.setattr(preview, "read_sidecar", sidecar)
        broker = preview.PreviewBroker.get(name, "claude", lambda: first[0])
        left = broker.subscribe()
        right = None
        task = None
        fast = None
        try:
            await anext(left)
            assert await asyncio.wait_for(anext(left), 1) == ("preview-a", True, True)
            assert preview.PreviewBroker.get(name, "claude", lambda: delayed[0]) is broker
            right = broker.subscribe()
            assert await anext(right) == ("preview-a", True, True)
            task = broker._task
            await asyncio.wait_for(entered.wait(), 1)
            first[0] = "b"
            fast = t.lease(name, "claude", lambda: first[0])
            await fast.start()
            broker.reset()
            release.set()
            await asyncio.sleep(0.3)
            assert not task.done(), "a delayed SSE getter terminated the shared producer"
            assert broker.text == ""
            delayed[0] = "b"
            assert await asyncio.wait_for(anext(left), 2) == ("preview-b", True, True)
            assert await asyncio.wait_for(anext(right), 1) == ("preview-b", True, True)
            assert broker._task is task
        finally:
            release.set()
            await left.aclose()
            if right is not None:
                await right.aclose()
            if task is not None:
                await asyncio.gather(task, return_exceptions=True)
            if fast is not None:
                await fast.close()
    asyncio.run(run())


@pytest.mark.parametrize("enabled", [False, True])
def test_delayed_sse_clear_keeps_state_stream_until_getter_converges(monkeypatch, enabled):
    t = bridge()
    t.configure("127.0.0.1:12345" if enabled else None, "test-only" if enabled else None)
    monkeypatch.setattr(state.tmux, "_pane_target", lambda name: "%8")
    monkeypatch.setattr(t, "_request", fake_http([], fail=True))
    monkeypatch.setattr(state, "pergunta_aberta", lambda sid: None)
    monkeypatch.setattr(state, "_sidecar_status", lambda sid: None)
    monkeypatch.setattr(state.plugin_bridge, "pergunta_pendente", lambda name: None)
    monkeypatch.setattr(state.plugin_bridge, "estado_recente", lambda name: None)
    monkeypatch.setattr(state.plugin_bridge, "vivo", lambda name: False)
    monkeypatch.setattr(state.hook_state, "get_state", lambda sid: None)
    monkeypatch.setattr(state.hook_state, "shells", lambda sid: [])
    from app.loop import LoopLink
    monkeypatch.setattr(LoopLink, "get", lambda self: None)
    delayed = ["a"]
    pane = ["✻ Thinking…\n❯ "]
    monkeypatch.setattr(state.tmux, "capture_pane", lambda name: pane[0])
    monkeypatch.setattr(state.StateMonitor, "FRAME_MAX_AGE", 0)
    async def run():
        stream = state.StateMonitor("state-delayed-clear", poll=0.01,
            sid_get=lambda: delayed[0], provider="claude").stream()
        fast = None
        pending = None
        try:
            assert (await anext(stream)).state == "working"
            fast = t.lease("state-delayed-clear", "claude", lambda: "b")
            await fast.start()
            state.forget_frame("state-delayed-clear")
            pending = asyncio.create_task(anext(stream))
            await asyncio.sleep(0.03)
            assert not pending.done(), "a delayed SSE getter terminated the state stream"
            delayed[0] = "b"
            pane[0] = "❯ "
            resumed = await asyncio.wait_for(pending, 1)
            assert resumed.state in ("working", "idle")
            if resumed.state == "working":
                assert (await asyncio.wait_for(anext(stream), 1)).state == "idle"
        finally:
            if pending is not None:
                if not pending.done():
                    pending.cancel()
                await asyncio.gather(pending, return_exceptions=True)
            await stream.aclose()
            if fast is not None:
                await fast.close()
    asyncio.run(run())


def test_preview_new_subscriber_restarts_completed_producer(monkeypatch):
    from app import preview
    t = bridge()
    t.configure(None, None)
    monkeypatch.setattr(preview, "read_sidecar", lambda stem: "resumed-preview")
    async def run():
        broker = preview.PreviewBroker("preview-dead-producer", "claude", lambda: "a")
        broker._task = asyncio.create_task(asyncio.sleep(0))
        await broker._task
        subscriber = broker.subscribe()
        task = None
        try:
            assert await anext(subscriber) == ("", False, False)
            assert await asyncio.wait_for(anext(subscriber), 1) == ("resumed-preview", True, True)
            task = broker._task
            assert not task.done()
        finally:
            task = broker._task
            await subscriber.aclose()
            if task is not None:
                await asyncio.gather(task, return_exceptions=True)
    asyncio.run(run())


@pytest.mark.parametrize("failure_site", ["target", "binding"])
def test_lease_watch_logs_exception_type_and_recovers_heartbeat(monkeypatch, caplog, failure_site):
    t = bridge()
    t.configure("127.0.0.1:12345", "test-only")
    monkeypatch.setattr(t, "HEARTBEAT", 0.01)
    fail_next = [False]
    def target(name):
        if failure_site == "target" and fail_next[0]:
            fail_next[0] = False
            raise RuntimeError("private-pane-and-secret")
        return "%8"
    def binding():
        if failure_site == "binding" and fail_next[0]:
            fail_next[0] = False
            raise RuntimeError("private-pane-and-secret")
        return "thread"
    monkeypatch.setattr(state.tmux, "_pane_target", target)
    async def run():
        recovered = asyncio.Event()
        calls = []
        async def request(payload):
            calls.append(payload)
            if len([p for p in calls if p["op"] == "acquire"]) >= 2:
                recovered.set()
            return {}
        monkeypatch.setattr(t, "_request", request)
        source = t.lease("watch-exception", "claude", binding)
        await source.start()
        fail_next[0] = True
        watcher = asyncio.create_task(source.watch())
        try:
            await asyncio.wait_for(recovered.wait(), 0.2)
            assert source.open and not watcher.done()
        finally:
            watcher.cancel()
            await asyncio.gather(watcher, return_exceptions=True)
            await source.close()
    asyncio.run(run())
    assert "RuntimeError" in caplog.text
    assert "private-pane-and-secret" not in caplog.text
    assert "test-only" not in caplog.text


def test_preview_cleanup_propagates_unexpected_heartbeat_failure(monkeypatch):
    from app import preview
    t = bridge()
    t.configure(None, None)
    async def broken_watch(self):
        raise RuntimeError("heartbeat failed")
    monkeypatch.setattr(t.Lease, "watch", broken_watch)
    monkeypatch.setattr(preview, "read_sidecar", lambda stem: "preview")
    async def run():
        broker = preview.PreviewBroker("heartbeat-cleanup", "claude", lambda: "a")
        task = asyncio.create_task(broker._loop())
        async with broker._cond:
            await asyncio.wait_for(broker._cond.wait_for(lambda: broker.text == "preview"), 1)
        task.cancel()
        with pytest.raises(RuntimeError, match="heartbeat failed"):
            await task
    asyncio.run(run())


@pytest.mark.parametrize("failure_site", ["target", "acquire", "acquire-release", "binding"])
@pytest.mark.parametrize("consumer", ["preview", "state"])
def test_lease_start_failure_preserves_python_consumer(monkeypatch, caplog, failure_site, consumer):
    from app import preview
    t = bridge()
    t.configure("127.0.0.1:12345", "test-only")
    def target(name):
        if failure_site == "target":
            raise RuntimeError("private-pane-and-secret")
        return "%8"
    monkeypatch.setattr(state.tmux, "_pane_target", target)
    calls = []
    async def request(payload):
        calls.append(payload)
        if ((payload["op"] == "acquire" and failure_site in ("acquire", "acquire-release"))
                or (payload["op"] == "release" and failure_site == "acquire-release")):
            raise RuntimeError("private-pane-and-secret")
        return None
    monkeypatch.setattr(t, "_request", request)
    reads = [0]
    def binding():
        reads[0] += 1
        if failure_site == "binding" and reads[0] == 1:
            raise RuntimeError("private-pane-and-secret")
        return "thread"
    monkeypatch.setattr(preview, "read_sidecar", lambda stem: None)
    pane = ["● Python reserve\n" if consumer == "preview" else "✻ Thinking…\n❯ "]
    monkeypatch.setattr(state.tmux, "capture_pane", lambda name: pane[0])
    monkeypatch.setattr(state, "pergunta_aberta", lambda sid: None)
    monkeypatch.setattr(state, "_sidecar_status", lambda sid: None)
    monkeypatch.setattr(state.plugin_bridge, "pergunta_pendente", lambda name: None)
    monkeypatch.setattr(state.plugin_bridge, "estado_recente", lambda name: None)
    monkeypatch.setattr(state.plugin_bridge, "vivo", lambda name: False)
    monkeypatch.setattr(state.hook_state, "get_state", lambda sid: None)
    monkeypatch.setattr(state.hook_state, "shells", lambda sid: [])
    monkeypatch.setattr(state.StateMonitor, "FRAME_MAX_AGE", 0)
    from app.loop import LoopLink
    monkeypatch.setattr(LoopLink, "get", lambda self: None)
    async def run():
        if consumer == "preview":
            broker = preview.PreviewBroker("start-failure-preview", "claude", binding)
            subscriber = broker.subscribe()
            task = None
            try:
                assert await anext(subscriber) == ("", False, False)
                assert await asyncio.wait_for(anext(subscriber), 1) == ("Python reserve", False, False)
                task = broker._task
                assert not task.done()
            finally:
                task = broker._task
                await subscriber.aclose()
                if task is not None:
                    await asyncio.gather(task, return_exceptions=True)
        else:
            stream = state.StateMonitor("start-failure-state", poll=0,
                sid_get=binding, provider="claude").stream()
            try:
                assert (await asyncio.wait_for(anext(stream), 1)).state == "working"
                pane[0] = "❯ "
                event = await asyncio.wait_for(anext(stream), 1)
                if event.state == "working":
                    event = await asyncio.wait_for(anext(stream), 1)
                assert event.state == "idle"
            finally:
                await stream.aclose()
    asyncio.run(run())
    assert "RuntimeError" in caplog.text
    assert "private-pane-and-secret" not in caplog.text
    assert "test-only" not in caplog.text
    assert not any(p["op"] in ("capture", "reduce") for p in calls)


def test_failed_lease_start_leaves_closed_source_and_allows_python_capture(monkeypatch):
    t = bridge()
    t.configure("127.0.0.1:12345", "test-only")
    def failed_target(name):
        raise RuntimeError("private-pane-and-secret")
    monkeypatch.setattr(state.tmux, "_pane_target", failed_target)
    async def run():
        async with t.lease("closed-start-failure", "claude", lambda: "thread") as source:
            assert not source.open
            assert source.identity() is None
            assert not t.retired("closed-start-failure")
            assert await state.shared_capture("closed-start-failure", 0) == "Python"
    asyncio.run(run())


def test_stalled_bridge_many_chats_keep_default_executor_free_and_bound_jobs(monkeypatch):
    import threading
    from concurrent.futures import ThreadPoolExecutor
    t = bridge()
    t.configure("127.0.0.1:12345", "test-only")
    monkeypatch.setattr(t, "TIMEOUT", 0.05)
    release = threading.Event()
    lock = threading.Lock()
    active, peak = [0], [0]
    def stalled(config, payload):
        with lock:
            active[0] += 1
            peak[0] = max(peak[0], active[0])
        try:
            assert release.wait(2)
            return {}
        finally:
            with lock:
                active[0] -= 1
    monkeypatch.setattr(t, "_http", stalled)
    async def run():
        loop = asyncio.get_running_loop()
        loop.set_default_executor(ThreadPoolExecutor(max_workers=1))
        started = loop.time()
        try:
            results = await asyncio.wait_for(asyncio.gather(*(
                t._request({"op": "release", "consumer": f"chat-{i}"}) for i in range(20))), 1)
            assert results == [None] * 20
            assert loop.time() - started < 0.3
            assert await asyncio.wait_for(asyncio.to_thread(lambda: "free"), 0.1) == "free"
            assert 1 <= peak[0] <= 4
        finally:
            release.set()
    asyncio.run(run())


def test_request_cancellation_keeps_running_io_capacity_until_job_finishes(monkeypatch):
    import threading
    from concurrent.futures import ThreadPoolExecutor
    t = bridge()
    t.configure("127.0.0.1:12345", "test-only")
    pool = ThreadPoolExecutor(max_workers=1)
    monkeypatch.setattr(t, "_io_pool", pool, raising=False)
    monkeypatch.setattr(t, "_io_slots", threading.BoundedSemaphore(1), raising=False)
    release = threading.Event()
    calls = []
    async def run():
        entered = asyncio.Event()
        loop = asyncio.get_running_loop()
        def stalled(config, payload):
            calls.append(payload)
            loop.call_soon_threadsafe(entered.set)
            assert release.wait(2)
            return {}
        monkeypatch.setattr(t, "_http", stalled)
        task = asyncio.create_task(t._request({"op": "release", "consumer": "first"}))
        try:
            await asyncio.wait_for(entered.wait(), 1)
            task.cancel()
            with pytest.raises(asyncio.CancelledError):
                await task
            assert await asyncio.wait_for(t._request({"op": "release", "consumer": "second"}), 0.1) is None
            assert len(calls) == 1
        finally:
            release.set()
            if not task.done():
                task.cancel()
            await asyncio.gather(task, return_exceptions=True)
    try:
        asyncio.run(run())
    finally:
        release.set()
        pool.shutdown(wait=True)


def test_bridge_circuit_pauses_after_three_failures_grows_and_resets(monkeypatch):
    from types import SimpleNamespace
    t = bridge()
    now = [100.0]
    monkeypatch.setattr(t, "time", SimpleNamespace(monotonic=lambda: now[0]))
    t.configure("127.0.0.1:12345", "test-only")
    monkeypatch.setattr(state.tmux, "_pane_target", lambda name: "%8")
    calls = []
    fail = [True]
    def response(config, payload):
        if payload["op"] == "acquire":
            return {}
        calls.append(payload)
        if fail[0]:
            raise TimeoutError("private-pane-and-secret")
        if payload["op"] == "capture":
            return dict(binding=payload["binding"], started=payload["started"], text="", analysis=analysis())
        return {}
    monkeypatch.setattr(t, "_http", response)
    async def run():
        source = t.lease("circuit-recovery", "claude", lambda: "b")
        await source.start()
        for _ in range(3):
            assert await t._request({"op": "release", "consumer": "c"}) is None
        assert await t._request({"op": "release", "consumer": "c"}) is None
        assert len(calls) == 3
        now[0] = 101.0
        assert await t._request({"op": "release", "consumer": "c"}) is None
        now[0] = 102.0
        assert await t._request({"op": "release", "consumer": "c"}) is None
        assert len(calls) == 4
        now[0] = 103.0
        fail[0] = False
        with t.use(source):
            assert await t.capture("circuit-recovery", 103.0) is not None
        fail[0] = True
        for _ in range(3):
            assert await t._request({"op": "release", "consumer": "c"}) is None
        t.configure("127.0.0.1:23456", "another-test-only")
        fail[0] = False
        assert await t._request({"op": "release", "consumer": "c"}) == {}
        await source.close()
    asyncio.run(run())


def test_malformed_capture_counts_as_failure_and_stops_io_until_retry(monkeypatch):
    t = bridge()
    t.configure("127.0.0.1:12345", "test-only")
    monkeypatch.setattr(state.tmux, "_pane_target", lambda name: "%8")
    calls = []
    def response(config, payload):
        calls.append(payload)
        return {}  # Corpo válido, quadro incompleto.
    monkeypatch.setattr(t, "_http", response)
    async def run():
        async with t.lease("malformed-circuit", "claude", lambda: "b"):
            for i in range(4):
                assert await t.capture("malformed-circuit", float(i)) is None
    asyncio.run(run())
    assert len([p for p in calls if p["op"] == "capture"]) == 3


def test_old_generation_io_failure_cannot_close_reconfigured_bridge(monkeypatch):
    import threading
    t = bridge()
    t.configure("127.0.0.1:12345", "old-test-only")
    release = threading.Event()
    async def run():
        entered = asyncio.Event()
        loop = asyncio.get_running_loop()
        attempts = [0]
        def response(config, payload):
            if config[1] == "old-test-only":
                loop.call_soon_threadsafe(entered.set)
                assert release.wait(2)
                return None
            attempts[0] += 1
            if attempts[0] <= 2:
                raise TimeoutError("private-pane-and-secret")
            return {}
        monkeypatch.setattr(t, "_http", response)
        old = asyncio.create_task(t._request({"op": "release", "consumer": "old"}))
        try:
            await asyncio.wait_for(entered.wait(), 1)
            t.configure("127.0.0.1:23456", "new-test-only")
            for _ in range(2):
                assert await t._request({"op": "release", "consumer": "new"}) is None
            release.set()
            assert await old is None
            assert await t._request({"op": "release", "consumer": "new"}) == {}
        finally:
            release.set()
            await asyncio.gather(old, return_exceptions=True)
    asyncio.run(run())


@pytest.mark.parametrize("failure,code", [(400, "http_400"), (503, "http_503"),
    ("timeout", "http_timeout"), ("connection", "http_connection"), ("protocol", "http_protocol"),
    ("url-timeout", "http_timeout")])
def test_bridge_diagnostics_preserve_safe_failure_reason_and_recovery(monkeypatch, caplog, failure, code):
    import http.client
    import urllib.error
    from app import diag
    t = bridge()
    t.configure("127.0.0.1:12345", "private-secret")
    monkeypatch.setattr(state.tmux, "_pane_target", lambda name: "%8")
    entries = []
    monkeypatch.setattr(diag, "registrar", lambda *args, **fields: entries.append((args, fields)))
    failing = [True]
    def response(config, payload):
        if payload["op"] == "acquire":
            return {}
        if not failing[0]:
            if payload["op"] == "capture":
                return dict(binding=payload["binding"], started=payload["started"], text="", analysis=analysis())
            return {}
        if isinstance(failure, int):
            raise urllib.error.HTTPError("http://private-url", failure, "private-message", None, None)
        if failure == "timeout":
            raise TimeoutError("private-message")
        if failure == "url-timeout":
            raise urllib.error.URLError(TimeoutError("private-message"))
        if failure == "connection":
            raise urllib.error.URLError(ConnectionRefusedError("private-message"))
        raise http.client.BadStatusLine("private-message")
    monkeypatch.setattr(t, "_http", response)
    async def run():
        source = t.lease("reason-recovery", "claude", lambda: "b")
        await source.start()
        for _ in range(2):
            assert await t._request({"op": "release", "consumer": "private-consumer"}) is None
        assert entries == [(("terminal_observer.fallback", "aviso"), {"codigo": code})]
        failing[0] = False
        assert await t._request({"op": "acquire"}) == {}
        assert entries == [(("terminal_observer.fallback", "aviso"), {"codigo": code})]
        with t.use(source):
            assert await t.capture("reason-recovery", 1.0) is not None
        assert entries[-1] == (("terminal_observer.recovered", "ok"), {"codigo": "rust_available"})
        failing[0] = True
        assert await t._request({"op": "release", "consumer": "private-consumer"}) is None
        assert entries[-1] == (("terminal_observer.fallback", "aviso"), {"codigo": code})
        await source.close()
    asyncio.run(run())
    assert caplog.text.count(code) == 2
    assert "private-" not in caplog.text
    assert "private-" not in repr(entries)


def test_malformed_frame_cannot_report_recovery_before_validated_capture(monkeypatch):
    from app import diag
    t = bridge()
    t.configure("127.0.0.1:12345", "test-only")
    monkeypatch.setattr(state.tmux, "_pane_target", lambda name: "%8")
    entries = []
    monkeypatch.setattr(diag, "registrar", lambda *args, **fields: entries.append((args, fields)))
    valid = [False]
    def response(config, payload):
        if payload["op"] != "capture":
            return {}
        if not valid[0]:
            return {}
        return dict(binding=payload["binding"], started=payload["started"], text="", analysis=analysis())
    monkeypatch.setattr(t, "_http", response)
    async def run():
        async with t.lease("diagnostic-validation", "claude", lambda: "b"):
            assert await t.capture("diagnostic-validation", 1.0) is None
            assert await t.capture("diagnostic-validation", 2.0) is None
            assert await t._request({"op": "acquire"}) == {}
            assert entries == [(("terminal_observer.fallback", "aviso"), {"codigo": "invalid_frame"})]
            valid[0] = True
            assert await t.capture("diagnostic-validation", 3.0) is not None
            assert entries[-1] == (("terminal_observer.recovered", "ok"), {"codigo": "rust_available"})
    asyncio.run(run())


@pytest.mark.parametrize("operation", ["payload", "start", "watch"])
@pytest.mark.parametrize("error", [OSError, TimeoutError, RuntimeError])
def test_old_resolver_error_does_not_charge_new_generation(monkeypatch, operation, error):
    t = bridge()
    t.configure("127.0.0.1:12345", "old-test-only")
    monkeypatch.setattr(t, "TIMEOUT", 1)
    monkeypatch.setattr(t, "HEARTBEAT", 0.01)
    async def run():
        entered, release = asyncio.Event(), asyncio.Event()
        async def io(fn, *args):
            entered.set()
            await release.wait()
            raise error("private-old-resolver")
        monkeypatch.setattr(t, "_io", io)
        source = t.lease("old-resolver", "claude", lambda: "binding")
        source.binding, source.open = "binding", True
        t._bindings[source.name] = ("claude", "binding")
        if operation == "payload":
            job = asyncio.create_task(source.payload("capture", 1.0))
        elif operation == "start":
            job = asyncio.create_task(source.start())
        else:
            job = asyncio.create_task(source.watch())
        await entered.wait()
        t.configure("127.0.0.1:23456", "new-test-only")
        t._failure("new-failure-one")
        t._failure("new-failure-two")
        release.set()
        if operation == "watch":
            await asyncio.sleep(0)
            job.cancel()
            await asyncio.gather(job, return_exceptions=True)
        else:
            await asyncio.gather(job, return_exceptions=True)
        assert t._failures == 2
        assert t._available()
    asyncio.run(run())

