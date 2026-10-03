"""Escopos de custos para o hangar-server (contrato versão 5)."""
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from app.api import app
from app import costs_cache, costs_sources, internal_api
from app.config import ConfigDirInfo


@pytest.fixture
def client():
    internal_api.set_secret("s3")
    yield TestClient(app, client=("127.0.0.1", 5000))
    internal_api.set_secret(None)


def _fake_roots(tmp_path, monkeypatch, *, omp_same_as_pi=False):
    cfg = tmp_path / "cfg"
    (cfg / "projects").mkdir(parents=True)
    (cfg / ".claude.json").write_text(
        '{"oauthAccount": {"accountUuid": "u1", "emailAddress": "a@b"}}', encoding="utf-8")
    monkeypatch.setattr(costs_sources, "list_config_dirs",
                        lambda: [ConfigDirInfo(path=str(cfg), label="padrão", active=True)])
    home = tmp_path / "codex"
    (home / "sessions").mkdir(parents=True)
    from app import codex_contas
    monkeypatch.setattr(costs_sources, "_contas_codex",
                        lambda: [codex_contas.Account("default", home, True)])
    pi = tmp_path / "pi"
    pi.mkdir()
    monkeypatch.setattr(costs_sources, "raiz_pi", lambda: pi)
    monkeypatch.setattr(costs_sources, "raiz_omp", lambda: pi if omp_same_as_pi else tmp_path / "omp-sem")
    monkeypatch.setattr(costs_sources, "raiz_kimi", lambda: tmp_path / "kimi-sem")
    return cfg, home, pi


def test_scopes_carry_accounts_roots_and_repo(client, tmp_path, monkeypatch):
    cfg, home, pi = _fake_roots(tmp_path, monkeypatch)
    response = client.get("/internal/costs/scopes", headers={"x-hangar-internal": "s3"})
    assert response.status_code == 200
    body = response.json()
    assert body["claude"] == [{"root": str(cfg / "projects"), "account": "anthropic:u1", "label": "a@b"}]
    identity = f"codex:{home.resolve()}"
    assert body["codex"] == [{"home": str(home.resolve()), "account": identity, "label": "Codex · default"}]
    assert body["pi"] == [{"root": str(pi), "source": "pi"}]
    assert body["kimi"] is None
    assert Path(body["repo"], "skills").is_dir()


def test_omp_on_the_pi_root_is_left_out(client, tmp_path, monkeypatch):
    _fake_roots(tmp_path, monkeypatch, omp_same_as_pi=True)
    body = client.get("/internal/costs/scopes", headers={"x-hangar-internal": "s3"}).json()
    assert [scope["source"] for scope in body["pi"]] == ["pi"]


def test_scopes_refuse_without_secret(client):
    assert client.get("/internal/costs/scopes").status_code == 404


def test_boot_warmup_skips_while_rust_serves(monkeypatch):
    calls = []
    monkeypatch.setattr(costs_sources, "aquecer_em_background", lambda: calls.append(1))
    costs_sources.set_served_by_rust(True)
    try:
        costs_sources._boot_warmup()
        assert calls == []
    finally:
        costs_sources.set_served_by_rust(False)
    costs_sources._boot_warmup()
    assert calls == [1]


def test_request_reaching_python_still_warms_on_demand(monkeypatch):
    """Pedido repassado ao Python dispara a varredura dele e responde com aquecimento."""
    calls = []
    monkeypatch.setattr(costs_sources, "aquecer_em_background", lambda: calls.append(1))
    monkeypatch.setattr(costs_sources._aquecido, "is_set", lambda: False)
    monkeypatch.setattr(costs_cache, "progresso_total", lambda: (0, 0))
    costs_sources.set_served_by_rust(True)
    try:
        with pytest.raises(costs_sources.Aquecendo):
            costs_sources.preparar()
    finally:
        costs_sources.set_served_by_rust(False)
    assert calls == [1]
