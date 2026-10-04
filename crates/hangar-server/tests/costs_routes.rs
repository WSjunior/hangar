mod common;

use axum::{Router, extract::{Path, Request, State}, http::StatusCode, response::{IntoResponse, Response}, routing::get};
use common::costs::fixtures_copy;
use hangar_server::{auth::TrustedHosts, config::Config, costs::collect::{Collector, HttpScopes, Scopes, ClaudeScope, CodexScope, PiScope, KimiScope}, costs::fx::Fx, routes::{AppState, router}};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const OWNER: &str = "dono-de-teste";
const SECRET: &str = "interno-de-teste";

fn session_error(message: &str) -> Value {
    json!({"detail": {"code": "erro_sessao_inexistente", "params": {}, "msg": message}})
}

fn session_rollout(path: &std::path::Path, model: &str, input: i64) {
    let lines = [
        json!({"type":"session_meta", "timestamp":"2026-10-01T12:00:00Z", "payload":{"id":"synthetic", "model_provider":"openai"}}),
        json!({"type":"turn_context", "timestamp":"2026-10-01T12:00:01Z", "payload":{"model":model, "turn_id":"turn"}}),
        json!({"type":"token_usage_record", "timestamp":"2026-10-01T12:00:02Z", "payload":{"thread_id":"synthetic", "usage":{"input_tokens":input}}}),
    ];
    std::fs::write(path, lines.iter().map(|line| format!("{line}\n")).collect::<String>()).unwrap();
}

struct Upstream {
    scopes: Mutex<Option<Scopes>>,
    infos: Mutex<indexmap::IndexMap<String, Value>>,
    hits: Mutex<Vec<(String, String)>>,
    info_failures: Mutex<std::collections::VecDeque<StatusCode>>,
    journal: Mutex<Vec<Value>>,
    recover_scopes: Mutex<Option<Scopes>>,
    recover_disk: Mutex<Option<PathBuf>>,
}

