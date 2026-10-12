//! A header row with no body row after it on its page moves to the next page (Word, measured on
//! `layout-table-pages`: the last table's header row does not stay at the bottom of page 17
//! when its first body row, which cannot split, does not fit below it).

mod common;

use common::{Mode, library, root};
use open365_docx::Document;
use open365_layout::{LayoutOptions, layout_document};
use open365_opc::Limits;

/// Story offset of the header row's first cell line and of the heading paragraph above it.
const HEADER_CELL: usize = 17103;
const HEADING: usize = 17063;

#[test]
fn header_row_without_a_body_row_moves_to_the_next_page() {
    let bytes = std::fs::read(root().join("fixtures/docx/layout-table-pages.docx")).expect("read");
    let doc = Document::open(&bytes, &Limits::default()).expect("open");
    let layout = layout_document(&doc, &library(Mode::from_env()), LayoutOptions::default());
    let at = |start: usize| -> Vec<usize> {
        layout
            .all_lines()
            .filter(|(_, l)| l.start == start)
            .map(|(p, _)| p)
            .collect()
    };
    let heading = at(HEADING);
    let header = at(HEADER_CELL);
    assert_eq!(heading.len(), 1);
    assert_eq!(
        header.len(),
        1,
        "the header row is drawn once, pages {header:?}"
    );
    assert_eq!(
        header[0],
        heading[0] + 1,
        "header row starts the page after its heading"
    );
}
