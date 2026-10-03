"""Estado autoritativo da fila gerenciada, compartilhado com a reserva Rust."""
from __future__ import annotations

import copy
import json
import os
import tempfile
import time
import uuid
from pathlib import Path

from app import atomico

_coordinator = None
_CALL_PREFIX = "call::"


def configure(coordinator):
    global _coordinator
    _coordinator = coordinator


def initial_state(key: str, generation: int, name: str, rows: list[dict]) -> dict:
    return {"version": 1, "owner_key": key, "generation": generation, "name": name,
            "rows": copy.deepcopy(rows), "operations": {}, "used_occurrences": {}, "runtime_state": {}}


def _operation(operation_id, payload, entry_id=None):
    return {"id": operation_id, "payload": copy.deepcopy(payload), "entry_id": entry_id,
            "status": "prepared", "result": None, "dispatch_cursor": None, "wire_attempts": {}}


class QueueStore:
    """IO só sob a lease do coordenador; o JSONL é uma projeção reparável."""

    def __init__(self, state_path: Path, projection_dir: Path, initial: dict):
        self.state_path, self.projection_dir = Path(state_path), Path(projection_dir)
        self.fenced = False
        self.state_path.parent.mkdir(parents=True, exist_ok=True)
        self.projection_dir.mkdir(parents=True, exist_ok=True)
        if self.state_path.exists():
            self.state = self._read_state()
            if self.state["owner_key"] != initial["owner_key"]:
                raise ValueError("estado da fila pertence a outra chave")
        else:
            self.state = copy.deepcopy(initial)
            self._persist(self.state)
        self.ensure_projection()

    def _read_state(self):
        state = json.loads(self.state_path.read_text(encoding="utf-8"))
        required = {"version", "owner_key", "generation", "name", "rows", "operations", "used_occurrences", "runtime_state"}
        if not isinstance(state, dict) or set(state) != required or state["version"] != 1:
            raise ValueError("estado da fila inválido")
        if not isinstance(state["rows"], list) or not all(isinstance(r, dict) for r in state["rows"]):
            raise ValueError("entradas da fila inválidas")
        for key in ("operations", "used_occurrences", "runtime_state"):
            if not isinstance(state[key], dict):
                raise ValueError("diário da fila inválido")
        return state

    @staticmethod
    def _atomic(path: Path, data: bytes):
        fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
        temp = Path(temporary)
        try:
            with os.fdopen(fd, "wb") as output:
                output.write(data)
                output.flush()
                os.fsync(output.fileno())
            atomico.substituir(temp, path)
            if os.name == "posix":
                directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
                try:
                    os.fsync(directory)
                finally:
                    os.close(directory)
        finally:
            temp.unlink(missing_ok=True)

    def _persist(self, state):
        self.fenced = True
        self._atomic(self.state_path, json.dumps(state, ensure_ascii=False).encode("utf-8"))
        self.state = state
        self.fenced = False

    def repair(self):
        self.state = self._read_state()
        self.ensure_projection()
        self.fenced = False

    def ensure_projection(self):
        from app.pqueue import _sanitize
        path = self.projection_dir / f"{_sanitize(self.state['name'])}.jsonl"
        data = "".join(json.dumps(r, ensure_ascii=False) + "\n" for r in self.state["rows"]).encode("utf-8")
        self._atomic(path, data)
        previous = self.state["runtime_state"].get("_queue_previous_name")
        if previous and previous != self.state["name"]:
            (self.projection_dir / f"{_sanitize(previous)}.jsonl").unlink(missing_ok=True)

    def exec(self, generation: int, call_id: str, clock: dict, action: dict):
        if self.fenced:
            if action["kind"] != "ensure_projection":
                raise OSError("fila bloqueada após falha de persistência; reparar antes de continuar")
            self.repair()
        if generation != self.state["generation"]:
            raise ValueError("geração da fila mudou")
        if not call_id:
            raise ValueError("operação da fila sem identificador")
        receipt_id = _CALL_PREFIX + call_id
        previous = self.state["operations"].get(receipt_id)
        if previous is not None:
            if previous["payload"] != action:
                raise ValueError("identificador reutilizado com outra operação")
            self.ensure_projection()
            return copy.deepcopy(previous["result"])
        self.ensure_projection()
        state = copy.deepcopy(self.state)
        result = apply_action(state, action, clock, call_id)
        if action["kind"] not in {"load", "entry_delivered", "ensure_projection"}:
            receipt = _operation(receipt_id, action)
            receipt.update(status="accepted", result=copy.deepcopy(result))
            state["operations"][receipt_id] = receipt
            self._persist(state)
            self.ensure_projection()
        return copy.deepcopy(result)


