//! Convidados (Sincronização): o mesmo cadastro do web, no mesmo formato, pra um lado editar o que o outro gravou. A conta
//! do convidado mora no hub; o acesso, em cada servidor escolhido, criado com o token do dono daquele servidor.
use super::*;
use super::sync::{PBKDF2_ITERATIONS, decrypt_json, derive_keys, encrypt_json, random, sync_field};
use super::settings::Page;
use super::create::{Root, Scan, choice, crumbs, scan_of};
use super::device::Remote;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GuestServer { server_id: String, guest_id: String, token: String, root: String }

/// O `admin_blob`, cifrado com a chave do dono: a senha fica junto pra editar sem pedi-la de novo.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GuestAdmin { user: String, password: String, sees_owner: bool, owner_sees: bool, servers: Vec<GuestServer> }

// À mão pra a senha nunca ir parar num log ou numa mensagem de teste.
impl std::fmt::Debug for GuestAdmin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuestAdmin").field("user", &self.user).field("sees_owner", &self.sees_owner)
            .field("owner_sees", &self.owner_sees).field("servers", &self.servers).finish_non_exhaustive()
    }
}

/// Servidor da lista do dono, como o cofre guarda. `base_url` vazio é o próprio hub.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OwnerServer {
    id: String,
    #[serde(default)] label: String,
    #[serde(default)] base_url: String,
    #[serde(default)] token: String,
    #[serde(default)] invite: bool,
}

struct Draft { user: String, password: String, sees_owner: bool, owner_sees: bool, servers: Vec<(String, String)> }

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ServerFailure { label: String, message: String }

/// `expired`: o hub recusou o cookie (401); a sessão caiu e a página volta pra entrada.
pub(super) struct SaveOutcome { saved: GuestAdmin, errors: Vec<ServerFailure>, hub_failed: bool, expired: bool }

/// Sessão do dono no hub: só em memória, some ao sair da página.
#[derive(Clone)]
pub(super) struct Session { user: String, cookie: String, key: [u8; 32], servers: Vec<OwnerServer> }

trait GuestOps {
    async fn create(&self, server: &OwnerServer, draft: &Draft, root: &str) -> Result<(String, String), Failure>;
    async fn update(&self, server: &OwnerServer, id: &str, draft: &Draft, root: &str) -> Result<(), Failure>;
    async fn delete(&self, server: &OwnerServer, id: &str) -> Result<(), Failure>;
    async fn put_hub(&self, saved: &GuestAdmin, servers: Value) -> Result<(), Failure>;
    async fn delete_hub(&self, user: &str) -> Result<(), Failure>;
}

fn describe(error: &Failure) -> String { Hangar::setting_failure(error) }

fn hub_message(error: &Failure) -> String {
    match error.status {
        Some(401) => tr_shared("erro_nao_autorizado", &[]),
        Some(403) => tr_shared("erro_so_dono", &[]),
        _ => describe(error),
    }
}

fn validate(draft: &Draft, editing: bool, owner: &str, guests: &[GuestAdmin]) -> Option<&'static str> {
    let name = draft.user.as_str();
    if name.is_empty() { return Some("convidados_usuario_obrigatorio"); }
    // O hub recusa "/" depois de os servidores já terem criado o convidado.
    if name.contains('/') { return Some("convidados_usuario_barra"); }
    // Nome repetido sobrescreveria outro convidado (ou bateria no dono) depois de já ter criado acesso nos servidores.
    if !editing && (name == owner || guests.iter().any(|g| g.user == name)) { return Some("convidados_usuario_em_uso"); }
    if draft.password.chars().count() < 8 { return Some("sync_password_min"); }
    if draft.servers.is_empty() { return Some("convidados_servidor_obrigatorio"); }
    if draft.servers.iter().any(|(_, root)| root.is_empty()) { return Some("convidados_pasta_obrigatoria"); }
    None
}

async fn save_guest(ops: &impl GuestOps, owner: &[OwnerServer], draft: &Draft, prev: Option<&GuestAdmin>) -> SaveOutcome {
    let find = |id: &str| owner.iter().find(|s| s.id == id);
    let before = |id: &str| prev.and_then(|p| p.servers.iter().find(|s| s.server_id == id));
    let gone = |error: &Failure| error.status == Some(404);
    let away = |label: &str| ServerFailure { label: label.to_owned(), message: tr_shared("convidados_servidor_fora_da_lista", &[]) };
    let (mut kept, mut errors) = (Vec::new(), Vec::new());
    for (id, root) in &draft.servers {
        let old = before(id);
        let Some(server) = find(id) else {
            // Sem o servidor na lista não dá pra agir nele; o token antigo fica no cadastro.
            errors.push(away(id));
            if let Some(old) = old { kept.push(old.clone()); }
            continue;
        };
        let create = || async {
            ops.create(server, draft, root).await
                .map(|(guest_id, token)| GuestServer { server_id: id.clone(), guest_id, token, root: root.clone() })
        };
        let result = match old {
            None => create().await,
            Some(old) => match ops.update(server, &old.guest_id, draft, root).await {
                Ok(()) => Ok(GuestServer { root: root.clone(), ..old.clone() }),
                // Sumiu no servidor: recria em vez de ficar com um id morto.
                Err(error) if gone(&error) => create().await,
                Err(error) => Err(error),
            },
        };
        match result {
            Ok(entry) => kept.push(entry),
            Err(error) => {
                errors.push(ServerFailure { label: server.label.clone(), message: describe(&error) });
                if let Some(old) = old { kept.push(old.clone()); }
            }
        }
    }
    for old in prev.map_or(&[][..], |p| &p.servers) {
        if draft.servers.iter().any(|(id, _)| *id == old.server_id) { continue; }
        let Some(server) = find(&old.server_id) else {
            errors.push(away(&old.server_id));
            kept.push(old.clone());
            continue;
        };
        match ops.delete(server, &old.guest_id).await {
            Ok(()) => {}
            Err(error) if gone(&error) => {} // já removido
            Err(error) => {
                // Fica no cadastro pra poder tentar remover de novo.
                errors.push(ServerFailure { label: server.label.clone(), message: describe(&error) });
                kept.push(old.clone());
            }
        }
    }
    let saved = GuestAdmin { user: draft.user.clone(), password: draft.password.clone(), sees_owner: draft.sees_owner,
        owner_sees: draft.owner_sees, servers: kept };
    // O endereço vai como está na lista do dono, inclusive vazio: o app do convidado resolve igual ao do dono.
    let list = saved.servers.iter().filter_map(|k| find(&k.server_id)
        .map(|s| json!({"id": s.id, "label": s.label, "baseUrl": s.base_url, "token": k.token}))).collect();
    if let Err(error) = ops.put_hub(&saved, Value::Array(list)).await {
        // Os servidores já mudaram: `saved` volta pro formulário e salvar de novo repete com ele, sem duplicar.
        errors.push(ServerFailure { label: tr_shared("sync_config_titulo", &[]), message: hub_message(&error) });
        return SaveOutcome { saved, errors, hub_failed: true, expired: error.status == Some(401) };
    }
    SaveOutcome { saved, errors, hub_failed: false, expired: false }
}

/// Devolve as falhas e se o hub recusou o cookie (401).
async fn remove_guest(ops: &impl GuestOps, owner: &[OwnerServer], guest: &GuestAdmin) -> (Vec<ServerFailure>, bool) {
    let draft = Draft { user: guest.user.clone(), password: guest.password.clone(), sees_owner: guest.sees_owner,
        owner_sees: guest.owner_sees, servers: Vec::new() };
    let out = save_guest(ops, owner, &draft, Some(guest)).await;
    // Só apaga a conta do hub quando todos os servidores largaram o acesso e o hub aceitou a gravação.
    if out.saved.servers.is_empty() && !out.hub_failed {
        if let Err(error) = ops.delete_hub(&guest.user).await {
            return (vec![ServerFailure { label: guest.user.clone(), message: hub_message(&error) }], error.status == Some(401));
        }
    }
    (out.errors, out.expired)
}

