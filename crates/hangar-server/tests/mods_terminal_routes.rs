//! Sessão com terminal aberta no Rust (fase 3): posse no `Mods` até o `close`, rotas dos apps levadas ao
//! elo, recusa do convidado e da digitação, vigia do tamanho e a janela esticada devolvida na abertura.
mod fake;
mod mods_support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use fake::*;
use hangar_server::mods::click::{Limits, Pane, PaneOp};
use hangar_server::mods::state::*;
use hangar_server::mods::terminal::TerminalLink;
use hangar_server::routes::AppState;
use hangar_server::runtime::gateway::{RuntimeRegistry, unstretch};
use hangar_server::runtime::terminal::TerminalTarget;
use hangar_server::terminal_input::TerminalBinding;
use mods_support::NoLink;
use mods_support::pane::{Effect::*, FakePane, view};
use serde_json::{Value, json};

/// O `reqwest` dos testes não tem a função `json`: o corpo vai pronto, com o tipo.
async fn post_as(server: std::net::SocketAddr, route: &str, body: Value, token: &str) -> (u16, Value) {
    let response = client().post(format!("http://{server}/api/sessions/t/plugin/{route}"))
        .header("authorization", format!("Bearer {token}")).header("content-type", "application/json")
        .body(body.to_string()).send().await.unwrap();
    let status = response.status().as_u16();
    let text = response.text().await.unwrap();
    (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

async fn terminal_app() -> (Arc<Fake>, std::net::SocketAddr, Mods, Arc<FakePane>) {
    let (python, upstream) = spawn_fake().await;
    let state = AppState::new(config(upstream, "127.0.0.1"));
    let mods = state.mods.clone();
    let server = spawn_state(state).await;
    let pane = Arc::new(FakePane::new(&mods, "t", "tmux-01-tres-paineis-150"));
    let life = mods.new_life();
    mods.attach_terminal("t", "proc-t", life, TerminalLink::new("t".into(), life, pane.clone(), mods.clone(), Limits::quick()));
    mods.terminal_ui("t", view(&[("pm-mock-pm", "xx-00000", "pm-a", "xxxxx"), ("pm-mock-mr", "MR ●2", "mr-a", "xxxxx"),
        ("pm-mock-jenkins", "Jenkins", "jenkins-a", "xxxxx")]));
    (python, server, mods, pane)
}

#[tokio::test]
async fn apps_reach_the_terminal_through_the_link() {
    let (_python, server, _mods, pane) = terminal_app().await;
    pane.on_click((0, 104), vec![Show("tmux-02-apos-clicar-mr-150")]);
    pane.on_click((0, 148), vec![CloseAll]);
    assert_eq!(post_as(server, "show", json!({"site": "pm-mock-mr"}), OWNER).await, (200, json!({"ok": true, "shown_id": "pm-mock-mr"})));
    let (status, body) = post_as(server, "input", json!({"site": "pm-mock-mr", "key": "k", "kind": "submit", "value": "x"}), OWNER).await;
    assert_eq!((status, body["detail"]["code"].as_str()), (409, Some("erro_mod_sem_digitacao")));
    assert_eq!(post_as(server, "press", json!({"site": "pm-mock-mr", "key": "__close__"}), OWNER).await, (200, json!({"ok": true})));
    assert_eq!(pane.actions(), ["click 0 104", "click 0 148"]);
}

#[tokio::test]
async fn typing_is_refused_without_waiting_the_turn_or_the_transfer_guard() {
    let (python, server, mods, pane) = terminal_app().await;
    // Um clique em curso segura a vez da sessão; a recusa da digitação não espera por ela.
    let turn = mods.link("t").unwrap().lock;
    let _held = turn.lock().await;
    let started = Instant::now();
    let (status, body) = post_as(server, "input", json!({"site": "pm-mock-mr", "key": "k", "kind": "change", "value": "x"}), OWNER).await;
    assert_eq!((status, body["detail"]["code"].as_str()), (409, Some("erro_mod_sem_digitacao")));
    assert!(started.elapsed() < Duration::from_secs(2), "a recusa saiu em {:?}", started.elapsed());
    assert_eq!(python.transfer_calls(), 0, "a guarda da troca de agente não é consultada");
    assert!(pane.actions().is_empty());
}

#[tokio::test]
async fn guests_are_refused_here_and_bad_tokens_go_to_python() {
    let (python, server, _mods, pane) = terminal_app().await;
    // A forma do token de convidado: `secrets.token_urlsafe(32)`, 43 caracteres de base64url.
    let guest = format!("{}-_", "g".repeat(41));
    for (route, body) in [("press", json!({"site": "pm-mock-mr", "key": "mr-a"})), ("show", json!({"site": "pm-mock-mr"})),
        ("input", json!({"site": "pm-mock-mr", "key": "k", "kind": "submit", "value": "x"}))] {
        let (status, answer) = post_as(server, route, body, &guest).await;
        assert_eq!((status, answer["detail"]["code"].as_str()), (403, Some("erro_mod_convidado")), "{route}");
    }
    assert_eq!(python.hits_to("/api/sessions/t/plugin/press") + python.hits_to("/api/sessions/t/plugin/show")
        + python.hits_to("/api/sessions/t/plugin/input"), 0,
        "o pedido do convidado não chega ao Python, que dirigiria o pane por fora");
    // Nem dono nem convidado: o Python responde (401 ou 429) e conta a falha, como antes.
    let (_, answer) = post_as(server, "press", json!({"site": "pm-mock-mr", "key": "mr-a"}), "errado").await;
    assert_eq!(answer, "from-python");
    assert!(pane.actions().is_empty());
}

fn target(dir: &std::path::Path, mux: &str) -> TerminalTarget {
    let binding = TerminalBinding { name: "t".into(), pane: "%1".into(), conversation: "sid".into(), generation: 1, created: 1,
        mux_argv: vec![mux.into()], windows: false, clipboard_lock_path: None };
    let transcript = dir.join("t.jsonl");
    std::fs::write(&transcript, "").unwrap();
    TerminalTarget { key: "key-t".into(), generation: 1, name: "t".into(), binding, lease_path: dir.join("lease"),
        state_path: dir.join("state"), projection_dir: dir.join("projection"), transcript, created: 0.0 }
}

fn registry(mods: &Mods) -> RuntimeRegistry {
    RuntimeRegistry::new("127.0.0.1:9".parse().unwrap(), "secret-test".into(), "instance-test".into()).with_mods(mods.clone())
}

#[tokio::test]
async fn terminal_session_is_owned_until_close() {
    let dir = tempfile::tempdir().unwrap();
    let mods = Mods::default();
    let registry = registry(&mods);
    registry.open_terminal(target(dir.path(), "/does-not-exist/hangar-test-tmux")).await.unwrap();
    assert!(mods.owns("t") && mods.is_terminal("t"));
    assert_eq!(mods.bridge_session("t").as_deref(), Some("t"), "a ponte acha a sessão com terminal");
    registry.close("key-t", 1).await.unwrap();
    assert!(!mods.owns("t"), "fechar a sessão tira a interface dos mods do Rust");
}

#[tokio::test]
async fn closing_the_terminal_forgets_only_its_own_life() {
    let dir = tempfile::tempdir().unwrap();
    let mods = Mods::default();
    let registry = registry(&mods);
    registry.open_terminal(target(dir.path(), "/does-not-exist/hangar-test-tmux")).await.unwrap();
    // Outra vida tomou o nome (uma sessão sem terminal, por exemplo): o fechar do terminal não a apaga.
    let other = mods.new_life();
    mods.attach("t", other, Arc::new(NoLink));
    registry.close("key-t", 1).await.unwrap();
    assert_eq!(mods.life("t"), Some(other));
}

/// O processo `pid` acabou (sumiu ou virou zumbi).
#[cfg(unix)]
fn gone(pid: &str) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).map_or(true, |stat| {
        stat.rsplit_once(") ").is_some_and(|(_, rest)| rest.starts_with('Z'))
    })
}

