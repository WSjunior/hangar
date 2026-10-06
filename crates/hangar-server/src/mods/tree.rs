//! Leituras puras da árvore que o Claude Code devolve. O Hangar não interpreta o desenho: só acha o
//! elemento que o app pediu e o `press` com que o clique volta ao mod.
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    pub kind: String,
    pub plugin: String,
    pub handle: i64,
}

/// O primeiro elemento de um dos `kinds` com esta `key`, na ordem do documento, com o `press` dele.
/// Sem `press` não há como acionar: conta como ausente.
pub fn find(tree: &Value, key: &str, kinds: &[&str]) -> Option<Control> {
    let mut stack = vec![tree];
    while let Some(node) = stack.pop() {
        let Some(object) = node.as_object() else { continue };
        let kind = object.get("type").and_then(Value::as_str).unwrap_or("");
        if kinds.contains(&kind) && node["props"]["key"] == key {
            let press = &node["press"];
            if let (Some(plugin), Some(handle)) = (press["plugin"].as_str(), press["handle"].as_i64()) {
                return Some(Control { kind: kind.to_owned(), plugin: plugin.to_owned(), handle });
            }
        }
        if let Some(children) = object.get("children").and_then(Value::as_array) {
            stack.extend(children.iter().rev());
        }
    }
    None
}

/// Faixa sem nada de mod: só o nó `engine` (`ref: 1` com mods carregados, `ref: 0` sem mod nenhum).
pub fn is_engine_only(tree: &Value) -> bool {
    tree["type"] == "engine"
}
