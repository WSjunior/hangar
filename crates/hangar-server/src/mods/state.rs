//! Estado da interface dos mods por sessão sem terminal atendida pelo Rust: quem leva os pedidos dos
//! apps (o ator), o último `plugin_ui`, os avisos vivos e o clique do app em aberto. Liga o ator do
//! runtime, as rotas dos apps e o hub de eventos dos aparelhos, que vivem em lugares diferentes.
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::model::{ModsCall, ModsError, BAND_SITE, TOAST_DEFAULT_MS};
use crate::side::{WeakHubs, TOASTS_KEPT};

pub type CallFuture = Pin<Box<dyn Future<Output = Result<Value, ModsError>> + Send>>;

/// O caminho de um pedido de app à sessão: quem o leva à superfície e a vez da sessão, que faz os
/// pedidos de aparelhos diferentes correrem um por vez.
pub struct Turn {
    pub link: Arc<dyn SurfaceLink>,
    pub lock: Arc<tokio::sync::Mutex<()>>,
}

/// Quem leva o pedido do app à superfície da sessão: o ator do runtime (`RuntimeHandle`). `deadline` é o
/// prazo de quem pediu: depois dele a resposta não serve, e a ação não pode rodar no mod.
pub trait SurfaceLink: Send + Sync {
    fn call(&self, call: ModsCall, deadline: Instant) -> CallFuture;
}

/// Mesmos tetos do Python (`plugin_bridge`): o app trata o aviso igual nas duas fontes. O máximo e a
/// quantidade guardada são os do `side.rs`, que já os aplica aos avisos que vêm do Python.
const TOAST_MIN_MS: u64 = 1000;
const TOAST_MAX_MS: u64 = crate::side::TOAST_MAX_MS as u64;
const TOAST_MAX_CHARS: usize = 2000;
const TOAST_PLUGIN_CHARS: usize = 64;
/// Por quanto tempo o efeito (cópia, URL) ainda é do clique: a mesma janela do plugin (`APP_PRESS_MS`).
const CLICK_WINDOW: Duration = Duration::from_millis(1500);

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
    /// Acorda quem espera o efeito do clique (`finish_click`) quando a cópia ou a URL chega.
    effect: Arc<tokio::sync::Notify>,
}

/// O último `plugin_ui`: a árvore, para achar o mod de um botão sem refazer o parse, e o texto, que é o
/// que sai aos aparelhos e o que se compara. Os dois por `Arc`: quem lê copia o ponteiro sob a trava
/// global e trabalha fora dela.
struct Ui {
    tree: Arc<Value>,
    raw: Arc<str>,
}

struct Session {
    /// A vida do ator que atende a sessão, única no servidor (`Mods::new_life`): a publicação de um ator
    /// velho, de outra sessão que teve o mesmo nome, não casa com ela.
    life: u64,
    /// O processo do `claude -p` (chave durável e cano): o renomear fecha e reabre a sessão no mesmo.
    process: String,
    /// O nome com que o processo nasceu (`CP_SESSION_NAME`), que é o que ele manda à ponte junto com o
    /// token desse nome. Muda só com processo novo, não com o renomear.
    born: String,
    link: Arc<dyn SurfaceLink>,
    lock: Arc<tokio::sync::Mutex<()>>,
    ui: Option<Ui>,
    toasts: Vec<(Instant, Value)>,
    click: Option<Click>,
}

#[derive(Default)]
struct Inner {
    sessions: HashMap<String, Session>,
    /// O nome de nascimento dos processos cujas sessões saíram do Rust: o renomear fecha e reabre a
    /// sessão com o mesmo processo, e a reabertura o herda daqui.
    departed: VecDeque<(String, String)>,
    toast_seq: u64,
}

/// Quantos processos que saíram do Rust guardam o nome de nascimento para uma reabertura.
const DEPARTED_KEPT: usize = 64;

#[derive(Clone, Default)]
pub struct Mods {
    inner: Arc<Mutex<Inner>>,
    hubs: Arc<OnceLock<WeakHubs>>,
    lives: Arc<std::sync::atomic::AtomicU64>,
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

    /// Identificador de uma vida de ator, único no servidor. A geração da sessão não serve: ela conta por
    /// chave durável, e duas sessões que tiveram o mesmo nome podem ter a mesma.
    pub fn new_life(&self) -> u64 {
        self.lives.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
    }

    /// O ator abriu a sessão no Rust: daqui em diante os pedidos dos apps são dele (dono único). Os
    /// avisos vivos ficam; a faixa espera o primeiro desenho do processo novo.
    pub fn attach(&self, name: &str, life: u64, link: Arc<dyn SurfaceLink>) {
        self.attach_process(name, name, life, link);
    }

