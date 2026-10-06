mod mods_support;

use hangar_server::runtime::{cano, protocol::*};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::test]
async fn cli_cannot_forge_private_ack() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut token = String::new();
        reader.read_line(&mut token).await.unwrap();
        assert_eq!(token, "secret-test\n");
        let snapshot = mods_support::cano_snapshot_json();
        reader.get_mut().write_all(format!("{snapshot}\n").as_bytes()).await.unwrap();
        let event = json!({"type":"cano_output","frame":
            json!({"type":"cano_input_ack","operation_id":"wire:1","outcome":"written"}).to_string()});
        reader.get_mut().write_all(format!("{event}\n").as_bytes()).await.unwrap();
    });
    // Como na vida real: o sidecar guarda o pid do cano, e o snapshot traz o pid do agente filho.
    let binding = CanoBinding { pid:41, escuta:format!("tcp:{address}"), token:"secret-test".into(), versao:2 };
    let connection = cano::connect(&binding).await.unwrap();
    let mut io = connection.start(1, 16);
    let event = io.events.recv().await.unwrap();
    assert!(matches!(event, cano::IoEvent::Line(v) if v["type"] == "cano_input_ack"));
    io.stop().await;
    server.await.unwrap();
}

#[tokio::test]
async fn v1_is_ineligible_without_connecting() {
    let binding = CanoBinding { pid:42, escuta:"tcp:127.0.0.1:1".into(), token:"secret-test".into(), versao:1 };
    assert!(cano::connect(&binding).await.is_err_and(|error|error.code == "cano_version"));
}

#[tokio::test]
async fn malformed_cli_message_keeps_following_ack_and_event() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream,_) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut raw = String::new(); reader.read_line(&mut raw).await.unwrap();
        let snapshot = mods_support::cano_snapshot_json();
        reader.get_mut().write_all(format!("{snapshot}\n").as_bytes()).await.unwrap();
        raw.clear(); reader.read_line(&mut raw).await.unwrap();
        for frame in ["warning from CLI", "[]"] {
            let event = json!({"type":"cano_output","frame":frame});
            reader.get_mut().write_all(format!("{event}\n").as_bytes()).await.unwrap();
        }
        let ack = json!({"type":"cano_input_ack","operation_id":"wire:1","outcome":"written"});
        let event = json!({"type":"cano_output","frame":json!({"type":"result"}).to_string()});
        reader.get_mut().write_all(format!("{ack}\n{event}\n").as_bytes()).await.unwrap();
    });
    let binding = CanoBinding { pid:42,escuta:format!("tcp:{address}"),token:"secret-test".into(),versao:2 };
    let mut io = cano::connect(&binding).await.unwrap().start(1,16);
    io.writer.send(cano::WireFrame { operation_id:"wire:1".into(),frame:json!({"type":"user"}),ephemeral:false }).await.unwrap();
    let next = tokio::time::timeout(std::time::Duration::from_secs(2),io.events.recv()).await.unwrap().unwrap();
    assert!(matches!(next,cano::IoEvent::WriteAck { outcome:WriteOutcome::Written,.. }));
    assert!(matches!(io.events.recv().await.unwrap(),cano::IoEvent::Line(value) if value["type"] == "result"));
    io.stop().await; server.await.unwrap();
}

#[tokio::test]
async fn invalid_envelope_reports_reader_end() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream,_) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut raw = String::new(); reader.read_line(&mut raw).await.unwrap();
        let snapshot = mods_support::cano_snapshot_json();
        reader.get_mut().write_all(format!("{snapshot}\n{{\"type\":\"invalid_envelope\"}}\n").as_bytes()).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    });
    let binding = CanoBinding { pid:42,escuta:format!("tcp:{address}"),token:"secret-test".into(),versao:2 };
    let mut io = cano::connect(&binding).await.unwrap().start(1,16);
    let next = tokio::time::timeout(std::time::Duration::from_secs(1),io.events.recv()).await.unwrap();
    assert!(matches!(next,Some(cano::IoEvent::End { .. })));
    io.stop().await; server.abort();
}

#[tokio::test]
async fn ephemeral_frames_are_not_remembered_by_the_writer() {
    // Pedido da interface dos mods sai a cada desenho: o escritor não guarda o id (a lista cresceria
    // sem fim) e um id repetido ainda sai; o de operação da fila continua sem repetir.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream,_) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut raw = String::new(); reader.read_line(&mut raw).await.unwrap();
        let snapshot = mods_support::cano_snapshot_json();
        reader.get_mut().write_all(format!("{snapshot}\n").as_bytes()).await.unwrap();
        let mut ids = Vec::new();
        for _ in 0..3 {
            raw.clear(); reader.read_line(&mut raw).await.unwrap();
            ids.push(serde_json::from_str::<serde_json::Value>(&raw).unwrap()["operation_id"].as_str().unwrap().to_owned());
        }
        ids
    });
    let binding = CanoBinding { pid:42,escuta:format!("tcp:{address}"),token:"secret-test".into(),versao:2 };
    let io = cano::connect(&binding).await.unwrap().start(1,16);
    for (id,ephemeral) in [("ui:1:1",true),("ui:1:1",true),("wire:1",false),("wire:1",false)] {
        io.writer.send(cano::WireFrame { operation_id:id.into(),frame:json!({"type":"control_request"}),ephemeral }).await.unwrap();
    }
    let ids = tokio::time::timeout(std::time::Duration::from_secs(2),server).await.unwrap().unwrap();
    assert_eq!(ids,["ui:1:1","ui:1:1","wire:1"]);
    io.stop().await;
}
