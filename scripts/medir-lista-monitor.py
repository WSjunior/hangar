#!/usr/bin/env python3
"""Mede o tique da lista do dono com chats abertos (parte 4, Task 6): capturas `capture-pane` por
segundo e CPU, para conferir que a linha com `Monitor` vivo não é capturada de novo pela lista.

    scripts/medir-lista-monitor.py <binário hangar-server> <binário hangar-cano> <rótulo> [--relatorio arq.md]

Mesma montagem de `medir-estado.py` (backend isolado, 20 panes de Claude de mentira), sem marcador
do hook: a lista cai no pane a cada tique, que é o caso em que ela mais captura. A lista do dono fica
aberta (`/api/sessions/events`) e 5 chats também; o `tmux` do PATH conta cada `capture-pane` (a
captura do `Monitor` é o cliente `-C`, fora da conta). Janela de 20 s parado e trabalhando. Rode
com o `REPO` de cada lado: o "antes" sai do `git archive` da base, com o backend dela.
"""
import argparse
import http.client
import importlib.util
import json
import threading
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("medir_estado", REPO / "scripts/medir-estado.py")
me = importlib.util.module_from_spec(spec)
spec.loader.exec_module(me)
base = me.base

CHATS = 5


class Medida(me.Medida):
    def preparar(self):
        super().preparar()
        self.capturas = self.raiz / "capturas.log"
        (self.bin / "tmux").write_text(
            "#!/bin/sh\n"
            f'[ "$1" = capture-pane ] && echo x >> "{self.capturas}"\n'
            f'exec /usr/bin/tmux -L {self.tmux} "$@"\n')
        for f in (self.home / ".claude/.hangar-state").glob("*.json"):
            f.unlink()

    def contadas(self):
        try:
            return len(self.capturas.read_bytes().splitlines())
        except OSError:
            return 0


class Lista(threading.Thread):
    def __init__(self, m):
        super().__init__(daemon=True)
        self.m, self.estados = m, {}

    def run(self):
        c = http.client.HTTPConnection("127.0.0.1", self.m.porta, timeout=120)
        c.request("GET", "/api/sessions/events", headers={"Authorization": "Bearer " + self.m.token})
        r = c.getresponse()
        ev = None
        for line in r:
            line = line.decode().rstrip("\r\n")
            if line.startswith("event:"):
                ev = line[6:].strip()
            elif line.startswith("data:") and ev == "sessions":
                self.estados = {s["name"]: s.get("state") for s in json.loads(line[5:])}


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("server")
    ap.add_argument("cano")
    ap.add_argument("rotulo")
    ap.add_argument("--relatorio")
    args = ap.parse_args()
    m = Medida(args.server, args.cano)
    out = {"rotulo": args.rotulo, "chats": CHATS}
    try:
        m.preparar()
        m.subir()
        py, rs = m.main_pid(), m.rust_pid()
        if not rs:
            raise SystemExit("hangar-server não está de pé: a medida seria do Python sozinho")
        tmux_pid = int(me.subprocess.run(["/usr/bin/tmux", "-L", m.tmux, "display", "-p", "#{pid}"],
                                         capture_output=True, text=True).stdout.strip())
        pids = {"python": py, "rust": rs, "tmux": tmux_pid}
        lista = Lista(m)
        lista.start()
        chats = [me.Chat(m, f"s{i:02d}") for i in range(CHATS)]
        for c in chats:
            c.start()
        if not base.esperar(lambda: all(c.estado is not None for c in chats) and len(lista.estados) == me.N, 30):
            raise SystemExit("chats ou lista sem estado")
        for modo in ("parado", "trabalhando"):
            for nome in m.sids:
                m.modo(nome, modo)
            alvo = "idle" if modo == "parado" else "working"
            if not base.esperar(lambda: all(v == alvo for v in lista.estados.values()), 40):
                raise SystemExit(f"{modo}: lista não chegou a {alvo}: {lista.estados}")
            time.sleep(me.ASSENTAR_S)
            n0, t0 = m.contadas(), time.monotonic()
            r = me.janela(pids, chats)
            r["capturas_por_s"] = round((m.contadas() - n0) / (time.monotonic() - t0), 2)
            out[modo] = r
        out["rss_mb"] = {"python": me.rss_mb(py), "rust": me.rss_mb(rs)}
    finally:
        m.limpar()
    texto = json.dumps(out, ensure_ascii=False)
    print(texto)
    if args.relatorio:
        with open(args.relatorio, "a") as f:
            f.write(texto + "\n")


if __name__ == "__main__":
    main()
