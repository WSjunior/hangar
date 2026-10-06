"""Criar/retomar sessão com motor.

O motor é aplicado prefixando o comando com `hangar-engine --exec`, não com `tmux -e`: assim a key não
aparece em /proc/<pid>/cmdline (legível por qualquer usuário) e o tmux.py não muda.
E o motor tem que sobreviver aos DOIS resumes — senão uma sessão Kimi ressuscita na conta Anthropic
continuando um transcript de Kimi, calado.
"""
import asyncio
import os

import pytest

from app import engines as eng
from app import procinfo
from app import registry as reg


# Capturado no IMPORT, antes de qualquer fixture: o `_isola` abaixo troca `reg._exigir_cp_engine`
# por um no-op, entao la dentro nao ha mais como alcancar a funcao de verdade.
_EXIGIR_ORIGINAL = reg._exigir_cp_engine


@pytest.fixture(autouse=True)
def _isola(tmp_path, monkeypatch):
    monkeypatch.setattr(eng, "caminho", lambda: tmp_path / "engines.json")
    # A guarda que recusa quando o `hangar-engine` nao esta no PATH e sobre o AMBIENTE do servidor, nao
    # sobre a montagem do comando, que e o assunto deste arquivo. Sem desliga-la aqui, todos estes
    # casos passariam a depender de o lancador estar instalado na maquina que roda a suite — verde
    # no Linux (onde o install-claude-wrapper.sh o poe no PATH) e vermelho no Windows, pelo
    # ambiente e nao pelo codigo. A guarda tem teste proprio, logo abaixo.
    monkeypatch.setattr(reg, "_exigir_cp_engine", lambda: None)
    yield


def test_recusa_alto_quando_o_cp_engine_nao_esta_no_path(monkeypatch):
    """A guarda em si — o unico caso que NAO desliga o `_exigir_cp_engine`.

    Sem ela o pane nasce rodando um comando que nao existe, morre no ato, e o `tmux new-session`
    devolve 0 assim mesmo (medido no psmux: rc=0 e, 3s depois, `has-session` ja responde 1). O app
    entao reporta "sessao criada" pra uma sessao que evaporou.
    """
    monkeypatch.setattr(reg.shutil, "which", lambda nome: None)
    with pytest.raises(ValueError, match="hangar-engine"):
        _EXIGIR_ORIGINAL()

    monkeypatch.setattr(reg.shutil, "which", lambda nome: "/qualquer/hangar-engine")
    _EXIGIR_ORIGINAL()           # com o lancador no PATH, passa calado


def _motor():
    eng.salvar("kimi", {"base_url": "https://api.kimi.com/coding",
                        "api_key": "sk-kimi-1234", "model": "k3"})


def _reg(tmp_path, monkeypatch, visto):
    def _fake_new(name, cwd, command, config_dir=None, *, provider="claude", env=None):
        visto["command"] = command
        return True

    monkeypatch.setattr(reg.tmux, "new_session", _fake_new)
    monkeypatch.setattr(reg.tmux, "has_session", lambda n: False)
    monkeypatch.setattr(reg, "_pretrust_cwd", lambda cwd, cfg: None)
    return reg.SessionRegistry(projects_dir=tmp_path)


def _fixed_proxy(monkeypatch):
    from app import cliproxy, cliproxy_accounts
    account = {"account": "default", "prefix": "fixed", "credential_id": "codex:/tmp/codex",
               "home": "/tmp/codex", "base_url": "http://127.0.0.1:8317"}
    monkeypatch.setattr(cliproxy, "account_for_engine", lambda cfg, name, home=None: account)
    monkeypatch.setattr(cliproxy_accounts, "resolve", lambda name, home=None: account)
    monkeypatch.setattr("app.engine_probe.listar_modelos", lambda *a: [{"id": "fixed/gpt-5.5"}])
    eng.salvar("proxy", {"base_url": "http://127.0.0.1:8317", "api_key": "test", "model": "gpt-5.5"})


