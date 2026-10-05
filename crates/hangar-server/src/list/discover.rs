//! Descoberta das sessões com terminal e resolução do transcript Claude (`registry.py:1064-1190`,
//! `:1265-1340`). Função pura sobre panes, processos e arquivos; os caches da resolução moram no
//! `Resolver`, que atravessa as rodadas.
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::SystemTime;

use hangar_workspace::worktrees::sanitize_cwd;

use super::capped::SESSION_CAP;
use super::mux::Pane;
use super::procs::{ChildrenMap, ProcessView};

/// Transcript de uma sessão Claude e se o vínculo é confiável (`tracked`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transcript {
    pub jsonl: Option<String>,
    pub tracked: bool,
}

/// Uma sessão do multiplexador, já no pane do agente.
#[derive(Clone, Debug, PartialEq)]
pub struct PaneSession {
    pub name: String,
    pub cwd: String,
    pub pane_id: String,
    pub pane_pid: Option<i64>,
    /// Pid do agente dentro do pane; conta e motor são lidos dele (ou do pane, sem agente).
    pub agent_pid: Option<i64>,
    pub provider: &'static str,
    pub session_created: Option<u64>,
    /// Só Claude. Pi, omp, Kimi e Codex resolvem o transcript pelo bilhete ou sidecar
    /// (`discover_other.rs`); os campos comuns da linha entram por `links.rs`.
    pub transcript: Option<Transcript>,
    /// `--session-id` do REPL principal: identifica a dona na guarda de colisão.
    pub repl_sid: Option<String>,
}

struct Marker { mtime: SystemTime, jsonl: Option<String>, pid: Option<i64>, ts: f64 }

/// Caches da resolução por nome de sessão (`_jsonl_cache`, `_fd_locked`) e o dos marcadores.
#[derive(Default)]
pub struct Resolver {
    jsonl: BTreeMap<String, String>,
    fd_locked: BTreeSet<String>,
    markers: HashMap<PathBuf, HashMap<OsString, Marker>>,
}

impl Resolver {
    /// O transcript que a criação, a troca de modo ou o resume já sabem.
    pub fn seed(&mut self, name: &str, jsonl: &str) { self.jsonl.insert(name.into(), jsonl.into()); }

    pub fn forget(&mut self, name: &str) {
        self.jsonl.remove(name);
        self.fd_locked.remove(name);
    }

    pub fn rename(&mut self, old: &str, new: &str) {
        if let Some(j) = self.jsonl.remove(old) {
            self.jsonl.insert(new.into(), j);
        }
        if self.fd_locked.remove(old) {
            self.fd_locked.insert(new.into());
        }
    }

    pub fn cached(&self) -> &BTreeMap<String, String> { &self.jsonl }
    pub fn fd_locked(&self) -> &BTreeSet<String> { &self.fd_locked }
}

const EXEC_PROVIDER: [(&str, &str); 7] = [("pi", "pi"), ("omp", "omp"), ("claude", "claude"), ("kimi", "kimi"),
    ("kimi-code", "kimi"), ("codex", "codex"), ("hangar-codex-tui", "codex")];
const PKG_PROVIDER: [(&str, &str); 1] = [("pi-coding-agent", "pi")];

static SID: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(
    r"--(?:session-id|resume)[ =]([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})").unwrap());
static PYTHON: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^python(?:\d+(?:\.\d+)*)?(?:\.exe)?$").unwrap());

fn basename(arg: &str) -> &str {
    Path::new(arg).file_name().and_then(|n| n.to_str()).unwrap_or(arg)
}

fn exec_provider(base: &str) -> Option<&'static str> {
    EXEC_PROVIDER.iter().find(|(exe, _)| *exe == base).map(|(_, p)| *p)
}

/// Provider pelo argv já separado (`_provider_do_argv`).
pub fn provider_from_argv(argv: &[String]) -> Option<&'static str> {
    let mut base = basename(argv.first()?);
    if cfg!(windows) {
        base = Path::new(base).file_stem().and_then(|s| s.to_str()).unwrap_or(base);
    }
    if let Some(p) = exec_provider(base) {
        return Some(p);
    }
    // A integração roda no lançador antes de existir qualquer processo `codex`.
    if PYTHON.is_match(base) && argv.get(1).is_some_and(|a| a.replace('\\', "/").rsplit('/').next() == Some("hangar-codex-tui")) {
        return Some("codex");
    }
    // Lançado por node: quem diz o agente é o caminho do script.
    if base == "node" || base == "node.exe" {
        for arg in &argv[1..] {
            let arg = arg.replace('\\', "/");
            if let Some((_, p)) = PKG_PROVIDER.iter().find(|(pkg, _)| arg.split('/').any(|part| part == *pkg)) {
                return Some(p);
            }
        }
    }
    None
}

