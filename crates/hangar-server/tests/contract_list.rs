//! Lista de sessões contra as entradas e saídas gravadas pelo Python (`gen_list.py`).
mod common;

use hangar_api::session::SessionRow;
use hangar_server::list::discover_other::{self, Dirs};
use hangar_server::list::links;
use hangar_server::list::mux::Pane;
use hangar_server::list::procs::{ChildrenMap, ProcessView};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

const ROOT: &str = "⟦ROOT⟧";
const SAN: &str = "⟦ROOT_SAN⟧";

/// A pasta de um caso: os marcadores do golden viram caminhos de verdade nela.
struct World {
    _dir: tempfile::TempDir,
    root: String,
}

impl World {
    fn new() -> Self {
        // Sem ponto no nome: a pasta do Pi troca só separador, o `⟦ROOT_SAN⟧` troca todo símbolo.
        let dir = tempfile::Builder::new().prefix("hangarlist").tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap().to_string_lossy().into_owned();
        assert!(!root.contains('.'), "{root}");
        Self { _dir: dir, root }
    }

    fn real_str(&self, s: &str) -> String {
        s.replace(SAN, &hangar_workspace::worktrees::sanitize_cwd(&self.root)).replace(ROOT, &self.root)
    }

    fn real(&self, v: &Value) -> Value {
        match v {
            Value::String(s) => Value::String(self.real_str(s)),
            Value::Array(a) => Value::Array(a.iter().map(|x| self.real(x)).collect()),
            Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (self.real_str(k), self.real(x))).collect()),
            other => other.clone(),
        }
    }

    fn dirs(&self) -> Dirs {
        let home = PathBuf::from(&self.root).join("home");
        Dirs {
            claude: home.join(".claude"),
            codex_home: home.join(".codex"),
            pi_sessions: home.join(".pi/agent/sessions"),
            omp_config: home.join(".omp"),
            omp_agent: home.join(".omp/agent"),
            kimi_home: home.join(".kimi-code"),
            home,
        }
    }

    fn apply_fs(&self, ops: &[Value], at: f64) {
        for op in ops {
            let path = PathBuf::from(self.real_str(op["path"].as_str().unwrap()));
            match op["op"].as_str().unwrap() {
                "mkdir" => std::fs::create_dir_all(&path).unwrap(),
                "rm" => {
                    if path.is_dir() { std::fs::remove_dir_all(&path).unwrap() } else { std::fs::remove_file(&path).unwrap() }
                }
                "write" => {
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(&path, self.real_str(op["text"].as_str().unwrap())).unwrap();
                    set_mtime(&path, op["mtime"].as_f64().unwrap_or(at));
                }
                "touch" => set_mtime(&path, op["mtime"].as_f64().unwrap()),
                other => panic!("op {other}"),
            }
        }
    }
}

fn set_mtime(path: &Path, at: f64) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs_f64(at)).unwrap();
}

/// O `/proc` do tique.
struct FakeProcs(HashMap<i64, Value>);

impl ProcessView for FakeProcs {
    fn children(&self, _: Duration) -> io::Result<Arc<ChildrenMap>> {
        let mut pids: Vec<i64> = self.0.keys().copied().collect();
        pids.sort();
        let mut map = ChildrenMap::new();
        for pid in pids {
            map.entry(self.0[&pid]["ppid"].as_i64().unwrap()).or_default().push(pid);
        }
        Ok(Arc::new(map))
    }
    fn argv(&self, pid: i64) -> Vec<String> {
        self.0.get(&pid).map(|p| p["argv"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_owned()).collect()).unwrap_or_default()
    }
    fn cwd(&self, pid: i64) -> Option<PathBuf> { self.0.get(&pid).map(|p| PathBuf::from(p["cwd"].as_str().unwrap())) }
    fn env_var(&self, pid: i64, name: &str) -> io::Result<Option<OsString>> {
        let p = self.0.get(&pid).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        Ok(p["env"].get(name).and_then(Value::as_str).map(OsString::from))
    }
    fn start_time(&self, pid: i64) -> Option<f64> { self.0.get(&pid).and_then(|p| p["start"].as_f64()) }
    fn fds(&self, pid: i64) -> Vec<PathBuf> {
        self.0.get(&pid).map(|p| p["fds"].as_array().unwrap().iter().map(|f| PathBuf::from(f.as_str().unwrap())).collect()).unwrap_or_default()
    }
}

const AGENTS: [&str; 7] = ["pi", "omp", "claude", "kimi", "kimi-code", "codex", "hangar-codex-tui"];

/// Pid do agente no pane (a descida da Task 6), só para saber de quem ler motor e conta.
fn agent_pid(procs: &FakeProcs, pane_pid: Option<i64>) -> Option<i64> {
    let map = procs.children(Duration::ZERO).unwrap();
    let mut stack = vec![pane_pid?];
    while let Some(pid) = stack.pop() {
        let argv = procs.argv(pid);
        let cmd = argv.join(" ");
        let skip = cmd.contains("daemon") || cmd.contains("--bg-") || cmd.contains("--agent");
        if !skip && argv.first().is_some_and(|a| AGENTS.contains(&Path::new(a).file_name().unwrap().to_str().unwrap())) {
            return Some(pid);
        }
        stack.extend(map.get(&pid).into_iter().flatten());
    }
    None
}

