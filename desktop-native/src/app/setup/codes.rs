//! Tabela de falhas (spec "Tabela de falhas"): o código que o script ou o app dá vira a frase e o botão que resolve.
//! Código fora da tabela é a falha não prevista: o painel mostra a última mensagem do script.
use crate::i18n::tr;

/// "Instalador de Aplicativo" (winget) na Microsoft Store.
pub(crate) const STORE_URL: &str = "ms-windows-store://pdp/?productid=9NBLGGH4NNS1";
pub(crate) const DEVELOPER_SETTINGS_URL: &str = "ms-settings:developers";
pub(crate) const TAILSCALE_LOGIN_URL: &str = "https://login.tailscale.com/";
pub(crate) const DOWNLOAD_URL: &str = "https://hangar.dev.br/#download";
pub(crate) const DOWNLOAD_URL_EN: &str = "https://hangar.dev.br/en/#download";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fix {
    Retry, Recheck, AskAgain, OpenStore, OpenTailscaleSettings, TailscaleLogin, OpenDeveloperSettings, HowToAllow, HowToStart,
    RefreshPackages, InstallLater, UpdateApp, FixMouseWheel,
}

impl Fix {
    pub(crate) fn id(self) -> &'static str {
        match self {
            Fix::Retry => "retry", Fix::Recheck => "recheck", Fix::AskAgain => "ask-again", Fix::OpenStore => "store",
            Fix::OpenTailscaleSettings => "tailscale-settings", Fix::TailscaleLogin => "tailscale-login",
            Fix::OpenDeveloperSettings => "developer-settings", Fix::HowToAllow => "how-allow", Fix::HowToStart => "how-start",
            Fix::RefreshPackages => "refresh-packages", Fix::InstallLater => "install-later", Fix::UpdateApp => "update-app",
            Fix::FixMouseWheel => "fix-mouse-wheel",
        }
    }

    pub(crate) fn label(self) -> String {
        match self {
            Fix::Retry => tr("setup_retry"),
            Fix::Recheck => tr("setup_recheck"),
            Fix::AskAgain => tr("setup_fix_ask_again"),
            Fix::OpenStore => tr("setup_fix_store"),
            Fix::OpenTailscaleSettings => tr("setup_tailscale_open_settings"),
            Fix::TailscaleLogin => tr("setup_tailscale_login"),
            Fix::OpenDeveloperSettings => tr("setup_item_open_settings"),
            Fix::HowToAllow => tr("setup_fix_how_allow"),
            Fix::HowToStart => tr("setup_fix_how_start"),
            Fix::RefreshPackages => tr("setup_fix_refresh"),
            Fix::InstallLater => tr("setup_fix_install_later"),
            Fix::UpdateApp => tr("setup_fix_update_app"),
            Fix::FixMouseWheel => tr("setup_fix_mouse_wheel"),
        }
    }

    /// Roda o script inteiro de novo: com um destes no painel, "Tentar de novo" seria o mesmo botão duas vezes.
    pub(crate) fn reruns(self) -> bool {
        matches!(self, Fix::Retry | Fix::Recheck | Fix::AskAgain | Fix::RefreshPackages | Fix::InstallLater | Fix::FixMouseWheel)
    }

    pub(crate) fn shows_help(self) -> bool { matches!(self, Fix::HowToAllow | Fix::HowToStart) }
}

pub(crate) fn sentence(code: &str) -> Option<String> {
    Some(match code {
        "sem-internet" => tr("setup_code_sem_internet"),
        "sem-winget" => tr("setup_code_sem_winget"),
        "sem-sudo" => tr("setup_code_sem_sudo"),
        "senha-cancelada" => tr("setup_code_senha_cancelada"),
        "politica-travada" => tr("setup_code_politica_travada"),
        "checkout-sujo" => tr("setup_code_checkout_sujo"),
        "sem-systemd" => tr("setup_code_sem_systemd"),
        "pacotes-desatualizados" => tr("setup_code_pacotes_desatualizados"),
        "tailscale-https" => tr("setup_code_tailscale_https"),
        "tailscale-login" => tr("setup_code_tailscale_login"),
        "agente-nao-instalou" => tr("setup_code_agente_nao_instalou"),
        "sem-agente" => tr("setup_code_sem_agente"),
        "versao-diferente" => tr("setup_code_versao_diferente"),
        "modo-desenvolvedor" => tr("setup_code_modo_desenvolvedor"),
        "roda-do-mouse" => tr("setup_code_roda_do_mouse"),
        _ => return None,
    })
}

/// Os botões da coluna "Botão" da spec, na ordem dela.
pub(crate) fn fixes(code: &str) -> &'static [Fix] {
    match code {
        "sem-internet" | "sem-agente" => &[Fix::Retry],
        "sem-winget" => &[Fix::OpenStore],
        "sem-sudo" | "politica-travada" => &[Fix::HowToAllow],
        "senha-cancelada" => &[Fix::AskAgain],
        "sem-systemd" => &[Fix::HowToStart],
        "pacotes-desatualizados" => &[Fix::RefreshPackages],
        "tailscale-https" => &[Fix::OpenTailscaleSettings, Fix::Recheck],
        "tailscale-login" => &[Fix::TailscaleLogin],
        "agente-nao-instalou" => &[Fix::Retry, Fix::InstallLater],
        "versao-diferente" => &[Fix::UpdateApp],
        "modo-desenvolvedor" => &[Fix::OpenDeveloperSettings, Fix::Recheck],
        "roda-do-mouse" => &[Fix::FixMouseWheel],
        _ => &[],
    }
}

