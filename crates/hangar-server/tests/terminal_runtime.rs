use hangar_server::runtime::{actor::PolicyClient,protocol::{RuntimeCommand,OperationKind,ClockSample},queue::{self,QueueActor,Store,Action},terminal::{TerminalActor,TerminalTarget,TerminalOptions}};
use hangar_server::terminal_input::*;
use serde_json::{Value,json};
use std::sync::{Arc,Mutex,atomic::AtomicU64};
use std::time::Duration;
use tokio::sync::{broadcast,Notify};

struct Io { calls:Mutex<Vec<CommandRequest>>, text:Mutex<String>, gate:Notify, blocked:std::sync::atomic::AtomicBool, fail_write:std::sync::atomic::AtomicBool, fail_enter:std::sync::atomic::AtomicBool, rotate_enter:std::sync::atomic::AtomicBool, conversation:Arc<Mutex<String>>, socket_calls:Mutex<Vec<Vec<u8>>> }
impl Io { fn new()->Self { Self { calls:Mutex::new(vec![]),text:Mutex::new(String::new()),gate:Notify::new(),blocked:std::sync::atomic::AtomicBool::new(false),fail_write:std::sync::atomic::AtomicBool::new(false),fail_enter:std::sync::atomic::AtomicBool::new(false),rotate_enter:std::sync::atomic::AtomicBool::new(false),conversation:Arc::new(Mutex::new("sid".into())),socket_calls:Mutex::new(vec![]) } } }
impl TerminalIo for Io {
    fn command<'a>(&'a self,r:CommandRequest)->IoFuture<'a,CommandOutput> { Box::pin(async move {
        let cmd=r.args[0].clone();
        self.calls.lock().unwrap().push(r.clone());
        let stdout=match cmd.as_str() {
            "display-message"=>b"session\t%1\t1\n".to_vec(),
            "capture-pane"=>format!("────────────────────────────────\n❯ {}\n────────────────────────────────\n",self.text.lock().unwrap()).into_bytes(),
            "send-keys"=>{
                if self.blocked.load(std::sync::atomic::Ordering::Acquire) { self.gate.notified().await; }
                let text=r.args.last().unwrap();
                if text=="\r" || text=="C-u" { self.text.lock().unwrap().clear(); if text=="\r" && self.rotate_enter.load(std::sync::atomic::Ordering::Acquire){*self.conversation.lock().unwrap()="new-sid".into();} if text=="\r" && self.fail_enter.load(std::sync::atomic::Ordering::Acquire){return Err(IoFailure {code:"enter_uncertain",may_have_written:true});} }
                else if r.args.contains(&"-l".into()) {
                    *self.text.lock().unwrap()=text.clone();
                    if self.fail_write.load(std::sync::atomic::Ordering::Acquire){return Err(IoFailure {code:"partial_write",may_have_written:true});}
                }
                vec![]
            }, _=>vec![]
        };
        Ok(CommandOutput {success:true,stdout})
    }) }
    fn socket<'a>(&'a self,_:&'a NativeMessage,envelope:Vec<u8>)->IoFuture<'a,WriteOutcome> { Box::pin(async move {self.socket_calls.lock().unwrap().push(envelope);Ok(WriteOutcome::Unknown)}) }
}
struct Fixture { _dir:tempfile::TempDir,target:TerminalTarget,policy:PolicyClient,io:Arc<Io>, idle:Arc<std::sync::atomic::AtomicBool>, ready:Arc<std::sync::atomic::AtomicBool>, generation:Arc<AtomicU64>, native:Arc<std::sync::atomic::AtomicBool>, control:Arc<Mutex<Value>>, unknown:Arc<std::sync::atomic::AtomicBool>, calls:Arc<Mutex<Vec<Value>>>,server:tokio::task::JoinHandle<()> }
impl Fixture {
    async fn new()->Self {
        let dir=tempfile::tempdir().unwrap(); let io=Arc::new(Io::new()); let conversation=io.conversation.clone();
        let binding=TerminalBinding {name:"session".into(),pane:"%1".into(),conversation:"sid".into(),generation:1,created:1,mux_argv:vec!["fake".into()],windows:false,clipboard_lock_path:None};
        let target=TerminalTarget {key:"key".into(),generation:1,name:"session".into(),binding:binding.clone(),lease_path:dir.path().join("lease"),state_path:dir.path().join("state"),projection_dir:dir.path().join("projection"),transcript:dir.path().join("chat.jsonl"),created:1.0};
        std::fs::write(&target.transcript,"").unwrap();
        let native=Arc::new(std::sync::atomic::AtomicBool::new(false)); let generation=Arc::new(AtomicU64::new(1)); let control=Arc::new(Mutex::new(json!({"disposition":"unavailable"})));
        let idle=Arc::new(std::sync::atomic::AtomicBool::new(true)); let ready=Arc::new(std::sync::atomic::AtomicBool::new(true)); let unknown=Arc::new(std::sync::atomic::AtomicBool::new(false)); let calls=Arc::new(Mutex::new(vec![]));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap(); let address=listener.local_addr().unwrap();
        let state_path=target.state_path.clone();
        let (i,r,u,c,g,control_reply,n)=(idle.clone(),ready.clone(),unknown.clone(),calls.clone(),generation.clone(),control.clone(),native.clone());
        let router=axum::Router::new().route("/internal/runtime/policy",axum::routing::post(move |body:String| {let (i,r,u,c,mut b,path,g,control_reply,n,conversation)=(i.clone(),r.clone(),u.clone(),c.clone(),binding.clone(),state_path.clone(),g.clone(),control_reply.clone(),n.clone(),conversation.clone()); async move {
            let v:Value=serde_json::from_str(&body).unwrap();
            let state:Value=serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            let phase=&state["operations"][v["phase_id"].as_str().unwrap()];
            assert_eq!(phase["status"],"dispatching");
            assert_eq!(phase["payload"],json!({"kind":v["kind"],"request_id":v["request_id"],"payload":v["payload"]}));
            c.lock().unwrap().push(v.clone());
            b.generation=g.load(std::sync::atomic::Ordering::Acquire); b.conversation=conversation.lock().unwrap().clone();
            let data=match v["kind"].as_str().unwrap() {
                "terminal_facts"=>json!({"binding":b,"ready":r.load(std::sync::atomic::Ordering::Acquire),"idle":i.load(std::sync::atomic::Ordering::Acquire),"open_question":false,"plugin_live":u.load(std::sync::atomic::Ordering::Acquire),"plugin_user":true,"native":if n.load(std::sync::atomic::Ordering::Acquire){Some(NativeMessage {socket:"fake".into(),origin:"peer".into(),sender:"peer".into(),mode:"message".into(),message_id:Some(format!("native:{}",v["payload"]["operation_id"].as_str().unwrap()))})}else{None}}),
                "terminal_publish"=>json!("unknown"),
                "terminal_plugin_control"=>control_reply.lock().unwrap().clone(), _=>panic!("unexpected policy")};
            ([("content-type","application/json")],json!({"ok":true,"data":data}).to_string())
        }}));
        let server=tokio::spawn(async move {axum::serve(listener,router).await.unwrap()});
        Self {_dir:dir,target,policy:PolicyClient::new(address,"test".into(),"instance".into()),io,idle,ready,generation,native,control,unknown,calls,server}
    }
    fn start(&self)->hangar_server::runtime::terminal::TerminalHandle {
        let lease=queue::acquire_lease(&self.target.lease_path).unwrap();
        let store=Store::open(&self.target.state_path,&self.target.projection_dir,queue::State::new("key",1,"session",vec![])).unwrap();
        let options=TerminalOptions {io:self.io.clone(),limits:InputLimits {settle:Duration::ZERO,literal_settle:Duration::ZERO,multiline_settle:Duration::ZERO,slash_settle:Duration::ZERO,proof_attempts:1,ready_attempts:1,cleanup_attempts:1},tick:Duration::from_millis(15)};
        TerminalActor::spawn(self.target.clone(),QueueActor::start(store,lease),self.policy.clone(),options,broadcast::channel(128).0,Arc::new(AtomicU64::new(0)))
    }
    fn command(&self,id:&str,text:&str)->RuntimeCommand { RuntimeCommand {operation_id:id.into(),kind:OperationKind::Input,payload:json!({"text":text,"pre_transcript":false})} }
    fn state(&self)->Value {serde_json::from_slice(&std::fs::read(&self.target.state_path).unwrap()).unwrap()}
}
impl Drop for Fixture {fn drop(&mut self){self.server.abort();}}

