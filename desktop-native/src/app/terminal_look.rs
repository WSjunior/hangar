//! Pele Terminal: a chamada como o Claude Code desenha no terminal. "● Update(arquivo)", a linha do desfecho com o
//! cotovelo e, nas edições, o diff com uma numeração só, a do arquivo.
use super::*;
use crate::conversation::Family;
use crate::editdiff::{self, Edit, Op};

/// Linhas de um arquivo novo antes do "… +N linhas": o mesmo corte do Claude Code.
const WRITE_SHOWN: usize = 10;
/// Linhas de uma edição antes do "… +N linhas": acima disso o cabeçalho expande.
const EDIT_SHOWN: usize = 40;
const INDENT: f32 = 22.;

/// Edit, MultiEdit e o patch do Codex aparecem como "Update", que é o nome que o terminal usa.
fn display_name(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "edit" | "multiedit" | "notebookedit" | "apply_patch" => "Update".into(),
        _ => conversation::tool_display_name(name),
    }
}

/// Ferramentas que agem sobre um arquivo: só nelas o `path` é o arquivo e o cabeçalho mostra o nome dele.
const FILE_TOOLS: [&str; 7] = ["read", "notebookread", "edit", "multiedit", "notebookedit", "write", "apply_patch"];

/// O arquivo que a chamada cita, pela mesma busca do `editdiff` (lista vale o primeiro item); vazio é "sem arquivo".
fn input_path(input: Option<&serde_json::Map<String, serde_json::Value>>) -> Option<String> {
    Some(editdiff::input_path(input)).filter(|p| !p.is_empty()).map(str::to_owned)
}

/// O argumento do cabeçalho e, quando é um nome de arquivo, o caminho inteiro para o tooltip. Busca (Grep, Glob) tem
/// `path` de diretório: fica com o resumo de sempre.
fn header_arg(name: Option<&str>, input: Option<&serde_json::Map<String, serde_json::Value>>) -> (String, Option<String>) {
    let file_tool = name.is_some_and(|n| FILE_TOOLS.contains(&n.to_ascii_lowercase().as_str()));
    match file_tool.then(|| input_path(input)).flatten() {
        Some(path) => (crate::composer::basename(&path).to_owned(), Some(path)),
        None => (conversation::summarize_input(name, input), None),
    }
}

fn count(one: &str, many: &str, n: usize) -> String {
    crate::i18n::tr_shared(if n == 1 { one } else { many }, &[("n", &n.to_string())])
}

/// "2 linhas adicionadas, 1 removida", ou a linha do arquivo novo.
fn edit_summary(edits: &[Edit], created: bool) -> String {
    let (added, removed) = editdiff::totals(edits);
    if created {
        let key = if added == 1 { "term_gravou_1" } else { "term_gravou_n" };
        let file = edits.first().map_or("", |e| crate::composer::basename(&e.path));
        return crate::i18n::tr_shared(key, &[("n", &added.to_string()), ("arquivo", file)]);
    }
    match (added, removed) {
        (0, 0) => activity::web("editdiff_sem_mudanca"),
        (a, 0) => count("term_adicionadas_1", "term_adicionadas_n", a),
        (0, r) => count("term_so_removidas_1", "term_so_removidas_n", r),
        (a, r) => format!("{}, {}", count("term_adicionadas_1", "term_adicionadas_n", a), count("term_removidas_1", "term_removidas_n", r)),
    }
}

/// A cor do fundo e do marcador de uma linha: verde na adicionada, vermelho na removida, nenhuma no contexto.
fn tint(op: Op) -> Option<Hsla> {
    match op { Op::Add => Some(theme::success()), Op::Del => Some(theme::removed()), Op::Same => None }
}

