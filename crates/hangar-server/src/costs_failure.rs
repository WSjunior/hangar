//! Reserva por parte durante a vida do processo, sem conteúdo de erros no diário.

use serde::Serialize;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const TOTAL_ATTEMPTS: u8 = 4;
pub const RETRY_PAUSE: Duration = Duration::from_secs(2);
pub const JOURNAL_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Part {
    Costs,
    Usage,
    ExchangeRate,
    SessionCost(String),
}

impl Part {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Costs => "costs",
            Self::Usage => "usage",
            Self::ExchangeRate => "exchange_rate",
            Self::SessionCost(_) => "session_cost",
        }
    }

    pub fn session(&self) -> Option<&str> {
        match self {
            Self::SessionCost(name) => Some(name),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureReason {
    NoScopes,
    NoDisk,
    ReaderPanic,
    Sqlite,
    Json,
    WorkerJoin,
    InfoUnavailable,
    Io,
    NonFinite,
}

impl FailureReason {
    pub fn code(self) -> &'static str {
        match self {
            Self::NoScopes => "no_scopes",
            Self::NoDisk => "no_disk",
            Self::ReaderPanic => "reader_panic",
            Self::Sqlite => "sqlite",
            Self::Json => "json",
            Self::WorkerJoin => "worker_join",
            Self::InfoUnavailable => "info_unavailable",
            Self::Io => "io",
            Self::NonFinite => "non_finite",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::NoScopes => "Escopos de custos indisponíveis.",
            Self::NoDisk => "Índice de custos indisponível.",
            Self::ReaderPanic => "Falha no leitor de custos.",
            Self::Sqlite => "Falha no índice de custos.",
            Self::Json => "Falha ao preparar a resposta de custos.",
            Self::WorkerJoin => "Falha no processamento de custos.",
            Self::InfoUnavailable => "Informações da sessão indisponíveis.",
            Self::Io => "Falha na leitura dos dados de custos.",
            Self::NonFinite => "Valor de custos ou uso inválido.",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplaySafety {
    DerivedRead,
    PossibleUserEffect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Failure {
    pub reason: FailureReason,
    pub replay: ReplaySafety,
}

impl Failure {
    pub fn read(reason: FailureReason) -> Self {
        Self {
            reason,
            replay: ReplaySafety::DerivedRead,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FailureEvent {
    pub part: &'static str,
    pub code: &'static str,
    pub attempt: u8,
    pub transferred: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

impl FailureEvent {
    pub fn new(part: &Part, reason: FailureReason, attempt: u8, transferred: bool) -> Option<Self> {
        if !(1..=TOTAL_ATTEMPTS).contains(&attempt) {
            return None;
        }
        let session = match part.session() {
            Some(name) if valid_session(name) => Some(name.to_owned()),
            Some(_) => return None,
            None => None,
        };
        Some(Self {
            part: part.code(),
            code: reason.code(),
            attempt,
            transferred,
            session,
        })
    }
}

fn valid_session(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

#[derive(Debug, Eq, PartialEq)]
pub enum Decision<T> {
    Rust(T),
    Python,
    NoReplay,
}

#[derive(Default)]
struct PartState {
    transferred: AtomicBool,
    recovery: tokio::sync::Mutex<()>,
}

#[derive(Default)]
pub struct PartFailures {
    costs: Arc<PartState>,
    usage: Arc<PartState>,
    exchange_rate: Arc<PartState>,
    sessions: Mutex<HashMap<String, Arc<PartState>>>,
}

impl PartFailures {
    fn state(&self, part: &Part, create: bool) -> Option<Arc<PartState>> {
        match part {
            Part::Costs => Some(self.costs.clone()),
            Part::Usage => Some(self.usage.clone()),
            Part::ExchangeRate => Some(self.exchange_rate.clone()),
            Part::SessionCost(name) => {
                if !valid_session(name) {
                    return None;
                }
                let mut sessions = self
                    .sessions
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Some(state) = sessions.get(name) {
                    return Some(state.clone());
                }
                if !create {
                    return None;
                }
                let state = Arc::new(PartState::default());
                sessions.insert(name.clone(), state.clone());
                Some(state)
            }
        }
    }

    fn transferred(&self, part: &Part) -> bool {
        self.state(part, false)
            .is_some_and(|state| state.transferred.load(Ordering::Acquire))
    }

    pub async fn execute<T, Execute, ExecuteFuture, Report, ReportFuture, Pause, PauseFuture>(
        &self,
        part: &Part,
        mut execute: Execute,
        mut report: Report,
        mut pause: Pause,
    ) -> Decision<T>
    where
        Execute: FnMut(u8) -> ExecuteFuture,
        ExecuteFuture: Future<Output = Result<T, Failure>>,
        Report: FnMut(FailureEvent) -> ReportFuture,
        ReportFuture: Future<Output = ()>,
        Pause: FnMut(Duration) -> PauseFuture,
        PauseFuture: Future<Output = ()>,
    {
        if part.session().is_some_and(|name| !valid_session(name)) {
            return Decision::Python;
        }
        if self.transferred(part) {
            return Decision::Python;
        }
        let mut failure = match execute(1).await {
            Ok(value) => {
                return if self.transferred(part) {
                    Decision::Python
                } else {
                    Decision::Rust(value)
                };
            }
            Err(failure) => failure,
        };
        Self::report(part, failure.reason, 1, false, &mut report).await;
        // Só a falha exige estado durável; ler sessões saudáveis não aumenta o mapa.
        let Some(state) = self.state(part, true) else {
            return Decision::Python;
        };
        let _recovery = state.recovery.lock().await;
        if state.transferred.load(Ordering::Acquire) {
            return if failure.replay == ReplaySafety::PossibleUserEffect {
                Decision::NoReplay
            } else {
                Decision::Python
            };
        }
        for attempt in 2..=TOTAL_ATTEMPTS {
            if failure.replay == ReplaySafety::PossibleUserEffect {
                Self::transfer(&state, part, failure.reason, attempt - 1, &mut report).await;
                return Decision::NoReplay;
            }
            if attempt == TOTAL_ATTEMPTS {
                pause(RETRY_PAUSE).await;
            }
            if state.transferred.load(Ordering::Acquire) {
                return Decision::Python;
            }
            failure = match execute(attempt).await {
                Ok(value) => {
                    return if state.transferred.load(Ordering::Acquire) {
                        Decision::Python
                    } else {
                        Decision::Rust(value)
                    };
                }
                Err(failure) => failure,
            };
            Self::report(part, failure.reason, attempt, false, &mut report).await;
        }
        Self::transfer(&state, part, failure.reason, TOTAL_ATTEMPTS, &mut report).await;
        if failure.replay == ReplaySafety::PossibleUserEffect {
            Decision::NoReplay
        } else {
            Decision::Python
        }
    }

    async fn transfer<Report, ReportFuture>(
        state: &PartState,
        part: &Part,
        reason: FailureReason,
        attempt: u8,
        report: &mut Report,
    ) where
        Report: FnMut(FailureEvent) -> ReportFuture,
        ReportFuture: Future<Output = ()>,
    {
        if state
            .transferred
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            Self::report(part, reason, attempt, true, report).await;
        }
    }

    async fn report<Report, ReportFuture>(
        part: &Part,
        reason: FailureReason,
        attempt: u8,
        transferred: bool,
        report: &mut Report,
    ) where
        Report: FnMut(FailureEvent) -> ReportFuture,
        ReportFuture: Future<Output = ()>,
    {
        tracing::warn!(
            part = part.code(),
            code = reason.code(),
            reason = reason.message(),
            attempt,
            transferred
        );
        if let Some(event) = FailureEvent::new(part, reason, attempt, transferred) {
            // O diário não pode atrasar a reserva nem iniciar outra recuperação.
            if tokio::time::timeout(JOURNAL_TIMEOUT, report(event))
                .await
                .is_err()
            {
                tracing::warn!(
                    code = "failure_journal_timeout",
                    "Diário interno indisponível."
                );
            }
        }
    }
}
