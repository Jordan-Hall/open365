//! Footnotes in unequal-width columns (`fixtures/docx/layout-cols-fn-unequal`, measured with
//! Word 16): notes are set at each column's own width and flow down the first column; a note
//! splits by line and its tail is broken again at the next column's width; the note area is the
//! smallest that holds all six notes. Word's 600 dpi PDF shows 10 lines of 150 pt in the first
//! column and 11 lines of 282 pt in the second, the tail of note 3 first.

use std::path::PathBuf;

use open365_docx::Document;
use open365_layout::{LayoutOptions, StoryKind, layout_document};
use open365_opc::Limits;
use open365_text::FontLibrary;

#[test]
fn notes_split_across_unequal_columns_and_wrap_at_each_width() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/docx/layout-cols-fn-unequal.docx");
    let bytes = std::fs::read(path).expect("fixture");
    let doc = Document::open(&bytes, &Limits::default()).expect("open");
    let layout = layout_document(&doc, &FontLibrary::bundled(), LayoutOptions::default());
    let notes: Vec<_> = layout
        .all_lines()
        .filter(|(_, l)| l.story == StoryKind::Footnotes)
        .map(|(_, l)| l)
        .collect();
    let first: Vec<_> = notes.iter().filter(|l| l.left < 100.0).collect();
    let second: Vec<_> = notes.iter().filter(|l| l.left > 200.0).collect();
    assert_eq!(first.len(), 10);
    assert_eq!(second.len(), 11);
    assert!(first.iter().all(|l| l.last_x <= 72.0 + 150.0 + 0.5));
    // The tail of note 3 heads the second column, broken again at 282 pt.
    assert!(
        second[0]
            .text
            .contains("several lines of a narrow column and shows")
    );
    assert!(second.iter().any(|l| l.last_x > 72.0 + 150.0 + 100.0));
}
