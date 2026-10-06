mod mods_support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use hangar_server::mods::state::*;
use mods_support::NoLink;
use serde_json::{Value, json};

/// Um `plugin_ui` com o painel `painel` e o botão `abrir` do mod `vitrine`: é dele que a janela do clique
/// toma a cópia.
fn with_button() -> Value {
    json!({"above": null, "panes": [{"id": "painel", "title": "Painel", "placement": "dock", "columns": 58,
        "tree": {"type": "Button", "props": {"key": "abrir", "label": "Abrir"}, "press": {"plugin": "vitrine", "handle": 1}}}],
        "shown_id": "painel", "columns": 110, "source": "surface"})
}

fn mods() -> Mods {
    let mods = Mods::default();
    mods.attach("s", 1, Arc::new(NoLink));
    mods
}

fn replayed(mods: &Mods, event: &str) -> Vec<Value> {
    mods.replay("s").into_iter().filter(|(name, _)| *name == event).map(|(_, data)| serde_json::from_str(&data).unwrap()).collect()
}

#[test]
fn attach_owns_and_forget_checks_generation() {
    let mods = mods();
    assert!(mods.owns("s") && !mods.owns("outra"));
    mods.forget("s", 2);
    assert!(mods.owns("s"), "geração diferente não esquece a sessão de agora");
    mods.forget("s", 1);
    assert!(!mods.owns("s") && mods.link("s").is_none());
}

#[test]
fn publish_keeps_the_latest_and_skips_repeats() {
    let mods = mods();
    let band = json!({"above": {"type": "Text"}, "panes": [], "shown_id": null, "columns": 110, "source": "surface"});
    assert!(mods.publish_ui("s", 1, &band));
    assert!(!mods.publish_ui("s", 1, &band), "igual ao último não sai de novo");
    assert!(!mods.publish_ui("s", 0, &json!({"above": null})), "geração velha não publica");
    assert_eq!(replayed(&mods, "plugin_ui"), vec![band]);
}

#[test]
fn toasts_follow_the_python_limits() {
    let mods = mods();
    mods.toast("s", 1, &"m".repeat(80), &"x".repeat(2500), 10);
    mods.toast("s", 1, "vitrine", "padrão", 0);
    mods.toast("s", 1, "vitrine", "longo", 10_000_000);
    mods.toast("s", 1, "vitrine", "   ", 4000);
    let toasts = replayed(&mods, "plugin_toast");
    assert_eq!(toasts.len(), 3, "texto vazio não vira aviso");
    assert_eq!(toasts[0]["text"].as_str().unwrap().chars().count(), 2000);
    assert_eq!(toasts[0]["plugin"].as_str().unwrap().chars().count(), 64);
    assert!(toasts[0]["timeoutMs"].as_u64().unwrap() <= 1000);
    assert!(toasts[1]["timeoutMs"].as_u64().unwrap() <= 4000 && toasts[1]["timeoutMs"].as_u64().unwrap() > 3000);
    assert!(toasts[2]["timeoutMs"].as_u64().unwrap() <= 300_000 && toasts[2]["timeoutMs"].as_u64().unwrap() > 299_000);
    assert!(toasts.iter().all(|toast| toast["id"].as_str().unwrap().starts_with("rs-")));
    for index in 0..25 { mods.toast("s", 1, "vitrine", &format!("n-{index}"), 9000); }
    let kept = replayed(&mods, "plugin_toast");
    assert_eq!(kept.len(), 20);
    assert_eq!(kept[0]["text"], "n-5", "o mais antigo sai primeiro");
}

#[tokio::test]
async fn click_effects_belong_to_the_open_click() {
    let mods = mods();
    mods.publish_ui("s", 1, &with_button());
    let attempt = mods.begin_click("s", "painel", "abrir");
    assert_eq!(mods.match_click("s", "painel", "outra"), None);
    assert_eq!(mods.match_click("s", "painel", "abrir").as_deref(), Some(attempt.as_str()));
    assert_eq!(mods.match_click("s", "painel", "abrir"), None, "o press só casa uma vez");
    assert!(!mods.opened("s", "outra-tentativa", "https://example.com"));
    assert!(mods.opened("s", &attempt, "https://example.com"));
    mods.copied("s", 1, "vitrine", "texto");
    assert_eq!(mods.finish_click("s", &attempt, Duration::from_millis(300)).await,
        (Some("texto".to_owned()), Some("https://example.com".to_owned())));
    assert!(replayed(&mods, "plugin_toast").is_empty(), "cópia de clique em aberto não vira aviso");
}

#[tokio::test]
async fn copy_from_another_mod_is_a_toast() {
    // A janela de 1,5 s é do mod dono do botão (A11): um relógio de outro mod que copia no meio não vai
    // para o aparelho de quem clicou.
    let mods = mods();
    mods.publish_ui("s", 1, &with_button());
    let attempt = mods.begin_click("s", "painel", "abrir");
    mods.copied("s", 1, "outro-mod", "texto de outro");
    assert_eq!(mods.finish_click("s", &attempt, Duration::from_millis(10)).await, (None, None));
    let toasts = replayed(&mods, "plugin_toast");
    assert_eq!((toasts[0]["text"].as_str(), toasts[0]["plugin"].as_str()), (Some("texto de outro"), Some("outro-mod")));
}

#[test]
fn copy_without_click_becomes_a_toast() {
    let mods = mods();
    mods.copied("s", 1, "vitrine", "Texto copiado pela vitrine (V44)");
    let toasts = replayed(&mods, "plugin_toast");
    assert_eq!((toasts[0]["text"].as_str(), toasts[0]["plugin"].as_str()), (Some("Texto copiado pela vitrine (V44)"), Some("vitrine")));
}

#[tokio::test]
async fn finish_waits_for_a_late_effect_only_when_the_plugin_matched() {
    let mods = mods();
    let attempt = mods.begin_click("s", "painel", "abrir");
    let started = Instant::now();
    assert_eq!(mods.finish_click("s", &attempt, Duration::from_millis(300)).await, (None, None));
    assert!(started.elapsed() < Duration::from_millis(100), "sem o plugin no press não há efeito a esperar");

    let attempt = mods.begin_click("s", "painel", "abrir");
    mods.match_click("s", "painel", "abrir").unwrap();
    let late = { let mods = mods.clone(); let attempt = attempt.clone();
        tokio::spawn(async move { tokio::time::sleep(Duration::from_millis(100)).await; mods.opened("s", &attempt, "https://example.com") }) };
    assert_eq!(mods.finish_click("s", &attempt, Duration::from_millis(300)).await.1.as_deref(), Some("https://example.com"));
    assert!(late.await.unwrap());
}
