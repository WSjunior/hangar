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
use super::capped;
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
#[derive(Clone, Default)]
pub struct ProduceFacts {
    /// Retrato do runtime das sessões sem terminal, por nome (`RuntimeRegistry::snapshots`).
    /// `None`: ninguém forneceu o retrato; `Some` sem a sessão: ela está parada.
    pub headless: Option<BTreeMap<String, Value>>,
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

type Op<T> = Box<dyn FnOnce(&mut T) + Send>;

/// Valor que a produção segura durante I/O longa (captura de até 5 s, rabo de transcript) e que
/// criar, fechar e renomear sessão mudam sem esperar: a mudança entra numa fila curta e quem segura
/// o valor a aplica antes de soltar; com o valor livre, quem pediu aplica na hora.
struct Guarded<T> { value: Mutex<T>, ops: Mutex<Vec<Op<T>>> }

impl<T: Default> Default for Guarded<T> {
    fn default() -> Self { Self { value: Mutex::default(), ops: Mutex::default() } }
}

impl<T> Guarded<T> {
    /// Para quem lê e grava o valor; produções juntas esperam uma à outra aqui.
    fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let mut v = lock(&self.value);
        self.drain(&mut v);
        let out = f(&mut v);
        // O que chegou durante a I/O não espera a próxima rodada.
        self.drain(&mut v);
        out
    }

    fn drain(&self, v: &mut T) {
        let ops = std::mem::take(&mut *lock(&self.ops));
        for op in ops { op(v) }
    }

    /// Nunca espera a produção. A ordem dos pedidos é mantida.
    fn apply(&self, op: impl FnOnce(&mut T) + Send + 'static) {
        lock(&self.ops).push(Box::new(op));
        let held = match self.value.try_lock() {
            Ok(v) => Some(v),
            Err(std::sync::TryLockError::Poisoned(e)) => Some(e.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        };
        if let Some(mut v) = held { self.drain(&mut v); }
    }
}

/// O que a decoração das linhas Claude e de todas guarda entre rodadas.
#[derive(Default)]
struct Decor { context: ContextCache, replies: ReplyCache, plans: PlanTracker }

/// Marcadores junto da classificação: duas produções seguidas não releem tudo do zero.
#[derive(Default)]
struct Classify { classifier: Classifier, hooks: HookStates }

/// Caches que atravessam rodadas, por nome de sessão, cada um com a própria trava.
#[derive(Default)]
struct Caches {
    resolver: Guarded<Resolver>,
    decor: Guarded<Decor>,
    classify: Guarded<Classify>,
    /// Último resumo de Git por pasta: a lista não espera o `git status` (`_git_ultimo`).
    git: Mutex<HashMap<String, (Value, Value)>>,
    config_dirs: Mutex<Option<(Instant, Arc<Vec<PathBuf>>)>>,
}

pub struct ListBridge {
    env: Arc<ListEnv>,
    facts: FactsClient,
    shadow_facts: FactsClient,
    caches: Arc<Caches>,
    /// Os fatos do último `produce` de verdade: o retrato pedido de fora usa os mesmos, senão a
    /// pergunta ao Python alternaria de chave e o runtime sem terminal sumiria da classificação.
    last_input: Mutex<ProduceFacts>,
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
        Self { env: Arc::new(env), shadow_facts: facts.sibling(), facts, caches: Arc::default(), last_input: Mutex::default(),
            discovery: tokio::sync::Mutex::new(None),
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
        let (rows, agent_pids, children, problems) = tokio::task::spawn_blocking(move || {
            let children = env.procs.children(max_age).map_err(|_| fail("list_procs_unreadable", "mapa de processos ilegível"))?;
            let (rows, pids, problems) = caches.resolver.with(|r| run_discovery(&p, &*env.procs, &children, r, &dirs));
            Ok::<_, ListError>((Arc::new(rows), Arc::new(pids), children, problems))
        }).await.map_err(|e| joined(e, "descoberta interrompida"))??;
        for (code, key, reason) in problems {
            self.facts.diag.report("rust.list_discovery", &key, code, reason);
        }
        self.flush_notes();
        *slot = Some(Discovery { at, wall, epoch, fresh: newer_than.is_some(), rows: rows.clone(), agent_pids: agent_pids.clone(),
            panes: panes.clone(), children: children.clone() });
        Ok((rows, agent_pids, panes, children))
    }

