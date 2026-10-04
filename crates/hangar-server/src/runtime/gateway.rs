use super::{actor::{PolicyClient,RuntimeActor,RuntimeEngine,RuntimeHandle},cano,protocol::*,queue::{Action,QueueActor,State as QueueState,Store,acquire_lease}};
use axum::{Router,body::to_bytes,extract::{ConnectInfo,State,Request},http::StatusCode,
    middleware::{self,Next},response::{IntoResponse,Response,sse::{Event,KeepAlive,Sse}},routing::{get,post}};
use serde::Deserialize;
use serde_json::{Value,json};
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration,SystemTime,UNIX_EPOCH};
use subtle::ConstantTimeEq;
use tokio::sync::{broadcast,Mutex};

#[derive(Clone)]
enum EntryHandle { Headless(RuntimeHandle), Terminal {target:super::terminal::TerminalTarget,handle:super::terminal::TerminalHandle} }
impl EntryHandle {
    async fn snapshot(&self)->Result<Value,RuntimeError> {match self {Self::Headless(h)=>h.snapshot().await,Self::Terminal {handle,..}=>handle.snapshot().await}}
    async fn stop(&self)->Result<(),RuntimeError> {match self {Self::Headless(h)=>h.stop().await,Self::Terminal {handle,..}=>handle.stop().await}}
    async fn command(&self,command:RuntimeCommand)->Result<RuntimeReply,RuntimeError> {match self {Self::Headless(h)=>h.command(command).await,Self::Terminal {handle,..}=>handle.command(command).await}}
    async fn queue(&self,id:String,action:Action)->Result<Value,RuntimeError> {match self {Self::Headless(h)=>h.queue(id,action).await,Self::Terminal {handle,..}=>handle.queue(id,action).await}}
    async fn drain(&self)->Result<Value,RuntimeError> {match self {Self::Headless(h)=>h.drain().await,Self::Terminal {handle,..}=>handle.drain().await}}
    async fn confirm(&self)->Result<Value,RuntimeError> {match self {Self::Headless(h)=>h.confirm().await,Self::Terminal {handle,..}=>handle.confirm().await}}
    async fn ensure_projection(&self)->Result<Value,RuntimeError> {match self {Self::Headless(h)=>h.ensure_projection().await,Self::Terminal {handle,..}=>handle.ensure_projection().await}}
}
struct Entry { generation:u64,handle:EntryHandle,lease_path:std::path::PathBuf }
pub struct RuntimeRegistry {
    entries:Mutex<BTreeMap<String,Entry>>,
    events:broadcast::Sender<RuntimeEvent>,
    policy:PolicyClient,
    instance:String,
    lifecycle:Mutex<BTreeMap<String,Arc<Mutex<()>>>>,
    revisions:Mutex<BTreeMap<String,Arc<AtomicU64>>>,
}

