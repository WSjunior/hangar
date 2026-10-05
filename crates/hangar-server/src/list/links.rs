//! Campos comuns a toda linha: vida da sessão, par, encadeamento, loop, worktree, motor e conta
//! (`registry.py:1376-1431`), e a guarda de colisão de transcript (`:1219-1262`).
use super::discover_other::{Dirs, config_dir_of, env, truthy};
use super::procs::ProcessView;
use hangar_api::session::SessionRow;
use hangar_workspace::git::head_info;
use hangar_workspace::worktrees::{main_repo_of, normpath, removed_at, repo_root_of, worktree_paths};
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

/// Linha com os padrões do `SessionInfo`.
pub fn blank_row(name: &str) -> SessionRow {
    serde_json::from_value(serde_json::json!({ "name": name })).expect("SessionRow só exige name")
}

/// `share_life.session_life` sem a parte de transferência (vem dos fatos): a chave do sidecar
/// vence; sem ela, o nascimento do terminal.
pub fn session_life(key: Option<&str>, birth: Option<u64>) -> Option<String> {
    if let Some(key) = key.filter(|k| !k.is_empty()) {
        return Some(format!("k:{key}"));
    }
    birth.filter(|b| *b > 0).map(|b| format!("t:{b}"))
}

/// `Path.resolve(strict=False)`: segue o link no que existe e junta o resto como está.
pub fn resolve_lenient(path: &Path) -> PathBuf {
    let abs = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(path) };
    let abs = normpath(&abs);
    for base in abs.ancestors() {
        if let Ok(real) = std::fs::canonicalize(base) {
            return match abs.strip_prefix(base) {
                Ok(rest) if !rest.as_os_str().is_empty() => real.join(rest),
                _ => real,
            };
        }
    }
    abs
}

/// `pqueue._sanitize`: nome do sidecar de vínculo (mantém o ponto, ao contrário do da sessão).
fn link_file(dir: &Path, name: &str) -> PathBuf {
    let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || "_.-".contains(c) { c } else { '-' }).collect();
    dir.join(format!("{safe}.json"))
}

/// JSON-objeto do sidecar; ausente, torto ou de outro tipo é "sem vínculo".
fn read_object(path: &Path) -> Option<Map<String, Value>> {
    match serde_json::from_slice(&std::fs::read(path).ok()?) {
        Ok(Value::Object(m)) => Some(m),
        _ => None,
    }
}

struct Pair {
    peers: Vec<String>,
    /// `None` só quando o arquivo grava `task: null`; ausente é `""`.
    task: Option<String>,
    gid: String,
}

/// `PairLink.get`: legado `{"peer": x}` vira `peers`; sem membros só vale o grupo `orq`; sem `gid`
/// deriva um do conjunto, igual em todos os membros.
fn pair_of(name: &str, dirs: &Dirs) -> Option<Pair> {
    let data = read_object(&link_file(&dirs.claude.join(".hangar-pair"), name))?;
    let raw = match data.get("peers") {
        Some(p) => p.clone(),
        None => data.get("peer").filter(|p| truthy(p)).map(|p| Value::Array(vec![p.clone()])).unwrap_or(Value::Null),
    };
    let peers: Vec<String> = raw.as_array().map(|a| a.iter().filter_map(Value::as_str).filter(|p| !p.is_empty()).map(str::to_owned).collect()).unwrap_or_default();
    if peers.is_empty() && data.get("orq") != Some(&Value::Bool(true)) {
        return None;
    }
    let task = match data.get("task") { None => Some(String::new()), Some(t) => t.as_str().map(str::to_owned) };
    let gid = data.get("gid").and_then(Value::as_str).filter(|g| !g.is_empty()).map(str::to_owned).unwrap_or_else(|| legacy_gid(name, &peers));
    Some(Pair { peers, task, gid })
}

