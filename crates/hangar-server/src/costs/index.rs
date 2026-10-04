//! Índice descartável em disco, com leitura retomável e um escritor por varredura.

use super::py::LocalTs;
use super::rows::{AreaEntries, FoldOutput, UsageRow, UsoLinha};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use rusqlite::{Connection, ErrorCode, OptionalExtension, TransactionBehavior, params};
use serde::{Serialize, de::DeserializeOwned};
use std::collections::BTreeMap;
use std::fs::{self, File, Metadata};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, atomic::{AtomicU64, AtomicUsize, Ordering}, mpsc};
use std::time::{Duration, Instant};

pub const SCHEMA: u32 = 1;
pub const FILE_NAME: &str = "custos-rust.sqlite3";
const TAIL_BYTES: u64 = 64;
const BATCH_TIME: Duration = Duration::from_secs(1);
const READ_WINDOW: usize = 64;
const BATCH_ROWS: usize = 2048;
const COST_FIELDS: &str = "ts, source, provider, model, project, session_id, input, output, cache_write, cache_read, subagente, account_id, codex_long_context, cache_write_1h, fast, regravado, regravado_1h";
const USAGE_FIELDS: &str = "dia, cwd, model, tipo, nome, plugin, detalhe, origem, chamadas, ctx_chars, tokens_est, input, output, cache_write, cache_read, cache_write_1h, fast, ocupados, respostas, ocupados_eq, fonte, subagente, session_id";
const TABLES: &str = "
CREATE TABLE meta(k TEXT PRIMARY KEY, v TEXT);
CREATE TABLE files(id INTEGER PRIMARY KEY, path TEXT UNIQUE NOT NULL, scope TEXT NOT NULL,
    versao TEXT NOT NULL, dev INTEGER, ino INTEGER, size INTEGER, mtime_ns INTEGER,
    offset INTEGER, cauda BLOB, estado BLOB, areas BLOB, areas_sig TEXT);
CREATE INDEX files_scope ON files(scope);
CREATE TABLE custo(file_id INTEGER NOT NULL, dia TEXT, ts, source, provider, model, project, session_id,
    input, output, cache_write, cache_read, subagente, account_id, codex_long_context, cache_write_1h, fast,
    regravado, regravado_1h);
CREATE INDEX custo_file ON custo(file_id);
CREATE TABLE uso(file_id INTEGER NOT NULL, dia, cwd, model, tipo, nome, plugin, detalhe, origem, chamadas,
    ctx_chars, tokens_est, input, output, cache_write, cache_read, cache_write_1h, fast, ocupados, respostas,
    ocupados_eq, fonte, subagente, session_id);
CREATE INDEX uso_file ON uso(file_id);";

pub trait Fold: Serialize + DeserializeOwned + Send {
    fn line(&mut self, raw: &[u8]);
    /// Pode consumir o estado: ele é serializado antes do fragmento e desta chamada.
    fn close(&mut self) -> FoldOutput;
}

#[derive(Debug)]
pub enum IndexError {
    NoDisk,
    ReaderPanic,
    Sqlite(rusqlite::Error),
}

impl std::fmt::Display for IndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDisk => f.write_str("índice de custos sem disco disponível"),
            Self::ReaderPanic => f.write_str("pânico no leitor do índice de custos"),
            Self::Sqlite(_) => f.write_str("falha SQLite no índice de custos"),
        }
    }
}

impl std::error::Error for IndexError {}

impl From<rusqlite::Error> for IndexError {
    fn from(error: rusqlite::Error) -> Self {
        match error.sqlite_error_code() {
            Some(ErrorCode::DiskFull | ErrorCode::ReadOnly | ErrorCode::CannotOpen | ErrorCode::SystemIoFailure) => Self::NoDisk,
            _ => Self::Sqlite(error),
        }
    }
}

#[derive(Default)]
pub struct Progress {
    scopes: Mutex<BTreeMap<String, ScopeProgress>>,
}

#[derive(Default)]
struct ScopeProgress {
    read: AtomicUsize,
    total: AtomicUsize,
}

impl Progress {
    pub fn reset(&self) { self.scopes.lock().unwrap().clear(); }

    pub fn total(&self) -> (usize, usize) {
        self.scopes.lock().unwrap().values().fold((0, 0), |(read, total), scope| {
            (read + scope.read.load(Ordering::Relaxed), total + scope.total.load(Ordering::Relaxed))
        })
    }

    pub(crate) fn set(&self, scope: &str, read: usize, total: usize) {
        let mut scopes = self.scopes.lock().unwrap();
        let value = scopes.entry(scope.to_owned()).or_default();
        value.read.store(read, Ordering::Relaxed);
        value.total.store(total, Ordering::Relaxed);
    }
}

#[derive(Clone)]
pub struct Index {
    path: PathBuf,
    pool: Option<Arc<rayon::ThreadPool>>,
    generation: Arc<AtomicU64>,
    operations: Arc<Mutex<OperationState>>,
}

#[derive(Default)]
struct OperationState {
    active: usize,
    epoch: u64,
    pending: bool,
}

struct OperationLease<'a> {
    state: &'a Mutex<OperationState>,
    epoch: u64,
}

impl Drop for OperationLease<'_> {
    fn drop(&mut self) { self.state.lock().unwrap().active -= 1; }
}

fn corruption_error() -> IndexError {
    rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT), None).into()
}

fn is_corrupt(error: &IndexError) -> bool {
    matches!(error, IndexError::Sqlite(error) if matches!(error.sqlite_error_code(), Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase)))
}

pub fn default_dir() -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from).unwrap_or_default();
    let cache = std::env::var_os(if cfg!(windows) { "LOCALAPPDATA" } else { "XDG_CACHE_HOME" });
    default_dir_from(&home, cache.as_deref())
}

#[doc(hidden)]
pub fn default_dir_from(home: &Path, cache: Option<&std::ffi::OsStr>) -> PathBuf {
    if cfg!(windows) {
        cache.filter(|v| !v.is_empty()).map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Local")).join("hangar").join("custos")
    } else {
        cache.map(PathBuf::from).filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join(".cache")).join("hangar").join("custos")
    }
}

