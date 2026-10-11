//! Diagnostic report (ignored by default): prints Excel and Open365 results for corpus cases whose id matches the
//! regex in `OPEN365_REPORT`, flagging mismatches. Run with
//! `OPEN365_REPORT='^chisq_inv_rt\.' cargo test -p open365-calc --test known_report -- --ignored --nocapture`.
//! `OPEN365_REPORT_ALL=1` also lists matching cases. Use `OPEN365_CORPUS` / `OPEN365_RESULTS` to read other files.
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Arc;

use open365_calc::numfmt::{format_value_in, validate_format};
use open365_calc::{CellStyle, Engine, ErrorKind, FormatLocale, HAlign, Value};
use open365_locale::Locale;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Corpus {
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    id: String,
    formula: String,
    #[serde(default)]
    cells: BTreeMap<String, CellSpec>,
    #[serde(default)]
    sheet2: BTreeMap<String, CellSpec>,
    at: Option<String>,
    #[serde(default)]
    date1904: bool,
    /// Number format applied to the formula cell; the case then also checks `Range.Text`.
    format: Option<String>,
    /// `"en-GB"`: the case depends on the oracle's system locale and Open365 evaluates it in that locale.
    /// Without it the case must give the same answer in every locale.
    locale: Option<String>,
    /// Cell styles on Sheet1 for `CELL`: number format, alignment, locked flag and column width.
    #[serde(default)]
    styles: BTreeMap<String, StyleSpec>,
}

#[derive(Debug, Deserialize)]
struct StyleSpec {
    nf: Option<String>,
    ha: Option<String>,
    locked: Option<bool>,
    width: Option<f64>,
}

fn apply_styles(engine: &mut Engine, styles: &BTreeMap<String, StyleSpec>) {
    // The oracle sets every column to 60 characters.
    engine.set_standard_column_width("Sheet1", 60.0);
    engine.set_standard_column_width("Sheet2", 60.0);
    for (addr, spec) in styles {
        let (row, col) = parse_addr(addr);
        let halign = match spec.ha.as_deref() {
            Some("left") => HAlign::Left,
            Some("right") => HAlign::Right,
            Some("center") => HAlign::Center,
            Some("fill") => HAlign::Fill,
            Some("justify") => HAlign::Justify,
            Some("across") => HAlign::CenterAcrossSelection,
            Some("distributed") => HAlign::Distributed,
            _ => HAlign::General,
        };
        let style = CellStyle {
            number_format: spec.nf.clone(),
            halign,
            locked: spec.locked.unwrap_or(true),
        };
        engine.set_cell_style("Sheet1", row, col, style);
        if let Some(width) = spec.width {
            engine.set_column_width("Sheet1", col, width);
        }
    }
}

/// The locale a case is evaluated in (en-US unless the case names one); built once per tag.
fn case_locale(case: &Case) -> Arc<FormatLocale> {
    thread_local! {
        static CACHE: RefCell<HashMap<String, Arc<FormatLocale>>> = RefCell::new(HashMap::new());
    }
    let tag = case.locale.as_deref().unwrap_or("en-US").to_owned();
    CACHE.with(|c| {
        c.borrow_mut()
            .entry(tag.clone())
            .or_insert_with(|| {
                let locale = Locale::from_bcp47(&tag).expect("locale tag");
                Arc::new(FormatLocale::new(&locale))
            })
            .clone()
    })
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CellSpec {
    Bool(bool),
    Number(f64),
    Text(String),
    Error { err: String },
    Formula { f: String },
}

#[derive(Debug, Deserialize)]
struct Reference {
    results: Vec<Expected>,
}

#[derive(Debug, Deserialize)]
struct Expected {
    id: String,
    kind: String,
    value: Option<serde_json::Value>,
    /// `Range.Text` of the formula cell.
    text: Option<String>,
    /// For rejected cases: `NumberFormat` when Excel refused the format code rather than the formula.
    hresult: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KnownFile {
    #[serde(default)]
    difference: Vec<Known>,
}

#[derive(Debug, Deserialize)]
struct Known {
    id: String,
    reason: String,
}

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel)
}

