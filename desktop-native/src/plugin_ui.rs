//! Faixa acima do prompt e painéis que os mods do Claude Code desenham (SSE `plugin_ui`). A árvore
//! chega como o engine a monta (`Box`, `Text`, `Raster`, `Svg`...) e é traduzida aqui sem saber de
//! que mod veio: mod novo aparece sem código novo.
use std::{borrow::Cow, collections::HashSet, rc::Rc};
use std::sync::Arc;
use std::time::Duration;
use gpui_kit::*;
use gpui_kit::prelude::FluentBuilder;
use serde_json::Value;
use crate::theme;

/// Medidas dos mods são em células de terminal; estas são as da fonte mono de 12 px.
pub const CELL_W: f32 = 7.2;
const CELL_H: f32 = 17.;
const TEXT_PX: f32 = 12.;

/// Faixa sem nada para mostrar: ninguém desenhou, ou só o marcador do próprio engine.
pub fn is_empty(tree: &Value) -> bool { !tree.is_object() || tree["type"] == "engine" }

/// Clique num botão de mod: (site, key). O site é `above-prompt` ou o id do painel.
pub type Press = Rc<dyn Fn(&str, &str, &mut Window, &mut App)>;
pub const BAND_SITE: &str = "above-prompt";
pub const PANE_CLOSE_KEY: &str = "__close__";

/// Troca de aba pedida no app: o id do painel.
pub type Show = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// O ponteiro entrou (`true`) ou saiu de um escopo de hover ou de um cartão absoluto; o id é o lugar e o caminho.
pub type Hover = Rc<dyn Fn(&str, bool, &mut Window, &mut App)>;

/// O que o app passa para desenhar a faixa e os painéis: quem atende o clique e a troca de aba, a largura da faixa
/// (`columns`) e o hover: quem avisa o app do ponteiro e os trechos com o ponteiro em cima. Sem `press`, botão é só
/// rótulo e não há `✕`. `columns` é a largura, em colunas, para a qual a faixa foi desenhada.
pub struct View<'a> {
    pub press: Option<Press>,
    pub show: Option<Show>,
    pub columns: Option<f64>,
    pub hover: Option<Hover>,
    pub hovered: &'a HashSet<String>,
}

/// Onde a árvore está desenhada e o que o app oferece. `links` numera os links na ordem da árvore: o mesmo endereço
/// duas vezes não repete o id do elemento.
struct Ctx<'a> { site: &'a str, view: &'a View<'a>, place: Option<f64>, links: std::cell::Cell<usize> }

impl Ctx<'_> {
    /// Id de um trecho: o lugar e o caminho dele na árvore.
    fn spot(&self, at: &Spot) -> String { format!("{}{}", self.site, at.path) }
}

/// Onde o nó está: o caminho na árvore, que vira o id do escopo de hover, e se o escopo mais próximo está aceso.
struct Spot { path: String, lit: bool }

impl Spot {
    fn root() -> Spot { Spot { path: String::new(), lit: false } }
    fn child(&self, i: usize, lit: bool) -> Spot { Spot { path: format!("{}/{i}", self.path), lit } }
}

/// Box com `key` é escopo de hover.
fn is_scope(v: &Value) -> bool { v["type"] == "Box" && v["props"]["key"].as_str().is_some_and(|k| !k.is_empty()) }

/// A subárvore tem algum `hover`: só então o escopo precisa avisar o app do ponteiro.
pub fn wants_hover(v: &Value) -> bool { v["hover"].is_object() || children(v).iter().any(wants_hover) }

/// Um escopo está aceso com o ponteiro nele ou num trecho dentro dele (o cartão absoluto, que pode sair da área do
/// escopo). A comparação é por segmento do caminho.
pub fn scope_active(hovered: &HashSet<String>, scope: &str) -> bool {
    hovered.iter().any(|h| h == scope || h.strip_prefix(scope).is_some_and(|rest| rest.starts_with('/')))
}

/// O `hover` que vale para o nó: `hover` com `scope` (grupo entre lugares) fica para depois, e o nó segue sem hover.
fn own_hover(v: &Value) -> Option<&serde_json::Map<String, Value>> {
    v["hover"].as_object().filter(|h| !h.contains_key("scope"))
}

/// As props do nó com o `hover` aplicado quando o escopo está aceso. Sem hover aplicado, as props são as do nó, sem
/// cópia: um Raster carrega as células inteiras nelas.
pub fn hover_props(v: &Value, lit: bool) -> Cow<'_, Value> {
    let Some(hover) = own_hover(v).filter(|_| lit) else { return Cow::Borrowed(&v["props"]) };
    let mut p = if v["props"].is_object() { v["props"].clone() } else { Value::Object(Default::default()) };
    for (k, value) in hover { p[k] = value.clone(); }
    Cow::Owned(p)
}

/// O Box avisa o app do ponteiro: escopo com algum `hover` dentro, ou cartão absoluto (com ou sem o hover aplicado).
fn tracks_hover(v: &Value) -> bool {
    v["type"] == "Box" && ((is_scope(v) && wants_hover(v)) || hover_props(v, false)["position"] == "absolute"
        || hover_props(v, true)["position"] == "absolute")
}

/// Ids de trecho que a árvore desenhada no lugar pode ter no conjunto de hover.
fn tracked(site: &str, path: &mut String, v: &Value, out: &mut HashSet<String>) {
    if tracks_hover(v) { out.insert(format!("{site}{path}")); }
    for (i, k) in children(v).iter().enumerate() {
        let len = path.len();
        path.push_str(&format!("/{i}"));
        tracked(site, path, k, out);
        path.truncate(len);
    }
}

/// Depois de um evento novo, de uma troca de aba ou de fechar um painel, fica no conjunto só o trecho que ainda está
/// desenhado num dos lugares à vista (`(lugar, árvore)`): o gpui não avisa a saída do ponteiro de um trecho que sumiu
/// debaixo dele, e o hover ficaria preso.
pub fn keep_hovered(hovered: &mut HashSet<String>, places: &[(&str, &Value)]) {
    if hovered.is_empty() { return; }
    let mut drawn = HashSet::new();
    for (site, tree) in places { tracked(site, &mut String::new(), tree, &mut drawn); }
    hovered.retain(|h| drawn.contains(h));
}

