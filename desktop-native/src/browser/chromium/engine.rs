//! Navegador do painel no Linux: um alvo do Chromium sem janela por sessão. A página chega como screencast, cada
//! quadro gravado numa textura do device da GPUI, e a entrada do painel vira `Input.*`.
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::Rc,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use base64::Engine as _;
use gpui_kit::*;
use gpui_wgpu::wgpu;
use serde_json::{Value, json};

use super::{
    launch,
    pipe::{Browser, CLOSED, Session},
};
use crate::browser::{Event, Pointer, model};

/// Último quadro do screencast, já na GPU.
struct Shown {
    texture: wgpu::Texture,
    device: wgpu::Device,
    /// Tamanho da página em px lógicos (CSS), o que o quadro cobre.
    width: f32,
    height: f32,
}

/// Dividido com a thread de leitura do pipe, que decodifica o quadro sem passar pela interface.
struct Surface {
    shown: Mutex<Option<Shown>>,
    /// Há um `Event::Frame` não pintado: não manda outro.
    pending: AtomicBool,
    events: async_channel::Sender<Event>,
}

impl Surface {
    fn frame(&self, params: &Value) {
        if let Err(e) = self.upload(params) { eprintln!("[nav] quadro do Chromium descartado: {e}"); }
    }

    fn upload(&self, params: &Value) -> Result<(), String> {
        let bytes = base64::engine::general_purpose::STANDARD.decode(params["data"].as_str().unwrap_or("")).map_err(|e| e.to_string())?;
        let pixels = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg).map_err(|e| e.to_string())?.into_rgba8();
        let (width, height) = pixels.dimensions();
        let (device, queue) = gpui_wgpu::WgpuContext::shared_device().ok_or("a GPUI não expôs o device wgpu")?;
        let mut shown = self.shown.lock().unwrap_or_else(PoisonError::into_inner);
        // Mesma textura enquanto o tamanho e o device não mudam: a fila do wgpu ordena a escrita depois do último desenho.
        let reuse = shown.as_ref().filter(|s| s.device == device && s.texture.width() == width && s.texture.height() == height);
        let texture = match reuse {
            Some(s) => s.texture.clone(),
            None => device.create_texture(&wgpu::TextureDescriptor {
                label: Some("navegador"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }),
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &pixels,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * width), rows_per_image: Some(height) },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        let meta = &params["metadata"];
        let css = |key: &str, fallback: u32| meta[key].as_f64().map_or(fallback as f32, |v| v as f32);
        *shown = Some(Shown { texture, device, width: css("deviceWidth", width), height: css("deviceHeight", height) });
        drop(shown);
        if !self.pending.swap(true, Ordering::SeqCst) { let _ = self.events.try_send(Event::Frame); }
        Ok(())
    }
}

/// A janela da GPUI não entra: o Chromium não tem janela. Guarda o executor para os eventos do pipe.
pub struct Starter {
    executor: ForegroundExecutor,
    scale: f32,
}

type Publish = Rc<dyn Fn(&dyn Fn(&mut model::PageState))>;

pub struct Engine {
    session: Rc<Session>,
    /// Edita o estado da barra e o manda ao painel se mudou.
    publish: Publish,
    target: String,
    window: i64,
    /// Altura da barra que o Chrome completo sem janela ainda desconta da janela (o shell não tem).
    decoration: f32,
    executor: ForegroundExecutor,
    surface: Arc<Surface>,
    placed: Cell<Option<(Size<Pixels>, f32)>>,
    visible: Cell<bool>,
    pressed: Cell<bool>,
    /// A página caiu ou se soltou com o Chromium vivo.
    dead: Rc<Cell<bool>>,
}

impl Engine {
    pub fn available() -> Result<(), String> { launch::find().map(|_| ()) }

    pub fn prepare(window: &Window, cx: &App) -> Result<Starter, String> {
        Ok(Starter { executor: cx.foreground_executor().clone(), scale: window.scale_factor() })
    }
}

