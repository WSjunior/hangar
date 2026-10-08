//! Rotas só do Codex (`/models`, `/model`, `/service-tier`, `/codex/mode`, `/limits`, `/question/skip`,
//! `/commands`, `/codex-permissions`) da sessão Codex sem terminal, com o corpo e o código da rota
//! Python no modo `python` (`api.py`). Codex com terminal e o que não é sessão Codex do Rust seguem ao
//! Python antes da porta: a lista de comandos do Claude não pode esperar uma troca de agente.
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{Map, Value, json};

use super::input::{MSG_CODEX_CONTROL, answer, codex_control, json_content_type};
use super::{Ctx, WriteRoute, admit, detail_body, enter_then_find, json_response, relay, session_name};
use crate::mods::state::random_hex;
use crate::routes::{AppState, cors, gate, pass};
use crate::runtime::protocol::{Disposition, OperationKind, RuntimeCommand, RuntimeError, RuntimeReply};

type Answer = (StatusCode, Value);

const MSG_INVALID_ANSWER: &str = "A pergunta mudou ou não aceita essas respostas. Confira as opções e tente novamente.";
const MSG_ANSWER_UNSENT: &str = "Não foi possível confirmar o envio da resposta ao Codex.";
/// `sem_terminal.MODOS` (nome e descrição), na ordem da tela.
const PERMISSION_MODES: [(&str, &str); 3] = [
    ("Ask for approval", "Codex só lê o workspace; editar ou rodar comando pede aprovação."),
    ("Approve for me", "Codex edita o workspace sozinho; fora dele ou com rede, pede aprovação."),
    ("Full Access", "Codex faz tudo sem perguntar."),
];
const DEFAULT_PERMISSION_MODE: &str = "Full Access";

fn accepted(sent: &Result<RuntimeReply, RuntimeError>) -> Option<&Value> {
    sent.as_ref().ok().filter(|reply| reply.disposition == Disposition::Accepted).map(|reply| &reply.payload)
}

fn refused() -> Answer { codex_control(MSG_CODEX_CONTROL) }

// ── corpos de resposta, sem E/S ─────────────────────────────────────────────────────────────────

/// `/models`: configuração lida antes; a lista que falha sai vazia, como no adapter Python.
pub fn models_answer(settings: &Result<RuntimeReply, RuntimeError>, models: &Result<RuntimeReply, RuntimeError>) -> Answer {
    let Some(current) = accepted(settings) else { return refused() };
    let models = accepted(models).filter(|m| m.is_array()).cloned().unwrap_or_else(|| json!([]));
    (StatusCode::OK, json!({"models": models, "current": current}))
}

pub fn model_answer(sent: &Result<RuntimeReply, RuntimeError>) -> Answer {
    if accepted(sent).is_some() { (StatusCode::OK, json!({"ok": true})) } else { refused() }
}

pub fn service_tier_answer(tier: &str, sent: &Result<RuntimeReply, RuntimeError>) -> Answer {
    match accepted(sent) {
        Some(done) if done["service_tier"] == tier => (StatusCode::OK, json!({"ok": true, "service_tier": tier})),
        _ => refused(),
    }
}

pub fn mode_answer(sent: &Result<RuntimeReply, RuntimeError>) -> Answer {
    accepted(sent).map_or_else(refused, |settings| (StatusCode::OK, settings.clone()))
}

/// `_normalize_rate_window`: só os três campos que o app lê.
fn rate_window(window: &Value) -> Value {
    if window.is_null() { return Value::Null; }
    json!({"usedPercent": window["usedPercent"], "windowMins": window["windowDurationMins"], "resetsAt": window["resetsAt"]})
}

/// `/limits`: falha de leitura é a resposta neutra, nunca erro (o app só não mostra o chip).
pub fn limits_answer(sent: &Result<RuntimeReply, RuntimeError>) -> Answer {
    let snapshot = accepted(sent).map_or(&Value::Null, |result| &result["rateLimits"]);
    if snapshot.is_null() { return (StatusCode::OK, json!({"primary": null, "secondary": null, "planType": null})); }
    (StatusCode::OK, json!({"primary": rate_window(&snapshot["primary"]), "secondary": rate_window(&snapshot["secondary"]), "planType": snapshot["planType"]}))
}

/// `/question/skip`: pergunta que não está mais lá (recusa do motor) é 409; falha de envio é 503.
pub fn skip_answer(sent: &Result<RuntimeReply, RuntimeError>) -> Answer {
    let invalid = || (StatusCode::CONFLICT, detail_body("erro_codex_resposta_invalida", MSG_INVALID_ANSWER, json!({})));
    match sent {
        Ok(reply) if reply.disposition == Disposition::Accepted => (StatusCode::OK, json!({"ok": true})),
        Ok(reply) if reply.disposition == Disposition::Rejected => invalid(),
        Err(error) if error.code == "codex_command" => invalid(),
        _ => (StatusCode::SERVICE_UNAVAILABLE, detail_body("erro_codex_resposta_envio", MSG_ANSWER_UNSENT, json!({}))),
    }
}

