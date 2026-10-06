//! `ListHub`: a lista do dono no Rust (`GET /api/sessions` e `/api/sessions/events`). Um produtor
//! por servidor, ligado enquanto houver lista aberta: tique de 1,5 s depois do trabalho (descoberta,
//! fatos do Python, classificação, decoração e rebaixamento, tudo pela ponte), assinatura, e JSON
//! só quando ela muda. Cada conexão só lê o último publicado. Contrato público do `list_events`
//! (`sse.py`): `sessions`, `list_error` uma vez na transição, `shortcut_terminals`, `nav` uma vez
//! por conexão, `ping` a cada 8 s e comentário a cada 15 s. Convidado continua no Python.
use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures_util::future::BoxFuture;
use hangar_api::session::SessionRow;
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};

use super::bridge::{ListBridge, ListError, Produced};
use super::sig;
use crate::diag::DiagClient;
use crate::routes::{AppState, cors, gate, pass};
use crate::tail::{comment_frame, ping_frame, sse_frame};

/// O tique do `_ListRefresher`, contado do fim do trabalho.
const TICK: Duration = Duration::from_millis(1500);
const PING_EVERY: Duration = Duration::from_secs(8);
const COMMENT_EVERY: Duration = Duration::from_secs(15);
/// Envio preso por 30 s fecha a conexão, como o `send_timeout` do Python.
const SEND_TIMEOUT: Duration = Duration::from_secs(30);

/// Retrato do runtime das sessões sem terminal, por chave da sessão.
pub trait HeadlessSource: Send + Sync {
    fn snapshots(&self) -> BoxFuture<'_, BTreeMap<String, Value>>;
}

impl HeadlessSource for crate::runtime::gateway::RuntimeRegistry {
    fn snapshots(&self) -> BoxFuture<'_, BTreeMap<String, Value>> { Box::pin(self.list_snapshots()) }
}

