//! Merge & Center menu through the command layer: model edit, discarded values, alignment, undo, save.

use std::path::PathBuf;

use open365_commands::{
    CellEntry, Command, MergeMode, RangeRef, Target, merge_discards, merge_plan,
};
use open365_xlsx::{CellValue, HorizontalAlignment};

fn blank() -> Target {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/xlsx/formulas-basic.xlsx");
    Target::open_path(&path).expect("open xlsx")
}

fn range(r1: u32, c1: u32, r2: u32, c2: u32) -> RangeRef {
    RangeRef {
        first_row: r1,
        first_col: c1,
        last_row: r2,
        last_col: c2,
    }
}

fn put(t: &mut Target, row: u32, col: u32, text: &str) {
    t.apply(Command::XlsxSetCellValue {
        sheet: 0,
        row,
        col,
        value: CellEntry::Text(text.into()),
    })
    .expect("set");
}

fn merges(t: &Target) -> Vec<(u32, u32, u32, u32)> {
    t.xlsx()
        .and_then(|w| w.sheet(0))
        .map(open365_xlsx::Worksheet::merged_cells)
        .unwrap_or_default()
}

fn text(t: &Target, row: u32, col: u32) -> Option<String> {
    t.xlsx()
        .and_then(|w| w.cell(0, row, col))
        .and_then(|c| c.value.as_text().map(str::to_owned))
}

fn is_empty(t: &Target, row: u32, col: u32) -> bool {
    t.xlsx()
        .and_then(|w| w.cell(0, row, col))
        .is_none_or(|c| matches!(c.value, CellValue::Empty))
}

fn merge(t: &mut Target, r: RangeRef, mode: MergeMode) {
    t.apply(Command::MergeCells {
        sheet: 0,
        range: r,
        mode,
    })
    .expect("merge");
}

#[test]
fn merge_and_center_keeps_the_top_left_value_and_centres() {
    let mut t = blank();
    // Rows below the fixture's data, so nothing else is disturbed.
    put(&mut t, 40, 1, "keep");
    put(&mut t, 40, 2, "lose");
    let r = range(40, 1, 40, 3);
    let w = t.xlsx().expect("xlsx");
    assert_eq!(
        merge_discards(w, 0, r, MergeMode::MergeCenter).expect("scan"),
        vec![(40, 2)]
    );
    merge(&mut t, r, MergeMode::MergeCenter);
    assert_eq!(merges(&t), vec![(40, 1, 40, 3)]);
    assert_eq!(text(&t, 40, 1).as_deref(), Some("keep"));
    assert!(is_empty(&t, 40, 2));
    let w = t.xlsx().expect("xlsx");
    assert_eq!(
        w.cell_format(0, 40, 1).alignment.horizontal,
        HorizontalAlignment::Center
    );

    // Survives a save and reopen.
    let bytes = t.snapshot_bytes().expect("save");
    let back = Target::open_xlsx(&bytes).expect("reopen");
    assert_eq!(merges(&back), vec![(40, 1, 40, 3)]);

    // One undo brings the value, the format and the merge back.
    t.undo().expect("undo");
    assert!(merges(&t).is_empty());
    assert_eq!(text(&t, 40, 2).as_deref(), Some("lose"));
    assert_ne!(
        t.xlsx()
            .expect("xlsx")
            .cell_format(0, 40, 1)
            .alignment
            .horizontal,
        HorizontalAlignment::Center
    );
    t.redo().expect("redo");
    assert_eq!(merges(&t), vec![(40, 1, 40, 3)]);
    assert!(is_empty(&t, 40, 2));
}

#[test]
fn across_merges_each_row_and_cells_leaves_alignment() {
    let mut t = blank();
    put(&mut t, 50, 1, "a");
    put(&mut t, 51, 1, "b");
    let r = range(50, 1, 52, 2);
    assert_eq!(merge_plan(r, MergeMode::Across).len(), 3);
    merge(&mut t, r, MergeMode::Across);
    assert_eq!(
        merges(&t),
        vec![(50, 1, 50, 2), (51, 1, 51, 2), (52, 1, 52, 2)]
    );
    assert_ne!(
        t.xlsx()
            .expect("xlsx")
            .cell_format(0, 50, 1)
            .alignment
            .horizontal,
        HorizontalAlignment::Center
    );
    // Merge Cells over them replaces the three merges with one.
    merge(&mut t, r, MergeMode::Cells);
    assert_eq!(merges(&t), vec![(50, 1, 52, 2)]);
    assert_eq!(text(&t, 50, 1).as_deref(), Some("a"));
    assert!(is_empty(&t, 51, 1), "only the upper-left value is kept");
}

#[test]
fn unmerge_keeps_the_value_in_the_top_left_cell() {
    let mut t = blank();
    put(&mut t, 60, 2, "x");
    merge(&mut t, range(60, 2, 61, 3), MergeMode::MergeCenter);
    merge(&mut t, range(61, 3, 61, 3), MergeMode::Unmerge);
    assert!(merges(&t).is_empty());
    assert_eq!(text(&t, 60, 2).as_deref(), Some("x"));
    assert_eq!(
        t.xlsx()
            .expect("xlsx")
            .cell_format(0, 60, 2)
            .alignment
            .horizontal,
        HorizontalAlignment::Center,
        "Excel leaves the centring in place"
    );
}

#[test]
fn a_single_cell_or_bad_range_changes_nothing() {
    let mut t = blank();
    merge(&mut t, range(70, 1, 70, 1), MergeMode::Cells);
    assert!(merges(&t).is_empty());
    assert!(
        t.apply(Command::MergeCells {
            sheet: 0,
            range: range(0, 1, 2, 2),
            mode: MergeMode::Cells,
        })
        .is_err()
    );
    assert!(
        t.apply(Command::MergeCells {
            sheet: 0,
            range: range(1, 1, 1_048_576, 3),
            mode: MergeMode::Cells,
        })
        .is_err()
    );
}
