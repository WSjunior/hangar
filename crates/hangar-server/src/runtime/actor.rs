use super::{cano::{CanoConnection,IoEvent,IoTasks,WireFrame},claude::ClaudeEngine,codex::Engine as CodexEngine,
    protocol::*,queue::{Action,QueueActor,Status},receipt::ReceiptIndex};
use serde_json::{Value,json};
use std::collections::{BTreeMap,VecDeque};
use std::sync::{Arc,atomic::{AtomicBool,AtomicU64,Ordering}};
use std::time::{Duration,Instant,SystemTime,UNIX_EPOCH};
use tokio::sync::{broadcast,mpsc,oneshot,Mutex,Notify};
use tokio::task::{JoinHandle,JoinSet};

enum Core { Claude(ClaudeEngine),Codex(CodexEngine) }

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
    async fn run(&self,target:&RuntimeTarget,kind:&str,request_id:&RequestId,payload:Value) -> Result<Value,RuntimeError> {
        let body = json!({"key":target.key,"generation":target.generation,"request_id":request_id,"kind":kind,"payload":payload});
        let request = axum::http::Request::post(format!("http://{}/internal/runtime/policy",self.upstream))
            .header("x-hangar-internal",&self.secret).header("x-hangar-runtime-instance",&self.instance)
            .header("content-type","application/json").body(axum::body::Body::from(body.to_string()))
            .map_err(|_|failure("policy_request"))?;
        let result = tokio::time::timeout(Duration::from_secs(15),async {
            let response = self.http.request(request).await.map_err(|_|failure("policy_transport"))?;
            if !response.status().is_success() { return Err(failure("policy_refused")); }
            let bytes = axum::body::to_bytes(axum::body::Body::new(response.into_body()),MAX_ENVELOPE)
                .await.map_err(|_|failure("policy_limit"))?;
            let value:Value = serde_json::from_slice(&bytes).map_err(|_|failure("policy_json"))?;
            if value["ok"] != true { return Err(failure("policy_failed")); }
            Ok(value["data"].clone())
        }).await.map_err(|_|failure("policy_timeout"))?;
        result
    }
}

pub struct RuntimeEngine {
    core:Core,
    policy:Option<PolicyClient>,
    publisher:Option<broadcast::Sender<RuntimeEvent>>,
    revision:Arc<AtomicU64>,
}

impl RuntimeEngine {
    pub fn new(provider:&str,metadata:Value,generation:u64,clock:ClockSample) -> Result<Self,RuntimeError> {
        let core = match provider { "claude"=>Core::Claude(ClaudeEngine::new(metadata,generation,clock)),
            "codex"=>Core::Codex(CodexEngine::new(metadata,generation,clock)),_=>return Err(failure("provider")) };
        Ok(Self { core,policy:None,publisher:None,revision:Arc::new(AtomicU64::new(0)) })
    }
    pub fn with_policy(mut self,policy:PolicyClient) -> Self { self.policy = Some(policy); self }
    pub fn with_publisher(mut self,publisher:broadcast::Sender<RuntimeEvent>) -> Self { self.publisher = Some(publisher); self }
    pub fn with_revision(mut self,revision:Arc<AtomicU64>) -> Self { self.revision = revision; self }
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
    fn write_is_current(&self,id:&str) -> bool { match &self.core { Core::Claude(_)=>true,Core::Codex(core)=>core.write_is_current(id) } }
    fn confirm_input(&mut self,id:&str) -> Vec<Effect> {
        match &mut self.core { Core::Claude(_)=>vec![Effect::Reply { operation_id:id.into(),disposition:Disposition::Accepted,payload:json!({"confirmed":true}) }],
            Core::Codex(core)=>core.confirm_input(id) }
    }
    fn restore(&mut self,state:&super::queue::State) {
        for phase in state.operations.values().filter(|phase|recover_phase(state,phase)) {
            if let Some(id) = phase.payload["logical_id"].as_str() {
                match &mut self.core {
                    Core::Codex(core)=>core.restore_rpc(id.into(),&phase.payload["frame"],phase.payload["state_revision"].as_u64().unwrap_or(0),
                        phase.payload["settings_revision"].as_u64().unwrap_or(0)),
                    Core::Claude(core)=>core.restore_control(id.into(),&phase.payload["frame"]),
                }
            }
        }
    }
}

fn failure(code:&str) -> RuntimeError { RuntimeError::new(code,"runtime indisponível; operação conservada no diário") }
fn io_failure(_:std::io::Error) -> RuntimeError { failure("queue_io") }
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
    Stop(oneshot::Sender<Result<(),RuntimeError>>),
}

