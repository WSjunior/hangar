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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inject_goes_first_in_head() {
        let out = inject("<!doctype html><html><head><style>p{}</style></head><body>x</body></html>");
        let head = out.find("<head>").unwrap();
        let ours = out.find("<style id=\"hangar-theme\"").unwrap();
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
