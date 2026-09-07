"""Controles do chat usam a thread viva e entradas estruturadas do Codex."""
import pytest
import asyncio
import shutil

from app.adapters.codex.adapter import CodexAdapter
from app.adapters.codex import sessions


class Client:
    closed = False

    def __init__(self):
        self.calls = []
        self.fail = None

    async def request(self, method, params):
        self.calls.append((method, params))
        if self.fail == method:
            raise RuntimeError("turno encerrado")
        if method == "thread/read":
            return {"thread": {"model": "gpt-6-astra", "reasoningEffort": "high"}}
        if method == "skills/list":
            return {"data": [{"skills": [
                {"name": "revisar", "path": "/skills/revisar/SKILL.md", "enabled": True},
                {"name": "oculta", "path": "/skills/oculta/SKILL.md", "enabled": False},
            ]}]}
        return {}


@pytest.fixture
def chat(tmp_path, monkeypatch):
    monkeypatch.setattr(sessions, "_dir", lambda: tmp_path / "sidecars")
    client = Client()
    adapter = CodexAdapter()
    adapter.attach("sess", client, "thread-1", model="antigo", effort="medium", subscribed=True)
    return adapter, client


async def test_leitura_viva_vence_esforco_antigo(chat):
    adapter, _ = chat
    assert await adapter.read_settings("sess") == {
        "model": "gpt-6-astra", "effort": "high", "mode": "default",
    }


async def test_plan_preserva_modelo_esforco_e_permissoes(chat):
    adapter, client = chat
    await adapter.set_mode("sess", "plan")
    method, params = client.calls[-1]
    assert method == "thread/settings/update"
    assert params == {"threadId": "thread-1", "collaborationMode": {
        "mode": "plan", "settings": {"model": "gpt-6-astra", "reasoning_effort": "high",
                                     "developer_instructions": None}}}


async def test_skill_usa_identidade_nativa_e_mantem_texto_para_historico(chat):
    adapter, client = chat
    await adapter.send_prompt("sess", "/revisar confira o diff")
    method, params = client.calls[-1]
    assert method == "turn/start"
    assert params["input"] == [{"type": "text", "text": "/revisar confira o diff"},
                               {"type": "skill", "name": "revisar", "path": "/skills/revisar/SKILL.md"}]
    assert "model" not in params and "effort" not in params
    assert [s["name"] for s in await adapter.list_skills("sess")] == ["revisar"]


async def test_orientar_exige_turno_atual_e_nao_inicia_outro(chat):
    adapter, client = chat
    adapter._sessions["sess"].update(turn_id="turno-1", in_progress=True)
    await adapter.steer("sess", "corrija isso")
    assert client.calls[-1] == ("turn/steer", {
        "threadId": "thread-1", "expectedTurnId": "turno-1",
        "input": [{"type": "text", "text": "corrija isso"}],
    })
    client.fail = "turn/steer"
    with pytest.raises(RuntimeError):
        await adapter.steer("sess", "outra orientação")
    assert all(method != "turn/start" for method, _ in client.calls)


async def test_modelo_rejeitado_nao_altera_estado(chat):
    adapter, client = chat
    client.fail = "thread/settings/update"
    with pytest.raises(RuntimeError):
        await adapter.set_model("sess", "outro", "low")
    assert adapter.current_model("sess")["model"] == "antigo"


async def test_notificacao_terminal_atualiza_modo_e_esforco(chat):
    adapter, client = chat
    async def notifications():
        yield {"method": "thread/settings/updated", "params": {
            "threadId": "thread-1", "threadSettings": {"model": "gpt-5.6-sol", "effort": "xhigh",
                                                       "collaborationMode": {"mode": "plan"}}}}
    client.notifications = notifications
    events = []
    await adapter._consumir("sess", client, adapter._sessions["sess"], events.append)
    assert events[-1].codex_mode == "plan"
    assert "xhigh" in events[-1].status_line
    assert adapter.current_model("sess") == {"model": "gpt-5.6-sol", "effort": "xhigh"}


async def test_falha_orientacao_restaura_fila(chat, monkeypatch, tmp_path):
    from app import pqueue
    monkeypatch.setattr(pqueue, "_queue_dir", lambda: tmp_path)
    adapter, client = chat
    adapter._sessions["sess"].update(turn_id="turno-1", in_progress=True)
    client.fail = "turn/steer"
    queue = pqueue.PromptQueue("sess")
    queue.append("mensagem pendente")
    with pytest.raises(RuntimeError):
        await adapter.steer_queue("sess")
    assert queue.load()[0]["delivered"] is False


@pytest.mark.skipif(not shutil.which("codex"), reason="Codex CLI necessário para validar o protocolo")
async def test_controles_no_codex_real_sem_inferencia(tmp_path, monkeypatch):
    from app.adapters.codex.appserver import AppServerClient
    (tmp_path / ".codex").mkdir()
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("CODEX_HOME", str(tmp_path / ".codex"))
    monkeypatch.setattr(sessions, "_dir", lambda: tmp_path / "sidecars")
    client = AppServerClient()
    adapter = CodexAdapter()
    try:
        await client.start_shared()
        await client.request("initialize", {"clientInfo": {"name": "hangar-test", "version": "1"},
                                            "capabilities": {"experimentalApi": True}})
        result = await client.request("thread/start", {"cwd": str(tmp_path), "model": "gpt-6-astra", "ephemeral": True})
        adapter.attach("native", client, result["thread"]["id"], subscribed=True)
        await adapter.set_model("native", "gpt-6-astra", "high")
        assert (await adapter.read_settings("native"))["effort"] == "high"
        await adapter.set_mode("native", "plan")
        async with asyncio.timeout(10):
            async for notification in client.notifications():
                if notification.get("method") == "thread/settings/updated":
                    settings = notification["params"]["threadSettings"]
                    if settings["collaborationMode"]["mode"] == "plan":
                        assert settings["effort"] == "high"
                        assert settings["sandboxPolicy"]["type"] == "readOnly"
                        break
        await adapter.set_mode("native", "default")
        with pytest.raises(RuntimeError):
            await adapter.steer("native", "sem turno")
    finally:
        await client.close()
