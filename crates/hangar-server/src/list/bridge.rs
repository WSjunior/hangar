//! Ponte privada `list.*` (Python → Rust): com o Rust de pé, a descoberta, o cache de resolução do
//! transcript e o retrato da lista são dele; o Python pergunta por aqui (`list_bridge.py`), na porta
//! privada, com o mesmo segredo da ponte de Git/arquivos. Falha volta com código, nunca lista vazia.
use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::{
    body::to_bytes,
    extract::{ConnectInfo, Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use hangar_api::session::SessionRow;
use serde::Deserialize;
use serde_json::{Value, json};

use super::classify::{CaptureSource, Classifier, Effect, Facts, MuxCapture};
use super::context::{self, ContextCache, ReadingInputs};
use super::discover::{self, Resolver};
use super::discover_other::{self, Dirs};
use super::facts::{self as list_facts, FactsClient, ListFacts};
use super::facts_files::{self, HookStates};
use super::mux::{Mux, Pane};
use super::plan::PlanTracker;
use super::procs::{self, ChildrenMap, ProcessView};
use super::reply::ReplyCache;
use crate::routes::AppState;

/// Descoberta reaproveitada por quem pede dentro deste prazo (`_LIST_TTL` do `api.py`).
pub const DISCOVER_TTL: Duration = Duration::from_secs(1);
/// Retrato decorado servido sem produzir de novo (`list_sessions` do `api.py`).
pub const SNAPSHOT_TTL: Duration = Duration::from_secs(2);
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(5);
const CONFIG_DIRS_TTL: Duration = Duration::from_secs(30);
const MAX_BODY: usize = 64 * 1024;
/// `git status` simultâneos da lista (`_git_pool`).
const GIT_SLOTS: usize = 4;

/// Falha da ponte: o código vai ao Python, que levanta (`mux_unavailable` vira `MuxIndisponivel`).
/// `detail` é código ou frase fixa, nunca saída de processo nem texto de conversa.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListError { pub code: &'static str, pub detail: &'static str }

fn fail(code: &'static str, detail: &'static str) -> ListError { ListError { code, detail } }

/// Entradas do sistema: multiplexador, processos e as pastas que o Python resolveu
/// (`HANGAR_LIST_DIRS`). Sem pastas, a ponte recusa em vez de adivinhar onde cada provedor grava.
pub struct ListEnv {
    pub mux: Mux,
    pub capture_program: OsString,
    pub procs: Arc<dyn ProcessView>,
    pub dirs: Option<Dirs>,
}

impl ListEnv {
    pub fn from_env() -> Self {
        let raw = std::env::var("HANGAR_LIST_DIRS").unwrap_or_default();
        let dirs = parse_dirs(&raw);
        if dirs.is_none() {
            // Cada pergunta da ponte vai recusar com `list_dirs_missing`; a causa fica dita uma vez.
            tracing::error!(code = "list_dirs_missing", present = !raw.is_empty(), "pastas da lista ausentes ou inválidas");
        }
        Self { mux: Mux::default(), capture_program: "tmux".into(), procs: Arc::new(procs::SystemProcs::default()), dirs }
    }
}

/// `{"home", "claude", "codex_home", "pi_sessions", "omp_config", "omp_agent", "kimi_home"}`;
/// faltou um campo, nenhum vale.
pub fn parse_dirs(raw: &str) -> Option<Dirs> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let p = |k: &str| v[k].as_str().filter(|s| !s.is_empty()).map(PathBuf::from);
    Some(Dirs { home: p("home")?, claude: p("claude")?, codex_home: p("codex_home")?, pi_sessions: p("pi_sessions")?,
        omp_config: p("omp_config")?, omp_agent: p("omp_agent")?, kimi_home: p("kimi_home")? })
}

/// O que quem produz sabe e o Python não: o runtime das sessões sem terminal (o hub completa,
/// Task 17; vazio, elas ficam no marcador) e quantas listas do dono estão abertas no Rust.
#[derive(Default)]
pub struct ProduceFacts {
    /// Retrato do runtime das sessões sem terminal, por nome (`RuntimeRegistry::snapshots`).
    pub headless: BTreeMap<String, Value>,
    pub owner_clients: u32,
    /// Rodada em sombra: nada do que ela produz sai daqui, nem o rebaixamento de `awaiting`.
    pub shadow: bool,
}

/// Lista decorada e os fatos do Python da mesma rodada (navegador, terminais de atalho, escondidas
/// do dono), que o hub entrega junto.
#[derive(Clone)]
pub struct Produced { pub rows: Arc<Vec<SessionRow>>, pub facts: Arc<ListFacts>,
    /// `false`: o Python não respondeu nesta rodada e `facts` é o último bom.
    pub facts_ok: bool }

struct Discovery { at: Instant, wall: f64, epoch: u64,
    /// Mapa de processos relido nesta descoberta, não o do cache de 3 s.
    fresh: bool, rows: Arc<Vec<SessionRow>>, agent_pids: Arc<HashMap<String, u32>>,
    panes: Arc<Vec<Pane>>, children: Arc<ChildrenMap> }

struct Snapshot { at: Instant, epoch: u64, produced: Produced }

/// Caches que atravessam rodadas, por nome de sessão. Uma trava só: semear e esquecer não podem
/// cair no meio de uma resolução.
#[derive(Default)]
struct Caches {
    resolver: Resolver,
    context: ContextCache,
    replies: ReplyCache,
    plans: PlanTracker,
    /// Último resumo de Git por pasta: a lista não espera o `git status` (`_git_ultimo`).
    git: HashMap<String, (Value, Value)>,
    config_dirs: Option<(Instant, Arc<Vec<PathBuf>>)>,
    /// Marcadores e registro nativo entre rodadas: só o arquivo que mudou é relido.
    hooks: HookStates,
}

pub struct ListBridge {
    env: Arc<ListEnv>,
    facts: FactsClient,
    shadow_facts: FactsClient,
    caches: Arc<Mutex<Caches>>,
    /// À parte dos outros caches: a classificação segura a dela durante as capturas.
    classifier: Arc<Mutex<Classifier>>,
    /// Trava assíncrona = um por vez: quem chega durante a varredura espera e reaproveita o resultado.
    discovery: tokio::sync::Mutex<Option<Discovery>>,
    snapshot: tokio::sync::Mutex<Option<Snapshot>>,
    git_running: Arc<Mutex<std::collections::HashSet<String>>>,
    git_slots: Arc<tokio::sync::Semaphore>,
    epoch: AtomicU64,
}

/// Tarefa bloqueante que entrou em pânico: o hook já registrou onde; aqui fica qual operação.
fn joined(e: tokio::task::JoinError, detail: &'static str) -> ListError {
    tracing::error!(code = "list_task_failed", detail, panic = e.is_panic(), "ponte da lista: tarefa interrompida");
    fail("list_task_failed", detail)
}

fn wall_now() -> f64 { SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64()) }

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> { m.lock().unwrap_or_else(|e| e.into_inner()) }

