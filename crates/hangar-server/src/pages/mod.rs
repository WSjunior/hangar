//! Páginas HTML que o agente publica na conversa: guardadas por sessão, com tema e ponte injetados.
pub mod chrome;
pub mod images;
pub mod routes;
pub mod store;
pub mod theme;

pub const WIDTHS: [u32; 3] = [360, 728, 1000];
pub const HTML_MAX_CHARS: usize = 512_000;
pub const TITLE_MAX_CHARS: usize = 200;
pub const HEIGHT_MIN: u32 = 80;
pub const HEIGHT_MAX: u32 = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme { Dark, Light }

impl Theme {
    pub fn parse(s: &str) -> Option<Theme> {
        match s { "dark" => Some(Theme::Dark), "light" => Some(Theme::Light), _ => None }
    }
    pub fn as_str(self) -> &'static str { match self { Theme::Dark => "dark", Theme::Light => "light" } }
}
