"""Oráculo sintético: executa os reducers da base, sem lifespan, cano ou CLI reais.

Executar somente quando o dono autorizar testes. O subprocesso importa uma cópia temporária
da base; a implementação em andamento nunca pode reescrever o comportamento esperado.
"""
from __future__ import annotations

import asyncio
import copy
import io
import json
import os
import subprocess
import sys
import tarfile
import tempfile
from contextlib import ExitStack
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

BASE = "93ce2f657f028b0ed5e696e4bd23bff2aa9054c6"
FIXTURES = Path(__file__).parent / "fixtures" / "headless_runtime"


def generate(provider: str, scenarios: list[dict]) -> list[dict]:
    if provider not in {"claude", "codex"}:
        raise ValueError("provedor fora do escopo")
    return asyncio.run(_generate(provider, scenarios))


async def _generate(provider: str, scenarios: list[dict]) -> list[dict]:
    from app.adapters.claude_headless import adapter as claude
    from app.adapters.codex import adapter as codex
    from app.adapters.codex.async_questions import AsyncQuestions
    from app.adapters.preview_push import PushPreviewSource

    result = []
    for scenario in scenarios:
        if scenario["provider"] != provider:
            continue
        outputs = []
        metadata = copy.deepcopy(scenario["metadata"])
        clock = dict(scenario["clock"])
        name = metadata["name"]

        def emit(channel, data):
            if hasattr(data, "model_dump"):
                data = data.model_dump()
            outputs.append({"channel": channel, "data": copy.deepcopy(data)})

        original_push = PushPreviewSource.push

        async def push(source, text):
            if text == source.text:
                return
            await original_push(source, text)
            channel = ("thinking" if source.name.endswith("#pensamento") else
                       "tool" if source.name.endswith("#ferramenta") else "preview")
            emit(channel, {"session": name, "text": text, "md": True, "full": True, "vivo": True})

        async def write(session, frame):
            emit("write", frame)

        async def note(session, text):
            emit("local", {"text": text})

        async def drain(*args):
            emit("wake_queue", {})

        class Queue:
            def __init__(self, queue_name):
                self.name = queue_name

            def append_saida_local(self, text):
                emit("local", {"text": text})
                return {"id": "synthetic-local", "text": text, "delivered": True,
                        "confirmed": True, "papel": "assistant", "ts": clock["epoch_s"]}

            def confirm_delivered(self, predicate=None):
                emit("confirm", {})
                return 0

        def update(session_name, **changes):
            metadata.update(changes)
            emit("patch_meta", changes)
            return copy.deepcopy(metadata)

        class Client:
            closed = False

            def __init__(self):
                self.server_requests = {}

            async def request(self, method, params=None, **kwargs):
                emit("rpc", {"method": method, "params": params or {}})
                return copy.deepcopy(scenario.get("rpc_results", {}).get(method, {}))

            async def respond(self, request_id, value=None, **kwargs):
                emit("response", {"id": request_id, "result": value, **kwargs})
                self.server_requests.pop(request_id, None)

            async def notifications(self):
                for step in scenario["steps"]:
                    clock.update(step.get("clock", {}))
                    if step["kind"] == "line":
                        event = copy.deepcopy(step["payload"])
                        if "id" in event and "method" in event:
                            self.server_requests[event["id"]] = event
                        if event.get("method") == "serverRequest/resolved":
                            self.server_requests.pop(event["params"]["requestId"], None)
                        yield event
                    elif step["kind"] == "command":
                        await command(step)
                    elif step["kind"] == "tick":
                        if buffer := session.get("preview_buffer"):
                            await buffer.flush()
                    else:
                        emit("contract_input", step)

        with ExitStack() as stack:
            stack.enter_context(patch.object(PushPreviewSource, "_sources", {}))
            stack.enter_context(patch.object(PushPreviewSource, "push", push))
            stack.enter_context(patch.object(claude, "PromptQueue", Queue))
            stack.enter_context(patch.object(codex, "PromptQueue", Queue))
            stack.enter_context(patch.object(claude.diag, "registrar", lambda *args, **kwargs: None))
            # Os relógios dos reducers e dos buffers usam o mesmo domínio sintético.
            stack.enter_context(patch.object(asyncio.get_running_loop(), "time", lambda: clock["monotonic_s"]))
            for module in (claude, codex):
                real_time = module.time
                fake_time = SimpleNamespace(monotonic=lambda: clock["monotonic_s"],
                                            time=lambda: clock["epoch_s"],
                                            strftime=real_time.strftime, localtime=real_time.localtime)
                stack.enter_context(patch.object(module, "time", fake_time))
            if provider == "claude":
                adapter = claude.ClaudeHeadlessAdapter()
                session = claude._Sessao(name, metadata)
                session.proc = SimpleNamespace(pid=4242, returncode=None)
                session.janelas_ts = clock["epoch_s"]
                if metadata.get("initialized"):
                    session.initialized.set()
                adapter._sessions[name] = session
                adapter._write, adapter._nota_local = write, note
                async def ensure_running(n, **kwargs):
                    return session
                adapter.ensure_running = ensure_running
                adapter._drenar_fim_de_turno = drain
                adapter._agendar_cota = lambda sess: emit("quota", {})
                adapter._gravar_marcador = lambda sess, state: emit("marker", {"state": state})

                async def unknown(sess, kind, payload):
                    emit("unknown_private", {"kind": kind})

                adapter._gravar_desconhecido = unknown
                stack.enter_context(patch.object(claude.hl_sessions, "update", update))
                stack.enter_context(patch.object(claude, "_esforco_padrao", lambda config_dir: None))

                async def command(step):
                    method = getattr(adapter, step["method"])
                    value = await method(name, **step.get("args", {}))
                    emit("command_result", {"method": step["method"], "result": value})

                for step in scenario["steps"]:
                    clock.update(step.get("clock", {}))
                    if step["kind"] == "line":
                        await adapter._on_event(session, step["payload"])
                    elif step["kind"] == "command":
                        await command(step)
                    elif step["kind"] == "tick":
                        for buffer in (session.preview_buffer, session.thinking_buffer, session.tool_buffer):
                            await buffer.flush()
                    else:
                        emit("contract_input", step)
                    emit("state", adapter._evento(session))
                for buffer in (session.preview_buffer, session.thinking_buffer, session.tool_buffer):
                    await buffer.discard()
                if adapter._tarefas:
                    await asyncio.gather(*tuple(adapter._tarefas))
            else:
                adapter = codex.CodexAdapter()
                client = Client()
                session = {"client": client, "thread_id": metadata["thread_id"],
                           "state": "idle", "in_progress": False, "headless": True,
                           "subscribed": True, "ouvintes": [], "model": metadata.get("model"),
                           "effort": metadata.get("effort"), "async_questions": AsyncQuestions(metadata["thread_id"])}
                adapter._sessions[name] = session
                adapter.drain = drain
                stack.enter_context(patch.object(codex.codex_sessions, "load", lambda n: metadata))
                stack.enter_context(patch.object(codex.codex_sessions, "update", update))

                async def ensure_running(n):
                    return client

                adapter.ensure_running = ensure_running

                async def command(step):
                    value = await getattr(adapter, step["method"])(name, **step.get("args", {}))
                    emit("command_result", {"method": step["method"], "result": value})

                await adapter._consumir(name, client, session, lambda event: emit("state", event))
                emit("state", adapter._question_state(name, session))
        result.append({"name": scenario["name"], "provider": provider, "base": BASE,
                       "outputs": outputs, "conservative_changes": scenario.get("conservative_changes", [])})
    return result


def main():
    scenarios = json.loads((FIXTURES / "scenarios.json").read_text(encoding="utf-8"))
    if "--worker" in sys.argv:
        for provider in ("claude", "codex"):
            data = generate(provider, scenarios)
            (FIXTURES / f"{provider}-golden.json").write_text(
                json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        return
    root = Path(__file__).resolve().parents[2]
    archive = subprocess.run(["git", "archive", BASE, "backend"], cwd=root,
                             capture_output=True, check=True).stdout
    with tempfile.TemporaryDirectory(prefix="hangar-runtime-oracle-") as folder:
        with tarfile.open(fileobj=io.BytesIO(archive)) as bundle:
            bundle.extractall(folder, filter="data")
        env = dict(os.environ, PYTHONPATH=str(Path(folder) / "backend"))
        subprocess.run([sys.executable, str(Path(__file__).resolve()), "--worker"],
                       cwd=folder, env=env, check=True)


if __name__ == "__main__":
    main()
