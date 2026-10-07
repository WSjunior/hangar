//! Uma janela só: a segunda execução (o link `hangar://` aberto pelo sistema) entrega o link à primeira e sai.
use std::{io::{BufRead, BufReader, Read, Write}, net::{Shutdown, TcpListener, TcpStream}, path::{Path, PathBuf}, time::Duration};

pub enum Claim { Forwarded, Primary(async_channel::Sender<String>, async_channel::Receiver<String>) }

pub fn invite_arg(args: impl Iterator<Item = String>) -> Option<String> { args.skip(1).find(|a| a.starts_with("hangar://")) }

fn port_file() -> Option<PathBuf> { Some(crate::appearance::dir()?.join("instance")) }

/// Pedido de senha de um `--askpass` (a `app_sudo` do script do assistente). `prompt` é o motivo; `retry`, que o `sudo -S`
/// recusou a senha anterior. `reply` leva a senha ou `None` (recusa).
pub struct AskpassRequest { pub code: String, pub prompt: String, pub retry: bool, pub reply: std::sync::mpsc::Sender<Option<String>> }

/// Quem atende os pedidos de senha: o assistente aberto. Sem ninguém, o pedido é recusado na hora.
static ASKPASS: std::sync::Mutex<Option<async_channel::Sender<AskpassRequest>>> = std::sync::Mutex::new(None);
const ASKPASS_TAG: &str = "askpass\t";
/// Quanto o `sudo` espera a pessoa digitar.
const ASKPASS_WAIT: Duration = Duration::from_secs(600);

pub fn serve_askpass(tx: Option<async_channel::Sender<AskpassRequest>>) { *ASKPASS.lock().unwrap_or_else(|e| e.into_inner()) = tx; }

/// Uma linha só (`askpass\t<código>\t<0|1>\t<motivo>`): quebra no motivo viraria a linha seguinte do protocolo.
pub fn askpass_line(code: &str, prompt: &str, retry: bool) -> String {
    format!("{ASKPASS_TAG}{code}\t{}\t{}", u8::from(retry), prompt.replace(['\n', '\r', '\t'], " "))
}

pub fn parse_askpass(line: &str) -> Option<(String, String, bool)> {
    let mut parts = line.strip_prefix(ASKPASS_TAG)?.splitn(3, '\t');
    let code = parts.next()?.to_owned();
    let retry = parts.next() == Some("1");
    Some((code, parts.next().unwrap_or_default().to_owned(), retry))
}

fn answer_askpass(code: String, prompt: String, retry: bool) -> Option<String> {
    let tx = ASKPASS.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
    let (reply, wait) = std::sync::mpsc::channel();
    tx.send_blocking(AskpassRequest { code, prompt, retry, reply }).ok()?;
    wait.recv_timeout(ASKPASS_WAIT).ok().flatten()
}

/// O `--askpass`: pede a senha à janela aberta e a imprime para o `sudo -S` do script. 0 = senha impressa.
pub fn askpass_client(prompt: &str, retry: bool) -> i32 {
    let code = std::env::var(crate::app::ASKPASS_CODE_ENV).unwrap_or_default();
    let Some(file) = port_file() else { return 1 };
    match askpass_client_at(&file, &code, prompt, retry) {
        Some(password) => { println!("{password}"); 0 }
        None => 1,
    }
}

pub fn askpass_client_at(file: &Path, code: &str, prompt: &str, retry: bool) -> Option<String> {
    if code.is_empty() { return None; }
    let text = std::fs::read_to_string(file).ok()?;
    let (port, nonce) = text.split_once('\n')?;
    let port = port.trim().parse::<u16>().ok()?;
    let mut stream = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(500)).ok()?;
    stream.set_read_timeout(Some(ASKPASS_WAIT + Duration::from_secs(5))).ok()?;
    write!(stream, "{}\n{}\n", nonce.trim(), askpass_line(code, prompt, retry)).ok()?;
    stream.shutdown(Shutdown::Write).ok()?;
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer).ok()?;
    answer.trim_end_matches(['\r', '\n']).strip_prefix("pw\t").map(str::to_owned)
}

