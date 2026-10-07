//! A falha na tela (spec "Falha e relatório"): a frase e o botão do código (`codes.rs`), "Ver detalhes" e, logo abaixo, o
//! relatório com o texto exato que sai e a caixa de envio. Falha sem código, ou com código fora da tabela, é a falha não
//! prevista: a última mensagem do script vira a frase.
use super::*;
use super::agent::Agent;
use super::codes::{self, Fix};
use super::flow::Screen;
use super::marks::Progress;
use super::report::Payload;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Failure {
    /// O `##HANGAR-ERRO##` do script ou o código do app; `None` é a falha não prevista.
    pub code: Option<String>,
    /// A última frase em português: a `##HANGAR-FALHA##`, o erro do app ou "interrompida".
    pub text: String,
    /// A etapa da lateral onde parou: o painel aparece nela.
    pub screen: Screen,
    /// Os botões da frase, escolhidos pelo assistente em `fail` (`codes::buttons`).
    pub fixes: Vec<Fix>,
    /// O passo a passo aberto por "Ver como liberar"/"Ver como iniciar".
    pub help: Option<String>,
}

impl Failure {
    pub(crate) fn from_progress(progress: &Progress, screen: Screen) -> Self {
        Self { code: progress.error.clone(), text: progress.failure.clone().unwrap_or_else(|| tr("setup_failure_title")), screen,
            fixes: Vec::new(), help: None }
    }

    pub(crate) fn app(code: Option<&str>, text: String, screen: Screen) -> Self {
        Self { code: code.map(str::to_owned), text, screen, fixes: Vec::new(), help: None }
    }
}

/// O que o painel pede ao assistente (`SetupWizard::failure_action`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FailureAction { Retry, ToggleDetails, Fix(Fix), ToggleSend, SendAgain(u64), Dismiss(u64), AskAgent(Agent), StopAgent }

pub(crate) type OnFailureAction = Rc<dyn Fn(FailureAction, &mut Window, &mut App)>;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SendState { Sending, Sent, Failed(String) }

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AgentPhase { Starting, Running, Restoring, Rechecking, Done { fixed: bool }, Failed(String) }

pub(crate) struct AgentPanel<'a> {
    pub agent: Agent,
    pub phase: &'a AgentPhase,
    pub explanation: Option<&'a str>,
    /// Depois de desfazer: o teto de tempo, o commit que ele fez e o que não voltou ao original.
    pub notes: Vec<String>,
    /// "Parar" sem identidade do processo: ele segue rodando.
    pub not_stopped: bool,
}

/// O que o painel do relatório mostra; montado pelo assistente a cada desenho.
pub(crate) struct PanelView<'a> {
    /// `None` enquanto monta (o doctor leva alguns segundos).
    pub report: Option<&'a str>,
    pub send: bool,
    /// Já foi entregue à fila de envio (ou a pessoa saiu do painel): a caixa não muda mais.
    pub locked: bool,
    /// "Atualizar e tentar de novo" rodando o `apt-get update`.
    pub refreshing: bool,
    /// Instalados e logados: sem login o botão não aparece.
    pub agents: &'a [Agent],
    pub agent: Option<AgentPanel<'a>>,
    /// Reconferência falhou com mudança na pasta e o envio saiu: a correção foi junto.
    pub fix_sent: bool,
}

/// Relatórios entregues para envio. Ficam fora da falha porque a pessoa sai do painel ao clicar (o `retry` apaga a
/// falha): o resultado continua na tela, em qualquer etapa, até sair ou a pessoa fechar o aviso. Uma falha nova
/// entra na fila e nunca apaga um envio pendente ou que falhou; só os já enviados saem quando chega outro.
#[derive(Default)]
pub(crate) struct Outbox { next: u64, pub items: Vec<(u64, SendState, Payload)> }

impl Outbox {
    /// Devolve o id e o que mandar agora.
    pub(crate) fn push(&mut self, payload: Payload) -> (u64, Payload) {
        self.items.retain(|(_, state, _)| *state != SendState::Sent);
        self.next += 1;
        self.items.push((self.next, SendState::Sending, payload.clone()));
        (self.next, payload)
    }

    /// A resposta de um envio; sem causa (canal fechado) vira a frase de "sem resposta".
    pub(crate) fn finish(&mut self, id: u64, result: Result<(), String>) {
        let Some(item) = self.items.iter_mut().find(|(n, ..)| *n == id) else { return };
        item.1 = match result {
            Ok(()) => SendState::Sent,
            Err(why) if why.trim().is_empty() => SendState::Failed(tr("setup_report_no_answer")),
            Err(why) => SendState::Failed(why),
        };
    }

