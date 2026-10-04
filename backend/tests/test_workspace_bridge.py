"""A reserva de leitura não pode repetir uma alteração já enviada."""
import json
import socket
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import pytest

from app import workspace_bridge
from app.git_ops import GitError


@pytest.fixture
def bridge_server():
    requests = []
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            payload = json.loads(self.rfile.read(int(self.headers["content-length"])))
            requests.append(payload)
            if payload["op"] == "push":
                self.close_connection = True
                return
            error = {"status": 409, "detail": "recusado"}
            if payload["op"] == "commit":
                error = {"status": 503, "code": "workspace_unavailable", "detail": "vagas cheias"}
            body = json.dumps({"ok": False, "error": error}).encode()
            self.send_response(200)
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    workspace_bridge.configure(f"127.0.0.1:{server.server_port}", "segredo-sintético")
    yield requests
    workspace_bridge.configure(None, None)
    server.shutdown()
    server.server_close()
    thread.join()


def test_domain_refusal_is_not_replaced_by_python(bridge_server):
    calls = []
    @workspace_bridge.delegate("list_branches", GitError)
    def original(cwd):
        calls.append(cwd)
    with pytest.raises(GitError) as error:
        original("pasta")
    assert error.value.status == 409
    assert calls == []
    assert len(bridge_server) == 1


def test_lost_mutation_reply_never_repeats_the_operation(bridge_server):
    calls = []
    @workspace_bridge.delegate("push", GitError, mutation=True)
    def original(cwd):
        calls.append(cwd)
    with pytest.raises(GitError) as error:
        original("pasta")
    assert error.value.status == 503
    assert calls == []
    assert len(bridge_server) == 1


def test_disabled_bridge_uses_python_once():
    workspace_bridge.configure(None, None)
    calls = []
    @workspace_bridge.delegate("list_branches", GitError)
    def original(cwd):
        calls.append(cwd)
        return {"current": "main"}
    assert original("pasta") == {"current": "main"}
    assert calls == ["pasta"]


@pytest.mark.parametrize("address", ["example.invalid:8765", "0.0.0.0:8765", "127.0.0.1:0", "127.0.0.1:8765/fora"])
def test_only_literal_loopback_is_accepted(address):
    with pytest.raises(ValueError):
        workspace_bridge.configure(address, "segredo-sintético")


def test_operation_rust_never_ran_goes_to_python_once(bridge_server):
    calls = []
    @workspace_bridge.delegate("commit", GitError, mutation=True)
    def original(cwd):
        calls.append(cwd)
        return {"ok": True}
    assert original("pasta") == {"ok": True}
    assert calls == ["pasta"]
    assert len(bridge_server) == 1


def test_refused_connection_runs_mutation_in_python():
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    workspace_bridge.configure(f"127.0.0.1:{port}", "segredo-sintético")
    calls = []
    @workspace_bridge.delegate("push", GitError, mutation=True)
    def original(cwd):
        calls.append(cwd)
        return {"ok": True}
    try:
        assert original("pasta") == {"ok": True}
    finally:
        workspace_bridge.configure(None, None)
    assert calls == ["pasta"]


def test_request_handed_off_by_rust_is_served_by_python_code(bridge_server, tmp_path, monkeypatch):
    from fastapi.testclient import TestClient
    from app import api
    from app.config import settings
    (tmp_path / "leia.txt").write_text("texto\n")
    monkeypatch.setattr(api, "_session_cwd", lambda name: str(tmp_path))
    monkeypatch.setattr(settings, "auth_token", "secret")
    client = TestClient(api.app)
    auth = {"Authorization": "Bearer secret"}
    assert client.get("/api/sessions/s/files/read", params={"path": "leia.txt"}, headers=auth).status_code == 409
    sent = len(bridge_server)
    r = client.get("/api/sessions/s/files/read", params={"path": "leia.txt"},
                   headers={**auth, "x-hangar-workspace-fallback": "indisponivel"})
    assert r.status_code == 200 and r.json()["text"] == "texto\n"
    assert len(bridge_server) == sent
