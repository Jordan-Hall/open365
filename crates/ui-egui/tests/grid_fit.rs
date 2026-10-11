//! Auto-fit and resize of columns and rows by their header borders, checked against Excel's
//! `AutoFit` (`fixtures/reference/xlsx/grid-autofit/autofit.json`, measured at 120 dpi).

use std::path::PathBuf;

use egui::{Event, Modifiers, PointerButton, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use open365_commands::Command;
use open365_ui_egui::{App, Resize};
use open365_xlsx::{GridMetrics, MAX_COL, MAX_ROW};

fn harness(name: &str, scale: f32, size: egui::Vec2) -> Harness<'static, App> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/xlsx")
        .join(format!("{name}.xlsx"));
    let app = App::new(Some(path));
    let mut h = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(scale)
        .with_step_dt(0.05)
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run();
    h.run();
    h
}

fn cell_rect(h: &Harness<'_, App>, row: u32, col: u32) -> egui::Rect {
    h.query_all_by_role(egui::accesskit::Role::Cell)
        .find(|c| {
            let d = c.accesskit_node().data().clone();
            d.row_index() == Some(row as usize - 1) && d.column_index() == Some(col as usize - 1)
        })
        .unwrap_or_else(|| panic!("cell {row},{col} is not published"))
        .rect()
}

fn press(h: &mut Harness<'_, App>, pos: egui::Pos2, pressed: bool) {
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    });
}

fn double_click(h: &mut Harness<'_, App>, pos: egui::Pos2) {
    // Let the previous click age past egui's double-click window.
    for _ in 0..8 {
        h.step();
    }
    h.event(Event::PointerMoved(pos));
    for _ in 0..2 {
        press(h, pos, true);
        h.step();
        press(h, pos, false);
        h.step();
    }
    h.run();
}

/// The resizes the app has applied (and not undone), read back from the command log.
fn resizes(h: &Harness<'_, App>) -> Vec<Resize> {
    let target = h.state().target().expect("target");
    target
        .applied_log()
        .commands()
        .iter()
        .filter_map(|c| match *c {
            Command::SetColumnWidth {
                sheet,
                first,
                last,
                width,
            } => Some(Resize::ColumnWidth {
                sheet,
                first,
                last,
                width,
            }),
            Command::SetRowHeight {
                sheet,
                first,
                last,
                points,
            } => Some(Resize::RowHeight {
                sheet,
                first,
                last,
                points,
            }),
            _ => None,
        })
        .collect()
}

/// Undoes the applied resize so the next measurement sees the original geometry.
fn undo_resize(h: &mut Harness<'_, App>) {
    h.state_mut().undo();
    h.run();
}

/// The pixel width a column request stands for at `dpi`.
fn width_px(h: &Harness<'_, App>, dpi: u32, r: &Resize) -> u32 {
    let wb = h.state().target().and_then(|t| t.xlsx()).expect("workbook");
    let Resize::ColumnWidth { width, .. } = *r else {
        panic!("not a column request: {r:?}")
    };
    GridMetrics::for_styles(wb.styles(), dpi).col_px(width)
}

fn reference() -> serde_json::Value {
    serde_json::from_slice(
        &std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/reference/xlsx/grid-autofit/autofit.json"),
        )
        .expect("reference file"),
    )
    .expect("reference json")
}

/// Excel's `Columns(c).AutoFit()` on the fixture, measured at 120 dpi. Our widths are the
/// width of the widest cell (hinted advances of the bundled Carlito and Caladea, which stand in
/// for Calibri and Cambria) plus Excel's 9 px of padding at that dpi. 25 of the 39 cases are
/// pixel-exact; the rest differ by the hinted advances of the real Calibri (a few pixels on
/// long text), by 5 px for wrapped text with a line break, and by 2 px for Calibri 9.
#[test]
fn column_auto_fit_matches_excel() {
    let mut h = harness("grid-autofit", 1.25, vec2(6000.0, 700.0));
    let json = reference();
    let dpi = json["dpi"].as_u64().unwrap() as u32;
    let geo = {
        let wb = h.state().target().and_then(|t| t.xlsx()).unwrap();
        wb.sheet(0)
            .unwrap()
            .geometry(GridMetrics::for_styles(wb.styles(), dpi))
    };
    let (mut exact, mut worst, mut total) = (0, 0i64, 0);
    for c in json["cols"].as_array().unwrap() {
        let col = c["col"].as_u64().unwrap() as u32;
        let want = (c["width_pt"].as_f64().unwrap() * f64::from(dpi) / 72.0).round() as i64;
        let r = cell_rect(&h, 1, 1);
        let x = r.min.x + geo.col_x(col + 1) as f32 / 1.25;
        double_click(&mut h, pos2(x, r.min.y - 12.0));
        let got = resizes(&h);
        undo_resize(&mut h);
        assert_eq!(got.len(), 1, "column {col}: {got:?}");
        let diff = i64::from(width_px(&h, dpi, &got[0])) - want;
        total += 1;
        exact += i64::from(diff == 0);
        worst = worst.max(diff.abs());
        assert!(
            diff.abs() <= 7,
            "column {col} {}: off by {diff}",
            c["label"]
        );
    }
    assert!(
        exact >= 24,
        "only {exact} of {total} columns are pixel-exact"
    );
    assert!(worst <= 7);
}

