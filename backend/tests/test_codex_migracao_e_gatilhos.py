"""Primeira rodada numa máquina com a ponte antiga, hook de estado próprio, interruptor e gatilho de sessão."""
import json
import sys

import pytest

from app import codex_hook_installer, codex_integracao, runtime_config
from app.codex_compat import normalizar_hooks, wrapper_instalado
from app.codex_integracao import IntegracaoCodex, sem_hooks_do_app, sincronizacao_ligada

ESTADO = '"/repo/backend/.venv/bin/python" "/repo/backend/hooks/state_hook.py" || true'
RTK_PIPE = "rtk hook claude | python3 /repo/scripts/codex-hook-allow.py"
PESSOAL = "python3 /home/x/.claude/hooks/skill-suggester.py"


def _grupo(*comandos, **extras):
    return {**extras, "hooks": [{"type": "command", "command": c} for c in comandos]}


def _home(tmp_path):
    (tmp_path / ".claude").mkdir()
    (tmp_path / ".codex").mkdir()
    (tmp_path / ".claude/settings.json").write_text('{"enabledPlugins": {}}')
    return tmp_path


# ------------------------------------------------------------- ponte antiga

def test_primeira_rodada_tira_so_o_que_o_instalador_antigo_escreveu(tmp_path):
    home = _home(tmp_path)
    espelho = {"hooks": {"SessionStart": [_grupo(PESSOAL), _grupo(ESTADO)],
                         "PreToolUse": [_grupo(RTK_PIPE, matcher="Bash")]}}
    depois_a_mao = "python3 /home/x/meu-hook.py"
    atual = {"hooks": {"SessionStart": [_grupo(PESSOAL), _grupo(ESTADO), _grupo(depois_a_mao)],
                       "PreToolUse": [_grupo(RTK_PIPE, matcher="Bash")]}}
    (home / ".codex/.hangar-hooks.json").write_text(json.dumps(espelho))
    (home / ".codex/hooks.json").write_text(json.dumps(atual))
    service = IntegracaoCodex(home, home / ".codex")
    service._estado = codex_integracao._snapshot()
    service._migrar_ponte_antiga()
    novo = json.loads((home / ".codex/hooks.json").read_text())
    estado_novo = _grupo(codex_hook_installer._STATE_COMMAND)
    assert novo == {"hooks": {"SessionStart": [_grupo(depois_a_mao), estado_novo],
                              "UserPromptSubmit": [estado_novo], "PreToolUse": [estado_novo],
                              "PostToolUse": [estado_novo], "Stop": [estado_novo]}}
    assert not (home / ".codex/.hangar-hooks.json").exists()
    assert list(service.backups.iterdir()), "o espelho e o hooks.json anterior vão pro backup"
    assert service._estado["confianca_pendente"] is True


def test_sem_espelho_nao_toca_no_hooks_json(tmp_path):
    home = _home(tmp_path)
    raw = json.dumps({"hooks": {"SessionStart": [_grupo(ESTADO)]}})
    (home / ".codex/hooks.json").write_text(raw)
    service = IntegracaoCodex(home, home / ".codex")
    service._estado = codex_integracao._snapshot()
    service._migrar_ponte_antiga()
    assert (home / ".codex/hooks.json").read_text() == raw
    assert not service.raiz.exists()


# ------------------------------------------------------------- hooks do app

def test_hooks_do_app_nao_vao_pro_importador_os_do_usuario_vao():
    hooks = {"SessionStart": [_grupo(PESSOAL, ESTADO)],
             "Stop": [_grupo('"/x/backend/hooks/preview_hook.py" || exit 0')],
             "PreToolUse": [_grupo("rtk hook claude", matcher="Bash")],
             "Estranho": "não é lista"}
    assert sem_hooks_do_app(hooks) == {"SessionStart": [_grupo(PESSOAL)],
                                       "PreToolUse": [_grupo("rtk hook claude", matcher="Bash")],
                                       "Estranho": "não é lista"}


def test_instalador_acrescenta_uma_vez_e_nao_reescreve_entrada_existente(tmp_path):
    codex = tmp_path / ".codex"
    codex.mkdir()
    (codex / "hooks.json").write_text(json.dumps({"hooks": {"SessionStart": [_grupo(ESTADO)],
                                                             "PreToolUse": [_grupo(PESSOAL)]}}))
    assert codex_hook_installer.ensure_codex_state_hook_installed(codex) == [
        "UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop"]
    data = json.loads((codex / "hooks.json").read_text())
    assert data["hooks"]["SessionStart"] == [_grupo(ESTADO)], "formato antigo fica como está"
    assert [h["command"] for g in data["hooks"]["PreToolUse"] for h in g["hooks"]] == [
        PESSOAL, codex_hook_installer._STATE_COMMAND]
    assert codex_hook_installer.ensure_codex_state_hook_installed(codex) == []
    assert json.loads((codex / "hooks.json").read_text()) == data


@pytest.mark.parametrize("conteudo", ["{ quebrado", '{"hooks": []}'])
def test_instalador_nao_clobra_hooks_json_estranho(tmp_path, conteudo):
    codex = tmp_path / ".codex"
    codex.mkdir()
    (codex / "hooks.json").write_text(conteudo)
    assert codex_hook_installer.ensure_codex_state_hook_installed(codex) == []
    assert (codex / "hooks.json").read_text() == conteudo
    assert codex_hook_installer.ensure_codex_state_hook_installed(tmp_path / "nao-existe") == []


# ------------------------------------------------------------- rtk estável

