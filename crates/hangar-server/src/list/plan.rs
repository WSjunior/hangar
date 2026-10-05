//! Progresso do plano do superpowers que a sessão executa (porte de `backend/app/planprog.py`,
//! só o que a lista lê). Seletor, pin, marcar e arquivar continuam no Python.
//!
//! Roda por sessão a cada rodada da lista, então tudo é cacheado e nada levanta: uma falha aqui
//! derrubaria a lista inteira. Arquivo ilegível é avisado no log e o plano some da linha.
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use hangar_api::session::SessionRow;
use regex::bytes::Regex;

/// Quantos níveis subir do cwd atrás da pasta de planos; a subida para no primeiro `.git`, senão
/// uma worktree sem planos mostraria o plano do checkout principal.
const MAX_PARENTS: usize = 6;
/// Plano parado há mais de 14 dias não reaparece.
const MAX_AGE_S: f64 = 14.0 * 86400.0;
const DISCOVERY_TTL: f64 = 3.0;
const PIN_FILE: &str = "cp-plan-pin";
const PIN_NONE: &str = "!none";
/// `registry._MAX_PLAN_TASK_SEGMENTS`: a barra segmentada não precisa de mais.
const MAX_TASK_SEGMENTS: usize = 9;

static STEP_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^- \[([ xX])\] \*\*Step\b[^*]*\*\*").unwrap());
static TASK_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^### Task\b").unwrap());
static DATE_PREFIX_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^\d{4}-\d{2}-\d{2}-").unwrap());

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub name: String,
    /// Posição ordinal (1-based) da primeira Task com step pendente.
    pub task_idx: u32,
    pub task_total: u32,
    pub done: u32,
    pub total: u32,
    pub complete: bool,
    /// `(feitos, total)` por Task.
    pub tasks: Vec<(u32, u32)>,
}

#[derive(Default)]
pub struct PlanTracker {
    /// (arquivo, exige começado) → (mtime, plano). O `None` memoriza "li e não serve".
    files: HashMap<(PathBuf, bool), (SystemTime, Option<Plan>)>,
    /// pasta de planos → (relógio monotônico, eleito).
    discovery: HashMap<PathBuf, (f64, Option<PathBuf>)>,
    /// Eleito anterior: mantém o posto enquanto tiver step pendente.
    sticky: HashMap<PathBuf, PathBuf>,
}

impl PlanTracker {
    /// `_decorate_plan`: preenche `plan_*` da linha, ou `plan_hidden` quando o repo escondeu.
    pub fn decorate(&mut self, row: &mut SessionRow, wall: f64, mono: f64) {
        let Some(root) = row.cwd.as_deref().filter(|c| !c.is_empty()).and_then(|c| plans_dir(Path::new(c)))
        else {
            return;
        };
        let pin = read_pin(&root);
        if pin.as_deref() == Some(PIN_NONE) {
            row.plan_hidden = Some(true);
            return;
        }
        let Some(p) = self.progress(&root, pin.as_deref(), wall, mono) else { return };
        row.plan_name = Some(p.name);
        row.plan_task = Some(p.task_idx);
        row.plan_task_total = Some(p.task_total);
        row.plan_done = Some(p.done);
        row.plan_total = Some(p.total);
        row.plan_complete = Some(p.complete);
        row.plan_tasks = Some(p.tasks.into_iter().take(MAX_TASK_SEGMENTS).collect());
    }

