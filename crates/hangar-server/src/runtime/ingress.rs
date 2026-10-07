//! Porta de entrada das escritas por nome de sessão. Quem troca de dono ou relança (Python) fecha;
//! a rota do Rust espera reabrir em vez de escrever numa entrada que está fechando.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;

#[derive(Clone, Default)]
pub struct IngressGates { inner: Arc<Mutex<HashMap<String, Gate>>> }

// Fechamentos em curso (contador, não flag: dois `close` não desfazem um ao outro) e escritas em curso.
struct Gate { closes: watch::Sender<usize>, passes: watch::Sender<usize> }

#[derive(Debug)]
pub struct GateClosed;
#[derive(Debug)]
pub struct IngressBusy;
pub struct IngressPass { gates: IngressGates, name: String }

/// Desfaz o fechamento de um `close` que desistiu ou foi cancelado no meio da espera.
struct CloseGuard<'a> { gates: &'a IngressGates, name: &'a str, armed: bool }
impl Drop for CloseGuard<'_> {
    fn drop(&mut self) { if self.armed { self.gates.open(self.name); } }
}

impl Gate {
    fn idle(&self) -> bool {
        *self.closes.borrow() == 0 && *self.passes.borrow() == 0 && self.closes.receiver_count() == 0 && self.passes.receiver_count() == 0
    }
}

impl IngressGates {
    fn with_gate<T>(&self, name: &str, f: impl FnOnce(&Gate) -> T) -> T {
        let mut map = self.inner.lock().unwrap();
        f(map.entry(name.into()).or_insert_with(|| Gate { closes: watch::channel(0).0, passes: watch::channel(0).0 }))
    }
    pub async fn enter(&self, name: &str, wait: Duration) -> Result<IngressPass, GateClosed> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            // Conferir e contar sob a mesma trava do `close`: senão um fechamento passa entre os dois.
            let mut closes = match self.with_gate(name, |g| {
                if *g.closes.borrow() > 0 { Err(g.closes.subscribe()) } else { g.passes.send_modify(|n| *n += 1); Ok(()) }
            }) {
                Ok(()) => return Ok(IngressPass { gates: self.clone(), name: name.into() }),
                Err(rx) => rx,
            };
            tokio::time::timeout_at(deadline, closes.wait_for(|c| *c == 0)).await.map_err(|_| GateClosed)?.map_err(|_| GateClosed)?;
        }
    }
    pub async fn close(&self, name: &str, wait: Duration) -> Result<(), IngressBusy> {
        let mut passes = self.with_gate(name, |g| { g.closes.send_modify(|n| *n += 1); g.passes.subscribe() });
        // Declarado antes de `passes`: o receptor cai primeiro e a porta desistida pode sair do mapa.
        let mut guard = CloseGuard { gates: self, name, armed: true };
        if tokio::time::timeout(wait, passes.wait_for(|n| *n == 0)).await.is_err() { return Err(IngressBusy); }
        guard.armed = false;
        Ok(())
    }
    /// Desfaz um fechamento (o contador não passa de zero).
    pub fn open(&self, name: &str) {
        let mut map = self.inner.lock().unwrap();
        let Some(g) = map.get(name) else { return };
        g.closes.send_modify(|n| *n = n.saturating_sub(1));
        // Aberta e sem escrita em curso é igual a não existir: o mapa não cresce com nomes antigos.
        if g.idle() { map.remove(name); }
    }
}

