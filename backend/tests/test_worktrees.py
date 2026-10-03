import json

import pytest

from app import git_ops, worktrees
from app.git_ops import GitError
from app.models import SessionInfo


def _repo(path):
    path.mkdir(parents=True, exist_ok=True)
    d = str(path)
    for args in (["init", "-q", "-b", "main"], ["config", "user.email", "t@t"],
                 ["config", "user.name", "t"], ["commit", "-q", "--allow-empty", "-m", "init"]):
        git_ops._run(d, *args)
    return d


def _wt(main, path, branch):
    assert git_ops._run(main, "worktree", "add", "-b", branch, str(path)).returncode == 0
    return str(path)


@pytest.fixture(autouse=True)
def _isolated_removed(tmp_path, monkeypatch):
    monkeypatch.setattr(worktrees, "REMOVED_FILE", tmp_path / "removidas.json")


def test_roots_and_worktree_list(tmp_path):
    main = _repo(tmp_path / "repo")
    wt = _wt(main, tmp_path / "repo-x", "x")
    (tmp_path / "repo-x" / "sub").mkdir()
    assert worktrees.repo_root_of(str(tmp_path / "repo-x" / "sub")) == wt
    assert worktrees.main_repo_of(wt) == main
    assert worktrees.main_repo_of(main) == main
    assert worktrees.worktree_paths(main) == [wt]


def test_claude_cwd_reads_last_line_with_cwd(tmp_path):
    f = tmp_path / "s.jsonl"
    f.write_text("\n".join([json.dumps({"cwd": "/a"}), json.dumps({"cwd": "/b"}),
                            json.dumps({"type": "summary"})]) + "\n")
    assert worktrees.claude_cwd(str(f)) == "/b"


def test_claude_cwd_without_cwd_is_none(tmp_path):
    f = tmp_path / "s.jsonl"
    f.write_text(json.dumps({"type": "summary"}) + "\n")
    assert worktrees.claude_cwd(str(f)) is None


def test_locate_claude_moved_into_worktree(tmp_path):
    main = _repo(tmp_path / "repo")
    wt = _wt(main, tmp_path / "repo-x", "x")
    f = tmp_path / "s.jsonl"
    f.write_text(json.dumps({"cwd": main}) + "\n" + json.dumps({"cwd": wt + "/sub"}) + "\n")
    loc = worktrees.locate("claude", main, str(f))
    assert (loc.branch, loc.worktree, loc.worktree_path, loc.worktree_gone) == ("x", True, wt, False)


def test_locate_claude_worktree_gone(tmp_path):
    main = _repo(tmp_path / "repo")
    f = tmp_path / "s.jsonl"
    f.write_text(json.dumps({"cwd": str(tmp_path / "repo-sumiu")}) + "\n")
    loc = worktrees.locate("claude", main, str(f))
    assert loc.worktree_gone and loc.worktree_path == str(tmp_path / "repo-sumiu")


def test_locate_codex_uses_last_workdir_of_same_repo(tmp_path):
    main = _repo(tmp_path / "repo")
    wt = _wt(main, tmp_path / "repo-x", "x")
    other = _repo(tmp_path / "outro")
    f = tmp_path / "rollout.jsonl"
    calls = [
        {"type": "response_item", "payload": {"type": "custom_tool_call", "name": "exec",
         "input": f'tools.exec_command({{cmd:"git status", workdir:"{wt}"}})'}},
        {"type": "response_item", "payload": {"type": "function_call", "name": "exec_command",
         "arguments": json.dumps({"cmd": f"cd {other} && ls"})}},
    ]
    f.write_text("\n".join(json.dumps(c) for c in calls) + "\n")
    loc = worktrees.locate("codex", main, str(f))
    assert loc.worktree_path == wt and loc.branch == "x"


def test_locate_without_signal_uses_opening_folder(tmp_path):
    main = _repo(tmp_path / "repo")
    loc = worktrees.locate("codex", main, None)
    assert (loc.branch, loc.worktree, loc.worktree_path) == ("main", False, None)


