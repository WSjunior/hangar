//! Verbos do hangar-preview sobre CDP, portados de `shell/preview_ctl.cjs`. Um controlador por navegador: é ele quem
//! guarda refs, tema, layout e os registros de console e rede entre um comando e outro.
use std::{cell::{Cell, RefCell}, collections::{HashMap, HashSet, VecDeque}, future::Future, time::Instant};

use futures::{future::{Either, select}, lock::Mutex};
use serde_json::{Value, json};

use super::preview_fmt::{Layout, compact_ax, key_event, parse_layout};

pub trait Page {
    fn call(&self, method: &str, params: Value) -> impl Future<Output = Result<Value, String>>;
    fn sleep(&self, ms: u64) -> impl Future<Output = ()>;
}

pub enum Reply { Text(String), Png(Vec<u8>) }

pub const EVENTS: [&str; 6] = ["Runtime.consoleAPICalled", "Log.entryAdded", "Network.requestWillBeSent",
    "Network.responseReceived", "Network.loadingFailed", "Page.frameNavigated"];

const LOG_MAX: usize = 200;
const WAIT_MS: u64 = 15_000;
const SHOT_MS: u64 = 15_000;
const POLL_MS: u64 = 120;
/// Escondida, a página fica sem tamanho; um viewport fixo a mantém desenhando para o `shot`.
const HIDDEN_VIEWPORT: (u32, u32) = (1280, 800);
const MOBILE_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1";
/// Dois quadros: só o segundo garante que o que a ação mudou já está pintado. O timer cobre página escondida.
const FRAME: &str = "new Promise(r=>{requestAnimationFrame(()=>requestAnimationFrame(r));setTimeout(r,100)})";
/// '' = dá para digitar; senão, o que está com foco.
const FOCUS: &str = "(()=>{const a=document.activeElement;if(!a||a===document.body)return 'nada';const t=a.tagName.toLowerCase();const ok=a.isContentEditable||t==='textarea'||(t==='input'&&!/^(button|submit|reset|checkbox|radio|file|image|range|color)$/i.test(a.type||''));return ok?'':t+(a.type?'['+a.type+']':'')})()";
const PROBE_READ: &str = "(typeof window.__hangarSonda===\"undefined\"?2:window.__hangarSonda)";

struct State {
    refs: HashMap<String, i64>,
    console: VecDeque<String>,
    network: VecDeque<String>,
    inflight: u32,
    last_network: Instant,
    theme: &'static str,
    layout: Layout,
    hidden: bool,
    navigated: bool,
    enabled: HashSet<&'static str>,
}

pub struct Controller<P: Page> {
    page: P,
    state: RefCell<State>,
    /// Um verbo por vez: dois clientes (agente e tool MCP) não intercalam eventos.
    turn: Mutex<()>,
    /// Último `hidden` pedido: quem pega o `turn` aplica este, não o seu, porque a fila do `turn` não garante ordem.
    wanted_hidden: Cell<bool>,
}

fn push(list: &mut VecDeque<String>, line: String) {
    list.push_back(line);
    if list.len() > LOG_MAX { list.pop_front(); }
}

impl<P: Page> Controller<P> {
    pub fn new(page: P) -> Self {
        Self { page, turn: Mutex::new(()), wanted_hidden: Cell::new(false), state: RefCell::new(State { refs: HashMap::new(), console: VecDeque::new(),
            network: VecDeque::new(), inflight: 0, last_network: Instant::now(), theme: "sistema", layout: Layout::Desktop,
            hidden: false, navigated: false, enabled: HashSet::new() }) }
    }

    pub async fn start(&self, hidden: bool) {
        self.enable_base().await;
        self.set_hidden(hidden).await;
    }

    /// Sem Runtime/Log/Page o console e as refs morrem calados; `run` tenta de novo a cada verbo (já ligado não chama o CDP).
    async fn enable_base(&self) {
        for domain in ["Runtime", "Log", "Page"] {
            if let Err(e) = self.enable(domain).await { eprintln!("[nav] {e}"); }
        }
    }

    pub async fn set_hidden(&self, hidden: bool) {
        self.wanted_hidden.set(hidden);
        let _turn = self.turn.lock().await;
        self.state.borrow_mut().hidden = self.wanted_hidden.get();
        let layout = self.state.borrow().layout;
        if let Err(e) = self.apply_layout(layout).await { eprintln!("[nav] layout nao aplicado: {e}"); }
    }

