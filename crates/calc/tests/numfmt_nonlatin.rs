//! Thai, Buddhist, number-shape, calendar-tag, `B1`/`B2` and DBNum number formats against Excel.
//!
//! `fixtures/calc/numfmt-nonlatin/cases.json` holds the probes and `results.json` what Excel gave for each
//! (`tools/office/make-numfmt-calc-refs.ps1`, Excel 16.0 on an en-GB machine, `TEXT` and `Range.Text`). Cases that
//! Open365 still renders differently are listed in `known.txt`; the list can only shrink (a case that passes while
//! listed fails the test). Set `NUMFMT_WRITE_KNOWN=1` to rewrite it after a deliberate change.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::collections::BTreeSet;

use open365_calc::numfmt::{format_value_in, text_function_in};
use open365_calc::{FormatLocale, Value};

const CASES: &str = include_str!("../../../fixtures/calc/numfmt-nonlatin/cases.json");
const RESULTS: &str = include_str!("../../../fixtures/calc/numfmt-nonlatin/results.json");
const KNOWN: &str = include_str!("../../../fixtures/calc/numfmt-nonlatin/known.txt");

fn run() -> (usize, BTreeSet<String>) {
    let cases: Vec<serde_json::Value> = serde_json::from_str(CASES).unwrap();
    let results: Vec<serde_json::Value> = serde_json::from_str(RESULTS).unwrap();
    assert_eq!(cases.len(), results.len());
    let gb = FormatLocale::for_lcid(0x0809).unwrap();
    let mut failed = BTreeSet::new();
    for (case, result) in cases.iter().zip(&results) {
        let id = case["id"].as_str().unwrap();
        assert_eq!(result["id"].as_str().unwrap(), id);
        let code = case["code"].as_str().unwrap();
        let value = match case.get("value") {
            Some(v) => Value::Number(v.as_f64().unwrap()),
            None => Value::Text(case["text"].as_str().unwrap().into()),
        };
        let tfn_ok = match text_function_in(&value, code, false, &gb) {
            Ok(Some(s)) => {
                result
                    .get("tfn_error")
                    .is_none_or(|e| e.as_bool() != Some(true))
                    && result["tfn"].as_str() == Some(&s)
            }
            Ok(None) => false,
            Err(_) => {
                result.get("tfn_rejected").is_some() || result["tfn"].as_str() == Some("#VALUE!")
            }
        };
        let cell_ok = match result["text"].as_str() {
            Some(want) => open365_calc::numfmt::try_format_value_in(&value, code, false, &gb)
                .is_ok_and(|s| s == want || format_value_in(&value, code, false, &gb) == want),
            None => open365_calc::numfmt::validate_format(code).is_err(),
        };
        if !(tfn_ok && cell_ok) {
            failed.insert(id.to_owned());
        }
    }
    (cases.len(), failed)
}

#[test]
fn nonlatin_formats_match_excel_except_the_known_list() {
    let (total, failed) = run();
    if std::env::var_os("NUMFMT_WRITE_KNOWN").is_some() {
        let text: String = failed.iter().map(|id| format!("{id}\n")).collect();
        std::fs::write(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/calc/numfmt-nonlatin/known.txt"
            ),
            text,
        )
        .unwrap();
        return;
    }
    let known: BTreeSet<String> = KNOWN.lines().map(str::to_owned).collect();
    let new: Vec<_> = failed.difference(&known).collect();
    let fixed: Vec<_> = known.difference(&failed).collect();
    assert!(
        new.is_empty(),
        "{} of {total} cases differ from Excel: {new:?}",
        new.len()
    );
    assert!(
        fixed.is_empty(),
        "now passing, remove from known.txt: {fixed:?}"
    );
}
