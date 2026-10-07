//! Conversa por voz: o Codex local fala, a sessão aberta na tela trabalha.
pub mod audio;
pub mod organizer;
pub mod rpc;
pub mod rtc;

use organizer::{Results, SendGate, SpokenTurns, ToolCall, parse_tool, tool_reply, tools, thread_config, ORGANIZER_PROMPT, VOICE_PROMPT};
use rpc::{Codex, Incoming, Rpc, RpcError, handshake};
use serde_json::{Value, json};
use std::{sync::{Arc, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use tokio::{runtime::Handle, sync::{Notify, mpsc}};

pub struct CallId(Value);
pub enum Phase { Connecting, Live, Closed }
#[derive(Debug, Clone)]
pub enum VoiceFailure { Microphone, Speaker, AppServer, Realtime(String), Network, Timeout, Organizer }
pub enum VoiceEvent { Phase(Phase), Levels(f32, f32), Draft(Option<String>), ReadSession(CallId), Send(CallId, String), Failed(VoiceFailure) }
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

async fn call(options: VoiceOptions, events: async_channel::Sender<VoiceEvent>, mut inbox: mpsc::UnboundedReceiver<Command>,
    muted: Arc<AtomicBool>, stopped: Arc<AtomicBool>, stop: Arc<Notify>) {
    let _ = events.send(VoiceEvent::Phase(Phase::Connecting)).await;
    // Parar vale em qualquer fase: largar o future derruba o app-server (kill_on_drop) e o flag para a thread do RTC.
    let outcome = tokio::select! {
        outcome = run_call(options, &events, &mut inbox, &muted, &stopped) => outcome,
        _ = stop.notified() => Ok(()),
    };
    stopped.store(true, Ordering::Relaxed);
    if let Err(failure) = outcome { let _ = events.send(VoiceEvent::Failed(failure)).await; }
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
    let (rpc, incoming) = Rpc::spawn(&options.codex).await.map_err(rpc_failure)?;
    let config = handshake(&rpc).await.map_err(rpc_failure)?;
    let directory = std::env::temp_dir().join(format!("hangar-voice-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&directory);
    let mut start = json!({"ephemeral": true, "cwd": directory, "sandbox": "read-only", "approvalPolicy": "never",
        "environments": [], "baseInstructions": ORGANIZER_PROMPT, "developerInstructions": options.context,
        "config": thread_config(&config), "dynamicTools": tools()});
    if let Some(model) = config["model"].as_str() { start["model"] = json!(model); }
    let thread = rpc.request("thread/start", start).await.map_err(rpc_failure)?["thread"]["id"].as_str().unwrap_or_default().to_owned();

    let offer = tokio::task::spawn_blocking(rtc::offer).await.map_err(|_| VoiceFailure::Network)?.map_err(rtc_failure)?;
    let mut realtime = json!({"threadId": thread, "version": "v3", "outputModality": "audio", "prompt": VOICE_PROMPT,
        "includeStartupContext": false, "delegationAckFiller": false, "clientManagedHandoffs": false,
        "realtimeStartInstructions": ORGANIZER_PROMPT, "initialItems": [{"role": "developer", "text": options.context}],
        "transport": {"type": "webrtc", "sdp": offer.sdp.clone()}});
    if let Some(voice) = &options.voice { realtime["voice"] = json!(voice); }
    rpc.request("thread/realtime/start", realtime).await.map_err(rpc_failure)?;

    // A resposta SDP chega como notificação; até lá, só ela interessa.
    let answer = tokio::time::timeout(Duration::from_secs(20), async {
        while let Ok(item) = incoming.recv().await {
            match item {
                Incoming::Notification { method, params } if method == "thread/realtime/sdp" => return Ok(params["sdp"].as_str().unwrap_or_default().to_owned()),
                Incoming::Notification { method, params } if method == "thread/realtime/error" =>
                    return Err(VoiceFailure::Realtime(params["message"].as_str().unwrap_or_default().to_owned())),
                Incoming::Exited => return Err(VoiceFailure::AppServer),
                _ => {}
            }
        }
        Err(VoiceFailure::AppServer)
    }).await.map_err(|_| VoiceFailure::Timeout)??;

    let (rtc_tx, rtc_rx) = async_channel::unbounded();
    let peer = rtc::run(offer, answer, muted.clone(), rtc_tx, stopped.clone());
    let mut results = Results::default();
    let mut gate: SendGate<Value> = SendGate::default();
    let mut organizer_busy = false;
    let mut spoken = SpokenTurns::default();
    let outcome = loop {
        if let Some((id, request)) = gate.due(Instant::now()) {
            let _ = events.send(VoiceEvent::Draft(None)).await;
            let _ = events.send(VoiceEvent::Send(CallId(id), request)).await;
        }
        tokio::select! {
            event = rtc_rx.recv() => match event {
                Ok(rtc::RtcEvent::Connected) => { let _ = events.send(VoiceEvent::Phase(Phase::Live)).await; }
                Ok(rtc::RtcEvent::Levels(i, o)) => { let _ = events.try_send(VoiceEvent::Levels(i, o)); }
                Ok(rtc::RtcEvent::Failed(error)) => break Err(rtc_failure(error)),
                Ok(rtc::RtcEvent::Closed) | Err(_) => break Ok(()),
            },
            item = incoming.recv() => match item {
                Ok(Incoming::Request { id, method, params }) if method == "item/tool/call" => match parse_tool(&params) {
                    ToolCall::ReadSession => { let _ = events.send(VoiceEvent::ReadSession(CallId(id))).await; }
                    ToolCall::Send(_) | ToolCall::Hold(_) if !spoken.allows(&params) => {
                        let _ = rpc.respond(id, tool_reply("Pedido recusado: só uma fala do usuário pode gerar envio.", false)).await;
                    }
                    ToolCall::Send(request) => match gate.offer(id, request, Instant::now()) {
                        Err((id, why)) => { let _ = rpc.respond(id, tool_reply(why, false)).await; }
                        Ok(Some((old, _))) => { let _ = rpc.respond(old, tool_reply("Substituído por um pedido mais recente; nada foi enviado.", false)).await; }
                        Ok(None) => {}
                    },
                    ToolCall::Hold(request) => {
                        // "Segura" dentro da janela de 1,5 s cancela o envio que ainda não saiu.
                        if let Some((old, _)) = gate.user_spoke() {
                            let _ = rpc.respond(old, tool_reply("Cancelado: o usuário pediu para segurar; nada foi enviado.", false)).await;
                        }
                        let _ = events.send(VoiceEvent::Draft(Some(request.clone()))).await;
                        let _ = rpc.respond(id, tool_reply(format!("Segurado, nada enviado: {request}. Envie com send_to_session quando o usuário liberar."), true)).await;
                    }
                    ToolCall::Discard => {
                        if let Some((old, _)) = gate.user_spoke() {
                            let _ = rpc.respond(old, tool_reply("Cancelado: o usuário desistiu; nada foi enviado.", false)).await;
                        }
                        let _ = events.send(VoiceEvent::Draft(None)).await;
                        let _ = rpc.respond(id, tool_reply("Rascunho descartado.", true)).await;
                    }
                    ToolCall::Unknown(name) => { let _ = rpc.respond(id, tool_reply(format!("Ferramenta inexistente: {name}"), false)).await; }
                },
                // Perguntas do organizador não têm tela: a resposta é pela voz.
                Ok(Incoming::Request { id, method, .. }) if method == "item/tool/requestUserInput" => {
                    let _ = rpc.respond(id, json!({"answers": {}})).await;
                }
                Ok(Incoming::Request { id, .. }) => { let _ = rpc.respond(id, json!({})).await; }
                Ok(Incoming::Notification { method, params }) => {
                    if params["threadId"].as_str() != Some(thread.as_str()) { continue; }
                    let user_spoke = (method == "thread/realtime/transcript/delta" && params["role"] == "user")
                        || (method == "item/started" && params["item"]["type"] == "userMessage");
                    // Só registra a fala; cancelar envio pendente continua só no started e no delta.
                    if method == "item/started" || method == "item/completed" { spoken.item_started(&params); }
                    if user_spoke && let Some((id, _)) = gate.user_spoke() {
                        let _ = rpc.respond(id, tool_reply("O usuário continuou falando; nada foi enviado. Monte o pedido com a fala completa.", false)).await;
                    }
                    match method.as_str() {
                        "turn/started" => { organizer_busy = true; results.turn_started(); }
                        "turn/completed" => {
                            organizer_busy = false;
                            spoken.turn_completed(&params);
                            // Turno interrompido ou falho não pode deixar um envio esperando a janela de 1,5 s.
                            if params["turn"]["status"] != "completed" && let Some((id, _)) = gate.user_spoke() {
                                let _ = rpc.respond(id, tool_reply("O turno foi interrompido; nada foi enviado.", false)).await;
                            }
                            if params["turn"]["status"] == "failed" { let _ = events.send(VoiceEvent::Failed(VoiceFailure::Organizer)).await; }
                            if let Some(input) = results.turn_completed() { start_summary(&rpc, &thread, input, &mut results, organizer_busy).await; }
                        }
                        "item/completed" if params["item"]["type"] == "agentMessage" && params["item"]["phase"] != "commentary" => {
                            if results.take_summary(params["turnId"].as_str().unwrap_or_default())
                                && let Some(text) = params["item"]["text"].as_str() {
                                let _ = rpc.request("thread/realtime/appendSpeech", json!({"threadId": thread, "text": text})).await;
                            }
                        }
                        "thread/realtime/error" => break Err(VoiceFailure::Realtime(params["message"].as_str().unwrap_or_default().to_owned())),
                        "thread/realtime/closed" => break Ok(()),
                        _ => {}
                    }
                }
                Ok(Incoming::Exited) | Err(_) => break Err(VoiceFailure::AppServer),
            },
            command = inbox.recv() => match command {
                Some(Command::Reply(id, reply)) => { let _ = rpc.respond(id, reply).await; }
                Some(Command::Retarget(name, context)) => {
                    let _ = rpc.request("thread/realtime/appendText", json!({"threadId": thread, "role": "developer", "text": context})).await;
                    let _ = rpc.request("thread/realtime/appendSpeech", json!({"threadId": thread, "text": format!("Agora estou na sessão {name}.")})).await;
                }
                Some(Command::Result(session, text)) => {
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
            Err(_) => {
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
