//! The built-in number format ids against Excel's own rendering, in the one locale Excel can be measured in
//! without changing the machine's regional settings (the oracle's locale: en-GB).
//!
//! `fixtures/reference/xlsx/numfmt-locale/measured-en-GB.json` is recorded by
//! `tools/office/make-numfmt-locale-refs.ps1 -Measure`: a workbook whose cell formats reference numFmtId 0..100
//! with no `numFmts` part, opened in Excel; per id the `NumberFormat`, `NumberFormatLocal` and the text of four
//! probe values. Every id must resolve to the code Excel reports, and the code must render the probes as Excel
//! did.

#![allow(clippy::unwrap_used)]

use open365_calc::numfmt::format_value_in;
use open365_calc::{FormatLocale, Value as Calc};
use open365_xlsx::builtin_number_format_in;
use serde_json::Value;

fn measured() -> Value {
    let path = format!(
        "{}/../../fixtures/reference/xlsx/numfmt-locale/measured-en-GB.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn lcid_of(m: &Value) -> u32 {
    u32::try_from(m["windowsLcid"].as_u64().unwrap()).unwrap()
}

#[test]
fn the_fixture_is_a_measurement_of_en_gb() {
    let m = measured();
    assert_eq!(m["label"], "measured");
    assert_eq!(m["windowsLocale"], "en-GB");
    assert_eq!(lcid_of(&m), 0x0809);
    assert_eq!(m["ids"].as_array().unwrap().len(), 101);
}

#[test]
fn every_id_resolves_to_the_code_excel_reports_under_en_gb() {
    let m = measured();
    let lcid = lcid_of(&m);
    for entry in m["ids"].as_array().unwrap() {
        let id = u32::try_from(entry["id"].as_u64().unwrap()).unwrap();
        let excel = entry["numberFormat"].as_str().unwrap();
        // Ids that are not built in render as General.
        let ours = builtin_number_format_in(id, lcid).unwrap_or_else(|| "General".to_owned());
        assert_eq!(ours, excel, "numFmtId {id}");
        assert_eq!(
            excel,
            entry["numberFormatLocal"].as_str().unwrap(),
            "id {id}"
        );
    }
}

#[test]
fn the_resolved_codes_render_the_probes_as_excel_did() {
    let m = measured();
    let lcid = lcid_of(&m);
    let locale = FormatLocale::for_lcid(lcid).unwrap();
    let probes = m["probes"].as_object().unwrap();
    let mut compared = 0;
    for entry in m["ids"].as_array().unwrap() {
        let id = u32::try_from(entry["id"].as_u64().unwrap()).unwrap();
        // Accounting formats pad to the column width, which depends on glyph widths (covered by the calc
        // number-format conformance tests), so only the unpadded kinds are compared here.
        if (41..=44).contains(&id) {
            continue;
        }
        let code = builtin_number_format_in(id, lcid).unwrap_or_else(|| "General".to_owned());
        for (key, value) in probes {
            let want = entry["text"][key].as_str().unwrap();
            // Excel shows `#` runs for a negative date and for a value too wide for the column.
            if want.starts_with('#') {
                continue;
            }
            let got = format_value_in(
                &Calc::Number(value.as_f64().unwrap()),
                &code,
                false,
                &locale,
            );
            assert_eq!(got, want, "numFmtId {id} ({code:?}) probe {key}");
            compared += 1;
        }
    }
    assert!(compared > 300, "{compared} comparisons");
}

#[test]
fn the_international_flags_match_the_derivation() {
    let m = measured();
    let lcid = lcid_of(&m);
    // dayLeadingZero selects the two-digit day of ids 15 and 16, timeLeadingZero the two-digit hour of 20 to 22.
    let day = if m["dayLeadingZero"] == 1 { "dd" } else { "d" };
    let hour = if m["timeLeadingZero"] == 1 { "hh" } else { "h" };
    assert_eq!(
        builtin_number_format_in(15, lcid).unwrap(),
        format!("{day}-mmm-yy")
    );
    assert_eq!(
        builtin_number_format_in(20, lcid).unwrap(),
        format!("{hour}:mm")
    );
    // currencyNegative 1 is the pattern "-$n": a minus sign, so ids 37 to 40 have no parentheses.
    assert_eq!(m["currencyNegative"], 1);
    assert!(!builtin_number_format_in(37, lcid).unwrap().contains('('));
}
