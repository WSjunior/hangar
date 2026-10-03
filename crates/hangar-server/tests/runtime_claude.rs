use hangar_server::runtime::{claude::ClaudeEngine, protocol::*};
use serde_json::{Value, json};

#[test]
fn model_effort_intent_is_not_an_extra_cli_model_field() {
    let mut engine = ClaudeEngine::new(json!({"name":"session","session_id":"sid","initialized":true}),1,
        ClockSample { monotonic_s:0.0,epoch_s:1_800_000_000.0 });
    let effects = engine.command(RuntimeCommand { operation_id:"model".into(),kind:OperationKind::SetModel,
        payload:json!({"model":"test-model","effort":"high"}) },ClockSample { monotonic_s:1.0,epoch_s:1_800_000_001.0 }).unwrap();
    let request = effects.iter().find_map(|effect|match effect { Effect::Write { frame,.. }=>Some(&frame["request"]),_=>None }).unwrap();
    assert_eq!(request,&json!({"subtype":"set_model","model":"test-model"}));
}

#[test]
fn hydrated_reader_requests_usage_and_reload_stamp_without_viewers() {
    let mut engine = engine(json!({"name":"session","session_id":"sid","initialized":true}));
    let snapshot = CanoSnapshot::parse(json!({"type":"cano_snapshot","versao":2,"pid":42,"init":null,
        "aberto":false,"pendentes":[],"ultimo_result":null,"rate_limit":null,"stderr_tail":[],"saiu":null,"inflight":{}})).unwrap();
    let effects = engine.hydrate(snapshot).unwrap();
    for service in ["last_usage","reload_stamp"] {
        assert!(effects.iter().any(|effect|matches!(effect,Effect::Policy { kind,.. } if kind == service)));
    }
}

fn clock(seconds:f64) -> ClockSample { ClockSample { monotonic_s:seconds,epoch_s:1_800_000_000.0 + seconds } }
fn engine(metadata:Value) -> ClaudeEngine { ClaudeEngine::new(metadata,1,clock(10.0)) }
fn line(engine:&mut ClaudeEngine,value:Value,seconds:f64) -> Vec<Effect> { engine.apply(EngineInput::Line(value),clock(seconds)).unwrap() }
fn command(kind:OperationKind,payload:Value) -> RuntimeCommand { RuntimeCommand { operation_id:"op-1".into(),kind,payload } }
fn writes(effects:&[Effect]) -> Vec<Value> { effects.iter().filter_map(|e|match e { Effect::Write { frame,.. } => Some(frame.clone()),_=>None }).collect() }

#[test]
fn init_timeout_not_deliverable() {
    let mut engine = engine(json!({"name":"session"}));
    let effects = engine.start_initialize("init".into()).unwrap();
    let request_id = writes(&effects)[0]["request_id"].clone();
    let timeout = engine.apply(EngineInput::Tick,clock(190.0)).unwrap();
    assert!(timeout.iter().any(|e|matches!(e,Effect::Reply { disposition:Disposition::Unknown,.. })));
    assert_eq!(engine.view()["iniciando"],true);
    assert_eq!(engine.view()["deliverable"],false);
    line(&mut engine,json!({"type":"control_response","response":{
        "request_id":request_id,"subtype":"success","response":{"commands":[]}}}),191.0);
    assert_eq!(engine.view()["iniciando"],false);
    assert_eq!(engine.view()["deliverable"],true);
}

#[test]
fn pending_plan_rules() {
    let mut engine = engine(json!({"name":"session","permission_mode":"plan","previous_non_plan":"bypassPermissions","initialized":true}));
    let read = line(&mut engine,json!({"type":"control_request","request_id":1,
        "request":{"subtype":"can_use_tool","tool_name":"Read","input":{}}}),10.0);
    assert_eq!(writes(&read)[0]["response"]["response"]["behavior"],"allow");
    for (index,tool) in ["ExitPlanMode","Edit","Write","MultiEdit","NotebookEdit"].into_iter().enumerate() {
        let effects = line(&mut engine,json!({"type":"control_request","request_id":index+2,
            "request":{"subtype":"can_use_tool","tool_name":tool,"input":{}}}),10.0);
        assert!(writes(&effects).is_empty());
    }
    assert_eq!(engine.view()["pending"].as_array().unwrap().len(),5);
}

#[test]
fn question_validates_current_request() {
    let mut engine = engine(json!({"name":"session","initialized":true}));
    line(&mut engine,json!({"type":"control_request","request_id":1,"request":{
        "subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{
        "question":"Qual opção?","header":"Opção","multiSelect":true,
        "options":[{"label":"A","description":"Primeira"},{"label":"B","description":"Segunda"}]}]}}}),10.0);
    assert!(engine.command(command(OperationKind::AnswerQuestions,json!({"request_id":"1","answers":[]})),clock(10.0)).is_err());
    let effects = engine.command(command(OperationKind::AnswerQuestions,json!({"request_id":1,"answers":[{"kind":"option","indices":[0,1]}]})),clock(10.0)).unwrap();
    assert_eq!(writes(&effects)[0]["response"]["response"]["updatedInput"]["answers"]["Qual opção?"],"A, B");
}

#[test]
fn interrupt_denies_all_pending_first() {
    let mut engine = engine(json!({"name":"session","initialized":true}));
    for id in [1,2] { line(&mut engine,json!({"type":"control_request","request_id":id,
        "request":{"subtype":"can_use_tool","tool_name":"Edit","input":{}}}),10.0); }
    let effects = engine.command(command(OperationKind::Interrupt,json!({})),clock(10.0)).unwrap();
    let frames = writes(&effects);
    assert_eq!(frames.len(),3);
    assert_eq!(frames[0]["response"]["response"]["behavior"],"deny");
    assert_eq!(frames[1]["response"]["response"]["behavior"],"deny");
    assert_eq!(frames[2]["request"]["subtype"],"interrupt");
}

