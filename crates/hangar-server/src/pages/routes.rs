//! Rotas das páginas: publicação pela ponte privada (MCP do Python), leitura pelo dono. Convidado segue ao Python.
use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use axum::body::{Body, to_bytes};
use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use serde_json::{Value, json};

use super::chrome::{self, Job};
use super::store::{NewPage, Store};
use super::{HEIGHT_MAX, HEIGHT_MIN, HTML_MAX_CHARS, TITLE_MAX_CHARS, Theme, WIDTHS, images, theme};
use crate::routes::{AppState, INFO_REASON, cors, fetch_info, gate, pass, route_failed};

const MESSAGE: &str = "Mostrada ao leitor acima da resposta. Não mencione nem descreva a página; responda só o que ela não diz.";
const BODY_LIMIT: usize = 4 * 1024 * 1024;
const BODY_TIMEOUT: Duration = Duration::from_secs(10);
const SHOT_WIDTH: u32 = 728;
const SWEEP_EVERY: Duration = Duration::from_secs(30);
const URL_HEIGHT: u32 = 640;
const HANGAR_PORTS: [u16; 3] = [8765, 8766, 8768];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishBody {
    session: String,
    #[serde(default)]
    html: Option<String>,
    #[serde(default)]
    url: Option<String>,
    title: String,
    // Com sinal: altura negativa recebe a mensagem do limite, não "corpo inválido".
    #[serde(default)]
    height: Option<i64>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    own_theme: bool,
}

fn json_reply(status: StatusCode, value: Value) -> Response {
    (status, [(header::CONTENT_TYPE, "application/json")], value.to_string()).into_response()
}

fn fail(code: &str, detail: &str) -> Value { json!({"ok": false, "error": {"code": code, "detail": detail}}) }

pub async fn publish_bridge(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    if !crate::terminal_routes::trusted_internal(&st, peer, req.headers()) { return StatusCode::NOT_FOUND.into_response(); }
    let reply = match tokio::time::timeout(BODY_TIMEOUT, to_bytes(req.into_body(), BODY_LIMIT)).await {
        Ok(Ok(bytes)) => match serde_json::from_slice::<PublishBody>(&bytes) {
            Ok(body) => publish(&st, body).await,
            Err(_) => fail("erro_pagina_invalida", "corpo inválido"),
        },
        _ => fail("erro_pagina_invalida", "corpo grande demais ou incompleto"),
    };
    json_reply(StatusCode::OK, reply)
}

struct Prepared { id: String, html: String, missing: Vec<String>, bin: Option<PathBuf> }

/// Endereço do modo URL, normalizado. O site abre no perfil do painel, que pode ter o login do Hangar.
fn site_url(raw: &str) -> Result<String, Value> {
    let bad = || fail("erro_pagina_invalida", "url precisa ser um endereço http ou https");
    let url = reqwest::Url::parse(raw.trim()).map_err(|_| bad())?;
    if !matches!(url.scheme(), "http" | "https") { return Err(bad()); }
    let host = url.host_str().ok_or_else(bad)?;
    if url.port_or_known_default().is_some_and(|p| HANGAR_PORTS.contains(&p)) && own_host(host) {
        return Err(fail("erro_pagina_endereco_recusado", "endereço do próprio Hangar não abre na conversa"));
    }
    Ok(url.into())
}

/// ponytail: sem listar as interfaces da máquina, qualquer endereço de rede local ou VPN conta como
/// próprio; só vale nas portas do Hangar, onde um par também teria o login dele.
fn own_host(host: &str) -> bool {
    let h = host.trim_start_matches('[').trim_end_matches(']').trim_end_matches('.').to_ascii_lowercase();
    if let Ok(ip) = h.parse::<IpAddr>() {
        let ip = match ip { IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4), v4 => v4 };
        return ip.is_loopback() || ip.is_unspecified() || match ip {
            IpAddr::V4(v4) => v4.is_private() || v4.is_link_local() || (v4.octets()[0] == 100 && v4.octets()[1] & 0xc0 == 64),
            IpAddr::V6(v6) => v6.is_unique_local() || v6.is_unicast_link_local(),
        };
    }
    h == "localhost" || h.ends_with(".localhost") || h.ends_with(".ts.net")
        || machine_name().is_some_and(|n| h.split('.').next() == Some(n.as_str()))
}

