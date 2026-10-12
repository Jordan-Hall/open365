//! The tagged structure of our PDF against Word's own export of the same fixture: catalog
//! language, role map, element sequence (roles) and link annotations. `O365_STRUCT_REPORT=1`
//! prints the differences of every fixture; the tests below pin the ones that are fixed.

mod common;

use std::collections::BTreeMap;

use common::structure::{Structure, read};
use common::{fixture_names, layout_fixture, layout_with_bundled_theme, root};
use open365_pdf::{PdfMetadata, export_pdf};
use open365_text::FontLibrary;

fn word(name: &str) -> Option<Structure> {
    let data = std::fs::read(root().join(format!("fixtures/reference/pdf/{name}.pdf"))).ok()?;
    Some(read(&data))
}

/// Our export of `fixtures/docx/<name>.docx`, or of `fixtures/reference/pdf/probes/<name>.docx`
/// for a name that starts with `probes/`.
fn ours(name: &str, fonts: &FontLibrary) -> Option<Structure> {
    if name.starts_with("probes/") {
        let bytes =
            std::fs::read(root().join(format!("fixtures/reference/pdf/{name}.docx"))).ok()?;
        let doc = open365_docx::Document::open(&bytes, &open365_opc::Limits::default()).ok()?;
        let layout =
            open365_layout::layout_document(&doc, fonts, open365_layout::LayoutOptions::default());
        let out = export_pdf(&doc, &layout, fonts, &PdfMetadata::default()).ok()?;
        return Some(read(&out.bytes));
    }
    let (doc, layout) = match layout_fixture(name, fonts) {
        Some(x) => x,
        None => std::panic::catch_unwind(|| layout_with_bundled_theme(name, fonts)).ok()?,
    };
    let out = export_pdf(&doc, &layout, fonts, &PdfMetadata::default()).expect("export");
    Some(read(&out.bytes))
}

fn probe_names() -> Vec<String> {
    let dir = root().join("fixtures/reference/pdf/probes");
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .expect("probes")
        .filter_map(|e| {
            let n = e.ok()?.file_name().into_string().ok()?;
            Some(format!("probes/{}", n.strip_suffix(".pdf")?))
        })
        .collect();
    v.sort();
    v
}

fn roles(s: &Structure) -> Vec<String> {
    s.elems
        .iter()
        .map(|e| format!("{}{}", "  ".repeat(e.depth), e.role))
        .collect()
}

fn counts(s: &Structure) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for e in &s.elems {
        *m.entry(e.role.clone()).or_default() += 1;
    }
    m
}

/// `O365_STRUCT_DUMP=<name>` prints Word's and our element trees side by side for one fixture.
#[test]
fn dump() {
    let Ok(name) = std::env::var("O365_STRUCT_DUMP") else {
        return;
    };
    let fonts = FontLibrary::bundled();
    let line = |e: &common::structure::Elem| {
        format!(
            "{}{} pg={:?} mc={} or={} {}{}",
            "  ".repeat(e.depth),
            e.role,
            e.page,
            e.mcids,
            e.objrs,
            e.lang.clone().unwrap_or_default(),
            e.attrs.join(",")
        )
    };
    for (who, s) in [("WORD", word(&name)), ("OURS", ours(&name, &fonts))] {
        println!("=== {who}");
        for e in s.iter().flat_map(|s| s.elems.iter()) {
            println!("{}", line(e));
        }
    }
}

