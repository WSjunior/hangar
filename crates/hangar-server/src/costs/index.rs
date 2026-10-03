//! Índice descartável em disco, com leitura retomável e um escritor por varredura.

use super::py::LocalTs;
use super::rows::{AreaEntries, FoldOutput, UsageRow, UsoLinha};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use rayon::prelude::*;
use rusqlite::{Connection, ErrorCode, OptionalExtension, TransactionBehavior, params};
use serde::{Serialize, de::DeserializeOwned};
use std::collections::BTreeMap;
use std::fs::{self, File, Metadata};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}, mpsc};
use std::time::{Duration, Instant};

pub const SCHEMA: u32 = 1;
pub const FILE_NAME: &str = "custos-rust.sqlite3";
const TAIL_BYTES: u64 = 64;
const BATCH_TIME: Duration = Duration::from_secs(1);
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

    fn set(&self, scope: &str, read: usize, total: usize) {
        let mut scopes = self.scopes.lock().unwrap();
        let value = scopes.entry(scope.to_owned()).or_default();
        value.read.store(read, Ordering::Relaxed);
        value.total.store(total, Ordering::Relaxed);
    }
}

pub struct Index {
    path: PathBuf,
    pool: Option<Arc<rayon::ThreadPool>>,
}

pub fn default_dir() -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from).unwrap_or_default();
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()).map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Local")).join("hangar").join("custos")
    } else {
        home.join(".claude").join(".hangar-custos")
    }
}

impl Index {
    pub fn open(dir: &Path) -> Result<Self, IndexError> {
        let index = Self { path: dir.join(FILE_NAME), pool: None };
        index.connect()?;
        Ok(index)
    }

    /// Seleciona o pool compartilhado sem deslocar o escritor para uma das suas threads.
    pub fn with_pool(mut self, pool: Arc<rayon::ThreadPool>) -> Self {
        self.pool = Some(pool);
        self
    }

