"""Regressões da escolha e da memória do provedor na abertura."""
import asyncio
from unittest.mock import AsyncMock

import pytest

from app import api, runtime_config
from app.registry import SessionInfo
from app.session_defaults import choose_provider


@pytest.mark.parametrize("remembered,connected,disconnected,expected", [
    (None, {"codex"}, {"claude"}, "codex"),
    ("claude", {"codex"}, {"claude"}, "codex"),
    ("codex", {"claude", "codex"}, set(), "codex"),
    ("pi", {"claude", "codex"}, set(), "pi"),
    ("kimi", {"codex"}, {"claude"}, "codex"),
    ("codex", set(), {"claude", "codex"}, "codex"),
])
def test_provider_choice(remembered, connected, disconnected, expected):
    probes = {provider: {"disponivel": provider != "kimi"}
              for provider in ("claude", "codex", "pi", "kimi")}
    assert choose_provider(probes, remembered, connected, disconnected) == expected


@pytest.fixture
def creation(monkeypatch, tmp_path):
    monkeypatch.setattr(runtime_config, "_backend_config_base", lambda: tmp_path)
    create = AsyncMock(return_value=SessionInfo(name="default-provider", cwd="/repo", provider="codex"))
    monkeypatch.setattr(api, "_criar_sessao", create)
    monkeypatch.setattr(api, "_session_provider_catalog", AsyncMock(return_value={
        "claude": {"disponivel": True, "default": False},
        "codex": {"disponivel": True, "default": True},
    }))
    return create


def test_omitted_provider_uses_server_choice_and_remembers_success(creation):
    result = asyncio.run(api.create_session(api.CreateBody(
        name="default-provider", cwd="/repo", remember_provider=True)))
    assert creation.call_args.args[0].provider == "codex"
    assert result.provider == "codex"
    assert runtime_config.get("last_session_provider") == "codex"


def test_failed_creation_preserves_preference(creation):
    runtime_config.aplicar({"last_session_provider": "pi"})
    creation.side_effect = ValueError("creation failed")
    with pytest.raises(ValueError, match="creation failed"):
        asyncio.run(api.create_session(api.CreateBody(
            name="default-provider", cwd="/repo", provider="codex", remember_provider=True)))
    assert runtime_config.get("last_session_provider") == "pi"


def test_automated_creation_preserves_preference(creation):
    runtime_config.aplicar({"last_session_provider": "pi"})
    asyncio.run(api.create_session(api.CreateBody(name="default-provider", cwd="/repo", provider="codex")))
    assert runtime_config.get("last_session_provider") == "pi"


def test_explicit_provider_wins(creation):
    creation.return_value = SessionInfo(name="default-provider", cwd="/repo", provider="pi")
    asyncio.run(api.create_session(api.CreateBody(name="default-provider", cwd="/repo", provider="pi")))
    assert creation.call_args.args[0].provider == "pi"


def test_preference_write_failure_does_not_hide_created_session(creation, monkeypatch):
    def fail(_changes):
        raise OSError("disk full")

    monkeypatch.setattr(runtime_config, "aplicar", fail)
    result = asyncio.run(api.create_session(api.CreateBody(
        name="default-provider", cwd="/repo", provider="codex", remember_provider=True)))
    assert result.name == "default-provider"
    assert "disk full" in result.avisos[0]


def test_codex_account_error_counts_as_unavailable(monkeypatch, tmp_path):
    from app import codex_contas

    broken = codex_contas.Account("broken", tmp_path / ".codex-broken", False)
    work = codex_contas.Account("work", tmp_path / ".codex-work", False)

    class Service:
        async def read_auth_rapido(self, account):
            if account is broken:
                raise codex_contas.AccountError(409, "codex_account_prepare_required", {})
            return {"status": "connected"}

    monkeypatch.setattr(api, "_codex_service", lambda: Service())
    monkeypatch.setattr(codex_contas, "list_visible_accounts", lambda: [broken, work])
    assert asyncio.run(api._connected_codex_accounts()) == [work]
