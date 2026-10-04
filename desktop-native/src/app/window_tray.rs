//! A janela e a bandeja: o ícone segue a opção de Configurações → Geral, e fechar esconde em vez de encerrar.
use super::*;
use crate::tray::{Tray, TrayEvent};

pub(super) struct WindowTray {
    events: async_channel::Sender<TrayEvent>,
    pub(super) icon: Option<Tray>,
    /// O ícone está sendo criado em outra thread.
    starting: bool,
    pub(super) error: Option<String>,
    /// A janela está escondida na bandeja.
    pub(super) hidden: bool,
}

impl WindowTray {
    pub(super) fn new(events: async_channel::Sender<TrayEvent>) -> Self { Self { events, icon: None, starting: false, error: None, hidden: false } }
}

/// Fechar só esconde com o ícone de pé e uma bandeja que o mostre: sem isso o app ficaria vivo e invisível.
pub(super) fn hides_on_close(keep: bool, icon_online: Option<bool>) -> bool { keep && icon_online == Some(true) }

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

    pub(super) fn closes_to_tray(&self) -> bool {
        hides_on_close(appearance::get().keep_in_tray, self.window_tray.icon.as_ref().map(Tray::online))
    }

    pub(super) fn hide_to_tray(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.set_hidden(true);
        self.window_tray.hidden = true;
        let (title, body) = (tr("tray_notice_title"), tr("tray_notice_body"));
        self.runtime.spawn_blocking(move || if appearance::take_tray_notice() { show_system_notification(&title, &body) });
        cx.notify();
    }

    pub(super) fn show_from_tray(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.window_tray.hidden {
            window.set_hidden(false);
            self.window_tray.hidden = false;
        }
        window.activate_window();
        cx.notify();
    }

    pub(super) fn on_tray_event(&mut self, event: TrayEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            TrayEvent::Toggle if !self.window_tray.hidden && self.closes_to_tray() => self.hide_to_tray(window, cx),
            TrayEvent::Toggle | TrayEvent::Show => self.show_from_tray(window, cx),
            TrayEvent::Quit => cx.quit(),
            TrayEvent::Host(online) => {
                // A bandeja sumiu com a janela escondida: sem ícone não haveria como voltar.
                if !online && self.window_tray.hidden { self.show_from_tray(window, cx); }
                cx.notify();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::hides_on_close;
    use core::prelude::v1::test;

    #[test]
    fn closing_only_hides_with_the_option_on_and_a_tray_showing_the_icon() {
        assert!(hides_on_close(true, Some(true)));
        // Opção desligada, ícone que ainda não subiu (ou falhou) e bandeja ausente: fechar encerra.
        assert!(!hides_on_close(false, Some(true)));
        assert!(!hides_on_close(true, None));
        assert!(!hides_on_close(true, Some(false)));
    }
}
