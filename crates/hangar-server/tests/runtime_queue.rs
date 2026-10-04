use hangar_server::runtime::{protocol::ClockSample, queue::*};
use serde_json::json;

fn clock() -> ClockSample { ClockSample { monotonic_s: 10.0, epoch_s: 1_800_000_000.0 } }
fn fill(store:&mut Store, count:usize, prefix:&str) {
    for index in 0..count { store.exec(1,&format!("{prefix}:{index}"),clock(),Action::SetRuntimeState { state:json!({}) }).unwrap(); }
}
fn receipts(state:&State) -> usize { state.operations.keys().filter(|key|key.starts_with("call::")).count() }

/// O roteiro e o resultado vêm do Python (backend/tests/test_runtime_queue.py): a poda tem de
/// deixar o mesmo estado nos dois donos.
#[test]
fn compaction_matches_python_oracle() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../backend/tests/fixtures/runtime_queue/compaction.json");
    let oracle:serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("state"),&dir.path().join("projection"),State::new("key",1,"session",vec![])).unwrap();
    let mut results = Vec::new();
    for step in oracle["steps"].as_array().unwrap() {
        let action:Action = serde_json::from_value(step[1].clone()).unwrap();
        results.push(store.exec(1,step[0].as_str().unwrap(),clock(),action).unwrap());
    }
    fill(&mut store,oracle["fill"].as_u64().unwrap() as usize,"fill");
    assert_eq!(json!(results),oracle["results"]);
    assert_eq!(serde_json::to_value(store.state()).unwrap(),oracle["final"]);
    // O eco da primeira continua usado: a segunda, com cursor antes dele, não o reaproveita.
    let mut proof = oracle["steps"].as_array().unwrap().iter().find(|step|step[0] == "proof-first").unwrap()[1]["proof"].clone();
    proof["cursor"] = store.state().operations["second"].dispatch_cursor.clone();
    let proof = serde_json::from_value(proof).unwrap();
    assert_eq!(store.exec(1,"proof-second",clock(),Action::ConfirmOccurrence { id:"second".into(),proof }).unwrap(),json!(false));
}

#[test]
fn state_stays_bounded_with_100kb_replies() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state");
    let mut store = Store::open(&path,&dir.path().join("projection"),State::new("key",1,"session",vec![])).unwrap();
    let big = "x".repeat(100_000);
    // 500 mensagens já passam a janela muitas vezes; as 2000 rodam no teste Python (debug é lento).
    for index in 1..=500u64 {
        let (op,wire) = (format!("op-{index}"),format!("wire:op-{index}:{index}"));
        let cursor = json!({"conversation":"c","file_identity":"1:2","offset":index,"anchor":"a"});
        for (call,action) in [
            ("root",Action::Prepare { id:op.clone(),payload:json!({"operation_id":op,"kind":"input"}),entry_id:None }),
            ("wire",Action::Prepare { id:wire.clone(),payload:json!({"logical_id":op,"frame":{"text":"Olá"}}),entry_id:None }),
            ("cursor",Action::BindDispatch { id:wire.clone(),cursor }),
            ("dispatch",Action::BeginDispatch { id:wire.clone(),wire_id:wire.clone() }),
            ("ack",Action::Finish { id:wire.clone(),status:Status::Accepted,result:json!({"write_outcome":"written"}) }),
            ("reply",Action::Finish { id:op.clone(),status:Status::Accepted,
                result:json!({"operation_id":op,"disposition":"accepted","payload":{"tool_result":big}}) }),
        ] { store.exec(1,&format!("{call}:{index}"),clock(),action).unwrap(); }
    }
    let state = store.state();
    assert_eq!(receipts(state),256);
    assert!(state.operations.len() - receipts(state) <= 256);
    assert!(std::fs::metadata(&path).unwrap().len() < 1_000_000);
}

#[test]
fn kept_reply_still_replays_instead_of_resending() {
    use hangar_server::runtime::protocol::{Disposition,RuntimeReply};
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("state"),dir.path(),State::new("key",1,"session",vec![])).unwrap();
    store.exec(1,"append",clock(),append()).unwrap();
    store.exec(1,"prepare",clock(),Action::Prepare { id:"op".into(),payload:json!({"kind":"input"}),entry_id:Some("entry-1".into()) }).unwrap();
    store.exec(1,"begin",clock(),Action::BeginDispatch { id:"op".into(),wire_id:"wire:op:1".into() }).unwrap();
    store.exec(1,"reply",clock(),Action::Finish { id:"op".into(),status:Status::Accepted,
        result:json!({"operation_id":"op","disposition":"accepted","payload":{"tool_result":"x".repeat(1000)}}) }).unwrap();
    fill(&mut store,300,"fill");
    // Linha sem recibo: a raiz fica, e o ator ainda lê a resposta guardada (sem o conteúdo).
    let reply:RuntimeReply = serde_json::from_value(store.state().operations["op"].result.clone()).unwrap();
    assert!(reply.disposition == Disposition::Accepted && reply.payload.is_null());
}

#[test]
fn v1_state_shrinks_on_first_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state");
    let old = json!({"id":"old","payload":{},"entry_id":null,"status":"accepted","result":null,"dispatch_cursor":null,"wire_attempts":{}});
    let mut stuck = old.clone(); stuck["id"] = json!("stuck"); stuck["status"] = json!("unknown");
    std::fs::write(&path,serde_json::to_vec(&json!({"version":1,"owner_key":"key","generation":1,"name":"session","rows":[],
        "operations":{"old":old,"stuck":stuck},"used_occurrences":{"legacy":{"operation_id":"old","generation":1}},"runtime_state":{}})).unwrap()).unwrap();
    let store = Store::open(&path,dir.path(),State::new("key",1,"session",vec![])).unwrap();
    // Encolhe já na abertura, antes de qualquer gravação nova.
    let saved:serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!((saved["version"].clone(),saved["next_seq"].clone()),(json!(2),json!(257)));
    assert_eq!(saved["operations"].as_object().unwrap().keys().collect::<Vec<_>>(),["stuck"]);
    assert!(store.state().used_occurrences.is_empty());
}
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

#[cfg(unix)]
#[test]
fn unchanged_projection_is_not_rewritten() {
    // Ler a fila ou gravar o estado privado não muda as mensagens: o arquivo do nome fica intacto.
    use std::os::unix::fs::MetadataExt;
    let dir = tempfile::tempdir().unwrap();
    let projection = dir.path().join("projection");
    let mut store = Store::open(&dir.path().join("key.queue-state.json"), &projection, State::new("key",1,"session",vec![])).unwrap();
    store.exec(1, "append", clock(), append()).unwrap();
    let inode = || std::fs::metadata(projection.join("session.jsonl")).unwrap().ino();
    let before = inode();
    store.exec(1, "load", clock(), Action::Load).unwrap();
    fill(&mut store, 3, "view");
    assert_eq!(inode(), before);
    std::fs::write(projection.join("session.jsonl"), "").unwrap();
    store.exec(1, "repair", clock(), Action::EnsureProjection).unwrap();
    assert_eq!(std::fs::read_to_string(projection.join("session.jsonl")).unwrap().lines().count(), 1);
}
