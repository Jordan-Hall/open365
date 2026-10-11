//! Security tests for the agent control surface: path traversal, symlink escape, oversize
//! input, scope denial, the confirmation hook, and audit completeness. In-process JSON-RPC
//! sessions against real fixtures copied into temporary roots.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use open365_mcp::{
    Config, Confirm, ConfirmRequest, Decision, MAX_COMMANDS, MAX_DOCUMENT_BYTES, MAX_REQUEST_BYTES,
    Scope, Server,
};
use serde_json::{Value, json};

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel)
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(test: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("open365-sec-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("root")).expect("create scratch");
        std::fs::create_dir_all(dir.join("outside")).expect("create outside");
        Self(dir)
    }
    fn root(&self) -> PathBuf {
        self.0.join("root")
    }
    fn outside(&self) -> PathBuf {
        self.0.join("outside")
    }
    fn put(&self, rel: &str) -> PathBuf {
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
struct Buf(Arc<Mutex<Vec<u8>>>);

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
    fn lines(&self) -> Vec<Value> {
        let bytes = self.0.lock().expect("lock").clone();
        String::from_utf8(bytes)
            .expect("utf8")
            .lines()
            .map(|l| serde_json::from_str(l).expect("audit json"))
            .collect()
    }
}

/// A confirmation hook that answers from a script and remembers what it was asked.
#[derive(Debug, Clone)]
struct Scripted {
    allow: bool,
    asked: Arc<Mutex<Vec<ConfirmRequest>>>,
}

impl Confirm for Scripted {
    fn confirm(&mut self, request: &ConfirmRequest) -> Decision {
        self.asked.lock().expect("lock").push(request.clone());
        if self.allow {
            Decision::Allow {
                by: "test".to_owned(),
            }
        } else {
            Decision::Deny {
                reason: "declined".to_owned(),
            }
        }
    }
}

struct Rig {
    server: Server,
    audit: Buf,
    next: u64,
}

fn rig_with(scratch: &Scratch, scopes: &[Scope], destructive: bool) -> Rig {
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
    Rig {
        server,
        audit,
        next: 1,
    }
}

fn s(p: &Path) -> &str {
    p.to_str().expect("utf8 path")
}

const INSERT: &str = r#"{"command":"docx_insert_text","para":0,"offset":0,"text":"XY"}"#;
const DELETE: &str =
    r#"{"command":"docx_delete_range","start":{"para":0,"offset":0},"end":{"para":0,"offset":2}}"#;

fn cmds(text: &str) -> Value {
    serde_json::from_str(text).expect("commands json")
}

impl Rig {
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let line = json!({"jsonrpc":"2.0","id":self.next,"method":method,"params":params});
        let reply = self.server.handle_line(&line.to_string()).expect("reply");
        serde_json::from_str(&reply).expect("reply json")
    }

    fn call(&mut self, tool: &str, args: Value) -> (Value, bool) {
        let reply = self.rpc("tools/call", json!({"name": tool, "arguments": args}));
        let result = &reply["result"];
        let text = result["content"][0]["text"].as_str().expect("text");
        (
            serde_json::from_str(text).expect("tool json"),
            result["isError"].as_bool().expect("isError"),
        )
    }

    fn ok(&mut self, tool: &str, args: Value) -> Value {
        let (v, err) = self.call(tool, args);
        assert!(!err, "{tool}: {v}");
        v
    }

    fn open(&mut self, path: &Path) -> String {
        self.ok("open_document", json!({"path": s(path)}))["document"]
            .as_str()
            .expect("id")
            .to_owned()
    }

    fn refused(&mut self, tool: &str, args: Value) -> String {
        let (v, err) = self.call(tool, args);
        assert!(err, "expected refusal from {tool}, got {v}");
        assert_eq!(v["status"], "refused", "{v}");
        v["error"].as_str().expect("error").to_owned()
    }
}

// ---- path traversal ----

