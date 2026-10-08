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
async fn row_delivered_before_the_runtime_is_confirmed_once_by_a_later_echo() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chat.jsonl");
    let echo = |text:&str| json!({"timestamp":"2026-10-06T12:00:00.000Z","type":"response_item","payload":{"type":"message",
        "role":"user","content":[{"type":"input_text","text":text}]}}).to_string();
    std::fs::write(&path, format!("{}\n{}\n{}\n",echo("sim"),echo("velha"),echo("continua"))).unwrap();
    // Entregues pelo Python antes da troca: sem operação nem cursor de despacho, já no estado quando
    // o ator sobe. Inseridas com ele vivo, a rodada do primeiro ocioso confirmaria "a" sem ver "b".
    // Desistida não chegou: um "continua" digitado depois não a dá por entregue.
    let rows = [("a","sim",1791287980.0,false),("b","sim",1791287995.0,false),("c","velha",1791290000.0,false),("d","continua",1791287990.0,true)]
        .map(|(id,text,ts,abandoned)|{
            let mut row = json!({"id":id,"text":text,"ts":ts,"delivered":true});
            if abandoned { row["desistiu"] = json!(true); }
            row
        });
    std::fs::write(dir.path().join("key.queue-state.json"),serde_json::to_vec(&State::new("key",1,"session",rows.to_vec())).unwrap()).unwrap();
    let (handle,server,dir) = setup_recovered(true,false,false,Some(dir),false).await;
    let confirmed = || {
        let state:State = serde_json::from_slice(&std::fs::read(dir.path().join("key.queue-state.json")).unwrap()).unwrap();
        state.rows.iter().filter(|row|row["confirmed"] == true).filter_map(|row|row["id"].as_str().map(str::to_owned)).collect::<Vec<_>>()
    };
    // A rodada do primeiro ocioso corre junto desta: a contagem é de quem gravou primeiro, o estado não.
    handle.confirm().await.unwrap();
    // A única linha "sim" é do envio mais recente (b), não do perdido (a). A linha "velha" foi
    // gravada antes do envio de "c": não prova a entrega dele.
    assert_eq!(confirmed(),["b"]);
    for index in 0..300 { handle.queue(format!("fill:{index}"),Action::SetRuntimeState { state:json!({}) }).await.unwrap(); }
    // Compactada a fila, a linha usada continua gasta: o outro "sim" não a reaproveita.
    assert_eq!(handle.confirm().await.unwrap()["confirmed"],0);
    assert_eq!(confirmed(),["b"]);
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

/// Cano Claude falso que só conta o que chega ao fio; a política aponta para uma porta fechada.
async fn setup_claude_unreachable_policy(initialized:bool) -> (RuntimeHandle,tokio::task::JoinHandle<usize>,tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream,_) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut header = String::new(); reader.read_line(&mut header).await.unwrap();
        let snapshot = json!({"type":"cano_snapshot","versao":2,"pid":42,"init":null,"aberto":false,
            "pendentes":[],"ultimo_result":null,"rate_limit":null,"stderr_tail":[],"saiu":null,"inflight":{}});
        reader.get_mut().write_all(format!("{snapshot}\n").as_bytes()).await.unwrap();
        let mut sent = 0;
        loop {
            let mut raw = String::new();
            if reader.read_line(&mut raw).await.unwrap() == 0 { break; }
            sent += 1;
            let envelope:Value = serde_json::from_str(&raw).unwrap();
            let ack = json!({"type":"cano_input_ack","operation_id":envelope["operation_id"],"outcome":"written"});
            reader.get_mut().write_all(format!("{ack}\n").as_bytes()).await.unwrap();
        }
        sent
    });
    let target = RuntimeTarget { key:"key".into(),generation:1,name:"session".into(),provider:"claude".into(),
        metadata:json!({"name":"session","headless":true,"session_id":"sid-1","initialized":initialized}),
        binding:CanoBinding { pid:42,escuta:format!("tcp:{address}"),token:"secret-test".into(),versao:2 },
        lease_path:dir.path().join("key.lock"),state_path:dir.path().join("key.queue-state.json"),projection_dir:dir.path().join("projection"),
        transcript:dir.path().join("chat.jsonl"),created:0.0 };
    let lease = acquire_lease(&target.lease_path).unwrap();
    let store = Store::open(&target.state_path,&target.projection_dir,State::new("key",1,"session",vec![])).unwrap();
    let queue = QueueActor::start(store,lease);
    let connection = cano::connect(&target.binding).await.unwrap();
    let engine = RuntimeEngine::new("claude",target.metadata.clone(),1,ClockSample { monotonic_s:0.0,epoch_s:1_800_000_000.0 }).unwrap()
        .with_policy(PolicyClient::new(closed,"secret".into(),"instance".into()));
    (RuntimeActor::spawn(target,queue,connection,engine),server,dir)
}

