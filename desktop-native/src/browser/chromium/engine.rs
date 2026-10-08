//! Navegador do painel no Linux: um alvo do Chromium sem janela por sessão. A página chega como screencast, cada
//! quadro gravado numa textura do device da GPUI, e a entrada do painel vira `Input.*`.
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    future::Future,
    pin::pin,
    rc::Rc,
    sync::{
        Arc, Condvar, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use base64::Engine as _;
use futures::future::{Either, select};
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
}

/// Dividido com a thread de leitura do pipe, que só guarda o quadro, e com a que o decodifica: nada passa pela interface.
struct Surface {
    shown: Mutex<Option<Shown>>,
    slot: Mutex<Slot>,
    wake: Condvar,
    /// A textura foi recriada (primeiro quadro, outro tamanho): um desenho guardado ainda aponta para a antiga.
    replaced: AtomicBool,
    events: async_channel::Sender<Event>,
    /// JPEG no painel, PNG nas páginas da conversa.
    format: image::ImageFormat,
}

#[derive(Default)]
struct Slot {
    /// Último quadro chegado, ainda em base64: o que chega antes de ele ser decodificado o substitui.
    data: Option<String>,
    /// Há um `Event::Frame` que a tela ainda não desenhou: o próximo quadro espera, sem decodificar à toa.
    unseen: bool,
    closed: bool,
}

impl Slot {
    /// O quadro a decodificar agora: só o mais recente, e só depois de a tela ter desenhado o anterior.
    fn next(&mut self) -> Option<String> { if self.unseen { None } else { self.data.take() } }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> { m.lock().unwrap_or_else(PoisonError::into_inner) }

impl Surface {
    fn new(events: async_channel::Sender<Event>, format: image::ImageFormat) -> Arc<Self> {
        let surface = Arc::new(Surface {
            shown: Mutex::new(None), slot: Mutex::default(), wake: Condvar::new(), replaced: AtomicBool::new(false), events, format,
        });
        let worker = surface.clone();
        if let Err(e) = std::thread::Builder::new().name("chromium-frames".into()).spawn(move || worker.decode_loop()) {
            eprintln!("[nav] sem thread para os quadros: {e}");
        }
        surface
    }

    /// Na thread de leitura do pipe: guarda e acorda quem decodifica.
    fn frame(&self, mut params: Value) {
        let Some(Value::String(data)) = params.get_mut("data").map(Value::take) else { return };
        lock(&self.slot).data = Some(data);
        self.wake.notify_one();
    }

    fn decode_loop(&self) {
        loop {
            let data = {
                let mut slot = lock(&self.slot);
                loop {
                    if slot.closed { return; }
                    if let Some(data) = slot.next() { break data; }
                    slot = self.wake.wait(slot).unwrap_or_else(PoisonError::into_inner);
                }
            };
            match self.upload(&data) {
                Ok(()) => {
                    lock(&self.slot).unseen = true;
                    let _ = self.events.try_send(Event::Frame);
                }
                Err(e) => eprintln!("[nav] quadro do Chromium descartado: {e}"),
            }
        }
    }

    fn seen(&self) {
        lock(&self.slot).unseen = false;
        self.wake.notify_one();
    }

    fn close(&self) {
        lock(&self.slot).closed = true;
        self.wake.notify_one();
    }

    fn upload(&self, data: &str) -> Result<(), String> {
        let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| e.to_string())?;
        let pixels = image::load_from_memory_with_format(&bytes, self.format).map_err(|e| e.to_string())?.into_rgba8();
        let (width, height) = pixels.dimensions();
        let (device, queue) = gpui_wgpu::WgpuContext::shared_device().ok_or("a GPUI não expôs o device wgpu")?;
        let mut shown = lock(&self.shown);
        // Mesma textura enquanto o tamanho e o device não mudam: a fila do wgpu ordena a escrita depois do último desenho.
        let reuse = shown.as_ref().filter(|s| s.device == device && s.texture.width() == width && s.texture.height() == height);
        if reuse.is_none() { self.replaced.store(true, Ordering::SeqCst); }
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
        *shown = Some(Shown { texture, device });
        Ok(())
    }
}

