"""Sessão criada por outra (MCP `new_session`, `hangar-send --new`) herda o que veio omitido."""

from pathlib import Path

import pytest
from fastapi import HTTPException

from app import api, cotas, default_model
from app.models import SessionInfo

pytestmark = pytest.mark.asyncio


@pytest.fixture
def criadora(monkeypatch):
    """Sessão `mae`, Claude com terminal, em bypass, na conta /c; cada teste muda o que precisa."""
    estado = {"info": SessionInfo(name="mae", provider="claude", headless=False, jsonl="/x/sid.jsonl"),
              "meta": None, "modo": "bypassPermissions", "conta": (Path("/c"), True)}

    async def cached(name):
        return estado["info"] if name == "mae" else None

    monkeypatch.setattr(api, "_cached_info", cached)
    monkeypatch.setattr(api.headless_sessions, "load", lambda name: estado["meta"])
    monkeypatch.setattr(api.permission_mode, "session_non_plan_mode", lambda jsonl: estado["modo"])
    monkeypatch.setattr(api, "_caller_config_dir", lambda name: estado["conta"])
    return estado


def _body(**kw):
    return api.CreateBody(**{"name": "nova", "cwd": "/tmp", "provider": "claude", "creator": "mae", **kw})


async def test_herda_bypass_terminal_e_conta(criadora):
    body, origem, _ = await api._inherit_from_creator(_body())
    # headless False explícito: o padrão "sem terminal" do servidor não vale mais para ela.
    assert (body.permission_mode, body.headless, body.config_dir, origem) == \
        ("bypassPermissions", False, "/c", "inherited")


async def test_plan_vindo_de_bypass_herda_bypass_e_sem_terminal(criadora):
    criadora["info"] = criadora["info"].model_copy(update={"headless": True})
    criadora["meta"] = {"permission_mode": "plan", "previous_non_plan": "bypassPermissions"}
    body, _, _ = await api._inherit_from_creator(_body())
    assert (body.permission_mode, body.headless) == ("bypassPermissions", True)


async def test_default_do_transcript_vira_manual(criadora):
    criadora["modo"] = "default"
    body, _, _ = await api._inherit_from_creator(_body())
    assert body.permission_mode == "manual"


async def test_explicito_vence(criadora):
    criadora["conta"] = None  # chamar _caller_config_dir quebraria o desempacotamento
    body, origem, _ = await api._inherit_from_creator(
        _body(permission_mode="manual", headless=True, config_dir="/outra"))
    assert (body.permission_mode, body.headless, body.config_dir, origem) == ("manual", True, "/outra", None)


async def test_sem_criadora_nada_muda(criadora):
    body, origem, _ = await api._inherit_from_creator(_body(creator=None))
    assert (body.permission_mode, body.headless, body.config_dir, origem) == (None, None, None, None)


async def test_codex_herda_sem_terminal_mas_nao_o_modo(criadora):
    criadora["info"] = SessionInfo(name="mae", provider="codex", headless=True)
    body, origem, _ = await api._inherit_from_creator(_body(provider="codex"))
    assert (body.headless, body.permission_mode, origem) == (True, None, None)


async def test_conta_ilegivel_recusa(criadora):
    criadora["conta"] = (None, False)
    with pytest.raises(HTTPException) as e:
        await api._inherit_from_creator(_body())
    assert e.value.status_code == 409


@pytest.mark.parametrize("uso, conta, origem", [(95, "/folga", "quota"), (50, "/c", "inherited")])
async def test_create_session_troca_conta_herdada_acabando(criadora, monkeypatch, uso, conta, origem):
    lidas = [cotas.CotaConta(id=f"claude:{p}", label=p, provedor="claude", estado="lida",
                             janelas=[cotas.JanelaCota(rotulo="5h", pct=pct)])
             for p, pct in (("/c", uso), ("/folga", 10))]
    monkeypatch.setattr(cotas, "cotas_claude", lambda: lidas)
    monkeypatch.setattr(api, "list_config_dirs", lambda: [])
    monkeypatch.setattr(default_model, "drop_foreign", lambda cfg: ([], []))
    criados = []

    async def criar(body, worktree):
        criados.append(body)
        return SessionInfo(name=body.name, cwd=body.cwd, provider=body.provider)

    monkeypatch.setattr(api, "_criar_sessao", criar)
    info = await api.create_session(_body())
    assert (criados[0].config_dir, info.config_dir, info.account_source) == (conta, conta, origem)


async def test_criadora_ausente_ou_modo_ilegivel_avisa(criadora):
    _, _, avisos = await api._inherit_from_creator(_body(creator="sumiu"))
    assert "não encontrada" in avisos[0]
    criadora["modo"] = None
    body, _, avisos = await api._inherit_from_creator(_body())
    assert body.permission_mode is None and "modo de permissão" in avisos[0]
