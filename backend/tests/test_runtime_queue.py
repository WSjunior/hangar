import json

import pytest

from app.runtime_queue import QueueStore, initial_state

CLOCK = {"monotonic_s": 10.0, "epoch_s": 1800000000.0}


def open_store(tmp_path, rows=None):
    return QueueStore(tmp_path / "key.queue-state.json", tmp_path / "projection",
                      initial_state("key", 1, "session", rows or []))


def append(text="Olá", entry_id="entry-1"):
    return {"kind": "append", "text": text, "entry_id": entry_id,
            "delivered": False, "ts": None, "pre_transcript": False}


def test_same_operation_does_not_append_twice(tmp_path):
    store = open_store(tmp_path)
    first = store.exec(1, "call-1", CLOCK, append())
    assert store.exec(1, "call-1", CLOCK, append()) == first
    assert len(store.state["rows"]) == 1
    with pytest.raises(ValueError):
        store.exec(1, "call-1", CLOCK, append("Outro texto"))


def test_state_before_projection(tmp_path, monkeypatch):
    store = open_store(tmp_path)
    original = store.ensure_projection
    monkeypatch.setattr(store, "ensure_projection", lambda: (_ for _ in ()).throw(OSError("projection")))
    with pytest.raises(OSError):
        store.exec(1, "call-1", CLOCK, append())
    assert len(json.loads(store.state_path.read_text())["rows"]) == 1
    monkeypatch.setattr(store, "ensure_projection", original)
    row = store.exec(1, "call-1", CLOCK, append())
    assert row["id"] == "entry-1"
    assert len(store.state["rows"]) == 1
    assert json.loads((store.projection_dir / "session.jsonl").read_text()) == row


def test_corrupt_state_is_not_empty_queue(tmp_path):
    store = open_store(tmp_path)
    store.state_path.write_text("{")
    with pytest.raises(ValueError):
        open_store(tmp_path)


def test_unknown_never_unclaims(tmp_path):
    store = open_store(tmp_path)
    store.exec(1, "append", CLOCK, append())
    store.exec(1, "prepare", CLOCK, {"kind": "prepare", "id": "op", "payload": {"text": "Olá"}, "entry_id": "entry-1"})
    store.exec(1, "dispatch", CLOCK, {"kind": "begin_dispatch", "id": "op", "wire_id": "wire:op:1"})
    store.exec(1, "unknown", CLOCK, {"kind": "finish", "id": "op", "status": "unknown", "result": None})
    with pytest.raises(ValueError):
        store.exec(1, "unclaim", CLOCK, {"kind": "set_delivered", "entry_id": "entry-1", "value": False, "steered": False})
    assert store.state["rows"][0]["delivered"] is True
    assert not store.state["rows"][0].get("confirmed")


def test_cap_keeps_pending(tmp_path):
    rows = [{"id": str(i), "text": "pending", "ts": 1, "delivered": True, "desistiu": True} for i in range(1000)]
    store = open_store(tmp_path, rows)
    with pytest.raises(ValueError):
        store.exec(1, "new", CLOCK, append())
    assert store.state["rows"] == rows


def test_rename_keeps_key(tmp_path):
    store = open_store(tmp_path)
    store.exec(1, "append", CLOCK, append())
    store.exec(1, "rename", CLOCK, {"kind": "rename", "name": "new-name"})
    assert store.state["owner_key"] == "key"
    assert store.state_path.name == "key.queue-state.json"
    assert not (store.projection_dir / "session.jsonl").exists()
    assert (store.projection_dir / "new-name.jsonl").exists()


def test_unknown_reply_cannot_downgrade_final(tmp_path):
    store = open_store(tmp_path)
    store.exec(1, "prepare", CLOCK, {"kind": "prepare", "id": "op", "payload": {}, "entry_id": None})
    store.exec(1, "accepted", CLOCK, {"kind": "finish", "id": "op", "status": "accepted", "result": {"ok": True}})
    store.exec(1, "late", CLOCK, {"kind": "finish", "id": "op", "status": "unknown", "result": None})
    assert store.state["operations"]["op"]["status"] == "accepted"
    assert store.state["operations"]["op"]["result"] == {"ok": True}


def test_wrong_generation_cannot_mutate(tmp_path):
    store = open_store(tmp_path)
    with pytest.raises(ValueError):
        store.exec(2, "append", CLOCK, append())
    assert store.state["rows"] == []


def test_recovery_keeps_uncertain_dispatch_visible(tmp_path):
    store = open_store(tmp_path)
    store.exec(1, "append", CLOCK, append())
    store.exec(1, "prepare", CLOCK, {"kind": "prepare", "id": "op", "payload": {}, "entry_id": "entry-1"})
    store.exec(1, "dispatch", CLOCK, {"kind": "begin_dispatch", "id": "op", "wire_id": "wire:op:1"})
    store = open_store(tmp_path)
    store.exec(1, "recover", CLOCK, {"kind": "recover"})
    assert store.state["operations"]["op"]["status"] == "unknown"
    assert store.state["rows"][0]["delivered"] is True


