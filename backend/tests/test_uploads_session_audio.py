"""`?arquivo=` com caminho absoluto: só dentro da pasta de uploads do MESMO projeto."""
import os
from pathlib import Path

import pytest

from app import uploads
from app.uploads import UploadError, resolve_session_audio


@pytest.fixture
def proj(tmp_path, monkeypatch):
    monkeypatch.setattr(uploads, "_raiz", lambda: tmp_path / "uploads")
    cwd = tmp_path / "proj"
    cwd.mkdir()
    return str(cwd)


def _status(cwd, ref, allow_absolute=True):
    with pytest.raises(UploadError) as ei:
        resolve_session_audio(cwd, "cc", ref, allow_absolute=allow_absolute)
    return ei.value.status


def test_nome_solto_continua_na_pasta_da_sessao(proj):
    salvo = uploads.save_upload(proj, "cc", b"a", "a.webm")
    assert resolve_session_audio(proj, "cc", Path(salvo).name, allow_absolute=False) == salvo


def test_caminho_de_outra_pasta_de_sessao_do_mesmo_projeto(proj):
    # Depois de um /clear o áudio ficou na pasta do transcript anterior.
    antigo = uploads.save_upload(proj, "transcript-anterior", b"a", "a.webm")
    assert resolve_session_audio(proj, "cc", antigo, allow_absolute=True) == antigo


def test_convidado_nao_usa_caminho_absoluto(proj):
    antigo = uploads.save_upload(proj, "transcript-anterior", b"a", "a.webm")
    assert _status(proj, antigo, allow_absolute=False) == 403


def test_outro_projeto_e_recusado(proj, tmp_path):
    outro = tmp_path / "outro-proj"
    outro.mkdir()
    alheio = uploads.save_upload(str(outro), "cc", b"a", "a.webm")
    assert _status(proj, alheio) == 400


@pytest.mark.parametrize("ref", [
    "\\\\servidor\\share\\a.webm",       # UNC do Windows
    "\\\\?\\C:\\Users\\x\\a.webm",       # prefixo de caminho longo do Windows
    "//servidor/share/a.webm",           # UNC com barra normal
])
def test_caminho_de_rede_e_recusado_antes_de_tocar_no_disco(proj, monkeypatch, ref):
    # A recusa é por texto, igual em qualquer sistema; o realpath nem pode ser chamado.
    monkeypatch.setattr(uploads.os.path, "realpath",
                        lambda *a: pytest.fail("realpath num caminho de rede"))
    with pytest.raises(UploadError) as ei:
        resolve_session_audio(proj, "cc", ref, allow_absolute=True)
    assert (ei.value.status, ei.value.detail) == (400, "caminho de rede recusado")


def test_fora_do_projeto_e_recusado_antes_do_realpath(proj, tmp_path, monkeypatch):
    fora = tmp_path / "segredo.txt"
    fora.write_text("nao", encoding="utf-8")
    real = os.path.realpath
    monkeypatch.setattr(uploads.os.path, "realpath",
                        lambda p, *a, **k: pytest.fail(f"realpath em {p}") if str(p) == str(fora) else real(p, *a, **k))
    assert _status(proj, str(fora)) == 400


@pytest.mark.skipif(os.name == "nt", reason="symlink exige privilégio no Windows")
def test_travessia_link_e_diretorio_sao_recusados(proj, tmp_path):
    proprio = Path(uploads.save_upload(proj, "cc", b"a", "a.webm"))
    fora = tmp_path / "segredo.txt"
    fora.write_text("nao", encoding="utf-8")
    link = proprio.parent / "link.webm"
    link.symlink_to(fora)
    (proprio.parent / "pasta.webm").mkdir()
    for ref in (f"{proprio.parent}/../../../segredo.txt",   # `..` no caminho
                str(link),                                    # link que sai da raiz de uploads
                str(proprio.parent / "pasta.webm"),           # não é arquivo regular
                str(proprio.parent.parent / "x.webm")):       # direto na pasta do projeto
        assert _status(proj, ref) == 400, ref


@pytest.mark.parametrize("absoluto", [True, False])
def test_nul_no_caminho_e_400(proj, absoluto):
    pasta = Path(uploads.save_upload(proj, "cc", b"a", "a.webm")).parent
    ref = f"{pasta}/a\x00.webm" if absoluto else "a\x00.webm"
    assert _status(proj, ref) == 400


def test_inexistente_na_pasta_do_projeto_e_404(proj):
    pasta = Path(uploads.save_upload(proj, "cc", b"a", "a.webm")).parent
    assert _status(proj, str(pasta / "sumiu.webm")) == 404
