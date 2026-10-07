use std::time::{Duration, Instant};
use super::sidebar::Target;

const INPUT_WAIT: Duration = Duration::from_millis(500);

fn brazilian_layout(layout: &str) -> bool { matches!(layout, "Portuguese (Brazil)" | "com.apple.keylayout.Brazilian-ABNT2") }

/// Layouts cujos símbolos com Shift na fileira de números sabemos traduzir de volta para dígitos.
fn known_layout(layout: &str) -> bool {
    brazilian_layout(layout) || matches!(layout, "English (US)" | "com.apple.keylayout.US" | "com.apple.keylayout.ABC")
}

pub(super) fn digit_for_key(key: &str, layout: &str) -> Option<char> {
    if key.len() == 1 && key.as_bytes()[0].is_ascii_digit() { return key.chars().next(); }
    if !known_layout(layout) { return None; }
    let brazilian = brazilian_layout(layout);
    if brazilian && matches!(key, "dead_diaeresis" | "¨") { return Some('6'); }
    // O mesmo símbolo corresponde a números diferentes em outros layouts.
    match key {
        "!" => Some('1'), "@" => Some('2'), "#" => Some('3'), "$" => Some('4'), "%" => Some('5'),
        "^" => Some('6'), "&" => Some('7'), "*" => Some('8'), "(" => Some('9'), ")" => Some('0'), _ => None,
    }
}

/// O dígito de uma tecla pelo id ou pelo nome do layout atual (o mesmo layout aparece com um ou outro).
pub(super) fn layout_digit(key: &str, layouts: [&str; 2]) -> Option<char> {
    layouts.into_iter().find_map(|layout| digit_for_key(key, layout))
}