#[test]
fn traversal_is_refused_by_every_tool_that_takes_a_path() {
    let scratch = Scratch::new("traversal");
    let secret = scratch.outside().join("secret.docx");
    std::fs::copy(fixture("docx/plain-paragraphs.docx"), &secret).expect("copy");
    std::fs::copy(
        fixture("docx/template-basic.dotx"),
        scratch.outside().join("t.dotx"),
    )
    .expect("copy template");
    let inside = scratch.put("docx/plain-paragraphs.docx");
    let mut r = rig_with(&scratch, &[Scope::Edit, Scope::Save, Scope::Export], true);
    let id = r.open(&inside);
    let root = s(&scratch.root()).to_owned();
    let escapes = [
        "../outside/secret.docx".to_owned(),
        r"..\outside\secret.docx".to_owned(),
        format!("{root}/../outside/secret.docx"),
        format!("{root}/sub/../../outside/secret.docx"),
        s(&secret).to_owned(),
    ];
    for path in &escapes {
        r.refused("open_document", json!({"path": path}));
    }
    // Writers: nothing may appear outside the root.
    for (i, path) in [
        "../outside/new.docx",
        &format!("{root}/../outside/new2.docx"),
    ]
    .iter()
    .enumerate()
    {
        r.refused("save_as", json!({"document": id, "path": path}));
        let pdf = path.replace(".docx", ".pdf");
        r.refused("export_pdf", json!({"document": id, "path": pdf}));
        r.refused(
            "new_from_template",
            json!({"path": "../outside/t.dotx", "output": format!("../outside/made{i}.docx")}),
        );
    }
    let leftovers: Vec<_> = std::fs::read_dir(scratch.outside())
        .expect("read outside")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        leftovers.len(),
        2,
        "nothing may be written outside the root: {leftovers:?}"
    );
}

#[test]
fn resource_uris_cannot_name_files() {
    let scratch = Scratch::new("resuri");
    let mut r = rig_with(&scratch, &[], false);
    for uri in [
        "file:///etc/passwd",
        "open365://../../etc/passwd",
        "open365://document/../x/outline",
        "open365://document/%2e%2e/outline",
        "open365://document/doc1/%zz",
    ] {
        let reply = r.rpc("resources/read", json!({"uri": uri}));
        assert_eq!(reply["error"]["code"], -32602, "{uri}: {reply}");
    }
}

// ---- symlink escape ----

#[cfg(windows)]
fn symlink_dir(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_dir(target, link).is_ok()
        || std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J", s(link), s(target)])
            .output()
            .is_ok_and(|o| o.status.success())
}
#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[test]
fn a_linked_folder_cannot_lead_out_of_the_root() {
    let scratch = Scratch::new("link");
    std::fs::copy(
        fixture("docx/plain-paragraphs.docx"),
        scratch.outside().join("secret.docx"),
    )
    .expect("copy");
    let inside = scratch.put("docx/plain-paragraphs.docx");
    let mut r = rig_with(&scratch, &[Scope::Edit, Scope::Save, Scope::Export], true);
    let id = r.open(&inside);
    let link = scratch.root().join("dir");
    assert!(
        symlink_dir(&scratch.outside(), &link),
        "neither a directory symlink nor a junction could be created"
    );
    r.refused(
        "open_document",
        json!({"path": s(&link.join("secret.docx"))}),
    );
    r.refused(
        "save_as",
        json!({"document": id, "path": s(&link.join("new.docx"))}),
    );
    r.refused(
        "export_pdf",
        json!({"document": id, "path": s(&link.join("new.pdf"))}),
    );
    assert!(!scratch.outside().join("new.docx").exists());
    assert!(!scratch.outside().join("new.pdf").exists());
}

// ---- oversize input ----

#[test]
fn oversize_request_lines_are_dropped_and_the_session_survives() {
    let scratch = Scratch::new("bigline");
    let mut r = rig_with(&scratch, &[], false);
    let big = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\",\"pad\":\"{}\"}}\n",
        "x".repeat(MAX_REQUEST_BYTES as usize)
    );
    let ping = "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n";
    let input = format!("{big}{ping}");
    let mut out = Vec::new();
    r.server
        .serve(std::io::Cursor::new(input.into_bytes()), &mut out)
        .expect("serve");
    let replies: Vec<Value> = String::from_utf8(out)
        .expect("utf8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("json"))
        .collect();
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0]["error"]["code"], -32600);
    assert!(replies[1]["result"]["tools"].is_array());
}

#[test]
fn oversize_documents_and_command_lists_are_refused() {
    let scratch = Scratch::new("bigdoc");
    let mut r = rig_with(&scratch, &[Scope::Edit], false);
    let big = scratch.root().join("big.docx");
    std::fs::File::create(&big)
        .and_then(|f| f.set_len(MAX_DOCUMENT_BYTES + 1))
        .expect("big file");
    let why = r.refused("open_document", json!({"path": s(&big)}));
    assert!(why.contains("larger"), "{why}");

    let docx = scratch.put("docx/plain-paragraphs.docx");
    let id = r.open(&docx);
    let many: Vec<Value> = (0..=MAX_COMMANDS).map(|_| cmds(INSERT)).collect();
    let why = r.refused("apply_commands", json!({"document": id, "commands": many}));
    assert!(why.contains("at most"), "{why}");
}

