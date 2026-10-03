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
