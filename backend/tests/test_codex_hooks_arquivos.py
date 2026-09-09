"""O importador nativo do Codex copia hook que é ARQUIVO e pula hook que é SYMLINK — mas reescreve
os dois comandos pra `<codex>/hooks/`. Medido em 07/09/2026 (codex-cli 0.153.4) numa HOME
descartável: `real.sh` foi copiado, `linkado.sh` não, e o hook quebrado bloqueava a ferramenta
("python3: can't open file ... No such file or directory") porque um PreToolUse que falha barra a
chamada.
"""
import json
import os

import pytest

from app import codex_hooks_arquivos as mod


def _doc(*comandos: str) -> dict:
    return {"hooks": {"PreToolUse": [{"hooks": [{"type": "command", "command": c}] } for c in comandos]}}


def test_materializa_o_que_o_importador_pulou(tmp_path):
    home, codex = tmp_path / "home", tmp_path / "home" / ".codex"
    (home / ".claude" / "hooks").mkdir(parents=True)
    (codex / "hooks").mkdir(parents=True)
    alvo = tmp_path / "repo" / "hooks" / "linkado.py"
    alvo.parent.mkdir(parents=True)
    alvo.write_text("print('alvo')\n")
    (home / ".claude" / "hooks" / "linkado.py").symlink_to(alvo)
    (codex / "hooks" / "real.sh").write_text("echo real\n")     # o importador copiou este
    doc = _doc(f"sh '{codex}/hooks/real.sh'", f"python3 '{codex}/hooks/linkado.py'")
    (codex / "hooks.json").write_text(json.dumps(doc))

    criados, orfaos = mod.materializar(codex, home)

    assert criados == ["linkado.py"] and orfaos == []
    destino = codex / "hooks" / "linkado.py"
    assert destino.exists() and destino.read_text() == "print('alvo')\n"
    # Symlink pro arquivo REAL, não pro link do meio: uma fonte só, e editar o script continua
    # valendo pros dois agentes sem cópia pra envelhecer.
    if os.name != "nt":
        assert destino.is_symlink() and destino.resolve() == alvo.resolve()
    # O que já existia não é tocado.
    assert (codex / "hooks" / "real.sh").read_text() == "echo real\n"


def test_sem_equivalente_no_claude_vira_aviso_e_nao_arquivo_vazio(tmp_path):
    home, codex = tmp_path / "home", tmp_path / "home" / ".codex"
    (home / ".claude" / "hooks").mkdir(parents=True)
    (codex / "hooks").mkdir(parents=True)
    (codex / "hooks.json").write_text(json.dumps(_doc(f"python3 '{codex}/hooks/sumiu.py'")))

    criados, orfaos = mod.materializar(codex, home)

    assert criados == [] and orfaos == ["sumiu.py"]
    assert not (codex / "hooks" / "sumiu.py").exists()   # nunca um arquivo vazio no lugar


def test_ignora_caminho_fora_da_pasta_de_hooks_do_codex(tmp_path):
    """Comando que aponta pro repo (o caso dos hooks do próprio Hangar) não é assunto desta etapa —
    ela só preenche o que o importador prometeu em `<codex>/hooks/`."""
    home, codex = tmp_path / "home", tmp_path / "home" / ".codex"
    (home / ".claude" / "hooks").mkdir(parents=True)
    codex.mkdir(parents=True)
    doc = _doc(f"python3 '{tmp_path}/repo/backend/hooks/state.py'")
    assert mod.faltantes(doc, codex) == []
    assert mod.materializar(codex, home, doc) == ([], [])


def test_citacao_com_dotdot_nao_escapa_da_pasta_de_hooks(tmp_path):
    """`<codex>/hooks/../../fora.py` não é um hook do Codex — e o caminho que entra na lista é o
    RESOLVIDO, não o cru, pra a escrita não depender de o SO normalizar o `..` de novo."""
    home, codex = tmp_path / "home", tmp_path / "home" / ".codex"
    (home / ".claude" / "hooks").mkdir(parents=True)
    (codex / "hooks").mkdir(parents=True)
    doc = _doc(f"python3 '{codex}/hooks/../../fora.py'")
    assert mod.faltantes(doc, codex) == []
    dentro = _doc(f"python3 '{codex}/hooks/./x.py'")
    assert mod.faltantes(dentro, codex) == [(codex / "hooks" / "x.py").resolve()]


