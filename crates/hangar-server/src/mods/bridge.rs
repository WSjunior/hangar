//! Rotas da ponte do plugin do Hangar para as sessões que o Rust atende: `press-start` (este press é o
//! clique que o app pediu?) e `opened` (a URL que o mod abriria). Na sessão sem terminal o plugin entra
//! no `claude -p` pelo `--plugin-dir` que o Python põe no lançamento (S7) e usa só estas duas, no clique
//! do app pela superfície `desktop`; a sessão com terminal do Rust as usa a partir da fase 3. O resto da
//! ponte, e estas rotas para as outras sessões, seguem no Python.
use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use subtle::ConstantTimeEq;

use super::http::{fits, invalid, reply};
use crate::routes::{AppState, gate, pass};

const BODY_LIMIT: usize = 16 * 1024;
const URL_MAX: usize = 8192;
/// Prazo da pergunta ao Python sobre o nome antigo: o plugin desiste do `press-start` em 3 s, e a resposta,
/// com o repasse ao Python incluído, tem de chegar antes; senão o clique roda depois de ele desistir.
const NAME_CHECK: std::time::Duration = std::time::Duration::from_secs(1);

/// O que toda chamada da ponte traz: a sessão (o nome com que o processo nasceu) e o token dela.
#[derive(Deserialize)]
struct Envelope<T> { sessao: String, token: String, #[serde(flatten)] body: T }

#[derive(Deserialize)]
struct PressStart { #[serde(rename = "requestId")] request_id: String, element: String }

#[derive(Deserialize)]
struct Opened { attempt: String, url: String }

/// O token da ponte de uma sessão, igual ao `plugin_bridge.mint` do Python: HMAC-SHA256 do token do
/// dono (ou "hangar", sem token) sobre `plugin:<nome>`, em 32 dígitos hexadecimais.
pub fn mint(secret: &str, name: &str) -> String {
    let secret = if secret.is_empty() { "hangar" } else { secret };
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
    ring::hmac::sign(&key, format!("plugin:{name}").as_bytes()).as_ref()[..16].iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Corpo lido uma vez: sessão que não é superfície do Rust volta ao Python com o mesmo corpo. Devolve o
/// corpo e o nome atual da sessão, que pode não ser o `sessao` do plugin (sessão renomeada sem relançar
/// o processo: o plugin manda o nome com que nasceu, e o token é o desse nome).
async fn owned<T: DeserializeOwned>(st: &Arc<AppState>, peer: SocketAddr, req: Request) -> Result<(Envelope<T>, String), Box<Response>> {
    let (fwd, _) = gate(st, peer, &req);
    let (parts, raw) = req.into_parts();
    let Ok(bytes) = to_bytes(raw, BODY_LIMIT).await else { return Err(Box::new(StatusCode::PAYLOAD_TOO_LARGE.into_response())) };
    let parsed = serde_json::from_slice::<Envelope<T>>(&bytes).ok()
        .and_then(|envelope| st.mods.bridge_session(&envelope.sessao).map(|name| (envelope, name)));
    match parsed {
        Some((envelope, name)) if name == envelope.sessao || !named_elsewhere(st, &envelope.sessao).await => Ok((envelope, name)),
        _ => Err(Box::new(pass(st, Request::from_parts(parts, Body::from(bytes)), &fwd).await)),
    }
}

/// O nome antigo de uma sessão renomeada pode ser o nome atual de outra, fora do Rust (com terminal, ou
/// criada depois no Python), cujo plugin manda o mesmo `sessao` com um token que vale. Pergunta ao Python
/// pelo status do `info`, sem o cache do `/events` (a resposta de um segundo atrás pode ser de antes do
/// renomear) e sem ler o corpo: 404 é sessão inexistente. Existindo a sessão, ou sem resposta em
/// `NAME_CHECK`, o pedido é dela e vai ao Python.
async fn named_elsewhere(st: &AppState, sessao: &str) -> bool {
    let url = format!("http://{}/internal/sessions/{}/info", st.cfg.upstream, utf8_percent_encode(sessao, NON_ALPHANUMERIC));
    let Ok(request) = axum::http::Request::get(url).header("x-hangar-internal", &st.cfg.internal_secret).body(Body::empty()) else { return true };
    !matches!(tokio::time::timeout(NAME_CHECK, st.http.request(request)).await,
        Ok(Ok(response)) if response.status() == StatusCode::NOT_FOUND)
}

/// Comparação em tempo constante, como o `secrets.compare_digest` do Python.
fn token_ok(st: &AppState, name: &str, token: &str) -> bool {
    mint(&st.cfg.auth_token, name).as_bytes().ct_eq(token.as_bytes()).into()
}

pub async fn press_start(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let (envelope, name) = match owned::<PressStart>(&st, peer, req).await { Ok(found) => found, Err(response) => return *response };
    let body = &envelope.body;
    // Os limites do Pydantic do Python (`PressBody`) vêm antes do token, como lá.
    if !fits(&body.request_id, 64) || !fits(&body.element, 256) {
        return invalid(None);
    }
    if !token_ok(&st, &envelope.sessao, &envelope.token) {
        return reply(None, StatusCode::FORBIDDEN, json!({"detail": "token do plugin inválido"}));
    }
    let attempt = st.mods.match_click(&name, &body.request_id, &body.element);
    reply(None, StatusCode::OK, json!({"fromApp": attempt.is_some(), "attempt": attempt}))
}

pub async fn opened(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let (envelope, name) = match owned::<Opened>(&st, peer, req).await { Ok(found) => found, Err(response) => return *response };
    let body = &envelope.body;
    // Os limites do `OpenedBody`. A URL não tem mínimo no Pydantic: vazia passa daqui e cai no 400 do esquema.
    if !fits(&body.attempt, 64) || body.url.chars().count() > URL_MAX {
        return invalid(None);
    }
    if !token_ok(&st, &envelope.sessao, &envelope.token) {
        return reply(None, StatusCode::FORBIDDEN, json!({"detail": "token do plugin inválido"}));
    }
    let lower = body.url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return reply(None, StatusCode::BAD_REQUEST, json!({"detail": "só http(s)"}));
    }
    if st.mods.opened(&name, &body.attempt, &body.url) {
        reply(None, StatusCode::OK, json!({"ok": true}))
    } else {
        // 409: o plugin segue com o `next` e o mod abre a URL na máquina do servidor, como no Python.
        reply(None, StatusCode::CONFLICT, json!({"detail": "clique do app já respondido"}))
    }
}
