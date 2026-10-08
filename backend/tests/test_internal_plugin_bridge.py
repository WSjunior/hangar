# backend/tests/test_internal_plugin_bridge.py
"""Rota interna temporária que o Rust usa para a pergunta do plugin (some quando o plugin for do Rust)."""
from unittest.mock import patch

import pytest
from fastapi.testclient import TestClient

import app.api as api_mod
from app import internal_api

SECRET = "ab" * 32
ROUTE = "/internal/sessions/s1/plugin"
HEADERS = {"X-Hangar-Internal": SECRET}


@pytest.fixture(autouse=True)
def _env():
    internal_api.set_secret(SECRET)
    yield
    internal_api.set_secret(None)


def _client():
    return TestClient(api_mod.app, client=("127.0.0.1", 50000))


def test_get_returns_the_pending_question():
    pending = {"id": "perm:1", "questions": [], "tool": "Bash", "resumo": None}
    with patch("app.plugin_bridge.pergunta_pendente", return_value=pending) as seen:
        r = _client().get(ROUTE, headers=HEADERS)
    assert (r.status_code, r.json()) == (200, {"pending": pending})
    seen.assert_called_once_with("s1")


def test_get_without_a_question_is_null():
    with patch("app.plugin_bridge.pergunta_pendente", return_value=None):
        assert _client().get(ROUTE, headers=HEADERS).json() == {"pending": None}


@pytest.mark.parametrize("interrupted", ["ask:7", None])
def test_post_tells_the_bridge_what_the_esc_closed(interrupted):
    with patch("app.plugin_bridge.interrompeu") as seen:
        r = _client().post(ROUTE, headers=HEADERS, json={"interrupted": interrupted})
    assert (r.status_code, r.json()) == (200, {"ok": True})
    seen.assert_called_once_with("s1", interrupted)


@pytest.mark.parametrize("body", [{}, {"interrupted": 5}, {"interrupted": "a", "x": 1}])
def test_post_with_a_bad_body_is_422(body):
    with patch("app.plugin_bridge.interrompeu") as seen:
        assert _client().post(ROUTE, headers=HEADERS, json=body).status_code == 422
    seen.assert_not_called()


@pytest.mark.parametrize("headers", [{}, {"X-Hangar-Internal": "errado"}])
def test_without_the_secret_both_methods_are_404(headers):
    with patch("app.plugin_bridge.pergunta_pendente") as read, patch("app.plugin_bridge.interrompeu") as write:
        assert _client().get(ROUTE, headers=headers).status_code == 404
        assert _client().post(ROUTE, headers=headers, json={"interrupted": None}).status_code == 404
    read.assert_not_called()
    write.assert_not_called()


def test_outside_loopback_is_404():
    client = TestClient(api_mod.app, client=("10.0.0.5", 50000))
    assert client.get(ROUTE, headers=HEADERS).status_code == 404


def test_the_rust_diary_accepts_the_select_events():
    for event in ("opcao.nao_convergiu", "opcao.envio_falhou"):
        body = {"evento": event, "sessao": "s1", "codigo": "x", "motivo": "m"}
        with patch("app.diag.registrar") as seen:
            assert _client().post("/internal/diag", headers=HEADERS, json=body).status_code == 200
        assert event in [call.args[0] for call in seen.call_args_list]
