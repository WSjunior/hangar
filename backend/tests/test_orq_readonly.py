"""Proteção real do código, preservando relatórios e o lançador no pane."""
import asyncio
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
from unittest.mock import Mock

import pytest

from app import orq_readonly


@pytest.mark.skipif(sys.platform != "linux" or not shutil.which("bwrap"),
                    reason="prova Linux exige bubblewrap instalado")
def test_child_cannot_write_worktree_or_git_but_can_save_report(tmp_path):
    repo = tmp_path / "repo"
    worktree = tmp_path / "worktree"
    subprocess.run(["git", "init", str(repo)], check=True, capture_output=True)
    (repo / "source.txt").write_text("base")
    subprocess.run(["git", "-C", str(repo), "add", "source.txt"], check=True, capture_output=True)
    subprocess.run(["git", "-C", str(repo), "-c", "user.name=Test", "-c",
                    "user.email=test@example.invalid", "-c", "commit.gpgsign=false",
                    "commit", "--allow-empty", "-m", "initial"], check=True, capture_output=True)
    subprocess.run(["git", "-C", str(repo), "worktree", "add", "--detach", str(worktree)],
                   check=True, capture_output=True)
    source = worktree / "source.txt"
    source.write_text("original")
    snapshot = subprocess.check_output(
        ["git", "-C", str(worktree), "-c", "user.name=Test", "-c",
         "user.email=test@example.invalid", "stash", "create"], text=True).strip()
    subprocess.run(["git", "-C", str(worktree), "stash", "store", snapshot],
                   check=True, capture_output=True)
    alias = tmp_path / "alias"
    alias.symlink_to(worktree, target_is_directory=True)
    prefix = orq_readonly.prepare(str(alias))
    gitdir = Path(subprocess.check_output(
        ["git", "-C", str(worktree), "rev-parse", "--absolute-git-dir"], text=True).strip())
    report = tmp_path / "report.txt"
    probe = """import pathlib,subprocess,sys
source,gitdir,common,report,copy=map(pathlib.Path,sys.argv[1:6])
snapshot=sys.argv[6]
assert source.read_text() == 'original'
for path in (source, gitdir/'forbidden', common/'forbidden'):
    try: path.write_text('changed')
    except OSError: pass
    else: raise AssertionError(f'write allowed: {path}')
subprocess.run(['git','clone','--no-hardlinks','--no-checkout',str(source.parent),str(copy)],check=True)
subprocess.run(['git','-C',str(copy),'checkout','--detach',snapshot],check=True)
assert (copy/'source.txt').read_text() == 'original'
(copy/'source.txt').write_text('changed copy')
assert source.read_text() == 'original'
report.write_text('verified')
"""
    child = shlex.join([sys.executable, "-c", probe, str(alias / "source.txt"),
                        str(gitdir), str(repo / ".git"), str(report), str(tmp_path / "copy"), snapshot])
    result = subprocess.run([*prefix, "/bin/sh", "-c", child], capture_output=True,
                            text=True, timeout=10)
    assert result.returncode == 0, result.stderr
    assert source.read_text() == "original"
    assert report.read_text() == "verified"


@pytest.mark.parametrize("platform,binary,message", [
    ("win32", "/usr/bin/bwrap", "Linux"),
    ("linux", None, "instale bwrap"),
])
def test_missing_support_refuses_before_process(monkeypatch, tmp_path, platform, binary, message):
    monkeypatch.setattr(orq_readonly.sys, "platform", platform)
    monkeypatch.setattr(orq_readonly.shutil, "which", lambda _: binary)
    run = Mock()
    monkeypatch.setattr(orq_readonly.subprocess, "run", run)
    with pytest.raises(ValueError, match=message):
        orq_readonly.prepare(str(tmp_path))
    run.assert_not_called()


def test_refused_probe_has_no_unprotected_fallback(monkeypatch, tmp_path):
    monkeypatch.setattr(orq_readonly.sys, "platform", "linux")
    monkeypatch.setattr(orq_readonly.shutil, "which", lambda _: "/usr/bin/bwrap")
    monkeypatch.setattr(orq_readonly.subprocess, "run", Mock(side_effect=[
        subprocess.CompletedProcess([], 128, "", "not a repository"),
        subprocess.CompletedProcess([], 1, "", "user namespaces disabled"),
    ]))
    with pytest.raises(ValueError, match="user namespaces disabled"):
        orq_readonly.prepare(str(tmp_path))


def test_home_is_not_made_read_only(monkeypatch, tmp_path):
    monkeypatch.setattr(orq_readonly.sys, "platform", "linux")
    monkeypatch.setattr(orq_readonly.shutil, "which", lambda _: "/usr/bin/bwrap")
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    run = Mock(return_value=subprocess.CompletedProcess([], 128, "", "not a repository"))
    monkeypatch.setattr(orq_readonly.subprocess, "run", run)
    with pytest.raises(ValueError, match="HOME inteira"):
        orq_readonly.prepare(str(tmp_path))
    assert run.call_count == 1


def test_runtime_inside_code_is_refused(monkeypatch, tmp_path):
    monkeypatch.setattr(orq_readonly.sys, "platform", "linux")
    monkeypatch.setattr(orq_readonly.shutil, "which", lambda _: "/usr/bin/bwrap")
    monkeypatch.setenv("CODEX_HOME", str(tmp_path / ".codex"))
    run = Mock(return_value=subprocess.CompletedProcess([], 128, "", "not a repository"))
    monkeypatch.setattr(orq_readonly.subprocess, "run", run)
    with pytest.raises(ValueError, match="runtime"):
        orq_readonly.prepare(str(tmp_path))
    assert run.call_count == 1


