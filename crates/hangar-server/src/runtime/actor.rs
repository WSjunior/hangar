use super::{cano::{CanoConnection,IoEvent,WireFrame},claude::ClaudeEngine,codex::Engine as CodexEngine,
    protocol::*,queue::{Action,QueueActor,Status},receipt::ReceiptIndex};
use crate::mods::model::{ModsCall,ModsError,SurfaceEffect};
use serde_json::{Value,json};
use std::collections::{BTreeMap,BTreeSet,VecDeque};
use std::sync::{Arc,atomic::{AtomicBool,AtomicU64,Ordering}};
use std::time::{Duration,Instant,SystemTime,UNIX_EPOCH};
use tokio::sync::{broadcast,mpsc,oneshot,Mutex,Notify};
use tokio::task::{JoinHandle,JoinSet};

enum Core { Claude(ClaudeEngine),Codex(CodexEngine) }

/// Teto da política no Python. A publicação no terminal espera o aviso do plugin, que só sai depois
/// dos hooks do UserPromptSubmit: o teto dela fica acima da espera do Python (`PUBLICA_S`).
const POLICY_TIMEOUT:Duration=Duration::from_secs(15);
const PUBLISH_POLICY_TIMEOUT:Duration=Duration::from_secs(40);
/// Teto de um pedido de app aos mods: o mais longo da superfície (desenho de novo e clique, 6 s) mais
/// 1 s de folga para o relógio do ator, abaixo dos 8 s em que o app desiste. O registro serializa os
/// pedidos da sessão, então um pedido sem teto prenderia os seguintes.
const MODS_CALL_LIMIT:Duration=Duration::from_secs(crate::mods::surface::APP_CALL_MAX_S as u64 + 1);

#[derive(Clone)]
pub struct PolicyClient {
    upstream:std::net::SocketAddr,
    secret:String,
    instance:String,
    http:crate::proxy::HttpClient,
}

impl PolicyClient {
    pub fn new(upstream:std::net::SocketAddr,secret:String,instance:String) -> Self {
        Self { upstream,secret,instance,http:crate::proxy::client() }
    }
    async fn run(&self,target:&RuntimeTarget,kind:&str,request_id:&RequestId,payload:Value,phase_id:&str) -> Result<Value,RuntimeError> {
        self.run_for(&target.key,target.generation,kind,request_id,payload,phase_id).await
    }
    pub async fn run_for(&self,key:&str,generation:u64,kind:&str,request_id:&RequestId,payload:Value,phase_id:&str) -> Result<Value,RuntimeError> {
        let body = json!({"key":key,"generation":generation,"request_id":request_id,"phase_id":phase_id,"kind":kind,"payload":payload});
        let request = axum::http::Request::post(format!("http://{}/internal/runtime/policy",self.upstream))
            .header("x-hangar-internal",&self.secret).header("x-hangar-runtime-instance",&self.instance)
            .header("content-type","application/json").body(axum::body::Body::from(body.to_string()))
            .map_err(|_|failure("policy_request"))?;
        // Detalhe só de forma: status, tipo de erro, posição ou nome da exceção; nunca o corpo.
        let limit = if kind == "terminal_publish" { PUBLISH_POLICY_TIMEOUT } else { POLICY_TIMEOUT };
        let result = tokio::time::timeout(limit,async {
            let response = self.http.request(request).await
                .map_err(|error|(failure("policy_transport"),format!("connect={}",error.is_connect())))?;
            if !response.status().is_success() { return Err((failure("policy_refused"),format!("status={}",response.status().as_u16()))); }
            let bytes = axum::body::to_bytes(axum::body::Body::new(response.into_body()),MAX_ENVELOPE)
                .await.map_err(|_|(failure("policy_limit"),String::new()))?;
            let value:Value = serde_json::from_slice(&bytes)
                .map_err(|error|(failure("policy_json"),format!("line={} column={}",error.line(),error.column())))?;
            if value["ok"] != true {
                let kind = value["error_type"].as_str().filter(|kind|kind.len() <= 64 && kind.bytes().all(|b|b.is_ascii_alphanumeric() || b == b'_'));
                return Err((failure("policy_failed"),format!("error_type={}",kind.unwrap_or("?"))));
            }
            Ok(value["data"].clone())
        }).await.unwrap_or_else(|_|Err((failure("policy_timeout"),String::new())));
        result.map_err(|(error,detail)|{
            if crate::warn_limit::allow(Some(key),&error.code) {
                tracing::warn!(key=%key,generation,policy=%kind,code=%error.code,detail=%detail,"política do Python falhou");
            }
            error
        })
    }
}

pub struct RuntimeEngine {
    core:Core,
    policy:Option<PolicyClient>,
    publisher:Option<broadcast::Sender<RuntimeEvent>>,
    revision:Arc<AtomicU64>,
    mods:Option<crate::mods::state::Mods>,
}

impl RuntimeEngine {
    pub fn new(provider:&str,metadata:Value,generation:u64,clock:ClockSample) -> Result<Self,RuntimeError> {
        let core = match provider { "claude"=>Core::Claude(ClaudeEngine::new(metadata,generation,clock)),
            "codex"=>Core::Codex(CodexEngine::new(metadata,generation,clock)),_=>return Err(failure("provider")) };
        Ok(Self { core,policy:None,publisher:None,revision:Arc::new(AtomicU64::new(0)),mods:None })
    }
    pub fn with_policy(mut self,policy:PolicyClient) -> Self { self.policy = Some(policy); self }
    pub fn with_publisher(mut self,publisher:broadcast::Sender<RuntimeEvent>) -> Self { self.publisher = Some(publisher); self }
    pub fn with_revision(mut self,revision:Arc<AtomicU64>) -> Self { self.revision = revision; self }
    /// Liga a interface dos mods: o Claude sem terminal vira superfície `desktop` e publica no `Mods`.
    /// O prefixo dos pedidos é único por ator, para a resposta de uma vida anterior não casar.
    pub fn with_mods(mut self,mods:crate::mods::state::Mods) -> Self {
        static ACTORS:AtomicU64 = AtomicU64::new(0);
        if let Core::Claude(core) = &mut self.core {
            core.enable_surface(format!("ui:{}.{}",std::process::id(),ACTORS.fetch_add(1,Ordering::Relaxed)));
            self.mods = Some(mods);
        }
        self
    }
    fn mods_call(&mut self,token:u64,call:ModsCall,clock:ClockSample) -> Result<Vec<Effect>,ModsError> {
        match &mut self.core { Core::Claude(core)=>core.mods_call(token,call,clock),Core::Codex(_)=>Err(crate::mods::model::missing()) }
    }
    fn view(&self) -> Value {
        match &self.core { Core::Claude(core)=>core.view(),Core::Codex(core)=> {
            let mut view = core.control_view(); view["public_state"] = core.view(); view["conversation"] = view["thread_id"].clone(); view
        } }
    }
    fn apply(&mut self,input:EngineInput,clock:ClockSample) -> Result<Vec<Effect>,RuntimeError> {
        match &mut self.core { Core::Claude(core)=>core.apply(input,clock),Core::Codex(core)=>core.apply(input,clock) }
    }
    fn command(&mut self,command:RuntimeCommand,clock:ClockSample) -> Result<Vec<Effect>,RuntimeError> {
        match &mut self.core { Core::Claude(core)=>core.command(command,clock),Core::Codex(core)=>core.command(command,clock) }
    }
    fn hydrate(&mut self,snapshot:CanoSnapshot) -> Result<Vec<Effect>,RuntimeError> {
        match &mut self.core { Core::Claude(core)=>core.hydrate(snapshot),Core::Codex(core)=>core.hydrate(snapshot) }
    }
    fn deadline(&self) -> Option<f64> { match &self.core { Core::Claude(core)=>core.next_deadline(),Core::Codex(core)=>core.next_deadline() } }
    fn initialize(&mut self,id:String) -> Result<Vec<Effect>,RuntimeError> {
        match &mut self.core { Core::Claude(core)=>core.start_initialize(id),Core::Codex(core)=>core.bootstrap(true,id) }
    }
    fn write_is_current(&self,id:&str) -> bool { match &self.core { Core::Claude(core)=>core.write_is_current(id),Core::Codex(core)=>core.write_is_current(id) } }
    fn forget_policy(&mut self,id:&RequestId) { match &mut self.core { Core::Claude(core)=>core.forget_policy(id),Core::Codex(core)=>core.forget_policy(id) } }
    fn confirm_input(&mut self,id:&str) -> Vec<Effect> {
        match &mut self.core { Core::Claude(_)=>vec![Effect::Reply { operation_id:id.into(),disposition:Disposition::Accepted,payload:json!({"confirmed":true}) }],
            Core::Codex(core)=>core.confirm_input(id) }
    }
    fn restore(&mut self,state:&super::queue::State) {
        for phase in state.operations.values().filter(|phase|recover_phase(state,phase)) {
            if let Some(id) = phase.payload["logical_id"].as_str() {
                match &mut self.core {
                    Core::Codex(core)=>{
                        core.restore_rpc(id.into(),&phase.payload["frame"],phase.payload["state_revision"].as_u64().unwrap_or(0),
                            phase.payload["settings_revision"].as_u64().unwrap_or(0));
                        if let Some(call_id) = state.operations.get(id).and_then(|root|root.payload["payload"]["call_id"].as_str()) {
                            core.restore_voice_scope(id,call_id);
                        }
                    },
                    Core::Claude(core)=>{
                        let mut frame = phase.payload["frame"].clone();
                        if frame["request"]["subtype"] == "set_model" {
                            if let Some(effort) = state.operations.get(id).and_then(|root|root.payload["payload"].get("effort")).filter(|effort|effort.is_string()) {
                                frame["request"]["effort"] = effort.clone();
                            }
                        }
                        core.restore_control(id.into(),&frame);
                    },
                }
            }
        }
    }
}

