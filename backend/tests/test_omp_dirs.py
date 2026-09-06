"""A raiz do agente omp tem UMA resposta: quem lê (sessões, saúde, login) e quem escreve
(plugin sync, contexto) enxergam o mesmo perfil."""
from pathlib import Path

import pytest

from app import harness_saude, oauth_codex, omp_dirs
from app.adapters.pi import sessions
from app.omp_plugin_sync import resolve_omp_directories


@pytest.fixture
def home(tmp_path, monkeypatch):
    monkeypatch.setattr(Path, "home", classmethod(lambda cls: tmp_path))
    for key in ("OMP_PROFILE", "PI_PROFILE", "PI_CONFIG_DIR", "PI_CODING_AGENT_DIR", "XDG_DATA_HOME"):
        monkeypatch.delenv(key, raising=False)
    return tmp_path


def test_sem_perfil_todos_apontam_para_omp_agent(home):
    raiz = home / ".omp" / "agent"
    assert omp_dirs.agent_dir() == raiz
    assert sessions.sessions_root("omp") == raiz / "sessions"
    assert harness_saude._raiz_agente("omp") == raiz
    assert oauth_codex._omp_db(None) == raiz / "agent.db"


def test_com_perfil_leitores_e_escritores_concordam(home, monkeypatch):
    monkeypatch.setenv("OMP_PROFILE", "trabalho")
    raiz = home / ".omp" / "profiles" / "trabalho" / "agent"
    escritor = resolve_omp_directories(home, {"OMP_PROFILE": "trabalho"}, home).agent_dir
    assert escritor == raiz
    assert sessions.sessions_root("omp") == raiz / "sessions"
    assert harness_saude._raiz_agente("omp") == raiz
    assert oauth_codex._omp_db(None) == raiz / "agent.db"


def test_pi_coding_agent_dir_continua_valendo_sem_perfil(home, monkeypatch):
    monkeypatch.setenv("PI_CODING_AGENT_DIR", str(home / "custom"))
    assert sessions.sessions_root("omp") == home / "custom" / "sessions"
    assert harness_saude._raiz_agente("omp") == home / "custom"


def test_perfil_invalido_nao_derruba_quem_le(home, monkeypatch, caplog):
    monkeypatch.setenv("OMP_PROFILE", "Nome Inválido")
    with caplog.at_level("WARNING", logger="hangar.omp_dirs"):
        assert sessions.sessions_root("omp") == home / ".omp" / "agent" / "sessions"
    assert "perfil do omp ignorado" in caplog.text


def test_home_explicita_e_raiz_fixa(home, monkeypatch):
    monkeypatch.setenv("OMP_PROFILE", "trabalho")
    outra = home / "outra"
    assert oauth_codex._omp_db(outra) == outra / ".omp" / "agent" / "agent.db"
