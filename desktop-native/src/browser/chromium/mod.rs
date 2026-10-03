//! Motor do navegador no Linux: Chromium sem janela, um alvo por sessão, CDP por pipe (sem porta de depuração).
//! Mesmo motor e mesmo CDP do WebView2 do Windows: o controlador do hangar-preview e o repasse da tela remota servem
//! aos dois sistemas.
mod engine;
mod launch;
pub mod pipe;

pub use engine::Engine;
