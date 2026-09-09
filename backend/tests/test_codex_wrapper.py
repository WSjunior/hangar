"""O wrapper do `codex` do shell (scripts/hangar-codex) com o BACKEND DESLIGADO.

O que estes testes protegem: antes, toda decisao do wrapper era uma pergunta a API local — com o
serviço parado ele nao abria nada, enquanto `claude`, `pi` e `kimi` abriam. Cada chamada deve criar uma nova sessao, mesmo quando ha outra no mesmo diretorio.

O modulo e carregado por caminho porque `scripts/hangar-codex` nao tem extensao `.py` (e um
executavel do PATH, nao um pacote).
"""
import importlib.machinery
import importlib.util
import os
import signal
from pathlib import Path

import pytest
from unittest.mock import patch

from app.adapters.codex import sessions as codex_sessions

_CAMINHO = Path(__file__).resolve().parents[2] / "scripts" / "hangar-codex"


def _wrapper():
    # Loader EXPLICITO: sem extensao `.py`, `spec_from_file_location` devolve None (nao acha um
    # loader pelo sufixo) e o erro sai como um AttributeError sem relacao com o que aconteceu.
    loader = importlib.machinery.SourceFileLoader("hangar_codex_wrapper", str(_CAMINHO))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    mod = importlib.util.module_from_spec(spec)
    loader.exec_module(mod)
    return mod


@pytest.fixture
def w():
    return _wrapper()


@pytest.fixture(autouse=True)
def _sidecars_em_tmp(tmp_path):
    sdir = tmp_path / "codex-sessions"
    with patch.object(codex_sessions, "_dir", lambda: sdir):
        yield sdir


def _salva(nome, cwd, rollout, app_pid=None):
    codex_sessions.save(nome, f"tid-{nome}", str(rollout), cwd,
                        endpoint="ws://127.0.0.1:1", app_pid=app_pid)


def test_cria_direto_no_tmux_com_o_comando_do_lancador(w):
    from app.adapters.codex import lancador
    chamadas = []

    class _R:
        returncode = 0
        stderr = ""

    with patch.object(w, "_tmux_vivas", return_value=set()), \
         patch.object(w.subprocess, "run", lambda *a, **k: chamadas.append(a[0]) or _R()):
        nome = w._create_local("/tmp/proj", "revise", lancador)

    assert nome == "proj"
    argv = chamadas[0]
    assert argv[:4] == ["tmux", "new-session", "-d", "-s"]
    # A identidade vai por env, que e de onde o lancador a le — nao repetida no comando.
    assert f"CP_SESSION_NAME={nome}" in argv
    assert "hangar-codex-tui --cwd /tmp/proj --prompt revise" in argv[-1]


def test_nome_livre_pula_o_que_o_tmux_ja_tem(w):
    from app.adapters.codex import lancador

    class _R:
        returncode = 0
        stderr = ""

    with patch.object(w, "_tmux_vivas", return_value={"proj", "proj-2"}), \
         patch.object(w.subprocess, "run", lambda *a, **k: _R()):
        assert w._create_local("/tmp/proj", None, lancador) == "proj-3"


def test_limpeza_sem_api_mata_o_app_server_e_apaga_o_sidecar(w, tmp_path):
    """Sem backend, era aqui que sobrava sidecar orfao mais um servidor escutando em loopback."""
    rollout = tmp_path / "a.jsonl"
    rollout.write_text("x")
    _salva("proj", "/tmp/proj", rollout, app_pid=4242)
    mortos = []

    with patch.object(w.os, "kill", lambda pid, sig: mortos.append((pid, sig))):
        w._limpar_local("proj", codex_sessions)

    assert mortos == [(4242, signal.SIGTERM)]
    assert codex_sessions.load("proj") is None


def test_limpeza_sobrevive_a_app_server_ja_morto(w, tmp_path):
    """O caso COMUM: o lancador chegou primeiro. Nao pode virar erro na saida do terminal."""
    rollout = tmp_path / "a.jsonl"
    rollout.write_text("x")
    _salva("proj", "/tmp/proj", rollout, app_pid=4242)

    def _morto(pid, sig):
        raise ProcessLookupError

    with patch.object(w.os, "kill", _morto):
        w._limpar_local("proj", codex_sessions)

    assert codex_sessions.load("proj") is None


