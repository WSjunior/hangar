// crates/hangar-server/src/transcript/mod.rs
//! Leitura das conversas do Claude e do Codex, portada de backend/app/transcript.py,
//! adapters/codex/rollout.py e pqueue.py. A saída tem que sair igual à do Python: os aparelhos
//! deduplicam por id, e um id diferente na troca vira mensagem repetida na tela.

mod py;
pub mod pyjson;

use serde_json::Value;

pub use py::ts_of_iso;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Claude,
    ClaudeHeadless,
    Codex,
}

impl Provider {
    pub fn parse(s: &str) -> Option<Provider> {
        match s {
            "claude" => Some(Self::Claude),
            "claude-headless" => Some(Self::ClaudeHeadless),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::ClaudeHeadless => "claude-headless",
            Self::Codex => "codex",
        }
    }
}

/// Linha crua do transcript como JSON, com surrogate solto trocado por U+FFFD (`scrub_surrogates`,
/// models.py:22). None em linha em branco ou que não é JSON.
pub fn decode_line(raw: &[u8]) -> Option<Value> {
    let text = String::from_utf8_lossy(raw);
    let mut v = pyjson::loads_lossless(py::strip(&text))?;
    py::scrub_value(&mut v);
    Some(v)
}
