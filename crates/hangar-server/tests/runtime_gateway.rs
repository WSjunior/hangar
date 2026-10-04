use hangar_server::runtime::gateway::{self,RuntimeRegistry};
use std::sync::Arc;

#[tokio::test]
async fn wrong_secret_instance_or_generation_denied() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let registry = Arc::new(RuntimeRegistry::new("127.0.0.1:9".parse().unwrap(),"secret-test".into(),"instance-test".into()));
    let server = tokio::spawn(gateway::serve(listener,registry,"secret-test".into(),"instance-test".into(),hangar_server::INTERNAL_PROTOCOL));
    let client = reqwest::Client::new();
    for (secret,instance) in [("wrong","instance-test"),("secret-test","wrong")] {
        let response = client.post(format!("http://{address}/runtime/op"))
            .header("x-hangar-internal",secret).header("x-hangar-runtime-instance",instance)
            .body("not-json").send().await.unwrap();
        assert_eq!(response.status(),404);
    }
    server.abort();
}

#[tokio::test]
async fn private_port_loopback_only() {
    let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
    let registry = Arc::new(RuntimeRegistry::new("127.0.0.1:9".parse().unwrap(),"secret-test".into(),"instance-test".into()));
    assert!(gateway::serve(listener,registry,"secret-test".into(),"instance-test".into(),hangar_server::INTERNAL_PROTOCOL).await.is_err());
}

#[test]
fn startup_one_line_no_secret() {
    let line = gateway::startup_line(hangar_server::INTERNAL_PROTOCOL,"instance-test",1234);
    assert!(!line.contains("secret-test"));
    let value:serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(value["type"],"runtime_ready");
    assert_eq!(value["port"],1234);
    assert_eq!(value.as_object().unwrap().len(),4);
}

#[tokio::test]
async fn unknown_command_fields_are_rejected() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let registry = Arc::new(RuntimeRegistry::new("127.0.0.1:9".parse().unwrap(),"secret-test".into(),"instance-test".into()));
    let server = tokio::spawn(gateway::serve(listener,registry,"secret-test".into(),"instance-test".into(),hangar_server::INTERNAL_PROTOCOL));
    let response = reqwest::Client::new().post(format!("http://{address}/runtime/op"))
        .header("x-hangar-internal","secret-test").header("x-hangar-runtime-instance","instance-test")
        .header("content-type","application/json")
        .body(serde_json::json!({"protocol":hangar_server::INTERNAL_PROTOCOL,"instance":"instance-test","key":"key",
            "generation":1,"operation_id":"op","clock":{"monotonic_s":0.0,"epoch_s":0.0},
            "command":{"kind":"detach","unexpected":true}}).to_string()).send().await.unwrap();
    assert!(!response.status().is_success());
    server.abort();
}

/// Registro com um cano Claude falso que só aceita entradas; a política aponta para uma porta fechada.
async fn adopted(dir:&std::path::Path) -> (RuntimeRegistry,serde_json::Value,tokio::task::JoinHandle<()>) {
    use hangar_server::runtime::protocol::*;
    use serde_json::json;
    use tokio::io::{AsyncBufReadExt,AsyncWriteExt,BufReader};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let cano = tokio::spawn(async move {
        let (stream,_) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut header = String::new(); reader.read_line(&mut header).await.unwrap();
        let snapshot = json!({"type":"cano_snapshot","versao":2,"pid":42,"init":null,"aberto":false,
            "pendentes":[],"ultimo_result":null,"rate_limit":null,"stderr_tail":[],"saiu":null,"inflight":{}});
        reader.get_mut().write_all(format!("{snapshot}\n").as_bytes()).await.unwrap();
        let mut raw = String::new();
        while reader.read_line(&mut raw).await.unwrap_or(0) > 0 { raw.clear(); }
    });
    let registry = RuntimeRegistry::new("127.0.0.1:9".parse().unwrap(),"secret-test".into(),"instance-test".into());
    let target = RuntimeTarget { key:"key".into(),generation:1,name:"session".into(),provider:"claude".into(),
        metadata:json!({"name":"session","headless":true,"session_id":"sid-1","initialized":true}),
        binding:CanoBinding { pid:42,escuta:format!("tcp:{address}"),token:"secret-test".into(),versao:2 },
        lease_path:dir.join("key.lock"),state_path:dir.join("key.queue-state.json"),projection_dir:dir.join("projection"),
        transcript:dir.join("chat.jsonl"),created:0.0 };
    let ready = registry.adopt(target,json!({})).await.unwrap();
    (registry,ready,cano)
}

