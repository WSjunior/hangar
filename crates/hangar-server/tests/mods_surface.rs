mod mods_support;

use hangar_server::mods::model::*;
use hangar_server::mods::surface::Surface;
use hangar_server::runtime::protocol::RequestId;
use mods_support::*;
use serde_json::{Value, json};

pub fn writes(out: &[SurfaceEffect]) -> Vec<Value> {
    out.iter().filter_map(|effect| match effect { SurfaceEffect::Write { frame } => Some(frame.clone()), _ => None }).collect()
}
pub fn request(out: &[SurfaceEffect], subtype: &str) -> Value {
    writes(out).into_iter().find(|frame| frame["request"]["subtype"] == subtype).unwrap_or_else(|| panic!("sem {subtype}"))
}
pub fn ok(surface: &mut Surface, frame: &Value, body: Value, now: f64) -> Vec<SurfaceEffect> {
    let id = RequestId::String(frame["request_id"].as_str().unwrap().into());
    surface.on_response(&id, &json!({"subtype": "success", "request_id": frame["request_id"], "response": body}), now)
}
pub fn reply_of(out: &[SurfaceEffect], token: u64) -> Option<Result<Value, ModsError>> {
    out.iter().find_map(|effect| match effect { SurfaceEffect::Reply { token: t, result } if *t == token => Some(result.clone()), _ => None })
}
pub fn published(out: &[SurfaceEffect]) -> Option<Value> {
    out.iter().rev().find_map(|effect| match effect { SurfaceEffect::Publish { data } => Some(data.clone()), _ => None })
}
pub fn panes(list: Value, shown: &str) -> Value {
    json!({"type": "system", "subtype": "ui_panes", "panes": list, "shown_id": shown, "focused_id": null, "focus_requested_id": null})
}
/// Superfície ligada, com a faixa `band` desenhada; devolve os pedidos de desenho e de rol pendentes.
pub fn ready(band: Value) -> Surface {
    let mut surface = Surface::new("ui:t".into());
    let attach = request(&surface.start(0.0), "ui_attach");
    let out = ok(&mut surface, &attach, json!({"surfaces": ["desktop"]}), 0.0);
    let render = request(&out, "ui_render");
    ok(&mut surface, &render, json!({"tree": band, "hooked": true}), 0.0);
    surface
}

#[test]
fn attach_then_band_and_panes_with_full_props() {
    let mut surface = Surface::new("ui:t".into());
    let attach = request(&surface.start(0.0), "ui_attach");
    assert!(attach["request_id"].as_str().unwrap().starts_with("ui:t:"));
    assert_eq!(attach["request"], json!({"subtype": "ui_attach", "surface": "desktop", "client_id": "hangar",
        "viewport": {"columns": 120, "rows": 40, "isFullscreen": true}, "answers": ["ui_copy"]}));
    let out = ok(&mut surface, &attach, json!({"surfaces": ["desktop"]}), 0.0);
    let render = request(&out, "ui_render");
    assert_eq!(render["request"]["component"], "AbovePrompt");
    assert_eq!(render["request"]["instance_id"], "above-prompt");
    assert_eq!(render["request"]["client_id"], "hangar");
    assert_eq!(render["request"]["props"], json!({"hasSurvey": false, "isWorking": false, "bodyColumns": 110, "maxRows": 30,
        "scroll": {"offset": 0, "bodyRows": 30}, "view": {}}));
    assert_eq!(request(&out, "ui_panes")["request"]["client_id"], "hangar");
    assert!(surface.is_ready());
}

#[test]
fn recorded_vitrine_band_is_published() {
    let drive = Drive::start("vitrine");
    let view = drive.view();
    assert_eq!((view["source"].as_str(), view["columns"].as_u64()), (Some("surface"), Some(110)));
    assert!(view["shown_id"].is_null() && view["panes"] == json!([]));
    assert!(texts(&view["above"]).contains("superfície desktop"), "V53 lido pelo mod");
}

#[test]
fn band_without_mods_is_null() {
    let drive = Drive::start("sem-plugins");
    assert!(drive.view()["above"].is_null());
}

#[test]
fn new_pane_is_drawn_with_its_columns() {
    let mut surface = ready(json!({"type": "engine", "ref": 1}));
    let out = surface.on_notice(&panes(json!([{"id": "p", "title": "Painel", "plugin": "m", "columns": 70}]), "p"), 1.0);
    let render = request(&out, "ui_render");
    assert_eq!(render["request"]["component"], "Pane");
    assert_eq!(render["request"]["instance_id"], "p");
    assert_eq!(render["request"]["props"], json!({"title": "Painel", "isFocused": false, "bodyColumns": 68, "placement": "dock",
        "scroll": {"offset": 0, "bodyRows": 40}, "view": {}}));
    let tree = json!({"type": "Text", "children": ["oi"]});
    let view = published(&ok(&mut surface, &render, json!({"tree": tree, "hooked": true}), 1.1)).unwrap();
    // O `columns` publicado é o `bodyColumns` que o mod recebeu (70 - 2), como o contrato dos apps pede (A3).
    assert_eq!(view["panes"], json!([{"id": "p", "title": "Painel", "placement": "dock", "columns": 68, "tree": tree}]));
    assert_eq!(view["shown_id"], "p");
    assert!(view["above"].is_null(), "faixa só com o nó engine sai como vazia");
}

