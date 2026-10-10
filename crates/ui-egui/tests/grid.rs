//! Spreadsheet grid: Excel-like geometry (widths, heights, hidden lines), merged cells, frozen
//! panes, range selection and the accessibility nodes, checked through egui_kittest.

use std::path::PathBuf;

use egui::accesskit::Role;
use egui::{Key, Modifiers, Rect, vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::{Harness, Node};
use open365_ui_egui::App;
use open365_xlsx::{CellValue, GridMetrics, SheetGeometry, Workbook};

fn harness(name: &str) -> Harness<'static, App> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/xlsx")
        .join(format!("{name}.xlsx"));
    let app = App::new(Some(path));
    let mut h = Harness::builder()
        .with_size(vec2(1100.0, 800.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run();
    h
}

fn workbook<'a>(h: &'a Harness<'_, App>) -> &'a Workbook {
    h.state()
        .target()
        .and_then(|t| t.xlsx())
        .expect("workbook open")
}

/// Pixel geometry the grid lays out with (96 dpi).
fn geometry(h: &Harness<'_, App>, sheet: usize) -> SheetGeometry {
    let wb = workbook(h);
    wb.sheet(sheet)
        .expect("worksheet")
        .geometry(GridMetrics::for_styles(wb.styles(), 96))
}

/// The published Cell node at 1-based `(row, col)`.
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

fn selected(h: &Harness<'_, App>, row: u32, col: u32) -> bool {
    cell(h, row, col)
        .unwrap_or_else(|| panic!("cell {row},{col} is not published"))
        .accesskit_node()
        .data()
        .is_selected()
        .unwrap_or(false)
}

#[test]
fn column_widths_and_row_heights_follow_the_sheet() {
    let h = harness("grid-geometry");
    let geo = geometry(&h, 0);
    // Columns A..H have different custom widths, rows 5..7 different heights.
    for col in 1..=8u32 {
        let r = rect_of(&h, 5, col);
        assert_eq!(r.width(), geo.col_px(col) as f32, "column {col}");
    }
    let widths: Vec<f32> = (1..=8).map(|c| rect_of(&h, 5, c).width()).collect();
    assert!(
        widths.windows(2).filter(|w| w[0] != w[1]).count() >= 6,
        "{widths:?}"
    );
    for row in [5u32, 6, 7, 9] {
        let r = rect_of(&h, row, 3);
        assert_eq!(r.height(), geo.row_px(row) as f32, "row {row}");
    }
    // Row 6 is 7.5 pt, row 7 is 40.5 pt: 10 px and 54 px at 96 dpi.
    assert_eq!(rect_of(&h, 6, 3).height(), 10.0);
    assert_eq!(rect_of(&h, 7, 3).height(), 54.0);
    // Cells are adjacent: no gap or overlap between neighbours.
    assert_eq!(rect_of(&h, 5, 3).max.x, rect_of(&h, 5, 4).min.x);
    assert_eq!(rect_of(&h, 6, 3).max.y, rect_of(&h, 7, 3).min.y);
}

#[test]
fn default_sizes_come_from_sheet_format() {
    let mut h = harness("grid-geometry");
    h.state_mut().grid.select_sheet(1);
    h.run();
    let geo = geometry(&h, 1);
    // Sheet 2 sets defaultColWidth 12.77 and a custom default row height of 20.25 pt.
    let r = rect_of(&h, 3, 4);
    assert_eq!(r.width(), geo.col_px(4) as f32);
    assert_eq!(r.height(), 27.0);
    assert!(r.width() > 80.0);
}

#[test]
fn hidden_rows_and_columns_take_no_space_and_are_skipped() {
    let mut h = harness("grid-geometry");
    // Row 8 and column K are hidden.
    assert!(cell(&h, 8, 1).is_none());
    assert_eq!(rect_of(&h, 7, 1).max.y, rect_of(&h, 9, 1).min.y);
    h.state_mut().grid.set_active(5, 12);
    h.run();
    h.run();
    assert!(cell(&h, 5, 11).is_none());
    assert_eq!(rect_of(&h, 5, 10).max.x, rect_of(&h, 5, 12).min.x);
    h.state_mut().grid.set_active(7, 1);
    h.run();
    h.run();
    let grid = h.get_by_role(Role::Grid);
    assert!(
        grid.accesskit_node()
            .data()
            .row_count()
            .is_some_and(|n| n >= 200)
    );
    // Arrow keys step over the hidden row.
    h.state_mut().grid.set_active(7, 1);
    h.run();
    h.key_press(Key::ArrowDown);
    h.run();
    assert_eq!(h.state().grid.active(), (9, 1));
    h.key_press(Key::ArrowUp);
    h.run();
    assert_eq!(h.state().grid.active(), (7, 1));
}

#[test]
fn merged_cells_are_one_cell_with_spans() {
    let mut h = harness("merged-cells");
    let geo = geometry(&h, 0);
    // A1:C1 is one cell spanning three columns; B1 and C1 are not published.
    let a1 = cell(&h, 1, 1).expect("A1 anchor");
    let d = a1.accesskit_node().data().clone();
    assert_eq!(d.column_span(), Some(3));
    assert_eq!(d.row_span(), None);
    assert_eq!(d.value(), Some("wide"));
    let expected: u32 = (1..=3).map(|c| geo.col_px(c)).sum();
    assert_eq!(a1.rect().width(), expected as f32);
    assert!(cell(&h, 1, 2).is_none() && cell(&h, 1, 3).is_none());
    assert!(cell(&h, 1, 4).is_some());
    // A3:A5 spans three rows.
    let a3 = cell(&h, 3, 1).expect("A3 anchor");
    let d = a3.accesskit_node().data().clone();
    assert_eq!(d.row_span(), Some(3));
    assert_eq!(d.column_span(), None);
    let expected: u32 = (3..=5).map(|r| geo.row_px(r)).sum();
    assert_eq!(a3.rect().height(), expected as f32);
    assert!(cell(&h, 4, 1).is_none() && cell(&h, 5, 1).is_none());
    // Moving right from the merge lands after it; moving down lands below it.
    h.key_press(Key::ArrowRight);
    h.run();
    assert_eq!(h.state().grid.active(), (1, 4));
    h.state_mut().grid.set_active(3, 1);
    h.run();
    h.key_press(Key::ArrowDown);
    h.run();
    assert_eq!(h.state().grid.active(), (6, 1));
    // Selecting a cell inside a merge selects the merge anchor.
    h.state_mut().grid.set_active(4, 1);
    h.run();
    assert_eq!(h.state().grid.active(), (3, 1));
}

#[test]
fn frozen_panes_stay_while_the_rest_scrolls() {
    let mut h = harness("grid-geometry");
    // Two frozen rows and two frozen columns.
    let a1_before = rect_of(&h, 1, 1);
    let b2_before = rect_of(&h, 2, 2);
    h.state_mut().grid.set_active(60, 8);
    h.run();
    h.run();
    assert_eq!(rect_of(&h, 1, 1), a1_before, "A1 moved");
    assert_eq!(rect_of(&h, 2, 2), b2_before, "B2 moved");
    // The active cell scrolled into view beside the frozen block, not under it.
    let active = rect_of(&h, 60, 8);
    assert!(
        active.min.x >= b2_before.max.x,
        "{active:?} under frozen columns"
    );
    assert!(
        active.min.y >= b2_before.max.y,
        "{active:?} under frozen rows"
    );
    assert!(active.max.x <= 1100.0 && active.max.y <= 800.0);
    // A frozen row cell above the scrolled column keeps its y and follows the column's x.
    let h1 = cell(&h, 1, 8).expect("frozen row cell above the scrolled column");
    assert_eq!(h1.rect().min.y, a1_before.min.y);
    assert_eq!(h1.rect().min.x, active.min.x);
    // Scrolled-out middle rows and columns are not published.
    assert!(cell(&h, 3, 3).is_none());
}

#[test]
fn shift_click_selects_a_range() {
    let mut h = harness("formulas-basic");
    cell(&h, 2, 2).expect("B2").click();
    h.run();
    assert_eq!(h.state().grid.active(), (2, 2));
    assert_eq!(h.state().grid.selection(), (2, 2, 2, 2));
    cell(&h, 4, 4)
        .expect("D4")
        .click_modifiers(Modifiers::SHIFT);
    h.run();
    assert_eq!(h.state().grid.active(), (2, 2));
    assert_eq!(h.state().grid.selection(), (2, 2, 4, 4));
    // Cells in the range are marked selected in the accessibility tree.
    for (r, c, want) in [
        (2, 2, true),
        (3, 3, true),
        (4, 4, true),
        (5, 5, false),
        (1, 1, false),
    ] {
        assert_eq!(selected(&h, r, c), want, "cell {r},{c}");
    }
    // A plain click collapses the range.
    cell(&h, 6, 1).expect("A6").click();
    h.run();
    assert_eq!(h.state().grid.selection(), (6, 1, 6, 1));
}

#[test]
fn shift_arrows_extend_the_range_and_plain_arrows_collapse_it() {
    let mut h = harness("formulas-basic");
    h.state_mut().grid.set_active(2, 2);
    h.run();
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowRight);
    h.run();
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowDown);
    h.run();
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowDown);
    h.run();
    assert_eq!(h.state().grid.active(), (2, 2));
    assert_eq!(h.state().grid.selection(), (2, 2, 4, 3));
    // Shrinking works from the same corner.
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowUp);
    h.run();
    assert_eq!(h.state().grid.selection(), (2, 2, 3, 3));
    // Extending to the left of the anchor normalises the range.
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowLeft);
    h.run();
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowLeft);
    h.run();
    assert_eq!(h.state().grid.selection(), (2, 1, 3, 2));
    h.key_press(Key::ArrowDown);
    h.run();
    assert_eq!(h.state().grid.selection(), (3, 2, 3, 2));
}

