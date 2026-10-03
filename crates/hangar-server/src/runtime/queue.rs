use super::protocol::{ClockSample, RequestId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{mpsc, oneshot};

const CALL_PREFIX: &str = "call::";
const CAP: usize = 1000;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status { Prepared, Dispatching, Accepted, Deferred, Rejected, Unknown, Confirmed }

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub id: String,
    pub payload: Value,
    pub entry_id: Option<String>,
    pub status: Status,
    pub result: Value,
    pub dispatch_cursor: Value,
    pub wire_attempts: BTreeMap<String, Value>,
}

impl Operation {
    fn new(id: &str, payload: Value, entry_id: Option<String>) -> Self {
        Self { id:id.into(), payload, entry_id, status:Status::Prepared,
            result:Value::Null, dispatch_cursor:Value::Null, wire_attempts:BTreeMap::new() }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub version: u32,
    pub owner_key: String,
    pub generation: u64,
    pub name: String,
    pub rows: Vec<Value>,
    pub operations: BTreeMap<String, Operation>,
    pub used_occurrences: BTreeMap<String, Value>,
    pub runtime_state: Value,
}

impl State {
    pub fn new(key: &str, generation: u64, name: &str, rows: Vec<Value>) -> Self {
        Self { version:1, owner_key:key.into(), generation, name:name.into(), rows,
            operations:BTreeMap::new(), used_occurrences:BTreeMap::new(), runtime_state:json!({}) }
    }

    fn protected(&self, entry_id: &str) -> bool {
        self.operations.iter().any(|(key, op)| !key.starts_with(CALL_PREFIX)
            && op.entry_id.as_deref() == Some(entry_id) && matches!(op.status, Status::Dispatching | Status::Unknown))
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Load,
    Append { text:String, delivered:bool, ts:Option<f64>, pre_transcript:bool, entry_id:Option<String> },
    AppendLocal { text:String, entry_id:Option<String> },
    Claim { min_ts:f64, limit:Option<usize>, entry_id:Option<String> },
    SetDelivered { entry_id:String, value:bool, steered:bool },
    Abandon { entry_id:String },
    BumpAttempts { entry_id:String },
    EntryDelivered { entry_id:String },
    Confirm { entry_ids:Vec<String> },
    Prune { min_ts:f64 },
    Reconcile { committed:Vec<String>, min_ts:f64, now:f64, grace:f64, max_attempts:u64,
        confirm_only:bool, na_fila_tui:Vec<String> },
    Remove { entry_id:String },
    Clear,
    Rename { name:String },
    Prepare { id:String, payload:Value, entry_id:Option<String> },
    BindDispatch { id:String, cursor:Value },
    BeginDispatch { id:String, wire_id:String },
    Finish { id:String, status:Status, result:Value },
    ConfirmOccurrence { id:String, proof:super::receipt::ReceiptProof },
    LateRpcResolution { id:String, wire_id:String, request_id:RequestId, generation:u64, result:Value },
    Recover,
    EnsureProjection,
    SetRuntimeState { state:Value },
    ReplaceRows { rows:Vec<Value> },
}

pub struct Store {
    state_path: PathBuf,
    projection_dir: PathBuf,
    state: State,
    fenced: bool,
}

fn invalid(message: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message) }

pub fn acquire_lease(path: &Path) -> io::Result<Arc<File>> {
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
    let file = options.open(path)?;
    file.try_lock().map_err(io::Error::from)?;
    Ok(Arc::new(file))
}
fn row_id(row: &Value) -> &str { row["id"].as_str().unwrap_or("") }
fn current(row: &Value, min_ts: f64) -> bool {
    row["ts"].as_f64().unwrap_or(0.0) >= min_ts - if row["pre_transcript"] == true { 900.0 } else { 0.0 }
}
fn sanitize(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c,'_' | '-' | '.') { c } else { '-' }).collect()
}

impl Store {
    pub fn open(state_path: &Path, projection_dir: &Path, initial: State) -> io::Result<Self> {
        if let Some(parent) = state_path.parent() { std::fs::create_dir_all(parent)?; }
        std::fs::create_dir_all(projection_dir)?;
        let state = match std::fs::read(state_path) {
            Ok(bytes) => {
                let state: State = serde_json::from_slice(&bytes).map_err(|_| invalid("estado da fila inválido"))?;
                if state.version != 1 || state.owner_key != initial.owner_key || !state.rows.iter().all(Value::is_object)
                    || !state.runtime_state.is_object() { return Err(invalid("estado da fila incompatível")); }
                state
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                atomic_write(state_path, &serde_json::to_vec(&initial)?)?;
                initial
            }
            Err(error) => return Err(error),
        };
        let mut store = Self { state_path:state_path.into(), projection_dir:projection_dir.into(), state, fenced:false };
        store.ensure_projection()?;
        Ok(store)
    }

