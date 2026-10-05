//! Diff das chamadas que editam arquivo (Edit, MultiEdit, Write, `apply_patch` do Codex), o `editdiff.ts` do web.
//! Fonte = a entrada da chamada, não o arquivo: os números de linha são do trecho editado, a partir de 1.
use std::{cell::RefCell, collections::HashMap, ops::Range, rc::Rc};
use serde_json::{Map, Value};
use crate::api::dto::{ChatEvent, PatchHunk};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op { Same, Del, Add }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line { pub op: Op, pub old: Option<u32>, pub new: Option<u32>, pub text: String }

/// Uma edição: o arquivo, os dois lados (tab já virou espaço, para o realce e a linha medirem igual) e o diff.
/// `start` é a primeira linha de cada lado no arquivo (1 quando só se conhece o trecho); `spans`, por linha, o que
/// mudou dentro dela. `patched`: a edição veio do patch gravado no resultado, não da entrada da chamada.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit { pub path: String, pub old: String, pub new: String, pub lines: Vec<Line>, pub added: usize, pub removed: usize,
    pub start: (u32, u32), pub spans: Vec<Vec<Range<usize>>>, pub patched: bool }

/// Acima disto o Myers para e o meio vira remoção + adição em bloco: diff válido, só não mínimo, e nunca trava.
const MAX_D: usize = 1_000;

/// Acima disto o realce por palavra desiste: uma linha minificada deixaria o diff pesado, e a web e o nativo têm de se comportar igual.
const WORD_DIFF_MAX_TOKENS: usize = 600;

fn split(text: &str) -> Vec<&str> {
    if text.is_empty() { return Vec::new(); }
    text.strip_suffix('\n').unwrap_or(text).split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect()
}

/// Myers O(ND) no trecho que sobra depois de tirar começo e fim iguais. `None` = passou do `MAX_D`.
fn myers(a: &[&str], b: &[&str]) -> Option<Vec<Op>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let max = (a.len() + b.len()).min(MAX_D) as isize;
    let off = max + 1;
    let mut v = vec![0isize; 2 * off as usize + 1];
    // Só a faixa -d..=d de cada passo: a memória cresce com D², não com D·(N+M).
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut found = None;
    'outer: for d in 0..=max {
        for k in (-d..=d).step_by(2) {
            let mut x = if k == -d || (k != d && v[(k - 1 + off) as usize] < v[(k + 1 + off) as usize]) { v[(k + 1 + off) as usize] }
                else { v[(k - 1 + off) as usize] + 1 };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] { x += 1; y += 1; }
            v[(k + off) as usize] = x;
            if x >= n && y >= m { found = Some(d); break 'outer; }
        }
        trace.push(v[(off - d) as usize..=(off + d) as usize].to_vec());
    }
    let found = found?;
    let mut ops = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (1..=found).rev() {
        let prev = &trace[d as usize - 1];
        let at = |k: isize| prev[(k + d - 1) as usize];
        let k = x - y;
        let prev_k = if k == -d || (k != d && at(k - 1) < at(k + 1)) { k + 1 } else { k - 1 };
        let prev_x = at(prev_k);
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y { ops.push(Op::Same); x -= 1; y -= 1; }
        if x == prev_x { ops.push(Op::Add); y -= 1; } else { ops.push(Op::Del); x -= 1; }
    }
    while x > 0 && y > 0 { ops.push(Op::Same); x -= 1; y -= 1; }
    ops.extend(std::iter::repeat_n(Op::Del, x as usize));
    ops.extend(std::iter::repeat_n(Op::Add, y as usize));
    ops.reverse();
    Some(ops)
}