#[test]
fn report() {
    if std::env::var("O365_STRUCT_REPORT").is_err() {
        return;
    }
    let fonts = FontLibrary::bundled();
    let only = std::env::var("O365_STRUCT_ONLY").ok();
    let mut totals: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut names = fixture_names();
    names.extend(probe_names());
    for name in names {
        if only.as_ref().is_some_and(|o| *o != name) {
            continue;
        }
        let Some(w) = word(&name) else { continue };
        let Some(o) = ours(&name, &fonts) else {
            continue;
        };
        let mut diffs = Vec::new();
        if w.lang != o.lang {
            diffs.push(format!("Lang word {:?} ours {:?}", w.lang, o.lang));
        }
        if w.role_map != o.role_map {
            diffs.push("RoleMap differs".to_owned());
        }
        let (cw, co) = (counts(&w), counts(&o));
        for k in cw
            .keys()
            .chain(co.keys())
            .collect::<std::collections::BTreeSet<_>>()
        {
            let (a, b) = (
                cw.get(k).copied().unwrap_or(0),
                co.get(k).copied().unwrap_or(0),
            );
            if a != b {
                diffs.push(format!("{k}: word {a} ours {b}"));
                let t = totals.entry(k.clone()).or_default();
                t.0 += a;
                t.1 += b;
            }
        }
        if roles(&w) != roles(&o) && diffs.is_empty() {
            diffs.push("same counts, different order/nesting".to_owned());
        }
        if roles(&w) == roles(&o) {
            let mc = |s: &Structure| s.elems.iter().map(|e| e.mcids).collect::<Vec<_>>();
            let (mw, mo) = (mc(&w), mc(&o));
            let bad = mw.iter().zip(&mo).filter(|(a, b)| a != b).count();
            if bad > 0 {
                diffs.push(format!(
                    "marked content per element differs in {bad}/{} elements (word {} ours {})",
                    mw.len(),
                    mw.iter().sum::<usize>(),
                    mo.iter().sum::<usize>()
                ));
            }
        }
        let (aw, ao) = (w.annots.len(), o.annots.len());
        if aw != ao {
            diffs.push(format!("annots word {aw} ours {ao}"));
        }
        if aw == ao {
            let key = |a: &common::structure::Annot| {
                (
                    a.page,
                    (-a.rect[3] * 10.0) as i64,
                    (a.rect[0] * 10.0) as i64,
                )
            };
            let (mut wa, mut oa): (Vec<_>, Vec<_>) =
                (w.annots.iter().collect(), o.annots.iter().collect());
            wa.sort_by_key(|a| key(a));
            oa.sort_by_key(|a| key(a));
            let bad = wa
                .iter()
                .zip(&oa)
                .filter(|(a, b)| a.rect.iter().zip(&b.rect).any(|(x, y)| (x - y).abs() > 0.6))
                .count();
            if bad > 0 && std::env::var("O365_STRUCT_RECTS").is_ok() {
                for (a, b) in wa.iter().zip(&oa) {
                    println!(
                        "  {name} p{} word {:?} ours {:?}",
                        a.page,
                        a.rect.map(|v| (v * 10.0).round() / 10.0),
                        b.rect.map(|v| (v * 10.0).round() / 10.0)
                    );
                }
            }
            if bad > 0 {
                let (a, b) = wa
                    .iter()
                    .zip(&oa)
                    .find(|(a, b)| a.rect.iter().zip(&b.rect).any(|(x, y)| (x - y).abs() > 0.6))
                    .unwrap();
                diffs.push(format!(
                    "annot rects differ {bad}/{aw}: word {:?} ours {:?}",
                    a.rect, b.rect
                ));
            }
        }
        if w.outline_titles != o.outline_titles {
            diffs.push(format!(
                "outline word {} ours {}",
                w.outline_titles.len(),
                o.outline_titles.len()
            ));
        }
        if !diffs.is_empty() {
            println!("{name}: {}", diffs.join("; "));
        }
    }
    println!("role totals (word, ours) over differing fixtures: {totals:?}");
}

fn compare(name: &str) -> (Structure, Structure) {
    let fonts = FontLibrary::bundled();
    let w = word(name).unwrap_or_else(|| panic!("Word's PDF of {name}"));
    let o = ours(name, &fonts).unwrap_or_else(|| panic!("our PDF of {name}"));
    (w, o)
}

