//! Defined names (relative, sheet-scoped), legacy array formulas and tables in the engine.

use crate::ErrorKind;
use crate::test_support::{SHEET, err, n, t};
use crate::{Engine, FixedClock, TableDef};

fn engine() -> Engine {
    let mut e = Engine::with_clock(Box::new(FixedClock(1_700_000_000.0)));
    e.add_sheet(SHEET);
    e
}

fn plan_table() -> TableDef {
    // A1:C4: headers in row 1, data rows 2 to 4.
    TableDef {
        name: "Plan".into(),
        rect: (1, 1, 4, 3),
        header_rows: 1,
        totals_rows: 0,
        columns: vec!["Name".into(), "Jan".into(), "Feb".into()],
    }
}

fn plan_engine() -> Engine {
    let mut e = engine();
    for (r, row) in [
        [t("Name"), t("Jan"), t("Feb")],
        [t("a"), n(1.0), n(10.0)],
        [t("b"), n(2.0), n(20.0)],
        [t("c"), n(3.0), n(30.0)],
    ]
    .iter()
    .enumerate()
    {
        for (c, v) in row.iter().enumerate() {
            e.set_value(SHEET, r as u32 + 1, c as u32 + 1, v.clone());
        }
    }
    e.define_table(SHEET, plan_table());
    e
}

#[test]
fn tables_work_with_index_match_and_row() {
    let mut e = plan_engine();
    e.set_formula(SHEET, 10, 1, "INDEX(Plan[],2,3)").unwrap();
    e.set_formula(SHEET, 11, 1, "ROW(Plan[[#Headers],[Jan]])")
        .unwrap();
    e.set_formula(SHEET, 12, 1, "MATCH(\"b\",INDEX(Plan[],,1),0)")
        .unwrap();
    e.set_formula(SHEET, 13, 1, "MATCH(\"Feb\",Plan[#Headers],0)")
        .unwrap();
    e.set_formula(SHEET, 14, 1, "ROW()-ROW(Plan[[#Headers],[Jan]])")
        .unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 10, 1), n(20.0));
    assert_eq!(e.get(SHEET, 11, 1), n(1.0));
    assert_eq!(e.get(SHEET, 12, 1), n(2.0));
    assert_eq!(e.get(SHEET, 13, 1), n(3.0));
    assert_eq!(e.get(SHEET, 14, 1), n(13.0));
}

#[test]
fn range_to_index_reference() {
    let mut e = engine();
    for r in 1..=5 {
        e.set_value(SHEET, r, 1, n(f64::from(r)));
    }
    e.set_formula(SHEET, 1, 3, "SUM(A1:INDEX(A1:A5,3))")
        .unwrap();
    e.define_name("Upto", "Sheet1!$A$1:INDEX(Sheet1!$A$1:$A$5,3)")
        .unwrap();
    e.set_formula(SHEET, 2, 3, "SUM(Upto)").unwrap();
    e.set_array_formula(
        SHEET,
        (3, 3, 3, 3),
        "MIN(IF(ISNUMBER(Upto),IF(Upto>1,Upto)))",
    )
    .unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 3), n(6.0));
    assert_eq!(e.get(SHEET, 2, 3), n(6.0));
    assert_eq!(e.get(SHEET, 3, 3), n(2.0));
}

#[test]
fn array_formulas_evaluate_if_over_ranges() {
    let mut e = engine();
    for r in 1..=5 {
        e.set_value(SHEET, r, 1, n(f64::from(r)));
    }
    e.set_array_formula(SHEET, (1, 3, 1, 3), "SUM(IF(A1:A5>2,1,0))")
        .unwrap();
    e.set_array_formula(SHEET, (2, 3, 2, 3), "MIN(IF(A1:A5>1,A1:A5))")
        .unwrap();
    e.set_array_formula(
        SHEET,
        (3, 3, 3, 3),
        "MIN(IF(ISNUMBER(A1:A5),IF(A1:A5>1,A1:A5)))",
    )
    .unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 3), n(3.0));
    assert_eq!(e.get(SHEET, 2, 3), n(2.0));
    assert_eq!(e.get(SHEET, 3, 3), n(2.0));
}

#[test]
fn relative_names_follow_the_formula_cell() {
    let mut e = engine();
    for r in 1..=4 {
        e.set_value(SHEET, r, 2, n(f64::from(r) * 10.0));
    }
    e.define_name("LeftOf", "Sheet1!$B1").unwrap();
    e.define_name("Wrap", "Sheet1!$B1048576").unwrap();
    e.set_formula(SHEET, 3, 3, "LeftOf").unwrap();
    e.set_formula(SHEET, 4, 3, "LeftOf+1").unwrap();
    e.set_formula(SHEET, 2, 4, "Wrap").unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 3, 3), n(30.0));
    assert_eq!(e.get(SHEET, 4, 3), n(41.0));
    assert_eq!(e.get(SHEET, 2, 4), n(10.0));
    e.set_value(SHEET, 3, 2, n(99.0));
    e.recalc_dirty();
    assert_eq!(
        e.get(SHEET, 3, 3),
        n(99.0),
        "dependency moves with the cell"
    );
}