fn legacy_gid(name: &str, peers: &[String]) -> String {
    let mut all: Vec<&str> = std::iter::once(name).chain(peers.iter().map(String::as_str)).collect();
    all.sort_unstable();
    sha1_smol::Sha1::from(all.join("\n")).digest().to_string()[..8].to_owned()
}

static EXTERNAL_WARNED: AtomicBool = AtomicBool::new(false);

/// `_pair_external`: o par de fora entre os peers da sessão. O Python põe o arquivo torto de lado
/// ao ler; aqui só se avisa, uma vez.
fn pair_external(name: &str, peers: &[String], dirs: &Dirs) -> Option<Map<String, Value>> {
    let path = dirs.claude.join(".hangar-pair").join("external_pairs.json");
    let records = match std::fs::read(&path) {
        Err(_) => return None,
        Ok(raw) => match serde_json::from_slice::<Value>(&raw) {
            Ok(Value::Array(items)) => items,
            _ => {
                if !EXTERNAL_WARNED.swap(true, Ordering::Relaxed) {
                    tracing::warn!(file = "external_pairs.json", "list: unreadable, external pairs ignored");
                }
                return None;
            }
        },
    };
    records.iter().filter_map(Value::as_object).find_map(|r| {
        let field = |k: &str| r.get(k).and_then(Value::as_str);
        if field("local_session")? != name {
            return None;
        }
        let address = format!("{}::{}", field("alias")?, field("peer_session")?);
        peers.contains(&address).then(|| {
            let mut out = Map::new();
            for (out_key, key) in [("alias", "alias"), ("owner", "peer_owner"), ("session", "peer_session")] {
                out.insert(out_key.to_owned(), r.get(key).cloned().unwrap_or(Value::Null));
            }
            out
        })
    })
}

/// Encadeamento e par (`ThenLink`, `PairLink`, `_pair_external`).
pub fn fill_links(row: &mut SessionRow, dirs: &Dirs) {
    row.then_target = read_object(&link_file(&dirs.claude.join(".hangar-chain"), &row.name))
        .and_then(|l| l.get("target").and_then(Value::as_str).map(str::to_owned));
    match pair_of(&row.name, dirs) {
        Some(pair) => {
            row.pair_external = pair_external(&row.name, &pair.peers, dirs);
            row.pair_peers = Some(pair.peers);
            row.pair_gid = Some(pair.gid);
            row.pair_task = pair.task;
        }
        None => (row.pair_external, row.pair_peers, row.pair_gid, row.pair_task) = (None, None, None, None),
    }
}

/// `_decorate_loop`: sem sidecar, nenhum badge.
pub fn fill_loop(row: &mut SessionRow, dirs: &Dirs) {
    let Some(d) = read_object(&link_file(&dirs.claude.join(".hangar-loop"), &row.name)) else { return };
    row.loop_status = d.get("status").and_then(Value::as_str).map(str::to_owned);
    row.loop_iter = d.get("iter").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok());
    row.loop_max = d.get("max_iters").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok());
}

