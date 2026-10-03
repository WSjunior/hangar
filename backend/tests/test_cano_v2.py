"""Transporte real com stdin sintético; nenhuma CLI ou serviço do Hangar é iniciado."""
import io
import json
import socket
import threading
from types import SimpleNamespace

import pytest

from app.adapters.claude_headless.cano import Cano, VERSAO


def make_cano():
    cano = Cano("tcp:127.0.0.1:0", "secret-test", io.StringIO())
    cano.proc = SimpleNamespace(pid=42, stdin=io.BytesIO())
    threading.Thread(target=cano._enviar, daemon=True).start()
    return cano


def connection(cano):
    client, server = socket.socketpair()
    client.settimeout(2)
    thread = threading.Thread(target=cano._atender, args=(server,), daemon=True)
    thread.start()
    return client, client.makefile("rb"), thread


def test_pending_ids_keep_type():
    cano = make_cano()
    for request_id in (1, "1"):
        cano._observar_claude(json.dumps({"id": request_id, "method": "approval"}))
    cano._observar_cliente('{"id":1,"result":{}}')
    pending = json.loads(cano.snapshot())["pendentes"]
    assert [json.loads(p)["id"] for p in pending] == ["1"]


def test_snapshot_keeps_full_prefix_and_parent():
    cano = make_cano()
    cano._observar_cliente('{"type":"user","message":{"content":"olá"}}')
    cano._observar_claude(json.dumps({"type": "stream_event", "event": {
        "type": "content_block_delta", "delta": {"type": "text_delta", "text": "Olá 🌎"}}}))
    cano._observar_claude('{"type":"result","parent_tool_use_id":"child"}')
    snapshot = json.loads(cano.snapshot())
    assert snapshot["aberto"] is True
    assert snapshot["inflight"]["claude"]["text"] == "Olá 🌎"
    assert snapshot["inflight"]["claude"]["complete"] is True


def test_peek_keeps_old_writer():
    cano = make_cano()
    client, stream, thread = connection(cano)
    peek_client, peek_stream, peek_thread = connection(cano)
    try:
        client.sendall(b"secret-test\n")
        assert json.loads(stream.readline())["versao"] == 2
        peek_client.sendall(b"peek secret-test\n")
        assert json.loads(peek_stream.readline())["type"] == "cano_snapshot"
        assert peek_stream.readline() == b""
        client.sendall(b'{"type":"cano_input","operation_id":"wire:1","frame":"{\\"type\\":\\"user\\"}"}\n')
        ack = json.loads(stream.readline())
        assert ack == {"type": "cano_input_ack", "operation_id": "wire:1", "outcome": "written"}
        assert cano.proc.stdin.getvalue() == b'{"type":"user"}\n'
    finally:
        for sock in (client, peek_client):
            sock.shutdown(socket.SHUT_RDWR)
            sock.close()
        stream.close()
        peek_stream.close()
        thread.join(2)
        peek_thread.join(2)


def test_partial_eof_not_forwarded():
    cano = make_cano()
    client, stream, thread = connection(cano)
    client.sendall(b"secret-test\n")
    stream.readline()
    client.sendall(b'{"type":"user"}')
    client.shutdown(socket.SHUT_WR)
    thread.join(2)
    assert cano.proc.stdin.getvalue() == b""
    client.close()
    stream.close()


@pytest.mark.parametrize("partial", [False, True])
def test_ack_requires_child_flush(partial):
    class BrokenStdin:
        def write(self, data):
            if not partial:
                raise BrokenPipeError()
            return 1

        def flush(self):
            raise BrokenPipeError()

    cano = make_cano()
    cano.proc.stdin = BrokenStdin()
    client, stream, thread = connection(cano)
    try:
        client.sendall(b"secret-test\n")
        stream.readline()
        client.sendall(b'{"type":"cano_input","operation_id":"wire:1","frame":"{\\"type\\":\\"user\\"}"}\n')
        ack = json.loads(stream.readline())
        assert ack["outcome"] == ("unknown" if partial else "not_written")
        assert not cano.aberto
    finally:
        client.shutdown(socket.SHUT_RDWR)
        client.close()
        stream.close()
        thread.join(2)


def test_wrong_token_does_not_replace_writer():
    cano = make_cano()
    old = object()
    cano.cliente = old
    client, stream, thread = connection(cano)
    client.sendall(b"wrong-token\n")
    assert stream.readline() == b""
    thread.join(2)
    assert cano.cliente is old
    client.close()
    stream.close()


def test_oversize_keeps_old_connection(monkeypatch):
    from app.adapters.claude_headless import cano as module
    cano = make_cano()
    old = object()
    cano.cliente = old
    request = json.dumps({"id": "pending", "method": "approval", "params": {"text": "a" * 1024}})
    cano._observar_claude(request)
    monkeypatch.setattr(module, "MAX_FRAME", 100)
    client, stream, thread = connection(cano)
    client.sendall(b"secret-test\n")
    assert stream.readline() == b""
    thread.join(2)
    assert cano.cliente is old
    assert json.loads(cano.snapshot())["pendentes"] == [request]
    client.close()
    stream.close()


def test_cli_output_cannot_forge_private_ack():
    cano = make_cano()
    output = io.BytesIO(b'{"type":"cano_input_ack","operation_id":"wire:1","outcome":"written"}\n')
    cano.proc.stdout = output
    cano.proc.wait = lambda: 0
    frames = []
    cano._mandar = frames.append
    cano._mandar_saida = lambda: None
    cano._ler_stdout()
    assert json.loads(frames[0])["type"] == "cano_output"
    assert json.loads(json.loads(frames[0])["frame"])["type"] == "cano_input_ack"


def test_result_before_ack_does_not_reopen_parent():
    cano = make_cano()

    class FastReply:
        def write(self, data):
            with cano.trava:
                cano._observar_claude('{"type":"result","subtype":"success"}')
            return len(data)

        def flush(self):
            pass

    cano.proc.stdin = FastReply()
    client, stream, thread = connection(cano)
    try:
        client.sendall(b"secret-test\n")
        stream.readline()
        client.sendall(b'{"type":"cano_input","operation_id":"wire:1","frame":"{\\"type\\":\\"user\\"}"}\n')
        assert json.loads(stream.readline())["outcome"] == "written"
        assert json.loads(cano.snapshot())["aberto"] is False
    finally:
        client.shutdown(socket.SHUT_RDWR)
        client.close()
        stream.close()
        thread.join(2)
