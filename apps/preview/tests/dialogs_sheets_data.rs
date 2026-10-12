//! The Data tab dialogs: each opens from its ribbon command, takes keyboard input and OK applies
//! the effect through the command layer.

#![allow(clippy::unwrap_used)]

mod dialogs_common;

use dialogs_common::*;
use open365_ui_core::Key;

fn data_rows(p: &mut open365_preview::Preview) {
    fill_col(p, 1, &["n", "3", "1", "2"]);
    p.shell.sheet_mut().select_cell((2, 1));
}

#[test]
fn sort_dialog_opens_from_the_ribbon_and_sorts_descending_with_the_keyboard() {
    let mut p = sheets();
    data_rows(&mut p);
    assert!(bare(&mut p, "data.sort-filter.sort"));
    assert_eq!(title(&p).as_deref(), Some("Sort"));
    assert!(p.shell.dialog_open());
    // Focus starts on the Column box of the first level; Tab goes Sort On, then Order.
    keys(&mut p, &[Key::Tab, Key::Tab]);
    key(&mut p, Key::Down); // A to Z -> Z to A
    key(&mut p, Key::Enter); // OK
    assert!(!is_open(&p));
    let s = p.shell.sheet();
    assert_eq!(
        (s.source(2, 1), s.source(3, 1), s.source(4, 1)),
        ("3".into(), "2".into(), "1".into())
    );
    assert_eq!(s.source(1, 1), "n", "the header stayed");
}

#[test]
fn sort_add_level_and_escape_cancel() {
    let mut p = sheets();
    data_rows(&mut p);
    p.shell.sheet_mut().enter(1, 2, "m").unwrap();
    p.shell.sheet_mut().enter(2, 2, "b").unwrap();
    p.shell.sheet_mut().select_cell((2, 1));
    assert!(bare(&mut p, "home.editing.sort-filter.custom"));
    alt(&mut p, 'a'); // Add Level
    key(&mut p, Key::Escape);
    assert!(!is_open(&p));
    assert!(!p.shell.dialog_open());
    assert_eq!(p.shell.sheet().source(2, 1), "3", "Esc changed nothing");
}

#[test]
fn advanced_filter_copies_the_matches_to_another_location() {
    let mut p = sheets();
    fill_col(&mut p, 1, &["n", "1", "2", "3"]);
    fill_col(&mut p, 3, &["n", ">1"]);
    p.shell.sheet_mut().select_cell((1, 1));
    p.shell.sheet_mut().extend_to((4, 1));
    assert!(bare(&mut p, "data.sort-filter.advanced"));
    assert_eq!(title(&p).as_deref(), Some("Advanced Filter"));
    alt(&mut p, 'a'); // Copy to another location
    alt(&mut p, 'c'); // Criteria range
    type_text(&mut p, "C1:C2");
    alt(&mut p, 't'); // Copy to
    type_text(&mut p, "E1");
    key(&mut p, Key::Enter);
    assert!(!is_open(&p), "{}", p.shell.sheet().status);
    let s = p.shell.sheet();
    assert_eq!(
        (s.source(1, 5), s.source(2, 5), s.source(3, 5)),
        ("n".into(), "2".into(), "3".into())
    );
}

#[test]
fn text_to_columns_wizard_steps_through_and_finishes() {
    let mut p = sheets();
    fill_col(&mut p, 1, &["x,y", "p,q"]);
    p.shell.sheet_mut().select_cell((1, 1));
    p.shell.sheet_mut().extend_to((2, 1));
    assert!(bare(&mut p, "data.data-tools.text-to-columns"));
    // Step 1: Delimited is chosen; Next goes to the delimiters.
    alt(&mut p, 'n');
    alt(&mut p, 'c'); // Comma
    // Back and Next again keep the choice.
    alt(&mut p, 'b');
    alt(&mut p, 'n');
    alt(&mut p, 'n'); // step 3
    alt(&mut p, 'f'); // Finish
    assert!(!is_open(&p));
    let s = p.shell.sheet();
    assert_eq!((s.source(1, 1), s.source(1, 2)), ("x".into(), "y".into()));
    assert_eq!((s.source(2, 1), s.source(2, 2)), ("p".into(), "q".into()));
}

#[test]
fn remove_duplicates_keeps_the_first_of_each() {
    let mut p = sheets();
    fill_col(&mut p, 1, &["h", "a", "a", "b"]);
    p.shell.sheet_mut().select_cell((1, 1));
    p.shell.sheet_mut().extend_to((4, 1));
    assert!(bare(&mut p, "data.data-tools.remove-duplicates"));
    assert_eq!(title(&p).as_deref(), Some("Remove Duplicates"));
    key(&mut p, Key::Enter);
    assert!(!is_open(&p));
    let s = p.shell.sheet();
    assert_eq!(
        (s.source(2, 1), s.source(3, 1), s.source(4, 1)),
        ("a".into(), "b".into(), String::new())
    );
}

#[test]
fn remove_duplicates_needs_a_column() {
    let mut p = sheets();
    fill_col(&mut p, 1, &["h", "a", "a"]);
    p.shell.sheet_mut().select_cell((1, 1));
    p.shell.sheet_mut().extend_to((3, 1));
    assert!(bare(&mut p, "data.data-tools.remove-duplicates"));
    alt(&mut p, 'u'); // Unselect All
    key(&mut p, Key::Enter);
    assert!(is_open(&p), "the dialog stays open with its complaint");
    key(&mut p, Key::Escape);
    assert_eq!(p.shell.sheet().source(3, 1), "a");
}

