#!/usr/bin/env python3
"""Prova da troca: a lista do dono servida pelo `ListHub` (Rust) com sessões reais no backend isolado
(`docs/migracao-rust/pedidos/2026-10-05-lista-estado-prova-troca.md`).

Mesma montagem de `prova-lista-sombra.py`, sem `CP_LIST_SHADOW`. Cada cenário tem o esperado escrito
aqui antes de rodar (`ESPERADO`) e é conferido no `GET /api/sessions` e no último `sessions` do SSE
`/api/sessions/events` do dono; o que só aparece de passagem (`working`, cartão) é conferido no
histórico do SSE do cenário.

    scripts/prova-lista-troca.py [--cenarios nascer,...] [--conta-b ~/.claude-jefferson]
                                 [--reserva] [--quedas] [--convidado] [--relatorio f.md] [--manter]

`--reserva` sobe com `CP_RUST_SERVER=0` (lista pelo Python). `--quedas` derruba o Rust 3 vezes em
60 s depois dos cenários. `--convidado` roda o cenário do convidado. O contador
`registry.PYTHON_DISCOVERY` é gravado pelo lançador num arquivo da pasta da prova a cada 0,5 s, com o
modo do dono do runtime; o relatório traz só nomes de sessão, campos, códigos e tempos.
"""
import argparse
import http.client
import importlib.util
import json
import os
import signal
import statistics
import subprocess
import sys
import threading
import time
import traceback
from pathlib import Path

_spec = importlib.util.spec_from_file_location("prova_lista_sombra", Path(__file__).with_name("prova-lista-sombra.py"))
sombra = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(sombra)
base = sombra.base
esperar = base.esperar

LANCADOR = r'''
import threading as _th, time as _t, json as _j, os as _os
import app.api as _api
async def _sem_poda():
    return None
_api._prune_loop = _sem_poda
from app import registry as _reg, runtime_coordinator as _rc, guest_users as _gu
_conv = {{"n": 0}}
_fv0 = _gu.filter_visible
def _fv(guest, items, name_of):
    if guest is not None:
        _conv["n"] += 1
    return _fv0(guest, items, name_of)
_gu.filter_visible = _fv
def _despejo():
    while True:
        o = _rc.current()
        d = {{"t": _t.time(), "modo": o.mode if o else "sem_coordenador",
              "descoberta": dict(_reg.PYTHON_DISCOVERY), "convidado_python": _conv["n"]}}
        tmp = {arq!r} + ".tmp"
        with open(tmp, "w") as f:
            f.write(_j.dumps(d))
        _os.replace(tmp, {arq!r})
        _t.sleep(0.5)
_th.Thread(target=_despejo, daemon=True).start()
'''

# Esperado por cenário, escrito antes de rodar. Cada item: (descrição, função(linhas por nome) -> bool).
# `fim`: conferido no GET e no último `sessions` do SSE ao fim do cenário; `visto`: em algum
# `sessions` do SSE durante o cenário.
def _st(nome, *estados):
    return lambda L: (L.get(nome) or {}).get("state") in estados


def _fora(nome):
    return lambda L: nome not in L


def _campo(nome, campo, valor):
    return lambda L: (L.get(nome) or {}).get(campo) == valor


