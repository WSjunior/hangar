//! Worktrees: página própria no molde de Custos. Contadores, onde está o disco e as worktrees de cada repositório numa
//! tabela; ao lado, o painel da que foi aberta (pela lista ou pelo chip da linha da conversa) com o que se perde ao apagá-la.
use super::*;
use super::costs::{card_plain, dec, empty_state, error_state, loading_state, note_box, page_frame, stack_bar, swatch};
use super::device::Remote;
use super::sidebar::Target;
use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub(super) struct WorktreeCommit { pub sha: String, #[serde(default)] pub subject: String, pub at: i64 }

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub(super) struct DirtyFile { pub code: String, pub path: String }

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub(super) struct BiggestDir { pub name: String, pub bytes: u64 }

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
    /// Leitura que falhou: dirty/ignored podem estar zerados sem ser verdade.
    #[serde(default)]
    pub degraded: bool,
    pub ahead: i64,
    #[serde(default)]
    pub behind: i64,
    pub dirty: i64,
    #[serde(default)]
    pub dirty_files: Vec<DirtyFile>,
    #[serde(default)]
    pub ignored: Vec<String>,
    #[serde(default)]
    pub last_commit: Option<WorktreeCommit>,
    #[serde(default)]
    pub commits: Vec<WorktreeCommit>,
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub size_biggest: Option<BiggestDir>,
    #[serde(default)]
    pub size_pending: bool,
    /// A medição falhou; o backend tenta de novo depois.
    #[serde(default)]
    pub size_error: bool,
    /// Houve pasta ilegível: o tamanho é um mínimo.
    #[serde(default)]
    pub size_partial: bool,
    #[serde(default)]
    pub sessions: Vec<String>,
    pub closed: i64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub(super) struct WorktreeRepo { pub repo: String, pub worktrees: Vec<WorktreeStatus> }

/// Branches da pasta principal (`/api/fs/branches`), para a base e o "Continuar uma branch".
#[derive(Clone, Debug, Default, Deserialize)]
pub(super) struct BranchList { current: Option<String>, #[serde(default)] branches: Vec<String>, #[serde(default)] remotes: Vec<String> }

/// Um estado só por worktree, na ordem do que mais importa antes de apagar (o `worktreeState` do core).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WtState { Gone, Session, Dirty, Merged, Detached, Active }

pub(super) fn state_of(w: &WorktreeStatus) -> WtState {
    if !w.exists { WtState::Gone }
    else if !w.sessions.is_empty() { WtState::Session }
    else if w.dirty > 0 { WtState::Dirty }
    else if w.merged { WtState::Merged }
    else if w.branch.is_none() { WtState::Detached }
    else { WtState::Active }
}

impl WtState {
    fn label(self) -> String {
        tr_shared(match self {
            WtState::Gone => "worktree_estado_sumida", WtState::Session => "worktree_estado_em_uso",
            WtState::Dirty => "worktree_estado_nao_commitado", WtState::Merged => "worktree_estado_mesclada",
            WtState::Detached => "worktree_estado_sem_branch", WtState::Active => "worktree_estado_andamento",
        }, &[])
    }
    pub(super) fn color(self) -> Hsla {
        match self {
            WtState::Gone => theme::danger(), WtState::Session => theme::accent(), WtState::Dirty => theme::warning(),
            WtState::Merged => theme::success(), WtState::Detached => theme::faint(), WtState::Active => theme::muted(),
        }
    }
    fn verdict(self, w: &WorktreeStatus) -> String {
        match self {
            WtState::Gone => tr_shared("worktree_veredito_sumida", &[]),
            WtState::Session => tr_shared("worktree_veredito_em_uso", &[]),
            WtState::Dirty => tr_shared("worktree_veredito_nao_commitado", &[("n", &w.dirty.to_string())]),
            WtState::Merged => tr_shared("worktree_veredito_mesclada", &[]),
            WtState::Detached => tr_shared("worktree_veredito_sem_branch", &[]),
            WtState::Active => tr_shared("worktree_veredito_andamento", &[("n", &w.ahead.to_string())]),
        }
    }
}

pub(super) const STALE_DAYS: i64 = 14;

/// Instante da última atividade: o último commit; sem commit lido, a criação.
fn activity_at(w: &WorktreeStatus) -> Option<i64> { w.last_commit.as_ref().map(|c| c.at).or(w.created_at) }

pub(super) fn age_days(w: &WorktreeStatus, now: f64) -> Option<i64> {
    activity_at(w).map(|at| ((now - at as f64) / 86_400.).floor().max(0.) as i64)
}

pub(super) fn stale_days(w: &WorktreeStatus, now: f64) -> Option<i64> { age_days(w, now).filter(|d| *d > STALE_DAYS) }

/// Pronta para apagar sem perder nada.
pub(super) fn ready(w: &WorktreeStatus) -> bool { w.merged && w.dirty == 0 && w.sessions.is_empty() && !w.degraded }

/// Worktree que o Claude cria para um subagente: nome gerado, agrupada à parte.
pub(super) fn is_agent(w: &WorktreeStatus) -> bool {
    base_name(&w.path).strip_prefix("agent-")
        .is_some_and(|h| h.len() >= 8 && h.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)))
}

/// Nome para mostrar: a branch quando a pasta tem nome gerado e a branch diz algo.
pub(super) fn title(w: &WorktreeStatus) -> String {
    match w.branch.as_deref() {
        Some(b) if is_agent(w) && !b.starts_with("worktree-agent-") => b.to_owned(),
        _ => base_name(&w.path),
    }
}

fn loses(w: &WorktreeStatus) -> bool { w.dirty > 0 || !w.ignored.is_empty() }

/// O lote de mescladas: as que entram na confirmação e as que ficam de fora (sessão aberta ou leitura falha).
pub(super) fn merged_batch(list: &[WorktreeStatus]) -> (Vec<WorktreeStatus>, Vec<WorktreeStatus>) {
    list.iter().filter(|w| w.merged).cloned().partition(|w| w.sessions.is_empty() && !w.degraded)
}

/// O que se perde ao apagar: a contagem de não commitados e os ignorados pelo nome.
pub(super) fn lost_files(s: &WorktreeStatus) -> Vec<String> {
    let mut out = Vec::new();
    if s.dirty > 0 { out.push(tr_shared("worktree_nao_commitados", &[("n", &s.dirty.to_string())])); }
    out.extend(s.ignored.iter().cloned());
    out
}

/// Pasta da worktree nova: a descrição em minúsculas, sem acento, com `-` entre as palavras.
pub(super) fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in super::create::sanitize(&text.to_lowercase()).chars() {
        let c = if c == '_' { '-' } else { c };
        if c == '-' && (out.is_empty() || out.ends_with('-')) { continue; }
        out.push(c);
    }
    out.chars().take(40).collect::<String>().trim_end_matches('-').to_owned()
}

pub(super) fn fmt_size(bytes: u64) -> String {
    const MB: f64 = 1024. * 1024.;
    let b = bytes as f64;
    if b >= 1024. * MB { format!("{} GB", dec(b / (1024. * MB), 1)) }
    else if b >= MB { format!("{} MB", dec(b / MB, 0)) }
    else { format!("{} KB", dec(b / 1024., 0)) }
}

fn sum_size<'a>(list: impl IntoIterator<Item = &'a WorktreeStatus>) -> u64 { list.into_iter().filter_map(|w| w.size).sum() }

/// Soma para mostrar: com alguma medida falha ou parcial, vira um mínimo ("≥").
fn sum_label<'a>(list: impl IntoIterator<Item = &'a WorktreeStatus>) -> String {
    let (mut total, mut partial) = (0, false);
    for w in list {
        total += w.size.unwrap_or(0);
        partial |= w.size_partial || w.size_error;
    }
    if partial { tr_shared("worktree_tamanho_parcial", &[("tamanho", &fmt_size(total))]) } else { fmt_size(total) }
}

/// Caminho com `/` e sem separador no fim: o backend no Windows devolve `\`.
fn norm_path(path: &str) -> String { path.replace('\\', "/").trim_end_matches('/').to_owned() }

fn base_name(path: &str) -> String {
    let p = norm_path(path);
    p.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or(path).to_owned()
}

/// `path` é `root` ou fica dentro dele.
pub(super) fn inside(path: &str, root: &str) -> bool {
    let (path, root) = (norm_path(path), norm_path(root));
    path == root || path.starts_with(&format!("{root}/"))
}

fn short_date(epoch: i64) -> String {
    chrono::DateTime::from_timestamp(epoch, 0).map(|t| t.with_timezone(&chrono::Local).format("%d/%m").to_string()).unwrap_or_default()
}

fn ago_at(at: i64, now: f64) -> String { super::side::ago(now - at as f64) }

fn bar_width(n: i64) -> f32 { if n <= 0 { 0. } else { (6. + ((n + 1) as f32).log2() * 7.).min(60.) } }

fn age_color(days: i64) -> Hsla {
    if days < 1 { theme::success() } else if days < 7 { theme::accent() } else if days < 30 { theme::muted() } else { theme::warning_text() }
}

fn chip(text: String, color: Hsla) -> Div {
    div().flex_shrink_0().px(px(6.)).py(px(1.)).rounded(px(6.)).bg(color.alpha(0.14)).text_size(px(11.5)).text_color(color).child(text)
}

fn dot(color: Hsla) -> Div { div().size(px(7.)).flex_shrink_0().rounded_full().bg(color) }

fn section(text: String) -> Div {
    div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::faint()).child(text.to_uppercase())
}

