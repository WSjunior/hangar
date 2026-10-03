//! Descoberta dos arquivos sem seguir links de diretórios.

use std::path::{Path, PathBuf};

pub fn list_files(root: &Path, matches: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    let mut output = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(path) else { continue };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() { stack.push(entry.path()); }
            else if matches(&entry.file_name().to_string_lossy()) { output.push(entry.path()); }
        }
    }
    output
}
