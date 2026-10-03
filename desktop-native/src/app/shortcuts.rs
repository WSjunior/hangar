//! Atalhos: a fileira de ações do painel da sessão, deste servidor (`runtime_config.shortcuts`, JSON numa string; vazio =
//! os cinco nativos). Porta de `packages/core/src/shortcuts.ts` e do editor `ShortcutsSettings.svelte`: cada mudança
//! (formulário, ordem, remover) grava a lista inteira na hora (`POST /api/config`), sem um segundo "Salvar"; o painel
//! lateral lê a mesma resolução, e o que for salvo ou lido aqui aparece lá na hora.
use super::*;
use super::device::Remote;
use super::settings::{section_head, segments, settings_box};
use serde_json::Map;

/// Os botões nativos de hoje, na ordem de hoje. Só "anexos" roda no painel nativo; os outros são preservados na ordem.
pub(super) const NATIVES: [&str; 6] = ["terminal", "modo", "navegador", "anexos", "rodar", "externo"];

/// Glifos curados do web (`ShortcutIcon.svelte`, `GLYPHS`), com o par no Lucide do kit (mesma família de traço).
const GLYPHS: [(&str, IconName); 12] = [("bolt", IconName::Zap), ("play", IconName::Play), ("rocket", IconName::Rocket),
    ("gear", IconName::Settings), ("git", IconName::GitBranch), ("chat", IconName::MessageCircle), ("star", IconName::Star),
    ("folder", IconName::Folder), ("key", IconName::Key), ("terminal", IconName::SquareTerminal), ("globe", IconName::Globe),
    ("robot", IconName::Bot)];

const LABEL_MAX: usize = 24;
const EMOJI_MAX: usize = 4;

/// Um atalho já validado, guardado como o objeto que veio: gravar de volta não perde campo que esta versão não conhece.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Item(Map<String, Value>);

impl Item {
    fn text(&self, key: &str) -> &str { self.0.get(key).and_then(Value::as_str).unwrap_or("") }
    pub(super) fn id(&self) -> &str { self.text("id") }
    pub(super) fn kind(&self) -> &str { self.text("type") }
    pub(super) fn action(&self) -> &str { self.text("action") }
    pub(super) fn label(&self) -> &str { self.text("label") }
    /// Texto do `send_text` ou comando do `shell`.
    pub(super) fn content(&self) -> &str { self.text(if self.kind() == "shell" { "command" } else { "text" }) }
    pub(super) fn icon(&self) -> Option<&str> { self.0.get("icon").and_then(Value::as_str) }
    pub(super) fn confirm(&self) -> bool { self.0.get("confirm") == Some(&Value::Bool(true)) }
    /// Ausente = envia direto; desligado pré-preenche o campo de mensagem.
    pub(super) fn sends_direct(&self) -> bool { self.0.get("send_direct") != Some(&Value::Bool(false)) }
    /// Pasta do `shell` do projeto (absoluta ou relativa à raiz da cópia da sessão); ausente = a pasta da sessão.
    pub(super) fn pasta(&self) -> Option<&str> { self.0.get("pasta").and_then(Value::as_str).map(str::trim).filter(|p| !p.is_empty()) }
    /// `shell` que roda numa cópia só do servidor, fora de qualquer sessão.
    pub(super) fn runs_in_hangar(&self) -> bool { self.kind() == "shell" && self.text("runs_in") == "hangar" }
    /// No Hangar: ausente = roda na home.
    pub(super) fn hangar_home(&self) -> bool { self.0.get("hangar_home") != Some(&Value::Bool(false)) }
    /// Ausente = a pergunta do terminal aparece no app.
    pub(super) fn answer_in_app(&self) -> bool { self.0.get("answer_in_app") != Some(&Value::Bool(false)) }
    /// Interno que também vira bloco em "Ações"; ausente = só no "+" do painel. O Rodar não depende disto.
    pub(super) fn tile(&self) -> bool { self.0.get("tile") == Some(&Value::Bool(true)) }

    fn native(action: &str) -> Self {
        Item(Map::from_iter([("id".into(), json!(action)), ("type".into(), json!("internal")), ("action".into(), json!(action))]))
    }

    /// `isValid` do web: opcional com tipo errado derrubaria quem lê, então o item inteiro sai.
    fn valid(o: &Map<String, Value>) -> bool {
        let filled = |key: &str| o.get(key).and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty());
        let optional = |key: &str, ok: fn(&Value) -> bool| o.get(key).is_none_or(ok);
        if !filled("id") || !optional("icon", Value::is_string) || !optional("confirm", Value::is_boolean)
            || !optional("send_direct", Value::is_boolean) { return false; }
        match o.get("type").and_then(Value::as_str) {
            Some("internal") => o.get("action").and_then(Value::as_str).is_some_and(|a| NATIVES.contains(&a)),
            Some("send_text") => filled("label") && filled("text"),
            Some("shell") => filled("label") && filled("command") && optional("pasta", |p| p.as_str().is_some_and(|p| !p.trim().is_empty()))
                && optional("runs_in", |v| matches!(v.as_str(), Some("session" | "hangar")))
                && optional("hangar_home", Value::is_boolean) && optional("answer_in_app", Value::is_boolean),
            _ => false,
        }
    }
}

pub(super) fn defaults() -> Vec<Item> { NATIVES.map(Item::native).to_vec() }

/// `items` de `GET/PUT /project-shortcuts`, na regra do `mergeProjectShortcuts` do web: só `send_text` e `shell` válidos
/// (interno é do servidor inteiro), id repetido fica o primeiro.
pub(super) fn project_items(items: Option<&Value>) -> Vec<Item> {
    let raw = items.and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    shown_slots(raw).into_iter().filter_map(|n| raw[n].as_object().map(|o| Item(o.clone()))).collect()
}

/// Posições em `raw` dos itens que `project_items` mostra.
fn shown_slots(raw: &[Value]) -> Vec<usize> {
    let mut seen = HashSet::new();
    raw.iter().enumerate().filter_map(|(n, v)| match v {
        Value::Object(o) if Item::valid(o) && o.get("type") != Some(&json!("internal"))
            && seen.insert(o.get("id").and_then(Value::as_str).unwrap_or("").to_owned()) => Some(n),
        _ => None,
    }).collect()
}

/// A lista a gravar: os itens mostrados, já editados, voltam às posições deles em `raw` (novo vai para o fim) e o que a
/// tela escondeu (inválido para esta versão, repetido) fica onde estava.
fn merge_project_items(raw: &[Value], shown: Vec<Item>) -> Vec<Value> {
    let slots = shown_slots(raw);
    let mut shown = shown.into_iter().map(|item| Value::Object(item.0));
    let mut out: Vec<Value> = raw.iter().enumerate()
        .filter_map(|(n, v)| if slots.contains(&n) { shown.next() } else { Some(v.clone()) }).collect();
    out.extend(shown);
    out
}

/// `resolveShortcuts`: nunca falha. Vazio, JSON quebrado ou forma estranha voltam aos nativos; item inválido sai sozinho;
/// id repetido fica o primeiro.
pub(super) fn resolve(raw: &str) -> Vec<Item> {
    if raw.trim().is_empty() { return defaults(); }
    let Ok(Value::Array(data)) = serde_json::from_str::<Value>(raw) else { return defaults() };
    let mut seen = HashSet::new();
    data.into_iter().filter_map(|v| match v { Value::Object(o) if Item::valid(&o) => Some(Item(o)), _ => None })
        .filter(|item| seen.insert(item.id().to_owned())).collect()
}

fn serialize(items: &[Item]) -> String {
    Value::Array(items.iter().map(|item| Value::Object(item.0.clone())).collect()).to_string()
}

/// Ícone da config ("emoji:🚀" | "glifo:bolt" | ausente): glifo desconhecido cai no raio, como no web.
pub(super) enum Glyph { Icon(IconName), Emoji(String) }

pub(super) fn parse_icon(icon: Option<&str>) -> Glyph {
    if let Some(e) = icon.and_then(|i| i.strip_prefix("emoji:")).map(str::trim).filter(|e| !e.is_empty()) { return Glyph::Emoji(e.to_owned()); }
    let name = icon.and_then(|i| i.strip_prefix("glifo:")).unwrap_or("bolt");
    Glyph::Icon(GLYPHS.iter().find(|(g, _)| *g == name).map_or(IconName::Zap, |(_, i)| *i))
}