#[cfg(unix)]
fn machine_name() -> Option<String> {
    let mut b = [0u8; 256];
    // SAFETY: buffer próprio, tamanho certo; o nome sai terminado em zero ou cortado no tamanho.
    if unsafe { libc::gethostname(b.as_mut_ptr().cast(), b.len()) } != 0 { return None; }
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    let name = String::from_utf8_lossy(&b[..n]).split('.').next()?.to_ascii_lowercase();
    (!name.is_empty()).then_some(name)
}

#[cfg(not(unix))]
fn machine_name() -> Option<String> {
    std::env::var("COMPUTERNAME").ok().map(|n| n.to_ascii_lowercase()).filter(|n| !n.is_empty())
}

async fn publish(st: &AppState, b: PublishBody) -> Value {
    let site = match (&b.html, &b.url) {
        (Some(html), None) => {
            let chars = html.chars().count();
            if chars == 0 || chars > HTML_MAX_CHARS { return fail("erro_pagina_invalida", "html precisa ter de 1 a 512000 caracteres"); }
            None
        }
        (None, Some(url)) => match site_url(url) { Ok(u) => Some(u), Err(v) => return v },
        _ => return fail("erro_pagina_invalida", "passe html ou url, nunca os dois"),
    };
    let title = b.title.trim().to_owned();
    let title_chars = title.chars().count();
    if title_chars == 0 || title_chars > TITLE_MAX_CHARS { return fail("erro_pagina_invalida", "title precisa ter de 1 a 200 caracteres"); }
    if b.height.is_some_and(|h| !(i64::from(HEIGHT_MIN)..=i64::from(HEIGHT_MAX)).contains(&h)) { return fail("erro_pagina_invalida", "height fica entre 80 e 2000"); }
    let height = b.height.map(|h| h as u32);
    // Rascunho de site só confere o endereço: sem o login do Jefferson o servidor nem abriria.
    if b.draft && let Some(url) = &site { return json!({"ok": true, "result": {"draft": {"url": url}}}); }
    let info = match fetch_info(&st.http, st.cfg.upstream, &st.cfg.internal_secret, &b.session).await {
        Ok(Some(i)) => i,
        Ok(None) => return fail("erro_sessao_desconhecida", "sessão não encontrada"),
        Err(_) => return fail("erro_pagina_sem_info", INFO_REASON),
    };
    // Dono vazio nunca casaria com a lista: a limpeza apagaria a página no minuto seguinte.
    let Some(jsonl) = info.jsonl.as_deref().map(|p| p.to_string_lossy().into_owned()) else {
        return fail("erro_pagina_sem_transcript", "a sessão ainda não tem transcript");
    };
    let key = info.session_key;
    if let Some(url) = site { return publish_site(st, key, jsonl, title, height.unwrap_or(URL_HEIGHT), url).await; }
    let (pages, chromium, draft, own_theme, html) = (st.pages.clone(), st.chromium, b.draft, b.own_theme, b.html.unwrap_or_default());
    let (key2, title2) = (key.clone(), title.clone());
    // Imagens, tema e gravação leem e escrevem disco: fora da thread do runtime.
    let prepared = tokio::task::spawn_blocking(move || -> Result<Prepared, Value> {
        let too_large = |e: images::InlineError| fail(e.code(), e.path().unwrap_or("a página com as imagens passa de 25 MiB"));
        let (html, missing) = if draft {
            images::scan(&html).map(|s| (s.html, s.missing)).map_err(too_large)?
        } else {
            (images::inline(&html).map_err(too_large)?, Vec::new())
        };
        let page = NewPage { html: theme::inject(&html, !own_theme), title: title2, height, heights: BTreeMap::new(), draft, own_theme, url: None };
        let id = pages.save(&key2, &jsonl, &page).map_err(|e| {
            tracing::warn!(key = %key2, "página não gravada: {e}");
            fail("erro_pagina_nao_gravada", "não foi possível gravar a página")
        })?;
        Ok(Prepared { id, html: page.html, missing, bin: chromium() })
    }).await;
    let Prepared { id, html, missing, bin } = match prepared {
        Ok(Ok(p)) => p,
        Ok(Err(v)) => return v,
        Err(e) => {
            tracing::error!(panic = e.is_panic(), "gravação da página interrompida");
            return fail("erro_pagina_nao_gravada", "a gravação caiu no servidor");
        }
    };
    let shot = draft.then(|| st.pages.shot_path(&key, &id, Theme::Dark, SHOT_WIDTH));
    let jobs: Vec<Job> = WIDTHS.iter()
        .map(|w| Job { width: *w, theme: Theme::Dark, shot: if *w == SHOT_WIDTH { shot.clone() } else { None } })
        .collect();
    let rendered = chrome::render_with(bin, &html, &jobs, own_theme).await;
    let (heights, console) = match &rendered {
        Ok(r) => (r.heights.clone(), r.console.clone()),
        Err(_) => (BTreeMap::new(), Vec::new()),
    };
    if !heights.is_empty() {
        let (pages, key, id, heights) = (st.pages.clone(), key.clone(), id.clone(), heights.clone());
        match tokio::task::spawn_blocking(move || pages.set_heights(&key, &id, heights)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!("alturas da página não gravadas: {e}"),
            Err(e) => tracing::error!(panic = e.is_panic(), "gravação das alturas interrompida"),
        }
    }
    if draft {
        // Caminho relativo e sem token: quem abre (o `browser_open`) completa endereço e credencial.
        let url = format!("/api/sessions/{}/pages/{id}", utf8_percent_encode(&b.session, NON_ALPHANUMERIC));
        let browser = rendered.as_ref().map_or_else(|e| e.status(), |_| "ok");
        return json!({"ok": true, "result": {"draft": {"id": id, "url": url,
            "shot": rendered.is_ok().then(|| shot.map(|p| p.display().to_string())).flatten(),
            "heights": heights, "console": console, "missing_images": missing, "browser": browser,
            "browser_reason": rendered.as_ref().err().map(|e| e.reason())}}});
    }
    json!({"ok": true, "result": {"hangar_page": {"id": id, "title": title, "height": height, "heights": heights, "own_theme": own_theme}, "message": MESSAGE}})
}