BASE4 = ("ct", "ch", "xt", "xh")
ESPERADO = {
    "nascer": {
        "fim": [(f"{n} idle", _st(n, "idle")) for n in BASE4]
        + [("ct claude com terminal", lambda L: (L.get("ct") or {}).get("provider") == "claude" and not L["ct"].get("headless")),
           ("ch claude sem terminal", lambda L: (L.get("ch") or {}).get("headless") is True),
           ("xt codex com terminal", lambda L: (L.get("xt") or {}).get("provider") == "codex" and not L["xt"].get("headless")),
           ("xh codex sem terminal", lambda L: (L.get("xh") or {}).get("provider") == "codex" and L["xh"].get("headless") is True),
           ("ct/ch com jsonl", lambda L: all((L.get(n) or {}).get("jsonl") for n in ("ct", "ch")))],
        "visto": [],
    },
    "trabalhando": {
        "fim": [(f"{n} idle", _st(n, "idle")) for n in BASE4],
        "visto": [(f"{n} working", _st(n, "working")) for n in BASE4],
    },
    "permissao": {
        "fim": [("pt e ph na lista", lambda L: "pt" in L and "ph" in L)],
        # pt com terminal não chegou ao cartão na sombra (igual nos dois lados); fica registrado, não esperado.
        "visto": [("ph awaiting_input", _st("ph", "awaiting_input"))],
    },
    "pergunta": {
        "fim": [("ct idle", _st("ct", "idle")), ("ch idle", _st("ch", "idle"))],
        "visto": [("ct awaiting_input com pergunta", lambda L: _st("ct", "awaiting_input")(L) and bool(L["ct"].get("question"))),
                  ("ch awaiting_input com pergunta", lambda L: _st("ch", "awaiting_input")(L) and bool(L["ch"].get("question")))],
    },
    "clear": {
        # mais "jsonl novo" em ct e ch, conferido contra o jsonl de antes (`c_clear`)
        "fim": [("ct idle", _st("ct", "idle")), ("ch idle", _st("ch", "idle"))],
        "visto": [],
    },
    "matar": {
        "fim": [("pt fora da lista", _fora("pt")), ("xt fora da lista", _fora("xt")),
                ("ph não working", lambda L: (L.get("ph") or {}).get("state") != "working")],
        "visto": [],
    },
    "recriar": {
        "fim": [(f"{n} idle", _st(n, "idle")) for n in ("ch", "xt", "ct")]
        + [("pt e ph fora", lambda L: "pt" not in L and "ph" not in L)],
        "visto": [],
    },
    "par_grupo": {
        "fim": [("ct pareado com xt, ch e xh", lambda L: set((L.get("ct") or {}).get("pair_peers") or []) >= {"xt", "ch", "xh"}),
                ("mesmo pair_gid nos quatro", lambda L: len({(L.get(n) or {}).get("pair_gid") for n in BASE4}) == 1
                 and (L.get("ct") or {}).get("pair_gid") is not None),
                ("pair_task = prova", _campo("ct", "pair_task", "prova"))],
        "visto": [],
    },
    "worktree": {
        "fim": [("wt idle", _st("wt", "idle")), ("wt branch prova-wt", _campo("wt", "branch", "prova-wt")),
                ("wt worktree", _campo("wt", "worktree", True))],
        "visto": [],
    },
    "plano": {
        # O nome do plano sai sem o prefixo de data (`planprog._DATE_PREFIX_RE`).
        "fim": [("ct plano prova-lista", _campo("ct", "plan_name", "prova-lista")),
                ("ct barra 1/3", lambda L: ((L.get("ct") or {}).get("plan_done"), L.get("ct", {}).get("plan_total")) == (1, 3))],
        "visto": [],
    },
    "conta": {
        "fim": [("ct idle", _st("ct", "idle")),
                # A conta é a pasta em que o `claude` roda: o embrulho troca `.claude-provab` pela conta B real.
                ("ct na conta B", lambda L: str((L.get("ct") or {}).get("conta") or "").endswith("/.claude-jefferson"))],
        "visto": [],
    },
    # restart: as sessões vivas antes continuam na lista (`c_restart`); convidado: `c_convidado`
    "restart": {"fim": [], "visto": []},
    "convidado": {"fim": [], "visto": []},
}


class ListaSSE(threading.Thread):
    """Lista do dono aberta como o app: guarda cada `sessions` com a hora de chegada e conta eventos."""

    def __init__(self, prova):
        super().__init__(daemon=True)
        self.p, self.parar = prova, False
        self.hist = []         # (monotonic, {nome: linha})
        self.eventos = {}
        self.list_error = []   # (monotonic, código)
        self.trava = threading.Lock()

    def run(self):
        while not self.parar:
            try:
                c = http.client.HTTPConnection("127.0.0.1", self.p.porta, timeout=40)
                c.request("GET", "/api/sessions/events", headers=self.p.cab())
                r = c.getresponse()
                ev, data = None, []
                while not self.parar:
                    linha = r.fp.readline()
                    if not linha:
                        break
                    linha = linha.decode(errors="replace").rstrip("\r\n")
                    if linha.startswith("event:"):
                        ev = linha[6:].strip()
                    elif linha.startswith("data:"):
                        data.append(linha[5:].lstrip())
                    elif linha == "" and ev:
                        agora = time.monotonic()
                        with self.trava:
                            self.eventos[ev] = self.eventos.get(ev, 0) + 1
                            if ev == "sessions":
                                rows = json.loads("\n".join(data))
                                self.hist.append((agora, {x["name"]: x for x in rows}))
                            elif ev == "list_error":
                                try:
                                    self.list_error.append((agora, json.loads("\n".join(data)).get("code")))
                                except ValueError:
                                    self.list_error.append((agora, "?"))
                        ev, data = None, []
            except (OSError, http.client.HTTPException, ValueError):
                pass
            time.sleep(0.5)

    def desde(self, t0):
        with self.trava:
            return [h for h in self.hist if h[0] >= t0]

    def ultima(self):
        with self.trava:
            return self.hist[-1][1] if self.hist else {}


