//! Edição do agente na pasta do Hangar é desfeita no fim (spec "Edição na pasta do Hangar é desfeita no fim"): antes do
//! agente o app anota o `git status` e o conteúdo de cada arquivo já mexido; no fim devolve ao original só o que o agente
//! mudou. Arquivo ignorado pelo git (`.venv`, `node_modules`, `backend/.env`) fica fora: é do instalador, não do código.
use std::{collections::{BTreeMap, BTreeSet}, path::{Path, PathBuf}, process::{Command, Stdio}, sync::atomic::{AtomicU64, Ordering}};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use super::system::{find_program, hidden, refreshed_path};

const FILE: &str = "agent-snapshot.json";
const STATUS: [&str; 4] = ["status", "--porcelain=v1", "-z", "--untracked-files=all"];
static DIFFS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Entry {
    pub status: String,
    /// Conteúdo antes do agente, em base64; `None` = o arquivo não existia.
    pub content: Option<String>,
    /// Nada no índice antes do agente: o que ele puser lá volta junto.
    pub index_clean: bool,
    /// "modo,hash" do índice antes do agente, para quem já tinha algo em stage (`None` = nada ou apagado em stage).
    #[serde(default)]
    pub index: Option<String>,
    /// Permissões (unix) antes do agente.
    #[serde(default)]
    pub mode: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Snapshot {
    pub dir: PathBuf,
    pub head: String,
    pub before: BTreeMap<String, Entry>,
    /// Gravado depois que o agente sobe: a abertura seguinte a um app que caiu pára quem sobrou.
    pub agent_pid: Option<u32>,
    /// Instante de início do processo do agente: pid reaproveitado nunca é parado (`run::alive/stop`).
    #[serde(default)]
    pub agent_started: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Restored {
    pub diff: String,
    pub changed: Vec<String>,
    pub head_moved: Option<(String, String)>,
    pub errors: Vec<String>,
}

struct Git { exe: PathBuf, path: String }

impl Git {
    fn new() -> Result<Self, String> {
        let path = refreshed_path();
        Ok(Self { exe: find_program("git", &path).ok_or("git")?, path })
    }

    fn output(&self, dir: &Path, args: &[&str]) -> Result<std::process::Output, String> {
        hidden(&mut Command::new(&self.exe)).arg("-C").arg(dir).args(args).env("PATH", &self.path)
            .env("GIT_TERMINAL_PROMPT", "0").env("GIT_LITERAL_PATHSPECS", "1").env("GIT_OPTIONAL_LOCKS", "0").stdin(Stdio::null()).output().map_err(|e| e.to_string())
    }

    fn run(&self, dir: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
        let out = self.output(dir, args)?;
        if out.status.success() { Ok(out.stdout) } else { Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim())) }
    }

    fn head(&self, dir: &Path) -> Result<String, String> { Ok(String::from_utf8_lossy(&self.run(dir, &["rev-parse", "HEAD"])?).trim().to_owned()) }

    fn status(&self, dir: &Path) -> Result<BTreeMap<String, String>, String> { Ok(parse_status(&self.run(dir, &STATUS)?).into_iter().collect()) }
}

/// `git status --porcelain=v1 -z`: "XY caminho\0"; renomeado traz depois a origem, apagada em stage ("D ").
pub(crate) fn parse_status(raw: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut parts = raw.split(|b| *b == 0).filter(|p| !p.is_empty());
    while let Some(part) = parts.next() {
        if part.len() < 4 { continue; }
        out.push((String::from_utf8_lossy(&part[3..]).into_owned(), String::from_utf8_lossy(&part[..2]).into_owned()));
        if matches!(part[0], b'R' | b'C') && let Some(origin) = parts.next() {
            out.push((String::from_utf8_lossy(origin).into_owned(), "D ".to_owned()));
        }
    }
    out
}

pub(crate) fn snapshot(dir: &Path) -> Result<Snapshot, String> {
    let git = Git::new()?;
    let head = git.head(dir)?;
    let before = git.status(dir)?.into_iter().map(|(path, status)| {
        let content = std::fs::read(dir.join(&path)).ok().map(|bytes| STANDARD.encode(bytes));
        let index_clean = matches!(status.as_bytes()[0], b' ' | b'?');
        let index = if index_clean { None } else { index_entry(&git, dir, &path) };
        let mode = file_mode(&dir.join(&path));
        (path, Entry { status, content, index_clean, index, mode })
    }).collect();
    Ok(Snapshot { dir: dir.to_owned(), head, before, agent_pid: None, agent_started: String::new() })
}