    /// "Enviar de novo": só de um envio que falhou.
    pub(crate) fn again(&mut self, id: u64) -> Option<Payload> {
        let item = self.items.iter_mut().find(|(n, state, _)| *n == id && matches!(state, SendState::Failed(_)))?;
        item.1 = SendState::Sending;
        Some(item.2.clone())
    }

    /// Fechar o aviso; um envio em andamento continua na tela até responder.
    pub(crate) fn dismiss(&mut self, id: u64) { self.items.retain(|(n, state, _)| *n != id || *state == SendState::Sending); }

    /// O que ainda não saiu, para a última tentativa ao fechar o assistente.
    pub(crate) fn failed(&self) -> Vec<Payload> {
        self.items.iter().filter(|(_, state, _)| matches!(state, SendState::Failed(_))).map(|(.., p)| p.clone()).collect()
    }
}

fn meta(text: String) -> Div { div().text_xs().font_family(theme::MONO).text_color(theme::muted()).child(text) }

fn fix_button(id: String, fix: Fix, primary: bool, on_action: OnFailureAction) -> Button {
    let button = Button::new(SharedString::from(id)).small().label(fix.label())
        .on_click(move |_, window, cx| on_action(FailureAction::Fix(fix), window, cx));
    if primary { button.primary() } else { button.outline() }
}

pub(crate) fn failure_panel(failure: &Failure, details_open: bool, lines: &[String], on_action: OnFailureAction) -> AnyElement {
    let sentence = failure.code.as_deref().and_then(codes::sentence);
    let code_line = match (&failure.code, &sentence) {
        (Some(code), Some(_)) => tr("setup_failure_code").replace("{code}", code),
        (Some(code), None) => format!("{} · {}", tr("setup_failure_unexpected"), tr("setup_failure_code").replace("{code}", code)),
        (None, _) => tr("setup_failure_unexpected"),
    };
    // Com frase da tabela, a mensagem do script fica como apoio; sem ela, é a própria frase.
    let support = sentence.as_ref().map(|_| failure.text.clone()).filter(|t| !t.is_empty() && *t != tr("setup_failure_title"));
    let buttons: Vec<Button> = failure.fixes.iter().enumerate()
        .map(|(n, fix)| fix_button(format!("setup-fix-{}", fix.id()), *fix, n == 0, on_action.clone())).collect();
    let toggle = on_action;
    div().id("setup-failure").role(Role::Alert).flex().flex_col().gap_3().p_4().rounded_lg().border_1()
        .border_color(theme::danger().opacity(0.4)).bg(theme::danger().opacity(0.06))
        .child(div().flex().items_center().gap_2().child(chrome::small_icon(IconName::CircleAlert, 16., theme::danger()))
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(tr("setup_failure_title"))))
        .child(div().text_base().whitespace_normal().child(sentence.unwrap_or_else(|| failure.text.clone())))
        .when_some(support, |el, text| el.child(div().text_sm().text_color(theme::muted()).whitespace_normal().child(text)))
        .child(meta(code_line))
        .when_some(failure.help.clone(), |el, help| el.child(div().id("setup-failure-help").p_3().rounded_md().bg(theme::inset())
            .text_sm().whitespace_normal().child(help)))
        .when(!buttons.is_empty(), |el| el.child(div().flex().flex_wrap().gap_2().children(buttons)))
        .child(details("setup-failure-details", details_open, lines, move |_, window, cx| toggle(FailureAction::ToggleDetails, window, cx)))
        .into_any_element()
}

/// Logo abaixo do painel: o relatório inteiro, a caixa de envio e o estado do envio.
pub(crate) fn after_panel(view: &PanelView, on_action: OnFailureAction) -> Div {
    div().flex().flex_col().gap_4().children(agent_block(view, on_action.clone())).child(report_block(view, on_action))
}

