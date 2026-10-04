"""Codex sem terminal: app-server (falso, em stdio) atrás do cano real, sem pane."""
import asyncio
import json
import os
import sys
import time
from pathlib import Path
from unittest.mock import patch

import pytest

from app import tmux
from app import api as api_mod
from app.adapters.codex import adapter as codex_adapter
from app.adapters.codex import sem_terminal
from app.adapters.codex import sessions as codex_sessions
from app.adapters.codex.adapter import CodexAdapter

pytestmark = pytest.mark.skipif(os.name == "nt", reason="socket unix")

# `codex app-server --stdio` de mentira: initialize (uma vez), thread/start, turn/start que pede
# aprovação de comando, thread/read.
_CODEX_FALSO = r'''#!/usr/bin/env python3
import json, sys
with open("server-starts.jsonl", "a") as recorded:
    recorded.write(json.dumps(sys.argv[1:]) + "\n")
def out(o):
    sys.stdout.write(json.dumps(o) + "\n"); sys.stdout.flush()
iniciado = False
status = "idle"
effort = None
for linha in sys.stdin:
    ev = json.loads(linha)
    m = ev.get("method")
    if m == "initialize":
        if iniciado:
            out({"jsonrpc": "2.0", "id": ev["id"], "error": {"code": -32600, "message": "Already initialized"}})
        else:
            iniciado = True
            out({"jsonrpc": "2.0", "id": ev["id"], "result": {"userAgent": "falso"}})
    elif m == "thread/start":
        out({"jsonrpc": "2.0", "id": ev["id"], "result": {"thread": {"id": "th-1", "path": ""}, "model": "gpt-falso"}})
    elif m == "thread/resume":
        with open("resume.txt", "w") as f:
            f.write(json.dumps(ev["params"]))
        with open("resume-history.jsonl", "a") as f:
            f.write(json.dumps(ev["params"]) + "\n")
        out({"jsonrpc": "2.0", "id": ev["id"], "result": {"thread": {"id": ev["params"]["threadId"]}, "model": "gpt-falso"}})
    elif m == "thread/settings/update":
        # O modo colaborativo também carrega o esforço; ajuste sem o campo não o apaga.
        mode = (ev["params"].get("collaborationMode") or {}).get("settings") or {}
        effort = ev["params"].get("effort", mode.get("reasoning_effort", effort))
        with open("settings.txt", "w") as f:
            f.write(json.dumps(ev["params"]))
        if ev["params"].get("effort") == "recusado":
            out({"jsonrpc": "2.0", "id": ev["id"], "error": {"code": -32602, "message": "effort invalido"}})
        else:
            out({"jsonrpc": "2.0", "id": ev["id"], "result": {}})
    elif m == "thread/read":
        out({"jsonrpc": "2.0", "id": ev["id"], "result": {"thread": {"id": "th-1", "model": "gpt-falso", "reasoningEffort": effort, "status": {"type": status}, "turns": []}}})
    elif m == "turn/start":
        status = "active"
        with open("turno.txt", "w") as f:
            f.write(json.dumps(ev["params"]))
        out({"jsonrpc": "2.0", "id": ev["id"], "result": {"turn": {"id": "t-1"}}})
        out({"jsonrpc": "2.0", "method": "turn/started", "params": {"threadId": "th-1", "turn": {"id": "t-1"}}})
        out({"jsonrpc": "2.0", "id": 0, "method": "item/commandExecution/requestApproval",
             "params": {"threadId": "th-1", "turnId": "t-1", "itemId": "exec-1", "command": "touch x",
                        "cwd": "/tmp", "reason": "fora do sandbox"}})
    elif m is None and ev.get("id") == 0:
        with open("decisao.txt", "w") as f:
            f.write(json.dumps(ev))
        out({"jsonrpc": "2.0", "method": "serverRequest/resolved", "params": {"threadId": "th-1", "requestId": 0}})
        out({"jsonrpc": "2.0", "id": 9, "method": "mcpServer/elicitation/request",
             "params": {"threadId": "th-1", "serverName": "x", "message": "?"}})
    elif m is None and ev.get("id") == 9:
        with open("elicitacao.txt", "w") as f:
            f.write(json.dumps(ev))
        status = "idle"
        out({"jsonrpc": "2.0", "method": "turn/completed", "params": {"threadId": "th-1", "turn": {"id": "t-1", "status": "completed"}}})
    else:
        out({"jsonrpc": "2.0", "id": ev.get("id"), "result": {}})
'''


