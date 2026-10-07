//! Acompanha o arquivo de saída do script destacado: lê só o que cresceu desde a última leitura, em linhas inteiras.
use std::{io::{Read, Seek, SeekFrom}, path::PathBuf};

pub(crate) struct Tail { path: PathBuf, offset: u64, partial: Vec<u8> }

/// Teto por leitura: roda na thread da janela, e um log grande (reabertura do app) não pode travá-la.
const READ_CAP: u64 = 1 << 20;
/// Linha sem `\n` maior que isto sai assim mesmo: senão o pedaço cresce sem fim.
const LINE_CAP: usize = 64 << 10;

fn text(line: &[u8]) -> String { String::from_utf8_lossy(line).trim_end_matches('\r').to_owned() }

impl Tail {
    pub(crate) fn new(path: PathBuf) -> Self { Self { path, offset: 0, partial: Vec::new() } }

    /// Linhas completas novas; o pedaço sem `\n` espera a próxima leitura. Inválido vira U+FFFD (`windows.md`).
    pub(crate) fn read_new(&mut self) -> std::io::Result<Vec<String>> { Ok(self.read_chunk()?.0) }

    /// Lê até `READ_CAP` bytes; o `bool` diz se ainda sobrou arquivo para ler.
    fn read_chunk(&mut self) -> std::io::Result<(Vec<String>, bool)> {
        let mut file = std::fs::File::open(&self.path)?;
        let len = file.metadata()?.len();
        // Arquivo recomeçado (outra execução no mesmo nome): lê do começo.
        if len < self.offset { self.offset = 0; self.partial.clear(); }
        file.seek(SeekFrom::Start(self.offset))?;
        let mut chunk = Vec::new();
        file.take((len - self.offset).min(READ_CAP)).read_to_end(&mut chunk)?;
        self.offset += chunk.len() as u64;
        self.partial.extend_from_slice(&chunk);
        let mut lines = Vec::new();
        let mut start = 0;
        while let Some(at) = self.partial[start..].iter().position(|&b| b == b'\n') {
            lines.push(text(&self.partial[start..start + at]));
            start += at + 1;
        }
        self.partial.drain(..start);
        if self.partial.len() > LINE_CAP { lines.push(text(&std::mem::take(&mut self.partial))); }
        Ok((lines, self.offset < len))
    }

    /// Como `read_new`, mas lê até o fim e o pedaço sem `\n` no fim também sai como linha: o processo acabou e nada
    /// mais virá.
    pub(crate) fn read_final(&mut self) -> std::io::Result<Vec<String>> {
        let mut lines = Vec::new();
        loop {
            let (more, left) = self.read_chunk()?;
            lines.extend(more);
            if !left { break; }
        }
        if !self.partial.is_empty() { lines.push(text(&std::mem::take(&mut self.partial))); }
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
    fn final_read_emits_the_last_line_without_newline() {
        let dir = std::env::temp_dir().join(format!("hangar-tail-final-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("saida.log");
        std::fs::write(&path, b"##HANGAR-PASSO## final ok\n##HANGAR-FIM## ok").unwrap();
        let mut tail = Tail::new(path);
        assert_eq!(tail.read_final().unwrap(), vec!["##HANGAR-PASSO## final ok".to_owned(), "##HANGAR-FIM## ok".to_owned()]);
        assert!(tail.read_final().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reads_are_capped_and_long_lines_are_flushed() {
        let dir = std::env::temp_dir().join(format!("hangar-tail-cap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("saida.log");
        // 2 MiB sem quebra e depois "fim": cada leitura pega no máximo `READ_CAP`.
        let mut big = vec![b'x'; 2 * READ_CAP as usize];
        big.extend_from_slice(b"\nfim\n");
        std::fs::write(&path, &big).unwrap();
        let mut tail = Tail::new(path.clone());
        // Passou de `LINE_CAP` sem `\n`: sai como linha e o pedaço volta a zero.
        for _ in 0..2 {
            let lines = tail.read_new().unwrap();
            assert_eq!(lines.iter().map(String::len).collect::<Vec<_>>(), vec![READ_CAP as usize]);
            assert!(tail.partial.is_empty());
        }
        assert_eq!(tail.read_new().unwrap(), vec![String::new(), "fim".to_owned()]);
        // A leitura final vai até o fim do arquivo, em várias voltas.
        let all = Tail::new(path).read_final().unwrap();
        assert_eq!(all.last().map(String::as_str), Some("fim"));
        assert_eq!(all.iter().map(String::len).sum::<usize>(), 2 * READ_CAP as usize + 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_file_is_an_error_not_a_panic() {
        let mut tail = Tail::new(std::env::temp_dir().join("hangar-tail-nao-existe.log"));
        assert!(tail.read_new().is_err());
    }
}