#[doc(hidden)]
pub fn dump_for_tests(ix: &Index, base: &Path, prefix: &str) -> serde_json::Value {
    use rusqlite::types::ValueRef;
    use serde_json::{Map, Value};
    let conn = ix.connect().unwrap();
    let mut stmt = conn.prepare("SELECT id, path FROM files ORDER BY path").unwrap();
    let files = stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .unwrap().collect::<Result<Vec<_>, _>>().unwrap();
    let mut output = Map::new();
    for (id, path) in files {
        let relative = Path::new(&path).strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
        if !relative.starts_with(prefix) { continue; }
        let mut parts = Map::new();
        for (name, table, fields, filter) in [
            ("custo", "custo", COST_FIELDS, ""),
            ("uso", "uso", USAGE_FIELDS, " AND tipo<>'area'"),
            ("areas", "uso", USAGE_FIELDS, " AND tipo='area'"),
        ] {
            let mut stmt = conn.prepare(&format!("SELECT {fields} FROM {table} WHERE file_id=?{filter} ORDER BY rowid")).unwrap();
            let count = stmt.column_count();
            let rows = stmt.query_map([id], |row| {
                let mut values = Vec::new();
                for i in 0..count {
                    values.push(match row.get_ref(i)? {
                        ValueRef::Null => Value::Null,
                        ValueRef::Integer(n) => Value::from(n),
                        ValueRef::Real(n) => Value::from(n),
                        ValueRef::Text(s) => Value::String(String::from_utf8_lossy(s).into_owned()),
                        ValueRef::Blob(_) => panic!("coluna de contrato inesperada"),
                    });
                }
                Ok(Value::Array(values))
            }).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
            parts.insert(name.into(), Value::Array(rows));
        }
        output.insert(relative, Value::Object(parts));
    }
    Value::Object(output)
}

impl Index {
    pub fn open(dir: &Path) -> Result<Self, IndexError> {
        let index = Self { path: dir.join(FILE_NAME), pool: None, generation: Arc::new(AtomicU64::new(0)), operations: Arc::new(Mutex::new(OperationState::default())) };
        index.with_recovery(|| index.connect().map(drop))?;
        Ok(index)
    }

    /// Seleciona o pool compartilhado sem deslocar o escritor para uma das suas threads.
    pub fn with_pool(mut self, pool: Arc<rayon::ThreadPool>) -> Self {
        self.pool = Some(pool);
        self
    }

    pub fn generation(&self) -> u64 { self.generation.load(Ordering::Acquire) }

    fn changed(&self) { self.generation.fetch_add(1, Ordering::Release); }

    fn connect(&self) -> Result<Connection, IndexError> {
        fs::create_dir_all(self.path.parent().unwrap()).map_err(|_| IndexError::NoDisk)?;
        open_connection(&self.path, &self.generation).map_err(Into::into)
    }

    fn rebuild(&self, state: &mut OperationState) -> Result<(), IndexError> {
        // Só a última operação pode apagar: callbacks podem consultar o mesmo índice.
        debug_assert_eq!(state.active, 0);
        state.epoch += 1;
        self.changed();
        tracing::warn!(code = "indice_custos_ilegivel");
        for suffix in ["", "-wal", "-shm"] {
            let mut path = self.path.as_os_str().to_os_string();
            path.push(suffix);
            match fs::remove_file(PathBuf::from(path)) {
                Ok(()) => {},
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                Err(_) => return Err(IndexError::NoDisk),
            }
        }
        drop(open_connection(&self.path, &self.generation)?);
        state.pending = false;
        Ok(())
    }

    fn with_recovery<T>(&self, mut run: impl FnMut() -> Result<T, IndexError>) -> Result<T, IndexError> {
        for attempt in 0..2 {
            let lease = {
                let mut state = self.operations.lock().unwrap();
                if state.pending {
                    if state.active != 0 { return Err(corruption_error()); }
                    self.rebuild(&mut state)?;
                }
                state.active += 1;
                OperationLease { state: &self.operations, epoch: state.epoch }
            };
            let epoch = lease.epoch;
            let result = run();
            // O retorno da tentativa fecha conexões, statements e transações antes da remoção.
            drop(lease);
            if let Some(value) = self.finish_attempt(epoch, attempt, result)? { return Ok(value); }
        }
        unreachable!()
    }

    fn finish_attempt<T>(&self, epoch: u64, attempt: usize, result: Result<T, IndexError>) -> Result<Option<T>, IndexError> {
        let mut state = self.operations.lock().unwrap();
        let corrupt = result.as_ref().err().is_some_and(is_corrupt);
        if corrupt && epoch == state.epoch && !state.pending {
            state.pending = true;
            self.changed();
        }
        let stale = epoch != state.epoch;
        if state.pending || stale {
            if matches!(result, Err(IndexError::ReaderPanic)) {
                if state.pending && state.active == 0 && self.rebuild(&mut state).is_err() {
                    tracing::warn!(code = "reconstrucao_indice_custos");
                }
                return result.map(Some);
            }
            if state.active != 0 || attempt == 1 {
                return result.and_then(|_| Err(corruption_error()));
            }
            if state.pending { self.rebuild(&mut state)?; }
            return Ok(None);
        }
        result.map(Some)
    }

    pub fn sync<F: Fold>(
        &self, scope: &str, files: &[PathBuf], new_fold: &(dyn Fn(&Path) -> F + Sync),
        version: &str, areas_sig: &str, redo_areas: &(dyn Fn(&AreaEntries) -> Vec<UsoLinha> + Sync),
        progress: &Progress,
    ) -> Result<bool, IndexError> {
        progress.set(scope, 0, files.len());
        let result = self.with_recovery(|| {
            progress.set(scope, 0, files.len());
            self.sync_inner(scope, files, new_fold, version, areas_sig, redo_areas, progress)
        });
        // Fontes concluídas continuam na soma exibida durante o aquecimento.
        progress.set(scope, files.len(), files.len());
        result
    }

