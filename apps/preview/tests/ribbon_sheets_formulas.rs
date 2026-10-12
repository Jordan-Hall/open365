//! Sheets Formulas tab: every control runs through `Shell::run_command` and changes the sheet.

#![allow(clippy::unwrap_used)]

use open365_preview::ribbon::Handled;
use open365_preview::sheet::sheet_ext::formulas::ArrowKind;
use open365_preview::{AppKind, Preview};

mod dialogs_common;

/// The bare control opened the dialog `title`: it is modal, and Esc closes it unchanged.
fn cancels(p: &mut Preview, title: &str) {
    assert!(p.shell.dialog_open(), "{title} is open");
    assert_eq!(dialogs_common::title(p).as_deref(), Some(title));
    dialogs_common::key(p, open365_ui_core::Key::Escape);
    assert!(!p.shell.dialog_open(), "Esc closes {title}");
}

fn rig() -> Preview {
    Preview::new(AppKind::Sheets, true, 1600, 1000, 1.25)
}

fn run(p: &mut Preview, id: &str) {
    assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::Yes, "{id}");
}

#[test]
fn auto_sum_sums_the_numbers_above_then_left_and_undoes_in_one_step() {
    let mut p = rig();
    {
        let s = p.shell.sheet_mut();
        for (r, v) in [(1, "Total"), (2, "1"), (3, "2"), (4, "3")] {
            s.enter(r, 1, v).unwrap();
        }
        s.select_cell((5, 1));
    }
    run(&mut p, "formulas.function-library.auto-sum");
    let s = p.shell.sheet();
    assert_eq!(
        s.source(5, 1),
        "=SUM(A2:A4)",
        "the text header stops the range"
    );
    assert_eq!(s.display(5, 1), "6");
    p.shell.sheet_mut().undo();
    assert_eq!(p.shell.sheet().source(5, 1), "");

    {
        let s = p.shell.sheet_mut();
        s.enter(7, 1, "4").unwrap();
        s.enter(7, 2, "5").unwrap();
        s.select_cell((7, 3));
    }
    run(&mut p, "formulas.function-library.auto-sum.average");
    assert_eq!(p.shell.sheet().source(7, 3), "=AVERAGE(A7:B7)");
    assert_eq!(p.shell.sheet().display(7, 3), "4.5");
}

#[test]
fn auto_sum_on_a_range_totals_each_column() {
    let mut p = rig();
    {
        let s = p.shell.sheet_mut();
        for (r, c, v) in [(1, 1, "1"), (2, 1, "2"), (1, 2, "10"), (2, 2, "20")] {
            s.enter(r, c, v).unwrap();
        }
        s.select_cell((1, 1));
        s.focus = (2, 2);
    }
    run(&mut p, "formulas.function-library.auto-sum.max");
    let s = p.shell.sheet();
    assert_eq!(s.source(3, 1), "=MAX(A1:A2)");
    assert_eq!(s.display(3, 2), "20");
    assert_eq!(s.active, (3, 1));
}

#[test]
fn function_library_lists_ask_for_the_arguments_dialog() {
    let mut p = rig();
    run(&mut p, "formulas.function-library.insert-function");
    cancels(&mut p, "Insert Function");
    run(&mut p, "formulas.function-library.logical");
    assert!(
        dialogs_common::dump(&p).iter().any(|l| l == "IF"),
        "the Logical category lists IF: {:?}",
        dialogs_common::dump(&p)
    );
    cancels(&mut p, "Insert Function");
    run(&mut p, "formulas.function-library.logical.if");
    cancels(&mut p, "Function Arguments");
    // OK in the arguments dialog writes the formula and fills Recently Used.
    p.shell.sheet_mut().enter(1, 1, "1").unwrap();
    p.shell.sheet_mut().select_cell((2, 1));
    p.shell
        .sheet_mut()
        .apply_function("sum", &["A1", ""])
        .unwrap();
    assert_eq!(p.shell.sheet().source(2, 1), "=SUM(A1)");
    assert_eq!(
        p.shell
            .sheet()
            .recent_functions()
            .first()
            .map(String::as_str),
        Some("SUM")
    );
}

