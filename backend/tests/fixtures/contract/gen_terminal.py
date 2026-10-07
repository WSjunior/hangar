"""Golden do pane e do StateMonitor Python, sem terminal ou sidecar real.

Uso, de backend/: uv run python tests/fixtures/contract/gen_terminal.py
"""
import asyncio
import inspect
import json
import sys
import time
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[2]))
from app import agentpane, permission_mode, plugin_bridge, procinfo, state, terminal_observer, tmux
from app.adapters.claude_headless import sessions as headless_sessions
from app.loop import LoopLink
from app.preview import extract_assistant_text


def analyze(pane):
    status, label, question, options = state.classify(pane)
    menu = state.menu_codex(pane)
    return dict(state=status, label=label, question=question, options=options,
                spinner=state._live_spinner(pane), status_line=state.status_line(pane),
                overlay=state.is_overlay(pane), login=state.is_login(pane),
                limit_reset=state.rate_limit_reset(pane),
                preview=extract_assistant_text(pane, "claude"),
                codex_menu=None if menu is None else dict(question=menu[0], options=menu[1]))


class Finished(Exception):
    pass


async def reference_sequence(frames):
    """Executa o reducer original; observa os locais na próxima aquisição de pane."""
    outputs, current = [], {}
    index = 0
    monitor = state.StateMonitor("fixture", sid_get=lambda: "fixture", poll=0, provider=None)

    async def capture(*args):
        nonlocal index, current
        caller = inspect.currentframe().f_back
        if index:
            values = caller.f_locals
            result = analyze(current["pane"])
            for name in ("state", "label", "question", "options", "overlay", "login", "limit_reset"):
                result[name] = values[name]
            result["status_line"] = values["status"]
            memory = {name: values[name] for name in
                      ("prev_spinner", "frozen", "no_spinner", "held_state", "held_label")}
            outputs.append(dict(analysis=result, memory=memory))
        if index == len(frames):
            raise Finished
        current = frames[index]
        index += 1
        monitor.hook_grace = current.get("facts", {}).get("hook_grace", 8)
        return current["pane"]

    def fact(name):
        return current.get("facts", {}).get(name)

    def open_question(*args):
        q = fact("open_question")
        return None if q is None else SimpleNamespace(questions=[SimpleNamespace(
            question=q.get("question"), options=[SimpleNamespace(label=o) for o in q["options"]])])

    async def no_sleep(*args):
        pass

    with patch.object(state, "shared_capture", capture), \
         patch.object(terminal_observer, "_config", None), \
         patch.object(state, "pergunta_aberta", open_question), \
         patch.object(state.plugin_bridge, "pergunta_pendente", lambda *a: fact("plugin_question")), \
         patch.object(state.plugin_bridge, "estado_recente", lambda *a: None if fact("plugin_state") is None else (fact("plugin_state"), 0)), \
         patch.object(state.plugin_bridge, "vivo", lambda *a: False), \
         patch.object(state.hook_state, "get_state", lambda *a: None if fact("hook_state") is None else (fact("hook_state"), 0)), \
         patch.object(state.hook_state, "shells", lambda *a: []), \
         patch.object(state, "_sidecar_status", lambda *a: fact("status_line")), \
         patch.object(LoopLink, "get", lambda *a: None), \
         patch.object(state.asyncio, "sleep", no_sleep):
        try:
            async for _ in monitor.stream():
                pass
        except Finished:
            pass
    return outputs


