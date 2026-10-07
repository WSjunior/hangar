//! A entidade do assistente: escolhas, conferência prévia, as duas execuções do script, a senha de administrador e a conexão
//! no fim. Ocupa a janela inteira enquanto existe; o render mora em `screens.rs`.
use super::*;
use super::askpass::{self, Vault};
use super::failure::{Failure, FailureAction};
use super::flow::{self, PasswordMode, PasswordProblem, PhoneOutcome, Primary, Runs, Screen, View};
use super::local::{self, LocalInstall};
use super::marks::{End, Progress};
use super::phone::{self, Qr};
use super::precheck::{self, Check, CheckRow};
use super::run::{self, Kind, Options, RunRecord, SetupState};
use super::system;
use super::tail::Tail;
use crate::single_instance::AskpassRequest;

const POLL_EVERY: Duration = Duration::from_millis(400);
/// Voltas sem saída nova antes de perguntar se o processo ainda vive (~5 s).
const QUIET_CHECKS: u32 = 12;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AppCopy { Done(PathBuf), Skipped, Failed(String) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfterPassword { InstallGit }

/// `Quiet` leva o pid e a hora de início: o processo só vale se for o mesmo que o app iniciou.
enum Watch { Grew, Quiet(u32, String), Stop }

pub(crate) struct SetupWizard {
    hangar: WeakEntity<Hangar>,
    runtime: tokio::runtime::Handle,
    /// Lido ao abrir: depois de uma troca o Linux responde "<caminho> (deleted)" (`update.rs`).
    exe: Option<PathBuf>,
    pub(super) dest: PathBuf,
    located: bool,
    pub(super) agents: Vec<&'static str>,
    agents_touched: bool,
    pub(super) installed: Vec<&'static str>,
    pub(super) outside: bool,
    pub(super) password_mode: PasswordMode,
    pub(super) password: Entity<InputState>,
    pub(super) confirm: Entity<InputState>,
    /// `None` enquanto confere.
    pub(super) precheck: Option<Vec<CheckRow>>,
    pkg: Option<&'static str>,
    pub(super) started: bool,
    /// O que o app faz antes das marcas chegarem (instalar o git, baixar o bootstrap).
    pub(super) preparing: Option<String>,
    git_ready: bool,
    bootstrap: Option<PathBuf>,
    /// A senha do celular desta execução, só em memória.
    token: Option<String>,
    pub(super) runs: Runs,
    check_tail: Option<Tail>,
    install_tail: Option<Tail>,
    records: (Option<RunRecord>, Option<RunRecord>),
    polling: bool,
    pub(super) finished: bool,
    pub(super) failure: Option<Failure>,
    vault: Vault,
    waiting: Vec<AskpassRequest>,
    prompt: Option<Entity<PasswordPrompt>>,
    after_password: Option<AfterPassword>,
    opened_link: Option<String>,
    pub(super) app_copy: Option<AppCopy>,
    /// Endereço conectado no fim, ou o motivo de não ter conectado.
    pub(super) connection: Option<Result<String, String>>,
    /// Acabou escondido com o app ligado a outro servidor: a conexão local espera o "Abrir Hangar".
    connect_later: bool,
    /// Reaberto com o FIM já no arquivo: o fim aconteceu com o app fechado.
    resumed_ended: bool,
    pub(super) viewing: Screen,
    pub(super) details_open: bool,
    pub(super) phone: Option<PhoneOutcome>,
    pub(super) qr: Qr,
    /// A conta Tailscale já respondeu `Running` ao `tailscale status`.
    pub(super) tailscale_running: bool,
    tailscale_checking: bool,
    ticks: u32,
    pub(super) focus: FocusHandle,
    /// Para fechar a janela de senha de fora de um update com a janela emprestada (`fail`/`finish`).
    window: AnyWindowHandle,
    _subscriptions: Vec<Subscription>,
}

impl Drop for SetupWizard {
    fn drop(&mut self) {
        crate::single_instance::serve_askpass(None);
        for request in self.waiting.drain(..) { let _ = request.reply.send(None); }
    }
}

/// O assistente rodando suspende a procura e a troca do app (`update.rs`).
fn suspend_updates(on: bool, cx: &mut App) {
    if let Some(updater) = cx.try_global::<crate::update::Handle>().map(|handle| handle.0.clone()) {
        updater.update(cx, |updater, cx| updater.set_suspended(on, cx));
    }
}

impl SetupWizard {
    pub(super) fn new(hangar: WeakEntity<Hangar>, runtime: tokio::runtime::Handle, origin: Origin, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let password = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder(tr("setup_password_field")));
        let confirm = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder(tr("setup_password_confirm")));
        let subscriptions = vec![cx.subscribe(&password, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe(&confirm, |_, _, _: &InputEvent, cx| cx.notify())];
        // Pedidos de senha do `--askpass` chegam pela instância única; cada um entra por um update novo.
        let (askpass_tx, askpass_rx) = async_channel::unbounded::<AskpassRequest>();
        crate::single_instance::serve_askpass(Some(askpass_tx));
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(request) = askpass_rx.recv().await {
                if this.update_in(cx, |w, window, cx| w.on_askpass(request, window, cx)).is_err() { break; }
            }
        }).detach();
        let mut wizard = Self {
            hangar, runtime, exe: std::env::current_exe().ok(), dest: local::default_dir().unwrap_or_default(), located: false,
            agents: vec!["claude"], agents_touched: false, installed: Vec::new(), outside: false, password_mode: PasswordMode::Generate,
            password, confirm, precheck: None, pkg: None, started: false, preparing: None,
            git_ready: false, bootstrap: None, token: None, runs: Runs::default(), check_tail: None, install_tail: None, records: (None, None),
            polling: false, finished: false, failure: None, vault: Vault::default(), waiting: Vec::new(), prompt: None, after_password: None,
            opened_link: None, app_copy: None, connection: None, connect_later: false, resumed_ended: false, viewing: Screen::Welcome, details_open: false, phone: None,
            qr: Qr::Idle, tailscale_running: false, tailscale_checking: false, ticks: 0,
            focus: cx.focus_handle(), window: window.window_handle(), _subscriptions: subscriptions,
        };
        match origin {
            Origin::Entry(install) => { wizard.apply_install(install); wizard.located = true; }
            Origin::Menu => {}
            Origin::Resume(state) => wizard.resume(state, window, cx),
        }
        if !wizard.started { wizard.recheck(window, cx); }
        wizard.focus.focus(window, cx);
        wizard
    }

    /// A pasta achada decide onde roda e se a senha do celular já existe ("senha atual mantida").
    fn apply_install(&mut self, install: Option<LocalInstall>) {
        if let Some(dir) = install.as_ref().map(|i| i.dir.clone()).or_else(local::default_dir) { self.dest = dir; }
        self.password_mode = if install.as_ref().is_some_and(|i| i.token.is_some()) { PasswordMode::Keep } else { PasswordMode::Generate };
    }

    /// Reabrir no meio: as escolhas vêm do `state.json`, o andamento do arquivo de cada execução, lido do começo.
    fn resume(&mut self, state: SetupState, window: &mut Window, cx: &mut Context<Self>) {
        self.dest = state.dest;
        self.located = true;
        self.agents = state.options.agents.iter().filter_map(|a| system::AGENTS.iter().find(|(id, _)| id == a).map(|(id, _)| *id)).collect();
        self.agents_touched = true;
        self.outside = state.options.outside;
        // Só a conferência começou: a senha do celular escolhida morava na memória e o `--check` não a grava. Para a conferência
        // e volta à tela 1 sem começar, para a pessoa escolher a senha de novo.
        let Some(install) = state.install else {
            // Sem hora de início guardada o pid pode já ser de outro processo: não mata.
            if let Some(record) = state.check.filter(|r| !r.started.is_empty()) {
                cx.background_executor().spawn(async move { run::stop(record.pid, &record.started) }).detach();
            }
            run::clear_state();
            let token = local::read_install(&self.dest).token.is_some();
            self.password_mode = if token { PasswordMode::Keep } else { PasswordMode::Generate };
            return;
        };
        // O passo 0 já gravou a senha do celular: nunca uma senha nova no meio.
        self.password_mode = PasswordMode::Keep;
        (self.started, self.git_ready, self.precheck) = (true, true, Some(Vec::new()));
        self.vault = Vault::new(state.askpass_code);
        if let Some(record) = state.check {
            self.check_tail = Some(Tail::new(record.log.clone()));
            self.runs.check = Some(Progress::default());
            self.records.0 = Some(record);
        }
        self.install_tail = Some(Tail::new(install.log.clone()));
        self.runs.install = Some(Progress::default());
        self.records.1 = Some(install);
        self.bootstrap = run::state_dir().map(|dir| dir.join(run::bootstrap_name(cfg!(windows)))).filter(|p| p.is_file());
        self.read_tails(false);
        self.resumed_ended = self.runs.end().is_some();
        self.go(self.frontier_now(), cx);
        suspend_updates(true, cx);
        self.start_poll(window, cx);
    }

    pub(super) fn view(&self) -> View<'_> {
        View { started: self.started, precheck_blocked: self.precheck.as_deref().is_some_and(precheck::blocked), runs: &self.runs,
            failed_at: self.failure.as_ref().map(|f| f.screen), phone: self.phone }
    }

    fn frontier_now(&self) -> Screen { flow::frontier(&flow::screens(self.outside), &self.view()) }

    pub(super) fn password_problem(&self, cx: &App) -> Option<PasswordProblem> {
        flow::phone_password(&self.password_mode, &self.password.read(cx).value(), &self.confirm.read(cx).value()).err()
    }

    pub(super) fn can_start(&self, cx: &App) -> bool {
        !self.agents.is_empty() && self.password_problem(cx).is_none() && self.precheck.as_deref().is_some_and(|rows| !precheck::blocked(rows))
    }

    /// Começou e ainda não acabou nem falhou: fechar a janela não pode soltar o canal da senha nem religar a atualização.
    pub(super) fn is_running(&self) -> bool { self.started && !self.finished && self.failure.is_none() }

    fn options(&self) -> Options { Options { agents: self.agents.iter().map(|a| a.to_string()).collect(), outside: self.outside } }

    pub(super) fn recheck(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.precheck = None;
        let (locate, dest) = (!self.located, self.dest.clone());
        let (done, result) = tokio::sync::oneshot::channel();
        self.runtime.spawn(async move {
            let install = if locate {
                tokio::task::spawn_blocking(|| local::find_dir().map(|dir| local::read_install(&dir))).await.ok().flatten()
            } else { None };
            let dest = install.as_ref().map(|i| i.dir.clone()).unwrap_or(dest);
            let facts = precheck::gather(dest).await;
            let installed = tokio::task::spawn_blocking(|| system::installed_agents(&system::refreshed_path())).await.unwrap_or_default();
            let _ = done.send((locate.then_some(install), facts, installed));
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok((install, facts, installed)) = result.await else { return };
            let _ = this.update_in(cx, |w, window, cx| w.prechecked(install, facts, installed, window, cx));
        }).detach();
        cx.notify();
    }

    fn prechecked(&mut self, install: Option<Option<LocalInstall>>, facts: precheck::Facts, installed: Vec<&'static str>,
        window: &mut Window, cx: &mut Context<Self>) {
        if let Some(install) = install { self.apply_install(install); self.located = true; }
        self.pkg = facts.pkg;
        self.precheck = Some(precheck::rows(&facts));
        self.installed = installed;
        if !self.agents_touched { self.agents = flow::default_agents(&self.installed); }
        // Só para provar telas sem mouse: `HANGAR_SETUP_DEMO=start` (só Wi-Fi) ou `start-fora` (com a Tailscale).
        let demo = std::env::var("HANGAR_SETUP_DEMO").unwrap_or_default();
        if demo.starts_with("start") && !self.started {
            self.outside = demo == "start-fora";
            if self.can_start(cx) { self.start(window, cx); }
        }
        cx.notify();
    }

    pub(super) fn toggle_agent(&mut self, id: &'static str, cx: &mut Context<Self>) {
        if self.started { return; }
        self.agents = flow::toggle_agent(&self.agents, id);
        self.agents_touched = true;
        cx.notify();
    }

    pub(super) fn set_outside(&mut self, on: bool, cx: &mut Context<Self>) {
        if !self.started { self.outside = on; cx.notify(); }
    }

    pub(super) fn set_password_mode(&mut self, mode: PasswordMode, cx: &mut Context<Self>) {
        if !self.started && self.password_mode != PasswordMode::Keep { self.password_mode = mode; cx.notify(); }
    }

    pub(super) fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.started || !self.can_start(cx) { return; }
        let before = self.frontier_now();
        self.token = flow::phone_password(&self.password_mode, &self.password.read(cx).value(), &self.confirm.read(cx).value()).ok().flatten();
        self.started = true;
        self.vault = Vault::new(askpass::new_code());
        suspend_updates(true, cx);
        self.continue_start(window, cx);
        self.follow(before, cx);
    }

    /// Cada pedaço antes do script termina chamando de novo: git → bootstrap → conferência.
    fn continue_start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.failure.is_some() { return; }
        if !self.git_ready && self.precheck.as_deref().is_some_and(precheck::needs_git) {
            if !cfg!(windows) && self.vault.password().is_none() {
                self.after_password = Some(AfterPassword::InstallGit);
                return self.open_password_prompt(tr("setup_sudo_reason").replace("{motivo}", &tr("setup_sudo_reason_git")), window, cx);
            }
            return self.install_git(window, cx);
        }
        if self.bootstrap.is_none() { return self.download(window, cx); }
        self.launch(Kind::Check, window, cx);
    }

    fn install_git(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.preparing = Some(tr("setup_git_installing"));
        let (pkg, password) = (self.pkg, self.vault.password().map(str::to_owned));
        let (done, result) = tokio::sync::oneshot::channel();
        self.runtime.spawn_blocking(move || { let _ = done.send(precheck::install_git(pkg, password.as_deref())); });
        cx.spawn_in(window, async move |this, cx| {
            let outcome = result.await.unwrap_or_else(|_| Err(String::new()));
            let _ = this.update_in(cx, |w, window, cx| match outcome {
                Ok(()) => {
                    w.git_ready = true;
                    w.preparing = None;
                    if let Some(row) = w.precheck.iter_mut().flatten().find(|r| r.check == Check::Git) { row.ok = true; }
                    w.continue_start(window, cx);
                }
                Err(why) => w.fail(Failure::app(None, format!("{}\n{why}", tr("setup_git_failed")), Screen::Prepare), cx),
            });
        }).detach();
        cx.notify();
    }

    fn download(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.preparing = Some(tr("setup_prepare_downloading"));
        let dir = run::state_dir();
        let (done, result) = tokio::sync::oneshot::channel();
        self.runtime.spawn(async move {
            let fetched = match dir { Some(dir) => run::fetch_bootstrap(&dir, cfg!(windows)).await, None => Err("APPDATA/XDG_CONFIG_HOME".into()) };
            let _ = done.send(fetched);
        });
        cx.spawn_in(window, async move |this, cx| {
            let fetched = result.await.unwrap_or_else(|_| Err(String::new()));
            let _ = this.update_in(cx, |w, window, cx| match fetched {
                Ok(path) => { w.bootstrap = Some(path); w.continue_start(window, cx); }
                Err(why) => w.fail(Failure::app(None, tr("setup_failure_download").replace("{erro}", &why), Screen::Prepare), cx),
            });
        }).detach();
        cx.notify();
    }

    /// Linux: o atalho do askpass ao lado do `state.json`, e o código desta instalação no ambiente.
    fn askpass_env(&self) -> Option<(PathBuf, String)> {
        if cfg!(windows) { return None; }
        let wrapper = askpass::write_wrapper(&run::state_dir()?, self.exe.as_ref()?).ok()?;
        Some((wrapper, self.vault.code().to_owned()))
    }

    fn launch(&mut self, kind: Kind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bootstrap) = self.bootstrap.clone() else { return };
        let log = run::log_path(kind);
        let launch = run::Launch { kind, bootstrap, dest: self.dest.clone(), options: self.options(), log: log.clone(), token: self.token.clone(),
            askpass: self.askpass_env() };
        match run::spawn(&launch) {
            Ok(pid) => {
                // A hora de início chega do fundo (`learn_identity`): no Windows ela abre um PowerShell.
                let record = RunRecord { kind, log: log.clone(), pid, started: String::new() };
                match kind {
                    Kind::Check => { self.runs.check = Some(Progress::default()); self.check_tail = Some(Tail::new(log)); self.records.0 = Some(record); }
                    Kind::Install => { self.runs.install = Some(Progress::default()); self.install_tail = Some(Tail::new(log)); self.records.1 = Some(record); }
                }
                self.preparing = None;
                self.save_state();
                self.learn_identity(pid, cx);
                if !self.polling { self.start_poll(window, cx); }
            }
            Err(why) => self.fail(Failure::app(None, tr("setup_failure_start").replace("{erro}", &why), Screen::Prepare), cx),
        }
        cx.notify();
    }

    fn save_state(&self) {
        let state = SetupState { dest: self.dest.clone(), options: self.options(), askpass_code: self.vault.code().to_owned(),
            check: self.records.0.clone(), install: self.records.1.clone() };
        if let Err(error) = run::save_state(&state) { crate::log_line(&format!("assistente: state.json não gravado: {error}")); }
    }

    /// Lê a hora de início do processo fora da thread da janela e a grava no registro e no `state.json`.
    fn learn_identity(&self, pid: u32, cx: &mut Context<Self>) {
        let task = cx.background_executor().spawn(async move { run::read_identity(pid) });
        cx.spawn(async move |this, cx| {
            // Gone/Unknown ficam para a verificação da cadência Quiet, que tenta de novo.
            if let run::Identity::Known(id) = task.await { let _ = this.update(cx, |w, _| w.set_started(pid, id)); }
        }).detach();
    }

    fn set_started(&mut self, pid: u32, started: String) {
        for record in [self.records.0.as_mut(), self.records.1.as_mut()].into_iter().flatten() {
            if record.pid == pid { record.started = started.clone(); }
        }
        self.save_state();
    }

    /// Pid e hora de início da execução que anda: a instalação, quando já começou.
    fn active_run(&self) -> Option<(u32, String)> {
        self.records.1.as_ref().or(self.records.0.as_ref()).map(|r| (r.pid, r.started.clone()))
    }

    /// ponytail: lê na thread da janela; são poucos KB por volta. Mover para o executor de fundo se a saída crescer.
    /// `last`: o processo morreu, então o pedaço sem `\n` no fim do arquivo também é uma linha (um FIM sem quebra).
    fn read_tails(&mut self, last: bool) -> bool {
        let mut grew = false;
        for (tail, progress) in [(&mut self.check_tail, &mut self.runs.check), (&mut self.install_tail, &mut self.runs.install)] {
            let (Some(tail), Some(progress)) = (tail.as_mut(), progress.as_mut()) else { continue };
            // Arquivo ainda não criado: tenta na próxima volta.
            if let Ok(lines) = if last { tail.read_final() } else { tail.read_new() } {
                grew |= !lines.is_empty();
                for line in &lines { progress.feed(line); }
            }
        }
        grew
    }

    fn start_poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.polling = true;
        cx.spawn_in(window, async move |this, cx| {
            let mut quiet = 0u32;
            loop {
                cx.background_executor().timer(POLL_EVERY).await;
                let Ok(watch) = this.update_in(cx, |w, window, cx| w.tick(window, cx)) else { return };
                match watch {
                    Watch::Stop => return,
                    Watch::Grew => quiet = 0,
                    Watch::Quiet(pid, started) => {
                        quiet += 1;
                        if quiet % QUIET_CHECKS != 0 { continue; }
                        // A identidade lê o /proc (Linux) ou abre o PowerShell (Windows): fora da thread da janela.
                        // Sem hora de início guardada: `Known` a guarda, `Gone` é prova de que acabou, `Unknown` conta como vivo.
                        let (alive, learned) = cx.background_executor().spawn(async move {
                            if !started.is_empty() { return (run::alive(pid, &started), None); }
                            match run::read_identity(pid) {
                                run::Identity::Known(id) => (true, Some(id)),
                                run::Identity::Gone => (false, None),
                                run::Identity::Unknown => (true, None),
                            }
                        }).await;
                        if let Some(id) = learned && this.update(cx, |w, _| w.set_started(pid, id)).is_err() { return; }
                        if !alive && this.update(cx, |w, cx| w.process_gone(cx)).is_err() { return; }
                    }
                }
            }
        }).detach();
    }

    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Watch {
        if !self.polling { return Watch::Stop; }
        let before = self.frontier_now();
        let grew = self.read_tails(false);
        if self.runs.latest().is_some_and(Progress::mismatch) {
            if let Some((pid, started)) = self.active_run() {
                // `stop` lê a identidade e, no Windows, roda o `taskkill`: fora da thread da janela.
                cx.background_executor().spawn(async move {
                    let started = if started.is_empty() { run::identity(pid).unwrap_or_default() } else { started };
                    run::stop(pid, &started);
                }).detach();
            }
            self.fail(Failure::app(Some("versao-diferente"), tr("setup_failure_protocol"), before), cx);
            return Watch::Stop;
        }
        self.open_new_link(cx);
        self.ticks = self.ticks.wrapping_add(1);
        if self.ticks % 8 == 0 { self.watch_tailscale(cx); }
        if self.runs.install.is_none() {
            match self.runs.check.as_ref().and_then(|p| p.end) {
                Some(End::Ok | End::Pending) => {
                    self.launch(Kind::Install, window, cx);
                    self.follow(before, cx);
                    return Watch::Grew;
                }
                Some(End::Failed) => {
                    if let Some(failure) = self.runs.check.as_ref().map(|p| Failure::from_progress(p, Screen::Prepare)) { self.fail(failure, cx); }
                    return Watch::Stop;
                }
                None => {}
            }
        } else if self.runs.end().is_some() {
            self.finish(window, cx);
            self.follow(before, cx);
            return Watch::Stop;
        }
        self.follow(before, cx);
        if grew { cx.notify(); return Watch::Grew; }
        self.active_run().map_or(Watch::Stop, |(pid, started)| Watch::Quiet(pid, started))
    }

    /// `##HANGAR-LINK## tailscale-login`: abre o navegador uma vez por link novo. Com o dublê não abre sozinho (o link é de
    /// mentira); o botão "Entrar na Tailscale" continua abrindo.
    fn open_new_link(&mut self, cx: &mut Context<Self>) {
        let Some(url) = self.runs.latest().and_then(|p| p.link("tailscale-login")).map(str::to_owned) else { return };
        if self.opened_link.as_deref() == Some(url.as_str()) { return; }
        if run::test_bootstrap().is_none() { cx.open_url(&url); }
        self.opened_link = Some(url);
    }

    /// O processo sumiu: lê o que sobrou; sem `##HANGAR-FIM##`, a instalação foi interrompida.
    fn process_gone(&mut self, cx: &mut Context<Self>) {
        if !self.polling || self.read_tails(false) { return; }
        self.read_tails(true);
        let ended = self.runs.latest().is_none_or(|p| p.end.is_some());
        if !ended { let screen = self.frontier_now(); self.fail(Failure::app(None, tr("setup_failure_interrupted"), screen), cx); }
    }

    pub(super) fn fail(&mut self, failure: Failure, cx: &mut Context<Self>) {
        self.viewing = failure.screen;
        self.failure = Some(failure);
        (self.polling, self.preparing) = (false, None);
        self.vault.forget();
        self.after_password = None;
        for request in self.waiting.drain(..) { let _ = request.reply.send(None); }
        self.close_prompt(cx);
        suspend_updates(false, cx);
        cx.notify();
    }

    /// Fecha a janela de senha sem acionar o `on_close` (que cancelaria); adiado porque a janela pode estar emprestada.
    fn close_prompt(&mut self, cx: &mut Context<Self>) {
        if self.prompt.take().is_none() { return; }
        let window = self.window;
        cx.defer(move |cx| { let _ = window.update(cx, |_, window, cx| window.close_dialog(cx)); });
    }

    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.polling = false;
        self.vault.forget();
        for request in self.waiting.drain(..) { let _ = request.reply.send(None); }
        self.close_prompt(cx);
        suspend_updates(false, cx);
        if self.runs.end() == Some(End::Failed) {
            let screen = self.frontier_now();
            if let Some(failure) = self.runs.install.as_ref().map(|p| Failure::from_progress(p, screen)) { self.fail(failure, cx); }
            return;
        }
        // Reaberto com o fim já lido: não copia nem conecta duas vezes nesta abertura.
        if self.finished { return; }
        self.finished = true;
        self.copy_app(cx);
        self.connect_local(window, cx);
        cx.notify();
    }

    pub(super) fn copy_app(&mut self, cx: &mut Context<Self>) {
        if run::test_bootstrap().is_some() { self.app_copy = Some(AppCopy::Skipped); return; }
        let Some(exe) = self.exe.clone() else { self.app_copy = Some(AppCopy::Failed("current_exe".into())); return };
        self.app_copy = None;
        let task = cx.background_executor().spawn(async move { super::app_copy::install_self(&exe) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |w, cx| {
                w.app_copy = Some(match result { Ok(path) => AppCopy::Done(path), Err(why) => AppCopy::Failed(why) });
                cx.notify();
            });
        }).detach();
    }

    fn connect_local(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let install = local::read_install(&self.dest);
        let Some(token) = install.token.clone() else {
            let env = self.dest.join("backend").join(".env");
            self.connection = Some(Err(tr("setup_connection_missing").replace("{path}", &env.display().to_string())));
            return;
        };
        let address = local::address(install.port);
        // Escondido com o app já ligado a um servidor: não troca de servidor calado; conecta no "Abrir Hangar".
        let me = cx.entity_id();
        let hidden_while_connected = self.hangar.read_with(cx, |hangar, _| hangar.api.is_some()
            && hangar.setup_hidden.as_ref().is_some_and(|hidden| hidden.entity_id() == me)).unwrap_or(false);
        // Reaberto depois do fim com uma conexão salva: também não troca calado.
        let resumed_over_saved = self.resumed_ended && super::super::load_connection().is_some();
        if hidden_while_connected || resumed_over_saved { self.connect_later = true; return; }
        self.connection = Some(Ok(address.clone()));
        let _ = self.hangar.update(cx, |hangar, cx| hangar.setup_connect(address, token, window, cx));
    }

    pub(super) fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Pendência não para o script: rodar de novo antes do FIM seria um segundo instalador na mesma pasta.
        if self.is_running() { return; }
        let before = self.frontier_now();
        (self.failure, self.finished, self.started) = (None, false, true);
        self.runs = Runs::default();
        (self.check_tail, self.install_tail, self.records) = (None, None, (None, None));
        (self.app_copy, self.connection, self.opened_link, self.connect_later, self.resumed_ended) = (None, None, None, false, false);
        (self.qr, self.tailscale_running) = (Qr::Idle, false);
        // A senha escolhida continua em `token`; sem ela, o instalador mantém a do `.env` ou gera uma.
        self.vault = Vault::new(askpass::new_code());
        suspend_updates(true, cx);
        self.continue_start(window, cx);
        self.follow(before, cx);
    }

    pub(super) fn failure_action(&mut self, action: FailureAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            FailureAction::Retry => self.retry(window, cx),
            FailureAction::ToggleDetails => { self.details_open = !self.details_open; cx.notify(); }
        }
    }

    fn follow(&mut self, before: Screen, cx: &mut Context<Self>) {
        let next = flow::follow(self.viewing, before, self.frontier_now());
        if next != self.viewing { self.go(next, cx); } else { self.want_qr(cx); }
    }

    pub(super) fn go(&mut self, screen: Screen, cx: &mut Context<Self>) {
        self.viewing = screen;
        // A tela 5 pede o QR ao abrir, só depois do fim da instalação (o backend está de pé).
        self.want_qr(cx);
        cx.notify();
    }

    /// Só pede o código quando ele ainda não foi pedido: Failed e NoAddress esperam o botão da tela.
    fn want_qr(&mut self, cx: &mut Context<Self>) {
        if self.viewing == Screen::Phone && matches!(self.qr, Qr::Idle) && flow::status(Screen::Phone, &self.view()) != flow::Status::Pending { self.load_qr(cx); }
    }

    pub(super) fn load_qr(&mut self, cx: &mut Context<Self>) {
        if matches!(self.qr, Qr::Loading | Qr::Shown { .. }) { return; }
        let install = local::read_install(&self.dest);
        let Some(api) = install.token.as_ref().and_then(|token| crate::api::Api::new(&local::address(install.port), token).ok()) else {
            let env = self.dest.join("backend").join(".env");
            self.qr = Qr::Failed(tr("setup_connection_missing").replace("{path}", &env.display().to_string()));
            return cx.notify();
        };
        self.qr = Qr::Loading;
        let (done, result) = tokio::sync::oneshot::channel();
        self.runtime.spawn(async move { let _ = done.send(phone::load(api).await); });
        cx.spawn(async move |this, cx| {
            let loaded = result.await.unwrap_or_else(|_| Err(tr("connection_failed")));
            let _ = this.update(cx, |w, cx| {
                w.qr = match loaded {
                    Ok(Some(pairing)) => Qr::Shown { url: pairing.url, tailscale: pairing.tailscale,
                        image: Arc::new(Image::from_bytes(ImageFormat::Svg, pairing.svg)) },
                    Ok(None) => Qr::NoAddress,
                    Err(why) => Qr::Failed(why),
                };
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    /// Com o link de login na tela, confere a conta pelo `tailscale status`; o teto de 5 min é o do script.
    fn watch_tailscale(&mut self, cx: &mut Context<Self>) {
        if self.tailscale_running || self.tailscale_checking { return; }
        let waiting_login = self.runs.latest().and_then(|p| p.link("tailscale-login")).is_some()
            && self.runs.step(super::marks::Step::Tailscale) == Some(super::marks::State::Doing);
        if !waiting_login { return; }
        self.tailscale_checking = true;
        let task = cx.background_executor().spawn(async move { phone::tailscale_status() });
        cx.spawn(async move |this, cx| {
            let running = task.await;
            let _ = this.update(cx, |w, cx| {
                w.tailscale_checking = false;
                if running && !w.tailscale_running { w.tailscale_running = true; cx.notify(); }
            });
        }).detach();
    }

    pub(super) fn back(&mut self, cx: &mut Context<Self>) {
        let previous = flow::prev(&flow::screens(self.outside), self.viewing);
        self.go(previous, cx);
    }

    pub(super) fn press_primary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let list = flow::screens(self.outside);
        match flow::primary(self.viewing, self.started) {
            Primary::Start => self.start(window, cx),
            Primary::Continue => {
                let target = if self.viewing == Screen::Welcome { self.frontier_now() } else { flow::next(&list, self.viewing) };
                self.go(target, cx);
            }
            Primary::PhoneConnected => self.finish_phone(PhoneOutcome::Connected, cx),
            Primary::OpenHangar => self.open_hangar(window, cx),
        }
    }

    pub(super) fn finish_phone(&mut self, outcome: PhoneOutcome, cx: &mut Context<Self>) {
        self.phone = Some(outcome);
        self.go(Screen::Done, cx);
    }

    fn open_hangar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        run::clear_state();
        if std::mem::take(&mut self.connect_later) { self.connect_local(window, cx); }
        let _ = self.hangar.update(cx, |hangar, cx| hangar.close_setup(window, cx));
    }

    /// Com o script rodando a entidade só sai da tela (o canal da senha e a atualização suspensa continuam); sem nada
    /// rodando, larga tudo.
    pub(super) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_running() {
            let _ = self.hangar.update(cx, |hangar, cx| hangar.hide_setup(window, cx));
            return;
        }
        // Sem nada rodando o fim (ou a falha) já foi visto: o `state.json` não pode reabrir o assistente a cada início.
        run::clear_state();
        suspend_updates(false, cx);
        let _ = self.hangar.update(cx, |hangar, cx| hangar.close_setup(window, cx));
    }

    fn on_askpass(&mut self, request: AskpassRequest, window: &mut Window, cx: &mut Context<Self>) {
        // `--retry`: o `sudo -S` do script recusou a senha guardada; ela sai da memória e a janela volta com o aviso.
        let retry = request.retry && self.vault.answer(&request.code) != askpass::Answer::Refuse;
        if retry { self.vault.reject(); }
        match self.vault.answer(&request.code) {
            askpass::Answer::Refuse => { let _ = request.reply.send(None); }
            askpass::Answer::Known(password) => { let _ = request.reply.send(Some(password)); }
            askpass::Answer::Ask => {
                // O motivo vem do script (`app_sudo`); sem ele, o item em andamento.
                let what = Some(request.prompt.trim().to_owned()).filter(|t| !t.is_empty())
                    .or_else(|| self.runs.latest().and_then(Progress::doing_item).map(|i| i.text.clone()).filter(|t| !t.is_empty()))
                    .unwrap_or_else(|| tr("setup_sudo_reason_generic"));
                self.waiting.push(request);
                self.open_password_prompt(tr("setup_sudo_reason").replace("{motivo}", &what), window, cx);
                if retry {
                    if let Some(prompt) = self.prompt.clone() {
                        prompt.update(cx, |p, cx| { p.error = Some(tr("setup_sudo_wrong")); cx.notify(); });
                    }
                }
            }
        }
    }

    fn open_password_prompt(&mut self, reason: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.prompt.is_some() { return; }
        let input = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder(tr("setup_sudo_field")));
        let wizard = cx.entity().downgrade();
        let prompt = cx.new(|_| PasswordPrompt { wizard: wizard.clone(), input: input.clone(), reason, error: None, checking: false });
        self.prompt = Some(prompt.clone());
        let (on_ok, on_close) = (wizard.clone(), wizard);
        window.open_dialog(cx, move |dialog, _, cx| {
            let checking = prompt.read(cx).checking;
            let (on_ok, on_close) = (on_ok.clone(), on_close.clone());
            popup::dialog(dialog).w(px(440.)).title(tr("setup_sudo_title")).child(prompt.clone())
                .keyboard(!checking).overlay_closable(false).close_button(!checking)
                // Enter confere a senha; o diálogo só fecha quando o sudo aceitar.
                .on_ok(move |_, window, cx| { let _ = on_ok.update(cx, |w, cx| w.submit_password(window, cx)); false })
                .on_close(move |_, window, cx| {
                    let _ = on_close.update(cx, |w, cx| if w.prompt.is_some() { w.cancel_password(window, cx) });
                })
        });
        input.update(cx, |input, cx| input.focus(window, cx));
    }

    fn submit_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prompt) = self.prompt.clone() else { return };
        let (input, checking) = { let p = prompt.read(cx); (p.input.clone(), p.checking) };
        let password = input.read(cx).value().to_string();
        if password.is_empty() || checking { return; }
        prompt.update(cx, |p, cx| { (p.checking, p.error) = (true, None); cx.notify(); });
        let (done, result) = tokio::sync::oneshot::channel();
        self.runtime.spawn_blocking(move || {
            let accepted = askpass::sudo_accepts(&password, &system::refreshed_path());
            let _ = done.send((accepted, password));
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok((accepted, password)) = result.await else { return };
            let _ = this.update_in(cx, |w, window, cx| w.password_checked(accepted, password, window, cx));
        }).detach();
    }

    fn password_checked(&mut self, accepted: Result<bool, String>, password: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prompt) = self.prompt.clone() else { return };
        match accepted {
            Ok(true) => {
                let for_git = self.after_password == Some(AfterPassword::InstallGit);
                // O pedido do script espera 600 s: com o prazo vencido o canal já fechou, e a senha não entra no cofre.
                let delivered = std::mem::take(&mut self.waiting).into_iter()
                    .fold(false, |any, request| request.reply.send(Some(password.clone())).is_ok() || any);
                self.prompt = None;
                // Adiado: fechar agora chamaria o `on_close` dentro deste update do assistente.
                window.defer(cx, |window, cx| window.close_dialog(cx));
                if delivered || for_git { self.vault.remember(password); }
                if for_git && self.after_password.take() == Some(AfterPassword::InstallGit) { self.continue_start(window, cx); }
                cx.notify();
            }
            Ok(false) => prompt.update(cx, |p, cx| { (p.checking, p.error) = (false, Some(tr("setup_sudo_wrong"))); cx.notify(); }),
            Err(why) => prompt.update(cx, |p, cx| { (p.checking, p.error) = (false, Some(why)); cx.notify(); }),
        }
    }

    fn cancel_password(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.prompt = None;
        for request in self.waiting.drain(..) { let _ = request.reply.send(None); }
        if self.after_password.take() == Some(AfterPassword::InstallGit) {
            self.fail(Failure::app(Some("senha-cancelada"), tr("setup_sudo_cancelled"), Screen::Prepare), cx);
        }
        cx.notify();
    }
}