/// Modo URL: só o endereço vai para o disco; sem medição nem print.
async fn publish_site(st: &AppState, key: String, jsonl: String, title: String, height: u32, url: String) -> Value {
    let page = NewPage { html: String::new(), title: title.clone(), height: Some(height), heights: BTreeMap::new(), draft: false, own_theme: false, url: Some(url.clone()) };
    let pages = st.pages.clone();
    match tokio::task::spawn_blocking(move || pages.save(&key, &jsonl, &page).map_err(|e| (key, e))).await {
        Ok(Ok(id)) => json!({"ok": true, "result": {"hangar_page": {"id": id, "title": title, "height": height, "heights": {}, "own_theme": false, "url": url}, "message": MESSAGE}}),
        Ok(Err((key, e))) => {
            tracing::warn!(key = %key, "página não gravada: {e}");
            fail("erro_pagina_nao_gravada", "não foi possível gravar a página")
        }
        Err(e) => {
            tracing::error!(panic = e.is_panic(), "gravação da página interrompida");
            fail("erro_pagina_nao_gravada", "a gravação caiu no servidor")
        }
    }
}

/// Leitura do disco que caiu no servidor: não é página expirada.
fn read_failed(st: &AppState, headers: &HeaderMap, name: &str, e: tokio::task::JoinError) -> Response {
    tracing::error!(panic = e.is_panic(), "leitura da página interrompida");
    route_failed(st, headers, "rust.pages_failed", name, "erro_pagina_falhou", "a leitura da página caiu no servidor")
}

fn not_found(req: &HeaderMap, code: &str, msg: &str) -> Response {
    let mut r = json_reply(StatusCode::NOT_FOUND, json!({"detail": {"code": code, "msg": msg}}));
    cors(req, r.headers_mut());
    r
}

