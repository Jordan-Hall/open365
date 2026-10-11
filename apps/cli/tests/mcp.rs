//! Scripted stdio sessions against the real `open365 mcp` binary, plus the M3 acceptance check:
//! the same commands through `open365 apply` and through MCP give byte-identical output and
//! identical command logs.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use open365_commands::CommandLog;
use serde_json::{Value, json};

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel)
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(test: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("open365-climcp-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("root")).expect("create scratch");
        Self(dir)
    }
    fn root(&self) -> PathBuf {
        self.0.join("root")
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn s(p: &Path) -> &str {
    p.to_str().expect("utf8")
}

/// A running `open365 mcp` process speaking newline-delimited JSON-RPC.
struct Session {
    child: std::process::Child,
    reader: BufReader<std::process::ChildStdout>,
    next: u64,
}

impl Session {
    fn start(root: &Path, audit: &Path, flags: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_open365"))
            .args(["mcp", "--root", s(root), "--audit-log", s(audit)])
            .args(flags)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn open365 mcp");
        let reader = BufReader::new(child.stdout.take().expect("stdout"));
        let mut session = Self {
            child,
            reader,
            next: 0,
        };
        let init = session.rpc(
            "initialize",
            json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}),
        );
        assert_eq!(init["result"]["serverInfo"]["name"], "open365-mcp");
        session.send(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        session
    }

    fn send(&mut self, message: &Value) {
        let stdin = self.child.stdin.as_mut().expect("stdin");
        writeln!(stdin, "{message}").expect("write request");
        stdin.flush().expect("flush");
    }

    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        self.send(&json!({"jsonrpc":"2.0","id":self.next,"method":method,"params":params}));
        let mut line = String::new();
        self.reader.read_line(&mut line).expect("read reply");
        let reply: Value = serde_json::from_str(&line).expect("reply is JSON");
        assert_eq!(reply["id"], self.next);
        reply
    }

    fn call(&mut self, tool: &str, args: Value) -> (Value, bool) {
        let reply = self.rpc("tools/call", json!({"name": tool, "arguments": args}));
        let text = reply["result"]["content"][0]["text"]
            .as_str()
            .expect("text");
        (
            serde_json::from_str(text).expect("tool json"),
            reply["result"]["isError"].as_bool().expect("isError"),
        )
    }

    fn finish(mut self) {
        drop(self.child.stdin.take());
        let status = self.child.wait().expect("wait");
        assert!(status.success());
    }
}

fn audit_lines(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .expect("audit log")
        .lines()
        .map(|l| serde_json::from_str(l).expect("audit json"))
        .collect()
}

#[test]
fn stdio_round_trip_is_read_only_by_default() {
    let scratch = Scratch::new("readonly");
    std::fs::copy(
        fixture("xlsx/formulas-basic.xlsx"),
        scratch.root().join("f.xlsx"),
    )
    .expect("copy");
    let audit = scratch.path("audit.jsonl");
    let mut m = Session::start(&scratch.root(), &audit, &[]);

    let tools = m.rpc("tools/list", json!({}));
    assert_eq!(
        tools["result"]["tools"].as_array().expect("tools").len(),
        17
    );

    let (open, err) = m.call("open_document", json!({"path": "f.xlsx"}));
    assert!(!err, "{open}");
    let id = open["document"].as_str().expect("id").to_owned();
    let (cells, _) = m.call(
        "read_cells",
        json!({"document": id, "sheet": 0, "range": "D1"}),
    );
    assert_eq!(cells["cells"][0]["value"], 15.0);

    let set = json!([{"command":"xlsx_set_cell_value","sheet":0,"row":1,"col":1,"value":{"type":"number","value":9}}]);
    let (refused, err) = m.call("apply_commands", json!({"document": id, "commands": set}));
    assert!(err && refused["status"] == "refused", "{refused}");
    let (outside, err) = m.call(
        "open_document",
        json!({"path": s(&fixture("xlsx/formulas-basic.xlsx"))}),
    );
    assert!(err && outside["status"] == "refused", "{outside}");
    m.finish();

    let lines = audit_lines(&audit);
    let statuses: Vec<&str> = lines
        .iter()
        .filter(|l| l["event"] == "tool_call")
        .map(|l| l["status"].as_str().expect("s"))
        .collect();
    assert_eq!(statuses, ["ok", "ok", "refused", "refused"]);
}

#[test]
fn destructive_flag_requires_edit_flag() {
    let scratch = Scratch::new("flags");
    let status = Command::new(env!("CARGO_BIN_EXE_open365"))
        .args([
            "mcp",
            "--root",
            s(&scratch.root()),
            "--audit-log",
            s(&scratch.path("a.jsonl")),
            "--allow-destructive",
        ])
        .stdin(Stdio::null())
        .output()
        .expect("run");
    assert!(!status.status.success());
}

fn cli_apply(commands: &Path, input: &Path, output: &Path) -> Vec<Value> {
    let o = Command::new(env!("CARGO_BIN_EXE_open365"))
        .args([
            "apply",
            s(commands),
            s(input),
            "-o",
            s(output),
            "--allow-destructive",
        ])
        .output()
        .expect("run apply");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stderr)
        .lines()
        .filter(|l| l.starts_with('{'))
        .map(|l| serde_json::from_str(l).expect("audit json"))
        .collect()
}