pub fn claim(link: Option<String>) -> Claim {
    match port_file() {
        // Aberto pela autoatualização: o antigo ainda está no ar esperando esta prova de vida e fecha em seguida.
        Some(file) if crate::update::relaunched() => take_over(&file, link),
        Some(file) => claim_at(&file, link),
        // Sem pasta de configuração não há como achar a outra instância: abre sozinha, como antes.
        None => { let (tx, rx) = async_channel::unbounded(); if let Some(link) = link { let _ = tx.try_send(link); } Claim::Primary(tx, rx) }
    }
}

/// O endereço desta janela, para a autoatualização devolver se a versão nova não subir.
pub fn snapshot() -> Option<(PathBuf, String)> {
    let file = port_file()?;
    let text = std::fs::read_to_string(&file).ok()?;
    Some((file, text))
}

pub fn restore((file, text): (PathBuf, String)) { let _ = std::fs::write(file, text); }

fn forward(file: &Path, link: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(file) else { return false };
    let Some((port, nonce)) = text.split_once('\n') else { return false };
    let Ok(port) = port.trim().parse::<u16>() else { return false };
    let Ok(mut stream) = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(500)) else { return false };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    if write!(stream, "{}\n{link}\n", nonce.trim()).is_err() { return false; }
    let _ = stream.shutdown(Shutdown::Write);
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer).is_ok() && answer.trim() == "ok"
}

pub fn claim_at(file: &Path, link: Option<String>) -> Claim {
    // Sem link a segunda execução também só avisa: a primeira vem para a frente.
    if forward(file, link.as_deref().unwrap_or("")) { return Claim::Forwarded; }
    take_over(file, link)
}

fn take_over(file: &Path, link: Option<String>) -> Claim {
    let (tx, rx) = async_channel::unbounded();
    if let Some(link) = link { let _ = tx.try_send(link); }
    let Ok(listener) = TcpListener::bind(("127.0.0.1", 0)) else { return Claim::Primary(tx, rx) };
    let thread_tx = tx.clone();
    let nonce = format!("{:x}{:x}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    if let Some(dir) = file.parent() { let _ = std::fs::create_dir_all(dir); }
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
    if let Ok(mut f) = options.open(file) { let _ = write!(f, "{port}\n{nonce}"); }
    std::thread::Builder::new().name("single-instance".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            // Uma conexão por thread: quem conecta e não fala (ou pinga devagar) não segura o próximo link.
            let (nonce, tx) = (nonce.clone(), thread_tx.clone());
            let _ = std::thread::Builder::new().name("single-instance-conn".into()).spawn(move || {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut reader = BufReader::new((&stream).take(8192));
                let (mut got, mut link) = (String::new(), String::new());
                if reader.read_line(&mut got).is_err() || got.trim() != nonce { return; }
                let _ = reader.read_line(&mut link);
                // Pedido de senha espera a resposta da janela; link de convite responde na hora.
                if let Some((code, prompt, retry)) = parse_askpass(link.trim_end_matches(['\r', '\n'])) {
                    let reply = match answer_askpass(code, prompt, retry) { Some(password) => format!("pw\t{password}\n"), None => "no\n".to_owned() };
                    let _ = (&stream).write_all(reply.as_bytes());
                    return;
                }
                let _ = (&stream).write_all(b"ok\n");
                let _ = tx.send_blocking(link.trim().to_owned());
            });
        }
    }).ok();
    Claim::Primary(tx, rx)
}

#[cfg(test)]
mod tests {
    use super::{AskpassRequest, Claim, askpass_client_at, askpass_line, claim_at, invite_arg, parse_askpass, serve_askpass, take_over};
    use core::prelude::v1::test;

