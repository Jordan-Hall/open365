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
    let v: Value = serde_json::from_slice(&plain.stdout).expect("json");
    assert!(v["parts"].as_array().is_some_and(|p| !p.is_empty()));
    assert!(v["relationships"].is_array());
    assert!(v["content_types"]["defaults"].is_array());
    assert!(v.get("hidden").is_none());

    let with = run(&[s("xlsx"), s("inspect"), s("--hidden"), s("--json"), &f]);
    let v: Value = serde_json::from_slice(&with.stdout).expect("json");
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
    let o = run(&[s("apply"), &cmds, &input, s("-o"), &out]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(audit_lines(&o).len(), 3);
    let check = run(&[s("roundtrip"), s("--check"), &out]);
    assert!(check.status.success(), "{}", stdout(&check));
}

#[test]
fn destructive_commands_are_refused_without_the_flag() {
    let scratch = Scratch::new("destructive");
    let cmds = scratch.write("c.json", &format!("[{INSERT},{DELETE}]"));
    let out = scratch.path("out.docx");
    let input = fixture("docx/plain-paragraphs.docx");
    let o = run(&[s("apply"), &cmds, &input, s("-o"), &out]);
    assert_eq!(o.status.code(), Some(3), "{}", stderr(&o));
    assert!(stdout(&o).contains("destructive"), "{}", stdout(&o));
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
