//! `w:vertAlign` runs: Word draws them at 0.65 of the size, raised by a third of an em
//! (superscript) or lowered by a twelfth (subscript); measured from Word's PDF, see
//! `docs/architecture/word-layout.md`.

use std::path::PathBuf;

use open365_docx::{DocPos, Document, RunPropsPatch, VertAlign};
use open365_layout::{LayoutOptions, layout_document};
use open365_opc::Limits;
use open365_text::FontLibrary;

fn doc(vert: Option<(usize, usize, VertAlign)>) -> Document {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/docx/plain-paragraphs.docx");
    let bytes = std::fs::read(path).expect("fixture");
    let mut d = Document::open(&bytes, &Limits::default()).expect("open");
    if let Some((a, b, v)) = vert {
        d.set_run_props(
            DocPos::new(0, a),
            DocPos::new(0, b),
            &RunPropsPatch::vert_align(v),
        )
        .expect("patch");
    }
    d
}

/// (run size, first glyph y_offset, vert_align) of every run on the first line.
fn first_line(d: &Document) -> Vec<(f64, f64, VertAlign)> {
    let fonts = FontLibrary::bundled();
    let layout = layout_document(d, &fonts, LayoutOptions::default());
    let (_, line) = layout.lines().next().expect("a line");
    line.runs
        .iter()
        .filter(|r| !r.glyphs.is_empty())
        .map(|r| (r.size, r.glyphs[0].y_offset, r.vert_align))
        .collect()
}

#[test]
fn baseline_runs_are_unchanged() {
    let runs = first_line(&doc(None));
    assert!(
        runs.iter()
            .all(|r| r.1 == 0.0 && r.2 == VertAlign::Baseline)
    );
}

#[test]
fn superscript_and_subscript_shrink_and_shift() {
    let base = first_line(&doc(None))[0].0;
    for (v, sign) in [(VertAlign::Superscript, 1.0), (VertAlign::Subscript, -1.0)] {
        let runs = first_line(&doc(Some((1, 3, v))));
        let marked: Vec<_> = runs.iter().filter(|r| r.2 == v).collect();
        assert_eq!(marked.len(), 1, "{runs:?}");
        let (size, y, _) = *marked[0];
        assert!((size - base * 0.65).abs() < 1e-9, "{size} vs {base}");
        assert!(y * sign > 0.0, "{v:?} moves the wrong way: {y}");
        let em = if sign > 0.0 { 1.0 / 3.0 } else { 1.0 / 12.0 };
        assert!(
            (y.abs() - base * em).abs() < 1e-6,
            "{v:?}: {y} for base {base}"
        );
        // The neighbours stay on the baseline.
        assert!(
            runs.iter()
                .filter(|r| r.2 == VertAlign::Baseline)
                .all(|r| r.1 == 0.0)
        );
    }
}
