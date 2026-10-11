//! Chart rectangles against Excel: `PlotArea.Inside*`, `PlotArea.Left/Top/Width/Height`,
//! `Legend.*` and `ChartTitle.*` recorded by `tools/office/chart-oracle.ps1 -Layout` for every
//! `fixtures/xlsx/chart-*.xlsx`, compared at +-1 pt.

mod common;

use open365_chart::{NoCells, Rect, RenderOptions, render, sheet_charts};
use serde_json::Value;

use common::{CarlitoMeasure, open, reference};

const TOLERANCE: f64 = 1.0;

/// Fixtures made after the layout constants were fitted; never used to choose them.
const HOLDOUT: [&str; 3] = [
    "chart-holdout-bar-stacked",
    "chart-holdout-line-left",
    "chart-holdout-column-titles",
];

const FIXTURES: [&str; 10] = [
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
];

fn rect(v: &Value) -> Option<Rect> {
    if v.is_null() {
        return None;
    }
    Some(Rect::new(
        v["left"].as_f64()?,
        v["top"].as_f64()?,
        v["width"].as_f64()?,
        v["height"].as_f64()?,
    ))
}

fn diff(
    label: &str,
    got: Option<Rect>,
    want: Option<Rect>,
    out: &mut Vec<String>,
    stats: &mut (usize, usize),
) {
    match (got, want) {
        (Some(g), Some(w)) => {
            for (name, a, b) in [
                ("left", g.x, w.x),
                ("top", g.y, w.y),
                ("width", g.width, w.width),
                ("height", g.height, w.height),
            ] {
                stats.1 += 1;
                if (a - b).abs() <= TOLERANCE {
                    stats.0 += 1;
                } else {
                    out.push(format!("{label}.{name}: got {a:.2}, Excel {b:.2}"));
                }
            }
        }
        (None, None) => {}
        (g, w) => out.push(format!("{label}: presence differs, got {g:?}, Excel {w:?}")),
    }
}

/// Fixtures whose layout is held to the tolerance. The list in `docs/architecture/charts.md`
/// says which are expected to match and why the rest do not yet.
fn run(name: &str) -> (Vec<String>, (usize, usize)) {
    let wb = open(name);
    let charts = sheet_charts(&wb, 0);
    assert_eq!(charts.len(), 1, "{name}: one chart");
    let c = &charts[0];
    let want = reference(name, "layout.json");
    // Anchor size agrees with Chart.ChartObject.Width/Height.
    assert!(
        (c.anchor.width - want["object"]["width"].as_f64().expect("number")).abs() < 0.01,
        "{name}: anchor width"
    );
    assert!(
        (c.anchor.height - want["object"]["height"].as_f64().expect("number")).abs() < 0.01,
        "{name}: anchor height"
    );
    let measure = CarlitoMeasure::new();
    let opts = RenderOptions::new(&NoCells, &measure);
    let list = render(&c.space, c.anchor.width, c.anchor.height, &opts);
    let mut out = Vec::new();
    let mut stats = (0, 0);
    diff(
        "plot_inside",
        Some(list.plot_inside),
        rect(&want["plot_inside"]),
        &mut out,
        &mut stats,
    );
    diff(
        "plot_outer",
        Some(list.plot_outer),
        rect(&want["plot_outer"]),
        &mut out,
        &mut stats,
    );
    diff(
        "legend",
        list.legend,
        rect(&want["legend"]),
        &mut out,
        &mut stats,
    );
    diff(
        "title",
        list.title,
        rect(&want["title"]),
        &mut out,
        &mut stats,
    );
    (out, stats)
}

#[test]
fn rectangles_match_excel_within_one_point() {
    let mut total = (0, 0);
    let mut report = Vec::new();
    for name in FIXTURES {
        let (diffs, stats) = run(name);
        total.0 += stats.0;
        total.1 += stats.1;
        eprintln!(
            "{name}: {} of {} values within {TOLERANCE} pt",
            stats.0, stats.1
        );
        for d in diffs {
            report.push(format!("{name} {d}"));
        }
    }
    eprintln!(
        "layout: {} of {} values within {TOLERANCE} pt",
        total.0, total.1
    );
    assert!(report.is_empty(), "{}", report.join("\n"));
}

#[test]
fn hold_out_fixtures_match_without_refitting() {
    let mut total = (0, 0);
    let mut report = Vec::new();
    for name in HOLDOUT {
        let (diffs, stats) = run(name);
        total.0 += stats.0;
        total.1 += stats.1;
        eprintln!(
            "{name}: {} of {} values within {TOLERANCE} pt",
            stats.0, stats.1
        );
        for d in diffs {
            report.push(format!("{name} {d}"));
        }
    }
    eprintln!(
        "hold-out layout: {} of {} values within {TOLERANCE} pt",
        total.0, total.1
    );
    assert!(
        report.is_empty(),
        "{}",
        report.join(
            "
"
        )
    );
}
