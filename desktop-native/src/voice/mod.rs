//! Conversa por voz: o Codex local fala, a sessão aberta na tela trabalha.
pub mod audio;
pub mod organizer;
pub mod plan;
pub mod rpc;
pub mod rtc;

use organizer::{MIC_VOICE_LEVEL, Results, SendGate, SpokenTurns, ToolCall, parse_tool, tool_reply, tools, thread_config, ORGANIZER_PROMPT, VOICE_PROMPT};
use rpc::{Codex, Incoming, Rpc, RpcError, handshake};
use serde_json::{Value, json};
use std::{sync::{Arc, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use tokio::{runtime::Handle, sync::{Notify, mpsc}};

pub struct CallId(Value);
pub enum Phase { Connecting, Live, Closed }
#[derive(Debug, Clone)]
pub enum VoiceFailure { Microphone, Speaker, AppServer, Realtime(String), Network, Timeout, Organizer }
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Activity { #[default] Idle, Thinking, Searching }
pub enum VoiceEvent { Phase(Phase), Levels(f32, f32), Draft(Option<String>), Activity(Activity), ReadSession(CallId), Send(CallId, String), Failed(VoiceFailure) }
pub struct VoiceOptions { pub codex: Codex, pub voice: Option<String>, pub context: String }

enum Command { Retarget(String, String), Result(String, String), Reply(Value, Value) }

pub struct Voice { commands: mpsc::UnboundedSender<Command>, muted: Arc<AtomicBool>, stopped: Arc<AtomicBool>, stop: Arc<Notify> }

impl Voice {
    pub fn start(runtime: &Handle, options: VoiceOptions, events: async_channel::Sender<VoiceEvent>) -> Voice {
        let (commands, inbox) = mpsc::unbounded_channel();
        let (muted, stopped, stop) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)), Arc::new(Notify::new()));
        runtime.spawn(call(options, events, inbox, muted.clone(), stopped.clone(), stop.clone()));
        Voice { commands, muted, stopped, stop }
    }
    pub fn set_muted(&self, muted: bool) { self.muted.store(muted, Ordering::Relaxed); }
    pub fn retarget(&self, name: String, context: String) { let _ = self.commands.send(Command::Retarget(name, context)); }
    pub fn session_result(&self, session: String, text: String) { let _ = self.commands.send(Command::Result(session, text)); }
    pub fn reply(&self, call: CallId, reply: Value) { let _ = self.commands.send(Command::Reply(call.0, reply)); }
    pub fn stop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        // notify_one guarda a licença mesmo sem ninguém esperando ainda.
        self.stop.notify_one();
    }
}

impl Drop for Voice { fn drop(&mut self) { self.stop(); } }

/// Diário da voz. Nunca recebe fala, transcrição, texto de pedido nem argumentos de ferramenta.
pub(crate) fn log(text: impl AsRef<str>) { crate::log_line(&format!("voice: {}", text.as_ref())); }

fn failed(step: &'static str) -> impl FnOnce(VoiceFailure) -> VoiceFailure {
    move |failure| { log(format!("{step} failed: {failure:?}")); failure }
}

/// Deltas vêm aos montes: só o primeiro de cada (método, papel) até virar o turno ou mudar o falante.
fn first_delta(last: &mut Option<(String, String)>, method: &str, role: &str) -> bool {
    if method == "turn/started" || method == "turn/completed" { *last = None; return true; }
    if !method.ends_with("/delta") { return true; }
    let key = (method.to_owned(), role.to_owned());
    if last.as_ref() == Some(&key) { return false; }
    *last = Some(key);
    true
}

fn log_notification(method: &str, params: &Value, ours: bool, last_delta: &mut Option<(String, String)>) {
    let role = params["role"].as_str().unwrap_or_default();
    if !first_delta(last_delta, method, role) { return; }
    let mut line = format!("notification {method}");
    if !ours { line.push_str(" thread=other"); }
    if method.starts_with("thread/realtime/transcript") { line.push_str(&format!(" role={role}")); }
    if method.starts_with("item/") { line.push_str(&format!(" item={}", params["item"]["type"].as_str().unwrap_or("?"))); }
    if method == "turn/completed" { line.push_str(&format!(" status={}", params["turn"]["status"].as_str().unwrap_or("?"))); }
    log(line);
}

