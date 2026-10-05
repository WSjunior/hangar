//! Áreas dos alvos guardados por turno, reconstruídas com o mapa atual.

use super::rows::UsoLinha;
use super::uso_rules::python_space;
use indexmap::{IndexMap, IndexSet};
use md5::{Digest, Md5};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolReg {
    P { rules_cwd: String, cwd: String, paths: Vec<String> },
    C { rules_cwd: String, cwd: String, candidates: Vec<String> },
    S { rules_cwd: String, target: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Unit {
    pub dia: String,
    pub cwd: String,
    pub model: String,
    pub fast: bool,
    pub values: [i64; 5],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AreaHeader {
    pub fonte: Option<String>,
    pub session_id: Option<String>,
    pub subagente: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AreaEntries {
    pub header: AreaHeader,
    pub turns: Vec<(Vec<ToolReg>, Vec<Unit>)>,
}

type Rules = Vec<(String, Vec<String>)>;

pub struct AreaMap {
    defaults: Rules,
    projects: IndexMap<String, Rules>,
    signature: String,
    matchers: Mutex<HashMap<Vec<String>, Regex>>,
    roots: Mutex<HashMap<String, String>>,
}

fn default_rules() -> Rules {
    [
        ("banco", vec!["*.sql", "migrations/*", "prisma/*", "skill:*database*"]),
        ("infra", vec!["Dockerfile*", "*.dockerfile", "docker-compose*", "k8s/*", "helm/*", "charts/*", ".github/*", ".gitlab-ci.yml", "Jenkinsfile", "scripts/*", "deploy/*", "install.*", "*.sh", "*.ps1"]),
        ("docs", vec!["docs/*", "*.md"]),
        ("front", vec!["frontend/*", "packages/core/*", "mobile/*", "messages/*", "locales/*", "*.svelte", "*.tsx", "*.jsx", "*.css", "*.scss", "*.html", "*.dart"]),
        ("back", vec!["backend/*", "*pserver*", "*.py", "*.cs", "*.pas", "*.go"]),
    ].into_iter().map(|(name, patterns)| (name.into(), patterns.into_iter().map(String::from).collect())).collect()
}

fn parse_rules(raw: &Value) -> Rules {
    raw.as_array().into_iter().flatten().filter_map(|item| {
        let pair = item.as_array()?;
        if pair.len() != 2 { return None; }
        let name = pair[0].as_str()?;
        let patterns = pair[1].as_array()?.iter().filter_map(Value::as_str).map(String::from).collect();
        Some((name.into(), patterns))
    }).collect()
}

impl AreaMap {
    pub fn load(file: &Path) -> Self {
        let (text, raw) = std::fs::read_to_string(file).ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok().map(|raw| (text, raw)))
            .unwrap_or_else(|| (String::new(), Value::Null));
        let defaults = default_rules();
        // O padrão do código participa da assinatura para invalidar áreas após uma atualização.
        // A seleção corrigida precisa reconstruir também as áreas já salvas no índice.
        let signature = format!("{:x}", Md5::digest(format!("divisao:3project-paths:1{}{}", serde_json::to_string(&defaults).unwrap(), text).as_bytes()));
        let mut projects = IndexMap::new();
        let defaults = if let Some(obj) = raw.as_object() {
            if let Some(project_rules) = obj.get("projetos").and_then(Value::as_object) {
                projects = project_rules.iter().map(|(key, rules)| (key.clone(), parse_rules(rules))).collect();
            }
            obj.get("padrao").map(parse_rules).unwrap_or(defaults)
        } else { defaults };
        Self { defaults, projects, signature, matchers: Mutex::new(HashMap::new()), roots: Mutex::new(HashMap::new()) }
    }

    pub fn signature(&self) -> &str { &self.signature }

    pub fn rules_for(&self, cwd: &str) -> Rules {
        // O transcript pode usar barras diferentes das do host que lê o mapa.
        let cwd = cwd.replace('\\', "/");
        let mut rules = Vec::new();
        let parts = project_parts(&cwd);
        for (key, project_rules) in &self.projects {
            let key = key.replace('\\', "/");
            let matches = if key.contains('/') {
                cwd == key || cwd.starts_with(&format!("{}/", key.trim_end_matches('/')))
            } else { parts.contains(&key) };
            if matches { rules.extend(project_rules.clone()); }
        }
        rules.extend(self.defaults.clone());
        rules
    }

    pub fn area_of_target(&self, target: &str, rules: &[(String, Vec<String>)]) -> Option<String> {
        let target = format!("/{target}");
        let mut cache = self.matchers.lock().unwrap_or_else(|e| e.into_inner());
        for (area, patterns) in rules {
            if patterns.is_empty() { continue; }
            if !cache.contains_key(patterns) {
                let alternatives = patterns.iter().map(|p| format!("/{}|{}", translate(p), translate(&format!("*/{p}")))).collect::<Vec<_>>().join("|");
                let matcher = Regex::new(&format!("^(?:{alternatives})")).expect("padrão fnmatch traduzido");
                if cache.len() >= 1024 { cache.clear(); }
                cache.insert(patterns.clone(), matcher);
            }
            if cache.get(patterns).unwrap().is_match(&target) { return Some(area.clone()); }
        }
        None
    }

    fn repo_root(&self, directory: &str) -> String {
        let mut cache = self.roots.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(root) = cache.get(directory) { return root.clone(); }
        let path = Path::new(if directory.is_empty() { "." } else { directory });
        let mut root = String::new();
        for parent in path.ancestors() {
            let parent = if parent.as_os_str().is_empty() { Path::new(".") } else { parent };
            if parent.join(".git").exists() {
                root = parent.to_string_lossy().into_owned();
                break;
            }
        }
        if cache.len() >= 4096 { cache.clear(); }
        cache.insert(directory.into(), root.clone());
        root
    }

    pub fn area_of_path(&self, path: &str, cwd: &str, rules: &[(String, Vec<String>)]) -> String {
        let path = normalize_path(&if absolute_path(path) || cwd.is_empty() { path.into() } else { join_path(cwd, path) });
        let mut root = self.repo_root(&dirname(&path));
        let own_rules;
        let rules = if !root.is_empty() && root != self.repo_root(cwd) {
            own_rules = self.rules_for(&root);
            &own_rules[..]
        } else {
            if root.is_empty() {
                root = if cwd.is_empty() { String::new() } else {
                    let cwd_root = self.repo_root(cwd);
                    if cwd_root.is_empty() { cwd.into() } else { cwd_root }
                };
            }
            rules
        };
        if !inside(&path, &root) { return "outros".into(); }
        self.area_of_target(&relative_path(&path, &root), rules).filter(|area| !area.is_empty()).unwrap_or_else(|| "outros".into())
    }

    pub fn count_areas(&self, regs: &[ToolReg]) -> IndexMap<String, i64> {
        let mut counts = IndexMap::new();
        for reg in regs {
            let (rules_cwd, cwd, targets, command) = match reg {
                ToolReg::S { rules_cwd, target } => {
                    if let Some(area) = self.area_of_target(target, &self.rules_for(rules_cwd)).filter(|area| !area.is_empty()) { *counts.entry(area).or_insert(0) += 1; }
                    continue;
                }
                ToolReg::P { rules_cwd, cwd, paths } => (rules_cwd, cwd, paths, false),
                ToolReg::C { rules_cwd, cwd, candidates } => (rules_cwd, cwd, candidates, true),
            };
            let rules = self.rules_for(rules_cwd);
            let distinct: IndexSet<_> = targets.iter().map(|p| self.area_of_path(p, cwd, &rules))
                .filter(|area| !command || area != "outros").collect();
            let mut distinct: Vec<_> = distinct.into_iter().collect();
            distinct.sort();
            for area in distinct { *counts.entry(area).or_insert(0) += 1; }
        }
        counts
    }

    pub fn area_lines(&self, entries: &AreaEntries) -> Vec<UsoLinha> {
        let mut rows: IndexMap<(String, String, String, String), UsoLinha> = IndexMap::new();
        for (regs, units) in &entries.turns {
            let counts = self.count_areas(regs);
            let weights = if counts.is_empty() { IndexMap::from([("conversa".into(), 1)]) } else { counts.clone() };
            for (index, unit) in units.iter().enumerate() {
                let split = unit.values.map(|v| repartir(v, &weights));
                for area in weights.keys() {
                    let key = (unit.dia.clone(), unit.cwd.clone(), unit.model.clone(), area.clone());
                    let row = rows.entry(key).or_insert_with(|| UsoLinha {
                        dia: unit.dia.clone(), cwd: unit.cwd.clone(), model: unit.model.clone(), tipo: "area".into(), nome: area.clone(),
                        fonte: entries.header.fonte.clone().unwrap_or_else(|| "claude".into()),
                        session_id: entries.header.session_id.clone().unwrap_or_default(),
                        subagente: entries.header.subagente.unwrap_or(false), ..UsoLinha::default()
                    });
                    if index == 0 { row.chamadas += counts.get(area).copied().unwrap_or(0); }
                    row.input += split[0][area];
                    row.output += split[1][area];
                    row.cache_write += split[2][area];
                    row.cache_read += split[3][area];
                    row.cache_write_1h += split[4][area].max(0).min(split[2][area].max(0));
                    row.fast |= unit.fast;
                }
            }
        }
        rows.into_values().collect()
    }
}

pub fn repartir(value: i64, weights: &IndexMap<String, i64>) -> IndexMap<String, i64> {
    let total: i128 = weights.values().map(|n| i128::from(*n)).sum();
    assert!(weights.is_empty() || total != 0, "soma dos pesos nula");
    let exact: IndexMap<_, _> = weights.iter().map(|(area, n)| (area.clone(), integer_ratio(i128::from(value) * i128::from(*n), total))).collect();
    let mut integers: IndexMap<_, _> = exact.iter().map(|(area, x)| {
        // MAX convertido em f64 já vale 2^63; a comparação superior precisa ser exclusiva.
        assert!(*x >= -9223372036854775808.0 && *x < 9223372036854775808.0, "area_allocation_out_of_range");
        (area.clone(), *x as i64)
    }).collect();
    let sum: i128 = integers.values().map(|n| i128::from(*n)).sum();
    let mut order: Vec<_> = exact.keys().collect();
    order.sort_by(|a, b| ((integers[*a] as f64) - exact[*a]).total_cmp(&((integers[*b] as f64) - exact[*b])).then_with(|| a.cmp(b)));
    let remaining = i128::from(value) - sum;
    // O slice negativo do Python exclui a cauda, mesmo para valores negativos.
    let count = if remaining < 0 { (order.len() as i128 + remaining).max(0) } else { remaining.min(order.len() as i128) } as usize;
    for area in order.into_iter().take(count) { integers[area] += 1; }
    integers
}

fn integer_ratio(numerator: i128, denominator: i128) -> f64 {
    if numerator == 0 || denominator == 0 { return numerator as f64 / denominator as f64; }
    let negative = (numerator < 0) != (denominator < 0);
    let n = numerator.unsigned_abs();
    let d = denominator.unsigned_abs();
    let mut exponent = d.leading_zeros() as i32 - n.leading_zeros() as i32;
    if if exponent >= 0 { n < d << exponent } else { n << -exponent < d } { exponent -= 1; }
    let shift = 52 - exponent;
    let (mut quotient, remainder, divisor) = if shift >= 0 {
        let mut quotient = n / d;
        let mut remainder = n % d;
        for _ in 0..shift {
            quotient <<= 1;
            remainder <<= 1;
            if remainder >= d { quotient += 1; remainder -= d; }
        }
        (quotient, remainder, d)
    } else {
        let divisor = d << -shift;
        (n / divisor, n % divisor, divisor)
    };
    // Converter o produto antes de dividir arredonda duas vezes; o Python arredonda a razão.
    let doubled = remainder << 1;
    if doubled > divisor || (doubled == divisor && quotient & 1 != 0) { quotient += 1; }
    let result = quotient as f64 * 2.0f64.powi(exponent - 52);
    if negative { -result } else { result }
}

pub fn candidates(cmd: &str) -> Vec<String> {
    cmd.replace('=', " ").split(python_space).filter_map(|token| {
        let target = token.trim_matches(['\'', '"', '(', ')', ';', '&', '|', '<', '>']);
        if target.is_empty() || target.starts_with('-') || target.contains("://") || target.contains('$') { return None; }
        if !target.contains('/') && !target.trim_start_matches('.').contains('.') { return None; }
        Some(target.into())
    }).collect()
}

pub fn default_map_file() -> PathBuf {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE").or_else(|| {
        Some(PathBuf::from(std::env::var_os("HOMEDRIVE")?).join(std::env::var_os("HOMEPATH")?).into_os_string())
    });
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    PathBuf::from(home.unwrap_or_default()).join(".hangar").join("uso-areas.json")
}

fn project_parts(path: &str) -> Vec<String> {
    let parts: Vec<_> = Path::new(path).components().filter(|c| !matches!(c, std::path::Component::CurDir))
        .map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    #[cfg(windows)]
    {
        let mut parts = parts;
        let mut components = Path::new(path).components();
        if matches!(components.next(), Some(std::path::Component::Prefix(_)))
            && matches!(components.next(), Some(std::path::Component::RootDir)) {
            let root = parts.remove(1);
            parts[0].push_str(&root);
        }
        parts
    }
    #[cfg(not(windows))]
    { parts }
}

fn translate(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        match c {
            '*' => { out.push_str(".*"); while chars.get(i) == Some(&'*') { i += 1; } }
            '?' => out.push('.'),
            '[' => {
                let mut j = i;
                if chars.get(j) == Some(&'!') { j += 1; }
                if chars.get(j) == Some(&']') { j += 1; }
                while j < chars.len() && chars[j] != ']' { j += 1; }
                if j == chars.len() { out.push_str(r"\["); } else {
                    out.push_str(&translate_set(&chars[i..j]));
                    i = j + 1;
                }
            }
            _ => out.push_str(&regex::escape(&c.to_string())),
        }
    }
    format!("(?s:{out})\\z")
}

