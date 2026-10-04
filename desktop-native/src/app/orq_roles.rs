//! Aba "Time do trabalho" da Orquestração (`OrquestracaoSheet.svelte`): as etapas do contrato `regras-<gid>.md` na ordem do
//! trabalho, o formulário de um papel, a troca rápida de conta e modelo e o "Começar". Salvar grava o contrato e vale na
//! próxima sessão de cada papel; sessão viva nunca é tocada. As contas liberadas ficam em Configurações > Orquestração.
use super::*;
use super::accounts::ModelChoice;
use super::create::choices::{CLAUDE_EFFORTS, CODEX_PERMISSIONS, PERMISSIONS, PI_EFFORTS};
use super::create::choice;
use super::orchestration::provider_name;
use super::settings::Page;
use super::Role as A11y;
use crate::status::model_label;
use gpui_kit::component::{IndexPath, WindowExt, searchable_list::SearchableVec, select::{Select, SelectEvent, SelectState}};
use serde::{Deserialize, Serialize};
use std::future::Future;

const PROVIDERS: [&str; 5] = ["claude", "codex", "pi", "kimi", "omp"];
/// Ordem em que as etapas acontecem num trabalho da skill `orquestrar` (`ORDEM_ETAPAS`).
const STAGES: [&str; 6] = ["árbitro", "par de research", "executor", "revisor", "revisão final", "retrospectiva"];
const CANONICAL: [&str; 5] = ["árbitro", "executor", "revisor", "revisão final", "par de research"];
const WINDOWS: [&str; 5] = ["30", "40", "60", "70", "80"];
/// Cota a partir da qual a linha oferece trocar de conta; a oferta só vai para conta com 20 pontos de folga.
const QUOTA_SWITCH: f64 = 90.;
const QUOTA_HIGH: f64 = 80.;
const NEW: &str = "novo";

fn t(key: &str) -> String { tr_shared(key, &[]) }
fn t1(key: &str, name: &str, value: &str) -> String { tr_shared(key, &[(name, value)]) }

/// Uma linha do contrato, nos campos que as rotas de gravação aceitam.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
struct Role {
    papel: String,
    sessao: String,
    provider: String,
    conta: String,
    modelo: String,
    esforco: String,
    vez: String,
    headless: Option<bool>,
    permissao: String,
    motor: String,
    jev: bool,
    subagente: String,
    perfil: String,
    janela: String,
}

impl Role {
    fn key(&self) -> String { format!("{}::{}", self.papel, self.vez) }
    /// Provider vazio no contrato é o formato antigo, e quer dizer Claude.
    fn filled(mut self) -> Self {
        if self.provider.is_empty() { self.provider = "claude".into(); }
        self
    }
    /// Só o que vale para o provider escolhido (`abertura()` do web): nenhum caminho de gravação manda motor para o Codex.
    fn normalized(mut self) -> Self {
        let claude = self.provider == "claude";
        if !(claude || self.provider == "codex") { self.headless = None; }
        if !(claude || self.headless == Some(true)) { self.permissao.clear(); }
        if !claude { self.motor.clear(); }
        if !claude || !self.motor.is_empty() { self.subagente.clear(); }
        if self.provider == "omp" { self.perfil = self.perfil.trim().to_owned(); } else { self.perfil.clear(); }
        self
    }
}

#[derive(Clone, Deserialize)]
struct Row {
    #[serde(flatten)]
    role: Role,
    id_cota: Option<String>,
}

#[derive(Deserialize)]
struct Plan { path: String, state: String }

#[derive(Deserialize)]
struct Readiness { phase: String, plan: Option<Plan> }

#[derive(Deserialize)]
struct Group {
    gid: String,
    #[serde(default)]
    grouped: Option<bool>,
    #[serde(default)]
    session_prefix: String,
    arquivo: String,
    mtime: f64,
    papeis: Vec<Row>,
    prontidao: Option<Readiness>,
}

impl Group {
    fn has_group(&self) -> bool {
        self.grouped.unwrap_or(self.gid != "padrao" && !self.gid.starts_with("draft-"))
    }
}

#[derive(Clone, Deserialize)]
struct Allowed { provider: String, conta: String, #[serde(default)] apelido: String, modelos: Vec<String>, trocar: bool }

#[derive(Clone, Deserialize)]
struct Model { #[serde(default)] id: String, name: Option<String>, #[serde(default)] efforts: Vec<String> }

#[derive(Clone, Deserialize)]
struct Account { provider: String, conta: String, #[serde(default)] apelido: String, id_cota: Option<String>, #[serde(default)] modelos: Vec<Model> }

#[derive(Deserialize)]
struct Policy { #[serde(rename = "politica")] allowed: Vec<Allowed>, #[serde(rename = "inventario")] accounts: Vec<Account> }

#[derive(Clone, PartialEq)]
enum Sel { Row(String), New }

impl Sel {
    fn key(&self) -> String { match self { Sel::Row(key) => key.clone(), Sel::New => NEW.into() } }
}

type Pick = (Entity<SelectState<SearchableVec<ModelChoice>>>, Subscription);

#[derive(Clone, Copy, PartialEq)]
enum Field { Conta, Modelo, Esforco, Permissao, Motor, Subagente, Janela, QuickConta, QuickModelo }

#[derive(Default)]
struct Picks { conta: Option<Pick>, modelo: Option<Pick>, esforco: Option<Pick>, permissao: Option<Pick>, motor: Option<Pick>,
    subagente: Option<Pick>, janela: Option<Pick> }

/// Uma etapa por papel-base: faixas ("executor faixa A") e rodízio viram linhas dentro dela.
struct Stage { base: String, rows: Vec<Row> }

/// "executor faixa A" → ("executor", Some("A")).
fn lane(papel: &str) -> (String, Option<String>) {
    let words: Vec<&str> = papel.split_whitespace().collect();
    match words.as_slice() {
        [base @ .., word, lane] if !base.is_empty() && word.eq_ignore_ascii_case("faixa") => (base.join(" "), Some(lane.to_uppercase())),
        _ => (papel.trim().to_owned(), None),
    }
}

fn stages(rows: &[Row]) -> Vec<Stage> {
    let mut list: Vec<Stage> = Vec::new();
    for row in rows {
        let base = lane(&row.role.papel).0;
        match list.iter_mut().find(|s| s.base.to_lowercase() == base.to_lowercase()) {
            Some(stage) => stage.rows.push(row.clone()),
            None => list.push(Stage { base, rows: vec![row.clone()] }),
        }
    }
    let at = |s: &Stage| STAGES.iter().position(|n| *n == s.base.to_lowercase()).unwrap_or(STAGES.len());
    list.sort_by_key(at);
    list
}

fn purpose(base: &str) -> Option<String> {
    let key = match base.to_lowercase().as_str() {
        "árbitro" => "orqcfg_fim_arbitro", "executor" => "orqcfg_fim_executor", "revisor" => "orqcfg_fim_revisor",
        "revisão final" => "orqcfg_fim_revisao_final", "retrospectiva" => "orqcfg_fim_retrospectiva",
        "par de research" => "orqcfg_fim_research", _ => return None,
    };
    Some(t(key))
}

/// Família do modelo, o que sobrevive entre id e rótulo: `opus[1m]` e `Opus4.8·1M` dão ("opus", true) (`familiaDe`).
fn family(model: &str) -> Option<(String, bool)> {
    let s = model.trim().to_lowercase();
    let one = s.contains("[1m]") || s.contains("·1m") || s.ends_with("1m") || s.split(|c: char| !c.is_alphanumeric()).any(|w| w == "1m");
    let tail = s.rsplit('/').next().unwrap_or_default();
    let name: String = tail.chars().skip_while(|c| !c.is_ascii_lowercase()).take_while(char::is_ascii_lowercase).collect();
    (!name.is_empty()).then_some((name, one))
}

fn effort_word(effort: &str) -> String {
    let s: String = effort.to_lowercase().chars().filter(char::is_ascii_lowercase).collect();
    match s.as_str() { "med" => "medium".into(), "min" => "minimal".into(), _ => s }
}

/// Conta medida na grafia do contrato (`padrao`, `200-01`): só existe de verdade para o Claude.
fn measured_account(session: &SessionInfo) -> Option<String> {
    let id = session.conta.as_deref()?;
    match id.strip_prefix("claude:") {
        Some(dir) => {
            let name = dir.trim_end_matches('/').rsplit('/').next().unwrap_or_default();
            Some(if name == ".claude" { "padrao".into() } else { name.trim_start_matches(".claude-").to_owned() })
        }
        None => id.split_once(':').map(|(_, rest)| rest.to_owned()).filter(|r| !r.is_empty()),
    }
}

/// Casa o nome do contrato com uma sessão viva: exato, ou prefixo com `*` no fim e a mais recente (`casarViva`).
fn live_session<'a>(pattern: &str, sessions: &'a [SessionInfo]) -> Option<&'a SessionInfo> {
    if pattern.is_empty() { return None; }
    match pattern.strip_suffix('*') {
        None => sessions.iter().find(|s| s.name == pattern),
        Some(prefix) => sessions.iter().filter(|s| s.name.starts_with(prefix))
            .max_by(|a, b| a.last_activity.unwrap_or(0.).total_cmp(&b.last_activity.unwrap_or(0.))),
    }
}

/// O que a sessão viva do papel roda de fato, e se discorda do contrato. Sem medição, não discorda (`estadoDoPapel`).
struct Live { name: String, working: bool, model: Option<String>, effort: Option<String>, account: Option<String>,
    model_differs: bool, effort_differs: bool, account_differs: bool }

impl Live {
    fn of(role: &Role, session: &SessionInfo) -> Self {
        let status = crate::status::parse(session.status_line.as_deref(), Some(session));
        let (model, effort) = status.map(|s| (s.model, s.effort)).unwrap_or_default();
        let model_differs = model.as_deref().and_then(family).zip(family(&role.modelo)).is_some_and(|(a, b)| a != b);
        let effort_differs = effort.as_deref().is_some_and(|e| !role.esforco.is_empty() && effort_word(e) != effort_word(&role.esforco));
        let account_differs = role.provider == "claude" && session.conta.as_deref().and_then(|c| c.strip_prefix("claude:")).is_some_and(|dir| {
            let name = dir.trim_end_matches('/').rsplit('/').next().unwrap_or_default();
            let target = if role.conta == "padrao" { ".claude".to_owned() } else { format!(".claude-{}", role.conta) };
            name != target && name != role.conta
        });
        Self { name: session.name.clone(), working: session.state == "working", model, effort, account: measured_account(session),
            model_differs, effort_differs, account_differs }
    }
    fn differs(&self) -> bool { self.model_differs || self.effort_differs || self.account_differs }
}

pub(super) struct OrqRoles {
    api: Api,
    runtime: Arc<Runtime>,
    name: String,
    hangar: WeakEntity<Hangar>,
    load_seq: u64,
    group: Option<Result<Group, String>>,
    policy: Option<Policy>,
    /// Pior janela de cada conta do `/api/cotas`: (pct, rótulo da janela).
    quotas: HashMap<String, (f64, String)>,
    engines: Vec<(String, String)>,
    jev_key: bool,
    jev_default: bool,
    sel: Option<Sel>,
    /// Edições pendentes por linha (chave papel::vez original, ou `novo`): trocar de linha não descarta nada.
    drafts: HashMap<String, Role>,
    other: bool,
    quick: Option<String>,
    busy: bool,
    starting: bool,
    error: Option<String>,
    conflict: bool,
    notice: Option<String>,
    stale: bool,
    picks: Picks,
    quick_picks: Option<(String, Pick, Pick)>,
    papel_input: Entity<InputState>,
    perfil_input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl OrqRoles {
    fn new(api: Api, runtime: Arc<Runtime>, name: String, hangar: Entity<Hangar>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let papel_input = cx.new(|cx| InputState::new(window, cx).placeholder(t("orqcfg_papel_placeholder")));
        let perfil_input = cx.new(|cx| InputState::new(window, cx));
        let subscriptions = vec![
            // A sessão viva de cada papel muda com a lista do app.
            cx.observe(&hangar, |_, _, cx| cx.notify()),
            cx.subscribe_in(&papel_input, window, |this: &mut Self, input, event: &InputEvent, _, cx| {
                if !matches!(event, InputEvent::Change) || this.sel != Some(Sel::New) || !this.other { return; }
                let value = input.read(cx).value().to_string();
                this.edit(|r| r.papel = value);
                cx.notify();
            }),
            cx.subscribe_in(&perfil_input, window, |this: &mut Self, input, event: &InputEvent, _, cx| {
                if !matches!(event, InputEvent::Change) { return; }
                let value = input.read(cx).value().to_string();
                if this.current().is_some_and(|r| r.provider == "omp" && r.perfil != value.trim()) { this.edit(|r| r.perfil = value); cx.notify(); }
            }),
        ];
        let mut this = Self { api, runtime, name, hangar: hangar.downgrade(), load_seq: 0, group: None, policy: None, quotas: HashMap::new(),
            engines: Vec::new(), jev_key: false, jev_default: false, sel: None, drafts: HashMap::new(), other: false, quick: None,
            busy: false, starting: false, error: None, conflict: false, notice: None, stale: false, picks: Picks::default(),
            quick_picks: None, papel_input, perfil_input, _subscriptions: subscriptions };
        this.load(window, cx);
        this
    }

    fn spawn<T: Send + 'static>(&self, job: impl Future<Output = T> + Send + 'static, window: &mut Window, cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static) {
        let job = self.runtime.spawn(job);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(value) = job.await else {
                // Tarefa que morreu não pode deixar os botões travados em "salvando" sem dizer nada.
                let _ = this.update(cx, |this, cx| {
                    (this.busy, this.starting, this.error) = (false, false, Some(tr("network_error")));
                    cx.notify();
                });
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| { done(this, value, window, cx); cx.notify(); });
        }).detach();
    }

