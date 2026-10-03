#!/usr/bin/env python3
"""Cano: dono do processo `claude` de uma sessão sem terminal, separado do backend.

Sobe o `claude` (stream-json), segura stdin/stdout dele e escuta num socket local. O backend é
um cliente que conecta, desconecta e reconecta — o processo não morre com o backend. Não entende
a conversa; só observa os dois sentidos o bastante pra dizer, a quem conectar, o que está em
aberto (snapshot):

  - a última linha `system/init` (sid, modelo, modo);
  - se há turno aberto (viu `user` do cliente ou `command_lifecycle/started` sem `result` depois);
  - os `control_request` do claude ainda sem `control_response` (a permissão pendente, literal);
  - o último `result` e o último `rate_limit_event`;
  - a cauda do stderr e, se o claude já saiu, o código de saída.

Stdlib puro e rodado por caminho (não `-m`): não importa nada do backend. Falha aqui nunca vira
traceback no terminal de ninguém — vai pro arquivo de log e o processo sai.

Uso: cano.py --escuta unix:/x.sock|tcp:127.0.0.1:PORT [--token T] [--log F] -- claude args...
"""
import argparse
import collections
import io
import json
import os
import queue
import signal
import socket
import subprocess
import sys
import threading
import time

VERSAO = 2
MAX_FRAME = 16 * 1024 * 1024
MAX_ENVELOPE = 2 * MAX_FRAME + 1024
_TETO_LINGER_S = 60.0       # após o claude sair, espera um cliente pra entregar o rc, depois morre


