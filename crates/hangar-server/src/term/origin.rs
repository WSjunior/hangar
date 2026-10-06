//! A Origin do painel continua decidida pelo Python (`termsock._origem_aceita`): as fontes dela
//! (`public_url`, `CP_TERM_ORIGINS`, a tela, o `peers.json`) moram lá.
use std::net::SocketAddr;
use std::time::Duration;

use axum::body::Body;
use http_body_util::BodyExt;

use crate::proxy::HttpClient;

const DEADLINE: Duration = Duration::from_secs(1);

/// `Ok(aceita)`, ou o código da falha da pergunta.
pub(crate) async fn ask(http: &HttpClient, upstream: SocketAddr, secret: &str, origin: &str, host: Option<&str>)
    -> Result<bool, &'static str> {
    let body = serde_json::json!({"origin": origin, "host": host}).to_string();
    let req = axum::http::Request::post(format!("http://{upstream}/internal/term/origin"))
        .header("x-hangar-internal", secret)
        .header("content-type", "application/json")
        .body(Body::from(body))
        .map_err(|_| "origin_request")?;
    let answer = tokio::time::timeout(DEADLINE, async {
        let resp = http.request(req).await.map_err(|_| "origin_unreachable")?;
        if !resp.status().is_success() {
            return Err("origin_refused");
        }
        resp.into_body().collect().await.map(|b| b.to_bytes()).map_err(|_| "origin_unreachable")
    }).await.map_err(|_| "origin_timeout")??;
    serde_json::from_slice::<serde_json::Value>(&answer).ok()
        .and_then(|v| v.get("ok").and_then(serde_json::Value::as_bool))
        .ok_or("origin_invalid")
}