class Marcadores(threading.Thread):
    """Vê o marcador do hook (`.hangar-state/<sid>.json`) das sessões Claude da prova mudar de estado e
    guarda (hora, sessão, estado) para medir marcador -> lista."""

    def __init__(self, prova, sse):
        super().__init__(daemon=True)
        self.p, self.sse, self.parar = prova, sse, False
        self.mudancas = []   # (wall time do mtime, monotonic visto, nome, estado)
        self.vistos = {}

    def run(self):
        while not self.parar:
            for nome, row in list(self.sse.ultima().items()):
                if row.get("provider") != "claude" or row.get("headless") or not row.get("jsonl"):
                    continue
                sid = Path(row["jsonl"]).stem
                for pasta in (self.p.home / ".claude/.hangar-state", self.p.home / ".claude-provab/.hangar-state"):
                    f = pasta / f"{sid}.json"
                    try:
                        st = f.stat()
                    except OSError:
                        continue
                    chave = (nome, sid)
                    if self.vistos.get(chave) == st.st_mtime_ns:
                        continue
                    self.vistos[chave] = st.st_mtime_ns
                    try:
                        estado = json.loads(f.read_text()).get("state")
                    except (OSError, ValueError):
                        continue
                    self.mudancas.append((st.st_mtime, time.monotonic(), nome, estado))
            time.sleep(0.01)


