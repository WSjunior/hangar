#!/usr/bin/env python3
"""Mede Codex sem terminal com N sessões e N chats abertos: Python × Rust (parte 5B, Task 8).

    scripts/medir-codex-sem-terminal.py <hangar-server> <hangar-cano> <rótulo> --rust 0|1 [--n 10]
                                        [--relatorio arq.jsonl]

Backend isolado (classe `Prova` de `prova-dono-unico.py`: HOME, portas, token e `tmux -L` próprios,
`matar_orfaos` no-op, portas 8766/8768 trocadas) com `CP_RUST_NO_ORPHAN_SWEEP=1`. `--rust 0` liga
`CP_RUST_SERVER=0` (o Python é dono do Codex sem terminal, como antes da 5B). Sem Codex real: o
`codex` do PATH é um app-server de mentira (stdio JSON-RPC) que, com turno aberto, manda um
`item/agentMessage/delta` a cada 50 ms até o arquivo `parar` aparecer.

Mede parado (sessões abertas, sem turno) e trabalhando (todas com turno), 3 janelas de 20 s depois
de 8 s assentando: CPU em ms por segundo do Python, do Rust, dos `hangar-cano` e dos `codex`
(soma), eventos `preview` por segundo nos chats e pico de RSS (VmHWM). Confere um cano por sessão
e que nenhum `hangar-cano` de fora da prova sumiu.
"""
import argparse
import importlib.util
import json
import os
import statistics
import sys
import threading
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("base", REPO / "scripts/prova-dono-unico.py")
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)

JANELAS = 3
TENTATIVAS = 10
JANELA_S = 20.0
ASSENTAR_S = 8.0

FALSO_CODEX = r'''#!/usr/bin/env python3
import json, os, sys, threading, time, uuid
VERSAO = "0.161.0"
PARAR = __PARAR__
if sys.argv[1:2] == ["--version"]:
    print("codex-cli " + VERSAO)
    sys.exit(0)
if sys.argv[1:2] != ["app-server"]:
    sys.exit(0)
lock = threading.Lock()
thread_id = str(uuid.uuid4())
home = os.environ.get("CODEX_HOME") or os.path.expanduser("~/.codex")
rollout = os.path.join(home, "sessions/2026/10/08", f"rollout-2026-10-08T00-00-00-{thread_id}.jsonl")
turno = {"id": None, "fim": threading.Event()}

def send(obj):
    with lock:
        sys.stdout.write(json.dumps(obj) + "\n")
        sys.stdout.flush()

def grava(obj):
    os.makedirs(os.path.dirname(rollout), exist_ok=True)
    with open(rollout, "a") as f:
        f.write(json.dumps({"timestamp": "2026-10-08T00:00:00.000Z", **obj}) + "\n")

def thread():
    if not os.path.exists(rollout):
        grava({"type": "session_meta", "payload": {"id": thread_id, "cwd": os.getcwd()}})
    return {"thread": {"id": thread_id, "path": rollout}, "model": "gpt-test"}

def stream(tid, iid):
    send({"method": "turn/started", "params": {"threadId": thread_id, "turn": {"id": tid, "status": "inProgress"}}})
    send({"method": "item/started", "params": {"threadId": thread_id, "turnId": tid,
          "item": {"type": "agentMessage", "id": iid, "text": ""}}})
    partes = []
    while not turno["fim"].wait(0.05) and not os.path.exists(PARAR):
        partes.append("palavra ")
        send({"method": "item/agentMessage/delta", "params": {"threadId": thread_id, "turnId": tid,
              "itemId": iid, "delta": "palavra "}})
    texto = "".join(partes)[:2000]
    grava({"type": "response_item", "payload": {"type": "message", "role": "assistant",
           "content": [{"type": "output_text", "text": texto}]}})
    send({"method": "item/completed", "params": {"threadId": thread_id, "turnId": tid,
          "item": {"type": "agentMessage", "id": iid, "text": texto}}})
    send({"method": "turn/completed", "params": {"threadId": thread_id, "turn": {"id": tid, "status": "completed"}}})
    turno["id"] = None

for line in sys.stdin:
    try:
        msg = json.loads(line)
    except ValueError:
        continue
    if "method" not in msg or "id" not in msg:
        continue
    m = msg["method"]
    p = msg.get("params") or {}
    with open(PARAR + f".{os.getpid()}.log", "a") as f:
        f.write(m + "\n")
    if m == "initialize":
        r = {"userAgent": f"codex_cli_rs/{VERSAO} (fake)"}
    elif m in ("thread/start", "thread/resume", "thread/read"):
        r = thread()
    elif m == "turn/start":
        texto = " ".join(i.get("text", "") for i in p.get("input") or [] if isinstance(i, dict))
        grava({"type": "response_item", "payload": {"type": "message", "role": "user",
               "content": [{"type": "input_text", "text": texto}]}})
        tid, iid = str(uuid.uuid4()), str(uuid.uuid4())
        turno["id"] = tid
        turno["fim"] = threading.Event()
        send({"id": msg["id"], "result": {"turn": {"id": tid, "status": "inProgress"}}})
        threading.Thread(target=stream, args=(tid, iid), daemon=True).start()
        continue
    elif m == "turn/interrupt":
        turno["fim"].set()
        r = {}
    else:
        r = {}
    send({"id": msg["id"], "result": r})
'''


