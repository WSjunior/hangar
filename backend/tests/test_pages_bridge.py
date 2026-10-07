import json
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer

import pytest

from app import pages_bridge


def _server(reply: dict, seen: list):
    class H(BaseHTTPRequestHandler):
        def do_POST(self):
            seen.append((self.path, self.headers.get("x-hangar-internal"), json.loads(self.rfile.read(int(self.headers["content-length"])))))
            body = json.dumps(reply).encode()
            self.send_response(200); self.send_header("content-type", "application/json"); self.end_headers(); self.wfile.write(body)
        def log_message(self, *a): pass
    srv = HTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


def test_publish_sends_secret_and_returns_result():
    seen = []
    srv = _server({"ok": True, "result": {"hangar_page": {"id": "a1"}}}, seen)
    pages_bridge.configure(f"127.0.0.1:{srv.server_port}", "s3gredo")
    try:
        out = pages_bridge.publish({"session": "x", "html": "<p></p>", "title": "t"})
    finally:
        srv.shutdown(); pages_bridge.configure(None, None)
    assert out == {"hangar_page": {"id": "a1"}}
    assert seen[0][0] == "/__hangar_server/pages" and seen[0][1] == "s3gredo"
    assert seen[0][2] == {"session": "x", "html": "<p></p>", "title": "t"}


def test_error_from_rust_is_raised_with_code():
    srv = _server({"ok": False, "error": {"code": "erro_pagina_invalida", "detail": "title"}}, [])
    pages_bridge.configure(f"127.0.0.1:{srv.server_port}", "s")
    try:
        with pytest.raises(pages_bridge.PagesBridgeError) as e:
            pages_bridge.publish({"session": "x", "html": "", "title": "t"})
    finally:
        srv.shutdown(); pages_bridge.configure(None, None)
    assert e.value.code == "erro_pagina_invalida"


def test_without_rust_server_is_clear_error():
    pages_bridge.configure(None, None)
    with pytest.raises(pages_bridge.PagesBridgeError) as e:
        pages_bridge.publish({"session": "x", "html": "<p></p>", "title": "t"})
    assert e.value.code == "erro_paginas_sem_servidor_rust"


def test_ok_without_result_is_unavailable():
    srv = _server({"ok": True}, [])
    pages_bridge.configure(f"127.0.0.1:{srv.server_port}", "s")
    try:
        with pytest.raises(pages_bridge.PagesBridgeError) as e:
            pages_bridge.publish({"session": "x", "html": "<p></p>", "title": "t"})
    finally:
        srv.shutdown(); pages_bridge.configure(None, None)
    assert e.value.code == "erro_paginas_indisponivel"