fn muted(text: String) -> Div { div().text_size(px(12.)).text_color(theme::muted()).whitespace_normal().child(text) }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Filter { #[default] All, InUse, Dirty, Ready, Stale, Detached }

impl Filter {
    const ALL: [Filter; 6] = [Filter::All, Filter::InUse, Filter::Dirty, Filter::Ready, Filter::Stale, Filter::Detached];
    fn label(self) -> String {
        tr_shared(match self {
            Filter::All => "worktrees_filtro_todas", Filter::InUse => "worktrees_filtro_em_uso", Filter::Dirty => "worktrees_filtro_nao_commitado",
            Filter::Ready => "worktrees_filtro_prontas", Filter::Stale => "worktrees_filtro_paradas", Filter::Detached => "worktrees_filtro_sem_branch",
        }, &[])
    }
    fn matches(self, w: &WorktreeStatus, now: f64) -> bool {
        match self {
            Filter::All => true,
            Filter::InUse => !w.sessions.is_empty(),
            Filter::Dirty => w.dirty > 0,
            Filter::Ready => ready(w),
            Filter::Stale => stale_days(w, now).is_some(),
            Filter::Detached => w.branch.is_none(),
        }
    }
}

/// O diálogo aberto pela página; só um por vez.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum WtDialog { Delete(String), Batch, Create }

/// A confirmação do lote congela o que a pessoa viu: o lote apaga essas e só essas.
pub(super) struct Batch { repo: String, deletable: Vec<WorktreeStatus>, blocked: Vec<WorktreeStatus> }

/// O diálogo "Nova worktree" de um repositório.
pub(super) struct NewWorktree {
    repo: String,
    existing_mode: bool,
    task: Entity<InputState>,
    branch: Entity<InputState>,
    folder: Entity<InputState>,
    edit_branch: bool,
    edit_folder: bool,
    base: String,
    fetch: bool,
    session: bool,
    pick: String,
    branches: Remote<BranchList>,
    busy: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

/// O que a criação vai pedir: branch, sufixo da pasta, se a branch é nova, a base e se busca o remoto antes.
struct Plan { branch: String, name: String, new_branch: bool, base: String, fetch: bool }

impl NewWorktree {
    fn plan(&self, cx: &App) -> Option<Plan> {
        let value = |input: &Entity<InputState>| input.read(cx).value().trim().to_owned();
        let plan = if self.existing_mode {
            let last = self.pick.rsplit('/').next().unwrap_or(&self.pick);
            Plan { branch: self.pick.clone(), name: slug(last), new_branch: false, base: String::new(), fetch: false }
        } else {
            let s = slug(&value(&self.task));
            Plan { branch: if self.edit_branch { value(&self.branch) } else if s.is_empty() { String::new() } else { format!("feat/{s}") },
                name: if self.edit_folder { slug(&value(&self.folder)) } else { s }, new_branch: true, base: self.base.clone(), fetch: self.fetch }
        };
        (!plan.branch.is_empty() && !plan.name.is_empty()).then_some(plan)
    }
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
    /// Falha ou sobra do lote de mescladas: é do repositório, mostrada na lista e nunca no painel de uma worktree.
    pub(super) batch_error: Option<String>,
    filter: Filter,
    search: Option<Entity<InputState>>,
    _search_events: Option<Subscription>,
    agents_open: HashSet<String>,
    dialog: Option<WtDialog>,
    batch: Option<Batch>,
    batch_dialog_error: Option<String>,
    create: Option<NewWorktree>,
    /// Releitura marcada enquanto o backend mede o espaço de alguma.
    size_poll: bool,
    size_tries: u32,
    /// Repositórios cujo fetch falhou na última atualização.
    fetch_failed: Vec<String>,
}

impl Worktrees {
    pub(super) fn status(&self, path: &str) -> Option<&WorktreeStatus> { self.cache.get(path) }
    fn busy(&self) -> bool { self.deleting || self.create.as_ref().is_some_and(|c| c.busy) }
}

/// Corpo dos diálogos da página: desenhado pelo `Hangar` a cada quadro, como o painel de grupo.
struct DialogBody { hangar: WeakEntity<Hangar>, draw: fn(&mut Hangar, &mut Window, &mut Context<Hangar>) -> Div, _observe: Subscription }

impl Render for DialogBody {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let draw = self.draw;
        self.hangar.update(cx, |hangar, cx| draw(hangar, window, cx)).unwrap_or_else(|_| div())
    }
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
        self.worktrees.size_tries = 0;
        if self.worktrees.search.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(tr_shared("worktrees_buscar", &[])).clean_on_escape());
            self.worktrees._search_events = Some(cx.subscribe(&input, |_, _, event: &InputEvent, cx| if matches!(event, InputEvent::Change) { cx.notify() }));
            self.worktrees.search = Some(input);
        }
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

    /// `then_fetch`: depois da primeira leitura, busca os remotos e relê, para "mesclada" valer contra a base atual.
    fn load_worktrees(&mut self, then_fetch: bool, cx: &mut Context<Self>) {
        let seq = self.worktrees.repos.start();
        self.server_get(vec!["worktrees".into()], vec![], 30, cx, move |this, result, cx| {
            let parsed = result.and_then(|v| serde_json::from_value::<Vec<WorktreeRepo>>(v["repos"].clone())
                .map_err(|_| Failure::local("invalid_response")));
            if let Ok(repos) = &parsed {
                for w in repos.iter().flat_map(|r| &r.worktrees) { this.worktrees.cache.insert(w.path.clone(), w.clone()); }
                if then_fetch { this.fetch_worktrees(repos.iter().map(|r| r.repo.clone()).collect(), cx); }
                if repos.iter().flat_map(|r| &r.worktrees).any(|w| w.size_pending) { this.poll_sizes(cx); }
                else { this.worktrees.size_tries = 0; }
            }
            this.worktrees.repos.finish(seq, parsed.map_err(|e| Self::failure(&e)));
            cx.notify();
        });
        cx.notify();
    }

    /// O backend mede o espaço em segundo plano: relê a lista a cada 5 s enquanto faltar medida, até 12 vezes.
    fn poll_sizes(&mut self, cx: &mut Context<Self>) {
        if self.worktrees.size_poll || self.worktrees.size_tries >= SIZE_TRIES { return; }
        self.worktrees.size_poll = true;
        self.worktrees.size_tries += 1;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            let _ = this.update(cx, |this, cx| {
                this.worktrees.size_poll = false;
                if this.worktrees.view.is_some() && !this.worktrees.repos.loading { this.load_worktrees(false, cx); }
            });
        }).detach();
    }

    fn fetch_worktrees(&mut self, repos: Vec<String>, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else { return };
        self.worktrees.refreshing = true;
        self.worktrees.fetch_failed.clear();
        let task = self.runtime.spawn(async move {
            // Fetch que falha deixa a base como estava: a lista relida vale, com o aviso de que pode estar desatualizada.
            let mut failed = Vec::new();
            for repo in repos {
                if api.server_send(reqwest::Method::POST, &["worktrees", "fetch"], Some(json!({"repo": &repo})), 150).await.is_err() { failed.push(repo); }
            }
            failed
        });
        cx.spawn(async move |this, cx| {
            let failed = task.await.unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.worktrees.refreshing = false;
                this.worktrees.fetch_failed = failed;
                this.load_worktrees(false, cx);
            });
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

    /// O chip da sessão: a página fecha e a conversa dela abre.
    fn go_to_session(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let target = Target::new(&self.active_key(), &name);
        self.close_worktrees(window, cx);
        self.open_target(&target, window, cx);
    }

    /// "Nova sessão aqui": a tela sem sessão na pasta principal, com esta worktree já escolhida na pílula de branch.
    fn new_session_here(&mut self, w: &WorktreeStatus, window: &mut Window, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else { return };
        self.close_worktrees(window, cx);
        self.go_home(window, cx);
        // A tela sem sessão pode estar em outra máquina: a worktree é desta, então ela recomeça aqui.
        if self.new_chat.as_ref().is_some_and(|v| v.read(cx).machine() != api.identity()) { self.new_chat = None; }
        let view = self.ensure_new_chat(api, window, cx);
        let (repo, path) = (w.repo.clone(), w.path.clone());
        view.update(cx, |view, cx| view.preset_folder(repo, Some(path), window, cx));
        cx.notify();
    }

    fn open_wt_dialog(&mut self, kind: WtDialog, width: f32, draw: fn(&mut Hangar, &mut Window, &mut Context<Hangar>) -> Div,
        window: &mut Window, cx: &mut Context<Self>) {
        self.worktrees.dialog = Some(kind);
        let hangar = cx.entity();
        let body = cx.new(|cx| DialogBody { _observe: cx.observe(&hangar, |_, _, cx| cx.notify()), hangar: hangar.downgrade(), draw });
        let weak = hangar.downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            // Com o pedido em voo o diálogo não fecha: a resposta (erro, sobra do lote) cairia no vazio.
            let busy = weak.upgrade().is_some_and(|h| h.read(cx).worktrees.busy());
            let close = weak.clone();
            popup::dialog(dialog).w(px(width)).keyboard(!busy).overlay_closable(!busy).close_button(!busy).child(body.clone())
                .on_close(move |_, _, cx| { let _ = close.update(cx, |this, cx| {
                    this.worktrees.dialog = None;
                    this.worktrees.batch = None;
                    this.worktrees.create = None;
                    cx.notify();
                }); })
        });
        cx.notify();
    }

    /// Fecha o diálogo da página se ele ainda é o que a resposta esperava.
    fn finish_wt_dialog(&mut self, kind: &WtDialog, window: &mut Window, cx: &mut Context<Self>) {
        if self.worktrees.dialog.as_ref() == Some(kind) {
            self.worktrees.dialog = None;
            window.close_dialog(cx);
        }
    }

    /// Cancelar: o fechar pelo programa não passa pelo `on_close`, então o estado do diálogo sai aqui.
    fn cancel_wt_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.worktrees.busy() { return; }
        (self.worktrees.batch, self.worktrees.create) = (None, None);
        if let Some(kind) = self.worktrees.dialog.clone() { self.finish_wt_dialog(&kind, window, cx); }
        cx.notify();
    }

    // ── Apagar ──────────────────────────────────────────────────────────────

    fn open_delete(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.worktrees.delete_branch = false;
        self.worktrees.delete_error = None;
        self.open_wt_dialog(WtDialog::Delete(path), 520., Hangar::render_delete_dialog, window, cx);
    }

    fn delete_worktree(&mut self, s: WorktreeStatus, window: &mut Window, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else { return };
        if self.worktrees.deleting { return; }
        self.worktrees.deleting = true;
        self.worktrees.delete_error = None;
        let body = json!({"repo": &s.repo, "path": &s.path, "confirm": !lost_files(&s).is_empty(),
                          "delete_branch": self.worktrees.delete_branch});
        let task = self.runtime.spawn(async move { api.server_send(reqwest::Method::POST, &["worktrees", "delete"], Some(body), 150).await });
        cx.spawn_in(window, async move |this, cx| {
            let joined = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.worktrees.deleting = false;
                match joined {
                    Ok(Ok(_)) => {
                        this.worktrees.cache.remove(&s.path);
                        this.finish_wt_dialog(&WtDialog::Delete(s.path.clone()), window, cx);
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

    fn open_batch(&mut self, repo: &WorktreeRepo, window: &mut Window, cx: &mut Context<Self>) {
        let (deletable, blocked) = merged_batch(&repo.worktrees);
        if deletable.is_empty() { return; }
        self.worktrees.batch = Some(Batch { repo: repo.repo.clone(), deletable, blocked });
        self.worktrees.batch_dialog_error = None;
        self.open_wt_dialog(WtDialog::Batch, 520., Hangar::render_batch_dialog, window, cx);
    }

    /// Manda só as que a confirmação mostrou; perde arquivos só a que ela mostrou perdendo.
    fn delete_batch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else { return };
        let Some(batch) = self.worktrees.batch.as_ref() else { return };
        if self.worktrees.deleting { return; }
        let shown: Vec<(String, String)> = batch.deletable.iter().map(|w| (w.path.clone(), base_name(&w.path))).collect();
        let body = json!({"repo": &batch.repo, "confirm": true,
            "paths": batch.deletable.iter().map(|w| &w.path).collect::<Vec<_>>(),
            "lossy": batch.deletable.iter().filter(|w| loses(w)).map(|w| &w.path).collect::<Vec<_>>()});
        self.worktrees.deleting = true;
        self.worktrees.batch_dialog_error = None;
        self.worktrees.batch_error = None;
        let task = self.runtime.spawn(async move { api.server_send(reqwest::Method::POST, &["worktrees", "delete-merged"], Some(body), 150).await });
        cx.spawn_in(window, async move |this, cx| {
            let joined = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.worktrees.deleting = false;
                match joined {
                    Ok(Ok(v)) => {
                        let removed: Vec<String> = v.get("removed").and_then(|r| serde_json::from_value(r.clone()).ok()).unwrap_or_default();
                        for p in &removed { this.worktrees.cache.remove(p); }
                        let left: Vec<String> = shown.iter().filter(|(p, _)| !removed.contains(p)).map(|(_, n)| n.clone()).collect();
                        if !left.is_empty() { this.worktrees.batch_error = Some(tr_shared("worktree_lote_nao_apagou", &[("nomes", &left.join(", "))])); }
                        if this.worktrees.open.as_ref().is_some_and(|p| removed.contains(p)) { this.close_worktree_panel(cx); }
                        this.worktrees.batch = None;
                        this.finish_wt_dialog(&WtDialog::Batch, window, cx);
                    }
                    Ok(Err(e)) => this.worktrees.batch_dialog_error = Some(Self::failure(&e)),
                    Err(_) => this.worktrees.batch_dialog_error = Some(tr("delivery_uncertain")),
                }
                // Mesmo com falha parte pode ter saído: a lista relida mostra o que sobrou.
                this.load_worktrees(false, cx);
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    // ── Nova worktree ───────────────────────────────────────────────────────

    fn open_create(&mut self, repo: String, window: &mut Window, cx: &mut Context<Self>) {
        let task = cx.new(|cx| InputState::new(window, cx));
        let branch = cx.new(|cx| InputState::new(window, cx).placeholder(tr_shared("worktree_nome_branch", &[])));
        let folder = cx.new(|cx| InputState::new(window, cx));
        let notify = |this: &mut Hangar, _: Entity<InputState>, event: &InputEvent, cx: &mut Context<Hangar>| {
            if matches!(event, InputEvent::Change) { if let Some(c) = this.worktrees.create.as_mut() { c.error = None; } cx.notify() }
        };
        let subscriptions = vec![cx.subscribe(&task, notify), cx.subscribe(&branch, notify), cx.subscribe(&folder, notify)];
        let focus = task.clone();
        cx.defer_in(window, move |_, window, cx| focus.update(cx, |input, cx| input.focus(window, cx)));
        self.worktrees.create = Some(NewWorktree { repo, existing_mode: false, task, branch, folder, edit_branch: false, edit_folder: false,
            base: String::new(), fetch: true, session: true, pick: String::new(), branches: Remote::default(), busy: false, error: None,
            _subscriptions: subscriptions });
        self.load_create_branches(cx);
        self.open_wt_dialog(WtDialog::Create, 600., Hangar::render_create_dialog, window, cx);
    }

    /// A raiz autorizada que contém o repositório, depois as branches dele.
    fn load_create_branches(&mut self, cx: &mut Context<Self>) {
        let Some(c) = self.worktrees.create.as_mut() else { return };
        let seq = c.branches.start();
        let repo = c.repo.clone();
        let done = |this: &mut Hangar, seq: u64, result: Result<BranchList, String>| {
            let Some(c) = this.worktrees.create.as_mut() else { return };
            if c.branches.finish(seq, result) && c.base.is_empty() {
                c.base = c.branches.ok().and_then(|b| b.current.clone().or_else(|| b.branches.first().cloned())).unwrap_or_default();
            }
        };
        self.server_get(vec!["fs".into(), "roots".into()], vec![], 15, cx, move |this, result, cx| {
            let roots = match result {
                Ok(v) => v.as_array().cloned().unwrap_or_default(),
                Err(e) => { done(this, seq, Err(Self::failure(&e))); cx.notify(); return; }
            };
            let root = roots.iter().filter_map(|r| r.get("path").and_then(Value::as_str)).filter(|p| inside(&repo, p))
                .max_by_key(|p| p.len()).map(str::to_owned);
            let Some(root) = root else { done(this, seq, Err(tr("create_roots_failed"))); cx.notify(); return };
            this.server_get(vec!["fs".into(), "branches".into()], vec![("root".into(), root), ("path".into(), repo)], 30, cx, move |this, result, cx| {
                done(this, seq, result.map_err(|e| Self::failure(&e))
                    .and_then(|v| serde_json::from_value::<BranchList>(v).map_err(|_| tr("invalid_response"))));
                cx.notify();
            });
        });
    }

    fn submit_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else { return };
        let Some(c) = self.worktrees.create.as_ref() else { return };
        if c.busy { return; }
        let Some(plan) = c.plan(cx) else { return };
        let (repo, session) = (c.repo.clone(), c.session);
        let base = (plan.new_branch && !plan.base.is_empty()).then(|| plan.base.clone());
        if let Some(c) = self.worktrees.create.as_mut() { (c.busy, c.error) = (true, None); }
        let task = self.runtime.spawn(async move {
            if !session {
                let body = json!({"repo": repo, "branch": plan.branch, "name": plan.name, "new_branch": plan.new_branch,
                    "base": base, "fetch": plan.fetch});
                return api.server_send(reqwest::Method::POST, &["worktrees", "create"], Some(body), 150).await;
            }
            // Com sessão, o mesmo caminho da tela de nova sessão: a worktree nasce com ela, no nome dela.
            if plan.fetch { api.server_send(reqwest::Method::POST, &["worktrees", "fetch"], Some(json!({"repo": &repo})), 150).await?; }
            let mut body = json!({"name": plan.name, "cwd": repo, "provider": "claude", "branch": plan.branch, "new_branch": plan.new_branch});
            if let Some(base) = base { body["base"] = json!(base); }
            api.server_send(reqwest::Method::POST, &["sessions"], Some(body), 120).await
        });
        cx.spawn_in(window, async move |this, cx| {
            let joined = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                let result = match joined {
                    Ok(Ok(v)) if session => serde_json::from_value::<SessionInfo>(v).map(Ok).map_err(|_| tr("invalid_response")),
                    Ok(Ok(v)) => v.get("path").and_then(Value::as_str).map(|p| Err(p.to_owned())).ok_or_else(|| tr("invalid_response")),
                    Ok(Err(e)) => Err(Self::failure(&e)),
                    Err(_) => Err(tr("delivery_uncertain")),
                };
                if let Some(c) = this.worktrees.create.as_mut() { c.busy = false; }
                match result {
                    Ok(Ok(session)) => {
                        this.worktrees.create = None;
                        this.finish_wt_dialog(&WtDialog::Create, window, cx);
                        this.close_worktrees(window, cx);
                        let key = this.active_key();
                        if this.select_on(&key, session.clone(), window, cx) { this.focus_composer_for(&session, window, cx); }
                    }
                    Ok(Err(path)) => {
                        this.worktrees.create = None;
                        this.finish_wt_dialog(&WtDialog::Create, window, cx);
                        this.load_worktrees(false, cx);
                        this.open_worktree(path, window, cx);
                    }
                    Err(error) => if let Some(c) = this.worktrees.create.as_mut() { c.error = Some(error); },
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    // ── Desenho ─────────────────────────────────────────────────────────────

    pub(super) fn render_worktrees(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let loading = self.worktrees.repos.loading;
        let refreshing = self.worktrees.refreshing;
        let search = self.worktrees.search.clone().map(|input| Input::new(&input).small().w(px(240.)).cleanable(true)
            .aria_label(tr_shared("worktrees_buscar", &[])).prefix(chrome::small_icon(IconName::Search, 13., theme::faint())).into_any_element());
        let header = self.costs_header(tr_shared("worktrees_titulo", &[]), |this, window, cx| this.close_worktrees(window, cx),
            search.into_iter().chain([
                div().when(refreshing, |el| el.text_size(px(12.5)).text_color(theme::muted()).child(tr_shared("worktrees_atualizando", &[])))
                    .into_any_element(),
                Button::new("worktrees-refresh").outline().small().label(tr_shared("custos_atualizar", &[])).loading(loading || refreshing)
                    .disabled(loading || refreshing).on_click(cx.listener(|this, _, _, cx| this.load_worktrees(true, cx))).into_any_element(),
            ]).collect(), cx);
        let body = self.render_worktrees_body(cx);
        let list = div().id("worktrees-scroll").flex_1().min_w_0().h_full().overflow_y_scroll()
            .child(div().w_full().flex().justify_center()
                .child(div().w_full().max_w(px(1180.)).px(px(24.)).pt(px(20.)).pb(px(48.)).flex().flex_col().gap(px(16.)).child(body)));
        let panel = self.worktrees.open.clone().map(|path| self.render_worktree_panel(&path, cx));
        page_frame(div().size_full().flex().flex_col().child(header)
            .child(div().flex_1().min_h_0().flex().child(list).children(panel)))
    }

    fn wt_search_text(&self, cx: &App) -> String {
        self.worktrees.search.as_ref().map(|i| i.read(cx).value().trim().to_lowercase()).unwrap_or_default()
    }

    fn render_worktrees_body(&self, cx: &mut Context<Self>) -> Div {
        let page = div().flex().flex_col().gap(px(16.))
            .when_some(self.worktrees.batch_error.clone(), |el, error| el.child(note_box(error, theme::danger())))
            .children(self.worktrees.fetch_failed.iter().map(|repo| note_box(tr_shared("worktrees_busca_remoto_falhou", &[("repo", &base_name(repo))]), theme::warning())));
        let repos = match &self.worktrees.repos.value {
            None => return page.child(loading_state()),
            Some(Err(error)) => return page.child(error_state(tr_shared("worktrees_erro", &[("motivo", error.as_str())]),
                cx.listener(|this, _, _, cx| this.load_worktrees(false, cx)))),
            Some(Ok(repos)) => repos,
        };
        if repos.iter().all(|r| r.worktrees.is_empty()) { return page.child(empty_state(tr_shared("worktrees_vazio", &[]))); }
        let now = super::side::now_seconds();
        let all: Vec<&WorktreeStatus> = repos.iter().flat_map(|r| &r.worktrees).collect();
        let query = self.wt_search_text(cx);
        let filter = self.worktrees.filter;
        let shown = |w: &WorktreeStatus| filter.matches(w, now) && (query.is_empty()
            || [title(w), base_name(&w.path), w.branch.clone().unwrap_or_default(),
                w.last_commit.as_ref().map(|c| format!("{} {}", c.sha, c.subject)).unwrap_or_default()]
                .iter().any(|t| t.to_lowercase().contains(&query)));
        let cards: Vec<Div> = repos.iter().filter_map(|r| {
            let visible: Vec<&WorktreeStatus> = r.worktrees.iter().filter(|w| shown(w)).collect();
            (!visible.is_empty()).then(|| self.render_worktree_repo(r, visible, filter != Filter::All || !query.is_empty(), now, cx))
        }).collect();
        let none = cards.is_empty();
        page.child(self.render_counters(&all, now, cx))
            .children(self.render_disk(&all, cx))
            .child(self.render_wt_filters(&all, now, cx))
            .children(cards)
            .when(none, |el| el.child(empty_state(tr_shared("worktrees_filtro_vazio", &[]))))
            .child(div().text_size(px(12.)).text_color(theme::faint()).child(tr_shared("worktrees_legenda_commits", &[])))
    }

    fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        self.worktrees.filter = if self.worktrees.filter == filter { Filter::All } else { filter };
        cx.notify();
    }

    fn render_counters(&self, all: &[&WorktreeStatus], now: f64, cx: &mut Context<Self>) -> Div {
        let count = |f: Filter| all.iter().filter(|w| f.matches(w, now)).count();
        let in_use: Vec<&str> = all.iter().flat_map(|w| w.sessions.iter().map(String::as_str)).collect();
        let freed = sum_label(all.iter().copied().filter(|w| ready(w)));
        let boxes = [
            (Filter::InUse, "worktrees_contador_em_uso", theme::accent(),
                if in_use.is_empty() { tr_shared("worktrees_contador_em_uso_vazio", &[]) } else { in_use.join(" · ") }),
            (Filter::Dirty, "worktrees_contador_nao_commitado", theme::warning(), tr_shared("worktrees_contador_nao_commitado_dica", &[])),
            (Filter::Ready, "worktrees_contador_prontas", theme::success(), tr_shared("worktrees_contador_prontas_dica", &[("tamanho", &freed)])),
            (Filter::Stale, "worktrees_contador_paradas", theme::warning_text(), tr_shared("worktrees_contador_paradas_dica", &[])),
        ];
        div().flex().gap(px(12.)).children(boxes.into_iter().map(|(f, key, color, hint)| {
            let on = self.worktrees.filter == f;
            div().id(SharedString::from(format!("worktrees-count-{key}"))).flex_1().min_w_0().p(px(14.)).rounded(px(12.)).border_1()
                .border_color(if on { color } else { theme::border() }).bg(theme::boxed()).cursor_pointer().hover(|el| el.bg(theme::hover()))
                .flex().flex_col().gap(px(4.))
                .child(div().flex().items_center().gap(px(6.)).child(dot(color))
                    .child(div().min_w_0().truncate().text_size(px(12.5)).text_color(theme::muted()).child(tr_shared(key, &[]))))
                .child(div().text_size(px(26.)).font_weight(FontWeight::SEMIBOLD).child(count(f).to_string()))
                .child(div().min_w_0().truncate().text_size(px(12.)).text_color(theme::muted()).child(hint))
                .on_click(cx.listener(move |this, _, _, cx| this.set_filter(f, cx)))
        }))
    }

    fn render_disk(&self, all: &[&WorktreeStatus], cx: &mut Context<Self>) -> Option<Div> {
        let mut sized: Vec<&WorktreeStatus> = all.iter().copied().filter(|w| w.size.is_some_and(|s| s > 0)).collect();
        let pending = all.iter().any(|w| w.size_pending);
        if sized.is_empty() && !pending { return None; }
        sized.sort_by_key(|w| std::cmp::Reverse(w.size.unwrap_or(0)));
        let total = sum_label(sized.iter().copied());
        let freed = sum_size(all.iter().copied().filter(|w| ready(w)));
        let freed_text = sum_label(all.iter().copied().filter(|w| ready(w)));
        // Teto de releituras atingido com medida pendente: diz que demora e deixa tentar de novo.
        let slow = pending && self.worktrees.size_tries >= SIZE_TRIES;
        let max = sized.first().and_then(|w| w.size).unwrap_or(1).max(1) as f32;
        let (top, rest) = sized.split_at(sized.len().min(6));
        let line = |name: String, state: String, bytes: u64, text: String, color: Hsla| div().w_full().flex().items_center().gap(px(10.)).text_size(px(12.5))
            .child(dot(color))
            .child(div().w(px(220.)).flex_shrink_0().min_w_0().truncate().child(name))
            .child(div().w(px(150.)).flex_shrink_0().min_w_0().truncate().text_color(theme::muted()).child(state))
            .child(div().flex_1().min_w_0().h(px(6.)).rounded(px(3.)).bg(theme::inset())
                .child(div().h_full().rounded(px(3.)).bg(color).w(relative((bytes as f32 / max).clamp(0.01, 1.)))))
            .child(div().w(px(72.)).flex_shrink_0().flex().justify_end().child(text));
        let rows: Vec<AnyElement> = top.iter().map(|w| {
            let path = w.path.clone();
            let st = state_of(w);
            div().id(SharedString::from(format!("worktrees-disk-{}", w.path))).px(px(6.)).py(px(4.)).rounded(px(6.)).cursor_pointer()
                .hover(|el| el.bg(theme::hover())).child(line(title(w), st.label(), w.size.unwrap_or(0), Self::size_text(w), st.color()))
                .on_click(cx.listener(move |this, _, window, cx| this.open_worktree(path.clone(), window, cx))).into_any_element()
        }).chain((!rest.is_empty()).then(|| div().px(px(6.)).py(px(4.))
            .child(line(tr_shared("worktrees_disco_outras", &[("n", &rest.len().to_string())]), String::new(), sum_size(rest.iter().copied()),
                sum_label(rest.iter().copied()), theme::border_strong())).into_any_element())).collect();
        let states = [WtState::Merged, WtState::Dirty, WtState::Session, WtState::Active, WtState::Detached];
        Some(card_plain().gap(px(10.))
            .child(div().flex().items_center().gap(px(10.)).flex_wrap()
                .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(tr_shared("worktrees_disco_titulo", &[])))
                .child(div().text_size(px(12.5)).text_color(theme::muted())
                    .child(tr_shared("worktrees_disco_resumo", &[("total", &total), ("n", &sized.len().to_string())])))
                .child(div().flex_1())
                .when(pending && !slow, |el| el.child(div().text_size(px(12.5)).text_color(theme::muted()).child(tr_shared("worktrees_disco_calculando", &[]))))
                .when(slow, |el| el.child(div().text_size(px(12.5)).text_color(theme::warning_text()).child(tr_shared("worktrees_disco_demorando", &[])))
                    .child(Button::new("worktrees-sizes-retry").outline().xsmall().label(tr_shared("busca_tentar_de_novo", &[]))
                        .on_click(cx.listener(|this, _, _, cx| { this.worktrees.size_tries = 0; this.load_worktrees(false, cx); }))))
                .when(freed > 0, |el| el.child(div().text_size(px(12.5)).text_color(theme::success_text())
                    .child(tr_shared("worktrees_disco_libera", &[("tamanho", &freed_text)])))))
            .child(stack_bar(sized.iter().map(|w| (w.size.unwrap_or(0) as f64, state_of(w).color())).collect()).h(px(14.)))
            .child(div().flex().flex_col().children(rows))
            .child(div().flex().flex_wrap().items_center().gap(px(12.)).text_size(px(12.)).text_color(theme::muted())
                .children(states.into_iter().map(|s| div().flex().items_center().gap(px(5.)).child(swatch(s.color())).child(s.label())))
                .child(div().text_color(theme::faint()).child(format!("· {}", tr_shared("worktrees_disco_dica", &[]))))))
    }

    fn render_wt_filters(&self, all: &[&WorktreeStatus], now: f64, cx: &mut Context<Self>) -> Div {
        div().flex().flex_wrap().gap(px(6.)).children(Filter::ALL.into_iter().map(|f| {
            let n = all.iter().filter(|w| f.matches(w, now)).count();
            Button::new(SharedString::from(format!("worktrees-filter-{f:?}"))).outline().small().selected(self.worktrees.filter == f)
                .label(format!("{} {n}", f.label()))
                .on_click(cx.listener(move |this, _, _, cx| { this.worktrees.filter = f; cx.notify(); }))
        }))
    }

    fn render_worktree_repo(&self, repo: &WorktreeRepo, visible: Vec<&WorktreeStatus>, narrowed: bool, now: f64, cx: &mut Context<Self>) -> Div {
        let (deletable, _) = merged_batch(&repo.worktrees);
        let deleting = self.worktrees.deleting;
        let main = repo.worktrees.iter().find_map(|w| w.main_branch.clone());
        let path_line = match &main {
            Some(b) => format!("{} · {}", repo.repo, tr_shared("worktree_repo_principal", &[("branch", b)])),
            None => repo.repo.clone(),
        };
        let total = sum_label(&repo.worktrees);
        let (repo_new, repo_batch) = (repo.repo.clone(), repo.clone());
        let head = div().flex().items_center().gap(px(10.))
            .child(chrome::small_icon(IconName::Folder, 16., theme::muted()))
            .child(div().flex_1().min_w_0().flex().flex_col()
                .child(div().truncate().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(base_name(&repo.repo)))
                .child(div().truncate().text_size(px(12.)).text_color(theme::muted()).child(path_line)))
            .child(div().flex_shrink_0().text_size(px(12.5)).text_color(theme::muted())
                .child(tr_shared("worktree_repo_resumo", &[("n", &repo.worktrees.len().to_string()), ("tamanho", &total)])))
            .child(Button::new(SharedString::from(format!("worktrees-new-{}", repo.repo))).outline().small().icon(IconName::Plus)
                .label(tr_shared("worktree_nova", &[]))
                .on_click(cx.listener(move |this, _, window, cx| this.open_create(repo_new.clone(), window, cx))))
            .when(!deletable.is_empty(), |el| el.child(Button::new(SharedString::from(format!("worktrees-clean-{}", repo.repo))).outline().small()
                .label(tr_shared("worktree_limpar_mescladas", &[("n", &deletable.len().to_string()), ("tamanho", &sum_label(&deletable))]))
                .loading(deleting).disabled(deleting)
                .on_click(cx.listener(move |this, _, window, cx| this.open_batch(&repo_batch, window, cx)))));
        let (agents, mains): (Vec<&WorktreeStatus>, Vec<&WorktreeStatus>) = visible.into_iter().partition(|w| is_agent(w));
        let cols = div().flex().items_center().gap(px(12.)).px(px(10.)).pb(px(6.)).border_b_1().border_color(theme::border())
            .text_size(px(11.5)).font_weight(FontWeight::MEDIUM).text_color(theme::faint())
            .child(div().flex_basis(px(0.)).flex_grow(1.).min_w_0().child(tr_shared("worktree_col_worktree", &[])))
            .child(div().flex_basis(px(0.)).flex_grow(1.).min_w_0().child(tr_shared("worktree_col_branch", &[])))
            .child(div().w(px(COL_COMMITS)).flex_shrink_0().child(tr_shared("worktree_col_commits", &[])))
            .child(div().w(px(COL_DIRTY)).flex_shrink_0().child(tr_shared("worktree_col_nao_commitado", &[])))
            .child(div().w(px(COL_ACTIVITY)).flex_shrink_0().child(tr_shared("worktree_col_atividade", &[])))
            .child(div().w(px(COL_SIZE)).flex_shrink_0().flex().justify_end().child(tr_shared("worktree_col_espaco", &[])));
        let rows: Vec<Stateful<Div>> = mains.iter().map(|w| self.render_worktree_row(w, now, cx)).collect();
        let agent_group = (!agents.is_empty()).then(|| {
            let open = narrowed || self.worktrees.agents_open.contains(&repo.repo);
            let newest = agents.iter().filter_map(|w| activity_at(w)).max();
            let key = repo.repo.clone();
            let toggle = div().id(SharedString::from(format!("worktrees-agents-{}", repo.repo))).mt(px(4.)).px(px(10.)).py(px(6.)).rounded(px(8.))
                .cursor_pointer().hover(|el| el.bg(theme::hover())).flex().items_center().gap(px(8.)).text_size(px(13.))
                .child(chrome::small_icon(if open { IconName::ChevronDown } else { IconName::ChevronRight }, 13., theme::muted()))
                .child(div().font_weight(FontWeight::MEDIUM).child(tr_shared("worktree_subagentes", &[])))
                .child(chip(agents.len().to_string(), theme::muted()))
                .children(newest.map(|at| div().min_w_0().truncate().text_size(px(12.)).text_color(theme::muted())
                    .child(format!("· {}", tr_shared("worktree_subagentes_dica", &[("quando", &ago_at(at, now))])))))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.worktrees.agents_open.remove(&key) { this.worktrees.agents_open.insert(key.clone()); }
                    cx.notify();
                }));
            let list: Vec<Stateful<Div>> = if open { agents.iter().map(|w| self.render_worktree_row(w, now, cx)).collect() } else { Vec::new() };
            div().flex().flex_col().child(toggle).children(list)
        });
        card_plain().gap(px(12.)).child(head)
            .child(div().flex().flex_col().gap(px(2.)).child(cols).children(rows).children(agent_group))
    }

    fn session_live(&self, name: &str) -> (String, Hsla) {
        match self.sessions.iter().find(|s| s.name == name).map(SessionInfo::display_state) {
            Some("working") => (tr_shared("worktree_sessao_trabalhando", &[]), theme::success()),
            Some("awaiting_input") => (tr_shared("worktree_sessao_esperando", &[]), theme::warning()),
            _ => (tr_shared("worktree_sessao_parada", &[]), theme::faint()),
        }
    }

    /// A sessão aberta agora, se for desta máquina.
    fn current_session(&self) -> Option<&str> { self.selected.as_ref().filter(|_| self.open_api.is_none()).map(|s| s.name.as_str()) }

    fn size_text(w: &WorktreeStatus) -> String {
        match (w.exists, w.size) {
            (false, _) => "—".into(),
            _ if w.size_error => tr_shared("worktree_tamanho_falhou", &[]),
            (_, Some(b)) if w.size_partial => tr_shared("worktree_tamanho_parcial", &[("tamanho", &fmt_size(b))]),
            (_, Some(b)) => fmt_size(b),
            _ if w.size_pending => tr_shared("worktrees_disco_calculando", &[]),
            _ => "—".into(),
        }
    }

    fn render_worktree_row(&self, w: &WorktreeStatus, now: f64, cx: &mut Context<Self>) -> Stateful<Div> {
        let path = w.path.clone();
        let open = self.worktrees.open.as_deref() == Some(w.path.as_str());
        let st = state_of(w);
        let you = self.current_session().is_some_and(|me| w.sessions.iter().any(|s| s == me));
        let sessions: Vec<Button> = w.sessions.iter().map(|name| {
            let (label, color) = self.session_live(name);
            let target = name.clone();
            Button::new(SharedString::from(format!("worktree-session-{}-{name}", w.path))).ghost().xsmall()
                .accessibility_label(tr_shared("worktree_ir_sessao_nome", &[("nome", name)]))
                .child(div().flex().items_center().gap(px(5.)).text_size(px(12.))
                    .child(dot(color)).child(name.clone()).child(div().text_color(theme::muted()).child(label))
                    .child(chrome::small_icon(IconName::ArrowRight, 11., theme::faint())))
                .on_click(cx.listener(move |this, _, window, cx| { cx.stop_propagation(); this.go_to_session(target.clone(), window, cx); }))
        }).collect();
        let name_cell = div().flex_basis(px(0.)).flex_grow(1.).min_w_0().flex().flex_col().gap(px(4.))
            .child(div().flex().items_center().gap(px(6.))
                .child(div().min_w_0().truncate().text_size(px(13.5)).font_weight(FontWeight::MEDIUM).child(title(w)))
                .when(you, |el| el.child(chip(tr_shared("worktree_esta_sessao", &[]), theme::accent()))))
            .child(div().flex().flex_wrap().gap(px(4.)).child(chip(st.label(), st.color()))
                .children(stale_days(w, now).map(|d| chip(tr_shared("worktree_parada_dias", &[("n", &d.to_string())]), theme::warning_text()))))
            .when(!sessions.is_empty(), |el| el.child(div().flex().flex_wrap().gap(px(2.)).children(sessions)))
            .when(w.degraded, |el| el.child(div().text_size(px(11.5)).text_color(theme::warning_text()).whitespace_normal()
                .child(tr_shared("worktree_leitura_incompleta", &[]))));
        let branch_cell = div().flex_basis(px(0.)).flex_grow(1.).min_w_0().flex().flex_col().gap(px(2.)).text_size(px(12.5))
            .child(div().flex().min_w_0()
                .child(div().min_w_0().truncate().font_family(theme::MONO)
                    .child(w.branch.clone().unwrap_or_else(|| tr_shared("worktree_sem_branch_rotulo", &[]))))
                .children(w.base.as_ref().map(|b| div().flex_shrink_0().text_color(theme::faint()).child(format!(" ← {b}")))))
            .children(w.last_commit.as_ref().map(|c| div().min_w_0().truncate().text_color(theme::muted()).child(c.subject.clone())));
        let bar = |n: i64, color: Hsla| div().h(px(3.)).w(px(bar_width(n))).rounded(px(2.)).bg(color);
        let commits = div().w(px(COL_COMMITS)).flex_shrink_0().flex().flex_col().gap(px(3.)).text_size(px(12.)).font_family(theme::MONO)
            .child(div().flex().items_center().gap(px(5.)).child(format!("↑{}", w.ahead)).child(bar(w.ahead, theme::accent())))
            .child(div().flex().items_center().gap(px(5.)).text_color(theme::muted()).child(format!("↓{}", w.behind)).child(bar(w.behind, theme::warning())));
        let dirty = div().w(px(COL_DIRTY)).flex_shrink_0().text_size(px(12.5)).map(|el| if w.dirty > 0 {
            el.text_color(theme::warning_text()).child(tr_shared("worktree_n_nao_commitados", &[("n", &w.dirty.to_string())]))
        } else { el.text_color(theme::faint()).child(tr_shared("worktree_limpa", &[])) });
        let activity = div().w(px(COL_ACTIVITY)).flex_shrink_0().flex().flex_col().gap(px(2.)).text_size(px(12.5))
            .children(activity_at(w).map(|at| div().flex().items_center().gap(px(5.))
                .child(dot(age_color(age_days(w, now).unwrap_or(0)))).child(ago_at(at, now))))
            .children(w.created_at.map(|at| div().text_size(px(11.5)).text_color(theme::faint())
                .child(tr_shared("worktree_criada", &[("quando", &short_date(at))]))));
        let size = div().w(px(COL_SIZE)).flex_shrink_0().flex().justify_end().text_size(px(12.5))
            .text_color(if w.size.unwrap_or(0) >= 1 << 30 { theme::text() } else { theme::muted() }).child(Self::size_text(w));
        div().id(SharedString::from(format!("worktree-row-{}", w.path))).px(px(10.)).py(px(8.)).rounded(px(8.)).flex().items_start().gap(px(12.))
            .cursor_pointer().border_l_2().border_color(if open { theme::accent() } else { transparent_black() })
            .map(|el| if open { el.bg(theme::selected_row()) } else { el.hover(|el| el.bg(theme::hover())) })
            .child(name_cell).child(branch_cell).child(commits).child(dirty).child(activity).child(size)
            .on_click(cx.listener(move |this, _, window, cx| this.open_worktree(path.clone(), window, cx)))
    }

    /// O painel da worktree aberta: veredito, sessões dentro, branch contra a base, fatos, commits e o que não foi commitado.
    fn render_worktree_panel(&self, path: &str, cx: &mut Context<Self>) -> Stateful<Div> {
        let close = Button::new("worktree-panel-close").ghost().small().icon(IconName::Close).accessibility_label(tr("close"))
            .on_click(cx.listener(|this, _, _, cx| this.close_worktree_panel(cx)));
        let panel = div().id("worktree-panel").w(px(360.)).flex_shrink_0().h_full().overflow_y_scroll().border_l_1().border_color(theme::border())
            .p(px(16.)).flex().flex_col().gap(px(14.)).text_size(px(13.)).whitespace_normal();
        let detail = self.worktrees.detail.value.as_ref().filter(|_| !self.worktrees.detail.loading);
        // Enquanto a leitura fresca não chega, o painel mostra a da lista.
        let st = match (detail, self.worktrees.cache.get(path)) {
            (Some(Ok(st)), _) => st.clone(),
            (Some(Err(error)), None) => return panel.child(div().flex().justify_end().child(close))
                .child(note_box(tr_shared("worktrees_erro", &[("motivo", error.as_str())]), theme::warning())),
            (_, Some(st)) => st.clone(),
            (None, None) => return panel.child(div().flex().justify_end().child(close)).child(loading_state()),
        };
        let state = state_of(&st);
        let now = super::side::now_seconds();
        let copy = st.path.clone();
        let head = div().flex().flex_col().gap(px(8.))
            .child(div().flex().items_start().gap(px(8.))
                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(4.))
                    .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(title(&st)))
                    .child(div().flex().child(chip(state.label(), state.color()))))
                .child(close))
            .child(div().flex().items_center().gap(px(4.))
                .child(div().flex_1().min_w_0().font_family(theme::MONO).text_size(px(11.5)).text_color(theme::muted()).child(st.path.clone()))
                .child(Button::new("worktree-copy-path").ghost().xsmall().icon(IconName::Copy).tooltip(tr_shared("worktree_copiar_caminho", &[]))
                    .accessibility_label(tr_shared("worktree_copiar_caminho", &[]))
                    .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone())))))
            .child(div().p(px(10.)).rounded(px(8.)).bg(theme::inset()).text_color(state.color()).child(state.verdict(&st)))
            .when(st.degraded, |el| el.child(note_box(tr_shared("worktree_leitura_incompleta", &[]), theme::warning())));
        let me = self.current_session();
        let sessions = (!st.sessions.is_empty()).then(|| div().flex().flex_col().gap(px(8.))
            .child(section(tr_shared("worktree_sessoes_aqui", &[])))
            .children(st.sessions.iter().map(|name| {
                let info = self.sessions.iter().find(|s| &s.name == name);
                let (label, color) = self.session_live(name);
                let here = me == Some(name.as_str());
                let harness = info.map(|s| match s.engine.as_deref().filter(|e| !e.is_empty()) {
                    Some(engine) => format!("{} · {engine}", super::agent_name(&s.provider)),
                    None => super::agent_name(&s.provider).to_owned(),
                });
                let doing = info.and_then(|s| s.question.clone().or_else(|| s.label.clone())
                    .or_else(|| s.last_reply.as_deref().and_then(|r| r.lines().find(|l| !l.trim().is_empty())).map(str::to_owned)));
                let when = info.and_then(|s| s.last_activity).map(|at| super::side::ago(now - at));
                let target = name.clone();
                div().p(px(10.)).rounded(px(8.)).border_1().border_color(theme::border()).bg(theme::inset()).flex().flex_col().gap(px(4.))
                    .child(div().flex().items_center().gap(px(6.)).child(dot(color))
                        .child(div().min_w_0().truncate().font_weight(FontWeight::MEDIUM).child(name.clone()))
                        .when(here, |el| el.child(chip(tr_shared("worktree_esta_sessao", &[]), theme::accent())))
                        .child(div().flex_1()).child(div().text_size(px(12.)).text_color(color).child(label)))
                    .children(harness.map(muted))
                    .when(doing.is_some() || when.is_some(), |el| el.child(div().text_size(px(12.)).text_color(theme::muted()).truncate()
                        .child([doing, when].into_iter().flatten().collect::<Vec<_>>().join(" · "))))
                    .when(!here, |el| el.child(div().flex().child(Button::new(SharedString::from(format!("worktree-go-{name}"))).outline().xsmall()
                        .label(tr_shared("worktree_ir_sessao", &[])).icon(IconName::ArrowRight)
                        .on_click(cx.listener(move |this, _, window, cx| this.go_to_session(target.clone(), window, cx))))))
            })));
        let branch = st.branch.clone().unwrap_or_else(|| tr_shared("worktree_sem_branch_rotulo", &[]));
        let diagram = div().flex().flex_col().gap(px(8.))
            .child(section(tr_shared("worktree_detalhe_branch", &[])))
            .child(div().pl(px(10.)).border_l_2().border_color(theme::border_strong()).flex().flex_col().gap(px(6.)).text_size(px(12.5))
                .children(st.base.as_ref().map(|b| div().flex().flex_wrap().gap(px(6.))
                    .child(div().font_family(theme::MONO).child(b.clone()))
                    .child(div().text_color(theme::muted()).child(tr_shared("worktree_detalhe_base", &[("n", &st.behind.to_string())])))))
                .child(div().flex().flex_wrap().gap(px(6.))
                    .child(div().font_family(theme::MONO).text_color(theme::accent_text()).child(branch))
                    .child(div().text_color(theme::muted()).child(tr_shared("worktree_detalhe_so_dela", &[("n", &st.ahead.to_string())])))));
        let fact = |label: String, value: String| div().flex().gap(px(8.)).text_size(px(12.5))
            .child(div().w(px(120.)).flex_shrink_0().text_color(theme::muted()).child(label)).child(div().flex_1().min_w_0().child(value));
        let facts = div().flex().flex_col().gap(px(5.))
            .child(fact(tr_shared("worktree_detalhe_criada", &[]), st.created_at.map(|t| super::share::when_label(t as f64)).unwrap_or_else(|| "—".into())))
            .child(fact(tr_shared("worktree_detalhe_ultimo_commit", &[]),
                st.last_commit.as_ref().map(|c| super::share::when_label(c.at as f64)).unwrap_or_else(|| "—".into())))
            .child(fact(tr_shared("worktree_detalhe_conversas", &[]), st.closed.to_string()))
            .child(fact(tr_shared("worktree_detalhe_espaco", &[]), Self::size_text(&st)))
            .children(st.size_biggest.as_ref().map(|b| fact(tr_shared("worktree_detalhe_maior", &[]), format!("{} · {}", b.name, fmt_size(b.bytes)))));
        let commit_rows: Vec<Div> = st.commits.iter().map(|c| div().flex().items_center().gap(px(8.)).text_size(px(12.5))
            .child(div().flex_shrink_0().font_family(theme::MONO).text_color(theme::muted()).child(c.sha.clone()))
            .child(div().flex_1().min_w_0().truncate().child(c.subject.clone()))
            .child(div().flex_shrink_0().text_size(px(11.5)).text_color(theme::faint()).child(short_date(c.at)))).collect();
        let commits = div().flex().flex_col().gap(px(5.)).child(section(tr_shared("worktree_detalhe_commits", &[])))
            .map(|el| if commit_rows.is_empty() { el.child(muted(tr_shared("worktree_detalhe_sem_commits", &[]))) } else { el.children(commit_rows) });
        let files = (st.dirty > 0).then(|| div().flex().flex_col().gap(px(4.))
            .child(section(tr_shared("worktree_detalhe_nao_commitado", &[("n", &st.dirty.to_string())])))
            .children(st.dirty_files.iter().map(|f| div().flex().gap(px(8.)).font_family(theme::MONO).text_size(px(11.5))
                .child(div().w(px(18.)).flex_shrink_0().text_color(theme::warning_text()).child(f.code.clone()))
                .child(div().flex_1().min_w_0().truncate().child(f.path.clone())))));
        let (here, target) = (st.clone(), st.path.clone());
        let blocked = !st.sessions.is_empty();
        let actions = div().flex().flex_wrap().gap(px(8.))
            .child(Button::new("worktree-new-session").outline().small().label(tr_shared(if st.sessions.is_empty() { "worktree_abrir_sessao_aqui" } else { "worktree_nova_sessao_aqui" }, &[])).disabled(!st.exists)
                .on_click(cx.listener(move |this, _, window, cx| this.new_session_here(&here, window, cx))))
            .child(div().flex_1())
            .child(Button::new("worktree-delete").danger().small().label(tr_shared("worktree_apagar_reticencias", &[]))
                .disabled(blocked || self.worktrees.deleting)
                .on_click(cx.listener(move |this, _, window, cx| this.open_delete(target.clone(), window, cx))));
        panel.child(head).children(sessions).child(diagram).child(facts).child(commits).children(files).child(actions)
            .when(blocked, |el| el.child(div().text_size(px(12.)).text_color(theme::warning_text())
                .child(tr_shared("worktree_bloqueada", &[("nomes", &st.sessions.join(", "))]))))
    }

    /// A confirmação de apagar, no texto do web: o que se perde, o que fica guardado e a branch.
    fn render_delete_dialog(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let Some(WtDialog::Delete(path)) = self.worktrees.dialog.clone() else { return div() };
        let fresh = self.worktrees.detail.ok().filter(|s| s.path == path);
        let Some(st) = fresh.or_else(|| self.worktrees.cache.get(&path)).cloned() else { return div().child(loading_state()) };
        let deleting = self.worktrees.deleting;
        let branch = st.branch.clone().unwrap_or_default();
        let base = st.base.clone().unwrap_or_default();
        let ok = |text: String| div().flex().gap(px(6.)).child(div().text_color(theme::success_text()).child("✓")).child(div().flex_1().child(text));
        let mut kept: Vec<Div> = Vec::new();
        if !branch.is_empty() {
            kept.push(ok(if st.merged { tr_shared("worktree_apagar_branch_juntada", &[("branch", &branch), ("base", &base)]) }
                else if st.ahead > 0 { tr_shared("worktree_fica_commits", &[("n", &st.ahead.to_string()), ("branch", &branch)]) }
                else { tr_shared("worktree_apagar_branch_fica", &[("branch", &branch)]) }));
        }
        kept.push(ok(if st.closed > 0 { tr_shared("worktree_fica_conversas", &[("n", &st.closed.to_string())]) }
            else { tr_shared("worktree_apagar_conversas", &[("branch", st.main_branch.as_deref().unwrap_or(&base))]) }));
        let lost = !st.dirty_files.is_empty() || st.dirty > 0 || !st.ignored.is_empty();
        let target = st.clone();
        let sub = [Some(base_name(&st.path)), st.branch.clone()].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        div().flex().flex_col().gap(px(14.)).text_size(px(13.)).whitespace_normal()
            .child(div().flex().flex_col().gap(px(4.))
                .child(div().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).child(tr_shared("worktree_apagar_titulo", &[("nome", &title(&st))])))
                .child(div().font_family(theme::MONO).text_size(px(12.)).text_color(theme::muted()).child(sub))
                .when(st.size.is_some_and(|b| b > 0) && !st.size_error, |el| el.child(div().text_color(theme::success_text())
                    .child(tr_shared("worktree_libera", &[("tamanho", &Self::size_text(&st))])))))
            .when(!st.sessions.is_empty(), |el| el.child(note_box(tr_shared("worktree_bloqueada", &[("nomes", &st.sessions.join(", "))]), theme::warning())))
            .when(lost, |el| el.child(div().flex().flex_col().gap(px(6.))
                .child(section(tr_shared("worktree_apagar_perde", &[])).text_color(theme::warning_text()))
                .child(div().p(px(10.)).rounded(px(8.)).border_1().border_color(theme::warning().alpha(0.35)).bg(theme::warning().alpha(0.06))
                    .flex().flex_col().gap(px(4.))
                    .when(st.dirty > 0, |el| el.child(div().font_weight(FontWeight::MEDIUM)
                        .child(tr_shared("worktree_n_nao_commitados", &[("n", &st.dirty.to_string())]))))
                    .children(st.dirty_files.iter().map(|f| div().flex().gap(px(8.)).font_family(theme::MONO).text_size(px(12.))
                        .child(div().w(px(18.)).flex_shrink_0().text_color(theme::warning_text()).child(f.code.clone()))
                        .child(div().flex_1().min_w_0().truncate().child(f.path.clone()))))
                    .when(!st.ignored.is_empty(), |el| el.child(div().text_color(theme::muted())
                        .child(tr_shared("worktree_ignorados", &[("nomes", &st.ignored.join(", "))])))))))
            .child(div().flex().flex_col().gap(px(6.)).child(section(tr_shared("worktree_fica_guardado", &[]))).children(kept))
            .when(!st.merged && !branch.is_empty() && st.ahead > 0, |el| el.child(Checkbox::new("worktree-delete-branch")
                .label(tr_shared("worktree_apagar_branch_tambem", &[("n", &st.ahead.to_string())]))
                .checked(self.worktrees.delete_branch).disabled(deleting)
                .on_change(cx.listener(|this, checked: &bool, _, cx| { this.worktrees.delete_branch = *checked; cx.notify(); }))))
            .when_some(self.worktrees.delete_error.clone(), |el, error| el.child(div().id("worktree-delete-error").role(Role::Alert)
                .text_color(theme::danger()).child(error)))
            .child(div().flex().justify_end().gap_2()
                .child(Button::new("worktree-delete-cancel").outline().label(tr_shared("worktree_nova_cancelar", &[])).disabled(deleting)
                    .on_click(cx.listener(|this, _, window, cx| this.cancel_wt_dialog(window, cx))))
                .child(Button::new("worktree-delete-confirm").danger()
                    .label(if st.dirty > 0 { tr_shared("worktree_apagar_perder", &[("n", &st.dirty.to_string())]) } else { tr_shared("worktree_apagar", &[]) })
                    .loading(deleting).disabled(deleting || !st.sessions.is_empty())
                    .on_click(cx.listener(move |this, _, window, cx| this.delete_worktree(target.clone(), window, cx)))))
    }

    /// O lote de mescladas: cada uma com o que perde, e as que ficam de fora com o motivo.
    fn render_batch_dialog(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let Some(batch) = self.worktrees.batch.as_ref() else { return div() };
        let deleting = self.worktrees.deleting;
        let item = |w: &WorktreeStatus| {
            let body = div().flex().flex_col().gap(px(2.)).child(div().font_weight(FontWeight::MEDIUM).child(title(w)));
            if loses(w) {
                body.child(div().text_color(theme::warning_text()).child(tr_shared("worktree_apagar_perde", &[])))
                    .children(lost_files(w).into_iter().map(|f| div().pl(px(10.)).font_family(theme::MONO).text_size(px(12.)).text_color(theme::muted()).child(f)))
            } else { body.child(muted(tr_shared("worktree_lote_nada_perde", &[]))) }
        };
        div().flex().flex_col().gap(px(12.)).text_size(px(13.)).whitespace_normal()
            .child(div().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD)
                .child(tr_shared("worktree_lote_titulo", &[("n", &batch.deletable.len().to_string())])))
            .child(div().id("worktree-batch-list").max_h(px(360.)).overflow_y_scroll().flex().flex_col().gap(px(10.))
                .children(batch.deletable.iter().map(item))
                .when(!batch.blocked.is_empty(), |el| el.child(div().text_color(theme::warning_text()).child(tr_shared("worktree_lote_ficam", &[])))
                    .children(batch.blocked.iter().map(|w| div().flex().flex_col().gap(px(2.))
                        .child(div().font_weight(FontWeight::MEDIUM).child(title(w)))
                        .child(muted(if w.sessions.is_empty() { tr_shared("worktree_lote_leitura_falhou", &[]) }
                            else { tr_shared("worktree_sessao_aberta", &[("nomes", &w.sessions.join(", "))]) }))))))
            .when_some(self.worktrees.batch_dialog_error.clone(), |el, error| el.child(div().id("worktree-batch-error").role(Role::Alert)
                .text_color(theme::danger()).child(error)))
            .child(div().flex().justify_end().gap_2()
                .child(Button::new("worktree-batch-cancel").outline().label(tr_shared("worktree_nova_cancelar", &[])).disabled(deleting)
                    .on_click(cx.listener(|this, _, window, cx| this.cancel_wt_dialog(window, cx))))
                .child(Button::new("worktree-batch-confirm").danger().label(tr_shared("worktree_lote_confirmar", &[])).loading(deleting).disabled(deleting)
                    .on_click(cx.listener(|this, _, window, cx| this.delete_batch(window, cx)))))
    }

    /// "Nova worktree": começar algo novo (branch e pasta saem da descrição) ou continuar uma branch que não está aberta.
    fn render_create_dialog(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let Some(c) = self.worktrees.create.as_ref() else { return div() };
        // Branch aberta em alguma worktree deste repositório: leva até ela em vez de criar outra.
        let taken: HashMap<String, (String, String)> = self.worktrees.repos.ok().into_iter().flatten().filter(|r| r.repo == c.repo)
            .flat_map(|r| &r.worktrees).filter_map(|w| w.branch.clone().map(|b| (b, (w.path.clone(), title(w))))).collect();
        let plan = c.plan(cx);
        let busy = c.busy;
        let repo_name = base_name(&c.repo);
        let folder_of = |name: &str| format!("{}-{name}", c.repo.trim_end_matches('/'));
        let modes = div().flex().gap(px(4.)).p(px(3.)).rounded(px(8.)).bg(theme::inset())
            .children([(false, "worktree_nova_comecar"), (true, "worktree_nova_continuar")].into_iter().map(|(existing, key)| {
                Button::new(SharedString::from(format!("worktree-create-mode-{existing}"))).ghost().small().flex_1()
                    .selected(c.existing_mode == existing).label(tr_shared(key, &[])).disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| { if let Some(c) = this.worktrees.create.as_mut() { c.existing_mode = existing; c.error = None; } cx.notify(); }))
            }));
        let value_row = |id: &'static str, label: String, value: String, editing: bool, input: &Entity<InputState>, edit: fn(&mut NewWorktree) -> &mut bool| {
            div().flex().items_center().gap(px(8.)).text_size(px(12.5))
                .child(div().w(px(70.)).flex_shrink_0().text_color(theme::muted()).child(label))
                .child(if editing { div().flex_1().child(Input::new(input).small()).into_any_element() }
                    else { div().flex_1().min_w_0().truncate().font_family(theme::MONO).child(value.clone()).into_any_element() })
                .when(!editing, |el| el.child(Button::new(id).ghost().xsmall().label(tr_shared("worktree_nova_editar", &[])).disabled(busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let Some(c) = this.worktrees.create.as_mut() else { return };
                        *edit(c) = true;
                        let field = if id.ends_with("branch") { c.branch.clone() } else { c.folder.clone() };
                        field.update(cx, |input, cx| { input.set_value(value.clone(), window, cx); input.focus(window, cx); });
                        cx.notify();
                    }))))
        };
        let body = if !c.existing_mode {
            let s = slug(&c.task.read(cx).value());
            let shown_branch = plan.as_ref().map(|p| p.branch.clone()).unwrap_or_else(|| if s.is_empty() { "feat/…".into() } else { format!("feat/{s}") });
            let shown_name = plan.as_ref().map(|p| p.name.clone()).unwrap_or(s);
            let bases: Vec<String> = match c.branches.ok() {
                Some(b) => b.current.iter().cloned().chain(["main", "master", "develop"].into_iter()
                    .filter(|n| b.current.as_deref() != Some(*n) && b.branches.iter().any(|x| x == n)).map(str::to_owned)).collect(),
                None => Vec::new(),
            };
            let current = c.branches.ok().and_then(|b| b.current.clone());
            div().flex().flex_col().gap(px(12.))
                .child(div().flex().flex_col().gap(px(5.)).child(div().text_size(px(12.5)).text_color(theme::muted()).child(tr_shared("worktree_nova_tarefa", &[])))
                    .child(Input::new(&c.task)))
                .child(div().flex().flex_col().gap(px(6.)).p(px(10.)).rounded(px(8.)).bg(theme::inset())
                    .child(value_row("worktree-create-edit-branch", tr_shared("worktree_detalhe_branch", &[]), shown_branch, c.edit_branch, &c.branch, |c| &mut c.edit_branch))
                    .child(value_row("worktree-create-edit-folder", tr_shared("worktree_nova_pasta", &[]), shown_name.clone(), c.edit_folder, &c.folder, |c| &mut c.edit_folder))
                    .when(!shown_name.is_empty(), |el| el.child(div().pl(px(78.)).min_w_0().truncate().font_family(theme::MONO).text_size(px(11.5))
                        .text_color(theme::faint()).child(folder_of(&shown_name)))))
                .child(div().flex().flex_col().gap(px(6.))
                    .child(div().text_size(px(12.5)).text_color(theme::muted()).child(tr_shared("worktree_base", &[])))
                    .map(|el| match &c.branches.value {
                        _ if c.branches.loading => el.child(popup::skeleton("worktree-create-bases", 2)),
                        Some(Err(error)) => el.child(div().flex().items_center().gap_2().child(div().text_color(theme::danger()).child(error.clone()))
                            .child(Button::new("worktree-create-bases-retry").outline().xsmall().label(tr("retry"))
                                .on_click(cx.listener(|this, _, _, cx| this.load_create_branches(cx))))),
                        _ if bases.is_empty() => el.child(muted(tr("ctl_no_results"))),
                        _ => el.children(bases.into_iter().map(|b| {
                            let note = if current.as_ref() == Some(&b) { tr_shared("worktree_nova_base_principal", &[]) }
                                else if matches!(b.as_str(), "main" | "master") { tr_shared("worktree_nova_base_publicada", &[]) } else { String::new() };
                            let pick = b.clone();
                            Radio::new(SharedString::from(format!("worktree-create-base-{b}"))).checked(c.base == b).disabled(busy)
                                .label(if note.is_empty() { b.clone() } else { format!("{b} · {note}") })
                                .on_change(cx.listener(move |this, _, _, cx| { if let Some(c) = this.worktrees.create.as_mut() { c.base = pick.clone(); } cx.notify(); }))
                        })),
                    })
                    .child(Checkbox::new("worktree-create-fetch").label(tr_shared("worktree_nova_buscar_remoto", &[])).checked(c.fetch).disabled(busy)
                        .on_change(cx.listener(|this, checked: &bool, _, cx| { if let Some(c) = this.worktrees.create.as_mut() { c.fetch = *checked; } cx.notify(); }))))
        } else {
            let rows: Vec<AnyElement> = match (&c.branches.value, c.branches.ok()) {
                _ if c.branches.loading => vec![popup::skeleton("worktree-create-branches", 4).into_any_element()],
                (Some(Err(error)), _) => vec![div().flex().items_center().gap_2().child(div().text_color(theme::danger()).child(error.clone()))
                    .child(Button::new("worktree-create-branches-retry").outline().xsmall().label(tr("retry"))
                        .on_click(cx.listener(|this, _, _, cx| this.load_create_branches(cx)))).into_any_element()],
                (_, Some(b)) => {
                    let list: Vec<(String, bool)> = b.branches.iter().filter(|x| b.current.as_ref() != Some(*x)).map(|x| (x.clone(), false))
                        .chain(b.remotes.iter().map(|x| (x.clone(), true))).collect();
                    if list.is_empty() { vec![muted(tr("ctl_no_results")).into_any_element()] } else {
                        list.into_iter().map(|(name, remote)| {
                            let row = div().flex().items_center().gap(px(8.)).py(px(3.)).text_size(px(12.5));
                            match taken.get(&name) {
                                Some((path, wt)) => {
                                    let path = path.clone();
                                    row.child(div().size(px(16.)).flex_shrink_0())
                                        .child(div().flex_1().min_w_0().truncate().font_family(theme::MONO).text_color(theme::muted()).child(name.clone()))
                                        .child(Button::new(SharedString::from(format!("worktree-create-taken-{name}"))).ghost().xsmall()
                                            .label(format!("{} →", tr_shared("worktree_nova_ja_aberta", &[("nome", wt)]))).disabled(busy)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.finish_wt_dialog(&WtDialog::Create, window, cx);
                                                this.worktrees.create = None;
                                                this.open_worktree(path.clone(), window, cx);
                                            })))
                                        .into_any_element()
                                }
                                None => {
                                    let pick = name.clone();
                                    row.child(Radio::new(SharedString::from(format!("worktree-create-pick-{name}"))).checked(c.pick == name).disabled(busy)
                                        .label(if remote { format!("{name} · {}", tr_shared("worktree_nova_so_remoto", &[])) } else { name.clone() })
                                        .on_change(cx.listener(move |this, _, _, cx| { if let Some(c) = this.worktrees.create.as_mut() { c.pick = pick.clone(); } cx.notify(); })))
                                        .into_any_element()
                                }
                            }
                        }).collect()
                    }
                }
                _ => Vec::new(),
            };
            div().flex().flex_col().gap(px(6.))
                .child(div().text_size(px(12.5)).text_color(theme::muted()).child(tr_shared("worktree_nova_qual_branch", &[])))
                .child(div().id("worktree-create-branches").max_h(px(260.)).overflow_y_scroll().flex().flex_col().children(rows))
                .child(muted(tr_shared("worktree_nova_uma_por_vez", &[])))
        };
        let mut steps: Vec<String> = Vec::new();
        if let Some(p) = &plan {
            if p.new_branch {
                steps.push(if p.fetch { tr_shared("worktree_nova_passo_buscar", &[("base", &p.base)]) } else { tr_shared("worktree_nova_passo_base", &[("base", &p.base)]) });
                steps.push(tr_shared("worktree_nova_passo_criar", &[("pasta", &folder_of(&p.name)), ("branch", &p.branch)]));
            } else {
                steps.push(tr_shared("worktree_nova_passo_continuar", &[("pasta", &folder_of(&p.name)), ("branch", &p.branch)]));
            }
            steps.push(tr_shared(if c.session { "worktree_nova_passo_sessao" } else { "worktree_nova_passo_sem_sessao" }, &[]));
        }
        div().flex().flex_col().gap(px(14.)).text_size(px(13.)).whitespace_normal()
            .child(div().flex().flex_col().gap(px(4.))
                .child(div().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).child(tr_shared("worktree_nova_titulo", &[("repo", &repo_name)])))
                .child(muted(tr_shared("worktree_nova_dica", &[]))))
            .child(modes)
            .child(body)
            .child(Checkbox::new("worktree-create-session").label(tr_shared("worktree_nova_abrir_sessao", &[])).checked(c.session).disabled(busy)
                .on_change(cx.listener(|this, checked: &bool, _, cx| { if let Some(c) = this.worktrees.create.as_mut() { c.session = *checked; } cx.notify(); })))
            .when(!steps.is_empty(), |el| el.child(div().flex().flex_col().gap(px(4.)).p(px(10.)).rounded(px(8.)).border_1().border_color(theme::border())
                .child(section(tr_shared("worktree_nova_vai_acontecer", &[])))
                .children(steps.into_iter().enumerate().map(|(n, text)| div().flex().gap(px(6.)).text_size(px(12.5))
                    .child(div().flex_shrink_0().text_color(theme::faint()).child(format!("{}.", n + 1))).child(div().flex_1().child(text))))))
            .when_some(c.error.clone(), |el, error| el.child(div().id("worktree-create-error").role(Role::Alert).text_color(theme::danger()).child(error)))
            .child(div().flex().justify_end().gap_2()
                .child(Button::new("worktree-create-cancel").outline().label(tr_shared("worktree_nova_cancelar", &[])).disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.cancel_wt_dialog(window, cx))))
                .child(Button::new("worktree-create-confirm").primary()
                    .label(tr_shared(if c.session { "worktree_nova_criar_sessao" } else { "worktree_nova_criar" }, &[]))
                    .loading(busy).disabled(busy || plan.is_none())
                    .on_click(cx.listener(|this, _, window, cx| this.submit_create(window, cx)))))
    }
}

