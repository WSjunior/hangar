"""Fachada dos adapters e leitura do estado da mesma chave e geração."""
from __future__ import annotations

import asyncio
import copy
import functools
import inspect
import uuid
import contextvars
import json
import time
from dataclasses import dataclass

from app import runtime_coordinator
from app.models import StateEvent

_legacy_operation = contextvars.ContextVar("runtime_legacy_operation", default=None)


def assert_legacy(name, *, reading=False):
    coordinator = runtime_coordinator.current()
    if coordinator is None or not coordinator.managed_queue(name):
        return
    slot = coordinator.slot(name)
    with slot.guard:
        restoring = coordinator.in_lifecycle(slot) and slot.phase == runtime_coordinator.Phase.RecoveringPython
        finishing = reading and slot.phase == runtime_coordinator.Phase.PreparingRust
        context = _legacy_operation.get()
        continuing = (context is not None and context.get("key") == slot.binding.key
            and context.get("generation") == slot.binding.generation and context.get("operation_id") in getattr(coordinator, "legacy_active", set()))
        normal = slot.phase == runtime_coordinator.Phase.Python and (not slot.frozen or coordinator.in_lifecycle(slot))
        if (slot.lease is None or slot.lease.closed or not (normal
                or restoring or finishing or continuing and slot.phase == runtime_coordinator.Phase.PreparingRust)):
            raise RuntimeError("Python não possui a sessão; cliente Legacy bloqueado")


@dataclass
class WireTicket:
    name: str
    key: str
    generation: int
    operation_id: str
    phase_id: str
    frame: dict
    aggregate: bool = False
    incomplete: bool = False


