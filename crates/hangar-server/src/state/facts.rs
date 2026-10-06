//! Fatos do estado que continuam do Python (`state_facts.py`): o que o plugin anunciou, troca de
//! conta em curso e operação de permissão controlada. Chegam por empurrão (`state.facts` na ponte
//! da lista) só para sessões cujo retrato o Rust leu (`GET /internal/sessions/{name}/state-facts`),
//! o que registra o interesse no Python por 30 s. As validades vêm como idade em ms e são aplicadas
//! aqui com o relógio do Rust. Também o cliente dos serviços que o `Monitor` pede ao Python.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use http_body_util::BodyExt;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use serde_json::{Value, json};

/// `vivo` no Python: long-poll aberto ou batida há menos disto (`ESPERA_S + 10`).
pub const ALIVE: Duration = Duration::from_secs(35);
/// Estado anunciado pelo plugin sem long-poll aberto (`VALIDADE_ESTADO_S`).
pub const PLUGIN_STATE: Duration = Duration::from_secs(90);
/// Pergunta segurada depois do último `visto` (`pergunta_pendente`).
pub const QUESTION: Duration = Duration::from_secs(35);
/// Releitura do retrato: abaixo dos 30 s em que o interesse vence no Python.
pub const SNAPSHOT_REFRESH: Duration = Duration::from_secs(25);
pub const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(1);
pub const SERVICE_TIMEOUT: Duration = Duration::from_secs(2);
/// O drain pode abrir o executor (`prepare_session`) antes de entregar.
pub const DELIVER_TIMEOUT: Duration = Duration::from_secs(30);
/// `problema` do estado quando o retrato não veio: fica o último valor, nunca estado inventado.
pub const UNAVAILABLE: &str = "state_facts_unavailable";
const MAX_BODY: usize = 1 << 20;

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct PluginState { pub state: String, pub reason: Option<String>, pub age_ms: u64 }

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Question {
    pub id: String,
    pub questions: Value,
    pub tool: Option<String>,
    pub resumo: Option<String>,
    pub seen_age_ms: u64,
}

/// Todos os campos são obrigatórios: chave que falta é contrato quebrado, nunca vazio.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct StateFacts {
    pub seq: u64,
    pub plugin_state: Option<PluginState>,
    pub waiter_open: bool,
    pub heartbeat_age_ms: Option<u64>,
    pub question: Option<Question>,
    pub suggestion: String,
    /// Largura da conversa no pane quando há painel ancorado (corte da prévia em `+5`).
    pub body_columns: Option<u32>,
    pub band_anchor: Option<String>,
    /// Quanto faltava da janela de troca terminal ⇄ sem terminal quando o Python enviou.
    pub in_transfer_ms: u64,
    pub transfer_active: bool,
    pub permission_op: bool,
}

/// Fatos e o instante em que chegaram aqui: as idades contam a partir dele.
#[derive(Clone, Debug)]
pub struct Received { pub facts: Arc<StateFacts>, pub at: Instant }

impl Received {
    /// Idade agora de um valor que tinha `age_ms` quando chegou.
    fn age(&self, age_ms: u64, now: Instant) -> Duration {
        Duration::from_millis(age_ms).saturating_add(now.saturating_duration_since(self.at))
    }

    /// `plugin_bridge.vivo`.
    pub fn alive(&self, now: Instant) -> bool {
        self.facts.waiter_open || self.facts.heartbeat_age_ms.is_some_and(|a| self.age(a, now) < ALIVE)
    }

    /// `plugin_bridge.estado_recente`: sem prazo com o long-poll aberto.
    pub fn plugin_state(&self, now: Instant) -> Option<&PluginState> {
        self.facts.plugin_state.as_ref().filter(|p| self.facts.waiter_open || self.age(p.age_ms, now) <= PLUGIN_STATE)
    }

    /// `plugin_bridge.pergunta_pendente`.
    pub fn question(&self, now: Instant) -> Option<&Question> {
        self.facts.question.as_ref().filter(|q| self.age(q.seen_age_ms, now) <= QUESTION)
    }

    /// `sessions.em_troca`.
    pub fn in_transfer(&self, now: Instant) -> bool {
        self.facts.transfer_active || Duration::from_millis(self.facts.in_transfer_ms) > now.saturating_duration_since(self.at)
    }
}

