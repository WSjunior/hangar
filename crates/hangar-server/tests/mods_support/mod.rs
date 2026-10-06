//! Apoio dos testes da interface dos mods: as conversas gravadas pela sonda, já limpas.
#![allow(dead_code)]

use std::path::PathBuf;

use serde_json::Value;

pub fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mods")
}

/// (direção, mensagem) de cada linha: `out` é o que a sonda mandou, `in` o que o Claude Code mandou.
pub fn fixture(name: &str) -> Vec<(String, Value)> {
    let raw = std::fs::read_to_string(fixtures().join(format!("{name}.jsonl"))).unwrap();
    raw.lines().map(|line| {
        let entry: Value = serde_json::from_str(line).unwrap();
        (entry["dir"].as_str().unwrap().to_owned(), entry["msg"].clone())
    }).collect()
}

/// A árvore da primeira resposta gravada ao desenho de `instance`.
pub fn first_render(name: &str, instance: &str) -> Value {
    let lines = fixture(name);
    let id = lines.iter().find(|(dir, msg)| dir == "out" && msg["request"]["subtype"] == "ui_render"
        && msg["request"]["instance_id"] == instance).map(|(_, msg)| msg["request_id"].clone()).unwrap();
    lines.iter().find(|(dir, msg)| dir == "in" && msg["type"] == "control_response" && msg["response"]["request_id"] == id)
        .map(|(_, msg)| msg["response"]["response"]["tree"].clone()).unwrap()
}

/// A árvore da `n`-ésima resposta (contando de 0) ao desenho de `instance`.
pub fn nth_render(name: &str, instance: &str, n: usize) -> Value {
    let lines = fixture(name);
    let ids: Vec<Value> = lines.iter().filter(|(dir, msg)| dir == "out" && msg["request"]["subtype"] == "ui_render"
        && msg["request"]["instance_id"] == instance).map(|(_, msg)| msg["request_id"].clone()).collect();
    let answers: Vec<Value> = ids.iter().filter_map(|id| lines.iter().find(|(dir, msg)| dir == "in"
        && msg["type"] == "control_response" && msg["response"]["request_id"] == *id)
        .map(|(_, msg)| msg["response"]["response"]["tree"].clone())).collect();
    answers.get(n).cloned()
        .unwrap_or_else(|| panic!("{name}: só há {} respostas ao desenho de {instance}, faltou a {n}", answers.len()))
}

/// Todo o texto dos filhos de uma árvore, na ordem do documento.
pub fn texts(node: &Value) -> String {
    match node {
        Value::String(text) => text.clone(),
        Value::Object(map) => map.get("children").and_then(Value::as_array)
            .map(|children| children.iter().map(texts).collect::<Vec<_>>().join("")).unwrap_or_default(),
        _ => String::new(),
    }
}
