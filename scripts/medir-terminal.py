#!/usr/bin/env python3
"""Medida do terminal real (parte 4, Task 8): vazão, eco de tecla, CPU por MB e memória por painel.

Sobe o backend desta árvore isolado (classe `Prova` de `prova-dono-unico.py`: HOME, portas, token e
servidor `tmux -L` próprios, `hangar-server` em release), abre o painel pelo WebSocket do dono e mede:

- vazão: `cat` de um arquivo de 50 MB no pane, do Enter até o marcador do fim chegar ao cliente;
- eco: 50 teclas uma a uma, do envio até o primeiro quadro de volta (mediana e p95);
- CPU por MB: `utime+stime` de `/proc/<pid>/stat` do Python, do Rust e do servidor tmux durante o `cat`;
- memória: RSS e threads do Python e do Rust antes e depois de abrir 10 painéis.

    scripts/medir-terminal.py [--reserva] [--relatorio arquivo.md] [--manter]

`--reserva` sobe com `CP_RUST_SERVER=0` (o Python sozinho na porta). Sem Claude nem sessão real:
as sessões são `bash` no tmux da prova. O relatório traz só números.
"""
import argparse
import importlib.util
import os
import signal
import statistics
import subprocess
import sys
import time
from pathlib import Path

from websockets.sync.client import connect

_spec = importlib.util.spec_from_file_location("prova_dono_unico", Path(__file__).with_name("prova-dono-unico.py"))
base = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(base)

MB = 50
PAINEIS = 10
TECLAS = 50
TICK = os.sysconf("SC_CLK_TCK")


def cpu(pid):
    """Segundos de CPU do processo (todas as threads), ou 0 sem processo."""
    try:
        campos = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
        return (int(campos[11]) + int(campos[12])) / TICK
    except (OSError, IndexError, ValueError):
        return 0.0


def status(pid):
    """(RSS em MB, threads)."""
    try:
        linhas = dict(l.split(":", 1) for l in Path(f"/proc/{pid}/status").read_text().splitlines() if ":" in l)
        return int(linhas["VmRSS"].split()[0]) / 1024, int(linhas["Threads"])
    except (OSError, KeyError, ValueError):
        return 0.0, 0


