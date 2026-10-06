//! Atualização do próprio app pela release fixa `native-latest`, reescrita pelo CI a cada push na main. Com o servidor
//! desta máquina num canal de testes (`pre_voo.alvo` fora da main), a release é a `native-<branch>` dessa branch.
//! Lê só o manifesto (pelo endereço de download, que não gasta a cota da API do GitHub), ao abrir e a cada 6 h.
//! Atualizar baixa o binário da plataforma, confere o sha256 do manifesto e troca o arquivo guardando o anterior em
//! `<exe>.old` (no Windows, `<exe>.old-<horário>` se o `.old` ainda estiver preso). O processo velho continua de pé
//! até o novo gravar o próprio pid em `<exe>.alive`: se ele morrer ou não der sinal a tempo, o anterior volta para o
//! lugar e este processo segue aberto. Nunca fica sem app.
use crate::{api::{Api, Failure}, i18n::tr, theme};
use gpui_kit::{assets::IconName, component::{button::*, notification::Notification, *}, *};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, ffi::OsString, path::{Path, PathBuf}, sync::Arc, time::Duration};
use tokio::runtime::Runtime;

const RELEASES: &str = "https://github.com/jeffer1312/hangar/releases/download";
/// Branch que o CI compilou (native.yml). Build local não sabe a dele: só versão mais nova o troca.
const BUILT_CHANNEL: Option<&str> = option_env!("HANGAR_NATIVE_CHANNEL");
const EVERY: Duration = Duration::from_secs(6 * 3600);
/// Janela GPUI fria num disco lento leva segundos; 30 s cobre com folga sem deixar o usuário esperando à toa.
const ALIVE_WAIT: Duration = Duration::from_secs(30);
const ALIVE_ENV: &str = "HANGAR_NATIVE_ALIVE";
pub const CURRENT: &str = env!("HANGAR_NATIVE_RELEASE");

/// Mesma limpeza do passo de publicação do native.yml: mudar uma exige mudar a outra.
fn release_tag(channel: &str) -> String {
    if channel == "main" { return "native-latest".into(); }
    format!("native-{}", channel.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '-' }).collect::<String>())
}

/// Só para provas: HANGAR_NATIVE_UPDATE_URL aponta para uma release falsa local.
fn base(channel: &str) -> String {
    std::env::var("HANGAR_NATIVE_UPDATE_URL").unwrap_or_else(|_| format!("{RELEASES}/{}", release_tag(channel)))
}

/// Nome do binário cru desta plataforma na release. Plataforma sem build no CI não procura nada.
fn asset() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("Hangar-linux-x86_64"),
        ("windows", "x86_64") => Some("Hangar-windows-x86_64.exe"),
        ("macos", "aarch64") => Some("Hangar-macos-aarch64"),
        _ => None,
    }
}

/// `0.1.0.2533` contra `0.1.0.2540`, número a número. Texto que não é versão nunca é "mais novo".
fn newer(remote: &str, local: &str) -> bool {
    let parse = |v: &str| v.split('.').map(str::parse::<u64>).collect::<Result<Vec<_>, _>>().ok();
    matches!((parse(remote), parse(local)), (Some(r), Some(l)) if r > l)
}

/// Versão mais nova da mesma release, ou qualquer versão de outra: trocar de canal (inclusive a volta para a main, que
/// costuma ter menos commits que a branch de teste) não pode esperar a contagem passar a do app instalado. Compara pela
/// release, não pelo nome: `a/b` e `a-b` publicam na mesma e não podem se oferecer uma à outra para sempre.
fn offered(remote: &str, channel: &str, current: &str, built: Option<&str>) -> bool {
    newer(remote, current) || (newer(remote, "0") && built.is_some_and(|built| release_tag(built) != release_tag(channel)))
}

/// Branch que o servidor segue. Sem o campo (servidor anterior ao canal, checkout ilegível), a main, como o backend.
pub(crate) fn alvo(state: &Value) -> String {
    state["pre_voo"]["alvo"].as_str().filter(|alvo| !alvo.is_empty()).unwrap_or("main").to_owned()
}

fn sha256_hex(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Deserialize)]
struct Manifest { version: String, files: HashMap<String, String> }

#[derive(Clone)]
struct Offer { version: String, url: String, sha256: String, channel: String }

/// Por que a procura não achou o que oferecer: a rede falhou, ou a máquina respondeu e a branch não tem o app.
#[derive(Clone, Debug, PartialEq)]
enum CheckFail { Network(String), NoRelease(String) }

impl std::fmt::Display for CheckFail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { CheckFail::Network(text) | CheckFail::NoRelease(text) => f.write_str(text) }
    }
}

async fn check(client: &reqwest::Client, channel: &str) -> Result<Option<Offer>, CheckFail> {
    let network = |e: reqwest::Error| CheckFail::Network(e.to_string());
    let missing = || if channel == "main" { Ok(None) }
        else { Err(CheckFail::NoRelease(tr("app_update_channel_missing").replace("{branch}", channel))) };
    let Some(name) = asset() else { return Ok(None) };
    let base = base(channel);
    let response = client.get(format!("{base}/native-latest.json")).send().await.map_err(network)?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        // Sem a release da main: nada a oferecer. Sem a da branch: avisa e fica, nunca oferece a da main no lugar.
        return missing();
    }
    let manifest: Manifest = response.error_for_status().map_err(network)?.json().await.map_err(network)?;
    // Branch cujo build desta plataforma falhou: avisa como a release ausente, em vez de dizer "em dia".
    let Some(sha256) = manifest.files.get(name) else { return missing() };
    Ok(offered(&manifest.version, channel, CURRENT, BUILT_CHANNEL).then(|| Offer { version: manifest.version,
        url: format!("{base}/{name}"), sha256: sha256.to_lowercase(), channel: channel.to_owned() }))
}

fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = exe.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Onde guardar o binário atual durante a troca.
#[cfg(unix)]
fn old_path(exe: &Path) -> PathBuf { sibling(exe, ".old") }

/// No Windows, um `.old` que ainda é a imagem de um processo vivo (versão anterior que não saiu) não pode ser apagado,
/// e renomear o app por cima dele dá "acesso negado". Os restos saem quando dá; o que ficou preso cede o nome.
#[cfg(windows)]
fn old_path(exe: &Path) -> PathBuf {
    // O NTFS não diferencia maiúsculas: o nome do `current_exe` pode vir com outra caixa que a do disco.
    let prefix = format!("{}.old", exe.file_name().unwrap_or_default().to_string_lossy()).to_lowercase();
    if let Some(entries) = exe.parent().and_then(|dir| std::fs::read_dir(dir).ok()) {
        for entry in entries.flatten().filter(|entry| entry.file_name().to_string_lossy().to_lowercase().starts_with(&prefix)) {
            if let Err(error) = std::fs::remove_file(entry.path()) {
                eprintln!("resto da atualização anterior preso em {}: {error}", entry.path().display());
            }
        }
    }
    let old = sibling(exe, ".old");
    // Arquivo que nega até a leitura (apagado mas ainda aberto) também ocupa o nome.
    if matches!(old.try_exists(), Ok(false)) { return old; }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    sibling(exe, &format!(".old-{stamp}"))
}

