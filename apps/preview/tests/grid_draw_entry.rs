//! The validation drop-down in a cell, and cell entry through `review_enter` (protection, then
//! data validation).

#![allow(clippy::unwrap_used)]

mod grid_draw_common;
use grid_draw_common::*;
use open365_commands::review_ops::SheetProtection;
use open365_ui_core::Key;

#[test]
fn a_list_cell_shows_an_arrow_and_picking_an_item_enters_it() {
    let mut p = open_fixture("data-validation.xlsx");
    p.shell.sheet_mut().select_cell((1, 1));
    assert_eq!(
        p.shell.sheet().data_validation_list(1, 1),
        ["Red", "Green", "Blue"]
    );
    let _ = p.draw_list();
    let at = in_cell_corner(&p, 1, 1, 6.0, 6.0);
    p.click(at.x, at.y);
    assert!(menu_open(&p), "the list opens");
    assert_eq!(menu_labels(&p), ["Red", "Green", "Blue"]);
    pick(&mut p, "Green");
    assert_eq!(p.shell.sheet().display(1, 1), "Green");
    assert_eq!(p.shell.sheet().active, (1, 1));
}

#[test]
fn a_cell_without_a_list_has_no_arrow() {
    let mut p = open_fixture("data-validation.xlsx");
    p.shell.sheet_mut().select_cell((8, 3));
    let at = in_cell_corner(&p, 8, 3, 6.0, 6.0);
    p.click(at.x, at.y);
    assert!(!menu_open(&p));
}

#[test]
fn typing_in_a_validated_cell_goes_through_review_enter() {
    let mut p = open_fixture("data-validation.xlsx");
    p.shell.sheet_mut().select_cell((2, 1));
    for ch in "Mauve".chars() {
        p.input(open365_ui_core::InputEvent::Text(ch.to_string()));
    }
    p.key(Key::Enter);
    // The rule is a stop alert for anything outside the list: the cell keeps its value.
    let shown = p.shell.sheet().display(2, 1);
    assert_ne!(shown, "Mauve", "the stop alert refuses the entry");
}

#[test]
fn a_locked_cell_on_a_protected_sheet_refuses_typing() {
    let mut p = sheets();
    p.shell.sheet_mut().enter(1, 1, "keep").unwrap();
    p.shell
        .sheet_mut()
        .review_protect_sheet(SheetProtection::default(), None)
        .unwrap();
    p.shell.sheet_mut().select_cell((1, 1));
    for ch in "x".chars() {
        p.input(open365_ui_core::InputEvent::Text(ch.to_string()));
    }
    p.key(Key::Enter);
    assert_eq!(p.shell.sheet().display(1, 1), "keep");
    assert!(!p.shell.is_editing());
    assert!(
        p.shell.note_text().contains("protected"),
        "{}",
        p.shell.note_text()
    );
}