    pub fn on_event(&self, method: &str, p: &Value) {
        let mut s = self.state.borrow_mut();
        match method {
            "Runtime.consoleAPICalled" => {
                let args: Vec<String> = p["args"].as_array().into_iter().flatten().map(|a| match a.get("value") {
                    None => a["description"].as_str().or(a["type"].as_str()).unwrap_or("").to_owned(),
                    Some(Value::String(t)) => t.clone(),
                    Some(v) => v.to_string(),
                }).collect();
                push(&mut s.console, format!("{}: {}", p["type"].as_str().unwrap_or("log"), args.join(" ")));
            }
            "Log.entryAdded" => push(&mut s.console, format!("{}: {}", p["entry"]["level"].as_str().unwrap_or(""), p["entry"]["text"].as_str().unwrap_or(""))),
            "Network.requestWillBeSent" => s.inflight += 1,
            "Network.responseReceived" => {
                s.inflight = s.inflight.saturating_sub(1);
                s.last_network = Instant::now();
                push(&mut s.network, format!("{} {}", p["response"]["status"], p["response"]["url"].as_str().unwrap_or("")));
            }
            "Network.loadingFailed" => s.inflight = s.inflight.saturating_sub(1),
            // Só o quadro principal: iframe navegando não invalida as refs da página.
            "Page.frameNavigated" if p["frame"]["parentId"].is_null() => s.navigated = true,
            _ => {}
        }
    }

    pub async fn run(&self, verb: &str, args: &[String]) -> Reply {
        let _turn = self.turn.lock().await;
        self.enable_base().await;
        self.after_navigation().await;
        let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
        let result = match verb {
            "snapshot" => self.snapshot().await.map(Reply::Text),
            "click" => self.click(arg(0)).await.map(Reply::Text),
            "fill" => self.fill(arg(0), arg(1)).await.map(Reply::Text),
            "type" => self.type_text(arg(0)).await.map(Reply::Text),
            "press" => self.press(arg(0)).await.map(Reply::Text),
            "hover" => self.hover(arg(0)).await.map(Reply::Text),
            "wait" => self.wait(args).await.map(Reply::Text),
            "eval" => self.eval(arg(0)).await.map(Reply::Text),
            "url" => self.eval("location.href").await.map(Reply::Text),
            "text" => self.value("document.body.innerText").await.map(|v| Reply::Text(v.as_str().unwrap_or("").to_owned())),
            "tema" => self.theme(arg(0)).await.map(Reply::Text),
            "layout" => self.layout(args).await.map(Reply::Text),
            "console" => Ok(Reply::Text(self.console(arg(0) == "--limpar"))),
            "network" => self.network().await.map(Reply::Text),
            "shot" => self.shot().await,
            "tab-list" | "tab-new" | "tab-switch" | "tab-close" => Ok(Reply::Text("erro: o app nativo ainda nao tem abas: e um navegador por sessao".into())),
            _ => Ok(Reply::Text(format!("erro: verbo desconhecido: {verb}"))),
        };
        result.unwrap_or_else(|e| Reply::Text(format!("erro: {e}")))
    }

    async fn after_navigation(&self) {
        if !std::mem::take(&mut self.state.borrow_mut().navigated) { return; }
        {
            let mut s = self.state.borrow_mut();
            s.refs.clear();
            // Requisição cancelada pela navegação nem sempre vira loadingFailed.
            s.inflight = 0;
        }
        // O tema emulado se perde ao navegar; o tamanho não.
        if self.state.borrow().theme != "sistema" && let Err(e) = self.apply_theme().await { eprintln!("[nav] tema nao reaplicado: {e}"); }
    }

    async fn enable(&self, domain: &'static str) -> Result<(), String> {
        if self.state.borrow().enabled.contains(domain) { return Ok(()); }
        self.page.call(&format!("{domain}.enable"), json!({})).await.map_err(|e| format!("{domain}.enable falhou: {e}"))?;
        let mut s = self.state.borrow_mut();
        s.enabled.insert(domain);
        if domain == "Network" { s.last_network = Instant::now(); }
        Ok(())
    }

    async fn within<T>(&self, ms: u64, fut: impl Future<Output = T>) -> Option<T> {
        match select(std::pin::pin!(fut), std::pin::pin!(self.page.sleep(ms))).await { Either::Left((v, _)) => Some(v), Either::Right(_) => None }
    }

    async fn frame(&self) {
        let _ = self.within(500, self.page.call("Runtime.evaluate", json!({"expression": FRAME, "awaitPromise": true}))).await;
    }

    async fn value(&self, expression: &str) -> Result<Value, String> {
        let r = self.page.call("Runtime.evaluate", json!({"expression": expression, "returnByValue": true})).await?;
        Ok(r["result"]["value"].clone())
    }

    async fn snapshot(&self) -> Result<String, String> {
        self.enable("Accessibility").await?;
        let tree = self.page.call("Accessibility.getFullAXTree", json!({})).await?;
        let out = compact_ax(tree["nodes"].as_array().map_or(&[][..], Vec::as_slice));
        self.state.borrow_mut().refs = out.refs;
        Ok(out.lines.join("\n"))
    }