#[test]
fn subagent_does_not_close_parent() {
    let mut engine = engine(json!({"name":"session","initialized":true}));
    line(&mut engine,json!({"type":"command_lifecycle","state":"started"}),10.0);
    line(&mut engine,json!({"type":"result","subtype":"success","parent_tool_use_id":"child"}),11.0);
    assert_eq!(engine.view()["in_progress"],true);
}

#[test]
fn usage_last_call_is_context() {
    let mut engine = engine(json!({"name":"session","initialized":true}));
    line(&mut engine,json!({"type":"assistant","message":{"content":[],"usage":{"input_tokens":2,"cache_read_input_tokens":39000,"output_tokens":4}}}),10.0);
    line(&mut engine,json!({"type":"result","subtype":"success","usage":{"input_tokens":500000}}),11.0);
    line(&mut engine,json!({"type":"assistant","message":{"content":[],"usage":{"input_tokens":0}}}),12.0);
    assert_eq!(engine.view()["usage"]["cache_read_input_tokens"],39000);
}

#[test]
fn first_150ms_last_and_clear() {
    let mut engine = engine(json!({"name":"session","initialized":true}));
    let mut publications = Vec::new();
    for (time,event) in [(10.0,json!({"type":"content_block_delta","delta":{"type":"text_delta","text":"Olá "}})),
        (10.1,json!({"type":"content_block_delta","delta":{"type":"text_delta","text":"mundo"}}))] {
        publications.extend(line(&mut engine,json!({"type":"stream_event","event":event}),time));
    }
    publications.extend(engine.apply(EngineInput::Tick,clock(10.15)).unwrap());
    publications.extend(line(&mut engine,json!({"type":"assistant","message":{"content":[{"type":"text","text":"Olá mundo"}]}}),10.2));
    publications.extend(engine.apply(EngineInput::Tick,clock(10.3)).unwrap());
    let texts:Vec<_> = publications.into_iter().filter_map(|e|match e {
        Effect::Publish { channel,data } if channel == "preview" => Some(data["text"].clone()),_=>None }).collect();
    assert_eq!(texts,vec![json!("Olá "),json!("Olá mundo"),json!("")]);
}

#[test]
fn unknown_control_is_neutral() {
    let mut engine = engine(json!({"name":"session"}));
    let effects = line(&mut engine,json!({"type":"control_request","request_id":"future",
        "request":{"subtype":"future_method"}}),10.0);
    assert_eq!(writes(&effects)[0]["response"]["response"],json!({}));
    assert!(effects.iter().any(|e|matches!(e,Effect::Policy { kind,.. } if kind == "unknown_private")));
}

#[test]
fn clear_discards_old_control_and_queued_write() {
    let mut engine = engine(json!({"name":"session", "initialized":true, "session_id":"before"}));
    let effects = engine.command(command(OperationKind::SetModel,json!({"model":"next"})),clock(10.0)).unwrap();
    let request_id = writes(&effects)[0]["request_id"].clone();
    let reset = line(&mut engine,json!({"type":"system", "subtype":"init", "session_id":"after"}),11.0);
    assert!(reset.iter().any(|effect|matches!(effect,Effect::Reply { disposition:Disposition::Unknown,.. })));
    assert!(!engine.write_is_current("op-1"));
    line(&mut engine,json!({"type":"control_response", "response":{"request_id":request_id,
        "subtype":"success", "response":{}}}),12.0);
    assert_ne!(engine.view()["model"], "next");
    assert_eq!(engine.view()["conversation"], "after");
}

#[test]
fn effort_is_deferred_until_a_safe_boundary() {
    let mut engine = engine(json!({"name":"session","initialized":true,"effort":"medium"}));
    line(&mut engine,json!({"type":"command_lifecycle","state":"started"}),10.0);
    let effects = engine.command(command(OperationKind::SetEffort,json!({"effort":"high"})),clock(11.0)).unwrap();
    assert!(writes(&effects).is_empty());
    assert_eq!(engine.view()["effort_intent"]["operation_id"],"op-1");
    let effects = line(&mut engine,json!({"type":"result","subtype":"success"}),12.0);
    assert!(effects.iter().any(|e|matches!(e,Effect::Write { operation_id:Some(id),.. } if id == "op-1:effort")));
    assert_eq!(engine.view()["effort"],"medium");
    line(&mut engine,json!({"type":"assistant","local_command_source":"/effort","message":{
        "content":[{"type":"text","text":"Set effort level to high (this session only)"}]}}),13.0);
    assert_eq!(engine.view()["effort"],"high");
}

#[test]
fn late_ack_does_not_override_result() {
    let mut engine = engine(json!({"name":"session"}));
    let effects = engine.start_initialize("init".into()).unwrap();
    let request_id = writes(&effects)[0]["request_id"].clone();
    line(&mut engine,json!({"type":"control_response","response":{
        "request_id":request_id,"subtype":"success","response":{}}}),11.0);
    let effects = engine.apply(EngineInput::WriteAck { operation_id:"init".into(),outcome:WriteOutcome::Unknown },clock(12.0)).unwrap();
    assert!(!effects.iter().any(|e|matches!(e,Effect::Reply { disposition:Disposition::Unknown,.. })));
    assert_eq!(engine.view()["deliverable"],true);
}