/// O que o produtor publicou por último, em quadros prontos: cada conexão só clona o `Bytes`.
/// Os `*_seq` andam quando o quadro deve sair de novo.
#[derive(Clone, Default)]
struct Published {
    sessions: Option<Bytes>,
    sessions_seq: u64,
    /// Código e quadro enquanto a lista está em erro.
    error: Option<(&'static str, Bytes)>,
    error_seq: u64,
    shortcuts: Option<Bytes>,
    shortcuts_seq: u64,
    /// (nome, `ts` do pedido, quadro `{name, url}`)
    nav: Arc<Vec<(String, f64, Bytes)>>,
}

#[derive(Default)]
struct Refs { clients: u32, running: bool }

pub struct ListHub {
    tx: watch::Sender<Arc<Published>>,
    refs: Mutex<Refs>,
}

impl Default for ListHub {
    fn default() -> Self { Self { tx: watch::channel(Arc::default()).0, refs: Mutex::default() } }
}

/// Uma lista do dono aberta; solta a contagem ao cair.
struct Subscription { hub: Arc<ListHub>, rx: watch::Receiver<Arc<Published>> }

impl Drop for Subscription {
    fn drop(&mut self) {
        let mut r = lock(&self.hub.refs);
        r.clients = r.clients.saturating_sub(1);
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> { m.lock().unwrap_or_else(|e| e.into_inner()) }

impl ListHub {
    fn subscribe(self: &Arc<Self>, list: &Arc<ListBridge>, diag: &DiagClient) -> Subscription {
        let mut r = lock(&self.refs);
        r.clients += 1;
        if !r.running {
            r.running = true;
            tokio::spawn(supervise(self.clone(), list.clone(), diag.clone()));
        }
        Subscription { hub: self.clone(), rx: self.tx.subscribe() }
    }
}

/// O que o produtor guarda entre tiques para só publicar mudança.
#[derive(Default)]
struct Last { sig: Option<String>, nav: Value, shortcuts: Option<String> }

/// Pânico fora da produção (assinatura, publicação) não deixa o hub sem produtor: as listas abertas
/// ficariam só com `ping`, e nenhuma nova religaria.
async fn supervise(hub: Arc<ListHub>, list: Arc<ListBridge>, diag: DiagClient) {
    loop {
        match tokio::spawn(produce(hub.clone(), list.clone(), diag.clone())).await {
            Err(e) if e.is_panic() => {
                diag.report("rust.list_failed", "", "list_producer_panic", "produtor da lista do dono caiu; recomeça");
                tokio::time::sleep(TICK).await;
            }
            _ => return,
        }
    }
}

/// O laço único. Para quando a última lista fecha; quem abrir depois religa.
async fn produce(hub: Arc<ListHub>, list: Arc<ListBridge>, diag: DiagClient) {
    let mut last = Last::default();
    loop {
        let clients = {
            let mut r = lock(&hub.refs);
            if r.clients == 0 {
                r.running = false;
                list.set_owner_clients(0);
                // Quem abrir depois não recebe primeiro a lista de quando esta fechou.
                hub.tx.send_replace(Arc::default());
                return;
            }
            r.clients
        };
        list.set_owner_clients(clients);
        // Pânico na produção vira erro da rodada, e o laço segue.
        let l = list.clone();
        let outcome = match tokio::spawn(async move { l.refresh().await }).await {
            Ok(Ok(p)) if p.facts.unknown => Err("list_facts_unknown"),
            Ok(Ok(p)) => Ok(p),
            Ok(Err(e)) => Err(e.code),
            Err(_) => Err("list_task_failed"),
        };
        if let Err(code) = outcome {
            diag.report("rust.list_failed", "", code, "lista do dono em erro");
        }
        publish(&hub.tx, &mut last, outcome, &diag);
        tokio::time::sleep(TICK).await;
    }
}

/// Linhas que o dono vê: as escondidas dele (sessão de convidado que ele não vê) não saem.
fn visible(p: &Produced) -> Vec<&SessionRow> {
    p.rows.iter().filter(|r| !p.facts.hidden.contains(&r.name)).collect()
}

fn error_frame(code: &str) -> Bytes { sse_frame("list_error", &json!({"code": code}).to_string(), None) }

fn publish(tx: &watch::Sender<Arc<Published>>, last: &mut Last, outcome: Result<Produced, &'static str>, diag: &DiagClient) {
    let prev = tx.borrow().clone();
    let mut next = (*prev).clone();
    let mut changed = false;
    match outcome {
        Err(code) => changed |= fail(&prev, &mut next, code),
        Ok(p) => {
            let rows = visible(&p);
            let sig = sig::list_sig(rows.iter().copied());
            // Volta do erro reemite a mesma lista: é ela que limpa o erro na tela.
            if prev.error.is_some() || last.sig.as_deref() != Some(sig.as_str()) {
                match serde_json::to_string(&rows) {
                    Ok(json) => {
                        next.sessions = Some(sse_frame("sessions", &json, None));
                        next.sessions_seq += 1;
                        next.error = None;
                        last.sig = Some(sig);
                        changed = true;
                    }
                    Err(_) => {
                        diag.report("rust.list_failed", "", "list_unserializable", "lista do dono sem serializar");
                        changed |= fail(&prev, &mut next, "list_unserializable");
                    }
                }
            }
            // `None` é antes da primeira leitura boa: nada a mandar, como no Python.
            if let Some(sc) = p.facts.shortcuts.as_deref().filter(|sc| last.shortcuts.as_deref() != Some(*sc)) {
                next.shortcuts = Some(sse_frame("shortcut_terminals", sc, None));
                next.shortcuts_seq += 1;
                last.shortcuts = Some(sc.to_owned());
                changed = true;
            }
            if p.facts.nav != last.nav {
                last.nav = p.facts.nav.clone();
                next.nav = Arc::new(nav_entries(&last.nav));
                changed = true;
            }
        }
    }
    if changed {
        tx.send_replace(Arc::new(next));
    }
}

/// Na transição ou na troca de motivo: lista vazia por falha seria igual a nenhuma sessão.
fn fail(prev: &Published, next: &mut Published, code: &'static str) -> bool {
    if prev.error.as_ref().map(|e| e.0) == Some(code) {
        return false;
    }
    next.error = Some((code, error_frame(code)));
    next.error_seq += 1;
    true
}

/// `{nome: {url, ts}}` dos fatos em quadros prontos.
fn nav_entries(nav: &Value) -> Vec<(String, f64, Bytes)> {
    let Some(map) = nav.as_object() else { return Vec::new() };
    map.iter().filter_map(|(name, m)| {
        let url = m["url"].as_str()?;
        Some((name.clone(), m["ts"].as_f64().unwrap_or(0.0), sse_frame("nav", &json!({"name": name, "url": url}).to_string(), None)))
    }).collect()
}

/// O que uma conexão já entregou.
#[derive(Default)]
struct Sent { sessions: u64, error: u64, shortcuts: u64, nav: HashMap<String, f64> }

/// Quadros que esta conexão ainda não mandou do que está publicado.
fn frames(p: &Published, sent: &mut Sent) -> Vec<Bytes> {
    let mut out = Vec::new();
    if let Some((_, frame)) = &p.error {
        if sent.error != p.error_seq {
            sent.error = p.error_seq;
            out.push(frame.clone());
        }
    } else if let Some(s) = p.sessions.as_ref().filter(|_| sent.sessions != p.sessions_seq) {
        sent.sessions = p.sessions_seq;
        out.push(s.clone());
    }
    if let Some(sc) = p.shortcuts.as_ref().filter(|_| sent.shortcuts != p.shortcuts_seq) {
        sent.shortcuts = p.shortcuts_seq;
        out.push(sc.clone());
    }
    for (name, ts, frame) in p.nav.iter() {
        if sent.nav.get(name) != Some(ts) {
            sent.nav.insert(name.clone(), *ts);
            out.push(frame.clone());
        }
    }
    // Pedido confirmado ou vencido sai do mapa: a memória da conexão não cresce com eles.
    sent.nav.retain(|n, _| p.nav.iter().any(|(m, ..)| m == n));
    out
}

async fn push(out: &mpsc::Sender<Bytes>, frame: Bytes) -> bool {
    matches!(tokio::time::timeout(SEND_TIMEOUT, out.send(frame)).await, Ok(Ok(())))
}

async fn client_loop(mut sub: Subscription, out: mpsc::Sender<Bytes>) {
    // O ping sai antes de qualquer lista: a primeira produção pode levar segundos.
    if !push(&out, ping_frame()).await {
        return;
    }
    let start = tokio::time::Instant::now();
    let mut ping = tokio::time::interval_at(start + PING_EVERY, PING_EVERY);
    let mut comment = tokio::time::interval_at(start + COMMENT_EVERY, COMMENT_EVERY);
    let mut sent = Sent::default();
    loop {
        let p = sub.rx.borrow_and_update().clone();
        for f in frames(&p, &mut sent) {
            if !push(&out, f).await {
                return;
            }
        }
        tokio::select! {
            _ = out.closed() => return,
            _ = ping.tick() => if !push(&out, ping_frame()).await { return },
            _ = comment.tick() => if !push(&out, comment_frame()).await { return },
            changed = sub.rx.changed() => if changed.is_err() { return },
        }
    }
}

/// `GET /api/sessions/events` do dono.
pub async fn events(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let (fwd, owner) = gate(&st, peer, &req);
    if !owner || req.method() != Method::GET {
        return pass(&st, req, &fwd).await;
    }
    let sub = st.hub.subscribe(&st.list, &st.diag);
    let (tx, rx) = mpsc::channel::<Bytes>(16);
    tokio::spawn(client_loop(sub, tx));
    let stream = futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|b| (Ok::<Bytes, Infallible>(b), rx)) });
    let mut resp = Response::new(Body::from_stream(stream));
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/event-stream; charset=utf-8"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));
    h.insert("x-accel-buffering", HeaderValue::from_static("no"));
    cors(req.headers(), h);
    resp
}

