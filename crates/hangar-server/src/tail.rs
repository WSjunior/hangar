// crates/hangar-server/src/tail.rs
//! Leitura do transcript para o chat ao vivo: um observador por pasta, um leitor por jsonl e o
//! mesmo quadro SSE já serializado para todos os aparelhos daquele chat.
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use bytes::Bytes;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::{Notify, broadcast};

use crate::side::Out;
use crate::transcript::{LineParser, Provider, SKIPPED_LINES};

/// Linhas da cauda de quem conecta sem posição.
pub const BACKFILL_LINES: usize = 200;
const TAIL_WINDOW: u64 = 256 * 1024;
/// Releitura sem aviso do sistema de arquivos: cobre aviso perdido e a fresta entre ler e armar
/// o observador, como o relógio do awatch no Python.
const HEARTBEAT: Duration = Duration::from_secs(5);
/// Pasta que ainda não existe (sessão nascendo): nova tentativa a cada segundo, aviso uma vez.
const DIR_RETRY: Duration = Duration::from_secs(1);
const DIR_WARN_AFTER: u32 = 30;

/// Quadro no formato do sse_starlette: id, event, data, linha em branco, tudo com `\r\n`.
pub fn sse_frame(event: &str, data: &str, id: Option<&str>) -> Bytes {
    let mut s = String::with_capacity(data.len() + event.len() + 48);
    if let Some(id) = id {
        s.push_str("id: ");
        s.push_str(id);
        s.push_str("\r\n");
    }
    s.push_str("event: ");
    s.push_str(event);
    s.push_str("\r\n");
    // Quebra de linha no dado vira várias linhas `data:`, como no sse_starlette.
    for line in data.replace("\r\n", "\n").split(['\r', '\n']) {
        s.push_str("data: ");
        s.push_str(line);
        s.push_str("\r\n");
    }
    s.push_str("\r\n");
    Bytes::from(s)
}

pub fn ping_frame() -> Bytes {
    sse_frame("ping", "{}", None)
}

pub fn reset_frame() -> Bytes {
    sse_frame("reset", "{}", None)
}

/// Keep-alive em comentário, que o EventSource ignora.
pub fn comment_frame() -> Bytes {
    Bytes::from(format!(": ping - {}\r\n\r\n", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S%.6f+00:00")))
}

/// Linhas completas de `from` até `to` (ou o fim) em quadros `message` com `id: <key>:<início da
/// linha>`. Linha sem `\n` fica para a próxima leitura; arquivo ausente não é erro.
pub fn read_frames(
    path: &Path,
    from: u64,
    to: Option<u64>,
    parser: &mut LineParser,
    key: &str,
) -> std::io::Result<(Vec<Bytes>, u64)> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), from)),
        Err(e) => return Err(e),
    };
    let mut reader = BufReader::new(file);
    reader.seek(SeekFrom::Start(from))?;
    let mut frames = Vec::new();
    let mut at = from;
    let mut line = Vec::new();
    let skipped_before = SKIPPED_LINES.load(Ordering::Relaxed);
    while to.is_none_or(|t| at < t) {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 || line.last() != Some(&b'\n') {
            break;
        }
        let id = format!("{key}:{at}");
        for ev in parser.feed(&line, at) {
            let data = serde_json::to_string(&ev).map_err(std::io::Error::other)?;
            frames.push(sse_frame("message", &data, Some(&id)));
        }
        at += n as u64;
    }
    log_skipped(key, skipped_before);
    Ok((frames, at))
}

/// Registra as linhas que o parser pulou desde `before`: só a conta e a chave, nunca o texto.
// ponytail: o contador é global; leitura simultânea de outra sessão pode entrar na conta. Um
// contador por leitura, se o log precisar apontar o arquivo exato.
pub fn log_skipped(key: &str, before: u64) {
    let skipped = SKIPPED_LINES.load(Ordering::Relaxed).saturating_sub(before);
    if skipped > 0 {
        tracing::warn!(key, skipped, "linhas ilegíveis do transcript puladas");
    }
}