/// Conexão com um servidor do dono, com o token dele; endereço vazio é o próprio hub.
fn server_api(hub: &Api, server: &OwnerServer) -> Result<Api, Failure> {
    let identity = hub.identity();
    Api::new(if server.base_url.is_empty() { &identity } else { &server.base_url }, &server.token)
}

/// As pastas são as do servidor que está sendo configurado, não as do ativo.
fn folder_api(hub: Option<&Api>, session: Option<&Session>, id: &str) -> Result<Api, String> {
    match (hub, session.and_then(|s| s.servers.iter().find(|s| s.id == id))) {
        (Some(hub), Some(server)) => server_api(hub, server).map_err(|error| Hangar::failure(&error)),
        _ => Err(tr_shared("convidados_servidor_fora_da_lista", &[])),
    }
}

/// `path` dentro de `root` (ou a própria raiz).
fn inside(root: &str, path: &str) -> bool {
    path.strip_prefix(root.trim_end_matches('/')).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Onde o seletor abre: na pasta já escolhida, pela raiz mais funda que a contém; sem ela, no topo da primeira raiz.
fn start_at(roots: &[Root], chosen: &str) -> Option<(Root, String)> {
    let holder = roots.iter().filter(|r| !chosen.is_empty() && inside(&r.path, chosen)).max_by_key(|r| r.path.len());
    match holder {
        Some(root) => Some((root.clone(), chosen.to_owned())),
        None => roots.first().map(|root| (root.clone(), root.path.clone())),
    }
}

// Número global: o seletor é recriado a cada abertura, e a resposta de um anterior não pode casar com o pedido do novo.
fn begin<T>(remote: &mut Remote<T>) -> u64 {
    (remote.seq, remote.loading) = (ticket(), true);
    remote.seq
}

struct Live { hub: Api, session: Session }

impl Live {
    fn api(&self, server: &OwnerServer) -> Result<Api, Failure> { server_api(&self.hub, server) }
}

impl GuestOps for Live {
    async fn create(&self, server: &OwnerServer, draft: &Draft, root: &str) -> Result<(String, String), Failure> {
        let body = json!({"name": draft.user, "root": root, "sees_owner": draft.sees_owner, "owner_sees": draft.owner_sees});
        let value = self.api(server)?.server_send(reqwest::Method::POST, &["guests"], Some(body), 30).await?;
        let field = |name: &str| value.get(name).and_then(Value::as_str).map(str::to_owned);
        field("id").zip(field("token")).ok_or_else(|| Failure::local("invalid_response"))
    }

    async fn update(&self, server: &OwnerServer, id: &str, draft: &Draft, root: &str) -> Result<(), Failure> {
        let body = json!({"root": root, "sees_owner": draft.sees_owner, "owner_sees": draft.owner_sees});
        self.api(server)?.server_send(reqwest::Method::POST, &["guests", id], Some(body), 30).await.map(|_| ())
    }

    async fn delete(&self, server: &OwnerServer, id: &str) -> Result<(), Failure> {
        self.api(server)?.server_post(&["guests", id, "delete"], 30).await.map(|_| ())
    }

    async fn put_hub(&self, saved: &GuestAdmin, servers: Value) -> Result<(), Failure> {
        let (saved, key) = (saved.clone(), self.session.key);
        // PBKDF2 com 600 mil voltas: fora das threads do runtime.
        let body = tokio::task::spawn_blocking(move || {
            let salt = random::<16>()?;
            let keys = derive_keys(&saved.password, &salt, PBKDF2_ITERATIONS)?;
            Some(json!({"user": saved.user, "salt": STANDARD.encode(salt), "auth_hash": keys.auth_hash,
                "enc_blob": encrypt_json(&keys.enc, &servers)?, "admin_blob": encrypt_json(&key, &saved)?}))
        }).await.ok().flatten().ok_or_else(|| Failure::local("sync_config_erro"))?;
        self.hub.hub(reqwest::Method::POST, &["sync", "guests"], &[], Some(body), Some(&self.session.cookie), 30).await.map(|_| ())
    }

    async fn delete_hub(&self, user: &str) -> Result<(), Failure> {
        match self.hub.hub(reqwest::Method::POST, &["sync", "guests", user, "delete"], &[], None, Some(&self.session.cookie), 30).await {
            Err(error) if error.status != Some(404) => Err(error),
            _ => Ok(()),
        }
    }
}

async fn load_guests(hub: &Api, session: &Session) -> Result<Vec<GuestAdmin>, Failure> {
    let (rows, _) = hub.hub(reqwest::Method::GET, &["sync", "guests"], &[], None, Some(&session.cookie), 15).await?;
    rows.as_array().ok_or_else(|| Failure::local("invalid_response"))?.iter()
        .map(|row| row.get("admin_blob").and_then(|blob| decrypt_json(&session.key, blob)).ok_or_else(|| Failure::local("invalid_response")))
        .collect()
}

/// Entra no hub como o web: prelogin → chaves → login (cookie) → cofre com a lista de servidores do dono.
async fn open_session(hub: &Api, user: String, password: String) -> Result<Session, String> {
    let get = |path: &'static [&'static str], query: Vec<(&'static str, String)>| {
        let hub = hub.clone();
        async move {
            let query: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
            hub.hub(reqwest::Method::GET, path, &query, None, None, 15).await
        }
    };
    let (pre, _) = get(&["sync", "prelogin"], vec![("user", user.clone())]).await.map_err(|e| describe(&e))?;
    let salt = pre.get("salt").and_then(Value::as_str).and_then(|s| STANDARD.decode(s).ok());
    let iterations = pre.get("iterations").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
    let (Some(salt), Some(iterations)) = (salt, iterations) else { return Err(tr("invalid_response")) };
    let keys = tokio::task::spawn_blocking(move || derive_keys(&password, &salt, iterations)).await.ok().flatten()
        .ok_or_else(|| tr("invalid_response"))?;
    let login = hub.hub(reqwest::Method::POST, &["sync", "login"], &[], Some(json!({"user": user, "auth_hash": keys.auth_hash})), None, 15).await;
    let cookie = match login {
        Ok((_, Some(cookie))) => cookie,
        Ok((_, None)) => return Err(tr("invalid_response")),
        Err(error) if error.status == Some(401) => return Err(tr_shared("sync_credenciais_invalidas", &[])),
        Err(error) if error.status == Some(429) => return Err(tr_shared("sync_muitas_tentativas", &[])),
        Err(error) => return Err(describe(&error)),
    };
    let (vault, _) = hub.hub(reqwest::Method::GET, &["sync", "vault"], &[], None, Some(&cookie), 15).await.map_err(|e| hub_message(&e))?;
    let servers: Vec<OwnerServer> = match vault.get("enc_blob") {
        None | Some(Value::Null) => Vec::new(),
        Some(blob) => decrypt_json(&keys.enc, blob).ok_or_else(|| tr("invalid_response"))?,
    };
    // Convite fica só no aparelho que resgatou: não é servidor do dono pra dar acesso.
    Ok(Session { user, cookie, key: keys.enc, servers: servers.into_iter().filter(|s| !s.invite).collect() })
}

struct LoginForm { user: Entity<InputState>, password: Entity<InputState>, _subscriptions: Vec<Subscription> }

