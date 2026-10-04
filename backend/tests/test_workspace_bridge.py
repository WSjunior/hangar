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
                error = {"status": 503, "code": "workspace_unavailable", "detail": "git não encontrado"}
            if payload["op"] in ("read_file", "citation_cwds", "git_summary"):
                error = {"status": 503, "code": "workspace_busy", "detail": "vagas cheias"}
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


def test_bridge_off_runs_python():
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


def test_bridge_failure_raises_instead_of_running_python(bridge_server, tmp_path, monkeypatch):
    calls = []
    @workspace_bridge.delegate("commit", GitError, mutation=True)
    def original(cwd):
        calls.append(cwd)
        return {"ok": True}
    with pytest.raises(GitError) as error:
        original("pasta")
    assert error.value.status == 503
    assert error.value.code == "workspace_unavailable"
    assert "git não encontrado" in error.value.detail
    assert calls == []
    assert len(bridge_server) == 1
    # Pela rota: o Rust sem vaga vira 503 com o código, sem o Python ler o arquivo.
    from fastapi.testclient import TestClient
    from app import api
    from app.config import settings
    (tmp_path / "leia.txt").write_text("texto\n")
    monkeypatch.setattr(api, "_session_cwd", lambda name: str(tmp_path))
    monkeypatch.setattr(settings, "auth_token", "secret")
    r = TestClient(api.app).get("/api/sessions/s/files/read", params={"path": "leia.txt"},
                                headers={"Authorization": "Bearer secret"})
    assert r.status_code == 503
    assert r.json()["detail"]["code"] == "workspace_busy"
    assert len(bridge_server) == 2


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


def test_rows_in_memory_go_as_text_and_too_large_raises_without_sending(bridge_server):
    calls = []
    @workspace_bridge.delegate("citation_cwds", GitError, prepare=workspace_bridge.text_rows)
    def original(jsonl, needles, *, rows=None):
        calls.append(len(rows))
        return {}
    with pytest.raises(GitError):
        original("conversa.jsonl", ["a.txt"], rows=[b'{"cwd":"/x","t":"a\xc3\xa7\xc3\xa3o.txt"}\n'])
    assert bridge_server[0]["args"]["rows"] == ['{"cwd":"/x","t":"ação.txt"}\n']
    huge = [b"x" * 1024 * 1024] * 5
    with pytest.raises(GitError) as error:
        original("conversa.jsonl", ["a.txt"], rows=huge)
    assert error.value.status == 413
    assert error.value.code == "workspace_request_too_large"
    assert calls == []
    assert len(bridge_server) == 1


def test_delegated_operations_send_only_arguments_the_rust_core_accepts():
    # Assinatura do Python que ganhou argumento sem o Rust acompanhar quebra a operação calada.
    import inspect, re
    from pathlib import Path
    from app import git_ops, transcript, api
    root = Path(__file__).resolve().parents[2]
    enum = (root / "crates/hangar-workspace/src/lib.rs").read_text(encoding="utf-8")
    enum = enum[enum.index("pub enum Operation"):]
    enum = enum[:enum.index("\n}\n")]
    snake = lambda n: re.sub(r"(?<!^)(?=[A-Z])", "_", n).lower()
    fields = {snake(m.group(1)): set(re.findall(r"(\w+):", m.group(2)))
              for m in re.finditer(r"\n    (\w+) \{(.*?)\n    \}", enum, re.S)}
    checks = {name: getattr(git_ops, name) for name in (
        "create_worktree", "remove_worktree", "switch_branch", "commit", "push", "git_action",
        "list_branches", "git_log", "changed_files", "file_diff", "path_diff")}
    checks.update(citation_cwds=transcript.citation_cwds, cited_elsewhere=transcript.cited_elsewhere,
                  find_elsewhere=api._cited_elsewhere)
    python_only: dict[str, set[str]] = {}
    for op, fn in checks.items():
        params = set(inspect.signature(inspect.unwrap(fn)).parameters) - python_only.get(op, set())
        assert params == fields[op], (op, params ^ fields[op])


def test_bridge_failure_returns_the_empty_value_of_functions_that_never_raise(bridge_server):
    # A listagem de sessões chama git_summary sem try: falha da ponte devolve None, nunca levanta.
    from app import git_ops
    assert git_ops.git_summary("pasta") is None
    assert bridge_server[-1]["op"] == "git_summary"


def test_git_error_escaping_a_route_keeps_status_and_code(bridge_server, tmp_path, monkeypatch):
    from types import SimpleNamespace
    from fastapi.testclient import TestClient
    from app import api
    from app.config import settings
    info = SimpleNamespace(cwd=str(tmp_path), jsonl=str(tmp_path / "c.jsonl"))
    monkeypatch.setattr(api, "_cached_info_sync", lambda name: info)
    monkeypatch.setattr(api, "_conversation_rows", lambda info: None)
    monkeypatch.setattr(settings, "auth_token", "secret")
    r = TestClient(api.app, raise_server_exceptions=False).post(
        "/api/sessions/s/files/resolver", json={"caminhos": ["a.txt"]}, headers={"Authorization": "Bearer secret"})
    assert r.status_code == 503
    assert r.json()["detail"]["code"] == "workspace_busy"
