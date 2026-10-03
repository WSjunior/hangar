use super::{LiveBuffer, protocol::*};
use hangar_api::state::StateEvent;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

struct Waiter {
    operation_id: String,
    subtype: String,
    payload: Value,
    deadline: f64,
    timed_out: bool,
}

struct Wire {
    kind: String,
    request_id: Option<RequestId>,
    final_result: bool,
}

pub struct ClaudeEngine {
    metadata: Value,
    generation: u64,
    clock: ClockSample,
    counter: u64,
    alive: bool,
    initialized: bool,
    initializing: bool,
    init_warning: Option<f64>,
    in_progress: bool,
    state: StateEvent,
    model: Option<String>,
    effort: Option<String>,
    permission_mode: Option<String>,
    previous_non_plan: Option<String>,
    restore_plan: Option<(String,String)>,
    pending: Vec<(RequestId,Value)>,
    question: Option<Value>,
    waiters: BTreeMap<RequestId,Waiter>,
    wires: BTreeMap<String,Wire>,
    retired_writes: BTreeSet<String>,
    policies: BTreeMap<RequestId,String>,
    last_format_request: Option<RequestId>,
    reload_deadline: f64,
    rate_limit_info: Value,
    commands: Option<Value>,
    terminal_commands: Value,
    preview: LiveBuffer,
    thinking: LiveBuffer,
    tool_input: LiveBuffer,
    tool_name: Option<String>,
    tool_visible: bool,
    label: Option<String>,
    compacting: bool,
    turn_start: Option<f64>,
    label_deadline: Option<f64>,
    tokens_closed: u64,
    tokens_message: Option<u64>,
    token_chars: u64,
    thinking_start: Option<f64>,
    thought_s: f64,
    tasks: Vec<(String,Value)>,
    usage: Value,
    context_window: Option<u64>,
    cost: Option<f64>,
    effort_intent: Option<Value>,
    effort_deadline: Option<f64>,
    active_input: Option<String>,
    unknown: BTreeSet<String>,
}

fn string(value:&Value) -> Option<String> { value.as_str().map(str::to_owned) }
fn error(message:&str) -> RuntimeError { RuntimeError::new("claude_command",message) }
fn mode(value:&str) -> String { if value == "default" { "manual".into() } else { value.into() } }

impl ClaudeEngine {
    pub fn new(metadata:Value,generation:u64,clock:ClockSample) -> Self {
        let permission_mode = metadata["permission_mode"].as_str().map(mode);
        let previous_non_plan = string(&metadata["previous_non_plan"]);
        let state = StateEvent { session:metadata["name"].as_str().unwrap_or("").into(), state:"idle".into(),
            headless:true, claude_permission_mode:permission_mode.clone(),claude_previous_non_plan:previous_non_plan.clone(),
            status_line:string(&metadata["status_line"]),..StateEvent::default() };
        let mut effort_intent = metadata["effort_intent"].is_object().then(||metadata["effort_intent"].clone());
        if let Some(intent) = effort_intent.as_mut() {
            if intent["status"] == "dispatching" { intent["status"] = json!("unknown"); }
        }
        let mut engine = Self { generation,clock,counter:metadata["runtime_counter"].as_u64().unwrap_or(0),alive:true,
            initialized:metadata["initialized"] == true,initializing:metadata["initialized"] != true,
            init_warning:None,in_progress:false,state,model:string(&metadata["model"]),effort:string(&metadata["effort"]),
            permission_mode,previous_non_plan,restore_plan:None,pending:Vec::new(),question:None,
            waiters:BTreeMap::new(),wires:BTreeMap::new(),retired_writes:BTreeSet::new(),policies:BTreeMap::new(),last_format_request:None,reload_deadline:clock.monotonic_s+10.0,
            rate_limit_info:Value::Null,commands:metadata.get("commands").filter(|commands|!commands.is_null()).cloned(),terminal_commands:metadata.get("terminal_commands").cloned().unwrap_or_else(||json!([])),
            preview:LiveBuffer::default(),thinking:LiveBuffer::default(),tool_input:LiveBuffer::default(),tool_name:None,tool_visible:false,
            label:None,compacting:false,turn_start:None,label_deadline:None,tokens_closed:0,tokens_message:None,
            token_chars:0,thinking_start:None,thought_s:0.0,tasks:Vec::new(),usage:metadata.get("usage").cloned().unwrap_or(Value::Null),
            context_window:metadata["context_window"].as_u64(),cost:metadata["cost"].as_f64(),effort_intent,effort_deadline:None,active_input:None,unknown:BTreeSet::new(),metadata };
        if let Some(controls) = engine.metadata["control_carry"].as_array() {
            for control in controls {
                let Ok(request_id) = serde_json::from_value::<RequestId>(control["request_id"].clone()) else { continue };
                let (Some(operation_id),Some(subtype)) = (control["operation_id"].as_str(),control["subtype"].as_str()) else { continue };
                engine.waiters.insert(request_id.clone(),Waiter { operation_id:operation_id.into(),subtype:subtype.into(),
                    payload:control["payload"].clone(),deadline:clock.monotonic_s,timed_out:true });
                engine.wires.insert(operation_id.into(),Wire { kind:"control".into(),request_id:Some(request_id),final_result:false });
            }
        }
        engine
    }

    pub fn write_is_current(&self,id:&str) -> bool { !self.retired_writes.contains(id) }

    fn reset_conversation(&mut self,effects:&mut Vec<Effect>) {
        for (id,wire) in &mut self.wires {
            self.retired_writes.insert(id.clone());
            if !wire.final_result {
                effects.push(Effect::Reply { operation_id:id.clone(),disposition:Disposition::Unknown,
                    payload:json!({"reason":"conversation_changed"}) });
                wire.final_result = true;
            }
        }
        self.waiters.clear(); self.policies.clear(); self.last_format_request = None;
        self.pending.clear(); self.question = None; self.effort_intent = None;
        self.effort_deadline = None; self.active_input = None; self.restore_plan = None;
        self.in_progress = false; self.turn_start = None; self.label_deadline = None;
        self.clear_streams(effects);
    }

    pub fn view(&self) -> Value {
        let state = self.public_state();
        json!({"public_state":state,"alive":self.alive,"iniciando":self.initializing,"initialized":self.initialized,
            "in_progress":self.in_progress,"pending":self.pending.iter().map(|(id,request)|json!({"request_id":id,"request":request})).collect::<Vec<_>>(),
            "question":self.question,"model":self.model,"effort":self.effort,"permission_mode":self.permission_mode,
            "previous_non_plan":self.previous_non_plan,"commands":self.commands,"terminal_commands":self.terminal_commands,
            "usage":self.usage,"context_window":self.context_window,"cost":self.cost,"effort_intent":self.effort_intent,
            "runtime_counter":self.counter,"conversation":self.metadata["session_id"],"deliverable":self.deliverable()})
    }