struct GuestForm {
    editing: Option<GuestAdmin>,
    user: Entity<InputState>,
    password: Entity<InputState>,
    sees_owner: bool,
    owner_sees: bool,
    // Servidor marcado = tem pasta aqui (vazia até escolher); a ordem é a da marcação, como no web.
    roots: Vec<(String, String)>,
    /// O seletor de pasta aberto; um por vez.
    browser: Option<FolderBrowser>,
    error: Option<String>,
}

/// Navega pelas pastas de um servidor marcado: raízes → subpastas → escolher, ou criar uma nova na pasta atual.
struct FolderBrowser {
    server: String,
    roots: Remote<Vec<Root>>,
    root: Option<Root>,
    dir: String,
    scan: Remote<Scan>,
    /// Campo do nome da pasta nova; a assinatura cria no Enter.
    naming: Option<(Entity<InputState>, Subscription)>,
    /// Número da criação em voo.
    making: Option<u64>,
    make_error: Option<String>,
}

impl FolderBrowser {
    fn new(server: String) -> Self {
        Self { server, roots: Remote::default(), root: None, dir: String::new(), scan: Remote::default(), naming: None, making: None,
            make_error: None }
    }
}

#[derive(Default)]
pub(in crate::app) struct Guests {
    login: Option<LoginForm>,
    login_error: Option<String>,
    session: Option<Session>,
    list: Option<Result<Vec<GuestAdmin>, String>>,
    /// Pedido em voo; a resposta só vale se for deste número.
    waiting: Option<u64>,
    form: Option<GuestForm>,
    failures: Vec<ServerFailure>,
    saved_ok: bool,
    /// Cadastro salvo nos servidores cujo hub recusou o cookie: volta ao formulário depois da nova entrada, pra o próximo
    /// Salvar só atualizar (sem isso os acessos já criados ficariam órfãos).
    pending: Option<GuestAdmin>,
}

impl Guests {
    /// Aplica o resultado de um Salvar; `true` quando a sessão do hub caiu e a página deve voltar pra entrada.
    fn apply_saved(&mut self, SaveOutcome { saved, errors, expired, .. }: SaveOutcome) -> bool {
        self.failures = errors;
        if expired {
            self.pending = Some(saved);
            return true;
        }
        if let Some(Ok(list)) = self.list.as_mut() {
            list.retain(|g| g.user != saved.user);
            list.push(saved.clone());
        }
        // Com falha parcial o formulário fica aberto e salvar de novo repete só o que faltou.
        if let Some(form) = self.form.as_mut() { form.editing = Some(saved); }
        self.saved_ok = self.failures.is_empty();
        if self.saved_ok { self.form = None; }
        false
    }
}

pub(super) enum GuestsReply {
    LoggedIn(u64, Result<(Session, Result<Vec<GuestAdmin>, Failure>), String>),
    Loaded(u64, Result<Vec<GuestAdmin>, Failure>),
    Saved(u64, SaveOutcome),
    Removed(u64, Vec<ServerFailure>, bool),
    FolderRoots(u64, Result<Vec<Root>, String>),
    FolderScan(u64, Result<Scan, String>),
    /// Caminho da pasta criada.
    FolderMade(u64, Result<String, String>),
}

