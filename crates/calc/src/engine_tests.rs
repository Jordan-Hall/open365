//! Tests of the in-memory engine: incremental recalculation, cycles, names, sheets.

use std::collections::HashSet;

use crate::ErrorKind::{DivZero, Name, Ref};
use crate::test_support::{CLOCK_SECONDS, SHEET, err, n, t};
use crate::{CellAddr, Engine, FixedClock, Value};

fn engine() -> Engine {
    Engine::with_clock(Box::new(FixedClock(CLOCK_SECONDS)))
}

fn addrs(cells: Vec<CellAddr>) -> HashSet<(String, u32, u32)> {
    cells.into_iter().map(|c| (c.sheet, c.row, c.col)).collect()
}

fn at(sheet: &str, row: u32, col: u32) -> (String, u32, u32) {
    (sheet.to_owned(), row, col)
}

#[test]
fn constants_round_trip() {
    let mut e = engine();
    e.set_value(SHEET, 1, 1, n(5.0));
    e.set_value(SHEET, 2, 1, t("hi"));
    e.set_value(SHEET, 3, 1, Value::Bool(true));
    assert_eq!(e.get(SHEET, 1, 1), n(5.0));
    assert_eq!(e.get(SHEET, 2, 1), t("hi"));
    assert_eq!(e.get(SHEET, 3, 1), Value::Bool(true));
    assert_eq!(e.get(SHEET, 9, 9), Value::Empty);
    assert_eq!(e.get("Nowhere", 1, 1), Value::Empty);
    e.clear(SHEET, 1, 1);
    assert_eq!(e.get(SHEET, 1, 1), Value::Empty);
    assert_eq!(e.sheet_names(), vec![SHEET]);
}

#[test]
fn formulas_are_evaluated_on_recalc() {
    let mut e = engine();
    e.set_value(SHEET, 1, 1, n(2.0));
    e.set_formula(SHEET, 1, 2, "=A1*10").unwrap();
    assert_eq!(e.dirty_count(), 1);
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 2), n(20.0));
    assert_eq!(e.dirty_count(), 0);
    assert_eq!(e.get_formula(SHEET, 1, 2).as_deref(), Some("A1*10"));
    assert_eq!(e.get_formula(SHEET, 1, 1), None);
}

#[test]
fn get_formula_is_canonical() {
    let mut e = engine();
    e.set_formula(SHEET, 1, 1, "= sum( a1:b2 ,  1 )").unwrap();
    assert_eq!(e.get_formula(SHEET, 1, 1).as_deref(), Some("SUM(A1:B2,1)"));
}

#[test]
fn only_affected_cells_are_recalculated() {
    let mut e = engine();
    e.set_value(SHEET, 1, 1, n(1.0)); // A1
    e.set_formula(SHEET, 1, 2, "A1+1").unwrap(); // B1
    e.set_formula(SHEET, 1, 3, "B1*2").unwrap(); // C1
    e.set_formula(SHEET, 1, 4, "5").unwrap(); // D1 unrelated
    let first = e.recalc_dirty();
    assert_eq!(first.len(), 3);
    assert_eq!(e.get(SHEET, 1, 3), n(4.0));

    e.set_value(SHEET, 1, 1, n(10.0));
    let second = addrs(e.recalc_dirty());
    assert_eq!(second, HashSet::from([at(SHEET, 1, 2), at(SHEET, 1, 3)]));
    assert_eq!(e.get(SHEET, 1, 2), n(11.0));
    assert_eq!(e.get(SHEET, 1, 3), n(22.0));
    assert!(e.recalc_dirty().is_empty());
}

#[test]
fn recalculation_follows_dependency_order() {
    let mut e = engine();
    // Defined bottom-up on purpose: A1 depends on A2 depends on A3 depends on A4.
    e.set_formula(SHEET, 1, 1, "A2+1").unwrap();
    e.set_formula(SHEET, 2, 1, "A3+1").unwrap();
    e.set_formula(SHEET, 3, 1, "A4+1").unwrap();
    e.set_value(SHEET, 4, 1, n(100.0));
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 1), n(103.0));
}