    pub fn state(&self) -> &State { &self.state }

    pub fn ensure_projection(&mut self) -> io::Result<()> {
        let mut output = Vec::new();
        for row in &self.state.rows { serde_json::to_writer(&mut output, row)?; output.push(b'\n'); }
        atomic_write(&self.projection_dir.join(format!("{}.jsonl", sanitize(&self.state.name))), &output)?;
        if let Some(previous) = self.state.runtime_state["_queue_previous_name"].as_str()
            .filter(|p| *p != self.state.name) {
            match std::fs::remove_file(self.projection_dir.join(format!("{}.jsonl", sanitize(previous)))) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
        }
        Ok(())
    }

    pub fn exec(&mut self, generation: u64, call_id: &str, clock: ClockSample, action: Action) -> io::Result<Value> {
        if self.fenced {
            if !matches!(&action, Action::EnsureProjection) { return Err(invalid("fila bloqueada após falha de persistência")); }
            self.state = serde_json::from_slice(&std::fs::read(&self.state_path)?)
                .map_err(|_| invalid("estado comprometido da fila inválido"))?;
            self.ensure_projection()?;
            self.fenced = false;
        }
        if generation != self.state.generation { return Err(invalid("geração da fila mudou")); }
        if call_id.is_empty() { return Err(invalid("operação da fila sem identificador")); }
        let receipt_id = format!("{CALL_PREFIX}{call_id}");
        let payload = serde_json::to_value(&action)?;
        if let Some(previous) = self.state.operations.get(&receipt_id) {
            if previous.payload != payload { return Err(invalid("identificador reutilizado com outra operação")); }
            let result = previous.result.clone();
            self.ensure_projection()?;
            return Ok(result);
        }
        let readonly = matches!(&action, Action::Load | Action::EntryDelivered { .. } | Action::EnsureProjection);
        if readonly { self.ensure_projection()?; }
        let mut state = self.state.clone();
        let result = apply(&mut state, action, clock, call_id)?;
        if !readonly {
            let mut receipt = Operation::new(&receipt_id, payload, None);
            receipt.status = Status::Accepted;
            receipt.result = result.clone();
            state.operations.insert(receipt_id, receipt);
            self.fenced = true;
            atomic_write(&self.state_path, &serde_json::to_vec(&state)?)?;
            self.state = state;
            self.fenced = false;
            self.ensure_projection()?;
        }
        Ok(result)
    }
}

fn append_row(state: &mut State, row: Value) -> io::Result<Value> {
    if state.rows.iter().any(|r| row_id(r) == row_id(&row)) { return Err(invalid("entrada da fila já existe")); }
    let overflow = state.rows.len().saturating_add(1).saturating_sub(CAP);
    let candidates: Vec<_> = state.rows.iter().filter(|r| (r["confirmed"] == true || r["papel"] == "assistant")
        && !state.protected(row_id(r))).take(overflow).map(|r| row_id(r).to_owned()).collect();
    if candidates.len() < overflow { return Err(invalid("fila cheia de entradas pendentes")); }
    state.rows.retain(|r| !candidates.iter().any(|id| id == row_id(r)));
    state.rows.push(row.clone());
    Ok(row)
}

