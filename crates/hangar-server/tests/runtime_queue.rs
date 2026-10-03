use hangar_server::runtime::{protocol::ClockSample, queue::*};
use serde_json::json;

fn clock() -> ClockSample { ClockSample { monotonic_s: 10.0, epoch_s: 1_800_000_000.0 } }
fn append() -> Action { Action::Append { text:"Olá".into(), delivered:false, ts:None,
    pre_transcript:false, entry_id:Some("entry-1".into()) } }

#[test]
fn same_operation_does_not_append_twice() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("key.queue-state.json"), &dir.path().join("projection"),
        State::new("key", 1, "session", vec![])).unwrap();
    let first = store.exec(1, "call-1", clock(), append()).unwrap();
    assert_eq!(store.exec(1, "call-1", clock(), append()).unwrap(), first);
    assert_eq!(store.state().rows.len(), 1);
    assert!(store.exec(1, "call-1", clock(), Action::Append { text:"Outro".into(), delivered:false,
        ts:None, pre_transcript:false, entry_id:Some("entry-1".into()) }).is_err());
}

#[test]
fn state_before_projection() {
    let dir = tempfile::tempdir().unwrap();
    let projection = dir.path().join("projection");
    let path = dir.path().join("key.queue-state.json");
    let mut store = Store::open(&path, &projection, State::new("key",1,"session",vec![])).unwrap();
    std::fs::remove_file(projection.join("session.jsonl")).unwrap();
    std::fs::create_dir(projection.join("session.jsonl")).unwrap();
    assert!(store.exec(1, "append", clock(), append()).is_err());
    let state: State = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(state.rows.len(), 1);
    std::fs::remove_dir(projection.join("session.jsonl")).unwrap();
    assert_eq!(store.exec(1,"append",clock(),append()).unwrap()["id"], "entry-1");
    assert_eq!(store.state().rows.len(), 1);
}

#[test]
fn corrupt_state_is_not_empty_queue() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("key.queue-state.json");
    std::fs::write(&path, "{").unwrap();
    assert!(Store::open(&path,dir.path(),State::new("key",1,"session",vec![])).is_err());
}

#[test]
fn unknown_never_unclaims() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("state"), dir.path(), State::new("key",1,"session",vec![])).unwrap();
    store.exec(1,"append",clock(),append()).unwrap();
    store.exec(1,"prepare",clock(),Action::Prepare { id:"op".into(),payload:json!({}),entry_id:Some("entry-1".into()) }).unwrap();
    store.exec(1,"dispatch",clock(),Action::BeginDispatch { id:"op".into(),wire_id:"wire:op:1".into() }).unwrap();
    store.exec(1,"unknown",clock(),Action::Finish { id:"op".into(),status:Status::Unknown,result:json!(null) }).unwrap();
    assert!(store.exec(1,"unclaim",clock(),Action::SetDelivered { entry_id:"entry-1".into(),value:false,steered:false }).is_err());
    assert_eq!(store.state().rows[0]["delivered"],true);
}

#[test]
fn cap_keeps_pending() {
    let dir = tempfile::tempdir().unwrap();
    let rows = (0..1000).map(|i|json!({"id":i.to_string(),"text":"pending","ts":1,"delivered":true,"desistiu":true})).collect();
    let mut store = Store::open(&dir.path().join("state"),dir.path(),State::new("key",1,"session",rows)).unwrap();
    assert!(store.exec(1,"append",clock(),append()).is_err());
    assert_eq!(store.state().rows.len(),1000);
}

#[test]
fn rename_keeps_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("key.queue-state.json");
    let projection = dir.path().join("projection");
    let mut store = Store::open(&path,&projection,State::new("key",1,"session",vec![])).unwrap();
    store.exec(1,"append",clock(),append()).unwrap();
    store.exec(1,"rename",clock(),Action::Rename { name:"renamed".into() }).unwrap();
    assert_eq!(store.state().owner_key,"key");
    assert!(path.exists());
    assert!(!projection.join("session.jsonl").exists());
    assert!(projection.join("renamed.jsonl").exists());
}

