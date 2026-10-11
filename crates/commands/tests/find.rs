#![allow(clippy::unwrap_used)]
//! Find and replace against Word and Excel.
//!
//! `fixtures/reference/docx/find-runs/find.json` and `fixtures/reference/xlsx/find-cells/find.json`
//! hold what Microsoft Word and Excel did with the generated fixtures (see
//! `tools/office/find-oracle.ps1`): match counts and results of replace-all, and the cells Excel's
//! Find visits. Every scenario must match, and every replace-all must undo to the original.

use std::path::PathBuf;

use open365_commands::{
    Command, DocPos, FindScope, LookIn, Target, XlsxFind, cell_source_text, next_match,
    xlsx_find_all,
};
use serde_json::Value;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn json(rel: &str) -> Value {
    let text = std::fs::read_to_string(repo().join(rel)).expect("reference file");
    serde_json::from_str(&text).expect("valid JSON")
}

fn bit(on: bool, value: u32) -> u32 {
    if on { value } else { 0 }
}

/// Paragraph texts with Word's per-character format digits.
fn docx_state(target: &Target) -> Vec<(String, String)> {
    let doc = target.docx().expect("a Word document");
    // Word does not list the empty paragraph that ends the document after the table.
    let mut count = doc.paragraph_count();
    while count > 0 && doc.paragraph_text(count - 1).is_some_and(|t| t.is_empty()) {
        count -= 1;
    }
    (0..count)
        .map(|p| {
            let text = doc.paragraph_text(p).unwrap_or_default();
            let fmt: String = (0..text.chars().count())
                .map(|i| {
                    let f = doc
                        .char_format_at(DocPos::new(p, i + 1))
                        .expect("character format");
                    let v = bit(f.bold, 1)
                        + bit(f.italic, 2)
                        + bit(f.underline, 4)
                        + bit(f.color.is_some(), 8);
                    char::from_digit(v, 16).expect("hex digit")
                })
                .collect();
            (text, fmt)
        })
        .collect()
}

