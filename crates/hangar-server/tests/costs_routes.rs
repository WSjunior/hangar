mod common;

use axum::{Router, extract::{Request, State}, http::StatusCode, response::{IntoResponse, Response}, routing::get};
use common::costs::fixtures_copy;
use hangar_server::{auth::TrustedHosts, config::Config, costs::collect::{Collector, HttpScopes, Scopes, ClaudeScope, CodexScope, PiScope, KimiScope}, costs::fx::Fx, routes::{AppState, router}};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const OWNER: &str = "dono-de-teste";
const SECRET: &str = "interno-de-teste";

struct Upstream {
    scopes: Mutex<Option<Scopes>>,
    hits: Mutex<Vec<(String, String)>>,
}

async fn scopes(State(up): State<Arc<Upstream>>, request: Request) -> Response {
    up.hits.lock().unwrap().push((request.method().to_string(), request.uri().to_string()));
    assert_eq!(request.headers()["x-hangar-internal"], SECRET);
    match up.scopes.lock().unwrap().as_ref() {
        Some(scopes) => ([("content-type", "application/json")], serde_json::to_vec(scopes).unwrap()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn python(State(up): State<Arc<Upstream>>, request: Request) -> Response {
    up.hits.lock().unwrap().push((request.method().to_string(), request.uri().to_string()));
    if request.headers().get("authorization").is_none_or(|token| token != &format!("Bearer {OWNER}")) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if request.uri().query().is_some_and(|query| query.contains("fresco=inválido") || query.contains("fresco=invalid")) {
        return (StatusCode::UNPROCESSABLE_ENTITY, "bool inválido").into_response();
    }
    (StatusCode::OK, [("content-type", "application/json")], json!({"from_python":true}).to_string()).into_response()
}

struct Harness {
    _dir: tempfile::TempDir,
    base: PathBuf,
    address: SocketAddr,
    upstream: Arc<Upstream>,
    collector: Arc<Collector>,
    state: Arc<AppState>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for Harness {
    fn drop(&mut self) { for task in &self.tasks { task.abort(); } }
}

impl Harness {
    async fn new(has_scopes: bool, bad_disk: bool) -> Self {
        Self::with_fx(has_scopes, bad_disk, Arc::new(Fx::with_fetch(|| Some(5.25)))).await
    }

    async fn with_fx(has_scopes: bool, bad_disk: bool, fx: Arc<Fx>) -> Self {
        let (dir, base) = fixtures_copy();
        let scopes = Scopes {
            claude: vec![ClaudeScope { root: base.join("claude/projects"), account: "anthropic:u-fixture".into(), label: "fixture@exemplo".into() }],
            codex: vec![CodexScope { home: base.join("codex"), account: format!("codex:{}", base.join("codex").display()), label: "Codex · default".into() }],
            pi: vec![PiScope { root: base.join("pi"), source: "pi".into() }],
            kimi: Some(KimiScope { root: base.join("kimi/sessions"), index: base.join("kimi/session_index.jsonl") }),
            repo: base.clone(),
        };
        let upstream = Arc::new(Upstream { scopes: Mutex::new(has_scopes.then_some(scopes)), hits: Mutex::new(Vec::new()) });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let up_address = listener.local_addr().unwrap();
        let app = Router::new().route("/internal/costs/scopes", get(self::scopes)).fallback(python).with_state(upstream.clone());
        let up_task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let index = if bad_disk {
            let blocked = dir.path().join("blocked");
            std::fs::create_dir(&blocked).unwrap();
            #[cfg(unix)] {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o500)).unwrap();
                // Root ignora o chmod; um pai que é arquivo força o mesmo erro real de disco.
                if std::fs::write(blocked.join("probe"), "probe").is_ok() {
                    std::fs::remove_file(blocked.join("probe")).unwrap();
                    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o700)).unwrap();
                    let file = dir.path().join("blocked-file"); std::fs::write(&file, "arquivo").unwrap();
                    file.join("idx")
                } else { blocked.join("idx") }
            }
            #[cfg(not(unix))] {
                let file = dir.path().join("blocked-file"); std::fs::write(&file, "arquivo").unwrap(); file.join("idx")
            }
        } else { dir.path().join("idx") };
        let collector = Arc::new(Collector::new(index, base.join("pricing"), dir.path().join("areas.json"), Arc::new(HttpScopes::new(up_address, SECRET.into()))));
        let cfg = Config { listen: "127.0.0.1:0".parse().unwrap(), upstream: up_address,
            internal_secret: SECRET.into(), auth_token: OWNER.into(), log_path: None, trusted: TrustedHosts::parse("127.0.0.1") };
        let state = Arc::new(AppState::with_parts(cfg, hangar_server::terminal_control::TerminalPool::new(), collector.clone(), fx));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = router(state.clone());
        let server = tokio::spawn(async move { axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap() });
        Self { _dir: dir, base, address, upstream, collector, state, tasks: vec![up_task, server] }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        reqwest::Client::new().request(method, format!("http://{}{path}", self.address)).bearer_auth(OWNER)
    }

    async fn ready(&self) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let response = self.request(reqwest::Method::GET, "/api/costs").send().await.unwrap();
            assert!(Instant::now() < deadline, "coleta não concluiu");
            if response.status() == StatusCode::OK { return serde_json::from_slice(&response.bytes().await.unwrap()).unwrap(); }
            assert_eq!(response.status(), StatusCode::ACCEPTED);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn forwarded(&self, method: &str, path: &str) -> bool {
        self.upstream.hits.lock().unwrap().iter().any(|hit| hit == &(method.into(), path.into()))
    }
}