    /// Contrato, política, cota, motores e o Jev numa leitura só. A lista fica na tela enquanto relê.
    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.load_seq += 1;
        let seq = self.load_seq;
        let (api, name) = (self.api.clone(), self.name.clone());
        self.spawn(async move {
            tokio::join!(api.read(&name, &["orq"], &[], 15), api.server_read(&["orquestracao", "politica"], &[], 15),
                api.server_read(&["cotas"], &[], 15), api.server_read(&["engines"], &[], 15), api.server_read(&["config"], &[], 8))
        }, window, cx, move |this, (group, policy, quotas, engines, config), _, _| {
            if seq != this.load_seq { return; }
            let parse = |r: Result<Value, Failure>| -> Result<Group, String> {
                r.map_err(|e| Hangar::fetch_failure(&e)).and_then(|v| serde_json::from_value(v).map_err(|_| tr("invalid_response")))
            };
            let group = parse(group);
            let policy = policy.map_err(|e| Hangar::fetch_failure(&e))
                .and_then(|v| serde_json::from_value::<Policy>(v).map_err(|_| tr("invalid_response")));
            // A política é o que decide as contas: sem ela a tela não oferece escolha nenhuma, e diz por quê.
            match (group, policy) {
                (Ok(group), Ok(policy)) => { this.policy = Some(policy); this.group = Some(Ok(group)); }
                // Releitura que falha com a lista já na tela vira aviso; a lista fica.
                (Err(error), _) | (_, Err(error)) if matches!(this.group, Some(Ok(_))) => this.error = Some(error),
                (Err(error), _) | (_, Err(error)) => this.group = Some(Err(error)),
            }
            if let Ok(list) = quotas {
                this.quotas = list.as_array().into_iter().flatten().filter_map(|q| {
                    let worst = q.get("janelas")?.as_array()?.iter().filter_map(|w| Some((w.get("pct")?.as_f64()?,
                        w.get("rotulo").and_then(Value::as_str).unwrap_or_default().to_owned()))).max_by(|a, b| a.0.total_cmp(&b.0))?;
                    Some((q.get("id")?.as_str()?.to_owned(), worst))
                }).collect();
            }
            if let Ok(value) = engines {
                let mut list: Vec<(String, String)> = value.get("motores").and_then(Value::as_object).into_iter().flatten()
                    .map(|(name, m)| (name.clone(), m.get("label").and_then(Value::as_str).unwrap_or(name).to_owned())).collect();
                list.sort();
                this.engines = list;
            }
            if let Ok(value) = config {
                this.jev_key = value.pointer("/campos/jev_api_key/definido").and_then(Value::as_bool).unwrap_or(false);
                this.jev_default = value.pointer("/campos/jev_padrao/valor").and_then(Value::as_bool).unwrap_or(false);
            }
            // Linha que sumiu do contrato (vez trocada, removida por fora) não deixa o formulário apontando para o nada.
            if this.sel.as_ref().is_some_and(|sel| matches!(sel, Sel::Row(key) if this.original(key).is_none())) { this.sel = None; }
            // Rascunho de linha que sumiu do contrato recriaria a linha no próximo Salvar, sem aparecer nas mudanças.
            let keys: Vec<String> = this.rows().iter().map(|r| r.role.key()).collect();
            this.drafts.retain(|key, _| key == NEW || keys.contains(key));
            this.stale = true;
        });
        cx.notify();
    }

    fn rows(&self) -> &[Row] { self.group.as_ref().and_then(|g| g.as_ref().ok()).map(|g| g.papeis.as_slice()).unwrap_or_default() }
    fn original(&self, key: &str) -> Option<&Row> { self.rows().iter().find(|r| r.role.key() == key) }

    fn blank(&self) -> Role {
        Role { provider: "claude".into(), jev: self.jev_default, ..Role::default() }
    }

    /// A linha como vai ficar: o rascunho por cima do contrato.
    fn role_of(&self, key: &str) -> Option<Role> {
        if key == NEW { return Some(self.drafts.get(NEW).cloned().unwrap_or_else(|| self.blank())); }
        self.drafts.get(key).cloned().or_else(|| self.original(key).map(|r| r.role.clone().filled()))
    }
    fn current(&self) -> Option<Role> { self.role_of(&self.sel.as_ref()?.key()) }

    fn edit_key(&mut self, key: &str, change: impl FnOnce(&mut Role)) {
        let Some(mut role) = self.role_of(key) else { return };
        change(&mut role);
        let role = self.settle(role);
        if self.original(key).is_some_and(|o| o.role.clone().filled() == role) { self.drafts.remove(key); }
        else { self.drafts.insert(key.to_owned(), role); }
    }
    fn edit(&mut self, change: impl FnOnce(&mut Role)) {
        if let Some(key) = self.sel.as_ref().map(Sel::key) { self.edit_key(&key, change); }
    }

    /// Conta travada roda no primeiro modelo liberado, sem escolha.
    fn settle(&self, role: Role) -> Role {
        let mut role = role.filled();
        if self.policy_of(&role.provider, &role.conta).is_some_and(|p| !p.trocar) {
            if let Some(first) = self.models(&role.provider, &role.conta).first() { role.modelo = first.id.clone(); }
        }
        role.normalized()
    }

    // ── Política ──

    /// Contas liberadas do provider. Política vazia = nada proibido ainda: o inventário inteiro vale, com todos os modelos.
    fn allowed(&self, provider: &str) -> Vec<Allowed> {
        let Some(policy) = &self.policy else { return Vec::new() };
        if policy.allowed.is_empty() {
            return policy.accounts.iter().filter(|a| a.provider == provider).map(|a| Allowed { provider: a.provider.clone(),
                conta: a.conta.clone(), apelido: a.apelido.clone(), modelos: vec!["*".into()], trocar: true }).collect();
        }
        policy.allowed.iter().filter(|a| a.provider == provider).cloned().collect()
    }
    fn policy_of(&self, provider: &str, conta: &str) -> Option<Allowed> { self.allowed(provider).into_iter().find(|a| a.conta == conta) }
    fn account(&self, provider: &str, conta: &str) -> Option<&Account> {
        self.policy.as_ref()?.accounts.iter().find(|a| a.provider == provider && a.conta == conta)
    }
    fn account_label(&self, conta: &str) -> String {
        self.policy.as_ref().and_then(|p| p.accounts.iter().find(|a| a.conta == conta && !a.apelido.is_empty()))
            .map_or_else(|| conta.to_owned(), |a| a.apelido.clone())
    }
    /// Modelos que a política deixa escolher: `*` abre o catálogo; lista fechada na ordem do catálogo, e o id digitado no fim.
    fn models(&self, provider: &str, conta: &str) -> Vec<Model> {
        let Some(policy) = self.policy_of(provider, conta) else { return Vec::new() };
        let catalog: Vec<Model> = self.account(provider, conta).map(|a| a.modelos.iter().filter(|m| !m.id.is_empty()).cloned().collect()).unwrap_or_default();
        if policy.modelos.iter().any(|m| m == "*") { return catalog; }
        let mut list: Vec<Model> = catalog.into_iter().filter(|m| policy.modelos.contains(&m.id)).collect();
        for id in &policy.modelos {
            if !list.iter().any(|m| &m.id == id) { list.push(Model { id: id.clone(), name: None, efforts: Vec::new() }); }
        }
        list
    }
    /// O modelo só acompanha a troca de conta se a conta nova o libera; senão fica no padrão dela.
    fn model_follows(&self, provider: &str, conta: &str, model: &str) -> String {
        let open = self.policy_of(provider, conta).is_some_and(|p| p.modelos.iter().any(|m| m == "*"));
        if open || self.models(provider, conta).iter().any(|m| m.id == model) { model.to_owned() } else { String::new() }
    }
    fn model_name(&self, role: &Role) -> String {
        if role.modelo.is_empty() { return t("criar_padrao"); }
        self.account(&role.provider, &role.conta).and_then(|a| a.modelos.iter().find(|m| m.id == role.modelo)).and_then(|m| m.name.clone())
            .unwrap_or_else(|| model_label(&role.modelo))
    }
    fn quota(&self, provider: &str, conta: &str, id: Option<&str>) -> Option<&(f64, String)> {
        let id = id.map(str::to_owned).or_else(|| self.account(provider, conta)?.id_cota.clone())?;
        self.quotas.get(&id)
    }
    /// Conta liberada do mesmo provider com mais folga. Só com leitura de cota: trocar às cegas pode dar noutra tão cheia.
    fn roomier(&self, role: &Role) -> Option<(String, f64)> {
        self.allowed(&role.provider).into_iter().filter(|a| a.conta != role.conta)
            .filter_map(|a| self.quota(&role.provider, &a.conta, None).map(|q| (a.conta, q.0)))
            .filter(|(_, pct)| *pct < QUOTA_SWITCH - 20.).min_by(|a, b| a.1.total_cmp(&b.1))
    }
    fn levels(&self, role: &Role) -> Vec<String> {
        match role.provider.as_str() {
            "claude" => CLAUDE_EFFORTS.map(String::from).to_vec(),
            "pi" | "omp" => PI_EFFORTS.map(String::from).to_vec(),
            "codex" => self.models(&role.provider, &role.conta).into_iter().find(|m| m.id == role.modelo).map(|m| m.efforts).unwrap_or_default(),
            _ => Vec::new(),
        }
    }
    fn permissions(role: &Role) -> Option<&'static [&'static str]> {
        match (role.provider.as_str(), role.headless) { ("claude", _) => Some(&PERMISSIONS), ("codex", Some(true)) => Some(&CODEX_PERMISSIONS), _ => None }
    }

    /// Papéis novos usam a identidade do trabalho, sem inferir nomes de outras linhas.
    fn derived_session(&self, papel: &str) -> String {
        let group = self.group.as_ref().and_then(|g| g.as_ref().ok());
        let base = group.map(|g| g.session_prefix.trim()).filter(|p| !p.is_empty()).unwrap_or(&self.name);
        let prefix = if base.ends_with('-') { base.to_owned() } else { format!("{base}-") };
        let lower = papel.to_lowercase();
        let suffix = match lower.as_str() {
            "árbitro" => "arbitro".into(), "executor" => "t*".into(), "revisor" => "review*".into(), "revisão final" => "final".into(),
            "par de research" => "mock".into(),
            _ => {
                let slug: String = lower.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
                format!("{}*", slug.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-"))
            }
        };
        prefix + &suffix
    }

    // ── Escolhas ──

    fn choose(&mut self, sel: Sel, window: &mut Window, cx: &mut Context<Self>) {
        self.sel = Some(sel.clone());
        (self.notice, self.error, self.conflict) = (None, None, false);
        let role = self.current().unwrap_or_default();
        self.other = sel == Sel::New && !role.papel.is_empty() && !CANONICAL.contains(&role.papel.as_str());
        let (papel, perfil) = (if self.other { role.papel.clone() } else { String::new() }, role.perfil.clone());
        self.papel_input.update(cx, |input, cx| input.set_value(papel, window, cx));
        self.perfil_input.update(cx, |input, cx| input.set_value(perfil, window, cx));
        self.stale = true;
        cx.notify();
    }

    fn pick(field: Field, choices: Vec<ModelChoice>, value: &str, on: impl Fn(&mut Self, String) + 'static, window: &mut Window,
        cx: &mut Context<Self>) -> Pick {
        let at = choices.iter().position(|c| c.id == value);
        let state = cx.new(|cx| SelectState::new(SearchableVec::new(choices), at.map(IndexPath::new), window, cx));
        let subscription = cx.subscribe_in(&state, window, move |this, _, event: &SelectEvent<SearchableVec<ModelChoice>>, window, cx| {
            if let SelectEvent::Confirm(Some(id)) = event {
                on(this, id.clone());
                this.build_picks(Some(field), window, cx);
                cx.notify();
            }
        });
        (state, subscription)
    }

    fn choice(id: &str, label: String, hint: String) -> ModelChoice { ModelChoice { id: id.to_owned(), label, hint } }

    fn account_choices(&self, provider: &str) -> Vec<ModelChoice> {
        self.allowed(provider).into_iter().map(|a| {
            let quota = self.quota(provider, &a.conta, None).map(|q| t1("orqcfg_cota_pct", "pct", &q.0.round().to_string()));
            let hint = [Some(a.conta.clone()).filter(|_| !a.apelido.is_empty()), quota].into_iter().flatten().collect::<Vec<_>>().join(" · ");
            Self::choice(&a.conta, if a.apelido.is_empty() { a.conta.clone() } else { a.apelido }, hint)
        }).collect()
    }

    fn model_choices(&self, role: &Role) -> Vec<ModelChoice> {
        let models = self.models(&role.provider, &role.conta);
        let mut list = vec![Self::choice("", t("criar_padrao"), String::new())];
        // O modelo gravado pode não estar no catálogo da conta: fica na lista, senão o campo não teria opção para ele.
        if !role.modelo.is_empty() && !models.iter().any(|m| m.id == role.modelo) { list.push(Self::choice(&role.modelo, model_label(&role.modelo), String::new())); }
        list.extend(models.into_iter().map(|m| Self::choice(&m.id, m.name.clone().unwrap_or_else(|| m.id.clone()),
            if m.name.is_some() { m.id.clone() } else { String::new() })));
        list
    }

    /// Refaz os seletores a partir do rascunho. O que acabou de ser escolhido (`keep`) fica o mesmo: recriá-lo tiraria o foco
    /// de quem está no teclado, e as opções dele não dependem do próprio valor.
    fn build_picks(&mut self, keep: Option<Field>, window: &mut Window, cx: &mut Context<Self>) {
        self.stale = false;
        let mut old = std::mem::take(&mut self.picks);
        let (mut old_quick_conta, mut old_quick_modelo) = match self.quick_picks.take() {
            Some((key, conta, modelo)) if self.quick.as_ref() == Some(&key) => (Some(conta), Some(modelo)),
            _ => (None, None),
        };
        let kept = |slot: &mut Option<Pick>, field: Field| slot.take().filter(|_| keep == Some(field));
        if let Some(key) = self.quick.clone() {
            self.quick_picks = self.role_of(&key).map(|role| {
                let (k1, k2) = (key.clone(), key.clone());
                let conta = kept(&mut old_quick_conta, Field::QuickConta).unwrap_or_else(|| Self::pick(Field::QuickConta,
                    self.account_choices(&role.provider), &role.conta, move |this, v| {
                        let Some(role) = this.role_of(&k1) else { return };
                        // O modelo acompanha a conta nova quando ela o libera.
                        let follow = this.model_follows(&role.provider, &v, &role.modelo);
                        this.edit_key(&k1, |r| { r.conta = v; r.modelo = follow; });
                    }, window, cx));
                let modelo = kept(&mut old_quick_modelo, Field::QuickModelo).unwrap_or_else(|| Self::pick(Field::QuickModelo,
                    self.model_choices(&role), &role.modelo, move |this, v| this.edit_key(&k2, |r| r.modelo = v), window, cx));
                (key, conta, modelo)
            });
        }
        let Some(role) = self.current() else { return };
        if !self.allowed(&role.provider).is_empty() {
            self.picks.conta = kept(&mut old.conta, Field::Conta).or_else(|| Some(Self::pick(Field::Conta, self.account_choices(&role.provider),
                &role.conta, |this, v| {
                    let Some(role) = this.current() else { return };
                    let follow = this.model_follows(&role.provider, &v, &role.modelo);
                    this.edit(|r| { r.conta = v; r.modelo = follow; });
                }, window, cx)));
        }
        self.picks.modelo = kept(&mut old.modelo, Field::Modelo).or_else(|| Some(Self::pick(Field::Modelo, self.model_choices(&role),
            &role.modelo, |this, v| this.edit(|r| r.modelo = v), window, cx)));
        let levels = self.levels(&role);
        if !levels.is_empty() {
            let choices = std::iter::once(Self::choice("", t("criar_padrao"), String::new()))
                .chain(levels.iter().map(|l| Self::choice(l, l.clone(), String::new()))).collect();
            self.picks.esforco = kept(&mut old.esforco, Field::Esforco).or_else(|| Some(Self::pick(Field::Esforco, choices, &role.esforco,
                |this, v| this.edit(|r| r.esforco = v), window, cx)));
        }
        if let Some(modes) = Self::permissions(&role) {
            let choices = std::iter::once(Self::choice("", tr("create_permission_default"), String::new()))
                .chain(modes.iter().map(|m| Self::choice(m, (*m).to_owned(), String::new()))).collect();
            self.picks.permissao = kept(&mut old.permissao, Field::Permissao).or_else(|| Some(Self::pick(Field::Permissao, choices,
                &role.permissao, |this, v| this.edit(|r| r.permissao = v), window, cx)));
        }
        if role.provider == "claude" && !self.engines.is_empty() {
            let choices = std::iter::once(Self::choice("", tr("create_own_account"), String::new()))
                .chain(self.engines.iter().map(|(name, label)| Self::choice(name, label.clone(), String::new()))).collect();
            self.picks.motor = kept(&mut old.motor, Field::Motor).or_else(|| Some(Self::pick(Field::Motor, choices, &role.motor,
                |this, v| this.edit(|r| r.motor = v), window, cx)));
        }
        let models = self.models(&role.provider, &role.conta);
        if role.provider == "claude" && role.motor.is_empty() && !models.is_empty() {
            let choices = std::iter::once(Self::choice("", tr("create_subagent_default"), String::new()))
                .chain(models.into_iter().map(|m| Self::choice(&m.id, m.name.unwrap_or_else(|| m.id.clone()), String::new()))).collect();
            self.picks.subagente = kept(&mut old.subagente, Field::Subagente).or_else(|| Some(Self::pick(Field::Subagente, choices,
                &role.subagente, |this, v| this.edit(|r| r.subagente = v), window, cx)));
        }
        let choices = std::iter::once(Self::choice("", t1("orqcfg_janela_padrao", "pct", "50"), String::new()))
            .chain(WINDOWS.iter().map(|n| Self::choice(n, format!("{n}%"), String::new()))).collect();
        self.picks.janela = kept(&mut old.janela, Field::Janela).or_else(|| Some(Self::pick(Field::Janela, choices, &role.janela,
            |this, v| this.edit(|r| r.janela = v), window, cx)));
    }

    fn set_provider(&mut self, provider: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        self.perfil_input.update(cx, |input, cx| input.set_value("", window, cx));
        let conta = self.allowed(provider).first().map(|a| a.conta.clone()).unwrap_or_default();
        // O Jev vale em qualquer provider; o resto da abertura é por provider e volta ao padrão.
        self.edit(|r| {
            r.provider = provider.into();
            r.conta = conta;
            (r.modelo, r.esforco, r.permissao, r.motor, r.subagente, r.perfil) = Default::default();
            r.headless = None;
        });
        self.stale = true;
        cx.notify();
    }

    /// Lines of the same role name, in contract order: the table order IS the rotation order.
    fn lines_of(&self, papel: &str) -> Vec<Row> {
        self.rows().iter().filter(|r| r.role.papel.trim().to_lowercase() == papel.trim().to_lowercase()).cloned().collect()
    }

    /// Troca o modo do papel inteiro: renumera TODAS as linhas dele, senão o backend leria meia fila como outra coisa.
    fn set_rotation(&mut self, rotate: bool, cx: &mut Context<Self>) {
        let Some(Sel::Row(key)) = self.sel.clone() else { return };
        if !rotate { self.edit(|r| r.vez.clear()); cx.notify(); return; }
        let papel = self.original(&key).map(|o| o.role.papel.clone()).unwrap_or_default();
        let lines = self.lines_of(&papel);
        for (n, line) in lines.iter().enumerate() { self.edit_key(&line.role.key(), |r| r.vez = (n + 1).to_string()); }
        if lines.is_empty() { self.edit(|r| r.vez = "1".into()); }
        cx.notify();
    }

    fn drop_drafts_of(&mut self, papel: &str) {
        let prefix = format!("{papel}::");
        self.drafts.retain(|key, _| !key.starts_with(&prefix));
    }

    // ── Gravação ──

    fn mtime(&self) -> f64 { self.group.as_ref().and_then(|g| g.as_ref().ok()).map_or(0., |g| g.mtime) }

    /// Mutação no contrato. 409 é o arquivo que mudou desde a leitura: pede recarregar, nunca sobrescreve.
    fn mutate(&mut self, path: &'static [&'static str], body: Value, delete: bool, window: &mut Window, cx: &mut Context<Self>,
        ok: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static) {
        if self.busy { return; }
        (self.busy, self.error, self.notice, self.conflict) = (true, None, None, false);
        let (api, name) = (self.api.clone(), self.name.clone());
        self.spawn(async move { api.act(&name, path, Some(body), delete, 20).await }, window, cx, |this, result, window, cx| {
            this.busy = false;
            match result {
                Ok(value) => {
                    // O mtime novo vale já: salvar de novo antes da releitura não pode virar um 409 falso.
                    if let (Some(mtime), Some(Ok(group))) = (value.get("mtime").and_then(Value::as_f64), this.group.as_mut()) { group.mtime = mtime; }
                    ok(this, window, cx);
                    this.load(window, cx);
                }
                Err(error) if error.status == Some(409) => this.conflict = true,
                Err(error) => this.error = Some(Hangar::fetch_failure(&error)),
            }
        });
        cx.notify();
    }

    fn write(&mut self, items: Vec<Role>, window: &mut Window, cx: &mut Context<Self>, ok: impl FnOnce(&mut Self) + 'static) {
        if items.is_empty() { return; }
        let body = json!({"papeis": items, "mtime": self.mtime()});
        self.mutate(&["orq", "papeis"], body, false, window, cx, move |this, _, _| {
            ok(this);
            this.notice = Some(t("orqcfg_aviso_proxima_sessao"));
        });
    }

    fn save(&mut self, back_to_list: bool, window: &mut Window, cx: &mut Context<Self>) {
        // Linha sem conta não grava: recusa o lote inteiro e diz qual, em vez de salvar o resto e apagar essa edição.
        let mut missing: Vec<String> = self.drafts.values().filter(|r| !r.papel.trim().is_empty() && r.conta.is_empty())
            .map(|r| r.papel.trim().to_owned()).collect();
        if !missing.is_empty() {
            missing.sort();
            self.error = Some(format!("{}: {}", missing.join(", "), t("orqcfg_fila_sem_conta")));
            cx.notify();
            return;
        }
        let items: Vec<Role> = self.drafts.iter().filter(|(_, r)| !r.papel.trim().is_empty()).map(|(key, r)| {
            let mut r = r.clone();
            r.papel = r.papel.trim().to_owned();
            if key == NEW && r.sessao.trim().is_empty() { r.sessao = self.derived_session(&r.papel); }
            r
        }).collect();
        // Vez trocada muda a chave da linha: o formulário apontaria para a linha antiga.
        let same_line = match &self.sel {
            Some(Sel::Row(key)) => self.drafts.get(key).is_none_or(|d| self.original(key).is_some_and(|o| o.role.vez == d.vez)),
            _ => false,
        };
        self.write(items, window, cx, move |this| {
            this.drafts.clear();
            if back_to_list || !same_line { this.sel = None; }
        });
    }

    /// Cota no limite: troca a conta da linha pela de mais folga; vale na próxima sessão do papel.
    fn switch_now(&mut self, key: String, conta: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut role) = self.role_of(&key) else { return };
        role.modelo = self.model_follows(&role.provider, &conta, &role.modelo);
        role.conta = conta;
        self.write(vec![role], window, cx, move |this| { this.drafts.remove(&key); });
    }

    /// Cria a próxima linha do rodízio. A primeira conta pode estar sem `vez` (papel que era único): vira a 1 na mesma gravação.
    fn add_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Sel::Row(key)) = self.sel.clone() else { return };
        let Some(papel) = self.original(&key).map(|o| o.role.papel.clone()) else { return };
        let lines = self.lines_of(&papel);
        let used: Vec<String> = lines.iter().map(|l| l.role.vez.trim().to_owned()).collect();
        let free = (1..).find(|n: &usize| !used.contains(&n.to_string())).unwrap_or(1);
        let mut items: Vec<Role> = lines.iter().enumerate().map(|(i, l)| {
            let mut r = l.role.clone().filled();
            if r.vez.trim().is_empty() { r.vez = (i + 1).to_string(); }
            r
        }).collect();
        let base = lines.first().map(|l| l.role.clone().filled()).unwrap_or_default();
        // Nasce na primeira conta liberada: com a política montada, conta vazia é recusada pelo backend.
        let conta = self.allowed(&base.provider).first().map(|a| a.conta.clone()).unwrap_or_default();
        items.push(Role { papel: base.papel.clone(), sessao: base.sessao.clone(), provider: base.provider.clone(), janela: base.janela.clone(),
            conta, vez: free.max(items.len() + 1).to_string(), ..Role::default() });
        let body = json!({"papeis": items, "mtime": self.mtime()});
        self.mutate(&["orq", "papeis"], body, false, window, cx, move |this, _, _| { this.drop_drafts_of(&papel); this.sel = None; });
    }

    /// Tira uma conta da fila, ou o papel inteiro quando é a única linha.
    fn remove_line(&mut self, key: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(role) = self.original(&key).map(|o| o.role.clone()) else { return };
        let body = json!({"papel": role.papel, "vez": role.vez, "mtime": self.mtime()});
        self.mutate(&["orq", "papel"], body, true, window, cx, move |this, _, _| { this.drop_drafts_of(&role.papel); this.sel = None; });
    }

    /// Acorda ESTA sessão no passo que falta. O único erro é o recado não chegar, e ele aparece na tela.
    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.starting { return; }
        (self.starting, self.error, self.notice) = (true, None, None);
        let (api, name) = (self.api.clone(), self.name.clone());
        self.spawn(async move { api.act(&name, &["orq", "comecar"], Some(json!({})), false, 20).await }, window, cx, |this, result, window, cx| {
            this.starting = false;
            match result {
                Ok(value) => {
                    let delivered = value.get("entregue").and_then(Value::as_bool) == Some(true);
                    window.close_dialog(cx);
                    window.push_notification(Notification::success(t(if delivered { "orqcfg_comecou" } else { "orqcfg_comecou_fila" })), cx);
                }
                Err(error) => this.error = Some(Hangar::fetch_failure(&error)),
            }
        });
        cx.notify();
    }

    fn open_accounts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
        let _ = self.hangar.update(cx, |hangar, cx| hangar.open_settings(Page::Orchestration, window, cx));
    }

    // ── Desenho ──

    /// A lista da máquina do diálogo (a da sessão que o abriu), não a da ativa.
    fn sessions(&self, cx: &App) -> Vec<SessionInfo> { self.hangar.upgrade().map(|h| h.read(cx).sessions_of(&self.api.identity()).to_vec()).unwrap_or_default() }

    fn pending(&self) -> Vec<(String, String)> {
        let mut list: Vec<(String, String)> = self.drafts.iter().filter_map(|(key, role)| {
            let Some(original) = self.original(key) else {
                return (key == NEW && !role.papel.trim().is_empty()).then(|| (role.papel.clone(), t("orqcfg_mudanca_novo")));
            };
            let before = original.role.clone().filled();
            let changes: Vec<String> = [("provider", "comum_provider"), ("conta", "orqcfg_conta"), ("modelo", "composer_modelo"),
                ("esforco", "composer_esforco"), ("janela", "orqcfg_janela"), ("headless", "criar_modo_exec"), ("permissao", "criar_permissao"),
                ("motor", "comum_motor"), ("subagente", "criar_subagente"), ("jev", "criar_jev"), ("perfil", "criar_perfil_omp"),
                ("sessao", "orqcfg_campo_sessao"), ("vez", "orqcfg_fila_titulo")]
                .into_iter().filter_map(|(field, label)| {
                    let (a, b) = (self.field(&before, field), self.field(role, field));
                    (a != b).then(|| format!("{} {a} → {b}", t(label).to_lowercase()))
                }).collect();
            (!changes.is_empty()).then(|| (role.papel.clone(), changes.join("; ")))
        }).collect();
        list.sort();
        list
    }

    fn field(&self, role: &Role, field: &str) -> String {
        let text = |v: &str| if v.is_empty() { t("criar_padrao") } else { v.to_owned() };
        match field {
            "provider" => provider_name(&role.provider).to_owned(),
            "conta" => if role.conta.is_empty() { t("criar_padrao") } else { self.account_label(&role.conta) },
            "modelo" => if role.modelo.is_empty() { t("criar_padrao") } else { model_label(&role.modelo) },
            "esforco" => text(&role.esforco),
            "janela" => if role.janela.is_empty() { t("criar_padrao") } else { format!("{}%", role.janela) },
            "headless" => t(match role.headless { Some(true) => "criar_modo_exec_headless", Some(false) => "criar_modo_exec_tmux", None => "criar_padrao" }),
            "jev" => t(if role.jev { "orqcfg_ligado" } else { "orqcfg_desligado" }),
            "permissao" => text(&role.permissao),
            "motor" => text(&role.motor),
            "subagente" => text(&role.subagente),
            "perfil" => text(&role.perfil),
            "sessao" => text(&role.sessao),
            _ if role.vez.is_empty() => t("orqcfg_modo_unica"),
            _ => t1("orqcfg_vez_n", "n", &role.vez),
        }
    }

    fn render_cell(&self, row: &Row, title: Option<String>, sub: Option<String>, sessions: &[SessionInfo], usage: &HashMap<String, usize>,
        cx: &mut Context<Self>) -> AnyElement {
        let key = row.role.key();
        let role = self.role_of(&key).unwrap_or_default();
        let selected = self.sel == Some(Sel::Row(key.clone()));
        let live = live_session(&role.sessao, sessions).map(|s| Live::of(&role, s));
        let quota = self.quota(&role.provider, &role.conta, row.id_cota.as_deref().filter(|_| row.role.conta == role.conta)).map(|q| q.0);
        let shared = usage.get(&format!("{}::{}", role.provider, role.conta)).copied().unwrap_or(0);
        let roomier = quota.filter(|pct| *pct >= QUOTA_SWITCH).and_then(|_| self.roomier(&role));
        let label = title.unwrap_or_else(|| match (lane(&role.papel).1, role.vez.is_empty()) {
            (Some(f), _) => t1("orqcfg_faixa_n", "f", &f),
            (None, false) => t1("orqcfg_vez_n", "n", &role.vez),
            _ => role.papel.clone(),
        });
        let who = format!("{}{}", sub.map(|s| format!("{s} · ")).unwrap_or_default(),
            if role.conta.is_empty() { t("orqcfg_fila_sem_conta") } else { self.account_label(&role.conta) });
        let high = quota.is_some_and(|p| p >= QUOTA_HIGH);
        let quick_open = self.quick.as_deref() == Some(key.as_str());
        let (k_open, k_quick, k_switch) = (key.clone(), key.clone(), key.clone());
        let (dot_color, dot_text) = match &live {
            Some(l) if l.differs() => (theme::warning(), t("orqcfg_viva")),
            Some(l) if l.working => (theme::status("working"), t("orqcfg_trabalhando")),
            Some(_) => (theme::success(), t("orqcfg_viva")),
            None => (theme::faint(), t("orqcfg_nao_aberta")),
        };
        let differs = live.as_ref().filter(|l| l.differs()).map(|l| {
            let parts: Vec<String> = [l.account_differs.then(|| l.account.clone()).flatten(), l.model_differs.then(|| l.model.clone()).flatten(),
                l.effort_differs.then(|| l.effort.clone()).flatten()].into_iter().flatten().collect();
            t1("orqcfg_rodando_em", "v", &parts.join(" · "))
        });
        let open = Button::new(SharedString::from(format!("orq-row-{key}"))).ghost().flex_1().min_w_0().justify_start().h_auto().py(px(6.))
            .accessibility_label(format!("{label} · {who} · {dot_text}"))
            .child(div().w_full().min_w_0().flex().flex_col().gap(px(2.))
                .child(div().flex().items_center().gap(px(6.)).min_w_0()
                    .child(div().truncate().font_weight(FontWeight::MEDIUM).child(label))
                    .when(self.drafts.contains_key(&key), |el| el.child(super::server_config::chip(t("orqcfg_editado"), theme::accent_text(), theme::accent_dim()))))
                // Cada pedaço não quebra por dentro: a linha só dobra entre um e outro.
                .child(div().flex().flex_wrap().gap_x(px(4.)).text_xs().text_color(theme::muted())
                    .child(div().whitespace_nowrap().child(who))
                    .when_some(quota, |el, pct| el.child(div().whitespace_nowrap().when(high, |el| el.text_color(theme::warning()))
                        .child(format!("· {}", t1("orqcfg_cota_pct", "pct", &pct.round().to_string())))))
                    .when(shared > 1, |el| el.child(div().whitespace_nowrap().child(format!("· {}", t1("orqcfg_conta_dividida", "n", &shared.to_string())))))))
            .on_click(cx.listener(move |this, _, window, cx| this.choose(Sel::Row(k_open.clone()), window, cx)));
        let model = chrome::pill_button(SharedString::from(format!("orq-quick-{key}")), cx).gap(px(5.)).selected(quick_open).disabled(self.busy)
            .tooltip(t("orqcfg_trocar_rapido"))
            .child(chrome::provider_glyph(&role.provider, 13.))
            .child(div().max_w(px(130.)).truncate().text_xs().child(self.model_name(&role)))
            .when(!role.esforco.is_empty(), |el| el.child(div().text_xs().text_color(theme::muted()).child(role.esforco.clone())))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.quick = if this.quick.as_deref() == Some(k_quick.as_str()) { None } else { Some(k_quick.clone()) };
                this.stale = true;
                cx.notify();
            }));
        // O estado já vai no rótulo da linha: um `Status` por linha faria o leitor anunciar cada mudança de todas.
        let dot = div().id(SharedString::from(format!("orq-state-{key}")))
            .size(px(8.)).flex_shrink_0().rounded_full()
            .map(|el| if live.is_some() { el.bg(dot_color) } else { el.border_1().border_color(dot_color) });
        let quick = self.quick_picks.as_ref().filter(|(k, ..)| quick_open && *k == key).map(|(_, conta, modelo)| {
            let locked = self.policy_of(&role.provider, &role.conta).is_some_and(|p| !p.trocar);
            div().flex().gap_2().px(px(8.)).pb(px(6.))
                .child(div().flex_1().min_w_0().child(Select::new(&conta.0).small().disabled(self.busy).accessibility_label(t("orqcfg_conta"))))
                .child(div().flex_1().min_w_0().child(Select::new(&modelo.0).small().disabled(locked || self.busy).accessibility_label(t("composer_modelo"))))
        });
        div().rounded(px(8.)).border_1().border_color(if selected { theme::accent() } else { theme::border() })
            .when(selected, |el| el.bg(theme::accent_dim()))
            .child(div().flex().items_center().gap(px(6.)).pr(px(12.)).child(open).child(dot))
            .child(div().flex().flex_wrap().items_center().gap(px(6.)).px(px(8.)).pb(px(8.)).child(model)
                .when_some(roomier, |el, (conta, pct)| el.child(
                    Button::new(SharedString::from(format!("orq-switch-{key}"))).outline().xsmall().disabled(self.busy)
                        .label(tr_shared("orqcfg_trocar_para", &[("conta", &conta), ("pct", &pct.round().to_string())]))
                        .on_click(cx.listener(move |this, _, window, cx| this.switch_now(k_switch.clone(), conta.clone(), window, cx))))))
            .when_some(differs, |el, text| el.child(div().px(px(12.)).pb(px(8.)).text_xs().text_color(theme::warning()).whitespace_normal().child(text)))
            .children(quick)
            .into_any_element()
    }

    fn render_list(&self, group: &Group, sessions: &[SessionInfo], cx: &mut Context<Self>) -> Div {
        let rows = &group.papeis;
        let viewed: Vec<Role> = rows.iter().filter_map(|r| self.role_of(&r.role.key())).collect();
        let mut usage: HashMap<String, usize> = HashMap::new();
        for role in viewed.iter().filter(|r| !r.conta.is_empty()) { *usage.entry(format!("{}::{}", role.provider, role.conta)).or_default() += 1; }
        let progress = rows.iter().filter_map(|r| live_session(&r.role.sessao, sessions))
            .find_map(|s| s.plan_task.zip(s.plan_task_total).filter(|(_, total)| *total > 0));
        let phase = group.prontidao.as_ref().map(|p| {
            let plan = p.plan.as_ref().map(|plan| plan.path.rsplit('/').next().unwrap_or_default().to_owned()).unwrap_or_default();
            match p.phase.as_str() {
                "planner" => t("orqcfg_fase_planner"),
                "prepare" if p.plan.as_ref().is_some_and(|plan| plan.state == "changed") => t1("orqcfg_fase_changed", "plano", &plan),
                "prepare" => t1("orqcfg_fase_prepare", "plano", &plan),
                "launch" => t1("orqcfg_fase_launch", "plano", &plan),
                _ => t1("orqcfg_fase_arbiter", "plano", &plan),
            }
        });
        let muted = |text: String| div().text_sm().text_color(theme::muted()).whitespace_normal().child(text);
        let mut list = div().flex().flex_col().gap_3()
            .when(!group.has_group(), |el| el.child(muted(t("orqcfg_sem_grupo"))))
            .child(muted(t(if rows.is_empty() { "orqcfg_sem_papeis" } else { "orqcfg_papeis_intro" })))
            .when_some(progress, |el, (task, total)| el.child(div().text_sm().text_color(theme::accent()).child(
                tr_shared("orqcfg_andamento", &[("t", &task.to_string()), ("total", &total.to_string())]))))
            // Começar é ação do grupo, não do formulário de um papel: a frase diz de que passo a sessão parte.
            .child(div().flex().flex_col().gap_2().p_3().rounded(px(8.)).bg(theme::inset())
                .children(phase.map(muted))
                .child(Button::new("orq-start").primary().w_full().label(t(if self.starting { "orqcfg_comecando" } else { "orqcfg_comecar" }))
                    .loading(self.starting).disabled(self.starting)
                    .on_click(cx.listener(|this, _, window, cx| this.start(window, cx)))));
        for (n, stage) in stages(rows).into_iter().enumerate() {
            let purpose = purpose(&stage.base);
            let working = stage.rows.iter().any(|r| live_session(&r.role.sessao, sessions).is_some_and(|s| s.state == "working"));
            let number = div().mt(px(8.)).size(px(20.)).flex_shrink_0().rounded_full().border_1()
                .border_color(if working { theme::accent() } else { theme::border_strong() }).flex().items_center().justify_center()
                .text_xs().text_color(if working { theme::accent() } else { theme::muted() }).child((n + 1).to_string());
            let body = if stage.rows.len() > 1 {
                let lanes = stage.rows.iter().any(|r| lane(&r.role.papel).1.is_some());
                let summary = t1(if lanes { "orqcfg_faixas_n" } else { "orqcfg_modo_reveza_resumo" }, "n", &stage.rows.len().to_string());
                div().flex().flex_col().gap(px(6.))
                    .child(div().pt(px(8.)).flex().flex_col()
                        .child(div().font_weight(FontWeight::MEDIUM).child(purpose.clone().unwrap_or_else(|| stage.base.clone())))
                        .child(div().text_xs().text_color(theme::muted()).child(match &purpose { Some(_) => format!("{} · {summary}", stage.base), None => summary })))
                    .children(stage.rows.iter().map(|r| self.render_cell(r, None, None, sessions, &usage, cx)))
            } else {
                let title = purpose.clone().unwrap_or_else(|| stage.base.clone());
                div().child(self.render_cell(&stage.rows[0], Some(title), purpose.map(|_| stage.base.clone()), sessions, &usage, cx))
            };
            list = list.child(div().flex().gap_2().child(number).child(body.flex_1().min_w_0()));
        }
        list.child(Button::new("orq-new").ghost().small().icon(IconName::Plus).label(t("orqcfg_novo_papel"))
            .selected(self.sel == Some(Sel::New)).justify_start()
            .on_click(cx.listener(|this, _, window, cx| this.choose(Sel::New, window, cx))))
    }

    fn render_form(&self, sessions: &[SessionInfo], cx: &mut Context<Self>) -> Div {
        let (Some(sel), Some(role)) = (self.sel.clone(), self.current()) else {
            return div().size_full().flex().items_center().justify_center().text_sm().text_color(theme::muted()).child(t("orqcfg_escolha_papel"));
        };
        let label = |text: String| div().text_sm().font_weight(FontWeight::MEDIUM).child(text);
        let hint = |text: String| div().text_xs().text_color(theme::muted()).whitespace_normal().child(text);
        let field = |title: String, body: AnyElement| div().flex().flex_col().gap(px(6.)).child(label(title)).child(body);
        let select = |pick: &Option<Pick>, title: String, disabled: bool| pick.as_ref().map(|(state, _)| field(title.clone(),
            Select::new(state).disabled(disabled || self.busy).accessibility_label(title).into_any_element()));
        let original = match &sel { Sel::Row(key) => self.original(key).cloned(), Sel::New => None };
        let mut form = div().flex().flex_col().gap_4()
            .child(div().flex().flex_col().gap_1()
                .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(original.as_ref().map_or_else(|| t("orqcfg_novo_papel"), |o| o.role.papel.clone())))
                .child(hint(t("orqcfg_aplica_proxima"))));
        if let Some(original) = &original {
            let lines = self.lines_of(&original.role.papel);
            let rotating = !role.vez.is_empty();
            let tile = |id: &'static str, on: bool, title: String, help: String, locked: bool, cx: &App| choice(id, on, cx).disabled(locked || self.busy)
                .flex_1().h_auto().py(px(8.)).child(div().flex().flex_col().items_start().gap(px(2.))
                    .child(div().font_weight(FontWeight::MEDIUM).child(title)).child(div().text_xs().text_color(theme::muted()).whitespace_normal().child(help)));
            // "Uma conta" trava com a fila cheia: remover conta é ação explícita pelo ✕, não efeito colateral de um clique.
            let locked = lines.len() > 1;
            form = form.child(field(t("orqcfg_modo_titulo"), div().flex().gap_2()
                .child(tile("orq-mode-single", !rotating, t("orqcfg_modo_unica"),
                    t(if locked { "orqcfg_modo_unica_travada" } else { "orqcfg_modo_unica_ajuda" }), locked, cx)
                    .on_click(cx.listener(|this, _, _, cx| this.set_rotation(false, cx))))
                .child(tile("orq-mode-rotate", rotating, t("orqcfg_modo_reveza"), t("orqcfg_modo_reveza_ajuda"), false, cx)
                    .on_click(cx.listener(|this, _, _, cx| this.set_rotation(true, cx))))
                .into_any_element()));
            if rotating {
                let current_key = original.role.key();
                let queue = div().flex().flex_col().gap_1().children(lines.iter().enumerate().map(|(n, line)| {
                    let key = line.role.key();
                    let r = &line.role;
                    let text = [Some(provider_name(if r.provider.is_empty() { "claude" } else { &r.provider }).to_owned()),
                        Some(if r.conta.is_empty() { t("orqcfg_fila_sem_conta") } else { r.conta.clone() }),
                        Some(r.modelo.clone()).filter(|m| !m.is_empty()), Some(r.esforco.clone()).filter(|e| !e.is_empty())]
                        .into_iter().flatten().collect::<Vec<_>>().join(" · ");
                    let (k_open, k_remove) = (key.clone(), key.clone());
                    div().flex().items_center().gap_2()
                        .child(div().w(px(18.)).text_xs().text_color(theme::muted()).child((n + 1).to_string()))
                        .child(Button::new(SharedString::from(format!("orq-queue-{key}"))).ghost().small().flex_1().min_w_0().justify_start()
                            .selected(key == current_key).label(text)
                            .on_click(cx.listener(move |this, _, window, cx| this.choose(Sel::Row(k_open.clone()), window, cx))))
                        .child(chrome::icon_button(SharedString::from(format!("orq-queue-remove-{key}")), IconName::Close, t("orqcfg_fila_remover"), cx)
                            .disabled(self.busy).on_click(cx.listener(move |this, _, window, cx| this.remove_line(k_remove.clone(), window, cx))))
                })).child(div().child(Button::new("orq-queue-add").ghost().small().icon(IconName::Plus).label(t("orqcfg_fila_adicionar"))
                    .disabled(self.busy).on_click(cx.listener(|this, _, window, cx| this.add_account(window, cx)))));
                form = form.child(field(t("orqcfg_fila_titulo"), queue.into_any_element()));
            }
        } else {
            let taken: Vec<String> = self.rows().iter().map(|r| r.role.papel.to_lowercase()).collect();
            let names = div().flex().flex_wrap().gap(px(6.)).children(CANONICAL.iter().filter(|n| !taken.contains(&n.to_string())).map(|name| {
                let on = !self.other && role.papel == *name;
                choice(SharedString::from(format!("orq-papel-{name}")), on, cx).small().label(name.to_string())
                    .on_click(cx.listener(move |this, _, _, cx| { this.other = false; this.edit(|r| r.papel = name.to_string()); cx.notify(); }))
            })).child(choice("orq-papel-other", self.other, cx).small().label(t("orqcfg_papel_outro"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.other = true;
                    this.edit(|r| r.papel.clear());
                    this.papel_input.update(cx, |input, cx| { input.set_value("", window, cx); input.focus(window, cx); });
                    cx.notify();
                })));
            form = form.child(field(t("orqcfg_papel"), div().flex().flex_col().gap_2().child(names)
                .when(self.other, |el| el.child(Input::new(&self.papel_input).disabled(self.busy).aria_label(t("orqcfg_papel")))).into_any_element()));
        }
        let providers = div().id("orq-providers").role(A11y::Group).aria_label(t("criar_provider_aria")).flex().flex_wrap().gap(px(6.))
            .children(PROVIDERS.iter().map(|&p| choice(SharedString::from(format!("orq-provider-{p}")), role.provider == p, cx).small()
                .disabled(self.allowed(p).is_empty() || self.busy)
                .child(div().flex().items_center().gap(px(6.)).child(chrome::provider_glyph(p, 14.)).child(provider_name(p)))
                .on_click(cx.listener(move |this, _, window, cx| this.set_provider(p, window, cx)))));
        form = form.child(field(t("comum_provider"), providers.into_any_element()));
        form = form.child(match select(&self.picks.conta, t("orqcfg_conta"), false) {
            Some(el) => el,
            None => field(t("orqcfg_conta"), div().id("orq-no-account").role(A11y::Status).child(hint(t("orqcfg_nenhuma_conta"))).into_any_element()),
        });
        let locked = self.policy_of(&role.provider, &role.conta).is_some_and(|p| !p.trocar);
        let trio: Vec<Div> = [select(&self.picks.modelo, t("composer_modelo"), locked),
            select(&self.picks.esforco, t(if matches!(role.provider.as_str(), "pi" | "omp") { "criar_raciocinio" } else { "composer_esforco" }), false),
            select(&self.picks.permissao, t("criar_permissao"), false)].into_iter().flatten().map(|el| el.flex_1().min_w(px(150.))).collect();
        form = form.child(div().flex().flex_wrap().gap_3().children(trio));
        if matches!(role.provider.as_str(), "claude" | "codex") {
            let mode = |id: &'static str, on: bool, text: String, headless: Option<bool>| choice(id, on, cx).small().label(text)
                .disabled(self.busy).on_click(cx.listener(move |this, _, _, cx| { this.edit(|r| r.headless = headless); this.stale = true; cx.notify(); }));
            form = form.child(field(t("criar_modo_exec"), div().flex().gap(px(6.))
                .child(mode("orq-exec-default", role.headless.is_none(), t("criar_padrao"), None))
                .child(mode("orq-exec-tmux", role.headless == Some(false), t("criar_modo_exec_tmux"), Some(false)))
                .child(mode("orq-exec-headless", role.headless == Some(true), t("criar_modo_exec_headless"), Some(true))).into_any_element()));
        }
        form = form.children(select(&self.picks.motor, t("comum_motor"), false)).children(select(&self.picks.subagente, t("criar_subagente"), false));
        if role.provider == "omp" {
            form = form.child(field(t("criar_perfil_omp"), Input::new(&self.perfil_input).font_family(theme::MONO).disabled(self.busy).aria_label(t("criar_perfil_omp")).into_any_element()));
        }
        // Já ligado sem chave no servidor: o interruptor fica na tela, senão não haveria como desligar.
        if self.jev_key || role.jev {
            form = form.child(Checkbox::new("orq-jev").label(t("criar_jev")).checked(role.jev).disabled(self.busy)
                .on_click(cx.listener(|this, on: &bool, _, cx| { let on = *on; this.edit(|r| r.jev = on); cx.notify(); })));
        }
        let quota = self.rows().iter().find(|r| r.role.provider == role.provider && r.role.conta == role.conta).and_then(|r| r.id_cota.clone());
        let quota = self.quota(&role.provider, &role.conta, quota.as_deref()).cloned();
        form = form.child(div().flex().flex_wrap().gap_3()
            .children(select(&self.picks.janela, t("orqcfg_janela"), false).map(|el| el.flex_1().min_w(px(150.)).child(hint(t("orqcfg_janela_ajuda")))))
            .child(field(t("orqcfg_cota"), div().text_sm().text_color(if quota.as_ref().is_some_and(|q| q.0 >= QUOTA_HIGH) { theme::warning() } else { theme::muted() })
                .child(quota.map_or_else(|| t("orqcfg_cota_sem"), |(pct, window)| tr_shared("orqcfg_cota_usada", &[("pct", &pct.round().to_string()), ("janela", &window)])))
                .into_any_element()).flex_1().min_w(px(150.))));
        let live = original.as_ref().and_then(|o| live_session(&o.role.sessao, sessions).map(|s| (o.role.clone().filled(), Live::of(&o.role.clone().filled(), s))));
        form.when_some(live, |el, (contract, live)| {
            let text = if live.model.is_some() || live.effort.is_some() {
                let mut text = tr_shared("orqcfg_agora_viva", &[("sessao", &live.name), ("modelo", live.model.as_deref().unwrap_or("—")),
                    ("esforco", live.effort.as_deref().unwrap_or("—"))]);
                if live.differs() {
                    text += " ";
                    text += &tr_shared("orqcfg_agora_diverge", &[("modelo", if contract.modelo.is_empty() { "—" } else { &contract.modelo }),
                        ("esforco", if contract.esforco.is_empty() { "—" } else { &contract.esforco })]);
                }
                text
            } else { t("orqcfg_agora_nao_medido") };
            el.child(div().p_3().rounded(px(8.)).bg(theme::inset()).text_sm().whitespace_normal()
                .text_color(if live.differs() { theme::warning() } else { theme::muted() }).child(text))
        })
    }

    fn render_footer(&self, group: Option<&Group>, cx: &mut Context<Self>) -> Option<Div> {
        let pending = self.pending();
        if pending.is_empty() && self.error.is_none() && self.notice.is_none() && !self.conflict { return None; }
        let status = if self.conflict {
            Some(div().id("orq-conflict").role(A11y::Alert).flex().items_center().gap_2().text_sm().text_color(theme::danger())
                .child(t("orqcfg_arquivo_mudou"))
                .child(Button::new("orq-reload").outline().xsmall().label(t("orqcfg_recarregar"))
                    .on_click(cx.listener(|this, _, window, cx| { this.conflict = false; this.load(window, cx); }))))
        } else if let Some(error) = &self.error {
            Some(div().id("orq-error").role(A11y::Alert).text_sm().text_color(theme::danger()).whitespace_normal().child(error.clone()))
        } else {
            self.notice.clone().filter(|_| pending.is_empty())
                .map(|text| div().id("orq-notice").role(A11y::Status).text_sm().text_color(theme::success()).child(text))
        };
        let file = group.map(|g| g.arquivo.rsplit('/').next().unwrap_or_default().to_owned()).unwrap_or_default();
        let changes = (!pending.is_empty()).then(|| div().flex().flex_col().gap_2()
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(if pending.len() == 1 { t("orqcfg_mudanca_1") }
                else { t1("orqcfg_mudancas_n", "n", &pending.len().to_string()) }))
            .child(div().id("orq-pending").max_h(px(96.)).overflow_y_scroll().flex().flex_col().gap(px(2.)).text_xs().text_color(theme::muted())
                .children(pending.into_iter().map(|(papel, text)| div().whitespace_normal()
                    .child(div().flex().gap_1().child(div().font_weight(FontWeight::SEMIBOLD).text_color(theme::text()).child(papel)).child(text)))))
            .child(div().flex().items_center().gap_2()
                .child(div().flex_1().min_w_0().text_xs().text_color(theme::faint()).truncate().child(t1("orqcfg_rodape_papel", "arquivo", &file)))
                .child(Button::new("orq-discard").ghost().small().label(t("orqcfg_descartar")).disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.drafts.clear();
                        // Os campos de texto também voltam ao que o contrato tem.
                        match this.sel.clone() { Some(sel) => this.choose(sel, window, cx), None => this.stale = true }
                        cx.notify();
                    })))
                .child(Button::new("orq-save-next").outline().small().label(t("orqcfg_salvar_continuar")).disabled(self.busy || self.conflict)
                    .on_click(cx.listener(|this, _, window, cx| this.save(true, window, cx))))
                .child(Button::new("orq-save").primary().small().label(t(if self.busy { "orqcfg_salvando" } else { "orqcfg_salvar" }))
                    .loading(self.busy).disabled(self.busy || self.conflict)
                    .on_click(cx.listener(|this, _, window, cx| this.save(false, window, cx))))));
        Some(div().pt_3().border_t_1().border_color(theme::border()).flex().flex_col().gap_2().children(status).children(changes))
    }
}

