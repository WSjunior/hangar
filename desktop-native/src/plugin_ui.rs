//! Faixa acima do prompt que os mods do Claude Code desenham (SSE `plugin_ui`). A árvore chega
//! como o engine a monta (`Box`, `Text`, `Raster`, `Svg`...) e é traduzida aqui sem saber de que
//! mod veio: mod novo aparece sem código novo.
use std::sync::Arc;
use gpui_kit::*;
use gpui_kit::prelude::FluentBuilder;
use serde_json::Value;
use crate::theme;

/// Medidas dos mods são em células de terminal; estas são as da fonte mono de 12 px.
const CELL_W: f32 = 7.2;
const CELL_H: f32 = 17.;
const TEXT_PX: f32 = 12.;

/// Faixa sem nada para mostrar: ninguém desenhou, ou só o marcador do próprio engine.
pub fn is_empty(tree: &Value) -> bool { !tree.is_object() || tree["type"] == "engine" }

pub fn band(tree: &Value) -> Option<AnyElement> {
    if is_empty(tree) { return None; }
    Some(div().w_full().px(px(10.)).py(px(6.)).mb(px(4.)).rounded(px(8.)).bg(theme::inset())
        .font_family(theme::MONO).text_size(px(TEXT_PX)).line_height(px(CELL_H)).text_color(theme::text())
        .overflow_hidden().child(node(tree)).into_any_element())
}

fn node(v: &Value) -> AnyElement {
    match v {
        Value::String(s) => div().flex_shrink_0().child(s.clone()).into_any_element(),
        Value::Number(n) => div().flex_shrink_0().child(n.to_string()).into_any_element(),
        Value::Object(_) => element(v),
        _ => div().into_any_element(),
    }
}

fn text_of(v: &Value) -> String { v.as_str().unwrap_or("").to_owned() }

fn children(v: &Value) -> &[Value] { v["children"].as_array().map(Vec::as_slice).unwrap_or(&[]) }

/// Texto direto dos filhos, para rótulos de botão e link.
fn plain(v: &Value) -> String {
    children(v).iter().map(|c| match c { Value::String(s) => s.clone(), Value::Number(n) => n.to_string(), _ => String::new() }).collect()
}

fn element(v: &Value) -> AnyElement {
    let p = &v["props"];
    match v["type"].as_str().unwrap_or("") {
        "Box" => boxed(p, children(v)),
        "Text" => text(p, children(v)),
        "Raster" => raster(p),
        "Svg" => svg(p),
        "Markdown" => div().whitespace_normal().when(p["dimColor"] == true, |el| el.opacity(0.6))
            .child(unmark(&text_of(&p["text"]))).into_any_element(),
        "Code" => div().whitespace_normal().text_color(theme::muted()).child(text_of(&p["source"])).into_any_element(),
        "Link" => {
            let label = Some(text_of(&p["label"])).filter(|s| !s.is_empty()).unwrap_or_else(|| plain(v));
            div().text_color(theme::accent()).underline()
                .child(if label.is_empty() { text_of(&p["href"]) } else { label }).into_any_element()
        }
        // Botão de mod ainda não responde no Hangar: aparece como rótulo, sem fingir que clica.
        "Button" => {
            let label = Some(text_of(&p["label"])).filter(|s| !s.is_empty()).unwrap_or_else(|| plain(v));
            div().flex_shrink_0().px(px(CELL_W)).rounded(px(4.)).bg(theme::raised())
                .when(p["variant"] == "primary", |el| el.text_color(theme::accent())).child(label).into_any_element()
        }
        "Image" => div().text_color(theme::muted()).child(text_of(&p["alt"])).into_any_element(),
        _ => div().flex().children(children(v).iter().map(node)).into_any_element(),
    }
}

fn cols(v: &Value) -> Option<f32> { v.as_f64().map(|n| n as f32 * CELL_W) }
fn lines(v: &Value) -> Option<f32> { v.as_f64().map(|n| n as f32 * CELL_H) }

/// O primeiro número entre as chaves, na ordem: a mais específica vence (`paddingLeft` > `paddingX` > `padding`).
fn first(p: &Value, keys: &[&str]) -> Value { keys.iter().map(|k| p[*k].clone()).find(Value::is_number).unwrap_or(Value::Null) }

