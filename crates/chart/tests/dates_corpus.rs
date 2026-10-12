//! Date category axes against Excel (`fixtures/reference/xlsx/chart-dates/{train,holdout}.json`):
//! the resolved scale, the major tick serials, their label texts and positions, and the
//! centre of every plotted point. Each case is built with [`ChartBuilder`] and rendered with
//! Excel's own plot rectangle (a manual inner layout).
//!
//! `train.json` (seed 1001) is the set the rules were fitted on; `holdout.json` (seed 2002)
//! was recorded afterwards. Run with `--nocapture` for the tables and the first failures.

mod common;

use std::collections::BTreeMap;

use open365_chart::axis::DateUnit;
use open365_chart::{
    AREA_INSET, BuildDates, BuildKind, BuildSeries, ChartBuilder, NoCells, RenderOptions, render,
};
use serde_json::Value;

use common::{CarlitoMeasure, repo_root};

/// Position tolerance in points.
const TOLERANCE: f64 = 0.5;

/// Measured rates (train, hold-out), rounded down.
const FLOORS: [(&str, f64, f64); 5] = [
    ("scale", 92.0, 91.0),
    ("tick_serials", 92.0, 91.0),
    ("tick_texts", 92.0, 91.0),
    ("tick_x", 92.0, 90.0),
    ("point_x", 98.0, 99.0),
];

fn load(name: &str) -> Option<Vec<Value>> {
    let path = repo_root()
        .join("fixtures/reference/xlsx/chart-dates")
        .join(name);
    let text = std::fs::read_to_string(path).ok()?;
    let doc: Value = serde_json::from_str(&text).expect("valid JSON");
    Some(doc["cases"].as_array().expect("cases").clone())
}

fn nums(v: &Value) -> Vec<f64> {
    v.as_array()
        .expect("array")
        .iter()
        .map(|x| x.as_f64().expect("number"))
        .collect()
}

fn unit(code: i64) -> DateUnit {
    match code {
        0 => DateUnit::Days,
        1 => DateUnit::Months,
        _ => DateUnit::Years,
    }
}

#[derive(Default)]
struct Stats {
    by_quantity: BTreeMap<String, (usize, usize)>,
    failures: Vec<String>,
}

impl Stats {
    fn hit(&mut self, quantity: &str, ok: bool, why: impl FnOnce() -> String) {
        let e = self.by_quantity.entry(quantity.to_owned()).or_default();
        e.1 += 1;
        e.0 += usize::from(ok);
        let seen = self
            .failures
            .iter()
            .filter(|f| f.starts_with(quantity))
            .count();
        if !ok && seen < 5 {
            self.failures.push(format!("{quantity}: {}", why()));
        }
    }

    fn rate(&self, quantity: &str) -> f64 {
        let (a, b) = self.by_quantity.get(quantity).copied().unwrap_or((0, 0));
        100.0 * a as f64 / b.max(1) as f64
    }
}