@pytest.fixture
def ambiente(tmp_path, monkeypatch):
    binario = tmp_path / "bin"
    binario.mkdir()
    fake = binario / "codex"
    fake.write_text(_CODEX_FALSO)
    fake.chmod(0o755)
    monkeypatch.setenv("PATH", f"{binario}{os.pathsep}{os.environ['PATH']}")
    pasta = tmp_path / "codex-sessions"
    fila = tmp_path / "fila"
    fila.mkdir()
    from app import pqueue
    with patch.object(codex_sessions, "_dir", lambda: pasta), \
         patch.object(pqueue, "_queue_dir", lambda: fila), \
         patch.object(tmux, "_scope_prefix", lambda: []):
        yield tmp_path


def _sidecar(nome: str, cwd: Path, **escolha) -> dict:
    codex_sessions.save(nome, None, "", str(cwd), headless=True, key=sem_terminal.nova_chave(),
                        permission_mode="Ask for approval", **escolha)
    return codex_sessions.load(nome)


def test_modo_padrao_e_full_access():
    assert sem_terminal.MODO_PADRAO == "Full Access"
    assert sem_terminal.politica(None) == ("never", "danger-full-access")
    assert sem_terminal.modos_para_tela(None)["current"] == "Full Access"


def test_esforco_escolhido_chega_na_thread(ambiente):
    """`thread/start` leva o modelo mas não tem campo de esforço; sem terminal ninguém mais aplica."""
    async def corpo():
        ad = CodexAdapter()
        _sidecar("cx-esforco", ambiente, model="gpt-6-astra", effort="high")
        assert await ad.ensure_running("cx-esforco") is not None
        ajuste = json.loads((ambiente / "settings.txt").read_text())
        assert ajuste == {"threadId": "th-1", "model": "gpt-6-astra", "effort": "high"}
        await asyncio.to_thread(ad.close_sync, "cx-esforco")
    asyncio.run(corpo())


def test_esforco_recusado_deixa_a_sessao_de_pe_e_o_problema_visivel(ambiente):
    """Nível que o servidor recusa não pode derrubar uma thread que já abriu — mas tem que aparecer."""
    async def corpo():
        ad = CodexAdapter()
        _sidecar("cx-esforco-ruim", ambiente, model="gpt-6-astra", effort="recusado")
        assert await ad.ensure_running("cx-esforco-ruim") is not None
        assert codex_sessions.load("cx-esforco-ruim")["thread_id"] == "th-1"
        assert ad.problema_de("cx-esforco-ruim") == "codex_esforco_nao_aplicado"
        assert "effort invalido" in ad._problemas["cx-esforco-ruim"][1]
        await asyncio.to_thread(ad.close_sync, "cx-esforco-ruim")
    asyncio.run(corpo())


def test_ambiente_identifica_a_sessao_headless_para_os_scripts(ambiente, monkeypatch):
    meta = _sidecar("cx-identidade", ambiente)
    monkeypatch.setenv("TMUX", "herdado")
    monkeypatch.setenv("TMUX_PANE", "%9")

    env = sem_terminal._ambiente(meta)

    assert env["CP_SESSION_NAME"] == "cx-identidade"
    assert env["CP_SESSION_KEY"] == meta["key"]
    assert env["HANGAR_CANO_KEY"] == meta["key"]
    assert env["HANGAR_CANO_OWNER"] == str(Path.home())
    assert "TMUX" not in env and "TMUX_PANE" not in env


