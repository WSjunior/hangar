use super::protocol::*;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}
type Socket = Box<dyn Stream>;

pub enum IoEvent {
    Line(Value),
    WriteAck { operation_id: String, outcome: WriteOutcome },
    Stderr(String),
    End { code: Option<i64> },
}

pub struct WireFrame {
    pub operation_id: String,
    pub frame: Value,
}

pub struct CanoConnection {
    pub snapshot: CanoSnapshot,
    stream: BufReader<Socket>,
}

pub struct IoTasks {
    pub writer: mpsc::Sender<WireFrame>,
    pub events: mpsc::Receiver<IoEvent>,
    stop: watch::Sender<bool>,
    reader_task: JoinHandle<()>,
    writer_task: JoinHandle<()>,
}

impl IoTasks {
    pub fn hold_lease(mut self, lease: std::sync::Arc<std::fs::File>) -> Self {
        let reader = self.reader_task;
        let writer = self.writer_task;
        let reader_lease = lease.clone();
        self.reader_task = tokio::spawn(async move { let _lease = reader_lease; let _ = reader.await; });
        self.writer_task = tokio::spawn(async move { let _lease = lease; let _ = writer.await; });
        self
    }

    pub async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = tokio::join!(self.reader_task, self.writer_task);
    }
}

pub async fn read_bounded<R: AsyncBufRead + Unpin>(reader: &mut R, limit: usize) -> Result<Option<Value>, RuntimeError> {
    let mut frame = Vec::new();
    read_into(reader, &mut frame, limit).await
}

async fn read_into<R: AsyncBufRead + Unpin>(reader: &mut R, frame: &mut Vec<u8>, limit: usize) -> Result<Option<Value>, RuntimeError> {
    loop {
        let bytes = reader.fill_buf().await.map_err(|_| RuntimeError::new("cano_read", "conexão do cano encerrada"))?;
        if bytes.is_empty() {
            return if frame.is_empty() { Ok(None) } else { Err(RuntimeError::new("partial_frame", "linha incompleta no cano")) };
        }
        let end = bytes.iter().position(|b| *b == b'\n');
        let take = end.map_or(bytes.len(), |n| n + 1);
        if frame.len() + take > limit + 1 {
            return Err(RuntimeError::new("frame_limit", "linha do cano acima do teto"));
        }
        frame.extend_from_slice(&bytes[..take]);
        reader.consume(take);
        if end.is_some() {
            let raw = std::mem::take(frame);
            let value: Value = serde_json::from_slice(&raw)
                .map_err(|_| RuntimeError::new("frame_json", "linha do cano inválida"))?;
            if !value.is_object() { return Err(RuntimeError::new("frame_shape", "linha do cano inválida")); }
            return Ok(Some(value));
        }
    }
}

