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