class Cano:
    def __init__(self, escuta: str, token: str | None, log):
        self.escuta = escuta
        self.token = token
        self.log = log
        self.proc: subprocess.Popen | None = None
        self.trava = threading.Lock()          # snapshot + troca de cliente
        self.stdin_lock = threading.Lock()
        self.saida: queue.Queue[str] = queue.Queue(maxsize=5000)   # linhas pro cliente
        self.saida_cheia = False
        self.cliente: socket.socket | None = None
        self.cliente_arq = None                # makefile do cliente: segura o socket, fecha junto
        self.init: str | None = None
        self.aberto = False
        self.pendentes: dict[tuple, str] = {}
        self.inflight_claude = self._new_claude_prefix()
        self.inflight_codex: dict[str, dict] = {}
        self.ultimo_result: str | None = None
        self.rate_limit: str | None = None
        self.stderr_tail: collections.deque[str] = collections.deque(maxlen=20)
        self.saiu: int | None = None
        self.saiu_entregue = threading.Event()
        self.child_revision = self.result_revision = 0

    # ── log ────────────────────────────────────────────────────────────────────────────────

    def _log(self, msg: str) -> None:
        try:
            self.log.write(f"{time.strftime('%H:%M:%S')} {msg}\n")
            self.log.flush()
        except Exception:
            pass

    # ── claude ─────────────────────────────────────────────────────────────────────────────

    def subir(self, argv: list[str], cwd: str) -> None:
        # Mesmo grupo de processos que o cano: matar o grupo do cano mata os dois.
        self.proc = subprocess.Popen(
            argv, cwd=cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            bufsize=0)
        threading.Thread(target=self._ler_stdout, daemon=True).start()
        threading.Thread(target=self._ler_stderr, daemon=True).start()
        threading.Thread(target=self._enviar, daemon=True).start()
        self._log(f"claude pid={self.proc.pid}")

    def _ler_stdout(self) -> None:
        assert self.proc and self.proc.stdout
        while bruto := self.proc.stdout.readline(MAX_FRAME + 2):
            if len(bruto) > MAX_FRAME + 1 or not bruto.endswith(b"\n"):
                self._log("stdout do filho com linha incompleta ou acima do teto")
                break
            linha = bruto.decode("utf-8", "replace").rstrip("\r\n")
            if not linha:
                continue
            with self.trava:
                self._observar_claude(linha)
                self._mandar(json.dumps({"type": "cano_output", "frame": linha}, ensure_ascii=False))
        rc = self.proc.wait()
        with self.trava:
            self.saiu = rc
            self.aberto = False
            self._log(f"claude saiu rc={rc}")
            self._mandar_saida()

    def _ler_stderr(self) -> None:
        assert self.proc and self.proc.stderr
        for bruto in self.proc.stderr:
            linha = _texto_do_stderr(bruto).rstrip("\r\n")
            if not linha:
                continue
            with self.trava:
                self.stderr_tail.append(linha)
                self._mandar(json.dumps({"type": "cano_stderr", "linha": linha}))

    def _observar_claude(self, linha: str) -> None:
        try:
            ev = json.loads(linha)
        except ValueError:
            return
        if not isinstance(ev, dict):
            return
        self.child_revision += 1
        t = ev.get("type")
        if ev.get("parent_tool_use_id") and t not in ("control_request", "sdk_control_request", "control_cancel_request", "control_response"):
            return
        self._observe_prefix(ev)
        if t == "system" and ev.get("subtype") == "init":
            self.init = linha
        elif t == "command_lifecycle" and ev.get("state") == "started":
            self.aberto = True
        elif t in ("control_request", "sdk_control_request"):
            self._put_pending(ev.get("request_id"), linha)
        elif t == "control_cancel_request":
            self.pendentes.pop(self._request_key(ev.get("request_id")), None)
        elif t == "result":
            self.result_revision = self.child_revision
            self.aberto = False
            self.ultimo_result = linha
            self.pendentes.clear()
        elif t == "rate_limit_event":
            self.rate_limit = linha
        elif "method" in ev:
            # JSON-RPC (app-server do Codex): pedido do servidor tem method + id; a resolução vem
            # como notificação própria quando outro cliente responde.
            if ev.get("id") is not None:
                self._put_pending(ev["id"], linha)
            elif ev["method"] == "serverRequest/resolved":
                self.pendentes.pop(self._request_key((ev.get("params") or {}).get("requestId")), None)
            elif ev["method"] == "turn/completed":
                # Turno fechado (interrompido inclusive) leva os pedidos da thread junto, como o
                # cliente faz — senão o snapshot repovoa um cartão que o servidor já esqueceu.
                thread = (ev.get("params") or {}).get("threadId")
                for rid, bruto in list(self.pendentes.items()):
                    try:
                        pedido = json.loads(bruto)
                    except ValueError:
                        continue
                    if (pedido.get("params") or {}).get("threadId") == thread:
                        self.pendentes.pop(rid, None)

    def _observar_cliente(self, linha: str) -> None:
        try:
            ev = json.loads(linha)
        except ValueError:
            return
        if not isinstance(ev, dict):
            return
        t = ev.get("type")
        if t == "user":
            self.aberto = True
        elif t == "control_response":
            rid = (ev.get("response") or {}).get("request_id")
            self.pendentes.pop(self._request_key(rid), None)
        elif t is None and "method" not in ev and ev.get("id") is not None:
            self.pendentes.pop(self._request_key(ev["id"]), None)

    @staticmethod
    def _request_key(value):
        if type(value) in (int, str):
            return type(value).__name__, value
        return None

    def _put_pending(self, request_id, line):
        if (key := self._request_key(request_id)) is not None:
            self.pendentes[key] = line

    @staticmethod
    def _new_claude_prefix():
        return {"complete": True, "text": io.StringIO(), "thinking": io.StringIO(),
                "tool": {"name": None, "input": io.StringIO(), "index": None}, "bytes": 0}

    def _observe_prefix(self, event):
        kind = event.get("type")
        prefix = self.inflight_claude
        if kind == "stream_event":
            stream = event.get("event") or {}
            block = stream.get("content_block") or {}
            if stream.get("type") == "content_block_start":
                if block.get("type") == "text":
                    prefix["text"] = io.StringIO()
                    prefix["complete"] = True
                    prefix["bytes"] = len(prefix["thinking"].getvalue().encode()) + len(prefix["tool"]["input"].getvalue().encode())
                elif block.get("type") == "thinking":
                    prefix["thinking"] = io.StringIO()
                elif block.get("type") in ("tool_use", "server_tool_use", "mcp_tool_use"):
                    prefix["tool"] = {"name": block.get("name"), "input": io.StringIO(), "index": stream.get("index")}
            elif stream.get("type") == "content_block_delta" and prefix["complete"]:
                delta = stream.get("delta") or {}
                piece = delta.get("text") or delta.get("thinking") or delta.get("partial_json") or ""
                prefix["bytes"] += len(piece.encode("utf-8"))
                if prefix["bytes"] > MAX_FRAME // 2:
                    prefix["complete"] = False
                    prefix["text"], prefix["thinking"], prefix["tool"]["input"] = io.StringIO(), io.StringIO(), io.StringIO()
                elif delta.get("type") == "text_delta":
                    prefix["text"].write(piece)
                elif delta.get("type") == "thinking_delta":
                    prefix["thinking"].write(piece)
                elif delta.get("type") == "input_json_delta":
                    prefix["tool"]["input"].write(piece)
            elif stream.get("type") == "content_block_stop":
                prefix["tool"] = {"name": None, "input": io.StringIO(), "index": None}
        elif kind in ("result", "conversation_reset"):
            self.inflight_claude = self._new_claude_prefix()
        elif kind == "assistant" and "local_command_source" not in event:
            for block in (event.get("message") or {}).get("content") or []:
                if block.get("type") in ("text", "thinking"):
                    prefix[block["type"]] = io.StringIO()
                elif block.get("type") == "tool_use":
                    prefix["tool"] = {"name": None, "input": io.StringIO(), "index": None}
        params = event.get("params") or {}
        thread = params.get("threadId")
        method = event.get("method")
        if not isinstance(thread, str) or method not in ("turn/started", "turn/completed", "item/agentMessage/delta", "item/started", "item/completed"):
            return
        prefix = self.inflight_codex.setdefault(thread, {"complete": True, "text": io.StringIO(), "bytes": 0, "itemId": None, "turnId": None})
        if method == "turn/started":
            prefix.update(complete=True, text=io.StringIO(), bytes=0, itemId=None, turnId=(params.get("turn") or {}).get("id"))
        elif method == "item/agentMessage/delta":
            item = params.get("itemId")
            if prefix["itemId"] is not None and item != prefix["itemId"]:
                prefix.update(complete=True, text=io.StringIO(), bytes=0)
            prefix.update(itemId=item, turnId=params.get("turnId"))
            piece = params.get("delta") or ""
            prefix["bytes"] += len(piece.encode())
            if prefix["bytes"] > MAX_FRAME // 2:
                prefix.update(complete=False, text=io.StringIO())
            elif prefix["complete"]:
                prefix["text"].write(piece)
        elif method == "turn/completed":
            self.inflight_codex.pop(thread, None)
        elif (params.get("item") or {}).get("type") == "agentMessage":
            prefix.update(complete=True, text=io.StringIO(), bytes=0)

    # ── cliente ────────────────────────────────────────────────────────────────────────────

    def _mandar(self, linha: str) -> None:
        # Sob self.trava. Só enfileira: quem faz o `sendall` é a thread de envio, fora da trava —
        # um backend travado não pode segurar o leitor do stdout (e o claude atrás dele) nem
        # impedir o próximo backend de conectar. Cliente ausente = linha descartada: o snapshot
        # carrega o que importa. Fila cheia = cliente que parou de ler há muito; descarta e avisa.
        if self.cliente is None:
            return
        try:
            self.saida.put_nowait((self.cliente, linha))
        except queue.Full:
            if not self.saida_cheia:
                self.saida_cheia = True
                self._log("fila de saída cheia: cliente não lê; descartando")

    def _mandar_saida(self) -> None:
        # Sob self.trava. Última mensagem: vai SÍNCRONA (com teto), não pela fila — o processo sai
        # logo depois e a thread de envio morreria com a linha dentro. Sem cliente, fica pro
        # snapshot de quem chegar.
        con = self.cliente
        if con is None:
            return
        linha = json.dumps({"type": "cano_saiu", "rc": self.saiu, "stderr_tail": list(self.stderr_tail)})
        try:
            con.settimeout(5)
            con.sendall((linha + "\n").encode("utf-8"))
        except OSError:
            self._fechar_cliente()
            return
        self.saiu_entregue.set()

    def _enviar(self) -> None:
        while True:
            destination, linha = self.saida.get()
            with self.trava:
                con = self.cliente
            if con is None or con is not destination:
                continue
            try:
                con.sendall((linha + "\n").encode("utf-8"))
            except OSError:
                with self.trava:
                    if self.cliente is con:
                        self._fechar_cliente()

    def _fechar_cliente(self) -> None:
        # Só derruba o socket: o makefile é fechado pela thread que lê dele. Fechá-lo daqui espera
        # o readline em curso (no Windows) com a trava na mão, e a thread leitora precisa dela.
        if self.cliente is not None:
            _derrubar(self.cliente)
            self.cliente = self.cliente_arq = None

    def snapshot(self) -> str:
        prefix = self.inflight_claude
        inflight = {"claude": {"complete": prefix["complete"], "text": prefix["text"].getvalue(),
            "thinking": prefix["thinking"].getvalue(), "tool": {**prefix["tool"], "input": prefix["tool"]["input"].getvalue()}},
            "codex": {thread: {"complete": p["complete"], "text": p["text"].getvalue(),
                                "itemId": p["itemId"], "turnId": p["turnId"]}
                      for thread, p in self.inflight_codex.items()}}
        return json.dumps({
            "type": "cano_snapshot", "versao": VERSAO,
            "pid": self.proc.pid if self.proc else None,
            "init": self.init, "aberto": self.aberto,
            "pendentes": list(self.pendentes.values()),
            "ultimo_result": self.ultimo_result, "rate_limit": self.rate_limit,
            "stderr_tail": list(self.stderr_tail), "saiu": self.saiu,
            "inflight": inflight,
        })

    def servir(self, srv: socket.socket) -> None:
        # Uma thread por cliente: atendendo em série, quem chega só recebe o snapshot quando o
        # ligado sai — e o backend que não recebe snapshot a tempo mata o cano como mudo.
        while True:
            try:
                con, _ = srv.accept()
            except OSError:
                return
            threading.Thread(target=self._atender, args=(con,), daemon=True).start()

    def _atender(self, con: socket.socket) -> None:
        arq = con.makefile("rb")
        try:
            con.settimeout(10)
            raw = arq.readline(4098)
            header = raw.decode("utf-8").rstrip("\r\n")
            peek = header == f"peek {self.token}"
            if not self.token or not raw.endswith(b"\n") or len(raw) > 4097 or (header != self.token and not peek):
                _fechar(con, arq)
                return
            con.settimeout(None)
            with self.trava:
                snapshot = (self.snapshot() + "\n").encode("utf-8")
            if len(snapshot) > MAX_FRAME + 1:
                _fechar(con, arq)
                return
            if peek:
                con.sendall(snapshot)
                _fechar(con, arq)
                return
        except (OSError, ValueError):
            _fechar(con, arq)
            return
        with self.stdin_lock, self.trava:
            # Releitura sob a trava da escrita: peek não é o snapshot do claim.
            snapshot = (self.snapshot() + "\n").encode("utf-8")
            if len(snapshot) > MAX_FRAME + 1:
                _fechar(con, arq)
                return
            self._fechar_cliente()      # um cliente por vez: o novo backend substitui o antigo
            # Linhas que sobraram pro cliente antigo já estão refletidas no snapshot; mandar
            # de novo duplicaria eventos no backend novo.
            while not self.saida.empty():
                try:
                    self.saida.get_nowait()
                except queue.Empty:
                    break
            self.saida_cheia = False
            self.cliente, self.cliente_arq = con, arq
            try:
                con.sendall(snapshot)
            except OSError:
                self._fechar_cliente()
                arq.close()
                return
            if self.saiu is not None:
                # Já saiu: entrega o evento de saída como se estivesse acontecendo agora, e
                # aí pode morrer — quem chegou levou o rc e o stderr.
                self._mandar_saida()
        self._log("cliente conectado")
        self._ler_cliente(con, arq)
        self._log("cliente saiu")

    def _ler_cliente(self, con: socket.socket, arq) -> None:
        try:
            while bruto := arq.readline(MAX_ENVELOPE + 2):
                if not bruto.endswith(b"\n") or len(bruto) > MAX_ENVELOPE + 1:
                    return
                try:
                    envelope = json.loads(bruto)
                    if not isinstance(envelope, dict):
                        continue
                    operation = envelope.get("operation_id") if envelope.get("type") == "cano_input" else None
                    linha = envelope.get("frame") if envelope.get("type") == "cano_input" else bruto.decode().rstrip("\r\n")
                    if not isinstance(linha, str) or len(linha.encode()) > MAX_FRAME or "\n" in linha or "\r" in linha or not isinstance(json.loads(linha), dict):
                        continue
                    if envelope.get("type") == "cano_input" and not isinstance(operation, str):
                        continue
                except (ValueError, UnicodeError):
                    continue
                with self.stdin_lock:
                    with self.trava:
                        if self.cliente is not con:
                            return
                        before = self.child_revision
                    outcome, written = "not_written", 0
                    if self.proc and self.proc.stdin and self.saiu is None:
                        data = (linha + "\n").encode()
                        try:
                            while written < len(data):
                                count = self.proc.stdin.write(data[written:])
                                if not isinstance(count, int) or count <= 0:
                                    raise OSError("stdin sem progresso")
                                written += count
                            self.proc.stdin.flush()
                            outcome = "written"
                        except OSError:
                            outcome = "unknown" if written else "not_written"
                            self._log("stdin do filho sem confirmação de escrita")
                    with self.trava:
                        if outcome == "written":
                            self._observar_cliente(linha)
                            if self.result_revision > before:
                                self.aberto = False
                        if operation is not None:
                            self._mandar(json.dumps({"type": "cano_input_ack", "operation_id": operation, "outcome": outcome}))
        except OSError:
            pass
        finally:
            with self.trava:
                if self.cliente is con:
                    self._fechar_cliente()
            _fechar(con, arq)

    def escutar(self) -> socket.socket:
        # ANTES de subir o claude: escuta que falha (path unix > 107 bytes, porta ocupada) tem
        # que derrubar o cano com log e código de saída, nunca deixar um claude órfão sem porta.
        if self.escuta.startswith("unix:"):
            caminho = self.escuta[5:]
            try:
                os.unlink(caminho)
            except FileNotFoundError:
                pass
            srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            srv.bind(caminho)
            os.chmod(caminho, 0o600)
        else:
            host, porta = self.escuta[4:].rsplit(":", 1)
            srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            srv.bind((host, int(porta)))
        srv.listen(2)
        return srv

    def esperar_fim(self) -> None:
        # Vive enquanto o claude viver; depois espera um cliente levar o rc (ou desiste).
        assert self.proc
        self.proc.wait()
        self.saiu_entregue.wait(_TETO_LINGER_S)
        if self.escuta.startswith("unix:"):
            try:
                os.unlink(self.escuta[5:])
            except OSError:
                pass


