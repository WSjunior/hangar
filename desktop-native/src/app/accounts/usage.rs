//! Cartões dos anéis do compositor. O de contexto mostra a ocupação e o uso da sessão. O de uso (o `account_usage` do
//! Zeron) mostra as contas Claude e Codex, a que a sessão usa
//! primeiro, com plano e as janelas de sessão e semana. Lê a mesma lista da página Contas e só quando abre: o servidor
//! devolve a cota guardada, sem releitura periódica daqui.
use super::*;

impl Hangar {
    /// Abre ou fecha o cartão, fechando o painel que estiver aberto sobre o compositor.
    pub(in crate::app) fn toggle_usage_card(&mut self, cx: &mut Context<Self>) {
        let open = !self.accounts.card || self.accounts.card_top;
        self.close_popups();
        (self.accounts.card, self.accounts.card_top) = (open, false);
        if open { self.load_session_accounts(cx); }
        cx.notify();
    }

    /// O anel de contexto abre o cartão dele pela mesma regra do de contas.
    pub(in crate::app) fn toggle_context_card(&mut self, cx: &mut Context<Self>) {
        let open = !self.context_card;
        self.close_popups();
        self.context_card = open;
        cx.notify();
    }

    /// Cartão do anel de contexto, no padrão do de contas: o número grande com a barra, como o bloco do painel, e o uso da
    /// sessão numa grade rotulada de duas colunas.
    pub(in crate::app) fn render_context_card(&self) -> AnyElement {
        let status = self.status();
        let pct = status.as_ref().and_then(|s| s.ctx_pct);
        let window = match (status.as_ref().and_then(|s| s.ctx_used), status.as_ref().and_then(|s| s.ctx_total)) {
            (Some(used), Some(total)) => Some(tr("side_ctx_of").replace("{used}", &side::tokens(used)).replace("{total}", &side::tokens(total))),
            (None, Some(total)) => Some(side::tokens(total)),
            _ => None,
        };
        let number = div().px(px(8.)).flex().items_baseline().gap(px(8.))
            .child(div().flex_shrink_0().text_size(px(28.)).line_height(px(32.)).font_weight(FontWeight::SEMIBOLD)
                .text_color(match pct { Some(p) if p >= 70. => chrome::ring_text(pct), Some(_) => theme::text(), None => theme::faint() })
                .child(pct.map(|p| format!("{}%", p.round())).unwrap_or_else(|| "—".into())))
            .child(div().min_w_0().truncate().text_xs().text_color(theme::faint())
                .child(window.map_or_else(|| tr("side_ctx_label"), |w| format!("{} · {w}", tr("side_ctx_label")))));
        let gauge = match pct {
            Some(p) => div().px(px(8.)).pt(px(8.)).pb(px(6.)).child(chrome::meter(p)),
            None => div().px(px(8.)).pt(px(4.)).pb(px(6.)).text_xs().text_color(theme::muted()).whitespace_normal().child(tr("side_ctx_unknown")),
        };
        let cells = self.stats.as_ref().map(side::stats_cells).unwrap_or_default();
        let grid = (!cells.is_empty()).then(|| div().px(px(8.)).pb(px(4.)).flex().flex_wrap().gap_y(px(6.))
            .children(cells.into_iter().map(|(label, value)| div().w(relative(0.5)).pr(px(12.)).flex().items_baseline().justify_between().gap(px(8.))
                .text_size(px(12.))
                .child(div().min_w_0().truncate().text_color(theme::muted()).child(label))
                .child(div().flex_shrink_0().font_weight(FontWeight::MEDIUM).text_color(theme::text()).child(value)))));
        div().p(px(popup::INSET)).rounded_md().bg(theme::popup_content_fill()).flex().flex_col().gap(px(2.))
            .child(popup::title(tr("ring_context"), None))
            .child(number)
            .child(gauge)
            .when_some(grid, |el, grid| el.child(popup::separator()).child(popup::title(tr("ctx_card_usage"), None)).child(grid))
            .into_any_element()
    }