def test_locate_codex_cd_wins_over_workdir_in_same_call(tmp_path):
    main = _repo(tmp_path / "repo")
    wt = _wt(main, tmp_path / "repo-x", "x")
    f = tmp_path / "rollout.jsonl"
    call = {"type": "response_item", "payload": {"type": "function_call", "name": "exec_command",
            "arguments": json.dumps({"cmd": f"cd {wt} && ls", "workdir": main})}}
    f.write_text(json.dumps(call) + "\n")
    loc = worktrees.locate("codex", main, str(f))
    assert loc.worktree_path == wt and loc.branch == "x"


def test_locate_codex_removed_worktree_is_gone(tmp_path):
    main = _repo(tmp_path / "repo")
    wt = _wt(main, tmp_path / "repo-x", "x")
    assert git_ops._run(main, "worktree", "remove", wt).returncode == 0
    f = tmp_path / "rollout.jsonl"
    call = {"type": "response_item", "payload": {"type": "custom_tool_call", "name": "exec",
            "input": f'tools.exec_command({{cmd:"ls", workdir:"{wt}"}})'}}
    f.write_text(json.dumps(call) + "\n")
    loc = worktrees.locate("codex", main, str(f))
    assert loc.worktree_gone and loc.worktree_path == wt


def test_locate_codex_missing_folder_of_other_repo_does_not_count(tmp_path):
    main = _repo(tmp_path / "repo")
    f = tmp_path / "rollout.jsonl"
    call = {"type": "response_item", "payload": {"type": "function_call", "name": "exec_command",
            "arguments": json.dumps({"cmd": "ls", "workdir": str(tmp_path / "outro" / "sumiu")})}}
    f.write_text(json.dumps(call) + "\n")
    loc = worktrees.locate("codex", main, str(f))
    assert (loc.branch, loc.worktree, loc.worktree_path, loc.worktree_gone) == ("main", False, None, False)


def test_locate_codex_deleted_patch_file_does_not_count(tmp_path):
    main = _repo(tmp_path / "repo")
    f = tmp_path / "rollout.jsonl"
    call = {"type": "response_item", "payload": {"type": "custom_tool_call", "name": "apply_patch",
            "input": f"*** Begin Patch\n*** Delete File: {tmp_path / 'outro' / 'a.txt'}\n*** End Patch"}}
    f.write_text(json.dumps(call) + "\n")
    loc = worktrees.locate("codex", main, str(f))
    assert (loc.branch, loc.worktree, loc.worktree_path, loc.worktree_gone) == ("main", False, None, False)


def test_worktree_paths_with_relative_pointers(tmp_path):
    main = _repo(tmp_path / "repo")
    wt = _wt(main, tmp_path / "repo-x", "x")
    # Ponteiros relativos, como grava `worktree.useRelativePaths`.
    (tmp_path / "repo-x" / ".git").write_text("gitdir: ../repo/.git/worktrees/repo-x\n")
    (tmp_path / "repo" / ".git" / "worktrees" / "repo-x" / "gitdir").write_text("../../../../repo-x/.git\n")
    assert worktrees.main_repo_of(wt) == main
    assert worktrees.worktree_paths(main) == [wt]


@pytest.mark.parametrize("lines", [
    ['["function_call", "cwd"]'],
    ["42"],
    [json.dumps({"type": "response_item", "payload": ["function_call"]})],
    [json.dumps({"type": "response_item", "payload": "custom_tool_call"})],
    ['{"type": "response_item", "payload": {"type": "function_call", "arguments": "{\\"cmd\\": \\"cd /'],
])
def test_locate_survives_malformed_lines(tmp_path, lines):
    main = _repo(tmp_path / "repo")
    f = tmp_path / "t.jsonl"
    f.write_text("\n".join(lines))
    for provider in ("codex", "claude"):
        loc = worktrees.locate(provider, main, str(f))
        assert (loc.branch, loc.worktree, loc.worktree_path, loc.worktree_gone) == ("main", False, None, False)


