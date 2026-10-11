//! `ERF` and `ERFC` against 3,500 values measured in Excel (`fixtures/reference/calc/erf-samples.txt`).
//!
//! Excel's own approximation is a few ulp off the exact value and no published coefficient set reproduces it, so
//! the engine's results are not bit-identical to Excel's. This test pins what is known: how far they may differ and
//! how often they are identical, so a change that makes the model worse (or better) is visible.

use open365_calc::{Engine, Value};
use std::fs;
use std::path::PathBuf;

fn samples() -> Vec<(String, f64, f64)> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/reference/calc/erf-samples.txt");
    fs::read_to_string(path)
        .expect("samples")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let mut p = l.split_whitespace();
            let f = p.next().expect("sample").to_string();
            let x = p.next().expect("sample").parse().expect("sample");
            let y = p.next().expect("sample").parse().expect("sample");
            (f, x, y)
        })
        .collect()
}

fn ulps(a: f64, b: f64) -> u64 {
    let ord = |v: f64| {
        let bits = v.to_bits() as i64;
        if bits < 0 { i64::MIN - bits } else { bits }
    };
    ord(a).abs_diff(ord(b))
}

#[test]
fn erf_and_erfc_stay_close_to_excel() {
    let all = samples();
    let mut engine = Engine::new();
    engine.add_sheet("Sheet1");
    for (i, (f, x, _)) in all.iter().enumerate() {
        let row = u32::try_from(i).expect("sample") + 1;
        engine.set_value("Sheet1", row, 1, Value::Number(*x));
        engine
            .set_formula("Sheet1", row, 2, &format!("={f}(A{row})"))
            .expect("sample");
    }
    engine.recalc_all();
    let mut stats = std::collections::BTreeMap::<String, (usize, usize, u64)>::new();
    for (i, (f, x, y)) in all.iter().enumerate() {
        let row = u32::try_from(i).expect("sample") + 1;
        let Value::Number(got) = engine.get("Sheet1", row, 2) else {
            panic!("{f}({x}) is not a number");
        };
        let d = ulps(got, *y);
        let e = stats.entry(f.clone()).or_default();
        e.0 += 1;
        e.1 += usize::from(d == 0);
        e.2 = e.2.max(d);
    }
    for (f, (n, same, worst)) in &stats {
        println!("{f}: {n} samples, {same} identical, worst {worst} ulp");
    }
    let (n, same, worst) = stats["ERF"];
    assert!(worst <= 4, "ERF worst {worst} ulp");
    assert!(same * 100 >= n * 70, "ERF identical {same} of {n}");
    let (n, same, worst) = stats["ERFC"];
    assert!(worst <= 20, "ERFC worst {worst} ulp");
    assert!(same * 100 >= n * 50, "ERFC identical {same} of {n}");
}