    fn progress(&mut self, root: &Path, pin: Option<&str>, wall: f64, mono: f64) -> Option<Plan> {
        // Pin vence a eleição só enquanto tiver step pendente; completo, volta ao automático.
        if let Some(stem) = pin {
            let path = root.join(format!("{stem}.md"));
            if let Ok(Some(p)) = mtime(&path).and_then(|m| self.load(&path, m, false)) {
                if !p.complete {
                    return Some(p);
                }
            }
        }
        let path = match self.discovery.get(root) {
            Some((at, path)) if mono - at < DISCOVERY_TTL => path.clone(),
            _ => match self.discover(root, wall) {
                Ok(path) => {
                    self.discovery.insert(root.to_path_buf(), (mono, path.clone()));
                    path
                }
                Err(e) => {
                    tracing::warn!(root = %root.display(), error = %e, "pasta de planos ilegivel");
                    return None;
                }
            },
        }?;
        let m = mtime(&path).ok()?;
        match self.load(&path, m, true) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "plano ilegivel");
                None
            }
        }
    }

    fn load(&mut self, path: &Path, mtime: SystemTime, require_started: bool) -> io::Result<Option<Plan>> {
        // A chave leva o require_started: o mesmo arquivo dá None na eleição e plano pelo pin.
        let key = (path.to_path_buf(), require_started);
        if let Some((m, p)) = self.files.get(&key) {
            if *m == mtime {
                return Ok(p.clone());
            }
        }
        let got = parse(&fs::read(path)?, path, require_started);
        self.files.insert(key, (mtime, got.clone()));
        Ok(got)
    }

    /// Plano com step pendente de mtime mais novo; o eleito anterior fica enquanto andar.
    fn discover(&mut self, root: &Path, wall: f64) -> io::Result<Option<PathBuf>> {
        let mut cands: Vec<(SystemTime, PathBuf, bool)> = Vec::new();
        for entry in fs::read_dir(root)? {
            let path = entry?.path();
            if !path.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".md")) {
                continue;
            }
            // Segue link como o `is_file()` do Python; erro aqui é "não é arquivo".
            let Ok(meta) = fs::metadata(&path) else { continue };
            if !meta.is_file() {
                continue;
            }
            let m = meta.modified()?;
            if wall - secs(m) > MAX_AGE_S {
                continue;
            }
            match self.load(&path, m, true) {
                Ok(Some(p)) => cands.push((m, path, p.complete)),
                Ok(None) => {}
                // Um arquivo ilegível não pode apagar o plano do repo inteiro.
                Err(e) => tracing::warn!(path = %path.display(), error = %e, "plano ilegivel"),
            }
        }
        if let Some(prev) = self.sticky.get(root) {
            if cands.iter().any(|(_, p, complete)| p == prev && !complete) {
                return Ok(Some(prev.clone()));
            }
        }
        let any_pending = cands.iter().any(|c| !c.2);
        let mut chosen: Option<&(SystemTime, PathBuf, bool)> = None;
        for c in cands.iter().filter(|c| !any_pending || !c.2) {
            if chosen.is_none_or(|best| c.0 > best.0) {
                chosen = Some(c);
            }
        }
        let chosen = chosen.map(|c| c.1.clone());
        if let Some(path) = &chosen {
            self.sticky.insert(root.to_path_buf(), path.clone());
        }
        Ok(chosen)
    }
}

fn mtime(path: &Path) -> io::Result<SystemTime> {
    fs::metadata(path)?.modified()
}

fn secs(t: SystemTime) -> f64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs_f64(),
        Err(e) => -e.duration().as_secs_f64(),
    }
}

/// `docs/superpowers/plans` subindo do cwd, parando no primeiro nível com `.git`.
fn plans_dir(cwd: &Path) -> Option<PathBuf> {
    let mut cur = cwd;
    for _ in 0..=MAX_PARENTS {
        let cand = cur.join("docs").join("superpowers").join("plans");
        if cand.is_dir() {
            return Some(cand);
        }
        if cur.join(".git").exists() {
            return None;
        }
        cur = cur.parent()?;
    }
    None
}

/// Dentro do `.git/` do repo dono dos planos; `.git` arquivo (worktree) cai na pasta de planos.
fn pin_path(root: &Path) -> PathBuf {
    let mut cur = Some(root);
    for _ in 0..=MAX_PARENTS {
        let Some(dir) = cur else { break };
        let git = dir.join(".git");
        if git.is_dir() {
            return git.join(PIN_FILE);
        }
        cur = dir.parent();
    }
    root.join(format!(".{PIN_FILE}"))
}

/// Stem fixado, ou None. Pin ilegível, inseguro ou de `.md` que sumiu é sem pin.
fn read_pin(root: &Path) -> Option<String> {
    let raw = fs::read(pin_path(root)).ok()?;
    let v = String::from_utf8_lossy(&raw).trim().to_owned();
    if v.is_empty() || v.contains(['/', '\\']) || v == "." || v == ".." {
        return None;
    }
    if v == PIN_NONE || root.join(format!("{v}.md")).is_file() {
        return Some(v);
    }
    None
}

/// Blocos cercados (``` ou ~~~) viram espaço, `\n` mantido: planos mostram steps de exemplo
/// dentro de bloco de código. Equivale a `^(```|~~~).*?^\1[^\n]*$` (M|S), sem retrorreferência.
fn blank_fences(b: &mut [u8]) {
    let eol = |b: &[u8], from: usize| b[from..].iter().position(|&c| c == b'\n').map_or(b.len(), |p| from + p);
    let mut i = 0;
    while i < b.len() {
        let marker: Option<&[u8]> = [b"```".as_slice(), b"~~~"].into_iter().find(|m| b[i..].starts_with(m));
        if let Some(m) = marker {
            let mut j = eol(b, i) + 1;
            while j < b.len() && !b[j..].starts_with(m) {
                j = eol(b, j) + 1;
            }
            if j < b.len() {
                let end = eol(b, j);
                b[i..end].iter_mut().filter(|c| **c != b'\n').for_each(|c| *c = b' ');
                i = end + 1;
                continue;
            }
        }
        i = eol(b, i) + 1;
    }
}

