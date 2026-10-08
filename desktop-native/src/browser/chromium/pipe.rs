//! Um Chromium por processo do app, compartilhado pelos navegadores das sessões, e as sessões CDP sobre o pipe dele.
//! Respostas voltam direto da thread de leitura; eventos são entregues na thread da interface, como no WebView2.
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    future::Future,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::Child,
    rc::{Rc, Weak},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};

use futures::future::{FutureExt, LocalBoxFuture, Shared};
use gpui_kit::{BackgroundExecutor, ForegroundExecutor, Task};
use serde_json::{Value, json};

use super::launch;

type Reply = Result<Value, String>;
type Handler = Rc<RefCell<dyn FnMut(Value)>>;
/// Consome um evento na própria thread de leitura (o quadro do painel, entregue sem passar pela interface).
pub type Sink = Box<dyn Fn(Value) + Send>;
type Ready = Shared<LocalBoxFuture<'static, Result<(), String>>>;
const LAUNCH_WAIT: Duration = Duration::from_secs(10);

/// O lado que qualquer thread usa: escrever e casar respostas.
struct Wire {
    writer: Mutex<std::io::PipeWriter>,
    next: AtomicU64,
    /// id → (sessão, quem espera).
    pending: Mutex<HashMap<u64, (Option<String>, Box<dyn FnOnce(Reply) + Send>)>>,
    /// Quadro de screencast por sessão, tratado antes de chegar à interface.
    sinks: Mutex<HashMap<String, Sink>>,
}

impl Wire {
    fn send(&self, session: Option<&str>, method: &str, params: Value, done: Option<Box<dyn FnOnce(Reply) + Send>>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut message = json!({"id": id, "method": method, "params": params});
        if let Some(session) = session { message["sessionId"] = session.into(); }
        if let Some(done) = done { lock(&self.pending).insert(id, (session.map(str::to_owned), done)); }
        let mut bytes = message.to_string().into_bytes();
        bytes.push(0);
        if let Err(e) = lock(&self.writer).write_all(&bytes) {
            // Sem o pipe não haverá resposta: quem espera recebe o erro agora.
            if let Some((_, done)) = lock(&self.pending).remove(&id) { done(Err(format!("o Chromium fechou: {e}"))); }
        }
    }