pub(crate) fn restore(s: &Snapshot) -> Restored {
    let mut out = Restored::default();
    let git = match Git::new() { Ok(git) => git, Err(e) => { out.errors.push(e); return out; } };
    let mut after = match git.status(&s.dir) { Ok(after) => after, Err(e) => { out.errors.push(e); return out; } };
    if let Ok(head) = git.head(&s.dir) && head != s.head { out.head_moved = Some((s.head.clone(), head)); }
    // `.gitignore` primeiro: desfeito, o que o agente des-ignorou volta a ser ignorado e sai da lista.
    let all: BTreeSet<String> = s.before.keys().chain(after.keys()).cloned().collect();
    let rules: Vec<String> = all.into_iter().filter(|p| p.rsplit('/').next() == Some(".gitignore")).collect();
    for path in &rules { undo(&git, s, path, after.get(path).map_or("  ", String::as_str), &mut out); }
    if !rules.is_empty() {
        match git.status(&s.dir) { Ok(now) => after = now, Err(e) => { out.errors.push(e); return out; } }
    }
    // Lista nova: o que o agente des-ignorou já não aparece e fica como está.
    let paths: BTreeSet<&String> = s.before.keys().chain(after.keys()).filter(|p| !rules.contains(p)).collect();
    for path in paths { undo(&git, s, path, after.get(path).map_or("  ", String::as_str), &mut out); }
    // Conferência: a pasta tem de voltar ao `git status` de antes do agente.
    match git.status(&s.dir) {
        Ok(now) => {
            let then: BTreeMap<String, String> = s.before.iter().map(|(p, e)| (p.clone(), e.status.clone())).collect();
            let off: Vec<&String> = now.keys().chain(then.keys()).filter(|p| now.get(*p) != then.get(*p)).collect();
            if !off.is_empty() { out.errors.push(format!("git status: {off:?}")); }
        }
        Err(e) => out.errors.push(e),
    }
    out
}

/// Devolve UM caminho ao estado de antes do agente (arquivo, modo e índice). `now` = status atual dele.
fn undo(git: &Git, s: &Snapshot, path: &str, now: &str, out: &mut Restored) {
    let entry = s.before.get(path);
    let in_head = git.run(&s.dir, &["cat-file", "-e", &format!("{}:{path}", s.head)]).is_ok();
    // O alvo é o conteúdo de antes do agente: o anotado, ou o do commit para quem estava limpo.
    let target = match entry {
        Some(e) => e.content.as_ref().and_then(|c| STANDARD.decode(c).ok()),
        None if in_head => git.run(&s.dir, &["show", &format!("{}:{path}", s.head)]).ok(),
        None => None,
    };
    let file = s.dir.join(path);
    let current = std::fs::read(&file).ok();
    let then = entry.map_or("  ", |e| e.status.as_str());
    let index_ok = entry.is_none_or(|e| e.index_clean || index_entry(git, &s.dir, path) == e.index);
    let mode_ok = entry.is_none_or(|e| e.mode.is_none() || file_mode(&file) == e.mode);
    if current == target && now == then && index_ok && mode_ok { return; }
    // Existia antes do agente e é ignorado: é do instalador (ex.: `backend/.env`), nunca se apaga.
    let ignored = entry.is_none() && !in_head && git.run(&s.dir, &["check-ignore", "-q", "--no-index", "--", path]).is_ok();
    if current != target && !ignored { out.diff.push_str(&diff(git, path, target.as_deref(), current.as_deref())); }
    out.changed.push(path.to_owned());
    let index = match entry {
        // Limpo antes: o commit traz modo e fim de linha certos.
        None if in_head => git.run(&s.dir, &["checkout", "-q", &s.head, "--", path]),
        None => {
            if !ignored && let Err(e) = put(&file, None) { out.errors.push(format!("{path}: {e}")); return; }
            git.run(&s.dir, &["rm", "--cached", "-q", "--ignore-unmatch", "--", path])
        }
        Some(e) => {
            if let Err(e) = put(&file, target.as_deref()) { out.errors.push(format!("{path}: {e}")); return; }
            if let Some(mode) = e.mode { set_mode(&file, mode); }
            match (&e.index, e.index_clean) {
                (Some(spec), false) => git.run(&s.dir, &["update-index", "--add", "--cacheinfo", &format!("{spec},{path}")]),
                (None, false) => git.run(&s.dir, &["rm", "--cached", "-q", "--ignore-unmatch", "--", path]),
                (_, true) if in_head => git.run(&s.dir, &["reset", "-q", &s.head, "--", path]),
                (_, true) => git.run(&s.dir, &["rm", "--cached", "-q", "--ignore-unmatch", "--", path]),
            }
        }
    };
    if let Err(e) = index { out.errors.push(e); }
}

