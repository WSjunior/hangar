//! O que sai junto com o estado (porte do laço de `sse.merged_events`): a sugestão quando muda, a
//! borda "voltou a aceitar texto" que pede a entrega ao Python e o problema do runtime de entrada.
use hangar_api::state::StateEvent;

/// O que o ator de entrada terminal publica: `error` é o código do canal `problem` ou do retrato,
/// `message` a frase dele, `input_stalled` o código do escritor parado.
#[derive(Clone, Debug, Default)]
pub struct RuntimeView { pub error: Option<String>, pub message: Option<String>, pub input_stalled: Option<String> }

/// `runtime_adapter.runtime_problem`: `(código, detalhe)` do runtime para o `problema` do estado.
pub fn runtime_problem(view: &RuntimeView) -> Option<(String, String)> {
    if let Some(code) = view.error.as_deref() {
        let text = match view.message.as_deref().filter(|m| !m.is_empty()) {
            Some(message) => format!("{code}: {message}"),
            None => code.to_owned(),
        };
        return Some(("runtime_falhou".into(), text.chars().take(300).collect()));
    }
    let stalled = view.input_stalled.as_deref().filter(|s| !s.is_empty())?;
    let code = match stalled {
        "composer_busy" => "terminal_input_composer_busy",
        "composer_unreadable" => "terminal_input_composer_unreadable",
        "capture_failed" | "capture_utf8" => "terminal_input_capture_failed",
        _ => "terminal_input_stalled",
    };
    Some((code.into(), stalled.chars().take(60).collect()))
}

/// Problema que veio do runtime, e não da observação nem dos fatos.
pub fn is_runtime_problem(code: &str) -> bool { code == "runtime_falhou" || code.starts_with("terminal_input_") }

/// O pane aceita texto livre: fora de pergunta e de menu.
pub fn deliverable(event: &StateEvent) -> bool {
    !matches!(event.state.as_str(), "awaiting_input" | "dead") && !event.overlay
}

/// Memória das bordas, que sobrevive ao `/clear`: a sugestão e a entrega não são da conversa.
#[derive(Default)]
pub struct Edges { suggestion: String, deliverable: bool }

impl Edges {
    /// A sugestão mudou desde a última que saiu. `None`: fato desconhecido, nada muda.
    pub fn suggestion(&mut self, now: Option<&str>) -> Option<String> {
        let now = now.filter(|s| *s != self.suggestion)?;
        self.suggestion = now.to_owned();
        Some(self.suggestion.clone())
    }

    /// Subiu a borda de entregável. Começa em falso: o primeiro estado entregável depois de nascer
    /// também entrega (o que ficou na fila durante um reinício).
    pub fn deliverable(&mut self, event: &StateEvent) -> bool {
        let now = deliverable(event);
        let edge = now && !self.deliverable;
        self.deliverable = now;
        edge
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_problem_like_python() {
        let v = |error: Option<&str>, message: Option<&str>, stalled: Option<&str>| RuntimeView {
            error: error.map(Into::into), message: message.map(Into::into), input_stalled: stalled.map(Into::into) };
        assert_eq!(runtime_problem(&v(None, None, None)), None);
        assert_eq!(runtime_problem(&v(Some("queue_io"), Some("disco"), Some("composer_busy"))),
            Some(("runtime_falhou".into(), "queue_io: disco".into())), "erro vence a entrada parada");
        assert_eq!(runtime_problem(&v(Some("queue_io"), Some(""), None)), Some(("runtime_falhou".into(), "queue_io".into())));
        assert_eq!(runtime_problem(&v(None, None, Some("capture_utf8"))),
            Some(("terminal_input_capture_failed".into(), "capture_utf8".into())));
        assert_eq!(runtime_problem(&v(None, None, Some("x".repeat(80).as_str()))).unwrap(),
            ("terminal_input_stalled".into(), "x".repeat(60)));
        assert!(is_runtime_problem("terminal_input_stalled") && !is_runtime_problem("terminal_observacao_falhou"));
    }
}
