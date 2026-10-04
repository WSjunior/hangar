//! Executor terminal serial; diário, posse e confirmação são os mesmos da fila.
use super::{actor::PolicyClient,protocol::{ClockSample,Disposition,RequestId,RuntimeCommand,RuntimeError,RuntimeEvent,RuntimeReply},queue::{Action,QueueActor,Status},receipt::{DispatchCursor,ReceiptIndex}};
use crate::terminal_input::{self as input,TerminalBinding,TerminalDriver,TerminalIo,TerminalServices,InputFacts,InputLimits,PluginRequest,PluginReply,ServiceFuture,ServiceError,DeliveryResult,QuestionAnswer,AnswerKind};
use serde_json::{Value,json};
use std::{path::PathBuf,sync::{Arc,atomic::{AtomicBool,AtomicU64,Ordering}},time::{Duration,SystemTime,UNIX_EPOCH}};
use tokio::sync::{Mutex,mpsc,oneshot,broadcast};

#[derive(Clone)]
pub struct TerminalTarget {
    pub key:String,pub generation:u64,pub name:String,pub binding:TerminalBinding,
    pub lease_path:PathBuf,pub state_path:PathBuf,pub projection_dir:PathBuf,pub transcript:PathBuf,pub created:f64,
}
pub struct TerminalOptions { pub io:Arc<dyn TerminalIo>,pub limits:InputLimits,pub tick:Duration }
impl Default for TerminalOptions {
    fn default()->Self {Self {io:Arc::new(input::ProcessIo::default()),limits:InputLimits::default(),tick:Duration::from_secs(1)}}
}
fn error(code:&str)->RuntimeError {RuntimeError::new(code,"operação terminal conservada no diário")}
fn sample()->ClockSample {ClockSample {monotonic_s:0.0,epoch_s:SystemTime::now().duration_since(UNIX_EPOCH).map(|d|d.as_secs_f64()).unwrap_or(0.0)}}
fn safe_characters(text:&str)->bool {!text.chars().any(|c| c.is_control() && !matches!(c,'\n'|'\t'))}
fn valid_text(text:&str)->bool {!text.trim().is_empty() && safe_characters(text)}
fn status(disposition:Disposition)->Status {match disposition {Disposition::Accepted=>Status::Accepted,Disposition::Deferred=>Status::Deferred,Disposition::Rejected=>Status::Rejected,Disposition::Unknown=>Status::Unknown}}
fn reply(id:&str,disposition:Disposition,payload:Value)->RuntimeReply {RuntimeReply {operation_id:id.into(),disposition,payload}}
fn delivery(id:&str,result:DeliveryResult)->RuntimeReply {
    let disposition=match result.disposition {input::Disposition::Accepted=>Disposition::Accepted,input::Disposition::Deferred=>Disposition::Deferred,input::Disposition::Rejected=>Disposition::Rejected,input::Disposition::Unknown=>Disposition::Unknown};
    reply(id,disposition,serde_json::to_value(result).unwrap())
}

