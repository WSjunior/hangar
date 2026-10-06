//! Clique de mod numa sessão com terminal que o Rust atende (Linux, macOS e Windows com psmux): lê a tela,
//! ativa a aba, alcança o botão e clica pelo mouse; o teclado é a reserva só nos casos medidos (T3 a T6,
//! T9). O `backend/app/plugin_click.py` de hoje fica só para o processo sem Rust. O pane é falado por
//! operações do executor do terminal (`TerminalHandle::pane`), seriais com a entrada e fora do diário da
//! fila.
use std::future::Future;
use std::pin::Pin;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use super::model::ModsError;
use crate::terminal_input::PaneFormats;

/// O que o clique pede ao pane. Linha e coluna a partir de 0.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PaneOp {
    Formats,
    Clients,
    Screen,
    Mouse { row: u16, col: u16 },
    Wheel { row: u16, col: u16, down: bool },
    Keys(Vec<String>),
    Resize { columns: u16, rows: u16 },
    /// Reserva o pane ao clique por `millis`: o executor guarda os comandos e a drenagem da fila até o
    /// `Release` ou o fim do prazo (C6).
    Hold { millis: u64 },
    Release,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PaneReply {
    Formats(PaneFormats),
    Clients(usize),
    Screen(String),
    Done,
}

pub type PaneFuture = Pin<Box<dyn Future<Output = Result<PaneReply, ModsError>> + Send>>;

/// O pane da sessão: o executor do terminal, ou um de mentira nos testes.
pub trait Pane: Send + Sync {
    /// `start_by`: a operação que chega à vez dela depois disso não age e volta recusada. É o prazo de
    /// quem pediu menos o que a ação ainda precisa (C1): uma tecla que ficou na caixa do executor não
    /// chega ao terminal depois de o app ouvir que o clique falhou.
    fn op(&self, op: PaneOp, start_by: Instant) -> PaneFuture;
}