/// Diff linha a linha de dois textos. Num bloco alterado as remoções vêm antes das adições, como no `git diff`.
pub fn diff(old: &str, new: &str) -> Vec<Line> {
    let (a, b) = (split(old), split(new));
    let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suf = a[pre..].iter().rev().zip(b[pre..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (mid_a, mid_b) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    let middle = myers(mid_a, mid_b).unwrap_or_else(|| {
        std::iter::repeat_n(Op::Del, mid_a.len()).chain(std::iter::repeat_n(Op::Add, mid_b.len())).collect()
    });
    let ops = std::iter::repeat_n(Op::Same, pre).chain(middle).chain(std::iter::repeat_n(Op::Same, suf));
    let (mut lines, mut o, mut n) = (Vec::new(), 0usize, 0usize);
    let mut adds: Vec<Line> = Vec::new();
    for op in ops {
        match op {
            Op::Same => {
                lines.append(&mut adds);
                lines.push(Line { op, old: Some(o as u32 + 1), new: Some(n as u32 + 1), text: a[o].to_owned() });
                o += 1; n += 1;
            }
            Op::Del => { lines.push(Line { op, old: Some(o as u32 + 1), new: None, text: a[o].to_owned() }); o += 1; }
            Op::Add => { adds.push(Line { op, old: None, new: Some(n as u32 + 1), text: b[n].to_owned() }); n += 1; }
        }
    }
    lines.append(&mut adds);
    lines
}

/// Palavra, espaço ou um símbolo sozinho: numa troca de concatenação o que muda é justamente o `+` e a aspa.
fn tokens(line: &str) -> Vec<&str> {
    let class = |c: char| if c.is_alphanumeric() || c == '_' { 0u8 } else if c.is_whitespace() { 1 } else { 2 };
    let (mut out, mut start, mut prev) = (Vec::new(), 0, None);
    for (i, c) in line.char_indices() {
        let now = class(c);
        if prev.is_some_and(|p| p != now || now == 2) { out.push(&line[start..i]); start = i; }
        prev = Some(now);
    }
    if start < line.len() { out.push(&line[start..]); }
    out
}

/// O que mudou dentro de um par de linhas (removida, adicionada), em bytes. `None` quando elas pouco têm em comum:
/// marcar quase tudo não diz nada.
pub fn word_spans(old: &str, new: &str) -> Option<(Vec<Range<usize>>, Vec<Range<usize>>)> {
    let (a, b) = (tokens(old), tokens(new));
    if a.is_empty() || b.is_empty() || a.len() + b.len() > WORD_DIFF_MAX_TOKENS { return None; }
    fn push(list: &mut Vec<Range<usize>>, range: Range<usize>) {
        match list.last_mut() { Some(last) if last.end == range.start => last.end = range.end, _ => list.push(range) }
    }
    let (mut del, mut add) = (Vec::new(), Vec::new());
    let (mut i, mut j, mut x, mut y, mut same) = (0, 0, 0, 0, 0);
    for op in myers(&a, &b)? {
        match op {
            Op::Same => { same += a[i].encode_utf16().count(); x += a[i].len(); y += b[j].len(); i += 1; j += 1; }
            Op::Del => { push(&mut del, x..x + a[i].len()); x += a[i].len(); i += 1; }
            Op::Add => { push(&mut add, y..y + b[j].len()); y += b[j].len(); j += 1; }
        }
    }
    // O limiar mede em unidades UTF-16, como o `editdiff.ts`; os intervalos devolvidos continuam em bytes.
    let units = |text: &str| text.encode_utf16().count();
    (same * 10 >= units(old).max(units(new)) * 4).then_some((del, add))
}

/// Dentro de um bloco alterado, a i-ésima removida faz par com a i-ésima adicionada.
fn line_spans(lines: &[Line]) -> Vec<Vec<Range<usize>>> {
    let mut out = vec![Vec::new(); lines.len()];
    let mut i = 0;
    while i < lines.len() {
        if lines[i].op != Op::Del { i += 1; continue; }
        let dels = lines[i..].iter().take_while(|l| l.op == Op::Del).count();
        let adds = lines[i + dels..].iter().take_while(|l| l.op == Op::Add).count();
        for k in 0..dels.min(adds) {
            if let Some((del, add)) = word_spans(&lines[i + k].text, &lines[i + dels + k].text) {
                out[i + k] = del;
                out[i + dels + k] = add;
            }
        }
        i += dels + adds;
    }
    out
}

/// As edições a partir dos trechos que o Claude Code gravou no resultado: uma por trecho, com a linha real do arquivo.
pub fn from_patch(path: &str, hunks: &[PatchHunk]) -> Vec<Edit> {
    hunks.iter().filter_map(|hunk| {
        let (mut o, mut n) = (hunk.old_start, hunk.new_start);
        let (mut old, mut new, mut lines) = (Vec::new(), Vec::new(), Vec::new());
        for raw in &hunk.lines {
            // Como em `split`: o `\r` do fim não é parte da linha.
            let raw = raw.strip_suffix('\r').unwrap_or(raw.as_str());
            // `\ No newline at end of file` é nota do diff, não linha do arquivo.
            if raw.starts_with('\\') { continue; }
            let text = raw.get(1..).unwrap_or("").replace('\t', "    ");
            match raw.as_bytes().first() {
                Some(b'-') => { lines.push(Line { op: Op::Del, old: Some(o), new: None, text: text.clone() }); old.push(text); o = o.saturating_add(1); }
                Some(b'+') => { lines.push(Line { op: Op::Add, old: None, new: Some(n), text: text.clone() }); new.push(text); n = n.saturating_add(1); }
                _ => {
                    lines.push(Line { op: Op::Same, old: Some(o), new: Some(n), text: text.clone() });
                    old.push(text.clone()); new.push(text); o = o.saturating_add(1); n = n.saturating_add(1);
                }
            }
        }
        if lines.is_empty() { return None; }
        Some(build(path, old.join("\n"), new.join("\n"), lines, (hunk.old_start, hunk.new_start), true))
    }).collect()
}

fn edit(path: &str, old: &str, new: &str) -> Edit {
    let (old, new) = (old.replace('\t', "    "), new.replace('\t', "    "));
    let lines = diff(&old, &new);
    build(path, old, new, lines, (1, 1), false)
}

/// Monta a edição a partir do diff: a contagem de linhas e o realce por palavra saem sempre da mesma conta.
fn build(path: &str, old: String, new: String, lines: Vec<Line>, start: (u32, u32), patched: bool) -> Edit {
    let added = lines.iter().filter(|l| l.op == Op::Add).count();
    let removed = lines.iter().filter(|l| l.op == Op::Del).count();
    let spans = line_spans(&lines);
    Edit { path: path.to_owned(), old, new, lines, added, removed, start, spans, patched }
}

/// Um par antigo/novo nos dois dialetos: Claude (`old_string`/`new_string`) e Pi (`oldText`/`newText`).
fn pair(value: &Map<String, Value>) -> Option<(&str, &str)> {
    let text = |a: &str, b: &str| value.get(a).or_else(|| value.get(b)).and_then(Value::as_str);
    Some((text("old_string", "oldText")?, text("new_string", "newText")?))
}

/// O patch do Codex: um bloco por `*** <verbo> File: <caminho>`; contexto entra nos dois lados, `-` no velho, `+` no novo.
fn patch_edits(patch: &str) -> Vec<Edit> {
    let mut out = Vec::new();
    let mut block: Option<(String, Vec<&str>, Vec<&str>)> = None;
    let close = |block: &mut Option<(String, Vec<&str>, Vec<&str>)>, out: &mut Vec<Edit>| {
        // Bloco sem linha nenhuma (`Delete File` sozinho) não vira diff: "sem mudança" seria mentira.
        if let Some((path, old, new)) = block.take().filter(|(_, o, n)| !o.is_empty() || !n.is_empty()) {
            out.push(edit(&path, &old.join("\n"), &new.join("\n")));
        }
    };
    for line in patch.split('\n') {
        let file = ["Add", "Update", "Delete"].iter().find_map(|verb| line.strip_prefix(&format!("*** {verb} File: ")));
        if let Some(path) = file {
            close(&mut block, &mut out);
            block = Some((path.trim().to_owned(), Vec::new(), Vec::new()));
            continue;
        }
        // Begin/End Patch, Move to, End of File: marcas do formato; conteúdo sempre começa com espaço, `+` ou `-`.
        if line.starts_with("*** ") { if line.starts_with("*** End Patch") { close(&mut block, &mut out); } continue; }
        let Some((_, old, new)) = block.as_mut() else { continue };
        // `@@` é cabeçalho de trecho e `\ No newline` é nota do diff: nenhum dos dois é linha do arquivo.
        if line.starts_with("@@") || line.starts_with('\\') { continue; }
        if let Some(rest) = line.strip_prefix('-') { old.push(rest); }
        else if let Some(rest) = line.strip_prefix('+') { new.push(rest); }
        else { let same = line.strip_prefix(' ').unwrap_or(line); old.push(same); new.push(same); }
    }
    close(&mut block, &mut out);
    out
}

pub(crate) fn input_path(input: Option<&Map<String, Value>>) -> &str {
    match input.and_then(|i| ["file_path", "path", "notebook_path"].iter().find_map(|k| i.get(*k))) {
        Some(Value::String(path)) => path.as_str(),
        Some(Value::Array(list)) => list.first().and_then(Value::as_str).unwrap_or(""),
        _ => "",
    }
}

/// As edições da chamada, ou `None` quando não é edição de arquivo ou o formato é outro (aí fica a entrada crua).
pub fn edits(name: Option<&str>, input: Option<&Map<String, Value>>) -> Option<Vec<Edit>> {
    let input = input?;
    let path = input_path(Some(input));
    let out = match name?.to_ascii_lowercase().as_str() {
        // Write não traz o conteúdo anterior: tudo entra como adição, como o próprio Claude Code desenha.
        "write" => vec![edit(path, "", input.get("content").and_then(Value::as_str).filter(|c| !c.is_empty())?)],
        "apply_patch" => patch_edits(input.get("patch").or_else(|| input.get("code")).and_then(Value::as_str)?),
        "edit" | "multiedit" => match pair(input) {
            Some((old, new)) => vec![edit(path, old, new)],
            None => input.get("edits")?.as_array()?.iter().map(|e| e.as_object().and_then(pair).map(|(o, n)| edit(path, o, n))).collect::<Option<_>>()?,
        },
        _ => return None,
    };
    (!out.is_empty()).then_some(out)
}

thread_local! {
    static CACHE: RefCell<HashMap<String, Option<Rc<[Edit]>>>> = RefCell::new(HashMap::new());
}

/// Acima disto o cache inteiro é descartado.
const CACHE_MAX: usize = 512;

/// Consulta o cache pela chave; no erro calcula e guarda (inclusive o `None`, para não recalcular a cada quadro).
fn cached(key: &str, make: impl FnOnce() -> Option<Rc<[Edit]>>) -> Option<Rc<[Edit]>> {
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(found) = cache.get(key) { return found.clone(); }
        if cache.len() >= CACHE_MAX { cache.clear(); }
        let found = make();
        cache.insert(key.to_owned(), found.clone());
        found
    })
}

