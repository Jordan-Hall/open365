//! Chart rectangles and value-axis scales against Excel over a generated corpus
//! (`fixtures/reference/xlsx/chart-scale/layout-*.json`, recorded by
//! `tools/office/make-chart-corpus.ps1 -Set layout`).
//!
//! `layout-train.json` (seed 1001) is the set the layout constants were fitted on;
//! `layout-holdout.json` (seed 2002) was recorded afterwards and never used to choose a
//! constant. Each case is built into a chart part with [`ChartBuilder`], rendered at the
//! object size and compared at +-1 pt (rectangles) or exactly (axis scale).
//!
//! `FLOOR_*` are the measured pass rates; a change that lowers one fails the test. Run with
//! `--nocapture` for the per-quantity table and the first failures.

mod common;

use std::collections::BTreeMap;

use open365_chart::{
    BuildKind, BuildSeries, ChartBuilder, LegendPos, NoCells, Rect, RenderOptions, render,
};
use serde_json::Value;

use common::{CarlitoMeasure, f64s, scale_corpus};

const TOLERANCE: f64 = 1.0;

/// Share of cases (percent) whose every compared quantity matches, measured on the train set.
const FLOOR_TRAIN_ALL: f64 = 29.0;
/// Same for the hold-out set.
const FLOOR_HOLDOUT_ALL: f64 = 35.0;
/// Share of value-axis scales (min, max, major, minor) that match, train and hold-out. This is
/// the interval budget of `axis::max_intervals` working on our own plot-area rectangle.
const FLOOR_TRAIN_VALUE_AXIS: f64 = 98.0;
const FLOOR_HOLDOUT_VALUE_AXIS: f64 = 97.0;

fn kind_of(name: &str) -> BuildKind {
    match name {
        "column" => BuildKind::Column,
        "column_stacked" => BuildKind::ColumnStacked,
        "column_percent" => BuildKind::ColumnPercent,
        "bar" => BuildKind::Bar,
        "bar_stacked" => BuildKind::BarStacked,
        "line" => BuildKind::Line,
        "line_marker" => BuildKind::LineMarker,
        "area" => BuildKind::Area,
        "area_stacked" => BuildKind::AreaStacked,
        "scatter" => BuildKind::Scatter,
        "pie" => BuildKind::Pie,
        "doughnut" => BuildKind::Doughnut,
        other => panic!("unknown chart type {other}"),
    }
}

fn build(c: &Value) -> ChartBuilder {
    let mut b = ChartBuilder::new(kind_of(c["type"].as_str().expect("type")));
    b.categories = c["cats"]
        .as_array()
        .expect("cats")
        .iter()
        .map(|v| v.as_str().expect("string").to_owned())
        .collect();
    let names: Vec<&str> = c["names"]
        .as_array()
        .expect("names")
        .iter()
        .map(|v| v.as_str().expect("string"))
        .collect();
    let xs = c["xs"]
        .as_array()
        .map(|_| f64s(&c["xs"]))
        .unwrap_or_default();
    b.series = c["series"]
        .as_array()
        .expect("series")
        .iter()
        .zip(names)
        .map(|(v, n)| BuildSeries {
            name: n.to_owned(),
            values: f64s(v),
            xs: xs.clone(),
            sizes: Vec::new(),
        })
        .collect();
    b.title = c["title"].as_str().map(str::to_owned);
    b.category_title = c["cat_title"].as_str().map(str::to_owned);
    b.value_title = c["val_title"].as_str().map(str::to_owned);
    b.legend = match c["legend"].as_str().expect("legend") {
        "r" => Some(LegendPos::Right),
        "l" => Some(LegendPos::Left),
        "t" => Some(LegendPos::Top),
        "b" => Some(LegendPos::Bottom),
        _ => None,
    };
    b
}

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

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= TOLERANCE
}

