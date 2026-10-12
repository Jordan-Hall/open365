//! Sheets Data tab: every wired control runs through `Shell::run_command` and changes the sheet.

#![allow(clippy::unwrap_used)]

use open365_commands::RangeRef;
use open365_commands::data_ops::{
    AllowKind, ColumnFilter, DataCmp, ErrorAlert, SortHeader, SortLevel, SortRequest,
    ValidationRule,
};
use open365_preview::ribbon::Handled;
use open365_preview::sheet::SheetModel;
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

fn sheet(p: &mut Preview) -> &mut SheetModel {
    p.shell.sheet_mut()
}

fn fill(p: &mut Preview, rows: &[&[&str]]) {
    for (r, line) in rows.iter().enumerate() {
        for (c, t) in line.iter().enumerate() {
            sheet(p).enter(r as u32 + 1, c as u32 + 1, t).unwrap();
        }
    }
}

fn col(p: &mut Preview, c: u32, rows: u32) -> Vec<String> {
    (1..=rows).map(|r| sheet(p).source(r, c)).collect()
}

fn hidden(p: &mut Preview, row: u32) -> bool {
    sheet(p).home_row_state(row).1
}

#[test]
fn sort_a_to_z_keeps_the_header_and_orders_like_excel() {
    let mut p = rig();
    fill(
        &mut p,
        &[
            &["Name", "Qty"],
            &["banana", "3"],
            &["Apple", "10"],
            &["10", "1"],
            &["cherry", "2"],
            &["TRUE", "5"],
            &["", "9"],
        ],
    );
    sheet(&mut p).select_cell((2, 1));
    run(&mut p, "data.sort-filter.sort-a-to-z");
    // Numbers, then text (case-insensitive), then logical values; blanks last; the header stays.
    assert_eq!(
        col(&mut p, 1, 7),
        ["Name", "10", "Apple", "banana", "cherry", "TRUE", ""]
    );
    assert_eq!(col(&mut p, 2, 7), ["Qty", "1", "10", "3", "2", "5", "9"]);
    assert!(sheet(&mut p).undo());
    assert_eq!(sheet(&mut p).source(2, 1), "banana");
    run(&mut p, "data.sort-filter.sort-z-to-a");
    // Descending is the exact reverse, blanks still last.
    assert_eq!(
        col(&mut p, 1, 7),
        ["Name", "TRUE", "cherry", "banana", "Apple", "10", ""]
    );
}

#[test]
fn sort_dialog_sorts_by_several_levels() {
    let mut p = rig();
    fill(
        &mut p,
        &[
            &["Region", "Sales"],
            &["East", "5"],
            &["West", "7"],
            &["East", "9"],
            &["West", "1"],
        ],
    );
    sheet(&mut p).select_cell((1, 1));
    run(&mut p, "data.sort-filter.sort=A,B:desc;header=yes");
    assert_eq!(
        col(&mut p, 1, 5),
        ["Region", "East", "East", "West", "West"]
    );
    assert_eq!(col(&mut p, 2, 5), ["Sales", "9", "5", "7", "1"]);
    // The bare button asks the shell for the Sort dialog.
    run(&mut p, "data.sort-filter.sort");
    cancels(&mut p, "Sort");
    // A custom list puts its items first.
    sheet(&mut p)
        .data_sort(SortRequest {
            range: RangeRef {
                first_row: 1,
                first_col: 1,
                last_row: 5,
                last_col: 2,
            },
            header: SortHeader::Yes,
            left_to_right: false,
            case_sensitive: false,
            levels: vec![SortLevel {
                index: 1,
                descending: false,
                custom_list: Some(vec!["West".to_owned(), "East".to_owned()]),
            }],
        })
        .unwrap();
    assert_eq!(
        col(&mut p, 1, 5),
        ["Region", "West", "West", "East", "East"]
    );
}

#[test]
fn sort_moves_formulas_with_their_row() {
    let mut p = rig();
    fill(
        &mut p,
        &[
            &["n", "double"],
            &["3", "=A2*2"],
            &["1", "=A3*2"],
            &["2", "=A4*2"],
        ],
    );
    sheet(&mut p).select_cell((2, 1));
    run(&mut p, "data.sort-filter.sort-a-to-z");
    assert_eq!(col(&mut p, 1, 4), ["n", "1", "2", "3"]);
    assert_eq!(col(&mut p, 2, 4), ["double", "=A2*2", "=A3*2", "=A4*2"]);
    assert_eq!(sheet(&mut p).display(2, 2), "2");
    assert_eq!(sheet(&mut p).display(4, 2), "6");
}