/// Pinta o filho depois do resto da árvore, recortado onde ele está: é o `position: absolute` dos mods, que no terminal
/// fica por cima dos vizinhos sem sair do lugar. O gpui não tem z-index, e o `deferred` dele pinta sem recorte.
///
/// O recorte vai pelo `Clip`, que embrulha o filho: o `defer_draw` só aplica a máscara na pintura, e o prepaint adiado
/// roda sem máscara, então o hitbox do trecho cortado pegaria hover e clique fora do lugar (sobre o compositor ou o
/// cabeçalho do painel). A máscara é a do lugar, lida no prepaint do `OnTop` e passada ao `Clip` pela célula.
struct OnTop { clip: Option<AnyElement>, mask: Rc<std::cell::Cell<Option<ContentMask<Pixels>>>> }

impl OnTop {
    fn new(child: AnyElement) -> OnTop {
        let mask = Rc::new(std::cell::Cell::new(None));
        OnTop { clip: Some(Clip { child, mask: mask.clone() }.into_any_element()), mask }
    }
}

impl IntoElement for OnTop {
    type Element = Self;
    fn into_element(self) -> Self { self }
}

impl Element for OnTop {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> { None }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> { None }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App)
        -> (LayoutId, ()) {
        (self.clip.as_mut().expect("filho antes do prepaint").request_layout(window, cx), ())
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, _: &mut (),
        window: &mut Window, _: &mut App) {
        let clip = self.clip.take().expect("prepaint uma vez só");
        let (offset, mask) = (window.element_offset(), window.content_mask());
        self.mask.set(Some(mask));
        window.defer_draw(clip, offset, 1, Some(mask));
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, _: &mut (), _: &mut (),
        _: &mut Window, _: &mut App) {}
}

/// Roda o prepaint e a pintura do filho dentro da máscara do lugar: o hitbox nasce recortado, como o desenho.
struct Clip { child: AnyElement, mask: Rc<std::cell::Cell<Option<ContentMask<Pixels>>>> }

impl IntoElement for Clip {
    type Element = Self;
    fn into_element(self) -> Self { self }
}

impl Element for Clip {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> { None }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> { None }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App)
        -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, _: &mut (),
        window: &mut Window, cx: &mut App) {
        let child = &mut self.child;
        window.with_content_mask(self.mask.get(), |window| { child.prepaint(window, cx); });
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, _: &mut (), _: &mut (),
        window: &mut Window, cx: &mut App) {
        let child = &mut self.child;
        window.with_content_mask(self.mask.get(), |window| child.paint(window, cx));
    }
}

pub fn button_key(v: &Value) -> Option<String> {
    (v["type"] == "Button").then(|| v["props"]["key"].as_str().filter(|k| !k.is_empty()).map(str::to_owned)).flatten()
}

/// Só http(s) vira link, como no web: `javascript:` ou `file:` abririam o que o mod não deveria.
pub fn safe_href(v: &Value) -> Option<String> {
    v.as_str().filter(|h| h.starts_with("https://") || h.starts_with("http://")).map(str::to_owned)
}

/// Aviso (`$.ui.toast`) que um mod mostrou no terminal; `plugin` é o mod que o emitiu.
#[derive(Debug, PartialEq)]
pub struct Toast { pub id: String, pub text: String, pub plugin: String, pub timeout: Duration }

/// O dado do SSE `plugin_toast`; sem id, sem texto ou sem prazo não é aviso.
pub fn toast(data: &Value) -> Option<Toast> {
    let id = data["id"].as_str().filter(|id| !id.is_empty())?;
    let text = data["text"].as_str().filter(|text| !text.trim().is_empty())?;
    let ms = data["timeoutMs"].as_u64().filter(|ms| *ms > 0)?;
    Some(Toast { id: id.to_owned(), text: short(text), plugin: data["plugin"].as_str().unwrap_or("").to_owned(), timeout: Duration::from_millis(ms) })
}

/// A notificação cresce com o texto: aviso longo vira no máximo 4 linhas e 300 caracteres.
fn short(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: String = lines.iter().take(4).copied().collect::<Vec<_>>().join("\n");
    let mut cut = lines.len() > 4;
    if out.chars().count() > 300 { out = out.chars().take(300).collect(); cut = true; }
    if cut { out.push('…'); }
    out
}

/// De onde vem a interface dos mods: superfície remota (sessão sem terminal) ou o plugin no terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiSource { Surface, Terminal }

/// O evento `plugin_ui` lido. `shown_id`: `None` quando o servidor não manda (antigo), `Some(None)` sem painel.
/// `columns` e `source` ausentes ou estranhos valem como "o servidor não mandou".
#[derive(Debug, Default, PartialEq)]
pub struct Surfaces { pub above: Value, pub panes: Vec<Value>, pub shown_id: Option<Option<String>>, pub columns: Option<f64>, pub source: Option<UiSource> }

/// O dado do SSE `plugin_ui`, por valor: a árvore é movida, não copiada. Painel sem id fica de fora, como no web.
pub fn surfaces(mut data: Value) -> Surfaces {
    if !data.is_object() { return Surfaces::default(); }
    let shown_id = match data.get("shown_id") {
        Some(Value::String(id)) if !id.is_empty() => Some(Some(id.clone())),
        Some(Value::Null) => Some(None),
        _ => None,
    };
    let panes = match data["panes"].take() {
        Value::Array(panes) => panes.into_iter().filter(|p| p["id"].as_str().is_some_and(|id| !id.is_empty())).collect(),
        _ => Vec::new(),
    };
    Surfaces {
        above: data["above"].take(),
        panes,
        shown_id,
        columns: data["columns"].as_f64().filter(|c| c.is_finite() && *c > 0.),
        source: match data["source"].as_str() { Some("surface") => Some(UiSource::Surface), Some("terminal") => Some(UiSource::Terminal), _ => None },
    }
}

