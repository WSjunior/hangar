//! Cliente JSON-RPC do app-server do Codex. O núcleo fala por dois canais de texto (uma mensagem
//! JSON por item); stdio e WebSocket só convertem o transporte nesses canais.
use crate::proto::{ClientRequest,RequestId};
use serde::de::DeserializeOwned;
use serde_json::{Value,json};
use std::collections::HashMap;
use std::sync::{Arc,Mutex,atomic::{AtomicI64,Ordering}};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt,AsyncRead,AsyncReadExt,AsyncWrite,AsyncWriteExt,BufReader};
use tokio::sync::{mpsc,oneshot};

pub const MAX_LINE:usize = 16 * 1024 * 1024;
pub const INCOMING_CAPACITY:usize = 1024;
const OUTGOING_CAPACITY:usize = 256;

#[derive(Debug)]
pub enum ClientError { Timeout, Closed, Rpc { code:i64, message:String }, Decode(String), Io(String) }

#[derive(Debug)]
pub enum Incoming { Notification { method:String, params:Value }, Request { id:RequestId, method:String, params:Value } }

type Pending = Arc<Mutex<Option<HashMap<RequestId,oneshot::Sender<Result<Value,ClientError>>>>>>;

#[derive(Clone)]
pub struct Client { out:mpsc::Sender<String>, pending:Pending, next:Arc<AtomicI64> }

impl Client {
    /// Núcleo: `lines_in` fecha quando a conexão acaba; aí todo pedido em voo falha com `Closed`.
    fn start(mut lines_in:mpsc::Receiver<String>,out:mpsc::Sender<String>) -> (Self,mpsc::Receiver<Incoming>) {
        let pending:Pending = Arc::new(Mutex::new(Some(HashMap::new())));
        let (tx,rx) = mpsc::channel(INCOMING_CAPACITY);
        let reader_pending = pending.clone();
        tokio::spawn(async move {
            while let Some(line) = lines_in.recv().await {
                let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
                let id = msg.get("id").filter(|id|!id.is_null()).and_then(|id|serde_json::from_value::<RequestId>(id.clone()).ok());
                match (id,msg["method"].as_str()) {
                    (Some(id),Some(method)) => {
                        if tx.send(Incoming::Request { id,method:method.into(),params:msg["params"].clone() }).await.is_err() { break; }
                    }
                    (None,Some(method)) => {
                        if tx.send(Incoming::Notification { method:method.into(),params:msg["params"].clone() }).await.is_err() { break; }
                    }
                    (Some(id),None) => {
                        let waiter = reader_pending.lock().unwrap().as_mut().and_then(|map|map.remove(&id));
                        if let Some(waiter) = waiter {
                            let outcome = match msg.get("error").filter(|e|!e.is_null()) {
                                Some(error) => Err(ClientError::Rpc { code:error["code"].as_i64().unwrap_or(0),
                                    message:error["message"].as_str().unwrap_or("").into() }),
                                None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                            };
                            let _ = waiter.send(outcome);
                        }
                    }
                    (None,None) => {},
                }
            }
            // Conexão acabou: quem espera resposta sai agora, não no prazo.
            if let Some(map) = reader_pending.lock().unwrap().take() {
                for (_,waiter) in map { let _ = waiter.send(Err(ClientError::Closed)); }
            }
        });
        (Self { out,pending,next:Arc::new(AtomicI64::new(1)) },rx)
    }

    pub fn over_lines(reader:impl AsyncRead+Unpin+Send+'static,mut writer:impl AsyncWrite+Unpin+Send+'static) -> (Self,mpsc::Receiver<Incoming>) {
        let (lines_tx,lines_rx) = mpsc::channel::<String>(INCOMING_CAPACITY);
        tokio::spawn(async move {
            let mut reader = BufReader::new(reader);
            let mut buffer = Vec::new();
            loop {
                buffer.clear();
                // `take` limita a linha: acima do teto a conexão é encerrada.
                let read = (&mut reader).take(MAX_LINE as u64 + 1).read_until(b'\n',&mut buffer).await;
                match read {
                    Ok(0) | Err(_) => break,
                    Ok(_) if buffer.len() > MAX_LINE => { tracing::warn!("linha do app-server do Codex acima do teto; conexão encerrada"); break; }
                    Ok(_) => {
                        let Ok(text) = std::str::from_utf8(&buffer) else { continue };
                        let text = text.trim_end();
                        if !text.is_empty() && lines_tx.send(text.to_owned()).await.is_err() { break; }
                    }
                }
            }
        });
        let (out_tx,mut out_rx) = mpsc::channel::<String>(OUTGOING_CAPACITY);
        tokio::spawn(async move {
            while let Some(line) = out_rx.recv().await {
                if writer.write_all(line.as_bytes()).await.is_err() || writer.write_all(b"\n").await.is_err() || writer.flush().await.is_err() { break; }
            }
        });
        Self::start(lines_rx,out_tx)
    }

