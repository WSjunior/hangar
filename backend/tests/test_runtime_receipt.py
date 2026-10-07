import json

from app.runtime_receipt import ReceiptIndex


def write_echo(path, text="Olá", uuid="echo-1", mode="a"):
    with path.open(mode, encoding="utf-8", newline="") as stream:
        stream.write(json.dumps({"type": "user", "uuid": uuid,
                                 "message": {"role": "user", "content": text}}, ensure_ascii=False) + "\r\n")


def test_one_echo_confirms_only_one_identical_prompt(tmp_path):
    path = tmp_path / "chat.jsonl"
    path.touch()
    index = ReceiptIndex("claude", "sid")
    cursor = index.capture(path)
    write_echo(path)
    index.scan(path)
    proof = index.match_after(cursor, {"text": "Olá"}, {})
    assert proof is not None
    used = {proof["occurrence"]["id"]: {"operation_id": "first"}}
    assert index.match_after(cursor, {"text": "Olá"}, used) is None
    index = ReceiptIndex("claude", "sid")
    index.scan(path)
    assert index.match_after(cursor, {"text": "Olá"}, used) is None


def test_cursor_is_dispatch_not_enqueue(tmp_path):
    path = tmp_path / "chat.jsonl"
    path.touch()
    index = ReceiptIndex("claude", "sid")
    first = index.capture(path)
    write_echo(path)
    second = index.capture(path)
    index.scan(path)
    assert index.match_after(first, {"text": "Olá"}, {}) is not None
    assert index.match_after(second, {"text": "Olá"}, {}) is None


def test_missing_or_partial_transcript_is_no_proof(tmp_path):
    path = tmp_path / "chat.jsonl"
    index = ReceiptIndex("claude", "sid")
    cursor = index.capture(path)
    assert index.scan(path) == []
    path.write_bytes(b'{"type":"user","message":{"content":"Ol')
    index.scan(path)
    assert index.match_after(cursor, {"text": "Olá"}, {}) is None


def test_rewritten_file_invalidates_anchor(tmp_path):
    path = tmp_path / "chat.jsonl"
    write_echo(path, "Texto anterior", "old", "w")
    index = ReceiptIndex("claude", "sid")
    cursor = index.capture(path)
    write_echo(path, "Reescrito aqui", "new", "w")
    write_echo(path)
    index.scan(path)
    assert index.match_after(cursor, {"text": "Olá"}, {}) is None


def test_old_conversation_is_no_proof(tmp_path):
    path = tmp_path / "chat.jsonl"
    path.touch()
    cursor = ReceiptIndex("claude", "old-sid").capture(path)
    write_echo(path)
    index = ReceiptIndex("claude", "new-sid")
    index.scan(path)
    assert index.match_after(cursor, {"text": "Olá"}, {}) is None


def test_enqueue_is_not_consumption(tmp_path):
    path = tmp_path / "chat.jsonl"
    path.touch()
    index = ReceiptIndex("claude", "sid")
    cursor = index.capture(path)
    path.write_text('{"type":"queue-operation","operation":"enqueue","content":"Olá"}\n')
    index.scan(path)
    assert index.match_after(cursor, {"text": "Olá"}, {}) is None
    with path.open("a") as stream:
        stream.write('{"type":"queue-operation","operation":"dequeue","content":"Olá"}\n')
    index.scan(path)
    assert index.match_after(cursor, {"text": "Olá"}, {}) is not None


def test_steer_attachment_is_delivery(tmp_path):
    path = tmp_path / "chat.jsonl"
    path.touch()
    index = ReceiptIndex("claude", "sid")
    cursor = index.capture(path)
    path.write_text(json.dumps({"type": "attachment", "uuid": "steer-1", "attachment": {
        "type": "queued_command", "prompt": [{"type": "text", "text": "Oriente o turno"}]}}) + "\n")
    index.scan(path)
    assert index.match_after(cursor, {"text": "Oriente o turno"}, {}) is not None


def test_attachments_and_unicode(tmp_path):
    path = tmp_path / "chat.jsonl"
    path.touch()
    index = ReceiptIndex("claude", "sid")
    cursor = index.capture(path)
    write_echo(path, "Olá 🌎\n[Image #1]\n📎 imagem: C:\\Fotos\\ação.png")
    index.scan(path)
    assert index.match_after(cursor, {"text": "Olá 🌎 — 📎 imagem: C:\\Fotos\\ação.png"}, {}) is not None


def test_first_prompt_missing_file_needs_new_timestamp_and_same_conversation(tmp_path):
    from datetime import datetime, timezone
    from app.runtime_receipt import validate_proof
    path = tmp_path / "chat.jsonl"
    index = ReceiptIndex("claude", "sid")
    cursor = index.capture(path)
    path.write_text(json.dumps({"type": "user", "sessionId": "sid", "uuid": "first",
        "timestamp": datetime.now(timezone.utc).isoformat(), "message": {"content": "Olá"}}) + "\n")
    index.scan(path)
    proof = index.match_after(cursor, {"text": "Olá"}, {})
    assert proof is not None
    assert validate_proof(proof, cursor, {"text": "Olá"})
    assert cursor["absent_since"] > 0
    assert proof["occurrence"]["recorded_conversation"] == "sid"
    assert index.match_after(cursor, {"text": "Olá"}, {proof["occurrence"]["id"]: {}}) is None