fn apply(state: &mut State, action: Action, clock: ClockSample, call_id: &str) -> io::Result<Value> {
    let protected: BTreeSet<String> = state.rows.iter().filter(|r| state.protected(row_id(r))).map(|r| row_id(r).into()).collect();
    let result = match action {
        Action::Load => json!(state.rows),
        Action::EnsureProjection => Value::Null,
        Action::Append { text, delivered, ts, pre_transcript, entry_id } => {
            let mut row = json!({"id":entry_id.unwrap_or_else(||call_id.into()),"text":text,"ts":ts.unwrap_or(clock.epoch_s),"delivered":delivered});
            if pre_transcript { row["pre_transcript"] = json!(true); }
            append_row(state,row)?
        }
        Action::AppendLocal { text, entry_id } => append_row(state,json!({"id":entry_id.unwrap_or_else(||call_id.into()),
            "text":text,"ts":clock.epoch_s,"delivered":true,"confirmed":true,"papel":"assistant"}))?,
        Action::Claim { min_ts, limit, entry_id } => {
            let mut claimed = Vec::new();
            for row in &mut state.rows {
                if protected.contains(row_id(row)) || entry_id.as_deref().is_some_and(|id| id != row_id(row)) { continue; }
                if row["delivered"] == false && current(row,min_ts) {
                    row["delivered"] = json!(true);
                    claimed.push(row.clone());
                    if limit.is_some_and(|limit| claimed.len() >= limit) { break; }
                }
            }
            json!(claimed)
        }
        Action::SetDelivered { entry_id, value, steered } => {
            if !value && protected.contains(&entry_id) { return Err(invalid("entrega incerta não pode voltar para a fila")); }
            if let Some(row) = state.rows.iter_mut().find(|r| row_id(r) == entry_id && !protected.contains(&entry_id)) {
                row["delivered"] = json!(value);
                if value && steered { row["steered"] = json!(true); }
                else if !value { row.as_object_mut().unwrap().remove("steered"); }
            }
            Value::Null
        }
        Action::Abandon { entry_id } => {
            if let Some(row) = state.rows.iter_mut().find(|r| row_id(r) == entry_id) {
                row["delivered"] = json!(true); row["desistiu"] = json!(true); row["desistiu_ts"] = json!(clock.epoch_s);
            }
            Value::Null
        }
        Action::BumpAttempts { entry_id } => {
            if let Some(row) = state.rows.iter_mut().find(|r| row_id(r) == entry_id && !protected.contains(&entry_id)) {
                let count = row["attempts"].as_u64().unwrap_or(0) + 1;
                row["attempts"] = json!(count); json!(count)
            } else { json!(0) }
        }
        Action::EntryDelivered { entry_id } => state.rows.iter().find(|r| row_id(r) == entry_id)
            .map(|r|json!(r["delivered"].as_bool().unwrap_or(false))).unwrap_or(Value::Null),
        Action::Confirm { entry_ids } => {
            let mut count = 0;
            for row in &mut state.rows {
                if !protected.contains(row_id(row)) && entry_ids.iter().any(|id| id == row_id(row)) && row["delivered"] == true && row["confirmed"] != true {
                    row["confirmed"] = json!(true); count += 1;
                }
            }
            json!(count)
        }
        Action::Prune { min_ts } => {
            let before = state.rows.len();
            if min_ts > 0.0 { state.rows.retain(|r| protected.contains(row_id(r)) || current(r,min_ts)); }
            json!(before - state.rows.len())
        }
        Action::Remove { entry_id } => {
            let before = state.rows.len();
            state.rows.retain(|r| row_id(r) != entry_id || r["desistiu"] != true || protected.contains(&entry_id));
            json!(before != state.rows.len())
        }
        Action::Clear => { state.rows.clear(); Value::Null }
        Action::Rename { name } => {
            if name.is_empty() { return Err(invalid("nome vazio na fila")); }
            state.runtime_state["_queue_previous_name"] = json!(state.name);
            state.name = name;
            Value::Null
        }
        Action::Prepare { id, payload, entry_id } => {
            if id.starts_with(CALL_PREFIX) { return Err(invalid("identificador reservado")); }
            if let Some(old) = state.operations.get(&id) {
                if old.payload != payload || old.entry_id != entry_id { return Err(invalid("intenção da operação mudou")); }
            } else { state.operations.insert(id.clone(),Operation::new(&id,payload,entry_id)); }
            if state.operations[&id].status == Status::Deferred {
                let old = state.operations.get_mut(&id).unwrap(); old.status = Status::Prepared; old.result = Value::Null;
            }
            serde_json::to_value(&state.operations[&id])?
        }
        Action::BindDispatch { id, cursor } => {
            let op = state.operations.get_mut(&id).ok_or_else(||invalid("operação não preparada"))?;
            if op.status != Status::Prepared { return Err(invalid("cursor precisa preceder o despacho")); }
            op.dispatch_cursor = cursor;
            serde_json::to_value(op)?
        }
        Action::BeginDispatch { id, wire_id } => {
            let op = state.operations.get_mut(&id).ok_or_else(||invalid("operação não preparada"))?;
            if !matches!(op.status,Status::Prepared | Status::Dispatching) { return Err(invalid("operação não pode ser reenviada")); }
            op.wire_attempts.entry(wire_id).or_insert_with(||json!({"status":"dispatching","result":null}));
            op.status = Status::Dispatching;
            for row in &mut state.rows { if Some(row_id(row)) == op.entry_id.as_deref() { row["delivered"] = json!(true); } }
            serde_json::to_value(op)?
        }
        Action::Finish { id, status, result } => {
            let op = state.operations.get_mut(&id).ok_or_else(||invalid("operação não preparada"))?;
            if matches!(op.status,Status::Accepted | Status::Confirmed | Status::Rejected)
                && op.result.get("disposition").is_some() && result.get("write_outcome").is_some() {
                return Ok(serde_json::to_value(op)?);
            }
            if matches!(op.status,Status::Accepted | Status::Confirmed | Status::Rejected) && status == Status::Unknown {
                return Ok(serde_json::to_value(op)?);
            }
            if op.status == Status::Unknown && matches!(status,Status::Prepared | Status::Dispatching | Status::Deferred) {
                return Err(invalid("resultado incerto não permite reenvio"));
            }
            op.status = status; op.result = result;
            serde_json::to_value(op)?
        }
        Action::LateRpcResolution { id, wire_id, request_id, generation, result } => {
            if generation != state.generation { return Err(invalid("resposta de outra geração")); }
            let op = state.operations.get_mut(&id).ok_or_else(||invalid("operação não preparada"))?;
            let attempt = op.wire_attempts.get_mut(&wire_id).ok_or_else(||invalid("tentativa não registrada"))?;
            let expected = op.payload.get("request_id").or_else(||op.payload.get("id"));
            if expected != Some(&serde_json::to_value(&request_id)?) { return Err(invalid("resposta não corresponde ao pedido")); }
            attempt["status"] = json!("accepted"); attempt["result"] = result.clone();
            if op.wire_attempts.values().all(|a|a["status"] == "accepted") && op.status != Status::Confirmed {
                op.status = Status::Accepted; op.result = result;
            }
            serde_json::to_value(op)?
        }
        Action::ConfirmOccurrence { id, proof } => {
            if state.used_occurrences.contains_key(&proof.occurrence.id) { return Ok(json!(false)); }
            let operation = state.operations.get(&id).ok_or_else(||invalid("operação não preparada"))?;
            if operation.status == Status::Confirmed { return Ok(json!(false)); }
            let cursor: super::receipt::DispatchCursor = serde_json::from_value(operation.dispatch_cursor.clone())
                .map_err(|_|invalid("operação sem cursor de despacho"))?;
            let row = state.rows.iter_mut().find(|r|Some(row_id(r)) == operation.entry_id.as_deref())
                .ok_or_else(||invalid("entrada da operação não existe"))?;
            if row["confirmed"] == true { return Ok(json!(false)); }
            if !proof.validates(&cursor,row) { return Err(invalid("prova de entrega não corresponde ao despacho")); }
            row["delivered"] = json!(true); row["confirmed"] = json!(true);
            row.as_object_mut().unwrap().remove("desistiu");
            state.used_occurrences.insert(proof.occurrence.id.clone(),json!({"operation_id":id,"generation":state.generation}));
            state.operations.get_mut(&id).unwrap().status = Status::Confirmed;
            json!(true)
        }
        Action::Recover => {
            for op in state.operations.values_mut() {
                if op.status == Status::Dispatching { op.status = Status::Unknown; }
                for attempt in op.wire_attempts.values_mut() {
                    if attempt["status"] == "dispatching" { attempt["status"] = json!("unknown"); }
                }
            }
            let phases:BTreeSet<_> = state.operations.values().filter(|op|matches!(op.status,Status::Accepted|Status::Unknown|Status::Dispatching|Status::Confirmed))
                .filter_map(|op|op.payload["logical_id"].as_str()).collect();
            let protected:BTreeSet<_> = state.operations.values().filter(|op|matches!(op.status,Status::Accepted|Status::Unknown|Status::Dispatching|Status::Confirmed)
                || phases.contains(op.id.as_str())).filter_map(|op|op.entry_id.as_deref()).collect();
            let claimed:BTreeSet<_> = state.operations.iter().filter(|(call,op)|call.starts_with("call::terminal:queue:") && op.payload["kind"]=="claim")
                .flat_map(|(_,op)|op.result.as_array().into_iter().flatten()).filter_map(|row|row["id"].as_str()).collect();
            // Só o claim do executor terminal, sem despacho, prova ausência de efeito na TUI.
            for row in &mut state.rows {
                let id=row_id(row);
                if row["delivered"]==true && row["confirmed"]!=true && row["desistiu"]!=true && claimed.contains(id) && !protected.contains(id) {
                    row["delivered"]=json!(false);
                }
            }
            Value::Null
        }
        Action::SetRuntimeState { state: runtime_state } => {
            if !runtime_state.is_object() { return Err(invalid("estado privado inválido")); }
            state.runtime_state = runtime_state;
            Value::Null
        }
        Action::ReplaceRows { rows } => {
            if state.rows.iter().any(|r| protected.contains(row_id(r)) && !rows.contains(r)) {
                return Err(invalid("substituição removeria uma entrada incerta"));
            }
            state.rows = rows;
            Value::Null
        }
        Action::Reconcile { committed, min_ts, now, grace, max_attempts, confirm_only, na_fila_tui } => {
            reconcile(state,&protected,committed,min_ts,now,grace,max_attempts,confirm_only,na_fila_tui)
        }
    };
    Ok(result)
}

