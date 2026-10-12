//! `Document::insert_inline_xml` at a field boundary: the new runs go beside the field, never
//! into its cached result, as Word types text next to a field.

mod common;

use open365_docx::{DocPos, Document};
use open365_opc::Limits;

use common::docx_with_body;

const FIELD: &str = r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> PAGE </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>7</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#;

fn doc() -> Document {
    // "AB", the field (result "7"), "CD".
    let body = format!(r#"<w:p><w:r><w:t>AB</w:t></w:r>{FIELD}<w:r><w:t>CD</w:t></w:r></w:p>"#);
    Document::open(&docx_with_body(&body), &Limits::default()).expect("opens")
}

fn insert(offset: usize) -> Document {
    let mut d = doc();
    d.insert_inline_xml(DocPos { para: 0, offset }, "<w:r><w:t>X</w:t></w:r>")
        .expect("inserts");
    d
}

#[test]
fn text_inserted_right_after_a_field_stays_out_of_its_result() {
    // Offset 3 is the end of the result "7" and the start of "CD".
    let d = insert(3);
    assert_eq!(d.paragraph_text(0).as_deref(), Some("AB7XCD"));
    let f = d.fields();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].result_text, "7");
}

#[test]
fn text_inserted_right_before_a_field_stays_out_of_it() {
    let d = insert(2);
    assert_eq!(d.paragraph_text(0).as_deref(), Some("ABX7CD"));
    assert_eq!(d.fields()[0].result_text, "7");
}

#[test]
fn text_inserted_inside_text_and_at_the_ends_is_unchanged() {
    assert_eq!(insert(1).paragraph_text(0).as_deref(), Some("AXB7CD"));
    assert_eq!(insert(0).paragraph_text(0).as_deref(), Some("XAB7CD"));
    // The end of the paragraph, after the last run.
    assert_eq!(insert(5).paragraph_text(0).as_deref(), Some("AB7CDX"));
}

#[test]
fn text_inserted_after_a_field_that_ends_the_paragraph_follows_it() {
    let body = format!(r#"<w:p><w:r><w:t>AB</w:t></w:r>{FIELD}</w:p>"#);
    let mut d = Document::open(&docx_with_body(&body), &Limits::default()).expect("opens");
    d.insert_inline_xml(DocPos { para: 0, offset: 3 }, "<w:r><w:t>X</w:t></w:r>")
        .expect("inserts");
    assert_eq!(d.paragraph_text(0).as_deref(), Some("AB7X"));
    assert_eq!(d.fields()[0].result_text, "7");
}

#[test]
fn two_fields_in_a_row_take_text_between_them() {
    let body = format!(r#"<w:p>{FIELD}{FIELD}</w:p>"#);
    let mut d = Document::open(&docx_with_body(&body), &Limits::default()).expect("opens");
    d.insert_inline_xml(DocPos { para: 0, offset: 1 }, "<w:r><w:t>-</w:t></w:r>")
        .expect("inserts");
    assert_eq!(d.paragraph_text(0).as_deref(), Some("7-7"));
    let f = d.fields();
    assert_eq!(
        (
            f.len(),
            f[0].result_text.as_str(),
            f[1].result_text.as_str()
        ),
        (2, "7", "7")
    );
}
