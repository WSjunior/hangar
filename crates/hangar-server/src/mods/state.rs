//! Estado da interface dos mods por sessão sem terminal atendida pelo Rust: quem leva os pedidos dos
//! apps (o ator), o último `plugin_ui`, os avisos vivos e o clique do app em aberto. Liga o ator do
//! runtime, as rotas dos apps e o hub de eventos dos aparelhos, que vivem em lugares diferentes.
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::model::{ModsCall, ModsError, BAND_SITE};
use crate::side::{WeakHubs, TOASTS_KEPT};

pub type CallFuture = Pin<Box<dyn Future<Output = Result<Value, ModsError>> + Send>>;

/// Quem leva o pedido do app à superfície da sessão: o ator do runtime (`RuntimeHandle`). `deadline` é o
/// prazo de quem pediu: depois dele a resposta não serve, e a ação não pode rodar no mod.
pub trait SurfaceLink: Send + Sync {
    fn call(&self, call: ModsCall, deadline: Instant) -> CallFuture;
}

/// Mesmos tetos do Python (`plugin_bridge`): o app trata o aviso igual nas duas fontes. O máximo e a
/// quantidade guardada são os do `side.rs`, que já os aplica aos avisos que vêm do Python.
const TOAST_DEFAULT_MS: u64 = 4000;
const TOAST_MIN_MS: u64 = 1000;
const TOAST_MAX_MS: u64 = crate::side::TOAST_MAX_MS as u64;
const TOAST_MAX_CHARS: usize = 2000;
const TOAST_PLUGIN_CHARS: usize = 64;
/// Por quanto tempo o efeito (cópia, URL) ainda é do clique: a mesma janela do plugin (`APP_PRESS_MS`).
const CLICK_WINDOW: Duration = Duration::from_millis(1500);
const CLICK_POLL: Duration = Duration::from_millis(20);

struct Click {
    site: String,
    key: String,
    /// O mod do botão, lido do último `plugin_ui`: só a cópia dele é do clique (A11).
    plugin: Option<String>,
    attempt: String,
    until: Instant,
    matched: bool,
    copied: Option<String>,
    opened: Option<String>,
}

/// O último `plugin_ui`: a árvore, para achar o mod de um botão sem refazer o parse, e o texto, que é o
/// que sai aos aparelhos e o que se compara. Os dois por `Arc`: quem lê copia o ponteiro sob a trava
/// global e trabalha fora dela.
struct Ui {
    tree: Arc<Value>,
    raw: Arc<str>,
}

struct Session {
    generation: u64,
    /// A chave durável da sessão, que não muda ao renomear.
    key: String,
    /// Nomes anteriores da sessão (renomeada sem relançar o `claude -p`): o processo continua mandando à
    /// ponte o nome com que nasceu (`CP_SESSION_NAME`) e o token dele.
    aliases: Vec<String>,
    link: Arc<dyn SurfaceLink>,
    lock: Arc<tokio::sync::Mutex<()>>,
    ui: Option<Ui>,
    toasts: Vec<(Instant, Value)>,
    click: Option<Click>,
}

#[derive(Default)]
struct Inner {
    sessions: HashMap<String, Session>,
    /// Nomes das sessões que saíram do Rust, por chave: o renomear fecha e reabre a sessão com o mesmo
    /// processo, e a reabertura herda daqui os nomes antigos.
    departed: VecDeque<(String, Vec<String>)>,
    toast_seq: u64,
}

/// Quantas sessões que saíram do Rust guardam os nomes para uma reabertura.
const DEPARTED_KEPT: usize = 64;

#[derive(Clone, Default)]
pub struct Mods {
    inner: Arc<Mutex<Inner>>,
    hubs: Arc<OnceLock<WeakHubs>>,
}

