//! Comandos que a sessão deixou rodando (`procinfo.shells_de`): os filhos diretos do processo do
//! agente que são shell. Servidor MCP, hook e statusline também nascem filhos diretos, e só o
//! comando do agente passa por um shell.
use std::path::Path;

use hangar_api::state::ShellVivo;

use crate::list::procs::{ProcessView, CHILDREN_TTL};

const SHELLS: [&str; 6] = ["sh", "bash", "zsh", "dash", "ksh", "fish"];
/// O Claude Code roda o Bash num shell com prólogo e o comando de verdade dentro de `eval '...'`.
const EVAL_START: &str = "&& eval '";
const ESCAPED_QUOTE: [&str; 2] = ["'\"'\"'", "'\\''"];

/// `_comando_pedido`: o texto do `eval` lido como string de shell, ou a linha inteira sem ele.
fn requested(raw: &str) -> String {
    let Some(i) = raw.find(EVAL_START) else { return raw.trim().to_owned() };
    let mut j = i + EVAL_START.len();
    let mut out = String::new();
    loop {
        let Some(k) = raw[j..].find('\'').map(|k| k + j) else {
            out.push_str(&raw[j..]);
            break;
        };
        out.push_str(&raw[j..k]);
        let Some(escaped) = ESCAPED_QUOTE.iter().find(|e| raw[k..].starts_with(*e)) else { break };
        out.push('\'');
        j = k + escaped.len();
    }
    out.trim().to_owned()
}

pub fn shells_of(pid: i64, procs: &dyn ProcessView) -> Vec<ShellVivo> {
    let children = match procs.children(CHILDREN_TTL) {
        Ok(c) => c,
        Err(e) => {
            // Vazio aqui diria "nenhum comando rodando" justo quando a TUI diz que há.
            if crate::warn_limit::allow(None, "state_shells_unreadable") {
                tracing::warn!(code = "state_shells_unreadable", kind = ?e.kind(), "estado: filhos do agente ilegíveis");
            }
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for &child in children.get(&pid).into_iter().flatten() {
        let argv = procs.argv(child);
        let shell = argv.first().and_then(|a| Path::new(a).file_name()).and_then(|n| n.to_str())
            .is_some_and(|n| SHELLS.contains(&n));
        if !shell {
            continue;
        }
        let cmd = requested(&argv.join(" "));
        if !cmd.is_empty() {
            out.push(ShellVivo { pid: child, cmd, desde: procs.start_time(child) });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::list::procs::ChildrenMap;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::Arc;

    struct Fake { children: ChildrenMap, argv: HashMap<i64, Vec<String>>, start: HashMap<i64, Option<f64>> }

    impl ProcessView for Fake {
        fn children(&self, _: std::time::Duration) -> std::io::Result<Arc<ChildrenMap>> { Ok(Arc::new(self.children.clone())) }
        fn argv(&self, pid: i64) -> Vec<String> { self.argv.get(&pid).cloned().unwrap_or_default() }
        fn cwd(&self, _: i64) -> Option<std::path::PathBuf> { None }
        fn env_var(&self, _: i64, _: &str) -> std::io::Result<Option<std::ffi::OsString>> { Ok(None) }
        fn start_time(&self, pid: i64) -> Option<f64> { self.start.get(&pid).copied().flatten() }
    }

    fn by_pid<T: serde::de::DeserializeOwned>(v: &Value) -> HashMap<i64, T> {
        v.as_object().unwrap().iter().map(|(k, v)| (k.parse().unwrap(), serde_json::from_value(v.clone()).unwrap())).collect()
    }

    #[test]
    fn shells_match_python() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../backend/tests/fixtures/contract/golden/shells.json");
        let rows: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        for row in rows.as_array().unwrap() {
            let fake = Fake { children: by_pid(&row["children"]), argv: by_pid(&row["argv"]), start: by_pid(&row["start"]) };
            let expected: Vec<ShellVivo> = serde_json::from_value(row["expected"].clone()).unwrap();
            assert_eq!(shells_of(row["pid"].as_i64().unwrap(), &fake), expected, "{}", row["name"]);
        }
    }
}
