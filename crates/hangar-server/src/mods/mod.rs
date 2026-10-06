//! Interface dos mods do Claude Code no Hangar: faixa, painéis, avisos, cliques, digitação e cópia.
//! Nesta fase só a fonte "superfície": a sessão sem terminal atendida pelo Rust, em que o servidor
//! entra como superfície remota `desktop` do `claude -p`.
pub mod model;
pub mod tree;
pub mod state;
pub mod surface;
pub mod routes;
