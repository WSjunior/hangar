//! Acúmulo retomável de ferramentas, cargas de skills e alvos por turno.

use super::areas::{AreaEntries, AreaHeader, AreaMap, ToolReg, Unit, candidates};
use super::py::{char_len, py_int};
use super::rows::UsoLinha;
use super::uso_rules::{comando_bash, pede_agente, plugin_de, plugin_de_hook, python_space, skill_do_caminho, texto_len, tokens_de_imagem};
use indexmap::{IndexMap, IndexSet};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::OnceLock;

// JSON não aceita chaves compostas de objeto; pares conservam identidade e ordem.
pub(crate) mod ordered_pairs {
    use indexmap::IndexMap;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::hash::Hash;

    pub fn serialize<K: Serialize, V: Serialize, S: Serializer>(map: &IndexMap<K, V>, serializer: S) -> Result<S::Ok, S::Error> {
        map.iter().collect::<Vec<_>>().serialize(serializer)
    }

    pub fn deserialize<'de, K: Deserialize<'de> + Eq + Hash, V: Deserialize<'de>, D: Deserializer<'de>>(deserializer: D) -> Result<IndexMap<K, V>, D::Error> {
        Ok(Vec::<(K, V)>::deserialize(deserializer)?.into_iter().collect())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum ResponseKey {
    Ident(Value, Value),
    Line(usize),
}

pub(crate) fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

pub(crate) fn identity(d: &Map<String, Value>, msg: &Map<String, Value>) -> Option<ResponseKey> {
    truthy(msg.get("id")).then(|| ResponseKey::Ident(d.get("requestId").cloned().unwrap_or(Value::Null), msg["id"].clone()))
}

pub(crate) fn text<'a>(obj: &'a Map<String, Value>, key: &str) -> Option<&'a str> { obj.get(key).and_then(Value::as_str) }

type RowKey = [String; 8];
type Group = (String, String, String, bool);

#[derive(Default)]
pub(crate) struct Sum<'a> {
    pub plugin: &'a str,
    pub detail: &'a str,
    pub origin: &'a str,
    pub calls: i64,
    pub chars: i64,
    pub tokens: i64,
    pub usage: Option<&'a Map<String, Value>>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Accumulator {
    #[serde(with = "ordered_pairs")]
    rows: IndexMap<RowKey, UsoLinha>,
    tools: IndexMap<String, (String, Map<String, Value>)>,
    seen: IndexSet<ResponseKey>,
    pending_skill: Option<(String, String, Option<RowKey>)>,
    loads: Vec<(RowKey, i64)>,
    loaded: IndexMap<String, RowKey>,
    prompt_id: Value,
    requested_agent: bool,
    pub(crate) day: String,
    pub(crate) cwd: String,
    pub(crate) model: String,
    turns: Vec<Vec<ToolReg>>,
    #[serde(with = "ordered_pairs")]
    responses: IndexMap<ResponseKey, (usize, Group, [i64; 5])>,
}

impl Default for Accumulator {
    fn default() -> Self {
        Self { rows: IndexMap::new(), tools: IndexMap::new(), seen: IndexSet::new(), pending_skill: None,
            loads: vec![], loaded: IndexMap::new(), prompt_id: Value::Null, requested_agent: false,
            day: String::new(), cwd: String::new(), model: String::new(), turns: vec![vec![]], responses: IndexMap::new() }
    }
}

impl Accumulator {
    pub(crate) fn carregar(&mut self, name: &str, origin: &str, chars: i64, calls: i64, key: Option<RowKey>) {
        let key = match key {
            None => self.somar("skill", name, Sum { plugin: &plugin_de(name), origin, calls, chars, ..Sum::default() }),
            Some(key) => { self.rows.get_mut(&key).unwrap().ctx_chars += chars; key },
        };
        if chars > 0 { self.loads.push((key.clone(), chars)); }
        self.loaded.insert(name.into(), key);
    }

