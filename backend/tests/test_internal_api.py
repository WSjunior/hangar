# backend/tests/test_internal_api.py
import asyncio
import time
from types import SimpleNamespace
from unittest.mock import AsyncMock, patch

import pytest
from fastapi.testclient import TestClient

import app.api as api_mod
from app import internal_api, pqueue
from app.models import SessionInfo

SECRET = "ab" * 32
ROUTE = "/internal/sessions/s1/info"


@pytest.fixture(autouse=True)
def _env(monkeypatch, tmp_path):
    internal_api.set_secret(SECRET)
    monkeypatch.setattr(pqueue, "_queue_dir", lambda: tmp_path)
    yield
    internal_api.set_secret(None)


def _client(ip="127.0.0.1"):
    return TestClient(api_mod.app, client=(ip, 50000))


def _info(**kw):
    return SessionInfo(**{"name": "s1", "cwd": "/p", "jsonl": "/p/abc-123.jsonl", "provider": "claude", **kw})


def _get(client, headers=None, info=None):
    with patch("app.api._cached_info", AsyncMock(return_value=info or _info())):
        return client.get(ROUTE, headers=headers if headers is not None else {"X-Hangar-Internal": SECRET})


def test_info_has_what_hangar_server_needs(tmp_path):
    with patch("app.adapters.chave_de", lambda name, provider: "claude-headless"):
        r = _get(_client())
    assert r.status_code == 200
    assert r.json() == {"provider": "claude-headless", "jsonl": "/p/abc-123.jsonl", "session_key": "abc-123",
                        "history": {"queue": str(tmp_path / "s1.jsonl")}}


def test_codex_session_key_is_the_rollout_id():
    info = _info(provider="codex",
                 jsonl="/c/rollout-2026-09-02T14-00-00-019f0000-0000-7000-8000-000000000001.jsonl")
    r = _get(_client(), info=info)
    assert r.json()["provider"] == "codex"
    assert r.json()["session_key"] == "019f0000-0000-7000-8000-000000000001"


def test_session_without_transcript_yet():
    r = _get(_client(), info=_info(jsonl=None))
    assert r.status_code == 200
    assert (r.json()["jsonl"], r.json()["session_key"]) == (None, "")


def test_unknown_session_404():
    with patch("app.api._cached_info", AsyncMock(return_value=None)):
        r = _client().get(ROUTE, headers={"X-Hangar-Internal": SECRET})
    assert r.status_code == 404


def test_workspace_context_contains_only_registry_metadata(tmp_path):
    with patch("app.api._guardar_snap", return_value=[_info()]), patch("app.fs.allowed_roots", return_value=[tmp_path]):
        response = _client().get("/internal/workspace/context?name=s1", headers={"X-Hangar-Internal": SECRET})
    assert response.status_code == 200
    assert response.json() == {"roots":[str(tmp_path)], "sessions":[{"name":"s1","cwd":"/p"}],
                               "session":{"name":"s1","cwd":"/p","jsonl":"/p/abc-123.jsonl","git_cwd":"/p"}}


def test_worktrees_context_has_folders_inside_roots_and_account_projects(tmp_path):
    inside = tmp_path / "repo"
    inputs = AsyncMock(return_value=([_info(cwd=str(inside), worktree_path=str(inside))], [str(inside)], [tmp_path]))
    with patch("app.api._worktree_inputs", inputs), \
            patch("app.archive._contas", return_value=[(None, "", tmp_path / "projects")]):
        response = _client().get("/internal/worktrees/context", headers={"X-Hangar-Internal": SECRET})
    assert response.status_code == 200
    assert response.json() == {
        "roots": [str(tmp_path)], "cwds": [str(inside)],
        "sessions": [{"name": "s1", "cwd": str(inside), "worktree_path": str(inside), "jsonl": "/p/abc-123.jsonl"}],
        "project_bases": [str(tmp_path / "projects")],
    }


