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

    /// The `tool_call` result lines only (not intents, session starts or protocol errors).
    fn calls(&self) -> Vec<Value> {
        self.lines()
            .into_iter()
            .filter(|l| l["event"] == "tool_call")
            .collect()
    }
}

/// True when no leftover temporary file (`*.open365-tmp`) is in `dir`.
fn no_temp_files(dir: &Path) -> bool {
    std::fs::read_dir(dir).expect("ls").all(|e| {
        !e.expect("entry")
            .file_name()
            .to_string_lossy()
            .ends_with(".open365-tmp")
    })
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
    assert_eq!(newer["result"]["protocolVersion"], "2025-11-25");

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
            "close",
            "export_pdf",
            "new_from_template",
            "layout_summary",
            "evaluate_formula",
            "list_commands",
            "read_formulas",
            "get_styles",
            "recalc",
            "request_scope",
            "share_send_copy",
            "share_send_pdf",
            "share_inspect",
            "share_sanitize_copy",
            "share_os_sheet"
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

/// File symlinks need a privilege on Windows (Developer Mode or elevation), so the tests that
/// need one are ignored there with that reason and run with `--ignored` where it is held. They
/// assert that the link was made: a refused link is a failure, never a silent pass. Folder
/// links use a junction on Windows (`mklink /J`), which needs no privilege, so those run
/// everywhere.
#[test]
#[cfg_attr(
    windows,
    ignore = "creating a file symlink on Windows needs Developer Mode or elevation; run with --ignored"
)]
fn file_symlinks_that_leave_the_root_are_refused() {
    let scratch = Scratch::new("symlink");
    let outside = scratch.outside().join("secret.docx");
    std::fs::copy(fixture("docx/plain-paragraphs.docx"), &outside).expect("copy");
    let link = scratch.root().join("link.docx");
    let inside = scratch.put("docx/plain-paragraphs.docx");
    let mut r = rig(&scratch, true, true, false);
    let id = r.open(&inside);
    assert!(
        symlink(&outside, &link),
        "the file symlink was refused by the operating system"
    );
    r.refused("open_document", json!({"path": s(&link)}));
    // Saving through a symlink target is refused as well.
    r.refused("save_as", json!({"document": id, "path": s(&link)}));
}

