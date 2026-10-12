//! Paste Special, the copy marker and insert/delete of rows and columns on the Sheets model.

#![allow(clippy::unwrap_used)]

use open365_preview::sheet::{MemoryClipboard, Paste, PasteKind, SheetModel};

fn model() -> SheetModel {
    let mut m = SheetModel::blank();
    m.set_clipboard(MemoryClipboard::default());
    m.enter(1, 1, "2").unwrap();
    m.enter(1, 2, "=A1*3").unwrap();
    m.enter(2, 1, "x").unwrap();
    m
}

fn copy_block(m: &mut SheetModel) {
    m.select_cell((1, 1));
    m.extend_to((2, 2));
    m.copy(false).unwrap();
}

#[test]
fn paste_special_values_replaces_formulas_with_results() {
    let mut m = model();
    copy_block(&mut m);
    m.select_cell((5, 1));
    m.paste_special(PasteKind::Values).unwrap();
    assert_eq!(m.source(5, 2), "6");
    assert_eq!(m.source(6, 1), "x");
}

#[test]
fn paste_special_formulas_shift_and_transpose_swaps_axes() {
    let mut m = model();
    copy_block(&mut m);
    m.select_cell((5, 1));
    m.paste_special(PasteKind::Formulas).unwrap();
    assert_eq!(m.source(5, 2), "=A5*3");
    m.select_cell((9, 1));
    m.paste_special(PasteKind::Transpose).unwrap();
    assert_eq!(m.source(9, 1), "2");
    assert_eq!(m.source(10, 1), "=A9*3", "the formula moved with its block");
    assert_eq!(m.source(9, 2), "x");
}

#[test]
fn clearing_the_copy_forgets_it_and_a_cut_empties_its_source() {
    let mut m = model();
    copy_block(&mut m);
    assert!(m.clear_copied());
    assert_eq!(m.paste_special(PasteKind::Values).unwrap(), Paste::Empty);
    m.select_cell((1, 1));
    m.copy(true).unwrap();
    m.select_cell((7, 7));
    m.paste().unwrap();
    assert_eq!(m.source(7, 7), "2");
    assert_eq!(m.source(1, 1), "");
}

#[test]
fn insert_and_delete_rows_and_columns_follow_the_selection() {
    let mut m = model();
    m.select_cell((2, 1));
    m.insert_selected(true).unwrap();
    assert_eq!(m.source(3, 1), "x");
    assert_eq!(m.source(2, 1), "");
    m.select_cols(2, 2, false);
    m.insert_selected(false).unwrap();
    assert_eq!(m.source(1, 3), "=A1*3");
    m.select_cell((1, 1));
    m.delete_selected(true).unwrap();
    assert_eq!(m.source(2, 1), "x");
    assert!(m.undo());
    assert_eq!(m.source(3, 1), "x");
}