#[tokio::test]
async fn input_is_prepared_in_rust_even_when_the_python_policy_is_unreachable() {
    // O preparo do prompt é local: o Python fora do ar não adia mais a entrada.
    let (handle,server,dir) = setup_claude_unreachable_policy(true).await;
    let input = RuntimeCommand { operation_id:"msg".into(),kind:OperationKind::Input,payload:json!({"text":"Olá","entry_id":"msg"}) };
    assert!(handle.command(input).await.unwrap().disposition == Disposition::Accepted);
    let state:State = serde_json::from_slice(&std::fs::read(dir.path().join("key.queue-state.json")).unwrap()).unwrap();
    assert!(!state.operations.keys().any(|id|id.contains("prepare_prompt")),"cálculo puro não entra no diário");
    handle.stop().await.unwrap();
    assert_eq!(server.await.unwrap(),1,"a mensagem chegou ao fio");
}

#[tokio::test]
async fn turn_end_confirms_the_delivered_input_without_being_asked() {
    // O adapter Python confirmava a fila em todo fim de turno; o ator faz o mesmo sozinho.
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("chat.jsonl");
    std::fs::write(&transcript,"").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let written = transcript.clone();
    let server = tokio::spawn(async move {
        let (stream,_) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut header = String::new(); reader.read_line(&mut header).await.unwrap();
        let snapshot = json!({"type":"cano_snapshot","versao":2,"pid":42,"init":null,"aberto":false,
            "pendentes":[],"ultimo_result":null,"rate_limit":null,"stderr_tail":[],"saiu":null,"inflight":{}});
        reader.get_mut().write_all(format!("{snapshot}\n").as_bytes()).await.unwrap();
        loop {
            let mut raw = String::new();
            if reader.read_line(&mut raw).await.unwrap() == 0 { break; }
            let envelope:Value = serde_json::from_str(&raw).unwrap();
            let frame:Value = serde_json::from_str(envelope["frame"].as_str().unwrap()).unwrap();
            let ack = json!({"type":"cano_input_ack","operation_id":envelope["operation_id"],"outcome":"written"});
            reader.get_mut().write_all(format!("{ack}\n").as_bytes()).await.unwrap();
            if frame["type"] == "user" {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new().append(true).open(&written).unwrap();
                writeln!(file,"{}",json!({"type":"user","uuid":"u-1","message":{"role":"user","content":"Olá"}})).unwrap();
                let result = json!({"type":"cano_output","frame":json!({"type":"result","subtype":"success","is_error":false}).to_string()});
                reader.get_mut().write_all(format!("{result}\n").as_bytes()).await.unwrap();
            }
        }
    });
    let target = RuntimeTarget { key:"key".into(),generation:1,name:"session".into(),provider:"claude".into(),
        metadata:json!({"name":"session","headless":true,"session_id":"sid-1","initialized":true}),
        binding:CanoBinding { pid:42,escuta:format!("tcp:{address}"),token:"secret-test".into(),versao:2 },
        lease_path:dir.path().join("key.lock"),state_path:dir.path().join("key.queue-state.json"),projection_dir:dir.path().join("projection"),
        transcript:transcript.clone(),created:0.0 };
    let lease = acquire_lease(&target.lease_path).unwrap();
    let store = Store::open(&target.state_path,&target.projection_dir,State::new("key",1,"session",vec![])).unwrap();
    let queue = QueueActor::start(store,lease);
    let connection = cano::connect(&target.binding).await.unwrap();
    let engine = RuntimeEngine::new("claude",target.metadata.clone(),1,ClockSample { monotonic_s:0.0,epoch_s:1_800_000_000.0 }).unwrap();
    let handle = RuntimeActor::spawn(target,queue,connection,engine);
    let input = RuntimeCommand { operation_id:"msg".into(),kind:OperationKind::Input,payload:json!({"text":"Olá","entry_id":"msg"}) };
    assert!(handle.command(input).await.unwrap().disposition == Disposition::Accepted);
    let path = dir.path().join("key.queue-state.json");
    tokio::time::timeout(std::time::Duration::from_secs(5),async {
        loop {
            let state:State = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            if state.rows.iter().any(|row|row["id"] == "msg" && row["confirmed"] == true) { break; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.expect("fim de turno precisa confirmar a entrada entregue");
    handle.stop().await.unwrap();
    server.await.unwrap();
}

/// Serviço de política HTTP que responde `ok` a tudo e conta as chamadas.
async fn policy_server() -> (std::net::SocketAddr,std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use tokio::io::AsyncReadExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = calls.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream,_)) = listener.accept().await else { return };
            let counter = counter.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stream);
                loop {
                    let mut length = 0usize;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).await.unwrap_or(0) == 0 { return; }
                        if line == "\r\n" { break; }
                        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") { length = value.trim().parse().unwrap(); }
                    }
                    let mut body = vec![0;length]; reader.read_exact(&mut body).await.unwrap();
                    // GET /internal/quota (sem corpo) não é uma política: responde sem contar.
                    let reply = if body.is_empty() { json!({"windows":[]}).to_string() } else {
                        counter.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
                        let kind = serde_json::from_slice::<Value>(&body).unwrap()["kind"].clone();
                        let data = if kind == "prepare_prompt" { json!({"content":"Olá","notices":[],"native_candidate":false}) } else { json!({}) };
                        json!({"ok":true,"data":data}).to_string()
                    };
                    let response = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{reply}",reply.len());
                    reader.get_mut().write_all(response.as_bytes()).await.unwrap();
                }
            });
        }
    });
    (address,calls)
}

