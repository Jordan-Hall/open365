//! A note reference whose run has no superscript style is plain text in Word: the digit keeps
//! the run's size and advance (`fixtures/reference/pdf/probes/probe-links`, where Word's link
//! rectangle of the mark ends 10.08 pt after its start, glyph 5.58 pt plus 2 x 2.25 pt).

use std::path::PathBuf;

use open365_docx::Document;
use open365_layout::{LayoutOptions, layout_document};
use open365_opc::Limits;
use open365_text::FontLibrary;

#[test]
fn unstyled_note_reference_keeps_the_run_size_and_advance() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/reference/pdf/probes/probe-links.docx");
    let bytes = std::fs::read(path).expect("probe");
    let doc = Document::open(&bytes, &Limits::default()).expect("open");
    let fonts = FontLibrary::bundled();
    let layout = layout_document(&doc, &fonts, LayoutOptions::default());
    let (_, line) = layout
        .lines()
        .find(|(_, l)| l.text.starts_with("Body"))
        .expect("body line");
    let runs: Vec<_> = line.runs.iter().filter(|r| !r.glyphs.is_empty()).collect();
    let body = runs.first().expect("run");
    let mark = runs
        .iter()
        .flat_map(|r| r.glyphs.iter().map(move |g| (r, g)))
        .find(|(_, g)| line.cluster_text(g) == "1")
        .expect("mark glyph");
    assert!(!mark.0.superscript, "plain, not a drawn-small mark");
    assert!((mark.0.size - body.size).abs() < 1e-9);
    assert_eq!(mark.1.y_offset, 0.0);
}
