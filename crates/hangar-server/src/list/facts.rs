//! Fatos da lista que continuam do Python (`POST /internal/list/facts`): estado dos provedores não
//! migrados, conta de Kimi/Pi/omp, transferências, orquestrações, acesso, navegador, terminais de
//! atalho e problemas do runtime. Pergunta só quando a entrada muda ou o último ficou velho, com
//! prazo de 1 s; sem resposta, fica o último valor, todas as linhas levam
//! `problema = list_facts_unavailable` e o motivo vai ao diário. A lista nunca passa ao Python.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::net::SocketAddr;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use hangar_api::session::SessionRow;
use http_body_util::BodyExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Prazo da resposta, e idade máxima da última para a mesma entrada.
pub const FACTS_TIMEOUT: Duration = Duration::from_secs(1);
pub const FACTS_TTL: Duration = Duration::from_secs(1);
const MAX_BODY: usize = 8 << 20;
pub const UNAVAILABLE: &str = "list_facts_unavailable";
pub const ORQ_UNAVAILABLE: &str = "list_orq_unavailable";
/// Provedores cujo estado o Python ainda calcula (`registry.list_with_state`).
pub const OTHERS: [&str; 4] = ["codex", "pi", "omp", "kimi"];

/// Estado de uma linha não migrada, com o código de hoje do Python.
#[derive(Debug, Deserialize)]
pub struct RowState {
    pub state: String,
    pub label: Option<String>,
    pub question: Option<String>,
    pub options: Option<Vec<String>>,
    pub problema: Option<String>,
    pub status_line: Option<String>,
    #[serde(default)]
    pub pending_questions: u32,
    #[serde(default)]
    pub startup_steps: Vec<String>,
    pub last_activity: Option<f64>,
    #[serde(default)]
    pub limited: bool,
    pub limit_reset: Option<String>,
    #[serde(default)]
    pub stalled: bool,
    /// Só Kimi, Pi e omp: a descoberta não sabe a credencial.
    #[serde(default)]
    pub conta: Option<String>,
    /// Só Codex: o snapshot ao vivo vence o sidecar.
    #[serde(default)]
    pub codex_service_tier: Option<String>,
}

/// Todos os campos são obrigatórios na resposta: chave que falta é contrato quebrado, nunca vazio.
#[derive(Debug, Deserialize)]
pub struct ListFacts {
    pub states: HashMap<String, RowState>,
    /// Linhas inteiras que a transferência substitui (pelo nome) ou cria.
    pub overrides: Vec<SessionRow>,
    /// Transferências em curso: nem classificadas nem decoradas.
    pub frozen: HashSet<String>,
    pub orq: Vec<SessionRow>,
    /// Leitura das orquestrações falhou: `orq` vem vazio e vale a da última resposta boa, marcada.
    #[serde(default)]
    pub orq_error: Option<String>,
    pub shared: HashSet<String>,
    pub owners: HashMap<String, String>,
    /// Escondidas do dono (sessão de convidado que ele não vê).
    pub hidden: HashSet<String>,
    pub problems: BTreeMap<String, String>,
    /// Pergunta que o hook do plugin segura agora, por nome (`plugin_bridge.pergunta_pendente`):
    /// a permissão segurada para o app não desenha cartão no pane.
    pub held: BTreeMap<String, Value>,
    pub stall_seconds: f64,
    /// Pedidos de navegador vivos, `{nome: {url, ts}}`, já vencidos pelo Python.
    pub nav: Value,
    /// JSON dos terminais de atalho, a última leitura boa; `None` antes da primeira.
    pub shortcuts: Option<String>,
    /// Só no pedido da sombra: a assinatura de cada linha da lista que o Python serviu, por nome.
    /// `None` sem lista recente dele (ninguém com ela aberta) ou fora da sombra.
    pub shadow: Option<super::shadow::PySigs>,
    /// Nenhuma resposta boa ainda: acesso, escondidas, transferências e orquestrações são
    /// desconhecidos, não vazios. Todas as linhas levam o problema, e o hub não serve a lista ao dono.
    #[serde(skip)]
    pub unknown: bool,
}

