mod common;
use common::costs::*;
use hangar_server::costs::areas::AreaMap;
use hangar_server::costs::claude;
use hangar_server::costs::index::Index;
use hangar_server::costs::index::Fold;
use serde_json::json;
use serde_json::{Map, Value};
use std::path::Path;

fn sync_claude(ix: &Index, base: &Path, areas: &AreaMap) {
    let root = base.join("claude/projects");
    let files = hangar_server::costs::collect::list_files(&root, |n| n.ends_with(".jsonl"));
    ix.sync(&format!("claude:{}", root.display()), &files, &claude::new_fold(&root), claude::VERSION,
            areas.signature(), &|e| areas.area_lines(e), &progress()).unwrap();
}

fn claude_golden(g: &Value) -> Value {
    Value::Object(g.as_object().unwrap().iter().filter(|(k, _)| k.starts_with("claude/"))
        .map(|(k, v)| (k.clone(), v.clone())).collect::<Map<_, _>>())
}

#[test]
fn claude_index_matches_python() {
    let (_d, base) = fixtures_copy();
    let areas = AreaMap::load(Path::new("/nao/existe.json"));
    let ix = Index::open(&base.join("../idx")).unwrap();
    sync_claude(&ix, &base, &areas);
    assert_close(&dump(&ix, &base, "claude/"), &claude_golden(&golden_index()), "claude");
}

#[test]
fn claude_resumed_in_two_halves_matches_python() {
    let (_d, base) = fixtures_copy();
    let areas = AreaMap::load(Path::new("/nao/existe.json"));
    let ix = Index::open(&base.join("../idx")).unwrap();
    let rest = halve_all(&base);
    sync_claude(&ix, &base, &areas);
    for (p, tail) in rest { append(&p, &tail); }
    sync_claude(&ix, &base, &areas);
    assert_close(&dump(&ix, &base, "claude/"), &claude_golden(&golden_index()["__resumed__"]), "claude retomado");
}

fn response(id: &str, input: i64) -> Vec<u8> {
    let mut raw = serde_json::to_vec(&json!({"type":"assistant", "timestamp":"2026-10-01T12:00:00Z", "cwd":"/repo/synthetic",
        "requestId":id, "message":{"id":id, "model":"claude-sonnet-5", "usage":{"input_tokens":input,"output_tokens":2},
        "content":[{"type":"tool_use","id":id,"name":"Read","input":{"file_path":"backend/a.py"}}]}})).unwrap();
    raw.push(b'\n');
    raw
}

#[test]
fn claude_effective_resume_preserves_parent_and_child_state() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("costs/claude/projects");
    let parent = root.join("project/main.jsonl");
    let child = root.join("project/main/subagents/agent-a.jsonl");
    std::fs::create_dir_all(child.parent().unwrap()).unwrap();
    for path in [&parent, &child] { std::fs::write(path, response("first", 10)).unwrap(); }
    let areas = AreaMap::load(Path::new("/nao/existe.json"));
    let ix = Index::open(&d.path().join("idx")).unwrap();
    let creations = AtomicUsize::new(0);
    let factory = claude::new_fold(&root);
    let tracked = |p: &Path| { creations.fetch_add(1, Ordering::Relaxed); factory(p) };
    for round in 0..2 {
        if round == 1 { for path in [&parent, &child] { append(path, &response("second", 20)); } }
        ix.sync("synthetic", &[parent.clone(), child.clone()], &tracked, claude::VERSION,
            areas.signature(), &|e| areas.area_lines(e), &progress()).unwrap();
    }
    assert_eq!(creations.load(Ordering::Relaxed), 2, "a segunda passada precisa retomar o estado salvo");
    let got = dump(&ix, &d.path().join("costs"), "claude/");
    for (path, child) in [("claude/projects/project/main.jsonl", 0), ("claude/projects/project/main/subagents/agent-a.jsonl", 1)] {
        assert_eq!(got[path]["custo"][0][6], 30);
        assert_eq!(got[path]["custo"][0][10], child);
        assert_eq!(got[path]["uso"][0][8], 2);
        assert_eq!(got[path]["uso"][0][21], child);
        assert_eq!(got[path]["areas"][0][8], 2);
        assert_eq!(got[path]["areas"][0][11], 30);
        assert_eq!(got[path]["areas"][0][21], child);
    }
    let mapping = d.path().join("areas.json");
    std::fs::write(&mapping, br#"{"padrao":[["reclassified",["*.py"]]]}"#).unwrap();
    let changed = AreaMap::load(&mapping);
    ix.sync("synthetic", &[parent, child], &tracked, claude::VERSION,
        changed.signature(), &|e| changed.area_lines(e), &progress()).unwrap();
    assert_eq!(creations.load(Ordering::Relaxed), 2, "áreas novas não releem transcript");
    let updated = dump(&ix, &d.path().join("costs"), "claude/");
    for (path, old) in got.as_object().unwrap() {
        assert_eq!(updated[path]["custo"], old["custo"]);
        assert_eq!(updated[path]["uso"], old["uso"]);
        assert_eq!(updated[path]["areas"][0][4], "reclassified");
        assert_eq!(updated[path]["areas"][0][11], 30);
    }
}