/// Sem agente chamado: os botões "Pedir ajuda ao X"; chamado: em que pé está, "Parar" e, no fim, a explicação dele.
pub(crate) fn agent_block(view: &PanelView, on_action: OnFailureAction) -> Option<AnyElement> {
    let Some(panel) = &view.agent else {
        if view.agents.is_empty() { return None; }
        let ready = view.report.is_some();
        let buttons: Vec<Button> = view.agents.iter().enumerate().map(|(n, agent)| {
            let (agent, on_action) = (*agent, on_action.clone());
            let button = Button::new(SharedString::from(format!("setup-agent-ask-{}", agent.id()))).small().disabled(!ready)
                .label(tr("setup_agent_ask").replace("{agente}", agent.name()))
                .on_click(move |_, window, cx| on_action(FailureAction::AskAgent(agent), window, cx));
            if n == 0 { button.primary() } else { button.outline() }
        }).collect();
        return Some(div().id("setup-agent-offer").flex().flex_col().gap_2()
            .child(div().flex().flex_wrap().gap_2().children(buttons))
            .child(div().text_xs().text_color(theme::muted()).whitespace_normal().child(tr("setup_agent_hint")))
            .into_any_element());
    };
    let name = panel.agent.name();
    let (busy, line) = match panel.phase {
        AgentPhase::Starting => (true, tr("setup_agent_starting").replace("{agente}", name)),
        AgentPhase::Running => (true, tr("setup_agent_running").replace("{agente}", name)),
        AgentPhase::Restoring => (true, tr("setup_agent_restoring")),
        AgentPhase::Rechecking => (true, tr("setup_agent_rechecking")),
        AgentPhase::Done { fixed: true } => (false, tr("setup_agent_fixed").replace("{agente}", name)),
        AgentPhase::Done { fixed: false } => (false, tr("setup_agent_not_fixed").replace("{agente}", name)),
        AgentPhase::Failed(why) => (false, tr("setup_agent_failed").replace("{agente}", name).replace("{erro}", why)),
    };
    let explanation = panel.explanation.filter(|_| matches!(panel.phase, AgentPhase::Done { .. })).map(str::to_owned);
    let stop = (*panel.phase == AgentPhase::Running).then(|| Button::new("setup-agent-stop").outline().small().label(tr("setup_agent_stop"))
        .on_click(move |_, window, cx| on_action(FailureAction::StopAgent, window, cx)));
    // As notas só valem depois de desfazer a pasta (na reconferência e no fim).
    let notes = if matches!(panel.phase, AgentPhase::Starting | AgentPhase::Running | AgentPhase::Restoring) { &[][..] } else { &panel.notes[..] };
    Some(div().id("setup-agent").flex().flex_col().gap_2().p_4().rounded_lg().border_1().border_color(theme::border()).bg(theme::boxed())
        .child(div().id("setup-agent-status").role(Role::Status).flex().items_center().gap_2().text_sm().font_weight(FontWeight::MEDIUM)
            .when(busy, |el| el.child(chrome::Spinner::new(SharedString::from("setup-agent-spin"), IconName::LoaderCircle, px(14.), theme::muted())))
            .child(line))
        .children(stop.map(|button| div().child(button)))
        .when(panel.not_stopped, |el| el.child(div().id("setup-agent-not-stopped").role(Role::Alert).text_sm()
            .text_color(theme::warning_text()).whitespace_normal().child(tr("setup_agent_not_stopped"))))
        .when_some(explanation, |el, text| el.child(TextView::markdown("setup-agent-explanation", text).selectable(true).scrollable(false)))
        .children(notes.iter().map(|note| div().text_sm().text_color(theme::warning_text()).whitespace_normal().child(note.clone())))
        .when(view.fix_sent, |el| el.child(div().text_sm().text_color(theme::warning_text()).child(tr("setup_agent_fix_sent"))))
        .into_any_element())
}