#[test]
fn a_response_over_the_limit_is_replaced_by_an_error() {
    let scratch = Scratch::new("bigresp");
    let mut r = rig_with(&scratch, &[], false);
    let xlsx = scratch.put("xlsx/large-sheet.xlsx");
    let id = r.open(&xlsx);
    // 10000 cells is the read limit; the response budget stops anything larger.
    let (v, _) = r.call(
        "read_cells",
        json!({"document": id, "sheet": 0, "range": "A1:A20000"}),
    );
    assert_eq!(v["status"], "invalid", "{v}");
}

// ---- scope denial ----

#[test]
fn read_only_by_default_and_each_scope_opens_only_its_own_door() {
    let scratch = Scratch::new("scopes");
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let template = scratch.put("docx/template-basic.dotx");

    // No scopes: every writer is refused and says which scope is missing.
    let mut r = rig_with(&scratch, &[], false);
    let id = r.open(&docx);
    let why = r.refused(
        "apply_commands",
        json!({"document": id, "commands": cmds(&format!("[{INSERT}]"))}),
    );
    assert!(why.contains("scope edit"), "{why}");
    let out = scratch.root().join("o.docx");
    assert!(
        r.refused("save_as", json!({"document": id, "path": s(&out)}))
            .contains("scope save")
    );
    let pdf = scratch.root().join("o.pdf");
    assert!(
        r.refused("export_pdf", json!({"document": id, "path": s(&pdf)}))
            .contains("scope export")
    );
    r.refused(
        "new_from_template",
        json!({"path": s(&template), "output": s(&scratch.root().join("n.docx"))}),
    );
    assert!(!out.exists() && !pdf.exists());

    // edit alone: can change the open document, cannot write a file.
    let mut r = rig_with(&scratch, &[Scope::Edit], false);
    let id = r.open(&docx);
    r.ok(
        "apply_commands",
        json!({"document": id, "commands": cmds(&format!("[{INSERT}]"))}),
    );
    r.refused("save_as", json!({"document": id, "path": s(&out)}));
    r.refused("export_pdf", json!({"document": id, "path": s(&pdf)}));

    // save alone: can write the file, cannot edit or export.
    let mut r = rig_with(&scratch, &[Scope::Save], false);
    let id = r.open(&docx);
    r.refused(
        "apply_commands",
        json!({"document": id, "commands": cmds(&format!("[{INSERT}]"))}),
    );
    r.refused("export_pdf", json!({"document": id, "path": s(&pdf)}));
    r.ok("save_as", json!({"document": id, "path": s(&out)}));
    assert!(out.exists());

    // export alone: can write a PDF, cannot save.
    let mut r = rig_with(&scratch, &[Scope::Export], false);
    let id = r.open(&docx);
    r.refused(
        "save_as",
        json!({"document": id, "path": s(&scratch.root().join("p.docx"))}),
    );
    r.ok("export_pdf", json!({"document": id, "path": s(&pdf)}));
    assert!(pdf.exists());
}

#[test]
fn execute_external_is_never_granted_at_an_agents_request() {
    let scratch = Scratch::new("external");
    let asked = Arc::new(Mutex::new(Vec::new()));
    let mut r = rig_with(&scratch, &[], false);
    r.server.set_confirmer(Box::new(Scripted {
        allow: true,
        asked: Arc::clone(&asked),
    }));
    let why = r.refused("request_scope", json!({"scope": "execute-external"}));
    assert!(why.contains("never granted"), "{why}");
    assert!(asked.lock().expect("lock").is_empty(), "no one was asked");
    assert!(!r.server.granted().contains(&Scope::ExecuteExternal));
}