class Prova(sombra.Prova):
    def __init__(self, args):
        super().__init__(args)
        self.contador = self.raiz / "descoberta.json"
        self.reserva = args.reserva
        self.linhas_rel = []    # (cenário, esperado, visto, tempo)
        self.gets = []
        self.leituras_contador = []
        self.lat_detalhe = []   # (sessão, estado do marcador, segundos)

    def preparar(self):
        super().preparar()
        if self.conta_b:
            # Mesmas pastas de estado da conta B, senão a sessão trocada cai no pane.
            for sub in (".hangar-state", "sessions", ".hangar-askq", ".hangar-status"):
                destino = (self.conta_b / sub)
                destino.mkdir(exist_ok=True)
                (self.home / ".claude-provab" / sub).symlink_to(destino.resolve())

    def lancador_extra(self):
        return LANCADOR.format(arq=str(self.contador))

    def ambiente_extra(self):
        env = {"CP_SCAN_ROOTS": str(self.home)}
        if self.reserva:
            env["CP_RUST_SERVER"] = "0"
        return env

    def ler_contador(self):
        try:
            return json.loads(self.contador.read_text())
        except (OSError, ValueError):
            return {}

    # ---- conferência ----------------------------------------------------------------------
    def get_lista(self):
        t0 = time.perf_counter()   # o `api()` da base arredonda para 10 ms
        st, dados, _, _ = self.api("GET", "/api/sessions", timeout=20)
        self.gets.append(time.perf_counter() - t0)
        return st, {x["name"]: x for x in dados} if st == 200 and isinstance(dados, list) else {}

    def conferir(self, nome, t0, extra_fim=(), extra_visto=()):
        esp = ESPERADO.get(nome, {"fim": [], "visto": []})
        fim = list(esp["fim"]) + list(extra_fim)
        # A sessão pode ainda estar terminando o turno do último envio (a espera do cenário vê o `idle`
        # de antes da entrega): até 20 s para GET e SSE baterem com o esperado, e o tempo vai à nota.
        t_ass = time.monotonic()
        while True:
            time.sleep(1.5)
            st, L = self.get_lista()
            S = self.sse.ultima()
            if all(st == 200 and f(L) and f(S) for _, f in fim) or time.monotonic() - t_ass > 20:
                break
        assentou = time.monotonic() - t_ass
        hist = self.sse.desde(t0)
        res = []
        for desc, f in fim:
            ok_get, ok_sse = bool(st == 200 and f(L)), bool(f(S))
            det = f"GET {'ok' if ok_get else 'não'}, SSE {'ok' if ok_sse else 'não'}"
            if not (ok_get and ok_sse):
                det += "; " + self.retrato(L)
            res.append((desc, ok_get and ok_sse, det))
        for desc, f in list(esp["visto"]) + list(extra_visto):
            ok = any(f(rows) for _, rows in hist)
            res.append((desc + " (visto no SSE)", ok, f"{sum(1 for _, r in hist if f(r))}/{len(hist)} publicações"))
        # Sem terminal mostra o estado do runtime: nenhuma linha headless com `list_runtime_absent`.
        ausente = sorted({n for _, rows in hist for n, r in rows.items()
                          if r.get("headless") and r.get("problema") == "list_runtime_absent"})
        res.append(("sem list_runtime_absent", not ausente, ", ".join(ausente) or "nenhuma linha"))
        res.append(("assentou", True, f"{assentou:.1f} s"))
        return res

    def retrato(self, L):
        """Estado de cada linha e, com terminal, se o pane mostra turno em curso: separa erro da lista
        de sessão que de fato não parou."""
        partes = []
        for nome, r in sorted(L.items()):
            pane = ""
            if not r.get("headless"):
                tela = self.tela(nome)
                pane = " pane:" + ("turno" if "esc to interrupt" in tela else "diálogo" if "❯ 1." in tela else "prompt")
            partes.append(f"{nome}={r.get('state')}{pane}")
        return " ".join(partes)

    def latencias(self, t0):
        """Marcador -> primeira publicação do SSE com o mesmo estado, para as mudanças do cenário."""
        out = []
        hist = self.sse.desde(t0 - 1)
        agora_wall, agora_mono = time.time(), time.monotonic()
        for mt, _visto, nome, estado in [m for m in self.marcas_mud.mudancas if m[1] >= t0]:
            t_marc = agora_mono - (agora_wall - mt)
            pub = next((t for t, rows in hist if t >= t_marc - 0.05 and (rows.get(nome) or {}).get("state") == estado), None)
            if pub is not None and pub - t_marc < 10:
                out.append(max(0.0, pub - t_marc))
                self.lat_detalhe.append((nome, estado, round(max(0.0, pub - t_marc), 3)))
        return out

    def cenario(self, nome, corpo):
        t0 = time.monotonic()
        notas = []
        try:
            extra = corpo(notas) or {}
        except Exception as e:
            notas.append(f"cenário quebrou: {type(e).__name__}")
            traceback.print_exc()
            extra = {}
        res = self.conferir(nome, t0, extra.get("fim", ()), extra.get("visto", ()))
        lat = self.latencias(t0)
        c = self.ler_contador()
        self.leituras_contador.append((nome, c.get("modo"), c.get("descoberta")))
        self.linhas_rel.append((nome, res, lat, "; ".join(notas), c))
        print(f"[{nome}] {[(d, ok) for d, ok, _ in res]} lat {self.fmt_lat(lat)} modo {c.get('modo')} "
              f"descoberta {c.get('descoberta')} notas {notas}", flush=True)

    @staticmethod
    def fmt_lat(v):
        return f"mediana {statistics.median(v) * 1000:.0f} ms, máx {max(v) * 1000:.0f} ms ({len(v)})" if v else "—"

    # ---- cenários que mudam em relação à sombra ------------------------------------------------
    def c_clear(self, notas):
        antes = {n: (self.sessao(n) or {}).get("jsonl") for n in ("ct", "ch")}
        # Transcript novo no disco separa "a lista não trocou" de "o /clear não chegou ao agente".
        pasta = base.CONTA / "projects" / base.re.sub(r"[^A-Za-z0-9]", "-", str(self.work))
        n0 = len(list(pasta.glob("*.jsonl")))
        super().c_clear(notas)
        notas.append(f"transcripts novos no disco: {len(list(pasta.glob('*.jsonl'))) - n0}")
        return {"fim": [(f"{n} jsonl novo", lambda L, n=n: bool((L.get(n) or {}).get("jsonl")) and L[n]["jsonl"] != antes[n])
                        for n in ("ct", "ch")]}

    def do_dono(self):
        """Vivas que o dono vê: a do convidado (`gh`) é escondida dele."""
        return set(self.vivas) - {"gh"}

    def c_restart(self, notas):
        vivas = self.do_dono()
        super().c_restart(notas)
        return {"fim": [("vivas na lista: " + ",".join(sorted(vivas)), lambda L: vivas <= set(L))]}

    def c_convidado(self, notas):
        raiz = self.home / "convidado"
        raiz.mkdir(exist_ok=True)
        st, corpo, *_ = self.api("POST", "/api/guests", {"name": "prova", "root": str(raiz), "sees_owner": False,
                                                          "owner_sees": False})
        if st != 200:
            notas.append(f"criar convidado {st} {base.codigo(corpo)}")
            return {}
        token = corpo["token"]
        dono_token = self.token
        n0 = self.ler_contador().get("convidado_python", 0)
        try:
            self.token = token
            st, corpo, *_ = self.api("POST", "/api/sessions", {"name": "gh", "cwd": str(raiz), "headless": True,
                                                                "provider": "claude"}, timeout=180)
            notas.append(f"convidado cria gh {st} {base.codigo(corpo)}")
            time.sleep(4)
            st_g, lista_g, *_ = self.api("GET", "/api/sessions", timeout=20)
            # SSE do convidado: só o primeiro `sessions`.
            sse_g = None
            c = http.client.HTTPConnection("127.0.0.1", self.porta, timeout=20)
            c.request("GET", "/api/sessions/events", headers=self.cab())
            r = c.getresponse()
            ev = None
            fim = time.monotonic() + 15
            while time.monotonic() < fim:
                linha = r.fp.readline().decode(errors="replace").rstrip("\r\n")
                if linha.startswith("event:"):
                    ev = linha[6:].strip()
                elif linha.startswith("data:") and ev == "sessions":
                    sse_g = [x["name"] for x in json.loads(linha[5:])]
                    break
            c.close()
        finally:
            self.token = dono_token
        self.vivas.add("gh")
        nomes_g = sorted(x["name"] for x in lista_g or []) if st_g == 200 else None
        time.sleep(1.5)   # o despejo do contador é a cada 0,5 s
        n1 =self.ler_contador().get("convidado_python", 0)
        notas.append(f"convidado GET {st_g} {nomes_g}, SSE {sse_g}; filtro Python chamado {n1 - n0}x")
        dono = set(self.get_lista()[1])
        return {"fim": [
            ("convidado vê só gh (GET)", lambda L: nomes_g == ["gh"]),
            ("convidado vê só gh (SSE)", lambda L: sse_g == ["gh"]),
            ("lista do convidado passou pelo Python", lambda L: n1 > n0),
            ("dono não vê gh (GET)", lambda L: "gh" not in dono),
            ("dono não vê gh (SSE)", lambda L: "gh" not in L),
        ]}

    def quedas(self):
        """Derruba o Rust 3 vezes em 60 s: o Python assume a porta e a lista segue."""
        t0 = time.monotonic()
        notas, mortos = [], []
        for i in range(3):
            pid = esperar(self.rust_pid, 30)
            if not pid:
                notas.append(f"queda {i + 1}: Rust não estava de pé")
                break
            os.kill(pid, signal.SIGKILL)
            mortos.append(pid)
            if i < 2:
                esperar(lambda: self.rust_pid() not in (0, pid), 30)
        assumiu = esperar(lambda: "o Python assume a porta" in self.log.read_text(errors="replace"), 30)
        notas.append(f"{len(mortos)} quedas em {time.monotonic() - t0:.0f} s; Python assumiu: {'sim' if assumiu else 'não'}")
        time.sleep(6)
        c = self.ler_contador()
        vivas = self.do_dono()
        st, L = self.get_lista()
        sse_depois = self.sse.desde(t0 + 1)
        res = [("Python assumiu a porta", bool(assumiu), ""),
               ("GET 200 com as vivas", st == 200 and vivas <= set(L), f"GET {st}"),
               ("SSE segue publicando", bool(sse_depois), f"{len(sse_depois)} publicações"),
               ("modo python e descoberta Python rodando", c.get("modo") == "python" and sum((c.get("descoberta") or {}).values()) > 0,
                f"modo {c.get('modo')}, descoberta {c.get('descoberta')}")]
        self.linhas_rel.append(("Rust derrubado 3x em 60 s", res, [], "; ".join(notas), c))
        print(f"[quedas] {res} {notas}", flush=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--cenarios", default=",".join(sombra.ORDEM))
    ap.add_argument("--conta-b")
    ap.add_argument("--codex", default="~/.codex-claude-200-2")
    ap.add_argument("--reserva", action="store_true")
    ap.add_argument("--quedas", action="store_true")
    ap.add_argument("--convidado", action="store_true")
    ap.add_argument("--relatorio")
    ap.add_argument("--manter", action="store_true")
    ap.add_argument("--pausa", type=int, default=0, help="segura N s antes da limpeza, para inspecionar à mão")
    args = ap.parse_args()
    ordem = sombra.ORDEM[:-1] + (["convidado"] if args.convidado else []) + sombra.ORDEM[-1:]
    cenarios = [c for c in ordem if c in args.cenarios.split(",") or (c == "convidado" and args.convidado)]
    for sinal in (signal.SIGTERM, signal.SIGHUP):
        signal.signal(sinal, lambda *_: sys.exit(1))
    p = Prova(args)
    print(f"prova em {p.raiz}: porta {p.porta}, tmux -L {p.tmux}, reserva {args.reserva}", flush=True)
    p.sse = ListaSSE(p)
    p.marcas_mud = Marcadores(p, p.sse)
    try:
        p.preparar()
        p.subir()
        rust = esperar(lambda: "hangar-server de pé" in p.log.read_text(errors="replace"), 30)
        if args.reserva == bool(rust):
            raise SystemExit(f"Rust na frente: {bool(rust)}, esperado {not args.reserva}")
        p.sse.start()
        p.marcas_mud.start()
        time.sleep(4)
        p.confiar_pasta()
        for c in cenarios:
            p.cenario(c, getattr(p, f"c_{c}"))
        if args.quedas:
            p.quedas()
        if args.pausa:
            print(f"pausa {args.pausa} s: porta {p.porta} token {p.token} tmux -L {p.tmux}", flush=True)
            time.sleep(args.pausa)
    finally:
        p.sse.parar = p.marcas_mud.parar = True
        try:
            p.limpar()
        finally:
            linhas = ["| Cenário | Esperado | Visto | Marcador → lista (SSE) | Notas |", "|---|---|---|---|---|"]
            todas = []
            for nome, res, lat, notas, c in p.linhas_rel:
                todas += lat
                for i, (desc, ok, det) in enumerate(res):
                    linhas.append(f"| {nome if i == 0 else ''} | {desc} | {'sim' if ok else '**não**'} ({det}) | "
                                  f"{p.fmt_lat(lat) if i == 0 else ''} | {notas.replace('|', '/') if i == 0 else ''} |")
            fora = [(n, d) for n, m, d in p.leituras_contador if sum((d or {}).values())]
            gets = sorted(p.gets)
            rodape = (f"\n\nMarcador → lista, todos: {p.fmt_lat(todas)}. `GET /api/sessions`: mediana "
                      f"{statistics.median(gets) * 1000:.0f} ms, máx {max(gets) * 1000:.0f} ms ({len(gets)}).\n"
                      if gets else "\n")
            rodape += (f"Contador `PYTHON_DISCOVERY` ao fim de cada cenário: "
                       f"{[(n, m, d) for n, m, d in p.leituras_contador]}; "
                       f"list_error no SSE: {p.sse.list_error or 'nenhum'}; eventos: {p.sse.eventos}.\n"
                       f"Latências (sessão, estado, s): {p.lat_detalhe}\n")
            tabela = "\n".join(linhas) + rodape
            print("\n" + tabela)
            if args.relatorio:
                Path(args.relatorio).write_text(tabela)
            if not args.reserva and fora:
                print(f"DESCOBERTA PYTHON NO MODO RUST: {fora}", flush=True)


if __name__ == "__main__":
    main()
