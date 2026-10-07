//! `/interrupt`, `/select`, `/select/submit`, `/keys`, `/term-input` e `DELETE …/queue/{id}` de sessão
//! Claude, com a mesma resposta do Python (`api.py`). Daqui para baixo o Rust é o dono: falha é erro
//! com código, nunca repasse. A pergunta que o plugin segura ainda mora no Python e vem pela rota
//! interna temporária `/internal/sessions/{name}/plugin` (sai quando o plugin for do Rust).
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use http_body_util::BodyExt;
use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use serde_json::{Value, json};

use super::input::{
    Diary, MSG_CONTROL_DEFERRED, MSG_CONTROL_UNCONFIRMED, MSG_STEER_REFUSED, MSG_STEER_UNKNOWN, answer, control_failed,
    diary_code, json_content_type, payload_code, runtime_failed, sem_turno,
};
use super::{Ctx, WriteRoute, admit, detail_body, relay};
use crate::mods::state::random_hex;
use crate::routes::AppState;
use crate::runtime::gateway::EntryHandle;
use crate::runtime::protocol::{Disposition, OperationKind, RuntimeCommand, RuntimeError, RuntimeReply};
use crate::runtime::queue::Action;

type Answer = (StatusCode, Value);

/// Pergunta humana e de uma vez só: sem resposta a escrita falha com código, não espera.
const PLUGIN_TIMEOUT: Duration = Duration::from_secs(5);

const MSG_NO_TURN: &str = "Não há turno ativo para interromper.";
const MSG_PERM_OPTION: &str = "opção fora do pedido de permissão";
const MSG_PANEL_OPEN: &str = "Terminal aberto nesta sessao. Feche o painel pra responder por aqui.";
const MSG_NOT_SENT: &str = "não consegui responder pelo terminal — opção NÃO enviada";
const MSG_UNCONFIRMED_ANSWER: &str = "resposta enviada, mas nao deu pra confirmar a tempo — confira na sessao antes de responder de novo";
const MSG_NOT_MARKED: &str = "não consegui marcar essa opção no terminal — tente de novo";
const MSG_NOT_SUBMITTED: &str = "não consegui enviar as opções marcadas — tente de novo";
const MSG_NO_PERMISSION: &str = "nenhum pedido de permissão pendente";
const MSG_NOT_IN_QUEUE: &str = "entrada não está na fila";
const MSG_PLUGIN_DOWN: &str = "não consegui conferir a pergunta pendente — nada foi enviado";

fn ok() -> Answer { (StatusCode::OK, json!({"ok": true})) }

fn failure(code: &str, msg: &str, params: Value) -> Answer { (StatusCode::CONFLICT, detail_body(code, msg, params)) }

/// O painel de terminal anexado trunca o pane: quem conta linha nele escolheria errado.
pub fn panel_open_refusal() -> Answer { failure("erro_terminal_aberto", MSG_PANEL_OPEN, json!({})) }

// ── /select ─────────────────────────────────────────────────────────────────────────────────────

/// O que o Python decide antes do ator: o controle a enviar, ou a recusa pronta. A pergunta que o
/// plugin segura (`pending`) manda no `request_id`; permissão `perm:` só tem as opções 1 e 2.
pub fn select_plan(pending: Option<&Value>, option: u64, panel_open: bool) -> Result<Value, Answer> {
    let mut payload = json!({"option": option});
    let held = pending.map(|p| p["id"].as_str().map_or_else(|| p["id"].to_string(), str::to_owned));
    let permission = held.as_deref().is_some_and(|id| id.starts_with("perm:"));
    if let Some(id) = &held {
        payload["request_id"] = json!(id);
        if permission && !matches!(option, 1 | 2) { return Err(failure("erro_opcao_nao_convergiu", MSG_PERM_OPTION, json!({}))); }
    }
    if !permission {
        // `ask:` pode acabar no teclado da TUI: vale a trava do painel e o cursor tem de ser lido.
        if panel_open { return Err(panel_open_refusal()); }
        if held.is_some() { payload["require_cursor"] = json!(true); }
    }
    Ok(payload)
}