class LegacyIO:
    def __init__(self, coordinator):
        self.coordinator = coordinator

    async def _exec(self, name, action, call_id=None, *, reading=False):
        coordinator = self.coordinator
        slot = coordinator.slot(name)
        assert_legacy(name, reading=reading)
        generation = slot.binding.generation
        with slot.guard:
            slot.active += 1
        def execute():
            with slot.guard:
                if slot.lease is None or slot.lease.closed or slot.binding.generation != generation:
                    raise RuntimeError("reserva perdeu a posse antes da gravação")
                return slot.store.exec(generation, call_id or uuid.uuid4().hex,
                    {"monotonic_s":time.monotonic(), "epoch_s":time.time()}, action)
        task = asyncio.create_task(asyncio.to_thread(execute))
        def finished(done):
            with slot.guard:
                slot.active -= 1
            coordinator._signal(slot)
            if not done.cancelled():
                done.exception()
        task.add_done_callback(finished)
        return await asyncio.shield(task)

    async def prepare_wire(self, name, phase, payload, operation_id=None):
        assert_legacy(name)
        slot = self.coordinator.slot(name)
        binding = slot.binding.descriptor()
        context = _legacy_operation.get() or {}
        if context.get("operation_id") not in self.coordinator.legacy_active:
            context = {}
        operation_id = operation_id or context.get("operation_id") or uuid.uuid4().hex
        phase_id = "reserve-wire:" + binding["key"] + ":" + str(binding["generation"]) + ":" + uuid.uuid4().hex
        with slot.guard:
            saved = copy.deepcopy(slot.store.state["operations"].get(operation_id))
        if saved is None:
            intent = context.get("command") or {"operation_id":operation_id, "kind":"legacy", "payload":{"phase":phase}}
            saved = await self._exec(name, {"kind":"prepare", "id":operation_id, "payload":intent, "entry_id":context.get("entry_id")})
        await self._exec(name, {"kind":"prepare", "id":phase_id, "payload":{"logical_id":operation_id,
            "generation":binding["generation"], "frame":payload,
            "request_id":payload.get("id", payload.get("request_id"))}, "entry_id":saved.get("entry_id")})
        from app.runtime_receipt import ReceiptIndex
        conversation = binding["meta"].get("session_id" if binding["provider"] == "claude" else "thread_id") or ""
        is_input = payload.get("type") == "user" or payload.get("method") in {"turn/start", "turn/steer"}
        cursor = (await asyncio.to_thread(ReceiptIndex(binding["provider"], conversation).capture, binding["jsonl"])
            if is_input and binding["jsonl"] else None)
        await self._exec(name, {"kind":"bind_dispatch", "id":phase_id, "cursor":cursor})
        if saved["status"] == "prepared":
            await self._exec(name, {"kind":"bind_dispatch", "id":operation_id, "cursor":cursor})
        await self._exec(name, {"kind":"begin_dispatch", "id":phase_id, "wire_id":phase_id})
        await self._exec(name, {"kind":"begin_dispatch", "id":operation_id, "wire_id":phase_id})
        return WireTicket(name, binding["key"], binding["generation"], operation_id, phase_id, copy.deepcopy(payload), bool(context))

    async def finish_wire(self, ticket, outcome, result=None, *, definitive=False):
        slot = self.coordinator.slot(ticket.name)
        if slot.binding.key != ticket.key or slot.binding.generation != ticket.generation:
            raise RuntimeError("recibo de outra geração")
        status = {"written":"accepted", "not_written":"rejected", "unknown":"unknown"}[outcome]
        record = result if definitive else {"write_outcome":outcome}
        await self._exec(ticket.name, {"kind":"finish", "id":ticket.phase_id, "status":status, "result":record}, reading=definitive)
        is_request = ticket.frame.get("type") == "control_request" or ticket.frame.get("method") is not None and ticket.frame.get("id") is not None
        if ticket.aggregate and ticket.operation_id in getattr(self.coordinator, "legacy_active", set()):
            return
        if ticket.incomplete and outcome == "written":
            await self._exec(ticket.name, {"kind":"finish", "id":ticket.operation_id, "status":"unknown",
                "result":{"operation_id":ticket.operation_id, "disposition":"unknown", "payload":{"remaining_phase":"effort"}}}, reading=definitive)
            return
        parent_status = status if definitive or not is_request or outcome != "written" else "unknown"
        await self._exec(ticket.name, {"kind":"finish", "id":ticket.operation_id, "status":parent_status,
            "result":{"operation_id":ticket.operation_id, "disposition":parent_status, "payload":record}}, reading=definitive)

    async def reply(self, name, endpoint, frame):
        request_id = frame.get("id") if frame.get("type") != "control_response" else (frame.get("response") or {}).get("request_id")
        if type(request_id) not in (int, str):
            raise ValueError("ID da resposta inválido")
        ticket = getattr(endpoint, "runtime_tickets", {}).get((type(request_id), request_id))
        if ticket is None:
            slot = self.coordinator.slot(name)
            with slot.guard:
                phases = tuple(copy.deepcopy(slot.store.state["operations"]).values())
            candidates = [phase for phase in phases if phase["payload"].get("generation") == slot.binding.generation
                and type(phase["payload"].get("request_id")) is type(request_id) and phase["payload"].get("request_id") == request_id
                and phase["payload"].get("frame")]
            if len(candidates) != 1:
                return
            phase = candidates[0]
            ticket = WireTicket(name, slot.binding.key, slot.binding.generation, phase["payload"]["logical_id"], phase["id"], phase["payload"]["frame"])
        with self.coordinator.slot(name).guard:
            parent = copy.deepcopy(self.coordinator.slot(name).store.state["operations"].get(ticket.operation_id))
        incomplete_effort = bool(parent and parent["payload"].get("kind") == "set_model"
            and isinstance(parent["payload"].get("payload", {}).get("effort"), str)
            and ticket.frame.get("request", {}).get("subtype") == "set_model")
        if incomplete_effort and ticket.operation_id not in getattr(self.coordinator, "legacy_active", set()):
            ticket.incomplete = True
        failure = frame.get("error") is not None or (frame.get("response") or {}).get("subtype") == "error"
        result = {"operation_id":ticket.operation_id, "disposition":"rejected" if failure else "accepted", "payload":frame}
        await self.finish_wire(ticket, "not_written" if failure else "written", result, definitive=True)
        future = getattr(endpoint, "runtime_acks", {}).get(ticket.phase_id)
        if future is not None and not future.done():
            future.set_result("written")

    async def finish_call(self, name, context, *, failed=False, deferred=False):
        slot = self.coordinator.slot(name)
        with slot.guard:
            parent = slot.store.state["operations"].get(context["operation_id"])
            phases = [phase for phase in slot.store.state["operations"].values()
                if phase["payload"].get("logical_id") == context["operation_id"]]
        if parent is None:
            return
        uncertain = any(phase["status"] in {"unknown", "dispatching"}
            or phase["result"] == {"write_outcome":"written"} and (phase["payload"].get("frame", {}).get("method") is not None
                or phase["payload"].get("frame", {}).get("type") == "control_request") for phase in phases)
        status = "unknown" if uncertain else "rejected" if failed else "deferred" if deferred else "accepted"
        await self._exec(name, {"kind":"finish", "id":context["operation_id"], "status":status,
            "result":{"operation_id":context["operation_id"], "disposition":status, "payload":{}}})
        if uncertain:
            raise RuntimeError("resultado incerto; diário e entrada foram conservados")

    async def write(self, name, endpoint, writer, frame, version):
        assert_legacy(name)
        ticket = await self.prepare_wire(name, "write", frame)
        future = asyncio.get_running_loop().create_future()
        endpoint.runtime_acks[ticket.phase_id] = future
        endpoint.runtime_tickets = getattr(endpoint, "runtime_tickets", {})
        request_id = frame.get("id", frame.get("request_id"))
        if request_id is not None:
            endpoint.runtime_tickets[(type(request_id), request_id)] = ticket
        try:
            assert_legacy(name)
            envelope = {"type":"cano_input", "operation_id":ticket.phase_id, "frame":json.dumps(frame)} if version == 2 else frame
            writer.write((json.dumps(envelope) + "\n").encode())
            await writer.drain()
            outcome = await asyncio.wait_for(asyncio.shield(future), 30) if version == 2 else "unknown"
        except asyncio.CancelledError:
            await self.finish_wire(ticket, "unknown")
            raise
        except Exception:
            with self.coordinator.slot(name).guard:
                phase = copy.deepcopy(self.coordinator.slot(name).store.state["operations"].get(ticket.phase_id))
            await self.finish_wire(ticket, "unknown")
            if not (phase and isinstance(phase.get("result"), dict) and phase["result"].get("disposition") in {"accepted", "rejected"}):
                raise RuntimeError("escrita incerta; operação conservada sem reenvio") from None
            outcome = "written"
        finally:
            endpoint.runtime_acks.pop(ticket.phase_id, None)
        await self.finish_wire(ticket, outcome)
        if outcome != "written":
            raise RuntimeError("entrada não confirmada pelo cano; diário conservado")
        return ticket