def test_fixed_account_is_in_terminal_command_and_headless_sidecar(tmp_path, monkeypatch):
    from app.adapters.claude_headless import sessions
    monkeypatch.setattr(sessions, "_dir", lambda: tmp_path / "hl")
    _fixed_proxy(monkeypatch)
    seen = {}
    registry = _reg(tmp_path, monkeypatch, seen)
    info = registry.create("terminal-fixed", str(tmp_path), engine="proxy", engine_account="default",
                           model="gpt-5.5", context_window=400000)
    assert "--account default --account-home /tmp/codex --account-base-url http://127.0.0.1:8317 --model fixed/gpt-5.5 --context 400000 -- claude" in seen["command"]
    assert "--model fixed/gpt-5.5" in seen["command"]
    assert info.engine_account == "default" and info.conta == "codex:/tmp/codex"
    info = registry.create("headless-fixed", str(tmp_path), engine="proxy", engine_account="default",
                           model="gpt-5.5", headless=True)
    meta = sessions.load(info.name)
    assert meta["engine_account"] == "default" and meta["model"] == "fixed/gpt-5.5"
    assert meta["engine_credential_id"] == "codex:/tmp/codex"
    assert "--account default --account-home /tmp/codex --account-base-url http://127.0.0.1:8317 --model fixed/gpt-5.5" in registry._comando_terminal(meta, resume=True)


@pytest.mark.parametrize("tier", ["default", "priority"])
@pytest.mark.parametrize("headless", [False, True])
def test_create_claude_service_tier_transport(tmp_path, monkeypatch, tier, headless):
    from app import cliproxy
    from app.adapters.claude_headless import sessions
    monkeypatch.setattr(sessions, "_dir", lambda: tmp_path / "hl")
    _fixed_proxy(monkeypatch)
    monkeypatch.setattr(cliproxy, "supports_fast", lambda engine, model=None: engine == "proxy")
    seen = {}
    registry = _reg(tmp_path, monkeypatch, seen)
    info = registry.create("fast", str(tmp_path), engine="proxy", model="gpt-5.5",
                           service_tier=tier, headless=headless)
    if headless:
        meta = sessions.load(info.name)
        assert meta["service_tier"] == tier
        assert f"--service-tier {tier} -- claude" in registry._comando_terminal(meta, resume=True)
    else:
        assert f"--service-tier {tier} -- claude" in seen["command"]


@pytest.mark.parametrize("tier", ["default", "priority"])
def test_create_incompatible_service_tier_has_no_effects(tmp_path, monkeypatch, tier):
    from app import cliproxy
    monkeypatch.setattr(cliproxy, "supports_fast", lambda *a: False)
    monkeypatch.setattr(reg, "_pretrust_cwd", lambda *a: pytest.fail("não pode preparar conta"))
    monkeypatch.setattr(reg.tmux, "new_session", lambda *a, **k: pytest.fail("não pode abrir pane"))
    with pytest.raises(ValueError, match="service_tier"):
        reg.SessionRegistry(tmp_path).create("fast", str(tmp_path), service_tier=tier)


@pytest.mark.parametrize("tier", ["default", "priority"])
def test_rebuild_incompatible_service_tier(tmp_path, monkeypatch, tier):
    from app import cliproxy
    _motor()
    monkeypatch.setattr(cliproxy, "supports_fast", lambda *a: False)
    meta = {"session_id": "11111111-1111-1111-1111-111111111111", "engine": "kimi", "model": "k3", "service_tier": tier}
    if tier == "priority":
        with pytest.raises(ValueError, match="service_tier"):
            reg.SessionRegistry._comando_terminal(meta, resume=True)
    else:
        assert "--service-tier" not in reg.SessionRegistry._comando_terminal(meta, resume=True)