impl Default for ListFacts {
    fn default() -> Self {
        Self { states: HashMap::new(), overrides: Vec::new(), frozen: HashSet::new(), orq: Vec::new(),
            orq_error: None, shared: HashSet::new(), owners: HashMap::new(), hidden: HashSet::new(), problems: BTreeMap::new(), held: BTreeMap::new(),
            stall_seconds: 300.0, nav: Value::Null, shortcuts: None, shadow: None, unknown: true }
    }
}

#[derive(Serialize)]
struct Request<'a> { rows: &'a [SessionRow], owner_clients: u32, pane_pids: &'a BTreeMap<String, u32>, shadow: bool }

pub struct Fetched {
    pub facts: Arc<ListFacts>,
    /// `false`: o Python não respondeu desta vez; `facts` é o último bom (ou vazio).
    pub ok: bool,
}

/// `ok = false`: a última pergunta falhou; até `at + FACTS_TTL` o tique não espera o prazo de novo.
struct Last { key: u64, at: Instant, ok: bool, facts: Arc<ListFacts> }

pub struct FactsClient {
    upstream: SocketAddr,
    secret: String,
    http: crate::proxy::HttpClient,
    /// O diário que o dono exporta: falha da lista que só chegasse ao `tracing` sumiria dele.
    pub diag: crate::diag::DiagClient,
    /// Trava assíncrona = uma pergunta por vez; quem chega no meio reaproveita a resposta.
    last: tokio::sync::Mutex<Option<Last>>,
}

impl FactsClient {
    pub fn new(upstream: SocketAddr, secret: String) -> Self {
        let diag = crate::diag::DiagClient::new(upstream, secret.clone());
        Self { upstream, secret, http: crate::proxy::client(), diag, last: tokio::sync::Mutex::new(None) }
    }

    /// Outro cliente para o mesmo Python, com fila e último valor próprios: a sombra não espera a
    /// produção de verdade nem lhe empresta uma falha.
    pub fn sibling(&self) -> Self {
        Self { upstream: self.upstream, secret: self.secret.clone(), http: self.http.clone(), diag: self.diag.clone(),
            last: tokio::sync::Mutex::new(None) }
    }

    /// `shadow`: pedido da rodada em sombra; o Python não mexe na presença do app e manda a
    /// assinatura da lista dele.
    pub async fn fetch(&self, rows: &[SessionRow], owner_clients: u32, pane_pids: &BTreeMap<String, u32>, shadow: bool) -> Fetched {
        let mut last = self.last.lock().await;
        let body = match serde_json::to_vec(&Request { rows, owner_clients, pane_pids, shadow }) {
            Ok(b) => b,
            Err(_) => return self.failed(&mut last, "list_facts_request".into()),
        };
        let mut h = std::collections::hash_map::DefaultHasher::new();
        body.hash(&mut h);
        let key = h.finish();
        if let Some(l) = last.as_ref().filter(|l| l.at.elapsed() < FACTS_TTL && (l.key == key || !l.ok)) {
            return Fetched { facts: l.facts.clone(), ok: l.ok };
        }
        match self.ask(body).await {
            Ok(mut facts) => {
                if let Some(code) = &facts.orq_error {
                    self.diag.report("rust.list_orq_unavailable", "", code, "orquestrações ilegíveis; ficam as da última resposta boa");
                    if let Some(prev) = last.as_ref().map(|l| &l.facts) {
                        keep_orq(&mut facts, prev);
                    }
                }
                let facts = Arc::new(facts);
                *last = Some(Last { key, at: Instant::now(), ok: true, facts: facts.clone() });
                Fetched { facts, ok: true }
            }
            Err(code) => self.failed(&mut last, code),
        }
    }

    fn failed(&self, last: &mut Option<Last>, code: String) -> Fetched {
        if crate::warn_limit::allow(None, &code) {
            tracing::warn!(code, "lista: fatos do Python indisponíveis; fica o último valor");
        }
        self.diag.report("rust.list_facts_unavailable", "", &code, "fatos do Python indisponíveis; fica o último valor");
        let facts = last.as_ref().map_or_else(Arc::default, |l| l.facts.clone());
        *last = Some(Last { key: 0, at: Instant::now(), ok: false, facts: facts.clone() });
        Fetched { facts, ok: false }
    }

