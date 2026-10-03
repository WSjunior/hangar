use hangar_server::terminal_control::{CaptureRequest, ControlEvent, ControlParser, FrameIdentity, Screen, TerminalPool, Limits};
use std::{path::PathBuf, process::Command, time::Duration};

fn request(consumer: &str) -> CaptureRequest {
    CaptureRequest { consumer: consumer.into(), name: "fixture".into(), provider: "claude".into(),
        binding: "conversation-a".into(), target: "=fixture:".into(), started: 42.5,
        lines: 200, colors: false, join: false }
}

#[test]
fn fragmented_frames_preserve_percent_body_and_exact_blank_lines() {
    let mut parser = ControlParser::default();
    let mut events = Vec::new();
    for chunk in b"%begin 1 7 0\n%begin exemplo literal\n%end 1 8 0\n\na\n%end 1 7 0\n".chunks(2) {
        events.extend(parser.push(chunk).unwrap());
    }
    assert_eq!(events, vec![ControlEvent::Frame { identity: FrameIdentity { timestamp: 1, command: 7, flags: 0 }, text: "%begin exemplo literal\n%end 1 8 0\n\na\n".into(), error: false }]);
}

#[test]
fn output_decodes_octal_bytes_without_consuming_frame_body() {
    let mut parser = ControlParser::default();
    assert_eq!(parser.push(b"%output %3 ol\\303\\241\\134\n%begin 2 8 0\n%output %4 literal\n%error 2 8 0\n").unwrap(), vec![
        ControlEvent::Output { pane: "%3".into(), bytes: "olá\\".as_bytes().to_vec() },
        ControlEvent::Frame { identity: FrameIdentity { timestamp: 2, command: 8, flags: 0 }, text: "%output %4 literal\n".into(), error: true },
    ]);
}

#[test]
fn invalid_utf8_and_oversized_or_unfinished_frames_are_errors() {
    let mut parser = ControlParser::default();
    assert!(parser.push(b"%begin 1 1 0\n\xff\n%end 1 1 0\n").is_err());
    let mut parser = ControlParser::default();
    assert!(parser.push(&vec![b'a'; 1024 * 1024 + 1]).is_err());
    let mut parser = ControlParser::default();
    parser.push(b"%begin 1 1 0\nbody\n").unwrap();
    assert!(parser.finish().is_err());
    let mut parser = ControlParser::default();
    assert!(parser.push(b"%output %2 bad\\12\n").is_err());
}

#[test]
fn screen_tracks_split_utf8_color_cursor_and_alternate_without_answering_ansi() {
    let mut screen = Screen::new(20, 4).unwrap();
    screen.feed(b"\x1b[31mol\xc3").unwrap();
    screen.feed(b"\xa1\x1b[0m\x1b[6n").unwrap();
    assert_eq!(screen.text(), "olá\n\n\n\n");
    assert_eq!(screen.cursor(), (3, 0));
    assert_eq!(screen.cell(0, 0).unwrap().fg, alacritty_terminal::vte::ansi::Color::Named(alacritty_terminal::vte::ansi::NamedColor::Red));
    screen.feed(b"\x1b[?1049hALT").unwrap();
    assert!(screen.alternate());
    assert!(screen.text().contains("ALT"));
    screen.feed(b"\x1b[?1049l").unwrap();
    assert_eq!(screen.text(), "olá\n\n\n\n");
    assert!(Screen::new(0, 4).is_err());
}

#[test]
fn restoring_pending_wrap_cursor_wraps_the_next_character_without_overwrite() {
    let mut screen = Screen::new(80, 3).unwrap();
    screen.feed("x".repeat(80).as_bytes()).unwrap();
    screen.restore_cursor(80, 0).unwrap();
    assert_eq!(screen.cursor(), (80, 0));
    screen.feed(b"Z").unwrap();
    assert_eq!(screen.text(), format!("{}\nZ\n\n", "x".repeat(80)));
    assert_eq!(screen.cursor(), (1, 1));
    assert!(screen.restore_cursor(81, 0).is_err());
}