def _protected(state, entry_id):
    return any(op["entry_id"] == entry_id and op["status"] in {"dispatching", "unknown"}
               for operation_id, op in state["operations"].items() if not operation_id.startswith(_CALL_PREFIX))


def apply_action(state, action, clock, call_id):
    from app.models import scrub_surrogates
    from app.pqueue import PromptQueue

    kind, rows, operations = action["kind"], state["rows"], state["operations"]
    if kind in {"load", "ensure_projection"}:
        return rows if kind == "load" else None
    if kind in {"append", "append_local"}:
        entry_id = action.get("entry_id") or call_id
        if any(row.get("id") == entry_id for row in rows):
            raise ValueError("entrada da fila já existe")
        row = {"id": entry_id, "text": scrub_surrogates(action["text"]),
               "ts": clock["epoch_s"] if action.get("ts") is None else action["ts"],
               "delivered": action.get("delivered", kind == "append_local")}
        if kind == "append_local":
            row.update(delivered=True, confirmed=True, papel="assistant")
        elif action.get("pre_transcript"):
            row["pre_transcript"] = True
        overflow = len(rows) + 1 - 1000
        candidates = [r for r in rows if (r.get("confirmed") or r.get("papel") == "assistant") and not _protected(state, r.get("id"))]
        if overflow > len(candidates):
            raise ValueError("fila cheia de entradas pendentes")
        for candidate in candidates[:max(0, overflow)]:
            rows.remove(candidate)
        rows.append(row)
        return row
    if kind == "prepare":
        operation_id = action["id"]
        if operation_id.startswith(_CALL_PREFIX):
            raise ValueError("identificador reservado")
        if old := operations.get(operation_id):
            if old["payload"] != action["payload"] or old["entry_id"] != action.get("entry_id"):
                raise ValueError("intenção da operação mudou")
            return old
        operations[operation_id] = _operation(operation_id, action["payload"], action.get("entry_id"))
        return operations[operation_id]
    if kind in {"bind_dispatch", "begin_dispatch", "finish", "late_rpc_resolution"}:
        operation = operations.get(action["id"])
        if operation is None:
            raise ValueError("operação não preparada")
        if kind == "bind_dispatch":
            if operation["status"] != "prepared":
                raise ValueError("cursor precisa preceder o despacho")
            operation["dispatch_cursor"] = action["cursor"]
        elif kind == "begin_dispatch":
            if operation["status"] not in {"prepared", "dispatching"}:
                raise ValueError("operação não pode ser reenviada")
            wire_id = action["wire_id"]
            operation["wire_attempts"].setdefault(wire_id, {"status": "dispatching", "result": None})
            operation["status"] = "dispatching"
            for row in rows:
                if row.get("id") == operation["entry_id"]:
                    row["delivered"] = True
        elif kind == "late_rpc_resolution":
            if action["generation"] != state["generation"]:
                raise ValueError("resposta de outra geração")
            request_id = action["request_id"]
            expected = operation["payload"].get("request_id", operation["payload"].get("id"))
            if type(request_id) not in (int, str) or type(request_id) is not type(expected) or request_id != expected:
                raise ValueError("resposta não corresponde ao pedido")
            attempt = operation["wire_attempts"].get(action["wire_id"])
            if attempt is None:
                raise ValueError("tentativa não registrada")
            attempt.update(status="accepted", result=action["result"])
            if all(wire["status"] == "accepted" for wire in operation["wire_attempts"].values()) and operation["status"] != "confirmed":
                operation.update(status="accepted", result=action["result"])
        else:
            status = action.get("status", "accepted")
            if operation["status"] in {"accepted", "confirmed", "rejected"} and status == "unknown":
                return operation
            if operation["status"] == "unknown" and status in {"prepared", "dispatching", "deferred"}:
                raise ValueError("resultado incerto não permite reenvio")
            operation.update(status=status, result=action.get("result"))
        return operation
    if kind == "recover":
        for operation in operations.values():
            if operation["status"] == "dispatching":
                operation["status"] = "unknown"
                for attempt in operation["wire_attempts"].values():
                    if attempt["status"] == "dispatching":
                        attempt["status"] = "unknown"
        return None
    if kind == "set_runtime_state":
        if not isinstance(action["state"], dict):
            raise ValueError("estado privado inválido")
        state["runtime_state"] = action["state"]
        return None
    if kind == "rename":
        if not action["name"]:
            raise ValueError("nome vazio na fila")
        state["runtime_state"]["_queue_previous_name"] = state["name"]
        state["name"] = action["name"]
        return None
    if kind == "clear":
        state["rows"] = []
        return None
    if kind == "replace_rows":
        replacement = action["rows"]
        for row in rows:
            if _protected(state, row.get("id")) and row not in replacement:
                raise ValueError("substituição removeria uma entrada incerta")
        state["rows"] = replacement
        return None
    if kind == "set_delivered" and not action["value"] and _protected(state, action["entry_id"]):
        raise ValueError("entrega incerta não pode voltar para a fila")
    if kind == "abandon":
        for row in rows:
            if row.get("id") == action["entry_id"]:
                row.update(delivered=True, desistiu=True, desistiu_ts=clock["epoch_s"])
        return None

    class MemoryQueue(PromptQueue):
        def __init__(self):
            self.name = state["name"]
            self.path = Path(f"{self.name}.jsonl")
            self.routing_disabled = True

        def load(self):
            return copy.deepcopy([r for r in state["rows"] if not _protected(state, r.get("id"))])

        def _write_atomic(self, new_rows):
            protected = [(i, r) for i, r in enumerate(state["rows"]) if _protected(state, r.get("id"))]
            for index, row in protected:
                new_rows.insert(min(index, len(new_rows)), copy.deepcopy(row))
            state["rows"] = new_rows

    queue = MemoryQueue()
    if kind == "entry_delivered":
        return next((bool(r.get("delivered")) for r in rows if r.get("id") == action["entry_id"]), None)
    methods = {"claim": "claim_undelivered", "set_delivered": "set_delivered", "abandon": "desistir",
               "bump_attempts": "bump_attempts", "prune": "prune_before", "reconcile": "reconcile_delivered", "remove": "remove"}
    if kind == "confirm":
        selected = set(action["entry_ids"])
        return queue.confirm_delivered(lambda row: row.get("id") in selected)
    if kind not in methods:
        raise ValueError("ação da fila desconhecida")
    args = {k: v for k, v in action.items() if k != "kind"}
    if kind == "reconcile":
        args["committed"] = set(args["committed"])
        args["na_fila_tui"] = set(args.get("na_fila_tui", []))
    return getattr(queue, methods[kind])(**args)


