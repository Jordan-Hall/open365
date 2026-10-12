//! Automatic axis scaling against Excel: every case of `fixtures/reference/xlsx/chart-axes`
//! (recorded by `tools/office/chart-oracle.ps1 -Axes -Sweep`) is scaled with
//! [`open365_chart::axis`] and compared with `Axes(xlValue).MinimumScale/MaximumScale/MajorUnit`.

use std::path::PathBuf;

use open365_chart::axis::{auto_scale, data_extent, max_intervals};
use open365_chart::{FontSpec, TextMeasure};

mod common;
use common::CarlitoMeasure;
use serde_json::Value;

/// `PlotArea` outer height minus inside height for one line of 10 pt category labels.
const OUTER_GAP: f64 = 14.2;
/// The same when all-negative data move the category labels into the plot.
const OUTER_GAP_MOVED: f64 = 0.2;
/// Width of the category-label region left of a sweep bar chart (feasible range 6 to 31 pt).
const SWEEP_BAR_LABELS: f64 = 10.0;
/// Gap between the value-axis labels and the plot, added to the widest label.
const VAL_LABEL_PAD: f64 = 4.73;

fn load(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/reference/xlsx/chart-axes")
        .join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).expect("valid JSON")
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

fn nums(v: &Value) -> Vec<f64> {
    v.as_array()
        .expect("array")
        .iter()
        .map(|x| x.as_f64().expect("number"))
        .collect()
}

fn f(v: &Value, key: &str) -> f64 {
    v[key].as_f64().unwrap_or_else(|| panic!("{key}"))
}

/// Replays one recorded corpus; returns (matches, total, differences).
fn replay(file: &str) -> (usize, usize, Vec<String>) {
    let measure = CarlitoMeasure::new();
    let font = FontSpec {
        family: "Calibri".into(),
        size_pt: 10.0,
        bold: false,
        italic: false,
    };
    let doc = load(file);
    let cases = doc["cases"].as_array().expect("cases");
    let (mut ok, mut bad) = (0usize, Vec::new());
    let mut x_ok = 0usize;
    let mut x_total = 0usize;
    for c in cases {
        let kind = c["kind"].as_str().expect("kind");
        let series: Vec<Vec<Option<f64>>> = c["series"]
            .as_array()
            .expect("series")
            .iter()
            .map(|s| nums(s).into_iter().map(Some).collect())
            .collect();
        let extent = data_extent(&series, kind == "stacked").expect("data");
        let want = &c["value_axis"];
        // All-negative data pull the category labels into the plot, so the outer rectangle is
        // barely larger than the inside one.
        let moved = f(want, "max") <= 0.0 && f(want, "min") < 0.0;
        let outer = f(c, "inside_h") + if moved { OUTER_GAP_MOVED } else { OUTER_GAP };
        let got = auto_scale(extent.0, extent.1, max_intervals(outer, false, 10.0));
        let hit = close(got.min, f(want, "min"))
            && close(got.max, f(want, "max"))
            && close(got.major, f(want, "major"))
            && close(got.minor, f(want, "minor"));
        if hit {
            ok += 1;
        } else {
            bad.push(format!(
                "{kind} h={} data={extent:?} got=({},{},{},{}) want={want}",
                c["height"], got.min, got.max, got.major, got.minor
            ));
        }
        if kind == "scatter" {
            x_total += 1;
            let xs = nums(&c["xs"]);
            let (lo, hi) = (
                xs.iter().cloned().fold(f64::INFINITY, f64::min),
                xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            );
            // The outer width adds the Y-axis labels (left of the plot) to the inside width.
            let wy = &c["value_axis"];
            let (ymin, ymax, ymajor) = (f(wy, "min"), f(wy, "max"), f(wy, "major"));
            let steps = ((ymax - ymin) / ymajor).round() as i32;
            let widest = (0..=steps)
                .map(|i| {
                    let v = ((ymin + f64::from(i) * ymajor) * 1e9).round() / 1e9;
                    let mut t = String::new();
                    open365_xlsx::format_number(v, &mut t);
                    measure.width(&t, &font)
                })
                .fold(0.0, f64::max);
            let wx = &c["x_axis"];
            let mut last = String::new();
            open365_xlsx::format_number(f(wx, "max"), &mut last);
            // The last X label is centred on the right edge of the plot and overhangs it.
            let outer_w =
                f(c, "inside_w") + widest + VAL_LABEL_PAD + measure.width(&last, &font) / 2.0;
            let gx = auto_scale(lo, hi, max_intervals(outer_w, true, 10.0));
            let wx = &c["x_axis"];
            if close(gx.min, f(wx, "min"))
                && close(gx.max, f(wx, "max"))
                && close(gx.major, f(wx, "major"))
            {
                x_ok += 1;
            } else {
                bad.push(format!(
                    "scatter x data=({lo},{hi}) got=({},{},{}) want={wx}",
                    gx.min, gx.max, gx.major
                ));
            }
        }
    }
    let total = cases.len() + x_total;
    eprintln!(
        "{file}: {} of {total} match ({x_ok} of {x_total} scatter X axes)",
        ok + x_ok
    );
    (ok + x_ok, total, bad)
}

