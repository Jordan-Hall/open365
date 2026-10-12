//! Insert and Delete cells (shift right, down, left, up), Clear Contents of a range and Paste
//! Special with Skip blanks and Transpose, on the Sheets model.

#![allow(clippy::unwrap_used)]

use open365_preview::sheet::{CellShift, MemoryClipboard, PasteSpecialOpts, PasteWhat, SheetModel};

fn model() -> SheetModel {
    let mut m = SheetModel::blank();
    m.set_clipboard(MemoryClipboard::default());
    m
}

fn col(m: &SheetModel, c: u32, rows: u32) -> Vec<String> {
    (1..=rows).map(|r| m.source(r, c)).collect()
}

fn select(m: &mut SheetModel, a: (u32, u32), b: (u32, u32)) {
    m.select_cell(a);
    m.extend_to(b);
}

#[test]
fn insert_cells_down_moves_only_the_selected_columns() {
    let mut m = model();
    for (i, v) in ["1", "2", "3", "4"].iter().enumerate() {
        m.enter(i as u32 + 1, 1, v).unwrap();
        m.enter(i as u32 + 1, 2, &format!("b{i}")).unwrap();
    }
    select(&mut m, (2, 1), (3, 1));
    m.insert_cells(CellShift::Down).unwrap();
    assert_eq!(col(&m, 1, 6), ["1", "", "", "2", "3", "4"]);
    assert_eq!(col(&m, 2, 4), ["b0", "b1", "b2", "b3"], "column B stays");
}

#[test]
fn insert_cells_right_moves_the_rows_cells() {
    let mut m = model();
    m.enter(1, 1, "a").unwrap();
    m.enter(1, 2, "b").unwrap();
    m.enter(2, 1, "c").unwrap();
    m.select_cell((1, 1));
    m.insert_cells(CellShift::Right).unwrap();
    assert_eq!(
        (m.source(1, 1), m.source(1, 2), m.source(1, 3)),
        (String::new(), "a".into(), "b".into())
    );
    assert_eq!(m.source(2, 1), "c", "row 2 stays");
}

#[test]
fn delete_cells_closes_the_gap() {
    let mut m = model();
    for (i, v) in ["a", "b", "c", "d"].iter().enumerate() {
        m.enter(i as u32 + 1, 1, v).unwrap();
    }
    select(&mut m, (2, 1), (3, 1));
    m.delete_cells(CellShift::Up).unwrap();
    assert_eq!(col(&m, 1, 4), ["a", "d", "", ""]);
    m.enter(1, 2, "x").unwrap();
    m.enter(1, 3, "y").unwrap();
    m.select_cell((1, 1));
    m.delete_cells(CellShift::Left).unwrap();
    assert_eq!(
        (m.source(1, 1), m.source(1, 2), m.source(1, 3)),
        ("x".into(), "y".into(), String::new())
    );
}

#[test]
fn a_shift_that_has_nothing_to_move_changes_nothing() {
    let mut m = model();
    m.enter(1, 1, "a").unwrap();
    m.select_cell((5, 5));
    m.insert_cells(CellShift::Down).unwrap();
    m.delete_cells(CellShift::Left).unwrap();
    assert_eq!(m.source(1, 1), "a");
}

#[test]
fn clear_range_empties_the_whole_selection() {
    let mut m = model();
    for r in 1..=3 {
        m.enter(r, 1, "v").unwrap();
        m.enter(r, 2, "w").unwrap();
    }
    select(&mut m, (1, 1), (2, 2));
    m.clear_range().unwrap();
    assert_eq!(col(&m, 1, 3), ["", "", "v"]);
    assert_eq!(col(&m, 2, 3), ["", "", "w"]);
    assert!(m.undo());
    assert_eq!(col(&m, 1, 3), ["v", "v", "v"], "one undo step");
}

#[test]
fn paste_special_skips_blanks_and_transposes() {
    let mut m = model();
    m.enter(1, 1, "x").unwrap();
    m.enter(3, 1, "z").unwrap();
    m.enter(2, 4, "keep").unwrap();
    select(&mut m, (1, 1), (3, 1));
    m.copy(false).unwrap();
    m.select_cell((1, 4));
    let all = PasteSpecialOpts {
        what: PasteWhat::Values,
        skip_blanks: true,
        transpose: false,
    };
    m.paste_special_with(all).unwrap();
    assert_eq!(col(&m, 4, 3), ["x", "keep", "z"]);
    m.select_cell((6, 1));
    let t = PasteSpecialOpts {
        what: PasteWhat::Values,
        skip_blanks: false,
        transpose: true,
    };
    m.paste_special_with(t).unwrap();
    assert_eq!(
        (m.source(6, 1), m.source(6, 2), m.source(6, 3)),
        ("x".into(), String::new(), "z".into())
    );
}

#[test]
fn the_copy_outline_marches_one_pixel_per_frame_and_repeats() {
    use open365_preview::sheet::ants_phase;
    assert_eq!(ants_phase(0, 5.0), 0.0);
    assert_eq!(ants_phase(3, 5.0), 3.0);
    assert_eq!(ants_phase(10, 5.0), 0.0, "a period is two dashes");
    assert_eq!(ants_phase(13, 5.0), 3.0);
}