#[test]
fn unknown_reply_cannot_downgrade_final() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("state"),dir.path(),State::new("key",1,"session",vec![])).unwrap();
    store.exec(1,"prepare",clock(),Action::Prepare { id:"op".into(),payload:json!({}),entry_id:None }).unwrap();
    store.exec(1,"accepted",clock(),Action::Finish { id:"op".into(),status:Status::Accepted,result:json!({"ok":true}) }).unwrap();
    store.exec(1,"late",clock(),Action::Finish { id:"op".into(),status:Status::Unknown,result:json!(null) }).unwrap();
    assert!(store.state().operations["op"].status == Status::Accepted);
    assert_eq!(store.state().operations["op"].result,json!({"ok":true}));
}

#[test]
fn late_reply_matches_generation_and_type() {
    use hangar_server::runtime::protocol::RequestId;
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("state"),dir.path(),State::new("key",1,"session",vec![])).unwrap();
    store.exec(1,"prepare",clock(),Action::Prepare { id:"op".into(),payload:json!({"request_id":1}),entry_id:None }).unwrap();
    store.exec(1,"begin",clock(),Action::BeginDispatch { id:"op".into(),wire_id:"wire:1".into() }).unwrap();
    for (generation,request_id) in [(2,RequestId::Integer(1)),(1,RequestId::String("1".into()))] {
        assert!(store.exec(1,"late",clock(),Action::LateRpcResolution { id:"op".into(),wire_id:"wire:1".into(),
            request_id,generation,result:json!({"ok":true}) }).is_err());
    }
    assert!(store.state().operations["op"].status == Status::Dispatching);
}

#[test]
fn terminal_runtime_store_recovery_only_unclaims_proved_unsent_terminal_claim() {
    for (case,root_status,side_effect,terminal_claim,expected) in [
        ("safe",None,false,true,false),
        ("prepared",Some(Status::Prepared),false,true,false),
        ("unknown",Some(Status::Unknown),false,true,true),
        ("dispatching",Some(Status::Dispatching),false,true,true),
        ("accepted",Some(Status::Accepted),false,true,true),
        ("confirmed",Some(Status::Confirmed),false,true,true),
        ("side_effect",Some(Status::Prepared),true,true,true),
        ("legacy",None,false,false,true),
    ] {
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("state"); let projection=dir.path().join("projection");
        let mut store=Store::open(&path,&projection,State::new("key",1,"session",vec![])).unwrap();
        let clock=ClockSample {monotonic_s:0.0,epoch_s:1800000000.0};
        store.exec(1,"append",clock,Action::Append {text:"Olá".into(),delivered:false,ts:None,pre_transcript:false,entry_id:Some("entry".into())}).unwrap();
        store.exec(1,if terminal_claim{"terminal:queue:999"}else{"legacy-claim"},clock,Action::Claim {min_ts:1.0,limit:Some(1),entry_id:None}).unwrap();
        if let Some(status)=root_status {
            store.exec(1,"root",clock,Action::Prepare {id:"root".into(),entry_id:Some("entry".into()),payload:json!({"kind":"input"})}).unwrap();
            if status==Status::Dispatching {store.exec(1,"dispatch",clock,Action::BeginDispatch {id:"root".into(),wire_id:"wire".into()}).unwrap();}
            else if status!=Status::Prepared {store.exec(1,"finish",clock,Action::Finish {id:"root".into(),status,result:json!({})}).unwrap();}
        }
        if side_effect {
            store.exec(1,"phase",clock,Action::Prepare {id:"phase".into(),entry_id:None,payload:json!({"logical_id":"root"})}).unwrap();
            store.exec(1,"phase-dispatch",clock,Action::BeginDispatch {id:"phase".into(),wire_id:"rpc".into()}).unwrap();
        }
        drop(store); let mut store=Store::open(&path,&projection,State::new("key",1,"session",vec![])).unwrap();
        store.exec(1,"recover",clock,Action::Recover).unwrap(); assert_eq!(store.state().rows[0]["delivered"],expected,"{case}");
    }
}