/// `"C12"` to 1-based `(row, col)`.
fn parse_addr(addr: &str) -> (u32, u32) {
    let letters = addr.bytes().take_while(u8::is_ascii_alphabetic).count();
    let col = addr[..letters].bytes().fold(0u32, |acc, b| {
        acc * 26 + u32::from(b.to_ascii_uppercase() - b'A' + 1)
    });
    let row: u32 = addr[letters..].parse().expect("row number");
    (row, col)
}

fn load_cells(engine: &mut Engine, sheet: &str, cells: &BTreeMap<String, CellSpec>) {
    for (addr, spec) in cells {
        let (row, col) = parse_addr(addr);
        match spec {
            CellSpec::Number(n) => engine.set_value(sheet, row, col, Value::Number(*n)),
            CellSpec::Text(t) => engine.set_value(sheet, row, col, Value::Text(t.clone())),
            CellSpec::Bool(b) => engine.set_value(sheet, row, col, Value::Bool(*b)),
            CellSpec::Error { err } => {
                let kind = ErrorKind::parse(err).expect("known error literal");
                engine.set_value(sheet, row, col, Value::Error(kind));
            }
            CellSpec::Formula { f } => engine
                .set_formula(sheet, row, col, f)
                .expect("input formula parses"),
        }
    }
}

/// What Open365 produced for a case.
#[derive(Debug)]
enum Outcome {
    Rejected,
    Value(Value),
}

fn run_case(case: &Case) -> Outcome {
    let mut engine = Engine::new();
    engine.set_date1904(case.date1904);
    engine.set_format_locale(case_locale(case));
    engine.add_sheet("Sheet1");
    engine.add_sheet("Sheet2");
    apply_styles(&mut engine, &case.styles);
    load_cells(&mut engine, "Sheet1", &case.cells);
    load_cells(&mut engine, "Sheet2", &case.sheet2);
    let (row, col) = parse_addr(case.at.as_deref().unwrap_or("J20"));
    if engine
        .set_formula("Sheet1", row, col, &case.formula)
        .is_err()
    {
        return Outcome::Rejected;
    }
    engine.recalc_all();
    Outcome::Value(engine.get("Sheet1", row, col))
}

fn show_value(v: &Value) -> String {
    match v {
        Value::Number(n) => format!("number {n:?}"),
        Value::Text(t) => format!("text {t:?}"),
        Value::Bool(b) => format!("bool {b}"),
        Value::Error(e) => format!("error {e}"),
        Value::Empty => "blank".to_owned(),
        Value::Array(_) => "array".to_owned(),
    }
}

fn show_expected(e: &Expected) -> String {
    match (&e.value, e.kind.as_str()) {
        (Some(serde_json::Value::Number(n)), "number") => format!("number {n}"),
        (Some(serde_json::Value::String(s)), "text") => format!("text {s:?}"),
        (Some(v), kind) => format!("{kind} {v}"),
        (None, kind) => kind.to_owned(),
    }
}

/// `None` when Open365 agrees with Excel, otherwise a description of the mismatch.
fn compare(expected: &Expected, outcome: &Outcome) -> Option<String> {
    let describe = |actual: String| {
        Some(format!(
            "Excel: {}  Open365: {actual}",
            show_expected(expected)
        ))
    };
    match (expected.kind.as_str(), outcome) {
        // Excel refuses to accept the formula. Open365 passes if its parser refuses too or the formula
        // evaluates to an error value (a loader must keep such a cell rather than fail the whole file).
        ("rejected", Outcome::Rejected) => None,
        ("rejected", Outcome::Value(Value::Error(_))) => None,
        (_, Outcome::Rejected) => describe("parse error".to_owned()),
        (kind, Outcome::Value(actual)) => {
            let ok = match (kind, actual, expected.value.as_ref()) {
                ("number", Value::Number(a), Some(serde_json::Value::Number(e))) => {
                    e.as_f64().is_some_and(|e| e == *a)
                }
                ("text", Value::Text(a), Some(serde_json::Value::String(e))) => a == e,
                ("bool", Value::Bool(a), Some(serde_json::Value::Bool(e))) => a == e,
                ("error", Value::Error(a), Some(serde_json::Value::String(e))) => {
                    // The oracle names only the classic errors; others are `#UNKNOWN(code)` with the name in `Range.Text`.
                    let name = match (e.starts_with("#UNKNOWN("), expected.text.as_deref()) {
                        (true, Some(t)) => t,
                        _ => e.as_str(),
                    };
                    a.as_str() == name
                }
                _ => false,
            };
            if ok {
                None
            } else {
                describe(show_value(actual))
            }
        }
    }
}

