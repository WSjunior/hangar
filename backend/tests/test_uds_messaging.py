import json
import os
import socket
import threading

import pytest

from app import uds_messaging


def test_separar_prefixo():
    assert uds_messaging.separar_prefixo("[de: hangar] oi") == ("hangar", "oi")
    assert uds_messaging.separar_prefixo("[grupo: x] aviso") == ("x", "aviso")
    assert uds_messaging.separar_prefixo("fala da pessoa") == (None, "fala da pessoa")


def test_socket_da_sessao_le_o_registro_do_cli(tmp_path, monkeypatch):
    # O CLI grava `<config>/sessions/<pid>.json` com sessionId e messagingSocketPath; e por ele
    # (nao pelo pane) que a sessao sem terminal e achada.
    monkeypatch.setattr(uds_messaging, "_cc_socks", lambda: tmp_path)
    sock = tmp_path / "1.sock"
    sock.touch()
    sessions = tmp_path / "cfg" / "sessions"
    sessions.mkdir(parents=True)
    (sessions / f"{os.getpid()}.json").write_text(json.dumps({
        "pid": str(os.getpid()), "sessionId": "abc", "messagingSocketPath": str(sock)}))
    assert uds_messaging.socket_da_sessao("abc", str(tmp_path / "cfg")) == str(sock)
    assert uds_messaging.socket_da_sessao("outra", str(tmp_path / "cfg")) is None


def test_enviar_escreve_o_quadro_do_send_message(tmp_path):
    # Formato medido em 21/09/2026 com um socket falso capturando o SendMessage do CLI 2.1.278.
    caminho = str(tmp_path / "alvo.sock")
    srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    srv.bind(caminho)
    srv.listen(1)
    recebido: list[bytes] = []

    def _aceita():
        c, _ = srv.accept()
        buf = b""
        while True:
            ch = c.recv(65536)
            if not ch:
                break
            buf += ch
        recebido.append(buf)
        c.close()

    t = threading.Thread(target=_aceita)
    t.start()
    mid = uds_messaging.enviar(caminho, "[de: a] corpo\nlinha 2", "a", "bypass")
    t.join(5)
    srv.close()
    linhas = recebido[0].decode().splitlines()
    assert len(linhas) == 1
    q = json.loads(linhas[0])
    assert q["msgV"] == 1 and q["msg_id"] == mid and q["type"] == "user" and q["priority"] == "next"
    assert q["message"]["role"] == "user"
    assert q["message"]["content"].startswith('<cross-session-message from="uds:')
    assert 'from-name="a" from-mode="bypass">\n[de: a] corpo\nlinha 2\n</cross-session-message>' in q["message"]["content"]
    assert q["from"].startswith("uds:")


def test_enviar_levanta_sem_socket(tmp_path):
    with pytest.raises(OSError):
        uds_messaging.enviar(str(tmp_path / "nao-existe.sock"), "[de: a] x", "a", "bypass")



@pytest.mark.parametrize("transcripts, memoria, conta, remetente, esperado", [
    # Backend recém-reiniciado: a conta diz `default`, mas a sessão roda em bypass.
    ({"rem": ["default", "bypassPermissions"]}, {}, "manual", "rem", "bypass"),
    ({"rem": ["bypassPermissions", "plan"]}, {}, "manual", "rem", "bypass"),
    # Shift+Tab visto no pane (memória pelo session-id) vence o transcript, que só grava na fala.
    ({"rem": ["bypassPermissions"]}, {"sid-rem": "acceptEdits"}, "bypassPermissions", "rem", "prompting"),
    ({"alvo": ["bypassPermissions"]}, {}, "manual", "rem", "bypass"),
    ({}, {}, "bypassPermissions", "rem", "bypass"),
    ({}, {}, "manual", "rem", "prompting"),
    # Rótulo de aviso do app não é sessão: vale a classe do alvo.
    ({"alvo": ["bypassPermissions"]}, {}, "manual", "entrega de recado", "bypass"),
], ids=["reinicio", "plan", "memoria", "alvo", "conta-bypass", "conta-manual", "painel"])
def test_classe_modo(tmp_path, monkeypatch, transcripts, memoria, conta, remetente, esperado):
    from collections import OrderedDict

    from app import api

    jsonls = {}
    for nome, modos in transcripts.items():
        p = tmp_path / f"sid-{nome}.jsonl"
        p.write_text("".join(json.dumps({"type": "user", "permissionMode": m}) + "\n" for m in modos))
        jsonls[nome] = str(p)
    resolvidos = []
    monkeypatch.setattr(api.headless_sessions, "load", lambda n: None)
    monkeypatch.setattr(api, "_jsonl_atual", lambda n: resolvidos.append(n) or jsonls.get(n))
    monkeypatch.setattr(api.permission_mode, "_ultimos_nao_plan", OrderedDict(memoria))
    monkeypatch.setattr(api.permission_mode, "modo_da_conta", lambda cfg: conta)
    assert api._classe_modo(remetente, "alvo", None) == esperado
    assert "entrega de recado" not in resolvidos


def test_classe_modo_usa_o_jsonl_do_alvo_que_o_chamador_ja_tem(tmp_path, monkeypatch):
    from app import api

    p = tmp_path / "sid-alvo.jsonl"
    p.write_text(json.dumps({"type": "user", "permissionMode": "bypassPermissions"}) + "\n")
    resolvidos = []
    monkeypatch.setattr(api.headless_sessions, "load", lambda n: None)
    monkeypatch.setattr(api, "_jsonl_atual", lambda n: resolvidos.append(n))
    monkeypatch.setattr(api.permission_mode, "modo_da_conta", lambda cfg: "manual")
    assert api._classe_modo("rem", "alvo", None, str(p)) == "bypass"
    assert resolvidos == ["rem"]