async fn session_info(State(up): State<Arc<Upstream>>, Path(name): Path<String>, request: Request) -> Response {
    up.hits.lock().unwrap().push((request.method().to_string(), request.uri().to_string()));
    assert_eq!(request.headers()["x-hangar-internal"], SECRET);
    if let Some(status) = up.info_failures.lock().unwrap().pop_front() { return status.into_response(); }
    match up.infos.lock().unwrap().get(&name) {
        Some(info) => ([("content-type", "application/json")], info.to_string()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn journal(State(up): State<Arc<Upstream>>, request: Request) -> Response {
    assert_eq!(request.headers()["x-hangar-internal"], SECRET);
    assert!(request.headers().get("authorization").is_none());
    let correlation = request.headers().get("x-hangar-req").cloned();
    let body = axum::body::to_bytes(request.into_body(), 1024).await.unwrap();
    let event: Value = serde_json::from_slice(&body).unwrap();
    if event["session"] == "broken" { assert_eq!(correlation.unwrap(), "request_15"); }
    if event["part"] == "costs" && event["attempt"] == 3
        && let Some(scopes) = up.recover_scopes.lock().unwrap().take() {
        *up.scopes.lock().unwrap() = Some(scopes);
    }
    if event["code"] == "no_disk" && event["attempt"] == 1
        && let Some(path) = up.recover_disk.lock().unwrap().take() {
        std::fs::remove_file(path).unwrap();
    }
    up.journal.lock().unwrap().push(event);
    StatusCode::NO_CONTENT.into_response()
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
        let upstream = Arc::new(Upstream { scopes: Mutex::new(has_scopes.then_some(scopes)), infos: Mutex::new(indexmap::IndexMap::new()), hits: Mutex::new(Vec::new()),
            info_failures: Mutex::new(Default::default()), journal: Mutex::new(Vec::new()), recover_scopes: Mutex::new(None), recover_disk: Mutex::new(None) });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let up_address = listener.local_addr().unwrap();
        let app = Router::new().route("/internal/costs/scopes", get(self::scopes))
            .route("/internal/rust-failure", axum::routing::post(journal))
            .route("/internal/sessions/{name}/info", get(session_info)).fallback(python).with_state(upstream.clone());
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
        let mut state = AppState::with_parts(cfg, hangar_server::terminal_control::TerminalPool::new(), collector.clone(), fx);
        state.origins_home = dir.path().join("home");
        for path in [state.origins_home.join(".claude/skills/minha-skill"), state.origins_home.join(".claude/plugins/cache/market/superpowers/1/skills/brainstorming")] {
            std::fs::create_dir_all(&path).unwrap(); std::fs::write(path.join("SKILL.md"), "# Skill sintética").unwrap();
        }
        let state = Arc::new(state);
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

    async fn usage(&self, path: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let response = self.request(reqwest::Method::GET, path).send().await.unwrap();
            assert!(Instant::now() < deadline, "uso não concluiu");
            if response.status() == StatusCode::OK { return serde_json::from_slice(&response.bytes().await.unwrap()).unwrap(); }
            assert_eq!(response.status(), StatusCode::ACCEPTED);
            tokio::task::yield_now().await;
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
async fn session_info_failure_retries_four_times_then_only_that_session_stays_python() {
    let h = Harness::new(false, false).await;
    let rollout = h.base.join("codex/sessions/2026/09/30/rollout-c1.jsonl");
    for name in ["broken", "healthy"] {
        h.upstream.infos.lock().unwrap().insert(name.into(), json!({"provider":"codex", "jsonl":rollout}));
    }
    h.upstream.info_failures.lock().unwrap().extend([StatusCode::SERVICE_UNAVAILABLE; 4]);
    let route = "/api/sessions/broken/cost";
    for _ in 0..2 {
        let response = h.request(reqwest::Method::GET, route).header("x-hangar-req", "request_15").send().await.unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap(), json!({"from_python":true}));
    }
    let hits = h.upstream.hits.lock().unwrap().clone();
    assert_eq!(hits.iter().filter(|(_, uri)| uri == "/internal/sessions/broken/info").count(), 4);
    assert_eq!(hits.iter().filter(|(_, uri)| uri == route).count(), 2);
    let events = h.upstream.journal.lock().unwrap().clone();
    assert_eq!(events.len(), 5);
    assert_eq!(events.iter().filter(|event| event["transferred"] == true).count(), 1);
    assert_eq!(events.iter().filter(|event| event["transferred"] == false).map(|event| event["attempt"].as_u64().unwrap()).collect::<Vec<_>>(), vec![1, 2, 3, 4]);
    assert!(events.iter().all(|event| event["part"] == "session_cost" && event["code"] == "info_unavailable" && event["session"] == "broken"));
    let response = h.request(reqwest::Method::GET, "/api/sessions/healthy/cost").send().await.unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap()["has_usage"], true);
    assert!(!h.forwarded("GET", "/api/sessions/healthy/cost"));
}

#[tokio::test]
async fn three_info_failures_then_success_do_not_transfer() {
    let h = Harness::new(false, false).await;
    h.upstream.infos.lock().unwrap().insert("session".into(), json!({"provider":"codex", "jsonl":h.base.join("codex/sessions/2026/09/30/rollout-c1.jsonl")}));
    h.upstream.info_failures.lock().unwrap().extend([StatusCode::SERVICE_UNAVAILABLE; 3]);
    let response = h.request(reqwest::Method::GET, "/api/sessions/session/cost").send().await.unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap()["has_usage"], true);
    assert_eq!(h.upstream.journal.lock().unwrap().len(), 3);
    assert!(!h.forwarded("GET", "/api/sessions/session/cost"));
}

#[tokio::test]
async fn scopes_recovery_really_scans_on_retry_and_succeeds_on_fourth() {
    let h = Harness::new(true, false).await;
    let saved = h.upstream.scopes.lock().unwrap().take().unwrap();
    h.request(reqwest::Method::GET, "/api/costs").send().await.unwrap();
    let collector = h.collector.clone();
    assert!(tokio::task::spawn_blocking(move || collector.prepare_blocking(true)).await.unwrap().is_err());
    *h.upstream.recover_scopes.lock().unwrap() = Some(saved);
    let response = h.request(reqwest::Method::GET, "/api/costs").send().await.unwrap();
    assert!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap()["totals"].is_object());
    let events = h.upstream.journal.lock().unwrap();
    assert_eq!(events.len(), 3);
    assert!(events.iter().all(|event| event["code"] == "no_scopes" && event["transferred"] == false));
    assert!(!h.forwarded("GET", "/api/costs"));
    assert!(h.upstream.hits.lock().unwrap().iter().filter(|(_, uri)| uri == "/internal/costs/scopes").count() >= 4);
}

#[tokio::test]
async fn disk_recovery_really_reopens_index_on_second_attempt() {
    let h = Harness::new(true, false).await;
    let index_path = h._dir.path().join("idx");
    std::fs::write(&index_path, "bloqueio sintético").unwrap();
    h.request(reqwest::Method::GET, "/api/costs").send().await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let collector = h.collector.clone();
        if tokio::task::spawn_blocking(move || collector.prepare_blocking(false)).await.unwrap().is_err() { break; }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    *h.upstream.recover_disk.lock().unwrap() = Some(index_path.clone());
    let response = h.request(reqwest::Method::GET, "/api/costs").send().await.unwrap();
    assert!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap()["totals"].is_object());
    assert!(index_path.is_dir());
    assert_eq!(h.upstream.journal.lock().unwrap().as_slice(), &[json!({"part":"costs", "code":"no_disk", "attempt":1, "transferred":false})]);
    assert!(!h.forwarded("GET", "/api/costs"));
}

