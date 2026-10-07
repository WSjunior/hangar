"""Fatos do estado por empurrão (`state_facts`) e os serviços que o Rust pede ao Python. Nada aqui é
consumido em produção antes do `Monitor` do Rust existir: sem interesse registrado, nada sai."""
import asyncio
from types import SimpleNamespace
from unittest.mock import AsyncMock, patch

import pytest
from fastapi.testclient import TestClient

import app.api as api_mod
from app import internal_api, permission_mode, plugin_bridge as pb, runtime_policy, state_facts
from app.models import SessionInfo

SECRET = "cd" * 32
UUID = "0b6e5c1a-1111-4222-8333-444455556666"


@pytest.fixture(autouse=True)
def _clean(monkeypatch):
    sent = []
    monkeypatch.setattr(state_facts, "_threaded", False)
    monkeypatch.setattr(state_facts, "_send", lambda name, body: sent.append((name, body)))
    monkeypatch.setattr(pb, "tracked_session_id", lambda name: UUID)
    internal_api.set_secret(SECRET)
    yield sent
    internal_api.set_secret(None)
    state_facts.reset()
    for d in (pb._perguntas, pb._waiters, pb._estados, pb._batidas, pb._eventos, pb._fechadas,
              pb._donos, pb._recusas, pb._sugestoes):
        d.clear()


def _state(estado: str) -> None:
    asyncio.run(pb.state(pb.StateBody(sessao="s1", token=pb.mint("s1"), estado=estado), None))


def test_state_facts_pushed_only_on_change_and_for_interest(_clean):
    sent = _clean
    _state("working")
    state_facts.flush()
    assert sent == [], "sem interesse registrado nada sai"

    r = TestClient(api_mod.app, client=("127.0.0.1", 5000)).get(
        "/internal/sessions/s1/state-facts", headers={"X-Hangar-Internal": SECRET})
    assert r.status_code == 200
    assert r.json()["plugin_state"]["state"] == "working"
    assert r.json()["seq"] == 0

    _state("idle")
    state_facts.flush()
    assert [(n, b["plugin_state"]["state"], b["seq"]) for n, b in sent] == [("s1", "idle", 1)]
    asyncio.run(pb.suggest(pb.SuggestBody(sessao="s1", token=pb.mint("s1"), texto="x", mostrada=True)))
    asyncio.run(pb.suggest(pb.SuggestBody(sessao="s1", token=pb.mint("s1"), texto="x", mostrada=True)))
    state_facts.flush()
    assert [b["suggestion"] for _, b in sent[1:]] == ["x"], "valor repetido não sai de novo"
    pb._guardar_faixa("other", None, None, [])
    state_facts.flush()
    assert len(sent) == 2, "outra sessão sem interesse não envia"


def test_state_facts_carry_ages_not_monotonic_instants(_clean):
    import time
    agora = time.monotonic()
    pb._estados["s1"] = (agora - 5.0, "working", None)
    pb._batidas["s1"] = agora - 2.0
    pb._perguntas["s1"] = {"id": "q1", "questions": [], "tool": None, "resumo": None, "visto": agora - 30.0}
    facts = state_facts.snapshot("s1")
    assert 4900 <= facts["plugin_state"]["age_ms"] <= 6000
    assert 1900 <= facts["heartbeat_age_ms"] <= 3000
    assert 29900 <= facts["question"]["seen_age_ms"] <= 31000
    assert "visto" not in facts["question"]
    assert facts["waiter_open"] is False and facts["in_transfer_ms"] == 0
    assert facts["transfer_active"] is False and facts["permission_op"] is False


