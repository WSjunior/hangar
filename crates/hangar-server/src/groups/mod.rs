//! Grupos de sessões (pareamento): arquivos em `.hangar-pair` e as regras sobre eles.
pub mod deliver;
pub mod local;
pub mod model;
pub mod orq;
pub mod routes;
pub mod service;
pub mod store;

use std::path::PathBuf;
use std::sync::Arc;

use crate::list::discover_other::Dirs;
use service::{GroupService, OrqFacts};
use store::PairDir;

/// Serviço do processo sobre o `.hangar-pair` da conta padrão (`settings.projects_dir.parent`, a
/// pasta `claude` da lista). Arquivo de contratos e id da máquina vêm do Python
/// (`HANGAR_PAIR_ARCHIVE`, `HANGAR_SERVER_ID`); sem as pastas da lista não há serviço.
pub fn from_env(dirs: Option<&Dirs>, orq: Arc<dyn OrqFacts>) -> Option<Arc<GroupService>> {
    let dirs = dirs?;
    let var = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
    // Mesmo padrão do `pair._arquivo_dir`: o cofre `~/.hangar`, não a conta.
    let archive = var("HANGAR_PAIR_ARCHIVE").map(PathBuf::from).unwrap_or_else(|| dirs.home.join(".hangar").join("pair-arquivo"));
    let dir = PairDir::new(dirs.claude.join(".hangar-pair"), archive);
    Some(Arc::new(GroupService::new(dir, orq, var("HANGAR_SERVER_ID").unwrap_or_default())))
}