#[tokio::test]
async fn a_failing_status_service_does_not_stop_the_session() {
    // O carimbo e a linha de status vêm do Python; sem ele a sessão perde só isso, não a posse.
    let dir = tempfile::tempdir().unwrap();
    let (registry,ready,cano) = adopted(dir.path()).await;
    assert_eq!(ready["ready"],true);
    registry.detach("key",1).await.unwrap();
    cano.abort();
}

#[tokio::test]
async fn an_actor_that_ended_with_an_error_is_released_and_leaves_the_others_alone() {
    // Antes, a entrada morta ficava registrada: o detach falhava para sempre, a sessão não voltava
    // ao Python e o retrato inicial de eventos (de todas as sessões) respondia erro.
    let dir = tempfile::tempdir().unwrap();
    let (registry,_,cano) = adopted(dir.path()).await;
    // A parada falha ao reparar a projeção: o ator termina com erro.
    std::fs::remove_dir_all(dir.path().join("projection")).unwrap();
    std::fs::write(dir.path().join("projection"),"").unwrap();
    let detached = registry.detach("key",1).await.unwrap();
    assert_eq!(detached["detached"],true);
    assert!(registry.snapshots().await.unwrap().is_empty());
    let _lease = hangar_server::runtime::queue::acquire_lease(&dir.path().join("key.lock")).unwrap();
    cano.abort();
}

#[tokio::test]
async fn one_bad_message_from_the_cli_does_not_end_the_actor() {
    use hangar_server::runtime::protocol::*;
    use serde_json::json;
    use tokio::io::{AsyncBufReadExt,AsyncWriteExt,BufReader};
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let cano = tokio::spawn(async move {
        let (stream,_) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut header = String::new(); reader.read_line(&mut header).await.unwrap();
        let snapshot = json!({"type":"cano_snapshot","versao":2,"pid":42,"init":null,"aberto":false,
            "pendentes":[],"ultimo_result":null,"rate_limit":null,"stderr_tail":[],"saiu":null,"inflight":{}});
        let bad = json!({"type":"cano_output","frame":json!({"type":"control_response","response":{"request_id":null}}).to_string()});
        reader.get_mut().write_all(format!("{snapshot}\n{bad}\n").as_bytes()).await.unwrap();
        let mut raw = String::new();
        while reader.read_line(&mut raw).await.unwrap_or(0) > 0 { raw.clear(); }
    });
    let registry = RuntimeRegistry::new("127.0.0.1:9".parse().unwrap(),"secret-test".into(),"instance-test".into());
    let target = RuntimeTarget { key:"key".into(),generation:1,name:"session".into(),provider:"claude".into(),
        metadata:json!({"name":"session","headless":true,"session_id":"sid-1","initialized":true}),
        binding:CanoBinding { pid:42,escuta:format!("tcp:{address}"),token:"secret-test".into(),versao:2 },
        lease_path:dir.path().join("key.lock"),state_path:dir.path().join("key.queue-state.json"),projection_dir:dir.path().join("projection"),
        transcript:dir.path().join("chat.jsonl"),created:0.0 };
    registry.adopt(target,json!({})).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(registry.snapshots().await.unwrap().len(),1,"o ator segue vivo depois da mensagem ruim");
    registry.detach("key",1).await.unwrap();
    cano.abort();
}