/// As edições da chamada, calculadas uma vez: o desenho pede a cada quadro, e o Myers de uma edição grande pesa.
// ponytail: o cache inteiro some ao passar de 512 chamadas; um LRU entra se a troca de sessão ficar lenta.
pub fn of(call: &ChatEvent) -> Option<Rc<[Edit]>> {
    cached(&call.id, || edits(call.tool_name.as_deref(), call.tool_input.as_ref()).map(Into::into))
}

/// O que o cartão mostra: os trechos do resultado, com a linha real, quando o Claude Code os gravou; senão, a entrada.
pub fn shown(call: &ChatEvent, result: Option<&ChatEvent>) -> Option<Rc<[Edit]>> {
    let hunks = result.filter(|r| r.is_error != Some(true)).and_then(|r| r.patch.as_deref()).filter(|h| !h.is_empty());
    let Some(hunks) = hunks else { return of(call) };
    let found = cached(&format!("{}#patch", call.id), || {
        let edits = from_patch(input_path(call.tool_input.as_ref()), hunks);
        (!edits.is_empty()).then(|| edits.into())
    });
    found.or_else(|| of(call))
}

/// Linhas postas e tiradas na chamada inteira.
pub fn totals(edits: &[Edit]) -> (usize, usize) {
    edits.iter().fold((0, 0), |(a, r), e| (a + e.added, r + e.removed))
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;
    use serde_json::json;
    use super::*;

    fn obj(value: Value) -> Map<String, Value> { value.as_object().cloned().unwrap() }

    fn marked(old: &str, new: &str) -> Vec<String> {
        diff(old, new).into_iter().map(|l| format!("{}{}", match l.op { Op::Same => ' ', Op::Del => '-', Op::Add => '+' }, l.text)).collect()
    }

    #[test]
    fn changed_block_lists_removals_before_additions_with_both_numbers() {
        let lines = diff("a\nb\nc\nd", "a\nB\nC\nd");
        assert_eq!(lines.iter().map(|l| (l.op, l.old, l.new)).collect::<Vec<_>>(), vec![
            (Op::Same, Some(1), Some(1)), (Op::Del, Some(2), None), (Op::Del, Some(3), None),
            (Op::Add, None, Some(2)), (Op::Add, None, Some(3)), (Op::Same, Some(4), Some(4)),
        ]);
    }

    #[test]
    fn diff_is_minimal_and_rebuilds_both_sides() {
        let (old, new) = ("x\na\nb\nc\ny\nz", "a\nb\nq\nc\nz\nw");
        let lines = diff(old, new);
        let side = |keep: Op| lines.iter().filter(|l| l.op != keep).map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
        assert_eq!(side(Op::Add), old);
        assert_eq!(side(Op::Del), new);
        assert_eq!(lines.iter().filter(|l| l.op != Op::Same).count(), 4, "x, y saem; q, w entram; a b c z ficam");
    }

    #[test]
    fn edges_pure_insert_remove_identical_and_trailing_newline() {
        assert_eq!(marked("", "a\nb"), ["+a", "+b"]);
        assert_eq!(marked("a\nb", ""), ["-a", "-b"]);
        assert_eq!(marked("a\nb\n", "a\nb"), [" a", " b"]);
        assert_eq!(marked("a\r\nb", "a\nb"), [" a", " b"]);
        assert!(diff("", "").is_empty());
    }

    #[test]
    fn past_the_limit_falls_back_to_a_block_without_hanging() {
        let old: String = (0..3000).map(|i| format!("o{i}\n")).collect();
        let new: String = (0..3000).map(|i| format!("n{i}\n")).collect();
        let lines = diff(&format!("head\n{old}tail"), &format!("head\n{new}tail"));
        assert_eq!(lines.len(), 6002);
        assert_eq!((lines[0].op, lines[1].op, lines[3001].op, lines[6001].op), (Op::Same, Op::Del, Op::Add, Op::Same));
    }

    #[test]
    fn extracts_edit_multiedit_pi_and_write() {
        let one = edits(Some("Edit"), Some(&obj(json!({"file_path": "/a.rs", "old_string": "a\n\tb", "new_string": "a\n\tc"})))).unwrap();
        assert_eq!((one[0].path.as_str(), one[0].added, one[0].removed, one[0].new.as_str()), ("/a.rs", 1, 1, "a\n    c"));
        let multi = edits(Some("MultiEdit"), Some(&obj(json!({"file_path": "/a", "edits": [
            {"old_string": "x", "new_string": "y"}, {"old_string": "p", "new_string": "p\nq"}]})))).unwrap();
        assert_eq!(totals(&multi), (2, 1));
        let pi = edits(Some("edit"), Some(&obj(json!({"path": "/p", "edits": [{"oldText": "a", "newText": "b"}]})))).unwrap();
        assert_eq!((pi[0].path.as_str(), totals(&pi)), ("/p", (1, 1)));
        let write = edits(Some("Write"), Some(&obj(json!({"file_path": "/w", "content": "1\n2\n3\n"})))).unwrap();
        assert_eq!(totals(&write), (3, 0));
        assert!(edits(Some("Write"), Some(&obj(json!({"file_path": "/w", "content": ""})))).is_none());
        assert!(edits(Some("Edit"), Some(&obj(json!({"file_path": "/a"})))).is_none());
        assert!(edits(Some("Read"), Some(&obj(json!({"file_path": "/a"})))).is_none());
    }

    #[test]
    fn apply_patch_splits_files_and_skips_format_marks() {
        let patch = ["*** Begin Patch", "*** Update File: /a.py", "*** Move to: /b.py", "@@ def f():", " keep", "-um",
            "\\ No newline at end of file", "+dois", "+*** divisor ***", "*** End of File", "*** Add File: /c.md", "+novo",
            "*** Delete File: /d.txt", "*** End Patch"].join("\n");
        let found = edits(Some("apply_patch"), Some(&obj(json!({"code": patch, "file_path": ["/a.py", "/c.md"]})))).unwrap();
        assert_eq!(found.iter().map(|e| (e.path.as_str(), e.old.as_str(), e.new.as_str())).collect::<Vec<_>>(), vec![
            ("/a.py", "keep\num", "keep\ndois\n*** divisor ***"), ("/c.md", "", "novo"),
        ]);
        assert!(edits(Some("apply_patch"), Some(&obj(json!({"code": "nada disso"})))).is_none());
    }

    #[test]
    fn word_spans_mark_only_what_changed() {
        assert_eq!(word_spans("const T = 4000;", "const T = 8000;"), Some((vec![10..14], vec![10..14])));
        assert_eq!(word_spans("x = a+b;", "x = a??b;"), Some((vec![5..6], vec![5..7])));
        assert_eq!(word_spans("return a + b;", "return a - c;"), Some((vec![9..10, 11..12], vec![9..10, 11..12])));
        assert_eq!(word_spans("import x from \"y\";", "export default function z() {}"), None);
        assert_eq!(word_spans("", "novo"), None);
    }

    #[test]
    fn word_spans_give_up_on_huge_lines() {
        let line = |last: &str| format!("{} {last}", vec!["w"; 399].join(" "));
        assert_eq!(word_spans(&line("a"), &line("b")), None);
    }

    #[test]
    fn patch_numbers_lines_by_the_file_and_skips_the_newline_note() {
        let hunk = crate::api::dto::PatchHunk { old_start: 8, new_start: 8,
            lines: [" ", "-const T = 4000;", "\\ No newline at end of file", "+const T = 8000;", " \tx"].map(str::to_owned).to_vec() };
        let found = from_patch("/a.ts", &[hunk]);
        assert_eq!(found.len(), 1);
        let edit = &found[0];
        assert_eq!(edit.lines.iter().map(|l| (l.op, l.old, l.new)).collect::<Vec<_>>(), vec![
            (Op::Same, Some(8), Some(8)), (Op::Del, Some(9), None), (Op::Add, None, Some(9)), (Op::Same, Some(10), Some(10)),
        ]);
        assert_eq!((edit.start, edit.added, edit.removed), ((8, 8), 1, 1));
        assert!(edit.patched);
        assert_eq!((edit.old.as_str(), edit.new.as_str()), ("\nconst T = 4000;\n    x", "\nconst T = 8000;\n    x"));
        assert_eq!(edit.spans, vec![vec![], vec![10..14], vec![10..14], vec![]]);
    }

    #[test]
    fn accented_pair_crosses_the_threshold_like_the_web() {
        assert!(word_spans("// Ação já está pronta", "// Função não está pronta, então espera").is_some());
    }

    #[test]
    fn patch_lines_lose_the_trailing_carriage_return() {
        let hunk = crate::api::dto::PatchHunk { old_start: 1, new_start: 1, lines: vec!["+a\r".into()] };
        assert_eq!(from_patch("/a", &[hunk])[0].lines[0].text, "a");
    }

    #[test]
    fn patch_near_u32_max_does_not_overflow() {
        let hunk = crate::api::dto::PatchHunk { old_start: u32::MAX, new_start: u32::MAX, lines: vec!["+a".into(), "+b".into(), "-c".into(), "-d".into()] };
        assert_eq!(from_patch("/a", &[hunk])[0].lines.len(), 4);
    }

    #[test]
    fn uneven_block_pairs_the_first_removal_with_the_addition() {
        let lines = diff("let value = 4000;\nother line here", "let value = 8000;");
        let spans = line_spans(&lines);
        assert_eq!(lines.iter().map(|l| l.op).collect::<Vec<_>>(), vec![Op::Del, Op::Del, Op::Add]);
        assert_eq!((spans[0].is_empty(), spans[1].is_empty(), spans[2].is_empty()), (false, true, false));
    }

    #[test]
    fn input_edits_start_at_one_and_carry_spans() {
        let one = edits(Some("Edit"), Some(&obj(json!({"file_path": "/a", "old_string": "x = 1", "new_string": "x = 2"})))).unwrap();
        assert_eq!((one[0].start, one[0].spans.clone()), ((1, 1), vec![vec![4..5], vec![4..5]]));
        assert!(!one[0].patched);
    }

    #[test]
    fn patch_at_the_top_of_the_file_is_still_marked_as_patched() {
        let hunk = crate::api::dto::PatchHunk { old_start: 1, new_start: 1, lines: vec!["-a".into(), "+b".into()] };
        let found = from_patch("/a", &[hunk]);
        assert_eq!((found[0].start, found[0].patched), ((1, 1), true));
    }

    #[test]
    fn shown_prefers_the_patch_and_falls_back_to_the_input() {
        let call = ChatEvent { kind: "tool_use".into(), id: "shown-1".into(), tool_name: Some("Edit".into()),
            tool_input: json!({"file_path": "/a", "old_string": "a", "new_string": "b"}).as_object().cloned(), ..Default::default() };
        let patched = ChatEvent { kind: "tool_result".into(), id: "r".into(),
            patch: Some(vec![crate::api::dto::PatchHunk { old_start: 9, new_start: 9, lines: vec!["-a".into(), "+b".into()] }]), ..Default::default() };
        assert_eq!(shown(&call, Some(&patched)).unwrap()[0].lines[0].old, Some(9));
        assert_eq!(shown(&call, None).unwrap()[0].lines[0].old, Some(1));
        let failed = ChatEvent { is_error: Some(true), ..patched };
        assert_eq!(shown(&call, Some(&failed)).unwrap()[0].lines[0].old, Some(1));
    }
}
