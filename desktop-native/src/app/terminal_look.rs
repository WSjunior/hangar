//! Pele Terminal: a chamada como o Claude Code desenha no terminal. "● Update(arquivo)", a linha do desfecho com o
//! cotovelo e, nas edições, o diff com uma numeração só, a do arquivo.
use super::*;
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

fn count(one: &str, many: &str, n: usize) -> String { count_with(one, many, n, &[]) }

fn count_with(one: &str, many: &str, n: usize, extra: &[(&str, &str)]) -> String {
    let n_text = n.to_string();
    let params: Vec<(&str, &str)> = std::iter::once(("n", n_text.as_str())).chain(extra.iter().copied()).collect();
    crate::i18n::tr_shared(if n == 1 { one } else { many }, &params)
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

/// O que uma chamada conta na linha dobrada; a mesma regra do `terminalFoldKind` do core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FoldKind { Search, Read, List, Mcp, Shell }

// Os conjuntos do Claude Code: comando de shell cujas partes só usam estes conta como busca, leitura ou listagem; os
// neutros não decidem nada.
const SHELL_SEARCH: [&str; 8] = ["find", "grep", "rg", "ag", "ack", "locate", "which", "whereis"];
const SHELL_READ: [&str; 15] = ["cat", "head", "tail", "less", "more", "wc", "stat", "file", "strings", "jq", "awk", "cut", "sort", "uniq", "tr"];
const SHELL_LIST: [&str; 3] = ["ls", "tree", "du"];
const SHELL_NEUTRAL: [&str; 5] = ["echo", "printf", "true", "false", ":"];

/// Classifica o comando parte a parte, separando em `|`, `&`, `;` e quebra de linha fora de aspas e parênteses, como o
/// `separarComando` do core. Para no primeiro comando que não é de leitura: roda a cada quadro e não aloca.
fn shell_fold_kind(cmd: &str) -> FoldKind {
    let (mut search, mut read, mut list) = (false, false, false);
    let mut classify = |part: &str| {
        let word = part.split_whitespace().next().unwrap_or_default();
        if word.is_empty() || SHELL_NEUTRAL.contains(&word) { return true; }
        if SHELL_SEARCH.contains(&word) { search = true; }
        else if SHELL_READ.contains(&word) { read = true; }
        else if SHELL_LIST.contains(&word) { list = true; }
        else { return false; }
        true
    };
    let (mut start, mut depth, mut quote, mut escaped) = (0, 0usize, None::<u8>, false);
    for (i, &c) in cmd.as_bytes().iter().enumerate() {
        if escaped { escaped = false; continue; }
        if let Some(q) = quote {
            if c == b'\\' && q == b'"' { escaped = true; } else if c == q { quote = None; }
            continue;
        }
        match c {
            b'\\' => escaped = true,
            b'\'' | b'"' | b'`' => quote = Some(c),
            b'(' | b'{' => depth += 1,
            b')' | b'}' => depth = depth.saturating_sub(1),
            b'|' | b'&' | b';' | b'\n' if depth == 0 => {
                if !classify(&cmd[start..i]) { return FoldKind::Shell; }
                start = i + 1;
            }
            _ => {}
        }
    }
    if !classify(&cmd[start..]) { return FoldKind::Shell; }
    // A mesma precedência do Claude Code: listagem, depois busca, depois leitura.
    if list { FoldKind::List } else if search { FoldKind::Search } else if read { FoldKind::Read } else { FoldKind::Shell }
}

fn is_any(name: &str, names: &[&str]) -> bool { names.iter().any(|n| name.eq_ignore_ascii_case(n)) }

/// O servidor de um nome `mcp__<servidor>__<ferramenta>`, sem o prefixo `hangar-`, como o cartão mostra.
fn mcp_server(name: &str) -> Option<&str> {
    let mut parts = name.split("__");
    (parts.next() == Some("mcp")).then_some(())?;
    let server = parts.next()?;
    parts.next()?;
    Some(server.strip_prefix("hangar-").unwrap_or(server))
}

const SHELL_TOOLS: &[&str] = &["bash", "powershell", "shell", "exec", "exec_command"];

/// Tipo da chamada na linha dobrada, como o Claude Code conta; `None` não dobra (edição, gravação, agente e o carregador
/// de ferramentas ficam sempre à vista).
fn fold_kind(name: Option<&str>, input: Option<&serde_json::Map<String, serde_json::Value>>) -> Option<FoldKind> {
    let name = name.unwrap_or_default();
    if is_any(name, &["read", "notebookread"]) { return Some(FoldKind::Read); }
    if is_any(name, &["grep", "glob", "find", "websearch", "webfetch"]) { return Some(FoldKind::Search); }
    if name.eq_ignore_ascii_case("ls") { return Some(FoldKind::List); }
    if is_any(name, SHELL_TOOLS) {
        let kind = match input.and_then(|i| i.get("command").or_else(|| i.get("cmd"))) {
            Some(serde_json::Value::String(command)) => shell_fold_kind(command),
            // O Codex manda o comando em lista de palavras.
            Some(serde_json::Value::Array(words)) => shell_fold_kind(&words.iter().filter_map(|w| w.as_str()).collect::<Vec<_>>().join(" ")),
            _ => FoldKind::Shell,
        };
        return Some(kind);
    }
    mcp_server(name).map(|_| FoldKind::Mcp)
}

/// A linha dobrada, na ordem e no tempo verbal do Claude Code: "Buscou 1 padrão, leu 2 arquivos, rodou 3 comandos de
/// shell"; enquanto alguma roda, "Lendo 2 arquivos…". `kinds` é o tipo de cada chamada, já calculado pelo grupo.
fn fold_title(events: &[ChatEvent], tools: &[Tool], kinds: &[FoldKind], running: bool) -> String {
    let (mut counts, mut files, mut servers) = ([0usize; 5], std::collections::HashSet::new(), Vec::<&str>::new());
    for (tool, &kind) in tools.iter().zip(kinds) {
        counts[kind as usize] += 1;
        let call = &events[tool.call];
        match kind {
            FoldKind::Read => { let path = editdiff::input_path(call.tool_input.as_ref()); if !path.is_empty() { files.insert(path); } }
            FoldKind::Mcp => if let Some(server) = call.tool_name.as_deref().and_then(mcp_server) { if !servers.contains(&server) { servers.push(server); } },
            _ => {}
        }
    }
    // Como no Claude Code: arquivos distintos quando há caminho, senão quantas leituras.
    if !files.is_empty() { counts[FoldKind::Read as usize] = files.len(); }
    let names = servers.join(", ");
    // Na ordem de `FoldKind`: a chave da frase concluída e a da que está em andamento.
    const KEYS: [(&str, &str); 5] = [("term_dobra_buscou", "term_dobra_buscando"), ("term_dobra_leu", "term_dobra_lendo"),
        ("term_dobra_listou", "term_dobra_listando"), ("term_dobra_chamou", "term_dobra_chamando"), ("term_dobra_rodou", "term_dobra_rodando")];
    let parts: Vec<String> = KEYS.iter().zip(counts).filter(|(_, n)| *n > 0).map(|(&(done, live), n)| {
        let key = if running { live } else { done };
        count_with(&format!("{key}_1"), key, n, &[("nome", &names)])
    }).collect();
    let text = super::controls::capitalized(&parts.join(", "));
    if running { format!("{text}…") } else { text }
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
        // O que o SendUserFile mandou fica à vista: é a razão da chamada, e no Claude Code a imagem chega à pessoa.
        let call = &self.chat.events[tool.call];
        let sent = sends_files(call).then(|| tool_file_refs(call)).filter(|refs| !refs.is_empty())
            .map(|refs| div().pl(px(INDENT + 16.)).pt_1().child(self.render_refs(&format!("{row}-{key}-sent"), refs, cx)));
        div().flex().flex_col().child(header).child(outcome_line).children(sent).children(body).into_any_element()
    }

    /// Chamadas seguidas na pele Terminal: uma embaixo da outra, sem moldura de grupo. Buscas, leituras, MCP e comandos
    /// seguidos somam numa linha cinza que abre no clique, rodando ou com erro, como no Claude Code; edição nunca fica
    /// escondida.
    pub(super) fn render_terminal_group(&mut self, row: &str, tools: &[Tool], cx: &mut Context<Self>) -> AnyElement {
        // Uma classificação por chamada e por quadro: o parse do comando não se repete no laço nem no título.
        let kinds: Vec<Option<FoldKind>> = tools.iter().map(|t| {
            let call = &self.chat.events[t.call];
            fold_kind(call.tool_name.as_deref(), call.tool_input.as_ref())
        }).collect();
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut i = 0;
        while i < tools.len() {
            if kinds[i].is_none() { rows.push(self.render_terminal_tool(tools[i], row, cx)); i += 1; continue; }
            let run = kinds[i..].iter().take_while(|k| k.is_some()).count();
            let folded = &tools[i..i + run];
            let folded_kinds: Vec<FoldKind> = kinds[i..i + run].iter().flatten().copied().collect();
            let key = format!("{row}#fold-{}", self.chat.events[tools[i].call].id);
            let open = self.expanded.contains(&key);
            let live = folded.iter().rev().find(|t| t.result.is_none() && self.running(t.call)).copied();
            let title = fold_title(&self.chat.events, folded, &folded_kinds, live.is_some());
            let toggle_key = key.clone();
            // Botão: foca pelo teclado, ativa com Enter/Espaço e leva o rótulo, como o cabeçalho da chamada.
            let mut block = vec![div().pl(px(INDENT - 8.)).child(Button::new(SharedString::from(key)).ghost().w_full().h_auto().p_0().justify_start().toggled(open)
                .accessibility_label(title.clone())
                .child(div().w_full().font_family(theme::MONO).text_size(px(12.5)).text_color(theme::muted()).child(title))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle(toggle_key.clone(), cx)))).into_any_element()];
            if !open {
                // Recolhida, a linha mostra o que roda agora e a saída de cada falha, como o Claude Code.
                let sub = |text: String, color: Hsla| div().flex().gap(px(8.)).pl(px(INDENT)).font_family(theme::MONO).text_size(px(12.5)).line_height(px(19.))
                    .child(div().flex_none().text_color(theme::faint()).child("⎿"))
                    .child(div().min_w_0().truncate().text_color(color).child(text)).into_any_element();
                if let Some(tool) = live {
                    let call = &self.chat.events[tool.call];
                    let summary = conversation::summarize_input(call.tool_name.as_deref(), call.tool_input.as_ref());
                    // Comando em voo com o `$` na frente, como o Claude Code desenha.
                    let shell = call.tool_name.as_deref().is_some_and(|name| is_any(name, SHELL_TOOLS));
                    block.push(sub(if shell { format!("$ {summary}") } else { summary }, theme::muted()));
                }
                for tool in folded.iter().filter(|t| t.result.is_some_and(|r| self.chat.events[r].is_error == Some(true))) {
                    block.push(sub(self.tool_status(*tool).0, theme::danger()));
                }
            }
            rows.push(div().flex().flex_col().children(block).into_any_element());
            if open { for tool in folded { rows.push(self.render_terminal_tool(*tool, row, cx)); } }
            i += run;
        }
        div().flex().flex_col().gap(px(10.)).children(rows).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{Between, EDIT_SHOWN, FoldKind, between, fold_kind, header_arg, mcp_server, shell_fold_kind, split_budget, visible_edits};
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
    fn fold_kind_counts_like_claude_code() {
        let kind = |name: &str| fold_kind(Some(name), None);
        assert_eq!(kind("Read"), Some(FoldKind::Read));
        assert_eq!(kind("Grep"), Some(FoldKind::Search));
        assert_eq!(kind("mcp__hangar__send"), Some(FoldKind::Mcp));
        assert_eq!(kind("Bash"), Some(FoldKind::Shell));
        assert_eq!(kind("mcp__x"), None);
        assert_eq!(kind("ToolSearch"), None);
        assert_eq!(kind("Edit"), None);
        assert_eq!(kind("Write"), None);
        assert_eq!(kind("Agent"), None);
        let codex = json!({"cmd": ["rg", "-n", "x"]});
        assert_eq!(fold_kind(Some("exec_command"), codex.as_object()), Some(FoldKind::Search));
    }

    #[test]
    fn a_shell_command_made_only_of_reads_counts_as_a_read() {
        assert_eq!(shell_fold_kind("rg -n foo src | head -5"), FoldKind::Search);
        assert_eq!(shell_fold_kind("cat a.txt | jq .x"), FoldKind::Read);
        assert_eq!(shell_fold_kind("ls -la && tree"), FoldKind::List);
        assert_eq!(shell_fold_kind("echo oi; cat a"), FoldKind::Read);
        assert_eq!(shell_fold_kind("cd src && ls"), FoldKind::Shell);
        assert_eq!(shell_fold_kind("echo \"a | rg\""), FoldKind::Shell);
        assert_eq!(shell_fold_kind("npm run build"), FoldKind::Shell);
        assert_eq!(shell_fold_kind("(cd src && ls)"), FoldKind::Shell);
        assert_eq!(shell_fold_kind("rg x | grep y"), FoldKind::Search);
    }

    #[test]
    fn mcp_server_drops_the_hangar_prefix_like_the_card() {
        assert_eq!(mcp_server("mcp__hangar-computer-control__objetivo"), Some("computer-control"));
        assert_eq!(mcp_server("mcp__pmedico__progresso"), Some("pmedico"));
        assert_eq!(mcp_server("mcp__x"), None);
        assert_eq!(mcp_server("Bash"), None);
    }
}