#[test]
fn directory_links_that_leave_the_root_are_refused() {
    let scratch = Scratch::new("dirlink");
    let outside = scratch.outside().join("secret.docx");
    std::fs::copy(fixture("docx/plain-paragraphs.docx"), &outside).expect("copy");
    let inside = scratch.put("docx/plain-paragraphs.docx");
    let mut r = rig(&scratch, true, true, false);
    let id = r.open(&inside);
    let dir_link = scratch.root().join("dir");
    assert!(
        symlink_dir(&scratch.outside(), &dir_link),
        "neither a directory symlink nor a junction could be created"
    );
    r.refused(
        "open_document",
        json!({"path": s(&dir_link.join("secret.docx"))}),
    );
    r.refused(
        "save_as",
        json!({"document": id, "path": s(&dir_link.join("new.docx"))}),
    );
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
    // Saving over the file it was opened from replaces an existing file: it needs the
    // operator's confirmation, like any overwrite.
    let why = r.refused("save_as", json!({"document": id, "path": s(&docx)}));
    assert!(why.contains("--allow-destructive"), "{why}");
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
    // With the flag, saving over the opened file goes through and is classed destructive.
    let (v, err) = d.call("save_as", json!({"document": id, "path": s(&other)}));
    assert!(!err, "{v}");
    assert_eq!(v["effect"], "destructive");
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
    let lines = r.audit.calls();
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
    assert!(lines.iter().all(|l| l["ts_ms"].is_string()));
    // The whole log is numbered without gaps; the refused apply_commands left an intent line
    // first, and its result line points back at it.
    let all = r.audit.lines();
    assert!(
        all.iter()
            .enumerate()
            .all(|(i, l)| l["seq"] == i as u64 + 1)
    );
    assert_eq!(all[0]["event"], "session_start");
    let intent = all
        .iter()
        .find(|l| l["event"] == "tool_intent")
        .expect("intent line");
    assert_eq!(intent["tool"], "apply_commands");
    assert_eq!(lines[2]["intent_seq"], intent["seq"]);
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

// ---- PDF export, templates, layout summary, formula evaluation ----

fn zip_has(path: &Path, name: &str) -> bool {
    let file = std::fs::File::open(path).expect("open zip");
    zip::ZipArchive::new(file)
        .expect("zip")
        .file_names()
        .any(|n| n.is_ok_and(|n| n == name))
}

fn put_as(scratch: &Scratch, rel: &str, name: &str) -> PathBuf {
    let dest = scratch.root().join(name);
    std::fs::copy(fixture(rel), &dest).expect("copy fixture");
    dest
}

#[test]
fn export_pdf_needs_edit_and_never_overwrites_without_the_flags() {
    let scratch = Scratch::new("pdf");
    let docx = scratch.put("docx/styles-headings.docx");
    let out = scratch.root().join("out.pdf");

    let mut ro = rig(&scratch, false, false, false);
    let id = ro.open(&docx);
    let why = ro.refused("export_pdf", json!({"document": id, "path": s(&out)}));
    assert!(why.contains("--allow-edit"), "{why}");
    assert!(!out.exists());

    let mut r = rig(&scratch, true, false, false);
    let id = r.open(&docx);
    let (v, err) = r.call("export_pdf", json!({"document": id, "path": s(&out)}));
    assert!(!err, "{v}");
    assert_eq!(v["effect"], "export");
    let pdf = std::fs::read(&out).expect("pdf");
    assert!(pdf.starts_with(b"%PDF-1.7"));
    assert!(!String::from_utf8_lossy(&pdf).contains("/Author"));

    // A second export is refused with and without the argument unless the operator allows it.
    let why = r.refused("export_pdf", json!({"document": id, "path": s(&out)}));
    assert!(why.contains("overwrite"), "{why}");
    let why = r.refused(
        "export_pdf",
        json!({"document": id, "path": s(&out), "overwrite": true}),
    );
    assert!(why.contains("--allow-destructive"), "{why}");
    assert_eq!(std::fs::read(&out).expect("pdf"), pdf);

    let mut d = rig(&scratch, true, true, false);
    let id = d.open(&docx);
    // The operator flag alone is not enough either: the call must ask.
    d.refused("export_pdf", json!({"document": id, "path": s(&out)}));
    let (v, err) = d.call(
        "export_pdf",
        json!({"document": id, "path": s(&out), "overwrite": true}),
    );
    assert!(!err, "{v}");
    assert!(no_temp_files(&scratch.root()));

    // Audited: one line per call with the effect, the resolved path and the outcome.
    let lines = r.audit.calls();
    assert_eq!(lines[1]["tool"], "export_pdf");
    assert_eq!(lines[1]["status"], "ok");
    assert_eq!(lines[1]["effect"], "export");
    assert_eq!(lines[2]["status"], "refused");
    assert!(
        lines[1]["path"]
            .as_str()
            .is_some_and(|p| p.ends_with("out.pdf"))
    );
    assert_eq!(ro.audit.calls()[1]["status"], "refused");
}

#[test]
fn export_pdf_is_fenced_and_checks_the_extension_and_document() {
    let scratch = Scratch::new("pdf-fence");
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let xlsx = scratch.put("xlsx/formulas-basic.xlsx");
    let mut r = rig(&scratch, true, true, true);
    let id = r.open(&docx);
    let outside = scratch.outside().join("x.pdf");
    r.refused("export_pdf", json!({"document": id, "path": s(&outside)}));
    r.refused(
        "export_pdf",
        json!({"document": id, "path": "../outside/x.pdf"}),
    );
    r.refused(
        "export_pdf",
        json!({"document": id, "path": r"\\server\share\x.pdf"}),
    );
    r.refused(
        "export_pdf",
        json!({"document": id, "path": "x.pdf:stream"}),
    );
    r.refused("export_pdf", json!({"document": id, "path": "NUL.pdf"}));
    assert!(!outside.exists());
    let (v, err) = r.call("export_pdf", json!({"document": id, "path": "x.docx"}));
    assert!(err && v["status"] == "invalid", "{v}");
    let x = r.open(&xlsx);
    let (v, err) = r.call("export_pdf", json!({"document": x, "path": "x.pdf"}));
    assert!(err && v["status"] == "invalid", "{v}");
    assert!(!scratch.root().join("x.pdf").exists());
}

#[test]
#[cfg_attr(
    windows,
    ignore = "creating a file symlink on Windows needs Developer Mode or elevation; run with --ignored"
)]
fn export_pdf_refuses_a_symlink_target() {
    let scratch = Scratch::new("pdf-link");
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let victim = scratch.outside().join("victim.pdf");
    std::fs::write(&victim, b"keep").expect("write");
    let link = scratch.root().join("link.pdf");
    assert!(
        symlink(&victim, &link),
        "the file symlink was refused by the operating system"
    );
    let mut r = rig(&scratch, true, true, false);
    let id = r.open(&docx);
    r.refused(
        "export_pdf",
        json!({"document": id, "path": "link.pdf", "overwrite": true}),
    );
    assert_eq!(std::fs::read(&victim).expect("read"), b"keep");
}