// Número global: a página zera o estado ao sair, e uma resposta antiga não pode casar com o pedido novo.
fn ticket() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Hangar {
    fn guests_send_later(&self) -> impl Fn(GuestsReply) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>> + Send + 'static {
        let (tx, connection) = (self.tx.clone(), self.connection);
        move |reply| {
            let tx = tx.clone();
            Box::pin(async move {
                let payload = Payload::Sync(super::sync::SyncReply::Guests(reply));
                let _ = tx.send(Envelope { connection, selection: None, payload }).await;
            })
        }
    }

    /// Formulário de entrada do dono, criado quando a página mostra a seção e ainda não há sessão.
    pub(super) fn guests_login_form(&mut self, owner: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let guests = &mut self.sync.guests;
        if guests.login.is_some() || guests.session.is_some() || self.settings != Some(Page::Sync) { return; }
        let owner = owner.unwrap_or_default().to_owned();
        let user = cx.new(|cx| InputState::new(window, cx).default_value(owner));
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let subscriptions = [&user, &password].map(|input| cx.subscribe_in(input, window,
            |this: &mut Hangar, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) { this.guests_login(cx); }
            }));
        guests.login = Some(LoginForm { user, password, _subscriptions: subscriptions.into() });
    }

    fn guests_login(&mut self, cx: &mut Context<Self>) {
        let (Some(api), Some(form)) = (self.api.clone(), self.sync.guests.login.as_ref()) else { return };
        if self.sync.guests.waiting.is_some() { return; }
        let user = form.user.read(cx).value().trim().to_string();
        let password = form.password.read(cx).value().to_string();
        if user.is_empty() { self.sync.guests.login_error = Some(tr_shared("convidados_usuario_obrigatorio", &[])); cx.notify(); return; }
        let seq = ticket();
        (self.sync.guests.waiting, self.sync.guests.login_error) = (Some(seq), None);
        let done = self.guests_send_later();
        self.runtime.spawn(async move {
            let result = match open_session(&api, user, password).await {
                Ok(session) => { let list = load_guests(&api, &session).await; Ok((session, list)) }
                Err(error) => Err(error),
            };
            done(GuestsReply::LoggedIn(seq, result)).await
        });
        cx.notify();
    }

    fn guests_reload(&mut self, cx: &mut Context<Self>) {
        let (Some(api), Some(session)) = (self.api.clone(), self.sync.guests.session.clone()) else { return };
        if self.sync.guests.waiting.is_some() { return; }
        let seq = ticket();
        (self.sync.guests.waiting, self.sync.guests.list) = (Some(seq), None);
        let done = self.guests_send_later();
        self.runtime.spawn(async move { done(GuestsReply::Loaded(seq, load_guests(&api, &session).await)).await });
        cx.notify();
    }

    fn guests_open(&mut self, guest: Option<GuestAdmin>, window: &mut Window, cx: &mut Context<Self>) {
        let input = |value: String, masked: bool, window: &mut Window, cx: &mut Context<Self>|
            cx.new(|cx| InputState::new(window, cx).masked(masked).default_value(value));
        let user = input(guest.as_ref().map(|g| g.user.clone()).unwrap_or_default(), false, window, cx);
        let password = input(guest.as_ref().map(|g| g.password.clone()).unwrap_or_default(), true, window, cx);
        let roots = guest.iter().flat_map(|g| &g.servers).map(|s| (s.server_id.clone(), s.root.clone())).collect();
        let guests = &mut self.sync.guests;
        guests.form = Some(GuestForm { sees_owner: guest.as_ref().is_some_and(|g| g.sees_owner),
            owner_sees: guest.as_ref().is_none_or(|g| g.owner_sees), editing: guest, user, password, roots, browser: None, error: None });
        (guests.failures, guests.saved_ok) = (Vec::new(), false);
        cx.notify();
    }

    fn guests_toggle(&mut self, id: String, on: bool, cx: &mut Context<Self>) {
        let Some(form) = self.sync.guests.form.as_mut() else { return };
        if !on {
            form.roots.retain(|(server, _)| *server != id);
            if form.browser.as_ref().is_some_and(|b| b.server == id) { form.browser = None; }
        } else if !form.roots.iter().any(|(server, _)| *server == id) {
            form.roots.push((id.clone(), String::new()));
            // Marcar já abre as pastas dele: sem pasta o servidor não salva.
            self.guests_browse(id, cx);
        }
        cx.notify();
    }

    fn guests_browse(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(form) = self.sync.guests.form.as_mut() else { return };
        form.browser = Some(FolderBrowser::new(id));
        self.guests_load_roots(cx);
    }

    fn guests_load_roots(&mut self, cx: &mut Context<Self>) {
        let done = self.guests_send_later();
        let guests = &mut self.sync.guests;
        let Some(browser) = guests.form.as_mut().and_then(|f| f.browser.as_mut()) else { return };
        let seq = begin(&mut browser.roots);
        match folder_api(self.api.as_ref(), guests.session.as_ref(), &browser.server) {
            Err(error) => { browser.roots.finish(seq, Err(error)); }
            Ok(api) => { self.runtime.spawn(async move {
                let roots = api.server_read(&["fs", "roots"], &[], 15).await.map_err(|e| Hangar::fetch_failure(&e))
                    .and_then(|v| serde_json::from_value::<Vec<Root>>(v).map_err(|_| tr("invalid_response")));
                done(GuestsReply::FolderRoots(seq, roots)).await
            }); }
        }
        cx.notify();
    }

    fn guests_pick_root(&mut self, root: Root, cx: &mut Context<Self>) {
        let Some(browser) = self.sync.guests.form.as_mut().and_then(|f| f.browser.as_mut()) else { return };
        let path = root.path.clone();
        browser.root = Some(root);
        self.guests_scan(path, cx);
    }

    fn guests_scan(&mut self, path: String, cx: &mut Context<Self>) {
        let done = self.guests_send_later();
        let guests = &mut self.sync.guests;
        let Some(browser) = guests.form.as_mut().and_then(|f| f.browser.as_mut()) else { return };
        let Some(root) = browser.root.as_ref().map(|r| r.path.clone()) else { return };
        (browser.dir, browser.make_error) = (path.clone(), None);
        let seq = begin(&mut browser.scan);
        match folder_api(self.api.as_ref(), guests.session.as_ref(), &browser.server) {
            Err(error) => { browser.scan.finish(seq, Err(error)); }
            Ok(api) => { self.runtime.spawn(async move {
                let query: Vec<(&str, &str)> = if path == root { vec![("root", root.as_str())] } else { vec![("root", root.as_str()), ("path", path.as_str())] };
                done(GuestsReply::FolderScan(seq, scan_of(api.server_read(&["fs", "scan"], &query, 15).await))).await
            }); }
        }
        cx.notify();
    }

    fn guests_pick_folder(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(form) = self.sync.guests.form.as_mut() else { return };
        let Some(browser) = form.browser.take() else { return };
        if let Some((_, root)) = form.roots.iter_mut().find(|(id, _)| *id == browser.server) { *root = path; }
        cx.notify();
    }

    fn guests_new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx));
        let subscription = cx.subscribe_in(&input, window, |this: &mut Hangar, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) { this.guests_make_folder(cx); }
        });
        input.update(cx, |state, cx| state.focus(window, cx));
        let Some(browser) = self.sync.guests.form.as_mut().and_then(|f| f.browser.as_mut()) else { return };
        (browser.naming, browser.make_error) = (Some((input, subscription)), None);
        cx.notify();
    }

    fn guests_make_folder(&mut self, cx: &mut Context<Self>) {
        let done = self.guests_send_later();
        let guests = &mut self.sync.guests;
        let Some(browser) = guests.form.as_mut().and_then(|f| f.browser.as_mut()) else { return };
        let (Some((input, _)), Some(root), None) = (browser.naming.as_ref(), browser.root.as_ref(), browser.making) else { return };
        let name = input.read(cx).value().trim().to_string();
        if name.is_empty() { return; }
        let root = root.path.clone();
        let path = (browser.dir != root).then(|| browser.dir.clone());
        let failed = |error: &str| tr_shared("arquivo_criar_pasta_erro", &[("erro", error)]);
        match folder_api(self.api.as_ref(), guests.session.as_ref(), &browser.server) {
            Err(error) => browser.make_error = Some(failed(&error)),
            Ok(api) => {
                let seq = ticket();
                (browser.making, browser.make_error) = (Some(seq), None);
                self.runtime.spawn(async move {
                    let body = json!({"root": root, "path": path, "name": name});
                    let made = api.server_send(reqwest::Method::POST, &["fs", "mkdir"], Some(body), 15).await
                        .map_err(|e| Hangar::fetch_failure(&e))
                        .and_then(|v| v.get("path").and_then(Value::as_str).map(str::to_owned).ok_or_else(|| tr("invalid_response")));
                    done(GuestsReply::FolderMade(seq, made)).await
                });
            }
        }
        cx.notify();
    }

    /// Respostas do seletor de pasta: não passam pelo `waiting`, que é do Salvar e da lista.
    fn receive_folder(&mut self, reply: GuestsReply, cx: &mut Context<Self>) {
        let Some(form) = self.sync.guests.form.as_mut() else { return };
        let Some(browser) = form.browser.as_mut() else { return };
        match reply {
            GuestsReply::FolderRoots(seq, result) => {
                if !browser.roots.finish(seq, result) { return; }
                let chosen = form.roots.iter().find(|(id, _)| *id == browser.server).map(|(_, p)| p.as_str()).unwrap_or_default();
                if let Some((root, dir)) = start_at(browser.roots.ok().map_or(&[][..], Vec::as_slice), chosen) {
                    browser.root = Some(root);
                    self.guests_scan(dir, cx);
                }
            }
            GuestsReply::FolderScan(seq, result) => { browser.scan.finish(seq, result); }
            GuestsReply::FolderMade(seq, result) => {
                if browser.making != Some(seq) { return; }
                browser.making = None;
                match result {
                    // Entra na pasta nova: "Usar esta pasta" já a escolhe.
                    Ok(path) => { browser.naming = None; self.guests_scan(path, cx); }
                    Err(error) => browser.make_error = Some(tr_shared("arquivo_criar_pasta_erro", &[("erro", &error)])),
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn guests_submit(&mut self, cx: &mut Context<Self>) {
        let (Some(api), Some(session)) = (self.api.clone(), self.sync.guests.session.clone()) else { return };
        let guests = &mut self.sync.guests;
        let Some(form) = guests.form.as_mut() else { return };
        if guests.waiting.is_some() { return; }
        let draft = Draft { user: form.user.read(cx).value().trim().to_string(), password: form.password.read(cx).value().to_string(),
            sees_owner: form.sees_owner, owner_sees: form.owner_sees,
            servers: form.roots.clone() };
        let list = guests.list.as_ref().and_then(|l| l.as_ref().ok()).map_or(&[][..], Vec::as_slice);
        form.error = validate(&draft, form.editing.is_some(), &session.user, list).map(|key| tr_shared(key, &[]));
        if form.error.is_some() { cx.notify(); return; }
        let prev = form.editing.clone();
        let seq = ticket();
        guests.waiting = Some(seq);
        let done = self.guests_send_later();
        self.runtime.spawn(async move {
            let live = Live { hub: api, session };
            let outcome = save_guest(&live, &live.session.servers, &draft, prev.as_ref()).await;
            done(GuestsReply::Saved(seq, outcome)).await
        });
        cx.notify();
    }

    fn guests_confirm_remove(&mut self, guest: GuestAdmin, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        let text = tr_shared("convidados_remover_aviso", &[("usuario", &guest.user)]);
        chrome::confirm_alert(window, cx, tr_shared("convidados_remover", &[]), text, tr_shared("convidados_remover", &[]),
            ButtonVariant::Danger, move |_, cx| {
                let guest = guest.clone();
                let _ = this.update(cx, |this, cx| this.guests_remove(guest, cx));
                true
            });
    }

    fn guests_remove(&mut self, guest: GuestAdmin, cx: &mut Context<Self>) {
        let (Some(api), Some(session)) = (self.api.clone(), self.sync.guests.session.clone()) else { return };
        if self.sync.guests.waiting.is_some() { return; }
        let seq = ticket();
        (self.sync.guests.waiting, self.sync.guests.saved_ok) = (Some(seq), false);
        let done = self.guests_send_later();
        self.runtime.spawn(async move {
            let live = Live { hub: api, session };
            let (errors, expired) = remove_guest(&live, &live.session.servers, &guest).await;
            done(GuestsReply::Removed(seq, errors, expired)).await
        });
        cx.notify();
    }

    /// Lista lida: 401 é a sessão do hub que caiu e 403 é usuário que não é o dono; os dois voltam pra entrada.
    fn guests_loaded(&mut self, result: Result<Vec<GuestAdmin>, Failure>, window: &mut Window, cx: &mut Context<Self>) {
        let guests = &mut self.sync.guests;
        match result {
            Err(error) if matches!(error.status, Some(401 | 403)) => {
                self.guests_expired(window, cx);
                self.sync.guests.login_error = (error.status == Some(403)).then(|| tr_shared("erro_so_dono", &[]));
            }
            result => guests.list = Some(result.map_err(|error| describe(&error))),
        }
    }

    pub(super) fn receive_guests(&mut self, reply: GuestsReply, window: &mut Window, cx: &mut Context<Self>) {
        let seq = match &reply {
            GuestsReply::LoggedIn(seq, _) | GuestsReply::Loaded(seq, _) | GuestsReply::Saved(seq, _) | GuestsReply::Removed(seq, ..) => *seq,
            GuestsReply::FolderRoots(..) | GuestsReply::FolderScan(..) | GuestsReply::FolderMade(..) => return self.receive_folder(reply, cx),
        };
        if self.sync.guests.waiting != Some(seq) { return; }
        self.sync.guests.waiting = None;
        match reply {
            GuestsReply::LoggedIn(_, Err(error)) => self.sync.guests.login_error = Some(error),
            GuestsReply::LoggedIn(_, Ok((session, list))) => {
                let guests = &mut self.sync.guests;
                (guests.session, guests.login, guests.login_error) = (Some(session), None, None);
                self.guests_loaded(list, window, cx);
                if self.sync.guests.session.is_some() && let Some(pending) = self.sync.guests.pending.take() {
                    let failures = std::mem::take(&mut self.sync.guests.failures);
                    self.guests_open(Some(pending), window, cx);
                    self.sync.guests.failures = failures;
                }
            }
            GuestsReply::Loaded(_, list) => self.guests_loaded(list, window, cx),
            // Cookie vencido: sem voltar pra entrada, salvar falharia pra sempre até sair da página.
            GuestsReply::Saved(_, outcome) => if self.sync.guests.apply_saved(outcome) { self.guests_expired(window, cx); },
            GuestsReply::Removed(_, errors, true) => {
                self.guests_expired(window, cx);
                self.sync.guests.failures = errors;
            }
            GuestsReply::Removed(_, errors, _) => {
                self.sync.guests.failures = errors;
                self.guests_reload(cx);
            }
            GuestsReply::FolderRoots(..) | GuestsReply::FolderScan(..) | GuestsReply::FolderMade(..) => {}
        }
        cx.notify();
    }

    fn guests_expired(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let guests = &mut self.sync.guests;
        (guests.session, guests.list, guests.form, guests.saved_ok) = (None, None, None, false);
        let owner = self.sync.owner();
        self.guests_login_form(owner.as_deref(), window, cx);
    }

    /// A pasta de um servidor marcado: a escolhida (ou o aviso de que falta) e o seletor, quando aberto.
    fn render_guest_folder(&self, server: &OwnerServer, path: &str, browser: Option<&FolderBrowser>, busy: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let id = server.id.clone();
        let label = tr_shared("convidados_pasta", &[("servidor", &server.label)]);
        let summary = div().flex().items_center().gap_2()
            .child(div().flex_1().min_w_0().text_sm().truncate().map(|el| if path.is_empty() {
                el.text_color(theme::muted()).child(tr_shared("convidados_pasta_nenhuma", &[]))
            } else { el.font_family(theme::MONO).child(path.to_owned()) }))
            .when(browser.is_none(), |el| el.child(Button::new(SharedString::from(format!("guests-browse-{}", server.id))).outline().small()
                .icon(IconName::FolderOpen).disabled(busy)
                .label(tr_shared(if path.is_empty() { "convidados_escolher_pasta" } else { "convidados_trocar_pasta" }, &[]))
                .on_click(cx.listener(move |this, _, _, cx| this.guests_browse(id.clone(), cx)))));
        div().id(SharedString::from(format!("guests-folder-{}", server.id))).role(Role::Group).aria_label(label.clone())
            .ml(px(24.)).flex().flex_col().gap(px(6.))
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(label))
            .child(summary)
            .when_some(browser, |el, browser| el.child(Self::render_folder_browser(browser, path, busy, cx)))
    }

    fn render_folder_browser(browser: &FolderBrowser, chosen: &str, busy: bool, cx: &mut Context<Self>) -> Div {
        let muted = |text: String| div().text_sm().text_color(theme::muted()).whitespace_normal().child(text);
        let loading = |id: &'static str| div().id(id).role(Role::Status).text_sm().text_color(theme::muted()).child(tr_shared("comum_carregando", &[]));
        let alert = |id: &'static str, text: String| div().id(id).role(Role::Alert).text_sm().text_color(theme::danger()).whitespace_normal().child(text);
        let retry = |id: &'static str| Button::new(id).outline().small().label(tr_shared("lista_tentar_novamente", &[])).disabled(busy);
        let frame = div().flex().flex_col().gap_2().p_2().rounded(px(8.)).border_1().border_color(theme::border());
        // Fechar no meio do mkdir descartaria a resposta: a pasta nasceria sem ninguém saber.
        let close = Button::new("guests-browse-close").ghost().small().label(tr_shared("comum_cancelar", &[])).disabled(busy || browser.making.is_some())
            .on_click(cx.listener(|this, _, _, cx| { if let Some(form) = this.sync.guests.form.as_mut() { form.browser = None; } cx.notify(); }));
        let roots = match browser.roots.value.as_ref() {
            _ if browser.roots.loading => return frame.child(loading("guests-roots-loading")).child(div().child(close)),
            None => return frame.child(div().child(close)),
            Some(Err(error)) => return frame.child(alert("guests-roots-error", format!("{} {error}", tr_shared("arquivo_carregar_raizes_erro", &[]))))
                .child(div().flex().gap_2().child(retry("guests-roots-retry").on_click(cx.listener(|this, _, _, cx| this.guests_load_roots(cx)))).child(close)),
            Some(Ok(list)) if list.is_empty() => return frame.child(muted(tr_shared("arquivo_sem_raizes", &[]))).child(div().child(close)),
            Some(Ok(list)) => list,
        };
        let Some(root) = browser.root.as_ref() else { return frame.child(div().child(close)) };
        let chips = div().id("guests-roots").role(Role::Group).aria_label(tr_shared("arquivo_raizes_aria", &[])).flex().flex_wrap().gap(px(6.))
            .children(roots.iter().map(|r| {
                let pick = r.clone();
                choice(SharedString::from(format!("guests-root-{}", r.path)), r.path == root.path, cx).small().rounded_full()
                    .label(r.name.clone()).tooltip(r.path.clone()).disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| this.guests_pick_root(pick.clone(), cx)))
            }));
        let trail = div().id("guests-crumbs").aria_label(tr_shared("arquivo_caminho_aria", &[])).flex().flex_wrap().items_center().gap(px(2.))
            .children(crumbs(root, &browser.dir).into_iter().enumerate().map(|(n, (text, path))| div().flex().items_center().gap(px(2.))
                .when(n > 0, |el| el.child(div().text_color(theme::faint()).text_size(px(12.)).child("/")))
                .child(Button::new(SharedString::from(format!("guests-crumb-{path}"))).ghost().xsmall().label(text).disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| this.guests_scan(path.clone(), cx))))));
        let rows = match browser.scan.value.as_ref() {
            _ if browser.scan.loading => loading("guests-scan-loading").into_any_element(),
            None => div().into_any_element(),
            Some(Err(error)) => {
                let dir = browser.dir.clone();
                div().flex().flex_col().gap_2().child(alert("guests-scan-error", error.clone()))
                    .child(div().child(retry("guests-scan-retry").on_click(cx.listener(move |this, _, _, cx| this.guests_scan(dir.clone(), cx)))))
                    .into_any_element()
            }
            Some(Ok(scan)) if scan.error.is_some() => muted(scan.error.clone().unwrap_or_default()).into_any_element(),
            Some(Ok(scan)) if scan.entries.is_empty() => muted(tr_shared("arquivo_sem_subpastas", &[])).into_any_element(),
            // ponytail: lista simples com rolagem; pasta com milhares de subpastas pede a lista virtual do `create.rs`.
            Some(Ok(scan)) => div().id("guests-folders").max_h(px(240.)).overflow_y_scroll().flex().flex_col().gap(px(2.))
                .children(scan.entries.iter().map(|entry| {
                    let (pick, open) = (entry.path.clone(), entry.path.clone());
                    let on = entry.path == chosen;
                    div().flex().items_center().gap(px(4.))
                        .child(choice(SharedString::from(format!("guests-pick-{}", entry.path)), on, cx).small().flex_1().min_w_0().justify_start()
                            .icon(IconName::Folder).label(entry.name.clone()).tooltip(entry.path.clone()).selected(on).disabled(busy)
                            .on_click(cx.listener(move |this, _, _, cx| this.guests_pick_folder(pick.clone(), cx))))
                        .child(Button::new(SharedString::from(format!("guests-open-{}", entry.path))).ghost().small().icon(IconName::ChevronRight)
                            .accessibility_label(tr_shared("arquivo_abrir", &[("nome", &entry.name)])).disabled(busy)
                            .on_click(cx.listener(move |this, _, _, cx| this.guests_scan(open.clone(), cx))))
                })).into_any_element(),
        };
        let making = browser.making.is_some();
        let naming = browser.naming.as_ref().map(|(input, _)| {
            let ready = !input.read(cx).value().trim().is_empty();
            div().flex().items_center().gap_2()
                .child(div().flex_1().min_w_0().child(Input::new(input).small().disabled(making || busy).aria_label(tr_shared("arquivo_nova_pasta_nome", &[]))))
                .child(Button::new("guests-mkdir").primary().small().label(tr_shared("arquivo_criar_pasta", &[])).loading(making)
                    .disabled(!ready || making || busy).on_click(cx.listener(|this, _, _, cx| this.guests_make_folder(cx))))
                .child(Button::new("guests-mkdir-cancel").ghost().small().label(tr_shared("comum_cancelar", &[])).disabled(making)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(b) = this.sync.guests.form.as_mut().and_then(|f| f.browser.as_mut()) { (b.naming, b.make_error) = (None, None); }
                        cx.notify();
                    })))
        });
        let dir = browser.dir.clone();
        // Durante a leitura `dir` já é a pasta nova e `scan` ainda é a anterior.
        let readable = !browser.scan.loading && browser.scan.ok().is_some_and(|scan| scan.error.is_none());
        frame.child(chips).child(trail).child(rows)
            .children(naming)
            .when_some(browser.make_error.clone(), |el, error| el.child(alert("guests-mkdir-error", error)))
            .child(div().flex().flex_wrap().items_center().gap_2()
                .child(Button::new("guests-use-folder").outline().small().icon(IconName::FolderOpen).label(tr_shared("arquivo_usar_pasta", &[]))
                    .disabled(busy || !readable).on_click(cx.listener(move |this, _, _, cx| this.guests_pick_folder(dir.clone(), cx))))
                .when(browser.naming.is_none(), |el| el.child(Button::new("guests-new-folder").ghost().small().icon(IconName::Plus)
                    .label(tr_shared("arquivo_nova_pasta", &[])).disabled(busy || !readable)
                    .on_click(cx.listener(|this, _, window, cx| this.guests_new_folder(window, cx)))))
                .child(div().flex_1())
                .child(close))
    }

    pub(super) fn render_guests(&mut self, cx: &mut Context<Self>) -> Div {
        let guests = &self.sync.guests;
        let busy = guests.waiting.is_some();
        let mut section = div().flex().flex_col().gap_3().pt_2()
            .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(tr_shared("convidados_titulo", &[])))
            .child(div().text_sm().text_color(theme::muted()).whitespace_normal().child(tr_shared("convidados_descricao", &[])));
        let alert = |id: SharedString, text: String| div().id(id).role(Role::Alert).text_sm().text_color(theme::danger()).whitespace_normal().child(text);
        // Antes da entrada também: a queda da sessão no meio de um Salvar não pode sumir calada.
        for (n, failure) in guests.failures.iter().enumerate() {
            section = section.child(alert(SharedString::from(format!("guests-failure-{n}")),
                tr_shared("convidados_falha_servidor", &[("servidor", &failure.label), ("erro", &failure.message)])));
        }
        let Some(session) = &guests.session else {
            let Some(form) = &guests.login else { return section };
            return section.child(div().text_sm().text_color(theme::muted()).whitespace_normal().child(tr_shared("convidados_entrar_nativo", &[])))
                .child(sync_field(tr("login_usuario"), &form.user, busy))
                .child(sync_field(tr("login_senha"), &form.password, busy))
                .when_some(guests.login_error.clone(), |el, error| el.child(alert("guests-login-error".into(), error)))
                .child(Button::new("guests-login").primary().small().label(tr_shared("login_entrar", &[])).disabled(busy).loading(busy)
                    .on_click(cx.listener(|this, _, _, cx| this.guests_login(cx))));
        };
        if guests.saved_ok {
            section = section.child(div().id("guests-saved").role(Role::Status).text_sm().font_weight(FontWeight::SEMIBOLD).child(tr_shared("convidados_salvo", &[])));
        }
        let list = match &guests.list {
            None => return section.child(div().id("guests-loading").role(Role::Status).text_sm().text_color(theme::muted()).child(tr_shared("comum_carregando", &[]))),
            Some(Err(error)) => return section.child(alert("guests-load-error".into(), tr_shared("convidados_erro", &[("erro", error)])))
                .child(Button::new("guests-retry").outline().small().label(tr_shared("lista_tentar_novamente", &[]))
                    .on_click(cx.listener(|this, _, _, cx| this.guests_reload(cx)))),
            Some(Ok(list)) => list,
        };
        if list.is_empty() {
            section = section.child(div().text_sm().text_color(theme::muted()).child(tr_shared("convidados_vazio", &[])));
        }
        for guest in list {
            let (edit, remove) = (guest.clone(), guest.clone());
            let count = guest.servers.len().to_string();
            section = section.child(div().flex().items_center().gap_3().px_3().py_2().rounded(px(8.)).border_1().border_color(theme::border())
                .child(div().font_weight(FontWeight::SEMIBOLD).truncate().child(guest.user.clone()))
                .child(div().flex_1().min_w_0().text_sm().text_color(theme::muted()).truncate()
                    .child(tr_shared("convidados_resumo", &[("servidores", &count)])))
                .child(Button::new(SharedString::from(format!("guests-edit-{}", guest.user))).outline().small()
                    .label(tr_shared("convidados_editar", &[])).disabled(busy)
                    .on_click(cx.listener(move |this, _, window, cx| this.guests_open(Some(edit.clone()), window, cx))))
                .child(Button::new(SharedString::from(format!("guests-remove-{}", guest.user))).outline().small()
                    .label(tr_shared("convidados_remover", &[])).disabled(busy)
                    .on_click(cx.listener(move |this, _, window, cx| this.guests_confirm_remove(remove.clone(), window, cx)))));
        }
        let Some(form) = &guests.form else {
            return section.child(Button::new("guests-add").outline().small().label(tr_shared("convidados_adicionar", &[])).disabled(busy)
                .on_click(cx.listener(|this, _, window, cx| this.guests_open(None, window, cx))));
        };
        let mut servers = div().flex().flex_col().gap_2().p_3().rounded(px(8.)).border_1().border_color(theme::border())
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(tr_shared("convidados_servidores", &[])));
        for server in &session.servers {
            let root = form.roots.iter().find(|(id, _)| *id == server.id).map(|(_, path)| path.as_str());
            let id = server.id.clone();
            let browser = form.browser.as_ref().filter(|b| b.server == server.id);
            servers = servers.child(Checkbox::new(SharedString::from(format!("guests-server-{}", server.id))).small()
                .label(server.label.clone()).checked(root.is_some()).disabled(busy)
                .on_click(cx.listener(move |this, on: &bool, _, cx| this.guests_toggle(id.clone(), *on, cx))))
                .when_some(root, |el, path| el.child(self.render_guest_folder(server, path, browser, busy, cx)));
        }
        section.child(div().flex().flex_col().gap_3()
            .child(sync_field(tr("login_usuario"), &form.user, busy || form.editing.is_some()))
            .child(sync_field(tr("login_senha"), &form.password, busy))
            .child(Checkbox::new("guests-sees-owner").small().label(tr_shared("convidados_ve_minhas", &[])).checked(form.sees_owner).disabled(busy)
                .on_click(cx.listener(|this, on: &bool, _, cx| {
                    if let Some(form) = this.sync.guests.form.as_mut() { form.sees_owner = *on; }
                    cx.notify();
                })))
            .child(Checkbox::new("guests-owner-sees").small().label(tr_shared("convidados_eu_vejo", &[])).checked(form.owner_sees).disabled(busy)
                .on_click(cx.listener(|this, on: &bool, _, cx| {
                    if let Some(form) = this.sync.guests.form.as_mut() { form.owner_sees = *on; }
                    cx.notify();
                })))
            .child(servers)
            .when_some(form.error.clone(), |el, error| el.child(alert("guests-form-error".into(), error)))
            .child(div().flex().gap_2()
                .child(Button::new("guests-save").primary().small().disabled(busy).loading(busy)
                    .label(tr_shared(if busy { "convidados_salvando" } else { "convidados_salvar" }, &[]))
                    .on_click(cx.listener(|this, _, _, cx| this.guests_submit(cx))))
                .child(Button::new("guests-cancel").outline().small().label(tr_shared("comum_cancelar", &[])).disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| { this.sync.guests.form = None; cx.notify(); })))))
    }
}

