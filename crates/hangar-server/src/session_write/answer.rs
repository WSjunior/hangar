//! `/answer` de sessão Claude (resposta ao AskUserQuestion), com a mesma resposta do Python (`api.answer`,
//! `runtime_terminal.answer_sync`). Com terminal, "Conversar sobre isso" fecha a pergunta (`interrupt`)
//! e manda as respostas como texto (`input`) pelo próprio ator, sem emprestar o teclado ao Python.
//! Divergências deliberadas, espelhadas no Python: falha do runtime é 502 `erro_envio_falhou`; texto do
//! "Conversar" não confirmado é 502 `erro_envio_falhou`; o ator sem terminal que recusa a resposta
//! (`claude_command`) é 409 `erro_codex_resposta_invalida`.
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{Value, json};

use super::control::{control_step_answer, panel_open_refusal, plugin_down, plugin_pending};
use super::input::{MSG_CONTROL_DEFERRED, MSG_CONTROL_UNCONFIRMED, answer as respond, json_content_type, runtime_failed};
use super::{Ctx, WriteRoute, admit, detail_body, relay};
use crate::mods::state::random_hex;
use crate::routes::AppState;
use crate::runtime::gateway::EntryHandle;
use crate::runtime::protocol::{Disposition, OperationKind, RuntimeCommand, RuntimeError, RuntimeReply};
use crate::runtime::terminal::{TerminalHandle, TerminalTarget};
use crate::state::ask;
use crate::state::capture::PaneCapture;

type Answer = (StatusCode, Value);

/// Quanto esperar o picker sumir do pane depois do Esc, antes de digitar o texto.
const PICKER_CLOSE_WAIT: Duration = Duration::from_secs(3);
const PICKER_POLL: Duration = Duration::from_millis(100);

const MSG_CHAT_EMPTY: &str = "resposta sem texto para conversar";
const MSG_CHANGED: &str = "a pergunta mudou; resposta conservada";
const MSG_CHAT_UNCONFIRMED: &str = "a pergunta foi fechada, mas a resposta por texto não foi confirmada — confira na sessão antes de responder de novo";
const MSG_HEADLESS_INVALID: &str = "A pergunta mudou ou não aceita essas respostas. Confira as opções e tente novamente.";
const MSG_HEADLESS_SEND: &str = "Não foi possível enviar a resposta.";
const MSG_CODEX_SEND: &str = "Não foi possível confirmar o envio da resposta ao Codex.";
/// Código com que o ator sem terminal recusa uma resposta inválida (`claude.rs::error`).
const ACTOR_REFUSAL: &str = "claude_command";

const FIELDS: [&str; 8] = ["kind", "question_id", "indices", "multi", "value", "labels", "type_index", "chat_index"];

/// Um item de `answers`, só nos tipos que o FastAPI aceita sem converter nada.
#[derive(Clone, Debug)]
pub struct Item {
    kind: String,
    question_id: Option<String>,
    indices: Option<Vec<i64>>,
    multi: bool,
    value: Option<String>,
    labels: Vec<String>,
    type_index: Option<i64>,
    chat_index: Option<i64>,
}

impl Item {
    /// O formato que o ator lê; `indices` ausente vira lista vazia, como no Python.
    fn to_json(&self) -> Value {
        json!({"kind": self.kind, "question_id": self.question_id, "indices": self.indices.clone().unwrap_or_default(),
            "multi": self.multi, "value": self.value, "labels": self.labels, "type_index": self.type_index, "chat_index": self.chat_index})
    }
}

pub struct Body {
    answers: Vec<Item>,
    /// `null`, texto ou inteiro: o `int | str | None` do `AnswerBody`.
    request_id: Value,
}

impl Body {
    pub fn answers_json(&self) -> Vec<Value> { self.answers.iter().map(Item::to_json).collect() }
}

fn optional<T>(value: Option<&Value>, read: impl Fn(&Value) -> Option<T>) -> Option<Option<T>> {
    match value { None | Some(Value::Null) => Some(None), Some(v) => read(v).map(Some) }
}