#[test]
fn terminal_runtime_finish_is_atomic_and_recover_does_not_repeat_retry_accounting() {
    for claimed in [false,true] {for attempts in [0,1,2] {for rejected in [false,true] {
        let dir=tempfile::tempdir().unwrap(); let path=dir.path().join("state"); let projection=dir.path().join("projection");
        let mut store=Store::open(&path,&projection,State::new("key",1,"session",vec![])).unwrap(); let clock=ClockSample {monotonic_s:0.0,epoch_s:1800000000.0};
        store.exec(1,"append",clock,Action::Append {text:"Olá".into(),delivered:false,ts:None,pre_transcript:true,entry_id:Some("entry".into())}).unwrap();
        for n in 0..attempts {store.exec(1,&format!("bump:{n}"),clock,Action::BumpAttempts {entry_id:"entry".into()}).unwrap();}
        if claimed {store.exec(1,"terminal:queue:999",clock,Action::Claim {min_ts:1.0,limit:Some(1),entry_id:None}).unwrap();}
        store.exec(1,"terminal:queue:1000",clock,Action::Prepare {id:"attempt".into(),entry_id:Some("entry".into()),payload:json!({"operation_id":"attempt","kind":"input","payload":{"text":"Olá","pre_transcript":true,"_terminal_generation":1}})}).unwrap();
        store.exec(1,"dispatch",clock,Action::BeginDispatch {id:"attempt".into(),wire_id:"terminal:1:attempt".into()}).unwrap();
        let status=if rejected{Status::Rejected}else{Status::Deferred}; let result=json!({"operation_id":"attempt","disposition":if rejected{"rejected"}else{"deferred"},"payload":{"cleanup":"proved"}});
        store.exec(1,"finish",clock,Action::Finish {id:"attempt".into(),status,result:result.clone()}).unwrap();
        let expected_abandoned=rejected||attempts==2; assert_eq!(store.state().rows[0]["delivered"],expected_abandoned,"claimed={claimed} attempts={attempts} rejected={rejected}");
        if expected_abandoned {assert_eq!(store.state().rows[0]["desistiu"],true);} else {assert_eq!(store.state().rows[0]["attempts"],attempts+1);}
        let row=store.state().rows[0].clone(); drop(store);
        let mut store=Store::open(&path,&projection,State::new("key",1,"session",vec![])).unwrap(); store.exec(1,"recover1",clock,Action::Recover).unwrap();
        store.exec(1,"finish-new-id",clock,Action::Finish {id:"attempt".into(),status,result}).unwrap(); store.exec(1,"recover2",clock,Action::Recover).unwrap(); assert_eq!(store.state().rows[0],row);
    }}}
}

#[test]
fn terminal_runtime_recover_finishes_legacy_partial_row_transition_once() {
    for claimed in [false,true] {for attempts in [0,1,2] {for rejected in [false,true] {
        let dir=tempfile::tempdir().unwrap(); let path=dir.path().join("state"); let projection=dir.path().join("projection");
        let mut store=Store::open(&path,&projection,State::new("key",1,"session",vec![])).unwrap(); let clock=ClockSample {monotonic_s:0.0,epoch_s:1800000000.0};
        store.exec(1,"append",clock,Action::Append {text:"Olá".into(),delivered:false,ts:None,pre_transcript:false,entry_id:Some("entry".into())}).unwrap();
        if claimed {store.exec(1,"terminal:queue:1000",clock,Action::Claim {min_ts:1.0,limit:Some(1),entry_id:None}).unwrap();}
        store.exec(1,"terminal:queue:1001",clock,Action::Prepare {id:"attempt".into(),entry_id:Some("entry".into()),payload:json!({"operation_id":"attempt","kind":"input","payload":{"text":"Olá","pre_transcript":false}})}).unwrap();
        store.exec(1,"dispatch",clock,Action::BeginDispatch {id:"attempt".into(),wire_id:"terminal:1:attempt".into()}).unwrap();
        let mut frozen=serde_json::to_value(store.state()).unwrap(); drop(store);
        frozen["rows"][0]["attempts"]=json!(attempts);
        frozen["operations"]["attempt"]["status"]=json!(if rejected{"rejected"}else{"deferred"});
        frozen["operations"]["attempt"]["result"]=json!({"operation_id":"attempt","disposition":if rejected{"rejected"}else{"deferred"},"payload":{"cleanup":"proved"}});
        std::fs::write(&path,serde_json::to_vec(&frozen).unwrap()).unwrap();
        let mut store=Store::open(&path,&projection,State::new("key",1,"session",vec![])).unwrap(); store.exec(1,"recover1",clock,Action::Recover).unwrap();
        let row=store.state().rows[0].clone(); assert_eq!(row["delivered"],rejected||attempts==2);
        if rejected||attempts==2 {assert_eq!(row["desistiu"],true);}else{assert_eq!(row["attempts"],attempts+1);}
        store.exec(1,"recover2",clock,Action::Recover).unwrap(); assert_eq!(store.state().rows[0],row);
    }}}
}
#[test]
fn terminal_runtime_missing_row_recovery_preserves_uncertain_final_and_headless_inputs() {
    for status in [Status::Unknown,Status::Dispatching,Status::Accepted,Status::Confirmed,Status::Prepared] {
        for marker in [true,false] {
            let dir=tempfile::tempdir().unwrap();let path=dir.path().join("state");let projection=dir.path().join("projection");
            let mut store=Store::open(&path,&projection,State::new("key",1,"session",vec![])).unwrap();let clock=ClockSample {monotonic_s:0.0,epoch_s:1800000000.0};
            let mut payload=json!({"operation_id":"root","kind":"input","payload":{"text":"Olá","pre_transcript":true}});
            if marker {payload["payload"]["_terminal_generation"]=json!(1);}
            store.exec(1,"headless-prepare",clock,Action::Prepare {id:"root".into(),entry_id:Some("entry".into()),payload}).unwrap();
            if status==Status::Dispatching {store.exec(1,"dispatch",clock,Action::BeginDispatch {id:"root".into(),wire_id:"wire".into()}).unwrap();}
            else if status!=Status::Prepared {store.exec(1,"finish",clock,Action::Finish {id:"root".into(),status,result:json!({})}).unwrap();}
            if status==Status::Prepared {
                store.exec(1,"phase",clock,Action::Prepare {id:"phase".into(),entry_id:None,payload:json!({"logical_id":"root"})}).unwrap();
                store.exec(1,"phase-dispatch",clock,Action::BeginDispatch {id:"phase".into(),wire_id:"wire".into()}).unwrap();
            }
            store.exec(1,"recover",clock,Action::Recover).unwrap();assert!(store.state().rows.is_empty());
        }
    }
}

