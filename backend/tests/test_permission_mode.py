"""Parser do rodapé do modo de permissão — puro, sem tmux real."""
import json
from collections import OrderedDict

import app.permission_mode as pm


def _pane(linha):
    # helper: pane com linhas dummy + linha do rodapé no fim
    return f"alguma conversa\n{linha}\n❯ "


def test_parse_plan():
    assert pm.parse_permission_mode(_pane("⏸ plan mode on (shift+tab to cycle) · ← for agents")) == "plan"


def test_parse_auto():
    assert pm.parse_permission_mode(_pane("⏵⏵ auto mode on (shift+tab to cycle) · ← for agents")) == "auto"


def test_parse_manual():
    assert pm.parse_permission_mode(_pane("⏸ manual mode on · ← for agents")) == "manual"


def test_parse_accept_edits():
    assert pm.parse_permission_mode(_pane("⏵⏵ accept edits on (shift+tab to cycle) · ← for agents")) == "acceptEdits"


def test_parse_bypass():
    assert pm.parse_permission_mode(_pane("⏵⏵ bypass permissions on (shift+tab to cycle) · ← for agents")) == "bypassPermissions"


def test_parse_dont_ask():
    assert pm.parse_permission_mode(_pane("⏵⏵ don't ask on (shift+tab to cycle) · ← for agents")) == "dontAsk"


def test_parse_dont_ask_curvo():
    # apóstrofe tipográfico ’
    assert pm.parse_permission_mode(_pane("⏵⏵ don’t ask on (shift+tab to cycle) · ← for agents")) == "dontAsk"


def test_parse_ultima_linha_vence():
    pane = "⏸ plan mode on\n⏵⏵ auto mode on (shift+tab to cycle)\n⏸ manual mode on\n"
    # última linha com modo é manual
    assert pm.parse_permission_mode(pane) == "manual"


def test_parse_sem_glifo_cai_no_fallback():
    # pane estreito que cortou o glifo: segunda passada pega sem glifo
    pane = "alguma conversa\nplan mode on (shift+tab to cycle)\n❯ "
    assert pm.parse_permission_mode(pane) == "plan"


def test_parse_sem_modo_retorna_none():
    assert pm.parse_permission_mode("conversa sem rodapé\n❯ ") is None


def test_parse_conversa_citando_modo_com_glifo_falso_positivo_evitado():
    # conversa que cita \"auto mode on\" sem glifo não deve casar na primeira passada,
    # mas a segunda passada casaria (fallback). O comportamento atual é casar no fallback,
    # mas a primeira passada com glifo protege o caso comum. Testa que com glifo vazio,
    # o fallback ainda acha (para pane estreito).
    pane = "user: auto mode on is great\n⏸ plan mode on\n"
    assert pm.parse_permission_mode(pane) == "plan"


def test_parse_case_insensitive():
    assert pm.parse_permission_mode(_pane("⏸ PLAN MODE ON")) == "plan"
    assert pm.parse_permission_mode(_pane("⏵⏵ AUTO MODE ON")) == "auto"


def test_listar_modos_dontask_nao_manda_tecla(monkeypatch):
    """Bloqueador 2: sondar em dontAsk não manda BTab e devolve ([], dontAsk)."""
    monkeypatch.setattr(pm, "ler_modo", lambda name: "dontAsk")
    chamadas = []
    monkeypatch.setattr(pm.tmux, "send_keys", lambda name, keys: chamadas.append(keys) or True)
    monkeypatch.setattr(pm, "_espera_modo", lambda *a, **kw: None)
    cur, modos = pm.listar_modos("sess")
    assert chamadas == []
    assert cur == "dontAsk"
    assert modos == []


def test_modo_da_conta_le_settings_e_cai_no_padrao_do_app(tmp_path):
    """Sem `defaultMode` na conta a sessão nasceria pedindo permissão por ferramenta."""
    conta = tmp_path / "conta"
    conta.mkdir()
    assert pm.modo_da_conta(str(conta)) == pm.PADRAO_DO_APP
    (conta / "settings.json").write_text('{"permissions": {"defaultMode": "plan"}}', encoding="utf-8")
    assert pm.modo_da_conta(str(conta)) == "plan"
    # "default" é o nome no settings.json; a flag da CLI só conhece "manual".
    (conta / "settings.json").write_text('{"permissions": {"defaultMode": "default"}}', encoding="utf-8")
    assert pm.modo_da_conta(str(conta)) == "manual"
    (conta / "settings.json").write_text("{ nao e json", encoding="utf-8")
    assert pm.modo_da_conta(str(conta)) == pm.PADRAO_DO_APP


