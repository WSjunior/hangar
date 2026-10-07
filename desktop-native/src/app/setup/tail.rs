//! Acompanha o arquivo de saída do script destacado: lê só o que cresceu desde a última leitura, em linhas inteiras.
use std::{io::{Read, Seek, SeekFrom}, path::{Path, PathBuf}};

pub(crate) struct Tail { path: PathBuf, offset: u64, partial: Vec<u8> }

impl Tail {
    pub(crate) fn new(path: PathBuf) -> Self { Self { path, offset: 0, partial: Vec::new() } }

    pub(crate) fn path(&self) -> &Path { &self.path }

    /// Linhas completas novas; o pedaço sem `\n` espera a próxima leitura. Inválido vira U+FFFD (`windows.md`).
    pub(crate) fn read_new(&mut self) -> std::io::Result<Vec<String>> {
        let mut file = std::fs::File::open(&self.path)?;
        let len = file.metadata()?.len();
        // Arquivo recomeçado (outra execução no mesmo nome): lê do começo.
        if len < self.offset { self.offset = 0; self.partial.clear(); }
        file.seek(SeekFrom::Start(self.offset))?;
        let mut chunk = Vec::new();
        file.take(len - self.offset).read_to_end(&mut chunk)?;
        self.offset += chunk.len() as u64;
        self.partial.extend_from_slice(&chunk);
        let mut lines = Vec::new();
        while let Some(at) = self.partial.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=at).collect();
            lines.push(String::from_utf8_lossy(&line[..line.len() - 1]).trim_end_matches('\r').to_owned());
        }
        Ok(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_whole_lines_across_chunks_and_bad_bytes() {
        let dir = std::env::temp_dir().join(format!("hangar-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("saida.log");
        let mut file = std::fs::File::create(&path).unwrap();
        let mut tail = Tail::new(path.clone());
        file.write_all(b"##HANGAR-PASSO## preparar fazendo\r\nmeia ").unwrap();
        file.flush().unwrap();
        assert_eq!(tail.read_new().unwrap(), vec!["##HANGAR-PASSO## preparar fazendo".to_owned()]);
        file.write_all(b"linha\nacentua\xe7\xe3o\n").unwrap();
        file.flush().unwrap();
        // Bytes que não são UTF-8 (cp1252 no Windows) viram U+FFFD em vez de derrubar a leitura.
        assert_eq!(tail.read_new().unwrap(), vec!["meia linha".to_owned(), "acentua\u{fffd}\u{fffd}o".to_owned()]);
        assert!(tail.read_new().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_file_is_an_error_not_a_panic() {
        let mut tail = Tail::new(std::env::temp_dir().join("hangar-tail-nao-existe.log"));
        assert!(tail.read_new().is_err());
    }
}
