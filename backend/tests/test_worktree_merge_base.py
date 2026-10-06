import subprocess

import pytest

from app import worktrees
from tests.test_worktrees_parity import git, rust


def _remote_merge_scene(tmp_path, source, remote="origin"):
    repo = tmp_path / "repo"
    repo.mkdir()
    git(repo, "init", "-b", "main")
    git(repo, "config", "user.name", "Teste")
    git(repo, "config", "user.email", "teste@example.invalid")
    git(repo, "commit", "--allow-empty", "-m", "Inicial")
    origin = tmp_path / "origin.git"
    git(tmp_path, "init", "--bare", "-b", "main", str(origin))
    git(repo, "remote", "add", remote, str(origin))
    git(repo, "push", "-u", remote, "main")
    git(repo, "branch", "server")
    git(repo, "push", remote, "server")
    base = "main" if source == "remote-main" else "server"
    start = "server" if source == "local-server" else f"{remote}/{base}"
    feature = tmp_path / "feature"
    git(repo, "worktree", "add", "--no-track", "-b", "feature", str(feature), start)
    (feature / "feature.txt").write_text("Implementação\n", encoding="utf-8")
    git(feature, "add", "feature.txt")
    git(feature, "commit", "-m", "Implementação")
    integration = tmp_path / "integration"
    git(repo, "worktree", "add", "--detach", str(integration), f"{remote}/{base}")
    git(integration, "merge", "--no-ff", "-m", "Integração", "feature")
    git(integration, "push", remote, f"HEAD:{base}")
    return repo, feature


@pytest.mark.parametrize("backend", ["python", "rust"])
@pytest.mark.parametrize("source, configured, action, remote, expected_base, merged, degraded", [
    ("remote-main", None, None, "origin", "origin/main", True, False),
    ("remote-server", None, None, "origin", "origin/server", True, False),
    ("local-server", None, None, "origin", "origin/server", True, False),
    ("remote-server", "server", None, "origin", "origin/server", True, False),
    ("remote-server", "origin/server", None, "origin", "origin/server", True, False),
    ("remote-server", "main", None, "origin", "origin/main", False, False),
    ("remote-server", None, "expire", "origin", "origin/main", False, False),
    ("remote-server", None, "switch", "origin", "origin/server", True, False),
    ("remote-server", None, "new-commit", "origin", "origin/server", False, False),
    ("remote-server", None, "missing", "origin", "origin/server", False, True),
    ("remote-server", "origin/missing", None, "origin", "origin/missing", False, True),
    ("local-server", "server", None, "team", "team/server", True, False),
    ("remote-server", None, "tag", "origin", "refs/remotes/origin/server", True, False),
    ("remote-server", "main", "tag-main", "origin", "refs/remotes/origin/main", False, False),
    ("local-server", None, "ambiguous", "origin", "server", False, True),
    ("local-server", None, "upstream", "origin", "origin/server", True, False),
])
def test_merge_base_uses_creation_ref_and_published_history(
        tmp_path, backend, source, configured, action, remote, expected_base, merged, degraded):
    repo, feature = _remote_merge_scene(tmp_path, source, remote)
    if configured:
        git(repo, "config", "branch.feature.hangar-base", configured)
    if action == "expire":
        git(repo, "reflog", "expire", "--expire=all", "refs/heads/feature")
    elif action == "switch":
        git(repo, "switch", "-c", "unrelated", f"{remote}/server")
    elif action == "new-commit":
        (feature / "pending.txt").write_text("Não integrada\n", encoding="utf-8")
        git(feature, "add", "pending.txt")
        git(feature, "commit", "-m", "Ainda em andamento")
    elif action == "missing":
        git(repo, "update-ref", "-d", f"refs/remotes/{remote}/server")
    elif action == "tag":
        git(repo, "tag", "origin/server", "main")
    elif action == "tag-main":
        git(repo, "tag", "origin/main", "refs/remotes/origin/server")
    elif action in ("ambiguous", "upstream"):
        git(repo, "remote", "add", "team", str(tmp_path / "origin.git"))
        git(repo, "fetch", "team")
        if action == "upstream":
            git(repo, "branch", "--set-upstream-to=origin/server", "server")
    if backend == "python":
        st = worktrees.status(str(feature), main=str(repo), measure=False)
    else:
        result = rust("worktree_status", path=str(feature), sessions=[], measure=False)
        assert result["ok"], result
        st = result["result"]
    assert st["base"] == expected_base
    assert st["merged"] is merged
    assert st["degraded"] is degraded
    if merged:
        assert st["ahead"] == 0 and st["behind"] == 1
    if action == "new-commit":
        assert st["ahead"] == 1


def test_reflog_error_degrades_even_when_fallback_is_merged(tmp_path, monkeypatch):
    repo, feature = _remote_merge_scene(tmp_path, "remote-main")
    real_run = worktrees._run

    def unreadable(cwd, *args, **kwargs):
        if args[:2] == ("reflog", "show"):
            return subprocess.CompletedProcess(args, 128, "", "Falha ao ler o reflog")
        return real_run(cwd, *args, **kwargs)

    monkeypatch.setattr(worktrees, "_run", unreadable)
    st = worktrees.status(str(feature), main=str(repo), measure=False)
    assert st["base"] == "origin/main"
    assert st["merged"] is False
    assert st["degraded"] is True