#[test]
fn terminal_runtime_legacy_completed_finalize_steps_are_not_repeated() {
    for transition in ["after_bump","after_unclaim","after_abandon"] {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("state");let projection=dir.path().join("projection");
        let mut store=Store::open(&path,&projection,State::new("key",1,"session",vec![])).unwrap();let clock=ClockSample {monotonic_s:0.0,epoch_s:1800000000.0};
        store.exec(1,"append",clock,Action::Append {text:"Olá".into(),delivered:false,ts:None,pre_transcript:false,entry_id:Some("entry".into())}).unwrap();
        store.exec(1,"terminal:queue:1",clock,Action::Prepare {id:"attempt".into(),entry_id:Some("entry".into()),payload:json!({"operation_id":"attempt","kind":"input","payload":{"text":"Olá","pre_transcript":false}})}).unwrap();
        store.exec(1,"dispatch",clock,Action::BeginDispatch {id:"attempt".into(),wire_id:"terminal:1:attempt".into()}).unwrap();
        let mut frozen=serde_json::to_value(store.state()).unwrap();drop(store);
        let status=if transition=="after_abandon"{"rejected"}else{"deferred"};
        let result=json!({"operation_id":"attempt","disposition":status,"payload":{"cleanup":"proved"}});
        frozen["operations"]["attempt"]["status"]=json!(status);frozen["operations"]["attempt"]["result"]=result.clone();
        let mut receipt=frozen["operations"]["call::dispatch"].clone();receipt["id"]=json!("call::terminal:queue:2");
        receipt["payload"]=json!({"kind":"finish","id":"attempt","status":status,"result":result});receipt["result"]=json!({});
        frozen["operations"]["call::terminal:queue:2"]=receipt.clone();frozen["rows"][0]["attempts"]=json!(1);
        if transition=="after_unclaim" {frozen["rows"][0]["delivered"]=json!(false);}
        else if transition=="after_abandon" {frozen["rows"][0]["desistiu"]=json!(true);frozen["rows"][0]["desistiu_ts"]=json!(clock.epoch_s-10.0);}
        if transition!="after_abandon" {
            receipt["id"]=json!("call::terminal:queue:3");receipt["payload"]=json!({"kind":"bump_attempts","entry_id":"entry"});receipt["result"]=json!(1);
            frozen["operations"]["call::terminal:queue:3"]=receipt;
        }
        std::fs::write(&path,serde_json::to_vec(&frozen).unwrap()).unwrap();
        let mut store=Store::open(&path,&projection,State::new("key",1,"session",vec![])).unwrap();store.exec(1,"recover",clock,Action::Recover).unwrap();
        let row=&store.state().rows[0];assert_eq!(row["attempts"],1,"{transition}");
        if transition=="after_abandon" {assert_eq!(row["desistiu_ts"],clock.epoch_s-10.0);}else{assert_eq!(row["delivered"],false);}
    }
}

