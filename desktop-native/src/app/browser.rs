//! Navegador do painel lateral: barra com voltar, avançar, recarregar e endereço, e a página embaixo.
//! O comportamento da barra (Enter navega, Esc desiste e devolve à página) vem do Zeron (MIT, crates/ui/src/browser/view.rs).
use std::{cell::Cell, rc::Rc};

use super::*;
use crate::appearance::SideTab;
use crate::browser::{Engine, Pointer, model};

pub(super) struct BrowserPanel {
    /// `servidor::sessão` dona deste navegador: nome do sidecar que o hangar-preview lê.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    key: String,
    #[cfg(not(target_os = "macos"))]
    controller: Option<Rc<crate::browser::control::Controller<crate::browser::control::CdpPage>>>,
    /// CDP do motor, que o controlador usa.
    #[cfg(not(target_os = "macos"))]
    cdp: Option<Rc<crate::browser::cdp::Cdp>>,
    /// CDP do repasse da tela remota (`browser::relay`), que não passa pelo turno do controlador. No Windows é o do
    /// motor; no Linux é uma sessão própria no alvo, criada no primeiro espectador, com o screencast dele.
    #[cfg(not(target_os = "macos"))]
    relay: std::cell::RefCell<Option<Rc<crate::browser::cdp::Cdp>>>,
    /// Quem olha a tela remota; os ouvintes de evento do CDP guardam só referência fraca.
    #[cfg(not(target_os = "macos"))]
    viewer: Rc<std::cell::RefCell<Option<crate::browser::relay::Viewer>>>,
    /// Nasce na primeira navegação, que tem a janela.
    engine: Option<Result<Rc<Engine>, String>>,
    /// Endereço a abrir quando o motor terminar de nascer; `Some` enquanto ele nasce.
    starting: Option<String>,
    _start: Option<Task<()>>,
    events: async_channel::Sender<crate::browser::Event>,
    address: Entity<InputState>,
    page: model::PageState,
    /// Endereço recusado, mostrado embaixo da barra.
    invalid: Option<String>,
    focus: FocusHandle,
    /// Origem da página no último desenho: o ponteiro chega ao motor relativo a ela.
    origin: Rc<Cell<Point<Pixels>>>,
    /// Decidido pelo `Hangar` a cada quadro (`browser_visible`).
    shown: bool,
    _drain: Task<()>,
    _subscriptions: [Subscription; 4],
}

