//! Pergunta nativa do AskUserQuestion do Claude com terminal: o sidecar do hook PreToolUse
//! (`<config>/.hangar-askq/<sid>.json`) casado com o menu que o pane mostra. Porte de
//! `sse._ask_question_event` e `askquestion.read_pending_askq`. Falhou o casamento, o app fica com
//! os botões do menu do pane; nunca abre o stepper sobre outra pergunta.
use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use hangar_api::ask::{AskQuestion, AskQuestionItem};
use hangar_api::state::StateEvent;
use serde::Deserialize;

/// Linhas que o TUI acrescenta a toda pergunta e que nunca estão no payload do hook, comparadas
/// sem a caixinha e sem o ponto final (a múltipla escolha mostra "Type something" sem ponto).
const TUI_EXTRAS: [&str; 2] = ["Type something", "Chat about this"];

/// `_CAIXA_MULTI` (`^\[.?\]\s*`): a caixinha da múltipla escolha é desenho, não rótulo.
fn without_box(label: &str) -> &str {
    let Some(rest) = label.strip_prefix('[') else { return label };
    let mut chars = rest.chars();
    let after = match chars.next() {
        Some(c) if c != '\n' && chars.as_str().starts_with(']') => &chars.as_str()[1..],
        _ => match rest.strip_prefix(']') {
            Some(after) => after,
            None => return label,
        },
    };
    after.trim_start()
}

fn is_tui_extra(label: &str) -> bool { TUI_EXTRAS.contains(&without_box(label).trim_end_matches('.')) }

/// O sidecar responde à pergunta que o pane mostra agora? Só em `awaiting_input`.
pub fn matches(state: &StateEvent, payload: &AskQuestion) -> bool {
    if state.state != "awaiting_input" {
        return false;
    }
    let Some(first) = payload.questions.first() else { return false };
    let first: BTreeSet<&str> = first.options.iter().map(|o| o.label.as_str()).collect();
    let pane: BTreeSet<&str> = state.options.iter().flatten().filter(|o| !is_tui_extra(o)).map(|o| without_box(o)).collect();
    if first.is_empty() || pane.is_empty() {
        return false;
    }
    let has_preview = payload.questions.iter().flat_map(|q| &q.options).any(|o| !o.preview.is_empty());
    // Sem prévia, igualdade exata: a resposta vai por índice, e opção a mais no pane cairia na
    // linha errada. Com prévia o pane trunca o rótulo: prefixo numa direção só, mesma contagem
    // (rótulo curto do pane casando com um longo do sidecar não troca a permissão por outra).
    let (ok, reason) = if has_preview {
        (first.len() == pane.len() && first.iter().all(|l| pane.iter().any(|s| !s.is_empty() && l.starts_with(s))), "askq_preview_mismatch")
    } else if !first.is_subset(&pane) {
        (false, "askq_label_outside_menu")
    } else {
        (pane.is_subset(&first), "askq_pane_extra_option")
    };
    // Degradar para os botões do pane é silencioso na tela: a frequência no diário é o sinal de
    // que o TUI mudou o texto das linhas dele.
    if !ok && crate::warn_limit::allow(Some(&state.session), reason) {
        tracing::info!(session = state.session.as_str(), code = reason, sidecar = first.len(), pane = pane.len(), "askq: menu não casa, fica com os botões do pane");
    }
    ok
}

/// `<config>/projects/<cwd>/<sid>.jsonl` → `<config>/.hangar-askq/<sid>.json`.
pub fn sidecar_path(jsonl: &Path) -> Option<PathBuf> {
    let config = jsonl.parent()?.parent()?.parent()?;
    Some(config.join(crate::list::facts_files::ASKQ_DIR).join(format!("{}.json", jsonl.file_stem()?.to_str()?)))
}

#[derive(Deserialize)]
struct Sidecar { tool_input: ToolInput }

#[derive(Deserialize)]
struct ToolInput { questions: Vec<AskQuestionItem> }

/// Pergunta pendente do sidecar. Ausente é o caso normal (`Ok(None)`); `Err` é o código de um
/// arquivo que existe e não serve (contrato do hook quebrado), que vai ao diário.
pub fn read_pending(jsonl: &Path) -> Result<Option<AskQuestion>, &'static str> {
    let Some(path) = sidecar_path(jsonl) else { return Ok(None) };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("askq_unreadable"),
    };
    // ponytail: `multiSelect` como texto ("true") o pydantic aceita e o serde recusa; o resultado
    // é o mesmo de um sidecar quebrado (fica o menu do pane), e o hook grava o booleano.
    let sidecar: Sidecar = serde_json::from_slice(&bytes).map_err(|_| "askq_malformed")?;
    Ok(Some(AskQuestion { questions: sidecar.tool_input.questions }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn matches_menu_python_golden() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../backend/tests/fixtures/contract/golden/ask_question.json");
        let rows: Vec<Value> = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert!(rows.len() >= 15);
        let dir = tempfile::tempdir().unwrap();
        for row in &rows {
            let name = row["name"].as_str().unwrap();
            let state = match row["pane"].as_str() {
                Some(pane) => {
                    let a = crate::terminal_state::analyze(pane);
                    assert_eq!(serde_json::to_value(&a.options).unwrap(), row["options"], "{name}: menu do pane");
                    StateEvent { state: a.state, options: a.options, ..Default::default() }
                }
                None => StateEvent { state: row["state"].as_str().unwrap().into(),
                    options: serde_json::from_value(row["options"].clone()).unwrap(), ..Default::default() },
            };
            let jsonl = dir.path().join(name).join("projects/p/sid.jsonl");
            let askq = sidecar_path(&jsonl).unwrap();
            std::fs::create_dir_all(askq.parent().unwrap()).unwrap();
            match &row["sidecar"] {
                Value::Null => {}
                Value::String(raw) => std::fs::write(&askq, raw).unwrap(),
                other => std::fs::write(&askq, other.to_string()).unwrap(),
            }
            let event = read_pending(&jsonl).ok().flatten()
                .filter(|p| matches(&state, p))
                .map(|p| serde_json::to_value(p).unwrap());
            assert_eq!(event.unwrap_or(Value::Null), row["expected"], "{name}");
        }
    }

    #[test]
    fn box_and_extras() {
        assert_eq!(without_box("[ ] Alfa"), "Alfa");
        assert_eq!(without_box("[x]Bravo"), "Bravo");
        assert_eq!(without_box("[]  C"), "C");
        assert_eq!(without_box("[]] D"), "D");
        assert_eq!(without_box("[ab] E"), "[ab] E");
        assert!(is_tui_extra("[ ] Type something.") && is_tui_extra("Chat about this"));
        assert!(!is_tui_extra("Type something else"));
    }

    #[test]
    fn missing_is_none_and_broken_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let jsonl = dir.path().join("projects/p/sid.jsonl");
        assert_eq!(read_pending(&jsonl), Ok(None));
        let askq = sidecar_path(&jsonl).unwrap();
        std::fs::create_dir_all(askq.parent().unwrap()).unwrap();
        std::fs::write(&askq, "{").unwrap();
        assert_eq!(read_pending(&jsonl), Err("askq_malformed"));
    }
}