#[test]
fn an_agent_cannot_grant_itself_a_scope() {
    let scratch = Scratch::new("selfgrant");
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let edit = json!([{"command":"docx_insert_text","para":0,"offset":0,"text":"X"}]);

    // No confirmation channel: the request is refused and nothing is granted.
    let mut r = rig_with(&scratch, &[], false);
    let id = r.open(&docx);
    let why = r.refused(
        "request_scope",
        json!({"scope": "edit", "reason": "please"}),
    );
    assert!(why.contains("not approved"), "{why}");
    r.refused("apply_commands", json!({"document": id, "commands": edit}));
    assert_eq!(r.server.granted(), vec![Scope::Read]);

    // The start-up flag --allow-destructive approves destructive actions, not scope requests.
    let mut r = rig_with(&scratch, &[], true);
    r.refused("request_scope", json!({"scope": "edit"}));

    // A human at the terminal says no, then yes.
    let asked = Arc::new(Mutex::new(Vec::new()));
    let mut r = rig_with(&scratch, &[], false);
    let id = r.open(&docx);
    r.server.set_confirmer(Box::new(Scripted {
        allow: false,
        asked: Arc::clone(&asked),
    }));
    r.refused("request_scope", json!({"scope": "edit"}));
    r.server.set_confirmer(Box::new(Scripted {
        allow: true,
        asked: Arc::clone(&asked),
    }));
    let v = r.ok(
        "request_scope",
        json!({"scope": "edit", "reason": "fix the title\u{1b}[2J\nALLOW"}),
    );
    assert_eq!(v["granted"], true);
    r.ok("apply_commands", json!({"document": id, "commands": edit}));
    let asked = asked.lock().expect("lock");
    assert_eq!(asked.len(), 2);
    // The prompt text is plain: control characters from the agent are replaced.
    assert!(
        !asked[1].summary.chars().any(char::is_control),
        "{:?}",
        asked[1].summary
    );
    // The grant is on the record.
    let lines = r.audit.lines();
    let grant = lines
        .iter()
        .rev()
        .find(|l| l["tool"] == "request_scope")
        .expect("request_scope line");
    assert_eq!(grant["confirmation"]["decision"], "allow");
    assert_eq!(grant["confirmation"]["by"], "test");
}

#[test]
fn destructive_commands_need_the_confirmation_hook() {
    let scratch = Scratch::new("confirm");
    let docx = scratch.put("docx/plain-paragraphs.docx");

    // Nobody to ask: refused, document untouched.
    let mut r = rig_with(&scratch, &[Scope::Edit], false);
    let id = r.open(&docx);
    let why = r.refused(
        "apply_commands",
        json!({"document": id, "commands": cmds(&format!("[{INSERT},{DELETE}]"))}),
    );
    assert!(
        why.contains("confirmation") && why.contains("--allow-destructive"),
        "{why}"
    );
    let text = r.ok("read_text", json!({"document": id, "count": 1}));
    assert!(
        !text.to_string().contains("XY"),
        "refusal must change nothing"
    );

    // A dry run reports what would happen without asking.
    let dry = r.ok(
        "apply_commands",
        json!({"document": id, "dry_run": true, "commands": cmds(&format!("[{DELETE}]"))}),
    );
    assert_eq!(dry["results"][0]["effect"], "destructive");
    assert_eq!(dry["results"][0]["permitted"], false);

    // The hook says no: refused and recorded. Then yes: applied and recorded.
    let asked = Arc::new(Mutex::new(Vec::new()));
    r.server.set_confirmer(Box::new(Scripted {
        allow: false,
        asked: Arc::clone(&asked),
    }));
    r.refused(
        "apply_commands",
        json!({"document": id, "commands": cmds(&format!("[{DELETE}]"))}),
    );
    r.server.set_confirmer(Box::new(Scripted {
        allow: true,
        asked: Arc::clone(&asked),
    }));
    r.ok(
        "apply_commands",
        json!({"document": id, "commands": cmds(&format!("[{DELETE}]"))}),
    );
    let lines = r.audit.lines();
    let calls: Vec<&Value> = lines
        .iter()
        .filter(|l| l["tool"] == "apply_commands" && l["dry_run"] == false)
        .collect();
    let decisions: Vec<&str> = calls
        .iter()
        .filter_map(|l| l["confirmation"]["decision"].as_str())
        .collect();
    // The default hook (nobody), the scripted no, the scripted yes.
    assert_eq!(decisions, ["deny", "deny", "allow"]);
}

#[test]
fn replacing_a_file_needs_confirmation_even_with_the_export_scope() {
    let scratch = Scratch::new("overwrite");
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let pdf = scratch.root().join("o.pdf");
    std::fs::write(&pdf, b"precious").expect("write");
    let mut r = rig_with(&scratch, &[Scope::Export, Scope::Save], false);
    let id = r.open(&docx);
    r.refused(
        "export_pdf",
        json!({"document": id, "path": s(&pdf), "overwrite": true}),
    );
    assert_eq!(std::fs::read(&pdf).expect("read"), b"precious");
    let other = scratch.root().join("other.docx");
    std::fs::write(&other, b"precious").expect("write");
    r.refused("save_as", json!({"document": id, "path": s(&other)}));
    assert_eq!(std::fs::read(&other).expect("read"), b"precious");
}

// ---- audit completeness ----