/// `/commands`: o `/compact` embutido primeiro; skill homônima dele sai, `path` e `native_name` não vão ao app.
pub fn commands_answer(sent: &Result<RuntimeReply, RuntimeError>) -> Answer {
    let Some(catalog) = accepted(sent) else { return refused() };
    let mut commands = vec![json!({"name": "compact", "display": "/compact", "source": "builtin",
        "description": "Resume e compacta o contexto", "destructive": true})];
    for mut skill in crate::runtime::local_policy::skills(catalog) {
        if skill["name"] == "compact" { continue; }
        if let Some(fields) = skill.as_object_mut() { fields.remove("path"); fields.remove("native_name"); }
        commands.push(skill);
    }
    (StatusCode::OK, Value::Array(commands))
}

/// `modos_para_tela`: o mesmo formato do picker da TUI.
pub fn permission_modes(current: Option<&str>) -> Value {
    let current = current.unwrap_or(DEFAULT_PERMISSION_MODE);
    let modes: Vec<Value> = PERMISSION_MODES.iter().enumerate().map(|(i, (name, desc))|
        json!({"numero": i + 1, "nome": name, "desc": desc, "cursor": *name == current, "atual": *name == current})).collect();
    json!({"modes": modes, "current": current})
}

pub fn permission_answer(sent: &Result<RuntimeReply, RuntimeError>) -> Answer {
    let picker = |status: StatusCode, msg: &str| (status, detail_body("erro_permissao_picker", msg, json!({})));
    match sent {
        Ok(reply) if reply.disposition == Disposition::Accepted => (StatusCode::OK, reply.payload.clone()),
        Err(error) if error.code == "erro_modo_desconhecido" => picker(StatusCode::BAD_REQUEST, &error.message),
        Err(error) if error.code == "erro_permissao_ocupada" => (StatusCode::CONFLICT, detail_body("erro_permissao_ocupada", &error.message, json!({}))),
        Err(error) => picker(StatusCode::SERVICE_UNAVAILABLE, &format!("não consegui reabrir o Codex: {}", error.message)),
        Ok(reply) => {
            let why = reply.payload["error"].as_str().unwrap_or("operação recusada pelo runtime");
            picker(StatusCode::SERVICE_UNAVAILABLE, &format!("não consegui reabrir o Codex: {why}"))
        }
    }
}

// ── corpos de pedido: o modelo estrito do FastAPI; o resto é o 422 dele, pelo repasse ──────────

fn object(bytes: &Bytes, headers: &axum::http::HeaderMap) -> Option<Map<String, Value>> {
    if !json_content_type(headers) { return None; }
    match serde_json::from_slice::<Value>(bytes).ok()? { Value::Object(fields) => Some(fields), _ => None }
}

/// Só os campos de `allowed`, com `required` presente e texto (ou `null` nos opcionais).
fn strings(fields: &Map<String, Value>, required: &str, optional: Option<&str>) -> Option<Map<String, Value>> {
    if fields.keys().any(|key| key != required && Some(key.as_str()) != optional) { return None; }
    fields.get(required)?.as_str()?;
    if let Some(key) = optional && !matches!(fields.get(key), None | Some(Value::Null) | Some(Value::String(_))) { return None; }
    Some(fields.clone())
}

fn literal(fields: &Map<String, Value>, key: &str, values: &[&str]) -> Option<String> {
    strings(fields, key, None)?;
    fields[key].as_str().filter(|value| values.contains(value)).map(str::to_owned)
}

// ── rotas ───────────────────────────────────────────────────────────────────────────────────────

async fn ours(st: &AppState, name: Option<&str>) -> bool {
    match (st.state.runtime.get(), name) {
        (Some(runtime), Some(name)) => runtime.writable(name).await.is_some_and(|t| t.provider == "codex" && !t.terminal),
        _ => false,
    }
}

/// Só a sessão Codex sem terminal do Rust passa pela porta; o resto vai ao Python como chegou.
async fn admit_codex(st: &Arc<AppState>, peer: SocketAddr, req: Request) -> Result<(Ctx, Bytes), Response> {
    if !ours(st, session_name(req.uri().path()).as_deref()).await {
        let (fwd, _) = gate(st, peer, &req);
        return Err(pass(st, req, &fwd).await);
    }
    admit(st, peer, req, WriteRoute::CodexControl, |_| false).await
}

async fn run(ctx: &Ctx, kind: OperationKind, payload: Value) -> Result<RuntimeReply, RuntimeError> {
    ctx.target.handle.command(RuntimeCommand { operation_id: random_hex(16), kind, payload }).await
}

