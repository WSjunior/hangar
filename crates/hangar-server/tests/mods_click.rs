mod mods_support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use hangar_server::mods::click::{self, Ctx, Limits, Parts, Undo};
use hangar_server::mods::model::*;
use hangar_server::mods::screen::{find_in, read_screen};
use hangar_server::mods::state::*;
use mods_support::Probe;
use mods_support::pane::{Effect::*, FakePane, capture, view};
use serde_json::{Value, json};

const S: &str = "t";

fn vitrine() -> TerminalView {
    view(&[("vitrine-texto", "Texto", "V04-vitrine-texto", "Contar"), ("vitrine-botoes", "Botões", "V04-vitrine-botoes", "Contar"),
           ("vitrine-hover", "Hover", "V04-vitrine-hover", "Contar")])
}
fn pm() -> TerminalView {
    view(&[("pm-mock-pm", "xx-00000", "pm-a", "xxxxx"), ("pm-mock-mr", "MR ●2", "mr-a", "xxxxx"), ("pm-mock-jenkins", "Jenkins", "jenkins-a", "xxxxx")])
}
fn longo() -> TerminalView { view(&[("vitrine-longo", "Longo", "V37-meio", "Botão do meio")]) }
fn eight() -> TerminalView {
    view(&[("vitrine-texto", "Texto", "a", "Contar"), ("vitrine-botoes", "Botões", "b", "Contar"), ("vitrine-hover", "Hover", "c", "Contar"),
           ("vitrine-rico", "Mídia", "d", "x"), ("vitrine-quebrado", "Quebrado", "e", "x"), ("pm-mock-pm", "xx-00000", "pm-a", "xxxxx"),
           ("pm-mock-mr", "MR ●2", "mr-a", "xxxxx"), ("pm-mock-jenkins", "Jenkins", "jenkins-a", "xxxxx")])
}

fn setup(screen: &str, view: TerminalView) -> (Mods, FakePane) {
    let mods = Mods::default();
    mods.attach_terminal(S, "proc-t", 1, Arc::new(Probe::default()));
    mods.terminal_ui(S, view);
    let pane = FakePane::new(&mods, S, screen);
    (mods, pane)
}

/// Bem longe: o prazo não aperta, salvo nos testes dele.
fn far() -> Instant { Instant::now() + Duration::from_secs(30) }

/// Um pedido do app como o `spawn` o atende, sem a tarefa: o pedido e depois a limpeza.
async fn run(mods: &Mods, pane: &FakePane, until: Instant, call: ModsCall) -> Result<Value, ModsError> {
    let (limits, undo) = (Limits::quick(), Undo::default());
    let ctx = Ctx { name: S, pane, mods, limits: &limits, until, undo: &undo, life: 1 };
    let result = click::dispatch(&ctx, call).await;
    click::finish(&ctx).await;
    result
}
async fn press(mods: &Mods, pane: &FakePane, site: &str, key: &str) -> Result<Value, ModsError> {
    run(mods, pane, far(), ModsCall::Press { site: site.into(), key: key.into() }).await
}
async fn close(mods: &Mods, pane: &FakePane, site: &str) -> Result<Value, ModsError> {
    run(mods, pane, far(), ModsCall::Close { site: site.into() }).await
}
async fn show(mods: &Mods, pane: &FakePane, site: &str) -> Result<Value, ModsError> {
    run(mods, pane, far(), ModsCall::Show { site: site.into() }).await
}
fn code(result: Result<Value, ModsError>) -> String { result.unwrap_err().code }
/// Espera a condição com prazo de 5 s.
async fn until(check: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async { while !check() { tokio::time::sleep(Duration::from_millis(5)).await; } })
        .await.expect("condição esperada");
}
fn parts(mods: &Mods, pane: &Arc<FakePane>) -> Parts {
    Parts { name: S.into(), pane: pane.clone(), mods: mods.clone(), limits: Limits::quick(), busy: Arc::default(), life: 1 }
}

#[tokio::test]
async fn hidden_tab_is_activated_by_its_title_before_the_click() {
    let (mods, pane) = setup("tmux-400-vitrine-abas-150", vitrine());
    pane.on_click((0, 87), vec![Show("tmux-402-vitrine-texto-150")]);
    pane.on_click((1, 88), vec![Pressed("vitrine-texto", "V04-vitrine-texto")]);
    press(&mods, &pane, "vitrine-texto", "V04-vitrine-texto").await.unwrap();
    assert_eq!(pane.actions(), ["click 0 87", "click 1 88"]);
}

#[tokio::test]
async fn no_confirmation_refuses_without_repeating() {
    let (mods, pane) = setup("tmux-400-vitrine-abas-150", vitrine());
    assert_eq!(code(press(&mods, &pane, "vitrine-hover", "V04-vitrine-hover").await), "erro_mod_clique_sem_resposta");
    assert_eq!(pane.actions(), ["click 1 88"]);
}