impl BrowserPanel {
    fn new(key: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (events, received) = async_channel::unbounded();
        let address = cx.new(|cx| InputState::new(window, cx).placeholder(tr("browser_address")));
        let focus = cx.focus_handle();
        let drain = cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = received.recv().await {
                if this.update_in(cx, |this, window, cx| this.receive(event, window, cx)).is_err() { break; }
            }
        });
        let address_focus = address.focus_handle(cx);
        let subscriptions = [
            cx.subscribe_in(&address, window, |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event { this.submit(window, cx); }
            }),
            cx.on_focus(&address_focus, window, |this, _, _| if let Some(engine) = this.engine() { engine.release_focus() }),
            cx.on_focus(&focus, window, |this, _, _| if let Some(engine) = this.engine() { engine.focus(true) }),
            cx.on_blur(&focus, window, |this, _, _| if let Some(engine) = this.engine() { engine.focus(false) }),
        ];
        Self { key, #[cfg(not(target_os = "macos"))] controller: None, #[cfg(not(target_os = "macos"))] cdp: None,
            #[cfg(not(target_os = "macos"))] relay: Default::default(), #[cfg(not(target_os = "macos"))] viewer: Rc::default(), engine: None, starting: None, _start: None, events, address, page: model::PageState::default(), invalid: None, focus,
            origin: Rc::default(), shown: false, _drain: drain, _subscriptions: subscriptions }
    }

    fn engine(&self) -> Option<&Rc<Engine>> { self.engine.as_ref()?.as_ref().ok() }

    fn receive(&mut self, event: crate::browser::Event, window: &mut Window, cx: &mut Context<Self>) {
        if let crate::browser::Event::State(page) = event {
            // Quem está digitando no endereço não perde o texto para a página que acabou de carregar.
            let url = page.url.clone().unwrap_or_default();
            if !self.address.focus_handle(cx).is_focused(window) && self.address.read(cx).value() != url {
                self.address.update(cx, |input, cx| input.set_value(url, window, cx));
            }
            self.page = page;
            #[cfg(not(target_os = "macos"))]
            crate::browser::server::write_sidecar(&self.key, self.page.url.as_deref().unwrap_or(""), &self.page.title);
        }
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.address.read(cx).value().to_string();
        match model::normalize_address(&text) {
            Err(key) => self.invalid = Some(tr(key)),
            Ok(url) => self.go(url, window, cx),
        }
        cx.notify();
    }

    /// Navega, ou faz o motor nascer e navega quando ele ficar pronto.
    pub(super) fn go(&mut self, url: String, window: &mut Window, cx: &mut Context<Self>) {
        self.invalid = None;
        // Motor que falhou (ou o Chromium que caiu) volta a ser tentado no próximo `open`.
        #[cfg(not(target_os = "macos"))]
        if matches!(&self.engine, Some(Err(_))) || self.engine().is_some_and(|e| !e.alive()) { self.drop_engine(); }
        if self.engine.is_some() {
            self.navigate(url, window, cx);
        } else if self.starting.replace(url).is_none() {
            // Enter repetido enquanto o motor nasce só troca o endereço pendente.
            self.start(window, cx);
        }
    }

    fn navigate(&mut self, url: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(engine) = self.engine() else { return };
        engine.load(&url);
        self.address.update(cx, |input, cx| input.set_value(url, window, cx));
        // Aberto pelo agente com o painel fora da tela: o foco fica onde o usuário está digitando.
        if self.shown { self.focus.focus(window, cx); }
    }

    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let starter = Engine::prepare(window, cx);
        let events = self.events.clone();
        self._start = Some(cx.spawn_in(window, async move |this, cx| {
            // Corpo da tarefa, fora de `update`: a App não está emprestada, então o laço de mensagens que o WebView2
            // roda até nascer pode executar outras tarefas da GPUI sem pânico.
            let engine = starter.and_then(|starter| starter.start(events)).map(Rc::new);
            if let Err(e) = &engine { eprintln!("[nav] motor do navegador falhou: {e}"); }
            // Painel fechado no meio: o motor cai junto com o resultado.
            let _ = this.update_in(cx, |this, window, cx| {
                this.engine = Some(engine);
                #[cfg(not(target_os = "macos"))]
                if let Some(Ok(engine)) = &this.engine { let engine = engine.clone(); this.attach(&engine, cx); }
                if let Some(url) = this.starting.take() { this.navigate(url, window, cx); }
                cx.notify();
            });
        }));
    }

    /// Liga o controlador do hangar-preview ao CDP do motor recém-nascido.
    #[cfg(not(target_os = "macos"))]
    fn attach(&mut self, engine: &Rc<Engine>, cx: &mut Context<Self>) {
        use crate::browser::control::{CdpPage, Controller, EVENTS};
        let cdp = engine.cdp();
        let ctl = Rc::new(Controller::new(CdpPage { cdp: cdp.clone(), executor: cx.background_executor().clone() }, !self.shown));
        for event in EVENTS {
            let weak = Rc::downgrade(&ctl);
            if let Err(e) = cdp.on(event, move |params| if let Some(ctl) = weak.upgrade() { ctl.on_event(event, &params) }) {
                eprintln!("[nav] evento {event} sem ouvinte: {e}");
            }
        }
        #[cfg(target_os = "windows")]
        { self.listen_relay(&cdp); *self.relay.borrow_mut() = Some(cdp.clone()); }
        let setup = ctl.clone();
        cx.spawn(async move |_, _| setup.start().await).detach();
        self.controller = Some(ctl);
        self.cdp = Some(cdp);
    }

    /// Entrega ao espectador atual os eventos que a tela remota acompanha.
    #[cfg(not(target_os = "macos"))]
    fn listen_relay(&self, cdp: &Rc<crate::browser::cdp::Cdp>) {
        for event in crate::browser::relay::WATCHED {
            let (viewer, page) = (Rc::downgrade(&self.viewer), Rc::downgrade(cdp));
            let listened = cdp.on(event, move |params| {
                let Some(viewer) = viewer.upgrade() else { return };
                let unsent = match viewer.borrow().as_ref() {
                    Some(viewer) => viewer.deliver(event, &params),
                    None => crate::browser::relay::frame_id(event, &params),
                };
                // Quadro sem entrega também é confirmado: sem o ack o Chromium para o screencast.
                if let (Some(id), Some(page)) = (unsent, page.upgrade()) {
                    drop(page.call("Page.screencastFrameAck", serde_json::json!({"sessionId": id})));
                }
            });
            if let Err(e) = listened { eprintln!("[nav] evento {event} sem ouvinte: {e}"); }
        }
    }

    /// Motor que falhou ou cujo Chromium caiu: some com tudo o que falava com ele, e o próximo `open` o refaz.
    #[cfg(not(target_os = "macos"))]
    fn drop_engine(&mut self) {
        self.end_viewer();
        self.relay.borrow_mut().take();
        self.controller = None;
        self.cdp = None;
        self.engine = None;
    }

    /// Espectador novo da tela remota; o anterior recebe o aviso de que outro aparelho assumiu.
    #[cfg(not(target_os = "macos"))]
    pub(super) fn watch(&self, viewer: crate::browser::relay::Viewer) -> Result<(), String> {
        if self.cdp.is_none() { return Err(format!("erro: o navegador da sessao {} ainda esta iniciando, tente de novo em instantes", self.key)); }
        #[cfg(target_os = "linux")]
        if self.relay.borrow().is_none() {
            let engine = self.engine().ok_or_else(|| format!("erro: a sessao {} nao tem navegador aberto", self.key))?;
            let session = engine.viewer_session().map_err(|e| format!("erro: tela remota sem sessao no navegador: {e}"))?;
            self.listen_relay(&session);
            *self.relay.borrow_mut() = Some(session);
        }
        if let Some(old) = self.viewer.borrow_mut().replace(viewer) { old.detach(crate::browser::relay::REPLACED); }
        Ok(())
    }

    /// O espectador `id` saiu: o screencast para, a menos que outro já o tenha substituído.
    #[cfg(not(target_os = "macos"))]
    pub(super) fn unwatch(&self, id: u64) {
        let mut viewer = self.viewer.borrow_mut();
        if viewer.as_ref().is_none_or(|v| v.id != id) { return; }
        *viewer = None;
        // No Linux a sessão do espectador cai inteira, e o screencast dela junto.
        #[cfg(target_os = "linux")]
        self.relay.borrow_mut().take();
        #[cfg(target_os = "windows")]
        if let Some(cdp) = &self.cdp { drop(cdp.call("Page.stopScreencast", serde_json::json!({}))); }
    }

    /// Navegador fechando: quem olha recebe o fim antes de o painel cair.
    #[cfg(not(target_os = "macos"))]
    pub(super) fn end_viewer(&self) {
        if let Some(viewer) = self.viewer.borrow_mut().take() { viewer.detach("target_closed"); }
    }

    /// CDP da tela remota: os comandos repassados do celular vão por ele.
    #[cfg(not(target_os = "macos"))]
    pub(super) fn relay_cdp(&self) -> Option<Rc<crate::browser::cdp::Cdp>> { self.relay.borrow().clone() }

    /// Por que ainda não há controlador: `None` = motor nunca pedido, `Some(Ok)` = nascendo, `Some(Err)` = falhou.
    #[cfg(not(target_os = "macos"))]
    pub(super) fn engine_status(&self) -> Option<Result<(), &str>> {
        match &self.engine {
            Some(Err(e)) => Some(Err(e)),
            Some(Ok(_)) => Some(Ok(())),
            None => self.starting.is_some().then_some(Ok(())),
        }
    }

    #[cfg(not(target_os = "macos"))]
    pub(super) fn controller(&self) -> Option<Rc<crate::browser::control::Controller<crate::browser::control::CdpPage>>> { self.controller.clone() }

    fn restore_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.page.url.clone().unwrap_or_default();
        self.address.update(cx, |input, cx| input.set_value(url, window, cx));
        self.invalid = None;
        self.focus.focus(window, cx);
        cx.notify();
    }

    /// Clique na GPUI com a página nativa segurando o foco do sistema: sem isto o teclado (Esc, atalhos) seguiria
    /// indo para a página.
    pub(super) fn release_focus(&self) {
        if let Some(engine) = self.engine() { engine.release_focus() }
    }

    pub(super) fn focus_address(&self, window: &mut Window, cx: &mut App) {
        self.address.update(cx, |input, cx| input.focus(window, cx));
    }

    /// Diz se mudou. Escondida, a página não se posiciona no desenho e a janela nativa some.
    /// O controlador põe um viewport fixo na página escondida, para ela seguir desenhando para o hangar-preview.
    fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) -> bool {
        if self.shown == shown { return false; }
        self.shown = shown;
        if !shown && let Some(engine) = self.engine() { engine.hide(); }
        #[cfg(not(target_os = "macos"))]
        if let Some(ctl) = self.controller.clone() { cx.spawn(async move |_, _| ctl.set_hidden(!shown).await).detach(); }
        #[cfg(target_os = "macos")]
        let _ = cx;
        true
    }

    fn pointer(&self, kind: Pointer, at: Point<Pixels>, clicks: usize) {
        if let Some(engine) = self.engine() { engine.pointer(kind, at - self.origin.get(), clicks); }
    }

    fn render_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let page = div().id("browser-page").relative().flex_1().min_h_0().overflow_hidden()
            .track_focus(&self.focus).key_context("BrowserPage");
        let note = |text: String, color: Hsla| div().size_full().flex().items_center().justify_center().p_4()
            .text_size(px(12.)).text_color(color).text_center().whitespace_normal().child(text);
        let engine = match &self.engine {
            None if self.starting.is_some() => return page.role(Role::Status).child(note(tr("loading"), theme::muted())).into_any_element(),
            None => return page.child(note(tr("browser_empty"), theme::faint())).into_any_element(),
            Some(Err(error)) => return page.role(Role::Alert)
                .child(note(tr("browser_failed").replace("{error}", error), theme::warning())).into_any_element(),
            Some(Ok(engine)) => engine.clone(),
        };
        // Sem página nenhuma, o erro da primeira carga é o conteúdo; com página, o motor mostra a dele.
        if let (None, Some(error)) = (&self.page.url, &self.page.error) {
            return page.role(Role::Alert).child(note(error.clone(), theme::warning())).into_any_element();
        }
        let origin = self.origin.clone();
        let label = if self.page.title.trim().is_empty() { tr("browser") } else { self.page.title.clone() };
        page.bg(theme::background()).aria_label(label)
            .when(self.shown, |el| el.child(canvas(|_, _, _| {}, move |bounds, _, window, _| {
                origin.set(bounds.origin);
                engine.place(bounds, window);
            }).absolute().inset_0()))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, window, cx| {
                this.focus.focus(window, cx);
                this.pointer(Pointer::Down, event.position, event.click_count);
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, event: &MouseUpEvent, _, _| this.pointer(Pointer::Up, event.position, 0)))
            // Soltar fora da página ainda termina o arrasto começado nela.
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, event: &MouseUpEvent, _, _| this.pointer(Pointer::Up, event.position, 0)))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, _| this.pointer(Pointer::Move, event.position, 0)))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, _| {
                let Some(engine) = this.engine() else { return };
                let delta = event.delta.pixel_delta(px(16.));
                // A GPUI manda o sinal oposto ao do motor.
                engine.wheel(event.position - this.origin.get(), point(-delta.x, -delta.y));
            }))
            // A tecla é da página: os atalhos do app (Esc, "/") não comem o que se digita nela.
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if let Some(engine) = this.engine() { engine.key(true, &event.keystroke); }
                cx.stop_propagation();
            }))
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, _, cx| {
                if let Some(engine) = this.engine() { engine.key(false, &event.keystroke); }
                cx.stop_propagation();
            }))
            .into_any_element()
    }
}

