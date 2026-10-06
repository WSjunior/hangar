//! Rotas da lista do dono no Rust (`GET /api/sessions` e `/api/sessions/events`): um produtor por
//! servidor, `list_error` uma vez na transição, `nav` uma vez por cliente, retrato de até 2 s fresco
//! depois de invalidação, 503 com código e convidado com o Python. O multiplexador é um script que
//! conta as chamadas e recusa enquanto existir o arquivo `fail`.
#![cfg(unix)]
mod fake;

use fake::{OWNER, SECRET, client, config, next_named, sse, Events};
use hangar_server::list::bridge::{ListBridge, ListEnv, parse_dirs};
use hangar_server::list::facts::FactsClient;
use hangar_server::list::hub::HeadlessSource;
use hangar_server::list::mux::Mux;
use hangar_server::routes::{AppState, router};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

fn now() -> f64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64() }

/// `n` sessões Claude paradas (`s0`..), marcador `idle`, pane pronto; `fail` presente = recusa.
fn sessions(root: &Path, n: usize) -> std::path::PathBuf {
    let home = root.join("home");
    let mut panes = String::new();
    for i in 0..n {
        let cwd = root.join(format!("w{i}"));
        let sid = format!("00000000-0000-0000-0000-{i:012}");
        let proj = home.join(".claude/projects").join(hangar_workspace::worktrees::sanitize_cwd(cwd.to_str().unwrap()));
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::write(proj.join(format!("{sid}.jsonl")), fake::claude_line(0)).unwrap();
        std::fs::create_dir_all(home.join(".claude/.hangar-state")).unwrap();
        std::fs::write(home.join(format!(".claude/.hangar-state/{sid}.json")), format!(r#"{{"state":"idle","ts":{}}}"#, now() + 60.0)).unwrap();
        panes.push_str(&format!("s{i}\\t1\\t\\t{}\\t%%{i}\\t\\t\\t\\t0\\t0\\n", cwd.display()));
    }
    let script = root.join("tmux");
    std::fs::write(&script, format!(
        "#!/bin/sh\necho \"$1\" >> '{log}'\n[ -e '{fail}' ] && exit 2\n[ \"$1\" = list-panes ] || {{ printf '● pronto\\n❯\\n'; exit 0; }}\nprintf '{panes}'\n",
        log = root.join("calls.log").display(), fail = root.join("fail").display())).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

fn calls(root: &Path, sub: &str) -> usize {
    std::fs::read_to_string(root.join("calls.log")).unwrap_or_default().lines().filter(|l| *l == sub).count()
}

struct Server { addr: SocketAddr, python: Arc<fake::Fake>, list: Arc<ListBridge> }

async fn server(root: &Path, n: usize) -> Server {
    let script = sessions(root, n);
    let home = root.join("home");
    let dirs = parse_dirs(&json!({"home": home, "claude": home.join(".claude"), "codex_home": home.join(".codex"),
        "pi_sessions": home.join(".pi/agent/sessions"), "omp_config": home.join(".omp"),
        "omp_agent": home.join(".omp/agent"), "kimi_home": home.join(".kimi-code")}).to_string());
    let (python, upstream) = fake::spawn_fake().await;
    let mut state = AppState::new(config(upstream, ""));
    let list = Arc::new(ListBridge::new(ListEnv { mux: Mux::with_program(&script, Duration::from_secs(5)),
        capture_program: script.into_os_string(), procs: Arc::new(hangar_server::list::procs::SystemProcs::default()), dirs },
        FactsClient::new(upstream, SECRET.into())));
    state.list = list.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(Arc::new(state)).into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server { addr, python, list }
}

async fn get(addr: SocketAddr, token: &str) -> (u16, Value) {
    let resp = client().get(format!("http://{addr}/api/sessions")).bearer_auth(token).send().await.unwrap();
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap();
    (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

async fn open(addr: SocketAddr) -> Events {
    let resp = client().get(format!("http://{addr}/api/sessions/events?token={OWNER}")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["cache-control"], "no-store");
    sse(resp)
}

fn names(v: &Value) -> Vec<&str> { v.as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap()).collect() }

fn set_facts(python: &fake::Fake, key: &str, value: Value) {
    python.list_facts.lock().unwrap().0[key] = value;
}

/// Cinco listas abertas pagam a varredura de uma: o produtor é do servidor, não da conexão.
#[tokio::test(flavor = "multi_thread")]
async fn one_producer_for_many_clients() {
    let dir = tempfile::tempdir().unwrap();
    let srv = server(dir.path(), 2).await;
    let mut streams = Vec::new();
    for _ in 0..5 {
        streams.push(open(srv.addr).await);
    }
    for es in &mut streams {
        let ev = next_named(es, "sessions").await;
        assert_eq!(names(&serde_json::from_str(&ev.data).unwrap()), ["s0", "s1"]);
    }
    tokio::time::sleep(Duration::from_millis(3200)).await;
    let scans = calls(dir.path(), "list-panes");
    assert!((1..=5).contains(&scans), "uma varredura por tique, não por cliente: {scans}");
    assert!(srv.python.list_facts_last.lock().unwrap()["owner_clients"] == 5, "o Python sabe quantas listas do dono estão abertas");
    drop(streams);
    tokio::time::sleep(Duration::from_millis(3500)).await;
    let after = calls(dir.path(), "list-panes");
    tokio::time::sleep(Duration::from_millis(3200)).await;
    assert_eq!(calls(dir.path(), "list-panes"), after, "sem cliente, o produtor para");
}

/// Falha vira `list_error` uma vez; a volta reemite a mesma lista para limpar o erro na tela.
#[tokio::test(flavor = "multi_thread")]
async fn error_then_recovery_reemits_same_list() {
    let dir = tempfile::tempdir().unwrap();
    let srv = server(dir.path(), 1).await;
    let mut es = open(srv.addr).await;
    let first = next_named(&mut es, "sessions").await.data;
    std::fs::write(dir.path().join("fail"), "").unwrap();
    let err = next_named(&mut es, "list_error").await;
    assert_eq!(serde_json::from_str::<Value>(&err.data).unwrap()["code"], "mux_unavailable");
    std::fs::remove_file(dir.path().join("fail")).unwrap();
    let again = next_named(&mut es, "sessions").await.data;
    assert_eq!(again, first, "mesma lista, reemitida depois do erro");
    let python = srv.python.clone();
    fake::wait_until(move || python.diag().iter().any(|d| d["evento"] == "rust.list_failed" && d["codigo"] == "mux_unavailable")).await;
}

/// O navegador pedido sai uma vez em cada lista aberta, nunca de novo no tique seguinte.
#[tokio::test(flavor = "multi_thread")]
async fn nav_once_per_client() {
    let dir = tempfile::tempdir().unwrap();
    let srv = server(dir.path(), 1).await;
    set_facts(&srv.python, "nav", json!({"s0": {"url": "http://127.0.0.1:5173/", "ts": 7.0}}));
    let mut a = open(srv.addr).await;
    let nav = next_named(&mut a, "nav").await;
    assert_eq!(serde_json::from_str::<Value>(&nav.data).unwrap(), json!({"name": "s0", "url": "http://127.0.0.1:5173/"}));
    let mut b = open(srv.addr).await;
    next_named(&mut b, "nav").await;
    let more = tokio::time::timeout(Duration::from_millis(3500), next_named(&mut a, "nav")).await;
    assert!(more.is_err(), "o mesmo pedido não sai duas vezes na mesma lista");
}

/// `GET` dentro de 2 s reaproveita o retrato; depois de invalidar, produz de novo.
#[tokio::test(flavor = "multi_thread")]
async fn get_after_invalidate_is_fresh() {
    let dir = tempfile::tempdir().unwrap();
    let srv = server(dir.path(), 1).await;
    let (status, body) = get(srv.addr, OWNER).await;
    assert_eq!((status, names(&body)), (200, vec!["s0"]));
    get(srv.addr, OWNER).await;
    assert_eq!(calls(dir.path(), "list-panes"), 1, "retrato de até 2 s");
    srv.list.invalidate();
    get(srv.addr, OWNER).await;
    assert_eq!(calls(dir.path(), "list-panes"), 2, "invalidado não serve o de antes");
    assert_eq!(srv.python.hits_to("/api/sessions"), 0, "a lista do dono nunca vai ao Python");
}

#[tokio::test(flavor = "multi_thread")]
async fn mux_unavailable_is_503() {
    let dir = tempfile::tempdir().unwrap();
    let srv = server(dir.path(), 1).await;
    std::fs::write(dir.path().join("fail"), "").unwrap();
    let (status, body) = get(srv.addr, OWNER).await;
    assert_eq!(status, 503);
    assert_eq!(body["detail"]["code"], "erro_mux_indisponivel");
    assert_eq!(body["detail"]["params"]["detalhe"], "mux_refused");
    let python = srv.python.clone();
    fake::wait_until(move || python.diag().iter().any(|d| d["evento"] == "rust.list_route_failed" && d["codigo"] == "mux_unavailable")).await;
}

/// Sem nenhuma resposta dos fatos, acesso e escondidas são desconhecidos: nem `GET` nem SSE servem.
#[tokio::test(flavor = "multi_thread")]
async fn unknown_facts_are_never_served() {
    let dir = tempfile::tempdir().unwrap();
    let srv = server(dir.path(), 1).await;
    *srv.python.list_facts.lock().unwrap() = (json!({"quebrado": true}), Duration::ZERO);
    let (status, body) = get(srv.addr, OWNER).await;
    assert_eq!((status, &body["detail"]["code"], &body["detail"]["params"]["detalhe"]),
        (503, &json!("erro_lista_indisponivel"), &json!("list_facts_unknown")));
    let mut es = open(srv.addr).await;
    let err = next_named(&mut es, "list_error").await;
    assert_eq!(serde_json::from_str::<Value>(&err.data).unwrap()["code"], "list_facts_unknown");
}

#[tokio::test(flavor = "multi_thread")]
async fn guest_token_goes_to_python() {
    let dir = tempfile::tempdir().unwrap();
    let srv = server(dir.path(), 1).await;
    assert_eq!(get(srv.addr, "convidado").await, (200, json!("from-python")));
    let resp = client().get(format!("http://{}/api/sessions/events?token=convidado", srv.addr)).send().await.unwrap();
    assert_eq!(resp.text().await.unwrap(), "from-python");
    let resp = client().post(format!("http://{}/api/sessions", srv.addr)).bearer_auth(OWNER).send().await.unwrap();
    assert_eq!(resp.text().await.unwrap(), "from-python", "criar sessão continua no Python");
    assert_eq!(calls(dir.path(), "list-panes"), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_from_owner_not_listed() {
    let dir = tempfile::tempdir().unwrap();
    let srv = server(dir.path(), 2).await;
    set_facts(&srv.python, "hidden", json!(["s1"]));
    let (_, body) = get(srv.addr, OWNER).await;
    assert_eq!(names(&body), ["s0"]);
    let mut es = open(srv.addr).await;
    let ev = next_named(&mut es, "sessions").await;
    assert_eq!(names(&serde_json::from_str(&ev.data).unwrap()), ["s0"]);
}

/// Fatos que caem depois de uma resposta boa marcam as linhas; a lista continua servida.
#[tokio::test(flavor = "multi_thread")]
async fn facts_down_marks_rows_not_list() {
    let dir = tempfile::tempdir().unwrap();
    let srv = server(dir.path(), 1).await;
    let mut es = open(srv.addr).await;
    let ok: Value = serde_json::from_str(&next_named(&mut es, "sessions").await.data).unwrap();
    assert_eq!(ok[0]["problema"], Value::Null);
    *srv.python.list_facts.lock().unwrap() = (json!({"quebrado": true}), Duration::ZERO);
    let marked: Value = serde_json::from_str(&next_named(&mut es, "sessions").await.data).unwrap();
    assert_eq!(marked[0]["problema"], "list_facts_unavailable");
}

struct Runtime(BTreeMap<String, Value>);

impl HeadlessSource for Runtime {
    fn snapshots(&self) -> futures_util::future::BoxFuture<'_, BTreeMap<String, Value>> {
        Box::pin(async move { self.0.clone() })
    }
}

/// Sessão sem terminal: com o runtime de pé, o estado vem do retrato dele (pela chave da sessão),
/// sem `list_runtime_absent`; retrato com erro aparece na linha.
#[tokio::test(flavor = "multi_thread")]
async fn headless_rows_take_the_runtime_state() {
    let dir = tempfile::tempdir().unwrap();
    let srv = server(dir.path(), 0).await;
    let hl = dir.path().join("home/.hangar/claude-headless");
    std::fs::create_dir_all(&hl).unwrap();
    for (name, key) in [("h0", "kh0"), ("h1", "kh1")] {
        std::fs::write(hl.join(format!("{name}.json")), json!({"name": name, "session_id": format!("sid-{name}"),
            "cwd": dir.path(), "key": key}).to_string()).unwrap();
    }
    srv.list.set_runtime(Arc::new(Runtime(BTreeMap::from([
        ("kh0".into(), json!({"view": {"alive": true, "public_state": {"state": "working", "label": "pensando"}}})),
        ("kh1".into(), json!({"error": "runtime_closed"})),
    ]))));
    let (status, body) = get(srv.addr, OWNER).await;
    assert_eq!(status, 200);
    let row = |n: &str| body.as_array().unwrap().iter().find(|r| r["name"] == n).unwrap().clone();
    assert_eq!((row("h0")["state"].clone(), row("h0")["problema"].clone()), (json!("working"), Value::Null));
    assert_eq!(row("h1")["problema"], "list_runtime_unavailable");
}
