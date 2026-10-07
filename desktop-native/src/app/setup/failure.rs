//! Ponto de extensão da falha. O plano 3 troca o corpo de `failure_panel` pela tabela código → frase → botão, acrescenta o
//! relatório, o agente e o envio, e dá a cada pendência de `pending_row` o botão dela. Aqui: o código, o texto cru,
//! "Ver detalhes" e "Tentar de novo".
use super::*;
use super::flow::Screen;
use super::marks::Progress;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Failure {
    /// O `##HANGAR-ERRO##` do script ou o código do app (`sem-internet`, `senha-cancelada`, `versao-diferente`…); `None` é a
    /// falha não prevista.
    pub code: Option<String>,
    /// A última frase em português: a `##HANGAR-FALHA##`, o erro do app ou "interrompida".
    pub text: String,
    /// A etapa da lateral onde parou: o painel aparece nela.
    pub screen: Screen,
}

impl Failure {
    pub(crate) fn from_progress(progress: &Progress, screen: Screen) -> Self {
        Self { code: progress.error.clone(), text: progress.failure.clone().unwrap_or_else(|| tr("setup_failure_title")), screen }
    }

    pub(crate) fn app(code: Option<&str>, text: String, screen: Screen) -> Self { Self { code: code.map(str::to_owned), text, screen } }
}

/// O que o painel pede ao assistente (`SetupWizard::failure_action`). O plano 3 acrescenta variantes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FailureAction { Retry, ToggleDetails }

pub(crate) type OnFailureAction = Rc<dyn Fn(FailureAction, &mut Window, &mut App)>;

pub(crate) fn failure_panel(failure: &Failure, details_open: bool, lines: &[String], on_action: OnFailureAction) -> AnyElement {
    let (retry, toggle) = (on_action.clone(), on_action);
    div().id("setup-failure").role(Role::Alert).flex().flex_col().gap_3().p_4().rounded_lg().border_1()
        .border_color(theme::danger().opacity(0.4)).bg(theme::danger().opacity(0.06))
        .child(div().flex().items_center().gap_2().child(chrome::small_icon(IconName::CircleAlert, 16., theme::danger()))
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(tr("setup_failure_title"))))
        .child(div().text_sm().whitespace_normal().child(failure.text.clone()))
        .when_some(failure.code.clone(), |el, code| el.child(div().text_xs().font_family(theme::MONO).text_color(theme::muted())
            .child(tr("setup_failure_code").replace("{code}", &code))))
        .child(div().flex().gap_2().child(Button::new("setup-failure-retry").primary().small().label(tr("setup_retry"))
            .on_click(move |_, window, cx| retry(FailureAction::Retry, window, cx))))
        .child(details("setup-failure-details", details_open, lines, move |_, window, cx| toggle(FailureAction::ToggleDetails, window, cx)))
        .into_any_element()
}

/// Uma `##HANGAR-PENDENCIA## <código> <texto>` da tela 6, em amarelo, com o botão que a resolve. O código cru nunca é
/// mostrado: só o texto da marca; ele fica no id do elemento.
pub(crate) fn pending_row(code: &str, text: &str, on_action: OnFailureAction) -> Stateful<Div> {
    let retry = Button::new(SharedString::from(format!("setup-pending-{code}"))).outline().small().label(tr("setup_retry"))
        .on_click(move |_, window, cx| on_action(FailureAction::Retry, window, cx));
    super::screens::row(&format!("pending-{code}"), super::screens::RowMark::Warn, text.to_owned(), None, Some(retry.into_any_element()))
}

/// "Ver detalhes" recolhido: as últimas linhas cruas do script.
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