pub fn pane_ids(panes: &[Value]) -> Vec<String> { panes.iter().filter_map(|p| p["id"].as_str().map(str::to_owned)).collect() }

/// O servidor diz qual painel está na frente, e ele está na lista: a aba segue o servidor.
pub fn follows_server(ids: &[String], shown: &Option<Option<String>>) -> bool {
    matches!(shown, Some(Some(id)) if ids.contains(id))
}

/// O painel desenhado: o do servidor quando ele diz um da lista; senão a escolha local; senão o último aberto.
pub fn active_pane(ids: &[String], shown: &Option<Option<String>>, local: Option<&str>) -> Option<String> {
    if let Some(Some(id)) = shown.as_ref().filter(|_| follows_server(ids, shown)) { return Some(id.clone()); }
    local.filter(|l| ids.iter().any(|id| id == l)).map(str::to_owned).or_else(|| ids.last().cloned())
}

/// A escolha local depois de um evento novo. Painel que acabou de abrir vai para a frente, como no terminal; fechado o
/// escolhido, fica o vizinho anterior (o seguinte, se não houver anterior); senão ela sobrevive ao redesenho.
pub fn follow_local(prev: &[String], next: &[String], local: Option<&str>) -> Option<String> {
    if next.is_empty() { return None; }
    if let Some(opened) = next.iter().rev().find(|id| !prev.contains(id)) { return Some(opened.clone()); }
    if let Some(local) = local.filter(|l| next.iter().any(|id| id == l)) { return Some(local.to_owned()); }
    let Some(at) = local.and_then(|l| prev.iter().position(|id| id == l)) else { return next.last().cloned() };
    prev[..at].iter().rev().find(|id| next.contains(id)).or_else(|| next.first()).cloned()
}

fn frame() -> Div {
    div().px(px(10.)).py(px(6.)).rounded(px(8.)).bg(theme::inset())
        .font_family(theme::MONO).text_size(px(TEXT_PX)).line_height(px(CELL_H)).text_color(theme::text())
}

pub fn band(tree: &Value, view: &View) -> Option<AnyElement> {
    if is_empty(tree) { return None; }
    let c = Ctx { site: BAND_SITE, view, place: view.columns, links: Default::default() };
    Some(frame().w_full().mb(px(4.)).overflow_hidden().child(node(tree, &c, &Spot::root())).into_any_element())
}

/// O `✕` do lugar: fecha o painel da frente, como a marca do engine no terminal.
fn close_mark(site: &str, view: &View) -> Option<AnyElement> {
    let press = view.press.clone()?;
    let site = site.to_owned();
    Some(div().id(SharedString::from(format!("plg-close-{site}"))).flex_shrink_0().cursor_pointer().px(px(4.))
        .text_color(theme::muted()).child("✕")
        .on_click(move |_, window, cx| press(&site, PANE_CLOSE_KEY, window, cx)).into_any_element())
}

/// Fileira das abas: um título por painel, o ativo em destaque, e um `✕` só à direita.
fn tabs(panes: &[Value], active: &str, view: &View) -> AnyElement {
    let row = div().flex().flex_row().items_center().gap_1().min_w_0().overflow_hidden()
        .children(panes.iter().map(|p| {
            let id = p["id"].as_str().unwrap_or("").to_owned();
            let title = p["title"].as_str().filter(|t| !t.is_empty()).unwrap_or(&id).to_owned();
            let on = id == active;
            let tab = div().id(SharedString::from(format!("plg-tab-{id}"))).flex_shrink_0().px(px(CELL_W)).rounded(px(4.))
                .when(on, |el| el.bg(theme::raised()).font_weight(FontWeight::SEMIBOLD))
                .when(!on, |el| el.text_color(theme::muted()).cursor_pointer())
                .child(title);
            match view.show.clone().filter(|_| !on) {
                Some(show) => tab.on_click(move |_, window, cx| show(&id, window, cx)).into_any_element(),
                None => tab.into_any_element(),
            }
        }));
    div().flex().items_center().justify_between().gap_2().child(row).children(close_mark(active, view)).into_any_element()
}

/// Os painéis dos mods, acima da faixa como o terminal os abre. Com mais de um, uma fileira de abas e só o ativo
/// desenhado; com um só, título e `✕`. O corpo rola dentro de `max_h`.
pub fn panes(panes: &[Value], active: Option<&str>, view: &View, max_h: f32) -> Option<AnyElement> {
    let pane = panes.iter().find(|p| p["id"].as_str() == active)?;
    let id = pane["id"].as_str().unwrap_or("").to_owned();
    let header = if panes.len() > 1 { tabs(panes, &id, view) } else {
        let title = pane["title"].as_str().filter(|t| !t.is_empty()).unwrap_or(&id).to_owned();
        div().flex().items_center().justify_between().gap_2()
            .child(div().min_w_0().truncate().font_weight(FontWeight::SEMIBOLD).child(title))
            .children(close_mark(&id, view)).into_any_element()
    };
    let c = Ctx { site: &id, view, place: pane["columns"].as_f64(), links: Default::default() };
    // Recorta o que passa da largura: no gpui, filho maior que a coluna desenha por cima do vizinho.
    Some(frame().flex().flex_col().gap_1().min_h_0().max_h(px(max_h)).w_full().overflow_hidden()
        .child(header)
        .child(div().id(SharedString::from(format!("plg-body-{id}"))).flex_1().min_h_0().overflow_y_scroll()
            .child(node(&pane["tree"], &c, &Spot::root())))
        .into_any_element())
}

fn node(v: &Value, c: &Ctx, at: &Spot) -> AnyElement {
    match v {
        Value::String(s) => div().flex_shrink_0().child(s.clone()).into_any_element(),
        Value::Number(n) => div().flex_shrink_0().child(n.to_string()).into_any_element(),
        Value::Object(_) => element(v, c, at),
        _ => div().into_any_element(),
    }
}