/// Confere o sha256 e troca o binário em disco. Devolve o caminho do anterior guardado, ou a frase da falha.
fn swap(exe: &Path, bytes: &[u8], sha256: &str) -> Result<PathBuf, String> {
    use std::io::Write;
    if sha256_hex(bytes) != sha256 { return Err(tr("app_update_bad_sha")); }
    let fail = |e: std::io::Error| tr("app_update_swap_failed").replace("{reason}", &e.to_string());
    let (new, old) = (sibling(exe, ".new"), old_path(exe));
    let place = || -> Result<(), String> {
        let mut file = std::fs::File::create(&new).map_err(fail)?;
        file.write_all(bytes).and_then(|_| file.sync_all()).map_err(fail)?;
        drop(file);
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755)).map_err(fail)?;
            // Cópia, não rename: entre dois renames o caminho do app ficaria vazio.
            std::fs::copy(exe, &old).map_err(fail)?;
            std::fs::rename(&new, exe).map_err(fail)?;
        }
        // No Windows o executável em uso não pode ser sobrescrito, mas pode ser renomeado.
        #[cfg(windows)] {
            std::fs::rename(exe, &old).map_err(fail)?;
            if let Err(error) = std::fs::rename(&new, exe) {
                // Sem desfazer, o caminho do app fica vazio: essa falha não pode sair como "nada foi trocado".
                if let Err(back) = std::fs::rename(&old, exe) {
                    return Err(tr("app_update_rollback_failed").replace("{reason}", &format!("{error}; {back}")));
                }
                return Err(fail(error));
            }
        }
        Ok(())
    };
    // Troca que falhou não deixa o binário baixado ao lado do app.
    place().inspect_err(|_| { let _ = std::fs::remove_file(&new); })?;
    Ok(old)
}

fn rollback(exe: &Path, old: &Path) -> std::io::Result<()> {
    // No Windows o `.old` é a imagem deste processo: renomear continua valendo, sobrescrever o novo exige tirá-lo antes.
    #[cfg(windows)] { let _ = std::fs::remove_file(exe); }
    std::fs::rename(old, exe)
}