def test_api_le_permissao_headless_durante_o_turno(ambiente, monkeypatch):
    _sidecar("cx-permissao", ambiente)

    class Adapter:
        async def deliverable(self, _name):
            return False

        def permission_modes_sem_terminal(self, _name):
            return sem_terminal.modos_para_tela("Full Access")

    adapter = Adapter()
    monkeypatch.setattr(api_mod, "_provider_of", lambda _name: "codex")
    monkeypatch.setattr(api_mod, "_codex_sem_terminal", lambda _name: True)
    monkeypatch.setattr(api_mod, "get_adapter", lambda _provider: adapter)

    result = asyncio.run(api_mod.permissoes_do_codex("cx-permissao"))

    assert result["current"] == "Full Access"


def test_api_explica_por_que_nao_troca_permissao_headless_durante_o_turno(ambiente, monkeypatch):
    _sidecar("cx-permissao", ambiente)

    class Adapter:
        async def deliverable(self, _name):
            return False

        async def set_permission_mode_sem_terminal(self, _name, _mode):
            raise sem_terminal.Ocupada(
                "a sessão está trabalhando; mudar o sandbox reiniciaria o Codex — espere ela terminar")

    adapter = Adapter()
    monkeypatch.setattr(api_mod, "_provider_of", lambda _name: "codex")
    monkeypatch.setattr(api_mod, "_codex_sem_terminal", lambda _name: True)
    monkeypatch.setattr(api_mod, "get_adapter", lambda _provider: adapter)

    with pytest.raises(api_mod.HTTPException) as exc:
        asyncio.run(api_mod.trocar_permissao_do_codex(
            "cx-permissao", api_mod.CodexPermissionBody(mode="Full Access")))

    assert exc.value.status_code == 409
    assert "sandbox reiniciaria o Codex" in str(exc.value.detail)


def test_sobe_no_cano_abre_thread_e_religa_com_aprovacao_pendente(ambiente):
    async def corpo():
        ad = CodexAdapter()
        meta = _sidecar("cx-sem-terminal", ambiente)
        client = await ad.ensure_running("cx-sem-terminal")
        assert client is not None
        meta = codex_sessions.load("cx-sem-terminal")
        assert meta["thread_id"] == "th-1" and meta["cano"]["pid"]
        assert ad._sessions["cx-sem-terminal"]["headless"] is True
        assert "cx-sem-terminal" not in ad._tmux_watchers
        # Turno que pede aprovação: o pedido fica em server_requests.
        assert await ad.send_prompt("cx-sem-terminal", "toca x") == "sent"
        for _ in range(50):
            if 0 in client.server_requests:
                break
            await asyncio.sleep(0.05)
        assert client.server_requests[0]["method"] == "item/commandExecution/requestApproval"
        # "Backend caiu": só a conexão morre; o cano e o app-server ficam.
        pid_cano = meta["cano"]["pid"]
        await client.close()
        ad._sessions.pop("cx-sem-terminal", None)
        await asyncio.sleep(0.2)
        assert Path(f"/proc/{pid_cano}").exists()
        client2 = await ad.ensure_running("cx-sem-terminal")
        assert client2 is not None and client2 is not client
        assert client2.server_requests[0]["params"]["itemId"] == "exec-1"   # veio do snapshot
        assert codex_sessions.load("cx-sem-terminal")["cano"]["pid"] == pid_cano   # mesmo cano
        # O pedido é o cartão do app; responder pelo /select fecha o turno.
        sess = ad._sessions["cx-sem-terminal"]
        ev = ad._question_state("cx-sem-terminal", sess)
        assert ev.state == "awaiting_input" and ev.headless is True
        assert ev.question.startswith("Rodar `touch x` em /tmp?") and ev.options == ["Permitir", "Negar", "Sempre permitir"]
        assert ad.aprovacao_pendente("cx-sem-terminal")[0] == ev.question
        assert await ad.select("cx-sem-terminal", 1) is True
        for _ in range(100):
            if (ambiente / "elicitacao.txt").exists() and not sess.get("in_progress"):
                break
            await asyncio.sleep(0.05)
        assert not sess.get("in_progress")
        assert json.loads((ambiente / "decisao.txt").read_text())["result"] == {"decision": "accept"}
        # O pedido que a sessão sem terminal não atende foi recusado com -32601 e virou nota no chat.
        recusa = json.loads((ambiente / "elicitacao.txt").read_text())
        assert recusa["error"]["code"] == -32601
        from app.pqueue import PromptQueue
        notas = [e for e in PromptQueue("cx-sem-terminal").load() if e.get("papel") == "assistant"]
        assert notas and "mcpServer/elicitation/request" in notas[-1]["text"]
        assert await ad.select("cx-sem-terminal", 1) is False    # nada mais pendente
        await asyncio.to_thread(ad.close_sync, "cx-sem-terminal")
        for _ in range(50):
            if not Path(f"/proc/{pid_cano}").exists():
                break
            await asyncio.sleep(0.05)
        assert not Path(f"/proc/{pid_cano}").exists()
    asyncio.run(corpo())