fn text_of(v: &Value) -> String { v.as_str().unwrap_or("").to_owned() }

fn children(v: &Value) -> &[Value] { v["children"].as_array().map(Vec::as_slice).unwrap_or(&[]) }

/// Texto direto dos filhos, para rótulos de botão e link.
fn plain(v: &Value) -> String {
    children(v).iter().map(|c| match c { Value::String(s) => s.clone(), Value::Number(n) => n.to_string(), _ => String::new() }).collect()
}

fn element(v: &Value, c: &Ctx, at: &Spot) -> AnyElement {
    // Box com `key` é escopo: o hover dele e o dos filhos valem com o ponteiro nele; os outros seguem o de cima.
    let lit = if is_scope(v) { scope_active(c.view.hovered, &c.spot(at)) } else { at.lit };
    let props = hover_props(v, lit);
    let p: &Value = &props;
    match v["type"].as_str().unwrap_or("") {
        "Box" => boxed(v, p, c, at, lit),
        "Text" => text(p, children(v), c, at),
        "Raster" => raster(p),
        "Svg" => svg(p),
        "Markdown" => div().whitespace_normal().when(p["dimColor"] == true, |el| el.opacity(0.6))
            .child(unmark(&text_of(&p["text"]))).into_any_element(),
        "Code" => div().whitespace_normal().text_color(theme::muted()).child(text_of(&p["source"])).into_any_element(),
        "Link" => {
            let label = Some(text_of(&p["label"])).filter(|s| !s.is_empty()).unwrap_or_else(|| plain(v));
            let shown = if label.is_empty() { text_of(&p["href"]) } else { label };
            let base = div().text_color(theme::accent()).underline();
            match safe_href(&p["href"]) {
                Some(href) => {
                    let n = c.links.get();
                    c.links.set(n + 1);
                    base.id(SharedString::from(format!("lnk-{}-{n}", c.site))).cursor_pointer()
                        .on_click(move |_, _, cx| cx.open_url(&href)).child(shown).into_any_element()
                }
                None => base.child(shown).into_any_element(),
            }
        }
        "Button" => {
            let label = Some(text_of(&p["label"])).filter(|s| !s.is_empty()).unwrap_or_else(|| plain(v));
            // `plain` sai como no terminal: texto, sem pílula.
            let base = div().flex_shrink_0()
                .when(p["plain"] != true, |el| el.px(px(CELL_W)).rounded(px(4.)).bg(theme::raised()))
                .when(p["dimColor"] == true, |el| el.opacity(0.6))
                .when(p["variant"] == "primary", |el| el.text_color(theme::accent()))
                // Estilo do rótulo como no web, das props ou do `hover` do escopo; o fundo vence o da pílula.
                .when_some(color(&p["color"]), |el, fg| el.text_color(fg))
                .when_some(color(&p["backgroundColor"]), |el, bg| el.bg(bg))
                .when(p["bold"] == true, |el| el.font_weight(FontWeight::BOLD))
                .when(p["italic"] == true, |el| el.italic())
                .when(p["underline"] == true, |el| el.underline())
                .when(p["strikethrough"] == true, |el| el.line_through());
            match (button_key(v), c.view.press.clone()) {
                (Some(key), Some(press)) => {
                    let site = c.site.to_owned();
                    base.id(SharedString::from(format!("plg-{site}-{key}"))).cursor_pointer()
                        .hover(|el| el.underline())
                        .on_click(move |_, window, cx| press(&site, &key, window, cx))
                        .child(label).into_any_element()
                }
                _ => base.child(label).into_any_element(),
            }
        }
        "Image" => div().text_color(theme::muted()).child(text_of(&p["alt"])).into_any_element(),
        _ => div().flex().children(children(v).iter().enumerate().map(|(i, k)| node(k, c, &at.child(i, lit)))).into_any_element(),
    }
}

/// `width` em colunas que alcança a largura do lugar ocupa o lugar inteiro: o mod desenhou para a coluna do terminal, e
/// o app pode ser mais largo. Menor continua teto. Sem a largura do lugar (servidor antigo), sempre teto.
pub fn fills_place(width: Option<f64>, place: Option<f64>) -> bool {
    matches!((width, place), (Some(w), Some(p)) if p > 0. && w >= p)
}

fn cols(v: &Value) -> Option<f32> { v.as_f64().map(|n| n as f32 * CELL_W) }
fn lines(v: &Value) -> Option<f32> { v.as_f64().map(|n| n as f32 * CELL_H) }

/// O primeiro número entre as chaves, na ordem: a mais específica vence (`paddingLeft` > `paddingX` > `padding`).
fn first(p: &Value, keys: &[&str]) -> Value { keys.iter().map(|k| p[*k].clone()).find(Value::is_number).unwrap_or(Value::Null) }

/// `Box` do Ink em flexbox do gpui. O padrão do Ink é linha, não coluna.
fn boxed(v: &Value, p: &Value, c: &Ctx, at: &Spot, lit: bool) -> AnyElement {
    if p["display"] == "none" { return div().into_any_element(); }
    let kids = children(v);
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
    // Largura fixa do terminal vira teto (a coluna da conversa pode ser mais estreita que o pane), salvo quando ela
    // alcança a largura do lugar: aí o mod quis a linha inteira.
    if let Some(w) = cols(&p["width"]) {
        el = if fills_place(p["width"].as_f64(), c.place) { el.w_full() } else { el.w_full().max_w(px(w)) };
    }
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
    // `absolute` sai do fluxo; deslocamento em células, negativo permitido.
    let absolute = p["position"] == "absolute";
    if absolute {
        // Por cima, o cartão fica com o ponteiro, como o `z-index` do web: hover e clique não chegam ao que está embaixo
        // (o escopo dono segue aceso pelo id do cartão). A rolagem passa.
        el = el.absolute().block_mouse_except_scroll();
        if let Some(n) = lines(&p["top"]) { el = el.top(px(n)); }
        if let Some(n) = lines(&p["bottom"]) { el = el.bottom(px(n)); }
        if let Some(n) = cols(&p["left"]) { el = el.left(px(n)); }
        if let Some(n) = cols(&p["right"]) { el = el.right(px(n)); }
    }
    // Linha de texto logo abaixo de um Raster (os rótulos sob os traços da barra de progresso)
    // segue a escala dele; sem isso o Raster cabe na coluna estreita e os rótulos saem do lugar.
    // Linha com hover fica no desenho comum, o único que aplica o hover.
    let el = el.children(kids.iter().enumerate().map(|(i, k)| {
        let at = at.child(i, lit);
        match i.checked_sub(1).filter(|_| text_row(k) && !wants_hover(k)).and_then(|j| raster_row(&kids[j])) {
            Some(frame) => aligned_row(k, frame, c, &at),
            None => node(k, c, &at),
        }
    }));
    // Escopo e cartão absoluto avisam o app do ponteiro: o cartão conta como parte do escopo que o contém.
    let id = c.spot(at);
    let el = match c.view.hover.clone().filter(|_| absolute || (is_scope(v) && wants_hover(v))) {
        Some(hover) => el.id(SharedString::from(format!("plg-hv-{id}")))
            .on_hover(move |on, window, cx| hover(&id, *on, window, cx)).into_any_element(),
        None => el.into_any_element(),
    };
    if absolute { OnTop::new(el).into_any_element() } else { el }
}