def test_normalizacao_reusa_o_wrapper_ja_gravado():
    gravado = {"hooks": {"PreToolUse": [_grupo("/venv-a/bin/python3 /checkout-a/scripts/codex-hook-allow.py -- rtk hook claude",
                                              matcher="Bash")]}}
    assert wrapper_instalado(gravado) == ("/venv-a/bin/python3", "/checkout-a/scripts/codex-hook-allow.py")
    assert wrapper_instalado({"hooks": {"PreToolUse": [_grupo(RTK_PIPE)]}}) is None
    fonte = {"hooks": {"PreToolUse": [_grupo("rtk hook claude", matcher="Bash")]}}
    python, wrapper = wrapper_instalado(gravado)
    from pathlib import Path
    assert normalizar_hooks(fonte, python, Path(wrapper)) == gravado


def test_hooks_de_outro_checkout_nao_reescrevem_o_rtk_instalado(tmp_path, monkeypatch):
    home = _home(tmp_path)
    linha = "/venv-a/bin/python3 /checkout-a/scripts/codex-hook-allow.py -- rtk hook claude"
    raw = json.dumps({"hooks": {"PreToolUse": [_grupo(linha, matcher="Bash")]}})
    (home / ".codex/hooks.json").write_text(raw)
    monkeypatch.setattr(sys, "executable", "/venv-b/bin/python3")
    monkeypatch.setattr(codex_integracao, "_REPO", tmp_path / "checkout-b")
    service = IntegracaoCodex(home, home / ".codex")
    service._estado = codex_integracao._snapshot()
    registro = {}
    service._hooks({"hooks": {"PreToolUse": [_grupo("rtk hook claude", matcher="Bash")]}}, registro)
    assert (home / ".codex/hooks.json").read_text() == raw
    assert service._estado["confianca_pendente"] is False
    assert registro["hooks"]["hooks"]["PreToolUse"][0]["hooks"][0]["command"] == linha


# ------------------------------------------------------------- interruptor

@pytest.fixture
def rc(tmp_path, monkeypatch):
    monkeypatch.setattr(runtime_config, "_backend_config_base", lambda: tmp_path)
    monkeypatch.delenv("CP_CODEX_SYNC_ENABLED", raising=False)
    return runtime_config


def test_interruptor_da_tela_e_o_kill_switch_desligam_os_gatilhos(rc, monkeypatch):
    assert sincronizacao_ligada() is True
    rc.aplicar({"codex_sync": False})
    assert sincronizacao_ligada() is False
    rc.aplicar({"codex_sync": True, "automations": False})
    assert sincronizacao_ligada() is False
    rc.aplicar({"automations": True})
    assert sincronizacao_ligada() is True
    monkeypatch.setenv("CP_CODEX_SYNC_ENABLED", "0")
    assert sincronizacao_ligada() is False


def _registro(service, *, fingerprint, estado="ok", ha_segundos=10, proxima_em=3600):
    import time
    from app.codex_integracao import _iso
    service.raiz.mkdir(parents=True, exist_ok=True)
    (service.raiz / "estado.json").write_text(json.dumps({
        "fingerprint": fingerprint,
        "status": {"estado": estado, "ultima_execucao": _iso(time.time() - ha_segundos),
                   "proxima_atualizacao": _iso(time.time() + proxima_em)}}))


def test_abrir_sessao_so_reconcilia_quando_algo_mudou(tmp_path, monkeypatch):
    home = _home(tmp_path)
    service = IntegracaoCodex(home, home / ".codex")
    monkeypatch.setattr(service, "fingerprint", lambda *a, **k: "fp-1")
    assert service.precisa_reconciliar() is True, "nunca rodou"
    _registro(service, fingerprint="fp-1")
    assert service.precisa_reconciliar() is False, "fonte igual, marketplace no prazo, última ok"
    _registro(service, fingerprint="fp-0")
    assert service.precisa_reconciliar() is True, "fonte mudou"
    _registro(service, fingerprint="fp-1", proxima_em=-1)
    assert service.precisa_reconciliar() is True, "marketplace venceu"
    _registro(service, fingerprint="fp-1", estado="parcial", ha_segundos=10)
    assert service.precisa_reconciliar() is False, "falha recente espera a retentativa"
    _registro(service, fingerprint="fp-1", estado="parcial", ha_segundos=301)
    assert service.precisa_reconciliar() is True
    _registro(service, fingerprint="fp-1", estado="ocioso")
    assert service.precisa_reconciliar() is True, "rodada interrompida retoma"


async def test_gatilho_de_sessao_respeita_interruptor_e_cache(rc, tmp_path, monkeypatch):
    home = _home(tmp_path)
    service = IntegracaoCodex(home, home / ".codex")
    chamadas = []

    async def iniciar(motivo, forcar):
        chamadas.append((motivo, forcar))
        return {"estado": "executando"}

    monkeypatch.setattr(service, "iniciar", iniciar)
    monkeypatch.setattr(service, "fingerprint", lambda *a, **k: "fp-1")
    rc.aplicar({"codex_sync": False})
    await service.sessao()
    assert chamadas == [], "interruptor desligado"
    rc.aplicar({"codex_sync": True})
    _registro(service, fingerprint="fp-1")
    await service.sessao()
    assert chamadas == [], "nada mudou: não chama o Codex"
    _registro(service, fingerprint="fp-0")
    assert (await service.sessao())["estado"] == "executando"
    assert chamadas == [("sessao", False)]


async def test_rodada_grava_assinatura_da_fonte(tmp_path, monkeypatch):
    home = _home(tmp_path)
    service = IntegracaoCodex(home, home / ".codex", nativo=object)
    (home / ".claude/settings.json").write_text('{"enabledPlugins": []}')  # inválido: rodada em erro
    await service.reconciliar()
    registro = json.loads((service.raiz / "estado.json").read_text())
    assert registro["fingerprint"] == service.fingerprint()
    assert registro["status"]["estado"] == "erro"
