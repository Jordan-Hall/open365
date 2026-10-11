//! Value and display-text fidelity over the local Microsoft xlsx template corpus.
//!
//! References come from Excel (`fixtures/reference/ms-templates/<id>/values.json`: Value2, Text and
//! Formula per cell); both they and the templates are local and gitignored, and the test skips
//! cleanly when either is absent. Every formula is recalculated with `open365-calc` through the
//! commands crate's engine, then Value2 and the displayed text (our number formatting) are compared
//! cell by cell. Per-template matched/total counts are kept in
//! `fixtures/reference/ms-templates-values-floor.toml` (counts only) and may not fall.
//!
//! Environment:
//! - `OPEN365_UPDATE_FLOORS=1` rewrites the floor file from this run.
//! - `OPEN365_MISMATCH_OUT=<file>` writes one tab-separated line per mismatch (local analysis).
//!
//! Volatile formulas (TODAY, NOW, RAND, RANDBETWEEN) are left out of the totals and counted
//! separately; so is text that depends on the column width (`###` overflow, `*` fill) from the text
//! totals (counted as `narrow`).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use open365_calc::{ErrorKind, Value};
use open365_commands::FormulaEngine;
use std::sync::Arc;

use open365_calc::FormatLocale;
use open365_calc::numfmt::format_value_in;
use open365_opc::Limits;
use open365_xlsx::{CellError, CellValue, SheetKind, Workbook};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

const FIELDS: [&str; 10] = [
    "cells",
    "formulas",
    "values_matched",
    "values_total",
    "text_matched",
    "text_total",
    "formula_values_matched",
    "formula_text_matched",
    "volatile",
    "narrow",
];

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Counts {
    n: [u64; 10],
}

impl Counts {
    fn idx(name: &str) -> usize {
        FIELDS.iter().position(|f| *f == name).expect("field")
    }
    fn get(&self, name: &str) -> u64 {
        self.n[Self::idx(name)]
    }
    fn add(&mut self, name: &str) {
        self.n[Self::idx(name)] += 1;
    }
}

fn parse_floors(text: &str) -> BTreeMap<String, Counts> {
    let mut out: BTreeMap<String, Counts> = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(id) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = Some(id.to_owned());
            out.entry(id.to_owned()).or_default();
        } else if let (Some(id), Some((k, v))) = (&current, line.split_once('='))
            && let (Some(c), Ok(n)) = (out.get_mut(id), v.trim().parse::<u64>())
            && let Some(i) = FIELDS.iter().position(|f| *f == k.trim())
        {
            c.n[i] = n;
        }
    }
    out
}

fn write_floors(map: &BTreeMap<String, Counts>) -> String {
    let mut s = String::from(
        "# Per-template counts of Open365 vs Excel (values and text). Counts only; no template content.\n\
         # Regenerate: OPEN365_UPDATE_FLOORS=1 cargo test -p open365-testkit --test ms_template_values\n",
    );
    for (id, c) in map {
        let _ = writeln!(s, "\n[{id}]");
        for f in FIELDS {
            let _ = writeln!(s, "{f} = {}", c.get(f));
        }
    }
    s
}

fn parse_addr(a: &str) -> Option<(u32, u32)> {
    let letters: String = a.chars().take_while(char::is_ascii_alphabetic).collect();
    let row: u32 = a[letters.len()..].parse().ok()?;
    let col = letters.bytes().fold(0u32, |n, b| {
        n * 26 + u32::from(b.to_ascii_uppercase() - b'A' + 1)
    });
    Some((row, col))
}

fn error_code(n: i64) -> Option<&'static str> {
    Some(match n {
        -2_146_826_288 => "#NULL!",
        -2_146_826_281 => "#DIV/0!",
        -2_146_826_273 => "#VALUE!",
        -2_146_826_265 => "#REF!",
        -2_146_826_259 => "#NAME?",
        -2_146_826_252 => "#NUM!",
        -2_146_826_246 => "#N/A",
        _ => return None,
    })
}