    pub(crate) fn resposta_nova(&mut self, usage: &Map<String, Value>) {
        let input = py_int(usage.get("input_tokens"));
        let write = py_int(usage.get("cache_creation_input_tokens"));
        let read = py_int(usage.get("cache_read_input_tokens"));
        let total = input + write + read;
        let weight = if total != 0 { (input as f64 + 1.25 * write as f64 + 0.1 * read as f64) / total as f64 } else { 1.0 };
        let mut seen = IndexSet::new();
        for (key, chars) in &self.loads {
            let row = self.rows.get_mut(key).unwrap();
            row.ocupados += chars;
            row.ocupados_eq += (*chars as f64 * weight).round_ties_even() as i64;
            row.respostas += i64::from(seen.insert(key));
        }
    }

    pub(crate) fn somar(&mut self, kind: &str, name: &str, sum: Sum<'_>) -> RowKey {
        let key = [self.day.clone(), self.cwd.clone(), self.model.clone(), kind.into(), name.into(), sum.plugin.into(), sum.detail.into(), sum.origin.into()];
        let row = self.rows.entry(key.clone()).or_insert_with(|| UsoLinha {
            dia: self.day.clone(), cwd: self.cwd.clone(), model: self.model.clone(), tipo: kind.into(), nome: name.into(),
            plugin: sum.plugin.into(), detalhe: sum.detail.into(), origem: sum.origin.into(), fonte: "claude".into(), ..UsoLinha::default()
        });
        row.chamadas += sum.calls;
        row.ctx_chars += sum.chars;
        row.tokens_est += sum.tokens;
        if let Some(u) = sum.usage {
            let write = py_int(u.get("cache_creation_input_tokens"));
            let hour = u.get("cache_creation").and_then(Value::as_object).map_or(0, |c| py_int(c.get("ephemeral_1h_input_tokens")));
            row.input += py_int(u.get("input_tokens"));
            row.output += py_int(u.get("output_tokens"));
            row.cache_write += write;
            row.cache_read += py_int(u.get("cache_read_input_tokens"));
            row.cache_write_1h += hour.max(0).min(write.max(0));
            row.fast |= text(u, "speed") == Some("fast");
        }
        key
    }

    pub(crate) fn clear_loads(&mut self) { self.loads.clear(); self.loaded.clear(); }

    pub fn line(&mut self, d: &Map<String, Value>, day: &str, _areas: &AreaMap) { self.consume(d, day); }

    pub(crate) fn consume(&mut self, d: &Map<String, Value>, day: &str) {
        if !day.is_empty() { self.day = day.into(); }
        if let Some(cwd) = text(d, "cwd") { self.cwd = cwd.into(); }
        if text(d, "subtype") == Some("compact_boundary") { self.clear_loads(); }
        else {
            match text(d, "type") {
                Some("assistant") => self.assistant(d),
                Some("user") => self.user(d),
                Some("attachment") => self.attachment(d),
                _ => {},
            }
        }
    }