    fn connect(&self) -> Result<Connection, IndexError> {
        fs::create_dir_all(self.path.parent().unwrap()).map_err(|_| IndexError::NoDisk)?;
        match open_connection(&self.path) {
            Ok(conn) => Ok(conn),
            Err(error) if matches!(error.sqlite_error_code(), Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase)) => {
                tracing::warn!(code = "indice_custos_ilegivel");
                // A conexão anterior já fechou; no Windows um arquivo aberto não pode ser apagado.
                for suffix in ["", "-wal", "-shm"] {
                    let mut path = self.path.as_os_str().to_os_string();
                    path.push(suffix);
                    match fs::remove_file(PathBuf::from(path)) {
                        Ok(()) => {},
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                        Err(_) => return Err(IndexError::NoDisk),
                    }
                }
                open_connection(&self.path).map_err(Into::into)
            },
            Err(error) => Err(error.into()),
        }
    }

    pub fn sync<F: Fold>(
        &self, scope: &str, files: &[PathBuf], new_fold: &(dyn Fn(&Path) -> F + Sync),
        version: &str, areas_sig: &str, redo_areas: &dyn Fn(&AreaEntries) -> Vec<UsoLinha>,
        progress: &Progress,
    ) -> Result<bool, IndexError> {
        progress.set(scope, 0, files.len());
        let result = self.sync_inner(scope, files, new_fold, version, areas_sig, redo_areas, progress);
        // Fontes concluídas continuam na soma exibida durante o aquecimento.
        progress.set(scope, files.len(), files.len());
        result
    }

    fn sync_inner<F: Fold>(
        &self, scope: &str, files: &[PathBuf], new_fold: &(dyn Fn(&Path) -> F + Sync),
        version: &str, areas_sig: &str, redo_areas: &dyn Fn(&AreaEntries) -> Vec<UsoLinha>,
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
            let record = load_record(&conn, &key)?;
            if record.as_ref().is_some_and(|r| r.current(&fingerprint, &version)) {
                conn.execute("UPDATE files SET scope=? WHERE id=? AND scope<>?", params![scope, record.unwrap().id, scope])?;
                continue;
            }
            jobs.push(ReadJob { position, path: path.clone(), fingerprint, record });
        }

        let (sender, receiver) = mpsc::channel();
        // O chamador escreve; só as leituras usam o pool corrente, inclusive um pool privado.
        in_pool_scope(self.pool.as_deref(), |rayon_scope| -> Result<(), IndexError> {
            let jobs_ref = &jobs;
            let version_ref = &version;
            rayon_scope.spawn(move |_| {
                jobs_ref.par_iter().enumerate().for_each_with(sender, |sender, (order, job)| {
                    let read = read_new(&job.path, &job.fingerprint, job.record.as_ref(), new_fold, version_ref);
                    let _ = sender.send((order, read));
                });
            });
            let mut ready = BTreeMap::new();
            let mut pending = Vec::new();
            let mut next = 0;
            let mut reader_panicked = false;
            let mut batch_started = Instant::now();
            while next < jobs.len() {
                // Um pool de um núcleo pode ter o escritor dentro dele; ceder evita bloqueá-lo.
                rayon::yield_now();
                match receiver.recv_timeout(Duration::from_millis(10)) {
                    Ok((order, result)) => { ready.insert(order, result); },
                    Err(mpsc::RecvTimeoutError::Disconnected) if !ready.contains_key(&next) => return Err(IndexError::ReaderPanic),
                    Err(_) => {},
                }
                while let Some(result) = ready.remove(&next) {
                    let job = &jobs[next];
                    progress.set(scope, job.position + 1, files.len());
                    match result {
                        Ok(read) => pending.push((job, read)),
                        Err(ReadError::Fold) => {
                            reader_panicked = true;
                            tracing::warn!(code = "panico_leitura_custos");
                        },
                        Err(_) => tracing::warn!(code = "leitura_custos"),
                    }
                    next += 1;
                }
                if batch_started.elapsed() >= BATCH_TIME {
                    write_batch(&mut conn, &mut pending, scope, &version, areas_sig, redo_areas)?;
                    batch_started = Instant::now();
                }
            }
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            for (job, read) in pending {
                write_file(&tx, job, scope, &version, read, false, areas_sig, redo_areas)?;
            }
            for record in known.values() { delete_file(&tx, record.id)?; }
            reader_panicked |= redo_saved_areas(&tx, scope, areas_sig, redo_areas)?;
            tx.commit()?;
            if reader_panicked { Err(IndexError::ReaderPanic) } else { Ok(()) }
        })?;
        Ok(conn.total_changes() != before)
    }

    pub fn sync_file<F: Fold>(
        &self, path: &Path, new_fold: &dyn Fn(&Path) -> F, version: &str, scope: &str,
        areas_sig: &str, redo_areas: &dyn Fn(&AreaEntries) -> Vec<UsoLinha>,
    ) -> Option<i64> {
        let result = (|| -> Result<i64, IndexError> {
            let mut conn = self.connect()?;
            let fingerprint = Fingerprint::from_metadata(&fs::metadata(path).map_err(|_| IndexError::NoDisk)?);
            let version = format!("{SCHEMA}:{version}");
            let record = load_record(&conn, &path.to_string_lossy())?;
            if let Some(record) = record.as_ref().filter(|r| r.current(&fingerprint, &version)) {
                return Ok(record.id);
            }
            let read = read_new(path, &fingerprint, record.as_ref(), new_fold, &version)
                .map_err(|_| IndexError::NoDisk)?;
            let job = ReadJob { position: 0, path: path.to_owned(), fingerprint, record };
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let id = write_file(&tx, &job, scope, &version, read, true, areas_sig, redo_areas)?;
            tx.commit()?;
            Ok(id)
        })();
        match result {
            Ok(id) => Some(id),
            Err(_) => { tracing::warn!(code = "leitura_custos"); None },
        }
    }

    pub fn forget_outside(&self, active: &[String]) -> Result<bool, IndexError> {
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
        Ok(true)
    }

    pub fn read_costs(&self, scope: Option<&str>, since: Option<&str>, file_id: Option<i64>) -> Result<Vec<UsageRow>, IndexError> {
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
        let conn = self.connect()?;
        let (filter, values) = filters(Some(scope), since, None);
        let mut stmt = conn.prepare(&format!("SELECT {USAGE_FIELDS} FROM uso WHERE {filter} ORDER BY rowid"))?;
        let rows = stmt.query_map(rusqlite::params_from_iter(values), |r| Ok(UsoLinha {
            dia: r.get(0)?, cwd: r.get(1)?, model: r.get(2)?, tipo: r.get(3)?, nome: r.get(4)?,
            plugin: r.get(5)?, detalhe: r.get(6)?, origem: r.get(7)?, chamadas: r.get(8)?,
            ctx_chars: r.get(9)?, tokens_est: r.get(10)?, input: r.get(11)?, output: r.get(12)?,
            cache_write: r.get(13)?, cache_read: r.get(14)?, cache_write_1h: r.get(15)?,
            fast: r.get(16)?, ocupados: r.get(17)?, respostas: r.get(18)?, ocupados_eq: r.get(19)?,
            fonte: r.get(20)?, subagente: r.get(21)?, session_id: r.get(22)?,
        }))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

fn in_pool_scope<'scope, R>(pool: Option<&rayon::ThreadPool>, run: impl FnOnce(&rayon::Scope<'scope>) -> R) -> R {
    match pool {
        Some(pool) => pool.in_place_scope(run),
        None => rayon::in_place_scope(run),
    }
}

