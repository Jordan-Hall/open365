//! Our PDF's line baselines against the baselines in Word's own PDF
//! (`fixtures/reference/layout/*.json`), paired exactly as `word_parity` pairs lines: by story
//! and first-character offset, on the same page, within 0.5 pt.

mod common;

use std::collections::HashMap;

use common::{Reader, extract, fixture_names, layout_fixture, root};
use open365_pdf::{PdfMetadata, export_pdf};
use open365_text::FontLibrary;
use serde_json::Value;

const TOLERANCE: f64 = 0.5;

/// Page, layout baseline, baseline read back from the PDF.
type Seen = (usize, f64, f64);

#[test]
fn pdf_baselines_agree_with_word() {
    let fonts = FontLibrary::bundled();
    let (mut compared, mut matched_pdf, mut matched_layout) = (0, 0, 0);
    let mut fixtures = 0;
    let mut disagreements = Vec::new();
    for name in fixture_names() {
        let Some((doc, layout)) = layout_fixture(&name, &fonts) else {
            continue;
        };
        let text =
            std::fs::read_to_string(root().join(format!("fixtures/reference/layout/{name}.json")))
                .expect("reference");
        let reference: Value = serde_json::from_str(&text).expect("json");
        let out = export_pdf(&doc, &layout, &fonts, &PdfMetadata::default()).expect("export");
        let reader = Reader::new(&out.bytes);
        let (pages, _) = extract(&reader);

        // (story, start) -> (page, layout baseline, baseline read back from the PDF).
        let mut engine: HashMap<(u8, usize), Vec<Seen>> = HashMap::new();
        for (pi, page) in layout.pages.iter().enumerate() {
            // Height of the page as written (Word's grid), not the section's.
            let media_h = reader.pages()[pi].1.dict()["MediaBox"].arr()[3].num();
            let with_glyphs = page
                .header
                .iter()
                .chain(&page.footer)
                .chain(&page.lines)
                .chain(page.frames.iter().flat_map(|f| f.lines.iter()))
                .chain(&page.notes)
                .filter(|l| {
                    l.runs.iter().any(|r| !r.glyphs.is_empty()) && !common::clipped_away(page, l)
                });
            // A line can be several marked-content sequences (list label, hyperlinks).
            let mut rest = pages[pi].iter().flat_map(|l| l.glyphs.iter().cloned());
            for line in with_glyphs {
                let want: usize = line.runs.iter().map(|r| r.glyphs.len()).sum();
                let pdf_glyphs: Vec<_> = rest.by_ref().take(want).collect();
                // The first glyph of a run that is not a note mark sits on the line baseline.
                let skip: usize = line
                    .runs
                    .iter()
                    .take_while(|r| r.superscript)
                    .map(|r| r.glyphs.len())
                    .sum();
                let Some(g) = pdf_glyphs.get(skip).or(pdf_glyphs.first()) else {
                    continue;
                };
                let pdf_baseline = media_h - g.y;
                engine
                    .entry((line.story.word_code(), line.start))
                    .or_default()
                    .push((page.number, line.baseline, pdf_baseline));
            }
        }
        fixtures += 1;
        for l in reference["lines"].as_array().expect("lines") {
            let Some(baseline) = l["baseline"].as_f64() else {
                continue;
            };
            let story = l["story"].as_u64().map_or(1, |v| v as u8);
            let page = l["page"].as_u64().expect("page") as usize;
            let start = l["start"].as_u64().expect("start") as usize;
            let Some(c) = engine.get(&(story, start)) else {
                continue;
            };
            let (p, ours, pdf) = *c.iter().find(|e| e.0 == page).unwrap_or(&c[0]);
            compared += 1;
            let layout_ok = p == page && (ours - baseline).abs() <= TOLERANCE;
            let pdf_ok = p == page && (pdf - baseline).abs() <= TOLERANCE;
            matched_layout += usize::from(layout_ok);
            matched_pdf += usize::from(pdf_ok);
            if layout_ok != pdf_ok {
                disagreements.push(format!(
                    "{name} p{page} @{start}: layout {ours:.3}, pdf {pdf:.3}, word {baseline:.3}"
                ));
            }
        }
    }
    println!(
        "word baselines: {fixtures} fixtures, {compared} paired lines, PDF within 0.5 pt of Word on {matched_pdf} (layout engine: {matched_layout})"
    );
    assert!(fixtures >= 20);
    assert!(
        disagreements.is_empty(),
        "PDF and layout disagree:\n{}",
        disagreements.join("\n")
    );
    assert_eq!(matched_pdf, matched_layout);
}