fn expired(req: &HeaderMap) -> Response { not_found(req, "erro_pagina_expirou", "página apagada com a sessão") }

/// Página do modo URL não tem documento: o app abre o endereço dela.
fn no_html(req: &HeaderMap) -> Response { not_found(req, "erro_pagina_sem_html", "página de site não tem html; abra a url dela") }

/// Chave da pasta da sessão; a resposta pronta quando não há (página expirada ou Python sem responder).
async fn session_key(st: &AppState, headers: &HeaderMap, name: &str) -> Result<String, Response> {
    match fetch_info(&st.http, st.cfg.upstream, &st.cfg.internal_secret, name).await {
        Ok(Some(info)) => Ok(info.session_key),
        Ok(None) => Err(expired(headers)),
        Err(_) => Err(route_failed(st, headers, "rust.pages_failed", name, "erro_pagina_sem_info", INFO_REASON)),
    }
}

pub async fn page(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>,
    path: Result<Path<(String, String)>, axum::extract::rejection::PathRejection>, req: Request) -> Response {
    let (fwd, owner) = gate(&st, peer, &req);
    let (name, id) = match path { Ok(Path(p)) if owner => p, _ => return pass(&st, req, &fwd).await };
    let (headers, raw) = (req.headers().clone(), crate::auth::query_param(req.uri().query(), "raw").is_some());
    drop(req);
    let key = match session_key(&st, &headers, &name).await { Ok(k) => k, Err(r) => return r };
    let pages = st.pages.clone();
    let found = tokio::task::spawn_blocking(move || {
        let meta = pages.meta(&key, &id)?;
        if meta.url.is_some() { return Some(None); }
        Some(Some((pages.html(&key, &id)?, meta)))
    }).await;
    let found = match found { Ok(f) => f, Err(e) => return read_failed(&st, &headers, &name, e) };
    let Some(found) = found else { return expired(&headers) };
    let Some((html, meta)) = found else { return no_html(&headers) };
    let mut r = if raw {
        let mut r = Response::new(Body::from(html));
        let h = r.headers_mut();
        h.insert(header::CONTENT_TYPE, "text/plain; charset=utf-8".parse().unwrap());
        h.insert("content-security-policy", "sandbox".parse().unwrap());
        h.insert("x-content-type-options", "nosniff".parse().unwrap());
        h.insert(header::CACHE_CONTROL, "private, max-age=3600".parse().unwrap());
        r
    } else {
        isolated_shell(&html, &meta.title)
    };
    cors(&headers, r.headers_mut());
    r
}

/// O iframe é opaco e sem `allow-top-navigation`: o link da página só abre se a casca abrir por ele.
const OPEN_LINK: &str = "var f=document.querySelector(\"iframe\");addEventListener(\"message\",function(e){var d=e.data;if(e.source===f.contentWindow&&d&&d.method===\"ui/open-link\"&&d.params&&typeof d.params.url===\"string\"&&/^https?:/i.test(d.params.url))window.open(d.params.url,\"_blank\",\"noopener\")});";
const STRIP_TOKEN: &str = "var q=new URLSearchParams(location.search);if(q.has(\"token\")){q.delete(\"token\");q=q.toString();history.replaceState(null,\"\",location.pathname+(q?\"?\"+q:\"\")+location.hash)}";