fn open_connection(path: &Path) -> rusqlite::Result<Connection> {
    let mut conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(30))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    if schema_version(&conn)?.as_deref() != Some(&SCHEMA.to_string()) {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if schema_version(&tx)?.as_deref() != Some(&SCHEMA.to_string()) {
            let tables = tx.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")?
                .query_map([], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
            for table in tables { tx.execute_batch(&format!("DROP TABLE \"{}\"", table.replace('"', "\"\"")))?; }
            tx.execute_batch(TABLES)?;
            tx.execute("INSERT INTO meta VALUES ('esquema', ?)", [SCHEMA.to_string()])?;
        }
        tx.commit()?;
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
    size: i64,
}

enum ReadError { Io, Codec, Fold }

impl From<std::io::Error> for ReadError {
    fn from(_: std::io::Error) -> Self { Self::Io }
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, ReadError> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(1));
    serde_json::to_writer(&mut encoder, value).map_err(|_| ReadError::Codec)?;
    Ok(encoder.finish()?)
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ReadError> {
    serde_json::from_reader(ZlibDecoder::new(bytes)).map_err(|_| ReadError::Codec)
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
    Ok(FileRead { offset, tail, state, output, areas, size: offset + fragment.len() as i64 })
}

fn write_batch(conn: &mut Connection, pending: &mut Vec<(&ReadJob, FileRead)>, scope: &str, version: &str, areas_sig: &str, redo_areas: &dyn Fn(&AreaEntries) -> Vec<UsoLinha>) -> Result<(), IndexError> {
    if pending.is_empty() { return Ok(()); }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for (job, read) in pending.drain(..) { write_file(&tx, job, scope, version, read, false, areas_sig, redo_areas)?; }
    tx.commit()?;
    Ok(())
}

fn write_file(conn: &Connection, job: &ReadJob, scope: &str, version: &str, read: FileRead, keep_scope: bool, areas_sig: &str, redo_areas: &dyn Fn(&AreaEntries) -> Vec<UsoLinha>) -> Result<i64, IndexError> {
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
    if let Some(areas) = read.output.areas {
        let rows = catch_unwind(AssertUnwindSafe(|| redo_areas(&areas))).map_err(|_| IndexError::ReaderPanic)?;
        write_usage(conn, id, rows)?;
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