impl Starter {
    /// Sobe o Chromium (se ainda não há um) e abre o alvo desta sessão. As respostas chegam pela thread de leitura,
    /// então esperar aqui não trava o pipe; o primeiro navegador espera o processo nascer.
    pub fn start(self, events: async_channel::Sender<Event>) -> Result<Engine, String> {
        let browser = Browser::shared(&self.executor, self.scale)?;
        let long = Duration::from_secs(10);
        let created = browser.call_blocking(None, "Target.createTarget", json!({"url": "about:blank", "newWindow": true}), long)?;
        let target = created["targetId"].as_str().ok_or("createTarget sem targetId")?.to_owned();
        browser.own(&target, true);
        browser.close_initial();
        let session = Rc::new(Session::attach(&browser, &target)?);
        let window = browser.call_blocking(None, "Browser.getWindowForTarget", json!({"targetId": target}), long)?["windowId"]
            .as_i64().ok_or("getWindowForTarget sem windowId")?;
        session.call_blocking("Page.enable", json!({}))?;
        // Queda do renderer chega só por aqui (`Inspector.targetCrashed`, na sessão da página).
        session.call_blocking("Inspector.enable", json!({}))?;
        // A página do agente, que ninguém olha, se comporta como focada: `:focus`, `focus()` e o teclado funcionam.
        session.call_blocking("Emulation.setFocusEmulationEnabled", json!({"enabled": true}))?;
        // Mesma regra de endereço do painel: só web (http/https sem usuário e senha); o resto não carrega.
        session.call_blocking("Fetch.enable", json!({"patterns": [{"resourceType": "Document", "requestStage": "Request"}]}))?;
        browser.call_blocking(None, "Browser.setWindowBounds", json!({"windowId": window, "bounds": {"width": 1280, "height": 800}}), long)?;
        let measured = session.call_blocking("Runtime.evaluate", json!({"expression": "outerHeight-innerHeight", "returnByValue": true}))?;
        let decoration = measured["result"]["value"].as_f64().unwrap_or(0.).max(0.) as f32;
        let state = Rc::new(RefCell::new(model::PageState::default()));
        let surface = Arc::new(Surface { shown: Mutex::new(None), pending: AtomicBool::new(false), events: events.clone() });
        let sink = surface.clone();
        browser.sink(session.id(), Box::new(move |params| sink.frame(params)));
        let dead = Rc::new(Cell::new(false));
        let publish = listen(&session, &target, &state, &events, &self.executor, &dead);
        Ok(Engine {
            session, publish, target, window, decoration, executor: self.executor, surface, dead,
            placed: Cell::new(None), visible: Cell::new(false), pressed: Cell::new(false),
        })
    }
}

