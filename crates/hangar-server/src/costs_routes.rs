//! Custos e cotação para o dono; falha local conserva o caminho do Python.

use std::sync::Arc;
use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use crate::costs::collect::{CollectError, Ready};
use crate::costs::index::IndexError;
use crate::costs::py::LocalTs;
use crate::costs::{CacheKey, report_costs};
use crate::routes::{AppState, cors, gate, maybe_gzip, pass};

pub(crate) fn warming(read: usize, total: usize) -> Response {
    (StatusCode::ACCEPTED, [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({"aquecendo":true, "lidos":read, "total":total}).to_string()).into_response()
}

fn response(headers: &HeaderMap, body: Vec<u8>) -> Response {
    let mut response = Response::new(Body::empty());
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let body = maybe_gzip(headers, response.headers_mut(), body);
    *response.body_mut() = Body::from(body);
    cors(headers, response.headers_mut());
    response
}

fn error_code(error: &CollectError) -> &'static str {
    match error {
        CollectError::NoScopes => "custos_escopos",
        CollectError::Index(IndexError::NoDisk) => "custos_disco",
        CollectError::Index(IndexError::ReaderPanic) => "custos_leitor",
        CollectError::Index(IndexError::Sqlite(_)) => "custos_sqlite",
    }
}

fn fresh(value: Option<String>) -> Option<bool> {
    match value {
        None => Some(false),
        Some(value) => match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "on" | "yes" | "t" | "y" => Some(true),
            "0" | "false" | "off" | "no" | "f" | "n" => Some(false),
            _ => None,
        },
    }
}

enum Prepared {
    Warming(usize, usize),
    Body(Vec<u8>),
}

pub async fn costs(State(state): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>, request: Request) -> Response {
    let (forward, owner) = gate(&state, peer, &request);
    if !owner || request.method() != Method::GET { return pass(&state, request, &forward).await; }
    let Some(fresh) = fresh(crate::auth::query_param(request.uri().query(), "fresco")) else {
        return pass(&state, request, &forward).await;
    };
    let period = crate::auth::query_param(request.uri().query(), "period").filter(|p| {
        p == "all" || report_costs::PERIODS.iter().any(|(key, _)| p == key)
    }).unwrap_or_else(|| "all".into());
    let worker = state.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<Prepared, &'static str> {
        match worker.costs.prepare_blocking(fresh).map_err(|error| error_code(&error))? {
            Ready::Warming { read, total } => return Ok(Prepared::Warming(read, total)),
            Ready::Go => {}
        }
        let now = LocalTs(chrono::Utc::now().timestamp_micros());
        let labels = worker.costs.labels_key();
        let key = CacheKey {
            data_version: worker.costs.data_version(), pricing_generation: worker.costs.pricing().generation(),
            area_signature: worker.costs.areas().signature().into(), labels: labels.clone(),
            route: vec!["costs".into(), period.clone(), now.day()],
        };
        let report = match worker.reports.get::<report_costs::CostReport>(&key) {
            Some(report) => report,
            None => {
                // Ler usa Pricing internamente: só adquirir a tarifa depois da leitura.
                let rows = worker.costs.read_costs(report_costs::since(&period, now).as_deref())
                    .map_err(|error| error_code(&error))?;
                let report = Arc::new(report_costs::build(rows, &period, now, &worker.costs.pricing(), &|key| {
                    labels.iter().find(|(name, _)| name == key).map(|(_, label)| label.clone())
                }));
                worker.reports.insert(key, report.clone());
                report
            }
        };
        let mut report = (*report).clone();
        report.usd_brl = worker.fx.usd_brl();
        serde_json::to_vec(&report).map(Prepared::Body).map_err(|_| "custos_json")
    }).await;
    match result {
        Ok(Ok(Prepared::Body(body))) => response(request.headers(), body),
        Ok(Ok(Prepared::Warming(read, total))) => {
            let mut response = warming(read, total);
            cors(request.headers(), response.headers_mut());
            response
        }
        other => {
            let code = match other { Ok(Err(code)) => code, _ => "custos_join" };
            tracing::warn!(code);
            pass(&state, request, &forward).await
        }
    }
}

pub async fn cotacao(State(state): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>, request: Request) -> Response {
    let (forward, owner) = gate(&state, peer, &request);
    if !owner || request.method() != Method::GET { return pass(&state, request, &forward).await; }
    let fx = state.fx.clone();
    match tokio::task::spawn_blocking(move || fx.usd_brl()).await {
        Ok(rate) => response(request.headers(), serde_json::json!({"usd_brl":rate}).to_string().into_bytes()),
        Err(_) => {
            tracing::warn!(code = "cotacao_join");
            pass(&state, request, &forward).await
        }
    }
}
