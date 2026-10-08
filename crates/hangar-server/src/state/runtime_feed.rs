//! Estado ao vivo de Claude e Codex sem terminal: o hub publica o último valor que o ator escreveu no canal
//! em processo (`RuntimeRegistry::live`), coalescido, no lugar do `Monitor` e sem passar pelo Python.
use std::sync::{Arc, Weak};
use std::time::Duration;

use hangar_api::preview::PreviewEvent;
use hangar_api::state::StateEvent;
use tokio::sync::Notify;

use crate::runtime::protocol::LiveReceiver;
use crate::side::Hub;
use crate::state::published::Published;

/// Mesma janela da lista: rajada de deltas vira uma publicação.
pub const COALESCE: Duration = Duration::from_millis(150);

pub struct RuntimeFeed {
    hub: Weak<Hub>,
    name: String,
    wake: Arc<Notify>,
    live: Option<LiveReceiver>,
    published: Arc<Published>,
    owner: u64,
    /// Último dado publicado de cada evento, na época `generation`: troca de ligação republica tudo.
    sent: Sent,
}

#[derive(Default)]
struct Sent { generation: Option<u64>, question: Option<String>, state: Option<String>, preview: Option<String>, thinking: Option<String>, tool: Option<String> }

/// Problema do ator, como o `runtime_problem` do Python: `<código>: <frase>` com teto de 300.
fn actor_problem(code: &str, message: &str) -> String {
    let text = if message.is_empty() { code.to_owned() } else { format!("{code}: {message}") };
    text.chars().take(300).collect()
}

fn idle(name: &str) -> StateEvent { StateEvent { session: name.into(), state: "idle".into(), headless: true, ..Default::default() } }

/// Problema do próprio feed. Sai como `runtime_falhou`, o código que web, app e nativo traduzem;
/// o motivo vai no detalhe. Código novo sem tradução some da tela do web.
fn feed_problem(name: &str, code: &str, message: &str) -> StateEvent {
    StateEvent { problema: Some("runtime_falhou".into()), problema_detalhe: Some(actor_problem(code, message)), ..idle(name) }
}

impl RuntimeFeed {
    /// `live`: `None` quando o servidor não tem o runtime ligado.
    pub fn new(hub: &Arc<Hub>, live: Option<LiveReceiver>, published: Arc<Published>) -> Self {
        Self { hub: Arc::downgrade(hub), name: hub.name.clone(), wake: hub.wake(), live, published,
            owner: super::live::next_owner(), sent: Sent::default() }
    }

    pub async fn run(mut self) {
        loop {
            if !self.round() {
                return;
            }
            self.wait().await;
            tokio::time::sleep(COALESCE).await;
        }
    }

    /// Acorda pelo canal do ator ou pelo hub (resposta gravada, religação).
    async fn wait(&mut self) {
        match self.live.as_mut() {
            Some(rx) => tokio::select! {
                changed = rx.changed() => if changed.is_err() {
                    // Registro sumiu (servidor encerrando): fica só o hub.
                    self.live = None;
                    self.wake.notified().await;
                },
                _ = self.wake.notified() => {}
            },
            None => self.wake.notified().await,
        }
    }

