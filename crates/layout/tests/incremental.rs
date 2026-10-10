//! Incremental layout must equal a full layout after any edit, and must be fast.
//!
//! The property test applies random edits (typing, deleting, splitting and joining paragraphs,
//! line and tab characters) to fixtures with keep rules, page breaks, lists and tables, and
//! after each edit compares [`IncrementalLayout::update`] with [`layout_document`] on the same
//! document. The timing test measures one keystroke in a 355-page generated document.

use std::path::PathBuf;
use std::time::Instant;

use open365_docx::{DocPos, Document};
use open365_layout::{IncrementalLayout, LayoutOptions, layout_document};
use open365_opc::Limits;
use open365_text::FontLibrary;

fn fixture(name: &str) -> Document {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/docx")
        .join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Document::open(&bytes, &Limits::default()).expect("open")
}

/// xorshift64*, deterministic across platforms.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

const SNIPPETS: [&str; 9] = [
    "x",
    "Z ",
    "a longer run of words that wraps onto another line when it is long enough ",
    "\n",
    "first\nsecond",
    "tab\there",
    "line\u{b}break",
    "wide WWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWW",
    "fi ffi ligatures office ",
];

fn para_len(doc: &Document, i: usize) -> usize {
    doc.paragraph_text(i).map_or(0, |t| t.chars().count())
}

fn random_edit(doc: &mut Document, rng: &mut Rng) {
    let count = doc.paragraph_count();
    let p = rng.below(count);
    let len = para_len(doc, p);
    let a = DocPos::new(p, rng.below(len + 1));
    match rng.below(5) {
        // Type.
        0..=2 => {
            let text = SNIPPETS[rng.below(SNIPPETS.len())];
            let _ = doc.replace_range(a, a, text);
        }
        // Delete within a paragraph or across a few.
        3 => {
            let q = (p + rng.below(3)).min(count - 1);
            let b = DocPos::new(q, rng.below(para_len(doc, q) + 1));
            let _ = doc.delete_range(a.min(b), a.max(b));
        }
        // Split.
        _ => {
            let _ = doc.split_paragraph(a);
        }
    }
}

fn grow(doc: &mut Document, paragraphs: usize) {
    let text = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor \
                incididunt ut labore et dolore magna aliqua. "
        .repeat(3);
    let all = vec![text; paragraphs].join("\n");
    doc.replace_range(DocPos::new(0, 0), DocPos::new(0, 0), &all)
        .expect("grow");
}

fn check_equal(inc: &IncrementalLayout, doc: &Document, fonts: &FontLibrary, what: &str) {
    let full = layout_document(doc, fonts, LayoutOptions::default());
    let got = inc.layout();
    assert_eq!(got.pages.len(), full.pages.len(), "{what}: page count");
    for (i, (a, b)) in got.pages.iter().zip(&full.pages).enumerate() {
        assert_eq!(a, b, "{what}: page {} differs", i + 1);
    }
    assert_eq!(got, &full, "{what}: layout differs");
}

fn property(name: &str, seed: u64, edits: usize, grow_by: usize) {
    let fonts = FontLibrary::bundled();
    let mut doc = fixture(name);
    if grow_by > 0 {
        grow(&mut doc, grow_by);
    }
    let mut inc = IncrementalLayout::new();
    inc.update(&doc, &fonts, LayoutOptions::default());
    check_equal(&inc, &doc, &fonts, &format!("{name} initial"));
    let mut rng = Rng(seed);
    let mut converged = 0;
    for step in 0..edits {
        random_edit(&mut doc, &mut rng);
        inc.update(&doc, &fonts, LayoutOptions::default());
        converged += usize::from(inc.stats().converged);
        check_equal(&inc, &doc, &fonts, &format!("{name} step {step}"));
    }
    println!("{name}: {edits} edits, {converged} converged early");
}

#[test]
fn incremental_equals_full_on_plain_text() {
    property("plain-paragraphs.docx", 0x9E37_79B9, 80, 60);
}

#[test]
fn incremental_equals_full_with_keep_rules_and_breaks() {
    property("layout-keep.docx", 11, 60, 0);
    property("layout-pagination.docx", 12, 60, 0);
    property("layout-keep.docx", 13, 40, 30);
}