impl ListBridge {
    pub fn new(env: ListEnv, facts: FactsClient) -> Self {
        Self { env: Arc::new(env), shadow_facts: facts.sibling(), facts, caches: Arc::default(), classifier: Arc::default(), discovery: tokio::sync::Mutex::new(None),
            snapshot: tokio::sync::Mutex::new(None), git_running: Arc::default(),
            git_slots: Arc::new(tokio::sync::Semaphore::new(GIT_SLOTS)), epoch: AtomicU64::new(0) }
    }

    fn dirs(&self) -> Result<Dirs, ListError> {
        self.env.dirs.clone().ok_or(fail("list_dirs_missing", "pastas da lista ausentes"))
    }

    /// Mudança de membro ou de modo: a próxima pergunta não serve a lista de antes.
    pub fn invalidate(&self) { self.epoch.fetch_add(1, Ordering::SeqCst); }

    /// `registry.list()`. `newer_than` (época em s) é a sessão criada há menos de 1 s: só vale uma
    /// descoberta que começou depois disso, com o mapa de processos relido.
    pub async fn discover(&self, newer_than: Option<f64>) -> Result<Arc<Vec<SessionRow>>, ListError> {
        Ok(self.discovery(newer_than).await?.0)
    }

    async fn discovery(&self, newer_than: Option<f64>)
        -> Result<(Arc<Vec<SessionRow>>, Arc<HashMap<String, u32>>, Arc<Vec<Pane>>, Arc<ChildrenMap>), ListError> {
        let mut slot = self.discovery.lock().await;
        let epoch = self.epoch.load(Ordering::SeqCst);
        if let Some(d) = slot.as_ref().filter(|d| d.epoch == epoch && match newer_than {
            Some(t) => d.fresh && d.wall > t,
            None => d.at.elapsed() < DISCOVER_TTL,
        }) {
            return Ok((d.rows.clone(), d.agent_pids.clone(), d.panes.clone(), d.children.clone()));
        }
        let dirs = self.dirs()?;
        let (at, wall) = (Instant::now(), wall_now());
        let panes = Arc::new(self.env.mux.list_panes().await.map_err(|e| fail("mux_unavailable", e.code))?);
        let (env, caches, p) = (self.env.clone(), self.caches.clone(), panes.clone());
        let max_age = if newer_than.is_some() { Duration::ZERO } else { procs::CHILDREN_TTL };
        let (rows, agent_pids, children) = tokio::task::spawn_blocking(move || {
            let children = env.procs.children(max_age).map_err(|_| fail("list_procs_unreadable", "mapa de processos ilegível"))?;
            let mut c = lock(&caches);
            let (rows, pids) = run_discovery(&p, &*env.procs, &children, &mut c.resolver, &dirs);
            Ok::<_, ListError>((Arc::new(rows), Arc::new(pids), children))
        }).await.map_err(|e| joined(e, "descoberta interrompida"))??;
        *slot = Some(Discovery { at, wall, epoch, fresh: newer_than.is_some(), rows: rows.clone(), agent_pids: agent_pids.clone(),
            panes: panes.clone(), children: children.clone() });
        Ok((rows, agent_pids, panes, children))
    }

