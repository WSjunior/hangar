//! Último `state` publicado por cada `Monitor` vivo. A lista o lê em vez de capturar o pane de
//! novo: lista e chat dizem a mesma coisa e a captura não dobra. Entra a cada publicação e sai
//! quando o `Monitor` acaba, então o mapa tem o tamanho dos `Monitor`s vivos.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use hangar_api::state::StateEvent;

struct Entry { owner: u64, sid: Option<String>, event: Arc<StateEvent> }

#[derive(Default)]
pub struct Published(Mutex<HashMap<String, Entry>>);

impl Published {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> { self.0.lock().unwrap_or_else(|e| e.into_inner()) }

    /// `owner` cresce a cada `Monitor`: o que está saindo (abortado no meio da publicação) não
    /// sobrescreve nem apaga o que nasceu no lugar dele com o mesmo nome.
    pub fn set(&self, owner: u64, name: &str, sid: Option<String>, event: Arc<StateEvent>) {
        let mut map = self.lock();
        if map.get(name).is_some_and(|e| e.owner > owner) {
            return;
        }
        map.insert(name.to_owned(), Entry { owner, sid, event });
    }

    pub fn clear(&self, owner: u64, name: &str) {
        let mut map = self.lock();
        if map.get(name).is_some_and(|e| e.owner == owner) {
            map.remove(name);
        }
    }

    /// O estado da conversa `sid`: depois do `/clear`, o da anterior não vale até a próxima rodada.
    pub fn get(&self, name: &str, sid: Option<&str>) -> Option<Arc<StateEvent>> {
        self.lock().get(name).filter(|e| e.sid.as_deref() == sid).map(|e| e.event.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_owner_does_not_clear_and_sid_must_match() {
        let p = Published::default();
        let ev = |s: &str| Arc::new(StateEvent { state: s.into(), ..Default::default() });
        p.set(1, "s", Some("a".into()), ev("idle"));
        p.set(2, "s", Some("a".into()), ev("working"));
        p.set(1, "s", Some("a".into()), ev("idle"));
        p.clear(1, "s");
        assert_eq!(p.get("s", Some("a")).map(|e| e.state.clone()).as_deref(), Some("working"));
        assert!(p.get("s", Some("b")).is_none(), "conversa nova não herda o estado da anterior");
        p.clear(2, "s");
        assert!(p.get("s", Some("a")).is_none());
    }
}
