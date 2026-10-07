//! Cotação com tentativa inicial síncrona e revalidação sem prender o relatório.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(3600);
const RATE_URL: &str = "https://economia.awesomeapi.com.br/json/last/USD-BRL";

#[derive(Default)]
struct State {
    rate: Option<f64>,
    attempted_at: Option<Duration>,
    refreshing: bool,
}

#[derive(Clone)]
pub struct Fx {
    state: Arc<Mutex<State>>,
    fetch: Arc<dyn Fn() -> Option<f64> + Send + Sync>,
    clock: Arc<dyn Fn() -> Duration + Send + Sync>,
}

impl Default for Fx {
    fn default() -> Self { Self::new() }
}

impl Fx {
    pub fn new() -> Self { Self::with_fetch(fetch_online) }

    pub fn with_fetch(fetch: impl Fn() -> Option<f64> + Send + Sync + 'static) -> Self {
        let started = Instant::now();
        Self::with_fetch_and_clock(fetch, move || started.elapsed())
    }

    #[doc(hidden)]
    pub fn with_fetch_and_clock(
        fetch: impl Fn() -> Option<f64> + Send + Sync + 'static,
        clock: impl Fn() -> Duration + Send + Sync + 'static,
    ) -> Self {
        Self { state: Arc::new(Mutex::new(State::default())), fetch: Arc::new(fetch), clock: Arc::new(clock) }
    }

    pub fn usd_brl(&self) -> Option<f64> {
        let now = (self.clock)();
        let mut state = self.state.lock().unwrap();
        if state.refreshing || state.attempted_at.is_some_and(|at| now.saturating_sub(at) < TTL) {
            return state.rate;
        }
        let first = state.attempted_at.is_none();
        let stale = state.rate;
        // Registrar a tentativa antes da rede também limita as chamadas durante uma falha.
        state.attempted_at = Some(now);
        state.refreshing = true;
        drop(state);
        if first {
            self.refresh();
            self.state.lock().unwrap().rate
        } else {
            let fx = self.clone();
            if std::thread::Builder::new().name("usd-brl".into()).spawn(move || fx.refresh()).is_err() {
                self.state.lock().unwrap().refreshing = false;
                tracing::warn!(code = "cotacao");
            }
            stale
        }
    }

    fn refresh(&self) {
        let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.fetch)())) {
            Ok(result) => result,
            Err(payload) => {
                // Uma falha do worker precisa continuar visível nas novas tentativas.
                let mut state = self.state.lock().unwrap();
                state.refreshing = false;
                state.attempted_at = None;
                drop(state);
                std::panic::resume_unwind(payload);
            }
        };
        let mut state = self.state.lock().unwrap();
        if let Some(rate) = result { state.rate = Some(rate); }
        else { tracing::warn!(code = "cotacao"); }
        state.refreshing = false;
    }
}

fn fetch_online() -> Option<f64> {
    // O runtime próprio fica fora de qualquer runtime que chamou a rota.
    std::thread::Builder::new().name("usd-brl-http".into()).spawn(|| {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?;
        runtime.block_on(async {
            let client = reqwest::Client::builder().timeout(Duration::from_secs(3)).build().ok()?;
            let response = client.get(RATE_URL).send().await.ok()?.error_for_status().ok()?;
            let body = response.bytes().await.ok()?;
            let value: serde_json::Value = serde_json::from_slice(&body).ok()?;
            let bid = &value["USDBRL"]["bid"];
            bid.as_str().and_then(|text| text.parse().ok()).or_else(|| bid.as_f64())
        })
    }).ok()?.join().ok()?
}