/// Estado da barra (endereço, título, carregando, voltar/avançar), diálogos e janelas novas.
fn listen(
    session: &Rc<Session>, target: &str, state: &Rc<RefCell<model::PageState>>, events: &async_channel::Sender<Event>, executor: &ForegroundExecutor,
    dead: &Rc<Cell<bool>>,
) -> Publish {
    let publish: Publish = {
        let (state, events) = (state.clone(), events.clone());
        Rc::new(move |edit: &dyn Fn(&mut model::PageState)| {
            let mut page = state.borrow_mut();
            let before = page.clone();
            edit(&mut page);
            if *page != before { let _ = events.try_send(Event::State(page.clone())); }
        })
    };
    let history = {
        let (session, publish, executor) = (Rc::downgrade(session), publish.clone(), executor.clone());
        Rc::new(move || {
            let Some(session) = session.upgrade() else { return };
            let (call, publish) = (session.call("Page.getNavigationHistory", json!({})), publish.clone());
            executor.spawn(async move {
                let Ok(h) = call.await else { return };
                let (index, count) = (h["currentIndex"].as_i64().unwrap_or(0), h["entries"].as_array().map_or(0, Vec::len) as i64);
                publish(&|page| { page.can_back = index > 0; page.can_forward = index + 1 < count; });
            }).detach();
        })
    };
    let main = target.to_owned();
    let (p, h) = (publish.clone(), history.clone());
    let _ = session.on("Page.frameNavigated", move |params| {
        let frame = &params["frame"];
        if !frame["parentId"].is_null() { return; }
        let url = frame["url"].as_str().unwrap_or("").to_owned();
        p(&|page| { page.url = Some(url.clone()); page.error = None; });
        h();
    });
    let (p, h, m) = (publish.clone(), history.clone(), main.clone());
    let _ = session.on("Page.navigatedWithinDocument", move |params| {
        if params["frameId"] != m.as_str() { return; }
        let url = params["url"].as_str().unwrap_or("").to_owned();
        p(&|page| page.url = Some(url.clone()));
        h();
    });
    for (event, loading) in [("Page.frameStartedLoading", true), ("Page.frameStoppedLoading", false)] {
        let (p, m) = (publish.clone(), main.clone());
        let _ = session.on(event, move |params| if params["frameId"] == m.as_str() { p(&|page| page.loading = loading) });
    }
    let (weak, p, m) = (Rc::downgrade(session), publish.clone(), main.clone());
    let _ = session.on("Fetch.requestPaused", move |params| {
        let Some(session) = weak.upgrade() else { return };
        let (id, url) = (params["requestId"].clone(), params["request"]["url"].as_str().unwrap_or("").to_owned());
        if model::allowed_request(&url) { return drop(session.call("Fetch.continueRequest", json!({"requestId": id}))); }
        drop(session.call("Fetch.failRequest", json!({"requestId": id, "errorReason": "BlockedByClient"})));
        if params["frameId"] == m.as_str() { p(&|page| page.error = Some(crate::i18n::tr("browser_blocked").replace("{url}", &url))); }
    });
    // Sem resposta o diálogo trava a página, e no app não há quem o veja: `alert` e `beforeunload` passam, mas
    // `confirm`/`prompt` são recusados, para nada ser confirmado sem alguém ter lido.
    let weak = Rc::downgrade(session);
    let _ = session.on("Page.javascriptDialogOpening", move |params| {
        let kind = params["type"].as_str().unwrap_or("");
        let accept = matches!(kind, "alert" | "beforeunload");
        let message: String = params["message"].as_str().unwrap_or("").chars().take(200).collect();
        eprintln!("[nav] dialogo {kind} {}: {message}", if accept { "aceito" } else { "recusado" });
        let Some(session) = weak.upgrade() else { return };
        drop(session.call("Page.handleJavaScriptDialog", json!({"accept": accept})));
    });
    let (p, m) = (publish.clone(), main.clone());
    session.watch("Target.targetInfoChanged", move |params| {
        let info = &params["targetInfo"];
        if info["targetId"] == m.as_str() {
            let title = info["title"].as_str().unwrap_or("").to_owned();
            // Sem título o Chromium devolve o endereço; a barra mostra o nome do app nesse caso.
            let title = if info["url"].as_str() == Some(title.as_str()) { String::new() } else { title };
            p(&|page| page.title = title.clone());
        }
    });
    // O nativo não tem abas: link com `target=_blank` e `window.open` abrem nesta mesma página. Página que nenhum
    // navegador do app abriu (popup `noopener`, sem dono) fecha: sem painel, ninguém veria o JS dela rodando.
    let (weak, m, closing) = (Rc::downgrade(session), main.clone(), Rc::new(RefCell::new(HashSet::<String>::new())));
    for event in ["Target.targetCreated", "Target.targetInfoChanged"] {
        let (weak, m, closing) = (weak.clone(), m.clone(), closing.clone());
        session.watch(event, move |params| {
            let info = &params["targetInfo"];
            let Some(id) = info["targetId"].as_str() else { return };
            let Some(session) = weak.upgrade() else { return };
            let browser = session.browser();
            if info["type"] != "page" || browser.owns(id) { return; }
            let (opener, url) = (info["openerId"].as_str().unwrap_or(""), info["url"].as_str().unwrap_or(""));
            let mine = opener == m;
            // Aberta por esta página: espera o endereço chegar. Por outra do app: o navegador dela cuida.
            if (mine && (url.is_empty() || url == "about:blank")) || (!mine && browser.owns(opener)) { return; }
            if !closing.borrow_mut().insert(id.to_owned()) { return; }
            if mine { drop(session.call("Page.navigate", json!({"url": url}))); }
            drop(browser.call(None, "Target.closeTarget", json!({"targetId": id})));
        });
    }
    session.watch("Target.targetDestroyed", move |params| { if let Some(id) = params["targetId"].as_str() { closing.borrow_mut().remove(id); } });
    let p = publish.clone();
    session.watch(CLOSED, move |_| p(&|page| { page.loading = false; page.error = Some("o Chromium fechou; feche e abra o navegador".into()); }));
    // A página caiu com o Chromium vivo: sem isto o painel congela no último quadro, calado.
    let fell: Rc<dyn Fn()> = {
        let (weak, p, dead) = (Rc::downgrade(session), publish.clone(), dead.clone());
        Rc::new(move || {
            if dead.replace(true) { return; }
            eprintln!("[nav] a pagina caiu");
            if let Some(session) = weak.upgrade() { session.fail_pending(); }
            p(&|page| { page.loading = false; page.error = Some("a pagina caiu; abra o endereco de novo".into()); });
        })
    };
    let f = fell.clone();
    let _ = session.on("Inspector.targetCrashed", move |_| f());
    for (event, key, id) in [("Target.targetCrashed", "targetId", main.clone()), ("Target.detachedFromTarget", "sessionId", session.id().to_owned())] {
        let f = fell.clone();
        session.watch(event, move |params| { if params[key] == id.as_str() { f(); } });
    }
    publish
}