/// Campos de uma linha de pane depois do transcript resolvido: vida, worktree, vínculos, motor e
/// conta. `provider` é o detectado no pane (`claude` quando não se reconhece); `pid_env` é o do
/// processo do agente, senão o do pane: quem declara conta e motor é o agente.
pub fn fill_pane_row(row: &mut SessionRow, provider: &str, pid_env: Option<i64>, birth: Option<u64>, procs: &dyn ProcessView, dirs: &Dirs) {
    row.lifecycle_id = session_life(None, birth);
    // Transcript de chute (untracked) pode ser de outra sessão: não decide onde esta está.
    let jsonl = row.jsonl.clone().filter(|_| row.tracked);
    apply_location(row, locate(provider, row.cwd.as_deref(), jsonl.as_deref(), dirs));
    fill_links(row, dirs);
    if matches!(provider, "pi" | "omp" | "kimi" | "codex") {
        row.provider = provider.to_owned();
    }
    row.engine = pid_env.and_then(|p| env(procs, p, "CP_ENGINE"));
    row.engine_account = match (pid_env, &row.engine) {
        (Some(p), Some(_)) => env(procs, p, "CP_ENGINE_ACCOUNT"),
        _ => None,
    };
    row.conta = if let Some(engine) = &row.engine {
        if row.engine_account.is_some() { pid_env.and_then(|p| env(procs, p, "CP_ENGINE_CREDENTIAL_ID")) } else { Some(format!("chave:{engine}")) }
    } else {
        match provider {
            // Casada pelas credenciais no `cotas.py`: chega pelos fatos.
            "kimi" | "pi" | "omp" => None,
            "codex" => {
                let home = resolve_lenient(&dirs.codex_home).to_string_lossy().into_owned();
                row.codex_home = Some(home.clone());
                Some(format!("codex:{home}"))
            }
            _ => {
                let cdir = pid_env.and_then(|p| config_dir_of(procs, p)).unwrap_or_else(|| dirs.default_claude());
                Some(format!("claude:{}", resolve_lenient(&cdir).to_string_lossy()))
            }
        }
    };
}

/// Linha de sidecar: o transcript é sempre dela, então decide a worktree.
pub fn fill_location(row: &mut SessionRow, provider: &str, dirs: &Dirs) {
    let loc = locate(provider, row.cwd.as_deref(), row.jsonl.as_deref(), dirs);
    apply_location(row, loc);
}

fn apply_location(row: &mut SessionRow, loc: Location) {
    row.branch = loc.branch;
    row.worktree = loc.worktree;
    row.worktree_path = loc.worktree_path;
    row.worktree_gone = loc.worktree_gone;
    row.git_cwd = loc.git_cwd;
}

/// 2+ linhas no mesmo transcript: só a dona (sid do cmdline = nome do arquivo, ou a única
/// `tracked`) fica com ele; as outras perdem o vínculo. Sem dona clara, todas perdem.
pub fn dedupe_collisions(rows: &mut [SessionRow], sids: &HashMap<String, Option<String>>) {
    let mut groups: Vec<(PathBuf, Vec<usize>)> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let Some(jsonl) = row.jsonl.as_deref() else { continue };
        let real = resolve_lenient(Path::new(jsonl));
        match groups.iter_mut().find(|(k, _)| *k == real) {
            Some((_, members)) => members.push(i),
            None => groups.push((real, vec![i])),
        }
    }
    for (jsonl, members) in groups {
        if members.len() < 2 {
            continue;
        }
        let base = jsonl.file_name().map(|n| n.to_string_lossy().trim_end_matches(".jsonl").to_owned()).unwrap_or_default();
        let mut owner = members.iter().copied().find(|&i| sids.get(&rows[i].name).and_then(Option::as_deref) == Some(base.as_str()));
        if owner.is_none() {
            let tracked: Vec<usize> = members.iter().copied().filter(|&i| rows[i].tracked).collect();
            if tracked.len() == 1 {
                owner = Some(tracked[0]);
            }
        }
        for i in members {
            if Some(i) == owner {
                continue;
            }
            tracing::info!(name = %rows[i].name, owner = owner.map(|o| rows[o].name.as_str()).unwrap_or("none"), "list: collision dropped borrowed transcript");
            rows[i].jsonl = None;
            rows[i].tracked = false;
        }
    }
}

// ── onde a sessão está (`worktrees.locate`) ──────────────────────────────────────────────────

#[derive(Debug, Default, PartialEq)]
pub struct Location {
    pub branch: Option<String>,
    pub worktree: bool,
    pub worktree_path: Option<String>,
    pub worktree_gone: bool,
    /// Raiz do repositório onde o agente trabalha, quando não é a da pasta de abertura.
    pub git_cwd: Option<String>,
}

fn is_dir(path: &str) -> bool { !path.is_empty() && Path::new(path).is_dir() }