    /// Lista decorada para quem pergunta fora do hub (vigia de travada, `prune`, convidado): o retrato
    /// de até 2 s, senão produz na hora com os fatos do último `produce` de verdade. Antes da Task 16
    /// nenhum consumidor lê isto. Task 17: o hub chama `produce` com o retrato do runtime e a contagem
    /// de clientes dele; até lá o retrato fica sem runtime (`None`) e com zero clientes.
    pub async fn snapshot(&self) -> Result<Produced, ListError> {
        let mut slot = self.snapshot.lock().await;
        let epoch = self.epoch.load(Ordering::SeqCst);
        if let Some(s) = slot.as_ref().filter(|s| s.epoch == epoch && s.at.elapsed() < SNAPSHOT_TTL) {
            return Ok(s.produced.clone());
        }
        let at = Instant::now();
        let input = lock(&self.last_input).clone();
        let produced = self.produce(&input).await?;
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
        if !input.shadow {
            *lock(&self.last_input) = input.clone();
        }
        let (rows, agent_pids, panes, children) = self.discovery(None).await?;
        let client = if input.shadow { &self.shadow_facts } else { &self.facts };
        let fetched = client.fetch(&rows, input.owner_clients, &pi_pane_pids(&rows, &panes), input.shadow).await;
        let (mut rows, aside) = list_facts::apply((*rows).clone(), &fetched.facts, fetched.ok);
        let targets = pane_targets(&panes, &agent_pids, &children);
        let (env, caches) = (self.env.clone(), self.caches.clone());
        let headless = input.headless.clone();
        let py = fetched.facts.clone();
        let handle = tokio::runtime::Handle::current();
        // Teto dos caches por sessão acompanha as linhas vivas: acima dele cada tique relia do zero.
        capped::set_live(rows.len() + aside.len());
        // Classificação e decoração leem arquivo (marcador, transcript, plano) e esperam captura:
        // fora da thread do runtime, que atende todas as conexões.
        let (rows, effects, git_dirs) = tokio::task::spawn_blocking(move || {
            let config_dirs = caches.config_dirs(&dirs);
            let alive = |pid: i64| pid_alive(&*env.procs, pid);
            let io = MuxCapture::new(env.capture_program.clone(), CAPTURE_TIMEOUT, targets);
            // Junção com a lista-tsombra: `Facts.headless` vira `Option` e recebe `headless.as_ref()`;
            // até lá `None` e `Some(vazio)` classificam igual.
            let no_runtime = BTreeMap::new();
            let effects = caches.classify.with(|Classify { classifier, hooks }| {
                hooks.refresh(&config_dirs);
                let facts = Facts { hooks, alive: &alive, config_dirs: &config_dirs,
                    headless: headless.as_ref().unwrap_or(&no_runtime), problems: &py.problems, stall_seconds: py.stall_seconds };
                handle.block_on(classifier.classify(&mut rows, &facts, &io))
            });
            let (wall, mono) = (io.wall(), io.mono());
            caches.decor.with(|d| {
                for row in rows.iter_mut().filter(|r| r.provider == "claude") {
                    let pid = agent_pids.get(&row.name).map(|p| i64::from(*p));
                    decorate_context(&mut d.context, row, pid, &*env.procs, &dirs, &config_dirs, wall, mono);
                }
                for (name, error) in d.replies.decorate(&mut rows, |_| None) {
                    if crate::warn_limit::allow(Some(&name), "list_reply_unreadable") {
                        tracing::warn!(session = %name, kind = ?error.kind(), "lista: última resposta ilegível");
                    }
                    list_facts::note("rust.list_reply_unreadable", &name, format!("{:?}", error.kind()), "última resposta ilegível");
                }
                for row in rows.iter_mut() {
                    d.plans.decorate(row, wall, mono);
                    super::links::fill_loop(row, &dirs);
                }
            });
            let git_dirs: Vec<String> = rows.iter().map(|r| git_dir(r).to_owned()).filter(|d| !d.is_empty()).collect();
            let mut git = lock(&caches.git);
            for row in rows.iter_mut() {
                let (summary, diff) = git.get(git_dir(row)).cloned().unwrap_or_default();
                apply_git(row, &summary, &diff);
            }
            // Pasta sem sessão sai: o cache não cresce com cada repositório que já passou pela lista.
            git.retain(|d, _| git_dirs.contains(d));
            drop(git);
            (rows, effects, git_dirs)
        }).await.map_err(|e| joined(e, "produção interrompida"))?;
        self.flush_notes();
        self.refresh_git(git_dirs);
        let demote: Vec<String> = effects.into_iter().map(|Effect::DemoteAwaiting { sid }| sid).collect();
        if !demote.is_empty() && !input.shadow {
            self.facts.demote(demote);
        }
        let mut rows = rows;
        rows.extend(aside);
        list_facts::mark_stale(&mut rows, &fetched.facts, fetched.ok);
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
            let diag = self.facts.diag.clone();
            tokio::spawn(async move {
                let Ok(_permit) = slots.acquire_owned().await else {
                    lock(&running).remove(&dir);
                    return;
                };
                let key = dir.clone();
                let done = tokio::task::spawn_blocking(move || {
                    let summary = hangar_workspace::git::summary(Some(&dir), false);
                    let diff = hangar_workspace::git::summary(Some(&dir), true);
                    // Sem `.git` o nulo é "não é repositório"; com ele, a consulta falhou.
                    let failed = (summary.is_null() || diff.is_null()) && Path::new(&dir).join(".git").exists();
                    let mut git = lock(&caches.git);
                    let before = git.remove(&dir).unwrap_or_default();
                    // Consulta que falhou fica com o último número bom, que segue o melhor valor.
                    let keep = |new: Value, old: Value| if new.is_null() { old } else { new };
                    git.insert(dir, (keep(summary, before.0), keep(diff, before.1)));
                    failed
                }).await;
                // A pasta no diário pelo nome, sem o caminho inteiro.
                let repo = Path::new(&key).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                match done {
                    Ok(false) => {}
                    Ok(true) => diag.report("rust.list_git_stale", &repo, "list_git_stale", "git da pasta falhou; fica o último número"),
                    Err(_) => diag.report("rust.list_git_stale", &repo, "list_git_failed", "resumo de Git interrompido; fica o último número"),
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
        let out = tokio::task::spawn_blocking(move || {
            let children = env.procs.children(procs::CHILDREN_TTL).map_err(|_| fail("list_procs_unreadable", "mapa de processos ilegível"))?;
            Ok(caches.resolver.with(|r| discover::resolve_one(&panes, &*env.procs, &children, &dirs.claude.join("projects"), r, &name, &cwd, pid)))
        }).await.map_err(|e| joined(e, "resolução interrompida"))?;
        self.flush_notes();
        out
    }

    /// Falhas vistas pela leitura síncrona da rodada, ao diário.
    fn flush_notes(&self) {
        for n in list_facts::take_notes() {
            self.facts.diag.report(n.event, &n.session, &n.code, n.reason);
        }
    }

    /// Não espera a rodada em curso: entra na fila e vale antes da próxima leitura do cache.
    pub fn seed(&self, name: &str, jsonl: &str) {
        let (name, jsonl) = (name.to_owned(), jsonl.to_owned());
        self.caches.resolver.apply(move |r| r.seed(&name, &jsonl));
    }

    /// `_forget`: nome reusado por outra sessão não herda nada da morta. Não espera, como `seed`.
    pub fn forget(&self, name: &str) {
        let c = &self.caches;
        let n = name.to_owned();
        c.resolver.apply({ let n = n.clone(); move |r| r.forget(&n) });
        c.decor.apply({ let n = n.clone(); move |d| { d.context.forget(&n); d.replies.forget(&n); } });
        c.classify.apply(move |k| k.classifier.forget(&n));
    }

    /// Não espera, como `seed`.
    pub fn rename(&self, old: &str, new: &str) {
        let c = &self.caches;
        let (o, n) = (old.to_owned(), new.to_owned());
        c.resolver.apply({ let (o, n) = (o.clone(), n.clone()); move |r| r.rename(&o, &n) });
        c.decor.apply({ let (o, n) = (o.clone(), n.clone()); move |d| { d.context.forget(&o); d.replies.rename(&o, &n); } });
        c.classify.apply(move |k| k.classifier.rename(&o, &n));
    }
}

impl Caches {
    /// Pastas de conta (`list_config_dirs` + a base do backend): `CP_CLAUDE_CONFIG_DIRS` ou
    /// `~/.claude*`. Sobrar pasta só custa um `read_dir` vazio; faltar some com o marcador.
    fn config_dirs(&self, dirs: &Dirs) -> Arc<Vec<PathBuf>> {
        if let Some((at, v)) = &*lock(&self.config_dirs) && at.elapsed() < CONFIG_DIRS_TTL {
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
        *lock(&self.config_dirs) = Some((Instant::now(), v.clone()));
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

/// (código, chave, motivo) de um arquivo ou processo que a descoberta não leu: vai ao diário.
type DiscoveryProblem = (&'static str, String, &'static str);

/// Linhas descobertas, o pid do agente de cada uma (contexto de abertura e alvo da captura) e o que
/// a descoberta não conseguiu ler.
fn run_discovery(panes: &[Pane], procs: &dyn ProcessView, children: &ChildrenMap, resolver: &mut Resolver, dirs: &Dirs)
    -> (Vec<SessionRow>, HashMap<String, u32>, Vec<DiscoveryProblem>) {
    let found = discover_other::discover_rows(panes, procs, children, resolver, dirs);
    // Junção com a lista-tdesc: `found.problems.into_iter().map(|p| (p.code, p.key, p.reason)).collect()`.
    (found.rows, found.agent_pids, Vec::new())
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
            let meta = headless_meta(&dirs.home.join(".hangar/claude-headless").join(format!("{}.json", row.name)), &row.name);
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

/// Sidecar da sessão sem terminal: ilegível ou torto avisa, senão o modelo e a janela caem calados
/// nos da conta.
fn headless_meta(path: &Path, session: &str) -> Value {
    let code = match std::fs::read(path) {
        Ok(raw) => match serde_json::from_slice::<Value>(&raw) {
            Ok(v) => return v,
            Err(e) => format!("list_headless_meta_invalid:{:?}", e.classify()),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Value::Null,
        Err(e) => format!("list_headless_meta_unreadable:{:?}", e.kind()),
    };
    list_facts::note("rust.list_file_rejected", session, code, "sidecar da sessão sem terminal ilegível");
    Value::Null
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
        Operation::Snapshot {} => rows(&bridge.snapshot().await?.rows),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guarded_change_never_waits_for_the_holder_and_lands_before_the_next_read() {
        let g: Arc<Guarded<Vec<&'static str>>> = Arc::default();
        let (held, release) = (std::sync::mpsc::channel(), std::sync::mpsc::channel::<()>());
        let holder = {
            let g = g.clone();
            std::thread::spawn(move || g.with(|v| {
                v.push("rodada");
                held.0.send(()).unwrap();
                release.1.recv().unwrap();
            }))
        };
        held.1.recv().unwrap();
        let start = Instant::now();
        g.apply(|v| v.push("seed"));
        assert!(start.elapsed() < Duration::from_millis(100), "esperou a rodada");
        release.0.send(()).unwrap();
        holder.join().unwrap();
        assert_eq!(g.with(|v| v.clone()), ["rodada", "seed"]);
        g.apply(|v| v.push("livre"));
        assert_eq!(*lock(&g.value), ["rodada", "seed", "livre"], "valor livre: aplicado na hora");
    }

    #[test]
    fn unreadable_headless_sidecar_reaches_the_diary() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("hl-bridge-test.json");
        assert_eq!(headless_meta(&path, "hl-bridge-test"), Value::Null, "ausente é normal");
        assert!(list_facts::notes_for("hl-bridge-test").is_empty());
        std::fs::write(&path, "{").unwrap();
        assert_eq!(headless_meta(&path, "hl-bridge-test"), Value::Null);
        assert_eq!(list_facts::notes_for("hl-bridge-test"), ["list_headless_meta_invalid:Eof"]);
    }
}