/// Falha que a resposta engole (lista vazia, limites neutros) ainda vai ao log, como no adapter Python.
fn note(name: &str, what: &str, sent: &Result<RuntimeReply, RuntimeError>) {
    if accepted(sent).is_some() { return; }
    let code = match sent { Err(error) => error.code.clone(), Ok(reply) => format!("{:?}", reply.disposition).to_lowercase() };
    if crate::warn_limit::allow(Some(name), &format!("{what}:{code}")) {
        tracing::warn!(session = %name, what, code = %code, "leitura do Codex falhou; a rota responde vazio");
    }
}

macro_rules! admitted {
    ($st:expr, $peer:expr, $req:expr) => {
        match admit_codex(&$st, $peer, $req).await { Ok(admitted) => admitted, Err(response) => return response }
    };
}

pub async fn models(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let (ctx, _) = admitted!(st, peer, req);
    let settings = run(&ctx, OperationKind::ReadSettings, json!({"include_turns": false})).await;
    if accepted(&settings).is_none() { return answer(&ctx, refused()); }
    let models = run(&ctx, OperationKind::ListModels, json!({})).await;
    note(&ctx.name, "model_list", &models);
    answer(&ctx, models_answer(&settings, &models))
}

pub async fn model(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let (ctx, bytes) = admitted!(st, peer, req);
    let Some(body) = object(&bytes, ctx.headers()).and_then(|f| strings(&f, "model", Some("effort"))) else { return relay(ctx, bytes).await };
    let payload = json!({"model": body["model"], "effort": body.get("effort").cloned().unwrap_or(Value::Null)});
    let sent = run(&ctx, OperationKind::SetModel, payload).await;
    answer(&ctx, model_answer(&sent))
}

pub async fn service_tier(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let (ctx, bytes) = admitted!(st, peer, req);
    let Some(tier) = object(&bytes, ctx.headers()).and_then(|f| literal(&f, "service_tier", &["default", "priority"])) else { return relay(ctx, bytes).await };
    let sent = run(&ctx, OperationKind::SetServiceTier, json!({"service_tier": tier})).await;
    answer(&ctx, service_tier_answer(&tier, &sent))
}

pub async fn mode(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let (ctx, bytes) = admitted!(st, peer, req);
    let Some(mode) = object(&bytes, ctx.headers()).and_then(|f| literal(&f, "mode", &["default", "plan"])) else { return relay(ctx, bytes).await };
    let sent = run(&ctx, OperationKind::SetMode, json!({"mode": mode})).await;
    answer(&ctx, mode_answer(&sent))
}

/// `/limits` nunca é erro nem espera, como no Python: porta fechada (troca em curso) ou leitura que falha
/// dão a resposta neutra.
pub async fn limits(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let (fwd, owner) = gate(&st, peer, &req);
    let name = session_name(req.uri().path());
    let (Some(runtime), Some(name), true) = (st.state.runtime.get(), name, owner) else { return pass(&st, req, &fwd).await };
    if !ours(&st, Some(&name)).await { return pass(&st, req, &fwd).await; }
    let sent = match enter_then_find(runtime, &name, Duration::ZERO).await {
        Ok(Some((_held, target))) if target.provider == "codex" && !target.terminal =>
            target.handle.command(RuntimeCommand { operation_id: random_hex(16), kind: OperationKind::ReadRateLimits, payload: json!({}) }).await,
        _ => Err(RuntimeError::new("session_transfer_busy", "porta de entrada fechada")),
    };
    note(&name, "rate_limits", &sent);
    let (status, body) = limits_answer(&sent);
    let mut response = json_response(status, body);
    cors(req.headers(), response.headers_mut());
    response
}

pub async fn skip_question(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let (ctx, bytes) = admitted!(st, peer, req);
    let Some(body) = object(&bytes, ctx.headers()).and_then(|f| strings(&f, "request_id", None)) else { return relay(ctx, bytes).await };
    let sent = run(&ctx, OperationKind::SkipQuestion, json!({"request_id": body["request_id"]})).await;
    answer(&ctx, skip_answer(&sent))
}

pub async fn commands(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let (ctx, _) = admitted!(st, peer, req);
    let sent = run(&ctx, OperationKind::ListSkills, json!({})).await;
    answer(&ctx, commands_answer(&sent))
}

pub async fn permissions(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let (ctx, _) = admitted!(st, peer, req);
    // A vista publicada pelo ator: vale com o processo caído, como o arquivo da sessão no Python.
    let result = match ctx.target.handle.snapshot().await {
        Ok(snapshot) => (StatusCode::OK, permission_modes(snapshot["view"]["permission_mode"].as_str())),
        Err(error) => (StatusCode::SERVICE_UNAVAILABLE, detail_body("erro_permissao_picker", &error.to_string(), json!({}))),
    };
    answer(&ctx, result)
}

pub async fn set_permission(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let (ctx, bytes) = admitted!(st, peer, req);
    let Some(body) = object(&bytes, ctx.headers()).and_then(|f| strings(&f, "mode", None)) else { return relay(ctx, bytes).await };
    let sent = run(&ctx, OperationKind::SetPermissionMode, json!({"mode": body["mode"]})).await;
    answer(&ctx, permission_answer(&sent))
}