    /// Ref velha e ref inexistente dão no mesmo lugar: o nó morre em qualquer re-render, e a saída é o mesmo snapshot novo.
    /// Outra falha do CDP (página travada, sessão caída) não é ref velha: sobe como erro, porque snapshot novo não resolve.
    async fn center(&self, r: &str) -> Result<Option<(i64, i64)>, String> {
        let Some(id) = self.state.borrow().refs.get(r).copied() else { return Ok(None) };
        let _ = self.page.call("DOM.scrollIntoViewIfNeeded", json!({"backendNodeId": id})).await;
        let model = match self.page.call("DOM.getBoxModel", json!({"backendNodeId": id})).await {
            Ok(m) => m,
            Err(e) if e.contains("No node") => return Ok(None),
            Err(e) => return Err(format!("{r}: {e}")),
        };
        let Some(content) = model["model"]["content"].as_array() else { return Ok(None) };
        let q: Vec<f64> = content.iter().filter_map(Value::as_f64).collect();
        Ok((q.len() >= 6).then(|| (((q[0] + q[4]) / 2.).round() as i64, ((q[1] + q[5]) / 2.).round() as i64)))
    }

    /// Ouvinte em captura prova que o EVENTO chegou ao documento. `pointerdown` antes de `mousedown`: um
    /// `preventDefault` no primeiro suprime o segundo (combobox do Radix).
    async fn arrived(&self, types: &[&str], fire: impl AsyncFn() -> Result<(), String>) -> Result<bool, String> {
        let arm = format!("(()=>{{window.__hangarSonda=0;const ts={};const f=()=>{{window.__hangarSonda=1;ts.forEach(t=>removeEventListener(t,f,true))}};ts.forEach(t=>addEventListener(t,f,true));return 1}})()",
            serde_json::to_string(types).unwrap_or_default());
        self.page.call("Runtime.evaluate", json!({"expression": arm})).await.map_err(|e| format!("sonda nao armada: {e}"))?;
        fire().await?;
        self.frame().await;
        // Marcador sumido (2) ou contexto da sonda destruído = documento novo: o evento navegou, logo chegou.
        // Outra falha de leitura não prova entrega.
        match self.value(PROBE_READ).await {
            Ok(v) => Ok(matches!(v.as_i64(), Some(1 | 2))),
            Err(e) if e.contains("Execution context was destroyed") || e.contains("Cannot find default execution context") => Ok(true),
            Err(e) => { eprintln!("[nav] sonda nao lida: {e}"); Ok(false) }
        }
    }

    async fn click(&self, r: &str) -> Result<String, String> {
        let Some((x, y)) = self.center(r).await? else { return Ok(format!("erro: ref {r} nao existe (rode snapshot de novo)")) };
        let base = |t: &str| json!({"type": t, "x": x, "y": y, "button": "left", "clickCount": 1});
        let ok = self.arrived(&["pointerdown", "mousedown"], async || {
            self.page.call("Input.dispatchMouseEvent", base("mousePressed")).await?;
            self.page.call("Input.dispatchMouseEvent", base("mouseReleased")).await.map(|_| ())
        }).await?;
        Ok(if ok { format!("ok: click {r}") } else { format!("erro: click {r}: o evento nao chegou na pagina (aba escondida sem quadro) — tire um shot e repita") })
    }

    async fn not_editable(&self) -> Result<Option<String>, String> {
        let v = self.value(FOCUS).await?;
        Ok(match v.as_str() { Some("") => None, Some(t) => Some(t.to_owned()), None => Some("desconhecido".into()) })
    }

    async fn fill(&self, r: &str, text: &str) -> Result<String, String> {
        let clicked = self.click(r).await?;
        if clicked.starts_with("erro:") { return Ok(clicked); }
        if let Some(focus) = self.not_editable().await? {
            return Ok(format!("erro: fill {r}: o clique nao deixou um campo de texto com foco (foco em {focus}) — a ref e mesmo um campo? rode snapshot"));
        }
        // `commands` seleciona o campo inteiro; insertText substitui a seleção, vazio incluído.
        self.page.call("Input.dispatchKeyEvent", json!({"type": "keyDown", "commands": ["selectAll"]})).await?;
        self.page.call("Input.insertText", json!({"text": text})).await?;
        self.frame().await;
        Ok(format!("ok: fill {r}"))
    }

    async fn type_text(&self, text: &str) -> Result<String, String> {
        if let Some(focus) = self.not_editable().await? {
            return Ok(format!("erro: type: nenhum campo de texto com foco (foco em {focus}) — use fill @eN, ou click no campo antes"));
        }
        self.page.call("Input.insertText", json!({"text": text})).await?;
        self.frame().await;
        Ok("ok: type".into())
    }

