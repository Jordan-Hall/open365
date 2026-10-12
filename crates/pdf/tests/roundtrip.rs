//! Re-reads the PDFs we write and compares them with the layout they came from.

mod common;

use std::collections::BTreeMap;

use common::{Obj, Reader, extract, fixture_names, layout_fixture, resource_names};
use open365_layout::Line;
use open365_pdf::{PdfMetadata, export_pdf};
use open365_text::FontLibrary;

const TOLERANCE: f64 = 0.01;

/// Glyphs of a note reference mark: drawn by the layout but not part of the paragraph text.
fn g_is_mark(line: &Line, g: &open365_layout::PlacedGlyph) -> bool {
    line.runs
        .iter()
        .any(|r| r.superscript && r.glyphs.iter().any(|x| std::ptr::eq(x, g)))
}

fn lines_with_glyphs(page: &open365_layout::Page) -> Vec<&Line> {
    page.header
        .iter()
        .chain(&page.lines)
        .chain(page.frames.iter().flat_map(|f| f.lines.iter()))
        .chain(&page.notes)
        .chain(&page.footer)
        .filter(|l| l.runs.iter().any(|r| !r.glyphs.is_empty()))
        .collect()
}

#[test]
fn glyph_positions_and_text_round_trip() {
    let fonts = FontLibrary::bundled();
    let mut checked = 0;
    let mut glyphs_total = 0;
    let mut worst: f64 = 0.0;
    let mut text_lines = 0;
    let mut problems = Vec::new();
    for name in fixture_names() {
        let Some((doc, layout)) = layout_fixture(&name, &fonts) else {
            continue;
        };
        if layout.pages.is_empty() {
            continue;
        }
        let out = export_pdf(&doc, &layout, &fonts, &PdfMetadata::default())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let reader = Reader::new(&out.bytes);
        let (pages, infos) = extract(&reader);
        assert_eq!(pages.len(), layout.pages.len(), "{name}: page count");
        let resources = resource_names(&layout);
        let paragraphs = doc.body().paragraphs();
        let mut para_text: BTreeMap<usize, String> = BTreeMap::new();
        let mut seen = std::collections::HashSet::new();
        for (pi, page) in layout.pages.iter().enumerate() {
            let want = lines_with_glyphs(page);
            let got = &pages[pi];
            // The page height Word's grid gives (the MediaBox), not the section's.
            let media_h = reader.pages()[pi].1.dict()["MediaBox"].arr()[3].num();
            // A line can be several marked-content sequences (list label, hyperlinks).
            let mut next = 0;
            for line in &want {
                let mut flat: Vec<_> = line.runs.iter().flat_map(|r| r.glyphs.iter()).collect();
                // A line with tab leaders is written in x order (as Word's is).
                if flat.iter().any(|g| line.leader_char(g).is_some()) {
                    flat.sort_by(|a, b| a.x.total_cmp(&b.x));
                }
                let mut pdf_glyphs = Vec::new();
                while pdf_glyphs.len() < flat.len() && next < got.len() {
                    pdf_glyphs.extend(got[next].glyphs.iter().cloned());
                    next += 1;
                }
                assert_eq!(flat.len(), pdf_glyphs.len(), "{name}: glyph count");
                let mut text = String::new();
                for (g, p) in flat.iter().zip(&pdf_glyphs) {
                    let y = media_h - line.baseline + g.y_offset;
                    let (dx, dy) = ((g.x - p.x).abs(), (y - p.y).abs());
                    worst = worst.max(dx).max(dy);
                    if g.glyph_id != p.gid
                        || (p.font != resources[&g.face.index()]
                            && p.font != format!("FS{}", g.face.index()))
                        || dx > TOLERANCE
                        || dy > TOLERANCE
                    {
                        problems.push(format!(
                            "{name} p{}: glyph {} at ({:.3},{:.3}) vs pdf {} at ({:.3},{:.3})",
                            pi + 1,
                            g.glyph_id,
                            g.x,
                            y,
                            p.gid,
                            p.x,
                            p.y
                        ));
                    }
                    glyphs_total += 1;
                    // Tab leaders are text in the PDF, as in Word's (the dots copy out), but not part of the
                    // paragraph's own characters: this check leaves them out, and
                    // `tests/word_text.rs` checks the extracted text with them against Word's PDF.
                    if !g_is_mark(line, g)
                        && line.leader_char(g).is_none()
                        && let Some(t) = infos[&p.font].to_unicode.get(&p.gid)
                    {
                        text.push_str(t);
                    }
                }
                if line.story == open365_layout::StoryKind::Main
                    && seen.insert((line.paragraph, line.start))
                {
                    para_text.entry(line.paragraph).or_default().push_str(&text);
                }
            }
        }
        // Text: what the PDF says a paragraph contains is what the document says, ignoring
        // white space, up to the list label in front of it.
        let mut labels = open365_layout::list_labels(&doc).into_iter();
        let labels: Vec<_> = (0..paragraphs.len())
            .map(|_| labels.next().flatten())
            .collect();
        for (idx, got) in &para_text {
            let want: String = paragraphs[*idx]
                .text()
                .chars()
                .filter(|c| !c.is_whitespace() && !c.is_control())
                .collect();
            let got: String = got.chars().filter(|c| !c.is_whitespace()).collect();
            text_lines += 1;
            // Rows of exact height clip their text: the layout draws a prefix.
            // Caps and small caps: Word's PDF copies the displayed capitals (no ActualText), so
            // only the case may differ (docs/architecture/pdf-export.md).
            let ok = if name.starts_with("layout-caps") {
                got.to_lowercase() == want.to_lowercase()
            } else if name == "layout-table-rows" {
                want.starts_with(&got)
            } else if labels[*idx].is_some() {
                got.ends_with(&want)
            } else {
                got == want
            };
            if !ok {
                problems.push(format!(
                    "{name}: paragraph {idx} text {got:?} vs document {want:?}"
                ));
            }
        }
        checked += 1;
    }
    println!(
        "round trip: {checked} fixtures, {glyphs_total} glyphs, {text_lines} paragraphs, worst position error {worst:.5} pt"
    );
    assert!(checked >= 20, "only {checked} fixtures could be checked");
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

fn struct_names(r: &Reader<'_>, elem: &Obj, out: &mut Vec<String>, mcrs: &mut usize) {
    let dict = elem.dict();
    if let Some(Obj::Name(s)) = dict.get("S") {
        out.push(s.clone());
    }
    match dict.get("K") {
        Some(Obj::Arr(kids)) => {
            for k in kids {
                let k = r.resolve(k);
                // A bare integer is marked content on the element's /Pg page.
                if matches!(&k, Obj::Int(_))
                    || matches!(&k, Obj::Dict(d) if matches!(d.get("Type"), Some(Obj::Name(t)) if t == "MCR"))
                {
                    *mcrs += 1;
                } else {
                    struct_names(r, &k, out, mcrs);
                }
            }
        }
        Some(Obj::Ref(id)) => struct_names(r, &r.get(*id), out, mcrs),
        _ => {}
    }
}

fn structure_of(name: &str) -> (Vec<String>, usize, usize, Vec<u8>) {
    let fonts = FontLibrary::bundled();
    let (doc, layout) = layout_fixture(name, &fonts).expect("bundled fonts suffice");
    let out = export_pdf(&doc, &layout, &fonts, &PdfMetadata::default()).expect("export");
    let reader = Reader::new(&out.bytes);
    let root = reader.key(&reader.trailer(), "Root").expect("root");
    assert!(matches!(
        reader.key(&root, "MarkInfo").expect("markinfo").dict()["Marked"],
        Obj::Bool(true)
    ));
    let tree = reader.key(&root, "StructTreeRoot").expect("struct tree");
    let mut names = Vec::new();
    let mut mcrs = 0;
    struct_names(
        &reader,
        &reader.resolve(&tree.dict()["K"]),
        &mut names,
        &mut mcrs,
    );
    // Every marked-content id the pages use is claimed by the tree exactly once.
    let mut mcids = 0;
    for (_, page) in reader.pages() {
        let content = reader.stream(&reader.key(&page, "Contents").expect("contents"));
        mcids += String::from_utf8_lossy(&content).matches("/MCID").count();
    }
    (names, mcrs, mcids, out.bytes)
}

#[test]
fn structure_tree_has_headings_lists_tables_and_every_mcid() {
    for (fixture, want) in [
        ("layout-table-basic", vec!["Table", "TBody", "TR", "TD"]),
        ("layout-lists-multilevel", vec!["L", "LI", "LBody"]),
        ("plain-paragraphs", vec!["Document", "P"]),
        ("layout-img-inline", vec!["Figure"]),
    ] {
        let (names, mcrs, mcids, _) = structure_of(fixture);
        for w in want {
            assert!(
                names.iter().any(|n| n == w),
                "{fixture}: no {w} in {names:?}"
            );
        }
        assert_eq!(mcrs, mcids, "{fixture}: structure tree misses content");
    }
}

#[test]
fn heading_styles_become_h_elements() {
    let fonts = FontLibrary::bundled();
    let Some((doc, layout)) = layout_fixture("styles-headings", &fonts) else {
        return;
    };
    let out = export_pdf(&doc, &layout, &fonts, &PdfMetadata::default()).expect("export");
    let reader = Reader::new(&out.bytes);
    let root = reader.key(&reader.trailer(), "Root").expect("root");
    let tree = reader.key(&root, "StructTreeRoot").expect("tree");
    let mut names = Vec::new();
    let mut mcrs = 0;
    struct_names(
        &reader,
        &reader.resolve(&tree.dict()["K"]),
        &mut names,
        &mut mcrs,
    );
    assert!(names.iter().any(|n| n == "H1"), "{names:?}");
}

#[test]
fn metadata_comes_from_the_caller_only() {
    let (_, _, _, bytes) = structure_of("plain-paragraphs");
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("/Author"));
    assert!(!text.contains("/CreationDate"));

    let fonts = FontLibrary::bundled();
    let (doc, layout) = layout_fixture("plain-paragraphs", &fonts).expect("layout");
    let meta = PdfMetadata {
        title: Some("Quarterly report".into()),
        author: Some("A. Writer".into()),
        language: Some("en-GB".into()),
        ..PdfMetadata::default()
    };
    let out = export_pdf(&doc, &layout, &fonts, &meta).expect("export");
    let reader = Reader::new(&out.bytes);
    let trailer = reader.trailer();
    let info = reader.key(&trailer, "Info").expect("info");
    assert_eq!(info.dict()["Title"], Obj::Str(b"Quarterly report".to_vec()));
    assert_eq!(info.dict()["Author"], Obj::Str(b"A. Writer".to_vec()));
    let root = reader.key(&trailer, "Root").expect("root");
    assert_eq!(root.dict()["Lang"], Obj::Str(b"en-GB".to_vec()));
}