impl Render for BrowserPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ready = self.engine().is_some();
        let back = chrome::icon_button("browser-back", IconName::ArrowLeft, tr("browser_back"), cx)
            .disabled(!ready || !self.page.can_back)
            .on_click(cx.listener(|this, _, _, _| if let Some(engine) = this.engine() { engine.back() }));
        let forward = chrome::icon_button("browser-forward", IconName::ArrowRight, tr("browser_forward"), cx)
            .disabled(!ready || !self.page.can_forward)
            .on_click(cx.listener(|this, _, _, _| if let Some(engine) = this.engine() { engine.forward() }));
        let reload = chrome::icon_button("browser-reload", IconName::RotateCw, tr("browser_reload"), cx)
            .disabled(!ready || self.page.url.is_none())
            .on_click(cx.listener(|this, _, _, _| if let Some(engine) = this.engine() { engine.reload() }));
        let address = div().flex_1().min_w_0()
            .capture_action(cx.listener(|this, _: &Escape, window, cx| { cx.stop_propagation(); this.restore_address(window, cx); }))
            .child(Input::new(&self.address).id("browser-address").small().aria_label(tr("browser_address"))
                .when(self.page.loading, |el| el.suffix(chrome::Spinner::new("browser-loading", IconName::LoaderCircle, px(12.), theme::muted()))));
        let toolbar = div().id("browser-toolbar").flex_shrink_0().flex().items_center().gap_1().px_2().py(px(6.))
            .border_b_1().border_color(theme::border())
            .child(back).child(forward).child(reload).child(address);
        // Com a página na tela, a falha dela fica numa linha; sem página, `render_page` a mostra no lugar dela.
        let notice = self.invalid.clone().or_else(|| self.page.error.clone().filter(|_| ready && self.page.url.is_some()));
        div().size_full().flex().flex_col()
            .child(toolbar)
            .when_some(notice, |el, text| el.child(div().id("browser-notice").flex_shrink_0().px_3().py(px(6.)).role(Role::Alert)
                .text_size(px(11.)).text_color(theme::danger()).whitespace_normal().child(text)))
            .child(self.render_page(cx))
    }
}

