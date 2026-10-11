//! Shared helpers: scratch folders, an in-memory audit sink and a JSON-RPC rig.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use open365_mcp::{Config, Scope, Server};
use serde_json::{Value, json};

pub fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel)
}

pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(test: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("open365-mcpt-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("root")).expect("create scratch");
        Self(dir)
    }
    pub fn root(&self) -> PathBuf {
        self.0.join("root")
    }
    pub fn put(&self, rel: &str) -> PathBuf {
        let name = Path::new(rel).file_name().expect("name");
        let dest = self.root().join(name);
        std::fs::copy(fixture(rel), &dest).expect("copy fixture");
        dest
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone, Default)]
pub struct Buf(Arc<Mutex<Vec<u8>>>);

impl Write for Buf {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("lock").extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Buf {
    pub fn lines(&self) -> Vec<Value> {
        let bytes = self.0.lock().expect("lock").clone();
        String::from_utf8(bytes)
            .expect("utf8")
            .lines()
            .map(|l| serde_json::from_str(l).expect("audit json"))
            .collect()
    }
}

impl Buf {
    /// The `tool_call` result lines only (not intents, session starts or protocol errors).
    pub fn calls(&self) -> Vec<Value> {
        self.lines()
            .into_iter()
            .filter(|l| l["event"] == "tool_call")
            .collect()
    }
}

pub struct Rig {
    pub server: Server,
    pub audit: Buf,
    next: u64,
}

pub fn s(p: &Path) -> &str {
    p.to_str().expect("utf8 path")
}

impl Rig {
    pub fn new(scratch: &Scratch, scopes: &[Scope], destructive: bool) -> Self {
        let audit = Buf::default();
        let server = Server::new(
            Config {
                roots: vec![scratch.root()],
                allow_destructive: destructive,
                scopes: scopes.iter().copied().collect::<BTreeSet<_>>(),
                ..Config::default()
            },
            Box::new(audit.clone()),
        )
        .expect("server");
        Self {
            server,
            audit,
            next: 1,
        }
    }

    pub fn all(scratch: &Scratch) -> Self {
        Self::new(scratch, &[Scope::Edit, Scope::Save, Scope::Export], true)
    }

    pub fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let line = json!({"jsonrpc":"2.0","id":self.next,"method":method,"params":params});
        let reply = self.server.handle_line(&line.to_string()).expect("reply");
        serde_json::from_str(&reply).expect("reply json")
    }

    pub fn call(&mut self, tool: &str, args: Value) -> (Value, bool) {
        let reply = self.rpc("tools/call", json!({"name": tool, "arguments": args}));
        let result = &reply["result"];
        let text = result["content"][0]["text"].as_str().expect("text");
        (
            serde_json::from_str(text).expect("tool json"),
            result["isError"].as_bool().expect("isError"),
        )
    }

    pub fn ok(&mut self, tool: &str, args: Value) -> Value {
        let (v, err) = self.call(tool, args);
        assert!(!err, "{tool}: {v}");
        v
    }

    pub fn open(&mut self, path: &Path) -> String {
        self.ok("open_document", json!({"path": s(path)}))["document"]
            .as_str()
            .expect("id")
            .to_owned()
    }
}
