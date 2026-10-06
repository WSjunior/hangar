//! Registro nativo `waiting` que o pane contradisse (`hook_state.demote_awaiting`): vale `idle` até
//! o Claude escrever outra versão do estado. O arquivo é do Claude e nunca é escrito; quem decide
//! rebaixar é a lista, e a lista e o `Monitor` leem o mesmo mapa.
use std::sync::Mutex;

use crate::list::capped::{Capped, SESSION_CAP};

/// session id → `statusUpdatedAt` (s) do registro rebaixado.
pub struct Demoted(Mutex<Capped<String, f64>>);

impl Default for Demoted {
    fn default() -> Self { Self(Mutex::new(Capped::new(SESSION_CAP))) }
}

impl Demoted {
    fn lock(&self) -> std::sync::MutexGuard<'_, Capped<String, f64>> { self.0.lock().unwrap_or_else(|e| e.into_inner()) }

    pub fn demote(&self, sid: &str, version: f64) { self.lock().insert(sid.to_owned(), version); }

    /// O registro desta versão foi rebaixado? Versão nova desfaz o rebaixamento.
    pub fn applies(&self, sid: &str, version: f64) -> bool {
        let mut map = self.lock();
        match map.peek(sid).copied() {
            Some(v) if v == version => true,
            Some(_) => {
                map.remove(sid);
                false
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::list::facts_files::HookStates;
    use std::sync::Arc;

    fn native(dir: &std::path::Path, status: &str, at: u64) {
        std::fs::write(dir.join("sessions/7.json"),
            format!(r#"{{"sessionId": "sid", "pid": 7, "status": "{status}", "statusUpdatedAt": {at}, "updatedAt": {at}}}"#)).unwrap();
    }

    #[test]
    fn invalidated_by_file_version() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sessions")).unwrap();
        let dirs = vec![dir.path().to_path_buf()];
        native(dir.path(), "waiting", 5000);
        let demoted = Arc::new(Demoted::default());
        let mut hooks = HookStates::default();
        hooks.set_demoted(demoted.clone());
        hooks.refresh(&dirs);
        let state = |h: &HookStates| h.get_state(Some("sid"), |_| true).map(|m| m.state);
        assert_eq!(state(&hooks).as_deref(), Some("awaiting_input"));
        demoted.demote("sid", 5.0);
        assert_eq!(state(&hooks).as_deref(), Some("idle"), "rebaixado vale para quem lê o mesmo mapa");
        // Mesma versão relida (outra escrita sem mudar `statusUpdatedAt`) continua rebaixada.
        native(dir.path(), "waiting", 5000);
        hooks.refresh(&dirs);
        assert_eq!(state(&hooks).as_deref(), Some("idle"));
        native(dir.path(), "waiting", 6000);
        hooks.refresh(&dirs);
        assert_eq!(state(&hooks).as_deref(), Some("awaiting_input"), "versão nova desfaz");
        native(dir.path(), "waiting", 5000);
        hooks.refresh(&dirs);
        assert_eq!(state(&hooks).as_deref(), Some("awaiting_input"), "o rebaixamento saiu do mapa");
    }
}
