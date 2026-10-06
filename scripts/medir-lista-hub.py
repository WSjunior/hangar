#!/usr/bin/env python3
"""Mede a lista do dono com 20 sessões Claude de mentira no backend isolado (lista-estado, Tasks 17-18).

    scripts/medir-lista-hub.py <binário hangar-server> <binário hangar-cano> <rótulo>

Sessões: pane que roda `claude --session-id <sid>` (python dormindo com argv[0]=claude), transcript e
marcador `idle` no HOME isolado. Mede com uma lista do dono aberta: CPU do Python, do Rust (com
filhos) e do servidor tmux em 60 s; tempo do marcador mudar até o `sessions` do SSE mostrar a mudança
(10 vezes); `GET /api/sessions` (20 vezes); pico de RSS. Binários em release (antes e depois).
`MANTER=1` guarda a pasta temporária para ler o log do backend.
"""
import importlib.util
import json
import os
import statistics
import subprocess
import sys
import threading
import time
import uuid
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("base", REPO / "scripts/prova-dono-unico.py")
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)

N = 20


class Args:
    conta_b = None
    manter = os.environ.get("MANTER") == "1"


class Medida(base.Prova):
    def __init__(self, server, cano):
        super().__init__(Args())
        self.server, self.cano = server, cano
        self.sids = {}

    def ambiente_extra(self):
        return {"CP_RUST_SERVER_BIN": self.server, "CP_RUST_CANO_BIN": self.cano}

    def preparar(self):
        for d in (self.home / ".claude/.hangar-state", self.bin, self.work):
            d.mkdir(parents=True, exist_ok=True)
        (self.bin / "tmux").write_text(f'#!/bin/sh\nexec /usr/bin/tmux -L {self.tmux} "$@"\n')
        (self.bin / "tmux").chmod(0o755)
        for i in range(N):
            name, sid = f"s{i:02d}", str(uuid.uuid4())
            cwd = self.work / name
            cwd.mkdir()
            proj = self.home / ".claude/projects" / "".join(c if c.isalnum() else "-" for c in str(cwd))
            proj.mkdir(parents=True, exist_ok=True)
            linhas = "".join(json.dumps({"type": "user", "uuid": f"u{n}", "timestamp": "2026-10-05T10:00:00.000Z",
                                         "message": {"role": "user", "content": f"linha {n}"}}) + "\n" for n in range(50))
            (proj / f"{sid}.jsonl").write_text(linhas)
            self.marcar(sid, "idle")
            cmd = f"exec -a claude python3 -c 'import time; time.sleep(1e9)' --session-id {sid}"
            subprocess.run(["/usr/bin/tmux", "-L", self.tmux, "new-session", "-d", "-s", name, "-c", str(cwd),
                            "bash", "-c", cmd], check=True)
            self.sids[name] = sid

    def marcar(self, sid, state):
        p = self.home / ".claude/.hangar-state" / f"{sid}.json"
        tmp = p.with_suffix(".tmp")
        tmp.write_text(json.dumps({"state": state, "ts": time.time()}))
        os.replace(tmp, p)


def cpu(pid):
    f = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    return sum(int(f[i]) for i in (11, 12, 13, 14)) / os.sysconf("SC_CLK_TCK")


def hwm(pid):
    for l in Path(f"/proc/{pid}/status").read_text().splitlines():
        if l.startswith("VmHWM:"):
            return int(l.split()[1])
    return 0


class Leitor(threading.Thread):
    """Lista do dono aberta: guarda o último `sessions` e quando chegou."""

    def __init__(self, m):
        super().__init__(daemon=True)
        self.m, self.ultimo, self.quando, self.eventos = m, None, 0.0, {}
        self.mudou = threading.Condition()

    def run(self):
        import http.client
        c = http.client.HTTPConnection("127.0.0.1", self.m.porta, timeout=60)
        c.request("GET", "/api/sessions/events", headers={"Authorization": "Bearer " + self.m.token})
        r = c.getresponse()
        ev, data = None, []
        while True:
            line = r.fp.readline()
            if not line:
                return
            line = line.decode().rstrip("\r\n")
            if line.startswith("event:"):
                ev = line[6:].strip()
            elif line.startswith("data:"):
                data.append(line[5:].lstrip())
            elif line == "":
                if ev:
                    self.eventos[ev] = self.eventos.get(ev, 0) + 1
                    if ev == "sessions":
                        with self.mudou:
                            self.ultimo, self.quando = json.loads("\n".join(data)), time.monotonic()
                            self.mudou.notify_all()
                ev, data = None, []

    def estado(self, nome):
        rows = self.ultimo or []
        return next((r["state"] for r in rows if r["name"] == nome), None)


def main():
    server, cano, rotulo = sys.argv[1:4]
    m = Medida(server, cano)
    out = {"rotulo": rotulo}
    try:
        m.preparar()
        m.subir()
        py = m.main_pid()
        rs = m.rust_pid()
        if not rs:
            print(m.log.read_text()[-4000:], file=sys.stderr)
            raise SystemExit("hangar-server não está de pé")
        tmux_pid = int(subprocess.run(["/usr/bin/tmux", "-L", m.tmux, "display", "-p", "#{pid}"],
                                      capture_output=True, text=True).stdout.strip())
        leitor = Leitor(m)
        leitor.start()
        assert base.esperar(lambda: leitor.ultimo is not None and len(leitor.ultimo) == N, 60), "lista não chegou"
        out["linhas"] = len(leitor.ultimo)
        out["estados"] = sorted({r["state"] for r in leitor.ultimo})
        time.sleep(10)
        c0 = (cpu(py), cpu(rs), cpu(tmux_pid), time.monotonic())
        time.sleep(60)
        c1 = (cpu(py), cpu(rs), cpu(tmux_pid), time.monotonic())
        dur = c1[3] - c0[3]
        out["cpu_ms_por_s"] = {"python": round((c1[0] - c0[0]) / dur * 1000, 1),
                               "rust": round((c1[1] - c0[1]) / dur * 1000, 1),
                               "tmux": round((c1[2] - c0[2]) / dur * 1000, 1)}
        lat = []
        for k in range(10):
            alvo = "working"
            nome = f"s{k:02d}"
            t0 = time.monotonic()
            m.marcar(m.sids[nome], alvo)
            ok = base.esperar(lambda: leitor.estado(nome) == alvo, 15, 0.01)
            lat.append(round(time.monotonic() - t0, 3) if ok else None)
            m.marcar(m.sids[nome], "idle")
            base.esperar(lambda: leitor.estado(nome) == "idle", 15, 0.05)
            time.sleep(0.7 + 0.37 * k % 1.5)
        out["latencia_s"] = lat
        vals = [x for x in lat if x is not None]
        out["latencia_mediana_max_s"] = (statistics.median(vals), max(vals)) if vals else None
        gets = []
        for _ in range(20):
            st, dados, t, _h = m.api("GET", "/api/sessions", timeout=20)
            assert st == 200 and len(dados) == N, st
            gets.append(t)
            time.sleep(0.3)
        out["get_s_mediana_max"] = (statistics.median(gets), max(gets))
        out["pico_rss_kb"] = {"python": hwm(py), "rust": hwm(rs)}
        out["eventos"] = leitor.eventos
    finally:
        m.limpar()
    print(json.dumps(out, ensure_ascii=False))


if __name__ == "__main__":
    main()
