//! Descoberta da lista de sessões: leitores do sistema (processos, panes) que o resto da lista
//! consome sem saber de que plataforma veio cada dado.
pub mod context;
pub mod discover;
pub mod facts_files;
pub mod mux;
pub mod plan;
pub mod procs;
pub mod sig;