fn failure(code:&str) -> RuntimeError { RuntimeError::new(code,"runtime indisponível; operação conservada no diário") }
fn io_failure(error:std::io::Error) -> RuntimeError {
    // Recusa da fila tem frase fixa e vai inteira (log e diário do Python); de resto só o tipo, porque
    // a mensagem do io::Error ou do serde pode trazer caminho ou texto.
    let reason = super::queue::refusal(&error).map_or_else(||format!("{:?}",error.kind()),str::to_owned);
    if crate::warn_limit::allow(None,&format!("queue_io:{reason}")) { tracing::warn!(reason=%reason,"runtime falhou em E/S (diário ou transcript)"); }
    RuntimeError::new("queue_io",&format!("fila recusou: {reason}"))
}

/// Loga só na entrada em erro ou na troca de código: o mesmo erro repetido não enche o log.
fn enter_error(error:&mut Option<RuntimeError>,target:&RuntimeTarget,failure:RuntimeError) {
    if error.as_ref().is_none_or(|current|current.code != failure.code) {
        tracing::warn!(key=%target.key,session=%target.name,code=%failure.code,reason=%failure.message,"runtime entrou em erro");
    }
    *error = Some(failure);
}
fn clock(start:Instant) -> ClockSample {
    let epoch_s = match SystemTime::now().duration_since(UNIX_EPOCH) { Ok(time)=>time.as_secs_f64(),Err(error)=>-error.duration().as_secs_f64() };
    ClockSample { monotonic_s:start.elapsed().as_secs_f64(),epoch_s }
}

type Response = oneshot::Sender<Result<RuntimeReply,RuntimeError>>;
enum Message {
    Command { command:RuntimeCommand,response:Response,from_queue:bool },
    Queue { call_id:String,action:Action,response:oneshot::Sender<Result<Value,RuntimeError>> },
    Snapshot(oneshot::Sender<Result<Value,RuntimeError>>),
    Drain(oneshot::Sender<Result<Value,RuntimeError>>),
    Confirm(oneshot::Sender<Result<Value,RuntimeError>>),
    /// `deadline`: quando o app deixa de esperar. Pedido que chega à vez depois disso não roda.
    Mods { call:ModsCall,deadline:Instant,response:oneshot::Sender<Result<Value,ModsError>> },
    Stop(oneshot::Sender<Result<(),RuntimeError>>),
}

#[derive(Clone)]
pub struct RuntimeHandle {
    sender:mpsc::Sender<Message>,
    task:Arc<Mutex<Option<JoinHandle<Result<(),RuntimeError>>>>>,
    closed:Arc<AtomicBool>,
    events:broadcast::Sender<RuntimeEvent>,
    stopped:Arc<Mutex<Option<Result<(),RuntimeError>>>>,
    key:String,
}

impl RuntimeHandle {
    pub async fn command(&self,command:RuntimeCommand) -> Result<RuntimeReply,RuntimeError> {
        if self.closed.load(Ordering::Acquire) { return Err(failure("runtime_stopping")); }
        let (response,receive) = oneshot::channel();
        self.sender.send(Message::Command { command,response,from_queue:false }).await.map_err(|_|self.gone("runtime_closed"))?;
        receive.await.map_err(|_|self.gone("runtime_closed"))?
    }
    pub async fn queue(&self,call_id:String,action:Action) -> Result<Value,RuntimeError> {
        let (response,receive) = oneshot::channel();
        self.sender.send(Message::Queue { call_id,action,response }).await.map_err(|_|self.gone("runtime_closed"))?;
        receive.await.map_err(|_|self.gone("runtime_closed"))?
    }
    pub async fn snapshot(&self) -> Result<Value,RuntimeError> {
        let (send,receive) = oneshot::channel();
        self.sender.send(Message::Snapshot(send)).await.map_err(|_|self.gone("runtime_closed"))?;
        receive.await.map_err(|_|self.gone("runtime_closed"))?
    }
    pub async fn drain(&self) -> Result<Value,RuntimeError> {
        let (send,receive) = oneshot::channel(); self.sender.send(Message::Drain(send)).await.map_err(|_|self.gone("runtime_closed"))?;
        receive.await.map_err(|_|self.gone("runtime_closed"))?
    }
    pub async fn confirm(&self) -> Result<Value,RuntimeError> {
        let (send,receive) = oneshot::channel(); self.sender.send(Message::Confirm(send)).await.map_err(|_|self.gone("runtime_closed"))?;
        receive.await.map_err(|_|self.gone("runtime_closed"))?
    }
    /// Pedido de um app à interface dos mods desta sessão. Ator parado ou sumido responde com código,
    /// nunca pendura o app.
    pub async fn mods(&self,call:ModsCall) -> Result<Value,ModsError> { self.mods_within(call,MODS_CALL_LIMIT).await }
    async fn mods_within(&self,call:ModsCall,limit:Duration) -> Result<Value,ModsError> {
        if self.closed.load(Ordering::Acquire) { return Err(crate::mods::model::no_answer()); }
        let (response,receive) = oneshot::channel();
        let deadline = Instant::now() + limit;
        tokio::time::timeout(limit,async {
            self.sender.send(Message::Mods { call,deadline,response }).await.map_err(|_|crate::mods::model::no_answer())?;
            receive.await.map_err(|_|crate::mods::model::no_answer())?
        }).await.unwrap_or_else(|_|Err(crate::mods::model::no_answer()))
    }
    pub async fn ensure_projection(&self) -> Result<Value,RuntimeError> {
        self.queue(format!("projection:{}",unique()),Action::EnsureProjection).await
    }
    /// O motivo real já saiu na linha de saída do ator; aqui fica qual chave o perdeu.
    fn gone(&self,code:&str) -> RuntimeError {
        if crate::warn_limit::allow(Some(&self.key),code) { tracing::warn!(key=%self.key,code,"runtime sem ator"); }
        failure(code)
    }
    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> { self.events.subscribe() }
    pub async fn stop(&self) -> Result<(),RuntimeError> {
        let mut stopped = self.stopped.lock().await;
        if let Some(result) = &*stopped { return result.clone(); }
        self.closed.store(true,Ordering::Release);
        let (send,receive) = oneshot::channel();
        let mut result = match self.sender.send(Message::Stop(send)).await {
            Ok(())=>receive.await.map_err(|_|self.gone("runtime_closed")).and_then(|result|result),
            Err(_)=>Err(self.gone("runtime_closed")),
        };
        if let Some(task) = self.task.lock().await.take() {
            let joined = task.await.map_err(|_|self.gone("runtime_panic")).and_then(|result|result);
            if joined.is_err() { result = joined; }
        }
        *stopped = Some(result.clone());
        result
    }
}

impl crate::mods::state::SurfaceLink for RuntimeHandle {
    fn call(&self,call:ModsCall) -> crate::mods::state::CallFuture {
        let handle = self.clone();
        Box::pin(async move { handle.mods(call).await })
    }
}

struct Pending {
    command:RuntimeCommand,
    original:Value,
    responses:Vec<Response>,
    result:Option<RuntimeReply>,
    preparing:bool,
    cancelled:bool,
    error:Option<RuntimeError>,
    deadline:f64,
    timed_out:bool,
    ready_to_run:bool,
    arrival:u64,
}

impl Pending {
    fn stored(command:RuntimeCommand,reply:RuntimeReply) -> Self {
        Self { original:serde_json::to_value(&command).unwrap(),command,responses:Vec::new(),result:Some(reply),preparing:false,
            cancelled:false,error:None,deadline:0.0,timed_out:false,ready_to_run:false,arrival:0 }
    }
}

#[derive(Clone)]
struct Attempt { logical_id:String,phase_id:String,frame:Value,order:u64 }

enum Job {
    Root { id:String,result:Result<Option<RuntimeReply>,RuntimeError> },
    Write { wire:String,result:Result<(),RuntimeError> },
    Ack { logical_id:String,outcome:WriteOutcome,result:Result<(),RuntimeError> },
    Finished { reply:RuntimeReply,result:Result<(),RuntimeError> },
    Policy { request_id:RequestId,kind:String,phase_id:String,result:Result<Value,RuntimeError> },
    PreparedInput { id:String,result:Result<Value,RuntimeError> },
    Saved(Result<(),RuntimeError>),
    Queued { wake:bool,result:Result<(),RuntimeError> },
    View { version:u64,view:Value,result:Result<(),RuntimeError> },
    Drained(Result<Vec<RuntimeCommand>,RuntimeError>),
    DrainFinished(Result<RuntimeReply,RuntimeError>),
    Confirmed { response:Option<oneshot::Sender<Result<Value,RuntimeError>>>,result:Result<(Vec<String>,usize),RuntimeError> },
    Steered { id:String,result:Result<Vec<String>,RuntimeError> },
    NativeInput { id:String,result:Result<Value,RuntimeError> },
}

fn unique() -> String {
    static COUNTER:std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!("{}:{}",std::process::id(),COUNTER.fetch_add(1,Ordering::Relaxed))
}

pub struct RuntimeActor;