def test_worktrees_context_refuses_before_reading_anything():
    with patch("app.api._worktree_inputs") as inputs:
        response = _client("203.0.113.7").get("/internal/worktrees/context", headers={"X-Hangar-Internal": SECRET})
        assert _client().get("/internal/worktrees/context", headers={"X-Hangar-Internal": "errado"}).status_code == 404
    assert response.status_code == 404
    inputs.assert_not_called()


def test_worktree_inputs_keep_recent_folders_inside_the_roots(tmp_path):
    (tmp_path / "repo").mkdir()
    sessions = [_info(cwd=str(tmp_path / "repo")), _info(name="s2", cwd="/fora")]
    folders = [SimpleNamespace(cwd=str(tmp_path), mtime=0), SimpleNamespace(cwd="/fora-recente", mtime=time.time()),
               SimpleNamespace(cwd=str(tmp_path / "recente"), mtime=time.time())]
    with patch("app.api.registry.list", return_value=sessions), patch("app.api.list_folders", return_value=folders), \
            patch("app.api.allowed_roots", return_value=[tmp_path]):
        _sessions, cwds, roots = asyncio.run(api_mod._worktree_inputs())
    assert cwds == [str(tmp_path / "repo"), str(tmp_path / "recente")] and roots == [tmp_path]


def test_workspace_context_refuses_before_reading_the_registry():
    with patch("app.api._guardar_snap") as snapshot:
        response = _client("203.0.113.7").get("/internal/workspace/context", headers={"X-Hangar-Internal":SECRET})
    assert response.status_code == 404
    snapshot.assert_not_called()


@pytest.mark.parametrize("headers", [{}, {"X-Hangar-Internal": "errado"}, {"X-Hangar-Internal": ""}])
def test_wrong_secret_404(headers):
    assert _get(_client(), headers=headers).status_code == 404


def test_no_secret_404():
    internal_api.set_secret(None)
    assert _get(_client(), headers={"X-Hangar-Internal": ""}).status_code == 404


@pytest.fixture
def events(monkeypatch):
    got = []
    # Só o evento desta rota: o middleware também registra cada 404 como `api.servidor`.
    monkeypatch.setattr(internal_api.diag, "registrar",
                        lambda evento, nivel="ok", **campos: evento == "internal.recusado"
                        and got.append((evento, nivel, campos)))
    return got


def test_refused_hangar_server_goes_to_the_diary_without_the_secret(events):
    assert _get(_client(), headers={"X-Hangar-Internal": "errado"}).status_code == 404
    internal_api.set_secret(None)
    assert _get(_client(), headers={"X-Hangar-Internal": SECRET}).status_code == 404
    assert events == [("internal.recusado", "aviso", {"codigo": "segredo_errado"}),
                      ("internal.recusado", "aviso", {"codigo": "sem_segredo"})]
    assert SECRET not in repr(events)


def test_requests_that_are_not_the_hangar_server_stay_out_of_the_diary(events):
    assert _get(_client(), headers={}).status_code == 404              # sem cabeçalho
    assert _get(_client("10.0.0.5"), headers={"X-Hangar-Internal": "x"}).status_code == 404   # de fora
    assert _get(_client()).status_code == 200                          # aceito
    assert events == []


def test_secret_never_goes_to_environ(monkeypatch):
    monkeypatch.delenv("HANGAR_INTERNAL_SECRET", raising=False)
    internal_api.set_secret(SECRET)
    import os
    assert "HANGAR_INTERNAL_SECRET" not in os.environ
    assert _get(_client()).status_code == 200


def test_info_payload_is_what_the_route_returns(tmp_path):
    with patch("app.adapters.chave_de", lambda name, provider: "claude-headless"):
        assert internal_api.info_payload("s1", "claude", "/p/abc-123.jsonl") == _get(_client()).json()


def test_outside_loopback_404_even_with_secret():
    assert _get(_client("10.0.0.7")).status_code == 404


def test_left_out_of_the_api_schema():
    paths = api_mod.app.openapi()["paths"]
    assert "/api/sessions/{name}/history" in paths
    assert not any(p.startswith("/internal") for p in paths)