def test_terminal_proof_requires_ready_claude_identity(tmp_path, monkeypatch):
    from app import terminal_input
    sid = "11111111-1111-1111-1111-111111111111"
    registry = reg.SessionRegistry(tmp_path)
    monkeypatch.setattr(registry, "_pane_of", lambda name: {"pid": 42})
    launched = iter([("claude", None), ("claude", 43)])
    monkeypatch.setattr(reg, "agente_do_pane", lambda pid: next(launched, ("claude", 43)))
    monkeypatch.setattr(reg, "_pid_do_agente", lambda pid: 42)
    monkeypatch.setattr(reg.tmux, "has_session", lambda name: True)
    monkeypatch.setattr(reg.time, "sleep", lambda seconds: None)
    def cmdline(pid):
        assert pid == 43
        return f"claude --resume {sid}"
    monkeypatch.setattr(reg, "_cmdline", cmdline)
    monkeypatch.setattr(reg, "_config_dir_of", lambda pid: tmp_path)
    monkeypatch.setattr(reg, "_engine_of", lambda pid: "proxy")
    monkeypatch.setattr(procinfo, "pid_vivo", lambda pid: True)
    monkeypatch.setattr(procinfo, "_env_var_of", lambda pid, name: {
        "CP_ENGINE_ACCOUNT": "default", "CP_ENGINE_CREDENTIAL_ID": "codex:/tmp/codex"}.get(name))
    monkeypatch.setattr(terminal_input, "_wait_input_ready", lambda *a, **k: True)
    meta = {"session_id": sid, "config_dir": str(tmp_path), "engine": "proxy", "engine_account": "default",
            "engine_credential_id": "codex:/tmp/codex"}
    registry.wait_for_claude("fixed", meta)
    monkeypatch.setattr(terminal_input, "_wait_input_ready", lambda *a, **k: False)
    with pytest.raises(ValueError, match="terminal não ficou pronto"):
        registry.wait_for_claude("fixed", meta)
    monkeypatch.setattr(reg, "_engine_of", lambda pid: None)
    with pytest.raises(ValueError, match="identidade"):
        registry.wait_for_claude("fixed", meta)
    from app import cliproxy
    monkeypatch.setattr(cliproxy, "supports_fast", lambda *a: True)
    monkeypatch.setattr(reg, "_engine_of", lambda pid: "proxy")
    monkeypatch.setattr(terminal_input, "_wait_input_ready", lambda *a, **k: True)
    meta["service_tier"] = "priority"
    with pytest.raises(ValueError, match="identidade"):
        registry.wait_for_claude("fixed", meta)
    monkeypatch.setattr(procinfo, "_env_var_of", lambda pid, name: {
        "CP_ENGINE_ACCOUNT": "default", "CP_ENGINE_CREDENTIAL_ID": "codex:/tmp/codex",
        "CP_ENGINE_SERVICE_TIER": "priority"}.get(name))
    registry.wait_for_claude("fixed", meta)


def test_automatic_headless_resume_rejects_subagent_missing_from_catalog(tmp_path, monkeypatch):
    from unittest.mock import MagicMock
    from app.adapters.claude_headless import adapter as A, sessions
    _fixed_proxy(monkeypatch)
    eng.salvar("proxy", {**eng.listar()["proxy"], "subagent_model": "gpt-image-2"})
    monkeypatch.setattr(sessions, "_dir", lambda: tmp_path / "hl")
    meta = sessions.save("fixed", str(tmp_path), "11111111-1111-1111-1111-111111111111",
                         engine="proxy", engine_account="default", model="fixed/gpt-5.5",
                         engine_credential_id="codex:/tmp/codex", engine_account_base_url="http://127.0.0.1:8317")
    spawn = MagicMock()
    monkeypatch.setattr(A, "subir_cano_processo", spawn)
    with pytest.raises(ValueError, match="subagentes"):
        asyncio.run(A.ClaudeHeadlessAdapter()._subir_cano(A._Sessao("fixed", meta)))
    spawn.assert_not_called()


def test_proxy_init_response_does_not_replace_fixed_route(tmp_path, monkeypatch):
    from app.adapters.claude_headless.adapter import ClaudeHeadlessAdapter, _Sessao
    from app.adapters.claude_headless import sessions
    monkeypatch.setattr(sessions, "_dir", lambda: tmp_path / "hl")
    meta = sessions.save("fixed", str(tmp_path), "11111111-1111-1111-1111-111111111111",
                         engine="proxy", engine_account="default", model="fixed/gpt-5.5")
    session = _Sessao("fixed", meta)
    asyncio.run(ClaudeHeadlessAdapter()._on_system(session, {"subtype": "init", "model": "gpt-5.5"}))
    assert session.model == "fixed/gpt-5.5"
    assert sessions.load("fixed")["engine_account"] == "default"


