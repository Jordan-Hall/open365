//! Append-only JSON-lines audit log. A tool call that can have a side effect is on the record
//! twice: an intent line before it starts and a result line after it finishes. Nothing the agent
//! sends is logged as it came: names and codes are fixed words, and every agent-supplied value
//! is cut to a fixed length and, when long, reduced to its length and a fingerprint.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

pub struct Audit {
    sink: Box<dyn Write + Send>,
    seq: u64,
}

impl std::fmt::Debug for Audit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Audit").field("seq", &self.seq).finish()
    }
}

impl Audit {
    pub fn new(sink: Box<dyn Write + Send>) -> Self {
        Self { sink, seq: 0 }
    }

    /// The sequence number of the last line written (0 before the first).
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Writes one line. `fields` are merged with `seq`, `ts_ms`, `origin` and `pid`.
    /// An error means the line is not durable; the caller must stop serving tool calls.
    pub fn record(&mut self, fields: Map<String, Value>) -> std::io::Result<()> {
        self.seq += 1;
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let mut line = json!({
            "seq": self.seq,
            "ts_ms": ts_ms.to_string(),
            "origin": "mcp",
            "pid": std::process::id(),
        });
        if let Some(map) = line.as_object_mut() {
            map.extend(fields);
        }
        writeln!(self.sink, "{line}")?;
        self.sink.flush()
    }
}

/// Longest agent-supplied value logged as it is, in characters.
const MAX_LOGGED_CHARS: usize = 256;
/// How much of a longer value is kept.
const KEPT_CHARS: usize = 64;

/// A short fingerprint of `text` (64-bit SipHash with fixed keys, not a cryptographic hash):
/// enough to tell whether two long values are the same without logging either.
pub(crate) fn fingerprint(text: &str) -> String {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Records an agent-supplied value under `key`. A value of up to 256 characters is kept with
/// control, format and separator characters replaced; a longer one is cut to 64 characters and
/// gets `<key>_len` and `<key>_fp` (a fingerprint) beside it.
pub(crate) fn put_text(map: &mut Map<String, Value>, key: &str, raw: &str) {
    let len = raw.chars().count();
    if len <= MAX_LOGGED_CHARS {
        map.insert(key.to_owned(), json!(crate::scope::sanitize_with(raw, len)));
        return;
    }
    let kept = crate::scope::sanitize_with(raw, KEPT_CHARS);
    map.insert(key.to_owned(), json!(kept));
    map.insert(format!("{key}_len"), json!(len));
    map.insert(format!("{key}_fp"), json!(fingerprint(raw)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_values_stay_and_long_ones_are_cut_and_fingerprinted() {
        let mut m = Map::new();
        put_text(&mut m, "p", "a.docx");
        assert_eq!(m["p"], "a.docx");
        let long = "x".repeat(10_000);
        put_text(&mut m, "q", &long);
        assert_eq!(
            m["q"].as_str().map(|s| s.chars().count()),
            Some(KEPT_CHARS + 3)
        );
        assert_eq!(m["q_len"], 10_000);
        assert_eq!(m["q_fp"], fingerprint(&long));
        assert_ne!(fingerprint("a"), fingerprint("b"));
    }
}