def test_modo_de_permissao_vai_no_turno_e_troca_de_sandbox_reabre_o_servidor(ambiente):
    async def corpo():
        ad = CodexAdapter()
        _sidecar("cx-modo", ambiente)
        await ad.ensure_running("cx-modo")
        pid1 = codex_sessions.load("cx-modo")["cano"]["pid"]
        assert ad.permission_modes_sem_terminal("cx-modo")["current"] == "Ask for approval"
        await ad.send_prompt("cx-modo", "oi")
        for _ in range(50):
            if (ambiente / "turno.txt").exists():
                break
            await asyncio.sleep(0.05)
        assert json.loads((ambiente / "turno.txt").read_text())["approvalPolicy"] == "on-request"
        for _ in range(50):
            if ad.aprovacao_pendente("cx-modo")[0]:
                break
            await asyncio.sleep(0.05)
        # Com o turno aberto, trocar o sandbox é recusado (derrubaria a conexão no meio).
        with pytest.raises(sem_terminal.Ocupada):
            await ad.set_permission_mode_sem_terminal("cx-modo", "Full Access")
        assert await ad.select("cx-modo", 2) is True
        for _ in range(100):
            if (ambiente / "elicitacao.txt").exists() and not ad._sessions["cx-modo"].get("in_progress"):
                break
            await asyncio.sleep(0.05)
        client = ad._sessions["cx-modo"]["client"]
        await client.close()
        sess = ad._sessions.pop("cx-modo", None)
        if sess is not None:
            sess["bomba"].cancel()
        assert (await ad.set_permission_mode_sem_terminal("cx-modo", "full access"))["current"] == "Full Access"
        meta = codex_sessions.load("cx-modo")
        assert meta["permission_mode"] == "Full Access" and meta["cano"]["pid"] != pid1
        assert not Path(f"/proc/{pid1}").exists()
        resume = json.loads((ambiente / "resume.txt").read_text())
        assert resume == {"threadId": "th-1", "cwd": str(ambiente), "approvalPolicy": "never",
                          "sandbox": "danger-full-access"}
        with pytest.raises(ValueError):
            await ad.set_permission_mode_sem_terminal("cx-modo", "yolo")
        await asyncio.to_thread(ad.close_sync, "cx-modo")
    asyncio.run(corpo())


