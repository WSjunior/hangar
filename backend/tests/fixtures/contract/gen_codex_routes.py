# backend/tests/fixtures/contract/gen_codex_routes.py
"""Rotas só do Codex (sessão sem terminal): o corpo e o código que a rota Python dá para respostas fixas
do app-server. O teste `contract_codex_routes.rs` sobe o roteador Rust com um cano falso que devolve as
MESMAS respostas e compara caso a caso.

A referência é a rota Python com o adapter do modo `python` (o `CodexAdapter` falando com o
app-server), não o repasse do modo `rust`: é esse o formato que o app conhece.

Uso, de backend/:  uv run python tests/fixtures/contract/gen_codex_routes.py
"""
import asyncio
import copy
import json
import sys
import threading
from pathlib import Path
from types import SimpleNamespace

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[2]))
GOLDEN = HERE / "golden"

THREAD = "thread-1"
ERR = {"error": {"code": -32000, "message": "falhou"}}
MODELS = {"data": [
    {"model": "gpt-5", "displayName": "GPT-5", "description": "rápido", "hidden": False,
     "supportedReasoningEfforts": [{"reasoningEffort": "low", "description": "pouco"},
                                   {"reasoningEffort": "high", "description": "muito"}],
     "defaultReasoningEffort": "high", "serviceTiers": [{"id": "priority", "name": "Fast"}],
     "defaultServiceTier": None},
    # Dentro do esquema do app-server (texto, nunca null): o Rust decodifica pelo tipo dele.
    {"model": "gpt-5-mini", "displayName": "Mini", "description": "", "hidden": False,
     "supportedReasoningEfforts": [], "defaultReasoningEffort": "low"},
    {"model": "escondido", "displayName": "X", "description": "x", "hidden": True,
     "supportedReasoningEfforts": [], "defaultReasoningEffort": "low"},
]}
MODELS_NO_FAST = {"data": [{**MODELS["data"][0], "serviceTiers": []}]}


def thread(model="gpt-5", effort="high"):
    return {"result": {"thread": {"id": THREAD, "status": {"type": "idle"}, "turns": [],
                                  "model": model, "reasoningEffort": effort}}}


SKILLS = {"data": [
    {"cwd": "/tmp/projeto", "skills": [
        {"name": "revisar", "path": "/a/revisar/SKILL.md", "description": "Revisa", "enabled": True},
        {"name": "compact", "path": "/a/compact/SKILL.md", "description": "homônima do builtin", "enabled": True},
        {"name": "dup", "path": "/a/dup/SKILL.md", "description": "um", "enabled": True},
        {"name": "dup", "path": "/b/dup/SKILL.md", "description": None, "enabled": True},
        {"name": "desligada", "path": "/a/off/SKILL.md", "description": "x", "enabled": False},
        {"name": "sem-caminho", "description": "x", "enabled": True},
    ]},
]}
LIMITS = {"rateLimits": {"limitId": "codex", "limitName": None, "planType": "plus",
                         "primary": {"usedPercent": 12, "windowDurationMins": 300, "resetsAt": 1800000000},
                         "secondary": None, "credits": None}}

BASE = {"model": "gpt-5", "effort": "high", "service_tier": "default", "mode": "default",
        "permission_mode": "Full Access", "busy": False, "async": []}


def case(name, method, route, body=None, rpc=None, relay=None, fault=None, **session):
    return {"name": name, "method": method, "route": route, "body": body, "rpc": rpc or {},
            "session": {**BASE, **session}, "relay": relay, "fault": fault}