#[tokio::test]
async fn non_finite_quote_and_reports_fall_back_instead_of_serializing_null() {
    let h = Harness::with_fx(true, false, Arc::new(Fx::with_fetch(|| Some(f64::INFINITY)))).await;
    for route in ["/api/cotacao", "/api/costs", "/api/uso"] {
        let response = loop {
            let response = h.request(reqwest::Method::GET, route).send().await.unwrap();
            if response.status() != StatusCode::ACCEPTED { break response; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap(), json!({"from_python":true}));
    }
    let events = h.upstream.journal.lock().unwrap();
    assert_eq!(events.len(), 15);
    assert!(events.iter().all(|event| event["code"] == "non_finite"));
}

#[tokio::test]
async fn cold_usage_index_returns_warming_before_owner_report() {
    let h = Harness::new(true, false).await;
    let response = h.request(reqwest::Method::GET, "/api/uso").send().await.unwrap();
    assert_eq!(response.status(), 202);
    let warming: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(warming["aquecendo"], true);
    assert!(warming["lidos"].is_u64()); assert!(warming["total"].is_u64());
    h.ready().await;
    let response = h.request(reqwest::Method::GET, "/api/uso").send().await.unwrap();
    assert_eq!(response.status(), 200);
    let report: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(report["applied"], json!({"period":"all"})); assert_eq!(report["usd_brl"], 5.25);
    assert!(report["by_skill"].is_array()); assert!(!h.forwarded("GET", "/api/uso"));
}

#[tokio::test]
async fn usage_repeated_filters_discard_empty_values_and_scalar_uses_last() {
    let h = Harness::new(true, false).await;
    h.ready().await;
    let path = "/api/uso?conta=a&conta=&projeto=/repo/a&modelo=x&modelo=y&plugin=&foco=old&foco=&period=7d&period=nada";
    let response = h.request(reqwest::Method::GET, path).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let report: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(report["conta"], json!(["a"])); assert_eq!(report["projeto"], json!(["/repo/a"]));
    assert_eq!(report["modelo"], json!(["x", "y"])); assert_eq!(report["plugin"], json!([]));
    assert!(report["foco"].is_null()); assert_eq!(report["applied"]["period"], "all");
    assert_eq!(report["totals"]["chamadas"], 0); assert!(!report["by_conta"].as_array().unwrap().is_empty());
    assert!(!h.forwarded("GET", path));
}

#[tokio::test]
async fn served_usage_matches_python_golden_for_all_and_each_filter() {
    let h = Harness::new(true, false).await;
    let golden: Value = serde_json::from_slice(&std::fs::read(common::costs::contract().join("golden/costs_reports.json")).unwrap()).unwrap();
    for (key, path) in [("all", "/api/uso"), ("conta", "/api/uso?conta=anthropic:u-fixture"),
        ("projeto", "/api/uso?projeto=/repo/a"), ("foco_skill", "/api/uso?foco=brainstorming"), ("foco_area", "/api/uso?foco=back")] {
        let got = h.usage(path).await;
        let mut want = golden["uso"][key].clone();
        for account in want["by_conta"].as_array_mut().unwrap() {
            if account["key"] == "codex:__BASE__/codex" { account["key"] = Value::String(format!("codex:{}", h.base.join("codex").display())); }
        }
        want["usd_brl"] = json!(5.25);
        common::costs::assert_close(&got, &want, key);
        assert!(!h.forwarded("GET", path));
    }
}

#[tokio::test]
async fn usage_auth_methods_and_invalid_bool_forward_before_reading_origins() {
    let h = Harness::new(true, false).await;
    for token in [None, Some("convidado")] {
        let request = reqwest::Client::new().get(format!("http://{}/api/uso", h.address));
        let request = if let Some(token) = token { request.bearer_auth(token) } else { request };
        assert_eq!(request.send().await.unwrap().status(), 401);
    }
    assert!(h.forwarded("GET", "/api/uso"));
    for method in [reqwest::Method::POST, reqwest::Method::HEAD, reqwest::Method::OPTIONS] {
        assert_eq!(h.request(method.clone(), "/api/uso").send().await.unwrap().status(), 200);
        assert!(h.forwarded(method.as_str(), "/api/uso"));
    }
    assert_eq!(h.request(reqwest::Method::GET, "/api/uso?fresco=invalid").send().await.unwrap().status(), 422);
    assert!(h.forwarded("GET", "/api/uso?fresco=invalid"));
    assert!(!h.forwarded("GET", "/internal/costs/scopes"));
    assert!(h.state.origins.lock().unwrap().is_empty());
}

#[tokio::test]
async fn usage_missing_scopes_or_unwritable_index_falls_back_to_python() {
    for (scopes, bad_disk) in [(false, false), (true, true)] {
        let h = Harness::new(scopes, bad_disk).await;
        assert_eq!(h.request(reqwest::Method::GET, "/api/uso").send().await.unwrap().status(), 202);
        assert_eq!(h.usage("/api/uso").await, json!({"from_python":true}));
        assert!(h.forwarded("GET", "/api/uso")); assert!(h.state.origins.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn usage_filter_cache_keeps_dimension_boundaries_and_value_order() {
    let h = Harness::new(true, false).await;
    for (query, expected) in [("conta=a%2Cb", json!(["a,b"])), ("conta=a&conta=b", json!(["a", "b"])),
        ("conta=b&conta=a", json!(["b", "a"])), ("conta=a&projeto=b", json!(["a"]))] {
        let report = h.usage(&format!("/api/uso?{query}")).await;
        assert_eq!(report["conta"], expected);
    }
    let first = h.usage("/api/uso?conta=a&projeto=b").await;
    let reordered = h.usage("/api/uso?projeto=b&conta=a").await;
    assert_eq!(first, reordered);
    assert_ne!(first["totals"], h.usage("/api/uso").await["totals"]);
}

#[tokio::test]
async fn usage_origin_change_invalidates_report_without_data_change() {
    use hangar_server::costs::origins::Origins;
    use std::sync::atomic::{AtomicU64, Ordering};
    let h = Harness::new(true, false).await;
    let clock = Arc::new(AtomicU64::new(1)); let c = clock.clone();
    h.state.origins.lock().unwrap().insert(h.base.clone(), Origins::with_clock(h.state.origins_home.clone(), h.base.clone(), Arc::new(move || c.load(Ordering::SeqCst))));
    let before = h.usage("/api/uso").await;
    assert_eq!(before["by_skill"].as_array().unwrap().iter().find(|b| b["key"] == "brainstorming").unwrap()["plugin"], "superpowers");
    let version = h.collector.data_version();
    let personal = h.state.origins_home.join(".claude/skills/brainstorming");
    std::fs::create_dir_all(&personal).unwrap(); std::fs::write(personal.join("SKILL.md"), "# Skill pessoal sintética").unwrap();
    clock.store(31_000_000_001, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let after = h.usage("/api/uso").await;
        if after["by_skill"].as_array().unwrap().iter().find(|b| b["key"] == "brainstorming").unwrap()["plugin"] == "@pessoal" {
            assert_eq!(after["totals"], before["totals"]); assert_eq!(h.collector.data_version(), version); break;
        }
        assert!(Instant::now() < deadline); tokio::task::yield_now().await;
    }
    let filtered = h.usage("/api/uso?plugin=%40pessoal").await;
    assert!(filtered["by_skill"].as_array().unwrap().iter().any(|b| b["key"] == "brainstorming"));
}

#[tokio::test]
async fn usage_origins_follow_changed_collector_repo_and_keep_cache_bound() {
    use hangar_server::costs::origins::Origins;
    let h = Harness::new(true, false).await;
    let before = h.usage("/api/uso?plugin=superpowers").await;
    assert!(!before["by_skill"].as_array().unwrap().is_empty());
    let original = h.base.clone();
    let next = h._dir.path().join("other-repo"); let skill = next.join("skills/brainstorming");
    std::fs::create_dir_all(&skill).unwrap(); std::fs::write(skill.join("SKILL.md"), "# Skill do repositório").unwrap();
    h.upstream.scopes.lock().unwrap().as_mut().unwrap().repo = next.clone();
    let after = h.usage("/api/uso?fresco=true&plugin=%40repo").await;
    assert_eq!(after["by_skill"][0]["key"], "brainstorming"); assert_eq!(after["by_skill"][0]["plugin"], "@repo");
    assert!(h.state.origins.lock().unwrap().contains_key(&original)); assert!(h.state.origins.lock().unwrap().contains_key(&next));
    assert!(h.usage("/api/uso?plugin=superpowers").await["by_skill"].as_array().unwrap().is_empty());
    for i in 0..9 {
        let repo = h._dir.path().join(format!("repo-{i}"));
        // Pré-carrega somente caches vazios e temporários, sem varredura de configuração real.
        h.state.origins.lock().unwrap().insert(repo.clone(), Origins::new(h.state.origins_home.clone(), repo.clone()));
        h.upstream.scopes.lock().unwrap().as_mut().unwrap().repo = repo;
        h.usage("/api/uso?fresco=true").await;
        assert!(h.state.origins.lock().unwrap().len() <= 8);
    }
}

#[tokio::test]
async fn usage_quote_is_above_cache_and_response_uses_gzip_and_cors() {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::io::Read;
    let clock = Arc::new(AtomicU64::new(0)); let calls = Arc::new(AtomicUsize::new(0));
    let fc = calls.clone(); let now = clock.clone();
    let fx = Arc::new(Fx::with_fetch_and_clock(move || Some(if fc.fetch_add(1, Ordering::SeqCst) == 0 { 5.25 } else { 5.5 }), move || Duration::from_secs(now.load(Ordering::SeqCst))));
    let h = Harness::with_fx(true, false, fx).await;
    let before = h.usage("/api/uso").await; let version = h.collector.data_version();
    clock.store(3600, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let after = h.usage("/api/uso").await;
        if after["usd_brl"] == 5.5 { assert_eq!(after["totals"], before["totals"]); assert_eq!(h.collector.data_version(), version); break; }
        assert!(Instant::now() < deadline); tokio::task::yield_now().await;
    }
    let response = h.request(reqwest::Method::GET, "/api/uso").header("accept-encoding", "gzip").header("origin", "https://teste.exemplo").send().await.unwrap();
    assert_eq!(response.headers()["content-encoding"], "gzip"); assert_eq!(response.headers()["access-control-allow-origin"], "*");
    let bytes = response.bytes().await.unwrap(); let mut body = Vec::new();
    flate2::read::GzDecoder::new(bytes.as_ref()).read_to_end(&mut body).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["usd_brl"], 5.5);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
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
    h.ready().await;
    h.usage("/api/uso").await;
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
    assert_eq!(h.usage("/api/uso").await, json!({"from_python":true}));
    assert!(h.forwarded("GET", "/api/uso"));
    std::fs::write(&wire, saved).unwrap();
    let collector = h.collector.clone();
    assert!(matches!(tokio::task::spawn_blocking(move || collector.prepare_blocking(true)).await.unwrap().unwrap(), Ready::Go));
    let recovered = h.ready().await;
    assert_eq!(recovered, json!({"from_python":true}));
    let mut current = key; current.data_version = h.collector.data_version();
    assert!(h.state.reports.get::<CostReport>(&current).is_none());
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

#[tokio::test]
async fn session_cost_reads_fixture_without_scopes_or_warming_scan() {
    let h = Harness::new(false, false).await;
    let rollout = h.base.join("codex/sessions/2026/09/30/rollout-c1.jsonl");
    h.upstream.infos.lock().unwrap().insert("codex-session".into(), json!({"provider":"codex", "jsonl":rollout}));
    let response = h.request(reqwest::Method::GET, "/api/sessions/codex-session/cost").send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "application/json");
    let body: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    common::costs::assert_close(&body, &json!({"cost_usd":0.0049875, "missing_models":[], "has_usage":true}), "custo da sessão");
    assert!(!h.forwarded("GET", "/api/sessions/codex-session/cost"));
    assert!(h.forwarded("GET", "/internal/sessions/codex%2Dsession/info"));
    assert!(!h.forwarded("GET", "/internal/costs/scopes"));
}

#[tokio::test]
async fn session_cost_other_provider_has_python_404_body() {
    let h = Harness::new(false, false).await;
    for provider in ["claude", "pi", "kimi", "omp"] {
        h.upstream.infos.lock().unwrap().insert("session".into(), json!({"provider":provider, "jsonl":h.base.join("codex/sessions/2026/09/30/rollout-c1.jsonl")}));
        let response = h.request(reqwest::Method::GET, "/api/sessions/session/cost").send().await.unwrap();
        assert_eq!(response.status(), 404, "{provider}");
        assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap(), session_error("Codex session not found"));
    }
    assert!(!h.forwarded("GET", "/api/sessions/session/cost"));
}

#[tokio::test]
async fn session_cost_missing_internal_info_has_python_404_body() {
    let h = Harness::new(false, false).await;
    let response = h.request(reqwest::Method::GET, "/api/sessions/absent/cost").send().await.unwrap();
    assert_eq!(response.status(), 404);
    assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap(), session_error("Codex session not found"));
    assert!(h.forwarded("GET", "/internal/sessions/absent/info"));
    assert!(!h.forwarded("GET", "/api/sessions/absent/cost"));
}

#[tokio::test]
async fn session_cost_missing_or_empty_rollout_field_has_session_404_body() {
    let h = Harness::new(false, false).await;
    for info in [json!({"provider":"codex"}), json!({"provider":"codex", "jsonl":null}), json!({"provider":"codex", "jsonl":""})] {
        h.upstream.infos.lock().unwrap().insert("session".into(), info);
        let response = h.request(reqwest::Method::GET, "/api/sessions/session/cost").send().await.unwrap();
        assert_eq!(response.status(), 404);
        assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap(), session_error("Codex session not found"));
    }
}

