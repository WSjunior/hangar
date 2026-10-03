use hangar_server::runtime::{codex::Engine,protocol::*};
use serde_json::{Value,json};

fn clock(seconds:f64) -> ClockSample { ClockSample { monotonic_s:seconds,epoch_s:1_800_000_000.0 + seconds } }
fn engine() -> Engine { Engine::new(json!({"name":"session","thread_id":"thread-1","headless":true,"model":"gpt-6","initialized":true,"ready":true}),1,clock(10.0)) }
fn frames(effects:&[Effect]) -> Vec<Value> { effects.iter().filter_map(|e|match e { Effect::Write { frame,.. }=>Some(frame.clone()),_=>None }).collect() }
fn command(kind:OperationKind,payload:Value) -> RuntimeCommand { RuntimeCommand { operation_id:"op-1".into(),kind,payload } }
fn line(engine:&mut Engine,value:Value,time:f64) -> Vec<Effect> { engine.apply(EngineInput::Line(value),clock(time)).unwrap() }

#[test]
fn initialize_then_resume() {
    let mut engine = Engine::new(json!({"name":"session","thread_id":"thread-1","headless":true}),1,clock(10.0));
    let effects = engine.bootstrap(true,"boot".into()).unwrap();
    let initialize = frames(&effects)[0].clone();
    assert_eq!(initialize["method"],"initialize");
    let effects = line(&mut engine,json!({"id":initialize["id"],"result":{}}),11.0);
    let requests = frames(&effects);
    assert_eq!(requests[0]["method"],"initialized");
    assert_eq!(requests[1]["method"],"thread/resume");
    assert_eq!(requests[1]["params"]["threadId"],"thread-1");
}

#[test]
fn reply_ids_and_generations() {
    let mut engine = engine();
    let effects = engine.command(command(OperationKind::ListModels,json!({})),clock(10.0)).unwrap();
    let id = frames(&effects)[0]["id"].clone();
    assert!(id.as_str().unwrap().starts_with("hangar:1:"));
    let old = line(&mut engine,json!({"id":"hangar:0:1","result":{"data":[]}}),11.0);
    assert!(!old.iter().any(|e|matches!(e,Effect::Reply { .. })));
    let current = line(&mut engine,json!({"id":id,"result":{"data":[]}}),12.0);
    assert!(current.iter().any(|e|matches!(e,Effect::Reply { disposition:Disposition::Accepted,.. })));
}

#[test]
fn rpc_timeout_keeps_pending() {
    let mut engine = engine();
    let effects = engine.command(command(OperationKind::Input,json!({"text":"Olá"})),clock(10.0)).unwrap();
    let id = frames(&effects)[0]["id"].clone();
    let timeout = engine.apply(EngineInput::Tick,clock(40.0)).unwrap();
    assert!(timeout.iter().any(|e|matches!(e,Effect::Reply { disposition:Disposition::Unknown,.. })));
    let reply = line(&mut engine,json!({"id":id,"result":{"turn":{"id":"turn-1"}}}),41.0);
    assert!(reply.iter().any(|e|matches!(e,Effect::Reply { disposition:Disposition::Accepted,.. })));
    assert!(frames(&reply).is_empty());
}

#[test]
fn server_request_once() {
    let mut engine = engine();
    let request = json!({"id":1,"method":"item/commandExecution/requestApproval","params":{"threadId":"thread-1","command":"pwd"}});
    line(&mut engine,request.clone(),10.0);
    let response = engine.command(command(OperationKind::Select,json!({"option":1})),clock(10.0)).unwrap();
    assert_eq!(frames(&response)[0]["id"],1);
    assert!(engine.command(command(OperationKind::Select,json!({"option":1})),clock(10.0)).is_err());
    let effects = line(&mut engine,json!({"id":"unknown","method":"future/request","params":{"threadId":"thread-1"}}),11.0);
    assert_eq!(frames(&effects)[0]["error"]["code"],-32601);
}