#[test]
fn invalidate_batches_in_100ms_and_ignores_closed_instances() {
    let mut surface = ready(json!({"type": "Text"}));
    let out = surface.on_notice(&panes(json!([{"id": "p", "title": "P", "plugin": "m"}]), "p"), 1.0);
    ok(&mut surface, &request(&out, "ui_render"), json!({"tree": {"type": "Text"}}), 1.0);
    let changed = json!({"type": "system", "subtype": "ui_invalidate", "event": "ui.render", "instances": [
        {"surface": "desktop", "component": "Pane", "instance_id": "p"},
        {"surface": "desktop", "component": "Pane", "instance_id": "fechado"}]});
    assert!(writes(&surface.on_notice(&changed, 2.0)).is_empty(), "espera a janela");
    assert!(writes(&surface.on_notice(&changed, 2.05)).is_empty());
    assert!(writes(&surface.tick(2.05)).is_empty());
    let out = surface.tick(2.1);
    let renders: Vec<Value> = writes(&out).into_iter().filter(|frame| frame["request"]["subtype"] == "ui_render").collect();
    assert_eq!(renders.len(), 1, "um pedido por instância montada");
    assert_eq!(renders[0]["request"]["instance_id"], "p");
    // Desenho em voo não é pedido de novo: responde antes da próxima rodada.
    ok(&mut surface, &renders[0], json!({"tree": {"type": "Text"}}), 2.2);
    let all = json!({"type": "system", "subtype": "ui_invalidate", "event": "ui.render"});
    surface.on_notice(&all, 3.0);
    let ids: Vec<Value> = writes(&surface.tick(3.1)).into_iter().map(|frame| frame["request"]["instance_id"].clone()).collect();
    assert!(ids.contains(&json!("above-prompt")) && ids.contains(&json!("p")), "sem instances é tudo o que está montado");
}

#[test]
fn toast_becomes_effect_and_status_is_ignored() {
    let mut surface = ready(json!({"type": "Text"}));
    let out = surface.on_notice(&json!({"type": "system", "subtype": "ui_toast", "plugin": "vitrine", "text": "linha 1\nlinha 2", "timeout_ms": 8000}), 1.0);
    assert_eq!(out, vec![SurfaceEffect::Toast { plugin: "vitrine".into(), text: "linha 1\nlinha 2".into(), timeout_ms: 8000 }]);
    assert!(surface.on_notice(&json!({"type": "system", "subtype": "ui_status", "plugin": "vitrine", "text": "x"}), 1.0).is_empty());
}

#[test]
fn copy_is_answered_true_and_handed_over() {
    let mut idle = Surface::new("ui:t".into());
    let out = idle.on_copy(&json!("uuid-1"), &json!({"subtype": "ui_copy", "plugin": "vitrine", "text": "antes"}));
    assert_eq!(writes(&out)[0]["response"]["response"], json!({"copied": true}));
    assert_eq!(out.len(), 1, "antes de ligar, só a resposta (pedido velho do snapshot do cano)");
    let mut surface = ready(json!({"type": "Text"}));
    let out = surface.on_copy(&json!("uuid-2"), &json!({"subtype": "ui_copy", "surface": "desktop", "client_id": "hangar", "plugin": "vitrine", "text": "Texto"}));
    assert_eq!(writes(&out)[0], json!({"type": "control_response", "response": {"subtype": "success", "request_id": "uuid-2", "response": {"copied": true}}}));
    assert!(out.contains(&SurfaceEffect::Copied { plugin: "vitrine".into(), text: "Texto".into() }));
}

#[test]
fn responses_from_another_life_are_ignored() {
    let mut surface = Surface::new("ui:a".into());
    surface.start(0.0);
    let foreign = RequestId::String("ui:b:1".into());
    assert!(!surface.owns(&foreign));
    assert!(surface.owns(&RequestId::String("ui:a:1".into())));
    assert!(!surface.owns(&RequestId::String("ui:ab:1".into())), "prefixo inteiro, não começo de outro");
    assert!(surface.on_response(&foreign, &json!({"subtype": "success", "response": {"surfaces": ["desktop"]}}), 0.1).is_empty());
    assert!(!surface.is_ready());
}

#[test]
fn refused_attach_turns_off_and_clears() {
    let mut surface = Surface::new("ui:t".into());
    let attach = request(&surface.start(0.0), "ui_attach");
    let id = RequestId::String(attach["request_id"].as_str().unwrap().into());
    let out = surface.on_response(&id, &json!({"subtype": "error", "request_id": attach["request_id"], "error": "desconhecido"}), 0.1);
    assert!(!surface.is_ready());
    assert_eq!(published(&out).unwrap()["panes"], json!([]));
    let silent = &mut Surface::new("ui:u".into());
    silent.start(0.0);
    assert_eq!(silent.deadline(), Some(15.0));
    assert!(published(&silent.tick(15.0)).is_some(), "sem resposta em 15 s também desliga");
}

#[test]
fn invalid_tree_keeps_the_pane_listed() {
    let mut surface = ready(json!({"type": "Text"}));
    let out = surface.on_notice(&panes(json!([{"id": "quebrado", "title": "Quebrado", "plugin": "m"}]), "quebrado"), 1.0);
    let view = published(&ok(&mut surface, &request(&out, "ui_render"), json!({"tree": {"type": "engine", "ref": 0}}), 1.1)).unwrap();
    assert_eq!(view["panes"][0]["tree"], json!({"type": "engine", "ref": 0}), "S8: fica na lista, desenhado vazio pelo app");
}
