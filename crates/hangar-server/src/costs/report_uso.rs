//! Agregação pura do uso; tokens e custos reais não se confundem com as estimativas de contexto.

use crate::costs::{pricing::Pricing, py::LocalTs, report_costs::{self, Applied}, rows::{UsageRow, UsoLinha}};
use indexmap::{IndexMap, IndexSet};
use serde::Serialize;
use std::cmp::Ordering;

#[derive(Clone, Debug, Default, Serialize)]
pub struct UsoBucket {
    pub key: String,
    pub label: Option<String>,
    pub plugin: String,
    pub sessions: i64,
    pub subagentes: i64,
    pub chamadas: i64,
    pub pedidas: i64,
    pub ctx_chars: i64,
    pub ctx_tokens_est: i64,
    pub input: i64,
    pub output: i64,
    pub cache_write: i64,
    pub cache_read: i64,
    pub cost: f64,
    pub cost_input: f64,
    pub cost_output: f64,
    pub cost_cache_write: f64,
    pub cost_cache_read: f64,
    pub ocupados_tokens_est: i64,
    pub ocupados_eq_tokens_est: i64,
    pub respostas: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct UsoReport {
    pub totals: UsoBucket,
    pub by_skill: Vec<UsoBucket>,
    pub by_tool: Vec<UsoBucket>,
    pub by_bash: Vec<UsoBucket>,
    pub by_mcp: Vec<UsoBucket>,
    pub by_agente: Vec<UsoBucket>,
    pub by_contexto: Vec<UsoBucket>,
    pub by_plugin: Vec<UsoBucket>,
    pub by_imagem: Vec<UsoBucket>,
    pub by_area: Vec<UsoBucket>,
    pub by_area_dia: Vec<UsoBucket>,
    pub by_conta: Vec<UsoBucket>,
    pub by_projeto: Vec<UsoBucket>,
    pub by_modelo: Vec<UsoBucket>,
    pub by_day: Vec<UsoBucket>,
    pub conta: Vec<String>,
    pub projeto: Vec<String>,
    pub modelo: Vec<String>,
    pub plugin: Vec<String>,
    pub foco: Option<String>,
    pub applied: Option<Applied>,
    pub usd_brl: Option<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct UsoFilters {
    pub conta: Vec<String>,
    pub projeto: Vec<String>,
    pub modelo: Vec<String>,
    pub plugin: Vec<String>,
    pub foco: Option<String>,
}

struct Bucket {
    output: UsoBucket,
    sessions: IndexSet<String>,
    subagents: IndexSet<String>,
    estimated: i64,
    occupied: i64,
    equivalent: i64,
    ruler: f64,
}

impl Default for Bucket {
    fn default() -> Self {
        Self { output: UsoBucket::default(), sessions: IndexSet::new(), subagents: IndexSet::new(),
            estimated: 0, occupied: 0, equivalent: 0, ruler: 4.0 }
    }
}

#[derive(Clone, Copy, Default)]
struct Share {
    total: f64,
    split: [f64; 4],
}

#[derive(Default)]
struct Agent {
    tokens: [i64; 4],
    cost: Share,
}

type Groups = IndexMap<String, Bucket>;
type Agents = IndexMap<String, Agent>;

fn usage_tokens(t: &UsoLinha) -> [i64; 4] { [t.input, t.output, t.cache_write, t.cache_read] }

fn sum_cost(values: [f64; 4]) -> f64 {
    // sum() compensa os componentes; += conserva a ordem original entre linhas.
    let mut high: f64 = 0.0;
    let mut low = 0.0;
    for value in values {
        let next = high + value;
        low += if high.abs() >= value.abs() { (high - next) + value } else { (value - next) + high };
        high = next;
    }
    if low != 0.0 && low.is_finite() { high + low } else { high }
}

fn real_cost(t: &UsoLinha, pricing: &Pricing) -> Share {
    if usage_tokens(t).iter().all(|n| *n == 0) { return Share::default(); }
    let row = UsageRow {
        ts: LocalTs::from_iso(&t.dia).expect("uso_dia_invalido"), source: t.fonte.clone(),
        provider: if t.fonte == "codex" { "openai" } else { "anthropic" }.into(),
        model: t.model.clone(), project: t.cwd.clone(), session_id: t.session_id.clone(),
        input: t.input, output: t.output, cache_write: t.cache_write, cache_read: t.cache_read,
        cache_write_1h: t.cache_write_1h, fast: t.fast, subagente: false,
        account_id: None, codex_long_context: false, regravado: 0, regravado_1h: 0,
    };
    let split = report_costs::row_cost(&row, pricing).unwrap_or([0.0; 4]);
    Share { total: sum_cost(split), split }
}

fn agent_costs<'a>(rows: impl Iterator<Item = &'a UsageRow>, pricing: &Pricing) -> Agents {
    let mut agents: Agents = IndexMap::new();
    for row in rows {
        if !row.session_id.contains("/subagents/agent-") { continue; }
        let id = row.session_id.rsplit_once("agent-").unwrap().1;
        let a = agents.entry(id.to_owned()).or_default();
        for (sum, value) in a.tokens.iter_mut().zip([row.input, row.output, row.cache_write, row.cache_read]) { *sum += value; }
        if let Some(split) = report_costs::row_cost(row, pricing) {
            a.cost.total += sum_cost(split);
            for (sum, value) in a.cost.split.iter_mut().zip(split) { *sum += value; }
        }
    }
    agents
}

fn row_share(t: &UsoLinha, cost: Share, agents: &Agents) -> Share {
    if t.tipo == "skill" { cost }
    else if t.tipo == "agente" && !t.detalhe.is_empty() { agents.get(&t.detalhe).map_or(Share::default(), |a| a.cost) }
    else { Share::default() }
}

impl Bucket {
    fn session(&mut self, t: &UsoLinha) {
        if t.subagente { self.subagents.insert(t.session_id.clone()); }
        else { self.sessions.insert(t.session_id.clone()); }
    }
    fn add_tokens(&mut self, values: [i64; 4]) {
        self.output.input += values[0]; self.output.output += values[1];
        self.output.cache_write += values[2]; self.output.cache_read += values[3];
    }
    fn add_cost(&mut self, share: Share) {
        self.output.cost += share.total;
        self.output.cost_input += share.split[0]; self.output.cost_output += share.split[1];
        self.output.cost_cache_write += share.split[2]; self.output.cost_cache_read += share.split[3];
    }
    fn add_agent(&mut self, t: &UsoLinha, agents: &Agents) {
        if let Some(agent) = agents.get(&t.detalhe).filter(|_| !t.detalhe.is_empty()) {
            self.add_tokens(agent.tokens); self.add_cost(agent.cost);
        }
    }
    fn summary(&mut self, t: &UsoLinha, share: Share, agents: &Agents, item: bool) {
        self.session(t);
        if t.tipo == "area" {
            if !item { self.add_tokens(usage_tokens(t)); }
            return;
        }
        let counted = if t.tipo == "tool" { !matches!(t.nome.as_str(), "Skill" | "Agent") }
            else { matches!(t.tipo.as_str(), "skill" | "agente" | "contexto" | "imagem") };
        if counted {
            if t.tipo != "contexto" { self.output.chamadas += t.chamadas; }
            self.output.ctx_chars += t.ctx_chars; self.estimated += t.tokens_est;
        }
        self.add_cost(share);
        if item {
            self.occupied += t.ocupados; self.equivalent += t.ocupados_eq; self.output.respostas += t.respostas;
            if t.tipo == "agente" && !t.detalhe.is_empty() {
                if let Some(agent) = agents.get(&t.detalhe) { self.add_tokens(agent.tokens); }
            } else if t.tipo == "skill" { self.add_tokens(usage_tokens(t)); }
        }
    }
    fn area(&mut self, t: &UsoLinha, cost: f64) {
        self.session(t); self.output.chamadas += t.chamadas; self.add_tokens(usage_tokens(t)); self.output.cost += cost;
    }
    fn finish(mut self, key: String) -> UsoBucket {
        self.output.key = key; self.output.sessions = self.sessions.len() as i64;
        self.output.subagentes = self.subagents.len() as i64;
        self.output.ctx_tokens_est = (self.output.ctx_chars as f64 / self.ruler) as i64 + self.estimated;
        self.output.ocupados_tokens_est = (self.occupied as f64 / 2.5) as i64;
        self.output.ocupados_eq_tokens_est = (self.equivalent as f64 / 2.5) as i64;
        self.output
    }
}

fn descending(a: f64, b: f64) -> Ordering { b.partial_cmp(&a).unwrap_or(Ordering::Equal) }
fn ranked(groups: Groups) -> Vec<UsoBucket> {
    let mut out: Vec<_> = groups.into_iter().map(|(k, v)| v.finish(k)).collect();
    out.sort_by(|a, b| b.ocupados_eq_tokens_est.cmp(&a.ocupados_eq_tokens_est)
        .then_with(|| b.ocupados_tokens_est.cmp(&a.ocupados_tokens_est))
        .then_with(|| descending(a.cost, b.cost)).then_with(|| b.ctx_chars.cmp(&a.ctx_chars))
        .then_with(|| b.chamadas.cmp(&a.chamadas)).then_with(|| a.key.cmp(&b.key)));
    out
}
fn dimension(groups: Groups, label: Option<&dyn Fn(&str) -> Option<String>>) -> Vec<UsoBucket> {
    let mut out: Vec<_> = groups.into_iter().map(|(k, v)| {
        let mut b = v.finish(k); b.label = label.and_then(|f| f(&b.key)); b
    }).collect();
    let count = |b: &UsoBucket| b.input + b.output + b.cache_write + b.cache_read;
    out.sort_by(|a, b| count(b).cmp(&count(a)).then_with(|| b.chamadas.cmp(&a.chamadas)).then_with(|| a.key.cmp(&b.key)));
    out
}
fn daily(groups: Groups) -> Vec<UsoBucket> {
    let mut out: Vec<_> = groups.into_iter().map(|(k, v)| v.finish(k)).collect();
    out.sort_by(|a, b| a.key.cmp(&b.key)); out
}
fn project(value: &str) -> &str { if value.is_empty() { "desconhecido" } else { value } }
fn prefix(value: &str) -> &str { value.split_once(':').map_or("", |(p, _)| p) }
fn matches_filter(values: &[String], value: &str) -> bool { values.is_empty() || values.iter().any(|v| v == value) }
fn filter_values(values: &[String]) -> Vec<String> { values.iter().filter(|s| !s.is_empty()).cloned().collect() }

pub fn build(uso: &[(UsoLinha, String)], tokens: &[UsageRow], period: &str, now: LocalTs,
             f: &UsoFilters, origins: Option<&IndexMap<String, String>>, pricing: &Pricing,
             label: &dyn Fn(&str) -> Option<String>) -> UsoReport {
    let accounts = filter_values(&f.conta); let projects = filter_values(&f.projeto);
    let models = filter_values(&f.modelo); let filters = filter_values(&f.plugin);
    let focus = f.foco.as_deref().filter(|s| !s.is_empty());
    let cutoff = report_costs::PERIODS.iter().find(|(p, _)| *p == period)
        .map(|(_, n)| LocalTs(now.0 - (n - 1) * 86_400_000_000).day());
    let in_period = |r: &&UsageRow| cutoff.as_ref().is_none_or(|c| r.ts.day() >= *c);
    let all_agents = agent_costs(tokens.iter().filter(in_period), pricing);
    let filtered_agents = if accounts.is_empty() && projects.is_empty() { None } else {
        Some(agent_costs(tokens.iter().filter(in_period).filter(|r|
            (accounts.is_empty() || r.account_id.as_ref().is_some_and(|a| accounts.contains(a)))
            && matches_filter(&projects, project(&r.project))), pricing))
    };
    let agents = filtered_agents.as_ref().unwrap_or(&all_agents);
    let mut by_account = Groups::new(); let mut by_project = Groups::new(); let mut by_model = Groups::new();
    let mut by_kind: IndexMap<String, Groups> = IndexMap::new(); let mut plugins = Groups::new();
    let mut by_day = Groups::new(); let mut by_area_day = Groups::new(); let mut total = Bucket::default();
    let mut series = Vec::new();
    for (t, account) in uso {
        if cutoff.as_ref().is_some_and(|c| t.dia.is_empty() || t.dia < *c) { continue; }
        let plugin = if !t.plugin.is_empty() { t.plugin.as_str() }
            else if t.tipo == "skill" && origins.is_some() { origins.unwrap().get(&t.nome).map_or("@embutida", String::as_str) }
            else if t.tipo == "agente" { prefix(&t.nome) } else { "" };
        let real = if matches!(t.tipo.as_str(), "skill" | "area") { real_cost(t, pricing) } else { Share::default() };
        let all_share = row_share(t, real, &all_agents);
        let row_project = project(&t.cwd);
        let canonical = pricing.canonizar(&t.model); let model = if canonical.is_empty() { "?" } else { &canonical };
        by_account.entry(account.clone()).or_default().summary(t, all_share, &all_agents, false);
        by_project.entry(row_project.into()).or_default().summary(t, all_share, &all_agents, false);
        by_model.entry(model.into()).or_default().summary(t, all_share, &all_agents, false);
        if !matches_filter(&accounts, account) || !matches_filter(&projects, row_project)
            || !matches_filter(&models, model) || !matches_filter(&filters, plugin) { continue; }
        let share = row_share(t, real, agents);
        total.summary(t, share, agents, false);
        if focus.is_none() && !t.dia.is_empty() { by_day.entry(t.dia.clone()).or_default().summary(t, share, agents, false); }
        let b = by_kind.entry(t.tipo.clone()).or_default().entry(t.nome.clone()).or_default();
        if b.output.plugin.is_empty() { b.output.plugin = plugin.into(); }
        b.session(t); b.output.chamadas += t.chamadas; b.output.ctx_chars += t.ctx_chars; b.estimated += t.tokens_est;
        if matches!(t.origem.as_str(), "voce" | "pedido") { b.output.pedidas += t.chamadas; }
        if t.tipo == "skill" {
            b.ruler = 2.5; b.occupied += t.ocupados; b.equivalent += t.ocupados_eq; b.output.respostas += t.respostas;
        }
        if matches!(t.tipo.as_str(), "skill" | "area") {
            b.add_tokens(usage_tokens(t)); b.output.cost += real.total;
            if t.tipo == "skill" { b.add_cost(Share { total: 0.0, ..real }); }
        } else if t.tipo == "agente" { b.add_agent(t, agents); }
        if !plugin.is_empty() && matches!(t.tipo.as_str(), "skill" | "contexto" | "agente") {
            let p = plugins.entry(plugin.into()).or_default(); p.output.plugin = plugin.into(); p.session(t);
            p.output.chamadas += t.chamadas; p.output.ctx_chars += t.ctx_chars;
            p.occupied += t.ocupados; p.equivalent += t.ocupados_eq; p.output.respostas += t.respostas;
            if t.tipo == "skill" { p.add_tokens(usage_tokens(t)); p.add_cost(real); }
            else if t.tipo == "agente" { p.add_agent(t, agents); }
        }
        if t.tipo == "area" && !t.dia.is_empty() {
            by_area_day.entry(format!("{}|{}", t.dia, t.nome)).or_default().area(t, real.total);
        }
        if focus == Some(t.nome.as_str()) { series.push((t, real.total, share)); }
    }
    let area_focus = focus.is_some_and(|f| by_kind.get("area").is_some_and(|areas| areas.contains_key(f)));
    for (t, cost, share) in series {
        if t.dia.is_empty() { continue; }
        if area_focus {
            if t.tipo == "area" { by_day.entry(t.dia.clone()).or_default().area(t, cost); }
        } else { by_day.entry(t.dia.clone()).or_default().summary(t, share, agents, true); }
    }
    let mut area_days = daily(by_area_day);
    for b in &mut area_days { b.label = b.key.split_once('|').map(|(_, area)| area.to_owned()); }
    let mut kind = |name: &str| ranked(by_kind.shift_remove(name).unwrap_or_default());
    UsoReport {
        totals: total.finish("totals".into()), by_skill: kind("skill"), by_tool: kind("tool"),
        by_bash: kind("bash"), by_mcp: kind("mcp"), by_agente: kind("agente"), by_contexto: kind("contexto"),
        by_plugin: ranked(plugins), by_imagem: kind("imagem"), by_area: kind("area"), by_area_dia: area_days,
        by_conta: dimension(by_account, Some(label)), by_projeto: dimension(by_project, None),
        by_modelo: dimension(by_model, None), by_day: daily(by_day), conta: accounts, projeto: projects,
        modelo: models, plugin: filters, foco: focus.map(str::to_owned),
        applied: Some(Applied { period: period.into() }), usd_brl: None,
    }
}