#[tokio::test]
async fn cold_index_returns_warming_then_owner_report_with_fx() {
    let h = Harness::new(true, false).await;
    let response = h.request(reqwest::Method::GET, "/api/costs").send().await.unwrap();
    assert_eq!(response.status(), 202);
    let warming: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(warming["aquecendo"], true);
    assert!(warming["lidos"].is_u64()); assert!(warming["total"].is_u64());
    let report = h.ready().await;
    assert_eq!(report["applied"], json!({"period":"all"}));
    assert_eq!(report["usd_brl"], 5.25);
    assert_eq!(report["totals"]["sessions"], 11);
    assert!(!h.forwarded("GET", "/api/costs"));
}

#[tokio::test]
async fn unknown_period_is_applied_as_all() {
    let h = Harness::new(true, false).await;
    h.ready().await;
    let response = h.request(reqwest::Method::GET, "/api/costs?period=nada").send().await.unwrap();
    assert_eq!(response.status(), 200);
    let report: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(report["applied"]["period"], "all");
}

#[tokio::test]
async fn missing_or_guest_token_goes_to_python_without_starting_scopes() {
    let h = Harness::new(true, false).await;
    let response = reqwest::Client::new().get(format!("http://{}/api/costs", h.address)).send().await.unwrap();
    assert_eq!(response.status(), 401);
    assert!(h.forwarded("GET", "/api/costs"));
    assert!(!h.forwarded("GET", "/internal/costs/scopes"));
    let response = reqwest::Client::new().get(format!("http://{}/api/costs", h.address)).bearer_auth("convidado").send().await.unwrap();
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn missing_internal_scopes_falls_back_instead_of_returning_empty_report() {
    let h = Harness::new(false, false).await;
    assert_eq!(h.request(reqwest::Method::GET, "/api/costs").send().await.unwrap().status(), 202);
    let report = h.ready().await;
    assert_eq!(report, json!({"from_python":true}));
    assert!(h.forwarded("GET", "/api/costs"));
    assert!(h.forwarded("GET", "/internal/costs/scopes"));
}

#[tokio::test]
async fn unwritable_index_falls_back_instead_of_using_memory() {
    let h = Harness::new(true, true).await;
    assert_eq!(h.request(reqwest::Method::GET, "/api/costs").send().await.unwrap().status(), 202);
    assert_eq!(h.ready().await, json!({"from_python":true}));
    assert!(h.forwarded("GET", "/api/costs"));
}

#[tokio::test]
async fn cotacao_get_only_returns_fx_and_post_is_forwarded() {
    let h = Harness::new(true, false).await;
    let response = h.request(reqwest::Method::GET, "/api/cotacao").send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap(), json!({"usd_brl":5.25}));
    assert!(!h.forwarded("GET", "/internal/costs/scopes"));
    let response = h.request(reqwest::Method::POST, "/api/cotacao").send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert!(h.forwarded("POST", "/api/cotacao"));
    let response = h.request(reqwest::Method::HEAD, "/api/cotacao").send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert!(h.forwarded("HEAD", "/api/cotacao"));
}

