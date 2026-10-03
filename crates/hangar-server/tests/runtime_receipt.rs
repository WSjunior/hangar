use hangar_server::runtime::receipt::*;
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn one_echo_confirms_only_one_identical_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chat.jsonl");
    std::fs::write(&path, "").unwrap();
    let mut index = ReceiptIndex::new("claude", "sid");
    let cursor = index.capture(&path).unwrap();
    std::fs::write(&path,"{\"type\":\"user\",\"uuid\":\"echo-1\",\"message\":{\"content\":\"Olá\"}}\r\n").unwrap();
    index.scan(&path).unwrap();
    let row = json!({"text":"Olá"});
    let proof = index.match_after(&cursor,&row,&BTreeMap::new()).unwrap();
    let used = BTreeMap::from([(proof.occurrence.id,json!({"operation_id":"first"}))]);
    assert!(index.match_after(&cursor,&row,&used).is_none());
    let mut restored = ReceiptIndex::new("claude","sid");
    restored.scan(&path).unwrap();
    assert!(restored.match_after(&cursor,&row,&used).is_none());
}

#[test]
fn missing_or_partial_transcript_is_no_proof() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chat.jsonl");
    let mut index = ReceiptIndex::new("claude","sid");
    let cursor = index.capture(&path).unwrap();
    assert!(index.scan(&path).unwrap().is_empty());
    std::fs::write(&path,r#"{"type":"user","message":{"content":"Olá"}}"#).unwrap();
    index.scan(&path).unwrap();
    assert!(index.match_after(&cursor,&json!({"text":"Olá"}),&BTreeMap::new()).is_none());
}

#[test]
fn enqueue_is_not_consumption() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chat.jsonl");
    std::fs::write(&path,"").unwrap();
    let mut index = ReceiptIndex::new("claude","sid");
    let cursor = index.capture(&path).unwrap();
    std::fs::write(&path,"{\"type\":\"queue-operation\",\"operation\":\"enqueue\",\"content\":\"Olá\"}\n").unwrap();
    index.scan(&path).unwrap();
    assert!(index.match_after(&cursor,&json!({"text":"Olá"}),&BTreeMap::new()).is_none());
}

#[test]
fn cursor_is_dispatch_not_enqueue() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chat.jsonl");
    std::fs::write(&path,"").unwrap();
    let mut index = ReceiptIndex::new("claude","sid");
    let first = index.capture(&path).unwrap();
    std::fs::write(&path,"{\"type\":\"user\",\"message\":{\"content\":\"Olá\"}}\n").unwrap();
    let second = index.capture(&path).unwrap();
    index.scan(&path).unwrap();
    assert!(index.match_after(&first,&json!({"text":"Olá"}),&BTreeMap::new()).is_some());
    assert!(index.match_after(&second,&json!({"text":"Olá"}),&BTreeMap::new()).is_none());
}

#[test]
fn rewritten_file_and_old_conversation_are_no_proof() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chat.jsonl");
    std::fs::write(&path,"{\"type\":\"user\",\"message\":{\"content\":\"Anterior\"}}\n").unwrap();
    let mut index = ReceiptIndex::new("claude","sid");
    let cursor = index.capture(&path).unwrap();
    std::fs::write(&path,"{\"type\":\"user\",\"message\":{\"content\":\"Reescrito\"}}\n{\"type\":\"user\",\"message\":{\"content\":\"Olá\"}}\n").unwrap();
    index.scan(&path).unwrap();
    assert!(index.match_after(&cursor,&json!({"text":"Olá"}),&BTreeMap::new()).is_none());
    let mut other = ReceiptIndex::new("claude","other-sid");
    other.scan(&path).unwrap();
    assert!(other.match_after(&cursor,&json!({"text":"Olá"}),&BTreeMap::new()).is_none());
}

#[test]
fn steer_attachment_is_delivery() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chat.jsonl");
    std::fs::write(&path,"").unwrap();
    let mut index = ReceiptIndex::new("claude","sid");
    let cursor = index.capture(&path).unwrap();
    std::fs::write(&path,"{\"type\":\"attachment\",\"attachment\":{\"type\":\"queued_command\",\"prompt\":[{\"type\":\"text\",\"text\":\"Olá 🌎\"}]}}\n").unwrap();
    index.scan(&path).unwrap();
    assert!(index.match_after(&cursor,&json!({"text":"Olá 🌎"}),&BTreeMap::new()).is_some());
}

#[test]
fn terminal_runtime_absent_cursor_needs_current_conversation_and_new_timestamp() {
    let dir=tempfile::tempdir().unwrap(); let path=dir.path().join("chat.jsonl");
    let mut index=ReceiptIndex::new("claude","sid"); let cursor=index.capture(&path).unwrap();
    for (sid,ts) in [("sid","2020-01-01T00:00:00Z".to_string()),("other",chrono::Utc::now().to_rfc3339())] {
        std::fs::write(&path,format!("{}\n",json!({"type":"user","sessionId":sid,"timestamp":ts,"message":{"content":"Olá"}}))).unwrap();
        index.scan(&path).unwrap(); assert!(index.match_after(&cursor,&json!({"text":"Olá"}),&BTreeMap::new()).is_none());
    }
}