fn pane_of(p: &Value) -> Pane {
    Pane {
        session: p["name"].as_str().unwrap().to_owned(),
        active: p["active"].as_bool().unwrap(),
        pid: p["pid"].as_u64().map(|v| v as u32),
        cwd: p["cwd"].as_str().unwrap().to_owned(),
        pane_id: p["pane_id"].as_str().unwrap().to_owned(),
        hidden: p["hidden"].as_bool().unwrap(),
        provider: p["provider"].as_str().map(str::to_owned),
        session_created: p["session_created"].as_u64(),
    }
}

/// O laço do `registry.list()` com as funções da Task 7. O provedor do pane e o transcript das
/// linhas Claude vêm do golden (são da Task 6); a escolha do pane repete a regra do agente.
fn list_rows(world: &World, tick: &Value, expected: &[Value]) -> Vec<SessionRow> {
    let dirs = world.dirs();
    let procs = FakeProcs(tick["procs"].as_array().unwrap().iter().map(|p| (p["pid"].as_i64().unwrap(), world.real(p))).collect());
    let panes: Vec<Pane> = tick["panes"].as_array().unwrap().iter().map(|p| pane_of(&world.real(p))).collect();
    let mut groups: Vec<(String, Vec<Pane>)> = Vec::new();
    for pane in panes {
        match groups.iter_mut().find(|(n, _)| *n == pane.session) {
            Some((_, g)) => g.push(pane),
            None => groups.push((pane.session.clone(), vec![pane])),
        }
    }
    let births: HashMap<String, u64> = groups.iter().filter_map(|(n, g)| g[0].session_created.map(|b| (n.clone(), b))).collect();
    let by_name = |name: &str| expected.iter().find(|r| r["name"] == name).cloned();
    let mut rows = Vec::new();
    for (name, group) in &groups {
        if group[0].hidden || discover_other::has_codex_sidecar(name, &dirs) {
            continue;
        }
        let mut ordered: Vec<&Pane> = group.iter().collect();
        ordered.sort_by_key(|p| !p.active);
        let pane = if group.len() > 1 {
            ordered.iter().find(|p| agent_pid(&procs, p.pid.map(i64::from)).is_some()).copied().unwrap_or(ordered[0])
        } else {
            &group[0]
        };
        let golden = by_name(name).unwrap_or_else(|| panic!("linha {name} fora do golden"));
        let provider = golden["provider"].as_str().unwrap_or("claude").to_owned();
        let mut row = links::blank_row(name);
        row.cwd = Some(pane.cwd.clone());
        let (jsonl, tracked) = match provider.as_str() {
            "pi" | "omp" | "kimi" => {
                let t = discover_other::ticket_transcript(&provider, pane, &procs, &dirs);
                let tracked = t.is_some();
                (t, tracked)
            }
            "codex" => (None, false),
            _ => (golden["jsonl"].as_str().map(str::to_owned), golden["tracked"].as_bool().unwrap_or(true)),
        };
        row.jsonl = jsonl;
        row.tracked = tracked;
        let pane_pid = pane.pid.map(i64::from);
        let pid_env = agent_pid(&procs, pane_pid).or(pane_pid);
        links::fill_pane_row(&mut row, &provider, pid_env, pane.session_created, &procs, &dirs);
        rows.push(row);
    }
    rows.extend(discover_other::codex_rows(&dirs, &births, &procs));
    rows.extend(discover_other::headless_rows(&dirs, &procs));
    rows
}

/// A linha como o golden grava: campo igual ao padrão sai.
fn dump(row: &SessionRow, defaults: &Map<String, Value>) -> Value {
    let Value::Object(mut map) = serde_json::to_value(row).unwrap() else { unreachable!() };
    map.retain(|k, v| defaults.get(k) != Some(v));
    Value::Object(map)
}

/// Repete cada caso tique a tique e devolve as divergências.
fn replay(golden: &Value, cases: &[&str]) -> Vec<String> {
    let defaults = golden["defaults"].as_object().unwrap();
    let t0 = golden["t0"].as_f64().unwrap();
    let mut failures = Vec::new();
    for name in cases {
        let case = golden["cases"].as_array().unwrap().iter().find(|c| c["name"] == *name).unwrap_or_else(|| panic!("caso {name}"));
        let world = World::new();
        for tick in case["ticks"].as_array().unwrap() {
            let at = tick["at"].as_f64().unwrap();
            world.apply_fs(tick["fs"].as_array().map(Vec::as_slice).unwrap_or(&[]), at);
            if tick["expected"].get("raises").is_some() {
                continue;
            }
            let expected: Vec<Value> = tick["expected"]["rows"].as_array().unwrap().iter().map(|r| world.real(r)).collect();
            let got: Vec<Value> = list_rows(&world, tick, &expected).iter().map(|r| dump(r, defaults)).collect();
            if got != expected {
                failures.push(format!("{name} @{}:\n  Rust   {}\n  Python {}", at - t0, json!(got), json!(expected)));
            }
        }
    }
    failures
}

