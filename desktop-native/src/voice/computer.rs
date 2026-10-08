//! Ferramenta `computer` da voz: o MCP do hangar-computer-control (HCC) em stdio, só para programas fora do Hangar.
use super::rpc::{Rpc, RpcError};
use serde_json::{Value, json};
use std::{ffi::OsString, path::{Path, PathBuf}, time::Duration};

/// O `objetivo` do HCC desiste sozinho em 240 s; a folga cobre a subida do Python e do agente.
const OBJECTIVE_DEADLINE: Duration = Duration::from_secs(300);
pub const JEV_KEY: &str = "TYPESAFE_API_KEY";
/// Só o fallback com imagem usa; sem elas o laço segue pela árvore de acessibilidade.
const OPTIONAL_KEYS: [&str; 4] = ["LLM_PROXY_KEY", "LLM_PROXY_URL", "LLM_MODEL", "LLM_EFFORT"];

pub struct Launch { pub python: PathBuf, pub script: PathBuf, pub env: Vec<(&'static str, OsString)> }

fn home() -> PathBuf { std::env::home_dir().unwrap_or_default() }

pub fn hcc_dir() -> PathBuf {
    std::env::var_os("HANGAR_HCC_DIR").filter(|v| !v.is_empty()).map(PathBuf::from)
        .unwrap_or_else(|| home().join("Projetos").join("hangar-computer-control"))
}

/// O agente desta máquina, nunca o Windows remoto do MCP das sessões: no Linux o Hyprland do checkout; no Windows o
/// primeiro alvo `local` que não seja o do Linux.
pub fn pick_agent_config(configs: &[(String, Value)], windows: bool) -> Option<&str> {
    configs.iter().find(|(name, config)| if windows { name != "linux-agent.json" && config["transport"] == "local" } else { name == "linux-agent.json" })
        .map(|(name, _)| name.as_str())
}

fn read_json(path: &Path) -> Value { std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default() }

/// As chaves que o Hangar já guarda para o HCC: a entrada do MCP (ativa no `~/.claude.json`, senão a guardada ao
/// desligar), o Jev do `settings.json` como o backend faz, e por último o ambiente do app.
fn stored_keys() -> Vec<(&'static str, String)> {
    let home = home();
    let active = &read_json(&home.join(".claude.json"))["mcpServers"]["hangar-computer-control"]["env"];
    let entry = if active.is_object() { active.clone() } else { read_json(&home.join(".hangar").join("computer-control.json"))["env"].clone() };
    let settings = read_json(&home.join(".claude").join("settings.json"))["env"].clone();
    std::iter::once(JEV_KEY).chain(OPTIONAL_KEYS).filter_map(|key| {
        let stored = entry[key].as_str().or_else(|| (key == JEV_KEY).then(|| settings[key].as_str()).flatten()).map(str::to_owned);
        stored.filter(|v| !v.is_empty()).or_else(|| std::env::var(key).ok().filter(|v| !v.is_empty())).map(|v| (key, v))
    }).collect()
}

/// Ambiente do filho; sem a chave do Jev o laço não decide nada, então é erro dito pelo nome.
pub fn child_env(keys: Vec<(&'static str, String)>, dir: &Path, agent: &Path) -> Result<Vec<(&'static str, OsString)>, String> {
    if !keys.iter().any(|(k, _)| *k == JEV_KEY) {
        return Err(format!("Falta a chave {JEV_KEY} (Jev): configure em Configurações > Controle do Windows ou no ambiente do app."));
    }
    let mut env: Vec<(&'static str, OsString)> = keys.into_iter().map(|(k, v)| (k, OsString::from(v))).collect();
    env.extend([("PYTHONPATH", dir.as_os_str().to_owned()), ("VIRTUAL_ENV", OsString::new()), ("HCC_AGENT_CONFIG", agent.as_os_str().to_owned())]);
    Ok(env)
}

/// Bloqueia (lê arquivos): quem chama faz fora da thread da tela.
pub fn launch() -> Result<Launch, String> {
    let dir = hcc_dir();
    let python = dir.join(".venv").join(if cfg!(windows) { "Scripts/python.exe" } else { "bin/python" });
    let script = dir.join("servidor_mcp.py");
    if !python.is_file() || !script.is_file() {
        return Err(format!("O hangar-computer-control não está instalado em {} (falta servidor_mcp.py ou o .venv). Defina HANGAR_HCC_DIR.", dir.display()));
    }
    let mut configs: Vec<(String, Value)> = std::fs::read_dir(&dir).map_err(|e| format!("Não consegui ler {}: {e}", dir.display()))?
        .filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with("-agent.json"))
        .map(|name| { let config = read_json(&dir.join(&name)); (name, config) }).collect();
    configs.sort_by(|a, b| a.0.cmp(&b.0));
    let agent = pick_agent_config(&configs, cfg!(windows)).map(|name| dir.join(name))
        .ok_or_else(|| format!("Sem configuração de agente local em {} ({}).", dir.display(), if cfg!(windows) { "um *-agent.json com transport local" } else { "linux-agent.json" }))?;
    let env = child_env(stored_keys(), &dir, &agent)?;
    Ok(Launch { python, script, env })
}

pub fn initialize_params() -> Value {
    json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "hangar-native-voice", "version": "1"}})
}

