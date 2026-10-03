//! Worktrees: página própria no molde de Custos, com as worktrees do servidor por repositório e, ao lado, o painel da
//! que foi aberta (pela lista ou pelo chip da linha da conversa) com o que se perde ao apagá-la.
use super::*;
use super::costs::{card_plain, empty_state, error_state, loading_state, note_box, page_frame};
use super::device::Remote;
use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub(super) struct WorktreeStatus {
    pub path: String,
    pub repo: String,
    pub exists: bool,
    pub branch: Option<String>,
    pub base: Option<String>,
    #[serde(default)]
    pub main_branch: Option<String>,
    pub merged: bool,
    pub ahead: i64,
    pub dirty: i64,
    #[serde(default)]
    pub ignored: Vec<String>,
    #[serde(default)]
    pub sessions: Vec<String>,
    pub closed: i64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub(super) struct WorktreeRepo { pub repo: String, pub worktrees: Vec<WorktreeStatus> }

/// O que se perde ao apagar: a contagem de não commitados e os ignorados pelo nome.
pub(super) fn lost_files(s: &WorktreeStatus) -> Vec<String> {
    let mut out = Vec::new();
    if s.dirty > 0 { out.push(tr_shared("worktree_nao_commitados", &[("n", &s.dirty.to_string())])); }
    out.extend(s.ignored.iter().cloned());
    out
}

pub(super) fn clean_merged(list: &[WorktreeStatus]) -> usize {
    list.iter().filter(|w| w.merged && w.dirty == 0 && w.ignored.is_empty() && w.sessions.is_empty()).count()
}

fn base_name(path: &str) -> String {
    path.trim_end_matches('/').rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or(path).to_owned()
}

fn branch_line(s: &WorktreeStatus) -> String {
    let branch = s.branch.as_deref().unwrap_or("—");
    match s.base.as_deref() { Some(base) => format!("{branch} ← {base}"), None => branch.to_owned() }
}

#[derive(Default)]
pub(super) struct Worktrees {
    pub(super) view: Option<()>,
    pub(super) repos: Remote<Vec<WorktreeRepo>>,
    pub(super) cache: HashMap<String, WorktreeStatus>,
    pub(super) open: Option<String>,
    pub(super) detail: Remote<WorktreeStatus>,
    pub(super) delete_branch: bool,
    pub(super) deleting: bool,
    pub(super) refreshing: bool,
    /// Falha de apagar: fica fora do `detail` para o painel continuar mostrando a worktree.
    pub(super) delete_error: Option<String>,
    /// Falha de "apagar as juntadas": é do repositório, mostrada na lista e nunca no painel de uma worktree.
    pub(super) batch_error: Option<String>,
}

impl Worktrees {
    pub(super) fn status(&self, path: &str) -> Option<&WorktreeStatus> { self.cache.get(path) }
}

impl Hangar {
    /// Abre a página (atalho ou botão); chamada de novo com ela aberta, fecha.
    pub(super) fn toggle_worktrees(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.worktrees.view.is_some() { self.close_worktrees(window, cx) } else { self.open_worktrees(window, cx) }
    }

    pub(super) fn open_worktrees(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connection_dialog { return; }
        self.close_settings(window, cx);
        self.close_costs(window, cx);
        self.close_popups();
        self.command_panel = false;
        self.worktrees.view = Some(());
        self.root_focus.focus(window, cx);
        self.load_worktrees(true, cx);
        cx.notify();
    }

    pub(super) fn close_worktrees(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.worktrees.view.take().is_some() {
            self.worktrees.open = None;
            self.root_focus.focus(window, cx);
            cx.notify();
        }
    }

    /// Outra conexão: as worktrees eram do servidor anterior. Com a página aberta, relê do novo.
    pub(super) fn worktrees_reconnected(&mut self, cx: &mut Context<Self>) {
        if self.worktrees.view.is_none() { self.worktrees = Default::default(); return; }
        self.worktrees.cache.clear();
        self.worktrees.open = None;
        self.load_worktrees(false, cx);
    }

    /// `then_fetch`: depois da primeira leitura, busca os remotos e relê, para "juntada" valer contra a base atual.
    fn load_worktrees(&mut self, then_fetch: bool, cx: &mut Context<Self>) {
        let seq = self.worktrees.repos.start();
        self.server_get(vec!["worktrees".into()], vec![], 30, cx, move |this, result, cx| {
            let parsed = result.and_then(|v| serde_json::from_value::<Vec<WorktreeRepo>>(v["repos"].clone())
                .map_err(|_| Failure::local("invalid_response")));
            if let Ok(repos) = &parsed {
                for w in repos.iter().flat_map(|r| &r.worktrees) { this.worktrees.cache.insert(w.path.clone(), w.clone()); }
                if then_fetch { this.fetch_worktrees(repos.iter().map(|r| r.repo.clone()).collect(), cx); }
            }
            this.worktrees.repos.finish(seq, parsed.map_err(|e| Self::failure(&e)));
            cx.notify();
        });
        cx.notify();
    }

    fn fetch_worktrees(&mut self, repos: Vec<String>, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else { return };
        self.worktrees.refreshing = true;
        let task = self.runtime.spawn(async move {
            // ponytail: fetch que falha (sem rede) só deixa a base como estava; a lista relida continua valendo.
            for repo in repos { let _ = api.server_send(reqwest::Method::POST, &["worktrees", "fetch"], Some(json!({"repo": repo})), 150).await; }
        });
        cx.spawn(async move |this, cx| {
            let _ = task.await;
            let _ = this.update(cx, |this, cx| { this.worktrees.refreshing = false; this.load_worktrees(false, cx); });
        }).detach();
    }

    pub(super) fn open_worktree(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.worktrees.view.is_none() { self.open_worktrees(window, cx); }
        // A página pode recusar abrir (diálogo de conexão): sem ela, nada de painel.
        if self.worktrees.view.is_none() { return; }
        self.worktrees.open = Some(path.clone());
        self.worktrees.delete_branch = false;
        self.worktrees.delete_error = None;
        let seq = self.worktrees.detail.start();
        self.server_get(vec!["worktrees".into(), "detail".into()], vec![("path".into(), path)], 30, cx, move |this, result, cx| {
            let parsed = result.and_then(|v| serde_json::from_value::<WorktreeStatus>(v).map_err(|_| Failure::local("invalid_response")));
            if let Ok(s) = &parsed { this.worktrees.cache.insert(s.path.clone(), s.clone()); }
            this.worktrees.detail.finish(seq, parsed.map_err(|e| Self::failure(&e)));
            cx.notify();
        });
        cx.notify();
    }

    fn close_worktree_panel(&mut self, cx: &mut Context<Self>) {
        self.worktrees.open = None;
        self.worktrees.delete_error = None;
        self.worktrees.detail.reset();
        cx.notify();
    }

    fn delete_worktree(&mut self, s: WorktreeStatus, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else { return };
        if self.worktrees.deleting { return; }
        self.worktrees.deleting = true;
        self.worktrees.delete_error = None;
        let body = json!({"repo": &s.repo, "path": &s.path, "confirm": !lost_files(&s).is_empty(),
                          "delete_branch": self.worktrees.delete_branch});
        let task = self.runtime.spawn(async move { api.server_send(reqwest::Method::POST, &["worktrees", "delete"], Some(body), 150).await });
        cx.spawn(async move |this, cx| {
            let joined = task.await;
            let _ = this.update(cx, |this, cx| {
                this.worktrees.deleting = false;
                match joined {
                    Ok(Ok(_)) => {
                        this.worktrees.cache.remove(&s.path);
                        if this.worktrees.open.as_deref() == Some(s.path.as_str()) { this.close_worktree_panel(cx); }
                        this.load_worktrees(false, cx);
                    }
                    Ok(Err(e)) => this.worktrees.delete_error = Some(Self::failure(&e)),
                    Err(_) => this.worktrees.delete_error = Some(tr("delivery_uncertain")),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    fn delete_merged(&mut self, repo: String, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else { return };
        if self.worktrees.deleting { return; }
        self.worktrees.deleting = true;
        self.worktrees.batch_error = None;
        let task = self.runtime.spawn(async move {
            api.server_send(reqwest::Method::POST, &["worktrees", "delete-merged"], Some(json!({"repo": repo})), 150).await
        });
        cx.spawn(async move |this, cx| {
            let joined = task.await;
            let _ = this.update(cx, |this, cx| {
                this.worktrees.deleting = false;
                match joined {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => this.worktrees.batch_error = Some(Self::failure(&e)),
                    Err(_) => this.worktrees.batch_error = Some(tr("delivery_uncertain")),
                }
                // Mesmo com falha parte pode ter saído: a lista relida mostra o que sobrou.
                this.load_worktrees(false, cx);
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    // ── Desenho ─────────────────────────────────────────────────────────────

    pub(super) fn render_worktrees(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let loading = self.worktrees.repos.loading;
        let refreshing = self.worktrees.refreshing;
        let header = self.costs_header(tr_shared("worktrees_titulo", &[]), |this, window, cx| this.close_worktrees(window, cx), vec![
            div().when(refreshing, |el| el.text_size(px(12.5)).text_color(theme::muted()).child(tr_shared("worktrees_atualizando", &[])))
                .into_any_element(),
            Button::new("worktrees-refresh").outline().small().label(tr_shared("custos_atualizar", &[])).loading(loading || refreshing)
                .disabled(loading || refreshing).on_click(cx.listener(|this, _, _, cx| this.load_worktrees(true, cx))).into_any_element(),
        ], cx);
        let body = self.render_worktrees_body(cx);
        let list = div().id("worktrees-scroll").flex_1().min_w_0().h_full().overflow_y_scroll()
            .child(div().w_full().flex().justify_center()
                .child(div().w_full().max_w(px(960.)).px(px(24.)).pt(px(20.)).pb(px(48.)).flex().flex_col().gap(px(16.)).child(body)));
        let panel = self.worktrees.open.clone().map(|path| self.render_worktree_panel(&path, cx));
        page_frame(div().size_full().flex().flex_col().child(header)
            .child(div().flex_1().min_h_0().flex().child(list).children(panel)))
    }

    fn render_worktrees_body(&self, cx: &mut Context<Self>) -> Div {
        let page = div().flex().flex_col().gap(px(16.))
            .when_some(self.worktrees.batch_error.clone(), |el, error| el.child(note_box(error, theme::danger())));
        let repos = match &self.worktrees.repos.value {
            None => return page.child(loading_state()),
            Some(Err(error)) => return page.child(error_state(tr_shared("worktrees_erro", &[("motivo", error.as_str())]),
                cx.listener(|this, _, _, cx| this.load_worktrees(false, cx)))),
            Some(Ok(repos)) => repos,
        };
        if repos.iter().all(|r| r.worktrees.is_empty()) { return page.child(empty_state(tr_shared("worktrees_vazio", &[]))); }
        let cards: Vec<Div> = repos.iter().filter(|r| !r.worktrees.is_empty()).map(|r| self.render_worktree_repo(r, cx)).collect();
        page.children(cards)
    }

    fn render_worktree_repo(&self, repo: &WorktreeRepo, cx: &mut Context<Self>) -> Div {
        let clean = clean_merged(&repo.worktrees);
        let repo_path = repo.repo.clone();
        let deleting = self.worktrees.deleting;
        let title = div().flex().items_center().gap(px(8.))
            .child(div().flex_1().min_w_0().truncate().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(base_name(&repo.repo)))
            .when(clean > 0, |el| el.child(Button::new(SharedString::from(format!("worktrees-clean-{}", repo.repo))).outline().small()
                .label(tr_shared("worktree_apagar_juntadas", &[("n", &clean.to_string())])).loading(deleting).disabled(deleting)
                .on_click(cx.listener(move |this, _, _, cx| this.delete_merged(repo_path.clone(), cx)))));
        let rows: Vec<Stateful<Div>> = repo.worktrees.iter().map(|w| self.render_worktree_row(w, cx)).collect();
        card_plain().gap(px(4.)).child(title).children(rows)
    }

    fn render_worktree_row(&self, w: &WorktreeStatus, cx: &mut Context<Self>) -> Stateful<Div> {
        let path = w.path.clone();
        let open = self.worktrees.open.as_deref() == Some(w.path.as_str());
        let mut facts = vec![branch_line(w),
            if w.merged { tr_shared("worktree_juntada", &[]) } else { tr_shared("worktree_nao_juntada", &[("n", &w.ahead.to_string())]) }];
        if w.dirty > 0 { facts.push(tr_shared("worktree_nao_commitados", &[("n", &w.dirty.to_string())])); }
        if !w.exists { facts.push(tr_shared("worktree_apagada", &[])); }
        div().id(SharedString::from(format!("worktree-row-{}", w.path))).px(px(10.)).py(px(8.)).rounded(px(8.)).flex().flex_col().gap(px(2.))
            .cursor_pointer()
            .map(|el| if open { el.bg(theme::elevated()) } else { el.hover(|el| el.bg(theme::hover())) })
            .child(div().flex().items_center().gap(px(6.))
                .child(chrome::small_icon(IconName::GitBranch, 13., theme::muted()))
                .child(div().min_w_0().truncate().text_size(px(13.5)).font_weight(FontWeight::MEDIUM).child(base_name(&w.path))))
            .child(div().pl(px(19.)).min_w_0().truncate().text_size(px(12.)).text_color(theme::muted()).child(facts.join(" · ")))
            .on_click(cx.listener(move |this, _, window, cx| this.open_worktree(path.clone(), window, cx)))
    }

    /// O painel da worktree aberta, no mesmo texto da janela do web: sessão aberta bloqueia; senão, o que se perde.
    fn render_worktree_panel(&self, path: &str, cx: &mut Context<Self>) -> Stateful<Div> {
        let head = div().flex().items_center().gap(px(8.))
            .child(div().flex_1().min_w_0().truncate().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(base_name(path)))
            .child(Button::new("worktree-panel-close").ghost().small().icon(IconName::Close).accessibility_label(tr("close"))
                .on_click(cx.listener(|this, _, _, cx| this.close_worktree_panel(cx))));
        let panel = div().id("worktree-panel").w(px(360.)).flex_shrink_0().h_full().overflow_y_scroll().border_l_1().border_color(theme::border())
            .p(px(16.)).flex().flex_col().gap(px(8.)).text_size(px(13.)).whitespace_normal().child(head);
        let line = |text: String| div().child(text);
        let muted = |text: String| div().text_color(theme::muted()).child(text);
        let warn = |text: String| div().text_color(theme::warning()).child(text);
        let st = match self.worktrees.detail.value.as_ref().filter(|_| !self.worktrees.detail.loading) {
            None => return panel.child(loading_state()),
            Some(Err(error)) => return panel.child(note_box(tr_shared("worktrees_erro", &[("motivo", error.as_str())]), theme::warning())),
            Some(Ok(st)) => st.clone(),
        };
        let sessions = st.sessions.join(", ");
        let panel = panel.child(line(branch_line(&st)))
            .child(line(if st.merged { tr_shared("worktree_juntada", &[]) } else { tr_shared("worktree_nao_juntada", &[("n", &st.ahead.to_string())]) }))
            .when(!st.sessions.is_empty(), |el| el.child(line(tr_shared("worktree_sessao_aberta", &[("nomes", &sessions)]))))
            .when(st.closed > 0, |el| el.child(muted(tr_shared("worktree_conversas_fechadas", &[("n", &st.closed.to_string())]))));
        if !st.sessions.is_empty() { return panel.child(warn(tr_shared("worktree_bloqueada", &[("nomes", &sessions)]))); }
        let lost = lost_files(&st);
        let (branch, base) = (st.branch.clone().unwrap_or_default(), st.base.clone().unwrap_or_default());
        let deleting = self.worktrees.deleting;
        let target = st.clone();
        panel
            .when(!lost.is_empty(), |el| el.child(warn(tr_shared("worktree_apagar_perde", &[])))
                .children(lost.into_iter().map(|f| div().pl(px(8.)).font_family(theme::MONO).text_size(px(12.)).child(format!("• {f}")))))
            .child(muted(tr_shared("worktree_apagar_conversas", &[("branch", st.main_branch.as_deref().unwrap_or(&base))])))
            .map(|el| if st.merged {
                el.child(muted(tr_shared("worktree_apagar_branch_juntada", &[("branch", &branch), ("base", &base)])))
            } else {
                el.child(muted(tr_shared("worktree_apagar_branch_fica", &[("branch", &branch)])))
                    .child(Checkbox::new("worktree-delete-branch").label(tr_shared("worktree_apagar_branch_tambem", &[("n", &st.ahead.to_string())]))
                        .checked(self.worktrees.delete_branch).disabled(deleting)
                        .on_change(cx.listener(|this, checked: &bool, _, cx| { this.worktrees.delete_branch = *checked; cx.notify(); })))
            })
            .when_some(self.worktrees.delete_error.clone(), |el, error| el.child(div().text_color(theme::danger()).child(error)))
            .child(div().pt(px(4.)).flex().justify_end()
                .child(Button::new("worktree-delete").danger().small().label(tr_shared("worktree_apagar", &[])).loading(deleting).disabled(deleting)
                    .on_click(cx.listener(move |this, _, _, cx| this.delete_worktree(target.clone(), cx)))))
    }
}

#[cfg(test)]
mod tests {
    use super::{clean_merged, lost_files, WorktreeStatus};
    use serde_json::json;

    fn st(v: serde_json::Value) -> WorktreeStatus { serde_json::from_value(v).unwrap() }

    #[test]
    fn lost_lists_dirty_count_and_ignored() {
        let s = st(json!({"path": "/r/x", "repo": "/r", "exists": true, "branch": "x", "base": "main", "merged": false,
                          "ahead": 2, "dirty": 3, "ignored": [".env.local"], "sessions": [], "closed": 0}));
        assert_eq!(lost_files(&s).len(), 2);
    }

    #[test]
    fn clean_merged_skips_dirty_and_open() {
        let ok = st(json!({"path": "/r/a", "repo": "/r", "exists": true, "branch": "a", "base": "main", "merged": true,
                           "ahead": 0, "dirty": 0, "ignored": [], "sessions": [], "closed": 1}));
        let busy = st(json!({"path": "/r/b", "repo": "/r", "exists": true, "branch": "b", "base": "main", "merged": true,
                             "ahead": 0, "dirty": 0, "ignored": [], "sessions": ["s1"], "closed": 0}));
        assert_eq!(clean_merged(&[ok, busy]), 1);
    }

    #[test]
    fn main_branch_is_optional() {
        let mut v = json!({"path": "/r/x", "repo": "/r", "exists": true, "branch": "x", "base": "main", "merged": false,
                           "ahead": 0, "dirty": 0, "closed": 0});
        assert_eq!(st(v.clone()).main_branch, None);
        v["main_branch"] = json!("dev");
        assert_eq!(st(v).main_branch.as_deref(), Some("dev"));
    }
}