    fn sync_inner<F: Fold>(
        &self, scope: &str, files: &[PathBuf], new_fold: &(dyn Fn(&Path) -> F + Sync),
        version: &str, areas_sig: &str, redo_areas: &(dyn Fn(&AreaEntries) -> Vec<UsoLinha> + Sync),
        progress: &Progress,
    ) -> Result<bool, IndexError> {
        let mut conn = self.connect()?;
        let version = format!("{SCHEMA}:{version}");
        let mut known = BTreeMap::new();
        {
            let mut stmt = conn.prepare("SELECT path, id, versao, dev, ino, size, mtime_ns FROM files WHERE scope=?")?;
            for row in stmt.query_map([scope], |row| Ok((row.get::<_, String>(0)?, FileRecord::light(row, 1)?)))? {
                let (path, record) = row?;
                known.insert(path, record);
            }
        }
        let before = conn.total_changes();
        let mut jobs = Vec::new();
        for (position, path) in files.iter().enumerate() {
            let key = path.to_string_lossy().into_owned();
            let light = known.remove(&key);
            let metadata = match fs::metadata(path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if let Some(record) = light { known.insert(key, record); }
                    continue;
                },
                Err(_) => continue,
            };
            let fingerprint = Fingerprint::from_metadata(&metadata);
            if light.as_ref().is_some_and(|r| r.current(&fingerprint, &version)) { continue; }
            let record = conn.query_row(
                "SELECT id, versao, dev, ino, size, mtime_ns FROM files WHERE path=?",
                [&key], |row| FileRecord::light(row, 0),
            ).optional()?;
            if record.as_ref().is_some_and(|r| r.current(&fingerprint, &version)) {
                if conn.execute("UPDATE files SET scope=? WHERE id=? AND scope<>?", params![scope, record.unwrap().id, scope])? > 0 {
                    self.changed();
                }
                continue;
            }
            jobs.push(ReadJob { position, path: path.clone(), fingerprint, record });
        }

