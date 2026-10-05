//! Coleta incremental com escopos do Python e uma varredura compartilhada por vez.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, atomic::{AtomicU64, Ordering}};
use std::time::{Duration, Instant};
use std::panic::{AssertUnwindSafe, catch_unwind};
use serde::{Deserialize, Serialize};
use indexmap::IndexMap;
use super::areas::AreaMap;
use super::index::{Index, IndexError, Progress};
use super::pricing::{Pricing, IGNORADOS, canonizar_provedor};
use super::rows::{UsageRow, UsoLinha};
use super::{claude, codex, simple};

const FRESHNESS: Duration = Duration::from_secs(30);
const FRESH_WAIT: Duration = Duration::from_secs(3);
// Falha que persiste (disco, escopos) não vira uma varredura por pedido.
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Scopes {
    pub claude: Vec<ClaudeScope>, pub codex: Vec<CodexScope>, pub pi: Vec<PiScope>,
    pub kimi: Option<KimiScope>, pub repo: PathBuf,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ClaudeScope { pub root: PathBuf, pub account: String, pub label: String }
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct CodexScope { pub home: PathBuf, pub account: String, pub label: String }
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct PiScope { pub root: PathBuf, pub source: String }
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct KimiScope { pub root: PathBuf, pub index: PathBuf }

#[derive(Debug)]
pub enum CollectError { NoScopes, Index(IndexError) }
impl std::fmt::Display for CollectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoScopes => f.write_str("escopos de custos indisponíveis"),
            Self::Index(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for CollectError {}
impl From<IndexError> for CollectError { fn from(error: IndexError) -> Self { Self::Index(error) } }

pub trait ScopeSource: Send + Sync { fn fetch(&self) -> Result<Scopes, CollectError>; }

pub struct HttpScopes { upstream: std::net::SocketAddr, secret: String }
impl HttpScopes {
    pub fn new(upstream: std::net::SocketAddr, secret: String) -> Self { Self { upstream, secret } }
}
impl ScopeSource for HttpScopes {
    fn fetch(&self) -> Result<Scopes, CollectError> {
        use http_body_util::BodyExt;
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()
            .map_err(|_| CollectError::NoScopes)?;
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(10), async {
                let request = axum::http::Request::builder()
                    .uri(format!("http://{}/internal/costs/scopes", self.upstream))
                    .header("X-Hangar-Internal", &self.secret).body(axum::body::Body::empty())
                    .map_err(|_| CollectError::NoScopes)?;
                let response = crate::proxy::client().request(request).await.map_err(|_| CollectError::NoScopes)?;
                if !response.status().is_success() { return Err(CollectError::NoScopes); }
                let body = response.into_body().collect().await.map_err(|_| CollectError::NoScopes)?.to_bytes();
                serde_json::from_slice(&body).map_err(|_| CollectError::NoScopes)
            }).await.map_err(|_| CollectError::NoScopes)?
        })
    }
}

#[derive(Debug)]
pub enum Ready { Go, Warming { read: usize, total: usize } }

#[derive(Clone, Copy)]
enum Failure { Scopes, Disk, Reader, Database }
impl Failure {
    fn from_error(error: &CollectError) -> Self {
        match error {
            CollectError::NoScopes => Self::Scopes,
            CollectError::Index(IndexError::NoDisk) => Self::Disk,
            CollectError::Index(IndexError::ReaderPanic) => Self::Reader,
            CollectError::Index(IndexError::Sqlite(_)) => Self::Database,
        }
    }
    fn error(self) -> CollectError {
        match self {
            Self::Scopes => CollectError::NoScopes,
            Self::Disk => IndexError::NoDisk.into(),
            Self::Reader => IndexError::ReaderPanic.into(),
            Self::Database => IndexError::Sqlite(rusqlite::Error::InvalidQuery).into(),
        }
    }
}
struct ScanTicket { result: Mutex<Option<Result<(), Failure>>>, done: Condvar }
#[derive(Default)]
struct Scanner {
    attempted: bool,
    running: Option<Arc<ScanTicket>>,
    completed: Option<Arc<ScanTicket>>,
    last_success: Option<Instant>,
    failure: Option<Failure>,
    failed_at: Option<Instant>,
}
#[derive(Default)]
struct Metadata {
    last_good: Option<Scopes>,
    active: Option<Scopes>,
    labels: Vec<(String, String)>,
    kimi_projects: std::collections::HashMap<String, String>,
    generation: u64,
}

