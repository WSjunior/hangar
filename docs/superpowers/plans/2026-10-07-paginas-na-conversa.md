# Páginas na conversa — plano de implementação

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** o agente (Claude ou Codex) publica uma página HTML com a tool `html_render` do MCP `hangar` e ela aparece viva dentro da conversa no desktop nativo (Linux) e no PWA; estática no macOS; Windows depois de uma prova; Expo por último.

**Architecture:** o `hangar-server` (Rust) guarda a página por sessão em `~/.hangar/paginas/<chave>/`, injeta tema e script de ponte, embute imagens locais e usa um Chromium sem janela para medir altura e gerar print. O MCP Python só repassa à rota privada do Rust. Os apps reconhecem o `tool_result` com `hangar_page` e desenham a página: iframe `srcdoc` isolado no PWA, alvo CDP próprio pintado pela GPUI no nativo Linux.

**Tech Stack:** Rust (axum, tokio, tokio-tungstenite, ring, base64), Python 3.14 (MCP `hangar`), Svelte 5 + Paraglide, TypeScript (`packages/core`), GPUI Kit (`desktop-native/`), Expo + `react-native-webview`.

**Spec:** [`docs/superpowers/specs/2026-10-07-paginas-na-conversa-design.md`](../specs/2026-10-07-paginas-na-conversa-design.md) — ler inteira antes de qualquer Task.

## Global Constraints

- Worktree `.claude/worktrees/html-na-conversa`, branch `feat/html-na-conversa`. Nunca escrever no checkout principal `/home/jefferson/Projetos/hangar` (backend vivo do Jefferson roda dali).
- Identificador novo em inglês; comentário e texto de tela em português. Comentário curto, só o porquê, sem data nem medição.
- Texto de tela: PWA/Expo por `m.<chave>()`; nativo por `crate::i18n::tr("<chave>")`. Chaves novas em `messages/pt.json` e `messages/en.json` no mesmo commit.
- Toda tela trata carregando, erro, expirou e sucesso (vazio não existe: HTML vazio é recusado na rota).
- Nunca `allow-same-origin` no iframe; nunca o token na URL do documento da página.
- `RUST_SERVER_PROTOCOL` (`backend/app/rust_server.py`) e `INTERNAL_PROTOCOL` (`crates/hangar-server/src/lib.rs`) sobem juntos de 36 para 37, no mesmo commit.
- Limites: HTML 1–512 000 caracteres; título 1–200; `height` 80–2000; imagem 10 MiB; página embutida 25 MiB; Chromium do servidor 8 s, no máximo 2 simultâneos; larguras medidas `360, 728, 1000`; nativo no máximo 4 páginas vivas.
- Testes automatizados: escrever conforme os Steps, **rodar só quando o Jefferson pedir** (regra do `CLAUDE.md`). A verificação de cada Task é o uso real descrito no Step de verificação manual. `cargo check`/`cargo build` só no fim de Task do nativo ou do Rust, sempre com `nice -n 19`. Nada de tela escondida, monitor virtual ou ydotool.
- Nativo: usar a skill `gpui-kit` antes de escrever código GPUI. Build do nativo sai em `~/.cache/cargo/target`.
- Commits por caminho explícito (`git add <arquivo>`), mensagem descritiva em inglês (`feat(pages): …`). Sem push.

## Review Focus

1. **Página que muda de altura depois de carregar** (gráfico que desenha em `requestAnimationFrame`, fonte web que chega depois): a moldura acompanha sem pular a conversa e, se o leitor estava no fim, continua no fim. Testes: Task 5 (`frameHeight`), Task 6 Step 25.
2. **Sessão encerrada com mensagem antiga na tela**: o cartão mostra "expirou", não erro genérico nem caixa vazia. Testes: Task 3 Step 12 (404), Task 5 (`pageFetchState`).
3. **HTML com `<head>` dentro de comentário ou de `<script>`**: o tema entra no `<head>` real, não dentro de texto inerte. Teste: Task 1 Step 2 (`inject_into_commented_head`).
4. **Caminho de imagem que aponta para arquivo que não é imagem** (um arquivo de texto renomeado para `.png`): recusado. Teste: Task 1 Step 2 (`rejects_non_image_bytes`).
5. **Servidor sem Chromium** (VPS, macOS sem Chrome): publicar funciona, rascunho diz `browser: "ausente"`, `/shot` responde 404 com código, o cartão estático mostra só título e botão. Testes: Task 2 Step 7, Task 3 Step 12.

---

### Task 1: Núcleo das páginas no Rust (tema, ponte, imagens, armazenamento)
Status: ready-for-agent
Risk: high

**Files:**
- Create: `crates/hangar-server/src/pages/mod.rs`
- Create: `crates/hangar-server/src/pages/theme.rs`
- Create: `crates/hangar-server/src/pages/images.rs`
- Create: `crates/hangar-server/src/pages/store.rs`
- Modify: `crates/hangar-server/src/lib.rs` (linha `pub mod pages;` junto dos outros `pub mod`)

**Interfaces:**
- Produces:
  - `pages::theme::inject(html: &str) -> String`
  - `pages::images::inline(html: &str) -> Result<String, InlineError>` e `pages::images::scan(html: &str) -> Inlined { html: String, missing: Vec<String> }`
  - `pages::store::Store::new(root: PathBuf) -> Store`, `Store::default_root() -> PathBuf`, `Store::save(&self, key: &str, jsonl: &str, page: &NewPage) -> io::Result<String>` (devolve o id), `Store::html(&self, key: &str, id: &str) -> Option<String>`, `Store::meta(&self, key: &str, id: &str) -> Option<PageMeta>`, `Store::shot_path(&self, key: &str, id: &str, theme: Theme, width: u32) -> PathBuf`, `Store::sweep(&self, live_jsonl: &HashSet<String>, now: SystemTime)`
  - `pub struct NewPage { pub html: String, pub title: String, pub height: Option<u32>, pub heights: BTreeMap<u32, u32>, pub draft: bool }`
  - `pub struct PageMeta { pub title: String, pub height: Option<u32>, pub heights: BTreeMap<u32, u32>, pub created: u64, pub draft: bool }` (serde)
  - `pub enum Theme { Dark, Light }` com `Theme::parse(&str) -> Option<Theme>` e `as_str()`
  - `pub const WIDTHS: [u32; 3] = [360, 728, 1000];`

- [ ] **Step 1: Criar `pages/mod.rs` e registrar o módulo**

```rust
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
```

`chrome` e `routes` nascem nas Tasks 2 e 3; até lá, crie os dois arquivos vazios com só a linha `//! (Task 2)` / `//! (Task 3)` para o módulo compilar. Em `lib.rs`, acrescente `pub mod pages;` em ordem alfabética entre `pub mod mods;` e `pub mod proxy;`.

- [ ] **Step 2: Escrever os testes de tema e imagens**

No fim de `theme.rs` e `images.rs`:

```rust
// theme.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inject_goes_first_in_head() {
        let out = inject("<!doctype html><html><head><style>p{}</style></head><body>x</body></html>");
        let head = out.find("<head>").unwrap();
        let ours = out.find("<style id=\"hangar-theme\">").unwrap();
        let theirs = out.find("<style>p{}").unwrap();
        assert!(head < ours && ours < theirs);
        assert!(out.contains("<script id=\"hangar-host\">"));
    }

    #[test]
    fn inject_into_commented_head() {
        let out = inject("<!-- <head> --><html><head></head><body></body></html>");
        let comment_end = out.find("-->").unwrap();
        assert!(out.find("hangar-theme").unwrap() > comment_end);
    }

    #[test]
    fn inject_without_head_or_html() {
        let out = inject("<p>oi</p>");
        assert!(out.starts_with("<head>"));
        assert!(out.ends_with("<p>oi</p>"));
    }

    #[test]
    fn background_is_transparent_in_both_schemes() {
        let out = inject("<p></p>");
        assert_eq!(out.matches("--background:transparent").count(), 2);
    }
}

// images.rs
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
        let out = scan("<img src=\"/nao/existe.png\">");
        assert_eq!(out.missing, vec!["/nao/existe.png".to_owned()]);
    }

    #[test]
    fn leaves_remote_urls() {
        let html = "<script src=\"https://cdn.example/x.js\"></script>";
        assert_eq!(inline(html).unwrap(), html);
    }
}
```

- [ ] **Step 3: Implementar `theme.rs`**

```rust
//! Tema e script de ponte que vão no começo do `<head>` de toda página publicada.
use regex::Regex;
use std::sync::LazyLock;

const DARK: &str = "--background:transparent;--foreground:#d2cbcd;--muted-foreground:#8d8489;--surface:#1a171a;\
--border:rgba(255,248,244,0.12);--accent:#7c87e8;--accent-foreground:#ffffff;--danger:#ff453a;--warning:#ff9f0a;\
--success:#34c759;--code-background:#100e11;--chart-1:#7c87e8;--chart-2:#d95926;--chart-3:#199e70;--chart-4:#c98500;";
const LIGHT: &str = "--background:transparent;--foreground:#221d1b;--muted-foreground:#6f6660;--surface:#fffdfa;\
--border:rgba(50,40,35,0.14);--accent:#5b6ad0;--accent-foreground:#ffffff;--danger:#b3251c;--warning:#7a5408;\
--success:#146c30;--code-background:#f8f6f2;--chart-1:#5b6ad0;--chart-2:#eb6834;--chart-3:#1baf7a;--chart-4:#eda100;";
const COMMON: &str = "--radius:10px;--font-sans:-apple-system,BlinkMacSystemFont,\"SF Pro Text\",system-ui,sans-serif;\
--font-mono:\"JetBrains Mono\",ui-monospace,\"SF Mono\",monospace;";
const BASE: &str = "html{background:var(--background);color:var(--foreground);font:14px/1.5 var(--font-sans);\
scrollbar-width:none}html::-webkit-scrollbar{display:none}body{margin:0}";

/// Ponte com o app: tamanho, link e troca de tema, no formato JSON-RPC do MCP Apps.
const HOST: &str = r#"(()=>{const send=m=>{const s=JSON.stringify(m);if(window.hangarHost)window.hangarHost(s);
else if(window.ReactNativeWebView)window.ReactNativeWebView.postMessage(s);else if(window.parent!==window)window.parent.postMessage(m,"*")};
let last=0;const size=()=>{const d=document.documentElement,b=document.body;if(!d)return;
const h=Math.ceil(d.scrollHeight>d.clientHeight?d.scrollHeight:Math.max(d.getBoundingClientRect().height,b?b.getBoundingClientRect().height:0));
if(h!==last){last=h;send({jsonrpc:"2.0",method:"ui/notifications/size-changed",params:{height:h}})}};
const watch=()=>{const r=new ResizeObserver(size);r.observe(document.documentElement);if(document.body)r.observe(document.body);size()};
document.readyState==="loading"?document.addEventListener("DOMContentLoaded",watch):watch();addEventListener("load",size);
document.addEventListener("click",e=>{const a=e.target.closest&&e.target.closest("a[href]");if(!a)return;
const u=a.href;if(!/^https?:/i.test(u))return;e.preventDefault();send({jsonrpc:"2.0",method:"ui/open-link",params:{url:u}})},true);
const apply=p=>{const v=(p&&p.styles&&p.styles.variables)||{};const t=p&&p.theme==="light"?"light":"dark";
const css=Object.entries(v).filter(([k])=>/^--[a-z0-9-]+$/.test(k)).map(([k,x])=>k+":"+String(x).replace(/[;{}<]/g,"")).join(";");
const el=document.getElementById("hangar-theme");if(el)el.textContent=":root{color-scheme:"+t+";"+css+"}"+el.dataset.base};
window.__hangarApply=apply;addEventListener("message",e=>{const d=e.data;if(d&&d.method==="ui/notifications/host-context-changed")apply(d.params)})})();"#;

static HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<head(\s[^>]*)?>").unwrap());
static HTML: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<html(\s[^>]*)?>").unwrap());
static INERT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<!--.*?-->|<script\b.*?</script\s*>|<style\b.*?</style\s*>|<template\b.*?</template\s*>").unwrap()
});

/// `data-base` guarda o que a troca de tema do app não reescreve (raio, fontes e o CSS base).
fn block() -> String {
    let css = format!(":root{{color-scheme:dark;{DARK}{COMMON}}}@media (prefers-color-scheme: light){{:root{{color-scheme:light;{LIGHT}}}}}{BASE}");
    let data_base = format!(":root{{{COMMON}}}{BASE}").replace('"', "&quot;");
    format!("<style id=\"hangar-theme\" data-base=\"{data_base}\">{css}</style><script id=\"hangar-host\">{HOST}</script>")
}

/// Posições cobertas por comentário, script, style ou template: um `<head>` ali dentro não conta.
fn inert_ranges(html: &str) -> Vec<(usize, usize)> {
    INERT.find_iter(html).map(|m| (m.start(), m.end())).collect()
}

fn first_live(re: &Regex, html: &str, inert: &[(usize, usize)]) -> Option<usize> {
    re.find_iter(html).find(|m| !inert.iter().any(|(a, b)| m.start() >= *a && m.start() < *b)).map(|m| m.end())
}

pub fn inject(html: &str) -> String {
    let inert = inert_ranges(html);
    let ours = block();
    if let Some(at) = first_live(&HEAD, html, &inert) {
        return format!("{}{ours}{}", &html[..at], &html[at..]);
    }
    if let Some(at) = first_live(&HTML, html, &inert) {
        return format!("{}<head>{ours}</head>{}", &html[..at], &html[at..]);
    }
    format!("<head>{ours}</head>{html}")
}
```