/// O novo processo prova que subiu gravando o próprio pid; pid e não "arquivo existe", para um resto antigo não enganar.
async fn alive(child: &mut std::process::Child, path: &Path) -> bool {
    let deadline = tokio::time::Instant::now() + ALIVE_WAIT;
    while tokio::time::Instant::now() < deadline {
        if std::fs::read_to_string(path).is_ok_and(|pid| pid.trim() == child.id().to_string()) { return true; }
        if !matches!(child.try_wait(), Ok(None)) { return false; }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let _ = child.kill();
    let _ = child.wait();
    false
}

/// Sobe o binário que está no caminho do app e espera a prova de vida. Sem ela, a janela única volta para este processo.
async fn relaunch(exe: &Path) -> bool {
    let signal = sibling(exe, ".alive");
    let _ = std::fs::remove_file(&signal);
    // A versão nova assume o arquivo da janela única antes de provar que subiu.
    let own_address = crate::single_instance::snapshot();
    let started = std::process::Command::new(exe).args(std::env::args_os().skip(1)).env(ALIVE_ENV, &signal).spawn();
    let up = match started { Ok(mut child) => alive(&mut child, &signal).await, Err(_) => false };
    let _ = std::fs::remove_file(&signal);
    if !up { if let Some(address) = own_address { crate::single_instance::restore(address); } }
    up
}

async fn install(client: reqwest::Client, exe: Option<PathBuf>, offer: Offer) -> Result<(), String> {
    // A oferta pode ter horas e a release é republicada a cada push: o sha que vale é o do manifesto de agora.
    let offer = check(&client, &offer.channel).await.ok().flatten().unwrap_or(offer);
    let exe = exe.ok_or_else(|| tr("app_update_swap_failed").replace("{reason}", "current_exe"))?;
    // O instalador pode ter posto esta versão no caminho do app com ele aberto: não há o que baixar nem trocar.
    let (disk, wanted) = (exe.clone(), offer.sha256.clone());
    let placed = tokio::task::spawn_blocking(move || std::fs::read(&disk).is_ok_and(|bytes| sha256_hex(&bytes) == wanted));
    if placed.await.unwrap_or(false) {
        return if relaunch(&exe).await { Ok(()) } else { Err(tr("app_update_relaunch_failed")) };
    }
    let bytes = client.get(&offer.url).send().await.and_then(reqwest::Response::error_for_status).map_err(|e| e.to_string())?
        .bytes().await.map_err(|e| e.to_string())?;
    // O CI sobe os binários antes do manifesto: no meio da publicação o binário já é o novo e o sha ainda o antigo.
    let sha256 = if sha256_hex(&bytes) == offer.sha256 { offer.sha256 }
        else { check(&client, &offer.channel).await.ok().flatten().map_or(offer.sha256, |fresh| fresh.sha256) };
    // Dezenas de MB conferidos, gravados e copiados: fora das duas threads do runtime.
    let target = exe.clone();
    let old = tokio::task::spawn_blocking(move || swap(&target, &bytes, &sha256)).await.map_err(|e| e.to_string())??;
    if relaunch(&exe).await { return Ok(()); }
    rollback(&exe, &old).map_err(|e| tr("app_update_rollback_failed").replace("{reason}", &e.to_string()))?;
    Err(tr("app_update_rolled_back"))
}

pub fn relaunched() -> bool { std::env::var_os(ALIVE_ENV).is_some() }

/// Chamado pelo processo novo quando a janela abriu: é a prova de vida que o antigo espera.
pub fn report_alive() {
    if let Some(path) = std::env::var_os(ALIVE_ENV) { let _ = std::fs::write(path, std::process::id().to_string()); }
}

/// Por que o "Atualizar tudo" não mexe no servidor desta máquina.
#[derive(Clone, Debug, PartialEq)]
enum Hold {
    /// A atualização alinha o disco com a branch alvo (`pre_voo.alvo`, main em servidor antigo) e arrastaria a branch.
    WorkBranch(String, String),
    /// Mudanças locais, commits não enviados ou divergência: o motor faria stash + reset e tiraria do disco o trabalho de
    /// outras sessões. É o mesmo portão da atualização automática.
    LocalChanges,
    Running,
    Missing(String),
    ChannelDraft,
}

impl Hold {
    /// Só impedimentos à execução conjunta bloqueiam também o app.
    fn stops(&self) -> bool { matches!(self, Hold::Running | Hold::Missing(_)) }

    fn text(&self) -> String {
        match self {
            Hold::WorkBranch(branch, target) => tr("app_update_hold_branch").replace("{branch}", branch).replace("{alvo}", target),
            Hold::LocalChanges => tr("app_update_hold_changes"),
            Hold::Running => tr("app_update_hold_running"),
            Hold::Missing(what) => tr("app_update_hold_missing").replace("{what}", what),
            Hold::ChannelDraft => tr("settings_channel_pending"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum ServerStep { Skip, Update, Held(Hold) }

#[derive(Debug, PartialEq)]
struct Plan { server: ServerStep, app: bool }

impl Plan {
    fn visible(&self) -> bool { self.app || matches!(&self.server, ServerStep::Update) || matches!(&self.server, ServerStep::Held(h) if h.stops()) }
}

/// Servidor atrás: há commits a puxar, roda uma versão mais velha que o app que vai ficar, ou o disco não é o que está no
/// ar. O `-dirty` sai da comparação: qualquer edição depois do boot deixaria o servidor "atrás" para sempre.
fn behind(state: &Value, target: &str) -> bool {
    let flag = state["atualizacao_disponivel"].as_bool() == Some(true);
    let older = state["versao_legivel"]["backend"].as_str().is_some_and(|running| newer(target, running));
    let clean = |v: &Value| v.as_str().map(|s| s.trim_end_matches("-dirty").to_owned());
    let restart = matches!((clean(&state["versoes"]["repo"]), clean(&state["versoes"]["backend"])), (Some(disk), Some(up)) if disk != up);
    flag || older || restart
}

/// Lido do mesmo `pre_voo` que o servidor usa para recusar o `iniciar`: o botão sabe antes do clique.
fn hold(state: &Value) -> Option<Hold> {
    let pre = &state["pre_voo"];
    if pre["branch_de_trabalho"].as_bool() == Some(true) {
        return Some(Hold::WorkBranch(pre["branch"].as_str().unwrap_or("?").to_owned(), pre["alvo"].as_str().unwrap_or("main").to_owned()));
    }
    let count = |key: &str| pre[key].as_u64().unwrap_or(0);
    if count("sujo") > 0 || count("ahead") > 0 || pre["divergiu"].as_bool() == Some(true) { return Some(Hold::LocalChanges); }
    if state["estado"]["fase"].as_str() == Some("rodando") { return Some(Hold::Running); }
    if pre["pode"].as_bool() == Some(false) {
        let missing: Vec<&str> = pre["faltando"].as_array().map(|list| list.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let what = if missing.is_empty() { pre["erro"].as_str().unwrap_or("?").to_owned() } else { missing.join(", ") };
        return Some(Hold::Missing(what));
    }
    None
}

/// O que o "Atualizar tudo" faz: `current` é a versão deste app, `offer` a da release, `server` o `GET /api/atualizacao` do
/// servidor desta máquina (nenhum = não há servidor local).
fn plan(current: &str, offer: Option<&str>, server: Option<&Value>) -> Plan {
    let target = offer.unwrap_or(current);
    let server = match server {
        Some(state) if behind(state, target) => hold(state).map_or(ServerStep::Update, ServerStep::Held),
        _ => ServerStep::Skip,
    };
    let stops = matches!(&server, ServerStep::Held(h) if h.stops());
    Plan { app: offer.is_some() && !stops, server }
}

/// Servidor ativo mais velho que este app. O app só ganha versão quando um commit mexe no nativo e o servidor avança a
/// cada commit, então servidor à frente é o normal. Sem o campo, o servidor é anterior a ele: justamente o mais velho.
/// Campo nulo ou ilegível não afirma nada.
fn outdated(state: &Value, app: &str) -> bool {
    match state.get("versao_legivel").map(|v| v.get("backend")) {
        None | Some(None) => true,
        Some(Some(running)) => running.as_str().is_some_and(|running| newer(app, running)),
    }
}

#[derive(Debug, PartialEq)]
enum Outcome { Updated, UpdatedManual, Failed(String) }

#[derive(Debug, PartialEq)]
enum Follow { Running { step: u64, total: u64, text: String }, Waiting, NotStarted, Done(Outcome) }

/// Uma leitura do estado durante a atualização do servidor. `baseline` é o `ts` do desfecho anterior: "pronto" com ele, sem
/// ter visto "rodando", é o desfecho velho, não o desta vez.
fn follow(state: &Value, saw_running: bool, baseline: Option<&str>) -> Follow {
    let text = |key: &str| state[key].as_str().unwrap_or("").to_owned();
    match state["fase"].as_str() {
        Some("rodando") => Follow::Running { step: state["passo"].as_u64().unwrap_or(0), total: state["total"].as_u64().unwrap_or(0), text: text("texto") },
        Some("pronto") if !saw_running && state["ts"].as_str() == baseline => Follow::NotStarted,
        Some("pronto") if state["ok"].as_bool() != Some(true) => {
            let reason = Some(text("erro").trim_end_matches('.').to_owned()).filter(|e| !e.is_empty()).unwrap_or_else(|| "?".into());
            Follow::Done(Outcome::Failed(reason))
        }
        Some("pronto") if state["reiniciar_manual"].as_bool() == Some(true) => Follow::Done(Outcome::UpdatedManual),
        Some("pronto") => Follow::Done(Outcome::Updated),
        _ => Follow::Waiting,
    }
}

enum Run {
    Idle,
    Searching,
    Server { step: u64, total: u64, text: String },
    Restarting,
    DesktopRestart,
    App,
    Failed(String),
}

impl Run {
    fn busy(&self) -> bool { !matches!(self, Self::Idle | Self::Failed(_)) }

    fn begin_server_search(&mut self) -> bool {
        if self.busy() { return false; }
        *self = Self::Searching;
        true
    }
}

pub struct Updater {
    runtime: Arc<Runtime>,
    client: reqwest::Client,
    /// Lido ao abrir: depois da primeira troca o Linux passa a responder "<caminho> (deleted)" para este processo.
    exe: Option<PathBuf>,
    offer: Option<Offer>,
    /// O servidor desta máquina (endereço de loopback), dado pelo app sempre que a lista de servidores muda.
    local: Option<Api>,
    server: Option<Value>,
    server_seq: u64,
    /// O servidor ativo, para o aviso de servidor desatualizado (pode ser o mesmo do `local`).
    active: Option<Api>,
    active_state: Option<Value>,
    active_seq: u64,
    run: Run,
    /// Procura do app em andamento e o desfecho da última, para a página Sobre.
    checking: bool,
    checked: Option<Result<(), CheckFail>>,
    /// Branch da última procura que terminou: o canal do servidor mudar depois dela pede outra.
    checked_channel: Option<String>,
    /// Último `pre_voo.alvo` lido do servidor desta máquina: leitura que falha não muda o canal.
    known_channel: Option<String>,
    channel_blocked: Option<String>,
}

/// Linha "Canal de testes" da página Sobre: a main não mostra nada.
pub fn test_channel(branch: &str) -> Option<String> {
    (!branch.is_empty() && branch != "main").then(|| tr("about_test_channel").replace("{branch}", branch))
}

/// Canal do app na página Sobre: a branch do build e, quando a procura já segue outra, a próxima.
pub fn app_channel(built: Option<&str>, following: &str) -> Vec<String> {
    let built = built.filter(|built| !built.is_empty()).unwrap_or("main");
    let next = (release_tag(following) != release_tag(built)).then(|| tr("about_following_channel").replace("{branch}", following));
    test_channel(built).into_iter().chain(next).collect()
}

/// O que a página Sobre mostra na linha do app.
pub enum AppCheck { Never, Checking, UpToDate, Available(String), Failed(String), NoRelease(String) }

pub struct Handle(pub Entity<Updater>);
impl Global for Handle {}

/// Cria o indicador e começa a procurar. A barra do topo o encontra pelo `Handle` global.
pub fn start(runtime: Arc<Runtime>, cx: &mut App) {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(300)).user_agent(concat!("hangar-native/", env!("HANGAR_NATIVE_RELEASE")))
        .build().unwrap_or_default();
    let entity = cx.new(|_| Updater { runtime, client, exe: std::env::current_exe().ok(), offer: None, local: None, server: None,
        server_seq: 0, active: None, active_state: None, active_seq: 0, run: Run::Idle, checking: false, checked: None, checked_channel: None, known_channel: None, channel_blocked: None });
    let weak = entity.downgrade();
    cx.spawn(async move |cx| loop {
        let Ok(()) = weak.update(cx, |this, cx| {
            this.refresh_server(cx);
            this.refresh_active(cx);
            this.check_app(cx);
        }) else { return };
        cx.background_executor().timer(EVERY).await;
    }).detach();
    cx.set_global(Handle(entity));
}

fn read_state(runtime: &Runtime, api: &Api, query: &'static [(&'static str, &'static str)], seconds: u64) -> tokio::task::JoinHandle<Result<Value, Failure>> {
    let api = api.clone();
    runtime.spawn(async move { api.server_read(&["atualizacao"], query, seconds).await })
}

impl Updater {
    pub fn set_channel_blocked(&mut self, address: Option<String>, cx: &mut Context<Self>) {
        if self.channel_blocked == address { return; }
        self.channel_blocked = address;
        cx.notify();
    }

    pub fn is_channel_blocked(&self) -> bool {
        self.local.as_ref().is_some_and(|api| self.channel_blocked.as_deref() == Some(api.identity().as_str()))
    }

    /// Branch que o app segue: a última lida do servidor desta máquina. Antes da primeira leitura, a branch em que este app
    /// foi compilado (servidor fora do ar não troca de canal); sem servidor local, a main.
    fn channel(&self) -> String {
        if self.local.is_none() { return "main".into(); }
        self.known_channel.clone().unwrap_or_else(|| BUILT_CHANNEL.unwrap_or("main").to_owned())
    }

    /// Todo estado lido do servidor desta máquina passa por aqui: canal trocado fora deste app (celular, web, `.env`)
    /// refaz a procura da release, e a oferta da branch anterior deixa de valer.
    fn receive_server(&mut self, state: Option<Value>, cx: &mut Context<Self>) {
        if let Some(state) = &state { self.known_channel = Some(alvo(state)); }
        self.server = state;
        if self.checked_channel.as_ref().is_some_and(|checked| *checked != self.channel()) { self.check_app(cx); }
    }

    /// O canal de testes do servidor mudou pela tela: relê o estado e, com ele, a release do app.
    pub fn refresh(&mut self, cx: &mut Context<Self>) { self.refresh_server(cx) }

    /// A lista de servidores ou o servidor ativo mudou: relê o estado dos dois.
    pub fn set_servers(&mut self, local: Option<Api>, active: Option<Api>, cx: &mut Context<Self>) {
        if self.local.as_ref().map(Api::identity) != local.as_ref().map(Api::identity) { self.known_channel = None; }
        self.local = local;
        self.active = active;
        self.refresh_server(cx);
        self.refresh_active(cx);
    }

    /// Procura versão nova do app agora. Falha não vira aviso na tela: fica no stderr e na linha da página Sobre.
    pub fn check_app(&mut self, cx: &mut Context<Self>) {
        if self.checking { return; }
        self.checking = true;
        cx.notify();
        let (client, channel) = (self.client.clone(), self.channel());
        let task = { let channel = channel.clone(); self.runtime.spawn(async move { check(&client, &channel).await }) };
        cx.spawn(async move |this, cx| {
            let result = task.await.unwrap_or_else(|e| Err(CheckFail::Network(e.to_string())));
            let _ = this.update(cx, |this, cx| {
                this.checking = false;
                // O canal mudou durante a procura: a resposta é de outra release.
                if this.channel() != channel { return this.check_app(cx); }
                // `Failed` também: o "Tentar de novo" instala a oferta guardada.
                let idle = !this.busy();
                match result {
                    Ok(found) => {
                        if idle { this.offer = found; }
                        this.checked = Some(Ok(()));
                    }
                    Err(error) => {
                        eprintln!("procura de atualização do app falhou: {error}");
                        // A oferta de outro canal não vale mais: o Atualizar instalaria o app da branch errada.
                        if idle { this.offer.take_if(|offer| offer.channel != channel); }
                        this.checked = Some(Err(error));
                    }
                }
                this.checked_channel = Some(channel);
                cx.notify();
            });
        }).detach();
    }

    pub fn app_check(&self) -> AppCheck {
        match (&self.offer, self.checking, &self.checked) {
            (Some(offer), false, _) => AppCheck::Available(offer.version.clone()),
            (_, true, _) => AppCheck::Checking,
            (None, _, Some(Ok(()))) => AppCheck::UpToDate,
            (None, _, Some(Err(CheckFail::Network(error)))) => AppCheck::Failed(error.clone()),
            (None, _, Some(Err(CheckFail::NoRelease(text)))) => AppCheck::NoRelease(text.clone()),
            (None, _, None) => AppCheck::Never,
        }
    }

    /// Mesma ação do botão do topo: atualiza o servidor desta máquina se estiver atrás e depois o app.
    pub fn start_update(&mut self, window: &mut Window, cx: &mut Context<Self>) { self.run(window, cx) }

    pub fn is_busy(&self) -> bool { self.busy() }

    pub fn restart_desktop(&mut self, cx: &mut Context<Self>) -> Result<tokio::task::JoinHandle<bool>, String> {
        if self.busy() { return Err(tr("app_restart_busy")); }
        let exe = self.exe.clone().ok_or_else(|| tr("app_restart_failed"))?;
        self.run = Run::DesktopRestart;
        cx.notify();
        Ok(self.runtime.spawn(async move { relaunch(&exe).await }))
    }

    pub fn finish_desktop_restart(&mut self, cx: &mut Context<Self>) {
        // Falha manual não é falha de instalação: nunca oferece download no botão de tentar de novo.
        if matches!(self.run, Run::DesktopRestart) { self.run = Run::Idle; cx.notify(); }
    }

    pub fn channel_lines(&self) -> Vec<String> { app_channel(BUILT_CHANNEL, &self.channel()) }

    pub fn server_outdated(&self) -> bool { self.active_state.as_ref().is_some_and(|state| outdated(state, CURRENT)) }

    /// Versão do servidor ativo e a deste app, para a explicação do aviso.
    pub fn outdated_versions(&self) -> (String, String) {
        let running = self.active_state.as_ref().and_then(|s| s["versao_legivel"]["backend"].as_str()).unwrap_or("?").to_owned();
        (running, CURRENT.to_owned())
    }

    fn refresh_active(&mut self, cx: &mut Context<Self>) {
        self.active_seq += 1;
        let seq = self.active_seq;
        let Some(api) = self.active.clone() else { self.active_state = None; cx.notify(); return };
        let task = read_state(&self.runtime, &api, &[], 20);
        cx.spawn(async move |this, cx| {
            let result = task.await.unwrap_or_else(|e| Err(Failure::local(e.to_string())));
            let _ = this.update(cx, |this, cx| {
                if this.active_seq != seq { return; }
                // Sem resposta não há o que afirmar sobre a versão: o aviso some.
                this.active_state = result.ok();
                cx.notify();
            });
        }).detach();
    }

    fn busy(&self) -> bool { self.run.busy() }

    fn plan(&self) -> Plan {
        if self.is_channel_blocked() { return Plan { server: ServerStep::Held(Hold::ChannelDraft), app: self.offer.is_some() }; }
        let server = self.local.as_ref().and(self.server.as_ref());
        plan(CURRENT, self.offer.as_ref().map(|o| o.version.as_str()), server)
    }

    fn refresh_server(&mut self, cx: &mut Context<Self>) {
        if self.busy() { return; }
        self.server_seq += 1;
        let seq = self.server_seq;
        let Some(api) = self.local.clone() else { self.server = None; cx.notify(); return };
        let task = read_state(&self.runtime, &api, &[], 20);
        cx.spawn(async move |this, cx| {
            let result = task.await.unwrap_or_else(|e| Err(Failure::local(e.to_string())));
            let _ = this.update(cx, |this, cx| {
                if this.server_seq != seq { return; }
                // Servidor local fora do ar: sem ele o botão cuida só do app.
                this.receive_server(result.map_err(|e| eprintln!("estado do servidor desta máquina: {}", e.detail)).ok(), cx);
                cx.notify();
            });
        }).detach();
    }

    fn fail(&mut self, reason: String, cx: &mut Context<Self>) {
        self.run = Run::Failed(reason);
        cx.notify();
    }

    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() { return; }
        let plan = self.plan();
        match &plan.server {
            ServerStep::Held(hold) if hold.stops() => self.fail(hold.text(), cx),
            ServerStep::Update => {
                let this = cx.entity().downgrade();
                let (title, desc) = if plan.app { ("app_update_confirm_title", "app_update_confirm_desc") }
                    else { ("app_update_confirm_server_title", "app_update_confirm_server_desc") };
                crate::app::chrome::confirm_alert(window, cx, tr(title), tr(desc), tr("update_confirm_ok"), ButtonVariant::Primary,
                    move |window, cx| { let _ = this.update(cx, |this, cx| this.search_server(window, cx)); true });
            }
            _ if plan.app => self.confirm_channel(window, cx),
            _ => {}
        }
    }

    /// O canal pode ter mudado no servidor depois da última leitura (até 6 h): relê antes de instalar o app.
    fn confirm_channel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(api) = self.local.clone() else { return self.install_app(window, cx) };
        self.run = Run::Searching;
        cx.notify();
        let task = read_state(&self.runtime, &api, &[], 20);
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let result = task.await.unwrap_or_else(|e| Err(Failure::local(e.to_string())));
            let _ = handle.update(cx, |_, window, cx| { let _ = this.update(cx, |this, cx| {
                this.run = Run::Idle;
                // Sem resposta vale o último canal lido; a oferta de outro canal é barrada no `install_app`.
                if let Ok(state) = result { this.receive_server(Some(state), cx); }
                this.install_app(window, cx);
            }); });
        }).detach();
    }

    /// No clique a versão vem da rede: o `origin/main` do servidor só é renovado a cada 30 min, e a release do app sai logo
    /// depois do push. Com o estado novo, o plano é refeito antes de pedir qualquer coisa.
    fn search_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(api) = self.local.clone() else { return };
        if !self.run.begin_server_search() {
            window.push_notification(Notification::error(tr("app_restart_busy")), cx);
            return;
        }
        cx.notify();
        let task = read_state(&self.runtime, &api, &[("procurar", "1")], 150);
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let result = task.await.unwrap_or_else(|e| Err(Failure::local(e.to_string())));
            let _ = handle.update(cx, |_, window, cx| { let _ = this.update(cx, |this, cx| match result {
                Err(error) => this.fail(tr("update_failed").replace("{reason}", error.detail.trim_end_matches('.')), cx),
                Ok(state) => {
                    this.run = Run::Idle;
                    this.receive_server(Some(state), cx);
                    let plan = this.plan();
                    match &plan.server {
                        ServerStep::Update => this.start_server(api, window, cx),
                        ServerStep::Held(hold) if hold.stops() => this.fail(hold.text(), cx),
                        _ if plan.app => this.install_app(window, cx),
                        _ => cx.notify(),
                    }
                }
            }); });
        }).detach();
    }

    fn start_server(&mut self, api: Api, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_channel_blocked() { self.fail(tr("settings_channel_pending"), cx); return; }
        let baseline = self.server.as_ref().and_then(|s| s["estado"]["ts"].as_str()).map(str::to_owned);
        self.run = Run::Server { step: 0, total: 0, text: String::new() };
        cx.notify();
        let task = { let api = api.clone(); self.runtime.spawn(async move { api.server_post(&["atualizacao", "iniciar"], 30).await }) };
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let result = task.await.unwrap_or_else(|e| Err(Failure::local(e.to_string())));
            let _ = handle.update(cx, |_, window, cx| { let _ = this.update(cx, |this, cx| match result {
                // O servidor grava "rodando" antes de responder: o próximo "pronto" é o desfecho.
                Ok(_) => this.follow_server(api, true, baseline, window, cx),
                Err(error) if error.uncertain => this.follow_server(api, false, baseline, window, cx),
                // Recusa que o estado lido não previa: relê e decide pela mesma regra (branch → só o app).
                Err(error) if error.status == Some(409) => this.recheck_after_refusal(api, error.detail, window, cx),
                Err(error) => this.fail(tr("update_refused").replace("{reason}", &error.detail), cx),
            }); });
        }).detach();
    }

    fn recheck_after_refusal(&mut self, api: Api, detail: String, window: &mut Window, cx: &mut Context<Self>) {
        let task = read_state(&self.runtime, &api, &[], 20);
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let result = task.await.unwrap_or_else(|e| Err(Failure::local(e.to_string())));
            let _ = handle.update(cx, |_, window, cx| { let _ = this.update(cx, |this, cx| {
                this.run = Run::Idle;
                this.receive_server(result.ok(), cx);
                match this.plan() {
                    Plan { server: ServerStep::Held(hold), app: true } if !hold.stops() => this.install_app(window, cx),
                    _ => this.fail(tr("update_refused").replace("{reason}", &detail), cx),
                }
            }); });
        }).detach();
    }

    /// Lê o estado a cada 2 s até o desfecho, sem depender do servidor ativo nem da conexão da janela; a queda durante o
    /// reinício é esperada. Dez minutos sem desfecho encerram.
    fn follow_server(&mut self, api: Api, saw_running: bool, baseline: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let mut saw = saw_running;
            for _ in 0..300 {
                let Ok(task) = this.update(cx, |this, _| read_state(&this.runtime, &api, &[], 10)) else { return };
                let read = task.await.unwrap_or_else(|e| Err(Failure::local(e.to_string())));
                let step = match &read {
                    Ok(value) => Some(follow(&value["estado"], saw, baseline.as_deref())),
                    Err(error) if matches!(error.status, Some(401 | 403)) => Some(Follow::Done(Outcome::Failed(tr("auth_error")))),
                    Err(_) => None,
                };
                if matches!(step, Some(Follow::Running { .. })) { saw = true; }
                let finished = handle.update(cx, |_, window, cx| this.update(cx, |this, cx| {
                    if let Ok(value) = &read { this.receive_server(Some(value.clone()), cx); }
                    match step {
                        Some(Follow::Running { step, total, text }) => { this.run = Run::Server { step, total, text }; cx.notify(); false }
                        None if saw => { this.run = Run::Restarting; cx.notify(); false }
                        None | Some(Follow::Waiting) => false,
                        Some(Follow::NotStarted) => { this.fail(tr("update_not_started"), cx); true }
                        Some(Follow::Done(outcome)) => { this.server_done(outcome, window, cx); true }
                    }
                }).unwrap_or(true)).unwrap_or(true);
                if finished { return; }
                cx.background_executor().timer(Duration::from_secs(2)).await;
            }
            let _ = this.update(cx, |this, cx| this.fail(tr("update_silent"), cx));
        }).detach();
    }

    fn server_done(&mut self, outcome: Outcome, window: &mut Window, cx: &mut Context<Self>) {
        let manual = matches!(outcome, Outcome::UpdatedManual);
        match outcome {
            Outcome::Failed(reason) => return self.fail(tr("update_failed").replace("{reason}", &reason), cx),
            Outcome::Updated | Outcome::UpdatedManual => {}
        }
        self.run = Run::Idle;
        if self.offer.is_some() { return self.install_app(window, cx); }
        let version = self.server.as_ref().and_then(|s| s["versao_legivel"]["backend"].as_str()).unwrap_or("?").to_owned();
        let text = if manual { tr("update_done_manual") } else { tr("update_done").replace("{version}", &version) };
        window.push_notification(Notification::success(text).id::<Updater>(), cx);
        self.refresh_server(cx);
        self.refresh_active(cx);
    }

    fn install_app(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(offer) = self.offer.clone() else { self.run = Run::Idle; cx.notify(); return };
        // Canal trocado depois da procura: a oferta é da branch anterior; procura de novo em vez de instalá-la.
        if offer.channel != self.channel() {
            self.offer = None;
            self.run = Run::Idle;
            return self.check_app(cx);
        }
        self.run = Run::App;
        cx.notify();
        let task = self.runtime.spawn(install(self.client.clone(), self.exe.clone(), offer));
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let result = task.await.unwrap_or_else(|e| Err(e.to_string()));
            let _ = this.update(cx, |this, cx| match result {
                Ok(()) => cx.quit(),
                Err(reason) => {
                    // O motivo não cabe na barra: vai no aviso e fica no tooltip do "Tentar de novo".
                    let _ = handle.update(cx, |_, window, cx| window.push_notification(Notification::error(reason.clone()).id::<Updater>(), cx));
                    this.fail(reason, cx);
                }
            });
        }).detach();
    }

    /// Rótulo curto e explicação do botão parado, conforme o plano.
    fn idle_texts(&self, plan: &Plan) -> (String, String) {
        let version = self.offer.as_ref().map(|o| o.version.clone()).unwrap_or_default();
        let tip = match (&plan.server, plan.app) {
            (ServerStep::Held(hold), _) if hold.stops() => hold.text(),
            (ServerStep::Update, true) => tr("app_update_all_tip").replace("{version}", &version),
            (ServerStep::Update, false) => tr("app_update_server_tip"),
            (ServerStep::Held(hold), true) => format!("{} {}", tr("app_update_available").replace("{version}", &version), hold.text()),
            _ => tr("app_update_available").replace("{version}", &version),
        };
        (tr("app_update_now"), tip)
    }
}