#[tokio::test]
async fn click_follows_a_label_that_moved() {
    let (mods, pane) = setup("tmux-400-vitrine-abas-150", vitrine());
    pane.queue(&["tmux-400-vitrine-abas-150", "tmux-240-ao-lado-110"]);
    let titles: Vec<String> = ["Texto", "Botões", "Hover"].map(String::from).to_vec();
    let narrow = read_screen(&capture("tmux-240-ao-lado-110"), 110, 45, &titles, Some("▸ xx-00000"));
    let moved = find_in(&narrow, "Contar", narrow.body.as_ref().unwrap())[0];
    assert_ne!(moved, (1, 88));
    pane.on_click((moved.0 as u16, moved.1 as u16), vec![Pressed("vitrine-hover", "V04-vitrine-hover")]);
    press(&mods, &pane, "vitrine-hover", "V04-vitrine-hover").await.unwrap();
    assert_eq!(pane.actions(), [format!("click {} {}", moved.0, moved.1)]);
}

#[tokio::test]
async fn dialog_only_allows_the_shown_pane_and_its_close() {
    let (mods, pane) = setup("tmux-510-dialogo-150", pm());
    assert_eq!(code(press(&mods, &pane, "pm-mock-mr", "mr-a").await), "erro_mod_dialogo_aberto");
    assert_eq!(code(press(&mods, &pane, "above-prompt", "pm-abrir").await), "erro_mod_dialogo_aberto");
    assert_eq!(code(show(&mods, &pane, "pm-mock-mr").await), "erro_mod_dialogo_aberto");
    assert!(pane.actions().is_empty());
    pane.on_click((0, 148), vec![CloseAll]);
    close(&mods, &pane, "pm-mock-pm").await.unwrap();
    assert_eq!(pane.actions(), ["click 0 148"]);
}

#[tokio::test]
async fn dialog_with_the_boxed_pane_refuses_everything() {
    let (mods, pane) = setup("psmux-700-dialogo-com-caixa-100", pm());
    assert_eq!(code(press(&mods, &pane, "pm-mock-pm", "pm-a").await), "erro_mod_dialogo_aberto");
    assert_eq!(code(close(&mods, &pane, "pm-mock-pm").await), "erro_mod_dialogo_aberto");
    assert!(pane.actions().is_empty());
}

#[tokio::test]
async fn close_activates_the_tab_and_clicks_the_exact_cell() {
    let (mods, pane) = setup("tmux-05-antes-de-fechar-150", pm());
    pane.on_click((0, 104), vec![Show("tmux-02-apos-clicar-mr-150")]);
    pane.on_click((0, 148), vec![CloseAll]);
    close(&mods, &pane, "pm-mock-mr").await.unwrap();
    assert_eq!(pane.actions(), ["click 0 104", "click 0 148"]);
}

#[tokio::test]
async fn collapsed_band_is_expanded_before_the_click() {
    let (mods, pane) = setup("tmux-140-faixa-recolhida-150", pm());
    pane.on_click((38, 11), vec![Show("tmux-452-faixa-expandida-150")]);
    pane.on_click((34, 4), vec![Pressed("above-prompt", "pm-abrir")]);
    press(&mods, &pane, "above-prompt", "pm-abrir").await.unwrap();
    assert_eq!(pane.actions(), ["click 38 11", "click 34 4"]);
}

#[tokio::test]
async fn shrunk_band_with_a_terminal_attached_is_unreachable() {
    let (mods, pane) = setup("tmux-240-caixa-104", vitrine());
    assert_eq!(code(press(&mods, &pane, "above-prompt", "pm-abrir").await), "erro_mod_painel_nao_alcancavel");
    assert!(pane.actions().is_empty());
}

#[tokio::test]
async fn without_a_terminal_the_floor_is_restored() {
    let (mods, pane) = setup("tmux-240-caixa-104", vitrine());
    pane.clients(0);
    let (limits, undo) = (Limits::quick(), Undo::default());
    click::floor(&Ctx { name: S, pane: &pane, mods: &mods, limits: &limits, until: far(), undo: &undo, life: 1 }).await.unwrap();
    assert_eq!(pane.actions()[0], "resize 144 45");
}

#[tokio::test]
async fn wheel_follows_the_offset_with_a_terminal_attached() {
    let (mods, pane) = setup("tmux-230-longo-topo-150", longo());
    pane.on_wheel(vec![Show("tmux-232-longo-meio-150"), Scroll("vitrine-longo", 67)]);
    pane.on_click((38, 92), vec![Pressed("vitrine-longo", "V37-meio")]);
    press(&mods, &pane, "vitrine-longo", "V37-meio").await.unwrap();
    assert_eq!(pane.actions(), ["wheel 20 115 true", "click 38 92"]);
}