def main():
    rows = [dict(name=p.name, pane=p.read_text(encoding="utf-8"))
            for p in sorted(HERE.parent.glob("pane_*.txt"))]
    synthetic = {
        "empty": "",
        "prose_utf8": "❯ teste\n● Olá, ação 🐍\n  continuação 漢字\n────────────\n❯\n────────────\nmodelo truncado",
        "tools_keep_prose": "● Resposta\n  texto\n● Bash(ls)\n⎿ output\n● Reading 4 files…\n✻ Thinking…",
        "plugin_reset": "❯ teste\n● Resposta anterior\n● ecc: hooks.json: unknown keys\n────────────",
        "banner": "Claude Code v2\n● aviso\n────────────",
        "subagent": "● Resposta\n  continuação\n● Subagent reviewer\n\n └ conferindo\n────────────",
        "agent_finished": '● Resposta\n● Agent "reviewer" finished · 9s\nSearched, ran 2 shell commands',
        "mcp": "● Resposta\nCalling chrome-devtools…\nfora",
        "mcp_prose": "● Calling this an edge case, veja.\n  continuação",
        "activity": "● Resposta\n  resumo ran 2 shell commands\nfora",
        "ascii_spinner": "● Resposta\n* Thinking…\nfora",
        "todo": "● Resposta\n○ Todos (1/2)\nfora",
        "draft": "✻ Worked for 1s\n────────────\n❯ 1. pode editar 2. mostre a data\n────────────",
        "preview_numbered": "☐ Escolha\nQual?\n❯ 1. ação 🐍          ┌───────────\n  2. outra             │ 1. exemplo 2. segundo\nEnter to select",
        "login": "Choose the text style\n\n\n",
        "quoted_login": "● /oauth/authorize\n────────────\n❯\n────────────\n⏵⏵ bypass",
        "overlay_padded": "painel\nEsc to cancel\n" + "\n" * 20,
        "overlay_old": "Esc to cancel\n" + "texto\n" * 12,
        "limit_old": "Usage limit reached resets 9:10pm\n" + "texto\n" * 12,
        "codex_wrap": "Aprovar?\n› 1. Opção ação\n     continua 🐍\n  2. Outra\nPress enter to confirm\n\n",
        "codex_short_footer": "  Update available · 0.159.3 → 0.160.0\n› 1. Update now\n  2. Skip\n\n  enter continue · esc skip\n\n\n",
        "codex_invalid": "Aprovar?\n› 1. Um\n  3. Três\nPress enter to confirm",
        "subagent_prose": "● Subagent reviewer pode ajudar\n  explique",
        "prose_box": "● Texto\n╭────────╮\n│ exemplo │\n╰────────╯",
        "unicode_columns": "Título\n  ❯ Sim 🐍\n    Não 漢字\nEsc to cancel",
        "unicode_numbering": "Aprovar?\n› ١. Um\n  ٢. Dois\nPress enter to confirm",
        "unicode_word_boundary": "● Running\u0301 palavras\n  continuação",
        "unicode_plugin_word": "● ecc\u0301: hooks.json: unknown keys\n  continuação",
        "combining_running": "● Running\u0345 palavras\n  continuação",
        "combining_calling": "● Calling\u05b0 serviço…\n  continuação",
        "combining_finished": '● Resposta\n● Agent "worker" finished\u0345\n  atividade',
        "combining_summary": "● Resposta\n  \u05b0ran 2 shell commands\n  fora",
        "login_dotless": "Select logın method",
        "login_dotted": "Select logİn method",
        "limit_dotless": "Usage lımıt reached · continuing automatically at 9:10pm",
        "limit_dotted": "Usage lİmİt reached · contİnuİng automatİcally at 9:10pm",
        "login_long_s": "ſelect login method",
        "login_ordinary": "Select LOGIN method",
        "information_whitespace": "● texto\n\x1f\x1f\n────────────\n\x1f",
        "line_separators": "● texto\rcontinuação\vlinha\fquadro\x1cregistro\x1dgrupo\x1earquivo\x85próxima\u2028separador\u2029parágrafo",
    }
    rows += [dict(name=name, pane=pane) for name, pane in synthetic.items()]
    for row in rows:
        row["expected"] = analyze(row["pane"])
    frame = lambda pane, **facts: dict(pane=pane, facts=facts)
    spinner = "✻ Thinking…\n────────────\n❯\n────────────"
    plain = "● Resposta\n────────────\n❯\n────────────"
    menu = synthetic["preview_numbered"]
    sequences = {
        "frozen_four": [frame(spinner)] * 4,
        "missing_four": [frame(spinner)] + [frame(plain)] * 4,
        "hook_grace_eight": [frame(plain, hook_state="working")] * 10,
        "hook_no_grace": [frame(plain, hook_state="working", hook_grace=None)] * 10,
        "menu_immediate": [frame(spinner), frame(menu, plugin_state="working", hook_state="working")],
        "plugin_idle_animation": [frame(spinner), frame(spinner.replace("✻", "✽"), plugin_state="idle")],
        "hook_idle_animation": [frame(spinner), frame(spinner.replace("✻", "✽"), hook_state="idle"), frame(spinner.replace("✻", "✽"), hook_state="idle")],
        "plugin_then_hook": [frame(plain, plugin_state="idle", hook_state="working")] * 10,
        "plugin_working_resets": [frame(plain, plugin_state="working")] * 10,
        "statusline_truthy": [frame(spinner, status_line="modelo inteiro 🐍"), frame(spinner, status_line="")],
        "offscreen_question": [frame(spinner, open_question=dict(question="Qual?", options=["Um", "Dois"]))],
        "pane_question_wins": [frame(menu, open_question=dict(question="Outro?", options=["X", "Y"]), plugin_question=dict(id="perm:a", tool="Bash", resumo="ls"))],
        "plugin_permission": [frame(spinner, plugin_question=dict(id="perm:a", tool="Bash", resumo="ls"))],
        "plugin_question": [frame(plain, plugin_question=dict(id="q", questions=[dict(question="Qual?", options=[dict(label="Um"), dict(label="Dois")])]))],
        "plugin_empty_question": [frame(plain, plugin_question=dict(id="q", questions=[]))],
        "non_animated_change": [frame(spinner), frame("✽ Worked for 3s", hook_state="idle")],
    }
    # A mesma coleção conserva o formato simples das fixtures estáticas.
    rows += [dict(name=name, sequence=frames, expected_sequence=asyncio.run(reference_sequence(frames)))
             for name, frames in sequences.items()]
    write("terminal.json", rows)
    write("terminal_monitor.json", [dict(name=name, frames=frames, rounds=asyncio.run(monitor_sequence(frames)))
                                    for name, frames in monitor_sequences().items()])
    write("permission_mode.json", permission_rows())
    write("shells.json", shells_rows())
    write("agent_pane.json", agent_pane_rows())