A troca de tema do app (script `HOST`, função `apply`) escreve `:root{color-scheme:<tema>;<variáveis>}` seguido do `data-base`; como o bloco fica no começo do `<head>`, regras `:root` da página continuam vencendo.

- [ ] **Step 4: Implementar `images.rs`**

```rust
//! Imagem local citada por caminho absoluto vira `data:`; só entra o que for imagem pelos primeiros bytes.
use base64::Engine as _;
use regex::{Captures, Regex};
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

// Caminho absoluto inteiro entre aspas, ou em url( ) sem aspas. Extensão de imagem obrigatória.
static PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(?P<q>["'])(?P<p>/[^"'<>\s]+?\.(?:png|jpe?g|gif|webp|avif|svg))["']|url\((?P<u>/[^)"'\s]+?\.(?:png|jpe?g|gif|webp|avif|svg))\)"#).unwrap()
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
    if meta.len() > IMAGE_MAX { return Err(InlineError::TooLarge(path.into())); }
    let bytes = std::fs::read(path).map_err(|_| InlineError::Missing(path.into()))?;
    let kind = mime(&bytes).ok_or_else(|| InlineError::NotImage(path.into()))?;
    Ok(format!("data:{kind};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes)))
}

fn replace(html: &str, mut on_error: impl FnMut(InlineError) -> Option<InlineError>) -> Result<String, InlineError> {
    let mut failed = None;
    let out = PATH.replace_all(html, |c: &Captures| {
        let (path, quoted) = match (c.name("p"), c.name("u")) { (Some(p), _) => (p.as_str(), true), (_, Some(u)) => (u.as_str(), false), _ => unreachable!() };
        match load(path) {
            Ok(data) if quoted => { let q = &c["q"]; format!("{q}{data}{q}") }
            Ok(data) => format!("url({data})"),
            Err(e) => { if failed.is_none() { failed = on_error(e); } c[0].to_owned() }
        }
    }).into_owned();
    if let Some(e) = failed { return Err(e); }
    if out.len() > PAGE_MAX { return Err(InlineError::PageTooLarge); }
    Ok(out)
}

/// Publicação: qualquer imagem que falte ou não seja imagem recusa a página inteira.
pub fn inline(html: &str) -> Result<String, InlineError> { replace(html, Some) }

/// Rascunho: o que falta vira lista, a página segue.
pub fn scan(html: &str) -> Inlined {
    let mut missing = Vec::new();
    let html = replace(html, |e| { if let Some(p) = e.path() { missing.push(p.to_owned()); } None })
        .unwrap_or_else(|_| html.to_owned());
    Inlined { html, missing }
}
```

- [ ] **Step 5: Implementar `store.rs` com teste**

```rust
//! Pasta por sessão em `~/.hangar/paginas/<chave>/`: `<id>.html`, `<id>.json`, prints e o `jsonl` do dono.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::Theme;

const GONE_AFTER: Duration = Duration::from_secs(60);
const DRAFT_TTL: Duration = Duration::from_secs(3600);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PageMeta { pub title: String, pub height: Option<u32>, pub heights: BTreeMap<u32, u32>, pub created: u64, pub draft: bool }

pub struct NewPage { pub html: String, pub title: String, pub height: Option<u32>, pub heights: BTreeMap<u32, u32>, pub draft: bool }

pub struct Store { root: PathBuf, absent_since: Mutex<HashMap<String, SystemTime>> }

fn valid(part: &str) -> bool { !part.is_empty() && part.len() <= 128 && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') }

fn new_id() -> String {
    use ring::rand::SecureRandom;
    let mut b = [0u8; 16];
    ring::rand::SystemRandom::new().fill(&mut b).expect("gerador do sistema");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

impl Store {
    pub fn new(root: PathBuf) -> Store { Store { root, absent_since: Mutex::new(HashMap::new()) } }

    pub fn default_root() -> PathBuf {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
        home.join(".hangar").join("paginas")
    }

    fn dir(&self, key: &str) -> Option<PathBuf> { valid(key).then(|| self.root.join(key)) }

    pub fn save(&self, key: &str, jsonl: &str, page: &NewPage) -> io::Result<String> {
        let dir = self.dir(key).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "chave inválida"))?;
        std::fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?; }
        std::fs::write(dir.join("jsonl"), jsonl)?;
        let id = new_id();
        let created = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let meta = PageMeta { title: page.title.clone(), height: page.height, heights: page.heights.clone(), created, draft: page.draft };
        write_atomic(&dir.join(format!("{id}.html")), page.html.as_bytes())?;
        write_atomic(&dir.join(format!("{id}.json")), &serde_json::to_vec(&meta)?)?;
        Ok(id)
    }

    pub fn html(&self, key: &str, id: &str) -> Option<String> {
        if !valid(id) { return None; }
        std::fs::read_to_string(self.dir(key)?.join(format!("{id}.html"))).ok()
    }

    pub fn meta(&self, key: &str, id: &str) -> Option<PageMeta> {
        if !valid(id) { return None; }
        serde_json::from_slice(&std::fs::read(self.dir(key)?.join(format!("{id}.json"))).ok()?).ok()
    }

    pub fn set_heights(&self, key: &str, id: &str, heights: BTreeMap<u32, u32>) -> io::Result<()> {
        let mut meta = self.meta(key, id).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        meta.heights = heights;
        write_atomic(&self.root.join(key).join(format!("{id}.json")), &serde_json::to_vec(&meta)?)
    }

    pub fn shot_path(&self, key: &str, id: &str, theme: Theme, width: u32) -> PathBuf {
        self.root.join(key).join(format!("{id}.{}.{width}.png", theme.as_str()))
    }

    /// Apaga a pasta cujo transcript não está vivo há `GONE_AFTER`, e rascunhos com mais de `DRAFT_TTL`.
    pub fn sweep(&self, live_jsonl: &HashSet<String>, now: SystemTime) {
        let Ok(entries) = std::fs::read_dir(&self.root) else { return };
        let mut absent = self.absent_since.lock().unwrap_or_else(|e| e.into_inner());
        let mut seen = HashSet::new();
        for entry in entries.flatten() {
            let key = entry.file_name().to_string_lossy().into_owned();
            let dir = entry.path();
            let owner = std::fs::read_to_string(dir.join("jsonl")).unwrap_or_default();
            seen.insert(key.clone());
            if live_jsonl.contains(owner.trim()) { absent.remove(&key); self.drop_old_drafts(&dir, now); continue; }
            let since = *absent.entry(key.clone()).or_insert(now);
            if now.duration_since(since).unwrap_or_default() >= GONE_AFTER {
                if let Err(e) = std::fs::remove_dir_all(&dir) { tracing::warn!(key = %key, "páginas da sessão não apagadas: {e}"); }
                absent.remove(&key);
            }
        }
        absent.retain(|k, _| seen.contains(k));
    }

    fn drop_old_drafts(&self, dir: &std::path::Path, now: SystemTime) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") { continue; }
            let Some(meta) = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice::<PageMeta>(&b).ok()) else { continue };
            let age = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs().saturating_sub(meta.created);
            if meta.draft && age >= DRAFT_TTL.as_secs() {
                let id = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_owned();
                for f in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                    if f.file_name().to_string_lossy().starts_with(&format!("{id}.")) { let _ = std::fs::remove_file(f.path()); }
                }
            }
        }
    }
}

pub(crate) fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> NewPage { NewPage { html: "<p>x</p>".into(), title: "t".into(), height: None, heights: BTreeMap::new(), draft: false } }

    #[test]
    fn save_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().into());
        let id = s.save("abc-1", "/t/abc-1.jsonl", &page()).unwrap();
        assert_eq!(s.html("abc-1", &id).unwrap(), "<p>x</p>");
        assert_eq!(s.meta("abc-1", &id).unwrap().title, "t");
    }

    #[test]
    fn rejects_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().into());
        assert!(s.save("../x", "/t", &page()).is_err());
        assert!(s.html("abc", "../../etc/passwd").is_none());
    }

    #[test]
    fn sweep_waits_before_deleting() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().into());
        s.save("k", "/t/k.jsonl", &page()).unwrap();
        let t0 = SystemTime::now();
        s.sweep(&HashSet::new(), t0);
        assert!(dir.path().join("k").exists());
        s.sweep(&HashSet::new(), t0 + GONE_AFTER + Duration::from_secs(1));
        assert!(!dir.path().join("k").exists());
    }

    #[test]
    fn sweep_keeps_live_session() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().into());
        s.save("k", "/t/k.jsonl", &page()).unwrap();
        let live: HashSet<String> = ["/t/k.jsonl".to_owned()].into();
        let t0 = SystemTime::now();
        s.sweep(&live, t0);
        s.sweep(&live, t0 + GONE_AFTER * 2);
        assert!(dir.path().join("k").exists());
    }
}
```

- [ ] **Step 6: Commit**

```bash
git add crates/hangar-server/src/pages/ crates/hangar-server/src/lib.rs
git commit -m "feat(pages): page store with theme bridge and local image inlining"
```

### Task 2: Chromium do servidor (altura, console, print)
Status: ready-for-agent
Risk: high

**Files:**
- Modify: `crates/hangar-server/src/pages/chrome.rs`
- Modify: `crates/hangar-server/Cargo.toml` (mover `tokio-tungstenite = "=0.29.0"` de `[dev-dependencies]` para `[dependencies]`)

**Interfaces:**
- Consumes: `pages::{Theme, WIDTHS}` (Task 1)
- Produces:
  - `pages::chrome::find() -> Option<PathBuf>`
  - `pages::chrome::render(html: &str, jobs: &[Job]) -> Result<Rendered, ChromeError>` (async)
  - `pub struct Job { pub width: u32, pub theme: Theme, pub shot: Option<PathBuf> }`
  - `pub struct Rendered { pub heights: BTreeMap<u32, u32>, pub console: Vec<ConsoleLine> }`
  - `#[derive(Serialize)] pub struct ConsoleLine { pub level: String, pub text: String }`
  - `pub enum ChromeError { Absent, Failed(&'static str) }` com `fn status(&self) -> &'static str` (`"ausente"`/`"falhou"`)

- [ ] **Step 7: Escrever os testes de `find` e de ausência**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_override_wins() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("chrome");
        std::fs::write(&bin, "").unwrap();
        assert_eq!(find_with(Some(bin.clone().into_os_string()), None, &[]), Some(bin));
    }

    #[test]
    fn marker_points_to_binary() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("c");
        std::fs::write(&bin, "").unwrap();
        let marker = dir.path().join("chromium-ok");
        std::fs::write(&marker, format!("{}\n", bin.display())).unwrap();
        assert_eq!(find_with(None, Some(marker), &[]), Some(bin));
    }

    #[test]
    fn nothing_found_is_absent() {
        assert_eq!(find_with(None, None, &[]), None);
    }
}
```

A marca do `install-chromium.sh` grava o caminho do binário ou uma palavra (`sistema:<caminho>`). Antes de implementar, leia `scripts/install-chromium.sh` (função `marca` e as chamadas dela) e trate exatamente os formatos que ele grava: a linha é aceita se, depois de tirar um prefixo `<palavra>:` opcional, for um caminho de arquivo existente.

- [ ] **Step 8: Implementar `chrome.rs`**

```rust
//! Chromium sem janela do servidor: mede a altura da página, guarda o console e tira print.
//! Porta de depuração em vez de pipe: o pipe por fd 3/4 não existe no Windows.
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio_tungstenite::tungstenite::Message;

use super::Theme;

const DEADLINE: Duration = Duration::from_secs(8);
static SLOTS: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(2));

pub struct Job { pub width: u32, pub theme: Theme, pub shot: Option<PathBuf> }
pub struct Rendered { pub heights: BTreeMap<u32, u32>, pub console: Vec<ConsoleLine> }
#[derive(Serialize, Clone, Debug)]
pub struct ConsoleLine { pub level: String, pub text: String }
#[derive(Debug)]
pub enum ChromeError { Absent, Failed(&'static str) }
impl ChromeError {
    pub fn status(&self) -> &'static str { match self { ChromeError::Absent => "ausente", ChromeError::Failed(_) => "falhou" } }
    pub fn reason(&self) -> &'static str { match self { ChromeError::Absent => "sem Chromium no servidor", ChromeError::Failed(r) => r } }
}

#[cfg(target_os = "macos")]
const KNOWN: &[&str] = &["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/Applications/Chromium.app/Contents/MacOS/Chromium"];
#[cfg(target_os = "windows")]
const KNOWN: &[&str] = &[r"C:\Program Files\Google\Chrome\Application\chrome.exe", r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const KNOWN: &[&str] = &[];
const ON_PATH: &[&str] = &["google-chrome-stable", "google-chrome", "chromium", "chromium-browser"];

pub fn find() -> Option<PathBuf> {
    let marker = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".hangar/native/chromium-ok"));
    let mut known: Vec<PathBuf> = KNOWN.iter().map(PathBuf::from).collect();
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) { for n in ON_PATH { known.push(dir.join(n)); } }
    }
    find_with(std::env::var_os("CP_CHROMIUM_BIN"), marker, &known)
}

