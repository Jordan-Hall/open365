//! TOC generation beyond the Word oracle set: stability, missing styles, simple fields.

mod common;

use open365_docx::{CivilDateTime, DocPos, Document, FieldContext, PageInfo, PageProvider};
use open365_opc::Limits;

use common::{W, build, repo_root, root_rels};

/// Every paragraph on page 1.
struct Flat;

impl PageProvider for Flat {
    fn page_at(&self, _pos: DocPos) -> Option<PageInfo> {
        Some(PageInfo {
            page: 1,
            number: 1,
            label: None,
            section: 0,
        })
    }
    fn total_pages(&self) -> Option<u32> {
        Some(1)
    }
    fn section_pages(&self, _section: usize) -> Option<u32> {
        Some(1)
    }
}

fn clock() -> CivilDateTime {
    CivilDateTime::parse_utc("2026-01-01T00:00:00").expect("clock")
}

fn part(bytes: &[u8], name: &str) -> String {
    let package = open365_opc::Package::read(bytes, &Limits::default()).expect("package");
    String::from_utf8(package.part(name).expect("part").data().to_vec()).expect("utf8")
}

#[test]
fn updating_twice_changes_nothing() {
    let fixture = std::fs::read(repo_root().join("fixtures/docx/fields-toc.docx")).expect("read");
    let ctx = FieldContext::new(clock()).with_pages(&Flat);
    let mut doc = Document::open(&fixture, &Limits::default()).expect("open");
    doc.update_fields(&ctx).expect("update");
    let first = doc.save().expect("save");
    let mut again = Document::open(&first, &Limits::default()).expect("reopen");
    again.update_fields(&ctx).expect("update again");
    let second = again.save().expect("save again");
    assert_eq!(
        part(&first, "word/document.xml"),
        part(&second, "word/document.xml")
    );
    // Every link and page reference lands on a bookmark that exists.
    let xml = part(&second, "word/document.xml");
    let mut checked = 0;
    for marker in ["w:anchor=\"", "PAGEREF "] {
        let mut rest = xml.as_str();
        while let Some(i) = rest.find(marker) {
            rest = &rest[i + marker.len()..];
            let name: String = rest
                .chars()
                .take_while(|c| !matches!(c, '"' | ' '))
                .collect();
            assert!(
                xml.contains(&format!("w:name=\"{name}\"")),
                "dangling reference to {name}"
            );
            checked += 1;
        }
    }
    assert!(checked > 100, "only {checked} references");
}

fn synthetic(toc: &str, headings: &[(&str, &str)]) -> Vec<u8> {
    let mut body = String::new();
    for (style, text) in headings {
        body.push_str(&format!(
            "<w:p><w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"
        ));
    }
    body.push_str(&format!(
        "<w:p><w:fldSimple w:instr=\"{toc}\"><w:r><w:t>old</w:t></w:r></w:fldSimple></w:p>"
    ));
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document xmlns:w=\"{W}\"><w:body>{body}</w:body></w:document>"
    );
    let styles = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:styles xmlns:w=\"{W}\"><w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/></w:style><w:style w:type=\"paragraph\" w:styleId=\"Heading1\"><w:name w:val=\"heading 1\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:outlineLvl w:val=\"0\"/></w:pPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Heading4\"><w:name w:val=\"heading 4\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:outlineLvl w:val=\"3\"/></w:pPr></w:style></w:styles>"
    );
    let rels = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/></Relationships>";
    build(
        &document,
        "word/document.xml",
        &root_rels("word/document.xml", ""),
        &[
            ("word/styles.xml", styles.as_bytes()),
            ("word/_rels/document.xml.rels", rels.as_bytes()),
        ],
    )
}

#[test]
fn a_simple_field_becomes_styled_paragraphs_and_missing_styles_are_added() {
    let bytes = synthetic(
        " TOC \\o &quot;1-9&quot; \\h ",
        &[("Heading1", "One"), ("Heading4", "Deep")],
    );
    let mut doc = Document::open(&bytes, &Limits::default()).expect("open");
    assert!(doc.styles().is_some_and(|s| !s.has_style("TOC4")));
    let ctx = FieldContext::new(clock()).with_pages(&Flat);
    let report = doc.update_fields(&ctx).expect("update");
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    let saved = doc.save().expect("save");
    let reopened = Document::open(&saved, &Limits::default()).expect("reopen");
    let styles = reopened.styles().expect("styles");
    for id in ["TOC1", "TOC4", "Hyperlink", "TOCHeading"] {
        assert!(styles.has_style(id), "{id} missing");
    }
    let paragraphs = reopened.body().paragraphs();
    let ids: Vec<Option<&str>> = paragraphs.iter().map(|p| p.style_id()).collect();
    assert_eq!(
        ids,
        vec![
            Some("Heading1"),
            Some("Heading4"),
            Some("TOC1"),
            Some("TOC4"),
            None
        ]
    );
    assert_eq!(paragraphs[2].text(), "One\t1");
    assert_eq!(paragraphs[3].text(), "Deep\t1");
    // Word's definition of TOC 4 (measured): indented 660 twips, 100 after.
    let styles_xml = part(&saved, "word/styles.xml");
    assert!(styles_xml.contains("w:styleId=\"TOC4\""));
    assert!(styles_xml.contains("<w:ind w:left=\"660\"/>"));
}

#[test]
fn no_entries_says_so_in_one_paragraph() {
    let bytes = synthetic(" TOC \\o &quot;1-3&quot; ", &[("Heading4", "Deep")]);
    let mut doc = Document::open(&bytes, &Limits::default()).expect("open");
    let ctx = FieldContext::new(clock()).with_pages(&Flat);
    doc.update_fields(&ctx).expect("update");
    let paragraphs = doc.body().paragraphs();
    assert_eq!(paragraphs.len(), 2);
    assert_eq!(paragraphs[1].text(), "No table of contents entries found.");
}

#[test]
fn without_a_page_provider_a_toc_with_page_numbers_is_left_alone() {
    let bytes = synthetic(" TOC \\o &quot;1-9&quot; ", &[("Heading1", "One")]);
    let mut doc = Document::open(&bytes, &Limits::default()).expect("open");
    let ctx = FieldContext::new(clock());
    let report = doc.update_fields(&ctx).expect("update");
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(doc.body().paragraphs().len(), 2);
}