class LegacyBridge:
    def __init__(self, coordinator, adapters):
        self.coordinator, self.adapters = coordinator, adapters

    def binding(self, name, provider):
        from app.pqueue import _queue_dir
        if provider == "claude":
            from app.adapters.claude_headless import sessions
        elif provider == "codex":
            from app.adapters.codex import sessions
        else:
            return None
        meta = sessions.load(name)
        if not meta or not meta.get("key"):
            if self.coordinator.managed_queue(name) and not self.coordinator.slot(name).binding.headless:
                return copy.deepcopy(self.coordinator.slot(name).binding)
            return None
        directory = _queue_dir()
        state_path = directory / "runtime" / (meta["key"] + ".json")
        headless = bool(meta.get("headless"))
        if not headless and not state_path.exists():
            return None
        path = self.adapters[provider].transcript_path_de(meta) if provider == "claude" else meta.get("rollout_path") or ""
        if provider == "codex" and not path and meta.get("thread_id"):
            from app.adapters.codex.sem_terminal import rollout_de
            path = rollout_de(meta["thread_id"], meta.get("codex_home"))
            current = sessions.load(name)
            if path and current and current.get("key") == meta["key"] and current.get("thread_id") == meta["thread_id"]:
                meta = sessions.update(name, rollout_path=path) or meta
        generation = (self.coordinator.slots[meta["key"]].binding.generation
            if meta["key"] in self.coordinator.slots else
            json.loads(state_path.read_bytes())["generation"] if state_path.exists() else 1)
        return runtime_coordinator.Binding(name, meta["key"], provider, headless, meta, path,
            directory, state_path, directory / "runtime" / (meta["key"] + ".lock"), generation)

    async def quiesce(self, descriptor):
        name, provider = descriptor["name"], descriptor["provider"]
        adapter = self.adapters[provider]
        slot = self.coordinator.slots[descriptor["key"]]
        with slot.guard:
            carry = copy.deepcopy(slot.store.state.get("runtime_state", {}).get("view") or {})
        sess = adapter._sessions.get(name)
        if provider == "codex" and sess is not None and sess["client"].tem_processo_proprio:
            raise RuntimeError("reserva headless encontrou processo próprio; transferência recusada")
        adapter._sessions.pop(name, None)
        tasks = []
        if provider == "claude" and sess is not None:
            carry.update(initialized=sess.initialized.is_set(), model=sess.model, effort=sess.effort,
                permission_mode=sess.permission_mode, previous_non_plan=sess.modo_nao_plan,
                commands=sess.comandos, terminal_commands=list(sess.comandos_terminal), usage=sess.usage, context_window=sess.context_window, cost=sess.cost)
            sess.desligando = True
            sess.live_active = lambda: False
            if sess.drenador is not None:
                tasks.append(sess.drenador)
            for task in tuple(adapter._tarefas):
                frame = getattr(task.get_coro(), "cr_frame", None)
                if frame is not None and (frame.f_locals.get("sess") is sess or frame.f_locals.get("name") == name):
                    tasks.append(task)
            if sess.proc is not None:
                sess.proc.saiu(None)
                sess.proc.stdin.close()
                await sess.proc.stdin.wait_closed()
            if sess.leitor is not None:
                tasks.append(sess.leitor)
        elif provider == "codex" and sess is not None:
            carry.update(initialized=True, ready=True, thread_id=sess["thread_id"],
                model=sess.get("model") or sess.get("default_model"), effort=sess.get("effort") or sess.get("default_effort"),
                in_progress=sess.get("in_progress", False), turn_id=sess.get("turn_id"))
            questions = sess.get("async_questions")
            if questions is not None:
                carry.update(async_questions=list(copy.deepcopy(questions._pending).items()),
                    async_seen=sorted(questions._seen), async_resolved=sorted(questions._resolved),
                    skipped_async_questions=sorted(questions.skipped), async_local_answers=copy.deepcopy(questions._local_answers),
                    async_echoes=dict(questions._echoes), async_during_load=copy.deepcopy(questions._during_load))
            for collection in (adapter._subscribers, adapter._tmux_watchers):
                if task := collection.pop(name, None):
                    tasks.append(task)
            if task := sess.get("bomba"):
                tasks.append(task)
            await sess["client"].close(strict=True)
        for task in tasks:
            if task is not asyncio.current_task():
                task.cancel()
        results = await asyncio.gather(*(task for task in tasks if task is not asyncio.current_task()), return_exceptions=True)
        for result in results:
            if isinstance(result, Exception):
                raise RuntimeError("cliente antigo não encerrou normalmente") from result
        if provider == "claude" and sess is not None:
            await asyncio.gather(sess.preview_buffer.discard(), sess.thinking_buffer.discard(), sess.tool_buffer.discard())
        carry["runtime_counter"] = max(carry.get("runtime_counter") or 0,
            slot.store.state.get("runtime_state", {}).get("view", {}).get("runtime_counter") or 0)
        carry["headless"], carry["name"] = True, name
        return {"runtime_state":carry}

    async def reconnect(self, descriptor, carry):
        name, provider = descriptor["name"], descriptor["provider"]
        adapter = self.adapters[provider]
        existing = adapter._sessions.get(name)
        if provider == "claude" and existing is not None and existing.vivo:
            return {"hydrated":True}
        if provider == "codex" and existing is not None and not existing["client"].closed:
            return {"hydrated":True}
        if provider == "claude":
            original = inspect.unwrap(adapter.ensure_running)
            sess = await original(adapter, name, so_reconectar=True)
            if sess is None or sess.proc is None:
                raise RuntimeError("reserva não reconectou ao cano existente")
        else:
            from app.adapters.codex import sessions
            meta = sessions.load(name)
            if not meta or meta.get("key") != descriptor["key"]:
                raise RuntimeError("sidecar mudou durante a recuperação")
            client = await adapter._ligar_sem_terminal(name, meta, reabrir=False)
            if client is None or client.closed:
                raise RuntimeError("reserva não reconectou ao cano existente")
        view = carry.get("runtime_state") or {}
        conversation = descriptor["meta"].get("session_id" if provider == "claude" else "thread_id")
        if view.get("conversation", view.get("thread_id")) in {None, conversation}:
            sess = adapter._sessions[name]
            if provider == "claude":
                for source, target in {"model":"model", "effort":"effort", "permission_mode":"permission_mode",
                    "previous_non_plan":"modo_nao_plan", "usage":"usage", "context_window":"context_window", "cost":"cost",
                    "commands":"comandos"}.items():
                    if source in view:
                        setattr(sess, target, copy.deepcopy(view[source]))
                if "terminal_commands" in view:
                    sess.comandos_terminal = frozenset(view["terminal_commands"] or [])
            else:
                for field in ("model", "effort", "mode", "token_usage", "rate_limits"):
                    if field in view:
                        sess[field] = copy.deepcopy(view[field])
                if "async_questions" in view:
                    from collections import Counter
                    questions = sess["async_questions"]
                    questions._pending = dict(copy.deepcopy(view["async_questions"]))
                    questions._seen = set(view.get("async_seen") or [])
                    questions._resolved = set(view.get("async_resolved") or [])
                    questions.skipped = set(view.get("skipped_async_questions") or [])
                    questions._local_answers = copy.deepcopy(view.get("async_local_answers") or {})
                    questions._echoes = Counter(view.get("async_echoes") or {})
                    questions._during_load = copy.deepcopy(view.get("async_during_load"))
        return {"hydrated":True}

    async def op(self, descriptor, command, operation_id):
        name, provider = descriptor["name"], descriptor["provider"]
        adapter, io = self.adapters[provider], LegacyIO(self.coordinator)
        kind = command["kind"]
        if kind == "ensure_projection":
            return await io._exec(name, {"kind":"ensure_projection"})
        if kind == "queue":
            return await io._exec(name, command["action"], operation_id)
        if kind == "snapshot":
            with self.coordinator.slot(name).guard:
                return copy.deepcopy(self.coordinator.slot(name).view)
        if kind == "confirm":
            return await self.confirm(descriptor)
        if kind == "drain":
            rows = await io._exec(name, {"kind":"claim", "min_ts":descriptor["meta"].get("created", 0), "limit":1, "entry_id":None})
            sent = 0
            for row in rows:
                reply = await self.op(descriptor, {"kind":"submit", "text":row["text"], "pre_transcript":row.get("pre_transcript", False)}, row["id"])
                sent += reply["disposition"] == "accepted"
            return {"sent":sent}
        if kind == "submit":
            method = "steer" if command.get("steer") else "send_prompt"
            payload = {"text":command["text"], "pre_transcript":command.get("pre_transcript", False)}
            rows = await io._exec(name, {"kind":"load"})
            if not any(row["id"] == operation_id for row in rows):
                await io._exec(name, {"kind":"append", "text":payload["text"], "delivered":False, "ts":None,
                    "pre_transcript":payload["pre_transcript"], "entry_id":operation_id})
            control_kind = "steer" if command.get("steer") else "input"
            entry_id = operation_id
        elif kind == "control":
            control_kind, payload, entry_id = command["control"], command.get("payload") or {}, None
            method = {"answer_questions":"answer_questions", "set_model":"set_model", "set_effort":"set_model",
                "set_permission_mode":"set_permission_mode", "list_models":"list_models", "list_skills":"list_skills",
                "read_rate_limits":"read_rate_limits", "read_settings":"read_settings", "interrupt":"interrupt",
                "select":"select", "compact":"compact", "skip_question":"skip_question", "set_mode":"set_mode"}.get(control_kind)
            if method is None:
                raise RuntimeError("controle não disponível na reserva")
            if provider == "codex" and control_kind == "set_permission_mode":
                method = "set_permission_mode_sem_terminal"
        else:
            raise RuntimeError("operação não disponível na reserva")
        intent = {"operation_id":operation_id, "kind":control_kind, "payload":payload}
        parent = await io._exec(name, {"kind":"prepare", "id":operation_id, "payload":intent, "entry_id":entry_id})
        if parent["status"] in {"accepted", "confirmed", "rejected", "unknown", "dispatching"}:
            if parent["status"] in {"unknown", "dispatching"}:
                return {"operation_id":operation_id, "disposition":"unknown", "payload":{}}
            return parent["result"]
        context = {"key":descriptor["key"], "generation":descriptor["generation"], "operation_id":operation_id,
            "entry_id":entry_id, "command":intent}
        token = _legacy_operation.set(context)
        self.coordinator.legacy_active.add(operation_id)
        try:
            original = inspect.unwrap(getattr(adapter, method))
            if inspect.ismethod(original):
                original = original.__func__
            arguments = {key:value for key,value in payload.items() if key in inspect.signature(original).parameters}
            if method == "set_model":
                arguments.setdefault("model", None)
                arguments.setdefault("effort", None)
            if provider == "codex" and control_kind == "set_permission_mode":
                arguments["modo"] = payload["mode"]
            result = await original(adapter, name, **arguments)
            await io.finish_call(name, context, deferred=result == "deferred")
            disposition = "deferred" if result == "deferred" else "accepted"
            reply = {"operation_id":operation_id, "disposition":disposition,
                "payload":result if isinstance(result, (dict, list)) else {}}
            await io._exec(name, {"kind":"finish", "id":operation_id, "status":disposition, "result":reply})
            if disposition == "deferred" and entry_id is not None:
                await io._exec(name, {"kind":"set_delivered", "entry_id":entry_id, "value":False, "steered":False})
            return reply
        except BaseException:
            await io.finish_call(name, context, failed=True)
            raise
        finally:
            self.coordinator.legacy_active.discard(operation_id)
            _legacy_operation.reset(token)

    async def confirm(self, descriptor):
        from app.runtime_receipt import ReceiptIndex
        slot = self.coordinator.slot(descriptor["name"])
        conversation = descriptor["meta"].get("session_id" if descriptor["provider"] == "claude" else "thread_id") or ""
        index = ReceiptIndex(descriptor["provider"], conversation)
        io = LegacyIO(self.coordinator)
        if not descriptor["jsonl"]:
            return {"confirmed":0}
        await asyncio.to_thread(index.scan, descriptor["jsonl"])
        with slot.guard:
            operations = copy.deepcopy(slot.store.state["operations"])
        count = 0
        for operation_id, operation in operations.items():
            if (operation["status"] == "confirmed" or operation["payload"].get("kind") not in {"input", "steer"}
                    or not operation.get("dispatch_cursor")):
                continue
            with slot.guard:
                state = copy.deepcopy(slot.store.state)
            row = next((row for row in state["rows"] if row["id"] == operation.get("entry_id")), None)
            if row is not None and not row.get("confirmed") and (proof := index.match_after(operation["dispatch_cursor"], row, state["used_occurrences"])):
                count += await io._exec(descriptor["name"], {"kind":"confirm_occurrence", "id":operation_id, "proof":proof}) is True
        return {"confirmed":count}

