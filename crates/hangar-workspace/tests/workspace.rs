use hangar_workspace::{Operation, execute};
use serde_json::{Value, json};
use std::{fs, process::Command};

fn git(dir: &std::path::Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn op(cwd: &std::path::Path, name: &str, args: Value) -> Value {
    let mut args = args.as_object().unwrap().clone();
    args.insert("cwd".into(), json!(cwd.to_string_lossy()));
    execute(serde_json::from_value::<Operation>(json!({"op": name, "args": args})).unwrap())
        .unwrap()
}

#[test]
fn selected_commit_keeps_other_staged_files_and_read_digest_guards_save() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-b", "main"]);
    git(dir.path(), &["config", "user.name", "Teste"]);
    git(
        dir.path(),
        &["config", "user.email", "teste@example.invalid"],
    );
    fs::write(dir.path().join("ação.txt"), "primeira\n").unwrap();
    fs::write(dir.path().join("outro.txt"), "outro\n").unwrap();
    git(dir.path(), &["add", "."]);
    op(
        dir.path(),
        "commit",
        json!({"message":"Inicial", "paths":["ação.txt"]}),
    );
    assert!(
        op(dir.path(), "changed_files", json!({}))
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["path"] == "outro.txt" && f["staged"] == true)
    );
    let read = op(dir.path(), "read_file", json!({"path":"ação.txt"}));
    let write = op(
        dir.path(),
        "write_file",
        json!({"path":"ação.txt", "texto":"segunda\n", "digest_lido":read["digest"]}),
    );
    assert_ne!(read["digest"], write["digest"]);
    let request = serde_json::from_value(json!({"op":"write_file", "args":{"cwd":dir.path(), "path":"ação.txt", "texto":"perdida", "digest_lido":read["digest"]}})).unwrap();
    assert_eq!(execute(request).unwrap_err().status, 409);
    assert_eq!(
        fs::read_to_string(dir.path().join("ação.txt")).unwrap(),
        "segunda\n"
    );
}

#[test]
fn paths_and_binary_reads_are_refused_without_changing_disk() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join(".git/config"), "segredo").unwrap();
    fs::write(dir.path().join("bin.dat"), "texto\0binário".as_bytes()).unwrap();
    for (path, status) in [("../fora", 400), (".git/config", 403), ("bin.dat", 415)] {
        let request = serde_json::from_value(
            json!({"op":"read_file", "args":{"cwd":dir.path(),"path":path}}),
        )
        .unwrap();
        assert_eq!(execute(request).unwrap_err().status, status);
    }
}

#[cfg(unix)]
#[test]
fn citation_found_elsewhere_through_a_symlink_never_reaches_git_internals() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::write(repo.join(".git/config"), "segredo").unwrap();
    std::os::unix::fs::symlink(".git", repo.join("gl")).unwrap();
    let jsonl = dir.path().join("t.jsonl");
    let cited = repo.join("gl/config");
    fs::write(
        &jsonl,
        json!({"cwd": repo, "text": format!("veja {}", cited.display())}).to_string() + "\n",
    )
    .unwrap();
    for write in [false, true] {
        let error = hangar_workspace::citations::resolve(&repo, hangar_workspace::citations::Transcript::File(&jsonl), "config", write).unwrap_err();
        assert_eq!(error.status, 403);
    }
}

#[test]
fn tilde_of_another_user_stays_relative_and_cannot_climb_out_of_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().join("sessao");
    fs::create_dir_all(&cwd).unwrap();
    let secret = dir.path().join("segredo.txt");
    fs::write(&secret, "x").unwrap();
    let process_cwd = std::env::current_dir().unwrap();
    let climb = "../".repeat(process_cwd.components().count() + 1);
    let path = format!("~ninguem/{climb}{}", secret.display().to_string().trim_start_matches('/'));
    let jsonl = dir.path().join("t.jsonl");
    fs::write(&jsonl, json!({"cwd": cwd, "text": path}).to_string() + "\n").unwrap();
    let error = hangar_workspace::citations::resolve(&cwd, hangar_workspace::citations::Transcript::File(&jsonl), &path, false).unwrap_err();
    assert_eq!(error.status, 403);
    let found = hangar_workspace::files::resolver(&cwd, std::slice::from_ref(&path), false).unwrap();
    assert!(found["ok"].as_object().unwrap().is_empty());
}

#[test]
fn remote_branch_named_like_an_option_is_not_passed_to_switch() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-b", "main"]);
    git(dir.path(), &["-c", "user.name=T", "-c", "user.email=t@t", "commit", "--allow-empty", "-m", "c"]);
    git(dir.path(), &["update-ref", "refs/remotes/origin/--detach", "HEAD"]);
    let request = serde_json::from_value(
        json!({"op":"switch_branch", "args":{"cwd":dir.path(),"branch":"--detach"}}),
    )
    .unwrap();
    assert_eq!(execute(request).unwrap_err().status, 400);
    let head = Command::new("git").arg("-C").arg(dir.path()).args(["symbolic-ref", "-q", "HEAD"]).output().unwrap();
    assert!(head.status.success(), "HEAD ficou destacado");
}

#[test]
fn empty_folder_path_means_the_root_like_python() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("projeto")).unwrap();
    let request = serde_json::from_value(json!({"op":"scan_dir", "args":{
        "root": dir.path(), "path": "", "roots": [dir.path()]}}))
    .unwrap();
    let listed = execute(request).unwrap();
    assert!(listed.to_string().contains("projeto"), "{listed}");
}

#[cfg(unix)]
#[test]
fn listed_symlink_shows_the_size_of_its_target() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("alvo.txt"), "1").unwrap();
    std::os::unix::fs::symlink(dir.path().join("alvo.txt"), dir.path().join("atalho.txt")).unwrap();
    let request = serde_json::from_value(json!({"op":"list_dir", "args":{
        "cwd": dir.path(), "path": null, "so_modificados": false}}))
    .unwrap();
    let listed = execute(request).unwrap();
    let link = listed["entries"].as_array().unwrap().iter().find(|e| e["name"] == "atalho.txt").unwrap().clone();
    assert_eq!(link["size"], 1);
}

#[test]
fn copy_entry_in_status_does_not_misread_its_source_as_an_entry() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    git(dir.path(), &["config", "user.name", "T"]);
    git(dir.path(), &["config", "user.email", "t@t"]);
    git(dir.path(), &["config", "status.renames", "copies"]);
    let lines = (1..=200).map(|n| n.to_string()).collect::<Vec<_>>().join("\n");
    fs::write(dir.path().join("R日本.txt"), &lines).unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "a"]);
    fs::write(dir.path().join("copia.txt"), &lines).unwrap();
    fs::write(dir.path().join("R日本.txt"), format!("{lines}\nmod\n")).unwrap();
    git(dir.path(), &["add", "."]);
    let done = op(dir.path(), "commit", json!({"message":"copia", "paths":["copia.txt"]}));
    assert_eq!(done["ok"], true);
}

/// Repositório recém-criado: o `diff HEAD` sai com erro, mas isso não é falha da consulta (nulo ia ao diário).
#[test]
fn diff_summary_of_repository_without_commits_is_empty_not_failed() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-b", "main"]);
    fs::write(dir.path().join("novo.txt"), "x\n").unwrap();
    let cwd = dir.path().to_string_lossy().into_owned();
    assert_eq!(hangar_workspace::git::summary(Some(&cwd), true), json!({}));
    assert!(!hangar_workspace::git::summary(Some(&cwd), false).is_null());
}