    async fn press(&self, key: &str) -> Result<String, String> {
        let down = key_event(key);
        let mut up = down.clone();
        if let Some(o) = up.as_object_mut() { o.remove("text"); o.remove("unmodifiedText"); }
        let ok = self.arrived(&["keydown"], async || {
            let mut d = down.clone();
            d["type"] = "keyDown".into();
            self.page.call("Input.dispatchKeyEvent", d).await?;
            let mut u = up.clone();
            u["type"] = "keyUp".into();
            self.page.call("Input.dispatchKeyEvent", u).await.map(|_| ())
        }).await?;
        Ok(if ok { format!("ok: press {key}") } else { format!("erro: press {key}: o evento nao chegou na pagina (aba escondida sem quadro) — tire um shot e repita") })
    }

    async fn hover(&self, r: &str) -> Result<String, String> {
        let Some((x, y)) = self.center(r).await? else { return Ok(format!("erro: ref {r} nao existe (rode snapshot de novo)")) };
        self.page.call("Input.dispatchMouseEvent", json!({"type": "mouseMoved", "x": x, "y": y})).await?;
        self.frame().await;
        Ok(format!("ok: hover {r}"))
    }

    async fn eval(&self, js: &str) -> Result<String, String> {
        let error = |r: &Value| r["exceptionDetails"]["exception"]["description"].as_str().or(r["exceptionDetails"]["text"].as_str()).map(str::to_owned);
        let wrapped = format!("(async()=>{{const v=({js});await {FRAME};return v}})()");
        let run = |params: Value| self.within(WAIT_MS, self.page.call("Runtime.evaluate", params));
        let Some(mut r) = run(json!({"expression": wrapped, "awaitPromise": true, "returnByValue": true})).await else {
            return Ok(format!("erro: eval nao respondeu em {WAIT_MS}ms"));
        };
        // Declaração não cabe entre parênteses: roda cru, como no console do DevTools.
        if r.as_ref().ok().and_then(error).is_some_and(|e| e.starts_with("SyntaxError")) {
            let Some(raw) = run(json!({"expression": js, "awaitPromise": true, "returnByValue": true, "replMode": true})).await else {
                return Ok(format!("erro: eval nao respondeu em {WAIT_MS}ms"));
            };
            r = raw;
            if r.as_ref().is_ok_and(|v| error(v).is_none()) { self.frame().await; }
        }
        let r = r?;
        if let Some(e) = error(&r) { return Ok(format!("erro: {e}")); }
        Ok(match r["result"].get("value") { Some(v) => format!("ok: {v}"), None => "ok: undefined".into() })
    }

    async fn wait(&self, args: &[String]) -> Result<String, String> {
        let target = args.first().map(String::as_str).unwrap_or("");
        let joined = args.join(" ");
        if let Some(ms) = target.bytes().all(|b| b.is_ascii_digit()).then(|| target.parse::<u64>().ok()).flatten() {
            self.page.sleep(ms).await;
            return Ok(format!("ok: wait {target}ms"));
        }
        let needle = args.get(1).map(String::as_str).unwrap_or("");
        // Alvo torto esperaria 15 s para dar um erro que já se sabe agora.
        let valid = match target {
            "--url" | "--text" => !needle.is_empty(),
            "--idle" => true,
            t => t.len() > 2 && t.starts_with("@e") && t[2..].bytes().all(|b| b.is_ascii_digit()),
        };
        if !valid { return Ok(format!("erro: wait: alvo invalido: {joined} (use @eN, --url texto, --text texto, --idle ou ms)")); }
        if target == "--idle" { self.enable("Network").await?; }
        // Conta o tempo das checagens além das pausas: página presa num laço de JS não responde ao
        // evaluate, e sem teto o verbo seguraria a vez de todos os outros.
        let mut waited = 0;
        while waited < WAIT_MS {
            let check = async {
                Ok::<_, String>(match target {
                    t if t.starts_with('@') => { self.snapshot().await?; self.state.borrow().refs.contains_key(t) }
                    "--url" => self.value("location.href").await?.as_str().is_some_and(|u| u.contains(needle)),
                    "--text" => self.value("document.body.innerText").await?.as_str().is_some_and(|u| u.contains(needle)),
                    // Rede parada (contador zero e 500 ms sem resposta) e documento completo.
                    "--idle" => {
                        let quiet = { let s = self.state.borrow(); s.inflight == 0 && s.last_network.elapsed().as_millis() > 500 };
                        quiet && self.value("document.readyState").await?.as_str() == Some("complete")
                    }
                    _ => false,
                })
            };
            let t0 = Instant::now();
            let Some(hit) = self.within(WAIT_MS.saturating_sub(waited), check).await else { break };
            if hit? { return Ok(format!("ok: wait {joined}")); }
            let spent = t0.elapsed().as_millis() as u64;
            self.page.sleep(POLL_MS).await;
            waited += POLL_MS + spent;
        }
        Ok(format!("erro: wait {joined} nao aconteceu em {WAIT_MS}ms"))
    }

