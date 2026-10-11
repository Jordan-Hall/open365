//! Frozen panes from the file: the frozen rows and columns keep their place while the rest
//! scrolls, hit testing follows, and Ctrl+Home lands below and right of the freeze.

#![allow(clippy::unwrap_used)]

use open365_preview::sheet::SheetModel;
use open365_preview::{AppKind, Preview};
use open365_ui_core::{Key, Modifiers};

/// `grid-geometry.xlsx` freezes two rows and two columns.
fn frozen() -> Preview {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/xlsx/grid-geometry.xlsx");
    let mut p = Preview::new(AppKind::Sheets, false, 1600, 1000, 1.25);
    let model = SheetModel::open(&path).unwrap();
    p.shell.set_sheet(&mut p.ui, model);
    p
}

#[test]
fn the_file_freezes_two_rows_and_two_columns() {
    let p = frozen();
    assert_eq!(p.shell.sheet().frozen(), (2, 2));
    // Scrolling starts after the freeze.
    assert_eq!(p.shell.sheet().top_left, (3, 3));
}

#[test]
fn frozen_cells_stay_while_the_others_scroll() {
    let mut p = frozen();
    let a1 = p.shell.cell_centre(1, 1);
    let b2 = p.shell.cell_centre(2, 2);
    p.shell.grid_wheel(&mut p.ui, -3.0, false);
    p.shell.grid_wheel(&mut p.ui, -1.0, true);
    assert_eq!(p.shell.sheet().top_left, (12, 6));
    assert_eq!(p.shell.cell_centre(1, 1), a1, "A1 is frozen");
    assert_eq!(p.shell.cell_centre(2, 2), b2, "B2 is frozen");
    // The first scrolling cell sits beside the frozen block.
    assert!(p.shell.cell_centre(12, 6).x > b2.x);
    assert!(p.shell.cell_centre(12, 6).y > b2.y);
    // A frozen row follows the scrolled column sideways but keeps its height.
    let f1 = p.shell.cell_centre(1, 6);
    assert_eq!(f1.y, a1.y);
    assert_eq!(f1.x, p.shell.cell_centre(12, 6).x);
    // Scrolling never goes into the frozen block.
    p.shell.grid_wheel(&mut p.ui, 100.0, false);
    p.shell.grid_wheel(&mut p.ui, 100.0, true);
    assert_eq!(p.shell.sheet().top_left, (3, 3));
}

#[test]
fn clicking_a_frozen_cell_selects_it_even_when_scrolled() {
    let mut p = frozen();
    p.shell.grid_wheel(&mut p.ui, -4.0, false);
    let at = p.shell.cell_centre(2, 1);
    p.click(at.x, at.y);
    assert_eq!(p.shell.sheet().active, (2, 1));
    let at = p.shell.cell_centre(p.shell.sheet().top_left.0, 2);
    p.click(at.x, at.y);
    assert_eq!(p.shell.sheet().active, (p.shell.sheet().top_left.0, 2));
}

#[test]
fn ctrl_home_goes_below_and_right_of_the_freeze() {
    let mut p = frozen();
    p.shell.sheet_mut().select_cell((20, 7));
    p.shell.grid_wheel(&mut p.ui, -2.0, false);
    let at = p.shell.cell_centre(p.shell.sheet().top_left.0, 3);
    p.click(at.x, at.y);
    p.input(open365_ui_core::InputEvent::Key {
        key: Key::Home,
        pressed: true,
        modifiers: Modifiers {
            ctrl: true,
            ..Modifiers::default()
        },
    });
    assert_eq!(p.shell.sheet().active, (3, 3));
    assert_eq!(p.shell.sheet().top_left, (3, 3));
}

#[test]
fn the_active_cell_scrolls_into_view_beside_the_freeze() {
    let mut p = frozen();
    p.shell.sheet_mut().select_cell((60, 3));
    p.shell.set_sheet(&mut p.ui, {
        let mut m = SheetModel::open(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/xlsx/grid-geometry.xlsx"),
        )
        .unwrap();
        m.select_cell((60, 4));
        m
    });
    let (r, c) = p.shell.sheet().active;
    let at = p.shell.cell_centre(r, c);
    let frozen_right = p.shell.cell_centre(2, 2).x;
    assert!(at.x > frozen_right, "{at:?} is under the frozen columns");
    assert!(p.shell.sheet().top_left.0 > 3, "scrolled down to row 60");
}
