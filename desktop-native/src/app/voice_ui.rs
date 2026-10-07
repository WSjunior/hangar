//! Voz nativa na barra de cima: a pílula, o painel e a ponte entre a chamada (`crate::voice`) e a sessão na tela.
use super::*;
use std::collections::VecDeque;
use gpui_kit::component::{select::{Select, SelectEvent, SelectState}, searchable_list::SearchableListItem};
use crate::voice::{CallId, Phase, Voice, VoiceEvent, VoiceFailure, VoiceOptions, rpc::Codex, organizer::{session_context, tool_reply}};

/// Vozes do Realtime; vazio é o padrão do Codex.
const VOICES: [&str; 19] = ["alloy", "arbor", "ash", "ballad", "breeze", "cedar", "coral", "cove", "echo", "ember", "juniper", "maple",
    "marin", "sage", "shimmer", "sol", "spruce", "vale", "verse"];

#[derive(Clone)]
pub(super) struct VoiceChoice { id: String }

impl SearchableListItem for VoiceChoice {
    type Value = String;
    fn title(&self) -> SharedString { if self.id.is_empty() { tr_shared("codex_voice_default", &[]).into() } else { self.id.clone().into() } }
    fn value(&self) -> &String { &self.id }
}

#[derive(Default)]
pub(super) struct VoiceUi {
    pub(super) enabled: bool,
    pub(super) codex: Option<Codex>,
    pub(super) call: Option<Voice>,
    /// Sobe a cada chamada nova ou parada: eventos de uma chamada velha não mexem na atual.
    pub(super) generation: u64,
    pub(super) phase: Option<Phase>,
    pub(super) levels: (f32, f32),
    pub(super) draft: Option<String>,
    pub(super) error: Option<String>,
    pub(super) muted: bool,
    pub(super) voice: Option<String>,
    pub(super) open: bool,
    pub(super) target: Option<String>,
    /// Ids de evento já falados: o mesmo texto em outro turno é outra resposta.
    pub(super) spoken: HashSet<String>,
    /// Sessão aberta cujo turno acabou antes de a resposta chegar.
    pub(super) reply_pending: Option<SessionKey>,
    pub(super) pending_sends: VecDeque<(SessionKey, String, CallId)>,
    /// Sessão que recebeu pedido da voz → já foi vista trabalhando.
    pub(super) watched: HashMap<SessionKey, bool>,
    pub(super) voice_select: Option<(Entity<SelectState<Vec<VoiceChoice>>>, Subscription)>,
}

pub(super) fn conversation_pairs(events: &[ChatEvent]) -> Vec<(String, String)> {
    events.iter().filter(|e| matches!(e.kind.as_str(), "user_msg" | "assistant_msg") && e.text.as_deref().is_some_and(|t| !t.trim().is_empty()))
        .map(|e| (e.kind.as_str().to_owned(), e.text.clone().unwrap_or_default())).collect()
}

/// `(id, kind, text)` → `(id da última resposta, respostas depois da última fala do usuário)`.
pub(super) fn last_reply(events: &[(String, String, String)]) -> Option<(String, String)> {
    let start = events.iter().rposition(|(_, kind, _)| kind == "user_msg").map(|i| i + 1).unwrap_or(0);
    let replies: Vec<&(String, String, String)> = events[start..].iter().filter(|(_, k, _)| k == "assistant_msg").collect();
    let last = replies.last()?;
    Some((last.0.clone(), replies.iter().map(|(_, _, t)| t.as_str()).collect::<Vec<_>>().join("\n\n")))
}

pub(super) fn went_idle(was_working: bool, state: &str) -> bool { was_working && state != "working" }

pub(super) fn send_reply(session: &str, result: &Result<Delivery, Failure>) -> Value {
    match result {
        Ok(d) if d.ok && d.delivered => tool_reply(format!("status sent: pedido entregue à sessão {session}. Aguarde o resultado real."), true),
        Ok(d) if d.ok => tool_reply(format!("status queued: a sessão {session} está ocupada; o pedido entrou na fila."), true),
        _ => tool_reply(format!("status failed: não foi possível confirmar a entrega à sessão {session}. Não reenvie; peça para o usuário conferir o chat."), false),
    }
}