    fn public_state(&self) -> StateEvent {
        let mut state = self.state.clone();
        state.claude_permission_mode = self.permission_mode.clone();
        state.claude_previous_non_plan = self.previous_non_plan.clone();
        state.codex_question = self.question.as_ref().and_then(|q|q.as_object().cloned());
        state.question = None; state.options = None; state.claude_plan_pending = None;
        state.state = if !self.alive { "dead" } else if !self.pending.is_empty() || self.question.is_some() { "awaiting_input" }
            else if self.in_progress || self.initializing { "working" } else { "idle" }.into();
        let mut label = self.label.clone();
        if self.initializing && !self.in_progress { label = Some("Iniciando sessão…".into()); }
        if state.state == "working" {
            if let Some(start) = self.turn_start {
                let seconds = (self.clock.monotonic_s - start).max(0.0) as u64;
                let elapsed = if seconds >= 60 { format!("{}m {}s",seconds/60,seconds%60) } else { format!("{seconds}s") };
                let mut counts = vec![elapsed];
                let tokens = self.tokens_closed + self.tokens_message.unwrap_or(self.token_chars/4);
                if tokens > 0 { counts.push(if tokens >= 1000 { format!("↓ {:.1}k tokens",tokens as f64/1000.0) } else { format!("↓ {tokens} tokens") }); }
                let thought = self.thought_s + self.thinking_start.map_or(0.0,|start|(self.clock.monotonic_s-start).max(0.0));
                if thought >= 1.0 { counts.push(format!("thought for {}s",thought as u64)); }
                label = Some(format!("{} ({})",label.as_deref().unwrap_or("Trabalhando…"),counts.join(" · ")));
            }
        }
        state.label = label;
        if self.question.is_none() {
            if let Some((_,request)) = self.pending.first() {
                if request["tool_name"] == "ExitPlanMode" {
                    state.question = Some("Aprovar o plano?".into());
                    state.options = Some(vec!["Aprovar plano".into(),"Continuar planejando".into()]);
                    state.claude_plan_pending = json!({"plan":request["input"]["plan"].as_str().unwrap_or(""),
                        "path":request["input"]["planFilePath"].as_str(),"tool_use_id":request["tool_use_id"].as_str()}).as_object().cloned();
                } else {
                    let detail = request["description"].as_str().or_else(||request["input"]["command"].as_str())
                        .or_else(||request["input"]["file_path"].as_str()).or_else(||request["input"]["path"].as_str()).unwrap_or("");
                    let detail = if detail.chars().count() > 200 { format!("{}…",detail.chars().take(200).collect::<String>()) } else { detail.into() };
                    state.question = Some(format!("Permitir {}? {detail}",request["tool_name"].as_str().unwrap_or("ferramenta")).trim().into());
                    let mut options = vec!["Permitir".into(),"Negar".into()];
                    if request["permission_suggestions"].as_array().is_some_and(|rules|rules.iter().any(Value::is_object)) { options.push("Sempre permitir".into()); }
                    state.options = Some(options);
                }
            }
        }
        state
    }

    fn deliverable(&self) -> bool { self.alive && self.initialized && !self.initializing && !self.in_progress
        && self.pending.is_empty() && self.question.is_none() && self.active_input.is_none() }

    fn policy(&mut self, kind:&str,payload:Value,effects:&mut Vec<Effect>) {
        self.counter += 1;
        let request_id = RequestId::String(format!("policy:{}:{}",self.generation,self.counter));
        if kind == "format_status" { self.last_format_request = Some(request_id.clone()); }
        self.policies.insert(request_id.clone(),kind.into());
        effects.push(Effect::Policy { kind:kind.into(),request_id,payload });
    }

    fn changed(&mut self,effects:&mut Vec<Effect>,format:bool) {
        effects.push(Effect::StateChanged);
        if format { self.policy("format_status",json!({"model":self.model,"effort":self.effort,"usage":self.usage,
            "context_window":self.context_window,"cost":self.cost,"rate_limit_info":self.rate_limit_info}),effects); }
    }

    fn publish(&self,channel:&str,text:String,effects:&mut Vec<Effect>) {
        effects.push(Effect::Publish { channel:channel.into(),data:json!({"session":self.state.session,
            "text":text,"md":true,"full":true,"vivo":true}) });
    }

    fn clear_streams(&mut self,effects:&mut Vec<Effect>) {
        if let Some(text) = self.preview.clear() { self.publish("preview",text,effects); }
        if let Some(text) = self.thinking.clear() { self.publish("thinking",text,effects); }
        self.clear_tool(effects);
        self.tool_name = None;
    }

    fn clear_tool(&mut self,effects:&mut Vec<Effect>) {
        self.tool_input.clear();
        if std::mem::take(&mut self.tool_visible) { self.publish("tool",String::new(),effects); }
    }

    fn start_turn(&mut self) {
        self.in_progress = true; self.turn_start = Some(self.clock.monotonic_s);
        self.label_deadline = Some(self.clock.monotonic_s+1.0); self.tokens_closed = 0; self.tokens_message = None;
        self.token_chars = 0; self.thinking_start = None; self.thought_s = 0.0; self.compacting = false;
    }

    fn control(&mut self,operation_id:String,subtype:&str,payload:Value,effects:&mut Vec<Effect>) {
        self.counter += 1;
        let request_id = RequestId::String(format!("hangar:{}:{}",self.generation,self.counter));
        let mut request = payload.as_object().cloned().unwrap_or_default();
        request.insert("subtype".into(),json!(subtype));
        self.waiters.insert(request_id.clone(),Waiter { operation_id:operation_id.clone(),subtype:subtype.into(),
            payload:payload.clone(),deadline:self.clock.monotonic_s + if subtype == "initialize" { 180.0 } else { 15.0 },timed_out:false });
        self.wires.insert(operation_id.clone(),Wire { kind:"control".into(),request_id:Some(request_id.clone()),final_result:false });
        effects.push(Effect::Write { frame:json!({"type":"control_request","request_id":request_id,"request":request}),operation_id:Some(operation_id) });
    }