def test_locate_missing_transcript_uses_opening_folder(tmp_path):
    main = _repo(tmp_path / "repo")
    for provider in ("codex", "claude"):
        loc = worktrees.locate(provider, main, str(tmp_path / "nao-existe.jsonl"))
        assert (loc.branch, loc.worktree_path, loc.worktree_gone) == ("main", None, False)


def test_locate_never_raises_on_reader_failure(tmp_path, monkeypatch):
    main = _repo(tmp_path / "repo")
    f = tmp_path / "s.jsonl"
    f.write_text("{}\n")

    def boom(*_):
        raise RuntimeError("transcript torto")
    monkeypatch.setattr(worktrees, "claude_cwd", boom)
    loc = worktrees.locate("claude", main, str(f))
    assert (loc.branch, loc.worktree_path, loc.worktree_gone) == ("main", None, False)


def _commit(path, name):
    (path / name).write_text(name)
    git_ops._run(str(path), "add", name)
    git_ops._run(str(path), "commit", "-q", "-m", name)


def test_status_ahead_dirty_and_ignored(tmp_path):
    main = _repo(tmp_path / "repo")
    (tmp_path / "repo" / ".gitignore").write_text(".env\nnotas.txt\nnode_modules/\n")
    _commit(tmp_path / "repo", ".gitignore")
    (tmp_path / "repo" / ".env").write_text("S=1")
    wt = _wt(main, tmp_path / "repo-x", "x")
    git_ops._run(wt, "config", "branch.x.hangar-base", "main")
    _commit(tmp_path / "repo-x", "a.txt")
    (tmp_path / "repo-x" / "solto.txt").write_text("?")
    (tmp_path / "repo-x" / ".env").write_text("S=1")        # cópia idêntica: não se perde nada
    (tmp_path / "repo-x" / "notas.txt").write_text("minha")  # só existe aqui
    (tmp_path / "repo-x" / "node_modules").mkdir()
    st = worktrees.status(wt, [SessionInfo(name="s1", cwd=main, worktree_path=wt)])
    assert st["branch"] == "x" and st["base"] == "main" and st["ahead"] == 1
    assert st["merged"] is False and st["dirty"] == 1
    assert st["ignored"] == ["notas.txt"]
    assert st["sessions"] == ["s1"] and st["exists"] is True and st["repo"] == main


def test_status_merged_by_ancestor(tmp_path):
    main = _repo(tmp_path / "repo")
    wt = _wt(main, tmp_path / "repo-x", "x")
    _commit(tmp_path / "repo-x", "a.txt")
    git_ops._run(main, "merge", "-q", "--no-ff", "-m", "m", "x")
    assert worktrees.status(wt)["merged"] is True


def test_merged_when_upstream_gone(tmp_path):
    remote = _repo(tmp_path / "remote")
    git_ops._run(remote, "branch", "x")
    main = str(tmp_path / "clone")
    git_ops._run(str(tmp_path), "clone", "-q", remote, main)
    git_ops._run(main, "config", "user.email", "t@t")
    git_ops._run(main, "config", "user.name", "t")
    git_ops._run(main, "switch", "-q", "x")
    _commit(tmp_path / "clone", "a.txt")            # squash do servidor: não é ancestral
    git_ops._run(main, "switch", "-q", "main")
    assert worktrees.is_merged(main, "x", "main") is False
    git_ops._run(remote, "branch", "-D", "x")
    git_ops._run(main, "fetch", "-q", "--prune")
    assert worktrees.is_merged(main, "x", "main") is True


def test_status_missing_folder(tmp_path):
    main = _repo(tmp_path / "repo")
    wt = _wt(main, tmp_path / "repo-x", "x")
    import shutil
    shutil.rmtree(wt)
    st = worktrees.status(wt)
    assert st["exists"] is False and st["repo"] == main and st["branch"] == "x"


def test_list_all_groups_by_main_repo(tmp_path):
    main = _repo(tmp_path / "repo")
    wt = _wt(main, tmp_path / "repo-x", "x")
    _repo(tmp_path / "sem-worktree")
    out = worktrees.list_all([main, wt, str(tmp_path / "sem-worktree")], [])
    assert [r["repo"] for r in out] == [main]
    assert [w["path"] for w in out[0]["worktrees"]] == [wt]


