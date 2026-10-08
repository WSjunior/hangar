//! Estado das linhas Claude da lista (`registry.list_with_state`): marcador do hook primeiro, pane
//! só quando ele não basta, statusline com teto de capturas e radar de limite da travada. Pura
//! sobre as linhas, os arquivos de fatos, os quadros e o relógio que `CaptureSource` entrega.
//! Codex, Pi, omp e Kimi ficam de fora: o estado deles vem dos fatos do Python.
use std::collections::{BTreeMap, HashSet};
use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use hangar_api::session::SessionRow;
use serde_json::Value;

use super::capped::SESSION_CAP;
use super::facts_files::{self, HookStates};
use crate::state::capture::MuxProcess;
use crate::state::published::Published;
use crate::terminal_state::{analyze, held_question, PaneAnalysis};
use hangar_api::state::StateEvent;

/// Escrita no transcript até 1 s depois do idle é o resumo pós-Stop, não turno novo.
pub const IDLE_STALE_S: f64 = 1.0;
/// O Notification de "idle 60s" chega depois do Stop; antes disso o menu pode ainda não ter desenhado.
pub const AWAITING_DEMOTE_GRACE_S: f64 = 10.0;
pub const STATUS_TTL_S: f64 = 20.0;
pub const STATUS_BUDGET: usize = 2;
pub const LIMIT_TTL_S: f64 = 30.0;
/// Intervalo da segunda captura: spinner que não mudou nele está congelado no scrollback.
pub const SPINNER_RECHECK: Duration = Duration::from_millis(150);
/// Sessão sem terminal que o retrato do runtime não trouxe: o estado sai do marcador, e a linha diz.
pub const RUNTIME_ABSENT: &str = "list_runtime_absent";

/// Capturas simultâneas numa rodada: cada uma é um processo do multiplexador.
const CAPTURE_PARALLEL: usize = 4;

/// Os quadros na ordem dos nomes, no máximo `CAPTURE_PARALLEL` processos de uma vez.
async fn capture_all<C: CaptureSource>(io: &C, names: Vec<&str>) -> Vec<Result<String, CaptureFailed>> {
    // Os futuros montados antes: um closure dentro do stream tira o `Send` da rodada.
    let pending: Vec<_> = names.into_iter().map(|n| io.capture(n)).collect();
    futures_util::stream::iter(pending).buffered(CAPTURE_PARALLEL).collect().await
}

/// Captura que não devolveu quadro; `code` vai ao log, nunca a saída do multiplexador.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureFailed { pub code: &'static str }

/// Quadros do pane e o relógio da rodada. `wall` é época em s; `mono` só serve para idade de cache.
pub trait CaptureSource: Sync {
    fn capture(&self, name: &str) -> impl Future<Output = Result<String, CaptureFailed>> + Send;
    fn pause(&self, d: Duration) -> impl Future<Output = ()> + Send;
    fn wall(&self) -> f64;
    fn mono(&self) -> f64;
}

/// O que a rodada pede a quem é dono do registro (o Python, a partir da Fase B).
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// O pane contradisse um marcador `awaiting_input` vencido: rebaixar para idle no mapa e no sidecar.
    /// `ts` é a versão do estado rebaixado (o `statusUpdatedAt` do registro nativo).
    DemoteAwaiting { sid: String, ts: f64 },
}

/// Fatos lidos fora daqui para esta rodada. Os arquivos (transcript, pergunta aberta, sidecar da
/// statusline) são lidos durante a rodada: quem chama roda `classify` fora da thread do runtime.
pub struct Facts<'a> {
    pub hooks: &'a HookStates,
    pub alive: &'a (dyn Fn(i64) -> bool + Sync),
    pub config_dirs: &'a [PathBuf],
    /// Retrato do runtime das sessões sem terminal, por nome (`RuntimeRegistry::snapshots`). `None`:
    /// servidor sem runtime ligado; sessão fora de um retrato fornecido está parada.
    pub headless: Option<&'a BTreeMap<String, Value>>,
    /// Código de problema do runtime, por nome.
    pub problems: &'a BTreeMap<String, String>,
    pub stall_seconds: f64,
    /// Pergunta que o hook do plugin segura agora (`plugin_bridge.pergunta_pendente`), por nome.
    pub held: &'a BTreeMap<String, Value>,
    /// Último estado de cada `Monitor` vivo.
    pub monitors: &'a Published,
}

/// Caches que atravessam rodadas.
#[derive(Default)]
pub struct Classifier {
    idle_checked: BTreeMap<String, f64>,
    status: BTreeMap<String, (f64, Option<String>)>,
    label: BTreeMap<String, Option<String>>,
    limit: BTreeMap<String, (f64, Option<String>)>,
}

fn sid(row: &SessionRow) -> Option<String> {
    row.jsonl.as_deref().and_then(|j| Path::new(j).file_stem()).and_then(|s| s.to_str()).map(str::to_owned)
}

