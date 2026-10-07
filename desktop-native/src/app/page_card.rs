//! Página publicada pelo agente desenhada dentro da conversa: viva no Linux, imagem estática nos outros.
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, VecDeque},
    rc::{Rc, Weak},
};

use serde::Deserialize;

use super::*;
use crate::browser::{Engine, Pointer};

pub const LIVE_MAX: usize = 4;
const MIN: u32 = 80;
const MAX: u32 = 2000;
const FALLBACK: f32 = 240.;
/// Clique ou tecla na página que ainda vale como gesto para ela abrir um link.
const GESTURE: Duration = Duration::from_secs(2);
/// Espera do último quadro da página que sai do orçamento.
#[cfg(target_os = "linux")]
const PARK_SHOT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct PageRef { pub id: String, pub title: String, #[serde(default)] pub height: Option<u32>, #[serde(default)] pub heights: BTreeMap<u32, u32> }

/// O resultado de MCP chega como texto ou como lista de blocos `{type: "text", text}`: os blocos são juntados antes.
pub fn page_from_result(tool_name: &str, result: &str) -> Option<PageRef> {
    if !conversation::is_page_call(Some(tool_name)) { return None; }
    #[derive(Deserialize)]
    struct Out { hangar_page: PageRef }
    let value: Value = serde_json::from_str(result).ok()?;
    let value = match value {
        Value::Array(blocks) => serde_json::from_str(&blocks.iter()
            .filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<String>()).ok()?,
        other => other,
    };
    serde_json::from_value::<Out>(value).ok().map(|o| o.hangar_page)
}

fn natural(page: &PageRef, width: f32, reported: Option<f32>) -> f32 {
    reported.unwrap_or_else(|| {
        page.heights.iter().filter(|(_, h)| **h > 0)
            .min_by_key(|(w, _)| (**w as f32 - width).abs() as u32).map_or(FALLBACK, |(_, h)| *h as f32)
    })
}

pub fn frame_height(page: &PageRef, width: f32, reported: Option<f32>) -> f32 {
    let natural = natural(page, width, reported);
    let capped = page.height.map_or(natural, |h| natural.min(h as f32));
    capped.clamp(MIN as f32, MAX as f32)
}

/// A página rola dentro do quadro (o agente limitou a altura ou ela passa do teto): só então a roda é dela.
fn scrolls_inside(page: &PageRef, width: f32, reported: Option<f32>) -> bool {
    natural(page, width, reported) > frame_height(page, width, reported) + 0.5
}

/// Fila das páginas vivas, mais recente no fim: passou do teto, a mais antiga sai.
#[derive(Default)]
pub struct Budget { order: VecDeque<String> }

impl Budget {
    /// Marca `id` como usada agora; devolve quem deve fechar.
    pub fn touch(&mut self, id: &str) -> Option<String> {
        self.order.retain(|x| x != id);
        self.order.push_back(id.to_owned());
        (self.order.len() > LIVE_MAX).then(|| self.order.pop_front()).flatten()
    }
}

#[derive(Debug, PartialEq)]
pub enum HostMsg { Height(f32), Link(String) }

/// Mensagem da página (`window.hangarHost`). Entrada não confiável: os iframes http(s) permitidos também chamam a
/// ponte. Só os dois métodos, altura finita e positiva, link só http(s).
pub fn parse_host(raw: &str) -> Option<HostMsg> {
    let message: Value = serde_json::from_str(raw).ok()?;
    let params = &message["params"];
    match message["method"].as_str()? {
        "ui/notifications/size-changed" => params["height"].as_f64().filter(|h| h.is_finite() && *h > 0.).map(|h| HostMsg::Height(h as f32)),
        "ui/open-link" => {
            let url = url::Url::parse(params["url"].as_str()?).ok()?;
            matches!(url.scheme(), "http" | "https").then(|| HostMsg::Link(url.into()))
        }
        _ => None,
    }
}

/// Quem pintou a página neste quadro. A lista não pinta linha fora da tela: quem estava no quadro anterior e não
/// voltou neste esconde o motor e fica "fora", e a página estacionada só recarrega depois de ter saído e voltado.
#[derive(Default)]
pub struct Paint {
    now: RefCell<Vec<(String, Option<Weak<Engine>>)>>,
    before: RefCell<Vec<(String, Option<Weak<Engine>>)>>,
    away: RefCell<HashSet<String>>,
}

impl Paint {
    fn mark(&self, id: &str, engine: Option<&Rc<Engine>>) { self.now.borrow_mut().push((id.to_owned(), engine.map(Rc::downgrade))); }

    /// Canvas antes (`start`) e depois da lista da conversa.
    pub fn edge(this: &Rc<Self>, start: bool) -> impl IntoElement {
        let paint = this.clone();
        canvas(|_, _, _| {}, move |_, _, _, _| if start { paint.now.borrow_mut().clear() } else { paint.end() }).absolute().size_0()
    }

    fn end(&self) {
        let now = self.now.borrow();
        for (id, engine) in self.before.borrow().iter().filter(|(id, _)| !now.iter().any(|(n, _)| n == id)) {
            if let Some(engine) = engine.as_ref().and_then(Weak::upgrade) { engine.hide(); }
            self.away.borrow_mut().insert(id.clone());
        }
        drop(now);
        self.before.swap(&self.now);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum ViewState { Loading, Ready, Error, Expired }

/// Motor vivo da página e o laço que lê os eventos dele; caem juntos.
#[cfg(target_os = "linux")]
struct Running { engine: Rc<Engine>, _drain: Task<()>, framed: bool, theme_sent: String }

struct PageView {
    page: PageRef,
    /// Id da chamada, que é o da linha na lista: a altura nova remede só ela.
    row: String,
    state: ViewState,
    /// Viva (Linux com Chromium); `false` é a imagem estática do servidor.
    live: bool,
    /// Pedido ou motor nascendo: não pede de novo.
    busy: bool,
    html: Option<Rc<str>>,
    reported: Option<f32>,
    #[cfg(target_os = "linux")]
    running: Option<Running>,
    /// Saiu do orçamento: mostra o último quadro até voltar à tela.
    parked: bool,
    last_frame: Option<Arc<RenderImage>>,
    /// Imagem estática, o tema (escuro?) e a largura do servidor em que foi tirada.
    shot: Option<(bool, u32, Arc<RenderImage>)>,
    no_image: bool,
    focus: FocusHandle,
    gesture: Option<Instant>,
    origin: Rc<Cell<Point<Pixels>>>,
    /// O botão desceu nesta página: só ela recebe a soltura de fora.
    pressed: bool,
}

pub struct Pages {
    views: HashMap<String, PageView>,
    budget: Budget,
    pub paint: Rc<Paint>,
    window: AnyWindowHandle,
    /// Largura do cartão no último desenho: escolhe a altura medida mais próxima e a imagem.
    width: Rc<Cell<f32>>,
    live: bool,
}

impl Pages {
    pub fn new(window: AnyWindowHandle) -> Self {
        let live = cfg!(target_os = "linux") && Engine::available().is_ok();
        Self { views: HashMap::new(), budget: Budget::default(), paint: Rc::default(), window, width: Rc::new(Cell::new(COLUMN)), live }
    }

    /// Conversa trocada ou recarregada: nada da anterior é reaproveitado (a página pode ter expirado). Os motores caem
    /// junto com as visões.
    pub fn clear(&mut self) {
        self.views.clear();
        self.budget = Budget::default();
        self.paint.away.borrow_mut().clear();
    }

    /// Conversa refeita: sai quem não tem mais linha.
    pub fn retain_rows(&mut self, keep: impl Fn(&String) -> bool) {
        self.views.retain(|_, view| keep(&view.row));
        let views = &self.views;
        self.budget.order.retain(|id| views.contains_key(id));
    }
}

/// Largura do cartão na pintura. Mudou: o próximo quadro redesenha, para a altura e a imagem seguirem a coluna (no
/// meio da pintura o `refresh` não vale).
fn record_width(cell: &Cell<f32>, bounds: Bounds<Pixels>, window: &mut Window) {
    let width = f32::from(bounds.size.width);
    if (cell.replace(width) - width).abs() > 0.5 { window.on_next_frame(|window, _| window.refresh()); }
}

/// Larguras em que o servidor tira a imagem (`WIDTHS` da rota `shot`).
fn shot_width(width: f32) -> u32 {
    [360, 728, 1000].into_iter().min_by_key(|w| (*w as f32 - width).abs() as u32).unwrap_or(728)
}

fn hex(color: Hsla) -> String {
    let c = color.to_rgb();
    let ch = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    // Borda e texto apagado do tema são translúcidos: o alfa vai junto, senão a linha sai pesada.
    if c.a >= 1. { format!("#{:02x}{:02x}{:02x}", ch(c.r), ch(c.g), ch(c.b)) } else { format!("#{:02x}{:02x}{:02x}{:02x}", ch(c.r), ch(c.g), ch(c.b), ch(c.a)) }
}

/// Script que entrega o tema do app à página, nos nomes de variável da spec.
#[cfg(target_os = "linux")]
fn theme_script() -> String {
    let vars = json!({
        "--background": "transparent", "--foreground": hex(theme::text()), "--muted-foreground": hex(theme::muted()),
        "--surface": hex(theme::surface()), "--border": hex(theme::border()), "--accent": hex(theme::accent()),
    });
    let params = json!({"theme": if theme::is_dark() { "dark" } else { "light" }, "styles": {"variables": vars}});
    format!("window.__hangarApply&&window.__hangarApply({params})")
}

impl Hangar {
    /// Página publicada no lugar da chamada `html_render`, quando o resultado pareado traz `hangar_page`.
    pub(super) fn tool_page(&self, tool: Tool) -> Option<PageRef> {
        let call = &self.chat.events[tool.call];
        page_from_result(call.tool_name.as_deref()?, self.chat.events[tool.result?].result.as_deref()?)
    }

    fn remeasure_page(&mut self, id: &str) {
        let Some(row) = self.pages.views.get(id).map(|v| v.row.clone()) else { return };
        if let Some(i) = self.row_ids.iter().position(|r| *r == row) { self.list_state.remeasure_items(i..i + 1); }
    }

    fn page_failed(&mut self, id: &str, error: &Failure) {
        let Some(view) = self.pages.views.get_mut(id) else { return };
        view.busy = false;
        match (error.status, error.code.as_deref()) {
            (Some(404), Some("erro_pagina_sem_imagem")) => { view.no_image = true; view.state = ViewState::Ready; }
            (Some(404), _) => view.state = ViewState::Expired,
            _ => { eprintln!("[pagina] {id}: {}", error.detail); view.state = ViewState::Error; }
        }
    }

    /// Busca o que falta para a página aparecer: o HTML e o motor (viva) ou a imagem no tema atual (estática).
    fn drive_page(&mut self, id: &str, cx: &mut Context<Self>) {
        let (Some(api), Some(key)) = (self.session_api(), self.selected_key()) else { return };
        let width = self.pages.width.get();
        let Some(view) = self.pages.views.get_mut(id) else { return };
        if view.busy || matches!(view.state, ViewState::Error | ViewState::Expired) { return; }
        #[cfg(target_os = "linux")]
        if view.live {
            if view.html.is_none() {
                view.busy = true;
                let (page_id, name) = (id.to_owned(), key.name.clone());
                let task = self.runtime.spawn(async move { api.page(&name, &page_id, &[], &[("raw", "1")]).await });
                let id = id.to_owned();
                cx.spawn(async move |this, cx| {
                    let result = task.await.unwrap_or_else(|_| Err(Failure::local("network_error")));
                    let _ = this.update(cx, |this, cx| {
                        match result {
                            Ok(bytes) => if let Some(view) = this.pages.views.get_mut(&id) {
                                view.busy = false;
                                view.html = Some(String::from_utf8_lossy(&bytes).into());
                            },
                            Err(error) => this.page_failed(&id, &error),
                        }
                        this.remeasure_page(&id);
                        cx.notify();
                    });
                }).detach();
            } else if view.running.is_none() && !view.parked {
                self.start_page_engine(id, cx);
            }
            return;
        }
        let dark = theme::is_dark();
        let bucket = shot_width(width);
        if view.no_image || view.shot.as_ref().is_some_and(|(d, w, _)| *d == dark && *w == bucket) { return; }
        view.busy = true;
        let (page_id, name, w) = (id.to_owned(), key.name.clone(), bucket.to_string());
        let task = self.runtime.spawn(async move {
            let bytes = api.page(&name, &page_id, &["shot"], &[("theme", if dark { "dark" } else { "light" }), ("width", &w)]).await?;
            Ok::<_, Failure>(tokio::task::spawn_blocking(move || media::decode(&bytes, 4096, 8192, None)).await.ok().flatten())
        });
        let id = id.to_owned();
        cx.spawn(async move |this, cx| {
            let result = task.await.unwrap_or_else(|_| Err(Failure::local("network_error")));
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(image) => if let Some(view) = this.pages.views.get_mut(&id) {
                        view.busy = false;
                        match image {
                            Some(image) => { view.shot = Some((dark, bucket, image)); view.state = ViewState::Ready; }
                            None => view.state = ViewState::Error,
                        }
                    },
                    Err(error) => this.page_failed(&id, &error),
                }
                this.remeasure_page(&id);
                cx.notify();
            });
        }).detach();
    }

    /// Abre o motor da página. Bloqueia como o do painel (fora do `update`); falhou, a página cai para a imagem.
    #[cfg(target_os = "linux")]
    fn start_page_engine(&mut self, id: &str, cx: &mut Context<Self>) {
        let (handle, width) = (self.pages.window, self.pages.width.get());
        let Some(view) = self.pages.views.get_mut(id) else { return };
        let Some(html) = view.html.clone() else { return };
        view.busy = true;
        let id = id.to_owned();
        cx.spawn(async move |this, cx| {
            // Sem limite: o motor manda com `try_send` e quadro perdido não volta.
            let (events, received) = async_channel::unbounded();
            let engine = handle.update(cx, |_, window, cx| Engine::prepare(window, cx)).map_err(|e| e.to_string()).and_then(|r| r)
                .and_then(|starter| starter.start_page(&html, width, None, events)).map(Rc::new);
            let _ = this.update(cx, |this, cx| this.page_engine_ready(id, engine, received, cx));
        }).detach();
    }

    #[cfg(target_os = "linux")]
    fn page_engine_ready(&mut self, id: String, engine: Result<Rc<Engine>, String>, received: async_channel::Receiver<crate::browser::Event>, cx: &mut Context<Self>) {
        let Some(view) = self.pages.views.get_mut(&id) else { return };
        view.busy = false;
        let engine = match engine {
            Ok(engine) => engine,
            Err(error) => {
                eprintln!("[pagina] motor da página {id} falhou: {error}");
                view.live = false;
                cx.notify();
                return;
            }
        };
        let owner = id.clone();
        let drain = cx.spawn(async move |this, cx| {
            while let Ok(event) = received.recv().await {
                if this.update(cx, |this, cx| this.page_event(&owner, event, cx)).is_err() { break; }
            }
        });
        view.running = Some(Running { engine, _drain: drain, framed: false, theme_sent: String::new() });
        view.state = ViewState::Ready;
        if let Some(old) = self.pages.budget.touch(&id) { self.park_page(&old, cx); }
        cx.notify();
    }

    /// Fora do orçamento: guarda a tela atual como imagem e fecha o motor (o `Drop` fecha alvo e contexto).
    #[cfg(target_os = "linux")]
    fn park_page(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(view) = self.pages.views.get_mut(id) else { return };
        let Some(running) = view.running.take() else { return };
        view.parked = true;
        self.pages.paint.away.borrow_mut().remove(id);
        let shot = running.engine.cdp().call("Page.captureScreenshot", json!({"format": "png"}));
        let id = id.to_owned();
        cx.spawn(async move |this, cx| {
            use base64::Engine as _;
            use futures::future::{Either, select};
            // Página escondida que não responde não segura o motor fora do orçamento: cai sem o último quadro.
            let timer = cx.background_executor().timer(PARK_SHOT);
            let reply = match select(std::pin::pin!(shot), std::pin::pin!(timer)).await { Either::Left((reply, _)) => reply.ok(), Either::Right(_) => None };
            let frame = reply
                .and_then(|v| base64::engine::general_purpose::STANDARD.decode(v["data"].as_str().unwrap_or("")).ok())
                .and_then(|bytes| media::decode(&bytes, 4096, 8192, None));
            drop(running);
            let _ = this.update(cx, |this, cx| {
                if let Some(view) = this.pages.views.get_mut(&id) { view.last_frame = frame; }
                cx.notify();
            });
        }).detach();
    }

    #[cfg(target_os = "linux")]
    fn page_event(&mut self, id: &str, event: crate::browser::Event, cx: &mut Context<Self>) {
        let Some(view) = self.pages.views.get_mut(id) else { return };
        match event {
            crate::browser::Event::Frame => {
                if let Some(running) = &mut view.running { running.framed = true; }
                cx.notify();
            }
            crate::browser::Event::Host(raw) => match parse_host(&raw) {
                Some(HostMsg::Height(height)) if view.reported != Some(height) => {
                    view.reported = Some(height);
                    self.remeasure_page(id);
                    cx.notify();
                }
                // Só logo depois de um clique ou tecla na página: sozinha ela não abre o navegador.
                Some(HostMsg::Link(url)) if view.gesture.take().is_some_and(|at| at.elapsed() < GESTURE) => cx.open_url(&url),
                _ => {}
            },
            crate::browser::Event::State(_) => {}
        }
    }

    fn retry_page(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(view) = self.pages.views.get_mut(id) {
            (view.state, view.busy, view.html, view.shot, view.no_image) = (ViewState::Loading, false, None, None, false);
            view.live = self.pages.live;
        }
        self.remeasure_page(id);
        cx.notify();
    }

    /// Baixa a casca isolada pelo `Api` (token no cabeçalho, nunca na URL), grava numa cópia privada e a abre.
    fn open_page_in_browser(&mut self, id: String, cx: &mut Context<Self>) {
        let (Some(api), Some(key)) = (self.session_api(), self.selected_key()) else { return };
        let name = key.name.clone();
        let task = self.runtime.spawn(async move {
            let bytes = api.page(&name, &id, &[], &[]).await.map_err(|error| Self::fetch_failure(&error))?;
            let path = private_copy(&format!("{}.html", composer::safe_name(&id))).map_err(|_| tr("save_failed"))?;
            tokio::task::spawn_blocking(move || std::fs::write(&path, bytes).map(|()| path)).await
                .ok().and_then(Result::ok).ok_or_else(|| tr("save_failed"))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await.unwrap_or_else(|_| Err(tr("save_failed")));
            let _ = this.update(cx, |this, cx| {
                match result.and_then(|path| url::Url::from_file_path(&path).map_err(|()| tr("save_failed"))) {
                    Ok(url) => cx.open_url(url.as_str()),
                    Err(error) => { this.action_feedback.insert(key, (error, true)); }
                }
                cx.notify();
            });
        }).detach();
    }

    #[cfg(target_os = "linux")]
    fn page_engine(&self, id: &str) -> Option<Rc<Engine>> {
        self.pages.views.get(id)?.running.as_ref().map(|r| r.engine.clone())
    }

    #[cfg(target_os = "linux")]
    fn page_pointer(&mut self, id: &str, kind: Pointer, at: Point<Pixels>, clicks: usize) {
        let Some(view) = self.pages.views.get_mut(id) else { return };
        match kind {
            Pointer::Down => view.pressed = true,
            // Soltura de um arrasto que não começou nesta página não é dela.
            Pointer::Up if !std::mem::take(&mut view.pressed) => return,
            _ => {}
        }
        let origin = view.origin.get();
        if let Some(engine) = self.page_engine(id) { engine.pointer(kind, at - origin, clicks); }
    }

    pub(super) fn render_page_card(&mut self, tool: Tool, page: PageRef, cx: &mut Context<Self>) -> AnyElement {
        let id = page.id.clone();
        let row = self.chat.events[tool.call].id.clone();
        let live = self.pages.live;
        self.pages.views.entry(id.clone()).or_insert_with(|| PageView {
            page: page.clone(), row, state: ViewState::Loading, live, busy: false, html: None, reported: None,
            #[cfg(target_os = "linux")]
            running: None,
            parked: false, last_frame: None, shot: None, no_image: false, focus: cx.focus_handle(), gesture: None, origin: Rc::default(), pressed: false,
        });
        // Estacionada que saiu da tela e voltou recarrega.
        #[cfg(target_os = "linux")]
        if self.pages.views.get(&id).is_some_and(|v| v.parked) && self.pages.paint.away.borrow_mut().remove(&id) {
            if let Some(view) = self.pages.views.get_mut(&id) { view.parked = false; }
        }
        self.drive_page(&id, cx);
        #[cfg(target_os = "linux")]
        if self.pages.views.get(&id).is_some_and(|v| v.running.is_some()) { self.pages.budget.touch(&id); }
        let width = self.pages.width.get();
        let Some(view) = self.pages.views.get_mut(&id) else { return div().into_any_element() };
        let title = view.page.title.clone();
        let height = px(frame_height(&view.page, width, view.reported));
        let note = |text: String| div().py(px(12.)).text_size(px(13.)).text_color(theme::muted()).whitespace_normal().child(text);
        match view.state {
            ViewState::Expired => return note(tr_shared("page_expired", &[])).into_any_element(),
            ViewState::Error => {
                let retry = id.clone();
                return h_flex().id(SharedString::from(format!("page-error-{id}"))).gap_2().role(Role::Alert)
                    .child(note(tr_shared("page_error", &[("title", &title)])))
                    .child(Button::new(SharedString::from(format!("page-retry-{id}"))).ghost().xsmall().label(tr_shared("page_retry", &[]))
                        .on_click(cx.listener(move |this, _, _, cx| this.retry_page(&retry, cx))))
                    .into_any_element();
            }
            _ => {}
        }
        let loading = || div().id(SharedString::from(format!("page-loading-{id}"))).size_full().flex().items_center().role(Role::Status).child(note(tr_shared("page_loading", &[("title", &title)])));
        if !view.live {
            let open = id.clone();
            let image = view.shot.as_ref().map(|(_, _, image)| image.clone());
            let width_cell = self.pages.width.clone();
            // A imagem e a altura estimada vêm da largura real da coluna, não só da página viva.
            let measure = canvas(|_, _, _| {}, move |bounds, _, window, _| record_width(&width_cell, bounds, window)).absolute().inset_0();
            let bar = h_flex().gap_2()
                .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(theme::muted()).child(title.clone()))
                .child(Button::new(SharedString::from(format!("page-open-{id}"))).ghost().xsmall().icon(IconName::ExternalLink)
                    .label(tr_shared("page_open_browser", &[]))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_page_in_browser(open.clone(), cx))));
            let picture = match (view.state, image, view.no_image) {
                (_, _, true) => Some(note(tr_shared("page_no_image", &[]))),
                (_, Some(image), _) => Some(div().w_full().h(height).child(img(image).size_full().object_fit(ObjectFit::Contain))),
                _ => Some(div().w_full().h(height).child(loading())),
            };
            return v_flex().relative().w_full().gap_1().child(measure).children(picture).child(bar).into_any_element();
        }
        #[cfg(target_os = "linux")]
        {
            let script = theme_script();
            if let Some(running) = view.running.as_mut().filter(|r| r.framed && r.theme_sent != script) {
                running.engine.evaluate(&script);
                running.theme_sent = script;
            }
            let engine = view.running.as_ref().map(|r| r.engine.clone());
            let framed = view.running.as_ref().is_some_and(|r| r.framed);
            let inside = scrolls_inside(&view.page, width, view.reported);
            let (paint, origin, width_cell, mark) = (self.pages.paint.clone(), view.origin.clone(), self.pages.width.clone(), id.clone());
            let shown = engine.clone();
            let surface = canvas(|bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal), move |bounds, hitbox, window, _| {
                origin.set(bounds.origin);
                record_width(&width_cell, bounds, window);
                paint.mark(&mark, shown.as_ref());
                let Some(engine) = shown else { return };
                engine.place(bounds, window);
                // Roda na fase de captura: antes da lista, que senão rolaria a conversa por cima da página.
                if inside {
                    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                        if phase != DispatchPhase::Capture || !hitbox.should_handle_scroll(window) { return; }
                        let delta = event.delta.pixel_delta(px(16.));
                        engine.wheel(event.position - bounds.origin, point(-delta.x, -delta.y));
                        cx.stop_propagation();
                    });
                }
            }).absolute().inset_0();
            let cover = (!framed).then(|| match view.last_frame.clone() {
                Some(frame) => img(frame).size_full().object_fit(ObjectFit::Fill).into_any_element(),
                None => loading().into_any_element(),
            });
            let focused = view.focus.clone();
            let (down, up, up_out, moved, key_down, key_up) = (id.clone(), id.clone(), id.clone(), id.clone(), id.clone(), id.clone());
            return div().id(SharedString::from(format!("page-{id}"))).relative().w_full().h(height).overflow_hidden()
                .rounded(px(8.)).border_1().border_color(transparent_black())
                .track_focus(&view.focus).key_context("BrowserPage").aria_label(title.clone())
                .focus(|el| el.border_color(theme::accent_focus()))
                .child(surface)
                .children(cover)
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    focused.focus(window, cx);
                    let Some(view) = this.pages.views.get_mut(&down) else { return };
                    view.gesture = Some(Instant::now());
                    if view.parked { view.parked = false; cx.notify(); }
                    this.page_pointer(&down, Pointer::Down, event.position, event.click_count);
                }))
                .on_mouse_up(MouseButton::Left, cx.listener(move |this, event: &MouseUpEvent, _, _| this.page_pointer(&up, Pointer::Up, event.position, 0)))
                .on_mouse_up_out(MouseButton::Left, cx.listener(move |this, event: &MouseUpEvent, _, _| this.page_pointer(&up_out, Pointer::Up, event.position, 0)))
                .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, _| this.page_pointer(&moved, Pointer::Move, event.position, 0)))
                // A tecla é da página; Esc devolve o foco à conversa.
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                    cx.stop_propagation();
                    let k = &event.keystroke;
                    if k.key == "escape" { this.root_focus.focus(window, cx); return; }
                    if let Some(view) = this.pages.views.get_mut(&key_down) { view.gesture = Some(Instant::now()); }
                    let Some(engine) = this.page_engine(&key_down) else { return };
                    // O Chromium sem janela tem área de transferência própria: colar lê a do sistema e digita o texto.
                    let paste = (k.modifiers.control && !k.modifiers.alt && k.key == "v") || (k.modifiers.shift && k.key == "insert");
                    match paste.then(|| cx.read_from_clipboard().and_then(|item| item.text())).flatten() {
                        Some(text) => engine.insert_text(&text),
                        None => engine.key(true, k),
                    }
                }))
                .on_key_up(cx.listener(move |this, event: &KeyUpEvent, _, cx| {
                    cx.stop_propagation();
                    if let Some(engine) = this.page_engine(&key_up) { engine.key(false, &event.keystroke); }
                }))
                .into_any_element();
        }
        #[cfg(not(target_os = "linux"))]
        div().w_full().h(height).child(loading()).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    // Sem `super::*`: o glob da gpui_kit traz um `test` próprio que sombreia o `#[test]` da std.
    use super::{Budget, HostMsg, PageRef, frame_height, page_from_result, parse_host, scrolls_inside};

    #[test]
    fn reads_only_hangar_page() {
        let ok = r#"{"hangar_page":{"id":"a","title":"T","height":null,"heights":{"728":300}},"message":"x"}"#;
        assert_eq!(page_from_result("mcp__hangar__html_render", ok).unwrap().id, "a");
        assert!(page_from_result("mcp__hangar__html_render", r#"{"draft":{"id":"b"}}"#).is_none());
        assert!(page_from_result("Read", ok).is_none());
    }

    #[test]
    fn reads_mcp_content_blocks() {
        let blocks = serde_json::json!([{"type": "text", "text": r#"{"hangar_page":{"id":"a","#}, {"type": "text", "text": r#""title":"T"}}"#}]).to_string();
        assert_eq!(page_from_result("mcp__hangar__html_render", &blocks).unwrap().title, "T");
    }

    #[test]
    fn budget_evicts_oldest() {
        let mut b = Budget::default();
        for id in ["a", "b", "c", "d"] { assert_eq!(b.touch(id), None); }
        assert_eq!(b.touch("a"), None);
        assert_eq!(b.touch("e"), Some("b".into()));
    }

    #[test]
    fn height_rules() {
        let p = PageRef { id: "a".into(), title: "T".into(), height: Some(300), heights: [(728, 380)].into() };
        assert_eq!(frame_height(&p, 700., None), 300.);
        assert!(scrolls_inside(&p, 700., None));
        let free = PageRef { height: None, ..p.clone() };
        assert_eq!(frame_height(&free, 700., Some(5000.)), 2000.);
        assert!(scrolls_inside(&free, 700., Some(5000.)));
        assert!(!scrolls_inside(&free, 700., None));
    }

    #[test]
    fn host_messages_are_filtered() {
        let size = |h: &str| format!(r#"{{"jsonrpc":"2.0","method":"ui/notifications/size-changed","params":{{"height":{h}}}}}"#);
        assert_eq!(parse_host(&size("320")), Some(HostMsg::Height(320.)));
        assert_eq!(parse_host(&size("-1")), None);
        assert_eq!(parse_host(&size("\"x\"")), None);
        let link = |u: &str| format!(r#"{{"method":"ui/open-link","params":{{"url":"{u}"}}}}"#);
        assert_eq!(parse_host(&link("https://a.dev/x")), Some(HostMsg::Link("https://a.dev/x".into())));
        assert_eq!(parse_host(&link("javascript:alert(1)")), None);
        assert_eq!(parse_host(&link("file:///etc/passwd")), None);
        assert_eq!(parse_host(r#"{"method":"ui/other"}"#), None);
        assert_eq!(parse_host("not json"), None);
    }
}