    pub fn restore_control(&mut self,operation_id:String,frame:&Value) {
        if frame["type"] != "control_request" {
            let kind = match frame["type"].as_str() {
                Some("control_response")=>"recovered_answer",
                Some("user") if self.effort_intent.as_ref().is_some_and(|intent|
                    operation_id == format!("{}:effort",intent["operation_id"].as_str().unwrap_or("")))=>"effort",
                Some("user")=>"input",
                _=>return,
            };
            self.wires.insert(operation_id,Wire { kind:kind.into(),request_id:None,final_result:false });
            return;
        }
        let (Ok(request_id),Some(subtype)) = (serde_json::from_value::<RequestId>(frame["request_id"].clone()),frame["request"]["subtype"].as_str()) else { return };
        if self.waiters.contains_key(&request_id) { return; }
        let mut payload = frame["request"].clone();
        if let Some(fields) = payload.as_object_mut() { fields.remove("subtype"); }
        self.waiters.insert(request_id.clone(),Waiter { operation_id:operation_id.clone(),subtype:subtype.into(),payload,
            deadline:self.clock.monotonic_s,timed_out:true });
        self.wires.insert(operation_id,Wire { kind:"control".into(),request_id:Some(request_id),final_result:false });
    }

    fn answer(&mut self,operation_id:String,request_id:RequestId,body:Value,effects:&mut Vec<Effect>) {
        self.wires.insert(operation_id.clone(),Wire { kind:"answer".into(),request_id:Some(request_id.clone()),final_result:false });
        effects.push(Effect::Write { frame:json!({"type":"control_response","response":{
            "subtype":"success","request_id":request_id,"response":body}}),operation_id:Some(operation_id) });
    }

    pub fn start_initialize(&mut self,operation_id:String) -> Result<Vec<Effect>,RuntimeError> {
        let mut effects = Vec::new();
        self.initialized = false; self.initializing = true;
        self.init_warning = Some(self.clock.monotonic_s+60.0);
        self.control(operation_id,"initialize",json!({}),&mut effects);
        self.changed(&mut effects,true);
        Ok(effects)
    }

    pub fn hydrate(&mut self,snapshot:CanoSnapshot) -> Result<Vec<Effect>,RuntimeError> {
        let mut effects = Vec::new();
        self.alive = snapshot.saiu.is_none();
        for raw in snapshot.init.iter().chain(snapshot.rate_limit.iter()) {
            effects.extend(self.apply(EngineInput::Line(serde_json::from_str(raw).map_err(|_|error("snapshot inválido"))?),self.clock)?);
        }
        if snapshot.aberto { self.start_turn(); }
        for raw in snapshot.pendentes {
            effects.extend(self.apply(EngineInput::Line(serde_json::from_str(&raw).map_err(|_|error("pedido pendente inválido"))?),self.clock)?);
        }
        let prefix = &snapshot.inflight["claude"];
        if prefix["complete"] == false { self.preview.block(); self.thinking.block(); self.tool_input.block(); }
        else {
            if let Some(text) = self.preview.append(prefix["text"].as_str().unwrap_or(""),self.clock.monotonic_s) { self.publish("preview",text,&mut effects); }
            if let Some(text) = self.thinking.append(prefix["thinking"].as_str().unwrap_or(""),self.clock.monotonic_s) { self.publish("thinking",text,&mut effects); }
            self.tool_name = string(&prefix["tool"]["name"]);
            if let Some(text) = self.tool_input.append(prefix["tool"]["input"].as_str().unwrap_or(""),self.clock.monotonic_s) { self.publish_tool(text,&mut effects); }
        }
        if let Some(raw) = snapshot.ultimo_result {
            let result:Value = serde_json::from_str(&raw).map_err(|_|error("resultado do snapshot inválido"))?;
            self.apply_usage(&result,&mut effects);
        }
        self.changed(&mut effects,true);
        if self.usage.is_null() { self.policy("last_usage",json!({}),&mut effects); }
        self.policy("reload_stamp",json!({}),&mut effects);
        Ok(effects)
    }

    pub fn command(&mut self,command:RuntimeCommand,clock:ClockSample) -> Result<Vec<Effect>,RuntimeError> {
        self.clock = clock;
        if !self.alive { return Err(error("processo não está vivo")); }
        let mut effects = Vec::new();
        let operation_id = command.operation_id;
        let payload = command.payload;
        match command.kind {
            OperationKind::Input | OperationKind::Steer => {
                if command.kind == OperationKind::Input && !self.deliverable() {
                    return Ok(vec![Effect::Reply { operation_id,disposition:Disposition::Deferred,payload:json!({}) }]);
                }
                let text = payload["text"].as_str().ok_or_else(||error("mensagem sem texto"))?;
                self.wires.insert(operation_id.clone(),Wire { kind:"input".into(),request_id:None,final_result:false });
                if !self.in_progress {
                    self.active_input = Some(operation_id.clone());
                    self.start_turn();
                }
                effects.push(Effect::Write { operation_id:Some(operation_id),frame:json!({"type":"user","session_id":"","parent_tool_use_id":null,"message":{
                    "role":"user","content":payload.get("content").cloned().unwrap_or_else(||json!([{"type":"text","text":text}]))}}) });
            }
            OperationKind::Interrupt => {
                let mut requests:Vec<_> = self.pending.iter().map(|(id,_)|id.clone()).collect();
                if let Some(question) = &self.question {
                    requests.push(serde_json::from_value(question["request_id"].clone()).map_err(|_|error("ID da pergunta inválido"))?);
                }
                for (index,id) in requests.into_iter().enumerate() {
                    self.answer(format!("{operation_id}:deny:{index}"),id,json!({"behavior":"deny","message":"Interrompido pelo usuário."}),&mut effects);
                }
                self.restore_plan = None;
                self.control(operation_id,"interrupt",json!({}),&mut effects);
                self.clear_streams(&mut effects);
            }
            OperationKind::Select => {
                let (id,request) = self.pending.first().cloned().ok_or_else(||error("nenhuma permissão pendente"))?;
                let option = payload["option"].as_u64().filter(|n| (1..=3).contains(n)).ok_or_else(||error("opção inválida"))?;
                let suggestions:Vec<_> = request["permission_suggestions"].as_array().map(|rules|rules.iter().filter(|v|v.is_object()).cloned().collect()).unwrap_or_default();
                let allow = option == 1 || option == 3 && !suggestions.is_empty();
                let mut body = if allow { json!({"behavior":"allow","updatedInput":request.get("input").cloned().unwrap_or_else(||json!({}))}) }
                    else { json!({"behavior":"deny","message":if request["tool_name"] == "ExitPlanMode" {
                        "O usuário não aprovou o plano. Continue no modo plano e aguarde as instruções dele." } else { "Usuário recusou." }}) };
                if option == 3 && allow { body["updatedPermissions"] = json!(suggestions); }
                if request["tool_name"] == "ExitPlanMode" && allow {
                    self.restore_plan = self.previous_non_plan.as_ref().filter(|mode|mode.as_str() != "manual")
                        .map(|mode|(mode.clone(),operation_id.clone()));
                }
                self.answer(operation_id,id,body,&mut effects);
            }
            OperationKind::AnswerQuestions | OperationKind::SkipQuestion => {
                let question = self.question.as_ref().ok_or_else(||error("nenhuma pergunta pendente"))?;
                let request_id:RequestId = serde_json::from_value(question["request_id"].clone()).map_err(|_|error("ID da pergunta inválido"))?;
                let supplied:RequestId = serde_json::from_value(payload["request_id"].clone()).map_err(|_|error("ID da resposta inválido"))?;
                if supplied != request_id { return Err(error("a pergunta mudou")); }
                let body = if command.kind == OperationKind::SkipQuestion { json!({"behavior":"deny","message":"O usuário prefere conversar antes de responder."}) }
                    else { answer_body(question,&payload["answers"])? };
                self.answer(operation_id,request_id,body,&mut effects);
            }
            OperationKind::SetPermissionMode => {
                let requested = payload["mode"].as_str().ok_or_else(||error("modo não informado"))?;
                self.control(operation_id,"set_permission_mode",json!({"mode":requested}),&mut effects);
            }
            OperationKind::SetModel | OperationKind::SetEffort => {
                if let Some(model) = payload["model"].as_str().filter(|m|!m.is_empty()) {
                    self.control(operation_id.clone(),"set_model",json!({"model":model}),&mut effects);
                    if let Some(waiter) = self.waiters.values_mut().find(|waiter|waiter.operation_id == operation_id) {
                        waiter.payload = payload.clone();
                    }
                } else { self.request_effort(operation_id,&payload,&mut effects)?; }
            }
            OperationKind::ListModels => self.control(operation_id,"list_models",json!({}),&mut effects),
            OperationKind::Compact => {
                if !self.deliverable() { return Err(error("aguarde a sessão ficar ociosa")); }
                self.local_command(operation_id,"/compact",&mut effects);
            }
            OperationKind::Commands => effects.push(Effect::Reply { operation_id,disposition:Disposition::Accepted,
                payload:json!({"commands":self.commands,"terminal_commands":self.terminal_commands}) }),
            OperationKind::Cwd => effects.push(Effect::Reply { operation_id,disposition:Disposition::Accepted,payload:self.metadata["cwd"].clone() }),
            OperationKind::Detach => effects.push(Effect::Stop { reason:"detach".into() }),
            _ => return Err(error("operação exige a barreira de lifecycle ou não é suportada pelo Claude")),
        }
        self.changed(&mut effects,true);
        Ok(effects)
    }