fn rect_ok(got: Option<Rect>, want: Option<Rect>) -> bool {
    match (got, want) {
        (Some(g), Some(w)) => {
            near(g.x, w.x) && near(g.y, w.y) && near(g.width, w.width) && near(g.height, w.height)
        }
        (None, None) => true,
        _ => false,
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

#[derive(Default)]
struct Stats {
    cases: usize,
    all: usize,
    /// quantity -> (matched, total)
    by_quantity: BTreeMap<&'static str, (usize, usize)>,
    /// chart type -> (all matched, total)
    by_type: BTreeMap<String, (usize, usize)>,
    failures: Vec<String>,
}

fn evaluate(file: &str) -> Option<Stats> {
    let cases = scale_corpus(file)?;
    let measure = CarlitoMeasure::new();
    let mut st = Stats::default();
    for (n, c) in cases.iter().enumerate() {
        let space = build(c).build().expect("builds");
        let (w, h) = (
            c["width"].as_f64().expect("width"),
            c["height"].as_f64().expect("height"),
        );
        let list = render(&space, w, h, &RenderOptions::new(&NoCells, &measure));
        let mut ok = true;
        let mut record = |name: &'static str, hit: bool, st: &mut Stats| {
            let e = st.by_quantity.entry(name).or_default();
            e.1 += 1;
            e.0 += usize::from(hit);
            ok &= hit;
            hit
        };
        let mut why = Vec::new();
        for (name, got, want) in [
            ("plot_inside", Some(list.plot_inside), rect(&c["inside"])),
            ("plot_outer", Some(list.plot_outer), rect(&c["outer"])),
            ("legend", list.legend, rect(&c["legend_rect"])),
            ("title", list.title, rect(&c["title_rect"])),
        ] {
            if !record(name, rect_ok(got, want), &mut st) {
                why.push(format!("{name} got {got:?} want {want:?}"));
            }
        }
        if let Some(axis) = c.get("value_axis").filter(|v| !v.is_null()) {
            let got = list.value_scale;
            let hit = got.is_some_and(|s| {
                close(s.min, axis["min"].as_f64().unwrap_or(f64::NAN))
                    && close(s.max, axis["max"].as_f64().unwrap_or(f64::NAN))
                    && close(s.major, axis["major"].as_f64().unwrap_or(f64::NAN))
                    && close(s.minor, axis["minor"].as_f64().unwrap_or(f64::NAN))
            });
            if !record("value_axis", hit, &mut st) {
                why.push(format!("value_axis got {got:?} want {axis}"));
            }
        }
        st.cases += 1;
        let t = st
            .by_type
            .entry(c["type"].as_str().expect("type").to_owned())
            .or_default();
        t.1 += 1;
        if ok {
            st.all += 1;
            t.0 += 1;
        } else if st.failures.len() < 40 {
            st.failures.push(format!(
                "#{n} {} {w}x{h} legend={} title={}: {}",
                c["type"],
                c["legend"],
                c["title"],
                why.join("; ")
            ));
        }
    }
    Some(st)
}

fn report(label: &str, st: &Stats) {
    eprintln!(
        "{label}: {} of {} cases match on every quantity ({:.1} %)",
        st.all,
        st.cases,
        100.0 * st.all as f64 / st.cases as f64
    );
    for (q, (a, b)) in &st.by_quantity {
        eprintln!("  {q}: {a} of {b} ({:.1} %)", 100.0 * *a as f64 / *b as f64);
    }
    for (t, (a, b)) in &st.by_type {
        eprintln!("  type {t}: {a} of {b}");
    }
    for f in st.failures.iter().take(12) {
        eprintln!("  FAIL {f}");
    }
}

fn pct(st: &Stats) -> f64 {
    100.0 * st.all as f64 / st.cases.max(1) as f64
}

fn value_axis_pct(st: &Stats) -> f64 {
    let (a, b) = st.by_quantity.get("value_axis").copied().unwrap_or((0, 1));
    100.0 * a as f64 / b.max(1) as f64
}

#[test]
fn train_corpus_matches_excel() {
    let Some(st) = evaluate("layout-train.json") else {
        eprintln!("layout-train.json not recorded; skipped");
        return;
    };
    report("train", &st);
    assert!(pct(&st) >= FLOOR_TRAIN_ALL, "train {:.1} %", pct(&st));
    assert!(
        value_axis_pct(&st) >= FLOOR_TRAIN_VALUE_AXIS,
        "train value axis {:.1} %",
        value_axis_pct(&st)
    );
}

#[test]
fn hold_out_corpus_matches_excel_without_refitting() {
    let Some(st) = evaluate("layout-holdout.json") else {
        eprintln!("layout-holdout.json not recorded; skipped");
        return;
    };
    report("hold-out", &st);
    assert!(pct(&st) >= FLOOR_HOLDOUT_ALL, "hold-out {:.1} %", pct(&st));
    assert!(
        value_axis_pct(&st) >= FLOOR_HOLDOUT_VALUE_AXIS,
        "hold-out value axis {:.1} %",
        value_axis_pct(&st)
    );
}
