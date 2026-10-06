//! ConPTY do `portable-pty` no Windows, com o `cmd.exe` do runner no lugar do `tmux attach`.
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use portable_pty::CommandBuilder;
use tokio::sync::mpsc;

use super::Slot;
use super::pty;

const PROMPT: &str = "HGP>";

fn open(cols: u16, rows: u16) -> pty::Opened {
    let mut cmd = CommandBuilder::new("cmd.exe");
    cmd.args(["/d", "/k", "prompt HGP$G"]);
    pty::spawn(cmd, cols, rows, Arc::new(Slot(Arc::default()))).expect("o ConPTY abre")
}

/// Lê até `done` valer ou o prazo acabar. Responde ao pedido de posição do cursor que o
/// `INHERIT_CURSOR` faz, como o xterm.js responde.
async fn read_until(output: &mut mpsc::Receiver<Bytes>, input: Option<&mpsc::Sender<Bytes>>, seen: &mut Vec<u8>,
                    within: Duration, done: impl Fn(&[u8]) -> bool) -> bool {
    let until = tokio::time::Instant::now() + within;
    while !done(seen) {
        match tokio::time::timeout_at(until, output.recv()).await {
            Ok(Some(b)) => {
                if let Some(input) = input.filter(|_| b.windows(4).any(|w| w == b"\x1b[6n")) {
                    let _ = input.send(Bytes::from_static(b"\x1b[1;1R")).await;
                }
                seen.extend_from_slice(&b);
            }
            Ok(None) | Err(_) => break,
        }
    }
    done(seen)
}

fn count(seen: &[u8], needle: &str) -> usize {
    seen.windows(needle.len()).filter(|w| *w == needle.as_bytes()).count()
}

fn text(seen: &[u8]) -> String {
    String::from_utf8_lossy(seen).into_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn spawn_echo_and_resize() {
    let pty::Opened { pty, mut output, input } = open(80, 24);
    let mut seen = Vec::new();
    assert!(read_until(&mut output, Some(&input), &mut seen, Duration::from_secs(15), |s| count(s, PROMPT) >= 1).await,
            "sem prompt: {}", text(&seen));
    // A variável vazia some só na saída: o eco da tecla não casa com o texto esperado.
    input.send(Bytes::from_static(b"echo hangar-%COMSPEC:~0,0%ok\r")).await.unwrap();
    assert!(read_until(&mut output, Some(&input), &mut seen, Duration::from_secs(15), |s| count(s, "hangar-ok") >= 1).await,
            "sem eco: {}", text(&seen));
    pty.resize(100, 30);
    let mark = seen.len();
    input.send(Bytes::from_static(b"mode con\r")).await.unwrap();
    let resized = |s: &[u8]| { let t = String::from_utf8_lossy(&s[mark..]).into_owned(); t.contains("100") && t.contains("30") };
    assert!(read_until(&mut output, Some(&input), &mut seen, Duration::from_secs(15), resized).await,
            "tamanho não chegou ao console: {}", text(&seen[mark..]));
    drop(input);
    pty::close(pty).await.expect("o filho sai e o pseudoconsole fecha");
}

#[tokio::test(flavor = "multi_thread")]
async fn child_killed_before_close() {
    let pty::Opened { pty, mut output, input } = open(80, 24);
    let mut seen = Vec::new();
    assert!(read_until(&mut output, Some(&input), &mut seen, Duration::from_secs(15), |s| count(s, PROMPT) >= 1).await,
            "sem prompt: {}", text(&seen));
    // Fechado antes, o pseudoconsole derrubaria o filho com CTRL_CLOSE (0xC000013A), ou travaria.
    let status = tokio::time::timeout(Duration::from_secs(10), pty::close(pty)).await
        .expect("a desmontagem não trava").expect("o filho sai");
    assert_eq!(status.exit_code(), 1, "o filho saiu por outro motivo que o TerminateProcess");
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_writer_writes_nothing() {
    let pty::Opened { pty, mut output, input } = open(80, 24);
    let mut seen = Vec::new();
    assert!(read_until(&mut output, Some(&input), &mut seen, Duration::from_secs(15), |s| count(s, PROMPT) >= 1).await,
            "sem prompt: {}", text(&seen));
    // Controle: um Enter de verdade desenha outro prompt.
    input.send(Bytes::from_static(b"\r")).await.unwrap();
    assert!(read_until(&mut output, Some(&input), &mut seen, Duration::from_secs(15), |s| count(s, PROMPT) >= 2).await,
            "o Enter não chegou: {}", text(&seen));
    let prompts = count(&seen, PROMPT);
    // A escritora sai e solta o escritor: no Unix o `take_writer` mandaria "\n" + Ctrl-D aqui.
    drop(input);
    read_until(&mut output, None, &mut seen, Duration::from_secs(2), |_| false).await;
    assert_eq!(count(&seen, PROMPT), prompts, "soltar o escritor entregou tecla: {}", text(&seen));
    pty::close(pty).await.expect("o filho sai e o pseudoconsole fecha");
}