    /// A pílula da barra do topo abre o mesmo cartão, só que preso a ela.
    pub(in crate::app) fn toggle_top_usage_card(&mut self, cx: &mut Context<Self>) {
        let open = !(self.accounts.card && self.accounts.card_top);
        self.close_popups();
        (self.accounts.card, self.accounts.card_top) = (open, open);
        if open { self.load_session_accounts(cx); }
        cx.notify();
    }

    /// Relê a lista de contas (com a cota guardada do servidor), para a pílula da barra do topo.
    pub(in crate::app) fn refresh_default_account(&mut self, cx: &mut Context<Self>) {
        self.load_session_accounts(cx);
    }

    /// Conta da sessão aberta, procurada na lista da máquina dela.
    fn focused_conta(&self) -> Option<&str> {
        self.selected.as_ref().and_then(|s| s.conta.as_deref())
    }

    /// A conta da sessão em foco para a pílula da barra do topo: provider, nome e a janela mais cheia, com o rótulo dela
    /// (o `piorJanela` do web). Sem sessão, o Claude; conta ausente (servidor sem o campo) é a padrão do provider.
    /// `None` antes da lista chegar ou sem conta daquele provider.
    pub(in crate::app) fn focused_account(&self) -> Option<(String, String, Option<(String, f64)>)> {
        let session = self.selected.as_ref();
        let kind = session.map(|s| s.provider.as_str()).filter(|p| !p.is_empty()).unwrap_or("claude");
        let conta = self.focused_conta();
        let c = self.session_accounts().ok()?.iter().find(|c| c.kind == kind && conta.map_or(c.active, |id| id == c.id))?;
        let login = c.login.as_ref().filter(|l| l.logged_in == Some(true));
        let title = c.alias.clone().filter(|a| !a.is_empty()).or_else(|| login.and_then(|l| l.email.clone())).unwrap_or_else(|| c.name.clone());
        let model = self.status().and_then(|s| s.model).map(|m| m.to_lowercase());
        let window = match build_row(c, &HashMap::new(), false, now()).quota {
            QuotaView::Bars { bars, .. } => fullest(&bars, model.as_deref()).map(|b| (b.label.clone(), b.pct)),
            _ => None,
        };
        Some((kind.to_owned(), title, window))
    }

