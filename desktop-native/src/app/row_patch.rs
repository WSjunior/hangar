use std::{collections::HashMap, ops::Range};
use crate::{api::dto::ChatEvent, conversation::{Item, Tool}};

/// Recebe somente o sufixo invalidado; os índices devolvidos pertencem à lista inteira.
pub(super) struct RowPatch {
    pub remove: Range<usize>,
    pub insert: Range<usize>,
    pub resized: Vec<usize>,
}

impl RowPatch {
    pub fn between(from: usize, old: &[String], old_signatures: &[String], new: &[String], signatures: &[String], dirty: usize) -> Self {
        let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        let suffix = old[prefix..].iter().rev().zip(new[prefix..].iter().rev()).take_while(|(a, b)| a == b).count();
        let previous: HashMap<_, _> = old.iter().zip(old_signatures).collect();
        let resized = new.iter().zip(signatures).enumerate().filter_map(|(i, (id, signature))| {
            previous.get(id).filter(|old| i < dirty || *old != &signature).map(|_| from + i)
        }).collect();
        Self { remove: from + prefix..from + old.len() - suffix, insert: prefix..new.len() - suffix, resized }
    }
}

pub(super) fn reset_rows(
    ids: &mut Vec<String>,
    arrived: &mut HashMap<String, std::time::Instant>,
    folds: &mut HashMap<String, (bool, Option<std::time::Instant>)>,
) {
    ids.clear();
    arrived.clear();
    folds.clear();
}

/// As chaves pertencem à linha, permitindo descartar só seus caches ao reconstruí-la.
#[derive(Default)]
pub(super) struct RowAssets {
    pub prepared: Vec<String>,
    pub parts: Vec<String>,
}

impl RowAssets {
    pub fn of(item: &Item, events: &[ChatEvent], paired: &HashMap<usize, usize>) -> Self {
        let mut assets = Self::default();
        let mut tools = Vec::new();
        match item {
            Item::Event(i) => assets.prepared.push(events[*i].id.clone()),
            Item::Orphan(i) => assets.prepared.push(format!("{}:result", events[*i].id)),
            Item::Tool(tool) => tools.push(*tool),
            Item::Group { tools: group, .. } => {
                assets.parts.extend(group.iter().map(|tool| events[tool.call].id.clone()));
                tools.extend(group.iter().copied().filter(|tool| events[tool.call].kind != "thinking"));
            }
            Item::Thinking { parts, .. } => tools.extend(parts.iter().filter(|&&i| events[i].kind != "thinking")
                .map(|&call| Tool { call, result: paired.get(&call).copied() })),
            Item::Tasks { .. } => {}
        }
        for tool in tools {
            assets.prepared.push(format!("{}:input", events[tool.call].id));
            assets.prepared.push(format!("{}:result", events[tool.call].id));
            if let Some(result) = tool.result { assets.prepared.push(format!("{}:lines", events[result].id)); }
        }
        assets
    }
}

#[cfg(test)]
mod tests {
    use super::RowPatch;

    fn strings(values: &[&str]) -> Vec<String> { values.iter().map(|s| (*s).into()).collect() }

    #[test]
    fn reset_releases_folds_and_arrivals_before_old_ids_are_lost() {
        use std::{collections::HashMap, time::Instant};
        let mut ids = strings(&["old", "group"]);
        let mut arrived = HashMap::from([("old".into(), Instant::now())]);
        let mut folds = HashMap::from([("group".into(), (true, Some(Instant::now())))]);
        super::reset_rows(&mut ids, &mut arrived, &mut folds);
        assert!(ids.is_empty());
        assert!(arrived.is_empty());
        assert!(folds.is_empty());
    }

    #[test]
    fn row_assets_track_late_results_and_group_parts_for_suffix_cleanup() {
        use crate::{appearance::ThinkingTools, conversation::{View, incremental::Incremental}};
        use serde_json::json;
        let mut events: Vec<crate::api::dto::ChatEvent> = [
            json!({"id":"message","kind":"assistant_msg","text":"Mensagem"}),
            json!({"id":"think","kind":"thinking","text":"Analisar"}),
            json!({"id":"call","kind":"tool_use","tool_use_id":"tool","tool_name":"Read"}),
        ].into_iter().map(|value| serde_json::from_value(value).unwrap()).collect();
        let view = View { merge_thinking: true, thinking: ThinkingTools::All, ..View::default() };
        let mut state = Incremental::default();
        state.update(&events, 0, view);
        let old = super::RowAssets::of(&state.items[1], &events, &state.paired);
        assert_eq!(old.parts, vec!["think", "call"]);
        assert_eq!(old.prepared, vec!["call:input", "call:result"]);
        events.push(serde_json::from_value(json!({"id":"out","kind":"tool_result","tool_use_id":"tool","result":"Saída"})).unwrap());
        let delta = state.update(&events, 3, view);
        assert_eq!(delta.from, 1);
        let new = super::RowAssets::of(&state.items[1], &events, &state.paired);
        assert_eq!(new.prepared, vec!["call:input", "call:result", "out:lines"]);
    }

    #[test]
    fn append_only_splices_new_rows_and_preserves_tail() {
        let old = strings(&["preview", "working"]);
        let new = strings(&["message", "preview", "working"]);
        let patch = RowPatch::between(1000, &old, &strings(&["", ""]), &new, &strings(&["", "", ""]), 1);
        assert_eq!(patch.remove, 1000..1000);
        assert_eq!(patch.insert, 0..1);
        assert!(patch.resized.is_empty());
    }

    #[test]
    fn dirty_content_remeasures_even_with_identical_id_and_signature() {
        let ids = strings(&["same", "working"]);
        let signatures = strings(&["", ""]);
        let patch = RowPatch::between(20, &ids, &signatures, &ids, &signatures, 1);
        assert_eq!(patch.remove, 22..22);
        assert_eq!(patch.resized, vec![20]);
    }

    #[test]
    fn replaced_row_and_changed_tail_get_correct_global_indices() {
        let patch = RowPatch::between(8, &strings(&["old", "live"]), &strings(&["", "before"]),
            &strings(&["new", "live"]), &strings(&["", "after"]), 1);
        assert_eq!(patch.remove, 8..9);
        assert_eq!(patch.insert, 0..1);
        assert_eq!(patch.resized, vec![9]);
    }

    #[test]
    fn empty_suffix_and_tail_only_update_do_not_invalidate_prefix() {
        let ids = strings(&["preview"]);
        let signatures = strings(&[""]);
        let patch = RowPatch::between(12000, &ids, &signatures, &ids, &signatures, 0);
        assert!(patch.resized.is_empty());
        assert_eq!(patch.remove, 12001..12001);
        let patch = RowPatch::between(12000, &[], &[], &[], &[], 0);
        assert_eq!(patch.remove, 12000..12000);
        assert!(patch.insert.is_empty());
    }
}