/// Raiz mais descendentes, em pilha (o último filho primeiro), como `_descendant_pids`.
fn descendants(root: i64, children: &ChildrenMap) -> Vec<i64> {
    let (mut out, mut seen, mut stack) = (Vec::new(), HashSet::new(), vec![root]);
    while let Some(p) = stack.pop() {
        // ppid reciclado no Windows fecha anel no mapa.
        if !seen.insert(p) {
            continue;
        }
        out.push(p);
        stack.extend(children.get(&p).into_iter().flatten());
    }
    out
}

/// Processo da árvore que não é o REPL dono: daemon, host de pty e subagente.
fn is_aux(cmd: &str) -> bool { cmd.contains("daemon") || cmd.contains("--bg-") || cmd.contains("--agent") }

fn split(cmd: &str) -> Vec<String> { cmd.split_whitespace().map(String::from).collect() }

/// `realpath` quando o arquivo existe; senão o próprio caminho.
fn real(path: &str) -> String {
    std::fs::canonicalize(path).map_or_else(|_| path.to_string(), |p| p.to_string_lossy().into_owned())
}

fn mtime(path: &Path) -> Option<SystemTime> { std::fs::metadata(path).and_then(|m| m.modified()).ok() }

fn jsonl_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries.flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".jsonl")).map(|e| e.path()).collect()
}

/// Os processos do pane com argv e cmdline de cada um, lidos uma vez por rodada.
struct Tree {
    pids: Vec<i64>,
    argvs: Vec<Vec<String>>,
    cmds: Vec<String>,
}

impl Tree {
    fn new(procs: &dyn ProcessView, root: i64, children: &ChildrenMap) -> Self {
        let pids = descendants(root, children);
        let argvs: Vec<Vec<String>> = pids.iter().map(|p| procs.argv(*p)).collect();
        let cmds = argvs.iter().map(|a| a.join(" ")).collect();
        Self { pids, argvs, cmds }
    }

    fn iter(&self) -> impl Iterator<Item = (i64, &str)> { self.pids.iter().copied().zip(self.cmds.iter().map(String::as_str)) }

    /// Os candidatos a REPL principal, na ordem da descida.
    fn repl(&self) -> impl Iterator<Item = (i64, &str)> { self.iter().filter(|(_, c)| !is_aux(c)) }

    fn repl_sid(&self) -> Option<String> { self.repl().find_map(|(_, c)| session_id(c)) }
}

fn session_id(cmd: &str) -> Option<String> { SID.captures(cmd).map(|c| c[1].to_string()) }

/// `CLAUDE_CONFIG_DIR` do processo. Vazio conta como ausente (o Python leria `./projects`).
/// Ambiente ilegível cai no padrão como no Python, mas avisa: a sessão de outra conta resolveria
/// no diretório errado nesta rodada.
fn config_dir(procs: &dyn ProcessView, pid: i64) -> Option<PathBuf> {
    match procs.env_var(pid, "CLAUDE_CONFIG_DIR") {
        Ok(v) => v.filter(|v| !v.is_empty()).map(PathBuf::from),
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound && crate::warn_limit::allow(None, "list_environ_unreadable") {
                tracing::warn!(code = "list_environ_unreadable", pid, io_kind = ?error.kind(), "ambiente do processo ilegível");
            }
            None
        }
    }
}

/// Primeiro fd aberto num `*.jsonl` dentro de `projects`. Fora do Linux o sinal não vale o custo:
/// enumerar os handles do Windows leva segundos, e o Claude não segura o fd em idle.
fn open_jsonl(procs: &dyn ProcessView, pid: i64, projects: &Path) -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let base = format!("{}{}", projects.to_string_lossy(), std::path::MAIN_SEPARATOR);
    procs.fds(pid).into_iter().map(|t| t.to_string_lossy().into_owned())
        .find(|t| t.ends_with(".jsonl") && t.starts_with(&base))
}

