//! Senha de administrador no Linux (spec "Senha de administrador"): a `app_sudo` do script chama este app pelo
//! `HANGAR_ASKPASS` e entrega a senha impressa ao `sudo -S`. O app pede uma vez numa janela com o motivo, confere com
//! `sudo -v` e guarda só na memória até o `##HANGAR-FIM##`; `--retry` a descarta e pede de novo. O agente do plano 3 nunca
//! recebe este ambiente.
use std::{io::Write, path::{Path, PathBuf}, process::{Command, Stdio}};
use super::system::{find_program, hidden};

pub(crate) fn new_code() -> String {
    let mut raw = [0u8; 16];
    let _ = ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut raw);
    raw.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Answer { Refuse, Known(String), Ask }

/// O código desta instalação e a senha já conferida. Nunca vai ao disco.
#[derive(Default)]
pub(crate) struct Vault { code: String, password: Option<String> }

impl Vault {
    pub(crate) fn new(code: String) -> Self { Self { code, password: None } }
    pub(crate) fn code(&self) -> &str { &self.code }
    pub(crate) fn answer(&self, code: &str) -> Answer {
        if self.code.is_empty() || code != self.code { return Answer::Refuse; }
        match &self.password { Some(password) => Answer::Known(password.clone()), None => Answer::Ask }
    }
    pub(crate) fn remember(&mut self, password: String) { self.password = Some(password); }
    /// O `sudo -S` do script recusou a senha guardada: esquece só ela, o código continua.
    pub(crate) fn reject(&mut self) { self.password = None; }
    pub(crate) fn password(&self) -> Option<&str> { self.password.as_deref() }
    pub(crate) fn forget(&mut self) { self.password = None; self.code.clear(); }
}

/// Confere a senha antes de guardá-la: errada, a janela pede de novo. `-k` ignora a senha que o sudo já tinha
/// (medido em sudo-rs 0.2.8 e sudo 1.9.17: `-S -k -p '' -v` aceita; certa sai 0, errada sai 1).
pub(crate) fn sudo_accepts(password: &str, path: &str) -> Result<bool, String> {
    let sudo = find_program("sudo", path).ok_or("sudo")?;
    let mut child = hidden(&mut Command::new(sudo)).args(["-S", "-k", "-v", "-p", ""]).env("PATH", path)
        .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() { let _ = writeln!(stdin, "{password}"); }
    Ok(child.wait().map_err(|e| e.to_string())?.success())
}

pub(crate) fn wrapper_text(exe: &Path) -> String {
    format!("#!/bin/sh\nexec '{}' --askpass \"$@\"\n", exe.to_string_lossy().replace('\'', "'\\''"))
}

/// O `HANGAR_ASKPASS` guarda só um caminho: o `--askpass` mora neste atalho, que repassa motivo e `--retry`.
pub(crate) fn write_wrapper(dir: &Path, exe: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("askpass.sh");
    std::fs::write(&path, wrapper_text(exe))?;
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_long_and_fresh() {
        let (a, b) = (new_code(), new_code());
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn wrong_code_is_refused_without_asking() {
        let mut vault = Vault::new("abc".into());
        assert_eq!(vault.answer("xyz"), Answer::Refuse);
        assert_eq!(vault.answer(""), Answer::Refuse);
        assert_eq!(vault.answer("abc"), Answer::Ask);
        vault.remember("s3nha".into());
        assert_eq!(vault.answer("abc"), Answer::Known("s3nha".into()));
        // `##HANGAR-FIM##`: a senha sai da memória e o código deixa de valer.
        vault.forget();
        assert_eq!(vault.password(), None);
        assert_eq!(vault.answer("abc"), Answer::Refuse);
    }

    #[test]
    fn rejected_password_is_asked_again_with_the_same_code() {
        let mut vault = Vault::new("abc".into());
        vault.remember("errada".into());
        // `--retry`: o `sudo -S` recusou a guardada; o código continua valendo e a janela abre de novo.
        vault.reject();
        assert_eq!(vault.password(), None);
        assert_eq!(vault.answer("abc"), Answer::Ask);
    }

    #[test]
    fn wrapper_quotes_the_executable_path() {
        assert_eq!(wrapper_text(Path::new("/home/dev/it's/hangar-native")),
            "#!/bin/sh\nexec '/home/dev/it'\\''s/hangar-native' --askpass \"$@\"\n");
    }

    #[cfg(unix)]
    #[test]
    fn wrapper_is_private_and_executable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("hangar-askpass-{}", std::process::id()));
        let path = write_wrapper(&dir, Path::new("/usr/bin/hangar-native")).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o700);
        let _ = std::fs::remove_dir_all(dir);
    }
}