def test_routes_refuse_outside_root(tmp_path, monkeypatch):
    from fastapi.testclient import TestClient
    from app import api, fs
    main = _repo(tmp_path / "repo")
    wt = _wt(main, tmp_path / "repo-x", "x")
    monkeypatch.setattr(fs, "resolve_scan_roots", lambda _s: [tmp_path / "repo"])
    monkeypatch.setattr(api, "resolve_scan_roots", lambda _s: [tmp_path / "repo"])
    monkeypatch.setattr(api.settings, "auth_token", "t")
    r = TestClient(api.app).get("/api/worktrees/detail", params={"path": wt},
                                headers={"Authorization": "Bearer t"})
    assert r.status_code == 403


def test_fresh_worktree_is_not_merged(tmp_path):
    main = _repo(tmp_path / "repo")
    wt = _wt(main, tmp_path / "repo-x", "x")
    assert worktrees.status(wt)["merged"] is False
    _commit(tmp_path / "repo-x", "a.txt")
    assert worktrees.status(wt)["merged"] is False
    git_ops._run(main, "merge", "-q", "--no-ff", "-m", "m", "x")
    assert worktrees.status(wt)["merged"] is True


def test_closed_skips_live_session_transcripts(tmp_path, monkeypatch):
    from app import archive
    from app.registry import sanitize_cwd
    main = _repo(tmp_path / "repo")
    wt = _wt(main, tmp_path / "repo-x", "x")
    proj = tmp_path / "projects"
    (proj / sanitize_cwd(wt)).mkdir(parents=True)
    (proj / sanitize_cwd(wt) / "a.jsonl").write_text("{}\n")
    (proj / sanitize_cwd(wt) / "b.jsonl").write_text("{}\n")
    monkeypatch.setattr(archive, "_contas", lambda *_a: [(None, "", proj)])
    live = SessionInfo(name="s1", cwd=wt, jsonl=str(proj / sanitize_cwd(wt) / "b.jsonl"))
    st = worktrees.status(wt, [live])
    assert st["sessions"] == ["s1"] and st["closed"] == 1


def test_sessions_match_through_symlink(tmp_path):
    main = _repo(tmp_path / "repo")
    wt = _wt(main, tmp_path / "repo-x", "x")
    (tmp_path / "link").symlink_to(tmp_path)
    s = SessionInfo(name="s1", cwd=str(tmp_path / "link" / "repo-x"))
    assert worktrees.status(wt, [s])["sessions"] == ["s1"]


def test_ignored_never_lists_folders(tmp_path):
    main = _repo(tmp_path / "repo")
    (tmp_path / "repo" / ".gitignore").write_text("cache\n")
    _commit(tmp_path / "repo", ".gitignore")
    wt = _wt(main, tmp_path / "repo-x", "x")
    (tmp_path / "repo-x" / "cache").mkdir()
    (tmp_path / "repo-x" / "cache" / "dado.bin").write_text("só aqui")
    assert worktrees.status(wt)["ignored"] == []


def test_status_survives_git_timeout(tmp_path, monkeypatch):
    main = _repo(tmp_path / "repo")
    wt = _wt(main, tmp_path / "repo-x", "x")
    (tmp_path / "repo-x" / "solto.txt").write_text("?")
    real_run = worktrees._run

    def slow(cwd, *args, **kw):
        if args[0] == "status":
            raise git_ops.GitError(504, "git timeout")
        return real_run(cwd, *args, **kw)
    monkeypatch.setattr(worktrees, "_run", slow)
    out = worktrees.list_all([main], [])
    st = out[0]["worktrees"][0]
    assert st["dirty"] == 0 and st["degraded"] is True and st["merged"] is False