#[test]
fn new_from_template_makes_documents_and_keeps_macros_unrun() {
    let scratch = Scratch::new("template");
    put_as(&scratch, "docx/template-basic.dotx", "t.dotx");
    put_as(&scratch, "docx/template-macro.dotm", "m.dotm");
    put_as(&scratch, "xlsx/template-basic.xltx", "t.xltx");
    put_as(&scratch, "xlsx/template-macro.xltm", "m.xltm");
    let mut ro = rig(&scratch, false, false, false);
    let why = ro.refused(
        "new_from_template",
        json!({"path": "t.dotx", "output": "n.docx"}),
    );
    assert!(why.contains("--allow-edit"), "{why}");
    assert!(!scratch.root().join("n.docx").exists());

    let mut r = rig(&scratch, true, false, false);
    for (t, o, vba) in [
        ("t.dotx", "n.docx", false),
        ("m.dotm", "n.docm", true),
        ("t.xltx", "n.xlsx", false),
        ("m.xltm", "n.xlsm", true),
    ] {
        let (v, err) = r.call("new_from_template", json!({"path": t, "output": o}));
        assert!(!err, "{v}");
        assert_eq!(v["vba_project"], vba, "{v}");
        let made = scratch.root().join(o);
        let part = if o.contains("doc") {
            "word/vbaProject.bin"
        } else {
            "xl/vbaProject.bin"
        };
        assert_eq!(zip_has(&made, part), vba, "{o}");
    }
    // The new plain documents open as documents.
    let id = r.open(&scratch.root().join("n.docx"));
    let (v, err) = r.call("get_outline", json!({"document": id}));
    assert!(!err, "{v}");
    r.open(&scratch.root().join("n.xlsx"));

    let lines = r.audit.calls();
    assert_eq!(lines[0]["tool"], "new_from_template");
    assert_eq!(lines[0]["effect"], "export");
    assert_eq!(lines[0]["status"], "ok");
}

