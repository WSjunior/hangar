//! O cartão de entrada e a ligação do assistente com o `Hangar` (abrir, fechar, conectar no fim).
use super::*;

impl Hangar {
    /// Fim do assistente: o app entra no Hangar desta máquina sem digitar nada; o `connect` grava a conexão quando a lista chega.
    pub(in crate::app) fn setup_connect(&mut self, address: String, token: String, window: &mut Window, cx: &mut Context<Self>) {
        self.address.update(cx, |input, cx| input.set_value(address, window, cx));
        self.token.update(cx, |input, cx| input.set_value(token, window, cx));
        self.entry = None;
        self.connect(window, cx);
    }

    /// Fechar o assistente sem nada rodando: larga a entidade (e com ela o canal da senha de administrador).
    pub(in crate::app) fn close_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.setup = None;
        self.setup_hidden = None;
        if self.api.is_none() { self.open_connection(window, cx); }
        cx.notify();
    }

    /// Fechar com o script ainda rodando: a entidade sai da tela mas fica viva, para o canal da senha e o acompanhamento
    /// continuarem. Reabrir pelo menu reaproveita `setup_hidden`.
    pub(in crate::app) fn hide_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.setup_hidden = self.setup.take();
        if self.api.is_none() { self.open_connection(window, cx); }
        cx.notify();
    }
}