/// For a case with a number format: the displayed text must equal `Range.Text`, and Excel refusing the format
/// code must match Open365 refusing it.
fn compare_display(case: &Case, expected: &Expected, outcome: &Outcome) -> Option<String> {
    let format = case.format.as_deref()?;
    if expected.kind == "rejected" {
        if expected.hresult.as_deref() != Some("NumberFormat") {
            return None;
        }
        return validate_format(format)
            .is_ok()
            .then(|| format!("Excel refuses the format {format:?}; Open365 accepts it"));
    }
    let Outcome::Value(value) = outcome else {
        return None;
    };
    let shown = format_value_in(value, format, case.date1904, &case_locale(case));
    let want = expected.text.as_deref().unwrap_or_default();
    (shown != want)
        .then(|| format!("Range.Text: Excel {want:?}  Open365 {shown:?}  (format {format:?})"))
}

fn load_path(var: &str, default: &str) -> String {
    match std::env::var(var) {
        Ok(p) => fs::read_to_string(p).expect("file"),
        Err(_) => fs::read_to_string(fixture(default)).expect("file"),
    }
}

#[test]
#[ignore]
fn report() {
    let pattern = std::env::var("OPEN365_REPORT").unwrap_or_else(|_| "^$".to_owned());
    let all = std::env::var("OPEN365_REPORT_ALL").is_ok();
    let re = regex_lite_match(&pattern);
    let corpus: Corpus =
        serde_json::from_str(&load_path("OPEN365_CORPUS", "calc/conformance.json")).unwrap();
    let reference: Reference = serde_json::from_str(&load_path(
        "OPEN365_RESULTS",
        "reference/calc/conformance-results.json",
    ))
    .unwrap();
    let answers: BTreeMap<&str, &Expected> = reference
        .results
        .iter()
        .map(|e| (e.id.as_str(), e))
        .collect();
    let (mut n, mut bad) = (0, 0);
    for case in &corpus.cases {
        if !re(&case.id) {
            continue;
        }
        let Some(expected) = answers.get(case.id.as_str()) else {
            println!("{}  NO EXCEL RESULT", case.id);
            continue;
        };
        n += 1;
        let outcome = catch_unwind(AssertUnwindSafe(|| run_case(case)));
        let mismatch = match &outcome {
            Ok(o) => compare(expected, o),
            Err(_) => Some("panic".to_owned()),
        };
        if mismatch.is_some() {
            bad += 1;
        }
        if mismatch.is_some() || all {
            let mut cells = String::new();
            for (k, v) in &case.cells {
                cells.push_str(&format!(" {k}={v:?}"));
            }
            println!(
                "{} {} {}
    {}{}",
                if mismatch.is_some() { "DIFF" } else { "ok  " },
                case.id,
                case.formula,
                mismatch.unwrap_or_else(|| show_expected(expected)),
                cells
            );
        }
    }
    println!("report: {n} cases, {bad} differ");
}

/// Minimal matcher: the pattern is a `|`-separated list of prefixes, each optionally starting with `^`
/// (anchoring is always at the start); a pattern `.` matches all.
fn regex_lite_match(pattern: &str) -> Box<dyn Fn(&str) -> bool> {
    let alts: Vec<String> = pattern
        .trim_start_matches("^(")
        .trim_end_matches(')')
        .split('|')
        .map(|s| s.trim_start_matches('^').to_owned())
        .collect();
    Box::new(move |id| alts.iter().any(|a| a == "." || id.starts_with(a.as_str())))
}