#[test]
fn new_from_template_is_fenced_and_never_overwrites() {
    let scratch = Scratch::new("template-fence");
    put_as(&scratch, "docx/template-basic.dotx", "t.dotx");
    let plain = scratch.put("docx/plain-paragraphs.docx");
    std::fs::write(scratch.outside().join("t.dotx"), b"x").expect("write");
    let mut r = rig(&scratch, true, false, false);
    let outside = scratch.outside().join("t.dotx");
    r.refused(
        "new_from_template",
        json!({"path": s(&outside), "output": "n.docx"}),
    );
    r.refused(
        "new_from_template",
        json!({"path": "t.dotx", "output": "../outside/n.docx"}),
    );
    r.refused(
        "new_from_template",
        json!({"path": "t.dotx", "output": r"\\server\share\n.docx"}),
    );
    // A document that is not a template, or the wrong output extension, is invalid.
    let (v, err) = r.call(
        "new_from_template",
        json!({"path": "plain-paragraphs.docx", "output": "n.docx"}),
    );
    assert!(err && v["status"] == "invalid", "{v}");
    let (v, err) = r.call(
        "new_from_template",
        json!({"path": "t.dotx", "output": "n.docm"}),
    );
    assert!(err && v["status"] == "invalid", "{v}");
    let renamed = scratch.root().join("fake.dotx");
    std::fs::copy(&plain, &renamed).expect("copy");
    let (v, err) = r.call(
        "new_from_template",
        json!({"path": "fake.dotx", "output": "n.docx"}),
    );
    assert!(err && v["status"] == "invalid", "{v}");
    assert!(!scratch.root().join("n.docx").exists());
    assert!(!scratch.root().join("n.docm").exists());

    let (_, err) = r.call(
        "new_from_template",
        json!({"path": "t.dotx", "output": "n.docx"}),
    );
    assert!(!err);
    let first = std::fs::read(scratch.root().join("n.docx")).expect("read");
    let why = r.refused(
        "new_from_template",
        json!({"path": "t.dotx", "output": "n.docx"}),
    );
    assert!(why.contains("overwrite"), "{why}");
    let why = r.refused(
        "new_from_template",
        json!({"path": "t.dotx", "output": "n.docx", "overwrite": true}),
    );
    assert!(why.contains("--allow-destructive"), "{why}");
    assert_eq!(
        std::fs::read(scratch.root().join("n.docx")).expect("read"),
        first
    );
}

#[test]
fn layout_summary_reports_pages_headings_and_tables_read_only() {
    let scratch = Scratch::new("layout");
    let heads = scratch.put("docx/styles-headings.docx");
    let table = scratch.put("docx/layout-table-pages.docx");
    let xlsx = scratch.put("xlsx/formulas-basic.xlsx");
    let mut r = rig(&scratch, false, false, false);
    let h = r.open(&heads);
    let (v, err) = r.call("layout_summary", json!({"document": h}));
    assert!(!err, "{v}");
    assert_eq!(v["page_count"], 1);
    assert_eq!(v["page_lines"].as_array().expect("lines").len(), 1);
    assert!(v["page_lines"][0].as_u64().expect("n") > 0);
    let heads_found = v["headings"].as_array().expect("headings");
    assert!(heads_found.len() >= 2, "{v}");
    assert_eq!(heads_found[0]["level"], 1);
    assert_eq!(heads_found[0]["page"], 1);
    assert!(v["notice"].as_str().is_some());

    let t = r.open(&table);
    let (v, err) = r.call("layout_summary", json!({"document": t}));
    assert!(!err, "{v}");
    assert!(v["page_count"].as_u64().expect("pages") >= 2, "{v}");
    let tables = v["tables"].as_array().expect("tables");
    assert!(!tables.is_empty(), "{v}");
    assert!(tables[0]["rows"].as_u64().expect("rows") > 0);
    assert!(tables[0]["columns"].as_u64().expect("cols") > 0);
    assert!(v.to_string().len() < 64 * 1024);

    let x = r.open(&xlsx);
    let (v, err) = r.call("layout_summary", json!({"document": x}));
    assert!(err && v["status"] == "invalid", "{v}");
    let (v, err) = r.call("layout_summary", json!({"document": "doc99"}));
    assert!(err && v["status"] == "invalid", "{v}");

    let lines = r.audit.calls();
    assert_eq!(lines[1]["tool"], "layout_summary");
    assert_eq!(lines[1]["effect"], "read");
    assert_eq!(lines[1]["status"], "ok");
}

