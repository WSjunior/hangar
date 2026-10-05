//! Descoberta dos provedores fora do Claude com terminal: bilhetes de Pi, omp e Kimi
//! (`registry.py:739-888`) e as linhas dos sidecars Codex e Claude sem terminal (`:1438-1480`).
//! A conta das linhas Pi, omp e Kimi casa credenciais (`cotas.py`) e chega pelos fatos do Python.
use super::discover::{Resolver, discover_panes};
use super::links;
use super::mux::Pane;
use super::procs::{ChildrenMap, ProcessView};
use hangar_api::session::SessionRow;
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::SystemTime;

/// Pastas que o Python tira do ambiente do backend. Em parâmetro, para o teste rodar sem tocar no
/// `HOME` do processo.
#[derive(Clone, Debug)]
pub struct Dirs {
    /// `Path.home()`: sidecars em `~/.hangar` e a conta Claude padrão (`~/.claude`).
    pub home: PathBuf,
    /// `settings.projects_dir.parent`: vínculos (`.hangar-pair`, `-chain`, `-loop`) e o transcript
    /// sem terminal quando o sidecar não declara conta.
    pub claude: PathBuf,
    /// `codex_contas.default_home()`.
    pub codex_home: PathBuf,
    /// `PI_CODING_AGENT_SESSION_DIR` ou `~/.pi/agent/sessions`.
    pub pi_sessions: PathBuf,
    /// `~/<PI_CONFIG_DIR ou .omp>`, base dos perfis do omp.
    pub omp_config: PathBuf,
    /// Diretório do agente omp do próprio backend (sem perfil na sessão).
    pub omp_agent: PathBuf,
    /// `KIMI_CODE_HOME` ou `~/.kimi-code`.
    pub kimi_home: PathBuf,
}

impl Dirs {
    /// Conta Claude padrão das sessões que não declaram `CLAUDE_CONFIG_DIR`.
    pub fn default_claude(&self) -> PathBuf { self.home.join(".claude") }
}

/// Variável do ambiente do processo; vazia é ausência. Ilegível também cai na ausência, como o
/// `_env_var_of`, mas avisa: motor e conta sairiam do padrão nesta rodada.
fn env_os(procs: &dyn ProcessView, pid: i64, name: &str) -> Option<OsString> {
    match procs.env_var(pid, name) {
        Ok(v) => v.filter(|v| !v.is_empty()),
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound && crate::warn_limit::allow(None, "list_environ_unreadable") {
                tracing::warn!(code = "list_environ_unreadable", pid, io_kind = ?error.kind(), "ambiente do processo ilegível");
            }
            None
        }
    }
}

pub(super) fn env(procs: &dyn ProcessView, pid: i64, name: &str) -> Option<String> {
    env_os(procs, pid, name).map(|v| v.to_string_lossy().into_owned())
}

/// Bytes do caminho intactos, como o `surrogateescape` do Python: o caminho tem de existir no disco.
pub(super) fn config_dir_of(procs: &dyn ProcessView, pid: i64) -> Option<PathBuf> {
    env_os(procs, pid, "CLAUDE_CONFIG_DIR").map(PathBuf::from)
}

/// Chave do bilhete, a mesma da extensão: no psmux o `%N` repete entre sessões e vale o
/// `PSMUX_SESSION`.
pub fn ticket_key(pane_id: &str, pid: Option<i64>, procs: &dyn ProcessView) -> String {
    if let Some(psmux) = pid.and_then(|p| env(procs, p, "PSMUX_SESSION")) {
        return psmux.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '-' }).collect();
    }
    pane_id.trim_start_matches('%').to_owned()
}