#[tokio::test]
async fn terminal_runtime_repeat_id_does_not_write_twice_and_conflicting_payload_fails() {
    let f=Fixture::new().await; let h=f.start(); let command=f.command("first","Olá");
    assert_eq!(h.command(command.clone()).await.unwrap().disposition,hangar_server::runtime::protocol::Disposition::Accepted);
    let count=f.io.calls.lock().unwrap().len();
    h.command(command).await.unwrap(); assert_eq!(f.io.calls.lock().unwrap().len(),count);
    assert!(h.command(f.command("first","Outro")).await.is_err()); h.stop().await.unwrap();
}
#[tokio::test]
async fn terminal_runtime_unknown_survives_restart_and_never_requeues() {
    let f=Fixture::new().await; f.unknown.store(true,std::sync::atomic::Ordering::Release); let h=f.start();
    let command=f.command("uncertain","Olá"); assert_eq!(h.command(command.clone()).await.unwrap().disposition,hangar_server::runtime::protocol::Disposition::Unknown);
    h.stop().await.unwrap(); let h=f.start(); h.command(command).await.unwrap(); h.drain().await.unwrap();
    assert!(f.io.calls.lock().unwrap().iter().all(|r|r.args[0]!="send-keys"));
    assert_eq!(f.state()["rows"][0]["delivered"],true); h.stop().await.unwrap();
}
#[tokio::test]
async fn terminal_runtime_idle_timer_drains_without_sse_and_claims_one() {
    let f=Fixture::new().await; f.idle.store(false,std::sync::atomic::Ordering::Release); f.ready.store(false,std::sync::atomic::Ordering::Release); let h=f.start();
    h.command(f.command("queued1","Um")).await.unwrap(); h.command(f.command("queued2","Dois")).await.unwrap();
    assert!(f.io.calls.lock().unwrap().is_empty());
    f.idle.store(true,std::sync::atomic::Ordering::Release); f.ready.store(true,std::sync::atomic::Ordering::Release);
    tokio::time::timeout(Duration::from_secs(2),async {loop {if f.state()["rows"].as_array().unwrap().iter().all(|r|r["delivered"]==true){break;} tokio::time::sleep(Duration::from_millis(10)).await;}}).await.unwrap();
    let state=f.state(); assert_eq!(state["rows"].as_array().unwrap().len(),2); h.stop().await.unwrap();
}
#[tokio::test]
async fn terminal_runtime_identical_text_uses_distinct_occurrences_and_enqueue_stays_visible() {
    let f=Fixture::new().await; let h=f.start(); h.command(f.command("one","Olá — 📎 imagem: /tmp/x.png")).await.unwrap(); h.command(f.command("two","Olá — 📎 imagem: /tmp/x.png")).await.unwrap();
    std::fs::write(&f.target.transcript,"{\"type\":\"queue-operation\",\"operation\":\"enqueue\",\"content\":\"Olá\"}\n").unwrap(); h.confirm().await.unwrap();
    assert!(f.state()["rows"].as_array().unwrap().iter().all(|r|r["confirmed"]!=true));
    std::fs::write(&f.target.transcript,"{\"type\":\"user\",\"uuid\":\"1\",\"message\":{\"content\":\"Olá\"}}\n").unwrap(); h.confirm().await.unwrap();
    assert_eq!(f.state()["rows"].as_array().unwrap().iter().filter(|r|r["confirmed"]==true).count(),1);
    use std::io::Write; let mut file=std::fs::OpenOptions::new().append(true).open(&f.target.transcript).unwrap(); writeln!(file,"{{\"type\":\"attachment\",\"uuid\":\"2\",\"attachment\":{{\"type\":\"queued_command\",\"prompt\":\"Olá\"}}}}").unwrap(); h.confirm().await.unwrap();
    assert!(f.state()["rows"].as_array().unwrap().iter().all(|r|r["confirmed"]==true)); h.stop().await.unwrap();
}
#[tokio::test]
async fn terminal_runtime_cancel_http_does_not_release_lease_and_stop_waits() {
    let f=Fixture::new().await; f.io.blocked.store(true,std::sync::atomic::Ordering::Release); let h=f.start(); let hc=h.clone(); let cmd=f.command("flight","Olá");
    let request=tokio::spawn(async move {hc.command(cmd).await});
    tokio::time::timeout(Duration::from_secs(2),async {loop {if f.io.calls.lock().unwrap().iter().any(|r|r.args[0]=="send-keys"){break;}tokio::time::sleep(Duration::from_millis(5)).await;}}).await.unwrap();
    request.abort(); let hc=h.clone(); let stopping=tokio::spawn(async move {hc.stop().await}); tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(!stopping.is_finished()); assert!(queue::acquire_lease(&f.target.lease_path).is_err());
    f.io.blocked.store(false,std::sync::atomic::Ordering::Release); f.io.gate.notify_waiters(); stopping.await.unwrap().unwrap(); assert!(queue::acquire_lease(&f.target.lease_path).is_ok());
}
#[tokio::test]
async fn terminal_runtime_policy_calls_are_journaled_and_root_id_is_stable() {
    let f=Fixture::new().await; let h=f.start(); h.command(f.command("root","Olá")).await.unwrap();
    for call in f.calls.lock().unwrap().iter() {let state=f.state(); let phase=call["phase_id"].as_str().unwrap(); assert!(state["operations"][phase]["payload"]==json!({"kind":call["kind"],"request_id":call["request_id"],"payload":call["payload"]})); assert_eq!(call["request_id"],"root");}
    h.stop().await.unwrap();
}
#[tokio::test]
async fn terminal_runtime_slash_has_no_prompt_row_and_private_snapshot_has_no_public_state() {
    let f=Fixture::new().await; let h=f.start(); h.command(f.command("slash","/help")).await.unwrap(); let s=h.snapshot().await.unwrap();
    assert_eq!(s["view"]["terminal"],true); assert!(s["view"].get("public_state").is_none()); assert!(f.state()["rows"].as_array().unwrap().is_empty()); h.stop().await.unwrap();
}
#[tokio::test]
async fn terminal_runtime_first_prompt_missing_transcript_confirms_without_redelivery() {
    let f=Fixture::new().await; std::fs::remove_file(&f.target.transcript).unwrap(); let h=f.start(); h.command(f.command("first","Olá")).await.unwrap();
    std::fs::write(&f.target.transcript,format!("{}\n",json!({"type":"user","sessionId":"sid","uuid":"first","timestamp":chrono::Utc::now().to_rfc3339(),"message":{"content":"Olá"}}))).unwrap();
    h.confirm().await.unwrap(); assert_eq!(f.state()["rows"][0]["confirmed"],true); h.stop().await.unwrap();
}

