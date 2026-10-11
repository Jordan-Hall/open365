//! Reader check against Excel's own answers: `fixtures/reference/xlsx/<name>/values.json`
//! holds `Value2` and the A1 formula of each cell as Excel reported them.

mod common;

use open365_opc::Limits;
use open365_xlsx::{CellValue, FormulaKind, Workbook, cell_ref_string, parse_cell_ref};
use serde_json::Value;

use common::{fixture, fixtures, repo_root};

fn values_json(name: &str) -> Value {
    let path = repo_root()
        .join("fixtures")
        .join("reference")
        .join("xlsx")
        .join(name)
        .join("values.json");
    serde_json::from_slice(&std::fs::read(&path).expect("values.json exists"))
        .expect("values.json parses")
}

fn open(name: &str) -> Workbook {
    Workbook::open(&fixture(name), &Limits::default()).expect("opens")
}

/// Compares one of our values with Excel's `value2`. Returns a description on mismatch.
fn compare(ours: Option<&CellValue>, excel: &Value) -> Result<(), String> {
    let empty = CellValue::Empty;
    let ours = ours.unwrap_or(&empty);
    let ok = match (ours, excel) {
        (CellValue::Empty, Value::Null) => true,
        (CellValue::Empty, Value::String(s)) => s.is_empty(),
        (CellValue::Number(n), Value::Number(x)) => {
            let x = x.as_f64().ok_or("number")?;
            (n - x).abs() <= 1e-12 * x.abs().max(1.0)
        }
        (CellValue::Bool(b), Value::Bool(x)) => b == x,
        (CellValue::Error(e), Value::Number(x)) => e.excel_code().map(i64::from) == x.as_i64(),
        (v, Value::String(s)) => v.as_text() == Some(s.as_str()),
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(format!("ours {ours:?}, Excel {excel}"))
    }
}

#[derive(Default)]
struct Tally {
    compared: usize,
    numbers: usize,
    strings: usize,
    bools: usize,
    errors: usize,
}

#[test]
fn cached_values_match_excel_for_every_fixture() {
    let mut total = Tally::default();
    for (name, _) in fixtures() {
        let wb = open(&name);
        let json = values_json(&name);
        let sheets = json["sheets"].as_array().expect("sheets");
        assert_eq!(sheets.len(), wb.sheets().len(), "{name}: sheet count");
        for (i, sheet) in sheets.iter().enumerate() {
            assert_eq!(
                sheet["name"],
                wb.sheets()[i].name.as_str(),
                "{name}: sheet name"
            );
            let cells = sheet["cells"].as_object().expect("cells");
            let truncated = sheet["truncatedAt2000"].as_bool().unwrap_or(false);
            for (reference, cell) in cells {
                let (row, col) = parse_cell_ref(reference).expect("A1 key");
                let excel = &cell["value2"];
                let ours = wb.cell(i, row, col).map(|c| &c.value);
                if let Err(why) = compare(ours, excel) {
                    panic!("{name}!{reference}: {why}");
                }
                total.compared += 1;
                match excel {
                    Value::Number(_) if matches!(ours, Some(CellValue::Error(_))) => {
                        total.errors += 1;
                    }
                    Value::Number(_) => total.numbers += 1,
                    Value::String(s) if !s.is_empty() => total.strings += 1,
                    Value::Bool(_) => total.bools += 1,
                    _ => {}
                }
            }
            if !truncated {
                // Everything we read as non-empty must be something Excel reported.
                for (row, col, cell) in wb.iter_cells(i) {
                    // Excel's list leaves out empty strings (a formula returning "").
                    let blank = match &cell.value {
                        CellValue::Empty => true,
                        CellValue::Str(t) => t.is_empty(),
                        _ => false,
                    };
                    if !blank {
                        let key = cell_ref_string(row, col);
                        assert!(
                            cells.contains_key(&key),
                            "{name}!{key}: not in Excel's list"
                        );
                    }
                }
            }
        }
    }
    eprintln!(
        "cached values compared with Excel: {} cells ({} numbers, {} strings, {} bools, {} errors)",
        total.compared, total.numbers, total.strings, total.bools, total.errors
    );
    assert!(total.compared > 2_000);
    assert!(total.errors >= 1 && total.bools >= 1 && total.strings >= 10);
}

#[test]
fn shared_formulas_expand_to_excels_own_text() {
    let wb = open("shared-formulas");
    let json = values_json("shared-formulas");
    let cells = json["sheets"][0]["cells"].as_object().expect("cells");
    let (mut dependents, mut masters) = (0, 0);
    for (reference, cell) in cells {
        let Some(excel) = cell["formula"].as_str() else {
            continue;
        };
        let (row, col) = parse_cell_ref(reference).expect("A1 key");
        let formula = wb.cell(0, row, col).and_then(|c| c.formula.as_deref());
        let text = wb
            .formula_text(0, row, col)
            .unwrap_or_else(|| panic!("{reference}: no formula"));
        assert_eq!(format!("={text}"), excel, "{reference}");
        match formula.map(|f| (&f.kind, f.text.is_empty())) {
            Some((FormulaKind::Shared { .. }, true)) => dependents += 1,
            Some((FormulaKind::Shared { .. }, false)) => masters += 1,
            other => panic!("{reference}: expected a shared formula, got {other:?}"),
        }
    }
    eprintln!("shared formulas: {masters} masters, {dependents} dependents expanded");
    assert!(dependents >= 90 && masters >= 2);
}

#[test]
fn every_formula_reads_as_excel_reports_it() {
    // Excel reports future functions without the `_xlfn.` storage prefix.
    let (mut checked, mut external) = (0, 0);
    for (name, _) in fixtures() {
        let wb = open(&name);
        let json = values_json(&name);
        for (i, sheet) in json["sheets"]
            .as_array()
            .expect("sheets")
            .iter()
            .enumerate()
        {
            for (reference, cell) in sheet["cells"].as_object().expect("cells") {
                let Some(excel) = cell["formula"].as_str() else {
                    continue;
                };
                let (row, col) = parse_cell_ref(reference).expect("A1 key");
                let text = wb
                    .formula_text(i, row, col)
                    .unwrap_or_else(|| panic!("{name}!{reference}: no formula"))
                    .replace("_xlfn.", "");
                if text.starts_with("[1]") {
                    // External workbooks are stored as an index into externalLink parts;
                    // Excel reports the resolved path. Resolving is out of scope here.
                    external += 1;
                    continue;
                }
                assert_eq!(format!("={text}"), excel, "{name}!{reference}");
                checked += 1;
            }
        }
    }
    eprintln!("formula texts compared with Excel: {checked} (+{external} external-link)");
    assert_eq!(external, 1);
    assert!(checked > 100);
}