pub struct Collector {
    index: OnceLock<Index>, index_dir: PathBuf, index_init: Mutex<()>, pool: OnceLock<Arc<rayon::ThreadPool>>,
    pricing: Mutex<Pricing>, areas: AreaMap,
    source: Arc<dyn ScopeSource>, progress: Progress, metadata: Mutex<Metadata>,
    scanner: Mutex<Scanner>, completed_scans: AtomicU64,
}
impl Collector {
    pub fn new(index_dir: PathBuf, pricing_dir: PathBuf, area_file: PathBuf, scopes: Arc<dyn ScopeSource>) -> Self {
        Self { index: OnceLock::new(), index_dir, index_init: Mutex::new(()), pool: OnceLock::new(),
            pricing: Mutex::new(Pricing::load(&pricing_dir)), areas: AreaMap::load(&area_file),
            source: scopes, progress: Progress::default(), metadata: Mutex::new(Metadata::default()),
            scanner: Mutex::new(Scanner::default()), completed_scans: AtomicU64::new(0) }
    }

    pub fn schedule_warmup(self: &Arc<Self>, delay: Duration) {
        let weak = Arc::downgrade(self);
        let _ = std::thread::Builder::new().name("custos-boot".into()).spawn(move || {
            std::thread::sleep(delay);
            if let Some(collector) = weak.upgrade() {
                let mut scanner = collector.scanner.lock().unwrap();
                if !scanner.attempted { collector.start_scan(&mut scanner); }
            }
        });
    }

    /// Workers síncronos podem esperar mesmo quando carregam o Handle do Tokio.
    pub fn prepare_blocking(self: &Arc<Self>, fresh: bool) -> Result<Ready, CollectError> {
        self.prepare_inner(fresh, true)
    }

    pub fn prepare(self: &Arc<Self>, fresh: bool) -> Result<Ready, CollectError> {
        self.prepare_inner(fresh, tokio::runtime::Handle::try_current().is_err())
    }

    fn prepare_inner(self: &Arc<Self>, fresh: bool, can_wait: bool) -> Result<Ready, CollectError> {
        let observed = self.completed_scans.load(Ordering::Acquire);
        self.pricing.lock().unwrap().reload_if_changed();
        let mut scanner = self.scanner.lock().unwrap();
        if !scanner.attempted {
            self.start_scan(&mut scanner);
            return Ok(self.warming());
        }
        if scanner.completed.is_none() { return Ok(self.warming()); }
        if !fresh {
            if let Some(failure) = scanner.failure {
                // Sem isto a falha ficava presa até um `fresco`: um pedido seguinte já pega a nova coleta.
                if scanner.running.is_none() && scanner.failed_at.is_none_or(|at| at.elapsed() >= RETRY_AFTER_FAILURE) {
                    self.start_scan(&mut scanner);
                }
                return Err(failure.error());
            }
            if scanner.last_success.is_some_and(|at| at.elapsed() >= FRESHNESS) && scanner.running.is_none() {
                self.start_scan(&mut scanner);
            }
            return Ok(Ready::Go);
        }
        // Uma conclusão durante a entrada já atende este pedido, sem repetir a coleta.
        let ticket = if self.completed_scans.load(Ordering::Acquire) != observed {
            scanner.completed.as_ref().unwrap().clone()
        } else if let Some(ticket) = &scanner.running {
            ticket.clone()
        } else { self.start_scan(&mut scanner) };
        drop(scanner);
        // A entrada explícita evita confundir o Handle de spawn_blocking com o executor.
        if !can_wait {
            return match *ticket.result.lock().unwrap() {
                Some(Ok(())) => Ok(Ready::Go), Some(Err(failure)) => Err(failure.error()), None => Ok(self.warming()),
            };
        }
        let result = ticket.result.lock().unwrap();
        let (result, _) = ticket.done.wait_timeout_while(result, FRESH_WAIT, |result| result.is_none()).unwrap();
        match *result {
            Some(Ok(())) => Ok(Ready::Go), Some(Err(failure)) => Err(failure.error()), None => Ok(self.warming()),
        }
    }

    fn warming(&self) -> Ready {
        let (read, total) = self.progress.total(); Ready::Warming { read, total }
    }