    fn assistant(&mut self, d: &Map<String, Value>) {
        let Some(msg) = d.get("message").and_then(Value::as_object) else { return };
        if let Some(model) = text(msg, "model") { self.model = model.into(); }
        let ident = identity(d, msg);
        if let Some(usage) = msg.get("usage").and_then(Value::as_object).filter(|u| !u.is_empty()) {
            self.add_turn_usage(usage, ident.clone());
            if ident.as_ref().is_none_or(|i| !self.seen.contains(i)) { self.resposta_nova(usage); }
        }
        if let Some(ident) = ident { self.seen.insert(ident); }
        for block in msg.get("content").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_object) {
            if text(block, "type") != Some("tool_use") { continue; }
            let Some(name) = text(block, "name") else { continue };
            let empty = Map::new();
            let input = block.get("input").and_then(Value::as_object).unwrap_or(&empty);
            if let Some(id) = text(block, "id") {
                self.tools.insert(id.into(), (name.into(), input.iter().filter(|(k, _)| matches!(k.as_str(), "file_path" | "command" | "subagent_type"))
                    .map(|(k, v)| (k.clone(), v.clone())).collect()));
            }
            self.add_turn_tool(name, input);
            if name == "Skill" {
                if let Some(skill) = text(input, "skill") {
                    let key = self.somar("skill", skill, Sum { plugin: &plugin_de(skill), origin: "modelo", calls: 1, ..Sum::default() });
                    self.pending_skill = Some((skill.into(), "modelo".into(), Some(key)));
                }
            } else if name == "Agent" {
                self.somar("agente", &agent_type(input), Sum { calls: 1, origin: if self.requested_agent { "pedido" } else { "sozinho" }, ..Sum::default() });
            } else if name == "Bash" {
                if let Some(cmd) = text(input, "command") { self.somar("bash", &comando_bash(cmd), Sum { calls: 1, ..Sum::default() }); }
            } else if name.starts_with("mcp__") {
                self.somar("mcp", server(name), Sum { detail: name, calls: 1, ..Sum::default() });
            }
            self.somar("tool", name, Sum { calls: 1, ..Sum::default() });
        }
    }

    fn user(&mut self, d: &Map<String, Value>) {
        let content = d.get("message").and_then(Value::as_object).and_then(|m| m.get("content"));
        if let Some(id) = d.get("promptId").filter(|id| truthy(Some(id)) && **id != self.prompt_id) {
            self.turns.push(vec![]);
            self.prompt_id = id.clone();
            self.pending_skill = None;
        }
        if let Some(Value::String(content)) = content {
            self.requested_agent = pede_agente(content);
            self.command(content);
            return;
        }
        let Some(Value::Array(blocks)) = content else { return };
        if !truthy(d.get("isMeta")) {
            let prompt = blocks.iter().filter_map(Value::as_object).filter(|b| text(b, "type") == Some("text"))
                .filter_map(|b| text(b, "text")).collect::<Vec<_>>().join(" ");
            if !prompt.is_empty() { self.requested_agent = pede_agente(&prompt); }
        }
        for block in blocks.iter().filter_map(Value::as_object) {
            if text(block, "type") == Some("image") {
                self.somar("imagem", "enviada", Sum { calls: 1, tokens: tokens_de_imagem(block).1, ..Sum::default() });
            }
            if text(block, "type") == Some("tool_result") {
                let (name, input) = text(block, "tool_use_id").and_then(|id| self.tools.get(id)).cloned().unwrap_or(("?".into(), Map::new()));
                let chars = texto_len(block.get("content"));
                let tool_chars = if self.skill_read(&name, &input, chars) { 0 } else { chars };
                self.somar("tool", &name, Sum { chars: tool_chars, ..Sum::default() });
                for image in block.get("content").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_object).filter(|b| text(b, "type") == Some("image")) {
                    self.somar("imagem", &format!("lida:{name}"), Sum { calls: 1, tokens: tokens_de_imagem(image).1, ..Sum::default() });
                }
                if name == "Bash" {
                    if let Some(cmd) = text(&input, "command") { self.somar("bash", &comando_bash(cmd), Sum { chars: tool_chars, ..Sum::default() }); }
                } else if name.starts_with("mcp__") {
                    self.somar("mcp", server(&name), Sum { detail: &name, chars, ..Sum::default() });
                } else if name == "Skill" && chars >= 50 {
                    if let Some((skill, origin, Some(key))) = self.pending_skill.clone() { self.carregar(&skill, &origin, chars, 1, Some(key)); }
                } else if name == "Agent" {
                    if let Some(id) = d.get("toolUseResult").and_then(Value::as_object).and_then(|r| text(r, "agentId")) {
                        self.somar("agente", &agent_type(&input), Sum { detail: id, ..Sum::default() });
                    }
                }
            } else if text(block, "type") == Some("text") {
                if let Some(body) = text(block, "text") {
                    if body.contains("<command-name>") { self.command(body); }
                    else if truthy(d.get("isMeta")) {
                        if let Some((skill, origin, key)) = self.pending_skill.clone() {
                            let key = key.unwrap_or_else(|| self.somar("skill", &skill, Sum { plugin: &plugin_de(&skill), origin: &origin, calls: 1, ..Sum::default() }));
                            self.pending_skill = Some((skill.clone(), origin.clone(), Some(key.clone())));
                            self.carregar(&skill, &origin, char_len(body) as i64, 1, Some(key));
                        }
                    }
                }
            }
        }
    }

    fn skill_read(&mut self, name: &str, input: &Map<String, Value>, chars: i64) -> bool {
        let path = if name == "Read" { text(input, "file_path").map(String::from) }
        else if name == "Bash" { text(input, "command").and_then(skill_path) }
        else { None };
        self.ler_arquivo_de_skill(path.as_deref(), chars)
    }

    pub(crate) fn ler_arquivo_de_skill(&mut self, path: Option<&str>, chars: i64) -> bool {
        let Some((skill, main)) = path.and_then(skill_do_caminho).filter(|_| chars >= 50) else { return false };
        if main { self.carregar(&skill, "leitura", chars, 1, None); true }
        else if let Some(key) = self.loaded.get(&skill).cloned() { self.carregar(&skill, "", chars, 1, Some(key)); true }
        else { false }
    }

    fn command(&mut self, body: &str) {
        let Some((_, rest)) = body.split_once("<command-name>") else { return };
        let Some((name, _)) = rest.split_once("</command-name>") else { return };
        let name = name.trim_matches(python_space);
        if !name.is_empty() { self.pending_skill = Some((name.trim_start_matches('/').into(), "voce".into(), None)); }
    }

    fn attachment(&mut self, d: &Map<String, Value>) {
        let Some(a) = d.get("attachment").and_then(Value::as_object) else { return };
        let Some(kind) = text(a, "type") else { return };
        let chars = if let Some(rendered) = d.get("rendered").filter(|v| !v.is_null()) { texto_len(Some(rendered)) }
        else { let content = texto_len(a.get("content")); if content != 0 { content } else { texto_len(a.get("text")) } };
        if kind == "invoked_skills" {
            if let Some(skills) = a.get("skills").and_then(Value::as_array) {
                for skill in skills.iter().filter_map(Value::as_object) {
                    if let Some(name) = text(skill, "name") { self.carregar(name, "compactacao", texto_len(skill.get("content")), 0, None); }
                }
                return;
            }
        }
        let mut plugin = String::new();
        let name = if kind.starts_with("hook") {
            if chars == 0 { return; }
            let raw = a.get("content");
            let pieces: Vec<&Value> = match raw { Some(Value::Array(blocks)) => blocks.iter().collect(), Some(v) => vec![v], None => vec![] };
            let start: String = pieces.iter().filter_map(|v| v.as_str().or_else(|| v.as_object().and_then(|b| text(b, "text"))))
                .collect::<String>().chars().take(600).collect();
            if let Some(skill) = hook_skill(&start) { self.carregar(&skill, "hook", chars, 1, None); return; }
            let event = a.get("hookName").filter(|v| truthy(Some(v))).or_else(|| a.get("hookEvent").filter(|v| truthy(Some(v))))
                .map(python_string).unwrap_or("?".into());
            let first = match raw {
                Some(Value::String(body)) => Some(body.as_str()),
                Some(Value::Array(blocks)) => blocks.iter()
                    .filter_map(|v| v.as_str().or_else(|| v.as_object().and_then(|b| text(b, "text"))))
                    .find(|s| !s.trim_matches(python_space).is_empty()),
                _ => None,
            }.unwrap_or("").trim_matches(python_space).split('\n').next().unwrap_or("")
                .chars().take(60).collect::<String>();
            plugin = plugin_de_hook(&first);
            format!("{kind}:{event}{}", if first.is_empty() { String::new() } else { format!(" · {first}") })
        } else { kind.into() };
        self.somar("contexto", &name, Sum { plugin: &plugin, calls: 1, chars, ..Sum::default() });
    }

    fn add_turn_usage(&mut self, usage: &Map<String, Value>, ident: Option<ResponseKey>) {
        let key = ident.unwrap_or(ResponseKey::Line(self.responses.len()));
        let turn = self.responses.get(&key).map_or(self.turns.len() - 1, |r| r.0);
        let write = py_int(usage.get("cache_creation_input_tokens"));
        let hour = usage.get("cache_creation").and_then(Value::as_object)
            .map_or(0, |c| py_int(c.get("ephemeral_1h_input_tokens")).max(0).min(write));
        self.responses.insert(key, (turn, (self.day.clone(), self.cwd.clone(), self.model.clone(), text(usage, "speed") == Some("fast")),
            [py_int(usage.get("input_tokens")), py_int(usage.get("output_tokens")), write, py_int(usage.get("cache_read_input_tokens")), hour]));
    }

    fn add_turn_tool(&mut self, name: &str, input: &Map<String, Value>) {
        let reg = if matches!(name, "Read" | "Edit" | "Write" | "NotebookEdit" | "Grep" | "Glob") {
            input.get("file_path").filter(|v| truthy(Some(v))).or_else(|| input.get("notebook_path").filter(|v| truthy(Some(v))))
                .or_else(|| input.get("path")).and_then(Value::as_str).filter(|p| !p.is_empty())
                .map(|path| ToolReg::P { rules_cwd: self.cwd.clone(), cwd: self.cwd.clone(), paths: vec![path.into()] })
        } else if name == "Bash" {
            text(input, "command").map(|cmd| ToolReg::C { rules_cwd: self.cwd.clone(), cwd: self.cwd.clone(), candidates: candidates(cmd) })
        } else if name == "Skill" {
            text(input, "skill").map(|skill| ToolReg::S { rules_cwd: self.cwd.clone(), target: format!("skill:{skill}") })
        } else { None };
        if let Some(reg) = reg { self.turns.last_mut().unwrap().push(reg); }
    }

    pub fn entries(&self) -> AreaEntries {
        let mut sums: IndexMap<usize, IndexMap<Group, [i64; 5]>> = IndexMap::new();
        for (turn, group, values) in self.responses.values() {
            let sum = sums.entry(*turn).or_default().entry(group.clone()).or_insert([0; 5]);
            for i in 0..5 { sum[i] += values[i]; }
        }
        AreaEntries { header: AreaHeader { fonte: None, session_id: None, subagente: None },
            turns: sums.into_iter().map(|(turn, groups)| (self.turns[turn].clone(), groups.into_iter()
                .map(|((dia, cwd, model, fast), values)| Unit { dia, cwd, model, fast, values }).collect())).collect() }
    }

    pub fn lines_without_area(&self) -> Vec<UsoLinha> {
        let mut rows = self.rows.values().cloned().collect::<Vec<_>>();
        rows.sort_by(|a, b| (&a.dia, &a.tipo, &a.nome, &a.detalhe).cmp(&(&b.dia, &b.tipo, &b.nome, &b.detalhe)));
        rows
    }
}

fn server(name: &str) -> &str { name.splitn(3, "__").nth(1).unwrap_or(name) }

fn agent_type(input: &Map<String, Value>) -> String {
    input.get("subagent_type").filter(|v| truthy(Some(v))).map(python_string).unwrap_or("general-purpose".into())
}

pub(crate) fn python_string(value: &Value) -> String {
    crate::transcript::py::py_str(value)
}

pub(crate) fn skill_path(command: &str) -> Option<String> {
    if command.contains("sed -i") { return None; }
    static PATH: OnceLock<Regex> = OnceLock::new();
    PATH.get_or_init(|| Regex::new(r#"[^\s\x1c-\x1f'"`]*/skills/[^\s\x1c-\x1f'"`]+\.md"#).unwrap())
        .find(command).map(|m| m.as_str().into())
}

fn hook_skill(body: &str) -> Option<String> {
    static SKILL: OnceLock<Regex> = OnceLock::new();
    SKILL.get_or_init(|| Regex::new(r"full content of your '([\p{L}\p{N}_.:-]+)' skill").unwrap())
        .captures(body).map(|c| c[1].into())
}
