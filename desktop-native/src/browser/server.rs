//! Servidor do hangar-preview dentro do app nativo: mesmo contrato do `shell/preview_srv.cjs` (porta efêmera, token
//! sorteado, `~/.hangar/nav/_srv.json`), para o CLI, o backend e o Jev não mudarem.
use std::{path::PathBuf, sync::Arc, time::Duration};

use futures::channel::oneshot;
use serde_json::{Value, json};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpStream, runtime::Runtime};

use super::{control::Reply, preview_fmt::sidecar_name};

const HEAD_MAX: usize = 16 * 1024;
const BODY_MAX: usize = 128 * 1024;
/// O verbo mais lento (wait, eval, shot) tem teto de 15 s; este cobre a fila de dois deles.
const REPLY_SECS: u64 = 45;

pub enum Request {
    /// Verbo do `/cmd`, o do hangar-preview.
    Command { key: String, verb: String, args: Vec<String>, tab: Option<i64>, reply: oneshot::Sender<Reply> },
    /// Comando CDP do `/cdp`, já conferido contra `relay::RELAYED`.
    Cdp { key: String, method: String, params: Value, reply: oneshot::Sender<Result<Value, String>> },
    /// Espectador novo da tela remota; tira o anterior daquele navegador.
    Watch { key: String, viewer: super::relay::Viewer, reply: oneshot::Sender<Result<(), String>> },
    /// O espectador `id` saiu; não vale se outro já o substituiu.
    Unwatch { key: String, id: u64 },
}

#[derive(Debug)]
struct Head { method: String, path: String, auth: Option<String>, length: usize, upgrade: Option<String>, ws_key: Option<String> }

#[derive(Debug, PartialEq, Eq)]
enum Route { Cmd, Cdp(String) }

struct Body { key: String, verb: String, args: Vec<String>, tab: Option<i64> }

fn nav_dir() -> Option<PathBuf> { std::env::home_dir().map(|h| h.join(".hangar").join("nav")) }

/// Escrita em tmp + rename: o CLI nunca lê um arquivo pela metade.
fn write_json(name: &str, value: &Value) -> std::io::Result<()> {
    let dir = nav_dir().ok_or_else(|| std::io::Error::other("sem pasta do usuario"))?;
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    // O `_srv.json` leva o token: só o dono lê, como no shell Electron.
    #[cfg(unix)]
    {
        use std::{io::Write, os::unix::fs::{DirBuilderExt, OpenOptionsExt}};
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
        std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?.write_all(value.to_string().as_bytes())?;
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(&tmp, value.to_string())?;
    }
    std::fs::rename(tmp, dir.join(format!("{name}.json")))
}

fn now_ms() -> u128 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis()) }

/// Sidecar no formato do Electron (`url`/`targetId` no topo, `abas` ao lado), com o `pid` do dono para o `list`.
pub fn write_sidecar(key: &str, url: &str, title: &str) {
    let tab = json!({"id": 1, "url": url, "titulo": title, "targetId": null});
    let value = json!({"chave": key, "url": url, "targetId": null, "ts": now_ms(), "ativa": 1, "abas": [tab], "pid": std::process::id()});
    if let Err(e) = write_json(&sidecar_name(key), &value) { eprintln!("[nav] sidecar de {key} nao gravado: {e}"); }
}

pub fn remove_sidecar(key: &str) {
    let Some(dir) = nav_dir() else { return };
    match std::fs::remove_file(dir.join(format!("{}.json", sidecar_name(key)))) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => eprintln!("[nav] sidecar de {key} nao apagado: {e}"),
        _ => {}
    }
}

/// Só o app nativo grava `pid`: sidecar do Electron (sem `pid`) nunca é tocado.
fn is_stale_native_sidecar(value: &Value, pid: u32) -> bool {
    value.get("pid").is_some_and(|p| p.as_u64() != Some(u64::from(pid)))
}

/// O app não apaga os sidecars ao sair; sem isto, cada reinício deixa navegadores fantasmas para o CLI e o backend.
fn drop_stale_sidecars() {
    let Some(dir) = nav_dir() else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    for path in entries.flatten().map(|e| e.path()) {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with(['_', '.']) || !name.ends_with(".json") { continue; }
        let Some(value) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else { continue };
        if is_stale_native_sidecar(&value, std::process::id()) && let Err(e) = std::fs::remove_file(&path) {
            eprintln!("[nav] sidecar antigo {name} nao apagado: {e}");
        }
    }
}

fn same(a: &[u8], b: &[u8]) -> bool { a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0 }