impl RuntimeActor {
    pub fn spawn(target:RuntimeTarget,queue:QueueActor,connection:CanoConnection,engine:RuntimeEngine) -> RuntimeHandle {
        let (sender,receiver) = mpsc::channel(64);
        let events = engine.publisher.clone().unwrap_or_else(||broadcast::channel(256).0);
        let closed = Arc::new(AtomicBool::new(false));
        let (key,name) = (target.key.clone(),target.name.clone());
        let run = run(target,queue,connection,engine,receiver,sender.clone(),closed.clone(),events.clone());
        let log_key = key.clone();
        let task = tokio::spawn(async move {
            let result = run.await;
            // Saída por `?` deixava o ator mudo: só sobrava o runtime_closed de quem chamasse depois.
            if let Err(error) = &result { tracing::warn!(key=%log_key,session=%name,code=%error.code,reason=%error.message,"ator do runtime terminou com erro"); }
            result
        });
        RuntimeHandle { sender,task:Arc::new(Mutex::new(Some(task))),closed,events,stopped:Arc::new(Mutex::new(None)),key }
    }
}

async fn run(target:RuntimeTarget,queue:QueueActor,connection:CanoConnection,mut engine:RuntimeEngine,
    mut receiver:mpsc::Receiver<Message>,internal:mpsc::Sender<Message>,closed:Arc<AtomicBool>,events:broadcast::Sender<RuntimeEvent>) -> Result<(),RuntimeError> {
    let start = Instant::now();
    let initial = queue.initial_state().clone();
    engine.restore(&initial);
    let snapshot = connection.snapshot.clone();
    let mut io = connection.start(target.generation,128).hold_lease(queue.lease());
    let queue = Arc::new(queue);
    let mut jobs = JoinSet::new();
    let mut roots:BTreeMap<String,Pending> = BTreeMap::new();
    let preparation = Arc::new((Mutex::new(1u64),Notify::new()));
    let mut arrival = 0u64;
    let mut attempts:BTreeMap<String,Attempt> = initial.operations.values().filter(|phase|recover_phase(&initial,phase))
        .filter_map(|phase|Some((phase.id.clone(),Attempt { logical_id:phase.payload["logical_id"].as_str()?.into(),
            phase_id:phase.id.clone(),frame:phase.payload["frame"].clone(),order:0 }))).collect();
    let mut effects:VecDeque<Effect> = VecDeque::new();
    let mut revision = Revision { value:engine.revision.load(Ordering::Acquire),counter:engine.revision.clone() };
    let mut state_version = 0u64;
    let mut published_state_version = 0u64;
    let state_gate = Arc::new(Mutex::new(SavedView::default()));
    let mut durable_view = json!({"alive":true,"initialized":false,"ready":false});
    let mut last_state = String::new();
    let mut confirming = false;
    let mut channels:BTreeMap<String,Value> = ["preview","thinking","tool"].into_iter().map(|channel|
        (channel.into(),json!({"session":target.name,"text":"","md":true,"full":true,"vivo":true}))).collect();
    let receipt = Arc::new(std::sync::Mutex::new(ReceiptIndex::new(&target.provider,engine.view()["conversation"].as_str().unwrap_or(""))));
    let mut sequence = initial.operations.keys().filter_map(|id|id.rsplit(':').next()?.parse::<u64>().ok()).max().unwrap_or(0);
    let mut write_order = 0u64;
    let mut next_write = 1u64;
    let mut native:BTreeSet<String> = BTreeSet::new();
    let mut prepared_writes:BTreeMap<u64,(String,Result<(),RuntimeError>)> = BTreeMap::new();
    let mut error:Option<RuntimeError> = None;
    let mut io_open = true;
    let mut drain_requested = true;
    let mut drain_active = false;
    let mut drain_waiters = Vec::new();
    let mut mods_waiters:ModsWaiters = BTreeMap::new();
    let mut mods_token = 0u64;
    let mut ui_writes = 0u64;
    effects.extend(engine.hydrate(snapshot)?);
    if engine.view()["initialized"] != true || target.provider == "codex" && engine.view()["ready"] != true {
        let id = format!("bootstrap:{}:{}",target.key,target.generation);
        queue.exec(target.generation,&format!("prepare:{id}"),clock(start),Action::Prepare { id:id.clone(),payload:json!({"kind":"bootstrap"}),entry_id:None }).await.map_err(io_failure)?;
        effects.extend(engine.initialize(id)?);
    }
    loop {
        loop {
            let first_input = roots.values().filter(|root|root.preparing && matches!(root.command.kind,OperationKind::Input | OperationKind::Steer))
                .map(|root|root.arrival).min();
            let ready = roots.iter().filter(|(_,root)|root.ready_to_run && root.preparing
                && (!matches!(root.command.kind,OperationKind::Input | OperationKind::Steer) || Some(root.arrival) == first_input))
                .min_by_key(|(_,root)|root.arrival).map(|(id,_)|id.clone());
            let Some(id) = ready else { break };
            let pending = roots.get_mut(&id).unwrap();
            pending.preparing = false;
            if pending.cancelled || pending.timed_out { continue; }
            if pending.command.kind == OperationKind::SteerQueue {
                // Mesmas recusas do adapter Python: sem turno não há o que orientar, e o Claude parado numa
                // permissão ou pergunta não lê o stdin; a fila sumiria da tela até alguém responder.
                let view = engine.view();
                let refusal = if view["alive"] != true || view["in_progress"] != true { Some("Não há turno em andamento para orientar") }
                    else if target.provider == "claude" && (view["pending"].as_array().is_some_and(|p|!p.is_empty()) || !view["question"].is_null()) {
                        Some("Responda a permissão ou pergunta pendente antes de orientar") }
                    else { None };
                if let Some(text) = refusal {
                    effects.push_back(Effect::Reply { operation_id:id,disposition:Disposition::Rejected,payload:json!({"error":text}) });
                    continue;
                }
                let queue = queue.clone(); let target = target.clone(); let sender = internal.clone(); let sample = clock(start);
                let entry_id = pending.command.payload["entry_id"].as_str().map(str::to_owned);
                jobs.spawn(async move {
                    let result = async {
                        let claimed = queue.exec(target.generation,&format!("steer-claim:{id}"),sample,
                            Action::Claim { min_ts:target.created,limit:None,entry_id }).await.map_err(io_failure)?;
                        let mut replies = Vec::new();
                        for row in claimed.as_array().ok_or_else(||failure("queue_shape"))? {
                            let entry = row["id"].as_str().ok_or_else(||failure("queue_entry"))?.to_owned();
                            let command = RuntimeCommand { operation_id:format!("{id}:{entry}"),kind:OperationKind::Steer,
                                payload:json!({"text":row["text"],"entry_id":entry,"pre_transcript":row["pre_transcript"].as_bool().unwrap_or(false)}) };
                            let (response,receive) = oneshot::channel();
                            sender.send(Message::Command { command,response,from_queue:false }).await.map_err(|_|failure("runtime_closed"))?;
                            replies.push((entry,receive));
                        }
                        let mut accepted = Vec::new();
                        let mut uncertain = false;
                        for (entry,receive) in replies {
                            let result = receive.await.map_err(|_|failure("runtime_closed"))?;
                            match result {
                                Ok(reply) if reply.disposition == Disposition::Accepted => {
                                    queue.exec(target.generation,&format!("steered:{id}:{entry}"),sample,
                                        Action::SetDelivered { entry_id:entry.clone(),value:true,steered:true }).await.map_err(io_failure)?;
                                    accepted.push(entry);
                                }
                                Ok(reply) if reply.disposition == Disposition::Unknown => { uncertain = true; },
                                _=>{
                                    if queue.exec(target.generation,&format!("steer-unclaim:{id}:{entry}"),sample,
                                        Action::SetDelivered { entry_id:entry,value:false,steered:false }).await.is_err() { uncertain = true; }
                                }
                            }
                        }
                        if uncertain { Err(failure("steer_unknown")) } else { Ok(accepted) }
                    }.await;
                    Job::Steered { id,result }
                });
                continue;
            }
            match engine.command(pending.command.clone(),clock(start)) {
                Ok(next)=>effects.extend(next),Err(error)=>defer_unwritten(&mut roots,&attempts,&native,&id,error,&mut effects),
            }
        }
        while let Some(effect) = effects.pop_front() {
            match effect {
                Effect::Write { frame,operation_id } => {
                    sequence += 1;
                    write_order += 1;
                    let logical_id = operation_id.unwrap_or_else(||format!("system:{}:{sequence}",target.generation));
                    let wire = format!("wire:{logical_id}:{sequence}");
                    let attempt = Attempt { logical_id:logical_id.clone(),phase_id:wire.clone(),frame:frame.clone(),order:write_order };
                    attempts.insert(wire.clone(),attempt.clone());
                    let queue = queue.clone(); let target = target.clone(); let view = engine.view(); let sample = clock(start);
                    let state_gate = state_gate.clone(); let state_version = state_version;
                    jobs.spawn(async move {
                        let result = async {
                            let authoritative = queue.snapshot().await.map_err(io_failure)?;
                            let entry_id = authoritative.operations.get(&logical_id).and_then(|op|op.entry_id.clone());
                            if !authoritative.operations.contains_key(&logical_id) {
                                queue.exec(target.generation,&format!("prepare-logical:{wire}"),sample,Action::Prepare { id:logical_id.clone(),
                                    payload:json!({"kind":"phase","frame":frame}),entry_id:None }).await.map_err(io_failure)?;
                            }
                            queue.exec(target.generation,&format!("prepare:{wire}"),sample,Action::Prepare {
                                id:wire.clone(),payload:json!({"logical_id":logical_id,"frame":frame,"request_id":frame.get("id").or_else(||frame.get("request_id")),
                                    "generation":target.generation,"conversation":view["conversation"],
                                    "state_revision":view["state_revision"],"settings_revision":view["settings_revision"]}),entry_id }).await.map_err(io_failure)?;
                            // Antes de cada escrita no fio a vista vai inteira: o contador dos IDs tem que estar salvo.
                            save_view(&queue,target.generation,sample,&state_gate,state_version,&view,true).await?;
                            let cursor = capture_cursor(&target,&view).await?;
                            queue.exec(target.generation,&format!("cursor:{wire}"),sample,Action::BindDispatch { id:wire.clone(),cursor:cursor.clone() }).await.map_err(io_failure)?;
                            if authoritative.operations.get(&logical_id).is_none_or(|op|op.status == Status::Prepared) {
                                queue.exec(target.generation,&format!("logical-cursor:{wire}"),sample,Action::BindDispatch { id:logical_id.clone(),cursor }).await.map_err(io_failure)?;
                            }
                            queue.exec(target.generation,&format!("dispatch:{wire}"),sample,Action::BeginDispatch { id:wire.clone(),wire_id:wire.clone(),staged:false }).await.map_err(io_failure)?;
                            queue.exec(target.generation,&format!("logical-dispatch:{wire}"),sample,Action::BeginDispatch { id:logical_id.clone(),wire_id:wire.clone(),staged:false }).await.map_err(io_failure)?;
                            Ok(())
                        }.await;
                        Job::Write { wire,result }
                    });
                }
                Effect::Publish { channel,data } => {
                    if ["preview","thinking","tool"].contains(&channel.as_str()) {
                        channels.insert(channel.clone(),data.clone()); publish(&events,&target,&mut revision,&channel,data);
                    } else if ["voice","voice_target","rate"].contains(&channel.as_str()) {
                        publish(&events,&target,&mut revision,&channel,data);
                    }
                }
                Effect::StateChanged => {
                    let view = engine.view();
                    state_version += 1;
                    let version = state_version;
                    let queue = queue.clone(); let generation = target.generation; let sample = clock(start);
                    let gate = state_gate.clone();
                    jobs.spawn(async move {
                        let result = save_view(&queue,generation,sample,&gate,version,&view,false).await;
                        Job::View { version,view,result }
                    });
                }
                Effect::Reply { operation_id,disposition,payload } => {
                    let reply = RuntimeReply { operation_id:operation_id.clone(),disposition,payload };
                    let queue = queue.clone(); let generation = target.generation; let sample = clock(start);
                    let related:Vec<_> = attempts.values().filter(|attempt|attempt.logical_id == operation_id).cloned().collect();
                    jobs.spawn(async move {
                        let result = async {
                            for attempt in related {
                                queue.exec(generation,&format!("finish:{}:{}",attempt.phase_id,unique()),sample,Action::Finish {
                                    id:attempt.phase_id,status:status(disposition),result:serde_json::to_value(&reply).unwrap() }).await.map_err(io_failure)?;
                            }
                            queue.exec(generation,&format!("finish:{operation_id}:{}",unique()),sample,Action::Finish {
                                id:operation_id,status:status(disposition),result:serde_json::to_value(&reply).unwrap() }).await.map_err(io_failure)?;
                            Ok(())
                        }.await;
                        Job::Finished { reply,result }
                    });
                }
                Effect::Policy { kind,request_id,payload } => {
                    if kind == "local_output" {
                        let queue = queue.clone(); let generation = target.generation; let sample = clock(start);
                        let text = payload["text"].as_str().unwrap_or("").to_owned();
                        jobs.spawn(async move { Job::Saved(queue.exec(generation,&format!("local:{}",unique()),sample,
                            Action::AppendLocal { text,entry_id:None }).await.map(|_|()).map_err(io_failure)) });
                        continue;
                    }
                    sequence += 1;
                    let phase_id = format!("policy:{}:{sequence}",target.generation);
                    let target = target.clone(); let policy = engine.policy.clone();
                    let save = if kind == "session.patch_meta" && payload.get("service_tier").is_some() {
                        state_version += 1;
                        Some((state_version,engine.view()))
                    } else { None };
                    let queue = queue.clone(); let gate = state_gate.clone(); let sample = clock(start);
                    // Estes serviços não escrevem na CLI (formatar status, carimbo, sidecar, log): repetir é
                    // inofensivo, então não passam pelo diário. Quatro gravações por chamada, a cada mudança de
                    // estado, eram a maior parte do disco gasto por mensagem.
                    jobs.spawn(async move {
                        let result = async {
                            if let Some((version,view)) = save {
                                save_view(&queue,target.generation,sample,&gate,version,&view,false).await?;
                            }
                            match policy {
                                Some(policy) => policy.run(&target,&kind,&request_id,payload,&phase_id).await,
                                None => Err(failure("policy_unavailable")),
                            }
                        }.await;
                        Job::Policy { request_id,kind,phase_id,result }
                    });
                }
                Effect::WakeQueue => { drain_requested = true; },
                Effect::ConfirmLocalCommands => {
                    // Mesma regra do adapter Python: comando local não aparece no transcript, então o
                    // reconcile nunca o confirmaria; a CLI já o consumiu. Só as entradas de barra.
                    let queue = queue.clone(); let generation = target.generation; let sample = clock(start);
                    jobs.spawn(async move { Job::Saved(async {
                        let state = queue.snapshot().await.map_err(io_failure)?;
                        // Só a que já foi ao fio: a próxima barra, reivindicada pelo drain ao mesmo tempo e ainda
                        // não escrita, confirmada aqui nunca seria enviada.
                        let entry_ids:Vec<String> = state.rows.iter().filter(|row|row["delivered"] == true && row["confirmed"] != true
                            && row["text"].as_str().is_some_and(|text|text.trim_start().starts_with('/'))
                            && row["id"].as_str().and_then(|id|state.operations.get(id)).is_some_and(|op|matches!(op.status,Status::Dispatching | Status::Accepted)))
                            .filter_map(|row|row["id"].as_str().map(str::to_owned)).collect();
                        if entry_ids.is_empty() { return Ok(()); }
                        queue.exec(generation,&format!("local-confirm:{}",unique()),sample,Action::Confirm { entry_ids }).await.map(|_|()).map_err(io_failure)
                    }.await) });
                },
                Effect::Surface { effect } => match effect {
                    SurfaceEffect::Write { frame } => {
                        // `ui_*` não muda a conversa nem precisa sobreviver a uma queda: sai direto, fora do
                        // diário, que gravaria no disco a cada desenho.
                        ui_writes += 1;
                        // Canal próprio e pequeno: o que não cabe é descartado e a superfície pede de novo no prazo.
                        let frame = WireFrame { operation_id:format!("ui:{}:{ui_writes}",target.generation),frame,ephemeral:true };
                        if io.try_send(frame).is_err() && crate::warn_limit::allow(Some(&target.key),"ui_write") {
                            tracing::warn!(key=%target.key,session=%target.name,"pedido da interface dos mods descartado com o canal do cano cheio");
                        }
                    }
                    SurfaceEffect::Publish { data } => { if let Some(mods) = &engine.mods { mods.publish_ui(&target.name,target.generation,&data); } }
                    SurfaceEffect::Toast { plugin,text,timeout_ms } => { if let Some(mods) = &engine.mods { mods.toast(&target.name,target.generation,&plugin,&text,timeout_ms); } }
                    SurfaceEffect::Copied { plugin,text } => { if let Some(mods) = &engine.mods { mods.copied(&target.name,target.generation,&plugin,&text); } }
                    SurfaceEffect::Reply { token,result } => { if let Some(waiter) = mods_waiters.remove(&token) { let _ = waiter.send(result); } }
                },
                Effect::Stop { .. } => { closed.store(true,Ordering::Release); },
            }
        }
        while let Some((wire,result)) = prepared_writes.remove(&next_write) {
            next_write += 1;
            let attempt = attempts.get(&wire).unwrap();
            let ended = roots.iter().any(|(id,root)|(attempt.logical_id == *id || attempt.logical_id.starts_with(&format!("{id}:")))
                && (root.cancelled || root.timed_out));
            if result.is_err() || ended || !engine.write_is_current(&attempt.logical_id) {
                effects.extend(engine.apply(EngineInput::WriteAck { operation_id:attempt.logical_id.clone(),outcome:WriteOutcome::NotWritten },clock(start))?);
                if let Err(failure) = result { enter_error(&mut error,&target,failure); }
            } else if io.try_send(WireFrame { operation_id:wire,frame:attempt.frame.clone(),ephemeral:false }).is_err() {
                effects.extend(engine.apply(EngineInput::WriteAck { operation_id:attempt.logical_id.clone(),outcome:WriteOutcome::NotWritten },clock(start))?);
            }
        }
        if drain_requested && !drain_active && !closed.load(Ordering::Acquire) && engine.view()["deliverable"] == true
            && !roots.values().any(|root|root.preparing && matches!(root.command.kind,OperationKind::Input | OperationKind::Steer)) {
            drain_requested = false; drain_active = true;
            let queue = queue.clone(); let target = target.clone(); let sample = clock(start);
            jobs.spawn(async move {
                let result = async {
                    let claimed = queue.exec(target.generation,&format!("claim:{}",unique()),sample,
                        Action::Claim { min_ts:target.created,limit:Some(1),entry_id:None }).await.map_err(io_failure)?;
                    let state = queue.snapshot().await.map_err(io_failure)?;
                    let mut commands = Vec::new();
                    for row in claimed.as_array().ok_or_else(||failure("queue_shape"))? {
                        let id = row["id"].as_str().ok_or_else(||failure("queue_entry"))?;
                        let command = state.operations.get(id).and_then(|op|serde_json::from_value::<RuntimeCommand>(op.payload.clone()).ok())
                            .unwrap_or_else(||RuntimeCommand { operation_id:id.into(),kind:OperationKind::Input,
                                payload:json!({"text":row["text"],"pre_transcript":row["pre_transcript"].as_bool().unwrap_or(false)}) });
                        commands.push(command);
                    }
                    Ok(commands)
                }.await;
                Job::Drained(result)
            });
        }
        let deadline = engine.deadline().into_iter().chain(roots.values().filter(|root|root.result.is_none() && !root.timed_out).map(|root|root.deadline))
            .min_by(f64::total_cmp).map(|seconds|start + Duration::from_secs_f64(seconds.max(0.0)))
            .unwrap_or_else(||Instant::now()+Duration::from_secs(3600));
        tokio::select! {
            message = receiver.recv() => {
                let Some(message) = message else { break };
                match message {
                    Message::Command { command,response,from_queue } => {
                        let id = command.operation_id.clone();
                        if from_queue && roots.get(&id).is_some_and(|pending|pending.result.as_ref().is_some_and(|reply|reply.disposition == Disposition::Deferred)) {
                            roots.remove(&id);
                        }
                        if let Some(pending) = roots.get_mut(&id) {
                            if pending.original != serde_json::to_value(&command).unwrap() {
                                let _ = response.send(Err(failure("operation_reused")));
                            } else if let Some(error) = &pending.error { let _ = response.send(Err(error.clone()));
                            } else if let Some(reply) = &pending.result { let _ = response.send(Ok(reply.clone())); }
                            else { pending.responses.push(response); }
                            continue;
                        }
                        if let Some(saved) = initial.operations.get(&id) {
                            if saved.payload != serde_json::to_value(&command).unwrap() { let _ = response.send(Err(failure("operation_reused"))); continue; }
                            let uncertain = matches!(saved.status,Status::Unknown | Status::Dispatching) || initial.operations.values()
                                .any(|phase|phase.payload["logical_id"] == id && matches!(phase.status,Status::Unknown | Status::Dispatching));
                            if uncertain {
                                let reply = RuntimeReply { operation_id:id.clone(),disposition:Disposition::Unknown,payload:json!({"stored":true}) };
                                roots.insert(id,Pending::stored(command,reply.clone()));
                                let _ = response.send(Ok(reply)); continue;
                            }
                            if !(from_queue && saved.status == Status::Deferred) {
                                if let Ok(reply) = serde_json::from_value::<RuntimeReply>(saved.result.clone()) { let _ = response.send(Ok(reply)); continue; }
                            }
                        }
                        if command.kind == OperationKind::Interrupt {
                            for pending in roots.values_mut().filter(|root|root.preparing && matches!(root.command.kind,OperationKind::Input | OperationKind::Steer)) { pending.cancelled = true; }
                        }
                        arrival += 1;
                        let ticket = arrival;
                        roots.insert(id.clone(),Pending { original:serde_json::to_value(&command).unwrap(),command:command.clone(),responses:vec![response],
                            result:None,preparing:true,cancelled:false,error:None,deadline:clock(start).monotonic_s+30.0,timed_out:false,ready_to_run:false,arrival });
                        let queue = queue.clone(); let target = target.clone(); let sample = clock(start);
                        let preparation = preparation.clone();
                        jobs.spawn(async move {
                            loop {
                                let next = preparation.1.notified();
                                if *preparation.0.lock().await == ticket { break; }
                                next.await;
                            }
                            let result = async {
                                if matches!(command.kind,OperationKind::Input | OperationKind::Steer) {
                                    let entry_id = command.payload["entry_id"].as_str().unwrap_or(&id);
                                    let rows = queue.exec(target.generation,&format!("load:{id}"),sample,Action::Load).await.map_err(io_failure)?;
                                    if !rows.as_array().is_some_and(|rows|rows.iter().any(|row|row["id"] == entry_id)) {
                                        queue.exec(target.generation,&format!("append:{id}"),sample,Action::Append { text:command.payload["text"].as_str().ok_or_else(||failure("input_text"))?.into(),
                                            delivered:false,ts:None,pre_transcript:command.payload["pre_transcript"] == true,entry_id:Some(entry_id.into()) }).await.map_err(io_failure)?;
                                    }
                                }
                                let prepared = queue.exec(target.generation,&format!("prepare:{id}:{}",unique()),sample,Action::Prepare { id:id.clone(),payload:serde_json::to_value(&command).unwrap(),
                                    entry_id:matches!(command.kind,OperationKind::Input | OperationKind::Steer)
                                        .then(||command.payload["entry_id"].as_str().unwrap_or(&id).into()) }).await.map_err(io_failure)?;
                                Ok(stored_reply(&id,&prepared))
                            }.await;
                            *preparation.0.lock().await += 1;
                            preparation.1.notify_waiters();
                            Job::Root { id,result }
                        });
                    }
                    Message::Mods { call,deadline,response } => {
                        effects.extend(take_mods(&mut engine,&mut mods_waiters,&mut mods_token,call,deadline,response,clock(start)));
                    }
                    Message::Queue { call_id,action,response } => {
                        let queue = queue.clone(); let generation = target.generation; let sample = clock(start);
                        let wake = matches!(&action,Action::Append { delivered:false,.. });
                        jobs.spawn(async move {
                            let result = queue.exec(generation,&call_id,sample,action).await.map_err(io_failure);
                            let saved = result.as_ref().map(|_|()).map_err(Clone::clone);
                            let _ = response.send(result);
                            Job::Queued { wake,result:saved }
                        });
                    }
                    Message::Snapshot(response) => {
                        let _ = response.send(Ok(json!({"key":target.key,"generation":target.generation,"revision":revision.value,
                            "view":durable_view,"channels":channels,"error":error.as_ref().map(|e|e.code.clone())})));
                    }
                    Message::Drain(response) => {
                        if engine.view()["deliverable"] != true && !drain_active {
                            let _ = response.send(Ok(json!({"sent":0})));
                        } else {
                            drain_requested = true;
                            drain_waiters.push(response);
                        }
                    }
                    Message::Confirm(response) => {
                        let job = confirm_inputs(queue.clone(),receipt.clone(),target.transcript.clone(),target.generation,clock(start));
                        jobs.spawn(async move { Job::Confirmed { response:Some(response),result:job.await } });
                    }
                    Message::Stop(response) => {
                        closed.store(true,Ordering::Release);
                        jobs.abort_all();
                        while jobs.join_next().await.is_some() {}
                        io.stop().await;
                        let result = queue.exec(target.generation,&format!("stop-repair:{}",unique()),clock(start),Action::EnsureProjection).await
                            .and_then(|_|Ok(())).map_err(io_failure);
                        if let Err(error) = result { let _ = response.send(Err(error.clone())); return Err(error); }
                        queue.exec(target.generation,&format!("stop-recover:{}",unique()),clock(start),Action::Recover).await.map_err(io_failure)?;
                        for pending in roots.values_mut() { for waiter in pending.responses.drain(..) { let _ = waiter.send(Err(failure("runtime_stopped"))); } }
                        let queue = Arc::try_unwrap(queue).map_err(|_|failure("queue_busy"))?;
                        queue.shutdown().await.map_err(io_failure)?;
                        let _ = response.send(Ok(()));
                        return Ok(());
                    }
                }
            }
            event = io.events.recv(), if io_open => {
                match event {
                    // Erro de UMA mensagem segue (como o leitor Python); só o erro de leitura encerra.
                    Some(IoEvent::Line(line)) => match engine.apply(EngineInput::Line(line),clock(start)) {
                        Ok(next)=>effects.extend(next),
                        Err(failure)=>if crate::warn_limit::allow(Some(&target.key),"cli_line") {
                            tracing::warn!(key=%target.key,session=%target.name,reason=%failure.message,"mensagem da CLI ignorada");
                        },
                    },
                    Some(IoEvent::WriteAck { operation_id,outcome }) => {
                        if let Some(attempt) = attempts.get(&operation_id) {
                            let logical_id = attempt.logical_id.clone();
                            let queue = queue.clone(); let generation = target.generation; let sample = clock(start);
                            jobs.spawn(async move {
                                let result = async {
                                  let state = queue.snapshot().await.map_err(io_failure)?;
                                  if state.operations.get(&operation_id).is_some_and(|phase|matches!(phase.status,Status::Accepted | Status::Rejected | Status::Confirmed)) { return Ok(()); }
                                  queue.exec(generation,&format!("ack:{operation_id}:{}",unique()),sample,Action::Finish {
                                    id:operation_id,status:match outcome { WriteOutcome::Written=>Status::Accepted,
                                        WriteOutcome::NotWritten=>Status::Rejected,WriteOutcome::Unknown=>Status::Unknown },
                                    result:json!({"write_outcome":outcome}) }).await.map(|_|()).map_err(io_failure)
                                }.await;
                                Job::Ack { logical_id,outcome,result }
                            });
                        }
                    }
                    Some(IoEvent::Stderr(_)) => {},
                    Some(IoEvent::End { code }) => effects.extend(engine.apply(EngineInput::Line(json!({"type":"cano_saiu","rc":code})),clock(start))?),
                    None => {
                        io_open = false; enter_error(&mut error,&target,failure("cano_closed"));
                        effects.extend(engine.apply(EngineInput::Line(json!({"type":"cano_saiu","rc":null})),clock(start))?);
                    },
                }
            }
            result = jobs.join_next(), if !jobs.is_empty() => {
                let job = result.ok_or_else(||failure("job_missing"))?.map_err(|_|failure("job_panic"))?;
                match job {
                    Job::Root { id,result } => {
                        let stored = match result { Ok(stored)=>stored,Err(failure)=>{ fail_root(&mut roots,&id,failure); continue; } };
                        let pending = roots.get_mut(&id).unwrap();
                        // A fila já tem o desfecho (linha confirmada ou operação final): responde sem escrever no fio.
                        if let Some(reply) = stored {
                            pending.preparing = false;
                            pending.result = Some(reply.clone());
                            for response in pending.responses.drain(..) { let _ = response.send(Ok(reply.clone())); }
                            continue;
                        }
                        // Esgotada: o prazo já respondeu (adiada, sem escrita); uma segunda resposta a tornaria incerta.
                        if pending.timed_out { pending.preparing = false; continue; }
                        if pending.cancelled {
                            pending.preparing = false;
                            effects.push_back(Effect::Reply { operation_id:id,disposition:Disposition::Rejected,
                                payload:json!({"error":"input cancelado antes do envio"}) });
                            continue;
                        }
                        if matches!(pending.command.kind,OperationKind::Input | OperationKind::Steer) && engine.policy.is_some() {
                            let policy = engine.policy.clone().unwrap(); let target = target.clone(); let command = pending.command.clone();
                            jobs.spawn(async move {
                                // Cálculo puro (texto → blocos): sem efeito, não entra no diário e pode repetir.
                                let result = policy.run(&target,"prepare_prompt",&RequestId::String(id.clone()),command.payload,&format!("{id}:prepare_prompt")).await;
                                Job::PreparedInput { id,result }
                            });
                        } else {
                            pending.ready_to_run = true;
                        }
                    }
                    Job::PreparedInput { id,result } => {
                        let Some(pending) = roots.get_mut(&id) else { continue };
                        if pending.cancelled || pending.timed_out { continue; }
                        match result {
                            Ok(payload) => {
                                for (key,value) in payload.as_object().cloned().unwrap_or_default() { pending.command.payload[key] = value; }
                                if target.provider == "claude" && pending.command.payload["native_candidate"] == true {
                                    // O recado nativo sai pelo Python, fora de `attempts`: a partir daqui ele pode ter
                                    // sido escrito, e nem o prazo nem uma falha podem devolvê-lo à fila.
                                    native.insert(id.clone());
                                    let queue = queue.clone(); let target = target.clone(); let policy = engine.policy.clone().unwrap();
                                    let command = pending.command.clone(); let original = pending.original.clone(); let view = engine.view(); let sample = clock(start);
                                    jobs.spawn(async move {
                                        let result = async {
                                            // Uma fase por tentativa: a adiada volta pelo drain e não pode reaproveitar os recibos da anterior.
                                            let phase = format!("{id}:native_message:{}",unique());
                                            let payload = json!({"text":command.payload["text"]});
                                            queue.exec(target.generation,&format!("prepare:{phase}"),sample,Action::Prepare { id:phase.clone(),
                                                payload:json!({"kind":"native_message","request_id":id,"payload":payload}),entry_id:None }).await.map_err(io_failure)?;
                                            let cursor = capture_cursor(&target,&view).await?;
                                            queue.exec(target.generation,&format!("native-cursor:{phase}"),sample,Action::BindDispatch { id:id.clone(),cursor }).await.map_err(io_failure)?;
                                            queue.exec(target.generation,&format!("native-dispatch:{phase}"),sample,Action::BeginDispatch { id:id.clone(),wire_id:phase.clone(),staged:false }).await.map_err(io_failure)?;
                                            queue.exec(target.generation,&format!("dispatch:{phase}"),sample,Action::BeginDispatch { id:phase.clone(),wire_id:phase.clone(),staged:false }).await.map_err(io_failure)?;
                                            let result = policy.run(&target,"native_message",&RequestId::String(id.clone()),payload,&phase).await?;
                                            let outcome = result["outcome"].as_str().ok_or_else(||failure("native_outcome"))?;
                                            let status = match outcome { "written"=>Status::Accepted,"not_written"=>Status::Rejected,"unknown"=>Status::Unknown,_=>return Err(failure("native_outcome")) };
                                            queue.exec(target.generation,&format!("finish:{phase}"),sample,Action::Finish { id:phase.clone(),status,result:result.clone() }).await.map_err(io_failure)?;
                                            if outcome == "not_written" {
                                                queue.exec(target.generation,&format!("native-defer:{phase}"),sample,Action::Finish { id:id.clone(),status:Status::Deferred,result:json!({"not_written":true}) }).await.map_err(io_failure)?;
                                                let state = queue.snapshot().await.map_err(io_failure)?;
                                                let entry_id = state.operations[&id].entry_id.clone();
                                                queue.exec(target.generation,&format!("native-fallback:{phase}"),sample,Action::Prepare { id:id.clone(),payload:original,entry_id }).await.map_err(io_failure)?;
                                            }
                                            Ok(result)
                                        }.await;
                                        Job::NativeInput { id,result }
                                    });
                                } else { pending.ready_to_run = true; }
                            }
                            Err(error)=>defer_unwritten(&mut roots,&attempts,&native,&id,error,&mut effects),
                        }
                    }
                    Job::Write { wire,result } => {
                        let attempt = attempts.get(&wire).unwrap();
                        prepared_writes.insert(attempt.order,(wire,result));
                    }
                    Job::Ack { logical_id,outcome,result } => {
                        match result {
                            Ok(())=>effects.extend(engine.apply(EngineInput::WriteAck { operation_id:logical_id,outcome },clock(start))?),
                            Err(failure)=>{
                                fail_root(&mut roots,&logical_id,failure.clone());
                                publish(&events,&target,&mut revision,"problem",json!({"error_code":failure.code,"message":failure.message}));
                                enter_error(&mut error,&target,failure);
                            }
                        }
                    }
                    Job::Finished { reply,result } => {
                        if let Err(failure) = result { fail_root(&mut roots,&reply.operation_id,failure.clone()); enter_error(&mut error,&target,failure); }
                        else {
                          if !roots.contains_key(&reply.operation_id) {
                              if let Some(command) = initial.operations.get(&reply.operation_id).and_then(|operation|serde_json::from_value::<RuntimeCommand>(operation.payload.clone()).ok()) {
                                  roots.insert(reply.operation_id.clone(),Pending::stored(command,reply.clone()));
                              }
                          }
                          if let Some(pending) = roots.get_mut(&reply.operation_id) {
                            pending.preparing = false;
                            pending.result = Some(reply.clone());
                            for response in pending.responses.drain(..) { let _ = response.send(Ok(reply.clone())); }
                            if reply.disposition == Disposition::Deferred && matches!(pending.command.kind,OperationKind::Input | OperationKind::Steer) {
                                let queue = queue.clone(); let id = reply.operation_id.clone(); let generation = target.generation; let sample = clock(start);
                                jobs.spawn(async move { Job::Saved(queue.exec(generation,&format!("unclaim:{}",unique()),sample,
                                    Action::SetDelivered { entry_id:id,value:false,steered:false }).await.map(|_|()).map_err(io_failure)) });
                            }
                          }
                        }
                    }
                    Job::Queued { wake,result } => {
                        match result {
                            Ok(()) if wake=>drain_requested = true,
                            Ok(())=>{},
                            Err(failure)=>enter_error(&mut error,&target,failure),
                        }
                    }
                    Job::Policy { request_id,kind,phase_id,result } => {
                        let _ = phase_id;
                        match result {
                            Ok(payload) => {
                                effects.extend(engine.apply(EngineInput::PolicyResult { request_id,payload },clock(start))?);
                            }
                            // Linha de status, carimbo, uso e registro que falham só perdem aquela parte: a sessão
                            // segue no Rust (o motivo já foi para o log pelo cliente da política). Sidecar e catálogo
                            // de skills seguram estado da sessão: a falha deles continua levando-a ao Python.
                            Err(failure) if COSMETIC_POLICIES.contains(&kind.as_str()) => {
                                if crate::warn_limit::allow(Some(&target.key),&format!("policy:{kind}")) {
                                    tracing::warn!(key=%target.key,session=%target.name,policy=%kind,code=%failure.code,"serviço cosmético falhou; a sessão segue no Rust");
                                }
                                engine.forget_policy(&request_id);
                                publish(&events,&target,&mut revision,"problem",json!({"error_code":failure.code,"message":failure.message}));
                            },
                            Err(failure)=>{
                                publish(&events,&target,&mut revision,"problem",json!({"error_code":failure.code,"message":failure.message}));
                                enter_error(&mut error,&target,failure);
                            },
                        }
                    }
                    Job::View { version,view,result } => {
                        match result {
                            Ok(()) if version > published_state_version => {
                                published_state_version = version;
                                // Confirmar em todo idle, como o adapter Python: sem isto nenhuma entrada vira
                                // confirmed, a poda não as alcança e a fila enche.
                                let state = view["public_state"]["state"].as_str().unwrap_or("").to_owned();
                                if state == "idle" && last_state != "idle" && !confirming {
                                    confirming = true;
                                    let job = confirm_inputs(queue.clone(),receipt.clone(),target.transcript.clone(),target.generation,clock(start));
                                    jobs.spawn(async move { Job::Confirmed { response:None,result:job.await } });
                                }
                                last_state = state;
                                // Vista igual à publicada não sai: cada aparelho redesenharia a tela à toa.
                                if view != durable_view {
                                    durable_view = view.clone();
                                    publish(&events,&target,&mut revision,"view",view.clone());
                                    publish(&events,&target,&mut revision,"state",view["public_state"].clone());
                                }
                            }
                            Ok(()) => {},
                            Err(failure) => {
                                publish(&events,&target,&mut revision,"problem",json!({"error_code":failure.code,"message":failure.message}));
                                enter_error(&mut error,&target,failure);
                            }
                        }
                    }
                    Job::Saved(result) => { if let Err(failure) = result {
                        publish(&events,&target,&mut revision,"problem",json!({"error_code":failure.code,"message":failure.message}));
                        enter_error(&mut error,&target,failure);
                    } },
                    Job::Drained(result) => {
                        match result {
                            Ok(commands) if commands.is_empty() => {
                                drain_active = false;
                                for waiter in drain_waiters.drain(..) { let _ = waiter.send(Ok(json!({"sent":0}))); }
                            }
                            Ok(commands) => for command in commands {
                                let (response,receive) = oneshot::channel();
                                let internal = internal.clone();
                                jobs.spawn(async move {
                                    let result = match internal.send(Message::Command { command,response,from_queue:true }).await {
                                        Ok(())=>receive.await.map_err(|_|failure("runtime_closed")).and_then(|result|result),
                                        Err(_)=>Err(failure("runtime_closed")),
                                    };
                                    Job::DrainFinished(result)
                                });
                            },
                            Err(failure)=>{
                                drain_active = false;
                                for waiter in drain_waiters.drain(..) { let _ = waiter.send(Err(failure.clone())); }
                                enter_error(&mut error,&target,failure);
                            },
                        }
                    }
                    Job::DrainFinished(result) => {
                        drain_active = false;
                        let count = result.as_ref().map(|reply|usize::from(reply.disposition == Disposition::Accepted));
                        for waiter in drain_waiters.drain(..) {
                            let _ = waiter.send(count.clone().map(|sent|json!({"sent":sent})).map_err(Clone::clone));
                        }
                        if let Err(failure) = result { enter_error(&mut error,&target,failure); }
                    }
                    Job::Confirmed { response,result } => {
                        if response.is_none() { confirming = false; }
                        match result {
                            Ok((ids,legacy))=>{
                                for id in &ids { effects.extend(engine.confirm_input(id)); }
                                if let Some(response) = response { let _ = response.send(Ok(json!({"confirmed":ids.len() + legacy}))); }
                            }
                            Err(error)=>{
                                if crate::warn_limit::allow(Some(&target.key),&error.code) {
                                    tracing::warn!(key=%target.key,session=%target.name,code=%error.code,"confirmação da fila falhou");
                                }
                                if let Some(response) = response { let _ = response.send(Err(error)); }
                            }
                        }
                    }
                    Job::Steered { id,result } => {
                        let (disposition,payload) = match result {
                            Ok(ids)=>(Disposition::Accepted,json!({"ids":ids})),
                            Err(error)=>(Disposition::Unknown,json!({"error":error.message,"error_code":error.code})),
                        };
                        effects.push_back(Effect::Reply { operation_id:id,disposition,payload });
                    }
                    Job::NativeInput { id,result } => {
                        match result {
                            Ok(result) if result["outcome"] == "not_written" => {
                                native.remove(&id);
                                if let Some(root) = roots.get_mut(&id) {
                                    if !root.cancelled && !root.timed_out { root.ready_to_run = true; }
                                }
                            }
                            Ok(result)=>effects.push_back(Effect::Reply { operation_id:id,
                                disposition:if result["outcome"] == "written" { Disposition::Accepted } else { Disposition::Unknown },payload:result }),
                            Err(error)=>{
                                effects.push_back(Effect::Reply { operation_id:id,disposition:Disposition::Unknown,payload:json!({"error_code":error.code,"error":error.message}) });
                            }
                        }
                    }
                }
            }
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
                let sample = clock(start);
                effects.extend(engine.apply(EngineInput::Tick,sample)?);
                for (id,pending) in &mut roots {
                    if pending.result.is_none() && !pending.timed_out && sample.monotonic_s >= pending.deadline {
                        pending.timed_out = true;
                        // Sem escrita começada a entrada não chegou à CLI: volta para a fila em vez de ficar incerta.
                        let prefix = format!("{id}:");
                        let written = native.contains(id) || attempts.values().any(|attempt|attempt.logical_id == *id || attempt.logical_id.starts_with(&prefix));
                        let unsent = !written && matches!(pending.command.kind,OperationKind::Input | OperationKind::Steer);
                        if crate::warn_limit::allow(Some(&target.key),"deadline") {
                            tracing::warn!(key=%target.key,session=%target.name,operation=%id,unsent,"operação sem resposta em 30 s");
                        }
                        effects.push_back(Effect::Reply { operation_id:id.clone(),disposition:if unsent { Disposition::Deferred } else { Disposition::Unknown },
                            payload:json!({"error":"operação sem resposta"}) });
                    }
                }
            },
        }
    }
    closed.store(true,Ordering::Release);
    jobs.abort_all(); while jobs.join_next().await.is_some() {}
    io.stop().await;
    Arc::try_unwrap(queue).map_err(|_|failure("queue_busy"))?.shutdown().await.map_err(io_failure)
}

