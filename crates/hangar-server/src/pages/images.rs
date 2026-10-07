//! Imagem local citada por caminho absoluto vira `data:`; só entra o que for imagem pelos primeiros bytes.
use base64::Engine as _;
use regex::{Captures, Regex};
use std::collections::HashMap;
use std::sync::LazyLock;

pub const IMAGE_MAX: u64 = 10 * 1024 * 1024;
pub const PAGE_MAX: usize = 25 * 1024 * 1024;

#[derive(Debug)]
pub enum InlineError { Missing(String), NotImage(String), TooLarge(String), PageTooLarge }

impl InlineError {
    pub fn code(&self) -> &'static str {
        match self {
            InlineError::Missing(_) => "erro_pagina_imagem_ausente",
            InlineError::NotImage(_) => "erro_pagina_nao_e_imagem",
            InlineError::TooLarge(_) => "erro_pagina_imagem_grande",
            InlineError::PageTooLarge => "erro_pagina_grande",
        }
    }
    pub fn path(&self) -> Option<&str> {
        match self { InlineError::Missing(p) | InlineError::NotImage(p) | InlineError::TooLarge(p) => Some(p), _ => None }
    }
}

pub struct Inlined { pub html: String, pub missing: Vec<String> }

// Caminho absoluto inteiro entre aspas, ou em url( ) sem aspas. Extensão de imagem obrigatória;
// `//host/...` é URL sem protocolo, não caminho local. `C:\` e `C:/` cobrem o servidor no Windows.
static PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(?P<q>["'])(?P<p>(?:/[^/"'<>\s]|[a-z]:[\\/])[^"'<>\s]*?\.(?:png|jpe?g|gif|webp|avif|svg))["']|url\((?P<u>(?:/[^/)"'\s]|[a-z]:[\\/])[^)"'\s]*?\.(?:png|jpe?g|gif|webp|avif|svg))\)"#).unwrap()
});

fn mime(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        [_, _, _, _, b'f', b't', b'y', b'p', b'a', b'v', b'i', b'f', ..] => Some("image/avif"),
        _ => {
            let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
            (head.trim_start().starts_with("<svg") || (head.contains("<?xml") && head.contains("<svg"))).then_some("image/svg+xml")
        }
    }
}

fn load(path: &str) -> Result<String, InlineError> {
    let meta = std::fs::metadata(path).map_err(|_| InlineError::Missing(path.into()))?;
    // FIFO ou dispositivo com nome de imagem travaria a leitura.
    if !meta.is_file() { return Err(InlineError::Missing(path.into())); }
    if meta.len() > IMAGE_MAX { return Err(InlineError::TooLarge(path.into())); }
    let bytes = std::fs::read(path).map_err(|_| InlineError::Missing(path.into()))?;
    let kind = mime(&bytes).ok_or_else(|| InlineError::NotImage(path.into()))?;
    Ok(format!("data:{kind};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes)))
}

/// O total é contado a cada imagem: o mesmo arquivo citado milhares de vezes estouraria a memória
/// antes de uma checagem só no fim. Cada caminho é lido uma vez por chamada.
fn replace(html: &str, max: usize, mut on_error: impl FnMut(InlineError) -> Option<InlineError>) -> Result<String, InlineError> {
    let mut failed = None;
    let mut too_large = false;
    let mut total = html.len();
    let mut cache: HashMap<String, Option<String>> = HashMap::new();
    let out = PATH.replace_all(html, |c: &Captures| {
        if failed.is_some() || too_large { return c[0].to_owned(); }
        let (path, quoted) = match (c.name("p"), c.name("u")) { (Some(p), _) => (p.as_str(), true), (_, Some(u)) => (u.as_str(), false), _ => unreachable!() };
        if !cache.contains_key(path) {
            let data = load(path).map_err(|e| failed = on_error(e)).ok();
            cache.insert(path.to_owned(), data);
        }
        let Some(data) = cache[path].as_deref() else { return c[0].to_owned() };
        total += data.len();
        if total > max { too_large = true; return c[0].to_owned(); }
        if quoted { let q = &c["q"]; format!("{q}{data}{q}") } else { format!("url({data})") }
    }).into_owned();
    if let Some(e) = failed { return Err(e); }
    if too_large || out.len() > max { return Err(InlineError::PageTooLarge); }
    Ok(out)
}

/// Publicação: qualquer imagem que falte ou não seja imagem recusa a página inteira.
pub fn inline(html: &str) -> Result<String, InlineError> { replace(html, PAGE_MAX, Some) }

/// Rascunho: o que falta vira lista e a página segue; só o tamanho total recusa.
pub fn scan(html: &str) -> Result<Inlined, InlineError> {
    let mut missing = Vec::new();
    let html = replace(html, PAGE_MAX, |e| { if let Some(p) = e.path() { missing.push(p.to_owned()); } None })?;
    Ok(Inlined { html, missing })
}

#[cfg(test)]
mod tests {
    use super::*;
    const PNG_1PX: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];

    #[test]
    fn inlines_quoted_and_css_url() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.png");
        std::fs::write(&p, PNG_1PX).unwrap();
        let html = format!("<img src=\"{0}\"><div style=\"background:url({0})\"></div>", p.display());
        let out = inline(&html).unwrap();
        assert_eq!(out.matches("data:image/png;base64,").count(), 2);
        assert!(!out.contains(&p.display().to_string()));
    }

    #[test]
    fn rejects_non_image_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("id_ed25519.png");
        std::fs::write(&p, b"texto qualquer que nao e imagem").unwrap();
        let err = inline(&format!("<img src=\"{}\">", p.display())).unwrap_err();
        assert!(matches!(err, InlineError::NotImage(_)));
    }

    #[test]
    fn scan_reports_missing_without_failing() {
        let out = scan("<img src=\"/nao/existe.png\">").unwrap();
        assert_eq!(out.missing, vec!["/nao/existe.png".to_owned()]);
    }

    #[test]
    fn recognizes_windows_paths() {
        let out = scan("<img src=\"C:\\dir\\a.png\"><div style=\"background:url(D:/x/b.jpg)\"></div>").unwrap();
        assert_eq!(out.missing, vec!["C:\\dir\\a.png".to_owned(), "D:/x/b.jpg".to_owned()]);
    }

    #[test]
    fn leaves_remote_urls() {
        let html = "<script src=\"https://cdn.example/x.js\"></script><img src=\"//cdn.example/x.png\">";
        assert_eq!(inline(html).unwrap(), html);
    }

    #[test]
    fn repeated_image_stops_at_page_limit() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.png");
        std::fs::write(&p, PNG_1PX).unwrap();
        let html = format!("<img src=\"{}\">", p.display()).repeat(10);
        let err = replace(&html, html.len() + 100, Some).unwrap_err();
        assert!(matches!(err, InlineError::PageTooLarge));
    }
}