/// A janela da GPUI não entra: o Chromium não tem janela. Guarda os executores para os eventos do pipe e os prazos.
pub struct Starter {
    executor: ForegroundExecutor,
    background: BackgroundExecutor,
    scale: f32,
}

/// Mantém o Chromium compartilhado de pé enquanto existir, para a primeira página não esperar o processo nascer.
pub struct Warm { _browser: Rc<Browser> }

/// Resposta do CDP sem travar a interface, com o prazo das chamadas que bloqueavam.
async fn answer<T>(background: &BackgroundExecutor, method: &str, call: impl Future<Output = Result<T, String>>) -> Result<T, String> {
    let timer = background.timer(Duration::from_secs(10));
    match select(pin!(call), pin!(timer)).await {
        Either::Left((reply, _)) => reply,
        Either::Right(_) => Err(format!("{method}: o Chromium nao respondeu")),
    }
}

/// Documento do agente posto direto na página: HTML, largura da coluna e fundo opaco (tema próprio).
type Document = (Rc<str>, f32, Option<(u8, u8, u8)>);

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
    /// Página ou site da conversa: screencast em PNG.
    png: bool,
    /// Desenhada no dobro da tela (página e site da conversa).
    double: bool,
    /// Contexto próprio da página da conversa, fechado junto com ela.
    context: Option<String>,
}

impl Engine {
    pub fn available() -> Result<(), String> { launch::find().map(|_| ()) }

    pub fn prepare(window: &Window, cx: &App) -> Result<Starter, String> {
        Ok(Starter { executor: cx.foreground_executor().clone(), background: cx.background_executor().clone(), scale: window.scale_factor() })
    }

    /// Sobe o Chromium agora, sem esperar ele responder.
    pub fn warm(window: &Window, cx: &App) -> Result<Warm, String> {
        Browser::shared_async(cx.foreground_executor(), cx.background_executor(), window.scale_factor()).map(|browser| Warm { _browser: browser })
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
        let surface = Surface::new(events.clone(), image::ImageFormat::Jpeg);
        let sink = surface.clone();
        browser.sink(session.id(), Box::new(move |params| sink.frame(params)));
        let dead = Rc::new(Cell::new(false));
        let publish = listen(&session, &target, &state, &events, &self.executor, &dead, false);
        Ok(Engine {
            session, publish, target, window, decoration, executor: self.executor, surface, dead, png: false, double: false, context: None,
            placed: Cell::new(None), visible: Cell::new(false), pressed: Cell::new(false),
        })
    }

    /// Site de verdade na conversa: o mesmo perfil do painel (cookies e login), navegação livre dentro do site e popup
    /// no próprio alvo, como o painel. Nasce sem travar a interface.
    pub fn start_url(self, url: String, events: async_channel::Sender<Event>) -> impl Future<Output = Result<Engine, String>> + use<> {
        async move {
            let browser = self.browser().await?;
            let engine = self.open(&browser, None, None, events).await?;
            engine.load(&url);
            Ok(engine)
        }
    }

    /// Página da conversa: alvo num contexto próprio (sem os cookies do painel) e documento posto direto, sem travar a
    /// interface. Sem `background` o fundo é transparente; com ele, opaco nessa cor.
    pub fn start_page(self, html: Rc<str>, width: f32, background: Option<(u8, u8, u8)>, events: async_channel::Sender<Event>)
        -> impl Future<Output = Result<Engine, String>> + use<> {
        async move {
            let browser = self.browser().await?;
            let created = self.browser_call(&browser, "Target.createBrowserContext", json!({"disposeOnDetach": true})).await?;
            let context = created["browserContextId"].as_str().ok_or("createBrowserContext sem id")?.to_owned();
            let opened = self.open(&browser, Some(context.clone()), Some((html, width, background)), events).await;
            // Falhou no meio: fechar o contexto fecha junto o alvo que já tenha nascido nele.
            if opened.is_err() { drop(browser.call(None, "Target.disposeBrowserContext", json!({"browserContextId": context}))); }
            opened
        }
    }

