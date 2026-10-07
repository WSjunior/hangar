//! Plano da conversa por voz em modo planejar: um .md por sessão, em `~/.hangar/voz/planos`.
use chrono::{DateTime, Local};
use std::{io, path::PathBuf};

const LIMIT: usize = 200_000;

pub struct PlanFile { pub path: PathBuf }

pub fn plans_dir() -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default();
    home.join(".hangar").join("voz").join("planos")
}

pub fn new_plan(session: &str, now: DateTime<Local>) -> PlanFile {
    let safe: String = session.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' }).collect();
    PlanFile { path: plans_dir().join(format!("{safe}-{}.md", now.format("%Y-%m-%d-%H%M"))) }
}

impl PlanFile {
    pub fn read(&self) -> String {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
            Err(error) => { super::log(format!("plan read failed: {error}")); String::new() }
        }
    }

    pub fn write(&self, markdown: &str) -> io::Result<()> {
        if markdown.len() > LIMIT { return Err(io::Error::new(io::ErrorKind::InvalidInput, "plano acima de 200 000 bytes")); }
        if let Some(dir) = self.path.parent() { std::fs::create_dir_all(dir)?; }
        // Tmp na mesma pasta (rename entre pastas não é atômico): o plano não fica pela metade se o app fechar.
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, markdown)?;
        std::fs::rename(&tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use core::prelude::v1::test;

    #[test]
    fn file_name_is_sanitized_and_dated() {
        let when = chrono::Local.with_ymd_and_hms(2026, 10, 7, 19, 5, 0).unwrap();
        let plan = new_plan("pm/../x y", when);
        assert_eq!(plan.path.file_name().unwrap().to_str().unwrap(), "pm-..-x-y-2026-10-07-1905.md");
    }

    #[test]
    fn write_is_atomic_and_readable() {
        let dir = std::env::temp_dir().join(format!("voice-plan-{}", std::process::id()));
        let plan = PlanFile { path: dir.join("p.md") };
        assert_eq!(plan.read(), "");
        plan.write("# Plano\n- item").unwrap();
        assert_eq!(plan.read(), "# Plano\n- item");
        assert!(std::fs::read_dir(&dir).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
        assert!(plan.write(&"x".repeat(200_001)).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