/// Linha do diff no desenho do terminal: um número, o marcador e a linha inteira com fundo.
fn line(op: Op, number: Option<u32>, width: usize, code: impl IntoElement) -> Div {
    let (marker, tint) = (match op { Op::Add => "+", Op::Del => "-", Op::Same => " " }, tint(op));
    let ink = tint.unwrap_or_else(|| theme::faint().opacity(0.8));
    div().w_full().flex().font_family(theme::MONO).text_size(px(12.)).line_height(px(19.))
        .when_some(tint, |el, tint| el.bg(tint.opacity(0.14)))
        .child(div().flex_none().w(px(width as f32 * 7.5 + 8.)).flex().justify_end().text_color(ink).child(number.map(|n| n.to_string()).unwrap_or_default()))
        .child(div().flex_none().w(px(18.)).flex().justify_center().text_color(ink).child(marker))
        .child(div().flex_1().min_w_0().text_color(theme::text().opacity(0.92)).child(code))
}

/// O que separa uma edição da anterior: nada na primeira, o intervalo `…` no mesmo arquivo, o nome do arquivo quando muda.
#[derive(Debug, PartialEq, Eq)]
enum Between { Nothing, Gap, File(String) }

fn between(previous: Option<&str>, current: &str) -> Between {
    match previous {
        None => Between::Nothing,
        Some(p) if p == current => Between::Gap,
        Some(_) => Between::File(crate::composer::basename(current).to_owned()),
    }
}

/// Linha sem número, marcador nem fundo: o intervalo e o nome do arquivo, alinhados ao código.
fn note_row(text: String, tip: Option<(String, String)>) -> AnyElement {
    let row = div().w_full().font_family(theme::MONO).text_size(px(12.)).line_height(px(19.)).text_color(theme::muted());
    match tip {
        Some((id, path)) => row.id(SharedString::from(id))
            .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(path.clone()).build(window, cx))
            .child(text).into_any_element(),
        None => row.child(text).into_any_element(),
    }
}

/// Reparte o orçamento de linhas entre as edições, na ordem: cada uma leva o que cabe e o resto vai para as seguintes.
fn split_budget(lens: &[usize], budget: usize) -> Vec<usize> {
    let mut left = budget;
    lens.iter().map(|len| { let take = (*len).min(left); left -= take; take }).collect()
}

/// As edições que têm linha a desenhar, com a posição de cada uma. Uma sem linhas não esconde as seguintes: o
/// orçamento de linhas só some quando acaba, e aí as que sobram também têm `take == 0`.
fn visible_edits(takes: &[usize]) -> Vec<(usize, usize)> {
    takes.iter().copied().enumerate().filter(|(_, take)| *take > 0).collect()
}

/// O corpo de uma edição: os trechos, um embaixo do outro, nunca mais que `SHOWN_MAX` linhas de diff. `limit` corta o
/// arquivo novo e a edição recolhidos. Devolve as linhas, quantas ficaram de fora e quantas linhas de diff a chamada tem no total.
fn diff_body(call: &ChatEvent, edits: &[Edit], limit: Option<usize>, cx: &App) -> (Vec<AnyElement>, usize, usize) {
    let painted = super::edits::painted(call, edits, cx);
    let total: usize = edits.iter().map(|e| e.lines.len()).sum();
    let takes = split_budget(&edits.iter().map(|e| e.lines.len()).collect::<Vec<_>>(), limit.unwrap_or(usize::MAX).min(super::edits::SHOWN_MAX));
    let visible = visible_edits(&takes);
    let width = edits.iter().zip(&takes).flat_map(|(e, t)| &e.lines[..*t]).filter_map(|l| l.new.max(l.old)).max().unwrap_or(1).to_string().len();
    let mut rows: Vec<AnyElement> = Vec::new();
    let mut previous: Option<&str> = None;
    for (n, take) in visible {
        let edit = &edits[n];
        match between(previous, &edit.path) {
            Between::Nothing => {}
            Between::Gap => rows.push(note_row("…".into(), None)),
            Between::File(name) => rows.push(note_row(name, Some((format!("term-file-{}-{n}", call.id), edit.path.clone())))),
        }
        previous = Some(&edit.path);
        let new_marks = &painted[n].1;
        rows.extend(edit.lines[..take].iter().zip(&edit.spans).map(|(l, spans)| {
            let tint = tint(l.op);
            let len = l.text.len();
            let words = tint.into_iter().flat_map(|tint| spans.iter().filter(|r| r.start < len).map(move |r| (r.start..r.end.min(len),
                HighlightStyle { background_color: Some(tint.opacity(0.3)), ..Default::default() })));
            // Como no terminal, a removida fica sem cor de sintaxe: só o vermelho e o trecho trocado.
            let syntax = l.new.filter(|_| l.op != Op::Del).and_then(|no| new_marks.get(no.saturating_sub(edit.start.1) as usize)).into_iter().flatten()
                .filter(|(r, _)| r.start < len).map(|(r, s)| (r.start..r.end.min(len), *s));
            let highlights = gpui::combine_highlights(syntax, words).collect::<Vec<_>>();
            line(l.op, l.new.or(l.old), width, StyledText::new(l.text.clone()).with_highlights(highlights)).into_any_element()
        }));
    }
    (rows, total - takes.iter().sum::<usize>(), total)
}