    #[test]
    fn only_the_first_hangar_link_counts() {
        let args = ["hangar-native", "--x", "hangar://convite/h:8443/AB", "hangar://convite/h:8443/CD"].map(String::from);
        assert_eq!(invite_arg(args.into_iter()).as_deref(), Some("hangar://convite/h:8443/AB"));
        assert_eq!(invite_arg(["hangar-native".to_owned()].into_iter()), None);
    }

    #[test]
    fn second_launch_hands_the_link_to_the_first_and_exits() {
        let dir = std::env::temp_dir().join(format!("hangar-si-{}", std::process::id()));
        let file = dir.join("instance");
        let Claim::Primary(_, rx) = claim_at(&file, None) else { panic!("first launch must be primary") };
        assert!(matches!(claim_at(&file, Some("hangar://convite/h:8443/AB".into())), Claim::Forwarded));
        assert_eq!(rx.recv_blocking().unwrap(), "hangar://convite/h:8443/AB");
        // Arquivo velho de outra execução (nonce que ninguém escuta): a nova vira a primária.
        std::fs::write(&file, "1\nnonce-velho").unwrap();
        assert!(matches!(claim_at(&file, None), Claim::Primary(..)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn relaunch_by_the_updater_takes_over_while_the_old_one_is_still_up() {
        let dir = std::env::temp_dir().join(format!("hangar-si-up-{}", std::process::id()));
        let file = dir.join("instance");
        let Claim::Primary(..) = claim_at(&file, None) else { panic!("first launch must be primary") };
        let Claim::Primary(_, rx) = take_over(&file, None) else { panic!("relaunch must not forward") };
        assert!(matches!(claim_at(&file, Some("hangar://convite/h:8443/AB".into())), Claim::Forwarded));
        assert_eq!(rx.recv_blocking().unwrap(), "hangar://convite/h:8443/AB");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn askpass_line_round_trips_and_never_carries_line_breaks() {
        let line = askpass_line("c0de", "instalar\to tmux\n", false);
        assert!(!line.contains('\n'));
        assert_eq!(parse_askpass(&line), Some(("c0de".to_owned(), "instalar o tmux ".to_owned(), false)));
        // `--retry`: o sudo recusou a senha guardada.
        assert_eq!(parse_askpass(&askpass_line("c0de", "instalar o tmux", true)),
            Some(("c0de".to_owned(), "instalar o tmux".to_owned(), true)));
        assert_eq!(parse_askpass("hangar://convite/h:8443/AB"), None);
    }

    // Um teste só: `serve_askpass` é global, e dois testes em paralelo trocariam o atendente um do outro.
    #[test]
    fn askpass_request_crosses_the_instance_channel() {
        let dir = std::env::temp_dir().join(format!("hangar-si-askpass-{}", std::process::id()));
        let file = dir.join("instance");
        let Claim::Primary(..) = claim_at(&file, None) else { panic!("first launch must be primary") };
        // Ninguém atendendo: recusa na hora.
        serve_askpass(None);
        assert_eq!(askpass_client_at(&file, "c0de", "instalar o tmux", false), None);
        let (tx, rx) = async_channel::unbounded::<AskpassRequest>();
        serve_askpass(Some(tx));
        let answering = std::thread::spawn(move || {
            let request = rx.recv_blocking().unwrap();
            assert_eq!((request.code.as_str(), request.prompt.as_str(), request.retry), ("c0de", "instalar o tmux", true));
            request.reply.send(Some("s3 nha\tx".into())).unwrap();
            let refused = rx.recv_blocking().unwrap();
            refused.reply.send(None).unwrap();
        });
        assert_eq!(askpass_client_at(&file, "c0de", "instalar o tmux", true).as_deref(), Some("s3 nha\tx"));
        assert_eq!(askpass_client_at(&file, "outro", "instalar o tmux", false), None);
        answering.join().unwrap();
        serve_askpass(None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
