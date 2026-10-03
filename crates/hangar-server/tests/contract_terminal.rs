use hangar_server::terminal_state::{analyze, reduce, ReducerFacts, ReducerMemory};

#[test]
fn terminal_parsers_and_sequences_match_python() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../backend/tests/fixtures/contract");
    let rows: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("golden/terminal.json")).unwrap(),
    ).unwrap();
    for row in rows.as_array().unwrap() {
        if let Some(pane) = row["pane"].as_str() {
            assert_eq!(serde_json::to_value(analyze(pane)).unwrap(), row["expected"], "{}", row["name"]);
        } else {
            let mut memory = ReducerMemory::default();
            for (index, frame) in row["sequence"].as_array().unwrap().iter().enumerate() {
                let facts: ReducerFacts = serde_json::from_value(frame["facts"].clone()).unwrap();
                let result = reduce(frame["pane"].as_str().unwrap(), memory, facts);
                assert_eq!(serde_json::to_value(&result).unwrap(), row["expected_sequence"][index], "{} frame {}", row["name"], index);
                memory = result.memory;
            }
        }
    }
}