/// `Box` do Ink em flexbox do gpui. O padrão do Ink é linha, não coluna.
fn boxed(p: &Value, kids: &[Value]) -> AnyElement {
    if p["display"] == "none" { return div().into_any_element(); }
    let mut el = div().flex().min_w_0();
    el = match p["flexDirection"].as_str() {
        Some("column") => el.flex_col(),
        Some("column-reverse") => el.flex_col_reverse(),
        Some("row-reverse") => el.flex_row_reverse(),
        _ => el.flex_row(),
    };
    el = match p["justifyContent"].as_str() {
        Some("space-between") => el.justify_between(),
        Some("space-around") => el.justify_around(),
        Some("space-evenly") => el.justify_evenly(),
        Some("center") => el.justify_center(),
        Some("flex-end") => el.justify_end(),
        _ => el,
    };
    el = match p["alignItems"].as_str() {
        Some("center") => el.items_center(),
        Some("flex-end") => el.items_end(),
        Some("flex-start") => el.items_start(),
        _ => el,
    };
    if let Some(g) = p["flexGrow"].as_f64() { el = el.flex_grow(g as f32); }
    if p["flexWrap"] == "wrap" { el = el.flex_wrap(); }
    // Largura fixa do terminal vira teto: a coluna da conversa pode ser mais estreita que o pane.
    if let Some(w) = cols(&p["width"]) { el = el.w_full().max_w(px(w)); }
    if let Some(w) = cols(&p["minWidth"]) { el = el.min_w(px(w)); }
    if let Some(g) = cols(&first(p, &["columnGap", "gap"])) { el = el.gap_x(px(g)); }
    if let Some(g) = lines(&first(p, &["rowGap", "gap"])) { el = el.gap_y(px(g)); }
    if let Some(v) = lines(&first(p, &["paddingTop", "paddingY", "padding"])) { el = el.pt(px(v)); }
    if let Some(v) = lines(&first(p, &["paddingBottom", "paddingY", "padding"])) { el = el.pb(px(v)); }
    if let Some(v) = cols(&first(p, &["paddingLeft", "paddingX", "padding"])) { el = el.pl(px(v)); }
    if let Some(v) = cols(&first(p, &["paddingRight", "paddingX", "padding"])) { el = el.pr(px(v)); }
    if let Some(v) = lines(&first(p, &["marginTop", "marginY", "margin"])) { el = el.mt(px(v)); }
    if let Some(v) = lines(&first(p, &["marginBottom", "marginY", "margin"])) { el = el.mb(px(v)); }
    if let Some(v) = cols(&first(p, &["marginLeft", "marginX", "margin"])) { el = el.ml(px(v)); }
    if let Some(v) = cols(&first(p, &["marginRight", "marginX", "margin"])) { el = el.mr(px(v)); }
    if let Some(c) = color(&p["backgroundColor"]) { el = el.bg(c); }
    if p["borderStyle"].is_string() {
        el = el.border_1().rounded(px(4.)).border_color(color(&p["borderColor"]).unwrap_or_else(theme::border));
    }
    if p["overflow"] == "hidden" { el = el.overflow_hidden(); }
    el.children(kids.iter().map(node)).into_any_element()
}

/// `Text` do Ink: cor, ênfase e corte. `dimColor` é opacidade, como no terminal.
fn text(p: &Value, kids: &[Value]) -> AnyElement {
    let fg = color(&p["color"]);
    let bg = color(&p["backgroundColor"]);
    let (fg, bg) = if p["inverse"] == true { (bg.or(Some(theme::background())), fg.or(Some(theme::text()))) } else { (fg, bg) };
    let truncate = p["wrap"].as_str().is_some_and(|w| w.starts_with("truncate") || w == "end" || w == "middle");
    // Texto dentro de texto vira trechos lado a lado: o gpui não tem span em linha.
    let mut el = div().flex().flex_row().min_w_0()
        .when(!truncate, |el| el.flex_shrink_0())
        .when_some(fg, |el, c| el.text_color(c))
        .when_some(bg, |el, c| el.bg(c))
        .when(p["bold"] == true, |el| el.font_weight(FontWeight::BOLD))
        .when(p["italic"] == true, |el| el.italic())
        .when(p["underline"] == true, |el| el.underline())
        .when(p["strikethrough"] == true, |el| el.line_through())
        .when(p["dimColor"] == true, |el| el.opacity(0.6));
    if truncate {
        el = el.overflow_hidden().whitespace_nowrap();
        return el.child(div().truncate().child(plain_deep(kids))).into_any_element();
    }
    el.children(kids.iter().map(node)).into_any_element()
}

/// Texto de uma subárvore inteira, para o corte com reticências que o gpui só faz num texto só.
fn plain_deep(kids: &[Value]) -> String {
    kids.iter().map(|c| match c {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Object(_) => plain_deep(children(c)),
        _ => String::new(),
    }).collect()
}