#[tokio::test]
async fn terminal_runtime_working_allows_safe_tui_enqueue_without_hiding_pending_row() {
    let f=Fixture::new().await; f.idle.store(false,std::sync::atomic::Ordering::Release); let h=f.start();
    let reply=h.command(f.command("working","Olá")).await.unwrap();
    assert_eq!(reply.disposition,hangar_server::runtime::protocol::Disposition::Accepted);
    assert_eq!(f.state()["rows"][0]["delivered"],true); assert_ne!(f.state()["rows"][0]["confirmed"],true);
    h.stop().await.unwrap();
}
#[tokio::test]
async fn terminal_runtime_explicit_steer_uses_driver_and_no_prompt_row() {
    let f=Fixture::new().await; let h=f.start();
    let result=h.control("steer-now".into(),"steer".into(),json!({})).await.unwrap();
    assert_eq!(result.disposition,hangar_server::runtime::protocol::Disposition::Deferred);
    assert!(f.state()["rows"].as_array().unwrap().is_empty()); h.stop().await.unwrap();
}

#[tokio::test]
async fn terminal_runtime_private_controls_validate_entire_payload_before_effect() {
    let f=Fixture::new().await; let h=f.start();
    for (kind,payload) in [("navigation_key",json!({"key":"C-c"})),("interactive_key",json!({"key":"-N"})),("terminal_input",json!({"text":"bad\u{1b}"})),("select",json!({"option":1,"bad":true})),("answer_questions",json!({"answers":[{"kind":"text","type_index":1,"value":"bad\u{1b}"}]}))] {
        let result=h.control(format!("bad-{kind}"),kind.into(),payload).await;
        assert!(result.is_err() || result.unwrap().disposition==hangar_server::runtime::protocol::Disposition::Rejected);
    }
    assert!(f.io.calls.lock().unwrap().is_empty()); assert!(f.calls.lock().unwrap().is_empty()); h.stop().await.unwrap();
}

