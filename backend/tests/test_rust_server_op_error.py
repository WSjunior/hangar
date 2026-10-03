import asyncio
import json
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer

import pytest

from app.rust_server import RuntimeTransport


def _server(status: int, body: dict):
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            self.rfile.read(int(self.headers["content-length"]))
            raw = json.dumps(body).encode()
            self.send_response(status)
            self.send_header("content-length", str(len(raw)))
            self.end_headers()
            self.wfile.write(raw)

        def log_message(self, *args):
            pass

    server = HTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


@pytest.mark.parametrize("status,body,expected", [
    (503, {"ok": False, "error_code": "cano_binding", "message": "snapshot de outro cano"},
     "503: cano_binding snapshot de outro cano"),
    (409, {}, "409: protocolo ou instância do Rust diferente"),
])
def test_refusal_carries_rust_reason(status, body, expected):
    server = _server(status, body)
    try:
        transport = RuntimeTransport(server.server_port, "s", "i", lambda: True)
        with pytest.raises(RuntimeError) as err:
            asyncio.run(transport.op({"key": "k", "generation": 1}, {"kind": "snapshot"}, "op-1", {}))
        assert expected in str(err.value)
    finally:
        server.shutdown()
