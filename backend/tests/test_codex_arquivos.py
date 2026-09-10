import asyncio
import json
from pathlib import Path

import pytest

from app import codex_arquivos
from app.codex_arquivos import (
    AlteradoExternamente, editar_config, exclusivo, gravar, json_obj, mesclar_hooks, transformar,
)


def test_snapshot_externo_e_preservado(tmp_path):
    path = tmp_path / "hooks.json"
    path.write_bytes(b"externo")
    with pytest.raises(AlteradoExternamente):
        gravar(path, b"nosso", b"anterior", tmp_path / "backup")
    assert path.read_bytes() == b"externo"
    assert not list(tmp_path.glob(".*.hangar-*"))


def test_transformacao_rele_snapshot_apos_alteracao_externa(tmp_path):
    path = tmp_path / "arquivo"
    path.write_bytes(b"a")
    vistos = []

    def fn(raw):
        vistos.append(raw)
        if len(vistos) == 1:
            path.write_bytes(b"externo")
        return raw + b" atualizado"

    transformar(path, fn, tmp_path / "backup")
    assert vistos == [b"a", b"externo"]
    assert path.read_bytes() == b"externo atualizado"
    backup = json_obj(next((tmp_path / "backup").glob("*.json")))
    assert bytes.fromhex(backup["conteudo_hex"]) == b"externo"


def test_idempotencia_preserva_mtime_e_backup_original(tmp_path):
    path = tmp_path / "config"
    path.write_bytes(b"original")
    backups = tmp_path / "backup"
    assert gravar(path, b"novo", b"original", backups)
    mtime = path.stat().st_mtime_ns
    assert not gravar(path, b"novo", b"novo", backups)
    assert path.stat().st_mtime_ns == mtime
    gravar(path, b"outro", b"novo", backups)
    assert len(list(backups.iterdir())) == 1
    assert bytes.fromhex(json.loads(next(backups.iterdir()).read_text())["conteudo_hex"]) == b"original"


def test_persona_symlink_nao_altera_fonte(tmp_path):
    fonte = tmp_path / "CLAUDE.md"
    fonte.write_bytes(b"claude")
    path = tmp_path / "AGENTS.md"
    try:
        path.symlink_to(fonte)
    except OSError:
        pytest.skip("Symlink indisponível nesta máquina")
    gravar(path, b"codex", b"claude", tmp_path / "backup")
    assert fonte.read_bytes() == b"claude"
    assert path.read_bytes() == b"codex"
    assert not path.is_symlink()


def test_mescla_hooks_remove_gerenciado_antigo_preserva_exclusivo_e_matcher():
    def grupo(cmd, matcher="Bash"):
        return {"matcher": matcher, "hooks": [{"type": "command", "command": cmd}]}
    atual = {"description": "próprio", "hooks": {"PreToolUse": [grupo("antigo"), grupo("exclusivo", "Write")]}}
    fonte = {"hooks": {"PreToolUse": [grupo("novo")]}}
    anterior = {"hooks": {"PreToolUse": [grupo("antigo")]}}
    out = mesclar_hooks(atual, fonte, anterior)
    assert out == {"description": "próprio", "hooks": {"PreToolUse": [grupo("exclusivo", "Write"), grupo("novo")]}}
    assert atual["hooks"]["PreToolUse"][0] == grupo("antigo")
    assert mesclar_hooks(out, fonte, fonte) == out


async def test_lock_serializa_e_cancelamento_nao_prende(tmp_path):
    path = tmp_path / "lock"
    ordem = []

    async def segundo():
        async with exclusivo(path):
            ordem.append("segundo")

    async with exclusivo(path):
        task = asyncio.create_task(segundo())
        await asyncio.sleep(0.02)
        assert ordem == []
        task.cancel()
        with pytest.raises(asyncio.CancelledError):
            await task
        ordem.append("primeiro")
    await asyncio.wait_for(segundo(), 1)
    assert ordem == ["primeiro", "segundo"]


class _NativoConfig:
    def __init__(self, home, codex_home, binario="codex"):
        self.codex_home = codex_home
        self.binario = binario

    async def __aenter__(self):
        return self

    async def __aexit__(self, *args):
        return None

    async def request(self, method, params):
        assert method == "config/batchWrite"
        self.codex_home.joinpath("config.toml").write_bytes(b'model = "novo"\n')
        return {}


async def test_editar_config_usa_copia_nativa_e_confirma_depois_da_troca(tmp_path):
    path = tmp_path / "config.toml"
    path.write_bytes(b'model = "antigo"\n')
    confirmado = []

    await editar_config(
        path, tmp_path / "backups", tmp_path / "work", _NativoConfig,
        lambda atual: ([{"keyPath": '"model"', "value": "novo", "mergeStrategy": "replace"}],
                       lambda: confirmado.append(True)),
        binario="codex-teste",
    )

    assert path.read_bytes() == b'model = "novo"\n'
    assert confirmado == [True]


async def test_editar_config_preserva_a_causa_da_colisao_final(tmp_path, monkeypatch):
    path = tmp_path / "config.toml"
    path.write_bytes(b'model = "antigo"\n')
    original = codex_arquivos.gravar

    def colidir(destino, data, esperado, backups=None):
        if destino == path:
            path.write_bytes(b'model = "externo"\n')
            raise AlteradoExternamente("causa original")
        return original(destino, data, esperado, backups)

    monkeypatch.setattr(codex_arquivos, "gravar", colidir)
    with pytest.raises(AlteradoExternamente) as erro:
        await editar_config(
            path, tmp_path / "backups", tmp_path / "work", _NativoConfig,
            lambda atual: ([{"keyPath": '"model"', "value": "novo", "mergeStrategy": "replace"}],
                           lambda: None),
        )

    assert isinstance(erro.value.__cause__, AlteradoExternamente)
    assert "continua mudando" in str(erro.value)
    assert path.read_bytes() == b'model = "externo"\n'
