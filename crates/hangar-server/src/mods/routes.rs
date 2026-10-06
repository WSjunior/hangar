//! Rotas dos apps para a interface dos mods de uma sessão sem terminal que o Rust atende como
//! superfície remota: clique (`press`), troca de aba (`show`) e digitação (`input`). Sessão que o Rust
//! não atende assim, ou pedido de convidado, segue para o Python, que é o dono, como no `/events`; a
//! digitação, que o Python não tem, é recusada aqui.
//!
//! Antes de cada operação a rota pergunta ao Python se a troca de agente está em curso
//! (`GET /internal/sessions/{name}/transfer`): a coordenação da troca mora lá, e o 409 volta ao app como
//! o Python o deu. Limite aceito (ruling A12): a rota do Python segura o ingresso (`session_ingress`)
//! até o fim, e esta pergunta uma vez, já com a vez da sessão; uma troca que comece entre a resposta e o
//! `ui_*` não é vista. Cada `change` do `input` também paga essa ida ao Python.
//!
//! Prazo: o app desiste em 8 s, contados de quando mandou o pedido. A espera pela vez da sessão, a
//! guarda e a chamada ao mod dividem um orçamento só, medido desde a entrada na rota, para que a
//! resposta (recusa incluída) chegue antes de o app mostrar erro.
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::extract::rejection::PathRejection;
use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use http_body_util::BodyExt;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::time::Instant;

use super::model::*;
use crate::routes::{AppState, cors, gate, pass, route_failed};

const BODY_LIMIT: usize = 64 * 1024;
/// Orçamento do pedido inteiro, desde a entrada: abaixo dos 8 s em que o app desiste, com folga para a
/// volta da resposta.
const REQUEST_BUDGET: Duration = Duration::from_millis(7500);
/// Prazo da guarda da troca de agente. Curto: é uma consulta local ao Python, e o silêncio dele não pode
/// comer o tempo do mod.
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(2);
/// O `onPress` do mod costuma abrir ou copiar sem `await`: o efeito pode chegar logo depois do press
/// (mesma espera do `plugin_click.EFFECT_S`).
const EFFECT_WAIT: Duration = Duration::from_millis(300);
const VALUE_MAX: usize = 16384;
const TRANSFER_REASON: &str = "o backend não confirmou que a sessão está livre da troca de agente";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PressBody { site: String, key: String }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShowBody { site: String }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputBody { site: String, key: String, kind: String, value: String }

fn fits(text: &str, max: usize) -> bool {
    !text.is_empty() && text.chars().count() <= max
}

fn reply(headers: &HeaderMap, status: StatusCode, body: Value) -> Response {
    let mut response = (status, [(header::CONTENT_TYPE, "application/json")], body.to_string()).into_response();
    cors(headers, response.headers_mut());
    response
}

fn refused(headers: &HeaderMap, error: &ModsError) -> Response {
    reply(headers, StatusCode::CONFLICT, json!({"detail": error.detail()}))
}

fn invalid(headers: &HeaderMap) -> Response {
    reply(headers, StatusCode::UNPROCESSABLE_ENTITY, json!({"detail": "corpo inválido"}))
}

/// O Rust atende quando o pedido é do dono e a sessão é superfície dele; senão, o Python. `outside`: a
/// recusa do dono numa sessão sem superfície no Rust, para a rota que o Python não tem (`input`).
async fn owned(st: &Arc<AppState>, peer: SocketAddr, path: Result<Path<String>, PathRejection>, req: Request,
    outside: Option<fn() -> ModsError>) -> Result<(String, HeaderMap, Body), Response> {
    let (fwd, owner) = gate(st, peer, &req);
    match (path, outside) {
        (Ok(Path(name)), _) if owner && st.mods.owns(&name) => {
            let headers = req.headers().clone();
            Ok((name, headers, req.into_body()))
        }
        (Ok(Path(_)), Some(refusal)) if owner => Err(refused(req.headers(), &refusal())),
        _ => Err(pass(st, req, &fwd).await),
    }
}

async fn body<T: DeserializeOwned>(headers: &HeaderMap, raw: Body) -> Result<T, Response> {
    let bytes = to_bytes(raw, BODY_LIMIT).await.map_err(|_| invalid(headers))?;
    serde_json::from_slice(&bytes).map_err(|_| invalid(headers))
}

/// A guarda da troca de agente das rotas do Python (`_transfer_check`), perguntada a ele, que coordena
/// a troca. `None`: livre. 409: a recusa dele, com o `detail` como veio. Sem resposta no prazo dela, ou
/// com outro status, recusa com código e motivo, sem repassar (dono único). Se o que a cortou foi o fim
/// do orçamento do pedido (a vez da sessão demorou), a recusa é a do mod sem resposta.
async fn transfer(st: &AppState, headers: &HeaderMap, name: &str, deadline: Instant) -> Option<Response> {
    let until = deadline.min(Instant::now() + TRANSFER_TIMEOUT);
    let failed = || {
        if Instant::now() >= deadline {
            return Some(refused(headers, &no_answer()));
        }
        Some(route_failed(st, headers, "rust.mods_failed", name, "erro_mod_guarda_indisponivel", TRANSFER_REASON))
    };
    let url = format!("http://{}/internal/sessions/{}/transfer", st.cfg.upstream, utf8_percent_encode(name, NON_ALPHANUMERIC));
    let Ok(request) = axum::http::Request::get(url).header("x-hangar-internal", &st.cfg.internal_secret).body(Body::empty()) else { return failed() };
    let Ok(Ok(response)) = tokio::time::timeout_at(until, st.http.request(request)).await else { return failed() };
    if response.status().is_success() {
        return None;
    }
    if response.status() != StatusCode::CONFLICT {
        return failed();
    }
    let Ok(Ok(collected)) = tokio::time::timeout_at(until, response.into_body().collect()).await else { return failed() };
    // Um 409 sem o `detail` do Python (corpo vazio, outro formato) não diz o que recusar: é falha da guarda.
    let Ok(answer) = serde_json::from_slice::<Value>(&collected.to_bytes()) else { return failed() };
    if !answer["detail"].is_object() {
        return failed();
    }
    Some(reply(headers, StatusCode::CONFLICT, json!({"detail": answer["detail"]})))
}