def test_longpoll_and_ask_push_rate_limited(_clean, monkeypatch):
    sent = _clean
    clock = [1000.0]
    monkeypatch.setattr(state_facts, "_clock", lambda: clock[0])
    monkeypatch.setattr(pb, "ESPERA_S", 0.05)
    state_facts.snapshot("s1")

    async def pull_open_then_close():
        task = asyncio.create_task(pb.pull(pb.PullBody(sessao="s1", token=pb.mint("s1"), instance="a",
                                                       session_id=UUID)))
        while "s1" not in pb._waiters:
            await asyncio.sleep(0.001)
        state_facts.flush()
        await task
        state_facts.flush()

    asyncio.run(pull_open_then_close())
    assert [b["waiter_open"] for _, b in sent] == [True, False], "início e fim do long-poll saem"

    def ask():
        asyncio.run(pb.ask(pb.AskBody(sessao="s1", token=pb.mint("s1"), id="q1", questions=[], janela_ms=1)))
        state_facts.flush()

    ask()
    assert sent[-1][1]["question"]["id"] == "q1", "pergunta nova sai na hora"
    before = len(sent)
    for _ in range(3):
        clock[0] += 5
        ask()
    assert len(sent) == before, "a renovação do /ask espera 25 s"
    clock[0] += 10
    state_facts.snapshot("s1")
    ask()
    assert len(sent) == before + 1, "25 s depois a renovação sai"
    clock[0] += 31
    ask()
    assert len(sent) == before + 1, "interesse vencido sem novo retrato: nada sai"


def test_dead_service_refuses_during_transfer(monkeypatch):
    from app.adapters.claude_headless import sessions
    forgotten = []
    monkeypatch.setattr(pb, "esquecer", lambda name: forgotten.append(("plugin", name)))
    monkeypatch.setattr("app.state.forget_frame", lambda name: forgotten.append(("frame", name)))
    sessions.marcar_troca("s1")
    try:
        assert runtime_policy.state_service_sync("session.dead", "s1", {}) == {"result": "em_troca"}
        assert forgotten == []
    finally:
        sessions._trocando.pop("s1", None)
    assert runtime_policy.state_service_sync("session.dead", "s1", {}) == {"result": "ok"}
    assert forgotten == [("plugin", "s1"), ("frame", "s1")]


def test_permission_observe_returns_previous_non_plan():
    sid = "sid-observe"
    assert runtime_policy.state_service_sync("permission.observe", "s1", {"sid": sid, "mode": "acceptEdits"}) == \
        {"mode": "acceptEdits", "previous_non_plan": "acceptEdits"}
    assert runtime_policy.state_service_sync("permission.observe", "s1", {"sid": sid, "mode": "plan"}) == \
        {"mode": "plan", "previous_non_plan": "acceptEdits"}
    with permission_mode.operacao_controlada("s1"):
        assert state_facts.snapshot("s1")["permission_op"] is True
        assert runtime_policy.state_service_sync("permission.observe", "s1", {"sid": sid, "mode": "auto"}) == \
            {"mode": "plan", "previous_non_plan": "acceptEdits"}, "operação controlada preserva o retrato"
    with pytest.raises(ValueError):
        runtime_policy.state_service_sync("permission.observe", "s1", {"sid": sid, "mode": "qualquer"})


def test_deliverable_service_runs_prepare_session():
    slot = SimpleNamespace(binding=SimpleNamespace(meta={"terminal": True}))
    owner = SimpleNamespace(legacy=object(), prepare_session=AsyncMock(return_value=True),
                            slot=lambda name: slot, op=AsyncMock(return_value={"disposition": "accepted", "sent": 2}))
    info = SessionInfo(name="s1", cwd="/p", jsonl=f"/p/{UUID}.jsonl", provider="claude")
    with patch("app.runtime_coordinator.current", lambda: owner), \
            patch("app.api._cached_info", AsyncMock(return_value=info)):
        r = TestClient(api_mod.app, client=("127.0.0.1", 5000)).post(
            "/internal/sessions/s1/state-service", headers={"X-Hangar-Internal": SECRET},
            json={"kind": "session.deliverable", "payload": {}})
    assert r.status_code == 200 and r.json() == {"ok": True, "data": {"sent": 2}}
    owner.prepare_session.assert_awaited_once_with("s1", "claude")
    assert owner.op.await_args.args[1] == {"kind": "drain"}