fn parse_item(value: &Value) -> Option<Item> {
    let object = value.as_object()?;
    if object.keys().any(|k| !FIELDS.contains(&k.as_str())) { return None; }
    let strings = |v: &Value| v.as_array()?.iter().map(|s| s.as_str().map(str::to_owned)).collect::<Option<Vec<_>>>();
    Some(Item {
        kind: object.get("kind")?.as_str()?.to_owned(),
        question_id: optional(object.get("question_id"), |v| v.as_str().map(str::to_owned))?,
        indices: optional(object.get("indices"), |v| v.as_array()?.iter().map(Value::as_i64).collect::<Option<Vec<_>>>())?,
        multi: match object.get("multi") { None => false, Some(Value::Bool(b)) => *b, Some(_) => return None },
        value: optional(object.get("value"), |v| v.as_str().map(str::to_owned))?,
        labels: match object.get("labels") { None => Vec::new(), Some(v) => strings(v)? },
        type_index: optional(object.get("type_index"), Value::as_i64)?,
        chat_index: optional(object.get("chat_index"), Value::as_i64)?,
    })
}

/// `AnswerBody` estrito. Tipo que o pydantic converte (`"1"`, `1.0`, `"true"`) ou recusa (campo a mais)
/// não passa daqui: quem decide é o FastAPI, que recebe o pedido intacto.
pub fn parse_body(bytes: &Bytes) -> Option<Body> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let object = value.as_object()?;
    if object.keys().any(|k| k != "answers" && k != "request_id") { return None; }
    let answers = object.get("answers")?.as_array()?.iter().map(parse_item).collect::<Option<Vec<_>>>()?;
    let request_id = match object.get("request_id") {
        None | Some(Value::Null) => Value::Null,
        Some(v @ Value::String(_)) => v.clone(),
        Some(v) if v.as_i64().is_some() => v.clone(),
        Some(_) => return None,
    };
    Some(Body { answers, request_id })
}

fn refused(msg: &str) -> Answer { (StatusCode::CONFLICT, detail_body("erro_sem_resposta", msg, json!({}))) }

fn success() -> Answer { (StatusCode::OK, json!({"ok": true, "fallback": false})) }

/// `repr()` de texto do Python, para a mensagem de tipo desconhecido.
fn py_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') { '"' } else { '\'' };
    let mut out = String::from(quote);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => { out.push('\\'); out.push(c); }
            c if c.is_control() && (c as u32) < 0x100 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// `terminal_input._validate`: tudo checado antes de tocar no pane; a primeira falha é a mensagem.
pub fn validate(items: &[Item]) -> Result<(), String> {
    for a in items {
        match a.kind.as_str() {
            "text" => {
                let Some(value) = &a.value else { return Err("value required for text kind".into()) };
                if value.chars().any(|c| (c as u32) < 32 && c != '\t') { return Err("control characters not allowed".into()); }
                if a.type_index.is_none() { return Err("type_index required for text kind".into()); }
            }
            "chat" if a.chat_index.is_none() => return Err("chat_index required for chat kind".into()),
            "chat" => {}
            "option" => {
                let indices = a.indices.as_deref().unwrap_or_default();
                if indices.is_empty() { return Err("indices required for option kind".into()); }
                if indices.iter().any(|i| *i < 0) { return Err("indices must be >= 0".into()); }
            }
            other => return Err(format!("unknown answer kind: {}", py_repr(other))),
        }
    }
    Ok(())
}