/// Sessão que nasceu numa worktree fica nela; senão os sinais do transcript (Claude) ou dos
/// comandos (Codex) dizem para qual pasta do mesmo repositório o agente foi.
pub fn locate(provider: &str, cwd: Option<&str>, jsonl: Option<&str>, dirs: &Dirs) -> Location {
    let cwd = cwd.filter(|c| !c.is_empty());
    let born_in_worktree = head_info(cwd.and_then(repo_root_of).as_deref()).1;
    let mut real = None;
    if !born_in_worktree {
        // Transcript ilegível nunca derruba a lista: a sessão fica no cwd.
        real = match (provider, jsonl, cwd) {
            ("claude", Some(j), _) => claude_worktree(cwd, j, dirs),
            ("codex", Some(j), Some(c)) => codex_cwd(c, j, dirs),
            _ => None,
        };
    }
    let Some(real) = real.filter(|r| !r.is_empty()).or_else(|| cwd.map(str::to_owned)) else { return Location::default() };
    if !is_dir(&real) {
        // ponytail: pasta sumida que não é a de abertura vira "worktree apagada"; uma subpasta
        // comum apagada também cairia aqui, como no Python.
        let gone = Some(real.as_str()) != cwd || removed(dirs).contains_key(&real);
        return Location { worktree_path: gone.then_some(real), worktree_gone: gone, ..Location::default() };
    }
    let root = repo_root_of(&real);
    let (branch, worktree) = head_info(Some(root.as_deref().unwrap_or(&real)));
    let moved = root.is_some() && root != cwd.and_then(repo_root_of);
    Location {
        branch,
        worktree,
        worktree_path: worktree.then(|| root.clone().unwrap_or_else(|| real.clone())),
        worktree_gone: false,
        git_cwd: if moved { root } else { None },
    }
}

fn removed(dirs: &Dirs) -> HashMap<String, String> {
    removed_at(&dirs.home.join(".hangar").join("worktrees-removidas.json"))
}

/// Uma leitura por versão do arquivo: a lista roda a cada segundo e o transcript pode ter megas.
type TailCache<T> = LazyLock<Mutex<HashMap<PathBuf, ((i128, u64), T)>>>;

fn file_key(path: &Path) -> Option<(i128, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as i128).unwrap_or(0);
    Some((mtime, meta.len()))
}

/// ponytail: sem poda, como o `_cache` do Python; o teto é o número de transcripts da máquina.
fn cached<T: Clone>(cache: &TailCache<T>, path: &Path, read: impl FnOnce() -> Option<T>) -> Option<T> {
    let key = file_key(path)?;
    if let Some((k, v)) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(path)
        && *k == key
    {
        return Some(v.clone());
    }
    let value = read()?;
    cache.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_path_buf(), (key, value.clone()));
    Some(value)
}

const TAIL: u64 = 256 * 1024;
const DEEP_TAIL: u64 = 8 * 1024 * 1024;

/// Linhas do fim para o começo, em blocos de `TAIL`, até `DEEP_TAIL` bytes (`reversed_lines`).
fn reversed_lines(path: &Path) -> std::io::Result<Vec<Vec<u8>>> {
    let mut fh = std::fs::File::open(path)?;
    let end = fh.seek(SeekFrom::End(0))?;
    let mut pos = end;
    let mut rest: Vec<u8> = Vec::new();
    let mut out = Vec::new();
    while pos > 0 && end - pos < DEEP_TAIL {
        let step = TAIL.min(pos);
        pos -= step;
        fh.seek(SeekFrom::Start(pos))?;
        let mut block = vec![0; step as usize];
        fh.read_exact(&mut block)?;
        if !block.contains(&b'\n') {
            block.extend_from_slice(&rest);
            rest = block;
            continue;
        }
        block.extend_from_slice(&rest);
        let mut lines: Vec<Vec<u8>> = block.split(|b| *b == b'\n').map(<[u8]>::to_vec).collect();
        rest = lines.remove(0);
        out.extend(lines.into_iter().rev());
    }
    if pos == 0 {
        out.push(rest);
    }
    Ok(out)
}