fn random_hex(bytes: usize) -> String {
    use ring::rand::SecureRandom;
    let mut buffer = vec![0u8; bytes];
    ring::rand::SystemRandom::new().fill(&mut buffer).expect("fonte de aleatoriedade do sistema");
    buffer.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// O número dos avisos recomeça a cada subida: o prefixo impede o app de tomar um aviso novo por um
/// já mostrado, como o `_TOAST_BOOT` do Python.
fn boot() -> &'static str {
    static BOOT: OnceLock<String> = OnceLock::new();
    BOOT.get_or_init(|| random_hex(4))
}

/// O mod do botão `key` no lugar `site` do `plugin_ui` guardado. Sem o botão (desenho vencido), nenhum:
/// a cópia que chegar no meio do clique vira aviso.
fn button_plugin(ui: &Value, site: &str, key: &str) -> Option<String> {
    let tree = if site == BAND_SITE { &ui["above"] }
        else { &ui["panes"].as_array()?.iter().find(|pane| pane["id"] == site)?["tree"] };
    super::tree::find(tree, key, &["Button"]).map(|control| control.plugin)
}

/// `plugin_ui` sem faixa e sem painel: a sessão saiu do Rust.
fn empty_ui() -> Value {
    json!({"above": null, "panes": [], "shown_id": null, "columns": null, "source": "surface"})
}

impl Mods {
    pub fn bind_hubs(&self, hubs: WeakHubs) {
        let _ = self.hubs.set(hubs);
    }

    /// O ator abriu a sessão no Rust: daqui em diante os pedidos dos apps são dele (dono único). Os
    /// avisos vivos ficam; a faixa espera o primeiro desenho do processo novo.
    pub fn attach(&self, name: &str, generation: u64, link: Arc<dyn SurfaceLink>) {
        self.attach_keyed(name, name, generation, link);
    }

    /// `attach` com a chave durável: reaberta com outro nome (renomear), a sessão herda os nomes de antes
    /// para a ponte. O nome atual de uma sessão vence o nome antigo de outra.
    pub fn attach_keyed(&self, name: &str, key: &str, generation: u64, link: Arc<dyn SurfaceLink>) {
        let mut inner = self.inner.lock().unwrap();
        let toasts = inner.sessions.remove(name).map(|old| old.toasts).unwrap_or_default();
        let aliases = match inner.departed.iter().position(|(departed, _)| departed == key) {
            Some(at) => inner.departed.remove(at).map(|(_, names)| names).unwrap_or_default(),
            None => Vec::new(),
        };
        let aliases: Vec<String> = aliases.into_iter().filter(|alias| alias != name).collect();
        for session in inner.sessions.values_mut() {
            session.aliases.retain(|alias| alias != name);
        }
        inner.sessions.insert(name.to_owned(), Session { generation, key: key.to_owned(), aliases, link,
            lock: Arc::default(), ui: None, toasts, click: None });
    }

    /// A sessão saiu do Rust (S9): esquece o estado e limpa a faixa dos aparelhos.
    pub fn forget(&self, name: &str, generation: u64) {
        let removed = {
            let mut inner = self.inner.lock().unwrap();
            match inner.sessions.get(name).is_some_and(|session| session.generation == generation).then(|| inner.sessions.remove(name)).flatten() {
                Some(session) => {
                    let mut names = session.aliases;
                    names.push(name.to_owned());
                    inner.departed.retain(|(key, _)| *key != session.key);
                    inner.departed.push_back((session.key, names));
                    if inner.departed.len() > DEPARTED_KEPT {
                        inner.departed.pop_front();
                    }
                    true
                }
                None => false,
            }
        };
        if removed {
            self.deliver(name, "plugin_ui", &empty_ui().to_string());
        }
    }

    pub fn owns(&self, name: &str) -> bool {
        self.inner.lock().unwrap().sessions.contains_key(name)
    }

    /// A sessão que a ponte do plugin quer dizer com `sessao`: o nome atual dela, ou o nome com que o
    /// processo nasceu, se ela foi renomeada sem relançar o `claude -p`. Devolve o nome atual.
    pub fn bridge_session(&self, sessao: &str) -> Option<String> {
        let inner = self.inner.lock().unwrap();
        if inner.sessions.contains_key(sessao) {
            return Some(sessao.to_owned());
        }
        inner.sessions.iter().find(|(_, session)| session.aliases.iter().any(|alias| alias == sessao)).map(|(name, _)| name.clone())
    }