    fn local_command(&mut self,operation_id:String,text:&str,effects:&mut Vec<Effect>) {
        self.wires.insert(operation_id.clone(),Wire { kind:if text.starts_with("/effort ") { "effort" } else { "local" }.into(),request_id:None,final_result:false });
        self.start_turn();
        effects.push(Effect::Write { operation_id:Some(operation_id),frame:json!({"type":"user","session_id":"","parent_tool_use_id":null,
            "message":{"role":"user","content":[{"type":"text","text":text}]}}) });
    }

    fn request_effort(&mut self,operation_id:String,payload:&Value,effects:&mut Vec<Effect>) -> Result<(),RuntimeError> {
        let effort = payload["effort"].as_str().filter(|s|!s.is_empty()).ok_or_else(||error("esforço não informado"))?;
        if self.effort.as_deref() == Some(effort) {
            effects.push(Effect::Reply { operation_id,disposition:Disposition::Accepted,payload:json!({"applied":true}) });
            return Ok(());
        }
        self.effort_intent = Some(json!({"operation_id":operation_id,"value":effort,"status":"prepared"}));
        if self.deliverable() { self.dispatch_effort(effects); }
        else { effects.push(Effect::Reply { operation_id,disposition:Disposition::Deferred,payload:json!({"applied":false}) }); }
        Ok(())
    }

    fn dispatch_effort(&mut self,effects:&mut Vec<Effect>) {
        let Some(intent) = self.effort_intent.as_mut() else { return };
        if intent["status"] != "prepared" { return; }
        intent["status"] = json!("dispatching");
        self.effort_deadline = Some(self.clock.monotonic_s+15.0);
        let id = format!("{}:effort",intent["operation_id"].as_str().unwrap_or(""));
        let text = format!("/effort {}",intent["value"].as_str().unwrap_or(""));
        effects.push(Effect::StateChanged);
        self.local_command(id,&text,effects);
    }

    pub fn apply(&mut self,input:EngineInput,clock:ClockSample) -> Result<Vec<Effect>,RuntimeError> {
        self.clock = clock;
        let mut effects = Vec::new();
        match input {
            EngineInput::Line(event) => self.on_line(event,&mut effects)?,
            EngineInput::WriteAck { operation_id,outcome } => self.on_ack(operation_id,outcome,&mut effects),
            EngineInput::Tick => self.on_tick(&mut effects),
            EngineInput::PolicyResult { request_id,payload } => {
                if let Some(kind) = self.policies.remove(&request_id) {
                    if kind == "format_status" && self.last_format_request.as_ref() != Some(&request_id) { return Ok(effects); }
                    match kind.as_str() {
                        "format_status" => {
                            self.state.status_line = string(&payload["status_line"]);
                            if payload.get("limit_reset").is_some() { self.state.limit_reset = string(&payload["limit_reset"]); }
                        }
                        "reload_stamp" => self.state.recarregar_motivo = string(&payload["reason"]),
                        "last_usage" => {
                            if self.usage.is_null() && payload["usage"].is_object() { self.usage = payload["usage"].clone(); }
                            self.changed(&mut effects,true);
                            return Ok(effects);
                        }
                        "quota" => {},
                        _ => {},
                    }
                    self.changed(&mut effects,false);
                }
            }
        }
        Ok(effects)
    }