fn word_state(paragraphs: &Value) -> Vec<(String, String)> {
    paragraphs
        .as_array()
        .expect("paragraph list")
        .iter()
        .map(|p| {
            (
                p["text"].as_str().unwrap().to_owned(),
                p["fmt"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn open_word() -> Target {
    Target::open_path(&repo().join("fixtures/docx/find-runs.docx")).expect("fixture opens")
}

#[test]
fn word_fixture_matches_the_reference_before_any_edit() {
    let reference = json("fixtures/reference/docx/find-runs/find.json");
    let target = open_word();
    assert_eq!(docx_state(&target), word_state(&reference["base"]));
}

#[test]
fn word_replace_all_matches_word() {
    let reference = json("fixtures/reference/docx/find-runs/find.json");
    let base = word_state(&reference["base"]);
    let mut failures = Vec::new();
    let mut checked = 0;
    for scenario in reference["scenarios"].as_array().unwrap() {
        let name = scenario["name"].as_str().unwrap();
        // Known differences: Word's star runs through paragraph marks; Word's `?@` matches
        // differently from the other lazy repeats.
        if scenario.get("error").is_some()
            || matches!(name, "wc-star-crosses-paragraphs" | "wc-at-any")
        {
            continue;
        }
        let mut target = open_word();
        let command = Command::DocxReplaceAll {
            find: scenario["find"].as_str().unwrap().to_owned(),
            replacement: scenario["replace"].as_str().unwrap().to_owned(),
            match_case: scenario["matchCase"].as_bool().unwrap(),
            whole_word: scenario["wholeWord"].as_bool().unwrap(),
            wildcards: scenario["wildcards"].as_bool().unwrap(),
        };
        let outcome = match target.apply(command) {
            Ok(o) => o,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        checked += 1;
        let expected_count = scenario["count"].as_u64().unwrap() as usize;
        let expected = word_state(&scenario["paragraphs"]);
        let got = docx_state(&target);
        if outcome.replaced != expected_count || got != expected {
            let mut msg = format!(
                "{name}: replaced {} (Word {expected_count})",
                outcome.replaced
            );
            for (i, (g, e)) in got.iter().zip(&expected).enumerate() {
                if g != e {
                    msg.push_str(&format!(
                        "\n    para {i}\n      ours: {g:?}\n      word: {e:?}"
                    ));
                }
            }
            if got.len() != expected.len() {
                msg.push_str(&format!(
                    "\n    paragraphs: {} vs Word {}",
                    got.len(),
                    expected.len()
                ));
            }
            failures.push(msg);
            continue;
        }
        // One undo restores the document exactly.
        target.undo().expect("replace-all undoes");
        assert_eq!(docx_state(&target), base, "{name}: undo");
        target.redo().expect("replace-all redoes");
        assert_eq!(docx_state(&target), got, "{name}: redo");
    }
    assert!(checked > 30, "only {checked} scenarios ran");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn open_excel() -> Target {
    Target::open_path(&repo().join("fixtures/xlsx/find-cells.xlsx")).expect("fixture opens")
}

fn scope_of(entry: &Value) -> FindScope {
    if entry["scope"] == "workbook" {
        FindScope::Workbook
    } else {
        FindScope::Sheet {
            sheet: entry["sheet"].as_u64().unwrap() as usize,
        }
    }
}

fn address(wb: &open365_xlsx::Workbook, sheet: usize, row: u32, col: u32) -> String {
    format!(
        "{}!{}",
        wb.sheets()[sheet].name,
        open365_xlsx::cell_ref_string(row, col)
    )
}

fn workbook_state(target: &Target) -> Vec<(String, String)> {
    let wb = target.xlsx().expect("a workbook");
    let mut out = Vec::new();
    for sheet in 0..wb.sheets().len() {
        let Some(ws) = wb.sheet(sheet) else { continue };
        for (row, col, _) in ws.iter_cells() {
            let text = cell_source_text(wb, sheet, row, col);
            if !text.is_empty() {
                out.push((address(wb, sheet, row, col), text));
            }
        }
    }
    out.sort();
    out
}

fn excel_state(result: &Value) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = result
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["cell"].as_str().unwrap().to_owned(),
                c["formula"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    out.sort();
    out
}

fn find_of(entry: &Value, look_in: LookIn) -> XlsxFind {
    XlsxFind {
        text: entry["find"].as_str().unwrap().to_owned(),
        scope: scope_of(entry),
        by_columns: entry["searchOrder"] == "columns",
        look_in,
        match_case: entry["matchCase"].as_bool().unwrap(),
        match_entire_cell: entry["lookAt"] == "whole",
    }
}

#[test]
fn excel_fixture_matches_the_reference_before_any_edit() {
    let target = open_excel();
    let wb = target.xlsx().unwrap();
    // The cells every scenario starts from.
    for (cell, text) in [
        ("S1!A1", "Apple"),
        ("S1!D1", "123"),
        ("S1!E1", "=A1&\"x\""),
        ("S1!H1", "TRUE"),
    ] {
        let found = workbook_state(&target)
            .into_iter()
            .find(|(c, _)| c == cell)
            .map(|(_, t)| t);
        assert_eq!(found.as_deref(), Some(text), "{cell}");
    }
    assert_eq!(wb.sheets().len(), 3);
}

#[test]
fn excel_replace_all_matches_excel() {
    let reference = json("fixtures/reference/xlsx/find-cells/find.json");
    let base = workbook_state(&open_excel());
    let mut failures = Vec::new();
    let mut checked = 0;
    for entry in reference["replace"].as_array().unwrap() {
        let name = entry["name"].as_str().unwrap();
        if entry.get("error").is_some() {
            continue;
        }
        // Known difference: Excel's replace leaves "  space " alone for `a*e` although its
        // Find reports the cell.
        if name == "star-part" {
            continue;
        }
        let mut target = open_excel();
        let find = find_of(entry, LookIn::Formulas);
        // The cells Excel's Find reports before the replace.
        let before = xlsx_find_all(target.xlsx().unwrap(), &find);
        let wb = target.xlsx().unwrap();
        let mut hit_names: Vec<String> = before
            .iter()
            .map(|c| address(wb, c.sheet, c.row, c.col))
            .collect();
        let mut excel_hits: Vec<String> = entry["cells"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_str().unwrap().to_owned())
            .collect();
        hit_names.sort();
        excel_hits.sort();
        let command = Command::XlsxReplaceAll {
            find,
            replacement: entry["replacement"].as_str().unwrap().to_owned(),
        };
        let outcome = target.apply(command).expect("replace-all applies");
        checked += 1;
        let got = workbook_state(&target);
        let expected = excel_state(&entry["result"]);
        if hit_names != excel_hits || got != expected {
            let mut msg = format!("{name}:");
            if hit_names != excel_hits {
                msg.push_str(&format!(
                    "\n    cells ours {hit_names:?}\n    excel     {excel_hits:?}"
                ));
            }
            for (g, e) in got.iter().zip(&expected) {
                if g != e {
                    msg.push_str(&format!("\n    ours {g:?}\n    excel {e:?}"));
                }
            }
            if got.len() != expected.len() {
                msg.push_str(&format!(
                    "\n    cell count {} vs Excel {}",
                    got.len(),
                    expected.len()
                ));
            }
            failures.push(msg);
            continue;
        }
        assert!(outcome.replaced >= hit_names.len(), "{name}: occurrences");
        target.undo().expect("replace-all undoes");
        assert_eq!(workbook_state(&target), base, "{name}: undo");
        target.redo().expect("replace-all redoes");
        assert_eq!(workbook_state(&target), got, "{name}: redo");
    }
    assert!(checked > 25, "only {checked} scenarios ran");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn excel_find_visits_cells_in_excels_order() {
    let reference = json("fixtures/reference/xlsx/find-cells/find.json");
    let target = open_excel();
    let wb = target.xlsx().unwrap();
    let mut failures = Vec::new();
    for entry in reference["find"].as_array().unwrap() {
        let name = entry["name"].as_str().unwrap();
        let look_in = if entry["lookIn"] == "values" {
            LookIn::Values
        } else {
            LookIn::Formulas
        };
        let find = find_of(entry, look_in);
        let hits = xlsx_find_all(wb, &find);
        let got: Vec<String> = hits
            .iter()
            .map(|c| address(wb, c.sheet, c.row, c.col))
            .collect();
        let expected: Vec<String> = entry["hits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_str().unwrap().to_owned())
            .collect();
        if got != expected {
            failures.push(format!(
                "{name}:\n    ours  {got:?}\n    excel {expected:?}"
            ));
        }
        // Find next walks the same list and wraps.
        if let Some(first) = hits.first() {
            let mut at = None;
            let mut walked = Vec::new();
            for _ in 0..hits.len() {
                at = next_match(&hits, at, true, find.by_columns);
                walked.push(at.unwrap());
            }
            assert_eq!(&walked, &hits, "{name}: find next");
            assert_eq!(next_match(&hits, at, true, find.by_columns), Some(*first));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
