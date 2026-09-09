"""Configuração oficial numa HOME temporária; nenhum pedido de inferência."""
import json
import shutil
import tomllib

import pytest

from app.codex_integracao import IntegracaoCodex
from app.codex_opcoes import ler_opcoes, salvar_opcoes


@pytest.mark.skipif(not shutil.which("codex"), reason="Codex CLI necessário para validar o escritor TOML")
@pytest.mark.parametrize("anterior", [None, 300000])
async def test_contexto_nativo_preserva_e_restaura_config(tmp_path, anterior):
    codex_home = tmp_path / ".codex"
    codex_home.mkdir()
    path = codex_home / "config.toml"
    path.write_text('# comentário pessoal\nmodel = "gpt-6-astra"\nmodel_auto_compact_token_limit = 250000\n'
                    + (f"model_context_window = {anterior}\n" if anterior else ""))
    service = IntegracaoCodex(home=tmp_path, codex_home=codex_home)
    assert not ler_opcoes(service)["contexto_estendido"]
    assert (await salvar_opcoes(service, True))["contexto_estendido"]
    assert (await salvar_opcoes(service, True))["contexto_estendido"]
    config = tomllib.loads(path.read_text())
    assert config["model_context_window"] == 1000000
    assert config["model_auto_compact_token_limit"] == 900000
    await salvar_opcoes(service, False)
    config = tomllib.loads(path.read_text())
    assert config.get("model_context_window") == anterior
    assert config["model_auto_compact_token_limit"] == 250000
    assert config["model"] == "gpt-6-astra"
    assert "# comentário pessoal" in path.read_text()


@pytest.mark.skipif(not shutil.which("codex"), reason="Codex CLI necessário para validar o escritor TOML")
@pytest.mark.parametrize("compactacao", [900000, 250000])
async def test_contexto_preexistente_desliga_sem_restaurar_um_milhao(tmp_path, compactacao):
    path = tmp_path / "config.toml"
    path.write_text(f"model_context_window = 1000000\nmodel_auto_compact_token_limit = {compactacao}\n")
    service = IntegracaoCodex(home=tmp_path, codex_home=tmp_path)
    if compactacao != 900000:
        await salvar_opcoes(service, True)
        assert tomllib.loads(path.read_text())["model_auto_compact_token_limit"] == 900000
    await salvar_opcoes(service, False)
    config = tomllib.loads(path.read_text())
    assert "model_context_window" not in config
    assert config.get("model_auto_compact_token_limit") == (None if compactacao == 900000 else compactacao)


def test_catalogo_anuncia_teto_real_sem_inventar_um_milhao(tmp_path):
    service = IntegracaoCodex(home=tmp_path, codex_home=tmp_path)
    (tmp_path / "models_cache.json").write_text(json.dumps({"models": [
        {"slug": "gpt-6-astra", "context_window": 272000, "max_context_window": 872000},
        {"slug": "pequeno", "context_window": 128000, "max_context_window": 128000},
        {"slug": "interno", "context_window": 272000, "max_context_window": 872000, "visibility": "hide"},
    ]}))
    assert ler_opcoes(service)["modelos"] == [{"model": "gpt-6-astra", "default": 272000, "max": 872000}]