async fn run(st: &AppState, headers: &HeaderMap, name: &str, call: ModsCall, deadline: Instant) -> Response {
    let Some((link, lock)) = st.mods.link(name) else { return refused(headers, &missing()) };
    // Um pedido por vez por sessão, como a trava do `plugin_click.press`: dois aparelhos não se cruzam. A
    // vez que não sai no orçamento é recusa, sem chamar o mod: o app já teria desistido.
    let Ok(_turn) = tokio::time::timeout_at(deadline, lock.lock()).await else { return refused(headers, &no_answer()) };
    // A guarda vem depois da vez: perguntada antes, a resposta envelheceria enquanto o pedido espera a
    // trava. Vale para toda operação, inclusive cada `change` do `input` (A12).
    if let Some(busy) = transfer(st, headers, name, deadline).await {
        return busy;
    }
    if Instant::now() >= deadline {
        return refused(headers, &no_answer());
    }
    let attempt = match &call { ModsCall::Press { site, key } => Some(st.mods.begin_click(name, site, key)), _ => None };
    // O que sobra do orçamento limita a chamada e vai junto até a superfície, que não leva ação ao mod sem
    // tempo para a resposta voltar antes dele. Cortada, a resposta que vier depois cai num canal fechado, e
    // o ator não leva à superfície um pedido que ainda estava na caixa dele.
    let result = tokio::time::timeout_at(deadline, link.call(call.clone(), deadline.into_std())).await.unwrap_or_else(|_| Err(no_answer()));
    let (copied, opened) = match &attempt {
        Some(attempt) => st.mods.finish_click(name, attempt, EFFECT_WAIT.min(deadline.saturating_duration_since(Instant::now()))).await,
        None => (None, None),
    };
    match result {
        Ok(value) => {
            let mut answer = json!({"ok": true});
            match call {
                ModsCall::Show { .. } => answer["shown_id"] = value["shown_id"].clone(),
                ModsCall::Input { .. } => answer["value"] = value["value"].clone(),
                ModsCall::Press { .. } | ModsCall::Close { .. } => {}
            }
            if let Some(text) = copied { answer["copied"] = json!(text); }
            if let Some(url) = opened { answer["opened"] = json!(url); }
            reply(headers, StatusCode::OK, answer)
        }
        Err(error) => refused(headers, &error),
    }
}

/// Clique num botão de mod; `key: "__close__"` fecha o painel `site`.
pub async fn press(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>,
    path: Result<Path<String>, PathRejection>, req: Request) -> Response {
    let deadline = Instant::now() + REQUEST_BUDGET;
    let (name, headers, raw) = match owned(&st, peer, path, req, None).await { Ok(parts) => parts, Err(response) => return response };
    let request: PressBody = match body(&headers, raw).await { Ok(request) => request, Err(response) => return response };
    if !fits(&request.site, 64) || !fits(&request.key, 256) {
        return invalid(&headers);
    }
    let call = if request.key == CLOSE_KEY { ModsCall::Close { site: request.site } }
        else { ModsCall::Press { site: request.site, key: request.key } };
    run(&st, &headers, &name, call, deadline).await
}

/// Troca de aba: o painel `site` vai para a frente (`ui_pane_show`).
pub async fn show(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>,
    path: Result<Path<String>, PathRejection>, req: Request) -> Response {
    let deadline = Instant::now() + REQUEST_BUDGET;
    let (name, headers, raw) = match owned(&st, peer, path, req, None).await { Ok(parts) => parts, Err(response) => return response };
    let request: ShowBody = match body(&headers, raw).await { Ok(request) => request, Err(response) => return response };
    if !fits(&request.site, 64) {
        return invalid(&headers);
    }
    run(&st, &headers, &name, ModsCall::Show { site: request.site }, deadline).await
}

/// Digitação num `Input` de mod: `change` a cada mudança, `submit` no Enter.
pub async fn input(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>,
    path: Result<Path<String>, PathRejection>, req: Request) -> Response {
    let deadline = Instant::now() + REQUEST_BUDGET;
    let (name, headers, raw) = match owned(&st, peer, path, req, Some(no_typing)).await { Ok(parts) => parts, Err(response) => return response };
    let request: InputBody = match body(&headers, raw).await { Ok(request) => request, Err(response) => return response };
    if !fits(&request.site, 64) || !fits(&request.key, 256) || !matches!(request.kind.as_str(), "change" | "submit")
        || request.value.chars().count() > VALUE_MAX {
        return invalid(&headers);
    }
    let call = ModsCall::Input { site: request.site, key: request.key, submit: request.kind == "submit", value: request.value };
    run(&st, &headers, &name, call, deadline).await
}