impl Render for OrqRoles {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.stale { self.build_picks(None, window, cx); }
        let sessions = self.sessions(cx);
        let header = |gid: Option<String>, cx: &mut Context<Self>| div().flex().items_center().gap_2()
            .child(div().font_weight(FontWeight::SEMIBOLD).child(t("orqcfg_aba_papeis")))
            .when_some(gid, |el, gid| el.child(super::server_config::chip(t1("orqcfg_sub_grupo", "gid", &gid), theme::muted(), theme::raised())))
            .child(div().flex_1())
            .child(Button::new("orq-accounts").ghost().small().icon(IconName::Users).label(t("orqcfg_aba_contas"))
                .on_click(cx.listener(|this, _, window, cx| this.open_accounts(window, cx))));
        let body = match &self.group {
            None => div().id("orq-loading").role(A11y::Status).aria_label(t("orqcfg_carregando")).flex().flex_col().gap_2()
                .children((0..4usize).map(|i| chrome::Skeleton::new(("orq-skeleton", i)).h(px(44.)).w_full().rounded(px(8.))))
                .into_any_element(),
            Some(Err(error)) => div().flex().flex_col().items_start().gap_2()
                .child(div().id("orq-load-error").role(A11y::Alert).text_sm().text_color(theme::danger()).whitespace_normal().child(error.clone()))
                .child(Button::new("orq-retry").outline().small().label(t("orqcfg_recarregar"))
                    .on_click(cx.listener(|this, _, window, cx| { this.group = None; this.load(window, cx); })))
                .into_any_element(),
            Some(Ok(group)) => div().flex().gap_5().h(px(560.))
                .child(div().id("orq-list").w(px(430.)).flex_shrink_0().h_full().overflow_y_scroll().pr_2().child(self.render_list(group, &sessions, cx)))
                .child(div().id("orq-form").flex_1().min_w_0().h_full().overflow_y_scroll().pl_5().border_l_1().border_color(theme::border())
                    .child(self.render_form(&sessions, cx)))
                .into_any_element(),
        };
        let group = self.group.as_ref().and_then(|g| g.as_ref().ok());
        let gid = group.filter(|g| g.has_group()).map(|g| g.gid.clone());
        let footer = self.render_footer(group, cx);
        div().w_full().flex().flex_col().gap_3().child(header(gid, cx)).child(body).children(footer)
    }
}