/// Resultado de um empurrão.
#[derive(Debug, PartialEq, Eq)]
pub enum Push {
    /// `gap`: faltou sequência no meio (envio que falhou no Python): o retrato tem que ser relido.
    Accepted { gap: bool },
    /// Sequência já vista: chegou fora de ordem ou depois de um retrato mais novo.
    Dropped,
    /// Ninguém observa a sessão aqui (empurrão atrasado depois do `forget`): não cria entrada.
    Unwatched,
}

#[derive(Default)]
struct Entry {
    last: Option<Received>,
    wake: Arc<tokio::sync::Notify>,
    /// Faltou sequência desde o último retrato: quem observa relê o retrato.
    gap: bool,
}

/// Último fato de cada sessão observada. Entrada só nasce por `watch` (o `Monitor` que nasce) e sai
/// por `forget`: o mapa tem o tamanho dos `Monitor`s vivos.
#[derive(Default)]
pub struct FactsStore { entries: Mutex<HashMap<String, Entry>> }

impl FactsStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Passa a observar a sessão e devolve o aviso de empurrão aceito. Quem espera arma o
    /// `notified()` antes de ler `get`: `notify_waiters` não guarda o aviso para depois.
    pub fn watch(&self, name: &str) -> Arc<tokio::sync::Notify> {
        self.lock().entry(name.to_owned()).or_default().wake.clone()
    }

    /// Empurrão do Python. Aceito acorda quem observa a sessão (idade 0, como `esperar_evento`).
    pub fn push(&self, name: &str, facts: StateFacts, at: Instant) -> Push {
        let mut entries = self.lock();
        let Some(entry) = entries.get_mut(name) else { return Push::Unwatched };
        let last = entry.last.as_ref().map(|r| r.facts.seq);
        if last.is_some_and(|l| facts.seq <= l) {
            return Push::Dropped;
        }
        let gap = last.is_some_and(|l| facts.seq > l.saturating_add(1));
        entry.gap |= gap;
        entry.last = Some(Received { facts: Arc::new(facts), at });
        entry.wake.notify_waiters();
        Push::Accepted { gap }
    }

    /// Retrato lido agora. Empurrão mais novo que chegou durante a leitura vence; com sequência
    /// igual ou maior, o retrato vale por inteiro e fecha o pulo. Não acorda: quem o leu é quem
    /// observa.
    pub fn snapshot(&self, name: &str, facts: StateFacts, at: Instant) {
        let mut entries = self.lock();
        let Some(entry) = entries.get_mut(name) else { return };
        if entry.last.as_ref().is_some_and(|r| r.facts.seq > facts.seq) {
            return;
        }
        entry.gap = false;
        entry.last = Some(Received { facts: Arc::new(facts), at });
    }

    pub fn get(&self, name: &str) -> Option<Received> { self.lock().get(name).and_then(|e| e.last.clone()) }

    /// Faltou sequência desde o último retrato.
    pub fn needs_snapshot(&self, name: &str) -> bool { self.lock().get(name).is_some_and(|e| e.gap) }

    pub fn forget(&self, name: &str) { self.lock().remove(name); }

    /// `forget` de quem observava com `wake`, só se ninguém mais observa: o `Monitor` que acaba
    /// depois de outro da mesma sessão nascer não apaga a entrada do novo.
    pub fn forget_watcher(&self, name: &str, wake: &Arc<tokio::sync::Notify>) {
        let mut entries = self.lock();
        // Uma referência no mapa e a de quem chama.
        if entries.get(name).is_some_and(|e| Arc::ptr_eq(&e.wake, wake) && Arc::strong_count(wake) == 2) {
            entries.remove(name);
        }
    }
}

/// Resposta de `session.dead`.
#[derive(Debug, PartialEq, Eq)]
pub enum Dead { Ok, InTransfer }

/// Retrato e serviços pedidos ao Python; `Err` é o código que vai ao diário e ao `problema`,
/// nunca o corpo nem a mensagem do serde.
#[derive(Clone)]
pub struct StateFactsClient { upstream: SocketAddr, secret: String, http: crate::proxy::HttpClient }

impl StateFactsClient {
    pub fn new(upstream: SocketAddr, secret: String) -> Self {
        Self { upstream, secret, http: crate::proxy::client() }
    }