#[test]
fn filter_arrows_value_filters_clear_and_reapply() {
    let mut p = rig();
    fill(
        &mut p,
        &[
            &["Fruit", "Qty"],
            &["apple", "1"],
            &["pear", "20"],
            &["apple", "3"],
            &["fig", "40"],
        ],
    );
    sheet(&mut p).select_cell((1, 1));
    run(&mut p, "data.sort-filter.filter");
    assert_eq!(sheet(&mut p).data_filter_range(), Some((1, 1, 5, 2)));
    assert_eq!(
        sheet(&mut p).data_filter_items(1),
        [
            ("apple".to_owned(), true),
            ("fig".to_owned(), true),
            ("pear".to_owned(), true)
        ]
    );
    run(&mut p, "data.sort-filter.filter=A:values=apple");
    assert!(!hidden(&mut p, 2) && hidden(&mut p, 3) && !hidden(&mut p, 4) && hidden(&mut p, 5));
    assert_eq!(sheet(&mut p).data_filtered_columns(), [1]);
    assert!(
        sheet(&mut p)
            .data_filter_items(1)
            .contains(&("pear".to_owned(), false))
    );
    // A second column's filter combines with AND.
    run(&mut p, "data.sort-filter.filter=B:gt=2");
    assert!(hidden(&mut p, 2) && hidden(&mut p, 3) && !hidden(&mut p, 4));
    // New data does not refilter until Reapply.
    sheet(&mut p).enter(2, 2, "50").unwrap();
    assert!(hidden(&mut p, 2));
    run(&mut p, "data.sort-filter.reapply");
    assert!(!hidden(&mut p, 2));
    run(&mut p, "data.sort-filter.clear");
    assert!((2..=5).all(|r| !hidden(&mut p, r)));
    assert!(sheet(&mut p).data_filter_range().is_some());
    run(&mut p, "data.sort-filter.filter=B:top=1");
    assert!(!hidden(&mut p, 2) && hidden(&mut p, 3));
    run(&mut p, "data.sort-filter.filter");
    assert_eq!(sheet(&mut p).data_filter_range(), None);
    assert!((2..=5).all(|r| !hidden(&mut p, r)));
    // Filtering through the typed API.
    run(&mut p, "data.sort-filter.filter");
    sheet(&mut p)
        .data_filter_column(2, Some(ColumnFilter::AboveAverage))
        .unwrap();
    assert!(hidden(&mut p, 4));
}

#[test]
fn advanced_filter_in_place_and_copy_with_unique_records() {
    let mut p = rig();
    fill(
        &mut p,
        &[
            &["Fruit", "Qty", "", "Fruit", "Qty"],
            &["apple", "1", "", "apple", ""],
            &["pear", "20", "", "", ">10"],
            &["apple", "3"],
            &["apple", "3"],
            &["fig", "40"],
        ],
    );
    run(
        &mut p,
        "data.sort-filter.advanced=list=A1:B6;criteria=D1:E3",
    );
    // Rows match when they satisfy any criteria row: apple, or Qty > 10.
    assert!(!hidden(&mut p, 2) && !hidden(&mut p, 3) && !hidden(&mut p, 4));
    assert!(!hidden(&mut p, 6));
    run(
        &mut p,
        "data.sort-filter.advanced=list=A1:B6;criteria=D1:E2;copy=G1;unique",
    );
    assert_eq!(sheet(&mut p).source(1, 7), "Fruit");
    assert_eq!(sheet(&mut p).source(2, 7), "apple");
    assert_eq!(sheet(&mut p).source(2, 8), "1");
    assert_eq!(sheet(&mut p).source(3, 8), "3");
    assert_eq!(
        sheet(&mut p).source(4, 7),
        "",
        "the duplicate record is copied once"
    );
    run(&mut p, "data.sort-filter.advanced");
    cancels(&mut p, "Advanced Filter");
}