/// O dígito de uma tecla com os modificadores de segurar: a posição física vale em qualquer layout; sem ela, o símbolo
/// é traduzido pela tabela dos layouts conhecidos.
pub(super) fn event_digit(physical: Option<char>, key: &str, layouts: [&str; 2]) -> Option<char> {
    physical.or_else(|| layout_digit(key, layouts))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Entry {
    pub(super) target: Target,
    pub(super) incarnation: Option<String>,
}

#[derive(Default)]
pub(super) struct Selection {
    entries: Option<Vec<Entry>>,
    input: String,
    deadline: Option<Instant>,
    revision: u64,
}

impl Selection {
    pub(super) fn begin(&mut self, entries: Vec<Entry>) {
        self.entries = Some(entries);
        self.input.clear();
        self.deadline = None;
        self.revision += 1;
    }

    pub(super) fn cancel(&mut self) {
        self.entries = None;
        self.input.clear();
        self.deadline = None;
        self.revision += 1;
    }

    pub(super) fn active(&self) -> bool { self.entries.is_some() }

    pub(super) fn number(&self, target: &Target) -> Option<usize> {
        self.entries.as_ref()?.iter().position(|entry| entry.target == *target).map(|ix| ix + 1)
    }

    /// Enter, Backspace e Esc só pertencem à seleção enquanto há um número sendo digitado;
    /// fora disso seguem para o campo de texto (Ctrl+Shift+Backspace apaga palavra, etc.).
    pub(super) fn claims_edit_keys(&self) -> bool { self.active() && !self.input.is_empty() }

    pub(super) fn input(&self) -> &str { &self.input }
    pub(super) fn deadline(&self) -> Option<Instant> { self.deadline }
    pub(super) fn revision(&self) -> u64 { self.revision }

    pub(super) fn push_digit(&mut self, digit: char, now: Instant) -> bool {
        if !self.active() || !digit.is_ascii_digit() { return false; }
        self.input.push(digit);
        self.deadline = Some(now + self.wait());
        self.revision += 1;
        true
    }

    pub(super) fn backspace(&mut self, now: Instant) {
        if !self.active() || self.input.pop().is_none() { return; }
        self.deadline = (!self.input.is_empty()).then(|| now + self.wait());
        self.revision += 1;
    }

    /// Só espera o próximo dígito quando ele ainda pode formar um número que existe: com 12 sessões, só o "1".
    fn wait(&self) -> Duration {
        let count = self.entries.as_ref().map_or(0, Vec::len);
        let grows = self.input.parse::<usize>().ok().and_then(|n| n.checked_mul(10)).is_some_and(|n| n > 0 && n <= count);
        if grows { INPUT_WAIT } else { Duration::ZERO }
    }

    pub(super) fn finish_if_ready(&mut self, now: Instant) -> Option<Entry> {
        if !self.deadline.is_some_and(|deadline| now >= deadline) { return None; }
        self.confirm()
    }

    pub(super) fn confirm(&mut self) -> Option<Entry> {
        if !self.active() || self.input.is_empty() { return None; }
        let number = std::mem::take(&mut self.input).parse::<usize>().ok();
        self.deadline = None;
        self.revision += 1;
        self.entries.as_ref()?.get(number?.checked_sub(1)?).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(count: usize) -> Vec<Entry> {
        (1..=count).map(|number| Entry {
            target: Target::new("server", &format!("session-{number}")),
            incarnation: Some(format!("life-{number}")),
        }).collect()
    }

    #[test]
    fn shifted_digits_follow_known_layouts_without_guessing_other_layouts() {
        for (key, number) in [("!", '1'), ("@", '2'), ("#", '3'), ("$", '4'), ("%", '5'), ("^", '6'), ("&", '7'), ("*", '8'), ("(", '9'), (")", '0')] {
            assert_eq!(super::digit_for_key(key, "English (US)"), Some(number));
        }
        assert_eq!(super::digit_for_key("dead_diaeresis", "Portuguese (Brazil)"), Some('6'));
        assert_eq!(super::digit_for_key("¨", "com.apple.keylayout.Brazilian-ABNT2"), Some('6'));
        assert_eq!(super::digit_for_key("&", "German"), None);
        assert_eq!(super::digit_for_key("6", "German"), Some('6'));
        assert_eq!(super::digit_for_key("f6", "English (US)"), None);
    }

    #[test]
    fn edit_keys_belong_to_selection_only_while_a_number_is_typed() {
        let now = Instant::now();
        let mut selection = Selection::default();
        assert!(!selection.claims_edit_keys());
        selection.begin(entries(3));
        assert!(!selection.claims_edit_keys());
        selection.push_digit('2', now);
        assert!(selection.claims_edit_keys());
        selection.backspace(now);
        assert!(!selection.claims_edit_keys());
        selection.push_digit('1', now);
        selection.confirm();
        assert!(!selection.claims_edit_keys());
    }

    #[test]
    fn physical_digit_wins_on_layouts_outside_the_table() {
        let intl = "English (US, intl., with dead keys)";
        assert_eq!(super::event_digit(Some('2'), "@", [intl, intl]), Some('2'));
        assert_eq!(super::event_digit(Some('6'), "dead_circumflex", [intl, intl]), Some('6'));
        assert_eq!(super::event_digit(None, "@", [intl, intl]), None);
        assert_eq!(super::event_digit(None, "@", ["English (US)", "English (US)"]), Some('2'));
    }

    #[test]
    fn only_table_layouts_are_known() {
        assert!(super::known_layout("English (US)"));
        assert!(super::known_layout("com.apple.keylayout.Brazilian-ABNT2"));
        assert!(!super::known_layout("French"));
        assert!(!super::known_layout("German"));
    }

    #[test]
    fn each_digit_restarts_wait_and_selects_the_whole_number() {
        let now = Instant::now();
        let mut selection = Selection::default();
        selection.begin(entries(12));
        assert!(selection.push_digit('1', now));
        let first_revision = selection.revision();
        assert_eq!(selection.deadline(), Some(now + INPUT_WAIT));
        assert!(selection.push_digit('2', now + Duration::from_millis(300)));
        assert_ne!(selection.revision(), first_revision);
        assert_eq!(selection.input(), "12");
        assert_eq!(selection.deadline(), Some(now + Duration::from_millis(300)));
        assert_eq!(selection.finish_if_ready(now + Duration::from_millis(200)), None);
        let selected = selection.finish_if_ready(now + Duration::from_millis(300)).unwrap();
        assert_eq!(selected.target, Target::new("server", "session-12"));
        assert_eq!(selected.incarnation.as_deref(), Some("life-12"));
        assert!(selection.active());
        assert_eq!(selection.number(&selected.target), Some(12));
        assert_eq!(selection.input(), "");
        assert_eq!(selection.deadline(), None);
        assert_eq!(selection.finish_if_ready(now + Duration::from_secs(3)), None);
    }

    #[test]
    fn digit_that_cannot_grow_into_a_session_number_selects_at_once() {
        let now = Instant::now();
        let mut selection = Selection::default();
        selection.begin(entries(12));
        selection.push_digit('3', now);
        assert_eq!(selection.finish_if_ready(now).unwrap().target, Target::new("server", "session-3"));
        selection.begin(entries(9));
        selection.push_digit('1', now);
        assert_eq!(selection.deadline(), Some(now));
        selection.begin(entries(12));
        selection.push_digit('0', now);
        assert_eq!(selection.deadline(), Some(now));
    }

    #[test]
    fn cancelling_clears_snapshot_input_and_pending_selection() {
        let now = Instant::now();
        let mut selection = Selection::default();
        selection.begin(entries(2));
        selection.push_digit('2', now);
        let pending_revision = selection.revision();
        selection.cancel();
        assert_ne!(selection.revision(), pending_revision);
        assert!(!selection.active());
        assert_eq!(selection.number(&Target::new("server", "session-2")), None);
        assert_eq!(selection.input(), "");
        assert_eq!(selection.deadline(), None);
        assert_eq!(selection.finish_if_ready(now + Duration::from_secs(2)), None);
        assert_eq!(selection.confirm(), None);
        assert!(!selection.push_digit('1', now));
        selection.begin(entries(1));
        selection.push_digit('1', now);
        assert_eq!(selection.confirm().unwrap().target, Target::new("server", "session-1"));
    }

    #[test]
    fn snapshot_keeps_order_machine_and_incarnation_until_released() {
        let mut live = vec![
            Entry { target: Target::new("server-a", "same-name"), incarnation: Some("first-a".into()) },
            Entry { target: Target::new("server-b", "same-name"), incarnation: Some("first-b".into()) },
        ];
        let mut selection = Selection::default();
        selection.begin(live.clone());
        live.swap(0, 1);
        live[0].incarnation = Some("replacement-b".into());
        assert_eq!(selection.number(&Target::new("server-a", "same-name")), Some(1));
        assert_eq!(selection.number(&Target::new("server-b", "same-name")), Some(2));
        selection.push_digit('2', Instant::now());
        let selected = selection.confirm().unwrap();
        assert_eq!(selected.target, Target::new("server-b", "same-name"));
        assert_eq!(selected.incarnation.as_deref(), Some("first-b"));
        assert!(selection.active());
    }

    #[test]
    fn invalid_input_never_falls_back_to_another_session() {
        let now = Instant::now();
        let mut selection = Selection::default();
        assert!(!selection.push_digit('1', now));
        selection.begin(entries(2));
        let revision = selection.revision();
        assert!(!selection.push_digit('x', now));
        assert_eq!(selection.revision(), revision);
        assert_eq!(selection.deadline(), None);
        for input in ["0", "3", "999999999999999999999999999999999999999999999999"] {
            for digit in input.chars() { assert!(selection.push_digit(digit, now)); }
            assert_eq!(selection.confirm(), None);
            assert_eq!(selection.input(), "");
            assert_eq!(selection.deadline(), None);
            assert!(selection.active());
        }
    }

    #[test]
    fn backspace_edits_number_and_restarts_or_cancels_wait() {
        let now = Instant::now();
        let mut selection = Selection::default();
        selection.begin(entries(12));
        selection.push_digit('1', now);
        selection.push_digit('2', now);
        let revision = selection.revision();
        selection.backspace(now + Duration::from_millis(600));
        assert_ne!(selection.revision(), revision);
        assert_eq!(selection.input(), "1");
        assert_eq!(selection.deadline(), Some(now + Duration::from_millis(600) + INPUT_WAIT));
        assert_eq!(selection.finish_if_ready(now + Duration::from_millis(1000)), None);
        selection.backspace(now + Duration::from_millis(900));
        assert_eq!(selection.input(), "");
        assert_eq!(selection.deadline(), None);
        assert_eq!(selection.finish_if_ready(now + Duration::from_secs(3)), None);
    }

    #[test]
    fn confirming_before_deadline_keeps_numbers_and_starts_next_input_fresh() {
        let now = Instant::now();
        let mut selection = Selection::default();
        selection.begin(entries(12));
        selection.push_digit('1', now);
        selection.push_digit('2', now);
        assert_eq!(selection.confirm().unwrap().target, Target::new("server", "session-12"));
        assert!(selection.active());
        assert_eq!(selection.deadline(), None);
        selection.push_digit('1', now);
        assert_eq!(selection.input(), "1");
        assert_eq!(selection.confirm().unwrap().target, Target::new("server", "session-1"));
    }
}
