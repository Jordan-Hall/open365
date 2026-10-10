//! Hostile and malformed input must be an error, never a panic or stack overflow.

mod common;

use open365_docx::{Document, DocxError, MAX_XML_DEPTH};
use open365_opc::Limits;

use common::{assert_infoset_eq, build, root_rels};

fn open_document_xml(document_xml: &str) -> Result<Document, DocxError> {
    let bytes = build(
        document_xml,
        "word/document.xml",
        &root_rels("word/document.xml", ""),
        &[],
    );
    Document::open(&bytes, &Limits::default())
}

fn wrap(body: &str) -> String {
    format!(
        r#"<?xml version="1.0"?><w:document xmlns:w="{}"><w:body>{body}</w:body></w:document>"#,
        common::W
    )
}

#[test]
fn doctype_is_rejected() {
    let xml = format!(
        r#"<?xml version="1.0"?><!DOCTYPE d [<!ENTITY x "boom">]><w:document xmlns:w="{}"><w:body><w:p><w:r><w:t>&x;</w:t></w:r></w:p></w:body></w:document>"#,
        common::W
    );
    assert!(matches!(
        open_document_xml(&xml),
        Err(DocxError::DoctypeForbidden)
    ));
}

#[test]
fn billion_laughs_is_rejected_before_expansion() {
    let xml = format!(
        r#"<!DOCTYPE lolz [<!ENTITY a "aaaaaaaaaa"><!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;">]><w:document xmlns:w="{}"><w:body><w:p><w:r><w:t>&b;</w:t></w:r></w:p></w:body></w:document>"#,
        common::W
    );
    assert!(matches!(
        open_document_xml(&xml),
        Err(DocxError::DoctypeForbidden)
    ));
}

#[test]
fn undefined_entities_are_an_error() {
    assert!(open_document_xml(&wrap("<w:p><w:r><w:t>&nope;</w:t></w:r></w:p>")).is_err());
}

#[test]
fn deep_nesting_is_an_error_not_a_stack_overflow() {
    let depth = 200_000;
    let mut body = String::new();
    for _ in 0..depth {
        body.push_str("<w:sdt>");
    }
    for _ in 0..depth {
        body.push_str("</w:sdt>");
    }
    assert!(matches!(
        open_document_xml(&wrap(&body)),
        Err(DocxError::DepthExceeded(MAX_XML_DEPTH))
    ));
}

#[test]
fn nesting_just_inside_the_limit_opens_and_saves() {
    // document + body + (limit - 2) nested elements.
    let nested = MAX_XML_DEPTH - 2;
    let mut body = String::new();
    for _ in 0..nested {
        body.push_str("<w:sdt>");
    }
    for _ in 0..nested {
        body.push_str("</w:sdt>");
    }
    let mut doc = open_document_xml(&wrap(&body)).expect("opens at the limit");
    assert!(doc.save().is_ok());
}

#[test]
fn nested_tables_deeper_than_the_limit_are_an_error() {
    let depth = 5_000;
    let mut body = String::new();
    for _ in 0..depth {
        body.push_str("<w:tbl><w:tr><w:tc>");
    }
    for _ in 0..depth {
        body.push_str("</w:tc></w:tr></w:tbl>");
    }
    assert!(matches!(
        open_document_xml(&wrap(&body)),
        Err(DocxError::DepthExceeded(_))
    ));
}

#[test]
fn malformed_and_non_word_documents_are_errors() {
    assert!(open_document_xml("not xml").is_err());
    assert!(open_document_xml("<w:document").is_err());
    assert!(open_document_xml(&wrap("<w:p>")).is_err());
    assert!(open_document_xml(&format!("{}<extra/>", wrap(""))).is_err());
    assert!(matches!(
        open_document_xml("<root/>"),
        Err(DocxError::NotWordprocessing(_))
    ));
    let no_body = format!(r#"<w:document xmlns:w="{}"/>"#, common::W);
    assert!(matches!(
        open_document_xml(&no_body),
        Err(DocxError::NotWordprocessing(_))
    ));
}

#[test]
fn non_utf8_main_part_is_an_error() {
    let bytes = build(
        "x",
        "word/document.xml",
        &root_rels("word/document.xml", ""),
        &[],
    );
    // Replace the main part with UTF-16 bytes (BOM + 'x').
    let mut package = open365_opc::Package::read(&bytes, &Limits::default()).expect("pkg");
    package.set_part(
        open365_opc::PartName::new("word/document.xml").expect("name"),
        vec![0xFF, 0xFE, b'<', 0, b'a', 0, b'/', 0, b'>', 0],
    );
    let bytes = package.write().expect("write");
    assert!(matches!(
        Document::open(&bytes, &Limits::default()),
        Err(DocxError::NotUtf8)
    ));
}

#[test]
fn missing_or_dangling_main_part_is_an_error() {
    let bytes = build(
        "x",
        "word/other.xml",
        &root_rels("word/document.xml", ""),
        &[],
    );
    assert!(matches!(
        Document::open(&bytes, &Limits::default()),
        Err(DocxError::MissingMainPart(_))
    ));
    let bytes = build(
        "x",
        "word/document.xml",
        &root_rels("../../etc/passwd", ""),
        &[],
    );
    assert!(Document::open(&bytes, &Limits::default()).is_err());
}

#[test]
fn utf8_bom_and_prolog_are_kept() {
    let xml = format!(
        "\u{feff}<?xml version=\"1.0\"?>\r\n<!-- c --><w:document xmlns:w=\"{}\"><w:body/></w:document>\r\n",
        common::W
    );
    let doc = open_document_xml(&xml).expect("opens");
    assert_eq!(doc.serialize_main(), xml.as_bytes());
}

#[test]
fn infoset_comparator_detects_differences() {
    assert!(
        assert_infoset_eq(
            b"<a x='1' y='2'><b/>t</a>",
            b"<a y=\"2\" x=\"1\"><b></b>t</a>"
        )
        .is_ok()
    );
    assert!(assert_infoset_eq(b"<a>t</a>", b"<a>u</a>").is_err());
    assert!(assert_infoset_eq(b"<a><b/><c/></a>", b"<a><c/><b/></a>").is_err());
    assert!(assert_infoset_eq(b"<a x='1'/>", b"<a x='2'/>").is_err());
    assert!(assert_infoset_eq(b"<a xmlns='u'/>", b"<a xmlns='v'/>").is_err());
    assert!(assert_infoset_eq(b"<a> t </a>", b"<a>t</a>").is_err());
    assert!(assert_infoset_eq(b"<p:a xmlns:p='u'/>", b"<q:a xmlns:q='u'/>").is_err()); // declarations are kept verbatim
}
