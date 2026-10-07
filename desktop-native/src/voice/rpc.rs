//! Cliente do `codex app-server` em stdio: JSON-RPC 2.0, um objeto por linha.
use serde_json::{Value, json};
use std::{collections::HashMap, path::{Path, PathBuf}, process::Stdio, sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU64, Ordering}}, time::Duration};
use tokio::{io::{AsyncBufReadExt, AsyncWriteExt, BufReader}, process::{Child, ChildStdin, Command}, sync::{Mutex as AsyncMutex, oneshot}};

#[derive(Debug)]
pub enum RpcError { Spawn, Closed, Timeout, Server(String) }

pub enum Incoming {
    Notification { method: String, params: Value },
    Request { id: Value, method: String, params: Value },
    Exited,
}

enum Routed { Reply(u64, Result<Value, RpcError>), Incoming(Incoming) }

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>>>;

pub struct Rpc { stdin: AsyncMutex<ChildStdin>, next: AtomicU64, pending: Pending, closed: Arc<AtomicBool>, _child: Child }

fn route(line: &str) -> Option<Routed> {
    let value: Value = serde_json::from_str(line).ok()?;
    match (value.get("method").and_then(Value::as_str), value.get("id")) {
        (Some(method), Some(id)) => Some(Routed::Incoming(Incoming::Request {
            id: id.clone(), method: method.to_owned(), params: value["params"].clone() })),
        (Some(method), None) => Some(Routed::Incoming(Incoming::Notification {
            method: method.to_owned(), params: value["params"].clone() })),
        (None, Some(id)) => {
            let id = id.as_u64()?;
            let result = match value.get("error") {
                Some(error) => Err(RpcError::Server(error["message"].as_str().unwrap_or("erro do app-server").to_owned())),
                None => Ok(value["result"].clone()),
            };
            Some(Routed::Reply(id, result))
        }
        (None, None) => None,
    }
}

fn find_in(dirs: &[PathBuf]) -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) { &["codex.exe", "codex.cmd"] } else { &["codex"] };
    dirs.iter().flat_map(|dir| names.iter().map(move |name| dir.join(name))).find(|path| path.is_file())
}

pub fn find_codex() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|path| std::env::split_paths(&path).collect()).unwrap_or_default();
    // App aberto pelo .desktop não herda o PATH do shell; ~/.local/bin é onde o instalador do Codex põe o atalho.
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        dirs.push(PathBuf::from(home).join(".local").join("bin"));
    }
    find_in(&dirs)
}

impl Rpc {
    pub async fn spawn(codex: &Path) -> Result<(Rpc, async_channel::Receiver<Incoming>), RpcError> {
        Self::spawn_program(codex.as_os_str(), &["app-server"]).await
    }

    async fn spawn_program(program: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> Result<(Rpc, async_channel::Receiver<Incoming>), RpcError> {
        let mut command = Command::new(program);
        command.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: sem console piscando ao ligar a voz
        let mut child = command.spawn().map_err(|_| RpcError::Spawn)?;
        let stdin = child.stdin.take().ok_or(RpcError::Spawn)?;
        let stdout = child.stdout.take().ok_or(RpcError::Spawn)?;
        let pending: Pending = Default::default();
        let closed = Arc::new(AtomicBool::new(false));
        let (tx, rx) = async_channel::unbounded();
        let (readers, reader_closed) = (pending.clone(), closed.clone());
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                match route(&line) {
                    Some(Routed::Reply(id, result)) => {
                        if let Some(waiter) = readers.lock().unwrap().remove(&id) { let _ = waiter.send(result); }
                    }
                    Some(Routed::Incoming(incoming)) => { if tx.send(incoming).await.is_err() { break; } }
                    None => {}
                }
            }
            // A bandeira sobe antes do esvaziamento: `request` confere depois de se registrar, então nenhuma fica sem resposta.
            reader_closed.store(true, Ordering::SeqCst);
            for (_, waiter) in readers.lock().unwrap().drain() { let _ = waiter.send(Err(RpcError::Closed)); }
            let _ = tx.send(Incoming::Exited).await;
        });
        Ok((Rpc { stdin: AsyncMutex::new(stdin), next: AtomicU64::new(0), pending, closed, _child: child }, rx))
    }

    async fn write(&self, value: Value) -> Result<(), RpcError> {
        let mut line = value.to_string();
        line.push('\n');
        self.stdin.lock().await.write_all(line.as_bytes()).await.map_err(|_| RpcError::Closed)
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if self.closed.load(Ordering::SeqCst) {
            self.pending.lock().unwrap().remove(&id);
            return Err(RpcError::Closed);
        }
        if let Err(error) = self.write(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).await {
            self.pending.lock().unwrap().remove(&id);
            return Err(error);
        }
        match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(RpcError::Closed),
            Err(_) => { self.pending.lock().unwrap().remove(&id); Err(RpcError::Timeout) }
        }
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<(), RpcError> {
        self.write(json!({"jsonrpc": "2.0", "method": method, "params": params})).await
    }

    pub async fn respond(&self, id: Value, result: Value) -> Result<(), RpcError> {
        self.write(json!({"jsonrpc": "2.0", "id": id, "result": result})).await
    }
}

pub async fn handshake(rpc: &Rpc) -> Result<Value, RpcError> {
    rpc.request("initialize", json!({"clientInfo": {"name": "hangar_native_voice", "version": "1"},
        "capabilities": {"experimentalApi": true}})).await?;
    rpc.notify("initialized", json!({})).await?;
    Ok(rpc.request("config/read", json!({"includeLayers": false})).await?["config"].clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn reply_notification_and_server_request_are_told_apart() {
        assert!(matches!(route(r#"{"jsonrpc":"2.0","id":3,"result":{}}"#), Some(Routed::Reply(3, _))));
        assert!(matches!(route(r#"{"jsonrpc":"2.0","method":"thread/realtime/sdp","params":{"sdp":"v=0"}}"#),
            Some(Routed::Incoming(Incoming::Notification { .. }))));
        // Pedido do servidor: tem `method` E `id`; o id do servidor é independente dos nossos.
        assert!(matches!(route(r#"{"jsonrpc":"2.0","id":3,"method":"item/tool/call","params":{}}"#),
            Some(Routed::Incoming(Incoming::Request { .. }))));
        assert!(route("warning: not json").is_none());
    }

    #[test]
    fn error_reply_carries_message() {
        let Some(Routed::Reply(1, Err(RpcError::Server(message)))) =
            route(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-1,"message":"conversation is not running"}}"#) else { panic!() };
        assert_eq!(message, "conversation is not running");
    }

    #[test]
    fn finds_codex_in_path_dir() {
        let dir = std::env::temp_dir().join(format!("voice-find-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let name = if cfg!(windows) { "codex.exe" } else { "codex" };
        std::fs::write(dir.join(name), b"").unwrap();
        assert_eq!(find_in(&[dir.clone()]), Some(dir.join(name)));
        assert_eq!(find_in(&[dir.join("nada")]), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn exited_child_fails_pending_requests() {
        // Um "codex" que sai na hora: a request pendente falha com Closed em vez de esperar o prazo.
        let (program, args): (&str, &[&str]) = if cfg!(windows) { ("cmd", &["/c", "exit"]) } else { ("true", &[]) };
        let (rpc, incoming) = Rpc::spawn_program(program, args).await.unwrap();
        let result = rpc.request("initialize", serde_json::json!({})).await;
        assert!(matches!(result, Err(RpcError::Closed)));
        assert!(matches!(incoming.recv().await, Ok(Incoming::Exited)));
    }
}