        let (sender, receiver) = mpsc::channel();
        // O chamador escreve; só as leituras usam o pool corrente, inclusive um pool privado.
        in_pool_scope(self.pool.as_deref(), |rayon_scope| -> Result<(), IndexError> {
            let total = jobs.len();
            let mut jobs = jobs.into_iter();
            let mut sender = Some(sender);
            let mut submitted = 0;
            let mut ready = BTreeMap::new();
            let mut pending = Vec::new();
            let mut pending_rows = 0;
            let mut next = 0;
            let mut reader_panicked = false;
            let mut batch_started = Instant::now();
            while next < total {
                let mut window = Vec::new();
                while submitted < total && submitted - next < READ_WINDOW {
                    let mut job = jobs.next().unwrap();
                    job.record = load_record(&conn, &job.path.to_string_lossy())?;
                    window.push((submitted, job));
                    submitted += 1;
                }
                // A fila local do Rayon é LIFO; o primeiro arquivo deve poder terminar num único worker.
                for (order, mut job) in window.into_iter().rev() {
                    let sender = sender.as_ref().unwrap().clone();
                    let version_ref = &version;
                    rayon_scope.spawn(move |_| {
                        let read = read_new(&job.path, &job.fingerprint, job.record.as_ref(), new_fold, version_ref)
                            .map(|read| read.with_area_rows(redo_areas));
                        job.record = None;
                        let _ = sender.send((order, job, read));
                    });
                }
                if submitted == total { sender.take(); }
                // Um pool de um núcleo pode ter o escritor dentro dele; ceder evita bloqueá-lo.
                rayon::yield_now();
                match receiver.recv_timeout(Duration::from_millis(10)) {
                    Ok((order, job, result)) => { ready.insert(order, (job, result)); },
                    Err(mpsc::RecvTimeoutError::Disconnected) if !ready.contains_key(&next) => return Err(IndexError::ReaderPanic),
                    Err(_) => {},
                }
                while let Some((job, result)) = ready.remove(&next) {
                    progress.set(scope, job.position + 1, files.len());
                    match result {
                        Ok(read) => {
                            pending_rows += read.output.costs.len() + read.output.usage.len();
                            pending.push((job, read));
                        },
                        Err(ReadError::Fold) => {
                            reader_panicked = true;
                            tracing::warn!(code = "panico_leitura_custos");
                        },
                        Err(_) => tracing::warn!(code = "leitura_custos"),
                    }
                    next += 1;
                    // Resultados completos não devem acumular até o próximo segundo em arquivos densos.
                    if pending.len() >= READ_WINDOW || pending_rows >= BATCH_ROWS {
                        write_batch(&mut conn, &mut pending, scope, &version, areas_sig, &self.generation)?;
                        pending_rows = 0;
                        batch_started = Instant::now();
                    }
                }
                if batch_started.elapsed() >= BATCH_TIME {
                    write_batch(&mut conn, &mut pending, scope, &version, areas_sig, &self.generation)?;
                    pending_rows = 0;
                    batch_started = Instant::now();
                }
            }
            let batch_before = conn.total_changes();
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            for (job, read) in pending {
                write_file(&tx, &job, scope, &version, read, false, areas_sig)?;
            }
            for record in known.values() { delete_file(&tx, record.id)?; }
            reader_panicked |= redo_saved_areas(&tx, scope, areas_sig, redo_areas)?;
            tx.commit()?;
            if conn.total_changes() != batch_before { self.changed(); }
            if reader_panicked { Err(IndexError::ReaderPanic) } else { Ok(()) }
        })?;
        Ok(conn.total_changes() != before)
    }

    pub fn sync_file<F: Fold>(
        &self, path: &Path, new_fold: &dyn Fn(&Path) -> F, version: &str, scope: &str,
        areas_sig: &str, redo_areas: &dyn Fn(&AreaEntries) -> Vec<UsoLinha>,
    ) -> Option<i64> {
        match self.try_sync_file(path, new_fold, version, scope, areas_sig, redo_areas) {
            Ok(id) => Some(id),
            Err(_) => { tracing::warn!(code = "leitura_custos"); None },
        }
    }

    pub fn try_sync_file<F: Fold>(
        &self, path: &Path, new_fold: &dyn Fn(&Path) -> F, version: &str, scope: &str,
        areas_sig: &str, redo_areas: &dyn Fn(&AreaEntries) -> Vec<UsoLinha>,
    ) -> Result<i64, IndexError> {
        self.with_recovery(|| {
            let mut conn = self.connect()?;
            let fingerprint = Fingerprint::from_metadata(&fs::metadata(path).map_err(|_| IndexError::NoDisk)?);
            let version = format!("{SCHEMA}:{version}");
            let record = load_record(&conn, &path.to_string_lossy())?;
            if let Some(record) = record.as_ref().filter(|r| r.current(&fingerprint, &version)) {
                return Ok(record.id);
            }
            let read = read_new(path, &fingerprint, record.as_ref(), new_fold, &version)
                .map_err(|error| match error { ReadError::Fold => IndexError::ReaderPanic, _ => IndexError::NoDisk })?
                .with_area_rows(redo_areas);
            let job = ReadJob { position: 0, path: path.to_owned(), fingerprint, record };
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let id = write_file(&tx, &job, scope, &version, read, true, areas_sig)?;
            tx.commit()?;
            self.changed();
            Ok(id)
        })
    }

    pub fn forget_outside(&self, active: &[String]) -> Result<bool, IndexError> {
        self.with_recovery(|| self.forget_outside_inner(active))
    }

    fn forget_outside_inner(&self, active: &[String]) -> Result<bool, IndexError> {
        let mut conn = self.connect()?;
        let mut removed = Vec::new();
        {
            let mut stmt = conn.prepare("SELECT id, scope, path FROM files ORDER BY rowid")?;
            for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))? {
                let (id, scope, path) = row?;
                if !active.contains(&scope) && !Path::new(&path).exists() { removed.push(id); }
            }
        }
        if removed.is_empty() { return Ok(false); }
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for id in removed { delete_file(&tx, id)?; }
        tx.commit()?;
        self.changed();
        Ok(true)
    }

    pub fn read_costs(&self, scope: Option<&str>, since: Option<&str>, file_id: Option<i64>) -> Result<Vec<UsageRow>, IndexError> {
        self.with_recovery(|| self.read_costs_inner(scope, since, file_id))
    }

    fn read_costs_inner(&self, scope: Option<&str>, since: Option<&str>, file_id: Option<i64>) -> Result<Vec<UsageRow>, IndexError> {
        let conn = self.connect()?;
        let (filter, values) = filters(scope, since, file_id);
        let mut stmt = conn.prepare(&format!("SELECT {COST_FIELDS} FROM custo WHERE {filter} ORDER BY rowid"))?;
        let rows = stmt.query_map(rusqlite::params_from_iter(values), |r| {
            let text: String = r.get(0)?;
            let ts = LocalTs::from_iso(&text).ok_or(rusqlite::Error::InvalidQuery)?;
            Ok(UsageRow {
                ts, source: r.get(1)?, provider: r.get(2)?, model: r.get(3)?, project: r.get(4)?,
                session_id: r.get(5)?, input: r.get(6)?, output: r.get(7)?, cache_write: r.get(8)?,
                cache_read: r.get(9)?, subagente: r.get(10)?, account_id: r.get(11)?,
                codex_long_context: r.get(12)?, cache_write_1h: r.get(13)?, fast: r.get(14)?,
                regravado: r.get(15)?, regravado_1h: r.get(16)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn read_usage(&self, scope: &str, since: Option<&str>) -> Result<Vec<UsoLinha>, IndexError> {
        let mut rows = Vec::new();
        self.append_usage(scope, since, &mut rows, |row| row)?;
        Ok(rows)
    }

    pub(crate) fn append_usage<T>(
        &self, scope: &str, since: Option<&str>, output: &mut Vec<T>,
        mut decorate: impl FnMut(UsoLinha) -> T,
    ) -> Result<(), IndexError> {
        let checkpoint = output.len();
        let result = self.with_recovery(|| {
            // Uma tentativa condenada não pode misturar suas linhas com o banco reconstruído.
            output.truncate(checkpoint);
            self.append_usage_inner(scope, since, output, &mut decorate)
        });
        if result.is_err() { output.truncate(checkpoint); }
        result
    }

    fn append_usage_inner<T>(
        &self, scope: &str, since: Option<&str>, output: &mut Vec<T>,
        decorate: &mut impl FnMut(UsoLinha) -> T,
    ) -> Result<(), IndexError> {
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        let (filter, values) = filters(Some(scope), since, None);
        // A reserva e as linhas precisam do mesmo snapshot diante de commits concorrentes.
        let count: i64 = tx.query_row(&format!("SELECT COUNT(*) FROM uso WHERE {filter}"),
            rusqlite::params_from_iter(values.iter()), |r| r.get(0))?;
        let count = usize::try_from(count).map_err(|_| rusqlite::Error::InvalidQuery)?;
        output.reserve_exact(count);
        {
            let mut stmt = tx.prepare(&format!("SELECT {USAGE_FIELDS} FROM uso WHERE {filter} ORDER BY rowid"))?;
            let rows = stmt.query_map(rusqlite::params_from_iter(values.iter()), usage_row)?;
            for row in rows { output.push(decorate(row?)); }
        }
        tx.commit()?;
        Ok(())
    }

    /// Entrega o uso na ordem de `read_usage` sem materializar as linhas. Cada tentativa parte de
    /// uma cópia de `state`: a releitura após reconstruir o índice não soma linhas em dobro.
    pub fn fold_usage<B: Clone>(&self, scope: &str, since: Option<&str>, state: B,
                                visit: &mut dyn FnMut(&mut B, UsoLinha)) -> Result<B, IndexError> {
        self.with_recovery(|| {
            let mut attempt = state.clone();
            self.each_usage(scope, since, &mut |row| visit(&mut attempt, row))?;
            Ok(attempt)
        })
    }

    fn each_usage(&self, scope: &str, since: Option<&str>, visit: &mut dyn FnMut(UsoLinha)) -> Result<(), IndexError> {
        let conn = self.connect()?;
        let (filter, values) = filters(Some(scope), since, None);
        let mut stmt = conn.prepare(&format!("SELECT {USAGE_FIELDS} FROM uso WHERE {filter} ORDER BY rowid"))?;
        for row in stmt.query_map(rusqlite::params_from_iter(values), usage_row)? { visit(row?); }
        Ok(())
    }
}

fn usage_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<UsoLinha> {
    Ok(UsoLinha {
        dia: r.get(0)?, cwd: r.get(1)?, model: r.get(2)?, tipo: r.get(3)?, nome: r.get(4)?,
        plugin: r.get(5)?, detalhe: r.get(6)?, origem: r.get(7)?, chamadas: r.get(8)?,
        ctx_chars: r.get(9)?, tokens_est: r.get(10)?, input: r.get(11)?, output: r.get(12)?,
        cache_write: r.get(13)?, cache_read: r.get(14)?, cache_write_1h: r.get(15)?,
        fast: r.get(16)?, ocupados: r.get(17)?, respostas: r.get(18)?, ocupados_eq: r.get(19)?,
        fonte: r.get(20)?, subagente: r.get(21)?, session_id: r.get(22)?,
    })
}

fn in_pool_scope<'scope, R>(pool: Option<&rayon::ThreadPool>, run: impl FnOnce(&rayon::Scope<'scope>) -> R) -> R {
    match pool {
        Some(pool) => pool.in_place_scope(run),
        None => rayon::in_place_scope(run),
    }
}

fn open_connection(path: &Path, generation: &AtomicU64) -> rusqlite::Result<Connection> {
    let mut conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(30))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    if schema_version(&conn)?.as_deref() != Some(&SCHEMA.to_string()) {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut changed = false;
        if schema_version(&tx)?.as_deref() != Some(&SCHEMA.to_string()) {
            let tables = tx.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")?
                .query_map([], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
            for table in tables { tx.execute_batch(&format!("DROP TABLE \"{}\"", table.replace('"', "\"\"")))?; }
            tx.execute_batch(TABLES)?;
            tx.execute("INSERT INTO meta VALUES ('esquema', ?)", [SCHEMA.to_string()])?;
            changed = true;
        }
        tx.commit()?;
        if changed { generation.fetch_add(1, Ordering::Release); }
    }
    Ok(conn)
}

fn schema_version(conn: &Connection) -> rusqlite::Result<Option<String>> {
    let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='meta')", [], |r| r.get(0))?;
    if !exists { return Ok(None); }
    conn.query_row("SELECT v FROM meta WHERE k='esquema'", [], |r| r.get(0)).optional()
}

struct Fingerprint {
    dev: i64,
    ino: i64,
    size: i64,
    mtime_ns: i64,
}

impl Fingerprint {
    fn from_metadata(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        let (dev, ino, mtime_ns) = {
            use std::os::unix::fs::MetadataExt;
            ((metadata.dev() & i64::MAX as u64) as i64, (metadata.ino() & i64::MAX as u64) as i64,
                metadata.mtime().saturating_mul(1_000_000_000).saturating_add(metadata.mtime_nsec()))
        };
        #[cfg(not(unix))]
        let (dev, ino, mtime_ns) = {
            use std::time::UNIX_EPOCH;
            let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
            let nanos = match modified.duration_since(UNIX_EPOCH) {
                Ok(duration) => duration.as_nanos().min(i64::MAX as u128) as i64,
                Err(error) => -(error.duration().as_nanos().min(i64::MAX as u128) as i64),
            };
            (0, 0, nanos)
        };
        Self { dev, ino, size: metadata.len() as i64, mtime_ns }
    }
}

struct FileRecord {
    id: i64,
    version: String,
    fingerprint: Fingerprint,
    offset: i64,
    tail: Vec<u8>,
    state: Option<Vec<u8>>,
}

impl FileRecord {
    fn light(row: &rusqlite::Row<'_>, start: usize) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(start)?, version: row.get(start + 1)?,
            fingerprint: Fingerprint { dev: row.get(start + 2)?, ino: row.get(start + 3)?, size: row.get(start + 4)?, mtime_ns: row.get(start + 5)? },
            offset: 0, tail: vec![], state: None,
        })
    }

    fn current(&self, f: &Fingerprint, version: &str) -> bool {
        self.version == version && self.fingerprint.dev == f.dev && self.fingerprint.ino == f.ino
            && self.fingerprint.size == f.size && self.fingerprint.mtime_ns == f.mtime_ns
    }
}