async fn open(binding: &CanoBinding) -> Result<BufReader<Socket>, RuntimeError> {
    if binding.versao != 2 || binding.token.is_empty() || binding.token.contains(['\r', '\n']) {
        return Err(RuntimeError::new("cano_version", "cano sem suporte ao runtime nativo"));
    }
    let deadline = Instant::now() + LISTEN_WAIT;
    let connect = async {
        let socket: Socket = loop {
            let attempt: std::io::Result<Socket> = if let Some(address) = binding.escuta.strip_prefix("tcp:") {
                let address: std::net::SocketAddr = address.parse()
                    .map_err(|_| RuntimeError::new("cano_address", "endereço do cano inválido"))?;
                if !address.ip().is_loopback() { return Err(RuntimeError::new("cano_address", "cano fora do loopback")); }
                tokio::net::TcpStream::connect(address).await.map(|mut tcp| { crate::nodelay(&mut tcp); Box::new(tcp) as Socket })
            } else if let Some(path) = binding.escuta.strip_prefix("unix:") {
                #[cfg(unix)]
                { tokio::net::UnixStream::connect(path).await.map(|unix| Box::new(unix) as Socket) }
                #[cfg(not(unix))]
                { let _ = path; return Err(RuntimeError::new("cano_address", "socket Unix indisponível")); }
            } else { return Err(RuntimeError::new("cano_address", "endereço do cano inválido")); };
            match attempt {
                Ok(socket) => break socket,
                // O cano recém-lançado ainda não escuta (escopo do systemd + exec levam milissegundos):
                // só "recusada" e "socket ausente" contam como subindo; outro erro responde na hora.
                Err(error) if matches!(error.kind(), std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound)
                    && Instant::now() < deadline => tokio::time::sleep(Duration::from_millis(50)).await,
                Err(_) => return Err(RuntimeError::new("cano_connect", "não foi possível conectar ao cano")),
            }
        };
        let mut reader = BufReader::new(socket);
        let header = format!("{}\n", binding.token);
        reader.get_mut().write_all(header.as_bytes()).await
            .map_err(|_| RuntimeError::new("cano_auth", "não foi possível autenticar no cano"))?;
        reader.get_mut().flush().await
            .map_err(|_| RuntimeError::new("cano_auth", "não foi possível autenticar no cano"))?;
        Ok(reader)
    };
    tokio::time::timeout(LISTEN_WAIT + Duration::from_secs(5), connect).await
        .map_err(|_| RuntimeError::new("cano_timeout", "cano não respondeu"))?
}

/// Quanto o `open` espera o cano lançado começar a escutar, como o Python antigo antes de matá-lo.
const LISTEN_WAIT: Duration = Duration::from_secs(10);

async fn snapshot(reader: &mut BufReader<Socket>) -> Result<CanoSnapshot, RuntimeError> {
    let value = tokio::time::timeout(Duration::from_secs(10), read_bounded(reader, MAX_FRAME)).await
        .map_err(|_| RuntimeError::new("cano_timeout", "snapshot não respondeu"))??
        .ok_or_else(|| RuntimeError::new("cano_eof", "cano encerrou antes do snapshot"))?;
    CanoSnapshot::parse(value)
}

// Sem comparar pid: o sidecar guarda o do cano e o snapshot traz o do agente filho. Quem prova que é
// o cano certo é o token único por subida, já conferido em `open`.
pub async fn connect(binding: &CanoBinding) -> Result<CanoConnection, RuntimeError> {
    let mut stream = open(binding).await?;
    let snapshot = snapshot(&mut stream).await?;
    Ok(CanoConnection { snapshot, stream })
}