fn projects_of(procs: &dyn ProcessView, pid: i64, default: &Path) -> PathBuf {
    config_dir(procs, pid).map_or_else(|| default.to_path_buf(), |c| c.join("projects"))
}

fn config_base_of(procs: &dyn ProcessView, pid: i64, default: &Path) -> PathBuf {
    config_dir(procs, pid).unwrap_or_else(|| default.parent().unwrap_or(default).to_path_buf())
}

/// Marcador do hook pelo `--session-id` do cmdline (`_active_marker_jsonl`): no resume o arquivo
/// do sid nunca nasce, e o marcador diz o transcript ativo.
fn active_marker(config_base: &Path, sid: &str, exclude: &HashSet<String>) -> Option<String> {
    let raw = std::fs::read(config_base.join(".hangar-active").join(format!("{sid}.json"))).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    let j = value.get("jsonl")?.as_str().filter(|j| !j.is_empty())?;
    (Path::new(j).exists() && !exclude.contains(&real(j))).then(|| j.to_string())
}

/// `float(o.get("ts") or 0.0)`; `None` é o que o Python recusa.
fn marker_ts(v: Option<&serde_json::Value>) -> Option<f64> {
    use serde_json::Value;
    match v {
        None | Some(Value::Null) => Some(0.0),
        Some(Value::Bool(b)) => Some(f64::from(u8::from(*b))),
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) if s.is_empty() => Some(0.0),
        Some(Value::String(s)) => s.trim().parse().ok(),
        Some(Value::Array(a)) if a.is_empty() => Some(0.0),
        Some(Value::Object(o)) if o.is_empty() => Some(0.0),
        Some(_) => None,
    }
}

/// Inteiro, ou float de valor inteiro (`401.0 in {401}` casa no Python).
fn marker_pid(v: Option<&serde_json::Value>) -> Option<i64> {
    let v = v?;
    v.as_i64().or_else(|| v.as_f64().filter(|f| f.fract() == 0.0 && f.abs() < 9.0e15).map(|f| f as i64))
}

fn read_marker(path: &Path, mtime: SystemTime) -> Option<Marker> {
    let o: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    o.is_object().then_some(())?;
    Some(Marker {
        mtime,
        jsonl: o.get("jsonl").and_then(|j| j.as_str()).map(String::from),
        pid: marker_pid(o.get("pid")),
        ts: marker_ts(o.get("ts"))?,
    })
}

impl Resolver {
    /// Marcador do hook casado por pid da árvore (`_marker_by_pids`); o mais recente vence. Relê só
    /// o marcador cujo mtime mudou: com centenas deles, ler todos a cada rodada era o custo.
    fn marker_by_pids(&mut self, config_base: &Path, pids: &[i64], exclude: &HashSet<String>) -> Option<String> {
        let dir = config_base.join(".hangar-active");
        let entries = std::fs::read_dir(&dir).ok()?;
        let mut previous = self.markers.remove(&dir).unwrap_or_default();
        let mut current: HashMap<OsString, Marker> = HashMap::new();
        let mut best: Option<(f64, String)> = None;
        for entry in entries.flatten() {
            let file = entry.file_name();
            if !file.to_string_lossy().ends_with(".json") {
                continue;
            }
            // Segue o symlink, como o `stat` do Python; sem mtime não há como saber se mudou.
            let Ok(when) = std::fs::metadata(entry.path()).and_then(|m| m.modified()) else { continue };
            let hit = match previous.remove(&file) {
                Some(hit) if hit.mtime == when => hit,
                _ => match read_marker(&entry.path(), when) {
                    Some(hit) => hit,
                    None => continue,
                },
            };
            if let (Some(j), Some(pid)) = (hit.jsonl.as_deref().filter(|j| !j.is_empty()), hit.pid) {
                if pids.contains(&pid) && Path::new(j).exists() && !exclude.contains(&real(j))
                    && best.as_ref().is_none_or(|(ts, _)| hit.ts > *ts) {
                    best = Some((hit.ts, j.to_string()));
                }
            }
            current.insert(file, hit);
        }
        self.markers.insert(dir, current);
        best.map(|(_, j)| j)
    }

