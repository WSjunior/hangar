//! Mede a coleta sem índice e os relatórios num processo avulso, sem servidor.

use hangar_server::costs::{
    collect::{CollectError, Collector, Ready, ScopeSource, Scopes},
    index::IndexError,
    origins::Origins,
    py::LocalTs,
    report_costs,
    report_uso::{self, UsoFilters},
};
use serde::Serialize;
use std::{env, fs, io::{self, Write}, path::PathBuf, sync::Arc, time::{Duration, Instant}};

const HELP: &str = "Uso: custos --scopes <json> --index <pasta vazia> --now <iso>\n\
    [--period all|1d|7d|30d|90d] [--home <pasta>] [--pricing <pasta>] [--areas <json>]\n\
    [--conta <valor>] [--projeto <valor>] [--modelo <valor>] [--plugin <valor>]\n\
    [--foco <valor>] [--timeout <segundos>] [--profile] [--incremental]\n\
    Os filtros de lista podem ser repetidos. O índice deve ser descartável.\n";

struct Args {
    scopes: PathBuf,
    index: PathBuf,
    now: LocalTs,
    period: String,
    home: PathBuf,
    pricing: PathBuf,
    areas: PathBuf,
    filters: UsoFilters,
    timeout: Duration,
    profile: bool,
    origins: Option<PathBuf>,
    incremental: bool,
}

fn arguments() -> Result<Option<Args>, &'static str> {
    let mut args = env::args().skip(1);
    let (mut scopes, mut index, mut now) = (None, None, None);
    let (mut home, mut pricing, mut areas) = (None, None, None);
    let mut period = "all".to_owned();
    let mut filters = UsoFilters::default();
    let mut timeout = Duration::from_secs(300);
    let mut profile = false;
    let mut origin_file = None;
    let mut incremental = false;
    while let Some(flag) = args.next() {
        if flag == "--help" || flag == "-h" { return Ok(None); }
        if flag == "--profile" { profile = true; continue; }
        if flag == "--incremental" { incremental = true; continue; }
        let value = args.next().ok_or("argument_value_missing")?;
        match flag.as_str() {
            "--scopes" => { if scopes.replace(PathBuf::from(value)).is_some() { return Err("argument_duplicate"); } }
            "--index" => { if index.replace(PathBuf::from(value)).is_some() { return Err("argument_duplicate"); } }
            "--now" => {
                let parsed = LocalTs::from_iso(&value).ok_or("now_invalid")?;
                if now.replace(parsed).is_some() { return Err("argument_duplicate"); }
            }
            "--period" => period = value,
            "--home" => home = Some(PathBuf::from(value)),
            "--pricing" => pricing = Some(PathBuf::from(value)),
            "--areas" => areas = Some(PathBuf::from(value)),
            "--origins" => origin_file = Some(PathBuf::from(value)),
            "--conta" => filters.conta.push(value),
            "--projeto" => filters.projeto.push(value),
            "--modelo" => filters.modelo.push(value),
            "--plugin" => filters.plugin.push(value),
            "--foco" => filters.foco = Some(value),
            "--timeout" => {
                let seconds = value.parse::<u64>().ok().filter(|s| *s > 0).ok_or("timeout_invalid")?;
                timeout = Duration::from_secs(seconds);
            }
            _ => return Err("argument_unknown"),
        }
    }
    if !["all", "1d", "7d", "30d", "90d"].contains(&period.as_str()) { return Err("period_invalid"); }
    let home = home.or_else(|| env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")).map(PathBuf::from))
        .ok_or("home_missing")?;
    Ok(Some(Args {
        scopes: scopes.ok_or("scopes_missing")?, index: index.ok_or("index_missing")?,
        now: now.ok_or("now_missing")?, period,
        pricing: pricing.unwrap_or_else(|| home.join(".claude/.hangar-pricing")),
        areas: areas.unwrap_or_else(|| home.join(".hangar/uso-areas.json")),
        home, filters, timeout, profile, origins: origin_file, incremental,
    }))
}