def accept_ack(endpoint, event):
    phase_id, outcome = event.get("operation_id"), event.get("outcome")
    if not isinstance(phase_id, str) or outcome not in {"written", "not_written", "unknown"}:
        raise ValueError("ACK do cano inválido")
    future = getattr(endpoint, "runtime_acks", {}).get(phase_id)
    if future is not None and not future.done():
        future.set_result(outcome)


def bind_client(name, client):
    coordinator = runtime_coordinator.current()
    if coordinator is None or not coordinator.managed_queue(name):
        return
    assert_legacy(name)
    slot = coordinator.slot(name)
    client.runtime_owner = (name, slot.binding.key, slot.binding.generation)


def runtime_data(name):
    slot = native_slot(name)
    return RuntimeAdapter(slot.binding.provider).view(name).data if slot is not None else None


def registry_method(original):
    signature = inspect.signature(original)
    @functools.wraps(original)
    def wrapper(self, *args, **kwargs):
        arguments = signature.bind(self, *args, **kwargs).arguments
        name = arguments.get("name", arguments.get("old"))
        coordinator = runtime_coordinator.current()
        if (coordinator is None or not coordinator.managed_queue(name)
                or coordinator.in_lifecycle(coordinator.slot(name))):
            return original(self, *args, **kwargs)
        async def action():
            return await asyncio.to_thread(original, self, *args, **kwargs)
        return run_sync(lambda: coordinator.change(name, action,
            new_name=arguments.get("new"), advance=original.__name__ != "rename",
            remove=original.__name__ == "kill"), coordinator.loop)
    return wrapper