fn translate_set(chars: &[char]) -> String {
    let mut chunks: Vec<Vec<char>> = Vec::new();
    let mut start = 0;
    let mut search = if chars.first() == Some(&'!') { 2 } else { 1 };
    while let Some(offset) = chars.get(search..).and_then(|s| s.iter().position(|c| *c == '-')) {
        let index = search + offset;
        chunks.push(chars[start..index].to_vec());
        start = index + 1;
        search = index + 3;
    }
    if start < chars.len() { chunks.push(chars[start..].to_vec()); }
    else if let Some(last) = chunks.last_mut() { last.push('-'); }
    for index in (1..chunks.len()).rev() {
        if chunks[index - 1].last() > chunks[index].first() {
            chunks[index - 1].pop();
            let tail: Vec<_> = chunks[index].iter().skip(1).copied().collect();
            chunks[index - 1].extend(tail);
            chunks.remove(index);
        }
    }
    let mut stuff = chunks.iter().map(|chunk| chunk.iter().map(|c| match c {
        '\\' | '-' | '&' | '~' | '|' | '[' | ']' | '^' => format!("\\{c}"),
        _ => c.to_string(),
    }).collect::<String>()).collect::<Vec<_>>().join("-");
    if stuff.is_empty() { return r"[^\s\S]".into(); }
    if stuff == "!" { return ".".into(); }
    if stuff.starts_with('!') { stuff.replace_range(..1, "^"); }
    format!("[{stuff}]")
}

