"""Contexto da sessão Claude lido do transcript, para quem não usa a statusline do Hangar."""
import json

from app import claude_context as cc


def _resposta(model="claude-opus-5-5", entrada=2, lido=0, escrito=0, lateral=False, tipo="assistant"):
    return {"type": tipo, "isSidechain": lateral, "message": {"model": model, "usage": {
        "input_tokens": entrada, "cache_read_input_tokens": lido, "cache_creation_input_tokens": escrito,
        "output_tokens": 50}}}


def _transcript(tmp_path, *linhas):
    p = tmp_path / "s.jsonl"
    p.write_text("".join(json.dumps(l) + "\n" for l in linhas) + '{"truncad', encoding="utf-8")
    return p


def test_usa_a_ultima_resposta_do_agente_principal(tmp_path):
    p = _transcript(tmp_path,
                    _resposta(lido=10_000),
                    _resposta(entrada=3, lido=90_000, escrito=2_000),
                    # Subagente e resposta sintética não são o contexto da conversa.
                    _resposta(lido=5, lateral=True),
                    _resposta(model="<synthetic>", entrada=0))
    assert cc.from_transcript(p, tmp_path) == {"used": 92_003, "window": 200_000}


def test_janela_de_1m_pelo_modelo_configurado_ou_pelo_uso(tmp_path):
    p = _transcript(tmp_path, _resposta(lido=90_000))
    (tmp_path / "settings.json").write_text(json.dumps({"model": "opus[1m]"}), encoding="utf-8")
    assert cc.from_transcript(p, tmp_path)["window"] == 1_000_000
    # Uso acima da janela padrão só cabe na de 1M, mesmo sem a configuração dizer.
    assert cc.window(250_000, tmp_path / "sem-config") == 1_000_000
    assert cc.window(150_000, tmp_path / "sem-config") == 200_000


def test_sem_resposta_ou_sem_arquivo_e_none(tmp_path):
    assert cc.from_transcript(_transcript(tmp_path, {"type": "user", "message": {"content": "oi"}}), tmp_path) is None
    assert cc.from_transcript(tmp_path / "nao-existe.jsonl", tmp_path) is None
    assert cc.from_transcript(None) is None


def test_modelo_da_sessao_vence_o_da_conta(tmp_path):
    # O Hangar abre a sessão com `--model opus[1m]` e não mexe no settings.json da conta.
    (tmp_path / "settings.json").write_text(json.dumps({"model": "sonnet"}), encoding="utf-8")
    assert cc.window(100_000, tmp_path, model="opus[1m]") == 1_000_000
    (tmp_path / "settings.json").write_text(json.dumps({"model": "opus[1m]"}), encoding="utf-8")
    assert cc.window(100_000, tmp_path, model="sonnet") == 200_000
    # Janela declarada pelo motor (CLAUDE_CODE_MAX_CONTEXT_TOKENS) vence os dois.
    assert cc.window(100_000, tmp_path, model="opus[1m]", window_tokens=256_000) == 256_000


def test_registry_le_o_modelo_do_processo_e_do_sidecar(tmp_path, monkeypatch):
    from app import registry
    from app.models import SessionInfo
    p = _transcript(tmp_path, _resposta(lido=100_000))
    monkeypatch.setattr(registry, "_escolhas_status", lambda _sid: (None, None))
    monkeypatch.setattr(registry.procinfo, "_model_of", lambda _pid: ("opus[1m]", None))
    monkeypatch.setattr(registry.procinfo, "_env_var_of", lambda _pid, _nome: None)
    info = SessionInfo(name="s", jsonl=str(p), conta=f"claude:{tmp_path}")
    assert registry._claude_context(info, 4242) == {"used": 100_002, "window": 1_000_000}
    monkeypatch.setattr(registry.headless_sessions, "load",
                        lambda _n: {"model": "opus", "context_window": 400_000})
    sem_terminal = SessionInfo(name="s", jsonl=str(p), headless=True, conta=f"claude:{tmp_path}")
    assert registry._claude_context(sem_terminal, None)["window"] == 400_000


async def test_clear_zera_o_contexto_ate_a_primeira_resposta(tmp_path, monkeypatch):
    import time
    from app import registry
    from app.models import SessionInfo
    from app.registry import SessionRegistry
    antes = _transcript(tmp_path, _resposta(lido=90_000))
    depois = tmp_path / "novo.jsonl"
    depois.write_text(json.dumps({"type": "user", "message": {"content": "oi"}}) + "\n", encoding="utf-8")
    reg = SessionRegistry(projects_dir=tmp_path)
    monkeypatch.setattr(SessionRegistry, "_context_cache", {})
    monkeypatch.setattr(SessionRegistry, "_status_cache", {"s": (time.monotonic(), None)})
    monkeypatch.setattr(registry, "_escolhas_status", lambda _sid: (None, None))
    monkeypatch.setattr(registry.hook_state, "get_state", lambda _sid: ("idle", 1.0))
    monkeypatch.setattr(registry, "pergunta_aberta", lambda _sid: None)
    info = SessionInfo(name="s", jsonl=str(antes), tracked=True, conta=f"claude:{tmp_path}")
    monkeypatch.setattr(reg, "list", lambda: [info])
    assert (await reg.list_with_state())[0].context == {"used": 90_002, "window": 200_000}
    # /clear: transcript novo, ainda sem resposta. O número da conversa anterior não vale mais.
    info.jsonl, info.context = str(depois), None
    assert (await reg.list_with_state())[0].context is None