#[test]
fn replacing_a_formula_with_a_constant_drops_its_dependencies() {
    let mut e = engine();
    e.set_value(SHEET, 1, 1, n(1.0));
    e.set_formula(SHEET, 1, 2, "A1").unwrap();
    e.recalc_dirty();
    e.set_value(SHEET, 1, 2, n(50.0));
    assert!(!e.graph().contains(crate::graph::CellId {
        sheet: 0,
        row: 1,
        col: 2
    }));
    e.set_value(SHEET, 1, 1, n(2.0));
    assert_eq!(e.dirty_count(), 0);
    assert_eq!(e.get(SHEET, 1, 2), n(50.0));
}

#[test]
fn changing_a_formula_rewires_dependencies() {
    let mut e = engine();
    e.set_value(SHEET, 1, 1, n(1.0));
    e.set_value(SHEET, 2, 1, n(2.0));
    e.set_formula(SHEET, 1, 2, "A1").unwrap();
    e.recalc_dirty();
    e.set_formula(SHEET, 1, 2, "A2").unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 2), n(2.0));
    e.set_value(SHEET, 1, 1, n(99.0));
    assert_eq!(e.dirty_count(), 0, "B1 no longer reads A1");
    e.set_value(SHEET, 2, 1, n(7.0));
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 2), n(7.0));
}

#[test]
fn parse_errors_leave_the_cell_untouched() {
    let mut e = engine();
    e.set_formula(SHEET, 1, 1, "1+1").unwrap();
    e.recalc_dirty();
    assert!(e.set_formula(SHEET, 1, 1, "1+").is_err());
    assert_eq!(e.get_formula(SHEET, 1, 1).as_deref(), Some("1+1"));
    assert_eq!(e.get(SHEET, 1, 1), n(2.0));
}

#[test]
fn two_cell_cycle_shows_zero_and_is_flagged() {
    let mut e = engine();
    e.set_formula(SHEET, 1, 1, "B1+1").unwrap(); // A1
    e.set_formula(SHEET, 1, 2, "A1+1").unwrap(); // B1
    e.set_formula(SHEET, 1, 3, "A1+5").unwrap(); // C1 downstream of the cycle
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 1), n(0.0));
    assert_eq!(e.get(SHEET, 1, 2), n(0.0));
    assert!(e.is_circular(SHEET, 1, 1));
    assert!(e.is_circular(SHEET, 1, 2));
    assert!(!e.is_circular(SHEET, 1, 3));
    assert_eq!(e.get(SHEET, 1, 3), n(5.0), "downstream cells see the 0");
    let cells = addrs(e.circular_cells());
    assert_eq!(cells, HashSet::from([at(SHEET, 1, 1), at(SHEET, 1, 2)]));
}

#[test]
fn breaking_a_cycle_restores_normal_evaluation() {
    let mut e = engine();
    e.set_formula(SHEET, 1, 1, "B1+1").unwrap();
    e.set_formula(SHEET, 1, 2, "A1+1").unwrap();
    e.recalc_dirty();
    assert!(e.is_circular(SHEET, 1, 1));
    e.set_value(SHEET, 1, 2, n(10.0));
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 1), n(11.0));
    assert!(!e.is_circular(SHEET, 1, 1));
    assert!(e.circular_cells().is_empty());
}

#[test]
fn self_reference_and_range_cycle() {
    let mut e = engine();
    e.set_formula(SHEET, 1, 1, "A1+1").unwrap();
    e.set_formula(SHEET, 5, 2, "SUM(B1:B10)").unwrap();
    e.recalc_dirty();
    assert!(e.is_circular(SHEET, 1, 1));
    assert!(e.is_circular(SHEET, 5, 2));
    assert_eq!(e.get(SHEET, 5, 2), n(0.0));
}

#[test]
fn cross_sheet_references() {
    let mut e = engine();
    e.set_value("Data", 1, 1, n(4.0));
    e.set_value("Data", 2, 1, n(6.0));
    e.set_formula("Calc", 1, 1, "SUM(Data!A1:A2)*2").unwrap();
    e.set_formula("Calc", 2, 1, "'data'!A1+1").unwrap();
    e.recalc_dirty();
    assert_eq!(e.get("Calc", 1, 1), n(20.0));
    assert_eq!(
        e.get("Calc", 2, 1),
        n(5.0),
        "sheet names are case-insensitive"
    );
    e.set_value("Data", 2, 1, n(1.0));
    e.recalc_dirty();
    assert_eq!(e.get("Calc", 1, 1), n(10.0));
}