#[test]
fn text_to_columns_splits_delimited_and_fixed_text() {
    let mut p = rig();
    fill(&mut p, &[&["a,1,\"x,y\""], &["b,2,z"]]);
    sheet(&mut p).select_cell((1, 1));
    sheet(&mut p).extend_to((2, 1));
    run(&mut p, "data.data-tools.text-to-columns=delim=comma");
    assert_eq!(sheet(&mut p).source(1, 1), "a");
    assert_eq!(sheet(&mut p).source(1, 2), "1");
    assert_eq!(
        sheet(&mut p).source(1, 3),
        "x,y",
        "the qualifier keeps the comma"
    );
    assert_eq!(sheet(&mut p).source(2, 3), "z");
    assert_eq!(sheet(&mut p).display(1, 2), "1");
    assert!(sheet(&mut p).undo());
    assert_eq!(sheet(&mut p).source(1, 1), "a,1,\"x,y\"");
    assert_eq!(sheet(&mut p).source(1, 2), "");
    // Fixed width, to another destination; consecutive delimiters as one.
    let mut p = rig();
    fill(&mut p, &[&["abcdef"], &["ghijkl"]]);
    sheet(&mut p).select_cell((1, 1));
    sheet(&mut p).extend_to((2, 1));
    run(&mut p, "data.data-tools.text-to-columns=fixed=2,4;dest=C1");
    assert_eq!(col(&mut p, 3, 2), ["ab", "gh"]);
    assert_eq!(col(&mut p, 5, 2), ["ef", "kl"]);
    let mut p = rig();
    fill(&mut p, &[&["a  b   c"]]);
    sheet(&mut p).select_cell((1, 1));
    run(
        &mut p,
        "data.data-tools.text-to-columns=delim=space;consecutive",
    );
    assert_eq!(sheet(&mut p).source(1, 3), "c");
    run(&mut p, "data.data-tools.text-to-columns");
    cancels(&mut p, "Convert Text to Columns Wizard");
}

#[test]
fn remove_duplicates_keeps_the_first_of_each() {
    let mut p = rig();
    fill(
        &mut p,
        &[
            &["k", "v"],
            &["a", "1"],
            &["b", "2"],
            &["A", "3"],
            &["b", "2"],
            &["c", "4"],
        ],
    );
    sheet(&mut p).select_cell((1, 1));
    run(
        &mut p,
        "data.data-tools.remove-duplicates=headers=yes;cols=A",
    );
    assert_eq!(col(&mut p, 1, 6), ["k", "a", "b", "c", "", ""]);
    assert_eq!(col(&mut p, 2, 6), ["v", "1", "2", "4", "", ""]);
    assert!(sheet(&mut p).status.contains("found and removed"));
    assert!(sheet(&mut p).undo());
    assert_eq!(col(&mut p, 1, 6), ["k", "a", "b", "A", "b", "c"]);
    // Compared on both columns, only the repeated b/2 row goes.
    run(&mut p, "data.data-tools.remove-duplicates=headers=yes");
    assert_eq!(col(&mut p, 1, 6), ["k", "a", "b", "A", "c", ""]);
}

fn list_rule(items: &str) -> ValidationRule {
    ValidationRule {
        allow: AllowKind::List,
        compare: DataCmp::Between,
        value1: items.to_owned(),
        value2: None,
        ignore_blank: true,
        in_cell_dropdown: true,
        input_message: None,
        error_alert: Some(ErrorAlert {
            style: "stop".to_owned(),
            title: "Not allowed".to_owned(),
            message: "Pick from the list.".to_owned(),
        }),
    }
}

#[test]
fn data_validation_checks_entries_and_offers_the_list() {
    let mut p = rig();
    sheet(&mut p).select_cell((1, 1));
    sheet(&mut p).extend_to((3, 1));
    sheet(&mut p)
        .data_set_validation(Some(list_rule("red,green,blue")))
        .unwrap();
    assert_eq!(
        sheet(&mut p).data_validation_list(2, 1),
        ["red", "green", "blue"]
    );
    assert!(sheet(&mut p).data_validation_at(4, 1).is_none());
    assert!(sheet(&mut p).data_check_entry(1, 1, "green").is_ok());
    let rejected = sheet(&mut p).data_check_entry(1, 1, "pink").unwrap_err();
    assert_eq!(
        (rejected.style, rejected.title.as_str()),
        ("stop", "Not allowed")
    );
    assert!(sheet(&mut p).data_enter(1, 1, "pink").is_err());
    assert_eq!(sheet(&mut p).source(1, 1), "");
    sheet(&mut p).data_enter(1, 1, "blue").unwrap();
    assert_eq!(sheet(&mut p).source(1, 1), "blue");
    // Numbers: whole number between 1 and 10 on a sub-range replaces the rule there.
    sheet(&mut p).select_cell((2, 1));
    run(&mut p, "data.data-tools.data-validation=whole=between,1,10");
    assert!(sheet(&mut p).data_check_entry(2, 1, "5").is_ok());
    assert!(sheet(&mut p).data_check_entry(2, 1, "11").is_err());
    assert!(sheet(&mut p).data_check_entry(2, 1, "5.5").is_err());
    assert!(
        sheet(&mut p).data_check_entry(1, 1, "pink").is_err(),
        "the rest keeps the list"
    );
    assert!(sheet(&mut p).data_check_entry(3, 1, "green").is_ok());
    // Clear All on the whole range.
    sheet(&mut p).select_cell((1, 1));
    sheet(&mut p).extend_to((3, 1));
    run(&mut p, "data.data-tools.data-validation=clear");
    assert!(sheet(&mut p).data_validation_at(1, 1).is_none());
    assert!(sheet(&mut p).data_check_entry(2, 1, "anything").is_ok());
    assert!(sheet(&mut p).undo());
    assert!(sheet(&mut p).data_validation_at(1, 1).is_some());
    run(&mut p, "data.data-tools.data-validation");
    cancels(&mut p, "Data Validation");
}