#[tokio::test]
async fn session_cost_deleted_rollout_has_python_404_body_and_cors() {
    let h = Harness::new(false, false).await;
    h.upstream.infos.lock().unwrap().insert("session".into(), json!({"provider":"codex", "jsonl":h.base.join("deleted.jsonl")}));
    let response = h.request(reqwest::Method::GET, "/api/sessions/session/cost").header("origin", "https://teste.exemplo").send().await.unwrap();
    assert_eq!(response.status(), 404);
    assert_eq!(response.headers()["access-control-allow-origin"], "*");
    assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap(), session_error("Codex rollout not found"));
    assert!(!h.forwarded("GET", "/api/sessions/session/cost"));
}

#[tokio::test]
async fn session_cost_recreated_session_reads_new_info_without_cache() {
    let h = Harness::new(false, false).await;
    let first = h.base.join("first.jsonl");
    let second = h.base.join("second.jsonl");
    session_rollout(&first, "gpt-5.6-sol", 1_000_000);
    session_rollout(&second, "gpt-5.5", 1_000_000);
    h.upstream.infos.lock().unwrap().insert("session".into(), json!({"provider":"codex", "jsonl":first}));
    let response = h.request(reqwest::Method::GET, "/api/sessions/session/cost").send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap(), json!({"cost_usd":5.0, "missing_models":[], "has_usage":true}));
    h.upstream.infos.lock().unwrap().insert("session".into(), json!({"provider":"codex", "jsonl":second}));
    let response = h.request(reqwest::Method::GET, "/api/sessions/session/cost").send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap(), json!({"cost_usd":2.0, "missing_models":[], "has_usage":true}));
    assert_eq!(h.upstream.hits.lock().unwrap().iter().filter(|(method, uri)| method == "GET" && uri == "/internal/sessions/session/info").count(), 2);
}