@pytest.mark.parametrize("method", ["resume", "resume_candidates"])
def test_live_resume_refuses_before_killing_protected_pane(monkeypatch, tmp_path, method):
    from app import registry

    reg = registry.SessionRegistry(projects_dir=tmp_path)
    monkeypatch.setattr(reg, "_pane_of", lambda _: {"pid": 123, "cwd": str(tmp_path)})
    monkeypatch.setattr(registry, "_descendant_pids", lambda _: [123])
    monkeypatch.setattr(registry, "_cmdline", lambda _: "bwrap --setenv HANGAR_ORQ_READ_ONLY 1")
    kill = Mock()
    monkeypatch.setattr(registry.tmux, "kill_session", kill)
    args = ["protected"]
    if method == "resume":
        args += ["12345678-1234-1234-1234-123456789abc"]
    with pytest.raises(ValueError, match="recrie com --read-only"):
        getattr(reg, method)(*args)
    kill.assert_not_called()


def test_codex_launcher_is_inside_protected_pane(monkeypatch, tmp_path):
    from app import registry

    prefix = ["/usr/bin/bwrap", "--bind", "/", "/", "--ro-bind", str(tmp_path), str(tmp_path), "--"]
    monkeypatch.setattr(orq_readonly, "prepare", Mock(return_value=prefix))
    monkeypatch.setattr(registry, "_exigir_lancador_codex", lambda: None)
    monkeypatch.setattr(registry.tmux, "has_session", lambda _: False)
    monkeypatch.setattr(registry.codex_sessions, "exists", lambda _: False)
    monkeypatch.setattr(registry.codex_sessions, "pretrust_cwd", lambda *a, **kw: None)
    pane = Mock(return_value=False)
    monkeypatch.setattr(registry.tmux, "new_session", pane)
    with pytest.raises(ValueError, match="falha ao criar sessao"):
        registry.SessionRegistry(projects_dir=tmp_path).create(
            "readonly-codex", str(tmp_path), provider="codex", read_only=True,
            initial_prompt="review only", model="gpt-6-astra", effort="high")
    argv = shlex.split(pane.call_args.args[2])
    assert argv[:-3] == prefix
    assert argv[-3:-1] == ["/bin/sh", "-c"]
    launcher = shlex.split(argv[-1])
    assert launcher[0] == "hangar-codex-tui"
    assert launcher[launcher.index("--prompt") + 1] == "review only"
    assert launcher[launcher.index("--model") + 1] == "gpt-6-astra"


def test_api_checks_protection_before_creation_and_passes_flag(monkeypatch, tmp_path):
    from fastapi import HTTPException
    from app import api

    prepare = Mock(side_effect=ValueError("bwrap unavailable"))
    monkeypatch.setattr(orq_readonly, "prepare", prepare)
    create = Mock()
    monkeypatch.setattr(api.registry, "create", create)
    with pytest.raises(HTTPException) as error:
        asyncio.run(api.create_session(api.CreateBody(
            name="readonly", cwd=str(tmp_path), provider="codex", read_only=True)))
    assert error.value.status_code == 400
    create.assert_not_called()

    prepare.side_effect = None
    create = Mock(return_value=api.SessionInfo(name="readonly", cwd=str(tmp_path), provider="kimi"))
    monkeypatch.setattr(api.registry, "create", create)
    asyncio.run(api.create_session(api.CreateBody(
        name="readonly", cwd=str(tmp_path), provider="kimi", read_only=True)))
    assert create.call_args.kwargs["read_only"] is True


@pytest.mark.skipif(sys.platform != "linux" or not shutil.which("bash"),
                    reason="stub de curl usa executável POSIX")
@pytest.mark.parametrize("status", [200, 422])
def test_cli_sends_read_only_and_does_not_claim_success_on_old_backend(tmp_path, status):
    scripts = tmp_path / "scripts"
    scripts.mkdir()
    backend = tmp_path / "backend"
    backend.mkdir()
    (backend / ".env").write_text("CP_AUTH_TOKEN=test-only\n")
    cli = scripts / "hangar-send"
    shutil.copyfile(Path(__file__).resolve().parents[2] / "scripts" / "hangar-send", cli)
    capture = tmp_path / "payload.json"
    curl = scripts / "curl"
    curl.write_text(f"#!{sys.executable}\n" + """import json,os,pathlib,sys
args=sys.argv[1:]
pathlib.Path(os.environ['TEST_PAYLOAD']).write_text(args[args.index('-d')+1])
print('{}')
print(os.environ['TEST_STATUS'])
""")
    curl.chmod(0o700)
    result = subprocess.run(["bash", str(cli), "--new", "review", str(tmp_path), "--read-only"],
                            env={**os.environ, "PATH": str(scripts) + os.pathsep + os.environ["PATH"],
                                 "TEST_PAYLOAD": str(capture), "TEST_STATUS": str(status)},
                            capture_output=True, text=True, timeout=10)
    assert json.loads(capture.read_text())["read_only"] is True
    assert (result.returncode == 0) is (status == 200)
    assert ("sessão criada" in result.stdout) is (status == 200)