CASES = [
    case("models_ok", "GET", "models", rpc={"thread/read": thread(), "model/list": {"result": MODELS}},
         service_tier="priority", mode="plan"),
    case("models_settings_refused", "GET", "models", rpc={"thread/read": ERR, "model/list": {"result": MODELS}}),
    case("models_list_refused", "GET", "models", rpc={"thread/read": thread("gpt-5-mini", None), "model/list": ERR}),
    case("model_ok", "POST", "model", {"model": "gpt-5-mini", "effort": "low"}, rpc={"thread/settings/update": {"result": {}}}),
    case("model_without_effort", "POST", "model", {"model": "gpt-5"}, rpc={"thread/settings/update": {"result": {}}}),
    case("model_refused", "POST", "model", {"model": "gpt-5"}, rpc={"thread/settings/update": ERR}),
    case("service_tier_unchanged", "POST", "service-tier", {"service_tier": "default"}),
    case("service_tier_fast_unavailable", "POST", "service-tier", {"service_tier": "priority"},
         rpc={"thread/read": thread(), "model/list": {"result": MODELS_NO_FAST}}),
    case("mode_plan", "POST", "codex/mode", {"mode": "plan"},
         rpc={"thread/read": thread(), "thread/settings/update": {"result": {}}}, service_tier="priority"),
    case("mode_settings_refused", "POST", "codex/mode", {"mode": "plan"},
         rpc={"thread/read": ERR, "thread/settings/update": {"result": {}}}),
    case("mode_update_refused", "POST", "codex/mode", {"mode": "default"},
         rpc={"thread/read": thread(), "thread/settings/update": ERR}),
    case("limits_ok", "GET", "limits", rpc={"account/rateLimits/read": {"result": LIMITS}}),
    case("limits_without_windows", "GET", "limits", rpc={"account/rateLimits/read": {"result": {"rateLimits": {"limitId": "codex"}}}}),
    case("limits_refused", "GET", "limits", rpc={"account/rateLimits/read": ERR}),
    case("skip_async", "POST", "question/skip", {"request_id": "async:thread-1:i1:0"}, **{"async": ["async:thread-1:i1:0"]}),
    case("skip_unknown", "POST", "question/skip", {"request_id": "async:outra"}, **{"async": ["async:thread-1:i1:0"]}),
    case("commands_ok", "GET", "commands", rpc={"skills/list": {"result": SKILLS}}),
    case("commands_refused", "GET", "commands", rpc={"skills/list": ERR}),
    case("permissions_list", "GET", "codex-permissions", permission_mode="Approve for me"),
    case("permissions_same_sandbox", "POST", "codex-permissions", {"mode": "ask FOR approval"}, permission_mode="Ask for approval"),
    case("permissions_unknown", "POST", "codex-permissions", {"mode": "Tudo"}),
    case("permissions_busy", "POST", "codex-permissions", {"mode": "Ask for approval"}, busy=True),
    # Falhas que o cano falso não produz: o Rust as confere pela função de resposta, com o erro do ator em
    # `fault.rust` (código e texto). O texto depois de "não consegui reabrir o Codex:" difere por natureza.
    case("skip_send_failed", "POST", "question/skip", {"request_id": "async:thread-1:i1:0"},
         fault={"python": "skip", "rust": {"code": "runtime_closed", "message": "ator saiu"}}, **{"async": ["async:thread-1:i1:0"]}),
    case("permissions_reopen_failed", "POST", "codex-permissions", {"mode": "Ask for approval"},
         fault={"python": "reopen", "rust": {"code": "codex_headless_nao_subiu", "message": "o Codex não subiu"}}),
    # Fora do Rust: sessão inexistente e Codex com terminal seguem ao Python, que responde como antes.
    case("models_unknown_session", "GET", "models", relay="unknown"),
    case("commands_terminal", "GET", "commands", relay="terminal"),
    case("model_terminal", "POST", "model", {"model": "gpt-5"}, relay="terminal"),
]


class FakeClient:
    """Cliente do app-server com respostas fixas por método (o mesmo que o cano falso do Rust devolve)."""

    def __init__(self, rpc):
        self.rpc, self.server_requests, self.closed = rpc, {}, False

    async def close(self):
        self.closed = True

    async def request(self, method, params):
        answer = self.rpc[method]
        if "error" in answer:
            raise RuntimeError(f"codex app-server error em '{method}': {answer['error']}")
        return copy.deepcopy(answer["result"])