fn mtime(name: &str, jsonl: Option<&str>) -> Option<f64> {
    match std::fs::metadata(jsonl?).and_then(|m| m.modified()) {
        Ok(t) => t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs_f64()),
        // Ausente é o transcript que ainda não nasceu; o resto deixa a atividade cega e precisa de rastro.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            if crate::warn_limit::allow(Some(name), "list_transcript_unreadable") {
                tracing::warn!(session = name, kind = ?e.kind(), "lista: mtime do transcript ilegível");
            }
            None
        }
    }
}

/// Sem terminal: o runtime vivo responde; parado (fora do retrato ou `alive: false`), vale o marcador
/// `working`/`idle`. Sem retrato nenhum, também o marcador, com a ausência na linha: sessão parada
/// calada esconderia que o runtime não foi consultado.
fn headless_state(row: &mut SessionRow, retrato: Option<&BTreeMap<String, Value>>, marker: Option<&facts_files::Marker>) {
    let Some(retrato) = retrato else {
        row.problema = Some(RUNTIME_ABSENT.into());
        row.state = marker.map(|m| m.state.as_str()).filter(|s| ["working", "idle"].contains(s)).unwrap_or("idle").into();
        return;
    };
    let snapshot = retrato.get(&row.name);
    // Ausente ou `alive: false` é a sessão parada; erro ou retrato sem estado é falha do runtime.
    let broken = snapshot.is_some_and(|s| !s["error"].is_null()
        || (s["view"]["alive"] == true && !s["view"]["public_state"]["state"].is_string()));
    if broken { row.problema = Some("list_runtime_unavailable".into()); }
    let live = snapshot.filter(|s| !broken && s["view"]["alive"] == true).map(|s| &s["view"]["public_state"]);
    let Some(state) = live else {
        row.state = marker.map(|m| m.state.as_str()).filter(|s| ["working", "idle"].contains(s)).unwrap_or("idle").into();
        return;
    };
    let text = |k: &str| state[k].as_str().map(str::to_owned);
    row.state = text("state").unwrap_or_default();
    row.label = text("label");
    row.question = text("question");
    row.options = state["options"].as_array().map(|o| o.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect());
    row.status_line = text("status_line");
    if let Some(q) = state["codex_question"].as_object() {
        row.state = "awaiting_input".into();
        row.question = q["questions"][0]["question"].as_str().map(str::to_owned);
    }
}

/// O que o `Monitor` vivo publicou: estado, pergunta, statusline e limite; o problema dele só
/// quando a lista não tem outro.
fn from_monitor(row: &mut SessionRow, ev: &StateEvent) {
    row.state.clone_from(&ev.state);
    row.label.clone_from(&ev.label);
    row.question.clone_from(&ev.question);
    row.options.clone_from(&ev.options);
    row.status_line.clone_from(&ev.status_line);
    row.limit_reset.clone_from(&ev.limit_reset);
    row.limited = ev.limited;
    if row.problema.is_none() { row.problema.clone_from(&ev.problema); }
}

fn apply(row: &mut SessionRow, a: &PaneAnalysis) {
    row.state = a.state.clone();
    row.label = a.label.clone();
    row.question = a.question.clone();
    row.options = a.options.clone();
}

impl Classifier {
    /// Caches expostos para o contrato com o Python comparar tique a tique.
    pub fn idle_checked(&self) -> &BTreeMap<String, f64> { &self.idle_checked }
    pub fn status_cache(&self) -> &BTreeMap<String, (f64, Option<String>)> { &self.status }
    pub fn label_cache(&self) -> &BTreeMap<String, Option<String>> { &self.label }
    pub fn limit_cache(&self) -> &BTreeMap<String, (f64, Option<String>)> { &self.limit }

    /// Nome reusado por outra sessão não herda a statusline, o spinner nem o limite da anterior.
    pub fn forget(&mut self, name: &str) {
        // O Python não limpa o `_idle_conferido`; aqui limpa, para o nome reusado não herdá-lo.
        self.idle_checked.remove(name);
        self.status.remove(name);
        self.label.remove(name);
        self.limit.remove(name);
    }

    /// Teto dos mapas por nome: acima dele, quem não está nas linhas da rodada sai. O caminho
    /// normal é o `forget` de quem fecha a sessão.
    fn cap(&mut self, rows: &[SessionRow]) {
        let over = [self.idle_checked.len(), self.status.len(), self.label.len(), self.limit.len()]
            .into_iter().any(|n| n > SESSION_CAP);
        if !over {
            return;
        }
        let live: HashSet<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        self.idle_checked.retain(|n, _| live.contains(n.as_str()));
        self.status.retain(|n, _| live.contains(n.as_str()));
        self.label.retain(|n, _| live.contains(n.as_str()));
        self.limit.retain(|n, _| live.contains(n.as_str()));
    }

    pub fn rename(&mut self, old: &str, new: &str) {
        if let Some(v) = self.idle_checked.remove(old) { self.idle_checked.insert(new.into(), v); }
        if let Some(v) = self.status.remove(old) { self.status.insert(new.into(), v); }
        if let Some(v) = self.label.remove(old) { self.label.insert(new.into(), v); }
        if let Some(v) = self.limit.remove(old) { self.limit.insert(new.into(), v); }
    }

