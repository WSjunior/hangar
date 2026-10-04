use std::sync::{Arc, Mutex};
use std::time::Duration;
use hangar_server::costs_failure::{Decision, Failure, FailureEvent, FailureReason, Part, PartFailures, ReplaySafety};

#[tokio::test]
async fn succeeds_on_fourth_attempt_and_pauses_only_before_fourth() {
    let policy = PartFailures::default();
    let attempts = Mutex::new(Vec::new());
    let events = Mutex::new(Vec::new());
    let pauses = Mutex::new(Vec::new());
    let result = policy.execute(&Part::Costs, |attempt| {
        attempts.lock().unwrap().push(attempt);
        std::future::ready(if attempt == 4 { Ok(42) } else { Err(Failure::read(FailureReason::NoDisk)) })
    }, |event| {
        events.lock().unwrap().push(event);
        std::future::ready(())
    }, |duration| {
        pauses.lock().unwrap().push((duration, attempts.lock().unwrap().clone()));
        std::future::ready(())
    }).await;
    assert_eq!(result, Decision::Rust(42));
    assert_eq!(*attempts.lock().unwrap(), [1, 2, 3, 4]);
    assert_eq!(*pauses.lock().unwrap(), [(Duration::from_secs(2), vec![1, 2, 3])]);
    let events = events.lock().unwrap();
    assert_eq!(events.len(), 3);
    assert!(events.iter().all(|event| !event.transferred));
}

#[tokio::test]
async fn four_failures_transfer_once_and_other_parts_keep_running() {
    let policy = PartFailures::default();
    let attempts = Mutex::new(Vec::new());
    let events = Mutex::new(Vec::new());
    let result = policy.execute(&Part::Usage, |attempt| {
        attempts.lock().unwrap().push(attempt);
        std::future::ready(Err::<(), _>(Failure::read(FailureReason::NoScopes)))
    }, |event| { events.lock().unwrap().push(event); std::future::ready(()) }, |_| std::future::ready(())).await;
    assert_eq!(result, Decision::Python);
    assert_eq!(*attempts.lock().unwrap(), [1, 2, 3, 4]);
    assert_eq!(events.lock().unwrap().iter().filter(|event| event.transferred).count(), 1);
    assert_eq!(events.lock().unwrap().iter().filter(|event| !event.transferred).count(), 4);
    let again = policy.execute(&Part::Usage, |_| async { panic!("a parte transferida não executa Rust") }, |_| async {}, |_| async {}).await;
    assert_eq!(again, Decision::<()>::Python);
    let other = policy.execute(&Part::Costs, |_| async { Ok::<_, Failure>("ok") }, |_| async {}, |_| async {}).await;
    assert_eq!(other, Decision::Rust("ok"));
}

#[tokio::test]
async fn possible_effect_transfers_without_replaying() {
    let policy = PartFailures::default();
    let attempts = Mutex::new(Vec::new());
    let result = policy.execute(&Part::Costs, |attempt| {
        attempts.lock().unwrap().push(attempt);
        std::future::ready(Err::<(), _>(Failure { reason: FailureReason::WorkerJoin, replay: ReplaySafety::PossibleUserEffect }))
    }, |_| async {}, |_| async { panic!("efeito possível não permite pausa ou repetição") }).await;
    assert_eq!(result, Decision::NoReplay);
    assert_eq!(*attempts.lock().unwrap(), [1]);
}

#[tokio::test]
async fn late_success_cannot_reactivate_a_transferred_part() {
    let policy = Arc::new(PartFailures::default());
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let late_policy = policy.clone();
    let late_started = started.clone();
    let late_release = release.clone();
    let late = tokio::spawn(async move {
        late_policy.execute(&Part::Costs, |_| {
            let started = late_started.clone(); let release = late_release.clone();
            async move { started.notify_one(); release.notified().await; Ok::<_, Failure>(7) }
        }, |_| async {}, |_| async {}).await
    });
    started.notified().await;
    assert_eq!(policy.execute(&Part::Costs, |_| async { Err::<(), _>(Failure::read(FailureReason::ReaderPanic)) }, |_| async {}, |_| async {}).await, Decision::Python);
    release.notify_one();
    assert_eq!(late.await.unwrap(), Decision::Python);
}

#[test]
fn journal_contains_only_closed_fields_and_bounded_session_names() {
    let event = FailureEvent::new(&Part::SessionCost("example-1".into()), FailureReason::Sqlite, 4, true).unwrap();
    assert_eq!(serde_json::to_value(event).unwrap(), serde_json::json!({"part":"session_cost","code":"sqlite","attempt":4,"transferred":true,"session":"example-1"}));
    assert!(FailureEvent::new(&Part::Costs, FailureReason::Sqlite, 0, false).is_none());
    assert!(FailureEvent::new(&Part::Usage, FailureReason::Sqlite, 5, false).is_none());
    assert!(FailureEvent::new(&Part::SessionCost("/secret/path".into()), FailureReason::Io, 1, false).is_none());
    assert!(FailureReason::NoScopes.message().contains("indisponíveis"));
}