/// Prova cada entrada despachada e ainda não confirmada contra o transcript, a partir do cursor do
/// despacho. Uma leitura do transcript por rodada; o estado só é relido depois de uma confirmação.
/// Devolve as operações confirmadas e quantas entradas legadas saíram junto.
async fn confirm_inputs(queue:Arc<QueueActor>,receipt:Arc<std::sync::Mutex<ReceiptIndex>>,path:std::path::PathBuf,
    generation:u64,sample:ClockSample) -> Result<(Vec<String>,usize),RuntimeError> {
    let mut state = queue.snapshot().await.map_err(io_failure)?;
    let confirmed_rows:std::collections::BTreeSet<&str> = state.rows.iter().filter(|r|r["confirmed"] == true).filter_map(|r|r["id"].as_str()).collect();
    let candidates:Vec<(String,String,super::receipt::DispatchCursor)> = state.operations.iter()
        .filter(|(_,op)|op.status != Status::Confirmed && matches!(op.payload["kind"].as_str(),Some("input" | "steer")))
        .filter_map(|(id,op)|Some((id.clone(),op.entry_id.clone()?,serde_json::from_value(op.dispatch_cursor.clone()).ok()?)))
        .filter(|(_,entry,_)|!confirmed_rows.contains(entry.as_str())).collect();
    let mut confirmed = Vec::new();
    if candidates.is_empty() && super::queue::legacy_rows(&state).is_empty() { return Ok((confirmed,0)); }
    let scanner = receipt.clone(); let transcript = path.clone();
    tokio::task::spawn_blocking(move || scanner.lock().map_err(|_|failure("receipt_panic"))?.scan(&transcript).map(|_|()).map_err(io_failure))
        .await.map_err(|_|failure("receipt_job"))??;
    for (id,entry,cursor) in candidates {
        let Some(row) = state.rows.iter().find(|r|r["id"] == entry.as_str()).cloned() else { continue };
        if row["confirmed"] == true { continue; }
        let receipt = receipt.clone(); let used = state.used_occurrences.clone(); let transcript = path.clone();
        let proof = tokio::task::spawn_blocking(move || {
            let receipt = receipt.lock().map_err(|_|failure("receipt_panic"))?;
            receipt.match_after(&transcript,&cursor,&row,&used).map_err(io_failure)
        }).await.map_err(|_|failure("receipt_job"))??;
        if let Some(proof) = proof {
            let accepted = queue.exec(generation,&format!("proof:{}",unique()),sample,Action::ConfirmOccurrence { id:id.clone(),proof }).await.map_err(io_failure)?;
            if accepted == true { confirmed.push(id); state = queue.snapshot().await.map_err(io_failure)?; }
        }
    }
    // Depois das despachadas: uma linha que prova a entrega nova não pode ser gasta por uma legada.
    let mut legacy = 0;
    for row in super::queue::legacy_rows(&state) {
        let Some(entry_id) = row["id"].as_str().map(str::to_owned) else { continue };
        let receipt = receipt.clone(); let used = state.used_occurrences.clone();
        let found = tokio::task::spawn_blocking(move || receipt.lock().map(|r|r.match_legacy(&row,&used)).map_err(|_|failure("receipt_panic")))
            .await.map_err(|_|failure("receipt_job"))??;
        let Some((occurrence,normalized_text)) = found else { continue };
        // Falha numa legada não desfaz as despachadas já confirmadas acima: fica para a próxima rodada.
        match queue.exec(generation,&format!("legacy-proof:{}",unique()),sample,Action::ConfirmLegacy { entry_id,occurrence,normalized_text }).await {
            Ok(accepted) => if accepted == true { legacy += 1; state = queue.snapshot().await.map_err(io_failure)?; },
            Err(error) => { let _ = io_failure(error); }  // io_failure já registra no log
        }
    }
    Ok((confirmed,legacy))
}