#[test]
fn consolidate_sums_by_position_and_by_label() {
    let mut p = rig();
    fill(
        &mut p,
        &[
            &["", "Q1", "Q2", "", "", "", "", ""],
            &["x", "1", "2"],
            &["y", "3", "4"],
        ],
    );
    // A second block, rows in another order and with an extra label.
    for (r, line) in [["y", "10", "20"], ["x", "30", "40"], ["z", "50", "60"]]
        .iter()
        .enumerate()
    {
        for (c, t) in line.iter().enumerate() {
            sheet(&mut p).enter(r as u32 + 6, c as u32 + 1, t).unwrap();
        }
    }
    sheet(&mut p).enter(5, 2, "Q1").unwrap();
    sheet(&mut p).enter(5, 3, "Q2").unwrap();
    // By position: 1+10, 2+20 / 3+30, 4+40.
    run(
        &mut p,
        "data.data-tools.consolidate=fn=sum;ref=B2:C3;ref=B6:C7;dest=F1",
    );
    assert_eq!(col(&mut p, 6, 2), ["11", "33"]);
    assert_eq!(col(&mut p, 7, 2), ["22", "44"]);
    // By labels in the left column and top row.
    run(
        &mut p,
        "data.data-tools.consolidate=fn=sum;ref=A1:C3;ref=A5:C8;top;left;dest=F10",
    );
    assert_eq!(sheet(&mut p).source(10, 7), "Q1");
    assert_eq!(
        col(&mut p, 6, 13)[10..],
        ["x", "y", "z"].map(String::from)[..]
    );
    assert_eq!(sheet(&mut p).source(11, 7), "31");
    assert_eq!(sheet(&mut p).source(12, 8), "24");
    assert_eq!(sheet(&mut p).source(13, 8), "60");
    run(
        &mut p,
        "data.data-tools.consolidate=fn=average;ref=B2:C3;ref=B6:C7;dest=J1",
    );
    assert_eq!(sheet(&mut p).source(1, 10), "5.5");
}

#[test]
fn goal_seek_solves_for_the_changing_cell() {
    let mut p = rig();
    fill(&mut p, &[&["2", "=A1*3+1"]]);
    run(&mut p, "data.forecast.what-if-analysis=goalseek=B1,10,A1");
    let a1: f64 = sheet(&mut p).source(1, 1).parse().unwrap();
    assert!((a1 - 3.0).abs() < 1e-6, "{a1}");
    assert!(sheet(&mut p).status.contains("found a solution"));
    assert!(sheet(&mut p).undo());
    assert_eq!(sheet(&mut p).source(1, 1), "2");
    // A target the formula can never reach reports failure and changes nothing.
    sheet(&mut p).enter(1, 2, "=0*A1+5").unwrap();
    run(&mut p, "data.forecast.what-if-analysis=goalseek=B1,10,A1");
    assert_eq!(sheet(&mut p).source(1, 1), "2");
    run(&mut p, "data.forecast.what-if-analysis");
    cancels(&mut p, "Goal Seek");
}