/// `registry.list()` sem as linhas `orq` e de transferência (vêm dos fatos): sessões do
/// multiplexador, guarda de colisão e as linhas dos sidecars Codex e Claude sem terminal.
pub fn discover_rows(panes: &[Pane], procs: &dyn ProcessView, children: &ChildrenMap, resolver: &mut Resolver, dirs: &Dirs) -> Vec<SessionRow> {
    let projects = dirs.claude.join("projects");
    let skip = |name: &str| has_codex_sidecar(name, dirs);
    let mut rows = Vec::new();
    let mut sids = HashMap::new();
    for s in discover_panes(panes, procs, children, &projects, resolver, &skip) {
        let mut row = links::blank_row(&s.name);
        row.cwd = Some(s.cwd.clone());
        let (jsonl, tracked) = match (s.provider, s.transcript) {
            (_, Some(t)) => (t.jsonl, t.tracked),
            // Pane Codex sem sidecar: a TUI ainda não abriu a thread, não há rollout.
            ("codex", None) => (None, false),
            (provider, None) => {
                let t = ticket_transcript(provider, &s.pane_id, s.pane_pid, &s.cwd, procs, dirs);
                let tracked = t.is_some();
                (t, tracked)
            }
        };
        row.jsonl = jsonl;
        row.tracked = tracked;
        links::fill_pane_row(&mut row, s.provider, s.agent_pid.or(s.pane_pid), s.session_created, procs, dirs);
        sids.insert(s.name, s.repl_sid);
        rows.push(row);
    }
    links::dedupe_collisions(&mut rows, &sids);
    // Nascimento do terminal de mesmo nome, escondidas incluídas, como o `terminal_births`.
    let mut births = HashMap::new();
    for pane in panes {
        if let Some(b) = pane.session_created {
            births.entry(pane.session.clone()).or_insert(b);
        }
    }
    rows.extend(codex_rows(dirs, &births, procs));
    rows.extend(headless_rows(dirs, procs));
    rows
}

/// Transcript de um pane Pi, omp ou Kimi pelo bilhete (e, no Pi, pelo `CP_PI_SESSION`). `None` =
/// sem vínculo, e a linha sai `tracked=false`. Lido do processo do pane, como faz a extensão.
pub fn ticket_transcript(provider: &str, pane_id: &str, pane_pid: Option<i64>, cwd: &str, procs: &dyn ProcessView, dirs: &Dirs) -> Option<String> {
    match provider {
        "pi" | "omp" => pi_transcript(pane_id, pane_pid, cwd, provider, procs, dirs),
        "kimi" => kimi_transcript(pane_id, pane_pid, cwd, procs, dirs),
        _ => None,
    }
}

/// Bilhete lido e já decodificado; o resto (`Err`) cai no reserva, como o `except (OSError,
/// ValueError)` do Python.
fn read_ticket(path: &Path) -> Result<Value, ()> {
    let raw = std::fs::read(path).map_err(|_| ())?;
    let text = std::str::from_utf8(&raw).map_err(|_| ())?;
    serde_json::from_str(text).map_err(|_| ())
}

/// `isinstance(ts, (int, float))`: `bool` também conta no Python.
fn number(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        _ => None,
    }
}

/// A lista roda a cada 1,5 s: um aviso por minuto por chave e código.
fn warn_limited(key: &str, code: &'static str, field: &str) {
    if crate::warn_limit::allow(Some(key), code) {
        tracing::warn!(code, key, field, "list: entrada recusada");
    }
}

/// Folga entre o `Date.now()` da extensão e o nascimento do processo pelo kernel.
const TICKET_SLACK: f64 = 2.0;

fn pi_transcript(pane_id: &str, pid: Option<i64>, cwd: &str, provider: &str, procs: &dyn ProcessView, dirs: &Dirs) -> Option<String> {
    let base = pid.and_then(|p| config_dir_of(procs, p)).unwrap_or_else(|| dirs.default_claude());
    let ticket = base.join(".hangar-pi").join(format!("{}.json", ticket_key(pane_id, pid, procs)));
    let sid = pid.and_then(|p| env(procs, p, "CP_PI_SESSION"));
    let profile = || if provider == "omp" { pid.and_then(|p| env(procs, p, "OMP_PROFILE")) } else { None };
    if let Ok(data) = read_ticket(&ticket) {
        let Value::Object(data) = data else { return None };
        let mut file = data.get("file").and_then(Value::as_str).filter(|f| !f.is_empty()).map(str::to_owned);
        let born = pid.and_then(|p| procs.start_time(p));
        match (born, number(data.get("ts"))) {
            // Frescor que não dá para provar é recusa: o pane reusado abriria a conversa anterior.
            (None, _) => { warn_limited(pane_id, "list_pi_ticket_refused", "nascimento"); file = None }
            (_, None) => { warn_limited(pane_id, "list_pi_ticket_refused", "ts"); file = None }
            (Some(born), Some(ts)) if ts < born - TICKET_SLACK => file = None,
            _ => {
                if file.as_deref().is_some_and(is_pi_subagent) {
                    warn_limited(pane_id, "list_pi_ticket_subagent", "file");
                    file = file.as_deref().and_then(pi_root_transcript);
                }
            }
        }
        if let Some(f) = file {
            // O omp grava o principal em `sessions/-/<nome>`, não no caminho do `--session`.
            if provider == "omp" && !Path::new(&f).exists() {
                let name = Path::new(&f).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                return Some(find_in_root(&sessions_root(provider, profile().as_deref(), dirs), |n| n == name).unwrap_or(f));
            }
            return Some(f);
        }
    }
    let sid = sid?;
    Some(pi_transcript_of_id(cwd, &sid, provider, profile().as_deref(), dirs)).filter(|t| !t.is_empty())
}

