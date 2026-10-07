//! Bandeja do Linux: um StatusNotifierItem no D-Bus da sessão, servido pelo próprio processo.
use super::TrayEvent;
use ksni::blocking::TrayMethods;
use std::{sync::{Arc, Once, atomic::{AtomicBool, Ordering}}, time::Duration};

const ICON_SIZE: u32 = 64;
const WATCHER: &str = "org.kde.StatusNotifierWatcher";

/// Há uma barra mostrando os ícones. O serviço da bandeja pode sobreviver à barra (o kded assume o nome quando ela
/// cai), e aí o ícone não aparece em lugar nenhum: sem hospedeiro, fechar não pode esconder a janela.
static HOSTED: AtomicBool = AtomicBool::new(true);

/// Nome que a barra registra no barramento enquanto hospeda os ícones.
fn is_host(name: &str) -> bool { name.starts_with("org.kde.StatusNotifierHost-") || name.starts_with("org.freedesktop.StatusNotifierHost-") }

fn hosted(dbus: &zbus::blocking::fdo::DBusProxy, watcher: &zbus::blocking::Proxy) -> bool {
    if dbus.list_names().is_ok_and(|names| names.iter().any(|name| is_host(name.as_str()))) { return true; }
    let owner = |name: &'static str| zbus::names::BusName::try_from(name).ok().and_then(|name| dbus.get_name_owner(name).ok());
    let Some(service) = owner(WATCHER) else { return false };
    // O kded responde "há hospedeiro" mesmo sem barra nenhuma: com ele de dono, só o nome de um hospedeiro vale.
    if ["org.kde.kded6", "org.kde.kded5"].into_iter().any(|kded| owner(kded).as_ref() == Some(&service)) { return false; }
    watcher.get_property::<bool>("IsStatusNotifierHostRegistered").unwrap_or(false)
}

/// Acompanha os hospedeiros pela vida do processo: cada nome de hospedeiro ou do serviço que entra ou sai do barramento.
fn watch_hosts(events: async_channel::Sender<TrayEvent>) {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| { std::thread::Builder::new().name("tray-hosts".into()).spawn(move || {
        let Ok(bus) = zbus::blocking::Connection::session() else { return };
        let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&bus) else { return };
        let watcher: zbus::Result<zbus::blocking::Proxy> = zbus::blocking::proxy::Builder::new(&bus)
            .destination(WATCHER).and_then(|b| b.path("/StatusNotifierWatcher")).and_then(|b| b.interface(WATCHER))
            // A propriedade muda sem aviso: cada leitura pergunta de novo.
            .and_then(|b| b.cache_properties(zbus::proxy::CacheProperties::No).build());
        let Ok(watcher) = watcher else { return };
        let refresh = || {
            let now = hosted(&dbus, &watcher);
            if HOSTED.swap(now, Ordering::Relaxed) != now { let _ = events.try_send(TrayEvent::Host(now)); }
        };
        // Assina antes da primeira leitura: uma barra que cai nesse intervalo ainda é vista.
        let Ok(changes) = dbus.receive_name_owner_changed() else { return };
        refresh();
        for change in changes {
            if change.args().is_ok_and(|args| args.name().as_str() == WATCHER || is_host(args.name().as_str())) { refresh(); }
        }
    }).ok(); });
}

/// Registra de novo no serviço da bandeja os ícones deste processo, pelo nome que o ksni deu a eles.
fn register_again() -> zbus::Result<bool> {
    let bus = zbus::blocking::Connection::session()?;
    let own = format!("org.kde.StatusNotifierItem-{}-", std::process::id());
    let watcher = zbus::blocking::Proxy::new(&bus, WATCHER, "/StatusNotifierWatcher", WATCHER)?;
    let mut registered = false;
    for name in zbus::blocking::fdo::DBusProxy::new(&bus)?.list_names()?.iter().filter(|name| name.as_str().starts_with(&own)) {
        watcher.call_method("RegisterStatusNotifierItem", &(name.as_str(),))?;
        registered = true;
    }
    Ok(registered)
}

fn retry_registration(online: Arc<AtomicBool>, events: async_channel::Sender<TrayEvent>) {
    std::thread::spawn(move || {
        for wait in [1, 2, 4, 8, 16] {
            std::thread::sleep(Duration::from_secs(wait));
            // O ksni voltou por conta própria (o serviço trocou de dono de novo).
            if online.load(Ordering::Relaxed) { return; }
            if register_again().is_ok_and(|registered| registered) {
                online.store(true, Ordering::Relaxed);
                let _ = events.try_send(TrayEvent::Host(true));
                return;
            }
        }
    });
}

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
        vec![Self::entry("tray_open", TrayEvent::Show), Self::entry("tray_restart", TrayEvent::Restart),
            ksni::MenuItem::Separator, Self::entry("tray_quit", TrayEvent::Quit)]
    }
    fn watcher_online(&self) {
        self.online.store(true, Ordering::Relaxed);
        let _ = self.events.try_send(TrayEvent::Host(true));
    }
    // `true`: o serviço continua de pé esperando a bandeja voltar.
    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        self.online.store(false, Ordering::Relaxed);
        let _ = self.events.try_send(TrayEvent::Host(false));
        // Registro recusado: o serviço da bandeja acabou de subir e ainda não atende. O ksni não tenta de novo.
        if matches!(reason, ksni::OfflineReason::Error(_)) { retry_registration(self.online.clone(), self.events.clone()); }
        true
    }
}

pub struct Handle { tray: ksni::blocking::Handle<Item>, online: Arc<AtomicBool> }

impl Handle {
    pub fn online(&self) -> bool { self.online.load(Ordering::Relaxed) && HOSTED.load(Ordering::Relaxed) }
    // Em outra thread: `update` espera o serviço, que pode estar preso no registro com a barra reiniciando.
    pub fn refresh(&self) {
        let tray = self.tray.clone();
        std::thread::spawn(move || { let _ = tray.update(|_| {}); });
    }
}

impl Drop for Handle {
    fn drop(&mut self) { let _ = self.tray.shutdown(); }
}

pub fn start(events: async_channel::Sender<TrayEvent>) -> Result<Handle, String> {
    let online = Arc::new(AtomicBool::new(true));
    watch_hosts(events.clone());
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