impl Render for Updater {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let plan = self.plan();
        let (id, label, tip, color) = match &self.run {
            Run::Idle if !plan.visible() => return div().into_any_element(),
            Run::Idle => { let (label, tip) = self.idle_texts(&plan); ("topbar-update", label, tip, theme::accent()) }
            Run::Searching => ("topbar-update", tr("app_update_searching"), tr("app_update_searching"), theme::accent()),
            Run::Server { step, total, text } if *total > 0 => ("topbar-update",
                tr("app_update_server_step").replace("{step}", &step.to_string()).replace("{total}", &total.to_string()),
                tr("update_step").replace("{step}", &step.to_string()).replace("{total}", &total.to_string()).replace("{text}", text), theme::accent()),
            Run::Server { .. } => ("topbar-update", tr("update_running"), tr("update_running"), theme::accent()),
            Run::Restarting => ("topbar-update", tr("app_update_server_restarting"), tr("update_restarting"), theme::accent()),
            Run::DesktopRestart => ("topbar-update", tr("app_restarting"), tr("app_restarting"), theme::accent()),
            Run::App => ("topbar-update", tr("app_update_running"), tr("app_update_running"), theme::accent()),
            Run::Failed(reason) => ("topbar-update-retry", tr("app_update_retry"), reason.clone(), theme::danger()),
        };
        Button::new(id).ghost().small().h(px(26.)).px(px(10.)).rounded_full().border_1().border_color(color)
            .disabled(self.busy())
            .child(div().flex().items_center().gap(px(6.)).text_size(px(12.5))
                .child(Icon::new(IconName::Download).size(px(14.)).text_color(color))
                .child(label))
            .accessibility_label(tip.clone()).tooltip(tip)
            .on_click(cx.listener(|this, _, window, cx| this.run(window, cx)))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // O glob pode trazer o `test` da gpui, que colide com o atributo padrão; o nome explícito vence o glob.
    use core::prelude::v1::test;