#[derive(Debug, PartialEq)]
enum Ref {
    Num(f64),
    Text(String),
    Bool(bool),
    Err(&'static str),
    Null,
}

fn ref_value(v: &serde_json::Value) -> Ref {
    match v {
        serde_json::Value::Bool(b) => Ref::Bool(*b),
        serde_json::Value::String(s) => Ref::Text(s.clone()),
        serde_json::Value::Number(n) => {
            let f = n.as_f64().unwrap_or(0.0);
            #[allow(clippy::cast_possible_truncation)]
            match error_code(f as i64).filter(|_| f < -2.0e9) {
                Some(e) => Ref::Err(e),
                None => Ref::Num(f),
            }
        }
        _ => Ref::Null,
    }
}

fn matches_value(r: &Ref, o: &Value) -> bool {
    match (r, o) {
        (Ref::Num(a), Value::Number(b)) => (a - b).abs() <= 1e-12 * a.abs().max(1.0),
        (Ref::Num(a), Value::Empty) => *a == 0.0,
        (Ref::Text(a), Value::Text(b)) => a == b,
        (Ref::Text(a), Value::Empty) => a.is_empty(),
        (Ref::Bool(a), Value::Bool(b)) => a == b,
        (Ref::Err(a), Value::Error(b)) => *a == b.as_str(),
        (Ref::Null, Value::Empty) => true,
        _ => false,
    }
}

fn to_calc_value(v: &CellValue) -> Value {
    match v {
        CellValue::Empty => Value::Empty,
        CellValue::Number(n) => Value::Number(*n),
        CellValue::Bool(b) => Value::Bool(*b),
        CellValue::Error(e) => Value::Error(match e {
            CellError::Null => ErrorKind::Null,
            CellError::Div0 => ErrorKind::DivZero,
            CellError::Value => ErrorKind::Value,
            CellError::Ref => ErrorKind::Ref,
            CellError::Name => ErrorKind::Name,
            CellError::Num => ErrorKind::Num,
            CellError::NA => ErrorKind::NA,
            CellError::GettingData => ErrorKind::GettingData,
            CellError::Other(code) => ErrorKind::parse(code).unwrap_or(ErrorKind::NA),
        }),
        other => Value::Text(other.as_text().unwrap_or_default().to_owned()),
    }
}

/// The locale the references were produced in (Windows English (United Kingdom), LCID 0x809): the
/// built-in short date and time formats follow it.
const ORACLE_LCID: u32 = 0x0809;

fn format_code(wb: &Workbook, xf: u32, locale: &FormatLocale) -> String {
    let styles = wb.styles();
    let id = styles.num_fmt_id(xf);
    if styles.custom_formats().contains_key(&id) {
        return styles.format_code(id).to_owned();
    }
    // Built-in ids 37 to 40 follow the Windows "negative numbers" setting (the oracle machine
    // shows a minus sign, not parentheses); 20 and 21 follow the locale's 24-hour time.
    match id {
        37 => return "#,##0;-#,##0".to_owned(),
        38 => return "#,##0;[Red]-#,##0".to_owned(),
        39 => return "#,##0.00;-#,##0.00".to_owned(),
        40 => return "#,##0.00;[Red]-#,##0.00".to_owned(),
        20 => return "hh:mm".to_owned(),
        21 => return "hh:mm:ss".to_owned(),
        _ => {}
    }
    locale
        .builtin_format_code(id)
        .unwrap_or_else(|| styles.format_code(id).to_owned())
}

/// The displayed text of a value through a number format (as the grid does).
fn display(v: &Value, code: &str, date1904: bool, locale: &FormatLocale) -> String {
    match v {
        Value::Empty | Value::Array(_) => String::new(),
        Value::Number(_) => format_value_in(v, code, date1904, locale),
        Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_owned(),
        Value::Error(e) => e.as_str().to_owned(),
        Value::Text(t) => {
            if code == "General" || code.is_empty() {
                t.clone()
            } else {
                format_value_in(v, code, date1904, locale)
            }
        }
    }
}

fn is_volatile(formula: &str) -> bool {
    let f = formula.to_ascii_uppercase();
    ["TODAY(", "NOW(", "RAND(", "RANDBETWEEN("]
        .iter()
        .any(|n| f.contains(n))
}

fn show(v: &Ref) -> String {
    match v {
        Ref::Num(n) => format!("{n}"),
        Ref::Text(t) => format!("{t:?}"),
        Ref::Bool(b) => b.to_string(),
        Ref::Err(e) => (*e).to_owned(),
        Ref::Null => String::new(),
    }
}

fn run_one(id: &str, xlsx: &Path, refs: &Path, report: &mut String) -> Result<Counts, String> {
    let bytes = fs::read(xlsx).map_err(|e| e.to_string())?;
    let wb = Workbook::open(&bytes, &Limits::default()).map_err(|e| format!("open: {e}"))?;
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(refs).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let mut engine = FormulaEngine::build(&wb, &|_, _, _| true);
    let locale: Arc<FormatLocale> =
        FormatLocale::for_lcid(ORACLE_LCID).unwrap_or_else(FormatLocale::en_us);
    // The references were produced under this locale: date text such as `Mar 15` reads under it too.
    engine.set_format_locale(Arc::clone(&locale));
    let mut c = Counts::default();
    let sheets = json["sheets"].as_array().cloned().unwrap_or_default();
    for sheet in &sheets {
        let name = sheet["name"].as_str().unwrap_or_default();
        let Some(si) = wb.sheet_index(name) else {
            continue;
        };
        if wb.sheets()[si].kind != SheetKind::Worksheet {
            continue;
        }
        let Some(cells) = sheet["cells"].as_object() else {
            continue;
        };
        for (addr, cell) in cells {
            let Some((row, col)) = parse_addr(addr) else {
                continue;
            };
            let rv = ref_value(&cell["value2"]);
            let rt = cell["text"].as_str().unwrap_or_default();
            let formula = wb.formula_text(si, row, col);
            c.add("cells");
            if let Some(f) = &formula {
                c.add("formulas");
                if is_volatile(f) {
                    c.add("volatile");
                    continue;
                }
            }
            let ours = if formula.is_some() {
                engine.value(name, row, col)
            } else {
                wb.cell(si, row, col)
                    .map_or(Value::Empty, |x| to_calc_value(&x.value))
            };
            let xf = wb.cell(si, row, col).and_then(|x| x.style).unwrap_or(0);
            let code = format_code(&wb, xf, &locale);
            let text = display(&ours, &code, wb.date1904(), &locale);
            let kind = if formula.is_some() { "F" } else { "C" };
            // Text that depends on the column width: `###` overflow and `*` fill padding.
            let narrow = (rt.len() > 1 && rt.chars().all(|ch| ch == '#')) || code.contains("* ");
            if narrow {
                c.add("narrow");
            }
            c.add("values_total");
            if matches_value(&rv, &ours) {
                c.add("values_matched");
                if formula.is_some() {
                    c.add("formula_values_matched");
                }
            } else {
                let _ = writeln!(
                    report,
                    "{id}\t{name}\t{addr}\tvalue\t{kind}\t{}\t{}\t{ours:?}\t{code}\t{rt}\t{text}",
                    formula.clone().unwrap_or_default(),
                    show(&rv),
                );
            }
            if !narrow {
                c.add("text_total");
                if text == rt {
                    c.add("text_matched");
                    if formula.is_some() {
                        c.add("formula_text_matched");
                    }
                } else {
                    let _ = writeln!(
                        report,
                        "{id}\t{name}\t{addr}\ttext\t{kind}\t{}\t{}\t{ours:?}\t{code}\t{rt}\t{text}",
                        formula.clone().unwrap_or_default(),
                        show(&rv),
                    );
                }
            }
        }
    }
    Ok(c)
}

#[test]
fn ms_template_values_meet_floor() {
    let root = repo_root();
    let corpus = root.join("fixtures/external/ms-templates");
    let refs = root.join("fixtures/reference/ms-templates");
    let Ok(dir) = fs::read_dir(&corpus) else {
        eprintln!("skip: corpus absent");
        return;
    };
    let mut files: Vec<PathBuf> = dir
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "xlsx"))
        .collect();
    files.sort();
    let floor_path = root.join("fixtures/reference/ms-templates-values-floor.toml");
    let floors = fs::read_to_string(&floor_path)
        .map(|t| parse_floors(&t))
        .unwrap_or_default();
    let mut results = BTreeMap::new();
    let mut report = String::new();
    for f in &files {
        let id = f
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let r = refs.join(&id).join("values.json");
        if !r.is_file() {
            continue;
        }
        let started = std::time::Instant::now();
        let outcome = run_one(&id, f, &r, &mut report);
        eprintln!("{id}: {:?}", started.elapsed());
        match outcome {
            Ok(c) => {
                results.insert(id, c);
            }
            Err(e) => panic!("{id}: {e}"),
        }
    }
    if results.is_empty() {
        eprintln!("skip: no references");
        return;
    }
    let sum = |n: &str| results.values().map(|c| c.get(n)).sum::<u64>();
    eprintln!(
        "templates {}: values {}/{} text {}/{} (formulas {}: values {}, text {}; volatile {}, narrow {})",
        results.len(),
        sum("values_matched"),
        sum("values_total"),
        sum("text_matched"),
        sum("text_total"),
        sum("formulas"),
        sum("formula_values_matched"),
        sum("formula_text_matched"),
        sum("volatile"),
        sum("narrow"),
    );
    if let Some(out) = std::env::var_os("OPEN365_MISMATCH_OUT") {
        fs::write(out, &report).expect("write mismatch report");
    }
    if std::env::var_os("OPEN365_UPDATE_FLOORS").is_some() {
        fs::write(&floor_path, write_floors(&results)).expect("write floors");
        return;
    }
    let mut failures = Vec::new();
    for (id, c) in &results {
        let Some(floor) = floors.get(id) else {
            continue;
        };
        if c.get("values_matched") < floor.get("values_matched")
            || c.get("text_matched") < floor.get("text_matched")
        {
            failures.push(format!(
                "{id}: values {}/{} (floor {}), text {}/{} (floor {})",
                c.get("values_matched"),
                c.get("values_total"),
                floor.get("values_matched"),
                c.get("text_matched"),
                c.get("text_total"),
                floor.get("text_matched")
            ));
        }
    }
    assert!(failures.is_empty(), "below floor:\n{}", failures.join("\n"));
}
