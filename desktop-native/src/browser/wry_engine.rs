//! WebView2 (Windows) / WKWebView (macOS) como janela filha da janela GPUI. A entrada chega direto nela.
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gpui_kit::*;
use wry::{
    NewWindowResponse, PageLoadEvent, Rect, WebContext, WebView, WebViewBuilder,
    dpi::{LogicalPosition, LogicalSize},
    raw_window_handle::{self, HasWindowHandle},
};

use super::{Event, Pointer, model};

pub struct Engine {
    view: WebView,
    // Declarado depois do `view` para cair depois dele: o contexto guarda a pasta de dados em uso.
    _context: WebContext,
    state: Rc<RefCell<model::PageState>>,
    events: async_channel::Sender<Event>,
    /// Área onde a janela filha está; `None` = escondida. Evita chamada nativa a cada quadro.
    placed: Cell<Option<Bounds<Pixels>>>,
}

fn send(events: &async_channel::Sender<Event>, state: &model::PageState) {
    let _ = events.try_send(Event::State(state.clone()));
}

/// Janela pai guardada sem o empréstimo da `Window` da GPUI, para o motor nascer fora dele.
pub struct Starter(raw_window_handle::RawWindowHandle);

impl HasWindowHandle for Starter {
    fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        // SAFETY: a janela GPUI dura mais que o motor: o painel que o cria e o guarda vive nela.
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(self.0) })
    }
}

impl Engine {
    pub fn available() -> Result<(), String> { Ok(()) }

    pub fn prepare(window: &Window, _cx: &App) -> Result<Starter, String> {
        // Caminho completo: a `Window` tem um `window_handle()` próprio, que devolve o identificador da GPUI.
        Ok(Starter(HasWindowHandle::window_handle(window).map_err(|e| e.to_string())?.as_raw()))
    }
}

impl Starter {
    /// No Windows o WebView2 roda um laço de mensagens aninhado até o controle ficar pronto, e a GPUI executa outras
    /// tarefas dentro dele: chamar fora de qualquer empréstimo da App, senão elas entram em pânico.
    pub fn start(self, events: async_channel::Sender<Event>) -> Result<Engine, String> {
        // Cookies e logins ficam ao lado das configurações do app. O macOS ignora a pasta e usa o armazenamento do app.
        let mut context = WebContext::new(crate::appearance::dir().map(|dir| dir.join("browser")));
        let state = Rc::new(RefCell::new(model::PageState::default()));
        let (on_load, on_title) = ((state.clone(), events.clone()), (state.clone(), events.clone()));
        let builder = WebViewBuilder::new_with_web_context(&mut context).with_visible(false);
        // Estacionada visível fora da tela (ver `park`): nascer com foco roubaria o teclado da janela do app.
        #[cfg(target_os = "windows")]
        let builder = builder.with_focused(false);
        let view = builder
            // Bloqueio sem aviso: o handler só recebe a URL e não distingue o quadro principal de um iframe.
            .with_navigation_handler(|url| model::allowed_request(&url))
            // ponytail: link com target=_blank não abre; carregar na mesma página quando fizer falta.
            .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
            .with_on_page_load_handler(move |event, url| {
                let mut page = on_load.0.borrow_mut();
                page.url = Some(url);
                page.loading = matches!(event, PageLoadEvent::Started);
                page.error = None;
                // O wry não expõe o histórico: depois da primeira página, voltar/avançar ficam sempre ligados.
                if !page.loading {
                    page.can_back = true;
                    page.can_forward = true;
                }
                send(&on_load.1, &page);
            })
            .with_document_title_changed_handler(move |title| {
                let mut page = on_title.0.borrow_mut();
                page.title = title;
                send(&on_title.1, &page);
            })
            .build_as_child(&self)
            .map_err(|e| e.to_string())?;
        let engine = Engine { view, _context: context, state, events, placed: Cell::new(None) };
        #[cfg(target_os = "windows")]
        engine.park();
        Ok(engine)
    }
}

impl Engine {
    /// Falha da chamada nativa vira `error` do estado, para a tela mostrar.
    fn report(&self, result: wry::Result<()>) {
        if let Err(e) = result {
            let mut page = self.state.borrow_mut();
            page.error = Some(e.to_string());
            send(&self.events, &page);
        }
    }

    pub fn load(&self, url: &str) { self.report(self.view.load_url(url)); }
    pub fn back(&self) { self.report(self.view.evaluate_script("history.back()")); }
    pub fn forward(&self) { self.report(self.view.evaluate_script("history.forward()")); }
    pub fn reload(&self) { self.report(self.view.reload()); }

    #[cfg(target_os = "windows")]
    pub fn cdp(&self) -> std::rc::Rc<super::cdp::Cdp> {
        use wry::WebViewExtWindows;
        std::rc::Rc::new(super::cdp::Cdp::new(self.view.webview()))
    }

    /// O WebView2 nasce e morre com o painel.
    #[cfg(target_os = "windows")]
    pub fn alive(&self) -> bool { true }

    pub fn place(&self, bounds: Bounds<Pixels>, _window: &mut Window) {
        let hidden = self.placed.get().is_none();
        if self.placed.get() == Some(bounds) { return; }
        self.placed.set(Some(bounds));
        // Lógico: o wry converte para físico com o DPI da janela pai.
        self.report(self.view.set_bounds(Rect {
            position: LogicalPosition::new(f32::from(bounds.origin.x) as f64, f32::from(bounds.origin.y) as f64).into(),
            size: LogicalSize::new(f32::from(bounds.size.width) as f64, f32::from(bounds.size.height) as f64).into(),
        }));
        if hidden { self.report(self.view.set_visible(true)); }
    }

    /// Escondido de verdade (`SW_HIDE`), o WebView2 para de gerar quadros e o CDP perde clique, tecla e `shot`.
    /// Fica visível para o Chromium, mas fora da área do pai, que recorta a janela filha: ninguém a vê.
    #[cfg(target_os = "windows")]
    fn park(&self) {
        // Longe demais estoura os 16 bits do WM_MOVE em DPI alto; -3000 já tira os 1280 da área do pai.
        self.report(self.view.set_bounds(Rect {
            position: LogicalPosition::new(-3000.0, 0.0).into(),
            size: LogicalSize::new(1280.0, 800.0).into(),
        }));
        self.report(self.view.set_visible(true));
    }

    pub fn hide(&self) {
        if self.placed.take().is_some() {
            #[cfg(target_os = "windows")]
            self.park();
            #[cfg(not(target_os = "windows"))]
            self.report(self.view.set_visible(false));
            // Página escondida com o foco do sistema deixaria o teclado sem dono.
            self.release_focus();
        }
    }

    pub fn pointer(&self, _kind: Pointer, _at: Point<Pixels>, _clicks: usize) {}
    pub fn wheel(&self, _at: Point<Pixels>, _delta: Point<Pixels>) {}
    pub fn key(&self, _down: bool, _keystroke: &Keystroke) {}
    /// O WebView nasce sem foco no Windows: sem mover o foco aqui, o que se digita depois do Enter na barra não chega.
    #[cfg(target_os = "windows")]
    pub fn focus(&self, focused: bool) { if focused { self.report(self.view.focus()); } }
    #[cfg(not(target_os = "windows"))]
    pub fn focus(&self, _focused: bool) {}
    /// A janela filha guarda o foco do sistema depois de um clique nela; sem devolver, o que se digita na barra vai à página.
    pub fn release_focus(&self) { self.report(self.view.focus_parent()); }
}
