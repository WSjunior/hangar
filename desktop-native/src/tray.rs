//! Ícone na bandeja do sistema. Aqui só há o ícone e o menu: o que cada evento faz com a janela é decisão da tela.
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows as imp;

pub const SUPPORTED: bool = cfg!(any(target_os = "linux", target_os = "windows"));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// Clique no ícone.
    Toggle,
    Show,
    Quit,
    /// A bandeja do sistema apareceu (`true`) ou sumiu (`false`).
    Host(bool),
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod imp {
    pub struct Handle;
    impl Handle {
        pub fn online(&self) -> bool { false }
        pub fn refresh(&self) {}
    }
    pub fn start(_: async_channel::Sender<super::TrayEvent>) -> Result<Handle, String> { Err("unsupported".into()) }
}

/// O ícone de pé. Soltar remove o ícone da bandeja.
pub struct Tray(imp::Handle);

impl Tray {
    /// Há uma bandeja mostrando o ícone agora.
    pub fn online(&self) -> bool { self.0.online() }
    /// Relê os textos do menu (troca de idioma).
    pub fn refresh(&self) { self.0.refresh() }
}

/// Bloqueante: fala com o sistema antes de voltar. Chamar fora da thread da janela.
pub fn start(events: async_channel::Sender<TrayEvent>) -> Result<Tray, String> { imp::start(events).map(Tray) }
