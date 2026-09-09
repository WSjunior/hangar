"""Identidades nativas só são associadas com prova da origem, nunca apenas pelo nome."""
import json

import pytest

from app.codex_integracao import IntegracaoCodex, _identidade_plugin, _origem_marketplace


def test_alias_git_aceita_https_e_ssh_do_mesmo_repositorio():
    origem = _origem_marketplace({"source": {"source": "github", "repo": "thedotmack/claude-mem"}}, claude=True)
    mercados = {"claude-mem-local": {"source_type": "git", "source": "git@github.com:thedotmack/claude-mem.git"}}
    assert _identidade_plugin("claude-mem@thedotmack", origem, mercados, {"claude-mem@claude-mem-local": {}}) == "claude-mem@claude-mem-local"


def test_nome_igual_em_repositorio_diferente_nao_e_alias():
    mercados = {"outro": {"source_type": "git", "source": "https://github.com/terceiro/claude-mem"}}
    assert _identidade_plugin("claude-mem@thedotmack", ("git", "github.com/thedotmack/claude-mem"), mercados,
                             {"claude-mem@outro": {}}) == "claude-mem@thedotmack"


def test_alias_ambiguo_nao_escolhe_instalacao_arbitraria():
    origem = {"source_type": "git", "source": "https://github.com/thedotmack/claude-mem"}
    with pytest.raises(ValueError, match="Mais de uma"):
        _identidade_plugin("claude-mem@thedotmack", ("git", "github.com/thedotmack/claude-mem"),
                           {"a": origem, "b": origem}, {"claude-mem@a": {}, "claude-mem@b": {}})


def test_alias_preexistente_vence_cadastro_fonte_sem_plugin_instalado():
    origem = {"source_type": "git", "source": "https://github.com/thedotmack/claude-mem"}
    assert _identidade_plugin("claude-mem@thedotmack", ("git", "github.com/thedotmack/claude-mem"),
                             {"thedotmack": origem, "nativo": origem}, {"claude-mem@nativo": {}}) == "claude-mem@nativo"


def test_skill_so_dispensa_ponte_quando_alias_nativo_esta_habilitado(tmp_path, monkeypatch):
    from app import skill_bridge
    monkeypatch.setattr(skill_bridge, "_REPO", tmp_path / "repo")
    service = IntegracaoCodex(tmp_path, tmp_path / ".codex")
    fonte = tmp_path / ".claude/plugins/cache/thedotmack/claude-mem/1/skills/mem-search"
    nativo = service.codex_home / "plugins/cache/claude-mem-local/claude-mem/1"
    for skill in (fonte, nativo / "skills/mem-search"):
        skill.mkdir(parents=True)
        (skill / "SKILL.md").write_text("---\nname: mem-search\n---\nBuscar memória", encoding="utf-8")
    id_ = "claude-mem@thedotmack"
    registro = {"plugins": {id_: {"id_codex": "claude-mem@claude-mem-local", "path": str(nativo)}}}
    service._plugins_confirmados = {id_}
    service._estado = {"avisos": []}
    config = service.codex_home / "config.toml"
    config.write_text('[plugins."claude-mem@claude-mem-local"]\nenabled = true\n')
    service._skills(registro)
    assert registro["skills"]["mem-search"]["mode"] == "native"
    assert not (service.codex_home / "skills/mem-search").exists()
    config.write_text('[plugins."claude-mem@claude-mem-local"]\nenabled = false\n')
    service._skills(registro)
    assert registro["skills"]["mem-search"]["mode"] == "symlink"
    assert (service.codex_home / "skills/mem-search").resolve() == fonte


@pytest.mark.parametrize("preinstalado", [False, True])
async def test_reconciliacao_preserva_clone_esparso_de_plugin_nativo(tmp_path, monkeypatch, preinstalado):
    home = tmp_path
    codex_home = home / ".codex"
    codex_home.mkdir()
    fonte = home / ".claude/plugins"
    fonte.mkdir(parents=True)
    (fonte.parent / "settings.json").write_text('{"enabledPlugins":{"plugin@fonte":true}}')
    (fonte / "known_marketplaces.json").write_text(json.dumps({
        "fonte": {"source": {"source": "github", "repo": "exemplo/mercado"}},
    }))
    config = codex_home / "config.toml"
    config.write_text('[marketplaces.nativo]\nsource_type="git"\n'
                      'source="https://github.com/exemplo/mercado.git"\n'
                      'sparse_paths=[".agents", "plugin"]\n')
    antes = config.read_bytes()
    cache = codex_home / "plugins/cache/nativo/plugin/1"
    cache.mkdir(parents=True)
    service = IntegracaoCodex(home, codex_home)
    service.raiz.mkdir(parents=True)
    service._estado = service.status()
    registro = {}

    class Nativo:
        instalado = preinstalado

        async def detectar(self):
            return [{"itemType": "PLUGINS", "details": {"plugins": [
                {"marketplaceName": "fonte", "pluginNames": ["plugin"]},
            ]}}]

        async def plugins_instalados(self):
            return [{"pluginId": "plugin@nativo", "version": "1"}] if self.instalado else []

        async def importar(self, itens):
            if self.instalado:
                return {"itemTypeResults": [{"failures": [{
                    "message": "marketplace is already added from a different source",
                }]}]}
            self.instalado = True
            return {}

        async def atualizar_marketplace(self, nome):
            return {"errors": []}

        async def instalar_plugin(self, nome):
            return {"installedPath": str(cache), "version": "1"}

    async def habilitar(*args):
        pass

    monkeypatch.setattr(service, "_habilitar_plugins", habilitar)
    await service._plugins(Nativo(), {"plugin@fonte"}, registro, True)

    assert service.status()["erros"] == []
    assert registro["plugins"]["plugin@fonte"]["id_codex"] == "plugin@nativo"
    assert registro["plugins_pendentes"] == []
    assert config.read_bytes() == antes