    /// `attach` com o processo do `claude -p`. Reaberta no mesmo processo (renomear), a sessão herda o nome
    /// de nascimento dele e os avisos vivos. Outro processo com o mesmo nome não herda nada: nem avisos, nem
    /// faixa, nem clique em aberto, nem o nome de nascimento de quem saiu.
    pub fn attach_process(&self, name: &str, process: &str, life: u64, link: Arc<dyn SurfaceLink>) {
        let replaced = {
            let mut inner = self.inner.lock().unwrap();
            let old = inner.sessions.remove(name);
            let replaced = old.as_ref().is_some_and(|old| old.process != process);
            let toasts = old.filter(|old| old.process == process).map(|old| old.toasts).unwrap_or_default();
            let born = match inner.departed.iter().position(|(departed, _)| departed == process) {
                Some(at) => inner.departed.remove(at).map(|(_, born)| born).unwrap_or_else(|| name.to_owned()),
                None => name.to_owned(),
            };
            inner.sessions.insert(name.to_owned(), Session { life, process: process.to_owned(), born, link,
                lock: Arc::default(), ui: None, toasts, click: None });
            replaced
        };
        // A faixa que os aparelhos guardam é da sessão substituída.
        if replaced {
            self.deliver(name, "plugin_ui", &empty_ui().to_string());
        }
    }

    /// A sessão saiu do Rust (S9): esquece o estado e limpa a faixa dos aparelhos.
    pub fn forget(&self, name: &str, life: u64) {
        let removed = {
            let mut inner = self.inner.lock().unwrap();
            match inner.sessions.get(name).is_some_and(|session| session.life == life).then(|| inner.sessions.remove(name)).flatten() {
                Some(session) => {
                    inner.departed.retain(|(process, _)| *process != session.process);
                    inner.departed.push_back((session.process, session.born));
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

    /// A sessão que a ponte do plugin quer dizer com `sessao`, o nome com que o processo dela nasceu (que
    /// difere do nome atual depois de um renomear sem relançar o `claude -p`). Devolve o nome atual.
    ///
    /// O token da ponte é derivado só do nome: dois processos que nasceram com o mesmo nome têm o mesmo
    /// token, e o servidor não os distingue. Com duas sessões vivas nessa situação (uma renomeada, outra
    /// criada depois com o nome antigo), a ponte não atende nenhuma das duas, para um processo não agir no
    /// clique da outra: o pedido segue ao Python, que não tem o clique, e o mod abre a URL no servidor.
    pub fn bridge_session(&self, sessao: &str) -> Option<String> {
        let inner = self.inner.lock().unwrap();
        let mut found = inner.sessions.iter().filter(|(_, session)| session.born == sessao).map(|(name, _)| name.clone());
        match (found.next(), found.next()) {
            (Some(name), None) => Some(name),
            _ => None,
        }
    }

    pub fn link(&self, name: &str) -> Option<Turn> {
        self.inner.lock().unwrap().sessions.get(name).map(|session| Turn { link: session.link.clone(), lock: session.lock.clone() })
    }

    /// Guarda e entrega o `plugin_ui`; devolve se mudou. É o único ponto que compara a vista nova com a
    /// anterior: a superfície publica a cada desenho guardado, sem guardar cópia para comparar.
    pub fn publish_ui(&self, name: &str, life: u64, data: Value) -> bool {
        let raw: Arc<str> = data.to_string().into();
        {
            let mut inner = self.inner.lock().unwrap();
            let Some(session) = inner.sessions.get_mut(name).filter(|session| session.life == life) else { return false };
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
    pub fn clear_ui(&self, name: &str, life: u64) {
        self.publish_ui(name, life, empty_ui());
    }

    /// Aviso de mod (`ui_toast`, S6): o Claude Code já descarta o que vem a menos de 2 s do anterior do
    /// mesmo mod, e o Hangar não limita de novo.
    pub fn toast(&self, name: &str, life: u64, plugin: &str, text: &str, timeout_ms: u64) {
        if text.trim().is_empty() {
            return;
        }
        let toast = {
            let mut inner = self.inner.lock().unwrap();
            inner.toast_seq += 1;
            let seq = inner.toast_seq;
            let Some(session) = inner.sessions.get_mut(name).filter(|session| session.life == life) else { return };
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
    pub fn copied(&self, name: &str, life: u64, plugin: &str, text: &str) {
        let taken = {
            let mut inner = self.inner.lock().unwrap();
            let Some(session) = inner.sessions.get_mut(name).filter(|session| session.life == life) else { return };
            match session.click.as_mut().filter(|click| click.until > Instant::now() && click.plugin.as_deref() == Some(plugin)) {
                Some(click) => {
                    click.copied = Some(text.to_owned());
                    click.effect.notify_one();
                    true
                }
                None => false,
            }
        };
        if !taken {
            self.toast(name, life, plugin, text, TOAST_DEFAULT_MS);
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
                until: Instant::now() + CLICK_WINDOW, matched: false, copied: None, opened: None, effect: Arc::default() });
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
        click.effect.notify_one();
        true
    }

    /// Fecha o clique e devolve a cópia e a URL dele. O `onPress` do mod costuma copiar ou abrir sem
    /// `await`: com o press casado pelo plugin e nada ainda, espera o efeito até `wait`.
    pub async fn finish_click(&self, name: &str, attempt: &str, wait: Duration) -> (Option<String>, Option<String>) {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            // O `notify_one` guarda a vez quando ninguém espera ainda: o efeito que chega entre a leitura e
            // a espera não se perde.
            let effect = {
                let inner = self.inner.lock().unwrap();
                match inner.sessions.get(name).and_then(|session| session.click.as_ref()).filter(|click| click.attempt == attempt) {
                    Some(click) if click.matched && click.copied.is_none() && click.opened.is_none() => click.effect.clone(),
                    _ => break,
                }
            };
            if tokio::time::timeout_at(deadline, effect.notified()).await.is_err() {
                break;
            }
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