fn new_token() -> std::io::Result<String> {
    use ring::rand::SecureRandom;
    let mut bytes = [0u8; 24];
    ring::rand::SystemRandom::new().fill(&mut bytes).map_err(|_| std::io::Error::other("sem aleatoriedade"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn parse_head(text: &str) -> Result<Head, (u16, String)> {
    let mut lines = text.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let (method, path) = (first.next().unwrap_or("").to_owned(), first.next().unwrap_or("").to_owned());
    let mut head = Head { method, path, auth: None, length: 0, upgrade: None, ws_key: None };
    for line in lines {
        let Some((name, value)) = line.split_once(':') else { continue };
        match name.trim().to_ascii_lowercase().as_str() {
            "authorization" => head.auth = Some(value.trim().to_owned()),
            "upgrade" => head.upgrade = Some(value.trim().to_owned()),
            "sec-websocket-key" => head.ws_key = Some(value.trim().to_owned()),
            "content-length" => head.length = value.trim().parse().map_err(|_| (400, "erro: corpo invalido".to_string()))?,
            _ => {}
        }
    }
    if head.length > BODY_MAX { return Err((413, "erro: corpo grande demais".into())); }
    Ok(head)
}

fn check(head: &Head, token: &str) -> Result<Route, (u16, String)> {
    let (path, query) = head.path.split_once('?').unwrap_or((&head.path, ""));
    let route = match (head.method.as_str(), path) {
        ("POST", "/cmd") => Route::Cmd,
        ("GET", "/cdp") => Route::Cdp(url::form_urlencoded::parse(query.as_bytes())
            .find_map(|(name, value)| (name == "chave").then(|| value.into_owned())).unwrap_or_default()),
        _ => return Err((404, "erro: rota desconhecida".into())),
    };
    if !same(head.auth.as_deref().unwrap_or("").as_bytes(), format!("Bearer {token}").as_bytes()) { return Err((401, "erro: token invalido".into())); }
    if let Route::Cdp(key) = &route {
        if key.is_empty() { return Err((400, "erro: /cdp precisa de ?chave=".into())); }
        if !head.upgrade.as_deref().is_some_and(|u| u.eq_ignore_ascii_case("websocket")) || head.ws_key.is_none() {
            return Err((400, "erro: /cdp so aceita WebSocket".into()));
        }
    }
    Ok(route)
}

fn parse_body(bytes: &[u8]) -> Result<Body, String> {
    let invalid = || "erro: corpo invalido".to_string();
    let v: Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let key = v["chave"].as_str().ok_or_else(invalid)?.to_owned();
    let verb = v["verbo"].as_str().ok_or_else(invalid)?.to_owned();
    let args = v["args"].as_array().into_iter().flatten().map(|a| a.as_str().map_or_else(|| a.to_string(), str::to_owned)).collect();
    Ok(Body { key, verb, args, tab: v["aba"].as_i64() })
}

pub fn start(runtime: &Runtime, requests: async_channel::Sender<Request>) -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let token = new_token()?;
    // Converte antes de publicar a porta: servidor que não sobe nunca aparece no `_srv.json`.
    let listener = { let _guard = runtime.enter(); tokio::net::TcpListener::from_std(listener)? };
    drop_stale_sidecars();
    write_json("_srv", &json!({"porta": port, "token": token, "pid": std::process::id(), "ts": now_ms()}))?;
    let token: Arc<str> = token.into();
    runtime.spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => { tokio::spawn(serve(stream, token.clone(), requests.clone())); }
                // Erro de accept (ex.: limite de sockets) é passageiro: sair do laço mataria o servidor calado.
                Err(e) => { eprintln!("[nav] accept falhou: {e}"); tokio::time::sleep(Duration::from_millis(100)).await; }
            }
        }
    });
    Ok(port)
}

async fn serve(mut stream: TcpStream, token: Arc<str>, requests: async_channel::Sender<Request>) {
    let (status, text) = match tokio::time::timeout(Duration::from_secs(10), read_request(&mut stream)).await {
        Err(_) => return,
        Ok(Err(fail)) => fail,
        Ok(Ok((head, body))) => match check(&head, &token) {
            Err(fail) => fail,
            Ok(Route::Cmd) => (200, answer(&body, &requests).await),
            Ok(Route::Cdp(key)) => {
                let ws_key = head.ws_key.unwrap_or_default();
                return super::relay::serve(stream, &ws_key, key, requests).await;
            }
        },
    };
    respond(&mut stream, status, &text).await;
}

/// Resposta HTTP de texto, uma por conexão, como o servidor do Electron.
pub(super) async fn respond(stream: &mut TcpStream, status: u16, text: &str) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        _ => "Internal Server Error",
    };
    let response = format!("HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len());
    let _ = stream.write_all(response.as_bytes()).await;
}

async fn read_request(stream: &mut TcpStream) -> Result<(Head, Vec<u8>), (u16, String)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") { break i; }
        if buf.len() > HEAD_MAX { return Err((431, "erro: cabecalho grande demais".into())); }
        let n = stream.read(&mut chunk).await.map_err(|e| (500, format!("erro: {e}")))?;
        if n == 0 { return Err((400, "erro: corpo invalido".into())); }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = parse_head(&String::from_utf8_lossy(&buf[..end]))?;
    let mut body = buf.split_off(end + 4);
    while body.len() < head.length {
        let n = stream.read(&mut chunk).await.map_err(|e| (500, format!("erro: {e}")))?;
        if n == 0 { break; }
        body.extend_from_slice(&chunk[..n]);
        if body.len() > BODY_MAX { return Err((413, "erro: corpo grande demais".into())); }
    }
    body.truncate(head.length);
    Ok((head, body))
}

