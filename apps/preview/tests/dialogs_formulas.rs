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

// ---- Name Manager: New, Edit, Delete -------------------------------------------------------------

#[test]
fn name_manager_creates_edits_and_deletes_names_with_excels_messages() {
    let mut p = sheets();
    fill_col(&mut p, 1, &["1", "2", "3"]);
    p.shell.sheet_mut().select_cell((1, 1));
    p.shell.sheet_mut().extend_to((3, 1));
    assert!(bare(&mut p, "formulas.defined-names.name-manager"));
    assert_eq!(title(&p).as_deref(), Some("Name Manager"));

    // New...: the reference starts as the selection.
    alt(&mut p, 'n');
    assert_eq!(title(&p).as_deref(), Some("New Name"));
    type_text(&mut p, "Tax");
    key(&mut p, Key::Enter);
    assert_eq!(title(&p).as_deref(), Some("Name Manager"), "{:?}", dump(&p));
    let rows = p.shell.sheet().name_rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "Tax");
    assert_eq!(rows[0].refers_to, "=Sheet1!$A$1:$A$3");
    assert!(
        dump(&p).iter().any(|l| l.starts_with("Tax")),
        "the list shows the new name"
    );
    // The name works in a formula.
    p.shell.sheet_mut().enter(1, 3, "=SUM(Tax)").unwrap();
    assert_eq!(
        p.shell.sheet().shown(1, 3).map(|s| s.0),
        Some("6".to_owned())
    );

    // A name Excel refuses: a message box over New Name, which stays.
    alt(&mut p, 'n');
    type_text(&mut p, "1bad");
    key(&mut p, Key::Enter);
    assert_eq!(title(&p).as_deref(), Some("Open365"));
    assert!(dump(&p).contains(&"The name that you entered is not valid.".to_owned()));
    key(&mut p, Key::Enter);
    assert_eq!(title(&p).as_deref(), Some("New Name"));
    // A name that exists.
    alt(&mut p, 'n');
    type_text(&mut p, "tax");
    key(&mut p, Key::Enter);
    assert_eq!(title(&p).as_deref(), Some("Open365"), "{:?}", dump(&p));
    assert!(dump(&p).contains(&"That name is already taken. Try a different one.".to_owned()));
    key(&mut p, Key::Enter);
    key(&mut p, Key::Escape);
    assert_eq!(title(&p).as_deref(), Some("Name Manager"));

    // Edit... renames and retargets; the scope cannot change.
    alt(&mut p, 'e');
    assert_eq!(title(&p).as_deref(), Some("Edit Name"));
    type_text(&mut p, "VAT");
    key(&mut p, Key::Enter);
    assert_eq!(title(&p).as_deref(), Some("Name Manager"), "{:?}", dump(&p));
    assert_eq!(p.shell.sheet().name_rows()[0].name, "VAT");

    // Delete asks first.
    alt(&mut p, 'd');
    assert_eq!(title(&p).as_deref(), Some("Open365"));
    assert!(dump(&p).contains(&"Are you sure you want to delete the name 'VAT'?".to_owned()));
    key(&mut p, Key::Escape);
    assert_eq!(p.shell.sheet().name_rows().len(), 1);
    alt(&mut p, 'd');
    key(&mut p, Key::Enter);
    assert_eq!(title(&p).as_deref(), Some("Name Manager"));
    assert!(p.shell.sheet().name_rows().is_empty());
    key(&mut p, Key::Escape);
    assert!(!is_open(&p));
    // One undo step each.
    assert!(p.shell.sheet_mut().undo());
    assert_eq!(p.shell.sheet().name_rows().len(), 1);
}

// ---- Function Arguments: live results and descriptions ---------------------------------------------