#[tokio::test]
async fn session_cost_unavailable_index_falls_back_to_python() {
    let h = Harness::new(false, true).await;
    h.upstream.infos.lock().unwrap().insert("session".into(), json!({"provider":"codex", "jsonl":h.base.join("codex/sessions/2026/09/30/rollout-c1.jsonl")}));
    let response = h.request(reqwest::Method::GET, "/api/sessions/session/cost").send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap(), json!({"from_python":true}));
    assert!(h.forwarded("GET", "/internal/sessions/session/info"));
    assert!(h.forwarded("GET", "/api/sessions/session/cost"));
    assert!(!h.forwarded("GET", "/internal/costs/scopes"));
}

#[tokio::test]
async fn session_cost_index_disk_loss_after_open_falls_back_to_python() {
    let h = Harness::new(false, false).await;
    h.upstream.infos.lock().unwrap().insert("session".into(), json!({"provider":"codex", "jsonl":h.base.join("codex/sessions/2026/09/30/rollout-c1.jsonl")}));
    let response = h.request(reqwest::Method::GET, "/api/sessions/session/cost").send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap()["has_usage"], true);
    let index_dir = h._dir.path().join("idx");
    std::fs::remove_dir_all(&index_dir).unwrap();
    std::fs::write(&index_dir, b"bloqueio").unwrap();
    let response = h.request(reqwest::Method::GET, "/api/sessions/session/cost").send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap(), json!({"from_python":true}));
    assert!(h.forwarded("GET", "/api/sessions/session/cost"));
}

