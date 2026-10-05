//! Arquivos que as sessões Claude escrevem e a lista lê: marcador do hook
//! (`.hangar-state/<sid>.json`), registro nativo do Claude Code (`<config>/sessions/<pid>.json`),
//! pergunta aberta (`.hangar-askq/<sid>.json`) e statusline inteira (`.hangar-status/<sid>.json`).
//! Porte de `hook_state.py`, `askquestion.py` e `statusline.py`.
use std::collections::{HashMap, HashSet};
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::UNIX_EPOCH;

use serde_json::Value;

use crate::transcript::ts_of_iso;

const MARKER_DIR: &str = ".hangar-state";
const NATIVE_DIR: &str = "sessions";
const ASKQ_DIR: &str = ".hangar-askq";
const STATUS_DIR: &str = ".hangar-status";
/// Teto para sidecar esquecido de uma sessão antiga cujo stem voltou a existir.
const STATUS_MAX_AGE: f64 = 86_400.0;
/// Rabo do transcript lido para achar a resposta da pergunta.
const ANSWER_SPAN: u64 = 256 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub state: String,
    pub ts: f64,
}

#[derive(Clone, Debug)]
struct Native {
    state: &'static str,
    ts: f64,
    pid: i64,
}

/// Status do registro nativo. `shell` é a sessão parada com comando de fundo vivo: aceita mensagem.
fn native_state(status: &str) -> Option<&'static str> {
    match status {
        "idle" | "shell" => Some("idle"),
        "busy" => Some("working"),
        "waiting" => Some("awaiting_input"),
        _ => None,
    }
}

/// Um aviso por status desconhecido, não por arquivo lido.
static UNKNOWN_STATUSES: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);

/// Marcadores e registro nativo lidos do disco, por session id.
#[derive(Default)]
pub struct HookStates {
    markers: HashMap<String, Marker>,
    native: HashMap<String, Native>,
}

/// `float()` do Python sobre número ou texto numérico.
fn py_float(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        _ => None,
    }
}

fn py_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// `str()` do Python para id e status.
fn py_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Null => "None".to_owned(),
        other => other.to_string(),
    }
}

/// Booleano como o pydantic em modo lax aceita.
fn lax_bool(v: &Value) -> bool {
    match v {
        Value::Bool(_) => true,
        Value::Number(n) => matches!(n.as_f64(), Some(x) if x == 0.0 || x == 1.0),
        Value::String(s) => matches!(s.to_ascii_lowercase().as_str(),
            "0" | "1" | "true" | "false" | "t" | "f" | "yes" | "no" | "y" | "n" | "on" | "off"),
        _ => false,
    }
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn read_marker(path: &Path) -> Option<Marker> {
    let o = read_json(path)?;
    // ponytail: o Python aceita `state` de qualquer tipo; só texto é estado que a lista conhece.
    Some(Marker { state: o.get("state")?.as_str()?.to_owned(), ts: py_float(o.get("ts")?)? })
}

/// (session id, entrada), ou o campo que impediu a leitura. Status desconhecido avisa uma vez.
fn read_native(path: &Path) -> Result<(String, Native), &'static str> {
    let o = read_json(path).ok_or("json")?;
    let sid = py_str(o.get("sessionId").ok_or("sessionId")?);
    let pid = match o.get("pid").ok_or("pid")? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.trunc() as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .ok_or("pid")?;
    let status = py_str(o.get("status").ok_or("status")?);
    let ts_raw = o.get("statusUpdatedAt").filter(|v| py_truthy(v)).or_else(|| o.get("updatedAt"));
    let ts = ts_raw.and_then(py_float).ok_or("updatedAt")? / 1000.0;
    let Some(state) = native_state(&status) else {
        if UNKNOWN_STATUSES.lock().unwrap_or_else(|e| e.into_inner()).insert(status.clone()) {
            tracing::warn!(status, "registro nativo com status desconhecido; a sessão cai no marcador ou no pane");
        }
        return Err("status");
    };
    Ok((sid, Native { state, ts, pid }))
}

/// Entradas da pasta; ausente é normal, outro erro (permissão, disco) avisa: calado, toda sessão
/// da conta ficaria sem marcador para sempre.
fn entries(dir: &Path, what: &'static str) -> Vec<std::fs::DirEntry> {
    match std::fs::read_dir(dir) {
        Ok(it) => it.flatten().collect(),
        Err(e) if e.kind() == ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            tracing::warn!(what, kind = ?e.kind(), "pasta de estado ilegível");
            Vec::new()
        }
    }
}