    /// Lista decorada para quem pergunta fora do hub (vigia de travada, `prune`, convidado): o retrato
    /// de até 2 s, senão produz na hora. Antes da Task 16 nenhum consumidor lê isto.
    pub async fn snapshot(&self, facts: &ProduceFacts) -> Result<Produced, ListError> {
        let mut slot = self.snapshot.lock().await;
        let epoch = self.epoch.load(Ordering::SeqCst);
        if let Some(s) = slot.as_ref().filter(|s| s.epoch == epoch && s.at.elapsed() < SNAPSHOT_TTL) {
            return Ok(s.produced.clone());
        }
        let at = Instant::now();
        let produced = self.produce(facts).await?;
        *slot = Some(Snapshot { at, epoch, produced: produced.clone() });
        Ok(produced)
    }

    /// A produção da lista: descoberta + fatos do Python + classificação + contexto, resposta,
    /// plano, loop e Git. É A função que o hub (Task 17) chama a cada tique; quem quer retrato pede
    /// `snapshot`, que segura a produção em um por vez.
    ///
    /// Linhas Codex, Pi, omp e Kimi levam o estado dos fatos; as de transferência em curso e as
    /// `orq` saem como o Python as deu, sem classificação nem decoração, no fim da lista.
    pub async fn produce(&self, input: &ProduceFacts) -> Result<Produced, ListError> {
        let dirs = self.dirs()?;
        let (rows, agent_pids, panes, children) = self.discovery(None).await?;
        let client = if input.shadow { &self.shadow_facts } else { &self.facts };
        let fetched = client.fetch(&rows, input.owner_clients, &pi_pane_pids(&rows, &panes), input.shadow).await;
        let (mut rows, aside) = list_facts::apply((*rows).clone(), &fetched.facts, fetched.ok);
        let targets = pane_targets(&panes, &agent_pids, &children);
        let (env, caches, classifier) = (self.env.clone(), self.caches.clone(), self.classifier.clone());
        let headless = input.headless.clone();
        let py = fetched.facts.clone();
        let handle = tokio::runtime::Handle::current();
        // Classificação e decoração leem arquivo (marcador, transcript, plano) e esperam captura:
        // fora da thread do runtime, que atende todas as conexões.
        let (rows, effects, git_dirs) = tokio::task::spawn_blocking(move || {
            // Tirado da trava durante a classificação; duas produções juntas só pagam uma releitura a mais.
            let (config_dirs, mut hooks) = {
                let mut c = lock(&caches);
                (c.config_dirs(&dirs), std::mem::take(&mut c.hooks))
            };
            hooks.refresh(&config_dirs);
            let alive = |pid: i64| pid_alive(&*env.procs, pid);
            let facts = Facts { hooks: &hooks, alive: &alive, config_dirs: &config_dirs, headless: &headless,
                problems: &py.problems, stall_seconds: py.stall_seconds };
            let io = MuxCapture::new(env.capture_program.clone(), CAPTURE_TIMEOUT, targets);
            let effects = handle.block_on(lock(&classifier).classify(&mut rows, &facts, &io));
            let (wall, mono) = (io.wall(), io.mono());
            let mut c = lock(&caches);
            c.hooks = hooks;
            for row in rows.iter_mut().filter(|r| r.provider == "claude") {
                let pid = agent_pids.get(&row.name).map(|p| i64::from(*p));
                decorate_context(&mut c.context, row, pid, &*env.procs, &dirs, &config_dirs, wall, mono);
            }
            for (name, error) in c.replies.decorate(&mut rows, |_| None) {
                if crate::warn_limit::allow(Some(&name), "list_reply_unreadable") {
                    tracing::warn!(session = %name, kind = ?error.kind(), "lista: última resposta ilegível");
                }
            }
            for row in rows.iter_mut() {
                c.plans.decorate(row, wall, mono);
                super::links::fill_loop(row, &dirs);
                let (summary, diff) = c.git.get(git_dir(row)).cloned().unwrap_or_default();
                apply_git(row, &summary, &diff);
            }
            let git_dirs: Vec<String> = rows.iter().map(|r| git_dir(r).to_owned()).filter(|d| !d.is_empty()).collect();
            // Pasta sem sessão sai: o cache não cresce com cada repositório que já passou pela lista.
            c.git.retain(|d, _| git_dirs.contains(d));
            (rows, effects, git_dirs)
        }).await.map_err(|e| joined(e, "produção interrompida"))?;
        self.refresh_git(git_dirs);
        let demote: Vec<String> = effects.into_iter().map(|Effect::DemoteAwaiting { sid }| sid).collect();
        if !demote.is_empty() && !input.shadow {
            self.facts.demote(demote);
        }
        let mut rows = rows;
        rows.extend(aside);
        Ok(Produced { rows: Arc::new(rows), facts: fetched.facts, facts_ok: fetched.ok })
    }