#[tokio::test]
async fn session_cost_reader_panic_and_read_error_fall_back_to_python() {
    let h = Harness::new(false, false).await;
    let path = h.base.join("rollout-malformed.jsonl");
    std::fs::write(&path, format!("{}\n", json!({"type":"session_meta", "payload":{"id":true}}))).unwrap();
    h.upstream.infos.lock().unwrap().insert("session".into(), json!({"provider":"codex", "jsonl":path}));
    let response = h.request(reqwest::Method::GET, "/api/sessions/session/cost").send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap(), json!({"from_python":true}));
    assert!(h.forwarded("GET", "/api/sessions/session/cost"));
    session_rollout(&path, "gpt-5.5", 1_000_000);
    let response = h.request(reqwest::Method::GET, "/api/sessions/session/cost").send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap(), json!({"from_python":true}));
    h.upstream.infos.lock().unwrap().insert("healthy".into(), json!({"provider":"codex", "jsonl":path}));
    let response = h.request(reqwest::Method::GET, "/api/sessions/healthy/cost").send().await.unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap()["cost_usd"], 2.0);
    let conn = rusqlite::Connection::open(h._dir.path().join("idx").join(hangar_server::costs::index::FILE_NAME)).unwrap();
    conn.execute("UPDATE custo SET ts='invalid'", []).unwrap();
    drop(conn);
    let response = h.request(reqwest::Method::GET, "/api/sessions/session/cost").send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap(), json!({"from_python":true}));
}

