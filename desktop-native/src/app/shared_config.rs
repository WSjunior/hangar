//! Configuração compartilhada: leva a configuração do Claude Code e do Codex de uma máquina para outras. A tela fala com
//! várias máquinas ao mesmo tempo, com o token de cada uma; o pacote sai da origem e vai, em sequência, para cada destino.
//! A prévia e as linhas são as do `configSync.ts` do web, refeitas aqui.
use super::*;
use super::server_config::chip;
use super::settings::{settings_box, Disclosure, Page};
use gpui_kit::component::progress::Progress;
use serde::Deserialize;
use std::collections::BTreeMap;

const ITEMS: [&str; 13] = ["claude_instructions", "claude_skills", "claude_agents", "claude_hooks", "claude_plugins",
    "claude_mcp", "claude_env", "claude_settings", "codex", "engines", "hangar_prefs", "claude_accounts", "hangar_appearance"];

fn item_label(item: &str) -> String { tr_shared(&format!("shared_config_item_{item}"), &[]) }

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
struct ManifestItem { hashes: BTreeMap<String, String>, labels: HashMap<String, Vec<String>>, descriptions: BTreeMap<String, String> }

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub(super) struct Manifest { items: HashMap<String, ManifestItem> }

#[derive(Clone, Default, Deserialize)]
struct Warning { code: String, #[serde(default)] params: HashMap<String, Value> }

#[derive(Clone, Deserialize)]
struct ItemResult { status: String, #[serde(default)] changed: Vec<String>, #[serde(default)] warnings: Vec<Warning> }

#[derive(Clone, Deserialize)]
pub(super) struct Report { #[serde(default)] items: HashMap<String, ItemResult>, #[serde(default)] backup: Option<String> }

/// Por item: o que a origem tem e o destino não, o que difere, o que já é igual e o que só existe no destino.
#[derive(Clone, Debug, Default, PartialEq)]
struct Diff { added: Vec<String>, changed: Vec<String>, same: Vec<String>, only_target: Vec<String> }

fn diff_manifests(origin: &Manifest, target: &Manifest, items: &[&'static str]) -> HashMap<&'static str, Diff> {
    let empty = BTreeMap::new();
    items.iter().map(|&item| {
        let a = origin.items.get(item).map_or(&empty, |i| &i.hashes);
        let b = target.items.get(item).map_or(&empty, |i| &i.hashes);
        let mut diff = Diff::default();
        for (name, hash) in a {
            match b.get(name) { None => diff.added.push(name.clone()), Some(h) if h != hash => diff.changed.push(name.clone()), _ => diff.same.push(name.clone()) }
        }
        diff.only_target = b.keys().filter(|name| !a.contains_key(*name)).cloned().collect();
        (item, diff)
    }).collect()
}

/// Ordem = a do web: o que o envio muda vem primeiro.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Status { Added, Changed, Same, OnlyTarget }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group { Settings, Files, Entries, Refs }

#[derive(Clone, Debug)]
struct Row { key: String, name: String, group: Group, status: Status, description: String, scripts: Vec<(String, String)>, selectable: bool }

/// `⟦HOME⟧/x` como a pessoa escreveria: `~/x`.
fn sync_path(marked: &str) -> String {
    let (open, close) = ('⟦', '⟧');
    let (mut out, mut rest) = (String::new(), marked);
    while let Some(start) = rest.find(open) {
        out.push_str(&rest[..start]);
        let after = &rest[start + open.len_utf8()..];
        match after.find(close) {
            Some(end) if end > 0 && after[..end].bytes().all(|b| b.is_ascii_uppercase()) => {
                let name = &after[..end];
                out.push_str(match name { "HOME" => "~", "CLAUDE" => "~/.claude", "CODEX" => "~/.codex", "HANGAR" => "hangar",
                    _ => &rest[start..start + open.len_utf8() + end + close.len_utf8()] });
                rest = &after[end + close.len_utf8()..];
            }
            _ => { out.push(open); rest = after; }
        }
    }
    out.push_str(rest);
    out
}

fn row_of(item: &str, key: &str) -> (String, Group) {
    if let Some(path) = key.strip_prefix("ref:") { return (sync_path(path), Group::Refs); }
    if item == "claude_skills" { return (key.strip_prefix("skills/").unwrap_or(key).to_owned(), Group::Entries); }
    if item != "claude_hooks" { return (key.to_owned(), Group::Entries); }
    if key == "statusLine" { return (key.to_owned(), Group::Settings); }
    if let Some(event) = key.strip_prefix("hooks:") { return (event.to_owned(), Group::Settings); }
    (key.strip_prefix("hooks/").unwrap_or(key).to_owned(), Group::Files)
}

/// Uma linha por entrada, juntando os destinos: basta um destino sem a entrada para ela ser "novo".
fn sync_rows(item: &str, diffs: &[&Diff], labels: &HashMap<String, Vec<String>>, descriptions: &BTreeMap<String, String>) -> Vec<Row> {
    let script_text = |name: &str| descriptions.iter().find(|(k, _)| k.ends_with(&format!("/{name}"))).map(|(_, v)| v.clone()).unwrap_or_default();
    let mut status: HashMap<String, Status> = HashMap::new();
    for d in diffs {
        for (list, s) in [(&d.added, Status::Added), (&d.changed, Status::Changed), (&d.same, Status::Same)] {
            for key in list { let now = status.entry(key.clone()).or_insert(s); if s < *now { *now = s; } }
        }
    }
    for d in diffs { for key in &d.only_target { status.entry(key.clone()).or_insert(Status::OnlyTarget); } }
    let mut rows: Vec<Row> = status.into_iter().map(|(key, status)| {
        let (name, group) = row_of(item, &key);
        let scripts = labels.get(&key).map(|names| names.iter().map(|n| (n.clone(), script_text(n))).collect()).unwrap_or_default();
        Row { description: descriptions.get(&key).cloned().unwrap_or_default(), key, name, group, status, scripts, selectable: status != Status::OnlyTarget }
    }).collect();
    rows.sort_by(|a, b| a.status.cmp(&b.status).then_with(|| collation(&a.name).cmp(&collation(&b.name))));
    rows
}

/// Perto do `localeCompare` do web: sem caixa, e pontuação antes de dígito antes de letra.
fn collation(name: &str) -> Vec<(u8, char)> {
    name.chars().flat_map(char::to_lowercase).map(|c| (if c.is_alphabetic() { 2 } else if c.is_numeric() { 1 } else { 0 }, c)).collect()
}

/// Aviso do servidor pela frase do web; código desconhecido aparece cru.
fn warning_text(w: &Warning) -> String {
    const PARAMS: &[(&str, &[&str])] = &[
        ("config_sync_heavy_dir_skipped", &["entry", "dir"]), ("config_sync_unreadable", &["entry", "error"]),
        ("config_sync_broken_link", &["entry"]), ("config_sync_invalid_json", &["file"]), ("config_sync_item_failed", &["error"]),
        ("config_sync_marketplace_unknown", &["marketplace"]), ("config_sync_marketplace_failed", &["marketplace", "detail"]),
        ("config_sync_plugin_failed", &["plugin", "detail"]), ("config_sync_missing_program", &["program", "where"]),
        ("config_sync_hangar_outdated", &["file"]), ("config_sync_hangar_skill_kept", &["entry"]),
        ("config_sync_hook_replaced", &["event", "command"]), ("config_sync_invalid_entry", &["entry"]),
        ("config_sync_pref_rejected", &["key", "error"]), ("config_sync_after_failed", &["step", "error"]),
        ("config_sync_item_missing", &[]), ("config_sync_unsupported_value", &["key"]), ("config_sync_link_replaced", &["entry", "target"]),
        ("config_sync_account_needs_login", &["account"]), ("config_sync_account_not_hangar", &["account"]),
        ("config_sync_account_failed", &["account", "error"]),
    ];
    let Some((_, names)) = PARAMS.iter().find(|(code, _)| *code == w.code) else { return w.code.clone() };
    let params = names.iter().map(|&n| (n.to_owned(), w.params.get(n).map_or_else(String::new, |v| v.as_str().map_or_else(|| v.to_string(), str::to_owned))))
        .collect();
    crate::i18n::tr_web(&w.code, &params).unwrap_or_else(|| w.code.clone())
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) enum Stage { #[default] Waiting, Connecting, Read, ReadDone, Packing, Packed, Uploading, Apply, Install, After, Applied, Partial, Failed }

/// Onde uma máquina está. `no_detail`: Hangar antigo, que só devolve o resultado no fim. `failed`: itens com falha
/// num relatório que terminou (`Partial`).
#[derive(Clone, Default)]
struct Step { stage: Stage, item: Option<String>, entry: Option<String>, index: Option<u64>, total: Option<u64>, no_detail: bool, error: Option<String>, failed: usize }

impl Step {
    fn at(stage: Stage) -> Self { Self { stage, ..Self::default() } }

    /// Linha `progress`: `index`/`total` só vêm na troca de item; nas de entrada ficam os anteriores.
    fn advance(&mut self, event: &Value) {
        let text = |k: &str| event.get(k).and_then(Value::as_str).map(str::to_owned);
        let stage = match event.get("phase").and_then(Value::as_str) {
            Some("read") => Stage::Read, Some("apply") => Stage::Apply, Some("install") => Stage::Install, Some("after") => Stage::After, _ => return,
        };
        let entry = text("entry");
        let total = event.get("total").and_then(Value::as_u64);
        if entry.is_none() && total.is_some() { (self.index, self.total) = (event.get("index").and_then(Value::as_u64), total); }
        // Os passos finais não são itens: contador e barra do último item ficariam parados.
        if stage == Stage::After { (self.index, self.total) = (None, None); }
        self.item = text("item").or(self.item.take());
        (self.stage, self.entry, self.no_detail) = (stage, entry, false);
    }

    fn text(&self) -> String {
        let item = self.item.as_deref().map(item_label).unwrap_or_default();
        let entry = self.entry.clone().unwrap_or_default();
        match self.stage {
            _ if self.no_detail => tr_shared("shared_config_step_no_detail", &[]),
            Stage::Waiting => tr_shared("shared_config_step_waiting", &[]),
            Stage::Connecting => tr_shared("shared_config_step_connecting", &[]),
            Stage::Read => tr_shared("shared_config_step_read", &[("item", &item)]),
            Stage::ReadDone => tr_shared("shared_config_step_read_done", &[]),
            Stage::Packing => tr_shared("shared_config_step_packing", &[]),
            Stage::Packed => tr_shared("shared_config_step_packed", &[]),
            Stage::Uploading => tr_shared("shared_config_step_uploading", &[]),
            Stage::Apply => tr_shared("shared_config_step_apply", &[("item", &item)]),
            Stage::Install => tr_shared("shared_config_step_install", &[("entry", &entry)]),
            Stage::After => match entry.as_str() {
                "skill_bridge" | "hangar_hooks" | "codex_integration" => tr_shared(&format!("shared_config_step_after_{entry}"), &[]),
                _ => tr_shared("shared_config_step_after_other", &[("step", &entry)]),
            },
            Stage::Applied => tr_shared("shared_config_step_applied", &[]),
            Stage::Partial => tr_shared("shared_config_step_partial", &[("count", &self.failed.to_string())]),
            Stage::Failed => tr_shared("shared_config_step_failed", &[]),
        }
    }

    /// Etapa em curso: as finais não mostram contador nem entrada.
    fn running(&self) -> bool { matches!(self.stage, Stage::Read | Stage::Apply | Stage::Install | Stage::After) }
}

struct Machine { id: String, label: String, origin: bool, step: Step }

struct Preview { item: &'static str, changes: Vec<Row>, same: Vec<Row>, only_target: Vec<Row>, line: String }

pub(in crate::app) struct SharedConfig {
    origin: Option<String>,
    target_ids: Vec<String>,
    items: Vec<&'static str>,
    busy: bool,
    /// Número da comparação: resposta (etapa, tradução) de outra é descartada.
    run: u64,
    base: Option<Manifest>,
    diffs: HashMap<String, Result<HashMap<&'static str, Diff>, String>>,
    reports: HashMap<String, Result<Report, String>>,
    /// Entradas marcadas por item da prévia. Sem comparação não há escolha: o item vai inteiro.
    picked: HashMap<&'static str, Vec<String>>,
    /// Descrição original → no idioma da tela.
    translated: HashMap<String, String>,
    translating: bool,
    translate_error: Option<String>,
    preview: Vec<Preview>,
    open: HashMap<&'static str, bool>,
    rest_open: HashSet<(&'static str, bool)>,
    progress: Vec<Machine>,
}

impl Default for SharedConfig {
    fn default() -> Self {
        Self { origin: None, target_ids: Vec::new(), items: ITEMS.to_vec(), busy: false, run: 0, base: None, diffs: HashMap::new(),
            reports: HashMap::new(), picked: HashMap::new(), translated: HashMap::new(), translating: false, translate_error: None,
            preview: Vec::new(), open: HashMap::new(), rest_open: HashSet::new(), progress: Vec::new() }
    }
}

pub(super) enum StepEvent { Stage(Stage), Progress(Value), NoDetail }

pub(super) enum SharedConfigReply {
    Step(u64, String, StepEvent),
    Compared(u64, Result<Manifest, Failure>, Vec<(String, Result<Manifest, Failure>)>),
    Packed(u64, Result<(), Failure>),
    Applied(u64, String, Result<Report, Failure>),
    Finished(u64),
    Translated(u64, Vec<String>, Result<Value, Failure>),
}

#[derive(Clone)]
struct Out { tx: async_channel::Sender<Envelope>, connection: u64 }

impl Out {
    async fn send(&self, reply: SharedConfigReply) {
        let _ = self.tx.send(Envelope { connection: self.connection, selection: None, payload: Payload::SharedConfig(reply) }).await;
    }

    /// Repassa as linhas `progress` de uma máquina como etapas dela.
    fn steps(&self, run: u64, id: String) -> impl FnMut(Option<Value>) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>> + use<> {
        let out = self.clone();
        move |event| {
            let (out, id) = (out.clone(), id.clone());
            Box::pin(async move { out.send(SharedConfigReply::Step(run, id, event.map_or(StepEvent::NoDetail, StepEvent::Progress))).await })
        }
    }
}

/// O motivo, sem o nome da máquina; 404 na rota = Hangar de lá ainda sem a feature.
fn failure_text(error: &Failure, label: &str) -> String {
    if error.status == Some(404) { return tr_shared("shared_config_outdated", &[("machine", label)]); }
    if error.detail == "shared_config_stream_cut" { return tr_shared("shared_config_stream_cut", &[]); }
    Hangar::fetch_failure(error)
}

/// A frase do web: "máquina: motivo", ou a de Hangar desatualizado.
fn machine_error(error: &Failure, label: &str) -> String {
    let text = failure_text(error, label);
    if error.status == Some(404) { text } else { tr_shared("shared_config_error", &[("machine", label), ("error", &text)]) }
}

fn parse<T: serde::de::DeserializeOwned>(value: Result<Value, Failure>) -> Result<T, Failure> {
    value.and_then(|v| serde_json::from_value(v).map_err(|_| Failure::local("invalid_response")))
}

impl Hangar {
    fn shared_out(&self) -> Out { Out { tx: self.tx.clone(), connection: self.connection } }

    /// Máquinas próprias: convite e desligada ficam fora, como no seletor do grupo Servidor.
    fn own_servers(&self) -> Vec<ServerEntry> { self.servers.iter().filter(|s| !s.invite && !s.disabled).cloned().collect() }

    /// Origem e destinos escolhidos que ainda existem; o destino nunca é a origem.
    fn shared_machines(&self) -> (Option<ServerEntry>, Vec<ServerEntry>) {
        let own = self.own_servers();
        let origin = own.iter().find(|s| Some(&s.id) == self.shared.origin.as_ref()).cloned();
        let targets = own.into_iter().filter(|s| Some(&s.id) != self.shared.origin.as_ref() && self.shared.target_ids.contains(&s.id)).collect();
        (origin, targets)
    }

    fn shared_ready(&self) -> bool {
        let (origin, targets) = self.shared_machines();
        origin.is_some() && !targets.is_empty() && !self.shared.items.is_empty()
    }

    /// Começa na máquina conectada, como o web começa no servidor do modal.
    pub(super) fn shared_config_opened(&mut self, cx: &mut Context<Self>) {
        let own = self.own_servers();
        if own.iter().any(|s| Some(&s.id) == self.shared.origin.as_ref()) { return; }
        let current = servers::norm(&self.address.read(cx).value());
        self.shared.origin = own.iter().find(|s| servers::norm(&s.address) == current).or(own.first()).map(|s| s.id.clone());
    }

    /// Prévia feita com outra origem ou outros itens decidiria a sobrescrita errada.
    fn clear_shared_results(&mut self) {
        let s = &mut self.shared;
        s.run += 1;
        (s.base, s.translating, s.translate_error) = (None, false, None);
        s.diffs.clear(); s.reports.clear(); s.picked.clear(); s.translated.clear(); s.preview.clear(); s.open.clear(); s.rest_open.clear(); s.progress.clear();
    }

    fn rebuild_shared_preview(&mut self) {
        let (_, targets) = self.shared_machines();
        let s = &mut self.shared;
        let ok: Vec<&HashMap<&'static str, Diff>> = targets.iter().filter_map(|t| s.diffs.get(&t.id)?.as_ref().ok()).collect();
        let empty = ManifestItem::default();
        s.preview = ITEMS.iter().filter(|item| s.items.contains(item) && ok.iter().any(|d| d.contains_key(*item))).filter_map(|&item| {
            let meta = s.base.as_ref().and_then(|b| b.items.get(item)).unwrap_or(&empty);
            let descriptions = meta.descriptions.iter().map(|(k, v)| (k.clone(), s.translated.get(v).cloned().unwrap_or_else(|| v.clone()))).collect();
            let diffs: Vec<&Diff> = ok.iter().filter_map(|d| d.get(item)).collect();
            let rows = sync_rows(item, &diffs, &meta.labels, &descriptions);
            if rows.is_empty() { return None; }
            let count = |st: Status| rows.iter().filter(|r| r.status == st).count().to_string();
            let line = tr_shared("shared_config_diff_line", &[("added", &count(Status::Added)), ("changed", &count(Status::Changed)),
                ("same", &count(Status::Same)), ("onlyTarget", &count(Status::OnlyTarget))]);
            let pick = |st: &[Status]| rows.iter().filter(|r| st.contains(&r.status)).cloned().collect::<Vec<_>>();
            Some(Preview { item, changes: pick(&[Status::Added, Status::Changed]), same: pick(&[Status::Same]), only_target: pick(&[Status::OnlyTarget]), line })
        }).collect();
    }

    fn set_shared_origin(&mut self, id: String, cx: &mut Context<Self>) {
        if self.shared.busy || self.shared.origin.as_ref() == Some(&id) { return; }
        self.shared.origin = Some(id);
        self.clear_shared_results();
        cx.notify();
    }

    fn toggle_shared_target(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        if self.shared.busy { return; }
        let candidates: Vec<String> = self.own_servers().into_iter().filter(|s| Some(&s.id) != self.shared.origin.as_ref()).map(|s| s.id).collect();
        let ids = &mut self.shared.target_ids;
        match id {
            // "Todas as outras": todas marcadas desmarca, senão marca todas.
            None => *ids = if candidates.iter().all(|c| ids.contains(c)) { Vec::new() } else { candidates },
            Some(id) => if let Some(at) = ids.iter().position(|t| *t == id) { ids.remove(at); } else { ids.push(id); },
        }
        self.rebuild_shared_preview();
        cx.notify();
    }

    fn toggle_shared_item(&mut self, item: &'static str, cx: &mut Context<Self>) {
        if self.shared.busy { return; }
        let items = &mut self.shared.items;
        if let Some(at) = items.iter().position(|i| *i == item) { items.remove(at); } else { items.push(item); }
        self.clear_shared_results();
        cx.notify();
    }

    fn toggle_shared_pick(&mut self, item: &'static str, key: String, cx: &mut Context<Self>) {
        if self.shared.busy { return; }
        let list = self.shared.picked.entry(item).or_default();
        if let Some(at) = list.iter().position(|k| *k == key) { list.remove(at); } else { list.push(key); }
        cx.notify();
    }

    fn pick_all_shared(&mut self, item: &'static str, on: bool, cx: &mut Context<Self>) {
        if self.shared.busy { return; }
        let keys = if on { self.shared.preview.iter().find(|p| p.item == item)
            .map(|p| p.changes.iter().filter(|r| r.selectable).map(|r| r.key.clone()).collect()).unwrap_or_default() } else { Vec::new() };
        self.shared.picked.insert(item, keys);
        cx.notify();
    }

    fn compare_shared(&mut self, cx: &mut Context<Self>) {
        let (Some(origin), targets) = self.shared_machines() else { return };
        if self.shared.busy || !self.shared_ready() { return; }
        self.clear_shared_results();
        let run = self.shared.run;
        self.shared.busy = true;
        self.shared.progress = std::iter::once((&origin, true)).chain(targets.iter().map(|t| (t, false)))
            .map(|(s, is_origin)| Machine { id: s.id.clone(), label: s.label.clone(), origin: is_origin, step: Step::at(Stage::Connecting) }).collect();
        let out = self.shared_out();
        self.runtime.spawn(async move {
            let fetch = |entry: ServerEntry| {
                let out = out.clone();
                async move {
                    let result = match Api::new(&entry.address, &entry.token) {
                        Ok(api) => parse(api.config_sync_manifest(out.steps(run, entry.id.clone())).await),
                        Err(error) => Err(error),
                    };
                    (entry.id, result)
                }
            };
            let ((_, from), rest) = futures::join!(fetch(origin), futures::future::join_all(targets.into_iter().map(&fetch)));
            out.send(SharedConfigReply::Compared(run, from, rest)).await;
        });
        cx.notify();
    }

    fn confirm_send_shared(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(origin), targets) = self.shared_machines() else { return };
        if self.shared.busy || !self.shared_ready() { return; }
        let names = targets.iter().map(|t| t.label.as_str()).collect::<Vec<_>>().join(", ");
        let this = cx.entity().downgrade();
        chrome::confirm_alert(window, cx, tr_shared("shared_config_confirm_title", &[]),
            tr_shared("shared_config_confirm_message", &[("origin", &origin.label), ("targets", &names)]),
            tr_shared("shared_config_confirm_label", &[]), ButtonVariant::Primary, move |_, cx| {
                let _ = this.update(cx, |this, cx| this.send_shared(cx));
                true
            });
    }

    /// O pacote sai UMA vez da origem e vai, em sequência, para cada destino.
    fn send_shared(&mut self, cx: &mut Context<Self>) {
        let (Some(origin), targets) = self.shared_machines() else { return };
        if self.shared.busy || !self.shared_ready() { return; }
        let run = self.shared.run;
        self.shared.busy = true;
        self.shared.reports.clear();
        self.shared.progress = std::iter::once(Machine { id: origin.id.clone(), label: origin.label.clone(), origin: true, step: Step::at(Stage::Packing) })
            .chain(targets.iter().map(|t| Machine { id: t.id.clone(), label: t.label.clone(), origin: false, step: Step::at(Stage::Waiting) })).collect();
        let items = ITEMS.iter().filter(|i| self.shared.items.contains(i)).copied().collect::<Vec<_>>().join(",");
        let keys = (!self.shared.picked.is_empty()).then(|| serde_json::to_string(&self.shared.picked).unwrap_or_default());
        let out = self.shared_out();
        self.runtime.spawn(async move {
            let bundle = match Api::new(&origin.address, &origin.token) {
                Ok(api) => api.config_sync_bundle(&items, keys.as_deref()).await,
                Err(error) => Err(error),
            };
            let bundle = match bundle {
                Ok(bundle) => { out.send(SharedConfigReply::Packed(run, Ok(()))).await; bundle }
                Err(error) => {
                    out.send(SharedConfigReply::Packed(run, Err(error))).await;
                    return out.send(SharedConfigReply::Finished(run)).await;
                }
            };
            for target in targets {
                out.send(SharedConfigReply::Step(run, target.id.clone(), StepEvent::Stage(Stage::Uploading))).await;
                let result = match Api::new(&target.address, &target.token) {
                    Ok(api) => parse(api.config_sync_apply(&items, bundle.clone(), out.steps(run, target.id.clone())).await),
                    Err(error) => Err(error),
                };
                out.send(SharedConfigReply::Applied(run, target.id, result)).await;
            }
            out.send(SharedConfigReply::Finished(run)).await;
        });
        cx.notify();
    }

    /// Descrições no idioma da tela, pela origem. Chegam depois da prévia; até lá vale o original.
    fn translate_shared(&mut self, origin: &ServerEntry) {
        let Some(base) = &self.shared.base else { return };
        let mut texts: Vec<String> = Vec::new();
        for text in base.items.values().flat_map(|i| i.descriptions.values()) { if !texts.contains(text) { texts.push(text.clone()); } }
        if texts.is_empty() { return; }
        self.shared.translating = true;
        let (run, out, entry) = (self.shared.run, self.shared_out(), origin.clone());
        let lang = if crate::i18n::english() { "en" } else { "pt" };
        self.runtime.spawn(async move {
            let result = match Api::new(&entry.address, &entry.token) {
                Ok(api) => api.server_send(reqwest::Method::POST, &["config-sync", "translate"], Some(json!({"texts": texts, "lang": lang})), 300).await,
                Err(error) => Err(error),
            };
            out.send(SharedConfigReply::Translated(run, texts, result)).await;
        });
    }

    fn shared_step(&mut self, id: &str) -> Option<&mut Step> { self.shared.progress.iter_mut().find(|m| m.id == id).map(|m| &mut m.step) }

    fn fail_shared_step(&mut self, id: &str, error: &Failure) {
        let label = self.shared.progress.iter().find(|m| m.id == id).map(|m| m.label.clone()).unwrap_or_default();
        if let Some(step) = self.shared_step(id) { *step = Step { error: Some(failure_text(error, &label)), ..Step::at(Stage::Failed) }; }
    }

    fn shared_label(&self, id: &str) -> String { self.servers.iter().find(|s| s.id == id).map(|s| s.label.clone()).unwrap_or_default() }

    pub(super) fn receive_shared_config(&mut self, reply: SharedConfigReply, cx: &mut Context<Self>) {
        let run = match &reply {
            SharedConfigReply::Step(run, ..) | SharedConfigReply::Compared(run, ..) | SharedConfigReply::Packed(run, _)
            | SharedConfigReply::Applied(run, ..) | SharedConfigReply::Finished(run) | SharedConfigReply::Translated(run, ..) => *run,
        };
        if run != self.shared.run { return; }
        match reply {
            SharedConfigReply::Step(_, id, event) => if let Some(step) = self.shared_step(&id) {
                match event {
                    StepEvent::Stage(stage) => *step = Step::at(stage),
                    StepEvent::Progress(value) => step.advance(&value),
                    StepEvent::NoDetail => step.no_detail = true,
                }
            },
            SharedConfigReply::Compared(_, from, rest) => {
                self.shared.busy = false;
                let origin_id = self.shared.progress.iter().find(|m| m.origin).map(|m| m.id.clone()).unwrap_or_default();
                for (id, result) in std::iter::once((&origin_id, &from)).chain(rest.iter().map(|(id, r)| (id, r))) {
                    match result {
                        Ok(_) => if let Some(step) = self.shared_step(id) { *step = Step::at(Stage::ReadDone); },
                        Err(error) => self.fail_shared_step(id, error),
                    }
                }
                let Ok(from) = from else { cx.notify(); return };
                let items = self.shared.items.clone();
                for (id, result) in rest {
                    let label = self.shared_label(&id);
                    self.shared.diffs.insert(id, result.map(|target| diff_manifests(&from, &target, &items)).map_err(|error| machine_error(&error, &label)));
                }
                self.shared.base = Some(from);
                self.rebuild_shared_preview();
                // Começa marcado o que o envio muda; o igual não precisa ir.
                self.shared.picked = self.shared.preview.iter()
                    .map(|p| (p.item, p.changes.iter().filter(|r| r.selectable).map(|r| r.key.clone()).collect())).collect();
                if let Some(origin) = self.shared_machines().0 { self.translate_shared(&origin); }
            }
            SharedConfigReply::Packed(_, result) => {
                let origin_id = self.shared.progress.iter().find(|m| m.origin).map(|m| m.id.clone()).unwrap_or_default();
                match result {
                    Ok(()) => if let Some(step) = self.shared_step(&origin_id) { *step = Step::at(Stage::Packed); },
                    Err(error) => {
                        // Sem pacote nenhum destino recebe nada: "Na fila" pararia na tela como se ainda fosse.
                        self.shared.progress.retain(|m| m.origin);
                        self.fail_shared_step(&origin_id, &error);
                    }
                }
            }
            SharedConfigReply::Applied(_, id, result) => {
                let label = self.shared_label(&id);
                match &result {
                    // `done` com item falho não é "aplicado".
                    Ok(report) => if let Some(step) = self.shared_step(&id) {
                        let failed = report.items.values().filter(|i| i.status == "failed").count();
                        *step = if failed > 0 { Step { failed, ..Step::at(Stage::Partial) } } else { Step::at(Stage::Applied) };
                    },
                    Err(error) => self.fail_shared_step(&id, error),
                }
                self.shared.reports.insert(id, result.map_err(|error| machine_error(&error, &label)));
            }
            SharedConfigReply::Finished(_) => self.shared.busy = false,
            SharedConfigReply::Translated(_, texts, result) => {
                self.shared.translating = false;
                match result {
                    Ok(value) => {
                        let got = value.get("texts").and_then(Value::as_array);
                        self.shared.translated = texts.iter().enumerate()
                            .map(|(n, t)| (t.clone(), got.and_then(|g| g.get(n)).and_then(Value::as_str).unwrap_or(t).to_owned())).collect();
                        self.shared.translate_error = value.get("error").and_then(Value::as_str).filter(|e| !e.is_empty()).map(str::to_owned);
                    }
                    // 404 = Hangar de lá sem a tradução: os originais já estão na tela, não é erro.
                    Err(error) if error.status == Some(404) => {}
                    Err(error) => self.shared.translate_error = Some(Self::fetch_failure(&error)),
                }
                self.rebuild_shared_preview();
            }
        }
        cx.notify();
    }

    pub(super) fn render_shared_config(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let muted = |text: String| div().text_sm().text_color(theme::muted()).whitespace_normal().child(text);
        let label = |text: String| div().text_sm().font_weight(FontWeight::MEDIUM).child(text);
        let mut page = div().flex().flex_col().gap_4()
            .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child(Page::SharedConfig.title()))
            .child(muted(tr_shared("shared_config_intro", &[])));
        let own = self.own_servers();
        if own.len() < 2 {
            return page.child(div().id("shared-need-two").role(Role::Status).text_sm().whitespace_normal().child(tr_shared("shared_config_need_two", &[])))
                .into_any_element();
        }
        let s = &self.shared;
        let busy = s.busy;
        let ready = self.shared_ready();
        let (origin, targets) = self.shared_machines();

        // Origem: o mesmo menu do seletor de servidor das Configurações.
        let weak = cx.entity().downgrade();
        let current = s.origin.clone();
        let entries = own.clone();
        let dot = |id: &str| div().size(px(7.)).flex_shrink_0().rounded_full().bg(theme::server_color(id));
        let picker = Button::new("shared-origin").outline().small().disabled(busy).accessibility_label(tr_shared("shared_config_origin", &[]))
            .child(div().flex().items_center().gap(px(6.))
                .children(origin.as_ref().map(|o| dot(&o.id)))
                .child(div().truncate().child(origin.as_ref().map(|o| o.label.clone()).unwrap_or_default()))
                .child(chrome::small_icon(IconName::ChevronDown, 12., theme::muted())))
            .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, _| {
                let mut menu = sidebar::menu_style(menu).min_w(px(220.));
                for entry in entries.clone() {
                    let (weak, on, id) = (weak.clone(), current.as_ref() == Some(&entry.id), entry.id.clone());
                    menu = menu.item(PopupMenuItem::element(move |_, _| div().w_full().flex().items_center().gap(px(8.))
                            .child(div().size(px(7.)).flex_shrink_0().rounded_full().bg(theme::server_color(&entry.id)))
                            .child(div().flex_1().min_w_0().truncate().child(entry.label.clone())))
                        .checked(on)
                        .on_click(move |_, _, cx| { let _ = weak.update(cx, |this, cx| this.set_shared_origin(id.clone(), cx)); }));
                }
                menu
            });
        page = page.child(div().flex().flex_col().gap(px(6.)).child(label(tr_shared("shared_config_origin", &[]))).child(div().flex().child(picker)));

        let candidates: Vec<&ServerEntry> = own.iter().filter(|e| s.origin.as_ref() != Some(&e.id)).collect();
        let all = !candidates.is_empty() && candidates.iter().all(|c| s.target_ids.contains(&c.id));
        page = page.child(div().flex().flex_col().gap_2().child(label(tr_shared("shared_config_targets", &[])))
            .child(Checkbox::new("shared-target-all").label(tr_shared("shared_config_all_targets", &[])).checked(all).disabled(busy)
                .on_change(cx.listener(|this, _: &bool, _, cx| this.toggle_shared_target(None, cx))))
            .children(candidates.iter().map(|c| {
                let id = c.id.clone();
                Checkbox::new(SharedString::from(format!("shared-target-{}", c.id))).label(c.label.clone()).checked(s.target_ids.contains(&c.id)).disabled(busy)
                    .on_change(cx.listener(move |this, _: &bool, _, cx| this.toggle_shared_target(Some(id.clone()), cx)))
            })));
        page = page.child(div().flex().flex_col().gap_2().child(label(tr_shared("shared_config_items", &[])))
            .children(ITEMS.iter().map(|&item| Checkbox::new(SharedString::from(format!("shared-item-{item}"))).label(item_label(item))
                .checked(s.items.contains(&item)).disabled(busy)
                .on_change(cx.listener(move |this, _: &bool, _, cx| this.toggle_shared_item(item, cx))))));

        if !ready { page = page.child(muted(tr_shared("shared_config_pick", &[]))); }
        page = page.child(div().flex().gap_2()
            .child(Button::new("shared-compare").outline().small().label(tr_shared("shared_config_compare", &[]))
                .disabled(!ready || busy).on_click(cx.listener(|this, _, _, cx| this.compare_shared(cx))))
            .child(Button::new("shared-send").primary().small().label(tr_shared("shared_config_send", &[]))
                .disabled(!ready || busy).on_click(cx.listener(|this, _, window, cx| this.confirm_send_shared(window, cx)))));

        if !s.progress.is_empty() { page = page.child(self.render_shared_progress()); }
        let s = &self.shared;
        if s.translating { page = page.child(div().id("shared-translating").role(Role::Status).text_xs().text_color(theme::muted()).child(tr_shared("shared_config_translating", &[]))); }
        if let Some(error) = &s.translate_error {
            page = page.child(div().id("shared-translate-error").role(Role::Status).text_xs().text_color(theme::muted()).whitespace_normal()
                .child(tr_shared("shared_config_translate_failed", &[("error", error)])));
        }
        let single = s.preview.len() == 1;
        let previews: Vec<AnyElement> = s.preview.iter().map(|p| self.render_shared_preview(p, single, busy, cx)).collect();
        page = page.children(previews);
        let s = &self.shared;
        let ok_count = targets.iter().filter(|t| matches!(s.diffs.get(&t.id), Some(Ok(_)))).count();
        for target in &targets {
            let (report, diff) = (s.reports.get(&target.id), s.diffs.get(&target.id));
            if report.is_none() && !matches!(diff, Some(Err(_))) && !(matches!(diff, Some(Ok(_))) && ok_count > 1) { continue; }
            let error = |text: &String| div().id(SharedString::from(format!("shared-error-{}", target.id))).role(Role::Alert).text_sm().text_color(theme::danger()).whitespace_normal().child(text.clone());
            let mut card = settings_box().p_3().gap_2().child(label(target.label.clone()));
            card = match (report, diff) {
                (Some(Err(text)), _) => card.child(error(text)),
                (Some(Ok(report)), _) => card.children(ITEMS.iter().filter_map(|&item| report.items.get(item).map(|r| (item, r))).map(|(item, r)| {
                        let (status, color) = match r.status.as_str() {
                            "applied" => ("shared_config_status_applied", theme::muted()),
                            "failed" => ("shared_config_status_failed", theme::danger()),
                            _ => ("shared_config_status_same", theme::muted()),
                        };
                        div().flex().flex_wrap().gap_x(px(12.)).text_sm()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(item_label(item)))
                            .child(div().text_color(color).child(tr_shared(status, &[])))
                            .when(!r.changed.is_empty(), |el| el.child(div().w_full().text_color(theme::faint()).whitespace_normal().child(r.changed.join(", "))))
                            .children(r.warnings.iter().map(|w| div().w_full().text_color(theme::faint()).whitespace_normal().child(warning_text(w))))
                    }))
                    .when_some(report.backup.as_ref().filter(|b| !b.is_empty()), |el, path| el.child(muted(tr_shared("shared_config_backup", &[("path", path)])))),
                (None, Some(Err(text))) => card.child(error(text)),
                (None, Some(Ok(diff))) => card.children(s.items.iter().filter_map(|item| diff.get(item).map(|d| (*item, d))).map(|(item, d)| {
                    let n = |l: &Vec<String>| l.len().to_string();
                    div().flex().flex_wrap().gap_x(px(12.)).text_sm()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(item_label(item)))
                        .child(div().text_color(theme::muted()).child(tr_shared("shared_config_diff_line", &[("added", &n(&d.added)), ("changed", &n(&d.changed)),
                            ("same", &n(&d.same)), ("onlyTarget", &n(&d.only_target))])))
                })),
                _ => card,
            };
            page = page.child(card);
        }
        page.into_any_element()
    }

    fn render_shared_progress(&self) -> Div {
        let rows = self.shared.progress.iter().map(|m| {
            let step = &m.step;
            let failed = matches!(step.stage, Stage::Failed | Stage::Partial);
            let (color, bg) = if m.origin { (theme::accent_text(), theme::accent_dim()) } else { (theme::muted(), theme::raised()) };
            let role = tr_shared(if m.origin { "shared_config_role_origin" } else { "shared_config_role_target" }, &[]);
            let count = step.running().then(|| step.index.zip(step.total)).flatten().filter(|(_, total)| *total > 0);
            div().id(SharedString::from(format!("shared-progress-{}", m.id))).flex().flex_col().gap_1().px_3().py(px(10.))
                .mt(px(-1.)).border_t_1().border_color(theme::border())
                .child(div().flex().items_center().gap_2()
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).truncate().child(m.label.clone()))
                    .child(chip(role, color, bg))
                    .child(div().ml_auto().text_sm().text_color(if failed { theme::danger() } else { theme::muted() }).child(step.text()))
                    .children(count.map(|(index, total)| div().text_xs().text_color(theme::faint())
                        .child(tr_shared("shared_config_step_count", &[("index", &(index + 1).to_string()), ("total", &total.to_string())])))))
                .children(count.map(|(index, total)| Progress::new(SharedString::from(format!("shared-bar-{}", m.id))).xsmall()
                    .accessibility_label(step.text()).value(((index + 1) as f32 * 100. / total as f32).min(100.))))
                // Instalar e finalizar já dizem a entrada na própria etapa.
                .children(step.entry.as_ref().filter(|_| matches!(step.stage, Stage::Read | Stage::Apply))
                    .map(|entry| div().font_family(theme::MONO).text_xs().text_color(theme::faint()).truncate().child(entry.clone())))
                .children(step.error.as_ref().map(|e| div().id(SharedString::from(format!("shared-progress-error-{}", m.id))).role(Role::Alert).text_xs().text_color(theme::danger()).whitespace_normal().child(e.clone())))
        });
        div().flex().flex_col().gap_2()
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(tr_shared("shared_config_progress_title", &[])))
            .child(settings_box().children(rows))
    }