fn triples(events: &[ChatEvent]) -> Vec<(String, String, String)> {
    events.iter().filter(|e| matches!(e.kind.as_str(), "user_msg" | "assistant_msg"))
        .map(|e| (e.id.clone(), e.kind.as_str().to_owned(), e.text.clone().unwrap_or_default())).collect()
}

fn failure_text(failure: &VoiceFailure) -> String {
    match failure {
        VoiceFailure::Microphone => tr_shared("composer_sem_acesso_mic", &[]),
        VoiceFailure::Speaker => tr("voice_speaker"),
        VoiceFailure::AppServer => tr("voice_app_server"),
        VoiceFailure::Realtime(detail) => format!("{} {detail}", tr_shared("codex_voice_failed", &[])),
        VoiceFailure::Network => tr("voice_network"),
        VoiceFailure::Timeout => tr_shared("codex_voice_timeout", &[]),
        VoiceFailure::Organizer => tr("voice_organizer"),
    }
}

fn voice_file() -> Option<std::path::PathBuf> { Some(appearance::dir()?.join("voice.json")) }

fn read_saved_voice() -> Option<String> {
    let value: Value = serde_json::from_slice(&std::fs::read(voice_file()?).ok()?).ok()?;
    value["voice"].as_str().filter(|v| !v.is_empty()).map(str::to_owned)
}

fn save_voice(voice: Option<&str>) -> Result<(), String> {
    let path = voice_file().ok_or_else(|| tr("keyboard_no_directory"))?;
    let dir = path.parent().ok_or_else(|| tr("keyboard_no_directory"))?;
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(&json!({"voice": voice})).map_err(|error| error.to_string())?;
    std::fs::write(&temporary, bytes).and_then(|_| std::fs::rename(&temporary, &path)).map_err(|error| error.to_string())
}

/// Nome curto para a pílula.
fn short(name: &str) -> String {
    if name.chars().count() <= 18 { return name.to_owned(); }
    format!("{}…", name.chars().take(17).collect::<String>())
}

impl Hangar {
    /// O servidor desta máquina: a voz usa o Codex daqui, e a opção beta é dele.
    pub(super) fn local_api(&self) -> Option<Api> {
        if let Some(api) = self.api.as_ref().filter(|api| api.is_loopback()) { return Some(api.clone()); }
        self.servers.iter().filter(|s| !s.disabled).find_map(|s| Api::new(&s.address, &s.token).ok().filter(Api::is_loopback))
    }

    pub(super) fn refresh_voice_gate(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.local_api() else {
            self.voice.enabled = false;
            // Sem a pílula, a chamada ficaria com o microfone aberto e nenhum controle na tela.
            self.stop_voice(cx);
            return;
        };
        let (connection, tx) = (self.connection, self.tx.clone());
        self.runtime.spawn(async move {
            // find_codex roda `npm prefix -g`: fora da thread da tela.
            let codex = tokio::task::spawn_blocking(crate::voice::rpc::find_codex).await.ok().flatten();
            let saved = tokio::task::spawn_blocking(read_saved_voice).await.ok().flatten();
            let enabled = api.server_read(&["harness", "codex", "opcoes"], &[], 8).await.ok()
                .and_then(|v| v["codex_voice_beta"].as_bool()).unwrap_or(false);
            let _ = tx.send(Envelope { connection, selection: None, payload: Payload::VoiceGate(enabled, codex, saved) }).await;
        });
    }

    pub(super) fn receive_voice_gate(&mut self, enabled: bool, codex: Option<Codex>, saved: Option<String>, cx: &mut Context<Self>) {
        (self.voice.enabled, self.voice.codex) = (enabled, codex);
        if self.voice.call.is_none() { self.voice.voice = saved; }
        if !enabled && self.voice.call.is_some() { self.stop_voice(cx); }
        cx.notify();
    }

    pub(super) fn voice_context(&self) -> String {
        let name = self.selected.as_ref().map_or("", |s| s.name.as_str());
        session_context(name, &conversation_pairs(&self.chat.events))
    }

