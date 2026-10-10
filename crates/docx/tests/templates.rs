//! Word templates (.dotx) and macro-enabled files (.docm, .dotm): kind detection,
#![allow(clippy::unwrap_used)]
//! instantiation and byte-identical preservation of the VBA project. Macros never run.

mod common;

use open365_docx::{Document, DocumentKind, HygieneAction};
use open365_opc::{ContentTypes, Limits, Package, PartName};

use common::{docx_with_body, repo_root};

const CT_DOC: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
const CT_DOTX: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml";
const CT_DOCM: &str = "application/vnd.ms-word.document.macroEnabled.main+xml";
const CT_DOTM: &str = "application/vnd.ms-word.template.macroEnabledTemplate.main+xml";

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(repo_root().join("fixtures").join("docx").join(name)).expect("fixture exists")
}

fn main_content_type(bytes: &[u8]) -> String {
    let package = Package::read(bytes, &Limits::default()).expect("package");
    let ct = ContentTypes::parse(package.part("[Content_Types].xml").unwrap().data()).unwrap();
    ct.content_type_of(&PartName::new("word/document.xml").unwrap())
        .unwrap()
        .to_owned()
}

fn part(bytes: &[u8], name: &str) -> Option<Vec<u8>> {
    Package::read(bytes, &Limits::default())
        .unwrap()
        .part(name)
        .map(|p| p.data().to_vec())
}

/// A copy of `bytes` with the main content type replaced.
fn with_main_type(bytes: &[u8], to: &str) -> Vec<u8> {
    let mut package = Package::read(bytes, &Limits::default()).unwrap();
    let ct = String::from_utf8(package.part("[Content_Types].xml").unwrap().data().to_vec())
        .unwrap()
        .replace(&main_content_type(bytes), to);
    package.set_part(
        PartName::new("[Content_Types].xml").unwrap(),
        ct.into_bytes(),
    );
    package.write().unwrap()
}

fn open(bytes: &[u8]) -> Document {
    Document::open(bytes, &Limits::default()).expect("opens")
}

#[test]
fn plain_document_has_document_kind() {
    let doc = open(&docx_with_body("<w:p/>"));
    assert_eq!(doc.kind(), DocumentKind::Document);
    assert!(!doc.has_vba_project());
    let mut doc = doc;
    assert!(
        !doc.instantiate_template().unwrap(),
        "not a template: no-op"
    );
    assert!(doc.hygiene_log().is_empty());
}

#[test]
fn kind_is_read_from_every_main_content_type() {
    let base = docx_with_body("<w:p/>");
    for (ct, kind) in [
        (CT_DOC, DocumentKind::Document),
        (CT_DOTX, DocumentKind::Template),
        (CT_DOCM, DocumentKind::MacroEnabledDocument),
        (CT_DOTM, DocumentKind::MacroEnabledTemplate),
    ] {
        let doc = open(&with_main_type(&base, ct));
        assert_eq!(doc.kind(), kind, "{ct}");
        assert_eq!(kind.content_type(), ct);
        assert_eq!(DocumentKind::from_content_type(ct), Some(kind));
        assert!(doc.dropped().is_empty());
    }
    assert_eq!(DocumentKind::from_content_type("text/plain"), None);
}

