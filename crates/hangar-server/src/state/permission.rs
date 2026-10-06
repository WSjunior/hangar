//! Modo de permissão lido no rodapé do Claude (`permission_mode.parse_permission_mode`) e quando
//! perguntar ao Python (`permission.observe`): a memória do modo é dele, por session-id, e a
//! resposta dele é a que vai no evento.
use std::sync::LazyLock;

use regex::Regex;

static FOOTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(
    r"(?i)(plan mode on|auto mode on|manual mode on|accept edits on|bypass permissions on|don['’]t ask on)").unwrap());

/// `_CANON` sobre a frase casada, normalizada como `_norm` (a frase só tem espaços simples).
fn canon(phrase: &str) -> Option<&'static str> {
    Some(match phrase.to_lowercase().replace('’', "'").as_str() {
        "plan mode on" => "plan",
        "auto mode on" => "auto",
        "manual mode on" => "manual",
        "accept edits on" => "acceptEdits",
        "bypass permissions on" => "bypassPermissions",
        "don't ask on" => "dontAsk",
        _ => return None,
    })
}

/// Última linha com o glifo (⏸/⏵) e a frase; sem nenhuma, a última com a frase (pane estreito
/// que cortou o glifo).
pub fn parse_permission_mode(pane: &str) -> Option<&'static str> {
    let lines = crate::terminal_state::lines(pane);
    let pass = |glyph: bool| lines.iter().rev()
        .filter(|l| !glyph || l.contains('⏸') || l.contains('⏵'))
        .find_map(|l| FOOTER.find(l).and_then(|m| canon(m.as_str())));
    pass(true).or_else(|| pass(false))
}

/// O que já foi perguntado ao Python. Fora da operação controlada a resposta dele para a mesma
/// leitura não muda, então só se pergunta na leitura nova, a cada rodada da operação e uma vez
/// quando ela acaba.
#[derive(Debug, Default)]
pub struct Watch {
    key: Option<String>,
    sent: Option<String>,
    during_op: bool,
    /// `(modo, previous_non_plan)` da última resposta.
    pub current: Option<(String, String)>,
}

impl Watch {
    pub fn due(&self, key: &str, observed: &str, op: bool) -> bool {
        op || self.during_op || self.key.as_deref() != Some(key) || self.sent.as_deref() != Some(observed)
    }

    pub fn answered(&mut self, key: &str, observed: &str, op: bool, answer: (String, String)) {
        self.key = Some(key.to_owned());
        self.sent = Some(observed.to_owned());
        self.during_op = op;
        self.current = Some(answer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_matches_python() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../backend/tests/fixtures/contract/golden/permission_mode.json");
        let rows: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let rows = rows.as_array().unwrap();
        assert!(rows.len() >= 10);
        for row in rows {
            assert_eq!(parse_permission_mode(row["pane"].as_str().unwrap()), row["expected"].as_str(), "{}", row["name"]);
        }
    }

    #[test]
    fn asks_on_new_reading_during_operation_and_once_after() {
        let mut w = Watch::default();
        assert!(w.due("sid", "plan", false), "nada perguntado ainda");
        w.answered("sid", "plan", false, ("plan".into(), "manual".into()));
        assert!(!w.due("sid", "plan", false), "mesma leitura fora da operação");
        assert!(w.due("sid", "auto", false));
        assert!(w.due("outra", "plan", false), "session-id novo depois do /clear");
        assert!(w.due("sid", "plan", true), "toda rodada da operação");
        w.answered("sid", "plan", true, ("plan".into(), "manual".into()));
        assert!(w.due("sid", "plan", false), "uma vez quando a operação acaba");
        w.answered("sid", "plan", false, ("plan".into(), "manual".into()));
        assert!(!w.due("sid", "plan", false));
    }
}
