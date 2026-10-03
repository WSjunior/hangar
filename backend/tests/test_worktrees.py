import json

import pytest

from app import git_ops, worktrees


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
