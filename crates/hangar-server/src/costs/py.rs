//! Conversões compartilhadas pelos leitores de custos, com a semântica do Python.

use chrono::{DateTime, FixedOffset, Timelike};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::OnceLock;

pub const LOCAL_OFFSET_S: i32 = -3 * 3600;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LocalTs(pub i64);

impl LocalTs {
    fn local(&self) -> DateTime<FixedOffset> {
        DateTime::from_timestamp_micros(self.0)
            .expect("timestamp dentro do intervalo de datetime")
            .with_timezone(&FixedOffset::east_opt(LOCAL_OFFSET_S).unwrap())
    }

    pub fn iso(&self) -> String {
        let dt = self.local();
        let mut text = dt.format("%Y-%m-%dT%H:%M:%S").to_string();
        let micros = dt.nanosecond() / 1000;
        if micros != 0 {
            text.push_str(&format!(".{micros:06}"));
        }
        text.push_str("-03:00");
        text
    }

    pub fn day(&self) -> String {
        self.local().format("%Y-%m-%d").to_string()
    }

    pub fn from_iso(s: &str) -> Option<Self> {
        let (micros, offset) = crate::transcript::py::fromisoformat(&s.replace('Z', "+00:00"))?;
        // A tupla conserva microssegundos e distingue hora local sem inspecionar a string.
        Some(Self(micros - offset.unwrap_or(i64::from(LOCAL_OFFSET_S) * 1_000_000)))
    }

    pub fn from_millis_f64(ms: f64) -> Self {
        let seconds = ms / 1000.0;
        let whole = seconds.trunc();
        // A fração assinada evita perder precisão ao somar um segundo antes de arredondar.
        let micros = ((seconds - whole) * 1_000_000.0).round_ties_even() as i64;
        Self((whole as i64).saturating_mul(1_000_000).saturating_add(micros))
    }
}

pub fn py_int(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Bool(b)) => i64::from(*b),
        Some(Value::Number(n)) => n.as_i64().unwrap_or_else(|| n.as_f64().unwrap_or(0.0) as i64),
        Some(Value::String(text)) => parse_integer(crate::transcript::py::strip(text)),
        _ => 0,
    }
}

fn parse_integer(text: &str) -> i64 {
    let Some(text) = normalized_number(text) else { return 0 };
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text.as_str()),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return 0;
    }
    digits.bytes().fold(0i64, |n, b| {
        let digit = i64::from(b - b'0');
        if negative {
            n.saturating_mul(10).saturating_sub(digit)
        } else {
            n.saturating_mul(10).saturating_add(digit)
        }
    })
}

pub(super) fn normalized_number(text: &str) -> Option<String> {
    let chars: Vec<char> = crate::transcript::py::strip(text).chars().collect();
    let mut result = String::with_capacity(text.len());
    for (index, c) in chars.iter().copied().enumerate() {
        if c == '_' {
            if index == 0 || decimal_digit(chars[index - 1]).is_none()
                || chars.get(index + 1).and_then(|c| decimal_digit(*c)).is_none() {
                return None;
            }
        } else if let Some(digit) = decimal_digit(c) {
            result.push(char::from(b'0' + digit));
        } else {
            result.push(c);
        }
    }
    Some(result)
}

fn decimal_digit(c: char) -> Option<u8> {
    if c.is_ascii() {
        return c.to_digit(10).map(|digit| digit as u8);
    }
    static DECIMAL: OnceLock<Regex> = OnceLock::new();
    let regex = DECIMAL.get_or_init(|| Regex::new(r"^\p{Nd}$").unwrap());
    let mut buffer = [0; 4];
    if !regex.is_match(c.encode_utf8(&mut buffer)) {
        return None;
    }
    let mut first = c as u32;
    // Blocos decimais Unicode têm dez dígitos; estilos adjacentes repetem a sequência.
    while let Some(previous) = char::from_u32(first - 1) {
        if !regex.is_match(previous.encode_utf8(&mut buffer)) {
            break;
        }
        first -= 1;
    }
    Some(((c as u32 - first) % 10) as u8)
}

pub fn char_len(s: &str) -> usize {
    s.chars().count()
}

pub fn parse_obj(raw: &[u8]) -> Option<Map<String, Value>> {
    // bytes.strip() não remove separadores Unicode nem os controles \x1c..\x1f.
    let is_space = |b: &u8| b.is_ascii_whitespace() || *b == b'\x0b';
    let start = raw.iter().position(|b| !is_space(b)).unwrap_or(raw.len());
    let end = raw.iter().rposition(|b| !is_space(b)).map_or(start, |i| i + 1);
    let raw = &raw[start..end];
    let text = String::from_utf8_lossy(raw);
    let value = serde_json::from_str::<Value>(&text).ok().or_else(|| {
        // O fallback de transcript tira espaços Unicode; bytes.strip() não os aceita aqui.
        (crate::transcript::py::strip(&text) == text).then(|| crate::transcript::decode_line(raw)).flatten()
    })?;
    match value {
        Value::Object(obj) => Some(obj),
        _ => None,
    }
}
