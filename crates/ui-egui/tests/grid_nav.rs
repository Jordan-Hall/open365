//! Keyboard navigation in the grid: Ctrl+arrow against Excel's `Range.End`
//! (`fixtures/reference/xlsx/grid-nav/nav.json`), the active cell inside a range or a whole
//! column or row, Enter and Tab through a selection, Home, End and the Page keys, and the
//! reach of the sheet to XFD1048576.
//!
//! Excel's Enter, Tab and Page rules cannot be measured through COM (key events do not reach a
//! hidden instance), so those tests pin the documented behaviour instead.

use std::path::PathBuf;

use egui::accesskit::Role;
use egui::{Key, Modifiers, Rect, vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::{Harness, Node};
use open365_ui_egui::App;
use open365_xlsx::{MAX_COL, MAX_ROW};

fn harness(name: &str) -> Harness<'static, App> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/xlsx")
        .join(format!("{name}.xlsx"));
    let app = App::new(Some(path));
    let mut h = Harness::builder()
        .with_size(vec2(1100.0, 800.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run();
    h.run();
    h
}

fn cell<'a>(h: &'a Harness<'_, App>, row: u32, col: u32) -> Option<Node<'a>> {
    h.query_all_by_role(Role::Cell).find(|c| {
        let d = c.accesskit_node().data().clone();
        d.row_index() == Some(row as usize - 1) && d.column_index() == Some(col as usize - 1)
    })
}

fn rect_of(h: &Harness<'_, App>, row: u32, col: u32) -> Rect {
    cell(h, row, col)
        .unwrap_or_else(|| panic!("cell {row},{col} is not published"))
        .rect()
}

fn press(h: &mut Harness<'_, App>, modifiers: Modifiers, key: Key) {
    h.key_press_modifiers(modifiers, key);
    h.run();
}

fn nav_json() -> serde_json::Value {
    serde_json::from_slice(
        &std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/reference/xlsx/grid-nav/nav.json"),
        )
        .expect("reference file"),
    )
    .expect("reference json")
}

/// Every start cell Excel's `End` was asked about: our jump lands where Excel's does. Covers
/// runs, gaps, hidden rows and columns (inside and at the end of a block), a merged cell, a
/// formatted-only cell, a formula that returns an empty string, and the sheet edges.
#[test]
fn ctrl_arrow_jumps_where_excel_end_does() {
    let h = harness("grid-nav");
    let target = h.state().target().expect("target");
    let json = nav_json();
    let ends = json["ends"].as_object().unwrap();
    let mut checked = 0;
    for (start, ends) in ends {
        let (r, c) = start.split_once(',').unwrap();
        let (r, c): (u32, u32) = (r.parse().unwrap(), c.parse().unwrap());
        let e: Vec<u32> = ends
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u32)
            .collect();
        let grid = &h.state().grid;
        assert_eq!(
            grid.ctrl_arrow_target(target, (r, c), -1, 0),
            (e[0], c),
            "up from {start}"
        );
        assert_eq!(
            grid.ctrl_arrow_target(target, (r, c), 1, 0),
            (e[1], c),
            "down from {start}"
        );
        assert_eq!(
            grid.ctrl_arrow_target(target, (r, c), 0, -1),
            (r, e[2]),
            "left from {start}"
        );
        assert_eq!(
            grid.ctrl_arrow_target(target, (r, c), 0, 1),
            (r, e[3]),
            "right from {start}"
        );
        checked += 4;
    }
    assert!(checked > 3000, "checked {checked}");
}

#[test]
fn ctrl_arrow_keys_move_the_active_cell_and_shift_extends_from_the_moving_corner() {
    let mut h = harness("grid-nav");
    h.state_mut().grid.set_active(1, 1);
    h.run();
    press(&mut h, Modifiers::COMMAND, Key::ArrowDown);
    assert_eq!(h.state().grid.active(), (3, 1));
    press(&mut h, Modifiers::COMMAND, Key::ArrowDown);
    assert_eq!(
        h.state().grid.active(),
        (6, 1),
        "from the end of a run to the next value"
    );
    press(&mut h, Modifiers::COMMAND, Key::ArrowUp);
    assert_eq!(h.state().grid.active(), (3, 1));
    press(
        &mut h,
        Modifiers::COMMAND | Modifiers::SHIFT,
        Key::ArrowDown,
    );
    assert_eq!(h.state().grid.active(), (3, 1));
    assert_eq!(h.state().grid.selection(), (3, 1, 6, 1));
    press(
        &mut h,
        Modifiers::COMMAND | Modifiers::SHIFT,
        Key::ArrowDown,
    );
    assert_eq!(h.state().grid.selection(), (3, 1, 8, 1));
    // Past the last value the jump runs to the end of the sheet.
    h.state_mut().grid.set_active(52, 1);
    h.run();
    press(&mut h, Modifiers::COMMAND, Key::ArrowDown);
    assert_eq!(h.state().grid.active(), (MAX_ROW, 1));
    press(&mut h, Modifiers::COMMAND, Key::ArrowRight);
    assert_eq!(h.state().grid.active(), (MAX_ROW, MAX_COL));
}