#[tokio::test]
async fn terminal_runtime_cleanup_proved_uses_same_queue_budget_original_plus_two() {
    let f=Fixture::new().await; f.io.fail_write.store(true,std::sync::atomic::Ordering::Release); let h=f.start();
    let result=h.command(f.command("retry","Olá")).await.unwrap(); assert_eq!(result.payload["cleanup"],"proved");
    tokio::time::timeout(Duration::from_secs(2),async {loop {if f.state()["rows"][0]["desistiu"]==true{break;}tokio::time::sleep(Duration::from_millis(10)).await;}}).await.unwrap();
    assert_eq!(f.state()["rows"][0]["attempts"],2);
    assert_eq!(f.io.calls.lock().unwrap().iter().filter(|r|r.args[0]=="send-keys" && r.args.last().unwrap()=="Olá").count(),3);
    assert!(!f.io.calls.lock().unwrap().iter().any(|r|r.args.last().is_some_and(|s|s=="\r")));
    h.command(f.command("retry","Olá")).await.unwrap(); h.drain().await.unwrap();
    assert_eq!(f.io.calls.lock().unwrap().iter().filter(|r|r.args[0]=="send-keys" && r.args.last().unwrap()=="Olá").count(),3); h.stop().await.unwrap();
}
#[tokio::test]
async fn terminal_runtime_dispatching_recovers_unknown_not_redigitated() {
    let f=Fixture::new().await;
    let mut store=Store::open(&f.target.state_path,&f.target.projection_dir,queue::State::new("key",1,"session",vec![])).unwrap();
    let clock=ClockSample {monotonic_s:0.0,epoch_s:chrono::Utc::now().timestamp() as f64};
    let command=f.command("crash","Olá");
    store.exec(1,"prepare",clock,Action::Prepare {id:"crash".into(),payload:serde_json::to_value(&command).unwrap(),entry_id:Some("crash".into())}).unwrap();
    store.exec(1,"append",clock,Action::Append {text:"Olá".into(),delivered:false,ts:None,pre_transcript:false,entry_id:Some("crash".into())}).unwrap();
    store.exec(1,"dispatch",clock,Action::BeginDispatch {id:"crash".into(),wire_id:"attempt".into()}).unwrap(); drop(store);
    let h=f.start(); assert_eq!(h.command(command).await.unwrap().disposition,hangar_server::runtime::protocol::Disposition::Unknown);
    h.drain().await.unwrap(); assert!(f.io.calls.lock().unwrap().is_empty()); assert_eq!(f.state()["operations"]["crash"]["status"],"unknown");
    assert!(h.queue("unclaim".into(),Action::SetDelivered {entry_id:"crash".into(),value:false,steered:false}).await.is_err()); h.stop().await.unwrap();
}