#[tokio::test]
async fn session_cost_owner_gate_methods_and_query_token_preserve_contract() {
    let h = Harness::new(false, false).await;
    h.upstream.infos.lock().unwrap().insert("session".into(), json!({"provider":"codex", "jsonl":h.base.join("codex/sessions/2026/09/30/rollout-c1.jsonl")}));
    for token in [None, Some("convidado")] {
        let mut request = reqwest::Client::new().get(format!("http://{}/api/sessions/session/cost", h.address));
        if let Some(token) = token { request = request.bearer_auth(token); }
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), 401);
        assert_eq!(response.bytes().await.unwrap().len(), 0);
    }
    assert!(!h.forwarded("GET", "/internal/sessions/session/info"));
    for method in [reqwest::Method::POST, reqwest::Method::HEAD, reqwest::Method::OPTIONS] {
        let response = h.request(method.clone(), "/api/sessions/session/cost").send().await.unwrap();
        assert_eq!(response.status(), 200);
        if method == reqwest::Method::HEAD { assert_eq!(response.bytes().await.unwrap().len(), 0); }
        else { assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap(), json!({"from_python":true})); }
        assert!(h.forwarded(method.as_str(), "/api/sessions/session/cost"));
    }
    assert!(!h.forwarded("GET", "/internal/sessions/session/info"));
    let response = reqwest::Client::new().get(format!("http://{}/api/sessions/session/cost?token={OWNER}", h.address)).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap()["has_usage"], true);
    assert!(h.forwarded("GET", "/internal/sessions/session/info"));
    assert!(!h.forwarded("GET", &format!("/api/sessions/session/cost?token={OWNER}")));
}

#[tokio::test]
async fn session_cost_empty_usage_is_null_and_pricing_reload_is_independent() {
    let h = Harness::new(false, false).await;
    let path = h.base.join("rollout-synthetic.jsonl");
    session_rollout(&path, "gpt-5.5", 0);
    h.upstream.infos.lock().unwrap().insert("session".into(), json!({"provider":"codex", "jsonl":path}));
    let response = h.request(reqwest::Method::GET, "/api/sessions/session/cost").send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap(), json!({"cost_usd":null, "missing_models":[], "has_usage":false}));
    session_rollout(&path, "gpt-5.5", 1_000_000);
    std::fs::write(h.base.join("pricing/overrides.json"), json!({"gpt-5.5":{"input":7,"output":8,"provider":"openai"}}).to_string()).unwrap();
    let response = h.request(reqwest::Method::GET, "/api/sessions/session/cost").send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap(), json!({"cost_usd":7.0, "missing_models":[], "has_usage":true}));
    assert!(!h.forwarded("GET", "/internal/costs/scopes"));
}