    async fn apply_theme(&self) -> Result<(), String> {
        let value = match self.state.borrow().theme { "claro" => "light", "escuro" => "dark", _ => "" };
        let features = if value.is_empty() { json!([]) } else { json!([{"name": "prefers-color-scheme", "value": value}]) };
        self.page.call("Emulation.setEmulatedMedia", json!({"features": features})).await.map(|_| ())
    }

    async fn theme(&self, mode: &str) -> Result<String, String> {
        let Some(mode) = ["claro", "escuro", "sistema"].into_iter().find(|m| *m == mode) else { return Ok(format!("erro: tema desconhecido: {mode}")) };
        self.state.borrow_mut().theme = mode;
        self.apply_theme().await?;
        Ok(format!("tema: {mode}"))
    }

    async fn apply_layout(&self, layout: Layout) -> Result<(), String> {
        let metrics = |w: u32, h: u32, scale: u32, mobile: bool| json!({"width": w, "height": h, "deviceScaleFactor": scale, "mobile": mobile});
        if layout == Layout::Mobile {
            self.page.call("Emulation.setDeviceMetricsOverride", metrics(390, 844, 2, true)).await?;
            self.page.call("Emulation.setTouchEmulationEnabled", json!({"enabled": true, "maxTouchPoints": 5})).await?;
            self.page.call("Emulation.setUserAgentOverride", json!({"userAgent": MOBILE_UA})).await?;
            return Ok(());
        }
        self.page.call("Emulation.setTouchEmulationEnabled", json!({"enabled": false})).await?;
        self.page.call("Emulation.setUserAgentOverride", json!({"userAgent": ""})).await?;
        match layout {
            Layout::Custom(w, h) => self.page.call("Emulation.setDeviceMetricsOverride", metrics(w, h, 1, false)).await?,
            _ if self.state.borrow().hidden => self.page.call("Emulation.setDeviceMetricsOverride", metrics(HIDDEN_VIEWPORT.0, HIDDEN_VIEWPORT.1, 1, false)).await?,
            _ => self.page.call("Emulation.clearDeviceMetricsOverride", json!({})).await?,
        };
        Ok(())
    }

    async fn layout(&self, args: &[String]) -> Result<String, String> {
        if args.is_empty() { return Ok(format!("layout: {}", self.state.borrow().layout.label())); }
        let wanted = match parse_layout(args) { Ok(l) => l, Err(e) => return Ok(e) };
        let previous = self.state.borrow().layout;
        if let Err(e) = self.apply_layout(wanted).await {
            let _ = self.apply_layout(previous).await;
            return Ok(format!("erro: layout {}: {e}", args.join(" ")));
        }
        self.state.borrow_mut().layout = wanted;
        Ok(format!("layout: {}", wanted.label()))
    }

    fn console(&self, clear: bool) -> String {
        let mut s = self.state.borrow_mut();
        let out = s.console.iter().cloned().collect::<Vec<_>>().join("\n");
        if clear { s.console.clear(); }
        out
    }

    async fn network(&self) -> Result<String, String> {
        self.enable("Network").await?;
        Ok(self.state.borrow().network.iter().cloned().collect::<Vec<_>>().join("\n"))
    }

    async fn shot(&self) -> Result<Reply, String> {
        use base64::Engine as _;
        self.frame().await;
        let mut params = json!({"format": "png"});
        if let Layout::Custom(w, h) = self.state.borrow().layout {
            params = json!({"format": "png", "captureBeyondViewport": true, "clip": {"x": 0, "y": 0, "width": w, "height": h, "scale": 1}});
        }
        let Some(r) = self.within(SHOT_MS, self.page.call("Page.captureScreenshot", params)).await else {
            return Ok(Reply::Text("erro: o navegador desta sessao nao produziu quadro — text/snapshot/click funcionam".into()));
        };
        let data = r.map_err(|e| format!("captureScreenshot falhou: {e}"))?;
        let png = base64::engine::general_purpose::STANDARD.decode(data["data"].as_str().unwrap_or("")).map_err(|e| e.to_string())?;
        if png.is_empty() { return Ok(Reply::Text("erro: o navegador desta sessao nao produziu quadro — text/snapshot/click funcionam".into())); }
        Ok(Reply::Png(png))
    }
}

/// A página de verdade: o CDP do motor (WebView2 no Windows, Chromium sem janela no Linux) e o relógio da GPUI.
#[cfg(not(target_os = "macos"))]
pub struct CdpPage {
    pub cdp: std::rc::Rc<super::cdp::Cdp>,
    pub executor: gpui_kit::BackgroundExecutor,
}