NAME, SID = "fixture", "fixture-sid"


def facts(**kw):
    """Retrato do `state_facts` (Task 1), com as idades em ms que o Rust aplica no relógio dele."""
    out = dict(seq=1, plugin_state=None, waiter_open=False, heartbeat_age_ms=None, question=None,
               suggestion="", body_columns=None, band_anchor=None, in_transfer_ms=0,
               transfer_active=False, permission_op=False)
    out.update(kw)
    return out


def _load_plugin(f):
    """Põe o retrato na memória do plugin_bridge: as validades de lá decidem, com o relógio de agora."""
    agora = time.monotonic()
    for d in (plugin_bridge._estados, plugin_bridge._batidas, plugin_bridge._waiters, plugin_bridge._perguntas):
        d.pop(NAME, None)
    permission_mode._operacoes_controladas.pop(NAME, None)
    if f is None:
        return
    if f["plugin_state"] is not None:
        p = f["plugin_state"]
        plugin_bridge._estados[NAME] = (agora - p["age_ms"] / 1000, p["state"], p.get("reason"))
    if f["heartbeat_age_ms"] is not None:
        plugin_bridge._batidas[NAME] = agora - f["heartbeat_age_ms"] / 1000
    if f["waiter_open"]:
        plugin_bridge._waiters[NAME] = None
    if f["question"] is not None:
        q = f["question"]
        plugin_bridge._perguntas[NAME] = dict(id=q["id"], questions=q["questions"], tool=q.get("tool"),
                                              resumo=q.get("resumo"), visto=agora - q["seen_age_ms"] / 1000)
    if f["permission_op"]:
        permission_mode._operacoes_controladas[NAME] = 1


def _in_transfer(f):
    return f is not None and (f["transfer_active"] or f["in_transfer_ms"] > 0)


