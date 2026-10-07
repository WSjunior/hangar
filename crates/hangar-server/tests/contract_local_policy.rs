use hangar_server::runtime::local_policy::{is_local, run_at};
use serde_json::{json, Value};
use std::path::Path;

fn golden(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../backend/tests/fixtures/contract/local_policy").join(name);
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn substitute(value: &Value, root: &str) -> Value {
    match value {
        Value::String(text) => Value::String(text.replace("{ROOT}", root)),
        Value::Array(items) => Value::Array(items.iter().map(|item| substitute(item, root)).collect()),
        Value::Object(fields) => Value::Object(fields.iter().map(|(key, item)| (key.clone(), substitute(item, root))).collect()),
        other => other.clone(),
    }
}

fn materialize(files: &Value, root: &Path) {
    use base64::Engine;
    for (rel, spec) in files.as_object().into_iter().flatten() {
        let path = root.join(rel);
        if spec["dir"] == true { std::fs::create_dir_all(&path).unwrap(); continue; }
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        if let Some(data) = spec["b64"].as_str() {
            std::fs::write(&path, base64::engine::general_purpose::STANDARD.decode(data).unwrap()).unwrap();
        } else if let Some(size) = spec["size"].as_u64() {
            std::fs::write(&path, vec![0u8; size as usize]).unwrap();
        } else {
            std::fs::write(&path, spec["text"].as_str().unwrap()).unwrap();
        }
    }
}

/// Um teste só: TZ, HOME e a variável de esforço são do processo, e testes paralelos se pisariam.
#[test]
fn local_policies_match_the_python_golden() {
    let mut failures = Vec::new();
    let mut total = 0;
    for file in ["prepare_prompt.json", "format_status_claude.json", "format_status_codex.json", "skill_catalog.json"] {
        let document = golden(file);
        // SAFETY: único teste deste binário; nenhuma outra thread lê o ambiente enquanto ele roda.
        unsafe { std::env::set_var("TZ", document["tz"].as_str().unwrap()); }
        for case in document["cases"].as_array().unwrap() {
            total += 1;
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().to_str().unwrap();
            materialize(&case["files"], dir.path());
            // SAFETY: idem.
            unsafe {
                std::env::remove_var("CLAUDE_CODE_EFFORT_LEVEL");
                std::env::set_var("HOME", dir.path().join("home"));
                for (key, value) in case["env"].as_object().into_iter().flatten() { std::env::set_var(key, value.as_str().unwrap()); }
            }
            let kind = case["kind"].as_str().unwrap();
            assert!(is_local(kind), "{kind} deve rodar no Rust");
            let payload = substitute(&case["payload"], root);
            let meta = substitute(&case["meta"], root);
            let quota = case.get("quota");
            let now = case["now"].as_f64().unwrap_or(0.0);
            let actual = match run_at(kind, &payload, &meta, quota, now) {
                Some(Ok(value)) => value,
                Some(Err(_)) => json!({"error": true}),
                None => json!("not local"),
            };
            let expected = substitute(&case["expected"], root);
            if actual != expected {
                failures.push(format!("{file}/{}: Rust {actual}; Python {expected}", case["name"]));
            }
        }
    }
    assert!(total > 150, "golden menor que o esperado: {total}");
    assert!(failures.is_empty(), "{} divergências:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn remote_kinds_are_not_local() {
    for kind in ["native_message", "last_usage", "reload_stamp", "session.patch_meta", "unknown_private", "terminal_facts", "quota", "answer_body"] {
        assert!(!is_local(kind), "{kind} continua no Python ou saiu do serviço");
        assert!(run_at(kind, &json!({}), &json!({"provider": "claude"}), None, 0.0).is_none());
    }
}
