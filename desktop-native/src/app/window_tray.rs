//! A janela e a bandeja: o ícone segue a opção de Configurações → Geral.
use super::*;
use crate::tray::{Tray, TrayEvent};

pub(super) struct WindowTray {
    events: async_channel::Sender<TrayEvent>,
    pub(super) icon: Option<Tray>,
    /// O ícone está sendo criado em outra thread.
    starting: bool,
    pub(super) error: Option<String>,
}

impl WindowTray {
    pub(super) fn new(events: async_channel::Sender<TrayEvent>) -> Self { Self { events, icon: None, starting: false, error: None } }
}

impl Hangar {
    /// Põe o ícone de acordo com a opção: cria quando liga, remove quando desliga.
    pub(super) fn sync_tray(&mut self, cx: &mut Context<Self>) {
        if !(crate::tray::SUPPORTED && appearance::get().keep_in_tray) {
            self.window_tray.icon = None;
            self.window_tray.error = None;
            cx.notify();
            return;
        }
        if self.window_tray.icon.is_some() || self.window_tray.starting { return; }
        self.window_tray.starting = true;
        let events = self.window_tray.events.clone();
        let task = self.runtime.spawn_blocking(move || crate::tray::start(events));
        cx.spawn(async move |this, cx| {
            let result = task.await.unwrap_or_else(|e| Err(e.to_string()));
            let _ = this.update(cx, |this, cx| {
                this.window_tray.starting = false;
                match result {
                    // Desligada enquanto o ícone subia: o que chegou é solto, e some.
                    Ok(icon) if appearance::get().keep_in_tray => { this.window_tray.icon = Some(icon); this.window_tray.error = None; }
                    Ok(_) => {}
                    Err(reason) => { eprintln!("tray: {reason}"); this.window_tray.error = Some(reason); }
                }
                cx.notify();
            });
        }).detach();
    }

    pub(super) fn on_tray_event(&mut self, event: TrayEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            TrayEvent::Toggle | TrayEvent::Show => window.activate_window(),
            TrayEvent::Quit => cx.quit(),
            TrayEvent::Host(_) => cx.notify(),
        }
    }
}
