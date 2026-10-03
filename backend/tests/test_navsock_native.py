"""Tela remota do navegador servida pelo app nativo: o `navsock` fala com o `/cdp` dele em vez da 9223."""
import asyncio
import json

import pytest
import websockets

from app import navshell, navsock

SIDECAR = {"chave": "srv1::nav-teste", "url": "http://x/", "targetId": None, "pid": 123}


class _Tela:
    """O `WebSocket` do celular, só com o que o fluxo de quadros usa."""

    def __init__(self):
        self.enviados: list[dict] = []

    async def send_text(self, texto: str) -> None:
        self.enviados.append(json.loads(texto))


def _srv(monkeypatch, porta: int, pid: int = 123) -> None:
    monkeypatch.setattr(navshell, "_servidor", lambda: {"porta": porta, "token": "tok", "pid": pid})


def test_conexao_escolhe_o_dono_do_navegador(monkeypatch):
    eletron = navsock._conexao({"chave": "s::a", "targetId": "ABC"})
    assert eletron and eletron._url == "ws://127.0.0.1:9223/devtools/page/ABC" and eletron._headers is None

    _srv(monkeypatch, 4567)
    nativo = navsock._conexao(SIDECAR)
    assert nativo and nativo._url == "ws://127.0.0.1:4567/cdp?chave=srv1%3A%3Anav-teste"
    assert nativo._headers == {"Authorization": "Bearer tok"}

    # O `_srv.json` é de outro processo (o Electron subiu depois): o token dele não abre este navegador.
    _srv(monkeypatch, 4567, pid=999)
    assert navsock._conexao(SIDECAR) is None

    def _sem_app():
        raise navshell.ShellIndisponivel("fechado")
    monkeypatch.setattr(navshell, "_servidor", _sem_app)
    assert navsock._conexao(SIDECAR) is None
    assert navsock._conexao({"chave": "s::a", "targetId": None}) is None


async def test_quadros_entrada_e_fim_pelo_repasse_do_nativo(monkeypatch):
    recebidos: list[dict] = []
    cabecalhos: dict = {}
    pronto = asyncio.Event()

    async def nativo(ws):
        cabecalhos["Authorization"] = ws.request.headers.get("Authorization")
        cabecalhos["path"] = ws.request.path
        async for bruto in ws:
            msg = json.loads(bruto)
            recebidos.append(msg)
            await ws.send(json.dumps({"id": msg["id"], "result": {}}))
            if msg["method"] == "Page.startScreencast":
                await ws.send(json.dumps({"method": "Page.screencastFrame", "params": {
                    "data": "QUJD", "sessionId": 7, "metadata": {"deviceWidth": 390, "deviceHeight": 844}}}))
                await ws.send(json.dumps({"method": "Page.frameNavigated",
                                          "params": {"frame": {"url": "http://y/"}}}))
            if msg["method"] == "Input.dispatchMouseEvent":
                pronto.set()
                await ws.send(json.dumps({"method": "Inspector.detached",
                                          "params": {"reason": "replaced_by_another_viewer"}}))

    async with websockets.serve(nativo, "127.0.0.1", 0) as server:
        _srv(monkeypatch, server.sockets[0].getsockname()[1])
        conexao = navsock._conexao(SIDECAR)
        assert conexao
        tela = _Tela()
        estado = {"w": 1280, "h": 800, "pedida": 390}
        async with conexao as cdp:
            assert await navsock._url_atual(cdp, SIDECAR) == "http://x/"
            fluxo = asyncio.create_task(navsock._fluxo_de_quadros(cdp, tela, estado))
            while not any(m.get("t") == "url" for m in tela.enviados):
                await asyncio.sleep(0.01)
            await navsock._aplicar_entrada(cdp, {"t": "m", "acao": "press", "x": 0.5, "y": 0.5},
                                           estado["w"], estado["h"])
            await asyncio.wait_for(pronto.wait(), 5)
            with pytest.raises(ConnectionError, match="outro aparelho"):
                await asyncio.wait_for(fluxo, 5)

    assert cabecalhos["Authorization"] == "Bearer tok"
    assert cabecalhos["path"] == "/cdp?chave=srv1%3A%3Anav-teste"
    metodos = [m["method"] for m in recebidos]
    # Sem `Runtime.evaluate`: o repasse do nativo não o abre, e a url vem do sidecar.
    assert "Runtime.evaluate" not in metodos
    assert metodos[:2] == ["Page.enable", "Page.startScreencast"]
    ack = next(m for m in recebidos if m["method"] == "Page.screencastFrameAck")
    assert ack["params"] == {"sessionId": 7}
    clique = next(m for m in recebidos if m["method"] == "Input.dispatchMouseEvent")
    assert (clique["params"]["x"], clique["params"]["y"]) == (195, 422)
    assert {"t": "q", "d": "QUJD", "w": 390, "h": 844} in tela.enviados
    assert {"t": "url", "url": "http://y/"} in tela.enviados


class _CdpFalso:
    def __init__(self):
        self.enviados: list[tuple[str, dict]] = []

    def envia(self, metodo: str, params: dict) -> None:
        self.enviados.append((metodo, params))


async def test_tecla_nomeada_leva_codigo_virtual():
    cdp = _CdpFalso()
    await navsock._aplicar_entrada(cdp, {"t": "k", "key": "Backspace"}, 390, 844)
    await navsock._aplicar_entrada(cdp, {"t": "k", "key": "Enter"}, 390, 844)
    (_, desce), (_, sobe), (_, enter), (_, enter_sobe) = cdp.enviados
    assert (desce["type"], desce["windowsVirtualKeyCode"], desce["code"]) == ("keyDown", 8, "Backspace")
    assert sobe["type"] == "keyUp" and "text" not in sobe
    # Enter leva `\r` só na descida: é o que faz o formulário enviar.
    assert (enter["windowsVirtualKeyCode"], enter["text"]) == (13, "\r")
    assert "text" not in enter_sobe
