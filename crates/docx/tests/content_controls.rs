//! Content controls (`w:sdt`): their paragraphs and text are addressable and editable, and a
//! save keeps the control (`layout-sdt`).

use open365_docx::{Block, DocPos, Document};
use open365_opc::Limits;

fn open() -> Document {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/docx/layout-sdt.docx"
    ))
    .expect("fixture");
    Document::open(&bytes, &Limits::default()).expect("opens")
}

#[test]
fn control_content_is_part_of_paragraph_text_and_addressing() {
    let doc = open();
    // An inline control's text is the paragraph's text.
    assert_eq!(
        doc.paragraph_text(0).as_deref(),
        Some("Alpha beta gamma delta epsilon zeta")
    );
    let block = (0..doc.paragraph_count())
        .find(|i| doc.paragraph_text(*i).as_deref() == Some("Block control paragraph"))
        .expect("paragraph inside a block-level control");
    assert!(doc.body().blocks.iter().any(|b| matches!(b, Block::Sdt(_))));
    assert!(doc.paragraph(block).is_some());
}

#[test]
fn editing_inside_controls_keeps_them() {
    let mut doc = open();
    let block = (0..doc.paragraph_count())
        .find(|i| doc.paragraph_text(*i).as_deref() == Some("Block control paragraph"))
        .expect("block control paragraph");
    doc.insert_text(block, 5, "Z")
        .expect("insert in the control");
    // Typing into an inline control's text.
    doc.insert_text(0, 8, "Q")
        .expect("insert in inline control");
    doc.split_paragraph(DocPos::new(block, 3))
        .expect("Enter in the control");
    let bytes = doc.save().expect("save");
    let back = Document::open(&bytes, &Limits::default()).expect("reopens");
    assert_eq!(
        back.paragraph_text(0).as_deref(),
        Some("Alpha beQta gamma delta epsilon zeta")
    );
    assert_eq!(back.paragraph_text(block).as_deref(), Some("Blo"));
    assert_eq!(
        back.paragraph_text(block + 1).as_deref(),
        Some("ckZ control paragraph")
    );
    assert!(
        back.body()
            .blocks
            .iter()
            .any(|b| matches!(b, Block::Sdt(_)))
    );
}