def test_absent_cursor_does_not_accept_old_or_unidentified_conversation(tmp_path):
    from datetime import datetime, timezone
    from app.runtime_receipt import validate_proof
    path = tmp_path / "chat.jsonl"
    index = ReceiptIndex("claude", "sid")
    cursor = index.capture(path)
    for sid, timestamp in [("sid", "2020-01-01T00:00:00Z"), ("other", datetime.now(timezone.utc).isoformat()),
                           (None, datetime.now(timezone.utc).isoformat())]:
        path.write_text(json.dumps({"type": "user", "sessionId": sid, "timestamp": timestamp,
            "message": {"content": "Olá"}}) + "\n")
        index.scan(path)
        assert index.match_after(cursor, {"text": "Olá"}, {}) is None
        occurrence = index.occurrences[0]
        assert not validate_proof({"cursor": cursor, "occurrence": occurrence, "normalized_text": "Olá",
            "observed_anchor": cursor["anchor"]}, cursor, {"text": "Olá"})


def test_rewrite_before_the_read_tail_is_seen_in_the_file_not_in_a_memory_copy(tmp_path):
    # O índice não guarda o transcript: a âncora do despacho é relida do arquivo.
    path = tmp_path / "chat.jsonl"
    first = '{"type":"user","message":{"content":"Anterior"}}\n'
    path.write_text(first, encoding="utf-8")
    index = ReceiptIndex("claude", "sid")
    cursor = index.capture(path)
    echo = '{"type":"user","message":{"content":"Olá"},"pad":"' + "x" * 400 + '"}\n'
    path.write_text(first + echo, encoding="utf-8")
    index.scan(path)
    assert index.match_after(cursor, {"text": "Olá"}, {}) is not None
    with path.open("r+b") as stream:
        stream.write(first.replace("Anterior", "Trocado!").encode())
    index.scan(path)
    assert index.match_after(cursor, {"text": "Olá"}, {}) is None


def test_first_message_dispatched_before_the_transcript_exists_is_confirmed(tmp_path):
    from datetime import datetime, timezone
    from app.runtime_receipt import validate_proof
    path = tmp_path / "chat.jsonl"
    index = ReceiptIndex("claude", "sid")
    cursor = index.capture(path)
    assert cursor["file_identity"] is None
    # Como o Claude grava: a conversa e o horário vêm em cada linha.
    path.write_text(json.dumps({"type": "user", "sessionId": "sid", "timestamp": datetime.now(timezone.utc).isoformat(),
                                "message": {"content": "um"}}) + "\n", encoding="utf-8")
    index.scan(path)
    row = {"text": "um"}
    proof = index.match_after(cursor, row, {})
    assert proof is not None and validate_proof(proof, cursor, row)


def _codex_rollout(path, conversation, text="um"):
    # O Codex grava a conversa só no `session_meta`, não nas linhas da fala.
    from datetime import datetime, timezone
    now = datetime.now(timezone.utc).isoformat()
    lines = [] if conversation is None else [{"timestamp": now, "type": "session_meta", "payload": {"id": conversation}}]
    lines.append({"timestamp": now, "type": "response_item", "payload": {
        "type": "message", "role": "user", "content": [{"type": "input_text", "text": text}]}})
    path.write_text("".join(json.dumps(line) + "\n" for line in lines), encoding="utf-8")


def test_codex_first_message_without_rollout_is_confirmed_once(tmp_path):
    from app.runtime_receipt import validate_proof
    path = tmp_path / "rollout.jsonl"
    index = ReceiptIndex("codex", "thread-1")
    cursor = index.capture(path)
    _codex_rollout(path, "thread-1")
    index.scan(path)
    row = {"text": "um"}
    proof = index.match_after(cursor, row, {})
    assert proof is not None and validate_proof(proof, cursor, row)
    assert proof["occurrence"]["recorded_conversation"] == "thread-1"
    assert index.match_after(cursor, row, {proof["occurrence"]["id"]: {}}) is None


def test_codex_rollout_of_another_conversation_does_not_confirm(tmp_path):
    path = tmp_path / "rollout.jsonl"
    index = ReceiptIndex("codex", "thread-1")
    cursor = index.capture(path)
    _codex_rollout(path, "thread-2")
    index.scan(path)
    assert index.match_after(cursor, {"text": "um"}, {}) is None


def test_codex_rollout_without_session_meta_falls_back_to_the_cursor_without_file(tmp_path):
    path = tmp_path / "rollout.jsonl"
    index = ReceiptIndex("codex", "thread-1")
    cursor = index.capture(path)
    _codex_rollout(path, None)
    index.scan(path)
    proof = index.match_after(cursor, {"text": "um"}, {})
    assert proof is not None and proof["occurrence"]["identity_unprovable"] is True
