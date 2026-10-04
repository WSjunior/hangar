//! Custo e uso do Codex, na mesma passada retomável pelo rollout.

use super::accumulator::{Accumulator, Sum, ordered_pairs, python_string, skill_path, text, truthy};
use super::areas::{AreaEntries, AreaHeader, AreaMap, ToolReg, Unit, candidates};
use super::index::{Fold, Index, IndexError};
use super::pricing::canonizar_provedor;
use super::py::{LocalTs, char_len, parse_obj, py_int};
use super::rows::{FoldOutput, UsageRow, UsoLinha};
use super::uso_rules::{comando_bash, python_space};
use indexmap::{IndexMap, IndexSet};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::Path;
use std::sync::OnceLock;

pub const VERSION: &str = "codex:1:3";
const TOKEN_FIELDS: [&str; 4] = ["input_tokens", "cached_input_tokens", "cache_write_input_tokens", "output_tokens"];
type Turn = Value;

#[derive(Clone, Serialize, Deserialize)]
struct Responses {
    session_id: String,
    cwd: String,
    provider: String,
    model: String,
    turn: Turn,
    start: Option<LocalTs>,
    subagent: bool,
    metadata_read: bool,
    inherited: bool,
    previous: [i64; 4],
    #[serde(with = "ordered_pairs")]
    responses: IndexMap<Turn, Vec<UsageRow>>,
    #[serde(with = "ordered_pairs")]
    legacy: IndexMap<Turn, Vec<UsageRow>>,
    seen: IndexSet<Value>,
}

impl Responses {
    fn new(session_id: String) -> Self {
        Self { session_id, cwd: String::new(), provider: String::new(), model: String::new(),
            turn: Value::String(String::new()), start: None, subagent: false, metadata_read: false,
            inherited: false, previous: [0; 4], responses: IndexMap::new(), legacy: IndexMap::new(), seen: IndexSet::new() }
    }

    fn record(&mut self, d: &Map<String, Value>) {
        let Some(p) = d.get("payload").and_then(Value::as_object) else { return };
        let kind = text(d, "type");
        if kind == Some("session_meta") {
            let identity = nonempty(p, "id").or_else(|| nonempty(p, "session_id"));
            if self.metadata_read {
                if let Some(id) = identity { self.inherited = id != &Value::String(self.session_id.clone()); }
            } else {
                self.metadata_read = true;
                if let Some(id) = identity { self.session_id = required_string(id); }
                self.cwd = string_or(p, "cwd", "");
                self.provider = string_or(p, "model_provider", "");
                self.start = timestamp(d);
                self.subagent = is_subagent(p.get("source"));
            }
        }
        if kind == Some("turn_context") {
            if self.inherited && self.start.zip(timestamp(d)).is_some_and(|(start, ts)| ts >= start) { self.inherited = false; }
            if let Some(turn) = nonempty(p, "turn_id") { self.turn = turn.clone(); }
            if let Some(model) = text(p, "model") { self.model = model.into(); }
        }
        let Some(ts) = timestamp(d).or(self.start) else { return };
        let usage;
        let destination;
        if kind == Some("token_usage_record") {
            let thread = p.get("thread_id");
            let turn = nonempty(p, "turn_id").unwrap_or(&self.turn).clone();
            if truthy(thread) && thread != Some(&Value::String(self.session_id.clone())) {
                self.responses.entry(turn).or_default();
                return;
            }
            if thread == Some(&Value::String(self.session_id.clone())) { self.inherited = false; }
            let Some(u) = p.get("usage").and_then(Value::as_object) else { return };
            if let Some(id) = nonempty(p, "response_id") {
                if !self.seen.insert(id.clone()) { return; }
            }
            usage = TOKEN_FIELDS.map(|key| py_int(u.get(key)).max(0));
            destination = self.responses.entry(turn).or_default();
        } else if kind == Some("event_msg") && text(p, "type") == Some("token_count") {
            let Some(total) = p.get("info").and_then(Value::as_object)
                .and_then(|info| info.get("total_token_usage")).and_then(Value::as_object) else { return };
            let current = TOKEN_FIELDS.map(|key| py_int(total.get(key)).max(0));
            if current == self.previous { return; }
            let reset = current.iter().zip(self.previous).any(|(now, before)| *now < before);
            usage = std::array::from_fn(|i| current[i] - if reset { 0 } else { self.previous[i] });
            self.previous = current;
            if self.inherited { return; }
            destination = self.legacy.entry(self.turn.clone()).or_default();
        } else { return; }
        let input = usage[0].max(0);
        let read = input.min(usage[1].max(0));
        let write = (input - read).min(usage[2].max(0));
        let provider = canonizar_provedor(&self.provider);
        destination.push(UsageRow { ts, source: "codex".into(), provider: if provider.is_empty() { "openai".into() } else { provider },
            model: if self.model.is_empty() { "?".into() } else { self.model.clone() },
            project: if self.cwd.is_empty() { "desconhecido".into() } else { self.cwd.clone() },
            session_id: self.session_id.clone(), input: input - read - write, output: usage[3].max(0),
            cache_write: write, cache_read: read, subagente: self.subagent, account_id: None,
            codex_long_context: input > 272_000, cache_write_1h: 0, fast: false, regravado: 0, regravado_1h: 0 });
    }

