"""Continuar a mesma conversa noutra conta: o processo para antes de a conversa mudar de conta,
reabre como estava (terminal ou sem terminal) na conta nova, e volta na de origem se não der pra mover."""
from pathlib import Path
from unittest.mock import AsyncMock, MagicMock, patch

import pytest
from fastapi.testclient import TestClient

from app.adapters.claude_headless import sessions as S
from app.adapters.claude_headless.adapter import ClaudeHeadlessAdapter
from app.config import ConfigDirInfo, settings
from app.models import SessionInfo
from app.registry import sanitize_cwd

SID = "22222222-2222-2222-2222-222222222222"
_H = {"Authorization": "Bearer secret"}


@pytest.fixture
def contas(tmp_path, monkeypatch):
    import app.api as api_mod
    import app.archive as archive_mod
    a, b = tmp_path / ".claude-a", tmp_path / ".claude-b"
    for c in (a, b):
        (c / "projects").mkdir(parents=True)
    lista = [ConfigDirInfo(path=str(a), label="a", active=True), ConfigDirInfo(path=str(b), label="b", active=False)]
    monkeypatch.setattr(api_mod, "list_config_dirs", lambda ordered=True: lista)
    monkeypatch.setattr(archive_mod, "list_config_dirs", lambda ordered=True: lista)
    monkeypatch.setattr("app.cotas.cotas_claude", lambda: [])
    monkeypatch.setattr(settings, "projects_dir", a / "projects")
    monkeypatch.setattr(S, "_dir", lambda: tmp_path / "hl")
    monkeypatch.setattr(S, "_trocando", {})
    settings.auth_token = "secret"
    return str(a), str(b)


def _conversa(conta: str, cwd: str) -> Path:
    jsonl = Path(conta) / "projects" / sanitize_cwd(cwd) / f"{SID}.jsonl"
    jsonl.parent.mkdir(parents=True, exist_ok=True)
    jsonl.write_text("{}\n")
    (jsonl.parent / SID).mkdir()
    return jsonl


def _post(name, destino, *, headless, conta, hl, **extra):
    import app.api as api_mod
    info = SessionInfo(name=name, cwd="/tmp", provider="claude", headless=headless, conta=f"claude:{conta}")
    with patch("app.api._cached_info", AsyncMock(return_value=info)), \
         patch("app.api._headless", return_value=headless), \
         patch("app.api._motivo_ocupada", AsyncMock(return_value=None)), \
         patch("app.api.get_adapter", return_value=hl), \
         patch.object(api_mod.registry, "_forget"), \
         patch.object(api_mod.perm_mode, "ler_modo", return_value="manual"), \
         patch.object(api_mod.registry, "para_headless", extra.get("ida")), \
         patch.object(api_mod.registry, "para_terminal", extra.get("volta")):
        return TestClient(api_mod.app).post(f"/api/sessions/{name}/conta", headers=_H, json={"config_dir": destino})


def _hl(ordem):
    hl = ClaudeHeadlessAdapter()
    hl.parar = AsyncMock(side_effect=lambda n: ordem.append("parou"))
    hl.acordar = MagicMock(side_effect=lambda n: ordem.append(("acordou", S.load(n)["config_dir"])))
    return hl


def test_sem_terminal_para_move_e_religa_na_conta_nova(contas, tmp_path):
    a, b = contas
    cwd = str(tmp_path / "repo")
    S.save("hl", cwd, SID, config_dir=a)
    origem = _conversa(a, cwd)
    ordem = []
    r = _post("hl", b, headless=True, conta=a, hl=_hl(ordem))
    assert r.status_code == 200 and r.json() == {"ok": True, "config_dir": b}
    destino = Path(b) / "projects" / origem.parent.name
    assert (destino / f"{SID}.jsonl").exists() and (destino / SID).is_dir() and not origem.exists()
    assert ordem == ["parou", ("acordou", b)]


def test_terminal_passa_por_sem_terminal_e_reabre_o_pane_na_conta_nova(contas, tmp_path):
    a, b = contas
    cwd = str(tmp_path / "repo")
    origem = _conversa(a, cwd)
    visto = {}
    ida = MagicMock(side_effect=lambda n, modo: S.save(n, cwd, SID, config_dir=a, permission_mode=modo))
    volta = MagicMock(side_effect=lambda n: visto.update(conta=S.load(n)["config_dir"], na_origem=origem.exists()))
    r = _post("t1", b, headless=False, conta=a, hl=_hl([]), ida=ida, volta=volta)
    assert r.status_code == 200
    ida.assert_called_once_with("t1", "manual")
    assert visto == {"conta": b, "na_origem": False}


def test_conta_destino_com_a_mesma_conversa_reabre_na_origem(contas, tmp_path):
    a, b = contas
    cwd = str(tmp_path / "repo")
    S.save("hl", cwd, SID, config_dir=a)
    origem = _conversa(a, cwd)
    _conversa(b, cwd)
    ordem = []
    r = _post("hl", b, headless=True, conta=a, hl=_hl(ordem))
    assert r.status_code == 409 and r.json()["detail"]["code"] == "erro_conversa_ja_na_conta"
    assert origem.exists() and ordem == ["parou", ("acordou", a)]


def test_conta_perto_do_limite_e_recusada_e_vai_para_o_fim_da_lista(contas, tmp_path, monkeypatch):
    import app.api as api_mod
    from app.cotas import CotaConta, JanelaCota
    a, b = contas
    c = str(tmp_path / ".claude-c")
    lista = api_mod.list_config_dirs() + [ConfigDirInfo(path=c, label="c", active=False)]
    monkeypatch.setattr(api_mod, "list_config_dirs", lambda ordered=True: lista)
    cheia = CotaConta(id=f"claude:{b}", label="b", provedor="claude", estado="lida",
                      janelas=[JanelaCota(rotulo="5h", pct=20), JanelaCota(rotulo="7d", pct=99)])
    acabando = CotaConta(id=f"claude:{c}", label="c", provedor="claude", estado="lida", janelas=[JanelaCota(rotulo="5h", pct=96)])
    monkeypatch.setattr("app.cotas.cotas_claude", lambda: [cheia, acabando])
    assert [(d["label"], d["low"], d["full"]) for d in api_mod._account_targets(a)] == [("c", True, False), ("b", True, True)]
    S.save("hl", str(tmp_path / "repo"), SID, config_dir=a)
    ordem = []
    r = _post("hl", b, headless=True, conta=a, hl=_hl(ordem))
    assert r.status_code == 409 and r.json()["detail"]["code"] == "erro_conta_cheia" and ordem == []


def test_conta_fora_da_lista_e_recusada(contas):
    import app.api as api_mod
    r = TestClient(api_mod.app).post("/api/sessions/hl/conta", headers=_H, json={"config_dir": "/etc"})
    assert r.status_code == 400 and r.json()["detail"]["code"] == "erro_config_dir_invalido"
