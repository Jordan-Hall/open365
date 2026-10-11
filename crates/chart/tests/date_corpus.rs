//! Automatic date-axis scaling against Excel: `fixtures/reference/xlsx/chart-scale/date-*.json`
//! (recorded by `tools/office/make-chart-corpus.ps1 -Set date`). `date-train.json` (seed 1001)
//! is the set the rules in [`open365_chart::axis::date_scale`] were fitted on;
//! `date-holdout.json` (seed 2002) was recorded afterwards.

mod common;

use open365_chart::axis::{DateUnit, date_scale};
use serde_json::Value;

use common::{f64s, scale_corpus};

fn unit(code: i64) -> DateUnit {
    match code {
        0 => DateUnit::Days,
        1 => DateUnit::Months,
        _ => DateUnit::Years,
    }
}

fn short(c: &Value) -> String {
    format!(
        "w {} h {} {} points {}",
        c["inside_w"],
        c["inside_h"],
        c["pattern"],
        c["dates"].as_array().map_or(0, Vec::len)
    )
}

/// (matches, total, first differences).
fn replay(file: &str) -> Option<(usize, usize, Vec<String>)> {
    let cases = scale_corpus(file)?;
    let (mut ok, mut bad) = (0, Vec::new());
    for c in &cases {
        let ax = &c["axis"];
        let int = |k: &str| ax[k].as_i64().expect(k);
        let want = (
            ax["min"].as_f64().expect("min"),
            ax["max"].as_f64().expect("max"),
            int("major"),
            unit(int("major_scale")),
            unit(int("base_unit")),
        );
        let got = date_scale(
            &f64s(&c["dates"]),
            c["inside_w"].as_f64().expect("inside_w"),
            c["inside_h"].as_f64().expect("inside_h"),
            10.0,
        )
        .map(|s| (s.min, s.max, i64::from(s.major), s.major_unit, s.base_unit));
        let minor_ok = ax["minor"].as_i64() == Some(1) && int("minor_scale") == int("major_scale");
        if got == Some(want) && minor_ok {
            ok += 1;
        } else if bad.len() < 10 {
            bad.push(format!("got {got:?} want {ax} {}", short(c)));
        }
    }
    Some((ok, cases.len(), bad))
}

fn check(file: &str, floor: f64) {
    let Some((ok, total, bad)) = replay(file) else {
        eprintln!("{file}: corpus not recorded; skipped");
        return;
    };
    let rate = 100.0 * ok as f64 / total as f64;
    eprintln!("{file}: {ok} of {total} match ({rate:.1} %)");
    assert!(rate >= floor, "{file}: {ok} of {total}\n{}", bad.join("\n"));
}

#[test]
fn date_axis_matches_excel_on_the_train_corpus() {
    check("date-train.json", 97.0);
}

#[test]
fn date_axis_matches_excel_on_the_hold_out_corpus() {
    check("date-holdout.json", 98.0);
}