async fn capture_cursor(target:&RuntimeTarget,view:&Value) -> Result<Value,RuntimeError> {
    let path = target.transcript.clone(); let provider = target.provider.clone(); let conversation = view["conversation"].as_str().unwrap_or("").to_owned();
    tokio::task::spawn_blocking(move ||ReceiptIndex::new(&provider,&conversation).capture(&path))
        .await.map_err(|_|failure("cursor_job"))?.map_err(io_failure).and_then(|cursor|serde_json::to_value(cursor).map_err(|_|failure("cursor_json")))
}

const COSMETIC_POLICIES:[&str;5] = ["format_status","reload_stamp","last_usage","quota","unknown_private"];

#[derive(Default)]
struct SavedView { version:u64, durable:Option<Value>, latest:Value }

/// Campos que mudam a cada evento e que ninguém relê do disco: o motor parte de `in_progress:false`
/// e o estado público é recalculado. O contador sai da comparação porque a escrita no fio salva a
/// vista inteira (`force`) antes de usar um ID novo.
const VOLATILE_VIEW:[&str;9] = ["public_state","alive","iniciando","in_progress","pending","question","deliverable","runtime_counter","turn_id"];

fn durable_part(view:&Value) -> Value {
    let mut durable = view.clone();
    if let Some(fields) = durable.as_object_mut() { for key in VOLATILE_VIEW { fields.remove(key); } }
    durable
}

