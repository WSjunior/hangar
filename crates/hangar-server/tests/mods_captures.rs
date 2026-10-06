//! As capturas do terminal que viraram dado de teste estão limpas e com a geometria de origem. O
//! repositório é público: nada pessoal nem de mod de empresa pode entrar.
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn folder() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mods_screen")
}

fn cases() -> Vec<Value> {
    serde_json::from_slice(&std::fs::read(folder().join("casos.json")).unwrap()).unwrap()
}

#[test]
fn every_case_has_its_file_and_no_file_is_extra() {
    let mut names: Vec<String> = cases().iter().map(|case| case["nome"].as_str().unwrap().to_owned()).collect();
    names.sort();
    let mut files: Vec<String> = std::fs::read_dir(folder()).unwrap()
        .filter_map(|entry| entry.unwrap().file_name().to_string_lossy().strip_suffix(".ansi").map(str::to_owned))
        .collect();
    files.sort();
    assert_eq!(names, files);
    assert_eq!(names.len(), 34);
}

#[test]
fn nothing_personal_nor_from_a_company() {
    for case in cases() {
        let name = case["nome"].as_str().unwrap();
        let raw = std::fs::read_to_string(folder().join(format!("{name}.ansi"))).unwrap().to_lowercase();
        for forbidden in ["pm-18", "ms_devops", "pserver", "promedico", "waldir", "pessoais", "claude.ai/code"] {
            assert!(!raw.contains(forbidden), "{name}: contém {forbidden}");
        }
    }
}

// Só onde há `python3` no PATH como nos runners Linux e macOS; o Windows comum não tem. A conferência
// roda também nos portões da Task 18.
#[cfg(unix)]
#[test]
fn every_word_is_allowed_or_replaced() {
    // A lista de palavras permitidas mora só na ferramenta: o teste pede a conferência a ela, em vez de
    // repetir a lista aqui.
    let output = Command::new("python3").arg(folder().join("limpar_capturas.py")).arg("--conferir")
        .output().expect("python3 no PATH (o teste do tmux real já usa)");
    assert!(output.status.success(), "{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
}