#[test]
fn dragging_selects_a_range() {
    let mut h = harness("formulas-basic");
    let from = rect_of(&h, 1, 2).center();
    let to = rect_of(&h, 4, 3).center();
    h.drag_at(from);
    h.hover_at(to);
    h.drop_at(to);
    h.run();
    assert_eq!(h.state().grid.active(), (1, 2));
    assert_eq!(h.state().grid.selection(), (1, 2, 4, 3));
}

#[test]
fn a_range_cutting_a_merge_grows_to_hold_it() {
    let mut h = harness("merged-cells");
    // A3:A5 is merged. A range from B2 to A3 must take in all of A3:A5.
    cell(&h, 2, 2).expect("B2").click();
    h.run();
    cell(&h, 3, 1)
        .expect("A3 anchor")
        .click_modifiers(Modifiers::SHIFT);
    h.run();
    assert_eq!(h.state().grid.selection(), (2, 1, 5, 2));
    assert!(selected(&h, 3, 1));
}

#[test]
fn delete_clears_every_cell_in_the_range() {
    let mut h = harness("formulas-basic");
    cell(&h, 1, 1).expect("A1").click();
    h.run();
    cell(&h, 3, 1)
        .expect("A3")
        .click_modifiers(Modifiers::SHIFT);
    h.run();
    h.key_press(Key::Delete);
    h.run();
    let wb = workbook(&h);
    for row in 1..=3 {
        assert!(
            wb.cell(0, row, 1)
                .is_none_or(|c| matches!(c.value, CellValue::Empty)),
            "A{row} not cleared"
        );
    }
    assert!(
        wb.cell(0, 4, 1)
            .is_some_and(|c| !matches!(c.value, CellValue::Empty))
    );
}

#[test]
fn displayed_text_matches_excel_for_the_number_format_fixture() {
    let h = harness("number-formats");
    let reference: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/reference/xlsx/number-formats/values.json"),
        )
        .expect("values.json"),
    )
    .expect("json");
    let cells = reference["sheets"][0]["cells"].as_object().expect("cells");
    let mut compared = 0;
    for (address, info) in cells {
        let (row, col) = open365_xlsx::parse_cell_ref(address).expect("address");
        let want = info["text"].as_str().expect("text");
        if address == "A5" {
            // numFmtId 14 is the locale short date: the oracle ran en-GB (15/01/2024), the
            // engine formats the en-US built-in.
            let node = cell(&h, row, col).expect("A5");
            assert_eq!(node.accesskit_node().data().value(), Some("1/15/2024"));
            continue;
        }
        let node = cell(&h, row, col).unwrap_or_else(|| panic!("{address} not published"));
        let got = node
            .accesskit_node()
            .data()
            .value()
            .unwrap_or_default()
            .to_owned();
        assert_eq!(got, want, "{address}");
        compared += 1;
    }
    assert!(compared > 5, "compared {compared}");
}
