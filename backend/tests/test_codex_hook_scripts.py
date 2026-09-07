"""Executa os scripts importados; um comando válido não comprova que o arquivo foi publicado."""
import json
import os
import shlex
import shutil
import subprocess
import sys

import pytest

from app.codex_integracao import IntegracaoCodex

pytestmark = [pytest.mark.integration, pytest.mark.skipif(
    os.environ.get("RUN_CODEX_INTEGRATION") != "1" or not shutil.which("codex"),
    reason="Exige Codex CLI e RUN_CODEX_INTEGRATION=1; somente HOME temporário",
)]


async def test_importa_script_symlink_dependencia_e_atualizacao(tmp_path):
    home = tmp_path / "home"
    hooks = home / ".claude/hooks"
    hooks.mkdir(parents=True)
    (home / ".codex").mkdir()
    original = tmp_path / "guard.py"
    original.write_text("from auxiliar import mensagem\nprint(mensagem)\n", encoding="utf-8")
    (hooks / "guard.py").symlink_to(original)
    (hooks / "auxiliar.py").write_text("mensagem = 'primeiro'\n", encoding="utf-8")
    command = shlex.join([sys.executable, str(hooks / "guard.py")])
    (home / ".claude/settings.json").write_text(json.dumps({"hooks": {"PreToolUse": [
        {"matcher": "Bash", "hooks": [{"type": "command", "command": command}]},
    ]}}))
    service = IntegracaoCodex(home, home / ".codex")
    result = await service.reconciliar()
    assert result["estado"] == "ok", result
    def executar():
        data = json.loads((home / ".codex/hooks.json").read_text())
        comando = data["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
        return subprocess.run(comando, shell=True, input=b'{}', capture_output=True, timeout=5)
    r = executar()
    assert r.returncode == 0, r.stderr
    assert r.stdout.strip() == b"primeiro"
    (hooks / "auxiliar.py").write_text("mensagem = 'segundo'\n", encoding="utf-8")
    assert service.precisa_reconciliar()
    assert (await service.reconciliar())["estado"] == "ok"
    assert executar().stdout.strip() == b"segundo"
    (home / ".codex/hooks/guard.py").unlink()
    assert service.precisa_reconciliar()
    assert (await service.reconciliar())["estado"] == "ok"
    assert executar().returncode == 0


@pytest.mark.skipif(os.name == "nt", reason="Permissão de execução POSIX")
async def test_script_executavel_importado_preserva_execucao_e_arquivo_exclusivo(tmp_path):
    hooks = tmp_path / ".claude/hooks"
    hooks.mkdir(parents=True)
    destino = tmp_path / ".codex/hooks"
    destino.mkdir(parents=True)
    (destino / "pessoal").write_text("conteúdo exclusivo")
    script = hooks / "verificar"
    script.write_text("#!/bin/sh\nprintf aprovado\n")
    script.chmod(0o700)
    (tmp_path / ".claude/settings.json").write_text(json.dumps({"hooks": {"SessionStart": [
        {"hooks": [{"type": "command", "command": shlex.quote(str(script))}]},
    ]}}))
    service = IntegracaoCodex(tmp_path, tmp_path / ".codex")
    assert (await service.reconciliar())["estado"] == "ok"
    assert subprocess.check_output([str(destino / "verificar")]) == b"aprovado"
    assert (destino / "pessoal").read_text() == "conteúdo exclusivo"


async def test_colisao_de_script_preserva_arquivo_e_configuracao_anterior(tmp_path):
    origem = tmp_path / ".claude/hooks"
    destino = tmp_path / ".codex/hooks"
    origem.mkdir(parents=True)
    destino.mkdir(parents=True)
    (origem / "guarda.py").write_text("print('fonte')\n")
    (destino / "guarda.py").write_text("print('pessoal')\n")
    velho = {"hooks": {"PreToolUse": [{"hooks": [{"type": "command", "command": "echo pessoal"}]}]}}
    hooks_json = tmp_path / ".codex/hooks.json"
    hooks_json.write_text(json.dumps(velho))
    (tmp_path / ".claude/settings.json").write_text(json.dumps({"hooks": {"PreToolUse": [
        {"hooks": [{"type": "command", "command": shlex.join([sys.executable, str(origem / "guarda.py")])}]},
    ]}}))
    service = IntegracaoCodex(tmp_path, tmp_path / ".codex")
    result = await service.reconciliar()
    assert result["estado"] == "erro"
    assert any(a["codigo"] == "aviso_artefato_sem_proveniencia" for a in result["avisos"])
    assert json.loads(hooks_json.read_text()) == velho
    assert (destino / "guarda.py").read_text() == "print('pessoal')\n"
