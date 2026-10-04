"""O diário interno recebe apenas causas conhecidas, sem transportar erros brutos."""
import json

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient

from app import diag, internal_api

SECRET = "segredo-sintetico"
ROUTE = "/internal/rust-failure"


@pytest.fixture(autouse=True)
def isolated_diary(monkeypatch):
    lines = []
    monkeypatch.setattr(diag, "_escrever", lambda rows: lines.extend(rows))
    internal_api.set_secret(SECRET)
    yield lines
    internal_api.set_secret(None)


def client(ip="127.0.0.1"):
    app = FastAPI()
    app.include_router(internal_api.router)
    return TestClient(app, client=(ip, 50000))


def payload(**updates):
    return {"part": "costs", "code": "sqlite", "attempt": 1, "transferred": False, **updates}


def post(body=None, **options):
    return client().post(ROUTE, json=payload() if body is None else body,
                         headers={"X-Hangar-Internal": SECRET, **options.pop("headers", {})}, **options)


@pytest.mark.parametrize("code,reason", [
    ("no_scopes", "Escopos de custos indisponíveis."),
    ("no_disk", "Índice de custos indisponível."),
    ("reader_panic", "Falha no leitor de custos."),
    ("sqlite", "Falha no índice de custos."),
    ("json", "Falha ao preparar a resposta de custos."),
    ("worker_join", "Falha no processamento de custos."),
    ("info_unavailable", "Informações da sessão indisponíveis."),
    ("io", "Falha na leitura dos dados de custos."),
    ("non_finite", "Valor de custos ou uso inválido."),
])
def test_known_reasons_are_derived_by_backend(isolated_diary, code, reason):
    response = post(payload(code=code), headers={"X-Hangar-Req": "request-123"})
    assert response.status_code == 200 and response.json() == {"ok": True}
    assert len(isolated_diary) == 1
    row = isolated_diary[0]
    assert (row["evento"], row["nivel"], row["codigo"], row["detalhe"]) == (
        "rust.part_failed", "erro", code, reason)
    assert (row["etapa"], row["tentativa"], row["req"]) == ("costs", 1, "request-123")
    assert "sessao" not in row and SECRET not in repr(row)


@pytest.mark.parametrize("part", ["costs", "usage", "exchange_rate", "session_cost"])
def test_transfer_is_a_separate_event(isolated_diary, part):
    body = payload(part=part, attempt=4, transferred=True)
    if part == "session_cost":
        body["session"] = "session-1._"
    assert post(body).status_code == 200
    assert len(isolated_diary) == 1
    row = isolated_diary[0]
    assert (row["evento"], row["nivel"], row["etapa"], row["tentativa"]) == (
        "rust.part_to_python", "aviso", part, 4)
    assert row.get("sessao") == body.get("session")


@pytest.mark.parametrize("updates", [
    {"part": "conteúdo-privado"}, {"part": []}, {"code": "conteúdo-privado"}, {"code": {}},
    {"message": "conteúdo-privado"}, {"req": "conteúdo-privado"}, {"query": "conteúdo-privado"},
    {"attempt": 0}, {"attempt": 5}, {"attempt": True}, {"attempt": 1.0}, {"attempt": "1"},
    {"transferred": 1}, {"transferred": "false"}, {"session": None}, {"session": "session-1"},
    {"part": "session_cost"}, {"part": "session_cost", "session": ""},
    {"part": "session_cost", "session": "x" * 65},
    {"part": "session_cost", "session": "../conteúdo-privado"},
    {"part": "session_cost", "session": "session\n"},
])
def test_invalid_payload_is_rejected_without_logging(isolated_diary, updates):
    response = post(payload(**updates))
    assert response.status_code == 400
    assert "conteúdo-privado" not in response.text and isolated_diary == []


@pytest.mark.parametrize("raw", [b"{", b"[]", b"null", b"\xff", b"{}",
    b'{"part":"costs","part":"usage","code":"sqlite","attempt":1,"transferred":false}',
])
def test_invalid_json_is_rejected_without_logging(isolated_diary, raw):
    response = client().post(ROUTE, content=raw, headers={"X-Hangar-Internal": SECRET})
    assert response.status_code == 400 and isolated_diary == []


def test_body_limit_checks_streamed_size_before_logging(isolated_diary):
    raw = json.dumps(payload()).encode() + b" " * 1024
    response = client().post(ROUTE, content=iter((raw[:500], raw[500:])),
                             headers={"X-Hangar-Internal": SECRET})
    assert response.status_code == 413 and isolated_diary == []


@pytest.mark.parametrize("ip,secret", [
    ("127.0.0.1", None), ("127.0.0.1", "errado"), ("10.0.0.7", SECRET), ("testclient", SECRET),
])
def test_loopback_and_secret_are_required(isolated_diary, ip, secret):
    headers = {} if secret is None else {"X-Hangar-Internal": secret}
    response = client(ip).post(ROUTE, json=payload(message="conteúdo-privado"), headers=headers)
    assert response.status_code == 404
    assert all(not row["evento"].startswith("rust.") for row in isolated_diary)
    assert "conteúdo-privado" not in repr(isolated_diary) and SECRET not in repr(isolated_diary)


def test_missing_configured_secret_rejects_request(isolated_diary):
    internal_api.set_secret(None)
    assert post().status_code == 404
    assert all(not row["evento"].startswith("rust.") for row in isolated_diary)


@pytest.mark.parametrize("request_id", ["", "x" * 33, "secret@example.com", "raw/request"])
def test_invalid_correlation_never_reaches_rust_diary(isolated_diary, request_id):
    inherited = diag.req_atual.set("outside-request")
    try:
        assert post(headers={"X-Hangar-Req": request_id}).status_code == 200
        assert diag.req_atual.get() == "outside-request"
    finally:
        diag.req_atual.reset(inherited)
    assert isolated_diary[0].get("req", "") == ""


def test_route_stays_out_of_public_schema():
    assert ROUTE not in client().app.openapi()["paths"]