#[test]
fn group_ungroup_and_hide_show_detail() {
    let mut p = rig();
    fill(&mut p, &[&["h"], &["1"], &["2"], &["3"], &["total"]]);
    sheet(&mut p).select_cell((2, 1));
    sheet(&mut p).extend_to((4, 1));
    run(&mut p, "data.outline.group");
    assert_eq!(
        (1..=5)
            .map(|r| sheet(&mut p).data_row_level(r))
            .collect::<Vec<_>>(),
        [0, 1, 1, 1, 0]
    );
    // Grouping again nests; ungroup unwinds one level at a time.
    run(&mut p, "data.outline.group");
    assert_eq!(sheet(&mut p).data_row_level(2), 2);
    run(&mut p, "data.outline.ungroup");
    assert_eq!(sheet(&mut p).data_row_level(2), 1);
    sheet(&mut p).select_cell((5, 1));
    run(&mut p, "data.outline.hide-detail");
    assert!(hidden(&mut p, 2) && hidden(&mut p, 3) && hidden(&mut p, 4) && !hidden(&mut p, 5));
    run(&mut p, "data.outline.show-detail");
    assert!(!hidden(&mut p, 2) && !hidden(&mut p, 4));
    sheet(&mut p).select_cell((2, 1));
    sheet(&mut p).extend_to((4, 1));
    run(&mut p, "data.outline.ungroup");
    assert_eq!(sheet(&mut p).data_row_level(3), 0);
    assert!(sheet(&mut p).undo());
    assert_eq!(sheet(&mut p).data_row_level(3), 1);
    // The grouping survives a save and reopen.
    let path = std::env::temp_dir().join("open365-data-group.xlsx");
    sheet(&mut p).save(&path).unwrap();
    let reopened = SheetModel::open(&path).unwrap();
    assert_eq!(reopened.data_row_level(3), 1);
    let _ = std::fs::remove_file(path);
}

#[test]
fn subtotal_inserts_rows_with_subtotal_formulas_and_removes_them() {
    let mut p = rig();
    fill(
        &mut p,
        &[
            &["Region", "Sales"],
            &["East", "5"],
            &["East", "7"],
            &["West", "1"],
            &["West", "2"],
            &["West", "3"],
        ],
    );
    sheet(&mut p).select_cell((1, 1));
    run(&mut p, "data.outline.subtotal=at=A;fn=sum;cols=B");
    assert_eq!(
        col(&mut p, 1, 9),
        [
            "Region",
            "East",
            "East",
            "East Total",
            "West",
            "West",
            "West",
            "West Total",
            "Grand Total"
        ]
    );
    assert_eq!(sheet(&mut p).source(4, 2), "=SUBTOTAL(9,B2:B3)");
    assert_eq!(sheet(&mut p).display(4, 2), "12");
    assert_eq!(sheet(&mut p).display(8, 2), "6");
    assert_eq!(sheet(&mut p).display(9, 2), "18");
    let levels: Vec<u32> = (1..=9).map(|r| sheet(&mut p).data_row_level(r)).collect();
    assert_eq!(levels, [0, 2, 2, 1, 2, 2, 2, 1, 0]);
    assert!(sheet(&mut p).undo());
    assert_eq!(sheet(&mut p).source(4, 1), "West");
    run(&mut p, "data.outline.subtotal=at=A;fn=sum;cols=B");
    sheet(&mut p).select_cell((1, 1));
    run(&mut p, "data.outline.subtotal=remove");
    assert_eq!(
        col(&mut p, 1, 7),
        ["Region", "East", "East", "West", "West", "West", ""]
    );
    assert_eq!(sheet(&mut p).data_row_level(2), 0);
    run(&mut p, "data.outline.subtotal");
    cancels(&mut p, "Subtotal");
}

#[test]
fn get_data_and_automate_controls_are_left_unhandled() {
    let mut p = rig();
    for id in [
        "data.get-transform-data.get-data",
        "data.queries-connections.refresh-all",
        "data.forecast.forecast-sheet",
        "data.data-tools.quick-fill",
        "automate.scripts.new-script",
    ] {
        assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::No, "{id}");
    }
}