fn sessions_root(provider: &str, profile: Option<&str>, dirs: &Dirs) -> PathBuf {
    if provider == "pi" {
        return dirs.pi_sessions.clone();
    }
    omp_agent_dir(profile, dirs).join("sessions")
}

static OMP_PROFILE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9._-]{0,63}$").unwrap());
static WINDOWS_RESERVED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(CON|PRN|AUX|NUL|COM[0-9]|LPT[0-9])(?:\.|$)").unwrap());

/// Perfil do omp daquela sessão (`omp_plugin_sync.resolve_omp_directories`). Perfil inválido cai no
/// diretório do backend, com aviso, como o `omp_dirs.agent_dir` de quem só lê.
fn omp_agent_dir(profile: Option<&str>, dirs: &Dirs) -> PathBuf {
    let Some(p) = profile.map(str::trim).filter(|p| !p.is_empty() && *p != "default") else { return dirs.omp_agent.clone() };
    if p == "." || p == ".." || p.ends_with('.') || !OMP_PROFILE_RE.is_match(p) || WINDOWS_RESERVED.is_match(p) {
        warn_limited(p, "list_omp_profile_invalid", "OMP_PROFILE");
        return dirs.omp_agent.clone();
    }
    dirs.omp_config.join("profiles").join(p).join("agent")
}

/// Pasta das sessões Pi de um cwd: só separador vira `-` (`getDefaultSessionDirPath` do Pi).
fn pi_cwd_slug(cwd: &str) -> String {
    let resolved = absolute(cwd);
    let trimmed = resolved.strip_prefix(['/', '\\']).unwrap_or(&resolved);
    format!("--{}--", trimmed.replace(['/', '\\', ':'], "-"))
}

/// `os.path.abspath(os.path.expanduser(p))` sem o `~`: o cwd do pane já vem absoluto.
fn absolute(path: &str) -> String {
    let p = Path::new(path);
    let joined = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(p) };
    hangar_workspace::worktrees::normpath(&joined).to_string_lossy().into_owned()
}

fn mtime(path: &Path) -> SystemTime {
    path.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH)
}

/// `pi_sessions.transcript_path`: o mais novo `*_<sid>.jsonl` da pasta do cwd; no omp, em qualquer
/// pasta da raiz.
fn pi_transcript_of_id(cwd: &str, sid: &str, provider: &str, profile: Option<&str>, dirs: &Dirs) -> String {
    let root = sessions_root(provider, profile, dirs);
    let suffix = format!("_{sid}.jsonl");
    let matches = |n: &str| n.ends_with(&suffix);
    let dir = root.join(pi_cwd_slug(cwd));
    if let Some(found) = newest(&dir, &matches) {
        return found;
    }
    if provider == "omp" { find_in_root(&root, matches).unwrap_or_default() } else { String::new() }
}

fn newest(dir: &Path, matches: &dyn Fn(&str) -> bool) -> Option<String> {
    let entries = std::fs::read_dir(dir).ok()?;
    entries.flatten()
        .filter(|e| matches(&e.file_name().to_string_lossy()))
        .map(|e| e.path())
        .max_by_key(|p| mtime(p))
        .map(|p| p.to_string_lossy().into_owned())
}