def run_sync(factory, loop):
    try:
        running = asyncio.get_running_loop()
    except RuntimeError:
        running = None
    if loop is None or not loop.is_running() or running is loop:
        raise RuntimeError("operação síncrona exige o pool e o loop do servidor")
    future = asyncio.run_coroutine_threadsafe(factory(), loop)
    try:
        return future.result(timeout=185)
    except BaseException:
        future.cancel()
        raise


@dataclass(frozen=True)
class RuntimeView:
    key: str
    generation: int
    revision: int
    data: dict

    @property
    def thread_id(self):
        return self.data.get("thread_id")


def native_slot(name):
    coordinator = runtime_coordinator.current()
    if coordinator is None or not coordinator.managed_runtime(name):
        return None
    slot = coordinator.slot(name)
    if slot.phase == runtime_coordinator.Phase.Python:
        return None
    context = _legacy_operation.get()
    if (slot.phase == runtime_coordinator.Phase.PreparingRust and slot.lease is not None and not slot.lease.closed
            and context is not None and context.get("operation_id") in coordinator.legacy_active):
        return None
    if slot.phase != runtime_coordinator.Phase.Rust:
        raise RuntimeError("sessão em transferência; aguarde a posse ser confirmada")
    return slot


def apply_event(slot, event):
    if (not isinstance(event, dict) or set(event) != {"key", "generation", "revision", "channel", "data"}
            or event["key"] != slot.binding.key or type(event["generation"]) is not int
            or event["generation"] != slot.binding.generation or type(event["revision"]) is not int
            or event["revision"] < 0 or not isinstance(event["channel"], str)):
        return False
    cached = slot.view
    previous = cached.get("revision", -1)
    channel, data, revision = event["channel"], event["data"], event["revision"]
    if channel == "snapshot":
        if (not isinstance(data, dict) or data.get("key") != slot.binding.key
                or data.get("generation") != slot.binding.generation or data.get("revision") != revision
                or not isinstance(data.get("view"), dict) or not isinstance(data.get("channels"), dict)):
            return False
        try:
            StateEvent.model_validate(data["view"]["public_state"])
        except (KeyError, ValueError):
            return False
        if revision < previous:
            return True
        slot.view = copy.deepcopy(data)
        slot.cache_valid = data.get("error") is None
        return True
    if revision <= previous:
        return True
    if not getattr(slot, "cache_valid", False) or revision != previous + 1:
        slot.cache_valid = False
        return False
    updated = copy.deepcopy(cached)
    try:
        if channel == "view":
            if not isinstance(data, dict):
                raise ValueError("view inválida")
            StateEvent.model_validate(data["public_state"])
            updated["view"] = copy.deepcopy(data)
        elif channel == "state":
            StateEvent.model_validate(data)
            updated["view"]["public_state"] = copy.deepcopy(data)
        elif channel in {"preview", "thinking", "tool"}:
            if (not isinstance(data, dict) or not isinstance(data.get("text"), str)
                    or data.get("full") is not True or data.get("md") is not True):
                raise ValueError("prévia inválida")
            updated.setdefault("channels", {})[channel] = copy.deepcopy(data)
        elif channel == "problem":
            if not isinstance(data, dict) or not isinstance(data.get("error_code"), str):
                raise ValueError("falha inválida")
            updated["error"] = data["error_code"]
            slot.cache_valid = False
        elif channel in {"voice", "voice_target"}:
            if not isinstance(data, dict) or not isinstance(data.get("event"), dict):
                raise ValueError("evento de voz inválido")
        else:
            raise ValueError("canal inválido")
    except (KeyError, ValueError):
        slot.cache_valid = False
        return False
    updated["revision"] = revision
    slot.view = updated
    return True