#[tokio::test]
async fn concurrent_failures_share_recovery_and_transfer_only_once() {
    let policy = Arc::new(PartFailures::default());
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let request = || {
        let policy = policy.clone(); let barrier = barrier.clone();
        let attempts = attempts.clone(); let events = events.clone();
        tokio::spawn(async move {
            policy.execute(&Part::Usage, |attempt| {
                let barrier = barrier.clone(); let attempts = attempts.clone();
                async move {
                    attempts.lock().unwrap().push(attempt);
                    if attempt == 1 { barrier.wait().await; }
                    Err::<(), _>(Failure::read(FailureReason::NoDisk))
                }
            }, |event| { events.lock().unwrap().push(event); std::future::ready(()) }, |_| async {}).await
        })
    };
    let first = request(); let second = request();
    assert_eq!(first.await.unwrap(), Decision::Python);
    assert_eq!(second.await.unwrap(), Decision::Python);
    let attempts = attempts.lock().unwrap();
    assert_eq!(attempts.iter().filter(|&&attempt| attempt == 1).count(), 2);
    assert_eq!(attempts.iter().filter(|&&attempt| attempt > 1).copied().collect::<Vec<_>>(), [2, 3, 4]);
    assert_eq!(events.lock().unwrap().iter().filter(|event| event.transferred).count(), 1);
}

#[tokio::test]
async fn session_failure_does_not_transfer_another_session() {
    let policy = PartFailures::default();
    assert_eq!(policy.execute(&Part::SessionCost("example-a".into()), |_| async {
        Err::<(), _>(Failure::read(FailureReason::Io))
    }, |_| async {}, |_| async {}).await, Decision::Python);
    assert_eq!(policy.execute(&Part::SessionCost("example-b".into()), |_| async {
        Ok::<_, Failure>(3)
    }, |_| async {}, |_| async {}).await, Decision::Rust(3));
}

#[tokio::test]
async fn ordinary_values_do_not_emit_failures_or_start_recovery() {
    let policy = PartFailures::default();
    for normal in ["warming", "empty", "not-found", "unpriced"] {
        assert_eq!(policy.execute(&Part::ExchangeRate, |_| async { Ok::<_, Failure>(normal) }, |_| async {
            panic!("resposta normal não gera falha")
        }, |_| async { panic!("resposta normal não inicia pausa") }).await, Decision::Rust(normal));
    }
}

#[tokio::test]
async fn stalled_journal_is_bounded_and_cannot_block_transfer() {
    let policy = PartFailures::default();
    let events = Mutex::new(Vec::new());
    let result = tokio::time::timeout(Duration::from_secs(1), policy.execute(&Part::Costs, |_| async {
        Err::<(), _>(Failure { reason: FailureReason::WorkerJoin, replay: ReplaySafety::PossibleUserEffect })
    }, |event| {
        events.lock().unwrap().push(event.clone());
        async move { if !event.transferred { std::future::pending::<()>().await; } }
    }, |_| async {})).await.unwrap();
    assert_eq!(result, Decision::NoReplay);
    let events = events.lock().unwrap();
    assert_eq!(events.len(), 2);
    assert!(!events[0].transferred);
    assert!(events[1].transferred);
}

#[test]
fn all_failure_messages_are_static_and_codes_are_unique() {
    let reasons = [FailureReason::NoScopes, FailureReason::NoDisk, FailureReason::ReaderPanic,
        FailureReason::Sqlite, FailureReason::Json, FailureReason::WorkerJoin,
        FailureReason::InfoUnavailable, FailureReason::Io, FailureReason::NonFinite];
    let mut codes = std::collections::HashSet::new();
    for reason in reasons {
        assert!(codes.insert(reason.code()));
        assert!(!reason.message().contains('/'));
        assert!(!reason.message().contains("SELECT"));
    }
}

#[tokio::test]
async fn healthy_sessions_do_not_consume_recovery_storage() {
    let policy = PartFailures::default();
    for index in 0..1030 {
        let part = Part::SessionCost(format!("example-{index}"));
        assert_eq!(policy.execute(&part, |_| async { Ok::<_, Failure>(1) }, |_| async {}, |_| async {}).await,
            Decision::Rust(1));
    }
}

#[tokio::test]
async fn more_than_a_thousand_failed_sessions_remain_individually_sticky() {
    let policy = PartFailures::default();
    for index in 0..1030 {
        let part = Part::SessionCost(format!("example-{index}"));
        assert_eq!(policy.execute(&part, |_| async {
            Err::<(), _>(Failure::read(FailureReason::Io))
        }, |_| async {}, |_| async {}).await, Decision::Python);
    }
    assert_eq!(policy.execute(&Part::SessionCost("example-0".into()), |_| async {
        panic!("uma sessão transferida não pode ser expulsa do mapa")
    }, |_| async {}, |_| async {}).await, Decision::<()>::Python);
    assert_eq!(policy.execute(&Part::SessionCost("example-new".into()), |_| async {
        Ok::<_, Failure>(4)
    }, |_| async {}, |_| async {}).await, Decision::Rust(4));
}

#[tokio::test]
async fn invalid_session_name_is_passed_without_rust_or_journal() {
    let policy = PartFailures::default();
    for name in ["", "/private/path", &"x".repeat(65)] {
        assert_eq!(policy.execute(&Part::SessionCost(name.into()), |_| async {
            panic!("nome fora do contrato não executa o atalho")
        }, |_| async { panic!("nome inválido não entra no diário") }, |_| async {}).await,
            Decision::<()>::Python);
    }
}
