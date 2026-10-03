import asyncio

import pytest

from app import runtime_policy


def test_policy_failure_is_visible_and_redacted(monkeypatch):
    from app import diag
    records = []
    monkeypatch.setattr(diag, "registrar", lambda *args, **kwargs: records.append((args, kwargs)))
    async def scenario():
        result = await runtime_policy.execute("unsupported", {"text":"private-input", "token":"private-secret"}, {"provider":"claude"})
        assert result["ok"] is False
        assert result["error_type"] == "ValueError"
    asyncio.run(scenario())
    assert "private-input" not in repr(records)
    assert "private-secret" not in repr(records)


def test_metadata_patch_has_a_strict_catalog(monkeypatch):
    from app.adapters.codex import sessions
    monkeypatch.setattr(sessions, "update", lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("escrita indevida")))
    with pytest.raises(ValueError):
        runtime_policy.run("session.patch_meta", {"cano":{"token":"changed"}}, {"provider":"codex", "name":"session"})


def test_prepare_prompt_does_not_open_client_or_queue(monkeypatch):
    from app.adapters.claude_headless import adapter
    monkeypatch.setattr(adapter, "_blocos_do_prompt", lambda text: ([{"type":"text", "text":text}], []))
    result = runtime_policy.run("prepare_prompt", {"text":"Olá"}, {"provider":"claude"})
    assert result["content"] == [{"type":"text", "text":"Olá"}]


def test_native_message_has_journal_before_uds(monkeypatch):
    from app import api, uds_messaging
    validated = []
    monkeypatch.setattr(uds_messaging, "socket_da_sessao", lambda *args: "fake.sock")
    monkeypatch.setattr(api, "_classe_modo", lambda *args: "prompting")
    def send(*args, **kwargs):
        assert validated == [True]
        assert kwargs["msg_id"]
        raise OSError("write sem prova")
    monkeypatch.setattr(uds_messaging, "enviar", send)
    result = runtime_policy.native_message({"text":"[de: source] Olá"}, {"key":"key", "name":"session",
        "session_id":"sid", "operation_id":"op", "validate":lambda: validated.append(True)})
    assert result["outcome"] == "unknown"


def test_quota_status_keeps_existing_window_format(tmp_path, monkeypatch):
    from types import SimpleNamespace
    from app import cotas
    account = SimpleNamespace(provedor="claude", id="claude:" + str(tmp_path),
        model_dump=lambda: {"janelas":[{"rotulo":"5h", "pct":42, "reset_ts":None, "por_modelo":False}]})
    monkeypatch.setattr(cotas, "listar_cotas", lambda: [account])
    data = runtime_policy.run("format_status", {}, {"provider":"claude", "config_dir":str(tmp_path)})
    assert "⚡5h:42%" in data["status_line"]


def test_skill_catalog_is_data_only():
    catalog = {"data":[{"skills":[{"name":"skill", "path":"/fake/skill", "enabled":True}]}]}
    data = runtime_policy.run("skill_catalog", {"catalog":catalog, "name":"skill"}, {"provider":"codex"})
    assert data["skill"]["native_name"] == "skill"