    /// Publica o que mudou; `false` com o hub fechado.
    fn round(&mut self) -> bool {
        let Some(hub) = self.hub.upgrade() else { return false };
        let Some(generation) = hub.generation() else { return false };
        if self.sent.generation != Some(generation) {
            self.sent = Sent { generation: Some(generation), ..Sent::default() };
        }
        let value = self.live.as_mut().map(|rx| rx.borrow_and_update().clone());
        let (state, texts) = match &value {
            None => (feed_problem(&self.name, "runtime_absent", "o servidor não tem o runtime ligado"), Default::default()),
            // Sessão parada (não aberta no Rust, encerrada, ou abrindo): `idle`, como o Python. Abertura
            // que falhou e vida que acabou com erro chegam como `Some` com o erro.
            Some(None) => (idle(&self.name), Default::default()),
            Some(Some(live)) => {
                let mut state = if live.public_state.is_null() { idle(&self.name) } else {
                    serde_json::from_value(live.public_state.clone()).unwrap_or_else(|_| {
                        if crate::warn_limit::allow(Some(&self.name), "state_feed_invalid") {
                            tracing::warn!(session = self.name.as_str(), code = "state_feed_invalid", "estado: vista do ator não é um estado");
                        }
                        feed_problem(&self.name, "state_feed_invalid", "a vista do ator não é um estado")
                    })
                };
                if let Some((code, message)) = &live.error {
                    state.problema = Some("runtime_falhou".into());
                    state.problema_detalhe = Some(actor_problem(code, message));
                }
                (state, [live.preview.clone(), live.thinking.clone(), live.tool.clone()])
            }
        };
        let [preview, thinking, tool] = texts;
        let preview = match hub.committed() {
            Some(committed) if super::preview::is_committed(&preview, &committed) => String::new(),
            _ => preview,
        };
        // Mesma ordem do `sse.py`: a pergunta sai antes do estado que a abre.
        let question = serde_json::to_string(&state.codex_question).unwrap_or_else(|_| "null".into());
        if !self.emit(&hub, "ask_question", question, |s| &mut s.question) { return false; }
        let data = serde_json::to_string(&state).unwrap_or_default();
        if self.sent.state.as_deref() != Some(data.as_str()) {
            let sid = hub.jsonl().and_then(|j| j.file_stem().and_then(|s| s.to_str()).map(str::to_owned));
            if !self.emit(&hub, "state", data, |s| &mut s.state) { return false; }
            self.published.set(self.owner, &self.name, sid, Arc::new(state));
        }
        let preview = serde_json::to_string(&PreviewEvent { session: self.name.clone(), text: preview, md: true, full: true, vivo: true }).unwrap_or_default();
        self.emit(&hub, "preview", preview, |s| &mut s.preview)
            && self.emit(&hub, "pensamento", serde_json::json!({"text": thinking}).to_string(), |s| &mut s.thinking)
            && self.emit(&hub, "ferramenta", serde_json::json!({"text": tool}).to_string(), |s| &mut s.tool)
    }

    /// Publica `data` se mudou desde o último deste evento; `false` com o hub fechado.
    fn emit(&mut self, hub: &Hub, event: &str, data: String, slot: impl FnOnce(&mut Sent) -> &mut Option<String>) -> bool {
        let last = slot(&mut self.sent);
        if last.as_deref() == Some(data.as_str()) {
            return true;
        }
        if !hub.publish_own(event, &data) {
            return false;
        }
        *last = Some(data);
        true
    }
}

impl Drop for RuntimeFeed {
    fn drop(&mut self) {
        // Sem feed a lista volta ao fato do Python: o estado parado aqui mentiria.
        self.published.clear(self.owner, &self.name);
    }
}

