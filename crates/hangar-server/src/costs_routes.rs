//! Custos e cotação para o dono; falha local conserva o caminho do Python.

use std::sync::Arc;
use axum::body::Body;
use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use crate::costs::collect::{CollectError, Ready};
use crate::costs::index::IndexError;
use crate::costs::py::LocalTs;
use crate::costs::{CacheKey, codex, report_costs, report_uso, session_cost};
use crate::routes::{AppState, cors, fetch_info, gate, maybe_gzip, pass};

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

fn session_not_found(headers: &HeaderMap, message: &'static str) -> Response {
    let body = serde_json::json!({"detail":{"code":"erro_sessao_inexistente", "params":{}, "msg":message}});
    let mut response = response(headers, body.to_string().into_bytes());
    *response.status_mut() = StatusCode::NOT_FOUND;
    response
}

pub async fn session_cost(State(state): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>, Path(name): Path<String>, request: Request) -> Response {
    let (forward, owner) = gate(&state, peer, &request);
    if !owner || request.method() != Method::GET { return pass(&state, request, &forward).await; }
    let info = fetch_info(&state.http, state.cfg.upstream, &state.cfg.internal_secret, &name).await;
    let Some(rollout) = info.filter(|info| info.provider == "codex").and_then(|info| info.jsonl)
        .filter(|path| !path.as_os_str().is_empty()) else {
        return session_not_found(request.headers(), "Codex session not found");
    };
    let worker = state.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<Option<Vec<u8>>, &'static str> {
        let Ok(path) = std::fs::canonicalize(rollout) else { return Ok(None); };
        let index = worker.costs.index().map_err(|error| error_code(&error))?;
        let rows = codex::try_session_rows(index, &path, worker.costs.areas())
            .map_err(|error| error_code(&CollectError::Index(error)))?;
        // A sessão avulsa não inicia a coleta global; reler tarifas só depois do índice.
        let mut pricing = worker.costs.pricing();
        pricing.reload_if_changed();
        let cost = session_cost::estimate(rows, &pricing);
        if cost.cost_usd.is_some_and(|value| !value.is_finite()) { return Err("session_cost_non_finite"); }
        serde_json::to_vec(&cost).map(Some).map_err(|_| "custo_sessao_json")
    }).await;
    match result {
        Ok(Ok(Some(body))) => response(request.headers(), body),
        Ok(Ok(None)) => session_not_found(request.headers(), "Codex rollout not found"),
        other => {
            let code = match other { Ok(Err(code)) => code, _ => "custo_sessao_join" };
            tracing::warn!(code);
            pass(&state, request, &forward).await
        }
    }
}

fn usage_filters(query: Option<&str>) -> report_uso::UsoFilters {
    let mut filters = report_uso::UsoFilters::default();
    for (key, value) in form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
        let values = match key.as_ref() {
            "conta" => &mut filters.conta, "projeto" => &mut filters.projeto,
            "modelo" => &mut filters.modelo, "plugin" => &mut filters.plugin,
            _ => continue,
        };
        if !value.is_empty() { values.push(value.into_owned()); }
    }
    filters.foco = crate::auth::query_param(query, "foco").filter(|f| !f.is_empty());
    filters
}

pub async fn usage(State(state): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>, request: Request) -> Response {
    let (forward, owner) = gate(&state, peer, &request);
    if !owner || request.method() != Method::GET { return pass(&state, request, &forward).await; }
    let Some(fresh) = fresh(crate::auth::query_param(request.uri().query(), "fresco")) else {
        return pass(&state, request, &forward).await;
    };
    let period = crate::auth::query_param(request.uri().query(), "period").filter(|p| {
        p == "all" || report_costs::PERIODS.iter().any(|(key, _)| p == key)
    }).unwrap_or_else(|| "all".into());
    let filters = usage_filters(request.uri().query());
    let worker = state.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<Prepared, &'static str> {
        match worker.costs.prepare_blocking(fresh).map_err(|error| error_code(&error))? {
            Ready::Warming { read, total } => return Ok(Prepared::Warming(read, total)),
            Ready::Go => {}
        }
        let now = LocalTs(chrono::Utc::now().timestamp_micros());
        let repo = worker.costs.repo().filter(|p| !p.as_os_str().is_empty()).ok_or("uso_escopos")?;
        let labels = worker.costs.labels_key();
        let data_version = worker.costs.data_version();
        let origins = worker.skill_origins(&repo);
        let (origins_generation, origins_map) = origins.recent();
        let since = report_costs::PERIODS.iter().find(|(p, _)| *p == period)
            .map(|(_, n)| LocalTs(now.0 - (n - 1) * 86_400_000_000).day());
        // A leitura usa Pricing: adquirir a tarifa antes dela travaria o próprio worker.
        let (usage, tokens) = worker.costs.read_usage(since.as_deref()).map_err(|error| error_code(&error))?;
        if worker.costs.repo().as_ref() != Some(&repo) { return Err("uso_escopos"); }
        let pricing = worker.costs.pricing();
        let filter_key = serde_json::to_string(&serde_json::json!([
            ["conta", filters.conta], ["foco", filters.foco], ["modelo", filters.modelo],
            ["plugin", filters.plugin], ["projeto", filters.projeto],
        ])).map_err(|_| "uso_filtros")?;
        let key = CacheKey {
            data_version, pricing_generation: pricing.generation(), area_signature: worker.costs.areas().signature().into(), labels: labels.clone(),
            route: vec!["uso".into(), period.clone(), now.day(),
                serde_json::to_string(&repo).map_err(|_| "uso_repo")?, origins_generation.to_string(), filter_key],
        };
        let report = match worker.reports.get::<report_uso::UsoReport>(&key) {
            Some(report) => report,
            None => {
                let report = Arc::new(report_uso::build(&usage, &tokens, &period, now, &filters, Some(&origins_map), &pricing, &|key| {
                    labels.iter().find(|(name, _)| name == key).map(|(_, label)| label.clone())
                }));
                worker.reports.insert(key, report.clone()); report
            }
        };
        drop(pricing);
        let mut report = (*report).clone(); report.usd_brl = worker.fx.usd_brl();
        serde_json::to_vec(&report).map(Prepared::Body).map_err(|_| "uso_json")
    }).await;
    match result {
        Ok(Ok(Prepared::Body(body))) => response(request.headers(), body),
        Ok(Ok(Prepared::Warming(read, total))) => {
            let mut response = warming(read, total); cors(request.headers(), response.headers_mut()); response
        }
        other => {
            let code = match other { Ok(Err(code)) => code, _ => "uso_join" };
            tracing::warn!(code); pass(&state, request, &forward).await
        }
    }
}
