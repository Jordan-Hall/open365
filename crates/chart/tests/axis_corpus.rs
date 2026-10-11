//! Automatic axis scaling against Excel: every case of `fixtures/reference/xlsx/chart-axes`
//! (recorded by `tools/office/chart-oracle.ps1 -Axes -Sweep`) is scaled with
//! [`open365_chart::axis`] and compared with `Axes(xlValue).MinimumScale/MaximumScale/MajorUnit`.

use std::path::PathBuf;

use open365_chart::axis::{auto_scale, data_extent, max_intervals};
use serde_json::Value;

/// Height Excel reserves under the plot for 10 pt category labels.
const LABEL_BAND: f64 = 13.86;

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

#[test]
fn value_axis_matches_excel_on_the_generated_corpus() {
    let doc = load("axes.json");
    let cases = doc["cases"].as_array().expect("cases");
    assert!(cases.len() >= 300, "corpus size {}", cases.len());
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
        // All-negative data pull the category labels into the plot (taller inside height).
        let moved = f(want, "max") <= 0.0 && f(want, "min") < 0.0;
        let inside = f(c, "inside_h") - if moved { LABEL_BAND } else { 0.0 };
        let got = auto_scale(extent.0, extent.1, max_intervals(inside, false, 10.0));
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
            let gx = auto_scale(lo, hi, max_intervals(f(c, "inside_w"), true, 10.0));
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
        "axis corpus: {} of {total} match ({x_ok} of {x_total} scatter X axes)",
        ok + x_ok
    );
    assert!(
        bad.is_empty(),
        "{} of {total} differ:\n{}",
        bad.len(),
        bad.join("\n")
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
        let got = auto_scale(0.0, v, max_intervals(len, horizontal, 10.0));
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