class RuntimeAdapter:
    def __init__(self, provider):
        self.provider = provider

    def view(self, name, *, mutating=False):
        slot = native_slot(name)
        if slot is None:
            raise RuntimeError("sessão fora da posse Rust")
        cached = slot.view
        if cached.get("key") != slot.binding.key or cached.get("generation") != slot.binding.generation:
            raise RuntimeError("estado de outra geração")
        if mutating and not getattr(slot, "cache_valid", False):
            raise RuntimeError("estado do runtime indisponível; aguarde a reposição")
        data = cached.get("view")
        if not isinstance(data, dict):
            raise RuntimeError("snapshot do runtime indisponível")
        return RuntimeView(slot.binding.key, slot.binding.generation, cached.get("revision", 0), copy.deepcopy(data))

    async def control(self, name, kind, payload=None, *, operation_id=None, allow_deferred=False):
        self.view(name, mutating=True)
        reply = await runtime_coordinator.current().op(name, {"kind":"control", "control":kind,
            "payload":payload or {}}, operation_id or uuid.uuid4().hex)
        if reply.get("disposition") != "accepted":
            if allow_deferred and reply.get("disposition") == "deferred":
                return {"_runtime_deferred":True}
            if reply.get("disposition") == "unknown":
                raise RuntimeError("resultado incerto; a operação foi conservada sem reenvio")
            raise ValueError((reply.get("payload") or {}).get("error") or "operação recusada pelo runtime")
        return reply.get("payload")

    async def ensure_running(self, name, **kwargs):
        view = self.view(name, mutating=True)
        if not view.data.get("alive"):
            raise RuntimeError("cano encerrado; recuperação requer ação explícita")
        return view

    async def list_models(self, name):
        return await self.control(name, "list_models")

    def current_model(self, name):
        data = self.view(name).data
        return {"model":data.get("model"), "effort":data.get("effort")}

    def escolhas(self, name):
        data = self.current_model(name)
        return data["model"], data["effort"]

    def snapshot(self, name, thread_id=None):
        view = self.view(name)
        if thread_id is not None and view.thread_id != thread_id:
            raise RuntimeError("snapshot de outra conversa")
        state = StateEvent.model_validate(view.data["public_state"])
        slot = runtime_coordinator.current().slot(name)
        if not slot.cache_valid:
            state = state.model_copy(update={"problema":"headless_turno_erro", "problema_detalhe":"Estado do runtime indisponível; aguarde a reposição."})
        return state

    def comandos(self, name):
        data = self.view(name).data
        return data.get("commands"), frozenset(data.get("terminal_commands") or [])

    def problema_de(self, name):
        state = self.snapshot(name)
        if self.provider == "codex":
            return state.problema
        return (state.problema, state.problema_detalhe) if state.problema else None

    def aprovacao_pendente(self, name):
        state = self.snapshot(name)
        return state.question, state.options

    def permission_modes_sem_terminal(self, name):
        from app.adapters.codex.sem_terminal import modos_para_tela
        return modos_para_tela(self.view(name).data.get("permission_mode"))

    def sync_lifecycle(self, method, name, arguments):
        coordinator = runtime_coordinator.current()
        if not hasattr(coordinator, "lifecycle_call"):
            raise RuntimeError("operação exige a barreira de lifecycle")
        return run_sync(lambda: coordinator.lifecycle_call(name, method, arguments), getattr(coordinator, "loop", None))

    async def deliverable(self, name):
        return self.view(name, mutating=True).data.get("deliverable") is True

    async def drain(self, name, path):
        self.view(name, mutating=True)
        result = await runtime_coordinator.current().op(name, {"kind":"drain"}, uuid.uuid4().hex)
        return int(result["sent"])

    async def state_stream(self, name, sid_get):
        slot = native_slot(name)
        key, generation, instance = slot.binding.key, slot.binding.generation, runtime_coordinator.current().instance
        await runtime_coordinator.current()._push_channels(slot)
        last = -1
        while True:
            coordinator = runtime_coordinator.current()
            if (coordinator.instance != instance or slot.binding.key != key or slot.binding.generation != generation
                    or slot.phase != runtime_coordinator.Phase.Rust):
                return
            slot.changed.clear()
            revision = slot.view.get("revision", -1)
            if revision != last or not slot.cache_valid:
                last = revision
                yield self.snapshot(name)
            await slot.changed.wait()

    async def dispatch(self, method, name, arguments):
        if method == "ensure_running":
            return await self.ensure_running(name)
        if method in {"deliverable", "list_models"}:
            return await getattr(self, method)(name)
        if method == "drain":
            return await self.drain(name, arguments.get("path", ""))
        if method == "send_prompt":
            self.view(name, mutating=True)
            reply = await runtime_coordinator.current().op(name, {"kind":"submit", "text":arguments["text"]}, uuid.uuid4().hex)
            if reply.get("disposition") == "unknown":
                raise RuntimeError("envio incerto; entrada conservada no diário")
            if reply.get("disposition") == "rejected":
                raise ValueError("entrada recusada pelo runtime")
            return "sent" if reply.get("disposition") == "accepted" else "deferred"
        if method == "steer":
            await self.control(name, "steer", {"text":arguments["text"], "turn_id":arguments.get("turn_id")})
            return None
        if method == "steer_queue":
            result = await self.control(name, "steer_queue", {"entry_id":arguments.get("entry_id")})
            return result["ids"]
        if method == "interrupt":
            result = await self.control(name, "interrupt")
            return result.get("interrupted", True) if isinstance(result, dict) else True
        if method == "select":
            await self.control(name, "select", {"option":arguments["option"]})
            return True
        if method == "answer_questions":
            await self.control(name, "answer_questions", {"request_id":arguments["request_id"], "answers":arguments["answers"]})
            return None
        if method == "set_model":
            result = await self.control(name, "set_model", {"model":arguments.get("model"), "effort":arguments.get("effort")}, allow_deferred=self.provider == "claude")
            return not (isinstance(result, dict) and result.get("_runtime_deferred")) if self.provider == "claude" else None
        if method == "set_permission_mode" and self.provider == "claude":
            await self.control(name, "set_permission_mode", {"mode":arguments["mode"]})
            return "manual" if arguments["mode"] == "default" else arguments["mode"]
        if method in {"skip_question", "read_settings", "read_rate_limits", "set_mode", "compact", "list_skills"}:
            payload = {key:value for key,value in arguments.items() if key not in {"self", "name"}}
            result = await self.control(name, method, payload)
            if method == "list_skills":
                from app.adapters.codex.chat_controls import skills_do_catalogo
                return skills_do_catalogo(result)
            return None if method in {"skip_question", "compact"} else result
        if method in {"parar", "recarregar", "restart", "open_terminal", "open_headless", "set_permission_mode_sem_terminal"}:
            return await runtime_coordinator.current().lifecycle_call(name, method, arguments)
        raise RuntimeError("método exige encaminhamento explícito ao responsável")