def test_upstream_timeout_is_not_merged(tmp_path, monkeypatch):
    main = _repo(tmp_path / "repo")
    wt = _wt(main, tmp_path / "repo-x", "x")
    _commit(tmp_path / "repo-x", "a.txt")
    # Branch que já teve upstream e ele não existe mais: a regra do squash lê como mesclada.
    git_ops._run(wt, "config", "branch.x.remote", "origin")
    git_ops._run(wt, "config", "branch.x.merge", "refs/heads/x")
    st = worktrees.status(wt)
    assert st["merged"] is True and st["degraded"] is False
    real_run = worktrees._run

    def slow(cwd, *args, **kw):
        if args[:2] == ("rev-parse", "--abbrev-ref"):
            raise git_ops.GitError(504, "git timeout")
        return real_run(cwd, *args, **kw)
    monkeypatch.setattr(worktrees, "_run", slow)
    st = worktrees.status(wt)
    assert st["merged"] is False and st["degraded"] is True
    assert worktrees.is_merged(wt, "x", "main") is False


def test_list_all_skips_repo_outside_roots(tmp_path):
    main = _repo(tmp_path / "repo")
    wt = _wt(main, tmp_path / "repo-x", "x")
    (tmp_path / "outra").mkdir()
    assert worktrees.list_all([wt], [], roots=[tmp_path / "outra"]) == []
    assert [r["repo"] for r in worktrees.list_all([wt], [], roots=[tmp_path])] == [main]


def test_detail_on_plain_folder_is_404(tmp_path, monkeypatch):
    from fastapi.testclient import TestClient
    from app import api, fs
    (tmp_path / "comum").mkdir()
    monkeypatch.setattr(fs, "resolve_scan_roots", lambda _s: [tmp_path])
    monkeypatch.setattr(api, "resolve_scan_roots", lambda _s: [tmp_path])
    monkeypatch.setattr(api.settings, "auth_token", "t")
    r = TestClient(api.app).get("/api/worktrees/detail", params={"path": str(tmp_path / "comum")},
                                headers={"Authorization": "Bearer t"})
    assert r.status_code == 404


def test_guest_gets_403_on_every_route(tmp_path, monkeypatch):
    from fastapi.testclient import TestClient
    from app import api, guest_users
    monkeypatch.setattr(guest_users, "_path_override", tmp_path / "guests.json")
    guest_users._reset()
    main = _repo(tmp_path / "repo")
    wt = _wt(main, tmp_path / "repo-x", "x")
    monkeypatch.setattr(api.settings, "auth_token", "secret")
    _, tok = guest_users.create("bia", str(tmp_path), False, True)
    h = {"Authorization": f"Bearer {tok}"}
    c = TestClient(api.app)
    try:
        assert c.get("/api/worktrees", headers=h).status_code == 403
        assert c.get("/api/worktrees/detail", params={"path": wt}, headers=h).status_code == 403
        assert c.post("/api/worktrees/fetch", json={"repo": main}, headers=h).status_code == 403
        assert c.post("/api/worktrees/delete", json={"repo": main, "path": wt},
                      headers=h).status_code == 403
        assert c.post("/api/worktrees/delete-merged", json={"repo": main}, headers=h).status_code == 403
        assert (tmp_path / "repo-x").exists()
    finally:
        guest_users._reset()


def _claude_project(tmp_path, monkeypatch, wt, sid="abc"):
    from app import archive
    from app.registry import sanitize_cwd
    base = tmp_path / "cfg" / "projects"
    (base / sanitize_cwd(wt)).mkdir(parents=True)
    (base / sanitize_cwd(wt) / f"{sid}.jsonl").write_text(json.dumps({"cwd": wt}) + "\n")
    (base / sanitize_cwd(wt) / sid).mkdir()
    monkeypatch.setattr(archive, "_contas", lambda config_dir=None: [(None, "", base)])
    return base


def _merged_wt(main, path, branch):
    """Worktree com um commit já juntado na principal: a recém-criada não conta como mesclada."""
    wt = _wt(main, path, branch)
    _commit(path, f"{branch}.txt")
    assert git_ops._run(main, "merge", "-q", "--no-ff", "-m", "m", branch).returncode == 0
    return wt


