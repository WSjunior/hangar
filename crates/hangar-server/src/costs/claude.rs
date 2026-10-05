//! Custo e uso do Claude, na mesma passada retomável pelo transcript.

use super::accumulator::{Accumulator, ResponseKey, identity, ordered_pairs, text};
use super::index::Fold;
use super::pricing::IGNORADOS;
use super::py::{LocalTs, parse_obj, py_int};
use super::rows::{FoldOutput, UsageRow};
use super::uso_rules::python_space;
use indexmap::{IndexMap, IndexSet};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::Path;
use std::sync::OnceLock;

pub const VERSION: &str = "claude:12";

#[derive(Clone, Serialize, Deserialize)]
pub struct ClaudeFold {
    session_id: String,
    subagent: bool,
    usage_subagent: bool,
    number: usize,
    #[serde(with = "ordered_pairs")]
    responses: IndexMap<ResponseKey, UsageRow>,
    after_compaction: IndexSet<ResponseKey>,
    compacted: bool,
    accumulator: Accumulator,
}

impl ClaudeFold {
    pub fn new(session_id: String, subagent: bool, usage_subagent: bool) -> Self {
        Self { session_id, subagent, usage_subagent, number: 0, responses: IndexMap::new(),
            after_compaction: IndexSet::new(), compacted: false, accumulator: Accumulator::default() }
    }

    fn costs(&self) -> Vec<UsageRow> {
        let mut groups: IndexMap<(String, String, String, bool), UsageRow> = IndexMap::new();
        let mut previous: Option<i64> = None;
        for (key, row) in &self.responses {
            let mut row = row.clone();
            if let Some(context) = previous {
                if !self.after_compaction.contains(key) && row.cache_write != 0 {
                    let lost = row.cache_write.min((context - row.cache_read).max(0));
                    if i128::from(lost) * 2 >= i128::from(context) {
                        row.regravado = lost;
                        let numerator = i128::from(row.cache_write_1h) * i128::from(lost);
                        let denominator = i128::from(row.cache_write);
                        let quotient = numerator / denominator;
                        let remainder = numerator % denominator;
                        row.regravado_1h = (quotient - i128::from(remainder != 0 && (numerator < 0) != (denominator < 0))) as i64;
                    }
                }
            }
            previous = Some(row.input + row.cache_write + row.cache_read);
            let group = (row.ts.day(), row.model.clone(), row.project.clone(), row.fast);
            if let Some(before) = groups.get_mut(&group) {
                before.input += row.input;
                before.output += row.output;
                before.cache_write += row.cache_write;
                before.cache_read += row.cache_read;
                before.cache_write_1h += row.cache_write_1h;
                before.regravado += row.regravado;
                before.regravado_1h += row.regravado_1h;
            } else { groups.insert(group, row); }
        }
        let mut rows = groups.into_values().collect::<Vec<_>>();
        rows.sort_by(|a, b| (a.ts, &a.model, &a.project).cmp(&(b.ts, &b.model, &b.project)));
        rows
    }
}