def test_resume_fixed_account_never_drops_its_route(tmp_path, monkeypatch):
    _fixed_proxy(monkeypatch)
    seen = {}
    registry, sid = _prep_resume(tmp_path, monkeypatch, seen, "proxy")
    monkeypatch.setattr(reg, "agente_do_pane", lambda pid, children=None: ("claude", 4243))
    monkeypatch.setattr(procinfo, "_model_of", lambda pid: ("fixed/gpt-5.5", "high"))
    monkeypatch.setattr(procinfo, "_env_var_of", lambda pid, name: {
        "CP_ENGINE_ACCOUNT": "default", "CP_ENGINE_CREDENTIAL_ID": "codex:/tmp/codex",
        "CP_ENGINE_ACCOUNT_BASE_URL": "http://127.0.0.1:8317"}.get(name))
    info = registry.resume("s", sid)
    assert "--account default --account-home /tmp/codex --account-base-url http://127.0.0.1:8317 --model fixed/gpt-5.5" in seen["command"]
    assert info.engine_account == "default" and info.conta == "codex:/tmp/codex"
    eng.remover("proxy")
    seen.clear()
    with pytest.raises(ValueError, match="indisponível"):
        registry.resume("s", sid)
    assert not seen


def test_create_com_motor_prefixa_o_comando(tmp_path, monkeypatch):
    _motor()
    visto = {}
    info = _reg(tmp_path, monkeypatch, visto).create("s", str(tmp_path), engine="kimi")
    assert visto["command"].startswith("hangar-engine --exec kimi -- claude --session-id ")
    assert info.engine == "kimi"


def test_create_com_motor_nao_poe_a_key_no_comando(tmp_path, monkeypatch):
    _motor()
    visto = {}
    _reg(tmp_path, monkeypatch, visto).create("s", str(tmp_path), engine="kimi")
    assert "sk-kimi" not in visto["command"]


def test_create_sem_motor_nao_muda_o_comando(tmp_path, monkeypatch):
    visto = {}
    info = _reg(tmp_path, monkeypatch, visto).create("s", str(tmp_path))
    assert visto["command"].startswith("claude --session-id ")
    assert info.engine is None


def test_create_com_motor_inexistente_estoura(tmp_path, monkeypatch):
    visto = {}
    r = _reg(tmp_path, monkeypatch, visto)
    with pytest.raises(ValueError, match="motor"):
        r.create("s", str(tmp_path), engine="fantasma")
    assert "command" not in visto


@pytest.mark.skipif(os.name != "posix",
                    reason="exercita o ramo /proc do _engine_of contra um /proc de mentira; no "
                           "Windows o despacho vai pro psutil e nao ha o que este caso testa")
def test_engine_of_le_o_cp_engine_do_proc(tmp_path, monkeypatch):
    # Mesmo truque do _config_dir_of: o env do processo VIVO é o registro autoritativo — um sidecar
    # em disco pode divergir do que está de fato rodando no pane.
    environ = tmp_path / "environ"
    environ.write_bytes(b"PATH=/usr/bin\x00CP_ENGINE=kimi\x00HOME=/home/x\x00")
    monkeypatch.setattr(procinfo, "_proc_environ_path", lambda pid: str(environ))
    assert reg._engine_of(1234) == "kimi"


def test_engine_of_sem_a_marca_e_none(tmp_path, monkeypatch):
    environ = tmp_path / "environ"
    environ.write_bytes(b"PATH=/usr/bin\x00")
    monkeypatch.setattr(procinfo, "_proc_environ_path", lambda pid: str(environ))
    assert reg._engine_of(1234) is None


def _prep_resume(tmp_path, monkeypatch, visto, motor):
    sid = "11111111-2222-3333-4444-555555555555"
    proj = tmp_path / "projects" / "-tmp"
    proj.mkdir(parents=True)
    (proj / f"{sid}.jsonl").write_text("", encoding="utf-8")

    def _fake_new(name, cwd, command, config_dir=None, *, provider="claude", env=None):
        visto["command"] = command
        visto["config_dir"] = config_dir
        return True

    monkeypatch.setattr(reg, "_engine_of", lambda pid: motor)
    monkeypatch.setattr(reg, "_config_dir_of", lambda pid: None)
    monkeypatch.setattr(reg, "sanitize_cwd", lambda cwd: "-tmp")
    monkeypatch.setattr(reg.tmux, "kill_session", lambda n: None)
    monkeypatch.setattr(reg.tmux, "new_session", _fake_new)
    r = reg.SessionRegistry(projects_dir=tmp_path / "projects")
    monkeypatch.setattr(r, "_pane_of", lambda name: {"cwd": "/tmp", "pid": 4242})
    monkeypatch.setattr(r, "_forget", lambda name: None)
    # Sem isto a busca do `claude` dentro do pane leria o /proc real, onde o pid 4242 pode existir.
    monkeypatch.setattr(reg, "agente_do_pane", lambda pid, children=None: ("claude", None))
    return r, sid


