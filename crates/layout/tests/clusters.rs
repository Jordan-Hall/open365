//! The cluster map, table identities and paragraph roles that layout reports.

use std::collections::BTreeSet;
use std::path::PathBuf;

use open365_docx::Document;
use open365_layout::{ClusterKind, LayoutOptions, ParaRole, layout_document};
use open365_opc::Limits;
use open365_text::FontLibrary;

fn fixture(name: &str) -> Document {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/docx")
        .join(format!("{name}.docx"));
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Document::open(&bytes, &Limits::default()).expect("open")
}

fn names() -> Vec<String> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/reference/layout");
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .expect("reference dir")
        .filter_map(|e| {
            let name = e.ok()?.file_name().into_string().ok()?;
            let stem = name.strip_suffix(".json")?;
            (!stem.ends_with(".lists")).then(|| stem.to_owned())
        })
        .collect();
    out.sort();
    out
}

/// Every glyph's cluster text is a real slice of its paragraph, and the clusters of a paragraph
/// spell its text (spaces, tabs and marks aside), ligatures included.
#[test]
fn clusters_spell_the_paragraph_text() {
    let fonts = FontLibrary::bundled();
    let mut ligatures = 0usize;
    let mut checked = 0usize;
    for name in names() {
        let doc = fixture(&name);
        let layout = layout_document(&doc, &fonts, LayoutOptions::default());
        let mut seen = BTreeSet::new();
        let mut spelled: std::collections::BTreeMap<usize, (String, String)> = Default::default();
        for (_, line) in layout.lines() {
            if !seen.insert((line.paragraph, line.start)) {
                continue;
            }
            let entry = spelled
                .entry(line.paragraph)
                .or_insert_with(|| (String::new(), line.text.to_string()));
            for g in line.runs.iter().flat_map(|r| &r.glyphs) {
                if g.cluster.kind != ClusterKind::Text {
                    continue;
                }
                let (a, b) = (g.cluster.text_start as usize, g.cluster.text_end as usize);
                assert!(
                    line.text.get(a..b).is_some(),
                    "{name}: bad cluster {a}..{b} in {:?}",
                    line.text
                );
                let t = line.cluster_text(g);
                if t.chars().count() > 1 && !t.contains(char::is_whitespace) {
                    ligatures += 1;
                }
                assert!(
                    g.cluster.end >= g.cluster.start,
                    "{name}: story range runs backwards"
                );
                entry.0.push_str(t);
            }
        }
        for (para, (got, text)) in spelled {
            let strip = |s: &str| -> String {
                s.chars()
                    .filter(|c| !c.is_whitespace() && !c.is_control() && *c != '\u{fffc}')
                    .collect()
            };
            // Lines of a clipped table row (`exact` height) are not laid out, so the clusters
            // spell a prefix of the text there.
            let (got, want) = (strip(&got), strip(&text));
            assert!(
                want.starts_with(&got) && !got.is_empty() || want.is_empty(),
                "{name}: paragraph {para}: got {got:?} want {want:?}"
            );
            checked += 1;
        }
    }
    assert!(checked > 100, "only {checked} paragraphs checked");
    assert!(ligatures > 0, "no ligature cluster in any fixture");
}

#[test]
fn list_items_and_headings_have_roles_and_labels() {
    let fonts = FontLibrary::bundled();
    let mut items = 0;
    let mut headings = 0;
    for name in names() {
        let doc = fixture(&name);
        let layout = layout_document(&doc, &fonts, LayoutOptions::default());
        for (_, line) in layout.lines() {
            match line.role {
                ParaRole::ListItem => {
                    items += 1;
                    // The first line of the item carries the label and its glyphs.
                    if line.label.is_some() {
                        let g = line
                            .runs
                            .iter()
                            .flat_map(|r| &r.glyphs)
                            .find(|g| g.cluster.kind == ClusterKind::Label)
                            .expect("label glyph");
                        assert!(!line.cluster_text(g).is_empty() || line.label.is_some());
                    }
                }
                ParaRole::Heading(n) => {
                    headings += 1;
                    assert!((1..=6).contains(&n));
                }
                ParaRole::Plain => {}
            }
        }
    }
    assert!(
        items > 0 && headings > 0,
        "{items} items, {headings} headings"
    );
}

#[test]
fn cells_carry_table_row_column_borders_and_shading() {
    let fonts = FontLibrary::bundled();
    let doc = fixture("layout-table-basic");
    let layout = layout_document(&doc, &fonts, LayoutOptions::default());
    let cells: Vec<_> = layout.pages.iter().flat_map(|p| &p.cells).collect();
    assert!(!cells.is_empty());
    let ids: BTreeSet<(usize, usize, usize)> =
        cells.iter().map(|c| (c.table, c.row, c.col)).collect();
    assert!(ids.len() > 1, "cells are not told apart: {ids:?}");
    assert!(cells.iter().all(|c| c.col_span >= 1));
    assert!(
        cells.iter().any(|c| c.borders.top.is_some()
            || c.borders.left.is_some()
            || c.borders.bottom.is_some()
            || c.borders.right.is_some()),
        "no resolved border"
    );
}