    async fn browser(&self) -> Result<Rc<Browser>, String> {
        let browser = Browser::shared_async(&self.executor, &self.background, self.scale)?;
        browser.ready().await?;
        Ok(browser)
    }

    async fn browser_call(&self, browser: &Browser, method: &str, params: Value) -> Result<Value, String> {
        answer(&self.background, method, browser.call(None, method, params)).await
    }

    async fn page_call(&self, session: &Session, method: &str, params: Value) -> Result<Value, String> {
        answer(&self.background, method, session.call(method, params)).await
    }

    /// Alvo da página da conversa. Falhou depois de nascer: o alvo fecha aqui, que ninguém mais o conhece.
    async fn open(&self, browser: &Rc<Browser>, context: Option<String>, document: Option<Document>, events: async_channel::Sender<Event>)
        -> Result<Engine, String> {
        let mut create = json!({"url": "about:blank", "newWindow": true});
        if let Some(context) = &context { create["browserContextId"] = context.as_str().into(); }
        let created = self.browser_call(browser, "Target.createTarget", create).await?;
        let target = created["targetId"].as_str().ok_or("createTarget sem targetId")?.to_owned();
        browser.own(&target, true);
        browser.close_initial();
        let opened = self.open_target(browser, &target, context, document, events).await;
        if opened.is_err() {
            browser.own(&target, false);
            drop(browser.call(None, "Target.closeTarget", json!({"targetId": target})));
        }
        opened
    }

    async fn open_target(
        &self, browser: &Rc<Browser>, target: &str, context: Option<String>, document: Option<Document>, events: async_channel::Sender<Event>,
    ) -> Result<Engine, String> {
        let session = Rc::new(answer(&self.background, "Target.attachToTarget", Session::attach_async(browser, target)).await?);
        // Ouvintes antes de qualquer espera: a interface segue rodando, e o que a página mandar enquanto nasce (a altura,
        // um pedido pausado) não pode chegar sem dono.
        let state = Rc::new(RefCell::new(model::PageState::default()));
        // PNG nas duas formas: o texto da conversa ao lado pede a nitidez que o JPEG perde.
        let surface = Surface::new(events.clone(), image::ImageFormat::Png);
        let sink = surface.clone();
        browser.sink(session.id(), Box::new(move |params| sink.frame(params)));
        let dead = Rc::new(Cell::new(false));
        let publish = listen(&session, target, &state, &events, &self.executor, &dead, document.is_some());
        let host = events.clone();
        let _ = session.on("Runtime.bindingCalled", move |params| {
            if params["name"] == "hangarHost" { let _ = host.try_send(Event::Host(params["payload"].as_str().unwrap_or("").to_owned())); }
        });
        match self.setup_target(browser, &session, target, &document).await {
            Ok((window, decoration)) => Ok(Engine {
                session, publish, target: target.to_owned(), window, decoration, executor: self.executor.clone(), surface, dead, png: true, double: true,
                context, placed: Cell::new(None), visible: Cell::new(false), pressed: Cell::new(false),
            }),
            Err(e) => { surface.close(); Err(e) }
        }
    }

