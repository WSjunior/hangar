//! Agregação pura dos custos; a coleta e a cotação têm ciclos próprios.

use crate::costs::pricing::{self, Pricing, Rate};
use crate::costs::py::LocalTs;
use crate::costs::rows::UsageRow;
use crate::transcript::pyjson;
use indexmap::{IndexMap, IndexSet};
use serde::Serialize;
use serde_json::json;
use std::cmp::Ordering;

pub const PERIODS: [(&str, i64); 4] = [("1d", 1), ("7d", 7), ("30d", 30), ("90d", 90)];
const KINDS: [&str; 4] = ["input", "output", "cache_write", "cache_read"];
const DAY_MICROS: i64 = 86_400_000_000;

#[derive(Clone, Debug, Default, Serialize)]
pub struct DimBucket {
    pub key: String,
    pub label: Option<String>,
    pub sessions: i64,
    pub input: i64,
    pub output: i64,
    pub cache_write: i64,
    pub cache_read: i64,
    pub cost: f64,
    pub cost_input: f64,
    pub cost_output: f64,
    pub cost_cache_write: f64,
    pub cost_cache_read: f64,
    pub cache_write_1h: i64,
    pub regravado: i64,
    pub custo_regravado: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct KindBucket {
    pub kind: String,
    pub tokens: i64,
    pub cost: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct RateInfo {
    pub model: String,
    pub provider: String,
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub origin: String,
    pub cache_estimado: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ComboRow {
    pub dia: String,
    pub provider: String,
    pub source: String,
    pub project: String,
    pub model: String,
    pub subagente: bool,
    pub sessions: i64,
    pub session_ids: Vec<String>,
    pub custo_sem_cache: f64,
    pub equivalente_cobrado: f64,
    pub input: i64,
    pub output: i64,
    pub cache_write: i64,
    pub cache_read: i64,
    pub cost: f64,
    pub cost_input: f64,
    pub cost_output: f64,
    pub cost_cache_write: f64,
    pub cost_cache_read: f64,
    pub cache_write_1h: i64,
    pub regravado: i64,
    pub custo_regravado: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessaoCusto {
    pub session_id: String,
    pub source: String,
    pub provider: String,
    pub project: String,
    pub model: String,
    pub inicio: String,
    pub fim: String,
    pub subagentes: i64,
    pub input: i64,
    pub output: i64,
    pub cache_write: i64,
    pub cache_read: i64,
    pub cost: f64,
    pub custo_regravado: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Applied {
    pub period: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CostReport {
    pub totals: DimBucket,
    pub by_day: Vec<DimBucket>,
    pub by_provider: Vec<DimBucket>,
    pub by_source: Vec<DimBucket>,
    pub by_project: Vec<DimBucket>,
    pub by_model: Vec<DimBucket>,
    pub by_kind: Vec<KindBucket>,
    pub rates: Vec<RateInfo>,
    pub sem_tarifa: Vec<String>,
    pub custo_sem_cache: f64,
    pub equivalente_cobrado: i64,
    pub anterior: Option<DimBucket>,
    pub applied: Option<Applied>,
    pub usd_brl: Option<f64>,
    pub combos: Vec<ComboRow>,
    pub sessoes: Vec<SessaoCusto>,
}

fn adjusted(row: &UsageRow, pricing: &Pricing, base: &Rate) -> Rate {
    let rate = if row.source == "codex" {
        pricing.rate_codex(base, &row.model, row.codex_long_context)
    } else {
        base.clone()
    };
    if row.fast { pricing.rate_fast(&rate, &row.model) } else { rate }
}

pub fn row_cost(row: &UsageRow, pricing: &Pricing) -> Option<[f64; 4]> {
    let rate = adjusted(row, pricing, &pricing.rate_for(&row.model)?);
    let mut cost = pricing::custo(&rate, row.input, row.output, row.cache_write, row.cache_read);
    if rate.provider == "anthropic" && rate.origin != "override" {
        cost[2] += row.cache_write_1h as f64 / 1e6 * (rate.input * 2.0 - rate.cache_write);
    }
    Some(cost)
}

fn rewrite_cost(row: &UsageRow, pricing: &Pricing) -> f64 {
    if row.regravado == 0 { return 0.0; }
    let Some(base) = pricing.rate_for(&row.model) else { return 0.0 };
    let rate = adjusted(row, pricing, &base);
    let mut extra = row.regravado as f64 / 1e6 * (rate.cache_write - rate.cache_read);
    if rate.provider == "anthropic" && rate.origin != "override" {
        extra += row.regravado_1h as f64 / 1e6 * (rate.input * 2.0 - rate.cache_write);
    }
    extra
}

fn tokens(row: &UsageRow) -> [i64; 4] {
    [row.input, row.output, row.cache_write, row.cache_read]
}

pub(crate) fn cost_sum(values: [f64; 4]) -> f64 {
    // sum() compensa os tipos dentro da linha; os totais entre linhas usam +=.
    let mut high: f64 = 0.0;
    let mut low = 0.0;
    for value in values {
        let next = high + value;
        low += if high.abs() >= value.abs() { (high - next) + value } else { (value - next) + high };
        high = next;
    }
    if low != 0.0 && low.is_finite() { high + low } else { high }
}

fn identity(row: &UsageRow) -> String {
    let account = row.account_id.as_deref().filter(|a| !a.is_empty()).unwrap_or(&row.provider);
    pyjson::dumps(&json!([row.source, account, row.session_id, row.subagente]), false)
}

#[derive(Default)]
struct Bucket {
    dim: DimBucket,
    session_ids: IndexSet<String>,
    full_cost: f64,
    charged_equivalent: f64,
}

impl Bucket {
    fn add(&mut self, row: &UsageRow, cost: Option<[f64; 4]>, pricing: &Pricing) {
        self.session_ids.insert(identity(row));
        let b = &mut self.dim;
        b.sessions = self.session_ids.len() as i64;
        b.input += row.input;
        b.output += row.output;
        b.cache_write += row.cache_write;
        b.cache_read += row.cache_read;
        b.cache_write_1h += row.cache_write_1h;
        b.regravado += row.regravado;
        b.custo_regravado += rewrite_cost(row, pricing);
        if let Some(cost) = cost {
            b.cost_input += cost[0];
            b.cost_output += cost[1];
            b.cost_cache_write += cost[2];
            b.cost_cache_read += cost[3];
            b.cost += cost_sum(cost);
        }
    }
}

fn descending_cost(a: f64, b: f64) -> Ordering {
    b.partial_cmp(&a).unwrap_or(Ordering::Equal)
}

fn group(rows: &[UsageRow], costs: &[Option<[f64; 4]>], pricing: &Pricing,
         key: impl Fn(&UsageRow) -> String, label: Option<&dyn Fn(&str) -> Option<String>>) -> Vec<DimBucket> {
    let mut buckets: IndexMap<String, Bucket> = IndexMap::new();
    for (row, cost) in rows.iter().zip(costs) {
        buckets.entry(key(row)).or_default().add(row, *cost, pricing);
    }
    let mut result: Vec<_> = buckets.into_iter().map(|(key, mut bucket)| {
        bucket.dim.label = label.and_then(|f| f(&key));
        bucket.dim.key = key;
        bucket.dim
    }).collect();
    result.sort_by(|a, b| descending_cost(a.cost, b.cost).then_with(|| a.key.cmp(&b.key)));
    result
}

fn days(period: &str) -> Option<i64> {
    PERIODS.iter().find(|(key, _)| *key == period).map(|(_, n)| *n)
}

fn earlier_day(now: LocalTs, days: i64) -> String {
    LocalTs(now.0 - days * DAY_MICROS).day()
}

pub fn since(period: &str, now: LocalTs) -> Option<String> {
    days(period).map(|n| earlier_day(now, n * 2 - 1))
}

fn previous_window(rows: &[UsageRow], days: i64, now: LocalTs, pricing: &Pricing) -> Option<DimBucket> {
    let start = earlier_day(now, days * 2 - 1);
    let end = earlier_day(now, days);
    let mut covered = IndexSet::new();
    let mut bucket = Bucket::default();
    for row in rows {
        let day = row.ts.day();
        if day >= start && day <= end {
            covered.insert(day);
            bucket.add(row, row_cost(row, pricing), pricing);
        }
    }
    if covered.is_empty() || (covered.len() as i64) * 3 < days { return None; }
    bucket.dim.key = "anterior".into();
    Some(bucket.dim)
}

struct Session {
    project: String,
    start: LocalTs,
    end: LocalTs,
    subagents: IndexSet<String>,
    models: IndexMap<String, f64>,
    tokens: [i64; 4],
    cost: f64,
    rewrite_cost: f64,
}

fn top_sessions(rows: &[UsageRow], costs: &[Option<[f64; 4]>], pricing: &Pricing) -> Vec<SessaoCusto> {
    let mut sessions: IndexMap<(String, String, String), Session> = IndexMap::new();
    for (row, cost) in rows.iter().zip(costs) {
        let parent = row.session_id.split("/subagents/").next().unwrap_or_default();
        let session = sessions.entry((row.source.clone(), row.provider.clone(), parent.to_owned()))
            .or_insert_with(|| Session {
                project: row.project.clone(), start: row.ts, end: row.ts,
                subagents: IndexSet::new(), models: IndexMap::new(), tokens: [0; 4],
                cost: 0.0, rewrite_cost: 0.0,
            });
        if row.subagente { session.subagents.insert(row.session_id.clone()); }
        else { session.project = row.project.clone(); }
        session.start = session.start.min(row.ts);
        session.end = session.end.max(row.ts);
        let cost = cost.map_or(0.0, cost_sum);
        *session.models.entry(pricing.canonizar(&row.model)).or_default() += cost;
        for (total, tokens) in session.tokens.iter_mut().zip(tokens(row)) { *total += tokens; }
        session.cost += cost;
        session.rewrite_cost += rewrite_cost(row, pricing);
    }
    let mut sessions: Vec<_> = sessions.into_iter().collect();
    sessions.sort_by(|a, b| descending_cost(a.1.cost, b.1.cost));
    sessions.into_iter().take(100).map(|((source, provider, session_id), session)| {
        // max_by escolheria o último empate; o Python conserva o primeiro modelo.
        let mut winner: Option<(&String, f64)> = None;
        for (model, cost) in &session.models {
            if winner.is_none_or(|(_, best)| *cost > best) { winner = Some((model, *cost)); }
        }
        SessaoCusto {
            session_id, source, provider, project: session.project,
            model: winner.map(|(model, _)| model.clone()).unwrap_or_default(),
            inicio: session.start.day(), fim: session.end.day(), subagentes: session.subagents.len() as i64,
            input: session.tokens[0], output: session.tokens[1], cache_write: session.tokens[2],
            cache_read: session.tokens[3], cost: session.cost, custo_regravado: session.rewrite_cost,
        }
    }).collect()
}

pub fn build(mut rows: Vec<UsageRow>, period: &str, now: LocalTs, pricing: &Pricing,
             label: &dyn Fn(&str) -> Option<String>) -> CostReport {
    let previous = days(period).and_then(|n| previous_window(&rows, n, now, pricing));
    if let Some(n) = days(period) {
        let cutoff = earlier_day(now, n - 1);
        rows.retain(|row| row.ts.day() >= cutoff);
    }
    // Os índices dos custos precisam corresponder à lista já cortada.
    let costs: Vec<_> = rows.iter().map(|row| row_cost(row, pricing)).collect();
    let mut total = Bucket::default();
    let mut kinds = KINDS.map(|kind| KindBucket { kind: kind.into(), tokens: 0, cost: 0.0 });
    let mut full_cost = 0.0;
    let mut charged_equivalent = 0.0;
    let mut missing_rates = IndexSet::new();
    let mut rates = IndexMap::new();
    type ComboKey = (String, String, String, String, String, bool);
    let mut combos: IndexMap<ComboKey, Bucket> = IndexMap::new();
    for (row, cost) in rows.iter().zip(&costs) {
        total.add(row, *cost, pricing);
        let canonical = pricing.canonizar(&row.model);
        let combo = combos.entry((row.ts.day(), row.provider.clone(), row.source.clone(),
            row.project.clone(), canonical.clone(), row.subagente)).or_default();
        combo.add(row, *cost, pricing);
        for (i, count) in tokens(row).into_iter().enumerate() {
            kinds[i].tokens += count;
            if let Some(cost) = cost { kinds[i].cost += cost[i]; }
        }
        let Some(rate) = pricing.rate_for(&row.model) else {
            if !pricing::IGNORADOS.contains(&canonical.as_str()) { missing_rates.insert(canonical); }
            continue;
        };
        let effective = adjusted(row, pricing, &rate);
        let full = (row.input + row.cache_write + row.cache_read) as f64 / 1e6 * effective.input
            + row.output as f64 / 1e6 * effective.output;
        full_cost += full;
        combo.full_cost += full;
        if effective.input != 0.0 {
            let mut weight = row.input as f64
                + row.output as f64 * (effective.output / effective.input)
                + row.cache_write as f64 * (effective.cache_write / effective.input)
                + row.cache_read as f64 * (effective.cache_read / effective.input);
            if effective.provider == "anthropic" && effective.origin != "override" {
                weight += row.cache_write_1h as f64 * (2.0 - effective.cache_write / effective.input);
            }
            charged_equivalent += weight;
            combo.charged_equivalent += weight;
        }
        rates.entry(canonical.clone()).or_insert_with(|| RateInfo {
            model: canonical, provider: rate.provider, input: rate.input, output: rate.output,
            cache_read: rate.cache_read, cache_write: rate.cache_write,
            origin: rate.origin, cache_estimado: rate.cache_estimado,
        });
    }
    total.dim.key = "totals".into();
    let mut by_day = group(&rows, &costs, pricing, |row| row.ts.day(), None);
    by_day.sort_by(|a, b| b.key.cmp(&a.key));
    let mut rates: Vec<_> = rates.into_values().collect();
    rates.sort_by(|a, b| a.model.cmp(&b.model));
    let mut missing_rates: Vec<_> = missing_rates.into_iter().collect();
    missing_rates.sort();
    let mut combos: Vec<_> = combos.into_iter().collect();
    combos.sort_by(|a, b| a.0.0.cmp(&b.0.0).then_with(|| descending_cost(a.1.dim.cost, b.1.dim.cost)));
    let combos = combos.into_iter().map(|((dia, provider, source, project, model, subagente), bucket)| {
        let mut session_ids: Vec<_> = bucket.session_ids.into_iter().collect();
        session_ids.sort();
        let b = bucket.dim;
        ComboRow {
            dia, provider, source, project, model, subagente, sessions: b.sessions, session_ids,
            custo_sem_cache: bucket.full_cost, equivalente_cobrado: bucket.charged_equivalent,
            input: b.input, output: b.output, cache_write: b.cache_write, cache_read: b.cache_read,
            cost: b.cost, cost_input: b.cost_input, cost_output: b.cost_output,
            cost_cache_write: b.cost_cache_write, cost_cache_read: b.cost_cache_read,
            cache_write_1h: b.cache_write_1h, regravado: b.regravado, custo_regravado: b.custo_regravado,
        }
    }).collect();
    CostReport {
        totals: total.dim, by_day,
        by_provider: group(&rows, &costs, pricing, |row| row.provider.clone(), Some(label)),
        by_source: group(&rows, &costs, pricing, |row| row.source.clone(), None),
        by_project: group(&rows, &costs, pricing, |row| row.project.clone(), None),
        by_model: group(&rows, &costs, pricing, |row| pricing.canonizar(&row.model), None),
        by_kind: kinds.into(), rates, sem_tarifa: missing_rates, custo_sem_cache: full_cost,
        equivalente_cobrado: charged_equivalent as i64, anterior: previous,
        applied: Some(Applied { period: period.into() }), usd_brl: None,
        combos, sessoes: top_sessions(&rows, &costs, pricing),
    }
}

/// Visão da tela inicial (`?view=summary`): só o que os três clientes leem ali, com os mesmos
/// valores do relatório inteiro (mesmas funções, mesma ordem de soma). Sem `combos`, que
/// sozinho era quase todo o megabyte da resposta.
#[derive(Clone, Debug, Serialize)]
pub struct SummaryReport {
    pub totals: DimBucket,
    pub by_day: Vec<DimBucket>,
    pub by_model: Vec<DimBucket>,
    pub sem_tarifa: Vec<String>,
    pub applied: Option<Applied>,
    pub usd_brl: Option<f64>,
}

pub fn build_summary(mut rows: Vec<UsageRow>, period: &str, now: LocalTs, pricing: &Pricing) -> SummaryReport {
    if let Some(n) = days(period) {
        let cutoff = earlier_day(now, n - 1);
        rows.retain(|row| row.ts.day() >= cutoff);
    }
    let costs: Vec<_> = rows.iter().map(|row| row_cost(row, pricing)).collect();
    let mut total = Bucket::default();
    let mut missing_rates = IndexSet::new();
    for (row, cost) in rows.iter().zip(&costs) {
        total.add(row, *cost, pricing);
        if pricing.rate_for(&row.model).is_none() {
            let canonical = pricing.canonizar(&row.model);
            if !pricing::IGNORADOS.contains(&canonical.as_str()) { missing_rates.insert(canonical); }
        }
    }
    total.dim.key = "totals".into();
    let mut by_day = group(&rows, &costs, pricing, |row| row.ts.day(), None);
    by_day.sort_by(|a, b| b.key.cmp(&a.key));
    let mut missing_rates: Vec<_> = missing_rates.into_iter().collect();
    missing_rates.sort();
    SummaryReport {
        totals: total.dim, by_day,
        by_model: group(&rows, &costs, pricing, |row| pricing.canonizar(&row.model), None),
        sem_tarifa: missing_rates, applied: Some(Applied { period: period.into() }), usd_brl: None,
    }
}
