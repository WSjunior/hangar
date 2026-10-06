#!/usr/bin/env python3
"""Mede o estado ao vivo de Claude com terminal com N chats abertos (parte 4, Task 5).

    scripts/medir-estado.py <binário hangar-server> <binário hangar-cano> <rótulo> [--relatorio arq.md]

Backend isolado (classe `Prova` de `prova-dono-unico.py`: HOME, portas, token e `tmux -L` próprios),
binários em release passados de fora: o "antes" sai de um `git archive` da base, para o código
novo não entrar nele. Sessões: pane que roda `claude --session-id <sid>` de mentira (um Python que
desenha a tela do Claude), transcript e marcador no HOME isolado. Cada sessão lê o próprio modo:

- `parado`: tela parada, sem spinner (sessão ociosa);
- `trabalhando`: spinner girando e a resposta crescendo a cada 0,1 s;
- `congelado`: spinner parado; o marcador do hook decide `working`/`idle` (latência).

Mede, com 1, 5 e 20 chats abertos (`/api/sessions/<nome>/events` do dono), parado e trabalhando:
CPU do Python, do Rust e do servidor tmux em ms por segundo (20 s), eventos `state`/`preview` por
segundo e RSS. Latência = escrita do marcador até o `state` com o valor novo chegar ao chat (10
vezes, com 1 e com 20 chats). Sem Claude real, sem conversa real: o relatório traz só números.
"""
import argparse
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
CHATS = (1, 5, 20)
JANELA_S = 20.0
ASSENTAR_S = 8.0
LATENCIAS = 10

FALSO_CLAUDE = r'''
import os, sys, time
modo_arq = sys.argv[1]
GIROS = "✻✽✶✺✢·"
RUA = "─" * 78
i = 0
t0 = time.time()
while True:
    try:
        modo = open(modo_arq).read().strip()
    except OSError:
        modo = "parado"
    if modo == "trabalhando":
        topo = f"● Resposta em voo, pedaço {i}\n\n{GIROS[i % len(GIROS)]} Thinking… ({int(time.time() - t0)}s · esc to interrupt)\n"
    elif modo == "congelado":
        topo = "● Resposta em voo, pedaço fixo\n\n✻ Thinking… (3s · esc to interrupt)\n"
    else:
        topo = "● Resposta pronta.\n\n"
    sys.stdout.write("\x1b[H\x1b[2J❯ pergunta\n\n" + topo + RUA + "\n❯ \n" + RUA)
    sys.stdout.flush()
    i += 1
    time.sleep(0.1 if modo == "trabalhando" else 1.0)
'''


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

    def raizes_projetos(self):
        return []

    def preparar(self):
        for d in (self.home / ".claude/.hangar-state", self.bin, self.work):
            d.mkdir(parents=True, exist_ok=True)
        (self.bin / "tmux").write_text(f'#!/bin/sh\nexec /usr/bin/tmux -L {self.tmux} "$@"\n')
        (self.bin / "tmux").chmod(0o755)
        falso = self.raiz / "falso_claude.py"
        falso.write_text(FALSO_CLAUDE)
        for i in range(N):
            name, sid = f"s{i:02d}", str(uuid.uuid4())
            cwd = self.work / name
            cwd.mkdir()
            proj = self.home / ".claude/projects" / "".join(c if c.isalnum() else "-" for c in str(cwd))
            proj.mkdir(parents=True, exist_ok=True)
            linhas = "".join(json.dumps({"type": "user", "uuid": f"u{n}", "timestamp": "2026-10-06T10:00:00.000Z",
                                         "message": {"role": "user", "content": f"linha {n}"}}) + "\n" for n in range(20))
            (proj / f"{sid}.jsonl").write_text(linhas)
            self.marcar(sid, "idle")
            self.modo(name, "parado")
            cmd = f"exec -a claude python3 {falso} {cwd / 'modo'} --session-id {sid}"
            subprocess.run(["/usr/bin/tmux", "-L", self.tmux, "new-session", "-d", "-s", name, "-x", "80", "-y", "24",
                            "-c", str(cwd), "bash", "-c", cmd], check=True)
            self.sids[name] = sid

    def marcar(self, sid, state):
        p = self.home / ".claude/.hangar-state" / f"{sid}.json"
        tmp = p.with_suffix(".tmp")
        tmp.write_text(json.dumps({"state": state, "ts": time.time()}))
        os.replace(tmp, p)

    def modo(self, name, modo):
        p = self.work / name / "modo"
        tmp = p.with_suffix(".tmp")
        tmp.write_text(modo)
        os.replace(tmp, p)


def cpu(pid):
    f = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    return sum(int(f[i]) for i in (11, 12, 13, 14)) / os.sysconf("SC_CLK_TCK")


def rss_mb(pid):
    for linha in Path(f"/proc/{pid}/status").read_text().splitlines():
        if linha.startswith("VmRSS:"):
            return round(int(linha.split()[1]) / 1024, 1)
    return 0.0