fn is_native_name(name: &str) -> bool {
    name.strip_suffix(".json").is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

impl HookStates {
    /// Marcadores de cada `<config>/.hangar-state` e o registro nativo de cada `<config>/sessions`
    /// (resolvido: as contas são symlink para o principal, e um só basta). Arquivo ilegível fica de fora.
    pub fn load(dirs: &[PathBuf]) -> Self {
        let mut me = Self::default();
        for base in dirs {
            for e in entries(&base.join(MARKER_DIR), MARKER_DIR) {
                let path = e.path();
                if path.extension().is_none_or(|x| x != "json") {
                    continue;
                }
                if let (Some(m), Some(stem)) = (read_marker(&path), path.file_stem().and_then(|s| s.to_str())) {
                    me.markers.insert(stem.to_owned(), m);
                }
            }
        }
        let mut native_dirs: Vec<PathBuf> = Vec::new();
        for base in dirs {
            let d = base.join(NATIVE_DIR);
            let d = std::fs::canonicalize(&d).unwrap_or(d);
            if !native_dirs.contains(&d) {
                native_dirs.push(d);
            }
        }
        for d in native_dirs {
            for e in entries(&d, NATIVE_DIR) {
                let name = e.file_name();
                let Some(name) = name.to_str().filter(|n| is_native_name(n)) else { continue };
                match read_native(&e.path()) {
                    Ok((sid, n)) => {
                        me.native.insert(sid, n);
                    }
                    Err("status") => {}
                    // O arquivo pode estar no meio da escrita; formato novo do Claude aparece aqui.
                    Err(field) => tracing::debug!(file = name, field, "registro nativo ilegível; usando marcador ou pane"),
                }
            }
        }
        me
    }

    /// Registro nativo enquanto o pid dele vive (é o estado que a TUI tem); senão o marcador.
    pub fn get_state(&self, sid: Option<&str>, alive: impl Fn(i64) -> bool) -> Option<Marker> {
        let sid = sid.filter(|s| !s.is_empty())?;
        if let Some(n) = self.native.get(sid).filter(|n| alive(n.pid)) {
            return Some(Marker { state: n.state.to_owned(), ts: n.ts });
        }
        self.markers.get(sid).cloned()
    }
}

/// Pergunta pendente que o pane pode não mostrar, e as opções dela (só a primeira pergunta).
#[derive(Clone, Debug, PartialEq)]
pub struct OpenQuestion {
    pub question: String,
    pub options: Vec<String>,
}

/// O `AskQuestion` do pydantic: cada pergunta com `header` e `question` em texto e cada opção com
/// `label`; `description`/`preview` em texto e `multiSelect` booleano quando vierem.
/// Lista vazia o pydantic aceita, mas a lista do Python quebra no `questions[0]`; aqui é recusa.
fn first_question(data: &Value) -> Result<OpenQuestion, &'static str> {
    let questions = data.get("tool_input").and_then(|t| t.get("questions")).and_then(Value::as_array)
        .ok_or("questions")?;
    let opt_str = |o: &Value, k| o.get(k).is_none_or(Value::is_string);
    let mut parsed = Vec::with_capacity(questions.len());
    for q in questions {
        if !q.get("header").is_some_and(Value::is_string) {
            return Err("header");
        }
        if !q.get("multiSelect").is_none_or(lax_bool) {
            return Err("multiSelect");
        }
        let question = q.get("question").and_then(Value::as_str).ok_or("question")?;
        let options = q.get("options").and_then(Value::as_array).ok_or("options")?.iter()
            .map(|o| {
                let label = o.get("label").and_then(Value::as_str).ok_or("label")?;
                if !opt_str(o, "description") || !opt_str(o, "preview") {
                    return Err("description");
                }
                Ok(label.to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        parsed.push(OpenQuestion { question: question.to_owned(), options });
    }
    parsed.into_iter().next().ok_or("questions")
}

/// A pergunta já foi respondida depois de `since` (mtime do sidecar)? Sem como provar que não,
/// responde sim: ficar do outro lado prenderia a sessão em `awaiting_input` para sempre.
fn answered_after(session: &str, jsonl: &str, since: f64) -> bool {
    let read = || -> std::io::Result<(u64, Vec<u8>)> {
        let mut fh = std::fs::File::open(jsonl)?;
        let size = fh.seek(SeekFrom::End(0))?;
        let start = size.saturating_sub(ANSWER_SPAN);
        fh.seek(SeekFrom::Start(start))?;
        let mut buf = Vec::new();
        fh.read_to_end(&mut buf)?;
        Ok((start, buf))
    };
    let (start, buf) = match read() {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!(session, kind = ?e.kind(), "askq: transcript ilegível; trata como respondida");
            return true;
        }
    };
    let when = |obj: &Value| obj.get("timestamp").and_then(Value::as_str).and_then(ts_of_iso).unwrap_or(0.0);
    let mut ids: Vec<Value> = Vec::new();
    let mut oldest: Option<f64> = None;
    // Com o seek no meio do arquivo, a primeira linha vem cortada.
    for line in buf.split(|&b| b == b'\n').skip(usize::from(start > 0)) {
        if line.trim_ascii().is_empty() {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<Value>(&String::from_utf8_lossy(line)) else { continue };
        if !obj.is_object() {
            continue;
        }
        let t = when(&obj);
        if t != 0.0 && oldest.is_none_or(|o| t < o) {
            oldest = Some(t);
        }
        let blocks = obj.get("message").and_then(|m| m.get("content")).and_then(Value::as_array);
        for b in blocks.into_iter().flatten() {
            let kind = b.get("type").and_then(Value::as_str);
            if kind == Some("tool_use") && b.get("name").and_then(Value::as_str) == Some("AskUserQuestion") {
                ids.push(b.get("id").cloned().unwrap_or(Value::Null));
            } else if kind == Some("tool_result")
                && ids.contains(b.get("tool_use_id").unwrap_or(&Value::Null))
                && t > since
            {
                return true;
            }
        }
    }
    // A janela é contada do fim e envelhece: quando a linha mais antiga lida já é posterior ao
    // sidecar, não vi a resposta porque não cheguei lá.
    let out_of_reach = start > 0 && oldest.is_none_or(|o| o > since);
    if out_of_reach {
        tracing::debug!(session, "askq: janela do transcript não alcança a pergunta; trata como respondida");
    }
    out_of_reach
}

/// Pergunta pendente da sessão `stem` pelo sidecar do hook PreToolUse, se o transcript não a
/// respondeu depois dele. O primeiro diretório que tem sidecar válido decide.
pub fn open_question(stem: Option<&str>, dirs: &[PathBuf]) -> Option<OpenQuestion> {
    let stem = stem.filter(|s| !s.is_empty())?;
    for base in dirs {
        let f = base.join(ASKQ_DIR).join(format!("{stem}.json"));
        let read = std::fs::read(&f).and_then(|raw| Ok((raw, f.metadata()?.modified()?)));
        let (raw, mtime) = match read {
            Ok(v) => v,
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(e) => {
                // Calar faria toda sessão da conta dizer "sem pergunta" para sempre.
                tracing::warn!(session = stem, kind = ?e.kind(), "askq: sidecar ilegível");
                continue;
            }
        };
        let Ok(data) = serde_json::from_slice::<Value>(&raw) else {
            tracing::debug!(session = stem, "askq: sidecar não é JSON");
            continue;
        };
        let Some(tp) = data.get("transcript_path").and_then(Value::as_str) else { continue };
        let since = mtime.duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
        if answered_after(stem, tp, since) {
            continue;
        }
        match first_question(&data) {
            Ok(q) => return Some(q),
            Err(field) => tracing::warn!(session = stem, field, "askq: sidecar malformado"),
        }
    }
    None
}

/// Statusline inteira publicada por quem a renderiza, com o modelo e o esforço em uso.
#[derive(Clone, Debug, PartialEq)]
pub struct Published {
    pub line: String,
    pub model: Option<String>,
    pub effort: Option<String>,
}

/// Sidecar da statusline da sessão `stem`, com até 1 dia de idade em `now` (época em s).
pub fn published_status(stem: Option<&str>, dirs: &[PathBuf], now: f64) -> Option<Published> {
    let stem = stem.filter(|s| !s.is_empty())?;
    let text = |o: &Value, k| o.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
    for base in dirs {
        let f = base.join(STATUS_DIR).join(format!("{stem}.json"));
        // Ausente é o caso normal: sessão sem a statusline do Hangar.
        let Ok(raw) = std::fs::read(&f) else { continue };
        let Ok(o) = serde_json::from_slice::<Value>(&raw) else {
            // Quem publica grava por tmp+rename: JSON torto é bug de escrita, não sessão sem sidecar.
            tracing::debug!(session = stem, "statusline: sidecar ilegível");
            continue;
        };
        if !o.is_object() {
            tracing::debug!(session = stem, "statusline: sidecar não é objeto");
            continue;
        }
        let Some(line) = o.get("line").and_then(Value::as_str).filter(|l| !l.trim().is_empty()) else { continue };
        if o.get("ts").and_then(Value::as_f64).is_some_and(|ts| now - ts > STATUS_MAX_AGE) {
            continue;
        }
        return Some(Published { line: line.to_owned(), model: text(&o, "model"), effort: text(&o, "effort") });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn native_registry_wins_only_while_pid_lives() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join(".claude");
        write(&cfg.join(".hangar-state/s1.json"), r#"{"state": "working", "ts": "12.5"}"#);
        write(&cfg.join("sessions/7.json"),
              r#"{"sessionId": "s1", "pid": 7, "status": "shell", "statusUpdatedAt": 0, "updatedAt": 9000}"#);
        write(&cfg.join("sessions/8.json"), r#"{"sessionId": "s2", "pid": 8, "status": "novo", "updatedAt": 1}"#);
        write(&cfg.join("sessions/x.json"), r#"{"sessionId": "s3", "pid": 9, "status": "busy", "updatedAt": 1}"#);
        let hs = HookStates::load(std::slice::from_ref(&cfg));
        assert_eq!(hs.get_state(Some("s1"), |p| p == 7), Some(Marker { state: "idle".into(), ts: 9.0 }));
        assert_eq!(hs.get_state(Some("s1"), |_| false), Some(Marker { state: "working".into(), ts: 12.5 }));
        assert_eq!(hs.get_state(Some("s2"), |_| true), None, "status desconhecido não vale");
        assert_eq!(hs.get_state(Some("s3"), |_| true), None, "nome fora de <pid>.json não é registro");
        assert_eq!(hs.get_state(None, |_| true), None);
    }

    #[test]
    fn status_sidecar_age_and_shape() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        write(&a.join(".hangar-status/s.json"), r#"{"line": "   ", "ts": 100}"#);
        write(&b.join(".hangar-status/s.json"), r#"{"line": "L", "model": "", "effort": "high", "ts": 100}"#);
        write(&a.join(".hangar-status/n.json"), "null");
        let dirs = [a, b];
        let want = Published { line: "L".into(), model: None, effort: Some("high".into()) };
        assert_eq!(published_status(Some("s"), &dirs, 100.0 + STATUS_MAX_AGE), Some(want));
        assert_eq!(published_status(Some("s"), &dirs, 101.0 + STATUS_MAX_AGE), None);
        assert_eq!(published_status(Some("n"), &dirs, 0.0), None);
    }

    #[test]
    fn open_question_needs_unanswered_and_valid_shape() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join(".claude");
        let jsonl = tmp.path().join("t.jsonl");
        write(&jsonl, "{\"type\": \"user\"}\n");
        let q = r#"{"questions": [{"question": "Q?", "header": "h", "options": [{"label": "A"}]}]}"#;
        let sidecar = format!(r#"{{"tool_input": {q}, "transcript_path": {:?}}}"#, jsonl.to_str().unwrap());
        write(&cfg.join(".hangar-askq/s.json"), &sidecar);
        let dirs = [cfg.clone()];
        assert_eq!(open_question(Some("s"), &dirs),
                   Some(OpenQuestion { question: "Q?".into(), options: vec!["A".into()] }));
        // multiSelect 1 o pydantic aceita; lista vazia e sem header, não.
        write(&cfg.join(".hangar-askq/s.json"), &sidecar.replace(r#""header": "h""#, r#""header": "h", "multiSelect": 1"#));
        assert!(open_question(Some("s"), &dirs).is_some());
        write(&cfg.join(".hangar-askq/s.json"), &sidecar.replace(q, r#"{"questions": []}"#));
        assert_eq!(open_question(Some("s"), &dirs), None);
        write(&cfg.join(".hangar-askq/s.json"), &sidecar.replace(r#""header": "h", "#, ""));
        assert_eq!(open_question(Some("s"), &dirs), None);
        // Transcript sumido: não há como provar que segue aberta.
        write(&cfg.join(".hangar-askq/s.json"), &sidecar);
        std::fs::remove_file(&jsonl).unwrap();
        assert_eq!(open_question(Some("s"), &dirs), None);
    }
}
