//! Insert Function, Function Arguments, Name Manager and Evaluate Formula.

#![allow(clippy::unwrap_used)]

mod dialogs_common;

use dialogs_common::*;
use open365_preview::dialogs::request::DialogRequest;
use open365_ui_core::Key;

#[test]
fn insert_function_search_leads_to_function_arguments_and_ok_writes_the_formula() {
    let mut p = sheets();
    fill_col(&mut p, 1, &["1", "2", "3"]);
    p.shell.sheet_mut().select_cell((5, 1));
    assert!(bare(&mut p, "formulas.function-library.insert-function"));
    assert_eq!(title(&p).as_deref(), Some("Insert Function"));
    // The search box has the focus: type, and the list narrows.
    type_text(&mut p, "sumproduct");
    key(&mut p, Key::Enter); // OK opens Function Arguments
    assert_eq!(title(&p).as_deref(), Some("Function Arguments"));
    type_text(&mut p, "A1:A3");
    key(&mut p, Key::Enter);
    assert!(!is_open(&p));
    assert_eq!(p.shell.sheet().source(5, 1), "=SUMPRODUCT(A1:A3)");
}

#[test]
fn insert_function_needs_a_function_and_escape_leaves() {
    let mut p = sheets();
    assert!(bare(&mut p, "formulas.function-library.insert-function"));
    type_text(&mut p, "zzzznotafunction");
    key(&mut p, Key::Enter);
    assert!(is_open(&p), "nothing selected: the dialog stays");
    assert_eq!(title(&p).as_deref(), Some("Insert Function"));
    key(&mut p, Key::Escape);
    assert!(!is_open(&p));
}

#[test]
fn function_arguments_takes_several_arguments_with_tab() {
    let mut p = sheets();
    fill_col(&mut p, 1, &["1", "2"]);
    p.shell.sheet_mut().select_cell((4, 2));
    assert!(p.shell.open_dialog(
        &mut p.ui,
        DialogRequest::FunctionArguments {
            name: "SUM".to_owned()
        }
    ));
    type_text(&mut p, "A1:A2");
    keys(&mut p, &[Key::Tab, Key::Tab]); // past the collapse button
    type_text(&mut p, "5");
    key(&mut p, Key::Enter);
    assert!(!is_open(&p));
    assert_eq!(p.shell.sheet().source(4, 2), "=SUM(A1:A2,5)");
    assert!(
        p.shell
            .sheet()
            .recent_functions()
            .contains(&"SUM".to_owned())
    );
}

#[test]
fn name_manager_opens_and_closes() {
    let mut p = sheets();
    assert!(bare(&mut p, "formulas.defined-names.name-manager"));
    assert_eq!(title(&p).as_deref(), Some("Name Manager"));
    key(&mut p, Key::Escape);
    assert!(!is_open(&p));
}

#[test]
fn evaluate_formula_steps_to_the_value() {
    let mut p = sheets();
    fill_col(&mut p, 1, &["1", "2", "=A1+A2*2"]);
    p.shell.sheet_mut().select_cell((3, 1));
    assert!(bare(&mut p, "formulas.formula-auditing.evaluate-formula"));
    assert_eq!(title(&p).as_deref(), Some("Evaluate Formula"));
    for _ in 0..8 {
        key(&mut p, Key::Enter); // Evaluate
    }
    assert!(find(&p, "=5").is_some(), "evaluated down to the value");
    alt(&mut p, 'r'); // Restart
    assert!(find(&p, "=A1+A2*2").is_some());
    key(&mut p, Key::Escape);
    assert!(!is_open(&p));
}

#[test]
fn a_constant_cell_does_not_open_evaluate() {
    let mut p = sheets();
    p.shell.sheet_mut().enter(1, 1, "7").unwrap();
    p.shell.sheet_mut().select_cell((1, 1));
    assert!(!bare(&mut p, "formulas.formula-auditing.evaluate-formula"));
}