#[test]
fn terminal_runtime_rejected_before_dispatch_abandons_existing_pending_entry() {
    let dir=tempfile::tempdir().unwrap();let mut store=Store::open(&dir.path().join("state"),&dir.path().join("projection"),State::new("key",1,"session",vec![])).unwrap();let clock=ClockSample {monotonic_s:0.0,epoch_s:1800000000.0};
    store.exec(1,"append",clock,Action::Append {text:"Olá".into(),delivered:false,ts:None,pre_transcript:false,entry_id:Some("entry".into())}).unwrap();
    store.exec(1,"prepare",clock,Action::Prepare {id:"op".into(),entry_id:Some("entry".into()),payload:json!({"operation_id":"op","kind":"input","payload":{"text":"Olá","_terminal_generation":1}})}).unwrap();
    store.exec(1,"finish",clock,Action::Finish {id:"op".into(),status:Status::Rejected,result:json!({"operation_id":"op","disposition":"rejected","payload":{"code":"refused"}})}).unwrap();
    assert_eq!(store.state().rows[0]["desistiu"],true);assert_eq!(store.state().rows[0]["delivered"],true);
}

#[test]
#[ignore = "exige HANGAR_TEST_PYTHON apontando ao Python do backend"]
fn terminal_runtime_store_python_rust_interop_preserves_finalization_marker() {
    let python=std::env::var_os("HANGAR_TEST_PYTHON").expect("HANGAR_TEST_PYTHON ausente");
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("state");let projection=dir.path().join("projection");
    let mut store=Store::open(&path,&projection,State::new("key",1,"session",vec![])).unwrap();let clock=ClockSample {monotonic_s:0.0,epoch_s:1800000000.0};
    store.exec(1,"append",clock,Action::Append {text:"Olá 🌎".into(),delivered:false,ts:None,pre_transcript:true,entry_id:Some("entry".into())}).unwrap();
    let payload=json!({"operation_id":"attempt","kind":"input","payload":{"text":"Olá 🌎","pre_transcript":true,"_terminal_generation":1}});
    store.exec(1,"prepare",clock,Action::Prepare {id:"attempt".into(),entry_id:Some("entry".into()),payload:payload.clone()}).unwrap();
    store.exec(1,"dispatch",clock,Action::BeginDispatch {id:"attempt".into(),wire_id:"terminal:1:attempt".into()}).unwrap();
    let result=json!({"operation_id":"attempt","disposition":"deferred","payload":{"cleanup":"proved"}});
    store.exec(1,"finish",clock,Action::Finish {id:"attempt".into(),status:Status::Deferred,result:result.clone()}).unwrap();drop(store);
    let script=r#"import sys
from pathlib import Path
from app.runtime_queue import QueueStore,initial_state
s=QueueStore(Path(sys.argv[1]),Path(sys.argv[2]),initial_state('key',1,'session',[]))
c={'monotonic_s':0.0,'epoch_s':1800000000.0}
assert s.state['operations']['attempt']['terminal_finalized'] is True
s.exec(1,'python-recover',c,{'kind':'recover'})
s.exec(1,'python-finish',c,{'kind':'finish','id':'attempt','status':'deferred','result':{'operation_id':'attempt','disposition':'deferred','payload':{'cleanup':'proved'}}})
assert s.state['rows'][0]['attempts']==1
s.exec(1,'python-missing-prepare',c,{'kind':'prepare','id':'missing','entry_id':'missing','payload':{'operation_id':'missing','kind':'input','payload':{'text':'Unicode 🌎','pre_transcript':True,'_terminal_generation':1}}})
s.exec(1,'python-missing-recover',c,{'kind':'recover'})
assert len(s.state['rows'])==2
"#;
    let backend=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../backend");
    let output=std::process::Command::new(python).args(["-c",script]).arg(&path).arg(&projection).env("PYTHONPATH",backend).output().unwrap();assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
    let mut store=Store::open(&path,&projection,State::new("key",1,"session",vec![])).unwrap();store.exec(1,"rust-recover",clock,Action::Recover).unwrap();
    assert!(store.state().operations["attempt"].terminal_finalized);assert_eq!(store.state().rows.len(),2);assert_eq!(store.state().rows[0]["attempts"],1);assert_eq!(store.state().rows[1]["text"],"Unicode 🌎");
    assert_eq!(store.state().rows[1]["pre_transcript"],true);
}