fn load_record(conn: &Connection, path: &str) -> rusqlite::Result<Option<FileRecord>> {
    conn.query_row("SELECT id, versao, dev, ino, size, mtime_ns, offset, cauda, estado FROM files WHERE path=?", [path], |row| {
        let mut record = FileRecord::light(row, 0)?;
        record.offset = row.get(6)?;
        record.tail = row.get(7)?;
        record.state = row.get(8)?;
        Ok(record)
    }).optional()
}

struct ReadJob {
    position: usize,
    path: PathBuf,
    fingerprint: Fingerprint,
    record: Option<FileRecord>,
}

struct FileRead {
    offset: i64,
    tail: Vec<u8>,
    state: Vec<u8>,
    output: FoldOutput,
    areas: Option<Vec<u8>>,
    area_rows: Option<Result<Vec<UsoLinha>, ()>>,
    size: i64,
}

impl FileRead {
    // As áreas saem nas threads de leitura; o pânico continua sendo erro do escritor.
    fn with_area_rows(mut self, redo_areas: &dyn Fn(&AreaEntries) -> Vec<UsoLinha>) -> Self {
        self.area_rows = self.output.areas.take()
            .map(|areas| catch_unwind(AssertUnwindSafe(|| redo_areas(&areas))).map_err(|_| ()));
        self
    }
}

enum ReadError { Io, Codec, Fold }