/// Rodapé colado na caixa: a ação que alimenta a lista mora DENTRO dela, com a mesma divisória das linhas — solta
/// embaixo, ela virava só mais um botão numa pilha que não dizia sobre o que agia.
fn card_footer() -> Div {
    div().mt(px(-1.)).border_t_1().border_color(theme::border()).px(px(16.)).py(px(10.))
        .flex().flex_wrap().items_center().gap(px(8.))
}

pub(super) fn icon_element(icon: Option<&str>, size: f32, color: Hsla) -> AnyElement {
    match parse_icon(icon) {
        Glyph::Icon(name) => chrome::small_icon(name, size, color).into_any_element(),
        Glyph::Emoji(e) => div().text_size(px(size)).line_height(px(size + 2.)).child(e).into_any_element(),
    }
}

/// Rótulo e ícone dos nativos (`INTERNAL_LABEL`/`INTERNAL_ICON` do web): vêm do app, não da config.
fn native_label(action: &str) -> String { tr(&format!("shortcuts_native_{action}")) }

fn native_icon(action: &str) -> &'static str {
    match action { "terminal" | "externo" => "glifo:terminal", "modo" => "glifo:git", "navegador" => "glifo:globe", "anexos" => "glifo:folder", _ => "glifo:play" }
}

/// Até `max` unidades UTF-16, o que o `maxlength` do web conta.
fn clip(text: &str, max: usize) -> Option<String> {
    if text.encode_utf16().count() <= max { return None; }
    let mut out = String::new();
    for c in text.chars() {
        if out.encode_utf16().count() + c.len_utf16() > max { break; }
        out.push(c);
    }
    Some(out)
}

/// `formId` do web para um atalho novo: "a-<tempo base 36>-<4 aleatórios>".
fn new_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let base36 = |mut n: u128| {
        let mut s = String::new();
        loop {
            s.insert(0, char::from_digit((n % 36) as u32, 36).unwrap_or('0'));
            n /= 36;
            if n == 0 { break s; }
        }
    };
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(nanos);
    format!("a-{}-{}", base36(nanos / 1_000_000), base36(u128::from(hasher.finish()) % 1_679_616))
}

/// Formulário de adicionar/editar, aberto abaixo da lista (como o web, sem diálogo).
struct Form {
    /// Id do atalho sendo editado; `None` = novo.
    editing: Option<String>,
    /// Id que o formulário grava: o do editado, ou um novo gerado uma vez só (o reenvio reusa o mesmo).
    id: String,
    /// O objeto gravado do atalho editado.
    original: Option<Map<String, Value>>,
    /// Atalho do projeto da sessão aberta: grava na hora (PUT) e o `shell` ganha a pasta.
    project: bool,
    shell: bool,
    label: Entity<InputState>,
    emoji: Entity<InputState>,
    content: Entity<InputState>,
    folder: Entity<InputState>,
    glyph: String,
    direct: bool,
    confirm: bool,
    /// `shell`: roda No Hangar (uma cópia por servidor) em vez de na sessão; na home, e com a pergunta do terminal no app.
    hangar: bool,
    home: bool,
    ask: bool,
    _subscriptions: Vec<Subscription>,
}

impl Form {
    fn value(input: &Entity<InputState>, cx: &App) -> String { input.read(cx).value().trim().to_owned() }
    /// `formValid`: rótulo e texto/comando preenchidos.
    fn valid(&self, cx: &App) -> bool { !Self::value(&self.label, cx).is_empty() && !Self::value(&self.content, cx).is_empty() }
}

/// O que o formulário confirmou, já lido dos campos.
struct Draft { shell: bool, label: String, content: String, icon: String, direct: bool, confirm: bool, pasta: String, hangar: bool, home: bool, ask: bool }

impl Draft {
    /// Editar parte do objeto gravado: campo que esta versão não conhece continua lá; os do formulário são reescritos.
    /// Só o que difere do padrão é gravado.
    fn into_item(self, original: Option<Map<String, Value>>, id: String) -> Item {
        let mut o = original.unwrap_or_default();
        for key in ["send_direct", "confirm", "pasta", "runs_in", "hangar_home", "answer_in_app"] { o.remove(key); }
        // `verify` é só de atalho de comando; o backend recusa a lista com ele num de texto.
        if !self.shell { o.remove("verify"); }
        if self.shell && !self.pasta.is_empty() { o.insert("pasta".into(), json!(self.pasta)); }
        if self.shell && self.hangar { o.insert("runs_in".into(), json!("hangar")); }
        if self.shell && self.hangar && !self.home { o.insert("hangar_home".into(), json!(false)); }
        if self.shell && !self.ask { o.insert("answer_in_app".into(), json!(false)); }
        o.insert("id".into(), json!(id));
        o.insert("type".into(), json!(if self.shell { "shell" } else { "send_text" }));
        o.insert((if self.shell { "command" } else { "text" }).into(), json!(self.content));
        if !self.shell && !self.direct { o.insert("send_direct".into(), json!(false)); }
        o.insert("label".into(), json!(self.label));
        o.insert("icon".into(), json!(self.icon));
        if self.confirm { o.insert("confirm".into(), json!(true)); }
        Item(o)
    }
}

/// Atalho novo vai para o fim; o editado troca de lugar com ele mesmo. Editado que saiu da lista com o formulário aberto
/// (removido, lista restaurada) não volta pela edição.
fn apply_edit(items: &mut Vec<Item>, editing: Option<&str>, item: Item) {
    match editing {
        Some(id) => if let Some(n) = items.iter().position(|i| i.id() == id) { items[n] = item; },
        // Reenviar um atalho novo depois de uma falha de rede não pode duplicá-lo se a primeira gravação chegou.
        None => match items.iter().position(|i| i.id() == item.id()) { Some(n) => items[n] = item, None => items.push(item) },
    }
}

/// Troca o atalho de lugar com o vizinho `delta` casas adiante; na ponta não muda nada.
fn move_by(items: &mut [Item], id: &str, delta: isize) {
    let Some(i) = items.iter().position(|item| item.id() == id) else { return };
    if let Some(j) = i.checked_add_signed(delta).filter(|j| *j < items.len()) { items.swap(i, j); }
}

/// Atalho sendo arrastado: o id para achar a posição e o rótulo para o que segue o ponteiro.
#[derive(Clone)]
struct Dragged { id: String, label: String }

impl Render for Dragged {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().px_3().py(px(6.)).rounded(px(8.)).border_1().border_color(theme::border_strong()).bg(theme::raised())
            .shadow(theme::popover_shadow()).text_sm().child(self.label.clone())
    }
}

#[derive(Default)]
pub(in crate::app) struct Shortcuts {
    load: Remote<()>,
    items: Vec<Item>,
    saving: bool,
    /// O formulário global espera a gravação: só fecha com ela feita (falhou, fica aberto com o que foi digitado).
    submitting: bool,
    /// Número do salvar cujo "Salvo" está na tela: some 2,5 s depois se nenhum outro o trocou.
    saved: Option<u64>,
    save_seq: u64,
    save_error: Option<String>,
    form: Option<Form>,
    suggestions: Vec<String>,
    suggesting: bool,
    /// Linha que começou o arrasto em curso; só vale enquanto a GPUI tem um arrasto ativo.
    dragging: Option<String>,
    /// Aviso do último exportar/importar (texto, é erro) e a importação esperando confirmação.
    pub(super) transfer_note: Option<(String, bool)>,
    pub(super) import: Option<super::shortcut_transfer::ImportDraft>,
    pub(super) export: Option<super::shortcut_transfer::ExportDraft>,
    pub(super) transfer_seq: u64,
    pub(super) import_loading: bool,
    pub(super) transfer_warnings: Vec<String>,
    /// Verificação dos atalhos importados (`verify`): em curso e o resultado de cada um.
    pub(super) verifying: bool,
    pub(super) checks: Option<super::shortcut_transfer::Checks>,
}

impl Shortcuts {
    /// Uma gravação da lista ou a aplicação de uma importação em curso: nenhuma outra escrita entra no meio.
    pub(super) fn busy(&self) -> bool {
        self.saving || self.import_loading || self.import.as_ref().is_some_and(|draft| draft.applying)
            || self.export.as_ref().is_some_and(|draft| draft.loading || draft.saving)
    }
}

pub(super) enum ShortcutsReply {
    Loaded(u64, Result<Value, Failure>),
    /// Número do pedido e a lista gravada (`None` = restaurar padrão).
    Saved(u64, Option<Vec<Item>>, Result<Value, Failure>),
    Commands(Result<Vec<CommandInfo>, Failure>),
    /// Exportar/importar (`shortcut_transfer.rs`): arquivo gravado e quantas credenciais saíram; conferência; gravação.
    ExportCandidates(u64, Result<Value, String>),
    Exported(u64, Result<(PathBuf, u64, Vec<String>), String>),
    Previewed(u64, Value, Result<Value, String>),
    Imported(u64, Result<Value, String>),
    Verified(Result<super::shortcut_transfer::Checks, String>),
}