async def monitor_sequence(frames):
    """Roda o `_stream` de verdade, uma rodada por captura; grava o evento de cada rodada e a
    resposta de `observar_ou_confirmado` (o serviço `permission.observe` que o Rust pede)."""
    rounds, index, current = [], 0, {}
    original_observe = permission_mode.observar_ou_confirmado
    for d in (permission_mode._ultimos_nao_plan, permission_mode._modos_confirmados,
              permission_mode._operacoes_controladas):
        d.clear()

    async def capture(*args):
        nonlocal index, current
        if index == len(frames):
            raise Finished
        current = frames[index]
        index += 1
        rounds.append(dict(event=None, observe=None))
        _load_plugin(current.get("facts"))
        if current.get("fail"):
            raise terminal_observer.ObservationFailed(current["fail"])
        return current["pane"]

    def observe(key, mode, sessao=None):
        result = original_observe(key, mode, sessao=sessao)
        assert rounds[-1]["observe"] is None, "duas perguntas de permissão na mesma rodada"
        rounds[-1]["observe"] = list(result)
        return result

    def open_question(*args):
        q = current.get("open_question")
        return None if q is None else SimpleNamespace(questions=[SimpleNamespace(
            question=q.get("question"), options=[SimpleNamespace(label=o) for o in q["options"]])])

    async def no_sleep(*args):
        pass

    monitor = state.StateMonitor(NAME, sid_get=lambda: SID, observe_permission=True, provider="claude")
    with patch.object(state, "shared_capture", capture), \
         patch.object(terminal_observer, "_config", None), \
         patch.object(state, "forget_frame", lambda *a: None), \
         patch.object(plugin_bridge, "esquecer", lambda *a: None), \
         patch.object(plugin_bridge, "esperar_evento", no_sleep), \
         patch.object(tmux, "sessao_existe", lambda *a: current.get("exists", True)), \
         patch.object(headless_sessions, "em_troca", lambda *a: _in_transfer(current.get("facts"))), \
         patch.object(permission_mode, "observar_ou_confirmado", observe), \
         patch.object(state, "pergunta_aberta", open_question), \
         patch.object(state.hook_state, "get_state",
                      lambda *a: None if current.get("marker") is None else (current["marker"], 0)), \
         patch.object(state.hook_state, "shells", lambda *a: current.get("shells", [])), \
         patch.object(state, "_sidecar_status", lambda *a: current.get("status_line")), \
         patch.object(LoopLink, "get", lambda *a: current.get("loop")), \
         patch.object(state.asyncio, "sleep", no_sleep):
        try:
            async for event in monitor._stream():
                assert rounds[-1]["event"] is None, "duas emissões na mesma rodada"
                rounds[-1]["event"] = event.model_dump(mode="json")
        except Finished:
            pass
        finally:
            _load_plugin(None)
    return rounds