def _rust_slot(tmp_path, monkeypatch):
    from app import runtime_coordinator
    from app.runtime_coordinator import Binding, Phase, RuntimeCoordinator
    coordinator = RuntimeCoordinator()
    coordinator.instance = "instance-1"
    slot = coordinator.register(Binding(name="session", key="key", provider="claude", headless=True,
        meta={"key": "key"}, jsonl=str(tmp_path / "chat.jsonl"), projection_dir=tmp_path / "projection",
        state_path=tmp_path / "key.json", lock_path=tmp_path / "key.lock", generation=1))
    slot.lease.close()
    slot.phase = Phase.Rust
    monkeypatch.setattr(runtime_coordinator, "_current", coordinator)
    return coordinator


def _policy(client, kind, phase_id, payload):
    return client.post("/internal/runtime/policy", headers={"X-Hangar-Internal": SECRET,
        "X-Hangar-Runtime-Instance": "instance-1"}, json={"key": "key", "generation": 1,
        "request_id": "op-1", "phase_id": phase_id, "kind": kind, "payload": payload})


def test_pure_policy_runs_without_a_journal_attempt(tmp_path, monkeypatch):
    # A entrada adiada volta pelo drain e prepara o texto de novo: o cálculo não deixa tentativa no
    # diário, e exigir uma recusava a segunda vez e perdia a mensagem.
    coordinator = _rust_slot(tmp_path, monkeypatch)
    client = _client()
    for _ in range(2):
        response = _policy(client, "prepare_prompt", "op-1:prepare_prompt", {"text": "Olá"})
        assert response.status_code == 200 and response.json()["ok"] is True
    assert internal_api._policy_calls == {}
    coordinator.close_python_leases()


def test_native_message_still_needs_its_journal_attempt(tmp_path, monkeypatch):
    coordinator = _rust_slot(tmp_path, monkeypatch)
    client = TestClient(api_mod.app, client=("127.0.0.1", 50000), raise_server_exceptions=False)
    response = _policy(client, "native_message", "op-1:native_message:x", {"text": "[de: a] oi"})
    assert response.status_code == 500
    coordinator.close_python_leases()


def test_rust_diag_route_records_event():
    body = {"evento": "rust.history_failed", "sessao": "s1", "codigo": "history_io", "motivo": "leitura falhou"}
    with patch("app.internal_api.diag.registrar") as registrar:
        ok = _client().post("/internal/diag", json=body, headers={"X-Hangar-Internal": SECRET})
        refused = _client().post("/internal/diag", json=body, headers={"X-Hangar-Internal": "errado"})
        missing = _client().post("/internal/diag", json=body)
        invalid = _client().post("/internal/diag", json={**body, "evento": "runtime.parte_para_python"},
                                 headers={"X-Hangar-Internal": SECRET})
    assert ok.status_code == 200
    assert (refused.status_code, missing.status_code) == (404, 404), "sem o segredo, nem de 127.0.0.1"
    assert invalid.status_code == 400, "o Rust só escreve eventos rust.*"
    rust_calls = [c for c in registrar.call_args_list if c.args[0].startswith("rust.")]
    assert len(rust_calls) == 1
    assert rust_calls[0].args[:2] == ("rust.history_failed", "erro")
    assert rust_calls[0].kwargs == {"sessao": "s1", "codigo": "history_io", "detalhe": "leitura falhou"}


@pytest.mark.parametrize("raw", [
    b"x" * 9000, b"not json", b"[" * 8000, b'{"evento":"rust.a","sessao":"s1","codigo":"c"}',
    b'{"evento":"rust.a","sessao":"s1","codigo":"C D","motivo":"m"}',
    b'{"evento":"rust.a","sessao":"s1","codigo":"c","motivo":1}',
])
def test_rust_diag_refuses_malformed_body(raw):
    with patch("app.internal_api.diag.registrar") as registrar:
        response = _client().post("/internal/diag", content=raw, headers={"X-Hangar-Internal": SECRET})
    assert response.status_code == 400
    assert not [c for c in registrar.call_args_list if c.args[0].startswith("rust.")]