    /// Git em segundo plano, um por pasta: um repositório lento não atrasa o card de ninguém. Não
    /// roda `git` a cada tique: `git::summary` guarda o resultado por pasta por 3 s.
    fn refresh_git(&self, dirs: Vec<String>) {
        for dir in dirs {
            if !lock(&self.git_running).insert(dir.clone()) {
                continue;
            }
            let (caches, running, slots) = (self.caches.clone(), self.git_running.clone(), self.git_slots.clone());
            tokio::spawn(async move {
                let Ok(_permit) = slots.acquire_owned().await else {
                    lock(&running).remove(&dir);
                    return;
                };
                let key = dir.clone();
                let done = tokio::task::spawn_blocking(move || {
                    let summary = hangar_workspace::git::summary(Some(&dir), false);
                    let diff = hangar_workspace::git::summary(Some(&dir), true);
                    let mut c = lock(&caches);
                    let before = c.git.remove(&dir).unwrap_or_default();
                    // Consulta que falhou fica com o último número bom.
                    let keep = |new: Value, old: Value| if new.is_null() { old } else { new };
                    c.git.insert(dir, (keep(summary, before.0), keep(diff, before.1)));
                }).await;
                if done.is_err() && crate::warn_limit::allow(None, "list_git_failed") {
                    tracing::warn!(code = "list_git_failed", "lista: resumo de Git interrompido; fica o último");
                }
                // Mesmo depois de um pânico: senão a pasta nunca mais teria Git.
                lock(&running).remove(&key);
            });
        }
    }