#[test]
fn show_formulas_switches_the_displayed_text() {
    let mut p = rig();
    p.shell.sheet_mut().enter(1, 1, "2").unwrap();
    p.shell.sheet_mut().enter(1, 2, "=A1*3").unwrap();
    let before = p.shell.sheet().ext_formulas.show_formulas;
    assert_eq!(p.shell.sheet().shown_for_view(1, 2).unwrap().0, "6");
    run(&mut p, "formulas.formula-auditing.show-formulas");
    assert_ne!(p.shell.sheet().ext_formulas.show_formulas, before);
    assert_eq!(p.shell.sheet().shown_for_view(1, 2).unwrap().0, "=A1*3");
    run(&mut p, "formulas.formula-auditing.show-formulas");
    assert_eq!(p.shell.sheet().shown_for_view(1, 2).unwrap().0, "6");
}

#[test]
fn trace_precedents_and_dependents_add_levels_and_remove_arrows_clears() {
    let mut p = rig();
    {
        let s = p.shell.sheet_mut();
        s.enter(1, 1, "1").unwrap();
        s.enter(1, 2, "=A1+1").unwrap();
        s.enter(1, 3, "=B1*2").unwrap();
        s.select_cell((1, 3));
    }
    run(&mut p, "formulas.formula-auditing.remove-arrows");
    run(&mut p, "formulas.formula-auditing.trace-precedents");
    let a = p.shell.sheet().ext_formulas.arrows.clone();
    assert_eq!(a.len(), 1);
    assert_eq!(
        (a[0].from, a[0].to, a[0].kind),
        ((1, 2, 1, 2), (1, 3), ArrowKind::Precedent)
    );
    run(&mut p, "formulas.formula-auditing.trace-precedents");
    assert_eq!(
        p.shell.sheet().ext_formulas.arrows.clone().len(),
        2,
        "second click adds the next level"
    );
    run(&mut p, "formulas.formula-auditing.remove-arrows");
    assert!(p.shell.sheet().ext_formulas.arrows.clone().is_empty());

    p.shell.sheet_mut().select_cell((1, 1));
    run(&mut p, "formulas.formula-auditing.trace-dependents");
    let a = p.shell.sheet().ext_formulas.arrows.clone();
    assert_eq!(
        (a[0].from, a[0].to, a[0].kind),
        ((1, 1, 1, 1), (1, 2), ArrowKind::Dependent)
    );
    run(&mut p, "formulas.formula-auditing.trace-dependents");
    assert_eq!(p.shell.sheet().ext_formulas.arrows.clone().len(), 2);
    run(&mut p, "formulas.formula-auditing.remove-arrows");
}

#[test]
fn error_checking_selects_the_next_error() {
    let mut p = rig();
    {
        let s = p.shell.sheet_mut();
        s.enter(1, 1, "=1/0").unwrap();
        s.enter(3, 2, "=1/0").unwrap();
        s.select_cell((1, 1));
    }
    run(&mut p, "formulas.formula-auditing.error-checking");
    assert_eq!(p.shell.sheet().active, (3, 2));
    assert!(p.shell.sheet().status.contains("B3"));
    run(&mut p, "formulas.formula-auditing.error-checking");
    assert_eq!(p.shell.sheet().active, (1, 1), "wraps around");
}

#[test]
fn evaluate_formula_steps_to_the_value() {
    let mut p = rig();
    {
        let s = p.shell.sheet_mut();
        s.enter(1, 1, "2").unwrap();
        s.enter(1, 2, "=(A1+1)*4").unwrap();
        s.select_cell((1, 2));
    }
    run(&mut p, "formulas.formula-auditing.evaluate-formula");
    cancels(&mut p, "Evaluate Formula");
    let mut session = p.shell.sheet().evaluate_session().unwrap();
    let mut seen = vec![session.text()];
    while !session.done() {
        assert!(p.shell.sheet().evaluate_step(&mut session));
        seen.push(session.text());
    }
    assert_eq!(seen.first().unwrap(), "=(A1+1)*4");
    assert!(seen.contains(&"=(2+1)*4".to_owned()), "{seen:?}");
    assert_eq!(seen.last().unwrap(), "=12");

    // A constant gets Excel's message and no dialog.
    p.shell.sheet_mut().select_cell((1, 1));
    run(&mut p, "formulas.formula-auditing.evaluate-formula");
    assert!(!p.shell.dialog_open());
    assert!(p.shell.sheet().status.contains("constant"));
}

#[test]
fn name_manager_requests_the_dialog() {
    let mut p = rig();
    run(&mut p, "formulas.defined-names.name-manager");
    cancels(&mut p, "Name Manager");
    assert!(p.shell.sheet().name_rows().is_empty());
}
