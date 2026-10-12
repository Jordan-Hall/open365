//! Calendars and names measured for runs of consecutive (or evenly spaced) serial numbers.
//!
//! `fixtures/calc/numfmt-nonlatin/series.json` holds what `make-numfmt-calc-refs.ps1 -Mode Series` got from Excel 16.0
//! (en-GB) for `TEXT(start + i * step, code)`: the lunisolar calendars (byte 14 to 19) for the whole date range, the Persian
//! leap years, and the month and weekday names of
//! the calendars with their own names. Every element must match; the Hebrew calendar from year 5999 on is not measured here
//! (see `known.txt` of the numfmt cases).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use open365_calc::numfmt::text_function_in;
use open365_calc::{FormatLocale, Value};

const SERIES: &str = include_str!("../../../fixtures/calc/numfmt-nonlatin/series.json");

#[test]
fn series_match_excel() {
    let series: Vec<serde_json::Value> = serde_json::from_str(SERIES).unwrap();
    let gb = FormatLocale::for_lcid(0x0809).unwrap();
    let (mut total, mut wrong) = (0usize, Vec::new());
    for case in &series {
        let id = case["id"].as_str().unwrap();
        let code = case["code"].as_str().unwrap();
        let start = case["start"].as_f64().unwrap();
        let step = case["step"].as_f64().unwrap();
        for (i, want) in case["values"].as_array().unwrap().iter().enumerate() {
            let want = want.as_str().unwrap();
            let serial = start + i as f64 * step;
            // A code with `|` stands for several codes whose results Excel joined with `|` (one cell per probe).
            let tag_end = code.find(']').map_or(0, |i| i + 1);
            let got = code[tag_end..]
                .split('|')
                .map(|part| {
                    match text_function_in(
                        &Value::Number(serial),
                        &format!("{}{part}", &code[..tag_end]),
                        false,
                        &gb,
                    ) {
                        Ok(Some(s)) => s,
                        Ok(None) | Err(_) => "#VALUE!".to_owned(),
                    }
                })
                .collect::<Vec<_>>()
                .join("|");
            total += 1;
            if got != want {
                if wrong.len() < 25 {
                    wrong.push(format!(
                        "{id} serial {serial}: Excel {want:?}, Open365 {got:?}"
                    ));
                } else {
                    wrong.push(String::new());
                }
            }
        }
    }
    let count = wrong.len();
    wrong.retain(|w| !w.is_empty());
    assert!(
        count == 0,
        "{count} of {total} series values differ from Excel; first ones:\n{}",
        wrong.join("\n")
    );
}
