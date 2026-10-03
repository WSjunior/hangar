use std::{io::Write, sync::{Arc, Mutex}};
use axum::{body::{Body, to_bytes}, extract::{ConnectInfo, State}, http::{Request, StatusCode}};
use hangar_server::{auth::TrustedHosts, config::Config, routes::AppState,
    terminal_control::{Limits, TerminalPool}, terminal_routes::terminal};

#[derive(Clone)]
struct Buffer(Arc<Mutex<Vec<u8>>>);
impl Write for Buffer {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

#[tokio::test]
async fn terminal_error_statuses_preserve_static_cause_without_private_data() {
    let output = Arc::new(Mutex::new(Vec::new()));
    let capture = output.clone();
    let subscriber = tracing_subscriber::fmt().without_time().with_ansi(false)
        .with_writer(move || Buffer(capture.clone())).finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();
    let cfg = Config { listen: "127.0.0.1:0".parse().unwrap(), upstream: "127.0.0.1:1".parse().unwrap(),
        internal_secret: "private-secret".into(), auth_token: "owner".into(), log_path: None,
        trusted: TrustedHosts::parse("127.0.0.1") };
    let state = Arc::new(AppState::with_terminal_pool(cfg,
        TerminalPool::with_program("/does-not-exist/private-program", None, Limits::default())));
    let valid = serde_json::json!({"op":"capture", "consumer":"private-consumer", "name":"fixture",
        "provider":"claude", "binding":"private-binding", "target":"%8", "started":1.0,
        "lines":200, "colors":false, "join":false});
    let mut invalid_target = valid.clone();
    invalid_target["target"] = serde_json::json!("private-pane");
    async {
        for (body, status, response) in [
            ("private-not-json".to_string(), StatusCode::BAD_REQUEST, "invalid terminal request"),
            (invalid_target.to_string(), StatusCode::BAD_REQUEST, "invalid terminal request"),
            (valid.to_string(), StatusCode::SERVICE_UNAVAILABLE, "terminal observer unavailable"),
        ] {
            let request = Request::builder().header("x-hangar-internal", "private-secret").body(Body::from(body)).unwrap();
            let result = terminal(State(state.clone()), ConnectInfo("127.0.0.1:12345".parse().unwrap()), request).await;
            assert_eq!(result.status(), status);
            assert_eq!(to_bytes(result.into_body(), 1024).await.unwrap().as_ref(), response.as_bytes());
        }
    }.await;
    let log = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    for code in ["invalid terminal request", "invalid terminal target", "cannot start terminal observer", "NotFound"] {
        assert!(log.contains(code), "missing safe cause {code}: {log}");
    }
    assert!(!log.contains("private-"), "private request data reached diagnostics");
}
