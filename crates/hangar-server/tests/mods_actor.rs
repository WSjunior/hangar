//! A interface dos mods no ator do runtime: pedido de app e escrita fora do diário.
mod mods_support;

use hangar_server::mods::model::ModsCall;
use hangar_server::mods::state::Mods;
use hangar_server::runtime::{actor::*,cano,protocol::*,queue::*};
use serde_json::json;
use tokio::io::{AsyncBufReadExt,AsyncWriteExt,BufReader};

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
    let target = RuntimeTarget { key:"key".into(),generation:1,name:"session".into(),provider:"claude".into(),
        metadata:json!({"name":"session","headless":true,"session_id":"sid-1","initialized":false}),
        binding:CanoBinding { pid:42,escuta,token:"secret-test".into(),versao:2 },
        lease_path:dir.path().join("key.lock"),state_path:dir.path().join("key.queue-state.json"),projection_dir:dir.path().join("projection"),
        transcript:dir.path().join("chat.jsonl"),created:0.0 };
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