#[test]
fn value_axis_matches_excel_on_the_generated_corpus() {
    let (_, total, bad) = replay("axes.json");
    assert!(total >= 300, "corpus size {total}");
    assert!(
        bad.is_empty(),
        "{} of {total} differ:
{}",
        bad.len(),
        bad.join(
            "
"
        )
    );
}

/// `axes-seed7.json` was recorded after the rules were written and is a different random
/// sweep (`chart-oracle.ps1 -Axes -Seed 7`).
#[test]
fn value_axis_matches_excel_on_the_hold_out_sweep() {
    let (ok, total, bad) = replay("axes-seed7.json");
    assert!(total >= 300, "corpus size {total}");
    assert!(
        bad.is_empty(),
        "{} of {total} differ ({ok} match):
{}",
        bad.len(),
        bad.join(
            "
"
        )
    );
}

#[test]
fn interval_budget_matches_excel_across_plot_sizes() {
    let doc = load("sweep.json");
    let mut bad = Vec::new();
    let cases = doc["cases"].as_array().expect("cases");
    for c in cases {
        let kind = c["kind"].as_str().expect("kind");
        let horizontal = kind == "bar";
        let len = if horizontal {
            f(c, "inside_w")
        } else {
            f(c, "inside_h")
        };
        if len < 30.0 {
            continue; // degenerate plot: Excel stops following its own rules
        }
        let v = f(c, "v");
        // The sweep records only the inside rectangle. Its category labels are the single
        // digits "1" and "2" (about 10 pt of width with the gap) under a bar chart.
        let outer = if horizontal {
            len + SWEEP_BAR_LABELS
        } else {
            len + OUTER_GAP
        };
        let got = auto_scale(0.0, v, max_intervals(outer, horizontal, 10.0));
        if !(close(got.min, f(c, "min"))
            && close(got.max, f(c, "max"))
            && close(got.major, f(c, "major")))
        {
            bad.push(format!(
                "{kind} len={len:.1} v={v} got=({},{},{}) want=({},{},{})",
                got.min,
                got.max,
                got.major,
                f(c, "min"),
                f(c, "max"),
                f(c, "major")
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {} differ:\n{}",
        bad.len(),
        cases.len(),
        bad.join("\n")
    );
}

/// The interval budget follows the plot area's OUTER rectangle: replays the recorded
/// `chart-scale/layout-{train,holdout}.json` with Excel's own outer length. Horizontal axes
/// whose labels have six or more digits are left out: Excel gives those more room than the
/// 38.2 pt per interval of shorter labels and no rule is fitted (see `docs/architecture/charts.md`).
#[test]
fn interval_budget_follows_the_outer_plot_rectangle() {
    for (file, label) in [
        ("layout-train.json", "train"),
        ("layout-holdout.json", "hold-out"),
    ] {
        let Some(cases) = common::scale_corpus(file) else {
            eprintln!("{file} not recorded; skipped");
            continue;
        };
        let (mut total, mut bad) = (0usize, Vec::new());
        for (n, c) in cases.iter().enumerate() {
            let kind = c["type"].as_str().expect("type");
            let horizontal = matches!(kind, "bar" | "bar_stacked");
            let vertical = matches!(
                kind,
                "column" | "column_stacked" | "line" | "line_marker" | "area" | "area_stacked"
            );
            let want = &c["value_axis"];
            if (!horizontal && !vertical) || want.is_null() {
                continue;
            }
            if horizontal && f(want, "max").abs().max(f(want, "min").abs()) >= 100_000.0 {
                continue;
            }
            let series: Vec<Vec<Option<f64>>> = c["series"]
                .as_array()
                .expect("series")
                .iter()
                .map(|s| nums(s).into_iter().map(Some).collect())
                .collect();
            let Some(extent) = data_extent(&series, kind.ends_with("stacked")) else {
                continue;
            };
            if extent.0 == extent.1 {
                continue;
            }
            let outer = if horizontal {
                f(&c["outer"], "width")
            } else {
                f(&c["outer"], "height")
            };
            let got = auto_scale(extent.0, extent.1, max_intervals(outer, horizontal, 10.0));
            total += 1;
            if !(close(got.min, f(want, "min"))
                && close(got.max, f(want, "max"))
                && close(got.major, f(want, "major")))
            {
                bad.push(format!(
                    "#{n} {kind} outer={outer:.1} data={extent:?} got=({},{},{}) want={want}",
                    got.min, got.max, got.major
                ));
            }
        }
        eprintln!(
            "{label} (Excel outer length): {} of {total} match",
            total - bad.len()
        );
        assert!(total > 400, "{label}: only {total} cases");
        assert!(
            bad.is_empty(),
            "{label}: {} differ:\n{}",
            bad.len(),
            bad.join("\n")
        );
    }
}