    pub fn link(&self, name: &str) -> Option<(Arc<dyn SurfaceLink>, Arc<tokio::sync::Mutex<()>>)> {
        self.inner.lock().unwrap().sessions.get(name).map(|session| (session.link.clone(), session.lock.clone()))
    }

    /// Guarda e entrega o `plugin_ui`; devolve se mudou. É o único ponto que compara a vista nova com a
    /// anterior: a superfície publica a cada desenho guardado, sem guardar cópia para comparar.
    pub fn publish_ui(&self, name: &str, generation: u64, data: Value) -> bool {
        let raw: Arc<str> = data.to_string().into();
        {
            let mut inner = self.inner.lock().unwrap();
            let Some(session) = inner.sessions.get_mut(name).filter(|session| session.generation == generation) else { return false };
            if session.ui.as_ref().is_some_and(|ui| ui.raw == raw) {
                return false;
            }
            session.ui = Some(Ui { tree: Arc::new(data), raw: raw.clone() });
        }
        self.deliver(name, "plugin_ui", &raw);
        true
    }

    /// O ator morreu sem passar pelo `close`: a faixa e os painéis somem dos apps, e a sessão segue com o
    /// mesmo dono até o `close` a esquecer.
    pub fn clear_ui(&self, name: &str, generation: u64) {
        self.publish_ui(name, generation, empty_ui());
    }

    /// Aviso de mod (`ui_toast`, S6): o Claude Code já descarta o que vem a menos de 2 s do anterior do
    /// mesmo mod, e o Hangar não limita de novo.
    pub fn toast(&self, name: &str, generation: u64, plugin: &str, text: &str, timeout_ms: u64) {
        if text.trim().is_empty() {
            return;
        }
        let toast = {
            let mut inner = self.inner.lock().unwrap();
            inner.toast_seq += 1;
            let seq = inner.toast_seq;
            let Some(session) = inner.sessions.get_mut(name).filter(|session| session.generation == generation) else { return };
            let ms = if timeout_ms == 0 { TOAST_DEFAULT_MS } else { timeout_ms }.clamp(TOAST_MIN_MS, TOAST_MAX_MS);
            let toast = json!({"id": format!("rs-{}-{seq}", boot()),
                "text": text.chars().take(TOAST_MAX_CHARS).collect::<String>(),
                "plugin": plugin.chars().take(TOAST_PLUGIN_CHARS).collect::<String>(), "timeoutMs": ms});
            let now = Instant::now();
            session.toasts.retain(|(until, _)| *until > now);
            session.toasts.push((now + Duration::from_millis(ms), toast.clone()));
            if session.toasts.len() > TOASTS_KEPT {
                let extra = session.toasts.len() - TOASTS_KEPT;
                session.toasts.drain(..extra);
            }
            toast
        };
        self.deliver(name, "plugin_toast", &toast.to_string());
    }

    /// O mod copiou um texto. Com clique do app em aberto, o texto volta na resposta do clique, para o
    /// aparelho de quem clicou; sem clique, vira aviso (spec, "Fonte superfície", passo 6).
    pub fn copied(&self, name: &str, generation: u64, plugin: &str, text: &str) {
        let taken = {
            let mut inner = self.inner.lock().unwrap();
            let Some(session) = inner.sessions.get_mut(name).filter(|session| session.generation == generation) else { return };
            match session.click.as_mut().filter(|click| click.until > Instant::now() && click.plugin.as_deref() == Some(plugin)) {
                Some(click) => { click.copied = Some(text.to_owned()); true }
                None => false,
            }
        };
        if !taken {
            self.toast(name, generation, plugin, text, TOAST_DEFAULT_MS);
        }
    }