    pub async fn resolve(&self, name: &str, cwd: &str, pid: Option<i64>) -> Result<discover::Transcript, ListError> {
        let dirs = self.dirs()?;
        let panes = self.env.mux.list_panes().await.map_err(|e| fail("mux_unavailable", e.code))?;
        let (env, caches, name, cwd) = (self.env.clone(), self.caches.clone(), name.to_owned(), cwd.to_owned());
        tokio::task::spawn_blocking(move || {
            let children = env.procs.children(procs::CHILDREN_TTL).map_err(|_| fail("list_procs_unreadable", "mapa de processos ilegível"))?;
            let mut c = lock(&caches);
            Ok(discover::resolve_one(&panes, &*env.procs, &children, &dirs.claude.join("projects"), &mut c.resolver, &name, &cwd, pid))
        }).await.map_err(|e| joined(e, "resolução interrompida"))?
    }

    /// Bloqueia até a rodada em curso soltar os caches: chamar fora da thread do runtime.
    pub fn seed(&self, name: &str, jsonl: &str) { lock(&self.caches).resolver.seed(name, jsonl); }

    /// `_forget`: nome reusado por outra sessão não herda nada da morta. Bloqueia como `seed`.
    pub fn forget(&self, name: &str) {
        {
            let mut c = lock(&self.caches);
            c.resolver.forget(name);
            c.context.forget(name);
            c.replies.forget(name);
        }
        lock(&self.classifier).forget(name);
    }

    /// Bloqueia como `seed`.
    pub fn rename(&self, old: &str, new: &str) {
        {
            let mut c = lock(&self.caches);
            c.resolver.rename(old, new);
            c.context.forget(old);
            c.replies.rename(old, new);
        }
        lock(&self.classifier).rename(old, new);
    }
}

impl Caches {
    /// Pastas de conta (`list_config_dirs` + a base do backend): `CP_CLAUDE_CONFIG_DIRS` ou
    /// `~/.claude*`. Sobrar pasta só custa um `read_dir` vazio; faltar some com o marcador.
    fn config_dirs(&mut self, dirs: &Dirs) -> Arc<Vec<PathBuf>> {
        if let Some((at, v)) = &self.config_dirs && at.elapsed() < CONFIG_DIRS_TTL {
            return v.clone();
        }
        let mut found: Vec<PathBuf> = match std::env::var("CP_CLAUDE_CONFIG_DIRS").ok().filter(|s| !s.trim().is_empty()) {
            Some(raw) => raw.split(',').map(str::trim).filter(|s| !s.is_empty())
                .map(|item| expand_home(&labelled_path(item), &dirs.home)).collect(),
            None => match std::fs::read_dir(&dirs.home) {
                Ok(entries) => entries.flatten()
                    .filter(|e| e.file_name().to_string_lossy().starts_with(".claude") && e.path().is_dir())
                    .map(|e| e.path()).collect(),
                Err(e) => {
                    // Sem as outras contas os marcadores delas somem: avisa e não guarda, tenta de novo no tique.
                    if crate::warn_limit::allow(None, "list_config_dirs_unreadable") {
                        tracing::warn!(code = "list_config_dirs_unreadable", kind = ?e.kind(), "pastas de conta ilegíveis");
                    }
                    return Arc::new(vec![dirs.claude.clone()]);
                }
            },
        };
        found.push(dirs.claude.clone());
        let mut seen = std::collections::HashSet::new();
        found.retain(|p| seen.insert(std::fs::canonicalize(p).unwrap_or_else(|_| p.clone())));
        let v = Arc::new(found);
        self.config_dirs = Some((Instant::now(), v.clone()));
        v
    }
}

fn expand_home(path: &str, home: &Path) -> PathBuf {
    match path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        Some(rest) => home.join(rest),
        None if path == "~" => home.to_owned(),
        None => PathBuf::from(path),
    }
}

/// `rótulo:caminho` ou só o caminho; letra única seguida de barra é drive do Windows.
fn labelled_path(item: &str) -> String {
    match item.split_once(':') {
        Some((label, path)) if !(cfg!(windows) && label.len() == 1 && path.starts_with(['\\', '/'])) => path.trim().to_owned(),
        _ => item.to_owned(),
    }
}