#[tokio::test]
async fn without_a_terminal_the_window_is_stretched_and_given_back() {
    let (mods, pane) = setup("tmux-230-longo-topo-150", longo());
    pane.clients(0);
    pane.on_resize(click::TALL_ROWS, vec![Show("tmux-232-longo-meio-150")]);
    pane.on_click((38, 92), vec![Pressed("vitrine-longo", "V37-meio")]);
    press(&mods, &pane, "vitrine-longo", "V37-meio").await.unwrap();
    // A altura volta na limpeza, depois do clique.
    assert_eq!(pane.actions(), ["resize 150 250", "click 38 92", "resize 150 45"]);
}

#[tokio::test]
async fn wheel_without_the_label_goes_down_then_up_and_refuses() {
    let (mods, pane) = setup("tmux-230-longo-topo-150", longo());
    assert_eq!(code(press(&mods, &pane, "vitrine-longo", "V37-meio").await), "erro_mod_painel_nao_alcancavel");
    assert_eq!(pane.actions(), ["wheel 20 115 true", "wheel 20 115 false"]);
}

#[tokio::test]
async fn show_clicks_the_title_and_publishes_the_shown_pane() {
    let (mods, pane) = setup("tmux-01-tres-paineis-150", pm());
    pane.on_click((0, 104), vec![Show("tmux-02-apos-clicar-mr-150")]);
    assert_eq!(show(&mods, &pane, "pm-mock-mr").await.unwrap(), json!({"shown_id": "pm-mock-mr"}));
    let ui: Value = serde_json::from_str(&mods.replay(S).into_iter().rev().find(|(e, _)| *e == "plugin_ui").unwrap().1).unwrap();
    assert_eq!(ui["shown_id"], "pm-mock-mr");
}

#[tokio::test]
async fn a_click_of_a_replaced_life_does_nothing_in_the_new_one() {
    // A sessão reabriu com outro processo e o mesmo nome: o pedido da vida 1 não lê o espelho nem clica na nova.
    let (mods, pane) = setup("tmux-01-tres-paineis-150", pm());
    mods.attach_terminal(S, "proc-novo", 2, Arc::new(Probe::default()));
    mods.terminal_ui(S, pm());
    pane.on_click((0, 104), vec![Show("tmux-02-apos-clicar-mr-150")]);
    assert_eq!(code(show(&mods, &pane, "pm-mock-mr").await), "erro_mod_painel_inexistente");
    assert!(pane.actions().is_empty());
    let ui: Value = serde_json::from_str(&mods.replay(S).into_iter().rev().find(|(e, _)| *e == "plugin_ui").unwrap().1).unwrap();
    assert_eq!(ui["shown_id"], "pm-mock-jenkins", "o painel na frente da vida nova é o do espelho dela");
}

#[tokio::test]
async fn show_refuses_without_fullscreen_and_with_the_title_off_the_row() {
    let (mods, pane) = setup("tmux-01-tres-paineis-150", pm());
    pane.mouse(false);
    assert_eq!(code(show(&mods, &pane, "pm-mock-mr").await), "erro_mod_mouse_desligado");
    let (mods, pane) = setup("tmux-262-abas-transbordando-150", eight());
    assert_eq!(code(show(&mods, &pane, "pm-mock-jenkins").await), "erro_mod_painel_nao_alcancavel");
}

#[tokio::test]
async fn read_shown_reads_the_active_tab() {
    let (limits, undo) = (Limits::quick(), Undo::default());
    let (mods, pane) = setup("tmux-02-apos-clicar-mr-150", pm());
    assert_eq!(click::read_shown(&Ctx { name: S, pane: &pane, mods: &mods, limits: &limits, until: far(), undo: &undo, life: 1 }).await.as_deref(), Some("pm-mock-mr"));
    let (mods, pane) = setup("psmux-700-dialogo-com-caixa-100", pm());
    assert_eq!(click::read_shown(&Ctx { name: S, pane: &pane, mods: &mods, limits: &limits, until: far(), undo: &undo, life: 1 }).await, None);
}

#[tokio::test]
async fn a_short_deadline_sends_nothing_to_the_mod() {
    // Faixa inteira e o rótulo achado: só falta o clique, e não sobra tempo para a confirmação (300 ms de
    // `confirm` no `quick` mais 300 ms de folga). O clique não sai.
    let (mods, pane) = setup("tmux-452-faixa-expandida-150", pm());
    let result = run(&mods, &pane, Instant::now() + Duration::from_millis(400), ModsCall::Press { site: "above-prompt".into(), key: "pm-abrir".into() }).await;
    assert_eq!(code(result), "erro_mod_clique_sem_resposta");
    assert!(pane.actions().is_empty(), "nenhuma ação sem tempo para ela: {:?}", pane.actions());
}