static SHELL_DIR_RE: LazyLock<Regex> = LazyLock::new(|| {
    // O `\1` do Python (aspas casadas) vira três alternativas: o Rust não tem retrorreferência.
    Regex::new(r#"(?:(?:^|[\n;&|(])\s*cd|\bgit\s+-C)\s+(?:"([^\s;&|"')]+)"|'([^\s;&|"')]+)'|([^\s;&|"')]+))"#).unwrap()
});

/// (caminho, é `cd`?) de uma chamada, do último citado para o primeiro.
fn tool_paths(block: &Map<String, Value>) -> Vec<(String, bool)> {
    let Some(Value::Object(args)) = block.get("input") else { return Vec::new() };
    let name = block.get("name").and_then(Value::as_str).unwrap_or("");
    if name == "Bash" && let Some(command) = args.get("command").and_then(Value::as_str) {
        let found: Vec<(String, bool)> = SHELL_DIR_RE.captures_iter(command)
            .filter_map(|c| c.get(1).or(c.get(2)).or(c.get(3)).map(|m| (m.as_str().to_owned(), true)))
            .collect();
        return found.into_iter().rev().collect();
    }
    let key = match name { "Edit" | "MultiEdit" | "Write" => "file_path", "NotebookEdit" => "notebook_path", _ => return Vec::new() };
    args.get(key).and_then(Value::as_str).map(|t| vec![(t.to_owned(), false)]).unwrap_or_default()
}

type ClaudeTail = (Option<String>, Vec<(String, bool, Option<String>)>);
static CLAUDE_TAILS: TailCache<ClaudeTail> = LazyLock::new(Default::default);

/// (último `cwd`, caminhos citados pelas ferramentas com o `cwd` da linha), do mais recente para o
/// mais antigo, numa leitura só.
fn claude_tail(jsonl: &Path) -> Option<ClaudeTail> {
    cached(&CLAUDE_TAILS, jsonl, || {
        let mut last: Option<String> = None;
        let mut hits = Vec::new();
        for raw in reversed_lines(jsonl).ok()? {
            let has_tool = contains(&raw, b"\"tool_use\"");
            // Achado o último `cwd`, só interessa linha com chamada: o resto pode ser imagem de megas.
            if !has_tool && (last.is_some() || !contains(&raw, b"\"cwd\"")) {
                continue;
            }
            let Ok(Value::Object(line)) = serde_json::from_slice::<Value>(&raw) else { continue };
            let cwd = line.get("cwd").and_then(Value::as_str).filter(|c| !c.is_empty()).map(str::to_owned);
            if last.is_none() {
                last = cwd.clone();
            }
            if has_tool && let Some(Value::Array(content)) = line.get("message").and_then(|m| m.get("content")) {
                for block in content.iter().rev().filter_map(Value::as_object) {
                    if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                        hits.extend(tool_paths(block).into_iter().map(|(p, cd)| (p, cd, cwd.clone())));
                    }
                }
            }
            if hits.len() >= 20 {
                break;
            }
        }
        Some((last, hits))
    })
}

fn contains(hay: &[u8], needle: &[u8]) -> bool { hay.windows(needle.len()).any(|w| w == needle) }

/// (principal, worktrees removidas, todas as pastas) do repositório que contém `path`.
fn repo_candidates(path: &str, dirs: &Dirs) -> Option<(String, Vec<String>, Vec<String>)> {
    let main = main_repo_of(&repo_root_of(path)?);
    let gone: Vec<String> = removed(dirs).into_iter().filter(|(_, v)| *v == main).map(|(k, _)| k).collect();
    let mut all = vec![main.clone()];
    all.extend(worktree_paths(&main));
    all.extend(gone.iter().cloned());
    Some((main, gone, all))
}