/// Linhas descobertas e o pid do agente de cada uma (contexto de abertura e alvo da captura).
fn run_discovery(panes: &[Pane], procs: &dyn ProcessView, children: &ChildrenMap, resolver: &mut Resolver, dirs: &Dirs)
    -> (Vec<SessionRow>, HashMap<String, u32>) {
    let found = discover_other::discover_rows(panes, procs, children, resolver, dirs);
    (found.rows, found.agent_pids)
}

/// Pid do pane das linhas Pi e omp: o sidecar do catálogo, de onde sai a conta, mora no
/// `CLAUDE_CONFIG_DIR` dele.
fn pi_pane_pids(rows: &[SessionRow], panes: &[Pane]) -> BTreeMap<String, u32> {
    rows.iter().filter(|r| r.provider == "pi" || r.provider == "omp").filter_map(|r| {
        let pane = panes.iter().filter(|p| p.session == r.name).max_by_key(|p| p.active)?;
        Some((r.name.clone(), pane.pid?))
    }).collect()
}

/// Alvo da captura de cada sessão: o pane do agente; sem como saber, `=<sessão>:` (o ativo).
fn pane_targets(panes: &[Pane], agent_pids: &HashMap<String, u32>, children: &ChildrenMap) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for pane in panes {
        let mine = panes.iter().filter(|p| p.session == pane.session).count() == 1
            || agent_pids.get(&pane.session).is_some_and(|a| pane.pid.is_some_and(|p| descends(i64::from(*a), i64::from(p), children)));
        if let (true, Some(t)) = (mine, pane.target()) {
            out.insert(pane.session.clone(), t);
        }
    }
    out
}

fn descends(pid: i64, root: i64, children: &ChildrenMap) -> bool {
    let mut stack = vec![root];
    let mut seen = std::collections::HashSet::new();
    while let Some(p) = stack.pop() {
        if p == pid {
            return true;
        }
        if seen.insert(p) {
            stack.extend(children.get(&p).into_iter().flatten());
        }
    }
    false
}

fn pid_alive(procs: &dyn ProcessView, pid: i64) -> bool { pid > 0 && procs.start_time(pid).is_some() }

fn git_dir(row: &SessionRow) -> &str { row.git_cwd.as_deref().or(row.cwd.as_deref()).unwrap_or("") }

fn apply_git(row: &mut SessionRow, summary: &Value, diff: &Value) {
    let n = |v: &Value, k: &str| v[k].as_u64().and_then(|x| u32::try_from(x).ok());
    if !summary.is_null() {
        (row.git_dirty, row.git_ahead, row.git_behind) = (n(summary, "dirty"), n(summary, "ahead"), n(summary, "behind"));
    }
    if !diff.is_null() {
        (row.git_added, row.git_removed) = (n(diff, "added"), n(diff, "removed"));
    }
}

/// `_claude_reading` com o cache de 20 s por (nome, transcript).
#[allow(clippy::too_many_arguments)]
fn decorate_context(cache: &mut ContextCache, row: &mut SessionRow, pid: Option<i64>, procs: &dyn ProcessView,
                    dirs: &Dirs, config_dirs: &[PathBuf], wall: f64, mono: f64) {
    let Some(jsonl) = row.jsonl.clone() else { return };
    let (ctx, model) = if cache.stale(&row.name, &jsonl, mono) {
        let stem = Path::new(&jsonl).file_stem().and_then(|s| s.to_str()).map(str::to_owned);
        let chosen = facts_files::published_status(stem.as_deref(), config_dirs, wall).and_then(|p| p.model);
        let (opened, declared) = if row.headless {
            let meta = std::fs::read(dirs.home.join(".hangar/claude-headless").join(format!("{}.json", row.name)))
                .ok().and_then(|raw| serde_json::from_slice::<Value>(&raw).ok()).unwrap_or(Value::Null);
            (meta["model"].as_str().map(str::to_owned), context::declared_window_value(&meta["context_window"]))
        } else if let Some(pid) = pid {
            let declared = procs.env_var(pid, "CLAUDE_CODE_MAX_CONTEXT_TOKENS").ok().flatten()
                .and_then(|v| context::declared_window(&v.to_string_lossy()));
            (context::opened_model(&procs.argv(pid)), declared)
        } else {
            (None, None)
        };
        let account_dir = context::config_dir_of(row.conta.as_deref()).unwrap_or_else(|| dirs.claude.clone());
        let reading = context::claude_reading(&ReadingInputs { jsonl: Path::new(&jsonl), account_dir: &account_dir,
            chosen: chosen.as_deref(), opened: opened.as_deref(), declared, engine: row.engine.is_some() });
        cache.store(&row.name, &jsonl, mono, reading)
    } else {
        cache.cached(&row.name, &jsonl)
    };
    (row.context, row.model) = (ctx, model);
}

