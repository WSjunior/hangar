//! Leitura de processos (`backend/app/procinfo.py`).
use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// ppid → filhos, na ordem da varredura.
pub type ChildrenMap = HashMap<i64, Vec<i64>>;

/// Acima da cadência do poll da lista (1,5 s): abaixo dela cada tique varreria de novo.
pub const CHILDREN_TTL: Duration = Duration::from_secs(3);

/// O que a lista precisa saber de um processo. Processo que morreu ou é de outro dono responde
/// vazio/`None`; só a varredura inteira falhar é erro. E/S bloqueante: em contexto async, chamar
/// por `spawn_blocking`.
pub trait ProcessView: Send + Sync {
    /// Mapa reusado por até `max_age`; `Duration::ZERO` lê de novo (sessão criada há menos de 1 s).
    fn children(&self, max_age: Duration) -> io::Result<Arc<ChildrenMap>>;
    fn argv(&self, pid: i64) -> Vec<String>;
    fn cwd(&self, pid: i64) -> Option<PathBuf>;
    /// `Ok(None)` = variável ausente; `Err` = ambiente ilegível (a exclusão de conta separa os dois).
    fn env_var(&self, pid: i64, name: &str) -> io::Result<Option<OsString>>;
    /// Nascimento em segundos desde a época.
    fn start_time(&self, pid: i64) -> Option<f64>;
    /// Destinos dos descritores abertos.
    fn fds(&self, pid: i64) -> Vec<PathBuf>;
}