#[cfg(not(target_os = "macos"))]
impl Page for CdpPage {
    fn call(&self, method: &str, params: Value) -> impl Future<Output = Result<Value, String>> { self.cdp.call(method, params) }
    fn sleep(&self, ms: u64) -> impl Future<Output = ()> {
        let timer = self.executor.timer(std::time::Duration::from_millis(ms));
        async move { timer.await; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use serde_json::json;
    use std::cell::RefCell;

    /// `stuck`: método que nunca responde, como página presa num laço de JS.
    struct Fake { calls: RefCell<Vec<(String, Value)>>, answer: Box<dyn Fn(&str, &Value) -> Result<Value, String>>, stuck: &'static str }
    impl Page for Fake {
        fn call(&self, method: &str, params: Value) -> impl Future<Output = Result<Value, String>> {
            self.calls.borrow_mut().push((method.into(), params.clone()));
            if method == self.stuck { Either::Left(std::future::pending()) } else { Either::Right(std::future::ready((self.answer)(method, &params))) }
        }
        fn sleep(&self, _: u64) -> impl Future<Output = ()> { std::future::ready(()) }
    }
    fn ctl(answer: impl Fn(&str, &Value) -> Result<Value, String> + 'static) -> Controller<Fake> {
        Controller::new(Fake { calls: RefCell::default(), answer: Box::new(answer), stuck: "" })
    }
    fn text(r: Reply) -> String { match r { Reply::Text(t) => t, Reply::Png(_) => "png".into() } }
    fn methods(c: &Controller<Fake>) -> Vec<String> { c.page.calls.borrow().iter().map(|(m, _)| m.clone()).collect() }
    fn s(v: &[&str]) -> Vec<String> { v.iter().map(|x| x.to_string()).collect() }

    fn tree() -> Value { json!({"nodes": [
        {"nodeId": "1", "role": {"value": "RootWebArea"}, "name": {"value": "x"}, "childIds": ["2"]},
        {"nodeId": "2", "role": {"value": "button"}, "name": {"value": "Ok"}, "backendDOMNodeId": 7, "childIds": []}]}) }

    #[test]
    fn snapshot_lists_refs() {
        let c = ctl(|m, _| Ok(if m == "Accessibility.getFullAXTree" { tree() } else { json!({}) }));
        assert_eq!(text(block_on(c.run("snapshot", &[]))), "- RootWebArea \"x\"\n  - button \"Ok\" [ref=@e1]");
    }

    #[test]
    fn click_with_stale_ref_asks_for_new_snapshot() {
        let c = ctl(|m, _| match m {
            "Accessibility.getFullAXTree" => Ok(tree()),
            "DOM.getBoxModel" => Err("No node with given id found".into()),
            _ => Ok(json!({})),
        });
        block_on(c.run("snapshot", &[]));
        assert_eq!(text(block_on(c.run("click", &s(&["@e1"])))), "erro: ref @e1 nao existe (rode snapshot de novo)");
        assert_eq!(text(block_on(c.run("click", &s(&["@e9"])))), "erro: ref @e9 nao existe (rode snapshot de novo)");
        assert!(!methods(&c).contains(&"Input.dispatchMouseEvent".to_string()));
    }

    #[test]
    fn click_confirms_the_event_arrived() {
        let c = ctl(|m, p| Ok(match m {
            "Accessibility.getFullAXTree" => tree(),
            "DOM.getBoxModel" => json!({"model": {"content": [0, 0, 20, 0, 20, 10, 0, 10]}}),
            // A sonda lida devolve 1 = o evento chegou.
            "Runtime.evaluate" if p["expression"].as_str().is_some_and(|e| e.starts_with("(typeof window.__hangarSonda")) => json!({"result": {"value": 1}}),
            _ => json!({}),
        }));
        block_on(c.run("snapshot", &[]));
        assert_eq!(text(block_on(c.run("click", &s(&["@e1"])))), "ok: click @e1");
        let presses: Vec<Value> = c.page.calls.borrow().iter().filter(|(m, _)| m == "Input.dispatchMouseEvent").map(|(_, p)| p.clone()).collect();
        assert_eq!(presses[0]["type"], "mousePressed");
        assert_eq!((presses[0]["x"].as_i64(), presses[0]["y"].as_i64()), (Some(10), Some(5)));
    }

    #[test]
    fn click_that_never_arrives_is_an_error() {
        let c = ctl(|m, p| Ok(match m {
            "Accessibility.getFullAXTree" => tree(),
            "DOM.getBoxModel" => json!({"model": {"content": [0, 0, 20, 0, 20, 10, 0, 10]}}),
            "Runtime.evaluate" if p["expression"].as_str().is_some_and(|e| e.starts_with("(typeof window.__hangarSonda")) => json!({"result": {"value": 0}}),
            _ => json!({}),
        }));
        block_on(c.run("snapshot", &[]));
        assert!(text(block_on(c.run("click", &s(&["@e1"])))).starts_with("erro: click @e1: o evento nao chegou"));
    }

    fn is_probe_read(p: &Value) -> bool { p["expression"].as_str().is_some_and(|e| e.starts_with("(typeof window.__hangarSonda")) }
    fn is_probe_arm(p: &Value) -> bool { p["expression"].as_str().is_some_and(|e| e.starts_with("(()=>{window.__hangarSonda")) }
    fn boxed() -> Value { json!({"model": {"content": [0, 0, 20, 0, 20, 10, 0, 10]}}) }

    #[test]
    fn click_with_unreadable_probe_is_not_confirmed() {
        let c = ctl(|m, p| match m {
            "Accessibility.getFullAXTree" => Ok(tree()),
            "DOM.getBoxModel" => Ok(boxed()),
            "Runtime.evaluate" if is_probe_read(p) => Err("Internal error".into()),
            _ => Ok(json!({})),
        });
        block_on(c.run("snapshot", &[]));
        assert!(text(block_on(c.run("click", &s(&["@e1"])))).starts_with("erro: click @e1: o evento nao chegou"));
    }

    #[test]
    fn click_that_navigates_away_counts_as_delivered() {
        let c = ctl(|m, p| match m {
            "Accessibility.getFullAXTree" => Ok(tree()),
            "DOM.getBoxModel" => Ok(boxed()),
            "Runtime.evaluate" if is_probe_read(p) => Err("Execution context was destroyed.".into()),
            _ => Ok(json!({})),
        });
        block_on(c.run("snapshot", &[]));
        assert_eq!(text(block_on(c.run("click", &s(&["@e1"])))), "ok: click @e1");
    }

    #[test]
    fn click_with_unarmed_probe_is_an_error() {
        let c = ctl(|m, p| match m {
            "Accessibility.getFullAXTree" => Ok(tree()),
            "DOM.getBoxModel" => Ok(boxed()),
            "Runtime.evaluate" if is_probe_arm(p) => Err("Cannot find context".into()),
            _ => Ok(json!({})),
        });
        block_on(c.run("snapshot", &[]));
        assert_eq!(text(block_on(c.run("click", &s(&["@e1"])))), "erro: sonda nao armada: Cannot find context");
        assert!(!methods(&c).contains(&"Input.dispatchMouseEvent".to_string()));
    }

    #[test]
    fn box_model_failure_other_than_missing_node_surfaces() {
        let c = ctl(|m, _| match m {
            "Accessibility.getFullAXTree" => Ok(tree()),
            "DOM.getBoxModel" => Err("Could not compute box model.".into()),
            _ => Ok(json!({})),
        });
        block_on(c.run("snapshot", &[]));
        assert_eq!(text(block_on(c.run("hover", &s(&["@e1"])))), "erro: @e1: Could not compute box model.");
    }

    #[test]
    fn wait_with_invalid_target_fails_at_once() {
        let c = ctl(|_, _| Ok(json!({})));
        for args in [&["--url"][..], &["--texto", "x"], &["@x1"], &[]] {
            assert_eq!(text(block_on(c.run("wait", &s(args)))),
                format!("erro: wait: alvo invalido: {} (use @eN, --url texto, --text texto, --idle ou ms)", args.join(" ")));
        }
        assert!(!methods(&c).contains(&"Runtime.evaluate".to_string()));
    }

    #[test]
    fn type_without_editable_focus_is_refused() {
        let c = ctl(|m, _| Ok(if m == "Runtime.evaluate" { json!({"result": {"value": "button"}}) } else { json!({}) }));
        assert_eq!(text(block_on(c.run("type", &s(&["oi"])))),
            "erro: type: nenhum campo de texto com foco (foco em button) — use fill @eN, ou click no campo antes");
        assert!(!methods(&c).contains(&"Input.insertText".to_string()));
    }

    #[test]
    fn eval_falls_back_to_statements_on_syntax_error() {
        let c = ctl(|m, p| Ok(if m != "Runtime.evaluate" { json!({}) }
            else if p["replMode"] == true { json!({"result": {"value": 2}}) }
            else if p["expression"].as_str().is_some_and(|e| e.starts_with("(async()=>")) { json!({"exceptionDetails": {"exception": {"description": "SyntaxError: Unexpected token 'const'"}}}) }
            else { json!({"result": {}}) }));
        assert_eq!(text(block_on(c.run("eval", &s(&["const a=1; a+1"])))), "ok: 2");
    }

    #[test]
    fn eval_without_value_prints_undefined() {
        let c = ctl(|_, _| Ok(json!({"result": {}})));
        assert_eq!(text(block_on(c.run("eval", &s(&["void 0"])))), "ok: undefined");
    }

    #[test]
    fn console_keeps_the_last_200_and_clears() {
        let c = ctl(|_, _| Ok(json!({})));
        for i in 0..205 { c.on_event("Runtime.consoleAPICalled", &json!({"type": "log", "args": [{"value": i}]})); }
        let out = text(block_on(c.run("console", &[])));
        assert_eq!(out.lines().count(), 200);
        assert!(out.starts_with("log: 5\n"));
        block_on(c.run("console", &s(&["--limpar"])));
        assert_eq!(text(block_on(c.run("console", &[]))), "");
    }

    #[test]
    fn wait_gives_up_with_the_electron_text() {
        let c = ctl(|_, _| Ok(json!({"result": {"value": "http://a/"}})));
        assert_eq!(text(block_on(c.run("wait", &s(&["--url", "/done"])))), "erro: wait --url /done nao aconteceu em 15000ms");
        assert_eq!(text(block_on(c.run("wait", &s(&["300"])))), "ok: wait 300ms");
    }

    #[test]
    fn wait_gives_up_when_the_page_never_answers() {
        let c = Controller::new(Fake { calls: RefCell::default(), answer: Box::new(|_, _| Ok(json!({}))), stuck: "Runtime.evaluate" });
        assert_eq!(text(block_on(c.run("wait", &s(&["--text", "x"])))), "erro: wait --text x nao aconteceu em 15000ms");
    }

    #[test]
    fn hidden_desktop_uses_fixed_viewport() {
        let c = ctl(|_, _| Ok(json!({})));
        block_on(c.set_hidden(true));
        let last = c.page.calls.borrow().iter().rev().find(|(m, _)| m == "Emulation.setDeviceMetricsOverride").cloned();
        assert_eq!(last.map(|(_, p)| (p["width"].clone(), p["height"].clone())), Some((json!(1280), json!(800))));
        block_on(c.set_hidden(false));
        assert_eq!(methods(&c).last().map(String::as_str), Some("Emulation.clearDeviceMetricsOverride"));
    }

    #[test]
    fn late_hidden_request_never_overrides_a_newer_one() {
        let c = ctl(|_, _| Ok(json!({})));
        let busy = block_on(c.turn.lock());
        let waker = futures::task::noop_waker();
        let mut cx = std::task::Context::from_waker(&waker);
        let (mut hide, mut show) = (Box::pin(c.set_hidden(true)), Box::pin(c.set_hidden(false)));
        assert!(hide.as_mut().poll(&mut cx).is_pending() && show.as_mut().poll(&mut cx).is_pending());
        drop(busy);
        // O pedido antigo pega o `turn` por último e ainda assim aplica o mais novo (visível).
        block_on(show);
        block_on(hide);
        assert!(!c.state.borrow().hidden);
        assert_eq!(methods(&c).last().map(String::as_str), Some("Emulation.clearDeviceMetricsOverride"));
    }

    #[test]
    fn layout_and_theme_answer_like_electron() {
        let c = ctl(|_, _| Ok(json!({})));
        assert_eq!(text(block_on(c.run("layout", &[]))), "layout: desktop");
        assert_eq!(text(block_on(c.run("layout", &s(&["390", "844"])))), "layout: 390x844");
        assert_eq!(text(block_on(c.run("tema", &s(&["escuro"])))), "tema: escuro");
        assert_eq!(text(block_on(c.run("tema", &s(&["roxo"])))), "erro: tema desconhecido: roxo");
    }

    #[test]
    fn navigation_drops_refs() {
        let c = ctl(|m, _| Ok(if m == "Accessibility.getFullAXTree" { tree() } else { json!({}) }));
        block_on(c.run("snapshot", &[]));
        c.on_event("Page.frameNavigated", &json!({"frame": {"id": "main"}}));
        assert_eq!(text(block_on(c.run("hover", &s(&["@e1"])))), "erro: ref @e1 nao existe (rode snapshot de novo)");
    }

    #[test]
    fn runs_are_serialized() {
        let c = ctl(|_, _| Ok(json!({"result": {"value": "x"}})));
        let (a, b) = block_on(futures::future::join(c.run("eval", &s(&["1"])), c.run("text", &[])));
        assert_eq!((text(a), text(b)), ("ok: \"x\"".into(), "x".into()));
        assert_eq!(methods(&c).iter().filter(|m| !m.ends_with(".enable")).count(), 2);
    }

    #[test]
    fn tabs_and_unknown_verbs_are_refused() {
        let c = ctl(|_, _| Ok(json!({})));
        assert!(text(block_on(c.run("tab-list", &[]))).starts_with("erro: o app nativo ainda nao tem abas"));
        assert_eq!(text(block_on(c.run("voar", &[]))), "erro: verbo desconhecido: voar");
    }
}