    fn by_turn(&self) -> IndexMap<Turn, Vec<UsageRow>> {
        let mut result = IndexMap::new();
        for turn in self.legacy.keys().chain(self.responses.keys()) {
            if result.contains_key(turn) { continue; }
            let modern = self.responses.get(turn).map(Vec::as_slice).unwrap_or(&[]);
            let mut rows = modern.to_vec();
            if self.responses.contains_key(turn) && modern.is_empty() { result.insert(turn.clone(), rows); continue; }
            let mut covered: IndexMap<(String, [i64; 4]), i64> = IndexMap::new();
            for row in modern { *covered.entry(signature(row)).or_default() += 1; }
            let mut pending = vec![];
            for row in self.legacy.get(turn).into_iter().flatten() {
                if let Some(count) = covered.get_mut(&signature(row)).filter(|n| **n > 0) { *count -= 1; }
                else { pending.push(row); }
            }
            let mut balance = [0; 4];
            for ((_, values), count) in covered { for i in 0..4 { balance[i] += values[i] * count; } }
            for row in pending {
                let mut values = signature(row).1;
                for i in 0..4 { let deduction = values[i].min(balance[i]); balance[i] -= deduction; values[i] -= deduction; }
                if values.iter().any(|v| *v != 0) {
                    let mut row = row.clone();
                    [row.input, row.cache_read, row.cache_write, row.output] = values;
                    rows.push(row);
                }
            }
            result.insert(turn.clone(), rows);
        }
        result
    }
}

fn signature(row: &UsageRow) -> (String, [i64; 4]) { (row.model.clone(), [row.input, row.cache_read, row.cache_write, row.output]) }

#[derive(Clone, Serialize, Deserialize)]
struct Usage {
    accumulator: Accumulator,
    session_id: String,
    subagent: bool,
    start: Option<LocalTs>,
    inherited: bool,
    turn: Turn,
    #[serde(with = "ordered_pairs")]
    turn_areas: IndexMap<Turn, Vec<ToolReg>>,
    #[serde(with = "ordered_pairs")]
    turn_cwd: IndexMap<Turn, String>,
    #[serde(with = "ordered_pairs")]
    scripts: IndexMap<Value, Vec<(String, Map<String, Value>)>>,
    counter: Option<Map<String, Value>>,
}

impl Default for Usage {
    fn default() -> Self {
        Self { accumulator: Accumulator::default(), session_id: String::new(), subagent: false, start: None,
            inherited: false, turn: Value::String(String::new()), turn_areas: IndexMap::new(), turn_cwd: IndexMap::new(),
            scripts: IndexMap::new(), counter: None }
    }
}