pub(crate) fn entry_lines(row: &Value) -> BTreeSet<String> {
    let raw = row["text"].as_str().unwrap_or("").trim();
    let stripped = crate::transcript::history::strip_attach(raw);
    let mut lines: BTreeSet<_> = [raw,stripped.trim()].into_iter()
        .flat_map(|text|std::iter::once(text).chain(text.split('\n')))
        .map(str::trim).filter(|text|!text.is_empty()).map(str::to_owned).collect();
    let shrunk: Vec<_> = lines.iter().map(|text| {
        let mut out = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '\\' { out.push(c); continue; }
            let mut count: usize = 1;
            while chars.peek() == Some(&'\\') { chars.next(); count += 1; }
            out.extend(std::iter::repeat_n('\\',count.div_ceil(2)));
        }
        out
    }).collect();
    lines.extend(shrunk);
    lines
}

#[allow(clippy::too_many_arguments)]
fn reconcile(state: &mut State, protected: &BTreeSet<String>, committed: Vec<String>, min_ts: f64,
    now: f64, grace: f64, max_attempts: u64, confirm_only: bool, pending: Vec<String>) -> Value {
    let mut available: BTreeSet<String> = committed.into_iter().collect();
    let rows_before = state.rows.clone();
    let mut reserved = BTreeSet::new();
    let mut owners: BTreeMap<String,String> = BTreeMap::new();
    for row in &state.rows {
        if row["delivered"] != true || row["confirmed"] == true || protected.contains(row_id(row)) { continue; }
        let lines = entry_lines(row);
        reserved.extend(lines.intersection(&available).cloned());
        for line in lines.iter().filter(|l|l.chars().count() >= 8) {
            for echo in available.iter().filter(|e|e.starts_with(line.as_str())) {
                if owners.get(echo).is_none_or(|old|old.chars().count() < line.chars().count()) { owners.insert(echo.clone(),line.clone()); }
            }
        }
    }
    let mut requeued = Vec::new();
    let mut removed = BTreeSet::new();
    for row in &mut state.rows {
        if row["delivered"] != true || row["confirmed"] == true || protected.contains(row_id(row)) { continue; }
        if row["desistiu"] == true {
            if !current(row,min_ts) { continue; }
            let cutoff = row["desistiu_ts"].as_f64().unwrap_or(row["ts"].as_f64().unwrap_or(0.0)+60.0);
            if rows_before.iter().any(|other|row_id(other) != row_id(row) && other["text"] == row["text"] && other["ts"].as_f64().unwrap_or(0.0) > cutoff) {
                removed.insert(row_id(row).to_owned()); continue;
            }
        } else {
            if !current(row,min_ts) { row["confirmed"] = json!(true); continue; }
            if now - row["ts"].as_f64().unwrap_or(0.0) < grace { continue; }
        }
        let lines = entry_lines(row);
        if lines.iter().any(|l|pending.contains(l)) { continue; }
        let mut consumed = BTreeSet::new();
        for line in lines {
            if available.contains(&line) { consumed.insert(line); }
            else if line.chars().count() >= 8 {
                if let Some(echo) = available.iter().filter(|e|e.starts_with(&line) && !reserved.contains(*e)
                    && owners.get(*e) == Some(&line)).min_by_key(|e|e.chars().count()) { consumed.insert(echo.clone()); }
            }
        }
        if !consumed.is_empty() || (row["desistiu"] != true && row["text"].as_str().unwrap_or("").trim().is_empty()) {
            for echo in consumed { available.remove(&echo); }
            row["confirmed"] = json!(true); row.as_object_mut().unwrap().remove("desistiu");
        } else if row["desistiu"] == true || confirm_only || row["steered"] == true { continue; }
        else if row["attempts"].as_u64().unwrap_or(0) >= max_attempts {
            row["desistiu"] = json!(true); row["desistiu_ts"] = json!(now);
        } else {
            row["delivered"] = json!(false);
            row["attempts"] = json!(row["attempts"].as_u64().unwrap_or(0)+1);
            requeued.push(row.clone());
        }
    }
    state.rows.retain(|r|!removed.contains(row_id(r)));
    json!(requeued)
}