def test_windows_refaz_a_copia_que_ficou_velha(tmp_path, monkeypatch):
    """No Windows o hook é CÓPIA, e cópia envelhece: sem isto, editar o hook em ~/.claude/hooks
    nunca chegava ao Codex — calado. Compara bytes, não mtime (cópia e original nascem com datas
    diferentes)."""
    # `mod._WINDOWS`, não `os.name`: trocar `os.name` num teste leva o `pathlib` junto e o caso
    # estoura no andaime, não no código (está registrado no CLAUDE.md).
    monkeypatch.setattr(mod, "_WINDOWS", True)
    home, codex = tmp_path / "home", tmp_path / "home" / ".codex"
    (home / ".claude" / "hooks").mkdir(parents=True)
    (codex / "hooks").mkdir(parents=True)
    (home / ".claude" / "hooks" / "x.py").write_text("novo\n")
    (codex / "hooks" / "x.py").write_text("velho\n")
    doc = _doc(f"python3 '{codex}/hooks/x.py'")

    criados, orfaos = mod.materializar(codex, home, doc)

    assert criados == ["x.py"] and orfaos == []
    assert (codex / "hooks" / "x.py").read_text() == "novo\n"
    # Conteúdo igual não reescreve (senão toda reconciliação mexeria no arquivo à toa).
    assert mod.materializar(codex, home, doc) == ([], [])


@pytest.mark.skipif(os.name == "nt", reason="symlink pendurado exige privilégio no Windows")
def test_link_pendurado_e_refeito(tmp_path):
    home, codex = tmp_path / "home", tmp_path / "home" / ".codex"
    (home / ".claude" / "hooks").mkdir(parents=True)
    (codex / "hooks").mkdir(parents=True)
    alvo = tmp_path / "repo" / "x.py"
    alvo.parent.mkdir(parents=True)
    alvo.write_text("ok\n")
    (home / ".claude" / "hooks" / "x.py").symlink_to(alvo)
    (codex / "hooks" / "x.py").symlink_to(tmp_path / "nao-existe")   # link morto de uma rodada velha

    criados, orfaos = mod.materializar(codex, home, _doc(f"python3 '{codex}/hooks/x.py'"))

    assert criados == ["x.py"] and orfaos == []
    assert (codex / "hooks" / "x.py").read_text() == "ok\n"


@pytest.mark.parametrize("windows", [False, True])
def test_materializa_subpastas_sem_confundir_arquivos_homonimos(tmp_path, monkeypatch, windows):
    if not windows and os.name == "nt":
        pytest.skip("symlink exige privilégio no Windows")
    monkeypatch.setattr(mod, "_WINDOWS", windows)
    home, codex = tmp_path / "home", tmp_path / "home" / ".codex"
    origem = home / ".claude" / "hooks"
    for pasta in ("gitnexus", "outro"):
        fonte = origem / pasta / "hook.cjs"
        fonte.parent.mkdir(parents=True)
        fonte.write_text(pasta)
    (origem / "hook.cjs").write_text("não é este arquivo")
    destinos = [codex / "hooks" / pasta / "hook.cjs" for pasta in ("gitnexus", "outro")]
    doc = _doc(*(f"node '{p}'" for p in destinos))

    assert mod.faltantes(doc, codex) == destinos
    assert mod.materializar(codex, home, doc) == (["gitnexus/hook.cjs", "outro/hook.cjs"], [])
    for pasta, destino in zip(("gitnexus", "outro"), destinos):
        assert destino.read_text() == pasta
        assert destino.is_symlink() is (not windows)
    assert mod.materializar(codex, home, doc) == ([], [])

    (origem / "gitnexus" / "hook.cjs").write_text("atualizado")
    esperado = (["gitnexus/hook.cjs"], []) if windows else ([], [])
    assert mod.materializar(codex, home, doc) == esperado
    assert destinos[0].read_text() == "atualizado"
    assert destinos[1].read_text() == "outro"


def test_subpasta_sem_fonte_informa_caminho_relativo(tmp_path):
    home, codex = tmp_path / "home", tmp_path / "home" / ".codex"
    origem = home / ".claude" / "hooks"
    origem.mkdir(parents=True)
    (origem / "hook.cjs").write_text("homônimo da raiz")
    destino = codex / "hooks" / "gitnexus" / "hook.cjs"

    assert mod.materializar(codex, home, _doc(f"node '{destino}'")) == ([], ["gitnexus/hook.cjs"])
    assert not destino.exists()


@pytest.mark.skipif(os.name == "nt", reason="symlink exige privilégio no Windows")
def test_subpasta_linkada_para_fora_nao_autoriza_escrita(tmp_path):
    home, codex = tmp_path / "home", tmp_path / "home" / ".codex"
    origem = home / ".claude" / "hooks" / "gitnexus"
    origem.mkdir(parents=True)
    (origem / "hook.cjs").write_text("fonte")
    (codex / "hooks").mkdir(parents=True)
    fora = tmp_path / "fora"
    fora.mkdir()
    (codex / "hooks" / "gitnexus").symlink_to(fora, target_is_directory=True)
    doc = _doc(f"node '{codex}/hooks/gitnexus/hook.cjs'")

    assert mod.faltantes(doc, codex) == []
    assert mod.materializar(codex, home, doc) == ([], [])
    assert not (fora / "hook.cjs").exists()