    pub(in crate::app) fn render_usage_card(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let session = self.selected.as_ref();
        let kind = session.map(|s| s.provider.as_str()).filter(|p| !p.is_empty()).unwrap_or("claude");
        // A conta da sessão; sem sessão, ou servidor sem esse campo, cai na conta padrão do provider.
        let conta = self.focused_conta();
        let in_use = |c: &Credential| c.kind == kind && conta.map_or(c.active, |id| id == c.id);
        let note = |text: String, color: Hsla| div().px(px(8.)).py(px(4.)).text_sm().text_color(color).whitespace_normal().child(text).into_any_element();
        let list = self.session_accounts();
        // Aberto pelo anel de uma sessão Claude na conta Anthropic: as outras contas Claude levam esta conversa para elas.
        let movable = !self.accounts.card_top && session.is_some_and(|s| s.provider == "claude"
            && s.conta.as_deref().is_some_and(|c| c.starts_with("claude:")) && !s.read_only());
        let idle = session.is_some_and(|s| s.state == "idle");
        let transfer = session.and_then(super::sidebar::transfer_source).zip(self.selected_target())
            .filter(|(_, target)| !self.accounts.card_top && !self.sidebar.moving.contains_key(target))
            .map(|((life, jsonl), target)| (target, life.to_owned(), jsonl.to_owned()));
        let body = match (&list.value, list.ok()) {
            (_, Some(list)) => {
                let mut mine: Vec<&Credential> = list.iter().filter(|c| matches!(c.kind.as_str(), "claude" | "codex")).collect();
                mine.sort_by_key(|c| !in_use(c));
                if mine.is_empty() { note(tr("usage_card_empty"), theme::muted()) } else {
                    let (engines, now) = (HashMap::new(), now());
                    let rows = mine.into_iter().map(|c| {
                        let quota = build_row(c, &engines, false, now).quota;
                        // A janela mais cheia, como a lista do backend: a partir de 95% pede confirmação, a partir de 99% não aceita.
                        let pct = match &quota { QuotaView::Bars { bars, .. } => bars.iter().map(|b| b.pct).fold(None, |m: Option<f64>, p| Some(m.map_or(p, |m| m.max(p)))), _ => None };
                        let target = c.id.strip_prefix("claude:").map(str::to_owned)
                            .filter(|_| movable && idle && c.kind == "claude" && !in_use(c) && pct.is_none_or(|p| p < 99.));
                        let row = account_row(c, in_use(c), quota);
                        if c.kind == "codex" && movable {
                            let account = c.codex_account.clone();
                            let connected = c.login.as_ref().is_some_and(|l| l.logged_in == Some(true));
                            let source = transfer.clone();
                            let available = source.is_some() && account.is_some() && connected && c.id.starts_with("codex:");
                            let title = tr("session_transfer_account_aria").replace("{name}", &session.map(|s| s.name.clone()).unwrap_or_default())
                                .replace("{account}", c.alias.as_deref().filter(|a| !a.is_empty()).unwrap_or(&c.name));
                            return Button::new(SharedString::from(format!("usage-transfer-{}", c.id))).ghost().w_full().h_auto()
                                .disabled(!available).accessibility_label(title).child(row)
                                .when(!connected, |b| b.tooltip(tr("create_codex_disconnected")))
                                .when(!available && connected, |b| b.tooltip(tr("session_transfer_unavailable")))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    let (Some((target, life, jsonl)), Some(account)) = (&source, &account) else { return };
                                    this.close_popups();
                                    this.open_transfer(target.clone(), life.clone(), jsonl.clone(), account.clone(), window, cx);
                                })).into_any_element();
                        }
                        match target {
                            Some(path) => {
                                let label = c.alias.clone().filter(|a| !a.is_empty()).unwrap_or_else(|| c.name.clone());
                                let warn = pct.filter(|p| *p >= 95.);
                                // Borda sempre à vista e realce na cor de destaque: o cinza do `hover` some sobre o papel de parede.
                                div().id(SharedString::from(format!("usage-move-{path}"))).rounded(px(7.)).cursor_pointer()
                                    .border_1().border_color(theme::border_strong())
                                    .hover(|el| el.bg(theme::accent_dim()).border_color(theme::accent())).child(row)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.close_popups();
                                        if let Some(owner) = this.selected_target() {
                                            this.confirm_account(owner, path.clone(), label.clone(), warn, window, cx);
                                        }
                                    })).into_any_element()
                            }
                            None => row.when(movable && c.kind == "claude" && !in_use(c) && pct.is_some_and(|p| p >= 99.), |el| el.opacity(0.55))
                                .into_any_element(),
                        }
                    });
                    div().flex().flex_col().gap(px(2.))
                        .when(movable, |el| el.child(note(tr(if transfer.is_some() { "session_transfer_accounts_hint" }
                            else if idle { "usage_card_move_hint" } else { "usage_card_move_busy" }), theme::faint())))
                        .children(rows)
                        .into_any_element()
                }
            }
            (Some(Err(error)), _) => note(tr("accounts_failed").replace("{reason}", error), theme::warning()),
            _ => popup::skeleton("usage-card-loading", 1).into_any_element(),
        };
        div().p(px(popup::INSET)).rounded_md().bg(theme::popup_content_fill()).flex().flex_col().gap(px(2.))
            .child(popup::title(tr("usage_card_title"), None))
            .child(div().id("usage-card-scroll").max_h((window.viewport_size().height - px(180.)).max(px(120.))).overflow_y_scroll().child(body))
            // Rodapé do web: atalho para a tela de contas. As configurações são do servidor ativo, então com sessão de
            // outra máquina o atalho levaria às contas erradas e some.
            .when(self.open_api.is_none(), |el| el.child(div().px(px(4.)).pt(px(4.)).border_t_1().border_color(theme::border()).flex()
                .child(Button::new("usage-card-accounts").ghost().small().label(crate::app::activity::web("contas_titulo"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.close_popups();
                        this.open_settings(super::settings::Page::Accounts, window, cx);
                    })))))
            .into_any_element()
    }
}