class Args:
    conta_b = None
    manter = os.environ.get("MANTER") == "1"


def cpu(pid):
    try:
        f = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    except OSError:
        return 0.0
    return sum(int(f[i]) for i in (11, 12, 13, 14)) / os.sysconf("SC_CLK_TCK")


def status_kb(pid, campo):
    try:
        for linha in Path(f"/proc/{pid}/status").read_text().splitlines():
            if linha.startswith(campo + ":"):
                return int(linha.split()[1])
    except OSError:
        pass
    return 0


def environ(pid):
    try:
        return dict(i.partition(b"=")[::2] for i in Path(f"/proc/{pid}/environ").read_bytes().split(b"\0") if b"=" in i)
    except OSError:
        return {}


def canos_de_fora(home):
    """`hangar-cano` vivos que não são desta prova (os reais do dono): pid → cmdline."""
    out = {}
    for p in Path("/proc").iterdir():
        if not p.name.isdigit() or os.path.basename(base.exe(int(p.name))) != "hangar-cano":
            continue
        if environ(int(p.name)).get(b"HOME") == str(home).encode():
            continue
        try:
            out[int(p.name)] = (p / "cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace")[:120]
        except OSError:
            pass
    return out


class Medida(base.Prova):
    PERFIL = "release"

    def __init__(self, server, cano, rust, n):
        super().__init__(Args())
        self.server, self.cano, self.rust, self.n = server, cano, rust, n
        self.parar_arq = self.raiz / "parar"
        self.nomes = [f"c{i:02d}" for i in range(n)]

    def ambiente_extra(self):
        return {"CP_RUST_SERVER_BIN": self.server, "CP_RUST_CANO_BIN": self.cano,
                "CP_RUST_NO_ORPHAN_SWEEP": "1", "CP_RUST_SERVER": "1" if self.rust else "0"}

    def raizes_projetos(self):
        return []

    def preparar(self):
        for d in (self.home / ".claude", self.home / ".codex", self.bin, self.work):
            d.mkdir(parents=True, exist_ok=True)
        (self.bin / "tmux").write_text(f'#!/bin/sh\nexec /usr/bin/tmux -L {self.tmux} "$@"\n')
        (self.bin / "codex").write_text(FALSO_CODEX.replace("__PARAR__", repr(str(self.parar_arq))))
        for f in ("tmux", "codex"):
            (self.bin / f).chmod(0o755)
        for nome in self.nomes:
            (self.work / nome).mkdir()

    def processos(self):
        """Canos e codex de mentira desta prova (pelo HOME no ambiente)."""
        canos, codex = {}, []
        alvo = str(self.home).encode()
        falso = str(self.bin / "codex").encode()
        for p in Path("/proc").iterdir():
            if not p.name.isdigit():
                continue
            pid = int(p.name)
            env = environ(pid)
            if env.get(b"HOME") != alvo:
                continue
            if os.path.basename(base.exe(pid)) == "hangar-cano":
                canos[pid] = env.get(b"HANGAR_CANO_KEY", b"").decode()
            else:
                try:
                    if falso in (p / "cmdline").read_bytes():
                        codex.append(pid)
                except OSError:
                    pass
        return canos, codex

    def rollout(self, nome):
        try:
            meta = json.loads((self.home / f".hangar/codex-sessions/{nome}.json").read_text())
        except (OSError, ValueError):
            return False
        return bool(meta.get("rollout_path")) and Path(meta["rollout_path"]).is_file()

    def estados(self):
        st, lista, *_ = self.api("GET", "/api/sessions", timeout=20)
        return {s["name"]: s.get("state") for s in lista or [] if s.get("name") in self.nomes} if st == 200 else {}


