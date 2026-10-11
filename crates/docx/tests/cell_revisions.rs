#![allow(clippy::unwrap_used, clippy::type_complexity)]
//! Cell-level revisions (`w:cellIns`, `w:cellDel`, `w:cellMerge`) at the edges of table rows.
//!
//! The fixtures `rev-cell-edge-*` are written by `tools/office/make-revision-refs.ps1`-style
//! XML (authors "Author A" and "Author B"); `tests/revisions.rs` compares every accept and reject
//! state with what Word 16 reports for each (`fixtures/reference/docx/rev-cell-edge-*`). These
//! tests state the expected outcome in plain terms so a regression reads clearly.
//! Word 16 itself never writes these elements: with Track Changes on it records cell merges,
//! deletions and splits as untracked edits and cell insertions as `w:tcPrChange` only.

use open365_docx::{Block, Document, RevisionKind, RowChild, TableChild, VerticalMerge};
use open365_opc::Limits;

fn open(name: &str) -> Document {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/docx")
            .join(format!("{name}.docx")),
    )
    .expect("fixture");
    Document::open(&bytes, &Limits::default()).unwrap()
}

/// Per row, the markup text of each cell.
fn rows(doc: &Document) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for b in &doc.body().blocks {
        let Block::Table(t) = b else { continue };
        for child in &t.children {
            let TableChild::Row(row) = child else {
                continue;
            };
            out.push(
                row.children
                    .iter()
                    .filter_map(|rc| match rc {
                        RowChild::Cell(c) => Some(
                            c.blocks
                                .iter()
                                .filter_map(|b| match b {
                                    Block::Paragraph(p) => Some(p.markup_text()),
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join("\n"),
                        ),
                        RowChild::Raw(_) => None,
                    })
                    .collect(),
            );
        }
    }
    out
}

fn kinds(doc: &Document) -> Vec<RevisionKind> {
    doc.revisions().iter().map(|r| r.kind).collect()
}

fn row(cells: &[&str]) -> Vec<String> {
    cells.iter().map(|s| (*s).to_owned()).collect()
}

fn both(name: &str) -> (Vec<Vec<String>>, Vec<Vec<String>>) {
    let mut a = open(name);
    assert!(a.accept_all_revisions() > 0, "{name}");
    let mut r = open(name);
    assert!(r.reject_all_revisions() > 0, "{name}");
    assert!(
        a.revisions().is_empty() && r.revisions().is_empty(),
        "{name}"
    );
    (rows(&a), rows(&r))
}

#[test]
fn inserted_cells_at_the_row_edges() {
    let cases: [(&str, [&[&str]; 2], [&[&str]; 2]); 4] = [
        (
            "rev-cell-edge-ins-first",
            [&["A1 new", "B1", "C1"], &["A2", "B2", "C2"]],
            [&["B1", "C1"], &["A2", "B2", "C2"]],
        ),
        (
            "rev-cell-edge-ins-last",
            [&["A1", "B1", "C1 new"], &["A2", "B2", "C2"]],
            [&["A1", "B1"], &["A2", "B2", "C2"]],
        ),
        (
            "rev-cell-edge-ins-two",
            [&["A1", "B1 new", "C1 new"], &["A2 new", "B2", "C2"]],
            [&["A1"], &["B2", "C2"]],
        ),
        (
            "rev-cell-edge-ins-allrow",
            [&["A1", "B1", "C1"], &["A2 new", "B2 new", "C2 new"]],
            [&["A1", "B1", "C1"], &[]],
        ),
    ];
    for (name, accepted, rejected) in cases {
        let doc = open(name);
        assert!(kinds(&doc).contains(&RevisionKind::CellInserted), "{name}");
        let (a, r) = both(name);
        let want = |x: [&[&str]; 2]| -> Vec<Vec<String>> { x.iter().map(|c| row(c)).collect() };
        assert_eq!(a, want(accepted), "{name} accepted");
        assert_eq!(r, want(rejected), "{name} rejected");
    }
}

#[test]
fn deleted_cells_at_the_row_edges() {
    let cases: [(&str, [&[&str]; 2], [&[&str]; 2]); 4] = [
        (
            "rev-cell-edge-del-first",
            [&["B1", "C1"], &["A2", "B2", "C2"]],
            [&["A1 old", "B1", "C1"], &["A2", "B2", "C2"]],
        ),
        (
            "rev-cell-edge-del-last",
            [&["A1", "B1"], &["A2", "B2", "C2"]],
            [&["A1", "B1", "C1 old"], &["A2", "B2", "C2"]],
        ),
        (
            "rev-cell-edge-del-two",
            [&["A1"], &["A2", "B2", "C2"]],
            [&["A1", "B1 old", "C1 old"], &["A2", "B2", "C2"]],
        ),
        (
            "rev-cell-edge-del-allrow",
            [&["A1", "B1", "C1"], &[]],
            [&["A1", "B1", "C1"], &["A2 old", "B2 old", "C2 old"]],
        ),
    ];
    for (name, accepted, rejected) in cases {
        let doc = open(name);
        assert!(kinds(&doc).contains(&RevisionKind::CellDeleted), "{name}");
        let (a, r) = both(name);
        let want = |x: [&[&str]; 2]| -> Vec<Vec<String>> { x.iter().map(|c| row(c)).collect() };
        assert_eq!(a, want(accepted), "{name} accepted");
        assert_eq!(r, want(rejected), "{name} rejected");
    }
}

#[test]
fn cell_revisions_keep_their_author() {
    let doc = open("rev-cell-edge-ins-b-author");
    let authors: Vec<_> = doc
        .revisions()
        .iter()
        .filter(|r| r.kind == RevisionKind::CellInserted)
        .map(|r| r.author.clone())
        .collect();
    assert_eq!(authors, ["Author B"]);
}

/// Whether the first cell of each row continues a vertical merge.
fn first_cell_continues(doc: &Document) -> Vec<bool> {
    let mut out = Vec::new();
    for b in &doc.body().blocks {
        let Block::Table(t) = b else { continue };
        for child in &t.children {
            let TableChild::Row(row) = child else {
                continue;
            };
            let first = row.children.iter().find_map(|rc| match rc {
                RowChild::Cell(c) => Some(c.properties().v_merge == VerticalMerge::Continue),
                RowChild::Raw(_) => None,
            });
            out.push(first.unwrap_or(false));
        }
    }
    out
}

#[test]
fn cell_merges_resolve_the_vertical_merge() {
    // (fixture, first-cell continuation per row after accept, after reject). Word: a
    // `w:cellMerge` with vMerge="rest" accepted splits the cell; vMerge="cont" accepted joins it.
    let cases: [(&str, [bool; 2], [bool; 2]); 3] = [
        (
            "rev-cell-edge-merge-b-author",
            [false, false],
            [false, true],
        ),
        // Rejecting also removes the inserted cell, which widens the continuation cell: Word
        // then shows it as a cell of its own (reference `reject` in review.tsv).
        (
            "rev-cell-edge-merge-ins-row",
            [false, false],
            [false, false],
        ),
        (
            "rev-cell-edge-merge-last-col",
            [false, false],
            [false, false],
        ),
    ];
    for (name, accepted, rejected) in cases {
        let doc = open(name);
        assert!(kinds(&doc).contains(&RevisionKind::CellMerged), "{name}");
        let mut a = open(name);
        a.accept_all_revisions();
        let mut r = open(name);
        r.reject_all_revisions();
        for d in [&a, &r] {
            assert!(!kinds(d).contains(&RevisionKind::CellMerged), "{name}");
            let bytes = d.serialize_main();
            assert!(
                !String::from_utf8_lossy(&bytes).contains("cellMerge"),
                "{name}"
            );
        }
        let (fa, fr) = (first_cell_continues(&a), first_cell_continues(&r));
        assert_eq!(fa, accepted, "{name} accepted");
        assert_eq!(fr, rejected, "{name} rejected");
    }
}