class Chat(threading.Thread):
    """Um chat aberto: conta os eventos e guarda o último `state` com a hora em que chegou."""

    def __init__(self, m, nome):
        super().__init__(daemon=True)
        self.m, self.nome = m, nome
        self.contagem = {}
        self.estado, self.quando = None, 0.0
        self.mudou = threading.Condition()

    def run(self):
        import http.client
        c = http.client.HTTPConnection("127.0.0.1", self.m.porta, timeout=120)
        c.request("GET", f"/api/sessions/{self.nome}/events", headers={"Authorization": "Bearer " + self.m.token})
        r = c.getresponse()
        ev, data = None, []
        while True:
            line = r.readline()
            if not line:
                return
            line = line.decode().rstrip("\r\n")
            if line.startswith("event:"):
                ev = line[6:].strip()
            elif line.startswith("data:"):
                data.append(line[5:].lstrip())
            elif line == "":
                if ev:
                    self.contagem[ev] = self.contagem.get(ev, 0) + 1
                    if ev == "state":
                        with self.mudou:
                            self.estado, self.quando = json.loads("\n".join(data)).get("state"), time.monotonic()
                            self.mudou.notify_all()
                ev, data = None, []


def janela(pids, chats):
    antes = {k: cpu(p) for k, p in pids.items()}
    ev0 = [dict(c.contagem) for c in chats]
    t0 = time.monotonic()
    time.sleep(JANELA_S)
    dur = time.monotonic() - t0
    out = {k: round((cpu(p) - antes[k]) / dur * 1000, 1) for k, p in pids.items()}
    for tipo in ("state", "preview"):
        n = sum(c.contagem.get(tipo, 0) - e.get(tipo, 0) for c, e in zip(chats, ev0))
        out[f"{tipo}_por_s"] = round(n / dur, 2)
    return out


def latencia(m, chat):
    m.modo(chat.nome, "congelado")
    sid = m.sids[chat.nome]
    m.marcar(sid, "idle")
    base.esperar(lambda: chat.estado == "idle", 20, 0.05)
    lat = []
    for k in range(LATENCIAS):
        for alvo in ("working", "idle"):
            t0 = time.monotonic()
            m.marcar(sid, alvo)
            ok = base.esperar(lambda: chat.estado == alvo, 10, 0.005)
            if alvo == "working":
                lat.append(round(time.monotonic() - t0, 3) if ok else None)
        time.sleep(0.3 + 0.37 * k % 0.75)
    m.modo(chat.nome, "parado")
    vals = [x for x in lat if x is not None]
    return {"amostras_s": lat, "mediana_s": statistics.median(vals) if vals else None, "max_s": max(vals) if vals else None}


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("server")
    ap.add_argument("cano")
    ap.add_argument("rotulo")
    ap.add_argument("--relatorio")
    args = ap.parse_args()
    m = Medida(args.server, args.cano)
    out = {"rotulo": args.rotulo, "chats": {}}
    try:
        m.preparar()
        m.subir()
        py, rs = m.main_pid(), m.rust_pid()
        if not rs:
            print(m.log.read_text()[-4000:], file=sys.stderr)
            raise SystemExit("hangar-server não está de pé: a medida seria do Python sozinho")
        tmux_pid = int(subprocess.run(["/usr/bin/tmux", "-L", m.tmux, "display", "-p", "#{pid}"],
                                      capture_output=True, text=True).stdout.strip())
        pids = {"python": py, "rust": rs, "tmux": tmux_pid}
        time.sleep(5)
        chats = []
        for n in CHATS:
            while len(chats) < n:
                c = Chat(m, f"s{len(chats):02d}")
                c.start()
                chats.append(c)
            if not base.esperar(lambda: all(c.estado is not None for c in chats), 30):
                raise SystemExit(f"nem todo chat recebeu `state` com {n} abertos")
            r = {}
            for modo in ("parado", "trabalhando"):
                for c in chats:
                    m.modo(c.nome, modo)
                alvo = "idle" if modo == "parado" else "working"
                if not base.esperar(lambda: all(c.estado == alvo for c in chats), 30):
                    raise SystemExit(f"{modo}: chats não chegaram a {alvo}: {[c.estado for c in chats]}")
                time.sleep(ASSENTAR_S)
                r[modo] = janela(pids, chats)
            for c in chats:
                m.modo(c.nome, "parado")
            r["rss_mb"] = {"python": rss_mb(py), "rust": rss_mb(rs)}
            if n in (1, CHATS[-1]):
                base.esperar(lambda: all(c.estado == "idle" for c in chats), 30)
                r["latencia_marcador"] = latencia(m, chats[0])
            out["chats"][n] = r
    finally:
        m.limpar()
    texto = json.dumps(out, ensure_ascii=False)
    print(texto)
    if args.relatorio:
        with open(args.relatorio, "a") as f:
            f.write(texto + "\n")


if __name__ == "__main__":
    main()