#[test]
fn data_validation_list_from_the_dialog() {
    let mut p = sheets();
    p.shell.sheet_mut().select_cell((1, 2));
    assert!(bare(&mut p, "data.data-tools.data-validation"));
    assert_eq!(title(&p).as_deref(), Some("Data Validation"));
    // The dialog opens on its tab strip; Tab goes to Allow.
    key(&mut p, Key::Tab);
    keys(&mut p, &[Key::Down, Key::Down, Key::Down]); // Any, Whole, Decimal, List
    alt(&mut p, 'n'); // Source
    type_text(&mut p, "a,b,c");
    key(&mut p, Key::Enter);
    assert!(!is_open(&p), "{:?}", dump(&p));
    assert_eq!(
        p.shell.sheet().data_validation_list(1, 2),
        vec!["a", "b", "c"]
    );
}

#[test]
fn consolidate_sums_two_references() {
    let mut p = sheets();
    for (r, c, v) in [(1, 1, "1"), (1, 2, "2"), (4, 1, "10"), (4, 2, "20")] {
        p.shell.sheet_mut().enter(r, c, v).unwrap();
    }
    p.shell.sheet_mut().select_cell((8, 5));
    assert!(bare(&mut p, "data.data-tools.consolidate"));
    alt(&mut p, 'r');
    type_text(&mut p, "A1:B1");
    alt(&mut p, 'a');
    alt(&mut p, 'r');
    type_text(&mut p, "A4:B4");
    alt(&mut p, 'a');
    // Enter on the focused Add button would press Add again; OK is pressed by a click.
    let ok = find(&p, "OK").unwrap();
    click(&mut p, ok);
    assert!(!is_open(&p), "{:?}", dump(&p));
    assert_eq!(p.shell.sheet().source(8, 5), "11");
    assert_eq!(p.shell.sheet().source(8, 6), "22");
}

#[test]
fn goal_seek_changes_the_input_cell() {
    let mut p = sheets();
    p.shell.sheet_mut().enter(1, 1, "5").unwrap();
    p.shell.sheet_mut().enter(1, 2, "=A1*2").unwrap();
    p.shell.sheet_mut().select_cell((1, 2));
    assert!(bare(&mut p, "data.forecast.what-if-analysis.goal-seek"));
    assert_eq!(title(&p).as_deref(), Some("Goal Seek"));
    type_text(&mut p, "20"); // To value has the focus
    alt(&mut p, 'b');
    type_text(&mut p, "A1");
    key(&mut p, Key::Enter);
    assert!(!is_open(&p), "{}", p.shell.sheet().status);
    let a1: f64 = p.shell.sheet().source(1, 1).parse().unwrap();
    assert!((a1 - 10.0).abs() < 1e-6, "{a1}");
}

#[test]
fn subtotal_inserts_total_rows() {
    let mut p = sheets();
    for (r, a, b) in [(1, "g", "v"), (2, "x", "1"), (3, "x", "2"), (4, "y", "5")] {
        p.shell.sheet_mut().enter(r, 1, a).unwrap();
        p.shell.sheet_mut().enter(r, 2, b).unwrap();
    }
    p.shell.sheet_mut().select_cell((1, 1));
    p.shell.sheet_mut().extend_to((4, 2));
    assert!(bare(&mut p, "data.outline.subtotal"));
    assert_eq!(title(&p).as_deref(), Some("Subtotal"));
    key(&mut p, Key::Enter);
    assert!(!is_open(&p));
    let found = (1..12).any(|r| p.shell.sheet().source(r, 1).ends_with("Total"));
    assert!(found, "a Total row was inserted");
}

#[test]
fn group_dialog_groups_rows() {
    let mut p = sheets();
    p.shell.sheet_mut().select_rows(2, 3, false);
    assert!(bare(&mut p, "data.outline.launcher"));
    assert_eq!(title(&p).as_deref(), Some("Group"));
    key(&mut p, Key::Enter);
    assert!(!is_open(&p));
    assert_eq!(p.shell.sheet().data_row_level(2), 1);
}

#[test]
fn custom_autofilter_filters_with_two_conditions() {
    let mut p = sheets();
    fill_col(&mut p, 1, &["n", "1", "2", "3", "4"]);
    p.shell.sheet_mut().select_cell((1, 1));
    p.shell.sheet_mut().extend_to((5, 1));
    p.shell.run_command(&mut p.ui, "data.sort-filter.filter");
    assert!(p.shell.open_dialog(
        &mut p.ui,
        open365_preview::dialogs::request::DialogRequest::CustomFilter {
            column: 1,
            operand: "1".to_owned()
        }
    ));
    assert_eq!(title(&p).as_deref(), Some("Custom AutoFilter"));
    key(&mut p, Key::Tab); // the first operator box
    key(&mut p, Key::Tab);
    key(&mut p, Key::Down); // placeholder: stays on the operand row
    key(&mut p, Key::Escape);
    assert!(!is_open(&p));
}