impl Fold for ClaudeFold {
    fn line(&mut self, raw: &[u8]) {
        let number = self.number;
        self.number += 1;
        // Uma só varredura vetorizada no lugar de uma comparação por posição para cada termo.
        static RELEVANT: OnceLock<regex::bytes::Regex> = OnceLock::new();
        let relevant = RELEVANT.get_or_init(|| regex::bytes::Regex::new(r#""(?:usage|user|attachment|compact_boundary)""#).unwrap());
        if !relevant.is_match(raw) { return; }
        let Some(d) = decode(raw) else { return };
        let when = text(&d, "timestamp").and_then(LocalTs::from_iso);
        let msg = d.get("message").and_then(Value::as_object);
        let model = msg.and_then(|m| text(m, "model"));
        if model.is_some_and(|m| IGNORADOS.contains(&m.trim_matches(python_space))) { return; }
        self.accumulator.consume(&d, &when.map_or(String::new(), |ts| ts.day()));
        if text(&d, "subtype") == Some("compact_boundary") { self.compacted = true; }
        if text(&d, "type") != Some("assistant") { return; }
        let Some(msg) = msg else { return };
        let Some(usage) = msg.get("usage").and_then(Value::as_object) else { return };
        let Some(ts) = when else { return };
        let key = identity(&d, msg).unwrap_or(ResponseKey::Line(number));
        if self.compacted && !self.responses.contains_key(&key) {
            self.after_compaction.insert(key.clone());
            self.compacted = false;
        }
        let write = py_int(usage.get("cache_creation_input_tokens"));
        let hour = usage.get("cache_creation").and_then(Value::as_object).map_or(0, |c| py_int(c.get("ephemeral_1h_input_tokens")));
        self.responses.insert(key, UsageRow {
            ts, source: "claude".into(), provider: String::new(), model: model.filter(|m| !m.is_empty()).unwrap_or("?").into(),
            project: text(&d, "cwd").unwrap_or("").into(), session_id: self.session_id.clone(),
            input: py_int(usage.get("input_tokens")), output: py_int(usage.get("output_tokens")), cache_write: write,
            cache_read: py_int(usage.get("cache_read_input_tokens")), cache_write_1h: hour.max(0).min(write.max(0)),
            subagente: self.subagent, account_id: None, codex_long_context: false, fast: text(usage, "speed") == Some("fast"),
            regravado: 0, regravado_1h: 0,
        });
    }

    fn close(&mut self) -> FoldOutput {
        let mut areas = self.accumulator.entries();
        let mut usage = self.accumulator.lines_without_area();
        if !self.session_id.is_empty() {
            areas.header.session_id = Some(self.session_id.clone());
            areas.header.subagente = Some(self.usage_subagent);
            for row in &mut usage { row.session_id = self.session_id.clone(); row.subagente = self.usage_subagent; }
        }
        FoldOutput { costs: self.costs(), usage, areas: Some(areas) }
    }
}

pub fn new_fold(root: &Path) -> impl Fn(&Path) -> ClaudeFold + Sync {
    let root = root.to_path_buf();
    move |path| {
        let relative = path.strip_prefix(&root).expect("transcript dentro da raiz").with_extension("");
        let id = relative.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
        ClaudeFold::new(id, path.components().any(|c| c.as_os_str() == "subagents"), relative.components().any(|c| c.as_os_str() == "subagents"))
    }
}

#[derive(Deserialize)]
struct Record {
    #[serde(rename = "type")] kind: Option<Value>,
    subtype: Option<Value>,
    timestamp: Option<Value>,
    cwd: Option<Value>,
    #[serde(rename = "requestId")] request_id: Option<Value>,
    #[serde(rename = "promptId")] prompt_id: Option<Value>,
    #[serde(rename = "isMeta")] is_meta: Option<Value>,
    message: Option<Value>,
    attachment: Option<Value>,
    rendered: Option<Value>,
    #[serde(rename = "toolUseResult")] tool_result: Option<Value>,
}

fn decode(raw: &[u8]) -> Option<Map<String, Value>> {
    // O derive de struct também aceita arrays; transcript só consome objetos.
    if raw.iter().find(|b| !b.is_ascii_whitespace()).copied() != Some(b'{') { return parse_obj(raw); }
    if let Ok(record) = serde_json::from_slice::<Record>(raw) {
        return Some([
            ("type", record.kind), ("subtype", record.subtype), ("timestamp", record.timestamp), ("cwd", record.cwd),
            ("requestId", record.request_id), ("promptId", record.prompt_id), ("isMeta", record.is_meta),
            ("message", record.message), ("attachment", record.attachment), ("rendered", record.rendered),
            ("toolUseResult", record.tool_result),
        ].into_iter().filter_map(|(k, v)| v.map(|v| (k.into(), v))).collect());
    }
    parse_obj(raw)
}