#[test]
fn dotx_fixture_opens_and_instantiates_to_a_plain_document() {
    let bytes = fixture("template-basic.dotx");
    let mut doc = open(&bytes);
    assert_eq!(doc.kind(), DocumentKind::Template);
    assert!(doc.kind().is_template() && !doc.kind().is_macro_enabled());
    assert!(!doc.has_vba_project());
    assert!(doc.dropped().is_empty());

    // Unedited round trip keeps the content type and every part.
    let saved = open(&bytes).save().unwrap();
    assert_eq!(main_content_type(&saved), CT_DOTX);

    assert!(doc.instantiate_template().unwrap());
    assert_eq!(doc.kind(), DocumentKind::Document);
    assert!(matches!(
        doc.hygiene_log().last(),
        Some(HygieneAction::ChangedContentType { from, to, .. }) if from == CT_DOTX && to == CT_DOC
    ));
    assert!(
        !doc.instantiate_template().unwrap(),
        "second call is a no-op"
    );
    assert_eq!(doc.hygiene_log().len(), 1);

    let out = doc.save().unwrap();
    assert_eq!(main_content_type(&out), CT_DOC);
    let reopened = open(&out);
    assert_eq!(reopened.kind(), DocumentKind::Document);
    assert_eq!(
        reopened.paragraph_text(0).as_deref(),
        Some("Template heading")
    );

    // Only [Content_Types].xml differs.
    let before = Package::read(&bytes, &Limits::default()).unwrap();
    let after = Package::read(&out, &Limits::default()).unwrap();
    for p in before.parts() {
        let q = after.part(p.name().as_str()).expect("part kept");
        if p.name().as_str() != "[Content_Types].xml" {
            assert_eq!(p.data(), q.data(), "{}", p.name());
        }
    }
    write_for_oracle("template-basic-instantiated.docx", &out);
}

#[test]
fn dotm_fixture_keeps_the_vba_project_byte_identical() {
    let bytes = fixture("template-macro.dotm");
    let vba = part(&bytes, "word/vbaProject.bin").expect("fixture has a VBA project");
    let mut doc = open(&bytes);
    assert_eq!(doc.kind(), DocumentKind::MacroEnabledTemplate);
    assert!(doc.kind().is_macro_enabled());
    assert!(doc.has_vba_project());
    assert!(doc.dropped().is_empty());

    // Unedited save: content type and VBA part preserved.
    let saved = open(&bytes).save().unwrap();
    assert_eq!(main_content_type(&saved), CT_DOTM);
    assert_eq!(part(&saved, "word/vbaProject.bin").unwrap(), vba);

    // Edited save: still preserved.
    doc.insert_text(0, 0, "Edited: ").unwrap();
    let edited = doc.save().unwrap();
    assert_eq!(main_content_type(&edited), CT_DOTM);
    assert_eq!(part(&edited, "word/vbaProject.bin").unwrap(), vba);
    assert_eq!(
        part(&edited, "word/_rels/vbaProject.bin.rels"),
        part(&bytes, "word/_rels/vbaProject.bin.rels")
    );

    // Instantiated: macro-enabled document, VBA untouched.
    let mut doc = open(&bytes);
    assert!(doc.instantiate_template().unwrap());
    assert_eq!(doc.kind(), DocumentKind::MacroEnabledDocument);
    assert!(matches!(
        doc.hygiene_log().last(),
        Some(HygieneAction::ChangedContentType { from, to, .. }) if from == CT_DOTM && to == CT_DOCM
    ));
    let out = doc.save().unwrap();
    assert_eq!(main_content_type(&out), CT_DOCM);
    assert_eq!(part(&out, "word/vbaProject.bin").unwrap(), vba);
    assert_eq!(open(&out).kind(), DocumentKind::MacroEnabledDocument);
    write_for_oracle("template-macro-instantiated.docm", &out);
    write_for_oracle("template-macro-roundtrip.dotm", &edited);
}

#[test]
fn template_without_vba_part_instantiates_to_plain_even_for_dotm() {
    let base = docx_with_body("<w:p/>");
    let mut doc = open(&with_main_type(&base, CT_DOTM));
    assert!(!doc.has_vba_project());
    assert!(doc.instantiate_template().unwrap());
    assert_eq!(doc.kind(), DocumentKind::Document);
}

#[test]
fn macro_enabled_document_is_not_a_template() {
    let mut doc = open(&with_main_type(&docx_with_body("<w:p/>"), CT_DOCM));
    assert!(!doc.kind().is_template());
    assert!(!doc.instantiate_template().unwrap());
    assert_eq!(doc.kind(), DocumentKind::MacroEnabledDocument);
}

/// Saves `bytes` under `target/docx-template-out` so `office-oracle` can open them.
fn write_for_oracle(name: &str, bytes: &[u8]) {
    let dir = repo_root().join("target").join("docx-template-out");
    std::fs::create_dir_all(&dir).expect("output dir");
    std::fs::write(dir.join(name), bytes).expect("write output");
}