#[test]
fn evaluate_formula_is_read_only_and_leaves_the_file_alone() {
    let scratch = Scratch::new("eval");
    let xlsx = scratch.put("xlsx/formulas-basic.xlsx");
    let before = std::fs::read(&xlsx).expect("read");
    let mut r = rig(&scratch, false, false, false);
    let id = r.open(&xlsx);
    let (v, err) = r.call(
        "evaluate_formula",
        json!({"document": id, "sheet": 0, "cell": "Z1", "formula": "=1+2"}),
    );
    assert!(!err, "{v}");
    assert_eq!(v["result"], json!({"type": "number", "value": 3.0}));
    assert_eq!(v["saved"], false);
    let (v, _) = r.call(
        "evaluate_formula",
        json!({"document": id, "sheet": 0, "cell": "Z1", "formula": "1/0"}),
    );
    assert_eq!(v["result"]["type"], "error");
    // Errors: bad cell, bad formula, no such sheet, empty cell without formula, wrong type.
    for args in [
        json!({"document": id, "sheet": 0, "cell": "ZZZZ", "formula": "1"}),
        json!({"document": id, "sheet": 0, "cell": "A1", "formula": "1+"}),
        json!({"document": id, "sheet": 9, "cell": "A1", "formula": "1"}),
        json!({"document": id, "sheet": 0, "cell": "Z9"}),
        json!({"document": id, "sheet": 0, "cell": "A1", "formula": 5}),
    ] {
        let (v, err) = r.call("evaluate_formula", args);
        assert!(err && v["status"] == "invalid", "{v}");
    }
    // Size cap: formulas over 8192 characters are refused.
    let long = format!("1{}", "+1".repeat(5000));
    let (v, err) = r.call(
        "evaluate_formula",
        json!({"document": id, "sheet": 0, "cell": "Z1", "formula": long}),
    );
    assert!(err, "{v}");
    assert!(
        v["error"].as_str().is_some_and(|e| e.contains("8192")),
        "{v}"
    );
    // The open document is untouched and clean; closing needs no discard.
    let (_, err) = r.call("close", json!({"document": id}));
    assert!(!err);
    assert_eq!(std::fs::read(&xlsx).expect("read"), before);
    let lines = r.audit.lines();
    let eval_lines: Vec<&Value> = lines
        .iter()
        .filter(|l| l["tool"] == "evaluate_formula")
        .collect();
    assert_eq!(eval_lines[0]["effect"], "read");
    assert_eq!(eval_lines[0]["cell"], "Z1");
    // The formula text is not logged, only its size.
    assert!(!lines.iter().any(|l| l.to_string().contains("1+2")));
    assert_eq!(eval_lines[0]["formula_chars"], 4);
}

#[test]
fn evaluate_formula_cannot_see_hidden_content_unless_included() {
    let scratch = Scratch::new("eval-hidden");
    let path = scratch.root().join("h.xlsx");
    std::fs::write(&path, hidden_rows_xlsx()).expect("write");
    let args = |f: &str| json!({"document": "doc1", "sheet": 0, "cell": "E5", "formula": f});
    let mut plain = rig(&scratch, false, false, false);
    plain.open(&path);
    for f in ["A2", "B2", "C1", "CONCAT(A1,A2,B2,C1,C3)"] {
        let (v, err) = plain.call("evaluate_formula", args(f));
        assert!(!err, "{v}");
        assert!(!v.to_string().contains("SECRET"), "{f}: {v}");
        assert_eq!(v["hidden_content_excluded"], true);
    }
    let (v, _) = plain.call("evaluate_formula", args("CONCAT(A1,A3)"));
    assert_eq!(v["result"]["value"], "a1a3");
    // A cell in a hidden row cannot be read back through its own formula either.
    let (v, err) = plain.call(
        "evaluate_formula",
        json!({"document": "doc1", "sheet": 0, "cell": "A2"}),
    );
    assert!(err, "{v}");

    let mut with = rig(&scratch, false, false, true);
    with.open(&path);
    let (v, _) = with.call("evaluate_formula", args("A2"));
    assert_eq!(v["result"]["value"], "SECRET-A2");
}

