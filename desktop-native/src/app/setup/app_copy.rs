//! Com `--sem-nativo` o script não baixa o app (o `install-native` baixaria a `native-latest` por cima do binário aberto):
//! o próprio app se copia para o caminho de sempre, onde o atalho, o link `hangar://` e o atualizador o procuram, e põe o
//! atalho e o `hangar://` como o `install-native` põe.
use std::{path::{Path, PathBuf}, process::{Command, Stdio}};

const WINDOWS_SHORTCUT: &str = r#"$app = '__APP__'
$ws = New-Object -ComObject WScript.Shell
foreach ($pasta in @([Environment]::GetFolderPath('Programs'), [Environment]::GetFolderPath('DesktopDirectory'))) {
  $l = $ws.CreateShortcut((Join-Path $pasta 'Hangar.lnk')); $l.TargetPath = $app; $l.WorkingDirectory = (Split-Path -Parent $app)
  $l.IconLocation = "$app,0"; $l.Description = 'Hangar'; $l.Save()
}
$c = 'HKCU:\Software\Classes\hangar'
New-Item -Path "$c\shell\open\command" -Force | Out-Null
Set-ItemProperty -Path $c -Name '(default)' -Value 'URL:Hangar'
Set-ItemProperty -Path $c -Name 'URL Protocol' -Value ''
Set-ItemProperty -Path "$c\shell\open\command" -Name '(default)' -Value ('"{0}" "%1"' -f $app)"#;

pub(crate) fn target(windows: bool, home: &Path, local_app_data: Option<&Path>) -> Option<PathBuf> {
    if windows { Some(local_app_data?.join("Programs").join("Hangar").join("Hangar.exe")) } else { Some(home.join(".local/bin/hangar-native")) }
}

/// O mesmo `.desktop` do `tools/install-linux.sh`.
pub(crate) fn desktop_entry(exe: &Path) -> String {
    format!("[Desktop Entry]\nType=Application\nName=Hangar\nExec=\"{}\" %u\nIcon=com.hangar.native\nTerminal=false\nCategories=Development;\nMimeType=x-scheme-handler/hangar;\nStartupWMClass=com.hangar.native\n", exe.display())
}

pub(crate) fn windows_shortcut_script(app: &Path) -> String { WINDOWS_SHORTCUT.replace("__APP__", &app.to_string_lossy().replace('\'', "''")) }

fn same_file(a: &Path, b: &Path) -> bool { matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b) }

/// Já sendo ele (assistente reaberto pelo menu), não faz nada.
pub(crate) fn copy_self(exe: &Path, target: &Path) -> Result<(), String> {
    if same_file(exe, target) { return Ok(()); }
    std::fs::create_dir_all(target.parent().ok_or("target")?).map_err(|e| e.to_string())?;
    let mut new = target.as_os_str().to_owned();
    new.push(".new");
    let new = PathBuf::from(new);
    std::fs::copy(exe, &new).map_err(|e| e.to_string())?;
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    // Rename e não cópia direta: por cima de um binário aberto o Linux daria "Text file busy".
    std::fs::rename(&new, target).map_err(|e| { let _ = std::fs::remove_file(&new); e.to_string() })
}

/// A marca que o passo de atualização confere (`install-native`): só sai com o `hangar://` registrado.
fn mark_scheme() {
    if let Some(home) = std::env::home_dir() {
        let dir = home.join(".hangar").join("native");
        if std::fs::create_dir_all(&dir).is_ok() { let _ = std::fs::write(dir.join("scheme-hangar"), ""); }
    }
}

#[cfg(target_os = "linux")]
fn install_shortcut(exe: &Path) -> Result<(), String> {
    const ICON: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/brand/icon.png"));
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .or_else(|| std::env::home_dir().map(|home| home.join(".local/share"))).ok_or("HOME")?;
    let icons = data.join("icons/hicolor");
    let icon = icons.join("512x512/apps/com.hangar.native.png");
    std::fs::create_dir_all(icon.parent().ok_or("icon")?).map_err(|e| e.to_string())?;
    std::fs::write(&icon, ICON).map_err(|e| e.to_string())?;
    let apps = data.join("applications");
    std::fs::create_dir_all(&apps).map_err(|e| e.to_string())?;
    std::fs::write(apps.join("com.hangar.native.desktop"), desktop_entry(exe)).map_err(|e| e.to_string())?;
    let run = |program: &str, args: &[&str]| super::system::hidden(&mut Command::new(program)).args(args)
        .stdout(Stdio::null()).stderr(Stdio::null()).status();
    // Cache velho de ícones esconde o novo; banco de .desktop velho esconde o atalho. Sem as ferramentas, segue.
    let _ = run("gtk-update-icon-cache", &["-f", "-t", &icons.to_string_lossy()]);
    let _ = run("update-desktop-database", &[&apps.to_string_lossy()]);
    match run("xdg-mime", &["default", "com.hangar.native.desktop", "x-scheme-handler/hangar"]) {
        Ok(status) if status.success() => mark_scheme(),
        // Sem xdg-mime o .desktop com MimeType é tudo que dá para deixar, como no install-linux.sh.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => mark_scheme(),
        _ => {}
    }
    Ok(())
}

#[cfg(windows)]
fn install_shortcut(exe: &Path) -> Result<(), String> {
    super::system::powershell(&windows_shortcut_script(exe))?;
    mark_scheme();
    Ok(())
}

#[cfg(not(any(target_os = "linux", windows)))]
fn install_shortcut(_: &Path) -> Result<(), String> { Err("unsupported".into()) }

pub(crate) fn install_self(exe: &Path) -> Result<PathBuf, String> {
    let home = std::env::home_dir().ok_or("HOME")?;
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let target = target(cfg!(windows), &home, local.as_deref()).ok_or("LOCALAPPDATA")?;
    copy_self(exe, &target)?;
    install_shortcut(&target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_are_the_install_native_paths() {
        assert_eq!(target(false, Path::new("/home/dev"), None), Some(PathBuf::from("/home/dev/.local/bin/hangar-native")));
        assert_eq!(target(true, Path::new(r"C:\Users\dev"), Some(Path::new(r"C:\Users\dev\AppData\Local"))),
            Some(Path::new(r"C:\Users\dev\AppData\Local").join("Programs").join("Hangar").join("Hangar.exe")));
        assert_eq!(target(true, Path::new(r"C:\Users\dev"), None), None);
    }

    #[test]
    fn desktop_entry_matches_install_linux_sh() {
        let entry = desktop_entry(Path::new("/home/dev/.local/bin/hangar-native"));
        assert!(entry.contains("Exec=\"/home/dev/.local/bin/hangar-native\" %u\n"));
        assert!(entry.contains("MimeType=x-scheme-handler/hangar;\n"));
        assert!(entry.contains("StartupWMClass=com.hangar.native\n"));
    }

    #[test]
    fn windows_script_escapes_the_path() {
        let script = windows_shortcut_script(Path::new(r"C:\Users\d'Ávila\AppData\Local\Programs\Hangar\Hangar.exe"));
        assert!(script.starts_with(r"$app = 'C:\Users\d''Ávila\AppData\Local\Programs\Hangar\Hangar.exe'"));
        assert!(!script.contains("__APP__"));
    }

    #[test]
    fn copy_places_the_binary_and_is_a_no_op_on_itself() {
        let dir = std::env::temp_dir().join(format!("hangar-copy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("hangar-baixado");
        std::fs::write(&exe, b"binario").unwrap();
        let target = dir.join("bin/hangar-native");
        copy_self(&exe, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"binario");
        copy_self(&target, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"binario");
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o755);
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
