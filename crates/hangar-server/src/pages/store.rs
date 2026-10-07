//! Pasta por sessão em `~/.hangar/paginas/<chave>/`: `<id>.html`, `<id>.json`, prints e o `jsonl` do dono.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::Theme;

const GONE_AFTER: Duration = Duration::from_secs(60);
const DRAFT_TTL: Duration = Duration::from_secs(3600);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PageMeta { pub title: String, pub height: Option<u32>, pub heights: BTreeMap<u32, u32>, pub created: u64, pub draft: bool }

pub struct NewPage { pub html: String, pub title: String, pub height: Option<u32>, pub heights: BTreeMap<u32, u32>, pub draft: bool }

pub struct Store { root: PathBuf, absent_since: Mutex<HashMap<String, SystemTime>> }

fn valid(part: &str) -> bool { !part.is_empty() && part.len() <= 128 && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') }

fn new_id() -> String {
    use ring::rand::SecureRandom;
    let mut b = [0u8; 16];
    ring::rand::SystemRandom::new().fill(&mut b).expect("gerador do sistema");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

impl Store {
    pub fn new(root: PathBuf) -> Store { Store { root, absent_since: Mutex::new(HashMap::new()) } }

    pub fn default_root() -> PathBuf {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
        home.join(".hangar").join("paginas")
    }

    fn dir(&self, key: &str) -> Option<PathBuf> { valid(key).then(|| self.root.join(key)) }

    pub fn save(&self, key: &str, jsonl: &str, page: &NewPage) -> io::Result<String> {
        let dir = self.dir(key).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "chave inválida"))?;
        std::fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?; }
        std::fs::write(dir.join("jsonl"), jsonl)?;
        let id = new_id();
        let created = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let meta = PageMeta { title: page.title.clone(), height: page.height, heights: page.heights.clone(), created, draft: page.draft };
        write_atomic(&dir.join(format!("{id}.html")), page.html.as_bytes())?;
        write_atomic(&dir.join(format!("{id}.json")), &serde_json::to_vec(&meta)?)?;
        Ok(id)
    }

    pub fn html(&self, key: &str, id: &str) -> Option<String> {
        if !valid(id) { return None; }
        std::fs::read_to_string(self.dir(key)?.join(format!("{id}.html"))).ok()
    }

    pub fn meta(&self, key: &str, id: &str) -> Option<PageMeta> {
        if !valid(id) { return None; }
        serde_json::from_slice(&std::fs::read(self.dir(key)?.join(format!("{id}.json"))).ok()?).ok()
    }

    pub fn set_heights(&self, key: &str, id: &str, heights: BTreeMap<u32, u32>) -> io::Result<()> {
        let mut meta = self.meta(key, id).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        meta.heights = heights;
        write_atomic(&self.root.join(key).join(format!("{id}.json")), &serde_json::to_vec(&meta)?)
    }

    pub fn shot_path(&self, key: &str, id: &str, theme: Theme, width: u32) -> PathBuf {
        self.root.join(key).join(format!("{id}.{}.{width}.png", theme.as_str()))
    }

    /// Apaga a pasta cujo transcript não está vivo há `GONE_AFTER`, e rascunhos com mais de `DRAFT_TTL`.
    /// Os dois lados são comparados canonicalizados: `~/.claude-<conta>` é link do `~/.claude`.
    pub fn sweep(&self, live_jsonl: &HashSet<String>, now: SystemTime) {
        let Ok(entries) = std::fs::read_dir(&self.root) else { return };
        let live: HashSet<String> = live_jsonl.iter().map(|p| canonical(p)).collect();
        let mut absent = self.absent_since.lock().unwrap_or_else(|e| e.into_inner());
        let mut seen = HashSet::new();
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_dir()) { continue; }
            let key = entry.file_name().to_string_lossy().into_owned();
            let dir = entry.path();
            let owner = std::fs::read_to_string(dir.join("jsonl")).unwrap_or_default();
            seen.insert(key.clone());
            if live.contains(&canonical(owner.trim())) { absent.remove(&key); self.drop_old_drafts(&dir, now); continue; }
            let since = *absent.entry(key.clone()).or_insert(now);
            if now.duration_since(since).unwrap_or_default() >= GONE_AFTER {
                if let Err(e) = std::fs::remove_dir_all(&dir) { tracing::warn!(key = %key, "páginas da sessão não apagadas: {e}"); }
                absent.remove(&key);
            }
        }
        absent.retain(|k, _| seen.contains(k));
    }

    /// Rodada pulada (lista incerta) zera a contagem: a ausência só vale medida em rodadas certas seguidas.
    pub fn forget_absences(&self) { self.absent_since.lock().unwrap_or_else(|e| e.into_inner()).clear(); }

    fn drop_old_drafts(&self, dir: &std::path::Path, now: SystemTime) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") { continue; }
            let Some(meta) = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice::<PageMeta>(&b).ok()) else { continue };
            let age = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs().saturating_sub(meta.created);
            if meta.draft && age >= DRAFT_TTL.as_secs() {
                let id = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_owned();
                for f in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                    if f.file_name().to_string_lossy().starts_with(&format!("{id}.")) { let _ = std::fs::remove_file(f.path()); }
                }
            }
        }
    }
}