#[test]
fn every_request_that_reaches_a_handler_leaves_exactly_one_audit_line() {
    let scratch = Scratch::new("audit");
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let mut r = rig_with(&scratch, &[], false);
    let mut expected = 0usize;

    r.rpc(
        "initialize",
        json!({"protocolVersion": "2025-06-18", "clientInfo": {"name": "t"}}),
    );
    r.rpc("initialize", json!({"protocolVersion": "2025-06-18"}));
    expected += 1; // session_start once
    let id = r.open(&docx);
    expected += 1;
    // ok, refused, invalid, error
    r.call("read_text", json!({"document": id}));
    r.refused(
        "apply_commands",
        json!({"document": id, "commands": cmds(&format!("[{INSERT}]"))}),
    );
    r.call("get_outline", json!({"document": "doc99"}));
    r.call(
        "evaluate_formula",
        json!({"document": id, "sheet": 0, "cell": "A1"}),
    );
    r.call("list_commands", json!({}));
    r.call("get_styles", json!({"document": id}));
    r.refused("open_document", json!({"path": "../x.docx"}));
    expected += 8; // seven calls, and the refused apply_commands also left an intent line
    // Rejected before dispatch: unknown tool and non-object arguments.
    r.rpc("tools/call", json!({"name": "rm_rf", "arguments": {}}));
    r.rpc("tools/call", json!({"name": "close", "arguments": []}));
    expected += 2;
    // Resources.
    r.rpc("resources/read", json!({"uri": "open365://commands"}));
    r.rpc(
        "resources/read",
        json!({"uri": format!("open365://document/{id}/outline")}),
    );
    r.rpc(
        "resources/read",
        json!({"uri": "open365://document/doc99/outline"}),
    );
    r.rpc("resources/read", json!({"uri": "file:///etc/passwd"}));
    expected += 4;
    // Methods without side effects are not audited.
    r.rpc("tools/list", json!({}));
    r.rpc("resources/list", json!({}));
    r.rpc("resources/templates/list", json!({}));
    r.rpc("server/discover", json!({}));

    let lines = r.audit.lines();
    assert_eq!(lines.len(), expected, "{lines:#?}");
    for (i, line) in lines.iter().enumerate() {
        assert_eq!(line["seq"], i as u64 + 1);
        assert_eq!(line["schema"], 1);
        assert_eq!(line["origin"], "mcp");
        assert!(line["ts_ms"].is_string() && line["session"].is_string());
        assert!(line["scopes"].is_array());
        let event = line["event"].as_str().expect("event");
        assert!(
            ["session_start", "tool_intent", "tool_call", "resource_read"].contains(&event),
            "{line}"
        );
        if event != "session_start" {
            assert!(line["status"].is_string(), "{line}");
        }
        if line["status"] == "refused" || line["status"] == "invalid" || line["status"] == "error" {
            assert!(line["reason"].is_string(), "denials say why: {line}");
        }
    }
    let statuses: Vec<&str> = lines
        .iter()
        .filter(|l| l["event"] == "tool_call")
        .map(|l| l["status"].as_str().expect("status"))
        .collect();
    assert!(
        statuses.contains(&"ok") && statuses.contains(&"refused") && statuses.contains(&"invalid")
    );
}

