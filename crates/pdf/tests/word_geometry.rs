//! Page geometry parity: where our PDF puts every glyph against where Word's own "Save as PDF"
//! (`fixtures/reference/pdf/<name>.pdf`, made by `tools/office/make-pdf-refs.ps1`) puts it.
//!
//! Both files are read by the same small content reader (`common::scan`). Word's glyphs and ours
//! are paired by aligning the two non-blank character sequences of a page (longest common
//! subsequence), so a difference of layout (a line that breaks elsewhere, a moved table) costs
//! pairs instead of misaligning the rest. A pair agrees when x and y differ by at most 0.5 pt.
//! Differences that come from the layout engine belong to the layout agent: this test measures and
//! reports, it does not explain.

mod common;

use common::scan::{Glyph, glyphs_of};
use common::{fixture_names, layout_fixture, root};
use open365_pdf::{PdfMetadata, export_pdf};
use open365_text::FontLibrary;

const TOLERANCE: f64 = 0.5;

/// Characters of a page, each with the position of the glyph it came from. Blanks are dropped:
/// Word and we place them differently (and nothing is drawn for them).
fn chars_of(glyphs: &[Glyph]) -> Vec<(char, f64, f64)> {
    glyphs
        .iter()
        .flat_map(|g| {
            g.text
                .chars()
                .filter(|c| !c.is_whitespace() && *c != '\u{fffd}')
                .map(|c| (c, g.x, g.y))
        })
        .collect()
}

