//! `Monitor` de estado de uma sessão Claude com terminal: porte do `StateMonitor` (`state.py`).
//! Uma rodada a cada `POLL`, ou antes quando o plugin vivo acorda (empurrão dos fatos). A memória
//! temporal do `reduce` anda por rodada, contadas como o Python conta, para as sequências gravadas
//! por ele valerem como prova. Ninguém cria um `Monitor` em produção antes da troca de dono do
//! estado: até lá só os testes o exercitam.
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hangar_api::preview::PreviewEvent;
use hangar_api::state::{ShellVivo, StateEvent};
use serde_json::{Value, json};
use tokio::sync::Notify;

use super::facts::{Dead, Received, UNAVAILABLE};
use super::permission;
use super::preview::{self, HookFile};
use crate::terminal_control::{CaptureRequest, TerminalPool};
use crate::terminal_state::{self, PaneAnalysis, ReducerFacts, ReducerMemory, TerminalQuestion};

pub const POLL: Duration = Duration::from_millis(750);
/// Rodadas sem spinner depois das quais o marcador `working` deixa de valer (`HOOK_WORKING_GRACE`).
pub const HOOK_GRACE: u32 = 8;
pub const OBSERVATION_FAILED: &str = "terminal_observacao_falhou";
pub const PERMISSION_FAILED: &str = "permission_observe_failed";

/// Quadro capturado e a análise que o pool já fez dele.
#[derive(Clone, Debug)]
pub struct Frame { pub text: String, pub analysis: PaneAnalysis }

/// Captura que falhou. `attempt` é a tentativa guardada do pool (`None`: falha que ele não guarda):
/// enquanto não muda, o pool só repete a mesma falha sem tentar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureFailed { pub code: String, pub attempt: Option<u32> }

/// Fatos do Python valendo agora.
#[derive(Clone, Debug, Default)]
pub struct RoundFacts {
    pub alive: bool,
    pub plugin_state: Option<String>,
    /// A pergunta segurada no formato de `pergunta_pendente` (`id`, `questions`, `tool`, `resumo`).
    pub question: Option<Value>,
    pub in_transfer: bool,
    pub permission_op: bool,
    /// O retrato não veio: código, e o último valor fica.
    pub unavailable: Option<String>,
    /// Largura da conversa com painel ancorado e começo da faixa dos mods: cortes da prévia.
    pub body_columns: Option<u32>,
    pub band_anchor: Option<String>,
}