impl Usage {
    fn record(&mut self, d: &Map<String, Value>) {
        let Some(p) = d.get("payload").and_then(Value::as_object) else { return };
        let kind = text(d, "type");
        let when = timestamp(d);
        if let Some(ts) = when { self.accumulator.day = ts.day(); }
        if kind == Some("session_meta") {
            let identity = nonempty(p, "id").or_else(|| nonempty(p, "session_id"));
            if self.session_id.is_empty() {
                self.session_id = identity.map(required_string).unwrap_or_default();
                self.accumulator.cwd = string_or(p, "cwd", "");
                self.start = when;
                self.subagent = is_subagent(p.get("source"));
            } else if identity.is_some_and(|id| id != &Value::String(self.session_id.clone())) { self.inherited = true; }
            return;
        }
        if kind == Some("turn_context") {
            if self.inherited && self.start.zip(when).is_some_and(|(start, ts)| ts >= start) { self.inherited = false; }
            if let Some(turn) = nonempty(p, "turn_id") { self.turn = turn.clone(); }
            self.accumulator.cwd = string_or(p, "cwd", &self.accumulator.cwd);
            if let Some(model) = text(p, "model") { self.accumulator.model = model.into(); }
            self.turn_cwd.entry(self.turn.clone()).or_insert_with(|| self.accumulator.cwd.clone());
            return;
        }
        if kind == Some("compacted") { self.accumulator.clear_loads(); return; }
        if self.inherited { return; }
        if kind == Some("event_msg") && text(p, "type") == Some("token_count") {
            if let Some(info) = p.get("info").and_then(Value::as_object) {
                if let Some(total) = info.get("total_token_usage").and_then(Value::as_object).filter(|total| self.counter.as_ref() != Some(total)) {
                    self.counter = Some(total.clone());
                    let empty = Map::new();
                    let last = info.get("last_token_usage").and_then(Value::as_object).unwrap_or(&empty);
                    let read = py_int(last.get("cached_input_tokens"));
                    let response = [("input_tokens".into(), Value::from((py_int(last.get("input_tokens")) - read).max(0))),
                        ("cache_read_input_tokens".into(), Value::from(read))].into_iter().collect();
                    self.accumulator.resposta_nova(&response);
                }
            }
            return;
        }
        if kind != Some("response_item") { return; }
        let call_id = nonempty(p, "call_id").cloned().unwrap_or(Value::String(String::new()));
        if text(p, "type") == Some("custom_tool_call") && text(p, "name") == Some("exec") {
            let input = nonempty(p, "input").map(python_string).unwrap_or_default();
            let calls = parse_calls(&input);
            for (name, info) in &calls { self.tool(name, info); }
            self.scripts.insert(call_id, calls);
        } else if text(p, "type") == Some("custom_tool_call_output") {
            let calls = self.scripts.shift_remove(&call_id).unwrap_or_default();
            self.output(&calls, output_chars(p.get("output")));
        } else if text(p, "type") == Some("function_call") {
            if let Some(name) = text(p, "name") {
                self.accumulator.somar("tool", name, Sum { calls: 1, ..Sum::default() });
                if name == "spawn_agent" {
                    let args = text(p, "arguments").and_then(|args| parse_obj(args.as_bytes()));
                    let agent = args.as_ref().and_then(|args| nonempty(args, "agent_type"))
                        .map(python_string).unwrap_or("spawn_agent".into());
                    self.accumulator.somar("agente", &agent, Sum { calls: 1, origin: "sozinho", ..Sum::default() });
                }
            }
        }
    }

    fn tool(&mut self, name: &str, info: &Map<String, Value>) {
        self.accumulator.somar("tool", name, Sum { calls: 1, ..Sum::default() });
        let cwd = &self.accumulator.cwd;
        let reg = match name {
            "exec_command" => text(info, "cmd").filter(|cmd| !cmd.is_empty()).map(|cmd| {
                self.accumulator.somar("bash", &comando_bash(cmd), Sum { calls: 1, ..Sum::default() });
                ToolReg::C { rules_cwd: self.accumulator.cwd.clone(), cwd: string_or(info, "workdir", &self.accumulator.cwd), candidates: candidates(cmd) }
            }),
            "apply_patch" => Some(ToolReg::P { rules_cwd: cwd.clone(), cwd: cwd.clone(),
                paths: info.get("arquivos").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(String::from).collect() }),
            "view_image" => { if text(info, "path").is_some_and(|p| !p.is_empty()) {
                self.accumulator.somar("imagem", "lida:view_image", Sum { calls: 1, ..Sum::default() });
            } None },
            _ => None,
        };
        let regs = self.turn_areas.entry(self.turn.clone()).or_default();
        if let Some(reg) = reg { regs.push(reg); }
    }

    fn output(&mut self, calls: &[(String, Map<String, Value>)], chars: i64) {
        if calls.is_empty() { return; }
        let share = chars / calls.len() as i64;
        for (name, info) in calls {
            let cmd = text(info, "cmd").unwrap_or("");
            if self.accumulator.ler_arquivo_de_skill(skill_path(cmd).as_deref(), share) { continue; }
            self.accumulator.somar("tool", name, Sum { chars: share, ..Sum::default() });
            if !cmd.is_empty() { self.accumulator.somar("bash", &comando_bash(cmd), Sum { chars: share, ..Sum::default() }); }
        }
    }

    fn entries(&self, turns: &IndexMap<Turn, Vec<UsageRow>>) -> AreaEntries {
        let mut cwd = self.accumulator.cwd.clone();
        let turns = turns.iter().map(|(turn, responses)| {
            if let Some(known) = self.turn_cwd.get(turn) { cwd = known.clone(); }
            (self.turn_areas.get(turn).cloned().unwrap_or_default(), responses.iter().map(|r| Unit {
                dia: r.ts.day(), cwd: cwd.clone(), model: r.model.clone(), fast: false,
                values: [r.input, r.output, r.cache_write, r.cache_read, 0] }).collect())
        }).collect();
        AreaEntries { header: AreaHeader { fonte: Some("codex".into()), session_id: Some(self.session_id.clone()), subagente: Some(self.subagent) }, turns }
    }