#[tokio::test]
async fn terminal_runtime_held_plugin_unknown_does_not_fall_back_and_repeat_does_not_publish() {
    let f=Fixture::new().await; *f.control.lock().unwrap()=json!({"disposition":"unknown"}); let h=f.start();
    let payload=json!({"option":1,"require_cursor":true,"request_id":"perm:held"});
    let result=h.control("held".into(),"select".into(),payload.clone()).await.unwrap();
    assert_eq!(result.disposition,hangar_server::runtime::protocol::Disposition::Unknown);
    assert!(f.io.calls.lock().unwrap().is_empty()); let count=f.calls.lock().unwrap().len();
    h.control("held".into(),"select".into(),payload).await.unwrap(); assert_eq!(f.calls.lock().unwrap().len(),count);
    let call=f.calls.lock().unwrap()[0].clone(); assert_eq!(call["request_id"],"perm:held"); assert_eq!(call["payload"]["generation"],1);
    h.stop().await.unwrap();
}
#[tokio::test]
async fn terminal_runtime_held_plugin_unavailable_permits_tui_and_accepted_answer_skips_keys() {
    let f=Fixture::new().await; let h=f.start();
    let result=h.control("select".into(),"select".into(),json!({"option":1,"request_id":"perm:held"})).await.unwrap();
    assert_eq!(result.disposition,hangar_server::runtime::protocol::Disposition::Deferred); assert!(!f.io.calls.lock().unwrap().is_empty());
    f.io.calls.lock().unwrap().clear(); *f.control.lock().unwrap()=json!({"disposition":"accepted"});
    let result=h.control("answer".into(),"answer_questions".into(),json!({"request_id":"ask:held","answers":[{"kind":"option","indices":[0],"labels":["A"]}]})).await.unwrap();
    assert_eq!(result.disposition,hangar_server::runtime::protocol::Disposition::Accepted); assert!(f.io.calls.lock().unwrap().is_empty()); h.stop().await.unwrap();
}
#[tokio::test]
async fn terminal_runtime_binding_changes_while_an_operation_waits_prevents_old_second_effect() {
    let f=Fixture::new().await; f.io.blocked.store(true,std::sync::atomic::Ordering::Release); let h=f.start();
    let hc=h.clone(); let cmd=f.command("flight","Primeiro"); let first=tokio::spawn(async move {hc.command(cmd).await});
    tokio::time::timeout(Duration::from_secs(2),async {loop {if f.io.calls.lock().unwrap().iter().any(|r|r.args[0]=="send-keys"){break;}tokio::time::sleep(Duration::from_millis(5)).await;}}).await.unwrap();
    let hc=h.clone(); let cmd=f.command("waiting","Segundo"); let second=tokio::spawn(async move {hc.command(cmd).await});
    f.generation.store(2,std::sync::atomic::Ordering::Release); f.io.blocked.store(false,std::sync::atomic::Ordering::Release); f.io.gate.notify_waiters();
    assert_eq!(first.await.unwrap().unwrap().disposition,hangar_server::runtime::protocol::Disposition::Unknown);
    assert_eq!(second.await.unwrap().unwrap().disposition,hangar_server::runtime::protocol::Disposition::Deferred);
    assert_eq!(f.io.calls.lock().unwrap().iter().filter(|r|r.args[0]=="send-keys").count(),1); h.stop().await.unwrap();
}