async fn save_view(queue:&QueueActor,generation:u64,sample:ClockSample,gate:&Mutex<SavedView>,version:u64,view:&Value,force:bool) -> Result<(),RuntimeError> {
    let mut saved = gate.lock().await;
    if version >= saved.version { saved.version = version; saved.latest = view.clone(); }
    else if !force { return Ok(()); }
    // A forçada pode chegar depois de uma mudança de estado mais nova que pulou o disco: grava a vista
    // mais nova conhecida, cujo contador é o maior.
    let latest = saved.latest.clone();
    let durable = durable_part(&latest);
    if force || saved.durable.as_ref() != Some(&durable) {
        queue.exec(generation,&format!("state:{}",unique()),sample,
            Action::SetRuntimeState { state:json!({"view":latest}) }).await.map_err(io_failure)?;
        saved.durable = Some(durable);
    }
    Ok(())
}

/// Operação que a fila devolve já final nunca volta a ser enviada; sem resposta guardada no
/// formato de RuntimeReply, a disposição sai do status.
fn stored_reply(id:&str,prepared:&Value) -> Option<RuntimeReply> {
    let disposition = match prepared["status"].as_str()? {
        "accepted" | "confirmed"=>Disposition::Accepted, "rejected"=>Disposition::Rejected, _=>return None,
    };
    serde_json::from_value(prepared["result"].clone()).ok()
        .or_else(||Some(RuntimeReply { operation_id:id.into(),disposition,payload:Value::Null }))
}