    #[test]
    fn restart_and_installation_share_the_same_busy_gate() {
        assert!(!Run::Idle.busy());
        assert!(!Run::Failed("installation failed".into()).busy());
        for run in [Run::Searching, Run::Server { step: 0, total: 0, text: String::new() },
            Run::Restarting, Run::DesktopRestart, Run::App] {
            assert!(run.busy());
        }
    }

    #[test]
    fn pending_update_confirmation_cannot_overwrite_a_desktop_restart() {
        let mut run = Run::DesktopRestart;
        assert!(!run.begin_server_search());
        assert!(matches!(run, Run::DesktopRestart));
        let mut run = Run::Idle;
        assert!(run.begin_server_search());
        assert!(matches!(run, Run::Searching));
        assert!(!run.begin_server_search());
        let mut run = Run::Failed("failed".into());
        assert!(run.begin_server_search());
    }

    #[test]
    fn about_shows_test_channel_only_off_main() {
        assert_eq!(test_channel("main"), None);
        assert_eq!(test_channel(""), None);
        assert!(test_channel("feat/x").is_some_and(|text| text.contains("feat/x")));
        assert!(app_channel(None, "main").is_empty());
        assert!(app_channel(Some("main"), "main").is_empty());
        assert!(app_channel(Some(""), "main").is_empty());
        let same = app_channel(Some("feat/x"), "feat/x");
        assert_eq!(same.len(), 1);
        assert!(same[0].contains("feat/x"));
        // `a/b` e `a-b` publicam na mesma release: não é outro canal.
        assert_eq!(app_channel(Some("feat/x"), "feat-x").len(), 1);
        let back = app_channel(Some("feat/x"), "main");
        assert!(back[0].contains("feat/x") && back[1].contains("main"));
        let local = app_channel(None, "feat/y");
        assert_eq!(local.len(), 1);
        assert!(local[0].contains("feat/y"));
    }