    fn on_ack(&mut self,operation_id:String,outcome:WriteOutcome,effects:&mut Vec<Effect>) {
        let Some(wire) = self.wires.get_mut(&operation_id) else { return };
        if wire.final_result { return; }
        if wire.kind == "control" && outcome == WriteOutcome::Written { return; }
        if wire.kind == "effort" && outcome == WriteOutcome::Written { return; }
        let kind = wire.kind.clone();
        let request_id = wire.request_id.clone();
        let disposition = match outcome { WriteOutcome::Written => Disposition::Accepted,
            WriteOutcome::NotWritten => Disposition::Rejected,WriteOutcome::Unknown => Disposition::Unknown };
        wire.final_result = outcome != WriteOutcome::Unknown;
        if kind == "answer" && outcome == WriteOutcome::Written {
            self.pending.retain(|(id,_)|Some(id) != request_id.as_ref());
            if self.question.as_ref().and_then(|q|serde_json::from_value::<RequestId>(q["request_id"].clone()).ok()) == request_id { self.question = None; }
        }
        if kind == "input" && self.active_input.as_deref() == Some(operation_id.as_str()) && outcome == WriteOutcome::NotWritten {
            self.active_input = None; self.in_progress = false; self.turn_start = None; self.label_deadline = None;
        }
        if kind == "effort" && outcome != WriteOutcome::Written {
            self.effort_deadline = None;
            if let Some(intent) = self.effort_intent.as_mut() {
                if operation_id == format!("{}:effort",intent["operation_id"].as_str().unwrap_or("")) {
                    intent["status"] = json!(if outcome == WriteOutcome::NotWritten { "rejected" } else { "unknown" });
                }
            }
        }
        effects.push(Effect::Reply { operation_id,disposition,payload:json!({"write_outcome":outcome}) });
        self.changed(effects,true);
    }