@pytest.mark.parametrize("tier", ["default", "priority"])
@pytest.mark.parametrize("ready", [False, True])
def test_resume_preserves_service_tier_before_kill(tmp_path, monkeypatch, tier, ready):
    from app import cliproxy
    _fixed_proxy(monkeypatch)
    monkeypatch.setattr(cliproxy, "supports_fast", lambda *a: True)
    monkeypatch.setenv("CLAUDE_CODE_EXTRA_BODY", "{}")
    seen = {}
    registry, sid = _prep_resume(tmp_path, monkeypatch, seen, "proxy")
    monkeypatch.setattr(registry, "_pane_of", lambda name: {"cwd": str(tmp_path), "pid": 4242})
    monkeypatch.setattr(reg, "_config_dir_of", lambda pid: tmp_path)
    monkeypatch.setattr(reg, "agente_do_pane", lambda pid, children=None: ("claude", 4243))
    monkeypatch.setattr(procinfo, "_model_of", lambda pid: ("gpt-5.5", "high"))
    stopped = []
    def read(pid, name):
        assert not stopped
        return tier if name == "CP_ENGINE_SERVICE_TIER" else None
    def confirm(name, meta):
        assert stopped == [name]
        assert "command" in seen
        assert meta == {
            "session_id": sid, "config_dir": str(tmp_path), "engine": "proxy",
            "model": "gpt-5.5", "service_tier": tier, "engine_account": None,
            "engine_credential_id": None, "engine_account_base_url": None,
        }
        if not ready:
            raise ValueError("o Claude saiu durante a reabertura")
    monkeypatch.setattr(procinfo, "_env_var_of", read)
    monkeypatch.setattr(reg.tmux, "kill_session", lambda name: stopped.append(name))
    monkeypatch.setattr(registry, "wait_for_claude", confirm)
    if ready:
        registry.resume("s", sid)
        assert registry._jsonl_cache["s"].endswith(f"{sid}.jsonl")
    else:
        with pytest.raises(ValueError, match="saiu durante a reabertura"):
            registry.resume("s", sid)
        assert "s" not in registry._jsonl_cache
    assert f"--service-tier {tier} -- claude" in seen["command"]
    assert stopped == ["s"]


@pytest.mark.parametrize("source", ["shell", "user", "project", "local"])
def test_resume_invalid_fast_body_preserves_terminal(tmp_path, monkeypatch, source):
    from app import cliproxy
    _fixed_proxy(monkeypatch)
    monkeypatch.setattr(cliproxy, "supports_fast", lambda *a: True)
    monkeypatch.setenv("CLAUDE_CODE_EXTRA_BODY", "broken" if source == "shell" else "{}")
    seen = {}
    registry, sid = _prep_resume(tmp_path, monkeypatch, seen, "proxy")
    monkeypatch.setattr(registry, "_pane_of", lambda name: {"cwd": str(tmp_path), "pid": 4242})
    monkeypatch.setattr(reg, "_config_dir_of", lambda pid: tmp_path)
    monkeypatch.setattr(reg, "agente_do_pane", lambda pid, children=None: ("claude", 4243))
    monkeypatch.setattr(procinfo, "_model_of", lambda pid: ("gpt-5.5", None))
    monkeypatch.setattr(procinfo, "_env_var_of", lambda pid, name: "priority" if name == "CP_ENGINE_SERVICE_TIER" else None)
    if source != "shell":
        folder = tmp_path if source == "user" else tmp_path / ".claude"
        folder.mkdir(exist_ok=True)
        filename = "settings.local.json" if source == "local" else "settings.json"
        (folder / filename).write_text('{"env":{"CLAUDE_CODE_EXTRA_BODY":"broken"}}', encoding="utf-8")
    monkeypatch.setattr(reg.tmux, "kill_session", lambda name: pytest.fail("a origem deve continuar viva"))
    monkeypatch.setattr(registry, "wait_for_claude", lambda *a: pytest.fail("não pode tentar reabrir"))
    with pytest.raises(ValueError, match="CLAUDE_CODE_EXTRA_BODY: JSON inválido"):
        registry.resume("s", sid)
    assert "command" not in seen


