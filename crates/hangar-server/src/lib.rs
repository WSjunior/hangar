//! Servidor do Hangar: lê as conversas do Claude e do Codex e repassa o resto ao backend Python.
pub mod auth;
pub mod config;
pub mod proxy;
pub mod routes;
pub mod side;
pub mod tail;
pub mod transcript;

/// Versão do contrato com o Python (rotas `/internal`, eventos do side-events, ambiente). O
/// Python (`RUST_SERVER_PROTOCOL`) recusa um binário de outra versão e atende sozinho.
pub const INTERNAL_PROTOCOL: u32 = 1;

/// Lê o cano até o fim ou erro. O Python segura a outra ponta; fechou = pai morreu.
pub async fn parent_gone<R: tokio::io::AsyncRead + Unpin>(mut pipe: R) {
    use tokio::io::AsyncReadExt;
    let mut buf = [0u8; 64];
    while let Ok(n) = pipe.read(&mut buf).await {
        if n == 0 {
            return;
        }
    }
}

/// `routes::serve` até `stop` terminar. Sem esperar as conexões abertas: SSE nunca fecha sozinho.
/// `Ok(())` só quando parou por `stop`.
pub async fn serve_until(
    listener: tokio::net::TcpListener,
    cfg: config::Config,
    stop: impl std::future::Future<Output = ()>,
) -> std::io::Result<()> {
    tokio::select! {
        r = routes::serve(listener, cfg) => r,
        () = stop => Ok(()),
    }
}

/// Log em arquivo (HANGAR_SERVER_LOG) ou no stderr. Nunca recebe texto de conversa.
pub fn init_log(path: Option<&std::path::Path>) {
    let file = path.and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok());
    let builder = tracing_subscriber::fmt().with_target(false);
    let _ = match file {
        Some(f) => builder.with_writer(std::sync::Mutex::new(f)).try_init(),
        None => builder.with_writer(std::io::stderr).try_init(),
    };
}