fn find_with(env: Option<OsString>, marker: Option<PathBuf>, known: &[PathBuf]) -> Option<PathBuf> {
    if let Some(p) = env.map(PathBuf::from).filter(|p| p.is_file()) { return Some(p); }
    if let Some(line) = marker.and_then(|m| std::fs::read_to_string(m).ok()) {
        let line = line.trim();
        let path = line.split_once(':').filter(|(w, _)| !w.contains('/') && !w.contains('\\')).map_or(line, |(_, p)| p);
        let p = PathBuf::from(path);
        if p.is_file() { return Some(p); }
    }
    known.iter().find(|p| p.is_file()).cloned()
}

pub async fn render(html: &str, jobs: &[Job]) -> Result<Rendered, ChromeError> {
    let bin = find().ok_or(ChromeError::Absent)?;
    let _slot = SLOTS.acquire().await.map_err(|_| ChromeError::Failed("fila do navegador fechada"))?;
    tokio::time::timeout(DEADLINE, run(&bin, html, jobs)).await.map_err(|_| ChromeError::Failed("navegador do servidor passou do prazo"))?
}

async fn run(bin: &Path, html: &str, jobs: &[Job]) -> Result<Rendered, ChromeError> {
    let profile = tempfile::tempdir().map_err(|_| ChromeError::Failed("perfil temporário"))?;
    let mut child = tokio::process::Command::new(bin)
        .args(["--headless", "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check", "--hide-scrollbars",
               "--disable-gpu", "--mute-audio", "--font-render-hinting=none"])
        .arg(format!("--user-data-dir={}", profile.path().display()))
        .arg("about:blank")
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn().map_err(|_| ChromeError::Failed("navegador do servidor não subiu"))?;
    let result = drive(profile.path(), html, jobs).await;
    let _ = child.kill().await;
    result
}