@pytest.mark.parametrize("removed", [False, True])
def test_resume_incompatible_priority_refuses_or_clears_removed_engine(tmp_path, monkeypatch, removed):
    from app import cliproxy
    _motor()
    monkeypatch.setattr(cliproxy, "supports_fast", lambda *a: False)
    seen = {}
    registry, sid = _prep_resume(tmp_path, monkeypatch, seen, "removed" if removed else "kimi")
    monkeypatch.setattr(reg, "agente_do_pane", lambda pid, children=None: ("claude", 4243))
    monkeypatch.setattr(procinfo, "_model_of", lambda pid: ("k3", None))
    monkeypatch.setattr(procinfo, "_env_var_of", lambda pid, name: "priority" if name == "CP_ENGINE_SERVICE_TIER" else None)
    stopped = []
    monkeypatch.setattr(reg.tmux, "kill_session", lambda name: stopped.append(name))
    if removed:
        registry.resume("s", sid)
        assert seen["command"] == f"claude --resume {sid}"
    else:
        with pytest.raises(ValueError, match="service_tier"):
            registry.resume("s", sid)
        assert not stopped and not seen


def test_resume_de_pane_vivo_preserva_o_motor(tmp_path, monkeypatch):
    _motor()
    visto = {}
    r, sid = _prep_resume(tmp_path, monkeypatch, visto, "kimi")
    info = r.resume("s", sid)
    assert visto["command"] == f"hangar-engine --exec kimi -- claude --resume {sid}"
    assert info.engine == "kimi"


def test_resume_de_pane_aberto_no_shell_usa_a_conta_do_claude_filho(tmp_path, monkeypatch):
    # O pid do pane é o fish; a conta mora no ambiente do `claude` filho (4243).
    visto = {}
    r, sid = _prep_resume(tmp_path, monkeypatch, visto, None)
    conta = tmp_path / ".claude-b"
    (conta / "projects" / "-tmp").mkdir(parents=True)
    (conta / "projects" / "-tmp" / f"{sid}.jsonl").write_text("", encoding="utf-8")
    monkeypatch.setattr(reg, "agente_do_pane", lambda pid, children=None: ("claude", 4243))
    monkeypatch.setattr(reg, "_config_dir_of", lambda pid: conta if pid == 4243 else None)
    monkeypatch.setattr(procinfo, "_model_of", lambda pid: ("sonnet", None) if pid == 4243 else (None, None))
    monkeypatch.setattr(procinfo, "_env_var_of", lambda pid, nome: None)
    info = r.resume("s", sid)
    assert info.jsonl == str(conta / "projects" / "-tmp" / f"{sid}.jsonl")
    assert visto["config_dir"] == str(conta) and "--model sonnet" in visto["command"]


def test_resume_de_motor_removido_nao_trava_a_sessao(tmp_path, monkeypatch):
    # Motor apagado no app depois da sessão nascer: melhor ressuscitar na conta Anthropic (e o badge
    # mostrar isso) do que recusar o resume e deixar a sessão inacessível.
    visto = {}
    r, sid = _prep_resume(tmp_path, monkeypatch, visto, "sumiu")
    info = r.resume("s", sid)
    assert visto["command"] == f"claude --resume {sid}"
    assert info.engine is None


def test_resume_do_arquivo_aceita_motor(tmp_path, monkeypatch):
    # api.py:1942 usa create(resume_session_id=...): o pane morreu, não há /proc para ler, então o
    # motor vem do cliente. Sem isto, retomar do Arquivo troca de motor calado.
    _motor()
    visto = {}
    r = _reg(tmp_path, monkeypatch, visto)
    sid = "11111111-2222-3333-4444-555555555555"
    info = r.create("s", str(tmp_path), resume_session_id=sid, engine="kimi")
    assert visto["command"] == f"hangar-engine --exec kimi -- claude --resume {sid}"
    assert info.engine == "kimi"