#[test]
fn missing_sheet_is_ref_error_until_created() {
    let mut e = engine();
    e.set_formula(SHEET, 1, 1, "Later!A1+1").unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 1), err(Ref));
    e.set_value("Later", 1, 1, n(41.0));
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 1), n(42.0));
}

#[test]
fn defined_names() {
    let mut e = engine();
    e.set_value(SHEET, 1, 1, n(0.2));
    e.set_value(SHEET, 2, 1, n(1.0));
    e.set_value(SHEET, 3, 1, n(2.0));
    e.set_formula(SHEET, 1, 2, "Rate*100").unwrap();
    e.set_formula(SHEET, 2, 2, "SUM(Items)").unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 2), err(Name), "undefined name");

    e.define_name("Rate", "Sheet1!$A$1").unwrap();
    e.define_name("items", "Sheet1!$A$2:$A$3").unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 2), n(20.0));
    assert_eq!(e.get(SHEET, 2, 2), n(3.0));

    e.set_value(SHEET, 1, 1, n(0.5));
    e.set_value(SHEET, 3, 1, n(9.0));
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 2), n(50.0), "dependency through the name");
    assert_eq!(e.get(SHEET, 2, 2), n(10.0));

    e.define_name("Rate", "Sheet1!$A$2").unwrap();
    e.recalc_dirty();
    assert_eq!(
        e.get(SHEET, 1, 2),
        n(100.0),
        "redefinition recalculates users"
    );
    e.set_value(SHEET, 2, 1, n(3.0));
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 2), n(300.0));

    e.remove_name("Rate");
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 2), err(Name));
}

#[test]
fn names_can_hold_constants_and_expressions() {
    let mut e = engine();
    e.define_name("Tau", "PI()*2").unwrap();
    e.define_name("Limit", "10").unwrap();
    e.set_formula(SHEET, 1, 1, "Limit+1").unwrap();
    e.set_formula(SHEET, 2, 1, "ROUND(Tau,1)").unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 1), n(11.0));
    assert_eq!(e.get(SHEET, 2, 1), n(6.3));
}

#[test]
fn self_referential_names_do_not_hang() {
    let mut e = engine();
    e.define_name("Loop", "Loop+1").unwrap();
    e.set_formula(SHEET, 1, 1, "Loop").unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 1), err(Name));
}

#[test]
fn volatile_formulas_recalculate_every_pass() {
    let mut e = Engine::with_clock(Box::new(FixedClock(1_700_000_000.0)));
    e.set_formula(SHEET, 1, 1, "TODAY()").unwrap();
    e.set_formula(SHEET, 2, 1, "A1+1").unwrap();
    e.set_formula(SHEET, 3, 1, "5").unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 1), n(45244.0));
    assert_eq!(e.get(SHEET, 2, 1), n(45245.0));
    e.set_clock(Box::new(FixedClock(1_700_000_000.0 + 86_400.0)));
    let recomputed = addrs(e.recalc_dirty());
    assert_eq!(
        recomputed,
        HashSet::from([at(SHEET, 1, 1), at(SHEET, 2, 1)]),
        "the non-volatile constant formula is left alone"
    );
    assert_eq!(e.get(SHEET, 1, 1), n(45245.0));
    assert_eq!(e.get(SHEET, 2, 1), n(45246.0));
}

#[test]
fn switching_date_system_recalculates_everything() {
    let mut e = engine();
    e.set_formula(SHEET, 1, 1, "DATE(2024,1,1)").unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 1), n(45292.0));
    e.set_date1904(true);
    assert!(e.date1904());
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 1), n(45292.0 - 1462.0));
}

#[test]
fn array_results_do_not_spill() {
    let mut e = engine();
    e.set_formula(SHEET, 1, 1, "{1,2;3,4}").unwrap();
    e.set_formula(SHEET, 1, 5, "A1+10").unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 1), n(1.0));
    assert_eq!(e.get(SHEET, 1, 2), Value::Empty);
    assert_eq!(
        e.get_full(SHEET, 1, 1),
        Value::Array(vec![vec![n(1.0), n(2.0)], vec![n(3.0), n(4.0)]])
    );
    assert_eq!(
        e.get(SHEET, 1, 5),
        n(11.0),
        "dependents read the top-left element"
    );
}