/// Roda o feed; pânico vira diário (`on_panic`) e `state` com problema, e o hub o dá por acabado
/// (volta com o próximo assinante).
pub async fn guarded(hub: Weak<Hub>, run: impl std::future::Future<Output = ()>, on_panic: impl FnOnce(&str)) {
    use futures_util::FutureExt;
    if std::panic::AssertUnwindSafe(run).catch_unwind().await.is_ok() {
        return;
    }
    let Some(hub) = hub.upgrade() else { return };
    tracing::error!(session = hub.name.as_str(), code = "state_feed_panic", "estado: feed do runtime caiu");
    on_panic(&hub.name);
    let state = feed_problem(&hub.name, "state_feed_failed", "o estado da sessão caiu; volta ao reabrir o chat");
    if let Ok(data) = serde_json::to_string(&state) {
        hub.publish_own("state", &data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::protocol::{LiveSender, LiveState};
    use crate::side::{Binding, Out, test_ctx};
    use crate::transcript::Provider;
    use serde_json::{Value, json};
    use tokio::sync::broadcast;
    use tokio::time::Instant;

    struct Fixture { _dir: tempfile::TempDir, lease: crate::side::Lease, published: Arc<Published> }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx();
        let binding = Binding { provider: Provider::Codex, jsonl: dir.path().join("rollout-abc.jsonl"), key: "thread-1".into(), headless: true };
        let lease = ctx.hubs.acquire("s", binding, &ctx);
        Fixture { _dir: dir, lease, published: Arc::default() }
    }

    fn live(state: &str) -> LiveState {
        LiveState { public_state: json!({"session": "s", "state": state, "headless": true}), ..Default::default() }
    }

    fn spawn(f: &Fixture, rx: Option<crate::runtime::protocol::LiveReceiver>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(RuntimeFeed::new(&f.lease.hub, rx, f.published.clone()).run())
    }

    /// (instante, evento, dado) do que o hub repassou até o prazo.
    async fn collect(rx: &mut broadcast::Receiver<Out>, limit: Duration) -> Vec<(Instant, String, Value)> {
        let mut out = Vec::new();
        let _ = tokio::time::timeout(limit, async {
            while let Ok(msg) = rx.recv().await {
                if let Out::Side(f) = msg {
                    let f = String::from_utf8_lossy(&f).into_owned();
                    let event = f.lines().next().unwrap_or("").trim_start_matches("event: ").to_owned();
                    let data = serde_json::from_str(f.lines().nth(1).unwrap_or("").trim_start_matches("data: ")).unwrap_or(Value::Null);
                    out.push((Instant::now(), event, data));
                }
            }
        }).await;
        out
    }

    fn of<'a>(got: &'a [(Instant, String, Value)], event: &str) -> Vec<&'a (Instant, String, Value)> {
        got.iter().filter(|(_, e, _)| e == event).collect()
    }

    fn channel(state: Option<LiveState>) -> (LiveSender, crate::runtime::protocol::LiveReceiver) {
        tokio::sync::watch::channel(state.map(Arc::new))
    }

    #[tokio::test(start_paused = true)]
    async fn burst_coalesces_into_one_round() {
        let f = fixture();
        let mut rx = f.lease.hub.tx.subscribe();
        let (tx, live_rx) = channel(Some(live("working")));
        let _feed = spawn(&f, Some(live_rx));
        collect(&mut rx, Duration::from_millis(50)).await;
        let start = Instant::now();
        for i in 0..20 {
            tx.send_replace(Some(Arc::new(LiveState { preview: format!("parte {i}"), ..live("working") })));
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let got = collect(&mut rx, Duration::from_millis(400)).await;
        let previews = of(&got, "preview");
        assert_eq!(previews.len(), 1, "{got:?}");
        assert_eq!(previews[0].2["text"], "parte 19");
        assert_eq!(previews[0].2["vivo"], true);
        assert!(previews[0].0 - start >= COALESCE, "sai depois da janela");
        assert!(of(&got, "state").is_empty(), "o estado não mudou");
    }

    #[tokio::test(start_paused = true)]
    async fn republishes_only_on_change() {
        let f = fixture();
        let mut rx = f.lease.hub.tx.subscribe();
        let (tx, live_rx) = channel(Some(live("idle")));
        let _feed = spawn(&f, Some(live_rx));
        let first = collect(&mut rx, Duration::from_millis(50)).await;
        assert_eq!(of(&first, "state").len(), 1);
        assert_eq!(of(&first, "ask_question").iter().map(|e| e.2.clone()).collect::<Vec<_>>(), [Value::Null], "primeiro retrato leva a pergunta vazia");
        tx.send_replace(Some(Arc::new(live("idle"))));
        let same = collect(&mut rx, Duration::from_millis(300)).await;
        assert!(of(&same, "state").is_empty() && of(&same, "ask_question").is_empty(), "{same:?}");
        let question = json!({"questions": [{"question": "Qual?"}]});
        let mut asking = live("awaiting_input");
        asking.public_state["codex_question"] = question.clone();
        tx.send_replace(Some(Arc::new(asking)));
        let asked = collect(&mut rx, Duration::from_millis(300)).await;
        assert_eq!(of(&asked, "ask_question")[0].2, question);
        assert_eq!(of(&asked, "state").len(), 1);
        tx.send_replace(Some(Arc::new(live("idle"))));
        let back = collect(&mut rx, Duration::from_millis(300)).await;
        assert_eq!(of(&back, "ask_question").iter().map(|e| e.2.clone()).collect::<Vec<_>>(), [Value::Null], "de volta a vazio");
    }

    #[tokio::test(start_paused = true)]
    async fn actor_error_is_runtime_falhou() {
        let f = fixture();
        let mut rx = f.lease.hub.tx.subscribe();
        let (_tx, live_rx) = channel(Some(LiveState { error: Some(("queue_io".into(), "fila recusou: x".into())), ..live("working") }));
        let _feed = spawn(&f, Some(live_rx));
        let got = collect(&mut rx, Duration::from_millis(50)).await;
        let state = &of(&got, "state")[0].2;
        assert_eq!((state["state"].as_str(), state["problema"].as_str(), state["problema_detalhe"].as_str()),
                   (Some("working"), Some("runtime_falhou"), Some("queue_io: fila recusou: x")));
    }

    #[tokio::test(start_paused = true)]
    async fn absent_entry_is_idle() {
        // Sessão parada não é falha: reiniciar, encerrar ou reabrir não pode acender o problema.
        let f = fixture();
        let mut rx = f.lease.hub.tx.subscribe();
        let (_tx, live_rx) = channel(None);
        let _feed = spawn(&f, Some(live_rx));
        let got = collect(&mut rx, Duration::from_millis(50)).await;
        let state = &of(&got, "state")[0].2;
        assert_eq!(state["state"], "idle");
        assert!(state["problema"].is_null(), "{state}");
    }

    #[tokio::test(start_paused = true)]
    async fn no_registry_is_runtime_absent() {
        let f = fixture();
        let mut rx = f.lease.hub.tx.subscribe();
        let _feed = spawn(&f, None);
        let got = collect(&mut rx, Duration::from_millis(50)).await;
        let state = &of(&got, "state")[0].2;
        assert_eq!(state["problema"], "runtime_falhou");
        assert!(state["problema_detalhe"].as_str().unwrap().starts_with("runtime_absent:"), "{state}");
    }

    #[tokio::test(start_paused = true)]
    async fn committed_preview_goes_out_empty() {
        let f = fixture();
        let mut rx = f.lease.hub.tx.subscribe();
        let answer = "Resposta já gravada no transcript da sessão.";
        f.lease.hub.set_committed(0, crate::state::preview::norm(answer), false);
        let (_tx, live_rx) = channel(Some(LiveState { preview: answer.into(), ..live("working") }));
        let _feed = spawn(&f, Some(live_rx));
        let got = collect(&mut rx, Duration::from_millis(50)).await;
        assert_eq!(of(&got, "preview")[0].2["text"], "", "prévia igual à resposta gravada sai vazia");
    }

    #[tokio::test(start_paused = true)]
    async fn never_publishes_suggest() {
        let f = fixture();
        let mut rx = f.lease.hub.tx.subscribe();
        let (tx, live_rx) = channel(Some(live("working")));
        let _feed = spawn(&f, Some(live_rx));
        tx.send_replace(Some(Arc::new(LiveState { thinking: "pensa".into(), tool: "{}".into(), ..live("idle") })));
        let got = collect(&mut rx, Duration::from_millis(400)).await;
        assert!(of(&got, "suggest").is_empty());
        assert_eq!(of(&got, "pensamento").last().unwrap().2, json!({"text": "pensa"}));
        assert_eq!(of(&got, "ferramenta").last().unwrap().2, json!({"text": "{}"}));
    }

    fn claude_fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx();
        let binding = Binding { provider: Provider::ClaudeHeadless, jsonl: dir.path().join("sid.jsonl"), key: "sid".into(), headless: false };
        let lease = ctx.hubs.acquire("s", binding, &ctx);
        Fixture { _dir: dir, lease, published: Arc::default() }
    }

    #[tokio::test(start_paused = true)]
    async fn claude_headless_publishes_the_actor_view() {
        let f = claude_fixture();
        let mut rx = f.lease.hub.tx.subscribe();
        let question = json!({"questions": [{"question": "Qual?"}]});
        let mut asking = live("awaiting_input");
        asking.public_state["codex_question"] = question.clone();
        let (_tx, live_rx) = channel(Some(LiveState { preview: "texto".into(), thinking: "pensa".into(), tool: "{}".into(), ..asking }));
        let _feed = spawn(&f, Some(live_rx));
        let got = collect(&mut rx, Duration::from_millis(50)).await;
        assert_eq!(of(&got, "ask_question")[0].2, question);
        assert_eq!(of(&got, "state")[0].2["state"], "awaiting_input");
        assert_eq!(of(&got, "preview")[0].2["text"], "texto");
        assert_eq!(of(&got, "pensamento")[0].2, json!({"text": "pensa"}));
        assert_eq!(of(&got, "ferramenta")[0].2, json!({"text": "{}"}));
        assert!(of(&got, "suggest").is_empty());
        assert_eq!(f.published.get("s", Some("sid")).map(|e| e.state.clone()).as_deref(), Some("awaiting_input"));
    }

    #[tokio::test(start_paused = true)]
    async fn claude_turn_reopened_mid_stream_is_working_until_result() {
        // Rust reiniciado no meio de um turno: a vista do motor reaberto fica `working` até o `result`.
        use crate::runtime::claude::ClaudeEngine;
        use crate::runtime::protocol::{ClockSample, EngineInput};
        let clock = |s: f64| ClockSample { monotonic_s: s, epoch_s: 1_800_000_000.0 + s };
        let mut engine = ClaudeEngine::new(json!({"name": "s", "session_id": "sid", "initialized": true}), 1, clock(10.0));
        let view = |engine: &ClaudeEngine| LiveState { public_state: engine.view()["public_state"].clone(), ..Default::default() };
        let f = claude_fixture();
        let mut rx = f.lease.hub.tx.subscribe();
        let (tx, live_rx) = channel(Some(view(&engine)));
        let _feed = spawn(&f, Some(live_rx));
        assert_eq!(of(&collect(&mut rx, Duration::from_millis(50)).await, "state")[0].2["state"], "idle");
        let delta = json!({"type": "stream_event", "event": {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "meio"}}});
        engine.apply(EngineInput::Line(delta), clock(11.0)).unwrap();
        tx.send_replace(Some(Arc::new(view(&engine))));
        assert_eq!(of(&collect(&mut rx, Duration::from_millis(300)).await, "state")[0].2["state"], "working");
        engine.apply(EngineInput::Line(json!({"type": "result", "subtype": "success"})), clock(20.0)).unwrap();
        tx.send_replace(Some(Arc::new(view(&engine))));
        assert_eq!(of(&collect(&mut rx, Duration::from_millis(300)).await, "state").last().unwrap().2["state"], "idle");
    }

    #[tokio::test(start_paused = true)]
    async fn closed_hub_ends_feed() {
        let f = fixture();
        let (tx, live_rx) = channel(Some(live("working")));
        let feed = spawn(&f, Some(live_rx));
        tokio::time::sleep(Duration::from_millis(50)).await;
        crate::side::close_hub(&f.lease.hub);
        tx.send_replace(Some(Arc::new(live("idle"))));
        tokio::time::timeout(Duration::from_secs(1), feed).await.expect("hub fechado: o feed acaba").unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn publishes_to_list_with_rollout_stem() {
        let f = fixture();
        let (tx, live_rx) = channel(Some(live("working")));
        let feed = spawn(&f, Some(live_rx));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(f.published.get("s", Some("rollout-abc")).map(|e| e.state.clone()).as_deref(), Some("working"));
        tx.send_replace(Some(Arc::new(live("idle"))));
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(f.published.get("s", Some("rollout-abc")).map(|e| e.state.clone()).as_deref(), Some("idle"));
        feed.abort();
        let _ = feed.await;
        assert!(f.published.get("s", Some("rollout-abc")).is_none(), "feed que acaba sai da lista");
    }
}
