use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchCursor {
    pub conversation: String,
    pub file_identity: Option<String>,
    pub offset: u64,
    pub anchor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub absent_since: Option<f64>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Occurrence {
    pub id: String,
    pub conversation: String,
    pub file_identity: String,
    pub offset: u64,
    pub end_offset: u64,
    pub text: String,
    pub kind: String,
    pub timestamp: Option<f64>,
    #[serde(default)]
    pub recorded_conversation: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptProof {
    pub cursor: DispatchCursor,
    pub occurrence: Occurrence,
    pub normalized_text: String,
    pub observed_anchor: String,
}

pub struct ReceiptIndex {
    provider: String,
    conversation: String,
    identity: Option<String>,
    data: Vec<u8>,
    occurrences: Vec<Occurrence>,
    scan_offset: usize,
}

fn anchor(data: &[u8], offset: u64) -> Option<String> {
    let offset = usize::try_from(offset).ok()?;
    if offset > data.len() { return None; }
    Some(sha1_smol::Sha1::from(&data[offset.saturating_sub(256)..offset]).digest().to_string())
}

fn identity(file: &File) -> io::Result<String> {
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        let stat = file.metadata()?;
        Ok(format!("{:x}:{:x}",stat.dev(),stat.ino()))
    }
    #[cfg(windows)] {
        use std::os::windows::io::AsRawHandle;
        #[repr(C)]
        struct FileIdInfo { volume:u64, file_id:[u8;16] }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetFileInformationByHandleEx(handle:*mut std::ffi::c_void, class:i32, buffer:*mut std::ffi::c_void, size:u32) -> i32;
        }
        let mut info = FileIdInfo { volume:0,file_id:[0;16] };
        // A estrutura e o handle permanecem válidos durante a chamada síncrona.
        let result = unsafe { GetFileInformationByHandleEx(file.as_raw_handle(),18,
            (&mut info as *mut FileIdInfo).cast(),std::mem::size_of::<FileIdInfo>() as u32) };
        if result == 0 { return Err(io::Error::last_os_error()); }
        Ok(format!("{:x}:{:x}",info.volume,u128::from_le_bytes(info.file_id)))
    }
    #[cfg(not(any(unix,windows)))] {
        let _ = file;
        Err(io::Error::new(io::ErrorKind::Unsupported,"identidade de arquivo indisponível"))
    }
}

impl ReceiptIndex {
    pub fn new(provider: &str, conversation: &str) -> Self {
        Self { provider:provider.into(),conversation:conversation.into(),identity:None,data:Vec::new(),occurrences:Vec::new(),scan_offset:0 }
    }