/// "modo,hash" do caminho no índice; `None` = fora do índice.
fn index_entry(git: &Git, dir: &Path, path: &str) -> Option<String> {
    let raw = git.run(dir, &["ls-files", "-s", "-z", "--", path]).ok()?;
    let line = String::from_utf8_lossy(raw.split(|b| *b == b'\t').next()?).into_owned();
    let mut parts = line.split_whitespace();
    Some(format!("{},{}", parts.next()?, parts.next()?))
}

#[cfg(unix)]
fn file_mode(path: &Path) -> Option<u32> { use std::os::unix::fs::PermissionsExt; Some(std::fs::metadata(path).ok()?.permissions().mode() & 0o777) }
#[cfg(not(unix))]
fn file_mode(_: &Path) -> Option<u32> { None }

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)); }
#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32) {}

fn put(path: &Path, bytes: Option<&[u8]>) -> std::io::Result<()> {
    match bytes {
        Some(bytes) => {
            if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
            std::fs::write(path, bytes)
        }
        None => match std::fs::remove_file(path) { Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()), other => other },
    }
}

/// Diff unificado pelo próprio git (`--no-index`, que funciona fora de repositório), com o nome do arquivo no cabeçalho.
fn diff(git: &Git, path: &str, before: Option<&[u8]>, after: Option<&[u8]>) -> String {
    let tmp = std::env::temp_dir().join(format!("hangar-diff-{}-{}", std::process::id(), DIFFS.fetch_add(1, Ordering::Relaxed)));
    let written = std::fs::create_dir_all(&tmp).and_then(|_| std::fs::write(tmp.join("a"), before.unwrap_or_default()))
        .and_then(|_| std::fs::write(tmp.join("b"), after.unwrap_or_default()));
    // `git diff --no-index` sai 1 quando há diferença: vale a saída, não o código. Prefixos e diff externo fixos: a
    // configuração da pessoa (`diff.mnemonicPrefix`, `diff.external`) mudaria o cabeçalho que é reescrito abaixo.
    let args = ["diff", "--no-index", "--no-color", "--no-ext-diff", "--src-prefix=a/", "--dst-prefix=b/", "--", "a", "b"];
    let text = match written.map_err(|e| e.to_string()).and_then(|_| git.output(&tmp, &args)) {
        Ok(out) => String::from_utf8_lossy(&out.stdout).into_owned(),
        Err(e) => format!("{path}: {e}\n"),
    };
    let _ = std::fs::remove_dir_all(&tmp);
    let old = if before.is_some() { format!("a/{path}") } else { "/dev/null".to_owned() };
    let new = if after.is_some() { format!("b/{path}") } else { "/dev/null".to_owned() };
    text.lines().map(|line| match line {
        "diff --git a/a b/b" => format!("diff --git a/{path} b/{path}"),
        "--- a/a" => format!("--- {old}"),
        "+++ b/b" => format!("+++ {new}"),
        other => other.to_owned(),
    }).map(|line| line + "\n").collect()
}

pub(crate) fn save_at(dir: &Path, snapshot: &Snapshot) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join("agent-snapshot.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    // Leva o conteúdo de arquivos da pessoa: só ela lê.
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
    std::io::Write::write_all(&mut options.open(&tmp)?, serde_json::to_string(snapshot).map_err(std::io::Error::other)?.as_bytes())?;
    std::fs::rename(tmp, dir.join(FILE))
}

pub(crate) fn load_at(dir: &Path) -> Option<Snapshot> { serde_json::from_slice(&std::fs::read(dir.join(FILE)).ok()?).ok() }