#[cfg(not(windows))]
fn absolute_path(path: &str) -> bool { path.starts_with('/') }

#[cfg(not(windows))]
fn join_path(base: &str, path: &str) -> String {
    if absolute_path(path) || base.is_empty() { path.into() }
    else if base.ends_with('/') { format!("{base}{path}") }
    else { format!("{base}/{path}") }
}

#[cfg(not(windows))]
fn normalize_path(path: &str) -> String {
    let root = if path.starts_with("//") && !path.starts_with("///") { "//" } else if path.starts_with('/') { "/" } else { "" };
    let mut parts = Vec::new();
    for part in path.split('/') {
        if part.is_empty() || part == "." { continue; }
        if part == ".." && parts.last().is_some_and(|p| *p != "..") { parts.pop(); }
        else if part != ".." || root.is_empty() { parts.push(part); }
    }
    let result = format!("{root}{}", parts.join("/"));
    if result.is_empty() { ".".into() } else { result }
}

#[cfg(windows)]
fn split_root(path: &str) -> (String, String, String) {
    let path = path.replace('/', "\\");
    if path.starts_with("\\\\") {
        let start = if path.get(..8).is_some_and(|s| s.eq_ignore_ascii_case("\\\\?\\UNC\\")) { 8 } else { 2 };
        if let Some(first) = path[start..].find('\\').map(|i| start + i) {
            if let Some(second) = path[first + 1..].find('\\').map(|i| first + 1 + i) {
                return (path[..second].into(), "\\".into(), path[second + 1..].into());
            }
        }
        (path, String::new(), String::new())
    } else if path.chars().nth(1) == Some(':') {
        let drive_end = path.chars().next().unwrap().len_utf8() + 1;
        if path.as_bytes().get(drive_end) == Some(&b'\\') { (path[..drive_end].into(), "\\".into(), path[drive_end + 1..].into()) }
        else { (path[..drive_end].into(), String::new(), path[drive_end..].into()) }
    } else if path.starts_with('\\') { (String::new(), "\\".into(), path[1..].into()) }
    else { (String::new(), String::new(), path) }
}