    /// Janela, domínios e documento do alvo; devolve a janela e o desconto da barra.
    async fn setup_target(&self, browser: &Rc<Browser>, session: &Session, target: &str, document: &Option<Document>) -> Result<(i64, f32), String> {
        let window = self.browser_call(browser, "Browser.getWindowForTarget", json!({"targetId": target})).await?["windowId"]
            .as_i64().ok_or("getWindowForTarget sem windowId")?;
        self.page_call(session, "Page.enable", json!({})).await?;
        // Queda do renderer chega só por aqui (`Inspector.targetCrashed`, na sessão da página).
        self.page_call(session, "Inspector.enable", json!({})).await?;
        if let Some((_, _, background)) = &document {
            self.page_call(session, "Runtime.enable", json!({})).await?;
            self.page_call(session, "Runtime.addBinding", json!({"name": "hangarHost"})).await?;
            let (r, g, b, a) = background.map_or((0, 0, 0, 0), |(r, g, b)| (r, g, b, 1));
            self.page_call(session, "Emulation.setDefaultBackgroundColorOverride", json!({"color": {"r": r, "g": g, "b": b, "a": a}})).await?;
        }
        // Várias páginas abertas não disputam o foco: `focus`/`release_focus` contam com isto.
        self.page_call(session, "Emulation.setFocusEmulationEnabled", json!({"enabled": true})).await?;
        // Mesma regra de endereço do painel: só web (http/https sem usuário e senha); o resto não carrega.
        self.page_call(session, "Fetch.enable", json!({"patterns": [{"resourceType": "Document", "requestStage": "Request"}]})).await?;
        let (width, height) = document.as_ref().map_or((1280, 800), |(_, width, _)| (width.round() as i64, 600));
        self.browser_call(browser, "Browser.setWindowBounds", json!({"windowId": window, "bounds": {"width": width, "height": height}})).await?;
        let measured = self.page_call(session, "Runtime.evaluate", json!({"expression": "outerHeight-innerHeight", "returnByValue": true})).await?;
        let decoration = measured["result"]["value"].as_f64().unwrap_or(0.).max(0.) as f32;
        if let Some((html, _, _)) = &document {
            let tree = self.page_call(session, "Page.getFrameTree", json!({})).await?;
            let frame = tree["frameTree"]["frame"]["id"].as_str().unwrap_or_default().to_owned();
            self.page_call(session, "Page.setDocumentContent", json!({"frameId": frame, "html": &**html})).await?;
        }
        Ok((window, decoration))
    }
}

/// Estado da barra (endereço, título, carregando, voltar/avançar), diálogos e janelas novas. `block_documents`:
/// o frame principal não navega para documento nenhum (a página da conversa já nasce com o seu).
fn listen(
    session: &Rc<Session>, target: &str, state: &Rc<RefCell<model::PageState>>, events: &async_channel::Sender<Event>, executor: &ForegroundExecutor,
    dead: &Rc<Cell<bool>>, block_documents: bool,
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
        // `Aborted` deixa a página onde está; `BlockedByClient` a trocaria pela tela de erro do Chromium.
        if block_documents && params["frameId"] == m.as_str() {
            return drop(session.call("Fetch.failRequest", json!({"requestId": id, "errorReason": "Aborted"})));
        }
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

    /// Roda um script na página sem esperar o resultado.
    pub fn evaluate(&self, expression: &str) { self.send("Runtime.evaluate", json!({"expression": expression})); }

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
        // A página da conversa é desenhada no dobro da tela e reduzida pela GPU: em 1x o Chromium sem janela só suaviza o
        // texto em cinza (o fundo é transparente) e ela sai com cara de vídeo em baixa resolução ao lado do texto do app.
        let frame_scale = if self.double { (scale * 2.).ceil() } else { scale };
        let resized = self.placed.get() != Some((bounds.size, scale));
        if resized {
            self.placed.set(Some((bounds.size, scale)));
            let size = json!({"width": w.round() as i64, "height": (h + self.decoration).round() as i64});
            drop(self.session.browser().call(None, "Browser.setWindowBounds", json!({"windowId": self.window, "bounds": size})));
            if self.double {
                self.send("Emulation.setDeviceMetricsOverride", json!({
                    "width": w.round() as i64, "height": h.round() as i64, "deviceScaleFactor": frame_scale, "mobile": false,
                }));
            }
        }
        if !self.visible.replace(true) || resized {
            let (pw, ph) = ((w * frame_scale).round() as i64, (h * frame_scale).round() as i64);
            // JPEG alto em vez de PNG: o PNG pesa na decodificação e no pipe em página animada; o q92 deixa o texto
            // legível (o q85 borrava). O `shot` continua em PNG.
            let mut params = json!({"format": "jpeg", "quality": 92, "maxWidth": pw.max(1), "maxHeight": ph.max(1), "everyNthFrame": 1});
            if self.png { params["format"] = "png".into(); params.as_object_mut().map(|p| p.remove("quality")); }
            self.send("Page.startScreencast", params);
        }
        // Antes de ler: um quadro que chegue no meio ainda avisa a tela.
        self.surface.seen();
        let device = gpui_wgpu::WgpuContext::shared_device().map(|(device, _)| device);
        let shown = {
            let mut latest = lock(&self.surface.shown);
            if latest.as_ref().is_some_and(|s| Some(&s.device) != device.as_ref()) { *latest = None; }
            latest.as_ref().map(|s| s.texture.clone())
        };
        // Esticado até o painel: durante um resize o quadro do tamanho antigo cobre tudo até chegar o do novo.
        if let Some(texture) = shown { window.paint_surface(pixel_aligned(bounds, &texture, scale, frame_scale), Arc::new(texture)); }
    }

    /// A tela desenhou o último quadro: o próximo já pode ser decodificado.
    pub fn frame_seen(&self) { self.surface.seen(); }

    /// A textura mudou desde a última pergunta: um desenho guardado ainda mostraria a antiga.
    pub fn texture_replaced(&self) -> bool { self.surface.replaced.swap(false, Ordering::SeqCst) }

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

    pub fn insert_text(&self, text: &str) { self.send("Input.insertText", json!({"text": text})); }

    /// A página finge estar sempre focada (`setFocusEmulationEnabled`): o foco do painel não precisa ir a ela.
    pub fn focus(&self, _focused: bool) {}
    pub fn release_focus(&self) {}
}

