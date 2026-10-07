//! Porta de entrada das escritas por nome de sessão. Quem troca de dono ou relança (Python) fecha;
//! a rota do Rust espera reabrir em vez de escrever numa entrada que está fechando.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;

#[derive(Clone, Default)]
pub struct IngressGates { inner: Arc<Mutex<HashMap<String, Gate>>> }

struct Gate { closed: watch::Sender<bool>, passes: watch::Sender<usize> }

#[derive(Debug)]
pub struct GateClosed;
#[derive(Debug)]
pub struct IngressBusy;
pub struct IngressPass { gates: IngressGates, key: String }

impl IngressGates {
    fn with_gate<T>(&self, key: &str, f: impl FnOnce(&Gate) -> T) -> T {
        let mut map = self.inner.lock().unwrap();
        f(map.entry(key.into()).or_insert_with(|| Gate { closed: watch::channel(false).0, passes: watch::channel(0).0 }))
    }
    pub async fn enter(&self, key: &str, wait: Duration) -> Result<IngressPass, GateClosed> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            // Conferir e contar sob a mesma trava do `close`: senão um fechamento passa entre os dois.
            let mut closed = match self.with_gate(key, |g| {
                if *g.closed.borrow() { Err(g.closed.subscribe()) } else { g.passes.send_modify(|n| *n += 1); Ok(()) }
            }) {
                Ok(()) => return Ok(IngressPass { gates: self.clone(), key: key.into() }),
                Err(rx) => rx,
            };
            tokio::time::timeout_at(deadline, closed.wait_for(|c| !*c)).await.map_err(|_| GateClosed)?.map_err(|_| GateClosed)?;
        }
    }
    pub async fn close(&self, key: &str, wait: Duration) -> Result<(), IngressBusy> {
        let mut passes = self.with_gate(key, |g| { g.closed.send_replace(true); g.passes.subscribe() });
        if tokio::time::timeout(wait, passes.wait_for(|n| *n == 0)).await.is_err() {
            // Quem fecha desiste com erro; a porta não pode ficar fechada sem dono.
            self.open(key);
            return Err(IngressBusy);
        }
        Ok(())
    }
    pub fn open(&self, key: &str) {
        let mut map = self.inner.lock().unwrap();
        let Some(g) = map.get(key) else { return };
        g.closed.send_replace(false);
        // Aberta e sem escrita em curso é igual a não existir: o mapa não cresce com nomes antigos.
        if *g.passes.borrow() == 0 && g.closed.receiver_count() == 0 && g.passes.receiver_count() == 0 { map.remove(key); }
    }
}

impl Drop for IngressPass {
    fn drop(&mut self) {
        let mut map = self.gates.inner.lock().unwrap();
        let Some(g) = map.get(&self.key) else { return };
        g.passes.send_modify(|n| *n -= 1);
        if *g.passes.borrow() == 0 && !*g.closed.borrow() && g.closed.receiver_count() == 0 && g.passes.receiver_count() == 0 {
            map.remove(&self.key);
        }
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
}