    fn url(&self, name: &str, tail: &str) -> String {
        format!("http://{}/internal/sessions/{}/{tail}", self.upstream, utf8_percent_encode(name, NON_ALPHANUMERIC))
    }

    async fn call(&self, req: axum::http::Request<Body>, timeout: Duration, prefix: &str) -> Result<Vec<u8>, String> {
        let work = async {
            let resp = self.http.request(req).await
                .map_err(|e| format!("{prefix}_{}", if e.is_connect() { "connect" } else { "unreachable" }))?;
            if !resp.status().is_success() {
                return Err(format!("{prefix}_status:{}", resp.status().as_u16()));
            }
            let body = http_body_util::Limited::new(resp.into_body(), MAX_BODY).collect().await
                .map_err(|_| format!("{prefix}_body"))?;
            Ok(body.to_bytes().to_vec())
        };
        tokio::time::timeout(timeout, work).await.map_err(|_| format!("{prefix}_timeout"))?
    }

    /// Lê o retrato e renova o interesse da sessão no Python.
    pub async fn snapshot(&self, name: &str) -> Result<StateFacts, String> {
        let req = axum::http::Request::get(self.url(name, "state-facts")).header("x-hangar-internal", &self.secret)
            .body(Body::empty()).map_err(|_| "state_facts_request".to_owned())?;
        let bytes = self.call(req, SNAPSHOT_TIMEOUT, "state_facts").await?;
        serde_json::from_slice(&bytes).map_err(|e| format!("state_facts_invalid:{:?}", e.classify()))
    }