async fn ws_url(profile: &Path) -> Result<String, ChromeError> {
    let file = profile.join("DevToolsActivePort");
    for _ in 0..100 {
        if let Ok(text) = tokio::fs::read_to_string(&file).await {
            let mut lines = text.lines();
            if let (Some(port), Some(path)) = (lines.next(), lines.next()) { return Ok(format!("ws://127.0.0.1:{port}{path}")); }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(ChromeError::Failed("navegador do servidor não anunciou a porta"))
}
```

Continue no mesmo arquivo com o cliente CDP mínimo (`struct Cdp { ws, next_id, events: Vec<Value> }` com `async fn call(&mut self, session: Option<&str>, method: &str, params: Value) -> Result<Value, ChromeError>` que manda `{"id","method","params","sessionId"}` e lê mensagens até a resposta de mesmo `id`, guardando eventos `Runtime.consoleAPICalled` e `Runtime.exceptionThrown` em `events`) e `drive`:

```rust
async fn drive(profile: &Path, html: &str, jobs: &[Job]) -> Result<Rendered, ChromeError> {
    let url = ws_url(profile).await?;
    let (ws, _) = tokio_tungstenite::connect_async(&url).await.map_err(|_| ChromeError::Failed("CDP do servidor recusou a conexão"))?;
    let mut cdp = Cdp { ws, next_id: 0, events: Vec::new() };
    let target = cdp.call(None, "Target.createTarget", json!({"url": "about:blank"})).await?["targetId"].as_str().unwrap_or_default().to_owned();
    let session = cdp.call(None, "Target.attachToTarget", json!({"targetId": target, "flatten": true})).await?["sessionId"].as_str().unwrap_or_default().to_owned();
    let s = Some(session.as_str());
    cdp.call(s, "Page.enable", json!({})).await?;
    cdp.call(s, "Runtime.enable", json!({})).await?;
    cdp.call(s, "Fetch.enable", json!({"patterns": [{"urlPattern": "file:*"}]})).await?;
    cdp.call(s, "Emulation.setDefaultBackgroundColorOverride", json!({"color": {"r": 0, "g": 0, "b": 0, "a": 0}})).await?;
    let frame = cdp.call(s, "Page.getFrameTree", json!({})).await?["frameTree"]["frame"]["id"].as_str().unwrap_or_default().to_owned();
    cdp.call(s, "Page.setDocumentContent", json!({"frameId": frame, "html": html})).await?;
    let mut heights = BTreeMap::new();
    for job in jobs {
        cdp.call(s, "Emulation.setEmulatedMedia", json!({"features": [{"name": "prefers-color-scheme", "value": job.theme.as_str()}]})).await?;
        cdp.call(s, "Emulation.setDeviceMetricsOverride", json!({"width": job.width, "height": 600, "deviceScaleFactor": 2, "mobile": false})).await?;
        tokio::time::sleep(Duration::from_millis(150)).await;
        let h = cdp.call(s, "Runtime.evaluate", json!({"expression": "Math.ceil(document.documentElement.scrollHeight)", "returnByValue": true})).await?;
        let height = (h["result"]["value"].as_f64().unwrap_or(0.) as u32).clamp(super::HEIGHT_MIN, super::HEIGHT_MAX);
        heights.insert(job.width, height);
        if let Some(path) = &job.shot {
            let shot = cdp.call(s, "Page.captureScreenshot", json!({"format": "png", "captureBeyondViewport": true,
                "clip": {"x": 0, "y": 0, "width": job.width, "height": height, "scale": 1}})).await?;
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::STANDARD.decode(shot["data"].as_str().unwrap_or("")).map_err(|_| ChromeError::Failed("print inválido"))?;
            super::store::write_atomic(path, &bytes).map_err(|_| ChromeError::Failed("print não gravado"))?;
        }
    }
    Ok(Rendered { heights, console: console_lines(&cdp.events) })
}
```

`Fetch.requestPaused` de `file:` (evento em `events`) recebe `Fetch.failRequest {requestId, errorReason: "BlockedByClient"}` dentro do laço de leitura do `call`. `console_lines` mapeia `consoleAPICalled` (`type` → `level`; `args[].value`/`description` unidos por espaço) e `exceptionThrown` (`level: "error"`, `exceptionDetails.exception.description` ou `text`), no máximo 50 linhas de 500 caracteres.

- [ ] **Step 9: Verificação manual — medir uma página real**

Escreva um teste ignorado que só roda à mão, e rode-o uma vez nesta máquina (Chrome em `/usr/bin/google-chrome-stable`):

```rust
#[tokio::test]
#[ignore = "sobe o Chrome da máquina"]
async fn measures_real_page() {
    let dir = tempfile::tempdir().unwrap();
    let shot = dir.path().join("s.png");
    let r = render("<div style=\"height:333px\"></div><script>console.error('oi')</script>",
        &[Job { width: 728, theme: Theme::Dark, shot: Some(shot.clone()) }]).await.unwrap();
    assert_eq!(r.heights[&728], 333);
    assert!(r.console.iter().any(|c| c.level == "error" && c.text.contains("oi")));
    assert!(std::fs::metadata(&shot).unwrap().len() > 0);
}
```

Run: `cd crates && nice -n 19 cargo test -p hangar-server pages::chrome::tests::measures_real_page -- --ignored`
Expected: PASS. Abra o PNG gerado (caminho impresso com `dbg!(&shot)`) e confira que o fundo é transparente.

- [ ] **Step 10: Commit**

```bash
git add crates/hangar-server/src/pages/chrome.rs crates/hangar-server/Cargo.toml crates/Cargo.lock
git commit -m "feat(pages): server-side headless Chromium for height, console and screenshots"
```

### Task 3: Rotas das páginas e limpeza com a sessão
Status: ready-for-agent
Risk: high

**Files:**
- Modify: `crates/hangar-server/src/pages/routes.rs`
- Modify: `crates/hangar-server/src/workspace_routes.rs:973-1080` (extrair a casca isolada para `pub(crate) fn isolated_html(bytes: Vec<u8>, title: &str) -> Response`)
- Modify: `crates/hangar-server/src/routes.rs:210-261` (rota privada, rotas públicas, 404 da privada na pública)
- Modify: `crates/hangar-server/src/list/bridge.rs:464` (varredura ao lado de `prune_gone`)
- Modify: `crates/hangar-server/src/lib.rs:30` (`INTERNAL_PROTOCOL` 36 → 37)
- Modify: `crates/hangar-server/src/migration_status.rs` (contar o grupo `pages` como rota do Rust, no mesmo formato dos grupos existentes)

**Interfaces:**
- Consumes: `pages::store::{Store, NewPage}`, `pages::theme::inject`, `pages::images::{inline, scan}`, `pages::chrome::{render, Job}`, `routes::{AppState, gate, pass, fetch_info, route_failed}`
- Produces:
  - `POST /__hangar_server/pages` corpo `{"session": str, "html": str, "title": str, "height": u32?, "draft": bool?}` → 200 `{"ok": true, "result": {...}}` (publicado: `{"hangar_page": {id,title,height,heights}, "message": str}`; rascunho: `{"draft": {id,url,shot,heights,console,missing_images,browser}}`) ou `{"ok": false, "error": {"code", "detail"}}`
  - `GET /api/sessions/{name}/pages/{id}` (casca isolada), `?raw=1` (texto + CSP sandbox), `GET /api/sessions/{name}/pages/{id}/shot?theme=&width=` (PNG)
  - `AppState.pages: pages::store::Store` (campo novo, `Store::new(Store::default_root())`)

- [ ] **Step 11: Extrair a casca isolada**

Em `workspace_routes.rs`, mova a montagem do invólucro HTML (o `prefix` com o `<iframe sandbox="allow-scripts allow-popups" referrerpolicy="no-referrer" src="data:…;base64,` e o `suffix`) para uma função reutilizável que recebe os bytes inteiros (a página já está em memória):

```rust
/// Documento ativo isolado: o HTML roda numa origem opaca dentro de uma casca, sem ver a URL (com o token) dela.
pub(crate) fn isolated_html(bytes: &[u8], title: &str) -> Response {
    use base64::Engine;
    let title = html_escape(title);
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{title}</title><style>html,body{{margin:0;height:100%;overflow:hidden}}iframe{{display:block;width:100%;height:100%;border:0}}</style></head><body><iframe title=\"{title}\" sandbox=\"allow-scripts allow-popups\" referrerpolicy=\"no-referrer\" src=\"data:text/html;charset=utf-8;base64,{}\"></iframe></body></html>",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    );
    let mut r = Response::new(Body::from(body));
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, "text/html; charset=utf-8".parse().unwrap());
    h.insert("x-content-type-options", "nosniff".parse().unwrap());
    h.insert("referrer-policy", "no-referrer".parse().unwrap());
    h.insert(header::CACHE_CONTROL, "private, max-age=3600".parse().unwrap());
    r
}
```

O `serve_file` continua com o streaming dele (arquivo pode ser grande); só a string da casca passa a vir de uma constante compartilhada `ISOLATED_PREFIX`/`ISOLATED_SUFFIX` usada pelas duas, para as duas cascas nunca divergirem.

- [ ] **Step 12: Escrever os testes das rotas**

Em `pages/routes.rs`, testes com o padrão dos testes de rota existentes (procure `#[tokio::test]` em `crates/hangar-server/src/mods/routes.rs` ou `costs_routes.rs` e use o mesmo montador de `AppState` de teste e o Python falso que responde `/internal/sessions/{name}/info` com `{"provider":"claude","jsonl":"/t/k.jsonl","session_key":"k","history":...}`):

```rust
#[tokio::test]
async fn publish_then_get_raw_and_isolated() {
    let (st, _py) = test_state_with_info("s1", "k", "/t/k.jsonl").await;
    let res = publish(&st, json!({"session": "s1", "html": "<p>oi</p>", "title": "Oi"})).await;
    let id = res["result"]["hangar_page"]["id"].as_str().unwrap().to_owned();
    let raw = owner_get(&st, &format!("/api/sessions/s1/pages/{id}?raw=1")).await;
    assert_eq!(raw.status(), 200);
    assert_eq!(raw.headers()["content-security-policy"], "sandbox");
    assert!(body_text(raw).await.contains("hangar-theme"));
    let page = owner_get(&st, &format!("/api/sessions/s1/pages/{id}")).await;
    assert!(body_text(page).await.contains("src=\"data:text/html;charset=utf-8;base64,"));
}

#[tokio::test]
async fn unknown_page_is_404_with_code() {
    let (st, _py) = test_state_with_info("s1", "k", "/t/k.jsonl").await;
    let r = owner_get(&st, "/api/sessions/s1/pages/0123abcd").await;
    assert_eq!(r.status(), 404);
    assert!(body_text(r).await.contains("erro_pagina_expirou"));
}

#[tokio::test]
async fn empty_html_and_long_title_refused() {
    let (st, _py) = test_state_with_info("s1", "k", "/t/k.jsonl").await;
    assert_eq!(publish(&st, json!({"session": "s1", "html": "", "title": "x"})).await["error"]["code"], "erro_pagina_invalida");
    assert_eq!(publish(&st, json!({"session": "s1", "html": "<p></p>", "title": "x".repeat(201)})).await["error"]["code"], "erro_pagina_invalida");
}

#[tokio::test]
async fn shot_without_chromium_is_404_with_code() {
    std::env::set_var("CP_CHROMIUM_BIN", "/nao/existe");
    let (st, _py) = test_state_with_info("s1", "k", "/t/k.jsonl").await;
    let id = publish(&st, json!({"session": "s1", "html": "<p>oi</p>", "title": "Oi"})).await["result"]["hangar_page"]["id"].as_str().unwrap().to_owned();
    let r = owner_get(&st, &format!("/api/sessions/s1/pages/{id}/shot?theme=dark&width=728")).await;
    assert_eq!(r.status(), 404);
    assert!(body_text(r).await.contains("erro_pagina_sem_imagem"));
}

#[tokio::test]
async fn guest_is_passed_to_python() {
    let (st, py) = test_state_with_info("s1", "k", "/t/k.jsonl").await;
    let _ = anonymous_get(&st, "/api/sessions/s1/pages/abc").await;
    assert_eq!(py.hits("/api/sessions/s1/pages/abc"), 1);
}
```

`test_state_with_info`, `publish` (POST na porta privada com o segredo), `owner_get`, `anonymous_get` e `body_text` são auxiliares do próprio módulo de teste; se já existirem equivalentes nos testes de `routes.rs`, reaproveite e não duplique. O `PATH` do teste não pode ter Chrome: os testes que publicam rodam com `CP_CHROMIUM_BIN=/nao/existe` (sem altura), exceto o ignorado da Task 2.

- [ ] **Step 13: Implementar `pages/routes.rs`**

```rust
//! Rotas das páginas: publicação pela ponte privada (MCP), leitura pelo dono. Convidado segue ao Python.
use std::collections::{BTreeMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::SystemTime;

use axum::body::{Body, to_bytes};
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use super::chrome::{self, Job};
use super::store::NewPage;
use super::{HEIGHT_MAX, HEIGHT_MIN, HTML_MAX_CHARS, TITLE_MAX_CHARS, Theme, WIDTHS, images, theme};
use crate::routes::{AppState, fetch_info, gate, pass};

const MESSAGE: &str = "Mostrada ao leitor acima da resposta. Não mencione nem descreva a página; responda só o que ela não diz.";
const BODY_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishBody { session: String, html: String, title: String, #[serde(default)] height: Option<u32>, #[serde(default)] draft: bool }

fn fail(code: &str, detail: &str) -> Value { json!({"ok": false, "error": {"code": code, "detail": detail}}) }

pub async fn publish_bridge(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    if !crate::terminal_routes::trusted_internal(&st, peer, req.headers()) { return StatusCode::NOT_FOUND.into_response(); }
    // Endereço público que o Python conhece (ele subiu o Rust); só vale para a URL do rascunho.
    let base = req.headers().get("x-hangar-public-base").and_then(|v| v.to_str().ok())
        .filter(|b| b.starts_with("http://") || b.starts_with("https://")).unwrap_or("").to_owned();
    let Ok(bytes) = to_bytes(req.into_body(), BODY_LIMIT).await else { return axum::Json(fail("erro_pagina_invalida", "corpo grande demais")).into_response() };
    let Ok(body) = serde_json::from_slice::<PublishBody>(&bytes) else { return axum::Json(fail("erro_pagina_invalida", "corpo inválido")).into_response() };
    axum::Json(publish(&st, body, &base).await).into_response()
}

async fn publish(st: &AppState, b: PublishBody, base: &str) -> Value {
    let chars = b.html.chars().count();
    if chars == 0 || chars > HTML_MAX_CHARS { return fail("erro_pagina_invalida", "html precisa ter de 1 a 512000 caracteres"); }
    let title_chars = b.title.trim().chars().count();
    if title_chars == 0 || title_chars > TITLE_MAX_CHARS { return fail("erro_pagina_invalida", "title precisa ter de 1 a 200 caracteres"); }
    if b.height.is_some_and(|h| !(HEIGHT_MIN..=HEIGHT_MAX).contains(&h)) { return fail("erro_pagina_invalida", "height fica entre 80 e 2000"); }
    let info = match fetch_info(&st.http, st.cfg.upstream, &st.cfg.internal_secret, &b.session).await {
        Ok(Some(i)) => i,
        Ok(None) => return fail("erro_sessao_desconhecida", "sessão não encontrada"),
        Err(_) => return fail("erro_pagina_sem_info", "o backend não respondeu quem é a sessão"),
    };
    let jsonl = info.jsonl.clone().unwrap_or_default();
    let (html, missing) = if b.draft {
        let s = images::scan(&b.html); (s.html, s.missing)
    } else {
        match images::inline(&b.html) {
            Ok(h) => (h, Vec::new()),
            Err(e) => return fail(e.code(), e.path().unwrap_or("página com imagens demais")),
        }
    };
    let html = theme::inject(&html);
    let page = NewPage { html: html.clone(), title: b.title.trim().to_owned(), height: b.height, heights: BTreeMap::new(), draft: b.draft };
    let pages = &st.pages;
    let id = match pages.save(&info.session_key, &jsonl, &page) {
        Ok(id) => id,
        Err(_) => return fail("erro_pagina_nao_gravada", "não foi possível gravar a página"),
    };
    let shot = b.draft.then(|| pages.shot_path(&info.session_key, &id, Theme::Dark, 728));
    let jobs: Vec<Job> = WIDTHS.iter().map(|w| Job { width: *w, theme: Theme::Dark, shot: (*w == 728).then(|| shot.clone()).flatten() }).collect();
    let rendered = chrome::render(&html, &jobs).await;
    let (heights, console, browser) = match &rendered {
        Ok(r) => (r.heights.clone(), r.console.clone(), "ok"),
        Err(e) => (BTreeMap::new(), Vec::new(), e.status()),
    };
    if !heights.is_empty() { let _ = pages.set_heights(&info.session_key, &id, heights.clone()); }
    // Sem rodada da lista ainda não há quem esteja vivo: varrer com conjunto vazio apagaria tudo.
    if let Some(live) = st.list.live_jsonl() { pages.sweep(&live, SystemTime::now()); }
    if b.draft {
        let name = percent_encoding::utf8_percent_encode(&b.session, percent_encoding::NON_ALPHANUMERIC);
        let url = format!("{base}/api/sessions/{name}/pages/{id}?token={}", st.cfg.auth_token);
        return json!({"ok": true, "result": {"draft": {"id": id, "url": url,
            "shot": rendered.is_ok().then(|| shot.map(|p| p.display().to_string())).flatten(),
            "heights": heights, "console": console, "missing_images": missing, "browser": browser,
            "browser_reason": rendered.as_ref().err().map(|e| e.reason())}}});
    }
    json!({"ok": true, "result": {"hangar_page": {"id": id, "title": page.title, "height": b.height, "heights": heights}, "message": MESSAGE}})
}
```

`trusted_internal` não existe ainda: extraia de `terminal_routes.rs:42-49` (peer loopback, nenhum `X-Forwarded-For` fora do loopback, segredo não vazio e comparado em tempo constante) uma `pub(crate) fn trusted_internal(st: &AppState, peer: SocketAddr, headers: &HeaderMap) -> bool` e use-a nas duas rotas, sem copiar a checagem. `st.list.live_jsonl()` nasce na Step 15. `st.pages` é `Arc<Store>` (Step 14).

GETs:

```rust
#[derive(Deserialize)]
pub struct PageQuery { #[serde(default)] raw: Option<String> }
#[derive(Deserialize)]
pub struct ShotQuery { theme: Option<String>, width: Option<u32> }

fn expired() -> Response {
    (StatusCode::NOT_FOUND, axum::Json(json!({"detail": {"code": "erro_pagina_expirou", "msg": "página apagada com a sessão"}}))).into_response()
}

pub async fn page(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path((name, id)): Path<(String, String)>, Query(q): Query<PageQuery>, req: Request) -> Response {
    let (fwd, owner) = gate(&st, peer, &req);
    if !owner { return pass(&st, req, &fwd).await; }
    let Ok(Some(info)) = fetch_info(&st.http, st.cfg.upstream, &st.cfg.internal_secret, &name).await else { return expired() };
    let (Some(html), Some(meta)) = (st.pages.html(&info.session_key, &id), st.pages.meta(&info.session_key, &id)) else { return expired() };
    if q.raw.is_some() {
        let mut r = Response::new(Body::from(html));
        let h = r.headers_mut();
        h.insert(header::CONTENT_TYPE, "text/plain; charset=utf-8".parse().unwrap());
        h.insert("content-security-policy", "sandbox".parse().unwrap());
        h.insert("x-content-type-options", "nosniff".parse().unwrap());
        h.insert(header::CACHE_CONTROL, "private, max-age=3600".parse().unwrap());
        crate::routes::cors(req.headers(), r.headers_mut());
        return r;
    }
    crate::workspace_routes::isolated_html(html.as_bytes(), &meta.title)
}

pub async fn shot(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path((name, id)): Path<(String, String)>, Query(q): Query<ShotQuery>, req: Request) -> Response {
    let (fwd, owner) = gate(&st, peer, &req);
    if !owner { return pass(&st, req, &fwd).await; }
    let theme = q.theme.as_deref().and_then(Theme::parse).unwrap_or(Theme::Dark);
    let want = q.width.unwrap_or(728);
    let width = *WIDTHS.iter().min_by_key(|w| w.abs_diff(want)).unwrap();
    let Ok(Some(info)) = fetch_info(&st.http, st.cfg.upstream, &st.cfg.internal_secret, &name).await else { return expired() };
    let Some(html) = st.pages.html(&info.session_key, &id) else { return expired() };
    let path = st.pages.shot_path(&info.session_key, &id, theme, width);
    if !path.is_file() {
        if let Err(e) = chrome::render(&html, &[Job { width, theme, shot: Some(path.clone()) }]).await {
            return (StatusCode::NOT_FOUND, axum::Json(json!({"detail": {"code": "erro_pagina_sem_imagem", "msg": e.reason()}}))).into_response();
        }
    }
    let Ok(bytes) = tokio::fs::read(&path).await else { return expired() };
    let mut r = Response::new(Body::from(bytes));
    r.headers_mut().insert(header::CONTENT_TYPE, "image/png".parse().unwrap());
    r.headers_mut().insert(header::CACHE_CONTROL, "private, max-age=3600".parse().unwrap());
    crate::routes::cors(req.headers(), r.headers_mut());
    r
}
```

Rascunho também tem `id` e é lido pelas mesmas rotas (o `browser_open` do rascunho usa a casca).

- [ ] **Step 14: Registrar as rotas, o estado e o protocolo**

Em `routes.rs`, no router privado (`:210-220`): `.route("/__hangar_server/pages", axum::routing::post(crate::pages::routes::publish_bridge))`. No router público (`:222-261`): `.route("/__hangar_server/pages", axum::routing::any(|| async { StatusCode::NOT_FOUND }))` junto das outras privadas, e:

```rust
.route("/api/sessions/{name}/pages/{id}", get(crate::pages::routes::page).fallback(pass_any))
.route("/api/sessions/{name}/pages/{id}/shot", get(crate::pages::routes::shot).fallback(pass_any))
```

Em `AppState`, campo `pub pages: Arc<crate::pages::store::Store>` criado com `Arc::new(Store::new(Store::default_root()))` e o mesmo `Arc` passado ao `ListBridge` na construção (Step 15). Em `lib.rs:30`, `INTERNAL_PROTOCOL: u32 = 37`.

- [ ] **Step 15: Varredura na rodada da lista**

Em `list/bridge.rs`, logo depois de `self.prune_gone(&rows, Instant::now());` (`:464`), guarde os `jsonl` vivos e varra:

```rust
let live: HashSet<String> = rows.iter().filter_map(|r| r.jsonl.clone()).collect();
*lock(&self.live_jsonl) = Some(live.clone());
self.pages.sweep(&live, SystemTime::now());
```

com os campos `live_jsonl: Mutex<Option<HashSet<String>>>` e `pages: Arc<Store>` no `ListBridge`, e `pub fn live_jsonl(&self) -> Option<HashSet<String>> { lock(&self.live_jsonl).clone() }`. O `sweep` é síncrono e lê só uma pasta por sessão; vai ao lado do `prune_gone`, que também é síncrono dentro da rodada. Acrescente um teste no módulo de testes de `bridge.rs` ao lado de `prune_gone` (`:907-913`): rodada com a linha `jsonl=/t/k.jsonl`, depois rodadas sem ela por mais de 60 s de relógio simulado → pasta `k` apagada.

- [ ] **Step 16: Verificação manual — publicar e ler pela ponte**

Não suba um segundo backend para testar: ele mata as sessões sem terminal vivas (memória `nunca-subir-segundo-backend`). O uso real da rota fica na Task 4, Step 22. Aqui, só `cd crates && nice -n 19 cargo check -p hangar-server` e confira que compila.

- [ ] **Step 17: Commit**

```bash
git add crates/hangar-server/src/pages/routes.rs crates/hangar-server/src/workspace_routes.rs crates/hangar-server/src/routes.rs crates/hangar-server/src/list/bridge.rs crates/hangar-server/src/lib.rs crates/hangar-server/src/migration_status.rs
git commit -m "feat(pages): publish bridge, owner routes and sweep tied to session lifetime"
```

### Task 4: Tool `html_render` no MCP `hangar`
Status: ready-for-agent
Risk: medium

**Files:**
- Create: `backend/app/pages_bridge.py`
- Modify: `backend/app/rust_server.py:35` (`RUST_SERVER_PROTOCOL = 37`) e `:379-392` (`pages_bridge.configure`)
- Modify: `backend/app/mcp_server.py` (tool nova depois de `browser_open`, `:256-265`)
- Test: `backend/tests/test_pages_bridge.py`

**Interfaces:**
- Consumes: `POST /__hangar_server/pages` (Task 3)
- Produces: `pages_bridge.configure(address: str | None, secret: str | None) -> None`, `pages_bridge.publish(payload: dict, public_base: str) -> dict` (levanta `PagesBridgeError(code, detail)`), tool MCP `html_render(html, title, height=None, draft=False)`

- [ ] **Step 18: Escrever o teste da ponte**

```python
import json
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer

import pytest

from app import pages_bridge


def _server(reply: dict, seen: list):
    class H(BaseHTTPRequestHandler):
        def do_POST(self):
            seen.append((self.path, self.headers.get("x-hangar-internal"), json.loads(self.rfile.read(int(self.headers["content-length"])))))
            body = json.dumps(reply).encode()
            self.send_response(200); self.send_header("content-type", "application/json"); self.end_headers(); self.wfile.write(body)
        def log_message(self, *a): pass
    srv = HTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


def test_publish_sends_secret_and_returns_result():
    seen = []
    srv = _server({"ok": True, "result": {"hangar_page": {"id": "a1"}}}, seen)
    pages_bridge.configure(f"127.0.0.1:{srv.server_port}", "s3gredo")
    try:
        out = pages_bridge.publish({"session": "x", "html": "<p></p>", "title": "t"}, "http://127.0.0.1:8765")
    finally:
        srv.shutdown(); pages_bridge.configure(None, None)
    assert out == {"hangar_page": {"id": "a1"}}
    assert seen[0][0] == "/__hangar_server/pages" and seen[0][1] == "s3gredo"


def test_error_from_rust_is_raised_with_code():
    srv = _server({"ok": False, "error": {"code": "erro_pagina_invalida", "detail": "title"}}, [])
    pages_bridge.configure(f"127.0.0.1:{srv.server_port}", "s")
    try:
        with pytest.raises(pages_bridge.PagesBridgeError) as e:
            pages_bridge.publish({"session": "x", "html": "", "title": "t"}, "http://x")
    finally:
        srv.shutdown(); pages_bridge.configure(None, None)
    assert e.value.code == "erro_pagina_invalida"


def test_without_rust_server_is_clear_error():
    pages_bridge.configure(None, None)
    with pytest.raises(pages_bridge.PagesBridgeError) as e:
        pages_bridge.publish({"session": "x", "html": "<p></p>", "title": "t"}, "http://x")
    assert e.value.code == "erro_paginas_sem_servidor_rust"
```

- [ ] **Step 19: Implementar `pages_bridge.py`**

```python
"""Ponte para a rota privada de páginas do hangar-server (Rust). Sem o Rust de pé, páginas não existem."""
import http.client
import json
import urllib.error
import urllib.request

_config: tuple[str, str] | None = None
_TIMEOUT = 15.0  # o Rust tem 8 s de Chromium; folga para gravar e responder
_MAX_RESPONSE = 256 * 1024
_opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))


class PagesBridgeError(Exception):
    def __init__(self, code: str, detail: str = ""):
        super().__init__(f"{code}: {detail}" if detail else code)
        self.code, self.detail = code, detail


def configure(address: str | None, secret: str | None) -> None:
    global _config
    _config = (address, secret) if address and secret else None


def publish(payload: dict, public_base: str) -> dict:
    config = _config
    if config is None:
        raise PagesBridgeError("erro_paginas_sem_servidor_rust", "páginas precisam do hangar-server de pé")
    req = urllib.request.Request(
        f"http://{config[0]}/__hangar_server/pages",
        data=json.dumps(payload, ensure_ascii=False).encode("utf-8"),
        headers={"content-type": "application/json", "x-hangar-internal": config[1], "x-hangar-public-base": public_base},
        method="POST")
    try:
        with _opener.open(req, timeout=_TIMEOUT) as response:
            body = response.read(_MAX_RESPONSE + 1)
        if len(body) > _MAX_RESPONSE:
            raise ValueError("resposta grande demais")
        value = json.loads(body)
    except (OSError, ValueError, urllib.error.URLError, http.client.HTTPException) as e:
        raise PagesBridgeError("erro_paginas_indisponivel", type(e).__name__) from e
    if not isinstance(value, dict) or value.get("ok") is not True:
        error = value.get("error") if isinstance(value, dict) and isinstance(value.get("error"), dict) else {}
        raise PagesBridgeError(str(error.get("code") or "erro_paginas_indisponivel"), str(error.get("detail") or ""))
    return value["result"]
```

Em `rust_server.py`, acrescente `pages_bridge.configure(address, env["HANGAR_INTERNAL_SECRET"])` junto das três chamadas de `configure` (`:383-385`) e `pages_bridge.configure(None, None)` no `except` (`:389-391`); importe o módulo como os outros. `RUST_SERVER_PROTOCOL = 37` (`:35`). Volte à Task 3 Step 13 item 3 e leia o `x-hangar-public-base` no Rust.

- [ ] **Step 20: Implementar a tool**

```python
_HTML_RENDER = (
    "Mostra uma página HTML pronta (gráfico, tabela, diagrama, mock de tela, comparação) DENTRO desta conversa, "
    "no lugar desta chamada e acima do seu texto final; chame antes de escrever a resposta. O leitor já vê a página: "
    "a resposta não a anuncia, não diz onde ela está e não repete o que ela mostra — diga só o que ela não diz. "
    "Confira antes de publicar com draft=true: devolve `shot` (PNG, leia com a ferramenta de imagem), `console` e "
    "`heights`; com o app desktop aberto, abra a `url` com browser_open e use browser para passar o mouse e clicar. "
    "Página publicada não muda: corrigir é publicar de novo. "
    "PÁGINA: um documento só, com <style> e <script> embutidos. Imagem local por caminho absoluto "
    "(src=\"/abs/a.png\", url(/abs/b.webp) ou string JS) é embutida sozinha; arquivo que não é imagem é recusado. "
    "URL http(s) (biblioteca de gráfico em CDN) carrega como está; file: não. "
    "LAYOUT: a moldura não tem borda e fica sobre o fundo da conversa, na largura da coluna (cerca de 728px no "
    "desktop, 360px no celular). Deixe html, body e o elemento mais externo SEM cor de fundo. Largura fluida, sem "
    "padding horizontal no elemento externo, sem cartão, borda ou título de banner em volta: a página é parte da "
    "resposta. Caixa que precisa de fundo próprio leva padding de 16px ou mais e cantos var(--radius). Gráfico com "
    "altura fixa em pixel. Nada de 100vh nem height:100% em html/body: a moldura cresce com a página. "
    "TEMA: use as variáveis --foreground, --muted-foreground, --surface, --border, --accent, --accent-foreground, "
    "--danger, --warning, --success, --code-background, --chart-1 a --chart-4, --radius, --font-sans, --font-mono; "
    "--background é transparente. Elas seguem o tema claro/escuro do app; seu CSS pode sobrescrever. "
    "height (80-2000) só para limitar a moldura e deixar o resto rolar dentro dela.")


@mcp.tool(description=_HTML_RENDER)
async def html_render(ctx: Context, html: str, title: str, height: int | None = None, draft: bool = False) -> dict[str, Any]:
    from app import pages_bridge
    eu = await _eu(ctx)
    payload = {"session": eu, "html": html, "title": title, "draft": draft}
    if height is not None:
        payload["height"] = height
    base = f"http://127.0.0.1:{settings.port}"
    try:
        return await asyncio.to_thread(pages_bridge.publish, payload, base)
    except pages_bridge.PagesBridgeError as e:
        raise ToolError(f"{e.code}: {e.detail}" if e.detail else e.code) from e
```

`settings` e `asyncio` já são importados em `mcp_server.py` (confira; se `settings` não for, use `from app.config import settings` como no resto do arquivo). A `url` do rascunho só é aberta no navegador embutido desta máquina, então o endereço de loopback serve; se o servidor público não escutar em loopback (`CP_LAN_BIND_IP` com IP de LAN), use o endereço que `rust_server.listen_addr` monta para o Rust.

- [ ] **Step 21: Conferir o nome que o Codex grava**

Depois do restart da Step 22, numa sessão Codex de teste (`luna`, esforço `low`; nunca o padrão), peça uma chamada a `html_render` com `draft=false` e leia o rollout dela em `~/.codex/sessions/…`: anote o `name` (e `namespace`, se houver) do `function_call`. Registre o valor exato num comentário de uma linha em `packages/core/src/htmlPage.ts` (Task 5) e no detector do nativo (Task 8), e ajuste `isHtmlRenderTool` para casar com ele.

- [ ] **Step 22: Verificação manual — publicar de verdade (precisa do Jefferson)**

Peça ao Jefferson para autorizar o restart do `hangar-backend.service` apontando para esta worktree (ou faça o teste depois do merge). Com a autorização:
1. Numa sessão Claude descartável (`cx-paginas`), peça uma página com um gráfico de barras e uma imagem local (`/home/jefferson/Imagens/...png` qualquer).
2. Confira o `tool_result`: `hangar_page.id`, `heights` com as três larguras.
3. `curl -s -H "Authorization: Bearer $CP_AUTH_TOKEN" "http://127.0.0.1:8765/api/sessions/cx-paginas/pages/<id>?raw=1" | head -c 400` mostra o `hangar-theme`.
4. `draft=true` devolve `shot`; abra o PNG.
5. Feche a sessão e, 60 s depois com a lista aberta, confira que `~/.hangar/paginas/<chave>` sumiu.

- [ ] **Step 23: Commit**

```bash
git add backend/app/pages_bridge.py backend/app/rust_server.py backend/app/mcp_server.py backend/tests/test_pages_bridge.py
git commit -m "feat(mcp): html_render tool publishing pages through the Rust bridge"
```

### Task 5: Reconhecer a página no `packages/core`
Status: ready-for-agent
Risk: low

**Files:**
- Create: `packages/core/src/htmlPage.ts`
- Create: `packages/core/src/htmlPage.test.ts`
- Modify: `packages/core/src/toolGroups.ts:46` (página fica fora do grupo, como o `Agent`)
- Modify: `packages/core/src/index.ts` (exportar, se o pacote tiver barril; confira como `askquestion.ts` é exportado)
- Modify: `messages/pt.json`, `messages/en.json`

**Interfaces:**
- Produces:
  - `isHtmlRenderTool(name: string | null | undefined): boolean`
  - `type HtmlPageRef = { id: string; title: string; height: number | null; heights: Record<string, number> }`
  - `htmlPageFromResult(toolName, result: string | null | undefined): HtmlPageRef | null`
  - `reservedHeight(ref: HtmlPageRef, width: number): number`
  - `frameHeight(ref: HtmlPageRef, width: number, reported: number | null): number`
  - `pageUrls(base: string, session: string, id: string): { raw: string; shot: (theme: 'dark' | 'light', width: number) => string; isolated: string }`
  - `type PageFetchState = 'loading' | 'ready' | 'error' | 'expired'` e `pageFetchState(status: number | 'network'): PageFetchState`
  - `themeVariables(get: (name: string) => string): Record<string, string>` (lê do app as variáveis que a página usa)
  - Chaves i18n: `page_loading`, `page_error`, `page_retry`, `page_expired`, `page_open_browser`, `page_no_image`

- [ ] **Step 24: Escrever os testes**

```ts
import { describe, expect, it } from 'vitest';
import { frameHeight, htmlPageFromResult, isHtmlRenderTool, pageFetchState, reservedHeight } from './htmlPage';

const ref = { id: 'a1', title: 'T', height: null, heights: { '360': 500, '728': 380, '1000': 360 } };

describe('htmlPage', () => {
  it('reconhece a tool do hangar e só ela', () => {
    expect(isHtmlRenderTool('mcp__hangar__html_render')).toBe(true);
    expect(isHtmlRenderTool('mcp__outro__html_render')).toBe(false);
    expect(isHtmlRenderTool('Read')).toBe(false);
  });

  it('lê a referência do resultado e ignora rascunho e erro', () => {
    const ok = JSON.stringify({ hangar_page: ref, message: 'x' });
    expect(htmlPageFromResult('mcp__hangar__html_render', ok)?.id).toBe('a1');
    expect(htmlPageFromResult('mcp__hangar__html_render', JSON.stringify({ draft: { id: 'b' } }))).toBeNull();
    expect(htmlPageFromResult('mcp__hangar__html_render', 'erro_pagina_invalida: title')).toBeNull();
    expect(htmlPageFromResult('Read', ok)).toBeNull();
  });

  it('reserva a altura da largura medida mais próxima', () => {
    expect(reservedHeight(ref, 390)).toBe(500);
    expect(reservedHeight(ref, 700)).toBe(380);
    expect(reservedHeight({ ...ref, heights: {} }, 700)).toBe(240);
  });

  it('altura informada pela página vence, com teto do agente e limites', () => {
    expect(frameHeight(ref, 728, 420)).toBe(420);
    expect(frameHeight({ ...ref, height: 300 }, 728, 420)).toBe(300);
    expect(frameHeight(ref, 728, 5000)).toBe(2000);
    expect(frameHeight(ref, 728, 10)).toBe(80);
    expect(frameHeight(ref, 728, null)).toBe(380);
  });

  it('404 é expirou, rede é erro', () => {
    expect(pageFetchState(200)).toBe('ready');
    expect(pageFetchState(404)).toBe('expired');
    expect(pageFetchState(500)).toBe('error');
    expect(pageFetchState('network')).toBe('error');
  });
});
```

O `tool_result` do Claude para MCP pode chegar como texto JSON puro ou como lista de blocos já unida em texto pelo backend (`transcript.py:580-596`). Se na Step 22 o `result` vier com algo em volta do JSON, ajuste `htmlPageFromResult` para procurar o primeiro objeto JSON que tenha `hangar_page` e acrescente esse caso real ao teste.

- [ ] **Step 25: Implementar `htmlPage.ts`**

```ts
// Página publicada pelo agente (tool html_render do MCP hangar), desenhada no lugar da chamada.
export type HtmlPageRef = { id: string; title: string; height: number | null; heights: Record<string, number> };
export type PageFetchState = 'loading' | 'ready' | 'error' | 'expired';

const MIN = 80;
const MAX = 2000;
const FALLBACK = 240;
// Claude: mcp__hangar__html_render. Codex: conferido na Task 4, Step 21.
const NAMES = new Set(['mcp__hangar__html_render']);

export function isHtmlRenderTool(name: string | null | undefined): boolean {
  return !!name && NAMES.has(name);
}

export function htmlPageFromResult(toolName: string | null | undefined, result: string | null | undefined): HtmlPageRef | null {
  if (!isHtmlRenderTool(toolName) || !result) return null;
  try {
    const page = JSON.parse(result)?.hangar_page;
    if (!page || typeof page.id !== 'string' || typeof page.title !== 'string') return null;
    return { id: page.id, title: page.title, height: typeof page.height === 'number' ? page.height : null, heights: page.heights ?? {} };
  } catch {
    return null;
  }
}

const clamp = (n: number) => Math.min(MAX, Math.max(MIN, Math.round(n)));

export function reservedHeight(ref: HtmlPageRef, width: number): number {
  const ws = Object.keys(ref.heights).map(Number).filter((w) => ref.heights[String(w)] > 0);
  if (!ws.length) return FALLBACK;
  const near = ws.reduce((a, b) => (Math.abs(b - width) < Math.abs(a - width) ? b : a));
  return clamp(ref.heights[String(near)]);
}

export function frameHeight(ref: HtmlPageRef, width: number, reported: number | null): number {
  const natural = reported ?? reservedHeight(ref, width);
  return clamp(ref.height != null ? Math.min(ref.height, natural) : natural);
}

export function pageFetchState(status: number | 'network'): PageFetchState {
  if (status === 404) return 'expired';
  if (typeof status === 'number' && status >= 200 && status < 300) return 'ready';
  return 'error';
}

export function pageUrls(base: string, session: string, id: string) {
  const root = `${base}/api/sessions/${encodeURIComponent(session)}/pages/${encodeURIComponent(id)}`;
  return {
    raw: `${root}?raw=1`,
    isolated: root,
    shot: (theme: 'dark' | 'light', width: number) => `${root}/shot?theme=${theme}&width=${Math.round(width)}`,
  };
}

const VARS = ['--foreground', '--muted-foreground', '--surface', '--border', '--accent', '--accent-foreground',
  '--danger', '--warning', '--success', '--code-background', '--chart-1', '--chart-2', '--chart-3', '--chart-4', '--font-sans', '--font-mono'];

// Lê do app (por nome da variável da página) o valor real do tema; vazio fica com o padrão injetado.
export function themeVariables(get: (name: string) => string): Record<string, string> {
  const out: Record<string, string> = { '--background': 'transparent' };
  for (const v of VARS) { const x = get(v).trim(); if (x) out[v] = x; }
  return out;
}
```

Em `toolGroups.ts:46`, troque a condição do `Agent` por `ev.kind === 'tool_use' && (ev.tool_name === 'Agent' || isHtmlRenderTool(ev.tool_name))`, importando de `./htmlPage`, e ajuste o comentário para citar os dois.

Chaves em `messages/pt.json` / `messages/en.json`:

```json
"page_loading": "Carregando {title}…",            "page_loading": "Loading {title}…",
"page_error": "Não foi possível carregar «{title}»", "page_error": "Couldn't load “{title}”",
"page_retry": "Tentar de novo",                     "page_retry": "Try again",
"page_expired": "Esta página foi apagada quando a sessão foi encerrada", "page_expired": "This page was deleted when the session closed",
"page_open_browser": "Abrir no navegador",          "page_open_browser": "Open in browser",
"page_no_image": "Sem prévia: o servidor não tem Chromium", "page_no_image": "No preview: the server has no Chromium"
```

(coluna da esquerda em `pt.json`, da direita em `en.json`, na ordem alfabética do arquivo).

- [ ] **Step 26: Commit**

```bash
git add packages/core/src/htmlPage.ts packages/core/src/htmlPage.test.ts packages/core/src/toolGroups.ts packages/core/src/index.ts messages/pt.json messages/en.json
git commit -m "feat(core): recognize html_render pages and compute frame sizes"
```

### Task 6: Página viva no PWA
Status: ready-for-agent
Risk: medium

**Files:**
- Create: `frontend/src/components/HtmlPageFrame.svelte`
- Modify: `frontend/src/components/ToolCard.svelte:86-91,372-381` (ramo da página antes do `HangarCommandCard`)
- Modify: `frontend/src/components/MessageList.svelte:471-490` (rolar para o fim quando a moldura cresce e a lista estava no fim)

**Interfaces:**
- Consumes: `htmlPageFromResult`, `frameHeight`, `reservedHeight`, `pageUrls`, `pageFetchState`, `themeVariables` (Task 5); `baseOf(server)` e o token do servidor (como o `fileUrl` monta, `packages/core/src/api.ts:105-125`)
- Produces: `<HtmlPageFrame page={HtmlPageRef} session={string} server={Server} onresize={() => void} />`

- [ ] **Step 27: Usar a skill `svelte-code-writer` e implementar o componente**

```svelte
<script lang="ts">
  import { onMount } from 'svelte';
  import * as m from '../paraglide/messages';
  import { frameHeight, pageFetchState, pageUrls, reservedHeight, themeVariables, type HtmlPageRef, type PageFetchState } from '@hangar/core/htmlPage';
  import { baseOf } from '@hangar/core/rota';
  import { tokenOf } from '../lib/auth';
  import { theme } from '../lib/theme.svelte';

  let { page, session, server, onresize }: { page: HtmlPageRef; session: string; server: any; onresize?: () => void } = $props();

  let state = $state<PageFetchState>('loading');
  let doc = $state('');
  let reported = $state<number | null>(null);
  let width = $state(0);
  let frame: HTMLIFrameElement | undefined = $state();
  let box: HTMLDivElement | undefined = $state();

  const height = $derived(width ? frameHeight(page, width, reported) : reservedHeight(page, 728));
  const scheme = $derived(theme.dark ? 'dark' : 'light');

  function vars() {
    const cs = getComputedStyle(document.documentElement);
    const map: Record<string, string> = {
      '--foreground': '--text-primary', '--muted-foreground': '--text-muted', '--surface': '--bg-surface', '--border': '--border-default',
      '--accent': '--accent', '--accent-foreground': '--text-inverse', '--danger': '--error', '--warning': '--warning', '--success': '--success',
      '--code-background': '--bg-base', '--chart-1': '--chart-1', '--chart-2': '--chart-2', '--chart-3': '--chart-3', '--chart-4': '--chart-4',
      '--font-sans': '--font-ui', '--font-mono': '--font-mono',
    };
    return themeVariables((name) => cs.getPropertyValue(map[name] ?? name));
  }

  function themed(html: string) {
    const css = `:root{color-scheme:${scheme};${Object.entries(vars()).map(([k, v]) => `${k}:${v}`).join(';')}}`;
    // Troca só o conteúdo do bloco de tema injetado pelo servidor; o data-base fica.
    return html.replace(/(<style id="hangar-theme" data-base="([^"]*)">)[\s\S]*?(<\/style>)/, (_all, open, base, close) =>
      `${open}${css}${base.replaceAll('&quot;', '"')}${close}`);
  }

  async function load() {
    state = 'loading';
    try {
      const r = await fetch(pageUrls(baseOf(server), session, page.id).raw, { headers: { Authorization: `Bearer ${tokenOf(server)}` } });
      state = pageFetchState(r.status);
      if (state === 'ready') doc = themed(await r.text());
    } catch {
      state = pageFetchState('network');
    }
  }

  function onMessage(e: MessageEvent) {
    if (!frame || e.source !== frame.contentWindow) return;
    const d = e.data;
    if (d?.method === 'ui/notifications/size-changed' && typeof d.params?.height === 'number') {
      reported = d.params.height;
      onresize?.();
    } else if (d?.method === 'ui/open-link' && typeof d.params?.url === 'string' && /^https?:/i.test(d.params.url)
      && (navigator as any).userActivation?.isActive) {
      window.open(d.params.url, '_blank', 'noopener,noreferrer');
    }
  }

  $effect(() => {
    // Tema do app mudou: a página recebe as variáveis novas sem recarregar.
    scheme;
    frame?.contentWindow?.postMessage({ jsonrpc: '2.0', method: 'ui/notifications/host-context-changed',
      params: { theme: scheme, styles: { variables: vars() } } }, '*');
  });

  onMount(() => {
    load();
    const ro = new ResizeObserver(([e]) => (width = e.contentRect.width));
    if (box) ro.observe(box);
    return () => ro.disconnect();
  });
</script>

<svelte:window onmessage={onMessage} />

<div class="page" bind:this={box} style:height="{height}px" aria-busy={state === 'loading'}>
  {#if state === 'ready'}
    <iframe bind:this={frame} title={page.title} srcdoc={doc} sandbox="allow-scripts allow-popups"
      referrerpolicy="no-referrer" style:color-scheme={scheme}></iframe>
  {:else if state === 'loading'}
    <p class="note">{m.page_loading({ title: page.title })}</p>
  {:else if state === 'expired'}
    <p class="note">{m.page_expired()}</p>
  {:else}
    <p class="note">{m.page_error({ title: page.title })} <button type="button" onclick={load}>{m.page_retry()}</button></p>
  {/if}
</div>

<style>
  .page { position: relative; width: 100%; background: transparent; }
  iframe { display: block; width: 100%; height: 100%; border: 0; background: transparent; }
  .note { margin: 0; padding: 12px 0; color: var(--text-muted); font-size: var(--text-sm, 13px); }
  .note button { margin-left: 8px; }
</style>
```

Antes de escrever, confira e use os nomes reais: de onde o front lê o token de um servidor (`frontend/src/lib/auth.ts` ou o `ApiEnv` que monta o `fileUrl`), o import de `baseOf`, como o tema atual é exposto (procure o store de tema/aparência em `frontend/src/lib/`) e o alias de import do `packages/core` usado pelos outros componentes. Troque `tokenOf`, `theme.dark` e os caminhos de import pelos reais; não crie um store novo de tema.

- [ ] **Step 28: Ligar no `ToolCard` e na lista**

Em `ToolCard.svelte`, perto de `hangarAcao` (`:86-91`):

```ts
const pagina = $derived(htmlPageFromResult(event.tool_name, result?.result ?? null));
```

e, antes do ramo `{#if hangarAcao}` (`:372`), `{#if pagina}<HtmlPageFrame page={pagina} {session} {server} onresize={onPageResize} />{:else if hangarAcao}…`. Enquanto a chamada roda (sem `result`), cai no cartão normal com o progresso, como hoje. `session`, `server` e `onPageResize` chegam como props do `MessageList` (adicione-as onde o `ToolCard` é montado, `MessageList.svelte:682`). Em `MessageList.svelte`, `onPageResize` reaproveita o caminho de auto-rolagem (`:471-490`): se `atBottom`, `tick()` → `requestAnimationFrame(scrollToBottom)`.

- [ ] **Step 29: Verificação manual — PWA no navegador embutido**

Com o backend da Task 4 de pé e a sessão `cx-paginas` com páginas publicadas:
1. `hangar-preview open http://<front>/#/…/cx-paginas` em largura de celular (`hangar-preview layout 390 844`) e avise o Jefferson que o painel abriu.
2. Confira os quatro estados: carregando (recarregue com rede lenta no `eval`), sucesso (gráfico aparece sem fundo próprio), erro (pare o backend? não — simule apontando o `id` para outro servidor desligado na lista; se não der, registre que não foi conferido), expirou (feche a sessão e reabra a conversa pelo histórico).
3. Troque o tema do app e veja a página trocar sem recarregar.
4. Passe o mouse e clique num link da página: abre em aba nova; a página não navega.
5. `eval 'document.querySelector("iframe").contentWindow.location.href'` deve falhar (origem opaca); `eval` dentro da página tentando `parent.localStorage` falha.
6. Tire um print (`hangar-preview shot`) e cite o caminho na resposta. Feche o preview ao terminar.

- [ ] **Step 30: Commit**

```bash
git add frontend/src/components/HtmlPageFrame.svelte frontend/src/components/ToolCard.svelte frontend/src/components/MessageList.svelte
git commit -m "feat(pwa): live html_render pages in an isolated srcdoc iframe"
```

### Task 7: Motor de página no Chromium do nativo (Linux)
Status: ready-for-agent
Risk: high

**Files:**
- Modify: `desktop-native/src/browser/chromium/engine.rs` (novo `Starter::start_page`, formato do quadro, evento de ponte)
- Modify: `desktop-native/src/browser/mod.rs:31-37` (variante `Event::Host(String)`)
- Modify: `desktop-native/src/browser/chromium/pipe.rs` (só se `Browser` não expuser `call_blocking` para `Target.createBrowserContext`; não deve precisar)

**Interfaces:**
- Consumes: `Browser::shared`, `Session::attach`, `Surface`, `listen` de `engine.rs`
- Produces:
  - `Starter::start_page(self, html: &str, width: f32, events: async_channel::Sender<Event>) -> Result<Engine, String>`
  - `Event::Host(String)` (JSON cru vindo de `window.hangarHost`)
  - `Engine::evaluate(&self, expression: &str)` (dispara `Runtime.evaluate` sem esperar)
  - `Surface` com campo `format: image::ImageFormat` (Jpeg no painel, Png na página)

- [ ] **Step 31: Ler a skill `gpui-kit` e o motor**

Leia a skill `gpui-kit` (Coding Guides) e `engine.rs` inteiro. O que muda: o painel continua idêntico; a página da conversa ganha um segundo jeito de nascer.

- [ ] **Step 32: Implementar `start_page`**

```rust
impl Starter {
    /// Página da conversa: alvo num contexto próprio (sem cookies do painel), documento posto direto, fundo
    /// transparente e quadros em PNG para o alfa chegar à GPUI.
    pub fn start_page(self, html: &str, width: f32, events: async_channel::Sender<Event>) -> Result<Engine, String> {
        let browser = Browser::shared(&self.executor, self.scale)?;
        let long = Duration::from_secs(10);
        let context = browser.call_blocking(None, "Target.createBrowserContext", json!({"disposeOnDetach": true}), long)?["browserContextId"]
            .as_str().ok_or("createBrowserContext sem id")?.to_owned();
        let created = browser.call_blocking(None, "Target.createTarget",
            json!({"url": "about:blank", "newWindow": true, "browserContextId": context}), long)?;
        let target = created["targetId"].as_str().ok_or("createTarget sem targetId")?.to_owned();
        browser.own(&target, true);
        let session = Rc::new(Session::attach(&browser, &target)?);
        let window = browser.call_blocking(None, "Browser.getWindowForTarget", json!({"targetId": target}), long)?["windowId"]
            .as_i64().ok_or("getWindowForTarget sem windowId")?;
        session.call_blocking("Page.enable", json!({}))?;
        session.call_blocking("Inspector.enable", json!({}))?;
        session.call_blocking("Runtime.enable", json!({}))?;
        session.call_blocking("Runtime.addBinding", json!({"name": "hangarHost"}))?;
        session.call_blocking("Emulation.setDefaultBackgroundColorOverride", json!({"color": {"r": 0, "g": 0, "b": 0, "a": 0}}))?;
        session.call_blocking("Fetch.enable", json!({"patterns": [{"resourceType": "Document", "requestStage": "Request"}]}))?;
        browser.call_blocking(None, "Browser.setWindowBounds", json!({"windowId": window, "bounds": {"width": width.round() as i64, "height": 600}}), long)?;
        let measured = session.call_blocking("Runtime.evaluate", json!({"expression": "outerHeight-innerHeight", "returnByValue": true}))?;
        let decoration = measured["result"]["value"].as_f64().unwrap_or(0.).max(0.) as f32;
        let frame = session.call_blocking("Page.getFrameTree", json!({}))?["frameTree"]["frame"]["id"].as_str().unwrap_or_default().to_owned();
        session.call_blocking("Page.setDocumentContent", json!({"frameId": frame, "html": html}))?;
        let state = Rc::new(RefCell::new(model::PageState::default()));
        let surface = Arc::new(Surface { shown: Mutex::new(None), pending: AtomicBool::new(false), events: events.clone(), format: image::ImageFormat::Png });
        let sink = surface.clone();
        browser.sink(session.id(), Box::new(move |params| sink.frame(params)));
        let dead = Rc::new(Cell::new(false));
        let publish = listen(&session, &target, &state, &events, &self.executor, &dead);
        let ev = events.clone();
        let _ = session.on("Runtime.bindingCalled", move |params| {
            if params["name"] == "hangarHost" { let _ = ev.try_send(Event::Host(params["payload"].as_str().unwrap_or("").to_owned())); }
        });
        Ok(Engine {
            session, publish, target, window, decoration, executor: self.executor, surface, dead, png: true,
            placed: Cell::new(None), visible: Cell::new(false), pressed: Cell::new(false),
        })
    }
}
```

Ajustes no resto de `engine.rs`:

- `Surface.format`: `upload` usa `image::load_from_memory_with_format(&bytes, self.format)`; o `start` do painel passa `ImageFormat::Jpeg`.
- `Engine.png: bool`: no `place`, `Page.startScreencast` com `"format": if self.png { "png" } else { "jpeg" }` e `quality` só no JPEG.
- **Navegação depois da carga**: no `Fetch.requestPaused` do `listen`, documento do frame principal numa página (`png == true`) é sempre `failRequest` — o documento já foi posto pelo `setDocumentContent`, e um link que escapou do script de ponte não troca a página. Passe essa regra por um parâmetro `block_documents: bool` de `listen` em vez de olhar o `png`.
- `Engine::evaluate(&self, expression: &str) { self.send("Runtime.evaluate", json!({"expression": expression})); }`.
- `Drop` da página também fecha o contexto: guarde `context: Option<String>` no `Engine` e, no `Drop`, depois do `closeTarget`, `Target.disposeBrowserContext {browserContextId}`.

Em `browser/mod.rs`, a variante nova com o mesmo `cfg_attr` da `Frame`:

```rust
    /// Mensagem da página da conversa (`window.hangarHost`), JSON cru no formato do MCP Apps.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Host(String),
```

Todo `match` sobre `Event` no painel (`app/browser.rs:79-96`) ganha o braço `Event::Host(_) => {}`.

- [ ] **Step 33: Compilar**

Run: `cd desktop-native && nice -n 19 cargo check`
Expected: compila sem aviso novo.

- [ ] **Step 34: Commit**

```bash
git add desktop-native/src/browser/chromium/engine.rs desktop-native/src/browser/mod.rs desktop-native/src/app/browser.rs
git commit -m "feat(native): chromium engine mode for conversation pages (own context, png frames, host binding)"
```

### Task 8: Cartão de página na conversa do nativo
Status: ready-for-agent
Risk: high

**Files:**
- Create: `desktop-native/src/app/page_card.rs`
- Modify: `desktop-native/src/app.rs:3300-3301` (ramo da página antes do `render_agent_card`), `:791` (estado novo do app), `:2539` (busca do HTML/PNG pelo `Api`, no mesmo padrão do `ensure_media`)
- Modify: `desktop-native/src/conversation.rs:53,127` (página fica fora do grupo, como o `Agent`)
- Modify: `desktop-native/src/api/mod.rs` (`page_raw(session, id) -> String`, `page_shot(session, id, theme, width) -> Vec<u8>`, erro 404 distinguível)
- Modify: `messages/pt.json`, `messages/en.json` (reaproveite as chaves `page_*` da Task 5; confira como o `i18n::tr` do nativo lê o catálogo e se exige prefixo `native_`; se exigir, crie `native_page_*` com os mesmos textos)

**Interfaces:**
- Consumes: `Starter::start_page`, `Event::Host`, `Engine::{place, hide, pointer, wheel, key, evaluate}` (Task 7); `theme::{is_dark, colors}`; `Api::endpoint`
- Produces: `page_card::PageRef { id, title, height: Option<u32>, heights: BTreeMap<u32,u32> }`, `page_card::page_from_result(tool_name: &str, result: &str) -> Option<PageRef>`, `page_card::Pages` (dono dos motores vivos, orçamento 4), `App::render_page_card(&mut self, tool: Tool, page: PageRef, cx) -> AnyElement`

- [ ] **Step 35: Detector e orçamento com testes**

Em `page_card.rs`:

```rust
//! Página publicada pelo agente desenhada dentro da conversa: viva no Linux, imagem estática nos outros.
use std::collections::{BTreeMap, VecDeque};
use serde::Deserialize;

pub const LIVE_MAX: usize = 4;
const MIN: u32 = 80;
const MAX: u32 = 2000;

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct PageRef { pub id: String, pub title: String, #[serde(default)] pub height: Option<u32>, #[serde(default)] pub heights: BTreeMap<u32, u32> }

// Mesmos nomes do `packages/core/src/htmlPage.ts`.
const NAMES: [&str; 1] = ["mcp__hangar__html_render"];

pub fn page_from_result(tool_name: &str, result: &str) -> Option<PageRef> {
    if !NAMES.contains(&tool_name) { return None; }
    #[derive(Deserialize)]
    struct Out { hangar_page: PageRef }
    serde_json::from_str::<Out>(result).ok().map(|o| o.hangar_page)
}

pub fn frame_height(page: &PageRef, width: f32, reported: Option<f32>) -> f32 {
    let natural = reported.unwrap_or_else(|| {
        page.heights.iter().min_by_key(|(w, _)| (**w as f32 - width).abs() as u32).map_or(240., |(_, h)| *h as f32)
    });
    let capped = page.height.map_or(natural, |h| natural.min(h as f32));
    capped.clamp(MIN as f32, MAX as f32)
}

/// Fila das páginas vivas, mais recente no fim: passou do teto, a mais antiga sai.
#[derive(Default)]
pub struct Budget { order: VecDeque<String> }

impl Budget {
    /// Marca `id` como usada agora; devolve quem deve fechar.
    pub fn touch(&mut self, id: &str) -> Option<String> {
        self.order.retain(|x| x != id);
        self.order.push_back(id.to_owned());
        (self.order.len() > LIVE_MAX).then(|| self.order.pop_front()).flatten()
    }
    pub fn forget(&mut self, id: &str) { self.order.retain(|x| x != id); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_only_hangar_page() {
        let ok = r#"{"hangar_page":{"id":"a","title":"T","height":null,"heights":{"728":300}},"message":"x"}"#;
        assert_eq!(page_from_result("mcp__hangar__html_render", ok).unwrap().id, "a");
        assert!(page_from_result("mcp__hangar__html_render", r#"{"draft":{"id":"b"}}"#).is_none());
        assert!(page_from_result("Read", ok).is_none());
    }

    #[test]
    fn budget_evicts_oldest() {
        let mut b = Budget::default();
        for id in ["a", "b", "c", "d"] { assert_eq!(b.touch(id), None); }
        assert_eq!(b.touch("a"), None);
        assert_eq!(b.touch("e"), Some("b".into()));
    }

    #[test]
    fn height_rules() {
        let p = PageRef { id: "a".into(), title: "T".into(), height: Some(300), heights: [(728, 380)].into() };
        assert_eq!(frame_height(&p, 700., None), 300.);
        assert_eq!(frame_height(&PageRef { height: None, ..p.clone() }, 700., Some(5000.)), 2000.);
    }
}
```

- [ ] **Step 36: Estado do cartão e busca**

`struct PageView { page: PageRef, state: ViewState, reported: Option<f32>, engine: Option<Engine>, last_frame: Option<Arc<RenderImage>>, focused: bool }` com `enum ViewState { Loading, Ready, Error, Expired }`, guardado num `HashMap<String, PageView>` no app (chave = `page.id`) ao lado do estado de mídia (`:791`). A busca segue o `ensure_media` (`app.rs:2539`): na primeira renderização do cartão, `Api::page_raw` no runtime tokio; 404 → `Expired`; outro erro → `Error`; sucesso → guarda o HTML e, no Linux, cria o motor com `Engine::prepare(window, cx)?.start_page(&html, largura_da_coluna, tx)` e um laço `cx.spawn` que lê o `rx`:

- `Event::Frame` → `cx.notify()`;
- `Event::Host(json)` → `size-changed` atualiza `reported` e chama `remeasure_items` da linha (`app.rs:2893-2990`); `ui/open-link` com `http(s)` → `cx.open_url(url)`;
- primeiro quadro → manda o tema: `engine.evaluate(&format!("window.__hangarApply({})", json!({"theme": …, "styles": {"variables": vars}})))`, com `vars` montado de `theme::colors()` nos mesmos nomes da spec (`--foreground` = `text()`, `--muted-foreground` = `muted()`, `--surface` = `surface()`, `--border`, `--accent` = `accent()`, cores em `#rrggbb` a partir do `Hsla`). Repete no observador de aparência (`app.rs:854-858`).

Sem Chromium (`Engine::available()` falha) ou fora do Linux: estado estático, que busca `Api::page_shot(session, id, tema, largura)` e mostra a imagem com o mesmo caminho de `img(Arc<RenderImage>)` das miniaturas (`app.rs:1633`). 404 com `erro_pagina_sem_imagem` → só título e botão.

- [ ] **Step 37: Desenhar o cartão**

`render_page_card` devolve uma coluna com altura fixa `frame_height(...)` e largura cheia da coluna da resposta:

- **Ready (Linux)**: um `canvas` cujo paint chama `engine.place(bounds, window)` (igual a `app/browser.rs:449-452`), com os ouvintes de mouse do painel (`app/browser.rs:453-483`) traduzindo para coordenadas relativas ao cartão. A roda só vai para a página quando `page.height` limita ou a altura informada passa de 2000; senão, a roda segue para a lista (não chame `stop_propagation`). Clique dá foco ao cartão (`FocusHandle` próprio); com foco, teclas vão para `engine.key`; Esc devolve o foco à conversa. Antes do primeiro quadro, mostra o `last_frame` ou o texto `page_loading`.
- **Fora da tela**: a lista virtualizada não pinta a linha, então o `place` para de ser chamado. No começo de cada pintura da lista, marque todas as páginas como "não vistas"; o paint do cartão marca a sua; num `on_next_frame`, quem não foi visto recebe `engine.hide()`. Ao voltar, `Budget::touch(id)`; se devolver um id, o `PageView` dele guarda o último quadro como `RenderImage` e solta o `engine` (o `Drop` fecha alvo e contexto).
- **Estático**: imagem + linha com o título e botão `page_open_browser` (`cx.open_url` na casca isolada: `Api::endpoint(session, "pages/<id>")` com `?token=`; é a mesma decisão do link "nova aba" de arquivos).
- **Error**: texto `page_error` + botão `page_retry` que zera o estado e busca de novo. **Expired**: texto `page_expired`.

No `render_tool` (`app.rs:3300`), antes do `render_agent_card`: se o `tool_result` pareado tem `page_from_result(...)`, devolva `render_page_card`. No `conversation::build` (`conversation.rs:53,127`), trate a chamada de `html_render` como o `Agent`: sai do grupo.

- [ ] **Step 38: Compilar e build otimizado**

Run: `cd desktop-native && nice -n 19 cargo build --release`
Expected: build sem erro; binário em `~/.cache/cargo/target/release/`.

- [ ] **Step 39: Verificação manual — nativo Linux com print**

Com o backend da Task 4 de pé e páginas publicadas em `cx-paginas`:
1. Abra o nativo do build novo (sem fechar o que o Jefferson usa sem avisar; se for preciso trocar, pergunte).
2. Na conversa `cx-paginas`: a página aparece viva; hover muda estilo; um botão com script altera a página; link abre no navegador do sistema.
3. Troque o tema (claro/escuro/papel de parede): a página troca e não há retângulo opaco sobre o vidro.
4. Role para longe e volte: a página volta; publique 6 páginas e confira que só 4 ficam vivas (as outras mostram o último quadro e recarregam ao voltar).
5. Feche a sessão e reabra a conversa: "expirou".
6. Print da janela (o print do nativo descrito na memória `feedback-conferir-no-nativo-com-print`) e cite o caminho.

- [ ] **Step 40: Commit**

```bash
git add desktop-native/src/app/page_card.rs desktop-native/src/app.rs desktop-native/src/conversation.rs desktop-native/src/api/mod.rs messages/pt.json messages/en.json
git commit -m "feat(native): conversation page card, live on Linux and static elsewhere"
```

### Task 9: Prova da WebView2 fora da tela na DELPHI-02
Status: ready-for-agent
Risk: medium

**Files:**
- Modify: `docs/decisoes/frontend.md` (entrada nova "Página da conversa no Windows: WebView2 fora da tela")

**Interfaces:**
- Produces: decisão registrada (passou / não passou) que destrava ou cancela a Task 10.

- [ ] **Step 41: Verificação manual — screencast da WebView2 estacionada**

Na DELPHI-02 (clone separado, nunca o Hangar vivo de lá; regra da memória `feedback-testar-windows-na-vm`), com o nativo desta branch compilado lá:
1. Abra o navegador do painel de uma sessão numa página animada (`data:text/html,<div id=c></div><script>setInterval(()=>c.textContent=Date.now(),50)</script>`).
2. Estacione a WebView2 em x=-3000 pelo caminho que o `hide` já usa (`wry_engine.rs:135-155`).
3. Por `CallDevToolsProtocolMethod` (`browser/cdp.rs`), `Page.startScreencast {format:"png"}` e conte `Page.screencastFrame` por 10 s (cada um com `Page.screencastFrameAck`).
4. Mande `Input.dispatchMouseEvent` num botão da página e confira, por `Runtime.evaluate`, que o clique chegou (ouvinte em captura, regra de `frontend.md`).
5. Minimize o app e repita 3.

- [ ] **Step 42: Registrar a decisão**

Entrada nova em `docs/decisoes/frontend.md` com: data, versão da WebView2, quadros por segundo estacionada e minimizada, se o clique chegou, e a decisão (Task 10 segue ou Windows fica estático). Commit:

```bash
git add docs/decisoes/frontend.md
git commit -m "docs(decisions): WebView2 off-screen screencast proof for conversation pages"
```

### Task 10: Página viva no Windows
Status: needs-info
Risk: high

Só entra se a Task 9 passou. Sem isso, marque `Status: wontfix` e o Windows fica no cartão estático da Task 8.

**Files:**
- Modify: `desktop-native/src/browser/wry_engine.rs` (modo página: WebView2 própria estacionada, screencast em PNG, quadros para uma textura da GPUI como no `Surface` do Linux)
- Modify: `desktop-native/src/app/page_card.rs` (ligar o modo vivo também em `cfg(windows)`)

**Interfaces:**
- Consumes: `PageView`, `Budget`, `frame_height` (Task 8)
- Produces: `Engine::start_page` também no Windows, mesma assinatura do Linux

- [ ] **Step 43: Implementar o modo página na WebView2**

`Starter::start_page` cria uma WebView2 filha estacionada em x=-3000 com o tamanho do cartão, `NavigateToString(html)` (limite de 2 MB da API: acima disso, grave o HTML num arquivo temporário e use `SetVirtualHostNameToFolderMapping` com um host `hangar-page.invalid` só para essa pasta), `AddHostObjectToScript` não: a ponte usa `window.chrome.webview.postMessage` — acrescente esse caminho ao script de ponte do servidor (`theme.rs`, `HOST`: `else if(window.chrome&&window.chrome.webview)window.chrome.webview.postMessage(s)`) e trate `WebMessageReceived` como `Event::Host`. Quadros por `Page.startScreencast` via `CallDevToolsProtocolMethod`, decodificados para a mesma `wgpu::Texture` do `Surface`; entrada por `Input.dispatchMouseEvent`/`dispatchKeyEvent` como no Linux. Fundo transparente: `DefaultBackgroundColor` com alfa 0 na criação.

- [ ] **Step 44: Verificação manual — DELPHI-02**

Repita a Step 39 na DELPHI-02 (clone separado), com print da janela.

- [ ] **Step 45: Commit**

```bash
git add desktop-native/src/browser/wry_engine.rs desktop-native/src/app/page_card.rs crates/hangar-server/src/pages/theme.rs
git commit -m "feat(native): live conversation pages on Windows via off-screen WebView2 screencast"
```

### Task 11: Página no app Expo
Status: ready-for-agent
Risk: medium

**Files:**
- Create: `mobile/src/chat/tools/HtmlPageCard.tsx`
- Modify: `mobile/src/chat/MessageList.tsx:152-191` (item `tool` com página vira `HtmlPageCard`)
- Modify: `mobile/src/chat/tools/fold.ts:17` (página fora do `fold`, se o `agruparConversa` da Task 5 não bastar)

**Interfaces:**
- Consumes: `htmlPageFromResult`, `frameHeight`, `pageFetchState`, `pageUrls`, `themeVariables` (Task 5); `fileAuthHeader` (`packages/core/src/api.ts:125`)
- Produces: `<HtmlPageCard page session server />`

- [ ] **Step 46: Usar as skills `vercel-react-native-skills` e `expo-native-ui`, depois implementar**

```tsx
import { useCallback, useEffect, useState } from 'react';
import { Linking, Pressable, Text, View, useWindowDimensions } from 'react-native';
import { WebView } from 'react-native-webview';
import { frameHeight, pageFetchState, pageUrls, type HtmlPageRef, type PageFetchState } from '@hangar/core/htmlPage';
import * as m from '../../paraglide/messages';

type Props = { page: HtmlPageRef; base: string; session: string; authHeader: Record<string, string>; themed: (html: string) => string };

export function HtmlPageCard({ page, base, session, authHeader, themed }: Props) {
  const { width } = useWindowDimensions();
  const [state, setState] = useState<PageFetchState>('loading');
  const [html, setHtml] = useState('');
  const [reported, setReported] = useState<number | null>(null);

  const load = useCallback(async () => {
    setState('loading');
    try {
      const r = await fetch(pageUrls(base, session, page.id).raw, { headers: authHeader });
      const s = pageFetchState(r.status);
      if (s === 'ready') setHtml(themed(await r.text()));
      setState(s);
    } catch {
      setState(pageFetchState('network'));
    }
  }, [base, session, page.id, authHeader, themed]);

  useEffect(() => { load(); }, [load]);

  const height = frameHeight(page, width - 32, reported);
  if (state === 'loading') return <Text style={{ height }}>{m.page_loading({ title: page.title })}</Text>;
  if (state === 'expired') return <Text>{m.page_expired()}</Text>;
  if (state === 'error') return (
    <View><Text>{m.page_error({ title: page.title })}</Text><Pressable onPress={load}><Text>{m.page_retry()}</Text></Pressable></View>
  );
  return (
    <WebView
      style={{ height, backgroundColor: 'transparent' }}
      source={{ html }}
      originWhitelist={['about:*']}
      scrollEnabled={page.height != null}
      onShouldStartLoadWithRequest={(r) => r.url === 'about:blank' || r.url.startsWith('about:srcdoc')}
      onMessage={(e) => {
        try {
          const d = JSON.parse(e.nativeEvent.data);
          if (d?.method === 'ui/notifications/size-changed' && typeof d.params?.height === 'number') setReported(d.params.height);
          else if (d?.method === 'ui/open-link' && /^https?:/i.test(d.params?.url ?? '')) Linking.openURL(d.params.url);
        } catch { /* mensagem que não é da ponte */ }
      }}
    />
  );
}
```

Antes de escrever, confira no `mobile/` como o tema atual é lido (para montar `themed`, mesma troca de bloco do PWA, usando `themeVariables`) e como o `AssistantBubble.tsx` obtém `base` e o cabeçalho de token (`:119, 210, 234`); passe-os pelo `MessageList`. Estilo: siga os componentes vizinhos (`ToolCard.tsx`), não os `Text` crus do esboço. Troca de tema com o cartão montado: `webviewRef.current?.injectJavaScript("window.__hangarApply(" + JSON.stringify(params) + ");true")`.

- [ ] **Step 47: Verificação manual — app no simulador ou aparelho**

Abra a conversa `cx-paginas` no app (build de desenvolvimento): página aparece, altura acompanha, link abre no navegador do sistema, troca de tema reflete, sessão encerrada mostra "expirou". Print da tela citado pelo caminho.

- [ ] **Step 48: Commit**

```bash
git add mobile/src/chat/tools/HtmlPageCard.tsx mobile/src/chat/MessageList.tsx mobile/src/chat/tools/fold.ts
git commit -m "feat(mobile): render html_render pages in a WebView"
```

### Task 12: Regra no `CLAUDE.md`, decisão e verificação final
Status: ready-for-agent
Risk: low

**Files:**
- Modify: `CLAUDE.md` (marcador novo em "Plataforma")
- Modify: `docs/decisoes/plataforma.md` (entrada de mesmo título)

- [ ] **Step 49: Escrever a regra e a decisão**

Marcador em "Plataforma", depois do de "HTML servido como arquivo executa isolado":

```markdown
- **Página da conversa (`html_render`) mora no Rust e some com a sessão.** Publicação só pela ponte
  privada (`/__hangar_server/pages`, MCP); leitura pelo dono; convidado segue ao Python e não vê.
  Nos apps, nunca `allow-same-origin` nem token na URL do documento: PWA por `srcdoc` com fetch
  autenticado, nativo num contexto de navegador próprio. Evidência em
  [plataforma.md](docs/decisoes/plataforma.md#página-da-conversa-mora-no-rust-e-some-com-a-sessão).
```

Entrada em `docs/decisoes/plataforma.md` com o título exato do marcador: o porquê de rascunho em vez de `html_preview`, de varredura por `jsonl` em vez de gancho no fechamento, de PNG no screencast da página, do contexto de navegador separado, e o resultado da Task 9.

- [ ] **Step 50: Verificação final (testes só se o Jefferson pedir)**

Quando o Jefferson pedir os testes focados, num comando só por linguagem:

```bash
cd crates && nice -n 19 cargo test -p hangar-server pages:: list::bridge
cd backend && uv run pytest tests/test_pages_bridge.py
cd packages/core && npx vitest run src/htmlPage.test.ts
cd desktop-native && nice -n 19 cargo test page_card
```

Falhou → repita só o que falhou. Sem pedido, reporte o que foi conferido no uso real (Steps 9, 22, 29, 39, 47) e que os testes automatizados não rodaram; o `pre-push` roda os ligados aos arquivos tocados.

- [ ] **Step 51: Commit**

```bash
git add CLAUDE.md docs/decisoes/plataforma.md
git commit -m "docs: rule and decision record for conversation pages"
```
