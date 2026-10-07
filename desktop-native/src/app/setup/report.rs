//! Relatório da falha (spec "Falha e relatório"): o log da instalação, o doctor, etapa e código, sistema e versão do app,
//! limpo de senha, nome do usuário, nomes da tailnet e IPs. A pessoa vê exatamente o texto que sai.
use std::{path::{Path, PathBuf}, process::{Command, Stdio}, time::Duration};
use serde::Serialize;
use crate::i18n::tr;
use super::flow::Screen;
use super::system::{hidden, refreshed_path};

/// O Worker aceita 256 KB de JSON: o texto fica abaixo disso com folga para o escape e para a seção do agente.
pub(crate) const BASE_MAX: usize = 140 * 1024;
pub(crate) const AGENT_MAX: usize = 60 * 1024;
pub(crate) const URL: &str = "https://hangar.dev.br/api/relatorio";
const SECRET: &str = "<senha>";
const USER: &str = "<usuario>";
const IP: &str = "<ip>";
const TAILNET_HOST: &str = "<maquina>";
/// Senha mais curta que isto não é procurada no texto: trocaria letras soltas no relatório inteiro.
const MIN_SECRET: usize = 4;
const SECRET_KEYS: [&str; 3] = ["token=", "password=", "bearer "];

pub(crate) struct Secrets { pub values: Vec<String>, pub home: Option<String>, pub users: Vec<String> }

impl Secrets {
    /// Os segredos que o app tem agora, mais a pasta pessoal e o nome do usuário deste computador.
    pub(crate) fn here(values: Vec<String>) -> Self {
        let home_dir = std::env::home_dir();
        let mut users: Vec<String> = ["USER", "USERNAME"].iter().filter_map(|k| std::env::var(k).ok()).collect();
        users.extend(home_dir.as_ref().and_then(|h| h.file_name()).map(|n| n.to_string_lossy().into_owned()));
        users.retain(|u| u.chars().count() >= 2);
        users.sort();
        users.dedup();
        Self { values, home: home_dir.map(|h| h.to_string_lossy().into_owned()), users }
    }
}

pub(crate) fn scrub(text: &str, secrets: &Secrets) -> String {
    let mut out = text.to_owned();
    let mut values: Vec<&str> = secrets.values.iter().map(String::as_str).filter(|v| v.chars().count() >= MIN_SECRET).collect();
    values.sort_by_key(|v| std::cmp::Reverse(v.len()));
    for value in values { out = out.replace(value, SECRET); }
    out = scrub_assignments(&out);
    if let Some(home) = &secrets.home {
        out = replace_path(&out, home, "~");
        out = replace_path(&out, &home.replace('\\', "/"), "~");
    }
    for user in &secrets.users {
        out = replace_path(&out, &format!("/{user}"), &format!("/{USER}"));
        out = replace_path(&out, &format!("\\{user}"), &format!("\\{USER}"));
    }
    scrub_ips(&scrub_tailnet(&out))
}

/// Troca `needle` (sem diferenciar maiúsculas ASCII: caminho do Windows) só quando o nome acaba ali.
fn replace_path(text: &str, needle: &str, with: &str) -> String {
    if needle.is_empty() { return text.to_owned(); }
    let (lower, pattern) = (text.to_ascii_lowercase(), needle.to_ascii_lowercase());
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (at, _) in lower.match_indices(&pattern) {
        let end = at + needle.len();
        if text[end..].chars().next().is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.')) { continue; }
        out.push_str(&text[last..at]);
        out.push_str(with);
        last = end;
    }
    out.push_str(&text[last..]);
    out
}

/// `token=…`, `password=…` e `Bearer …`: senha que o app não conhecia (outra URL de pareamento, um cabeçalho).
fn scrub_assignments(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut starts: Vec<usize> = SECRET_KEYS.iter().flat_map(|k| lower.match_indices(k).map(move |(at, _)| at + k.len())).collect();
    starts.sort_unstable();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for start in starts {
        if start < last { continue; }
        let end = text[start..].find(|c: char| c.is_whitespace() || matches!(c, '&' | '"' | '\'' | ',' | ';' | ')' | '<'))
            .map_or(text.len(), |n| start + n);
        if end == start { continue; }
        out.push_str(&text[last..start]);
        out.push_str(SECRET);
        last = end;
    }
    out.push_str(&text[last..]);
    out
}