fn read_window(f: &mut File, size: u64, window: u64) -> Option<(u64, Vec<u8>)> {
    let start = size.saturating_sub(window);
    let mut buf = Vec::new();
    f.seek(SeekFrom::Start(start)).ok()?;
    f.take(size - start).read_to_end(&mut buf).ok()?;
    Some((start, buf))
}

/// Fim da última linha completa: onde o leitor compartilhado começa.
fn complete_end(path: &Path) -> u64 {
    let Ok(mut f) = File::open(path) else { return 0 };
    let Ok(size) = f.seek(SeekFrom::End(0)) else { return 0 };
    let mut window = TAIL_WINDOW;
    loop {
        let Some((start, buf)) = read_window(&mut f, size, window) else { return 0 };
        if let Some(i) = buf.iter().rposition(|&b| b == b'\n') {
            return start + i as u64 + 1;
        }
        if start == 0 {
            return 0;
        }
        window *= 4;
    }
}

/// Início da `max_lines`-ésima linha contada do fim; poucas linhas, arquivo vazio ou ausente → 0.
/// Só conta `\n`: cauda sem `\n` (gravação em curso) não entra.
pub fn tail_offset(path: &Path, max_lines: usize) -> u64 {
    let Ok(mut f) = File::open(path) else { return 0 };
    let Ok(size) = f.seek(SeekFrom::End(0)) else { return 0 };
    let mut window = TAIL_WINDOW;
    loop {
        let Some((start, buf)) = read_window(&mut f, size, window) else { return 0 };
        if buf.iter().filter(|&&b| b == b'\n').count() > max_lines {
            let mut idx = buf.len();
            for _ in 0..=max_lines {
                idx = buf[..idx].iter().rposition(|&b| b == b'\n').expect("contado acima");
            }
            return start + idx as u64 + 1;
        }
        if start == 0 {
            return 0;
        }
        // Janela curta, ou uma linha gigante (imagem em base64): cresce.
        window *= 4;
    }
}

/// Cauda de UM aparelho, de onde ele pediu até `cut`, com parser próprio (cada leitor tem o seu).
/// `resume` só vale com o stem desta sessão e dentro do arquivo; senão, as últimas 200 linhas.
pub fn backfill(path: &Path, key: &str, provider: Provider, resume: Option<&str>, cut: u64) -> Vec<Bytes> {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let start = resume
        .and_then(|raw| {
            let (stem, off) = raw.rsplit_once(':')?;
            if stem.is_empty() || stem != key {
                return None;
            }
            off.trim().parse::<u64>().ok().filter(|o| *o <= size)
        })
        .unwrap_or_else(|| tail_offset(path, BACKFILL_LINES));
    if start >= cut {
        return Vec::new();
    }
    let mut parser = LineParser::new(provider);
    match read_frames(path, start, Some(cut), &mut parser, key) {
        Ok((frames, _)) => frames,
        Err(e) => {
            tracing::warn!(key, "cauda do transcript falhou: {e}");
            Vec::new()
        }
    }
}

/// Um observador por pasta, compartilhado por todos os leitores de arquivos dela.
#[derive(Clone, Default)]
pub struct Watchers(Arc<Mutex<HashMap<PathBuf, DirWatch>>>);

struct DirWatch {
    _watcher: RecommendedWatcher,
    subs: Vec<(OsString, Weak<Notify>)>,
}