/// Casca da página: o HTML vai num JSON embutido e vira URL `blob:` no iframe isolado. Nem `data:`
/// (o Chromium corta URL grande) nem `srcdoc` (o documento herdaria o endereço da casca, com o token).
pub(crate) fn isolated_shell(html: &str, title: &str) -> Response {
    let title = crate::workspace_routes::html_escape(title);
    // Sem `<` cru o conteúdo nunca fecha o `<script>` nem abre comentário dentro dele.
    let data = serde_json::to_string(html).unwrap_or_default()
        .replace('<', "\\u003c").replace('\u{2028}', "\\u2028").replace('\u{2029}', "\\u2029");
    // O token sai do endereço antes de tudo: `browser url`/abas/snapshot levariam ele ao transcript,
    // que convidado e par leem. Recarregar a casca depois dá 401; rascunho é de vida curta.
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{title}</title><style>html,body{{margin:0;height:100%;overflow:hidden}}iframe{{display:block;width:100%;height:100%;border:0}}</style></head><body><iframe title=\"{title}\" sandbox=\"allow-scripts allow-popups\" referrerpolicy=\"no-referrer\"></iframe><script type=\"application/json\" id=\"p\">{data}</script><script>{STRIP_TOKEN}{OPEN_LINK}f.src=URL.createObjectURL(new Blob([JSON.parse(document.getElementById(\"p\").textContent)],{{type:\"text/html;charset=utf-8\"}}))</script></body></html>"
    );
    let mut r = Response::new(Body::from(body));
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, "text/html; charset=utf-8".parse().unwrap());
    h.insert("x-content-type-options", "nosniff".parse().unwrap());
    h.insert("referrer-policy", "no-referrer".parse().unwrap());
    h.insert(header::CACHE_CONTROL, "private, max-age=3600".parse().unwrap());
    r
}

pub async fn shot(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>,
    path: Result<Path<(String, String)>, axum::extract::rejection::PathRejection>, req: Request) -> Response {
    let (fwd, owner) = gate(&st, peer, &req);
    let (name, id) = match path { Ok(Path(p)) if owner => p, _ => return pass(&st, req, &fwd).await };
    let (headers, query) = (req.headers().clone(), req.uri().query().map(str::to_owned));
    drop(req);
    let query = query.as_deref();
    let theme = crate::auth::query_param(query, "theme").as_deref().and_then(Theme::parse).unwrap_or(Theme::Dark);
    let want = crate::auth::query_param(query, "width").and_then(|w| w.parse::<u32>().ok()).unwrap_or(SHOT_WIDTH);
    let width = *WIDTHS.iter().min_by_key(|w| w.abs_diff(want)).unwrap();
    let key = match session_key(&st, &headers, &name).await { Ok(k) => k, Err(r) => return r };
    let (pages, chromium) = (st.pages.clone(), st.chromium);
    let found = tokio::task::spawn_blocking(move || {
        let meta = pages.meta(&key, &id)?;
        if meta.url.is_some() { return Some(None); }
        let html = pages.html(&key, &id)?;
        let path = pages.shot_path(&key, &id, theme, width);
        let ready = path.is_file();
        Some(Some((html, meta.own_theme, path, ready, if ready { None } else { chromium() })))
    }).await;
    let found = match found { Ok(f) => f, Err(e) => return read_failed(&st, &headers, &name, e) };
    let Some(found) = found else { return expired(&headers) };
    let Some((html, own_theme, path, ready, bin)) = found else { return no_html(&headers) };
    if !ready {
        if let Err(e) = chrome::render_with(bin, &html, &[Job { width, theme, shot: Some(path.clone()) }], own_theme).await {
            let mut r = json_reply(StatusCode::NOT_FOUND, json!({"detail": {"code": "erro_pagina_sem_imagem", "msg": e.reason()}}));
            cors(&headers, r.headers_mut());
            return r;
        }
    }
    let Ok(bytes) = tokio::fs::read(&path).await else { return expired(&headers) };
    let mut r = Response::new(Body::from(bytes));
    r.headers_mut().insert(header::CONTENT_TYPE, "image/png".parse().unwrap());
    r.headers_mut().insert(header::CACHE_CONTROL, "private, max-age=3600".parse().unwrap());
    cors(&headers, r.headers_mut());
    r
}