/// Transcript que ainda não existe resolve pela pasta; nada resolvível fica com o texto cru.
fn canonical(path: &str) -> String {
    let p = std::path::Path::new(path);
    std::fs::canonicalize(p).ok()
        .or_else(|| Some(std::fs::canonicalize(p.parent()?).ok()?.join(p.file_name()?)))
        .map_or_else(|| path.to_owned(), |c| c.to_string_lossy().into_owned())
}

pub(crate) fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> NewPage { NewPage { html: "<p>x</p>".into(), title: "t".into(), height: None, heights: BTreeMap::new(), draft: false } }

    #[test]
    fn save_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().into());
        let id = s.save("abc-1", "/t/abc-1.jsonl", &page()).unwrap();
        assert_eq!(s.html("abc-1", &id).unwrap(), "<p>x</p>");
        assert_eq!(s.meta("abc-1", &id).unwrap().title, "t");
    }

    #[test]
    fn rejects_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().into());
        assert!(s.save("../x", "/t", &page()).is_err());
        assert!(s.html("abc", "../../etc/passwd").is_none());
    }

    #[test]
    fn sweep_waits_before_deleting() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().into());
        s.save("k", "/t/k.jsonl", &page()).unwrap();
        let t0 = SystemTime::now();
        s.sweep(&HashSet::new(), t0);
        assert!(dir.path().join("k").exists());
        s.sweep(&HashSet::new(), t0 + GONE_AFTER + Duration::from_secs(1));
        assert!(!dir.path().join("k").exists());
    }

    #[test]
    fn skipped_round_restarts_the_wait() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().into());
        s.save("k", "/t/k.jsonl", &page()).unwrap();
        let t0 = SystemTime::now();
        s.sweep(&HashSet::new(), t0);
        s.forget_absences();
        s.sweep(&HashSet::new(), t0 + GONE_AFTER + Duration::from_secs(1));
        assert!(dir.path().join("k").exists(), "primeira rodada certa depois da pausa só começa a contar");
    }

    #[test]
    fn sweep_ignores_loose_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("solto"), "").unwrap();
        let s = Store::new(dir.path().into());
        let t0 = SystemTime::now();
        s.sweep(&HashSet::new(), t0);
        s.sweep(&HashSet::new(), t0 + GONE_AFTER * 2);
        assert!(dir.path().join("solto").exists());
    }

    #[test]
    fn sweep_keeps_live_session() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().into());
        s.save("k", "/t/k.jsonl", &page()).unwrap();
        let live: HashSet<String> = ["/t/k.jsonl".to_owned()].into();
        let t0 = SystemTime::now();
        s.sweep(&live, t0);
        s.sweep(&live, t0 + GONE_AFTER * 2);
        assert!(dir.path().join("k").exists());
    }

    #[cfg(unix)]
    #[test]
    fn sweep_matches_owner_through_symlinked_account_dir() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("claude");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("k.jsonl"), "").unwrap();
        std::os::unix::fs::symlink(&real, dir.path().join("claude-conta")).unwrap();
        let s = Store::new(dir.path().join("paginas"));
        s.save("k", dir.path().join("claude-conta/k.jsonl").to_str().unwrap(), &page()).unwrap();
        let live: HashSet<String> = [real.join("k.jsonl").to_string_lossy().into_owned()].into();
        let t0 = SystemTime::now();
        s.sweep(&live, t0);
        s.sweep(&live, t0 + GONE_AFTER * 2);
        assert!(dir.path().join("paginas/k").exists());
    }
}
