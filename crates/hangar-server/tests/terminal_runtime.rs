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
const WAIT:Duration=Duration::from_secs(10);
/// Processo do teste que morre junto com ele, inclusive quando uma asserção falha antes do fim.
struct KillOnDrop(std::process::Child);
impl Drop for KillOnDrop {fn drop(&mut self){let _=self.0.kill();let _=self.0.wait();}}
struct Fixture { _dir:tempfile::TempDir,target:TerminalTarget,policy:PolicyClient,io:Arc<Io>, mux:Arc<Mutex<Vec<String>>>, idle:Arc<std::sync::atomic::AtomicBool>, ready:Arc<std::sync::atomic::AtomicBool>, generation:Arc<AtomicU64>, native:Arc<std::sync::atomic::AtomicBool>, control:Arc<Mutex<Value>>, unknown:Arc<std::sync::atomic::AtomicBool>, calls:Arc<Mutex<Vec<Value>>>,server:tokio::task::JoinHandle<()> }
impl Fixture {
    async fn new()->Self {
        let dir=tempfile::tempdir().unwrap(); let io=Arc::new(Io::new()); let conversation=io.conversation.clone();
        let binding=TerminalBinding {name:"session".into(),pane:"%1".into(),conversation:"sid".into(),generation:1,created:1,mux_argv:vec!["fake".into()],windows:false,clipboard_lock_path:None};
        let mux=Arc::new(Mutex::new(binding.mux_argv.clone()));let server_mux=mux.clone();
        let target=TerminalTarget {key:"key".into(),generation:1,name:"session".into(),binding:binding.clone(),lease_path:dir.path().join("lease"),state_path:dir.path().join("state"),projection_dir:dir.path().join("projection"),transcript:dir.path().join("chat.jsonl"),created:1.0};
        std::fs::write(&target.transcript,"").unwrap();
        let native=Arc::new(std::sync::atomic::AtomicBool::new(false)); let generation=Arc::new(AtomicU64::new(1)); let control=Arc::new(Mutex::new(json!({"disposition":"unavailable"})));
        let idle=Arc::new(std::sync::atomic::AtomicBool::new(true)); let ready=Arc::new(std::sync::atomic::AtomicBool::new(true)); let unknown=Arc::new(std::sync::atomic::AtomicBool::new(false)); let calls=Arc::new(Mutex::new(vec![]));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap(); let address=listener.local_addr().unwrap(); let t0=std::time::Instant::now();
        let state_path=target.state_path.clone();
        let (i,r,u,c,g,control_reply,n)=(idle.clone(),ready.clone(),unknown.clone(),calls.clone(),generation.clone(),control.clone(),native.clone());
        let router=axum::Router::new().route("/internal/runtime/policy",axum::routing::post(move |body:String| {let (i,r,u,c,mut b,path,g,control_reply,n,conversation,mux)=(i.clone(),r.clone(),u.clone(),c.clone(),binding.clone(),state_path.clone(),g.clone(),control_reply.clone(),n.clone(),conversation.clone(),server_mux.clone()); async move {
            let v:Value=serde_json::from_str(&body).unwrap();
            let state:Value=serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            let phase=&state["operations"][v["phase_id"].as_str().unwrap()];
            assert_eq!(phase["status"],"dispatching");
            assert_eq!(phase["payload"],json!({"kind":v["kind"],"request_id":v["request_id"],"payload":v["payload"]}));
            let mut seen=v.clone(); seen["_ms"]=json!(t0.elapsed().as_millis() as u64); c.lock().unwrap().push(seen);
            b.generation=g.load(std::sync::atomic::Ordering::Acquire); b.conversation=conversation.lock().unwrap().clone();b.mux_argv=mux.lock().unwrap().clone();
            let data=match v["kind"].as_str().unwrap() {
                "terminal_facts"=>json!({"binding":b,"ready":r.load(std::sync::atomic::Ordering::Acquire),"idle":i.load(std::sync::atomic::Ordering::Acquire),"open_question":false,"plugin_live":u.load(std::sync::atomic::Ordering::Acquire),"plugin_user":true,"native":if n.load(std::sync::atomic::Ordering::Acquire){Some(NativeMessage {socket:"fake".into(),origin:"peer".into(),sender:"peer".into(),mode:"message".into(),message_id:Some(format!("native:{}",v["payload"]["operation_id"].as_str().unwrap()))})}else{None}}),
                "terminal_publish"=>json!("unknown"),
                "terminal_plugin_control"=>control_reply.lock().unwrap().clone(), _=>panic!("unexpected policy")};
            ([("content-type","application/json")],json!({"ok":true,"data":data}).to_string())
        }}));
        let server=tokio::spawn(async move {axum::serve(listener,router).await.unwrap()});
        Self {_dir:dir,target,policy:PolicyClient::new(address,"test".into(),"instance".into()),io,mux,idle,ready,generation,native,control,unknown,calls,server}
    }
    fn start(&self)->hangar_server::runtime::terminal::TerminalHandle {
        self.start_with_events(broadcast::channel(128).0)
    }
    fn start_with_events(&self,events:broadcast::Sender<hangar_server::runtime::protocol::RuntimeEvent>)->hangar_server::runtime::terminal::TerminalHandle {
        let lease=queue::acquire_lease(&self.target.lease_path).unwrap();
        let store=Store::open(&self.target.state_path,&self.target.projection_dir,queue::State::new("key",1,"session",vec![])).unwrap();
        let options=TerminalOptions {io:self.io.clone(),limits:InputLimits {settle:Duration::ZERO,literal_settle:Duration::ZERO,multiline_settle:Duration::ZERO,slash_settle:Duration::ZERO,proof_attempts:1,ready_attempts:1,cleanup_attempts:1},tick:Duration::from_millis(15)};
        TerminalActor::spawn(self.target.clone(),QueueActor::start(store,lease),self.policy.clone(),options,events,Arc::new(AtomicU64::new(0)))
    }
    fn command(&self,id:&str,text:&str)->RuntimeCommand { RuntimeCommand {operation_id:id.into(),kind:OperationKind::Input,payload:json!({"text":text,"pre_transcript":false})} }
    fn state(&self)->Value {serde_json::from_slice(&std::fs::read(&self.target.state_path).unwrap()).unwrap()}
    /// Espera com prazo; no estouro mostra onde a operação parou (política, multiplexador e diário).
    /// O teto cobre o runner Windows, onde cada leitura de fatos grava o diário durável em ~0,1–0,3 s.
    async fn wait_for(&self,what:&str,mut done:impl FnMut()->bool) {
        let start=std::time::Instant::now();
        while !done() {
            if start.elapsed()>WAIT {
                let io:Vec<String>=self.io.calls.lock().unwrap().iter().map(|r|r.args[0].clone()).collect();
                let policy:Vec<String>=self.calls.lock().unwrap().iter().map(|v|format!("{}@{}ms",v["kind"].as_str().unwrap_or("?"),v["_ms"])).collect();
                let ops:Vec<String>=self.state()["operations"].as_object().map(|o|o.iter().map(|(k,v)|format!("{k}={}",v["status"])).collect()).unwrap_or_default();
                panic!("{what}: prazo de {WAIT:?} estourou; política={policy:?} io={io:?} operações={ops:?}");
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}
impl Drop for Fixture {fn drop(&mut self){self.server.abort();}}

#[tokio::test]
async fn real_process_timeout_ends_grandchild_before_detach_and_new_lease() {
    let mut f=Fixture::new().await;let script=f._dir.path().join("fake_mux.py");let late=f._dir.path().join("late-write");
    let code=format!(r#"import os,sys,time,subprocess
path={}
mode=sys.argv[1]
if mode in ('--child','--grand'):
 open(path+mode+'.pid','w').write(str(os.getpid()))
 if mode=='--child':subprocess.Popen([sys.executable,__file__,'--grand'])
 else:time.sleep(2.5);open(path,'w').write('late')
 time.sleep(10)
elif mode=='--other':time.sleep(60)
elif mode=='display-message':print('session\t%1\t1')
elif mode=='capture-pane':print('─'*32+'\n❯ \n'+'─'*32)
elif mode=='send-keys' and '-l' in sys.argv:
 subprocess.Popen([sys.executable,__file__,'--child'])
 # O prazo só pode vencer com o neto já nascido; senão o teste não prova nada.
 deadline=time.monotonic()+10
 while not os.path.exists(path+'--grand.pid') and time.monotonic()<deadline:time.sleep(.005)
 time.sleep(10)
"#,json!(late.to_str().unwrap()));
    std::fs::write(&script,code).unwrap();
    let python=std::env::var("HANGAR_TEST_PYTHON").unwrap_or_else(|_|if cfg!(windows){"python".into()}else{"python3".into()});
    let mut other=KillOnDrop(std::process::Command::new(&python).arg(&script).arg("--other").spawn().unwrap());let other_born=std::time::Instant::now();
    f.target.binding.mux_argv=vec![python,"-X".into(),"utf8".into(),script.to_str().unwrap().into()];*f.mux.lock().unwrap()=f.target.binding.mux_argv.clone();
    let lease=queue::acquire_lease(&f.target.lease_path).unwrap();let store=Store::open(&f.target.state_path,&f.target.projection_dir,queue::State::new("key",1,"session",vec![])).unwrap();
    let options=TerminalOptions {io:Arc::new(ProcessIo {command_timeout:Duration::from_millis(1500),socket_timeout:Duration::from_millis(150)}),
        limits:InputLimits {settle:Duration::ZERO,literal_settle:Duration::ZERO,proof_attempts:1,ready_attempts:1,cleanup_attempts:1,..InputLimits::default()},tick:Duration::from_secs(10)};
    let h=TerminalActor::spawn(f.target.clone(),QueueActor::start(store,lease),f.policy.clone(),options,broadcast::channel(128).0,Arc::new(AtomicU64::new(0)));
    let result=tokio::time::timeout(Duration::from_secs(10),h.command(f.command("timeout","A"))).await.unwrap().unwrap();
    assert_eq!(result.disposition,hangar_server::runtime::protocol::Disposition::Unknown);
    h.stop().await.unwrap();let python_lease=queue::acquire_lease(&f.target.lease_path).unwrap();
    let grandchild_born=std::path::Path::new(&format!("{}--grand.pid",late.display())).exists();
    // O neto escreveria 2,5 s depois de nascer, e ele nasce antes do prazo de 1,5 s.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let old_writer=late.exists();let other_exit=other.0.try_wait().unwrap();let other_age=other_born.elapsed();let unrelated_alive=other_exit.is_none();
    for suffix in if old_writer {vec!["--child.pid","--grand.pid"]}else{vec![]} {
        if let Ok(pid)=std::fs::read_to_string(format!("{}{suffix}",late.display())) {
            #[cfg(unix)] {let _=std::process::Command::new("kill").args(["-9",pid.trim()]).output();}
            #[cfg(windows)] {let _=std::process::Command::new("taskkill.exe").args(["/PID",pid.trim(),"/T","/F"]).output();}
        }
    }
    drop(other);drop(python_lease);
    assert!(grandchild_born,"timeout fired before the grandchild existed; nothing was proved");
    // Saída 0 é o `sleep` do processo alheio que acabou sozinho; outro código é morte por terceiro.
    assert!(unrelated_alive,"processo alheio saiu: {other_exit:?} depois de {other_age:?}");assert!(!old_writer,"auxiliary grandchild wrote after detach released the lease");
}

#[cfg(target_os="linux")]
#[tokio::test]
async fn command_leader_stays_unreaped_until_its_group_is_gone() {
    // O neto vigia o líder: se o número dele some enquanto o grupo ainda vive, outro processo
    // poderia herdá-lo e levar o SIGKILL do grupo.
    let dir=tempfile::tempdir().unwrap();let script=dir.path().join("leader.py");let base=dir.path().join("probe");
    let code=format!(r#"import os,sys,time,subprocess
base={}
if sys.argv[1]=='--grand':
 leader=sys.argv[2];open(base+".ready","w").write("1")
 while True:
  if not os.path.exists('/proc/'+leader):open(base+'.reaped','w').write('1');break
  time.sleep(.0002)
 time.sleep(10)
else:
 subprocess.Popen([sys.executable,__file__,'--grand',str(os.getpid())],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL)
 deadline=time.monotonic()+5
 while not os.path.exists(base+'.ready') and time.monotonic()<deadline:time.sleep(.005)
 print('leader-done')
"#,json!(base.to_str().unwrap()));
    std::fs::write(&script,code).unwrap();
    let python=std::env::var("HANGAR_TEST_PYTHON").unwrap_or_else(|_|"python3".into());
    let io=ProcessIo {command_timeout:Duration::from_secs(10),socket_timeout:Duration::from_secs(1)};
    for _ in 0..5 {
        for suffix in [".ready",".reaped"] {let _=std::fs::remove_file(format!("{}{suffix}",base.display()));}
        let output=io.command(CommandRequest {program:python.clone(),args:vec![script.to_str().unwrap().into(),"--leader".into()],stdin:vec![]}).await.unwrap();
        assert!(output.success);assert_eq!(String::from_utf8_lossy(&output.stdout).trim(),"leader-done");
        assert!(std::path::Path::new(&format!("{}.ready",base.display())).exists(),"grandchild never started");
        tokio::time::sleep(Duration::from_millis(100)).await;
        let reaped=std::path::Path::new(&format!("{}.reaped",base.display())).exists();
        assert!(!reaped,"leader was reaped while its group was still alive");
    }
}

#[tokio::test]
async fn terminal_v2_prepare_confirmed_has_zero_policy_or_key() {
    let f=Fixture::new().await;
    let initial=queue::State::new("key",1,"session",vec![json!({"id":"root","text":"fixture-input","ts":1.0,"delivered":true,"confirmed":true})]);
    std::fs::write(&f.target.state_path,serde_json::to_vec(&initial).unwrap()).unwrap();
    let h=f.start();let result=h.command(f.command("root","fixture-input")).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().disposition,hangar_server::runtime::protocol::Disposition::Accepted);
    assert!(f.calls.lock().unwrap().is_empty());assert!(f.io.calls.lock().unwrap().is_empty());
    h.stop().await.unwrap();
}

#[tokio::test]
async fn terminal_v2_sequence_uses_durable_watermark() {
    let f=Fixture::new().await;let mut initial=queue::State::new("key",1,"session",vec![]);initial.next_seq=50_000;
    std::fs::write(&f.target.state_path,serde_json::to_vec(&initial).unwrap()).unwrap();
    let h=f.start();h.command(f.command("root","fixture-input")).await.unwrap();
    let state=f.state();
    assert!(state["operations"].as_object().unwrap().keys().filter(|id|id.starts_with("call::terminal:")||id.starts_with("terminal-policy:"))
        .all(|id|id.rsplit(':').next().unwrap().parse::<u64>().unwrap()>=50_000));
    h.stop().await.unwrap();
}

#[tokio::test]
async fn unknown_fill_blocks_second_input_after_detach_restart_and_same_sid_generation() {
    let mut f=Fixture::new().await;f.unknown.store(true,std::sync::atomic::Ordering::Release);
    let h=f.start();assert_eq!(h.command(f.command("A","A")).await.unwrap().disposition,hangar_server::runtime::protocol::Disposition::Unknown);
    *f.io.text.lock().unwrap()="A".into();
    h.queue("append-B".into(),Action::Append {text:"B".into(),delivered:false,ts:None,pre_transcript:false,entry_id:Some("B".into())}).await.unwrap();
    h.stop().await.unwrap();
    let mut state=f.state();state["generation"]=json!(2);
    std::fs::write(&f.target.state_path,serde_json::to_vec(&state).unwrap()).unwrap();
    f.target.generation=2;f.target.binding.generation=2;f.generation.store(2,std::sync::atomic::Ordering::Release);
    let effects=f.io.calls.lock().unwrap().len();let publications=f.calls.lock().unwrap().len();
    f.unknown.store(false,std::sync::atomic::Ordering::Release);
    let lease=queue::acquire_lease(&f.target.lease_path).unwrap();
    let store=Store::open(&f.target.state_path,&f.target.projection_dir,queue::State::new("key",2,"session",vec![])).unwrap();
    let options=TerminalOptions {io:f.io.clone(),limits:InputLimits::default(),tick:Duration::from_millis(15)};
    let h=TerminalActor::spawn(f.target.clone(),QueueActor::start(store,lease),f.policy.clone(),options,broadcast::channel(128).0,Arc::new(AtomicU64::new(0)));
    assert_eq!(h.drain().await.unwrap()["drained"],0);
    assert_eq!(h.command(f.command("C","C")).await.unwrap().disposition,hangar_server::runtime::protocol::Disposition::Deferred);
    assert_eq!(f.io.calls.lock().unwrap().len(),effects);assert_eq!(f.calls.lock().unwrap().len(),publications);
    assert_eq!(*f.io.text.lock().unwrap(),"A");h.stop().await.unwrap();
}

#[tokio::test]
async fn clear_with_arguments_passes_terminal_write_barrier_like_python() {
    for text in ["/clear","  /clear keep"] {
        let f=Fixture::new().await;f.unknown.store(true,std::sync::atomic::Ordering::Release);
        let h=f.start();assert_eq!(h.command(f.command("A","A")).await.unwrap().disposition,hangar_server::runtime::protocol::Disposition::Unknown);
        f.unknown.store(false,std::sync::atomic::Ordering::Release);
        let result=h.command(f.command("clear",text)).await.unwrap();
        assert_ne!(result.payload["code"],"terminal_write_barrier","{text}");
        h.stop().await.unwrap();
    }
}

#[tokio::test]
async fn terminal_runtime_maintenance_failure_publishes_problem_and_stops_uncoordinated_retry() {
    let f=Fixture::new().await;
    let (events,mut receiver)=broadcast::channel(128); let h=f.start_with_events(events);
    h.command(f.command("accepted","Olá")).await.unwrap();
    std::fs::remove_file(&f.target.transcript).unwrap();
    std::fs::create_dir(&f.target.transcript).unwrap();
    let problem=tokio::time::timeout(WAIT,async {
        loop {let event=receiver.recv().await.unwrap();if event.channel=="problem"{break event;}}
    }).await.unwrap();
    assert_eq!(problem.data["error_code"],"receipt_scan");
    assert_eq!(h.snapshot().await.unwrap()["error"],"receipt_scan");
    let count=f.state()["operations"].as_object().unwrap().len();
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(f.state()["operations"].as_object().unwrap().len(),count);
    std::fs::remove_dir(&f.target.transcript).unwrap();
    std::fs::write(&f.target.transcript,"").unwrap();
    h.confirm().await.unwrap();
    assert!(h.snapshot().await.unwrap()["error"].is_null());
    h.stop().await.unwrap();
}

#[tokio::test]
async fn terminal_runtime_unknown_delivery_is_signaled_without_retyping() {
    let f=Fixture::new().await; f.unknown.store(true,std::sync::atomic::Ordering::Release);
    let (events,mut receiver)=broadcast::channel(128); let h=f.start_with_events(events);
    let command=f.command("unknown","Olá");
    assert_eq!(h.command(command.clone()).await.unwrap().disposition,hangar_server::runtime::protocol::Disposition::Unknown);
    let problem=tokio::time::timeout(WAIT,async {
        loop {let event=receiver.recv().await.unwrap();if event.channel=="problem"{break event;}}
    }).await.unwrap();
    assert_eq!(problem.data["error_code"],"terminal_delivery_unknown");
    assert_eq!(h.snapshot().await.unwrap()["error"],"terminal_delivery_unknown");
    let calls=f.io.calls.lock().unwrap().len();
    h.command(command).await.unwrap(); tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(f.io.calls.lock().unwrap().len(),calls);
    h.stop().await.unwrap();
}

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
    f.wait_for("espera 1",||f.state()["rows"].as_array().unwrap().iter().all(|r|r["delivered"]==true)).await;
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
    f.wait_for("espera 2",||f.io.calls.lock().unwrap().iter().any(|r|r.args[0]=="send-keys")).await;
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
    f.wait_for("espera 3",||f.state()["rows"][0]["desistiu"]==true).await;
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
    f.wait_for("espera 4",||f.io.calls.lock().unwrap().iter().any(|r|r.args[0]=="send-keys")).await;
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
    f.wait_for("espera 5",||f.io.calls.lock().unwrap().iter().any(|r|r.args[0]=="send-keys")).await;
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
async fn terminal_runtime_clear_new_conversation_after_enter_is_accepted_and_old_life_stays_blocked() {
    let f=Fixture::new().await; f.io.rotate_enter.store(true,std::sync::atomic::Ordering::Release); let h=f.start();
    let result=h.command(f.command("clear-changes-sid","/clear")).await.unwrap();
    // A troca de conversa é o efeito do próprio /clear: o composer vazio no mesmo pane prova a submissão.
    assert_eq!(result.disposition,hangar_server::runtime::protocol::Disposition::Accepted,"{}",result.payload); assert_eq!(result.payload["code"],"submitted");
    assert_eq!(*f.io.conversation.lock().unwrap(),"new-sid"); let snapshot=h.snapshot().await.unwrap(); assert_eq!(snapshot["view"]["conversation"],"sid"); assert_eq!(snapshot["view"]["preserve_binding"],true);
    assert!(h.command(f.command("after-sid-change","Olá")).await.is_err()); h.stop().await.unwrap();
}

#[tokio::test]
async fn terminal_runtime_prepare_without_append_restores_prompt_and_delivers_once() {
    let f=Fixture::new().await; f.ready.store(false,std::sync::atomic::Ordering::Release);
    let text="Olá 🌎 C:\\text — 📎 imagem: /tmp/x.png";
    let mut command=f.command("prepared-missing",text); command.payload["pre_transcript"]=json!(true);
    let mut original=serde_json::to_value(&command).unwrap(); original["payload"]["_terminal_generation"]=json!(1);
    let mut store=Store::open(&f.target.state_path,&f.target.projection_dir,queue::State::new("key",1,"session",vec![])).unwrap();
    store.exec(1,"terminal:queue:1",ClockSample {monotonic_s:0.0,epoch_s:chrono::Utc::now().timestamp() as f64},Action::Prepare {id:command.operation_id.clone(),payload:original,entry_id:Some(command.operation_id.clone())}).unwrap(); drop(store);
    let h=f.start(); h.snapshot().await.unwrap();
    let rows=f.state()["rows"].clone(); assert_eq!(rows.as_array().unwrap().len(),1); assert_eq!(rows[0]["id"],"prepared-missing"); assert_eq!(rows[0]["text"],text); assert_eq!(rows[0]["pre_transcript"],true);
    let replay=h.command(command.clone()).await.unwrap(); assert_eq!(replay.disposition,hangar_server::runtime::protocol::Disposition::Deferred);
    f.ready.store(true,std::sync::atomic::Ordering::Release); h.drain().await.unwrap(); h.command(command).await.unwrap(); h.drain().await.unwrap();
    assert_eq!(f.io.calls.lock().unwrap().iter().filter(|r|r.args[0]=="send-keys" && r.args.last().unwrap()==text).count(),1); assert_eq!(f.state()["rows"].as_array().unwrap().len(),1); h.stop().await.unwrap();
}
#[tokio::test]
async fn terminal_runtime_native_receipt_matches_queue_append_without_submit_root() {
    for (native_status,status) in [("delivered",queue::Status::Accepted),("refused",queue::Status::Rejected)] {
        let f=Fixture::new().await; f.ready.store(false,std::sync::atomic::Ordering::Release); let h=f.start();
        h.queue("producer".into(),Action::Append {text:"[de: peer] Olá".into(),delivered:false,ts:None,pre_transcript:false,entry_id:Some("producer-entry".into())}).await.unwrap();
        f.ready.store(true,std::sync::atomic::Ordering::Release); f.native.store(true,std::sync::atomic::Ordering::Release); h.drain().await.unwrap();
        assert!(f.state()["operations"].get("producer-entry").is_none());
        h.queue("native-ack".into(),Action::Finish {id:"producer-entry".into(),status,result:json!({"operation_id":"producer-entry","disposition":if native_status=="delivered"{"accepted"}else{"rejected"},"payload":{"native_status":native_status}})}).await.unwrap();
        let row=f.state()["rows"][0].clone(); assert_ne!(row["confirmed"],true); if native_status=="refused" {assert_eq!(row["desistiu"],true);}
        h.drain().await.unwrap(); assert_eq!(f.io.socket_calls.lock().unwrap().len(),1); assert!(f.io.calls.lock().unwrap().iter().all(|r|r.args[0]!="send-keys")); h.stop().await.unwrap();
    }
}

#[tokio::test]
async fn terminal_runtime_input_primitive_payload_is_rejected_without_stopping_actor() {
    let f=Fixture::new().await; let h=f.start();
    for payload in [Value::Null,json!(false),json!([]),json!("")] {
        assert!(h.control("malformed".into(),"input".into(),payload).await.is_err()); h.snapshot().await.unwrap();
    }
    assert!(f.state()["rows"].as_array().unwrap().is_empty());assert!(f.io.calls.lock().unwrap().is_empty());h.stop().await.unwrap();
}

#[tokio::test]
async fn terminal_runtime_removed_recovered_root_has_no_further_delivery_or_publication() {
    for exhausted in [false,true] {
        let f=Fixture::new().await;let clock=ClockSample {monotonic_s:0.0,epoch_s:chrono::Utc::now().timestamp() as f64};
        let mut original=serde_json::to_value(f.command("removed-root","[de: peer] Olá")).unwrap();original["payload"]["_terminal_generation"]=json!(1);
        let mut store=Store::open(&f.target.state_path,&f.target.projection_dir,queue::State::new("key",1,"session",vec![])).unwrap();
        store.exec(1,"prepare-root",clock,Action::Prepare {id:"removed-root".into(),entry_id:Some("removed-root".into()),payload:original}).unwrap();
        store.exec(1,"recover-create",clock,Action::Recover).unwrap();
        if exhausted {for n in 0..2 {store.exec(1,&format!("bump:{n}"),clock,Action::BumpAttempts {entry_id:"removed-root".into()}).unwrap();}}
        store.exec(1,"prepare-attempt",clock,Action::Prepare {id:"old-attempt".into(),entry_id:Some("removed-root".into()),payload:json!({"operation_id":"old-attempt","kind":"input","payload":{"text":"[de: peer] Olá","_terminal_generation":1}})}).unwrap();
        store.exec(1,"dispatch",clock,Action::BeginDispatch {id:"old-attempt".into(),wire_id:"terminal:1:old-attempt".into()}).unwrap();
        store.exec(1,"finish",clock,Action::Finish {id:"old-attempt".into(),status:if exhausted{queue::Status::Deferred}else{queue::Status::Rejected},result:json!({"operation_id":"old-attempt","disposition":if exhausted{"deferred"}else{"rejected"},"payload":{"cleanup":"proved"}})}).unwrap();
        store.exec(1,"remove",clock,Action::Remove {entry_id:"removed-root".into()}).unwrap();drop(store);
        f.native.store(true,std::sync::atomic::Ordering::Release);f.unknown.store(true,std::sync::atomic::Ordering::Release);
        let h=f.start();h.snapshot().await.unwrap();h.drain().await.unwrap();h.drain().await.unwrap();
        assert!(f.state()["rows"].as_array().unwrap().is_empty());assert!(f.calls.lock().unwrap().is_empty());assert!(f.io.socket_calls.lock().unwrap().is_empty());assert!(f.io.calls.lock().unwrap().is_empty());h.stop().await.unwrap();
    }
}