def test_carregar_os_modulos_nao_puxa_o_pacote_dos_adapters(w):
    """`from app.adapters.codex import sessions` executa `app/adapters/__init__.py`, que instancia
    os quatro adapters e puxa `app.config` -> pydantic. Este wrapper roda no `python3` do SISTEMA:
    numa maquina sem pydantic instalado ali, aquele import falharia e o wrapper voltaria a depender
    da API — que e justamente o que ele nao pode fazer. Aqui passaria por acidente (esta maquina tem
    pydantic no sistema), entao o que se afirma e o FATO que segura a garantia: o pacote nao e
    tocado."""
    import sys as _sys
    for nome in ("app.adapters", "app.adapters.codex"):
        _sys.modules.pop(nome, None)

    sessions, lancador = w._do_backend()

    assert sessions is not None and lancador is not None
    assert "app.adapters" not in _sys.modules
    assert callable(lancador.comando_do_lancador)


@pytest.mark.parametrize("prompt", [None, "revise"])
@pytest.mark.parametrize("backend", ["ativo", "fora", "sem_token"])
@pytest.mark.parametrize("pane", ["fora", "vivo", "morto"])
def test_main_cria_sessao_nova_e_escolhe_cliente(w, monkeypatch, tmp_path, prompt, backend, pane):
    from app.adapters.codex import lancador
    from types import SimpleNamespace

    cwd = str(tmp_path)
    _salva(tmp_path.name, cwd, tmp_path / "antigo.jsonl")
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(w.sys, "argv", ["hangar-codex"] + ([prompt] if prompt else []))
    monkeypatch.setattr(w, "_do_backend", lambda: (codex_sessions, lancador))
    monkeypatch.setattr(w, "_tmux_vivas", lambda: {tmp_path.name})
    monkeypatch.setattr(w, "_env", lambda: {})
    monkeypatch.delenv("CP_AUTH_TOKEN", raising=False)
    if pane == "fora":
        monkeypatch.delenv("TMUX", raising=False)
        monkeypatch.delenv("TMUX_PANE", raising=False)
    else:
        monkeypatch.setenv("TMUX", "socket-herdado")
        monkeypatch.setenv("TMUX_PANE", "%42")
    chamadas, pedidos = [], []

    def api(method, path, body=None):
        pedidos.append((method, path, body))
        if backend == "fora":
            raise RuntimeError("backend inacessivel")
        if method == "GET":
            return [{"name": tmp_path.name, "cwd": cwd, "provider": "codex"}]
        if method == "POST":
            return {"name": body["name"]}
        return {}

    if backend != "sem_token":
        monkeypatch.setattr(w, "_api", api)

    def run(argv, **kwargs):
        chamadas.append(argv)
        command = argv[1]
        if command == "list-panes":
            return SimpleNamespace(returncode=0 if pane == "vivo" else 1)
        if command == "has-session":
            return SimpleNamespace(returncode=1)
        if command == "attach-session":
            assert "TMUX" not in os.environ
            assert "TMUX_PANE" not in os.environ
        return SimpleNamespace(returncode=0, stderr="")

    monkeypatch.setattr(w.subprocess, "run", run)
    assert w.main() == 0
    name = f"{tmp_path.name}-2"
    if backend == "ativo":
        assert pedidos[1] == ("POST", "/api/sessions", {
            "name": name, "cwd": cwd, "provider": "codex", "initial_prompt": prompt,
        })
    else:
        creation = next(c for c in chamadas if c[1] == "new-session")
        assert creation[4] == name
        assert "--resume" not in creation[-1]
        if prompt:
            assert f"--prompt {prompt}" in creation[-1]
    action = "switch-client" if pane == "vivo" else "attach-session"
    assert ["tmux", action, "-t", f"={name}"] in chamadas
    assert all("/input" not in path for _, path, _ in pedidos)
    assert codex_sessions.load(tmp_path.name) is not None