#[test]
fn evaluate_formula_cannot_reach_hidden_sheets() {
    let scratch = Scratch::new("eval-hidden-sheet");
    let path = scratch.put("xlsx/hidden-sheet.xlsx");
    let mut all = rig(&scratch, false, false, true);
    let aid = all.open(&path);
    let (o, _) = all.call("get_outline", json!({"document": aid}));
    let names: Vec<String> = o["sheets"]
        .as_array()
        .expect("sheets")
        .iter()
        .filter(|s| s["state"].as_str().is_some_and(|st| st != "visible"))
        .map(|s| s["name"].as_str().expect("name").to_owned())
        .collect();
    assert!(!names.is_empty(), "{o}");
    let mut r = rig(&scratch, false, false, false);
    let id = r.open(&path);
    for name in names {
        let f = format!("'{name}'!A1");
        let (v, err) = r.call(
            "evaluate_formula",
            json!({"document": id, "sheet": 0, "cell": "H9", "formula": f}),
        );
        assert!(!err, "{v}");
        assert_ne!(
            v["result"]["type"], "text",
            "hidden sheet {name} leaked: {v}"
        );
    }
}

fn zip_part(path: &Path, name: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path).expect("open zip");
    let mut zip = zip::ZipArchive::new(file).expect("zip");
    let mut part = zip.by_name(name).ok()?;
    let mut data = Vec::new();
    part.read_to_end(&mut data).expect("read part");
    Some(data)
}

#[test]
fn macro_enabled_files_and_templates_open_and_keep_their_vba_project() {
    let scratch = Scratch::new("macros");
    put_as(&scratch, "docx/template-macro.dotm", "m.dotm");
    put_as(&scratch, "docx/template-basic.dotx", "t.dotx");
    put_as(&scratch, "xlsx/template-macro.xltm", "m.xltm");
    put_as(&scratch, "xlsx/template-basic.xltx", "t.xltx");
    let mut r = rig(&scratch, true, false, false);
    for (t, o) in [("m.dotm", "m.docm"), ("m.xltm", "m.xlsm")] {
        let (v, err) = r.call("new_from_template", json!({"path": t, "output": o}));
        assert!(!err, "{v}");
    }
    let cases = [
        ("m.dotm", "word/vbaProject.bin", true),
        ("m.docm", "word/vbaProject.bin", true),
        ("m.xltm", "xl/vbaProject.bin", false),
        ("m.xlsm", "xl/vbaProject.bin", false),
    ];
    for (name, part, docx) in cases {
        let path = scratch.root().join(name);
        let vba = zip_part(&path, part).unwrap_or_else(|| panic!("{name} has no {part}"));
        let id = r.open(&path);
        let (v, err) = r.call("get_outline", json!({"document": id}));
        assert!(!err, "{name}: {v}");
        // An edit and a save keep the VBA project byte for byte; nothing is run.
        let command = if docx {
            json!([{"command": "docx_insert_text", "para": 0, "offset": 0, "text": "X"}])
        } else {
            json!([{"command": "xlsx_set_cell_value", "sheet": 0, "row": 50, "col": 1,
                    "value": {"type": "number", "value": 7.0}}])
        };
        let (v, err) = r.call(
            "apply_commands",
            json!({"document": id, "commands": command}),
        );
        assert!(!err, "{name}: {v}");
        let saved = format!("saved-{name}");
        let (v, err) = r.call("save_as", json!({"document": id, "path": saved}));
        assert!(!err, "{name}: {v}");
        let out = scratch.root().join(&saved);
        assert_eq!(zip_part(&out, part), Some(vba), "{name}: VBA part changed");
        // Saving under another extension would mislabel the package.
        let other = if docx { "other.docx" } else { "other.xlsx" };
        let (v, err) = r.call("save_as", json!({"document": id, "path": other}));
        assert!(err && v["status"] == "invalid", "{name}: {v}");
        assert!(!scratch.root().join(other).exists());
    }
    // Plain templates open too, and keep their extension on save.
    for name in ["t.dotx", "t.xltx"] {
        let id = r.open(&scratch.root().join(name));
        let (v, err) = r.call("get_outline", json!({"document": id}));
        assert!(!err, "{name}: {v}");
    }
    // Other extensions still do not.
    let junk = scratch.root().join("x.dotm.txt");
    std::fs::write(&junk, b"x").expect("write");
    let (v, err) = r.call("open_document", json!({"path": s(&junk)}));
    assert!(err && v["status"] == "invalid", "{v}");
}

