//! Rodada em sombra (`CP_LIST_SHADOW=1`): com o Python ainda dono da lista, o Rust a produz a cada
//! tique sem servir, compara campo a campo com a assinatura da lista que o Python serviu (vem na
//! resposta dos fatos) e grava `rust.list_shadow_diff` no diário só com o nome da sessão e o do
//! campo. Nada do Rust é entregue: sem rebaixar marcador, sem mexer na presença do app.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hangar_api::session::SessionRow;
use serde_json::{Map, Value, json};

use super::bridge::{ListBridge, ProduceFacts};
use super::sig;
use crate::diag::DiagClient;

pub const ENV: &str = "CP_LIST_SHADOW";
/// O tique do `_ListRefresher`.
const TICK: Duration = Duration::from_millis(1500);
/// O Python sem lista recente (ninguém com ela aberta): nada a comparar, e a descoberta não roda à toa.
const IDLE: Duration = Duration::from_secs(10);
/// Sombra cega por motivo esperado (ninguém com a lista aberta, multiplexador recusando) vai ao diário
/// só depois deste tempo seguido; falha de verdade, na hora.
const BLIND_LIMIT: Duration = Duration::from_secs(120);
/// Teto de diferenças gravadas por rodada: um defeito em todas as linhas não inunda o diário; o resto
/// sai nas rodadas seguintes.
const MAX_REPORTS: usize = 50;
pub const ROW_MISSING: &str = "row_missing";
pub const ROW_EXTRA: &str = "row_extra";
pub const ROW_UNSERIALIZABLE: &str = "row_unserializable";

/// Assinatura de cada linha que o Python serviu, por nome: `{campo: valor reduzido}`.
pub type PySigs = HashMap<String, Map<String, Value>>;
/// (sessão, campo)
pub type Diff = (String, String);

/// Diferenças de propósito, registradas pelas Tasks que as criaram: não vão ao diário. Erro da
/// produção com estes códigos não é diferença: o Python lia a mesma recusa como zero sessões.
pub const ACCEPTED_ERRORS: [&str; 2] = ["mux_refused", "mux_unparsed"];
/// Campos que o retrato do runtime preenche nas sessões Claude sem terminal: na sombra elas ficam no
/// marcador, porque o retrato por nome só chega com o hub (Task 17).
const RUNTIME_FIELDS: [&str; 6] = ["state", "label", "question", "status_line", "pending_questions", "problema"];

pub fn enabled() -> bool { enabled_from(std::env::var(ENV).ok().as_deref()) }

fn enabled_from(v: Option<&str>) -> bool { v == Some("1") }

/// Diferença que alguma Task fez de propósito, com o motivo ao lado.
fn accepted(row: &SessionRow, field: &str) -> bool {
    match row.problema.as_deref() {
        // Captura que falhou (Task 12): o Rust fica no marcador sem rebaixar e mostra a falha.
        Some("list_capture_failed") if ["state", "problema", "label"].contains(&field) => return true,
        // Runtime sem terminal com erro (Task 12): a falha aparece na linha em vez de parada calada.
        Some("list_runtime_unavailable") if ["state", "problema"].contains(&field) => return true,
        _ => {}
    }
    (row.provider == "claude" && row.headless && RUNTIME_FIELDS.contains(&field))
        // A descoberta não sabe a credencial de Kimi/Pi/omp (Task 7); só o fato a preenche (Task 14).
        || (["kimi", "pi", "omp"].contains(&row.provider.as_str()) && field == "conta" && row.conta.is_none())
}

