//! Interface dos mods do Claude Code no Hangar: faixa, painéis, avisos, cliques, digitação e cópia.
//! Duas fontes alimentam o mesmo estado: a "superfície" (a sessão sem terminal atendida pelo Rust, em
//! que o servidor entra como superfície remota `desktop` do `claude -p`) e o "terminal" (a sessão com
//! terminal que o Rust atende, em que o plugin do Hangar manda a faixa pela ponte e o servidor lê a
//! tela e clica pelo executor do terminal).
pub mod model;
pub mod tree;
pub mod state;
pub mod surface;
pub mod routes;
pub mod bridge;
pub mod screen;
mod http;