    fn start_scan(self: &Arc<Self>, scanner: &mut Scanner) -> Arc<ScanTicket> {
        let ticket = Arc::new(ScanTicket { result: Mutex::new(None), done: Condvar::new() });
        scanner.attempted = true;
        scanner.running = Some(ticket.clone());
        self.progress.reset();
        let collector = self.clone(); let running = ticket.clone();
        let spawned = std::thread::Builder::new().name("custos-scan".into()).spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| collector.scan()))
                .unwrap_or_else(|_| Err(IndexError::ReaderPanic.into())).map_err(|e| Failure::from_error(&e));
            let mut scanner = collector.scanner.lock().unwrap();
            scanner.running = None;
            scanner.completed = Some(running.clone());
            scanner.failure = result.err();
            scanner.failed_at = scanner.failure.map(|_| Instant::now());
            if result.is_ok() { scanner.last_success = Some(Instant::now()); }
            *running.result.lock().unwrap() = Some(result);
            collector.completed_scans.fetch_add(1, Ordering::Release);
            running.done.notify_all();
        });
        if spawned.is_err() {
            scanner.running = None; scanner.completed = Some(ticket.clone()); scanner.failure = Some(Failure::Disk);
            scanner.failed_at = Some(Instant::now());
            *ticket.result.lock().unwrap() = Some(Err(Failure::Disk));
            self.completed_scans.fetch_add(1, Ordering::Release);
            ticket.done.notify_all();
        }
        ticket
    }

    fn scan(&self) -> Result<(), CollectError> {
        let scopes = match self.source.fetch() {
            Ok(scopes) => {
                let mut labels = IndexMap::new();
                for scope in &scopes.claude { labels.insert(scope.account.clone(), scope.label.clone()); }
                for scope in &scopes.codex { labels.insert(scope.account.clone(), scope.label.clone()); }
                let labels: Vec<_> = labels.into_iter().collect();
                let mut metadata = self.metadata.lock().unwrap();
                if metadata.labels != labels { metadata.generation += 1; }
                metadata.labels = labels; metadata.last_good = Some(scopes.clone());
                scopes
            },
            Err(_) => {
                tracing::warn!(code = "escopos_custos");
                self.metadata.lock().unwrap().last_good.clone().ok_or(CollectError::NoScopes)?
            },
        };
        let index = self.index()?;
        let owners = rollout_owners(&scopes.codex);
        let mut active = scopes.clone();
        active.codex = owners.values().map(|(scope, _)| scope.clone()).collect();
        let pi_root = scopes.pi.iter().find(|s| s.source == "pi").map(|s| &s.root);
        active.pi.retain(|scope| scope.root.is_dir() && !(scope.source == "omp" && pi_root == Some(&scope.root)));
        active.kimi = scopes.kimi.filter(|scope| scope.root.is_dir());
        let mut claude_files = Vec::new(); let mut pi_files = Vec::new();
        for scope in &active.claude {
            let files = list_files(&scope.root, |n| n.ends_with(".jsonl"));
            self.progress.set(&claude_key(scope), 0, files.len()); claude_files.push(files);
        }
        for (scope, files) in owners.values() { self.progress.set(&scope.account, 0, files.len()); }
        for scope in &active.pi {
            let files = list_files(&scope.root, |n| n.ends_with(".jsonl"));
            self.progress.set(&pi_key(scope), 0, files.len()); pi_files.push(files);
        }
        let kimi_files = active.kimi.as_ref().map(|scope| {
            let files = list_files(&scope.root, |n| n == "wire.jsonl");
            self.progress.set(&kimi_key(scope), 0, files.len()); files
        });
        let redo = |entries: &_| self.areas.area_lines(entries);
        let signature = self.areas.signature(); let mut keys = Vec::new();
        for (scope, files) in active.claude.iter().zip(claude_files) {
            let key = claude_key(scope);
            index.sync(&key, &files, &claude::new_fold(&scope.root), claude::VERSION, signature, &redo, &self.progress)?;
            keys.push(key);
        }
        for (scope, files) in owners.values() {
            index.sync(&scope.account, files, &codex::new_fold, codex::VERSION, signature, &redo, &self.progress)?;
            keys.push(scope.account.clone());
        }
        for (scope, files) in active.pi.iter().zip(pi_files) {
            let key = pi_key(scope);
            index.sync(&key, &files, &simple::new_pi_fold(&scope.root, &scope.source), simple::PI_VERSION, signature, &redo, &self.progress)?;
            keys.push(key);
        }
        if let (Some(scope), Some(files)) = (&active.kimi, kimi_files) {
            let key = kimi_key(scope);
            index.sync(&key, &files, &simple::new_kimi_fold, simple::KIMI_VERSION, signature, &redo, &self.progress)?;
            keys.push(key);
        }
        index.forget_outside(&keys)?;
        let mut metadata = self.metadata.lock().unwrap();
        if metadata.active.as_ref() != Some(&active) { metadata.generation += 1; }
        metadata.active = Some(active);
        Ok(())
    }

    fn active_scopes(&self) -> Result<Scopes, CollectError> {
        let scanner = self.scanner.lock().unwrap();
        if let Some(failure) = scanner.failure { return Err(failure.error()); }
        drop(scanner);
        self.metadata.lock().unwrap().active.clone().ok_or(CollectError::NoScopes)
    }

    fn claude_rows(&self, scope: &ClaudeScope, since: Option<&str>, stamp: bool, pricing: &Pricing) -> Result<Vec<UsageRow>, CollectError> {
        let mut rows = self.index()?.read_costs(Some(&claude_key(scope)), since, None)?;
        rows.retain(|row| !IGNORADOS.contains(&crate::transcript::py::strip(&row.model)));
        for row in &mut rows {
            row.model = crate::transcript::py::strip(&row.model).to_owned();
            if row.project.is_empty() { row.project = "desconhecido".into(); }
            let provider = canonizar_provedor(&pricing.provider_for(&row.model).unwrap_or_default());
            row.provider = if provider.is_empty() || provider == "anthropic" { scope.account.clone() } else { provider };
            row.account_id = stamp.then(|| scope.account.clone());
        }
        Ok(rows)
    }

    pub fn read_costs(&self, since: Option<&str>) -> Result<Vec<UsageRow>, CollectError> {
        let active = self.active_scopes()?; let index = self.index()?; let mut output = Vec::new();
        {
            let pricing = self.pricing.lock().unwrap();
            for scope in &active.claude { output.extend(self.claude_rows(scope, since, false, &pricing)?); }
        }
        for scope in &active.codex {
            let mut rows = index.read_costs(Some(&scope.account), since, None)?;
            for row in &mut rows {
                row.account_id = Some(scope.account.clone());
                if row.provider == "openai" { row.provider = scope.account.clone(); }
            }
            output.extend(rows);
        }
        for scope in &active.pi { output.extend(index.read_costs(Some(&pi_key(scope)), since, None)?); }
        if let Some(scope) = &active.kimi {
            let projects = simple::kimi_projects(&scope.index);
            let mut rows = index.read_costs(Some(&kimi_key(scope)), since, None)?;
            for row in &mut rows {
                row.project = projects.get(&row.session_id).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| "desconhecido".into());
            }
            output.extend(rows);
        }
        Ok(output)
    }

    pub fn read_usage(&self, since: Option<&str>) -> Result<(Vec<(UsoLinha, String)>, Vec<UsageRow>), CollectError> {
        let active = self.active_scopes()?; let index = self.index()?;
        let mut usage = Vec::new(); let mut tokens = Vec::new();
        {
            let pricing = self.pricing.lock().unwrap();
            for scope in &active.claude {
                tokens.extend(self.claude_rows(scope, since, true, &pricing)?);
                index.append_usage(&claude_key(scope), since, &mut usage, |row| (row, scope.account.clone()))?;
            }
        }
        for scope in &active.codex {
            index.append_usage(&scope.account, since, &mut usage, |row| (row, scope.account.clone()))?;
        }
        Ok((usage, tokens))
    }

    /// `read_usage` sem materializar o uso: `start` recebe os tokens antes da primeira linha, e
    /// as linhas chegam na mesma ordem, com a conta. A tarifa fica travada até o fim.
    pub fn fold_usage<B: Clone>(&self, since: Option<&str>, start: impl FnOnce(&[UsageRow], &Pricing) -> B,
                                visit: &mut dyn FnMut(&mut B, &UsoLinha, &str, &Pricing)) -> Result<B, CollectError> {
        let active = self.active_scopes()?; let index = self.index()?;
        let pricing = self.pricing.lock().unwrap();
        let mut tokens = Vec::new();
        for scope in &active.claude { tokens.extend(self.claude_rows(scope, since, true, &pricing)?); }
        let mut state = start(&tokens, &pricing);
        drop(tokens);
        let scopes = active.claude.iter().map(|s| (claude_key(s), &s.account))
            .chain(active.codex.iter().map(|s| (s.account.clone(), &s.account)));
        for (key, account) in scopes {
            state = index.fold_usage(&key, since, state, &mut |b, row| visit(b, &row, account, &pricing))?;
        }
        Ok(state)
    }

    pub fn index(&self) -> Result<&Index, CollectError> {
        if let Some(index) = self.index.get() { return Ok(index); }
        // Construir o estado das rotas não deve tocar o índice de produção nos testes.
        let _init = self.index_init.lock().unwrap();
        if self.index.get().is_none() {
            if self.pool.get().is_none() {
                let threads = std::thread::available_parallelism().map_or(1, usize::from).min(4);
                let pool = rayon::ThreadPoolBuilder::new().num_threads(threads)
                    .thread_name(|i| format!("custos-{i}")).build().map_err(|_| IndexError::NoDisk)?;
                let _ = self.pool.set(Arc::new(pool));
            }
            let index = Index::open(&self.index_dir)?.with_pool(self.pool.get().unwrap().clone());
            let _ = self.index.set(index);
        }
        Ok(self.index.get().unwrap())
    }
    pub fn label(&self, key: &str) -> Option<String> {
        self.metadata.lock().unwrap().labels.iter().find(|(name, _)| name == key).map(|(_, label)| label.clone())
    }
    pub fn labels_key(&self) -> Vec<(String, String)> {
        self.metadata.lock().unwrap().labels.clone()
    }
    pub fn data_version(&self) -> u64 {
        // Nunca segura metadados junto de Pricing: os relatórios podem já ter a tarifa travada.
        let mut metadata = self.metadata.lock().unwrap();
        let projects = metadata.active.as_ref().and_then(|s| s.kimi.as_ref()).map(|s| simple::kimi_projects(&s.index)).unwrap_or_default();
        if projects != metadata.kimi_projects { metadata.kimi_projects = projects; metadata.generation += 1; }
        metadata.generation + self.index.get().map_or(0, Index::generation)
    }
    pub fn pricing(&self) -> MutexGuard<'_, Pricing> { self.pricing.lock().unwrap() }
    pub fn areas(&self) -> &AreaMap { &self.areas }
    pub fn repo(&self) -> Option<PathBuf> { self.metadata.lock().unwrap().active.as_ref().map(|s| s.repo.clone()) }
}