    fn on_line(&mut self,event:Value,effects:&mut Vec<Effect>) -> Result<(),RuntimeError> {
        let kind = event["type"].as_str().unwrap_or("");
        if !event["parent_tool_use_id"].is_null() && !kind.starts_with("control_") && kind != "sdk_control_request" { return Ok(()); }
        match kind {
            "control_response" => {
                let response = &event["response"];
                let id:RequestId = serde_json::from_value(response["request_id"].clone()).map_err(|_|error("ID de controle inválido"))?;
                if let Some(waiter) = self.waiters.remove(&id) {
                    let rejected = response["subtype"] == "error";
                    if let Some(wire) = self.wires.get_mut(&waiter.operation_id) { wire.final_result = true; }
                    if !rejected {
                        let result = response.get("response").cloned().unwrap_or_else(||json!({}));
                        match waiter.subtype.as_str() {
                            "initialize" => {
                                self.initialized = true; self.initializing = false; self.init_warning = None;
                                self.commands = result.get("commands").cloned();
                                if self.state.problema.as_deref() == Some("headless_sem_resposta") { self.state.problema = None; self.state.problema_detalhe = None; }
                                if self.effort_intent.as_ref().is_some_and(|intent|intent["status"] == "prepared") { self.dispatch_effort(effects); }
                                else { effects.push(Effect::WakeQueue); }
                            }
                            "set_permission_mode" => {
                                let selected = result["mode"].as_str().or_else(||waiter.payload["mode"].as_str());
                                if let Some(selected) = selected { self.set_mode(selected,effects); }
                            }
                            "set_model" => {
                                self.model = string(&waiter.payload["model"]);
                                self.policy("session.patch_meta",json!({"model":self.model}),effects);
                                if !waiter.timed_out && waiter.payload["effort"].is_string() {
                                    self.request_effort(waiter.operation_id.clone(),&waiter.payload,effects)?;
                                }
                            }
                            _ => {},
                        }
                        if waiter.subtype != "set_model" || !waiter.payload["effort"].is_string() {
                            effects.push(Effect::Reply { operation_id:waiter.operation_id,disposition:Disposition::Accepted,payload:result });
                        } else if waiter.timed_out {
                            effects.push(Effect::Reply { operation_id:waiter.operation_id,disposition:Disposition::Unknown,
                                payload:json!({"phase_resolved":true,"applied":false,"remaining_phase":"effort"}) });
                        }
                    } else {
                        if waiter.subtype == "initialize" {
                            self.initializing = false; self.initialized = false; self.init_warning = None;
                            self.state.problema = Some("headless_nao_subiu".into());
                            self.state.problema_detalhe = Some(response["error"].as_str().unwrap_or("initialize recusado").chars().take(300).collect());
                        }
                        effects.push(Effect::Reply { operation_id:waiter.operation_id,disposition:Disposition::Rejected,
                            payload:json!({"error":response["error"]}) });
                    }
                    self.changed(effects,true);
                }
            }
            "system" => self.on_system(&event,effects),
            "stream_event" => self.on_stream(&event["event"],effects),
            "command_lifecycle" if event["state"] == "started" => { if !self.in_progress { self.start_turn(); self.changed(effects,true); } }
            "assistant" => {
                if !event["local_command_source"].is_null() {
                    let text = text_blocks(&event["message"]["content"]);
                    let effort_source = event["local_command_source"].as_str().is_some_and(|source|source.trim_start_matches('/').split_whitespace().next() == Some("effort"));
                    if let Some(intent) = self.effort_intent.clone().filter(|i|effort_source && (i["status"] == "dispatching" || i["status"] == "unknown")) {
                        let value = intent["value"].as_str().unwrap_or("");
                        let child = format!("{}:effort",intent["operation_id"].as_str().unwrap_or(""));
                        if text.starts_with(&format!("Set effort level to {value}")) && self.wires.contains_key(&child) {
                            self.effort = Some(value.into()); self.effort_intent = None; self.effort_deadline = None;
                            if let Some(wire) = self.wires.get_mut(&child) { wire.final_result = true; }
                            effects.push(Effect::Reply { operation_id:child,disposition:Disposition::Accepted,payload:json!({"applied":true}) });
                            effects.push(Effect::Reply { operation_id:intent["operation_id"].as_str().unwrap_or("").into(),
                                disposition:Disposition::Accepted,payload:json!({"applied":true}) });
                            self.policy("session.patch_meta",json!({"effort":self.effort}),effects);
                        } else if self.wires.contains_key(&child) {
                            self.effort_deadline = None;
                            if let Some(pending) = self.effort_intent.as_mut() { pending["status"] = json!("rejected"); }
                            if let Some(wire) = self.wires.get_mut(&child) { wire.final_result = true; }
                            self.state.problema = Some("headless_turno_erro".into());
                            self.state.problema_detalhe = Some(format!("esforço {value:?} não aceito: {}",text.chars().take(200).collect::<String>()));
                            for operation_id in [child,intent["operation_id"].as_str().unwrap_or("").into()] {
                                effects.push(Effect::Reply { operation_id,disposition:Disposition::Rejected,payload:json!({"applied":false}) });
                            }
                        }
                    }
                    if !text.trim().is_empty() { self.policy("local_output",json!({"text":text}),effects); }
                } else {
                    let usage = &event["message"]["usage"];
                    if ["input_tokens","cache_read_input_tokens","cache_creation_input_tokens"].iter().any(|k|usage[*k].as_u64().unwrap_or(0) > 0) { self.usage = usage.clone(); }
                    if let Some(blocks) = event["message"]["content"].as_array() {
                        for block in blocks {
                            match block["type"].as_str() {
                                Some("text") => { if let Some(text) = self.preview.clear() { self.publish("preview",text,effects); } }
                                Some("thinking") => { if let Some(text) = self.thinking.clear() { self.publish("thinking",text,effects); } }
                                Some("tool_use") => {
                                    self.label = Some(tool_label(block["name"].as_str().unwrap_or("tool"),&block["input"]));
                                    self.clear_tool(effects);
                                    self.tool_name = None;
                                }
                                _ => {},
                            }
                        }
                    }
                }
                self.changed(effects,true);
            }
            "control_request" | "sdk_control_request" => {
                let request_id:RequestId = serde_json::from_value(event["request_id"].clone()).map_err(|_|error("ID de pedido inválido"))?;
                let request = event.get("request").cloned().unwrap_or_else(||json!({}));
                let tool = request["tool_name"].as_str().unwrap_or("");
                if request["subtype"] != "can_use_tool" {
                    self.answer(format!("server:{}:{}",self.generation,self.counter),request_id,json!({}),effects);
                    let subtype = request["subtype"].as_str().unwrap_or("");
                    self.policy("unknown_private",json!({"kind":format!("control_request/{subtype}"),"event":event}),effects);
                    if self.unknown.insert(format!("control_request/{subtype}")) {
                        self.policy("local_output",json!({"text":format!("⚙️ A CLI pediu `{subtype}`; respondi vazio")}),effects);
                    }
                } else if tool == "AskUserQuestion" {
                    self.question = Some(json!({"provider":"claude","request_id":request_id,"questions":request["input"]["questions"]}));
                } else if self.permission_mode.as_deref() == Some("plan") && self.previous_non_plan.as_deref() == Some("bypassPermissions")
                    && !["ExitPlanMode","Edit","Write","MultiEdit","NotebookEdit"].contains(&tool) {
                    self.answer(format!("server:{}:{}",self.generation,self.counter),request_id,
                        json!({"behavior":"allow","updatedInput":request.get("input").cloned().unwrap_or_else(||json!({}))}),effects);
                } else if let Some((_,old)) = self.pending.iter_mut().find(|(id,_)|*id == request_id) { *old = request; }
                else { self.pending.push((request_id,request)); }
                self.changed(effects,true);
            }
            "control_cancel_request" => {
                let id:RequestId = serde_json::from_value(event["request_id"].clone()).map_err(|_|error("ID de cancelamento inválido"))?;
                self.pending.retain(|(request_id,_)|request_id != &id);
                if self.question.as_ref().and_then(|q|serde_json::from_value::<RequestId>(q["request_id"].clone()).ok()) == Some(id) { self.question = None; }
                self.changed(effects,true);
            }
            "result" => {
                self.in_progress = false; self.turn_start = None; self.label_deadline = None; self.label = None;
                self.pending.clear(); self.question = None; self.tasks.clear(); self.restore_plan = None;
                if let Some(id) = self.active_input.take() {
                    if let Some(wire) = self.wires.get_mut(&id) { wire.final_result = true; }
                    effects.push(Effect::Reply { operation_id:id,disposition:Disposition::Accepted,payload:json!({"consumed":true}) });
                }
                let subtype = event["subtype"].as_str().unwrap_or("");
                if subtype != "error_during_execution" && (event["is_error"] == true || subtype.starts_with("error")) {
                    self.state.problema = Some(if event["result"].as_str().unwrap_or("").to_lowercase().contains("not logged in") {
                        "headless_sem_login" } else { "headless_turno_erro" }.into());
                    self.state.problema_detalhe = Some(format!("{subtype}: {}",event["result"].as_str().unwrap_or("").chars().take(300).collect::<String>()));
                } else if subtype == "success" && event["local_command"] != true { self.state.problema = None; self.state.problema_detalhe = None; }
                self.apply_usage(&event,effects);
                self.clear_streams(effects); self.changed(effects,true);
                if self.effort_intent.as_ref().is_some_and(|intent|intent["status"] == "prepared") { self.dispatch_effort(effects); }
                else { effects.push(Effect::WakeQueue); }
            }
            "rate_limit_event" => {
                self.rate_limit_info = event["rate_limit_info"].clone();
                self.state.limited = event["rate_limit_info"]["status"] == "rejected";
                if !self.state.limited { self.state.limit_reset = None; }
                self.policy("format_status",json!({"rate_limit_info":event["rate_limit_info"],"model":self.model,
                    "effort":self.effort,"usage":self.usage,"context_window":self.context_window,"cost":self.cost}),effects);
                self.changed(effects,false);
            }
            "conversation_reset" => { self.reset_conversation(effects); }
            "cano_saiu" => {
                self.alive = false; self.in_progress = false; self.initializing = false;
                self.turn_start = None; self.label_deadline = None; self.init_warning = None; self.effort_deadline = None;
                self.pending.clear(); self.question = None; self.clear_streams(effects);
                self.state.problema = Some("headless_caiu".into());
                for (operation_id,wire) in &mut self.wires {
                    if !wire.final_result {
                        effects.push(Effect::Reply { operation_id:operation_id.clone(),disposition:Disposition::Unknown,payload:json!({"error":"cano encerrado"}) });
                    }
                }
                self.waiters.clear(); self.changed(effects,true);
            }
            "user" => { self.label = None; self.changed(effects,true); }
            "keep_alive" | "tool_progress" | "command_lifecycle" => {},
            _ => {
                self.policy("unknown_private",json!({"kind":kind,"event":event}),effects);
                if self.unknown.insert(kind.into()) { self.policy("local_output",json!({"text":format!("⚙️ Evento desconhecido da CLI: {kind}")}),effects); }
            }
        }
        Ok(())
    }

    fn set_mode(&mut self,selected:&str,effects:&mut Vec<Effect>) {
        let selected = mode(selected);
        if selected != "plan" { self.previous_non_plan = Some(selected.clone()); }
        self.permission_mode = Some(selected);
        self.policy("session.patch_meta",json!({"permission_mode":self.permission_mode,"previous_non_plan":self.previous_non_plan}),effects);
    }