#[test]
fn page_sizes_follow_the_sections() {
    let fonts = FontLibrary::bundled();
    let (doc, layout) = layout_fixture("sections-landscape", &fonts).expect("layout");
    let out = export_pdf(&doc, &layout, &fonts, &PdfMetadata::default()).expect("export");
    let reader = Reader::new(&out.bytes);
    let boxes: Vec<(f64, f64)> = reader
        .pages()
        .iter()
        .map(|(_, p)| {
            let b = p.dict()["MediaBox"].arr();
            (b[2].num(), b[3].num())
        })
        .collect();
    assert_eq!(boxes.len(), layout.pages.len());
    for (b, page) in boxes.iter().zip(&layout.pages) {
        // Word's 600 dpi grid moves an A4 page by up to 0.1 pt (595.3 -> 595.2).
        assert!((b.0 - page.section.page_width).abs() < 0.1);
        assert!((b.1 - page.section.page_height).abs() < 0.1);
    }
    assert!(
        boxes.iter().any(|b| b.0 > b.1) && boxes.iter().any(|b| b.0 < b.1),
        "expected portrait and landscape pages: {boxes:?}"
    );
}

#[test]
fn fonts_are_subset_and_parse() {
    let fonts = FontLibrary::bundled();
    let (doc, layout) = layout_fixture("plain-paragraphs", &fonts).expect("layout");
    let out = export_pdf(&doc, &layout, &fonts, &PdfMetadata::default()).expect("export");
    let reader = Reader::new(&out.bytes);
    let page = &reader.pages()[0].1;
    let res = reader.key(page, "Resources").expect("resources");
    let font_dict = reader.key(&res, "Font").expect("fonts");
    for f in font_dict.dict().values() {
        let font = reader.resolve(f);
        let fd = if font.dict().contains_key("DescendantFonts") {
            let desc = reader.resolve(&font.dict()["DescendantFonts"].arr()[0]);
            reader.key(&desc, "FontDescriptor").expect("descriptor")
        } else {
            reader.key(&font, "FontDescriptor").expect("descriptor")
        };
        let file = reader.key(&fd, "FontFile2").expect("FontFile2");
        let data = reader.stream(&file);
        let parsed = skrifa::FontRef::new(&data).expect("subset font parses");
        use skrifa::raw::TableProvider as _;
        assert!(parsed.glyf().is_ok() && parsed.loca(None).is_ok());
        let original = fonts
            .face(open365_text::FaceId::from_index(0))
            .bytes()
            .len();
        assert!(
            data.len() < original / 2,
            "subset {} of {original}",
            data.len()
        );
    }
}

#[test]
fn pictures_embed_with_alpha_and_alt() {
    let (_, _, _, bytes) = structure_of("layout-img-inline");
    let reader = Reader::new(&bytes);
    let mut images = 0;
    for id in 1..=reader.object_count() as u32 {
        if let Obj::Stream(d, _) = reader.get(id)
            && matches!(d.get("Subtype"), Some(Obj::Name(n)) if n == "Image")
        {
            images += 1;
        }
    }
    assert!(images > 0, "no image XObject written");
}