class Medida(base.Prova):
    PERFIL = "release"

    def preparar(self):
        server = base.REPO / "crates/target/release/hangar-server"
        cano = base.REPO / "crates/target/release/hangar-cano"
        # Sempre: o build é incremental, e um binário velho (outro contrato) mediria o Python calado.
        subprocess.run(["cargo", "build", "--locked", "--release", "-p", "hangar-server", "-p", "hangar-cano"],
                       cwd=base.REPO / "crates", check=True, env={**os.environ, "CARGO_BUILD_JOBS": "4"})
        if not (server.exists() and cano.exists()):
            raise SystemExit("binários release não encontrados")
        for d in (self.home / ".claude", self.bin, self.work):
            d.mkdir(parents=True)
        (self.bin / "tmux").write_text(f'#!/bin/sh\nexec /usr/bin/tmux -L {self.tmux} "$@"\n')
        (self.bin / "tmux").chmod(0o755)
        self.arquivo = self.raiz / "grande.txt"
        linha = (("0123456789abcdefghijklmnopqrstuvwxyz" * 3)[:99] + "\n").encode()
        with open(self.arquivo, "wb") as f:
            for _ in range(MB * 1024 * 1024 // len(linha)):
                f.write(linha)

    def ambiente_extra(self):
        return {"CP_RUST_SERVER": "0"} if self.args.reserva else {}

    def raizes_projetos(self):
        return []

    def tmux_cru(self, *args):
        return subprocess.run(["/usr/bin/tmux", "-L", self.tmux, *args], capture_output=True, text=True,
                              env={**os.environ, "HOME": str(self.home)})

    def sessao_bash(self, nome):
        r = self.tmux_cru("new-session", "-d", "-s", nome, "-x", "200", "-y", "50", "-c", str(self.work),
                          "env", "-i", f"HOME={self.home}", "TERM=xterm-256color", "PS1=$ ",
                          "/bin/bash", "--norc", "--noprofile")
        if r.returncode != 0:
            raise RuntimeError(f"tmux new-session falhou ({r.returncode})")

    def painel(self, nome):
        url = f"ws://127.0.0.1:{self.porta}/api/sessions/{nome}/term?token={self.token}&cols=200&rows=50"
        return connect(url, max_size=None, open_timeout=10, compression=None)

    def pids(self):
        tmux = self.tmux_cru("display", "-p", "#{pid}").stdout.strip()
        return {"python": self.main_pid(), "rust": self.rust_pid(), "tmux": int(tmux or 0)}


def drenar(ws, ate=0.5):
    while True:
        try:
            ws.recv(timeout=ate)
        except TimeoutError:
            return


def medir(m):
    m.sessao_bash("med0")
    pids = m.pids()
    if not m.args.reserva and not pids["rust"]:
        raise SystemExit("o hangar-server não está de pé: a medida seria do Python sozinho")
    ws = m.painel("med0")
    drenar(ws, 1.0)

    # Vazão e CPU por MB.
    marca = b"FIM-MED-2X"
    cpu0 = {k: cpu(p) for k, p in pids.items()}
    t0 = time.monotonic()
    ws.send(f"cat {m.arquivo}; echo FIM-MED-$((1+1))X\r".encode())
    recebidos, cauda = 0, b""
    while True:
        if time.monotonic() - t0 > 180:
            raise SystemExit("o marcador do fim do cat não chegou em 180 s")
        b = ws.recv(timeout=120)
        if isinstance(b, str):
            continue
        recebidos += len(b)
        cauda = (cauda + b)[-64:]
        if marca in cauda:
            break
    dur = time.monotonic() - t0
    cpu1 = {k: cpu(p) for k, p in pids.items()}
    drenar(ws)
    ms_por_mb = {k: round((cpu1[k] - cpu0[k]) * 1000 / MB, 1) for k in pids}

    # Eco de tecla.
    ws.send(b"clear\r")
    drenar(ws)
    ecos = []
    for _ in range(TECLAS):
        t = time.monotonic()
        ws.send(b"a")
        while not (isinstance(q := ws.recv(timeout=5), bytes) and b"a" in q):
            pass
        ecos.append((time.monotonic() - t) * 1000)
        drenar(ws, 0.05)
    ws.send(b"\x15")
    ecos.sort()

    # Memória por painel.
    for i in range(1, PAINEIS + 1):
        m.sessao_bash(f"med{i}")
    time.sleep(2)
    antes = {k: status(pids[k]) for k in ("python", "rust")}
    abertos = [m.painel(f"med{i}") for i in range(1, PAINEIS + 1)]
    for p in abertos:
        drenar(p, 0.3)
    time.sleep(2)
    depois = {k: status(pids[k]) for k in ("python", "rust")}
    for p in abertos + [ws]:
        p.close()

    return {
        "modo": "reserva (Python sozinho)" if m.args.reserva else "Rust de pé",
        "vazao_mb_s": round(MB / dur, 1), "segundos": round(dur, 2), "bytes_no_cliente_mb": round(recebidos / 2**20, 1),
        "cpu_ms_por_mb": ms_por_mb,
        "eco_ms": {"mediana": round(statistics.median(ecos), 2), "p95": round(ecos[min(len(ecos) - 1, int(len(ecos) * 0.95))], 2)},
        "rss_mb_por_painel": {k: round((depois[k][0] - antes[k][0]) / PAINEIS, 2) for k in antes if pids[k]},
        "threads_por_painel": {k: (depois[k][1] - antes[k][1]) / PAINEIS for k in antes if pids[k]},
        "rss_mb_total": {k: round(depois[k][0], 1) for k in depois if pids[k]},
    }


def tabela(r):
    linhas = [f"### {r['modo']}", "",
              "| Medida | Valor |", "|---|---|",
              f"| Vazão (`cat` de {MB} MB) | {r['vazao_mb_s']} MB/s ({r['segundos']} s; {r['bytes_no_cliente_mb']} MB no cliente) |",
              f"| Eco de tecla | mediana {r['eco_ms']['mediana']} ms, p95 {r['eco_ms']['p95']} ms |"]
    for k, v in r["cpu_ms_por_mb"].items():
        linhas.append(f"| CPU por MB, {k} | {v} ms |")
    for k, v in r["rss_mb_por_painel"].items():
        linhas.append(f"| RSS por painel, {k} | {v} MB ({r['threads_por_painel'][k]:+.0f} threads; total {r['rss_mb_total'][k]} MB) |")
    return "\n".join(linhas).replace(".", ",") + "\n"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--reserva", action="store_true", help="sobe com CP_RUST_SERVER=0")
    ap.add_argument("--relatorio", help="acrescenta a tabela neste arquivo")
    ap.add_argument("--manter", action="store_true", help="não apaga a pasta da medida")
    args = ap.parse_args()
    args.conta_b = None
    # Terminal fechado ou kill: o finally ainda para a unit e o tmux da medida.
    for sinal in (signal.SIGTERM, signal.SIGHUP):
        signal.signal(sinal, lambda *_: sys.exit(1))
    m = Medida(args)
    try:
        m.preparar()
        m.subir()
        r = medir(m)
    finally:
        m.limpar()
    texto = tabela(r)
    print(texto)
    if args.relatorio:
        with open(args.relatorio, "a") as f:
            f.write(texto + "\n")


if __name__ == "__main__":
    sys.exit(main())
