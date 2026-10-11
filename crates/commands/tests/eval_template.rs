//! `evaluate_formula` and `Target::open_template` against the real fixtures.
#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use open365_commands::{
    Command, EffectClass, EvalValue, MAX_FORMULA_CHARS, Target, evaluate_formula,
    template_output_extension,
};
use open365_opc::Limits;
use open365_xlsx::Workbook;

fn fixture(rel: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel);
    std::fs::read(path).unwrap()
}

fn workbook() -> Workbook {
    Workbook::open(&fixture("xlsx/formulas-basic.xlsx"), &Limits::default()).unwrap()
}

fn all(_: usize, _: u32, _: u32) -> bool {
    true
}

#[test]
fn evaluates_without_changing_the_workbook() {
    let wb = workbook();
    let out = evaluate_formula(&wb, 0, 100, 5, Some("=2*(3+4)"), &all).unwrap();
    assert_eq!(out.formula, "2*(3+4)");
    assert_eq!(out.result, EvalValue::Number(14.0));
    assert!(wb.cell(0, 100, 5).is_none());
}

#[test]
fn errors_are_values_and_parse_errors_are_errors() {
    let wb = workbook();
    let out = evaluate_formula(&wb, 0, 1, 1, Some("1/0"), &all).unwrap();
    assert_eq!(out.result, EvalValue::Error("#DIV/0!".to_owned()));
    assert!(evaluate_formula(&wb, 0, 1, 1, Some("1+"), &all).is_err());
    assert!(evaluate_formula(&wb, 7, 1, 1, Some("1"), &all).is_err());
    let long = format!("1{}", "+1".repeat(MAX_FORMULA_CHARS));
    assert!(evaluate_formula(&wb, 0, 1, 1, Some(&long), &all).is_err());
}

#[test]
fn without_a_formula_the_cells_own_formula_is_used() {
    let wb = workbook();
    assert!(evaluate_formula(&wb, 0, 500, 5, None, &all).is_err());
    let (row, col) = wb
        .sheet(0)
        .unwrap()
        .iter_cells()
        .find(|(_, _, c)| c.formula.is_some())
        .map(|(r, c, _)| (r, c))
        .unwrap();
    let out = evaluate_formula(&wb, 0, row, col, None, &all).unwrap();
    assert!(!out.formula.is_empty());
    // A filter that hides the cell hides its formula too.
    let hide = move |_: usize, r: u32, c: u32| !(r == row && c == col);
    assert!(evaluate_formula(&wb, 0, row, col, None, &hide).is_err());
}

#[test]
fn array_results_are_capped() {
    let wb = workbook();
    let out = evaluate_formula(&wb, 0, 1, 1, Some("SEQUENCE(300,300)"), &all);
    if let Ok(out) = out
        && let EvalValue::Array(rows) = out.result
    {
        assert!(rows.len() <= 100 && rows[0].len() <= 100);
        assert!(out.truncated);
    }
}

#[test]
fn templates_become_unsaved_documents_and_save_as_exports() {
    for (rel, ext, out_ext, vba) in [
        ("docx/template-basic.dotx", "dotx", "docx", false),
        ("docx/template-macro.dotm", "dotm", "docm", true),
        ("xlsx/template-basic.xltx", "xltx", "xlsx", false),
        ("xlsx/template-macro.xltm", "xltm", "xlsm", true),
    ] {
        assert_eq!(template_output_extension(ext), Some(out_ext));
        let mut t = Target::open_template(&fixture(rel), ext).unwrap();
        assert!(t.path().is_none());
        let has_vba = t.docx().is_some_and(|d| d.has_vba_project())
            || t.xlsx().is_some_and(|w| w.has_vba_project());
        assert_eq!(has_vba, vba, "{rel}");
        let dir = std::env::temp_dir().join(format!("open365-commands-tpl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("out.{out_ext}"));
        let cmd = Command::Save { path: path.clone() };
        assert_eq!(cmd.effect(t.path()), EffectClass::Export);
        t.apply(cmd).unwrap();
        assert!(path.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[test]
fn non_templates_and_unknown_extensions_are_refused() {
    let plain = fixture("docx/plain-paragraphs.docx");
    assert!(Target::open_template(&plain, "dotx").is_err());
    assert!(Target::open_template(&plain, "docx").is_err());
    assert_eq!(template_output_extension("docx"), None);
    let book = fixture("xlsx/formulas-basic.xlsx");
    assert!(Target::open_template(&book, "xltx").is_err());
}
