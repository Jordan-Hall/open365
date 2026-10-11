//! Chart parts of the Excel-made fixtures: typed model, lossless round trip, value resolution.

mod common;

use open365_chart::{
    ChartSpace, GroupKind, Grouping, PointValue, WorkbookCells, resolve, sheet_charts,
};
use open365_xlsx::parse_xml;

const NAMES: [&str; 13] = [
    "chart",
    "chart-area",
    "chart-bar-clustered",
    "chart-column-clustered",
    "chart-column-percent",
    "chart-column-stacked",
    "chart-doughnut",
    "chart-fixed-axis",
    "chart-line",
    "chart-pie",
    "chart-scatter",
    "chart-holdout-bar-stacked",
    "chart-holdout-line-left",
];

fn chart_part(name: &str) -> Vec<u8> {
    let wb = common::open(name);
    let dc = wb.sheet_charts(0).remove(0);
    wb.package()
        .part(dc.chart_part.as_str())
        .expect("chart part")
        .data()
        .to_vec()
}

#[test]
fn chart_parts_round_trip_with_every_element() {
    for name in NAMES {
        let bytes = chart_part(name);
        let space = ChartSpace::parse(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let again = space.to_bytes();
        assert_eq!(
            parse_xml(&bytes).expect("orig").root,
            parse_xml(&again).expect("rewritten").root,
            "{name}"
        );
        assert!(!space.chart.plot_area.groups.is_empty(), "{name}");
    }
}

#[test]
fn model_reads_series_axes_and_legend() {
    let space = ChartSpace::parse(&chart_part("chart-column-clustered")).expect("parses");
    let group = &space.chart.plot_area.groups[0];
    assert_eq!(group.kind, GroupKind::Bar);
    assert_eq!(group.grouping, Grouping::Clustered);
    assert_eq!(group.series.len(), 3);
    let v = group.series[0].values.as_ref().expect("values");
    assert_eq!(v.formula.as_deref(), Some("Data!$B$2:$B$6"));
    assert_eq!(v.cache[4], PointValue::Number(22.0));
    assert_eq!(
        space.chart.title.as_ref().expect("title").text,
        "Sales by region"
    );
    assert!(space.chart.legend.is_some());
    assert_eq!(space.chart.plot_area.axes.len(), 2);
    let stacked = ChartSpace::parse(&chart_part("chart-column-percent")).expect("parses");
    assert_eq!(
        stacked.chart.plot_area.groups[0].grouping,
        Grouping::PercentStacked
    );
}

#[test]
fn series_values_resolve_from_the_workbook_and_match_the_cache() {
    let wb = common::open("chart-column-clustered");
    let charts = sheet_charts(&wb, 0);
    let s = &charts[0].space.chart.plot_area.groups[0].series[1];
    let data = s.values.as_ref().expect("values");
    let live = resolve(data, &WorkbookCells(&wb));
    assert_eq!(live, data.cache);
    assert_eq!(live.len(), 5);
}