struct JsonScopes(Scopes);
impl ScopeSource for JsonScopes {
    fn fetch(&self) -> Result<Scopes, CollectError> { Ok(self.0.clone()) }
}

fn collect_code(error: CollectError) -> &'static str {
    match error {
        CollectError::NoScopes => "scan_scopes_unavailable",
        CollectError::Index(IndexError::NoDisk) => "scan_disk_unavailable",
        CollectError::Index(IndexError::ReaderPanic) => "scan_reader_panic",
        CollectError::Index(IndexError::Sqlite(error)) => {
            match error {
                rusqlite::Error::InvalidColumnType(index, _, _) => eprintln!("diagnostic: sqlite_column_type index={index}"),
                rusqlite::Error::FromSqlConversionFailure(index, _, _) => eprintln!("diagnostic: sqlite_conversion index={index}"),
                rusqlite::Error::IntegralValueOutOfRange(index, _) => eprintln!("diagnostic: sqlite_integer_range index={index}"),
                rusqlite::Error::InvalidQuery => eprintln!("diagnostic: sqlite_invalid_query"),
                _ => eprintln!("diagnostic: sqlite_other"),
            }
            "scan_sqlite_failed"
        },
    }
}

fn peak_rss_mb() -> Result<u64, &'static str> {
    if !cfg!(target_os = "linux") { return Ok(0); }
    let status = fs::read_to_string("/proc/self/status").map_err(|_| "memory_unavailable")?;
    let kib = status.lines().find_map(|line| line.strip_prefix("VmHWM:"))
        .and_then(|line| line.split_whitespace().next()).and_then(|value| value.parse::<u64>().ok())
        .ok_or("memory_unavailable")?;
    // Arredondar para cima evita aprovar um pico acima do limite por truncamento.
    Ok(kib.div_ceil(1024))
}

#[derive(Serialize)]
struct Phase {
    phase: &'static str,
    elapsed_s: f64,
    rss_mb: u64,
    peak_rss_mb: u64,
    rows: usize,
    usage_rows: usize,
    token_rows: usize,
}

#[derive(Serialize)]
struct OriginCheck {
    expected: usize,
    actual: usize,
    missing: usize,
    extra: usize,
    different: usize,
    order_matches: bool,
}

#[derive(Serialize)]
struct IncrementalCheck {
    scan_s: f64,
    files: usize,
    reports_equal: bool,
    peak_rss_mb: u64,
}

fn rebuild_reports(collector: &Arc<Collector>, args: &Args,
                   origins: &indexmap::IndexMap<String, String>) -> Result<(Vec<u8>, Vec<u8>), &'static str> {
    let rows = collector.read_costs(None).map_err(collect_code)?;
    let builder = collector.fold_usage(None,
        |tokens, pricing| report_uso::UsoBuilder::new(tokens, &args.period, args.now, &args.filters, Some(origins), pricing),
        &mut |builder: &mut report_uso::UsoBuilder, row, account, pricing| builder.push(row, account, pricing),
    ).map_err(collect_code)?;
    let pricing = collector.pricing();
    let label = |key: &str| collector.label(key);
    let costs = report_costs::build(rows, &args.period, args.now, &pricing, &label);
    drop(pricing);
    let uso = builder.finish(&label);
    Ok((serde_json::to_vec(&costs).map_err(|_| "report_serialization_failed")?,
        serde_json::to_vec(&uso).map_err(|_| "report_serialization_failed")?))
}