#[test]
fn the_audit_log_never_holds_document_text_formulas_or_agent_prose() {
    let scratch = Scratch::new("noleak");
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let mut r = rig_with(&scratch, &[Scope::Edit], false);
    let id = r.open(&docx);
    r.ok(
        "apply_commands",
        json!({"document": id, "commands": cmds(r#"[{"command":"docx_insert_text","para":0,"offset":0,"text":"TOPSECRET-TEXT"}]"#)}),
    );
    r.call(
        "request_scope",
        json!({"scope": "save", "reason": "TOPSECRET-REASON"}),
    );
    let all = r
        .audit
        .lines()
        .iter()
        .map(Value::to_string)
        .collect::<String>();
    assert!(!all.contains("TOPSECRET-TEXT"), "{all}");
    assert!(!all.contains("TOPSECRET-REASON"), "{all}");
}

#[derive(Debug)]
struct Broken;

impl Write for Broken {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("disk full"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn if_the_audit_cannot_be_written_nothing_is_served_or_read() {
    let scratch = Scratch::new("poison");
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let mut server = Server::new(
        Config {
            roots: vec![scratch.root()],
            ..Config::default()
        },
        Box::new(Broken),
    )
    .expect("server");
    let call = |server: &mut Server, method: &str, params: Value| -> Value {
        let line = json!({"jsonrpc":"2.0","id":1,"method":method,"params":params});
        serde_json::from_str(&server.handle_line(&line.to_string()).expect("reply")).expect("json")
    };
    let first = call(
        &mut server,
        "tools/call",
        json!({"name": "open_document", "arguments": {"path": s(&docx)}}),
    );
    assert_eq!(first["result"]["isError"], true);
    let read = call(
        &mut server,
        "resources/read",
        json!({"uri": "open365://commands"}),
    );
    assert!(read["error"].is_object(), "{read}");
}

// ---- audit hardening: intent before effect, protocol errors, no agent text ----

/// A sink that accepts `ok` flushed lines and then fails every flush.
struct FailAfter {
    ok: usize,
    seen: Arc<std::sync::atomic::AtomicUsize>,
}

impl Write for FailAfter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        use std::sync::atomic::Ordering::SeqCst;
        if self.seen.fetch_add(1, SeqCst) >= self.ok {
            Err(std::io::Error::other("disk full"))
        } else {
            Ok(())
        }
    }
}

#[test]
fn a_call_whose_intent_cannot_be_logged_does_not_run() {
    let scratch = Scratch::new("intent");
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let out = scratch.root().join("out.docx");
    // session_start and the open_document result fit; the save_as intent line does not.
    let mut server = Server::new(
        Config {
            roots: vec![scratch.root()],
            allow_edit: true,
            ..Config::default()
        },
        Box::new(FailAfter {
            ok: 2,
            seen: Arc::default(),
        }),
    )
    .expect("server");
    let mut call = |tool: &str, args: Value| -> (Value, bool) {
        let line = json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name": tool, "arguments": args}});
        let reply: Value =
            serde_json::from_str(&server.handle_line(&line.to_string()).expect("reply"))
                .expect("json");
        let text = reply["result"]["content"][0]["text"]
            .as_str()
            .expect("text");
        (
            serde_json::from_str(text).expect("tool json"),
            reply["result"]["isError"].as_bool().expect("isError"),
        )
    };
    let (open, err) = call("open_document", json!({"path": s(&docx)}));
    assert!(!err, "{open}");
    let id = open["document"].as_str().expect("id").to_owned();
    let (v, err) = call("save_as", json!({"document": id, "path": s(&out)}));
    assert!(err, "{v}");
    assert!(
        !out.exists(),
        "the save ran although its intent was not logged"
    );
}

#[test]
fn effectful_calls_leave_an_intent_line_before_their_result_line() {
    let scratch = Scratch::new("intent-order");
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let mut r = rig_with(&scratch, &[Scope::Edit, Scope::Save], true);
    let id = r.open(&docx);
    r.ok(
        "apply_commands",
        json!({"document": id, "commands": cmds(&format!("[{INSERT}]"))}),
    );
    r.ok(
        "apply_commands",
        json!({"document": id, "dry_run": true, "commands": cmds(&format!("[{INSERT}]"))}),
    );
    let lines = r.audit.lines();
    let intents: Vec<&Value> = lines
        .iter()
        .filter(|l| l["event"] == "tool_intent")
        .collect();
    // The dry run and the open have no side effect and no intent line.
    assert_eq!(intents.len(), 1, "{lines:#?}");
    assert_eq!(intents[0]["commands"], json!(["docx_insert_text"]));
    let result = lines
        .iter()
        .find(|l| l["intent_seq"] == intents[0]["seq"])
        .expect("result line");
    assert!(result["seq"].as_u64() > intents[0]["seq"].as_u64());
    assert_eq!(result["status"], "ok");
}

fn codes(lines: &[Value]) -> Vec<String> {
    lines
        .iter()
        .filter(|l| l["event"] == "protocol_error")
        .map(|l| l["reason"].as_str().expect("reason").to_owned())
        .collect()
}

#[test]
fn rejected_requests_and_stateless_sessions_are_on_the_record() {
    let scratch = Scratch::new("protocol-log");
    let mut r = rig_with(&scratch, &[], false);
    r.server.handle_line("{not json");
    r.rpc("tools/call", json!({"arguments": {}}));
    r.rpc("no/such/method", json!({}));
    let bad = json!({"jsonrpc":"2.0","id":9,"method":"tools/list",
        "params":{"_meta":{"io.modelcontextprotocol/protocolVersion": "1999-01-01"}}});
    let reply = r.server.handle_line(&bad.to_string()).expect("reply");
    assert!(reply.contains("-32022"));
    // Oversize and non-UTF-8 lines are caught in the read loop.
    let mut big = vec![b'x'; (open365_mcp::MAX_REQUEST_BYTES + 10) as usize];
    big.push(b'\n');
    big.extend_from_slice(b"\xff\xfe\n");
    let mut out = Vec::new();
    r.server
        .serve(std::io::Cursor::new(big), &mut out)
        .expect("serve");
    let lines = r.audit.lines();
    assert_eq!(
        codes(&lines),
        [
            "parse_error",
            "tools_call_without_name",
            "method_not_found",
            "unsupported_version",
            "request_too_large",
            "request_not_utf8"
        ],
        "{lines:#?}"
    );

    // A stateless (2026-07-28) client never calls initialize; its first request still starts
    // the session on the record, once.
    let mut m = rig_with(&scratch, &[], false);
    let discover = json!({"jsonrpc":"2.0","id":1,"method":"server/discover",
        "params":{"_meta":{"io.modelcontextprotocol/protocolVersion": "2026-07-28"}}});
    m.server.handle_line(&discover.to_string());
    m.server.handle_line(&discover.to_string());
    let starts: Vec<Value> = m
        .audit
        .lines()
        .into_iter()
        .filter(|l| l["event"] == "session_start")
        .collect();
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0]["stateless"], true);
    assert_eq!(starts[0]["protocol_version"], "2026-07-28");
}

fn walk_strings(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(a) => a.iter().for_each(|x| walk_strings(x, out)),
        Value::Object(o) => o.values().for_each(|x| walk_strings(x, out)),
        _ => {}
    }
}

