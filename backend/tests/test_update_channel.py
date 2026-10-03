"""Canal do Atualizar: sem tocar no ambiente ou origin reais."""
from fastapi.testclient import TestClient

from app.api import app
from app.config import settings

TOKEN = "test-update-channel"
AUTH = {"Authorization": f"Bearer {TOKEN}"}


def test_update_channel_requires_owner_token(monkeypatch):
    monkeypatch.setattr(settings, "auth_token", TOKEN)
    client = TestClient(app)
    assert client.get("/api/update-channel").status_code == 401
    assert client.put("/api/update-channel", json={"branch": ""}).status_code == 401

import subprocess
from pathlib import Path
from unittest.mock import patch

import pytest


def git(repo, *args):
    return subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True,
                          text=True, encoding="utf-8", errors="replace").stdout.strip()


@pytest.fixture
def isolated(tmp_path, monkeypatch):
    from app import atualizar, update_channel
    repo = tmp_path / "repo"
    repo.mkdir()
    git(repo, "init", "-b", "main")
    git(repo, "-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "--allow-empty", "-m", "fixture")
    origin = tmp_path / "origin.git"
    git(repo, "clone", "--bare", str(repo), str(origin))
    git(repo, "remote", "add", "origin", str(origin))
    git(origin, "branch", "test/channel")
    env = repo / "backend" / ".env"
    env.parent.mkdir()
    env.write_bytes(b"# config\r\nCP_AUTH_TOKEN=private\r\nEXTRA=value")
    monkeypatch.setattr(update_channel, "ENV_FILE", env)
    monkeypatch.setattr(atualizar, "REPO", repo)
    monkeypatch.setattr(atualizar, "_base", lambda: tmp_path / "update")
    monkeypatch.setattr(settings, "auth_token", TOKEN)
    monkeypatch.setattr(settings, "update_branch", "")
    monkeypatch.setattr(settings, "update_last_branch", "")
    monkeypatch.setenv("CP_UPDATE_BRANCH", "")
    monkeypatch.setenv("CP_UPDATE_LAST_BRANCH", "")
    monkeypatch.setattr(update_channel.diag, "registrar", lambda *a, **kw: None)
    return TestClient(app), env, repo


def test_update_channel_applies_and_remembers_after_disable(isolated):
    client, env, _ = isolated
    assert client.get("/api/update-channel", headers=AUTH).json() == {
        "branch": "", "checkout_branch": "main", "last_branch": ""}
    response = client.put("/api/update-channel", headers=AUTH, json={"branch": " test/channel "})
    assert response.status_code == 200
    assert response.json() == {"branch": "test/channel", "checkout_branch": "main", "last_branch": "test/channel"}
    assert env.read_bytes() == (b"# config\r\nCP_AUTH_TOKEN=private\r\nEXTRA=value\r\n"
                                b"CP_UPDATE_BRANCH=test/channel\r\nCP_UPDATE_LAST_BRANCH=test/channel\r\n")
    from app import atualizar
    assert atualizar.alvo() == "test/channel"
    from app.config import Settings
    assert Settings(_env_file=env).update_branch == "test/channel"
    assert client.put("/api/update-channel", headers=AUTH, json={"branch": ""}).json()["last_branch"] == "test/channel"
    assert atualizar.alvo() == "main"
    assert Settings(_env_file=env).update_last_branch == "test/channel"
    child = subprocess.run(["python", "-c", "import os; print(os.environ['CP_UPDATE_BRANCH'])"],
                           capture_output=True, text=True, check=True)
    assert child.stdout == "\n"


@pytest.mark.parametrize("branch", ["../bad", "-option", "x\nCP_AUTH_TOKEN=stolen", "a b", "x" * 101, None, 1])
def test_update_channel_invalid_preserves_everything(isolated, branch):
    client, env, _ = isolated
    before = env.read_bytes()
    assert client.put("/api/update-channel", headers=AUTH, json={"branch": branch}).status_code in (400, 422)
    assert env.read_bytes() == before
    assert settings.update_branch == ""


def test_update_channel_missing_branch_and_origin_fail_without_write(isolated):
    client, env, repo = isolated
    before = env.read_bytes()
    assert client.put("/api/update-channel", headers=AUTH, json={"branch": "missing"}).status_code == 400
    git(repo, "remote", "set-url", "origin", str(repo / "absent"))
    assert client.put("/api/update-channel", headers=AUTH, json={"branch": "test/channel"}).status_code == 503
    assert client.put("/api/update-channel", headers=AUTH, json={"branch": ""}).status_code == 503
    assert env.read_bytes() == before
    assert settings.update_branch == ""


def test_update_channel_atomic_failure_preserves_memory_and_file(isolated):
    client, env, _ = isolated
    before = env.read_bytes()
    with patch("app.update_channel.atomico.substituir", side_effect=PermissionError("busy")):
        assert client.put("/api/update-channel", headers=AUTH, json={"branch": "test/channel"}).status_code == 500
    assert env.read_bytes() == before
    assert settings.update_branch == ""
    assert not list(env.parent.glob(".env.*.tmp"))


def test_update_channel_export_duplicates_and_empty_file(isolated):
    client, env, _ = isolated
    env.write_bytes(b"export CP_UPDATE_BRANCH = 'old'\nCP_UPDATE_BRANCH=duplicate\nOTHER=x\n")
    assert client.put("/api/update-channel", headers=AUTH, json={"branch": "test/channel"}).status_code == 200
    assert env.read_text().count("CP_UPDATE_BRANCH=") == 1
    assert "OTHER=x\n" in env.read_text()
    env.unlink()
    assert client.put("/api/update-channel", headers=AUTH, json={"branch": ""}).status_code == 200
    assert b"CP_UPDATE_BRANCH=\n" in env.read_bytes()


def test_update_channel_refuses_during_update(isolated):
    client, env, _ = isolated
    before = env.read_bytes()
    with patch("app.update_channel.atualizar.estado_para_tela", return_value={"fase": "rodando"}):
        assert client.put("/api/update-channel", headers=AUTH, json={"branch": "test/channel"}).status_code == 409
    assert env.read_bytes() == before


def test_update_channel_owner_guard_rejects_both_guest_types(isolated):
    client, env, _ = isolated
    before = env.read_bytes()
    with patch("app.update_channel.guest_of", return_value=object()):
        assert client.get("/api/update-channel", headers=AUTH).status_code == 403
        assert client.put("/api/update-channel", headers=AUTH, json={"branch": ""}).status_code == 403
    from app import guest_users, update_channel
    from fastapi import HTTPException
    token = guest_users.current.set(object())
    try:
        with pytest.raises(HTTPException) as exc:
            update_channel.require_owner(None)
        assert exc.value.status_code == 403
    finally:
        guest_users.current.reset(token)
    assert env.read_bytes() == before


def test_update_channel_preserves_multiline_variable(isolated):
    client, env, _ = isolated
    other = b'OTHER="first\r\nCP_UPDATE_BRANCH=inside-value\r\nlast"\r\n'
    env.write_bytes(other)
    assert client.put("/api/update-channel", headers=AUTH, json={"branch": "test/channel"}).status_code == 200
    assert env.read_bytes().startswith(other)
    from dotenv import dotenv_values
    assert dotenv_values(env)["CP_UPDATE_BRANCH"] == "test/channel"


def test_update_channel_auto_start_rechecks_target_under_lock(isolated):
    from app import atualizar
    client, _, _ = isolated
    assert client.put("/api/update-channel", headers=AUTH, json={"branch": "test/channel"}).status_code == 200
    with patch("app.atualizar.subprocess.Popen", side_effect=AssertionError("must not launch")):
        result = atualizar.iniciar(expected_branch="main")
    assert result == {"ok": False, "erro": "canal_mudou"}
    assert not (atualizar._base() / "rodando.lock").exists()


def test_update_channel_auto_loop_cannot_use_stale_approval(isolated, monkeypatch):
    import asyncio
    from app import api
    client, _, _ = isolated
    sleeps = 0

    async def sleep(_):
        nonlocal sleeps
        sleeps += 1
        if sleeps > 1:
            raise asyncio.CancelledError

    async def sessions():
        settings.update_branch = "test/channel"
        return []

    monkeypatch.setattr(api.asyncio, "sleep", sleep)
    monkeypatch.setattr(api, "automations_enabled", lambda: True)
    monkeypatch.setattr(api, "_auto_update_motivo", lambda: None)
    monkeypatch.setattr(api.registry, "list_with_state", sessions)
    monkeypatch.setattr(api.atualizar, "_git", lambda *a, **kw: subprocess.CompletedProcess([], 0, "", ""))
    with patch("app.atualizar.subprocess.Popen") as launch:
        with pytest.raises(asyncio.CancelledError):
            asyncio.run(api._auto_update_loop())
        assert launch.call_count == 0
