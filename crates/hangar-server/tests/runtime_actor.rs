use hangar_server::runtime::{actor::*,cano,protocol::*,queue::*};
use serde_json::{Value,json};
use tokio::io::{AsyncBufReadExt,AsyncWriteExt,BufReader};

async fn setup(reply_before_ack:bool) -> (RuntimeHandle,tokio::task::JoinHandle<usize>,tempfile::TempDir) {
    setup_behavior(reply_before_ack,false).await
}

async fn setup_behavior(reply_before_ack:bool,blocked_rpc:bool) -> (RuntimeHandle,tokio::task::JoinHandle<usize>,tempfile::TempDir) {
    setup_mode(reply_before_ack,blocked_rpc,false).await
}

async fn setup_mode(reply_before_ack:bool,blocked_rpc:bool,compound:bool) -> (RuntimeHandle,tokio::task::JoinHandle<usize>,tempfile::TempDir) {
    setup_recovered(reply_before_ack,blocked_rpc,compound,None,false).await
}

async fn setup_recovered(reply_before_ack:bool,blocked_rpc:bool,compound:bool,previous:Option<tempfile::TempDir>,late_reply:bool) -> (RuntimeHandle,tokio::task::JoinHandle<usize>,tempfile::TempDir) {
    let dir = previous.unwrap_or_else(||tempfile::tempdir().unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let state_path = dir.path().join("key.queue-state.json");
    let state_check = state_path.clone();
    let late_frame = if late_reply {
        let state:State = serde_json::from_slice(&std::fs::read(&state_path).unwrap()).unwrap();
        let request_id = state.operations.values().find(|phase|phase.payload["logical_id"] == "op-1").unwrap().payload["frame"]["id"].clone();
        Some(json!({"type":"cano_output","frame":json!({"id":request_id,"result":{"data":[]}}).to_string()}))
    } else { None };
    let server = tokio::spawn(async move {
        let (stream,_) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut header = String::new(); reader.read_line(&mut header).await.unwrap();
        assert_eq!(header,"secret-test\n");
        let snapshot = json!({"type":"cano_snapshot","versao":2,"pid":42,"init":null,"aberto":false,
            "pendentes":[],"ultimo_result":null,"rate_limit":null,"stderr_tail":[],"saiu":null,"inflight":{}});
        reader.get_mut().write_all(format!("{snapshot}\n").as_bytes()).await.unwrap();
        if let Some(frame) = late_frame { reader.get_mut().write_all(format!("{frame}\n").as_bytes()).await.unwrap(); }
        let mut sent = 0;
        loop {
            let mut raw = String::new();
            if reader.read_line(&mut raw).await.unwrap() == 0 { break; }
            let envelope:Value = serde_json::from_str(&raw).unwrap();
            let wire = envelope["operation_id"].as_str().unwrap();
            let state:State = serde_json::from_slice(&std::fs::read(&state_check).unwrap()).unwrap();
            assert!(state.operations[wire].status == Status::Dispatching,"journal precisa preceder os bytes");
            let frame:Value = serde_json::from_str(envelope["frame"].as_str().unwrap()).unwrap();
            sent += 1;
            let result = if compound && frame["method"] == "thread/read" {
                json!({"thread":{"id":"thread-1","status":{"type":"active"},"turns":[{"id":"turn-1","status":"inProgress"}]}})
            } else if frame["method"] == "turn/start" { json!({"turn":{"id":"turn-1","status":"inProgress"}}) }
            else { json!({"data":[]}) };
            let reply = json!({"type":"cano_output","frame":json!({"id":frame["id"],"result":result}).to_string()});
            let ack = json!({"type":"cano_input_ack","operation_id":wire,"outcome":"written"});
            if blocked_rpc && frame["method"] == "model/list" {
                continue;
            }
            let frames = if reply_before_ack { vec![reply,ack] } else { vec![ack,reply] };
            for frame in frames { reader.get_mut().write_all(format!("{frame}\n").as_bytes()).await.unwrap(); }
        }
        sent
    });
    let target = RuntimeTarget { key:"key".into(),generation:1,name:"session".into(),provider:"codex".into(),
        metadata:json!({"name":"session","headless":true,"thread_id":"thread-1","initialized":true,"ready":true}),
        binding:CanoBinding { pid:42,escuta:format!("tcp:{address}"),token:"secret-test".into(),versao:2 },
        lease_path:dir.path().join("key.lock"),state_path:state_path.clone(),projection_dir:dir.path().join("projection"),
        transcript:dir.path().join("chat.jsonl"),created:0.0 };
    let lease = acquire_lease(&target.lease_path).unwrap();
    let store = Store::open(&target.state_path,&target.projection_dir,State::new("key",1,"session",vec![])).unwrap();
    let queue = QueueActor::start(store,lease);
    let connection = cano::connect(&target.binding).await.unwrap();
    let engine = RuntimeEngine::new("codex",target.metadata.clone(),1,ClockSample { monotonic_s:0.0,epoch_s:1_800_000_000.0 }).unwrap();
    let handle = RuntimeActor::spawn(target,queue,connection,engine);
    (handle,server,dir)
}

fn command() -> RuntimeCommand { RuntimeCommand { operation_id:"op-1".into(),kind:OperationKind::ListModels,payload:json!({}) } }

#[tokio::test]
async fn no_sse_runtime_still_drains() {
    let (handle,server,dir) = setup(true).await;
    handle.queue("append".into(),Action::Append { text:"Olá".into(),delivered:false,ts:None,
        pre_transcript:false,entry_id:Some("input-entry".into()) }).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5),async {
        loop {
            let state:State = serde_json::from_slice(&std::fs::read(dir.path().join("key.queue-state.json")).unwrap()).unwrap();
            if state.operations.get("input-entry").is_some_and(|operation|operation.status == Status::Accepted) { break; }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    handle.stop().await.unwrap();
    assert_eq!(server.await.unwrap(),1);
}

#[tokio::test]
async fn confirmed_prompt_does_not_consume_next_echo() {
    use hangar_server::runtime::receipt::ReceiptIndex;
    let (handle,server,dir) = setup(true).await;
    let path = dir.path().join("chat.jsonl");
    std::fs::write(&path, "").unwrap();
    for id in ["first", "second"] {
        let cursor = ReceiptIndex::new("codex", "thread-1").capture(&path).unwrap();
        handle.queue(format!("{id}:append"),Action::Append { text:"Olá".into(),delivered:true,ts:None,
            pre_transcript:false,entry_id:Some(id.into()) }).await.unwrap();
        handle.queue(format!("{id}:prepare"),Action::Prepare { id:id.into(),entry_id:Some(id.into()),
            payload:json!({"kind":"input"}) }).await.unwrap();
        handle.queue(format!("{id}:cursor"),Action::BindDispatch { id:id.into(),cursor:serde_json::to_value(cursor).unwrap() }).await.unwrap();
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file,"{}",json!({"type":"response_item", "payload":{"type":"message", "role":"user",
            "content":[{"type":"input_text","text":"Olá"}]}})).unwrap();
        assert_eq!(handle.confirm().await.unwrap()["confirmed"],1);
    }
    let state:State = serde_json::from_slice(&std::fs::read(dir.path().join("key.queue-state.json")).unwrap()).unwrap();
    assert!(state.rows.iter().all(|row|row["confirmed"] == true));
    // As duas confirmadas: não resta operação que possa casar os ecos, e o uso sai da poda.
    assert!(state.used_occurrences.is_empty());
    handle.stop().await.unwrap();
    assert_eq!(server.await.unwrap(),0);
}

#[tokio::test]
async fn resubmit_after_prune_of_confirmed_row_does_not_send_again() {
    let input = ||RuntimeCommand { operation_id:"msg".into(),kind:OperationKind::Input,payload:json!({"text":"Olá","entry_id":"msg"}) };
    let (handle,server,dir) = setup(true).await;
    assert!(handle.command(input()).await.unwrap().disposition == Disposition::Accepted);
    handle.queue("confirm".into(),Action::Confirm { entry_ids:vec!["msg".into()] }).await.unwrap();
    for index in 0..300 { handle.queue(format!("fill:{index}"),Action::SetRuntimeState { state:json!({}) }).await.unwrap(); }
    handle.stop().await.unwrap();
    assert_eq!(server.await.unwrap(),1);
    let state:State = serde_json::from_slice(&std::fs::read(dir.path().join("key.queue-state.json")).unwrap()).unwrap();
    assert!(!state.operations.contains_key("msg"));
    // Ator novo, sem a resposta guardada: a linha confirmada responde e nada vai ao fio.
    let (handle,server,_dir) = setup_recovered(true,false,false,Some(dir),false).await;
    assert!(handle.command(input()).await.unwrap().disposition == Disposition::Accepted);
    handle.stop().await.unwrap();
    assert_eq!(server.await.unwrap(),0);
}

#[tokio::test]
async fn prepare_before_every_write_and_cli_reply_before_ack_is_final() {
    let (handle,server,dir) = setup(true).await;
    let result = handle.command(command()).await.unwrap();
    assert!(result.disposition == Disposition::Accepted);
    handle.stop().await.unwrap();
    assert_eq!(server.await.unwrap(),1);
    let state:State = serde_json::from_slice(&std::fs::read(dir.path().join("key.queue-state.json")).unwrap()).unwrap();
    let phase = state.operations.values().find(|operation|operation.payload["logical_id"] == "op-1").unwrap();
    assert_eq!(phase.result["disposition"],"accepted");
}

#[tokio::test]
async fn concurrent_same_id_has_one_dispatch() {
    let (handle,server,_dir) = setup(false).await;
    let (first,second) = tokio::join!(handle.command(command()),handle.command(command()));
    assert!(first.unwrap().disposition == Disposition::Accepted);
    assert!(second.unwrap().disposition == Disposition::Accepted);
    handle.stop().await.unwrap();
    assert_eq!(server.await.unwrap(),1);
}

#[tokio::test]
async fn stop_joins_io_and_persistence_before_unlock() {
    let (handle,server,dir) = setup(false).await;
    handle.command(command()).await.unwrap();
    handle.stop().await.unwrap();
    server.await.unwrap();
    let _next = acquire_lease(&dir.path().join("key.lock")).unwrap();
}

#[tokio::test]
async fn published_view_has_durable_state() {
    let (handle,server,dir) = setup(false).await;
    let mut events = handle.subscribe();
    handle.command(command()).await.unwrap();
    while let Ok(event) = events.try_recv() {
        if event.channel == "view" {
            let state:State = serde_json::from_slice(&std::fs::read(dir.path().join("key.queue-state.json")).unwrap()).unwrap();
            assert!(state.runtime_state["view"].is_object());
        }
    }
    handle.stop().await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn operation_id_cannot_be_reused_with_another_payload() {
    let (handle,server,_dir) = setup(false).await;
    handle.command(command()).await.unwrap();
    let mut changed = command();
    changed.payload = json!({"limit":7});
    let Err(error) = handle.command(changed).await else { panic!("ID reutilizada precisa falhar") };
    assert_eq!(error.code,"operation_reused");
    handle.stop().await.unwrap();
    assert_eq!(server.await.unwrap(),1);
}

#[tokio::test]
async fn blocked_rpc_does_not_block_interrupt_or_state() {
    let (handle,server,dir) = setup_behavior(false,true).await;
    let request = handle.clone();
    let pending = tokio::spawn(async move { request.command(command()).await });
    tokio::time::timeout(std::time::Duration::from_secs(5),async {
        loop {
            let state:State = serde_json::from_slice(&std::fs::read(dir.path().join("key.queue-state.json")).unwrap()).unwrap();
            if state.operations.contains_key("op-1") { break; }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5),handle.command(RuntimeCommand {
        operation_id:"interrupt".into(),kind:OperationKind::Interrupt,payload:json!({}) })).await.unwrap().unwrap();
    assert!(result.disposition == Disposition::Accepted);
    assert!(handle.snapshot().await.unwrap()["view"].is_object());
    handle.stop().await.unwrap();
    assert!(pending.await.unwrap().is_err());
    assert_eq!(server.await.unwrap(),2);
}

#[tokio::test]
async fn queue_failure_prevents_write() {
    let (handle,server,dir) = setup(false).await;
    let path = dir.path().join("key.queue-state.json");
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(handle.command(command()).await.is_err());
    assert!(handle.stop().await.is_err());
    assert_eq!(server.await.unwrap(),0);
}

#[tokio::test]
async fn ack_timeout_is_unknown_without_resend() {
    let (handle,server,dir) = setup_behavior(false,true).await;
    let result = handle.command(command()).await.unwrap();
    assert!(result.disposition == Disposition::Unknown);
    let again = handle.command(command()).await.unwrap();
    assert!(again.disposition == Disposition::Unknown);
    let state:State = serde_json::from_slice(&std::fs::read(dir.path().join("key.queue-state.json")).unwrap()).unwrap();
    assert!(state.operations["op-1"].status == Status::Unknown);
    handle.stop().await.unwrap();
    assert_eq!(server.await.unwrap(),1);
}

#[tokio::test]
async fn wire_ids_distinguish_compound_control() {
    let (handle,server,dir) = setup_mode(false,false,true).await;
    let result = handle.command(RuntimeCommand { operation_id:"interrupt".into(),kind:OperationKind::Interrupt,payload:json!({}) }).await.unwrap();
    assert!(result.disposition == Disposition::Accepted);
    handle.stop().await.unwrap();
    assert_eq!(server.await.unwrap(),2);
    let state:State = serde_json::from_slice(&std::fs::read(dir.path().join("key.queue-state.json")).unwrap()).unwrap();
    let phases:Vec<_> = state.operations.values().filter(|operation|operation.payload["logical_id"] == "interrupt:read").collect();
    assert_eq!(phases.len(),1);
    assert_ne!(phases[0].id,"interrupt");
    assert!(state.operations["interrupt"].status == Status::Accepted);
    let final_phase = state.operations.values().find(|operation|operation.payload["logical_id"] == "interrupt").unwrap();
    assert_ne!(final_phase.id,phases[0].id);
}

#[test]
fn unknown_preparation_never_starts_new_mutable_phase() {
    use hangar_server::runtime::codex::Engine;
    let sample = ClockSample { monotonic_s:0.0,epoch_s:1_800_000_000.0 };
    let mut engine = Engine::new(json!({"name":"session","headless":true,"thread_id":"thread-1","ready":true,"initialized":true}),1,sample);
    engine.restore_rpc("mode:settings".into(),&json!({"id":"hangar:1:7","method":"thread/read",
        "params":{"threadId":"thread-1","includeTurns":false}}),0,0);
    let effects = engine.apply(EngineInput::Line(json!({"id":"hangar:1:7","result":{"thread":{"id":"thread-1","model":"test-model"}}})),sample).unwrap();
    assert!(!effects.iter().any(|effect|matches!(effect,Effect::Write { .. })));
    assert!(effects.iter().any(|effect|matches!(effect,Effect::Reply { operation_id,disposition:Disposition::Accepted,.. } if operation_id == "mode:settings")));
}

#[tokio::test]
async fn late_wire_reply_resolves_parent() {
    let (handle,server,dir) = setup_behavior(false,true).await;
    assert!(handle.command(command()).await.unwrap().disposition == Disposition::Unknown);
    handle.stop().await.unwrap();
    assert_eq!(server.await.unwrap(),1);
    let (handle,server,_dir) = setup_recovered(false,false,false,Some(dir),true).await;
    tokio::time::timeout(std::time::Duration::from_secs(5),async {
        loop {
            if handle.command(command()).await.unwrap().disposition == Disposition::Accepted { break; }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    handle.stop().await.unwrap();
    assert_eq!(server.await.unwrap(),0);
}
