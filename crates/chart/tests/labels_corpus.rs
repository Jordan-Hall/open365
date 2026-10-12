//! Tick-label texts, data-label texts and data-label rectangles against Excel
//! (`fixtures/reference/xlsx/chart-labels/{train,holdout}.json`).
//!
//! Each case is built with [`ChartBuilder`] and rendered with Excel's own plot rectangle
//! (a manual inner layout), so the label rules are measured apart from the plot-area
//! layout, which `layout_corpus.rs` covers. `train.json` (seed 1001) is the set the rules were
//! fitted on; `holdout.json` (seed 2002) was recorded afterwards. Run with `--nocapture` for
//! the per-quantity and per-position tables and the first failures.

mod common;

use std::collections::BTreeMap;

use open365_chart::{
    AREA_INSET, BuildKind, BuildLabels, BuildSeries, ChartBuilder, NoCells, RenderOptions, render,
};
use serde_json::Value;

use common::{CarlitoMeasure, repo_root};

/// Rectangle tolerance in points.
const TOLERANCE: f64 = 1.0;

/// Share of items (percent) that must match, per quantity prefix: the measured rate on the train
/// set and on the hold-out set, rounded down. A change that lowers one fails the test.
const FLOORS: [(&str, f64, f64); 10] = [
    ("tick_text", 96.0, 97.0),
    ("label_text", 99.0, 99.0),
    ("label_rect.bar", 97.0, 99.0),
    ("label_rect.column", 98.0, 99.0),
    ("label_rect.line", 99.0, 99.0),
    ("label_rect.scatter", 95.0, 99.0),
    ("label_rect.pie.center", 99.0, 96.0),
    ("label_rect.pie.inside_end", 32.0, 0.0),
    ("label_rect.pie.outside_end", 70.0, 47.0),
    ("label_rect.pie.best_fit", 10.0, 7.0),
];

fn load(name: &str) -> Option<Vec<Value>> {
    let path = repo_root()
        .join("fixtures/reference/xlsx/chart-labels")
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

fn kind_of(name: &str) -> BuildKind {
    match name {
        "column" => BuildKind::Column,
        "column_stacked" => BuildKind::ColumnStacked,
        "bar" => BuildKind::Bar,
        "bar_stacked" => BuildKind::BarStacked,
        "line" => BuildKind::Line,
        "scatter" => BuildKind::Scatter,
        "pie" => BuildKind::Pie,
        other => panic!("unknown chart type {other}"),
    }
}

fn dlbl_pos(name: &str) -> &'static str {
    match name {
        "outside_end" => "outEnd",
        "inside_end" => "inEnd",
        "inside_base" => "inBase",
        "center" => "ctr",
        "best_fit" => "bestFit",
        "above" => "t",
        "below" => "b",
        "left" => "l",
        "right" => "r",
        other => panic!("unknown position {other}"),
    }
}

/// Excel plots a scatter case by rows when the table has more columns than rows: one series per
/// category, the X column first as the first point, at X = 1, 2, 3, ... Otherwise the X column
/// is the X values of the series.
fn transposed(c: &Value) -> bool {
    c["type"] == "scatter"
        && (c["data_labels"][0].as_array().map_or(0, Vec::len)
            != c["cats"].as_array().map_or(0, Vec::len)
            || c["data_labels"].as_array().map_or(0, Vec::len)
                != c["series"].as_array().map_or(0, Vec::len))
}

fn series_of(c: &Value) -> Vec<(Vec<f64>, Vec<f64>)> {
    let cols: Vec<Vec<f64>> = c["series"]
        .as_array()
        .expect("series")
        .iter()
        .map(nums)
        .collect();
    if transposed(c) {
        let xs = nums(&c["xs"]);
        (0..xs.len())
            .map(|i| {
                let ys: Vec<f64> = std::iter::once(xs[i])
                    .chain(cols.iter().map(|s| s[i]))
                    .collect();
                ((1..=ys.len()).map(|v| v as f64).collect(), ys)
            })
            .collect()
    } else if c["type"] == "scatter" {
        let xs = nums(&c["xs"]);
        cols.into_iter().map(|ys| (xs.clone(), ys)).collect()
    } else {
        cols.into_iter().map(|ys| (Vec::new(), ys)).collect()
    }
}