pub(crate) fn clear_at(dir: &Path) { let _ = std::fs::remove_file(dir.join(FILE)); }

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(dir: &Path, args: &[&str]) {
        let out = Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"])
            .args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    fn repo(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hangar-repo-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        sh(&dir, &["init", "-q"]);
        std::fs::write(dir.join("install.sh"), "echo 1\n").unwrap();
        std::fs::write(dir.join("README.md"), "leia\n").unwrap();
        std::fs::write(dir.join(".gitignore"), "backend/.env\n").unwrap();
        sh(&dir, &["add", "."]);
        sh(&dir, &["commit", "-q", "-m", "base"]);
        dir
    }

    fn read(dir: &Path, path: &str) -> Option<String> { std::fs::read_to_string(dir.join(path)).ok() }

    fn status(dir: &Path) -> String {
        String::from_utf8(Command::new("git").arg("-C").arg(dir).args(["status", "--porcelain"]).output().unwrap().stdout).unwrap()
    }

    /// A pessoa mexeu antes; o agente mexe em tudo.
    fn agent_edits(dir: &Path) {
        std::fs::write(dir.join("install.sh"), "echo 2\n").unwrap();
        sh(dir, &["add", "install.sh"]);
        std::fs::write(dir.join("README.md"), "leia\nminha nota\ndo agente\n").unwrap();
        std::fs::write(dir.join("novo.sh"), "novo\n").unwrap();
        std::fs::remove_file(dir.join("notas.txt")).unwrap();
        std::fs::write(dir.join("backend/.env"), "CP_PORT=10\n").unwrap();
    }

    fn person_edits(dir: &Path) {
        std::fs::write(dir.join("README.md"), "leia\nminha nota\n").unwrap();
        std::fs::write(dir.join("notas.txt"), "minhas\n").unwrap();
        std::fs::create_dir_all(dir.join("backend")).unwrap();
        std::fs::write(dir.join("backend/.env"), "CP_PORT=9\n").unwrap();
    }

    #[test]
    fn restore_undoes_only_what_the_agent_changed() {
        let dir = repo("undo");
        person_edits(&dir);
        let before = status(&dir);
        let snap = snapshot(&dir).unwrap();
        agent_edits(&dir);
        let restored = restore(&snap);
        assert!(restored.errors.is_empty(), "{:?}", restored.errors);
        assert_eq!(read(&dir, "install.sh").as_deref(), Some("echo 1\n"));
        assert_eq!(read(&dir, "README.md").as_deref(), Some("leia\nminha nota\n"));
        assert_eq!(read(&dir, "notas.txt").as_deref(), Some("minhas\n"));
        assert_eq!(read(&dir, "novo.sh"), None);
        // Ignorado pelo git é do instalador: fica como o agente deixou.
        assert_eq!(read(&dir, "backend/.env").as_deref(), Some("CP_PORT=10\n"));
        assert_eq!(status(&dir), before);
        assert_eq!(restored.changed, vec!["README.md", "install.sh", "notas.txt", "novo.sh"]);
        for needle in ["a/install.sh", "+echo 2", "+do agente", "-minhas", "+novo"] {
            assert!(restored.diff.contains(needle), "{needle} faltou no diff:\n{}", restored.diff);
        }
        assert_eq!(restored.head_moved, None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn untouched_folder_has_no_diff() {
        let dir = repo("untouched");
        person_edits(&dir);
        let snap = snapshot(&dir).unwrap();
        let restored = restore(&snap);
        assert_eq!(restored, Restored::default());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn saved_snapshot_restores_after_a_reload() {
        let (dir, state) = (repo("reload"), std::env::temp_dir().join(format!("hangar-repo-state-{}", std::process::id())));
        person_edits(&dir);
        let mut snap = snapshot(&dir).unwrap();
        snap.agent_pid = Some(4242);
        snap.agent_started = "12345".to_owned();
        save_at(&state, &snap).unwrap();
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(state.join(FILE)).unwrap().permissions().mode() & 0o777, 0o600);
        }
        agent_edits(&dir);
        // O app caiu: a próxima abertura só tem o arquivo.
        let loaded = load_at(&state).unwrap();
        assert_eq!(loaded, snap);
        assert!(restore(&loaded).errors.is_empty());
        assert_eq!(read(&dir, "install.sh").as_deref(), Some("echo 1\n"));
        clear_at(&state);
        assert_eq!(load_at(&state), None);
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(state);
    }

    #[test]
    fn unignored_env_is_never_deleted() {
        for force_add in [false, true] {
            let dir = repo(if force_add { "envadd" } else { "envgit" });
            person_edits(&dir);
            let before = status(&dir);
            let snap = snapshot(&dir).unwrap();
            if force_add { sh(&dir, &["add", "-f", "backend/.env"]); } else { std::fs::write(dir.join(".gitignore"), "").unwrap(); }
            let restored = restore(&snap);
            assert!(restored.errors.is_empty(), "{:?}", restored.errors);
            assert_eq!(read(&dir, "backend/.env").as_deref(), Some("CP_PORT=9\n"));
            assert_eq!(read(&dir, ".gitignore").as_deref(), Some("backend/.env\n"));
            assert_eq!(status(&dir), before);
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[cfg(unix)]
    #[test]
    fn exec_bit_survives_delete_and_chmod() {
        use std::os::unix::fs::PermissionsExt;
        let dir = repo("mode");
        for f in ["run.sh", "mine.sh"] {
            std::fs::write(dir.join(f), "x\n").unwrap();
            std::fs::set_permissions(dir.join(f), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        sh(&dir, &["add", "run.sh", "mine.sh"]);
        sh(&dir, &["commit", "-q", "-m", "scripts"]);
        std::fs::write(dir.join("mine.sh"), "x\nminha\n").unwrap();
        let before = status(&dir);
        let snap = snapshot(&dir).unwrap();
        std::fs::remove_file(dir.join("run.sh")).unwrap();
        std::fs::write(dir.join("mine.sh"), "agente\n").unwrap();
        std::fs::set_permissions(dir.join("mine.sh"), std::fs::Permissions::from_mode(0o644)).unwrap();
        let restored = restore(&snap);
        assert!(restored.errors.is_empty(), "{:?}", restored.errors);
        for f in ["run.sh", "mine.sh"] { assert_eq!(std::fs::metadata(dir.join(f)).unwrap().permissions().mode() & 0o777, 0o755, "{f}"); }
        assert_eq!(read(&dir, "mine.sh").as_deref(), Some("x\nminha\n"));
        assert_eq!(status(&dir), before);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn glob_characters_in_names_are_literal() {
        let dir = repo("glob");
        std::fs::write(dir.join("a*[1].txt"), "base\n").unwrap();
        sh(&dir, &["add", "a*[1].txt"]);
        sh(&dir, &["commit", "-q", "-m", "glob"]);
        let snap = snapshot(&dir).unwrap();
        std::fs::write(dir.join("a*[1].txt"), "agente\n").unwrap();
        sh(&dir, &["add", "a*[1].txt"]);
        let restored = restore(&snap);
        assert!(restored.errors.is_empty(), "{:?}", restored.errors);
        assert_eq!(read(&dir, "a*[1].txt").as_deref(), Some("base\n"));
        assert_eq!(status(&dir), "");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn staged_content_of_the_person_comes_back() {
        let dir = repo("staged");
        std::fs::write(dir.join("README.md"), "leia\nstage\n").unwrap();
        sh(&dir, &["add", "README.md"]);
        std::fs::write(dir.join("README.md"), "leia\nstage\ndepois\n").unwrap();
        let before = status(&dir);
        let snap = snapshot(&dir).unwrap();
        sh(&dir, &["reset", "--hard"]);
        let restored = restore(&snap);
        assert!(restored.errors.is_empty(), "{:?}", restored.errors);
        assert_eq!(read(&dir, "README.md").as_deref(), Some("leia\nstage\ndepois\n"));
        let staged = Command::new("git").arg("-C").arg(&dir).args(["show", ":README.md"]).output().unwrap().stdout;
        assert_eq!(String::from_utf8(staged).unwrap(), "leia\nstage\n");
        assert_eq!(status(&dir), before);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn status_parser_reads_spaces_and_renames() {
        let raw = b" M a b.txt\0?? novo\0R  novo.sh\0velho.sh\0";
        assert_eq!(parse_status(raw), vec![("a b.txt".to_owned(), " M".to_owned()), ("novo".into(), "??".into()),
            ("novo.sh".into(), "R ".into()), ("velho.sh".into(), "D ".into())]);
    }
}
