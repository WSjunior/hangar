//! Linha da lista de sessões (`SessionInfo`, backend/app/models.py:73).
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Contexto da sessão Claude lido do transcript, em tokens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextUse {
    pub used: u64,
    pub window: u64,
}

/// Campos na ordem de models.py, para o JSON sair igual ao `model_dump_json`. `state` e `provider`
/// ficam em texto como no `StateEvent`: valor novo do Python passa adiante igual. Só `name` é
/// obrigatório; o resto tem o padrão do pydantic.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionRow {
    pub name: String,
    #[serde(default)]
    pub lifecycle_id: Option<String>,
    #[serde(default)]
    pub transfer_id: Option<String>,
    #[serde(default)]
    pub transfer_phase: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub jsonl: Option<String>,
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default)]
    pub headless: bool,
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub engine_account: Option<String>,
    #[serde(default)]
    pub codex_home: Option<String>,
    #[serde(default)]
    pub conta: Option<String>,
    #[serde(default = "default_state")]
    pub state: String,
    #[serde(default)]
    pub last_activity: Option<f64>,
    #[serde(default)]
    pub last_reply: Option<String>,
    #[serde(default)]
    pub last_reply_at: Option<f64>,
    #[serde(default = "default_tracked")]
    pub tracked: bool,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub worktree: bool,
    #[serde(default)]
    pub worktree_path: Option<String>,
    #[serde(default)]
    pub worktree_gone: bool,
    #[serde(default)]
    pub git_cwd: Option<String>,
    #[serde(default)]
    pub git_dirty: Option<u32>,
    #[serde(default)]
    pub git_ahead: Option<u32>,
    #[serde(default)]
    pub git_behind: Option<u32>,
    #[serde(default)]
    pub git_added: Option<u32>,
    #[serde(default)]
    pub git_removed: Option<u32>,
    #[serde(default)]
    pub avisos: Vec<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub startup_steps: Vec<String>,
    #[serde(default)]
    pub question: Option<String>,
    #[serde(default)]
    pub pending_questions: u32,
    #[serde(default)]
    pub options: Option<Vec<String>>,
    #[serde(default)]
    pub stalled: bool,
    #[serde(default)]
    pub problema: Option<String>,
    #[serde(default)]
    pub limited: bool,
    #[serde(default)]
    pub limit_reset: Option<String>,
    #[serde(default)]
    pub shared: bool,
    #[serde(default)]
    pub guest_kind: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub then_target: Option<String>,
    #[serde(default)]
    pub status_line: Option<String>,
    #[serde(default)]
    pub context: Option<ContextUse>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub pair_peers: Option<Vec<String>>,
    #[serde(default)]
    pub pair_external: Option<Map<String, Value>>,
    #[serde(default)]
    pub pair_gid: Option<String>,
    #[serde(default)]
    pub pair_task: Option<String>,
    #[serde(default)]
    pub orq_arbiter: Option<String>,
    #[serde(default)]
    pub loop_status: Option<String>,
    #[serde(default)]
    pub loop_iter: Option<u32>,
    #[serde(default)]
    pub loop_max: Option<u32>,
    #[serde(default)]
    pub plan_name: Option<String>,
    #[serde(default)]
    pub plan_task: Option<u32>,
    #[serde(default)]
    pub plan_task_total: Option<u32>,
    #[serde(default)]
    pub plan_done: Option<u32>,
    #[serde(default)]
    pub plan_total: Option<u32>,
    #[serde(default)]
    pub plan_complete: Option<bool>,
    /// `(feitos, total)` por Task do plano: sai como lista de pares, igual à tupla do pydantic.
    #[serde(default)]
    pub plan_tasks: Option<Vec<(u32, u32)>>,
    #[serde(default)]
    pub plan_hidden: Option<bool>,
}

fn default_provider() -> String {
    "claude".to_owned()
}

fn default_state() -> String {
    "idle".to_owned()
}

fn default_tracked() -> bool {
    true
}