fn claude_key(scope: &ClaudeScope) -> String { format!("claude:{}", scope.root.display()) }
fn pi_key(scope: &PiScope) -> String { format!("{}:{}", scope.source, scope.root.display()) }
fn kimi_key(scope: &KimiScope) -> String { format!("kimi:{}", scope.root.display()) }

pub fn rollout_owners(codex: &[CodexScope]) -> IndexMap<String, (CodexScope, Vec<PathBuf>)> {
    let homes: Vec<_> = codex.iter().map(|scope| std::fs::canonicalize(&scope.home).ok()).collect();
    let mut owners: IndexMap<String, (CodexScope, Vec<PathBuf>)> = IndexMap::new();
    for scope in codex {
        for root in [scope.home.join("sessions"), scope.home.join("archived_sessions")] {
            for path in list_files(&root, |n| n.starts_with("rollout-") && n.ends_with(".jsonl")) {
                let Ok(path) = std::fs::canonicalize(path) else { continue };
                let mut matches = codex.iter().zip(&homes).filter(|(_, home)| home.as_ref().is_some_and(|home| {
                    path.starts_with(home.join("sessions")) || path.starts_with(home.join("archived_sessions"))
                }));
                let Some((owner, _)) = matches.next() else { continue };
                if matches.next().is_some() { continue; }
                let (_, files) = owners.entry(owner.account.clone()).or_insert_with(|| (owner.clone(), Vec::new()));
                if !files.contains(&path) { files.push(path); }
            }
        }
    }
    for (_, files) in owners.values_mut() { files.sort(); }
    owners
}

pub fn list_files(root: &Path, matches: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    let mut output = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(path) else { continue };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() { stack.push(entry.path()); }
            else if matches(&entry.file_name().to_string_lossy()) { output.push(entry.path()); }
        }
    }
    // O caminho textual acompanha a ordem usada na leitura do índice.
    output.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
    output
}
