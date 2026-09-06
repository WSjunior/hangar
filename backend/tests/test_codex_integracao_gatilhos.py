"""Gatilhos usam o reconciliador e encerram seu trabalho sem tocar no perfil real."""
import asyncio
import importlib.util
import sys
from importlib.machinery import SourceFileLoader
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock

import pytest

from app import api, codex_integracao


@pytest.mark.parametrize("falha", [False, True])
def test_lancador_reconcilia_antes_de_abrir_servidor_e_tolera_falha(tmp_path, monkeypatch, capsys, falha):
    caminho = Path(__file__).resolve().parents[2] / "scripts" / "hangar-codex-tui"
    loader = SourceFileLoader("lancador_codex_teste", str(caminho))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    lancador = importlib.util.module_from_spec(spec)
    loader.exec_module(lancador)
    ordem = []

    async def reconciliar():
        ordem.append("integracao")
        if falha:
            raise RuntimeError("Falha simulada")
        return {"estado": "parcial", "etapa": "Concluída", "avisos": ["Aviso simulado"],
                "erros": [], "plugins": [], "confianca_pendente": True}

    class AntesDoServidor(Exception):
        pass

    def porta():
        ordem.append("servidor")
        raise AntesDoServidor

    monkeypatch.setattr(codex_integracao, "SERVICO", SimpleNamespace(status=lambda: {"estado": "ocioso"}))
    monkeypatch.setattr(codex_integracao, "antes_da_sessao", reconciliar)
    monkeypatch.setattr(lancador, "_com_websockets", lambda: None)
    monkeypatch.setattr(lancador, "_porta_livre", porta)
    monkeypatch.setattr(sys, "argv", [str(caminho), "--name", "teste", "--cwd", str(tmp_path)])
    monkeypatch.setattr(sys, "path", sys.path.copy())
    with pytest.raises(AntesDoServidor):
        lancador.main()
    assert ordem == ["integracao", "servidor"]
    texto = capsys.readouterr().err
    if falha:
        assert "a sessão abre assim mesmo" in texto
    else:
        assert "Aviso simulado" in texto
        assert "confirmação de confiança" in texto


async def test_lifespan_inicia_acompanhamento_e_aguarda_encerramento(tmp_path, monkeypatch):
    ordem = []

    async def acompanhar():
        ordem.append("inicio")
        try:
            await asyncio.Event().wait()
        finally:
            ordem.append("cancelado")

    async def fechar():
        ordem.append("fechado")

    async def nada(*args):
        pass

    monkeypatch.setattr(codex_integracao, "SERVICO", SimpleNamespace(acompanhar=acompanhar, fechar=fechar))
    monkeypatch.setattr(api, "list_config_dirs", lambda: [])
    monkeypatch.setattr(api, "_backend_config_base", lambda: tmp_path)
    monkeypatch.setattr(api.registry, "list", lambda: [])
    monkeypatch.setattr(api.loop_mod, "_loop_dir", lambda: tmp_path)
    monkeypatch.setattr(api.hook_state, "watch", nada)
    monkeypatch.setattr(api.stall_watch, "watch", nada)
    for nome in ("_renova_token_loop", "_fetch_loop", "_auto_update_loop", "_prune_loop"):
        monkeypatch.setattr(api, nome, nada)
    monkeypatch.setattr(api.pricing, "atualizar_em_background", lambda: None)
    monkeypatch.setattr(api, "threading", SimpleNamespace(Thread=Mock(return_value=SimpleNamespace(start=lambda: None))))
    monkeypatch.setattr(api.INBOX, "ligar_loop", lambda loop: None)
    monkeypatch.setattr(api, "_loop_servidor", None)
    async with api._lifespan(api.app):
        await asyncio.sleep(0)
        assert ordem == ["inicio"]
    assert ordem == ["inicio", "cancelado", "fechado"]