#[test]
fn sheet_scoped_names_hide_workbook_names() {
    let mut e = engine();
    e.add_sheet("Other");
    e.set_value(SHEET, 1, 1, n(1.0));
    e.set_value("Other", 1, 1, n(2.0));
    e.define_name("X", "Sheet1!$A$1").unwrap();
    e.define_name_scoped("Other", "X", "Other!$A$1").unwrap();
    e.define_name_scoped("Other", "OnlyHere", "Other!$A$1")
        .unwrap();
    e.set_formula(SHEET, 2, 1, "X").unwrap();
    e.set_formula("Other", 2, 1, "X").unwrap();
    e.set_formula(SHEET, 3, 1, "OnlyHere").unwrap();
    e.set_formula("Other", 3, 1, "OnlyHere").unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 2, 1), n(1.0));
    assert_eq!(e.get("Other", 2, 1), n(2.0));
    assert_eq!(e.get(SHEET, 3, 1), err(ErrorKind::Name));
    assert_eq!(e.get("Other", 3, 1), n(2.0));
}

#[test]
fn legacy_array_formulas_fill_their_range() {
    let mut e = engine();
    for c in 1..=3 {
        e.set_value(SHEET, 1, c, n(f64::from(c)));
    }
    e.set_array_formula(SHEET, (3, 1, 3, 3), "A1:C1*2").unwrap();
    e.set_array_formula(SHEET, (5, 1, 7, 1), "TRANSPOSE(A1:C1)")
        .unwrap();
    e.set_array_formula(SHEET, (9, 1, 9, 2), "5").unwrap();
    e.set_formula(SHEET, 1, 5, "C3+A7").unwrap();
    e.recalc_dirty();
    assert_eq!(
        [e.get(SHEET, 3, 1), e.get(SHEET, 3, 2), e.get(SHEET, 3, 3)],
        [n(2.0), n(4.0), n(6.0)]
    );
    assert_eq!(e.get(SHEET, 7, 1), n(3.0));
    assert_eq!(e.get(SHEET, 9, 2), n(5.0), "scalar fills the range");
    assert_eq!(e.get(SHEET, 1, 5), n(9.0));
    e.set_value(SHEET, 1, 3, n(10.0));
    e.recalc_dirty();
    assert_eq!(
        e.get(SHEET, 3, 3),
        n(20.0),
        "members follow their precedents"
    );
    assert_eq!(
        e.get(SHEET, 1, 5),
        n(30.0),
        "dependents of a member recalculate"
    );
}

#[test]
fn structured_references() {
    let mut e = plan_engine();
    e.set_formula(SHEET, 10, 1, "SUM(Plan[Jan])").unwrap();
    e.set_formula(
        SHEET,
        3,
        5,
        "Plan[[#This Row],[Jan]]*Plan[[#This Row],[Feb]]",
    )
    .unwrap();
    e.set_formula(SHEET, 11, 1, "Plan[[#Headers],[Feb]]")
        .unwrap();
    e.set_formula(SHEET, 12, 1, "SUM(Nope[Jan])").unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 10, 1), n(6.0));
    assert_eq!(e.get(SHEET, 3, 5), n(40.0));
    assert_eq!(e.get(SHEET, 11, 1), t("Feb"));
    assert_eq!(e.get(SHEET, 12, 1), err(ErrorKind::Ref));
    e.set_value(SHEET, 2, 2, n(100.0));
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 10, 1), n(105.0));
    // Defining the table after the formula re-reads it.
    let mut late = engine();
    late.set_value(SHEET, 2, 2, n(7.0));
    late.set_formula(SHEET, 10, 1, "SUM(Plan[Jan])").unwrap();
    late.recalc_dirty();
    assert_eq!(late.get(SHEET, 10, 1), err(ErrorKind::Ref));
    late.define_table(SHEET, plan_table());
    late.recalc_dirty();
    assert_eq!(late.get(SHEET, 10, 1), n(7.0));
}

#[test]
fn a_two_dimensional_range_gives_the_cell_at_the_formula_position() {
    let mut e = engine();
    e.add_sheet("Other");
    e.set_value("Other", 2, 2, t("hit"));
    e.set_formula(SHEET, 2, 2, "Other!B2:D3").unwrap();
    e.set_formula(SHEET, 9, 9, "Other!B2:D3").unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 2, 2), t("hit"));
    assert_eq!(e.get(SHEET, 9, 9), err(ErrorKind::Value));
}
