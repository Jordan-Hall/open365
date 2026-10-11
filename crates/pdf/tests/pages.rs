//! `PdfOptions::pages` exports a range of the laid-out pages.

mod common;

use std::sync::atomic::AtomicBool;

use common::{layout_fixture, scan};
use open365_pdf::{PdfError, PdfMetadata, PdfOptions, export_pdf_with};
use open365_text::FontLibrary;

#[test]
fn a_page_range_keeps_only_those_pages() {
    let fonts = FontLibrary::bundled();
    let (doc, layout) = layout_fixture("layout-fn-endnotes", &fonts).expect("bundled fonts");
    assert!(layout.pages.len() >= 3);
    let run = |pages| {
        export_pdf_with(
            &doc,
            &layout,
            &fonts,
            &PdfMetadata::default(),
            &PdfOptions {
                pages,
                ..PdfOptions::default()
            },
            &AtomicBool::new(false),
        )
    };
    let all = run(None).expect("all");
    let one = run(Some((2, 2))).expect("one page");
    let tail = run(Some((2, 99))).expect("clamped");
    let sizes = |b: &[u8]| scan::glyphs_of(b).len();
    assert_eq!(sizes(&all.bytes), layout.pages.len());
    assert_eq!(sizes(&one.bytes), 1);
    assert_eq!(sizes(&tail.bytes), layout.pages.len() - 1);
    assert!(one.bytes.len() < all.bytes.len());
    assert_eq!(
        run(Some((layout.pages.len() + 1, 99))).unwrap_err(),
        PdfError::NoPages
    );
}