    /// Decide estado, rótulo, pergunta, statusline, travada e limite das linhas Claude.
    pub async fn classify<C: CaptureSource>(&mut self, rows: &mut [SessionRow], facts: &Facts<'_>, io: &C) -> Vec<Effect> {
        let effects = self.classify_some(rows, facts, io).await;
        self.cap(rows);
        effects
    }

    /// `classify` de parte das linhas (a sessão cujo arquivo mudou): sem o teto dos caches, que
    /// tiraria as demais por não estarem na rodada.
    pub async fn classify_some<C: CaptureSource>(&mut self, rows: &mut [SessionRow], facts: &Facts<'_>, io: &C) -> Vec<Effect> {
        let mut effects = Vec::new();
        let mut pending = Vec::new();
        // Linhas com `Monitor` vivo: nenhuma captura, nem a da statusline nem a do limite.
        let mut watched = HashSet::new();
        for (i, row) in rows.iter_mut().enumerate() {
            // Codex sem terminal com chat aberto: o feed do hub e o chat dizem o mesmo; sem ele, vale o
            // fato do Python (o mesmo espelho da vista do Rust). Contagem, conta e atividade seguem dos fatos.
            if row.provider == "codex" && row.headless
                && let Some(ev) = facts.monitors.get(&row.name, sid(row).as_deref())
            {
                from_monitor(row, &ev);
                if let Some(q) = &ev.codex_question {
                    row.state = "awaiting_input".into();
                    row.question = q.get("questions").and_then(|qs| qs[0]["question"].as_str()).map(str::to_owned);
                }
                continue;
            }
            if row.provider != "claude" {
                continue;
            }
            let sid = sid(row);
            let marker = facts.hooks.get_state(sid.as_deref(), facts.alive);
            if row.headless {
                row.last_activity = mtime(&row.name, row.jsonl.as_deref());
                headless_state(row, facts.headless, marker.as_ref());
                // O problema registrado pelo adaptador é mais preciso que a falha genérica do retrato; a
                // ausência do retrato fica, senão a linha passaria por consultada.
                if let Some(p) = facts.problems.get(&row.name)
                    && row.problema.as_deref() != Some(RUNTIME_ABSENT) { row.problema = Some(p.clone()); }
                continue;
            }
            if row.problema.is_none() {
                row.problema = facts.problems.get(&row.name).cloned();
            }
            if let Some(ev) = facts.monitors.get(&row.name, sid.as_deref()) {
                from_monitor(row, &ev);
                row.last_activity = mtime(&row.name, row.jsonl.as_deref());
                watched.insert(row.name.clone());
                continue;
            }
            let Some(m) = marker else { pending.push(i); continue };
            let mtime = mtime(&row.name, row.jsonl.as_deref());
            if m.state == "idle" && mtime.is_some_and(|t| t > m.ts + IDLE_STALE_S)
                && self.idle_checked.get(&row.name).copied() != mtime {
                // Transcript escrito depois do idle é turno aberto sem UserPromptSubmit (agente de fundo).
                pending.push(i);
            } else if m.state != "awaiting_input" {
                row.state = m.state;
                row.last_activity = mtime;
                if row.state != "working" { self.label.remove(&row.name); }
            } else {
                // O marcador não carrega a pergunta: só o pane tem as opções.
                pending.push(i);
            }
        }

        if !pending.is_empty() {
            let frames = capture_all(io, pending.iter().map(|&i| rows[i].name.as_str()).collect()).await;
            let mut analyses: Vec<Option<PaneAnalysis>> = frames.iter().map(|f| f.as_ref().ok().map(|t| analyze(t))).collect();
            let spinning: Vec<usize> = (0..pending.len())
                .filter(|&k| analyses[k].as_ref().is_some_and(|a| a.state == "working")).collect();
            if !spinning.is_empty() {
                io.pause(SPINNER_RECHECK).await;
                let again = capture_all(io, spinning.iter().map(|&k| rows[pending[k]].name.as_str()).collect()).await;
                for (k, frame) in spinning.into_iter().zip(again) {
                    let a = analyses[k].as_mut().unwrap();
                    match frame {
                        Ok(text) => if analyze(&text).spinner.as_ref().is_none_or(|s| Some(s) == a.spinner.as_ref()) {
                            (a.state, a.label, a.question, a.options) = ("idle".into(), None, None, None);
                        },
                        Err(e) => { analyses[k] = None; self.capture_failed(&rows[pending[k]].name, e); }
                    }
                }
            }
            let wall = io.wall();
            for (k, &i) in pending.iter().enumerate() {
                let row = &mut rows[i];
                let marker = facts.hooks.get_state(sid(row).as_deref(), facts.alive);
                row.last_activity = mtime(&row.name, row.jsonl.as_deref());
                let Some(a) = &analyses[k] else {
                    // Sem quadro não há prova de nada: fica o marcador, sem rebaixar nem conferir o idle.
                    if let Err(e) = &frames[k] { self.capture_failed(&row.name, *e); }
                    row.state = marker.map_or_else(|| "idle".into(), |m| m.state);
                    row.problema.get_or_insert_with(|| "list_capture_failed".into());
                    // O spinner guardado é de antes da falha: preenchido de volta, viraria rótulo fantasma.
                    self.label.remove(&row.name);
                    continue;
                };
                apply(row, a);
                match row.last_activity {
                    Some(t) if a.state == "idle" => { self.idle_checked.insert(row.name.clone(), t); }
                    _ => { self.idle_checked.remove(&row.name); }
                }
                if let (Some(m), Some(sid)) = (&marker, sid(row)) {
                    if m.state == "awaiting_input" && a.state != "awaiting_input" && wall - m.ts > AWAITING_DEMOTE_GRACE_S {
                        effects.push(Effect::DemoteAwaiting { sid, ts: m.ts });
                    }
                }
                row.limit_reset = a.limit_reset.clone();
                row.limited = row.limit_reset.is_some();
                let now = io.mono();
                self.limit.insert(row.name.clone(), (now, a.limit_reset.clone()));
                self.status.insert(row.name.clone(), (now, a.status_line.clone()));
                self.label.insert(row.name.clone(), a.label.clone());
            }
        }

        // Pergunta que o pane não mostra: a permissão que o hook do plugin segura para o app (a TUI
        // não desenha cartão e o registro nativo segue `busy`), ou o menu que rolou para fora e o
        // marcador não carrega.
        for row in rows.iter_mut() {
            if row.provider != "claude" || row.headless || row.state == "awaiting_input"
                || row.options.as_ref().is_some_and(|o| !o.is_empty()) || watched.contains(&row.name) {
                continue;
            }
            if let Some(q) = facts.held.get(&row.name).and_then(held_question) {
                row.state = "awaiting_input".into();
                row.label = None;
                row.question = q.question;
                row.options = Some(q.options);
                continue;
            }
            if row.jsonl.is_none() {
                continue;
            }
            if let Some(q) = facts_files::open_question(sid(row).as_deref(), facts.config_dirs) {
                row.state = "awaiting_input".into();
                row.label = None;
                row.question = Some(q.question);
                row.options = Some(q.options);
            }
        }

        // Statusline das que não foram raspadas: no máximo STATUS_BUDGET capturas, das mais velhas.
        let mut scraped: HashSet<String> = pending.iter().map(|&i| rows[i].name.clone()).collect();
        scraped.extend(watched.iter().cloned());
        let now = io.mono();
        let age = |c: &BTreeMap<String, (f64, Option<String>)>, n: &str| c.get(n).map_or(0.0, |v| v.0);
        let mut stale: Vec<String> = rows.iter()
            .filter(|r| r.provider == "claude" && !r.headless && !scraped.contains(&r.name)
                && now - age(&self.status, &r.name) > STATUS_TTL_S)
            .map(|r| r.name.clone()).collect();
        stale.sort_by(|a, b| age(&self.status, a).total_cmp(&age(&self.status, b)));
        stale.truncate(STATUS_BUDGET);
        for name in stale {
            match io.capture(&name).await {
                Ok(text) => {
                    let a = analyze(&text);
                    let now = io.mono();
                    self.status.insert(name.clone(), (now, a.status_line.clone()));
                    self.limit.insert(name.clone(), (now, a.limit_reset.clone()));
                    // Uma captura só não separa spinner vivo de congelado: só guarda o que parece vivo.
                    if a.state == "working" { self.label.insert(name, a.label); } else { self.label.remove(&name); }
                }
                Err(e) => {
                    // Statusline velha ainda é verdadeira; spinner velho vira fantasma.
                    self.capture_failed(&name, e);
                    let prev = self.status.get(&name).and_then(|v| v.1.clone());
                    self.status.insert(name.clone(), (io.mono(), prev));
                    self.label.remove(&name);
                }
            }
        }

        let wall = io.wall();
        for row in rows.iter_mut().filter(|r| r.provider == "claude") {
            let published = || facts_files::published_status(sid(row).as_deref(), facts.config_dirs, wall).map(|p| p.line);
            if row.headless {
                if row.status_line.is_none() { row.status_line = published(); }
                continue;
            }
            if watched.contains(&row.name) {
                continue;
            }
            // Sidecar antes do pane: a captura traz a linha cortada na largura da janela.
            row.status_line = published().or_else(|| self.status.get(&row.name).and_then(|v| v.1.clone()));
            if row.state == "working" && row.label.is_none() {
                row.label = self.label.get(&row.name).cloned().flatten();
            }
        }
        for row in rows.iter_mut().filter(|r| r.provider == "claude") {
            row.stalled = row.state == "working" && row.last_activity.is_some_and(|t| wall - t > facts.stall_seconds);
        }
        self.limit_radar(rows, &scraped, io).await;
        effects
    }