@pytest.mark.parametrize("estado", ["active", "active_sem_aprovacao", "desconhecido", "desconhecido_anexado", "erro"])
def test_troca_de_sandbox_apos_restart_preserva_turno_vivo(ambiente, estado):
    sem_aprovacao = estado in {"active_sem_aprovacao", "desconhecido_anexado"}
    if estado != "active":
        fake = ambiente / "bin" / "codex"
        codigo = fake.read_text()
        if sem_aprovacao:
            codigo = codigo.replace(
                '"id": 0, "method": "item/commandExecution/requestApproval"',
                '"method": "teste/turnoAtivo"')
        if estado == "erro":
            codigo = codigo.replace(
                'elif m == "thread/read":',
                'elif m == "thread/read":\n'
                '        out({"jsonrpc": "2.0", "id": ev["id"], "error": {"code": -32603, "message": "indisponível"}})\n'
                '        continue')
        elif estado.startswith("desconhecido"):
            codigo = codigo.replace('"type": status', '"type": "unknown"')
        fake.write_text(codigo)

    async def corpo():
        ad = CodexAdapter()
        _sidecar("cx-reinicio", ambiente)
        client = await ad.ensure_running("cx-reinicio")
        pid = codex_sessions.load("cx-reinicio")["cano"]["pid"]
        try:
            assert await ad.send_prompt("cx-reinicio", "oi") == "sent"
            for _ in range(50):
                if sem_aprovacao or 0 in client.server_requests:
                    break
                await asyncio.sleep(0.05)
            assert (0 in client.server_requests) == (not sem_aprovacao)
            await client.close()
            sess = ad._sessions.pop("cx-reinicio", None)
            if sess is not None:
                sess["bomba"].cancel()
            if estado == "desconhecido_anexado":
                assert await ad.ensure_running("cx-reinicio") is not None
            with pytest.raises(sem_terminal.Ocupada if estado.startswith("active") else RuntimeError):
                await ad.set_permission_mode_sem_terminal("cx-reinicio", "Full Access")
            meta = codex_sessions.load("cx-reinicio")
            assert meta["cano"]["pid"] == pid
            assert Path(f"/proc/{pid}").exists()
            assert meta["permission_mode"] == "Ask for approval"
            assert not (ambiente / "resume.txt").exists()
            if estado == "active":
                assert await ad.select("cx-reinicio", 1) is True
                for _ in range(50):
                    if (ambiente / "elicitacao.txt").exists():
                        break
                    await asyncio.sleep(0.05)
                assert (ambiente / "elicitacao.txt").exists()
        finally:
            await asyncio.to_thread(ad.close_sync, "cx-reinicio")
    asyncio.run(corpo())


def test_binario_ausente_para_no_teto_de_subidas(ambiente, monkeypatch):
    monkeypatch.setenv("PATH", str(ambiente / "vazio"))

    async def corpo():
        ad = CodexAdapter()
        _sidecar("cx-sem-codex", ambiente)
        for _ in range(CodexAdapter.TETO_SUBIDAS):
            with pytest.raises(RuntimeError):
                await ad.ensure_running("cx-sem-codex")
        assert await ad.ensure_running("cx-sem-codex") is None   # desistiu, sem levantar de novo
        # O motivo não some: a lista e o chat mostram por que a sessão está morta.
        assert ad.problema_de("cx-sem-codex") == "codex_headless_nao_subiu"
        ev = [e async for e in ad._state_stream("cx-sem-codex")]
        assert ev[-1].state == "dead" and ev[-1].problema == "codex_headless_nao_subiu"
        assert "codex" in (ev[-1].problema_detalhe or "")
        await asyncio.to_thread(ad.close_sync, "cx-sem-codex")
        assert ad.problema_de("cx-sem-codex") is None
    asyncio.run(corpo())


def test_politica_por_modo():
    assert sem_terminal.politica("Ask for approval") == ("on-request", "read-only")
    assert sem_terminal.politica("Full Access") == ("never", "danger-full-access")
    assert sem_terminal.politica(None) == ("never", "danger-full-access")
    assert sem_terminal.politica("qualquer coisa") == sem_terminal.politica(sem_terminal.MODO_PADRAO)
    assert '-c' in sem_terminal.argv({"permission_mode": "Full Access"})
    assert 'approval_policy="never"' in sem_terminal.argv({"permission_mode": "Full Access"})