static TEMP_ID: AtomicU64 = AtomicU64::new(0);
fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(||invalid("arquivo sem diretório"))?;
    let tick = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_err(|_|invalid("relógio do arquivo temporário inválido"))?.as_nanos();
    let temp = parent.join(format!(".queue-{}-{tick}-{}.tmp",std::process::id(),TEMP_ID.fetch_add(1,Ordering::Relaxed)));
    let mut created = false;
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
        let mut file = options.open(&temp)?;
        created = true;
        file.write_all(bytes)?; file.sync_all()?; drop(file);
        #[cfg(windows)]
        for wait in [20,40,60,80,100,120] {
            match std::fs::rename(&temp,path) {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => std::thread::sleep(std::time::Duration::from_millis(wait)),
                Err(error) => return Err(error),
            }
        }
        std::fs::rename(&temp,path)?;
        #[cfg(unix)] { File::open(parent)?.sync_all()?; }
        Ok(())
    })();
    if created {
        match std::fs::remove_file(&temp) {
            Err(error) if error.kind() != io::ErrorKind::NotFound && result.is_ok() => return Err(error),
            _ => {}
        }
    }
    result
}

enum QueueMessage {
    Exec { generation:u64, call_id:String, clock:ClockSample, action:Action, reply:oneshot::Sender<io::Result<Value>> },
    Stop,
    Snapshot(oneshot::Sender<io::Result<State>>),
}

