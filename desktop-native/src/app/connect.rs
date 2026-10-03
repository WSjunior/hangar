//! Hangar Connect deste servidor: endereço público sem Tailscale, ligado por um código.
use super::*;
use super::device::Remote;
use super::server_config::chip;
use super::settings::Page;

#[derive(Clone, Debug)]
pub(super) struct Status { enabled: bool, url: Option<String>, error: Option<String>, running: bool }

fn parse_status(value: Value) -> Result<Status, String> {
    let enabled = value.get("enabled").and_then(Value::as_bool).ok_or_else(|| tr("invalid_response"))?;
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    let running = value.get("processes").and_then(Value::as_object)
        .is_some_and(|all| !all.is_empty() && all.values().all(|p| p.get("running").and_then(Value::as_bool) == Some(true)));
    Ok(Status { enabled, url: text("url"), error: text("error"), running })
}

#[derive(Default)]
pub(in crate::app) struct Connect {
    status: Remote<Status>,
    saving: bool,
    error: Option<String>,
    code: Option<Entity<InputState>>,
}

pub(super) enum ConnectReply {
    Loaded(u64, Result<Value, Failure>),
    Saved(Result<Value, Failure>),
}

impl Hangar {
    fn connect_send_later(&self) -> impl Fn(ConnectReply) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>> + Send + 'static {
        let (tx, connection) = (self.tx.clone(), self.connection);
        move |reply| {
            let tx = tx.clone();
            Box::pin(async move { let _ = tx.send(Envelope { connection, selection: None, payload: Payload::Connect(reply) }).await; })
        }
    }

    // O código guarda a chave do túnel: fora da página, o campo não fica montado.
    pub(super) fn connect_page_left(&mut self) { self.connect.code = None; }

    pub(super) fn connect_opened(&mut self, cx: &mut Context<Self>) {
        self.connect.error = None;
        self.load_connect(cx);
    }

    fn load_connect(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else { return };
        let seq = self.connect.status.start();
        let done = self.connect_send_later();
        self.runtime.spawn(async move { done(ConnectReply::Loaded(seq, api.server_read(&["connect"], &[], 10).await)).await });
        cx.notify();
    }

    fn write_connect(&mut self, on: bool, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else { return };
        if self.connect.saving { return; }
        let code = self.connect.code.as_ref().map(|input| input.read(cx).value().trim().to_string()).unwrap_or_default();
        if on && code.is_empty() { self.connect.error = Some(tr("connect_code_required")); cx.notify(); return; }
        (self.connect.saving, self.connect.error) = (true, None);
        let done = self.connect_send_later();
        self.runtime.spawn(async move {
            // Ligar pode baixar o frpc e o Caddy na primeira vez: prazo longo.
            let result = if on { api.server_send(reqwest::Method::PUT, &["connect"], Some(json!({"code": code})), 180).await }
                else { api.server_send(reqwest::Method::DELETE, &["connect"], None, 30).await };
            done(ConnectReply::Saved(result)).await
        });
        cx.notify();
    }

    pub(super) fn receive_connect(&mut self, reply: ConnectReply, window: &mut Window, cx: &mut Context<Self>) {
        match reply {
            ConnectReply::Loaded(seq, result) => {
                let result = result.map_err(|error| Self::fetch_failure(&error)).and_then(parse_status);
                if !self.connect.status.finish(seq, result) { return; }
            }
            ConnectReply::Saved(result) => {
                self.connect.saving = false;
                match result.map_err(|error| Self::fetch_failure(&error)).and_then(parse_status) {
                    Ok(status) => {
                        self.connect.status.seq += 1;
                        self.connect.status.loading = false;
                        self.connect.status.value = Some(Ok(status));
                    }
                    // O PUT pode ter estourado o prazo com o servidor já ligado: relê em vez de supor.
                    Err(error) => { self.connect.error = Some(error); self.load_connect(cx); }
                }
            }
        }
        if self.connect.code.is_none() && self.settings == Some(Page::Connect) {
            self.connect.code = Some(cx.new(|cx| InputState::new(window, cx).masked(true)));
        }
        cx.notify();
    }

    pub(super) fn render_connect(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let title = div().flex().items_center().gap_2()
            .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child(Page::Connect.title()))
            .child(chip(tr("server_scope"), theme::muted(), theme::raised()));
        let mut page = div().flex().flex_col().gap_4().child(title)
            .child(div().id("connect-draft").role(Role::Alert).text_sm().font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::danger()).whitespace_normal().child(tr("connect_draft")))
            .child(div().text_sm().text_color(theme::muted()).whitespace_normal().child(tr("connect_intro")));
        if self.api.is_none() {
            return page.child(div().text_sm().text_color(theme::muted()).child(tr("settings_offline"))).into_any_element();
        }
        if let Some(error) = &self.connect.error {
            page = page.child(div().id("connect-write-error").role(Role::Alert).text_sm().text_color(theme::danger()).whitespace_normal().child(error.clone()));
        }
        if self.connect.status.loading {
            return page.child(div().text_sm().text_color(theme::muted()).child(tr("loading"))).into_any_element();
        }
        let status = match &self.connect.status.value {
            Some(Ok(status)) => status.clone(),
            Some(Err(error)) => return page.child(div().id("connect-load-error").role(Role::Alert).text_sm().text_color(theme::danger()).child(error.clone()))
                .child(Button::new("connect-retry").outline().small().label(tr("server_retry"))
                    .on_click(cx.listener(|this, _, _, cx| { this.connect.error = None; this.load_connect(cx); }))).into_any_element(),
            None => return page.into_any_element(),
        };
        if let Some(error) = &status.error {
            page = page.child(div().id("connect-run-error").role(Role::Alert).text_sm().text_color(theme::danger()).whitespace_normal().child(error.clone()));
        }
        if status.enabled {
            let address = status.url.clone().unwrap_or_default();
            let open = address.clone();
            page = page.child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(tr(if status.running { "connect_on" } else { "connect_starting" })))
                .child(div().font_family(theme::MONO).text_sm().truncate().child(address))
                .child(div().flex().items_center().gap_2()
                    .child(Button::new("connect-open").outline().small().label(tr("connect_open")).on_click(move |_, _, cx| cx.open_url(&open)))
                    .child(Button::new("connect-refresh").outline().small().label(tr("connect_refresh"))
                        .on_click(cx.listener(|this, _, _, cx| this.load_connect(cx))))
                    .child(Button::new("connect-off").outline().small().label(tr("connect_off")).disabled(self.connect.saving)
                        .on_click(cx.listener(|this, _, _, cx| this.write_connect(false, cx)))));
        } else if let Some(code) = &self.connect.code {
            page = page.child(super::sync::sync_field(tr("connect_code"), code, self.connect.saving))
                .child(div().text_sm().text_color(theme::muted()).whitespace_normal().child(tr("connect_code_help")))
                .child(Button::new("connect-on").primary().small()
                    .label(tr(if self.connect.saving { "connect_turning_on" } else { "connect_turn_on" }))
                    .disabled(self.connect.saving).on_click(cx.listener(|this, _, _, cx| this.write_connect(true, cx))));
        }
        page.into_any_element()
    }
}