/// Leitura ou busca que já terminou sem erro entra na linha cinza; erro e chamada em andamento ficam à vista. O
/// carregador de ferramentas fica fora, como na web.
fn folds_into_reads(name: Option<&str>, has_result: bool, is_error: bool) -> bool {
    has_result && !is_error && !name.is_some_and(|n| n.eq_ignore_ascii_case("toolsearch"))
        && matches!(conversation::family(name), Family::Read | Family::Search)
}

impl Hangar {
    /// Uma chamada na pele Terminal. Edição mostra o diff sempre; o resto abre entrada e saída no clique.
    pub(super) fn render_terminal_tool(&mut self, tool: Tool, row: &str, cx: &mut Context<Self>) -> AnyElement {
        if let Some(card) = self.render_agent_card(tool, cx) { return card; }
        let (key, name, error, edits, path, arg, created, outcome, status_color, dot, expandable, shown);
        {
            // Empresta o evento em vez de clonar: o resultado pode ser uma saída enorme e isto roda a cada quadro.
            let call = &self.chat.events[tool.call];
            let result = tool.result.map(|i| &self.chat.events[i]);
            key = call.id.clone();
            error = result.is_some_and(|r| r.is_error == Some(true));
            name = display_name(call.tool_name.as_deref().unwrap_or_default());
            edits = (!error).then(|| editdiff::shown(call, result)).flatten();
            (arg, path) = header_arg(call.tool_name.as_deref(), call.tool_input.as_ref());
            let status;
            (status, status_color) = self.tool_status(tool);
            dot = if error { theme::danger() } else if result.is_none() { theme::accent() } else { theme::success() };
            // Write sem patch e sem linha removida é arquivo novo; com patch os números já são os do arquivo.
            created = call.tool_name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case("write"))
                && edits.as_deref().is_some_and(|e| e.iter().all(|e| !e.patched && e.removed == 0));
            // Só o diff acima do corte (10 no arquivo novo, 40 na edição) tem o que abrir ou recolher.
            shown = if created { WRITE_SHOWN } else { EDIT_SHOWN };
            expandable = edits.as_deref().is_some_and(|e| e.iter().map(|e| e.lines.len()).sum::<usize>() > shown);
            outcome = match &edits { Some(edits) if result.is_some() => edit_summary(edits, created), _ => status };
        }
        let open = self.expanded.contains(&key);