pub struct QueueActor {
    sender: mpsc::Sender<QueueMessage>,
    task: tokio::task::JoinHandle<()>,
    initial: State,
    lease: Arc<File>,
}

impl QueueActor {
    pub fn start(store: Store, lease: Arc<File>) -> Self {
        let initial = store.state().clone();
        let actor_lease = lease.clone();
        let (sender,mut receiver) = mpsc::channel(32);
        let task = tokio::spawn(async move {
            let mut store = store;
            while let Some(message) = receiver.recv().await {
                match message {
                    QueueMessage::Stop => break,
                    QueueMessage::Snapshot(reply) => {
                        let result = if store.fenced { Err(invalid("estado bloqueado após falha de persistência")) } else { Ok(store.state.clone()) };
                        let _ = reply.send(result);
                    }
                    QueueMessage::Exec { generation,call_id,clock,action,reply } => {
                        let lease = lease.clone();
                        let job = tokio::task::spawn_blocking(move || {
                            let _lease = lease;
                            let result = store.exec(generation,&call_id,clock,action);
                            (store,result)
                        }).await;
                        match job {
                            Ok((next,result)) => { store = next; let _ = reply.send(result); }
                            Err(_) => { let _ = reply.send(Err(invalid("persistência da fila interrompida"))); break; }
                        }
                    }
                }
            }
        });
        Self { sender,task,initial,lease:actor_lease }
    }

    pub fn initial_state(&self) -> &State { &self.initial }
    pub fn lease(&self) -> Arc<File> { self.lease.clone() }

    pub async fn exec(&self, generation:u64, call_id:&str, clock:ClockSample, action:Action) -> io::Result<Value> {
        let (reply,response) = oneshot::channel();
        self.sender.send(QueueMessage::Exec { generation,call_id:call_id.into(),clock,action,reply }).await
            .map_err(|_|invalid("fila encerrada"))?;
        response.await.map_err(|_|invalid("fila encerrada sem recibo"))?
    }

    pub async fn snapshot(&self) -> io::Result<State> {
        let (reply,response) = oneshot::channel();
        self.sender.send(QueueMessage::Snapshot(reply)).await.map_err(|_|invalid("fila encerrada"))?;
        response.await.map_err(|_|invalid("fila encerrada sem estado"))?
    }

    pub async fn shutdown(self) -> io::Result<()> {
        self.sender.send(QueueMessage::Stop).await.map_err(|_|invalid("fila já encerrada"))?;
        self.task.await.map_err(|_|invalid("persistência da fila interrompida"))
    }
}