    /// Travada (`working` sem o transcript andar) é quem espera o limite voltar: só ela paga a
    /// captura que o marcador poupou, uma vez a cada LIMIT_TTL_S.
    async fn limit_radar<C: CaptureSource>(&mut self, rows: &mut [SessionRow], scraped: &HashSet<String>, io: &C) {
        let now = io.mono();
        let targets: Vec<usize> = (0..rows.len()).filter(|&i| {
            let r = &rows[i];
            r.provider == "claude" && !r.headless && r.stalled && !scraped.contains(&r.name)
        }).collect();
        let fresh: Vec<usize> = targets.iter().copied()
            .filter(|&i| now - self.limit.get(&rows[i].name).map_or(0.0, |v| v.0) > LIMIT_TTL_S).collect();
        let frames = capture_all(io, fresh.iter().map(|&i| rows[i].name.as_str()).collect()).await;
        for (&i, frame) in fresh.iter().zip(frames) {
            let name = rows[i].name.clone();
            let reset = match frame {
                Ok(text) => analyze(&text).limit_reset,
                Err(e) => {
                    // Captura falhou: fica o último valor bom.
                    self.capture_failed(&name, e);
                    self.limit.get(&name).and_then(|v| v.1.clone())
                }
            };
            self.limit.insert(name, (now, reset));
        }
        for i in targets {
            let row = &mut rows[i];
            row.limit_reset = self.limit.get(&row.name).and_then(|v| v.1.clone());
            row.limited = row.limit_reset.is_some();
        }
    }