fn account_row(c: &Credential, in_use: bool, quota: QuotaView) -> Div {
    let login = c.login.as_ref().filter(|l| l.logged_in == Some(true));
    let email = login.and_then(|l| l.email.clone()).filter(|e| !e.is_empty());
    let title = c.alias.clone().filter(|a| !a.is_empty()).or_else(|| login.and_then(|l| l.email.clone())).unwrap_or_else(|| c.name.clone());
    // O servidor manda o plano cru ("max", "pro").
    let plan = login.and_then(|l| l.plan.as_deref()).and_then(|p| {
        let mut chars = p.chars();
        chars.next().map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
    });
    let meta = div().flex().items_center().gap(px(6.)).text_size(px(12.)).text_color(theme::muted())
        .child(side::agent_label(&c.kind))
        .when(plan.is_some(), |el| el.child(div().text_color(theme::faint()).child("·")))
        .when_some(plan.clone(), |el, plan| el.child(plan))
        .when(in_use, |el| el.child(div().text_color(theme::faint()).child("·")))
        .when(in_use, |el| el.child(div().text_color(theme::accent()).child(tr("usage_card_in_use"))));
    let meters = match quota {
        QuotaView::Bars { bars, stale } => div().flex().flex_col().gap(px(4.)).when(stale.is_some(), |el| el.opacity(0.6))
            .children(bars.iter().take(2).map(meter))
            .children(stale.map(|text| div().text_size(px(11.)).text_color(theme::faint()).child(text))),
        QuotaView::Note(text) => div().text_size(px(12.)).text_color(theme::muted()).whitespace_normal().child(text),
        QuotaView::Nothing => div(),
    };
    div().flex().items_center().gap(px(12.)).px(px(8.)).py(px(7.)).rounded(px(7.)).when(in_use, |el| el.bg(theme::accent_dim()))
        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.))
            .child(div().truncate().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(theme::text()).child(title.clone()))
            // Apelido não diz qual login é: o email vem embaixo, a menos que já seja o próprio título.
            .when_some(email.filter(|e| *e != title), |el, email| el.child(div().truncate().text_size(px(11.5)).text_color(theme::faint()).child(email)))
            .child(meta))
        .child(div().w(px(176.)).flex_shrink_0().child(meters))
}

/// A janela mais cheia; janela de um modelo só conta quando é o modelo da sessão, e no empate vence a que renova antes.
fn fullest<'a>(bars: &'a [Bar], model: Option<&str>) -> Option<&'a Bar> {
    bars.iter()
        .filter(|b| !(b.per_model && model.is_some_and(|m| !m.contains(&b.label.to_lowercase()))))
        .fold(None, |best: Option<&Bar>, b| match best {
            None => Some(b),
            Some(best) if b.pct > best.pct => Some(b),
            Some(best) if b.pct == best.pct && b.reset_at.is_some_and(|at| best.reset_at.is_none_or(|was| at < was)) => Some(b),
            best => best,
        })
}

/// Uma janela numa linha: rótulo, barra, % e, embaixo, quando ela renova ("↺ 1h20" na curta, "↺ sáb 27/09 15h" na
/// longa), como o cartão da pílula do web.
fn meter(bar: &Bar) -> Div {
    let label = if bar.label == "5h" { tr("usage_card_session") } else { window_label(&bar.label) };
    div().flex().flex_col().gap(px(1.))
        .child(div().flex().items_center().gap(px(8.)).text_size(px(12.))
            .child(div().w(px(52.)).flex_shrink_0().truncate().text_color(theme::muted()).child(label))
            .child(div().flex_1().h(px(4.)).rounded_full().bg(theme::border_strong())
                .child(div().h_full().rounded_full().bg(level(bar.pct)).w(relative((bar.pct.clamp(0., 100.) / 100.) as f32))))
            .child(div().w(px(34.)).flex_shrink_0().flex().justify_end().text_color(if bar.pct > 80. { level(bar.pct) } else { theme::muted() })
                .child(format!("{}%", bar.pct.round()))))
        .when(!bar.reset.is_empty(), |el| el.child(div().pl(px(60.)).truncate().text_size(px(11.)).text_color(theme::faint())
            .child(format!("↺ {}", bar.reset))))
}
