//! In-process JSON-RPC sessions against the real fixtures, copied into temp roots, plus one
//! test per security rule.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use open365_mcp::{
    Config, Fence, MAX_COMMANDS, MAX_DOCUMENT_BYTES, MAX_OPEN_DOCUMENTS, MAX_REQUEST_BYTES, Server,
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
        let dir = std::env::temp_dir().join(format!("open365-mcp-{}-{test}", std::process::id()));
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
    /// Copies a fixture into the root and returns its path.
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

struct Rig {
    server: Server,
    audit: Buf,
    next: u64,
}

fn rig(scratch: &Scratch, edit: bool, destructive: bool, hidden: bool) -> Rig {
    let audit = Buf::default();
    let server = Server::new(
        Config {
            roots: vec![scratch.root()],
            allow_edit: edit,
            allow_destructive: destructive,
            include_hidden: hidden,
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

impl Rig {
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let line = json!({"jsonrpc":"2.0","id":self.next,"method":method,"params":params});
        let reply = self.server.handle_line(&line.to_string()).expect("reply");
        serde_json::from_str(&reply).expect("reply json")
    }

    /// Returns (parsed tool output, is_error).
    fn call(&mut self, tool: &str, args: Value) -> (Value, bool) {
        let reply = self.rpc("tools/call", json!({"name": tool, "arguments": args}));
        let result = &reply["result"];
        let text = result["content"][0]["text"].as_str().expect("text");
        (
            serde_json::from_str(text).expect("tool json"),
            result["isError"].as_bool().expect("isError"),
        )
    }

    fn open(&mut self, path: &Path) -> String {
        let (v, err) = self.call("open_document", json!({"path": s(path)}));
        assert!(!err, "{v}");
        v["document"].as_str().expect("id").to_owned()
    }

    fn refused(&mut self, tool: &str, args: Value) -> String {
        let (v, err) = self.call(tool, args);
        assert!(err, "expected refusal, got {v}");
        assert_eq!(v["status"], "refused", "{v}");
        v["error"].as_str().expect("error").to_owned()
    }
}

/// A 3x3 sheet whose row 2 and column C are hidden.
fn hidden_rows_xlsx() -> Vec<u8> {
    let types = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#;
    let root = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let book = r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let book_rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;
    let cell = |r: &str, v: &str| format!(r#"<c r="{r}" t="inlineStr"><is><t>{v}</t></is></c>"#);
    let sheet = format!(
        r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cols><col min="3" max="3" width="9" hidden="1" customWidth="1"/></cols><sheetData><row r="1">{}{}{}</row><row r="2" hidden="1">{}{}{}</row><row r="3">{}{}{}</row></sheetData></worksheet>"#,
        cell("A1", "a1"),
        cell("B1", "b1"),
        cell("C1", "SECRET-C1"),
        cell("A2", "SECRET-A2"),
        cell("B2", "SECRET-B2"),
        cell("C2", "SECRET-C2"),
        cell("A3", "a3"),
        cell("B3", "b3"),
        cell("C3", "SECRET-C3"),
    );
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in [
        ("[Content_Types].xml", types.to_owned()),
        ("_rels/.rels", root.to_owned()),
        ("xl/workbook.xml", book.to_owned()),
        ("xl/_rels/workbook.xml.rels", book_rels.to_owned()),
        ("xl/worksheets/sheet1.xml", sheet),
    ] {
        w.start_file(name, zip::write::SimpleFileOptions::default())
            .expect("start");
        w.write_all(data.as_bytes()).expect("write");
    }
    w.finish().expect("finish").into_inner()
}

const INSERT: &str = r#"{"command":"docx_insert_text","para":0,"offset":5,"text":"XY"}"#;
const DELETE: &str =
    r#"{"command":"docx_delete_range","start":{"para":0,"offset":0},"end":{"para":0,"offset":3}}"#;

fn cmds(text: &str) -> Value {
    serde_json::from_str(text).expect("commands json")
}

fn s(p: &Path) -> &str {
    p.to_str().expect("utf8")
}

// ---- protocol ----

#[test]
fn json_rpc_session_covers_the_protocol() {
    let scratch = Scratch::new("protocol");
    let mut r = rig(&scratch, false, false, false);
    let init = r.rpc(
        "initialize",
        json!({"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"0"}}),
    );
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(init["result"]["serverInfo"]["name"], "open365-mcp");
    assert!(init["result"]["capabilities"]["tools"].is_object());
    let newer = r.rpc("initialize", json!({"protocolVersion":"2099-01-01"}));
    assert_eq!(newer["result"]["protocolVersion"], "2025-06-18");

    // Notifications get no reply.
    let note = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
    assert!(r.server.handle_line(&note.to_string()).is_none());

    assert_eq!(r.rpc("ping", json!({}))["result"], json!({}));
    let list = r.rpc("tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().expect("tools");
    let names: Vec<&str> = tools
        .iter()
        .map(|t| t["name"].as_str().expect("name"))
        .collect();
    assert_eq!(
        names,
        [
            "open_document",
            "get_outline",
            "read_text",
            "read_cells",
            "inspect",
            "apply_commands",
            "save_as",
            "close"
        ]
    );
    assert!(tools.iter().all(|t| t["inputSchema"].is_object()));

    assert_eq!(r.rpc("nope", json!({}))["error"]["code"], -32601);
    let unknown = r.rpc("tools/call", json!({"name":"rm_rf","arguments":{}}));
    assert_eq!(unknown["error"]["code"], -32602);
    let bad_args = r.rpc("tools/call", json!({"name":"close","arguments":[]}));
    assert_eq!(bad_args["error"]["code"], -32602);

    let bad = r.server.handle_line("{not json").expect("reply");
    assert!(bad.contains("-32700"));
    let batch = r
        .server
        .handle_line(r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#)
        .expect("reply");
    assert!(batch.contains("-32600"));
    let bad_id = r
        .server
        .handle_line(r#"{"jsonrpc":"2.0","id":{"x":1},"method":"ping"}"#)
        .expect("reply");
    assert!(bad_id.contains("-32600"));
    let string_id = r
        .server
        .handle_line(r#"{"jsonrpc":"2.0","id":"abc","method":"ping"}"#)
        .expect("reply");
    assert!(string_id.contains("\"abc\""));
}

#[test]
fn serve_reads_lines_until_eof_and_limits_request_size() {
    let scratch = Scratch::new("serve");
    let mut r = rig(&scratch, false, false, false);
    let mut input = Vec::new();
    input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n\n");
    input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n");
    input.extend(vec![
        b'a';
        usize::try_from(MAX_REQUEST_BYTES).expect("fits") + 10
    ]);
    input.push(b'\n');
    input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n");
    let mut out = Vec::new();
    r.server
        .serve(std::io::Cursor::new(input), &mut out)
        .expect("serve");
    let lines: Vec<Value> = String::from_utf8(out)
        .expect("utf8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("json"))
        .collect();
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert_eq!(lines[0]["id"], 1);
    assert_eq!(lines[1]["error"]["code"], -32600);
    assert_eq!(lines[2]["id"], 2);
}

// ---- reads ----

#[test]
fn reads_docx_outline_and_text_without_hidden_runs() {
    let scratch = Scratch::new("docx-read");
    let headings = scratch.put("docx/styles-headings.docx");
    let hidden = scratch.put("docx/hidden-text.docx");
    let mut r = rig(&scratch, false, false, false);
    let id = r.open(&headings);
    let (outline, err) = r.call("get_outline", json!({"document": id}));
    assert!(!err);
    assert_eq!(outline["headings"][0]["text"], "Heading one");
    assert_eq!(outline["headings"][1]["level"], 2);
    let (text, _) = r.call("read_text", json!({"document": id, "start": 1, "count": 2}));
    assert_eq!(text["paragraphs"].as_array().expect("paragraphs").len(), 2);
    assert_eq!(text["paragraphs"][0]["paragraph"], 1);

    let h = r.open(&hidden);
    let (t, _) = r.call("read_text", json!({"document": h}));
    let body = t.to_string();
    assert!(!body.contains("HIDDEN-SECRET-TEXT"), "{body}");
    assert!(body.contains("Visible start."));
    assert_eq!(t["paragraphs"][0]["offsets_match_text"], false);

    let mut with = rig(&scratch, false, false, true);
    let h = with.open(&hidden);
    let (t, _) = with.call("read_text", json!({"document": h}));
    assert!(t.to_string().contains("HIDDEN-SECRET-TEXT"));
}

#[test]
fn reads_cells_and_skips_hidden_sheets_rows_and_columns() {
    let scratch = Scratch::new("xlsx-read");
    let formulas = scratch.put("xlsx/formulas-basic.xlsx");
    let sheets = scratch.put("xlsx/hidden-sheet.xlsx");
    let geometry = scratch.root().join("hidden-rows.xlsx");
    std::fs::write(&geometry, hidden_rows_xlsx()).expect("write");
    let mut r = rig(&scratch, false, false, false);
    let id = r.open(&formulas);
    let (cells, err) = r.call(
        "read_cells",
        json!({"document": id, "sheet": "Sheet1", "range": "A1:D2"}),
    );
    assert!(!err, "{cells}");
    let all = cells["cells"].as_array().expect("cells");
    assert!(
        all.iter()
            .any(|c| c["ref"] == "D1" && c["formula"] == "SUM(A1:A5)" && c["value"] == 15.0)
    );
    let (e, err) = r.call(
        "read_cells",
        json!({"document": id, "sheet": 0, "range": "A1:ZZ9999"}),
    );
    assert!(
        err && e["error"].as_str().expect("e").contains("limit"),
        "{e}"
    );
    let (e, err) = r.call(
        "read_cells",
        json!({"document": id, "sheet": 0, "range": "nonsense"}),
    );
    assert!(err, "{e}");

    let hs = r.open(&sheets);
    let (outline, _) = r.call("get_outline", json!({"document": hs}));
    let o = outline.to_string();
    assert!(
        o.contains("Visible") && !o.contains("HiddenSheet") && !o.contains("VeryHiddenSheet"),
        "{o}"
    );
    for sheet in [
        json!(1),
        json!(2),
        json!("HiddenSheet"),
        json!("VeryHiddenSheet"),
    ] {
        let (e, err) = r.call(
            "read_cells",
            json!({"document": hs, "sheet": sheet, "range": "A1"}),
        );
        assert!(err, "{e}");
        assert_eq!(e["error"], "no such sheet");
    }

    let g = r.open(&geometry);
    let (plain, err) = r.call(
        "read_cells",
        json!({"document": g, "sheet": 0, "range": "A1:C3"}),
    );
    assert!(!err, "{plain}");
    let shown: Vec<&str> = plain["cells"]
        .as_array()
        .expect("cells")
        .iter()
        .map(|c| c["ref"].as_str().expect("ref"))
        .collect();
    assert_eq!(shown, ["A1", "B1", "A3", "B3"]);
    assert!(!plain.to_string().contains("SECRET"));
    let mut with = rig(&scratch, false, false, true);
    let g = with.open(&geometry);
    let (full, _) = with.call(
        "read_cells",
        json!({"document": g, "sheet": 0, "range": "A1:C3"}),
    );
    assert_eq!(full["cells"].as_array().expect("cells").len(), 9);
    let hs = with.open(&sheets);
    let (v, err) = with.call(
        "read_cells",
        json!({"document": hs, "sheet": "HiddenSheet", "range": "A1"}),
    );
    assert!(!err, "{v}");
}

#[test]
fn inspect_omits_hidden_parts_unless_included() {
    let scratch = Scratch::new("inspect");
    let comments = scratch.put("docx/comments.docx");
    let mut r = rig(&scratch, false, false, false);
    let id = r.open(&comments);
    let (v, err) = r.call("inspect", json!({"document": id}));
    assert!(!err);
    let text = v.to_string().to_lowercase();
    assert!(
        !text.contains("comments") && !text.contains("docprops"),
        "{text}"
    );
    assert!(v.get("hidden").is_none());
    assert!(v["parts"].as_array().is_some_and(|p| !p.is_empty()));

    let mut with = rig(&scratch, false, false, true);
    let id = with.open(&comments);
    let (v, _) = with.call("inspect", json!({"document": id}));
    assert!(v.to_string().to_lowercase().contains("comments"));
    assert!(
        v["hidden"]
            .as_array()
            .is_some_and(|h| h.iter().any(|f| f["category"] == "comment"))
    );
}

// ---- fenced roots ----

#[test]
fn paths_outside_the_roots_are_refused() {
    let scratch = Scratch::new("fence");
    scratch.put("docx/plain-paragraphs.docx");
    let outside = scratch.outside().join("secret.docx");
    std::fs::copy(fixture("docx/plain-paragraphs.docx"), &outside).expect("copy");
    let mut r = rig(&scratch, true, true, false);
    let escape = format!("{}/../outside/secret.docx", s(&scratch.root()));
    for path in [s(&outside), escape.as_str(), "../outside/secret.docx"] {
        r.refused("open_document", json!({"path": path}));
    }
    // Relative paths inside the root work.
    let (v, err) = r.call("open_document", json!({"path": "plain-paragraphs.docx"}));
    assert!(!err, "{v}");
}

#[cfg(windows)]
fn symlink(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_file(target, link).is_ok()
}
/// Falls back to a junction, which needs no privilege.
#[cfg(windows)]
fn symlink_dir(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_dir(target, link).is_ok()
        || std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J", s(link), s(target)])
            .output()
            .is_ok_and(|o| o.status.success())
}
#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}
#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) -> bool {
    symlink(target, link)
}

#[test]
fn symlinks_that_leave_the_root_are_refused() {
    let scratch = Scratch::new("symlink");
    let outside = scratch.outside().join("secret.docx");
    std::fs::copy(fixture("docx/plain-paragraphs.docx"), &outside).expect("copy");
    let link = scratch.root().join("link.docx");
    let inside = scratch.put("docx/plain-paragraphs.docx");
    let mut r = rig(&scratch, true, true, false);
    let id = r.open(&inside);
    if symlink(&outside, &link) {
        r.refused("open_document", json!({"path": s(&link)}));
        // Saving through a symlink target is refused as well.
        r.refused("save_as", json!({"document": id, "path": s(&link)}));
    } else {
        eprintln!("file symlinks unavailable on this machine");
    }
    // A linked folder that leaves the root is refused too.
    let dir_link = scratch.root().join("dir");
    if !symlink_dir(&scratch.outside(), &dir_link) {
        eprintln!("directory links unavailable on this machine");
    } else {
        r.refused(
            "open_document",
            json!({"path": s(&dir_link.join("secret.docx"))}),
        );
        r.refused(
            "save_as",
            json!({"document": id, "path": s(&dir_link.join("new.docx"))}),
        );
    }
}

#[test]
fn unc_device_and_stream_paths_are_refused() {
    let scratch = Scratch::new("unc");
    let mut r = rig(&scratch, true, true, false);
    for path in [
        r"\\server\share\a.docx",
        "//server/share/a.docx",
        r"\\?\C:\Windows\a.docx",
        r"\\.\PhysicalDrive0",
        r"\\?\UNC\server\share\a.docx",
        "NUL.docx",
        "con",
        "a.docx:stream",
        "",
    ] {
        r.refused("open_document", json!({"path": path}));
    }
    assert!(Fence::new(&[PathBuf::from(r"\\server\share")]).is_err());
}

#[test]
fn roots_must_exist_and_be_given() {
    assert!(Fence::new(&[]).is_err());
    assert!(Fence::new(&[PathBuf::from("definitely/not/here")]).is_err());
}

// ---- policy ----

#[test]
fn read_only_by_default() {
    let scratch = Scratch::new("readonly");
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let mut r = rig(&scratch, false, false, false);
    let id = r.open(&docx);
    let why = r.refused(
        "apply_commands",
        json!({"document": id, "commands": cmds(&format!("[{INSERT}]"))}),
    );
    assert!(why.contains("--allow-edit"), "{why}");
    let out = scratch.root().join("out.docx");
    let why = r.refused("save_as", json!({"document": id, "path": s(&out)}));
    assert!(why.contains("--allow-edit"), "{why}");
    assert!(!out.exists());
    // Suggesting is allowed: a dry run reports effects and changes nothing.
    let (v, err) = r.call(
        "apply_commands",
        json!({"document": id, "dry_run": true, "commands": cmds(&format!("[{INSERT},{DELETE}]"))}),
    );
    assert!(!err, "{v}");
    assert_eq!(v["results"][0]["permitted"], false);
    assert_eq!(v["results"][1]["effect"], "destructive");
    let (t, _) = r.call("read_text", json!({"document": id}));
    assert!(!t.to_string().contains("XY"));
}

#[test]
fn destructive_needs_its_own_flag() {
    let scratch = Scratch::new("destructive");
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let mut edit = rig(&scratch, true, false, false);
    let id = edit.open(&docx);
    let why = edit.refused(
        "apply_commands",
        json!({"document": id, "commands": cmds(&format!("[{INSERT},{DELETE}]"))}),
    );
    assert!(
        why.contains("--allow-destructive") && why.contains("confirmation"),
        "{why}"
    );
    // The whole call was refused: the insert did not run.
    let (t, _) = edit.call("read_text", json!({"document": id, "count": 1}));
    assert!(!t.to_string().contains("XY"));
    let (v, err) = edit.call(
        "apply_commands",
        json!({"document": id, "commands": cmds(&format!("[{INSERT}]"))}),
    );
    assert!(!err, "{v}");
    assert_eq!(v["results"][0]["effect"], "edit");

    let mut both = rig(&scratch, true, true, false);
    let id = both.open(&docx);
    let (v, err) = both.call(
        "apply_commands",
        json!({"document": id, "commands": cmds(&format!("[{DELETE}]"))}),
    );
    assert!(!err, "{v}");
    assert_eq!(v["results"][0]["effect"], "destructive");
}

#[test]
fn save_inside_apply_commands_is_refused_and_save_as_is_checked() {
    let scratch = Scratch::new("save");
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let other = scratch.put("docx/styles-headings.docx");
    let mut r = rig(&scratch, true, false, false);
    let id = r.open(&docx);
    let save = json!([{"command": "save", "path": s(&scratch.outside().join("x.docx"))}]);
    let why = r.refused("apply_commands", json!({"document": id, "commands": save}));
    assert!(why.contains("save_as"));

    r.refused(
        "save_as",
        json!({"document": id, "path": s(&scratch.outside().join("x.docx"))}),
    );
    // Saving over the file it was opened from is a plain edit.
    let (v, err) = r.call("save_as", json!({"document": id, "path": s(&docx)}));
    assert!(!err, "{v}");
    assert_eq!(v["effect"], "edit");
    // A new path is an export.
    let (v, err) = r.call("save_as", json!({"document": id, "path": "copy.docx"}));
    assert!(!err, "{v}");
    assert_eq!(v["effect"], "export");
    // Overwriting a different existing file needs --allow-destructive.
    let why = r.refused("save_as", json!({"document": id, "path": s(&other)}));
    assert!(why.contains("--allow-destructive"));
    // Wrong extension for the document type.
    let (v, err) = r.call("save_as", json!({"document": id, "path": "copy.xlsx"}));
    assert!(err && v["status"] == "invalid", "{v}");

    let mut d = rig(&scratch, true, true, false);
    let id = d.open(&docx);
    let (v, err) = d.call("save_as", json!({"document": id, "path": s(&other)}));
    assert!(!err, "{v}");
}

#[test]
fn close_refuses_unsaved_changes() {
    let scratch = Scratch::new("close");
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let mut r = rig(&scratch, true, false, false);
    let id = r.open(&docx);
    r.call(
        "apply_commands",
        json!({"document": id, "commands": cmds(&format!("[{INSERT}]"))}),
    );
    r.refused("close", json!({"document": id}));
    let (v, err) = r.call("close", json!({"document": id, "discard": true}));
    assert!(!err, "{v}");
    let (v, err) = r.call("get_outline", json!({"document": id}));
    assert!(err && v["status"] == "invalid");
}

// ---- limits ----

#[test]
fn size_limits_are_enforced() {
    let scratch = Scratch::new("limits");
    let big = scratch.root().join("big.docx");
    let f = std::fs::File::create(&big).expect("create");
    f.set_len(MAX_DOCUMENT_BYTES + 1).expect("len");
    drop(f);
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let mut r = rig(&scratch, true, true, false);
    let why = r.refused("open_document", json!({"path": s(&big)}));
    assert!(why.contains("larger"));

    let id = r.open(&docx);
    let many = vec![cmds(INSERT); MAX_COMMANDS + 1];
    let why = r.refused("apply_commands", json!({"document": id, "commands": many}));
    assert!(why.contains("at most"));

    // A long document is cut off with a cursor instead of an oversized reply.
    let big_insert: Vec<Value> = (0..400)
        .map(|_| json!({"command":"docx_insert_text","para":0,"offset":0,"text":"x".repeat(2000)}))
        .collect();
    let (v, err) = r.call(
        "apply_commands",
        json!({"document": id, "commands": big_insert}),
    );
    assert!(!err, "{v}");
    let (t, err) = r.call("read_text", json!({"document": id, "count": 1000}));
    assert!(!err);
    assert!(t.to_string().len() < 300_000);

    for i in 0..MAX_OPEN_DOCUMENTS - 1 {
        let (v, err) = r.call("open_document", json!({"path": s(&docx)}));
        assert!(!err, "open {i}: {v}");
    }
    r.refused("open_document", json!({"path": s(&docx)}));
}

// ---- audit ----

#[test]
fn every_call_is_audited_including_refusals() {
    let scratch = Scratch::new("audit");
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let mut r = rig(&scratch, false, false, false);
    let id = r.open(&docx);
    r.call("read_text", json!({"document": id}));
    r.refused(
        "apply_commands",
        json!({"document": id, "commands": cmds(&format!("[{INSERT}]"))}),
    );
    r.refused("open_document", json!({"path": r"\\server\share\a.docx"}));
    r.call("get_outline", json!({"document": "doc99"}));
    let lines = r.audit.lines();
    assert_eq!(lines.len(), 5);
    let tools: Vec<&str> = lines
        .iter()
        .map(|l| l["tool"].as_str().expect("tool"))
        .collect();
    assert_eq!(
        tools,
        [
            "open_document",
            "read_text",
            "apply_commands",
            "open_document",
            "get_outline"
        ]
    );
    let statuses: Vec<&str> = lines
        .iter()
        .map(|l| l["status"].as_str().expect("status"))
        .collect();
    assert_eq!(statuses, ["ok", "ok", "refused", "refused", "invalid"]);
    assert_eq!(lines[2]["commands"], json!(["docx_insert_text"]));
    assert_eq!(lines[2]["origin"], "mcp");
    assert!(
        lines
            .iter()
            .enumerate()
            .all(|(i, l)| l["seq"] == i as u64 + 1 && l["ts_ms"].is_string())
    );
    // Document text never reaches the audit log.
    assert!(!lines.iter().any(|l| l.to_string().contains("XY")));
}

struct BrokenSink;

impl Write for BrokenSink {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("disk full"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn an_unwritable_audit_log_stops_the_server() {
    let scratch = Scratch::new("poison");
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let mut server = Server::new(
        Config {
            roots: vec![scratch.root()],
            ..Config::default()
        },
        Box::new(BrokenSink),
    )
    .expect("server");
    let call = |server: &mut Server, args: Value| -> Value {
        let line = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"open_document","arguments":args}});
        serde_json::from_str(&server.handle_line(&line.to_string()).expect("reply")).expect("json")
    };
    let first = call(&mut server, json!({"path": s(&docx)}));
    assert_eq!(first["result"]["isError"], true);
    assert!(first.to_string().contains("audit log"));
    let second = call(&mut server, json!({"path": s(&docx)}));
    assert_eq!(second["result"]["isError"], true);
}