impl Hangar {
    /// Relê a lista gravada (depois de importar): a página e o painel lateral mostram o resultado.
    pub(super) fn reload_shortcuts(&mut self, cx: &mut Context<Self>) { self.load_shortcuts(cx); }

    pub(super) fn shortcuts_send_later(&self) -> impl Fn(ShortcutsReply) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>> + Send + 'static {
        let (tx, connection) = (self.tx.clone(), self.connection);
        move |reply| {
            let tx = tx.clone();
            Box::pin(async move { let _ = tx.send(Envelope { connection, selection: None, payload: Payload::Shortcuts(reply) }).await; })
        }
    }

    /// Página aberta: edição de uma visita anterior não volta (o web desmonta o editor); relê do servidor.
    /// Os números dos pedidos ficam, para resposta de antes não passar por resposta de agora.
    pub(super) fn shortcuts_opened(&mut self, cx: &mut Context<Self>) {
        let (load, save_seq, saving) = (std::mem::take(&mut self.shortcuts.load.seq), self.shortcuts.save_seq, self.shortcuts.saving);
        self.shortcuts = Shortcuts { save_seq, saving, transfer_seq: self.shortcuts.transfer_seq + 1, ..Shortcuts::default() };
        self.shortcuts.load.seq = load;
        self.load_shortcuts(cx);
        self.load_project_shortcuts();
    }

    fn load_shortcuts(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else { return };
        let seq = self.shortcuts.load.start();
        let done = self.shortcuts_send_later();
        self.runtime.spawn(async move { done(ShortcutsReply::Loaded(seq, api.config().await)).await });
        cx.notify();
    }

    /// Toda mudança da lista global grava na hora (`POST /api/config`); `None` restaura o padrão.
    fn save_shortcuts(&mut self, list: Option<Vec<Item>>, cx: &mut Context<Self>) {
        let s = &mut self.shortcuts;
        if s.busy() { return; }
        let Some(api) = self.api.clone() else { return };
        s.save_seq += 1;
        (s.saving, s.save_error, s.saved) = (true, None, None);
        let (seq, done) = (s.save_seq, self.shortcuts_send_later());
        let body = json!({"shortcuts": list.as_deref().map(serialize)});
        self.runtime.spawn(async move {
            done(ShortcutsReply::Saved(seq, list, api.server_send(reqwest::Method::POST, &["config"], Some(body), 8).await)).await
        });
        cx.notify();
    }

    /// Grava a lista inteira do projeto (`PUT /project-shortcuts`): acrescentar, editar, mover e apagar são a mesma gravação.
    /// A lista na tela só muda com a resposta, então o que se vê é sempre o que está gravado.
    fn save_project_shortcuts(&mut self, items: Vec<Value>, cx: &mut Context<Self>) {
        let (Some(api), Some(key)) = (self.session_api(), self.selected_key()) else { return };
        let project = &mut self.side.project;
        if project.saving.is_some() || project.owner.as_ref() != Some(&key) { return; }
        project.save_seq += 1;
        (project.saving, project.save_error) = (Some(project.save_seq), None);
        let seq = project.save_seq;
        let (connection, tx) = (self.connection, self.tx.clone());
        let body = json!({"items": items});
        self.runtime.spawn(async move {
            let result = api.server_send(reqwest::Method::PUT, &["sessions", &key.name, "project-shortcuts"], Some(body), 15).await;
            let _ = tx.send(Envelope { connection, selection: None, payload: Payload::Reply(key, Reply::ProjectSaved(seq), result) }).await;
        });
        cx.notify();
    }

    pub(super) fn receive_project_saved(&mut self, key: SessionKey, seq: u64, result: Result<Value, Failure>, window: &mut Window, cx: &mut Context<Self>) {
        let project = &mut self.side.project;
        if project.owner.as_ref() != Some(&key) || project.saving != Some(seq) { return; }
        project.saving = None;
        match result {
            Ok(value) => {
                project.list.set(Ok(super::side::Project::parse(&value)));
                // O formulário do projeto ficou aberto durante a gravação para não perder o que foi digitado se ela falhasse.
                if self.shortcuts.form.as_ref().is_some_and(|form| form.project) { self.close_shortcut_form(window, cx); }
            }
            Err(error) => project.save_error = Some(Self::fetch_failure(&error)),
        }
    }

    fn project_edit(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Vec<Item>)) {
        let key = self.selected_key();
        if self.side.project.saving.is_some() { return; }
        let Some(project) = self.side.project.of(key.as_ref()) else { return };
        let (mut items, raw) = (project.items.clone(), project.raw.clone());
        change(&mut items);
        self.save_project_shortcuts(merge_project_items(&raw, items), cx);
    }

    /// Sugestões de skill: comandos da primeira sessão viva deste servidor. Sem sessão, o campo fica livre.
    fn load_suggestions(&mut self) {
        if self.shortcuts.suggesting || !self.shortcuts.suggestions.is_empty() { return; }
        let Some(api) = self.api.clone() else { return };
        let Some(name) = self.sessions.iter().find(|s| s.state != "dead").map(|s| s.name.clone()) else { return };
        self.shortcuts.suggesting = true;
        let done = self.shortcuts_send_later();
        self.runtime.spawn(async move { done(ShortcutsReply::Commands(api.commands(&name).await)).await });
    }

    pub(super) fn receive_shortcuts(&mut self, reply: ShortcutsReply, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(reply, ShortcutsReply::ExportCandidates(..) | ShortcutsReply::Exported(..) | ShortcutsReply::Previewed(..)
            | ShortcutsReply::Imported(..) | ShortcutsReply::Verified(..)) {
            self.receive_transfer(reply, window, cx);
            return;
        }
        let s = &mut self.shortcuts;
        let mut reload = false;
        match reply {
            ShortcutsReply::Loaded(seq, result) => {
                let parsed = result.map_err(|e| Self::failure(&e)).and_then(|config| match config.pointer("/campos") {
                    Some(campos) => Ok(resolve(campos.pointer("/shortcuts/valor").and_then(Value::as_str).unwrap_or(""))),
                    None => Err(tr("invalid_response")),
                });
                let list = parsed.as_ref().ok().cloned();
                if !s.load.finish(seq, parsed.map(|_| ())) { return; }
                if let Some(list) = list {
                    s.items = list.clone();
                    // O painel mostra os atalhos da máquina da conversa aberta; esta página é da ativa.
                    if self.open_api.is_none() { self.side.set_shortcuts(&list); }
                }
            }
            ShortcutsReply::Saved(seq, list, result) => {
                let saved = list.unwrap_or_else(defaults);
                // Gravado vale para o painel mesmo com a página já fechada.
                if result.is_ok() && self.open_api.is_none() { self.side.set_shortcuts(&saved); }
                if seq != s.save_seq { return; }
                s.saving = false;
                match result {
                    Ok(_) => {
                        s.items = saved;
                        s.saved = Some(seq);
                        if std::mem::take(&mut s.submitting) && s.form.as_ref().is_some_and(|form| !form.project) { self.close_shortcut_form(window, cx); }
                        cx.spawn(async move |this, cx| {
                            cx.background_executor().timer(Duration::from_millis(2500)).await;
                            let _ = this.update(cx, |this, cx| if this.shortcuts.saved == Some(seq) { this.shortcuts.saved = None; cx.notify(); });
                        }).detach();
                    }
                    // Erro de validação do backend chega como veio ("shortcuts: item 2 …").
                    // Uma falha de rede pode ter gravado mesmo assim: relê para a tela mostrar o que ficou.
                    Err(error) => { (s.save_error, s.submitting) = (Some(Self::fetch_failure(&error)), false); reload = true; }
                }
            }
            ShortcutsReply::Commands(result) => {
                s.suggesting = false;
                if let Ok(commands) = result {
                    s.suggestions = commands.into_iter().map(|c| if c.display.is_empty() { format!("/{}", c.name) } else { c.display }).collect();
                }
            }
            ShortcutsReply::ExportCandidates(..) | ShortcutsReply::Exported(..) | ShortcutsReply::Previewed(..)
                | ShortcutsReply::Imported(..) | ShortcutsReply::Verified(..) => {}
        }
        self.drop_stale_form(window, cx);
        if reload { self.load_shortcuts(cx); }
        cx.notify();
    }

    /// O atalho aberto no formulário global saiu da lista (removido, padrão restaurado, lista relida): não há o que gravar.
    fn drop_stale_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let stale = self.shortcuts.form.as_ref().is_some_and(|form| !form.project
            && form.editing.as_deref().is_some_and(|id| !self.shortcuts.items.iter().any(|i| i.id() == id)));
        if stale { self.close_shortcut_form(window, cx); }
    }

    fn shortcuts_edit(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Vec<Item>)) {
        if self.shortcuts.busy() { return; }
        let mut list = self.shortcuts.items.clone();
        change(&mut list);
        self.save_shortcuts(Some(list), cx);
    }

    fn shortcut_position(&self, id: &str) -> Option<usize> { self.shortcuts.items.iter().position(|i| i.id() == id) }

    fn move_shortcut(&mut self, id: &str, delta: isize, project: bool, cx: &mut Context<Self>) {
        if project { self.project_edit(cx, |items| move_by(items, id, delta)); return; }
        let Some(i) = self.shortcut_position(id) else { return };
        let Some(j) = i.checked_add_signed(delta).filter(|j| *j < self.shortcuts.items.len()) else { return };
        self.shortcuts_edit(cx, |items| items.swap(i, j));
    }

    /// Soltar sobre outra linha: o arrastado passa a ocupar o lugar dela. Soltar fora de uma linha não muda nada.
    fn drop_shortcut(&mut self, from: &str, onto: &str, cx: &mut Context<Self>) {
        let (Some(i), Some(j)) = (self.shortcut_position(from), self.shortcut_position(onto)) else { return };
        if i == j { return; }
        self.shortcuts_edit(cx, |items| { let item = items.remove(i); items.insert(j, item); });
    }

    fn open_shortcut_form(&mut self, editing: Option<String>, project: bool, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.selected_key();
        let items = if project { self.side.project.of(key.as_ref()).map_or(&[][..], |p| p.items.as_slice()) } else { self.shortcuts.items.as_slice() };
        let item = editing.as_deref().and_then(|id| items.iter().find(|i| i.id() == id)).cloned();
        if editing.is_some() && item.as_ref().is_none_or(|i| i.kind() == "internal") { return; }
        let shell = item.as_ref().is_some_and(|i| i.kind() == "shell");
        let field = |value: String, placeholder: String, window: &mut Window, cx: &mut Context<Self>| cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder(placeholder);
            state.set_value(value, window, cx);
            state
        });
        let label = field(item.as_ref().map(|i| i.label().to_owned()).unwrap_or_default(), String::new(), window, cx);
        let (glyph, emoji) = match item.as_ref().and_then(Item::icon) {
            Some(icon) if icon.starts_with("emoji:") => ("bolt".to_owned(), icon["emoji:".len()..].to_owned()),
            // Glifo que esta versão não conhece fica com o nome (sem seleção na grade) e volta igual ao gravar, como no web.
            icon => (icon.and_then(|i| i.strip_prefix("glifo:")).unwrap_or("bolt").to_owned(), String::new()),
        };
        let emoji = field(emoji, tr("shortcuts_emoji_hint"), window, cx);
        let content = field(item.as_ref().map(|i| i.content().to_owned()).unwrap_or_default(),
            tr(if shell { "shortcuts_command_hint" } else { "shortcuts_text_hint" }), window, cx);
        let folder = field(item.as_ref().and_then(Item::pasta).unwrap_or_default().to_owned(), tr("shortcuts_folder_hint"), window, cx);
        let mut subscriptions = Vec::new();
        for (input, max) in [(&label, Some(LABEL_MAX)), (&emoji, Some(EMOJI_MAX)), (&content, None), (&folder, None)] {
            subscriptions.push(cx.subscribe_in(input, window, move |this: &mut Hangar, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    if let Some(clipped) = max.and_then(|max| clip(&input.read(cx).value(), max)) {
                        input.update(cx, |state, cx| state.set_value(clipped, window, cx));
                    }
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.submit_shortcut_form(window, cx),
                _ => {}
            }));
        }
        label.update(cx, |state, cx| state.focus(window, cx));
        let id = editing.clone().unwrap_or_else(new_id);
        self.shortcuts.form = Some(Form { editing, id, original: item.as_ref().map(|i| i.0.clone()), project, shell, label, emoji, content, folder, glyph, direct: item.as_ref().is_none_or(Item::sends_direct),
            confirm: item.as_ref().is_some_and(Item::confirm), hangar: item.as_ref().is_some_and(Item::runs_in_hangar),
            home: item.as_ref().is_none_or(Item::hangar_home), ask: item.as_ref().is_none_or(Item::answer_in_app), _subscriptions: subscriptions });
        if !shell { self.load_suggestions(); }
        cx.notify();
    }

    fn submit_shortcut_form(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        // Com um salvar em voo a lista não muda: o formulário fica aberto com o que foi digitado até ele terminar.
        if self.shortcuts.busy() || self.side.project.saving.is_some() { return; }
        let Some(form) = self.shortcuts.form.as_ref().filter(|f| f.valid(cx)) else { return };
        let emoji = Form::value(&form.emoji, cx);
        let icon = if emoji.is_empty() { format!("glifo:{}", form.glyph) } else { format!("emoji:{emoji}") };
        let pasta = if form.project { Form::value(&form.folder, cx) } else { String::new() };
        let draft = Draft { shell: form.shell, label: Form::value(&form.label, cx), content: Form::value(&form.content, cx), icon,
            direct: form.direct, confirm: form.confirm, pasta, hangar: form.hangar, home: form.home, ask: form.ask };
        let (editing, project) = (form.editing.clone(), form.project);
        let item = draft.into_item(form.original.clone(), form.id.clone());
        // O editado pode ter saído da lista com o formulário aberto: sem isto a gravação não mudaria nada e diria "Salvo".
        let key = self.selected_key();
        let present = editing.as_deref().is_none_or(|id| if project {
            self.side.project.of(key.as_ref()).is_some_and(|p| p.items.iter().any(|i| i.id() == id))
        } else { self.shortcuts.items.iter().any(|i| i.id() == id) });
        if !present {
            let error = tr("shortcuts_gone");
            if project { self.side.project.save_error = Some(error); } else { self.shortcuts.save_error = Some(error); }
            cx.notify();
            return;
        }
        if project { self.project_edit(cx, |items| apply_edit(items, editing.as_deref(), item)); return; }
        // O global grava agora e o formulário só fecha com a gravação feita.
        self.shortcuts_edit(cx, |items| apply_edit(items, editing.as_deref(), item));
        self.shortcuts.submitting = self.shortcuts.saving;
    }

    /// O campo com foco sai junto com o formulário: o foco volta à raiz, senão o Esc seguinte não chega a lugar nenhum.
    fn close_shortcut_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.shortcuts.form = None;
        self.root_focus.focus(window, cx);
        cx.notify();
    }

    /// Esc com o formulário aberto fecha só ele.
    pub(super) fn shortcuts_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.settings != Some(super::settings::Page::Shortcuts) || self.shortcuts.form.is_none() { return false; }
        // Gravando, o formulário fica: o Esc não pode esconder o que ainda pode falhar.
        if self.shortcuts.busy() || self.side.project.saving.is_some() { return true; }
        self.close_shortcut_form(window, cx);
        true
    }

    pub(super) fn render_shortcuts_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let page = div().flex().flex_col().child(self.page_top("settings_page_shortcuts", tr("shortcuts_lead")));
        let note = |text: String, color: Hsla| div().px_4().py(px(18.)).text_size(px(13.)).text_color(color).whitespace_normal().child(text);
        if self.api.is_none() { return page.child(settings_box().mt(px(24.)).child(note(tr("settings_offline"), theme::muted()))).into_any_element(); }
        let s = &self.shortcuts;
        match (&s.load.value, s.load.loading) {
            (None, _) => return page.child(settings_box().mt(px(24.)).child(note(tr("shortcuts_loading"), theme::muted()))).into_any_element(),
            (Some(Err(error)), loading) => {
                let retry = Button::new("shortcuts-retry").outline().small().icon(IconName::RefreshCw)
                    .label(tr(if loading { "shortcuts_loading" } else { "shortcuts_retry" })).disabled(loading)
                    .on_click(cx.listener(|this, _, _, cx| this.load_shortcuts(cx)));
                return page.child(settings_box().mt(px(24.))
                    .child(note(format!("{} {error}", tr("shortcuts_load_failed")), theme::danger()))
                    .child(div().px_4().pb(px(16.)).flex().child(retry))).into_any_element();
            }
            _ => {}
        }
        let saving = s.busy();
        let count = s.items.len();
        let mut list = settings_box();
        if count == 0 { list = list.child(note(tr("shortcuts_empty"), theme::muted())); }
        for (n, item) in s.items.iter().enumerate() {
            list = list.child(self.render_shortcut_row(item, n, count, saving, false, cx));
        }
        // Repor um nativo é adicionar um item à lista: vive no rodapé DELA, ao lado do Adicionar, não numa linha solta
        // no meio da pilha de botões.
        let missing: Vec<&str> = NATIVES.into_iter().filter(|a| !s.items.iter().any(|i| i.kind() == "internal" && i.action() == *a)).collect();
        let restore_natives = (!missing.is_empty()).then(|| div().flex().flex_wrap().items_center().justify_end().gap(px(6.))
            .child(div().text_size(px(12.5)).text_color(theme::muted()).child(tr("shortcuts_restore_native")))
            .children(missing.into_iter().map(|action| Button::new(SharedString::from(format!("shortcut-native-{action}"))).ghost().small()
                .icon(IconName::Plus).label(native_label(action)).disabled(saving)
                .on_click(cx.listener(move |this, _, _, cx| this.shortcuts_edit(cx, |items| items.push(Item::native(action))))))));
        // Adicionar é a ação da lista: botão primário no rodapé DA CAIXA, colado nela, em vez de solto embaixo com o
        // mesmo peso de Importar, Exportar e Restaurar.
        let add = self.mark(card_footer(), "shortcuts_add")
            .child(Button::new("shortcut-add").primary().small().icon(IconName::Plus).label(tr("shortcuts_add"))
                .disabled(saving).on_click(cx.listener(|this, _, window, cx| this.open_shortcut_form(None, false, window, cx))))
            .child(div().flex_1().min_w_0().flex().justify_end().children(restore_natives));
        let open_form = matches!(&s.form, Some(form) if !form.project);
        let list = list.when(!open_form, |el| el.child(add));
        let form = open_form.then(|| match &s.form { Some(form) => self.render_shortcut_form(form, cx), None => div() });
        // A falha fica logo abaixo da lista (e dentro do formulário aberto), nunca só no rodapé fora da tela.
        let error_line = s.save_error.clone().map(|error| div().id("shortcuts-save-error").role(Role::Alert).text_size(px(12.5))
            .text_color(theme::danger()).whitespace_normal().child(error));
        let feedback = s.saved.filter(|_| s.save_error.is_none()).map(|_| div().text_color(theme::success()).child(tr("shortcuts_saved")));
        // Caixa própria para mover a lista para outra máquina; repor o padrão fica no rodapé da página.
        let transfer = settings_box().child(div().px(px(16.)).py(px(12.))
            .flex().flex_wrap().items_center().gap(px(8.))
            .child(Button::new("shortcuts-import").outline().small().icon(IconName::Upload).label(tr("shortcuts_import"))
                .loading(s.import_loading)
                .disabled(saving || s.import.is_some() || s.export.is_some()).on_click(cx.listener(|this, _, _, cx| this.import_shortcuts(cx))))
            .child(Button::new("shortcuts-export").outline().small().icon(IconName::Download).label(tr("shortcuts_export"))
                .disabled(saving || s.import.is_some() || s.export.is_some()).on_click(cx.listener(|this, _, _, cx| this.export_shortcuts(cx))))
            .children(self.transfer_note_element())
            .children(self.render_checks(cx)));
        let draft = self.render_import_draft(cx);
        let export = self.render_export_draft(cx);
        let footer = self.mark(div().mt(px(8.)).pt(px(16.)), "shortcuts_restore").border_t_1().border_color(theme::border()).flex().items_center().gap(px(10.))
            .child(Button::new("shortcuts-restore").outline().small().label(tr("shortcuts_restore")).tooltip(tr("shortcuts_restore_help"))
                .disabled(saving).on_click(cx.listener(|this, _, _, cx| this.save_shortcuts(None, cx))))
            .child(div().flex_1().min_w_0().flex().justify_end().text_size(px(12.5)).whitespace_normal().children(feedback));
        let project = self.render_project_shortcuts(cx);
        // Uma pilha de caixas com o mesmo vão, como Máquinas e Contas: a página deixa de ser lista + botões avulsos.
        page.child(div().mt(px(20.)).flex().flex_col().gap(px(16.)).pb(px(8.))
            .child(list).children(error_line).children(form).child(transfer).children(export).children(draft).child(footer).children(project)).into_any_element()
    }

    /// Seção "Deste projeto": só com uma sessão aberta. Cada mudança grava na hora, sem o Salvar dos globais.
    fn render_project_shortcuts(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let key = self.selected_key()?;
        let p = &self.side.project;
        if p.owner.as_ref() != Some(&key) { return None; }
        let note = |text: String, color: Hsla| div().px_4().py(px(18.)).text_size(px(13.)).text_color(color).whitespace_normal().child(text);
        // Mesmo cabeçalho das outras páginas de config (ícone no quadradinho, título, linha que explica): o título
        // miúdo em negrito não dizia que dali para baixo era outra lista.
        let head = |title: String| section_head(IconName::Folder, title, Some(tr("shortcuts_project_lead")), None, px(16.));
        let section = div().id("project-shortcuts").flex().flex_col().gap(px(10.));
        let save_error = p.save_error.clone().map(|error| div().text_size(px(12.5)).text_color(theme::danger()).whitespace_normal().child(error));
        let project = match (&p.list.value, p.list.loading) {
            (None, _) => return Some(section.child(settings_box().child(head(tr("shortcuts_project")))
                .child(note(tr("shortcuts_project_loading"), theme::muted())))),
            (Some(Err(error)), loading) => {
                let retry = Button::new("project-shortcuts-retry").outline().small().icon(IconName::RefreshCw)
                    .label(tr(if loading { "shortcuts_loading" } else { "shortcuts_retry" })).disabled(loading)
                    .on_click(cx.listener(|this, _, _, cx| { this.load_project_shortcuts(); cx.notify(); }));
                return Some(section.child(settings_box().child(head(tr("shortcuts_project")))
                    .child(note(tr("shortcuts_project_failed").replace("{reason}", error), theme::danger()))
                    .child(div().px_4().pb(px(16.)).flex().child(retry))).children(save_error));
            }
            (Some(Ok(project)), _) => project,
        };
        let saving = p.saving.is_some();
        let count = project.items.len();
        let mut list = settings_box().child(head(tr("shortcuts_project_named").replace("{name}", &project.name)));
        if count == 0 { list = list.child(note(tr("shortcuts_project_empty"), theme::muted())); }
        for (n, item) in project.items.iter().enumerate() {
            list = list.child(self.render_shortcut_row(item, n, count, saving, true, cx));
        }
        let open_form = matches!(&self.shortcuts.form, Some(form) if form.project);
        let list = list.when(!open_form, |el| el.child(card_footer()
            .child(Button::new("project-shortcut-add").primary().small().icon(IconName::Plus)
                .label(tr("shortcuts_project_add")).loading(saving).disabled(saving)
                .on_click(cx.listener(|this, _, window, cx| this.open_shortcut_form(None, true, window, cx))))));
        let form = open_form.then(|| match &self.shortcuts.form { Some(form) => self.render_shortcut_form(form, cx), None => div() });
        Some(section.child(list).children(form).children(save_error))
    }

    /// Linha de um atalho; `project` = da seção "Deste projeto" (ids próprios, grava na hora, sem arrastar).
    fn render_shortcut_row(&self, item: &Item, n: usize, count: usize, saving: bool, project: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let native = item.kind() == "internal";
        let (label, icon) = if native { (native_label(item.action()), Some(native_icon(item.action()))) } else { (item.label().to_owned(), item.icon()) };
        let id = item.id().to_owned();
        let prefix = if project { "project-shortcut" } else { "shortcut" };
        let button = |key: &str, icon: IconName, tip: &str, off: bool| chrome::icon_button(SharedString::from(format!("{prefix}-{key}-{id}")), icon, tr(tip), cx)
            .small().disabled(off || saving);
        let (up, down, remove, edit) = (id.clone(), id.clone(), id.clone(), id.clone());
        // Reordenar é um par; editar e remover são outra coisa. Quatro glifos com o mesmo vão liam como um borrão —
        // o par fica junto, com folga dos vizinhos.
        // Interno escolhe se também aparece como bloco em "Ações"; o Rodar está sempre lá.
        let pinnable = native && !project && item.action() != "rodar";
        let (pin, pinned) = (id.clone(), item.tile());
        let actions = div().flex().flex_shrink_0().items_center().gap(px(8.))
            .when(pinnable, |el| el.child(Checkbox::new(SharedString::from(format!("{prefix}-tile-{id}"))).label(tr("shortcuts_tile"))
                .checked(pinned).disabled(saving)
                .on_change(cx.listener(move |this, checked: &bool, _, cx| {
                    let checked = *checked;
                    this.shortcuts_edit(cx, |items| if let Some(item) = items.iter_mut().find(|i| i.id() == pin) {
                        if checked { item.0.insert("tile".into(), Value::Bool(true)); } else { item.0.remove("tile"); }
                    });
                }))))
            .when(!native, |el| el.child(button("edit", IconName::Pencil, "shortcuts_edit", false)
                .on_click(cx.listener(move |this, _, window, cx| this.open_shortcut_form(Some(edit.clone()), project, window, cx)))))
            .child(div().flex().gap(px(1.))
                .child(button("up", IconName::ArrowUp, "shortcuts_up", n == 0).on_click(cx.listener(move |this, _, _, cx| this.move_shortcut(&up, -1, project, cx))))
                .child(button("down", IconName::ArrowDown, "shortcuts_down", n + 1 == count).on_click(cx.listener(move |this, _, _, cx| this.move_shortcut(&down, 1, project, cx)))))
            .child(button("remove", IconName::Close, "shortcuts_remove", false).on_click(cx.listener(move |this, _, _, cx| {
                let change = |items: &mut Vec<Item>| items.retain(|i| i.id() != remove);
                if project { this.project_edit(cx, change) } else { this.shortcuts_edit(cx, change) }
            })));
        let content = match item.pasta() { Some(pasta) => format!("{} · {pasta}", item.content()), None => item.content().to_owned() };
        // Onde o atalho roda: "Na sessão · texto" (envio), "Na sessão" ou "No Hangar" (comando).
        let mark = match item.kind() {
            "send_text" => Some((tr_shared("atalhos_marca_sessao_texto", &[]), false)),
            "shell" if item.runs_in_hangar() => Some((tr_shared("atalhos_marca_hangar", &[]), true)),
            "shell" => Some((tr_shared("atalhos_marca_sessao_comando", &[]), false)),
            _ => None,
        };
        // As duas marcas com o mesmo peso: só o "No Hangar" tinha fundo visível, e a coluna ficava desalinhada a olho
        // (o fundo da outra some no vidro). A borda entra nas duas, senão uma fica 2px mais alta que a vizinha.
        let mark = mark.map(|(text, hangar)| div().flex_shrink_0().px(px(8.)).py(px(3.)).rounded_full().text_size(px(12.)).border_1()
            .when(hangar, |el| el.bg(theme::accent().opacity(0.16)).border_color(theme::accent().opacity(0.35)).text_color(theme::accent_text()))
            .when(!hangar, |el| el.bg(theme::raised()).border_color(theme::border()).text_color(theme::muted())).child(text));
        // A linha aberta no formulário troca a marca por "editando" e ganha um realce de destaque.
        let editing = self.shortcuts.form.as_ref().is_some_and(|f| f.project == project && f.editing.as_deref() == Some(id.as_str()));
        let mark = if editing { Some(div().flex_shrink_0().text_size(px(12.)).text_color(theme::faint()).child(tr_shared("atalhos_editando", &[]))) } else { mark };
        // A marca vem colada ao nome, não encostada nas setas: encostada, sobrava uma faixa vazia no meio de toda linha
        // e as marcas nunca começavam na mesma coluna.
        let text = div().flex_1().min_w_0().flex().flex_col().gap(px(2.))
            .child(div().flex().items_center().gap(px(8.)).min_w_0()
                .child(div().min_w_0().text_size(px(15.)).font_weight(FontWeight::MEDIUM).truncate().child(label.clone()))
                .children(mark))
            .when(!native, |el| el.child(div().text_size(px(12.)).font_family(theme::MONO).text_color(theme::faint()).truncate().child(content)));
        let dragged = Dragged { id: id.clone(), label };
        // A linha que saiu do lugar esmaece, como a `.linha.arrastando` do web.
        let lifted = !project && cx.has_active_drag() && self.shortcuts.dragging.as_deref() == Some(id.as_str());
        let this = cx.entity().downgrade();
        // Divisória em cima de toda linha, como nas outras páginas; a da primeira some sob a borda da caixa.
        // Altura igual em toda linha: a do nativo tem uma linha de texto e a do customizado duas, e a lista ficava
        // com degraus. O quadradinho do ícone ganha borda — sem ela o fundo somia no vidro.
        div().id(SharedString::from(format!("{prefix}-row-{id}"))).mt(px(-1.)).border_t_1().border_color(theme::border())
            .flex().items_center().gap(px(12.)).px(px(16.)).py(px(10.)).min_h(px(56.))
            .when(editing, |el| el.bg(theme::accent().opacity(0.06)))
            .child(chrome::small_icon(IconName::GripVertical, 16., theme::faint()))
            .child(div().size(px(34.)).flex_shrink_0().rounded(px(9.)).bg(theme::raised()).border_1().border_color(theme::border())
                .flex().items_center().justify_center().child(icon_element(icon, 17., theme::muted())))
            .child(text)
            .child(actions)
            .when(lifted, |el| el.opacity(0.45))
            .when(!saving && !project, |el| el.on_drag(dragged, move |d, _, _, cx| {
                    this.update(cx, |this, cx| { this.shortcuts.dragging = Some(d.id.clone()); cx.notify(); }).ok();
                    cx.new(|_| d.clone())
                })
                .drag_over::<Dragged>(|style, _, _, _| style.bg(theme::accent_dim()))
                .on_drop(cx.listener(move |this, d: &Dragged, _, cx| this.drop_shortcut(&d.id, &id, cx))))
    }

    fn render_shortcut_form(&self, form: &Form, cx: &mut Context<Self>) -> Div {
        let field = |key: &str, control: AnyElement| div().flex().flex_col().gap(px(6.))
            .child(div().text_size(px(13.)).text_color(theme::muted()).child(tr(key))).child(control);
        let editing = form.editing.is_some();
        let kinds = [tr("shortcuts_type_send"), tr("shortcuts_type_shell")];
        let kind = segments("shortcut-type", &kinds, form.shell as usize, if editing { 0 } else { 2 }, editing, String::new(),
            |this, n, window, cx| {
                let Some(form) = this.shortcuts.form.as_mut() else { return };
                form.shell = n == 1;
                let hint = tr(if form.shell { "shortcuts_command_hint" } else { "shortcuts_text_hint" });
                form.content.update(cx, |state, cx| state.set_placeholder(hint, window, cx));
                if n == 0 { this.load_suggestions(); }
                cx.notify();
            }, cx);
        let emoji_set = !Form::value(&form.emoji, cx).is_empty();
        let glyphs = div().flex().flex_wrap().items_center().gap(px(2.))
            .children(GLYPHS.iter().map(|(g, icon)| {
                let on = !emoji_set && form.glyph == *g;
                Button::new(SharedString::from(format!("shortcut-glyph-{g}")))
                    .custom(ButtonCustomVariant::new(cx).color(if on { theme::accent_dim() } else { transparent_black() })
                        .foreground(if on { theme::accent_text() } else { theme::muted() }).hover(theme::hover()).active(theme::hover()))
                    .size(px(34.)).rounded(px(8.)).icon(Icon::new(*icon).size(px(16.))).accessibility_label(*g).tooltip(*g)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let Some(form) = this.shortcuts.form.as_mut() else { return };
                        form.glyph = (*g).to_owned();
                        form.emoji.update(cx, |state, cx| state.set_value("", window, cx));
                        cx.notify();
                    }))
            }))
            .child(div().ml(px(8.)).w(px(110.)).child(Input::new(&form.emoji).small()));
        let typed = Form::value(&form.content, cx);
        let picks: Vec<String> = if form.shell { Vec::new() } else {
            self.shortcuts.suggestions.iter().filter(|s| **s != typed && s.to_lowercase().contains(&typed.to_lowercase())).take(6).cloned().collect()
        };
        // O comando de um `shell` é lido como código.
        let content = div().flex().flex_col().gap(px(6.))
            .child(div().when(form.shell, |el| el.font_family(theme::MONO).text_size(px(13.))).child(Input::new(&form.content)))
            .when(!picks.is_empty(), |el| el.child(div().flex().flex_wrap().gap(px(6.)).children(picks.into_iter().map(|pick| {
                Button::new(SharedString::from(format!("shortcut-pick-{pick}"))).ghost().small().label(pick.clone())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(form) = this.shortcuts.form.as_ref() { form.content.update(cx, |state, cx| state.set_value(pick.clone(), window, cx)); }
                        cx.notify();
                    }))
            }))));
        let direct = (!form.shell).then(|| div().flex().flex_col().gap(px(2.))
            .child(Checkbox::new("shortcut-direct").label(tr("shortcuts_send_direct")).checked(form.direct)
                .on_click(cx.listener(|this, on: &bool, _, cx| { if let Some(f) = this.shortcuts.form.as_mut() { f.direct = *on; } cx.notify(); })))
            .child(div().pl(px(24.)).text_size(px(12.)).text_color(theme::muted()).whitespace_normal().child(tr("shortcuts_send_direct_help"))));
        let confirm = Checkbox::new("shortcut-confirm").label(tr("shortcuts_confirm")).checked(form.confirm)
            .on_click(cx.listener(|this, on: &bool, _, cx| { if let Some(f) = this.shortcuts.form.as_mut() { f.confirm = *on; } cx.notify(); }));
        let folder = (form.project && form.shell).then(|| field("shortcuts_folder", div().flex().flex_col().gap(px(6.))
            .child(Input::new(&form.folder))
            .child(div().text_size(px(12.)).text_color(theme::muted()).whitespace_normal().child(tr("shortcuts_folder_help"))).into_any_element()));
        let own = if form.project { self.side.project.saving.is_some() } else { self.shortcuts.saving };
        let busy = self.shortcuts.busy() || self.side.project.saving.is_some();
        let form_error = if form.project { self.side.project.save_error.clone() } else { self.shortcuts.save_error.clone() };
        let where_card = |id: &'static str, hangar: bool, title: &str, help: &str, usage: &str, cx: &mut Context<Self>| {
            let on = form.hangar == hangar;
            Button::new(id)
                .custom(ButtonCustomVariant::new(cx).color(if on { theme::accent().opacity(0.10) } else { transparent_black() })
                    .foreground(theme::text()).hover(theme::hover()).active(theme::hover()))
                .flex_1().min_w(px(200.)).h_auto().p(px(16.)).rounded(px(10.)).border_1()
                .border_color(if on { theme::accent() } else { theme::border_strong() }).accessibility_label(title.to_owned())
                .child(div().w_full().flex().flex_col().items_start().gap(px(8.)).whitespace_normal().text_left()
                    .child(div().flex().items_center().gap(px(10.))
                        .child(div().size(px(14.)).flex_shrink_0().rounded_full().border_color(if on { theme::accent() } else { theme::faint() })
                            .when(on, |el| el.border_4().bg(theme::on_press())).when(!on, |el| el.border_1()))
                        .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(title.to_owned())))
                    .child(div().text_size(px(13.)).line_height(px(19.)).text_color(theme::muted()).child(help.to_owned()))
                    .child(div().text_size(px(12.)).text_color(theme::faint()).child(usage.to_owned())))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(f) = this.shortcuts.form.as_mut() { f.hangar = hangar; }
                    cx.notify();
                }))
        };
        let (where_title, where_click) = (tr_shared("atalhos_onde_clique_titulo", &[]), tr_shared("atalhos_onde_clique", &[]));
        let home_text = tr_shared("atalhos_onde_home", &[("home", "~")]);
        let (home_before, home_after) = match home_text.split_once('~') {
            Some((before, after)) => (before.trim_end().to_owned(), after.trim_start().to_owned()),
            None => (home_text.clone(), String::new()),
        };
        let where_block = form.shell.then(|| div().flex().flex_col().gap(px(10.))
            .child(div().text_size(px(13.)).text_color(theme::muted()).child(tr_shared("atalhos_onde", &[])))
            .child(div().flex().flex_wrap().gap(px(12.))
                .child(where_card("shortcut-where-session", false, &tr_shared("atalhos_onde_sessao", &[]),
                    &tr_shared("atalhos_onde_sessao_ajuda", &[]), &tr_shared("atalhos_onde_sessao_uso", &[]), cx))
                .child(where_card("shortcut-where-hangar", true, &tr_shared("atalhos_onde_hangar", &[]),
                    &tr_shared("atalhos_onde_hangar_ajuda", &[]), &tr_shared("atalhos_onde_hangar_uso", &[]), cx)))
            .when(form.hangar, |el| el.child(div().flex().flex_col().gap(px(12.)).p(px(16.)).rounded(px(10.)).bg(theme::inset()).border_1().border_color(theme::border())
                .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(where_title))
                .child(div().flex().items_start().gap(px(10.))
                    .child(div().mt(px(2.)).flex_shrink_0().child(chrome::small_icon(IconName::ArrowRight, 16., theme::accent())))
                    .child(div().flex_1().min_w_0().text_size(px(13.)).line_height(px(19.)).text_color(theme::muted()).whitespace_normal().child(where_click)))
                .child(div().flex().items_center().gap(px(10.))
                    .child(Checkbox::new("shortcut-home").accessibility_label(home_text.clone()).checked(form.home)
                        .on_click(cx.listener(|this, on: &bool, _, cx| { if let Some(f) = this.shortcuts.form.as_mut() { f.home = *on; } cx.notify(); })))
                    // O `~` é código: o texto do cartão vem partido nele.
                    .child(div().id("shortcut-home-text").flex().flex_wrap().items_center().gap(px(4.)).text_size(px(13.)).text_color(theme::muted()).cursor_pointer()
                        .child(home_before).child(div().font_family(theme::MONO).text_color(theme::text()).child("~")).child(home_after)
                        .on_click(cx.listener(|this, _, _, cx| { if let Some(f) = this.shortcuts.form.as_mut() { f.home = !f.home; } cx.notify(); })))))));
        // "Responder perguntas pelo app." em destaque e a ajuda na mesma frase, como no desenho.
        let ask_title = tr_shared("atalhos_perguntas", &[]);
        let ask_text = StyledText::new(format!("{ask_title} {}", tr_shared("atalhos_perguntas_ajuda", &[])))
            .with_highlights([(0..ask_title.len(), HighlightStyle { color: Some(theme::text()), ..Default::default() })]);
        let ask = form.shell.then(|| div().flex().items_start().gap(px(10.))
            .child(div().mt(px(2.)).flex_shrink_0().child(Checkbox::new("shortcut-ask").accessibility_label(ask_title.clone()).checked(form.ask)
                .on_click(cx.listener(|this, on: &bool, _, cx| { if let Some(f) = this.shortcuts.form.as_mut() { f.ask = *on; } cx.notify(); }))))
            .child(div().id("shortcut-ask-text").flex_1().min_w_0().text_size(px(13.)).line_height(px(19.)).text_color(theme::muted()).whitespace_normal().cursor_pointer()
                .child(ask_text).on_click(cx.listener(|this, _, _, cx| { if let Some(f) = this.shortcuts.form.as_mut() { f.ask = !f.ask; } cx.notify(); }))));
        // Rótulo e comando lado a lado; o ícone desce para depois dos dois.
        let pair = div().flex().flex_wrap().gap(px(16.))
            .child(field("shortcuts_label", Input::new(&form.label).into_any_element()).flex_1().min_w(px(220.)))
            .child(field(if form.shell { "shortcuts_command" } else { "shortcuts_text" }, content.into_any_element()).flex_1().min_w(px(220.)));
        settings_box().mt(px(16.)).p(px(24.)).gap(px(18.)).rounded(px(12.)).bg(theme::surface()).border_color(theme::border_strong())
            .child(field("shortcuts_type", div().flex().child(kind).into_any_element()))
            .child(pair)
            .child(field("shortcuts_icon", glyphs.into_any_element()))
            .children(folder)
            .children(where_block)
            .children(ask)
            .children(direct)
            .child(confirm)
            .children(form_error.map(|error| div().id("shortcut-form-error").role(Role::Alert).text_size(px(13.)).text_color(theme::danger())
                .whitespace_normal().child(error)))
            .child(div().flex().justify_end().gap(px(10.))
                .child(Button::new("shortcut-form-cancel").outline().h(px(38.)).px(px(16.)).rounded(px(8.)).label(tr("cancel")).disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.close_shortcut_form(window, cx))))
                .child(Button::new("shortcut-form-ok")
                    .custom(ButtonCustomVariant::new(cx).color(theme::accent_press()).foreground(theme::on_press()).hover(theme::accent()).active(theme::accent_press()))
                    .h(px(38.)).px(px(18.)).rounded(px(8.)).label(tr("shortcuts_form_ok")).loading(own)
                    .disabled(!form.valid(cx) || busy)
                    .on_click(cx.listener(|this, _, window, cx| this.submit_shortcut_form(window, cx)))))
    }
}