fn owner(path: &str, candidates: &[String]) -> Option<String> {
    candidates.iter()
        .filter(|c| path == c.as_str() || path.starts_with(&format!("{}/", c.trim_end_matches('/'))))
        .fold(None::<&String>, |best, c| if best.is_none_or(|b| c.len() > b.len()) { Some(c) } else { best })
        .cloned()
}

/// Pasta sumida só conta se for deste repositório, de uma worktree removida dele ou de uma irmã
/// no padrão do Hangar (`<repo>-<x>`).
fn of_this_repo(path: &str, main: &str, gone: &[String]) -> bool {
    let mut own = vec![main.to_owned()];
    own.extend(gone.iter().cloned());
    if owner(path, &own).is_some() {
        return true;
    }
    let trimmed = main.trim_end_matches('/');
    let (parent, name) = trimmed.rsplit_once('/').unwrap_or(("", trimmed));
    let prefix = format!("{}/", parent.trim_end_matches('/'));
    path.strip_prefix(&prefix).is_some_and(|rest| rest.split('/').next().unwrap_or("").starts_with(&format!("{name}-")))
}

fn expand_user(path: &str, dirs: &Dirs) -> String {
    match path.strip_prefix('~') {
        Some("") => dirs.home.to_string_lossy().into_owned(),
        Some(rest) if rest.starts_with('/') => format!("{}{rest}", dirs.home.to_string_lossy()),
        _ => path.to_owned(),
    }
}

/// `os.path.normpath(os.path.join(base, p))`.
fn join_norm(base: &str, p: &str) -> String {
    normpath(&Path::new(base).join(p)).to_string_lossy().into_owned()
}

/// A pasta do mesmo repositório onde o Claude trabalha: `cd X`/`git -C X` e o arquivo editado.
/// Nada na principal tira a sessão da worktree.
fn claude_worktree(cwd: Option<&str>, jsonl: &str, dirs: &Dirs) -> Option<String> {
    let (last, hits) = claude_tail(Path::new(jsonl))?;
    let Some(base) = last.clone().or_else(|| cwd.map(str::to_owned)) else { return last };
    let Some((main, gone, candidates)) = repo_candidates(&base, dirs) else { return last };
    let home = owner(&base, &candidates);
    for (raw, is_cd, line_cwd) in hits {
        if let Some(lc) = &line_cwd
            && owner(lc, &candidates) != home
        {
            break; // chamada anterior à última troca de `cwd` (EnterWorktree/ExitWorktree)
        }
        let p = join_norm(line_cwd.as_deref().unwrap_or(&base), &expand_user(&raw, dirs));
        if Path::new(&p).exists() {
            if let Some(o) = owner(&p, &candidates).filter(|o| *o != main) {
                return Some(o);
            }
        } else if is_cd && (raw.starts_with('/') || raw.starts_with('~')) && of_this_repo(&p, &main, &gone) {
            // Pasta absoluta que sumiu: a worktree foi removida. `cd -` e `cd $W` não dizem nada.
            return Some(owner(&p, &gone).unwrap_or(p));
        }
    }
    last
}

