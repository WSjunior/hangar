//! Validação e respostas JSON comuns às rotas dos mods: as dos apps (`routes`) e as da ponte do plugin
//! (`bridge`).
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::routes::cors;

/// Os limites do Pydantic do Python: não vazio e até `max` caracteres (não bytes).
pub(crate) fn fits(text: &str, max: usize) -> bool {
    !text.is_empty() && text.chars().count() <= max
}

/// Resposta JSON. Com os cabeçalhos do pedido, leva o CORS dos apps; a ponte é chamada pelo plugin, sem
/// `Origin`, e passa `None`.
pub(crate) fn reply(headers: Option<&HeaderMap>, status: StatusCode, body: Value) -> Response {
    let mut response = (status, [(header::CONTENT_TYPE, "application/json")], body.to_string()).into_response();
    if let Some(headers) = headers {
        cors(headers, response.headers_mut());
    }
    response
}

pub(crate) fn invalid(headers: Option<&HeaderMap>) -> Response {
    reply(headers, StatusCode::UNPROCESSABLE_ENTITY, json!({"detail": "corpo inválido"}))
}
