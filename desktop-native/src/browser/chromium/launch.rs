//! Acha o Chromium da máquina e o sobe sem janela, falando CDP por pipe (fd 3 e 4), sem porta de depuração.
use std::{
    env,
    fs::File,
    io::{self, PipeReader, PipeWriter},
    os::{fd::AsRawFd, unix::process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::OnceLock,
};

/// Ordem de busca: o pedido explícito, o baixado pelo instalador do app e o do sistema.
const SYSTEM: [&str; 4] = ["google-chrome-stable", "google-chrome", "chromium", "chromium-browser"];

/// Achado uma vez por processo: roda a cada desenho do menu do painel.
pub fn find() -> Result<&'static Path, String> {
    static FOUND: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    FOUND.get_or_init(|| search(env::var_os("HANGAR_CHROMIUM").map(PathBuf::from), downloaded(), env::var_os("PATH")))
        .as_ref().map(PathBuf::as_path).map_err(Clone::clone)
}

fn downloaded() -> Option<PathBuf> {
    Some(PathBuf::from(env::var_os("HOME")?).join(".hangar/native/chromium/chrome-headless-shell"))
}

fn search(explicit: Option<PathBuf>, downloaded: Option<PathBuf>, path: Option<std::ffi::OsString>) -> Result<PathBuf, String> {
    if let Some(bin) = explicit {
        return usable(&bin).then_some(bin.clone()).ok_or_else(|| format!("HANGAR_CHROMIUM aponta para {}, que nao e um executavel", bin.display()));
    }
    if let Some(bin) = downloaded.filter(|b| usable(b)) { return Ok(bin); }
    let dirs: Vec<PathBuf> = path.map(|p| env::split_paths(&p).collect()).unwrap_or_default();
    for name in SYSTEM {
        for dir in &dirs {
            let bin = dir.join(name);
            if usable(&bin) && !snap(&bin) { return Ok(bin); }
        }
    }
    Err("nenhum Chrome ou Chromium encontrado (instale o google-chrome ou o chromium, ou rode scripts/install-native.sh)".into())
}

/// O Chromium do snap roda confinado e não lê o perfil em pasta oculta da home (`~/.config`): vale como ausente.
/// Ele aparece como link para `/snap/...` ou para o `snap`, ou como script em `/usr/bin` que chama o `/snap/bin`.
fn snap(bin: &Path) -> bool {
    let real = std::fs::canonicalize(bin).unwrap_or_else(|_| bin.to_path_buf());
    if real.starts_with("/snap/") || real.file_name().is_some_and(|n| n == "snap") { return true; }
    let mut head = Vec::new();
    let _ = std::io::Read::read_to_end(&mut std::io::Read::take(match File::open(&real) { Ok(f) => f, Err(_) => return false }, 4096), &mut head);
    head.starts_with(b"#!") && head.windows(6).any(|w| w == b"/snap/")
}

fn usable(bin: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(bin).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// O `chrome-headless-shell` já é sem janela; o Chrome completo precisa pedir.
fn is_shell(bin: &Path) -> bool {
    bin.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.contains("headless-shell") || n.contains("headless_shell"))
}

pub fn args(bin: &Path, profile: &Path, scale: f32) -> Vec<String> {
    let mut args = vec![
        "--remote-debugging-pipe".into(),
        format!("--user-data-dir={}", profile.display()),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        // Escala inteira, arredondada para cima: com escala fracionária o Chromium arredonda o viewport (800 vira 801)
        // e o `layout` do hangar-preview deixa de ser exato. O quadro sai no máximo do tamanho físico do painel.
        format!("--force-device-scale-factor={}", scale.ceil().max(1.)),
        // Sem isto a página do agente, que ninguém está olhando, para de rodar timers e de pintar.
        "--disable-background-timer-throttling".into(),
        "--disable-renderer-backgrounding".into(),
        "--disable-backgrounding-occluded-windows".into(),
    ];
    if !is_shell(bin) { args.insert(0, "--headless".into()); }
    args
}

/// O Chromium lê de `writer` (fd 3 dele) e escreve em `reader` (fd 4 dele).
pub struct Launched {
    pub child: Child,
    pub reader: PipeReader,
    pub writer: PipeWriter,
}

pub fn spawn(bin: &Path, profile: &Path, scale: f32) -> io::Result<Launched> {
    let (their_in, writer) = io::pipe()?;
    let (reader, their_out) = io::pipe()?;
    let (fd_in, fd_out) = (their_in.as_raw_fd(), their_out.as_raw_fd());
    let mut command = Command::new(bin);
    command.args(args(bin, profile, scale)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(log_file());
    // SAFETY: só chamadas async-signal-safe entre o fork e o exec.
    unsafe {
        command.pre_exec(move || {
            // Copia para longe antes: uma das pontas pode já ser o fd 3 ou 4.
            let a = libc::fcntl(fd_in, libc::F_DUPFD, 10);
            let b = libc::fcntl(fd_out, libc::F_DUPFD, 10);
            if a < 0 || b < 0 || libc::dup2(a, 3) < 0 || libc::dup2(b, 4) < 0 { return Err(io::Error::last_os_error()); }
            // App que morre sem fechar leva o Chromium junto.
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            Ok(())
        });
    }
    let child = command.spawn()?;
    drop((their_in, their_out));
    Ok(Launched { child, reader, writer })
}

/// A saída do Chromium vai para o diário do app, não para o terminal de quem o abriu.
fn log_file() -> Stdio {
    crate::appearance::dir()
        .and_then(|dir| File::options().create(true).append(true).open(dir.join("chromium.log")).ok())
        .map_or_else(Stdio::null, Stdio::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exe(dir: &Path, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join(name);
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    #[test]
    fn search_prefers_explicit_then_downloaded_then_system() {
        let dir = std::env::temp_dir().join(format!("hangar-chromium-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let system = exe(&dir, "chromium");
        let shell = exe(&dir, "chrome-headless-shell");
        let path = Some(dir.clone().into_os_string());
        assert_eq!(search(None, None, path.clone()), Ok(system.clone()));
        assert_eq!(search(None, Some(shell.clone()), path.clone()), Ok(shell.clone()));
        assert_eq!(search(Some(system.clone()), Some(shell.clone()), path.clone()), Ok(system.clone()));
        assert!(search(Some(dir.join("nada")), None, path).is_err());
        assert!(search(None, Some(dir.join("nada")), None).is_err());
        // O wrapper do Ubuntu em /usr/bin, que só chama o snap, não conta.
        std::fs::write(&system, "#!/bin/sh\nexec /snap/bin/chromium \"$@\"\n").unwrap();
        assert!(search(None, None, Some(dir.clone().into_os_string())).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_the_full_chrome_asks_for_headless() {
        let profile = Path::new("/p");
        assert_eq!(args(Path::new("/usr/bin/google-chrome-stable"), profile, 2.0)[0], "--headless");
        let shell = args(Path::new("/x/chrome-headless-shell"), profile, 1.0);
        assert!(!shell.contains(&"--headless".to_string()));
        assert!(shell.contains(&"--force-device-scale-factor=1".to_string()));
        assert!(args(Path::new("/x/chrome-headless-shell"), profile, 1.2).contains(&"--force-device-scale-factor=2".to_string()));
    }
}
