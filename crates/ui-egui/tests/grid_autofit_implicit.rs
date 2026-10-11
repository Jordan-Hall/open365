//! Rows without a custom height re-fit when a cell's wrapping, font or value changes, as Excel does
//! (`fixtures/reference/xlsx/autofit-implicit/autofit.json`, `tools/office/make-autofit-refs.ps1`),
//! and the explicit AutoFit Row Height / Column Width actions store Excel's flags.

use std::path::PathBuf;

use egui::vec2;
use egui_kittest::Harness;
use open365_commands::{AlignmentChange, CellFormatChange, FontChange};
use open365_commands::{CellEntry, Command, RangeRef};
use open365_ui_egui::App;
use open365_xlsx::GridMetrics;

const TEXT: &str = "The quick brown fox jumps over the lazy dog again and again";

fn harness() -> Harness<'static, App> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/xlsx/grid-autofit.xlsx");
    let mut app = App::new(Some(path));
    app.grid.select_sheet(1);
    let mut h = Harness::builder()
        .with_size(vec2(1400.0, 900.0))
        .with_pixels_per_point(1.25)
        .with_step_dt(0.05)
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run();
    h.run();
    h
}

fn reference() -> serde_json::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/reference/xlsx/autofit-implicit/autofit.json");
    serde_json::from_slice(&std::fs::read(path).expect("present")).expect("present")
}

/// Excel's height in points for a step of a reference case.
fn want(json: &serde_json::Value, label: &str, step: &str) -> f64 {
    json["cases"]
        .as_array()
        .expect("present")
        .iter()
        .find(|c| c["label"] == label && c["step"] == step)
        .unwrap_or_else(|| panic!("{label} / {step}"))["height_pt"]
        .as_f64()
        .expect("present")
}

fn apply(h: &mut Harness<'_, App>, c: Command) {
    h.state_mut().apply_command(c).expect("applies");
    h.run();
    h.run();
}

fn text_at(row: u32) -> Command {
    Command::XlsxSetCellValue {
        sheet: 1,
        row,
        col: 1,
        value: CellEntry::Text(TEXT.to_owned()),
    }
}

fn format(row: u32, change: CellFormatChange) -> Command {
    Command::FormatCells {
        sheet: 1,
        range: RangeRef::cell(row, 1),
        change: Box::new(change),
    }
}

fn wrap(on: bool) -> CellFormatChange {
    CellFormatChange {
        alignment: AlignmentChange {
            wrap_text: Some(on),
            ..AlignmentChange::default()
        },
        ..CellFormatChange::default()
    }
}

fn size(pt: f64) -> CellFormatChange {
    CellFormatChange {
        font: FontChange {
            size: Some(pt),
            ..FontChange::default()
        },
        ..CellFormatChange::default()
    }
}

/// The row's height in 120 dpi pixels and whether it is flagged custom.
fn row_state(h: &Harness<'_, App>, row: u32) -> (u32, bool) {
    let wb = h.state().target().and_then(|t| t.xlsx()).expect("present");
    let ws = wb.sheet(1).expect("present");
    let m = GridMetrics::for_styles(wb.styles(), 120);
    let r = ws.rows().iter().find(|r| r.r == row).expect("present");
    let pt = r.height().unwrap_or(ws.sheet_format().default_row_height);
    (m.row_px(pt), r.custom_height())
}

fn px(pt: f64) -> u32 {
    (pt * 120.0 / 72.0).round() as u32
}

#[test]
fn wrapping_and_font_changes_refit_the_row_like_excel() {
    let json = reference();
    let mut h = harness();
    let row = 60;
    apply(&mut h, text_at(row));
    assert_eq!(
        row_state(&h, row),
        (px(want(&json, "wrap toggle", "start")), false)
    );
    apply(&mut h, format(row, wrap(true)));
    let got = row_state(&h, row);
    assert_eq!(got.0, px(want(&json, "wrap toggle", "wrap on")), "wrap on");
    assert!(!got.1, "an auto-fitted row is not custom");
    apply(&mut h, format(row, wrap(false)));
    assert_eq!(
        row_state(&h, row).0,
        px(want(&json, "wrap toggle", "wrap off")),
        "wrap off"
    );

    // A larger font on a plain cell.
    let row = 61;
    apply(&mut h, text_at(row));
    apply(&mut h, format(row, size(24.0)));
    assert_eq!(
        row_state(&h, row).0,
        px(want(&json, "size only", "size 24")),
        "size 24"
    );
}

#[test]
fn a_custom_height_survives_until_an_explicit_auto_fit() {
    let json = reference();
    let mut h = harness();
    let row = 62;
    apply(&mut h, text_at(row));
    apply(
        &mut h,
        Command::SetRowHeight {
            sheet: 1,
            first: row,
            last: row,
            points: 39.6,
        },
    );
    apply(&mut h, format(row, wrap(true)));
    assert_eq!(
        row_state(&h, row),
        (px(want(&json, "custom height", "wrap on")), true)
    );
    // Home > Format > AutoFit Row Height on the row releases the flag and fits the wrapped text.
    h.state_mut().grid.set_active(row, 1);
    h.state_mut().grid.autofit_selection(false);
    h.run();
    h.state_mut().apply_resizes();
    h.run();
    assert_eq!(
        row_state(&h, row),
        (px(want(&json, "custom height", "rows autofit")), false)
    );
}

#[test]
fn auto_fit_column_width_stores_best_fit() {
    let mut h = harness();
    apply(&mut h, text_at(63));
    h.state_mut().grid.set_active(63, 1);
    h.state_mut().grid.autofit_selection(true);
    h.run();
    h.state_mut().apply_resizes();
    h.run();
    let wb = h.state().target().and_then(|t| t.xlsx()).expect("present");
    let col = wb
        .sheet(1)
        .expect("present")
        .columns()
        .iter()
        .find(|c| c.min <= 1 && c.max >= 1)
        .cloned()
        .expect("present");
    assert!(col.custom_width);
    // The text is wider than the 12-character column it started in.
    assert!(col.width.expect("present") > 20.0, "{col:?}");
    h.state_mut().undo();
    h.run();
}