impl Hangar {
    /// Time do trabalho da sessão aberta; as contas liberadas continuam em Configurações.
    pub(super) fn open_orq_roles(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(api), Some(key)) = (self.session_api(), self.selected_key()) else { return };
        let (runtime, hangar) = (self.runtime.clone(), cx.entity());
        let panel = cx.new(|cx| OrqRoles::new(api, runtime, key.name.clone(), hangar, window, cx));
        window.open_dialog(cx, move |dialog, _, _| popup::dialog(dialog).w(px(1040.)).title(t("orqcfg_titulo")).child(panel.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::{Row, Role, family, lane, model_label, stages};

    fn row(papel: &str, vez: &str) -> Row { Row { role: Role { papel: papel.into(), vez: vez.into(), ..Role::default() }, id_cota: None } }

    #[test]
    fn stages_follow_the_work_order_and_join_lanes_and_rotation() {
        let rows = [row("revisor", ""), row("executor faixa A", ""), row("mock", ""), row("árbitro", ""), row("executor faixa b", ""),
            row("revisor", "2")];
        let list: Vec<(String, usize)> = stages(&rows).into_iter().map(|s| (s.base, s.rows.len())).collect();
        assert_eq!(list, [("árbitro".into(), 1), ("executor".into(), 2), ("revisor".into(), 2), ("mock".into(), 1)]);
        assert_eq!(lane("executor faixa b"), ("executor".into(), Some("B".into())));
        assert_eq!(lane("faixa A"), ("faixa A".into(), None));
    }

    #[test]
    fn model_labels_and_families_read_like_the_web() {
        assert_eq!(model_label("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(model_label("opus[1m]"), "Opus 1M");
        assert_eq!(model_label("gpt-6"), "gpt-6");
        assert_eq!(family("opus[1m]"), family("Opus4.8·1M"));
        assert_ne!(family("opus"), family("opus[1m]"));
        assert_eq!(family("apikey/k3").map(|f| f.0), Some("k".into()));
    }

    #[test]
    fn opening_fields_only_keep_what_the_provider_accepts() {
        let role = Role { provider: "codex".into(), motor: "x".into(), permissao: "auto".into(), subagente: "s".into(), perfil: "p".into(),
            ..Role::default() }.normalized();
        assert_eq!((role.motor.as_str(), role.permissao.as_str(), role.subagente.as_str(), role.perfil.as_str()), ("", "", "", ""));
        let role = Role { provider: "codex".into(), headless: Some(true), permissao: "Full Access".into(), ..Role::default() }.normalized();
        assert_eq!(role.permissao, "Full Access");
    }
}
