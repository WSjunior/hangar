//! Contrato terminal privado: autentica antes de consumir o corpo.
use std::{net::SocketAddr, sync::Arc, time::Duration};
use axum::{body::to_bytes, extract::{ConnectInfo, Request, State}, http::StatusCode, response::{IntoResponse, Response}};
use serde::Deserialize;
use serde_json::Value;
use subtle::ConstantTimeEq;
use crate::{routes::AppState, terminal_control::CaptureRequest, terminal_state::{self, ReducerFacts, ReducerMemory}};

pub const MAX_BODY: usize = 16 * 1024 * 1024;

fn json(value: Value) -> Response {
    ([("content-type", "application/json")], value.to_string()).into_response()
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Operation {
    Acquire(CaptureRequest),
    Capture(CaptureRequest),
    Release { consumer: String },
    Reduce { pane: String, memory: Value, facts: Value },
}

fn exact(value: &Value, fields: &[&str]) -> bool {
    value.as_object().is_some_and(|o| o.len() == fields.len() && fields.iter().all(|k| o.contains_key(*k)))
}
fn optional_string(value: &Value) -> bool { value.is_null() || value.is_string() }
fn strings(value: &Value) -> bool { value.as_array().is_some_and(|a| a.iter().all(Value::is_string)) }
fn state(value: &Value) -> bool { value.as_str().is_some_and(|s| matches!(s, "working" | "idle" | "awaiting_input" | "dead")) }
fn question(value: &Value) -> bool {
    value.is_null() || (exact(value, &["question", "options"]) && optional_string(&value["question"]) && strings(&value["options"]))
}
fn valid_plugin(value: &Value) -> bool {
    if value.is_null() { return true; }
    let Some(o) = value.as_object() else { return false; };
    if o.get("id").and_then(Value::as_str).is_none() { return false; }
    if value["id"].as_str().unwrap().starts_with("perm:") {
        return o.get("tool").is_none_or(optional_string) && o.get("resumo").is_none_or(optional_string);
    }
    value["questions"].as_array().is_some_and(|qs| qs.iter().all(|q| {
        q.is_object() && q.get("question").is_none_or(optional_string)
            && q.get("options").is_none_or(|opts| opts.is_null() || opts.as_array().is_some_and(|a|
                a.iter().all(|option| option.is_object() && option.get("label").is_some_and(Value::is_string))))
    }))
}
fn reducer_inputs(memory: Value, facts: Value) -> Option<(ReducerMemory, ReducerFacts)> {
    if !exact(&memory, &["prev_spinner", "frozen", "no_spinner", "held_state", "held_label"])
        || !optional_string(&memory["prev_spinner"]) || !optional_string(&memory["held_label"]) || !state(&memory["held_state"])
        || !["frozen", "no_spinner"].iter().all(|k| memory[k].as_u64().is_some_and(|n| n <= u32::MAX as u64))
        || !exact(&facts, &["open_question", "plugin_question", "plugin_state", "hook_state", "hook_grace", "status_line"])
        || !question(&facts["open_question"]) || !valid_plugin(&facts["plugin_question"])
        || !["plugin_state", "hook_state"].iter().all(|k| facts[k].is_null() || matches!(facts[k].as_str(), Some("working" | "idle")))
        || !(facts["hook_grace"].is_null() || facts["hook_grace"].as_u64().is_some_and(|n| n <= u32::MAX as u64))
        || !optional_string(&facts["status_line"]) {
        return None;
    }
    Some((serde_json::from_value(memory).ok()?, serde_json::from_value(facts).ok()?))
}

pub async fn terminal(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let supplied = req.headers().get("x-hangar-internal").map(|v| v.as_bytes()).unwrap_or_default();
    let external = req.headers().get_all("x-forwarded-for").iter().any(|header|
        header.to_str().map_or(true, |v| v.split(',').any(|ip|
            ip.trim().parse::<std::net::IpAddr>().map_or(true, |ip| !ip.is_loopback()))));
    if !peer.ip().is_loopback() || external || st.cfg.internal_secret.is_empty()
        || !bool::from(supplied.ct_eq(st.cfg.internal_secret.as_bytes())) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let invalid = || {
        tracing::warn!(code = "invalid terminal request", "observação terminal recusada");
        (StatusCode::BAD_REQUEST, "invalid terminal request").into_response()
    };
    let bytes = match tokio::time::timeout(Duration::from_secs(6), to_bytes(req.into_body(), MAX_BODY)).await {
        Ok(Ok(bytes)) => bytes,
        _ => return invalid(),
    };
    let Ok(op) = serde_json::from_slice::<Operation>(&bytes) else { return invalid(); };
    let result = match op {
        Operation::Acquire(request) => st.terminal.acquire(request).await.map(|_| serde_json::json!({})),
        Operation::Capture(request) => st.terminal.capture(request).await.and_then(|r| serde_json::to_value(r)
            .map_err(|_| crate::terminal_control::TerminalError("invalid terminal result"))),
        Operation::Release { consumer } => {
            if consumer.is_empty() || consumer.len() > 128 { return invalid(); }
            st.terminal.release(&consumer).await.map(|_| serde_json::json!({}))
        }
        Operation::Reduce { pane, memory, facts } => {
            let Some((memory, facts)) = reducer_inputs(memory, facts) else { return invalid(); };
            let (result, diagnostic) = terminal_state::reduce_with_diagnostics(&pane, memory, facts);
            return json(serde_json::json!({"analysis": result.analysis, "memory": result.memory, "diagnostic": diagnostic}));
        }
    };
    match result {
        Ok(value) => json(value),
        Err(e) => {
            tracing::warn!(code = e.0, "observação terminal usa reserva Python");
            if matches!(e.0, "invalid capture request" | "invalid terminal target") {
                invalid()
            } else {
                (StatusCode::SERVICE_UNAVAILABLE, "terminal observer unavailable").into_response()
            }
        }
    }
}