    fn lines(&self) -> Vec<UsoLinha> {
        let mut rows = self.accumulator.lines_without_area();
        for row in &mut rows { row.fonte = "codex".into(); row.session_id = self.session_id.clone(); row.subagente = self.subagent; }
        rows
    }
}

fn nonempty<'a>(obj: &'a Map<String, Value>, key: &str) -> Option<&'a Value> { obj.get(key).filter(|v| truthy(Some(v))) }
fn required_string(value: &Value) -> String { value.as_str().expect("campo textual do rollout").into() }
fn string_or(obj: &Map<String, Value>, key: &str, default: &str) -> String { nonempty(obj, key).map(required_string).unwrap_or_else(|| default.into()) }
fn timestamp(d: &Map<String, Value>) -> Option<LocalTs> { text(d, "timestamp").and_then(LocalTs::from_iso) }
fn is_subagent(source: Option<&Value>) -> bool { source.is_some_and(|s| s.as_object().is_some_and(|o| o.contains_key("subagent")) || s.as_str() == Some("subagent")) }

fn parse_literal(s: &str) -> String {
    if s.starts_with('"') {
        if let Some(value) = parse_obj(format!("{{\"literal\":{s}}}").as_bytes())
            .and_then(|obj| obj.get("literal").and_then(Value::as_str).map(String::from)) { return value; }
    }
    s[1..s.len() - 1].replace("\\'", "'").replace("\\`", "`").replace("\\\"", "\"").replace("\\\\", "\\")
}

fn argument(fragment: &str, key: &str) -> String {
    static CMD: OnceLock<Regex> = OnceLock::new();
    static WORKDIR: OnceLock<Regex> = OnceLock::new();
    static PATH: OnceLock<Regex> = OnceLock::new();
    let slot = match key { "cmd" => &CMD, "workdir" => &WORKDIR, _ => &PATH };
    let regex = slot.get_or_init(|| Regex::new(&format!(r#"(?:^|[^\p{{L}}\p{{N}}_]){key}[\s\x1c-\x1f]*:[\s\x1c-\x1f]*("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|`(?:[^`\\]|\\.)*`)"#)).unwrap());
    regex.captures(fragment).map(|c| parse_literal(&c[1])).unwrap_or_default()
}

fn parse_calls(js: &str) -> Vec<(String, Map<String, Value>)> {
    static CALL: OnceLock<Regex> = OnceLock::new();
    static PATCH: OnceLock<Regex> = OnceLock::new();
    let calls = CALL.get_or_init(|| Regex::new(r"tools\.([\p{L}\p{N}_]+)\(").unwrap()).captures_iter(js).collect::<Vec<_>>();
    let patch = PATCH.get_or_init(|| Regex::new(r#"\*\*\* (?:Update|Add|Delete) File: ([^\n\\\"]+)"#).unwrap());
    calls.iter().enumerate().map(|(i, capture)| {
        let mark = capture.get(0).unwrap();
        let end = calls.get(i + 1).map(|c| c.get(0).unwrap().start()).unwrap_or_else(|| {
            js[mark.end()..].char_indices().nth(4000).map_or(js.len(), |(offset, _)| mark.end() + offset)
        });
        let fragment = &js[mark.end()..end];
        let name = &capture[1];
        let info = match name {
            "exec_command" => [("cmd".into(), Value::String(argument(fragment, "cmd"))), ("workdir".into(), Value::String(argument(fragment, "workdir")))].into_iter().collect(),
            "apply_patch" => [("arquivos".into(), Value::Array(patch.captures_iter(fragment).map(|c| Value::String(c[1].trim_matches(python_space).into())).collect()))].into_iter().collect(),
            "view_image" => [("path".into(), Value::String(argument(fragment, "path")))].into_iter().collect(),
            _ => Map::new(),
        };
        (name.into(), info)
    }).collect()
}

fn output_chars(output: Option<&Value>) -> i64 {
    match output {
        Some(Value::String(s)) => char_len(s) as i64,
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_object).map(|obj| nonempty(obj, "text").map_or(0, |v| match v {
            Value::String(s) => char_len(s), Value::Array(a) => a.len(), Value::Object(o) => o.len(), _ => panic!("conteúdo sem tamanho")
        }) as i64).sum(),
        _ => 0,
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CodexFold { responses: Responses, usage: Usage }

pub fn new_fold(path: &Path) -> CodexFold {
    CodexFold { responses: Responses::new(path.file_stem().unwrap_or_default().to_string_lossy().into()), usage: Usage::default() }
}

#[derive(Deserialize)]
struct LinePeek<'a> {
    #[serde(rename = "type", borrow)]
    kind: Option<&'a str>,
    #[serde(borrow)]
    timestamp: Option<&'a str>,
    #[serde(borrow)]
    payload: Option<PayloadKind<'a>>,
}

/// O `type` do payload, exigindo objeto como o `as_object` dos leitores.
struct PayloadKind<'a>(Option<&'a str>);

impl<'de: 'a, 'a> Deserialize<'de> for PayloadKind<'a> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor<'a>(std::marker::PhantomData<&'a ()>);
        impl<'de: 'a, 'a> serde::de::Visitor<'de> for Visitor<'a> {
            type Value = PayloadKind<'a>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str("objeto") }
            fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut kind = None;
                let mut seen = false;
                while let Some(key) = map.next_key::<&'de str>()? {
                    if key == "type" {
                        // Chave repetida: o Map do serde_json fica com a última, então o caminho completo decide.
                        if seen { return Err(serde::de::Error::duplicate_field("type")); }
                        seen = true;
                        kind = map.next_value::<Option<&'de str>>()?;
                    } else { map.next_value::<serde::de::IgnoredAny>()?; }
                }
                Ok(PayloadKind(kind))
            }
        }
        deserializer.deserialize_map(Visitor(std::marker::PhantomData))
    }
}

