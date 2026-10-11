//! Chart anchors of the Excel-made `chart-*` fixtures against `Chart.ChartObject` size in points.

mod common;

use open365_opc::Limits;
use open365_xlsx::{Anchor, GridMetrics, Workbook};

use common::{fixture, repo_root};

#[test]
fn two_cell_anchor_size_matches_excel() {
    let wb = Workbook::open(&fixture("chart-column-clustered"), &Limits::default()).expect("opens");
    let charts = wb.sheet_charts(0);
    assert_eq!(charts.len(), 1);
    assert!(matches!(charts[0].anchor, Anchor::TwoCell { .. }));
    assert_eq!(charts[0].chart_part.as_str(), "xl/charts/chart1.xml");
    let ws = wb.sheet(0).expect("sheet");
    let (w, h) = charts[0]
        .anchor
        .size_pt(ws, GridMetrics::for_styles(wb.styles(), 96));
    let text = std::fs::read_to_string(
        repo_root().join("fixtures/reference/xlsx/chart-column-clustered/layout.json"),
    )
    .expect("layout reference");
    let json: serde_json::Value = serde_json::from_str(&text).expect("json");
    let (ew, eh) = (
        json["object"]["width"].as_f64().expect("w"),
        json["object"]["height"].as_f64().expect("h"),
    );
    assert!(
        (w - ew).abs() < 0.01 && (h - eh).abs() < 0.01,
        "{w}x{h} vs {ew}x{eh}"
    );
}

#[test]
fn sheets_without_a_drawing_have_no_charts() {
    let wb = Workbook::open(&fixture("formulas-basic"), &Limits::default()).expect("opens");
    assert!(wb.sheet_charts(0).is_empty());
}
