"""Estado autoritativo da fila gerenciada, compartilhado com a reserva Rust."""
from __future__ import annotations

import copy
import json
import os
import tempfile
import threading
import time
import uuid
from pathlib import Path

from app import atomico

_coordinator = None
_CALL_PREFIX = "call::"
_VERSION = 2
# Janela de chamadas recentes que nunca sai: cobre a repetição da mesma chamada (3 tentativas,
# pausa e a última pelo Python) e o ACK atrasado de uma fase. O resto é podado pelo estado.
_RECENT_CALLS = 256
_FINAL = {"accepted", "rejected", "confirmed"}
_TARGETED = {"prepare", "bind_dispatch", "begin_dispatch", "finish", "late_rpc_resolution", "confirm_occurrence"}


def configure(coordinator):
    global _coordinator
    _coordinator = coordinator


def initial_state(key: str, generation: int, name: str, rows: list[dict]) -> dict:
    return {"version": _VERSION, "owner_key": key, "generation": generation, "name": name,
            "rows": copy.deepcopy(rows), "operations": {}, "used_occurrences": {}, "runtime_state": {}, "next_seq": 1}


def _operation(operation_id, payload, entry_id=None):
    return {"id": operation_id, "payload": copy.deepcopy(payload), "entry_id": entry_id,
            "status": "prepared", "result": None, "dispatch_cursor": None, "wire_attempts": {}, "seq": 0}


class QueueStore:
    """IO só sob a lease do coordenador; o JSONL é uma projeção reparável."""

    def __init__(self, state_path: Path, projection_dir: Path, initial: dict):
        self.state_path, self.projection_dir = Path(state_path), Path(projection_dir)
        self.fenced = False
        # Serializa a gravação aqui, e não na trava do slot: o laço de eventos confere a posse
        # naquela trava e não pode esperar o disco.
        self._lock = threading.RLock()
        self.state_path.parent.mkdir(parents=True, exist_ok=True)
        self.projection_dir.mkdir(parents=True, exist_ok=True)
        if self.state_path.exists():
            self.state = self._read_state()
            if self.state["owner_key"] != initial["owner_key"]:
                raise ValueError("estado da fila pertence a outra chave")
            if self._migrated:
                self._persist(self.state)
        else:
            self.state = copy.deepcopy(initial)
            self._persist(self.state)
        self.ensure_projection()

    def _read_state(self):
        state = json.loads(self.state_path.read_text(encoding="utf-8"))
        self._migrated = isinstance(state, dict) and state.get("version") == 1
        required = {"version", "owner_key", "generation", "name", "rows", "operations", "used_occurrences", "runtime_state"}
        if not isinstance(state, dict) or state.get("version") not in {1, _VERSION}:
            raise ValueError("estado da fila inválido")
        if state["version"] == 1 and set(state) == required:
            # Sem ordem gravada, tudo o que veio da v1 conta como antigo: fora da janela já na leitura.
            state.update(version=_VERSION, next_seq=_RECENT_CALLS + 1)
            for operation in state["operations"].values() if isinstance(state["operations"], dict) else ():
                if isinstance(operation, dict):
                    operation.setdefault("seq", 0)
        if (set(state) != required | {"next_seq"} or state["version"] != _VERSION
                or type(state["next_seq"]) is not int or state["next_seq"] < 1):
            raise ValueError("estado da fila inválido")
        if not isinstance(state["rows"], list) or not all(isinstance(r, dict) for r in state["rows"]):
            raise ValueError("entradas da fila inválidas")
        for key in ("operations", "used_occurrences", "runtime_state"):
            if not isinstance(state[key], dict):
                raise ValueError("diário da fila inválido")
        if not all(isinstance(op, dict) and type(op.get("seq")) is int for op in state["operations"].values()):
            raise ValueError("diário da fila inválido")
        compact(state)
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
        # As etapas de uma entrega não mudam as mensagens: regravar igual só custava fsync. Compara
        # com o arquivo, não com memória, para continuar consertando o que mudou por fora.
        try:
            unchanged = path.read_bytes() == data
        except OSError:
            unchanged = False
        if not unchanged:
            self._atomic(path, data)
        previous = self.state["runtime_state"].get("_queue_previous_name")
        if previous and previous != self.state["name"]:
            (self.projection_dir / f"{_sanitize(previous)}.jsonl").unlink(missing_ok=True)

    def exec(self, generation: int, call_id: str, clock: dict, action: dict):
        with self._lock:
            return self._exec(generation, call_id, clock, action)

    def _exec(self, generation: int, call_id: str, clock: dict, action: dict):
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
            if previous["payload"] != _receipt_payload(action):
                raise ValueError("identificador reutilizado com outra operação")
            self.ensure_projection()
            return copy.deepcopy(previous["result"])
        if action["kind"] in {"load", "entry_delivered", "ensure_projection"}:
            self.ensure_projection()
        state = copy.deepcopy(self.state)
        # Cópia antes do carimbo: o resultado guarda a operação como o Rust a serializa no apply.
        result = copy.deepcopy(apply_action(state, action, clock, call_id))
        if action["kind"] not in {"load", "entry_delivered", "ensure_projection"}:
            seq = state["next_seq"]
            state["next_seq"] = seq + 1
            if action["kind"] in _TARGETED and action["id"] in state["operations"]:
                state["operations"][action["id"]]["seq"] = seq
            receipt = _operation(receipt_id, _receipt_payload(action))
            receipt_result = copy.deepcopy(result)
            if action["kind"] in _TARGETED and isinstance(receipt_result, dict):
                _slim_operation(receipt_result)
            receipt.update(status="accepted", result=receipt_result, seq=seq)
            state["operations"][receipt_id] = receipt
            compact(state)
            self._persist(state)
            self.ensure_projection()
        return copy.deepcopy(result)