    #[test]
    fn update_channel_draft_holds_only_its_local_server_update() {
        let local = Api::new("http://127.0.0.1:8765", "synthetic-token").unwrap();
        let mut updater = Updater { runtime: Arc::new(Runtime::new().unwrap()), client: reqwest::Client::new(), exe: None,
            offer: Some(Offer { version: "9999.0.0.0".into(), url: String::new(), sha256: String::new(), channel: "main".into() }),
            local: Some(local.clone()), server: Some(server(serde_json::json!({"atualizacao_disponivel": true}))),
            server_seq: 0, active: None, active_state: None, active_seq: 0, run: Run::Idle, checking: false, checked: None,
            checked_channel: None, known_channel: None, channel_blocked: Some(local.identity()) };
        assert_eq!(updater.plan(), Plan { server: ServerStep::Held(Hold::ChannelDraft), app: true });
        assert!(!Hold::ChannelDraft.stops());
        updater.channel_blocked = Some("http://other-machine:8765".into());
        assert!(updater.plan().app);
        assert_eq!(updater.plan().server, ServerStep::Update);
        updater.channel_blocked = None;
        assert_eq!(updater.plan().server, ServerStep::Update);
        updater.channel_blocked = Some(local.identity());
        updater.offer = None;
        assert!(!updater.plan().app);
        assert!(!updater.plan().visible());
    }