enum Message {
    Command {id:String,kind:String,payload:Value,response:oneshot::Sender<Result<RuntimeReply,RuntimeError>>},
    Queue {id:String,action:Action,response:oneshot::Sender<Result<Value,RuntimeError>>},
    Snapshot(oneshot::Sender<Result<Value,RuntimeError>>),Drain(oneshot::Sender<Result<Value,RuntimeError>>),
    Confirm(oneshot::Sender<Result<Value,RuntimeError>>),Stop(oneshot::Sender<Result<(),RuntimeError>>),
}
#[derive(Clone)]
pub struct TerminalHandle {sender:mpsc::Sender<Message>,closed:Arc<AtomicBool>,task:Arc<Mutex<Option<tokio::task::JoinHandle<Result<(),RuntimeError>>>>>,stopped:Arc<Mutex<Option<Result<(),RuntimeError>>>>}
impl TerminalHandle {
    pub async fn command(&self,command:RuntimeCommand)->Result<RuntimeReply,RuntimeError> {
        let kind=serde_json::to_value(command.kind).unwrap().as_str().unwrap().to_string();
        self.control(command.operation_id,kind,command.payload).await
    }
    pub async fn control(&self,id:String,kind:String,payload:Value)->Result<RuntimeReply,RuntimeError> {
        if self.closed.load(Ordering::Acquire) {return Err(error("runtime_stopping"));}
        let (response,receive)=oneshot::channel(); self.sender.send(Message::Command {id,kind,payload,response}).await.map_err(|_|error("runtime_closed"))?;
        receive.await.map_err(|_|error("runtime_closed"))?
    }
    pub async fn queue(&self,id:String,action:Action)->Result<Value,RuntimeError> {
        if self.closed.load(Ordering::Acquire) {return Err(error("runtime_stopping"));}
        let (response,receive)=oneshot::channel(); self.sender.send(Message::Queue {id,action,response}).await.map_err(|_|error("runtime_closed"))?;
        receive.await.map_err(|_|error("runtime_closed"))?
    }
    async fn query(&self,kind:&str)->Result<Value,RuntimeError> {
        if self.closed.load(Ordering::Acquire) {return Err(error("runtime_stopping"));}
        let (send,receive)=oneshot::channel(); let message=match kind {"drain"=>Message::Drain(send),"confirm"=>Message::Confirm(send),_=>Message::Snapshot(send)};
        self.sender.send(message).await.map_err(|_|error("runtime_closed"))?; receive.await.map_err(|_|error("runtime_closed"))?
    }
    pub async fn snapshot(&self)->Result<Value,RuntimeError> {self.query("snapshot").await}
    pub async fn drain(&self)->Result<Value,RuntimeError> {self.query("drain").await}
    pub async fn confirm(&self)->Result<Value,RuntimeError> {self.query("confirm").await}
    pub async fn ensure_projection(&self)->Result<Value,RuntimeError> {self.queue(format!("terminal-projection:{}",sample().epoch_s),Action::EnsureProjection).await}
    pub async fn stop(&self)->Result<(),RuntimeError> {
        let mut stopped=self.stopped.lock().await; if let Some(result)=&*stopped {return result.clone();}
        self.closed.store(true,Ordering::Release);
        let (send,receive)=oneshot::channel();
        let mut result=match self.sender.send(Message::Stop(send)).await {Ok(())=>receive.await.map_err(|_|error("runtime_closed")).and_then(|r|r),Err(_)=>Err(error("runtime_closed"))};
        if let Some(task)=self.task.lock().await.take() {let joined=task.await.map_err(|_|error("runtime_panic")).and_then(|r|r); if joined.is_err(){result=joined;}}
        *stopped=Some(result.clone()); result
    }
}

struct Services {
    target:TerminalTarget,queue:Arc<QueueActor>,policy:PolicyClient,root:String,attempt:String,text:String,sequence:Arc<AtomicU64>,
}
impl Services {
    async fn action(&self,label:&str,action:Action)->Result<Value,RuntimeError> {
        let seq=self.sequence.fetch_add(1,Ordering::Relaxed);
        self.queue.exec(self.target.generation,&format!("terminal:{label}:{seq}"),sample(),action).await.map_err(|_|error("queue_io"))
    }
    async fn call(&self,kind:&str,request_id:RequestId,payload:Value)->Result<Value,RuntimeError> {
        let phase=format!("terminal-policy:{}:{}:{}",self.target.generation,self.attempt,self.sequence.fetch_add(1,Ordering::Relaxed));
        self.action("policy-prepare",Action::Prepare {id:phase.clone(),payload:json!({"kind":kind,"request_id":request_id,"payload":payload}),entry_id:None}).await?;
        self.action("policy-dispatch",Action::BeginDispatch {id:phase.clone(),wire_id:phase.clone()}).await?;
        let result=self.policy.run_for(&self.target.key,self.target.generation,kind,&request_id,payload,&phase).await;
        self.action("policy-finish",Action::Finish {id:phase,status:if result.is_ok(){Status::Accepted}else{Status::Unknown},result:result.clone().unwrap_or_else(|e|json!({"error_code":e.code}))}).await?;
        result
    }
}
impl TerminalServices for Services {
    fn facts<'a>(&'a self,binding:&'a TerminalBinding)->ServiceFuture<'a,InputFacts> {Box::pin(async move {
        let value=self.call("terminal_facts",RequestId::String(self.root.clone()),json!({"binding":binding,"operation_id":self.root,"text":self.text})).await.map_err(|_|ServiceError("terminal_facts"))?;
        serde_json::from_value(value).map_err(|_|ServiceError("terminal_facts_shape"))
    })}
    fn publish<'a>(&'a self,binding:&'a TerminalBinding,request:PluginRequest)->ServiceFuture<'a,PluginReply> {Box::pin(async move {
        let value=self.call("terminal_publish",RequestId::String(self.root.clone()),json!({"binding":binding,"operation_id":self.root,"publication":request,"generation":self.target.generation})).await.map_err(|_|ServiceError("terminal_publish"))?;
        serde_json::from_value(value).map_err(|_|ServiceError("terminal_publish_shape"))
    })}
}

