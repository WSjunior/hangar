//! Assistente de instalação (spec `2026-10-06-instalador-grafico-design.md`): a entrada que acha o Hangar deste computador e
//! as seis telas que o instalam pelos scripts de sempre no modo `--app`. Só Linux e Windows; no macOS o app segue no cartão
//! de endereço + token.
use super::*;

mod agent;
mod app_copy;
mod askpass;
mod codes;
mod entry;
mod failure;
mod flow;
mod local;
mod marks;
mod phone;
mod precheck;
mod repo;
mod report;
mod run;
mod screens;
mod system;
mod tail;
mod wizard;

pub(super) use wizard::SetupWizard;

/// O assistente existe onde há script de instalação com modo `--app`.
pub(super) fn supported() -> bool { cfg!(any(target_os = "linux", target_os = "windows")) }

/// De onde o assistente foi aberto: a entrada já sabe a pasta; o menu procura de novo; reabrir retoma o que estava rodando.
pub(super) enum Origin {
    Entry(Option<local::LocalInstall>),
    Menu,
    Resume(run::SetupState),
}

/// O cartão da entrada enquanto não há conexão salva (spec "Entrada").
pub(super) enum Entry {
    Probing,
    Found(local::Found),
}

/// O assistente que estava rodando quando o app fechou (`state.json`): reabrir o app retoma nele.
pub(super) fn saved_run() -> Option<run::SetupState> { if supported() { run::load_state() } else { None } }

/// Só para provar telas: `HANGAR_SETUP_DEMO` abre o assistente ao iniciar.
pub(super) fn demo() -> bool { supported() && std::env::var_os("HANGAR_SETUP_DEMO").is_some() }
