import asyncio
import json

import pytest

from app import pqueue, sse
from app.models import ChatEvent


@pytest.mark.parametrize("reconnect", [False, True])
async def test_codex_confirms_rollout_before_replaying_queue(tmp_path, monkeypatch, reconnect):
    monkeypatch.setattr(pqueue, "_queue_dir", lambda: tmp_path)
    queue = pqueue.PromptQueue("codex-queue")
    queue.append("[de: outra] orientação", delivered=True)
    queue.append("ainda aguardando", delivered=False)
    queue.append("RPC aceito, ainda sem escrita", delivered=True)
    path = tmp_path / "rollout.jsonl"
    path.write_text("")

    def commit():
        path.write_text(json.dumps({"type": "response_item", "payload": {
            "type": "message", "role": "user", "id": "real",
            "content": [{"type": "input_text", "text": "[de: outra] orientação"}],
        }}) + "\n")

    class Adapter:
        async def transcript_stream(self, path, start_offset=None):
            if not reconnect:
                commit()
            yield ChatEvent(kind="user_msg", id="real", text="[de: outra] orientação")

        async def state_monitor(self, name, sid_get):
            if False:
                yield

    if reconnect:
        commit()
    monkeypatch.setattr(sse, "get_adapter", lambda _: Adapter())

    async def follow(self, min_ts=0, emit_confirmed=False):
        if reconnect:
            assert self.load()[0]["confirmed"] is True
        await asyncio.Event().wait()
        yield

    monkeypatch.setattr(pqueue.PromptQueue, "follow", follow)
    stream = sse.merged_events("codex-queue", str(path), provider="codex", start_offset=0)
    try:
        async with asyncio.timeout(3):
            async for event in stream:
                if event["event"] == "message":
                    assert json.loads(event["data"])["id"] == "real"
                    break
    finally:
        await stream.aclose()
    rows = queue.load()
    assert rows[0]["confirmed"] is True
    assert rows[1]["delivered"] is False and not rows[1].get("confirmed")
    assert rows[2]["delivered"] is True and not rows[2].get("confirmed")
    assert not any(row.get("attempts") or row.get("desistiu") for row in rows)


def test_unreadable_rollout_preserves_queue(tmp_path, monkeypatch):
    monkeypatch.setattr(pqueue, "_queue_dir", lambda: tmp_path)
    queue = pqueue.PromptQueue("codex-queue")
    queue.append("entregue", delivered=True)
    before = queue.path.read_bytes()
    sse._confirm_codex_queue("codex-queue", str(tmp_path / "missing.jsonl"))
    assert queue.path.read_bytes() == before
