import asyncio

import pytest

from app.adapters.codex import adapter as codex_adapter
from app.adapters.codex.adapter import CodexAdapter


@pytest.fixture
async def codex(monkeypatch, tmp_path):
    from app import pqueue
    monkeypatch.setattr(pqueue, "_queue_dir", lambda: tmp_path)
    queue = asyncio.Queue()
    calls = []

    class Client:
        closed = False
        async def notifications(self):
            while (event := await queue.get()) is not None:
                yield event
        async def request(self, method, params=None, **kwargs):
            calls.append((method, params))
            return {}

    client = Client()
    adapter = CodexAdapter()
    adapter.attach("cx", client, "thread", subscribed=True)
    adapter._sessions["cx"].update(state="working", in_progress=True, turn_id="turn")
    stream = adapter.state_monitor("cx", lambda: "thread")
    await anext(stream)
    try:
        yield adapter, queue, calls, client
    finally:
        await stream.aclose()
        await queue.put(None)
        bomba = (adapter._sessions.get("cx") or {}).get("bomba")
        if bomba is not None:
            await bomba


async def until(check, timeout=1.0):
    loop = asyncio.get_running_loop()
    end = loop.time() + timeout
    while not check():
        if loop.time() > end:
            raise AssertionError("condição não chegou")
        await asyncio.sleep(0.01)


def command(method, item_id, process):
    return {"method": method, "params": {"threadId": "thread", "turnId": "turn",
            "item": {"type": "commandExecution", "id": item_id, "processId": process}}}


async def test_stop_encerra_os_comandos_que_o_turno_interrompido_rodava(codex, monkeypatch):
    adapter, queue, calls, client = codex
    await queue.put(command("item/started", "exec-1", "43041"))
    await queue.put(command("item/started", "exec-2", "43042"))
    await queue.put(command("item/completed", "exec-2", "43042"))
    await until(lambda: adapter._sessions["cx"].get("running_commands") == {"exec-1": ("turn", "43041")})

    async def ensure_running(name):
        return client
    async def active_turn(name):
        return "turn"
    monkeypatch.setattr(adapter, "ensure_running", ensure_running)
    monkeypatch.setattr(adapter, "_active_turn_id", active_turn)
    assert await adapter.interrupt("cx")
    # A observação da sessão também lê a thread por conta própria; aqui só contam as chamadas do Stop.
    stop = [c for c in calls if c[0] in ("turn/interrupt", "thread/backgroundTerminals/terminate")]
    assert stop == [("turn/interrupt", {"threadId": "thread", "turnId": "turn"}),
                    ("thread/backgroundTerminals/terminate", {"threadId": "thread", "processId": "43041"})]


async def test_pensamento_ao_vivo_vai_para_a_fonte_e_some_com_a_resposta(codex, monkeypatch):
    adapter, queue, _, _ = codex
    pushed = []

    class Source:
        async def push(self, text):
            pushed.append(text)
    monkeypatch.setattr(codex_adapter, "fonte_pensamento", lambda name: Source())
    await queue.put({"method": "item/reasoning/summaryTextDelta", "params": {
        "threadId": "thread", "turnId": "turn", "itemId": "rs-1", "summaryIndex": 0, "delta": "**Plano**"}})
    await until(lambda: pushed == ["**Plano**"])
    await queue.put({"method": "item/started", "params": {"threadId": "thread", "turnId": "turn",
                                                          "item": {"type": "agentMessage", "id": "m1", "text": ""}}})
    await until(lambda: pushed[-1:] == [""])


async def test_app_server_morto_com_turno_no_ar_fica_marcado_para_conferir(codex):
    adapter, queue, _, client = codex
    adapter._sessions["cx"]["headless"] = True
    bomba = adapter._sessions["cx"]["bomba"]
    client.closed = True
    await queue.put(None)
    await bomba
    assert "cx" in adapter._cortados and "cx" not in adapter._sessions
