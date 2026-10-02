//! Rotas do hangar-server. As de conversa entram na Task 12; o resto é repasse.
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use tokio::net::TcpListener;

use crate::auth::{self, Auth};
use crate::config::Config;
use crate::proxy::{self, Forward, HttpClient};

pub struct AppState {
    pub cfg: Config,
    pub auth: Auth,
    pub http: HttpClient,
}

pub async fn serve(listener: TcpListener, cfg: Config) -> std::io::Result<()> {
    let state = Arc::new(AppState { auth: Auth::new(&cfg.auth_token), http: proxy::client(), cfg });
    axum::serve(listener, router(state).into_make_service_with_connect_info::<SocketAddr>()).await
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/__hangar_server/health", get(health))
        .fallback(pass_any)
        .with_state(state)
}

async fn health(headers: HeaderMap) -> Response {
    let body = format!(
        "{{\"ok\":true,\"version\":\"{}\",\"protocol\":{}}}",
        env!("CARGO_PKG_VERSION"),
        crate::INTERNAL_PROTOCOL
    );
    let mut resp = ([(header::CONTENT_TYPE, "application/json")], body).into_response();
    cors(&headers, resp.headers_mut());
    resp
}

/// Quem pede e se é o dono, resolvidos uma vez por pedido.
#[allow(dead_code)] // Usado pelos handlers de conversa (Task 12).
pub(crate) fn gate(st: &AppState, peer: SocketAddr, req: &Request) -> (Forward, bool) {
    let (client_ip, https) = st.cfg.trusted.resolve(peer.ip(), req.headers());
    let token = auth::presented_token(req.headers(), req.uri().query(), req.method(), https);
    let owner = st.auth.is_owner(&client_ip, token.as_deref());
    (Forward { client_ip, https }, owner)
}

pub(crate) async fn pass(st: &AppState, req: Request, fwd: &Forward) -> Response {
    let resp = proxy::forward(&st.http, st.cfg.upstream, req, fwd).await;
    if resp.status() == StatusCode::UNAUTHORIZED {
        st.auth.record_fail(&fwd.client_ip);
    }
    resp
}

async fn pass_any(
    State(st): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
) -> Response {
    let (client_ip, https) = st.cfg.trusted.resolve(peer.ip(), req.headers());
    pass(&st, req, &Forward { client_ip, https }).await
}

/// CORS das respostas do próprio Rust, como o CORSMiddleware do Python: `*`, sem credenciais,
/// ETag legível pelo JS. Preflight não chega aqui: vai sem token e é repassado.
pub(crate) fn cors(req: &HeaderMap, resp: &mut HeaderMap) {
    if req.contains_key(header::ORIGIN) {
        resp.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
        resp.insert(header::ACCESS_CONTROL_EXPOSE_HEADERS, HeaderValue::from_static("ETag"));
    }
}

/// Gzip só quando o cliente pede e o corpo passa de 1 KB. Nunca chamado para text/event-stream.
#[allow(dead_code)] // Usado pelos handlers de conversa (Task 12).
pub(crate) fn maybe_gzip(req: &HeaderMap, resp: &mut HeaderMap, body: Vec<u8>) -> Vec<u8> {
    let wants = req
        .get(header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("gzip"));
    if !wants || body.len() < 1024 {
        return body;
    }
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(5));
    std::io::Write::write_all(&mut enc, &body).expect("escrita em memória");
    resp.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    resp.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    enc.finish().expect("escrita em memória")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn cors_only_with_origin() {
        let mut resp = HeaderMap::new();
        cors(&HeaderMap::new(), &mut resp);
        assert!(resp.is_empty());
        let mut req = HeaderMap::new();
        req.insert(header::ORIGIN, HeaderValue::from_static("http://outra"));
        cors(&req, &mut resp);
        assert_eq!(resp[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        assert_eq!(resp[header::ACCESS_CONTROL_EXPOSE_HEADERS], "ETag");
    }

    #[test]
    fn gzip_only_when_asked_and_large() {
        let big = vec![b'a'; 2048];
        let mut resp = HeaderMap::new();
        assert_eq!(maybe_gzip(&HeaderMap::new(), &mut resp, big.clone()), big);
        let mut req = HeaderMap::new();
        req.insert(header::ACCEPT_ENCODING, HeaderValue::from_static("gzip, deflate"));
        assert_eq!(maybe_gzip(&req, &mut resp, b"curto".to_vec()), b"curto");
        assert!(resp.get(header::CONTENT_ENCODING).is_none());
        let packed = maybe_gzip(&req, &mut resp, big.clone());
        assert_eq!(resp[header::CONTENT_ENCODING], "gzip");
        assert_eq!(resp[header::VARY], "Accept-Encoding");
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(&packed[..]).read_to_end(&mut out).unwrap();
        assert_eq!(out, big);
    }
}