async fn answer(bytes: &[u8], requests: &async_channel::Sender<Request>) -> String {
    let body = match parse_body(bytes) { Ok(b) => b, Err(e) => return e };
    let path = (body.verb == "shot").then(|| body.args.first().cloned()).flatten();
    if body.verb == "shot" && path.is_none() { return "erro: shot precisa de um caminho de arquivo".into(); }
    let (tx, rx) = oneshot::channel();
    if requests.send(Request::Command { key: body.key, verb: body.verb, args: body.args, tab: body.tab, reply: tx }).await.is_err() {
        return "erro: o app nativo esta fechando".into();
    }
    match tokio::time::timeout(Duration::from_secs(REPLY_SECS), rx).await {
        Err(_) => format!("erro: o app nao respondeu em {REPLY_SECS}s"),
        Ok(Err(_)) => "erro: o navegador fechou no meio do comando".into(),
        Ok(Ok(Reply::Text(text))) => text,
        Ok(Ok(Reply::Png(png))) => {
            let path = path.unwrap_or_default();
            // Fora da thread da interface: disco lento não trava a janela.
            match tokio::task::spawn_blocking({ let path = path.clone(); move || std::fs::write(path, png) }).await {
                Ok(Ok(())) => format!("ok: shot {path}"),
                Ok(Err(e)) => format!("erro: shot {path}: {e}"),
                Err(e) => format!("erro: shot {path}: {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_is_parsed_and_checked_in_electron_order() {
        let head = parse_head("POST /cmd HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer abc\r\nContent-Length: 12").unwrap();
        assert_eq!((head.length, head.auth.as_deref()), (12, Some("Bearer abc")));
        assert_eq!(check(&head, "abc"), Ok(Route::Cmd));
        assert_eq!(check(&head, "zzz"), Err((401, "erro: token invalido".into())));
        let other = parse_head("GET /cmd HTTP/1.1").unwrap();
        assert_eq!(check(&other, "abc"), Err((404, "erro: rota desconhecida".into())));
        assert_eq!(parse_head("POST /cmd HTTP/1.1\r\nContent-Length: 999999").err(), Some((413, "erro: corpo grande demais".into())));
    }

    #[test]
    fn cdp_route_needs_token_key_and_websocket() {
        let upgrade = "\r\nAuthorization: Bearer abc\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==";
        let head = parse_head(&format!("GET /cdp?chave=srv%3A%3Anav-teste HTTP/1.1{upgrade}")).unwrap();
        assert_eq!(check(&head, "abc"), Ok(Route::Cdp("srv::nav-teste".into())));
        assert_eq!(check(&head, "zzz"), Err((401, "erro: token invalido".into())));
        let plain = parse_head("GET /cdp?chave=s%3A%3Aa HTTP/1.1\r\nAuthorization: Bearer abc").unwrap();
        assert_eq!(check(&plain, "abc"), Err((400, "erro: /cdp so aceita WebSocket".into())));
        let keyless = parse_head(&format!("GET /cdp HTTP/1.1{upgrade}")).unwrap();
        assert_eq!(check(&keyless, "abc"), Err((400, "erro: /cdp precisa de ?chave=".into())));
        let post = parse_head(&format!("POST /cdp?chave=a HTTP/1.1{upgrade}")).unwrap();
        assert_eq!(check(&post, "abc"), Err((404, "erro: rota desconhecida".into())));
    }

    #[test]
    fn body_with_accent_survives_split_reads() {
        let body = r#"{"chave":"s::a","verbo":"fill","args":["@e2","ação"]}"#.as_bytes();
        // O "ç" (2 bytes) cortado entre duas leituras: a decodificação é uma só, no fim.
        let cut = body.iter().position(|&b| b == 0xc3).unwrap() + 1;
        let mut joined = body[..cut].to_vec();
        joined.extend_from_slice(&body[cut..]);
        let parsed = parse_body(&joined).unwrap();
        assert_eq!((parsed.key.as_str(), parsed.args[1].as_str()), ("s::a", "ação"));
    }

    #[test]
    fn body_errors_use_the_shell_text() {
        assert_eq!(parse_body(b"{").err(), Some("erro: corpo invalido".into()));
        assert_eq!(parse_body(br#"{"verbo":"snapshot"}"#).err(), Some("erro: corpo invalido".into()));
        let with_tab = parse_body(br#"{"chave":"k","verbo":"url","aba":2}"#).unwrap();
        assert_eq!((with_tab.tab, with_tab.args.len()), (Some(2), 0));
    }

    #[test]
    fn only_native_sidecars_of_another_pid_are_stale() {
        assert!(is_stale_native_sidecar(&json!({"chave": "s::a", "pid": 10}), 20));
        assert!(!is_stale_native_sidecar(&json!({"chave": "s::a", "pid": 20}), 20));
        assert!(!is_stale_native_sidecar(&json!({"chave": "s::a", "url": "http://x/"}), 20));
    }

    #[test]
    fn same_is_constant_shape() {
        assert!(same(b"abc", b"abc"));
        assert!(!same(b"abc", b"abd"));
        assert!(!same(b"abc", b"ab"));
    }
}
