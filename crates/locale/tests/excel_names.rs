//! Compares the crate's month, day and AM/PM names against Excel (`TEXT()` with `[$-LCID]` prefixes).
//!
//! The reference is `fixtures/reference/locale/excel-names.json`, written by `tools/office/locale-oracle.ps1`.
//! Every difference between CLDR and Excel is an entry in `office_overrides::NAME_OVERRIDES`, so after
//! overrides the crate must equal Excel for every LCID Excel can use.

use open365_locale::office_overrides::NameField;
use open365_locale::{CalendarNames, Locale};
use serde_json::Value;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// LCIDs Excel cannot use as `[$-LCID]` (it answers with en-US names); measured, see office_overrides.
const EXCEL_FALLS_BACK_TO_EN_US: [u32; 3] = [0x0001, 0x0002, 0x0471];

fn fixture() -> Result<Value, serde_json::Error> {
    serde_json::from_str(include_str!(
        "../../../fixtures/reference/locale/excel-names.json"
    ))
}

fn list(entry: &Value, key: &str) -> Vec<String> {
    entry[key]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn excel_field(entry: &Value, field: NameField) -> Vec<String> {
    match field {
        NameField::MonthFull => list(entry, "month_full"),
        NameField::MonthAbbr => list(entry, "month_abbr"),
        NameField::DayFull => list(entry, "day_full"),
        NameField::DayAbbr => list(entry, "day_abbr"),
        NameField::AmPm => ["am", "pm"]
            .iter()
            .filter_map(|k| entry[*k].as_str().map(str::to_owned))
            .collect(),
    }
}

fn our_field(names: &CalendarNames, field: NameField) -> Vec<String> {
    match field {
        NameField::MonthFull => names.month_full.to_vec(),
        NameField::MonthAbbr => names.month_abbr.to_vec(),
        NameField::DayFull => names.day_full.to_vec(),
        NameField::DayAbbr => names.day_abbr.to_vec(),
        NameField::AmPm => vec![names.am.clone(), names.pm.clone()],
    }
}

const FIELDS: [NameField; 5] = [
    NameField::MonthFull,
    NameField::MonthAbbr,
    NameField::DayFull,
    NameField::DayAbbr,
    NameField::AmPm,
];

fn lcid_of(entry: &Value) -> Result<u32, &'static str> {
    entry["lcid"].as_u64().map(|v| v as u32).ok_or("lcid")
}

#[test]
fn names_match_excel() -> TestResult {
    let fixture = fixture()?;
    let locales = fixture["locales"].as_array().ok_or("locales")?;
    let (mut raw_matched, mut matched, mut total) = (0usize, 0usize, 0usize);
    let mut mismatches = Vec::new();
    for entry in locales {
        let lcid = lcid_of(entry)?;
        if EXCEL_FALLS_BACK_TO_EN_US.contains(&lcid) {
            continue;
        }
        let locale = Locale::from_lcid(lcid)?;
        let cldr = locale.calendar_names_cldr()?;
        let office = locale.calendar_names()?;
        for field in FIELDS {
            let excel = excel_field(entry, field);
            total += 1;
            raw_matched += usize::from(excel == our_field(&cldr, field));
            if excel == our_field(&office, field) {
                matched += 1;
            } else {
                mismatches.push(format!(
                    "{lcid:#06x} {} {field:?}: excel={excel:?} ours={:?}",
                    locale.bcp47(),
                    our_field(&office, field)
                ));
            }
        }
    }
    println!(
        "name lists matching Excel: CLDR alone {raw_matched}/{total}, with overrides {matched}/{total}"
    );
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    Ok(())
}

/// Measured: `d mmmm` gives the same month text as `mmmm` in every locale, so Excel never uses genitive forms.
#[test]
fn excel_never_inflects_month_names() -> TestResult {
    let fixture = fixture()?;
    for entry in fixture["locales"].as_array().ok_or("locales")? {
        for (plain, with_day) in [
            ("month_full", "month_full_with_day"),
            ("month_abbr", "month_abbr_with_day"),
        ] {
            let stripped: Vec<String> = list(entry, with_day)
                .iter()
                .map(|t| t.strip_prefix('1').unwrap_or(t).trim().to_owned())
                .collect();
            assert_eq!(stripped, list(entry, plain), "{}", entry["tag"]);
        }
    }
    Ok(())
}

/// Every LCID in the crate table was measured.
#[test]
fn fixture_covers_every_lcid() -> TestResult {
    let fixture = fixture()?;
    let measured: std::collections::BTreeSet<u32> = fixture["locales"]
        .as_array()
        .ok_or("locales")?
        .iter()
        .map(lcid_of)
        .collect::<Result<_, _>>()?;
    let table: std::collections::BTreeSet<u32> = Locale::all_lcids().map(|(l, _)| l).collect();
    assert_eq!(measured, table);
    Ok(())
}

/// Writes the generated `NAME_OVERRIDES` block to `$OPEN365_LOCALE_OVERRIDES_OUT`; paste it between
/// the BEGIN/END GENERATED markers of `src/office_overrides.rs`.
#[test]
#[ignore = "regenerates source; run on demand"]
fn regenerate() -> TestResult {
    let out = std::env::var("OPEN365_LOCALE_OVERRIDES_OUT")?;
    let fixture = fixture()?;
    let mut text = String::from("pub const NAME_OVERRIDES: &[NameOverride] = &[\n");
    for entry in fixture["locales"].as_array().ok_or("locales")? {
        let lcid = lcid_of(entry)?;
        if EXCEL_FALLS_BACK_TO_EN_US.contains(&lcid) {
            continue;
        }
        let cldr = Locale::from_lcid(lcid)?.calendar_names_cldr()?;
        for field in FIELDS {
            let excel = excel_field(entry, field);
            if excel != our_field(&cldr, field) {
                let values: Vec<String> = excel.iter().map(|v| format!("{v:?}")).collect();
                text.push_str(&format!(
                    "    NameOverride {{ lcid: {lcid:#06x}, field: NameField::{field:?}, values: &[{}] }},\n",
                    values.join(", ")
                ));
            }
        }
    }
    text.push_str("];\n");
    std::fs::write(out, text)?;
    Ok(())
}
