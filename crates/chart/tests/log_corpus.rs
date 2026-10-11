//! Logarithmic value axes against Excel: `fixtures/reference/xlsx/chart-scale/log-*.json`
//! (recorded by `tools/office/make-chart-corpus.ps1 -Set log`).
//!
//! `log-train.json` (seed 1001) is the set the rules in [`open365_chart::axis::log_scale`]
//! were fitted on; `log-holdout.json` (seed 2002) was recorded afterwards.

mod common;

use open365_chart::axis::{LogNarrow, log_far_intervals, log_max_intervals, log_scale};
use open365_chart::{BuildKind, BuildSeries, ChartBuilder, NoCells, RenderOptions, render};
use serde_json::Value;

use common::{CarlitoMeasure, f64s, scale_corpus};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

fn scale_matches(got: &open365_chart::axis::AxisScale, c: &Value) -> bool {
    close(got.min, c["min"].as_f64().unwrap_or(f64::NAN))
        && close(got.max, c["max"].as_f64().unwrap_or(f64::NAN))
        && close(got.major, c["major"].as_f64().unwrap_or(f64::NAN))
        && close(got.minor, c["minor"].as_f64().unwrap_or(f64::NAN))
}

/// (matches, total, first differences) of the isolated rules, given Excel's own plot height.
fn scale_only(file: &str) -> Option<(usize, usize, Vec<String>)> {
    let cases = scale_corpus(file)?;
    let (mut ok, mut bad) = (0, Vec::new());
    for c in &cases {
        let v = f64s(&c["values"]);
        let lo = v.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let base = c["base"].as_f64().expect("base");
        let got = log_scale(
            lo,
            hi,
            base,
            log_max_intervals(c["inside_h"].as_f64().expect("inside_h"), 10.0, lo < 1.0),
            match c["kind"].as_str() {
                Some("line") => LogNarrow::Half,
                Some("scatter") => LogNarrow::Whole,
                _ => LogNarrow::Never,
            },
            log_far_intervals(c["inside_h"].as_f64().expect("inside_h"), 10.0),
        );
        if scale_matches(&got, c) {
            ok += 1;
        } else if bad.len() < 15 {
            bad.push(format!("base {base} data {lo}..{hi} got {got:?} want {c}"));
        }
    }
    Some((ok, cases.len(), bad))
}

/// Same through [`ChartBuilder`] and [`render`], so the plot height is ours, not Excel's.
fn end_to_end(file: &str) -> Option<(usize, usize, Vec<String>)> {
    let cases = scale_corpus(file)?;
    let measure = CarlitoMeasure::new();
    let (mut ok, mut bad) = (0, Vec::new());
    for c in &cases {
        let kind = match c["kind"].as_str().expect("kind") {
            "column" => BuildKind::Column,
            "line" => BuildKind::Line,
            _ => BuildKind::Scatter,
        };
        let mut b = ChartBuilder::new(kind);
        let values = f64s(&c["values"]);
        b.categories = (1..=values.len()).map(|i| i.to_string()).collect();
        let xs: Vec<f64> = (1..=values.len()).map(|i| i as f64).collect();
        b.series = vec![BuildSeries {
            name: "Series1".into(),
            values,
            xs,
            sizes: Vec::new(),
        }];
        b.legend = None;
        b.log_base = c["base"].as_f64();
        let space = b.build().expect("builds");
        let list = render(
            &space,
            360.0,
            c["height"].as_f64().expect("height"),
            &RenderOptions::new(&NoCells, &measure),
        );
        if list.value_scale.is_some_and(|s| scale_matches(&s, c)) {
            ok += 1;
        } else if bad.len() < 15 {
            bad.push(format!("{} got {:?} want {c}", c["kind"], list.value_scale));
        }
    }
    Some((ok, cases.len(), bad))
}

/// Fails below `floor` percent. The floors are the measured rates: 100 % where the rules were
/// fitted, and a hair under for the hold-out and end-to-end runs (see `docs/architecture/charts.md`
/// for the two known misses: a narrow band of large values on a scatter chart, and a plot height
/// that lands 0.09 pt on the wrong side of Excel's interval budget).
fn check(label: &str, r: Option<(usize, usize, Vec<String>)>, floor: f64) {
    let Some((ok, total, bad)) = r else {
        eprintln!("{label}: corpus not recorded; skipped");
        return;
    };
    let rate = 100.0 * ok as f64 / total as f64;
    eprintln!("{label}: {ok} of {total} match ({rate:.1} %)");
    assert!(
        rate >= floor,
        "{label}: {ok} of {total}
{}",
        bad.join(
            "
"
        )
    );
}

#[test]
fn log_axis_rules_match_excel_on_the_train_corpus() {
    check(
        "log train (rules, Excel plot height)",
        scale_only("log-train.json"),
        100.0,
    );
}

#[test]
fn log_axis_rules_match_excel_on_the_hold_out_corpus() {
    check(
        "log hold-out (rules, Excel plot height)",
        scale_only("log-holdout.json"),
        99.0,
    );
}

#[test]
fn lognarrow_rules_match_excel_on_the_train_corpus() {
    check(
        "lognarrow train (rules, Excel plot height)",
        scale_only("lognarrow-train.json"),
        97.0,
    );
}

#[test]
fn lognarrow_rules_match_excel_on_the_hold_out_corpus() {
    check(
        "lognarrow hold-out (rules, Excel plot height)",
        scale_only("lognarrow-holdout.json"),
        99.0,
    );
}

#[test]
fn lognarrow_end_to_end() {
    check(
        "lognarrow train (end to end)",
        end_to_end("lognarrow-train.json"),
        97.0,
    );
    check(
        "lognarrow hold-out (end to end)",
        end_to_end("lognarrow-holdout.json"),
        99.0,
    );
}

#[test]
fn log_axis_end_to_end_train() {
    check("log train (end to end)", end_to_end("log-train.json"), 99.0);
}

#[test]
fn log_axis_end_to_end_hold_out() {
    check(
        "log hold-out (end to end)",
        end_to_end("log-holdout.json"),
        99.0,
    );
}