fn evaluate(file: &str) -> Option<Stats> {
    let cases = load(file)?;
    let measure = CarlitoMeasure::new();
    let mut st = Stats::default();
    for (n, c) in cases.iter().enumerate() {
        let kind = c["kind"].as_str().expect("kind");
        let mut b = ChartBuilder::new(if kind == "column" {
            BuildKind::Column
        } else {
            BuildKind::Line
        });
        let dates = nums(&c["dates"]);
        let ax = &c["axis"];
        b.dates = Some(BuildDates {
            serials: dates.clone(),
            // A linked axis shows the cells' format, which Excel reports as the axis format
            // (its en-GB short date `dd/mm/yyyy` is reported as `m/d/yyyy`).
            cell_format: if ax["linked"] == true {
                ax["number_format"].as_str().expect("format").to_owned()
            } else {
                c["cell_format"].as_str().expect("format").to_owned()
            },
            axis_format: (ax["linked"] == false)
                .then(|| ax["number_format"].as_str().map(str::to_owned))
                .flatten(),
            base_unit_days: c["base_unit_forced_days"] == true,
        });
        b.series = vec![BuildSeries {
            name: "Series1".into(),
            values: (1..=dates.len()).map(|v| v as f64).collect(),
            xs: Vec::new(),
            sizes: Vec::new(),
        }];
        b.legend = None;
        let w = c["width"].as_f64().expect("width");
        let i = &c["inside"];
        let (left, top, iw, ih) = (
            i["left"].as_f64().expect("left"),
            i["top"].as_f64().expect("top"),
            i["width"].as_f64().expect("width"),
            i["height"].as_f64().expect("height"),
        );
        // The case does not record the chart height; room for the labels below the plot is
        // enough, since only the horizontal layout is compared.
        let h = top + ih + 40.0;
        b.inner_layout = Some([
            left / (w - AREA_INSET),
            top / (h - AREA_INSET),
            iw / (w - AREA_INSET),
            ih / (h - AREA_INSET),
        ]);
        let space = b.build().expect("builds");
        let list = render(&space, w, h, &RenderOptions::new(&NoCells, &measure));
        let tag = format!("#{n} {kind} {} {}", c["pattern"], c["cell_format"]);

        let int = |k: &str| ax[k].as_i64().expect(k);
        let want = (
            ax["min"].as_f64().expect("min"),
            ax["max"].as_f64().expect("max"),
            int("major"),
            unit(int("major_scale")),
            unit(int("base_unit")),
        );
        let got = list
            .date_scale
            .map(|s| (s.min, s.max, i64::from(s.major), s.major_unit, s.base_unit));
        st.hit("scale", got == Some(want), || {
            format!("{tag} got {got:?} want {want:?}")
        });

        let ticks = &c["ticks"];
        let want_serials = nums(&ticks["serials"]);
        let got_serials: Vec<f64> = list.date_ticks.iter().map(|t| t.serial).collect();
        st.hit("tick_serials", got_serials == want_serials, || {
            format!("{tag} got {got_serials:?} want {want_serials:?}")
        });
        let want_texts: Vec<&str> = ticks["texts"]
            .as_array()
            .expect("texts")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let got_texts: Vec<&str> = list.date_ticks.iter().map(|t| t.text.as_str()).collect();
        st.hit("tick_texts", got_texts == want_texts, || {
            format!(
                "{tag} fmt {} got {got_texts:?} want {want_texts:?}",
                ax["number_format"]
            )
        });
        let want_x = nums(&ticks["x_pt"]);
        let got_x: Vec<f64> = list.date_ticks.iter().map(|t| t.x).collect();
        // `x_pt` is a least-squares fit of the label centres; allow its residual.
        let slack = c["fit"]["max_residual"].as_f64().unwrap_or(0.0);
        st.hit(
            "tick_x",
            got_x.len() == want_x.len()
                && got_x
                    .iter()
                    .zip(&want_x)
                    .all(|(g, w)| (g - w).abs() <= TOLERANCE + slack),
            || format!("{tag} got {got_x:.1?} want {want_x:.1?}"),
        );
        // The reference points are data-label centres, which Excel pushes inside the chart
        // area; the last point can be moved, so it is left out.
        let mut want_pts = nums(&c["points"]["x_centre_pt"]);
        let mut got_pts = list.category_x.clone();
        want_pts.pop();
        got_pts.pop();
        let got_pts = &got_pts;
        st.hit(
            "point_x",
            got_pts.len() == want_pts.len()
                && got_pts
                    .iter()
                    .zip(&want_pts)
                    .all(|(g, w)| (g - w).abs() <= TOLERANCE),
            || format!("{tag} got {got_pts:.1?} want {want_pts:.1?}"),
        );
    }
    Some(st)
}

fn check(file: &str, label: &str, holdout: bool) {
    let Some(st) = evaluate(file) else {
        eprintln!("{file} not recorded; skipped");
        return;
    };
    eprintln!("{label}:");
    for (q, (a, b)) in &st.by_quantity {
        eprintln!("  {q}: {a} of {b} ({:.1} %)", 100.0 * *a as f64 / *b as f64);
    }
    for f in &st.failures {
        eprintln!("  FAIL {f}");
    }
    for (q, train, hold) in FLOORS {
        assert!(
            st.rate(q) >= if holdout { hold } else { train },
            "{label} {q}: {:.1} %",
            st.rate(q)
        );
    }
}

#[test]
fn date_axis_matches_excel_on_the_train_corpus() {
    check("train.json", "train", false);
}

#[test]
fn date_axis_matches_excel_on_the_hold_out_corpus() {
    check("holdout.json", "hold-out", true);
}
