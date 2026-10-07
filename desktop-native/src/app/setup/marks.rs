//! Marcas que os scripts imprimem no modo `--app` (spec "Contrato entre o app e os scripts"): uma por linha. O que não é
//! marca é saída crua, guardada para "Ver detalhes".
use std::collections::VecDeque;

/// Versão do contrato que este app entende; `##HANGAR-PROTOCOLO##` diferente pára com `versao-diferente`.
pub(crate) const PROTOCOL: u32 = 1;
/// Linhas cruas guardadas por execução: "Ver detalhes" e o relatório do plano 3.
pub(crate) const KEEP_LINES: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Step { Prepare, Install, Tailscale, Phone, Final }

impl Step {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw {
            "preparar" => Some(Self::Prepare), "instalar" => Some(Self::Install), "tailscale" => Some(Self::Tailscale),
            "celular" => Some(Self::Phone), "final" => Some(Self::Final), _ => None,
        }
    }
}

/// Estado de etapa e de item. `fila` só aparece em item (a linha "na fila" do mock).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State { Queued, Doing, Ok, Pending, Failed }

impl State {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw {
            "fila" => Some(Self::Queued), "fazendo" => Some(Self::Doing), "ok" => Some(Self::Ok),
            "pendente" => Some(Self::Pending), "falhou" => Some(Self::Failed), _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum End { Ok, Pending, Failed }

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Mark {
    Protocol(u32),
    Step(Step, State),
    Item { id: String, state: State, text: String },
    Link { kind: String, url: String },
    Error(String),
    Pending { code: String, text: String },
    End(End),
    Failure(String),
    Warning(String),
}

pub(crate) fn parse_line(line: &str) -> Option<Mark> {
    let line = line.trim();
    let (tag, rest) = line.split_once(' ').unwrap_or((line, ""));
    let rest = rest.trim();
    let mut words = rest.splitn(3, ' ');
    match tag {
        "##HANGAR-PROTOCOLO##" => rest.parse().ok().map(Mark::Protocol),
        "##HANGAR-PASSO##" => Some(Mark::Step(Step::parse(words.next()?)?, State::parse(words.next()?)?)),
        "##HANGAR-ITEM##" => {
            let id = words.next().filter(|w| !w.is_empty())?.to_owned();
            let state = State::parse(words.next()?)?;
            Some(Mark::Item { id, state, text: words.next().unwrap_or("").trim().to_owned() })
        }
        "##HANGAR-LINK##" => {
            let kind = words.next().filter(|w| !w.is_empty())?.to_owned();
            let url = words.next()?.to_owned();
            // Só https: a URL vai direto para o navegador.
            url.starts_with("https://").then_some(Mark::Link { kind, url })
        }
        "##HANGAR-ERRO##" => words.next().filter(|w| !w.is_empty()).map(|code| Mark::Error(code.to_owned())),
        "##HANGAR-PENDENCIA##" => {
            let (code, text) = rest.split_once(' ').unwrap_or((rest, ""));
            (!code.is_empty()).then(|| Mark::Pending { code: code.to_owned(), text: text.trim().to_owned() })
        }
        "##HANGAR-FIM##" => match rest { "ok" => Some(Mark::End(End::Ok)), "pendente" => Some(Mark::End(End::Pending)),
            "falhou" => Some(Mark::End(End::Failed)), _ => None },
        "##HANGAR-FALHA##" => Some(Mark::Failure(rest.to_owned())),
        "##HANGAR-AVISO##" => Some(Mark::Warning(rest.to_owned())),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ItemRow { pub step: Option<Step>, pub id: String, pub state: State, pub text: String }

/// O acumulado de uma execução do script, refeito do arquivo inteiro ao reabrir o app.
#[derive(Clone, Debug, Default)]
pub(crate) struct Progress {
    pub protocol: Option<u32>,
    pub error: Option<String>,
    pub failure: Option<String>,
    pub pendings: Vec<(String, String)>,
    pub end: Option<End>,
    steps: Vec<(Step, State)>,
    items: Vec<ItemRow>,
    links: Vec<(String, String)>,
    lines: VecDeque<String>,
    current: Option<Step>,
}

impl Progress {
    pub(crate) fn feed(&mut self, line: &str) {
        match parse_line(line) {
            Some(Mark::Protocol(n)) => self.protocol = Some(n),
            Some(Mark::Step(step, state)) => {
                if state == State::Doing { self.current = Some(step); }
                match self.steps.iter_mut().find(|(s, _)| *s == step) { Some(slot) => slot.1 = state, None => self.steps.push((step, state)) }
            }
            // O item não diz a etapa: é a última que começou.
            Some(Mark::Item { id, state, text }) => match self.items.iter_mut().find(|i| i.id == id) {
                Some(row) => { row.state = state; if !text.is_empty() { row.text = text; } }
                None => self.items.push(ItemRow { step: self.current, id, state, text }),
            },
            Some(Mark::Link { kind, url }) => match self.links.iter_mut().find(|(k, _)| *k == kind) {
                Some(slot) => slot.1 = url,
                None => self.links.push((kind, url)),
            },
            Some(Mark::Error(code)) => self.error = Some(code),
            Some(Mark::Pending { code, text }) => match self.pendings.iter_mut().find(|(c, _)| *c == code) {
                Some(slot) => slot.1 = text,
                None => self.pendings.push((code, text)),
            },
            Some(Mark::End(end)) => self.end = Some(end),
            Some(Mark::Failure(text)) => { self.push_line(text.clone()); self.failure = Some(text); }
            Some(Mark::Warning(text)) => self.push_line(text),
            None => self.push_line(line.trim_end().to_owned()),
        }
    }

    fn push_line(&mut self, line: String) {
        if self.lines.len() == KEEP_LINES { self.lines.pop_front(); }
        self.lines.push_back(line);
    }

    pub(crate) fn step(&self, step: Step) -> Option<State> { self.steps.iter().find(|(s, _)| *s == step).map(|(_, st)| *st) }
    pub(crate) fn items(&self, step: Step) -> impl Iterator<Item = &ItemRow> { self.items.iter().filter(move |i| i.step == Some(step)) }
    pub(crate) fn all_items(&self) -> &[ItemRow] { &self.items }
    pub(crate) fn link(&self, kind: &str) -> Option<&str> { self.links.iter().find(|(k, _)| k == kind).map(|(_, u)| u.as_str()) }
    pub(crate) fn lines(&self) -> impl Iterator<Item = &String> { self.lines.iter() }
    pub(crate) fn mismatch(&self) -> bool { self.protocol.is_some_and(|n| n != PROTOCOL) }
    pub(crate) fn current(&self) -> Option<Step> { self.current }
    /// O item que está andando agora: título da barra da tela 3 e motivo da janela de senha.
    pub(crate) fn doing_item(&self) -> Option<&ItemRow> { self.items.iter().rev().find(|i| i.state == State::Doing) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mark_parses() {
        assert_eq!(parse_line("##HANGAR-PROTOCOLO## 1"), Some(Mark::Protocol(1)));
        assert_eq!(parse_line("##HANGAR-PASSO## instalar fazendo"), Some(Mark::Step(Step::Install, State::Doing)));
        assert_eq!(parse_line("##HANGAR-LINK## tailscale-login https://login.tailscale.com/a/abc"),
            Some(Mark::Link { kind: "tailscale-login".into(), url: "https://login.tailscale.com/a/abc".into() }));
        assert_eq!(parse_line("##HANGAR-ERRO## sem-internet"), Some(Mark::Error("sem-internet".into())));
        assert_eq!(parse_line("##HANGAR-PENDENCIA## tailscale-https o HTTPS da conta está desligado"),
            Some(Mark::Pending { code: "tailscale-https".into(), text: "o HTTPS da conta está desligado".into() }));
        assert_eq!(parse_line("##HANGAR-FIM## pendente"), Some(Mark::End(End::Pending)));
        assert_eq!(parse_line("##HANGAR-FALHA## git pull falhou"), Some(Mark::Failure("git pull falhou".into())));
        assert_eq!(parse_line("##HANGAR-AVISO## terminou com pendencias"), Some(Mark::Warning("terminou com pendencias".into())));
    }

    #[test]
    fn item_text_keeps_spaces_and_windows_line_ends() {
        assert_eq!(parse_line("##HANGAR-ITEM## claude fazendo baixando 31 de 48 MB\r"),
            Some(Mark::Item { id: "claude".into(), state: State::Doing, text: "baixando 31 de 48 MB".into() }));
        assert_eq!(parse_line("  ##HANGAR-ITEM## tmux ok"), Some(Mark::Item { id: "tmux".into(), state: State::Ok, text: String::new() }));
    }

    #[test]
    fn unknown_values_are_plain_output() {
        assert_eq!(parse_line("##HANGAR-PASSO## instalar talvez"), None);
        assert_eq!(parse_line("##HANGAR-PASSO## outra ok"), None);
        assert_eq!(parse_line("##HANGAR-FIM## quase"), None);
        assert_eq!(parse_line("##HANGAR-PROTOCOLO## um"), None);
        // Link fora de https nunca vira URL aberta no navegador.
        assert_eq!(parse_line("##HANGAR-LINK## tailscale-login file:///etc/passwd"), None);
        assert_eq!(parse_line("$ curl -fsSL https://claude.ai/install.sh | bash"), None);
    }

    #[test]
    fn items_belong_to_the_step_being_done_and_update_in_place() {
        let mut p = Progress::default();
        for line in ["##HANGAR-PROTOCOLO## 1", "##HANGAR-PASSO## preparar fazendo", "##HANGAR-ITEM## tmux ok tmux 3.5a",
            "##HANGAR-ITEM## claude fazendo baixando", "saída crua", "##HANGAR-ITEM## claude ok", "##HANGAR-PASSO## preparar ok",
            "##HANGAR-PASSO## instalar fazendo", "##HANGAR-ITEM## servidor fazendo Baixar o servidor"] { p.feed(line); }
        assert_eq!(p.step(Step::Prepare), Some(State::Ok));
        assert_eq!(p.step(Step::Install), Some(State::Doing));
        assert_eq!(p.current(), Some(Step::Install));
        let prepare: Vec<_> = p.items(Step::Prepare).map(|i| (i.id.as_str(), i.state, i.text.as_str())).collect();
        // Texto vazio na atualização mantém o anterior.
        assert_eq!(prepare, vec![("tmux", State::Ok, "tmux 3.5a"), ("claude", State::Ok, "baixando")]);
        assert_eq!(p.doing_item().map(|i| i.id.as_str()), Some("servidor"));
        assert_eq!(p.lines().cloned().collect::<Vec<_>>(), vec!["saída crua".to_owned()]);
        assert!(!p.mismatch());
        assert_eq!(p.end, None);
    }

    #[test]
    fn end_errors_pendings_and_protocol_are_kept() {
        let mut p = Progress::default();
        for line in ["##HANGAR-PROTOCOLO## 2", "##HANGAR-ERRO## sem-systemd", "##HANGAR-FALHA## este Linux não tem systemd",
            "##HANGAR-PENDENCIA## tailscale-https desligado", "##HANGAR-PENDENCIA## tailscale-https desligado na conta",
            "##HANGAR-LINK## tailscale-login https://a", "##HANGAR-LINK## tailscale-login https://b", "##HANGAR-FIM## falhou"] { p.feed(line); }
        assert!(p.mismatch());
        assert_eq!(p.error.as_deref(), Some("sem-systemd"));
        assert_eq!(p.failure.as_deref(), Some("este Linux não tem systemd"));
        assert_eq!(p.pendings, vec![("tailscale-https".to_owned(), "desligado na conta".to_owned())]);
        assert_eq!(p.link("tailscale-login"), Some("https://b"));
        assert_eq!(p.end, Some(End::Failed));
        // A frase da falha também aparece em "Ver detalhes".
        assert!(p.lines().any(|l| l == "este Linux não tem systemd"));
    }

    #[test]
    fn details_keep_only_the_last_lines() {
        let mut p = Progress::default();
        for n in 0..(KEEP_LINES + 10) { p.feed(&format!("linha {n}")); }
        assert_eq!(p.lines().count(), KEEP_LINES);
        assert_eq!(p.lines().next().map(String::as_str), Some("linha 10"));
    }
}
