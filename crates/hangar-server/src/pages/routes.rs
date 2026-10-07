//! Rotas das páginas: publicação pela ponte privada (MCP do Python), leitura pelo dono. Convidado segue ao Python.
use std::collections::BTreeMap;
use std::net::SocketAddr;
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishBody {
    session: String,
    html: String,
    title: String,
    // Com sinal: altura negativa recebe a mensagem do limite, não "corpo inválido".
    #[serde(default)]
    height: Option<i64>,
    #[serde(default)]
    draft: bool,
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

async fn publish(st: &AppState, b: PublishBody) -> Value {
    let chars = b.html.chars().count();
    if chars == 0 || chars > HTML_MAX_CHARS { return fail("erro_pagina_invalida", "html precisa ter de 1 a 512000 caracteres"); }
    let title = b.title.trim().to_owned();
    let title_chars = title.chars().count();
    if title_chars == 0 || title_chars > TITLE_MAX_CHARS { return fail("erro_pagina_invalida", "title precisa ter de 1 a 200 caracteres"); }
    if b.height.is_some_and(|h| !(i64::from(HEIGHT_MIN)..=i64::from(HEIGHT_MAX)).contains(&h)) { return fail("erro_pagina_invalida", "height fica entre 80 e 2000"); }
    let height = b.height.map(|h| h as u32);
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
    let (pages, chromium, draft, html) = (st.pages.clone(), st.chromium, b.draft, b.html);
    let (key2, title2) = (key.clone(), title.clone());
    // Imagens, tema e gravação leem e escrevem disco: fora da thread do runtime.
    let prepared = tokio::task::spawn_blocking(move || -> Result<Prepared, Value> {
        let too_large = |e: images::InlineError| fail(e.code(), e.path().unwrap_or("a página com as imagens passa de 25 MiB"));
        let (html, missing) = if draft {
            images::scan(&html).map(|s| (s.html, s.missing)).map_err(too_large)?
        } else {
            (images::inline(&html).map_err(too_large)?, Vec::new())
        };
        let page = NewPage { html: theme::inject(&html), title: title2, height, heights: BTreeMap::new(), draft };
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
    let rendered = chrome::render_with(bin, &html, &jobs).await;
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
    json!({"ok": true, "result": {"hangar_page": {"id": id, "title": title, "height": height, "heights": heights}, "message": MESSAGE}})
}

/// Leitura do disco que caiu no servidor: não é página expirada.
fn read_failed(st: &AppState, headers: &HeaderMap, name: &str, e: tokio::task::JoinError) -> Response {
    tracing::error!(panic = e.is_panic(), "leitura da página interrompida");
    route_failed(st, headers, "rust.pages_failed", name, "erro_pagina_falhou", "a leitura da página caiu no servidor")
}

fn expired(req: &HeaderMap) -> Response {
    let mut r = json_reply(StatusCode::NOT_FOUND, json!({"detail": {"code": "erro_pagina_expirou", "msg": "página apagada com a sessão"}}));
    cors(req, r.headers_mut());
    r
}

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
    let found = match tokio::task::spawn_blocking(move || Some((pages.html(&key, &id)?, pages.meta(&key, &id)?))).await {
        Ok(f) => f,
        Err(e) => return read_failed(&st, &headers, &name, e),
    };
    let Some((html, meta)) = found else { return expired(&headers) };
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
        let html = pages.html(&key, &id)?;
        let path = pages.shot_path(&key, &id, theme, width);
        let ready = path.is_file();
        Some((html, path, ready, if ready { None } else { chromium() }))
    }).await;
    let found = match found { Ok(f) => f, Err(e) => return read_failed(&st, &headers, &name, e) };
    let Some((html, path, ready, bin)) = found else { return expired(&headers) };
    if !ready {
        if let Err(e) = chrome::render_with(bin, &html, &[Job { width, theme, shot: Some(path.clone()) }]).await {
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
        assert!(!b.draft && b.height.is_none());
        let b: PublishBody = serde_json::from_str(r#"{"session":"s","html":"x","title":"t","height":-5}"#).unwrap();
        assert_eq!(b.height, Some(-5), "negativa passa pelo corpo e cai na mensagem do limite");
    }
}