/// Retângulo em pixels inteiros da tela, do tamanho do quadro: posição ou largura fracionada faz a GPU reamostrar a
/// textura inteira e o texto da página perde a nitidez.
fn pixel_aligned(bounds: Bounds<Pixels>, texture: &wgpu::Texture, scale: f32, frame_scale: f32) -> Bounds<Pixels> {
    let snap = |v: Pixels| px((f32::from(v) * scale).round() / scale);
    let origin = point(snap(bounds.origin.x), snap(bounds.origin.y));
    let fits = (texture.width() as f32 - f32::from(bounds.size.width) * frame_scale).abs() <= 2. * frame_scale
        && (texture.height() as f32 - f32::from(bounds.size.height) * frame_scale).abs() <= 2. * frame_scale;
    // Durante um resize o quadro antigo ainda tem outro tamanho: aí ele estica até o painel, como antes.
    let size = if fits { size(snap(px(texture.width() as f32 / frame_scale)), snap(px(texture.height() as f32 / frame_scale))) }
        else { size(snap(bounds.size.width), snap(bounds.size.height)) };
    Bounds { origin, size }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.surface.close();
        self.session.browser().own(&self.target, false);
        drop(self.session.browser().call(None, "Target.closeTarget", json!({"targetId": self.target})));
        if let Some(context) = &self.context {
            drop(self.session.browser().call(None, "Target.disposeBrowserContext", json!({"browserContextId": context})));
        }
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
    use super::{Keystroke, Modifiers, Slot, key_event};
    use serde_json::json;

    #[test]
    fn only_the_latest_frame_is_decoded_after_the_last_one_was_drawn() {
        let mut slot = Slot::default();
        slot.data = Some("a".into());
        slot.data = Some("b".into());
        assert_eq!(slot.next().as_deref(), Some("b"), "o quadro trocado antes de decodificar não volta");
        slot.unseen = true;
        slot.data = Some("c".into());
        slot.data = Some("d".into());
        assert_eq!(slot.next(), None, "quadro anterior ainda não desenhado: espera");
        slot.unseen = false;
        assert_eq!(slot.next().as_deref(), Some("d"));
        assert_eq!(slot.next(), None);
    }

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