/// `localizar_na_raiz`: o arquivo mais novo que casa em qualquer pasta de cwd da raiz.
fn find_in_root(root: &Path, matches: impl Fn(&str) -> bool) -> Option<String> {
    let dirs = std::fs::read_dir(root).ok()?;
    dirs.flatten()
        .filter_map(|d| std::fs::read_dir(d.path()).ok())
        .flat_map(|d| d.flatten())
        .filter(|e| matches(&e.file_name().to_string_lossy()) && e.path().is_file())
        .map(|e| e.path())
        .max_by_key(|p| mtime(p))
        .map(|p| p.to_string_lossy().into_owned())
}

static STEM_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2}-\d{3}Z_[0-9a-fA-F-]{36}$").unwrap());
static RUN_DIR_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^run-\d+$").unwrap());

/// Transcript de subagente: `session.jsonl`, uma pasta `run-N` ou a pasta com o nome da sessão.
pub fn is_pi_subagent(path: &str) -> bool {
    let p = Path::new(path);
    let parents: Vec<String> = p.parent().map(|d| d.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    p.file_name().is_some_and(|n| n == "session.jsonl")
        || parents.iter().any(|d| RUN_DIR_RE.is_match(d) || STEM_RE.is_match(d))
}

/// Do subagente para a conversa: a pasta que guarda os runs tem o nome do arquivo da sessão.
fn pi_root_transcript(path: &str) -> Option<String> {
    Path::new(path).ancestors().skip(1).take(4)
        .map(|anc| PathBuf::from(format!("{}.jsonl", anc.to_string_lossy())))
        .find(|cand| cand.is_file())
        .map(|cand| cand.to_string_lossy().into_owned())
}

fn kimi_transcript(pane_id: &str, pid: Option<i64>, cwd: &str, procs: &dyn ProcessView, dirs: &Dirs) -> Option<String> {
    let base = pid.and_then(|p| config_dir_of(procs, p)).unwrap_or_else(|| dirs.default_claude());
    let ticket = base.join(".hangar-kimi").join(format!("{}.json", ticket_key(pane_id, pid, procs)));
    let Ok(Value::Object(data)) = read_ticket(&ticket) else { return None };
    let sid = data.get("session_id").and_then(Value::as_str).filter(|s| !s.is_empty())?;
    let born = pid.and_then(|p| procs.start_time(p));
    match (born, number(data.get("ts"))) {
        (None, _) => { warn_limited(pane_id, "list_kimi_ticket_refused", "nascimento"); return None }
        (_, None) => { warn_limited(pane_id, "list_kimi_ticket_refused", "ts"); return None }
        (Some(born), Some(ts)) if ts < born - TICKET_SLACK => return None,
        _ => {}
    }
    let wd = data.get("cwd").and_then(Value::as_str).filter(|c| !c.is_empty()).unwrap_or(cwd);
    kimi_transcript_of_id(wd, sid, dirs)
}

fn kimi_wire(session_dir: &Path) -> String {
    session_dir.join("agents").join("main").join("wire.jsonl").to_string_lossy().into_owned()
}

/// `kimi_sessions.transcript_path`: o índice primeiro; a pasta calculada cobre o índice atrasado.
/// `None`: índice com byte inválido, que no Python levanta e deixa a sessão sem transcript.
fn kimi_transcript_of_id(cwd: &str, sid: &str, dirs: &Dirs) -> Option<String> {
    if let Ok(raw) = std::fs::read(dirs.kimi_home.join("session_index.jsonl")) {
        for line in std::str::from_utf8(&raw).ok()?.lines() {
            let Ok(Value::Object(o)) = serde_json::from_str::<Value>(line) else { continue };
            if o.get("sessionId").and_then(Value::as_str) == Some(sid)
                && let Some(dir) = o.get("sessionDir").and_then(Value::as_str).filter(|d| !d.is_empty())
            {
                return Some(kimi_wire(Path::new(dir)));
            }
        }
    }
    let dir = dirs.kimi_home.join("sessions").join(kimi_workdir_key(cwd)).join(sid);
    dir.is_dir().then(|| kimi_wire(&dir))
}

static KIMI_SLUG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^a-z0-9._-]+").unwrap());

/// `wd_<slug do nome>_<sha256 do caminho>[:12]`, porte do `slugifyWorkDirName` do Kimi.
pub fn kimi_workdir_key(cwd: &str) -> String {
    let resolved = absolute(cwd);
    let name = Path::new(&resolved).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let slug = KIMI_SLUG_RE.replace_all(&name.to_lowercase(), "-").trim_matches('-').chars().take(40).collect::<String>();
    let slug = slug.trim_matches('-');
    let slug = if matches!(slug, "" | "." | "..") { "workspace" } else { slug };
    let digest = ring::digest::digest(&ring::digest::SHA256, resolved.as_bytes());
    let hex: String = digest.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    format!("wd_{slug}_{}", &hex[..12])
}