fn report_block(view: &PanelView, on_action: OnFailureAction) -> Div {
    let toggle = on_action;
    // O estado do envio mora em `outbox_lines`, visível em qualquer etapa; aqui só o aviso de quando sai.
    // Com o agente chamado, o relatório espera ele terminar e a caixa não muda até lá.
    let waiting = view.agent.as_ref().filter(|p| !matches!(p.phase, AgentPhase::Done { .. } | AgentPhase::Failed(_)));
    let status = view.send.then(|| match waiting {
        Some(panel) => Some(tr("setup_report_after_agent").replace("{agente}", panel.agent.name())),
        None => (!view.locked).then(|| tr("setup_report_on_leave")),
    }).flatten();
    div().id("setup-report").flex().flex_col().gap_2()
        .when(view.refreshing, |el| el.child(div().id("setup-packages-refreshing").role(Role::Status).flex().items_center().gap_2()
            .text_sm().text_color(theme::muted())
            .child(chrome::Spinner::new(SharedString::from("setup-packages-refreshing-spin"), IconName::LoaderCircle, px(14.), theme::muted()))
            .child(tr("setup_packages_refreshing"))))
        .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(tr("setup_report_title")))
        .child(Checkbox::new("setup-report-send").label(tr("setup_report_send")).checked(view.send).disabled(view.locked || waiting.is_some())
            .on_click(move |_, window, cx| toggle(FailureAction::ToggleSend, window, cx)))
        .when_some(status, |el, status| el.child(div().id("setup-report-status").role(Role::Status).text_xs()
            .text_color(theme::muted()).whitespace_normal().child(status)))
        .child(div().text_xs().text_color(theme::muted()).whitespace_normal().child(tr("setup_report_hint")))
        .child(match view.report {
            None => div().id("setup-report-building").role(Role::Status).text_sm().text_color(theme::muted()).child(tr("setup_report_building")),
            Some(text) => div().id("setup-report-preview").max_h_64().overflow_y_scroll().rounded_md().border_1().border_color(theme::border())
                .bg(theme::inset()).p_2()
                .child(TextView::markdown("setup-report-text", crate::conversation::fenced(text)).selectable(true).scrollable(false)),
        })
}

/// Uma linha por relatório entregue, em qualquer etapa: enviando, enviado ou o erro com "Enviar de novo".
pub(crate) fn outbox_lines(outbox: &Outbox, on_action: OnFailureAction) -> Option<Div> {
    if outbox.items.is_empty() { return None; }
    let title = tr("setup_report_title");
    Some(div().flex().flex_col().gap_2().children(outbox.items.iter().map(|(id, state, _)| {
        let id = *id;
        let (text, failed) = match state {
            SendState::Sending => (tr("setup_report_sending"), false),
            SendState::Sent => (tr("setup_report_sent"), false),
            SendState::Failed(why) => (tr("setup_report_send_failed").replace("{erro}", why), true),
        };
        let (again, dismiss) = (on_action.clone(), on_action.clone());
        div().id(SharedString::from(format!("setup-outbox-{id}"))).role(if failed { Role::Alert } else { Role::Status })
            .flex().items_center().gap_2().text_xs().text_color(if failed { theme::warning_text() } else { theme::muted() })
            .child(div().flex_1().min_w_0().whitespace_normal().child(format!("{title}: {text}")))
            .when(failed, |el| el.child(Button::new(SharedString::from(format!("setup-outbox-again-{id}"))).outline().xsmall()
                .label(tr("setup_report_send_again")).on_click(move |_, window, cx| again(FailureAction::SendAgain(id), window, cx))))
            .when(*state != SendState::Sending, |el| el.child(Button::new(SharedString::from(format!("setup-outbox-dismiss-{id}"))).ghost()
                .xsmall().label(tr("close")).on_click(move |_, window, cx| dismiss(FailureAction::Dismiss(id), window, cx))))
    })))
}

/// O texto de uma pendência: a frase da tabela, senão o texto do script, senão "falha não prevista"; o código cru nunca
/// aparece (fica no id). Com frase, o texto do script vai para a descrição junto com o passo a passo.
fn pending_text(code: &str, text: &str, folder: &str) -> (String, Option<String>) {
    let sentence = codes::sentence(code);
    let script = Some(text.trim().to_owned()).filter(|t| !t.is_empty());
    let detail: Vec<String> = [sentence.as_ref().and(script.clone()), codes::help(code).map(|h| h.replace("{pasta}", folder))]
        .into_iter().flatten().collect();
    let name = sentence.or(script).unwrap_or_else(|| tr("setup_failure_unexpected"));
    (name, (!detail.is_empty()).then(|| detail.join("\n")))
}

/// Uma `##HANGAR-PENDENCIA## <código> <texto>` da tela final, em amarelo, com os botões da tabela.
pub(crate) fn pending_row(code: &str, text: &str, on_action: OnFailureAction) -> Stateful<Div> {
    // ponytail: a assinatura do plano 2 não leva a pasta; só `sem-systemd` usa `{pasta}`, e ele é falha, nunca pendência.
    let folder = super::local::default_dir().map(|d| d.display().to_string()).unwrap_or_default();
    let (name, detail) = pending_text(code, text, &folder);
    let buttons: Vec<Button> = codes::pending_buttons(code).into_iter().enumerate()
        .map(|(n, fix)| fix_button(format!("setup-pending-{code}-{}", fix.id()), fix, n == 0, on_action.clone())).collect();
    super::screens::row(&format!("pending-{code}"), super::screens::RowMark::Warn, name, detail,
        Some(div().flex_shrink_0().flex().gap_2().children(buttons).into_any_element()))
}