impl RoundFacts {
    pub fn from_received(r: Option<&Received>, now: Instant, unavailable: Option<String>) -> Self {
        let Some(r) = r else { return Self { unavailable, ..Self::default() } };
        Self {
            alive: r.alive(now),
            plugin_state: r.plugin_state(now).map(|p| p.state.clone()),
            question: r.question(now).map(|q| json!({"id": q.id, "questions": q.questions, "tool": q.tool, "resumo": q.resumo})),
            in_transfer: r.in_transfer(now),
            permission_op: r.facts.permission_op,
            unavailable,
            body_columns: r.facts.body_columns,
            band_anchor: r.facts.band_anchor.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoopInfo { pub status: Option<String>, pub iter: Option<u32>, pub max: Option<u32> }

/// Arquivos da sessão lidos na rodada; sem session-id, só o loop.
#[derive(Clone, Debug, Default)]
pub struct FileFacts {
    pub marker: Option<String>,
    /// `ts` do marcador, no relógio de parede.
    pub marker_ts: Option<f64>,
    pub open_question: Option<TerminalQuestion>,
    pub status_line: Option<String>,
    pub loop_info: Option<LoopInfo>,
    pub shells: Vec<ShellVivo>,
}

/// O que o `Monitor` lê e a quem publica. Quem implementa faz a E/S bloqueante fora do runtime.
pub trait Sources: Send + Sync {
    fn name(&self) -> &str;
    fn sid(&self) -> Option<String>;
    /// Muda no `rebind` do hub (`/clear`, troca do filho): a rodada em curso não vale mais.
    fn epoch(&self) -> u64;
    /// Avisado quando o Python empurra fatos novos desta sessão.
    fn wake(&self) -> Arc<Notify>;
    fn facts(&self) -> impl Future<Output = RoundFacts> + Send;
    fn capture(&self) -> impl Future<Output = Result<Frame, CaptureFailed>> + Send;
    /// `None` = o multiplexador não respondeu, que não é sessão morta.
    fn has_session(&self) -> impl Future<Output = Option<bool>> + Send;
    fn dead(&self) -> impl Future<Output = Result<Dead, String>> + Send;
    fn observe_permission(&self, key: &str, mode: &str) -> impl Future<Output = Result<(String, String), String>> + Send;
    fn files(&self, sid: Option<&str>) -> impl Future<Output = FileFacts> + Send;
    /// `false`: ninguém mais ouve, e o `Monitor` acaba.
    fn publish(&self, event: StateEvent) -> impl Future<Output = bool> + Send;
    /// Captura só para a prévia, entre as rodadas de estado; `None`: a fonte não as faz.
    fn preview_capture(&self) -> impl Future<Output = Option<Result<Frame, CaptureFailed>>> + Send { async { None } }
    /// `.hangar-preview/<stem>.json` legíveis, na ordem das pastas de config. Chamado a cada toque
    /// rápido: quem implementa lê o disco fora do runtime (`HookFiles` em `spawn_blocking`).
    fn preview_files(&self, _stem: &str) -> impl Future<Output = Vec<HookFile>> + Send { async { Vec::new() } }
    /// Última resposta já gravada no transcript, normalizada (`preview::norm`).
    fn committed(&self) -> Option<Arc<str>> { None }
    fn publish_preview(&self, _event: PreviewEvent) -> impl Future<Output = bool> + Send { async { true } }
    /// Relógio de parede em segundos, o do `ts` do arquivo do hook e do marcador.
    fn wall(&self) -> f64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Exit { Dead, Closed }

enum Step { Exit(Exit), Again, Sleep, Wait { alive: bool } }

#[derive(Default)]
struct Memory {
    reducer: ReducerMemory,
    /// Último evento publicado, com os shells reduzidos aos pids (o tempo deles corre sozinho).
    key: Option<StateEvent>,
    last: Option<StateEvent>,
    permission: permission::Watch,
    /// Em falha da observação: a tentativa do pool já conferida com `has-session`.
    failure: Option<Option<u32>>,
    preview: preview::Slot,
}

pub struct Monitor<S> { src: S, poll: Duration, mem: Memory, epoch: u64 }

fn key_of(event: &StateEvent) -> StateEvent {
    let shells = event.shells.iter().map(|s| ShellVivo { pid: s.pid, ..ShellVivo::default() }).collect();
    StateEvent { shells, ..event.clone() }
}

fn anchor(state: Option<&str>) -> Option<&str> { state.filter(|s| matches!(*s, "working" | "idle")) }

impl<S: Sources> Monitor<S> {
    pub fn new(src: S) -> Self { Self::with_poll(src, POLL) }

    pub fn with_poll(src: S, poll: Duration) -> Self {
        let epoch = src.epoch();
        Self { src, poll, mem: Memory::default(), epoch }
    }

    pub async fn run(mut self) -> Exit {
        let wake = self.src.wake();
        loop {
            // Armado antes da rodada: empurrão que chega durante ela acorda a espera seguinte.
            let notified = wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let exit = match self.round().await {
                Step::Exit(exit) => Some(exit),
                Step::Again => None,
                Step::Sleep | Step::Wait { alive: false } => self.idle(None).await,
                Step::Wait { alive: true } => self.idle(Some(notified.as_mut())).await,
            };
            if let Some(exit) = exit {
                return exit;
            }
        }
    }

    /// Espera a próxima rodada de estado (o relógio, ou o empurrão quando `woken` vem). Enquanto
    /// a prévia corre, toques de `preview::FAST` só para ela, fora da contagem das rodadas.
    async fn idle(&mut self, mut woken: Option<std::pin::Pin<&mut tokio::sync::futures::Notified<'_>>>) -> Option<Exit> {
        let deadline = tokio::time::Instant::now() + self.poll;
        loop {
            let until = if self.mem.preview.fast { (tokio::time::Instant::now() + preview::FAST).min(deadline) } else { deadline };
            let wake = async {
                match woken.as_mut() {
                    Some(n) => n.as_mut().await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                () = tokio::time::sleep_until(until) => {}
                () = wake => return None,
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            if !preview::tick(&self.src, &mut self.mem.preview, None, self.epoch).await {
                return Some(Exit::Closed);
            }
        }
    }

    async fn publish(&mut self, event: StateEvent) -> Option<Step> {
        self.mem.last = Some(event.clone());
        (!self.src.publish(event).await).then_some(Step::Exit(Exit::Closed))
    }

    async fn round(&mut self) -> Step {
        let epoch = self.src.epoch();
        if epoch != self.epoch {
            // `/clear` ou troca do filho: quadro, chave e memória temporal são da conversa anterior.
            // A prévia zera sem publicar: o `rebind` do hub apaga o retrato e o app recebe `reset`.
            (self.mem, self.epoch) = (Memory::default(), epoch);
        }
        let captured = self.src.capture().await;
        let facts = self.src.facts().await;
        if self.src.epoch() != epoch {
            return Step::Again;
        }
        // `has-session` só separa "morreu" de "pane em branco": no quadro vazio, e na falha ao
        // entrar nela e a cada nova tentativa do pool (até lá ele repete a mesma falha sem tentar).
        // Falha que o pool não guarda é tentativa nova a cada rodada.
        let probe = match &captured {
            Ok(frame) => {
                self.mem.failure = None;
                frame.text.is_empty()
            }
            Err(f) => f.attempt.is_none() || self.mem.failure != Some(f.attempt),
        };
        if probe {
            match self.src.has_session().await {
                // Só a resposta definitiva conta como conferida; o resto repete no tique seguinte.
                Some(true) => if let Err(f) = &captured { self.mem.failure = Some(f.attempt) },
                None => {
                    if crate::warn_limit::allow(Some(self.src.name()), "state_mux_no_answer") {
                        tracing::warn!(session = self.src.name(), code = "state_mux_no_answer", "estado: has-session sem resposta");
                    }
                }
                Some(false) if facts.in_transfer => return Step::Sleep,
                Some(false) => match self.src.dead().await {
                    Ok(Dead::Ok) => {
                        let _ = self.src.publish(StateEvent { session: self.src.name().to_owned(), state: "dead".into(), ..Default::default() }).await;
                        return Step::Exit(Exit::Dead);
                    }
                    Ok(Dead::InTransfer) => return Step::Sleep,
                    Err(code) => return self.fail(UNAVAILABLE, code, &facts).await,
                },
            }
        }
        match captured {
            Err(f) => self.fail(OBSERVATION_FAILED, f.code, &facts).await,
            Ok(frame) => self.reduce(frame, facts, epoch).await,
        }
    }

    /// O estado fica no último evento, com o problema; um evento por código.
    async fn fail(&mut self, problem: &str, detail: String, facts: &RoundFacts) -> Step {
        // A prévia fica com o texto que tinha e espera a próxima rodada boa.
        self.mem.preview.fast = false;
        if self.mem.last.as_ref().is_some_and(|l| l.problema_detalhe.as_deref() == Some(detail.as_str())) {
            return Step::Sleep;
        }
        let base = match self.mem.last.take() {
            Some(last) => last,
            None => {
                // Sem quadro anterior, o estado sai das âncoras que não dependem do pane.
                let sid = self.src.sid();
                let found = match (&facts.plugin_state, &sid) {
                    (Some(p), _) => Some(p.clone()),
                    (None, Some(sid)) => self.src.files(Some(sid)).await.marker,
                    (None, None) => None,
                };
                let state = anchor(found.as_deref()).unwrap_or(&self.mem.reducer.held_state).to_owned();
                StateEvent { session: self.src.name().to_owned(), state, ..Default::default() }
            }
        };
        if crate::warn_limit::allow(Some(self.src.name()), problem) {
            tracing::warn!(session = self.src.name(), code = problem, detail, "estado: rodada sem quadro");
        }
        let event = StateEvent { problema: Some(problem.to_owned()), problema_detalhe: Some(detail), ..base };
        // A rodada boa seguinte publica de novo, mesmo igual à de antes da falha.
        self.mem.key = None;
        self.publish(event).await.unwrap_or(Step::Sleep)
    }

    async fn reduce(&mut self, frame: Frame, facts: RoundFacts, epoch: u64) -> Step {
        let name = self.src.name().to_owned();
        let sid = self.src.sid();
        let files = self.src.files(sid.as_deref()).await;
        let mut problem = facts.unavailable.clone().map(|d| (UNAVAILABLE, d));
        if let Some(observed) = permission::parse_permission_mode(&frame.text) {
            let key = sid.clone().unwrap_or_else(|| name.clone());
            if self.mem.permission.due(&key, observed, facts.permission_op) {
                match self.src.observe_permission(&key, observed).await {
                    Ok(answer) => self.mem.permission.answered(&key, observed, facts.permission_op, answer),
                    Err(code) => {
                        if crate::warn_limit::allow(Some(&name), PERMISSION_FAILED) {
                            tracing::warn!(session = name.as_str(), code = PERMISSION_FAILED, detail = code.as_str(), "estado: permission.observe falhou");
                        }
                        problem.get_or_insert((PERMISSION_FAILED, code));
                    }
                }
            }
        }
        if self.src.epoch() != epoch {
            return Step::Again;
        }
        let marker = files.marker.clone().zip(files.marker_ts);
        self.mem.preview.set_view(facts.body_columns, facts.band_anchor.clone(), marker);
        if !preview::tick(&self.src, &mut self.mem.preview, Some(&frame), epoch).await {
            return Step::Exit(Exit::Closed);
        }
        let reducer_facts = ReducerFacts {
            open_question: files.open_question, plugin_question: facts.question, plugin_state: facts.plugin_state,
            hook_state: files.marker, hook_grace: Some(HOOK_GRACE), status_line: files.status_line,
        };
        let (reduced, diagnostic) = terminal_state::reduce_analysis(frame.analysis, std::mem::take(&mut self.mem.reducer), reducer_facts);
        self.mem.reducer = reduced.memory;
        if let Some((plugin, pane)) = diagnostic.divergence {
            tracing::info!(session = name.as_str(), code = "state_plugin_diverged", plugin, pane, "estado: o plugin corrigiu o pane");
        }
        let a = reduced.analysis;
        let (mode, previous) = self.mem.permission.current.clone().unzip();
        let loop_info = files.loop_info.unwrap_or_default();
        let (problema, problema_detalhe) = problem.map(|(p, d)| (p.to_owned(), d)).unzip();
        let event = StateEvent {
            session: name, state: a.state, label: a.label, question: a.question, options: a.options,
            status_line: a.status_line, overlay: a.overlay, login: a.login, limited: a.limit_reset.is_some(),
            limit_reset: a.limit_reset, loop_status: loop_info.status, loop_iter: loop_info.iter, loop_max: loop_info.max,
            claude_permission_mode: mode, claude_previous_non_plan: previous, shells: files.shells,
            problema, problema_detalhe, ..Default::default()
        };
        let key = key_of(&event);
        if self.mem.key.as_ref() != Some(&key) {
            self.mem.key = Some(key);
            if let Some(step) = self.publish(event).await {
                return step;
            }
        }
        Step::Wait { alive: facts.alive }
    }
}

/// Captura em processo pelo `TerminalPool` (cliente `-C`), sem HTTP.
pub struct PoolCapture { pool: TerminalPool, request: CaptureRequest }

impl PoolCapture {
    /// `binding` é o session-id da conversa; `target` o pane do agente ou `=nome:`.
    pub fn new(pool: TerminalPool, name: &str, binding: &str, target: String) -> Self {
        let request = CaptureRequest { consumer: format!("monitor:{name}"), name: name.to_owned(), provider: "claude".into(),
            binding: binding.to_owned(), target, started: 0.0, lines: 200, colors: false, join: false };
        Self { pool, request }
    }

    pub async fn capture(&self) -> Result<Frame, CaptureFailed> {
        match self.pool.capture(self.request.clone()).await {
            Ok(r) => Ok(Frame { text: r.text, analysis: r.analysis }),
            Err(e) => Err(CaptureFailed { code: e.0.to_owned(), attempt: self.pool.failure_attempts(&self.request, &e).await }),
        }
    }

    pub async fn release(&self) {
        if let Err(e) = self.pool.release(&self.request.consumer).await {
            tracing::warn!(session = self.request.name.as_str(), code = e.0, "estado: liberar a observação falhou");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

    /// Fonte de mentira: um quadro por rodada, repetindo o último quando acabam.
    struct Fake {
        frames: Vec<Result<&'static str, CaptureFailed>>,
        round: AtomicU32,
        has_session: AtomicU32,
        facts: Mutex<RoundFacts>,
        wake: Arc<Notify>,
        epoch: AtomicU64,
        events: Mutex<Vec<(u32, StateEvent)>>,
    }

    impl Fake {
        fn new(frames: Vec<Result<&'static str, CaptureFailed>>) -> Arc<Self> {
            Arc::new(Self { frames, round: AtomicU32::new(0), has_session: AtomicU32::new(0), facts: Mutex::default(),
                wake: Arc::default(), epoch: AtomicU64::new(0), events: Mutex::default() })
        }
        fn rounds(&self) -> u32 { self.round.load(Ordering::SeqCst) }
        fn states(&self) -> Vec<String> { self.events.lock().unwrap().iter().map(|(_, e)| e.state.clone()).collect() }
    }

    impl Sources for Arc<Fake> {
        fn name(&self) -> &str { "s" }
        fn sid(&self) -> Option<String> { Some("sid".into()) }
        fn epoch(&self) -> u64 { self.epoch.load(Ordering::SeqCst) }
        fn wake(&self) -> Arc<Notify> { self.wake.clone() }
        async fn facts(&self) -> RoundFacts { self.facts.lock().unwrap().clone() }
        async fn capture(&self) -> Result<Frame, CaptureFailed> {
            let i = self.round.fetch_add(1, Ordering::SeqCst) as usize;
            let frame = self.frames[i.min(self.frames.len() - 1)].clone();
            frame.map(|t| Frame { text: t.into(), analysis: terminal_state::analyze(t) })
        }
        async fn has_session(&self) -> Option<bool> { self.has_session.fetch_add(1, Ordering::SeqCst); Some(true) }
        async fn dead(&self) -> Result<Dead, String> { Ok(Dead::Ok) }
        async fn observe_permission(&self, _: &str, mode: &str) -> Result<(String, String), String> { Ok((mode.into(), "manual".into())) }
        async fn files(&self, _: Option<&str>) -> FileFacts { FileFacts::default() }
        async fn publish(&self, event: StateEvent) -> bool {
            self.events.lock().unwrap().push((self.rounds(), event));
            true
        }
    }

    const SPINNER: &str = "✻ Thinking…\n────────────\n❯\n────────────";

    #[tokio::test(start_paused = true)]
    async fn rounds_counted_like_python() {
        // Spinner parado: STALE_LIMIT rodadas iguais viram idle. Com o plugin vivo acordando, as
        // rodadas acordadas contam igual às do relógio, e o idle chega antes de 3 × POLL.
        let fake = Fake::new(vec![Ok(SPINNER)]);
        fake.facts.lock().unwrap().alive = true;
        let task = tokio::spawn(Monitor::new(fake.clone()).run());
        for _ in 0..4 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            fake.wake.notify_waiters();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(fake.rounds(), 5, "uma rodada no início e uma por empurrão");
        assert_eq!(fake.states(), ["working", "idle"]);
        assert_eq!(fake.events.lock().unwrap()[1].0, 4, "idle na 4ª rodada, como o Python");
        // Sem plugin vivo, o empurrão não acorda: só o relógio.
        fake.facts.lock().unwrap().alive = false;
        tokio::time::sleep(POLL).await;
        let before = fake.rounds();
        fake.wake.notify_waiters();
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(fake.rounds(), before, "empurrão sem long-poll vivo não conta rodada");
        tokio::time::sleep(POLL).await;
        assert_eq!(fake.rounds(), before + 1);
        task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn has_session_only_on_failure_entry_and_retry() {
        let fail = |attempt| Err(CaptureFailed { code: "terminal observer EOF".into(), attempt });
        let mut frames = vec![Ok(SPINNER)];
        frames.extend(std::iter::repeat_n(fail(Some(0)), 5));
        frames.extend(std::iter::repeat_n(fail(Some(1)), 3));
        frames.extend(std::iter::repeat_n(fail(None), 3));
        frames.push(Ok(SPINNER));
        frames.push(fail(Some(0)));
        let fake = Fake::new(frames);
        let task = tokio::spawn(Monitor::new(fake.clone()).run());
        tokio::time::sleep(POLL * 14).await;
        assert!(fake.rounds() >= 14);
        // Entrada (tentativa 0), nova tentativa (1), falha que o pool não guarda (toda rodada: 3),
        // e a entrada de novo depois do quadro bom.
        assert_eq!(fake.has_session.load(Ordering::SeqCst), 6);
        let problems: Vec<_> = fake.events.lock().unwrap().iter().map(|(_, e)| e.problema.clone()).collect();
        assert_eq!(problems, [None, Some(OBSERVATION_FAILED.into()), None, Some(OBSERVATION_FAILED.into())],
            "um evento por código; a volta publica de novo");
        task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn rebind_resets_memory_and_publishes_again() {
        let fake = Fake::new(vec![Ok(SPINNER)]);
        let task = tokio::spawn(Monitor::new(fake.clone()).run());
        tokio::time::sleep(POLL * 4 + Duration::from_millis(10)).await;
        assert_eq!(fake.states(), ["working", "idle"]);
        fake.epoch.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(POLL + Duration::from_millis(10)).await;
        assert_eq!(fake.states(), ["working", "idle", "working"], "memória nova: o spinner conta do zero");
        task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn unavailable_facts_become_the_problem() {
        let fake = Fake::new(vec![Ok(SPINNER)]);
        fake.facts.lock().unwrap().unavailable = Some("state_facts_status:503".into());
        let task = tokio::spawn(Monitor::new(fake.clone()).run());
        tokio::time::sleep(Duration::from_millis(10)).await;
        fake.facts.lock().unwrap().unavailable = None;
        tokio::time::sleep(POLL).await;
        let events: Vec<_> = fake.events.lock().unwrap().iter().map(|(_, e)| (e.state.clone(), e.problema_detalhe.clone())).collect();
        assert_eq!(events, [("working".into(), Some("state_facts_status:503".into())), ("working".into(), None)]);
        assert_eq!(fake.events.lock().unwrap()[0].1.problema.as_deref(), Some(UNAVAILABLE));
        task.abort();
    }
}
