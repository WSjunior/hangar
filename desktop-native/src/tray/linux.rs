//! Bandeja do Linux: um StatusNotifierItem no D-Bus da sessão, servido pelo próprio processo.
use super::TrayEvent;
use ksni::blocking::TrayMethods;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

const ICON_SIZE: u32 = 64;

struct Item { events: async_channel::Sender<TrayEvent>, online: Arc<AtomicBool>, icon: Vec<ksni::Icon> }

impl Item {
    fn entry(label: &str, event: TrayEvent) -> ksni::MenuItem<Self> {
        ksni::menu::StandardItem {
            label: crate::i18n::tr(label),
            activate: Box::new(move |item: &mut Self| { let _ = item.events.try_send(event); }),
            ..Default::default()
        }.into()
    }
}

impl ksni::Tray for Item {
    fn id(&self) -> String { "com.hangar.native".into() }
    fn title(&self) -> String { "Hangar".into() }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> { self.icon.clone() }
    fn activate(&mut self, _x: i32, _y: i32) { let _ = self.events.try_send(TrayEvent::Toggle); }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![Self::entry("tray_open", TrayEvent::Show), ksni::MenuItem::Separator, Self::entry("tray_quit", TrayEvent::Quit)]
    }
    fn watcher_online(&self) {
        self.online.store(true, Ordering::Relaxed);
        let _ = self.events.try_send(TrayEvent::Host(true));
    }
    // `true`: o serviço continua de pé esperando a bandeja voltar.
    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        self.online.store(false, Ordering::Relaxed);
        let _ = self.events.try_send(TrayEvent::Host(false));
        true
    }
}

pub struct Handle { tray: ksni::blocking::Handle<Item>, online: Arc<AtomicBool> }

impl Handle {
    pub fn online(&self) -> bool { self.online.load(Ordering::Relaxed) }
    pub fn refresh(&self) { let _ = self.tray.update(|_| {}); }
}

impl Drop for Handle {
    fn drop(&mut self) { let _ = self.tray.shutdown(); }
}

pub fn start(events: async_channel::Sender<TrayEvent>) -> Result<Handle, String> {
    let online = Arc::new(AtomicBool::new(true));
    // O app pode subir antes da barra: sem bandeja o serviço fica esperando e avisa por `watcher_offline`.
    let tray = Item { events, online: online.clone(), icon: icon() }.assume_sni_available(true).spawn().map_err(|e| e.to_string())?;
    Ok(Handle { tray, online })
}

/// Pixmap em ARGB32, na ordem que o protocolo pede. Não depende do tema de ícones instalado.
fn icon() -> Vec<ksni::Icon> {
    let Ok(image) = image::load_from_memory(include_bytes!("../../assets/brand/icon.png")) else { return Vec::new() };
    let mut data = image.resize_exact(ICON_SIZE, ICON_SIZE, image::imageops::FilterType::Lanczos3).into_rgba8().into_raw();
    for pixel in data.chunks_exact_mut(4) { pixel.rotate_right(1); }
    vec![ksni::Icon { width: ICON_SIZE as i32, height: ICON_SIZE as i32, data }]
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    #[test]
    fn icon_is_one_argb_pixmap_with_alpha_first() {
        let icons = super::icon();
        assert_eq!(icons.len(), 1);
        let size = super::ICON_SIZE as usize;
        assert_eq!(icons[0].data.len(), size * size * 4);
        // O centro da marca é opaco: em ARGB o primeiro byte do pixel é o alfa.
        let center = (size / 2 * size + size / 2) * 4;
        assert_eq!(icons[0].data[center], 255);
    }
}