    /// `_resolve_tracked_impl`, na mesma ordem: fd aberto, marcador pelo sid, trava do fd,
    /// `--session-id`, marcador pelo pid, cache e, por último, o mais novo do cwd (não confiável).
    fn resolve(&mut self, name: &str, cwd: &str, pane: Option<(i64, &Tree)>, procs: &dyn ProcessView,
               projects_dir: &Path, has_siblings: bool) -> Transcript {
        let tracked = |jsonl: String| Transcript { jsonl: Some(jsonl), tracked: true };
        if let Some((pid, tree)) = pane {
            // Transcripts que subagente ou daemon seguram abertos agora são de outra sessão lógica.
            let aux_open: HashSet<String> = tree.iter()
                .filter(|(_, c)| is_aux(c) && provider_from_argv(&split(c)).is_some())
                .filter_map(|(p, _)| open_jsonl(procs, p, &projects_of(procs, p, projects_dir)))
                .map(|j| real(&j)).collect();
            // 1. fd aberto do REPL: o transcript ativo agora, inclusive depois de um /clear. Só um
            //    CLI de agente abre transcript; ler o fd do resto da árvore era o passo mais caro.
            for (p, cmd) in tree.repl() {
                if provider_from_argv(&split(cmd)).is_none() {
                    continue;
                }
                if let Some(j) = open_jsonl(procs, p, &projects_of(procs, p, projects_dir)) {
                    self.jsonl.insert(name.into(), j.clone());
                    self.fd_locked.insert(name.into());
                    return tracked(j);
                }
            }
            // 1.5. Marcador pelo sid do cmdline: reescrito a cada evento, destrava o fd velho.
            for (p, cmd) in tree.repl() {
                let Some(sid) = session_id(cmd) else { continue };
                if let Some(marker) = active_marker(&config_base_of(procs, p, projects_dir), &sid, &aux_open) {
                    if self.jsonl.get(name) != Some(&marker) {
                        self.fd_locked.remove(name);
                    }
                    self.jsonl.insert(name.into(), marker.clone());
                    return tracked(marker);
                }
            }
            // Sem fd agora: o travado por fd segura o cache (o sid de um resume nunca vira arquivo).
            if self.fd_locked.contains(name) {
                if let Some(cached) = self.jsonl.get(name) {
                    return tracked(cached.clone());
                }
                self.fd_locked.remove(name);
            }
            // 2. --session-id: vale antes de o arquivo existir. Com irmã no mesmo cwd, o mais novo de
            //    uma contaminaria a outra; sozinha, segue o mais novo para pegar o pós-/clear.
            if let Some((p, sid)) = tree.repl().find_map(|(p, c)| session_id(c).map(|s| (p, s))) {
                let projdir = projects_of(procs, p, projects_dir).join(sanitize_cwd(cwd));
                let sid_jsonl = projdir.join(format!("{sid}.jsonl")).to_string_lossy().into_owned();
                let j = if has_siblings { sid_jsonl } else { newest_after_clear(&projdir, sid_jsonl, &aux_open) };
                self.jsonl.insert(name.into(), j.clone());
                return tracked(j);
            }
            // 2.5. Marcador casado por pid da árvore (sessão sem --session-id).
            let base = config_base_of(procs, pid, projects_dir);
            if let Some(marker) = self.marker_by_pids(&base, &tree.pids, &aux_open) {
                self.jsonl.insert(name.into(), marker.clone());
                return tracked(marker);
            }
        }
        // 3. Último sinal confiável: estabiliza quando o processo com --session-id some por um tempo.
        if let Some(cached) = self.jsonl.get(name) {
            return tracked(cached.clone());
        }
        // 4. O mais novo do cwd: ambíguo com várias sessões sem id no mesmo cwd, por isso não confiável.
        let projects = pane.map_or_else(|| projects_dir.to_path_buf(), |(p, _)| projects_of(procs, p, projects_dir));
        Transcript { jsonl: newest(&projects.join(sanitize_cwd(cwd))), tracked: false }
    }
}

/// `resolve_jsonl`: o `.jsonl` de mtime mais novo da pasta; em empate, o primeiro da varredura.
fn newest(dir: &Path) -> Option<String> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for f in jsonl_files(dir) {
        let mt = mtime(&f).unwrap_or(SystemTime::UNIX_EPOCH);
        if best.as_ref().is_none_or(|(b, _)| mt > *b) {
            best = Some((mt, f));
        }
    }
    best.map(|(_, f)| f.to_string_lossy().into_owned())
}