def _protected_ids(state):
    # Um conjunto por ação, como o `protected` do apply em Rust: conferir linha a linha contra o
    # diário inteiro custava linhas × operações a cada gravação.
    return {op["entry_id"] for operation_id, op in state["operations"].items()
            if not operation_id.startswith(_CALL_PREFIX) and op["status"] in {"dispatching", "unknown"}}


def _slim(result):
    # Resposta guardada só é relida pela disposição (e pelo formato de RuntimeReply); o conteúdo
    # já foi entregue a quem pediu e era o que fazia o diário pesar megabytes.
    if isinstance(result, dict) and "payload" in result:
        return {**result, "payload": None}
    return result


def _slim_operation(operation):
    operation["result"] = _slim(operation.get("result"))
    for attempt in operation.get("wire_attempts", {}).values():
        if isinstance(attempt, dict) and "result" in attempt:
            attempt["result"] = _slim(attempt["result"])


def _receipt_payload(action):
    """O recibo guarda a ação sem o volume: basta para detectar reuso do mesmo identificador."""
    if action.get("kind") == "set_runtime_state":
        return {**action, "state": None}
    if "result" in action:
        return {**action, "result": _slim(action["result"])}
    return action


def _group(key, operation):
    payload = operation["payload"]
    logical = payload.get("logical_id") if isinstance(payload, dict) else None
    return logical if isinstance(logical, str) else key


def _may_confirm(operation, record) -> bool:
    """A ocorrência ainda pode confirmar esta operação? Na dúvida, sim."""
    if operation["entry_id"] is None or operation["status"] == "confirmed":
        return False
    # Preparada ainda pode ligar cursor. Todo bind captura depois de a operação existir, então
    # quem nasce depois da poda nasce com cursor depois da ocorrência.
    if operation["status"] == "prepared":
        return True
    cursor = operation["dispatch_cursor"]
    if not isinstance(cursor, dict):
        return False
    offset, cursor_offset = record.get("offset"), cursor.get("offset")
    if type(offset) is not int or type(cursor_offset) is not int:
        return True
    return (cursor.get("conversation") == record.get("conversation")
            and cursor.get("file_identity") == record.get("file_identity") and offset >= cursor_offset)


