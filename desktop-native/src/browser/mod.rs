//! Navegador embutido do painel lateral. Motor por sistema, mesma superfície de chamada:
//! Windows/macOS usam o webview do sistema (wry) como janela filha; Linux, WPE.
pub mod model;
pub mod preview_fmt;
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub mod control;
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub mod server;
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub mod relay;
#[cfg(target_os = "windows")]
pub mod cdp;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::Engine;

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod wry_engine;
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub use wry_engine::Engine;

pub enum Event {
    State(model::PageState),
    /// Quadro novo para desenhar; só o motor que pinta pela GPUI manda.
    // Fica em todo sistema para o `if let` de quem recebe não virar padrão irrefutável.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Frame,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pointer {
    Down,
    Up,
    Move,
}