const SIZE_TRIES: u32 = 12;
const COL_COMMITS: f32 = 92.;
const COL_DIRTY: f32 = 110.;
const COL_ACTIVITY: f32 = 112.;
const COL_SIZE: f32 = 76.;

#[cfg(test)]
mod tests {
    use super::{lost_files, merged_batch, ready, slug, state_of, title, is_agent, WorktreeStatus, WtState};
    use serde_json::json;

    fn st(v: serde_json::Value) -> WorktreeStatus { serde_json::from_value(v).unwrap() }

    fn base(path: &str) -> serde_json::Value {
        json!({"path": path, "repo": "/r", "exists": true, "branch": "x", "base": "main", "merged": false,
               "ahead": 0, "dirty": 0, "ignored": [], "sessions": [], "closed": 0})
    }

    #[test]
    fn lost_lists_dirty_count_and_ignored() {
        let s = st(json!({"path": "/r/x", "repo": "/r", "exists": true, "branch": "x", "base": "main", "merged": false,
                          "ahead": 2, "dirty": 3, "ignored": [".env.local"], "sessions": [], "closed": 0}));
        assert_eq!(lost_files(&s).len(), 2);
    }

    #[test]
    fn batch_keeps_dirty_merged_and_leaves_out_open_and_unreadable() {
        let mut dirty = base("/r/a"); dirty["merged"] = json!(true); dirty["dirty"] = json!(2);
        let mut busy = base("/r/b"); busy["merged"] = json!(true); busy["sessions"] = json!(["s1"]);
        let mut broken = base("/r/c"); broken["merged"] = json!(true); broken["degraded"] = json!(true);
        let (deletable, blocked) = merged_batch(&[st(dirty), st(busy), st(broken), st(base("/r/d"))]);
        assert_eq!(deletable.iter().map(|w| w.path.as_str()).collect::<Vec<_>>(), ["/r/a"]);
        assert_eq!(blocked.len(), 2);
        assert!(!ready(&deletable[0]));
    }

    #[test]
    fn state_priority_and_agent_title() {
        let mut gone = base("/r/a"); gone["exists"] = json!(false); gone["sessions"] = json!(["s"]);
        assert_eq!(state_of(&st(gone)), WtState::Gone);
        let mut detached = base("/r/b"); detached["branch"] = json!(null);
        assert_eq!(state_of(&st(detached)), WtState::Detached);
        let mut agent = base("/r/.claude/worktrees/agent-ab1ae47a0c"); agent["branch"] = json!("feat/peers");
        assert!(is_agent(&st(agent.clone())));
        assert_eq!(title(&st(agent.clone())), "feat/peers");
        agent["branch"] = json!("worktree-agent-ab1ae47a0c");
        assert_eq!(title(&st(agent)), "agent-ab1ae47a0c");
        assert!(!is_agent(&st(base("/r/agent-XYZ12345"))));
    }

    #[test]
    fn slug_folds_accents_and_dashes() {
        assert_eq!(slug("Tela de Worktrees!"), "tela-de-worktrees");
        assert_eq!(slug("  Ação  rápida__já "), "acao-rapida-ja");
        assert_eq!(slug("???"), "");
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