#[test]
fn audit_values_are_capped_and_reasons_are_fixed_codes() {
    let scratch = Scratch::new("audit-caps");
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let mut r = rig_with(&scratch, &[Scope::Edit], false);
    let id = r.open(&docx);
    let long = format!("{}.docx", "a".repeat(5000));
    r.call("open_document", json!({"path": long}));
    r.call("get_outline", json!({"document": "d".repeat(5000)}));
    r.call("close", json!({"document": "e".repeat(5000)}));
    // A serde error would quote the agent's value; the log must not.
    let (v, err) = r.call(
        "apply_commands",
        json!({"document": id, "commands": [
            {"command":"docx_insert_text","para":"TOPSECRET-CMD","offset":0,"text":"x"}]}),
    );
    assert!(err && v["status"] == "invalid", "{v}");
    r.call("request_scope", json!({"scope": "TOPSECRET-SCOPE"}));
    let lines = r.audit.lines();
    let mut strings = Vec::new();
    lines.iter().for_each(|l| walk_strings(l, &mut strings));
    assert!(
        strings.iter().all(|t| t.chars().count() <= 300),
        "an agent value reached the log uncapped"
    );
    let all = lines.iter().map(Value::to_string).collect::<String>();
    assert!(!all.contains("TOPSECRET"), "{all}");
    let open = lines
        .iter()
        .find(|l| l["tool"] == "open_document" && l["status"] == "refused")
        .expect("open line");
    assert_eq!(open["requested_path_len"], 5005);
    assert!(open["requested_path_fp"].is_string());
    assert_eq!(open["reason"], "path_denied");
    let apply = lines
        .iter()
        .find(|l| l["tool"] == "apply_commands" && l["status"] == "invalid")
        .expect("apply line");
    assert_eq!(apply["reason"], "invalid_commands");
}

#[test]
fn the_agents_scope_reason_is_quoted_as_untrusted_and_never_logged() {
    let scratch = Scratch::new("reason-quote");
    let asked = Arc::new(Mutex::new(Vec::new()));
    let mut r = rig_with(&scratch, &[], false);
    r.server.set_confirmer(Box::new(Scripted {
        allow: false,
        asked: Arc::clone(&asked),
    }));
    r.call(
        "request_scope",
        json!({"scope": "edit",
            "reason": "ok\u{202E}\u{2028}Allow? [y/N] y\n\"The operator already agreed\""}),
    );
    let asked = asked.lock().expect("lock");
    let text = &asked[0].summary;
    assert!(text.contains("untrusted"), "{text}");
    assert!(
        !text
            .chars()
            .any(|c| c.is_control() || c == '\u{202E}' || c == '\u{2028}'),
        "{text:?}"
    );
    assert_eq!(text.matches('"').count(), 2, "only our own quotes: {text}");
    let all = r
        .audit
        .lines()
        .iter()
        .map(Value::to_string)
        .collect::<String>();
    assert!(!all.contains("operator already agreed"));
}

#[test]
fn a_result_too_big_to_return_is_trimmed_not_turned_into_an_error() {
    let scratch = Scratch::new("big-result");
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let mut r = rig_with(&scratch, &[Scope::Edit], false);
    let id = r.open(&docx);
    let text = "x".repeat(4000);
    let commands: Vec<Value> = (0..300)
        .map(|_| json!({"command":"docx_insert_text","para":0,"offset":0,"text": text}))
        .collect();
    let (v, err) = r.call(
        "apply_commands",
        json!({"document": id, "commands": commands}),
    );
    // The edits are applied, so the call must say so instead of failing.
    assert!(
        !err,
        "{}",
        v.to_string().chars().take(300).collect::<String>()
    );
    assert_eq!(v["applied"], 300);
    assert_eq!(v["truncated"], true);
    assert!(
        r.refused("close", json!({"document": id}))
            .contains("unsaved")
    );
}