#[tokio::test]
async fn session_cost_missing_tariff_uses_shared_gzip_and_cors() {
    use std::io::Read;
    let h = Harness::new(false, false).await;
    let path = h.base.join("rollout-synthetic.jsonl");
    let missing = "modelo-a".repeat(160);
    session_rollout(&path, &format!("openai/{missing}"), 1);
    h.upstream.infos.lock().unwrap().insert("session".into(), json!({"provider":"codex", "jsonl":path}));
    let response = h.request(reqwest::Method::GET, "/api/sessions/session/cost").header("accept-encoding", "gzip").header("origin", "https://teste.exemplo").send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-encoding"], "gzip");
    assert_eq!(response.headers()["access-control-allow-origin"], "*");
    assert_eq!(response.headers()["access-control-expose-headers"], "ETag");
    let bytes = response.bytes().await.unwrap();
    let mut body = Vec::new();
    flate2::read::GzDecoder::new(bytes.as_ref()).read_to_end(&mut body).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), json!({"cost_usd":null, "missing_models":[missing], "has_usage":true}));
}

#[tokio::test]
async fn usage_labels_and_pricing_generations_invalidate_cached_dimensions() {
    let h = Harness::new(true, false).await;
    let before = h.usage("/api/uso").await;
    h.upstream.scopes.lock().unwrap().as_mut().unwrap().claude[0].label = "Conta de uso renomeada".into();
    let collector = h.collector.clone();
    tokio::task::spawn_blocking(move || collector.prepare_blocking(true)).await.unwrap().unwrap();
    let relabeled = h.usage("/api/uso").await;
    assert!(relabeled["by_conta"].as_array().unwrap().iter().any(|b| b["label"] == "Conta de uso renomeada"));
    let mut overrides = serde_json::Map::new();
    for bucket in before["by_modelo"].as_array().unwrap() {
        overrides.insert(bucket["key"].as_str().unwrap().into(), json!({"input":100, "output":100, "cache_write":100, "cache_read":100, "provider":"anthropic"}));
    }
    std::fs::write(h.base.join("pricing/overrides.json"), Value::Object(overrides).to_string()).unwrap();
    let repriced = h.usage("/api/uso").await;
    assert_ne!(repriced["totals"]["cost"], before["totals"]["cost"]);
}

#[tokio::test]
async fn session_cost_finite_tariff_overflow_falls_back_but_zero_remains_numeric() {
    let h = Harness::new(false, false).await;
    let path = h.base.join("rollout-overflow.jsonl");
    session_rollout(&path, "gpt-5.5", 2_000_000);
    std::fs::write(h.base.join("pricing/overrides.json"), json!({"gpt-5.5":{
        "input":1e308, "output":0, "cache_write":0, "cache_read":0, "provider":"openai"
    }}).to_string()).unwrap();
    h.upstream.infos.lock().unwrap().insert("session".into(), json!({"provider":"codex", "jsonl":path}));
    let response = h.request(reqwest::Method::GET, "/api/sessions/session/cost").send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap(), json!({"from_python":true}));
    assert!(h.forwarded("GET", "/internal/sessions/session/info"));
    assert!(h.forwarded("GET", "/api/sessions/session/cost"));
    assert!(!h.forwarded("GET", "/internal/costs/scopes"));
    h.upstream.hits.lock().unwrap().clear();
    std::fs::write(h.base.join("pricing/overrides.json"), json!({"gpt-5.5":{
        "input":0, "output":0, "cache_write":0, "cache_read":0, "provider":"openai"
    }}).to_string()).unwrap();
    let response = h.request(reqwest::Method::GET, "/api/sessions/session/cost").send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap(), json!({"from_python":true}));
    assert!(h.forwarded("GET", "/api/sessions/session/cost"));
    h.upstream.infos.lock().unwrap().insert("healthy".into(), json!({"provider":"codex", "jsonl":path}));
    let response = h.request(reqwest::Method::GET, "/api/sessions/healthy/cost").send().await.unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap(), json!({"cost_usd":0.0, "missing_models":[], "has_usage":true}));
}

#[tokio::test]
async fn usage_failed_quote_is_null_and_panicking_worker_falls_back() {
    let h = Harness::with_fx(true, false, Arc::new(Fx::with_fetch(|| None))).await;
    assert!(h.usage("/api/uso").await["usd_brl"].is_null());
    let h = Harness::with_fx(true, false, Arc::new(Fx::with_fetch(|| panic!("falha sintética de uso")))).await;
    assert_eq!(h.usage("/api/uso").await, json!({"from_python":true}));
    assert!(h.forwarded("GET", "/api/uso"));
}