#[cfg(windows)]
fn absolute_path(path: &str) -> bool {
    let path = path.replace('/', "\\");
    let first: Vec<_> = path.chars().take(3).collect();
    path.starts_with("\\\\") || first.get(1..3) == Some(&[':', '\\'][..])
}

#[cfg(windows)]
fn join_path(base: &str, path: &str) -> String {
    let (mut drive, mut root, mut tail) = split_root(base);
    let (pdrive, proot, ptail) = split_root(path);
    if !proot.is_empty() {
        if !pdrive.is_empty() || drive.is_empty() { drive = pdrive; }
        root = proot; tail = ptail;
    } else if !pdrive.is_empty() && pdrive.to_lowercase() != drive.to_lowercase() {
        drive = pdrive; root = proot; tail = ptail;
    } else {
        if !pdrive.is_empty() { drive = pdrive; }
        if !tail.is_empty() && !tail.ends_with('\\') { tail.push('\\'); }
        tail.push_str(&ptail);
    }
    if !tail.is_empty() && root.is_empty() && !drive.is_empty() && !drive.ends_with([':', '\\']) { drive.push('\\'); }
    format!("{drive}{root}{tail}")
}

#[cfg(windows)]
fn normalize_path(path: &str) -> String {
    let (drive, root, tail) = split_root(path);
    let mut parts = Vec::new();
    for part in tail.split('\\') {
        if part.is_empty() || part == "." { continue; }
        if part == ".." && parts.last().is_some_and(|p| *p != "..") { parts.pop(); }
        else if part != ".." || root.is_empty() { parts.push(part); }
    }
    let result = format!("{drive}{root}{}", parts.join("\\"));
    if result.is_empty() { ".".into() } else { result }
}