    fn capture_failed(&self, name: &str, e: CaptureFailed) {
        if crate::warn_limit::allow(Some(name), e.code) {
            tracing::warn!(session = name, code = e.code, "lista: captura do pane falhou");
        }
    }
}

/// Captura avulsa pelo multiplexador: a sessão sem chat aberto não tem cliente `-C` para alugar.
pub struct MuxCapture {
    mux: std::sync::Arc<MuxProcess>,
    /// Pane do agente por nome, da descoberta; sem ele vale o ativo da sessão (`=nome:`).
    pub targets: std::sync::Arc<BTreeMap<String, String>>,
}

/// Uma origem por processo: os caches do `Classifier` atravessam rodadas e instâncias de captura.
fn process_start() -> std::time::Instant {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    *START.get_or_init(std::time::Instant::now)
}

impl MuxCapture {
    pub fn new(program: impl Into<OsString>, timeout: Duration, targets: std::sync::Arc<BTreeMap<String, String>>) -> Self {
        Self { mux: std::sync::Arc::new(MuxProcess::new(program, timeout)), targets }
    }
}

impl CaptureSource for MuxCapture {
    fn capture(&self, name: &str) -> impl Future<Output = Result<String, CaptureFailed>> + Send {
        let target = self.targets.get(name).cloned().unwrap_or_else(|| format!("={name}:"));
        let (mux, name) = (self.mux.clone(), name.to_owned());
        async move { mux.capture_checked(&name, &target).await.map_err(|code| CaptureFailed { code }) }
    }
    fn pause(&self, d: Duration) -> impl Future<Output = ()> + Send { tokio::time::sleep(d) }
    fn wall(&self) -> f64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
    }
    fn mono(&self) -> f64 { process_start().elapsed().as_secs_f64() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    const IDLE: &str = "● pronto\n────────────\n❯\n────────────\n🤖 Opus 4.5";

    struct Fixed { frame: Result<String, CaptureFailed>, wall: f64, calls: Mutex<u32> }

    impl CaptureSource for Fixed {
        async fn capture(&self, _: &str) -> Result<String, CaptureFailed> {
            *self.calls.lock().unwrap() += 1;
            self.frame.clone()
        }
        async fn pause(&self, _: Duration) {}
        fn wall(&self) -> f64 { self.wall }
        fn mono(&self) -> f64 { self.wall }
    }

    fn setup(marker: &str, ts: f64) -> (tempfile::TempDir, Vec<PathBuf>, SessionRow) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join(".claude");
        std::fs::create_dir_all(cfg.join(".hangar-state")).unwrap();
        std::fs::write(cfg.join(".hangar-state/abc.json"), format!(r#"{{"state":"{marker}","ts":{ts}}}"#)).unwrap();
        let row: SessionRow = serde_json::from_value(serde_json::json!({
            "name": "s", "jsonl": dir.path().join("abc.jsonl").to_str().unwrap()})).unwrap();
        (dir, vec![cfg], row)
    }

    async fn run(row: &mut SessionRow, dirs: &[PathBuf], io: &Fixed, headless: Option<&BTreeMap<String, Value>>) -> Vec<Effect> {
        let hooks = HookStates::load(dirs);
        let facts = Facts { hooks: &hooks, alive: &|_| false, config_dirs: dirs, headless,
                            problems: &BTreeMap::new(), stall_seconds: 300.0, held: &BTreeMap::new(), monitors: &Published::default() };
        Classifier::default().classify(std::slice::from_mut(row), &facts, io).await
    }

    #[tokio::test]
    async fn permission_card_after_bash_is_awaiting() {
        // Com o app aberto, o hook do plugin segura a permissão do Bash: a TUI não desenha cartão,
        // o registro nativo fica `busy` e o marcador `working`. Só a pergunta segurada diz que espera.
        let (_d, dirs, mut row) = setup("working", 990.0);
        let io = Fixed { frame: Ok(IDLE.into()), wall: 1000.0, calls: Mutex::new(0) };
        let hooks = HookStates::load(&dirs);
        let held = BTreeMap::from([("s".to_owned(), serde_json::json!({"id": "perm:t1", "questions": [], "tool": "Bash", "resumo": "ls"}))]);
        let facts = Facts { hooks: &hooks, alive: &|_| false, config_dirs: &dirs, headless: Some(&BTreeMap::new()),
                            problems: &BTreeMap::new(), stall_seconds: 300.0, held: &held, monitors: &Published::default() };
        Classifier::default().classify(std::slice::from_mut(&mut row), &facts, &io).await;
        assert_eq!((row.state.as_str(), row.question.as_deref()), ("awaiting_input", Some("Bash: ls")));
        assert_eq!(row.options, Some(vec!["Yes".into(), "No".into()]));
        assert_eq!(row.label, None);
        // Sem pergunta segurada, o marcador responde como antes.
        let mut row2 = setup("working", 990.0).2;
        row2.jsonl = row.jsonl.clone();
        run(&mut row2, &dirs, &io, Some(&BTreeMap::new())).await;
        assert_eq!(row2.state, "working");
    }

    #[tokio::test]
    async fn uses_monitor_state_when_alive() {
        // Marcador awaiting manda capturar; com `Monitor` vivo, vale o que ele publicou, sem captura.
        let (_d, dirs, mut row) = setup("awaiting_input", 900.0);
        let io = Fixed { frame: Ok(IDLE.into()), wall: 1000.0, calls: Mutex::new(0) };
        let hooks = HookStates::load(&dirs);
        let monitors = Published::default();
        monitors.set(1, "s", Some("abc".into()), std::sync::Arc::new(hangar_api::state::StateEvent {
            session: "s".into(), state: "awaiting_input".into(), question: Some("Bash: ls".into()),
            options: Some(vec!["Yes".into(), "No".into()]), status_line: Some("🤖 Haiku".into()),
            limited: true, limit_reset: Some("9:10pm".into()), ..Default::default() }));
        let facts = Facts { hooks: &hooks, alive: &|_| false, config_dirs: &dirs, headless: Some(&BTreeMap::new()),
                            problems: &BTreeMap::new(), stall_seconds: 300.0, held: &BTreeMap::new(), monitors: &monitors };
        let effects = Classifier::default().classify(std::slice::from_mut(&mut row), &facts, &io).await;
        assert_eq!(*io.calls.lock().unwrap(), 0, "nenhuma captura com o Monitor vivo");
        assert_eq!(effects, vec![], "o Monitor não rebaixa o registro nativo pela lista");
        assert_eq!((row.state.as_str(), row.question.as_deref()), ("awaiting_input", Some("Bash: ls")));
        assert_eq!((row.status_line.as_deref(), row.limited, row.limit_reset.as_deref()), (Some("🤖 Haiku"), true, Some("9:10pm")));
        // Outra conversa (depois do `/clear`): o retrato do Monitor não vale e a lista captura.
        monitors.set(1, "s", Some("outra".into()), std::sync::Arc::new(hangar_api::state::StateEvent {
            state: "working".into(), ..Default::default() }));
        Classifier::default().classify(std::slice::from_mut(&mut row), &facts, &io).await;
        assert!(*io.calls.lock().unwrap() > 0);
        assert_eq!(row.state, "idle");
    }

    #[tokio::test]
    async fn asks_demote_after_grace() {
        // Marcador awaiting com 5 s: pane sem menu ainda não rebaixa; com 60 s, rebaixa.
        for (age, want) in [(5.0, vec![]), (60.0, vec![Effect::DemoteAwaiting { sid: "abc".into(), ts: 940.0 }])] {
            let (_d, dirs, mut row) = setup("awaiting_input", 1000.0 - age);
            let io = Fixed { frame: Ok(IDLE.into()), wall: 1000.0, calls: Mutex::new(0) };
            assert_eq!(run(&mut row, &dirs, &io, Some(&BTreeMap::new())).await, want, "idade {age}");
            assert_eq!(row.state, "idle");
        }
        // Sem quadro não há prova: o marcador fica, sem rebaixar, e a falha aparece na linha.
        let (_d, dirs, mut row) = setup("awaiting_input", 900.0);
        let io = Fixed { frame: Err(CaptureFailed { code: "capture_refused" }), wall: 1000.0, calls: Mutex::new(0) };
        assert_eq!(run(&mut row, &dirs, &io, Some(&BTreeMap::new())).await, vec![]);
        assert_eq!((row.state.as_str(), row.problema.as_deref()), ("awaiting_input", Some("list_capture_failed")));
    }

    /// Custo de um tique com 20 sessões, sem o subprocesso (quadro fixo). Medir em release:
    /// `cargo test --release -p hangar-server --lib list::classify::tests::tick_cost -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn tick_cost() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join(".claude");
        std::fs::create_dir_all(cfg.join(".hangar-state")).unwrap();
        let dirs = vec![cfg.clone()];
        let rows: Vec<SessionRow> = (0..20).map(|i| {
            let jsonl = dir.path().join(format!("s{i}.jsonl"));
            std::fs::write(&jsonl, "{}\n").unwrap();
            serde_json::from_value(serde_json::json!({"name": format!("s{i}"), "jsonl": jsonl.to_str().unwrap()})).unwrap()
        }).collect();
        let frame = format!("{}\n{IDLE}", "● linha de resposta comprida o bastante\n".repeat(190));
        for (label, marker) in [("marcador working", Some("working")), ("sem marcador (tudo no pane)", None),
                                ("marcador awaiting (tudo no pane)", Some("awaiting_input"))] {
            for i in 0..20 {
                let f = cfg.join(format!(".hangar-state/s{i}.json"));
                match marker {
                    Some(m) => std::fs::write(&f, format!(r#"{{"state":"{m}","ts":1.0}}"#)).unwrap(),
                    None => { let _ = std::fs::remove_file(&f); }
                }
            }
            let hooks = HookStates::load(&dirs);
            let (headless, problems) = (BTreeMap::new(), BTreeMap::new());
            let facts = Facts { hooks: &hooks, alive: &|_| false, config_dirs: &dirs, headless: Some(&headless),
                                problems: &problems, stall_seconds: 1e12, held: &BTreeMap::new(), monitors: &Published::default() };
            let mut c = Classifier::default();
            let io = Fixed { frame: Ok(frame.clone()), wall: 1e9, calls: Mutex::new(0) };
            let n = 200;
            let t = std::time::Instant::now();
            for _ in 0..n {
                let mut r = rows.clone();
                c.classify(&mut r, &facts, &io).await;
            }
            let calls = *io.calls.lock().unwrap() as f64 / n as f64;
            println!("{label}: {:.0} µs/tique, {calls:.1} capturas/tique", t.elapsed().as_micros() as f64 / n as f64);
        }
    }

    /// Captura que cede a vez enquanto está "rodando", contando quantas há ao mesmo tempo.
    struct Slow { now: Mutex<usize>, peak: Mutex<usize> }

    impl CaptureSource for Slow {
        async fn capture(&self, _: &str) -> Result<String, CaptureFailed> {
            { let mut n = self.now.lock().unwrap(); *n += 1; let mut p = self.peak.lock().unwrap(); *p = (*p).max(*n); }
            for _ in 0..3 { tokio::task::yield_now().await; }
            *self.now.lock().unwrap() -= 1;
            Ok(IDLE.into())
        }
        async fn pause(&self, _: Duration) {}
        fn wall(&self) -> f64 { 1000.0 }
        fn mono(&self) -> f64 { 1000.0 }
    }

    #[tokio::test]
    async fn captures_have_a_ceiling() {
        let mut rows: Vec<SessionRow> = (0..20).map(|i| serde_json::from_value(serde_json::json!({"name": format!("s{i}")})).unwrap()).collect();
        let hooks = HookStates::default();
        let (headless, problems) = (BTreeMap::new(), BTreeMap::new());
        let facts = Facts { hooks: &hooks, alive: &|_| false, config_dirs: &[], headless: Some(&headless),
                            problems: &problems, stall_seconds: 300.0, held: &BTreeMap::new(), monitors: &Published::default() };
        let io = Slow { now: Mutex::new(0), peak: Mutex::new(0) };
        Classifier::default().classify(&mut rows, &facts, &io).await;
        assert!(rows.iter().all(|r| r.state == "idle"));
        assert_eq!(*io.peak.lock().unwrap(), CAPTURE_PARALLEL, "20 linhas sem marcador, no máximo 4 capturas juntas");
    }

    #[test]
    fn classify_runs_in_a_spawned_task() {
        fn send<T: Send>(_: T) {}
        let hooks = HookStates::default();
        let (headless, problems) = (BTreeMap::new(), BTreeMap::new());
        let facts = Facts { hooks: &hooks, alive: &|_| false, config_dirs: &[], headless: Some(&headless),
                            problems: &problems, stall_seconds: 300.0, held: &BTreeMap::new(), monitors: &Published::default() };
        let io = MuxCapture::new("tmux", Duration::from_secs(1), Default::default());
        send(Classifier::default().classify(&mut [], &facts, &io));
    }

    #[tokio::test]
    async fn headless_from_runtime() {
        let (_d, dirs, mut row) = setup("working", 990.0);
        row.headless = true;
        let io = Fixed { frame: Ok(String::new()), wall: 1000.0, calls: Mutex::new(0) };
        let snap = |alive: bool| BTreeMap::from([("s".to_owned(), serde_json::json!({"error": null, "view": {"alive": alive,
            "public_state": {"state": "awaiting_input", "label": null, "question": "Permitir Bash?",
                             "options": ["Permitir", "Negar"], "status_line": "🤖 Haiku 4.5"}}}))]);
        run(&mut row, &dirs, &io, Some(&snap(true))).await;
        assert_eq!((row.state.as_str(), row.question.as_deref()), ("awaiting_input", Some("Permitir Bash?")));
        assert_eq!(row.options, Some(vec!["Permitir".into(), "Negar".into()]));
        assert_eq!(row.status_line.as_deref(), Some("🤖 Haiku 4.5"));
        // Processo parado: o marcador responde. Pane nunca é raspado.
        row.question = None;
        run(&mut row, &dirs, &io, Some(&snap(false))).await;
        assert_eq!(row.state, "working");
        assert_eq!(*io.calls.lock().unwrap(), 0);
        // Runtime com erro não passa por sessão parada calada.
        let broken = BTreeMap::from([("s".to_owned(), serde_json::json!({"error": "cano_exited", "view": {"alive": true}}))]);
        run(&mut row, &dirs, &io, Some(&broken)).await;
        assert_eq!((row.state.as_str(), row.problema.as_deref()), ("working", Some("list_runtime_unavailable")));
    }

    #[tokio::test]
    async fn codex_headless_reads_the_feed() {
        // Linha do Codex sem terminal com o feed vivo: estado e pergunta dele; sem feed, o fato do Python.
        let row = || -> SessionRow { serde_json::from_value(serde_json::json!({"name": "cx", "provider": "codex", "headless": true,
            "state": "idle", "pending_questions": 2, "jsonl": "/x/rollout-abc.jsonl"})).unwrap() };
        let hooks = HookStates::default();
        let (headless, problems) = (BTreeMap::new(), BTreeMap::new());
        let monitors = Published::default();
        let io = Fixed { frame: Ok(String::new()), wall: 1000.0, calls: Mutex::new(0) };
        let facts = Facts { hooks: &hooks, alive: &|_| false, config_dirs: &[], headless: Some(&headless),
                            problems: &problems, stall_seconds: 1e12, held: &BTreeMap::new(), monitors: &monitors };
        let mut without = row();
        Classifier::default().classify(std::slice::from_mut(&mut without), &facts, &io).await;
        assert_eq!(without.state, "idle", "sem chat aberto vale o fato do Python");
        let question = serde_json::json!({"questions": [{"question": "Qual caminho?"}]});
        monitors.set(1, "cx", Some("rollout-abc".into()), std::sync::Arc::new(hangar_api::state::StateEvent {
            state: "working".into(), status_line: Some("gpt".into()), codex_question: question.as_object().cloned(), ..Default::default() }));
        let mut with = row();
        Classifier::default().classify(std::slice::from_mut(&mut with), &facts, &io).await;
        assert_eq!((with.state.as_str(), with.question.as_deref(), with.status_line.as_deref()),
                   ("awaiting_input", Some("Qual caminho?"), Some("gpt")));
        assert_eq!(with.pending_questions, without.pending_questions, "contagem segue dos fatos");
        assert_eq!(*io.calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn headless_without_runtime_view_is_marked() {
        let (_d, dirs, mut row) = setup("working", 990.0);
        row.headless = true;
        let io = Fixed { frame: Ok(String::new()), wall: 1000.0, calls: Mutex::new(0) };
        // Sem retrato fornecido, o marcador responde, mas a linha diz que o runtime não foi consultado.
        run(&mut row, &dirs, &io, None).await;
        assert_eq!((row.state.as_str(), row.problema.as_deref()), ("working", Some(RUNTIME_ABSENT)));
        // Retrato fornecido sem a sessão: ela está parada, como o `hl.snapshot() = None` do Python.
        let other = BTreeMap::from([("outra".to_owned(), serde_json::json!({"error": null, "view": {"alive": true}}))]);
        row.problema = None;
        run(&mut row, &dirs, &io, Some(&other)).await;
        assert_eq!((row.state.as_str(), row.problema.as_deref()), ("working", None));
        assert_eq!(*io.calls.lock().unwrap(), 0);
        // O problema do adaptador não apaga a ausência do retrato.
        let hooks = HookStates::load(&dirs);
        let problems = BTreeMap::from([("s".to_owned(), "cano_exited".to_owned())]);
        let facts = Facts { hooks: &hooks, alive: &|_| false, config_dirs: &dirs, headless: None,
                            problems: &problems, stall_seconds: 300.0, held: &BTreeMap::new(), monitors: &Published::default() };
        Classifier::default().classify(std::slice::from_mut(&mut row), &facts, &io).await;
        assert_eq!(row.problema.as_deref(), Some(RUNTIME_ABSENT));
    }
}