/// /clear rola um session-id novo sem mudar o cmdline: um `.jsonl` mais novo no projeto (que não
/// seja de auxiliar) é o transcript pós-clear. Sem o arquivo do sid ainda, confia nele.
fn newest_after_clear(projdir: &Path, sid_jsonl: String, exclude: &HashSet<String>) -> String {
    let Some(mut best_mt) = mtime(Path::new(&sid_jsonl)) else { return sid_jsonl };
    let mut best = sid_jsonl;
    for f in jsonl_files(projdir) {
        let path = f.to_string_lossy().into_owned();
        if !exclude.is_empty() && exclude.contains(&real(&path)) {
            continue;
        }
        if let Some(mt) = mtime(&f).filter(|mt| *mt > best_mt) {
            best = path;
            best_mt = mt;
        }
    }
    best
}

/// Agente estrito do pane (`agentpane._pane_do_agente`): nenhum casa = não é do agente.
fn pane_has_agent(procs: &dyn ProcessView, pid: i64, children: &ChildrenMap) -> bool {
    descendants(pid, children).into_iter().any(|p| {
        let cmd = procs.argv(p).join(" ");
        !is_aux(&cmd) && cmd.split_whitespace().next().is_some_and(|a| exec_provider(basename(a)).is_some())
    })
}

/// O pane que roda o agente entre os da sessão; o ativo desempata, e sem agente fica o ativo.
fn agent_pane<'p>(panes: &[&'p Pane], procs: &dyn ProcessView, children: &ChildrenMap) -> &'p Pane {
    if panes.len() > 1 {
        let mut by_active = panes.to_vec();
        by_active.sort_by_key(|p| !p.active);
        if let Some(p) = by_active.into_iter()
            .find(|p| p.pid.is_some_and(|pid| pane_has_agent(procs, pid.into(), children))) {
            return p;
        }
        let name = &panes[0].session;
        if crate::warn_limit::allow(Some(name), "list_no_agent_pane") {
            tracing::warn!(session = %name, panes = panes.len(), "nenhum pane parece do agente; usando o ativo");
        }
    }
    panes.iter().find(|p| p.active).copied().unwrap_or(panes[0])
}

/// Qual agente roda no pane e o pid dele (`agente_do_pane`); sem agente, "claude" e nenhum pid.
/// O argv íntegro, não o cmdline separado por espaço: no Windows o caminho do node tem espaço.
fn agent_of_pane(tree: Option<&Tree>) -> (&'static str, Option<i64>) {
    tree.into_iter()
        .flat_map(|t| t.pids.iter().zip(&t.argvs).zip(&t.cmds))
        .filter(|(_, cmd)| !is_aux(cmd))
        .find_map(|((p, argv), _)| provider_from_argv(argv).map(|prov| (prov, Some(*p))))
        .unwrap_or(("claude", None))
}

