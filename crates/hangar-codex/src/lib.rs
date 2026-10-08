//! Protocolo do app-server do Codex: tipos conferidos contra o schema da versão conferida e o
//! cliente JSON-RPC. Sem axum: serve ao servidor e ao lançador.
pub mod proto;
pub mod version;
pub mod client;
#[cfg(test)]
mod schema_check;