fn record_phase(phases: &mut Vec<Phase>, phase: &'static str, started: Instant, rows: usize) -> Result<(), &'static str> {
    let rss_mb = if cfg!(target_os = "linux") {
        let status = fs::read_to_string("/proc/self/status").map_err(|_| "memory_unavailable")?;
        status.lines().find_map(|line| line.strip_prefix("VmRSS:"))
            .and_then(|line| line.split_whitespace().next()).and_then(|value| value.parse::<u64>().ok())
            .ok_or("memory_unavailable")?.div_ceil(1024)
    } else { 0 };
    phases.push(Phase { phase, elapsed_s: started.elapsed().as_secs_f64(), rss_mb, peak_rss_mb: peak_rss_mb()?, rows,
                        usage_rows: 0, token_rows: 0 });
    eprintln!("phase: {}", serde_json::to_string(phases.last().unwrap()).map_err(|_| "phase_serialization_failed")?);
    Ok(())
}

fn run(args: Args) -> Result<(), &'static str> {
    if args.index.exists() {
        let mut entries = fs::read_dir(&args.index).map_err(|_| "index_not_empty_directory")?;
        if entries.next().is_some() { return Err("index_not_empty_directory"); }
    }
    let bytes = fs::read(&args.scopes).map_err(|_| "scopes_unreadable")?;
    let scopes: Scopes = serde_json::from_slice(&bytes).map_err(|_| "scopes_invalid")?;
    let origins = Origins::new(args.home.clone(), scopes.repo.clone());
    let collector = Arc::new(Collector::new(args.index.clone(), args.pricing.clone(), args.areas.clone(), Arc::new(JsonScopes(scopes))));
    let started = Instant::now();
    let mut first = true;
    let mut files = 0;
    let mut phases = Vec::new();
    loop {
        if started.elapsed() >= args.timeout { return Err("scan_timeout"); }
        // Só a primeira entrada pede coleta: uma conclusão entre polls não deve iniciar outra.
        let ready = collector.prepare_blocking(first).map_err(collect_code)?;
        first = false;
        match ready {
            Ready::Go => break,
            Ready::Warming { total, .. } => { files = total; std::thread::sleep(Duration::from_millis(10)); },
        }
    }
    let scan_s = started.elapsed().as_secs_f64();
    if args.profile { record_phase(&mut phases, "after_prepare", started, files)?; }
    // As leituras também usam a trava de preços: adquiri-la antes causaria deadlock.
    let costs_rows = collector.read_costs(None).map_err(|_| "costs_read_failed")?;
    if args.profile { record_phase(&mut phases, "after_read_costs", started, costs_rows.len())?; }
    let origin_snapshot = origins.recent().1;
    let origin_check = if let Some(path) = &args.origins {
        let bytes = fs::read(path).map_err(|_| "origins_unreadable")?;
        let expected: indexmap::IndexMap<String, String> = serde_json::from_slice(&bytes).map_err(|_| "origins_invalid")?;
        Some(OriginCheck {
            expected: expected.len(), actual: origin_snapshot.len(),
            missing: expected.keys().filter(|key| !origin_snapshot.contains_key(*key)).count(),
            extra: origin_snapshot.keys().filter(|key| !expected.contains_key(*key)).count(),
            different: expected.iter().filter(|(key, value)| origin_snapshot.get(*key).is_some_and(|actual| actual != *value)).count(),
            order_matches: expected.keys().eq(origin_snapshot.keys()),
        })
    } else { None };
    if args.profile { record_phase(&mut phases, "after_origins", started, origin_snapshot.len())?; }
    // O uso é montado enquanto o índice entrega as linhas, como na rota.
    let (usage_rows, token_rows, builder) = collector.fold_usage(None,
        |tokens, pricing| (0usize, tokens.len(), report_uso::UsoBuilder::new(tokens, &args.period, args.now, &args.filters, Some(&origin_snapshot), pricing)),
        &mut |(count, _, builder): &mut (usize, usize, report_uso::UsoBuilder), row, account, pricing| { *count += 1; builder.push(row, account, pricing); },
    ).map_err(collect_code)?;
    if args.profile {
        record_phase(&mut phases, "after_read_usage", started, usage_rows + token_rows)?;
        let phase = phases.last_mut().unwrap();
        phase.usage_rows = usage_rows; phase.token_rows = token_rows;
    }
    let pricing = collector.pricing();
    let label = |key: &str| collector.label(key);
    let costs = report_costs::build(costs_rows, &args.period, args.now, &pricing, &label);
    if args.profile { record_phase(&mut phases, "after_build_costs", started, costs.combos.len())?; }
    drop(pricing);
    let uso = builder.finish(&label);
    if args.profile { record_phase(&mut phases, "after_build_usage", started, uso.by_tool.len())?; }
    // Inclui a serialização e os dois relatórios no pico, além da varredura.
    let costs_json = serde_json::to_vec(&costs).map_err(|_| "report_serialization_failed")?;
    let uso_json = serde_json::to_vec(&uso).map_err(|_| "report_serialization_failed")?;
    if args.profile { record_phase(&mut phases, "after_serialize", started, costs_json.len() + uso_json.len())?; }
    let incremental_check = if args.incremental {
        let warm_started = Instant::now();
        // Um único pedido fresco, seguido só por consultas, mede o índice já preenchido.
        let mut ready = collector.prepare_blocking(true).map_err(collect_code)?;
        loop {
            match ready {
                Ready::Go => break,
                Ready::Warming { total, .. } => {
                    files = total;
                    if warm_started.elapsed() >= args.timeout { return Err("incremental_timeout"); }
                    std::thread::sleep(Duration::from_millis(10));
                    ready = collector.prepare_blocking(false).map_err(collect_code)?;
                }
            }
        }
        let warm_scan_s = warm_started.elapsed().as_secs_f64();
        if args.profile { record_phase(&mut phases, "after_incremental", started, files)?; }
        let (warm_costs, warm_uso) = rebuild_reports(&collector, &args, &origin_snapshot)?;
        let equal = warm_costs == costs_json && warm_uso == uso_json;
        if args.profile { record_phase(&mut phases, "after_incremental_reports", started, warm_costs.len() + warm_uso.len())?; }
        Some(IncrementalCheck { scan_s: warm_scan_s, files, reports_equal: equal, peak_rss_mb: peak_rss_mb()? })
    } else { None };
    let peak = peak_rss_mb()?;
    let phase_json = if args.profile { Some(serde_json::to_vec(&phases).map_err(|_| "report_serialization_failed")?) } else { None };
    let origin_json = origin_check.map(|check| serde_json::to_vec(&check)).transpose().map_err(|_| "report_serialization_failed")?;
    let incremental_json = incremental_check.map(|check| serde_json::to_vec(&check)).transpose().map_err(|_| "report_serialization_failed")?;
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    stdout.write_all(b"{\"costs\":").and_then(|_| stdout.write_all(&costs_json))
        .and_then(|_| stdout.write_all(b",\"uso\":"))
        .and_then(|_| stdout.write_all(&uso_json))
        .and_then(|_| write!(stdout, ",\"scan_s\":{scan_s},\"peak_rss_mb\":{peak}"))
        .and_then(|_| {
            if let Some(phases) = phase_json { stdout.write_all(b",\"phases\":").and_then(|_| stdout.write_all(&phases)) }
            else { Ok(()) }
        })
        .and_then(|_| {
            if let Some(check) = origin_json { stdout.write_all(b",\"origins_check\":").and_then(|_| stdout.write_all(&check)) }
            else { Ok(()) }
        })
        .and_then(|_| {
            if let Some(check) = incremental_json { stdout.write_all(b",\"incremental\":").and_then(|_| stdout.write_all(&check)) }
            else { Ok(()) }
        })
        .and_then(|_| stdout.write_all(b"}\n"))
        .map_err(|_| "output_failed")
}

fn main() {
    // Segurança: o ajuste ocorre antes da criação de qualquer thread.
    unsafe { hangar_server::tune_allocator(); }
    let result = match arguments() {
        Ok(Some(args)) => run(args),
        Ok(None) => { print!("{HELP}"); return; }
        Err(code) => Err(code),
    };
    if let Err(code) = result {
        eprintln!("erro: {code}");
        std::process::exit(2);
    }
}
