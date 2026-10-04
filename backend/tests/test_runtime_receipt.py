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