/// O passo a passo de "Ver como liberar"/"Ver como iniciar": dentro do app, porque a falha pode ser justamente a internet.
pub(crate) fn help(code: &str) -> Option<String> {
    match code {
        "sem-sudo" => Some(tr("setup_help_sem_sudo")),
        "politica-travada" => Some(tr("setup_help_politica_travada")),
        "sem-systemd" => Some(tr("setup_help_sem_systemd")),
        _ => None,
    }
}

/// Os botões do painel: os da tabela, sem "Instalar depois" quando isso deixaria a instalação sem agente, e "Tentar de
/// novo" no fim quando nenhum deles roda o script de novo (o painel do plano 2 sempre teve como tentar de novo).
pub(crate) fn buttons(code: Option<&str>, can_drop_agent: bool) -> Vec<Fix> {
    let mut list: Vec<Fix> = code.map_or(&[][..], fixes).iter().copied().filter(|f| *f != Fix::InstallLater || can_drop_agent).collect();
    if !list.iter().any(|f| f.reruns()) { list.push(Fix::Retry); }
    list
}

/// Uma pendência da tela final: o passo a passo vai como texto da linha, e "Instalar depois" já é o que a pendência é.
pub(crate) fn pending_buttons(code: &str) -> Vec<Fix> {
    let mut list: Vec<Fix> = fixes(code).iter().copied().filter(|f| !f.shows_help() && *f != Fix::InstallLater).collect();
    if !list.iter().any(|f| f.reruns()) { list.push(Fix::Retry); }
    list.truncate(2);
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tabela da spec e o `modo-desenvolvedor` do plano 1 (o `outro` usa o texto do próprio script).
    const KNOWN: [&str; 15] = ["sem-internet", "sem-winget", "sem-sudo", "senha-cancelada", "politica-travada", "checkout-sujo",
        "sem-systemd", "pacotes-desatualizados", "tailscale-https", "tailscale-login", "agente-nao-instalou", "sem-agente",
        "versao-diferente", "modo-desenvolvedor", "roda-do-mouse"];

    #[test]
    fn every_known_code_has_its_sentence_and_button() {
        for code in KNOWN {
            let text = sentence(code).unwrap_or_else(|| panic!("{code} sem frase"));
            // Chave sem tradução volta crua ("setup_code_…").
            assert!(!text.starts_with("setup_"), "{code}: {text}");
            // `checkout-sujo`: "Ver detalhes" e "Pedir ajuda" já moram no painel; os outros têm botão próprio.
            assert!(code == "checkout-sujo" || !fixes(code).is_empty(), "{code} sem botão");
        }
    }

    #[test]
    fn spec_buttons_per_code() {
        assert_eq!(fixes("sem-internet"), &[Fix::Retry]);
        assert_eq!(fixes("sem-winget"), &[Fix::OpenStore]);
        assert_eq!(fixes("sem-sudo"), &[Fix::HowToAllow]);
        assert_eq!(fixes("senha-cancelada"), &[Fix::AskAgain]);
        assert_eq!(fixes("politica-travada"), &[Fix::HowToAllow]);
        assert_eq!(fixes("sem-systemd"), &[Fix::HowToStart]);
        assert_eq!(fixes("pacotes-desatualizados"), &[Fix::RefreshPackages]);
        assert_eq!(fixes("tailscale-https"), &[Fix::OpenTailscaleSettings, Fix::Recheck]);
        assert_eq!(fixes("tailscale-login"), &[Fix::TailscaleLogin]);
        assert_eq!(fixes("agente-nao-instalou"), &[Fix::Retry, Fix::InstallLater]);
        assert_eq!(fixes("versao-diferente"), &[Fix::UpdateApp]);
        assert_eq!(fixes("roda-do-mouse"), &[Fix::FixMouseWheel]);
    }

    #[test]
    fn unknown_code_is_the_unexpected_failure() {
        assert_eq!(sentence("disco-cheio"), None);
        assert_eq!(buttons(Some("disco-cheio"), true), vec![Fix::Retry]);
        assert_eq!(buttons(None, true), vec![Fix::Retry]);
    }

    #[test]
    fn retry_joins_only_when_nothing_reruns() {
        assert_eq!(buttons(Some("sem-winget"), false), vec![Fix::OpenStore, Fix::Retry]);
        assert_eq!(buttons(Some("tailscale-https"), false), vec![Fix::OpenTailscaleSettings, Fix::Recheck]);
        assert_eq!(buttons(Some("agente-nao-instalou"), true), vec![Fix::Retry, Fix::InstallLater]);
        // Só o Claude Code escolhido: "Instalar depois" deixaria a instalação sem agente nenhum.
        assert_eq!(buttons(Some("agente-nao-instalou"), false), vec![Fix::Retry]);
    }

    #[test]
    fn help_exists_exactly_where_the_button_asks_for_it() {
        for code in KNOWN { assert_eq!(help(code).is_some(), fixes(code).iter().any(|f| f.shows_help()), "{code}"); }
        assert!(help("sem-systemd").is_some_and(|h| h.contains("{pasta}")));
    }

    #[test]
    fn pending_rows_get_at_most_two_buttons_and_no_help_toggle() {
        assert_eq!(pending_buttons("politica-travada"), vec![Fix::Retry]);
        assert_eq!(pending_buttons("agente-nao-instalou"), vec![Fix::Retry]);
        assert_eq!(pending_buttons("tailscale-https"), vec![Fix::OpenTailscaleSettings, Fix::Recheck]);
        assert_eq!(pending_buttons("modo-desenvolvedor"), vec![Fix::OpenDeveloperSettings, Fix::Recheck]);
        assert_eq!(pending_buttons("outro"), vec![Fix::Retry]);
    }
}