# ── Task 3: escolha de modelo/janela entra no prefixo hangar-engine ────────────────────────────────


def test_create_com_escolha_poe_modelo_e_esforco_no_comando(tmp_path, monkeypatch):
    # A flag do modelo chega ao claude; o uuid é aleatório, então confere por prefixo. O modo de
    # permissão vem da conta na criação (fixado aqui pra não ler o settings.json de quem roda).
    visto = {}
    monkeypatch.setattr(reg.modo_permissao, "modo_da_conta", lambda cfg: "acceptEdits")
    _reg(tmp_path, monkeypatch, visto).create("s", str(tmp_path), model="k3-256k", effort="high")
    assert visto["command"].startswith("claude --session-id ")
    assert visto["command"].endswith("--model k3-256k --effort high --permission-mode acceptEdits")


def test_create_com_motor_e_escolha_remonta_o_prefixo_com_modelo_e_janela(tmp_path, monkeypatch):
    """O prefixo do motor leva modelo E janela: a flag sozinha ganharia só de ANTHROPIC_MODEL, e o
    ambiente (aliases, subagente, janela) voltaria pro modelo do motor — o cenário "motor de 1M
    com modelo de 262k"."""
    _motor()
    visto = {}
    _reg(tmp_path, monkeypatch, visto).create("s", str(tmp_path), engine="kimi",
                                              model="k3-256k", context_window=262144)
    assert visto["command"].startswith(
        "hangar-engine --exec kimi --model k3-256k --context 262144 -- claude --session-id ")


def test_create_com_motor_e_modelo_sem_janela_omite_o_context(tmp_path, monkeypatch):
    # Provedor que não reporta context_length: sem o número do modelo, o --context não pode sair
    # (exportar a janela do MOTOR com outro modelo é o bug de volta).
    _motor()
    visto = {}
    _reg(tmp_path, monkeypatch, visto).create("s", str(tmp_path), engine="kimi",
                                              model="k3-256k", context_window=None)
    assert visto["command"].startswith(
        "hangar-engine --exec kimi --model k3-256k -- claude --session-id ")


def test_resume_preserva_modelo_e_janela_do_pane(tmp_path, monkeypatch):
    """Dívida de teste do caminho de resume: sem o par procinfo._model_of/_env_var_of aplicado, a
    sessão ressuscita com a flag num modelo e o AMBIENTE noutro (as cinco chaves, o subagente e a
    janela voltariam pro modelo do motor) — e nada na tela acusa."""
    _motor()
    monkeypatch.setattr(procinfo, "_model_of", lambda pid: ("k3-256k", "high"))
    monkeypatch.setattr(procinfo, "_env_var_of", lambda pid, nome: "262144" if nome == "CLAUDE_CODE_MAX_CONTEXT_TOKENS" else None)
    visto = {}
    r, sid = _prep_resume(tmp_path, monkeypatch, visto, "kimi")
    info = r.resume("s", sid)
    assert visto["command"] == (
        f"hangar-engine --exec kimi --model k3-256k --context 262144 -- "
        f"claude --resume {sid} --model k3-256k --effort high")
    assert info.engine == "kimi"


def test_resume_com_modelo_marcado_de_janela_nao_estoura(tmp_path, monkeypatch):
    """`opus[1m]` é o que o próprio Claude Code anexa ao nome do modelo, e está no settings.json das
    contas do usuário. Com o colchete fora da whitelist, retomar uma sessão dessas estourava."""
    _motor()
    monkeypatch.setattr(procinfo, "_model_of", lambda pid: ("opus[1m]", None))
    monkeypatch.setattr(procinfo, "_env_var_of", lambda pid, nome: None)
    visto = {}
    r, sid = _prep_resume(tmp_path, monkeypatch, visto, None)
    r.resume("s", sid)
    assert visto["command"] == f"claude --resume {sid} --model 'opus[1m]'"