impl From<std::io::Error> for ReadError {
    fn from(_: std::io::Error) -> Self { Self::Io }
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, ReadError> {
    // Cada write no encoder zera o buffer de saída inteiro; o serde_json escreve token a token.
    let mut writer = BufWriter::with_capacity(64 * 1024, ZlibEncoder::new(Vec::new(), Compression::new(1)));
    serde_json::to_writer(&mut writer, value).map_err(|_| ReadError::Codec)?;
    Ok(writer.into_inner().map_err(|_| ReadError::Codec)?.finish()?)
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ReadError> {
    // from_reader lê byte a byte; descomprimir antes evita uma chamada ao zlib por byte.
    let mut json = Vec::new();
    ZlibDecoder::new(bytes).read_to_end(&mut json).map_err(|_| ReadError::Codec)?;
    serde_json::from_slice(&json).map_err(|_| ReadError::Codec)
}

fn read_new<F: Fold>(path: &Path, fingerprint: &Fingerprint, record: Option<&FileRecord>, new_fold: &dyn Fn(&Path) -> F, version: &str) -> Result<FileRead, ReadError> {
    let read = |record| {
        // O hook do servidor oculta o payload antes de a captura isolar a dobra defeituosa.
        catch_unwind(AssertUnwindSafe(|| read_file(path, fingerprint, record, new_fold, version)))
            .unwrap_or(Err(ReadError::Fold))
    };
    match read(record) {
        Err(ReadError::Codec | ReadError::Fold) if record.is_some() => {
            tracing::warn!(code = "retomada_custos");
            read(None)
        },
        result => result,
    }
}

fn read_file<F: Fold>(path: &Path, fingerprint: &Fingerprint, record: Option<&FileRecord>, new_fold: &dyn Fn(&Path) -> F, version: &str) -> Result<FileRead, ReadError> {
    let mut file = File::open(path)?;
    let mut fold = None;
    let mut offset = 0;
    if let Some(record) = record.filter(|r| {
        r.state.is_some() && r.version == version && r.fingerprint.dev == fingerprint.dev
            && r.fingerprint.ino == fingerprint.ino && r.offset >= 0 && r.offset <= fingerprint.size
            && r.tail.len() <= TAIL_BYTES as usize && r.tail.len() as i64 <= r.offset
    }) {
        file.seek(SeekFrom::Start(record.offset as u64 - record.tail.len() as u64))?;
        let mut tail = vec![0; record.tail.len()];
        file.read_exact(&mut tail)?;
        if tail == record.tail {
            match decode(record.state.as_ref().unwrap()) {
                Ok(saved) => { fold = Some(saved); offset = record.offset; },
                Err(_) => tracing::warn!(code = "estado_custos_ilegivel"),
            }
        }
    }
    let mut fold = fold.unwrap_or_else(|| new_fold(path));
    file.seek(SeekFrom::Start(offset as u64))?;
    let mut reader = BufReader::new(&mut file);
    let mut line = Vec::new();
    let mut fragment = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 { break; }
        if line.ends_with(b"\n") {
            fold.line(&line);
            offset += line.len() as i64;
        } else {
            fragment = std::mem::take(&mut line);
        }
    }
    drop(reader);
    let tail_size = (offset as u64).min(TAIL_BYTES);
    file.seek(SeekFrom::Start(offset as u64 - tail_size))?;
    let mut tail = vec![0; tail_size as usize];
    file.read_exact(&mut tail)?;
    // O fragmento ainda pode crescer; só linhas completas pertencem ao estado retomável.
    let state = encode(&fold)?;
    if !fragment.is_empty() { fold.line(&fragment); }
    let output = fold.close();
    let areas = output.areas.as_ref().map(encode).transpose()?;
    Ok(FileRead { offset, tail, state, output, areas, area_rows: None, size: offset + fragment.len() as i64 })
}

fn write_batch(conn: &mut Connection, pending: &mut Vec<(ReadJob, FileRead)>, scope: &str, version: &str, areas_sig: &str, generation: &AtomicU64) -> Result<(), IndexError> {
    if pending.is_empty() { return Ok(()); }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for (job, read) in pending.drain(..) { write_file(&tx, &job, scope, version, read, false, areas_sig)?; }
    tx.commit()?;
    // O erro de um lote posterior não pode ocultar dados já confirmados deste lote.
    generation.fetch_add(1, Ordering::Release);
    Ok(())
}

fn write_file(conn: &Connection, job: &ReadJob, scope: &str, version: &str, read: FileRead, keep_scope: bool, areas_sig: &str) -> Result<i64, IndexError> {
    let f = &job.fingerprint;
    let id = conn.query_row(
        "INSERT INTO files(scope, versao, dev, ino, size, mtime_ns, offset, cauda, estado, areas, areas_sig, path)
        VALUES (?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(path) DO UPDATE SET
        scope=IIF(?, files.scope, excluded.scope), versao=excluded.versao, dev=excluded.dev, ino=excluded.ino,
        size=excluded.size, mtime_ns=excluded.mtime_ns, offset=excluded.offset, cauda=excluded.cauda,
        estado=excluded.estado, areas=excluded.areas, areas_sig=excluded.areas_sig RETURNING id",
        params![scope, version, f.dev, f.ino, read.size, f.mtime_ns, read.offset, read.tail, read.state,
            read.areas, areas_sig, job.path.to_string_lossy(), keep_scope], |r| r.get::<_, i64>(0),
    )?;
    conn.execute("DELETE FROM custo WHERE file_id=?", [id])?;
    conn.execute("DELETE FROM uso WHERE file_id=?", [id])?;
    let mut stmt = conn.prepare("INSERT INTO custo VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)")?;
    for row in read.output.costs {
        stmt.execute(params![id, row.ts.day(), row.ts.iso(), row.source, row.provider, row.model, row.project,
            row.session_id, row.input, row.output, row.cache_write, row.cache_read, row.subagente,
            row.account_id, row.codex_long_context, row.cache_write_1h, row.fast, row.regravado, row.regravado_1h])?;
    }
    write_usage(conn, id, read.output.usage)?;
    if let Some(rows) = read.area_rows {
        write_usage(conn, id, rows.map_err(|_| IndexError::ReaderPanic)?)?;
    }
    Ok(id)
}

fn write_usage(conn: &Connection, id: i64, rows: Vec<UsoLinha>) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare("INSERT INTO uso VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)")?;
    for row in rows {
        stmt.execute(params![id, row.dia, row.cwd, row.model, row.tipo, row.nome, row.plugin, row.detalhe,
            row.origem, row.chamadas, row.ctx_chars, row.tokens_est, row.input, row.output, row.cache_write,
            row.cache_read, row.cache_write_1h, row.fast, row.ocupados, row.respostas, row.ocupados_eq,
            row.fonte, row.subagente, row.session_id])?;
    }
    Ok(())
}

fn redo_saved_areas(conn: &Connection, scope: &str, signature: &str, redo_areas: &dyn Fn(&AreaEntries) -> Vec<UsoLinha>) -> Result<bool, IndexError> {
    let mut stmt = conn.prepare("SELECT id, areas FROM files WHERE scope=? AND areas IS NOT NULL AND areas_sig IS NOT ? ORDER BY rowid")?;
    let records = stmt.query_map(params![scope, signature], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut panicked = false;
    for (id, blob) in records {
        let rows = catch_unwind(AssertUnwindSafe(|| {
            decode::<AreaEntries>(&blob).map(|areas| redo_areas(&areas))
        }));
        let rows = match rows {
            Ok(Ok(rows)) => rows,
            failed => {
                panicked |= failed.is_err();
                tracing::warn!(code = "areas_custos_ilegivel", file_id = id);
                conn.execute("UPDATE files SET versao='' WHERE id=?", [id])?;
                continue;
            },
        };
        conn.execute("DELETE FROM uso WHERE file_id=? AND tipo='area'", [id])?;
        write_usage(conn, id, rows)?;
        conn.execute("UPDATE files SET areas_sig=? WHERE id=?", params![signature, id])?;
    }
    Ok(panicked)
}

fn delete_file(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM custo WHERE file_id=?", [id])?;
    conn.execute("DELETE FROM uso WHERE file_id=?", [id])?;
    conn.execute("DELETE FROM files WHERE id=?", [id])?;
    Ok(())
}

fn filters(scope: Option<&str>, since: Option<&str>, file_id: Option<i64>) -> (String, Vec<rusqlite::types::Value>) {
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    if let Some(scope) = scope {
        clauses.push("file_id IN (SELECT id FROM files WHERE scope=?)");
        values.push(scope.to_owned().into());
    }
    if let Some(id) = file_id { clauses.push("file_id=?"); values.push(id.into()); }
    if let Some(since) = since.filter(|s| !s.is_empty()) { clauses.push("dia>=?"); values.push(since.to_owned().into()); }
    (if clauses.is_empty() { "1".into() } else { clauses.join(" AND ") }, values)
}

#[cfg(test)]
mod recovery_tests {
    use super::*;

    fn usage_fixture(index: &Index) -> Vec<UsoLinha> {
        let rows = (1..=3).map(|n| UsoLinha {
            dia: "2026-10-01".into(), cwd: "projeto".into(), model: "modelo".into(),
            tipo: "ferramenta".into(), nome: format!("ação-{n}"), plugin: "extensão".into(),
            detalhe: "detalhe".into(), origem: "origem".into(), chamadas: n, ctx_chars: 2 * n,
            tokens_est: 3 * n, input: 4 * n, output: 5 * n, cache_write: 6 * n, cache_read: 7 * n,
            cache_write_1h: 8 * n, fast: true, ocupados: 9 * n, respostas: 10 * n, ocupados_eq: 11 * n,
            fonte: "fonte".into(), subagente: true, session_id: "sessão".into(),
        }).collect::<Vec<_>>();
        let conn = index.connect().unwrap();
        conn.execute("INSERT INTO files(id, path, scope, versao) VALUES (1, 'fixture', 'scope', '1')", []).unwrap();
        write_usage(&conn, 1, rows.clone()).unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
        rows
    }

    #[test]
    fn usage_append_moves_complete_rows_in_order_and_preserves_the_existing_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let index = Index::open(dir.path()).unwrap();
        let rows = usage_fixture(&index);
        let prefix = (UsoLinha { nome: "prefixo".into(), ..UsoLinha::default() }, "prévia".to_owned());
        let mut output = vec![prefix.clone()];
        index.append_usage("scope", Some("2026-10-01"), &mut output, |row| (row, "conta".to_owned())).unwrap();
        assert_eq!(output, std::iter::once(prefix.clone()).chain(rows.into_iter().map(|row| (row, "conta".to_owned()))).collect::<Vec<_>>());
        index.append_usage("scope", Some("2026-10-02"), &mut output, |row| (row, "vazia".to_owned())).unwrap();
        assert_eq!(output.len(), 4);
        assert_eq!(index.read_usage("outro", None).unwrap(), vec![]);
    }

    #[test]
    fn usage_append_keeps_one_snapshot_while_another_connection_commits_new_rows() {
        let dir = tempfile::tempdir().unwrap();
        let index = Index::open(dir.path()).unwrap();
        let expected = usage_fixture(&index);
        let added = UsoLinha { nome: "nova após a contagem".into(), dia: "2026-10-01".into(), ..UsoLinha::default() };
        let mut output = Vec::new();
        let mut first = true;
        index.append_usage("scope", None, &mut output, |row| {
            if first {
                first = false;
                write_usage(&index.connect().unwrap(), 1, vec![added.clone()]).unwrap();
            }
            row
        }).unwrap();
        assert_eq!(output, expected);
        let mut after = expected;
        after.push(added);
        assert_eq!(index.read_usage("scope", None).unwrap(), after);
    }

    #[test]
    fn usage_append_discards_partial_rows_after_real_sqlite_conversion_failure() {
        let dir = tempfile::tempdir().unwrap();
        let index = Index::open(dir.path()).unwrap();
        usage_fixture(&index);
        index.connect().unwrap().execute("UPDATE uso SET ctx_chars=x'ff' WHERE chamadas=2", []).unwrap();
        let before = index.generation();
        let prefix = (UsoLinha { nome: "prefixo".into(), ..UsoLinha::default() }, "conta".to_owned());
        let mut output = vec![prefix.clone()];
        let mut appended = 0;
        let error = index.append_usage("scope", None, &mut output, |row| {
            appended += 1;
            (row, "nova".to_owned())
        }).unwrap_err();
        assert!(matches!(error, IndexError::Sqlite(rusqlite::Error::InvalidColumnType(9, _, _))));
        assert_eq!(appended, 1, "a primeira linha é válida antes da coluna inválida");
        assert_eq!(output, vec![prefix], "o destino não publica uma leitura parcial com erro");
        assert_eq!(index.generation(), before, "erro de conversão não recria o banco");
        assert!(index.read_usage("scope", None).is_err());
    }

    #[test]
    fn usage_fold_preserves_prefix_row_order_and_conversion_failure_atomicity() {
        let dir = tempfile::tempdir().unwrap();
        let index = Index::open(dir.path()).unwrap();
        let expected = usage_fixture(&index);
        let prefix = UsoLinha { nome: "prefixo".into(), ..UsoLinha::default() };
        let seed = vec![prefix.clone()];
        let output = index.fold_usage("scope", Some("2026-10-01"), seed.clone(),
            &mut |rows, row| rows.push(row)).unwrap();
        assert_eq!(output, std::iter::once(prefix).chain(expected).collect::<Vec<_>>());
        assert_eq!(index.fold_usage("scope", Some("2026-10-02"), seed.clone(),
            &mut |rows, row| rows.push(row)).unwrap(), seed);
        index.connect().unwrap().execute("UPDATE uso SET ctx_chars=x'ff' WHERE chamadas=2", []).unwrap();
        let before = index.generation();
        let mut visits = 0;
        let error = index.fold_usage("scope", None, seed.clone(), &mut |rows, row| {
            visits += 1;
            rows.push(row);
        }).unwrap_err();
        assert!(matches!(error, IndexError::Sqlite(rusqlite::Error::InvalidColumnType(9, _, _))));
        assert_eq!(visits, 1);
        assert_eq!(seed.len(), 1);
        assert_eq!(index.generation(), before);
        assert!(index.read_usage("scope", None).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn usage_fold_retries_from_a_fresh_clone_after_real_sqlite_corruption() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let index = Index::open(dir.path()).unwrap();
        usage_fixture(&index);
        let conn = index.connect().unwrap();
        let page: i64 = conn.query_row("SELECT rootpage FROM sqlite_master WHERE name='custo'", [], |r| r.get(0)).unwrap();
        let size: i64 = conn.pragma_query_value(None, "page_size", |r| r.get(0)).unwrap();
        drop(conn);
        let before = index.generation();
        let seed = vec![UsoLinha { nome: "prefixo".into(), ..UsoLinha::default() }];
        let mut visits = 0;
        let output = index.fold_usage("scope", None, seed.clone(), &mut |rows, row| {
            visits += 1;
            if visits == 1 {
                let mut file = fs::OpenOptions::new().write(true).open(&index.path).unwrap();
                file.seek(SeekFrom::Start(((page - 1) * size) as u64)).unwrap();
                file.write_all(&[0xff]).unwrap();
                file.sync_all().unwrap();
                let error = index.read_costs(None, None, None).unwrap_err();
                assert!(matches!(error, IndexError::Sqlite(ref error) if error.sqlite_error_code() == Some(ErrorCode::DatabaseCorrupt)));
                assert!(index.operations.lock().unwrap().pending);
            }
            rows.push(row);
        }).unwrap();
        assert_eq!(visits, 3, "a tentativa condenada precisa realmente consumir as três linhas");
        assert_eq!(output, seed, "a releitura não publica as linhas da tentativa condenada");
        assert!(index.generation() > before);
        assert!(index.read_usage("scope", None).unwrap().is_empty());
        assert!(!index.operations.lock().unwrap().pending);
    }

    #[cfg(unix)]
    #[test]
    fn usage_append_discards_rows_from_the_condemned_attempt_before_recovery() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let index = Index::open(dir.path()).unwrap();
        usage_fixture(&index);
        let conn = index.connect().unwrap();
        let page: i64 = conn.query_row("SELECT rootpage FROM sqlite_master WHERE name='custo'", [], |r| r.get(0)).unwrap();
        let size: i64 = conn.pragma_query_value(None, "page_size", |r| r.get(0)).unwrap();
        drop(conn);
        let before = index.generation();
        let prefix = (UsoLinha { nome: "prefixo".into(), ..UsoLinha::default() }, "conta".to_owned());
        let mut output = vec![prefix.clone()];
        let mut appended = 0;
        index.append_usage("scope", None, &mut output, |row| {
            appended += 1;
            if appended == 1 {
                let mut file = fs::OpenOptions::new().write(true).open(&index.path).unwrap();
                file.seek(SeekFrom::Start(((page - 1) * size) as u64)).unwrap();
                file.write_all(&[0xff]).unwrap();
                file.sync_all().unwrap();
                let error = index.read_costs(None, None, None).unwrap_err();
                assert!(matches!(error, IndexError::Sqlite(ref error) if error.sqlite_error_code() == Some(ErrorCode::DatabaseCorrupt)));
                assert!(index.operations.lock().unwrap().pending);
            }
            (row, "condenada".to_owned())
        }).unwrap();
        assert_eq!(appended, 3, "a tentativa condenada realmente acrescentou linhas antes de repetir");
        assert_eq!(output, vec![prefix], "o novo banco vazio não recebe linhas da tentativa anterior");
        assert!(index.generation() > before);
        assert!(index.read_usage("scope", None).unwrap().is_empty());
        assert!(!index.operations.lock().unwrap().pending);
    }

    #[test]
    fn real_sqlite_lock_and_disk_full_errors_do_not_remove_the_index() {
        for expected in [ErrorCode::DatabaseBusy, ErrorCode::DatabaseLocked, ErrorCode::DiskFull] {
            let dir = tempfile::tempdir().unwrap();
            let index = Index::open(dir.path()).unwrap();
            let conn = Connection::open(&index.path).unwrap();
            conn.execute_batch("CREATE TABLE preserved(value BLOB); INSERT INTO preserved VALUES (1)").unwrap();
            let before = index.generation();
            let calls = AtomicUsize::new(0);
            let result = index.with_recovery(|| -> Result<(), IndexError> {
                calls.fetch_add(1, Ordering::SeqCst);
                let error = match expected {
                    ErrorCode::DatabaseBusy => {
                        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
                        let other = Connection::open(&index.path).unwrap();
                        other.busy_timeout(Duration::ZERO).unwrap();
                        let error = other.execute_batch("INSERT INTO preserved VALUES (2)").unwrap_err();
                        conn.execute_batch("ROLLBACK").unwrap();
                        error
                    },
                    ErrorCode::DatabaseLocked => {
                        let mut stmt = conn.prepare("SELECT * FROM preserved").unwrap();
                        let mut rows = stmt.query([]).unwrap();
                        assert!(rows.next().unwrap().is_some());
                        conn.execute_batch("DROP TABLE preserved").unwrap_err()
                    },
                    _ => {
                        let count: i64 = conn.pragma_query_value(None, "page_count", |r| r.get(0)).unwrap();
                        conn.pragma_update(None, "max_page_count", count).unwrap();
                        conn.execute_batch("INSERT INTO preserved VALUES (zeroblob(1000000))").unwrap_err()
                    },
                };
                assert_eq!(error.sqlite_error_code(), Some(expected));
                Err(error.into())
            });
            assert!(if expected == ErrorCode::DiskFull { matches!(result, Err(IndexError::NoDisk)) } else { matches!(result, Err(IndexError::Sqlite(_))) });
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert_eq!(index.generation(), before);
            for suffix in ["", "-wal", "-shm"] { assert!(dir.path().join(format!("{FILE_NAME}{suffix}")).exists()); }
            assert_eq!(conn.query_row("SELECT value FROM preserved", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
            let reopened = Connection::open(&index.path).unwrap();
            assert_eq!(reopened.query_row("SELECT value FROM preserved", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        }
    }

    #[test]
    fn late_error_from_a_rebuilt_epoch_retries_without_removing_new_data() {
        let dir = tempfile::tempdir().unwrap();
        let index = Index::open(dir.path()).unwrap();
        let old_epoch = index.operations.lock().unwrap().epoch;
        let broken = dir.path().join("broken.sqlite3");
        fs::write(&broken, b"not a database").unwrap();
        let error = Connection::open(broken).unwrap().execute_batch("SELECT * FROM sqlite_master").unwrap_err();
        assert_eq!(error.sqlite_error_code(), Some(ErrorCode::NotADatabase));
        index.rebuild(&mut index.operations.lock().unwrap()).unwrap();
        let conn = index.connect().unwrap();
        conn.execute("INSERT INTO meta VALUES ('preserved', 'new')", []).unwrap();
        drop(conn);
        let before = index.generation();
        assert!(index.finish_attempt::<()>(old_epoch, 0, Err(error.into())).unwrap().is_none());
        assert_eq!(index.generation(), before);
        let conn = index.connect().unwrap();
        assert_eq!(conn.query_row("SELECT v FROM meta WHERE k='preserved'", [], |r| r.get::<_, String>(0)).unwrap(), "new");
        assert!(!index.operations.lock().unwrap().pending);
    }
}