impl Watchers {
    /// Inscreve o arquivo no observador da pasta dele. false = pasta inexistente ou observador
    /// que não armou; quem chama relê no relógio e tenta de novo.
    pub fn subscribe(&self, file: &Path, wake: &Arc<Notify>) -> bool {
        let (Some(dir), Some(name)) = (file.parent(), file.file_name()) else { return false };
        let mut map = self.0.lock().unwrap();
        if !map.contains_key(dir) {
            let shared = Arc::downgrade(&self.0);
            let key = dir.to_path_buf();
            let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                let (Ok(ev), Some(shared)) = (res, shared.upgrade()) else { return };
                let map = shared.lock().unwrap();
                let Some(d) = map.get(&key) else { return };
                // Escrita de arquivo irmão (subagente) não acorda este leitor.
                for p in &ev.paths {
                    let Some(n) = p.file_name() else { continue };
                    for (sub, w) in &d.subs {
                        if sub == n {
                            if let Some(w) = w.upgrade() {
                                w.notify_one();
                            }
                        }
                    }
                }
            });
            let Ok(mut watcher) = watcher else { return false };
            if watcher.watch(dir, RecursiveMode::NonRecursive).is_err() {
                return false;
            }
            map.insert(dir.to_path_buf(), DirWatch { _watcher: watcher, subs: Vec::new() });
        }
        let d = map.get_mut(dir).expect("inserido acima");
        d.subs.retain(|(_, w)| w.strong_count() > 0);
        d.subs.push((name.to_os_string(), Arc::downgrade(wake)));
        true
    }

    pub fn unsubscribe(&self, file: &Path, wake: &Arc<Notify>) {
        let Some(dir) = file.parent() else { return };
        let removed = {
            let mut map = self.0.lock().unwrap();
            let Some(d) = map.get_mut(dir) else { return };
            d.subs.retain(|(_, w)| w.strong_count() > 0 && !std::ptr::eq(w.as_ptr(), Arc::as_ptr(wake)));
            if d.subs.is_empty() { map.remove(dir) } else { None }
        };
        // Solta o observador fora da trava: o callback dele também a pega.
        drop(removed);
    }
}

pub struct TailState {
    path: PathBuf,
    key: String,
    provider: Provider,
    generation: u64,
    pos: Option<u64>,
    parser: LineParser,
}

impl TailState {
    /// Ponto até onde o leitor compartilhado já leu; na primeira consulta, o fim das linhas
    /// completas (a cauda de cada aparelho cobre o que vem antes).
    pub fn cut(&mut self) -> u64 {
        *self.pos.get_or_insert_with(|| complete_end(&self.path))
    }

    /// Lê o que chegou e manda a todos, sob a trava. Arquivo menor que a posição = truncado:
    /// `reset` e releitura do início com parser novo.
    fn poll(&mut self, tx: &broadcast::Sender<Out>) {
        let Some(pos) = self.pos else {
            self.cut();
            return;
        };
        let Ok(meta) = std::fs::metadata(&self.path) else { return };
        if meta.len() < pos {
            tracing::info!(key = %self.key, "transcript encolheu; recomeça do início");
            self.parser = LineParser::new(self.provider);
            self.pos = Some(0);
            let _ = tx.send(Out::Tail(self.generation, reset_frame()));
        }
        let from = self.pos.unwrap_or(0);
        match read_frames(&self.path, from, None, &mut self.parser, &self.key) {
            Ok((frames, end)) => {
                self.pos = Some(end);
                for f in frames {
                    let _ = tx.send(Out::Tail(self.generation, f));
                }
            }
            Err(e) => tracing::warn!(key = %self.key, "leitura do transcript falhou: {e}"),
        }
    }
}

/// Leitor compartilhado de um jsonl. Some ao ser solto (troca de transcript ou último aparelho).
pub struct FileTail {
    pub state: Arc<tokio::sync::Mutex<TailState>>,
    task: tokio::task::AbortHandle,
}

impl Drop for FileTail {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FileTail {
    pub fn spawn(
        path: PathBuf,
        key: String,
        provider: Provider,
        generation: u64,
        tx: broadcast::Sender<Out>,
        watchers: Watchers,
    ) -> Arc<FileTail> {
        let state = Arc::new(tokio::sync::Mutex::new(TailState {
            path: path.clone(),
            key,
            provider,
            generation,
            pos: None,
            parser: LineParser::new(provider),
        }));
        let task = tokio::spawn(run(path, state.clone(), tx, watchers)).abort_handle();
        Arc::new(FileTail { state, task })
    }
}

/// Inscrição que sai junto com a tarefa: o abort derruba o future e roda o Drop.
struct Subscription {
    watchers: Watchers,
    path: PathBuf,
    wake: Arc<Notify>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.watchers.unsubscribe(&self.path, &self.wake);
    }
}