struct Executor {
    target:TerminalTarget,queue:Arc<QueueActor>,policy:PolicyClient,options:TerminalOptions,
    events:broadcast::Sender<RuntimeEvent>,revision:Arc<AtomicU64>,sequence:Arc<AtomicU64>,receipt:ReceiptIndex,deliverable:bool,last_error:Option<String>,
}
pub struct TerminalActor;
impl TerminalActor {
    pub fn spawn(target:TerminalTarget,queue:QueueActor,policy:PolicyClient,options:TerminalOptions,events:broadcast::Sender<RuntimeEvent>,revision:Arc<AtomicU64>)->TerminalHandle {
        let previous=queue.initial_state().operations.keys().filter(|id|id.starts_with("call::terminal:") || id.starts_with("terminal-policy:") || id.starts_with("queue:"))
            .filter_map(|id|id.rsplit(':').next()?.parse::<u64>().ok()).max().unwrap_or(0);
        let sequence=Arc::new(AtomicU64::new(previous.max(queue.initial_state().operations.len() as u64).saturating_add(1)));
        let receipt=ReceiptIndex::new("claude",&target.binding.conversation);
        let (sender,receiver)=mpsc::channel(64); let closed=Arc::new(AtomicBool::new(false));
        let executor=Executor {target,queue:Arc::new(queue),policy,options,events,revision,sequence,receipt,deliverable:false,last_error:None};
        let task=tokio::spawn(executor.run(receiver,closed.clone()));
        TerminalHandle {sender,closed,task:Arc::new(Mutex::new(Some(task))),stopped:Arc::new(Mutex::new(None))}
    }
}
impl Executor {
    fn cleared(&self,state:&super::queue::State)->bool {
        state.runtime_state["clear_barrier"]["generation"]==self.target.generation
            && state.runtime_state["clear_barrier"]["conversation"]==self.target.binding.conversation
    }
    fn services(&self,root:&str,attempt:&str,text:&str)->Arc<Services> {Arc::new(Services {target:self.target.clone(),queue:self.queue.clone(),policy:self.policy.clone(),root:root.into(),attempt:attempt.into(),text:text.into(),sequence:self.sequence.clone()})}
    async fn action(&self,action:Action)->Result<Value,RuntimeError> {self.services("maintenance","maintenance","").action("queue",action).await}
    async fn snapshot(&self)->Result<Value,RuntimeError> {
        let state=self.queue.snapshot().await.map_err(|_|error("queue_io"))?;
        if state.generation!=self.target.generation{return Err(error("runtime_generation"));}
        Ok(json!({"key":self.target.key,"generation":self.target.generation,"revision":self.revision.load(Ordering::Acquire),
            "view":{"terminal":true,"conversation":self.target.binding.conversation,"deliverable":self.deliverable && !self.cleared(&state),"preserve_binding":state.runtime_state["preserve_binding"],"clear_barrier":state.runtime_state["clear_barrier"]},"channels":{},"error":self.last_error}))
    }
    async fn publish(&self)->Result<(),RuntimeError> {
        self.revision.fetch_add(1,Ordering::AcqRel);
        let data=self.snapshot().await?;
        let _=self.events.send(RuntimeEvent {key:self.target.key.clone(),generation:self.target.generation,revision:self.revision.load(Ordering::Acquire),channel:"snapshot".into(),data}); Ok(())
    }
    async fn enter_error(&mut self,failure:RuntimeError)->Result<(),RuntimeError> {
        if self.last_error.as_deref()==Some(failure.code.as_str()){return Ok(());}
        tracing::warn!(key=%self.target.key,session=%self.target.name,code=%failure.code,reason=%failure.message,"entrada terminal entrou em erro");
        self.last_error=Some(failure.code.clone()); self.deliverable=false;
        self.publish().await?;
        let revision=self.revision.fetch_add(1,Ordering::AcqRel)+1;
        let _=self.events.send(RuntimeEvent {key:self.target.key.clone(),generation:self.target.generation,revision,channel:"problem".into(),
            data:json!({"error_code":failure.code,"message":failure.message})}); Ok(())
    }
    async fn clear_maintenance_error(&mut self,code:&str)->Result<(),RuntimeError> {
        if self.last_error.as_deref()==Some(code) {
            self.last_error=None; self.publish().await?;
        }
        Ok(())
    }
    async fn run(mut self,mut receiver:mpsc::Receiver<Message>,closed:Arc<AtomicBool>)->Result<(),RuntimeError> {
        self.action(Action::Recover).await?; self.action(Action::EnsureProjection).await?;
        let recovered=self.queue.snapshot().await.map_err(|_|error("queue_io"))?;
        if let Some(clear)=recovered.operations.values().find(|op|op.payload["kind"]=="input" && op.payload["payload"]["text"].as_str().is_some_and(is_clear)
            && matches!(op.status,Status::Accepted|Status::Unknown|Status::Confirmed)
            && op.wire_attempts.keys().any(|wire|wire.starts_with(&format!("terminal:{}:",self.target.generation)))) {
            let mut state=recovered.runtime_state.clone(); state["preserve_binding"]=json!(true);
            state["clear_barrier"]=json!({"generation":self.target.generation,"conversation":self.target.binding.conversation,"operation_id":clear.id});
            self.action(Action::SetRuntimeState {state}).await?;
        }
        let mut timer=tokio::time::interval(self.options.tick.max(Duration::from_millis(1))); timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {biased;
                message=receiver.recv()=>match message {
                    Some(Message::Command {id,kind,payload,response})=>{let result=self.execute(&id,&kind,payload,None).await; let _=response.send(result);},
                    Some(Message::Queue {id,action,response})=>{
                        let result=match action {
                            Action::Finish {id,status,result}=>self.native_receipt(&id,status,result).await,
                            Action::Claim {..}|Action::SetDelivered {value:false,..}|Action::BumpAttempts {..}|Action::Reconcile {..}|Action::ReplaceRows {..}|Action::Prepare {..}|Action::BeginDispatch {..}|Action::BindDispatch {..}|Action::Recover|Action::Confirm {..}|Action::ConfirmOccurrence {..}|Action::SetRuntimeState {..}|Action::LateRpcResolution {..}=>Err(error("terminal_queue_action")),
                            action=>self.queue.exec(self.target.generation,&id,sample(),action).await.map_err(|_|error("queue_io")),
                        };
                        if result.is_ok(){self.publish().await?;} let _=response.send(result);
                    },
                    Some(Message::Snapshot(response))=>{let _=response.send(self.snapshot().await);},
                    Some(Message::Drain(response))=>{let result=self.drain_once(None).await;
                        if let Err(failure)=&result {self.enter_error(failure.clone()).await?;}else{self.clear_maintenance_error("terminal_facts").await?;}
                        let _=response.send(result);},
                    Some(Message::Confirm(response))=>{let result=self.confirm_rows().await;
                        if let Err(failure)=&result {self.enter_error(failure.clone()).await?;}else{self.clear_maintenance_error("receipt_scan").await?;}
                        let _=response.send(result);},
                    Some(Message::Stop(response))=>{
                        closed.store(true,Ordering::Release);
                        let queue=Arc::try_unwrap(self.queue).map_err(|_|error("terminal_lease_inflight"))?;
                        let result=queue.shutdown().await.map_err(|_|error("queue_stop")); let _=response.send(result.clone()); return result;
                    },
                    None=>{let queue=Arc::try_unwrap(self.queue).map_err(|_|error("terminal_lease_inflight"))?;return queue.shutdown().await.map_err(|_|error("queue_stop"));}
                },
                _=timer.tick(),if !closed.load(Ordering::Acquire) && self.last_error.is_none()=>{
                    let result=async {self.confirm_rows().await?;self.drain_once(None).await?;Ok::<_,RuntimeError>(())}.await;
                    if let Err(failure)=result {self.enter_error(failure).await?;}
                }
            }
        }
    }
    async fn execute(&mut self,id:&str,kind:&str,payload:Value,entry:Option<String>)->Result<RuntimeReply,RuntimeError> {
        if id.is_empty() || id.starts_with("call::") || id.starts_with("terminal-") {return Err(error("operation_id"));}
        validate(kind,&payload)?;
        let requested=json!({"operation_id":id,"kind":kind,"payload":payload});
        let mut original=requested.clone();
        if kind=="input" && !payload["text"].as_str().unwrap_or("").trim_start().starts_with('/') {
            original["payload"]["_terminal_generation"]=json!(self.target.generation);
        }
        let state=self.queue.snapshot().await.map_err(|_|error("queue_io"))?;
        if state.generation!=self.target.generation{return Err(error("runtime_generation"));}
        if let Some(old)=state.operations.get(id) {
            if old.payload!=original && !(old.payload["payload"].get("_terminal_generation").is_none() && old.payload==requested) {return Err(error("operation_payload"));}
            if let Ok(stored)=serde_json::from_value::<RuntimeReply>(old.result.clone()){return Ok(stored);}
            if matches!(old.status,Status::Prepared|Status::Deferred) {
                return Ok(reply(id,Disposition::Deferred,json!({"code":"prepared_before_dispatch","queued":old.entry_id.is_some(),"cleanup":"not_needed"})));
            }
            return Ok(reply(id,Disposition::Unknown,json!({"code":"recovered_operation"})));
        }
        if self.cleared(&state){return Err(error("runtime_clear_barrier"));}
        let text=payload["text"].as_str().unwrap_or("");
        let prompt=kind=="input"; let slash=prompt && text.trim_start().starts_with('/');
        let row_id=if prompt && !slash {Some(entry.unwrap_or_else(||id.into()))}else{None};
        self.action(Action::Prepare {id:id.into(),payload:original,entry_id:row_id.clone()}).await?;
        if let Some(row)=&row_id {
            if !state.rows.iter().any(|r|r["id"]==*row) {
                self.action(Action::Append {text:text.into(),delivered:false,ts:None,pre_transcript:payload["pre_transcript"]==true,entry_id:Some(row.clone())}).await?;
            }
        }
        let root=row_id.as_deref().unwrap_or(id);
        let services=self.services(root,id,text);
        let driver=TerminalDriver::new(self.target.binding.clone(),services.clone(),self.options.io.clone(),self.options.limits.clone());
        let facts=if prompt {Some(services.facts(&self.target.binding).await)}else{None};
        let current=facts.as_ref().is_none_or(|result|result.as_ref().is_ok_and(|facts|facts.binding==self.target.binding));
        if let Some(facts)=&facts {self.deliverable=facts.as_ref().is_ok_and(|facts|current && facts.ready && facts.idle && !facts.open_question);}
        let mut result=if !current {reply(id,Disposition::Deferred,json!({"code":"terminal_facts","cleanup":"not_needed"}))}
            else if prompt && !slash && !facts.as_ref().is_some_and(|result|result.as_ref().is_ok_and(|f|f.ready && !f.open_question || f.native.is_some())) {reply(id,Disposition::Deferred,json!({"queued":true,"cleanup":"not_needed"}))}
            else {
                if row_id.is_some() {
                    let cursor=self.receipt.capture(&self.target.transcript).map_err(|_|error("receipt_cursor"))?;
                    self.action(Action::BindDispatch {id:id.into(),cursor:serde_json::to_value(cursor).unwrap()}).await?;
                }
                self.action(Action::BeginDispatch {id:id.into(),wire_id:format!("terminal:{}:{id}",self.target.generation)}).await?;
                let publication=format!("{}:{}:{id}",self.target.key,self.target.generation);
                let held=payload["request_id"].as_str().filter(|s|s.starts_with("perm:")||s.starts_with("ask:"));
                let plugin=if matches!(kind,"select"|"answer_questions") {
                    if let Some(request)=held {
                        Some(services.call("terminal_plugin_control",RequestId::String(request.into()),json!({"binding":self.target.binding,"operation_id":root,"control":kind,"payload":payload,"publication_id":publication,"generation":self.target.generation})).await)
                    }else{None}
                }else{None};
                match plugin {
                    Some(Ok(value)) if value["disposition"]!="unavailable"=> {
                        let disposition=match value["disposition"].as_str(){Some("accepted")=>Disposition::Accepted,Some("rejected")=>Disposition::Rejected,_=>Disposition::Unknown}; reply(id,disposition,value)
                    },
                    Some(Err(_))=>reply(id,Disposition::Unknown,json!({"code":"plugin_control_uncertain"})),
                    _=>delivery(id,match kind {
                        "input"=>driver.prompt(text,&publication).await,
                        "steer"|"steer_queue"=>driver.steer().await,
                        "key"|"navigation_key"=>driver.key(payload["key"].as_str().unwrap(),false).await,
                        "interactive_key"=>driver.key(payload["key"].as_str().unwrap(),true).await,
                        "terminal_input"=>driver.text(text).await,
                        "select"=>driver.select(payload["option"].as_u64().unwrap() as usize,payload["require_cursor"].as_bool().unwrap_or(true)).await,
                        "answer_questions"=>driver.answer(&serde_json::from_value::<Vec<QuestionAnswer>>(payload["answers"].clone()).unwrap()).await,
                        "submit_selected"=>driver.submit_selected().await,
                        "interrupt"=>driver.interrupt(payload["clear"].as_bool().unwrap_or(false)).await,
                        _=>unreachable!(),
                    })
                }
            };
        if slash && is_clear(text) && matches!(result.disposition,Disposition::Accepted|Disposition::Unknown) {
            let mut state=self.queue.snapshot().await.map_err(|_|error("queue_io"))?.runtime_state;
            state["preserve_binding"]=json!(true);
            state["clear_barrier"]=json!({"generation":self.target.generation,"conversation":self.target.binding.conversation,"operation_id":id});
            self.action(Action::SetRuntimeState {state}).await?;
            result.payload["preserve_binding"]=json!(true);
        }
        self.action(Action::Finish {id:id.into(),status:status(result.disposition),result:serde_json::to_value(&result).unwrap()}).await?;
        if matches!(result.disposition,Disposition::Deferred|Disposition::Rejected) {
            tracing::info!(key=%self.target.key,session=%self.target.name,code=%result.payload["code"].as_str().unwrap_or("terminal_not_executed"),
                reason="operação adiada ou recusada; resultado conservado no diário",stage=%result.payload["stage"].as_str().unwrap_or("plugin"),"resultado da entrada terminal");
        }
        if result.disposition==Disposition::Unknown {
            tracing::warn!(key=%self.target.key,session=%self.target.name,code=%result.payload["code"].as_str().unwrap_or("plugin_control_uncertain"),
                reason="a entrega não foi comprovada",stage=%result.payload["stage"].as_str().unwrap_or("plugin"),"entrega terminal incerta");
            self.enter_error(error("terminal_delivery_unknown")).await?;
        }else{self.last_error=None; self.publish().await?;}
        Ok(result)
    }
    async fn drain_once(&mut self,entry:Option<String>)->Result<Value,RuntimeError> {
        let state=self.queue.snapshot().await.map_err(|_|error("queue_io"))?;
        if self.cleared(&state){return Ok(json!({"drained":0,"preserve_binding":true}));}
        if !state.rows.iter().any(|r|r["delivered"]==false && entry.as_ref().is_none_or(|id|r["id"]==*id)) {return Ok(json!({"drained":0}));}
        let services=self.services("maintenance","maintenance","");
        let facts=services.facts(&self.target.binding).await.map_err(|_|error("terminal_facts"))?;
        self.deliverable=facts.binding==self.target.binding && facts.ready && facts.idle && !facts.open_question;
        if !self.deliverable || facts.binding!=self.target.binding {return Ok(json!({"drained":0}));}
        let rows=self.action(Action::Claim {min_ts:self.target.created,limit:Some(1),entry_id:entry}).await?;
        let Some(row)=rows.as_array().and_then(|r|r.first())else{return Ok(json!({"drained":0}));};
        let row_id=row["id"].as_str().ok_or_else(||error("queue_row"))?.to_string();
        let attempt=format!("queue:{}:{}",row_id,self.sequence.fetch_add(1,Ordering::Relaxed));
        let outcome=self.execute(&attempt,"input",json!({"text":row["text"],"pre_transcript":row["pre_transcript"]==true}),Some(row_id)).await?;
        Ok(json!({"drained":1,"reply":outcome}))
    }
    async fn confirm_rows(&mut self)->Result<Value,RuntimeError> {
        let state=self.queue.snapshot().await.map_err(|_|error("queue_io"))?;
        if !state.rows.iter().any(|r|r["delivered"]==true && r["confirmed"]!=true){return Ok(json!({"confirmed":0}));}
        self.receipt.scan(&self.target.transcript).map_err(|_|error("receipt_scan"))?;
        let mut count=0;
        for operation in state.operations.values().filter(|op|op.entry_id.is_some() && matches!(op.status,Status::Accepted|Status::Unknown|Status::Dispatching)) {
            let current=self.queue.snapshot().await.map_err(|_|error("queue_io"))?;
            let Some(row)=current.rows.iter().find(|r|r["id"].as_str()==operation.entry_id.as_deref() && r["confirmed"]!=true) else {continue;};
            let Ok(cursor)=serde_json::from_value::<DispatchCursor>(operation.dispatch_cursor.clone())else{continue;};
            if let Some(proof)=self.receipt.match_after(&cursor,row,&current.used_occurrences) {
                if self.action(Action::ConfirmOccurrence {id:operation.id.clone(),proof}).await?==true {count+=1;}
            }
        }
        if count>0{self.publish().await?;}Ok(json!({"confirmed":count}))
    }
    async fn native_receipt(&self,id:&str,receipt_status:Status,result:Value)->Result<Value,RuntimeError> {
        let native_status=result["payload"]["native_status"].as_str().ok_or_else(||error("native_receipt"))?;
        let disposition=match native_status {"delivered"|"released"|""=>Disposition::Accepted,"rejected"|"refused"=>Disposition::Rejected,_=>Disposition::Unknown};
        if result.as_object().is_none_or(|o|o.len()!=3) || result["operation_id"]!=id
            || result["disposition"]!=serde_json::to_value(disposition).unwrap()
            || result["payload"].as_object().is_none_or(|o|o.len()!=1)
            || serde_json::to_value(receipt_status).unwrap()!=serde_json::to_value(status(disposition)).unwrap() {return Err(error("native_receipt"));}
        let state=self.queue.snapshot().await.map_err(|_|error("queue_io"))?;
        if !state.rows.iter().any(|row|row["id"]==id) {return Err(error("native_receipt"));}
        let root=state.operations.get(id).filter(|op|op.payload["kind"]=="input" && op.entry_id.as_deref()==Some(id));
        let attempts:Vec<_>=state.operations.values().filter(|op|op.entry_id.as_deref()==Some(id) && op.payload["kind"]=="input"
            && (op.payload["payload"]["_terminal_generation"].as_u64()==Some(self.target.generation)
                || op.payload["payload"].get("_terminal_generation").is_none() && op.wire_attempts.keys().any(|wire|wire.starts_with(&format!("terminal:{}:",self.target.generation))))
            && op.result["payload"]["native"]==true && op.result["payload"]["message_id"].is_string()).collect();
        let source=attempts.last().ok_or_else(||error("native_receipt"))?;
        let mut payload=source.result["payload"].clone(); payload["native_status"]=json!(native_status);
        for attempt in &attempts {
            if attempt.status==Status::Confirmed {continue;}
            let resolved=reply(&attempt.id,disposition,payload.clone());
            self.action(Action::Finish {id:attempt.id.clone(),status:receipt_status,result:serde_json::to_value(resolved).unwrap()}).await?;
        }
        let resolved=reply(id,disposition,payload);
        if root.is_some_and(|root|root.status!=Status::Confirmed) {
            self.action(Action::Finish {id:id.into(),status:receipt_status,result:serde_json::to_value(&resolved).unwrap()}).await?;
        }
        Ok(serde_json::to_value(resolved).unwrap())
    }
}
fn is_clear(text:&str)->bool {text.split_whitespace().next()==Some("/clear")}
fn validate(kind:&str,payload:&Value)->Result<(),RuntimeError> {
    let fields:&[&str]=match kind {
        "input"=>&["text","pre_transcript"],"steer"=>&[],"key"|"navigation_key"|"interactive_key"=>&["key"],"terminal_input"=>&["text"],
        "select"=>&["option","require_cursor","request_id"],"answer_questions"=>&["answers","request_id"],"interrupt"=>&["clear"],"submit_selected"=>&[],"steer_queue"=>&["entry_id"],_=>return Err(error("terminal_control")),
    };
    if !payload.as_object().is_some_and(|p|p.keys().all(|k|fields.contains(&k.as_str()))){return Err(error("terminal_payload"));}
    let valid=match kind {
        "input"=>payload["text"].as_str().is_some_and(valid_text) && payload.get("pre_transcript").is_none_or(Value::is_boolean),
        "terminal_input"=>payload["text"].as_str().is_some_and(safe_characters),
        "key"|"navigation_key"|"interactive_key"=>payload["key"].is_string(),
        "select"=>payload["option"].as_u64().is_some_and(|v|v>0 && v<=100) && payload.get("require_cursor").is_none_or(Value::is_boolean) && payload.get("request_id").is_none_or(Value::is_string),
        "answer_questions"=>serde_json::from_value::<Vec<QuestionAnswer>>(payload["answers"].clone()).is_ok_and(|answers|!answers.is_empty() && answers.iter().all(|a|match a.kind {
            AnswerKind::Option=>!a.indices.is_empty() && a.indices.iter().all(|i|*i<100) && (a.multi||a.indices.len()==1) && !a.labels.is_empty(),
            AnswerKind::Text=>a.type_index.is_some_and(|i|i<100) && a.value.as_ref().is_some_and(|v|valid_text(v)&&!v.contains('\n')),
            AnswerKind::Chat=>a.chat_index.is_some_and(|i|i<100),
        })) && payload.get("request_id").is_none_or(Value::is_string),
        "interrupt"=>payload.get("clear").is_none_or(Value::is_boolean),"steer_queue"=>payload.get("entry_id").is_none_or(Value::is_string),_=>true,
    };
    if !valid{return Err(error("terminal_payload"));}Ok(())
}