def monitor_sequences():
    footer = "\n⏵⏵ bypass permissions on (shift+tab to cycle)"
    spinner = "✻ Thinking…\n────────────\n❯\n────────────" + footer
    plain = "● Resposta\n────────────\n❯\n────────────" + footer
    plan = plain.replace("⏵⏵ bypass permissions on", "⏸ plan mode on")
    accept = plain.replace("bypass permissions on", "accept edits on")
    working = dict(plugin_state=dict(state="working", reason=None, age_ms=1_000))
    idle = dict(plugin_state=dict(state="idle", reason=None, age_ms=1_000))
    f = lambda pane, **kw: dict(pane=pane, **kw)
    loop = dict(status="running", iter=2, max_iters=5)
    shell = [dict(pid=4242, cmd="sleep 600", desde=1000.0)]
    eof = "terminal observer EOF"
    question = lambda age: dict(id="perm:1", questions=[], tool="Bash", resumo="ls", seen_age_ms=age)
    ask = dict(id="q", questions=[dict(question="Qual?", options=[dict(label="A"), dict(label="B")])], seen_age_ms=0)
    manual = "\n  ⏸ manual mode on · ← for agents"
    manual_idle = "────────────\n❯\n────────────" + manual
    held_pane = lambda glyph, t: (f"{glyph} Photosynthesizing… (running PreToolUse hooks… 6/10 · {t}s · thinking)\n"
                                  "  ⎿  Tip: Hit shift+tab to cycle between manual mode, auto-accept edit mode, and plan mode\n"
                                  + manual_idle)
    perm = lambda age: dict(id="perm:t1", questions=[], tool="Bash", resumo="ls", seen_age_ms=age)
    stale = lambda age, **kw: facts(plugin_state=dict(state="working", reason=None, age_ms=age), **kw)
    return {
        # Spinner congelado: STALE_LIMIT rodadas iguais viram idle. O Python conta por rodada; `wake`
        # só é lido pelo replay do Rust, que acorda a espera por empurrão. Cada quadro traz todos os
        # fatos da rodada: ausente é vazio, não "o de antes".
        "frozen_spinner_with_wakes": [f(spinner, facts=facts(waiter_open=True), wake=True)] * 5,
        # Sem spinner: IDLE_DEBOUNCE rodadas segurando o working.
        "debounce": [f(spinner)] + [f(plain)] * 5,
        # Marcador working com graça de 8 rodadas sem spinner.
        "marker_grace": [f(plain, marker="working")] * 11,
        "marker_idle_wins_static_spinner": [f(spinner), f(spinner, marker="idle")],
        # Âncora do plugin; a mesma âncora vencida pelos 90 s sem long-poll, e sem prazo com ele.
        "plugin_anchor": [f(plain, facts=facts(**working))] * 3 + [f(spinner, facts=facts(**idle))] * 2,
        "plugin_state_expired": [f(plain, facts=stale(91_000))] * 2 + [f(plain, facts=stale(89_000))],
        "plugin_state_open_waiter": [f(plain, facts=stale(500_000, waiter_open=True))] * 2,
        # Pergunta segurada vence 35 s depois do último `visto`.
        "plugin_question_seen_age": [f(plain, facts=facts(question=question(34_000))),
                                     f(plain, facts=facts(question=question(36_000))),
                                     f(plain, facts=facts(question=ask))],
        "offscreen_question": [f(plain, open_question=dict(question="Qual?", options=["Um", "Dois"]))],
        # Repetição: a mesma chave não emite; statusline, loop e pids dos shells entram nela.
        "repeat_key": [f(plain)] * 6 + [f(plain, status_line="modelo 🐍")] * 2
                      + [f(plain, status_line="modelo 🐍", loop=loop)] * 2
                      + [f(plain, status_line="modelo 🐍", loop=loop, shells=shell)] * 2
                      + [f(plain, status_line="modelo 🐍", loop=loop, shells=[dict(shell[0], desde=2000.0)])],
        "overlay_login_limit": [f("painel\nEsc to cancel\n"), f("Select login method"),
                                f("Usage limit reached resets 9:10pm\n────────────\n❯\n────────────")],
        # Permissão: leitura livre, troca, e a operação controlada que segura o retrato e confere no fim.
        "permission": [f(plain)] * 2 + [f(plan)] + [f(accept, facts=facts(permission_op=True))] * 2
                      + [f(accept)] * 2 + [f(plain)],
        # Falha da observação: repete o último evento com o problema, uma vez por código.
        "observation_failure": [f(plain)] + [f(None, fail="terminal command timeout", attempt=0)] * 3
                               + [f(None, fail=eof, attempt=1)] * 2 + [f(plain)],
        "failure_first_round": [f(None, fail="terminal startup timeout", attempt=0, facts=facts(**working))] * 2,
        "failure_first_round_marker": [f(None, fail="terminal startup timeout", attempt=0, marker="idle")],
        # Morta: troca de conta em curso espera; depois da troca, `dead` e fim.
        "dead_after_transfer": [f(plain),
                                f(None, fail=eof, attempt=0, exists=False, facts=facts(in_transfer_ms=4_000)),
                                f(None, fail=eof, attempt=1, exists=False, facts=facts(transfer_active=True)),
                                f(None, fail=eof, attempt=2, exists=False), f(plain)],
        "empty_pane_alive": [f(""), f("", exists=None), f(plain)],
        "empty_pane_dead": [f(plain), f("", exists=False)],
        # Caso real (Haiku, modo manual, app aberto): o hook do plugin segura a permissão do Bash
        # para o app, a TUI fica em "running PreToolUse hooks" sem cartão e o registro nativo
        # segue `busy` (marcador `working`). Esc: o registro volta a `idle` e a pergunta segurada
        # ainda vale até vencer.
        "permission_card_after_bash": [f(held_pane("✽", 1), marker="working", facts=facts(waiter_open=True))]
        + [f(held_pane(g, t), marker="working", facts=facts(waiter_open=True, question=perm(age)))
           for g, t, age in (("·", 2, 0), ("✢", 3, 1_000), ("✳", 4, 2_000), ("✶", 5, 3_000))]
        + [f(manual_idle, marker="idle", facts=facts(waiter_open=True, question=perm(5_000))),
           f(manual_idle, marker="idle", facts=facts(waiter_open=True))],
    }


