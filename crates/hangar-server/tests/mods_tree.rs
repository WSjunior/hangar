mod mods_support;

use hangar_server::mods::tree;
use mods_support::{first_render, nth_render};
use serde_json::json;

#[test]
fn finds_button_by_key_with_its_press() {
    let tree = first_render("vitrine", "vitrine-botoes");
    let comum = tree::find(&tree, "V15-comum", &["Button"]).unwrap();
    assert_eq!((comum.kind.as_str(), comum.plugin.as_str()), ("Button", "vitrine"));
    assert!(comum.handle > 0);
    let a = tree::find(&tree, "V16-a", &["Button"]).unwrap();
    let b = tree::find(&tree, "V16-b", &["Button"]).unwrap();
    assert_ne!(a.handle, b.handle, "mesmo rótulo, handle próprio (P04)");
}

#[test]
fn input_is_found_only_as_input() {
    let tree = first_render("vitrine", "vitrine-campos");
    assert!(tree::find(&tree, "V18-campo", &["Button"]).is_none());
    assert_eq!(tree::find(&tree, "V18-campo", &["Input"]).unwrap().kind, "Input");
}

#[test]
fn key_without_press_or_unknown_is_none() {
    let tree = json!({"type": "Box", "children": [
        "texto solto",
        {"type": "Text", "props": {"key": "k"}, "children": ["k"]},
        {"type": "Button", "props": {"key": "sem-press", "label": "x"}},
    ]});
    assert!(tree::find(&tree, "k", &["Button"]).is_none());
    assert!(tree::find(&tree, "sem-press", &["Button"]).is_none());
    assert!(tree::find(&tree, "nao-existe", &["Button"]).is_none());
}

#[test]
fn first_in_document_order_wins() {
    let tree = json!({"type": "Box", "children": [
        {"type": "Box", "children": [{"type": "Button", "props": {"key": "k"}, "press": {"plugin": "m", "handle": 1}}]},
        {"type": "Button", "props": {"key": "k"}, "press": {"plugin": "m", "handle": 2}},
    ]});
    assert_eq!(tree::find(&tree, "k", &["Button"]).unwrap().handle, 1);
}

#[test]
fn engine_only_band_is_empty() {
    assert!(tree::is_engine_only(&first_render("sem-plugins", "above-prompt")));
    assert!(tree::is_engine_only(&first_render("vitrine", "above-prompt")), "ref 1 antes de o mod desenhar");
    // A segunda resposta é a faixa já desenhada pelo mod: um Box que contém o `engine`.
    let band = nth_render("vitrine", "above-prompt", 1);
    assert!(!tree::is_engine_only(&band));
    assert_eq!(band["type"], "Box");
    assert_eq!(band["children"][0]["type"], "engine");
}