    /// Abre o clique do app: o plugin do Hangar casa o press com ele (`press-start`) e o efeito volta
    /// para quem clicou.
    pub fn begin_click(&self, name: &str, site: &str, key: &str) -> String {
        let attempt = random_hex(8);
        // A busca do botão na árvore (até ~400 KB) fica fora da trava de todas as sessões.
        let tree = self.inner.lock().unwrap().sessions.get(name).and_then(|session| session.ui.as_ref().map(|ui| ui.tree.clone()));
        let plugin = tree.and_then(|tree| button_plugin(&tree, site, key));
        if let Some(session) = self.inner.lock().unwrap().sessions.get_mut(name) {
            session.click = Some(Click { site: site.to_owned(), key: key.to_owned(), plugin, attempt: attempt.clone(),
                until: Instant::now() + CLICK_WINDOW, matched: false, copied: None, opened: None });
        }
        attempt
    }

    /// O press é o clique que o app pediu? Sim uma vez só, como o `_do_app` do Python.
    pub fn match_click(&self, name: &str, site: &str, key: &str) -> Option<String> {
        let mut inner = self.inner.lock().unwrap();
        let click = inner.sessions.get_mut(name)?.click.as_mut()?;
        if click.matched || click.site != site || click.key != key || click.until <= Instant::now() {
            return None;
        }
        click.matched = true;
        Some(click.attempt.clone())
    }

    /// A URL que o mod abriria, para o aparelho de quem clicou. Fora do clique, `false`: o plugin deixa
    /// o mod abrir na máquina do servidor.
    pub fn opened(&self, name: &str, attempt: &str, url: &str) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let Some(click) = inner.sessions.get_mut(name).and_then(|session| session.click.as_mut()) else { return false };
        if click.attempt != attempt || click.until <= Instant::now() {
            return false;
        }
        click.opened = Some(url.to_owned());
        true
    }

    /// Fecha o clique e devolve a cópia e a URL dele. O `onPress` do mod costuma copiar ou abrir sem
    /// `await`: com o press casado pelo plugin e nada ainda, espera o efeito até `wait`.
    pub async fn finish_click(&self, name: &str, attempt: &str, wait: Duration) -> (Option<String>, Option<String>) {
        let deadline = Instant::now() + wait;
        loop {
            let (done, matched) = {
                let inner = self.inner.lock().unwrap();
                match inner.sessions.get(name).and_then(|session| session.click.as_ref()).filter(|click| click.attempt == attempt) {
                    Some(click) => (click.copied.is_some() || click.opened.is_some(), click.matched),
                    None => (true, false),
                }
            };
            if done || !matched || Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(CLICK_POLL).await;
        }
        let mut inner = self.inner.lock().unwrap();
        let Some(session) = inner.sessions.get_mut(name) else { return (None, None) };
        match session.click.take() {
            Some(click) if click.attempt == attempt => (click.copied, click.opened),
            other => { session.click = other; (None, None) }
        }
    }

    /// O que um hub novo precisa para nascer em dia: a última faixa e os avisos vivos, com o tempo
    /// que resta a cada um.
    pub fn replay(&self, name: &str) -> Vec<(&'static str, String)> {
        let (ui, toasts) = {
            let inner = self.inner.lock().unwrap();
            let Some(session) = inner.sessions.get(name) else { return Vec::new() };
            (session.ui.as_ref().map(|ui| ui.raw.clone()), session.toasts.clone())
        };
        let now = Instant::now();
        let mut frames: Vec<(&'static str, String)> = ui.iter().map(|ui| ("plugin_ui", ui.to_string())).collect();
        for (until, toast) in toasts.iter().filter(|(until, _)| *until > now) {
            let mut toast = toast.clone();
            toast["timeoutMs"] = json!(crate::side::remaining_ms(*until, now));
            frames.push(("plugin_toast", toast.to_string()));
        }
        frames
    }

    fn deliver(&self, name: &str, event: &str, data: &str) {
        if let Some(hubs) = self.hubs.get().and_then(WeakHubs::upgrade) {
            hubs.deliver(name, event, data);
        }
    }
}