def test_fsync_after_rename_fences_store(tmp_path, monkeypatch):
    store = open_store(tmp_path)
    original = store._atomic

    def fail_after_commit(path, data):
        original(path, data)
        if path == store.state_path:
            raise OSError("directory fsync")

    monkeypatch.setattr(store, "_atomic", fail_after_commit)
    with pytest.raises(OSError):
        store.exec(1, "call-1", CLOCK, append())
    assert json.loads(store.state_path.read_text())["rows"][0]["id"] == "entry-1"
    with pytest.raises(OSError):
        store.exec(1, "call-2", CLOCK, append(entry_id="entry-2"))
    monkeypatch.setattr(store, "_atomic", original)
    store.exec(1, "repair", CLOCK, {"kind": "ensure_projection"})
    assert store.exec(1, "call-1", CLOCK, append())["id"] == "entry-1"
    assert len(store.state["rows"]) == 1


@pytest.mark.parametrize("generation, request_id", [(2, 1), (1, "1"), (1, True)])
def test_late_reply_matches_generation_and_type(tmp_path, generation, request_id):
    store = open_store(tmp_path)
    store.exec(1, "prepare", CLOCK, {"kind": "prepare", "id": "op", "payload": {"request_id": 1}, "entry_id": None})
    store.exec(1, "begin", CLOCK, {"kind": "begin_dispatch", "id": "op", "wire_id": "wire:1"})
    with pytest.raises(ValueError):
        store.exec(1, "late", CLOCK, {"kind": "late_rpc_resolution", "id": "op", "wire_id": "wire:1",
                                     "generation": generation, "request_id": request_id, "result": {"ok": True}})
    assert store.state["operations"]["op"]["status"] == "dispatching"


def test_facade_and_reserve_share_authoritative_state(tmp_path, monkeypatch):
    from contextlib import contextmanager
    from app import runtime_queue
    from app.pqueue import PromptQueue
    store = open_store(tmp_path)
    monkeypatch.setattr("app.pqueue._queue_dir", lambda: store.projection_dir)

    class Coordinator:
        @contextmanager
        def queue_gate(self, name):
            yield store

        def queue_rpc(self, route, call_id, clock, action):
            return route.exec(1, call_id, clock, action)

    runtime_queue.configure(Coordinator())
    try:
        queue = PromptQueue("session")
        row = queue.append("Fila gerenciada")
        assert queue.load() == [row]
        assert queue.claim_undelivered() == [{**row, "delivered": True}]
        assert queue.entry_delivered(row["id"]) is True
        assert queue.bump_attempts(row["id"]) == 1
        queue.set_delivered(row["id"], False)
        assert queue.entry_delivered(row["id"]) is False
        queue.desistir(row["id"])
        assert queue.remove(row["id"]) is True
        local = queue.append_saida_local("Saída local")
        assert local["confirmed"] is True
        queue.rename("renamed")
        assert store.state["name"] == "renamed"
        queue.clear()
        assert queue.load() == []
    finally:
        runtime_queue.configure(None)


def test_distinct_occurrence_is_committed_with_confirmation(tmp_path):
    from app.runtime_receipt import ReceiptIndex
    store = open_store(tmp_path)
    transcript = tmp_path / "chat.jsonl"
    transcript.touch()
    index = ReceiptIndex("claude", "sid")
    cursor = index.capture(transcript)
    for operation_id in ("op-1", "op-2"):
        row_id = "entry-" + operation_id
        store.exec(1, "append-" + operation_id, CLOCK, append(entry_id=row_id))
        store.exec(1, "prepare-" + operation_id, CLOCK, {"kind": "prepare", "id": operation_id, "payload": {}, "entry_id": row_id})
        store.exec(1, "bind-" + operation_id, CLOCK, {"kind": "bind_dispatch", "id": operation_id, "cursor": cursor})
        store.exec(1, "begin-" + operation_id, CLOCK, {"kind": "begin_dispatch", "id": operation_id, "wire_id": "wire:" + operation_id})
    transcript.write_text('{"type":"user","uuid":"echo-1","message":{"content":"Olá"}}\n')
    index.scan(transcript)
    proof = index.match_after(cursor, store.state["rows"][0], {})
    assert store.exec(1, "confirm-1", CLOCK, {"kind": "confirm_occurrence", "id": "op-1", "proof": proof}) is True
    assert store.exec(1, "confirm-2", CLOCK, {"kind": "confirm_occurrence", "id": "op-2", "proof": proof}) is False
    store = open_store(tmp_path)
    assert sum(bool(row.get("confirmed")) for row in store.state["rows"]) == 1
    assert len(store.state["used_occurrences"]) == 1