async fn call(options: VoiceOptions, events: async_channel::Sender<VoiceEvent>, mut inbox: mpsc::UnboundedReceiver<Command>,
    muted: Arc<AtomicBool>, stopped: Arc<AtomicBool>, stop: Arc<Notify>) {
    log(format!("call start voice={}", options.voice.as_deref().unwrap_or("default")));
    log("phase Connecting");
    let _ = events.send(VoiceEvent::Phase(Phase::Connecting)).await;
    // Parar vale em qualquer fase: largar o future derruba o app-server (kill_on_drop) e o flag para a thread do RTC.
    let outcome = tokio::select! {
        outcome = run_call(options, &events, &mut inbox, &muted, &stopped) => outcome,
        _ = stop.notified() => { log("stop requested"); Ok(()) },
    };
    stopped.store(true, Ordering::Relaxed);
    match &outcome { Ok(()) => log("call end ok"), Err(failure) => log(format!("call end failure={failure:?}")) }
    if let Err(failure) = outcome { let _ = events.send(VoiceEvent::Failed(failure)).await; }
    log("phase Closed");
    let _ = events.send(VoiceEvent::Phase(Phase::Closed)).await;
}

fn rpc_failure(error: RpcError) -> VoiceFailure {
    match error { RpcError::Timeout => VoiceFailure::Timeout, RpcError::Server(m) => VoiceFailure::Realtime(m), _ => VoiceFailure::AppServer }
}

fn rtc_failure(error: rtc::RtcError) -> VoiceFailure {
    match error { rtc::RtcError::Microphone => VoiceFailure::Microphone, rtc::RtcError::Speaker => VoiceFailure::Speaker, _ => VoiceFailure::Network }
}