fn build(c: &Value) -> (ChartBuilder, f64, f64) {
    let kind = c["type"].as_str().expect("type");
    let mut b = ChartBuilder::new(kind_of(kind));
    let rows = series_of(c);
    b.categories = if kind == "scatter" {
        Vec::new()
    } else {
        c["cats"]
            .as_array()
            .expect("cats")
            .iter()
            .map(|v| v.as_str().expect("string").to_owned())
            .collect()
    };
    b.series = rows
        .into_iter()
        .enumerate()
        .map(|(i, (xs, values))| BuildSeries {
            name: format!("Series{}", i + 1),
            xs,
            values,
            sizes: Vec::new(),
        })
        .collect();
    b.legend = None;
    b.title = None;
    b.bar_first_at_top = false;
    b.cell_format = c["cell_format"].as_str().map(str::to_owned);
    if let Some(va) = c.get("value_axis").filter(|v| !v.is_null()) {
        if va["linked"] == false {
            b.value_axis_format = va["number_format"].as_str().map(str::to_owned);
        } else if transposed(c) {
            // The axis follows the first series' first cell, which is the unformatted X column.
            b.value_axis_format = Some("General".to_owned());
        }
        if let Some(base) = va["log_base"].as_f64() {
            b.log_base = Some(base);
        }
    }
    let content = c["pie_content"].as_str();
    b.labels = Some(BuildLabels {
        position: Some(dlbl_pos(c["position"].as_str().expect("position")).to_owned()),
        show_value: content.is_none() || matches!(content, Some("value" | "category_percent")),
        show_category: content == Some("category_percent"),
        show_percent: matches!(content, Some("percent" | "category_percent")),
        font_pt: c["label_font_size"].as_f64(),
        number_format: (c["label_linked"] == false)
            .then(|| c["label_number_format"].as_str().map(str::to_owned))
            .flatten(),
    });
    let (w, h) = (
        c["width"].as_f64().expect("width"),
        c["height"].as_f64().expect("height"),
    );
    let i = &c["inside"];
    b.inner_layout = Some([
        i["left"].as_f64().expect("left") / (w - AREA_INSET),
        i["top"].as_f64().expect("top") / (h - AREA_INSET),
        i["width"].as_f64().expect("width") / (w - AREA_INSET),
        i["height"].as_f64().expect("height") / (h - AREA_INSET),
    ]);
    (b, w, h)
}

#[derive(Default)]
struct Stats {
    /// quantity -> offsets (got - want) of left, top, width and height
    deltas: BTreeMap<String, Vec<[f64; 4]>>,
    /// quantity -> (matched, total)
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
        if !ok && seen < 4 {
            self.failures.push(format!("{quantity}: {}", why()));
        }
    }

    fn rate(&self, prefix: &str) -> f64 {
        let (a, b) = self
            .by_quantity
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .fold((0, 0), |s, (_, v)| (s.0 + v.0, s.1 + v.1));
        100.0 * a as f64 / b.max(1) as f64
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= TOLERANCE
}