/// `names.sanitize_session_name`: acento vira a letra sem ele, o resto fora de `[A-Za-z0-9_-]` vira `-`.
pub fn sanitize_session_name(name: &str) -> String {
    // ponytail: só os acentos do português e vizinhos, sem NFKD completo (crate novo); outra letra
    // composta some em vez de virar a base. Nome criado pelo app já chega sanitizado.
    let ascii: String = name.chars().filter_map(fold_accent).collect();
    if name.chars().any(|c| c.is_alphabetic() && fold_accent(c).is_none()) {
        warn_limited(name, "list_session_name_unfolded", "name");
    }
    ascii.trim().chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '-' }).collect::<String>().trim_matches('-').to_owned()
}

/// Letra latina acentuada → a base ASCII (o NFKD + `encode("ascii", "ignore")`); outro não-ASCII some.
fn fold_accent(c: char) -> Option<char> {
    if c.is_ascii() {
        return Some(c);
    }
    const MAP: [(&str, char); 14] = [
        ("ÀÁÂÃÄÅ", 'A'), ("àáâãäå", 'a'), ("ÈÉÊË", 'E'), ("èéêë", 'e'), ("ÌÍÎÏ", 'I'), ("ìíîï", 'i'),
        ("ÒÓÔÕÖ", 'O'), ("òóôõö", 'o'), ("ÙÚÛÜ", 'U'), ("ùúûü", 'u'), ("Ç", 'C'), ("ç", 'c'), ("Ñ", 'N'), ("ñ", 'n'),
    ];
    MAP.iter().find(|(set, _)| set.contains(c)).map(|(_, b)| *b)
}

/// O pane de mesmo nome sai da lista: identidade e histórico do Codex vêm do sidecar.
pub fn has_codex_sidecar(name: &str, dirs: &Dirs) -> bool {
    dirs.home.join(".hangar").join("codex-sessions").join(format!("{}.json", sanitize_session_name(name))).exists()
}

/// Sidecars de uma pasta em ordem de nome, com o nome do arquivo; ilegível ou não-objeto fica de
/// fora, como no Python. Pasta que existe e não se lê avisa: as sessões dela sumiriam caladas.
fn sidecars(dir: &Path) -> Vec<(String, Map<String, Value>)> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                warn_limited(&dir.to_string_lossy(), "list_sidecar_dir_unreadable", "dir");
            }
            return Vec::new();
        }
    };
    let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    files.into_iter().filter_map(|f| match read_ticket(&f) {
        Ok(Value::Object(m)) => Some((f.file_name().unwrap_or_default().to_string_lossy().into_owned(), m)),
        _ => None,
    }).collect()
}

fn text(meta: &Map<String, Value>, key: &str) -> Option<String> {
    meta.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned)
}

/// `cwd_atual`: renomeada com a sessão viva, a pasta gravada some e o processo (conferido pela
/// chave no cmdline) mostra a nova.
fn current_cwd(meta: &Map<String, Value>, procs: &dyn ProcessView) -> Option<String> {
    let cwd = text(meta, "cwd")?;
    let pid = meta.get("cano").and_then(|c| c.get("pid")).and_then(Value::as_i64);
    let key = text(meta, "key");
    let (Some(pid), Some(key)) = (pid, key) else { return Some(cwd) };
    if Path::new(&cwd).is_dir() {
        return Some(cwd);
    }
    let prefix: String = key.chars().take(16).collect();
    if !procs.argv(pid).join("\0").contains(&prefix) {
        return Some(cwd);
    }
    match procs.cwd(pid) {
        Some(live) if live.is_dir() => Some(live.to_string_lossy().into_owned()),
        _ => Some(cwd),
    }
}

fn expand_user(path: &str, dirs: &Dirs) -> PathBuf {
    match path.strip_prefix('~') {
        Some("") => dirs.home.clone(),
        Some(rest) if rest.starts_with('/') => dirs.home.join(&rest[1..]),
        _ => PathBuf::from(path),
    }
}