#[test]
fn recalc_all_recomputes_everything() {
    let mut e = engine();
    e.set_formula(SHEET, 1, 1, "1+1").unwrap();
    e.set_formula(SHEET, 2, 1, "A1+1").unwrap();
    e.recalc_dirty();
    assert_eq!(e.recalc_all().len(), 2);
}

#[test]
fn error_values_flow_through_dependents() {
    let mut e = engine();
    e.set_formula(SHEET, 1, 1, "1/0").unwrap();
    e.set_formula(SHEET, 2, 1, "A1+1").unwrap();
    e.set_formula(SHEET, 3, 1, "IFERROR(A2,-1)").unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 1), err(DivZero));
    assert_eq!(e.get(SHEET, 2, 1), err(DivZero));
    assert_eq!(e.get(SHEET, 3, 1), n(-1.0));
}

#[test]
fn long_dependency_chain_recalculates_without_deep_recursion() {
    let mut e = engine();
    e.set_value(SHEET, 1, 1, n(1.0));
    for row in 2..=5000 {
        e.set_formula(SHEET, row, 1, &format!("A{}+1", row - 1))
            .unwrap();
    }
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 5000, 1), n(5000.0));
    e.set_value(SHEET, 1, 1, n(101.0));
    assert_eq!(e.recalc_dirty().len(), 4999);
    assert_eq!(e.get(SHEET, 5000, 1), n(5100.0));
}

#[test]
fn long_flat_formula_evaluates_within_a_test_thread_stack() {
    let mut e = engine();
    let formula = format!("1{}", "+1".repeat(900));
    e.set_formula(SHEET, 1, 1, &formula).unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 1), n(901.0));
}

#[test]
fn whole_column_sum_does_not_expand_into_cell_entries() {
    let mut e = engine();
    for row in 1..=100_000u32 {
        e.set_value(SHEET, row, 1, n(f64::from(row)));
    }
    e.set_formula(SHEET, 1, 2, "SUM(A:A)").unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 2), n(5_000_050_000.0));

    let graph = e.graph();
    assert_eq!(graph.formula_count(), 1);
    assert_eq!(graph.area_count(), 1);
    assert!(
        graph.index_entry_count() <= 1,
        "A:A must be one index entry, got {}",
        graph.index_entry_count()
    );

    // Editing a cell far down the column recalculates exactly the one dependent.
    e.set_value(SHEET, 99_999, 1, n(0.0));
    let recomputed = addrs(e.recalc_dirty());
    assert_eq!(recomputed, HashSet::from([at(SHEET, 1, 2)]));
    assert_eq!(e.get(SHEET, 1, 2), n(5_000_050_000.0 - 99_999.0));

    // Cells outside the column do not trigger it.
    e.set_value(SHEET, 5, 3, n(1.0));
    assert!(e.recalc_dirty().is_empty());
}

#[test]
fn many_formulas_over_one_column_share_one_area() {
    let mut e = engine();
    for row in 1..=50u32 {
        e.set_value(SHEET, row, 1, n(1.0));
    }
    for row in 1..=500u32 {
        e.set_formula(SHEET, row, 2, "SUM(A:A)").unwrap();
    }
    e.recalc_dirty();
    assert_eq!(e.graph().area_count(), 1);
    assert_eq!(e.get(SHEET, 500, 2), n(50.0));
    e.set_value(SHEET, 7, 1, n(11.0));
    assert_eq!(e.recalc_dirty().len(), 500);
    assert_eq!(e.get(SHEET, 1, 2), n(60.0));
}

#[test]
fn lookups_over_whole_columns_use_the_used_area() {
    let mut e = engine();
    for row in 1..=1000u32 {
        e.set_value(SHEET, row, 1, n(f64::from(row)));
        e.set_value(SHEET, row, 2, n(f64::from(row) * 2.0));
    }
    e.set_formula(SHEET, 1, 4, "VLOOKUP(777,A:B,2,FALSE)")
        .unwrap();
    e.set_formula(SHEET, 2, 4, "MATCH(500,A:A,0)").unwrap();
    e.set_formula(SHEET, 3, 4, "COUNTIF(A:A,\">990\")").unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 4), n(1554.0));
    assert_eq!(e.get(SHEET, 2, 4), n(500.0));
    assert_eq!(e.get(SHEET, 3, 4), n(10.0));
}

