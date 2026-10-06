//! Rotas da ponte do plugin do Hangar para as sessões que o Rust atende: `press-start` (este press é o
//! clique que o app pediu?) e `opened` (a URL que o mod abriria). Na sessão sem terminal o plugin entra
//! no `claude -p` pelo `--plugin-dir` que o Python põe no lançamento (S7) e usa só estas duas, no clique
//! do app pela superfície `desktop`; a sessão com terminal do Rust as usa a partir da fase 3. O resto da
//! ponte, e estas rotas para as outras sessões, seguem no Python.
use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use subtle::ConstantTimeEq;

use crate::routes::{AppState, gate, pass};

const BODY_LIMIT: usize = 16 * 1024;
const URL_MAX: usize = 8192;

trait Named { fn session(&self) -> &str; }

#[derive(Deserialize)]
struct PressStart { sessao: String, token: String, #[serde(rename = "requestId")] request_id: String, element: String }

#[derive(Deserialize)]
struct Opened { sessao: String, token: String, attempt: String, url: String }

impl Named for PressStart { fn session(&self) -> &str { &self.sessao } }
impl Named for Opened { fn session(&self) -> &str { &self.sessao } }

/// O token da ponte de uma sessão, igual ao `plugin_bridge.mint` do Python: HMAC-SHA256 do token do
/// dono (ou "hangar", sem token) sobre `plugin:<nome>`, em 32 dígitos hexadecimais.
pub fn mint(secret: &str, name: &str) -> String {
    let secret = if secret.is_empty() { "hangar" } else { secret };
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
    ring::hmac::sign(&key, format!("plugin:{name}").as_bytes()).as_ref()[..16].iter().map(|byte| format!("{byte:02x}")).collect()
}

fn answer(status: StatusCode, body: Value) -> Response {
    (status, [(header::CONTENT_TYPE, "application/json")], body.to_string()).into_response()
}

/// Corpo lido uma vez: sessão que não é superfície do Rust volta ao Python com o mesmo corpo.
async fn owned<T: DeserializeOwned + Named>(st: &Arc<AppState>, peer: SocketAddr, req: Request) -> Result<T, Response> {
    let (fwd, _) = gate(st, peer, &req);
    let (parts, raw) = req.into_parts();
    let Ok(bytes) = to_bytes(raw, BODY_LIMIT).await else { return Err(StatusCode::PAYLOAD_TOO_LARGE.into_response()) };
    match serde_json::from_slice::<T>(&bytes) {
        Ok(body) if st.mods.owns(body.session()) => Ok(body),
        _ => Err(pass(st, Request::from_parts(parts, Body::from(bytes)), &fwd).await),
    }
}

/// Comparação em tempo constante, como o `secrets.compare_digest` do Python.
fn token_ok(st: &AppState, name: &str, token: &str) -> bool {
    mint(&st.cfg.auth_token, name).as_bytes().ct_eq(token.as_bytes()).into()
}

pub async fn press_start(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let body: PressStart = match owned(&st, peer, req).await { Ok(body) => body, Err(response) => return response };
    if !token_ok(&st, &body.sessao, &body.token) {
        return answer(StatusCode::FORBIDDEN, json!({"detail": "token do plugin inválido"}));
    }
    let attempt = st.mods.match_click(&body.sessao, &body.request_id, &body.element);
    answer(StatusCode::OK, json!({"fromApp": attempt.is_some(), "attempt": attempt}))
}

pub async fn opened(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let body: Opened = match owned(&st, peer, req).await { Ok(body) => body, Err(response) => return response };
    if !token_ok(&st, &body.sessao, &body.token) {
        return answer(StatusCode::FORBIDDEN, json!({"detail": "token do plugin inválido"}));
    }
    let lower = body.url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) || body.url.len() > URL_MAX {
        return answer(StatusCode::BAD_REQUEST, json!({"detail": "só http(s)"}));
    }
    if st.mods.opened(&body.sessao, &body.attempt, &body.url) {
        answer(StatusCode::OK, json!({"ok": true}))
    } else {
        // 409: o plugin segue com o `next` e o mod abre a URL na máquina do servidor, como no Python.
        answer(StatusCode::CONFLICT, json!({"detail": "clique do app já respondido"}))
    }
}