/// Na subida e a cada 30 s: apaga as páginas das sessões que saíram da lista. Sem rodada recente
/// e certa da lista do dono, pula: um conjunto vazio apagaria tudo.
pub fn spawn_sweep(list: Arc<crate::list::bridge::ListBridge>, pages: Arc<Store>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_EVERY);
        loop {
            tick.tick().await;
            let Some(live) = list.live_jsonl() else { pages.forget_absences(); continue };
            let pages = pages.clone();
            if let Err(e) = tokio::task::spawn_blocking(move || pages.sweep(&live, SystemTime::now())).await {
                tracing::error!(panic = e.is_panic(), "limpeza das páginas interrompida");
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn body(r: Response) -> String {
        String::from_utf8(to_bytes(r.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap()
    }

    #[tokio::test]
    async fn shell_never_lets_the_page_close_its_script() {
        let r = isolated_shell("<p>a</p></script><script>alert(1)</script><!-- \u{2028}", "T \"<b>\"");
        assert_eq!(r.headers()["referrer-policy"], "no-referrer");
        let text = body(r).await;
        let data = &text[text.find("id=\"p\">").unwrap()..];
        let data = &data[..data.find("</script>").unwrap()];
        assert!(!data[7..].contains('<'), "{data}");
        assert!(!data.contains('\u{2028}'));
        assert_eq!(text.matches("</script>").count(), 2, "só os dois scripts da casca");
        assert!(text.contains("<title>T &quot;&lt;b&gt;&quot;</title>"));
        assert!(text.contains("sandbox=\"allow-scripts allow-popups\""));
        assert!(!text.contains("allow-same-origin") && !text.contains("srcdoc") && !text.contains("data:text/html"));
        let json: String = serde_json::from_str(&data[7..]).unwrap();
        assert!(json.starts_with("<p>a</p></script>"));
        let strip = text.find("history.replaceState(").expect("casca tira o token do endereço");
        assert!(strip < text.find("createObjectURL").unwrap(), "antes de criar o iframe");
        assert!(text[text.rfind("<script>").unwrap()..].starts_with(&format!("<script>{STRIP_TOKEN}{OPEN_LINK}")), "primeira coisa do script");
        // Link morto não: a casca abre só o que veio do próprio iframe, e só http(s).
        assert!(OPEN_LINK.contains("e.source===f.contentWindow") && OPEN_LINK.contains("/^https?:/i.test(d.params.url)"));
        assert!(OPEN_LINK.contains("window.open(d.params.url,\"_blank\",\"noopener\")"));
    }

    #[test]
    fn unknown_body_field_is_refused() {
        assert!(serde_json::from_str::<PublishBody>(r#"{"session":"s","html":"x","title":"t","base":"http://x"}"#).is_err());
        let b: PublishBody = serde_json::from_str(r#"{"session":"s","html":"x","title":"t"}"#).unwrap();
        assert!(!b.draft && b.height.is_none() && !b.own_theme);
        let b: PublishBody = serde_json::from_str(r#"{"session":"s","html":"x","title":"t","own_theme":true}"#).unwrap();
        assert!(b.own_theme);
        let b: PublishBody = serde_json::from_str(r#"{"session":"s","html":"x","title":"t","height":-5}"#).unwrap();
        assert_eq!(b.height, Some(-5), "negativa passa pelo corpo e cai na mensagem do limite");
        let b: PublishBody = serde_json::from_str(r#"{"session":"s","url":"http://localhost:3000/cidades","title":"t"}"#).unwrap();
        assert!(b.html.is_none() && b.url.is_some());
    }

    #[test]
    fn site_url_is_normalized_and_hangar_is_refused() {
        assert_eq!(site_url(" http://LocalHost:3000/cidades ").unwrap(), "http://localhost:3000/cidades");
        assert_eq!(site_url("https://example.com").unwrap(), "https://example.com/");
        let code = |raw: &str| site_url(raw).unwrap_err()["error"]["code"].as_str().unwrap().to_owned();
        for bad in ["file:///etc/passwd", "javascript:alert(1)", "ftp://example.com", "localhost:3000", ""] {
            assert_eq!(code(bad), "erro_pagina_invalida", "{bad}");
        }
        for own in ["http://127.0.0.1:8765/", "http://localhost:8766/x", "http://[::1]:8768/", "http://0.0.0.0:8765",
            "http://192.168.0.10:8765", "http://100.64.0.2:8766", "https://maquina.tail1234.ts.net:8765/"] {
            assert_eq!(code(own), "erro_pagina_endereco_recusado", "{own}");
        }
        assert!(site_url("http://localhost:3000/").is_ok(), "app local em outra porta abre");
        assert!(site_url("http://example.com:8765/").is_ok(), "site de fora na mesma porta não é o Hangar");
    }
}
