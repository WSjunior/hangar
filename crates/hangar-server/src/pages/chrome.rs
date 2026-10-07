//! Chromium sem janela do servidor: mede a altura da página, guarda o console e tira print.
//! Porta de depuração em vez de pipe: o pipe por fd 3/4 não existe no Windows.
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio_tungstenite::tungstenite::{Message, protocol::WebSocketConfig};

use super::Theme;

const DEADLINE: Duration = Duration::from_secs(8);
const CONSOLE_MAX: usize = 50;
const LINE_MAX: usize = 500;
static SLOTS: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(2));

/// Mesma conta do script `HOST` (theme.rs), para a altura medida aqui bater com a que a página anuncia.
const MEASURE: &str = "(()=>{const d=document.documentElement,b=document.body;\
return Math.ceil(d.scrollHeight>d.clientHeight?d.scrollHeight:Math.max(d.getBoundingClientRect().height,b?b.getBoundingClientRect().height:0))})()";
// Recurso externo que nunca responde segura o `load`: passados 3 s, mede o que já desenhou.
const LOADED: &str = "Promise.race([new Promise(r=>document.readyState===\"complete\"?r():addEventListener(\"load\",()=>r(),{once:true})),\
new Promise(r=>setTimeout(r,3000))])";

pub struct Job { pub width: u32, pub theme: Theme, pub shot: Option<PathBuf> }
pub struct Rendered { pub heights: BTreeMap<u32, u32>, pub console: Vec<ConsoleLine> }
#[derive(Serialize, Clone, Debug)]
pub struct ConsoleLine { pub level: String, pub text: String }
#[derive(Debug)]
pub enum ChromeError { Absent, Failed(&'static str) }
impl ChromeError {
    pub fn status(&self) -> &'static str { match self { ChromeError::Absent => "ausente", ChromeError::Failed(_) => "falhou" } }
    pub fn reason(&self) -> &'static str { match self { ChromeError::Absent => "sem Chromium no servidor", ChromeError::Failed(r) => r } }
}

#[cfg(target_os = "macos")]
const KNOWN: &[&str] = &["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/Applications/Chromium.app/Contents/MacOS/Chromium"];
#[cfg(target_os = "windows")]
const KNOWN: &[&str] = &[r"C:\Program Files\Google\Chrome\Application\chrome.exe", r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const KNOWN: &[&str] = &[];
#[cfg(not(target_os = "windows"))]
const ON_PATH: &[&str] = &["google-chrome-stable", "google-chrome", "chromium", "chromium-browser"];
#[cfg(target_os = "windows")]
const ON_PATH: &[&str] = &["chrome.exe", "msedge.exe"];

/// Mesma variável e mesmo baixado do app nativo; depois a marca do `install-chromium.sh`, o PATH e os caminhos fixos.
pub fn find() -> Option<PathBuf> {
    let native = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".hangar/native"));
    let downloaded = native.as_ref().map(|n| n.join("chromium/chrome-headless-shell"));
    let marker = native.and_then(|n| std::fs::read_to_string(n.join("chromium-ok")).ok());
    let mut known: Vec<PathBuf> = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) { for n in ON_PATH { known.push(dir.join(n)); } }
    }
    known.extend(KNOWN.iter().map(PathBuf::from));
    // Chrome instalado só para o usuário fica no perfil dele, não em Program Files.
    #[cfg(target_os = "windows")]
    known.extend(std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join(r"Google\Chrome\Application\chrome.exe")));
    find_with(std::env::var_os("HANGAR_CHROMIUM"), downloaded, marker.as_deref(), &known)
}

fn find_with(env: Option<OsString>, downloaded: Option<PathBuf>, marker: Option<&str>, known: &[PathBuf]) -> Option<PathBuf> {
    // Pedido explícito e inválido não cai para outro navegador: quem definiu a variável quer saber que errou.
    if let Some(p) = env.map(PathBuf::from) { return p.is_file().then_some(p); }
    downloaded.filter(|p| p.is_file())
        .or_else(|| marker.and_then(marker_path).filter(|p| p.is_file()))
        .or_else(|| known.iter().find(|p| p.is_file()).cloned())
}