fn scrub_tailnet(text: &str) -> String {
    const SUFFIX: &str = ".ts.net";
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (at, _) in lower.match_indices(SUFFIX) {
        let end = at + SUFFIX.len();
        if text[end..].chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '-') { continue; }
        let start = text[..at].char_indices().rev().take_while(|&(_, c)| c.is_ascii_alphanumeric() || c == '-' || c == '.')
            .last().map_or(at, |(i, _)| i);
        if start >= at || start < last { continue; }
        out.push_str(&text[last..start]);
        out.push_str(TAILNET_HOST);
        last = at;
    }
    out.push_str(&text[last..]);
    out
}

fn scrub_ips(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let (mut i, mut last) = (0, 0);
    while i < bytes.len() {
        let boundary = i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || matches!(bytes[i - 1], b'.' | b'_' | b':'));
        if boundary && let Some(len) = ipv4_at(&bytes[i..]).or_else(|| ipv6_at(&bytes[i..])) {
            out.push_str(&text[last..i]);
            out.push_str(IP);
            i += len;
            last = i;
            continue;
        }
        i += 1;
    }
    out.push_str(&text[last..]);
    out
}

fn ipv4_at(b: &[u8]) -> Option<usize> {
    let (mut at, mut octets) = (0, [0u32; 4]);
    for (n, slot) in octets.iter_mut().enumerate() {
        if n > 0 {
            if b.get(at) != Some(&b'.') { return None; }
            at += 1;
        }
        let digits = b[at..].iter().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 || digits > 3 { return None; }
        *slot = std::str::from_utf8(&b[at..at + digits]).ok()?.parse().ok()?;
        if *slot > 255 { return None; }
        at += digits;
    }
    // "1.2.3.4.5" e "1.2.3.4abc" não são IP (versão, número de build).
    if b.get(at).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') || (b.get(at) == Some(&b'.') && b.get(at + 1).is_some_and(u8::is_ascii_digit)) {
        return None;
    }
    // Loopback e "qualquer endereço" não identificam ninguém e ajudam a ler o log.
    if octets[0] == 127 || octets == [0, 0, 0, 0] { return None; }
    Some(at)
}

fn ipv6_at(b: &[u8]) -> Option<usize> {
    let len = b.iter().take_while(|c| c.is_ascii_hexdigit() || **c == b':').count();
    let run = &b[..len];
    let colons = run.iter().filter(|c| **c == b':').count();
    // Hora ("12:34:56") tem dois-pontos mas não tem "::" nem cinco separadores.
    if len - colons < 2 || colons < 2 || !(run.windows(2).any(|w| w == b"::") || colons >= 5) { return None; }
    if b.get(len).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') { return None; }
    Some(len)
}

/// O fim do texto em até `max` bytes, começando numa linha inteira: no log de instalação o que importa é o fim.
pub(crate) fn tail(text: &str, max: usize) -> String {
    if text.len() <= max { return text.to_owned(); }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) { start += 1; }
    let start = text[start..].find('\n').map_or(start, |n| start + n + 1);
    format!("\n{}\n{}", tr("setup_report_log_cut").replace("{bytes}", &start.to_string()), &text[start..])
}

pub(crate) struct Facts {
    pub step: Screen,
    pub code: Option<String>,
    pub text: String,
    pub system: String,
    pub app: String,
    pub doctor: Option<String>,
    /// (`check`/`install`, conteúdo do arquivo de saída), na ordem em que rodaram.
    pub logs: Vec<(String, String)>,
}

pub(crate) fn screen_slug(screen: Screen) -> &'static str {
    match screen {
        Screen::Welcome => "bem-vindo", Screen::Prepare => "preparar", Screen::Install => "instalar",
        Screen::Tailscale => "tailscale", Screen::Phone => "celular", Screen::Done => "final",
    }
}

