//! Custo de uma sessão, com agrupamento independente dos dias do índice.

use crate::costs::{pricing::Pricing, rows::UsageRow};
use crate::costs::report_costs::{cost_sum, row_cost};
use indexmap::IndexMap;
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SessionCost {
    pub cost_usd: Option<f64>,
    pub missing_models: Vec<String>,
    pub has_usage: bool,
}

pub fn estimate(rows: Vec<UsageRow>, pricing: &Pricing) -> SessionCost {
    let mut grouped: IndexMap<(String, bool), UsageRow> = IndexMap::new();
    for row in rows {
        if row.input == 0 && row.output == 0 && row.cache_write == 0 && row.cache_read == 0 {
            continue;
        }
        let key = (row.model.clone(), row.codex_long_context);
        if let Some(before) = grouped.get_mut(&key) {
            before.input += row.input;
            before.output += row.output;
            before.cache_write += row.cache_write;
            before.cache_read += row.cache_read;
        } else {
            grouped.insert(key, row);
        }
    }
    let has_usage = !grouped.is_empty();
    let mut total = 0.0;
    let mut missing_models = BTreeSet::new();
    for row in grouped.into_values() {
        if let Some(parts) = row_cost(&row, pricing) {
            total += cost_sum(parts);
        } else {
            missing_models.insert(pricing.canonizar(&row.model));
        }
    }
    SessionCost {
        cost_usd: (has_usage && missing_models.is_empty()).then_some(total),
        missing_models: missing_models.into_iter().collect(),
        has_usage,
    }
}
