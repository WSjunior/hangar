//! ConPTY do `portable-pty` no Windows, com o `cmd.exe` do runner no lugar do `tmux attach`.
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use portable_pty::CommandBuilder;
use tokio::sync::mpsc;

use super::Slot;
use super::pty;

const PROMPT: &str = "HGP>";
const DSR: &str = "\x1b[6n";

fn open(cols: u16, rows: u16) -> pty::Opened {
    let mut cmd = CommandBuilder::new("cmd.exe");
    cmd.args(["/d", "/k", "prompt HGP$G"]);
    pty::spawn(cmd, cols, rows, Arc::new(Slot(Arc::default())), false).expect("o ConPTY abre")
}

/// Lê até `done` valer ou o prazo acabar. Responde ao pedido de posição do cursor que o
/// `INHERIT_CURSOR` faz, como o xterm.js responde.
async fn read_until(output: &mut mpsc::Receiver<Bytes>, input: Option<&mpsc::Sender<Bytes>>, seen: &mut Vec<u8>,
                    within: Duration, done: impl Fn(&[u8]) -> bool) -> bool {
    let until = tokio::time::Instant::now() + within;
    // Contado sobre tudo que chegou: o pedido pode vir partido entre dois blocos.
    let mut answered = count(seen, DSR);
    while !done(seen) {
        match tokio::time::timeout_at(until, output.recv()).await {
            Ok(Some(b)) => {
                seen.extend_from_slice(&b);
                let asked = count(seen, DSR);
                if let Some(input) = input {
                    for _ in answered..asked {
                        input.send(Bytes::from_static(b"\x1b[1;1R")).await.expect("a entrada do ConPTY aceita a resposta");
                    }
                }
                answered = asked;
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
    let pty::Opened { pty, mut output, input, .. } = open(80, 24);
    let mut seen = Vec::new();
    assert!(read_until(&mut output, Some(&input), &mut seen, Duration::from_secs(15), |s| count(s, PROMPT) >= 1).await,
            "sem prompt: {}", text(&seen));
    // O `^` some só na saída: o eco da tecla não casa com o texto esperado.
    input.send(Bytes::from_static(b"echo hangar-^ok\r")).await.unwrap();
    assert!(read_until(&mut output, Some(&input), &mut seen, Duration::from_secs(15), |s| count(s, "hangar-ok") >= 1).await,
            "sem eco: {}", text(&seen));
    pty.resize(100, 30);
    let mark = seen.len();
    input.send(Bytes::from_static(b"mode con\r")).await.unwrap();
    // Com espaço antes: nas sequências de cursor do repaint o número vem depois de `[` ou `;`.
    let resized = |s: &[u8]| { let t = String::from_utf8_lossy(&s[mark..]).into_owned(); t.contains(" 100") && t.contains(" 30") };
    assert!(read_until(&mut output, Some(&input), &mut seen, Duration::from_secs(15), resized).await,
            "tamanho não chegou ao console: {}", text(&seen[mark..]));
    drop(input);
    pty::close(pty).await.expect("o filho sai e o pseudoconsole fecha");
}

#[tokio::test(flavor = "multi_thread")]
async fn child_killed_before_close() {
    let pty::Opened { pty, mut output, input, .. } = open(80, 24);
    let mut seen = Vec::new();
    assert!(read_until(&mut output, Some(&input), &mut seen, Duration::from_secs(15), |s| count(s, PROMPT) >= 1).await,
            "sem prompt: {}", text(&seen));
    // Não travar é a prova da ordem; o código 1 diz que quem saiu foi o `TerminateProcess`.
    let status = tokio::time::timeout(Duration::from_secs(10), pty::close(pty)).await
        .expect("a desmontagem não trava").expect("o filho sai");
    assert_eq!(status.exit_code(), 1, "o filho saiu por outro motivo que o TerminateProcess");
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_writer_writes_nothing() {
    let pty::Opened { pty, mut output, input, .. } = open(80, 24);
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

/// O prompt entra na tela alternativa como o `tmux attach` do psmux: é o sinal de pronto. A tecla
/// vai antes de qualquer saída, e a resposta ao pedido de cursor tem que passar pela entrada
/// segurada, senão o conhost não sobe o filho e só o prazo abriria.
#[tokio::test(flavor = "multi_thread")]
async fn held_input_reaches_the_child_after_ready() {
    let mut cmd = CommandBuilder::new("cmd.exe");
    cmd.args(["/d", "/k", "prompt $E[?1049hHGP$G"]);
    let pty::Opened { pty, mut output, input, held_failure } =
        pty::spawn(cmd, 80, 24, Arc::new(Slot(Arc::default())), true).expect("o ConPTY abre");
    input.send(Bytes::from_static(b"echo hangar-^cedo\r")).await.unwrap();
    let mut seen = Vec::new();
    assert!(read_until(&mut output, Some(&input), &mut seen, Duration::from_secs(4), |s| count(s, "hangar-cedo") >= 1).await,
            "a tecla segurada não chegou antes do prazo: {}", text(&seen));
    let mut held_failure = held_failure.expect("portão ligado");
    assert!(held_failure.try_recv().is_err(), "a porta abriu sem o sinal");
    drop(input);
    pty::close(pty).await.expect("o filho sai e o pseudoconsole fecha");
}