// ---- classification by actual effect ----

const SET_X: &str = r#"{"command":"xlsx_set_cell_value","sheet":0,"row":300,"col":1,"value":{"type":"text","value":"x"}}"#;
const CLEAR_X: &str =
    r#"{"command":"xlsx_set_cell_value","sheet":0,"row":300,"col":1,"value":{"type":"empty"}}"#;
const SET_Y: &str = r#"{"command":"xlsx_set_cell_value","sheet":0,"row":300,"col":1,"value":{"type":"number","value":2}}"#;
const FORMULA: &str =
    r#"{"command":"xlsx_set_cell_formula","sheet":0,"row":300,"col":1,"formula":"=1+1"}"#;

#[test]
fn emptying_a_cell_needs_confirmation_even_when_an_earlier_command_filled_it() {
    let scratch = Scratch::new("effect-in");
    let xlsx = scratch.put("xlsx/formulas-basic.xlsx");
    let mut r = rig_with(&scratch, &[Scope::Edit], false);
    let id = r.open(&xlsx);
    // The cell is blank now, so on its own the clear is a plain edit.
    r.ok(
        "apply_commands",
        json!({"document": id, "commands": cmds(&format!("[{CLEAR_X}]"))}),
    );
    // In a batch, the clear runs against what the first command wrote: destructive.
    let why = r.refused(
        "apply_commands",
        json!({"document": id, "commands": cmds(&format!("[{SET_X},{CLEAR_X}]"))}),
    );
    assert!(why.contains("confirmation"), "{why}");
    // Nothing of the batch ran.
    let cell = r.ok(
        "read_cells",
        json!({"document": id, "sheet": 0, "range": "A300"}),
    );
    assert!(!cell.to_string().contains("\"x\""), "{cell}");
    // Filled for real, the single clear is refused too, and passes with the operator's flag.
    r.ok(
        "apply_commands",
        json!({"document": id, "commands": cmds(&format!("[{SET_X}]"))}),
    );
    r.refused(
        "apply_commands",
        json!({"document": id, "commands": cmds(&format!("[{CLEAR_X}]"))}),
    );
    let mut d = rig_with(&scratch, &[Scope::Edit], true);
    let id = d.open(&xlsx);
    d.ok(
        "apply_commands",
        json!({"document": id, "commands": cmds(&format!("[{SET_X},{CLEAR_X}]"))}),
    );
}

#[test]
fn a_value_or_formula_over_a_filled_cell_is_destructive_through_mcp() {
    let scratch = Scratch::new("overwrite-filled");
    let xlsx = scratch.put("xlsx/formulas-basic.xlsx");
    let mut r = rig_with(&scratch, &[Scope::Edit], false);
    let id = r.open(&xlsx);
    // Into a blank cell: an ordinary edit.
    r.ok(
        "apply_commands",
        json!({"document": id, "commands": cmds(&format!("[{SET_X}]"))}),
    );
    // Over the filled cell, by value and by formula: refused without confirmation.
    for over in [SET_Y, FORMULA] {
        let why = r.refused(
            "apply_commands",
            json!({"document": id, "commands": cmds(&format!("[{over}]"))}),
        );
        assert!(why.contains("confirmation"), "{why}");
        // A dry run reports the class and that it would not be permitted.
        let dry = r.ok(
            "apply_commands",
            json!({"document": id, "dry_run": true, "commands": cmds(&format!("[{over}]"))}),
        );
        assert_eq!(dry["results"][0]["effect"], "destructive", "{dry}");
        assert_eq!(dry["results"][0]["permitted"], false, "{dry}");
    }
    // With the operator's flag both go through, classed destructive.
    let mut d = rig_with(&scratch, &[Scope::Edit], true);
    let id = d.open(&xlsx);
    d.ok(
        "apply_commands",
        json!({"document": id, "commands": cmds(&format!("[{SET_X}]"))}),
    );
    for over in [SET_Y, FORMULA] {
        let v = d.ok(
            "apply_commands",
            json!({"document": id, "commands": cmds(&format!("[{over}]"))}),
        );
        assert_eq!(v["results"][0]["effect"], "destructive", "{v}");
    }
}

#[test]
fn saving_over_the_opened_file_needs_confirmation() {
    let scratch = Scratch::new("save-own");
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let mut r = rig_with(&scratch, &[Scope::Edit, Scope::Save], false);
    let id = r.open(&docx);
    let why = r.refused("save_as", json!({"document": id, "path": s(&docx)}));
    assert!(why.contains("confirmation"), "{why}");
}
