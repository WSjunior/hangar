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
#[cfg(not(target_os = "linux"))]
pub use other::SysInfo;

/// O leitor desta plataforma.
#[cfg(target_os = "linux")]
pub type SystemProcs = ProcFs;
#[cfg(not(target_os = "linux"))]
pub type SystemProcs = SysInfo;

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::sync::Mutex;
    use std::time::Instant;

    pub struct ProcFs {
        root: PathBuf,
        cache: Mutex<Option<(Instant, Arc<ChildrenMap>)>>,
        /// O boot não muda enquanto o processo vive.
        btime: std::sync::OnceLock<f64>,
    }

    impl Default for ProcFs {
        fn default() -> Self { Self::with_root("/proc") }
    }

    impl ProcFs {
        /// Raiz trocável só para o teste apontar para um /proc de mentira.
        pub fn with_root(root: impl Into<PathBuf>) -> Self {
            Self { root: root.into(), cache: Mutex::new(None), btime: std::sync::OnceLock::new() }
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
            if let Some(b) = self.btime.get() {
                return Some(*b);
            }
            // Só o valor lido fica: uma falha guardada deixaria todo nascimento em branco.
            let raw = std::fs::read_to_string(self.root.join("stat")).ok()?;
            let b = raw.lines().find_map(|l| l.strip_prefix("btime ")).and_then(|v| v.trim().parse().ok())?;
            Some(*self.btime.get_or_init(|| b))
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

// Compilado também no teste do Linux: é a única plataforma onde o CI local roda.
#[cfg(any(not(target_os = "linux"), test))]
mod other {
    use super::*;
    use std::sync::Mutex;
    use std::time::Instant;
    use sysinfo::{Pid, Process, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

    struct Snapshot {
        system: System,
        /// Quando a tabela inteira foi relida (com argv e ambiente) e o mapa que saiu dela.
        cache: Option<(Instant, Arc<ChildrenMap>)>,
        /// Pids lidos por inteiro nesse retrato; um relido sozinho só tem o que aquela leitura pediu.
        full: std::collections::HashSet<Pid>,
    }

    /// O ramo `psutil` do `procinfo.py`. No Windows toda releitura, mesmo de um pid, tira o retrato
    /// de todos os processos (`CreateToolhelp32Snapshot`): uma por tique serve argv, ambiente e
    /// nascimento de todos os pids da rodada.
    pub struct SysInfo {
        state: Mutex<Snapshot>,
        #[cfg(test)]
        pub(super) refreshes: std::sync::atomic::AtomicUsize,
    }

    impl Default for SysInfo {
        fn default() -> Self {
            Self {
                state: Mutex::new(Snapshot { system: System::new(), cache: None, full: Default::default() }),
                #[cfg(test)]
                refreshes: Default::default(),
            }
        }
    }

    impl SysInfo {
        fn counted(&self) {
            #[cfg(test)]
            self.refreshes.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        /// Do retrato da rodada quando `from_snapshot` e o pid está nele; senão relê só `pid`, com
        /// só o que `kind` pede. `None` = morto ou pid fora do alcance.
        fn with<T>(&self, pid: i64, from_snapshot: bool, kind: ProcessRefreshKind, read: impl FnOnce(&Process) -> T) -> Option<T> {
            let pid = Pid::from_u32(u32::try_from(pid).ok()?);
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let fresh = state.cache.as_ref().is_some_and(|(at, _)| at.elapsed() < CHILDREN_TTL);
            if !(from_snapshot && fresh && state.full.contains(&pid)) {
                self.counted();
                state.system.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, kind);
            }
            state.system.process(pid).map(read)
        }
    }

    fn as_i64(pid: Pid) -> i64 { i64::from(pid.as_u32()) }

    impl ProcessView for SysInfo {
        fn children(&self, max_age: Duration) -> io::Result<Arc<ChildrenMap>> {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((at, map)) = state.cache.as_ref() {
                if at.elapsed() < max_age {
                    return Ok(map.clone());
                }
            }
            self.counted();
            // `Always`: no macOS o pid sobrevive ao exec, e o argv guardado seria o do shell.
            let kind = ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always).with_environ(UpdateKind::Always);
            state.system.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
            state.full = state.system.processes().keys().copied().collect();
            if state.full.is_empty() {
                state.cache = None;
                return Err(io::Error::other("nenhum processo listado"));
            }
            // Em ordem de pid, como o `psutil.process_iter`; sem pai fica sob 0, como lá.
            let mut pids: Vec<(i64, i64)> = state.system.processes().iter()
                .map(|(pid, p)| (as_i64(*pid), p.parent().map_or(0, as_i64))).collect();
            pids.sort_unstable();
            let mut map = ChildrenMap::new();
            for (pid, ppid) in pids {
                map.entry(ppid).or_default().push(pid);
            }
            let map = Arc::new(map);
            state.cache = Some((Instant::now(), map.clone()));
            Ok(map)
        }

        fn argv(&self, pid: i64) -> Vec<String> {
            self.with(pid, true, ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
                |p| p.cmd().iter().map(|a| a.to_string_lossy().into_owned()).collect()).unwrap_or_default()
        }

        /// Fora do retrato: só a sessão sem terminal com a pasta renomeada pergunta.
        fn cwd(&self, pid: i64) -> Option<PathBuf> {
            self.with(pid, false, ProcessRefreshKind::nothing().with_cwd(UpdateKind::Always),
                |p| p.cwd().map(|c| c.to_path_buf())).flatten()
        }

        fn env_var(&self, pid: i64, name: &str) -> io::Result<Option<OsString>> {
            let prefix = [name.as_bytes(), b"="].concat();
            let found = self.with(pid, true, ProcessRefreshKind::nothing().with_environ(UpdateKind::Always), |p| {
                // Ambiente vazio é o que o `sysinfo` devolve quando não consegue ler (outro dono).
                (!p.environ().is_empty()).then(|| p.environ().iter().find_map(|kv| {
                    let value = kv.as_encoded_bytes().strip_prefix(prefix.as_slice())?;
                    // SAFETY: corte logo após o `=` ASCII, fronteira válida do encoding do OsStr.
                    Some(unsafe { std::ffi::OsStr::from_encoded_bytes_unchecked(value) }.to_os_string())
                }))
            });
            match found {
                None => Err(io::Error::new(io::ErrorKind::NotFound, "processo ausente")),
                Some(None) => Err(io::Error::new(io::ErrorKind::PermissionDenied, "ambiente ilegível")),
                Some(Some(value)) => Ok(value),
            }
        }

        fn start_time(&self, pid: i64) -> Option<f64> {
            // Segundos inteiros: o `psutil` dava fração, o `sysinfo` não tem.
            self.with(pid, true, ProcessRefreshKind::nothing(), |p| p.start_time())
                .filter(|t| *t > 0).map(|t| t as f64)
        }

        /// Fora do Linux o transcript não sai do fd aberto (`procinfo._open_jsonl`): enumerar
        /// handles leva segundos.
        fn fds(&self, _pid: i64) -> Vec<PathBuf> { Vec::new() }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::atomic::Ordering;

        fn sleeper(env: &str) -> std::process::Child {
            let mut c = if cfg!(windows) {
                let mut c = std::process::Command::new("ping");
                c.args(["-n", "30", "127.0.0.1"]);
                c
            } else {
                let mut c = std::process::Command::new("sleep");
                c.arg("30");
                c
            };
            c.env("HANGAR_PROCS_MARK", env).stdout(std::process::Stdio::null()).spawn().unwrap()
        }

        /// Depois do exec o argv é o do programa; antes dele, o do pai.
        fn wait_exec(pid: i64) {
            let deadline = Instant::now() + Duration::from_secs(5);
            let probe = SysInfo::default();
            while !probe.argv(pid).last().is_some_and(|a| a == "30" || a == "127.0.0.1") && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
        }

        #[test]
        fn one_snapshot_serves_the_round() {
            let mut a = sleeper("a");
            let pid_a = i64::from(a.id());
            wait_exec(pid_a);
            let procs = SysInfo::default();
            assert!(procs.children(Duration::ZERO).unwrap().get(&i64::from(std::process::id())).is_some_and(|k| k.contains(&pid_a)));
            for _ in 0..3 {
                assert!(procs.argv(pid_a).last().is_some_and(|x| x == "30" || x == "127.0.0.1"));
                assert_eq!(procs.env_var(pid_a, "HANGAR_PROCS_MARK").unwrap(), Some(OsString::from("a")));
                assert!(procs.start_time(pid_a).is_some());
            }
            assert_eq!(procs.refreshes.load(Ordering::Relaxed), 1, "argv, ambiente e nascimento saem do retrato");
            // Nascido depois do retrato: relido sozinho, sem esperar o próximo.
            let mut b = sleeper("b");
            let pid_b = i64::from(b.id());
            wait_exec(pid_b);
            assert!(!procs.argv(pid_b).is_empty());
            // Relido só com o argv: o ambiente dele ainda não foi lido e não pode sair vazio.
            assert_eq!(procs.env_var(pid_b, "HANGAR_PROCS_MARK").unwrap(), Some(OsString::from("b")));
            assert_eq!(procs.refreshes.load(Ordering::Relaxed), 3);
            for c in [&mut a, &mut b] {
                c.kill().unwrap();
                c.wait().unwrap();
            }
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
        fs::write(root.path().join("stat"), "btime 2000\n").unwrap();
        assert_eq!(procs.start_time(11), Some(1005.0), "o boot é lido uma vez");
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

#[cfg(test)]
mod platform_tests {
    use super::*;
    use std::time::Instant;

    /// Filho real lido pelo leitor da plataforma (`/proc` no Linux, `sysinfo` nos outros).
    #[test]
    fn sysinfo_reads_argv_and_env() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = if cfg!(windows) {
            let mut c = std::process::Command::new("ping");
            c.args(["-n", "30", "127.0.0.1"]);
            c
        } else {
            let mut c = std::process::Command::new("sleep");
            c.arg("30");
            c
        };
        let mut child = command.current_dir(dir.path()).env("PSMUX_SESSION", "sim")
            .stdout(std::process::Stdio::null()).spawn().unwrap();
        let pid = i64::from(child.id());
        let procs = SystemProcs::default();
        // Logo após o spawn o processo ainda pode ter o argv do pai (antes do exec).
        let deadline = Instant::now() + Duration::from_secs(5);
        let argv = loop {
            let argv = procs.argv(pid);
            if argv.last().is_some_and(|a| a == "30" || a == "127.0.0.1") || Instant::now() > deadline { break argv; }
            std::thread::sleep(Duration::from_millis(50));
        };
        let env = procs.env_var(pid, "PSMUX_SESSION");
        let absent = procs.env_var(pid, "HANGAR_PROCS_ABSENT");
        let cwd = procs.cwd(pid);
        let start = procs.start_time(pid);
        let kids = procs.children(Duration::ZERO).unwrap().get(&i64::from(std::process::id())).cloned();
        child.kill().unwrap();
        child.wait().unwrap();

        assert!(argv.last().is_some_and(|a| a == "30" || a == "127.0.0.1"), "argv: {argv:?}");
        assert_eq!(env.unwrap(), Some(OsString::from("sim")));
        assert_eq!(absent.unwrap(), None);
        let same = |p: &std::path::Path| std::fs::canonicalize(p).unwrap();
        assert_eq!(cwd.as_deref().map(same), Some(same(dir.path())));
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
        assert!(start.is_some_and(|s| (now - s).abs() < 120.0), "start: {start:?}, now: {now}");
        assert!(kids.is_some_and(|k| k.contains(&pid)), "o filho aparece no mapa sob o pai");
        let gone = SystemProcs::default();
        assert!(gone.argv(pid).is_empty() && gone.cwd(pid).is_none(), "processo morto responde vazio");
        assert!(gone.env_var(pid, "PSMUX_SESSION").is_err(), "morto é ilegível, não ausência");
    }
}