#[derive(Deserialize)]
#[serde(tag = "op", content = "args")]
enum Operation {
    #[serde(rename = "list.discover")]
    Discover { #[serde(default)] newer_than: Option<f64> },
    #[serde(rename = "list.snapshot")]
    Snapshot {},
    #[serde(rename = "list.invalidate")]
    Invalidate {},
    #[serde(rename = "list.resolve")]
    Resolve { name: String, cwd: String, #[serde(default)] pid: Option<i64> },
    #[serde(rename = "list.seed")]
    Seed { name: String, jsonl: String },
    #[serde(rename = "list.forget")]
    Forget { name: String },
    #[serde(rename = "list.rename")]
    Rename { old: String, new: String },
}

async fn execute(bridge: &Arc<ListBridge>, op: Operation) -> Result<Value, ListError> {
    let rows = |r: &[SessionRow]| serde_json::to_value(r).map_err(|_| fail("list_task_failed", "linha sem serializar"));
    let cache = |f: Box<dyn FnOnce(&ListBridge) + Send>| {
        let bridge = bridge.clone();
        async move {
            tokio::task::spawn_blocking(move || f(&bridge)).await
                .map(|()| Value::Null).map_err(|e| joined(e, "cache interrompido"))
        }
    };
    match op {
        Operation::Discover { newer_than } => rows(&bridge.discover(newer_than).await?),
        Operation::Snapshot {} => rows(&bridge.snapshot(&ProduceFacts::default()).await?.rows),
        Operation::Invalidate {} => { bridge.invalidate(); Ok(Value::Null) }
        Operation::Resolve { name, cwd, pid } => {
            let t = bridge.resolve(&name, &cwd, pid).await?;
            Ok(json!({"jsonl": t.jsonl, "tracked": t.tracked}))
        }
        Operation::Seed { name, jsonl } => cache(Box::new(move |b| b.seed(&name, &jsonl))).await,
        Operation::Forget { name } => cache(Box::new(move |b| b.forget(&name))).await,
        Operation::Rename { old, new } => cache(Box::new(move |b| b.rename(&old, &new))).await,
    }
}

fn reply(value: Value) -> Response {
    (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], value.to_string()).into_response()
}

pub async fn private(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    if !crate::workspace_routes::private_ok(&st, peer, req.headers()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let refused = |code: &'static str| {
        if crate::warn_limit::allow(None, code) {
            tracing::warn!(code, "ponte da lista recusou o pedido");
        }
        StatusCode::BAD_REQUEST.into_response()
    };
    let Ok(Ok(bytes)) = tokio::time::timeout(Duration::from_secs(6), to_bytes(req.into_body(), MAX_BODY)).await else {
        return refused("list_bridge_body");
    };
    // Operação desconhecida aqui costuma ser Python e Rust de versões diferentes.
    let Ok(op) = serde_json::from_slice::<Operation>(&bytes) else {
        return refused("list_bridge_invalid_request");
    };
    match execute(&st.list, op).await {
        Ok(result) => reply(json!({"ok": true, "result": result})),
        Err(e) => {
            if crate::warn_limit::allow(None, e.code) {
                tracing::warn!(code = e.code, detail = e.detail, "ponte da lista falhou");
            }
            reply(json!({"ok": false, "error": {"code": e.code, "detail": e.detail}}))
        }
    }
}
