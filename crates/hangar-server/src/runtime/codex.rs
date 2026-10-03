use super::{LiveBuffer,protocol::*};
use hangar_api::state::StateEvent;
use serde_json::{Value,json};
use std::collections::{BTreeMap,BTreeSet};

struct Rpc {
    operation_id:String,
    method:String,
    params:Value,
    deadline:f64,
    timed_out:bool,
    continuation:Option<Value>,
    state_revision:u64,
    settings_revision:u64,
}

struct Wire {
    request_id:Option<RequestId>,
    server_request:Option<RequestId>,
    server_epoch:Option<u64>,
    final_result:bool,
}

#[derive(Default)]
struct AsyncQuestions {
    pending:Vec<(String,Value)>,
    seen:BTreeSet<String>,
    resolved:BTreeSet<String>,
    skipped:BTreeSet<String>,
    local_answers:BTreeMap<String,String>,
    echoes:BTreeMap<String,usize>,
    during_load:Option<Vec<Value>>,
}

impl AsyncQuestions {
    fn observe(&mut self,thread:&str,item:&Value) {
        let Some(id) = item["id"].as_str() else { return };
        let kind = item["type"].as_str().unwrap_or("");
        if self.seen.contains(id) || !(kind == "userMessage" || kind == "agentMessage" && item["delivery"] == "async"
            && item["questions"].as_array().is_some_and(|questions|!questions.is_empty())) { return; }
        self.seen.insert(id.into());
        if let Some(items) = self.during_load.as_mut() { items.push(item.clone()); }
        if kind == "agentMessage" {
            for (index,question) in item["questions"].as_array().unwrap().iter().enumerate() {
                let Some(title) = question["title"].as_str().filter(|s|!s.trim().is_empty()) else { continue };
                let request_id = format!("async:{thread}:{id}:{index}");
                if self.resolved.contains(&request_id) { continue; }
                let options:Vec<_> = question["options"].as_array().map(|options|options.iter().filter_map(Value::as_str)
                    .map(|label|json!({"label":label,"description":""})).collect()).unwrap_or_default();
                self.pending.push((request_id.clone(),json!({"provider":"codex","request_id":request_id,"is_async":true,
                    "questions":[{"id":"answer","header":(index+1).to_string(),"question":title,"multiSelect":false,
                    "isOther":true,"isSecret":false,"options":options}]})));
            }
        } else {
            let text = item["content"].as_array().map(|blocks|blocks.iter().filter(|b|b["type"] == "text")
                .filter_map(|b|b["text"].as_str()).collect::<String>()).unwrap_or_default();
            if let Some(count) = self.echoes.get_mut(&text).filter(|c|**c > 0) { *count -= 1; return; }
            let text = text.trim();
            if let Some(body) = text.strip_prefix("<send_user_message_question_reply>").and_then(|s|s.strip_suffix("</send_user_message_question_reply>")) {
                if let Ok(Value::Array(replies)) = serde_json::from_str::<Value>(body) {
                    for reply in replies {
                        if !reply["answer"].as_str().is_some_and(|s|!s.trim().is_empty()) { continue; }
                        if let Some(encoded) = reply["questionItemId"].as_str() {
                            if let Ok(Value::Array(identity)) = serde_json::from_str::<Value>(encoded) {
                                if identity.len() == 3 && identity[0] == "request_user_input_async" && identity[1].is_string() && identity[2].is_u64() {
                                    self.resolve(&format!("async:{thread}:{}:{}",identity[1].as_str().unwrap(),identity[2].as_u64().unwrap()));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn hydrate(&mut self,thread_id:&str,thread:&Value) {
        let mut restored = Self { skipped:self.skipped.clone(),resolved:self.skipped.clone(),..Self::default() };
        if let Some(turns) = thread["turns"].as_array() {
            for turn in turns { if let Some(items) = turn["items"].as_array() { for item in items { restored.observe(thread_id,item); } } }
        }
        for item in self.during_load.take().unwrap_or_default() { restored.observe(thread_id,&item); }
        for (id,text) in &self.local_answers { restored.record_answer(id,text); }
        *self = restored;
    }

    fn resolve(&mut self,id:&str) { self.pending.retain(|(key,_)|key != id); self.resolved.insert(id.into()); }
    fn record_answer(&mut self,id:&str,text:&str) {
        self.local_answers.insert(id.into(),text.into());
        if self.pending.iter().any(|(key,_)|key == id) { self.resolve(id); *self.echoes.entry(text.into()).or_default() += 1; }
    }
}

pub struct Engine {
    metadata:Value,
    generation:u64,
    clock:ClockSample,
    counter:u64,
    headless:bool,
    alive:bool,
    initialized:bool,
    ready:bool,
    reconnect:bool,
    thread_id:String,
    turn_id:Option<String>,
    in_progress:bool,
    state:StateEvent,
    state_revision:u64,
    settings_revision:u64,
    model:Option<String>,
    effort:Option<String>,
    mode:Option<String>,
    permission_mode:String,
    token_usage:Value,
    rate_limits:Value,
    preview:LiveBuffer,
    response_started:bool,
    compacting:bool,
    rpc:BTreeMap<RequestId,Rpc>,
    server_requests:Vec<(RequestId,Value)>,
    request_epochs:BTreeMap<RequestId,u64>,
    answering:BTreeSet<RequestId>,
    wires:BTreeMap<String,Wire>,
    policies:BTreeMap<RequestId,String>,
    last_format_request:Option<RequestId>,
    async_questions:AsyncQuestions,
}

fn error(message:&str) -> RuntimeError { RuntimeError::new("codex_command",message) }
fn string(value:&Value) -> Option<String> { value.as_str().map(str::to_owned) }
fn approval(mode:&str) -> &str { if mode == "Full Access" { "never" } else { "on-request" } }
fn sandbox(mode:&str) -> &str { match mode { "Ask for approval"=>"read-only","Approve for me"=>"workspace-write",_=>"danger-full-access" } }

impl Engine {
    pub fn new(metadata:Value,generation:u64,clock:ClockSample) -> Self {
        let mut async_questions = AsyncQuestions { during_load:Some(Vec::new()),..AsyncQuestions::default() };
        if let Some(skipped) = metadata["skipped_async_questions"].as_array() {
            async_questions.skipped = skipped.iter().filter_map(Value::as_str).map(str::to_owned).collect();
            async_questions.resolved = async_questions.skipped.clone();
        }
        Self { generation,clock,counter:metadata["runtime_counter"].as_u64().unwrap_or(0),headless:metadata["headless"] != false,
            alive:true,initialized:metadata["initialized"] == true,ready:metadata["ready"] == true,reconnect:false,
            thread_id:metadata["thread_id"].as_str().unwrap_or("").into(),turn_id:None,in_progress:false,
            state:StateEvent { session:metadata["name"].as_str().unwrap_or("").into(),state:"idle".into(),headless:true,
                status_line:string(&metadata["status_line"]),..StateEvent::default() },state_revision:metadata["state_revision"].as_u64().unwrap_or(0),settings_revision:metadata["settings_revision"].as_u64().unwrap_or(0),
            model:string(&metadata["model"]),effort:string(&metadata["effort"]),mode:string(&metadata["mode"]),
            permission_mode:metadata["permission_mode"].as_str().unwrap_or("Full Access").into(),token_usage:Value::Null,rate_limits:Value::Null,
            preview:LiveBuffer::default(),response_started:false,compacting:false,rpc:BTreeMap::new(),server_requests:Vec::new(),
            request_epochs:BTreeMap::new(),answering:BTreeSet::new(),wires:BTreeMap::new(),policies:BTreeMap::new(),last_format_request:None,async_questions,metadata }
    }

    pub fn view(&self) -> Value {
        let mut state = self.state.clone();
        let question = self.blocking_question().or_else(||self.async_questions.pending.first().map(|(_,q)|q.clone()));
        let pending_approval = self.server_requests.iter().find(|(_,r)|["item/commandExecution/requestApproval","item/fileChange/requestApproval"].contains(&r["method"].as_str().unwrap_or("")));
        state.state = if !self.alive { "dead" } else if self.blocking_question().is_some() || pending_approval.is_some()
            || question.is_some() && !self.in_progress { "awaiting_input" } else if self.in_progress { "working" } else { "idle" }.into();
        state.codex_question = question.and_then(|q|q.as_object().cloned());
        state.codex_mode = self.mode.clone();
        state.label = self.compacting.then(||"Compactando…".into());
        state.question = None; state.options = None;
        if let Some((_,request)) = pending_approval {
            let params = &request["params"];
            let target = if request["method"] == "item/fileChange/requestApproval" { format!("Editar arquivos{}",
                params["grantRoot"].as_str().map_or(String::new(),|p|format!(" em {p}"))) }
                else { format!("Rodar `{}`{}",params["command"].as_str().unwrap_or("?"),params["cwd"].as_str().map_or(String::new(),|p|format!(" em {p}"))) };
            state.question = Some(format!("{target}?{}",params["reason"].as_str().map_or(String::new(),|r|format!(" {r}"))));
            state.options = Some(vec!["Permitir".into(),"Negar".into(),"Sempre permitir".into()]);
        }
        serde_json::to_value(state).unwrap()
    }

    pub fn control_view(&self) -> Value {
        json!({"alive":self.alive,"initialized":self.initialized,"ready":self.ready,"in_progress":self.in_progress,
            "thread_id":self.thread_id,"turn_id":self.turn_id,"model":self.model,"effort":self.effort,"mode":self.mode,
            "permission_mode":self.permission_mode,"token_usage":self.token_usage,"rate_limits":self.rate_limits,
            "runtime_counter":self.counter,"state_revision":self.state_revision,"settings_revision":self.settings_revision,"deliverable":self.deliverable(),"pending":self.server_requests.iter()
                .map(|(id,request)|json!({"request_id":id,"request":request})).collect::<Vec<_>>(),
            "async_questions":self.async_questions.pending,"async_local_answers":self.async_questions.local_answers,
            "skipped_async_questions":self.async_questions.skipped})
    }

    fn blocking_question(&self) -> Option<Value> {
        let (id,request) = self.server_requests.iter().find(|(_,r)|r["method"] == "item/tool/requestUserInput")?;
        let questions:Vec<_> = request["params"]["questions"].as_array()?.iter().map(|q|json!({
            "id":q["id"],"header":q["header"],"question":q["question"],"multiSelect":false,
            "isOther":q["isOther"].as_bool().unwrap_or(false),"isSecret":q["isSecret"].as_bool().unwrap_or(false),
            "options":q.get("options").cloned().unwrap_or_else(||json!([]))})).collect();
        Some(json!({"provider":"codex","request_id":id,"questions":questions}))
    }

    fn deliverable(&self) -> bool { self.alive && self.ready && !self.in_progress && self.server_requests.is_empty()
        && !self.rpc.values().any(|rpc|matches!(rpc.method.as_str(),"turn/start" | "turn/steer" | "thread/compact/start")) }
    fn idle(&self) -> bool { self.deliverable() && self.rpc.is_empty() && self.answering.is_empty() && self.async_questions.pending.is_empty() }

    fn policy(&mut self,kind:&str,payload:Value,effects:&mut Vec<Effect>) {
        self.counter += 1;
        let request_id = RequestId::String(format!("policy:{}:{}",self.generation,self.counter));
        self.policies.insert(request_id.clone(),kind.into());
        if kind == "format_status" { self.last_format_request = Some(request_id.clone()); }
        effects.push(Effect::Policy { kind:kind.into(),request_id,payload });
    }

    fn changed(&mut self,effects:&mut Vec<Effect>,format:bool) {
        effects.push(Effect::StateChanged);
        if format { self.policy("format_status",json!({"model":self.model,"effort":self.effort,"token_usage":self.token_usage,"rate_limits":self.rate_limits}),effects); }
    }

    fn rpc(&mut self,operation_id:String,method:&str,params:Value,continuation:Option<Value>,effects:&mut Vec<Effect>) {
        self.counter += 1;
        let request_id = RequestId::String(format!("hangar:{}:{}",self.generation,self.counter));
        if method == "thread/read" && params["includeTurns"] == true { self.async_questions.during_load = Some(Vec::new()); }
        self.rpc.insert(request_id.clone(),Rpc { operation_id:operation_id.clone(),method:method.into(),params:params.clone(),
            deadline:self.clock.monotonic_s+30.0,timed_out:false,continuation,state_revision:self.state_revision,settings_revision:self.settings_revision });
        self.wires.insert(operation_id.clone(),Wire { request_id:Some(request_id.clone()),server_request:None,server_epoch:None,final_result:false });
        effects.push(Effect::Write { operation_id:Some(operation_id),frame:json!({"jsonrpc":"2.0","id":request_id,"method":method,"params":params}) });
    }

    pub fn restore_rpc(&mut self,operation_id:String,frame:&Value,state_revision:u64,settings_revision:u64) {
        let Ok(request_id) = serde_json::from_value::<RequestId>(frame["id"].clone()) else { return };
        let Some(method) = frame["method"].as_str() else {
            if frame.get("result").is_some() || frame.get("error").is_some() {
                self.wires.insert(operation_id,Wire { request_id:None,server_request:Some(request_id),server_epoch:None,final_result:false });
            }
            return;
        };
        if self.rpc.contains_key(&request_id) { return; }
        self.rpc.insert(request_id.clone(),Rpc { operation_id:operation_id.clone(),method:method.into(),params:frame["params"].clone(),
            deadline:self.clock.monotonic_s,timed_out:true,continuation:None,state_revision,settings_revision });
        self.wires.insert(operation_id,Wire { request_id:Some(request_id),server_request:None,server_epoch:None,final_result:false });
    }

    pub fn bootstrap(&mut self,reconnect:bool,operation_id:String) -> Result<Vec<Effect>,RuntimeError> {
        if !self.headless { return Err(error("Codex com terminal conserva o adapter existente")); }
        self.reconnect = reconnect;
        self.ready = false;
        let mut effects = Vec::new();
        self.rpc(format!("{operation_id}:initialize"),"initialize",json!({"clientInfo":{"name":"hangar","title":null,"version":"0.1.0"},
            "capabilities":{"experimentalApi":true}}),Some(json!({"kind":"bootstrap","parent":operation_id})),&mut effects);
        self.changed(&mut effects,true);
        Ok(effects)
    }

    pub fn hydrate(&mut self,snapshot:CanoSnapshot) -> Result<Vec<Effect>,RuntimeError> {
        let mut effects = Vec::new();
        self.alive = snapshot.saiu.is_none();
        for raw in snapshot.pendentes {
            effects.extend(self.apply(EngineInput::Line(serde_json::from_str(&raw).map_err(|_|error("pedido do snapshot inválido"))?),self.clock)?);
        }
        let prefix = &snapshot.inflight["codex"][&self.thread_id];
        if prefix.is_object() {
            self.turn_id = string(&prefix["turnId"]);
            self.in_progress = self.turn_id.is_some() || prefix["text"].as_str().is_some_and(|s|!s.is_empty());
            if prefix["complete"] == false { self.preview.block(); }
            else if let Some(text) = self.preview.append(prefix["text"].as_str().unwrap_or(""),self.clock.monotonic_s) { self.publish(text,&mut effects); }
        }
        self.changed(&mut effects,true);
        Ok(effects)
    }

    fn publish(&self,text:String,effects:&mut Vec<Effect>) {
        effects.push(Effect::Publish { channel:"preview".into(),data:json!({"session":self.state.session,"text":text,"md":true,"full":true,"vivo":true}) });
    }

    fn clear_preview(&mut self,effects:&mut Vec<Effect>) {
        if let Some(text) = self.preview.clear() { self.publish(text,effects); }
    }

    pub fn command(&mut self,command:RuntimeCommand,clock:ClockSample) -> Result<Vec<Effect>,RuntimeError> {
        self.clock = clock;
        if !self.headless || !self.alive { return Err(error("runtime sem terminal indisponível")); }
        let id = command.operation_id;
        let payload = command.payload;
        let mut effects = Vec::new();
        match command.kind {
            OperationKind::Input => {
                if !self.deliverable() { return Ok(vec![Effect::Reply { operation_id:id,disposition:Disposition::Deferred,payload:json!({}) }]); }
                let text = payload["text"].as_str().ok_or_else(||error("mensagem sem texto"))?;
                self.rpc(id,"turn/start",json!({"threadId":self.thread_id,"approvalPolicy":approval(&self.permission_mode),
                    "input":payload.get("input").cloned().unwrap_or_else(||json!([{"type":"text","text":text}]))}),None,&mut effects);
            }
            OperationKind::Steer => {
                let text = payload["text"].as_str().filter(|s|!s.trim().is_empty()).ok_or_else(||error("a orientação não pode estar vazia"))?;
                let turn = payload["turn_id"].as_str().or(self.turn_id.as_deref()).ok_or_else(||error("não há turno em andamento para orientar"))?;
                if !self.in_progress { return Err(error("não há turno em andamento para orientar")); }
                self.rpc(id,"turn/steer",json!({"threadId":self.thread_id,"expectedTurnId":turn,
                    "input":payload.get("input").cloned().unwrap_or_else(||json!([{"type":"text","text":text}]))}),None,&mut effects);
            }
            OperationKind::Interrupt => {
                if let Some(turn) = &self.turn_id {
                    self.rpc(id,"turn/interrupt",json!({"threadId":self.thread_id,"turnId":turn}),None,&mut effects);
                } else {
                    self.rpc(format!("{id}:read"),"thread/read",json!({"threadId":self.thread_id,"includeTurns":true}),
                        Some(json!({"kind":"interrupt","parent":id})),&mut effects);
                }
            }
            OperationKind::Compact => {
                if !self.idle() { return Err(error("espere o Codex terminar e responda às perguntas antes de compactar")); }
                self.rpc(id,"thread/compact/start",json!({"threadId":self.thread_id}),None,&mut effects);
            }
            OperationKind::ListModels => self.rpc(id,"model/list",json!({}),None,&mut effects),
            OperationKind::ListSkills => self.rpc(id,"skills/list",json!({"cwds":[self.metadata["cwd"]]}),None,&mut effects),
            OperationKind::ReadRateLimits => self.rpc(id,"account/rateLimits/read",json!({}),None,&mut effects),
            OperationKind::ReadSettings => self.rpc(id,"thread/read",json!({"threadId":self.thread_id,
                "includeTurns":payload["include_turns"].as_bool().unwrap_or(false)}),None,&mut effects),
            OperationKind::SetModel | OperationKind::SetEffort => self.rpc(id,"thread/settings/update",json!({"threadId":self.thread_id,
                "model":payload.get("model").cloned().unwrap_or_else(||json!(self.model)),"effort":payload.get("effort").cloned().unwrap_or_else(||json!(self.effort))}),None,&mut effects),
            OperationKind::SetMode => {
                let mode = payload["mode"].as_str().filter(|mode|["default","plan"].contains(mode)).ok_or_else(||error("modo Codex inválido"))?;
                self.rpc(format!("{id}:settings"),"thread/read",json!({"threadId":self.thread_id,"includeTurns":false}),
                    Some(json!({"kind":"set_mode","parent":id,"mode":mode})),&mut effects);
            }
            OperationKind::Select => {
                let (request_id,_) = self.server_requests.iter().find(|(id,request)|!self.answering.contains(id)
                    && ["item/commandExecution/requestApproval","item/fileChange/requestApproval"].contains(&request["method"].as_str().unwrap_or("")))
                    .cloned().ok_or_else(||error("nenhuma aprovação pendente"))?;
                let option = payload["option"].as_u64().ok_or_else(||error("opção inválida"))?;
                let decision = match option { 1=>"accept",2=>"decline",3=>"acceptForSession",_=>return Err(error("opção inválida")) };
                self.answer(id,request_id,json!({"decision":decision}),None,&mut effects)?;
            }
            OperationKind::AnswerQuestions => {
                let request_id:RequestId = serde_json::from_value(payload["request_id"].clone()).map_err(|_|error("ID da resposta inválido"))?;
                if let RequestId::String(request) = &request_id {
                    if request.starts_with("async:") {
                        let question = self.async_questions.pending.iter().find(|(key,_)|key == request).map(|(_,q)|q.clone()).ok_or_else(||error("a pergunta já foi respondida ou pertence a outra conversa"))?;
                        let response = question_response(&question,&payload["answers"])?;
                        let answer = response["answers"]["answer"]["answers"][0].as_str().unwrap_or("");
                        let text = format!("> {}\n\n{answer}",question["questions"][0]["question"].as_str().unwrap_or(""));
                        self.rpc(id,"turn/start",json!({"threadId":self.thread_id,"input":[{"type":"text","text":text}]}),
                            Some(json!({"kind":"async_answer","request_id":request,"text":text})),&mut effects);
                        self.changed(&mut effects,true);
                        return Ok(effects);
                    }
                }
                let question = self.blocking_question().ok_or_else(||error("a pergunta já foi respondida ou cancelada"))?;
                if question["request_id"] != serde_json::to_value(&request_id).unwrap() { return Err(error("a pergunta mudou")); }
                let response = question_response(&question,&payload["answers"])?;
                self.answer(id,request_id,response,None,&mut effects)?;
            }
            OperationKind::SkipQuestion => {
                let request = payload["request_id"].as_str().ok_or_else(||error("ID da pergunta inválido"))?;
                if !self.async_questions.pending.iter().any(|(id,_)|id == request) { return Err(error("a pergunta já foi respondida ou pertence a outra conversa")); }
                self.async_questions.skipped.insert(request.into()); self.async_questions.resolve(request);
                self.policy("session.patch_meta",json!({"skipped_async_questions":self.async_questions.skipped}),&mut effects);
                effects.push(Effect::Reply { operation_id:id,disposition:Disposition::Accepted,payload:json!({}) });
            }
            OperationKind::SetPermissionMode | OperationKind::Restart | OperationKind::OpenTerminal | OperationKind::Reload => {
                if !self.idle() { return Err(error("aguarde a sessão ficar ociosa antes de mudar o sandbox ou o modo")); }
                return Err(RuntimeError::new("lifecycle_required","operação exige a barreira de lifecycle sob a mesma posse"));
            }
            OperationKind::Cwd => effects.push(Effect::Reply { operation_id:id,disposition:Disposition::Accepted,payload:self.metadata["cwd"].clone() }),
            OperationKind::Detach => effects.push(Effect::Stop { reason:"detach".into() }),
            _ => return Err(error("operação não suportada pelo Codex")),
        }
        self.changed(&mut effects,true);
        Ok(effects)
    }

    fn answer(&mut self,operation_id:String,request_id:RequestId,result:Value,rpc_error:Option<Value>,effects:&mut Vec<Effect>) -> Result<(),RuntimeError> {
        if !self.answering.insert(request_id.clone()) { return Err(error("pedido já está sendo respondido")); }
        self.wires.insert(operation_id.clone(),Wire { request_id:None,server_request:Some(request_id.clone()),
            server_epoch:self.request_epochs.get(&request_id).copied(),final_result:false });
        let frame = if let Some(error) = rpc_error { json!({"jsonrpc":"2.0","id":request_id,"error":error}) }
            else { json!({"jsonrpc":"2.0","id":request_id,"result":result}) };
        effects.push(Effect::Write { operation_id:Some(operation_id),frame });
        Ok(())
    }

    pub fn apply(&mut self,input:EngineInput,clock:ClockSample) -> Result<Vec<Effect>,RuntimeError> {
        self.clock = clock;
        let mut effects = Vec::new();
        match input {
            EngineInput::Line(line) => {
                if line.get("method").is_some() { self.notification(line,&mut effects)?; }
                else if line.get("id").is_some() { self.reply(line,&mut effects)?; }
                else if line["type"] == "cano_saiu" {
                    self.alive = false; self.in_progress = false; self.ready = false; self.clear_preview(&mut effects);
                    self.state.problema = Some("headless_caiu".into()); self.changed(&mut effects,true);
                }
            }
            EngineInput::WriteAck { operation_id,outcome } => {
                if let Some(wire) = self.wires.get_mut(&operation_id) {
                    if wire.final_result { return Ok(effects); }
                    if wire.request_id.is_some() && outcome == WriteOutcome::Written { return Ok(effects); }
                    let server = wire.server_request.clone();
                    let server_epoch = wire.server_epoch;
                    let request_id = wire.request_id.clone();
                    wire.final_result = outcome != WriteOutcome::Unknown;
                    if let Some(id) = server {
                        if outcome == WriteOutcome::Written && self.request_epochs.get(&id).copied() == server_epoch {
                            self.server_requests.retain(|(key,_)|key != &id); self.answering.remove(&id); self.request_epochs.remove(&id);
                        }
                    }
                    if outcome == WriteOutcome::NotWritten {
                        if let Some(id) = request_id { self.rpc.remove(&id); }
                    }
                    effects.push(Effect::Reply { operation_id,disposition:match outcome { WriteOutcome::Written=>Disposition::Accepted,
                        WriteOutcome::NotWritten=>Disposition::Rejected,WriteOutcome::Unknown=>Disposition::Unknown },payload:json!({"write_outcome":outcome}) });
                    self.changed(&mut effects,true);
                }
            }
            EngineInput::Tick => {
                if let Some(text) = self.preview.tick(clock.monotonic_s) { self.publish(text,&mut effects); }
                for rpc in self.rpc.values_mut() {
                    if !rpc.timed_out && clock.monotonic_s >= rpc.deadline {
                        rpc.timed_out = true;
                        effects.push(Effect::Reply { operation_id:rpc.operation_id.clone(),disposition:Disposition::Unknown,payload:json!({"error":"RPC sem resposta"}) });
                        if let Some(parent) = rpc.continuation.as_ref().and_then(|next|next["parent"].as_str()) {
                            effects.push(Effect::Reply { operation_id:parent.into(),disposition:Disposition::Unknown,payload:json!({"error":"preparação sem resposta"}) });
                        }
                    }
                }
            }
            EngineInput::PolicyResult { request_id,payload } => {
                if let Some(kind) = self.policies.remove(&request_id) {
                    if kind == "format_status" && self.last_format_request.as_ref() == Some(&request_id) {
                        self.state.status_line = string(&payload["status_line"]); self.changed(&mut effects,false);
                    }
                }
            }
        }
        Ok(effects)
    }

    fn reply(&mut self,line:Value,effects:&mut Vec<Effect>) -> Result<(),RuntimeError> {
        let id:RequestId = serde_json::from_value(line["id"].clone()).map_err(|_|error("ID RPC inválido"))?;
        let Some(rpc) = self.rpc.remove(&id) else { return Ok(()) };
        let already_initialized = rpc.method == "initialize" && line["error"]["message"].as_str()
            .is_some_and(|message|message.to_lowercase().contains("already initialized"));
        if let Some(wire) = self.wires.get_mut(&rpc.operation_id) { wire.final_result = true; }
        if !line["error"].is_null() && !already_initialized {
            effects.push(Effect::Reply { operation_id:rpc.operation_id,disposition:Disposition::Rejected,payload:json!({"error":line["error"]}) });
            return Ok(());
        }
        let result = line.get("result").cloned().unwrap_or_else(||json!({}));
        match rpc.method.as_str() {
            "initialize" => self.initialized = true,
            "thread/resume" | "thread/start" => {
                let thread = &result["thread"];
                if let Some(id) = thread["id"].as_str() {
                    if id != self.thread_id { self.clear_preview(effects); self.thread_id = id.into(); }
                }
                self.model = string(&result["model"]).or(self.model.clone());
                self.effort = string(&result["reasoningEffort"]).or(self.effort.clone());
                self.restore_thread(thread,&rpc);
                self.async_questions.hydrate(&self.thread_id,thread);
                self.policy("session.patch_meta",json!({"thread_id":self.thread_id,"rollout_path":thread["path"]}),effects);
            }
            "turn/start" => {
                if rpc.state_revision == self.state_revision {
                    self.turn_id = string(&result["turn"]["id"]); self.in_progress = true; self.state_revision += 1;
                }
            }
            "thread/compact/start" => {
                if rpc.state_revision == self.state_revision { self.in_progress = true; self.compacting = true; self.state_revision += 1; }
            }
            "thread/read" => {
                self.restore_thread(&result["thread"],&rpc);
                if rpc.params["includeTurns"] == true { self.async_questions.hydrate(&self.thread_id,&result["thread"]); }
            }
            "thread/settings/update" => {
                if rpc.settings_revision == self.settings_revision {
                    if rpc.params.get("model").is_some() { self.model = string(&rpc.params["model"]); }
                    if rpc.params.get("effort").is_some() { self.effort = string(&rpc.params["effort"]); }
                    if let Some(mode) = rpc.params["collaborationMode"]["mode"].as_str() { self.mode = Some(mode.into()); }
                    self.settings_revision += 1;
                    self.policy("session.patch_meta",json!({"model":self.model,"effort":self.effort}),effects);
                }
            }
            "account/rateLimits/read" if result["rateLimits"]["limitId"].is_null() || result["rateLimits"]["limitId"] == "codex" => self.rate_limits = result["rateLimits"].clone(),
            _ => {},
        }
        if let Some(next) = rpc.continuation.clone() {
            if rpc.timed_out {
                effects.push(Effect::Reply { operation_id:rpc.operation_id,disposition:Disposition::Accepted,payload:result });
                self.changed(effects,true); return Ok(());
            }
            match next["kind"].as_str() {
                Some("bootstrap") if rpc.method == "initialize" => {
                    let parent = next["parent"].as_str().unwrap_or("");
                    let notification = format!("{parent}:initialized");
                    self.wires.insert(notification.clone(),Wire { request_id:None,server_request:None,server_epoch:None,final_result:false });
                    effects.push(Effect::Write { operation_id:Some(notification),
                        frame:json!({"jsonrpc":"2.0","method":"initialized","params":{}}) });
                    let (method,params) = if self.reconnect && !self.thread_id.is_empty() { ("thread/resume",json!({"threadId":self.thread_id})) }
                        else { ("thread/start",json!({"cwd":self.metadata["cwd"],"model":self.model,
                            "approvalPolicy":approval(&self.permission_mode),"sandbox":sandbox(&self.permission_mode)})) };
                    self.rpc(format!("{parent}:thread"),method,params,
                        Some(json!({"kind":"bootstrap_thread","parent":parent})),effects);
                }
                Some("bootstrap_thread") => {
                    let parent = next["parent"].as_str().unwrap_or("");
                    if let Some(effort) = self.metadata["effort"].as_str().map(str::to_owned) {
                        self.rpc(format!("{parent}:effort"),"thread/settings/update",json!({"threadId":self.thread_id,"effort":effort}),
                            Some(json!({"kind":"bootstrap_ready","parent":parent})),effects);
                    } else { self.ready = true; effects.push(Effect::Reply { operation_id:parent.into(),disposition:Disposition::Accepted,payload:json!({"ready":true}) }); effects.push(Effect::WakeQueue); }
                }
                Some("bootstrap_ready") => {
                    self.ready = true;
                    effects.push(Effect::Reply { operation_id:next["parent"].as_str().unwrap_or("").into(),disposition:Disposition::Accepted,payload:json!({"ready":true}) });
                    effects.push(Effect::WakeQueue);
                }
                Some("set_mode") => {
                    let Some(model) = &self.model else { return Err(error("modelo atual indisponível")); };
                    self.rpc(next["parent"].as_str().unwrap_or("").into(),"thread/settings/update",json!({"threadId":self.thread_id,
                        "collaborationMode":{"mode":next["mode"],"settings":{"model":model,"reasoning_effort":self.effort,"developer_instructions":null}}}),None,effects);
                }
                Some("interrupt") => {
                    let parent = next["parent"].as_str().unwrap_or("");
                    if let Some(turn) = &self.turn_id { self.rpc(parent.into(),"turn/interrupt",json!({"threadId":self.thread_id,"turnId":turn}),None,effects); }
                    else { effects.push(Effect::Reply { operation_id:parent.into(),disposition:Disposition::Accepted,payload:json!({"interrupted":false}) }); }
                }
                Some("async_answer") => self.async_questions.record_answer(next["request_id"].as_str().unwrap_or(""),next["text"].as_str().unwrap_or("")),
                _ => {},
            }
        }
        let payload = if rpc.method == "model/list" { json!(result["data"].as_array().map(|models|models.iter().filter(|m|m["hidden"] != true).map(|model|json!({
            "model":model["model"],"displayName":model["displayName"],"description":model["description"],
            "efforts":model["supportedReasoningEfforts"].as_array().map(|efforts|efforts.iter().map(|e|json!({"value":e["reasoningEffort"],"description":e["description"]})).collect::<Vec<_>>()).unwrap_or_default(),
            "defaultEffort":model["defaultReasoningEffort"]})).collect::<Vec<_>>()).unwrap_or_default()) }
            else { result };
        effects.push(Effect::Reply { operation_id:rpc.operation_id,disposition:Disposition::Accepted,payload });
        self.changed(effects,true);
        Ok(())
    }

    fn restore_thread(&mut self,thread:&Value,rpc:&Rpc) {
        if rpc.state_revision == self.state_revision {
            match thread["status"]["type"].as_str() {
                Some("active") => {
                    self.in_progress = true;
                    if rpc.params["includeTurns"] == true {
                        self.turn_id = thread["turns"].as_array().and_then(|turns|turns.iter().rev().find(|turn|turn["status"] == "inProgress"))
                            .and_then(|turn|string(&turn["id"]));
                    }
                }
                Some("idle") => { self.in_progress = false; self.turn_id = None; self.state.codex_buffering = false; },
                _ => {},
            }
        }
        if rpc.settings_revision == self.settings_revision {
            if thread["model"].is_string() { self.model = string(&thread["model"]); }
            if thread.get("reasoningEffort").is_some() { self.effort = string(&thread["reasoningEffort"]); }
        }
    }

    fn notification(&mut self,line:Value,effects:&mut Vec<Effect>) -> Result<(),RuntimeError> {
        let method = line["method"].as_str().unwrap_or("");
        let params = &line["params"];
        if params["threadId"].as_str().is_some_and(|thread|thread != self.thread_id) { return Ok(()); }
        if let Some(id) = line.get("id").filter(|id|!id.is_null()) {
            let request_id:RequestId = serde_json::from_value(id.clone()).map_err(|_|error("ID de pedido do servidor inválido"))?;
            if self.server_requests.iter().any(|(id,request)|id == &request_id && request == &line) { return Ok(()); }
            self.counter += 1;
            self.request_epochs.insert(request_id.clone(),self.counter);
            self.answering.remove(&request_id);
            if let Some((_,request)) = self.server_requests.iter_mut().find(|(id,_)|id == &request_id) { *request = line.clone(); }
            else { self.server_requests.push((request_id.clone(),line.clone())); }
            if !["item/commandExecution/requestApproval","item/fileChange/requestApproval","item/tool/requestUserInput"].contains(&method) {
                self.counter += 1;
                self.answer(format!("server:{}:{}",self.generation,self.counter),request_id,Value::Null,
                    Some(json!({"code":-32601,"message":"método não suportado pelo Hangar"})),effects)?;
                self.policy("unknown_private",json!({"kind":method,"event":line}),effects);
            }
            self.changed(effects,true); return Ok(());
        }
        match method {
            "serverRequest/resolved" => {
                let request_id:RequestId = serde_json::from_value(params["requestId"].clone()).map_err(|_|error("ID de resolução inválido"))?;
                self.server_requests.retain(|(id,_)|id != &request_id); self.answering.remove(&request_id); self.request_epochs.remove(&request_id);
            }
            "turn/started" => {
                self.in_progress = true; self.turn_id = string(&params["turn"]["id"]); self.state_revision += 1;
                self.response_started = false; self.state.codex_buffering = false; self.clear_preview(effects);
                self.state.problema = None; self.state.problema_detalhe = None;
            }
            "turn/completed" => {
                if params["turn"]["id"].as_str().is_some_and(|id|self.turn_id.as_deref().is_some_and(|current|current != id)) { return Ok(()); }
                self.in_progress = false; self.turn_id = None; self.state_revision += 1; self.compacting = false;
                self.state.codex_buffering = false; self.response_started = false; self.clear_preview(effects);
                self.server_requests.clear(); self.answering.clear(); self.request_epochs.clear();
                if params["turn"]["status"] == "failed" {
                    self.state.problema = Some("headless_turno_erro".into());
                    self.state.problema_detalhe = string(&params["turn"]["error"]["message"]);
                } else if self.state.problema.as_deref() == Some("codex_sem_conexao") { self.state.problema = None; self.state.problema_detalhe = None; }
                self.changed(effects,true); effects.push(Effect::WakeQueue); return Ok(());
            }
            "thread/status/changed" => {
                match params["status"]["type"].as_str() {
                    Some("active") => self.in_progress = true,
                    Some("idle") => { self.in_progress = false; self.turn_id = None; self.state.codex_buffering = false; },
                    _ => return Ok(()),
                }
                self.state_revision += 1;
            }
            "thread/settings/updated" => {
                if params["threadId"] != self.thread_id { return Ok(()); }
                let settings = &params["threadSettings"];
                self.model = string(&settings["model"]); self.effort = string(&settings["effort"]);
                self.mode = Some(settings["collaborationMode"]["mode"].as_str().unwrap_or("default").into()); self.settings_revision += 1;
            }
            "item/agentMessage/delta" => {
                if params["turnId"].as_str().is_some_and(|id|self.turn_id.as_deref().is_some_and(|current|current != id)) { return Ok(()); }
                self.response_started = true; self.state.codex_buffering = false;
                if let Some(text) = self.preview.append(params["delta"].as_str().unwrap_or(""),self.clock.monotonic_s) { self.publish(text,effects); }
                return Ok(());
            }
            "item/started" | "item/completed" => {
                let item = &params["item"];
                self.async_questions.observe(&self.thread_id,item);
                if item["type"] == "contextCompaction" { self.compacting = method == "item/started"; }
                if item["type"] == "agentMessage" { self.clear_preview(effects); }
                if item["type"] != "userMessage" && self.state.problema.as_deref() == Some("codex_sem_conexao") {
                    self.state.problema = None; self.state.problema_detalhe = None;
                }
            }
            "model/safetyBuffering/updated" => {
                if !self.in_progress || self.response_started || params["threadId"] != self.thread_id { return Ok(()); }
                if self.turn_id.as_deref().is_some_and(|turn|params["turnId"] != turn) { return Ok(()); }
                if let Some(buffering) = params["showBufferingUi"].as_bool() { self.state.codex_buffering = buffering; }
            }
            "thread/tokenUsage/updated" => { if params["tokenUsage"].is_object() { self.token_usage = params["tokenUsage"].clone(); } }
            "account/rateLimits/updated" => {
                if params["rateLimits"].is_object() && (params["rateLimits"]["limitId"].is_null() || params["rateLimits"]["limitId"] == "codex") { self.rate_limits = params["rateLimits"].clone(); }
                else { return Ok(()); }
            }
            "error" => {
                self.state.problema = Some(if params["willRetry"] == true { "codex_sem_conexao" } else { "headless_turno_erro" }.into());
                self.state.problema_detalhe = string(&params["error"]["message"]);
            }
            "hook/completed" if params["run"]["eventName"] == "userPromptSubmit"
                && ["blocked","stopped"].contains(&params["run"]["status"].as_str().unwrap_or("")) => {
                let run = &params["run"];
                let source = run["sourcePath"].as_str().unwrap_or("hook");
                let normalized = source.replace('\\',"/");
                let parts:Vec<_> = normalized.split('/').collect();
                let origin = parts.iter().position(|part|*part == "cache").and_then(|index|parts.get(index+2)).copied().unwrap_or(source);
                let reason = run["entries"].as_array().and_then(|entries|entries.iter().find(|entry|
                    ["stop","feedback","error"].contains(&entry["kind"].as_str().unwrap_or("")) && entry["text"].is_string()))
                    .and_then(|entry|entry["text"].as_str()).unwrap_or("");
                self.state.problema = Some("codex_prompt_bloqueado".into());
                self.state.problema_detalhe = Some(format!("{origin}: {reason}").trim_matches([' ',':']).chars().take(300).collect());
            }
            _ => return Ok(()),
        }
        self.changed(effects,true);
        Ok(())
    }

    pub fn next_deadline(&self) -> Option<f64> {
        self.preview.deadline().into_iter().chain(self.rpc.values().filter(|rpc|!rpc.timed_out).map(|rpc|rpc.deadline)).min_by(f64::total_cmp)
    }

    pub fn confirm_input(&mut self,operation_id:&str) -> Vec<Effect> {
        let ids:Vec<_> = self.rpc.iter().filter(|(_,rpc)|rpc.operation_id == operation_id
            && ["turn/start","turn/steer"].contains(&rpc.method.as_str())).map(|(id,_)|id.clone()).collect();
        for id in ids { self.rpc.remove(&id); }
        if let Some(wire) = self.wires.get_mut(operation_id) { wire.final_result = true; }
        vec![Effect::Reply { operation_id:operation_id.into(),disposition:Disposition::Accepted,payload:json!({"confirmed":true}) }]
    }

    pub fn write_is_current(&self,operation_id:&str) -> bool {
        self.wires.get(operation_id).is_none_or(|wire|wire.server_request.as_ref().is_none_or(|id|
            self.request_epochs.get(id).copied() == wire.server_epoch))
    }
}

fn question_response(question:&Value,answers:&Value) -> Result<Value,RuntimeError> {
    let questions = question["questions"].as_array().ok_or_else(||error("perguntas inválidas"))?;
    let answers = answers.as_array().filter(|answers|answers.len() == questions.len()).ok_or_else(||error("responda a todas as perguntas"))?;
    let mut result = serde_json::Map::new();
    for answer in answers {
        let id = answer["question_id"].as_str().ok_or_else(||error("a resposta não corresponde às perguntas pendentes"))?;
        let question = questions.iter().find(|q|q["id"] == id).ok_or_else(||error("a resposta não corresponde às perguntas pendentes"))?;
        if result.contains_key(id) { return Err(error("pergunta respondida duas vezes")); }
        let value = match answer["kind"].as_str() {
            Some("text") => {
                if question["options"].as_array().is_some_and(|o|!o.is_empty()) && question["isOther"] != true { return Err(error("esta pergunta não aceita resposta em texto")); }
                answer["value"].as_str().filter(|s|!s.trim().is_empty()).ok_or_else(||error("resposta vazia"))?.to_owned()
            }
            Some("option") => {
                let indices = answer["indices"].as_array().filter(|i|i.len() == 1).ok_or_else(||error("escolha uma opção válida"))?;
                let index = indices[0].as_u64().ok_or_else(||error("opção inválida"))?;
                question["options"].as_array().and_then(|options|options.get(index as usize)).and_then(|option|option["label"].as_str()).ok_or_else(||error("opção inválida"))?.into()
            }
            _ => return Err(error("responda à pergunta antes de enviar")),
        };
        result.insert(id.into(),json!({"answers":[value]}));
    }
    Ok(json!({"answers":result}))
}
