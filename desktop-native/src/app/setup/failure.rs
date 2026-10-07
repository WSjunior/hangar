//! A falha na tela (spec "Falha e relatório"): a frase e o botão do código (`codes.rs`), "Ver detalhes" e, logo abaixo, o
//! relatório com o texto exato que sai e a caixa de envio. Falha sem código, ou com código fora da tabela, é a falha não
//! prevista: a última mensagem do script vira a frase.
use super::*;
use super::codes::{self, Fix};
use super::flow::Screen;
use super::marks::Progress;
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
pub(crate) enum FailureAction { Retry, ToggleDetails, Fix(Fix), ToggleSend, SendAgain }

pub(crate) type OnFailureAction = Rc<dyn Fn(FailureAction, &mut Window, &mut App)>;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SendState { Idle, Sending, Sent, Failed(String) }

/// O que o painel do relatório mostra; montado pelo assistente a cada desenho.
pub(crate) struct PanelView<'a> {
    /// `None` enquanto monta (o doctor leva alguns segundos).
    pub report: Option<&'a str>,
    pub send: bool,
    pub sent: &'a SendState,
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
    div().flex().flex_col().gap_4().child(report_block(view, on_action))
}

fn report_block(view: &PanelView, on_action: OnFailureAction) -> Div {
    let (toggle, again) = (on_action.clone(), on_action);
    let locked = matches!(view.sent, SendState::Sending | SendState::Sent);
    let status = match view.sent {
        SendState::Idle if view.send => Some(tr("setup_report_on_leave")),
        SendState::Idle => None,
        SendState::Sending => Some(tr("setup_report_sending")),
        SendState::Sent => Some(tr("setup_report_sent")),
        SendState::Failed(why) => Some(tr("setup_report_send_failed").replace("{erro}", why)),
    };
    div().id("setup-report").flex().flex_col().gap_2()
        .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(tr("setup_report_title")))
        .child(Checkbox::new("setup-report-send").label(tr("setup_report_send")).checked(view.send).disabled(locked)
            .on_click(move |_, window, cx| toggle(FailureAction::ToggleSend, window, cx)))
        .when_some(status, |el, status| el.child(div().id("setup-report-status").role(Role::Status).text_xs()
            .text_color(theme::muted()).whitespace_normal().child(status)))
        .when(matches!(view.sent, SendState::Failed(_)), |el| el.child(div().child(Button::new("setup-report-again").outline().small()
            .label(tr("setup_report_send_again")).on_click(move |_, window, cx| again(FailureAction::SendAgain, window, cx)))))
        .child(div().text_xs().text_color(theme::muted()).whitespace_normal().child(tr("setup_report_hint")))
        .child(match view.report {
            None => div().id("setup-report-building").role(Role::Status).text_sm().text_color(theme::muted()).child(tr("setup_report_building")),
            Some(text) => div().id("setup-report-preview").max_h_64().overflow_y_scroll().rounded_md().border_1().border_color(theme::border())
                .bg(theme::inset()).p_2()
                .child(TextView::markdown("setup-report-text", crate::conversation::fenced(text)).selectable(true).scrollable(false)),
        })
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