fn status(disposition:Disposition) -> Status {
    match disposition { Disposition::Accepted=>Status::Accepted,Disposition::Deferred=>Status::Deferred,
        Disposition::Rejected=>Status::Rejected,Disposition::Unknown=>Status::Unknown }
}

fn recover_phase(state:&super::queue::State,phase:&super::queue::Operation) -> bool {
    phase.payload["generation"].as_u64() == Some(state.generation)
        && phase.payload["frame"].is_object() && (matches!(phase.status,Status::Unknown | Status::Dispatching)
        || phase.payload["logical_id"].as_str().and_then(|id|state.operations.get(id))
            .is_some_and(|root|matches!(root.status,Status::Unknown | Status::Dispatching)))
}

/// Falha antes de qualquer escrita no fio: nada chegou à CLI, então a entrada volta para a fila
/// (adiada, como o `deferred` do Python) em vez de ficar incerta e presa para sempre.
fn defer_unwritten(roots:&mut BTreeMap<String,Pending>,attempts:&BTreeMap<String,Attempt>,native:&BTreeSet<String>,id:&str,error:RuntimeError,effects:&mut VecDeque<Effect>) {
    let prefix = format!("{id}:");
    let written = native.contains(id) || attempts.values().any(|attempt|attempt.logical_id == id || attempt.logical_id.starts_with(&prefix));
    let input = roots.get(id).is_some_and(|pending|matches!(pending.command.kind,OperationKind::Input | OperationKind::Steer));
    if written || !input { return fail_root(roots,id,error); }
    let Some(pending) = roots.get_mut(id) else { return };
    if crate::warn_limit::allow(Some(id),&error.code) {
        tracing::warn!(operation=%id,code=%error.code,reason=%error.message,"entrada adiada antes de qualquer escrita");
    }
    pending.preparing = false;
    // Adiada não é erro: quem espera (API ou drain) vê a entrada de volta na fila, e o drain não põe
    // a sessão inteira em erro por isso.
    let payload = json!({"error_code":error.code});
    let reply = RuntimeReply { operation_id:id.into(),disposition:Disposition::Deferred,payload:payload.clone() };
    for response in pending.responses.drain(..) { let _ = response.send(Ok(reply.clone())); }
    pending.result = Some(reply);
    effects.push_back(Effect::Reply { operation_id:id.into(),disposition:Disposition::Deferred,payload });
}