    pub(super) fn start_voice(&mut self, cx: &mut Context<Self>) {
        let Some(codex) = self.voice.codex.clone() else { self.voice.error = Some(tr("voice_no_codex")); cx.notify(); return; };
        if self.dictation.recording() { self.voice.error = Some(tr("voice_dictation_busy")); cx.notify(); return; }
        let (events_tx, events) = async_channel::unbounded();
        let options = VoiceOptions { codex, voice: self.voice.voice.clone(), context: self.voice_context() };
        self.voice.generation += 1;
        self.voice.call = Some(Voice::start(self.runtime.handle(), options, events_tx));
        self.voice.target = self.selected.as_ref().map(|s| s.name.clone());
        self.voice.spoken.clear();
        self.voice.reply_pending = None;
        self.voice.pending_sends.clear();
        self.voice.watched.clear();
        self.voice.error = None;
        self.voice.muted = false;
        self.voice.draft = None;
        self.voice.levels = (0., 0.);
        self.voice.phase = Some(Phase::Connecting);
        let (generation, connection, tx) = (self.voice.generation, self.connection, self.tx.clone());
        self.runtime.spawn(async move {
            while let Ok(event) = events.recv().await {
                if tx.send(Envelope { connection, selection: None, payload: Payload::Voice(generation, event) }).await.is_err() { break; }
            }
        });
        cx.notify();
    }

    pub(super) fn stop_voice(&mut self, cx: &mut Context<Self>) {
        if let Some(mut call) = self.voice.call.take() { call.stop(); }
        self.voice.generation += 1; // eventos atrasados da chamada parada não mexem na próxima
        self.voice.phase = None;
        self.voice.draft = None;
        self.voice.levels = (0., 0.);
        self.voice.pending_sends.clear();
        cx.notify();
    }

    fn voice_reply(&self, call: CallId, reply: Value) {
        if let Some(voice) = &self.voice.call { voice.reply(call, reply); }
    }

    pub(super) fn receive_voice(&mut self, generation: u64, event: VoiceEvent, cx: &mut Context<Self>) {
        if generation != self.voice.generation { return; }
        match event {
            VoiceEvent::Phase(Phase::Closed) => {
                self.voice.call = None;
                self.voice.phase = None;
                self.voice.draft = None;
                self.voice.levels = (0., 0.);
                self.voice.pending_sends.clear();
            }
            VoiceEvent::Phase(phase) => self.voice.phase = Some(phase),
            VoiceEvent::Levels(input, output) => self.voice.levels = (input, output),
            VoiceEvent::Draft(draft) => self.voice.draft = draft,
            VoiceEvent::Failed(failure) => { self.voice.error = Some(failure_text(&failure)); self.voice.open = true; }
            VoiceEvent::ReadSession(call) => self.voice_reply(call, tool_reply(self.voice_context(), true)),
            VoiceEvent::Send(call, request) => {
                // A sessão é a da tela neste instante, não a de quando a fala começou.
                let Some(key) = self.selected_key() else {
                    self.voice_reply(call, tool_reply("Nenhuma sessão aberta na tela; nada foi enviado.", false));
                    cx.notify();
                    return;
                };
                if self.api_for(&key.server).is_none() {
                    self.voice_reply(call, send_reply(&key.name, &Err(Failure::local(tr("server_changed")))));
                    cx.notify();
                    return;
                }
                let was_working = self.chat.state.state == "working";
                self.voice.watched.insert(key.clone(), was_working);
                let known = self.known_user_ids();
                if self.post(key.clone(), request.clone(), request.clone(), false, known, None, cx) {
                    self.voice.pending_sends.push_back((key, request, call));
                } else {
                    let name = key.name.clone();
                    self.delivery.hold(key, request, false, None);
                    self.voice_reply(call, tool_reply(format!("status queued: a sessão {name} tem outro envio em andamento; o pedido entrou na fila."), true));
                }
            }
        }
        cx.notify();
    }

    pub(super) fn voice_sent(&mut self, key: &SessionKey, text: &str, result: &Result<Delivery, Failure>) {
        let Some(index) = self.voice.pending_sends.iter().position(|(k, t, _)| k == key && t == text) else { return };
        let Some((_, _, call)) = self.voice.pending_sends.remove(index) else { return };
        self.voice_reply(call, send_reply(&key.name, result));
    }