/// Props do `Box` que o `boxed` desenha e o molde do Raster não reproduz: com qualquer uma, a
/// linha segue o desenho comum em vez de perder o recuo, o espaçamento ou o fundo.
const BOX_LAYOUT: &[&str] = &[
    "justifyContent", "alignItems", "flexGrow", "flexWrap", "width", "minWidth", "gap", "columnGap", "rowGap",
    "padding", "paddingX", "paddingY", "paddingTop", "paddingBottom", "paddingLeft", "paddingRight",
    "margin", "marginX", "marginY", "marginTop", "marginBottom", "marginLeft", "marginRight",
    "backgroundColor", "borderStyle", "overflow", "position",
];

/// `Box` em linha sem nada além dos filhos: cada filho ocupa as suas células, como no terminal.
fn plain_row(v: &Value) -> bool {
    let p = &v["props"];
    v["type"] == "Box" && matches!(p["flexDirection"].as_str(), None | Some("row"))
        && BOX_LAYOUT.iter().all(|k| p[*k].is_null()) && p["display"] != "none"
        && !children(v).is_empty()
}

/// Texto de um `Text` sem corte e sem texto aninhado, o único que se mede em células.
fn flat_text(v: &Value) -> Option<String> {
    let flat = v["type"] == "Text" && !v["props"]["wrap"].is_string()
        && children(v).iter().all(|k| k.is_string() || k.is_number());
    flat.then(|| plain(v))
}

fn text_cells(v: &Value) -> Option<usize> { flat_text(v).map(|t| t.chars().count()) }

fn text_row(v: &Value) -> bool { plain_row(v) && children(v).iter().all(|k| flat_text(k).is_some()) }

/// Molde de uma linha com Raster: células de texto antes, colunas do Raster e o texto depois.
struct RasterFrame<'a> { before: usize, columns: usize, after: &'a [Value] }

fn raster_row(v: &Value) -> Option<RasterFrame<'_>> {
    if !plain_row(v) { return None; }
    let kids = children(v);
    let at = kids.iter().position(|k| k["type"] == "Raster")?;
    let (before, after) = (&kids[..at], &kids[at + 1..]);
    if after.iter().any(|k| flat_text(k).is_none()) { return None; }
    let columns = kids[at]["props"]["columns"].as_u64().filter(|&n| n > 0)? as usize;
    Some(RasterFrame { before: before.iter().map(text_cells).sum::<Option<usize>>()?, columns, after })
}

/// Trilho do Raster: ocupa o que sobra da linha até a largura natural das colunas. O Raster e a
/// linha alinhada a ele usam o mesmo, para encolherem juntos.
fn raster_track(columns: usize) -> Div {
    div().flex().flex_basis(px(0.)).flex_grow(1.).min_w_0().max_w(px(columns as f32 * CELL_W))
}

/// Monta a linha de texto no molde do Raster de cima: o começo com a largura natural, o trecho
/// sob o Raster na mesma escala dele e, no fim, o texto de depois invisível, só para ocupar o
/// mesmo espaço. No trecho escalado cada palavra corta onde começa a próxima, não antes.
fn aligned_row(row: &Value, frame: RasterFrame, c: &Ctx, at: &Spot) -> AnyElement {
    let piece = |k: &Value, t: &[char]| text(&k["props"], &[Value::from(t.iter().collect::<String>())], c, at);
    let (mut head, mut words): (Vec<AnyElement>, Vec<(Vec<AnyElement>, usize)>) = (Vec::new(), Vec::new());
    let mut seen = 0;
    for k in children(row) {
        let chars: Vec<char> = plain(k).chars().collect();
        let cut = frame.before.saturating_sub(seen).min(chars.len());
        seen += chars.len();
        if cut > 0 { head.push(piece(k, &chars[..cut])); }
        let rest = &chars[cut..];
        if rest.is_empty() { continue; }
        match words.last_mut() {
            Some((els, cells)) if rest.iter().all(|ch| ch.is_whitespace()) => { els.push(piece(k, rest)); *cells += rest.len(); }
            _ => words.push((vec![piece(k, rest)], rest.len())),
        }
    }
    let last = words.len().saturating_sub(1);
    let scaled = raster_track(frame.columns).flex_row()
        .children(words.into_iter().enumerate().map(|(i, (els, cells))| {
            div().flex().flex_row().flex_shrink_0().whitespace_nowrap().w(relative(cells as f32 / frame.columns as f32))
                .when(i < last, |el| el.overflow_hidden())
                .children(els)
        }));
    div().flex().flex_row().min_w_0()
        .children(head)
        .child(scaled)
        .child(div().flex().flex_row().flex_shrink_0().opacity(0.).children(frame.after.iter().enumerate().map(|(i, n)| node(n, c, &at.child(i, at.lit)))))
        .into_any_element()
}