#[test]
fn discovery_other_sequences() {
    let golden = common::golden("list_discovery.json");
    let failures = replay(&golden, &["pi_ticket", "kimi_ticket", "codex_with_and_without_thread", "claude_headless"]);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn links_cases() {
    let golden = common::golden("list_discovery.json");
    // Os demais casos têm linhas Claude: aqui conferem vida, worktree, vínculos, motor e conta.
    let failures = replay(&golden, &[
        "links_worktree_engine_account", "claude_fd_open_then_locked", "claude_fd_aux_ignored",
        "claude_session_id_then_clear", "claude_session_id_with_sibling", "claude_marker_by_pid_cache_newest",
        "claude_marker_by_session_id", "seed_rename_forget", "created_under_one_second", "mux_unavailable",
        "agent_pane_choice", "collision",
    ]);
    assert!(failures.is_empty(), "{}", failures.join("\n"));

    // Loop pelo sidecar, nas linhas da decoração.
    let golden = common::golden("list_decorate.json");
    let case = golden["cases"].as_array().unwrap().iter().find(|c| c["name"] == "loop_stalled_last_reply_headless").unwrap();
    let world = World::new();
    let tick = &case["ticks"][0];
    world.apply_fs(tick["fs"].as_array().unwrap(), tick["at"].as_f64().unwrap());
    let rows = tick["expected"]["rows"].as_array().unwrap();
    assert!(rows.iter().any(|r| r.get("loop_status").is_some()), "o caso tem loop");
    for expected in rows {
        let mut row = links::blank_row(expected["name"].as_str().unwrap());
        links::fill_loop(&mut row, &world.dirs());
        for field in ["loop_status", "loop_iter", "loop_max"] {
            assert_eq!(serde_json::to_value(&row).unwrap()[field], expected.get(field).cloned().unwrap_or(Value::Null), "{} {field}", row.name);
        }
    }

    // Colisão: a dona pelo sid fica; sem ela, a única tracked; duas tracked, ninguém.
    let row = |name: &str, jsonl: &str, tracked: bool| {
        let mut r = links::blank_row(name);
        r.jsonl = Some(jsonl.to_owned());
        r.tracked = tracked;
        r
    };
    let sids = HashMap::from([("dona".to_owned(), Some("abc".to_owned())), ("b".to_owned(), None)]);
    let mut rows = vec![row("dona", "/x/abc.jsonl", false), row("b", "/x/abc.jsonl", true), row("so", "/x/s.jsonl", false)];
    links::dedupe_collisions(&mut rows, &sids);
    assert_eq!((rows[0].jsonl.as_deref(), rows[1].jsonl.as_deref(), rows[2].jsonl.as_deref()), (Some("/x/abc.jsonl"), None, Some("/x/s.jsonl")));
    assert!(!rows[1].tracked);
    let mut rows = vec![row("a", "/x/m.jsonl", false), row("b", "/x/m.jsonl", true)];
    links::dedupe_collisions(&mut rows, &HashMap::new());
    assert_eq!((rows[0].jsonl.as_deref(), rows[1].jsonl.as_deref()), (None, Some("/x/m.jsonl")));
    let mut rows = vec![row("a", "/x/m.jsonl", true), row("b", "/x/m.jsonl", true)];
    links::dedupe_collisions(&mut rows, &HashMap::new());
    assert!(rows.iter().all(|r| r.jsonl.is_none() && !r.tracked));
}

#[test]
fn ticket_key_and_pi_paths() {
    let procs = FakeProcs(HashMap::from([(7, json!({"ppid": 1, "argv": ["omp"], "cwd": "/", "env": {"PSMUX_SESSION": "omp s/1"}, "start": 0.0, "fds": []}))]));
    assert_eq!(discover_other::ticket_key("%%3", None, &procs), "3");
    assert_eq!(discover_other::ticket_key("%3", Some(7), &procs), "omp-s-1");
    assert!(discover_other::is_pi_subagent("/s/x/2027-01-15T08-00-00-000Z_88888888-8888-4888-8888-888888888888/44bad0fb/a.jsonl"));
    assert!(discover_other::is_pi_subagent("/s/x/run-2/a.jsonl"));
    assert!(!discover_other::is_pi_subagent("/s/x/2027-01-15T08-00-00-000Z_88888888-8888-4888-8888-888888888888.jsonl"));
    // Valores calculados pelo `kimi_sessions.workdir_key` do Python.
    assert_eq!(discover_other::kimi_workdir_key("/tmp/kimi-acp-probe"), "wd_kimi-acp-probe_15ca61fc9ec9");
    assert_eq!(discover_other::sanitize_session_name("Área de trabalho."), "Area-de-trabalho");
}