/// O corpo da janela de senha: o motivo, o campo e os dois botões.
pub(crate) struct PasswordPrompt {
    wizard: WeakEntity<SetupWizard>,
    input: Entity<InputState>,
    reason: String,
    error: Option<String>,
    checking: bool,
}

impl Render for PasswordPrompt {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let submit = self.wizard.clone();
        div().flex().flex_col().gap_3()
            .child(div().text_sm().whitespace_normal().child(self.reason.clone()))
            .child(Input::new(&self.input).aria_label(tr("setup_sudo_field")))
            .child(div().text_xs().text_color(theme::muted()).whitespace_normal().child(tr("setup_sudo_kept")))
            .when(self.checking, |el| el.child(div().id("setup-sudo-checking").role(Role::Status).text_sm().text_color(theme::muted())
                .child(tr("setup_sudo_checking"))))
            .when_some(self.error.clone(), |el, error| el.child(div().id("setup-sudo-error").role(Role::Alert).text_sm()
                .text_color(theme::danger()).child(error)))
            .child(div().flex().justify_end().gap_2()
                .child(Button::new("setup-sudo-cancel").label(tr("setup_sudo_cancel")).disabled(self.checking)
                    .on_click(|_, window, cx| window.close_dialog(cx)))
                .child(Button::new("setup-sudo-ok").primary().label(tr("setup_sudo_ok")).disabled(self.checking)
                    .on_click(move |_, window, cx| { let _ = submit.update(cx, |w, cx| w.submit_password(window, cx)); })))
    }
}
