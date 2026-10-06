//! A interface dos mods no ator do runtime: pedido de app e escrita fora do diário.
mod mods_support;

use hangar_server::mods::model::ModsCall;
use hangar_server::mods::state::Mods;
use hangar_server::runtime::{actor::*,cano,gateway::RuntimeRegistry,protocol::*,queue::*};
use serde_json::json;
use std::sync::{Arc,Mutex};
use tokio::io::{AsyncBufReadExt,AsyncWriteExt,BufReader};

/// Alvo de sessão Claude sem terminal. `initialized: false` deixa a superfície sem ligar: ela espera o
/// `initialize` dizer se o processo a aceita (A13).
fn claude_target(dir:&std::path::Path,escuta:String,initialized:bool) -> RuntimeTarget {
    RuntimeTarget { key:"key".into(),generation:1,name:"session".into(),provider:"claude".into(),
        metadata:json!({"name":"session","headless":true,"session_id":"sid-1","initialized":initialized}),
        binding:CanoBinding { pid:42,escuta,token:"secret-test".into(),versao:2 },
        lease_path:dir.join("key.lock"),state_path:dir.join("key.queue-state.json"),projection_dir:dir.join("projection"),
        transcript:dir.join("chat.jsonl"),created:0.0 }
}

/// Cano que só confirma as escritas.
async fn quiet_cano() -> (String,tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (stream,_) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut raw = String::new(); reader.read_line(&mut raw).await.unwrap();
        let snapshot = mods_support::cano_snapshot_json();
        reader.get_mut().write_all(format!("{snapshot}\n").as_bytes()).await.unwrap();
        loop {
            raw.clear();
            if reader.read_line(&mut raw).await.unwrap_or(0) == 0 { break; }
            let envelope:serde_json::Value = serde_json::from_str(&raw).unwrap();
            let ack = json!({"type":"cano_input_ack","operation_id":envelope["operation_id"],"outcome":"written"});
            reader.get_mut().write_all(format!("{ack}\n").as_bytes()).await.unwrap();
        }
    });
    (format!("tcp:{address}"),task)
}

#[tokio::test]
async fn mods_call_before_attach_answers_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let (escuta,server) = quiet_cano().await;
    let target = claude_target(dir.path(),escuta,false);
    let lease = acquire_lease(&target.lease_path).unwrap();
    let store = Store::open(&target.state_path,&target.projection_dir,State::new("key",1,"session",vec![])).unwrap();
    let connection = cano::connect(&target.binding).await.unwrap();
    let engine = RuntimeEngine::new("claude",target.metadata.clone(),1,ClockSample { monotonic_s:0.0,epoch_s:1_800_000_000.0 }).unwrap()
        .with_mods(Mods::default());
    let handle = RuntimeActor::spawn(target,QueueActor::start(store,lease),connection,engine);
    let result = tokio::time::timeout(std::time::Duration::from_secs(2),handle.mods(ModsCall::Press { site:"above-prompt".into(),key:"k".into() })).await.unwrap();
    assert_eq!(result.unwrap_err().code,"erro_mod_botao_inexistente");
    handle.stop().await.unwrap();
    let after = handle.mods(ModsCall::Show { site:"p".into() }).await.unwrap_err();
    assert_eq!(after.code,"erro_mod_clique_sem_resposta","ator parado responde com código");
    server.abort();
}

/// Cano que responde os `ui_*` pela vitrine gravada, aceita várias conexões (reabertura) e anota os
/// ids dos pedidos. `swallow` lista subtipos que ficam sem resposta.
async fn vitrine_cano(swallow:&'static [&'static str]) -> (String,Arc<Mutex<Vec<String>>>,tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let ids = Arc::new(Mutex::new(Vec::new()));
    let seen = ids.clone();
    let task = tokio::spawn(async move {
        loop {
            let Ok((stream,_)) = listener.accept().await else { return };
            let seen = seen.clone();
            tokio::spawn(async move {
                let (read,mut write) = tokio::io::split(stream);
                let mut reader = BufReader::new(read);
                let mut raw = String::new();
                if reader.read_line(&mut raw).await.unwrap_or(0) == 0 || raw != "secret-test\n" { return; }
                let snapshot = mods_support::cano_snapshot_json();
                write.write_all(format!("{snapshot}\n").as_bytes()).await.unwrap();
                let mut fake = mods_support::FakeClaude::new("vitrine");
                loop {
                    raw.clear();
                    if reader.read_line(&mut raw).await.unwrap_or(0) == 0 { return; }
                    let envelope:serde_json::Value = serde_json::from_str(&raw).unwrap();
                    let ack = json!({"type":"cano_input_ack","operation_id":envelope["operation_id"],"outcome":"written"});
                    if write.write_all(format!("{ack}\n").as_bytes()).await.is_err() { return; }
                    let frame:serde_json::Value = serde_json::from_str(envelope["frame"].as_str().unwrap()).unwrap();
                    let subtype = frame["request"]["subtype"].as_str().unwrap_or("").to_owned();
                    if frame["type"] != "control_request" || !subtype.starts_with("ui_") { continue; }
                    seen.lock().unwrap().push(frame["request_id"].as_str().unwrap_or("").to_owned());
                    if swallow.contains(&subtype.as_str()) { continue; }
                    for line in fake.answer(&frame) {
                        let out = json!({"type":"cano_output","frame":line.to_string()});
                        if write.write_all(format!("{out}\n").as_bytes()).await.is_err() { return; }
                    }
                }
            });
        }
    });
    (format!("tcp:{address}"),ids,task)
}