#[cfg(target_os = "linux")]
pub use linux::ProcFs;

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::sync::Mutex;
    use std::time::Instant;

    pub struct ProcFs {
        root: PathBuf,
        cache: Mutex<Option<(Instant, Arc<ChildrenMap>)>>,
    }

    impl Default for ProcFs {
        fn default() -> Self { Self::with_root("/proc") }
    }

    impl ProcFs {
        /// Raiz trocável só para o teste apontar para um /proc de mentira.
        pub fn with_root(root: impl Into<PathBuf>) -> Self {
            Self { root: root.into(), cache: Mutex::new(None) }
        }

        fn path(&self, pid: i64, leaf: &str) -> PathBuf { self.root.join(pid.to_string()).join(leaf) }

        /// (ppid, nascimento em ticks). O comm (campo 2) pode ter espaço e parêntese: só o último
        /// ')' delimita.
        fn stat(&self, pid: i64) -> Option<(i64, u64)> {
            // `comm` aceita byte arbitrário (`prctl`): não pode tirar o processo do mapa.
            let raw = String::from_utf8_lossy(&std::fs::read(self.path(pid, "stat")).ok()?).into_owned();
            let mut fields = raw[raw.rfind(')')? + 1..].split_whitespace();
            let ppid = fields.nth(1)?.parse().ok()?;
            let start = fields.nth(17)?.parse().ok()?;
            Some((ppid, start))
        }

        /// Processo que some no meio da varredura fica de fora; raiz ilegível ou nenhum `stat` lido
        /// (`hidepid`) é erro, nunca um mapa vazio.
        fn scan(&self) -> io::Result<ChildrenMap> {
            let mut map = ChildrenMap::new();
            let mut seen = 0usize;
            for entry in std::fs::read_dir(&self.root)? {
                let Ok(entry) = entry else { continue };
                let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<i64>().ok()) else { continue };
                seen += 1;
                if let Some((ppid, _)) = self.stat(pid) {
                    map.entry(ppid).or_default().push(pid);
                }
            }
            if seen > 0 && map.is_empty() {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "nenhum stat legível"));
            }
            Ok(map)
        }

        fn btime(&self) -> Option<f64> {
            let raw = std::fs::read_to_string(self.root.join("stat")).ok()?;
            raw.lines().find_map(|l| l.strip_prefix("btime ")).and_then(|v| v.trim().parse().ok())
        }
    }

    /// `None` quando o sistema não responde: um nascimento inventado erraria todos os processos.
    fn clock_ticks() -> Option<f64> {
        static TICKS: std::sync::OnceLock<Option<f64>> = std::sync::OnceLock::new();
        *TICKS.get_or_init(|| {
            unsafe extern "C" { fn sysconf(name: i32) -> i64; }
            const SC_CLK_TCK: i32 = 2;
            // SAFETY: sysconf só lê um valor de configuração; 2 é _SC_CLK_TCK em glibc e musl.
            let ticks = unsafe { sysconf(SC_CLK_TCK) };
            (ticks > 0).then_some(ticks as f64)
        })
    }

    impl ProcessView for ProcFs {
        fn children(&self, max_age: Duration) -> io::Result<Arc<ChildrenMap>> {
            // Trava durante a varredura: N leitores que erram juntos varrem uma vez só.
            let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((at, map)) = cache.as_ref() {
                if at.elapsed() < max_age {
                    return Ok(map.clone());
                }
            }
            let map = Arc::new(self.scan()?);
            *cache = Some((Instant::now(), map.clone()));
            Ok(map)
        }

        fn argv(&self, pid: i64) -> Vec<String> {
            let Ok(raw) = std::fs::read(self.path(pid, "cmdline")) else { return Vec::new() };
            raw.split(|b| *b == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned()).collect()
        }

        fn cwd(&self, pid: i64) -> Option<PathBuf> { std::fs::read_link(self.path(pid, "cwd")).ok() }

        fn env_var(&self, pid: i64, name: &str) -> io::Result<Option<OsString>> {
            use std::os::unix::ffi::OsStringExt;
            let raw = std::fs::read(self.path(pid, "environ"))?;
            let prefix = [name.as_bytes(), b"="].concat();
            Ok(raw.split(|b| *b == 0).find_map(|kv| kv.strip_prefix(prefix.as_slice()))
                .map(|v| OsString::from_vec(v.to_vec())))
        }

        fn start_time(&self, pid: i64) -> Option<f64> {
            let (_, ticks) = self.stat(pid)?;
            Some(self.btime()? + ticks as f64 / clock_ticks()?)
        }

        fn fds(&self, pid: i64) -> Vec<PathBuf> {
            let Ok(entries) = std::fs::read_dir(self.path(pid, "fd")) else { return Vec::new() };
            entries.flatten().filter_map(|e| std::fs::read_link(e.path()).ok()).collect()
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn add(root: &Path, pid: i64, ppid: i64, comm: &str, argv: &[&str]) {
        let dir = root.join(pid.to_string());
        fs::create_dir_all(dir.join("fd")).unwrap();
        // comm com espaço e parêntese: só o último ')' delimita os campos.
        let mut stat = format!("{pid} ({comm}) S {ppid}");
        for _ in 0..17 { stat.push_str(" 0"); }
        stat.push_str(" 500 0 0\n");
        fs::write(dir.join("stat"), stat).unwrap();
        let mut cmd = Vec::new();
        for a in argv { cmd.extend_from_slice(a.as_bytes()); cmd.push(0); }
        fs::write(dir.join("cmdline"), cmd).unwrap();
    }

    fn fake_proc() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("stat"), "cpu 1 2 3\nbtime 1000\n").unwrap();
        add(root.path(), 10, 1, "fish", &["fish"]);
        add(root.path(), 11, 10, "node (x)", &["/usr/bin/node", "claude", "--session-id", "abc"]);
        add(root.path(), 12, 10, "tmux", &["tmux"]);
        root
    }

    #[test]
    fn reads_children_and_argv() {
        let root = fake_proc();
        let dir = root.path().join("11");
        fs::write(dir.join("environ"), b"A=1\0CLAUDE_CONFIG_DIR=/x/\xffy\0").unwrap();
        std::os::unix::fs::symlink("/work/dir", dir.join("cwd")).unwrap();
        std::os::unix::fs::symlink("/p/s.jsonl", dir.join("fd").join("3")).unwrap();
        fs::create_dir(root.path().join("self")).unwrap();
        let procs = ProcFs::with_root(root.path());

        let map = procs.children(CHILDREN_TTL).unwrap();
        let mut kids = map.get(&10).cloned().unwrap_or_default();
        kids.sort();
        assert_eq!(kids, vec![11, 12]);
        assert_eq!(map.get(&1), Some(&vec![10]));
        assert_eq!(procs.argv(11), ["/usr/bin/node", "claude", "--session-id", "abc"]);
        assert_eq!(procs.cwd(11), Some(PathBuf::from("/work/dir")));
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(procs.env_var(11, "CLAUDE_CONFIG_DIR").unwrap().unwrap().as_bytes(), b"/x/\xffy");
        assert_eq!(procs.env_var(11, "CP_ENGINE").unwrap(), None);
        assert!(procs.env_var(12, "CP_ENGINE").is_err(), "sem environ legível é erro, não ausência");
        assert_eq!(procs.fds(11), [PathBuf::from("/p/s.jsonl")]);
        // 500 ticks / 100 Hz depois do boot em 1000.
        assert_eq!(procs.start_time(11), Some(1005.0));
        assert_eq!(procs.argv(99), Vec::<String>::new());
        assert_eq!(procs.start_time(99), None);
    }

    #[test]
    fn fresh_read_sees_new_child() {
        let root = fake_proc();
        let procs = ProcFs::with_root(root.path());
        assert!(procs.children(CHILDREN_TTL).unwrap().get(&11).is_none());
        add(root.path(), 13, 11, "claude", &["claude"]);
        assert!(procs.children(CHILDREN_TTL).unwrap().get(&11).is_none(), "dentro do prazo reusa o mapa");
        assert_eq!(procs.children(Duration::ZERO).unwrap().get(&11), Some(&vec![13]));
    }

    #[test]
    fn unreadable_root_is_error() {
        let procs = ProcFs::with_root("/nonexistent/proc");
        assert!(procs.children(Duration::ZERO).is_err());
        let hidden = tempfile::tempdir().unwrap();
        fs::create_dir(hidden.path().join("10")).unwrap();
        assert!(ProcFs::with_root(hidden.path()).children(Duration::ZERO).is_err(), "nenhum stat legível");
    }

    #[test]
    fn invalid_utf8_comm_keeps_process() {
        let root = fake_proc();
        let dir = root.path().join("20");
        fs::create_dir(&dir).unwrap();
        let mut stat = b"20 (a\xff b) S 10".to_vec();
        stat.extend(" 0".repeat(17).bytes());
        stat.extend(b" 7 0 0\n");
        fs::write(dir.join("stat"), stat).unwrap();
        let procs = ProcFs::with_root(root.path());
        assert!(procs.children(Duration::ZERO).unwrap()[&10].contains(&20));
        assert_eq!(procs.start_time(20), Some(1000.07));
    }
}