#[test]
fn slow_tools_report_a_timeout_and_a_late_export_writes_nothing() {
    let scratch = Scratch::new("deadline");
    let docx = scratch.put("docx/styles-headings.docx");
    let xlsx = scratch.put("xlsx/formulas-basic.xlsx");
    let mut config_rig = rig(&scratch, true, false, false);
    config_rig.server = Server::new(
        Config {
            roots: vec![scratch.root()],
            allow_edit: true,
            deadline: std::time::Duration::ZERO,
            ..Config::default()
        },
        Box::new(config_rig.audit.clone()),
    )
    .expect("server");
    let r = &mut config_rig;
    let d = r.open(&docx);
    let x = r.open(&xlsx);
    let (v, err) = r.call("layout_summary", json!({"document": d}));
    assert!(err && v["status"] == "timeout", "{v}");
    assert!(v["error"].as_str().is_some_and(|e| e.contains("cancelled")));
    let (v, err) = r.call("export_pdf", json!({"document": d, "path": "late.pdf"}));
    assert!(err && v["status"] == "timeout", "{v}");
    let (v, err) = r.call(
        "evaluate_formula",
        json!({"document": x, "sheet": 0, "cell": "Z1", "formula": "1+2"}),
    );
    assert!(err && v["status"] == "timeout", "{v}");
    // The abandoned export saw the cancellation flag and never created the file.
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(!scratch.root().join("late.pdf").exists());
    assert!(no_temp_files(&scratch.root()));
    let lines = r.audit.lines();
    assert!(lines.iter().any(|l| l["status"] == "timeout"));

    // The same documents still work with a sensible deadline.
    let mut ok = rig(&scratch, true, false, false);
    let d = ok.open(&docx);
    let (v, err) = ok.call("layout_summary", json!({"document": d}));
    assert!(!err, "{v}");
}

#[test]
fn the_formula_engine_is_cached_per_workbook_and_rebuilt_when_things_change() {
    let scratch = Scratch::new("engine");
    let xlsx = scratch.put("xlsx/formulas-basic.xlsx");
    let mut r = rig(&scratch, true, true, false);
    let id = r.open(&xlsx);
    let eval = |r: &mut Rig, formula: &str| {
        let (v, err) = r.call(
            "evaluate_formula",
            json!({"document": id, "sheet": 0, "cell": "Z1", "formula": formula}),
        );
        assert!(!err, "{v}");
        v
    };
    let engines = |r: &Rig| -> Vec<String> {
        r.audit
            .lines()
            .iter()
            .filter(|l| l["tool"] == "evaluate_formula")
            .map(|l| l["engine"].as_str().expect("engine").to_owned())
            .collect()
    };
    assert_eq!(eval(&mut r, "A1+1")["result"]["type"], "number");
    eval(&mut r, "A1+2");
    // A scratch formula never lingers in the cached engine.
    let (v, _) = r.call(
        "evaluate_formula",
        json!({"document": id, "sheet": 0, "cell": "Z1"}),
    );
    assert_eq!(v["status"], "invalid", "Z1 has no formula of its own: {v}");
    assert_eq!(engines(&r), ["built", "reused", "reused"]);

    // An edit changes the document: the engine is rebuilt and sees the new value.
    let edit = json!([{"command": "xlsx_set_cell_value", "sheet": 0, "row": 1, "col": 1,
                       "value": {"type": "number", "value": 40.0}}]);
    let (v, err) = r.call("apply_commands", json!({"document": id, "commands": edit}));
    assert!(!err, "{v}");
    assert_eq!(
        eval(&mut r, "A1+2")["result"],
        json!({"type": "number", "value": 42.0})
    );
    eval(&mut r, "A1+2");
    // The file changing on disk invalidates it too.
    let mut bytes = std::fs::read(&xlsx).expect("read");
    bytes.push(0);
    std::fs::write(&xlsx, bytes).expect("write");
    eval(&mut r, "A1+2");
    assert_eq!(
        engines(&r),
        ["built", "reused", "reused", "built", "reused", "built"]
    );
}