/// Longest common subsequence of two character sequences as index pairs. The common head and tail
/// are paired first so that the quadratic table only covers the part that differs.
fn align(a: &[(char, f64, f64)], b: &[(char, f64, f64)]) -> Vec<(usize, usize)> {
    let mut head = 0;
    while head < a.len() && head < b.len() && a[head].0 == b[head].0 {
        head += 1;
    }
    let mut tail = 0;
    while tail < a.len() - head
        && tail < b.len() - head
        && a[a.len() - 1 - tail].0 == b[b.len() - 1 - tail].0
    {
        tail += 1;
    }
    let (am, bm) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
    let mut pairs: Vec<(usize, usize)> = (0..head).map(|i| (i, i)).collect();
    if !am.is_empty() && !bm.is_empty() && am.len() * bm.len() <= 40_000_000 {
        let w = bm.len() + 1;
        let mut t = vec![0u16; (am.len() + 1) * w];
        for i in (0..am.len()).rev() {
            for j in (0..bm.len()).rev() {
                t[i * w + j] = if am[i].0 == bm[j].0 {
                    t[(i + 1) * w + j + 1] + 1
                } else {
                    t[(i + 1) * w + j].max(t[i * w + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < am.len() && j < bm.len() {
            if am[i].0 == bm[j].0 {
                pairs.push((head + i, head + j));
                i += 1;
                j += 1;
            } else if t[(i + 1) * w + j] >= t[i * w + j + 1] {
                i += 1;
            } else {
                j += 1;
            }
        }
    }
    let (an, bn) = (a.len() - tail, b.len() - tail);
    pairs.extend((0..tail).map(|k| (an + k, bn + k)));
    pairs
}

#[derive(Default, Debug)]
struct Tally {
    word_glyphs: usize,
    paired: usize,
    agree: usize,
    pages_word: usize,
    pages_ours: usize,
    same_size_pages: usize,
    /// Pages whose every Word glyph is paired and agrees.
    exact_pages: usize,
    /// Paired glyphs that miss the tolerance in x only, in y only, in both.
    miss_x: usize,
    miss_y: usize,
    miss_xy: usize,
}

#[test]
fn glyph_positions_agree_with_words_pdf() {
    let fonts = FontLibrary::bundled();
    let dir = root().join("fixtures/reference/pdf");
    let mut total = Tally::default();
    let mut rows = Vec::new();
    for name in fixture_names() {
        let Ok(word_bytes) = std::fs::read(dir.join(format!("{name}.pdf"))) else {
            continue;
        };
        let Some((doc, layout)) = layout_fixture(&name, &fonts) else {
            continue;
        };
        let ours = export_pdf(&doc, &layout, &fonts, &PdfMetadata::default()).expect("export");
        let word_pages = glyphs_of(&word_bytes);
        let our_pages = glyphs_of(&ours.bytes);
        let mut t = Tally {
            pages_word: word_pages.len(),
            pages_ours: our_pages.len(),
            ..Tally::default()
        };
        for (w, o) in word_pages.iter().zip(&our_pages) {
            if (w.width - o.width).abs() < 0.01 && (w.height - o.height).abs() < 0.01 {
                t.same_size_pages += 1;
            } else {
                println!(
                    "{name}: page size ours {}x{} Word {}x{}",
                    o.width, o.height, w.width, w.height
                );
            }
            // y up, from the page bottom: compare distances from the top.
            let wc: Vec<_> = chars_of(&w.glyphs)
                .into_iter()
                .map(|(c, x, y)| (c, x, w.height - y))
                .collect();
            let oc: Vec<_> = chars_of(&o.glyphs)
                .into_iter()
                .map(|(c, x, y)| (c, x, o.height - y))
                .collect();
            let mut agree = 0;
            let pairs = align(&wc, &oc);
            for (i, j) in &pairs {
                let ok_x = (wc[*i].1 - oc[*j].1).abs() <= TOLERANCE;
                let ok_y = (wc[*i].2 - oc[*j].2).abs() <= TOLERANCE;
                match (ok_x, ok_y) {
                    (true, true) => agree += 1,
                    (false, true) => t.miss_x += 1,
                    (true, false) => t.miss_y += 1,
                    (false, false) => t.miss_xy += 1,
                }
            }
            t.word_glyphs += wc.len();
            t.paired += pairs.len();
            t.agree += agree;
            t.exact_pages += usize::from(agree == wc.len() && wc.len() == oc.len());
        }
        rows.push(format!(
            "{name:<34} pages {:>2}/{:<2} glyphs {:>6} paired {:>5.1}% within 0.5 pt {:>5.1}%",
            t.pages_ours,
            t.pages_word,
            t.word_glyphs,
            100.0 * t.paired as f64 / t.word_glyphs.max(1) as f64,
            100.0 * t.agree as f64 / t.word_glyphs.max(1) as f64,
        ));
        total.word_glyphs += t.word_glyphs;
        total.paired += t.paired;
        total.agree += t.agree;
        total.pages_word += t.pages_word;
        total.pages_ours += t.pages_ours;
        total.same_size_pages += t.same_size_pages;
        total.exact_pages += t.exact_pages;
        total.miss_x += t.miss_x;
        total.miss_y += t.miss_y;
        total.miss_xy += t.miss_xy;
    }
    for r in &rows {
        println!("{r}");
    }
    println!(
        "word geometry: {} fixtures, {} Word glyphs, {:.2}% paired, {:.2}% within {TOLERANCE} pt of Word; pages {} ours / {} Word, same size {}, exact {}",
        rows.len(),
        total.word_glyphs,
        100.0 * total.paired as f64 / total.word_glyphs.max(1) as f64,
        100.0 * total.agree as f64 / total.word_glyphs.max(1) as f64,
        total.pages_ours,
        total.pages_word,
        total.same_size_pages,
        total.exact_pages,
    );
    println!(
        "paired glyphs outside the tolerance: x only {}, y only {}, both {}",
        total.miss_x, total.miss_y, total.miss_xy
    );
    assert!(rows.len() >= 20, "only {} fixtures compared", rows.len());
    assert!(total.word_glyphs > 10_000);
    // Page count and MediaBox agree on every page (Word's 600 dpi grid for A4 included).
    assert_eq!(total.pages_ours, total.pages_word);
    assert_eq!(total.same_size_pages, total.pages_word);
    // Floors, measured 2026-10-11: they only move up. The rest is layout-engine disagreement
    // (justification, tabs, list labels, text boxes, footnotes), the layout agent's to close.
    let pct = |n: usize| 100.0 * n as f64 / total.word_glyphs as f64;
    assert!(
        pct(total.paired) >= 99.3,
        "paired {:.2}%",
        pct(total.paired)
    );
    assert!(
        pct(total.agree) >= 87.5,
        "within tolerance {:.2}%",
        pct(total.agree)
    );
    assert!(
        total.exact_pages >= 185,
        "exact pages {}",
        total.exact_pages
    );
}
