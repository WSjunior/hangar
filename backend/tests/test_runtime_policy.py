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


@pytest.mark.parametrize("kind", ["prepare_prompt", "format_status", "skill_catalog", "answer_body", "quota",
                                  "session.marker", "diag.error"])
def test_services_that_moved_to_rust_are_gone(kind):
    # O ator Rust roda estes por conta própria; o Python não responde mais.
    for provider in ("claude", "codex"):
        with pytest.raises(ValueError):
            runtime_policy.run(kind, {"text": "Olá", "catalog": {}, "questions": [], "answers": []}, {"provider": provider})


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


def test_quota_windows_drops_per_model_windows(tmp_path, monkeypatch):
    from types import SimpleNamespace
    from app import cotas
    runtime_policy._quota_cache.clear()
    account = SimpleNamespace(provedor="claude", id="claude:" + str(tmp_path), model_dump=lambda: {"janelas": [
        {"rotulo": "5h", "pct": 42, "reset_ts": None, "por_modelo": False},
        {"rotulo": "7d", "pct": 9, "reset_ts": None, "por_modelo": True}]})
    monkeypatch.setattr(cotas, "listar_cotas", lambda: [account])
    assert runtime_policy.quota_windows(str(tmp_path)) == [{"rotulo": "5h", "pct": 42, "reset_ts": None, "por_modelo": False}]
    runtime_policy._quota_cache.clear()


def _codex_patch(tmp_path, monkeypatch, sidecar_thread):
    import json
    from app.adapters.codex import sessions
    writes = []
    state = tmp_path / "state.json"
    # Vista que o Rust grava antes de chamar o serviço (control_view com service_tier no topo).
    state.write_text(json.dumps({"runtime_state":{"view":{"thread_id":"t1", "service_tier":"priority"}}}))
    monkeypatch.setattr(sessions, "load", lambda name: {"key":"k", "thread_id":sidecar_thread})
    monkeypatch.setattr(sessions, "update", lambda name, **fields: writes.append(fields) or fields)
    result = runtime_policy.run("session.patch_meta", {"service_tier":"priority"},
        {"provider":"codex", "name":"session", "key":"k", "validate":lambda: None, "state_path":str(state)})
    return result, writes


def test_codex_patch_accepts_service_tier_from_rust(tmp_path, monkeypatch):
    result, writes = _codex_patch(tmp_path, monkeypatch, "t1")
    assert result == {"updated": True}
    assert writes == [{"service_tier":"priority"}]


def test_codex_service_tier_skips_sidecar_of_another_thread(tmp_path, monkeypatch):
    result, writes = _codex_patch(tmp_path, monkeypatch, "t2")
    assert result == {"updated": False, "stale": True}
    assert writes == []
