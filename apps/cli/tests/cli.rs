//! End-to-end tests of the `open365` binary against the real fixtures. Output goes to a
//! per-test temporary directory.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel)
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(test: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("open365-cli-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let p = self.path(name);
        std::fs::write(&p, text).expect("write scratch file");
        p
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(args: &[&Path]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_open365"))
        .args(args)
        .output()
        .expect("run open365")
}

fn s(text: &str) -> &Path {
    Path::new(text)
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// The `result` of a successful `--json` envelope.
fn result(o: &Output) -> Value {
    let v: Value = serde_json::from_slice(&o.stdout).expect("json envelope");
    assert_eq!(v["schema"], "open365.cli/1", "{v}");
    assert_eq!(v["ok"], true, "{v}");
    v["result"].clone()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn hidden(kind: &str, rel: &str) -> String {
    let f = fixture(rel);
    let o = run(&[s(kind), s("inspect"), s("--hidden"), &f]);
    assert!(o.status.success(), "{rel}: {}", stderr(&o));
    stdout(&o)
}

#[test]
fn docx_inspect_lists_parts_relationships_and_content_types() {
    let f = fixture("docx/plain-paragraphs.docx");
    let o = run(&[s("docx"), s("inspect"), &f]);
    assert!(o.status.success(), "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("word/document.xml"), "{out}");
    assert!(out.contains("RELATIONSHIPS"), "{out}");
    assert!(out.contains("CONTENT TYPES"), "{out}");
    assert!(!out.contains("HIDDEN CONTENT"), "{out}");
}

#[test]
fn inspect_json_is_valid_and_has_hidden_only_on_request() {
    let f = fixture("xlsx/hidden-sheet.xlsx");
    let plain = run(&[s("xlsx"), s("inspect"), s("--json"), &f]);
    let v = result(&plain);
    assert!(v["parts"].as_array().is_some_and(|p| !p.is_empty()));
    assert!(v["relationships"].is_array());
    assert!(v["content_types"]["defaults"].is_array());
    assert!(v.get("hidden").is_none());

    let with = run(&[s("xlsx"), s("inspect"), s("--hidden"), s("--json"), &f]);
    let v = result(&with);
    let cats: Vec<&str> = v["hidden"]
        .as_array()
        .expect("hidden array")
        .iter()
        .filter_map(|h| h["category"].as_str())
        .collect();
    assert_eq!(cats, ["hidden_sheet", "hidden_sheet"]);
}

#[test]
fn inspect_rejects_the_wrong_format() {
    let f = fixture("docx/plain-paragraphs.docx");
    let o = run(&[s("xlsx"), s("inspect"), &f]);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("not a xlsx"), "{}", stderr(&o));
}

#[test]
fn hidden_docx_content_is_listed() {
    assert!(hidden("docx", "docx/hidden-text.docx").contains("hidden_text"));
    assert!(hidden("docx", "docx/comments.docx").contains("comment"));
    assert!(hidden("docx", "docx/tracked-changes.docx").contains("tracked_change"));
    assert!(hidden("docx", "docx/custom-xml-part.docx").contains("custom_xml"));
}

#[test]
fn hidden_xlsx_content_is_listed() {
    assert!(hidden("xlsx", "xlsx/hidden-sheet.xlsx").contains("VeryHiddenSheet"));
    assert!(hidden("xlsx", "xlsx/defined-names.xlsx").contains("defined_name"));
    assert!(hidden("xlsx", "xlsx/external-link-formula.xlsx").contains("external_link"));
    assert!(hidden("xlsx", "xlsx/comments-notes.xlsx").contains("comment"));
}

fn zip_of(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in entries {
        w.start_file(*name, zip::write::SimpleFileOptions::default())
            .expect("start");
        w.write_all(data.as_bytes()).expect("write");
    }
    w.finish().expect("finish").into_inner()
}

#[test]
fn hidden_lists_metadata_unreachable_parts_macros_and_thumbnails() {
    let scratch = Scratch::new("synthetic");
    let types = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="bin" ContentType="application/octet-stream"/><Default Extension="jpeg" ContentType="image/jpeg"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
    let root = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/></Relationships>"#;
    let doc = r#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>x</w:t></w:r></w:p></w:body></w:document>"#;
    let core = r#"<?xml version="1.0"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>Test Author</dc:creator><cp:lastModifiedBy>Test Editor</cp:lastModifiedBy></cp:coreProperties>"#;
    let bytes = zip_of(&[
        ("[Content_Types].xml", types),
        ("_rels/.rels", root),
        ("word/document.xml", doc),
        ("docProps/core.xml", core),
        ("docProps/thumbnail.jpeg", "jpegdata"),
        ("word/vbaProject.bin", "macro"),
        ("orphan/secret.xml", "<a/>"),
    ]);
    let f = scratch.path("synthetic.docx");
    std::fs::write(&f, bytes).expect("write");
    let o = run(&[s("docx"), s("inspect"), s("--hidden"), &f]);
    assert!(o.status.success(), "{}", stderr(&o));
    let out = stdout(&o);
    for needle in [
        "creator: Test Author",
        "lastModifiedBy: Test Editor",
        "unreachable_part",
        "orphan/secret.xml",
        "vba",
        "thumbnail",
    ] {
        assert!(out.contains(needle), "missing {needle}:\n{out}");
    }
}

fn audit_lines(o: &Output) -> Vec<Value> {
    stderr(o)
        .lines()
        .filter(|l| l.starts_with('{'))
        .map(|l| serde_json::from_str(l).expect("audit line is JSON"))
        .collect()
}

const INSERT: &str = r#"{"command":"docx_insert_text","para":0,"offset":5,"text":"XY"}"#;
const DELETE: &str =
    r#"{"command":"docx_delete_range","start":{"para":0,"offset":0},"end":{"para":0,"offset":3}}"#;

#[test]
fn apply_edits_a_docx_and_audits_each_command() {
    let scratch = Scratch::new("apply-docx");
    let cmds = scratch.write("c.json", &format!("[{INSERT}]"));
    let out = scratch.path("out.docx");
    let input = fixture("docx/plain-paragraphs.docx");
    let o = run(&[s("apply"), &cmds, &input, s("-o"), &out]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("docx_insert_text"));
    assert!(stdout(&o).contains("edit"));

    let lines = audit_lines(&o);
    assert_eq!(lines.len(), 2, "{}", stderr(&o));
    assert_eq!(lines[0]["status"], "applied");
    assert_eq!(lines[0]["effect"], "edit");
    assert_eq!(lines[1]["command"], "save");
    assert_eq!(lines[1]["effect"], "export");

    assert!(out.exists());
    let check = run(&[s("roundtrip"), s("--check"), &out]);
    assert!(check.status.success(), "{}", stdout(&check));
    let inspect = run(&[s("docx"), s("inspect"), &out]);
    assert!(inspect.status.success());
}

#[test]
fn apply_edits_an_xlsx_formula() {
    let scratch = Scratch::new("apply-xlsx");
    let cmds = scratch.write(
        "c.json",
        r#"[{"command":"xlsx_set_cell_value","sheet":0,"row":1,"col":1,"value":{"type":"number","value":41}},
            {"command":"xlsx_set_cell_formula","sheet":0,"row":2,"col":1,"formula":"=A1+1"}]"#,
    );
    let out = scratch.path("out.xlsx");
    let input = fixture("xlsx/formulas-basic.xlsx");
    // The fixture's A1 and A2 hold data, so overwriting them needs the operator's flag.
    let o = run(&[s("apply"), &cmds, &input, s("-o"), &out]);
    assert_eq!(o.status.code(), Some(3), "{}", stderr(&o));
    assert!(!out.exists());
    let o = run(&[
        s("apply"),
        &cmds,
        &input,
        s("-o"),
        &out,
        s("--allow-destructive"),
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(audit_lines(&o).len(), 3);
    let check = run(&[s("roundtrip"), s("--check"), &out]);
    assert!(check.status.success(), "{}", stdout(&check));
}

#[test]
fn apply_resizes_columns_and_rows_as_edits() {
    let scratch = Scratch::new("apply-resize");
    let cmds = scratch.write(
        "c.json",
        r#"[{"command":"set_column_width","sheet":0,"first":2,"last":3,"width":24.5},
            {"command":"set_row_height","sheet":0,"first":2,"last":2,"points":40}]"#,
    );
    let out = scratch.path("out.xlsx");
    let input = fixture("xlsx/formulas-basic.xlsx");
    let o = run(&[s("apply"), &cmds, &input, s("-o"), &out]);
    assert!(o.status.success(), "{}", stderr(&o));
    let lines = audit_lines(&o);
    assert_eq!(lines.len(), 3, "{}", stderr(&o));
    for (line, name) in lines.iter().zip(["set_column_width", "set_row_height"]) {
        assert_eq!(line["command"], name);
        assert_eq!(line["effect"], "edit");
        assert_eq!(line["status"], "applied");
    }
    let check = run(&[s("roundtrip"), s("--check"), &out]);
    assert!(check.status.success(), "{}", stdout(&check));
    let bytes = std::fs::read(&out).expect("output");
    let wb = open365_xlsx::Workbook::open(&bytes, &open365_opc::Limits::default()).expect("opens");
    let sheet = wb.sheet(0).expect("sheet");
    let cols = sheet.columns();
    assert!(
        cols.iter()
            .any(|c| c.min == 2 && c.max == 3 && c.width == Some(24.5) && c.custom_width),
        "{cols:?}"
    );
    let row = sheet.rows().iter().find(|r| r.r == 2).expect("row 2");
    assert_eq!(row.height(), Some(40.0));
    assert!(row.custom_height());
}

#[test]
fn apply_refuses_sizes_beyond_the_excel_limits() {
    let scratch = Scratch::new("apply-resize-limits");
    let input = fixture("xlsx/formulas-basic.xlsx");
    for body in [
        r#"[{"command":"set_column_width","sheet":0,"first":1,"last":1,"width":256}]"#,
        r#"[{"command":"set_row_height","sheet":0,"first":1,"last":1,"points":410}]"#,
    ] {
        let cmds = scratch.write("c.json", body);
        let out = scratch.path("out.xlsx");
        let o = run(&[s("apply"), &cmds, &input, s("-o"), &out]);
        assert!(!o.status.success(), "{body}");
        assert!(!out.exists(), "a failed run must not write output");
    }
}

#[test]
fn destructive_commands_are_refused_without_the_flag() {
    let scratch = Scratch::new("destructive");
    let cmds = scratch.write("c.json", &format!("[{INSERT},{DELETE}]"));
    let out = scratch.path("out.docx");
    let input = fixture("docx/plain-paragraphs.docx");
    let o = run(&[s("apply"), &cmds, &input, s("-o"), &out]);
    assert_eq!(o.status.code(), Some(3), "{}", stderr(&o));
    assert!(stderr(&o).contains("destructive"), "{}", stderr(&o));
    assert!(!out.exists(), "refused run must not write output");
    let lines = audit_lines(&o);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["status"], "refused");
    assert_eq!(lines[0]["effect"], "destructive");

    let o = run(&[
        s("apply"),
        s("--allow-destructive"),
        &cmds,
        &input,
        s("-o"),
        &out,
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(out.exists());
}

#[test]
fn dry_run_writes_nothing() {
    let scratch = Scratch::new("dry");
    let cmds = scratch.write("c.json", &format!("[{INSERT}]"));
    let out = scratch.path("out.docx");
    let input = fixture("docx/plain-paragraphs.docx");
    let o = run(&[s("apply"), s("--dry-run"), &cmds, &input, s("-o"), &out]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(!out.exists());
    assert_eq!(audit_lines(&o)[0]["status"], "dry_run_ok");
}

#[test]
fn save_inside_a_command_file_is_refused() {
    let scratch = Scratch::new("save");
    let target = scratch.path("elsewhere.docx");
    let quoted = serde_json::to_string(&target.display().to_string()).expect("json");
    let cmds = scratch.write(
        "c.json",
        &format!(r#"[{{"command":"save","path":{quoted}}}]"#),
    );
    let out = scratch.path("out.docx");
    let input = fixture("docx/plain-paragraphs.docx");
    let o = run(&[s("apply"), &cmds, &input, s("-o"), &out]);
    assert_eq!(o.status.code(), Some(3));
    assert!(!target.exists() && !out.exists());
}

#[test]
fn failing_command_and_bad_json_exit_nonzero_without_output() {
    let scratch = Scratch::new("fail");
    let input = fixture("docx/plain-paragraphs.docx");
    let out = scratch.path("out.docx");
    let bad = scratch.write("bad.json", "not json");
    assert!(
        !run(&[s("apply"), &bad, &input, s("-o"), &out])
            .status
            .success()
    );
    let oob = scratch.write(
        "oob.json",
        r#"[{"command":"docx_insert_text","para":9999,"offset":0,"text":"x"}]"#,
    );
    let o = run(&[s("apply"), &oob, &input, s("-o"), &out]);
    assert!(!o.status.success());
    assert_eq!(audit_lines(&o)[0]["status"], "error");
    assert!(!out.exists());
}

#[test]
fn roundtrip_check_passes_on_every_fixture() {
    let mut files: Vec<PathBuf> = Vec::new();
    for dir in ["docx", "xlsx"] {
        for e in std::fs::read_dir(fixture(dir)).expect("read fixtures") {
            let p = e.expect("entry").path();
            if p.extension().is_some_and(|x| x == dir) {
                files.push(p);
            }
        }
    }
    assert!(files.len() >= 20);
    let mut args: Vec<&Path> = vec![s("roundtrip"), s("--check")];
    args.extend(files.iter().map(PathBuf::as_path));
    let o = run(&args);
    assert!(o.status.success(), "{}", stdout(&o));
    assert_eq!(stdout(&o).matches("PASS").count(), files.len());
}

#[test]
fn roundtrip_check_fails_on_garbage() {
    let scratch = Scratch::new("garbage");
    let f = scratch.write("junk.docx", "this is not a zip");
    let o = run(&[s("roundtrip"), s("--check"), &f]);
    assert!(!o.status.success());
    assert!(stdout(&o).starts_with("FAIL"));
}

#[test]
fn hostile_inputs_fail_cleanly_without_panicking() {
    let scratch = Scratch::new("hostile");
    let good = std::fs::read(fixture("docx/plain-paragraphs.docx")).expect("read fixture");
    let mut cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty.docx", Vec::new()),
        ("garbage.docx", b"PK\x03\x04 not really".to_vec()),
        ("truncated.docx", good[..good.len() / 2].to_vec()),
    ];
    let dtd = br#"<?xml version="1.0"?><!DOCTYPE x [<!ENTITY a "aaaa">]><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#;
    cases.push(("dtd.docx", zip_of_bytes(&[("[Content_Types].xml", dtd)])));
    for (name, bytes) in &cases {
        let f = scratch.path(name);
        std::fs::write(&f, bytes).expect("write");
        for args in [
            vec![s("docx"), s("inspect"), s("--hidden"), &f],
            vec![s("inspect"), &f],
            vec![s("roundtrip"), s("--check"), &f],
        ] {
            let o = run(&args);
            // 101 is the Rust panic exit status.
            assert_ne!(o.status.code(), Some(101), "{name}: {}", stderr(&o));
            assert!(!stderr(&o).contains("panicked"), "{name}: {}", stderr(&o));
        }
        let o = run(&[s("docx"), s("inspect"), &f]);
        assert!(!o.status.success(), "{name} must be rejected");
    }
}

fn zip_of_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in entries {
        w.start_file(*name, zip::write::SimpleFileOptions::default())
            .expect("start");
        w.write_all(data).expect("write");
    }
    w.finish().expect("finish").into_inner()
}

#[test]
fn docx_export_pdf_writes_a_tagged_pdf() {
    let scratch = Scratch::new("export-pdf");
    let out = scratch.path("out.pdf");
    let o = run(&[
        s("docx"),
        s("export-pdf"),
        &fixture("docx/styles-headings.docx"),
        s("-o"),
        &out,
        s("--title"),
        s("Headings"),
        s("--language"),
        s("en-GB"),
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("pages"), "{}", stdout(&o));
    let pdf = std::fs::read(&out).expect("pdf written");
    let text = String::from_utf8_lossy(&pdf);
    assert!(pdf.starts_with(b"%PDF-1.7"));
    assert!(text.contains("/StructTreeRoot") && text.contains("/Marked true"));
    assert!(text.contains("/FontFile2") && text.contains("/ToUnicode"));
    assert!(text.contains("/Lang (en-GB)") && text.contains("/Title (Headings)"));
    assert!(!text.contains("/Author"), "no author unless asked for");
}

#[test]
fn docx_export_pdf_rejects_a_non_docx() {
    let scratch = Scratch::new("export-pdf-bad");
    let bad = scratch.write("bad.docx", "not a zip");
    let out = scratch.path("out.pdf");
    let o = run(&[s("docx"), s("export-pdf"), &bad, s("-o"), &out]);
    assert!(!o.status.success());
    assert!(!out.exists());
}

fn zip_has(path: &Path, name: &str) -> bool {
    let file = std::fs::File::open(path).expect("open zip");
    zip::ZipArchive::new(file)
        .expect("zip")
        .file_names()
        .any(|n| n.is_ok_and(|n| n == name))
}

#[test]
fn docx_layout_json_has_pages_lines_roles_and_cell_boxes() {
    let o = run(&[
        s("docx"),
        s("layout"),
        s("--json"),
        &fixture("docx/styles-headings.docx"),
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    let v = result(&o);
    assert_eq!(v["page_count"], 1);
    assert_eq!(v["truncated"], false);
    let lines = v["pages"][0]["lines"].as_array().expect("lines");
    assert_eq!(lines[0]["role"], "heading");
    assert_eq!(lines[0]["level"], 1);
    assert!(lines.iter().any(|l| l["role"] == "plain"));
    assert!(v["pages"][0]["width"].as_f64().is_some());

    let o = run(&[
        s("docx"),
        s("layout"),
        s("--json"),
        &fixture("docx/layout-table-basic.docx"),
    ]);
    let v = result(&o);
    let cell = &v["pages"][0]["cells"][0];
    for key in ["table", "row", "col", "x", "y", "width", "height"] {
        assert!(cell[key].is_number(), "{key}: {cell}");
    }
}

#[test]
fn docx_layout_caps_pages_and_has_a_text_form() {
    let f = fixture("docx/layout-table-basic.docx");
    let o = run(&[
        s("docx"),
        s("layout"),
        s("--json"),
        s("--max-pages"),
        s("1"),
        &f,
    ]);
    let v = result(&o);
    assert_eq!(v["pages"].as_array().expect("pages").len(), 1);
    assert_eq!(v["truncated"], true);
    assert!(v["page_count"].as_u64().expect("count") > 1);
    let o = run(&[s("docx"), s("layout"), &f]);
    assert!(stdout(&o).starts_with("2 pages"), "{}", stdout(&o));
}

#[test]
fn docx_layout_rejects_a_bad_file() {
    let scratch = Scratch::new("layout-bad");
    let bad = scratch.write("bad.docx", "not a zip");
    let o = run(&[s("docx"), s("layout"), s("--json"), &bad]);
    assert!(!o.status.success());
    let v: Value = serde_json::from_slice(&o.stdout).expect("json envelope");
    assert_eq!(v["ok"], false);
    assert!(v["error"]["message"].is_string());
}

#[test]
fn xlsx_eval_evaluates_without_saving() {
    let f = fixture("xlsx/formulas-basic.xlsx");
    let before = std::fs::read(&f).expect("read");
    let o = run(&[
        s("xlsx"),
        s("eval"),
        &f,
        s("--sheet"),
        s("0"),
        s("--cell"),
        s("Z1"),
        s("--formula"),
        s("=SUM(A1:A3)"),
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    let v: Value = serde_json::from_slice(&o.stdout).expect("json");
    assert_eq!(v["result"]["type"], "number");
    assert_eq!(v["saved"], false);
    assert_eq!(std::fs::read(&f).expect("read"), before);
    // An error value is a result, not a failure; a parse error is a failure.
    let o = run(&[
        s("xlsx"),
        s("eval"),
        &f,
        s("--sheet"),
        s("0"),
        s("--cell"),
        s("Z1"),
        s("--formula"),
        s("1/0"),
    ]);
    let v: Value = serde_json::from_slice(&o.stdout).expect("json");
    assert_eq!(v["result"]["value"], "#DIV/0!");
    for (sheet, cell, formula) in [("0", "Z1", "1+"), ("nope", "A1", "1"), ("0", "??", "1")] {
        let o = run(&[
            s("xlsx"),
            s("eval"),
            &f,
            s("--sheet"),
            s(sheet),
            s("--cell"),
            s(cell),
            s("--formula"),
            s(formula),
        ]);
        assert!(!o.status.success(), "{sheet} {cell} {formula}");
        assert!(o.stdout.is_empty());
    }
}

#[test]
fn xlsx_eval_rejects_a_docx() {
    let f = fixture("docx/plain-paragraphs.docx");
    let o = run(&[
        s("xlsx"),
        s("eval"),
        &f,
        s("--sheet"),
        s("0"),
        s("--cell"),
        s("A1"),
        s("--formula"),
        s("1"),
    ]);
    assert!(!o.status.success());
}

#[test]
fn new_from_template_writes_documents_and_refuses_to_overwrite() {
    let scratch = Scratch::new("template");
    let docx = scratch.path("n.docx");
    let o = run(&[
        s("docx"),
        s("new-from-template"),
        &fixture("docx/template-basic.dotx"),
        s("-o"),
        &docx,
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(zip_has(&docx, "word/document.xml"));
    let first = std::fs::read(&docx).expect("read");
    let again = run(&[
        s("docx"),
        s("new-from-template"),
        &fixture("docx/template-basic.dotx"),
        s("-o"),
        &docx,
    ]);
    assert!(!again.status.success());
    assert!(stderr(&again).contains("--overwrite"), "{}", stderr(&again));
    assert_eq!(std::fs::read(&docx).expect("read"), first);
    let over = run(&[
        s("docx"),
        s("new-from-template"),
        &fixture("docx/template-basic.dotx"),
        s("-o"),
        &docx,
        s("--overwrite"),
    ]);
    // Replacing a file is destructive under the shared policy: --overwrite alone is refused.
    assert_eq!(over.status.code(), Some(3), "{}", stderr(&over));
    assert_eq!(std::fs::read(&docx).expect("read"), first);
    let over = run(&[
        s("docx"),
        s("new-from-template"),
        &fixture("docx/template-basic.dotx"),
        s("-o"),
        &docx,
        s("--overwrite"),
        s("--allow-destructive"),
    ]);
    assert!(over.status.success(), "{}", stderr(&over));

    let macro_out = scratch.path("m.xlsm");
    let o = run(&[
        s("xlsx"),
        s("new-from-template"),
        &fixture("xlsx/template-macro.xltm"),
        s("-o"),
        &macro_out,
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(zip_has(&macro_out, "xl/vbaProject.bin"), "macros are kept");

    let word_macro = scratch.path("m.docm");
    let o = run(&[
        s("docx"),
        s("new-from-template"),
        &fixture("docx/template-macro.dotm"),
        s("-o"),
        &word_macro,
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(zip_has(&word_macro, "word/vbaProject.bin"));
}

#[test]
fn new_from_template_checks_kinds_and_extensions() {
    let scratch = Scratch::new("template-bad");
    let plain = fixture("docx/plain-paragraphs.docx");
    let out = scratch.path("n.docx");
    // Not a template.
    let o = run(&[s("docx"), s("new-from-template"), &plain, s("-o"), &out]);
    assert!(!o.status.success());
    // Wrong subcommand for the template, and wrong output extension.
    let t = fixture("docx/template-basic.dotx");
    let o = run(&[
        s("xlsx"),
        s("new-from-template"),
        &t,
        s("-o"),
        &scratch.path("n.xlsx"),
    ]);
    assert!(!o.status.success());
    let o = run(&[
        s("docx"),
        s("new-from-template"),
        &t,
        s("-o"),
        &scratch.path("n.docm"),
    ]);
    assert!(!o.status.success());
    assert!(!out.exists() && !scratch.path("n.docm").exists());
}

#[test]
fn docx_export_pdf_never_overwrites_without_the_flag() {
    let scratch = Scratch::new("export-pdf-overwrite");
    let out = scratch.write("out.pdf", "keep");
    let input = fixture("docx/styles-headings.docx");
    let o = run(&[s("docx"), s("export-pdf"), &input, s("-o"), &out]);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("--overwrite"), "{}", stderr(&o));
    assert_eq!(std::fs::read(&out).expect("read"), b"keep");
    // Replacing a file is destructive under the shared policy: --overwrite alone is refused.
    let o = run(&[
        s("docx"),
        s("export-pdf"),
        &input,
        s("-o"),
        &out,
        s("--overwrite"),
    ]);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("--allow-destructive"), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("\"status\":\"refused\""),
        "{}",
        stderr(&o)
    );
    assert_eq!(std::fs::read(&out).expect("read"), b"keep");
    let o = run(&[
        s("docx"),
        s("export-pdf"),
        &input,
        s("-o"),
        &out,
        s("--overwrite"),
        s("--allow-destructive"),
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("\"status\":\"applied\""),
        "{}",
        stderr(&o)
    );
    assert!(std::fs::read(&out).expect("pdf").starts_with(b"%PDF-"));
}

#[test]
fn docx_export_pdf_root_fence_refuses_outside_and_writes_the_resolved_path() {
    let scratch = Scratch::new("export-pdf-root");
    let inside = scratch.path("inside");
    let outside = scratch.path("outside");
    std::fs::create_dir_all(&inside).expect("dir");
    std::fs::create_dir_all(&outside).expect("dir");
    let input = fixture("docx/styles-headings.docx");
    let stray = outside.join("out.pdf");
    let o = run(&[
        s("docx"),
        s("export-pdf"),
        &input,
        s("-o"),
        &stray,
        s("--root"),
        &inside,
    ]);
    assert!(!o.status.success());
    assert!(!stray.exists(), "nothing is written outside the root");
    assert!(
        stderr(&o).contains("\"status\":\"refused\""),
        "{}",
        stderr(&o)
    );
    assert!(
        stderr(&o).contains("outside the allowed roots"),
        "{}",
        stderr(&o)
    );

    let good = inside.join("out.pdf");
    let o = run(&[
        s("docx"),
        s("export-pdf"),
        &input,
        s("-o"),
        &good,
        s("--root"),
        &inside,
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    let resolved = inside.canonicalize().expect("canonical").join("out.pdf");
    let audit: serde_json::Value = stderr(&o)
        .lines()
        .find_map(|l| serde_json::from_str(l).ok())
        .expect("one audit line on stderr");
    assert_eq!(audit["status"], "applied");
    assert_eq!(audit["command"], "export_pdf");
    assert_eq!(audit["path"], resolved.display().to_string());
    assert!(good.exists());
}

#[test]
fn docx_export_pdf_writes_pdfa_and_can_drop_bookmarks() {
    let scratch = Scratch::new("export-pdf-pdfa");
    let input = fixture("docx/styles-headings.docx");
    let plain = scratch.path("plain.pdf");
    let o = run(&[s("docx"), s("export-pdf"), &input, s("-o"), &plain]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(String::from_utf8_lossy(&std::fs::read(&plain).expect("pdf")).contains("/Outlines"));

    let a1 = scratch.path("a1.pdf");
    let o = run(&[
        s("docx"),
        s("export-pdf"),
        &input,
        s("-o"),
        &a1,
        s("--pdfa"),
        s("1b"),
        s("--no-bookmarks"),
        s("--lang"),
        s("de-DE"),
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    let bytes = std::fs::read(&a1).expect("pdf");
    let text = String::from_utf8_lossy(&bytes);
    assert!(bytes.starts_with(b"%PDF-1.4"));
    assert!(text.contains("<pdfaid:part>1</pdfaid:part>"));
    assert!(text.contains("/OutputIntents") && text.contains("/Lang (de-DE)"));
    assert!(!text.contains("/Outlines"), "--no-bookmarks");

    let a2 = scratch.path("a2.pdf");
    let o = run(&[
        s("docx"),
        s("export-pdf"),
        &input,
        s("-o"),
        &a2,
        s("--pdfa"),
        s("2b"),
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    let bytes = std::fs::read(&a2).expect("pdf");
    assert!(bytes.starts_with(b"%PDF-1.7"));
    assert!(String::from_utf8_lossy(&bytes).contains("<pdfaid:part>2</pdfaid:part>"));

    let o = run(&[
        s("docx"),
        s("export-pdf"),
        &input,
        s("-o"),
        &scratch.path("bad.pdf"),
        s("--pdfa"),
        s("3b"),
    ]);
    assert!(!o.status.success());
}