#[test]
fn claude_snapshot_fragment_and_repeated_close_preserve_state() {
    let mut fold = claude::ClaudeFold::new("project/main".into(), false, false);
    fold.line(&response("first", 10));
    let saved = serde_json::to_vec(&fold).unwrap();
    let first = fold.close();
    assert_eq!(first.costs, fold.close().costs);
    assert_eq!(saved, serde_json::to_vec(&fold).unwrap(), "fechar não altera o snapshot");
    let mut resumed: claude::ClaudeFold = serde_json::from_slice(&saved).unwrap();
    resumed.line(&response("second", 20));
    assert_eq!(resumed.close().costs[0].input, 30);

    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("costs/claude/projects");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("fragment.jsonl");
    let mut raw = response("first", 10);
    let mut second = response("second", 20);
    second.pop();
    raw.extend(second);
    std::fs::write(&path, raw).unwrap();
    let ix = Index::open(&d.path().join("idx")).unwrap();
    let areas = AreaMap::load(Path::new("/nao/existe.json"));
    let sync = || ix.sync("fragment", &[path.clone()], &claude::new_fold(&root), claude::VERSION,
        areas.signature(), &|e| areas.area_lines(e), &progress()).unwrap();
    sync();
    assert_eq!(dump(&ix, &d.path().join("costs"), "claude/")["claude/projects/fragment.jsonl"]["custo"][0][6], 30);
    let mut tail = b"\n".to_vec();
    tail.extend(response("third", 40));
    append(&path, &tail);
    sync();
    let got = dump(&ix, &d.path().join("costs"), "claude/");
    assert_eq!(got["claude/projects/fragment.jsonl"]["custo"][0][6], 70);
    assert_eq!(got["claude/projects/fragment.jsonl"]["uso"][0][8], 3);
}

#[test]
fn claude_fallback_keeps_surrogates_and_invalid_bytes_but_rejects_non_objects() {
    let mut fold = claude::ClaudeFold::new(String::new(), false, false);
    fold.line(r#"{"type":"attachment","attachment":{"type":"surrogate","content":"a\ud800é"}}"#.as_bytes());
    let mut raw = br#"{"type":"attachment","attachment":{"type":"invalid","content":"a"#.to_vec();
    raw.push(0xff);
    raw.extend(r#"é"}}"#.as_bytes());
    fold.line(&raw);
    let before = serde_json::to_value(fold.close().usage).unwrap();
    assert_eq!(before[0]["ctx_chars"], 3);
    assert_eq!(before[1]["ctx_chars"], 3);
    // Serde aceita sequência como struct; o JSON Python exige objeto nesta entrada.
    fold.line(br#"["attachment",null,null,null,null,null,null,null,{"type":"array","content":"usage"},null,null]"#);
    fold.line(br#"null"#);
    assert_eq!(before, serde_json::to_value(fold.close().usage).unwrap());
}

#[test]
fn claude_ignored_model_discards_usage_tools_and_context_changes() {
    let mut fold = claude::ClaudeFold::new(String::new(), false, false);
    fold.line(&response("first", 10));
    fold.line(br#"{"type":"assistant","cwd":"/ignored","message":{"model":"  <synthetic>  ","usage":{"input_tokens":999},"content":[{"type":"tool_use","name":"Ignored","input":{}}]}}"#);
    fold.line(r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"é"}]}}"#.as_bytes());
    let out = fold.close();
    assert_eq!(out.costs[0].input, 10);
    assert!(out.usage.iter().all(|r| r.nome != "Ignored" && r.cwd == "/repo/synthetic" && r.model == "claude-sonnet-5"));
}

#[test]
fn claude_typed_and_fallback_keep_the_same_fields() {
    let mut fast = claude::ClaudeFold::new(String::new(), false, false);
    let mut fallback = claude::ClaudeFold::new(String::new(), false, false);
    fast.line(r#"{"type":"attachment","cwd":"/repo/synthetic","rendered":null,"attachment":{"type":"hook_context","hookName":"Start","content":"[fixture] Texto sintético"}}"#.as_bytes());
    // Campo repetido recusa o struct tipado, mas json.loads mantém a última versão.
    fallback.line(r#"{"type":"ignored","type":"attachment","cwd":"/repo/synthetic","rendered":null,"attachment":{"type":"hook_context","hookName":"Start","content":"[fixture] Texto sintético"}}"#.as_bytes());
    assert_eq!(fast.close().usage, fallback.close().usage);
}

#[test]
fn claude_agent_type_uses_python_string_conversion() {
    let mut fold = claude::ClaudeFold::new(String::new(), false, false);
    let mut raw = response("agent", 1);
    let mut value: Value = serde_json::from_slice(&raw).unwrap();
    value["message"]["content"] = json!([{"type":"tool_use","name":"Agent","input":{"subagent_type":["worker",true]}}]);
    raw = serde_json::to_vec(&value).unwrap();
    fold.line(&raw);
    let rows = fold.close().usage;
    assert_eq!(rows.iter().find(|r| r.tipo == "agente").unwrap().nome, "['worker', True]");
}

#[test]
fn claude_factory_distinguishes_absolute_and_relative_subagent_markers() {
    let root = Path::new("/synthetic/subagents/projects");
    let mut fold = claude::new_fold(root)(&root.join("project/main.jsonl"));
    fold.line(&response("first", 1));
    let out = fold.close();
    assert_eq!(out.costs[0].session_id, "project/main");
    assert!(out.costs[0].subagente);
    assert!(!out.usage[0].subagente);
    assert_eq!(out.areas.unwrap().header.subagente, Some(false));
}