    #[test]
    fn newer_compares_number_by_number() {
        assert!(newer("0.1.0.2540", "0.1.0.2533"));
        assert!(newer("0.1.0.10", "0.1.0.9"));
        assert!(newer("0.2.0.1", "0.1.0.9999"));
        assert!(newer("0.1.0.1", "0.1.0"), "build sem git recebe a release");
        assert!(!newer("0.1.0.2533", "0.1.0.2533"));
        assert!(!newer("0.1.0.2500", "0.1.0.2533"));
        assert!(!newer("lixo", "0.1.0.1"));
        assert!(!newer("0.1.0.1", "2026.09.27-abc"));
    }

    #[test]
    fn release_tag_matches_the_workflow_cleanup() {
        assert_eq!(release_tag("main"), "native-latest");
        assert_eq!(release_tag("hangar-server-parte1"), "native-hangar-server-parte1");
        assert_eq!(release_tag("feat/native-test-channel"), "native-feat-native-test-channel");
        assert_eq!(release_tag("a+b_c.d"), "native-a-b_c.d");
    }

    #[test]
    fn offered_takes_a_newer_version_or_any_version_of_another_channel() {
        assert!(offered("0.1.0.110", "main", APP, Some("main")));
        assert!(!offered("0.1.0.90", "main", APP, Some("main")), "mesma branch, versão velha");
        assert!(offered("0.1.0.90", "main", APP, Some("hangar-server-parte1")), "canal desligado volta para a main");
        assert!(offered("0.1.0.90", "hangar-server-parte1", APP, Some("main")), "canal ligado troca mesmo com contagem menor");
        assert!(!offered("0.1.0.100", "hangar-server-parte1", APP, Some("hangar-server-parte1")));
        assert!(!offered("0.1.0.90", "hangar-server-parte1", APP, None), "build local: só a versão decide");
        assert!(!offered("0.1.0.90", "feat/x", APP, Some("feat-x")), "mesma release com outro nome não troca");
        assert!(!offered("lixo", "main", APP, Some("hangar-server-parte1")), "texto que não é versão nunca é oferecido");
    }

    #[test]
    fn channel_follows_the_last_state_read_from_the_local_server() {
        let mut updater = Updater { runtime: Arc::new(Runtime::new().unwrap()), client: reqwest::Client::new(), exe: None, offer: None,
            local: None, server: None, server_seq: 0, active: None, active_state: None, active_seq: 0, run: Run::Idle, checking: false,
            checked: None, checked_channel: None, known_channel: Some("hangar-server-parte1".into()), channel_blocked: None };
        assert_eq!(updater.channel(), "main", "sem servidor local não há canal");
        updater.local = Some(Api::new("http://127.0.0.1:8765", "synthetic-token").unwrap());
        assert_eq!(updater.channel(), "hangar-server-parte1", "leitura que falhou mantém o último canal");
        updater.known_channel = None;
        assert_eq!(updater.channel(), BUILT_CHANNEL.unwrap_or("main"), "antes da primeira leitura, o canal do build");
    }

    #[test]
    fn alvo_defaults_to_main_like_the_backend() {
        assert_eq!(alvo(&server(serde_json::json!({"pre_voo": {"alvo": "hangar-server-parte1"}}))), "hangar-server-parte1");
        assert_eq!(alvo(&server(serde_json::json!({}))), "main", "o server() de teste não traz alvo");
        assert_eq!(alvo(&serde_json::json!({"pre_voo": {"pode": false, "erro": "nao_e_repo"}})), "main");
        assert_eq!(alvo(&serde_json::json!({"versoes": {}})), "main", "servidor anterior ao canal");
    }

    fn server(extra: Value) -> Value {
        let mut base = serde_json::json!({
            "atualizacao_disponivel": false,
            "versoes": {"repo": "dist-latest-3-gabc", "backend": "dist-latest-3-gabc"},
            "versao_legivel": {"backend": "0.1.0.100"},
            "pre_voo": {"pode": true, "faltando": [], "branch": "main", "branch_de_trabalho": false, "sujo": 0, "ahead": 0, "divergiu": false},
            "estado": {"fase": "pronto", "ts": "t0"},
        });
        fn merge(into: &mut Value, from: Value) {
            match (into, from) {
                (Value::Object(a), Value::Object(b)) => for (k, v) in b { merge(a.entry(k).or_insert(Value::Null), v) },
                (slot, v) => *slot = v,
            }
        }
        merge(&mut base, extra);
        base
    }