    /// `Err` é o código que vai ao diário: motivo e status, nunca o corpo nem a mensagem do serde,
    /// que podem ecoar valores.
    async fn ask(&self, body: Vec<u8>) -> Result<ListFacts, String> {
        let req = axum::http::Request::post(format!("http://{}/internal/list/facts", self.upstream))
            .header("x-hangar-internal", &self.secret).header("content-type", "application/json")
            .body(Body::from(body)).map_err(|_| "list_facts_request".to_owned())?;
        let work = async {
            let resp = self.http.request(req).await.map_err(|e| unreachable_code("list_facts", &e))?;
            if !resp.status().is_success() {
                return Err(format!("list_facts_status:{}", resp.status().as_u16()));
            }
            let bytes = http_body_util::Limited::new(resp.into_body(), MAX_BODY).collect().await
                .map_err(|_| "list_facts_body".to_owned())?.to_bytes();
            serde_json::from_slice(&bytes).map_err(|e| format!("list_facts_invalid:{:?}", e.classify()))
        };
        tokio::time::timeout(FACTS_TIMEOUT, work).await.map_err(|_| "list_facts_timeout".to_owned())?
    }

    /// `hooks.demote_awaiting` no Python, sem segurar a produção: quem é dono do mapa de marcadores
    /// é ele. Falhou, o próximo tique captura de novo e pede outra vez.
    pub fn demote(&self, sids: Vec<String>) {
        let body = serde_json::json!({"sids": sids}).to_string();
        let (http, url, secret) = (self.http.clone(), format!("http://{}/internal/list/demote", self.upstream), self.secret.clone());
        let diag = self.diag.clone();
        tokio::spawn(async move {
            let code = match axum::http::Request::post(url).header("x-hangar-internal", secret)
                .header("content-type", "application/json").body(Body::from(body)) {
                Err(_) => "list_demote_request".to_owned(),
                Ok(req) => match tokio::time::timeout(Duration::from_secs(5), http.request(req)).await {
                    Ok(Ok(r)) if r.status().is_success() => return,
                    Ok(Ok(r)) => format!("list_demote_status:{}", r.status().as_u16()),
                    Ok(Err(e)) => unreachable_code("list_demote", &e),
                    Err(_) => "list_demote_timeout".to_owned(),
                },
            };
            if crate::warn_limit::allow(None, &code) {
                tracing::warn!(code, "lista: rebaixamento de awaiting não chegou ao Python");
            }
            diag.report("rust.list_demote_failed", "", &code, "rebaixamento de awaiting não chegou ao Python");
        });
    }
}

/// As linhas `orq` da resposta boa anterior, com o acesso que valia para elas: o Python desta vez
/// não as conhecia e não decidiu escondida, compartilhada nem dono.
fn keep_orq(facts: &mut ListFacts, prev: &ListFacts) {
    facts.orq = prev.orq.clone();
    for row in &mut facts.orq {
        row.problema = Some(ORQ_UNAVAILABLE.into());
        if prev.hidden.contains(&row.name) { facts.hidden.insert(row.name.clone()); }
        if prev.shared.contains(&row.name) { facts.shared.insert(row.name.clone()); }
        if let Some(o) = prev.owners.get(&row.name) { facts.owners.insert(row.name.clone(), o.clone()); }
    }
}

fn unreachable_code(prefix: &str, e: &hyper_util::client::legacy::Error) -> String {
    format!("{prefix}_{}", if e.is_connect() { "connect" } else { "unreachable" })
}

/// Fatos velhos ou desconhecidos: acesso, escondidas e transferências de toda linha saíram deles, então
/// toda linha leva o aviso. Problema mais preciso que a linha já tenha fica.
pub fn mark_stale(rows: &mut [SessionRow], facts: &ListFacts, ok: bool) {
    if ok && !facts.unknown {
        return;
    }
    for row in rows {
        row.problema.get_or_insert_with(|| UNAVAILABLE.into());
    }
}

/// Falha vista em código síncrono (arquivo lido dentro do `spawn_blocking`) a caminho do diário:
/// a produção esvazia e manda pelo `diag`. Uma por minuto por (sessão, código); fila com teto.
pub struct Note { pub event: &'static str, pub session: String, pub code: String, pub reason: &'static str }

#[cfg(not(test))]
const MAX_NOTES: usize = 64;
// Nos testes ninguém esvazia a fila.
#[cfg(test)]
const MAX_NOTES: usize = 100_000;
static NOTES: LazyLock<Mutex<Vec<Note>>> = LazyLock::new(Default::default);