#[cfg(unix)]
struct IsolatedTmux { _dir: tempfile::TempDir, socket: PathBuf }
#[cfg(unix)]
impl IsolatedTmux {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("tmux.sock");
        let server = Self { _dir: dir, socket };
        server.run(&["-f", "/dev/null", "new-session", "-d", "-s", "fixture", "-x", "120", "-y", "30", "-e", "HANGAR_PROBE=kept", "cat"]);
        server
    }
    fn run(&self, args: &[&str]) -> String {
        let output = Command::new("tmux").arg("-u").arg("-S").arg(&self.socket).args(args).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap()
    }
    fn clients(&self) -> String { self.run(&["list-clients", "-t", "=fixture", "-F", "#{client_pid}\t#{client_control_mode}\t#{client_tty}"]) }
    fn pool(&self) -> TerminalPool { TerminalPool::with_program("tmux", Some(self.socket.clone()), Limits::default()) }
    async fn await_text(&self, pool: &TerminalPool, r: &CaptureRequest, text: &str) -> String {
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                let value = pool.capture(r.clone()).await.unwrap().text;
                if value.contains(text) { break value; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap()
    }
}
#[cfg(unix)]
impl Drop for IsolatedTmux { fn drop(&mut self) { let _ = Command::new("tmux").arg("-S").arg(&self.socket).arg("kill-server").output(); } }

#[cfg(unix)]
#[tokio::test]
async fn isolated_clients_share_pid_and_last_release_reaps_only_observer() {
    let server = IsolatedTmux::new();
    let pool = server.pool();
    let before = server.run(&["display-message", "-p", "-t", "=fixture:", "#{pane_width}x#{pane_height}"]);
    let env = server.run(&["show-environment", "-t", "=fixture", "HANGAR_PROBE"]);
    let r = request("state");
    let captured = pool.capture(r.clone()).await.unwrap();
    assert_eq!(captured.binding, "conversation-a");
    assert_eq!(captured.started, 42.5);
    let clients = server.clients();
    assert_eq!(clients.lines().count(), 1);
    assert!(clients.ends_with("\t1\t\n"), "{clients}");
    pool.acquire(request("preview")).await.unwrap();
    pool.capture(r).await.unwrap();
    assert_eq!(server.clients(), clients);
    pool.release("state").await.unwrap();
    assert_eq!(server.clients(), clients);
    pool.release("preview").await.unwrap();
    assert_eq!(server.clients(), "");
    assert_eq!(server.run(&["display-message", "-p", "-t", "=fixture:", "#{pane_width}x#{pane_height}"]), before);
    assert_eq!(server.run(&["show-environment", "-t", "=fixture", "HANGAR_PROBE"]), env);
    pool.release("unknown").await.unwrap();
    assert_eq!(server.clients(), "");
}