/// A trava fica com as tarefas de E/S do ator até elas saírem; espera até 3 s por isso.
async fn lease_released(path:&std::path::Path) -> bool {
    for _ in 0..60 {
        let path = path.to_owned();
        if tokio::task::spawn_blocking(move ||acquire_lease(&path).is_ok()).await.unwrap_or(false) { return true; }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

fn failure(code:&str) -> RuntimeError { RuntimeError::new(code,"runtime indisponível para esta chave ou geração") }

impl RuntimeRegistry {
    pub fn new(upstream:SocketAddr,secret:String,instance:String) -> Self {
        Self { entries:Mutex::new(BTreeMap::new()),events:broadcast::channel(1024).0,
            policy:PolicyClient::new(upstream,secret,instance.clone()),instance,lifecycle:Mutex::new(BTreeMap::new()),revisions:Mutex::new(BTreeMap::new()) }
    }
    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> { self.events.subscribe() }
    pub async fn handle(&self,key:&str,generation:u64) -> Result<RuntimeHandle,RuntimeError> {
        match self.entry(key,generation).await? {EntryHandle::Headless(handle)=>Ok(handle),_=>Err(failure("runtime_provider"))}
    }
    async fn entry(&self,key:&str,generation:u64)->Result<EntryHandle,RuntimeError> {
        self.entries.lock().await.get(key).filter(|entry|entry.generation==generation)
            .map(|entry|entry.handle.clone()).ok_or_else(||failure("runtime_binding"))
    }
    pub async fn adopt(&self,target:RuntimeTarget,carry:Value) -> Result<Value,RuntimeError> {
        let barrier = self.barrier(&target.key).await;
        let _guard = barrier.lock().await;
        let existing = self.entries.lock().await.get(&target.key).map(|entry|(entry.generation,entry.handle.clone()));
        let handle = if let Some((generation,handle)) = existing {
            if generation != target.generation { return Err(failure("runtime_generation")); }
            match handle {EntryHandle::Headless(handle)=>handle,_=>return Err(failure("runtime_provider"))}
        } else {
        if !["claude","codex"].contains(&target.provider.as_str()) || target.binding.versao != 2 { return Err(failure("runtime_provider")); }
        let lease = acquire_lease(&target.lease_path).map_err(|_|failure("runtime_lease"))?;
        let state_path = target.state_path.clone(); let projection_dir = target.projection_dir.clone();
        let initial = QueueState::new(&target.key,target.generation,&target.name,Vec::new());
        let opening_lease = lease.clone();
        let store = tokio::task::spawn_blocking(move || {
            let _lease = opening_lease;
            Store::open(&state_path,&projection_dir,initial)
        }).await
            .map_err(|_|failure("queue_job"))?.map_err(|error|{
                // A frase da recusa da fila é fixa e diz por que a adoção falhou; o resto só pelo tipo.
                let reason = super::queue::refusal(&error).map_or_else(||format!("{:?}",error.kind()),str::to_owned);
                RuntimeError::new("queue_io",&format!("fila recusou: {reason}"))
            })?;
        if store.state().generation != target.generation { return Err(failure("runtime_generation")); }
        let mut metadata = target.metadata.clone();
        if let Some(fields) = store.state().runtime_state["view"].as_object() {
            if fields.get("conversation").is_some_and(|conversation|conversation == &metadata[if target.provider == "claude" { "session_id" } else { "thread_id" }]) {
                for (key,value) in fields { if key != "public_state" { metadata[key] = value.clone(); } }
            }
        }
        if let Some(fields) = carry["runtime_state"].as_object() { for (key,value) in fields { metadata[key] = value.clone(); } }
        let queue = QueueActor::start(store,lease);
        let connection = match cano::connect(&target.binding).await {
            Ok(connection)=>connection,
            Err(error)=>{ queue.shutdown().await.map_err(|_|failure("queue_stop"))?; return Err(error); },
        };
        let epoch_s = SystemTime::now().duration_since(UNIX_EPOCH).map(|d|d.as_secs_f64()).unwrap_or(0.0);
        let revision = self.revisions.lock().await.entry(target.key.clone()).or_insert_with(||Arc::new(AtomicU64::new(0))).clone();
        let engine = RuntimeEngine::new(&target.provider,metadata,target.generation,ClockSample { monotonic_s:0.0,epoch_s })?
            .with_policy(self.policy.clone()).with_publisher(self.events.clone()).with_revision(revision);
        let handle = RuntimeActor::spawn(target.clone(),queue,connection,engine);
        self.entries.lock().await.insert(target.key.clone(),Entry { generation:target.generation,handle:EntryHandle::Headless(handle.clone()),lease_path:target.lease_path.clone() });
        handle
        };
        let deadline = tokio::time::Instant::now()+Duration::from_secs(180);
        loop {
            let snapshot = handle.snapshot().await?;
            let view = &snapshot["view"];
            if !snapshot["error"].is_null() { return Err(failure("runtime_prepare")); }
            if view["alive"] == false { return Err(failure("cano_exited")); }
            let ready = if target.provider == "claude" { view["initialized"] == true } else { view["ready"] == true };
            if ready {
                handle.ensure_projection().await?;
                return Ok(json!({"ready":true,"instance":self.instance,"key":target.key,"generation":target.generation,"state":snapshot}));
            }
            if tokio::time::Instant::now() >= deadline { return Err(failure("runtime_initialize")); }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    pub async fn adopt_terminal(&self,target:super::terminal::TerminalTarget)->Result<Value,RuntimeError> {
        let barrier=self.barrier(&target.key).await; let _guard=barrier.lock().await;
        let existing=self.entries.lock().await.get(&target.key).map(|e|(e.generation,e.handle.clone()));
        let handle=match existing {
            Some((generation,EntryHandle::Terminal {target:old,handle})) if generation==target.generation && old.binding==target.binding
                && old.transcript==target.transcript && old.state_path==target.state_path && old.projection_dir==target.projection_dir && old.lease_path==target.lease_path=>handle,
            Some(_)=>return Err(failure("runtime_generation")),
            None=>{
                let lease=acquire_lease(&target.lease_path).map_err(|_|failure("runtime_lease"))?;
                let (state_path,projection_dir,initial)=(target.state_path.clone(),target.projection_dir.clone(),QueueState::new(&target.key,target.generation,&target.name,Vec::new()));
                let opening_lease=lease.clone();
                let store=tokio::task::spawn_blocking(move ||{let _lease=opening_lease;Store::open(&state_path,&projection_dir,initial)}).await
                    .map_err(|_|failure("queue_job"))?.map_err(|_|failure("queue_io"))?;
                if store.state().generation!=target.generation{return Err(failure("runtime_generation"));}
                let revision=self.revisions.lock().await.entry(target.key.clone()).or_insert_with(||Arc::new(AtomicU64::new(0))).clone();
                let handle=super::terminal::TerminalActor::spawn(target.clone(),QueueActor::start(store,lease),self.policy.clone(),super::terminal::TerminalOptions::default(),self.events.clone(),revision);
                self.entries.lock().await.insert(target.key.clone(),Entry {generation:target.generation,handle:EntryHandle::Terminal {target:target.clone(),handle:handle.clone()},lease_path:target.lease_path.clone()});handle
            }
        };
        let snapshot=handle.snapshot().await?;
        Ok(json!({"ready":true,"instance":self.instance,"key":target.key,"generation":target.generation,"state":snapshot}))
    }
    pub async fn detach(&self,key:&str,generation:u64) -> Result<Value,RuntimeError> {
        let barrier = self.barrier(key).await;
        let _guard = barrier.lock().await;
        let (handle,lease_path) = match self.entries.lock().await.get(key) {
            None=>return Ok(json!({"detached":true})),
            Some(entry) if entry.generation == generation=>(entry.handle.clone(),entry.lease_path.clone()),
            _=>return Err(failure("runtime_generation")),
        };
        if let Err(error) = handle.stop().await {
            // `stop` sempre junta a tarefa do ator: se ele saiu por erro, a posse acaba com ele. Sem
            // isto a entrada morta ficava para sempre, a sessão não voltava ao Python e o retrato de
            // eventos de todas as sessões caía. Só solta depois de a trava estar livre de fato.
            if !lease_released(&lease_path).await {
                tracing::warn!(key,code=%error.code,"ator terminou mas a trava não liberou em 3 s; sessão segue presa");
                return Err(error);
            }
            tracing::warn!(key,code=%error.code,"ator do runtime já tinha terminado; sessão liberada");
        }
        self.entries.lock().await.remove(key);
        Ok(json!({"detached":true}))
    }
    async fn barrier(&self,key:&str) -> Arc<Mutex<()>> {
        self.lifecycle.lock().await.entry(key.into()).or_insert_with(||Arc::new(Mutex::new(()))).clone()
    }
    pub async fn snapshots(&self) -> Result<Vec<RuntimeEvent>,RuntimeError> {
        let entries:Vec<_> = self.entries.lock().await.iter().map(|(key,entry)|(key.clone(),entry.generation,entry.handle.clone())).collect();
        let mut output = Vec::new();
        for (key,generation,handle) in entries {
            // Uma sessão sem ator não tira o retrato das outras: ela fica de fora até ser liberada.
            match handle.snapshot().await {
                Ok(data)=>output.push(RuntimeEvent { key,generation,revision:data["revision"].as_u64().unwrap_or(0),channel:"snapshot".into(),data }),
                Err(error)=>if crate::warn_limit::allow(Some(&key),&error.code) { tracing::warn!(key,code=%error.code,"sessão fora do retrato inicial dos eventos") },
            }
        }
        Ok(output)
    }
    pub async fn shutdown(&self) -> Result<(),RuntimeError> {
        let entries:Vec<_> = self.entries.lock().await.iter().map(|(key,entry)|(key.clone(),entry.generation)).collect();
        for (key,generation) in entries { self.detach(&key,generation).await?; }
        Ok(())
    }
}

#[derive(Clone)]
struct Gateway { registry:Arc<RuntimeRegistry>,secret:String,instance:String,protocol:u32 }

pub fn startup_line(protocol:u32,instance:&str,port:u16) -> String {
    json!({"type":"runtime_ready","protocol":protocol,"instance":instance,"port":port}).to_string()
}

pub async fn serve(listener:tokio::net::TcpListener,registry:Arc<RuntimeRegistry>,secret:String,instance:String,protocol:u32) -> std::io::Result<()> {
    if !listener.local_addr()?.ip().is_loopback() { return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied,"IPC somente no loopback")); }
    let state = Gateway { registry,secret,instance,protocol };
    let router = Router::new().route("/runtime/op",post(operation)).route("/runtime/events",get(events))
        .layer(middleware::from_fn_with_state(state.clone(),authorize)).with_state(state);
    axum::serve(listener,router.into_make_service_with_connect_info::<SocketAddr>()).await
}

async fn authorize(State(state):State<Gateway>,request:Request,next:Next) -> Response {
    let local = request.extensions().get::<ConnectInfo<SocketAddr>>().is_some_and(|info|info.0.ip().is_loopback());
    let secret = request.headers().get("x-hangar-internal").map(|h|h.as_bytes()).unwrap_or(&[]);
    let instance = request.headers().get("x-hangar-runtime-instance").map(|h|h.as_bytes()).unwrap_or(&[]);
    if !local || secret.ct_eq(state.secret.as_bytes()).unwrap_u8() != 1 || instance.ct_eq(state.instance.as_bytes()).unwrap_u8() != 1 {
        return StatusCode::NOT_FOUND.into_response();
    }
    next.run(request).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    protocol:u32,instance:String,key:String,generation:u64,operation_id:String,clock:ClockSample,command:Value,
}

async fn operation(State(state):State<Gateway>,request:Request) -> Response {
    let body = match to_bytes(request.into_body(),MAX_ENVELOPE).await { Ok(body)=>body,Err(_)=>return refuse(StatusCode::PAYLOAD_TOO_LARGE,None,"body") };
    let envelope:Envelope = match serde_json::from_slice(&body) { Ok(envelope)=>envelope,Err(_)=>return refuse(StatusCode::BAD_REQUEST,None,"envelope") };
    let _ = envelope.clock;
    let mismatch = if envelope.protocol != state.protocol { Some("protocol") } else if envelope.instance != state.instance { Some("instance") }
        else if envelope.operation_id.is_empty() { Some("operation_id") } else { None };
    if let Some(check) = mismatch { return refuse(StatusCode::CONFLICT,Some(&envelope.key),check); }
    let result = dispatch(&state.registry,&envelope).await;
    match result {
        Ok(result)=>json_response(StatusCode::OK,json!({"ok":true,"result":result})),
        Err(error)=>{
            if crate::warn_limit::allow(Some(&envelope.key),&error.code) {
                // Só o nome vem do descritor: o resto dele é caminho e credencial do cano.
                let name = envelope.command["descriptor"]["name"].as_str().unwrap_or("");
                tracing::warn!(key=%envelope.key,session=%name,kind=%envelope.command["kind"].as_str().unwrap_or("?"),
                    code=%error.code,reason=%error.message,"runtime recusou operação");
            }
            json_response(StatusCode::SERVICE_UNAVAILABLE,json!({"ok":false,"error_code":error.code,"message":error.message}))
        }
    }
}

/// Recusa antes do despacho: diz qual conferência falhou, sem o corpo.
fn refuse(status:StatusCode,key:Option<&str>,check:&'static str) -> Response {
    if crate::warn_limit::allow(key,check) {
        tracing::warn!(status=status.as_u16(),key=%key.unwrap_or(""),check=%check,"runtime recusou envelope");
    }
    status.into_response()
}

fn json_response(status:StatusCode,value:Value) -> Response {
    (status,[("content-type","application/json")],value.to_string()).into_response()
}

async fn dispatch(registry:&RuntimeRegistry,envelope:&Envelope) -> Result<Value,RuntimeError> {
    let command = &envelope.command;
    let kind = command["kind"].as_str().ok_or_else(||failure("command_kind"))?;
    let fields:&[&str] = match kind {
        "adopt"=>&["kind","descriptor","carry"],
        "submit"=>&["kind","text","steer","pre_transcript"],
        "control"=>&["kind","control","payload"],
        "queue"=>&["kind","action"],
        "detach" | "snapshot" | "drain" | "confirm" | "ensure_projection"=>&["kind"],
        _=>return Err(failure("command_kind")),
    };
    if !command.as_object().is_some_and(|object|object.keys().all(|key|fields.contains(&key.as_str()))) {
        return Err(failure("command_fields"));
    }
    if kind == "adopt" {
        return match descriptor(&command["descriptor"])? {
            Target::Headless(target)=>{
                if target.key!=envelope.key || target.generation!=envelope.generation{return Err(failure("runtime_binding"));}
                registry.adopt(target,command["carry"].clone()).await
            },
            Target::Terminal(target)=>{
                if target.key!=envelope.key || target.generation!=envelope.generation{return Err(failure("runtime_binding"));}
                registry.adopt_terminal(target).await
            }
        };
    }
    if kind == "detach" { return registry.detach(&envelope.key,envelope.generation).await; }
    let handle = registry.entry(&envelope.key,envelope.generation).await?;
    match kind {
        "submit"=> {
            if matches!(&handle,EntryHandle::Terminal {..}) && (command.get("steer").is_some_and(|value|!value.is_boolean())
                || command.get("pre_transcript").is_some_and(|value|!value.is_boolean())) {return Err(failure("terminal_payload"));}
            let kind = if command["steer"] == true && matches!(&handle,EntryHandle::Headless(_)) { OperationKind::Steer } else { OperationKind::Input };
            let reply = handle.command(RuntimeCommand { operation_id:envelope.operation_id.clone(),kind,
                payload:json!({"text":command["text"],"pre_transcript":command["pre_transcript"].as_bool().unwrap_or(false)}) }).await?;
            serde_json::to_value(reply).map_err(|_|failure("reply_json"))
        }
        "control"=> {
            if let EntryHandle::Terminal {handle,..}=&handle {
                let control=command["control"].as_str().ok_or_else(||failure("control_kind"))?;
                let reply=handle.control(envelope.operation_id.clone(),control.into(),command["payload"].clone()).await?;
                return serde_json::to_value(reply).map_err(|_|failure("reply_json"));
            }
            let control:OperationKind = serde_json::from_value(command["control"].clone()).map_err(|_|failure("control_kind"))?;
            let reply = handle.command(RuntimeCommand { operation_id:envelope.operation_id.clone(),kind:control,payload:command["payload"].clone() }).await?;
            serde_json::to_value(reply).map_err(|_|failure("reply_json"))
        }
        "queue"=>handle.queue(envelope.operation_id.clone(),serde_json::from_value::<Action>(command["action"].clone()).map_err(|_|failure("queue_action"))?).await,
        "snapshot"=>handle.snapshot().await,
        "drain"=>handle.drain().await,
        "confirm"=>handle.confirm().await,
        "ensure_projection"=>handle.ensure_projection().await,
        _=>Err(failure("command_kind")),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Descriptor {
    name:String,key:String,provider:String,headless:bool,meta:Value,jsonl:String,
    projection_dir:std::path::PathBuf,state_path:std::path::PathBuf,lock_path:std::path::PathBuf,generation:u64,
}

enum Target {Headless(RuntimeTarget),Terminal(super::terminal::TerminalTarget)}
fn descriptor(value:&Value) -> Result<Target,RuntimeError> {
    let descriptor:Descriptor = serde_json::from_value(value.clone()).map_err(|_|failure("descriptor_shape"))?;
    if descriptor.meta["key"] != descriptor.key || descriptor.key.is_empty() { return Err(failure("descriptor_binding")); }
    if !descriptor.headless {
        if descriptor.provider!="claude" || descriptor.meta.get("cano").is_some(){return Err(failure("descriptor_provider"));}
        let binding:crate::terminal_input::TerminalBinding=serde_json::from_value(descriptor.meta["terminal"].clone()).map_err(|_|failure("terminal_binding"))?;
        if binding.generation!=descriptor.generation || binding.name!=descriptor.name || binding.conversation.is_empty() || binding.mux_argv.is_empty()
            || binding.mux_argv.iter().any(|v|v.contains('\0')) || descriptor.jsonl.is_empty()
            || (!binding.windows && (!binding.pane.starts_with('%') || binding.pane[1..].parse::<u64>().is_err()))
            || (binding.windows && !binding.pane.starts_with(&format!("={}:",binding.name))) {return Err(failure("terminal_binding"));}
        return Ok(Target::Terminal(super::terminal::TerminalTarget {key:descriptor.key,generation:descriptor.generation,name:descriptor.name,
            created:descriptor.meta["created"].as_f64().unwrap_or(binding.created as f64),binding,
            lease_path:descriptor.lock_path,state_path:descriptor.state_path,projection_dir:descriptor.projection_dir,transcript:descriptor.jsonl.into()}));
    }
    if descriptor.meta.get("terminal").is_some(){return Err(failure("descriptor_provider"));}
    let cano = &descriptor.meta["cano"];
    let binding = CanoBinding { pid:cano["pid"].as_u64().and_then(|pid|u32::try_from(pid).ok()).ok_or_else(||failure("cano_pid"))?,
        escuta:cano["escuta"].as_str().ok_or_else(||failure("cano_address"))?.into(),
        token:cano["token"].as_str().ok_or_else(||failure("cano_token"))?.into(),
        versao:cano["versao"].as_u64().and_then(|version|u32::try_from(version).ok()).ok_or_else(||failure("cano_version"))? };
    Ok(Target::Headless(RuntimeTarget { key:descriptor.key,generation:descriptor.generation,name:descriptor.name,provider:descriptor.provider,
        created:descriptor.meta["created"].as_f64().unwrap_or(0.0),metadata:descriptor.meta,binding,
        lease_path:descriptor.lock_path,state_path:descriptor.state_path,projection_dir:descriptor.projection_dir,transcript:descriptor.jsonl.into() }))
}

async fn events(State(state):State<Gateway>) -> Response {
    let receiver = state.registry.subscribe();
    let initial = match state.registry.snapshots().await { Ok(events)=>std::collections::VecDeque::from(events),Err(error)=>{
        tracing::warn!(code=%error.code,reason=%error.message,"runtime sem retrato inicial dos eventos");
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    } };
    let stream = futures_util::stream::unfold((receiver,initial),| (mut receiver,mut initial) | async move {
        let event = if let Some(event) = initial.pop_front() { event } else {
            match receiver.recv().await { Ok(event)=>event,Err(_)=>return None }
        };
        Some((Ok::<_,Infallible>(Event::default().event("runtime").data(serde_json::to_string(&event).unwrap())),(receiver,initial)))
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(10))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_and_headless_sources_do_not_overlap() {
        let value = json!({"name":"session","key":"key","provider":"codex","headless":false,
            "meta":{"key":"key","cano":{"pid":42,"escuta":"tcp:127.0.0.1:1","token":"test","versao":2}},
            "jsonl":"chat.jsonl","projection_dir":"projection","state_path":"state","lock_path":"lock","generation":1});
        assert!(descriptor(&value).is_err());
        let mut headless = value;
        headless["headless"] = json!(true);
        assert!(descriptor(&headless).is_ok());
    }

    #[tokio::test]
    async fn one_sse_reader_multiple_keys() {
        let registry = RuntimeRegistry::new("127.0.0.1:9".parse().unwrap(),"test".into(),"instance".into());
        let mut receiver = registry.subscribe();
        assert_eq!(registry.events.receiver_count(),1);
        for key in ["claude-key","codex-key"] {
            assert!(registry.events.send(RuntimeEvent { key:key.into(),generation:1,revision:1,channel:"state".into(),data:json!({}) }).is_ok());
        }
        assert_eq!(receiver.recv().await.unwrap().key,"claude-key");
        assert_eq!(receiver.recv().await.unwrap().key,"codex-key");
    }
}