impl Drop for IngressPass {
    fn drop(&mut self) {
        let mut map = self.gates.inner.lock().unwrap();
        let Some(g) = map.get(&self.name) else { return };
        g.passes.send_modify(|n| *n -= 1);
        if g.idle() { map.remove(&self.name); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn close_waits_for_passes_and_blocks_new_ones() {
        let gates = IngressGates::default();
        let pass = gates.enter("k", Duration::from_millis(10)).await.unwrap();
        let g = gates.clone();
        let closing = tokio::spawn(async move { g.close("k", Duration::from_secs(1)).await.is_ok() });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!closing.is_finished());
        drop(pass);
        assert!(closing.await.unwrap());
        assert!(gates.enter("k", Duration::from_millis(20)).await.is_err());
        gates.open("k");
        assert!(gates.enter("k", Duration::from_millis(20)).await.is_ok());
    }

    #[tokio::test]
    async fn closed_gate_lets_writer_through_when_reopened_in_time() {
        let gates = IngressGates::default();
        assert!(gates.close("k", Duration::from_secs(1)).await.is_ok());
        let g = gates.clone();
        tokio::spawn(async move { tokio::time::sleep(Duration::from_millis(30)).await; g.open("k") });
        assert!(gates.enter("k", Duration::from_secs(1)).await.is_ok());
    }

    #[tokio::test]
    async fn close_gives_up_busy_and_reopens() {
        let gates = IngressGates::default();
        let _pass = gates.enter("k", Duration::from_millis(10)).await.unwrap();
        assert!(gates.close("k", Duration::from_millis(30)).await.is_err());
        assert!(gates.enter("k", Duration::from_millis(20)).await.is_ok());
    }

    #[tokio::test]
    async fn gates_are_per_name() {
        let gates = IngressGates::default();
        assert!(gates.close("a", Duration::from_millis(20)).await.is_ok());
        assert!(gates.enter("b", Duration::from_millis(20)).await.is_ok());
        assert!(gates.enter("a", Duration::from_millis(20)).await.is_err());
    }

    #[tokio::test]
    async fn idle_gate_is_dropped_from_map() {
        let gates = IngressGates::default();
        drop(gates.enter("k", Duration::from_millis(10)).await.unwrap());
        assert!(gates.inner.lock().unwrap().is_empty());
        gates.close("k", Duration::from_millis(10)).await.unwrap();
        gates.open("k");
        assert!(gates.inner.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn no_pass_after_close_returns_until_open() {
        let gates = IngressGates::default();
        let mut tasks = Vec::new();
        for i in 0..100u64 {
            let g = gates.clone();
            tasks.push(tokio::spawn(async move {
                tokio::time::sleep(Duration::from_micros(i * 50)).await;
                let pass = g.enter("k", Duration::from_millis(5)).await;
                if pass.is_ok() { tokio::time::sleep(Duration::from_millis(1)).await; }
                pass.is_ok()
            }));
        }
        tokio::time::sleep(Duration::from_micros(2000)).await;
        gates.close("k", Duration::from_secs(1)).await.unwrap();
        // Depois que o close volta, nenhum enter novo passa até o open.
        let late: Vec<_> = (0..100).map(|_| { let g = gates.clone(); tokio::spawn(async move { g.enter("k", Duration::from_millis(5)).await.is_ok() }) }).collect();
        for t in late { assert!(!t.await.unwrap()); }
        for t in tasks { let _ = t.await; }
        gates.open("k");
        assert!(gates.enter("k", Duration::from_millis(20)).await.is_ok());
    }

    #[tokio::test]
    async fn timed_out_close_does_not_reopen_another_close() {
        let gates = IngressGates::default();
        let pass = gates.enter("k", Duration::from_millis(10)).await.unwrap();
        assert!(gates.close("k", Duration::from_millis(30)).await.is_err());
        drop(pass);
        assert!(gates.close("k", Duration::from_millis(30)).await.is_ok());
        assert!(gates.enter("k", Duration::from_millis(20)).await.is_err());
        // Primeiro close que deu certo: um open por close bem-sucedido.
        let held = gates.enter("j", Duration::from_millis(10)).await.unwrap();
        let g = gates.clone();
        let first = tokio::spawn(async move { g.close("j", Duration::from_secs(5)).await });
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(gates.close("j", Duration::from_millis(20)).await.is_err());
        drop(held);
        assert!(first.await.unwrap().is_ok());
        assert!(gates.enter("j", Duration::from_millis(20)).await.is_err());
        gates.open("j");
        assert!(gates.enter("j", Duration::from_millis(20)).await.is_ok());
    }

    #[tokio::test]
    async fn two_successful_closes_need_two_opens() {
        let gates = IngressGates::default();
        gates.close("k", Duration::from_millis(10)).await.unwrap();
        gates.close("k", Duration::from_millis(10)).await.unwrap();
        gates.open("k");
        assert!(gates.enter("k", Duration::from_millis(20)).await.is_err());
        gates.open("k");
        assert!(gates.enter("k", Duration::from_millis(20)).await.is_ok());
        gates.open("k");
        gates.open("k");
        assert!(gates.enter("k", Duration::from_millis(20)).await.is_ok());
    }

    #[tokio::test]
    async fn cancelled_close_leaves_gate_open() {
        let gates = IngressGates::default();
        let _pass = gates.enter("k", Duration::from_millis(10)).await.unwrap();
        let g = gates.clone();
        let closing = tokio::spawn(async move { g.close("k", Duration::from_secs(30)).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        closing.abort();
        let _ = closing.await;
        assert!(gates.enter("k", Duration::from_millis(20)).await.is_ok());
    }
}