class Chat(threading.Thread):
    def __init__(self, m, nome):
        super().__init__(daemon=True)
        self.m, self.nome = m, nome
        self.contagem = {}

    def run(self):
        import http.client
        c = http.client.HTTPConnection("127.0.0.1", self.m.porta, timeout=300)
        c.request("GET", f"/api/sessions/{self.nome}/events", headers={"Authorization": "Bearer " + self.m.token})
        r = c.getresponse()
        ev = None
        while True:
            line = r.readline()
            if not line:
                return
            line = line.decode().rstrip("\r\n")
            if line.startswith("event:"):
                ev = line[6:].strip()
            elif line == "":
                if ev:
                    self.contagem[ev] = self.contagem.get(ev, 0) + 1
                ev = None


def janela(grupos, chats):
    antes = {k: sum(cpu(p) for p in ps) for k, ps in grupos.items()}
    ev0 = [dict(c.contagem) for c in chats]
    t0 = time.monotonic()
    time.sleep(JANELA_S)
    dur = time.monotonic() - t0
    out = {k: round((sum(cpu(p) for p in ps) - antes[k]) / dur * 1000, 1) for k, ps in grupos.items()}
    out["total"] = round(sum(out[k] for k in grupos), 1)
    for tipo in ("state", "preview", "message"):
        out[f"{tipo}_por_s"] = round(sum(c.contagem.get(tipo, 0) - e.get(tipo, 0) for c, e in zip(chats, ev0)) / dur, 2)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("server")
    ap.add_argument("cano")
    ap.add_argument("rotulo")
    ap.add_argument("--rust", type=int, choices=(0, 1), required=True)
    ap.add_argument("--n", type=int, default=10)
    ap.add_argument("--relatorio")
    args = ap.parse_args()
    m = Medida(args.server, args.cano, bool(args.rust), args.n)
    reais = canos_de_fora(m.home)
    out = {"rotulo": args.rotulo, "rust": args.rust, "n": args.n, "canos_reais_antes": sorted(reais)}
    try:
        m.preparar()
        m.subir()
        py, rs = m.main_pid(), m.rust_pid()
        if bool(rs) != m.rust:
            print(m.log.read_text()[-4000:], file=sys.stderr)
            raise SystemExit(f"hangar-server {'ausente' if m.rust else 'de pé'} com --rust {args.rust}")
        out["recriadas"] = 0
        for nome in m.nomes:
            # Uma de cada vez: o chat só abre com o rollout da conversa no disco. A conversa que não
            # abre (laço initialize → thread/start no motor, ver medicao-5b.md) é apagada e recriada.
            for tentativa in range(TENTATIVAS):
                st, corpo, *_ = m.api("POST", "/api/sessions", {"name": nome, "cwd": str(m.work / nome),
                                                                "provider": "codex", "headless": True}, timeout=120)
                if st != 200:
                    raise SystemExit(f"criar {nome}: {st} {base.codigo(corpo)}")
                if base.esperar(lambda: m.rollout(nome), 10, 0.2):
                    break
                out["recriadas"] += 1
                m.api("DELETE", f"/api/sessions/{nome}", timeout=30)
                base.esperar(lambda: nome not in m.estados(), 15, 0.5)
            else:
                raise SystemExit(f"{nome}: conversa não abriu em {TENTATIVAS} tentativas")
        if not base.esperar(lambda: set(m.estados().values()) == {"idle"} and len(m.estados()) == m.n, 90, 1):
            print(m.log.read_text()[-4000:], file=sys.stderr)
            raise SystemExit(f"sessões não ficaram idle: {m.estados()}")
        chats = [Chat(m, nome) for nome in m.nomes]
        for c in chats:
            c.start()
        if not base.esperar(lambda: all(c.contagem.get("state") for c in chats), 30):
            raise SystemExit("nem todo chat recebeu `state`")
        canos, codex = m.processos()
        chaves = list(canos.values())
        out["canos"] = len(canos)
        out["codex"] = len(codex)
        if len(canos) != m.n or len(set(chaves)) != m.n or "" in chaves or len(codex) != m.n:
            raise SystemExit(f"esperado 1 cano e 1 codex por sessão: {len(canos)} canos, {len(set(chaves))} chaves, {len(codex)} codex")
        grupos = {"python": [py], "rust": [rs] if rs else [], "canos": list(canos), "codex": codex}
        out["janelas"] = {}
        for modo in ("parado", "trabalhando"):
            if modo == "trabalhando":
                m.parar_arq.unlink(missing_ok=True)
                for nome in m.nomes:
                    st, corpo, *_ = m.api("POST", f"/api/sessions/{nome}/input", {"text": "trabalhe"}, timeout=60)
                    if st != 200:
                        raise SystemExit(f"input {nome}: {st} {base.codigo(corpo)}")
                if not base.esperar(lambda: set(m.estados().values()) == {"working"}, 30, 0.5):
                    raise SystemExit(f"nem toda sessão foi a working: {m.estados()}")
            time.sleep(ASSENTAR_S)
            out["janelas"][modo] = [janela(grupos, chats) for _ in range(JANELAS)]
            if modo == "trabalhando":
                out["rss_trabalhando_mb"] = {k: round(sum(status_kb(p, "VmRSS") for p in ps) / 1024, 1)
                                             for k, ps in grupos.items()}
                if set(m.estados().values()) != {"working"}:
                    raise SystemExit(f"sessão saiu de working durante a janela: {m.estados()}")
        out["pico_rss_mb"] = {k: round(sum(status_kb(p, "VmHWM") for p in ps) / 1024, 1) for k, ps in grupos.items()}
        m.parar_arq.write_text("")
        base.esperar(lambda: set(m.estados().values()) == {"idle"}, 30, 0.5)
        canos2, _ = m.processos()
        if set(canos2) != set(canos):
            raise SystemExit("os canos mudaram durante a medida (religou ou morreu)")
        for nome in m.nomes:
            m.api("DELETE", f"/api/sessions/{nome}", timeout=30)
        out["canos_depois_de_apagar"] = len(m.processos()[0])
    finally:
        m.limpar()
        depois = canos_de_fora(m.home)
        out["canos_reais_depois"] = sorted(depois)
        if set(reais) - set(depois):
            print(f"PARE: canos reais sumiram: {sorted(set(reais) - set(depois))}", file=sys.stderr)
    for modo, js in (out.get("janelas") or {}).items():
        out.setdefault("mediana", {})[modo] = {k: statistics.median(j[k] for j in js) for k in js[0]}
    texto = json.dumps(out, ensure_ascii=False)
    print(texto)
    if args.relatorio:
        with open(args.relatorio, "a") as f:
            f.write(texto + "\n")
    if set(reais) - set(depois):
        raise SystemExit(2)


if __name__ == "__main__":
    main()
