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
    // A lista de pastas do Arquivo pode levar dezenas de segundos na primeira leitura.
    let resp = tokio::time::timeout(Duration::from_secs(60), st.http.request(req))
        .await
        .map_err(|_| "prazo")?
        .map_err(|e| {
            tracing::warn!(error = %e, "worktrees: contexto sem conexão com o Python");
            "conexao"
        })?;
    if !resp.status().is_success() {
        tracing::warn!(status = %resp.status(), "worktrees: contexto recusado pelo Python");
        return Err("status");
    }
    // O teto vale durante a leitura: corpo grande não chega a ser guardado inteiro.
    let body = http_body_util::Limited::new(resp.into_body(), MAX_CONTEXT);
    let bytes = tokio::time::timeout(Duration::from_secs(60), body.collect())
        .await
        .map_err(|_| "prazo")?
        .map_err(|_| "tamanho")?
        .to_bytes();
    // O erro do serde diz linha e coluna, nunca o conteúdo.
    serde_json::from_slice(&bytes).map_err(|e| {
        tracing::warn!(error = %e, "worktrees: contexto do Python fora do formato");
        "json"
    })
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
    // O diário só leva frase fixa; o motivo separa as causas.
    let reason = match motivo {
        "prazo" => "contexto do Python sem resposta no prazo",
        "conexao" => "sem conexão com o Python",
        "status" => "Python recusou o contexto",
        "json" | "tamanho" => "contexto do Python fora do formato",
        "vagas cheias" => "vagas cheias",
        "pânico na leitura" => "pânico na leitura",
        _ => "lista de worktrees indisponível",
    };
    st.diag.report("rust.worktrees_failed", "", code, reason);
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
    // Sem vaga de leitura: quem limita os `git` da lista é o teto global do núcleo, e pedido
    // que espera por ele não vira 503 de "ocupado" como no Python, que também esperava.
    let repo = params.get("repo").cloned();
    let done = tokio::task::spawn_blocking(move || -> hangar_workspace::Result<Value> {
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
        Err(e) => {
            tracing::error!(panic = e.is_panic(), "worktrees: leitura não terminou");
            refuse(&st, &route, hangar_workspace::UNAVAILABLE, "pânico na leitura")
        }
    }
}
