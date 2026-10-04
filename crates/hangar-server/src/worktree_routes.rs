//! `GET /api/worktrees` e `/api/worktrees/detail`: do Rust, com o Python só dando os metadados.
use crate::{proxy::Forward, routes::AppState};
use axum::{
    body::Body,
    extract::Request,
    http::{Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use hangar_workspace::worktrees::{self, Session};
use http_body_util::BodyExt;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc, time::Duration};

const CONTEXT: &str = "worktrees_context";
const MAX_CONTEXT: usize = 16 * 1024 * 1024;

#[derive(Deserialize)]
struct Context {
    roots: Vec<String>,
    cwds: Vec<String>,
    sessions: Vec<Session>,
    project_bases: Vec<String>,
}

pub fn matches(method: &Method, path: &str) -> bool {
    *method == Method::GET && matches!(path, "/api/worktrees" | "/api/worktrees/detail")
}

fn response(value: Value, status: u16) -> Response {
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        [(header::CONTENT_TYPE, "application/json")],
        value.to_string(),
    )
        .into_response()
}

/// Como o `bool` do FastAPI; valor fora disso fica com o 422 dele.
fn flag(value: Option<&String>) -> Option<bool> {
    match value.map(|s| s.to_lowercase()).as_deref() {
        None => Some(true),
        Some("1" | "true" | "on" | "yes" | "t" | "y") => Some(true),
        Some("0" | "false" | "off" | "no" | "f" | "n") => Some(false),
        _ => None,
    }
}

async fn context(st: &AppState) -> Result<Context, &'static str> {
    let req = axum::http::Request::get(format!(
        "http://{}/internal/worktrees/context",
        st.cfg.upstream
    ))
    .header("x-hangar-internal", &st.cfg.internal_secret)
    .body(Body::empty())
    .map_err(|_| "pedido")?;
    // A lista de pastas do Arquivo pode levar alguns segundos na primeira leitura.
    let resp = tokio::time::timeout(Duration::from_secs(20), st.http.request(req))
        .await
        .map_err(|_| "prazo")?
        .map_err(|_| "conexao")?;
    if !resp.status().is_success() {
        return Err("status");
    }
    let bytes = tokio::time::timeout(Duration::from_secs(20), resp.into_body().collect())
        .await
        .map_err(|_| "prazo")?
        .map_err(|_| "conexao")?
        .to_bytes();
    if bytes.len() > MAX_CONTEXT {
        return Err("tamanho");
    }
    serde_json::from_slice(&bytes).map_err(|_| "json")
}

/// Não rodou: 503 com código e motivo, como as outras rotas do Rust; o Python nunca atende.
fn refuse(st: &AppState, route: &str, code: &'static str, motivo: &str) -> Response {
    let msg = if code == hangar_workspace::BUSY {
        "Git ocupado, tente em instantes.".to_owned()
    } else {
        format!("Lista de worktrees indisponível: {motivo}.")
    };
    if crate::warn_limit::allow(None, &format!("{code}:{motivo}")) {
        tracing::warn!(route = %route, code, motivo = %motivo, "worktrees recusado");
    }
    st.diag.report("rust.worktrees_failed", "", code, "lista de worktrees indisponível");
    let mut resp = response(
        json!({"ok":false,"error_code":code,"message":msg,
            "detail":{"code":code,"params":{"motivo":motivo},"msg":msg}}),
        503,
    );
    if code == hangar_workspace::BUSY {
        resp.headers_mut()
            .insert(header::RETRY_AFTER, header::HeaderValue::from_static("2"));
    }
    resp
}

pub async fn public(st: Arc<AppState>, req: Request, forward: Forward) -> Response {
    let route = req.uri().path().to_owned();
    let params = form_urlencoded::parse(req.uri().query().unwrap_or("").as_bytes())
        .into_owned()
        .collect::<HashMap<String, String>>();
    let detail = route == "/api/worktrees/detail";
    let measure = flag(params.get("sizes"));
    let path = params.get("path").cloned();
    // Pedido malformado: o 422 do FastAPI, no formato que os clientes já leem.
    if (detail && path.is_none()) || (!detail && measure.is_none()) {
        return crate::routes::pass(&st, req, &forward).await;
    }
    let ctx = match context(&st).await {
        Ok(ctx) => ctx,
        Err(motivo) => return refuse(&st, &route, CONTEXT, motivo),
    };
    let Ok(permit) = st.workspace_read_slots.clone().try_acquire_owned() else {
        return refuse(&st, &route, hangar_workspace::BUSY, "vagas cheias");
    };
    let repo = params.get("repo").cloned();
    let done = tokio::task::spawn_blocking(move || -> hangar_workspace::Result<Value> {
        let _permit = permit;
        if detail {
            let path = worktrees::allowed_worktree(&path.unwrap_or_default(), &ctx.roots)?;
            Ok(worktrees::status_of(
                &path,
                &ctx.sessions,
                true,
                &ctx.project_bases,
            ))
        } else {
            let repo = repo
                .map(|r| worktrees::allowed_repo(&r, &ctx.roots))
                .transpose()?;
            let repos = worktrees::list_all(
                &ctx.cwds,
                &ctx.sessions,
                Some(&ctx.roots),
                repo.as_deref(),
                measure.unwrap_or(true),
                &ctx.project_bases,
            );
            Ok(json!({"repos": repos}))
        }
    })
    .await;
    match done {
        Ok(Ok(value)) => response(value, 200),
        // Recusa de caminho: o mesmo `{"detail": ...}` do HTTPException do Python.
        Ok(Err(e)) => response(json!({"detail": e.detail}), e.status),
        Err(_) => refuse(&st, &route, hangar_workspace::UNAVAILABLE, "pânico na leitura"),
    }
}