pub fn select_terminal_answer(sent: &Result<RuntimeReply, RuntimeError>) -> (Answer, Diary) {
    let reply = match sent {
        Ok(reply) => reply,
        // Antes da entrega: nada chegou ao pane.
        Err(error) => return ((StatusCode::SERVICE_UNAVAILABLE, detail_body("erro_opcao_nao_convergiu", MSG_NOT_SENT, json!({"detalhe": error.to_string()}))), None),
    };
    match reply.disposition {
        Disposition::Accepted => (ok(), None),
        Disposition::Deferred => (control_failed(MSG_CONTROL_DEFERRED), None),
        Disposition::Unknown => (failure("erro_sem_confirmacao_resposta", MSG_UNCONFIRMED_ANSWER, json!({})), None),
        Disposition::Rejected => {
            let code = payload_code(reply).unwrap_or("rejected");
            (failure("erro_opcao_nao_convergiu", MSG_NOT_MARKED, json!({"detalhe": code})), Some(("opcao.nao_convergiu", diary_code(code))))
        }
    }
}

/// Sem terminal a opção responde ao pedido de permissão em aberto (1 permite, 2 nega).
pub fn select_headless_answer(sent: &Result<RuntimeReply, RuntimeError>) -> Answer {
    let not_answered = |msg: &str| (StatusCode::SERVICE_UNAVAILABLE, detail_body("erro_opcao_nao_convergiu", &format!("não consegui responder: {msg}"), json!({})));
    match sent {
        // O ator recusa a opção sem permissão em aberto com este erro (não como resposta).
        Err(error) if error.message == "nenhuma permissão pendente" => failure("erro_opcao_nao_convergiu", MSG_NO_PERMISSION, json!({})),
        Err(error) => not_answered(&error.to_string()),
        Ok(reply) => match reply.disposition {
            Disposition::Accepted => ok(),
            Disposition::Rejected => failure("erro_opcao_nao_convergiu", MSG_NO_PERMISSION, json!({})),
            Disposition::Unknown => not_answered("resultado incerto; a operação foi conservada sem reenvio"),
            Disposition::Deferred => not_answered(MSG_STEER_REFUSED),
        },
    }
}

/// Corpo do `SelectBody`: só `option`, inteiro de 1 a 50. O resto (texto "1", decimal, campo a mais)
/// é a conversão e o 422 do FastAPI, que recebe o pedido intacto.
fn parse_option(bytes: &Bytes) -> Option<u64> {
    let object = serde_json::from_slice::<Value>(bytes).ok()?;
    let object = object.as_object()?;
    if object.len() != 1 { return None; }
    object.get("option")?.as_u64().filter(|n| (1..=50).contains(n))
}

// ── /select/submit ──────────────────────────────────────────────────────────────────────────────

pub fn submit_answer(sent: &Result<RuntimeReply, RuntimeError>) -> (Answer, Diary) {
    let reply = match sent {
        Ok(reply) => reply,
        Err(error) => return (runtime_failed(error), None),
    };
    match reply.disposition {
        Disposition::Accepted => (ok(), None),
        Disposition::Deferred => (control_failed(MSG_CONTROL_DEFERRED), None),
        Disposition::Unknown => (control_failed(MSG_CONTROL_UNCONFIRMED), None),
        Disposition::Rejected => {
            let code = payload_code(reply).unwrap_or("rejected");
            (failure("erro_opcao_nao_convergiu", MSG_NOT_SUBMITTED, json!({"detalhe": code})), Some(("opcao.envio_falhou", diary_code(code))))
        }
    }
}

// ── /interrupt, /keys, /term-input ──────────────────────────────────────────────────────────────

/// Um controle de terminal que só responde ok ou recusa; a tecla ou o texto que o ator recusa é a
/// validação do Python (`ValueError` → 400 com texto puro).
pub fn control_step_answer(sent: &Result<RuntimeReply, RuntimeError>) -> Result<(), Answer> {
    let reply = sent.as_ref().map_err(runtime_failed)?;
    match reply.disposition {
        Disposition::Accepted => Ok(()),
        Disposition::Deferred => Err(control_failed(MSG_CONTROL_DEFERRED)),
        Disposition::Rejected if payload_code(reply) == Some("key_not_allowed") => Err((StatusCode::BAD_REQUEST, json!({"detail": "tecla não permitida"}))),
        Disposition::Rejected if payload_code(reply) == Some("invalid_text") => Err((StatusCode::BAD_REQUEST, json!({"detail": "texto inválido para terminal"}))),
        _ => Err(control_failed(MSG_CONTROL_UNCONFIRMED)),
    }
}