#[tokio::test]
async fn terminal_runtime_clear_deferred_without_submission_never_repeats_same_operation() {
    let f=Fixture::new().await; f.io.fail_write.store(true,std::sync::atomic::Ordering::Release); let h=f.start();
    // O texto parcial pode ser limpo antes de Enter; o comando ainda não alterou a conversa.
    let result=h.command(f.command("clear","/clear")).await.unwrap();
    assert_eq!(result.disposition,hangar_server::runtime::protocol::Disposition::Deferred);
    let count=f.io.calls.lock().unwrap().len(); h.command(f.command("clear","/clear")).await.unwrap(); assert_eq!(f.io.calls.lock().unwrap().len(),count);
    assert!(f.state()["rows"].as_array().unwrap().is_empty()); assert_ne!(f.state()["runtime_state"]["preserve_binding"],true);
    f.io.fail_write.store(false,std::sync::atomic::Ordering::Release);
    assert_eq!(h.command(f.command("after-deferred-clear","Olá")).await.unwrap().disposition,hangar_server::runtime::protocol::Disposition::Accepted);
    h.stop().await.unwrap();
}

#[tokio::test]
async fn terminal_runtime_native_receipt_for_queue_attempt_keeps_root_uuid_and_does_not_confirm() {
    let f=Fixture::new().await; f.ready.store(false,std::sync::atomic::Ordering::Release); let h=f.start();
    let command=f.command("native-root","[de: peer] Olá"); h.command(command.clone()).await.unwrap();
    f.ready.store(true,std::sync::atomic::Ordering::Release); f.native.store(true,std::sync::atomic::Ordering::Release); h.drain().await.unwrap();
    assert_eq!(f.io.socket_calls.lock().unwrap().len(),1);
    let envelope:Value=serde_json::from_slice(&f.io.socket_calls.lock().unwrap()[0]).unwrap(); assert_eq!(envelope["msg_id"],"native:native-root");
    h.queue("receipt".into(),Action::Finish {id:"native-root".into(),status:queue::Status::Accepted,result:json!({"operation_id":"native-root","disposition":"accepted","payload":{"native_status":"delivered"}})}).await.unwrap();
    let replay=h.command(command).await.unwrap(); assert_eq!(replay.payload["native"],true); assert_eq!(replay.payload["native_status"],"delivered"); assert_ne!(f.state()["rows"][0]["confirmed"],true);
    assert_eq!(f.io.socket_calls.lock().unwrap().len(),1); h.stop().await.unwrap();
}
#[tokio::test]
async fn terminal_runtime_native_refusal_is_visible_and_unrelated_finish_is_rejected() {
    let f=Fixture::new().await; f.native.store(true,std::sync::atomic::Ordering::Release); let h=f.start(); h.command(f.command("native","[de: peer] Olá")).await.unwrap();
    h.queue("refused".into(),Action::Finish {id:"native".into(),status:queue::Status::Rejected,result:json!({"operation_id":"native","disposition":"rejected","payload":{"native_status":"refused"}})}).await.unwrap();
    assert_eq!(f.state()["rows"][0]["desistiu"],true); h.drain().await.unwrap(); assert_eq!(f.io.socket_calls.lock().unwrap().len(),1);
    h.command(f.command("ordinary","Olá")).await.unwrap();
    assert!(h.queue("forge".into(),Action::Finish {id:"ordinary".into(),status:queue::Status::Accepted,result:json!({"operation_id":"ordinary","disposition":"accepted","payload":{"native_status":"delivered"}})}).await.is_err()); h.stop().await.unwrap();
}