    pub(super) fn voice_session_opened(&mut self) {
        if self.voice.call.is_none() { return; }
        let name = self.selected.as_ref().map(|s| s.name.clone());
        if name == self.voice.target { return; }
        self.voice.target = name.clone();
        // A resposta atrasada da sessão que saiu da tela passa a vir pela lista e pelo histórico.
        if let Some(key) = self.voice.reply_pending.take() { self.voice.watched.insert(key, true); }
        let Some(voice) = &self.voice.call else { return };
        let name = name.unwrap_or_default();
        // O chat novo ainda está vazio aqui; o organizador lê o resto pelo read_session.
        voice.retarget(name.clone(), format!("A sessão na tela agora é {name}."));
    }

    pub(super) fn voice_turn_finished(&mut self, state: &str) {
        if self.voice.call.is_none() { return; }
        if let Some(key) = self.selected_key() { self.voice.watched.remove(&key); }
        if state == "awaiting_input" {
            let name = self.voice.target.clone().unwrap_or_default();
            if let Some(voice) = &self.voice.call { voice.session_result(name, "A sessão está esperando uma resposta ou aprovação no chat.".into()); }
            return;
        }
        // O estado pode chegar antes do último assistant_msg: se a resposta ainda não está aqui, espera por ela.
        if !self.voice_speak_last_reply() { self.voice.reply_pending = self.selected_key(); }
    }

    /// Fala a última resposta da sessão aberta se ainda não foi falada; devolve se havia uma nova.
    fn voice_speak_last_reply(&mut self) -> bool {
        let Some((id, text)) = last_reply(&triples(&self.chat.events)) else { return false };
        if !self.voice.spoken.insert(id) { return false; }
        let name = self.voice.target.clone().unwrap_or_default();
        if let Some(voice) = &self.voice.call { voice.session_result(name, text); }
        true
    }

    /// Chamado a cada `assistant_msg` da sessão aberta.
    pub(super) fn voice_message(&mut self) {
        if self.voice.reply_pending.is_some() && self.chat.state.state != "working" && self.voice_speak_last_reply() {
            self.voice.reply_pending = None;
        }
    }

    /// Sessão que recebeu pedido e não está na tela: a lista diz quando o turno dela acabou, e a resposta vem do histórico.
    pub(super) fn voice_sessions(&mut self) {
        if self.voice.call.is_none() || self.voice.watched.is_empty() { return; }
        let open = self.selected_key();
        let rows: Vec<(SessionKey, String)> = self.voice.watched.keys()
            .filter(|key| Some(*key) != open.as_ref()) // a aberta é tratada pelo SSE
            .filter_map(|key| self.sessions_of(&key.server).iter().find(|s| s.name == key.name).map(|s| (key.clone(), s.state.clone())))
            .collect();
        let mut finished = Vec::new();
        for (key, state) in rows {
            let Some(was_working) = self.voice.watched.get_mut(&key) else { continue };
            if state == "working" { *was_working = true; } else if went_idle(*was_working, &state) { finished.push(key); }
        }
        for key in finished {
            // Sessão de outra máquina que não é a aberta nem a ativa: a conexão é a da lista dela.
            let api = self.api_for(&key.server).or_else(|| self.remote.get(&servers::norm(&key.server)).and_then(|l| l.api.clone()));
            // Sem conexão agora, fica vigiada e a próxima lista tenta de novo.
            let Some(api) = api else { continue };
            self.voice.watched.remove(&key);
            let (generation, connection, tx) = (self.voice.generation, self.connection, self.tx.clone());
            self.runtime.spawn(async move {
                let result = api.history(&key.name, 20, None).await;
                let _ = tx.send(Envelope { connection, selection: None, payload: Payload::VoiceHistory(generation, key, result) }).await;
            });
        }
    }

    pub(super) fn voice_history(&mut self, generation: u64, key: SessionKey, result: Result<api::History, Failure>) {
        if generation != self.voice.generation { return; }
        let Some(events) = result.ok().and_then(|history| history.events) else { return };
        let Some((id, text)) = last_reply(&triples(&events)) else { return };
        if !self.voice.spoken.insert(id) { return; }
        if let Some(voice) = &self.voice.call { voice.session_result(key.name, text); }
    }

