//! Regras puras de classificação do uso, com a semântica do leitor Python.

use super::py::char_len;
use regex::Regex;
use serde_json::{Map, Value};
use std::sync::OnceLock;

pub(super) fn python_space(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}')
}

pub fn texto_len(content: Option<&Value>) -> i64 {
    match content {
        Some(Value::String(text)) => char_len(text) as i64,
        Some(Value::Array(blocks)) => blocks.iter().map(|block| match block {
            Value::String(text) => char_len(text) as i64,
            Value::Object(obj) => texto_len(if obj.get("text").is_some_and(Value::is_string) {
                obj.get("text")
            } else { obj.get("content") }),
            _ => 0,
        }).sum(),
        _ => 0,
    }
}

pub fn comando_bash(cmd: &str) -> String {
    static SEPARATORS: OnceLock<Regex> = OnceLock::new();
    let separators = SEPARATORS.get_or_init(|| Regex::new(r"&&|\|\||[;|\n]").unwrap());
    for segment in separators.split(cmd) {
        for token in segment.split(python_space).filter(|t| !t.is_empty()) {
            let mut text = token.trim_start_matches('(');
            if let Some((key, value)) = text.split_once('=') {
                if !key.is_empty() {
                    let Some(command) = value.strip_prefix("$(") else { continue };
                    text = command;
                }
            }
            if text.is_empty() || text.starts_with('-') || text.chars().next().is_some_and(python_digit)
                || matches!(text, "sudo" | "env" | "time" | "timeout" | "rtk" | "command" | "exec" | "nohup" | "nice") {
                continue;
            }
            if matches!(text, "do" | "then" | "else" | "done" | "fi" | "in" | "cd") { break; }
            return text.rsplit('/').next().unwrap().into();
        }
    }
    "?".into()
}

fn python_digit(c: char) -> bool {
    static DECIMAL: OnceLock<Regex> = OnceLock::new();
    let mut buf = [0; 4];
    DECIMAL.get_or_init(|| Regex::new(r"^\p{Nd}$").unwrap()).is_match(c.encode_utf8(&mut buf))
        || matches!(c, '\u{b2}'..='\u{b3}' | '\u{b9}' | '\u{1369}'..='\u{1371}'
            | '\u{19da}' | '\u{2070}' | '\u{2074}'..='\u{2079}' | '\u{2080}'..='\u{2089}'
            | '\u{2460}'..='\u{2468}' | '\u{2474}'..='\u{247c}' | '\u{2488}'..='\u{2490}'
            | '\u{24ea}' | '\u{24f5}'..='\u{24fd}' | '\u{24ff}' | '\u{2776}'..='\u{277e}'
            | '\u{2780}'..='\u{2788}' | '\u{278a}'..='\u{2792}' | '\u{10a40}'..='\u{10a43}'
            | '\u{10e60}'..='\u{10e68}' | '\u{11052}'..='\u{1105a}' | '\u{1f100}'..='\u{1f10a}')
}

pub fn pede_agente(prompt: &str) -> bool {
    static AGENT: OnceLock<Regex> = OnceLock::new();
    static WORD: OnceLock<Regex> = OnceLock::new();
    let word = WORD.get_or_init(|| Regex::new(r"^[\p{L}\p{N}_]$").unwrap());
    let is_word = |c: char| {
        let mut buf = [0; 4];
        word.is_match(c.encode_utf8(&mut buf))
    };
    // O re do Python não inclui marcas/conectores em \w e equipara os quatro I.
    let pattern = AGENT.get_or_init(|| Regex::new(r"(?i)(sub-?agentes?|agentes?|agents?|explore|paralelo|parallel|d[iİı]spara|delega|workflow|fan-?out|subagent-dr[iİı]ven)").unwrap());
    pattern.find_iter(prompt).any(|found| {
        !prompt[..found.start()].chars().next_back().is_some_and(is_word)
            && !prompt[found.end()..].chars().next().is_some_and(is_word)
    })
}

pub fn plugin_de(name: &str) -> String {
    name.split_once(':').map_or_else(String::new, |(plugin, _)| plugin.into())
}

pub fn plugin_de_hook(first: &str) -> String {
    let text = first.trim_matches(python_space);
    if let Some(rest) = text.strip_prefix('[') {
        if let Some((label, _)) = rest.split_once(']') { return label.trim_matches(python_space).into(); }
    }
    if text.contains(" MODE ACTIVE") { return text.split(' ').next().unwrap().to_lowercase(); }
    String::new()
}

pub fn skill_do_caminho(path: &str) -> Option<(String, bool)> {
    let path = path.replace('\\', "/");
    let parts: Vec<&str> = path.split('/').collect();
    let last_skills = parts.iter().rposition(|p| *p == "skills")?;
    if !parts.last()?.ends_with(".md") { return None; }
    let is_main = *parts.last()? == "SKILL.md";
    let name = if is_main {
        *parts.get(parts.len().checked_sub(2)?)?
    } else {
        let index = parts.iter().rposition(|p| matches!(*p, "references" | "reference" | "scripts" | "assets" | "templates" | "examples"))
            .unwrap_or(last_skills + 2);
        if index <= last_skills + 1 || index > parts.len() - 1 { return None; }
        parts[index - 1]
    };
    let mut plugin = "";
    if let Some(index) = parts.iter().position(|p| *p == "plugins") {
        if parts.get(index + 1) == Some(&"cache") {
            plugin = parts.get(index + 3).copied().unwrap_or("");
        } else if parts.get(index + 1) == Some(&"marketplaces") {
            plugin = parts.get(index + 2).copied().unwrap_or("").strip_suffix("-marketplace")
                .unwrap_or_else(|| parts.get(index + 2).copied().unwrap_or(""));
        }
    }
    Some((if plugin.is_empty() { name.into() } else { format!("{plugin}:{name}") }, is_main))
}

pub fn tokens_de_imagem(block: &Map<String, Value>) -> (i64, i64) {
    let source = block.get("source").and_then(Value::as_object);
    let data = source.and_then(|s| s.get("data")).and_then(Value::as_str).unwrap_or("");
    let mut tokens = 0;
    if source.and_then(|s| s.get("media_type")).and_then(Value::as_str) == Some("image/png") && char_len(data) >= 32 {
        if let Some(bytes) = decode_header(&data.chars().take(32).collect::<String>()) {
            if bytes.len() >= 24 && bytes[..8] == *b"\x89PNG\r\n\x1a\n" {
                let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
                let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
                tokens = ((u64::from(width) * u64::from(height)) / 750) as i64;
            }
        }
    }
    (char_len(data) as i64, tokens)
}

fn decode_header(text: &str) -> Option<Vec<u8>> {
    if !text.is_ascii() { return None; }
    let mut bytes = Vec::with_capacity(24);
    let mut value = 0u32;
    let mut bits = 0;
    let mut padding = 0;
    for c in text.bytes() {
        let digit = match c {
            b'A'..=b'Z' => c - b'A', b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52, b'+' => 62, b'/' => 63,
            b'=' => { padding += 1; if bits == 2 || (bits == 4 && padding >= 2) { break; } continue; }
            _ => continue,
        };
        padding = 0;
        value = (value << 6) | u32::from(digit);
        bits += 6;
        if bits >= 8 { bits -= 8; bytes.push((value >> bits) as u8); }
    }
    if bits != 0 && padding == 0 { return None; }
    Some(bytes)
}