fn evaluate(file: &str) -> Option<Stats> {
    let cases = load(file)?;
    let measure = CarlitoMeasure::new();
    let mut st = Stats::default();
    for (n, c) in cases.iter().enumerate() {
        let (b, w, h) = build(c);
        let space = b.build().expect("builds");
        let list = render(&space, w, h, &RenderOptions::new(&NoCells, &measure));
        let kind = c["type"].as_str().expect("type");
        let pos = c["position"].as_str().expect("position");
        let tag = format!("#{n} {kind} {pos}");

        if let Some(va) = c.get("value_axis").filter(|v| !v.is_null()) {
            let want: Vec<&str> = va["tick_texts"]
                .as_array()
                .expect("tick_texts")
                .iter()
                .filter_map(Value::as_str)
                .collect();
            let got: Vec<&str> = list.value_tick_texts.iter().map(String::as_str).collect();
            st.hit("tick_text.value", got == want, || {
                format!(
                    "{tag} fmt {} got {got:?} want {want:?}",
                    va["number_format"]
                )
            });
        }
        if let Some(xa) = c.get("x_axis").filter(|v| !v.is_null()) {
            let want: Vec<&str> = xa["tick_texts"]
                .as_array()
                .expect("tick_texts")
                .iter()
                .filter_map(Value::as_str)
                .collect();
            let got: Vec<&str> = list.x_tick_texts.iter().map(String::as_str).collect();
            st.hit("tick_text.x", got == want, || {
                format!("{tag} got {got:?} want {want:?}")
            });
        }

        let want_labels = c["data_labels"].as_array().expect("data_labels");
        for (si, series) in want_labels.iter().enumerate() {
            for (pi, want) in series.as_array().expect("series labels").iter().enumerate() {
                let got = list
                    .data_labels
                    .iter()
                    .find(|l| l.series == si && l.point == pi);
                // Scatter cases carry the X column as the first point, which has no cell
                // format of its own here.
                let skip_text = transposed(c) && pi == 0;
                // Excel (en-GB) shows a bare "$" in a label format it was handed as the local
                // currency symbol, so those texts are not comparable.
                let local_currency = c["label_linked"] == false
                    && c["label_number_format"]
                        .as_str()
                        .is_some_and(|f| f.replace(r"\$", "").contains('$'));
                if !skip_text && !local_currency && !want["width"].is_null() {
                    let want_text = want["text"].as_str().unwrap_or("");
                    st.hit(
                        &format!("label_text.{kind}"),
                        got.is_some_and(|g| g.text == want_text),
                        || {
                            format!(
                                "{tag} fmt {} linked {} got {:?} want {want_text:?}",
                                c["label_number_format"],
                                c["label_linked"],
                                got.map(|g| &g.text)
                            )
                        },
                    );
                }
                if skip_text {
                    continue;
                }
                let (l, t, ww, hh) = (
                    want["left"].as_f64().unwrap_or(f64::NAN),
                    want["top"].as_f64().unwrap_or(f64::NAN),
                    want["width"].as_f64().unwrap_or(f64::NAN),
                    want["height"].as_f64().unwrap_or(f64::NAN),
                );
                if want["width"].is_null() {
                    continue;
                }
                if let Some(g) = got {
                    st.deltas.entry(format!("{kind}.{pos}")).or_default().push([
                        g.rect.x - l,
                        g.rect.y - t,
                        g.rect.width - ww,
                        g.rect.height - hh,
                    ]);
                }
                let ok = got.is_some_and(|g| {
                    near(g.rect.x, l)
                        && near(g.rect.y, t)
                        && near(g.rect.width, ww)
                        && near(g.rect.height, hh)
                });
                st.hit(&format!("label_rect.{kind}.{pos}"), ok, || {
                    format!(
                        "{tag} s{si} p{pi} got {:?} want ({l:.1},{t:.1},{ww:.1},{hh:.1})",
                        got.map(|g| (
                            (g.rect.x * 10.0).round() / 10.0,
                            (g.rect.y * 10.0).round() / 10.0,
                            (g.rect.width * 10.0).round() / 10.0,
                            (g.rect.height * 10.0).round() / 10.0
                        ))
                    )
                });
            }
        }
    }
    Some(st)
}

fn report(label: &str, st: &Stats) {
    eprintln!("{label}:");
    for (q, (a, b)) in &st.by_quantity {
        eprintln!("  {q}: {a} of {b} ({:.1} %)", 100.0 * *a as f64 / *b as f64);
    }
    for (k, d) in &st.deltas {
        let range = |i: usize| {
            let lo = d.iter().map(|v| v[i]).fold(f64::INFINITY, f64::min);
            let hi = d.iter().map(|v| v[i]).fold(f64::NEG_INFINITY, f64::max);
            format!("{lo:.1}..{hi:.1}")
        };
        eprintln!(
            "  delta {k}: n={} dx {} dy {} dw {} dh {}",
            d.len(),
            range(0),
            range(1),
            range(2),
            range(3)
        );
    }
    for f in &st.failures {
        eprintln!("  FAIL {f}");
    }
}

fn check(file: &str, label: &str, holdout: bool) {
    let Some(st) = evaluate(file) else {
        eprintln!("{file} not recorded; skipped");
        return;
    };
    report(label, &st);
    for (prefix, train, hold) in FLOORS {
        let rate = st.rate(prefix);
        eprintln!("  group {prefix}: {rate:.1} %");
        assert!(
            rate >= if holdout { hold } else { train },
            "{label} {prefix}: {rate:.1} %"
        );
    }
}

#[test]
fn label_rules_match_excel_on_the_train_corpus() {
    check("train.json", "train", false);
}

#[test]
fn label_rules_hold_out() {
    check("holdout.json", "hold-out", true);
}