        let toggle_key = key.clone();
        // Botão sem ícone: foca pelo teclado, ativa com Enter/Espaço e leva o rótulo de acessibilidade, como o `disclosure`.
        let inner = |name: String| div().w_full().flex().items_start().justify_start().gap(px(8.)).font_family(theme::MONO).text_size(px(12.5)).line_height(px(19.))
            .child(div().mt(px(6.)).size(px(7.)).flex_none().rounded_full().bg(dot))
            .child(div().min_w_0().flex().flex_wrap()
                .child(div().font_weight(FontWeight::BOLD).text_color(theme::text()).child(name))
                .child(div().min_w_0().text_color(theme::text()).child(format!("({arg})"))));
        // Edição com o diff todo à vista não tem o que abrir: o cabeçalho fica sem foco e sem clique, só com a dica do caminho.
        let inert = edits.is_some() && !expandable;
        let header: AnyElement = if inert {
            div().id(SharedString::from(format!("term-{key}"))).w_full()
                .when_some(path.clone(), |el, path| el.tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(path.clone()).build(window, cx)))
                .child(inner(name)).into_any_element()
        } else {
            // O rótulo só existe aqui: o cabeçalho inerte não o usa, e isto roda a cada quadro.
            let label = format!("{name}: {}. {outcome}", path.as_deref().unwrap_or(&arg));
            Button::new(SharedString::from(format!("term-{key}"))).ghost().w_full().h_auto().p_0().justify_start().toggled(open)
                .accessibility_label(label)
                .when_some(path.clone(), |el, path| el.tooltip(path))
                .child(inner(name))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle(toggle_key.clone(), cx))).into_any_element()
        };
        let outcome_line = div().flex().gap(px(8.)).pl(px(INDENT - 8.)).font_family(theme::MONO).text_size(px(12.5)).line_height(px(19.))
            .child(div().flex_none().text_color(theme::faint()).child("⎿"))
            .child(div().min_w_0().text_color(if error { theme::danger() } else if edits.is_some() { theme::text() } else { status_color }).child(outcome));

        let body = match &edits {
            Some(edits) => {
                // Recolhido, corta no `shown`; o clique no cabeçalho mostra o resto.
                let (rows, hidden, total) = diff_body(&self.chat.events[tool.call], edits, (!open).then_some(shown), cx);
                let faint = |text: String| div().text_size(px(12.)).font_family(theme::MONO).text_color(theme::faint()).child(text);
                let more = (!open && hidden > 0).then(|| faint(crate::i18n::tr_shared("term_mais_linhas", &[("n", &hidden.to_string())])));
                let clipped = (more.is_none() && total > super::edits::SHOWN_MAX).then(|| faint(tr("diff_clipped")
                    .replace("{shown}", &super::edits::SHOWN_MAX.to_string()).replace("{total}", &total.to_string())));
                let collapse = (open && expandable).then(|| faint(crate::i18n::tr_shared("term_recolher", &[])));
                Some(div().pl(px(INDENT + 16.)).flex().flex_col().children(rows).children(more).children(clipped).children(collapse))
            }
            None => open.then(|| self.tool_body(tool, row, cx).pl(px(INDENT + 16.))),
        };
        div().flex().flex_col().child(header).child(outcome_line).children(body).into_any_element()
    }

    /// Chamadas seguidas na pele Terminal: uma embaixo da outra, sem moldura de grupo. Leituras e buscas seguidas somam
    /// numa linha cinza que abre no clique; edição e comando nunca ficam escondidos.
    pub(super) fn render_terminal_group(&mut self, row: &str, tools: &[Tool], cx: &mut Context<Self>) -> AnyElement {
        let reads = |this: &Self, tool: &Tool| {
            let events = &this.chat.events;
            folds_into_reads(events[tool.call].tool_name.as_deref(), tool.result.is_some(), tool.result.is_some_and(|i| events[i].is_error == Some(true)))
        };
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut i = 0;
        while i < tools.len() {
            if !reads(self, &tools[i]) { rows.push(self.render_terminal_tool(tools[i], row, cx)); i += 1; continue; }
            let run = tools[i..].iter().take_while(|t| reads(self, t)).count();
            let key = format!("{row}#reads-{}", self.chat.events[tools[i].call].id);
            let open = self.expanded.contains(&key);
            // A mesma frase da contagem por família que o grupo dos Chips usa.
            let title = super::rows::family_title(&self.chat.events, &tools[i..i + run]);
            let toggle_key = key.clone();
            // Botão: foca pelo teclado, ativa com Enter/Espaço e leva o rótulo, como o cabeçalho da chamada.
            rows.push(div().pl(px(INDENT - 8.)).child(Button::new(SharedString::from(key)).ghost().w_full().h_auto().p_0().justify_start().toggled(open)
                .accessibility_label(title.clone())
                .child(div().w_full().font_family(theme::MONO).text_size(px(12.5)).text_color(theme::muted()).child(title))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle(toggle_key.clone(), cx)))).into_any_element());
            if open { for tool in &tools[i..i + run] { rows.push(self.render_terminal_tool(*tool, row, cx)); } }
            i += run;
        }
        div().flex().flex_col().gap(px(10.)).children(rows).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{Between, EDIT_SHOWN, between, folds_into_reads, header_arg, split_budget, visible_edits};
    use serde_json::json;

    #[test]
    fn file_tools_show_the_file_name_and_keep_the_full_path() {
        let read = json!({"file_path": "/a/b/models.py"});
        assert_eq!(header_arg(Some("Read"), read.as_object()), ("models.py".to_owned(), Some("/a/b/models.py".to_owned())));
        let pi = json!({"path": "src/x.rs"});
        assert_eq!(header_arg(Some("edit"), pi.as_object()).0, "x.rs");
    }

    #[test]
    fn search_and_shell_tools_keep_the_summary() {
        let grep = json!({"pattern": "patch", "path": "backend/app"});
        let (arg, path) = header_arg(Some("Grep"), grep.as_object());
        assert_eq!((arg, path), (crate::conversation::summarize_input(Some("Grep"), grep.as_object()), None));
        let bash = json!({"command": "npm run check"});
        assert_eq!(header_arg(Some("Bash"), bash.as_object()), (crate::conversation::summarize_input(Some("Bash"), bash.as_object()), None));
    }

    #[test]
    fn between_edits_picks_nothing_gap_or_file_name() {
        assert_eq!(between(None, "/a/x.rs"), Between::Nothing);
        assert_eq!(between(Some("/a/x.rs"), "/a/x.rs"), Between::Gap);
        assert_eq!(between(Some("/a/x.rs"), "/a/y.rs"), Between::File("y.rs".into()));
    }

    #[test]
    fn an_edit_without_lines_does_not_hide_the_next_ones() {
        assert_eq!(visible_edits(&[0, 3, 2]), vec![(1, 3), (2, 2)]);
        assert_eq!(visible_edits(&[4, 0, 0]), vec![(0, 4)]);
    }

    #[test]
    fn the_line_budget_is_split_across_edits_in_order() {
        assert_eq!(split_budget(&[30], EDIT_SHOWN), vec![30]);
        assert_eq!(split_budget(&[100], EDIT_SHOWN), vec![40]);
        assert_eq!(split_budget(&[30, 30], EDIT_SHOWN), vec![30, 10]);
        assert_eq!(split_budget(&[450], 400), vec![400]);
        assert_eq!(split_budget(&[40, 5], EDIT_SHOWN), vec![40, 0]);
    }

    #[test]
    fn only_settled_reads_and_searches_fold() {
        assert!(folds_into_reads(Some("Read"), true, false));
        assert!(!folds_into_reads(Some("Read"), true, true));
        assert!(!folds_into_reads(Some("Read"), false, false));
        assert!(folds_into_reads(Some("Grep"), true, false));
        assert!(!folds_into_reads(Some("ToolSearch"), true, false));
        assert!(!folds_into_reads(Some("Bash"), true, false));
        assert!(!folds_into_reads(Some("Edit"), true, false));
    }
}
