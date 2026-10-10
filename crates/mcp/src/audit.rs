//! Append-only JSON-lines audit log. One line per tool call, written after the call finishes.

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