#[test]
fn ctrl_home_ctrl_end_and_home() {
    let mut h = harness("grid-nav");
    h.state_mut().grid.set_active(20, 5);
    h.run();
    press(&mut h, Modifiers::NONE, Key::Home);
    assert_eq!(h.state().grid.active(), (20, 1));
    press(&mut h, Modifiers::COMMAND, Key::End);
    // The used range of the Nav sheet ends at M60.
    assert_eq!(h.state().grid.active(), (60, 13));
    press(&mut h, Modifiers::COMMAND, Key::Home);
    assert_eq!(h.state().grid.active(), (1, 1));
    press(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::End);
    assert_eq!(h.state().grid.selection(), (1, 1, 60, 13));
    press(&mut h, Modifiers::SHIFT, Key::Home);
    assert_eq!(h.state().grid.selection(), (1, 1, 60, 1));
}

#[test]
fn ctrl_home_goes_below_and_right_of_frozen_panes() {
    let mut h = harness("grid-nav");
    h.state_mut().grid.select_sheet(1);
    h.run();
    h.state_mut().grid.set_active(8, 5);
    h.run();
    press(&mut h, Modifiers::COMMAND, Key::Home);
    // Two rows and two columns are frozen.
    assert_eq!(h.state().grid.active(), (3, 3));
}

#[test]
fn enter_and_tab_move_through_a_range_and_wrap() {
    let mut h = harness("formulas-basic");
    h.state_mut().grid.select_range((2, 2), (3, 3));
    h.run();
    // Tab goes along the row, then to the next row, then wraps; the range stays.
    for want in [(2, 3), (3, 2), (3, 3), (2, 2)] {
        press(&mut h, Modifiers::NONE, Key::Tab);
        assert_eq!(h.state().grid.active(), want);
        assert_eq!(h.state().grid.selection(), (2, 2, 3, 3));
    }
    // Enter goes down the column, then to the next column.
    for want in [(3, 2), (2, 3), (3, 3), (2, 2)] {
        press(&mut h, Modifiers::NONE, Key::Enter);
        assert_eq!(h.state().grid.active(), want);
    }
    // Shift reverses.
    press(&mut h, Modifiers::SHIFT, Key::Enter);
    assert_eq!(h.state().grid.active(), (3, 3));
    press(&mut h, Modifiers::SHIFT, Key::Tab);
    assert_eq!(h.state().grid.active(), (3, 2));
    assert_eq!(h.state().grid.selection(), (2, 2, 3, 3));
    // A single cell: Enter moves down, Tab right, and the selection follows.
    h.state_mut().grid.set_active(5, 1);
    h.run();
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(h.state().grid.selection(), (6, 1, 6, 1));
    press(&mut h, Modifiers::NONE, Key::Tab);
    assert_eq!(h.state().grid.selection(), (6, 2, 6, 2));
}

#[test]
fn the_active_cell_sits_anywhere_in_a_whole_column_or_row_selection() {
    let mut h = harness("formulas-basic");
    h.state_mut().grid.set_active(3, 2);
    h.run();
    h.state_mut().grid.select_columns(2, 3);
    h.run();
    assert_eq!(h.state().grid.active(), (3, 2));
    assert_eq!(h.state().grid.selection(), (1, 2, MAX_ROW, 3));
    // Enter walks down column B inside the selection.
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(h.state().grid.active(), (4, 2));
    // Tab crosses to column C, then wraps to B of the next row.
    press(&mut h, Modifiers::NONE, Key::Tab);
    assert_eq!(h.state().grid.active(), (4, 3));
    press(&mut h, Modifiers::NONE, Key::Tab);
    assert_eq!(h.state().grid.active(), (5, 2));
    assert_eq!(h.state().grid.selection(), (1, 2, MAX_ROW, 3));
    // Enter at the bottom of the sheet wraps to the top of the next column.
    h.state_mut().grid.set_active(MAX_ROW, 2);
    h.run();
    h.key_press_modifiers(Modifiers::CTRL, Key::Space);
    h.run();
    press(&mut h, Modifiers::SHIFT, Key::ArrowRight);
    assert_eq!(h.state().grid.selection(), (1, 2, MAX_ROW, 3));
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(h.state().grid.active(), (1, 3));
    assert_eq!(h.state().grid.selection(), (1, 2, MAX_ROW, 3));
    // Whole rows: Enter walks down the rows, then wraps to the next column.
    h.state_mut().grid.set_active(2, 4);
    h.run();
    h.state_mut().grid.select_rows(2, 3);
    h.run();
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(h.state().grid.active(), (3, 4));
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(h.state().grid.active(), (2, 5));
    assert_eq!(h.state().grid.selection(), (2, 1, 3, MAX_COL));
}