/// `Rows(r).AutoFit()`: one line of the font is GDI's text height (Windows ascent and descent)
/// plus 2 px, plus one more from 36 px of text; wrapped text takes a line per wrapped line.
#[test]
fn row_auto_fit_matches_excel() {
    let mut h = harness("grid-autofit", 1.25, vec2(1200.0, 900.0));
    h.state_mut().grid.select_sheet(1);
    h.run();
    h.run();
    let json = reference();
    let dpi = json["dpi"].as_u64().unwrap() as u32;
    let metrics = {
        let wb = h.state().target().and_then(|t| t.xlsx()).unwrap();
        GridMetrics::for_styles(wb.styles(), dpi)
    };
    let mut exact = 0;
    let rows = json["rows"].as_array().unwrap();
    for c in rows {
        let row = c["row"].as_u64().unwrap() as u32;
        let want = (c["height_pt"].as_f64().unwrap() * f64::from(dpi) / 72.0).round() as i64;
        let r = cell_rect(&h, row, 1);
        double_click(&mut h, pos2(r.min.x - 20.0, r.max.y));
        let got = resizes(&h);
        undo_resize(&mut h);
        assert_eq!(got.len(), 1, "row {row}: {got:?}");
        let Resize::RowHeight { points, .. } = got[0] else {
            panic!("not a row request")
        };
        let diff = i64::from(metrics.row_px(points)) - want;
        exact += i64::from(diff == 0);
        // Cambria 12 is the one size the Windows metrics of Caladea miss, by a pixel.
        assert!(diff.abs() <= 1, "row {row} {}: off by {diff}", c["label"]);
    }
    assert!(exact + 1 >= rows.len() as i64, "{exact} of {}", rows.len());
}

/// A header border is grabbed within a few points; dragging it asks for the new size and the
/// size is the pixel count the pointer reached.
#[test]
fn dragging_a_column_border_requests_the_new_width() {
    let mut h = harness("formulas-basic", 1.0, vec2(1100.0, 800.0));
    let r = cell_rect(&h, 1, 2);
    let (from, to) = (
        pos2(r.max.x, r.min.y - 12.0),
        pos2(r.max.x + 40.0, r.min.y - 12.0),
    );
    h.event(Event::PointerMoved(from));
    press(&mut h, from, true);
    h.step();
    h.event(Event::PointerMoved(to));
    h.step();
    press(&mut h, to, false);
    h.step();
    h.run();
    let got = resizes(&h);
    let [
        Resize::ColumnWidth {
            first, last, width, ..
        },
    ] = got[..]
    else {
        panic!("{got:?}")
    };
    assert_eq!((first, last), (2, 2));
    let wb = h.state().target().and_then(|t| t.xlsx()).unwrap();
    let metrics = GridMetrics::for_styles(wb.styles(), 96);
    assert_eq!(metrics.col_px(width), r.width() as u32 + 40);
    // The border did not select anything.
    assert_eq!(h.state().grid.selection(), (1, 1, 1, 1));
}

#[test]
fn dragging_a_row_border_requests_the_new_height() {
    let mut h = harness("formulas-basic", 1.0, vec2(1100.0, 800.0));
    let r = cell_rect(&h, 2, 1);
    let (from, to) = (
        pos2(r.min.x - 20.0, r.max.y),
        pos2(r.min.x - 20.0, r.max.y + 10.0),
    );
    h.event(Event::PointerMoved(from));
    press(&mut h, from, true);
    h.step();
    h.event(Event::PointerMoved(to));
    h.step();
    press(&mut h, to, false);
    h.step();
    h.run();
    let got = resizes(&h);
    let [
        Resize::RowHeight {
            first,
            last,
            points,
            ..
        },
    ] = got[..]
    else {
        panic!("{got:?}")
    };
    assert_eq!((first, last), (2, 2));
    let wb = h.state().target().and_then(|t| t.xlsx()).unwrap();
    let metrics = GridMetrics::for_styles(wb.styles(), 96);
    assert_eq!(metrics.row_px(points), r.height() as u32 + 10);
}

/// Dragging a border inside a selection of whole columns resizes all of them, as in Excel.
#[test]
fn resizing_inside_a_column_selection_resizes_the_selection() {
    let mut h = harness("formulas-basic", 1.0, vec2(1100.0, 800.0));
    h.state_mut().grid.select_columns(2, 3);
    h.run();
    let r = cell_rect(&h, 1, 3);
    let (from, to) = (
        pos2(r.max.x, r.min.y - 12.0),
        pos2(r.max.x - 20.0, r.min.y - 12.0),
    );
    h.event(Event::PointerMoved(from));
    press(&mut h, from, true);
    h.step();
    h.event(Event::PointerMoved(to));
    h.step();
    press(&mut h, to, false);
    h.step();
    h.run();
    let got = resizes(&h);
    assert!(
        matches!(
            got[..],
            [Resize::ColumnWidth {
                first: 2,
                last: 3,
                ..
            }]
        ),
        "{got:?}"
    );
    assert_eq!(h.state().grid.selection(), (1, 2, MAX_ROW, 3));
    let _ = MAX_COL;
}

#[test]
fn the_pixel_to_width_conversion_round_trips() {
    let mut h = harness("formulas-basic", 1.0, vec2(1100.0, 800.0));
    h.run();
    let wb = h.state().target().and_then(|t| t.xlsx()).unwrap();
    for dpi in [96, 120, 144] {
        let m = GridMetrics::for_styles(wb.styles(), dpi);
        for px in [0u32, 1, 8, 20, 64, 100, 333, 1789] {
            let width = if px == 0 {
                0.0
            } else {
                f64::from(px) / f64::from(m.mdw)
            };
            assert!(m.col_px(width) >= px.saturating_sub(1));
        }
    }
}