impl CanoConnection {
    pub fn start(self, _generation: u64, capacity: usize) -> IoTasks {
        let (read, mut write) = tokio::io::split(self.stream);
        let (writer, mut commands) = mpsc::channel::<WireFrame>(capacity.max(1));
        let (events_tx, events) = mpsc::channel(capacity.max(1));
        let (stop, stop_rx) = watch::channel(false);
        let pending = Arc::new(Mutex::new(HashMap::<String, Instant>::new()));
        let reader_task = {
            let pending = pending.clone();
            let events = events_tx.clone();
            let mut stop = stop_rx.clone();
            let mut stopping = stop_rx.clone();
            tokio::spawn(async move {
                let work = async {
                    let mut read = BufReader::new(read);
                    let mut frame = Vec::new();
                    let mut timed_out = HashSet::new();
                    loop {
                        let deadline = pending.lock().unwrap().values().copied().min()
                            .unwrap_or_else(|| Instant::now() + Duration::from_secs(30));
                        let value = tokio::select! {
                            result = read_into(&mut read, &mut frame, MAX_ENVELOPE) => match result {
                                Ok(Some(value)) => Some(value),
                                _ => { let _ = events.send(IoEvent::End { code: None }).await; return; }
                            },
                            _ = tokio::time::sleep_until(deadline) => None,
                            _ = stopping.changed() => return,
                        };
                        if let Some(value) = value {
                            let event = match value["type"].as_str() {
                                Some("cano_output") => {
                                    let Some(raw) = value["frame"].as_str().filter(|r| r.len() <= MAX_FRAME) else {
                                        tracing::warn!("cano_output com envelope inválido; leitor encerrado");
                                        let _ = events.send(IoEvent::End { code:None }).await; return;
                                    };
                                    let frame = match serde_json::from_str::<Value>(raw) {
                                        Ok(frame) if frame.is_object()=>frame,
                                        _=>{ tracing::warn!("mensagem da CLI fora do JSON esperado; leitura continua"); continue; },
                                    };
                                    IoEvent::Line(frame)
                                }
                                Some("cano_input_ack") => {
                                    let Some(operation_id) = value["operation_id"].as_str() else {
                                        tracing::warn!("ACK sem identificador; leitor encerrado");
                                        let _ = events.send(IoEvent::End { code:None }).await; return;
                                    };
                                    let Ok(outcome) = serde_json::from_value::<WriteOutcome>(value["outcome"].clone()) else {
                                        tracing::warn!("ACK com resultado inválido; leitor encerrado");
                                        let _ = events.send(IoEvent::End { code:None }).await; return;
                                    };
                                    if pending.lock().unwrap().remove(operation_id).is_none() && !timed_out.remove(operation_id) { continue; }
                                    IoEvent::WriteAck { operation_id: operation_id.to_owned(), outcome }
                                }
                                Some("cano_stderr") => IoEvent::Stderr(value["linha"].as_str().unwrap_or("").to_owned()),
                                Some("cano_saiu") => IoEvent::End { code: value["rc"].as_i64() },
                                _ => {
                                    tracing::warn!("envelope desconhecido do cano; leitor encerrado");
                                    let _ = events.send(IoEvent::End { code:None }).await; return;
                                },
                            };
                            let end = matches!(&event, IoEvent::End { .. });
                            if events.send(event).await.is_err() || end { return; }
                        }
                        let expired = {
                            let mut pending = pending.lock().unwrap();
                            let ids: Vec<_> = pending.iter().filter(|(_, d)| **d <= Instant::now()).map(|(id, _)| id.clone()).collect();
                            for id in &ids { pending.remove(id); }
                            ids
                        };
                        for operation_id in expired {
                            timed_out.insert(operation_id.clone());
                            if events.send(IoEvent::WriteAck { operation_id, outcome: WriteOutcome::Unknown }).await.is_err() { return; }
                        }
                    }
                };
                tokio::select! { _ = work => {}, _ = stop.changed() => {} }
            })
        };
        let writer_task = tokio::spawn(async move {
            let mut stop = stop_rx;
            let work = async {
                let mut used = HashSet::new();
                while let Some(command) = commands.recv().await {
                    if !used.insert(command.operation_id.clone()) { continue; }
                    let frame = command.frame.to_string();
                    if !command.frame.is_object() || frame.len() > MAX_FRAME {
                        let _ = events_tx.send(IoEvent::WriteAck { operation_id: command.operation_id, outcome: WriteOutcome::NotWritten }).await;
                        continue;
                    }
                    let envelope = json!({"type":"cano_input","operation_id":command.operation_id,"frame":frame});
                    let raw = format!("{envelope}\n");
                    if raw.len() > MAX_ENVELOPE + 1 {
                        let _ = events_tx.send(IoEvent::WriteAck { operation_id: command.operation_id, outcome: WriteOutcome::NotWritten }).await;
                        continue;
                    }
                    pending.lock().unwrap().insert(command.operation_id.clone(), Instant::now() + Duration::from_secs(30));
                    if write.write_all(raw.as_bytes()).await.is_err() || write.flush().await.is_err() {
                        pending.lock().unwrap().remove(&command.operation_id);
                        let _ = events_tx.send(IoEvent::WriteAck { operation_id: command.operation_id, outcome: WriteOutcome::Unknown }).await;
                        return;
                    }
                }
            };
            tokio::select! { _ = work => {}, _ = stop.changed() => {} }
            let _ = write.shutdown().await;
        });
        IoTasks { writer, events, stop, reader_task, writer_task }
    }
}