/// `Text` do Ink: cor, ênfase e corte. `dimColor` é opacidade, como no terminal.
fn text(p: &Value, kids: &[Value], c: &Ctx, at: &Spot) -> AnyElement {
    let fg = color(&p["color"]);
    let bg = color(&p["backgroundColor"]);
    let (fg, bg) = if p["inverse"] == true { (bg.or(Some(theme::background())), fg.or(Some(theme::text()))) } else { (fg, bg) };
    let truncate = p["wrap"].as_str().is_some_and(|w| w.starts_with("truncate") || w == "end" || w == "middle");
    // Texto dentro de texto vira trechos lado a lado: o gpui não tem span em linha.
    let el = div().flex().flex_row().min_w_0()
        .when(!truncate, |el| el.flex_shrink_0())
        .when_some(fg, |el, c| el.text_color(c))
        .when_some(bg, |el, c| el.bg(c))
        .when(p["bold"] == true, |el| el.font_weight(FontWeight::BOLD))
        .when(p["italic"] == true, |el| el.italic())
        .when(p["underline"] == true, |el| el.underline())
        .when(p["strikethrough"] == true, |el| el.line_through())
        .when(p["dimColor"] == true, |el| el.opacity(0.6))
        .when(truncate, |el| el.overflow_hidden().whitespace_nowrap());
    // Juntar em texto puro tiraria o clique de link e botão: com eles, a linha cortada só recorta, sem reticências.
    if let Some(text) = truncate.then(|| plain_deep(kids)).flatten() {
        return el.child(div().truncate().child(text)).into_any_element();
    }
    el.children(kids.iter().enumerate().map(|(i, k)| node(k, c, &at.child(i, at.lit)))).into_any_element()
}

/// Texto de uma subárvore inteira, para o corte com reticências que o gpui só faz num texto só; `None` quando ela tem
/// algo que se clica.
fn plain_deep(kids: &[Value]) -> Option<String> {
    kids.iter().map(|c| match c {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Object(_) if matches!(c["type"].as_str(), Some("Link" | "Button")) => None,
        Value::Object(_) => plain_deep(children(c)),
        _ => Some(String::new()),
    }).collect()
}

/// Células do `Raster`: base64 de triplas u32 little-endian `[código, frente, fundo]`, por linha.
/// Vizinhas da mesma cor viram um trecho só. O Raster vem com a largura do pane do terminal: em
/// coluna mais estreita cada trecho encolhe na proporção das suas células, sem rolar de lado.
fn raster(p: &Value) -> AnyElement {
    let columns = p["columns"].as_u64().unwrap_or(0) as usize;
    let mut grid = raster_track(columns).flex_col();
    for runs in raster_runs(p) {
        grid = grid.child(div().flex().flex_row().w_full().min_w_0().whitespace_nowrap().children(runs.into_iter().map(|(t, fg, bg)| {
            div().flex_basis(px(0.)).flex_grow(t.chars().count() as f32).flex_shrink(1.).min_w_0().overflow_hidden()
                .when_some(fg, |el, c| el.text_color(c)).when_some(bg, |el, c| el.bg(c)).child(t)
        })));
    }
    grid.into_any_element()
}

type Run = (String, Option<Hsla>, Option<Hsla>);