    const APP: &str = "0.1.0.100";

    #[test]
    fn plan_updates_only_the_app_when_the_server_is_current() {
        let s = server(serde_json::json!({"versao_legivel": {"backend": "0.1.0.110"}}));
        assert_eq!(plan(APP, Some("0.1.0.110"), Some(&s)), Plan { server: ServerStep::Skip, app: true });
        assert_eq!(plan(APP, Some("0.1.0.110"), None), Plan { server: ServerStep::Skip, app: true }, "sem servidor local");
        assert!(!plan(APP, None, Some(&s)).visible(), "nada a fazer: botão some");
    }

    #[test]
    fn plan_updates_a_behind_server_first() {
        let s = server(serde_json::json!({"atualizacao_disponivel": true}));
        assert_eq!(plan(APP, Some("0.1.0.110"), Some(&s)), Plan { server: ServerStep::Update, app: true });
        let only = plan(APP, None, Some(&s));
        assert_eq!(only, Plan { server: ServerStep::Update, app: false });
        assert!(only.visible(), "servidor atrás sem app novo ainda mostra o botão");
    }

    #[test]
    fn plan_sees_a_server_older_than_the_offered_app_even_when_not_behind_main() {
        let s = server(serde_json::json!({"versao_legivel": {"backend": "0.1.0.105"}}));
        assert_eq!(plan(APP, Some("0.1.0.110"), Some(&s)).server, ServerStep::Update);
        assert_eq!(plan("0.1.0.105", None, Some(&s)).server, ServerStep::Skip);
    }

    #[test]
    fn plan_ignores_dirty_but_not_a_pending_restart() {
        let dirty = server(serde_json::json!({"versoes": {"repo": "dist-latest-3-gabc-dirty"}}));
        assert_eq!(plan(APP, None, Some(&dirty)).server, ServerStep::Skip);
        let restart = server(serde_json::json!({"versoes": {"repo": "dist-latest-4-gdef"}}));
        assert_eq!(plan(APP, None, Some(&restart)).server, ServerStep::Update);
    }

    #[test]
    fn plan_leaves_a_work_branch_or_local_changes_alone_and_updates_the_app() {
        let branch = server(serde_json::json!({"atualizacao_disponivel": true, "pre_voo": {"branch": "feat-x", "branch_de_trabalho": true}}));
        assert_eq!(plan(APP, Some("0.1.0.110"), Some(&branch)),
            Plan { server: ServerStep::Held(Hold::WorkBranch("feat-x".into(), "main".into())), app: true });
        let target = server(serde_json::json!({"atualizacao_disponivel": true, "pre_voo": {"branch": "outra", "alvo": "teste", "branch_de_trabalho": true}}));
        assert_eq!(plan(APP, Some("0.1.0.110"), Some(&target)).server,
            ServerStep::Held(Hold::WorkBranch("outra".into(), "teste".into())));
        for extra in [serde_json::json!({"sujo": 2}), serde_json::json!({"ahead": 1}), serde_json::json!({"divergiu": true})] {
            let s = server(serde_json::json!({"atualizacao_disponivel": true, "pre_voo": extra}));
            assert_eq!(plan(APP, Some("0.1.0.110"), Some(&s)), Plan { server: ServerStep::Held(Hold::LocalChanges), app: true });
        }
        assert!(!plan(APP, None, Some(&branch)).visible(), "sem app novo e servidor bloqueado: nada a fazer");
    }

    #[test]
    fn plan_stops_everything_while_missing_tools_or_already_running() {
        let missing = server(serde_json::json!({"atualizacao_disponivel": true, "pre_voo": {"pode": false, "faltando": ["npm", "uv"]}}));
        let p = plan(APP, Some("0.1.0.110"), Some(&missing));
        assert_eq!(p, Plan { server: ServerStep::Held(Hold::Missing("npm, uv".into())), app: false });
        assert!(p.visible(), "mostra o motivo");
        let running = server(serde_json::json!({"atualizacao_disponivel": true, "estado": {"fase": "rodando"}}));
        assert_eq!(plan(APP, Some("0.1.0.110"), Some(&running)), Plan { server: ServerStep::Held(Hold::Running), app: false });
    }

    #[test]
    fn outdated_only_when_the_server_is_older_than_the_app() {
        assert!(outdated(&serde_json::json!({"versao_legivel": {"backend": "0.1.0.90"}}), APP));
        assert!(!outdated(&serde_json::json!({"versao_legivel": {"backend": "0.1.0.100"}}), APP), "igual");
        assert!(!outdated(&serde_json::json!({"versao_legivel": {"backend": "0.1.0.140"}}), APP), "servidor à frente é o normal");
        assert!(outdated(&serde_json::json!({"versoes": {}}), APP), "servidor anterior ao campo");
        assert!(!outdated(&serde_json::json!({"versao_legivel": {"backend": null}}), APP), "nulo não afirma nada");
        assert!(!outdated(&serde_json::json!({"versao_legivel": {"backend": "2026.09.27-abc"}}), APP), "ilegível não afirma nada");
    }

    #[test]
    fn follow_reads_each_outcome() {
        let state = |v: Value| v;
        assert_eq!(follow(&state(serde_json::json!({"fase": "rodando", "passo": 4, "total": 5, "texto": "deps"})), false, Some("t0")),
            Follow::Running { step: 4, total: 5, text: "deps".into() });
        assert_eq!(follow(&state(serde_json::json!({"fase": "pronto", "ok": true, "ts": "t0"})), false, Some("t0")), Follow::NotStarted,
            "o desfecho antigo não é o desta vez");
        assert_eq!(follow(&state(serde_json::json!({"fase": "pronto", "ok": true, "ts": "t1"})), true, Some("t0")), Follow::Done(Outcome::Updated));
        assert_eq!(follow(&state(serde_json::json!({"fase": "pronto", "ok": true, "reiniciar_manual": true, "ts": "t1"})), true, Some("t0")),
            Follow::Done(Outcome::UpdatedManual));
        assert_eq!(follow(&state(serde_json::json!({"fase": "pronto", "ok": false, "erro": "npm ci falhou.", "ts": "t1"})), true, Some("t0")),
            Follow::Done(Outcome::Failed("npm ci falhou".into())));
        assert_eq!(follow(&Value::Null, true, Some("t0")), Follow::Waiting);
    }

    #[test]
    fn failed_swap_leaves_no_download_behind() {
        let dir = std::env::temp_dir().join(format!("hangar-swap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // O app não existe: a troca falha depois de gravar o `.new`.
        let exe = dir.join("app");
        assert!(swap(&exe, b"abc", &sha256_hex(b"abc")).is_err());
        assert!(!sibling(&exe, ".new").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sha256_matches_known_digest() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