/// "Ver detalhes" recolhido: as últimas linhas cruas do script (e, com o agente, os comandos dele).
pub(crate) fn details(id: &'static str, open: bool, lines: &[String], on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Div {
    let shown = &lines[lines.len().saturating_sub(200)..];
    div().flex().flex_col().gap_2()
        .child(div().child(Button::new(id).ghost().small().icon(if open { IconName::ChevronDown } else { IconName::ChevronRight })
            .label(if open { tr("setup_details_hide") } else { tr("setup_details_show") }).toggled(open).on_click(on_toggle)))
        .when(open, |el| el.child(div().id(SharedString::from(format!("{id}-log"))).max_h_40().overflow_y_scroll().p_3().rounded_md()
            .bg(theme::inset()).border_1().border_color(theme::border()).font_family(theme::MONO).text_xs().text_color(theme::muted())
            .map(|el| if shown.is_empty() { el.child(tr("setup_details_empty")) }
                else { el.children(shown.iter().map(|line| div().whitespace_normal().child(line.clone()))) })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::report::{self, Outcome};

    fn payload(code: &str) -> Payload { report::payload(Screen::Prepare, Some(code.into()), Outcome::Aberto, None, "r".into()) }
    fn states(outbox: &Outbox) -> Vec<SendState> { outbox.items.iter().map(|(_, s, _)| s.clone()).collect() }

    #[test]
    fn send_goes_sending_then_sent_or_failed() {
        let mut outbox = Outbox::default();
        let (a, _) = outbox.push(payload("sem-internet"));
        assert_eq!(states(&outbox), vec![SendState::Sending]);
        outbox.finish(a, Err("dns".into()));
        assert_eq!(states(&outbox), vec![SendState::Failed("dns".into())]);
        // Só o que falhou volta a enviar, e uma vez por clique.
        assert_eq!(outbox.again(a).and_then(|p| p.code).as_deref(), Some("sem-internet"));
        assert!(outbox.again(a).is_none());
        outbox.finish(a, Ok(()));
        assert_eq!(states(&outbox), vec![SendState::Sent]);
    }

    #[test]
    fn a_new_failure_never_hides_an_unsent_report() {
        let mut outbox = Outbox::default();
        let (a, _) = outbox.push(payload("sem-internet"));
        outbox.finish(a, Err(String::new()));
        let (b, _) = outbox.push(payload("pacotes-desatualizados"));
        assert_eq!(states(&outbox), vec![SendState::Failed(tr("setup_report_no_answer")), SendState::Sending]);
        assert_eq!(outbox.failed().len(), 1);
        // Enviado sai quando chega o próximo; em andamento não se fecha.
        outbox.finish(b, Ok(()));
        outbox.dismiss(a);
        let (c, _) = outbox.push(payload("sem-agente"));
        assert_eq!(outbox.items.iter().map(|(n, ..)| *n).collect::<Vec<_>>(), vec![c]);
        outbox.dismiss(c);
        assert_eq!(states(&outbox), vec![SendState::Sending]);
    }

    #[test]
    fn pending_never_shows_the_raw_code() {
        // Com frase: o texto do script desce para a descrição.
        let (name, detail) = pending_text("tailscale-https", "HTTPS desligado na tailnet", "/x");
        assert_eq!(name, codes::sentence("tailscale-https").unwrap());
        assert_eq!(detail.as_deref(), Some("HTTPS desligado na tailnet"));
        // Sem frase: o texto do script é o nome.
        assert_eq!(pending_text("outro", "algo faltou", "/x"), ("algo faltou".to_owned(), None));
        // Nem frase nem texto: "falha não prevista", nunca "disco-cheio".
        let (name, _) = pending_text("disco-cheio", " ", "/x");
        assert_eq!(name, tr("setup_failure_unexpected"));
        assert_ne!(name, "disco-cheio");
    }

    #[test]
    fn pending_help_gets_the_folder() {
        let (_, detail) = pending_text("sem-systemd", "", "/srv/hangar");
        let detail = detail.unwrap();
        assert!(detail.contains("/srv/hangar") && !detail.contains("{pasta}"), "{detail}");
    }
}