async fn run_call(options: VoiceOptions, events: &async_channel::Sender<VoiceEvent>, inbox: &mut mpsc::UnboundedReceiver<Command>,
    muted: &Arc<AtomicBool>, stopped: &Arc<AtomicBool>) -> Result<(), VoiceFailure> {
    let (rpc, incoming) = Rpc::spawn(&options.codex).await.map_err(rpc_failure).map_err(failed("app-server spawn"))?;
    log("app-server spawned");
    let config = handshake(&rpc).await.map_err(rpc_failure).map_err(failed("handshake"))?;
    log("handshake ok");
    let directory = std::env::temp_dir().join(format!("hangar-voice-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&directory);
    let mut start = json!({"ephemeral": true, "cwd": directory, "sandbox": "read-only", "approvalPolicy": "never",
        "environments": [], "baseInstructions": ORGANIZER_PROMPT, "developerInstructions": options.context,
        "config": thread_config(&config), "dynamicTools": tools()});
    if let Some(model) = config["model"].as_str() { start["model"] = json!(model); }
    let thread = rpc.request("thread/start", start).await.map_err(rpc_failure).map_err(failed("thread/start"))?["thread"]["id"].as_str().unwrap_or_default().to_owned();
    log(format!("thread started id={thread}"));

    let offer = tokio::task::spawn_blocking(rtc::offer).await.map_err(|_| VoiceFailure::Network)
        .and_then(|offer| offer.map_err(rtc_failure)).map_err(failed("rtc offer"))?;
    let mut realtime = json!({"threadId": thread, "version": "v3", "outputModality": "audio", "prompt": VOICE_PROMPT,
        "includeStartupContext": false, "delegationAckFiller": false, "clientManagedHandoffs": false,
        "realtimeStartInstructions": ORGANIZER_PROMPT, "initialItems": [{"role": "developer", "text": options.context}],
        "transport": {"type": "webrtc", "sdp": offer.sdp.clone()}});
    if let Some(voice) = &options.voice { realtime["voice"] = json!(voice); }
    rpc.request("thread/realtime/start", realtime).await.map_err(rpc_failure).map_err(failed("thread/realtime/start"))?;
    log(format!("realtime/start sent offer_bytes={}", offer.sdp.len()));

    // A resposta SDP chega como notificação; até lá, só ela interessa.
    let mut last_delta = None;
    let answer = tokio::time::timeout(Duration::from_secs(20), async {
        while let Ok(item) = incoming.recv().await {
            match item {
                Incoming::Notification { method, params } if method == "thread/realtime/sdp" => return Ok(params["sdp"].as_str().unwrap_or_default().to_owned()),
                Incoming::Notification { method, params } if method == "thread/realtime/error" =>
                    return Err(VoiceFailure::Realtime(params["message"].as_str().unwrap_or_default().to_owned())),
                Incoming::Notification { method, params } => {
                    let ours = params["threadId"].as_str() == Some(thread.as_str());
                    log_notification(&method, &params, ours, &mut last_delta);
                }
                Incoming::Request { method, .. } => log(format!("request before sdp {method} (unanswered)")),
                Incoming::Exited => return Err(VoiceFailure::AppServer),
            }
        }
        Err(VoiceFailure::AppServer)
    }).await.map_err(|_| VoiceFailure::Timeout).and_then(|answer| answer).map_err(failed("sdp answer"))?;
    log(format!("sdp answer received bytes={}", answer.len()));

    let (rtc_tx, rtc_rx) = async_channel::unbounded();
    let peer = rtc::run(offer, answer, muted.clone(), rtc_tx, stopped.clone());
    log("rtc thread started");
    let mut greeted = false;
    let mut results = Results::default();
    let mut gate: SendGate<Value> = SendGate::default();
    let mut organizer_busy = false;
    let mut spoken = SpokenTurns::default();
    let mut activity = Activity::Idle;
    let outcome = loop {
        if let Some((id, request)) = gate.due(Instant::now()) {
            log(format!("gate sent words={}", request.split_whitespace().count()));
            let _ = events.send(VoiceEvent::Draft(None)).await;
            let _ = events.send(VoiceEvent::Send(CallId(id), request)).await;
        }
        tokio::select! {
            event = rtc_rx.recv() => match event {
                Ok(rtc::RtcEvent::Connected) => {
                    log("phase Live");
                    let _ = events.send(VoiceEvent::Phase(Phase::Live)).await;
                    // A voz V3 nunca fala primeiro: sem isto a pessoa não tem prova de que o alto-falante funciona.
                    if !greeted {
                        greeted = true;
                        let spoke = rpc.request("thread/realtime/appendSpeech", json!({"threadId": thread, "text": "Conectado. Pode falar."})).await;
                        log(format!("greeting appendSpeech ok={}", spoke.is_ok()));
                    }
                }
                Ok(rtc::RtcEvent::Levels(i, o)) => {
                    if i >= MIC_VOICE_LEVEL { gate.heard_voice(Instant::now()); }
                    let _ = events.try_send(VoiceEvent::Levels(i, o));
                }
                Ok(rtc::RtcEvent::Failed(error)) => { log(format!("rtc failed error={error:?}")); break Err(failed("rtc")(rtc_failure(error))); }
                Ok(rtc::RtcEvent::Closed) | Err(_) => { log("rtc closed"); break Ok(()); }
            },
            item = incoming.recv() => match item {
                Ok(Incoming::Request { id, method, params }) if method == "item/tool/call" => {
                    let tool = params["tool"].as_str().unwrap_or("?").to_owned();
                    let outcome = match parse_tool(&params) {
                        ToolCall::ReadSession => { let _ = events.send(VoiceEvent::ReadSession(CallId(id))).await; "read" }
                        ToolCall::Send(_) | ToolCall::Hold(_) if !spoken.allows(&params) => {
                            let _ = rpc.respond(id, tool_reply("Pedido recusado: só uma fala do usuário pode gerar envio.", false)).await;
                            "refused-not-spoken"
                        }
                        ToolCall::Send(request) => match gate.offer(id, request, Instant::now()) {
                            Err((id, why)) => { let _ = rpc.respond(id, tool_reply(why, false)).await; "refused-short" }
                            Ok(Some((old, _))) => { let _ = rpc.respond(old, tool_reply("Substituído por um pedido mais recente; nada foi enviado.", false)).await; "offered-superseded" }
                            Ok(None) => "offered",
                        },
                        ToolCall::Hold(request) => {
                            // "Segura" dentro da janela de 1,5 s cancela o envio que ainda não saiu.
                            let cancelled = gate.user_spoke().map(|(old, _)| old);
                            let outcome = if cancelled.is_some() { "held-cancelled" } else { "held" };
                            if let Some(old) = cancelled {
                                let _ = rpc.respond(old, tool_reply("Cancelado: o usuário pediu para segurar; nada foi enviado.", false)).await;
                            }
                            let _ = events.send(VoiceEvent::Draft(Some(request.clone()))).await;
                            let _ = rpc.respond(id, tool_reply(format!("Segurado, nada enviado: {request}. Envie com send_to_session quando o usuário liberar."), true)).await;
                            outcome
                        }
                        ToolCall::Discard => {
                            if let Some((old, _)) = gate.user_spoke() {
                                let _ = rpc.respond(old, tool_reply("Cancelado: o usuário desistiu; nada foi enviado.", false)).await;
                            }
                            let _ = events.send(VoiceEvent::Draft(None)).await;
                            let _ = rpc.respond(id, tool_reply("Rascunho descartado.", true)).await;
                            "discarded"
                        }
                        ToolCall::Unknown(name) => { let _ = rpc.respond(id, tool_reply(format!("Ferramenta inexistente: {name}"), false)).await; "unknown" }
                    };
                    log(format!("tool call {tool} outcome={outcome}"));
                }
                // Perguntas do organizador não têm tela: a resposta é pela voz.
                Ok(Incoming::Request { id, method, .. }) if method == "item/tool/requestUserInput" => {
                    log("request item/tool/requestUserInput answered empty");
                    let _ = rpc.respond(id, json!({"answers": {}})).await;
                }
                Ok(Incoming::Request { id, method, .. }) => { log(format!("request {method} answered empty")); let _ = rpc.respond(id, json!({})).await; }
                Ok(Incoming::Notification { method, params }) => {
                    let ours = params["threadId"].as_str() == Some(thread.as_str());
                    log_notification(&method, &params, ours, &mut last_delta);
                    if !ours { continue; }
                    // A transcrição da fala chega atrasada e cancelava o próprio pedido: só uma fala nova
                    // encaminhada (outro userMessage) prova que o usuário continuou.
                    let user_spoke = method == "item/started" && params["item"]["type"] == "userMessage";
                    // Só registra a fala; cancelar envio pendente continua só no started e no delta.
                    if method == "item/started" || method == "item/completed" { spoken.item_started(&params); }
                    if user_spoke && let Some((id, _)) = gate.user_spoke() {
                        log("gate cancelled: user kept talking");
                        let _ = rpc.respond(id, tool_reply("O usuário continuou falando; nada foi enviado. Monte o pedido com a fala completa.", false)).await;
                    }
                    let next = match method.as_str() {
                        // O turno só entra em SpokenTurns quando o userMessage chega, depois do turn/started.
                        "item/started" | "item/completed" if params["item"]["type"] == "userMessage" && (method == "item/started" || organizer_busy) && spoken.allows(&params) => Some(Activity::Thinking),
                        "item/started" if params["item"]["type"] == "webSearch" => Some(Activity::Searching),
                        "turn/completed" => Some(Activity::Idle),
                        _ => None,
                    };
                    if let Some(next) = next && next != activity {
                        activity = next;
                        log(format!("activity {activity:?}"));
                        let _ = events.send(VoiceEvent::Activity(activity)).await;
                    }
                    match method.as_str() {
                        "turn/started" => { organizer_busy = true; results.turn_started(); }
                        "turn/completed" => {
                            organizer_busy = false;
                            spoken.turn_completed(&params);
                            // Turno interrompido ou falho não pode deixar um envio esperando a janela de 1,5 s.
                            if params["turn"]["status"] != "completed" && let Some((id, _)) = gate.user_spoke() {
                                log("gate cancelled: turn interrupted");
                                let _ = rpc.respond(id, tool_reply("O turno foi interrompido; nada foi enviado.", false)).await;
                            }
                            if params["turn"]["status"] == "failed" {
                                log(format!("organizer failure: {:?}", VoiceFailure::Organizer));
                                let _ = events.send(VoiceEvent::Failed(VoiceFailure::Organizer)).await;
                            }
                            if let Some(input) = results.turn_completed() { start_summary(&rpc, &thread, input, &mut results, organizer_busy).await; }
                        }
                        "item/completed" if params["item"]["type"] == "agentMessage" && params["item"]["phase"] != "commentary" => {
                            if results.take_summary(params["turnId"].as_str().unwrap_or_default())
                                && let Some(text) = params["item"]["text"].as_str() {
                                let spoke = rpc.request("thread/realtime/appendSpeech", json!({"threadId": thread, "text": text})).await;
                                log(format!("summary appendSpeech bytes={} ok={}", text.len(), spoke.is_ok()));
                            }
                        }
                        "thread/realtime/error" => break Err(failed("realtime")(VoiceFailure::Realtime(params["message"].as_str().unwrap_or_default().to_owned()))),
                        "thread/realtime/closed" => break Ok(()),
                        _ => {}
                    }
                }
                Ok(Incoming::Exited) | Err(_) => break Err(failed("app-server exited")(VoiceFailure::AppServer)),
            },
            command = inbox.recv() => match command {
                Some(Command::Reply(id, reply)) => { log(format!("tool reply success={}", reply["success"])); let _ = rpc.respond(id, reply).await; }
                Some(Command::Retarget(name, context)) => {
                    log("retarget");
                    let _ = rpc.request("thread/realtime/appendText", json!({"threadId": thread, "role": "developer", "text": context})).await;
                    let _ = rpc.request("thread/realtime/appendSpeech", json!({"threadId": thread, "text": format!("Agora estou na sessão {name}.")})).await;
                }
                Some(Command::Result(session, text)) => {
                    log(format!("session result bytes={}", text.len()));
                    if let Some(input) = results.push(session, text) { start_summary(&rpc, &thread, input, &mut results, organizer_busy).await; }
                }
                None => break Ok(()),
            },
            // Acorda para soltar o envio que assentou.
            _ = tokio::time::sleep(Duration::from_millis(200)) => {}
        }
    };
    stopped.store(true, Ordering::Relaxed);
    let _ = rpc.request("thread/realtime/stop", json!({"threadId": thread})).await;
    let _ = tokio::task::spawn_blocking(move || peer.join()).await;
    let _ = std::fs::remove_dir_all(&directory);
    outcome
}

async fn start_summary(rpc: &Rpc, thread: &str, first: Value, results: &mut Results, organizer_busy: bool) {
    let mut next = Some(first);
    // Laço, não recursão: um resumo recusado com o organizador ocioso solta o próximo da fila na hora.
    while let Some(mut input) = next.take() {
        input["threadId"] = json!(thread);
        match rpc.request("turn/start", input).await {
            Ok(result) => { if let Some(turn) = result["turn"]["id"].as_str() { results.mark_summary(turn.to_owned()); } }
            Err(error) => {
                log(format!("summary turn/start failed: {error:?} organizer_busy={organizer_busy}"));
                // Ocioso e recusado: nenhum turn/completed virá; fala o começo do texto e drena a fila.
                if let Some(text) = results.turn_start_failed(organizer_busy) {
                    let short: String = text.chars().take(400).collect();
                    let _ = rpc.request("thread/realtime/appendSpeech", json!({"threadId": thread, "text": format!("A sessão respondeu: {short}")})).await;
                    next = results.turn_completed();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn deltas_log_once_per_speaker_turn() {
        let mut last = None;
        let delta = "thread/realtime/transcript/delta";
        assert!(first_delta(&mut last, delta, "user"));
        assert!(!first_delta(&mut last, delta, "user"));
        assert!(first_delta(&mut last, delta, "assistant"), "troca de falante loga");
        assert!(first_delta(&mut last, delta, "user"));
        assert!(first_delta(&mut last, "turn/started", ""));
        assert!(first_delta(&mut last, delta, "user"), "turno novo loga de novo");
        assert!(first_delta(&mut last, "item/started", ""), "não-delta sempre loga");
    }
}