/// Cada pedaço é limpo antes de montar; o log é cortado pelo começo para caber.
pub(crate) fn compose(f: &Facts, secrets: &Secrets) -> String {
    let code = match &f.code { Some(code) => tr("setup_failure_code").replace("{code}", code), None => tr("setup_failure_unexpected") };
    let mut head = format!("# {}\n\n{}\n{}\n{}\n{}\n{}\n\n## {}\n\n", tr("setup_report_heading"),
        tr("setup_report_step").replace("{step}", screen_slug(f.step)), code, scrub(&f.text, secrets),
        tr("setup_report_system").replace("{system}", &scrub(&f.system, secrets)), tr("setup_report_app").replace("{app}", &f.app),
        tr("setup_report_doctor"));
    head.push_str(&match &f.doctor { Some(text) => scrub(text, secrets), None => tr("setup_report_doctor_missing") });
    let logs: String = f.logs.iter().map(|(run, text)| format!("\n\n## {}\n\n{}", tr("setup_report_log").replace("{run}", run), scrub(text, secrets))).collect();
    let budget = BASE_MAX.saturating_sub(head.len() + 256);
    format!("{head}{}\n", tail(&logs, budget))
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Recheck { Passed, Failed(String), NotRun(String) }

pub(crate) struct AgentSection {
    pub agent: String,
    pub commands: Vec<String>,
    pub explanation: String,
    pub diff: String,
    pub notes: Vec<String>,
    pub recheck: Recheck,
}

/// A seção do conserto vem depois do relatório de base; o diff, que é o que mais cresce, fica por último e é o que corta.
pub(crate) fn with_agent(base: &str, s: &AgentSection, secrets: &Secrets) -> String {
    let recheck = match &s.recheck {
        Recheck::Passed => tr("setup_report_recheck_passed"),
        Recheck::Failed(detail) => tr("setup_report_recheck_failed").replace("{detail}", detail),
        Recheck::NotRun(detail) => tr("setup_report_recheck_not_run").replace("{detail}", detail),
    };
    let commands = s.commands.iter().map(|c| format!("$ {c}")).collect::<Vec<_>>().join("\n");
    let mut section = format!("\n\n# {}\n\n## {}\n\n{recheck}\n\n## {}\n\n{}\n\n## {}\n\n{commands}\n",
        tr("setup_report_agent").replace("{agente}", &s.agent), tr("setup_report_recheck"), tr("setup_report_explanation"),
        s.explanation, tr("setup_report_commands"));
    for note in &s.notes { section.push_str(&format!("\n{note}\n")); }
    let diff = if s.diff.is_empty() { tr("setup_report_no_diff") } else { s.diff.clone() };
    section.push_str(&format!("\n## {}\n\n{diff}\n", tr("setup_report_diff")));
    let section = scrub(&section, secrets);
    if section.len() <= AGENT_MAX { return format!("{base}{section}"); }
    let mut end = AGENT_MAX;
    while !section.is_char_boundary(end) { end -= 1; }
    format!("{base}{}\n{}\n", &section[..end], tr("setup_report_cut"))
}

pub(crate) fn restore_notes(head_moved: Option<&(String, String)>, errors: &[String], timed_out: bool) -> Vec<String> {
    let mut notes = Vec::new();
    if timed_out { notes.push(tr("setup_report_timed_out").replace("{min}", &(super::agent::MAX_RUN.as_secs() / 60).to_string())); }
    if let Some((before, after)) = head_moved { notes.push(tr("setup_report_head_moved").replace("{before}", before).replace("{after}", after)); }
    if !errors.is_empty() { notes.push(tr("setup_report_not_restored").replace("{detail}", &errors.join("; "))); }
    notes
}

fn venv_python(dest: &Path) -> PathBuf {
    if cfg!(windows) { dest.join(r"backend\.venv\Scripts\python.exe") } else { dest.join("backend/.venv/bin/python") }
}

/// `python -m app.doctor` pelo Python do venv, com o PATH refeito. Sem venv, o backend ainda não existe: `None`.
pub(crate) fn doctor(dest: &Path) -> Option<String> {
    let python = venv_python(dest);
    if !python.is_file() { return None; }
    // `--qr` imprimiria a URL com o token; sem ele o doctor só lista itens.
    let out = hidden(&mut Command::new(python)).args(["-m", "app.doctor"]).current_dir(dest.join("backend"))
        .env("PATH", refreshed_path()).env("PYTHONIOENCODING", "utf-8").stdin(Stdio::null()).output();
    Some(match out {
        Ok(out) => format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
        Err(error) => format!("doctor: {error}"),
    })
}

pub(crate) fn read_log(path: &Path) -> String {
    std::fs::read(path).map(|bytes| String::from_utf8_lossy(&bytes).into_owned()).unwrap_or_else(|e| format!("{}: {e}", path.display()))
}

pub(crate) fn system_line() -> String {
    let name = os_name().unwrap_or_else(|| std::env::consts::OS.to_owned());
    format!("{name} ({}-{})", std::env::consts::OS, std::env::consts::ARCH)
}

#[cfg(target_os = "linux")]
fn os_name() -> Option<String> {
    std::fs::read_to_string("/etc/os-release").ok()?.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=")).map(|v| v.trim_matches('"').to_owned())
}

#[cfg(windows)]
fn os_name() -> Option<String> {
    super::system::powershell("$o = Get-CimInstance Win32_OperatingSystem; \"$($o.Caption) $($o.Version)\"").ok()
}

#[cfg(not(any(target_os = "linux", windows)))]
fn os_name() -> Option<String> { None }

pub(crate) fn app_line() -> String { format!("{} · {}", crate::update::CURRENT, super::run::COMMIT) }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Outcome { Consertado, Aberto }

/// O corpo do `POST /api/relatorio` (Task 5 confere cada campo).
#[derive(Clone, Debug, Serialize)]
pub(crate) struct Payload {
    pub v: u8,
    pub app: String,
    pub commit: String,
    pub os: String,
    pub step: String,
    pub code: Option<String>,
    pub outcome: Outcome,
    pub agent: Option<String>,
    pub report: String,
}

pub(crate) fn payload(step: Screen, code: Option<String>, outcome: Outcome, agent: Option<&str>, report: String) -> Payload {
    let code = code.filter(|c| (1..=64).contains(&c.len()) && c.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'));
    Payload { v: 1, app: crate::update::CURRENT.to_owned(), commit: super::run::COMMIT.to_owned(),
        os: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH), step: screen_slug(step).to_owned(), code, outcome,
        agent: agent.map(str::to_owned), report }
}

/// Carimbo do app: barra robô genérico, não alguém determinado (o código é aberto).
pub(crate) fn stamp() -> String { format!("hangar-native/{}", crate::update::CURRENT) }

/// `HANGAR_REPORT_URL` só para provar o envio com um receptor local, sem tocar no Worker de verdade.
pub(crate) async fn send(payload: Payload) -> Result<(), String> {
    let url = std::env::var("HANGAR_REPORT_URL").unwrap_or_else(|_| URL.to_owned());
    send_to(&url, payload).await
}

pub(crate) async fn send_to(url: &str, payload: Payload) -> Result<(), String> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
    client.post(url).header("X-Hangar-Stamp", stamp()).json(&payload).send().await
        .and_then(reqwest::Response::error_for_status).map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secrets() -> Secrets {
        Secrets { values: vec!["s3nh4-do-celular".into(), "abcdef0123456789token".into(), "adm1n!".into()],
            home: Some("/home/maria".into()), users: vec!["maria".into()] }
    }

    #[test]
    fn scrub_removes_known_secrets_and_token_assignments() {
        let text = "CP_AUTH_TOKEN=abcdef0123456789token\nabra http://192.168.0.5:5173/?token=outro-token-qualquer&api=x\n\
            sudo: adm1n! errado\nAuthorization: Bearer eyJabc.def\nHANGAR_TOKEN=s3nh4-do-celular\npassword=segredo123 ok\n";
        let out = scrub(text, &secrets());
        for leaked in ["abcdef0123456789token", "outro-token-qualquer", "adm1n!", "eyJabc.def", "s3nh4-do-celular", "segredo123", "192.168.0.5"] {
            assert!(!out.contains(leaked), "{leaked} vazou: {out}");
        }
        assert!(out.contains("CP_AUTH_TOKEN=<senha>"));
        assert!(out.contains("&api=x"));
        assert!(out.contains("HANGAR_TOKEN=<senha>"));
    }

    #[test]
    fn scrub_removes_the_askpass_one_time_code() {
        let code = "0123456789abcdef0123456789abcdef";
        let s = Secrets::here(vec![code.into()]);
        let out = scrub(&format!("HANGAR_ASKPASS_CODE vazou {code} no log"), &s);
        assert!(!out.contains(code), "{out}");
        assert!(out.contains("vazou <senha> no log"), "{out}");
    }

    #[test]
    fn scrub_swaps_user_paths_tailnet_names_and_ips_but_keeps_loopback() {
        let s = Secrets { values: vec![], home: Some("/home/maria".into()), users: vec!["maria".into()] };
        let out = scrub("venv em /home/maria/hangar/.venv\noutro /srv/maria/x e /home/mariana/y\n\
            https://pc-da-maria.tail1234.ts.net:8443\nip 100.64.0.2 e 127.0.0.1 e 0.0.0.0:8765 e fe80::1c2b:3a4d e ::1\n\
            versão 1.2.3 e 10.0.19045.1 e 12:34:56 e app::doctor\n", &s);
        assert!(out.contains("venv em ~/hangar/.venv"), "{out}");
        assert!(out.contains("/srv/<usuario>/x"), "{out}");
        // Nome inteiro, nunca pedaço: "/home/mariana" é outra pessoa.
        assert!(out.contains("/home/mariana/y"), "{out}");
        assert!(out.contains("https://<maquina>.ts.net:8443"), "{out}");
        assert!(out.contains("ip <ip> e 127.0.0.1 e 0.0.0.0:8765 e <ip> e ::1"), "{out}");
        assert!(out.contains("versão 1.2.3 e 10.0.19045.1 e 12:34:56 e app::doctor"), "{out}");
    }

    #[test]
    fn scrub_handles_windows_paths_in_any_case() {
        let s = Secrets { values: vec![], home: Some(r"C:\Users\Maria".into()), users: vec!["Maria".into()] };
        let out = scrub(r"C:\Users\Maria\hangar e c:/users/maria/x e D:\Dados\maria\y", &s);
        assert_eq!(out, r"~\hangar e ~/x e D:\Dados\<usuario>\y");
    }

    fn facts(log: String) -> Facts {
        Facts { step: Screen::Install, code: Some("sem-systemd".into()), text: "este Linux não tem como iniciar o Hangar sozinho".into(),
            system: "Ubuntu 26.04 (linux-x86_64)".into(), app: "0.20.1.3456 · abc123".into(), doctor: None,
            logs: vec![("install".into(), log)] }
    }

    #[test]
    fn compose_has_step_code_system_app_doctor_and_log() {
        let out = compose(&facts("linha 1\n##HANGAR-ERRO## sem-systemd\nabra ?token=abcdef0123456789token\n".into()), &secrets());
        for needle in ["instalar", "sem-systemd", "Ubuntu 26.04", "0.20.1.3456 · abc123", "linha 1", tr("setup_report_doctor_missing").as_str()] {
            assert!(out.contains(needle), "{needle} faltou: {out}");
        }
        assert!(!out.contains("abcdef0123456789token"));
    }

    #[test]
    fn compose_keeps_the_end_of_a_huge_log_under_the_limit() {
        let log: String = (0..200_000).map(|n| format!("linha {n}\n")).collect();
        let out = compose(&facts(log), &secrets());
        assert!(out.len() <= BASE_MAX, "{}", out.len());
        assert!(out.contains("linha 199999"));
        assert!(!out.contains("linha 0\n"));
        assert!(out.contains("sem-systemd"));
    }

    #[test]
    fn agent_section_is_scrubbed_capped_and_keeps_the_recheck_first() {
        let section = AgentSection { agent: "Claude Code".into(), commands: vec!["cat backend/.env".into()],
            explanation: "o token era abcdef0123456789token".into(), diff: "+x\n".repeat(40_000), notes: vec![],
            recheck: Recheck::Failed("sem-systemd".into()) };
        let out = with_agent("base", &section, &secrets());
        assert!(out.starts_with("base"));
        assert!(out.len() <= "base".len() + AGENT_MAX + 64);
        assert!(!out.contains("abcdef0123456789token"));
        assert!(out.contains("$ cat backend/.env"));
        assert!(out.find("sem-systemd") < out.find("+x"));
    }

    #[test]
    fn payload_carries_the_worker_contract() {
        let p = payload(Screen::Install, Some("sem-systemd".into()), Outcome::Aberto, Some("claude"), "texto".into());
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["v"], 1);
        assert_eq!(v["step"], "instalar");
        assert_eq!(v["code"], "sem-systemd");
        assert_eq!(v["outcome"], "aberto");
        assert_eq!(v["agent"], "claude");
        assert!(v["os"].as_str().unwrap().starts_with(std::env::consts::OS));
        // Código fora do formato do Worker (`[a-z0-9-]`) vai como nulo: o texto do relatório já o leva.
        let odd = payload(Screen::Install, Some("Disco Cheio!".into()), Outcome::Consertado, None, "t".into());
        assert_eq!(serde_json::to_value(&odd).unwrap()["code"], serde_json::Value::Null);
        assert_eq!(serde_json::to_value(&odd).unwrap()["outcome"], "consertado");
        assert!(stamp().starts_with("hangar-native/"));
    }

    #[test]
    fn send_error_names_the_cause() {
        // Porta 9 sem ninguém: o envio falha com o motivo, nunca em silêncio.
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let p = payload(Screen::Install, None, Outcome::Aberto, None, "t".into());
        let error = runtime.block_on(send_to("http://127.0.0.1:9/api/relatorio", p)).unwrap_err();
        assert!(!error.is_empty());
    }
}