/// Trechos de cada linha do Raster. `rows` vem do mod: sem o teto pelos bytes que chegaram, um
/// número enorme prende a thread de desenho criando linha vazia.
fn raster_runs(p: &Value) -> Vec<Vec<Run>> {
    use base64::Engine as _;
    let columns = p["columns"].as_u64().unwrap_or(0) as usize;
    let bytes = base64::engine::general_purpose::STANDARD.decode(p["cells"].as_str().unwrap_or("")).unwrap_or_default();
    let cells = bytes.len() / 12;
    let rows = if columns == 0 { 0 } else { (p["rows"].as_u64().unwrap_or(0) as usize).min(cells.div_ceil(columns)) };
    let word = |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    (0..rows).map(|r| {
        let mut runs: Vec<Run> = Vec::new();
        for i in (r * columns..).take(columns).take_while(|&i| i < cells) {
            let at = i * 12;
            let ch = char::from_u32(word(at)).filter(|c| !c.is_control()).unwrap_or(' ');
            let (fg, bg) = (cell_color(word(at + 4)), cell_color(word(at + 8)));
            match runs.last_mut() {
                Some(run) if run.1 == fg && run.2 == bg => run.0.push(ch),
                _ => runs.push((ch.to_string(), fg, bg)),
            }
        }
        runs
    }).collect()
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
    // Importação explícita: `super::*` traz o `test` do gpui_kit, e o `#[test]` passaria a ser o dele.
    use super::{active_pane, button_key, cell_color, color, fills_place, follow_local, follows_server, hover_props, is_empty,
        keep_hovered, pane_ids, plain_deep, raster_row, raster_runs, safe_href, scope_active, surfaces, text_row, toast, wants_hover,
        Surfaces, Toast, UiSource};
    use serde_json::{json, Value};
    use std::borrow::Cow;
    use std::collections::HashSet;
    use std::time::Duration;

    #[test]
    fn toast_keeps_the_mod_and_its_timeout_and_refuses_what_is_not_a_toast() {
        assert_eq!(toast(&json!({"id": "ab-1", "text": "Jenkins configurado.", "plugin": "demo", "timeoutMs": 9000})),
            Some(Toast { id: "ab-1".into(), text: "Jenkins configurado.".into(), plugin: "demo".into(), timeout: Duration::from_millis(9000) }));
        assert_eq!(toast(&json!({"id": "ab-2", "text": "oi", "timeoutMs": 1})).map(|t| t.plugin), Some(String::new()));
        assert_eq!(toast(&json!({"text": "oi", "timeoutMs": 4000})), None);
        assert_eq!(toast(&json!({"id": "ab-3", "text": "  ", "timeoutMs": 4000})), None);
        assert_eq!(toast(&json!({"id": "ab-4", "text": "oi"})), None);
        let long = toast(&json!({"id": "ab-5", "text": "x".repeat(2000), "timeoutMs": 1})).unwrap().text;
        assert_eq!((long.chars().count(), long.ends_with('…')), (301, true));
        assert_eq!(toast(&json!({"id": "ab-6", "text": "1\n2\n3\n4\n5", "timeoutMs": 1})).unwrap().text, "1\n2\n3\n4…");
    }

    fn cells(words: &[u32]) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(words.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>())
    }

    #[test]
    fn raster_never_builds_rows_beyond_the_cells_that_arrived() {
        let three = cells(&[0x41, 0x0100_0000, 0x0100_0000, 0x42, 0x0100_0000, 0x0100_0000, 0x43, 0x0100_0000, 0x0100_0000]);
        let runs = raster_runs(&json!({"columns": 2, "rows": 4_000_000_000_000u64, "cells": three}));
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0][0].0, "AB");
        assert_eq!(runs[1][0].0, "C");
        assert!(raster_runs(&json!({"columns": 0, "rows": 1_000_000, "cells": three})).is_empty());
        assert!(raster_runs(&json!({"columns": u64::MAX, "rows": u64::MAX, "cells": three}))[0].len() == 1);
    }

    #[test]
    fn label_row_follows_the_raster_frame_and_only_plain_text_counts() {
        let text = |s: &str| json!({"type": "Text", "children": [s]});
        let bar = json!({"type": "Box", "props": {"flexDirection": "row"}, "children": [
            text("  "), {"type": "Raster", "props": {"columns": 20, "rows": 1, "cells": ""}}, text("  29%")]});
        let frame = raster_row(&bar).unwrap();
        assert_eq!((frame.before, frame.columns, frame.after.len()), (2, 20, 1));
        let labels = json!({"type": "Box", "children": [text("  "), {"type": "Text", "props": {"bold": true}, "children": ["Correção"]}, text("   Entrega")]});
        assert!(text_row(&labels) && raster_row(&labels).is_none());
        // Largura, distribuição, corte e texto aninhado não são células: seguem o desenho comum.
        assert!(!text_row(&json!({"type": "Box", "props": {"width": 30}, "children": [text("a")]})));
        assert!(!text_row(&json!({"type": "Box", "props": {"justifyContent": "space-between"}, "children": [text("a")]})));
        assert!(!text_row(&json!({"type": "Box", "children": [{"type": "Text", "props": {"wrap": "truncate-end"}, "children": ["a"]}]})));
        assert!(!text_row(&json!({"type": "Box", "children": [{"type": "Text", "children": [text("a")]}]})));
        assert!(!text_row(&json!({"type": "Box", "children": []})));
        // Recuo, espaçamento e fundo o molde não reproduz: a linha fica no desenho comum.
        for prop in ["paddingLeft", "marginLeft", "gap", "columnGap", "backgroundColor", "borderStyle"] {
            let row = json!({"type": "Box", "props": {prop: 1}, "children": [text("a")]});
            assert!(!text_row(&row), "{prop}");
            let bar = json!({"type": "Box", "props": {prop: 1}, "children": [text("  "), {"type": "Raster", "props": {"columns": 20}}]});
            assert!(raster_row(&bar).is_none(), "{prop}");
        }
        assert!(raster_row(&json!({"type": "Box", "children": [{"type": "Raster", "props": {"columns": 0}}]})).is_none());
    }

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
    fn button_key_reads_only_buttons() {
        assert_eq!(button_key(&json!({"type": "Button", "props": {"key": "cp-1"}})), Some("cp-1".to_owned()));
        assert_eq!(button_key(&json!({"type": "Button", "props": {}})), None);
        assert_eq!(button_key(&json!({"type": "Text", "props": {"key": "x"}})), None);
    }

    #[test]
    fn only_http_links_open() {
        assert_eq!(safe_href(&json!("https://gitlab.exemplo/mr/1")), Some("https://gitlab.exemplo/mr/1".to_owned()));
        assert_eq!(safe_href(&json!("javascript:alert(1)")), None);
        assert_eq!(safe_href(&json!("file:///etc/passwd")), None);
    }

    #[test]
    fn cut_text_keeps_links_and_buttons_clickable() {
        let link = json!({"type": "Link", "props": {"href": "https://gitlab.exemplo/pm/PM-1"}, "children": ["PM-1"]});
        assert_eq!(plain_deep(&[json!({"type": "Text", "children": ["PM ", link]})]), None);
        assert_eq!(plain_deep(&[json!({"type": "Button", "props": {"key": "k"}})]), None);
        assert_eq!(plain_deep(&[json!("texto "), json!({"type": "Text", "children": ["só texto"]})]).as_deref(), Some("texto só texto"));
    }

    #[test]
    fn colors_accept_hex_and_ink_names() {
        assert!(color(&json!("#5aa6ff")).is_some());
        assert!(color(&json!("redBright")).is_some());
        assert_eq!(color(&json!("nope")), None);
    }

    fn amostras() -> Value { serde_json::from_str(include_str!("../../packages/core/src/__fixtures__/plugin-ui-arvores.json")).unwrap() }

    #[test]
    fn plugin_ui_event_reads_the_new_fields_and_tolerates_their_absence() {
        let rol = amostras()["rolPm"]["panes"].clone();
        let old = surfaces(json!({"above": amostras()["faixaPm"], "panes": rol}));
        assert_eq!(pane_ids(&old.panes), ["pm-mock-pm", "pm-mock-mr", "pm-mock-jenkins"]);
        assert_eq!((old.shown_id, old.columns, old.source), (None, None, None));
        let new = surfaces(json!({"above": null, "panes": [{"title": "sem id"}], "shown_id": null, "columns": 110, "source": "surface"}));
        assert!(new.panes.is_empty());
        assert_eq!((new.shown_id, new.columns, new.source), (Some(None), Some(110.), Some(UiSource::Surface)));
        let odd = surfaces(json!({"shown_id": 7, "columns": -1, "source": "mobile"}));
        assert_eq!((odd.shown_id, odd.columns, odd.source), (None, None, None));
        assert_eq!(surfaces(json!("texto")), Surfaces::default());
    }

    #[test]
    fn active_pane_follows_the_server_only_when_it_names_a_pane_in_the_list() {
        let ids: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        assert_eq!(active_pane(&ids, &Some(Some("a".into())), Some("b")).as_deref(), Some("a"));
        // shown_id de painel que ainda não chegou: vale a escolha local, nunca o corpo vazio.
        assert_eq!(active_pane(&ids, &Some(Some("z".into())), Some("b")).as_deref(), Some("b"));
        assert_eq!(active_pane(&ids, &None, None).as_deref(), Some("c"));
        assert_eq!(active_pane(&[], &Some(None), Some("a")), None);
        assert!(follows_server(&ids, &Some(Some("c".into()))));
        assert!(!follows_server(&ids, &None) && !follows_server(&ids, &Some(None)));
    }

    #[test]
    fn width_that_reaches_the_place_fills_it_and_a_missing_place_keeps_the_cap() {
        assert!(fills_place(Some(110.), Some(110.)) && fills_place(Some(120.), Some(110.)));
        assert!(!fills_place(Some(24.), Some(58.)));
        // Servidor de hoje, sem `columns`: continua teto, nada vira 100% por engano.
        assert!(!fills_place(Some(110.), None) && !fills_place(None, Some(110.)) && !fills_place(Some(5.), Some(0.)));
    }

    #[test]
    fn local_tab_starts_on_the_newest_survives_redraws_and_falls_back_to_the_previous_neighbour() {
        let v = |ids: &[&str]| ids.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(follow_local(&[], &v(&["a", "b", "c"]), None).as_deref(), Some("c"));
        assert_eq!(follow_local(&v(&["a", "b", "c"]), &v(&["a", "b", "c"]), Some("a")).as_deref(), Some("a"));
        assert_eq!(follow_local(&v(&["a", "b"]), &v(&["a", "b", "d"]), Some("a")).as_deref(), Some("d"));
        assert_eq!(follow_local(&v(&["a", "b", "c"]), &v(&["a", "c"]), Some("b")).as_deref(), Some("a"));
        assert_eq!(follow_local(&v(&["a", "b"]), &v(&["b"]), Some("a")).as_deref(), Some("b"));
        assert_eq!(follow_local(&v(&["a"]), &[], Some("a")), None);
    }

    #[test]
    fn hover_scope_is_lit_by_itself_or_by_an_absolute_box_inside_it() {
        let card: HashSet<String> = ["above-prompt/1/2".to_owned()].into();
        assert!(scope_active(&card, "above-prompt/1"));
        assert!(scope_active(&card, "above-prompt/1/2"));
        assert!(!scope_active(&card, "above-prompt/1/2/0"));
        // O caminho é por segmento: "/12" não acende "/1".
        let other: HashSet<String> = ["above-prompt/12".to_owned()].into();
        assert!(!scope_active(&other, "above-prompt/1"));
    }

    #[test]
    fn hover_props_apply_only_when_lit_and_skip_cross_place_scopes() {
        let card = amostras()["hoverV29"]["children"][1].clone();
        assert_eq!(hover_props(&card, false)["display"], "none");
        assert_eq!(hover_props(&card, true)["display"], "flex");
        assert_eq!(hover_props(&card, true)["position"], "absolute");
        let v30 = json!({"type": "Text", "hover": {"scope": "vitrine-V30", "color": "#e8a33d"}});
        assert!(hover_props(&v30, true)["color"].is_null());
        assert!(wants_hover(&amostras()["hoverV29"]) && wants_hover(&amostras()["hoverV28"]));
        assert!(!wants_hover(&json!({"type": "Box", "props": {"key": "k"}, "children": [{"type": "Text", "children": ["a"]}]})));
    }

    #[test]
    fn hover_props_copy_the_props_only_when_a_hover_applies() {
        let card = amostras()["hoverV29"]["children"][1].clone();
        assert!(matches!(hover_props(&card, false), Cow::Borrowed(_)));
        assert!(matches!(hover_props(&card, true), Cow::Owned(_)));
        let v30 = json!({"type": "Text", "props": {"bold": true}, "hover": {"scope": "vitrine-V30", "color": "#e8a33d"}});
        assert!(matches!(hover_props(&v30, true), Cow::Borrowed(_)));
        // Hover aplicado sobre nó sem props: as props nascem só com o hover.
        assert_eq!(*hover_props(&json!({"type": "Text", "hover": {"bold": true}}), true), json!({"bold": true}));
    }

    #[test]
    fn hovered_pieces_survive_a_redraw_only_while_they_are_still_drawn_in_a_shown_place() {
        let v29 = amostras()["hoverV29"].clone();
        let (scope, card) = ("above-prompt".to_owned(), "above-prompt/1".to_owned());
        let lit = || -> HashSet<String> { [scope.clone(), card.clone(), "painel/0".to_owned()].into() };
        // Mesma árvore na faixa, painel fora da tela (outra aba): fica só o que a faixa desenha.
        let mut hovered = lit();
        keep_hovered(&mut hovered, &[("above-prompt", &v29)]);
        assert_eq!(hovered, [scope.clone(), card.clone()].into());
        // O cartão saiu da árvore debaixo do ponteiro: o id dele sai junto e o escopo segue aceso.
        let without_card = json!({"type": "Box", "props": {"key": "V29-escopo"}, "hover": {"borderColor": "#5aa6ff"}, "children": []});
        let mut hovered = lit();
        keep_hovered(&mut hovered, &[("above-prompt", &without_card)]);
        assert_eq!(hovered, [scope.clone()].into());
        // Escopo sem nenhum hover não avisa o app do ponteiro: não pode ficar no conjunto.
        let mut hovered = lit();
        keep_hovered(&mut hovered, &[("above-prompt", &json!({"type": "Box", "props": {"key": "k"}, "children": []}))]);
        assert!(hovered.is_empty());
    }
}