    async fn service(&self, name: &str, kind: &str, payload: Value, timeout: Duration) -> Result<Value, String> {
        let body = json!({"kind": kind, "payload": payload}).to_string();
        let req = axum::http::Request::post(self.url(name, "state-service")).header("x-hangar-internal", &self.secret)
            .header("content-type", "application/json").body(Body::from(body))
            .map_err(|_| "state_service_request".to_owned())?;
        let bytes = self.call(req, timeout, "state_service").await?;
        let v: Value = serde_json::from_slice(&bytes).map_err(|e| format!("state_service_invalid:{:?}", e.classify()))?;
        match v["ok"].as_bool() {
            Some(true) => Ok(v["data"].clone()),
            // O tipo vai ao `problema` e ao diário: só identificador curto, nunca texto livre.
            Some(false) => Err(format!("state_service_failed:{}", v["error_type"].as_str()
                .filter(|t| !t.is_empty() && t.len() <= 40 && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or("unknown"))),
            None => Err("state_service_invalid:ok".to_owned()),
        }
    }

    /// `permission.observe`: a memória do modo é do Python, por session-id; devolve
    /// `(modo, previous_non_plan)`.
    pub async fn observe_permission(&self, name: &str, sid: &str, mode: &str) -> Result<(String, String), String> {
        let v = self.service(name, "permission.observe", json!({"sid": sid, "mode": mode}), SERVICE_TIMEOUT).await?;
        match (v["mode"].as_str(), v["previous_non_plan"].as_str()) {
            (Some(m), Some(p)) => Ok((m.to_owned(), p.to_owned())),
            _ => Err("state_service_invalid:permission".to_owned()),
        }
    }

    /// `session.dead`: o Python confere a troca na hora e só com `Ok` esquece o plugin e o quadro.
    pub async fn dead(&self, name: &str) -> Result<Dead, String> {
        let v = self.service(name, "session.dead", json!({}), SERVICE_TIMEOUT).await?;
        match v["result"].as_str() {
            Some("ok") => Ok(Dead::Ok),
            Some("em_troca") => Ok(Dead::InTransfer),
            _ => Err("state_service_invalid:dead".to_owned()),
        }
    }

    /// `session.deliverable`: o `adapter.drain` do Python; devolve quantas saíram.
    pub async fn deliverable(&self, name: &str) -> Result<u64, String> {
        let v = self.service(name, "session.deliverable", json!({}), DELIVER_TIMEOUT).await?;
        v["sent"].as_u64().ok_or_else(|| "state_service_invalid:deliverable".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn facts(seq: u64) -> StateFacts {
        StateFacts { seq, plugin_state: Some(PluginState { state: "working".into(), reason: None, age_ms: 80_000 }),
            waiter_open: false, heartbeat_age_ms: Some(30_000),
            question: Some(Question { id: "q".into(), questions: Value::Null, tool: None, resumo: None, seen_age_ms: 30_000 }),
            suggestion: String::new(), body_columns: None, band_anchor: None, in_transfer_ms: 4_000,
            transfer_active: false, permission_op: false }
    }

    #[test]
    fn validity_with_own_clock() {
        let t0 = Instant::now();
        let r = Received { facts: Arc::new(facts(1)), at: t0 };
        assert!(r.alive(t0) && r.plugin_state(t0).is_some() && r.question(t0).is_some() && r.in_transfer(t0));
        let later = t0 + Duration::from_secs(6);
        assert!(!r.alive(later), "batida de 30 s + 6 s passa dos 35 s");
        assert!(r.question(later).is_none());
        assert!(!r.in_transfer(later));
        assert!(r.plugin_state(later).is_some(), "80 s + 6 s ainda dentro dos 90 s");
        assert!(r.plugin_state(t0 + Duration::from_secs(11)).is_none());
        let open = Received { facts: Arc::new(StateFacts { waiter_open: true, ..facts(1) }), at: t0 };
        let hour = t0 + Duration::from_secs(3600);
        assert!(open.alive(hour) && open.plugin_state(hour).is_some(), "long-poll aberto não tem prazo");
    }

    #[test]
    fn older_sequence_dropped() {
        let store = FactsStore::default();
        let now = Instant::now();
        assert_eq!(store.push("s", facts(1), now), Push::Unwatched);
        store.watch("s");
        assert_eq!(store.push("s", facts(2), now), Push::Accepted { gap: false });
        assert_eq!(store.push("s", facts(1), now), Push::Dropped);
        assert_eq!(store.push("s", facts(2), now), Push::Dropped, "sequência já vista");
        assert_eq!(store.push("s", facts(4), now), Push::Accepted { gap: true });
        assert_eq!(store.get("s").unwrap().facts.seq, 4);
        assert!(store.needs_snapshot("s"));
        store.snapshot("s", facts(3), now);
        assert_eq!(store.get("s").unwrap().facts.seq, 4, "retrato mais velho que o empurrão não vence");
        store.snapshot("s", facts(4), now);
        assert!(!store.needs_snapshot("s"));
        assert_eq!(store.push("s", facts(5), now), Push::Accepted { gap: false });
    }

    #[test]
    fn late_forget_keeps_the_new_watcher() {
        let store = FactsStore::default();
        let old = store.watch("s");
        let new = store.watch("s");
        store.forget_watcher("s", &old);
        assert_eq!(store.push("s", facts(1), Instant::now()), Push::Accepted { gap: false }, "o novo ainda observa");
        drop(new);
        store.forget_watcher("s", &old);
        assert_eq!(store.push("s", facts(2), Instant::now()), Push::Unwatched);
    }

    #[tokio::test]
    async fn push_wakes_the_observer() {
        let store = Arc::new(FactsStore::default());
        let wake = store.watch("s");
        let waiting = tokio::spawn(async move { wake.notified().await });
        tokio::task::yield_now().await;
        store.push("s", facts(1), Instant::now());
        tokio::time::timeout(Duration::from_secs(2), waiting).await.expect("não acordou").unwrap();
    }

    /// Python de mentira que responde sempre o mesmo status e corpo.
    async fn python(status: u16, body: &'static str) -> SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf).await;
                let _ = s.write_all(format!("HTTP/1.1 {status} X\r\ncontent-length: {}\r\n\r\n{body}", body.len()).as_bytes()).await;
            }
        });
        addr
    }

    #[tokio::test]
    async fn snapshot_failure_is_error() {
        let down = StateFactsClient::new(python(503, "x").await, "s".into());
        assert_eq!(down.snapshot("s").await.unwrap_err(), "state_facts_status:503");
        let partial = StateFactsClient::new(python(200, r#"{"seq":1}"#).await, "s".into());
        assert!(partial.snapshot("s").await.unwrap_err().starts_with("state_facts_invalid:"), "campo faltando não vira vazio");
        let refused = StateFactsClient::new("127.0.0.1:9".parse().unwrap(), "s".into());
        assert!(refused.snapshot("s").await.is_err());
        let failed = StateFactsClient::new(python(200, r#"{"ok":false,"error_type":"ValueError"}"#).await, "s".into());
        assert_eq!(failed.dead("s").await.unwrap_err(), "state_service_failed:ValueError");
    }
}