/// `Some` quando a linha não alimenta nenhum dos dois leitores; traz o dia que o uso ainda registra.
/// `None` manda a linha para o caminho completo, inclusive em qualquer dúvida de formato.
fn skippable(raw: &[u8]) -> Option<Option<LocalTs>> {
    let is_space = |b: &u8| b.is_ascii_whitespace() || *b == b'\x0b';
    let start = raw.iter().position(|b| !is_space(b))?;
    let end = raw.iter().rposition(|b| !is_space(b)).map_or(start, |i| i + 1);
    let text = std::str::from_utf8(&raw[start..end]).ok()?;
    // O derive de struct também aceita arrays; só objeto vira linha.
    if !text.starts_with('{') { return None; }
    let peek = serde_json::from_str::<LinePeek>(text).ok()?;
    let inner = peek.payload.as_ref().and_then(|p| p.0);
    let consumed = match peek.kind {
        Some("session_meta" | "turn_context" | "token_usage_record" | "compacted") => true,
        Some("event_msg") => inner == Some("token_count"),
        Some("response_item") => matches!(inner, Some("custom_tool_call" | "custom_tool_call_output" | "function_call")),
        _ => false,
    };
    if consumed { return None; }
    Some(peek.payload.and(peek.timestamp).and_then(LocalTs::from_iso))
}

impl Fold for CodexFold {
    fn line(&mut self, raw: &[u8]) {
        if let Some(when) = skippable(raw) {
            if let Some(ts) = when { self.usage.accumulator.day = ts.day(); }
            return;
        }
        if let Some(d) = parse_obj(raw) { self.responses.record(&d); self.usage.record(&d); }
    }

    fn close(&mut self) -> FoldOutput {
        let turns = self.responses.by_turn();
        let mut grouped: IndexMap<(String, String, bool), UsageRow> = IndexMap::new();
        for row in turns.values().flatten() {
            let key = (row.ts.day(), row.model.clone(), row.codex_long_context);
            if let Some(before) = grouped.get_mut(&key) {
                before.input += row.input; before.output += row.output; before.cache_read += row.cache_read; before.cache_write += row.cache_write;
            } else { grouped.insert(key, row.clone()); }
        }
        let mut costs = grouped.into_values().collect::<Vec<_>>();
        costs.sort_by(|a, b| (a.ts, &a.model).cmp(&(b.ts, &b.model)));
        FoldOutput { costs, usage: self.usage.lines(), areas: Some(self.usage.entries(&turns)) }
    }
}

pub fn session_rows(ix: &Index, rollout: &Path, areas: &AreaMap) -> Option<Vec<UsageRow>> {
    try_session_rows(ix, rollout, areas).ok()
}

pub fn try_session_rows(ix: &Index, rollout: &Path, areas: &AreaMap) -> Result<Vec<UsageRow>, IndexError> {
    let id = ix.try_sync_file(rollout, &new_fold, VERSION, "codex:avulso", areas.signature(), &|entries| areas.area_lines(entries))?;
    ix.read_costs(None, None, Some(id))
}