#[derive(Clone)]
pub struct RuntimeHandle {
    sender:mpsc::Sender<Message>,
    task:Arc<Mutex<Option<JoinHandle<Result<(),RuntimeError>>>>>,
    closed:Arc<AtomicBool>,
    events:broadcast::Sender<RuntimeEvent>,
    stopped:Arc<Mutex<Option<Result<(),RuntimeError>>>>,
}

impl RuntimeHandle {
    pub async fn command(&self,command:RuntimeCommand) -> Result<RuntimeReply,RuntimeError> {
        if self.closed.load(Ordering::Acquire) { return Err(failure("runtime_stopping")); }
        let (response,receive) = oneshot::channel();
        self.sender.send(Message::Command { command,response,from_queue:false }).await.map_err(|_|failure("runtime_closed"))?;
        receive.await.map_err(|_|failure("runtime_closed"))?
    }
    pub async fn queue(&self,call_id:String,action:Action) -> Result<Value,RuntimeError> {
        let (response,receive) = oneshot::channel();
        self.sender.send(Message::Queue { call_id,action,response }).await.map_err(|_|failure("runtime_closed"))?;
        receive.await.map_err(|_|failure("runtime_closed"))?
    }
    pub async fn snapshot(&self) -> Result<Value,RuntimeError> {
        let (send,receive) = oneshot::channel();
        self.sender.send(Message::Snapshot(send)).await.map_err(|_|failure("runtime_closed"))?;
        receive.await.map_err(|_|failure("runtime_closed"))?
    }
    pub async fn drain(&self) -> Result<Value,RuntimeError> {
        let (send,receive) = oneshot::channel(); self.sender.send(Message::Drain(send)).await.map_err(|_|failure("runtime_closed"))?;
        receive.await.map_err(|_|failure("runtime_closed"))?
    }
    pub async fn confirm(&self) -> Result<Value,RuntimeError> {
        let (send,receive) = oneshot::channel(); self.sender.send(Message::Confirm(send)).await.map_err(|_|failure("runtime_closed"))?;
        receive.await.map_err(|_|failure("runtime_closed"))?
    }
    pub async fn ensure_projection(&self) -> Result<Value,RuntimeError> {
        self.queue(format!("projection:{}",unique()),Action::EnsureProjection).await
    }
    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> { self.events.subscribe() }
    pub async fn stop(&self) -> Result<(),RuntimeError> {
        let mut stopped = self.stopped.lock().await;
        if let Some(result) = &*stopped { return result.clone(); }
        self.closed.store(true,Ordering::Release);
        let (send,receive) = oneshot::channel();
        let mut result = match self.sender.send(Message::Stop(send)).await {
            Ok(())=>receive.await.map_err(|_|failure("runtime_closed")).and_then(|result|result),
            Err(_)=>Err(failure("runtime_closed")),
        };
        if let Some(task) = self.task.lock().await.take() {
            let joined = task.await.map_err(|_|failure("runtime_panic")).and_then(|result|result);
            if joined.is_err() { result = joined; }
        }
        *stopped = Some(result.clone());
        result
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
    Root { id:String,result:Result<(),RuntimeError> },
    Write { wire:String,result:Result<(),RuntimeError> },
    Ack { logical_id:String,outcome:WriteOutcome,result:Result<(),RuntimeError> },
    Finished { reply:RuntimeReply,result:Result<(),RuntimeError> },
    Policy { request_id:RequestId,kind:String,phase_id:String,result:Result<Value,RuntimeError> },
    PreparedInput { id:String,result:Result<Value,RuntimeError> },
    Saved(Result<(),RuntimeError>),
    View { version:u64,view:Value,result:Result<(),RuntimeError> },
    Drained(Result<Vec<RuntimeCommand>,RuntimeError>),
    DrainFinished(Result<RuntimeReply,RuntimeError>),
    Confirmed { response:oneshot::Sender<Result<Value,RuntimeError>>,result:Result<Vec<String>,RuntimeError> },
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
        let task = tokio::spawn(run(target,queue,connection,engine,receiver,sender.clone(),closed.clone(),events.clone()));
        RuntimeHandle { sender,task:Arc::new(Mutex::new(Some(task))),closed,events,stopped:Arc::new(Mutex::new(None)) }
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
    let state_gate = Arc::new(Mutex::new(0u64));
    let mut durable_view = json!({"alive":true,"initialized":false,"ready":false});
    let mut channels:BTreeMap<String,Value> = ["preview","thinking","tool"].into_iter().map(|channel|
        (channel.into(),json!({"session":target.name,"text":"","md":true,"full":true,"vivo":true}))).collect();
    let receipt = Arc::new(std::sync::Mutex::new(ReceiptIndex::new(&target.provider,engine.view()["conversation"].as_str().unwrap_or(""))));
    let mut sequence = initial.operations.keys().filter_map(|id|id.rsplit(':').next()?.parse::<u64>().ok()).max().unwrap_or(0);
    let mut write_order = 0u64;
    let mut next_write = 1u64;
    let mut prepared_writes = BTreeMap::new();
    let mut error:Option<RuntimeError> = None;
    let mut io_open = true;
    let mut drain_requested = true;
    let mut drain_active = false;
    let mut drain_waiters = Vec::new();
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
            match engine.command(pending.command.clone(),clock(start)) {
                Ok(next)=>effects.extend(next),Err(error)=>fail_root(&mut roots,&id,error),
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
                                    "state_revision":view["state_revision"],"settings_revision":view["settings_revision"]}),entry_id }).await.map_err(io_failure)?;
                            save_view(&queue,target.generation,sample,&state_gate,state_version,&view).await?;
                            let cursor = capture_cursor(&target,&view).await?;
                            queue.exec(target.generation,&format!("cursor:{wire}"),sample,Action::BindDispatch { id:wire.clone(),cursor:cursor.clone() }).await.map_err(io_failure)?;
                            if authoritative.operations.get(&logical_id).is_none_or(|op|op.status == Status::Prepared) {
                                queue.exec(target.generation,&format!("logical-cursor:{wire}"),sample,Action::BindDispatch { id:logical_id.clone(),cursor }).await.map_err(io_failure)?;
                            }
                            queue.exec(target.generation,&format!("dispatch:{wire}"),sample,Action::BeginDispatch { id:wire.clone(),wire_id:wire.clone() }).await.map_err(io_failure)?;
                            queue.exec(target.generation,&format!("logical-dispatch:{wire}"),sample,Action::BeginDispatch { id:logical_id.clone(),wire_id:wire.clone() }).await.map_err(io_failure)?;
                            Ok(())
                        }.await;
                        Job::Write { wire,result }
                    });
                }
                Effect::Publish { channel,data } => {
                    if ["preview","thinking","tool"].contains(&channel.as_str()) {
                        channels.insert(channel.clone(),data.clone()); publish(&events,&target,&mut revision,&channel,data);
                    }
                }
                Effect::StateChanged => {
                    let view = engine.view();
                    state_version += 1;
                    let version = state_version;
                    let queue = queue.clone(); let generation = target.generation; let sample = clock(start);
                    let gate = state_gate.clone();
                    jobs.spawn(async move {
                        let result = save_view(&queue,generation,sample,&gate,version,&view).await;
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
                    let queue = queue.clone(); let target = target.clone(); let policy = engine.policy.clone(); let sample = clock(start);
                    jobs.spawn(async move {
                        let result = async {
                            queue.exec(target.generation,&format!("prepare:{phase_id}"),sample,Action::Prepare { id:phase_id.clone(),
                                payload:json!({"kind":kind,"request_id":request_id,"payload":payload}),entry_id:None }).await.map_err(io_failure)?;
                            queue.exec(target.generation,&format!("dispatch:{phase_id}"),sample,Action::BeginDispatch { id:phase_id.clone(),wire_id:phase_id.clone() }).await.map_err(io_failure)?;
                            let result = policy.ok_or_else(||failure("policy_unavailable"))?.run(&target,&kind,&request_id,payload).await;
                            let (status,stored) = match &result {
                                Ok(payload)=>(Status::Accepted,payload.clone()),
                                Err(error)=>(Status::Unknown,json!({"error_code":error.code})),
                            };
                            queue.exec(target.generation,&format!("finish:{phase_id}"),sample,
                                Action::Finish { id:phase_id.clone(),status,result:stored }).await.map_err(io_failure)?;
                            result
                        }.await;
                        Job::Policy { request_id,kind,phase_id,result }
                    });
                }
                Effect::WakeQueue => { drain_requested = true; },
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
                if let Err(failure) = result { error = Some(failure); }
            } else if io.writer.try_send(WireFrame { operation_id:wire,frame:attempt.frame.clone() }).is_err() {
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
                                    let rows = queue.exec(target.generation,&format!("load:{id}"),sample,Action::Load).await.map_err(io_failure)?;
                                    if !rows.as_array().is_some_and(|rows|rows.iter().any(|row|row["id"] == id)) {
                                        queue.exec(target.generation,&format!("append:{id}"),sample,Action::Append { text:command.payload["text"].as_str().ok_or_else(||failure("input_text"))?.into(),
                                            delivered:false,ts:None,pre_transcript:command.payload["pre_transcript"] == true,entry_id:Some(id.clone()) }).await.map_err(io_failure)?;
                                    }
                                }
                                queue.exec(target.generation,&format!("prepare:{id}:{}",unique()),sample,Action::Prepare { id:id.clone(),payload:serde_json::to_value(&command).unwrap(),
                                    entry_id:matches!(command.kind,OperationKind::Input | OperationKind::Steer).then(||id.clone()) }).await.map_err(io_failure)?;
                                Ok(())
                            }.await;
                            *preparation.0.lock().await += 1;
                            preparation.1.notify_waiters();
                            Job::Root { id,result }
                        });
                    }
                    Message::Queue { call_id,action,response } => {
                        let queue = queue.clone(); let generation = target.generation; let sample = clock(start);
                        jobs.spawn(async move { let _ = response.send(queue.exec(generation,&call_id,sample,action).await.map_err(io_failure)); Job::Saved(Ok(())) });
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
                        let queue = queue.clone(); let receipt = receipt.clone(); let target = target.clone(); let sample = clock(start);
                        jobs.spawn(async move {
                          let result = async {
                            let mut confirmed = Vec::new();
                            let current = queue.snapshot().await.map_err(io_failure)?;
                            for (id,operation) in &current.operations {
                                if !matches!(operation.payload["kind"].as_str(),Some("input" | "steer")) { continue; }
                                let Some(entry) = operation.entry_id.as_deref() else { continue };
                                let Ok(cursor) = serde_json::from_value(operation.dispatch_cursor.clone()) else { continue };
                                let state = queue.snapshot().await.map_err(io_failure)?;
                                if let Some(row) = state.rows.iter().find(|r|r["id"] == entry).cloned() {
                                    let receipt = receipt.clone(); let path = target.transcript.clone();
                                    let proof = tokio::task::spawn_blocking(move || {
                                        let mut receipt = receipt.lock().map_err(|_|failure("receipt_panic"))?;
                                        receipt.scan(&path).map_err(io_failure)?;
                                        Ok::<_,RuntimeError>(receipt.match_after(&cursor,&row,&state.used_occurrences))
                                    }).await.map_err(|_|failure("receipt_job"))??;
                                    if let Some(proof) = proof {
                                        let accepted = queue.exec(target.generation,&format!("proof:{}",unique()),sample,Action::ConfirmOccurrence { id:id.clone(),proof }).await.map_err(io_failure)?;
                                        if accepted == true { confirmed.push(id.clone()); }
                                    }
                                }
                            }
                            Ok(confirmed)
                          }.await;
                          Job::Confirmed { response,result }
                        });
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
                    Some(IoEvent::Line(line)) => effects.extend(engine.apply(EngineInput::Line(line),clock(start))?),
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
                        io_open = false; error = Some(failure("cano_closed"));
                        effects.extend(engine.apply(EngineInput::Line(json!({"type":"cano_saiu","rc":null})),clock(start))?);
                    },
                }
            }
            result = jobs.join_next(), if !jobs.is_empty() => {
                let job = result.ok_or_else(||failure("job_missing"))?.map_err(|_|failure("job_panic"))?;
                match job {
                    Job::Root { id,result } => {
                        if let Err(failure) = result { fail_root(&mut roots,&id,failure); continue; }
                        let pending = roots.get_mut(&id).unwrap();
                        if pending.cancelled || pending.timed_out {
                            pending.preparing = false;
                            effects.push_back(Effect::Reply { operation_id:id,disposition:if pending.cancelled { Disposition::Rejected } else { Disposition::Unknown },
                                payload:json!({"error":if pending.cancelled { "input cancelado antes do envio" } else { "preparação sem resposta" }}) });
                            continue;
                        }
                        if matches!(pending.command.kind,OperationKind::Input | OperationKind::Steer) && engine.policy.is_some() {
                            let queue = queue.clone(); let policy = engine.policy.clone().unwrap(); let target = target.clone(); let command = pending.command.clone(); let sample = clock(start);
                            jobs.spawn(async move {
                                let result = async {
                                    let phase = format!("{id}:prepare_prompt");
                                    queue.exec(target.generation,&format!("prepare:{phase}"),sample,Action::Prepare { id:phase.clone(),payload:command.payload.clone(),entry_id:None }).await.map_err(io_failure)?;
                                    queue.exec(target.generation,&format!("dispatch:{phase}"),sample,Action::BeginDispatch { id:phase.clone(),wire_id:phase }).await.map_err(io_failure)?;
                                    let payload = policy.run(&target,"prepare_prompt",&RequestId::String(id.clone()),command.payload).await?;
                                    queue.exec(target.generation,&format!("finish:{id}:prepare_prompt"),sample,Action::Finish {
                                        id:format!("{id}:prepare_prompt"),status:Status::Accepted,result:payload.clone() }).await.map_err(io_failure)?;
                                    Ok(payload)
                                }.await;
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
                                pending.ready_to_run = true;
                            }
                            Err(error)=>fail_root(&mut roots,&id,error),
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
                                error = Some(failure);
                            }
                        }
                    }
                    Job::Finished { reply,result } => {
                        if let Err(failure) = result { fail_root(&mut roots,&reply.operation_id,failure.clone()); error = Some(failure); }
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
                    Job::Policy { request_id,kind:_,phase_id,result } => {
                        let _ = phase_id;
                        match result {
                            Ok(payload) => {
                                effects.extend(engine.apply(EngineInput::PolicyResult { request_id,payload },clock(start))?);
                            }
                            Err(failure)=>{
                                publish(&events,&target,&mut revision,"problem",json!({"error_code":failure.code,"message":failure.message}));
                                error = Some(failure);
                            },
                        }
                    }
                    Job::View { version,view,result } => {
                        match result {
                            Ok(()) if version > published_state_version => {
                                published_state_version = version;
                                durable_view = view.clone();
                                publish(&events,&target,&mut revision,"view",view.clone());
                                publish(&events,&target,&mut revision,"state",view["public_state"].clone());
                            }
                            Ok(()) => {},
                            Err(failure) => {
                                publish(&events,&target,&mut revision,"problem",json!({"error_code":failure.code,"message":failure.message}));
                                error = Some(failure);
                            }
                        }
                    }
                    Job::Saved(result) => { if let Err(failure) = result {
                        publish(&events,&target,&mut revision,"problem",json!({"error_code":failure.code,"message":failure.message}));
                        error = Some(failure);
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
                                error = Some(failure);
                            },
                        }
                    }
                    Job::DrainFinished(result) => {
                        drain_active = false;
                        let count = result.as_ref().map(|reply|usize::from(reply.disposition == Disposition::Accepted));
                        for waiter in drain_waiters.drain(..) {
                            let _ = waiter.send(count.clone().map(|sent|json!({"sent":sent})).map_err(Clone::clone));
                        }
                        if let Err(failure) = result { error = Some(failure); }
                    }
                    Job::Confirmed { response,result } => {
                        match result {
                            Ok(ids)=>{
                                for id in &ids { effects.extend(engine.confirm_input(id)); }
                                let _ = response.send(Ok(json!({"confirmed":ids.len()})));
                            }
                            Err(error)=>{ let _ = response.send(Err(error)); }
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
                        effects.push_back(Effect::Reply { operation_id:id.clone(),disposition:Disposition::Unknown,
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

async fn capture_cursor(target:&RuntimeTarget,view:&Value) -> Result<Value,RuntimeError> {
    let path = target.transcript.clone(); let provider = target.provider.clone(); let conversation = view["conversation"].as_str().unwrap_or("").to_owned();
    tokio::task::spawn_blocking(move ||ReceiptIndex::new(&provider,&conversation).capture(&path))
        .await.map_err(|_|failure("cursor_job"))?.map_err(io_failure).and_then(|cursor|serde_json::to_value(cursor).map_err(|_|failure("cursor_json")))
}

async fn save_view(queue:&QueueActor,generation:u64,sample:ClockSample,gate:&Mutex<u64>,version:u64,view:&Value) -> Result<(),RuntimeError> {
    let mut saved = gate.lock().await;
    if version >= *saved {
        queue.exec(generation,&format!("state:{}",unique()),sample,
            Action::SetRuntimeState { state:json!({"view":view}) }).await.map_err(io_failure)?;
        *saved = version;
    }
    Ok(())
}

fn status(disposition:Disposition) -> Status {
    match disposition { Disposition::Accepted=>Status::Accepted,Disposition::Deferred=>Status::Deferred,
        Disposition::Rejected=>Status::Rejected,Disposition::Unknown=>Status::Unknown }
}

fn recover_phase(state:&super::queue::State,phase:&super::queue::Operation) -> bool {
    phase.payload["frame"].is_object() && (matches!(phase.status,Status::Unknown | Status::Dispatching)
        || phase.payload["logical_id"].as_str().and_then(|id|state.operations.get(id))
            .is_some_and(|root|matches!(root.status,Status::Unknown | Status::Dispatching)))
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