#[test]
fn indirect_follows_its_target_after_set_value() {
    let mut e = engine();
    e.set_value(SHEET, 1, 1, n(1.0));
    e.set_value(SHEET, 1, 2, t("A1"));
    e.set_formula(SHEET, 1, 3, "=INDIRECT(B1)*10").unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 3), n(10.0));
    // The referenced cell changes: no static reference connects it to C1.
    e.set_value(SHEET, 1, 1, n(7.0));
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 3), n(70.0));
    // The text that names the target changes.
    e.set_value(SHEET, 2, 1, n(3.0));
    e.set_value(SHEET, 1, 2, t("A2"));
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 3), n(30.0));
}

#[test]
fn offset_follows_its_target_after_set_value() {
    let mut e = engine();
    e.set_value(SHEET, 1, 1, n(1.0));
    e.set_value(SHEET, 2, 1, n(2.0));
    e.set_value(SHEET, 3, 1, n(3.0));
    e.set_value(SHEET, 1, 3, n(1.0)); // C1: how far to move
    e.set_formula(SHEET, 1, 2, "=OFFSET(A1,C1,0)+SUM(OFFSET(A1,0,0,C1+1,1))")
        .unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 2), n(5.0)); // A2 + (A1 + A2)
    e.set_value(SHEET, 2, 1, n(20.0));
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 2), n(41.0));
    e.set_value(SHEET, 1, 3, n(2.0));
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 2), n(3.0 + 24.0));
}

#[test]
fn indirect_reads_a_formula_that_is_calculated_later() {
    let mut e = engine();
    // C1 is placed before the formula it reads in plan order only by accident of insertion: either way the
    // result must be the settled value of A2.
    e.set_formula(SHEET, 1, 3, "=INDIRECT(\"A2\")+1").unwrap();
    e.set_formula(SHEET, 2, 1, "=A1*2").unwrap();
    e.set_value(SHEET, 1, 1, n(4.0));
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 2, 1), n(8.0));
    assert_eq!(e.get(SHEET, 1, 3), n(9.0));
    e.set_value(SHEET, 1, 1, n(10.0));
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 3), n(21.0));
}

#[test]
fn chains_of_indirect_and_offset_settle() {
    let mut e = engine();
    // D1 reads C1 through INDIRECT, C1 reads B1 through OFFSET, B1 reads A1 through INDIRECT, and a plain
    // formula depends on D1. Insert them in the worst order for a single pass.
    e.set_formula(SHEET, 1, 5, "=D1+1").unwrap();
    e.set_formula(SHEET, 1, 4, "=INDIRECT(\"C1\")+1").unwrap();
    e.set_formula(SHEET, 1, 3, "=OFFSET(A1,0,1)+1").unwrap();
    e.set_formula(SHEET, 1, 2, "=INDIRECT(\"A\"&1)+1").unwrap();
    e.set_value(SHEET, 1, 1, n(1.0));
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 5), n(5.0));
    e.set_value(SHEET, 1, 1, n(100.0));
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 2), n(101.0));
    assert_eq!(e.get(SHEET, 1, 3), n(102.0));
    assert_eq!(e.get(SHEET, 1, 4), n(103.0));
    assert_eq!(e.get(SHEET, 1, 5), n(104.0));
}

#[test]
fn indirect_to_a_missing_sheet_recovers_when_the_sheet_appears() {
    let mut e = engine();
    e.set_formula(SHEET, 1, 1, "=INDIRECT(\"Later!A1\")")
        .unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 1), err(Ref));
    e.set_value("Later", 1, 1, n(9.0));
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 1), n(9.0));
}

#[test]
fn position_functions_do_not_make_a_cycle() {
    let mut e = engine();
    e.set_formula(SHEET, 3, 2, "=ROW(B3)*100+COLUMN(B3)+ROWS(A1:C3)")
        .unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 3, 2), n(305.0));
    assert!(!e.is_circular(SHEET, 3, 2));
}

#[test]
fn isformula_follows_edits() {
    let mut e = engine();
    e.set_value(SHEET, 1, 1, n(1.0));
    e.set_formula(SHEET, 1, 2, "=ISFORMULA(A1)").unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 2), Value::Bool(false));
    e.set_formula(SHEET, 1, 1, "=1+1").unwrap();
    e.recalc_dirty();
    assert_eq!(e.get(SHEET, 1, 2), Value::Bool(true));
}