pub fn objective_params(objective: &str) -> Value { json!({"name": "objetivo", "arguments": {"texto": objective}}) }

/// Texto do `tools/call`; `isError` vira `Err` com o texto do HCC.
pub fn call_text(result: &Value) -> Result<String, String> {
    let text: String = result["content"].as_array().map(|parts| parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default();
    if result["isError"] == true { Err(if text.is_empty() { "O hangar-computer-control falhou sem dizer o motivo.".into() } else { text }) } else { Ok(text) }
}

fn rpc_text(step: &str, error: RpcError) -> String {
    match error {
        RpcError::Spawn => "Não consegui iniciar o hangar-computer-control.".into(),
        RpcError::Closed => format!("O hangar-computer-control fechou durante {step}."),
        RpcError::Timeout => format!("O hangar-computer-control não respondeu a tempo ({step})."),
        RpcError::Server(message) => format!("O hangar-computer-control recusou {step}: {message}"),
    }
}

/// O receptor volta junto: largado, o leitor do Rpc para na primeira notificação e as respostas nunca chegam.
async fn session(launch: &Launch) -> Result<(Rpc, async_channel::Receiver<super::rpc::Incoming>), String> {
    let (rpc, incoming) = Rpc::spawn_program(&launch.python, &[launch.script.as_os_str()], &launch.env).await.map_err(|e| rpc_text("a subida", e))?;
    rpc.request("initialize", initialize_params()).await.map_err(|e| rpc_text("initialize", e))?;
    rpc.notify("notifications/initialized", json!({})).await.map_err(|e| rpc_text("initialize", e))?;
    Ok((rpc, incoming))
}

/// Um processo por objetivo: largar o future (parar a chamada) derruba o Python junto (`kill_on_drop`).
pub async fn run_objective(launch: Launch, objective: &str) -> Result<String, String> {
    let (rpc, _incoming) = session(&launch).await?;
    let result = rpc.request_within("tools/call", objective_params(objective), OBJECTIVE_DEADLINE).await.map_err(|e| rpc_text("o objetivo", e))?;
    call_text(&result)
}

pub async fn list_tools(launch: Launch) -> Result<Vec<String>, String> {
    let (rpc, _incoming) = session(&launch).await?;
    let result = rpc.request("tools/list", json!({})).await.map_err(|e| rpc_text("tools/list", e))?;
    Ok(result["tools"].as_array().map(|t| t.iter().filter_map(|t| t["name"].as_str().map(str::to_owned)).collect()).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn mcp_requests_have_the_hcc_shape() {
        let init = initialize_params();
        assert_eq!(init["protocolVersion"], json!("2025-06-18"));
        assert!(init["capabilities"].is_object() && init["clientInfo"]["name"].is_string());
        assert_eq!(objective_params("abrir o Bloco de Notas"), json!({"name": "objetivo", "arguments": {"texto": "abrir o Bloco de Notas"}}));
    }

    #[test]
    fn call_result_text_and_error() {
        assert_eq!(call_text(&json!({"content": [{"type": "text", "text": "concluído: aberto"}]})), Ok("concluído: aberto".into()));
        assert_eq!(call_text(&json!({"content": [{"type": "text", "text": "alvo desconhecido"}], "isError": true})), Err("alvo desconhecido".into()));
    }

    #[test]
    fn missing_jev_key_is_named_and_env_points_to_the_local_agent() {
        let (dir, agent) = (Path::new("/p/hcc"), Path::new("/p/hcc/linux-agent.json"));
        let missing = child_env(vec![("LLM_PROXY_KEY", "x".into())], dir, agent).unwrap_err();
        assert!(missing.starts_with("Falta a chave TYPESAFE_API_KEY (Jev)"), "{missing}");
        let env = child_env(vec![(JEV_KEY, "k".into())], dir, agent).unwrap();
        assert!(env.contains(&("HCC_AGENT_CONFIG", OsString::from("/p/hcc/linux-agent.json"))));
        assert!(env.contains(&("PYTHONPATH", OsString::from("/p/hcc"))) && env.contains(&("VIRTUAL_ENV", OsString::new())));
    }

    #[test]
    fn agent_config_is_this_machine() {
        let configs = vec![("delphi-02-agent.json".to_owned(), json!({"transport": "ssh"})), ("linux-agent.json".to_owned(), json!({"transport": "local"})),
            ("pc-agent.json".to_owned(), json!({"transport": "local"}))];
        assert_eq!(pick_agent_config(&configs, false), Some("linux-agent.json"));
        assert_eq!(pick_agent_config(&configs, true), Some("pc-agent.json"));
        assert_eq!(pick_agent_config(&configs[..1], true), None, "alvo remoto nunca é o desta máquina");
    }

    /// Só leitura: `initialize` + `tools/list` no HCC instalado; nunca roda `objetivo`.
    #[tokio::test]
    #[ignore]
    async fn hcc_dry_run_lists_tools() {
        let mut launch = launch().unwrap_or_else(|e| panic!("{e}"));
        launch.env.retain(|(k, _)| !k.starts_with("LLM_") && *k != JEV_KEY);
        let tools = list_tools(launch).await.unwrap();
        assert!(tools.iter().any(|t| t == "objetivo"), "{tools:?}");
        println!("hcc tools: {tools:?}");
    }
}