#[test]
fn turn_end_idle_before_drain() {
    let mut engine = engine();
    line(&mut engine,json!({"method":"turn/started","params":{"threadId":"thread-1","turn":{"id":"turn-1"}}}),10.0);
    let effects = line(&mut engine,json!({"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"completed"}}}),11.0);
    assert_eq!(engine.view()["state"],"idle");
    let state = effects.iter().position(|e|matches!(e,Effect::StateChanged)).unwrap();
    let drain = effects.iter().position(|e|matches!(e,Effect::WakeQueue)).unwrap();
    assert!(state < drain);
}

#[test]
fn thread_switch_clears_old_preview_and_foreign_deltas_are_ignored() {
    let mut engine = engine();
    let effects = line(&mut engine,json!({"method":"item/agentMessage/delta","params":{"threadId":"other","delta":"filho"}}),10.0);
    assert!(!effects.iter().any(|e|matches!(e,Effect::Publish { .. })));
}

#[test]
fn sandbox_requires_idle() {
    let mut engine = engine();
    line(&mut engine,json!({"method":"turn/started","params":{"threadId":"thread-1","turn":{"id":"turn-1"}}}),10.0);
    assert!(engine.command(command(OperationKind::SetPermissionMode,json!({"mode":"Ask for approval"})),clock(10.0)).is_err());
}

#[test]
fn terminal_adapter_untouched() {
    let mut engine = Engine::new(json!({"name":"session","headless":false}),1,clock(10.0));
    assert!(engine.bootstrap(true,"boot".into()).is_err());
}

#[test]
fn question_hydrate_merges_notifications() {
    let mut engine = engine();
    let read = engine.command(command(OperationKind::ReadSettings,json!({"include_turns":true})),clock(10.0)).unwrap();
    let id = frames(&read)[0]["id"].clone();
    line(&mut engine,json!({"method":"item/completed","params":{"threadId":"thread-1","item":{
        "id":"question-new","type":"agentMessage","delivery":"async","questions":[{"title":"Nova pergunta","options":["A","B"]}]}}}),11.0);
    line(&mut engine,json!({"id":id,"result":{"thread":{"id":"thread-1","status":{"type":"idle"},"turns":[]}}}),12.0);
    assert_eq!(engine.view()["codex_question"]["questions"][0]["question"],"Nova pergunta");
    line(&mut engine,json!({"method":"item/completed","params":{"threadId":"thread-1","item":{
        "id":"ordinary-user","type":"userMessage","content":[{"type":"text","text":"> Nova pergunta\n\nA"}]}}}),13.0);
    assert_eq!(engine.view()["codex_question"]["questions"][0]["question"],"Nova pergunta");
}

#[test]
fn preview_full_prefix_on_takeover() {
    let mut engine = engine();
    let snapshot = CanoSnapshot::parse(json!({"type":"cano_snapshot","versao":2,"pid":42,"init":null,
        "aberto":false,"pendentes":[],"ultimo_result":null,"rate_limit":null,"stderr_tail":[],"saiu":null,
        "inflight":{"codex":{"thread-1":{"complete":true,"text":"Olá 🌎","itemId":"item-1","turnId":"turn-1"}}}})).unwrap();
    let effects = engine.hydrate(snapshot).unwrap();
    let text = effects.into_iter().find_map(|e|match e { Effect::Publish { channel,data } if channel == "preview"=>Some(data["text"].clone()),_=>None }).unwrap();
    assert_eq!(text,"Olá 🌎");
    assert_eq!(engine.control_view()["turn_id"],"turn-1");
}

#[test]
fn late_reply_does_not_reopen_a_completed_turn() {
    let mut engine = engine();
    let effects = engine.command(command(OperationKind::Input,json!({"text":"Olá"})),clock(10.0)).unwrap();
    let id = frames(&effects)[0]["id"].clone();
    line(&mut engine,json!({"method":"turn/started","params":{"threadId":"thread-1","turn":{"id":"turn-1"}}}),11.0);
    line(&mut engine,json!({"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"completed"}}}),12.0);
    line(&mut engine,json!({"id":id,"result":{"turn":{"id":"turn-1"}}}),13.0);
    assert_eq!(engine.view()["state"],"idle");
    assert_eq!(engine.control_view()["in_progress"],false);
}

#[test]
fn late_ack_does_not_remove_reused_server_request() {
    let mut engine = engine();
    let approval = json!({"id":1,"method":"item/commandExecution/requestApproval","params":{"threadId":"thread-1","command":"pwd"}});
    line(&mut engine,approval.clone(),10.0);
    engine.command(command(OperationKind::Select,json!({"option":1})),clock(10.0)).unwrap();
    line(&mut engine,json!({"method":"serverRequest/resolved","params":{"threadId":"thread-1","requestId":1}}),11.0);
    line(&mut engine,approval,12.0);
    engine.apply(EngineInput::WriteAck { operation_id:"op-1".into(),outcome:WriteOutcome::Written },clock(13.0)).unwrap();
    assert_eq!(engine.control_view()["pending"].as_array().unwrap().len(),1);
}

#[test]
fn voice_organizer_uses_same_writer_without_changing_target_thread() {
    let mut engine = engine();
    engine.command(RuntimeCommand { operation_id:"voice-open".into(),kind:OperationKind::VoiceOpen,payload:json!({"call_id":"call-1"}) },clock(10.0)).unwrap();
    let effects = engine.command(RuntimeCommand { operation_id:"voice-thread".into(),kind:OperationKind::VoiceRpc,
        payload:json!({"call_id":"call-1","method":"thread/start","params":{"ephemeral":true,"sandbox":"read-only","approvalPolicy":"never"}}) },clock(11.0)).unwrap();
    let id = frames(&effects)[0]["id"].clone();
    line(&mut engine,json!({"id":id,"result":{"thread":{"id":"organizer-1"}}}),12.0);
    assert_eq!(engine.control_view()["thread_id"],"thread-1");
    let effects = line(&mut engine,json!({"id":7,"method":"item/tool/call","params":{"threadId":"organizer-1","name":"draft"}}),13.0);
    assert!(effects.iter().any(|effect|matches!(effect,Effect::Publish { channel,data } if channel == "voice" && data["call_id"] == "call-1")));
    assert!(!effects.iter().any(|effect|matches!(effect,Effect::Write { .. })));
    assert!(engine.control_view()["pending"].as_array().unwrap().is_empty());
    assert!(engine.command(RuntimeCommand { operation_id:"wrong-id".into(),kind:OperationKind::VoiceRespond,
        payload:json!({"call_id":"call-1","request_id":"7","result":{}}) },clock(14.0)).is_err());
    assert!(engine.command(RuntimeCommand { operation_id:"wrong-thread".into(),kind:OperationKind::VoiceRpc,
        payload:json!({"call_id":"call-1","method":"turn/start","params":{"threadId":"thread-1","input":[]}}) },clock(15.0)).is_err());
}

#[test]
fn voice_initialize_is_virtual_and_never_sends_another_initialize() {
    let mut engine = engine();
    engine.command(RuntimeCommand { operation_id:"open".into(),kind:OperationKind::VoiceOpen,payload:json!({"call_id":"call"}) },clock(10.0)).unwrap();
    let effects = engine.command(RuntimeCommand { operation_id:"init".into(),kind:OperationKind::VoiceRpc,
        payload:json!({"call_id":"call","method":"initialize","params":{}}) },clock(11.0)).unwrap();
    assert!(frames(&effects).is_empty());
    assert!(effects.iter().any(|effect|matches!(effect,Effect::Reply { disposition:Disposition::Accepted,.. })));
}

#[test]
fn slash_skill_waits_for_catalog_before_starting_turn() {
    let mut engine = engine();
    let effects = engine.command(command(OperationKind::Input,json!({"text":"/skill Olá", "skill_name":"skill",
        "input":[{"type":"text","text":"/skill Olá"}]})),clock(10.0)).unwrap();
    let read = frames(&effects)[0].clone();
    assert_eq!(read["method"],"skills/list");
    let effects = line(&mut engine,json!({"id":read["id"],"result":{"data":[]}}),11.0);
    let (request_id,_) = effects.iter().find_map(|effect|match effect {
        Effect::Policy { kind,request_id,payload } if kind == "skill_catalog"=>Some((request_id.clone(),payload.clone())),_=>None }).unwrap();
    assert!(frames(&effects).is_empty());
    let effects = engine.apply(EngineInput::PolicyResult { request_id,payload:json!({"skill":{"native_name":"skill","path":"/fake/skill"}}) },clock(12.0)).unwrap();
    let input = frames(&effects)[0].clone();
    assert_eq!(input["method"],"turn/start");
    assert_eq!(input["params"]["input"][1],json!({"type":"skill","name":"skill","path":"/fake/skill"}));
}

#[test]
fn organizer_request_before_thread_reply_is_preserved_once() {
    let mut engine = engine();
    engine.command(RuntimeCommand { operation_id:"open".into(),kind:OperationKind::VoiceOpen,payload:json!({"call_id":"call"}) },clock(10.0)).unwrap();
    let effects = engine.command(RuntimeCommand { operation_id:"start".into(),kind:OperationKind::VoiceRpc,
        payload:json!({"call_id":"call","method":"thread/start","params":{"ephemeral":true,"sandbox":"read-only","approvalPolicy":"never"}}) },clock(11.0)).unwrap();
    let id = frames(&effects)[0]["id"].clone();
    let request = json!({"id":7,"method":"item/tool/call","params":{"threadId":"organizer","name":"draft"}});
    assert!(frames(&line(&mut engine,request.clone(),11.1)).is_empty());
    let effects = line(&mut engine,json!({"id":id,"result":{"thread":{"id":"organizer"}}}),12.0);
    assert_eq!(effects.iter().filter(|effect|matches!(effect,Effect::Publish { channel,.. } if channel == "voice")).count(),1);
    let duplicate = line(&mut engine,request,12.1);
    assert!(!duplicate.iter().any(|effect|matches!(effect,Effect::Publish { channel,.. } if channel == "voice")));
}