def rows() -> list:
    from fastapi import HTTPException
    from app import api
    from app.adapters.codex import sessions as codex_sessions
    from app.adapters.codex.adapter import CodexAdapter
    from app.adapters.codex.async_questions import AsyncQuestions

    loop = asyncio.new_event_loop()
    threading.Thread(target=loop.run_forever, daemon=True).start()
    api._loop_servidor = loop
    meta = {}
    codex_sessions.load = lambda name: dict(meta) if meta else None
    codex_sessions.update = lambda name, **fields: meta.update(fields) or dict(meta)
    codex_sessions.update_model = lambda name, model, effort: meta.update(model=model, effort=effort)
    codex_sessions.update_service_tier = lambda name, thread_id, tier: meta.update(service_tier=tier) or True

    def setup(c):
        s = c["session"]
        terminal = c["relay"] == "terminal"
        meta.clear()
        if c["relay"] != "unknown":
            meta.update(headless=not terminal, thread_id=THREAD, cwd="/tmp/projeto", permission_mode=s["permission_mode"],
                        model=s["model"], effort=s["effort"], service_tier=s["service_tier"])
        provider = "claude" if c["relay"] == "unknown" else "codex"
        api._provider_of = lambda name: provider
        api._cached_info_sync = lambda name: SimpleNamespace(provider=provider)
        api._recusa_se_painel_aberto = lambda name: None
        adapter = CodexAdapter()
        client = FakeClient(c["rpc"])
        questions = AsyncQuestions(THREAD)
        for rid in s["async"]:
            questions._pending[rid] = {"provider": "codex", "request_id": rid, "is_async": True, "questions": [{"question": "?"}]}
        adapter._sessions["s"] = {"client": client, "thread_id": THREAD, "state": "working" if s["busy"] else "idle",
                                  "in_progress": s["busy"], "turn_state_known": True, "model": s["model"], "effort": s["effort"],
                                  "default_effort": s["effort"], "service_tier": s["service_tier"], "mode": s["mode"],
                                  "async_questions": questions, "ouvintes": []}

        async def ensure_running(name, **kwargs):
            return client
        adapter.ensure_running = ensure_running
        fault = (c["fault"] or {}).get("python")
        if fault == "skip":
            async def skip_question(name, request_id):
                raise RuntimeError("runtime_closed: ator saiu")
            adapter.skip_question = skip_question
        elif fault == "reopen":
            async def reopen(name, meta):
                raise RuntimeError("o Codex não subiu")
            adapter._subir_sem_terminal = reopen
        api.get_adapter = lambda key: adapter

    async def call(c):
        body, route = c["body"], c["route"]
        if route == "models":
            return await api.modelos_da_sessao_codex("s")
        if route == "model":
            return await api.set_codex_model("s", api.CodexModelBody(**body))
        if route == "service-tier":
            return await api.set_codex_service_tier("s", api.CodexServiceTierBody(**body))
        if route == "codex/mode":
            return await api.set_codex_mode("s", api.CodexModeBody(**body))
        if route == "limits":
            return await api.limits("s")
        if route == "question/skip":
            return await asyncio.to_thread(api.skip_question, "s", api.SkipQuestionBody(**body))
        if route == "commands":
            return await api.commands("s")
        if route == "codex-permissions" and c["method"] == "GET":
            return await api.permissoes_do_codex("s")
        return await api.trocar_permissao_do_codex("s", api.CodexPermissionBody(**body))

    async def run(c):
        setup(c)
        try:
            return {"status": 200, "body": await asyncio.wrap_future(asyncio.run_coroutine_threadsafe(call(c), loop))}
        except HTTPException as exc:
            return {"status": exc.status_code, "body": {"detail": exc.detail}}

    async def main():
        out = []
        for c in CASES:
            if c["relay"] == "terminal":
                # Com terminal a rota Python dirige a TUI; o golden só registra que o Rust não a atende.
                out.append({**c, "expect": None})
                continue
            out.append({**c, "expect": await run(c)})
        return out

    return asyncio.run(main())


if __name__ == "__main__":
    GOLDEN.mkdir(parents=True, exist_ok=True)
    (GOLDEN / "codex_routes.json").write_text(json.dumps(rows(), ensure_ascii=True, indent=1) + "\n", encoding="utf-8")