impl Hangar {
    /// Linha Navegador do menu do painel: abre o navegador na primeira vez e leva à aba dele.
    pub(super) fn open_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.browser_key() else { return };
        let browser = self.browser_for(key, window, cx);
        self.show_browser_tab(window, cx);
        browser.update(cx, |panel, cx| panel.focus_address(window, cx));
    }

    /// Painel aberto com a aba Navegador da sessão aberta à frente.
    fn show_browser_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.side_key() else { return };
        if !self.side.open { self.toggle_side(cx); }
        self.side.browser_open.insert(key);
        self.choose_side_tab(SideTab::Browser, window, cx);
    }

    /// O × da aba Navegador: a aba sai desta sessão e a página se esconde, mas o motor fica para o "+" reabrir a mesma
    /// página.
    pub(super) fn close_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let showing = self.side_tab() == SideTab::Browser;
        if let Some(key) = self.side_key() { self.side.browser_open.remove(&key); }
        for browser in self.side.browsers.values().cloned().collect::<Vec<_>>() { browser.update(cx, |panel, cx| { panel.set_shown(false, cx); }); }
        if showing { self.choose_side_tab(SideTab::Context, window, cx); } else { cx.notify(); }
    }

    /// No Windows e no macOS a página é uma janela filha, por cima de tudo o que a GPUI desenha: qualquer camada sobre o
    /// painel a esconde. Cobre o painel fora de vista (fechado, estreito, sem sessão, visor de arquivos expandido), outra
    /// aba à frente (menu, subagente), as páginas de Configurações e Custos, a caixa de configurações ao vivo, a conexão,
    /// a busca, os painéis presos ao compositor e à barra do topo, os diálogos e folhas do kit e as notificações. A aba
    /// fechada no × sai de `side_tab`, então também esconde. No
    /// Linux a página é desenhada pela própria GPUI, e as camadas passam por cima dela sem precisar escondê-la.
    fn browser_visible(&self, window: &mut Window, cx: &mut App) -> bool {
        let covered = cfg!(not(target_os = "linux")) && (self.connection_dialog || self.search.open || self.popup_open()
            || window.has_active_dialog(cx) || window.has_active_sheet(cx) || !window.notifications(cx).is_empty());
        self.side_width(window).is_some() && !self.files_expanded()
            && !self.side_menu_shown() && !self.subagent_tab_open() && self.side_tab() == SideTab::Browser
            && self.settings.is_none() && self.costs.view.is_none() && self.worktrees.view.is_none() && !covered
    }

    /// A cada quadro da janela, antes das áreas guardadas desenharem.
    pub(super) fn sync_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let visible = self.browser_visible(window, cx);
        let current = self.browser_key();
        for (key, browser) in self.side.browsers.iter().map(|(k, b)| (k.clone(), b.clone())).collect::<Vec<_>>() {
            let shown = visible && current.as_deref() == Some(key.as_str());
            if browser.update(cx, |panel, cx| panel.set_shown(shown, cx)) && shown {
                // O painel é guardado entre quadros: sem um redesenho dele, a página não volta a se posicionar.
                let id = browser.entity_id();
                window.on_next_frame(move |_, cx| cx.notify(id));
            }
        }
    }

    /// Dono do navegador na tela: a sessão aberta, com a máquina dela; uma chave só no macOS.
    pub(super) fn browser_key(&self) -> Option<String> {
        if !cfg!(target_os = "macos") { self.side_key() } else { Some("*".into()) }
    }

    /// `servidor::sessão` da sessão aberta: dono da aba lembrada e da aba Navegador do painel.
    pub(super) fn side_key(&self) -> Option<String> {
        Some(format!("{}::{}", super::servers::norm(&self.session_server()?), self.selected.as_ref()?.name))
    }

    fn browser_for(&mut self, key: String, window: &mut Window, cx: &mut Context<Self>) -> Entity<BrowserPanel> {
        self.side.browsers.entry(key.clone()).or_insert_with(|| cx.new(|cx| BrowserPanel::new(key, window, cx))).clone()
    }

    /// `hangar-preview open` → backend → evento `nav` na lista. Com a sessão na tela a aba Navegador vem à frente; fora
    /// dela o navegador nasce escondido, o CLI já o dirige, e a aba aparece quando a sessão for aberta. Só do servidor
    /// desta máquina: o CLI que pediu roda nela.
    pub(super) fn receive_nav(&mut self, data: serde_json::Value, window: &mut Window, cx: &mut Context<Self>) {
        if cfg!(target_os = "macos") { return; }
        if !self.api.as_ref().is_some_and(|api| api.is_loopback()) { eprintln!("[nav] nav ignorado: servidor ativo nao e desta maquina"); return; }
        let (Some(name), Some(url)) = (data["name"].as_str(), data["url"].as_str()) else { eprintln!("[nav] nav ignorado: sem name/url: {data}"); return };
        // Mesma chave de `browser_key` com essa sessão aberta: a lista que traz o evento é a do servidor ativo.
        let Some(server) = self.server.as_deref() else { eprintln!("[nav] nav ignorado: sem servidor ativo"); return };
        let key = format!("{}::{name}", super::servers::norm(server));
        let browser = self.browser_for(key.clone(), window, cx);
        browser.update(cx, |panel, cx| panel.go(url.to_owned(), window, cx));
        if self.browser_key().as_deref() == Some(key.as_str()) { self.show_browser_tab(window, cx); } else {
            self.side.browser_open.insert(key.clone());
            self.side.tabs.insert(key, SideTab::Browser);
        }
        if let Some(api) = self.api.clone() {
            let name = name.to_owned();
            // Sem a confirmação o backend manda o mesmo `nav` de novo na próxima conexão, que renavega e traz a aba de volta.
            self.runtime.spawn(async move {
                if let Err(e) = api.server_send(reqwest::Method::DELETE, &["sessions", &name, "nav"], None, 15).await {
                    eprintln!("[nav] confirmação do nav de {name} falhou, ele volta na próxima conexão: {e:?}");
                }
            });
        }
    }

    /// Pedido do servidor local: vai ao navegador da sessão pedida, esteja ela na tela ou não.
    #[cfg(not(target_os = "macos"))]
    pub(super) fn dispatch_preview(&mut self, request: crate::browser::server::Request, cx: &mut Context<Self>) {
        use crate::browser::{control::Reply, server::Request};
        fn answer(reply: futures::channel::oneshot::Sender<Reply>, text: String) { let _ = reply.send(Reply::Text(text)); }
        let missing = |key: &str| format!("erro: a sessao {key} nao tem navegador aberto");
        let (key, verb, args, tab, reply) = match request {
            Request::Command { key, verb, args, tab, reply } => (key, verb, args, tab, reply),
            Request::Cdp { key, method, params, reply } => {
                let Some(cdp) = self.side.browsers.get(&key).and_then(|b| b.read(cx).relay_cdp()) else { let _ = reply.send(Err(missing(&key))); return };
                let call = cdp.call(&method, params);
                return cx.spawn(async move |_, _| { let _ = reply.send(call.await); }).detach();
            }
            Request::Watch { key, viewer, reply } => {
                let _ = reply.send(match self.side.browsers.get(&key) { Some(b) => b.read(cx).watch(viewer), None => Err(missing(&key)) });
                return;
            }
            Request::Unwatch { key, id } => {
                if let Some(b) = self.side.browsers.get(&key) { b.read(cx).unwatch(id); }
                return;
            }
        };
        if tab.is_some() { return answer(reply, "erro: o app nativo ainda nao tem abas: e um navegador por sessao".into()); }
        if verb == "close" {
            let closed = self.side.browsers.remove(&key).inspect(|b| b.read(cx).end_viewer()).is_some();
            if closed { crate::browser::server::remove_sidecar(&key); self.side.browser_open.remove(&key); cx.notify(); }
            return answer(reply, if closed { "ok: close".into() } else { format!("erro: a sessao {key} nao tem navegador aberto") });
        }
        let Some(browser) = self.side.browsers.get(&key) else { return answer(reply, missing(&key)) };
        let panel = browser.read(cx);
        // Página que caiu não responde: sem isto cada verbo esperava o prazo inteiro do controlador.
        if panel.engine().is_some_and(|e| !e.alive()) {
            return answer(reply, format!("erro: a pagina da sessao {key} caiu; abra de novo com hangar-preview open <url>"));
        }
        let Some(ctl) = panel.controller() else {
            return answer(reply, match panel.engine_status() {
                Some(Err(e)) => format!("erro: o navegador da sessao {key} falhou ao iniciar: {e}"),
                Some(Ok(())) => format!("erro: o navegador da sessao {key} ainda esta iniciando, tente de novo em instantes"),
                None => format!("erro: a sessao {key} nao tem navegador aberto"),
            });
        };
        cx.spawn(async move |_, _| { let _ = reply.send(ctl.run(&verb, &args).await); }).detach();
    }
}
