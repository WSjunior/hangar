use hangar_api::state::StateEvent;
use serde_json::Value;

#[test]
fn golden_preserves_public_fields() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../backend/tests/fixtures/headless_runtime");
    for provider in ["claude", "codex"] {
        let path = root.join(format!("{provider}-golden.json"));
        let raw = std::fs::read(&path).expect("gerar o oráculo Python antes da comparação");
        let scenarios: Vec<Value> = serde_json::from_slice(&raw).unwrap();
        assert!(!scenarios.is_empty(), "o oráculo não pode ser vazio");
        for scenario in scenarios {
            for output in scenario["outputs"].as_array().unwrap() {
                if output["channel"] == "state" {
                    let data = output["data"].clone();
                    let event: StateEvent = serde_json::from_value(data.clone()).unwrap();
                    assert_eq!(serde_json::to_value(event).unwrap(), data, "{}", scenario["name"]);
                }
            }
        }
    }
}