#[cfg(unix)]
#[tokio::test]
async fn canonical_capture_matches_tmux_history_ansi_join_blanks_and_literal_percent() {
    let server = IsolatedTmux::new();
    let pool = server.pool();
    server.run(&["respawn-pane", "-k", "-t", "=fixture:", "python3 -u -c 'import sys,time; print(\"\\n\".join(\"history-%d\"%i for i in range(40))); print(\"%begin exemplo literal\"); print(\"\\x1b[31molá\\x1b[0m\"); print(\"wrap-\"+\"x\"*250); print(\"\\nblank\\n\"); time.sleep(30)'"]);
    let mut r = request("state");
    server.await_text(&pool, &r, "blank").await;
    for (colors, join) in [(false, false), (true, false), (false, true), (true, true)] {
        r.colors = colors; r.join = join;
        let result = pool.capture(r.clone()).await.unwrap();
        let mut args = vec!["capture-pane", "-p", "-t", "=fixture:", "-S", "-200"];
        if colors { args.push("-e"); }
        if join { args.push("-J"); }
        assert_eq!(result.text, server.run(&args));
        assert!(result.text.contains("%begin exemplo literal"));
        assert!(result.text.contains("history-0"));
    }
    server.run(&["resize-window", "-t", "=fixture:", "-x", "90", "-y", "22"]);
    assert_eq!(pool.capture(r).await.unwrap().text, server.run(&["capture-pane", "-p", "-e", "-J", "-t", "=fixture:", "-S", "-200"]));
    pool.release("state").await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn invalid_requests_and_exact_target_changes_never_reuse_old_capture() {
    let server = IsolatedTmux::new();
    let pool = server.pool();
    let mut r = request("state");
    let original = server.run(&["display-message", "-p", "-t", "=fixture:", "#{pane_id}"]).trim().to_owned();
    r.target = original.clone();
    pool.capture(r.clone()).await.unwrap();
    server.run(&["split-window", "-d", "-t", "=fixture:", "cat"]);
    server.run(&["kill-pane", "-t", &original]);
    assert!(pool.capture(r.clone()).await.is_err());
    for invalid in ["=fixture:\nkill-server", "$(kill-server)", "=fixture:;kill-server"] {
        r.target = invalid.into();
        assert!(pool.capture(r.clone()).await.is_err());
    }
    r.target = "=fixture:".into(); r.started = f64::NAN;
    assert!(pool.capture(r).await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn idle_expiry_is_not_postponed_by_continuous_output() {
    let server = IsolatedTmux::new();
    server.run(&["respawn-pane", "-k", "-t", "=fixture:", "python3 -u -c 'import time; exec(\"while True:\\n print(\\\"stream\\\"); time.sleep(.01)\")'"]);
    let limits = Limits { lease: Duration::from_millis(200), ..Limits::default() };
    let pool = TerminalPool::with_program("tmux", Some(server.socket.clone()), limits);
    pool.acquire(request("state")).await.unwrap();
    assert_eq!(server.clients().lines().count(), 1);
    tokio::time::sleep(Duration::from_millis(450)).await;
    assert_eq!(server.clients(), "");
    assert!(server.run(&["display-message", "-p", "-t", "=fixture:", "#{pane_pid}"]).trim().parse::<u32>().is_ok());
}

#[cfg(unix)]
#[tokio::test]
async fn child_startup_timeout_and_eof_are_errors_and_reaped() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fake");
    for body in ["#!/bin/sh\nexec sleep 30\n", "#!/bin/sh\nexit 0\n"] {
        std::fs::write(&script, body).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let limits = Limits { startup: Duration::from_millis(100), command: Duration::from_millis(100), ..Limits::default() };
        let pool = TerminalPool::with_program(script.clone(), None, limits);
        assert!(tokio::time::timeout(Duration::from_secs(2), pool.capture(request("state"))).await.unwrap().is_err());
    }
}

#[cfg(unix)]
fn fake_observer(mode: &str) -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("observer");
    let pid = dir.path().join("pid");
    let log = dir.path().join("commands");
    let source = r#"#!/usr/bin/python3
import os, sys, time
from pathlib import Path
root = Path(__file__).parent
root.joinpath('pid').write_text(str(os.getpid()))
mode = '__MODE__'
def frame(n, body=''):
    sys.stdout.write(f'%begin 1 {n} 0\n' + body + f'%end 1 {n} 0\n')
    sys.stdout.flush()
frame(0)
for n, line in enumerate(sys.stdin, 1):
    with root.joinpath('commands').open('a') as log:
        log.write(line)
    commands = line.rstrip('\n').split(' ; ')
    wrapped = len(commands) == 3
    command = commands[1] if wrapped else commands[0]
    if not command.startswith(('display-message -p -t ', 'capture-pane -p ')):
        sys.exit(7)
    if wrapped and (not commands[0].startswith('display-message -p -l HG_START_') or not commands[2].startswith('display-message -p -l HG_END_')):
        sys.exit(8)
    start = commands[0].removeprefix('display-message -p -l ') + '\n'
    end = commands[2].removeprefix('display-message -p -l ') + '\n' if wrapped else ''
    if n == 4 and mode == 'late':
        body = '%begin 1 99 0\nstale\n\n\n\n%end 1 99 0\n'
        for chunk in [body[:7], body[7:21], body[21:]]:
            time.sleep(.01)
            sys.stdout.write(chunk)
            sys.stdout.flush()
        continue
    if wrapped:
        frame(n * 100, start)
    if n >= 3 and mode == 'timeout':
        time.sleep(30)
    if n >= 3 and mode == 'eof':
        sys.exit(0)
    if n >= 3 and mode == 'utf8':
        sys.stdout.buffer.write(f'%begin 1 {n * 100 + 17} 0\n'.encode() + b'\xff\n' + f'%end 1 {n * 100 + 17} 0\n'.encode())
        sys.stdout.buffer.flush()
        continue
    if n >= 3 and mode == 'error':
        sys.stdout.write(f'%begin 1 {n * 100 + 17} 0\nfailed\n%error 1 {n * 100 + 17} 0\n')
        sys.stdout.flush()
        continue
    if n == 3 and mode == 'prefill':
        frames = [(n * 100 + 17, '%3\tfixture\t20\t4\t0\t0\t0\n'), (78, 'stale\n\n\n\n'),
                  (79, 'stale\n\n\n\n'), (80, '%3\tfixture\t20\t4\t0\t0\t0\n')]
        sys.stdout.write(''.join(f'%begin 1 {index} 0\n' + body + f'%end 1 {index} 0\n' for index, body in frames))
        sys.stdout.flush()
        continue
    if n == 2:
        sys.stdout.write('%output %9 wrong-pane\n%output %3 \\033[6n\n')
    frame(n * 100 + 17, '%3\tfixture\t20\t4\t0\t0\t0\n' if command.startswith('display-message') else 'ready\n\n\n\n')
    if wrapped:
        if n == 4 and mode == 'missing-end':
            continue
        if n == 4 and mode == 'late-end':
            time.sleep(.12)
            root.joinpath('end-sent').write_text('yes')
        frame(n * 100 + 39, 'wrong-marker\n' if n == 4 and mode == 'wrong-end' else end)
"#.replace("__MODE__", mode);
    std::fs::write(&program, source).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    (dir, program, pid, log)
}

#[cfg(unix)]
fn process_exists(pid: &str) -> bool {
    Command::new("kill").args(["-0", pid]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().unwrap().success()
}

#[cfg(unix)]
#[tokio::test]
async fn command_timeout_active_eof_protocol_error_and_bad_utf8_discard_observer() {
    for mode in ["timeout", "eof", "utf8", "error"] {
        let (_dir, program, pid_file, _) = fake_observer(mode);
        let limits = Limits { startup: Duration::from_millis(500), command: Duration::from_millis(150), ..Limits::default() };
        let pool = TerminalPool::with_program(program, None, limits);
        pool.acquire(request("state")).await.unwrap();
        let pid = std::fs::read_to_string(&pid_file).unwrap();
        assert!(process_exists(&pid));
        let error = pool.capture(request("state")).await.unwrap_err();
        assert!(matches!(error.0, "terminal command timeout" | "terminal observer EOF" | "invalid frame UTF-8" | "tmux command failed"), "{mode}: {error}");
        tokio::time::timeout(Duration::from_secs(2), async {
            while process_exists(&pid) { tokio::time::sleep(Duration::from_millis(10)).await; }
        }).await.unwrap();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn ansi_queries_do_not_write_to_terminal_and_final_release_reaps_child() {
    let (_dir, program, pid_file, log) = fake_observer("normal");
    let pool = TerminalPool::with_program(program, None, Limits::default());
    assert_eq!(pool.capture(request("state")).await.unwrap().text, "ready\n\n\n\n");
    let pid = std::fs::read_to_string(pid_file).unwrap();
    pool.release("state").await.unwrap();
    assert!(!process_exists(&pid));
    let commands = std::fs::read_to_string(log).unwrap();
    assert!(commands.lines().flat_map(|line| line.split(" ; ")).all(|command| command.starts_with("display-message -p -t ") || command.starts_with("capture-pane -p ") || command.starts_with("display-message -p -l HG_")));
    assert!(!commands.contains('\x1b'));
}

#[cfg(unix)]
#[tokio::test]
async fn unsolicited_queued_frames_never_supply_a_successful_capture() {
    let (_dir, program, _, _) = fake_observer("prefill");
    let pool = TerminalPool::with_program(program, None, Limits::default());
    assert!(pool.capture(request("state")).await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn late_fragmented_unsolicited_frame_never_serves_the_next_request() {
    let (_dir, program, _, _) = fake_observer("late");
    let pool = TerminalPool::with_program(program, None, Limits::default());
    assert!(pool.capture(request("state")).await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn capture_waits_for_end_marker_and_rejects_missing_or_wrong_end() {
    for mode in ["missing-end", "wrong-end", "late-end"] {
        let (dir, program, _, _) = fake_observer(mode);
        let limits = Limits { command: Duration::from_millis(250), ..Limits::default() };
        let pool = TerminalPool::with_program(program, None, limits);
        let result = pool.capture(request("state")).await;
        if mode == "late-end" {
            assert_eq!(result.unwrap().text, "ready\n\n\n\n");
            assert!(dir.path().join("end-sent").exists());
            pool.release("state").await.unwrap();
        } else {
            assert!(matches!(result.unwrap_err().0, "terminal command timeout" | "terminal command marker mismatch"));
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn exact_full_line_without_newline_preserves_pending_wrap_cursor() {
    let server = IsolatedTmux::new();
    server.run(&["resize-window", "-t", "=fixture:", "-x", "80", "-y", "30"]);
    server.run(&["respawn-pane", "-k", "-t", "=fixture:", "python3 -u -c 'import sys,time; sys.stdout.write(\"x\"*80); sys.stdout.flush(); time.sleep(30)'"]);
    tokio::time::timeout(Duration::from_secs(3), async {
        while server.run(&["display-message", "-p", "-t", "=fixture:", "#{pane_width}\t#{cursor_x}"]) != "80\t80\n" {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    let pool = server.pool();
    let text = pool.capture(request("state")).await.unwrap().text;
    assert_eq!(text, server.run(&["capture-pane", "-p", "-t", "=fixture:", "-S", "-200"]));
    assert!(text.starts_with(&format!("{}\n", "x".repeat(80))));
    pool.release("state").await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn consumer_binding_change_reaps_old_actor_before_new_capture() {
    let server = IsolatedTmux::new();
    let pool = server.pool();
    let mut r = request("state");
    pool.capture(r.clone()).await.unwrap();
    let old_pid = server.clients().split('\t').next().unwrap().to_owned();
    r.binding = "conversation-b".into();
    let result = pool.capture(r).await.unwrap();
    assert_eq!(result.binding, "conversation-b");
    let new_pid = server.clients().split('\t').next().unwrap().to_owned();
    assert_ne!(old_pid, new_pid);
    #[cfg(unix)]
    assert!(!process_exists(&old_pid));
    pool.release("state").await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn another_session_pane_and_changed_active_target_fail_without_substitution() {
    let server = IsolatedTmux::new();
    let pool = server.pool();
    let r = request("state");
    pool.capture(r.clone()).await.unwrap();
    server.run(&["split-window", "-t", "=fixture:", "cat"]);
    assert_eq!(pool.capture(r).await.unwrap_err().0, "terminal target changed");
    server.run(&["new-session", "-d", "-s", "other", "cat"]);
    let id = server.run(&["display-message", "-p", "-t", "=other:", "#{pane_id}"]).trim().to_owned();
    let mut r = request("other"); r.target = id;
    assert_eq!(pool.capture(r).await.unwrap_err().0, "invalid terminal metadata");
}

#[test]
fn unterminated_ansi_stream_is_bounded_and_invalid_notification_utf8_is_error() {
    let mut screen = Screen::new(20, 4).unwrap();
    screen.feed(b"\x1b]0;").unwrap();
    let data = vec![b'x'; 1024 * 1024];
    for _ in 0..7 { screen.feed(&data).unwrap(); }
    assert!(screen.feed(&data).is_err());
    let mut parser = ControlParser::default();
    assert!(parser.push(b"%notice \xff\n").is_err());
}

#[cfg(windows)]
#[tokio::test]
async fn windows_never_launches_tmux_or_psmux() {
    let pool = TerminalPool::with_program("does-not-exist", None, Limits::default());
    assert_eq!(pool.capture(request("state")).await.unwrap_err().0, "terminal control unavailable");
    pool.release("state").await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn actual_alternate_screen_capture_and_numeric_session_keep_exact_target() {
    let server = IsolatedTmux::new();
    server.run(&["respawn-pane", "-k", "-t", "=fixture:", "python3 -u -c 'import sys,time; sys.stdout.write(\"main\\x1b[?1049h\\x1b[2J\\x1b[HALTERNATE\"); sys.stdout.flush(); time.sleep(30)'"]);
    let pool = server.pool();
    let r = request("state");
    server.await_text(&pool, &r, "ALTERNATE").await;
    assert_eq!(server.run(&["display-message", "-p", "-t", "=fixture:", "#{alternate_on}"]), "1\n");
    assert_eq!(pool.capture(r).await.unwrap().text, server.run(&["capture-pane", "-p", "-t", "=fixture:", "-S", "-200"]));
    server.run(&["new-session", "-d", "-s", "0", "python3 -u -c 'import time; print(\"NUMERIC_SESSION\"); time.sleep(30)'"]);
    let mut numeric = request("numeric"); numeric.name = "0".into(); numeric.target = "=0:".into();
    assert!(server.await_text(&pool, &numeric, "NUMERIC_SESSION").await.contains("NUMERIC_SESSION"));
    pool.release("numeric").await.unwrap();
    pool.release("state").await.unwrap();
}