#[test]
fn role_map_and_catalog_language_are_words() {
    for name in [
        "footnotes-endnotes",
        "probes/p-lang-settings",
        "layout-txbx-basic",
        "probes/p-tshd",
    ] {
        let (w, o) = compare(name);
        assert!(w.marked && o.marked, "{name}");
        assert_eq!(w.role_map, o.role_map, "{name}: RoleMap");
        assert_eq!(w.lang, o.lang, "{name}: catalog /Lang");
        assert_eq!(w.lang.as_deref(), Some("en"));
    }
    // A German document: Word still says `en` and spans the German text.
    let (w, o) = compare("probes/p-lang-settings");
    let spans = |s: &Structure| {
        s.elems
            .iter()
            .filter(|e| e.role == "Span")
            .map(|e| e.lang.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(spans(&w), vec![Some("de".to_owned())]);
    assert_eq!(spans(&w), spans(&o));
}

/// Every link annotation of these fixtures has the rectangle Word gives it (3 px of padding on
/// each side, the line box widened by the paragraph's space above and below, contextual spacing
/// honoured), including the one with space before/after, in a table cell and double spaced.
#[test]
fn link_rectangles_equal_words() {
    for name in [
        "probes/p-rect",
        "hyperlink-external",
        "layout-anc-hyperlink",
        "layout-anc-hyperlink-hist0",
        "layout-fn-basic",
        "layout-fn-endnotes",
        "footnotes-endnotes",
    ] {
        let (w, o) = compare(name);
        assert_eq!(w.annots.len(), o.annots.len(), "{name}: link count");
        let key = |a: &common::structure::Annot| {
            (
                a.page,
                (-a.rect[3] * 10.0) as i64,
                (a.rect[0] * 10.0) as i64,
            )
        };
        let (mut wa, mut oa): (Vec<_>, Vec<_>) =
            (w.annots.iter().collect(), o.annots.iter().collect());
        wa.sort_by_key(|a| key(a));
        oa.sort_by_key(|a| key(a));
        for (a, b) in wa.iter().zip(&oa) {
            // x also carries the layout's glyph advances (up to half a point in a table cell).
            let tol = [0.6, 0.05, 0.6, 0.05];
            for (i, t) in tol.iter().enumerate() {
                assert!(
                    (a.rect[i] - b.rect[i]).abs() <= *t,
                    "{name}: Word {:?} ours {:?}",
                    a.rect,
                    b.rect
                );
            }
        }
    }
}

/// The endnote text's own links are found (they were lost to the layout's paragraph numbering).
#[test]
fn links_in_endnote_text_are_found() {
    let (w, o) = compare("layout-anc-hyperlink-hf");
    let links = |s: &Structure| {
        s.annots
            .iter()
            .filter(|a| a.uri.is_some() || a.dest)
            .count()
    };
    assert_eq!(links(&w), links(&o));
}

/// Word tags a table only when it draws a border or a fill; shading alone counts, and a table
/// inside a tagged table is always a table with its rows directly under it.
#[test]
fn only_tables_with_borders_or_fills_are_tables() {
    let count = |s: &Structure, role: &str| s.elems.iter().filter(|e| e.role == role).count();
    for (name, tables) in [
        ("probes/p-tshd", 1),
        ("probes/p-thdr", 0),
        ("probes/p-tcell", 1),
        ("probes/p-tmix", 1),
        ("layout-table-rows", 0),
        ("layout-table-basic", 1),
        ("layout-table-flow", 4),
        ("layout-pbdr-ctx", 0),
    ] {
        let (w, o) = compare(name);
        assert_eq!(count(&w, "Table"), tables, "{name}: Word");
        assert_eq!(count(&o, "Table"), tables, "{name}: ours");
    }
    let (w, o) = compare("probes/p-tnest");
    assert_eq!(count(&w, "Table"), 2);
    assert_eq!(count(&o, "Table"), 2);
    // The inner table has TR directly under Table in both.
    let inner = |s: &Structure| {
        let at = s
            .elems
            .iter()
            .rposition(|e| e.role == "Table")
            .expect("inner table");
        s.elems[at + 1].role.clone()
    };
    assert_eq!(inner(&w), "TR");
    assert_eq!(inner(&o), "TR");
}

/// A link's text is in a `Span` inside the `Link`, which also holds the annotation.
#[test]
fn link_text_is_a_span_inside_the_link() {
    let (w, o) = compare("hyperlink-external");
    let shape = |s: &Structure| {
        let at = s.elems.iter().position(|e| e.role == "Link").expect("link");
        (
            s.elems[at].objrs,
            s.elems[at + 1].role.clone(),
            s.elems[at + 1].mcids,
        )
    };
    assert_eq!(shape(&w), (1, "Span".to_owned(), 1));
    assert_eq!(shape(&w), shape(&o));
}

/// Links in headers and footers are annotations without a structure element in Word's file.
#[test]
fn header_and_footer_links_are_untagged_as_in_words() {
    let (w, o) = compare("probes/probe-links");
    let untagged = |s: &Structure| s.annots.iter().filter(|a| !a.has_struct_parent).count();
    assert_eq!(untagged(&w), 2);
    assert_eq!(untagged(&w), untagged(&o));
}

/// Fixtures whose element tree (roles and nesting) equals Word's: blanks after paragraphs and note
/// marks, adjacent note marks, tables of contents and REF links, the table header rule
/// (`p-thead*`) and a link in a text box.
#[test]
fn element_trees_equal_words_for_the_matched_fixtures() {
    let fonts = FontLibrary::bundled();
    for name in [
        "layout-fn-basic",
        "layout-fn-endnotes",
        "layout-fn-mixed",
        "fields",
        "probes/p-thead",
        "probes/p-thead-nest",
        "probes/p-thead-nest2",
        "probes/p-txbx-link",
        "probes/p-rect",
        "probes/p-tnest",
        "probes/probe-lang",
        "comments",
        "layout-lists-multilevel",
        // Footnotes that go on over a page or into the next column, floating shapes, the
        // order of figures behind the text, cells with an empty clip rectangle, drop caps,
        // text frames, merged cells and the text box probes.
        "layout-fn-long",
        "layout-cols-fn-unequal",
        "layout-float-wrap-tight",
        "layout-float-wrap-behind-front",
        "layout-img-none",
        "layout-trh-small",
        "layout-trh-pad",
        "layout-frame-dropcap",
        "layout-frame-basic",
        "layout-frame-bdr",
        "layout-frame-gap",
        "layout-frame-nobdr",
        "layout-frame-page",
        "table-merged-cells",
        "probes/p-vmerge",
        "probes/p-txbx-link2",
        "probes/p-tabs2",
    ] {
        let (Some(w), Some(o)) = (word(name), ours(name, &fonts)) else {
            panic!("{name}: no reference or no layout");
        };
        assert_eq!(roles(&w), roles(&o), "{name}");
    }
}

/// The decompressed content stream of every page of a PDF.
fn page_streams(data: &[u8]) -> Vec<String> {
    let s = common::scan::Scan::parse(data);
    s.pages()
        .iter()
        .map(|p| {
            s.key(p, "Contents").map_or_else(String::new, |c| {
                String::from_utf8_lossy(&s.stream(&c)).into_owned()
            })
        })
        .collect()
}

fn our_pdf(name: &str, fonts: &FontLibrary) -> Vec<u8> {
    if let Some(probe) = name.strip_prefix("probes/") {
        let bytes =
            std::fs::read(root().join(format!("fixtures/reference/pdf/probes/{probe}.docx")))
                .expect("probe");
        let doc =
            open365_docx::Document::open(&bytes, &open365_opc::Limits::default()).expect("open");
        let layout =
            open365_layout::layout_document(&doc, fonts, open365_layout::LayoutOptions::default());
        return export_pdf(&doc, &layout, fonts, &PdfMetadata::default())
            .expect("export")
            .bytes;
    }
    let (doc, layout) =
        layout_fixture(name, fonts).unwrap_or_else(|| layout_with_bundled_theme(name, fonts));
    export_pdf(&doc, &layout, fonts, &PdfMetadata::default())
        .expect("export")
        .bytes
}

/// Word's blank (a space in a simple font) at each paragraph mark, line break, tab, list label
/// and tab leader end, and its marked-content sequences (one per paragraph, not per line; the
/// header and footer as pagination artifacts), page by page.
#[test]
fn blanks_and_marked_content_equal_words() {
    let fonts = FontLibrary::bundled();
    for name in [
        "probes/p-tabs2",
        "layout-lists-suff",
        "layout-lists-multilevel",
        "layout-lists-style",
        "layout-cols-balance",
        "layout-cols-footnote",
        "layout-hf-basic",
        "layout-fn-basic",
        "layout-fn-long",
        "layout-sect-empty",
        "probes/p-rect",
        "probes/p-txbx-link2",
        "layout-frame-basic",
        "layout-float-wrap-tight",
        "layout-trh-small",
        "layout-trh-pad",
    ] {
        let word_pdf = std::fs::read(root().join(format!("fixtures/reference/pdf/{name}.pdf")))
            .expect("word pdf");
        let (w, o) = (
            page_streams(&word_pdf),
            page_streams(&our_pdf(name, &fonts)),
        );
        assert_eq!(w.len(), o.len(), "{name}: pages");
        for (i, (a, b)) in w.iter().zip(&o).enumerate() {
            let count = |t: &str, pat: &str| t.matches(pat).count();
            assert_eq!(
                count(a, "[( )] TJ"),
                count(b, "[( )] TJ"),
                "{name} p{}: blanks",
                i + 1
            );
            assert_eq!(
                count(a, "MCID"),
                count(b, "MCID"),
                "{name} p{}: MCIDs",
                i + 1
            );
            let squash = |t: &str| t.replace(' ', "");
            for pat in ["/Subtype/Header", "/Subtype/Footer"] {
                assert_eq!(
                    count(&squash(a), pat),
                    count(&squash(b), pat),
                    "{name} p{}: {pat}",
                    i + 1
                );
            }
        }
    }
}