/// Uma linha por sidecar Codex (`include_incomplete=True`). `births` é o nascimento do terminal de
/// mesmo nome; a vida de uma transferência em curso vem dos fatos.
pub fn codex_rows(dirs: &Dirs, births: &HashMap<String, u64>, procs: &dyn ProcessView) -> Vec<SessionRow> {
    let mut out = Vec::new();
    for (file, meta) in sidecars(&dirs.home.join(".hangar").join("codex-sessions")) {
        let Some(name) = text(&meta, "name") else {
            warn_limited(&file, "list_codex_sidecar_skipped", "name");
            continue;
        };
        let home = text(&meta, "codex_home").map(|h| expand_user(&h, dirs)).unwrap_or_else(|| dirs.codex_home.clone());
        let codex_home = links::resolve_lenient(&home).to_string_lossy().into_owned();
        let cwd = current_cwd(&meta, procs);
        let jsonl = text(&meta, "rollout_path");
        let mut row = links::blank_row(&name);
        row.lifecycle_id = links::session_life(text(&meta, "key").as_deref(), births.get(&name).copied());
        row.cwd = cwd;
        row.jsonl = jsonl;
        row.provider = "codex".to_owned();
        row.conta = Some(format!("codex:{codex_home}"));
        row.codex_home = Some(codex_home);
        row.headless = meta.get("headless").is_some_and(truthy);
        links::fill_location(&mut row, "codex", dirs);
        links::fill_links(&mut row, dirs);
        out.push(row);
    }
    out
}

pub(super) fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// `transcript_path` do Claude sem terminal: `<conta>/projects/<cwd>/<sid>.jsonl`, ou onde o
/// `EnterWorktree` o levou (o sid não repete entre pastas).
fn headless_transcript(cwd: &str, sid: &str, config_dir: Option<&str>, dirs: &Dirs) -> String {
    let base = config_dir.map(|c| Path::new(c).join("projects")).unwrap_or_else(|| dirs.claude.join("projects"));
    let file = format!("{sid}.jsonl");
    let expected = base.join(hangar_workspace::worktrees::sanitize_cwd(cwd)).join(&file);
    if expected.exists() {
        return expected.to_string_lossy().into_owned();
    }
    let mut moved: Vec<PathBuf> = std::fs::read_dir(&base).into_iter().flatten().flatten()
        .map(|d| d.path().join(&file)).filter(|p| p.exists()).collect();
    moved.sort();
    moved.into_iter().next().unwrap_or(expected).to_string_lossy().into_owned()
}

/// Uma linha por sessão Claude sem terminal: a identidade vem do sidecar.
pub fn headless_rows(dirs: &Dirs, procs: &dyn ProcessView) -> Vec<SessionRow> {
    let mut out = Vec::new();
    for (file, meta) in sidecars(&dirs.home.join(".hangar").join("claude-headless")) {
        // Sem nome ou sid o Python também filtra calado: é o sidecar ainda sendo escrito.
        let (Some(name), Some(sid)) = (text(&meta, "name"), text(&meta, "session_id")) else { continue };
        let Some(saved_cwd) = text(&meta, "cwd") else {
            warn_limited(&file, "list_headless_sidecar_skipped", "cwd");
            continue;
        };
        let config_dir = text(&meta, "config_dir");
        let engine = text(&meta, "engine");
        let account = text(&meta, "engine_account");
        let mut row = links::blank_row(&name);
        row.lifecycle_id = links::session_life(text(&meta, "key").as_deref(), None);
        row.cwd = current_cwd(&meta, procs);
        row.jsonl = Some(headless_transcript(&saved_cwd, &sid, config_dir.as_deref(), dirs));
        row.headless = true;
        row.conta = if account.is_some() {
            text(&meta, "engine_credential_id")
        } else if let Some(e) = &engine {
            Some(format!("chave:{e}"))
        } else {
            let cdir = config_dir.map(PathBuf::from).unwrap_or_else(|| dirs.default_claude());
            Some(format!("claude:{}", links::resolve_lenient(&cdir).to_string_lossy()))
        };
        row.engine = engine;
        row.engine_account = account;
        links::fill_location(&mut row, "claude", dirs);
        links::fill_links(&mut row, dirs);
        out.push(row);
    }
    out
}