fn dirname(path: &str) -> String {
    #[cfg(windows)]
    let (drive, root, tail) = split_root(path);
    #[cfg(not(windows))]
    let (drive, root, tail) = (String::new(), String::new(), path.to_owned());
    let sep = std::path::MAIN_SEPARATOR;
    let head = tail.rfind(sep).map_or("", |i| &tail[..=i]);
    let head = if head.chars().any(|c| c != sep) { head.trim_end_matches(sep) } else { head };
    format!("{drive}{root}{head}")
}

fn normcase(path: &str) -> String {
    #[cfg(windows)]
    { path.replace('/', "\\").to_lowercase() }
    #[cfg(not(windows))]
    { path.into() }
}

fn inside(path: &str, root: &str) -> bool {
    if root.is_empty() { return false; }
    let path = normcase(&normalize_path(path));
    let root = normcase(&normalize_path(root));
    path == root || path.starts_with(&format!("{}{}", root.trim_end_matches(std::path::MAIN_SEPARATOR), std::path::MAIN_SEPARATOR))
}

fn relative_path(path: &str, root: &str) -> String {
    let absolute = |p: &str| {
        if absolute_path(p) { normalize_path(p) }
        else { normalize_path(&join_path(&std::env::current_dir().unwrap_or_default().to_string_lossy(), p)) }
    };
    let path = absolute(path);
    let root = absolute(root);
    let sep = std::path::MAIN_SEPARATOR;
    let paths: Vec<_> = path.split(sep).filter(|p| !p.is_empty()).collect();
    let roots: Vec<_> = root.split(sep).filter(|p| !p.is_empty()).collect();
    let common = paths.iter().zip(&roots).take_while(|(a, b)| normcase(a) == normcase(b)).count();
    let mut parts = vec![".."; roots.len() - common];
    parts.extend_from_slice(&paths[common..]);
    if parts.is_empty() { ".".into() } else { parts.join("/") }
}