#[cfg(test)]
mod tests {
    // Sem glob: o `test` do gpui_kit, que o `super::*` traz, esconderia o `#[test]` da linguagem.
    use super::{Draft, Failure, GuestAdmin, GuestOps, GuestServer, Guests, OwnerServer, SaveOutcome, ServerFailure, STANDARD, decrypt_json, derive_keys, encrypt_json,
        Root, remove_guest, save_guest, start_at, validate};
    use base64::Engine as _;
    use crate::i18n::tr_shared;
    use serde_json::{Value, json};
    use std::cell::RefCell;

    #[test]
    fn keys_match_the_web_derivation() {
        // Vetor calculado com o `deriveKeys` do web (WebCrypto no Node) pra mesma senha, sal e voltas.
        let keys = derive_keys("senha-teste", b"0123456789abcdef", 1000).unwrap();
        assert_eq!(keys.auth_hash, WEB_AUTH_HASH);
        let blob = encrypt_json(&keys.enc, &json!({"a": 1})).unwrap();
        assert_eq!(decrypt_json::<Value>(&keys.enc, &blob).unwrap(), json!({"a": 1}));
        assert_eq!(STANDARD.decode(blob["iv"].as_str().unwrap()).unwrap().len(), 12);
        // Cifrado pelo web com a chave `cp-enc` desta senha.
        assert_eq!(decrypt_json::<Value>(&keys.enc, &json!({"iv": WEB_IV, "data": WEB_DATA})).unwrap(), json!({"web": true}));
    }