#[tokio::test]
async fn status_formatting_and_state_changes_do_not_rewrite_the_journal() {
    // Uma mensagem custava ~60 regravações do estado: cada format_status passava pelo diário e
    // cada mudança de estado gravava a vista inteira, mesmo sem nada durável mudar.
    status_turn(false).await;
}

#[tokio::test]
async fn a_local_format_failure_is_cosmetic_and_leaves_the_session_alive() {
    // `rate_limit_info` que não é objeto faz o formatador local falhar (o Python também falhava):
    // a sessão só perde a linha de status e o turno termina.
    let problems = status_turn(true).await;
    assert!(problems.iter().any(|problem|problem["error_code"] == "policy_input"),"a falha precisa aparecer: {problems:?}");
}

/// Um turno completo com o cano falso; devolve os `problem` publicados. `bad_rate` manda um
/// `rate_limit_event` com `rate_limit_info` inválido antes do resultado.
async fn status_turn(bad_rate:bool) -> Vec<Value> {
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("chat.jsonl");
    std::fs::write(&transcript,"").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream,_) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut header = String::new(); reader.read_line(&mut header).await.unwrap();
        let snapshot = json!({"type":"cano_snapshot","versao":2,"pid":42,"init":null,"aberto":false,
            "pendentes":[],"ultimo_result":null,"rate_limit":null,"stderr_tail":[],"saiu":null,"inflight":{}});
        reader.get_mut().write_all(format!("{snapshot}\n").as_bytes()).await.unwrap();
        loop {
            let mut raw = String::new();
            if reader.read_line(&mut raw).await.unwrap() == 0 { break; }
            let envelope:Value = serde_json::from_str(&raw).unwrap();
            let ack = json!({"type":"cano_input_ack","operation_id":envelope["operation_id"],"outcome":"written"});
            reader.get_mut().write_all(format!("{ack}\n").as_bytes()).await.unwrap();
            let mut events = vec![json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"m1"}}}),
                json!({"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}),
                json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"oi"}}}),
                json!({"type":"stream_event","event":{"type":"content_block_stop","index":0}}),
                json!({"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"oi"}]}}),
                json!({"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":1,"output_tokens":1}})];
            if bad_rate { events.insert(1,json!({"type":"rate_limit_event","rate_limit_info":"boom"})); }
            for event in events {
                let frame = json!({"type":"cano_output","frame":event.to_string()});
                reader.get_mut().write_all(format!("{frame}\n").as_bytes()).await.unwrap();
            }
        }
    });
    let (policy,calls) = policy_server().await;
    let target = RuntimeTarget { key:"key".into(),generation:1,name:"session".into(),provider:"claude".into(),
        metadata:json!({"name":"session","headless":true,"session_id":"sid-1","initialized":true}),
        binding:CanoBinding { pid:42,escuta:format!("tcp:{address}"),token:"secret-test".into(),versao:2 },
        lease_path:dir.path().join("key.lock"),state_path:dir.path().join("key.queue-state.json"),projection_dir:dir.path().join("projection"),
        transcript,created:0.0 };
    let lease = acquire_lease(&target.lease_path).unwrap();
    let store = Store::open(&target.state_path,&target.projection_dir,State::new("key",1,"session",vec![])).unwrap();
    let queue = QueueActor::start(store,lease);
    let connection = cano::connect(&target.binding).await.unwrap();
    let engine = RuntimeEngine::new("claude",target.metadata.clone(),1,ClockSample { monotonic_s:0.0,epoch_s:1_800_000_000.0 }).unwrap()
        .with_policy(PolicyClient::new(policy,"secret".into(),"instance".into()));
    let handle = RuntimeActor::spawn(target,queue,connection,engine);
    let mut events = handle.subscribe();
    let input = RuntimeCommand { operation_id:"msg".into(),kind:OperationKind::Input,payload:json!({"text":"Olá","entry_id":"msg"}) };
    assert!(handle.command(input).await.unwrap().disposition == Disposition::Accepted);
    let mut problems = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(5),async {
        let mut idle_seen = false;
        loop {
            // A condição vale depois de cada evento e também sem evento: o último `idle` pode vir antes do problema ou da chamada.
            if let Ok(event) = tokio::time::timeout(std::time::Duration::from_millis(100),events.recv()).await {
                let event = event.unwrap();
                if event.channel == "problem" { problems.push(event.data.clone()); }
                if event.channel == "state" && event.data["state"] == "idle" { idle_seen = true; }
            }
            // Os serviços do Python que sobraram (uso, carimbo, sidecar) já foram pedidos; o preparo e o status são locais.
            if idle_seen && calls.load(std::sync::atomic::Ordering::SeqCst) > 0 && (!bad_rate || !problems.is_empty()) { break; }
        }
    }).await.expect("o turno precisa terminar");
    handle.stop().await.unwrap();
    server.await.unwrap();
    let state:State = serde_json::from_slice(&std::fs::read(dir.path().join("key.queue-state.json")).unwrap()).unwrap();
    assert!(!state.operations.keys().any(|id|id.starts_with("policy:")),"serviço sem efeito não entra no diário");
    let views = state.operations.values().filter(|op|op.payload["kind"] == "set_runtime_state").count();
    assert!(views <= 3,"vista gravada {views} vezes num turno sem mudança durável relevante");
    problems
}