/// A marca é `<palavra>: <caminho>`, às vezes seguida de ` (<versão>…)`; as linhas sem caminho não passam no `is_file`.
fn marker_path(text: &str) -> Option<PathBuf> {
    let (_, rest) = text.lines().next()?.split_once(": ")?;
    let path = rest.split_once(" (").map_or(rest, |(p, _)| p).trim();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

pub async fn render(html: &str, jobs: &[Job]) -> Result<Rendered, ChromeError> {
    render_with(find(), html, jobs).await
}

pub async fn render_with(bin: Option<PathBuf>, html: &str, jobs: &[Job]) -> Result<Rendered, ChromeError> {
    let bin = bin.ok_or(ChromeError::Absent)?;
    let _slot = SLOTS.acquire().await.map_err(|_| ChromeError::Failed("fila do navegador fechada"))?;
    tokio::time::timeout(DEADLINE, run(&bin, html, jobs)).await.map_err(|_| ChromeError::Failed("navegador do servidor passou do prazo"))?
}

async fn run(bin: &Path, html: &str, jobs: &[Job]) -> Result<Rendered, ChromeError> {
    let profile = tempfile::tempdir().map_err(|_| ChromeError::Failed("perfil temporário"))?;
    let mut child = tokio::process::Command::new(bin)
        .args(["--headless", "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check", "--hide-scrollbars",
               "--disable-gpu", "--mute-audio", "--font-render-hinting=none"])
        .arg(format!("--user-data-dir={}", profile.path().display()))
        .arg("about:blank")
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn().map_err(|_| ChromeError::Failed("navegador do servidor não subiu"))?;
    let result = match ws_url(profile.path(), &mut child).await {
        Ok(url) => drive(&url, html, jobs).await,
        Err(e) => Err(e),
    };
    let _ = child.kill().await;
    result
}

async fn ws_url(profile: &Path, child: &mut tokio::process::Child) -> Result<String, ChromeError> {
    let file = profile.join("DevToolsActivePort");
    for _ in 0..100 {
        if let Ok(Some(_)) = child.try_wait() { return Err(ChromeError::Failed("navegador do servidor morreu na partida")); }
        if let Ok(text) = tokio::fs::read_to_string(&file).await {
            let mut lines = text.lines();
            if let (Some(port), Some(path)) = (lines.next(), lines.next()) { return Ok(format!("ws://127.0.0.1:{port}{path}")); }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(ChromeError::Failed("navegador do servidor não anunciou a porta"))
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Cdp { ws: Ws, next_id: u64, console: Vec<ConsoleLine> }

impl Cdp {
    async fn send(&mut self, session: Option<&str>, method: &str, params: Value) -> Result<u64, ChromeError> {
        self.next_id += 1;
        let mut msg = json!({"id": self.next_id, "method": method, "params": params});
        if let Some(s) = session { msg["sessionId"] = json!(s); }
        self.ws.send(Message::text(msg.to_string())).await.map_err(|_| ChromeError::Failed("CDP do servidor caiu"))?;
        Ok(self.next_id)
    }

    /// Lê até a resposta do mesmo `id`; no caminho, bloqueia `file:` e guarda o console.
    async fn call(&mut self, session: Option<&str>, method: &str, params: Value) -> Result<Value, ChromeError> {
        let id = self.send(session, method, params).await?;
        loop {
            let msg = self.ws.next().await.ok_or(ChromeError::Failed("CDP do servidor caiu"))?
                .map_err(|_| ChromeError::Failed("CDP do servidor caiu"))?;
            let Message::Text(text) = msg else { continue };
            let Ok(mut v) = serde_json::from_str::<Value>(text.as_str()) else { continue };
            if v["id"].as_u64() == Some(id) {
                if v.get("error").is_some() { return Err(ChromeError::Failed("CDP do servidor recusou um comando")); }
                return Ok(v.get_mut("result").map(Value::take).unwrap_or(Value::Null));
            }
            self.event(&v).await?;
        }
    }

    async fn event(&mut self, v: &Value) -> Result<(), ChromeError> {
        match v["method"].as_str() {
            Some("Fetch.requestPaused") => {
                let params = json!({"requestId": v["params"]["requestId"], "errorReason": "BlockedByClient"});
                self.send(v["sessionId"].as_str(), "Fetch.failRequest", params).await?;
            }
            Some(m @ ("Runtime.consoleAPICalled" | "Runtime.exceptionThrown")) if self.console.len() < CONSOLE_MAX => {
                self.console.push(console_line(m, &v["params"]));
            }
            _ => {}
        }
        Ok(())
    }
}

fn console_line(method: &str, p: &Value) -> ConsoleLine {
    let (level, text) = if method == "Runtime.exceptionThrown" {
        let d = &p["exceptionDetails"];
        ("error".to_owned(), d["exception"]["description"].as_str().or(d["text"].as_str()).unwrap_or_default().to_owned())
    } else {
        let text = p["args"].as_array().map(|args| args.iter().map(arg_text).collect::<Vec<_>>().join(" ")).unwrap_or_default();
        (p["type"].as_str().unwrap_or("log").to_owned(), text)
    };
    ConsoleLine { level, text: text.chars().take(LINE_MAX).collect() }
}

fn arg_text(a: &Value) -> String {
    match &a["value"] {
        Value::String(s) => s.clone(),
        Value::Null => a["description"].as_str().or(a["unserializableValue"].as_str())
            .or(a["subtype"].as_str()).or(a["type"].as_str()).unwrap_or_default().to_owned(),
        v => v.to_string(),
    }
}

async fn drive(url: &str, html: &str, jobs: &[Job]) -> Result<Rendered, ChromeError> {
    // Print de página alta em escala 2 passa do limite de quadro padrão do tungstenite.
    let config = WebSocketConfig::default().max_frame_size(None).max_message_size(None);
    let (ws, _) = tokio_tungstenite::connect_async_with_config(url, Some(config), true).await
        .map_err(|_| ChromeError::Failed("CDP do servidor recusou a conexão"))?;
    let mut cdp = Cdp { ws, next_id: 0, console: Vec::new() };
    let target = cdp.call(None, "Target.createTarget", json!({"url": "about:blank"})).await?["targetId"].as_str().unwrap_or_default().to_owned();
    let session = cdp.call(None, "Target.attachToTarget", json!({"targetId": target, "flatten": true})).await?["sessionId"].as_str().unwrap_or_default().to_owned();
    let s = Some(session.as_str());
    cdp.call(s, "Page.enable", json!({})).await?;
    cdp.call(s, "Runtime.enable", json!({})).await?;
    cdp.call(s, "Fetch.enable", json!({"patterns": [{"urlPattern": "file:*"}]})).await?;
    cdp.call(s, "Emulation.setDefaultBackgroundColorOverride", json!({"color": {"r": 0, "g": 0, "b": 0, "a": 0}})).await?;
    let frame = cdp.call(s, "Page.getFrameTree", json!({})).await?["frameTree"]["frame"]["id"].as_str().unwrap_or_default().to_owned();
    cdp.call(s, "Page.setDocumentContent", json!({"frameId": frame, "html": html})).await?;
    cdp.call(s, "Runtime.evaluate", json!({"expression": LOADED, "awaitPromise": true})).await?;
    let mut heights = BTreeMap::new();
    for job in jobs {
        cdp.call(s, "Emulation.setEmulatedMedia", json!({"features": [{"name": "prefers-color-scheme", "value": job.theme.as_str()}]})).await?;
        // Viewport baixa: com ela alta, `scrollHeight` nunca fica abaixo dela e a página curta mediria alta.
        cdp.call(s, "Emulation.setDeviceMetricsOverride", json!({"width": job.width, "height": super::HEIGHT_MIN, "deviceScaleFactor": 2, "mobile": false})).await?;
        tokio::time::sleep(Duration::from_millis(150)).await;
        let h = cdp.call(s, "Runtime.evaluate", json!({"expression": MEASURE, "returnByValue": true})).await?;
        let height = (h["result"]["value"].as_f64().unwrap_or(0.) as u32).clamp(super::HEIGHT_MIN, super::HEIGHT_MAX);
        heights.insert(job.width, height);
        if let Some(path) = &job.shot {
            let shot = cdp.call(s, "Page.captureScreenshot", json!({"format": "png", "captureBeyondViewport": true,
                "clip": {"x": 0, "y": 0, "width": job.width, "height": height, "scale": 1}})).await?;
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::STANDARD.decode(shot["data"].as_str().unwrap_or("")).map_err(|_| ChromeError::Failed("print inválido"))?;
            super::store::write_atomic(path, &bytes).map_err(|_| ChromeError::Failed("print não gravado"))?;
        }
    }
    Ok(Rendered { heights, console: cdp.console })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, "").unwrap();
        p
    }

    #[test]
    fn marker_formats_of_the_installer() {
        assert_eq!(marker_path("sistema: /x/chrome\n"), Some(PathBuf::from("/x/chrome")));
        assert_eq!(marker_path("HANGAR_CHROMIUM: /opt/c\n"), Some(PathBuf::from("/opt/c")));
        assert_eq!(marker_path("baixado: /x/chrome (131.0.6778.85)\n"), Some(PathBuf::from("/x/chrome")));
        assert_eq!(marker_path("baixado: /x/chrome (131.0; renovar falhou: sem rede)\n"), Some(PathBuf::from("/x/chrome")));
        assert_eq!(marker_path("sem chromium (aarch64)\n"), None);
    }

    #[test]
    fn env_override_wins() {
        let dir = tempfile::tempdir().unwrap();
        let bin = touch(dir.path(), "chrome");
        let shell = touch(dir.path(), "shell");
        assert_eq!(find_with(Some(bin.clone().into_os_string()), Some(shell), None, &[]), Some(bin));
    }

    #[test]
    fn env_set_but_missing_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        let shell = touch(dir.path(), "shell");
        let known = [touch(dir.path(), "known")];
        assert_eq!(find_with(Some(dir.path().join("nope").into_os_string()), Some(shell), None, &known), None);
    }

    #[test]
    fn downloaded_before_marker() {
        let dir = tempfile::tempdir().unwrap();
        let shell = touch(dir.path(), "shell");
        let sys = touch(dir.path(), "sys");
        let marker = format!("sistema: {}\n", sys.display());
        assert_eq!(find_with(None, Some(shell.clone()), Some(&marker), &[]), Some(shell));
    }

    #[test]
    fn marker_points_to_binary() {
        let dir = tempfile::tempdir().unwrap();
        let bin = touch(dir.path(), "c");
        let known = [touch(dir.path(), "known")];
        let marker = format!("baixado: {} (131.0.6778.85)\n", bin.display());
        assert_eq!(find_with(None, Some(dir.path().join("absent")), Some(&marker), &known), Some(bin));
    }

    #[test]
    fn marker_without_binary_falls_to_known() {
        let dir = tempfile::tempdir().unwrap();
        let known = [dir.path().join("missing"), touch(dir.path(), "known")];
        assert_eq!(find_with(None, None, Some("download falhou: sem rede\n"), &known), Some(known[1].clone()));
    }

    #[test]
    fn nothing_found_is_absent() {
        assert_eq!(find_with(None, None, None, &[]), None);
    }

    #[tokio::test]
    async fn no_binary_is_absent() {
        let r = render_with(None, "<p>x</p>", &[Job { width: 360, theme: Theme::Dark, shot: None }]).await;
        assert_eq!(r.err().map(|e| e.status()), Some("ausente"));
    }

    #[test]
    fn console_lines_from_cdp_events() {
        let log = console_line("Runtime.consoleAPICalled", &json!({"type": "warning", "args": [{"type": "string", "value": "a"}, {"type": "number", "value": 2}, {"type": "undefined"}]}));
        assert_eq!((log.level.as_str(), log.text.as_str()), ("warning", "a 2 undefined"));
        let exc = console_line("Runtime.exceptionThrown", &json!({"exceptionDetails": {"text": "Uncaught", "exception": {"description": "Error: x"}}}));
        assert_eq!((exc.level.as_str(), exc.text.as_str()), ("error", "Error: x"));
        let long = console_line("Runtime.consoleAPICalled", &json!({"type": "log", "args": [{"type": "string", "value": "é".repeat(900)}]}));
        assert_eq!(long.text.chars().count(), LINE_MAX);
    }

    /// Alfa do primeiro pixel de um PNG RGBA 8 bits: o primeiro pixel da primeira linha sai cru em qualquer filtro.
    fn corner_alpha(png: &[u8]) -> u8 {
        use std::io::Read as _;
        let (mut at, mut idat) = (8, Vec::new());
        while at + 8 <= png.len() {
            let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            let kind = &png[at + 4..at + 8];
            let data = &png[at + 8..at + 8 + len];
            if kind == b"IHDR" { assert_eq!((data[8], data[9]), (8, 6), "PNG não é RGBA 8 bits"); }
            if kind == b"IDAT" { idat.extend_from_slice(data); }
            at += 12 + len;
        }
        let mut raw = [0u8; 5];
        flate2::read::ZlibDecoder::new(&idat[..]).read_exact(&mut raw).unwrap();
        raw[4]
    }

    #[tokio::test]
    #[ignore = "sobe o Chrome da máquina"]
    async fn measures_real_page() {
        let dir = tempfile::tempdir().unwrap();
        let shot = dir.path().join("s.png");
        let html = super::super::theme::inject("<!doctype html><div style=\"height:333px\"></div><script>console.error('oi')</script>");
        dbg!(find());
        let r = render(&html, &[Job { width: 728, theme: Theme::Dark, shot: Some(shot.clone()) }]).await.unwrap();
        dbg!(&r.heights, &r.console, &shot);
        assert_eq!(r.heights[&728], 333);
        assert!(r.console.iter().any(|c| c.level == "error" && c.text.contains("oi")));
        let png = std::fs::read(&shot).unwrap();
        assert!(!png.is_empty());
        assert_eq!(corner_alpha(&png), 0, "fundo do print não é transparente");
    }
}