    pub fn spawn_stdio(mut command:tokio::process::Command) -> std::io::Result<(Self,mpsc::Receiver<Incoming>,tokio::process::Child)> {
        command.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).kill_on_drop(true);
        let mut child = command.spawn()?;
        let stdout = child.stdout.take().ok_or_else(||std::io::Error::other("sem stdout"))?;
        let stdin = child.stdin.take().ok_or_else(||std::io::Error::other("sem stdin"))?;
        let (client,incoming) = Self::over_lines(stdout,stdin);
        Ok((client,incoming,child))
    }

    pub async fn connect_ws(url:&str) -> Result<(Self,mpsc::Receiver<Incoming>),ClientError> {
        use futures_util::{SinkExt,StreamExt};
        use tokio_tungstenite::tungstenite::{Message,protocol::WebSocketConfig};
        let config = WebSocketConfig::default().max_message_size(Some(MAX_LINE)).max_frame_size(Some(MAX_LINE));
        let (socket,_) = tokio_tungstenite::connect_async_with_config(url,Some(config),false).await.map_err(|e|ClientError::Io(e.to_string()))?;
        let (mut sink,mut stream) = socket.split();
        let (lines_tx,lines_rx) = mpsc::channel::<String>(INCOMING_CAPACITY);
        tokio::spawn(async move {
            while let Some(Ok(message)) = stream.next().await {
                match message {
                    Message::Text(text) => { if lines_tx.send(text.to_string()).await.is_err() { break; } }
                    Message::Close(_) => break,
                    _ => {},
                }
            }
        });
        let (out_tx,mut out_rx) = mpsc::channel::<String>(OUTGOING_CAPACITY);
        tokio::spawn(async move {
            while let Some(line) = out_rx.recv().await { if sink.send(Message::text(line)).await.is_err() { break; } }
            let _ = sink.close().await;
        });
        Ok(Self::start(lines_rx,out_tx))
    }

    pub async fn request<R:DeserializeOwned>(&self,request:ClientRequest,timeout:Duration) -> Result<R,ClientError> {
        let id = RequestId::Integer(self.next.fetch_add(1,Ordering::Relaxed));
        let (tx,rx) = oneshot::channel();
        match self.pending.lock().unwrap().as_mut() { Some(map) => { map.insert(id.clone(),tx); }, None => return Err(ClientError::Closed) }
        let (method,params) = request.into_parts();
        let line = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}).to_string();
        if self.out.send(line).await.is_err() { self.forget(&id); return Err(ClientError::Closed); }
        let value = match tokio::time::timeout(timeout,rx).await {
            Err(_) => { self.forget(&id); return Err(ClientError::Timeout); }
            Ok(Err(_)) => return Err(ClientError::Closed),
            Ok(Ok(outcome)) => outcome?,
        };
        serde_json::from_value(value).map_err(|e|ClientError::Decode(format!("{method}: {}",crate::proto::error_kind(&e))))
    }

    fn forget(&self,id:&RequestId) { if let Some(map) = self.pending.lock().unwrap().as_mut() { map.remove(id); } }

    pub async fn notify(&self,method:&str,params:Value) -> Result<(),ClientError> {
        self.out.send(json!({"jsonrpc":"2.0","method":method,"params":params}).to_string()).await.map_err(|_|ClientError::Closed)
    }

    pub async fn respond(&self,id:RequestId,outcome:Result<Value,(i64,String)>) -> Result<(),ClientError> {
        let line = match outcome {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err((code,message)) => json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}),
        };
        self.out.send(line.to_string()).await.map_err(|_|ClientError::Closed)
    }
}

