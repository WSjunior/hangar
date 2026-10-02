//! Python falso para os testes do hangar-server: rotas internas (info, side-events) e o resto
//! respondendo "from-python", com o que chegou anotado.
#![allow(dead_code)]

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::get;
use bytes::Bytes;
use futures_util::StreamExt;
use hangar_server::auth::TrustedHosts;
use hangar_server::config::Config;
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Notify, broadcast};

pub const OWNER: &str = "dono-token";
pub const SECRET: &str = "segredo-interno";

pub struct Fake {
    info: Mutex<Value>,
    side_tx: broadcast::Sender<String>,
    side_conns: AtomicUsize,
    side_apps: Mutex<Vec<String>>,
    info_calls: AtomicUsize,
    hits: Mutex<Vec<(String, HeaderMap)>>,
    pub release: Notify,
}

impl Fake {
    /// O que /internal/.../info devolve e o que a conexão interna manda como primeiro `info`.
    /// `Value::Null` = sessão inexistente (404).
    pub fn set_info(&self, v: Value) {
        *self.info.lock().unwrap() = v;
    }
    pub fn push_side(&self, event: &str, data: &str) {
        let _ = self.side_tx.send(format!("event: {event}\r\ndata: {data}\r\n\r\n"));
    }
    pub fn side_conns(&self) -> usize {
        self.side_conns.load(SeqCst)
    }
    pub fn side_apps(&self) -> Vec<String> {
        self.side_apps.lock().unwrap().clone()
    }
    pub fn info_calls(&self) -> usize {
        self.info_calls.load(SeqCst)
    }
    pub fn hits_to(&self, path: &str) -> usize {
        self.hits.lock().unwrap().iter().filter(|(p, _)| p.split('?').next() == Some(path)).count()
    }
    pub fn last_hit(&self) -> (String, HeaderMap) {
        self.hits.lock().unwrap().last().cloned().expect("algum pedido repassado")
    }
}

pub async fn spawn_fake() -> (Arc<Fake>, SocketAddr) {
    let (side_tx, _) = broadcast::channel(64);
    let fake = Arc::new(Fake {
        info: Mutex::new(Value::Null),
        side_tx,
        side_conns: AtomicUsize::new(0),
        side_apps: Mutex::default(),
        info_calls: AtomicUsize::new(0),
        hits: Mutex::default(),
        release: Notify::new(),
    });
    let app = Router::new()
        .route("/internal/sessions/{name}/info", get(fake_info))
        .route("/internal/sessions/{name}/side-events", get(fake_side))
        .fallback(fake_python)
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (fake, addr)
}

fn internal_ok(h: &HeaderMap) -> bool {
    h.get("x-hangar-internal").is_some_and(|v| v.as_bytes() == SECRET.as_bytes())
}

fn status(s: StatusCode) -> Response {
    Response::builder().status(s).body(Body::empty()).unwrap()
}

async fn fake_info(State(f): State<Arc<Fake>>, headers: HeaderMap) -> Response {
    f.info_calls.fetch_add(1, SeqCst);
    let info = f.info.lock().unwrap().clone();
    if !internal_ok(&headers) || info.is_null() {
        return status(StatusCode::NOT_FOUND);
    }
    Response::builder()
        .header("content-type", "application/json")
        .body(Body::from(info.to_string()))
        .unwrap()
}

async fn fake_side(
    State(f): State<Arc<Fake>>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let info = f.info.lock().unwrap().clone();
    if !internal_ok(&headers) || info.is_null() {
        return status(StatusCode::NOT_FOUND);
    }
    let rx = f.side_tx.subscribe();
    f.side_apps.lock().unwrap().push(q.get("app").cloned().unwrap_or_default());
    f.side_conns.fetch_add(1, SeqCst);
    let first = futures_util::stream::once(async move {
        Ok::<_, Infallible>(Bytes::from(format!("event: info\r\ndata: {info}\r\n\r\n")))
    });
    let rest = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.ok().map(|s| (Ok(Bytes::from(s)), rx))
    });
    Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from_stream(first.chain(rest)))
        .unwrap()
}

async fn fake_python(State(f): State<Arc<Fake>>, mut req: Request) -> Response {
    let full = req.uri().path_and_query().map(|p| p.to_string()).unwrap_or_default();
    f.hits.lock().unwrap().push((full, req.headers().clone()));
    let path = req.uri().path().to_owned();
    match path.as_str() {
        "/redirect" => Response::builder().status(302).header("location", "/outro").body(Body::empty()).unwrap(),
        "/probe" => status(StatusCode::UNAUTHORIZED),
        "/stream" => {
            let f2 = f.clone();
            let s = futures_util::stream::once(async { Ok::<_, Infallible>(Bytes::from_static(b"um")) })
                .chain(futures_util::stream::once(async move {
                    f2.release.notified().await;
                    Ok(Bytes::from_static(b"dois"))
                }));
            Response::new(Body::from_stream(s))
        }
        "/ws" => {
            let upgrade = hyper::upgrade::on(&mut req);
            tokio::spawn(async move {
                let Ok(up) = upgrade.await else { return };
                let mut io = TokioIo::new(up);
                let mut buf = [0u8; 64];
                while let Ok(n) = io.read(&mut buf).await {
                    if n == 0 || io.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
            Response::builder()
                .status(101)
                .header("connection", "upgrade")
                .header("upgrade", "eco")
                .body(Body::empty())
                .unwrap()
        }
        _ => Response::new(Body::from("from-python")),
    }
}

pub fn config(upstream: SocketAddr, trusted: &str) -> Config {
    Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream,
        internal_secret: SECRET.into(),
        auth_token: OWNER.into(),
        log_path: None,
        trusted: TrustedHosts::parse(trusted),
    }
}

pub async fn spawn_server(cfg: Config) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(hangar_server::routes::serve(listener, cfg));
    addr
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap()
}