async fn run(path: PathBuf, state: Arc<tokio::sync::Mutex<TailState>>, tx: broadcast::Sender<Out>, watchers: Watchers) {
    let sub = Subscription { watchers, path, wake: Arc::new(Notify::new()) };
    let mut watching = false;
    let mut misses = 0u32;
    loop {
        if !watching {
            watching = sub.watchers.subscribe(&sub.path, &sub.wake);
            if !watching {
                misses += 1;
                if misses == DIR_WARN_AFTER {
                    tracing::warn!(path = %sub.path.display(), "pasta do transcript ausente ou sem observador; segue relendo a cada segundo");
                }
            }
        }
        let st = state.clone();
        let tx2 = tx.clone();
        if let Err(e) = tokio::task::spawn_blocking(move || st.blocking_lock().poll(&tx2)).await {
            if e.is_panic() {
                tracing::error!(path = %sub.path.display(), "leitor do transcript caiu");
            }
            return;
        }
        let wait = if watching { HEARTBEAT } else { DIR_RETRY };
        tokio::select! {
            _ = sub.wake.notified() => {}
            _ = tokio::time::sleep(wait) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(path: &Path, lines: std::ops::Range<usize>, tail: &str) -> Vec<u64> {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        let mut at = f.metadata().unwrap().len();
        let mut offs = Vec::new();
        for i in lines {
            let l = format!(
                "{{\"type\":\"user\",\"uuid\":\"u{i}\",\"timestamp\":\"2026-10-02T10:{m:02}:{s:02}.000Z\",\"message\":{{\"role\":\"user\",\"content\":\"linha {i}\"}}}}\n",
                m = i / 60 % 60,
                s = i % 60
            );
            f.write_all(l.as_bytes()).unwrap();
            offs.push(at);
            at += l.len() as u64;
        }
        f.write_all(tail.as_bytes()).unwrap();
        offs
    }

    fn first_line(f: &Bytes) -> String {
        String::from_utf8_lossy(f).split("\r\n").next().unwrap().to_owned()
    }

    #[test]
    fn frame_matches_sse_starlette() {
        assert_eq!(
            &sse_frame("message", "{\"a\":1}", Some("k:5"))[..],
            b"id: k:5\r\nevent: message\r\ndata: {\"a\":1}\r\n\r\n"
        );
        assert_eq!(&sse_frame("x", "a\r\nb\nc", None)[..], b"event: x\r\ndata: a\r\ndata: b\r\ndata: c\r\n\r\n");
        let c = comment_frame();
        assert!(c.starts_with(b": ping - ") && c.ends_with(b"+00:00\r\n\r\n"));
    }

    #[test]
    fn tail_offset_counts_complete_lines_from_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let offs = write(&p, 0..5, "{\"parcial\":");
        assert_eq!(tail_offset(&p, 2), offs[3]);
        assert_eq!(tail_offset(&p, 5), 0);
        assert_eq!(tail_offset(&dir.path().join("nao-existe.jsonl"), 2), 0);
    }

    #[test]
    fn read_frames_stops_before_partial_line_and_ids_line_start() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let offs = write(&p, 0..3, "{\"parcial\":");
        let mut parser = LineParser::new(Provider::Claude);
        let (frames, end) = read_frames(&p, 0, None, &mut parser, "k").unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(first_line(&frames[1]), format!("id: k:{}", offs[1]));
        assert_eq!(end, complete_end(&p));
        assert!(end < std::fs::metadata(&p).unwrap().len());
    }

    #[test]
    fn backfill_honours_only_own_stem_inside_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let offs = write(&p, 0..250, "");
        let cut = complete_end(&p);
        let own = backfill(&p, "k", Provider::Claude, Some(&format!("k:{}", offs[245])), cut);
        assert_eq!(own.len(), 5);
        assert_eq!(first_line(&own[0]), format!("id: k:{}", offs[245]));
        let foreign = backfill(&p, "k", Provider::Claude, Some(&format!("outro:{}", offs[245])), cut);
        assert_eq!(foreign.len(), 200);
        assert_eq!(first_line(&foreign[0]), format!("id: k:{}", offs[50]));
        assert_eq!(backfill(&p, "k", Provider::Claude, Some("k:999999999"), cut).len(), 200);
        assert_eq!(backfill(&p, "k", Provider::Claude, None, offs[10]).len(), 0, "cauda começa depois do corte");
    }
}