    fn on_system(&mut self,event:&Value,effects:&mut Vec<Effect>) {
        let subtype = event["subtype"].as_str().unwrap_or("");
        if let Some(selected) = event["permissionMode"].as_str() {
            let restore = self.restore_plan.clone();
            self.set_mode(selected,effects);
            if selected != "plan" {
                if let Some((base,id)) = restore {
                    self.restore_plan = None;
                    if self.permission_mode.as_deref() != Some(base.as_str()) { self.control(format!("{id}:restore_mode"),"set_permission_mode",json!({"mode":base}),effects); }
                }
            }
        }
        match subtype {
            "init" => {
                if let Some(sid) = event["session_id"].as_str() {
                    if self.metadata["session_id"].as_str() != Some(sid) {
                        if self.metadata["session_id"].is_string() { self.reset_conversation(effects); }
                        self.metadata["session_id"] = json!(sid); self.clear_streams(effects);
                        self.policy("session.patch_meta",json!({"session_id":sid}),effects);
                    }
                }
                if let Some(model) = event["model"].as_str() { self.model = Some(model.into()); }
                if event["terminal_slash_commands"].is_array() { self.terminal_commands = event["terminal_slash_commands"].clone(); }
            }
            "status" => {
                if event["status"] == "compacting" { self.compacting = true; self.label = Some("Compactando…".into()); }
                else if event["status"].is_null() && event.get("permissionMode").is_none() && self.compacting { self.compacting = false; self.label = None; }
                else if event["status"] == "requesting" && self.in_progress && !self.compacting { self.label = Some("Pensando…".into()); }
            }
            "thinking_tokens" if self.in_progress && !self.compacting => self.label = Some("Pensando…".into()),
            "compact_boundary" => { self.compacting = false; self.label = None; }
            "task_started" | "task_progress" | "task_notification" | "task_updated" => {
                let id = event["task_id"].as_str().map(str::to_owned).unwrap_or_else(||event["task_id"].to_string());
                if subtype == "task_started" {
                    self.tasks.push((id,json!({"kind":event["subagent_type"].as_str().or_else(||event["task_type"].as_str()).unwrap_or("agente"),"step":event["description"]})));
                } else if subtype == "task_progress" {
                    if let Some((_,task)) = self.tasks.iter_mut().find(|(key,_)|*key == id) { task["step"] = event["description"].clone(); }
                } else if event["status"].as_str().or_else(||event["patch"]["status"].as_str())
                    .is_some_and(|s|["completed","failed","killed","cancelled"].contains(&s)) { self.tasks.retain(|(key,_)|key != &id); }
                self.label = self.tasks.last().map(|(_,task)| {
                    let step = task["step"].as_str().unwrap_or("").trim_start_matches("Running ").trim();
                    let label = if step.is_empty() { format!("{}…",task["kind"].as_str().unwrap_or("agente")) }
                        else { format!("{}: {step}",task["kind"].as_str().unwrap_or("agente")) };
                    let label = if self.tasks.len() > 1 { format!("{} agentes · {label}",self.tasks.len()) } else { label };
                    label.chars().take(120).collect()
                });
            }
            _ => return,
        }
        self.changed(effects,true);
    }

    fn on_stream(&mut self,event:&Value,effects:&mut Vec<Effect>) {
        match event["type"].as_str() {
            Some("message_start") => {
                self.tokens_closed += self.tokens_message.unwrap_or(self.token_chars/4);
                self.tokens_message = None; self.token_chars = 0;
                if self.turn_start.is_none() { self.start_turn(); }
                self.changed(effects,true);
            }
            Some("content_block_start") => {
                let block = &event["content_block"];
                match block["type"].as_str() {
                    Some("text") => { if let Some(text) = self.preview.clear() { self.publish("preview",text,effects); } self.label = None; }
                    Some("thinking") => { self.thinking.unblock(); self.thinking_start = Some(self.clock.monotonic_s); self.label = Some("Pensando…".into()); }
                    Some("tool_use" | "server_tool_use" | "mcp_tool_use") => {
                        self.tool_input.clear(); self.tool_name = string(&block["name"]);
                        self.tool_visible = true;
                        self.label = Some(tool_label(self.tool_name.as_deref().unwrap_or("tool"),&json!({})));
                        self.publish("tool",crate::transcript::pyjson::dumps(&json!({"nome":self.tool_name.as_deref().unwrap_or("tool"),"input":{}}),false),effects);
                    }
                    _ => {},
                }
                self.changed(effects,true);
            }
            Some("content_block_delta") => {
                let delta = &event["delta"];
                let piece = delta["text"].as_str().or_else(||delta["thinking"].as_str()).or_else(||delta["partial_json"].as_str()).unwrap_or("");
                self.token_chars += piece.chars().count() as u64;
                match delta["type"].as_str() {
                    Some("text_delta") => { if let Some(text) = self.preview.append(piece,self.clock.monotonic_s) { self.publish("preview",text,effects); } }
                    Some("thinking_delta") => { if let Some(text) = self.thinking.append(piece,self.clock.monotonic_s) { self.publish("thinking",text,effects); } }
                    Some("input_json_delta") if self.tool_name.is_some() => {
                        if let Some(text) = self.tool_input.append(piece,self.clock.monotonic_s) { self.publish_tool(text,effects); }
                    }
                    _ => {},
                }
            }
            Some("content_block_stop") => {
                if let Some(text) = self.preview.flush(self.clock.monotonic_s) { self.publish("preview",text,effects); }
                if let Some(text) = self.thinking.flush(self.clock.monotonic_s) { self.publish("thinking",text,effects); }
                if let Some(text) = self.tool_input.flush(self.clock.monotonic_s) { self.publish_tool(text,effects); }
                self.tool_name = None; self.tool_input.clear();
                if let Some(start) = self.thinking_start.take() { self.thought_s += self.clock.monotonic_s-start; }
            }
            Some("message_delta") => self.tokens_message = event["usage"]["output_tokens"].as_u64(),
            _ => {},
        }
    }

    fn publish_tool(&mut self,text:String,effects:&mut Vec<Effect>) {
        let Some(name) = self.tool_name.clone() else { return };
        let input = partial_input(&text);
        self.label = Some(tool_label(&name,&input));
        self.tool_visible = true;
        self.publish("tool",crate::transcript::pyjson::dumps(&json!({"nome":name,"input":input}),false),effects);
        self.changed(effects,true);
    }

    fn apply_usage(&mut self,result:&Value,effects:&mut Vec<Effect>) {
        if let Some(cost) = result["total_cost_usd"].as_f64() { self.cost = Some(cost); }
        if let Some(models) = result["modelUsage"].as_object() {
            if let Some((model,usage)) = models.iter().filter(|(_,u)|u["contextWindow"].as_u64().is_some())
                .min_by_key(|(_,u)|std::cmp::Reverse(["inputTokens","cacheReadInputTokens","cacheCreationInputTokens"].iter().map(|key|u[*key].as_u64().unwrap_or(0)).sum::<u64>())) {
                self.context_window = usage["contextWindow"].as_u64();
                if self.model.is_none() { self.model = Some(model.clone()); }
                self.policy("session.patch_meta",json!({"context_window":self.context_window}),effects);
            }
        }
    }