def test_resume_com_modelo_invalido_nao_mata_o_pane(tmp_path, monkeypatch):
    """O comando é montado ANTES do kill. Montar depois trocava 'o resume falhou' por 'a sessão foi
    destruída e não relançada' — o pane já não existia quando a validação estourava."""
    _motor()
    monkeypatch.setattr(procinfo, "_model_of", lambda pid: ("opus; rm -rf /", None))
    monkeypatch.setattr(procinfo, "_env_var_of", lambda pid, nome: None)
    visto = {}
    mortes = []
    r, sid = _prep_resume(tmp_path, monkeypatch, visto, None)
    monkeypatch.setattr(reg.tmux, "kill_session", lambda n: mortes.append(n))
    with pytest.raises(ValueError):
        r.resume("s", sid)
    assert mortes == []                 # a sessão continua de pé
    assert "command" not in visto       # e nada foi relançado


def test_resume_com_modelo_sem_janela_omite_o_context(tmp_path, monkeypatch):
    _motor()
    monkeypatch.setattr(procinfo, "_model_of", lambda pid: ("k3-256k", "high"))
    monkeypatch.setattr(procinfo, "_env_var_of", lambda pid, nome: None)
    visto = {}
    r, sid = _prep_resume(tmp_path, monkeypatch, visto, "kimi")
    r.resume("s", sid)
    assert visto["command"] == (
        f"hangar-engine --exec kimi --model k3-256k -- claude --resume {sid} --model k3-256k --effort high")


def test_resume_sem_modelo_no_pane_nao_poe_flags_nem_context(tmp_path, monkeypatch):
    # Sessão que subiu sem escolha: o resume tem que continuar byte por byte o de hoje.
    _motor()
    monkeypatch.setattr(procinfo, "_model_of", lambda pid: (None, None))
    monkeypatch.setattr(procinfo, "_env_var_of", lambda pid, nome: "262144" if nome == "CLAUDE_CODE_MAX_CONTEXT_TOKENS" else None)
    visto = {}
    r, sid = _prep_resume(tmp_path, monkeypatch, visto, "kimi")
    r.resume("s", sid)
    assert visto["command"] == f"hangar-engine --exec kimi -- claude --resume {sid}"


def test_resume_le_a_janela_antes_de_matar_o_pane(tmp_path, monkeypatch):
    """B2 da revisão final. A janela mora no /proc/<pid>/environ do processo que está no pane:
    lê-la DEPOIS do kill_session devolve nada (o /proc some junto), e a sessão ressuscita sem
    --context — compactando em ~167k com um modelo de 262k, calado. O fake deixa o environ
    ILEGÍVEL depois do kill: se a ordem voltar a errar, o --context 262144 some do comando."""
    _motor()
    monkeypatch.setattr(procinfo, "_model_of", lambda pid: ("k3-256k", "high"))
    morto = {"sim": False}

    def _env(pid, nome):
        if morto["sim"] or nome != "CLAUDE_CODE_MAX_CONTEXT_TOKENS":
            return None
        return "262144"

    def _kill(nome):
        morto["sim"] = True

    monkeypatch.setattr(procinfo, "_env_var_of", _env)
    visto = {}
    r, sid = _prep_resume(tmp_path, monkeypatch, visto, "kimi")
    monkeypatch.setattr(reg.tmux, "kill_session", _kill)
    r.resume("s", sid)
    assert visto["command"] == (
        f"hangar-engine --exec kimi --model k3-256k --context 262144 -- "
        f"claude --resume {sid} --model k3-256k --effort high")


def test_resume_de_motor_removido_descarta_a_escolha_no_fallback(tmp_path, monkeypatch):
    """B3 (versão estreita) da revisão final. Motor apagado cai pro fallback da conta Anthropic —
    decisão anterior a esta branch, não se mexe. O que ESTA branch piorou é carregar junto o
    modelo/esforço do MOTOR: `claude --resume … --model k3-256k --effort high` na conta Anthropic
    é sessão inviável (id que ela não conhece). No fallback, descartar modelo, esforço e janela:
    resume pelado, como antes desta branch."""
    monkeypatch.setattr(procinfo, "_model_of", lambda pid: ("k3-256k", "high"))
    monkeypatch.setattr(procinfo, "_env_var_of", lambda pid, nome: "262144" if nome == "CLAUDE_CODE_MAX_CONTEXT_TOKENS" else None)
    visto = {}
    r, sid = _prep_resume(tmp_path, monkeypatch, visto, "sumiu")
    info = r.resume("s", sid)
    assert visto["command"] == f"claude --resume {sid}"
    assert info.engine is None