/// Kimi, Pi e omp têm nome de `sanitize_session_name`, que no Rust só desfaz os acentos do
/// português (`discover_other.rs`): letra de fora some em vez de virar a base, e o nome fica mais
/// curto. Mesma conversa (transcript e pasta, ambos conhecidos) com o nome do Rust contido no do
/// Python, a partir da mesma letra, é essa sessão.
fn renamed(row: &SessionRow, py_name: &str, sig: &Map<String, Value>) -> bool {
    let (Some(jsonl), Some(cwd)) = (&row.jsonl, &row.cwd) else { return false };
    let mut rest = py_name.chars();
    ["kimi", "pi", "omp"].contains(&row.provider.as_str())
        && sig.get("jsonl").and_then(Value::as_str) == Some(jsonl) && sig.get("cwd").and_then(Value::as_str) == Some(cwd)
        && row.name.chars().next().is_some_and(|c| py_name.starts_with(c))
        && row.name.chars().all(|c| rest.any(|p| p == c))
}

/// O valor do campo como o `_list_sig` o reduz.
fn field_value(row: &SessionRow, raw: &Value, field: &str) -> Value {
    match field {
        "status_line" => sig::status_sig(row.status_line.as_deref()),
        "context" => json!(sig::context_sig(row.context.as_ref())),
        "label" if row.provider == "codex" && !row.tracked => json!(row.label),
        "label" => json!(row.label.as_deref().is_some_and(|l| !l.is_empty())),
        "plan_tasks" => json!(row.plan_tasks.as_deref().unwrap_or_default()),
        // Campo que o Rust não tem vale nulo: o do Python com valor aparece como diferença.
        _ => raw.get(field).cloned().unwrap_or(Value::Null),
    }
}

/// Igualdade do JSON com número comparado pelo valor: `1` do Python e `1.0` do Rust são o mesmo.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_i64(), y.as_i64()) {
            (Some(x), Some(y)) => x == y,
            _ => x.as_f64() == y.as_f64(),
        },
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(x, y)| same(x, y)),
        (Value::Object(x), Value::Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w))),
        _ => a == b,
    }
}

/// Campos em que a lista do Rust diverge da do Python, fora as diferenças aceitas.
pub fn compare(rust: &[SessionRow], py: &PySigs) -> HashSet<Diff> {
    let mut out = HashSet::new();
    let by_name: HashMap<&str, &SessionRow> = rust.iter().map(|r| (r.name.as_str(), r)).collect();
    let mut extra: Vec<&SessionRow> = rust.iter().filter(|r| !py.contains_key(&r.name)).collect();
    for (name, sig) in py {
        let row = match by_name.get(name.as_str()) {
            Some(row) => *row,
            None => match extra.iter().position(|r| renamed(r, name, sig)) {
                Some(k) => extra.swap_remove(k),
                None => {
                    out.insert((name.clone(), ROW_MISSING.into()));
                    continue;
                }
            },
        };
        let Ok(raw) = serde_json::to_value(row) else {
            out.insert((name.clone(), ROW_UNSERIALIZABLE.into()));
            continue;
        };
        for (field, want) in sig.iter().filter(|(f, _)| *f != "name") {
            if !same(&field_value(row, &raw, field), want) && !accepted(row, field) {
                out.insert((name.clone(), field.clone()));
            }
        }
    }
    out.extend(extra.into_iter().map(|r| (r.name.clone(), ROW_EXTRA.into())));
    out
}

/// Grava cada (sessão, campo) uma vez enquanto a diferença durar. Só conta a diferença vista em duas
/// rodadas seguidas: a lista do Python é de até um tique antes da do Rust.
#[derive(Default)]
pub struct Reporter { prev: HashSet<Diff>, reported: HashSet<Diff> }

impl Reporter {
    pub fn update(&mut self, found: HashSet<Diff>) -> Vec<Diff> {
        let mut new: Vec<Diff> = found.iter().filter(|d| self.prev.contains(*d) && !self.reported.contains(*d)).cloned().collect();
        new.sort();
        new.truncate(MAX_REPORTS);
        self.reported.retain(|d| found.contains(d));
        self.reported.extend(new.iter().cloned());
        self.prev = found;
        new
    }
}