    fn toggle_voice_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open = !self.voice.open;
        self.close_popups();
        self.voice.open = open;
        if open && self.voice.voice_select.is_none() {
            let items: Vec<VoiceChoice> = std::iter::once(String::new()).chain(VOICES.iter().map(|v| (*v).to_owned()))
                .map(|id| VoiceChoice { id }).collect();
            let at = items.iter().position(|c| Some(&c.id) == self.voice.voice.as_ref()).unwrap_or(0);
            let picker = cx.new(|cx| SelectState::new(items, Some(gpui_kit::component::IndexPath::new(at)), window, cx));
            let sub = cx.subscribe_in(&picker, window, |this: &mut Hangar, _, event: &SelectEvent<Vec<VoiceChoice>>, _, cx| {
                let SelectEvent::Confirm(Some(id)) = event else { return };
                this.voice.voice = (!id.is_empty()).then(|| id.clone());
                let saved = this.voice.voice.clone();
                let write = cx.background_executor().spawn(async move { save_voice(saved.as_deref()) });
                cx.spawn(async move |this, cx| {
                    if let Err(error) = write.await {
                        let _ = this.update(cx, |this, cx| {
                            this.voice.error = Some(format!("{} {error}", tr("voice_not_saved")));
                            cx.notify();
                        });
                    }
                }).detach();
                cx.notify();
            });
            self.voice.voice_select = Some((picker, sub));
        }
        cx.notify();
    }

    /// Pílula da barra de cima; `None` sem a opção beta ou sem Codex nesta máquina.
    pub(super) fn render_voice_pill(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.voice.enabled || self.voice.codex.is_none() { return None; }
        let title = tr_shared("codex_voice_title", &[]);
        let button = Button::new("topbar-voice").ghost().small().h(px(26.)).px(px(8.)).rounded_full().selected(self.voice.open)
            .accessibility_label(title.clone())
            .on_click(cx.listener(|this, _, window, cx| this.toggle_voice_panel(window, cx)));
        let button = if self.voice.call.is_none() {
            button.tooltip(title).child(div().flex().items_center().gap(px(6.))
                .child(chrome::small_icon(IconName::Mic, 14., theme::muted()))
                .when(self.voice.error.is_some(), |el| el.child(div().size(px(6.)).rounded_full().bg(theme::danger()))))
        } else {
            let (input, output) = self.voice.levels;
            // Só a altura de um div muda: nada de transform.
            let bar = |level: f32| div().w(px(3.)).h(px(4. + level.clamp(0., 1.) * 12.)).rounded_full().bg(theme::accent());
            let target = self.voice.target.as_deref().map(short);
            button.child(div().flex().items_center().gap(px(6.)).text_size(px(12.5))
                .child(div().h(px(16.)).flex().items_center().gap(px(2.)).child(bar(input)).child(bar(output)))
                .child(div().text_color(theme::muted()).child(self.voice_status()))
                .children(target.map(|name| div().text_color(theme::faint()).child(name)))
                .when(self.voice.draft.is_some(), |el| el.child(div().size(px(6.)).rounded_full().bg(theme::warning())))
                .child(beta_badge()))
        };
        Some(popup::anchor(div(), "topbar-voice").child(button).into_any_element())
    }

    fn voice_status(&self) -> String {
        match &self.voice.phase {
            Some(Phase::Live) if self.voice.muted => tr_shared("codex_voice_muted", &[]),
            Some(Phase::Live) if self.voice.levels.1 > 0.02 => tr("voice_speaking"),
            Some(Phase::Live) => tr_shared("codex_voice_listening", &[]),
            _ => tr_shared("codex_voice_connecting", &[]),
        }
    }

    /// Conteúdo cru do painel: o `render_popup` já põe a superfície.
    pub(super) fn render_voice_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let live = self.voice.call.is_some();
        let mut body = div().flex().flex_col().gap(px(12.)).p(px(16.))
            .child(div().flex().items_center().gap(px(8.))
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).text_color(theme::text()).child(tr_shared("codex_voice_title", &[])))
                .child(beta_badge()));
        if live {
            let status = match self.voice.target.as_deref() {
                Some(name) => format!("{} · {name}", self.voice_status()),
                None => self.voice_status(),
            };
            body = body.child(div().text_xs().text_color(theme::muted()).child(status));
        }
        if let Some((picker, _)) = &self.voice.voice_select {
            body = body.child(div().flex().items_center().justify_between().gap(px(12.))
                .child(div().text_xs().text_color(theme::muted()).child(tr_shared("codex_voice_label", &[])))
                .child(div().w(px(200.)).child(Select::new(picker).small().disabled(live).accessibility_label(tr_shared("codex_voice_label", &[])))));
        }
        if let Some(draft) = &self.voice.draft {
            body = body.child(div().flex().flex_col().gap(px(4.)).p(px(10.)).rounded(px(8.)).border_1().border_color(theme::warning())
                .child(div().text_xs().text_color(theme::warning()).child(tr("voice_draft_held")))
                .child(div().text_sm().text_color(theme::text()).whitespace_normal().child(draft.clone())));
        }
        if let Some(error) = &self.voice.error {
            body = body.child(div().text_xs().text_color(theme::danger()).whitespace_normal().child(error.clone()));
        }
        let actions = if live {
            let mute = if self.voice.muted { "codex_voice_unmute_short" } else { "codex_voice_mute_short" };
            div().flex().justify_end().gap(px(8.))
                .child(Button::new("voice-mute").ghost().small().label(tr_shared(mute, &[]))
                    .disabled(!matches!(self.voice.phase, Some(Phase::Live)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.voice.muted = !this.voice.muted;
                        if let Some(call) = &this.voice.call { call.set_muted(this.voice.muted); }
                        cx.notify();
                    })))
                .child(Button::new("voice-stop").danger().small().label(tr_shared("codex_voice_stop_short", &[]))
                    .on_click(cx.listener(|this, _, _, cx| this.stop_voice(cx))))
        } else {
            div().flex().justify_end().child(Button::new("voice-connect").primary().small().label(tr_shared("codex_voice_connect_short", &[]))
                .on_click(cx.listener(|this, _, _, cx| this.start_voice(cx))))
        };
        body.child(actions).into_any_element()
    }
}

