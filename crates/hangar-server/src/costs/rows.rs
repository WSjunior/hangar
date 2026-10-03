//! Linhas persistidas na mesma ordem dos campos dos leitores Python.

use super::py::LocalTs;
use serde::{Deserialize, Serialize};

pub use crate::costs::areas::AreaEntries;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UsageRow {
    pub ts: LocalTs,
    pub source: String,
    pub provider: String,
    pub model: String,
    pub project: String,
    pub session_id: String,
    pub input: i64,
    pub output: i64,
    pub cache_write: i64,
    pub cache_read: i64,
    pub subagente: bool,
    pub account_id: Option<String>,
    pub codex_long_context: bool,
    pub cache_write_1h: i64,
    pub fast: bool,
    pub regravado: i64,
    pub regravado_1h: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UsoLinha {
    pub dia: String,
    pub cwd: String,
    pub model: String,
    pub tipo: String,
    pub nome: String,
    pub plugin: String,
    pub detalhe: String,
    pub origem: String,
    pub chamadas: i64,
    pub ctx_chars: i64,
    pub tokens_est: i64,
    pub input: i64,
    pub output: i64,
    pub cache_write: i64,
    pub cache_read: i64,
    pub cache_write_1h: i64,
    pub fast: bool,
    pub ocupados: i64,
    pub respostas: i64,
    pub ocupados_eq: i64,
    pub fonte: String,
    pub subagente: bool,
    pub session_id: String,
}

pub struct FoldOutput {
    pub costs: Vec<UsageRow>,
    pub usage: Vec<UsoLinha>,
    pub areas: Option<AreaEntries>,
}
