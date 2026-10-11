//! The header context menu (Hide, Unhide, Column Width, Row Height), its dialogs and the
//! drag-to-unhide border, all through the command layer and undoable.

use std::path::PathBuf;

use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::{Harness, Node};
use open365_ui_egui::App;
use open365_xlsx::{GridMetrics, SheetGeometry, Workbook};

fn harness() -> Harness<'static, App> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/xlsx/formulas-basic.xlsx");
    let app = App::new(Some(path));
    let mut h = Harness::builder()
        .with_size(vec2(1100.0, 800.0))
        .with_step_dt(0.05)
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run();
    h
}

fn workbook<'a>(h: &'a Harness<'_, App>) -> &'a Workbook {
    h.state().target().and_then(|t| t.xlsx()).expect("workbook")
}

fn geometry(h: &Harness<'_, App>) -> SheetGeometry {
    let wb = workbook(h);
    wb.sheet(0)
        .expect("sheet")
        .geometry(GridMetrics::for_styles(wb.styles(), 96))
}

fn cell_node<'a>(h: &'a Harness<'_, App>, row: u32, col: u32) -> Node<'a> {
    h.query_all_by_role(egui::accesskit::Role::Cell)
        .find(|c| {
            let d = c.accesskit_node().data().clone();
            d.row_index() == Some(row as usize - 1) && d.column_index() == Some(col as usize - 1)
        })
        .unwrap_or_else(|| panic!("cell {row},{col} is not published"))
}

fn col_header(h: &Harness<'_, App>, col: u32) -> Pos2 {
    let r = cell_node(h, 1, col).rect();
    Pos2::new(r.center().x, r.top() - 12.0)
}

fn row_header(h: &Harness<'_, App>, row: u32) -> Pos2 {
    let r = cell_node(h, row, 1).rect();
    Pos2::new(r.left() - 25.0, r.center().y)
}

fn right_click(h: &mut Harness<'_, App>, pos: Pos2) {
    h.event(Event::PointerMoved(pos));
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.run();
}

fn choose(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label(label).click();
    h.run();
    h.run();
}

fn dialog(h: &mut Harness<'_, App>, text: &str) {
    h.event(Event::Text(text.to_owned()));
    h.run();
    h.get_by_label("OK").click();
    h.run();
    h.run();
}

#[test]
fn hide_column_from_the_header_menu_and_undo() {
    let mut h = harness();
    assert!(!geometry(&h).col_hidden(2));
    let at = col_header(&h, 2);
    right_click(&mut h, at);
    choose(&mut h, "Hide");
    assert!(geometry(&h).col_hidden(2), "B is hidden");
    assert!(!geometry(&h).col_hidden(1) && !geometry(&h).col_hidden(3));
    h.state_mut().undo();
    h.run();
    assert!(!geometry(&h).col_hidden(2), "undo shows B again");
}

#[test]
fn hide_and_unhide_rows() {
    let mut h = harness();
    let at = row_header(&h, 3);
    right_click(&mut h, at);
    choose(&mut h, "Hide");
    assert!(geometry(&h).row_hidden(3));
    // Select rows 2:4 and unhide.
    h.state_mut().grid.select_rows(2, 4);
    h.run();
    let at = row_header(&h, 2);
    right_click(&mut h, at);
    choose(&mut h, "Unhide");
    assert!(!geometry(&h).row_hidden(3), "row 3 is shown again");
}

#[test]
fn column_width_dialog_applies_within_limits_and_undoes() {
    let mut h = harness();
    let before = geometry(&h).col_px(3);
    let at = col_header(&h, 3);
    right_click(&mut h, at);
    choose(&mut h, "Column Width...");
    dialog(&mut h, "300");
    assert_eq!(geometry(&h).col_px(3), before, "300 is above Excel's 255");
    assert!(h.query_by_label_contains("between 0 and 255").is_some());
    h.get_by_label("Cancel").click();
    h.run();

    let at = col_header(&h, 3);
    right_click(&mut h, at);
    choose(&mut h, "Column Width...");
    dialog(&mut h, "20");
    let metrics = GridMetrics::for_styles(workbook(&h).styles(), 96);
    assert_eq!(geometry(&h).col_px(3), metrics.col_px(20.0));
    h.state_mut().undo();
    h.run();
    assert_eq!(geometry(&h).col_px(3), before);
}

#[test]
fn row_height_dialog_applies() {
    let mut h = harness();
    let at = row_header(&h, 2);
    right_click(&mut h, at);
    choose(&mut h, "Row Height...");
    dialog(&mut h, "30");
    let metrics = GridMetrics::for_styles(workbook(&h).styles(), 96);
    assert_eq!(geometry(&h).row_px(2), metrics.row_px(30.0));
}

#[test]
fn select_all_sets_every_column_width() {
    let mut h = harness();
    h.state_mut().grid.select_all();
    h.run();
    assert_eq!(h.state().grid.selection(), (1, 1, 1_048_576, 16_384));
    let at = col_header(&h, 2);
    right_click(&mut h, at);
    choose(&mut h, "Column Width...");
    dialog(&mut h, "12");
    let metrics = GridMetrics::for_styles(workbook(&h).styles(), 96);
    let geo = geometry(&h);
    assert_eq!(geo.col_px(2), metrics.col_px(12.0));
    assert_eq!(
        geo.col_px(40),
        metrics.col_px(12.0),
        "the default width changed"
    );
}

#[test]
fn dragging_the_double_line_unhides_a_column() {
    let mut h = harness();
    let at = col_header(&h, 2);
    right_click(&mut h, at);
    choose(&mut h, "Hide");
    assert!(geometry(&h).col_hidden(2));
    // The boundary between A and C is where B used to start.
    let c = cell_node(&h, 1, 3).rect();
    let border = Pos2::new(c.left(), c.top() - 12.0);
    h.event(Event::PointerMoved(border));
    h.run();
    for (pressed, x) in [(true, 0.0), (false, 60.0)] {
        if !pressed {
            h.event(Event::PointerMoved(border + vec2(x, 0.0)));
            h.step();
        }
        h.event(Event::PointerButton {
            pos: border + vec2(x, 0.0),
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.run();
    assert!(!geometry(&h).col_hidden(2), "B is visible again");
    assert!(geometry(&h).col_px(2) >= 55);
}

#[test]
fn corner_menu_sets_every_row_height() {
    let mut h = harness();
    let a1 = cell_node(&h, 1, 1).rect();
    right_click(&mut h, Pos2::new(a1.left() - 25.0, a1.top() - 12.0));
    choose(&mut h, "Row Height...");
    dialog(&mut h, "25");
    let metrics = GridMetrics::for_styles(workbook(&h).styles(), 96);
    let geo = geometry(&h);
    assert_eq!(geo.row_px(2), metrics.row_px(25.0));
    assert_eq!(
        geo.row_px(500),
        metrics.row_px(25.0),
        "the default height changed"
    );
    assert_eq!(
        h.state().grid.selection().2,
        1_048_576,
        "the sheet is selected"
    );
}