    fn on_tick(&mut self,effects:&mut Vec<Effect>) {
        let now = self.clock.monotonic_s;
        if now >= self.reload_deadline {
            self.reload_deadline = now + 10.0;
            self.policy("reload_stamp",json!({}),effects);
        }
        if let Some(text) = self.preview.tick(now) { self.publish("preview",text,effects); }
        if let Some(text) = self.thinking.tick(now) { self.publish("thinking",text,effects); }
        if let Some(text) = self.tool_input.tick(now) { self.publish_tool(text,effects); }
        if self.init_warning.is_some_and(|deadline|now >= deadline) {
            self.init_warning = None;
            self.state.problema = Some("headless_sem_resposta".into()); self.changed(effects,true);
        }
        if self.effort_deadline.is_some_and(|deadline|now >= deadline) {
            self.effort_deadline = None;
            if let Some(intent) = self.effort_intent.as_mut() {
                intent["status"] = json!("unknown");
                let parent = intent["operation_id"].as_str().unwrap_or("").to_owned();
                self.state.problema = Some("headless_turno_erro".into());
                self.state.problema_detalhe = Some("esforço sem confirmação da CLI".into());
                for operation_id in [format!("{parent}:effort"),parent] {
                    effects.push(Effect::Reply { operation_id,disposition:Disposition::Unknown,payload:json!({"applied":false}) });
                }
                self.changed(effects,true);
            }
        }
        for waiter in self.waiters.values_mut() {
            if !waiter.timed_out && now >= waiter.deadline {
                waiter.timed_out = true;
                effects.push(Effect::Reply { operation_id:waiter.operation_id.clone(),disposition:Disposition::Unknown,
                    payload:json!({"error":"controle sem resposta"}) });
            }
        }
        if self.label_deadline.is_some_and(|deadline|now >= deadline) {
            self.label_deadline = self.turn_start.map(|_|now+1.0); self.changed(effects,false);
        }
    }

    pub fn next_deadline(&self) -> Option<f64> {
        [self.preview.deadline(),self.thinking.deadline(),self.tool_input.deadline(),self.init_warning,self.label_deadline,self.effort_deadline,Some(self.reload_deadline)]
            .into_iter().flatten().chain(self.waiters.values().filter(|w|!w.timed_out).map(|w|w.deadline))
            .min_by(f64::total_cmp)
    }
}

fn text_blocks(blocks:&Value) -> String {
    blocks.as_array().map(|blocks|blocks.iter().filter(|b|b["type"] == "text").filter_map(|b|b["text"].as_str())
        .collect::<Vec<_>>().join("\n")).unwrap_or_default()
}

fn partial_input(raw:&str) -> Value {
    if let Ok(value) = serde_json::from_str::<Value>(raw) { return if value.is_object() { value } else { json!({}) }; }
    static FIELDS:std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let fields = FIELDS.get_or_init(||regex::Regex::new(r#""(command|file_path|notebook_path|path|pattern|url|query|description|prompt)"\s*:\s*"((?:[^"\\]|\\.)*)"#).unwrap());
    let mut input = serde_json::Map::new();
    for captures in fields.captures_iter(raw) {
        let value = serde_json::from_str::<Value>(&format!("\"{}\"",&captures[2])).unwrap_or_else(|_|json!(&captures[2]));
        input.entry(captures[1].to_owned()).or_insert(value);
    }
    Value::Object(input)
}

fn tool_label(name:&str,input:&Value) -> String {
    for key in ["command","file_path","notebook_path","path","pattern","url","query","description","prompt"] {
        if let Some(value) = input[key].as_str().map(str::trim).filter(|s|!s.is_empty()) {
            let value = value.lines().next().unwrap_or("");
            let value = if matches!(key,"file_path" | "notebook_path") { value.trim_end_matches(['\\','/']).rsplit(['\\','/']).next().unwrap_or(value) } else { value };
            let value = if value.chars().count() > 80 { format!("{}…",value.chars().take(80).collect::<String>()) } else { value.into() };
            return format!("{name}: {value}");
        }
    }
    format!("{name}…")
}

fn answer_body(question:&Value,answers:&Value) -> Result<Value,RuntimeError> {
    let questions = question["questions"].as_array().ok_or_else(||error("perguntas inválidas"))?;
    let answers = answers.as_array().filter(|answers|answers.len() == questions.len()).ok_or_else(||error("responda a todas as perguntas"))?;
    let mut response = serde_json::Map::new();
    let mut chat = Vec::new();
    for (question,answer) in questions.iter().zip(answers) {
        let title = question["question"].as_str().ok_or_else(||error("pergunta inválida"))?;
        let text = match answer["kind"].as_str() {
            Some("chat") => { chat.push(title); continue; }
            Some("text") => answer["value"].as_str().unwrap_or("").trim().to_owned(),
            Some("option") => {
                let indices = answer["indices"].as_array().ok_or_else(||error("opções inválidas"))?;
                if question["multiSelect"] != true && indices.len() != 1 { return Err(error("escolha uma opção")); }
                let options = question["options"].as_array().ok_or_else(||error("opções inválidas"))?;
                let labels:Result<Vec<_>,_> = indices.iter().map(|index| {
                    index.as_u64().and_then(|index|options.get(index as usize)).and_then(|o|o["label"].as_str()).ok_or_else(||error("opção inválida"))
                }).collect();
                labels?.join(", ")
            }
            _ => return Err(error("responda a todas as perguntas")),
        };
        if text.is_empty() { return Err(error("responda a todas as perguntas")); }
        response.insert(title.into(),json!(text));
    }
    if chat.is_empty() { Ok(json!({"behavior":"allow","updatedInput":{"questions":questions,"answers":response}})) }
    else {
        let mut lines = if chat.len() == 1 { vec![format!("Sobre «{}» ele prefere conversar antes de responder.",chat[0])] }
            else { let mut lines = vec!["Sobre estas perguntas ele prefere conversar antes de responder:".into()]; lines.extend(chat.iter().map(|q|format!("- {q}"))); lines };
        if !response.is_empty() { lines.push("Ele já respondeu:".into()); lines.extend(response.iter().map(|(q,a)|format!("- {q} → {}",a.as_str().unwrap_or("")))); }
        lines.push("Não repita a pergunta: responda em texto e aguarde a mensagem dele.".into());
        Ok(json!({"behavior":"deny","message":lines.join("\n")}))
    }
}