#[test]
fn resizing_is_an_edit_tier_command() {
    let scratch = Scratch::new("resize");
    let xlsx = scratch.put("xlsx/formulas-basic.xlsx");
    let resize = json!([
        {"command": "set_column_width", "sheet": 0, "first": 1, "last": 2, "width": 30.0},
        {"command": "set_row_height", "sheet": 0, "first": 1, "last": 1, "points": 45.0}
    ]);
    // Read-only: refused, with the edit flag named.
    let mut r = rig(&scratch, false, false, false);
    let id = r.open(&xlsx);
    let why = r.refused(
        "apply_commands",
        json!({"document": id, "commands": resize}),
    );
    assert!(why.contains("--allow-edit"), "{why}");
    let (v, err) = r.call(
        "apply_commands",
        json!({"document": id, "dry_run": true, "commands": resize}),
    );
    assert!(!err, "{v}");
    assert_eq!(v["results"][0]["effect"], "edit");
    assert_eq!(v["results"][0]["permitted"], false);

    // The edit tier is enough; the destructive flag is not needed.
    let mut r = rig(&scratch, true, false, false);
    let id = r.open(&xlsx);
    let (v, err) = r.call(
        "apply_commands",
        json!({"document": id, "commands": resize}),
    );
    assert!(!err, "{v}");
    assert_eq!(v["results"][0]["effect"], "edit");
    assert_eq!(v["results"][1]["effect"], "edit");
    let out = scratch.root().join("resized.xlsx");
    let (v, err) = r.call("save_as", json!({"document": id, "path": s(&out)}));
    assert!(!err, "{v}");
    let saved = open365_commands::Target::open_path(&out).expect("reopens");
    let sheet = saved.xlsx().and_then(|w| w.sheet(0)).expect("sheet");
    assert!(
        sheet
            .columns()
            .iter()
            .any(|c| (c.min, c.max, c.width) == (1, 2, Some(30.0)))
    );
    assert_eq!(sheet.rows()[0].height(), Some(45.0));

    // Out-of-range sizes are errors.
    let bad =
        json!([{"command": "set_column_width", "sheet": 0, "first": 1, "last": 1, "width": 300.0}]);
    let (v, err) = r.call("apply_commands", json!({"document": id, "commands": bad}));
    assert!(err, "{v}");
}

#[test]
fn apply_commands_cannot_smuggle_an_export() {
    let scratch = Scratch::new("smuggle");
    let docx = scratch.put("docx/plain-paragraphs.docx");
    let mut r = rig(&scratch, true, true, false);
    let id = r.open(&docx);
    let why = r.refused(
        "apply_commands",
        json!({"document": id, "commands": [{"command": "export_pdf", "path": "../outside/x.pdf", "overwrite": true}]}),
    );
    assert!(why.contains("export_pdf tool"), "{why}");
    assert!(!scratch.outside().join("x.pdf").exists());
}
