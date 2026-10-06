"""Tela de migração: o Python sozinho na porta diz isso, com o motivo, e mede o próprio processo."""
import os

from fastapi.testclient import TestClient

from app import migration_status
from app.api import app
from app.config import settings

TOKEN = "test-migration-status"


def test_python_alone_answers_with_reason_and_own_usage(monkeypatch, tmp_path):
    monkeypatch.setattr(settings, "auth_token", TOKEN)
    monkeypatch.setattr(settings, "reload", False)
    monkeypatch.setattr(settings, "rust_server", False)
    monkeypatch.setattr(migration_status, "_reason", None)
    monkeypatch.setattr(migration_status, "_canos", (0.0, []))
    monkeypatch.setenv("HOME", str(tmp_path))
    client = TestClient(app)
    assert client.get("/api/migration/status").status_code == 401
    body = client.get("/api/migration/status", headers={"Authorization": f"Bearer {TOKEN}"}).json()
    assert body["served_by"] == "python" and body["areas"] is None
    facts = body["python"]
    assert facts["reason"] == "desligado"
    assert facts["processes"]["python"]["pid"] == os.getpid() and facts["processes"]["python"]["rss_bytes"] > 0
    assert facts["processes"]["rust"] is None and facts["processes"]["cano"]["count"] == 0


def test_reason_recorded_by_the_supervisor_wins(monkeypatch):
    monkeypatch.setattr(settings, "reload", True)
    monkeypatch.setattr(migration_status, "_reason", "quedas")
    assert migration_status.reason() == "quedas"
    monkeypatch.setattr(migration_status, "_reason", None)
    assert migration_status.reason() == "reload"