#[cfg(unix)]
async fn wait_until(what: &str, check: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !check() { tokio::time::sleep(Duration::from_millis(20)).await; }
    }).await.unwrap_or_else(|_| panic!("{what}"));
}

/// O vigia do tamanho fica vivo depois de abrir e cai com o fechar. O multiplexador é um script: com `-C`
/// (o cliente de controle do vigia), anota o pid e fica lendo a entrada; o resto falha.
#[cfg(unix)]
#[tokio::test]
async fn the_size_watch_is_alive_after_opening_and_stops_on_close() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("watch.pid");
    let mux = dir.path().join("mux");
    std::fs::write(&mux, format!("#!/bin/sh\ncase \" $* \" in *\" -C \"*) echo $$ > '{}'; exec cat;; esac\nexit 1\n",
        marker.display())).unwrap();
    std::fs::set_permissions(&mux, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mods = Mods::default();
    let registry = registry(&mods);
    registry.open_terminal(target(dir.path(), mux.to_str().unwrap())).await.unwrap();
    wait_until("o vigia não subiu", || std::fs::read_to_string(&marker).is_ok_and(|pid| !pid.trim().is_empty())).await;
    let pid = std::fs::read_to_string(&marker).unwrap().trim().to_owned();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!gone(&pid), "o cliente de controle do vigia continua ligado depois de abrir");
    registry.close("key-t", 1).await.unwrap();
    wait_until("o vigia continuou depois do fechar", || gone(&pid)).await;
}

/// Um clique cortado pelo fim do servidor deixa a janela com 250 linhas; a abertura seguinte, sem terminal
/// ligado, devolve o mínimo (o redimensionar do executor volta ao `window-size latest`).
#[tokio::test]
async fn opening_gives_back_a_window_left_stretched() {
    let mods = Mods::default();
    let until = || Instant::now() + Duration::from_secs(2);
    let pane = FakePane::new(&mods, "t", "tmux-01-tres-paineis-150");
    pane.clients(0);
    pane.op(PaneOp::Resize { columns: 150, rows: 250 }, until()).await.unwrap();
    assert!(unstretch(&pane, until()).await);
    assert_eq!(pane.actions(), ["resize 150 250", "resize 150 40"]);
    // Já no tamanho de antes: nada a devolver.
    assert!(!unstretch(&pane, until()).await);

    // Estreita demais além de esticada: volta também às colunas mínimas.
    let narrow = FakePane::new(&mods, "t", "tmux-01-tres-paineis-150");
    narrow.clients(0);
    narrow.op(PaneOp::Resize { columns: 100, rows: 250 }, until()).await.unwrap();
    assert!(unstretch(&narrow, until()).await);
    assert_eq!(narrow.actions(), ["resize 100 250", "resize 144 40"]);

    // Com um terminal ligado, o tamanho é dele.
    let attached = FakePane::new(&mods, "t", "tmux-01-tres-paineis-150");
    attached.op(PaneOp::Resize { columns: 150, rows: 250 }, until()).await.unwrap();
    assert!(!unstretch(&attached, until()).await);
    assert_eq!(attached.actions(), ["resize 150 250"]);
}