def test_listar_modos_devolve_ficou_nao_orig(monkeypatch):
    """Bloqueador 3: GET devolve o que FICOU, não o de antes (plan -> auto -> manual -> preso)."""
    # orig = plan, ciclo descobre auto, manual, depois repete manual (sub-ciclo) -> cur=manual
    # volta ao orig falha (pane ilegível), então cur permanece manual
    seq_ler = iter(["plan"])  # só o primeiro ler_modo (orig)
    monkeypatch.setattr(pm, "ler_modo", lambda name: next(seq_ler, "manual"))
    seq_espera = iter(["auto", "manual", "manual", None, None, None, None])
    monkeypatch.setattr(pm, "_espera_modo", lambda name, anterior=None, timeout=2.0: next(seq_espera, None))
    chamadas = []
    monkeypatch.setattr(pm.tmux, "send_keys", lambda name, keys: chamadas.append(keys) or True)
    cur, modos = pm.listar_modos("sess")
    # com o fix, cur é manual (ficou), não plan (orig)
    assert cur == "manual"
    assert "plan" in modos
    assert "manual" in modos


def test_troca_acompanha_o_rodape_sem_pausa_fixa(monkeypatch):
    """O rodapé muda logo depois do BTab: cada tecla espera só a primeira leitura curta, sem pausa fixa."""
    ciclo = ["manual", "acceptEdits", "plan", "auto"]
    pos = [0]
    dormidas = []
    monkeypatch.setattr(pm.time, "sleep", dormidas.append)
    monkeypatch.setattr(pm, "ler_modo", lambda name: ciclo[pos[0] % len(ciclo)])
    monkeypatch.setattr(pm.tmux, "send_keys", lambda name, keys: pos.__setitem__(0, pos[0] + 1) or True)
    assert pm.trocar_modo("sess", "auto") == "auto"
    assert pos[0] == 3
    assert dormidas == [pm.INTERVALO_POLL] * 3


def test_espera_de_pane_parado_alonga_o_intervalo(monkeypatch):
    """Pane que não muda não é lido a cada 20 ms até o teto: o intervalo cresce até o máximo."""
    agora = [0.0]
    dormidas = []

    def dormir(segundos):
        dormidas.append(segundos)
        agora[0] += segundos

    monkeypatch.setattr(pm.time, "sleep", dormir)
    monkeypatch.setattr(pm.time, "monotonic", lambda: agora[0])
    monkeypatch.setattr(pm, "ler_modo", lambda name: "plan")
    assert pm._espera_modo("sess", anterior="plan") == "plan"
    assert dormidas[0] == pm.INTERVALO_POLL
    assert max(dormidas) == pm.INTERVALO_POLL_MAX
    assert len(dormidas) < 20


def _transcript(tmp_path, nome, modos):
    p = tmp_path / f"{nome}.jsonl"
    p.write_text("".join(json.dumps({"type": "user", "permissionMode": m}) + "\n" for m in modos))
    return str(p)


def test_transcript_non_plan_mode_pula_plan(tmp_path):
    assert pm.transcript_non_plan_mode(_transcript(tmp_path, "a", ["default", "bypassPermissions", "plan"])) \
        == "bypassPermissions"
    assert pm.transcript_non_plan_mode(_transcript(tmp_path, "b", ["plan"])) is None
    assert pm.transcript_non_plan_mode(str(tmp_path / "sumiu.jsonl")) is None


def test_transcript_non_plan_mode_nao_rele_o_mesmo_transcript(tmp_path, monkeypatch):
    import app.worktrees as wt

    leituras = []
    original = wt.reversed_lines
    monkeypatch.setattr(wt, "reversed_lines", lambda path: leituras.append(path) or original(path))
    caminho = _transcript(tmp_path, "c", ["default", "acceptEdits"])
    assert pm.transcript_non_plan_mode(caminho) == "acceptEdits"
    assert pm.transcript_non_plan_mode(caminho) == "acceptEdits"
    assert len(leituras) == 1
    with open(caminho, "a") as f:
        f.write(json.dumps({"type": "user", "permissionMode": "bypassPermissions"}) + "\n")
    assert pm.transcript_non_plan_mode(caminho) == "bypassPermissions"
    assert len(leituras) == 2


def test_session_non_plan_mode_le_a_memoria_pelo_session_id(tmp_path, monkeypatch):
    # O monitor grava a memória pelo session-id (stem do jsonl); pelo nome da sessão ela não vale.
    monkeypatch.setattr(pm, "_ultimos_nao_plan", OrderedDict({"sess": "acceptEdits"}))
    jsonl = _transcript(tmp_path, "sid-1", ["bypassPermissions"])
    assert pm.session_non_plan_mode(jsonl) == "bypassPermissions"
    monkeypatch.setattr(pm, "_ultimos_nao_plan", OrderedDict({"sid-1": "acceptEdits"}))
    assert pm.session_non_plan_mode(jsonl) == "acceptEdits"
    assert pm.session_non_plan_mode(None) is None
