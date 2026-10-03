//! Contrato terminal privado: autentica antes de consumir o corpo.
use std::{net::SocketAddr, sync::Arc, time::Duration};
use axum::{body::to_bytes, extract::{ConnectInfo, Request, State}, http::StatusCode, response::{IntoResponse, Response}};
use serde::Deserialize;
use serde_json::Value;
use subtle::ConstantTimeEq;
use crate::{routes::AppState, terminal_control::CaptureRequest};

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