def test_delete_clean_merged_moves_conversations_and_branch(tmp_path, monkeypatch):
    main = _repo(tmp_path / "repo")
    wt = _merged_wt(main, tmp_path / "repo-x", "x")
    base = _claude_project(tmp_path, monkeypatch, wt)
    from app.registry import sanitize_cwd
    out = worktrees.delete(main, wt, [])
    assert out == {"removed": wt, "branch_deleted": True, "moved": 2}
    assert not (tmp_path / "repo-x").exists()
    assert (base / sanitize_cwd(main) / "abc.jsonl").exists()
    assert (base / sanitize_cwd(main) / "abc").is_dir()
    assert worktrees.removed()[wt] == main
    assert worktrees.redirect(wt) == main
    assert "x" not in git_ops.list_branches(main)["branches"]


def test_delete_refuses_open_session_and_main(tmp_path):
    main = _repo(tmp_path / "repo")
    wt = _wt(main, tmp_path / "repo-x", "x")
    with pytest.raises(GitError) as busy:
        worktrees.delete(main, wt, [SessionInfo(name="s1", cwd=main, worktree_path=wt)])
    assert busy.value.status == 409 and "s1" in busy.value.detail
    with pytest.raises(GitError) as principal:
        worktrees.delete(main, main, [])
    assert principal.value.status == 404


def test_delete_dirty_needs_confirm_and_keeps_unmerged_branch(tmp_path, monkeypatch):
    main = _repo(tmp_path / "repo")
    wt = _wt(main, tmp_path / "repo-x", "x")
    _claude_project(tmp_path, monkeypatch, wt)
    _commit(tmp_path / "repo-x", "a.txt")
    (tmp_path / "repo-x" / "solto.txt").write_text("?")
    with pytest.raises(GitError) as e:
        worktrees.delete(main, wt, [])
    assert e.value.status == 409
    assert (tmp_path / "repo-x").exists()
    out = worktrees.delete(main, wt, [], confirm=True)
    assert out["branch_deleted"] is False
    assert "x" in git_ops.list_branches(main)["branches"]


def test_delete_refuses_degraded_even_confirmed(tmp_path, monkeypatch):
    main = _repo(tmp_path / "repo")
    wt = _wt(main, tmp_path / "repo-x", "x")
    real_run = worktrees._run

    def slow(cwd, *args, **kw):
        if args[0] == "status":
            raise GitError(504, "git timeout")
        return real_run(cwd, *args, **kw)
    monkeypatch.setattr(worktrees, "_run", slow)
    with pytest.raises(GitError) as e:
        worktrees.delete(main, wt, [], confirm=True)
    assert e.value.status == 409
    assert (tmp_path / "repo-x").exists()


def test_delete_folder_already_gone(tmp_path, monkeypatch):
    import shutil
    main = _repo(tmp_path / "repo")
    wt = _wt(main, tmp_path / "repo-x", "x")
    _claude_project(tmp_path, monkeypatch, wt)
    shutil.rmtree(wt)
    out = worktrees.delete(main, wt, [])
    assert out["removed"] == wt
    assert worktrees.worktree_paths(main) == []


def test_delete_does_not_overwrite_same_uuid(tmp_path, monkeypatch):
    main = _repo(tmp_path / "repo")
    wt = _wt(main, tmp_path / "repo-x", "x")
    base = _claude_project(tmp_path, monkeypatch, wt)
    from app.registry import sanitize_cwd
    (base / sanitize_cwd(main)).mkdir()
    (base / sanitize_cwd(main) / "abc.jsonl").write_text("principal\n")
    out = worktrees.delete(main, wt, [])
    assert out["moved"] == 0
    assert (base / sanitize_cwd(main) / "abc.jsonl").read_text() == "principal\n"
    assert (base / sanitize_cwd(wt) / "abc.jsonl").exists()


def test_delete_merged_only_takes_clean(tmp_path, monkeypatch):
    main = _repo(tmp_path / "repo")
    a = _merged_wt(main, tmp_path / "repo-a", "a")
    _merged_wt(main, tmp_path / "repo-b", "b")
    _claude_project(tmp_path, monkeypatch, a)
    (tmp_path / "repo-b" / "solto.txt").write_text("?")
    assert worktrees.delete_merged(main, []) == [a]
    assert (tmp_path / "repo-b").exists()