#[tokio::test]
async fn terminal_runtime_clear_after_dispatch_holds_old_binding_until_detach() {
    let f=Fixture::new().await; f.io.fail_enter.store(true,std::sync::atomic::Ordering::Release); let h=f.start();
    let result=h.command(f.command("clear-now","/clear")).await.unwrap(); assert_eq!(result.disposition,hangar_server::runtime::protocol::Disposition::Unknown);
    assert_eq!(f.state()["runtime_state"]["preserve_binding"],true);
    let count=f.io.calls.lock().unwrap().len(); h.command(f.command("clear-now","/clear")).await.unwrap(); assert_eq!(f.io.calls.lock().unwrap().len(),count);
    assert!(h.command(f.command("old-life","Olá")).await.is_err());
    assert!(h.control("old-key".into(),"interactive_key".into(),json!({"key":"C-c"})).await.is_err());
    h.drain().await.unwrap(); assert_eq!(f.io.calls.lock().unwrap().len(),count); h.stop().await.unwrap();
}

#[tokio::test]
async fn terminal_runtime_restart_after_clear_dispatch_conserves_barrier() {
    let f=Fixture::new().await;
    let mut store=Store::open(&f.target.state_path,&f.target.projection_dir,queue::State::new("key",1,"session",vec![])).unwrap();
    let clock=ClockSample {monotonic_s:0.0,epoch_s:chrono::Utc::now().timestamp() as f64};
    store.exec(1,"prepare",clock,Action::Prepare {id:"clear-crash".into(),payload:serde_json::to_value(f.command("clear-crash","/clear")).unwrap(),entry_id:None}).unwrap();
    store.exec(1,"dispatch",clock,Action::BeginDispatch {id:"clear-crash".into(),wire_id:"terminal:1:clear-crash".into()}).unwrap(); drop(store);
    let h=f.start();
    assert!(h.command(f.command("after-crash","Olá")).await.is_err()); assert_eq!(f.state()["runtime_state"]["preserve_binding"],true);
    assert!(f.io.calls.lock().unwrap().is_empty()); h.stop().await.unwrap();
}

#[tokio::test]
async fn terminal_runtime_claim_crash_before_intent_restores_safe_pending_row() {
    let f=Fixture::new().await; f.ready.store(false,std::sync::atomic::Ordering::Release);
    let mut store=Store::open(&f.target.state_path,&f.target.projection_dir,queue::State::new("key",1,"session",vec![])).unwrap();
    let clock=ClockSample {monotonic_s:0.0,epoch_s:chrono::Utc::now().timestamp() as f64};
    store.exec(1,"append",clock,Action::Append {text:"Olá".into(),delivered:false,ts:None,pre_transcript:false,entry_id:Some("stray".into())}).unwrap();
    store.exec(1,"terminal:queue:999",clock,Action::Claim {min_ts:1.0,limit:Some(1),entry_id:None}).unwrap(); drop(store);
    let h=f.start(); h.snapshot().await.unwrap(); assert_eq!(f.state()["rows"][0]["delivered"],false);
    f.ready.store(true,std::sync::atomic::Ordering::Release); h.drain().await.unwrap(); assert_eq!(f.state()["rows"][0]["delivered"],true); assert_eq!(f.io.calls.lock().unwrap().iter().filter(|r|r.args[0]=="send-keys" && r.args.last().unwrap()=="Olá").count(),1); h.stop().await.unwrap();
}
#[tokio::test]
async fn terminal_runtime_claim_only_one_entry_while_driver_is_in_flight() {
    let f=Fixture::new().await; f.ready.store(false,std::sync::atomic::Ordering::Release); let h=f.start();
    h.command(f.command("first-claim","Um")).await.unwrap(); h.command(f.command("second-claim","Dois")).await.unwrap();
    f.io.blocked.store(true,std::sync::atomic::Ordering::Release); f.ready.store(true,std::sync::atomic::Ordering::Release);
    let hc=h.clone(); let draining=tokio::spawn(async move {hc.drain().await});
    tokio::time::timeout(Duration::from_secs(2),async {loop {if f.io.calls.lock().unwrap().iter().any(|r|r.args[0]=="send-keys"){break;}tokio::time::sleep(Duration::from_millis(5)).await;}}).await.unwrap();
    let rows=f.state()["rows"].clone(); assert_eq!(rows[0]["delivered"],true); assert_eq!(rows[1]["delivered"],false);
    f.io.blocked.store(false,std::sync::atomic::Ordering::Release); f.io.gate.notify_waiters(); draining.await.unwrap().unwrap(); h.stop().await.unwrap();
}