#[test]
fn sort_dialog_model_edits_levels_and_applies() {
    use open365_preview::dialogs::data_sort::SortDialog;
    let mut p = rig();
    fill(
        &mut p,
        &[
            &["Region", "Sales"],
            &["East", "5"],
            &["West", "7"],
            &["East", "9"],
        ],
    );
    sheet(&mut p).select_cell((2, 2));
    let mut d = SortDialog::open(sheet(&mut p));
    assert!(d.has_header, "text above numbers is a header");
    assert_eq!(d.levels.len(), 1);
    assert_eq!(d.levels[0].index, 2, "the active cell's column");
    assert_eq!(
        d.column_names(sheet(&mut p)),
        [(1, "Region".to_owned()), (2, "Sales".to_owned())]
    );
    d.add_level().unwrap();
    assert_eq!(d.levels[1].index, 1, "the first unused column");
    assert!(d.add_level().is_ok());
    assert!(d.to_request().is_err(), "a column used twice is refused");
    d.delete_level(2);
    d.levels[0].descending = true;
    d.move_level(1, true);
    assert_eq!(d.levels[0].index, 1);
    d.copy_level(0).unwrap();
    d.delete_level(1);
    d.apply(sheet(&mut p)).unwrap();
    assert_eq!(col(&mut p, 2, 4), ["Sales", "9", "5", "7"]);
    d.levels.pop();
    d.levels[0].index = 9;
    assert!(d.to_request().is_err());
}

#[test]
fn group_columns_nests_hides_and_survives_a_save() {
    let mut p = rig();
    fill(&mut p, &[&["a", "b", "c", "d", "total"]]);
    sheet(&mut p).select_cols(2, 4, false);
    run(&mut p, "data.outline.group");
    assert_eq!(
        (1..=5)
            .map(|c| sheet(&mut p).data_col_level(c))
            .collect::<Vec<_>>(),
        [0, 1, 1, 1, 0]
    );
    sheet(&mut p).select_cols(5, 5, false);
    run(&mut p, "data.outline.hide-detail");
    assert!(
        sheet(&mut p)
            .home_col_state(3)
            .is_some_and(|(_, hidden)| hidden)
    );
    run(&mut p, "data.outline.show-detail");
    assert!(
        sheet(&mut p)
            .home_col_state(3)
            .is_none_or(|(_, hidden)| !hidden)
    );
    let path = std::env::temp_dir().join("open365-data-group-cols.xlsx");
    sheet(&mut p).save(&path).unwrap();
    let reopened = SheetModel::open(&path).unwrap();
    assert_eq!(reopened.data_col_level(3), 1);
    let _ = std::fs::remove_file(path);
    sheet(&mut p).select_cols(3, 3, false);
    run(&mut p, "data.outline.ungroup");
    assert_eq!(sheet(&mut p).data_col_level(3), 0);
    assert_eq!(sheet(&mut p).data_col_level(2), 1);
}

#[test]
fn subtotal_with_the_summary_above_and_validation_and_filter_survive_a_save() {
    let mut p = rig();
    fill(
        &mut p,
        &[&["k", "v"], &["a", "1"], &["a", "2"], &["b", "4"]],
    );
    sheet(&mut p).select_cell((1, 1));
    run(&mut p, "data.outline.subtotal=at=A;fn=sum;cols=B;above");
    assert_eq!(
        col(&mut p, 1, 7),
        ["k", "Grand Total", "a Total", "a", "a", "b Total", "b"]
    );
    assert_eq!(sheet(&mut p).display(2, 2), "7");
    assert_eq!(sheet(&mut p).display(3, 2), "3");
    assert_eq!(sheet(&mut p).display(6, 2), "4");
    let levels: Vec<u32> = (1..=7).map(|r| sheet(&mut p).data_row_level(r)).collect();
    assert_eq!(levels, [0, 0, 1, 2, 2, 1, 2]);
    // Filter, validation and the hidden rows are kept in the file.
    let mut p = rig();
    fill(
        &mut p,
        &[&["f", "n"], &["a", "1"], &["b", "2"], &["a", "3"]],
    );
    sheet(&mut p).select_cell((1, 1));
    run(&mut p, "data.sort-filter.filter");
    run(&mut p, "data.sort-filter.filter=A:values=a");
    sheet(&mut p).select_cell((2, 2));
    sheet(&mut p).extend_to((4, 2));
    run(&mut p, "data.data-tools.data-validation=decimal=gt,0");
    let path = std::env::temp_dir().join("open365-data-persist.xlsx");
    sheet(&mut p).save(&path).unwrap();
    let again = SheetModel::open(&path).unwrap();
    let _ = std::fs::remove_file(path);
    assert_eq!(again.data_filter_range(), Some((1, 1, 4, 2)));
    assert_eq!(again.data_filtered_columns(), [1]);
    assert!(again.home_row_state(3).1);
    assert!(again.data_check_entry(2, 2, "-1").is_err());
    assert!(again.data_check_entry(2, 2, "1.5").is_ok());
}