    pub fn capture(&self, path: &Path) -> io::Result<DispatchCursor> {
        let (identity, data, offset) = match File::open(path) {
            Ok(mut file) => {
                let id = identity(&file)?;
                let offset = file.seek(SeekFrom::End(0))?;
                let mut data = vec![0;offset.min(256) as usize];
                file.seek(SeekFrom::Start(offset.saturating_sub(256)))?;
                file.read_exact(&mut data)?;
                (Some(id),data,offset)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => (None,Vec::new(),0),
            Err(error) => return Err(error),
        };
        let absent_since = if identity.is_none() { Some(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map_err(io::Error::other)?.as_secs_f64()) } else { None };
        Ok(DispatchCursor { conversation:self.conversation.clone(),file_identity:identity,absent_since,
            offset,anchor:anchor(&data,data.len() as u64).unwrap() })
    }

    pub fn scan(&mut self, path: &Path) -> io::Result<Vec<Occurrence>> {
        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.data.clear(); self.identity = None; self.occurrences.clear(); self.scan_offset = 0; return Ok(Vec::new());
            }
            Err(error) => return Err(error),
        };
        let id = identity(&file)?;
        let mut unchanged = self.identity.as_deref() == Some(id.as_str()) && file.metadata()?.len() >= self.data.len() as u64;
        if unchanged {
            let mut tail = vec![0;self.data.len().min(256)];
            file.seek(SeekFrom::Start(self.data.len().saturating_sub(256) as u64))?;
            file.read_exact(&mut tail)?;
            unchanged = tail == self.data[self.data.len().saturating_sub(256)..];
        }
        let mut data = if unchanged { self.data.clone() } else { Vec::new() };
        let mut occurrences = if unchanged { self.occurrences.clone() } else { Vec::new() };
        let start_offset = if unchanged { self.scan_offset } else { 0 };
        file.seek(SeekFrom::Start(data.len() as u64))?;
        file.read_to_end(&mut data)?;
        let current = File::open(path)?;
        if identity(&current)? != id || current.metadata()?.len() < data.len() as u64 {
            return Err(io::Error::new(io::ErrorKind::InvalidData,"transcript mudou durante a leitura"));
        }
        let mut parser = crate::transcript::LineParser::new(crate::transcript::Provider::Codex);
        let mut offset = start_offset as u64;
        let mut complete_offset = start_offset;
        for raw in data[start_offset..].split_inclusive(|byte| *byte == b'\n') {
            let start = offset; offset += raw.len() as u64;
            if raw.last() != Some(&b'\n') { break; }
            complete_offset = offset as usize;
            let Some(obj) = crate::transcript::decode_line(raw) else { continue };
            if !obj.is_object() { continue; }
            let (text,kind) = if self.provider == "codex" {
                (parser.feed(raw,start).into_iter().filter(|e|e.kind == hangar_api::chat::ChatKind::UserMsg)
                    .filter_map(|e|e.text).collect::<Vec<_>>().join("\n"),"user")
            } else {
                match obj["type"].as_str() {
                    Some("user") => (content_text(&obj["message"]["content"]),"user"),
                    Some("queue-operation") if obj["operation"] == "dequeue" => (obj["content"].as_str().unwrap_or("").to_owned(),"dequeue"),
                    Some("attachment") if obj["attachment"]["type"] == "queued_command" => (content_text(&obj["attachment"]["prompt"]),"steer"),
                    _ => continue,
                }
            };
            if text.trim().is_empty() { continue; }
            let provider_id = obj["uuid"].as_str().or_else(||obj["id"].as_str()).or_else(||obj["payload"]["id"].as_str());
            let record = provider_id.filter(|s|!s.is_empty()).map_or_else(||format!("offset:{start}"),|id|format!("id:{id}"));
            let timestamp = obj["timestamp"].as_str().and_then(crate::transcript::ts_of_iso);
            occurrences.push(Occurrence { id:format!("{}|{id}|{record}",self.conversation),conversation:self.conversation.clone(),
                file_identity:id.clone(),offset:start,end_offset:offset,text,kind:kind.into(),timestamp,
                recorded_conversation:obj["sessionId"].as_str().map(str::to_owned) });
        }
        self.data = data; self.identity = Some(id); self.occurrences = occurrences;
        self.scan_offset = complete_offset;
        Ok(self.occurrences.clone())
    }

    pub fn match_after(&self, cursor: &DispatchCursor, row: &Value, used: &BTreeMap<String,Value>) -> Option<ReceiptProof> {
        if cursor.conversation != self.conversation || self.identity.is_none()
            || (cursor.file_identity.is_some() && cursor.file_identity != self.identity) { return None; }
        let observed = anchor(&self.data,cursor.offset)?;
        if observed != cursor.anchor { return None; }
        for occurrence in &self.occurrences {
            if used.contains_key(&occurrence.id) || occurrence.offset < cursor.offset { continue; }
            if !cursor_accepts(cursor,occurrence) { continue; }
            let candidates = super::queue::entry_lines(row);
            let committed = crate::transcript::history::chaves_de_commit(&occurrence.text);
            if let Some(normalized_text) = candidates.into_iter().find(|c|committed.contains(c)) {
                return Some(ReceiptProof { cursor:cursor.clone(),occurrence:occurrence.clone(),normalized_text,observed_anchor:observed });
            }
        }
        None
    }
}

fn content_text(content: &Value) -> String {
    if let Some(text) = content.as_str() { return text.into(); }
    content.as_array().map(|blocks| blocks.iter().filter(|b|b["type"] == "text")
        .filter_map(|b|b["text"].as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default()
}

impl ReceiptProof {
    pub fn validates(&self, cursor: &DispatchCursor, row: &Value) -> bool {
        self.cursor == *cursor && cursor_accepts(cursor,&self.occurrence)
            && self.occurrence.conversation == cursor.conversation && self.occurrence.offset >= cursor.offset
            && self.occurrence.end_offset > self.occurrence.offset && self.observed_anchor == cursor.anchor
            && matches!(self.occurrence.kind.as_str(),"user" | "dequeue" | "steer")
            && super::queue::entry_lines(row).contains(&self.normalized_text)
            && crate::transcript::history::chaves_de_commit(&self.occurrence.text).contains(&self.normalized_text)
    }
}

fn cursor_accepts(cursor:&DispatchCursor,occurrence:&Occurrence)->bool {
    match &cursor.file_identity {
        Some(identity)=>identity==&occurrence.file_identity,
        // Arquivo novo só comprova a conversa explícita e uma ocorrência posterior ao despacho.
        None=>cursor.offset==0 && cursor.anchor==anchor(&[],0).unwrap()
            && occurrence.recorded_conversation.as_deref()==Some(cursor.conversation.as_str())
            && cursor.absent_since.is_some_and(|since|occurrence.timestamp.is_some_and(|ts|ts>=since)),
    }
}
