//! Fatos da lista que continuam do Python (`POST /internal/list/facts`): estado dos provedores não
//! migrados, conta de Kimi/Pi/omp, transferências, orquestrações, acesso, navegador, terminais de
//! atalho e problemas do runtime. Pergunta só quando a entrada muda ou o último ficou velho, com
//! prazo de 1 s; sem resposta, fica o último valor e as linhas que dependem dele levam
//! `problema = list_facts_unavailable`. A lista nunca passa ao Python.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::net::SocketAddr;
use std::sync::Arc;
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
    pub shared: HashSet<String>,
    pub owners: HashMap<String, String>,
    /// Escondidas do dono (sessão de convidado que ele não vê).
    pub hidden: HashSet<String>,
    pub problems: BTreeMap<String, String>,
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
            shared: HashSet::new(), owners: HashMap::new(), hidden: HashSet::new(), problems: BTreeMap::new(),
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
    /// Trava assíncrona = uma pergunta por vez; quem chega no meio reaproveita a resposta.
    last: tokio::sync::Mutex<Option<Last>>,
}

impl FactsClient {
    pub fn new(upstream: SocketAddr, secret: String) -> Self {
        Self { upstream, secret, http: crate::proxy::client(), last: tokio::sync::Mutex::new(None) }
    }

    /// Outro cliente para o mesmo Python, com fila e último valor próprios: a sombra não espera a
    /// produção de verdade nem lhe empresta uma falha.
    pub fn sibling(&self) -> Self {
        Self { upstream: self.upstream, secret: self.secret.clone(), http: self.http.clone(), last: tokio::sync::Mutex::new(None) }
    }

    /// `shadow`: pedido da rodada em sombra; o Python não mexe na presença do app e manda a
    /// assinatura da lista dele.
    pub async fn fetch(&self, rows: &[SessionRow], owner_clients: u32, pane_pids: &BTreeMap<String, u32>, shadow: bool) -> Fetched {
        let mut last = self.last.lock().await;
        let body = match serde_json::to_vec(&Request { rows, owner_clients, pane_pids, shadow }) {
            Ok(b) => b,
            Err(_) => return self.failed(&mut last, "list_facts_request"),
        };
        let mut h = std::collections::hash_map::DefaultHasher::new();
        body.hash(&mut h);
        let key = h.finish();
        if let Some(l) = last.as_ref().filter(|l| l.at.elapsed() < FACTS_TTL && (l.key == key || !l.ok)) {
            return Fetched { facts: l.facts.clone(), ok: l.ok };
        }
        match self.ask(body).await {
            Ok(facts) => {
                let facts = Arc::new(facts);
                *last = Some(Last { key, at: Instant::now(), ok: true, facts: facts.clone() });
                Fetched { facts, ok: true }
            }
            Err(code) => self.failed(&mut last, code),
        }
    }

    fn failed(&self, last: &mut Option<Last>, code: &'static str) -> Fetched {
        if crate::warn_limit::allow(None, code) {
            tracing::warn!(code, "lista: fatos do Python indisponíveis; fica o último valor");
        }
        let facts = last.as_ref().map_or_else(Arc::default, |l| l.facts.clone());
        *last = Some(Last { key: 0, at: Instant::now(), ok: false, facts: facts.clone() });
        Fetched { facts, ok: false }
    }

    async fn ask(&self, body: Vec<u8>) -> Result<ListFacts, &'static str> {
        let req = axum::http::Request::post(format!("http://{}/internal/list/facts", self.upstream))
            .header("x-hangar-internal", &self.secret).header("content-type", "application/json")
            .body(Body::from(body)).map_err(|_| "list_facts_request")?;
        let work = async {
            let resp = self.http.request(req).await.map_err(|_| "list_facts_unreachable")?;
            if !resp.status().is_success() {
                return Err("list_facts_status");
            }
            let bytes = http_body_util::Limited::new(resp.into_body(), MAX_BODY).collect().await
                .map_err(|_| "list_facts_body")?.to_bytes();
            serde_json::from_slice(&bytes).map_err(|_| "list_facts_invalid")
        };
        tokio::time::timeout(FACTS_TIMEOUT, work).await.map_err(|_| "list_facts_timeout")?
    }

    /// `hooks.demote_awaiting` no Python, sem segurar a produção: quem é dono do mapa de marcadores
    /// é ele. Falhou, o próximo tique captura de novo e pede outra vez.
    pub fn demote(&self, sids: Vec<String>) {
        let body = serde_json::json!({"sids": sids}).to_string();
        let (http, url, secret) = (self.http.clone(), format!("http://{}/internal/list/demote", self.upstream), self.secret.clone());
        tokio::spawn(async move {
            let Ok(req) = axum::http::Request::post(url).header("x-hangar-internal", secret)
                .header("content-type", "application/json").body(Body::from(body)) else { return };
            let ok = matches!(tokio::time::timeout(Duration::from_secs(5), http.request(req)).await,
                Ok(Ok(ref r)) if r.status().is_success());
            if !ok && crate::warn_limit::allow(None, "list_demote_failed") {
                tracing::warn!(code = "list_demote_failed", "lista: rebaixamento de awaiting não chegou ao Python");
            }
        });
    }
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
            if row.provider != "codex" { row.conta.clone_from(&s.conta); }
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