#[test]
fn shift_arrow_after_tab_grows_the_range_from_the_active_cell() {
    let mut h = harness("formulas-basic");
    h.state_mut().grid.select_range((2, 2), (4, 4));
    h.run();
    press(&mut h, Modifiers::NONE, Key::Tab);
    assert_eq!(h.state().grid.active(), (2, 3));
    press(&mut h, Modifiers::SHIFT, Key::ArrowDown);
    assert_eq!(h.state().grid.selection(), (2, 3, 3, 3));
}

#[test]
fn enter_skips_hidden_rows_inside_a_range() {
    let mut h = harness("grid-geometry");
    // Row 8 is hidden.
    h.state_mut().grid.select_range((6, 1), (9, 1));
    h.run();
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(h.state().grid.active(), (7, 1));
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(h.state().grid.active(), (9, 1));
}

/// The grid is the whole sheet: the last cell is reachable, drawn and exact to the pixel even
/// though it lies 21 million pixels from the top.
#[test]
fn the_sheet_scrolls_to_xfd1048576() {
    let mut h = harness("formulas-basic");
    h.state_mut().grid.set_active(MAX_ROW, MAX_COL);
    h.run();
    h.run();
    let r = rect_of(&h, MAX_ROW, MAX_COL);
    // Rows of this sheet are 19 px (14.25 pt); the last one is exact, not off by rounding.
    assert_eq!(
        r.height(),
        19.0,
        "row height is exact at the end of the sheet"
    );
    assert!(r.max.x <= 1100.0 && r.max.y <= 800.0, "{r:?} is off screen");
    let above = rect_of(&h, MAX_ROW - 1, MAX_COL);
    assert_eq!(above.max.y, r.min.y);
    assert_eq!(above.height(), 19.0);
    let left = rect_of(&h, MAX_ROW, MAX_COL - 1);
    assert_eq!(left.max.x, r.min.x);
    // Back to the top.
    press(&mut h, Modifiers::COMMAND, Key::Home);
    h.run();
    assert!(cell(&h, 1, 1).is_some());
    assert!(cell(&h, MAX_ROW, MAX_COL).is_none());
}

#[test]
fn page_down_and_up_move_a_screen_and_keep_the_cell_in_place_on_screen() {
    let mut h = harness("large-sheet");
    h.state_mut().grid.set_active(5, 2);
    h.run();
    h.run();
    let before = rect_of(&h, 5, 2);
    press(&mut h, Modifiers::NONE, Key::PageDown);
    h.run();
    let (row, col) = h.state().grid.active();
    assert_eq!(col, 2);
    assert!(row > 20 && row < 60, "moved to row {row}");
    let after = rect_of(&h, row, col);
    assert_eq!(
        after.min.y, before.min.y,
        "the cell keeps its place on screen"
    );
    press(&mut h, Modifiers::NONE, Key::PageUp);
    h.run();
    assert_eq!(h.state().grid.active(), (5, 2));
    assert_eq!(rect_of(&h, 5, 2).min.y, before.min.y);
    // Shift+Page Down extends the selection by a screen and keeps the active cell.
    press(&mut h, Modifiers::SHIFT, Key::PageDown);
    let sel = h.state().grid.selection();
    assert_eq!(h.state().grid.active(), (5, 2));
    assert_eq!((sel.0, sel.1, sel.3), (5, 2, 2));
    assert!(sel.2 > 20);
    // Page Up at the top stays on row 1.
    h.state_mut().grid.set_active(3, 2);
    h.run();
    press(&mut h, Modifiers::NONE, Key::PageUp);
    assert_eq!(h.state().grid.active().0, 1);
}