#[test]
fn function_arguments_shows_each_value_the_result_and_what_the_argument_wants() {
    let mut p = sheets();
    fill_col(&mut p, 1, &["1", "2"]);
    p.shell.sheet_mut().select_cell((4, 2));
    assert!(p.shell.open_dialog(
        &mut p.ui,
        DialogRequest::FunctionArguments {
            name: "SUM".to_owned()
        }
    ));
    let lines = dump(&p);
    assert!(lines.contains(&"Adds the numbers.".to_owned()), "{lines:?}");
    assert!(
        lines.contains(&"number1: The number to work on.".to_owned()),
        "{lines:?}"
    );
    type_text(&mut p, "A1:A2");
    let lines = dump(&p);
    assert!(lines.contains(&"= {1;2}".to_owned()), "{lines:?}");
    assert!(
        lines.contains(&"Formula result = 3".to_owned()),
        "{lines:?}"
    );
    keys(&mut p, &[Key::Tab, Key::Tab]);
    type_text(&mut p, "5");
    let lines = dump(&p);
    assert!(lines.contains(&"= 5".to_owned()), "{lines:?}");
    assert!(
        lines.contains(&"Formula result = 8".to_owned()),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"number2: Optional. The number to work on.".to_owned()),
        "the description follows the box being edited: {lines:?}"
    );
    // Text shows in quotes beside its box, and plain in the result.
    key(&mut p, Key::Escape);
    assert!(p.shell.open_dialog(
        &mut p.ui,
        DialogRequest::FunctionArguments {
            name: "LEFT".to_owned()
        }
    ));
    type_text(&mut p, "\"hello\"");
    keys(&mut p, &[Key::Tab, Key::Tab]);
    type_text(&mut p, "2");
    let lines = dump(&p);
    assert!(lines.contains(&"= \"hello\"".to_owned()), "{lines:?}");
    assert!(
        lines.contains(&"Formula result = he".to_owned()),
        "{lines:?}"
    );
}

#[test]
fn every_function_of_the_engine_opens_function_arguments() {
    let mut p = sheets();
    for name in open365_calc::functions::function_names() {
        assert!(
            p.shell.open_dialog(
                &mut p.ui,
                DialogRequest::FunctionArguments {
                    name: name.to_owned()
                }
            ),
            "{name}"
        );
        assert!(
            dump(&p).iter().any(|l| l.contains(':')),
            "{name} describes its arguments"
        );
        key(&mut p, Key::Escape);
        assert!(!is_open(&p), "{name}");
    }
}

// ---- Evaluate Formula: Step In and Step Out ---------------------------------------------------------

#[test]
fn evaluate_formula_steps_in_to_a_cells_formula_and_out_again() {
    let mut p = sheets();
    fill_col(&mut p, 1, &["2", "=A1*3", "=A2+1"]);
    p.shell.sheet_mut().select_cell((3, 1));
    assert!(bare(&mut p, "formulas.formula-auditing.evaluate-formula"));
    assert!(find(&p, "=A2+1").is_some());
    assert!(find(&p, "Sheet1!$A$3").is_some());
    let enabled = |p: &open365_preview::Preview, label: &str| {
        p.ui.tree.get(find(p, label).unwrap()).unwrap().enabled
    };
    assert!(enabled(&p, "Step In"), "A2 holds a formula");
    assert!(!enabled(&p, "Step Out"));
    alt(&mut p, 'i');
    assert!(find(&p, "=A1*3").is_some(), "{:?}", dump(&p));
    assert!(find(&p, "Sheet1!$A$2").is_some());
    assert!(!enabled(&p, "Step In"), "A1 is a constant");
    assert!(enabled(&p, "Step Out"));
    alt(&mut p, 'o');
    // Stepping out works the inner formula out in one go: A2 is 6.
    assert!(find(&p, "=6+1").is_some(), "{:?}", dump(&p));
    assert!(find(&p, "Sheet1!$A$3").is_some());
    assert!(!enabled(&p, "Step Out"));
    alt(&mut p, 'e');
    assert!(find(&p, "=7").is_some());
    alt(&mut p, 'r');
    assert!(find(&p, "=A2+1").is_some());
    assert!(enabled(&p, "Step In"));
}