impl Engine {
    pub fn alive(&self) -> bool { self.session.browser().alive() && !self.dead.get() }

    /// A sessão do painel, para o controlador do hangar-preview.
    pub fn cdp(&self) -> Rc<Session> { self.session.clone() }

    /// Sessão própria para a tela remota do celular: o screencast dela tem o tamanho do aparelho, sem mexer no do painel.
    pub fn viewer_session(&self) -> Result<Rc<Session>, String> {
        let session = Session::attach(self.session.browser(), &self.target)?;
        session.call_blocking("Page.enable", json!({}))?;
        Ok(Rc::new(session))
    }

    fn send(&self, method: &str, params: Value) { drop(self.session.call(method, params)); }

    pub fn load(&self, url: &str) {
        let (call, publish) = (self.session.call("Page.navigate", json!({"url": url})), self.publish.clone());
        self.executor.spawn(async move {
            // Falha de rede: o Chromium mostra a página de erro dele, e a barra o motivo.
            let error = match call.await {
                Ok(r) => r["errorText"].as_str().filter(|e| !e.is_empty()).map(str::to_owned),
                Err(e) => Some(e),
            };
            if let Some(error) = error { publish(&|page| page.error = Some(error.clone())); }
        }).detach();
    }

    pub fn back(&self) { self.history_step(-1); }
    pub fn forward(&self) { self.history_step(1); }
    pub fn reload(&self) { self.send("Page.reload", json!({})); }

    fn history_step(&self, step: i64) {
        let (call, session) = (self.session.call("Page.getNavigationHistory", json!({})), Rc::downgrade(&self.session));
        self.executor.spawn(async move {
            let Ok(h) = call.await else { return };
            let index = h["currentIndex"].as_i64().unwrap_or(0) + step;
            let Some(entry) = usize::try_from(index).ok().and_then(|i| h["entries"].get(i)) else { return };
            if let Some(session) = session.upgrade() { drop(session.call("Page.navigateToHistoryEntry", json!({"entryId": entry["id"]}))); }
        }).detach();
    }

