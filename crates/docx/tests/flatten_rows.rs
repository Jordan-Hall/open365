//! Row-level content controls (`w:sdt` around `w:tc`): Word counts a start and an end character
//! for each (`layout-sdt`, corpus template `ad265748`), so the flattened view carries a start
//! marker on the first paragraph of the first wrapped cell and an end marker on the last
//! paragraph of the last one.

mod common;

use open365_docx::{
    Block, Cell, Document, Inline, RowChild, TableChild, flatten_content_controls, is_sdt_marker,
};
use open365_opc::Limits;

fn cell(text: &str) -> String {
    format!("<w:tc><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>")
}

fn open(row: &str) -> Document {
    let body = format!("<w:tbl><w:tblGrid/><w:tr>{row}</w:tr></w:tbl><w:p/>");
    Document::open(&common::docx_with_body(&body), &Limits::default()).expect("opens")
}

fn cells(doc: &Document) -> Vec<Cell> {
    let flat = flatten_content_controls(&doc.body().blocks).expect("has a control");
    let Some(Block::Table(table)) = flat.blocks.first() else {
        panic!("table first");
    };
    let row = table
        .children
        .iter()
        .find_map(|c| match c {
            TableChild::Row(r) => Some(r),
            TableChild::Raw(_) => None,
        })
        .expect("a row");
    row.children
        .iter()
        .filter_map(|c| match c {
            RowChild::Cell(c) => Some(c.clone()),
            RowChild::Raw(_) => None,
        })
        .collect()
}

/// Markers of every top-level paragraph of a cell: (start, end) counts.
fn marks(cell: &Cell) -> Vec<(usize, usize)> {
    cell.blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph(p) => {
                let markers: Vec<_> = p
                    .content
                    .iter()
                    .filter(|i| is_sdt_marker(i))
                    .map(|i| match i {
                        Inline::Raw(open365_docx::RawXml::Element(e)) => e.head.is_w("sdtStart"),
                        _ => unreachable!(),
                    })
                    .collect();
                Some((
                    markers.iter().filter(|s| **s).count(),
                    markers.iter().filter(|s| !**s).count(),
                ))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn row_control_around_one_cell_marks_start_and_end() {
    let doc = open(&format!(
        "{}<w:sdt><w:sdtPr/><w:sdtContent>{}</w:sdtContent></w:sdt>{}",
        cell("a"),
        "<w:tc><w:p><w:r><w:t>b</w:t></w:r></w:p><w:p><w:r><w:t>c</w:t></w:r></w:p></w:tc>",
        cell("d")
    ));
    let cells = cells(&doc);
    assert_eq!(cells.len(), 3);
    assert_eq!(marks(&cells[0]), vec![(0, 0)]);
    assert_eq!(marks(&cells[1]), vec![(1, 0), (0, 1)]);
    assert_eq!(marks(&cells[2]), vec![(0, 0)]);
}

#[test]
fn row_control_around_two_cells_spans_them() {
    let doc = open(&format!(
        "<w:sdt><w:sdtPr/><w:sdtContent>{}{}</w:sdtContent></w:sdt>{}",
        cell("a"),
        cell("b"),
        cell("c")
    ));
    let cells = cells(&doc);
    assert_eq!(cells.len(), 3);
    assert_eq!(marks(&cells[0]), vec![(1, 0)]);
    assert_eq!(marks(&cells[1]), vec![(0, 1)]);
    assert_eq!(marks(&cells[2]), vec![(0, 0)]);
}