/// Células do `Raster`: base64 de triplas u32 little-endian `[código, frente, fundo]`, por linha.
/// Vizinhas da mesma cor viram um trecho só. O Raster vem com a largura do pane do terminal: em
/// coluna mais estreita cada trecho encolhe na proporção das suas células, sem rolar de lado.
fn raster(p: &Value) -> AnyElement {
    use base64::Engine as _;
    let columns = p["columns"].as_u64().unwrap_or(0) as usize;
    let rows = p["rows"].as_u64().unwrap_or(0) as usize;
    let bytes = base64::engine::general_purpose::STANDARD.decode(p["cells"].as_str().unwrap_or("")).unwrap_or_default();
    let word = |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let mut grid = div().flex().flex_col().flex_grow(1.).min_w_0().max_w(px(columns as f32 * CELL_W));
    for r in 0..rows {
        let mut runs: Vec<(String, Option<Hsla>, Option<Hsla>)> = Vec::new();
        for c in 0..columns {
            let at = (r * columns + c) * 12;
            if at + 12 > bytes.len() { break; }
            let ch = char::from_u32(word(at)).filter(|c| !c.is_control()).unwrap_or(' ');
            let (fg, bg) = (cell_color(word(at + 4)), cell_color(word(at + 8)));
            match runs.last_mut() {
                Some(run) if run.1 == fg && run.2 == bg => run.0.push(ch),
                _ => runs.push((ch.to_string(), fg, bg)),
            }
        }
        grid = grid.child(div().flex().flex_row().w_full().min_w_0().whitespace_nowrap().children(runs.into_iter().map(|(t, fg, bg)| {
            div().flex_basis(px(0.)).flex_grow(t.chars().count() as f32).flex_shrink(1.).min_w_0().overflow_hidden()
                .when_some(fg, |el, c| el.text_color(c)).when_some(bg, |el, c| el.bg(c)).child(t)
        })));
    }
    grid.into_any_element()
}

/// Bit 24 sozinho é a cor padrão do terminal; o resto é 0x00RRGGBB.
fn cell_color(v: u32) -> Option<Hsla> { (v & 0x0100_0000 == 0).then(|| rgb(v & 0x00ff_ffff).into()) }

/// O SVG vira imagem; as animações de CSS dele ficam paradas no primeiro quadro.
fn svg(p: &Value) -> AnyElement {
    let source = text_of(&p["source"]);
    if source.is_empty() { return div().into_any_element(); }
    let image = Arc::new(Image::from_bytes(ImageFormat::Svg, source.into_bytes()));
    let mut el = img(image).max_w_full();
    if let Some(w) = p["width"].as_f64() { el = el.w(px(w as f32)); }
    if let Some(h) = p["height"].as_f64() { el = el.h(px(h as f32)); }
    el.into_any_element()
}

/// Cor de um `Text`/`Box`: `#rrggbb` ou nome do Ink, no tom do terminal.
fn color(v: &Value) -> Option<Hsla> {
    let name = v.as_str()?;
    if let Some(hex) = name.strip_prefix('#') {
        let hex = if hex.len() == 3 { hex.chars().flat_map(|c| [c, c]).collect() } else { hex.to_owned() };
        return u32::from_str_radix(&hex, 16).ok().map(|n| rgb(n).into());
    }
    let value = match name {
        "black" => 0x000000, "red" => 0xcd3131, "green" => 0x0dbc79, "yellow" => 0xe5e510,
        "blue" => 0x2472c8, "magenta" => 0xbc3fbc, "cyan" => 0x11a8cd, "white" => 0xe5e5e5,
        "gray" | "grey" => 0x808080, "blackBright" => 0x666666, "redBright" => 0xf14c4c,
        "greenBright" => 0x23d18b, "yellowBright" => 0xf5f543, "blueBright" => 0x3b8eea,
        "magentaBright" => 0xd670d6, "cyanBright" => 0x29b8db, "whiteBright" => 0xffffff,
        _ => return None,
    };
    Some(rgb(value).into())
}

/// Markdown de mod sem as marcações: o nativo não abre um leitor de markdown para uma faixa.
fn unmark(text: &str) -> String {
    text.lines().map(|l| l.trim_start_matches('#').trim_start().replace("**", "").replace("__", "").replace('`', ""))
        .collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_band_is_the_engine_marker_or_nothing() {
        assert!(is_empty(&Value::Null));
        assert!(is_empty(&json!({ "type": "engine", "ref": 1 })));
        assert!(!is_empty(&json!({ "type": "Box", "children": [] })));
    }

    #[test]
    fn default_terminal_color_is_none() {
        assert_eq!(cell_color(0x0100_0000), None);
        assert!(cell_color(0x5aa6ff).is_some());
    }

    #[test]
    fn colors_accept_hex_and_ink_names() {
        assert!(color(&json!("#5aa6ff")).is_some());
        assert!(color(&json!("redBright")).is_some());
        assert_eq!(color(&json!("nope")), None);
    }
}