/// `parse_plan`: None sem step nenhum ou, com `require_started`, sem step marcado.
fn parse(raw: &[u8], path: &Path, require_started: bool) -> Option<Plan> {
    let mut text = String::from_utf8_lossy(raw).into_owned().into_bytes();
    blank_fences(&mut text);
    let steps: Vec<(usize, bool)> = STEP_RE
        .captures_iter(&text)
        .map(|c| (c.get(0).map_or(0, |m| m.start()), &c[1] != b" "))
        .collect();
    if steps.is_empty() {
        return None;
    }
    let done = steps.iter().filter(|s| s.1).count();
    if done == 0 && require_started {
        return None;
    }
    let mut heads: Vec<usize> = TASK_RE.find_iter(&text).map(|m| m.start()).collect();
    // Steps antes da 1ª Task ganham uma Task implícita, senão somem da barra segmentada.
    if heads.first().is_none_or(|&h| h > 0 && steps.iter().any(|s| s.0 < h)) {
        heads.insert(0, 0);
    }
    let tasks: Vec<(u32, u32)> = heads
        .iter()
        .enumerate()
        .map(|(i, &pos)| {
            let end = heads.get(i + 1).copied().unwrap_or(text.len());
            let mine = steps.iter().filter(|s| pos <= s.0 && s.0 < end);
            let (d, t) = mine.fold((0, 0), |(d, t), s| (d + u32::from(s.1), t + 1));
            (d, t)
        })
        .collect();
    // Ordinal, não o N do título: existe "### Task 0".
    let task_idx = tasks.iter().position(|(d, t)| d < t).map_or(tasks.len(), |i| i + 1);
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    Some(Plan {
        name: DATE_PREFIX_RE.replace(&stem, "").into_owned(),
        task_idx: task_idx as u32,
        task_total: tasks.len() as u32,
        done: done as u32,
        total: steps.len() as u32,
        complete: done == steps.len(),
        tasks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(text: &str) -> Option<Plan> {
        parse(text.as_bytes(), Path::new("/x/2027-01-01-nome.md"), true)
    }

    #[test]
    fn fenced_steps_and_loose_steps() {
        let p = plan(
            "- [x] **Step 0: solto**\n\n### Task 1: A\n\n```md\n- [x] **Step 9: exemplo**\n```\n\
             - [ ] **Step 1: a**\n\n~~~\n- [x] **Step 8: aberto sem fechar**\n",
        )
        .unwrap();
        assert_eq!(p.name, "nome");
        assert_eq!((p.done, p.total), (2, 3)); // o ~~~ sem fechamento não esconde nada
        assert_eq!(p.tasks, vec![(1, 1), (1, 2)]);
        assert_eq!((p.task_idx, p.task_total), (2, 2));
        assert_eq!(plan("```\n- [x] **Step 1: a**\n```\n- [ ] **Step 2: b**\n"), None);
    }

    #[test]
    fn sticky_and_stop_at_git() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let plans = repo.join("docs/superpowers/plans");
        fs::create_dir_all(&plans).unwrap();
        fs::create_dir_all(repo.join(".git")).unwrap();
        let wt = repo.join("wt");
        fs::create_dir_all(&wt).unwrap();
        fs::write(wt.join(".git"), "gitdir: x").unwrap();
        let now = secs(SystemTime::now());
        let write = |name: &str, body: &str, age: f64| {
            let p = plans.join(name);
            fs::write(&p, body).unwrap();
            let at = UNIX_EPOCH + std::time::Duration::from_secs_f64(now - age);
            fs::File::options().write(true).open(&p).unwrap().set_modified(at).unwrap();
        };
        write("2027-01-01-a.md", "- [x] **Step 1: a**\n- [ ] **Step 2: b**\n", 10.0);
        let mut t = PlanTracker::default();
        let mut row: SessionRow = serde_json::from_value(serde_json::json!({"name": "s",
            "cwd": repo.join("src").to_str().unwrap()}))
        .unwrap();
        t.decorate(&mut row, now, 100.0);
        assert_eq!(row.plan_name.as_deref(), Some("a"));
        write("2027-01-02-b.md", "- [x] **Step 1: a**\n- [ ] **Step 2: b**\n", 1.0);
        let mut row2 = row.clone();
        t.decorate(&mut row2, now, 104.0);
        assert_eq!(row2.plan_name.as_deref(), Some("a"), "sticky");
        let mut inside: SessionRow =
            serde_json::from_value(serde_json::json!({"name": "w", "cwd": wt.to_str().unwrap()})).unwrap();
        t.decorate(&mut inside, now, 104.0);
        assert_eq!(inside.plan_name, None, "worktree não sobe ao checkout principal");
    }
}
