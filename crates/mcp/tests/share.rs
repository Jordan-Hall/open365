//! The Stage 0 sharing tools over MCP (ADR-013): scopes, confirmation, fenced paths, the
//! inspector and the sanitiser. The document is generated here; the author is "Author A".

mod common;

use std::io::Write;

use common::{Rig, Scratch, s};
use open365_mcp::Scope;
use serde_json::json;

fn commented_docx() -> Vec<u8> {
    let types = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/comments.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/></Types>"#;
    let root = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/></Relationships>"#;
    let doc = r#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:commentRangeStart w:id="0"/><w:r><w:t>Hello</w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r></w:p></w:body></w:document>"#;
    let comments = r#"<?xml version="1.0"?><w:comments xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:comment w:id="0" w:author="Author A"><w:p><w:r><w:t>Note.</w:t></w:r></w:p></w:comment></w:comments>"#;
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in [
        ("[Content_Types].xml", types),
        ("_rels/.rels", root),
        ("word/document.xml", doc),
        ("word/_rels/document.xml.rels", rels),
        ("word/comments.xml", comments),
    ] {
        w.start_file(name, zip::write::SimpleFileOptions::default())
            .expect("start");
        w.write_all(data.as_bytes()).expect("write");
    }
    w.finish().expect("finish").into_inner()
}

fn setup(
    test: &str,
    scopes: &[Scope],
    destructive: bool,
) -> (Scratch, Rig, String, std::path::PathBuf) {
    let scratch = Scratch::new(test);
    let file = scratch.root().join("memo.docx");
    std::fs::write(&file, commented_docx()).expect("write");
    let mut rig = Rig::new(&scratch, scopes, destructive);
    let id = rig.open(&file);
    (scratch, rig, id, file)
}

#[test]
fn inspect_is_a_read_and_reports_the_comment_and_author() {
    let (_scratch, mut r, id, _) = setup("share-inspect", &[], false);
    let out = r.ok("share_inspect", json!({"document": id}));
    assert_eq!(out["format"], "docx");
    assert_eq!(out["authors"], json!(["Author A"]));
    assert!(
        out["items"]
            .as_array()
            .is_some_and(|a| a.iter().any(|i| i["kind"] == "comments"))
    );
    assert!(out["notice"].as_str().is_some());
}

#[test]
fn copies_need_the_export_scope_and_stay_inside_the_roots() {
    let (scratch, mut r, id, file) = setup("share-copy-scope", &[], false);
    let to = scratch.root().join("copy.docx");
    let why = r.refused("share_send_copy", json!({"document": id, "path": s(&to)}));
    assert!(why.contains("scope export"), "{why}");
    assert!(!to.exists());

    let (scratch, mut r, id, file2) = setup("share-copy-ok", &[Scope::Export], false);
    let to = scratch.root().join("copy.docx");
    r.ok("share_send_copy", json!({"document": id, "path": s(&to)}));
    assert!(to.exists());
    // Not over an existing file without overwrite, and never over the document itself.
    let why = r.refused("share_send_copy", json!({"document": id, "path": s(&to)}));
    assert!(why.contains("overwrite"), "{why}");
    let why = r.refused(
        "share_send_copy",
        json!({"document": id, "path": s(&file2), "overwrite": true}),
    );
    assert!(
        why.contains("confirm") || why.contains("destructive") || why.contains("operator"),
        "{why}"
    );
    let outside = std::env::temp_dir().join("open365-share-outside.docx");
    r.refused(
        "share_send_copy",
        json!({"document": id, "path": s(&outside)}),
    );
    assert!(!outside.exists());
    let _ = file;
}

#[test]
fn the_document_itself_is_never_replaced_even_when_confirmed() {
    let (_scratch, mut r, id, file) = setup("share-self", &[Scope::Export], true);
    let before = std::fs::read(&file).expect("read");
    let (v, err) = r.call(
        "share_send_copy",
        json!({"document": id, "path": s(&file), "overwrite": true}),
    );
    assert!(err, "{v}");
    assert_eq!(std::fs::read(&file).expect("read"), before);
}

#[test]
fn sanitise_writes_a_clean_copy_and_leaves_the_original_and_the_open_document() {
    let (scratch, mut r, id, file) = setup("share-sanitize", &[Scope::Export], false);
    let before = std::fs::read(&file).expect("read");
    let to = scratch.root().join("clean.docx");
    let out = r.ok(
        "share_sanitize_copy",
        json!({"document": id, "path": s(&to), "remove": ["comments", "personal_info"]}),
    );
    assert_eq!(out["report"]["removed"]["comments"], 1);
    assert_eq!(std::fs::read(&file).expect("read"), before);
    let clean = r.open(&to);
    let report = r.ok("share_inspect", json!({"document": clean}));
    assert_eq!(report["items"], json!([]));
    // The original document still shows the comment.
    let again = r.ok("share_inspect", json!({"document": id}));
    assert!(again["items"].as_array().is_some_and(|a| !a.is_empty()));
    let why = r.refused(
        "share_sanitize_copy",
        json!({"document": id, "path": s(&scratch.root().join("x.docx")), "remove": ["nope"]}),
    );
    assert!(why.contains("remove"), "{why}");
}

#[test]
fn os_sheet_is_egress_and_needs_the_scope_and_a_confirmation() {
    let (_scratch, mut r, id, file) = setup("share-os-none", &[Scope::Export], true);
    let why = r.refused("share_os_sheet", json!({"document": id, "path": s(&file)}));
    assert!(why.contains("execute-external"), "{why}");

    let (_scratch, mut r, id, file) = setup(
        "share-os-unconfirmed",
        &[Scope::Export, Scope::ExecuteExternal],
        false,
    );
    r.refused("share_os_sheet", json!({"document": id, "path": s(&file)}));

    let (_scratch, mut r, id, file) = setup(
        "share-os-ok",
        &[Scope::Export, Scope::ExecuteExternal],
        true,
    );
    let out = r.ok("share_os_sheet", json!({"document": id, "path": s(&file)}));
    assert_eq!(out["performed"], false);
    assert_eq!(out["file_name"], "memo.docx");
}

#[test]
fn share_commands_are_not_accepted_by_apply_commands() {
    let (_scratch, mut r, id, _) = setup("share-apply", &[Scope::Edit, Scope::Export], true);
    for command in [
        json!({"command": "share_inspect"}),
        json!({"command": "share_send_copy", "path": "x.docx"}),
    ] {
        let why = r.refused(
            "apply_commands",
            json!({"document": id, "commands": [command]}),
        );
        assert!(why.contains("share_*"), "{why}");
    }
}
