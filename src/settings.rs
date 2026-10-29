//! Settings kept between sessions in a small JSON file next to the binary (`RM2_SETTINGS`
//! overrides the path): the ink mode, pixel-perfect, grid, view, and the tablet OS version the
//! editor last ran on. A flat object of strings, integers and booleans, so it is read and
//! written here without a JSON crate.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Str(String),
    Int(i64),
    Bool(bool),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub map: BTreeMap<String, Value>,
}

impl Settings {
    /// Missing or unreadable file: empty settings (the defaults apply).
    pub fn load(path: &Path) -> Settings {
        std::fs::read_to_string(path).ok().and_then(|t| Settings::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        std::fs::write(path, self.to_json())
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        match self.map.get(key) {
            Some(Value::Str(s)) => Some(s),
            _ => None,
        }
    }

    pub fn int(&self, key: &str) -> Option<i64> {
        match self.map.get(key) {
            Some(Value::Int(i)) => Some(*i),
            _ => None,
        }
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        match self.map.get(key) {
            Some(Value::Bool(b)) => Some(*b),
            _ => None,
        }
    }

    pub fn set_str(&mut self, key: &str, v: &str) {
        self.map.insert(key.to_string(), Value::Str(v.to_string()));
    }

    pub fn set_int(&mut self, key: &str, v: i64) {
        self.map.insert(key.to_string(), Value::Int(v));
    }

    pub fn set_bool(&mut self, key: &str, v: bool) {
        self.map.insert(key.to_string(), Value::Bool(v));
    }

    pub fn to_json(&self) -> String {
        let mut out = String::from("{\n");
        let n = self.map.len();
        for (i, (k, v)) in self.map.iter().enumerate() {
            out.push_str("  ");
            push_string(&mut out, k);
            out.push_str(": ");
            match v {
                Value::Str(s) => push_string(&mut out, s),
                Value::Int(i) => out.push_str(&i.to_string()),
                Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            }
            out.push_str(if i + 1 < n { ",\n" } else { "\n" });
        }
        out.push_str("}\n");
        out
    }

    /// One flat JSON object. Anything else (nested values, trailing garbage) is rejected.
    pub fn parse(text: &str) -> Option<Settings> {
        let mut p = Parser { s: text.as_bytes(), i: 0 };
        p.ws();
        p.expect(b'{')?;
        let mut map = BTreeMap::new();
        p.ws();
        if p.peek() == Some(b'}') {
            p.i += 1;
        } else {
            loop {
                p.ws();
                let key = p.string()?;
                p.ws();
                p.expect(b':')?;
                p.ws();
                let v = p.value()?;
                map.insert(key, v);
                p.ws();
                match p.next()? {
                    b',' => continue,
                    b'}' => break,
                    _ => return None,
                }
            }
        }
        p.ws();
        if p.i != p.s.len() {
            return None;
        }
        Some(Settings { map })
    }
}

fn push_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && (self.s[self.i] as char).is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn next(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.i += 1;
        Some(c)
    }
    fn expect(&mut self, c: u8) -> Option<()> {
        (self.next()? == c).then_some(())
    }
    fn string(&mut self) -> Option<String> {
        self.expect(b'"')?;
        let mut out = Vec::new();
        loop {
            match self.next()? {
                b'"' => break,
                b'\\' => match self.next()? {
                    b'"' => out.push(b'"'),
                    b'\\' => out.push(b'\\'),
                    b'/' => out.push(b'/'),
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'r' => out.push(b'\r'),
                    b'u' => {
                        let hex = self.s.get(self.i..self.i + 4)?;
                        let code = u32::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
                        self.i += 4;
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(char::from_u32(code)?.encode_utf8(&mut buf).as_bytes());
                    }
                    _ => return None,
                },
                c => out.push(c),
            }
        }
        String::from_utf8(out).ok()
    }
    fn value(&mut self) -> Option<Value> {
        match self.peek()? {
            b'"' => Some(Value::Str(self.string()?)),
            b't' if self.s[self.i..].starts_with(b"true") => {
                self.i += 4;
                Some(Value::Bool(true))
            }
            b'f' if self.s[self.i..].starts_with(b"false") => {
                self.i += 5;
                Some(Value::Bool(false))
            }
            b'-' | b'0'..=b'9' => {
                let start = self.i;
                self.i += 1;
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.i += 1;
                }
                std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok().map(Value::Int)
            }
            _ => None,
        }
    }
}

/// `RM2_SETTINGS`, else `settings.json` next to the running binary, else in the current dir.
pub fn default_path() -> PathBuf {
    if let Ok(p) = std::env::var("RM2_SETTINGS") {
        return PathBuf::from(p);
    }
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("settings.json")))
        .unwrap_or_else(|| PathBuf::from("settings.json"))
}

/// The tablet's OS version ("3.29.0.153"), or "pc" anywhere else.
pub fn os_version() -> String {
    for (path, key) in [("/etc/os-release", "IMG_VERSION="), ("/usr/share/remarkable/update.conf", "REMARKABLE_RELEASE_VERSION=")] {
        if let Ok(text) = std::fs::read_to_string(path) {
            for line in text.lines() {
                if let Some(v) = line.strip_prefix(key) {
                    let v = v.trim().trim_matches('"');
                    if !v.is_empty() {
                        return v.to_string();
                    }
                }
            }
        }
    }
    "pc".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults() {
        let mut s = Settings::default();
        s.set_str("stroke_mode", "native");
        s.set_bool("pixel_perfect", true);
        s.set_int("batched_flush_ms", 300);
        s.set_str("name", "quote \" back \\ slash\nnewline");
        let text = s.to_json();
        assert!(text.starts_with("{\n  \"batched_flush_ms\": 300,\n"));
        assert_eq!(Settings::parse(&text), Some(s.clone()));
        assert_eq!(s.str("stroke_mode"), Some("native"));
        assert_eq!(s.bool("pixel_perfect"), Some(true));
        assert_eq!(s.int("batched_flush_ms"), Some(300));
        assert_eq!(s.int("stroke_mode"), None, "wrong type reads as absent");
        assert_eq!(Settings::parse("{}"), Some(Settings::default()));
        assert_eq!(Settings::parse("{ \"a\": [1] }"), None, "nested values are rejected");
        assert_eq!(Settings::parse("{\"a\": 1} x"), None, "trailing garbage is rejected");
        assert_eq!(Settings::parse("{\"u\": \"\\u0041\"}").unwrap().str("u"), Some("A"));
    }

    #[test]
    fn load_and_save_files() {
        let dir = std::env::temp_dir().join(format!("rm2-settings-{}", std::process::id()));
        let path = dir.join("sub").join("settings.json");
        assert_eq!(Settings::load(&path), Settings::default(), "missing file: defaults");
        let mut s = Settings::default();
        s.set_bool("grid", false);
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path).bool("grid"), Some(false));
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(Settings::load(&path), Settings::default(), "broken file: defaults");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