pub fn note(event: &'static str, session: &str, code: String, reason: &'static str) {
    let mut q = NOTES.lock().unwrap_or_else(|e| e.into_inner());
    // Cheia (ninguém produzindo para esvaziar): o aviso vai ao log, e o teto por minuto não é gasto.
    if q.len() >= MAX_NOTES {
        if crate::warn_limit::allow(None, "list_notes_full") {
            tracing::warn!(event, code, reason, "fila do diário da lista cheia; esta falha fica só no log");
        }
        return;
    }
    if crate::warn_limit::allow(Some(session), &format!("note:{event}:{code}")) {
        q.push(Note { event, session: session.to_owned(), code, reason });
    }
}

pub fn take_notes() -> Vec<Note> { std::mem::take(&mut *NOTES.lock().unwrap_or_else(|e| e.into_inner())) }

#[cfg(test)]
pub fn notes_for(session: &str) -> Vec<String> {
    NOTES.lock().unwrap().iter().filter(|n| n.session == session).map(|n| n.code.clone()).collect()
}

/// Fatos sobre as linhas descobertas. Devolve as que vão à classificação e à decoração e as postas
/// de lado (transferência em curso, orquestração), que saem como o Python as deu, no fim da lista.
pub fn apply(rows: Vec<SessionRow>, facts: &ListFacts, ok: bool) -> (Vec<SessionRow>, Vec<SessionRow>) {
    let mut overrides: HashMap<&str, &SessionRow> = facts.overrides.iter().map(|r| (r.name.as_str(), r)).collect();
    let mut work = Vec::with_capacity(rows.len());
    let mut aside = Vec::new();
    for row in rows {
        let row = overrides.remove(row.name.as_str()).cloned().unwrap_or(row);
        if facts.frozen.contains(&row.name) { aside.push(row) } else { work.push(row) }
    }
    // Linha que a transferência cria (a sessão de origem já saiu da descoberta).
    for row in facts.overrides.iter().filter(|r| overrides.contains_key(r.name.as_str())) {
        if facts.frozen.contains(&row.name) { aside.push(row.clone()) } else { work.push(row.clone()) }
    }
    aside.extend(facts.orq.iter().cloned());
    for row in work.iter_mut().filter(|r| OTHERS.contains(&r.provider.as_str())) {
        if let Some(s) = facts.states.get(&row.name) {
            row.state.clone_from(&s.state);
            row.label.clone_from(&s.label);
            row.question.clone_from(&s.question);
            row.options.clone_from(&s.options);
            row.problema.clone_from(&s.problema);
            row.status_line.clone_from(&s.status_line);
            row.pending_questions = s.pending_questions;
            row.startup_steps.clone_from(&s.startup_steps);
            row.last_activity = s.last_activity;
            row.limited = s.limited;
            row.limit_reset.clone_from(&s.limit_reset);
            row.stalled = s.stalled;
            if row.provider == "codex" {
                row.codex_service_tier.clone_from(&s.codex_service_tier);
            } else {
                row.conta.clone_from(&s.conta);
            }
        }
        if !ok { row.problema = Some(UNAVAILABLE.into()); }
    }
    if !ok {
        for row in aside.iter_mut() { row.problema = Some(UNAVAILABLE.into()); }
    }
    if facts.unknown {
        for row in work.iter_mut() { row.problema = Some(UNAVAILABLE.into()); }
    }
    for row in work.iter_mut().chain(aside.iter_mut()) {
        row.shared = facts.shared.contains(&row.name);
        row.owner = facts.owners.get(&row.name).cloned();
    }
    (work, aside)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    type Seen = Arc<Mutex<Vec<(String, Value)>>>;

    /// Python de mentira: `answer(caminho, n-ésimo pedido dos fatos)` dá (status, corpo); guarda caminho e corpo.
    async fn python(answer: fn(&str, usize) -> (u16, String)) -> (SocketAddr, Seen) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen: Seen = Arc::default();
        let log = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else { return };
                let log = log.clone();
                tokio::spawn(async move {
                    let mut reader = BufReader::new(stream);
                    loop {
                        let (mut first, mut length) = (String::new(), 0usize);
                        if reader.read_line(&mut first).await.unwrap_or(0) == 0 { return; }
                        loop {
                            let mut line = String::new();
                            if reader.read_line(&mut line).await.unwrap_or(0) == 0 { return; }
                            if line == "\r\n" { break; }
                            if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") { length = v.trim().parse().unwrap(); }
                        }
                        let mut body = vec![0; length];
                        reader.read_exact(&mut body).await.unwrap();
                        let path = first.split_whitespace().nth(1).unwrap().to_owned();
                        let n = {
                            let mut l = log.lock().unwrap();
                            l.push((path.clone(), serde_json::from_slice(&body).unwrap_or(Value::Null)));
                            l.iter().filter(|(p, _)| p == &path).count()
                        };
                        let (status, out) = answer(&path, n);
                        let head = format!("HTTP/1.1 {status} X\r\ncontent-length: {}\r\n\r\n", out.len());
                        reader.get_mut().write_all(format!("{head}{out}").as_bytes()).await.unwrap();
                    }
                });
            }
        });
        (addr, seen)
    }

    async fn diary(seen: &Seen, event: &str) -> Value {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let hit = seen.lock().unwrap().iter().find(|(p, b)| p == "/internal/diag" && b["evento"] == event).map(|(_, b)| b.clone());
                if let Some(b) = hit { return b; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("diário não recebeu")
    }

    fn facts_json(orq: Value, orq_error: Value) -> String {
        json!({"states": {}, "overrides": [], "frozen": [], "orq": orq, "orq_error": orq_error, "shared": [], "owners": {},
            "hidden": [], "problems": {}, "held": {}, "stall_seconds": 300.0, "nav": null, "shortcuts": null, "shadow": null}).to_string()
    }

    #[tokio::test]
    async fn failure_reaches_the_diary_with_the_status() {
        let (addr, seen) = python(|path, _| if path == "/internal/diag" { (200, "{}".into()) } else { (503, "x".into()) }).await;
        let client = FactsClient::new(addr, "s".into());
        let got = client.fetch(&[], 0, &BTreeMap::new(), false).await;
        assert!(!got.ok);
        assert_eq!(diary(&seen, "rust.list_facts_unavailable").await["codigo"], "list_facts_status:503");
        client.demote(vec!["sid".into()]);
        assert_eq!(diary(&seen, "rust.list_demote_failed").await["codigo"], "list_demote_status:503");
    }

    #[tokio::test]
    async fn unreadable_orq_keeps_the_last_good_rows_marked() {
        let (addr, seen) = python(|path, n| match (path, n) {
            ("/internal/diag", _) => (200, "{}".into()),
            (_, 1) => (200, facts_json(json!([{"name": "orq-1", "provider": "orq"}]), Value::Null).replace(r#""hidden":[]"#, r#""hidden":["orq-1"]"#)),
            _ => (200, facts_json(json!([]), json!("orq_unreadable"))),
        }).await;
        let client = FactsClient::new(addr, "s".into());
        let first = client.fetch(&[], 1, &BTreeMap::new(), false).await;
        assert_eq!((first.ok, first.facts.orq[0].problema.as_deref()), (true, None));
        let second = client.fetch(&[], 2, &BTreeMap::new(), false).await;
        let names: Vec<_> = second.facts.orq.iter().map(|r| (r.name.as_str(), r.problema.as_deref())).collect();
        assert_eq!(names, [("orq-1", Some(ORQ_UNAVAILABLE))]);
        assert!(second.facts.hidden.contains("orq-1"), "escondida continua escondida");
        assert_eq!(diary(&seen, "rust.list_orq_unavailable").await["codigo"], "orq_unreadable");
    }

    #[test]
    fn stale_facts_mark_every_row_without_hiding_a_sharper_problem() {
        let row = |v: Value| serde_json::from_value::<SessionRow>(v).unwrap();
        let mut rows = vec![row(json!({"name": "c", "provider": "claude"})), row(json!({"name": "h", "provider": "claude",
            "headless": true, "problema": "list_runtime_unavailable"}))];
        let good = ListFacts { unknown: false, ..Default::default() };
        mark_stale(&mut rows, &good, true);
        assert_eq!(rows[0].problema, None);
        mark_stale(&mut rows, &good, false);
        let got: Vec<_> = rows.iter().map(|r| r.problema.as_deref()).collect();
        assert_eq!(got, [Some(UNAVAILABLE), Some("list_runtime_unavailable")]);
    }
}