/// `GET /api/sessions` do dono: o retrato de até 2 s (o do tique, com lista aberta).
pub async fn list(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let (fwd, owner) = gate(&st, peer, &req);
    if !owner || req.method() != Method::GET {
        return pass(&st, req, &fwd).await;
    }
    let produced = match st.list.snapshot().await {
        // Sem resposta dos fatos, acesso e escondidas são desconhecidos, não vazios.
        Ok(p) if p.facts.unknown => return unavailable(&st, req.headers(), ListError { code: "list_facts_unknown", detail: "" }),
        Ok(p) => p,
        Err(e) => return unavailable(&st, req.headers(), e),
    };
    let body = match serde_json::to_vec(&visible(&produced)) {
        Ok(b) => b,
        Err(_) => return unavailable(&st, req.headers(), ListError { code: "list_unserializable", detail: "" }),
    };
    let mut resp = Response::new(Body::empty());
    resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let body = crate::routes::maybe_gzip(req.headers(), resp.headers_mut(), body);
    *resp.body_mut() = Body::from(body);
    cors(req.headers(), resp.headers_mut());
    resp
}

/// 503 no formato do `api.py` (`_mux_indisponivel`, `_lista_indisponivel`): "não sei quais sessões
/// existem" nunca vira "você não tem nenhuma".
fn unavailable(st: &AppState, req: &HeaderMap, e: ListError) -> Response {
    st.diag.report("rust.list_route_failed", "", e.code, "lista do dono indisponível no GET");
    let detail = if e.code == "mux_unavailable" {
        json!({"code": "erro_mux_indisponivel", "params": {"detalhe": e.detail},
            "msg": "o tmux não respondeu — a lista de sessões está indisponível"})
    } else {
        json!({"code": "erro_lista_indisponivel", "params": {"detalhe": e.code}, "msg": "a lista de sessões está indisponível"})
    };
    let mut resp = (StatusCode::SERVICE_UNAVAILABLE, [(header::CONTENT_TYPE, "application/json")],
        json!({"detail": detail}).to_string()).into_response();
    cors(req, resp.headers_mut());
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    fn published(sessions: &str, seq: u64) -> Published {
        Published { sessions: Some(sse_frame("sessions", sessions, None)), sessions_seq: seq, ..Default::default() }
    }

    #[test]
    fn a_connection_sends_each_frame_once() {
        let mut sent = Sent::default();
        let p = published("[1]", 1);
        assert_eq!(frames(&p, &mut sent).len(), 1);
        assert!(frames(&p, &mut sent).is_empty(), "nada mudou");
        let err = Published { error: Some(("mux_unavailable", error_frame("mux_unavailable"))), error_seq: 1, ..p.clone() };
        let f = frames(&err, &mut sent);
        assert!(f.len() == 1 && f[0].starts_with(b"event: list_error"));
        assert!(frames(&err, &mut sent).is_empty(), "erro uma vez só");
        let back = Published { sessions_seq: 2, error: None, ..err.clone() };
        assert!(frames(&back, &mut sent)[0].starts_with(b"event: sessions"));
    }

    #[test]
    fn nav_map_is_pruned_with_the_request() {
        let mut sent = Sent::default();
        let p = Published { nav: Arc::new(nav_entries(&json!({"a": {"url": "u", "ts": 1.0}}))), ..Default::default() };
        assert_eq!(frames(&p, &mut sent).len(), 1);
        frames(&Published::default(), &mut sent);
        assert!(sent.nav.is_empty());
    }
}