def permission_rows():
    panes = {
        "glyph_bypass": "texto\n⏵⏵ bypass permissions on (shift+tab to cycle)",
        "glyph_plan": "⏸ plan mode on (shift+tab to cycle)",
        "last_glyph_wins": "⏸ plan mode on\n⏵⏵ accept edits on",
        "conversation_without_glyph_loses": "plan mode on\n⏵⏵ auto mode on",
        "no_glyph_fallback": "x\nmanual mode on",
        "upper_and_curly": "⏵⏵ DON’T ASK ON",
        "kelvin": "⏵⏵ don't asK on",
        "long_s": "⏵⏵ bypaſs permiſsions on",
        "unknown": "⏵⏵ turbo mode on",
        "empty": "",
        "separators": "⏵⏵ plan mode on ⏵⏵ auto mode on\x1c",
    }
    return [dict(name=n, pane=p, expected=permission_mode.parse_permission_mode(p)) for n, p in panes.items()]


def shells_rows():
    tree = {100: [101, 102, 103, 104, 105]}
    argv = {101: ["/bin/bash", "-c", "source /tmp/snap && eval 'sleep 600' < /dev/null && pwd -P >| '/tmp/x'"],
            102: ["node", "mcp-server.js"],
            103: ["zsh", "-c", "x && eval 'echo '\\''oi'\\'' fim'"],
            104: ["/usr/bin/fish", "-c", "y && eval 'a'\"'\"'b' resto"],
            105: ["bash"]}
    start = {101: 1000.5, 103: 2000.0, 104: None, 105: 5.0}
    out = []
    for name, pid in (("children", 100), ("no_children", 7)):
        with patch.object(procinfo, "_proc_children_map", lambda *a: tree), \
             patch.object(procinfo, "_argv", lambda p: argv.get(p, [])), \
             patch.object(procinfo, "_cmdline", lambda p: " ".join(argv.get(p, [])) + " "), \
             patch.object(procinfo, "_proc_start_time", lambda p: start.get(p)):
            expected = procinfo.shells_de(pid)
        out.append(dict(name=name, pid=pid, children={str(k): v for k, v in tree.items()},
                        argv={str(k): v for k, v in argv.items()},
                        start={str(k): v for k, v in start.items()}, expected=expected))
    return out


def agent_pane_rows():
    pane = lambda pid, pane_id, active=False: dict(pid=pid, pane_id=pane_id, active=active, window_index=0, pane_index=0)
    cases = {
        "single_pane": ([pane(10, "%1", True)], {}, {}),
        "agent_in_second": ([pane(10, "%1", True), pane(20, "%2")], {20: [21]},
                            {10: "-fish ", 20: "-bash ", 21: "claude --session-id x "}),
        "active_first_tiebreak": ([pane(10, "%1"), pane(20, "%2", True)], {10: [11], 20: [22]},
                                  {11: "/usr/bin/claude ", 22: "codex --remote ", 10: "bash", 20: "bash"}),
        "daemon_skipped": ([pane(10, "%1", True), pane(20, "%2")], {10: [11], 20: [21]},
                           {11: "claude daemon ", 21: "pi --mode x ", 10: "sh", 20: "sh"}),
        "no_agent": ([pane(10, "%1", True), pane(20, "%2")], {}, {10: "bash", 20: "vim"}),
        "no_panes": ([], {}, {}),
    }
    out = []
    for name, (panes, children, cmd) in cases.items():
        agentpane.invalidate()
        with patch.object(tmux, "list_panes_of", lambda *a: panes), \
             patch.object(agentpane, "_proc_children_map", lambda *a: children), \
             patch.object(agentpane, "_cmdline", lambda p: cmd.get(p, "")):
            expected = agentpane.resolve_target(NAME)
        out.append(dict(name=name, panes=panes, children={str(k): v for k, v in children.items()},
                        cmdline={str(k): v for k, v in cmd.items()}, expected=expected))
    agentpane.invalidate()
    return out


def write(name, rows):
    target = HERE / "golden" / name
    target.write_text(json.dumps(rows, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"{len(rows)} casos gravados em {target}")


if __name__ == "__main__":
    main()
