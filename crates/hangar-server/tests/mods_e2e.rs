//! A interface dos mods de ponta a ponta: rota do app, guarda da troca de agente (Python falso), ator
//! do runtime, superfície, cano falso com a vitrine gravada e a faixa no SSE dos aparelhos.
mod fake;
mod mods_support;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fake::*;
use hangar_server::mods::state::Mods;
use hangar_server::routes::AppState;
use hangar_server::runtime::gateway::RuntimeRegistry;
use mods_support::*;
use serde_json::Value;

struct World {
    python: Arc<Fake>,
    server: SocketAddr,
    mods: Mods,
    registry: RuntimeRegistry,
    seen: Seen,
    cano: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

impl Drop for World {
    fn drop(&mut self) {
        self.cano.abort();
    }
}

/// Servidor com o `Mods` ligado ao registro do runtime, como no `lib.rs`, e a sessão `session` aberta
/// nele sobre o cano da vitrine, já com a faixa desenhada.
async fn world(swallow: &'static [&'static str]) -> World {
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("chat.jsonl");
    append_lines(&transcript, 0..1);
    let (python, upstream) = spawn_fake().await;
    python.set_info(info_json("claude-headless", &transcript));
    let state = AppState::new(config(upstream, "127.0.0.1"));
    let mods = state.mods.clone();
    let server = spawn_state(state).await;
    let (escuta, seen, cano) = vitrine_cano(swallow).await;
    let registry = registry(&mods);
    registry.open(claude_target(dir.path(), escuta, true)).await.unwrap();
    wait_ui(&mods, |ui| ui["above"].to_string().contains("superfície desktop")).await;
    World { python, server, mods, registry, seen, cano, _dir: dir }
}

async fn press(server: SocketAddr, key: &'static str) -> (u16, Value) {
    let response = client().post(format!("http://{server}/api/sessions/session/plugin/press"))
        .header("content-type", "application/json").header("authorization", format!("Bearer {OWNER}"))
        .body(serde_json::json!({"site": "above-prompt", "key": key}).to_string()).send().await.unwrap();
    let status = response.status().as_u16();
    let text = response.text().await.unwrap();
    (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

fn pressed(seen: &Seen) -> usize {
    seen.lock().unwrap().iter().filter(|(kind, _)| kind == "ui_press").count()
}

#[tokio::test]
async fn turn_and_guard_that_eat_the_budget_keep_the_press_off_the_mod() {
    // I1: a vez da sessão e a guarda gastam o orçamento da rota; o que sobra não cobre o prazo do
    // clique, e o `ui_press` não pode sair (rodaria no mod depois de o app mostrar o erro).
    let world = world(&[]).await;
    let (_, turn) = world.mods.link("session").unwrap();
    let held = turn.lock().await;
    world.python.set_transfer_delay(Duration::from_millis(300));
    let start = Instant::now();
    let request = tokio::spawn(press(world.server, "abrir-vitrine-botoes"));
    tokio::time::sleep(Duration::from_millis(4500)).await;
    drop(held);
    let (status, body) = request.await.unwrap();
    assert_eq!((status, body["detail"]["code"].as_str()), (409, Some("erro_mod_clique_sem_resposta")));
    assert!(start.elapsed() < Duration::from_secs(8), "{:?}", start.elapsed());
    assert_eq!(world.python.transfer_calls(), 1, "a guarda foi perguntada");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(pressed(&world.seen), 0, "o press não saiu ao mod");
    // Com o orçamento inteiro, o mesmo clique chega ao mod.
    world.python.set_transfer_delay(Duration::ZERO);
    assert_eq!(press(world.server, "abrir-vitrine-botoes").await.0, 200);
    assert_eq!(pressed(&world.seen), 1);
    world.registry.close("key", 1).await.unwrap();
}