    const WEB_AUTH_HASH: &str = "fewlsDVCTvVcrbrLvNFlw10sAtj5G+YyqW3dt+XxYeg=";
    const WEB_IV: &str = "JQty6M7zR3xv37QM";
    const WEB_DATA: &str = "Uud7Id6F3q4oKV0LhFLyMIYDqTe3tsd8pZ5QPQ==";

    #[test]
    fn admin_blob_keeps_the_web_shape() {
        let web = json!({"user": "ana", "password": "12345678", "seesOwner": false, "ownerSees": true,
            "servers": [{"serverId": "srv-1", "guestId": "g1", "token": "t", "root": "/home/ana"}]});
        let admin: GuestAdmin = serde_json::from_value(web.clone()).unwrap();
        assert_eq!(serde_json::to_value(&admin).unwrap(), web);
    }

    fn draft(user: &str, password: &str, servers: &[(&str, &str)]) -> Draft {
        Draft { user: user.into(), password: password.into(), sees_owner: false, owner_sees: true,
            servers: servers.iter().map(|(a, b)| ((*a).into(), (*b).into())).collect() }
    }

    #[test]
    fn validation_follows_the_web_order() {
        let existing = [GuestAdmin { user: "bia".into(), password: "x".into(), sees_owner: false, owner_sees: true, servers: vec![] }];
        let check = |d: Draft, editing| validate(&d, editing, "dono", &existing);
        assert_eq!(check(draft("", "12345678", &[("s", "/")]), false), Some("convidados_usuario_obrigatorio"));
        assert_eq!(check(draft("a/b", "12345678", &[("s", "/")]), false), Some("convidados_usuario_barra"));
        assert_eq!(check(draft("dono", "12345678", &[("s", "/")]), false), Some("convidados_usuario_em_uso"));
        assert_eq!(check(draft("bia", "12345678", &[("s", "/")]), false), Some("convidados_usuario_em_uso"));
        assert_eq!(check(draft("bia", "12345678", &[("s", "/")]), true), None);
        assert_eq!(check(draft("ana", "1234567", &[("s", "/")]), false), Some("sync_password_min"));
        assert_eq!(check(draft("ana", "12345678", &[]), false), Some("convidados_servidor_obrigatorio"));
        assert_eq!(check(draft("ana", "12345678", &[("s", "")]), false), Some("convidados_pasta_obrigatoria"));
    }