#[tokio::test]
async fn owner_report_uses_shared_gzip_and_cors_contract() {
    use std::io::Read;
    let h = Harness::new(true, false).await;
    h.ready().await;
    let response = h.request(reqwest::Method::GET, "/api/costs").header("accept-encoding", "gzip").header("origin", "https://teste.exemplo").send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-encoding"], "gzip");
    assert_eq!(response.headers()["access-control-allow-origin"], "*");
    assert_eq!(response.headers()["access-control-expose-headers"], "ETag");
    let bytes = response.bytes().await.unwrap();
    let mut body = Vec::new();
    flate2::read::GzDecoder::new(bytes.as_ref()).read_to_end(&mut body).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["applied"]["period"], "all");
}

#[tokio::test]
async fn fresh_accepts_fastapi_bool_spellings_and_invalid_value_is_forwarded() {
    let h = Harness::new(true, false).await;
    h.ready().await;
    for value in ["0", "1", "true", "FALSE", "on", "Off", "yes", "NO", "t", "Y", "F", "n"] {
        let response = h.request(reqwest::Method::GET, &format!("/api/costs?fresco={value}")).send().await.unwrap();
        assert_eq!(response.status(), 200, "{value}");
    }
    let response = h.request(reqwest::Method::GET, "/api/costs?fresco=invalid").send().await.unwrap();
    assert_eq!(response.status(), 422);
    assert!(h.forwarded("GET", "/api/costs?fresco=invalid"));
}

#[tokio::test]
async fn cached_report_is_invalidated_by_single_session_index_write() {
    use std::io::Write;
    let h = Harness::new(true, false).await;
    let before = h.ready().await;
    let version = h.collector.data_version();
    let rollout = hangar_server::costs::collect::list_files(&h.base.join("codex/sessions"), |name| name.starts_with("rollout-") && name.ends_with(".jsonl"))[0].clone();
    let mut file = std::fs::OpenOptions::new().append(true).open(&rollout).unwrap();
    writeln!(file, "{}", json!({"type":"event_msg", "timestamp":"2026-10-03T12:00:00-03:00", "payload": {
        "type":"token_count", "info":{"total_token_usage":{"input_tokens":1_000_000, "cached_input_tokens":0, "output_tokens":3000}}
    }})).unwrap();
    let collector = h.collector.clone();
    tokio::task::spawn_blocking(move || {
        assert!(hangar_server::costs::codex::session_rows(collector.index().unwrap(), &rollout, collector.areas()).is_some());
    }).await.unwrap();
    assert!(h.collector.data_version() > version);
    let after = h.ready().await;
    assert!(after["totals"]["input"].as_i64().unwrap() > before["totals"]["input"].as_i64().unwrap());
    assert!(!h.forwarded("GET", "/api/costs"));
    assert!(h.state.costs.data_version() > version);
}