#[tokio::test]
async fn the_task_answers_then_cleans_up_and_releases_the_pane() {
    let (mods, pane) = setup("tmux-230-longo-topo-150", longo());
    let pane = Arc::new(pane);
    pane.clients(0);
    pane.on_resize(click::TALL_ROWS, vec![Show("tmux-232-longo-meio-150")]);
    pane.on_click((38, 92), vec![Pressed("vitrine-longo", "V37-meio")]);
    let (task, answer) = click::spawn(parts(&mods, &pane), ModsCall::Press { site: "vitrine-longo".into(), key: "V37-meio".into() }, far());
    assert_eq!(answer.await.unwrap().unwrap(), json!({}));
    task.await.unwrap();
    assert_eq!(pane.actions(), ["resize 150 250", "click 38 92", "resize 150 45"]);
    assert!(!pane.held(), "a reserva do pane é solta no fim");
}

#[tokio::test]
async fn a_cut_in_the_middle_of_the_stretch_gives_the_height_back() {
    let (mods, pane) = setup("tmux-230-longo-topo-150", longo());
    let pane = Arc::new(pane);
    pane.clients(0);
    pane.on_resize(click::TALL_ROWS, vec![Show("tmux-232-longo-meio-150")]);
    pane.stall_on("click 38 92");
    let (task, _answer) = click::spawn(parts(&mods, &pane), ModsCall::Press { site: "vitrine-longo".into(), key: "V37-meio".into() }, far());
    until(|| pane.actions().contains(&"click 38 92".to_string())).await;
    assert!(pane.held());
    // A tarefa some no meio: a guarda passa a limpeza a outra tarefa.
    task.abort();
    until(|| pane.actions().contains(&"resize 150 45".to_string()) && !pane.held()).await;
    assert_eq!(pane.actions(), ["resize 150 250", "click 38 92", "resize 150 45"]);
}

#[tokio::test]
async fn a_cut_in_the_middle_of_the_cleanup_redoes_what_was_left() {
    // A tarefa some com a volta da altura em curso: a guarda refaz a volta e solta o pane.
    let (mods, pane) = setup("tmux-230-longo-topo-150", longo());
    let pane = Arc::new(pane);
    pane.clients(0);
    pane.on_resize(click::TALL_ROWS, vec![Show("tmux-232-longo-meio-150")]);
    pane.on_click((38, 92), vec![Pressed("vitrine-longo", "V37-meio")]);
    pane.stall_on("resize 150 45");
    let (task, answer) = click::spawn(parts(&mods, &pane), ModsCall::Press { site: "vitrine-longo".into(), key: "V37-meio".into() }, far());
    assert_eq!(answer.await.unwrap().unwrap(), json!({}));
    until(|| pane.actions().contains(&"resize 150 45".to_string())).await;
    assert!(pane.held());
    task.abort();
    until(|| !pane.held()).await;
    assert_eq!(pane.actions(), ["resize 150 250", "click 38 92", "resize 150 45", "resize 150 45"]);
}

#[tokio::test]
async fn a_give_back_that_fails_in_the_request_is_retried_in_the_cleanup() {
    // A janela esticada não mostra o rótulo, e a volta imediata da altura fica sem resposta até o prazo:
    // a altura continua pendente e a limpeza a devolve.
    let (mods, pane) = setup("tmux-230-longo-topo-150", longo());
    pane.clients(0);
    pane.stall_on("resize 150 45");
    let until = Instant::now() + Duration::from_millis(1200);
    let result = run(&mods, &pane, until, ModsCall::Press { site: "vitrine-longo".into(), key: "V37-meio".into() }).await;
    assert_eq!(code(result), "erro_mod_clique_sem_resposta");
    assert_eq!(pane.actions(), ["resize 150 250", "resize 150 45", "resize 150 45"]);
}

#[tokio::test]
async fn one_request_at_a_time_in_the_pane_counting_the_cleanup() {
    let (mods, pane) = setup("tmux-230-longo-topo-150", longo());
    let pane = Arc::new(pane);
    let parts = parts(&mods, &pane);
    let held = parts.busy.clone().try_lock_owned().unwrap();
    let (_task, answer) = click::spawn(parts, ModsCall::Show { site: "vitrine-longo".into() }, Instant::now() + Duration::from_millis(100));
    assert_eq!(code(answer.await.unwrap()), "erro_mod_clique_sem_resposta", "a vez não veio no prazo");
    assert!(pane.actions().is_empty() && !pane.held());
    drop(held);
}