def _texto_do_stderr(bruto: bytes) -> str:
    # O claude escreve UTF-8; scripts do Windows no meio (hangar-engine.CMD, cmd) escrevem na
    # codepage local, e decodificar como UTF-8 punha U+FFFD no aviso de problema.
    try:
        return bruto.decode("utf-8")
    except UnicodeDecodeError:
        import codecs
        import locale
        try:
            enc = codecs.lookup(locale.getpreferredencoding(False) or "cp1252").name
        except LookupError:
            enc = "cp1252"
        if enc == "utf-8":
            # Locale já é UTF-8 (Linux, cp65001): repetir o UTF-8 só trocaria os acentos por U+FFFD.
            enc = "cp1252"
        return bruto.decode(enc, "replace")


def _derrubar(con: socket.socket) -> None:
    # shutdown acorda o leitor no Linux; fechar o descritor de verdade acorda no Windows. O
    # detach evita que o close do makefile, depois, feche um descritor já reaproveitado.
    try:
        con.shutdown(socket.SHUT_RDWR)
    except OSError:
        pass
    try:
        fd = con.detach()
    except OSError:
        return
    if fd != -1:
        try:
            socket.close(fd)
        except OSError:
            pass


def _fechar(con: socket.socket, arq) -> None:
    for f in (arq, con):
        try:
            if f is not None:
                f.close()
        except OSError:
            pass


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--escuta", required=True)
    ap.add_argument("--token", default=None)
    ap.add_argument("--log", default=None)
    ap.add_argument("--cwd", default=os.getcwd())
    ap.add_argument("argv", nargs=argparse.REMAINDER)
    a = ap.parse_args()
    argv = a.argv[1:] if a.argv and a.argv[0] == "--" else a.argv
    if not argv:
        print("cano: faltou o comando do claude depois de --", file=sys.stderr)
        return 2
    if not a.token:
        print("cano: token obrigatório para o cano v2", file=sys.stderr)
        return 2
    log = open(a.log, "a", encoding="utf-8") if a.log else sys.stderr
    cano = Cano(a.escuta, a.token, log)

    def _thread_estourou(args):
        # stderr do cano é DEVNULL: sem isto, exceção numa thread sumia sem rastro.
        cano._log(f"thread {getattr(args.thread, 'name', '?')} estourou: {args.exc_type.__name__}: {args.exc_value}")
    threading.excepthook = _thread_estourou
    try:
        srv = cano.escutar()
    except Exception as e:
        cano._log(f"não consegui escutar em {a.escuta}: {e}")
        return 1
    try:
        cano.subir(argv, a.cwd)
    except Exception as e:
        cano._log(f"claude não subiu: {e}")
        srv.close()
        return 1
    threading.Thread(target=cano.servir, args=(srv,), daemon=True).start()

    def _terminar(signum, frame):
        # SIGTERM do backend (encerrar sessão): derruba o claude e não deixa socket velho.
        cano._log(f"sinal {signum}: encerrando")
        try:
            if cano.proc and cano.proc.poll() is None:
                cano.proc.terminate()
        except OSError:
            pass
        if a.escuta.startswith("unix:"):
            try:
                os.unlink(a.escuta[5:])
            except OSError:
                pass
        os._exit(0)
    if hasattr(signal, "SIGTERM"):
        signal.signal(signal.SIGTERM, _terminar)
    cano.esperar_fim()
    return 0


if __name__ == "__main__":
    sys.exit(main())