#[tokio::test]
async fn quote_refresh_is_applied_above_the_cached_report() {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    let clock = Arc::new(AtomicU64::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let fetch_calls = calls.clone(); let fetch_clock = clock.clone();
    let fx = Arc::new(Fx::with_fetch_and_clock(move || {
        Some(if fetch_calls.fetch_add(1, Ordering::SeqCst) == 0 { 5.25 } else { 5.5 })
    }, move || Duration::from_secs(fetch_clock.load(Ordering::SeqCst))));
    let h = Harness::with_fx(true, false, fx).await;
    let before = h.ready().await;
    let version = h.collector.data_version();
    clock.store(3600, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let after = h.ready().await;
        if after["usd_brl"] == 5.5 {
            assert_eq!(after["totals"], before["totals"]);
            assert_eq!(h.collector.data_version(), version);
            break;
        }
        assert!(Instant::now() < deadline);
        tokio::task::yield_now().await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn failed_quote_returns_null_and_panicking_worker_is_forwarded() {
    let h = Harness::with_fx(true, false, Arc::new(Fx::with_fetch(|| None))).await;
    let report = h.ready().await;
    assert!(report["usd_brl"].is_null());
    let h = Harness::with_fx(true, false, Arc::new(Fx::with_fetch(|| panic!("falha sintética")))).await;
    let response = h.request(reqwest::Method::GET, "/api/cotacao").send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap(), json!({"from_python":true}));
    assert!(h.forwarded("GET", "/api/cotacao"));
}

#[tokio::test]
async fn partial_commit_reader_panic_cannot_reuse_a_cached_success() {
    use common::costs::append;
    use hangar_server::costs::{CacheKey, collect::{CollectError, Ready}, index::IndexError, report_costs::CostReport};
    let h = Harness::new(true, false).await;
    let before = h.ready().await;
    let key = CacheKey { data_version: h.collector.data_version(), pricing_generation: h.collector.pricing().generation(),
        area_signature: h.collector.areas().signature().into(), labels: h.collector.labels_key(),
        route: vec!["costs".into(), "all".into(), hangar_server::costs::py::LocalTs(chrono::Utc::now().timestamp_micros()).day()] };
    assert!(h.state.reports.get::<CostReport>(&key).is_some());
    let rollout = hangar_server::costs::collect::list_files(&h.base.join("codex/sessions"), |name| name.starts_with("rollout-") && name.ends_with(".jsonl"))[0].clone();
    append(&rollout, b"{}\n");
    let wire = h.base.join("kimi/sessions/wd_x/session_k1/agents/main/wire.jsonl");
    let saved = std::fs::read(&wire).unwrap();
    append(&wire, b"{\"type\":\"usage.record\",\"time\":1e308,\"model\":\"apikey/k3\",\"usage\":{\"inputOther\":1}}\n");
    let collector = h.collector.clone();
    let failure = tokio::task::spawn_blocking(move || collector.prepare_blocking(true)).await.unwrap();
    assert!(matches!(failure, Err(CollectError::Index(IndexError::ReaderPanic))), "{failure:?}");
    assert!(h.collector.data_version() > key.data_version);
    let mut changed = key.clone(); changed.data_version = h.collector.data_version();
    assert!(h.state.reports.get::<CostReport>(&changed).is_none());
    assert_eq!(h.ready().await, json!({"from_python":true}));
    assert!(h.forwarded("GET", "/api/costs"));
    std::fs::write(&wire, saved).unwrap();
    let collector = h.collector.clone();
    assert!(matches!(tokio::task::spawn_blocking(move || collector.prepare_blocking(true)).await.unwrap().unwrap(), Ready::Go));
    let recovered = h.ready().await;
    common::costs::assert_close(&recovered["totals"], &before["totals"], "totals");
    assert!(h.state.reports.get::<CostReport>(&key).is_none());
}

#[tokio::test]
async fn labels_and_pricing_changes_invalidate_the_served_report() {
    let h = Harness::new(true, false).await;
    let before = h.ready().await;
    h.upstream.scopes.lock().unwrap().as_mut().unwrap().claude[0].label = "Conta renomeada".into();
    let collector = h.collector.clone();
    tokio::task::spawn_blocking(move || collector.prepare_blocking(true)).await.unwrap().unwrap();
    let relabeled = h.ready().await;
    assert!(relabeled["by_provider"].as_array().unwrap().iter().any(|value| value["label"] == "Conta renomeada"));
    let model = before["rates"][0]["model"].as_str().unwrap();
    let mut overrides = serde_json::Map::new();
    overrides.insert(model.into(), json!({"input":100, "output":100, "cache_write":100, "cache_read":100, "provider":"anthropic"}));
    std::fs::write(h.base.join("pricing/overrides.json"), Value::Object(overrides).to_string()).unwrap();
    let repriced = h.ready().await;
    assert_ne!(repriced["totals"]["cost"], before["totals"]["cost"]);
    assert!(repriced["rates"].as_array().unwrap().iter().any(|value| value["model"] == model && value["origin"] == "override"));
}