def compact(state):
    """Poda o que nada mais lê; mesma regra do `compact` em queue.rs.

    Fica: as últimas `_RECENT_CALLS` chamadas (recibo e operação tocada), operação não final,
    operação de entrada ainda não confirmada e o grupo inteiro (raiz + fases por `logical_id`)
    de quem ficou. Operação final que fica perde o conteúdo da resposta (`_slim`). Ocorrência
    usada só sai quando nenhuma operação restante pode casá-la.
    """
    operations = state["operations"]
    cutoff = state["next_seq"] - _RECENT_CALLS
    open_rows = {row.get("id") for row in state["rows"] if row.get("confirmed") is not True}

    def held(key, operation):
        if operation["seq"] >= cutoff:
            return True
        return not key.startswith(_CALL_PREFIX) and (operation["status"] not in _FINAL
            or operation["entry_id"] is not None and operation["entry_id"] in open_rows)

    groups = {_group(key, op) for key, op in operations.items() if not key.startswith(_CALL_PREFIX) and held(key, op)}
    state["operations"] = {key: op for key, op in operations.items() if held(key, op)
        or not key.startswith(_CALL_PREFIX) and _group(key, op) in groups}
    for key, op in state["operations"].items():
        if not key.startswith(_CALL_PREFIX) and op["status"] in _FINAL:
            _slim_operation(op)
    # Linha confirmada não volta a confirmar (confirm_occurrence recusa), mesmo que a resposta
    # tardia tenha devolvido a operação para `accepted`.
    confirmed_rows = {row.get("id") for row in state["rows"] if row.get("confirmed") is True}
    candidates = [op for key, op in state["operations"].items()
                  if not key.startswith(_CALL_PREFIX) and op["entry_id"] not in confirmed_rows]
    state["used_occurrences"] = {occurrence: record for occurrence, record in state["used_occurrences"].items()
        if not isinstance(record, dict) or any(_may_confirm(op, record) for op in candidates)}


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
        protected = _protected_ids(state)
        candidates = [r for r in rows if (r.get("confirmed") or r.get("papel") == "assistant") and r.get("id") not in protected]
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
            if old["status"] == "deferred":
                old.update(status="prepared", result=None)
            return old
        operation = operations[operation_id] = _operation(operation_id, action["payload"], action.get("entry_id"))
        # Linha já confirmada pela fila: a mesma intenção chegando depois da poda não reenvia.
        if (not (isinstance(action["payload"], dict) and "logical_id" in action["payload"])
                and operation["entry_id"] is not None
                and any(row.get("id") == operation["entry_id"] and row.get("confirmed") is True for row in rows)):
            operation.update(status="accepted", result={"operation_id": operation_id, "disposition": "accepted",
                                                         "payload": {"already_confirmed": True}})
        return operation
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
            if (operation["status"] in {"accepted", "confirmed", "rejected"}
                    and isinstance(operation["result"], dict) and "disposition" in operation["result"]
                    and isinstance(action.get("result"), dict) and "write_outcome" in action["result"]):
                return operation
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
    if kind == "confirm_occurrence":
        from app.runtime_receipt import validate_proof
        proof = action["proof"]
        occurrence_id = proof["occurrence"]["id"]
        if occurrence_id in state["used_occurrences"]:
            return False
        operation = operations.get(action["id"])
        if operation is None or operation["dispatch_cursor"] is None:
            raise ValueError("operação sem cursor de despacho")
        row = next((r for r in rows if r.get("id") == operation["entry_id"]), None)
        if operation["status"] == "confirmed" or row is not None and row.get("confirmed"):
            return False
        if row is None or not validate_proof(proof, operation["dispatch_cursor"], row):
            raise ValueError("prova de entrega não corresponde ao despacho")
        row.update(delivered=True, confirmed=True)
        row.pop("desistiu", None)
        occurrence = proof["occurrence"]
        state["used_occurrences"][occurrence_id] = {"operation_id": action["id"], "generation": state["generation"],
            "conversation": occurrence["conversation"], "file_identity": occurrence["file_identity"],
            "offset": occurrence["offset"]}
        operation["status"] = "confirmed"
        return True
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
        protected = _protected_ids(state)
        for row in rows:
            if row.get("id") in protected and row not in replacement:
                raise ValueError("substituição removeria uma entrada incerta")
        state["rows"] = replacement
        return None
    if kind == "set_delivered" and not action["value"] and action["entry_id"] in _protected_ids(state):
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
            protected = _protected_ids(state)
            return copy.deepcopy([r for r in state["rows"] if r.get("id") not in protected])

        def _write_atomic(self, new_rows):
            ids = _protected_ids(state)
            protected = [(i, r) for i, r in enumerate(state["rows"]) if r.get("id") in ids]
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