class NativeVoiceClient:
    virtual = True
    endpoint = None

    def __init__(self, coordinator, name, call_id, target_events):
        self.coordinator, self.name, self.call_id = coordinator, name, call_id
        slot = coordinator.slot(name)
        self.binding = (coordinator.instance, slot.binding.key, slot.binding.generation)
        self.target_thread = RuntimeAdapter("codex").view(name).thread_id
        self.target_events = target_events
        self.events = asyncio.Queue(maxsize=256)
        self.closed = False
        self.failed = False
        self._closing = asyncio.Lock()
        self.thread_id = None
        self._close_id = uuid.uuid4().hex

    def valid(self):
        if self.closed or self.failed:
            return False
        try:
            slot = self.coordinator.slot(self.name)
            return ((self.coordinator.instance, slot.binding.key, slot.binding.generation) == self.binding
                and slot.phase == runtime_coordinator.Phase.Rust and slot.cache_valid
                and slot.view["view"].get("thread_id") == self.target_thread)
        except (KeyError, ValueError):
            return False

    async def request(self, method, params, timeout=30.0):
        if not self.valid():
            raise RuntimeError("chamada de outra posse ou geração")
        result = await asyncio.wait_for(RuntimeAdapter("codex").control(self.name, "voice_rpc",
            {"call_id":self.call_id, "method":method, "params":params}), timeout)
        if not isinstance(result, dict):
            raise RuntimeError("resposta do organizador inválida")
        if method == "thread/start":
            self.thread_id = result["thread"]["id"]
        return result

    async def respond(self, request_id, result, *, erro=None):
        if not self.valid() or type(request_id) not in (int, str):
            raise RuntimeError("resposta de outra chamada ou geração")
        await RuntimeAdapter("codex").control(self.name, "voice_respond",
            {"call_id":self.call_id, "request_id":request_id, "result":result, "error":erro})

    async def notifications(self):
        while True:
            event = await self.events.get()
            if isinstance(event, Exception):
                raise event
            if event is None:
                return
            yield event

    def receive(self, channel, event):
        if not self.valid():
            self.fail(RuntimeError("voz invalidada pela troca de posse"))
            return
        queue = self.events if channel == "voice" else self.target_events
        if queue.full():
            self.fail(RuntimeError("eventos de voz excederam a fila; chamada encerrada"))
            return
        queue.put_nowait(copy.deepcopy(event))

    def fail(self, error):
        self.failed = True
        while self.events.full():
            self.events.get_nowait()
        self.events.put_nowait(error)

    async def close(self):
        async with self._closing:
            if self.closed:
                return
            slot = self.coordinator.slot(self.name)
            owned = (self.coordinator.instance, slot.binding.key, slot.binding.generation) == self.binding and slot.phase == runtime_coordinator.Phase.Rust
            try:
                if owned:
                    if not slot.cache_valid and not await self.coordinator.refresh_snapshot(self.name):
                        raise RuntimeError("estado não reposto; fechamento da voz não confirmado")
                    await RuntimeAdapter("codex").control(self.name, "voice_close", {"call_id":self.call_id}, operation_id=self._close_id)
            finally:
                self.closed = True
                self.coordinator.voice_clients.pop((self.binding[1], self.call_id), None)
                self.fail(RuntimeError("chamada encerrada"))


async def open_voice(name, target_events):
    coordinator = runtime_coordinator.current()
    facade = RuntimeAdapter("codex")
    view = facade.view(name, mutating=True)
    if any(key == view.key and not client.closed for (key, _), client in coordinator.voice_clients.items()):
        raise RuntimeError("chamada de voz já aberta")
    call_id = uuid.uuid4().hex
    client = NativeVoiceClient(coordinator, name, call_id, target_events)
    coordinator.voice_clients[(view.key, call_id)] = client
    try:
        await facade.control(name, "voice_open", {"call_id":call_id})
    except BaseException:
        coordinator.voice_clients.pop((view.key, call_id), None)
        client.closed = True
        raise
    return {"thread_id":view.thread_id, "model":view.data.get("model"), "client":client,
        "voice_events":target_events, "runtime_voice":True}