#[tokio::test]
async fn terminal_runtime_prepared_without_dispatch_is_deferred_not_unknown() {
    let f=Fixture::new().await; f.ready.store(false,std::sync::atomic::Ordering::Release);
    let mut store=Store::open(&f.target.state_path,&f.target.projection_dir,queue::State::new("key",1,"session",vec![])).unwrap();
    let clock=ClockSample {monotonic_s:0.0,epoch_s:chrono::Utc::now().timestamp() as f64}; let command=f.command("prepared","Olá");
    store.exec(1,"prepare",clock,Action::Prepare {id:"prepared".into(),payload:serde_json::to_value(&command).unwrap(),entry_id:Some("prepared".into())}).unwrap();
    store.exec(1,"append",clock,Action::Append {text:"Olá".into(),delivered:false,ts:None,pre_transcript:false,entry_id:Some("prepared".into())}).unwrap(); drop(store);
    let h=f.start(); assert_eq!(h.command(command).await.unwrap().disposition,hangar_server::runtime::protocol::Disposition::Deferred);
    assert_eq!(f.state()["rows"][0]["delivered"],false); h.stop().await.unwrap();
}

#[tokio::test]
async fn terminal_runtime_invalid_prompt_is_rejected_before_queue_and_dispatch() {
    let f=Fixture::new().await; let h=f.start();
    for (id,text) in [("empty",""),("blank"," \n\t"),("escape","bad\u{1b}"),("delete","bad\u{7f}")] {
        assert!(h.command(f.command(id,text)).await.is_err()); assert!(f.state()["operations"].get(id).is_none());
    }
    assert!(f.state()["rows"].as_array().unwrap().is_empty()); assert!(f.calls.lock().unwrap().is_empty()); assert!(f.io.calls.lock().unwrap().is_empty()); h.stop().await.unwrap();
}
#[tokio::test]
async fn terminal_runtime_interactive_text_keeps_whitespace_character() {
    let f=Fixture::new().await; let h=f.start();
    let result=h.control("space".into(),"terminal_input".into(),json!({"text":" "})).await.unwrap();
    assert_eq!(result.disposition,hangar_server::runtime::protocol::Disposition::Accepted);
    assert!(f.io.calls.lock().unwrap().iter().any(|r|r.args[0]=="send-keys" && r.args.last().unwrap()==" ")); h.stop().await.unwrap();
}

#[tokio::test]
async fn terminal_runtime_clear_new_conversation_after_enter_is_unknown_and_old_life_stays_blocked() {
    let f=Fixture::new().await; f.io.rotate_enter.store(true,std::sync::atomic::Ordering::Release); let h=f.start();
    let result=h.command(f.command("clear-changes-sid","/clear")).await.unwrap();
    assert_eq!(result.disposition,hangar_server::runtime::protocol::Disposition::Unknown); assert_eq!(result.payload["code"],"submit_unproved");
    assert_eq!(*f.io.conversation.lock().unwrap(),"new-sid"); let snapshot=h.snapshot().await.unwrap(); assert_eq!(snapshot["view"]["conversation"],"sid"); assert_eq!(snapshot["view"]["preserve_binding"],true);
    assert!(h.command(f.command("after-sid-change","Olá")).await.is_err()); h.stop().await.unwrap();
}
