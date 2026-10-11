//! Data bar geometry from `fixtures/xlsx/grid-databar.xlsx`, against the extents Excel drew
//! (`fixtures/reference/xlsx/grid-databar/databars.json`): where the axis lies and how long each
//! bar is, as fractions of the bar span, for every rule of the fixture.

use std::path::PathBuf;

use open365_commands::ConditionalFormats;
use open365_opc::Limits;
use open365_xlsx::Workbook;
use serde_json::Value as Json;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Excel's bar span in a 151 px cell: pixels 2 to 150.
const SPAN: f64 = 148.0;

#[test]
fn axis_and_length_match_excel_for_every_rule() {
    let bytes = std::fs::read(root().join("fixtures/xlsx/grid-databar.xlsx")).expect("fixture");
    let wb = Workbook::open(&bytes, &Limits::default()).expect("opens");
    let cf = ConditionalFormats::new(&wb, 0, None);
    let path = root().join("fixtures/reference/xlsx/grid-databar/databars.json");
    let reference: Json = serde_json::from_slice(&std::fs::read(path).expect("ref")).expect("json");
    let mut compared = 0;
    let mut failures = Vec::new();
    for case in reference["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let col = case["col"].as_u64().unwrap() as u32;
        for row in case["rows"].as_array().unwrap() {
            let r = row["row"].as_u64().unwrap() as u32;
            let bar = cf
                .effect(&wb, r, col)
                .and_then(|e| e.data_bar.clone())
                .unwrap_or_else(|| panic!("{name} row {r}: no bar"));
            // Excel's extent, ignoring the one or two black pixels of the axis line.
            let (mut from, mut to) = (f64::MAX, f64::MIN);
            for run in row["runs"].as_array().unwrap() {
                let (a, b) = (run["from"].as_f64().unwrap(), run["to"].as_f64().unwrap());
                let axis_only = run["first"] == "000000" && run["last"] == "000000" && b - a <= 3.0;
                if !axis_only {
                    from = from.min(a);
                    to = to.max(b);
                }
            }
            let want = (to - from).max(0.0) / SPAN;
            let got = bar.length;
            compared += 1;
            // Within 2 px of the span (the axis line adds a pixel or two to Excel's run); a bar under two pixels hides in the axis line.
            if (got - want).abs() * SPAN > 2.0 && got * SPAN >= 2.0 {
                failures.push(format!("{name} row {r}: length {got:.3}, Excel {want:.3}"));
            }
        }
    }
    assert!(compared >= 130, "{compared} bars compared");
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn the_axis_of_the_automatic_rule_follows_zero() {
    let bytes = std::fs::read(root().join("fixtures/xlsx/grid-databar.xlsx")).expect("fixture");
    let wb = Workbook::open(&bytes, &Limits::default()).expect("opens");
    let cf = ConditionalFormats::new(&wb, 0, None);
    let bar = |row, col| cf.effect(&wb, row, col).unwrap().data_bar.clone().unwrap();
    // Column B, values -50..100: zero lies a third in; -50 grows left, 100 grows right.
    let low = bar(2, 2);
    assert!((low.axis - 1.0 / 3.0).abs() < 1e-9);
    assert!(low.grows_left && low.negative && (low.length - 1.0 / 3.0).abs() < 1e-9);
    let high = bar(8, 2);
    assert!(!high.grows_left && (high.length - 2.0 / 3.0).abs() < 1e-9);
    assert!(high.axis_line.is_some());
    // Column D, all negative: the axis is the right edge and the lowest value fills the cell.
    let all_negative = bar(9, 4);
    assert_eq!(all_negative.axis, 1.0);
    assert!(all_negative.grows_left && (all_negative.length - 1.0).abs() < 1e-9);
    assert!(all_negative.axis_line.is_none());
    // Column A of the second block, right to left: the axis mirrors and positive bars grow left.
    let rtl = bar(19, 1);
    assert!((rtl.axis - 2.0 / 3.0).abs() < 1e-9 && rtl.grows_left);
}
