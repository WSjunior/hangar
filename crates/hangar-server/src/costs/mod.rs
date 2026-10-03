//! Custos e uso: leitores dos transcripts, índice incremental próprio e relatórios.
//! Porte dos módulos de custos e uso do backend; o Python é a referência dos golden.
pub mod pricing;
pub mod py;
pub mod index;
pub mod rows;
pub mod areas;
pub mod uso_rules;
