//! Navegador embutido do painel lateral. Motor por sistema, mesma superfície de chamada:
//! Windows/macOS usam o webview do sistema (wry) como janela filha; Linux, um Chromium sem janela pintado pela GPUI.
//! Windows e Linux falam CDP, e neles o app atende o hangar-preview e a tela remota.
pub mod model;
pub mod preview_fmt;
#[cfg(not(target_os = "macos"))]
pub mod chrome_import;
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub mod control;
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub mod server;
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub mod relay;
#[cfg(target_os = "windows")]
pub mod cdp;
/// Mesmo nome nos dois sistemas: uma sessão CDP no pipe do Chromium faz o papel do CDP do WebView2.
#[cfg(target_os = "linux")]
pub mod cdp {
    pub use super::chromium::pipe::Session as Cdp;
}

#[cfg(target_os = "linux")]
mod chromium;
#[cfg(target_os = "linux")]
pub use chromium::Engine;

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