fn beta_badge() -> Div {
    div().flex_shrink_0().px(px(5.)).rounded(px(4.)).border_1().border_color(theme::border()).text_size(px(10.)).text_color(theme::faint())
        .child(tr_shared("comum_beta", &[]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    fn ev(id: &str, kind: &str, text: &str) -> (String, String, String) { (id.into(), kind.into(), text.into()) }

    #[test]
    fn last_reply_is_after_last_user_message() {
        let events = vec![ev("1", "user_msg", "a"), ev("2", "assistant_msg", "velha"), ev("3", "user_msg", "b"),
            ev("4", "assistant_msg", "nova 1"), ev("5", "assistant_msg", "nova 2")];
        assert_eq!(last_reply(&events), Some(("5".into(), "nova 1\n\nnova 2".into())));
        assert_eq!(last_reply(&[ev("1", "user_msg", "só pergunta")]), None);
    }

    #[test]
    fn equal_texts_with_different_ids_are_both_spoken() {
        // Dedup é por id: "Pronto." duas vezes em turnos diferentes fala duas vezes.
        let first = last_reply(&[ev("1", "user_msg", "a"), ev("2", "assistant_msg", "Pronto.")]).unwrap();
        let second = last_reply(&[ev("3", "user_msg", "b"), ev("4", "assistant_msg", "Pronto.")]).unwrap();
        assert_ne!(first.0, second.0);
    }

    #[test]
    fn send_targets_session_on_screen_at_send_time() {
        let ok = send_reply("demo-session", &Ok(Delivery { ok: true, delivered: true }));
        let text = ok["contentItems"][0]["text"].as_str().unwrap();
        assert!(text.contains("demo-session") && text.contains("sent"));
        let queued = send_reply("demo-session", &Ok(Delivery { ok: true, delivered: false }));
        assert!(queued["contentItems"][0]["text"].as_str().unwrap().contains("queued"));
    }

    #[test]
    fn went_idle_only_from_working() {
        assert!(went_idle(true, "idle"));
        assert!(went_idle(true, "awaiting_input"));
        assert!(!went_idle(false, "idle"));
        assert!(!went_idle(true, "working"));
    }
}