async fn wait_ui(mods:&Mods,check:impl Fn(&serde_json::Value)->bool) -> serde_json::Value {
    tokio::time::timeout(std::time::Duration::from_secs(5),async {
        loop {
            if let Some(ui) = mods.replay("session").into_iter().find(|(event,_)|*event == "plugin_ui")
                .map(|(_,data)|serde_json::from_str::<serde_json::Value>(&data).unwrap()).filter(|ui|check(ui)) { return ui; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.expect("plugin_ui esperado")
}

fn registry(mods:&Mods) -> RuntimeRegistry {
    // Política num endereço sem ninguém: só serviços cosméticos a usam aqui.
    RuntimeRegistry::new("127.0.0.1:9".parse().unwrap(),"secret-test".into(),"instance-test".into()).with_mods(mods.clone())
}

#[tokio::test]
async fn surface_session_publishes_the_band_and_answers_a_press() {
    let dir = tempfile::tempdir().unwrap();
    let (escuta,_,server) = vitrine_cano(&[]).await;
    let mods = Mods::default();
    let registry = registry(&mods);
    registry.open(claude_target(dir.path(),escuta,true)).await.unwrap();
    assert!(mods.owns("session"));
    let band = wait_ui(&mods,|ui|ui["above"].to_string().contains("superfície desktop")).await;
    assert_eq!(band["source"],"surface");
    let (link,_) = mods.link("session").unwrap();
    let pressed = link.call(ModsCall::Press { site:"above-prompt".into(),key:"abrir-vitrine-botoes".into() }).await.unwrap();
    assert_eq!(pressed["element"],"abrir-vitrine-botoes");
    wait_ui(&mods,|ui|ui["panes"][0]["id"] == "vitrine-botoes").await;
    let journal = std::fs::read_to_string(dir.path().join("key.queue-state.json")).unwrap();
    assert!(!journal.contains("ui_attach") && !journal.contains("ui_render") && !journal.contains("ui_press"),"pedido da superfície não entra no diário");
    registry.close("key",1).await.unwrap();
    assert!(!mods.owns("session"),"fechar esquece a sessão (S9)");
    server.abort();
}

#[tokio::test]
async fn closing_the_session_answers_pending_calls() {
    let dir = tempfile::tempdir().unwrap();
    let (escuta,_,server) = vitrine_cano(&["ui_press"]).await;
    let mods = Mods::default();
    let registry = registry(&mods);
    registry.open(claude_target(dir.path(),escuta,true)).await.unwrap();
    wait_ui(&mods,|ui|!ui["above"].is_null()).await;
    let (link,_) = mods.link("session").unwrap();
    let pending = tokio::spawn(async move { link.call(ModsCall::Press { site:"above-prompt".into(),key:"abrir-vitrine-botoes".into() }).await });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    registry.close("key",1).await.unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(2),pending).await.unwrap().unwrap();
    assert_eq!(result.unwrap_err().code,"erro_mod_clique_sem_resposta");
    server.abort();
}

#[tokio::test]
async fn reopening_attaches_again_with_a_new_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let (escuta,ids,server) = vitrine_cano(&[]).await;
    let mods = Mods::default();
    let registry = registry(&mods);
    let target = claude_target(dir.path(),escuta,true);
    registry.open(target.clone()).await.unwrap();
    wait_ui(&mods,|ui|!ui["above"].is_null()).await;
    registry.close("key",1).await.unwrap();
    registry.open(target).await.unwrap();
    wait_ui(&mods,|ui|!ui["above"].is_null()).await;
    let prefixes:std::collections::BTreeSet<String> = ids.lock().unwrap().iter().map(|id|id.rsplit_once(':').unwrap().0.to_owned()).collect();
    assert_eq!(prefixes.len(),2,"cada vida do ator tem prefixo próprio: {prefixes:?}");
    registry.close("key",1).await.unwrap();
    server.abort();
}
