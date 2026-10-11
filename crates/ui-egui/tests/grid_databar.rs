//! Data bars in the grid against Excel's own rendering
//! (`fixtures/reference/xlsx/grid-databar/databars.json`, recorded by
//! `tools/office/grid-databar-oracle.ps1`): where the axis sits for all-positive, mixed-sign and
//! all-negative ranges, the three axis positions, right-to-left bars, fixed and automatic
//! minimum and maximum, and the colour of negative bars.

use std::path::PathBuf;

use egui::{Color32, vec2};
use egui_kittest::Harness;
use open365_ui_egui::{App, PaintedCell};
use serde_json::Value as Json;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn reference() -> Json {
    let path = root().join("fixtures/reference/xlsx/grid-databar/databars.json");
    serde_json::from_slice(&std::fs::read(path).expect("reference")).expect("json")
}

fn harness() -> Harness<'static, App> {
    let app = App::new(Some(root().join("fixtures/xlsx/grid-databar.xlsx")));
    let mut h = Harness::builder()
        .with_size(vec2(1400.0, 1100.0))
        .with_pixels_per_point(1.0)
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run();
    h.run();
    h
}

fn painted(h: &Harness<'_, App>, row: u32, col: u32) -> Option<PaintedCell> {
    h.state()
        .grid
        .painted()
        .into_iter()
        .find(|p| p.row == row && p.col == col)
}

/// The extent Excel drew for one value: the first and last bar pixel, ignoring the one or two
/// pixels of the axis line (black, at most three wide) and an empty row.
fn excel_extent(runs: &Json) -> Option<(f64, f64)> {
    let mut from = f64::MAX;
    let mut to = f64::MIN;
    for r in runs.as_array().expect("runs") {
        let (a, b) = (
            r["from"].as_f64().expect("from"),
            r["to"].as_f64().expect("to"),
        );
        let axis_only = r["first"] == "000000" && r["last"] == "000000" && b - a <= 3.0;
        if !axis_only {
            from = from.min(a);
            to = to.max(b);
        }
    }
    (from < to).then_some((from, to))
}

#[test]
fn bars_match_excel_for_every_rule() {
    let h = harness();
    let reference = reference();
    let mut compared = 0;
    let mut worst = 0.0_f64;
    let mut failures = Vec::new();
    for case in reference["cases"].as_array().expect("cases") {
        let name = case["name"].as_str().unwrap();
        let col = case["col"].as_u64().unwrap() as u32;
        let excel_width = case["cell_width_px"].as_f64().unwrap();
        for row in case["rows"].as_array().unwrap() {
            let r = row["row"].as_u64().unwrap() as u32;
            let value = row["value"].as_f64().unwrap();
            let cell = painted(&h, r, col).unwrap_or_else(|| panic!("{name} row {r} not drawn"));
            let width = f64::from(cell.cell.width());
            // A bar under two pixels hides in the axis line Excel draws over it.
            let ours = cell
                .data_bar
                .as_ref()
                .filter(|b| b.to_px - b.from_px >= 2.0)
                .map(|b| (f64::from(b.from_px), f64::from(b.to_px)));
            let want = excel_extent(&row["runs"]);
            // Compare in pixels of Excel's cell width (a different cell width scales).
            let scale = excel_width / width;
            compared += 1;
            match (ours, want) {
                (None, None) => {}
                (Some((a, b)), Some((wa, wb))) => {
                    // Excel's first pixel is the axis pixel: one before the span of a bar that
                    // grows to the right from it.
                    let (a, b) = (a * scale, b * scale);
                    let d = (a - wa).abs().max((b - wb).abs());
                    worst = worst.max(d);
                    if d > 2.5 {
                        failures.push(format!(
                            "{name} value {value}: ours {a:.1}..{b:.1}, Excel {wa}..{wb}"
                        ));
                    }
                }
                (o, w) => failures.push(format!("{name} value {value}: ours {o:?}, Excel {w:?}")),
            }
        }
    }
    eprintln!("data bars: {compared} bars compared, worst edge {worst:.1} px");
    assert!(
        failures.is_empty(),
        "{} of {compared}:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn negative_bars_use_their_own_colour_and_the_axis_line_follows_the_rule() {
    let h = harness();
    let reference = reference();
    let case = |name: &str| {
        reference["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap_or_else(|| panic!("{name}"))
            .clone()
    };
    let bar = |name: &str, i: usize| {
        let c = case(name);
        let col = c["col"].as_u64().unwrap() as u32;
        let row = c["first_row"].as_u64().unwrap() as u32 + i as u32;
        painted(&h, row, col).unwrap().data_bar.unwrap()
    };
    // Mixed range with a red negative colour: -50 is red, 100 keeps the positive blue.
    assert_eq!(
        bar("mixed red negative", 0).color,
        Color32::from_rgb(255, 0, 0)
    );
    assert_eq!(
        bar("mixed red negative", 6).color,
        Color32::from_rgb(0x63, 0x8E, 0xC6)
    );
    // Negative bars that are "same as positive" keep the positive colour.
    assert_eq!(
        bar("mixed automatic", 0).color,
        Color32::from_rgb(0x63, 0x8E, 0xC6)
    );
    // The axis line shows for mixed ranges with an axis, never without one or for positives.
    assert!(bar("mixed automatic", 2).has_axis_line);
    assert!(bar("mixed midpoint", 4).has_axis_line);
    assert!(!bar("mixed no axis", 4).has_axis_line);
    assert!(!bar("positive automatic", 4).has_axis_line);
    // Solid fill is solid, the default is a gradient.
    assert!(!bar("mixed solid", 4).gradient);
    assert!(bar("mixed automatic", 4).gradient);
}