#[test]
fn incremental_equals_full_with_lists_and_tables() {
    property("layout-lists-multilevel.docx", 21, 60, 0);
    property("layout-lists-style.docx", 22, 40, 0);
    property("layout-table-pages.docx", 23, 60, 0);
    property("layout-table-flow.docx", 24, 60, 0);
    property("layout-table-rows.docx", 25, 40, 0);
}

#[test]
fn incremental_equals_full_with_sections_and_mixed_sizes() {
    property("layout-indents-spacing.docx", 31, 40, 20);
    property("layout-mixed-sizes.docx", 32, 40, 0);
    property("layout-marks.docx", 33, 40, 0);
}

/// A 355-page document like the desktop's long-document test.
fn long_document() -> Document {
    let mut doc = fixture("plain-paragraphs.docx");
    let sentence =
        "The quick brown fox jumps over the lazy dog while office files fluff difficult fixtures. ";
    let text = vec![sentence.repeat(6); 2600].join("\n");
    doc.replace_range(DocPos::new(0, 0), DocPos::new(0, 0), &text)
        .expect("insert");
    doc
}

#[test]
fn one_keystroke_in_a_355_page_document_relayouts_in_under_16_ms() {
    let fonts = FontLibrary::bundled();
    let mut doc = long_document();
    let mut inc = IncrementalLayout::new();
    let started = Instant::now();
    inc.update(&doc, &fonts, LayoutOptions::default());
    let first = started.elapsed();
    let pages = inc.layout().pages.len();
    assert!(pages >= 300, "only {pages} pages");

    let mut samples = Vec::new();
    let mut at = DocPos::new(1300, 3);
    for _ in 0..25 {
        doc.replace_range(at, at, "Z").expect("type");
        at.offset += 1;
        let started = Instant::now();
        inc.update(&doc, &fonts, LayoutOptions::default());
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
        let s = inc.stats();
        assert!(s.converged, "pagination did not re-converge: {s:?}");
        assert!(s.blocks_laid_out <= 1, "{s:?}");
    }
    samples.sort_by(f64::total_cmp);
    let median = samples[samples.len() / 2];
    let worst = samples[samples.len() - 1];
    println!(
        "{pages} pages: first layout {first:?}, keystroke median {median:.2} ms, worst {worst:.2} ms, {:?}",
        inc.stats()
    );
    check_equal(&inc, &doc, &fonts, "after typing");
    // Debug builds are an order of magnitude slower; the budget is for release.
    let budget = if cfg!(debug_assertions) { 400.0 } else { 16.0 };
    assert!(
        median < budget && worst < budget * 2.0,
        "keystroke relayout median {median:.2} ms, worst {worst:.2} ms over {budget} ms"
    );
}

#[test]
fn typing_in_the_first_and_last_paragraph_is_cheap_too() {
    let fonts = FontLibrary::bundled();
    let mut doc = long_document();
    let mut inc = IncrementalLayout::new();
    inc.update(&doc, &fonts, LayoutOptions::default());
    for para in [0, doc.paragraph_count() - 1] {
        let at = DocPos::new(para, 0);
        doc.replace_range(at, at, "Q").expect("type");
        let started = Instant::now();
        inc.update(&doc, &fonts, LayoutOptions::default());
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        println!("paragraph {para}: {ms:.2} ms, {:?}", inc.stats());
        check_equal(&inc, &doc, &fonts, "edge edit");
    }
}

#[test]
fn glyphs_carry_the_face_they_were_shaped_with() {
    let fonts = FontLibrary::bundled();
    let doc = fixture("layout-align.docx");
    let layout = layout_document(&doc, &fonts, LayoutOptions::default());
    let mut glyphs = 0;
    for (_, line) in layout.lines() {
        for run in &line.runs {
            for g in &run.glyphs {
                assert!(g.face.index() < fonts.len());
                glyphs += 1;
            }
        }
    }
    assert!(glyphs > 0);
    // Text the run's own face covers is drawn from that face.
    let first = layout
        .lines()
        .flat_map(|(_, l)| l.runs.iter())
        .find(|r| !r.glyphs.is_empty())
        .expect("a run");
    assert!(first.glyphs.iter().all(|g| g.face == first.face));
}
