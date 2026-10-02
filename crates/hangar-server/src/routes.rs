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
    let upgrade = req.headers().contains_key(header::UPGRADE);
    let resp = proxy::forward(&st.http, st.cfg.upstream, req, fwd).await;
    match resp.status() {
        StatusCode::UNAUTHORIZED => st.auth.record_fail(&fwd.client_ip),
        // WebSocket recusado antes do aceite chega como 403 e o Python já contou a falha.
        // Um 403 de origem também conta: só desliga o atalho, o lado seguro.
        StatusCode::FORBIDDEN if upgrade => st.auth.record_fail(&fwd.client_ip),
        StatusCode::TOO_MANY_REQUESTS => st.auth.mark_blocked(&fwd.client_ip),
        _ => {}
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

    /// Estado com um Python mínimo: `/ws` recusa com 403, `/limited` responde 429.
    async fn state_with_upstream() -> Arc<AppState> {
        use axum::routing::any;
        let app = Router::new()
            .route("/ws", any(|| async { StatusCode::FORBIDDEN }))
            .route("/limited", any(|| async { StatusCode::TOO_MANY_REQUESTS }));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let cfg = Config {
            listen: "127.0.0.1:0".parse().unwrap(),
            upstream,
            internal_secret: "s".into(),
            auth_token: "dono".into(),
            log_path: None,
            trusted: auth::TrustedHosts::parse("127.0.0.1"),
        };
        Arc::new(AppState { auth: Auth::new("dono"), http: proxy::client(), cfg })
    }

    fn request(path: &str, ip: &str, ws: bool, token: &str) -> Request {
        let mut b = Request::builder()
            .uri(path)
            .header("x-forwarded-for", ip)
            .header(header::AUTHORIZATION, format!("Bearer {token}"));
        if ws {
            b = b.header("connection", "Upgrade").header("upgrade", "websocket");
        }
        b.body(axum::body::Body::empty()).unwrap()
    }

    fn is_owner(st: &AppState, ip: &str) -> bool {
        let peer: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        gate(st, peer, &request("/x", ip, false, "dono")).1
    }

    #[tokio::test]
    async fn websocket_403_and_429_from_python_lock_the_shortcut() {
        let st = state_with_upstream().await;
        let peer: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let fwd = |ip: &str| gate(&st, peer, &request("/x", ip, false, "x")).0;

        // 8 palpites errados em WebSocket: o Python conta e fecha com 403.
        for _ in 0..8 {
            let r = pass(&st, request("/ws", "198.51.100.7", true, "errado"), &fwd("198.51.100.7")).await;
            assert_eq!(r.status(), StatusCode::FORBIDDEN);
        }
        assert!(!is_owner(&st, "198.51.100.7"), "o token certo não abre o atalho nessa origem");
        assert!(is_owner(&st, "198.51.100.8"), "outra origem segue normal");

        // Um 429 do Python satura a origem de uma vez.
        let r = pass(&st, request("/limited", "198.51.100.9", false, "errado"), &fwd("198.51.100.9")).await;
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(!is_owner(&st, "198.51.100.9"));

        // 403 sem Upgrade não é falha de token.
        let st2 = state_with_upstream().await;
        for _ in 0..8 {
            pass(&st2, request("/ws", "198.51.100.10", false, "x"), &fwd("198.51.100.10")).await;
        }
        assert!(is_owner(&st2, "198.51.100.10"));

        // Loopback é isento.
        let r = pass(&st, request("/limited", "127.0.0.1", false, "x"), &fwd("127.0.0.1")).await;
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(is_owner(&st, "127.0.0.1"));
    }

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