def test_transferred_stdio_budget_survives_restart_and_resume(ambiente, monkeypatch):
    import hashlib
    import uuid
    from app import conversation_transfer as transfers
    monkeypatch.setattr(transfers, "_base", lambda: ambiente / "transfers")
    transfer_id = str(uuid.uuid4())
    thread_id = str(uuid.uuid4())
    key = sem_terminal.nova_chave()
    (ambiente / "bin" / "codex").write_text(_CODEX_FALSO.replace('"th-1"', json.dumps(thread_id)))
    home = ambiente / "account"
    from app import codex_contas
    account = codex_contas.Account("test-transfer", home, False)
    monkeypatch.setattr(codex_contas, "resolve_account", lambda name: account)
    rollout = home / "sessions" / f"rollout-test-{thread_id}.jsonl"
    rollout.parent.mkdir(parents=True)
    prefix = (json.dumps({"type": "session_meta", "payload": {"id": thread_id}}) + "\n").encode()
    rollout.write_bytes(prefix)
    source = ambiente / "source.jsonl"
    source.write_text(json.dumps({"type": "user", "uuid": "u", "parentUuid": None,
                                  "message": {"role": "user", "content": "histórico"}}) + "\n")
    record = transfers.TransferRecord(transfer_id, "imported", f"k:{key}", transfers.TransferPhase.COMPLETE,
        transfers.ConversationSource(str(source), "claude", hashlib.sha256(source.read_bytes()).hexdigest(), ("u",)),
        {"name": "imported", "key": key, "cwd": str(ambiente)},
        {"codex_home": str(home), "codex_account": account.id, "thread_id": thread_id, "rollout_path": str(rollout),
         "tool_output_token_limit": 144000},
        transfers.ImportBoundary(thread_id, str(rollout), len(prefix), (), hashlib.sha256(prefix).hexdigest()), None)
    transfers.save_transfer(record)
    async def body():
        adapter = CodexAdapter()
        _sidecar("imported", ambiente, model="gpt-falso", effort="high",
                 transfer_id=transfer_id, codex_home=str(home), codex_account=account.id, tool_output_token_limit=144000)
        codex_sessions.update("imported", thread_id=thread_id, rollout_path=str(rollout), key=key)
        try:
            assert await adapter.ensure_running("imported") is not None
            await adapter.restart("imported")
            starts = [json.loads(line) for line in (ambiente / "server-starts.jsonl").read_text().splitlines()]
            resumes = [json.loads(line) for line in (ambiente / "resume-history.jsonl").read_text().splitlines()]
            assert len(starts) == len(resumes) == 2
            assert all("tool_output_token_limit=144000" in args for args in starts)
            assert all(params["threadId"] == thread_id for params in resumes)
            meta = codex_sessions.load("imported")
            assert meta["tool_output_token_limit"] == 144000 and meta["transfer_id"] == transfer_id
            assert transfers.transfer_for_session("imported").id == transfer_id
            assert transfers.load_transfer(transfer_id).phase == transfers.TransferPhase.COMPLETE
            assert not (ambiente / "turno.txt").exists()
        finally:
            # Fora do laço, como nos outros testes: o encerramento espera o cano sair de fato, e o
            # laço preso aqui não recolheria o processo filho.
            await asyncio.to_thread(adapter.close_sync, "imported")
    asyncio.run(body())


def test_stdio_budget_is_optional_and_validated():
    argv = sem_terminal.argv({"permission_mode": "Full Access", "tool_output_token_limit": 144000})
    assert argv[:3] == ["codex", "app-server", "--stdio"]
    assert "tool_output_token_limit=144000" in argv
    assert not any("tool_output_token_limit" in arg for arg in sem_terminal.argv({}))
    with pytest.raises(ValueError):
        sem_terminal.argv({"tool_output_token_limit": True})