/// Uma linha por sessão do multiplexador, na ordem do `list-panes`, sem as escondidas e sem as que
/// `skip` tira pelo nome (sidecar Codex de mesmo nome, `registry.py:1331`): estas nem passam pela
/// resolução, para não mexer no cache de quem não aparece.
pub fn discover_panes(panes: &[Pane], procs: &dyn ProcessView, children: &ChildrenMap, projects_dir: &Path,
                      resolver: &mut Resolver, skip: &dyn Fn(&str) -> bool) -> Vec<PaneSession> {
    let mut groups: Vec<Vec<&Pane>> = Vec::new();
    for pane in panes {
        match groups.iter_mut().find(|g| g[0].session == pane.session) {
            Some(group) => group.push(pane),
            None => groups.push(vec![pane]),
        }
    }
    // A marca de escondida é da sessão: a do shell nasce no cwd do agente e o tiraria do pós-/clear.
    groups.retain(|g| !g[0].hidden);
    let mut out = Vec::new();
    for group in &groups {
        let p = agent_pane(group, procs, children);
        if skip(&p.session) {
            continue;
        }
        // pid 0 tem a máquina inteira embaixo no mapa.
        let pid = p.pid.filter(|pid| *pid != 0).map(i64::from);
        let tree = pid.map(|pid| Tree::new(procs, pid, children));
        let (mut provider, agent_pid) = agent_of_pane(tree.as_ref());
        // Durante o boot só há shell: a escolha da criação já identifica o dono.
        if agent_pid.is_none() {
            if let Some(chosen) = p.provider.as_deref().and_then(exec_provider) {
                provider = chosen;
            }
        }
        let transcript = (provider == "claude").then(|| {
            let siblings = groups.iter().filter(|g| g.iter().any(|q| q.cwd == p.cwd)).count() > 1;
            resolver.resolve(&p.session, &p.cwd, pid.zip(tree.as_ref()), procs, projects_dir, siblings)
        });
        out.push(PaneSession {
            name: p.session.clone(), cwd: p.cwd.clone(), pane_id: p.pane_id.clone(), pane_pid: pid, agent_pid,
            provider, session_created: p.session_created, transcript,
            repl_sid: tree.as_ref().and_then(Tree::repl_sid),
        });
    }
    // Teto do cache por nome: acima dele, quem não apareceu nesta rodada sai. O caminho normal é
    // o `forget` de quem fecha a sessão.
    if resolver.jsonl.len() > SESSION_CAP || resolver.fd_locked.len() > SESSION_CAP {
        let live: HashSet<&str> = groups.iter().map(|g| g[0].session.as_str()).collect();
        resolver.jsonl.retain(|n, _| live.contains(n.as_str()));
        resolver.fd_locked.retain(|n| live.contains(n.as_str()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::sync::Arc;
    use std::time::Duration;

    struct NoProcs;
    impl ProcessView for NoProcs {
        fn children(&self, _: Duration) -> io::Result<Arc<ChildrenMap>> { Ok(Arc::default()) }
        fn argv(&self, _: i64) -> Vec<String> { Vec::new() }
        fn cwd(&self, _: i64) -> Option<PathBuf> { None }
        fn env_var(&self, _: i64, _: &str) -> io::Result<Option<OsString>> { Ok(None) }
        fn start_time(&self, _: i64) -> Option<f64> { None }
        fn fds(&self, _: i64) -> Vec<PathBuf> { Vec::new() }
    }

    fn pane(name: &str, cwd: &str) -> Pane {
        Pane { session: name.into(), active: true, pid: None, cwd: cwd.into(), pane_id: "%1".into(), hidden: false,
            provider: None, session_created: None, ..Pane::default() }
    }

    #[test]
    fn seed_then_forget() {
        let dir = tempfile::tempdir().unwrap();
        let projects = dir.path().join("projects");
        let old = projects.join(sanitize_cwd("/w")).join("old.jsonl");
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        std::fs::write(&old, "{}\n").unwrap();
        let mut r = Resolver::default();
        let run = |r: &mut Resolver, name: &str| {
            discover_panes(&[pane(name, "/w")], &NoProcs, &ChildrenMap::new(), &projects, r, &|_| false)[0]
                .transcript.clone().unwrap()
        };
        r.seed("a", "/seeded.jsonl");
        assert_eq!(run(&mut r, "a"), Transcript { jsonl: Some("/seeded.jsonl".into()), tracked: true });
        r.rename("a", "b");
        assert_eq!(run(&mut r, "b").jsonl.as_deref(), Some("/seeded.jsonl"), "o cache segue o nome novo");
        assert!(!r.cached().contains_key("a"));
        r.forget("b");
        assert_eq!(run(&mut r, "b"), Transcript { jsonl: Some(old.to_string_lossy().into()), tracked: false },
            "esquecida volta ao mais novo do cwd, sem vínculo");
    }

    #[test]
    fn name_cache_has_a_ceiling() {
        let mut r = Resolver::default();
        for i in 0..=SESSION_CAP {
            r.seed(&format!("gone{i}"), "/x.jsonl");
        }
        r.seed("live", "/live.jsonl");
        discover_panes(&[pane("live", "/w")], &NoProcs, &ChildrenMap::new(), Path::new("/p"), &mut r, &|_| false);
        assert_eq!(r.cached().keys().collect::<Vec<_>>(), ["live"], "acima do teto só fica quem apareceu");
    }

    #[test]
    fn provider_from_argv_cases() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(provider_from_argv(&v(&["/usr/bin/claude", "--session-id", "x"])), Some("claude"));
        assert_eq!(provider_from_argv(&v(&["python3.14", "/x/hangar-codex-tui"])), Some("codex"));
        assert_eq!(provider_from_argv(&v(&["node", "/n/node_modules/@e/pi-coding-agent/dist/cli.js"])), Some("pi"));
        assert_eq!(provider_from_argv(&v(&["node", "mcp.js"])), None);
        assert_eq!(provider_from_argv(&[]), None);
    }
}
