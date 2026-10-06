//! O elo da sessão com terminal que o Rust atende: leva os pedidos dos apps ao clique (`click.rs`), dentro
//! do prazo de quem pediu, lê o painel na frente pela tela e vigia o tamanho do terminal (T9 e T10). Sem
//! terminal de verdade ligado à sessão o Hangar é dono do tamanho e repõe o mínimo; com um ligado, o
//! tamanho é da pessoa.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::click::{self, Ctx, Limits, Pane, Parts, Undo};
use super::model::{ModsCall, no_answer, no_typing};
use super::state::{CallFuture, Mods, ShownFuture, SurfaceLink, TerminalProbe};

/// Prazo da reposição do mínimo pelo vigia: redimensionar e assentar (até 1 s) com o piso das ações.
const FLOOR_BUDGET: Duration = Duration::from_secs(5);
/// Quanto o vigia espera a vez do pane: um pedido do app inteiro, do orçamento da rota à limpeza.
const FLOOR_WAIT: Duration = super::routes::REQUEST_BUDGET.saturating_add(click::UNDO_MAX);
/// Prazo da leitura do painel na frente, que não é pedido de app.
const SHOWN_READ_MAX: Duration = Duration::from_secs(2);

pub struct TerminalLink {
    parts: Parts,
    watch: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

/// Aborta a tarefa ao sair de escopo: o leitor do vigia morre junto com o laço que o consome.
struct AbortOnDrop(tokio::task::JoinHandle<()>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) { self.0.abort(); }
}

impl TerminalLink {
    /// `life` é a vida com que a sessão é ligada (`Mods::attach_terminal`): o clique, a leitura do painel na
    /// frente e o vigia só leem e escrevem o registro dessa vida.
    pub fn new(name: String, life: u64, pane: Arc<dyn Pane>, mods: Mods, limits: Limits) -> Arc<Self> {
        Arc::new(Self { parts: Parts { name, pane, mods, limits, busy: Arc::default(), life }, watch: Mutex::new(None) })
    }

    fn ctx<'a>(&'a self, until: Instant, undo: &'a Undo) -> Ctx<'a> {
        let parts = &self.parts;
        Ctx { name: &parts.name, pane: parts.pane.as_ref(), mods: &parts.mods, limits: &parts.limits, until, undo, life: parts.life }
    }

    /// Repõe o tamanho mínimo quando nenhum terminal de verdade está ligado (T9). Com um pedido do app em
    /// curso espera a vez dele, até `FLOOR_WAIT`, e só então relê clientes e tamanho: o terminal que se
    /// desliga no meio do clique não avisa de novo, e desistir deixaria a janela abaixo do mínimo.
    pub async fn floor(&self) {
        let Ok(_busy) = tokio::time::timeout(FLOOR_WAIT, self.parts.busy.lock()).await else {
            tracing::debug!(session = %self.parts.name, code = "mods_floor_wait", "tamanho mínimo do terminal não reposto");
            return;
        };
        let undo = Undo::default();
        if let Err(error) = click::floor(&self.ctx(Instant::now() + FLOOR_BUDGET, &undo)).await {
            tracing::debug!(session = %self.parts.name, code = %error.code, "tamanho mínimo do terminal não reposto");
        }
    }

    /// Liga o vigia (T10): a cada aviso de cliente ou de janela, relê clientes e tamanho e repõe o mínimo se
    /// ninguém estiver ligado. Um aumento de altura em curso não precisa de aviso: o `window-size latest`
    /// que acompanha todo redimensionamento já entrega o tamanho a quem se ligar. No Windows o
    /// `watch_notices` recusa, e o mínimo volta no `prepare` de cada operação.
    pub fn watch(self: &Arc<Self>, mux_argv: &[String]) {
        match crate::terminal_control::watch_notices(mux_argv, &self.parts.name) {
            Ok((mut notices, reader)) => {
                let link = Arc::downgrade(self);
                let task = tokio::spawn(async move {
                    let _reader = AbortOnDrop(reader);
                    while notices.recv().await.is_some() {
                        // Uma reposição atende a rajada inteira: ela lê o estado de agora.
                        while notices.try_recv().is_ok() {}
                        let Some(link) = link.upgrade() else { return };
                        link.floor().await;
                    }
                });
                if let Some(old) = self.watch.lock().unwrap().replace(task) { old.abort(); }
            }
            // No Windows a recusa é o esperado (ruling C7): não é aviso.
            Err(error) if cfg!(windows) => tracing::debug!(session = %self.parts.name, code = error.0, "vigia de tamanho do terminal recusado"),
            Err(error) => tracing::warn!(session = %self.parts.name, code = error.0, "vigia de tamanho do terminal não subiu"),
        }
    }

    fn stop_watch(&self) {
        if let Some(task) = self.watch.lock().unwrap().take() { task.abort(); }
    }
}

/// O elo que ninguém mais tem não deixa o cliente de controle do vigia ligado à sessão.
impl Drop for TerminalLink {
    fn drop(&mut self) { self.stop_watch(); }
}

impl SurfaceLink for TerminalLink {
    /// O pedido roda numa tarefa própria (`click::spawn`): se a rota desistir no fim do orçamento, ele não
    /// começa ação nova e a limpeza roda mesmo assim. A resposta chega antes da limpeza.
    fn call(&self, call: ModsCall, deadline: Instant) -> CallFuture {
        // Com terminal não há por onde digitar no campo do mod (fora do escopo desta entrega): recusa sem
        // reservar o pane.
        if matches!(call, ModsCall::Input { .. }) {
            return Box::pin(async { Err(no_typing()) });
        }
        let (_task, reply) = click::spawn(self.parts.clone(), call, deadline);
        Box::pin(async move { reply.await.unwrap_or_else(|_| Err(no_answer())) })
    }
}

impl TerminalProbe for TerminalLink {
    fn read_shown(&self) -> ShownFuture {
        let parts = self.parts.clone();
        Box::pin(async move {
            let undo = Undo::default();
            let ctx = Ctx { name: &parts.name, pane: parts.pane.as_ref(), mods: &parts.mods, limits: &parts.limits,
                until: Instant::now() + SHOWN_READ_MAX, undo: &undo, life: parts.life };
            click::read_shown(&ctx).await
        })
    }

    /// Encerra o vigia e, com ele, o cliente de controle (`kill_on_drop`).
    fn stop(&self) { self.stop_watch(); }
}