def voice_current(name, target, adapter):
    if target.get("runtime_voice"):
        return target["client"].valid()
    return adapter._sessions.get(name) is target


_ASYNC = {"ensure_running", "send_prompt", "deliverable", "drain", "steer", "steer_queue", "interrupt", "select",
    "answer_questions", "set_model", "set_permission_mode", "list_models", "read_settings", "read_rate_limits", "set_mode",
    "compact", "list_skills", "skip_question", "parar", "recarregar", "restart", "open_terminal", "open_headless", "set_permission_mode_sem_terminal"}
_SYNC = {"snapshot", "escolhas", "comandos", "problema_de", "current_model", "aprovacao_pendente", "permission_modes_sem_terminal", "rename", "close_sync"}


async def reserve_call(coordinator, original, adapter, method, name, arguments, args, kwargs):
    context = _legacy_operation.get()
    if context is not None and context.get("operation_id") in coordinator.legacy_active:
        return await original(adapter, *args, **kwargs)
    slot = coordinator.slot(name)
    operation_id = uuid.uuid4().hex
    kind = {"send_prompt":"input", "steer":"steer", "set_permission_mode":"set_permission_mode"}.get(method, method)
    payload = {key:value for key,value in arguments.items() if key not in {"self", "name"}}
    context = {"key":slot.binding.key, "generation":slot.binding.generation, "operation_id":operation_id,
        "command":{"operation_id":operation_id, "kind":kind, "payload":payload}}
    token = _legacy_operation.set(context)
    coordinator.legacy_active.add(operation_id)
    try:
        with coordinator.queue_gate(name):
            try:
                result = await original(adapter, *args, **kwargs)
            except BaseException:
                await LegacyIO(coordinator).finish_call(name, context, failed=True)
                raise
            await LegacyIO(coordinator).finish_call(name, context, deferred=result == "deferred")
            return result
    finally:
        coordinator.legacy_active.discard(operation_id)
        _legacy_operation.reset(token)


def install_adapter(cls, provider):
    for method in _ASYNC | _SYNC | {"state_monitor"}:
        original = getattr(cls, method, None)
        if original is None or getattr(original, "runtime_wrapped", False):
            continue
        signature = inspect.signature(original)
        facade = RuntimeAdapter(provider)
        if method == "state_monitor":
            @functools.wraps(original)
            def wrapper(self, name, sid_get, _original=original, _facade=facade):
                if native_slot(name) is not None:
                    return _facade.state_stream(name, sid_get)
                return _original(self, name, sid_get)
        elif method in _SYNC:
            @functools.wraps(original)
            def wrapper(self, *args, _original=original, _signature=signature, _method=method, _facade=facade, **kwargs):
                bound = _signature.bind(self, *args, **kwargs)
                bound.apply_defaults()
                name = bound.arguments.get("name", bound.arguments.get("old"))
                if native_slot(name) is not None:
                    if _method in {"rename", "close_sync"}:
                        return _facade.sync_lifecycle(_method, name, bound.arguments)
                    rest = {key:value for key,value in bound.arguments.items() if key not in {"self", "name"}}
                    return getattr(_facade, _method)(name, **rest)
                return _original(self, *args, **kwargs)
        else:
            @functools.wraps(original)
            async def wrapper(self, *args, _original=original, _signature=signature, _method=method, _facade=facade, **kwargs):
                bound = _signature.bind(self, *args, **kwargs)
                bound.apply_defaults()
                name = bound.arguments["name"]
                coordinator = runtime_coordinator.current()
                context = _legacy_operation.get()
                if context is not None and coordinator is not None and context.get("operation_id") in coordinator.legacy_active:
                    return await _original(self, *args, **kwargs)
                if (coordinator is not None and getattr(coordinator, "legacy", None) is not None
                        and not (coordinator.managed_queue(name) and coordinator.in_lifecycle(coordinator.slot(name)))):
                    await coordinator.prepare_session(name, _facade.provider)
                if native_slot(name) is not None:
                    return await _facade.dispatch(_method, name, bound.arguments)
                coordinator = runtime_coordinator.current()
                if coordinator is not None and coordinator.managed_runtime(name):
                    return await reserve_call(coordinator, _original, self, _method, name, bound.arguments, args, kwargs)
                return await _original(self, *args, **kwargs)
        wrapper.runtime_wrapped = True
        setattr(cls, method, wrapper)
    original = getattr(cls, "acordar", None)
    if original is not None and not getattr(original, "runtime_wrapped", False):
        @functools.wraps(original)
        def wake(self, name, _original=original):
            if native_slot(name) is not None:
                runtime_coordinator.current().request_drain(name)
                return
            coordinator = runtime_coordinator.current()
            if coordinator is not None and coordinator.legacy is not None:
                async def start():
                    try:
                        await self.ensure_running(name)
                        await coordinator.prepare_session(name, provider)
                        await coordinator.op(name, {"kind":"drain"}, uuid.uuid4().hex)
                    except Exception as exc:
                        from app import diag
                        diag.registrar("runtime.wake_failed", "erro", sessao=name, codigo=type(exc).__name__)
                task = coordinator.loop.create_task(start())
                self._tarefas.add(task)
                task.add_done_callback(self._tarefas.discard)
                return
            _original(self, name)
        wake.runtime_wrapped = True
        cls.acordar = wake