#[tokio::test]
async fn a_queue_refusal_carries_its_reason() {
    // O diário do Python e o log do Rust mostravam só "queue_io"; a frase da fila é fixa e diz a causa.
    let (handle,server,_dir) = setup(true).await;
    let append = ||Action::Append { text:"Olá".into(),delivered:true,ts:None,pre_transcript:false,entry_id:Some("same".into()) };
    handle.queue("first".into(),append()).await.unwrap();
    let error = handle.queue("second".into(),append()).await.unwrap_err();
    assert_eq!(error.code,"queue_io");
    assert!(error.message.contains("entrada da fila já existe"),"{}",error.message);
    handle.stop().await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn steering_the_queue_without_a_turn_is_refused_and_keeps_the_entry() {
    // Sessão ainda sem inicialização: o drain não entrega, e a entrada só pode mudar pela orientação.
    let (handle,server,dir) = setup_claude_unreachable_policy(false).await;
    handle.queue("append".into(),Action::Append { text:"Depois".into(),delivered:false,ts:None,pre_transcript:false,entry_id:Some("later".into()) }).await.unwrap();
    let steer = RuntimeCommand { operation_id:"steer-1".into(),kind:OperationKind::SteerQueue,payload:json!({"entry_id":"later"}) };
    let reply = handle.command(steer).await.unwrap();
    assert!(reply.disposition == Disposition::Rejected);
    assert_eq!(reply.payload["error"],"Não há turno em andamento para orientar");
    // A entrada continua na fila e não ganha a marca de entregue: a orientação recusada não escreve nada.
    tokio::time::timeout(std::time::Duration::from_secs(5),async {
        loop {
            let state:State = serde_json::from_slice(&std::fs::read(dir.path().join("key.queue-state.json")).unwrap()).unwrap();
            if state.rows.iter().any(|row|row["id"] == "later" && row["delivered"] == false) { break; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.unwrap_or_else(|_|panic!("a entrada precisa continuar na fila: {}",std::fs::read_to_string(dir.path().join("key.queue-state.json")).unwrap()));
    handle.stop().await.unwrap();
    assert_eq!(server.await.unwrap(),0);
}

#[tokio::test]
async fn local_command_result_confirms_the_slash_entry() {
    // Comando local não vira linha `user`: o `result` com local_command é a prova, como no adapter Python.
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("chat.jsonl");
    std::fs::write(&transcript,"").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let written = transcript.clone();
    let server = tokio::spawn(async move {
        let (stream,_) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut header = String::new(); reader.read_line(&mut header).await.unwrap();
        let snapshot = json!({"type":"cano_snapshot","versao":2,"pid":42,"init":null,"aberto":false,
            "pendentes":[],"ultimo_result":null,"rate_limit":null,"stderr_tail":[],"saiu":null,"inflight":{}});
        reader.get_mut().write_all(format!("{snapshot}\n").as_bytes()).await.unwrap();
        loop {
            let mut raw = String::new();
            if reader.read_line(&mut raw).await.unwrap() == 0 { break; }
            let envelope:Value = serde_json::from_str(&raw).unwrap();
            let frame:Value = serde_json::from_str(envelope["frame"].as_str().unwrap()).unwrap();
            let ack = json!({"type":"cano_input_ack","operation_id":envelope["operation_id"],"outcome":"written"});
            reader.get_mut().write_all(format!("{ack}\n").as_bytes()).await.unwrap();
            if frame["type"] == "user" {
                let _ = &written;
                let result = json!({"type":"cano_output","frame":json!({"type":"result","subtype":"success","is_error":false,"local_command":"clear"}).to_string()});
                reader.get_mut().write_all(format!("{result}\n").as_bytes()).await.unwrap();
            }
        }
    });
    let target = RuntimeTarget { key:"key".into(),generation:1,name:"session".into(),provider:"claude".into(),
        metadata:json!({"name":"session","headless":true,"session_id":"sid-1","initialized":true}),
        binding:CanoBinding { pid:42,escuta:format!("tcp:{address}"),token:"secret-test".into(),versao:2 },
        lease_path:dir.path().join("key.lock"),state_path:dir.path().join("key.queue-state.json"),projection_dir:dir.path().join("projection"),
        transcript:transcript.clone(),created:0.0 };
    let lease = acquire_lease(&target.lease_path).unwrap();
    let store = Store::open(&target.state_path,&target.projection_dir,State::new("key",1,"session",vec![])).unwrap();
    let queue = QueueActor::start(store,lease);
    let connection = cano::connect(&target.binding).await.unwrap();
    let engine = RuntimeEngine::new("claude",target.metadata.clone(),1,ClockSample { monotonic_s:0.0,epoch_s:1_800_000_000.0 }).unwrap();
    let handle = RuntimeActor::spawn(target,queue,connection,engine);
    // Outra barra já reivindicada pelo drain e ainda não escrita: o result do /clear não pode confirmá-la.
    handle.queue("claimed".into(),Action::Append { text:"/cost".into(),delivered:true,ts:None,pre_transcript:false,entry_id:Some("next".into()) }).await.unwrap();
    let input = RuntimeCommand { operation_id:"msg".into(),kind:OperationKind::Input,payload:json!({"text":"/clear","entry_id":"msg"}) };
    assert!(handle.command(input).await.unwrap().disposition == Disposition::Accepted);
    let path = dir.path().join("key.queue-state.json");
    tokio::time::timeout(std::time::Duration::from_secs(5),async {
        loop {
            let state:State = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            if state.rows.iter().any(|row|row["id"] == "msg" && row["confirmed"] == true) { break; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.expect("comando local consumido precisa ficar confirmado");
    let state:State = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(state.rows.iter().any(|row|row["id"] == "next" && row["confirmed"] != true),"barra ainda não escrita não é confirmada");
    handle.stop().await.unwrap();
    server.await.unwrap();
}


/// Cano Claude falso que, depois do retrato, manda as linhas dadas e aceita tudo o que chega.
async fn claude_cano(lines:Vec<Value>) -> (std::net::SocketAddr,tokio::task::JoinHandle<usize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream,_) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut header = String::new(); reader.read_line(&mut header).await.unwrap();
        let snapshot = json!({"type":"cano_snapshot","versao":2,"pid":42,"init":null,"aberto":false,
            "pendentes":[],"ultimo_result":null,"rate_limit":null,"stderr_tail":[],"saiu":null,"inflight":{}});
        reader.get_mut().write_all(format!("{snapshot}\n").as_bytes()).await.unwrap();
        for line in lines {
            let frame = json!({"type":"cano_output","frame":line.to_string()});
            reader.get_mut().write_all(format!("{frame}\n").as_bytes()).await.unwrap();
        }
        let mut sent = 0;
        loop {
            let mut raw = String::new();
            if reader.read_line(&mut raw).await.unwrap() == 0 { break; }
            let envelope:Value = serde_json::from_str(&raw).unwrap();
            let ack = json!({"type":"cano_input_ack","operation_id":envelope["operation_id"],"outcome":"written"});
            reader.get_mut().write_all(format!("{ack}\n").as_bytes()).await.unwrap();
            sent += 1;
        }
        sent
    });
    (address,server)
}

async fn claude_actor(dir:&std::path::Path,cano:std::net::SocketAddr,policy:std::net::SocketAddr) -> RuntimeHandle {
    let target = RuntimeTarget { key:"key".into(),generation:1,name:"session".into(),provider:"claude".into(),
        metadata:json!({"name":"session","headless":true,"session_id":"sid-1","initialized":true}),
        binding:CanoBinding { pid:42,escuta:format!("tcp:{cano}"),token:"secret-test".into(),versao:2 },
        lease_path:dir.join("key.lock"),state_path:dir.join("key.queue-state.json"),projection_dir:dir.join("projection"),
        transcript:dir.join("chat.jsonl"),created:0.0 };
    let lease = acquire_lease(&target.lease_path).unwrap();
    let store = Store::open(&target.state_path,&target.projection_dir,State::new("key",1,"session",vec![])).unwrap();
    let queue = QueueActor::start(store,lease);
    let connection = cano::connect(&target.binding).await.unwrap();
    let engine = RuntimeEngine::new("claude",target.metadata.clone(),1,ClockSample { monotonic_s:0.0,epoch_s:1_800_000_000.0 }).unwrap()
        .with_policy(PolicyClient::new(policy,"secret".into(),"instance".into()));
    RuntimeActor::spawn(target,queue,connection,engine)
}

#[tokio::test]
async fn a_failed_sidecar_update_still_hands_the_session_over() {
    // Só status/carimbo/uso/registro são perdoados; o sidecar segura a conversa atual (session_id
    // depois do /clear), e perdê-lo calado deixaria o app lendo o transcript velho.
    let dir = tempfile::tempdir().unwrap();
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
    let (cano,server) = claude_cano(vec![json!({"type":"system","subtype":"init","session_id":"sid-2"})]).await;
    let handle = claude_actor(dir.path(),cano,closed).await;
    tokio::time::timeout(std::time::Duration::from_secs(5),async {
        loop {
            if handle.snapshot().await.unwrap()["error"] == "policy_transport" { break; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.expect("falha do sidecar precisa pôr a sessão em erro");
    handle.stop().await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn the_deadline_never_puts_back_a_native_message_that_may_have_been_sent() {
    // O recado nativo sai pelo Python; se ele demora até o prazo de 30 s, a entrada pode já ter sido
    // entregue e não pode voltar para a fila (seria enviada de novo).
    use tokio::io::AsyncReadExt;
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let policy = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((stream,_)) = listener.accept().await else { return };
            tokio::spawn(async move {
                let mut reader = BufReader::new(stream);
                loop {
                    let mut length = 0usize;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).await.unwrap_or(0) == 0 { return; }
                        if line == "\r\n" { break; }
                        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") { length = value.trim().parse().unwrap(); }
                    }
                    let mut body = vec![0;length]; reader.read_exact(&mut body).await.unwrap();
                    let kind = serde_json::from_slice::<Value>(&body).unwrap()["kind"].clone();
                    let data = if kind == "prepare_prompt" { json!({"content":"[de: par] oi","notices":[],"native_candidate":true}) }
                        else if kind == "native_message" { tokio::time::sleep(std::time::Duration::from_secs(33)).await; json!({"outcome":"written","msg_id":"m"}) }
                        else { json!({}) };
                    let reply = json!({"ok":true,"data":data}).to_string();
                    let response = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{reply}",reply.len());
                    if reader.get_mut().write_all(response.as_bytes()).await.is_err() { return; }
                }
            });
        }
    });
    let (cano,server) = claude_cano(vec![]).await;
    let handle = claude_actor(dir.path(),cano,policy).await;
    let input = RuntimeCommand { operation_id:"msg".into(),kind:OperationKind::Input,payload:json!({"text":"[de: par] oi","entry_id":"msg"}) };
    let reply = handle.command(input).await.unwrap();
    assert!(reply.disposition == Disposition::Unknown,"prazo com envio nativo em curso é incerto, nunca adiado");
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let state:State = serde_json::from_slice(&std::fs::read(dir.path().join("key.queue-state.json")).unwrap()).unwrap();
    assert!(state.rows.iter().any(|row|row["id"] == "msg" && row["delivered"] == true),"a entrada não volta para a fila");
    assert!(state.operations.get("msg").is_some_and(|op|op.status != Status::Deferred));
    handle.stop().await.unwrap();
    assert_eq!(server.await.unwrap(),0,"nada foi escrito no cano: o recado foi pelo Python");
}

// --- Codex sem terminal nasce e religa no Rust (5B Task 4) ---

#[cfg(target_os = "linux")]
mod launch {
    use super::*;
    use hangar_server::runtime::gateway::RuntimeRegistry;
    use hangar_server::runtime::process;
    use std::sync::{Arc,Mutex};
    use tokio::io::AsyncReadExt;

    fn unique_key() -> String {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64;
        format!("{:016x}{:04x}",nanos ^ ((std::process::id() as u64) << 32),rand_suffix())
    }
    fn rand_suffix() -> u16 { static N:std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0); N.fetch_add(1,std::sync::atomic::Ordering::Relaxed) }

    /// O `hangar-cano` de outro crate mora em `target/<perfil>/`, ao lado de `deps/`.
    fn use_cano_bin() {
        static ONCE:std::sync::Once = std::sync::Once::new();
        ONCE.call_once(||{
            let bin = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().join("hangar-cano");
            assert!(bin.exists(),"rode `cargo build -p hangar-cano` antes: {}",bin.display());
            // SAFETY: valor único, posto antes de qualquer sonda do binário neste processo.
            unsafe { std::env::set_var("CP_RUST_CANO_BIN",bin); }
        });
    }

    /// `codex app-server --stdio` falso: responde `initialize` e a abertura da conversa.
    fn fake_codex(dir:&std::path::Path) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("codex");
        std::fs::write(&path,format!(r#"#!/usr/bin/env python3
import json, sys
for line in sys.stdin:
    msg = json.loads(line)
    if "id" not in msg or "method" not in msg:
        continue
    method = msg["method"]
    if method == "initialize":
        result = {{"userAgent": "hangar/{} (x)"}}
    elif method in ("thread/start", "thread/resume"):
        result = {{"thread": {{"id": "thread-new", "path": "/tmp/rollout-thread-new.jsonl"}}, "model": "gpt-test"}}
    else:
        result = {{}}
    print(json.dumps({{"id": msg["id"], "result": result}}), flush=True)
"#,hangar_codex::version::CHECKED)).unwrap();
        std::fs::set_permissions(&path,std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn env(key:&str,owner:&str) -> Value {
        let mut env:serde_json::Map<String,Value> = std::env::vars().filter(|(k,_)|!k.starts_with("HANGAR_CANO_")).map(|(k,v)|(k,json!(v))).collect();
        env.insert("HANGAR_CANO_KEY".into(),json!(key));
        env.insert("HANGAR_CANO_OWNER".into(),json!(owner));
        Value::Object(env)
    }

    /// Python falso da política: `launch_env` devolve o `codex` falso; o resto só é anotado.
    async fn policy(launch:Value) -> (std::net::SocketAddr,Arc<Mutex<Vec<(String,Value)>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = calls.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream,_)) = listener.accept().await else { return };
                let (seen,launch) = (seen.clone(),launch.clone());
                tokio::spawn(async move {
                    let mut reader = BufReader::new(stream);
                    loop {
                        let mut length = 0usize;
                        loop {
                            let mut line = String::new();
                            if reader.read_line(&mut line).await.unwrap_or(0) == 0 { return; }
                            if line == "\r\n" { break; }
                            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") { length = value.trim().parse().unwrap(); }
                        }
                        let mut body = vec![0;length]; reader.read_exact(&mut body).await.unwrap();
                        let body:Value = serde_json::from_slice(&body).unwrap();
                        let kind = body["kind"].as_str().unwrap().to_owned();
                        seen.lock().unwrap().push((kind.clone(),body["payload"].clone()));
                        let data = if kind == "launch_env" { launch.clone() } else { json!({}) };
                        let reply = json!({"ok":true,"data":data}).to_string();
                        let response = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{reply}",reply.len());
                        if reader.get_mut().write_all(response.as_bytes()).await.is_err() { return; }
                    }
                });
            }
        });
        (address,calls)
    }

    fn target(dir:&std::path::Path,key:&str,binding:CanoBinding) -> RuntimeTarget {
        RuntimeTarget { key:key.into(),generation:1,name:"cx".into(),provider:"codex".into(),
            metadata:json!({"name":"cx","key":key,"headless":true,"cwd":dir}),binding,
            lease_path:dir.join("q.lock"),state_path:dir.join("q.json"),projection_dir:dir.join("projection"),
            transcript:dir.join("rollout.jsonl"),created:0.0 }
    }

    async fn until_ready(registry:&RuntimeRegistry,key:&str) {
        let handle = registry.handle(key,1).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(15),async {
            loop {
                if handle.snapshot().await.unwrap()["view"]["ready"] == true { break; }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }).await.expect("o motor chega a ready no processo novo");
    }

    fn no_cano() -> CanoBinding { CanoBinding { pid:0,escuta:String::new(),token:String::new(),versao:2 } }

    #[tokio::test]
    async fn open_with_launch_and_no_cano_spawns_records_and_gets_ready() {
        use_cano_bin();
        let dir = tempfile::tempdir().unwrap();
        let key = unique_key();
        let owner = dir.path().to_string_lossy().into_owned();
        let codex = fake_codex(dir.path());
        let (address,calls) = policy(json!({"program":[codex,"app-server","--stdio"],"env":env(&key,&owner),"cano_extra":{"marca":"m1"}})).await;
        let registry = RuntimeRegistry::new(address,"secret".into(),"instance".into());
        let opened = registry.open_with_launch(target(dir.path(),&key,no_cano()),dir.path().into()).await.unwrap();
        assert_eq!(opened["opened"],true);
        until_ready(&registry,&key).await;
        let calls = calls.lock().unwrap().clone();
        assert_eq!(calls[0].0,"launch_env","o ambiente é pedido ao Python a cada subida");
        let patch = calls.iter().find(|(kind,payload)|kind == "session.patch_meta" && payload.get("cano").is_some()).expect("o cano novo vai para o arquivo da sessão");
        let pid = patch.1["cano"]["pid"].as_u64().unwrap() as u32;
        assert_eq!(patch.1["cano"]["versao"],2);
        assert_eq!(patch.1["cano"]["marca"],"m1");
        assert!(matches!(process::liveness(pid,&key),process::Liveness::Ours));
        registry.close(&key,1).await.unwrap();
        let cano:process::Cano = serde_json::from_value(patch.1["cano"].clone()).unwrap();
        process::kill(&cano,&key,dir.path()).await.unwrap();
    }

    #[tokio::test]
    async fn open_with_launch_and_live_cano_connects_without_spawning() {
        use_cano_bin();
        let dir = tempfile::tempdir().unwrap();
        let key = unique_key();
        let owner = dir.path().to_string_lossy().into_owned();
        let codex = fake_codex(dir.path());
        let env:Vec<(String,String)> = env(&key,&owner).as_object().unwrap().iter().map(|(k,v)|(k.clone(),v.as_str().unwrap().to_owned())).collect();
        let live = process::spawn(&process::LaunchSpec { provider:process::Provider::Codex,key:key.clone(),cwd:dir.path().into(),
            program:vec![codex.to_string_lossy().into_owned(),"app-server".into(),"--stdio".into()],env,
            cano_extra:Default::default(),sidecar_dir:dir.path().into() }).await.unwrap();
        let (address,calls) = policy(json!({})).await;
        let registry = RuntimeRegistry::new(address,"secret".into(),"instance".into());
        let binding = CanoBinding { pid:live.pid,escuta:live.escuta.clone(),token:live.token.clone(),versao:2 };
        registry.open_with_launch(target(dir.path(),&key,binding),dir.path().into()).await.unwrap();
        until_ready(&registry,&key).await;
        assert!(calls.lock().unwrap().iter().all(|(kind,_)|kind != "launch_env"),"cano vivo da sessão: conecta e não sobe outro");
        registry.close(&key,1).await.unwrap();
        process::kill(&live,&key,dir.path()).await.unwrap();
    }
}