fn fail_root(roots:&mut BTreeMap<String,Pending>,id:&str,error:RuntimeError) {
    if let Some(pending) = roots.get_mut(id) {
        pending.error = Some(error.clone()); pending.preparing = false;
        for response in pending.responses.drain(..) { let _ = response.send(Err(error.clone())); }
    }
}

struct Revision { value:u64,counter:Arc<AtomicU64> }

fn publish(events:&broadcast::Sender<RuntimeEvent>,target:&RuntimeTarget,revision:&mut Revision,channel:&str,data:Value) {
    revision.value += 1;
    revision.counter.store(revision.value,Ordering::Release);
    let _ = events.send(RuntimeEvent { key:target.key.clone(),generation:target.generation,revision:revision.value,channel:channel.into(),data });
}

type ModsWaiters = BTreeMap<u64,oneshot::Sender<Result<Value,ModsError>>>;

/// Pedido de app que chegou à vez. Já vencido (o `timeout` do app venceu com a mensagem na caixa), não
/// roda: virar clique depois que o app mostrou erro seria um clique fantasma.
fn take_mods(engine:&mut RuntimeEngine,waiters:&mut ModsWaiters,token:&mut u64,call:ModsCall,deadline:Instant,
    response:oneshot::Sender<Result<Value,ModsError>>,clock:ClockSample) -> Vec<Effect> {
    if Instant::now() >= deadline { let _ = response.send(Err(crate::mods::model::no_answer())); return Vec::new(); }
    *token += 1;
    match engine.mods_call(*token,call,clock) {
        Ok(next) => { waiters.insert(*token,response); next }
        Err(error) => { let _ = response.send(Err(error)); Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mods_call_to_an_actor_that_never_answers_gives_up_with_a_code() {
        // Ator vivo que não lê a caixa: o pedido do app não pode pendurar o registro da sessão.
        let (sender,_inbox) = mpsc::channel(1);
        let handle = RuntimeHandle { sender,task:Arc::new(Mutex::new(None)),closed:Arc::new(AtomicBool::new(false)),
            events:broadcast::channel(1).0,stopped:Arc::new(Mutex::new(None)),key:"key".into() };
        let call = || ModsCall::Show { site:"p".into() };
        let first = tokio::time::timeout(Duration::from_secs(2),handle.mods_within(call(),Duration::from_millis(50))).await.unwrap();
        assert_eq!(first.unwrap_err().code,"erro_mod_clique_sem_resposta");
        // A caixa cheia (o primeiro pedido ficou nela) também cai no prazo, agora no envio.
        let second = tokio::time::timeout(Duration::from_secs(2),handle.mods_within(call(),Duration::from_millis(50))).await.unwrap();
        assert_eq!(second.unwrap_err().code,"erro_mod_clique_sem_resposta");
        assert_eq!(MODS_CALL_LIMIT,Duration::from_secs(7));
    }

    #[test]
    fn mods_call_that_waited_past_its_deadline_in_the_inbox_does_not_run() {
        let mut engine = RuntimeEngine::new("claude",json!({"name":"session","initialized":true}),1,ClockSample { monotonic_s:0.0,epoch_s:0.0 })
            .unwrap().with_mods(crate::mods::state::Mods::default());
        let (mut waiters,mut token) = (ModsWaiters::new(),0u64);
        let press = || ModsCall::Press { site:"above-prompt".into(),key:"k".into() };
        let sample = ClockSample { monotonic_s:1.0,epoch_s:1.0 };
        // Vencido: responde sem levar o pedido à superfície (nenhum efeito, nenhum token gasto).
        let (response,mut receive) = oneshot::channel();
        let effects = take_mods(&mut engine,&mut waiters,&mut token,press(),Instant::now() - Duration::from_millis(1),response,sample);
        assert!(effects.is_empty() && waiters.is_empty() && token == 0);
        assert_eq!(receive.try_recv().unwrap().unwrap_err().code,"erro_mod_clique_sem_resposta");
        // No prazo: vai à superfície, que ainda não ligou e responde que o botão não está lá.
        let (response,_receive) = oneshot::channel();
        let effects = take_mods(&mut engine,&mut waiters,&mut token,press(),Instant::now() + Duration::from_secs(5),response,sample);
        assert!(effects.iter().any(|effect|matches!(effect,Effect::Surface { effect:SurfaceEffect::Reply { token:1,result:Err(error) } }
            if error.code == "erro_mod_botao_inexistente")));
        assert!(waiters.contains_key(&1));
    }

    #[tokio::test]
    async fn a_forced_save_after_a_skipped_newer_change_still_saves_the_counter() {
        // Mudança de estado mais nova que pulou o disco não pode fazer a gravação forçada (antes de
        // uma escrita no fio) ser descartada: o contador dos IDs precisa estar salvo.
        let dir = tempfile::tempdir().unwrap();
        let lease = super::super::queue::acquire_lease(&dir.path().join("key.lock")).unwrap();
        let store = super::super::queue::Store::open(&dir.path().join("state.json"),&dir.path().join("projection"),
            super::super::queue::State::new("key",1,"session",vec![])).unwrap();
        let queue = QueueActor::start(store,lease);
        let gate = Mutex::new(SavedView::default());
        let sample = ClockSample { monotonic_s:0.0,epoch_s:0.0 };
        let view = |counter:u64,working:bool| json!({"model":"m","runtime_counter":counter,"in_progress":working});
        save_view(&queue,1,sample,&gate,1,&view(1,false),false).await.unwrap();
        save_view(&queue,1,sample,&gate,3,&view(3,true),false).await.unwrap();
        save_view(&queue,1,sample,&gate,2,&view(2,true),true).await.unwrap();
        let state = queue.snapshot().await.unwrap();
        assert_eq!(state.runtime_state["view"]["runtime_counter"],3);
        queue.shutdown().await.unwrap();
    }
}
