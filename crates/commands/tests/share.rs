//! Stage 0 sharing commands (ADR-013): the hidden-content inspector, the sanitiser, the copy
//! and PDF commands, the share-sheet request and the effect classes. Every fixture is
//! generated here; authors are "Author A" and "Author B", never a real person.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use open365_commands::{
    Command, CommandError, EffectClass, HiddenKind, Target, inspect_bytes, remove_share_temp,
    sanitize_bytes,
};
use serde_json::Value;

fn zip_of(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in parts {
        w.start_file(*name, zip::write::SimpleFileOptions::default())
            .expect("start");
        w.write_all(data.as_bytes()).expect("write");
    }
    w.finish().expect("finish").into_inner()
}

fn part(bytes: &[u8], name: &str) -> Option<String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    let mut file = zip.by_name(name).ok()?;
    let mut text = String::new();
    file.read_to_string(&mut text).expect("utf8");
    Some(text)
}

fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("open365-sharetest-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

const REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const OD: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn root_rels(target: &str) -> String {
    format!(
        r#"<?xml version="1.0"?><Relationships xmlns="{REL_NS}"><Relationship Id="rId1" Type="{OD}/officeDocument" Target="{target}"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/><Relationship Id="rId3" Type="{OD}/extended-properties" Target="docProps/app.xml"/></Relationships>"#
    )
}

const CORE: &str = r#"<?xml version="1.0"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Quarterly Draft</dc:title><dc:subject>Budget</dc:subject><dc:creator>Author A</dc:creator><cp:keywords>internal</cp:keywords><cp:lastModifiedBy>Author B</cp:lastModifiedBy></cp:coreProperties>"#;
const APP: &str = r#"<?xml version="1.0"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>Test</Application><Company>Example Org</Company><Manager>Author C</Manager></Properties>"#;
const CUSTOM: &str = r#"<?xml version="1.0"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="Project"><lpwstr>Falcon</lpwstr></property></Properties>"#;

fn docx_with_hidden() -> Vec<u8> {
    let types = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/comments.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/><Override PartName="/docProps/custom.xml" ContentType="application/vnd.openxmlformats-officedocument.custom-properties+xml"/></Types>"#;
    let doc_rels = format!(
        r#"<?xml version="1.0"?><Relationships xmlns="{REL_NS}"><Relationship Id="rId1" Type="{OD}/comments" Target="comments.xml"/></Relationships>"#
    );
    let doc = r#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Visible text.</w:t></w:r></w:p><w:p><w:ins w:id="1" w:author="Author A" w:date="2026-01-01T00:00:00Z"><w:r><w:t>Inserted text.</w:t></w:r></w:ins><w:del w:id="2" w:author="Author B" w:date="2026-01-01T00:00:00Z"><w:r><w:delText>Deleted text.</w:delText></w:r></w:del></w:p><w:p><w:commentRangeStart w:id="0"/><w:r><w:t>Commented text.</w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r></w:p><w:p><w:r><w:rPr><w:vanish/></w:rPr><w:t>Secret words.</w:t></w:r><w:r><w:rPr><w:b/><w:vanish w:val="0"/></w:rPr><w:t>Shown bold.</w:t></w:r></w:p></w:body></w:document>"#;
    let comments = r#"<?xml version="1.0"?><w:comments xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:comment w:id="0" w:author="Author A" w:initials="AA"><w:p><w:r><w:t>Check this number.</w:t></w:r></w:p></w:comment></w:comments>"#;
    zip_of(&[
        ("[Content_Types].xml", types),
        ("_rels/.rels", &root_rels("word/document.xml")),
        ("word/document.xml", doc),
        ("word/_rels/document.xml.rels", &doc_rels),
        ("word/comments.xml", comments),
        ("docProps/core.xml", CORE),
        ("docProps/app.xml", APP),
        ("docProps/custom.xml", CUSTOM),
    ])
}

fn xlsx_with_hidden() -> Vec<u8> {
    let types = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="vml" ContentType="application/vnd.openxmlformats-officedocument.vmlDrawing"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/comments1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml"/></Types>"#;
    let book = r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Open" sheetId="1" r:id="rId1"/><sheet name="Backstage" sheetId="2" state="hidden" r:id="rId2"/></sheets></workbook>"#;
    let book_rels = format!(
        r#"<?xml version="1.0"?><Relationships xmlns="{REL_NS}"><Relationship Id="rId1" Type="{OD}/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="{OD}/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rId3" Type="{OD}/revisionHeaders" Target="revisions/revisionHeaders.xml"/></Relationships>"#
    );
    let cell = |r: &str, v: &str| format!(r#"<c r="{r}" t="inlineStr"><is><t>{v}</t></is></c>"#);
    let sheet1 = format!(
        r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><cols><col min="3" max="3" width="9" hidden="1" customWidth="1"/></cols><sheetData><row r="1">{}{}</row><row r="2" hidden="1">{}</row></sheetData><legacyDrawing r:id="rId2"/></worksheet>"#,
        cell("A1", "a1"),
        cell("B1", "b1"),
        cell("A2", "a2"),
    );
    let sheet2 = format!(
        r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1">{}</row></sheetData></worksheet>"#,
        cell("A1", "h1")
    );
    let sheet_rels = format!(
        r#"<?xml version="1.0"?><Relationships xmlns="{REL_NS}"><Relationship Id="rId1" Type="{OD}/comments" Target="../comments1.xml"/><Relationship Id="rId2" Type="{OD}/vmlDrawing" Target="../drawings/vmlDrawing1.vml"/></Relationships>"#
    );
    let comments = r#"<?xml version="1.0"?><comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><authors><author>Author A</author></authors><commentList><comment ref="A1" authorId="0"><text><r><t>Why this value?</t></r></text></comment></commentList></comments>"#;
    let vml = r#"<xml xmlns:x="urn:schemas-microsoft-com:office:excel"><v:shape id="s1" xmlns:v="urn:schemas-microsoft-com:vml"><x:ClientData ObjectType="Note"><x:Row>0</x:Row><x:Column>0</x:Column></x:ClientData></v:shape></xml>"#;
    let revisions = r#"<?xml version="1.0"?><headers xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"/>"#;
    zip_of(&[
        ("[Content_Types].xml", types),
        ("_rels/.rels", &root_rels("xl/workbook.xml")),
        ("xl/workbook.xml", book),
        ("xl/_rels/workbook.xml.rels", &book_rels),
        ("xl/worksheets/sheet1.xml", &sheet1),
        ("xl/worksheets/sheet2.xml", &sheet2),
        ("xl/worksheets/_rels/sheet1.xml.rels", &sheet_rels),
        ("xl/comments1.xml", comments),
        ("xl/drawings/vmlDrawing1.vml", vml),
        ("xl/revisions/revisionHeaders.xml", revisions),
        ("docProps/core.xml", CORE),
        ("docProps/app.xml", APP),
    ])
}

fn kinds(bytes: &[u8]) -> Vec<HiddenKind> {
    let report = inspect_bytes(bytes).expect("inspect");
    HiddenKind::ALL
        .into_iter()
        .filter(|k| report.has(*k))
        .collect()
}

#[test]
fn the_inspector_finds_every_hidden_content_type_in_a_document() {
    let report = inspect_bytes(&docx_with_hidden()).expect("inspect");
    assert_eq!(report.format, "docx");
    let find = |kind: HiddenKind, part: &str| {
        report
            .items
            .iter()
            .find(|i| i.kind == kind && i.part == part)
            .unwrap_or_else(|| panic!("no {kind:?} in {part}: {report:?}"))
    };
    assert_eq!(find(HiddenKind::Comments, "word/comments.xml").count, 1);
    let tracked = find(HiddenKind::TrackedChanges, "word/document.xml");
    assert_eq!(tracked.count, 2, "{tracked:?}");
    assert!(tracked.detail.contains("w:ins") && tracked.detail.contains("w:del"));
    // The run with vanish switched off is not hidden.
    assert_eq!(find(HiddenKind::HiddenText, "word/document.xml").count, 1);
    let props = find(HiddenKind::Properties, "docProps/core.xml");
    assert!(props.detail.contains("Quarterly Draft"), "{props:?}");
    assert_eq!(find(HiddenKind::Properties, "docProps/custom.xml").count, 1);
    let people = find(HiddenKind::PersonalInfo, "docProps/core.xml");
    assert!(people.detail.contains("Author A") && people.detail.contains("Author B"));
    let app = find(HiddenKind::PersonalInfo, "docProps/app.xml");
    assert!(app.detail.contains("Example Org") && app.detail.contains("Author C"));
    assert_eq!(report.authors, ["Author A", "Author B"]);
    assert!(report.items.iter().all(|i| i.removable));
}

#[test]
fn the_inspector_finds_hidden_content_in_a_workbook() {
    let report = inspect_bytes(&xlsx_with_hidden()).expect("inspect");
    assert_eq!(report.format, "xlsx");
    let of = |k: HiddenKind| -> Vec<_> { report.items.iter().filter(|i| i.kind == k).collect() };
    assert_eq!(of(HiddenKind::Comments).len(), 1);
    assert_eq!(of(HiddenKind::Comments)[0].part, "xl/comments1.xml");
    assert_eq!(of(HiddenKind::TrackedChanges).len(), 1);
    let hidden = of(HiddenKind::HiddenText);
    assert_eq!(
        hidden.len(),
        2,
        "hidden sheet, and rows plus columns: {hidden:?}"
    );
    assert!(hidden.iter().all(|i| !i.removable));
    assert!(hidden.iter().any(|i| i.detail.contains("Backstage")));
    assert!(hidden.iter().any(|i| i.detail.contains("1 hidden row")));
    assert!(!of(HiddenKind::Properties).is_empty());
    assert!(!of(HiddenKind::PersonalInfo).is_empty());
    assert_eq!(report.authors, ["Author A"]);
}

#[test]
fn a_clean_document_has_no_findings() {
    let clean = zip_of(&[
        ("[Content_Types].xml", r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#),
        ("_rels/.rels", &root_rels("word/document.xml").replace("<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/><Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/>", "")),
        ("word/document.xml", r#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Plain.</w:t></w:r></w:p></w:body></w:document>"#),
    ]);
    assert!(inspect_bytes(&clean).expect("inspect").clean());
    assert!(inspect_bytes(b"not a zip").is_err());
}

#[test]
fn sanitising_a_document_removes_everything_chosen_and_keeps_the_text() {
    let original = docx_with_hidden();
    let out = sanitize_bytes(&original, &HiddenKind::ALL).expect("sanitize");
    assert!(
        inspect_bytes(&out.bytes).expect("inspect").clean(),
        "{:?}",
        inspect_bytes(&out.bytes)
    );
    assert_eq!(out.removed[&HiddenKind::Comments], 1);
    assert_eq!(out.removed[&HiddenKind::TrackedChanges], 2);
    assert_eq!(out.removed[&HiddenKind::HiddenText], 1);
    let doc = part(&out.bytes, "word/document.xml").expect("document");
    for gone in [
        "Deleted text",
        "Secret words",
        "commentRange",
        "commentReference",
        "w:ins",
        "w:del",
    ] {
        assert!(!doc.contains(gone), "{gone} still in {doc}");
    }
    for kept in [
        "Visible text.",
        "Inserted text.",
        "Commented text.",
        "Shown bold.",
    ] {
        assert!(doc.contains(kept), "{kept} lost from {doc}");
    }
    assert!(part(&out.bytes, "word/comments.xml").is_none());
    assert!(part(&out.bytes, "docProps/custom.xml").is_none());
    let types = part(&out.bytes, "[Content_Types].xml").expect("types");
    assert!(
        !types.contains("comments.xml") && !types.contains("custom.xml"),
        "{types}"
    );
    let rels = part(&out.bytes, "word/_rels/document.xml.rels").expect("rels");
    assert!(!rels.contains("comments"), "{rels}");
    let core = part(&out.bytes, "docProps/core.xml").expect("core");
    for gone in ["Author A", "Author B", "Quarterly Draft", "internal"] {
        assert!(!core.contains(gone), "{gone} in {core}");
    }
    assert!(
        !part(&out.bytes, "docProps/app.xml")
            .expect("app")
            .contains("Example Org")
    );
    // The result is still a document the editor opens.
    let target = Target::open_docx(&out.bytes).expect("reopen");
    assert!(target.docx().is_some());
}

#[test]
fn sanitising_only_some_kinds_leaves_the_others() {
    let out = sanitize_bytes(&docx_with_hidden(), &[HiddenKind::HiddenText]).expect("sanitize");
    assert_eq!(
        kinds(&out.bytes),
        [
            HiddenKind::Comments,
            HiddenKind::TrackedChanges,
            HiddenKind::Properties,
            HiddenKind::PersonalInfo,
        ]
    );
    // Personal info alone renames the authors that stay and drops the author fields.
    let out = sanitize_bytes(&docx_with_hidden(), &[HiddenKind::PersonalInfo]).expect("sanitize");
    let doc = part(&out.bytes, "word/document.xml").expect("document");
    assert!(
        !doc.contains("Author A")
            && !doc.contains("Author B")
            && doc.contains("w:author=\"Author\"")
    );
    let comments = part(&out.bytes, "word/comments.xml").expect("comments stay");
    assert!(!comments.contains("Author A") && comments.contains("Check this number."));
    assert!(sanitize_bytes(&docx_with_hidden(), &[]).is_err());
}

#[test]
fn sanitising_a_workbook_removes_comments_history_and_names_but_never_hidden_cells() {
    let original = xlsx_with_hidden();
    let refused = sanitize_bytes(&original, &HiddenKind::ALL).expect_err("hidden sheet");
    assert!(refused.to_string().contains("hidden sheets"), "{refused}");
    let removable = [
        HiddenKind::Comments,
        HiddenKind::TrackedChanges,
        HiddenKind::Properties,
        HiddenKind::PersonalInfo,
    ];
    let out = sanitize_bytes(&original, &removable).expect("sanitize");
    assert_eq!(kinds(&out.bytes), [HiddenKind::HiddenText]);
    assert!(part(&out.bytes, "xl/comments1.xml").is_none());
    assert!(part(&out.bytes, "xl/drawings/vmlDrawing1.vml").is_none());
    assert!(part(&out.bytes, "xl/revisions/revisionHeaders.xml").is_none());
    let sheet = part(&out.bytes, "xl/worksheets/sheet1.xml").expect("sheet");
    assert!(
        !sheet.contains("legacyDrawing") && sheet.contains("a1"),
        "{sheet}"
    );
    let rels = part(&out.bytes, "xl/worksheets/_rels/sheet1.xml.rels").expect("rels");
    assert!(
        !rels.contains("comments") && !rels.contains("vml"),
        "{rels}"
    );
    let book_rels = part(&out.bytes, "xl/_rels/workbook.xml.rels").expect("book rels");
    assert!(!book_rels.contains("revision") && book_rels.contains("sheet2.xml"));
    let target = Target::open_xlsx(&out.bytes).expect("reopen");
    assert!(target.xlsx().is_some());
    // A workbook with nothing hidden accepts every kind.
    let plain = sanitize_bytes(&out.bytes, &removable).expect("again");
    assert_eq!(kinds(&plain.bytes), [HiddenKind::HiddenText]);
}

#[test]
fn a_vml_drawing_with_other_controls_is_kept() {
    // The same workbook, with a button as well as a note in the VML.
    let vml = r#"<xml xmlns:x="urn:schemas-microsoft-com:office:excel"><x:ClientData ObjectType="Note"/><x:ClientData ObjectType="Button"/></xml>"#;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(xlsx_with_hidden())).expect("zip");
    let mut parts: Vec<(String, String)> = Vec::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).expect("entry");
        let name = f.name().expect("name").into_owned();
        let mut text = String::new();
        f.read_to_string(&mut text).expect("text");
        parts.push((
            name.clone(),
            if name.ends_with(".vml") {
                vml.to_owned()
            } else {
                text
            },
        ));
    }
    let refs: Vec<(&str, &str)> = parts
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let out = sanitize_bytes(&zip_of(&refs), &[HiddenKind::Comments]).expect("sanitize");
    assert!(part(&out.bytes, "xl/drawings/vmlDrawing1.vml").is_some());
    assert!(part(&out.bytes, "xl/comments1.xml").is_none());
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write fixture");
    path
}

#[test]
fn sanitise_copy_writes_a_clean_copy_and_leaves_the_original_alone() {
    let dir = scratch("sanitize");
    let original = docx_with_hidden();
    let path = write(&dir, "report.docx", &original);
    let mut target = Target::open_path(&path).expect("open");
    let revision = target.revision();
    let copy = dir.join("report-clean.docx");
    let out = target
        .apply(Command::ShareSanitizeCopy {
            path: Some(copy.clone()),
            overwrite: false,
            remove: HiddenKind::ALL.to_vec(),
        })
        .expect("sanitize copy");
    assert_eq!(out.effect, EffectClass::Export);
    assert_eq!(out.saved_to.as_deref(), Some(copy.as_path()));
    assert!(out.notes.iter().any(|n| n.contains("comments")));
    let report = out.report.expect("report");
    assert_eq!(report["removed"]["comments"], 1);
    let written = std::fs::read(&copy).expect("copy");
    assert!(inspect_bytes(&written).expect("inspect").clean());
    // The original file, and the open document, are unchanged.
    assert_eq!(std::fs::read(&path).expect("original"), original);
    assert_eq!(target.revision(), revision);
    assert!(!target.is_dirty());
    assert_eq!(target.path(), Some(path.as_path()));
    assert!(
        inspect_bytes(&target.snapshot_bytes().expect("bytes"))
            .expect("inspect")
            .has(HiddenKind::Comments)
    );
    // Without a path the copy goes to a temporary folder and is named after the document.
    let temp = target
        .apply(Command::ShareSanitizeCopy {
            path: None,
            overwrite: false,
            remove: vec![HiddenKind::Comments],
        })
        .expect("temp copy");
    let temp_path = temp.saved_to.expect("temp path");
    assert_eq!(
        temp_path.file_name().and_then(|n| n.to_str()),
        Some("report-clean.docx")
    );
    assert!(temp_path.starts_with(std::env::temp_dir()) && temp_path.exists());
    assert!(remove_share_temp(&temp_path));
    assert!(!temp_path.exists() && !temp_path.parent().is_some_and(Path::exists));
    // An empty choice, and the document's own path, are refused.
    assert!(matches!(
        target.apply(Command::ShareSanitizeCopy {
            path: Some(dir.join("x.docx")),
            overwrite: false,
            remove: vec![]
        }),
        Err(CommandError::Share(_))
    ));
    let own = target.apply(Command::ShareSanitizeCopy {
        path: Some(path.clone()),
        overwrite: true,
        remove: vec![HiddenKind::Comments],
    });
    assert!(matches!(own, Err(CommandError::Share(m)) if m.contains("the document itself")));
    assert_eq!(std::fs::read(&path).expect("original"), original);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn send_copy_is_byte_identical_to_a_save_and_never_replaces_silently() {
    let dir = scratch("copy");
    let path = write(&dir, "plan.docx", &docx_with_hidden());
    let mut target = Target::open_path(&path).expect("open");
    let saved = dir.join("saved.docx");
    target
        .apply(Command::Save {
            path: saved.clone(),
        })
        .expect("save");
    let target_path = target.path().map(Path::to_path_buf);
    let copy = dir.join("attachment.docx");
    let out = target
        .apply(Command::ShareSendCopy {
            path: Some(copy.clone()),
            overwrite: false,
        })
        .expect("copy");
    assert_eq!(out.effect, EffectClass::Export);
    assert_eq!(
        std::fs::read(&copy).expect("copy"),
        std::fs::read(&saved).expect("saved")
    );
    assert_eq!(out.bytes_written, std::fs::read(&copy).expect("copy").len());
    // A copy does not move the document to the new path.
    assert_eq!(target.path().map(Path::to_path_buf), target_path);
    // Replacing a file needs overwrite.
    let again = target.apply(Command::ShareSendCopy {
        path: Some(copy.clone()),
        overwrite: false,
    });
    assert!(matches!(again, Err(CommandError::Exists { .. })));
    target
        .apply(Command::ShareSendCopy {
            path: Some(copy.clone()),
            overwrite: true,
        })
        .expect("overwrite");
    // The document's own file is never a target, with or without overwrite.
    for overwrite in [false, true] {
        let own = target.apply(Command::ShareSendCopy {
            path: Some(saved.clone()),
            overwrite,
        });
        assert!(matches!(own, Err(CommandError::Share(_))), "{own:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn send_copy_carries_unsaved_edits_and_the_workbook_extension() {
    let dir = scratch("edits");
    let path = write(&dir, "book.xlsx", &xlsx_with_hidden());
    let mut target = Target::open_path(&path).expect("open");
    target
        .apply(Command::xlsx_enter(0, 1, 1, "typed"))
        .expect("edit");
    assert!(target.is_dirty());
    let out = target
        .apply(Command::ShareSendCopy {
            path: None,
            overwrite: false,
        })
        .expect("temp copy");
    let temp = out.saved_to.expect("path");
    assert_eq!(temp.file_name().and_then(|n| n.to_str()), Some("book.xlsx"));
    let reopened = Target::open_path(&temp).expect("reopen");
    let wb = reopened.xlsx().expect("xlsx");
    assert_eq!(
        wb.cell(0, 1, 1)
            .and_then(|c| c.value.as_text().map(str::to_owned))
            .as_deref(),
        Some("typed")
    );
    assert!(target.is_dirty(), "sending a copy does not save");
    assert!(remove_share_temp(&temp));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn restricted_writes_refuse_a_temporary_copy_and_a_path_outside_the_roots() {
    let dir = scratch("roots");
    let inside = dir.join("inside");
    let outside = dir.join("outside");
    std::fs::create_dir_all(&inside).expect("inside");
    std::fs::create_dir_all(&outside).expect("outside");
    let path = write(&inside, "a.docx", &docx_with_hidden());
    let mut target = Target::open_path(&path).expect("open");
    target.restrict_writes_to(&[inside.canonicalize().expect("canonical")]);
    assert!(matches!(
        target.apply(Command::ShareSendCopy {
            path: None,
            overwrite: false
        }),
        Err(CommandError::Share(_))
    ));
    for overwrite in [false, true] {
        let to = outside.join("b.docx");
        let denied = target.apply(Command::ShareSendCopy {
            path: Some(to.clone()),
            overwrite,
        });
        assert!(matches!(denied, Err(CommandError::Io { .. })), "{denied:?}");
        assert!(!to.exists());
    }
    target
        .apply(Command::ShareSendCopy {
            path: Some(inside.join("ok.docx")),
            overwrite: false,
        })
        .expect("inside");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn send_pdf_goes_through_the_pdf_export() {
    let dir = scratch("pdf");
    let path = write(&dir, "memo.docx", &docx_with_hidden());
    let mut target = Target::open_path(&path).expect("open");
    let pdf = dir.join("memo.pdf");
    let out = target
        .apply(Command::ShareSendPdf {
            path: Some(pdf.clone()),
            overwrite: false,
        })
        .expect("pdf");
    assert_eq!(out.effect, EffectClass::Export);
    assert!(out.pages >= 1);
    let bytes = std::fs::read(&pdf).expect("pdf");
    assert!(bytes.starts_with(b"%PDF"));
    // The same bytes and page count as export_pdf.
    let direct = dir.join("direct.pdf");
    let export = target
        .apply(Command::ExportPdf {
            path: direct.clone(),
            overwrite: false,
        })
        .expect("export");
    assert_eq!(export.pages, out.pages);
    assert_eq!(std::fs::read(&direct).expect("direct").len(), bytes.len());
    assert!(matches!(
        target.apply(Command::ShareSendPdf {
            path: Some(pdf.clone()),
            overwrite: false
        }),
        Err(CommandError::Exists { .. })
    ));
    // A temporary PDF is named after the document.
    let temp = target
        .apply(Command::ShareSendPdf {
            path: None,
            overwrite: false,
        })
        .expect("temp pdf")
        .saved_to
        .expect("path");
    assert_eq!(temp.file_name().and_then(|n| n.to_str()), Some("memo.pdf"));
    assert!(remove_share_temp(&temp));
    // A workbook has no PDF export.
    let mut book = Target::open_xlsx(&xlsx_with_hidden()).expect("xlsx");
    assert!(matches!(
        book.apply(Command::ShareSendPdf {
            path: Some(dir.join("b.pdf")),
            overwrite: false
        }),
        Err(CommandError::WrongTarget { .. })
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn inspect_reports_through_the_command_and_changes_nothing() {
    let mut target = Target::open_docx(&docx_with_hidden()).expect("open");
    let before = target.audit_log().0.len();
    let out = target.apply(Command::ShareInspect).expect("inspect");
    assert_eq!(out.effect, EffectClass::Read);
    let report = out.report.expect("report");
    assert_eq!(report["format"], "docx");
    assert!(report["items"].as_array().is_some_and(|a| a.len() >= 6));
    assert!(out.notes.iter().any(|n| n.starts_with("hidden_text")));
    assert!(out.saved_to.is_none() && !target.is_dirty() && !target.can_undo());
    assert_eq!(target.audit_log().0.len(), before + 1);
}

#[test]
fn os_sheet_is_only_a_request() {
    let dir = scratch("sheet");
    let file = write(&dir, "send.docx", b"x");
    let mut target = Target::open_docx(&docx_with_hidden()).expect("open");
    let out = target
        .apply(Command::ShareOsSheet { path: file.clone() })
        .expect("request");
    assert_eq!(out.effect, EffectClass::Egress);
    let request: Value = out.report.expect("request");
    assert_eq!(request["schema"], "open365.os-share-request/1");
    assert_eq!(request["performed"], false);
    assert_eq!(request["file_name"], "send.docx");
    assert!(
        request["mime"]
            .as_str()
            .is_some_and(|m| m.contains("wordprocessingml"))
    );
    assert_eq!(request["temporary"], false);
    assert!(
        target
            .apply(Command::ShareOsSheet {
                path: dir.join("missing.docx")
            })
            .is_err()
    );
    assert!(
        target
            .apply(Command::ShareOsSheet { path: dir.clone() })
            .is_err()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_share_command_has_its_effect_class() {
    let dir = scratch("effects");
    let own = write(&dir, "own.docx", b"x");
    let other = dir.join("other.docx");
    let missing_own = dir.join("gone.docx");
    let copy = |path: Option<PathBuf>, overwrite| Command::ShareSendCopy { path, overwrite };
    let pdf = |path: Option<PathBuf>, overwrite| Command::ShareSendPdf { path, overwrite };
    let sanitize = |path: Option<PathBuf>, overwrite| Command::ShareSanitizeCopy {
        path,
        overwrite,
        remove: vec![HiddenKind::Comments],
    };
    let current = Some(own.as_path());
    for make in [
        &copy as &dyn Fn(Option<PathBuf>, bool) -> Command,
        &pdf,
        &sanitize,
    ] {
        // A new file, a temporary file, and a replaced file are export-class.
        assert_eq!(
            make(Some(other.clone()), false).effect(current),
            EffectClass::Export
        );
        assert_eq!(
            make(Some(other.clone()), true).effect(current),
            EffectClass::Export
        );
        assert_eq!(make(None, false).effect(current), EffectClass::Export);
        assert_eq!(make(None, false).effect(None), EffectClass::Export);
        // The document's own file is the overwrite of the original: destructive (and refused
        // when it runs).
        assert_eq!(
            make(Some(own.clone()), true).effect(current),
            EffectClass::Destructive
        );
        assert_eq!(
            make(Some(missing_own.clone()), true).effect(Some(&missing_own)),
            EffectClass::Export
        );
    }
    assert_eq!(Command::ShareInspect.effect(current), EffectClass::Read);
    assert_eq!(
        Command::ShareOsSheet { path: own.clone() }.effect(current),
        EffectClass::Egress
    );
    // Names are the catalogue names, and the parser accepts the documented JSON.
    let json = r#"[{"command":"share_send_copy"},{"command":"share_send_pdf","path":"a.pdf","overwrite":true},{"command":"share_inspect"},{"command":"share_sanitize_copy","remove":["comments","personal_info"]},{"command":"share_os_sheet","path":"a.docx"}]"#;
    let parsed: Vec<Command> = serde_json::from_str(json).expect("parse");
    let names: Vec<&str> = parsed.iter().map(Command::name).collect();
    assert_eq!(
        names,
        [
            "share_send_copy",
            "share_send_pdf",
            "share_inspect",
            "share_sanitize_copy",
            "share_os_sheet"
        ]
    );
    assert!(parsed.iter().all(Command::is_share));
    assert_eq!(parsed.iter().filter(|c| c.writes_file()).count(), 3);
    assert!(serde_json::from_str::<Command>(r#"{"command":"share_sanitize_copy"}"#).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