    /// Servidores de mentira: `fail` diz que status cada operação devolve por servidor.
    #[derive(Default)]
    struct Fake { fail: Vec<(&'static str, &'static str, u16)>, hub_fails: Option<u16>, calls: RefCell<Vec<String>>, hub: RefCell<Option<Value>> }

    impl Fake {
        fn run(&self, op: &str, server: &str) -> Result<(), Failure> {
            self.calls.borrow_mut().push(format!("{op} {server}"));
            match self.fail.iter().find(|(o, s, _)| *o == op && *s == server) {
                Some((_, _, status)) => Err(Failure { status: Some(*status), detail: "boom".into(), retry_after: None, uncertain: false, code: None }),
                None => Ok(()),
            }
        }
    }

    impl GuestOps for Fake {
        async fn create(&self, server: &OwnerServer, _: &Draft, _: &str) -> Result<(String, String), Failure> {
            self.run("create", &server.id).map(|_| (format!("new-{}", server.id), format!("tok-{}", server.id)))
        }
        async fn update(&self, server: &OwnerServer, _: &str, _: &Draft, _: &str) -> Result<(), Failure> { self.run("update", &server.id) }
        async fn delete(&self, server: &OwnerServer, _: &str) -> Result<(), Failure> { self.run("delete", &server.id) }
        async fn put_hub(&self, _: &GuestAdmin, servers: Value) -> Result<(), Failure> {
            *self.hub.borrow_mut() = Some(servers);
            match self.hub_fails {
                Some(status) => Err(Failure { status: Some(status), detail: "boom".into(), retry_after: None, uncertain: false, code: None }),
                None => Ok(()),
            }
        }
        async fn delete_hub(&self, user: &str) -> Result<(), Failure> { self.run("delete_hub", user) }
    }