    /// Chamado na pintura da página, a cada quadro visível.
    pub fn place(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        let scale = window.scale_factor();
        let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        let resized = self.placed.get() != Some((bounds.size, scale));
        if resized {
            self.placed.set(Some((bounds.size, scale)));
            let size = json!({"width": w.round() as i64, "height": (h + self.decoration).round() as i64});
            drop(self.session.browser().call(None, "Browser.setWindowBounds", json!({"windowId": self.window, "bounds": size})));
        }
        if !self.visible.replace(true) || resized {
            let (pw, ph) = ((w * scale).round() as i64, (h * scale).round() as i64);
            self.send("Page.startScreencast", json!({"format": "jpeg", "quality": 85, "maxWidth": pw.max(1), "maxHeight": ph.max(1), "everyNthFrame": 1}));
        }
        // Zera antes de ler: um quadro que chegue no meio ainda avisa a tela.
        self.surface.pending.store(false, Ordering::SeqCst);
        let device = gpui_wgpu::WgpuContext::shared_device().map(|(device, _)| device);
        let shown = {
            let mut latest = self.surface.shown.lock().unwrap_or_else(PoisonError::into_inner);
            if latest.as_ref().is_some_and(|s| Some(&s.device) != device.as_ref()) { *latest = None; }
            latest.as_ref().map(|s| (s.texture.clone(), s.width, s.height))
        };
        if let Some((texture, width, height)) = shown {
            // No tamanho da página que o quadro mostra: durante um resize a sobra fica vazia, nunca esticada.
            let drawn = size(px(width), px(height));
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                window.paint_surface(Bounds::new(bounds.origin, drawn), Arc::new(texture))
            });
        }
    }

    /// Página fora da tela: sem screencast. O controlador cuida do tamanho dela para o `shot`.
    pub fn hide(&self) {
        if self.visible.replace(false) { self.send("Page.stopScreencast", json!({})); }
    }

    /// `at` relativo à origem da página, em px lógicos. Só o botão esquerdo.
    pub fn pointer(&self, kind: Pointer, at: Point<Pixels>, clicks: usize) {
        let (x, y) = (f32::from(at.x) as f64, f32::from(at.y) as f64);
        let (kind, button, clicks) = match kind {
            Pointer::Down => { self.pressed.set(true); ("mousePressed", "left", clicks.max(1)) }
            Pointer::Up => { self.pressed.set(false); ("mouseReleased", "left", 1) }
            Pointer::Move => ("mouseMoved", if self.pressed.get() { "left" } else { "none" }, 0),
        };
        let buttons = if self.pressed.get() { 1 } else { 0 };
        self.send("Input.dispatchMouseEvent", json!({"type": kind, "x": x, "y": y, "button": button, "buttons": buttons, "clickCount": clicks}));
    }

    /// `delta` em px lógicos; `delta.y > 0` avança a página. Quem chama já inverteu o sinal da GPUI.
    pub fn wheel(&self, at: Point<Pixels>, delta: Point<Pixels>) {
        self.send("Input.dispatchMouseEvent", json!({
            "type": "mouseWheel", "x": f32::from(at.x), "y": f32::from(at.y),
            "deltaX": f32::from(delta.x), "deltaY": f32::from(delta.y), "button": "none",
        }));
    }

    pub fn key(&self, down: bool, keystroke: &Keystroke) {
        if let Some(event) = key_event(down, keystroke) { self.send("Input.dispatchKeyEvent", event); }
    }

    /// A página finge estar sempre focada (`setFocusEmulationEnabled`): o foco do painel não precisa ir a ela.
    pub fn focus(&self, _focused: bool) {}
    pub fn release_focus(&self) {}
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.session.browser().own(&self.target, false);
        drop(self.session.browser().call(None, "Target.closeTarget", json!({"targetId": self.target})));
    }
}