def route_queue(queue, method: str, args: dict):
    if _coordinator is None or getattr(queue, "routing_disabled", False):
        return False, None
    with _coordinator.queue_gate(queue.name) as route:
        if route is None:
            return False, None
        kind = {"append_saida_local": "append_local", "claim_undelivered": "claim", "desistir": "abandon",
                "prune_before": "prune", "reconcile_delivered": "reconcile", "confirm_delivered": "confirm",
                "rename": "rename", "_write_atomic": "replace_rows"}.get(method, method)
        payload = dict(args)
        if kind in {"append", "append_local"}:
            payload["entry_id"] = uuid.uuid4().hex
        if kind == "confirm":
            rows = _coordinator.queue_rpc(route, uuid.uuid4().hex, _clock(), {"kind": "load"})
            predicate = payload.pop("apenas", None)
            payload["entry_ids"] = [r["id"] for r in rows if predicate is None or predicate(r)]
        if kind == "rename":
            payload["name"] = payload.pop("new_name")
        for key in ("committed", "na_fila_tui"):
            if key in payload:
                payload[key] = sorted(payload[key])
        result = _coordinator.queue_rpc(route, uuid.uuid4().hex, _clock(), {"kind": kind, **payload})
        return True, result


def _clock():
    return {"monotonic_s": time.monotonic(), "epoch_s": time.time()}