/// `runtime_terminal._validate_text`: texto de terminal não é branco nem tem caractere de controle.
fn text_ok(text: &str) -> bool { !text.trim().is_empty() && !text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\t')) }

/// `runtime_terminal._validate_control` para `answer_questions`, depois de `validate`.
fn validate_control(items: &[Item], request_id: &Value) -> Result<(), String> {
    if items.is_empty() { return Err("resposta terminal vazia".into()); }
    let in_range = |i: &i64| (0..100).contains(i);
    for a in items {
        match a.kind.as_str() {
            "option" => {
                let indices = a.indices.as_deref().unwrap_or_default();
                if a.labels.is_empty() || !indices.iter().all(in_range) || (!a.multi && indices.len() != 1) { return Err("opção fora da pergunta".into()); }
            }
            "text" | "chat" => {
                let position = if a.kind == "text" { a.type_index } else { a.chat_index };
                if !position.as_ref().is_some_and(in_range) { return Err("posição fora da pergunta".into()); }
                if let Some(value) = a.value.as_deref().filter(|_| a.kind == "text") {
                    if !text_ok(value) { return Err("texto inválido para terminal".into()); }
                    if value.contains('\n') { return Err("resposta terminal com quebra de linha".into()); }
                }
            }
            _ => {}
        }
    }
    if !request_id.is_null() && !request_id.is_string() { return Err("pedido terminal sem identidade válida".into()); }
    Ok(())
}

/// `x or y` do Python sobre o `request_id`: nulo, vazio e zero são falsos.
fn truthy(value: &Value) -> bool {
    match value { Value::Null => false, Value::String(s) => !s.is_empty(), Value::Number(n) => n.as_i64() != Some(0), _ => true }
}

pub enum Plan {
    /// O controle `answer_questions` com este corpo.
    Control(Value),
    /// "Conversar sobre isso" sem pergunta segurada: as respostas viram texto.
    Chat,
}

/// A decisão do Python antes de tocar no pane, na ordem dele: painel, validação, identidade da
/// pergunta, e só então o caminho do chat ou o controle.
pub fn terminal_plan(body: &Body, pending: Option<&Value>, panel_open: bool) -> Result<Plan, Answer> {
    if pending.is_none() && panel_open { return Err(panel_open_refusal()); }
    validate(&body.answers).map_err(|msg| refused(&msg))?;
    let request_id = if truthy(&body.request_id) { body.request_id.clone() } else { pending.map_or(Value::Null, |p| p["id"].clone()) };
    let mut payload = json!({"answers": body.answers_json()});
    match pending {
        Some(held) => {
            let id = held["id"].as_str().map_or_else(|| held["id"].to_string(), str::to_owned);
            let same = request_id.as_str().is_some_and(|r| r == id || r == id.strip_prefix("ask:").unwrap_or(&id));
            if truthy(&request_id) && !same { return Err(refused(MSG_CHANGED)); }
            payload["request_id"] = json!(if id.starts_with("ask:") || id.starts_with("perm:") { id } else { format!("ask:{id}") });
        }
        None => {
            if body.answers.iter().any(|a| a.kind == "chat") { return Ok(Plan::Chat); }
            if truthy(&request_id) { payload["request_id"] = request_id.clone(); }
        }
    }
    validate_control(&body.answers, payload.get("request_id").unwrap_or(&Value::Null)).map_err(|msg| refused(&msg))?;
    Ok(Plan::Control(payload))
}

/// Porta de `_askq_conversar_text`: as respostas já dadas viram mensagem, porque a TUI descarta o
/// que foi respondido ao escolher "Chat about this". Sem nada a preservar devolve vazio.
pub fn chat_text(answers: &[Value], questions: &[String]) -> String {
    let question = |i: usize| questions.get(i).map(String::as_str).filter(|q| !q.is_empty());
    let (mut answered, mut chatting) = (Vec::new(), Vec::new());
    for (i, a) in answers.iter().enumerate() {
        if a["kind"] == "chat" {
            if let Some(q) = question(i) { chatting.push(q); }
            continue;
        }
        let response = if a["kind"] == "text" { a["value"].as_str().unwrap_or_default().to_owned() }
            else { a["labels"].as_array().map(|l| l.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default() };
        if response.is_empty() { continue; }
        answered.push(match question(i) { Some(q) => format!("- {q} → {response}"), None => format!("- {response}") });
    }
    if answered.is_empty() { return String::new(); }
    let mut lines = match chatting.len() {
        0 => vec!["Sobre uma das perguntas prefiro conversar antes de responder.".to_owned()],
        1 => vec![format!("Sobre «{}» prefiro conversar antes de responder.", chatting[0])],
        _ => {
            let mut lines = vec!["Sobre estas perguntas prefiro conversar antes de responder:".to_owned()];
            lines.extend(chatting.iter().map(|q| format!("- {q}")));
            lines
        }
    };
    lines.push("Já respondi:".into());
    lines.extend(answered);
    lines.join("\n")
}

pub fn chat_refusal(text: &str) -> Option<Answer> { text.is_empty().then(|| refused(MSG_CHAT_EMPTY)) }

pub fn control_answer(sent: &Result<RuntimeReply, RuntimeError>) -> Answer {
    let reply = match sent {
        Ok(reply) => reply,
        Err(error) => return runtime_failed(error),
    };
    match reply.disposition {
        Disposition::Accepted => success(),
        Disposition::Deferred => refused(MSG_CONTROL_DEFERRED),
        _ => refused(MSG_CONTROL_UNCONFIRMED),
    }
}

/// O texto entrou ou ficou na fila (`deferred`, sai quando o terminal ficar livre); o resto não foi confirmado.
pub fn chat_submit_answer(sent: &Result<RuntimeReply, RuntimeError>) -> Answer {
    match sent {
        Err(error) => runtime_failed(error),
        Ok(reply) if matches!(reply.disposition, Disposition::Accepted | Disposition::Deferred) => success(),
        // 502, não 409: o app trata 409 como "nada digitado", e aqui a pergunta já fechou e o texto pode ter entrado.
        Ok(_) => (StatusCode::BAD_GATEWAY, detail_body("erro_envio_falhou", MSG_CHAT_UNCONFIRMED, json!({"erro": MSG_CHAT_UNCONFIRMED}))),
    }
}

/// O corpo do `answer_questions` sem terminal: o ator confere a pergunta pendente e monta a resposta.
pub fn headless_command(body: &Body) -> Value { json!({"request_id": body.request_id, "answers": body.answers_json()}) }

/// O ator exige o id da pergunta; sem ele quem decide é o Python, que não confere.
pub fn relays_headless(body: &Body) -> bool { body.request_id.is_null() }

/// No Codex a recusa do ator (`codex_command`) é falha de envio, como no Python, com a frase do Codex.
pub fn headless_answer(codex: bool, sent: &Result<RuntimeReply, RuntimeError>) -> Answer {
    let invalid = || (StatusCode::CONFLICT, detail_body("erro_codex_resposta_invalida", MSG_HEADLESS_INVALID, json!({})));
    let unsent = || (StatusCode::SERVICE_UNAVAILABLE, detail_body("erro_codex_resposta_envio",
        if codex { MSG_CODEX_SEND } else { MSG_HEADLESS_SEND }, json!({})));
    match sent {
        Err(error) if error.code == ACTOR_REFUSAL && !codex => invalid(),
        Err(_) => unsent(),
        Ok(reply) => match reply.disposition {
            Disposition::Accepted => success(),
            Disposition::Rejected | Disposition::Deferred => invalid(),
            Disposition::Unknown => unsent(),
        },
    }
}

/// Espera o menu sair do pane. `read` devolve se o rodapé ainda aparece; `None` (captura que falhou)
/// não segura o texto. Estourou o prazo: devolve `false` e quem chama envia assim mesmo.
pub async fn wait_footer_gone<F, Fut>(mut read: F, limit: Duration, poll: Duration) -> bool
where F: FnMut() -> Fut, Fut: Future<Output = Option<bool>> {
    let start = Instant::now();
    loop {
        if read().await != Some(true) { return true; }
        if start.elapsed() >= limit { return false; }
        tokio::time::sleep(poll).await;
    }
}

async fn picker_closed(ctx: &Ctx, target: &TerminalTarget) -> bool {
    // Consumidor só desta espera: o `monitor:` da sessão divide o observador e não pode ser solto aqui.
    let capture = PaneCapture::with_consumer(ctx.st.terminal.clone(), format!("answer:{}:{}", ctx.name, random_hex(8)),
        ctx.st.list.env().capture_program.clone(), &ctx.name, &target.binding.conversation, target.binding.pane.clone());
    let gone = wait_footer_gone(|| async {
        match capture.capture().await {
            Ok(frame) => Some(crate::terminal_state::footer_visible(&frame.text)),
            Err(failed) => {
                tracing::warn!(session = %ctx.name, code = %failed.code, "answer: captura do pane falhou; o texto segue sem esperar o menu fechar");
                None
            }
        }
    }, PICKER_CLOSE_WAIT, PICKER_POLL).await;
    capture.release().await;
    gone
}

/// As perguntas do sidecar, na ordem; ausente ou quebrado é "sem perguntas", como no Python.
async fn sidecar_questions(target: &TerminalTarget) -> Vec<String> {
    let jsonl = target.transcript.clone();
    tokio::task::spawn_blocking(move || match ask::read_pending(&jsonl) {
        Ok(Some(pending)) => pending.questions.into_iter().map(|q| q.question).collect(),
        _ => Vec::new(),
    }).await.unwrap_or_default()
}

async fn chat(ctx: &Ctx, body: &Body, target: &TerminalTarget, handle: &TerminalHandle) -> Answer {
    let text = chat_text(&body.answers_json(), &sidecar_questions(target).await);
    if let Some(empty) = chat_refusal(&text) { return empty; }
    // O Esc fecha a pergunta inteira; o texto só entra quando o menu saiu, senão a TUI o engole.
    if let Err(not_closed) = control_step_answer(&handle.control(random_hex(16), "interrupt".into(), json!({})).await) { return not_closed; }
    if !picker_closed(ctx, target).await {
        tracing::warn!(session = %ctx.name, "answer: o menu não fechou em 3 s depois do Esc; enviando o texto assim mesmo");
    }
    let command = RuntimeCommand { operation_id: random_hex(16), kind: OperationKind::Input, payload: json!({"text": text, "pre_transcript": false}) };
    chat_submit_answer(&handle.command(command).await)
}

async fn terminal(ctx: &Ctx, body: &Body, target: &TerminalTarget, handle: &TerminalHandle) -> Answer {
    let pending = match plugin_pending(ctx).await {
        Ok(pending) => pending,
        Err(()) => return plugin_down("erro_sem_resposta"),
    };
    let done = match terminal_plan(body, pending.as_ref(), ctx.st.term.is_active(&ctx.name)) {
        Err(refusal) => return refusal,
        Ok(Plan::Control(payload)) => control_answer(&handle.control(random_hex(16), "answer_questions".into(), payload).await),
        Ok(Plan::Chat) => chat(ctx, body, target, handle).await,
    };
    if done.0 == StatusCode::OK && let Some(sidecar) = ask::sidecar_path(&target.transcript) {
        // Pergunta respondida: o sidecar não pode reabri-la na tela. Ausente é o caso normal.
        let _ = tokio::fs::remove_file(sidecar).await;
    }
    done
}

pub async fn answer(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let (ctx, bytes) = match admit(&st, peer, req, WriteRoute::Answer, |_| false).await {
        Ok(admitted) => admitted,
        Err(response) => return response,
    };
    let Some(body) = parse_body(&bytes).filter(|_| json_content_type(ctx.headers())) else { return relay(ctx, bytes).await };
    let result = match &ctx.target.handle {
        EntryHandle::Terminal { target, handle } => terminal(&ctx, &body, target, handle).await,
        // Sem pergunta pendente de referência, o Python decide o que `request_id` ausente significa.
        _ if relays_headless(&body) => return relay(ctx, bytes).await,
        headless => headless_answer(ctx.target.provider == "codex", &headless.command(RuntimeCommand { operation_id: random_hex(16), kind: OperationKind::AnswerQuestions, payload: headless_command(&body) }).await),
    };
    respond(&ctx, result)
}