static WORKDIR_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""?workdir"?\s*:\s*"(/[^"]+)""#).unwrap());
static CD_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""?cmd"?\s*:\s*"\s*cd\s+(/[^\s&;"]+)"#).unwrap());
static PATCH_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\*\*\* (?:Add|Update|Delete) File: (/[^\s\\"]+)"#).unwrap());
static CODEX_PATHS: TailCache<Vec<(String, bool)>> = LazyLock::new(Default::default);

/// Últimos `TAIL` bytes em linhas; a primeira vem cortada quando o arquivo é maior.
fn tail_lines(path: &Path) -> std::io::Result<Vec<Vec<u8>>> {
    let mut fh = std::fs::File::open(path)?;
    let size = fh.seek(SeekFrom::End(0))?;
    fh.seek(SeekFrom::Start(size.saturating_sub(TAIL)))?;
    let mut buf = Vec::new();
    fh.read_to_end(&mut buf)?;
    let mut lines: Vec<Vec<u8>> = buf.split(|b| *b == b'\n').map(<[u8]>::to_vec).collect();
    if size > TAIL {
        lines.remove(0);
    }
    Ok(lines)
}

/// (caminho, é pasta?) dos comandos do Codex, da chamada mais recente para a mais antiga: `cd`
/// vence `workdir`, e os dois vencem o arquivo do patch.
fn codex_paths(rollout: &Path) -> Option<Vec<(String, bool)>> {
    cached(&CODEX_PATHS, rollout, || {
        let mut out = Vec::new();
        for raw in tail_lines(rollout).ok()?.into_iter().rev() {
            if !contains(&raw, b"_call") {
                continue;
            }
            let Ok(Value::Object(line)) = serde_json::from_slice::<Value>(&raw) else { continue };
            let Some(Value::Object(payload)) = line.get("payload") else { continue };
            if !matches!(payload.get("type").and_then(Value::as_str), Some("function_call" | "custom_tool_call")) {
                continue;
            }
            let text = [payload.get("arguments"), payload.get("input")].into_iter().flatten()
                .find(|v| truthy(v));
            let Some(Value::String(text)) = text else { continue };
            for (rx, is_dir) in [(&*CD_RE, true), (&*WORKDIR_RE, true), (&*PATCH_RE, false)] {
                let found: Vec<String> = rx.captures_iter(text).map(|c| c[1].to_owned()).collect();
                out.extend(found.into_iter().rev().map(|p| (p, is_dir)));
            }
            if out.len() >= 50 {
                break;
            }
        }
        Some(out)
    })
}

/// A worktree (ou a principal) do MESMO repositório onde o último comando do Codex rodou.
fn codex_cwd(cwd: &str, rollout: &str, dirs: &Dirs) -> Option<String> {
    let (main, gone, candidates) = repo_candidates(cwd, dirs)?;
    for (p, is_dir) in codex_paths(Path::new(rollout))? {
        if Path::new(&p).exists() {
            if let Some(o) = owner(&p, &candidates) {
                return Some(o);
            }
        } else if is_dir && of_this_repo(&p, &main, &gone) {
            // Pasta que sumiu: volta como está para o `locate` marcar "apagada".
            return Some(owner(&p, &gone).unwrap_or(p));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_dir_regex_matches_python_quotes() {
        let paths = |cmd: &str| {
            let mut block = Map::new();
            block.insert("name".into(), "Bash".into());
            block.insert("input".into(), serde_json::json!({ "command": cmd }));
            tool_paths(&block).into_iter().map(|(p, _)| p).collect::<Vec<_>>()
        };
        assert_eq!(paths("cd /a && git -C '/b' status; cd \"/c\""), ["/c", "/b", "/a"]);
        // Aspa sem par não casa, como o `\1` do Python.
        assert_eq!(paths("cd \"/x y\""), Vec::<String>::new());
        assert_eq!(paths("echo cd /nao"), Vec::<String>::new());
    }

    #[test]
    fn owner_takes_longest_and_sibling_rule() {
        let c = vec!["/r".to_owned(), "/r/wt".to_owned()];
        assert_eq!(owner("/r/wt/x", &c).as_deref(), Some("/r/wt"));
        assert_eq!(owner("/rx", &c), None);
        assert!(of_this_repo("/r-feat/a", "/r", &[]));
        assert!(!of_this_repo("/outro/a", "/r", &[]));
    }

    #[test]
    fn session_life_prefers_key() {
        assert_eq!(session_life(Some("k1"), Some(5)).as_deref(), Some("k:k1"));
        assert_eq!(session_life(None, Some(5)).as_deref(), Some("t:5"));
        assert_eq!(session_life(Some(""), None), None);
    }
}