    fn owner() -> Vec<OwnerServer> {
        serde_json::from_value(json!([{"id": "a", "label": "A", "baseUrl": "", "token": "ta"},
            {"id": "b", "label": "B", "baseUrl": "https://b", "token": "tb"}])).unwrap()
    }

    fn entry(id: &str, root: &str) -> GuestServer {
        GuestServer { server_id: id.into(), guest_id: format!("old-{id}"), token: format!("old-tok-{id}"), root: root.into() }
    }

    fn prev(servers: Vec<GuestServer>) -> GuestAdmin {
        GuestAdmin { user: "ana".into(), password: "12345678".into(), sees_owner: false, owner_sees: true, servers }
    }

    #[test]
    fn save_creates_updates_recreates_and_reports() {
        let fake = Fake { fail: vec![("update", "a", 404), ("create", "b", 500)], ..Fake::default() };
        let old = prev(vec![entry("a", "/old"), entry("b", "/b")]);
        // b já existia: o update dele passa; a sumiu no servidor e é recriado.
        let out = futures::executor::block_on(save_guest(&fake, &owner(), &draft("ana", "12345678", &[("a", "/new"), ("b", "/b2")]), Some(&old)));
        assert_eq!(out.errors, vec![]);
        assert_eq!(out.saved.servers, vec![GuestServer { server_id: "a".into(), guest_id: "new-a".into(), token: "tok-a".into(), root: "/new".into() },
            GuestServer { root: "/b2".into(), ..entry("b", "/b") }]);
        // O endereço vazio do hub vai como está.
        assert_eq!(fake.hub.borrow().clone().unwrap(), json!([{"id": "a", "label": "A", "baseUrl": "", "token": "tok-a"},
            {"id": "b", "label": "B", "baseUrl": "https://b", "token": "old-tok-b"}]));
        // Novo em b falha: erro com o rótulo do servidor e nada guardado dele.
        let out = futures::executor::block_on(save_guest(&fake, &owner(), &draft("ana", "12345678", &[("b", "/b")]), None));
        assert_eq!(out.errors.len(), 1);
        assert_eq!(out.errors[0].label, "B");
        assert!(out.saved.servers.is_empty());
    }

    #[test]
    fn save_keeps_entries_it_could_not_touch() {
        let fake = Fake { fail: vec![("delete", "b", 500)], hub_fails: Some(500), ..Fake::default() };
        let old = prev(vec![entry("a", "/a"), entry("b", "/b"), entry("gone", "/g")]);
        let out = futures::executor::block_on(save_guest(&fake, &owner(), &draft("ana", "12345678", &[]), Some(&old)));
        // a removido; b falhou e fica; "gone" saiu da lista do dono e fica; o hub falhou e vira erro, sem estourar.
        assert_eq!(out.saved.servers, vec![entry("b", "/b"), entry("gone", "/g")]);
        assert!(out.hub_failed && !out.expired);
        assert_eq!(out.errors.iter().map(|e| e.label.as_str()).collect::<Vec<_>>(), ["B", "gone", tr_shared("sync_config_titulo", &[]).as_str()]);
    }

    #[test]
    fn remove_deletes_the_hub_account_only_when_everything_let_go() {
        let fake = Fake { fail: vec![("delete", "a", 404)], ..Fake::default() };
        let (errors, expired) = futures::executor::block_on(remove_guest(&fake, &owner(), &prev(vec![entry("a", "/a")])));
        assert!(errors.is_empty() && !expired);
        assert!(fake.calls.borrow().contains(&"delete_hub ana".to_string()));
        let fake = Fake { fail: vec![("delete", "a", 500)], ..Fake::default() };
        let (errors, _) = futures::executor::block_on(remove_guest(&fake, &owner(), &prev(vec![entry("a", "/a")])));
        assert_eq!(errors.len(), 1);
        assert!(!fake.calls.borrow().contains(&"delete_hub ana".to_string()));
    }

    #[test]
    fn hub_refusing_the_cookie_marks_the_session_expired() {
        // Sem isso o formulário ficaria aberto e cada Salvar bateria no mesmo 401.
        let fake = Fake { hub_fails: Some(401), ..Fake::default() };
        let out = futures::executor::block_on(save_guest(&fake, &owner(), &draft("ana", "12345678", &[("a", "/a")]), None));
        assert!(out.hub_failed && out.expired);
        let (_, expired) = futures::executor::block_on(remove_guest(&fake, &owner(), &prev(vec![entry("a", "/a")])));
        assert!(expired);
    }

    #[test]
    fn expired_save_keeps_the_record_for_after_login() {
        let saved = prev(vec![entry("a", "/a")]);
        let failure = ServerFailure { label: "hub".into(), message: "401".into() };
        let mut guests = Guests::default();
        let outcome = SaveOutcome { saved: saved.clone(), errors: vec![failure.clone()], hub_failed: true, expired: true };
        assert!(guests.apply_saved(outcome));
        assert_eq!(guests.pending, Some(saved.clone()));
        assert_eq!(guests.failures, vec![failure]);
        let mut guests = Guests { list: Some(Ok(vec![])), ..Guests::default() };
        assert!(!guests.apply_saved(SaveOutcome { saved: saved.clone(), errors: vec![], hub_failed: false, expired: false }));
        assert!(guests.pending.is_none() && guests.saved_ok);
        assert_eq!(guests.list, Some(Ok(vec![saved])));
    }

    #[test]
    fn browser_opens_at_the_saved_folder() {
        let root = |path: &str| Root { name: path.into(), path: path.into() };
        let roots = [root("/home/ana"), root("/home/ana/pessoal"), root("/srv")];
        let at = |chosen: &str| start_at(&roots, chosen).map(|(r, dir)| (r.path, dir));
        // A raiz mais funda que contém a pasta; "/home/anabela" não está dentro de "/home/ana".
        assert_eq!(at("/home/ana/pessoal/hangar"), Some(("/home/ana/pessoal".into(), "/home/ana/pessoal/hangar".into())));
        assert_eq!(at("/srv"), Some(("/srv".into(), "/srv".into())));
        assert_eq!(at("/home/anabela"), Some(("/home/ana".into(), "/home/ana".into())));
        assert_eq!(at(""), Some(("/home/ana".into(), "/home/ana".into())));
        assert!(start_at(&[], "/x").is_none());
    }

    #[test]
    fn debug_hides_the_password() {
        assert!(!format!("{:?}", prev(vec![])).contains("12345678"));
    }
}
