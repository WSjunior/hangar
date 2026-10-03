"""CLIProxyAPI local: a chave sai do config dele direto para o engines.json, nunca para a tela."""
import pytest
from fastapi.testclient import TestClient

from app import cliproxy
from app import engines as eng
from app.api import app
from app.config import settings

TOKEN = "secret-de-teste"
AUTH = {"Authorization": f"Bearer {TOKEN}"}
CHAVE = "chave-de-teste-cliproxy"


@pytest.fixture(autouse=True)
def _isola(tmp_path, monkeypatch):
    monkeypatch.setattr(eng, "caminho", lambda: tmp_path / "engines.json")
    monkeypatch.setattr(cliproxy, "config_path", lambda: tmp_path / "config.yaml")
    monkeypatch.setattr(settings, "auth_token", TOKEN)


@pytest.fixture
def cli():
    return TestClient(app)


def _config(tmp_path, texto):
    (tmp_path / "config.yaml").write_text(texto, encoding="utf-8")


def test_local_le_host_porta_e_primeira_chave(tmp_path):
    _config(tmp_path, f'host: ""\nport: 9000\napi-keys:\n  - "{CHAVE}"\n  - outra\n')
    assert cliproxy.local() == {"base_url": "http://127.0.0.1:9000", "api_key": CHAVE}


def test_local_sem_config_e_none(tmp_path):
    assert cliproxy.local() is None


def test_local_sem_chave_levanta(tmp_path):
    _config(tmp_path, "port: 8317\napi-keys: []\n")
    with pytest.raises(ValueError, match="api-keys"):
        cliproxy.local()


def test_local_com_yaml_quebrado_levanta(tmp_path):
    _config(tmp_path, "api-keys: [\n")
    with pytest.raises(ValueError):
        cliproxy.local()


def test_get_devolve_modelos_sem_imagem_e_sem_a_chave(cli, tmp_path, monkeypatch):
    _config(tmp_path, f"host: 127.0.0.1\nport: 8317\napi-keys:\n  - {CHAVE}\n")
    vistos = []

    def _fake(base_url, api_key):
        vistos.append((base_url, api_key))
        return [{"id": "gpt-5.5", "context_length": None, "vision": None},
                {"id": "gpt-image-2", "context_length": None, "vision": None}]

    monkeypatch.setattr("app.engine_probe.listar_modelos", _fake)
    r = cli.get("/api/engines/cliproxy", headers=AUTH)
    assert r.status_code == 200
    assert r.json() == {"found": True, "base_url": "http://127.0.0.1:8317", "error": None,
                        "models": [{"id": "gpt-5.5", "context_length": None, "vision": None}]}
    assert vistos == [("http://127.0.0.1:8317", CHAVE)]
    assert CHAVE not in r.text


def test_get_sem_instancia(cli):
    assert cli.get("/api/engines/cliproxy", headers=AUTH).json() == {
        "found": False, "base_url": None, "models": [], "error": None}


def test_put_com_use_cliproxy_key_grava_a_chave_do_config(cli, tmp_path):
    _config(tmp_path, f"port: 8317\napi-keys:\n  - {CHAVE}\n")
    r = cli.put("/api/engines/cliproxy", headers=AUTH, json={
        "label": "CLIProxyAPI", "base_url": "http://127.0.0.1:8317/v1", "model": "gpt-5.5",
        "use_cliproxy_key": True})
    assert r.status_code == 200, r.text
    assert CHAVE not in r.text
    salvo = eng.listar()["cliproxy"]
    assert salvo["api_key"] == CHAVE
    assert "use_cliproxy_key" not in salvo


def test_put_com_use_cliproxy_key_para_outro_endereco_e_recusado(cli, tmp_path):
    _config(tmp_path, f"port: 8317\napi-keys:\n  - {CHAVE}\n")
    r = cli.put("/api/engines/x", headers=AUTH, json={
        "base_url": "https://outro.example.com", "model": "gpt-5.5", "use_cliproxy_key": True})
    assert r.status_code == 400
    assert "x" not in eng.listar()