/// Rodada sem comparação, pelo motivo. `quiet`: esperado por um tempo (ninguém com a lista do Python
/// aberta, multiplexador recusando); só vira registro se durar `BLIND_LIMIT`.
#[derive(Default)]
pub struct Blind { code: Option<&'static str>, since: Option<Instant>, told: bool }

impl Blind {
    /// `None` = a rodada comparou. Devolve o código a gravar agora, uma vez por sequência.
    pub fn round(&mut self, code: Option<&'static str>, quiet: bool, now: Instant) -> Option<&'static str> {
        let Some(c) = code else {
            if self.told {
                tracing::info!(code = self.code, "lista: rodada em sombra voltou a comparar");
            }
            *self = Self::default();
            return None;
        };
        if self.code != Some(c) {
            *self = Self { code: Some(c), since: Some(now), told: false };
        }
        let long = self.since.is_some_and(|s| now.duration_since(s) >= BLIND_LIMIT);
        if self.told || (quiet && !long) {
            return None;
        }
        self.told = true;
        Some(c)
    }
}

/// Liga a sombra se `CP_LIST_SHADOW=1`. O laço recomeça depois de um pânico, com registro.
pub fn spawn(list: Arc<ListBridge>, diag: DiagClient) -> Option<tokio::task::JoinHandle<()>> {
    if !enabled() {
        return None;
    }
    tracing::info!("lista: rodada em sombra ligada");
    Some(tokio::spawn(async move {
        loop {
            // Abortar a de fora derruba esta junto.
            let mut inner = crate::AbortOnDrop(tokio::spawn(run(list.clone(), diag.clone())));
            match (&mut inner.0).await {
                Err(e) if e.is_panic() => {
                    diag.report("rust.list_shadow_failed", "", "panic", "rodada em sombra caiu; recomeça");
                    tokio::time::sleep(IDLE).await;
                }
                _ => return,
            }
        }
    }))
}

async fn run(list: Arc<ListBridge>, diag: DiagClient) {
    let input = ProduceFacts { shadow: true, ..Default::default() };
    let (mut reporter, mut blind) = (Reporter::default(), Blind::default());
    loop {
        // (motivo de não comparar, se é esperado por um tempo, espera)
        let (code, quiet, wait) = match list.produce(&input).await {
            Ok(p) if !p.facts_ok => (Some("facts_unavailable"), false, TICK),
            Ok(p) => match p.facts.shadow.as_ref() {
                Some(py) => {
                    for (name, field) in reporter.update(compare(&p.rows, py)) {
                        diag.report("rust.list_shadow_diff", &name, &field, "lista do Rust diverge da do Python neste campo");
                    }
                    (None, false, TICK)
                }
                None => (Some("python_list_absent"), true, IDLE),
            },
            Err(e) => (Some(e.code), ACCEPTED_ERRORS.contains(&e.code), TICK),
        };
        if code.is_some() {
            // Diferença de antes da pausa não conta como vista "na rodada anterior".
            reporter.update(HashSet::new());
        }
        if let Some(c) = blind.round(code, quiet, Instant::now()) {
            let event = if quiet { "rust.list_shadow_blind" } else { "rust.list_shadow_failed" };
            diag.report(event, "", c, "rodada em sombra sem comparar");
        }
        tokio::time::sleep(wait).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(v: Value) -> SessionRow { serde_json::from_value(v).unwrap() }

    /// A assinatura que o `list_facts._row_sig` do Python manda para a linha.
    fn sig_of(r: &SessionRow) -> Map<String, Value> {
        let raw = serde_json::to_value(r).unwrap();
        ["name", "cwd", "state", "jsonl", "label", "status_line", "context", "plan_tasks", "conta", "problema", "pending_questions"]
            .iter().map(|f| ((*f).to_owned(), field_value(r, &raw, f))).collect()
    }

    fn py(rows: &[SessionRow]) -> PySigs { rows.iter().map(|r| (r.name.clone(), sig_of(r))).collect() }

    #[test]
    fn off_by_default() {
        assert!(!enabled_from(None));
        assert!(!enabled_from(Some("0")));
        assert!(!enabled_from(Some("")));
        assert!(enabled_from(Some("1")));
    }

    #[test]
    fn diff_names_fields_only() {
        let cx = row(json!({"name": "cx", "provider": "codex", "tracked": false, "label": "texto da Rust", "state": "idle"}));
        let mut sigs = py(&[cx.clone()]);
        sigs.get_mut("cx").unwrap().insert("label".into(), json!("texto do Python"));
        let found = compare(&[cx], &sigs);
        assert_eq!(found, HashSet::from([("cx".into(), "label".into())]));
        // O que vai ao diário é o nome do campo; o texto do rótulo nunca sai daqui.
        assert!(found.iter().all(|(s, f)| !s.contains("texto") && !f.contains("texto")));
    }

    #[test]
    fn equal_lists_have_no_diff_and_numbers_compare_by_value() {
        let cc = row(json!({"name": "cc", "state": "working", "label": "x", "status_line": "🤖 Haiku │ 💬 1k/2k 50k/200k",
                            "context": {"used": 50_000, "window": 200_000}, "pending_questions": 1}));
        let mut sigs = py(&[cc.clone()]);
        assert!(compare(&[cc.clone()], &sigs).is_empty());
        // `label` de Claude é só presença: o texto do spinner muda a cada quadro.
        let mut other = cc.clone();
        other.label = Some("outro".into());
        assert!(compare(&[other], &sigs).is_empty());
        sigs.get_mut("cc").unwrap().insert("pending_questions".into(), json!(1.0));
        assert!(compare(&[cc], &sigs).is_empty());
    }

    #[test]
    fn missing_and_extra_rows() {
        let a = row(json!({"name": "a"}));
        let b = row(json!({"name": "b"}));
        let found = compare(&[b], &py(&[a]));
        assert_eq!(found, HashSet::from([("a".into(), ROW_MISSING.into()), ("b".into(), ROW_EXTRA.into())]));
    }

    #[test]
    fn accepted_differences_stay_out() {
        // Captura que falhou: o Rust fica no marcador sem rebaixar e mostra a falha.
        let mut cc = row(json!({"name": "cc", "state": "awaiting_input", "problema": "list_capture_failed"}));
        let mut sigs = py(&[row(json!({"name": "cc", "state": "idle"}))]);
        assert!(compare(&[cc.clone()], &sigs).is_empty());
        // Fora da falha, o mesmo estado diferente é diferença.
        cc.problema = None;
        assert_eq!(compare(&[cc], &sigs), HashSet::from([("cc".into(), "state".into()), ]));
        // Runtime sem terminal com erro.
        let hl = row(json!({"name": "hl", "headless": true, "state": "working", "problema": "list_runtime_unavailable"}));
        sigs = py(&[row(json!({"name": "hl", "headless": true, "state": "idle"}))]);
        assert!(compare(&[hl], &sigs).is_empty());
        // Sem terminal, o retrato do runtime ainda não chega à sombra: estado e pergunta ficam no marcador.
        let hl = row(json!({"name": "hl", "headless": true, "state": "idle", "question": "q"}));
        sigs = py(&[row(json!({"name": "hl", "headless": true, "state": "awaiting_input"}))]);
        assert!(compare(&[hl], &sigs).is_empty());
        // Conta de Kimi/Pi/omp, que só o fato preenche; a do Codex continua comparada.
        let k = row(json!({"name": "k", "provider": "kimi", "conta": null}));
        let cx = row(json!({"name": "cx", "provider": "codex", "conta": null}));
        let mut sigs = py(&[k.clone(), cx.clone()]);
        for n in ["k", "cx"] { sigs.get_mut(n).unwrap().insert("conta".into(), json!("kimi:x")); }
        assert_eq!(compare(&[k.clone(), cx.clone()], &sigs), HashSet::from([("cx".into(), "conta".into())]));
        // Com a conta preenchida pelo fato, ela é comparada.
        let mut k = k;
        k.conta = Some("kimi:y".into());
        assert_eq!(compare(&[k, cx], &sigs), HashSet::from([("k".into(), "conta".into()), ("cx".into(), "conta".into())]));
    }

    #[test]
    fn name_folded_differently_is_the_same_session() {
        // "pi-ā" vira "pi-a" no NFKD do Python e "pi" no Rust, que só conhece os acentos do português.
        let rust = row(json!({"name": "pi", "provider": "pi", "jsonl": "/t/1.jsonl", "cwd": "/w", "state": "idle"}));
        let mut python = rust.clone();
        python.name = "pi-a".into();
        assert!(compare(&[rust.clone()], &py(&[python.clone()])).is_empty());
        // Outra conversa com o mesmo começo de nome não é ela.
        python.jsonl = Some("/t/2.jsonl".into());
        assert_eq!(compare(&[rust.clone()], &py(&[python])).len(), 2);
        // Sem transcript, pasta igual não basta: duas sessões novas na mesma pasta se confundiriam.
        let (mut r2, mut p2) = (rust.clone(), rust.clone());
        r2.jsonl = None;
        p2.jsonl = None;
        p2.name = "pi-a".into();
        assert_eq!(compare(&[r2], &py(&[p2])).len(), 2);
        // Nome que não começa igual não é ela.
        let mut p3 = rust.clone();
        p3.name = "api".into();
        assert_eq!(compare(&[rust], &py(&[p3])).len(), 2);
    }

    #[test]
    fn blind_rounds_are_told_once() {
        let t0 = Instant::now();
        let mut b = Blind::default();
        // Falha de verdade: na hora, uma vez por sequência.
        assert_eq!(b.round(Some("facts_unavailable"), false, t0), Some("facts_unavailable"));
        assert_eq!(b.round(Some("facts_unavailable"), false, t0), None);
        assert_eq!(b.round(None, false, t0), None);
        assert_eq!(b.round(Some("facts_unavailable"), false, t0), Some("facts_unavailable"));
        // Esperado: só depois de 2 min seguidos.
        assert_eq!(b.round(Some("python_list_absent"), true, t0), None);
        assert_eq!(b.round(Some("python_list_absent"), true, t0 + Duration::from_secs(60)), None);
        assert_eq!(b.round(Some("python_list_absent"), true, t0 + BLIND_LIMIT), Some("python_list_absent"));
        assert_eq!(b.round(Some("python_list_absent"), true, t0 + BLIND_LIMIT * 2), None);
    }

    #[test]
    fn reports_are_capped_per_round() {
        let mut r = Reporter::default();
        let many: HashSet<Diff> = (0..80).map(|i| (format!("s{i:02}"), "state".to_owned())).collect();
        r.update(many.clone());
        assert_eq!(r.update(many.clone()).len(), MAX_REPORTS);
        assert_eq!(r.update(many.clone()).len(), 80 - MAX_REPORTS, "o resto sai na rodada seguinte");
        assert!(r.update(many).is_empty());
    }

    #[test]
    fn reports_once_per_field_while_it_lasts() {
        let d = |s: &str, f: &str| (s.to_owned(), f.to_owned());
        let mut r = Reporter::default();
        // Uma rodada só pode ser a lista do Python um tique atrás.
        assert!(r.update(HashSet::from([d("a", "state")])).is_empty());
        assert_eq!(r.update(HashSet::from([d("a", "state"), d("b", "label")])), vec![d("a", "state")]);
        // Enquanto dura, não se repete.
        assert_eq!(r.update(HashSet::from([d("a", "state"), d("b", "label")])), vec![d("b", "label")]);
        assert!(r.update(HashSet::from([d("a", "state"), d("b", "label")])).is_empty());
        // Sumiu e voltou: é outra ocorrência.
        assert!(r.update(HashSet::new()).is_empty());
        assert!(r.update(HashSet::from([d("a", "state")])).is_empty());
        assert_eq!(r.update(HashSet::from([d("a", "state")])), vec![d("a", "state")]);
    }
}