/// Tecla da GPUI → `Input.dispatchKeyEvent`. Texto digitado vence, para "ç"/"ã" de tecla morta; tecla morta ainda
/// compondo (sem texto) não vai à página.
fn key_event(down: bool, keystroke: &Keystroke) -> Option<Value> {
    let m = &keystroke.modifiers;
    let modifiers = [(m.alt, 1), (m.control, 2), (m.platform, 4), (m.shift, 8)].iter().filter(|(on, _)| *on).fold(0, |a, (_, b)| a | b);
    let typed = keystroke.key_char.as_deref().filter(|_| !m.control && !m.alt).and_then(single_char).filter(|c| !c.is_control());
    let named = match keystroke.key.as_str() {
        "enter" => Some("Enter"), "backspace" => Some("Backspace"), "delete" => Some("Delete"), "escape" => Some("Escape"),
        "tab" => Some("Tab"), "left" => Some("ArrowLeft"), "up" => Some("ArrowUp"), "right" => Some("ArrowRight"),
        "down" => Some("ArrowDown"), "home" => Some("Home"), "end" => Some("End"), "pageup" => Some("PageUp"),
        "pagedown" => Some("PageDown"), "space" => Some("Space"), _ => None,
    };
    // Espaço digitado vai como texto; a tabela só o usa sem texto.
    let mut event = if let Some(name) = named.filter(|n| *n != "Space" || typed.is_none()) {
        crate::browser::preview_fmt::key_event(name)
    } else if let Some(c) = typed {
        let text = c.to_string();
        let vk = if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() as u32 } else if c == ' ' { 32 } else { 0 };
        json!({"key": text, "text": text, "unmodifiedText": text, "windowsVirtualKeyCode": vk})
    } else if m.control || m.alt {
        // Ctrl/Alt + letra: atalho da página (Ctrl+A, Ctrl+Z), sem texto.
        let c = single_char(&keystroke.key)?;
        let upper = c.to_ascii_uppercase();
        let code = if upper.is_ascii_alphabetic() { format!("Key{upper}") } else if upper.is_ascii_digit() { format!("Digit{upper}") } else { String::new() };
        json!({"key": c.to_string(), "code": code, "windowsVirtualKeyCode": upper as u32})
    } else {
        return None;
    };
    event["type"] = (if down { "keyDown" } else { "keyUp" }).into();
    event["modifiers"] = modifiers.into();
    if !down || m.control || m.alt {
        let object = event.as_object_mut()?;
        object.remove("text");
        object.remove("unmodifiedText");
    }
    Some(event)
}

fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let c = chars.next()?;
    chars.next().is_none().then_some(c)
}

#[cfg(test)]
mod tests {
    // Sem `super::*`: o glob da gpui_kit traz um `test` próprio que sombreia o `#[test]` da std.
    use super::{Keystroke, Modifiers, key_event};
    use serde_json::json;

    fn stroke(key: &str, key_char: Option<&str>, control: bool, shift: bool) -> Keystroke {
        Keystroke { modifiers: Modifiers { control, shift, ..Default::default() }, key: key.into(), key_char: key_char.map(Into::into) }
    }

    #[test]
    fn maps_named_keys_text_and_shortcuts() {
        let enter = key_event(true, &stroke("enter", Some("\n"), false, false)).unwrap();
        assert_eq!((enter["key"].as_str(), enter["windowsVirtualKeyCode"].as_u64(), enter["text"].as_str()), (Some("Enter"), Some(13), Some("\r")));
        let up = key_event(false, &stroke("enter", Some("\n"), false, false)).unwrap();
        assert_eq!((up["type"].as_str(), up.get("text")), (Some("keyUp"), None));
        let shift_tab = key_event(true, &stroke("tab", None, false, true)).unwrap();
        assert_eq!((shift_tab["key"].as_str(), shift_tab["modifiers"].as_u64()), (Some("Tab"), Some(8)));
        let cedilla = key_event(true, &stroke("c", Some("ç"), false, false)).unwrap();
        assert_eq!((cedilla["key"].as_str(), cedilla["text"].as_str()), (Some("ç"), Some("ç")));
        let capital = key_event(true, &stroke("a", Some("A"), false, true)).unwrap();
        assert_eq!((capital["text"].as_str(), capital["windowsVirtualKeyCode"].as_u64()), (Some("A"), Some(65)));
        let space = key_event(true, &stroke("space", Some(" "), false, false)).unwrap();
        assert_eq!(space["text"], json!(" "));
        let select_all = key_event(true, &stroke("a", Some("a"), true, false)).unwrap();
        assert_eq!((select_all["code"].as_str(), select_all["modifiers"].as_u64(), select_all.get("text")), (Some("KeyA"), Some(2), None));
        assert_eq!(key_event(true, &stroke("f13", None, false, false)), None);
        // Tecla morta compondo (ABNT2 "~"): nada vai à página até o "ã" chegar.
        assert_eq!(key_event(true, &stroke("'", None, false, false)), None);
    }
}