pub fn interrupt_terminal_answer(sent: &Result<RuntimeReply, RuntimeError>) -> Answer {
    control_step_answer(sent).map_or_else(|refused| refused, |()| ok())
}

/// Sem turno em voo não há o que interromper; responder ok seria fingir.
pub fn interrupt_headless_answer(sent: &Result<RuntimeReply, RuntimeError>) -> Answer {
    let reply = match sent {
        Ok(reply) => reply,
        Err(error) => return sem_turno(&error.to_string()),
    };
    match reply.disposition {
        Disposition::Accepted if reply.payload["interrupted"] == false => sem_turno(MSG_NO_TURN),
        Disposition::Accepted => ok(),
        Disposition::Unknown => sem_turno(MSG_STEER_UNKNOWN),
        _ => sem_turno(reply.payload["error"].as_str().filter(|e| !e.is_empty()).unwrap_or(MSG_STEER_REFUSED)),
    }
}

/// Texto do terminal interativo: qualquer coisa, menos caractere de controle (fora `\n` e `\t`).
fn terminal_text_ok(text: &str) -> bool { !text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\t')) }

/// Os controles do `/term-input` na ordem do Python: texto, depois tecla. Texto inválido recusa antes
/// de enviar qualquer coisa.
pub fn term_input_steps(text: Option<&str>, key: Option<&str>) -> Result<Vec<(&'static str, Value)>, Answer> {
    let mut steps = Vec::new();
    if let Some(text) = text.filter(|t| !t.is_empty()) {
        if !terminal_text_ok(text) { return Err((StatusCode::BAD_REQUEST, json!({"detail": "texto inválido para terminal"}))); }
        steps.push(("terminal_input", json!({"text": text})));
    }
    if let Some(key) = key.filter(|k| !k.is_empty()) { steps.push(("interactive_key", json!({"key": key}))); }
    Ok(steps)
}

/// `clear` como o FastAPI lê um booleano de query; `None` = valor que ele recusa (422 dele).
fn parse_clear(query: Option<&str>) -> Option<bool> {
    let mut clear = Some(false);
    for pair in query.unwrap_or_default().split('&') {
        let Some(("clear", value)) = pair.split_once('=').or(Some((pair, ""))) else { continue };
        clear = match percent_decode_str(value).decode_utf8().ok()?.to_ascii_lowercase().as_str() {
            "1" | "true" | "t" | "yes" | "y" | "on" => Some(true),
            "0" | "false" | "f" | "no" | "n" | "off" => Some(false),
            _ => None,
        };
    }
    clear
}

/// `{"key": texto}` do `KeyBody` estrito.
fn parse_key(bytes: &Bytes) -> Option<String> {
    let object = serde_json::from_slice::<Value>(bytes).ok()?;
    let object = object.as_object()?;
    if object.len() != 1 { return None; }
    object.get("key")?.as_str().map(str::to_owned)
}

/// `{"text": texto|null, "key": texto|null}` do `TermInputBody` estrito, os dois opcionais.
fn parse_term_input(bytes: &Bytes) -> Option<(Option<String>, Option<String>)> {
    let object = serde_json::from_slice::<Value>(bytes).ok()?;
    let object = object.as_object()?;
    if object.keys().any(|k| k != "text" && k != "key") { return None; }
    let field = |name: &str| match object.get(name) { None | Some(Value::Null) => Some(None), Some(Value::String(s)) => Some(Some(s.clone())), Some(_) => None };
    Some((field("text")?, field("key")?))
}

// ── DELETE …/queue/{id} ─────────────────────────────────────────────────────────────────────────

pub fn queue_answer(removed: &Result<Value, RuntimeError>) -> Answer {
    match removed {
        Ok(done) if *done == json!(true) => ok(),
        Ok(_) => (StatusCode::NOT_FOUND, detail_body("erro_fila_entrada_nao_encontrada", MSG_NOT_IN_QUEUE, json!({}))),
        Err(error) => runtime_failed(error),
    }
}

// ── a pergunta do plugin, pela rota interna do Python ───────────────────────────────────────────

fn plugin_request(st: &AppState, name: &str, post: Option<Value>) -> Option<Request> {
    let url = format!("http://{}/internal/sessions/{}/plugin", st.cfg.upstream, utf8_percent_encode(name, NON_ALPHANUMERIC));
    let builder = match post { Some(_) => Request::post(url).header("content-type", "application/json"), None => Request::get(url) };
    builder.header("x-hangar-internal", &st.cfg.internal_secret).body(post.map_or_else(Body::empty, |body| Body::from(body.to_string()))).ok()
}

async fn plugin_call(st: &AppState, name: &str, post: Option<Value>) -> Option<Value> {
    let request = plugin_request(st, name, post)?;
    let response = tokio::time::timeout(PLUGIN_TIMEOUT, st.http.request(request)).await.ok()?.ok()?;
    if !response.status().is_success() { return None; }
    let body = tokio::time::timeout(PLUGIN_TIMEOUT, response.into_body().collect()).await.ok()?.ok()?.to_bytes();
    serde_json::from_slice(&body).ok()
}

/// A pergunta que o plugin segura; `Err` quando o Python não respondeu (a escrita não começa).
async fn plugin_pending(ctx: &Ctx) -> Result<Option<Value>, ()> {
    match plugin_call(&ctx.st, &ctx.name, None).await {
        Some(body) => Ok(Some(body["pending"].clone()).filter(|p| !p.is_null())),
        None => {
            ctx.st.diag.report("rust.plugin_lookup_failed", &ctx.name, "plugin_lookup", "a pergunta do plugin não foi lida; a escrita não saiu");
            Err(())
        }
    }
}

/// O Esc já foi dado: falha aqui não desfaz nada, só vai ao diário.
async fn plugin_interrupted(ctx: &Ctx, id: Option<&str>) {
    if plugin_call(&ctx.st, &ctx.name, Some(json!({"interrupted": id}))).await.is_none() {
        ctx.st.diag.report("rust.plugin_notice_failed", &ctx.name, "plugin_notice", "o plugin não soube da interrupção; a pergunta pode seguir aberta");
    }
}

// ── rotas ───────────────────────────────────────────────────────────────────────────────────────

fn report(ctx: &Ctx, diary: Diary) {
    let Some((event, code)) = diary else { return };
    let reason = if event == "opcao.nao_convergiu" { "o terminal não confirmou a opção marcada" } else { "o terminal não aceitou o envio das opções marcadas" };
    ctx.st.diag.report(event, &ctx.name, &code, reason);
}

fn plugin_down(code: &str) -> Answer {
    (StatusCode::SERVICE_UNAVAILABLE, detail_body(code, MSG_PLUGIN_DOWN, json!({"erro": "plugin_unavailable"})))
}

pub async fn select(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let (ctx, bytes) = match admit(&st, peer, req, WriteRoute::Select, |_| false).await {
        Ok(admitted) => admitted,
        Err(response) => return response,
    };
    let Some(option) = parse_option(&bytes).filter(|_| json_content_type(ctx.headers())) else { return relay(ctx, bytes).await };
    let operation_id = random_hex(16);
    let result = match &ctx.target.handle {
        EntryHandle::Terminal { handle, .. } => match plugin_pending(&ctx).await {
            Err(()) => plugin_down("erro_opcao_nao_convergiu"),
            Ok(pending) => match select_plan(pending.as_ref(), option, st.term.is_active(&ctx.name)) {
                Err(refused) => refused,
                Ok(payload) => {
                    let (answered, diary) = select_terminal_answer(&handle.control(operation_id, "select".into(), payload).await);
                    report(&ctx, diary);
                    answered
                }
            },
        },
        headless => select_headless_answer(&headless.command(RuntimeCommand { operation_id, kind: OperationKind::Select, payload: json!({"option": option}) }).await),
    };
    answer(&ctx, result)
}

pub async fn select_submit(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let (ctx, bytes) = match admit(&st, peer, req, WriteRoute::SelectSubmit, |_| false).await {
        Ok(admitted) => admitted,
        Err(response) => return response,
    };
    // Sem pane não há aba Submit para dirigir: quem responde, como hoje, é o Python.
    let EntryHandle::Terminal { handle, .. } = &ctx.target.handle else { return relay(ctx, bytes).await };
    let result = if st.term.is_active(&ctx.name) { panel_open_refusal() } else {
        let (answered, diary) = submit_answer(&handle.control(random_hex(16), "submit_selected".into(), json!({})).await);
        report(&ctx, diary);
        answered
    };
    answer(&ctx, result)
}

pub async fn interrupt(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let query = req.uri().query().map(str::to_owned);
    let (ctx, bytes) = match admit(&st, peer, req, WriteRoute::Interrupt, |_| false).await {
        Ok(admitted) => admitted,
        Err(response) => return response,
    };
    let Some(clear) = parse_clear(query.as_deref()) else { return relay(ctx, bytes).await };
    let operation_id = random_hex(16);
    let result = match &ctx.target.handle {
        EntryHandle::Terminal { handle, .. } => match plugin_pending(&ctx).await {
            Err(()) => plugin_down("erro_envio_falhou"),
            Ok(pending) => {
                let asked = pending.as_ref().and_then(|p| p["id"].as_str()).map(str::to_owned);
                let interrupted = interrupt_terminal_answer(&handle.control(operation_id, "interrupt".into(), json!({"clear": clear})).await);
                if interrupted.0 == StatusCode::OK { plugin_interrupted(&ctx, asked.as_deref()).await; }
                interrupted
            }
        },
        headless => interrupt_headless_answer(&headless.command(RuntimeCommand { operation_id, kind: OperationKind::Interrupt, payload: json!({}) }).await),
    };
    answer(&ctx, result)
}

pub async fn keys(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let (ctx, bytes) = match admit(&st, peer, req, WriteRoute::Keys, |_| false).await {
        Ok(admitted) => admitted,
        Err(response) => return response,
    };
    let (EntryHandle::Terminal { handle, .. }, Some(key)) = (&ctx.target.handle, parse_key(&bytes).filter(|_| json_content_type(ctx.headers()))) else { return relay(ctx, bytes).await };
    let result = control_step_answer(&handle.control(random_hex(16), "navigation_key".into(), json!({"key": key})).await).map_or_else(|refused| refused, |()| ok());
    answer(&ctx, result)
}

pub async fn term_input(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let (ctx, bytes) = match admit(&st, peer, req, WriteRoute::TermInput, |_| false).await {
        Ok(admitted) => admitted,
        Err(response) => return response,
    };
    let (EntryHandle::Terminal { handle, .. }, Some((text, key))) = (&ctx.target.handle, parse_term_input(&bytes).filter(|_| json_content_type(ctx.headers()))) else { return relay(ctx, bytes).await };
    let result = match term_input_steps(text.as_deref(), key.as_deref()) {
        Err(refused) => refused,
        Ok(steps) => {
            let mut done = ok();
            for (control, payload) in steps {
                if let Err(refused) = control_step_answer(&handle.control(random_hex(16), control.into(), payload).await) { done = refused; break; }
            }
            done
        }
    };
    answer(&ctx, result)
}

pub async fn queue_remove(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let (ctx, bytes) = match admit(&st, peer, req, WriteRoute::QueueRemove, |_| false).await {
        Ok(admitted) => admitted,
        Err(response) => return response,
    };
    // `/api/sessions/{name}/queue/{id}`: o id é um segmento só; o que sobrar é rota que o FastAPI não casa.
    let path = ctx.parts.uri.path().to_owned();
    let segments: Vec<&str> = path.trim_start_matches("/api/sessions/").split('/').collect();
    let Some(entry_id) = (segments.len() == 3 && segments[1] == "queue").then(|| percent_decode_str(segments[2]).decode_utf8().ok()).flatten().filter(|id| !id.is_empty()) else { return relay(ctx, bytes).await };
    let removed = ctx.target.handle.queue(random_hex(16), Action::Remove { entry_id: entry_id.into_owned() }).await;
    answer(&ctx, queue_answer(&removed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_reads_like_a_fastapi_bool() {
        assert_eq!(parse_clear(None), Some(false));
        assert_eq!(parse_clear(Some("clear=true")), Some(true));
        assert_eq!(parse_clear(Some("x=1&clear=No")), Some(false));
        assert_eq!(parse_clear(Some("clear=0&clear=on")), Some(true), "o último vale");
        assert_eq!(parse_clear(Some("clear=talvez")), None);
        assert_eq!(parse_clear(Some("clear=")), None);
        assert_eq!(parse_clear(Some("clear")), None);
    }

    #[test]
    fn terminal_text_refuses_control_characters_but_not_newline_or_tab() {
        assert!(terminal_text_ok("ls\n\tx"));
        assert!(!terminal_text_ok("a\u{1}b"));
    }
}
