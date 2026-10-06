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

/// O rótulo do `Button` de `key`, como o terminal o desenha: `label`, ou o texto dos filhos.
pub fn label(tree: &Value, key: &str) -> Option<String> {
    let mut stack = vec![tree];
    while let Some(node) = stack.pop() {
        let Some(object) = node.as_object() else { continue };
        if object.get("type").and_then(Value::as_str) == Some("Button") && node["props"]["key"] == key {
            let text = match node["props"]["label"].as_str().filter(|label| !label.is_empty()) {
                Some(label) => label.to_owned(),
                None => object.get("children").and_then(Value::as_array)
                    .map(|children| children.iter().filter_map(Value::as_str).collect()).unwrap_or_default(),
            };
            let text = text.trim();
            return (!text.is_empty()).then(|| text.to_owned());
        }
        if let Some(children) = object.get("children").and_then(Value::as_array) {
            stack.extend(children.iter().rev());
        }
    }
    None
}

/// Quantas vezes `label` aparece no texto que a árvore desenha, também fora da área visível: rótulos de
/// botão, link e campo, opções, valores, códigos e o texto dos filhos de cada nó (juntos, como o terminal
/// os desenha). Conta o texto que só contém o rótulo, porque a busca na tela casa trecho (`find_label`):
/// com mais de uma, o clique pelo mouse pode cair no elemento errado.
pub fn label_count(tree: &Value, label: &str) -> usize {
    let target = label.trim();
    if target.is_empty() { return 0; }
    let hits = |text: Option<&str>| text.map_or(0, |text| text.matches(target).count());
    let mut total = 0;
    let mut stack = vec![tree];
    while let Some(node) = stack.pop() {
        match node {
            Value::String(text) => total += hits(Some(text)),
            Value::Object(object) => {
                let props = &node["props"];
                total += ["label", "placeholder", "value", "submitLabel", "source"].iter().map(|name| hits(props[*name].as_str())).sum::<usize>();
                total += props["options"].as_array().map_or(0, |options| options.iter().map(|o| hits(o["label"].as_str())).sum());
                if let Some(children) = object.get("children").and_then(Value::as_array) {
                    // Os textos soltos de um nó saem juntos na tela: um rótulo partido entre eles conta uma vez.
                    let text: String = children.iter().filter_map(Value::as_str).collect();
                    total += hits(Some(&text));
                    stack.extend(children.iter().filter(|child| child.is_object()));
                }
            }
            _ => {}
        }
    }
    total
}

/// Botões de todas as faixas: o anel do `ctrl+x tab` passa por cada um antes dos painéis ((t)).
pub fn count_buttons(tree: &Value) -> usize {
    let mut stack = vec![tree];
    let mut total = 0;
    while let Some(node) = stack.pop() {
        let Some(object) = node.as_object() else { continue };
        total += usize::from(object.get("type").and_then(Value::as_str) == Some("Button"));
        if let Some(children) = object.get("children").and_then(Value::as_array) {
            stack.extend(children);
        }
    }
    total
}

/// O começo do primeiro texto legível da faixa, como ele aparece no pane (`plugin_bridge.band_anchor`):
/// é por ele que a leitura da tela acha onde a faixa começa, e a prévia, onde a conversa acaba.
pub fn anchor(tree: &Value) -> Option<String> {
    let mut stack = vec![tree];
    while let Some(node) = stack.pop() {
        match node {
            Value::String(text) => {
                let text = text.trim();
                if text.chars().filter(|c| c.is_alphanumeric()).count() >= 3 {
                    return Some(text.chars().take(16).collect());
                }
            }
            Value::Object(object) => {
                if let Some(children) = object.get("children").and_then(Value::as_array) {
                    stack.extend(children.iter().rev());
                }
                // Botão e link desenham o `label` antes dos filhos.
                if let Some(label @ Value::String(_)) = object.get("props").and_then(|props| props.get("label")) {
                    stack.push(label);
                }
            }
            _ => {}
        }
    }
    None
}