#[cfg(test)]
mod tests {
    use super::{Draft, Item, apply_edit, clip, defaults, merge_project_items, move_by, project_items, resolve, serialize};

    #[test]
    fn project_items_keep_only_valid_send_and_shell_with_their_folder() {
        let value = serde_json::json!([
            {"id":"d","type":"shell","label":"Debug","command":"make debug","pasta":" backend "},
            {"id":"d","type":"shell","label":"dup","command":"x"},
            {"id":"t","type":"internal","action":"terminal"},
            {"id":"v","type":"shell","label":"Vazia","command":"x","pasta":"  "},
            {"id":"s","type":"send_text","label":"Revisa","text":"/review"}]);
        let items = project_items(Some(&value));
        assert_eq!(items.iter().map(Item::id).collect::<Vec<_>>(), ["d", "s"]);
        assert_eq!((items[0].pasta(), items[1].pasta()), (Some("backend"), None));
        assert!(project_items(Some(&serde_json::json!({"x": 1}))).is_empty() && project_items(None).is_empty());
    }

    #[test]
    fn saving_the_project_list_keeps_what_the_screen_hides() {
        let raw = serde_json::json!([
            {"id":"a","type":"send_text","label":"A","text":"a"},
            {"id":"f","type":"futuro","label":"F"},
            {"id":"b","type":"send_text","label":"B","text":"b"},
            {"id":"a","type":"shell","label":"dup","command":"x"}]);
        let raw = raw.as_array().unwrap();
        let ids = |list: &[serde_json::Value]| list.iter().map(|v| v["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
        let mut shown = project_items(Some(&serde_json::Value::Array(raw.clone())));
        move_by(&mut shown, "b", -1);
        assert_eq!(ids(&merge_project_items(raw, shown.clone())), ["b", "f", "a", "a"]);
        shown.retain(|i| i.id() != "b");
        shown.push(project_items(Some(&serde_json::json!([{"id":"n","type":"send_text","label":"N","text":"n"}])))[0].clone());
        let merged = merge_project_items(raw, shown);
        assert_eq!(ids(&merged), ["a", "f", "n", "a"]);
        assert_eq!(merged[1]["type"], "futuro");
    }

    #[test]
    fn folder_is_written_only_for_a_shell_and_cleared_when_empty() {
        let draft = |shell: bool, pasta: &str| Draft { shell, label: "L".into(), content: "c".into(), icon: "glifo:bolt".into(), direct: true, confirm: false, pasta: pasta.into(),
            hangar: false, home: true, ask: true };
        let original = project_items(Some(&serde_json::json!([{"id":"a","type":"shell","label":"L","command":"c","pasta":"old"}])))[0].0.clone();
        assert_eq!(draft(true, "tools").into_item(Some(original.clone()), "a".into()).pasta(), Some("tools"));
        assert_eq!(draft(true, "").into_item(Some(original), "a".into()).pasta(), None);
        assert_eq!(draft(false, "tools").into_item(None, "b".into()).pasta(), None);
    }

    #[test]
    fn where_it_runs_is_written_only_when_it_differs_from_the_default() {
        let draft = |hangar: bool, home: bool, ask: bool| Draft { shell: true, label: "L".into(), content: "c".into(), icon: "glifo:bolt".into(),
            direct: true, confirm: false, pasta: String::new(), hangar, home, ask };
        let plain = draft(false, true, true).into_item(None, "a".into());
        assert!(["runs_in", "hangar_home", "answer_in_app"].iter().all(|key| !plain.0.contains_key(*key)));
        assert!(!plain.runs_in_hangar() && plain.hangar_home() && plain.answer_in_app());
        let hangar = draft(true, false, false).into_item(Some(plain.0.clone()), "a".into());
        assert!(hangar.runs_in_hangar() && !hangar.hangar_home() && !hangar.answer_in_app());
        // Sair do Hangar apaga as marcas dele; a home só vale No Hangar.
        let back = draft(false, false, true).into_item(Some(hangar.0), "a".into());
        assert!(!back.runs_in_hangar() && !back.0.contains_key("hangar_home") && back.answer_in_app());
        assert!(resolve(r#"[{"id":"a","type":"shell","label":"L","command":"c","runs_in":"hangar","hangar_home":"x"}]"#).is_empty());
    }

    #[test]
    fn move_by_swaps_with_the_neighbour_and_stops_at_the_ends() {
        let mut items = project_items(Some(&serde_json::json!([
            {"id":"a","type":"send_text","label":"A","text":"a"},{"id":"b","type":"send_text","label":"B","text":"b"}])));
        move_by(&mut items, "a", 1);
        assert_eq!(items.iter().map(Item::id).collect::<Vec<_>>(), ["b", "a"]);
        move_by(&mut items, "a", 1);
        move_by(&mut items, "b", -1);
        assert_eq!(items.iter().map(Item::id).collect::<Vec<_>>(), ["b", "a"]);
    }

    #[test]
    fn editing_keeps_unknown_fields_and_never_revives_a_removed_item() {
        let mut items = resolve(r#"[{"id":"a","type":"send_text","label":"R","text":"/r","send_direct":false,"confirm":true,"novo":1,"icon":"glifo:futuro"}]"#);
        let draft = Draft { shell: false, label: "R2".into(), content: "/r2".into(), icon: "glifo:futuro".into(), direct: true, confirm: false, pasta: String::new(),
            hangar: false, home: true, ask: true };
        let edited = draft.into_item(Some(items[0].0.clone()), "a".into());
        // O campo desconhecido e o glifo que esta versão não conhece ficam; as marcas desligadas saem.
        assert_eq!(edited.0.get("novo"), Some(&serde_json::json!(1)));
        assert_eq!((edited.icon(), edited.label(), edited.content()), (Some("glifo:futuro"), "R2", "/r2"));
        assert!(edited.sends_direct() && !edited.confirm() && !edited.0.contains_key("send_direct"));
        apply_edit(&mut items, Some("a"), edited.clone());
        assert_eq!(items, vec![edited.clone()]);
        items.clear();
        apply_edit(&mut items, Some("a"), edited);
        assert!(items.is_empty());
    }

    #[test]
    fn resending_a_new_shortcut_with_the_same_id_does_not_duplicate_it() {
        let mut items = resolve(r#"[{"id":"a","type":"send_text","label":"A","text":"a"}]"#);
        let again = resolve(r#"[{"id":"n","type":"send_text","label":"N","text":"n"}]"#).remove(0);
        apply_edit(&mut items, None, again.clone());
        apply_edit(&mut items, None, again);
        assert_eq!(items.iter().map(Item::id).collect::<Vec<_>>(), ["a", "n"]);
    }

    #[test]
    fn resolve_keeps_the_web_rules_and_unknown_fields() {
        let ids = |raw: &str| resolve(raw).iter().map(|i| i.id().to_owned()).collect::<Vec<_>>();
        assert_eq!(resolve(""), defaults());
        assert_eq!(resolve("{quebrado"), defaults());
        assert_eq!(resolve(r#"{"id":"x"}"#), defaults());
        assert!(resolve("[]").is_empty());
        let raw = r#"[{"id":"a","type":"send_text","label":"R","text":"/r","novo":1},{"id":"a","type":"shell","label":"d","command":"x"},
            {"id":"b","type":"shell","label":"B","command":"make","icon":7},{"id":"c","type":"internal","action":"modo"},
            {"id":"d","type":"internal","action":"outra"},{"id":" ","type":"shell","label":"x","command":"y"},
            {"id":"e","type":"send_text","label":"E","text":"t","send_direct":null}]"#;
        assert_eq!(ids(raw), ["a", "c"]);
        // O campo desconhecido volta na gravação.
        assert!(serialize(&resolve(raw)).contains(r#""novo":1"#));
        assert_eq!(resolve(&serialize(&defaults())), defaults());
        let direct = |raw: &str| resolve(raw)[0].sends_direct();
        assert!(direct(r#"[{"id":"a","type":"send_text","label":"R","text":"/r"}]"#));
        assert!(!direct(r#"[{"id":"a","type":"send_text","label":"R","text":"/r","send_direct":false}]"#));
        assert_eq!(Item::native("rodar").action(), "rodar");
    }

    #[test]
    fn clip_counts_like_maxlength() {
        assert_eq!(clip("🚀🚀", 4), None);
        assert_eq!(clip("🚀🚀🚀", 4).as_deref(), Some("🚀🚀"));
        assert_eq!(clip(&"a".repeat(25), 24).map(|s| s.len()), Some(24));
    }
}