fn parity(fixture_rel: &str, name: &str, commands: &Value) {
    let scratch = Scratch::new(&format!("parity-{name}"));
    let input = scratch.root().join(format!("in.{name}"));
    std::fs::copy(fixture(fixture_rel), &input).expect("copy");
    let cmd_file = scratch.path("cmds.json");
    std::fs::write(&cmd_file, commands.to_string()).expect("write commands");

    let cli_out = scratch.path(&format!("cli-out.{name}"));
    let cli_audit = cli_apply(&cmd_file, &input, &cli_out);

    let mcp_out = scratch.root().join(format!("mcp-out.{name}"));
    let audit = scratch.path("audit.jsonl");
    let mut m = Session::start(
        &scratch.root(),
        &audit,
        &["--allow-edit", "--allow-destructive"],
    );
    let (open, err) = m.call("open_document", json!({"path": s(&input)}));
    assert!(!err, "{open}");
    let id = open["document"].as_str().expect("id").to_owned();
    let (applied, err) = m.call(
        "apply_commands",
        json!({"document": id, "commands": commands}),
    );
    assert!(!err, "{applied}");
    let (saved, err) = m.call("save_as", json!({"document": id, "path": s(&mcp_out)}));
    assert!(!err, "{saved}");
    m.finish();

    // Byte-identical output.
    assert_eq!(
        std::fs::read(&cli_out).expect("cli out"),
        std::fs::read(&mcp_out).expect("mcp out"),
        "outputs differ"
    );

    // Identical command logs: the commands MCP recorded are the commands that were sent, and
    // per command the CLI and MCP agree on name, effect and changed cells.
    let parse =
        |v: &Value| -> CommandLog { serde_json::from_value(v.clone()).expect("command log") };
    assert_eq!(parse(&applied["command_log"]), parse(commands));
    let results = applied["results"].as_array().expect("results");
    assert_eq!(results.len() + 1, cli_audit.len(), "{cli_audit:?}");
    for (mcp, cli) in results.iter().zip(&cli_audit) {
        assert_eq!(mcp["command"], cli["command"]);
        assert_eq!(mcp["effect"], cli["effect"]);
        assert_eq!(mcp["changed_cells"], cli["changed_cells"]);
        assert_eq!(mcp["notes"], cli["notes"]);
    }
    let last = cli_audit.last().expect("save line");
    assert_eq!(last["command"], "save");
    assert_eq!(last["effect"], saved["effect"]);
    assert_eq!(last["bytes_written"], saved["bytes_written"]);
}

#[test]
fn mcp_and_cli_apply_agree_on_a_docx() {
    parity(
        "docx/plain-paragraphs.docx",
        "docx",
        &json!([
            {"command":"docx_insert_text","para":0,"offset":5,"text":"XY"},
            {"command":"docx_set_bold","para":1,"start":0,"end":3,"bold":true},
            {"command":"docx_delete_range","start":{"para":0,"offset":0},"end":{"para":0,"offset":3}},
            {"command":"docx_split_paragraph","pos":{"para":1,"offset":2}}
        ]),
    );
}

#[test]
fn mcp_and_cli_apply_agree_on_an_xlsx() {
    parity(
        "xlsx/formulas-basic.xlsx",
        "xlsx",
        &json!([
            {"command":"xlsx_set_cell_value","sheet":0,"row":1,"col":1,"value":{"type":"number","value":41}},
            {"command":"xlsx_set_cell_formula","sheet":0,"row":7,"col":1,"formula":"=A1+1"}
        ]),
    );
}

#[test]
fn the_modern_transcript_replays_against_the_real_binary() {
    let scratch = Scratch::new("transcript");
    let audit = scratch.path("audit.jsonl");
    let input =
        std::fs::File::open(fixture("mcp/transcripts/modern-readonly.jsonl")).expect("open");
    let out = Command::new(env!("CARGO_BIN_EXE_open365"))
        .args([
            "mcp",
            "--root",
            s(&scratch.root()),
            "--audit-log",
            s(&audit),
        ])
        .stdin(Stdio::from(input))
        .output()
        .expect("run");
    assert!(out.status.success());
    let replies: Vec<Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).expect("json"))
        .collect();
    assert_eq!(replies.len(), 4);
    assert_eq!(replies[0]["result"]["resultType"], "complete");
    assert_eq!(replies[0]["result"]["supportedVersions"][0], "2026-07-28");
    assert_eq!(
        replies[1]["result"]["tools"].as_array().map(Vec::len),
        Some(17)
    );
    assert!(replies[2]["result"]["contents"][0]["text"].is_string());
    assert_eq!(replies[3]["result"]["isError"], true);
    // The stateless session start, the resource read and the tool call (with the intent line
    // written before it ran) are on the record.
    let lines = audit_lines(&audit);
    let events: Vec<&str> = lines.iter().filter_map(|l| l["event"].as_str()).collect();
    assert_eq!(
        events,
        ["session_start", "resource_read", "tool_intent", "tool_call"]
    );
}

#[test]
fn grant_opens_exactly_the_named_scope() {
    let scratch = Scratch::new("grant");
    std::fs::copy(
        fixture("docx/plain-paragraphs.docx"),
        scratch.root().join("d.docx"),
    )
    .expect("copy");
    let audit = scratch.path("audit.jsonl");
    let mut m = Session::start(&scratch.root(), &audit, &["--grant", "edit"]);
    let (open, _) = m.call("open_document", json!({"path": "d.docx"}));
    let id = open["document"].as_str().expect("id").to_owned();
    let edit = json!([{"command":"docx_insert_text","para":0,"offset":0,"text":"Z"}]);
    let (ok, err) = m.call("apply_commands", json!({"document": id, "commands": edit}));
    assert!(!err, "{ok}");
    let (no, err) = m.call(
        "save_as",
        json!({"document": id, "path": s(&scratch.root().join("o.docx"))}),
    );
    assert!(err && no["status"] == "refused", "{no}");
    m.finish();
}