    /// Sessão que caiu não responde o que já estava em curso: quem espera por ela recebe o erro agora.
    fn fail(&self, session: &str) {
        let gone: Vec<_> = lock(&self.pending).extract_if(|_, (s, _)| s.as_deref() == Some(session)).collect();
        for (_, (_, done)) in gone { done(Err("a pagina fechou".into())); }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> { m.lock().unwrap_or_else(PoisonError::into_inner) }

/// Texto do erro do CDP, o mesmo que o WebView2 devolve: o controlador reconhece "No node", "SyntaxError" etc.
fn reply_of(message: &Value) -> Reply {
    match message.get("error") {
        Some(error) => Err(error["message"].as_str().map_or_else(|| error.to_string(), str::to_owned)),
        None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
    }
}

pub struct Browser {
    wire: Arc<Wire>,
    child: RefCell<Child>,
    /// (sessão, evento) → ouvintes.
    handlers: RefCell<HashMap<(String, String), Vec<Handler>>>,
    /// Evento do navegador (`Target.*`, `CLOSED`) → (sessão dona, ouvinte); sai junto com a sessão dona.
    watchers: RefCell<HashMap<String, Vec<(String, Handler)>>>,
    /// Abas que o Chromium abre sozinho ao nascer; o primeiro navegador as fecha.
    initial: RefCell<Vec<String>>,
    /// Páginas que os navegadores do app abriram; qualquer outra é janela que a página abriu.
    owned: RefCell<HashSet<String>>,
    alive: std::cell::Cell<bool>,
    /// Aperto de mão do processo que nasceu sem travar a interface; `None` quando já respondeu.
    ready: RefCell<Option<Ready>>,
    _drain: RefCell<Option<Task<()>>>,
}

thread_local! {
    static SHARED: RefCell<Weak<Browser>> = RefCell::new(Weak::new());
}

impl Browser {
    /// O Chromium em uso, ou um novo. Nasce com a escala da janela que abriu o primeiro navegador.
    pub fn shared(executor: &ForegroundExecutor, scale: f32) -> Result<Rc<Browser>, String> {
        if let Some(browser) = Self::current() { return Ok(browser); }
        let bin = launch::find()?;
        let browser = Self::launch(bin, &Self::profile()?, scale, executor)?;
        SHARED.with(|s| *s.borrow_mut() = Rc::downgrade(&browser));
        Ok(browser)
    }

    /// Como `shared`, sem esperar o processo responder: o aperto de mão corre sozinho e `ready` o espera. Para a
    /// interface não congelar enquanto o Chromium nasce.
    pub fn shared_async(executor: &ForegroundExecutor, background: &BackgroundExecutor, scale: f32) -> Result<Rc<Browser>, String> {
        if let Some(browser) = Self::current() { return Ok(browser); }
        let bin = launch::find()?;
        let browser = Self::spawn(bin, &Self::profile()?, scale, executor)?;
        let handshake = browser.handshake(bin, background).boxed_local().shared();
        // Corre mesmo sem ninguém esperar: quem só aquece o processo não fica olhando.
        executor.spawn(handshake.clone()).detach();
        *browser.ready.borrow_mut() = Some(handshake);
        SHARED.with(|s| *s.borrow_mut() = Rc::downgrade(&browser));
        Ok(browser)
    }

    /// Resolve quando o processo já respondeu ao aperto de mão (na hora, se nasceu bloqueando).
    pub fn ready(&self) -> impl Future<Output = Result<(), String>> + use<> {
        let pending = self.ready.borrow().clone();
        async move { match pending { Some(ready) => ready.await, None => Ok(()) } }
    }

    fn current() -> Option<Rc<Browser>> { SHARED.with(|s| s.borrow().upgrade()).filter(|b| b.alive.get()) }

    fn profile() -> Result<std::path::PathBuf, String> {
        crate::appearance::dir().map(|d| d.join("chromium")).ok_or_else(|| "sem pasta de configuracao do app".into())
    }

    /// As chamadas saem já, nesta ordem: o `getTargets` chega antes de qualquer `createTarget` de quem vier depois, e
    /// as abas iniciais continuam sendo só as do nascimento.
    fn handshake(self: &Rc<Self>, bin: &Path, background: &BackgroundExecutor) -> impl Future<Output = Result<(), String>> + use<> {
        let version = self.call(None, "Browser.getVersion", json!({}));
        let discover = self.call(None, "Target.setDiscoverTargets", json!({"discover": true}));
        let targets = self.call(None, "Target.getTargets", json!({}));
        let (weak, timer, bin) = (Rc::downgrade(self), background.timer(LAUNCH_WAIT), bin.display().to_string());
        async move {
            let answers = async { Ok::<_, String>((version.await?, discover.await?, targets.await?)) };
            let answers = match futures::future::select(std::pin::pin!(answers), std::pin::pin!(timer)).await {
                futures::future::Either::Left((answers, _)) => answers,
                futures::future::Either::Right(_) => Err("tempo esgotado".into()),
            };
            let Some(browser) = weak.upgrade() else { return Err("o Chromium fechou".into()) };
            match answers {
                Ok((_, _, targets)) => {
                    *browser.initial.borrow_mut() = targets["targetInfos"].as_array().into_iter().flatten()
                        .filter(|t| t["type"] == "page").filter_map(|t| t["targetId"].as_str().map(str::to_owned)).collect();
                    drop(browser.call(None, "Browser.setDownloadBehavior", json!({"behavior": "deny"})));
                    Ok(())
                }
                Err(e) => {
                    // Processo mudo não fica como o compartilhado: o próximo pedido sobe outro.
                    browser.alive.set(false);
                    Err(format!("o Chromium ({bin}) nao respondeu: {e}"))
                }
            }
        }
    }

    fn launch(bin: &Path, profile: &Path, scale: f32, executor: &ForegroundExecutor) -> Result<Rc<Browser>, String> {
        let browser = Self::spawn(bin, profile, scale, executor)?;
        browser.call_blocking(None, "Browser.getVersion", json!({}), Duration::from_secs(10))
            .map_err(|e| format!("o Chromium ({}) nao respondeu: {e}", bin.display()))?;
        // Títulos e endereços das páginas, e janelas abertas por `window.open`.
        browser.call_blocking(None, "Target.setDiscoverTargets", json!({"discover": true}), Duration::from_secs(5))?;
        let targets = browser.call_blocking(None, "Target.getTargets", json!({}), Duration::from_secs(5))?;
        *browser.initial.borrow_mut() = targets["targetInfos"].as_array().into_iter().flatten()
            .filter(|t| t["type"] == "page").filter_map(|t| t["targetId"].as_str().map(str::to_owned)).collect();
        drop(browser.call(None, "Browser.setDownloadBehavior", json!({"behavior": "deny"})));
        Ok(browser)
    }

    /// Processo, thread de leitura e entrega dos eventos; nenhuma espera pelo CDP.
    fn spawn(bin: &Path, profile: &Path, scale: f32, executor: &ForegroundExecutor) -> Result<Rc<Browser>, String> {
        let launched = launch::spawn(bin, profile, scale).map_err(|e| format!("o Chromium ({}) nao abriu: {e}", bin.display()))?;
        let wire = Arc::new(Wire {
            writer: Mutex::new(launched.writer),
            next: AtomicU64::new(1),
            pending: Mutex::default(),
            sinks: Mutex::default(),
        });
        let (events, received) = async_channel::unbounded::<Value>();
        let reading = wire.clone();
        std::thread::Builder::new().name("chromium-pipe".into())
            .spawn(move || read(launched.reader, &reading, &events))
            .map_err(|e| e.to_string())?;
        let browser = Rc::new(Browser {
            wire, child: RefCell::new(launched.child), handlers: RefCell::default(), watchers: RefCell::default(),
            initial: RefCell::default(), owned: RefCell::default(), alive: std::cell::Cell::new(true), ready: RefCell::new(None),
            _drain: RefCell::new(None),
        });
        let weak = Rc::downgrade(&browser);
        *browser._drain.borrow_mut() = Some(executor.spawn(async move {
            while let Ok(message) = received.recv().await {
                let Some(browser) = weak.upgrade() else { break };
                browser.dispatch(message);
            }
            if let Some(browser) = weak.upgrade() { browser.closed(); }
        }));
        Ok(browser)
    }

    pub fn alive(&self) -> bool { self.alive.get() }

    pub fn own(&self, target: &str, owned: bool) {
        if owned { self.owned.borrow_mut().insert(target.to_owned()); } else { self.owned.borrow_mut().remove(target); }
    }

    pub fn owns(&self, target: &str) -> bool { self.owned.borrow().contains(target) }

    /// Fecha as abas que nasceram com o processo, depois que já existe outra (sem nenhuma o Chromium sairia).
    pub fn close_initial(&self) {
        for target in self.initial.take() { drop(self.call(None, "Target.closeTarget", json!({"targetId": target}))); }
    }

    fn closed(&self) {
        self.alive.set(false);
        self.notify(CLOSED, Value::Null);
    }

    fn notify(&self, method: &str, params: Value) {
        // Clonados antes de chamar: o ouvinte pode registrar outro sem o mapa estar emprestado.
        let watchers: Vec<Handler> = self.watchers.borrow().get(method).into_iter().flatten().map(|(_, h)| h.clone()).collect();
        for handler in watchers { (handler.borrow_mut())(params.clone()); }
    }

    fn dispatch(&self, message: Value) {
        let Some(method) = message["method"].as_str() else { return };
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let Some(session) = message["sessionId"].as_str() else { return self.notify(method, params) };
        let handlers = self.handlers.borrow().get(&(session.to_owned(), method.to_owned())).cloned().unwrap_or_default();
        for handler in handlers { (handler.borrow_mut())(params.clone()); }
    }

    /// Envia já; o futuro só espera a resposta. Descartar o futuro não cancela o comando.
    pub fn call(&self, session: Option<&str>, method: &str, params: Value) -> impl Future<Output = Reply> + use<> {
        let (tx, rx) = futures::channel::oneshot::channel();
        self.wire.send(session, method, params, Some(Box::new(move |reply| { let _ = tx.send(reply); })));
        async move { rx.await.map_err(|_| "o Chromium fechou".to_string())? }
    }

    /// Para o nascimento do navegador, que acontece numa chamada síncrona da interface. A resposta vem da thread de
    /// leitura, então esperar aqui não trava nada além desta chamada.
    pub fn call_blocking(&self, session: Option<&str>, method: &str, params: Value, timeout: Duration) -> Reply {
        let (tx, rx) = mpsc::channel();
        self.wire.send(session, method, params, Some(Box::new(move |reply| { let _ = tx.send(reply); })));
        rx.recv_timeout(timeout).map_err(|_| format!("{method}: o Chromium nao respondeu"))?
    }

    pub fn on(&self, session: &str, event: &str, f: impl FnMut(Value) + 'static) {
        let handler: Handler = Rc::new(RefCell::new(f));
        self.handlers.borrow_mut().entry((session.to_owned(), event.to_owned())).or_default().push(handler);
    }

    /// Evento do navegador, ouvido enquanto a sessão `owner` existir.
    pub fn watch(&self, owner: &str, event: &str, f: impl FnMut(Value) + 'static) {
        let handler: Handler = Rc::new(RefCell::new(f));
        self.watchers.borrow_mut().entry(event.to_owned()).or_default().push((owner.to_owned(), handler));
    }

    pub fn sink(&self, session: &str, sink: Sink) { lock(&self.wire.sinks).insert(session.to_owned(), sink); }

    fn forget(&self, session: &str) {
        self.handlers.borrow_mut().retain(|(s, _), _| s != session);
        for list in self.watchers.borrow_mut().values_mut() { list.retain(|(owner, _)| owner != session); }
        lock(&self.wire.sinks).remove(session);
    }
}

/// Evento sintético entregue a quem ouve o navegador quando o pipe fecha.
pub const CLOSED: &str = "Hangar.closed";

impl Drop for Browser {
    fn drop(&mut self) {
        // Saída limpa primeiro: o perfil é persistente, e o SIGKILL perde cookies e localStorage ainda não gravados.
        self.wire.send(None, "Browser.close", json!({}), None);
        let mut child = self.child.borrow_mut();
        for _ in 0..40 {
            if !matches!(child.try_wait(), Ok(None)) { return; }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Lê mensagens terminadas em NUL. Respostas resolvem quem espera; quadros com dono vão ao dono; o resto vai à interface.
fn read(reader: std::io::PipeReader, wire: &Wire, events: &async_channel::Sender<Value>) {
    let mut reader = BufReader::with_capacity(1 << 20, reader);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        match reader.read_until(0, &mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if buffer.last() == Some(&0) { buffer.pop(); }
        let Ok(message) = serde_json::from_slice::<Value>(&buffer) else { continue };
        route(message, wire, events);
    }
    for (_, (_, done)) in lock(&wire.pending).drain() { done(Err("o Chromium fechou".into())); }
    events.close();
}

fn route(mut message: Value, wire: &Wire, events: &async_channel::Sender<Value>) {
    if let Some(id) = message["id"].as_u64() {
        if let Some((_, done)) = lock(&wire.pending).remove(&id) { done(reply_of(&message)); }
        return;
    }
    if message["method"] == "Page.screencastFrame"
        && let Some(session) = message["sessionId"].as_str().map(str::to_owned)
        && lock(&wire.sinks).contains_key(&session)
    {
        // Os parâmetros vão inteiros ao dono, sem copiar o quadro.
        let params = message["params"].take();
        let ack = json!({"sessionId": params["sessionId"]});
        if let Some(sink) = lock(&wire.sinks).get(&session) { sink(params); }
        // Sem o ack o Chromium para de mandar quadros.
        wire.send(Some(&session), "Page.screencastFrameAck", ack, None);
        return;
    }
    let _ = events.try_send(message);
}

/// Uma sessão CDP num alvo, com a mesma forma do `cdp::Cdp` do WebView2. Cair a sessão a desliga do alvo.
pub struct Session {
    browser: Rc<Browser>,
    id: String,
}

impl Session {
    pub fn attach(browser: &Rc<Browser>, target: &str) -> Result<Session, String> {
        let attached = browser.call_blocking(None, "Target.attachToTarget", json!({"targetId": target, "flatten": true}), Duration::from_secs(5))?;
        let id = attached["sessionId"].as_str().ok_or("attachToTarget sem sessionId")?.to_owned();
        Ok(Session { browser: browser.clone(), id })
    }

    /// `attach` sem travar a interface.
    pub fn attach_async(browser: &Rc<Browser>, target: &str) -> impl Future<Output = Result<Session, String>> + use<> {
        let (call, browser) = (browser.call(None, "Target.attachToTarget", json!({"targetId": target, "flatten": true})), browser.clone());
        async move {
            let id = call.await?["sessionId"].as_str().ok_or("attachToTarget sem sessionId")?.to_owned();
            Ok(Session { browser, id })
        }
    }

    pub fn id(&self) -> &str { &self.id }
    pub fn browser(&self) -> &Rc<Browser> { &self.browser }

    /// O alvo caiu ou a sessão se soltou: as chamadas em curso recebem erro em vez de esperar para sempre.
    pub fn fail_pending(&self) { self.browser.wire.fail(&self.id); }

    pub fn call(&self, method: &str, params: Value) -> impl Future<Output = Reply> + use<> {
        self.browser.call(Some(&self.id), method, params)
    }

    pub fn call_blocking(&self, method: &str, params: Value) -> Reply {
        self.browser.call_blocking(Some(&self.id), method, params, Duration::from_secs(5))
    }

    pub fn on(&self, event: &str, f: impl FnMut(Value) + 'static) -> Result<(), String> {
        self.browser.on(&self.id, event, f);
        Ok(())
    }

    /// Evento do navegador inteiro, ouvido enquanto esta sessão existir.
    pub fn watch(&self, event: &str, f: impl FnMut(Value) + 'static) { self.browser.watch(&self.id, event, f); }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.browser.forget(&self.id);
        drop(self.browser.call(None, "Target.detachFromTarget", json!({"sessionId": self.id})));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire() -> (Arc<Wire>, std::io::PipeReader) {
        let (reader, writer) = std::io::pipe().unwrap();
        (Arc::new(Wire { writer: Mutex::new(writer), next: AtomicU64::new(1), pending: Mutex::default(), sinks: Mutex::default() }), reader)
    }

    #[test]
    fn messages_are_nul_terminated_and_carry_the_session() {
        let (wire, reader) = wire();
        wire.send(Some("S1"), "Page.enable", json!({}), None);
        wire.send(None, "Browser.getVersion", json!({}), None);
        drop(wire);
        let mut all = Vec::new();
        std::io::Read::read_to_end(&mut { reader }, &mut all).unwrap();
        let parts: Vec<Value> = all.split(|b| *b == 0).filter(|p| !p.is_empty()).map(|p| serde_json::from_slice(p).unwrap()).collect();
        assert_eq!(parts[0], json!({"id": 1, "method": "Page.enable", "params": {}, "sessionId": "S1"}));
        assert_eq!(parts[1], json!({"id": 2, "method": "Browser.getVersion", "params": {}}));
    }

    #[test]
    fn replies_resolve_by_id_and_errors_keep_the_cdp_text() {
        let (wire, _reader) = wire();
        let (events, received) = async_channel::unbounded();
        let (tx, rx) = mpsc::channel();
        let ok = tx.clone();
        wire.send(None, "A", json!({}), Some(Box::new(move |r| ok.send(r).unwrap())));
        wire.send(None, "B", json!({}), Some(Box::new(move |r| tx.send(r).unwrap())));
        route(json!({"id": 2, "error": {"code": -32000, "message": "No node with given id found"}}), &wire, &events);
        route(json!({"id": 1, "result": {"v": 1}}), &wire, &events);
        assert_eq!(rx.recv().unwrap(), Err("No node with given id found".into()));
        assert_eq!(rx.recv().unwrap(), Ok(json!({"v": 1})));
        route(json!({"method": "Page.frameNavigated", "sessionId": "S", "params": {}}), &wire, &events);
        assert_eq!(received.try_recv().unwrap()["method"], "Page.frameNavigated");
    }

    #[test]
    fn a_dropped_session_fails_only_its_own_calls() {
        let (wire, _reader) = wire();
        let (tx, rx) = mpsc::channel();
        for s in [Some("S"), Some("T"), None] {
            let tx = tx.clone();
            wire.send(s, "A", json!({}), Some(Box::new(move |r| tx.send((s, r)).unwrap())));
        }
        wire.fail("S");
        assert_eq!(rx.try_recv().unwrap(), (Some("S"), Err("a pagina fechou".into())));
        assert!(rx.try_recv().is_err());
        assert_eq!(lock(&wire.pending).len(), 2);
    }

    /// Prova do modo página: fundo transparente chega ao quadro PNG do screencast com alfa 0.
    #[test]
    #[ignore = "sobe o Chromium da máquina"]
    fn page_mode_png_frames_keep_the_transparent_background() {
        let bin = launch::find().unwrap();
        let profile = std::env::temp_dir().join(format!("hangar-page-proof-{}", std::process::id()));
        let launched = launch::spawn(bin, &profile, 1.0).unwrap();
        let wire = Arc::new(Wire { writer: Mutex::new(launched.writer), next: AtomicU64::new(1), pending: Mutex::default(), sinks: Mutex::default() });
        let (events, _received) = async_channel::unbounded();
        let reading = wire.clone();
        std::thread::spawn(move || read(launched.reader, &reading, &events));
        let call = |session: Option<&str>, method: &str, params: Value| -> Value {
            let (tx, rx) = mpsc::channel();
            wire.send(session, method, params, Some(Box::new(move |r| { let _ = tx.send(r); })));
            rx.recv_timeout(Duration::from_secs(15)).unwrap().unwrap_or_else(|e| panic!("{method}: {e}"))
        };
        let context = call(None, "Target.createBrowserContext", json!({"disposeOnDetach": true}))["browserContextId"].as_str().unwrap().to_owned();
        let target = call(None, "Target.createTarget", json!({"url": "about:blank", "newWindow": true, "browserContextId": context}))["targetId"]
            .as_str().unwrap().to_owned();
        let session = call(None, "Target.attachToTarget", json!({"targetId": target, "flatten": true}))["sessionId"].as_str().unwrap().to_owned();
        let s = Some(session.as_str());
        let window = call(None, "Browser.getWindowForTarget", json!({"targetId": target}))["windowId"].as_i64().unwrap();
        call(s, "Page.enable", json!({}));
        call(s, "Emulation.setDefaultBackgroundColorOverride", json!({"color": {"r": 0, "g": 0, "b": 0, "a": 0}}));
        call(None, "Browser.setWindowBounds", json!({"windowId": window, "bounds": {"width": 400, "height": 300}}));
        let frame = call(s, "Page.getFrameTree", json!({}))["frameTree"]["frame"]["id"].as_str().unwrap().to_owned();
        let html = r#"<style>html,body{margin:0;background:transparent}</style>
            <div style="position:absolute;left:50px;top:50px;width:100px;height:100px;background:rgb(255,0,0)"></div>"#;
        call(s, "Page.setDocumentContent", json!({"frameId": frame, "html": html}));
        let decode = |data: &str| {
            let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data).unwrap();
            image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).unwrap().into_rgba8()
        };
        let shot = decode(call(s, "Page.captureScreenshot", json!({"format": "png"}))["data"].as_str().unwrap());
        eprintln!("captureScreenshot {:?}: canto {:?}, caixa {:?}", shot.dimensions(), shot.get_pixel(5, 5), shot.get_pixel(100, 100));
        let (tx, rx) = mpsc::channel();
        lock(&wire.sinks).insert(session.clone(), Box::new(move |p: Value| { let _ = tx.send(p["data"].as_str().unwrap_or("").to_owned()); }));
        call(s, "Page.startScreencast", json!({"format": "png", "maxWidth": 400, "maxHeight": 300, "everyNthFrame": 1}));
        let pixels = decode(&rx.recv_timeout(Duration::from_secs(15)).unwrap());
        eprintln!("screencast {:?}: canto {:?}, caixa {:?}", pixels.dimensions(), pixels.get_pixel(5, 5), pixels.get_pixel(100, 100));
        drop(call(None, "Target.disposeBrowserContext", json!({"browserContextId": context})));
        wire.send(None, "Browser.close", json!({}), None);
        let _ = std::fs::remove_dir_all(&profile);
        assert_eq!(pixels.get_pixel(5, 5)[3], 0, "fundo deveria ser transparente");
        assert_eq!(pixels.get_pixel(100, 100).0, [255, 0, 0, 255], "caixa deveria ser opaca");
    }

    #[test]
    fn frames_with_an_owner_skip_the_interface_and_are_acked() {
        let (wire, reader) = wire();
        let (events, received) = async_channel::unbounded();
        let (tx, rx) = mpsc::channel();
        lock(&wire.sinks).insert("P".into(), Box::new(move |p: Value| tx.send(p["data"].clone()).unwrap()));
        route(json!({"method": "Page.screencastFrame", "sessionId": "P", "params": {"data": "QQ==", "sessionId": 9}}), &wire, &events);
        route(json!({"method": "Page.screencastFrame", "sessionId": "V", "params": {"data": "Qg==", "sessionId": 3}}), &wire, &events);
        assert_eq!(rx.recv().unwrap(), "QQ==");
        // O quadro de outra sessão (o espectador do celular) segue para a interface, sem ack daqui.
        assert_eq!(received.try_recv().unwrap()["sessionId"], "V");
        drop(wire);
        let mut all = Vec::new();
        std::io::Read::read_to_end(&mut { reader }, &mut all).unwrap();
        let ack: Value = serde_json::from_slice(all.split(|b| *b == 0).next().unwrap()).unwrap();
        assert_eq!((ack["method"].as_str(), ack["sessionId"].as_str(), ack["params"]["sessionId"].as_i64()), (Some("Page.screencastFrameAck"), Some("P"), Some(9)));
    }
}
