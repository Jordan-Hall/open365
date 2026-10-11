//! Extracted text against Word's own PDF. Word draws a tab leader as a string of the leader
//! character inside the paragraph's marked content, so the dots copy out with the text; we do the
//! same (no artifact, no ActualText).

mod common;

use common::scan::glyphs_of;
use common::{layout_fixture, root};
use open365_pdf::{PdfMetadata, export_pdf};
use open365_text::FontLibrary;

/// Text of each page with every run of four or more leader characters shown as `~`. Word draws
/// blanks and leaders with a simple font the scanner cannot decode (it reads U+FFFD), so in Word's
/// text a long run of U+FFFD is a leader and a single one is a blank; in ours the leader is dots.
fn page_text(data: &[u8]) -> Vec<String> {
    let leader = |c: char| matches!(c, '.' | '-' | '_' | '\u{fffd}');
    glyphs_of(data)
        .iter()
        .map(|p| {
            let raw: Vec<char> = p.glyphs.iter().flat_map(|g| g.text.chars()).collect();
            let mut out = String::new();
            let mut i = 0;
            while i < raw.len() {
                let run = raw[i..].iter().take_while(|c| leader(**c)).count();
                if run >= 4 {
                    out.push('~');
                    i += run;
                } else {
                    if !raw[i].is_whitespace() && raw[i] != '\u{fffd}' {
                        out.push(raw[i]);
                    }
                    i += 1;
                }
            }
            out
        })
        .collect()
}

#[test]
fn layout_tabs_text_includes_leaders_as_words_does() {
    let fonts = FontLibrary::bundled();
    let word = std::fs::read(root().join("fixtures/reference/pdf/layout-tabs.pdf"))
        .expect("Word's PDF of layout-tabs");
    let (doc, layout) = layout_fixture("layout-tabs", &fonts).expect("layout");
    let ours = export_pdf(&doc, &layout, &fonts, &PdfMetadata::default()).expect("export");
    let (w, o) = (page_text(&word), page_text(&ours.bytes));
    assert!(
        w.iter().any(|t| t.contains('~')),
        "Word draws leaders: {w:?}"
    );
    assert_eq!(w.len(), o.len());
    for (i, (w, o)) in w.iter().zip(&o).enumerate() {
        assert_eq!(w, o, "page {}", i + 1);
    }
    let bytes = String::from_utf8_lossy(&ours.bytes);
    assert!(
        !bytes.contains("/Artifact BMC") && !bytes.contains("/Artifact <<"),
        "leaders are not artifacts"
    );
}