    fn render_shared_preview(&self, p: &Preview, single: bool, busy: bool, cx: &mut Context<Self>) -> AnyElement {
        let item = p.item;
        let choice = self.shared.picked.get(item);
        let open = self.shared.open.get(item).copied().unwrap_or(single && !p.changes.is_empty());
        let selectable = p.changes.iter().filter(|r| r.selectable).count();
        let weak = cx.entity().downgrade();
        let head = div().flex().flex_wrap().items_center().gap_x(px(12.)).gap_y(px(4.))
            .child(Disclosure::new(format!("shared-open-{item}"), open, item_label(item), false)
                .on_change(move |open, cx| { let _ = weak.update(cx, |this, cx| { this.shared.open.insert(item, open); cx.notify(); }); }))
            .child(div().text_sm().text_color(theme::muted()).child(p.line.clone()))
            .children(choice.map(|c| div().text_sm().text_color(theme::accent_text())
                .child(tr_shared("shared_config_picked", &[("picked", &c.len().to_string()), ("total", &selectable.to_string())]))));
        let mut card = settings_box().p_3().gap_3().child(head);
        if !open { return card.into_any_element(); }
        if choice.is_some() && !p.changes.is_empty() {
            card = card.child(div().flex().gap_2()
                .child(Button::new(SharedString::from(format!("shared-all-{item}"))).ghost().xsmall().label(tr_shared("shared_config_pick_all", &[]))
                    .disabled(busy).on_click(cx.listener(move |this, _, _, cx| this.pick_all_shared(item, true, cx))))
                .child(Button::new(SharedString::from(format!("shared-none-{item}"))).ghost().xsmall().label(tr_shared("shared_config_pick_none", &[]))
                    .disabled(busy).on_click(cx.listener(move |this, _, _, cx| this.pick_all_shared(item, false, cx)))));
        }
        for (group, title, hint) in [(Group::Settings, Some("shared_config_group_settings"), Some("shared_config_group_settings_hint")),
            (Group::Files, Some("shared_config_group_files"), None), (Group::Entries, None, None),
            (Group::Refs, Some("shared_config_group_refs"), Some("shared_config_group_refs_hint"))] {
            let rows: Vec<&Row> = p.changes.iter().filter(|r| r.group == group).collect();
            if rows.is_empty() { continue; }
            card = card.child(div().flex().flex_col().gap_1()
                .children(title.map(|t| div().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(theme::faint()).child(tr_shared(t, &[]))))
                .children(hint.filter(|_| choice.is_some()).map(|h| div().text_xs().text_color(theme::faint()).whitespace_normal().child(tr_shared(h, &[]))))
                .children(rows.into_iter().map(|r| {
                    let key = r.key.clone();
                    let (badge, color, bg) = if r.status == Status::Added { ("shared_config_row_added", theme::accent_text(), theme::accent_dim()) }
                        else { ("shared_config_row_changed", theme::warning(), theme::warning().alpha(0.16)) };
                    let body = div().min_w_0().flex().flex_col().gap(px(2.))
                        .child(div().flex().flex_wrap().items_center().gap_2()
                            .child(div().font_family(theme::MONO).text_xs().text_color(theme::text()).child(r.name.clone()))
                            .child(chip(tr_shared(badge, &[]), color, bg)))
                        .when(!r.description.is_empty(), |el| el.child(div().max_w(px(560.)).text_xs().text_color(theme::muted()).line_clamp(2).text_ellipsis().child(r.description.clone())))
                        .children(r.scripts.iter().map(|(name, text)| div().mt_1().pl_3().border_l_1().border_color(theme::border()).flex().flex_col()
                            .child(div().font_family(theme::MONO).text_xs().text_color(theme::text()).child(name.clone()))
                            .when(!text.is_empty(), |el| el.child(div().text_xs().text_color(theme::muted()).line_clamp(2).text_ellipsis().child(text.clone())))));
                    Checkbox::new(SharedString::from(format!("shared-pick-{item}-{}", r.key))).accessibility_label(r.name.clone())
                        .checked(choice.is_some_and(|c| c.contains(&r.key))).disabled(busy || !r.selectable).py_1()
                        .on_change(cx.listener(move |this, _: &bool, _, cx| this.toggle_shared_pick(item, key.clone(), cx)))
                        .child(body)
                })));
        }
        for (only_target, rows, key) in [(false, &p.same, "shared_config_same"), (true, &p.only_target, "shared_config_only_target")] {
            if rows.is_empty() { continue; }
            let open = self.shared.rest_open.contains(&(item, only_target));
            let weak = cx.entity().downgrade();
            card = card.child(div().flex().flex_col().gap_1()
                .child(div().flex().child(Disclosure::new(format!("shared-rest-{item}-{only_target}"), open, tr_shared(key, &[("count", &rows.len().to_string())]), true)
                    .on_change(move |open, cx| { let _ = weak.update(cx, |this, cx| {
                        if open { this.shared.rest_open.insert((item, only_target)); } else { this.shared.rest_open.remove(&(item, only_target)); }
                        cx.notify();
                    }); })))
                .when(open, |el| el.child(div().font_family(theme::MONO).text_xs().text_color(theme::faint()).whitespace_normal()
                    .child(rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(" · ")))));
        }
        card.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    fn manifest(item: &str, hashes: &[(&str, &str)]) -> Manifest {
        let hashes = hashes.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        Manifest { items: HashMap::from([(item.to_owned(), ManifestItem { hashes, ..Default::default() })]) }
    }

    #[test]
    fn preview_rows_match_the_web() {
        let origin = manifest("claude_hooks", &[("hooks:Stop", "a"), ("hooks/x.sh", "b"), ("ref:⟦HOME⟧/bin/y", "c"), ("statusLine", "d")]);
        let one = manifest("claude_hooks", &[("hooks:Stop", "z"), ("statusLine", "d"), ("hooks:Old", "q")]);
        let two = manifest("claude_hooks", &[("hooks:Stop", "a"), ("hooks/x.sh", "b"), ("statusLine", "d")]);
        let (d1, d2) = (diff_manifests(&origin, &one, &["claude_hooks"]), diff_manifests(&origin, &two, &["claude_hooks"]));
        assert_eq!(d1["claude_hooks"].only_target, vec!["hooks:Old".to_owned()]);
        let rows = sync_rows("claude_hooks", &[&d1["claude_hooks"], &d2["claude_hooks"]], &HashMap::new(), &BTreeMap::new());
        let got: Vec<_> = rows.iter().map(|r| (r.name.as_str(), r.group, r.status, r.selectable)).collect();
        // "novo" num destino vence "igual" no outro; o que só existe lá fica por último e não é escolhível.
        assert_eq!(got, vec![("~/bin/y", Group::Refs, Status::Added, true), ("x.sh", Group::Files, Status::Added, true),
            ("Stop", Group::Settings, Status::Changed, true), ("statusLine", Group::Settings, Status::Same, true),
            ("Old", Group::Settings, Status::OnlyTarget, false)]);
        assert_eq!(sync_path("⟦CLAUDE⟧/a ⟦X1⟧ ⟦ODD⟧"), "~/.claude/a ⟦X1⟧ ⟦ODD⟧");
    }
}
