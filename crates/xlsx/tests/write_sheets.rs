//! Sheet operations (insert, delete, rename, move, copy, tab colour, hide) and Insert/Delete cells.

mod common;
mod write_support;

use open365_xlsx::{
    CellInput, CellValue, Rgb, SheetState, ShiftDirection, TabColor, TableSpec, Workbook, XlsxError,
};
use write_support::{fixture, has_part, open, reopen, text_of};

fn names(wb: &Workbook) -> Vec<String> {
    wb.sheets().iter().map(|s| s.name.clone()).collect()
}

fn formula(wb: &Workbook, sheet: usize, row: u32, col: u32) -> String {
    wb.formula_text(sheet, row, col).unwrap_or_default()
}

#[test]
fn a_new_sheet_is_a_blank_worksheet_part_with_its_wiring() {
    let mut wb = fixture("cross-sheet");
    let at = wb.add_sheet(1, None).expect("adds");
    assert_eq!(at, 1);
    assert_eq!(names(&wb), ["One", "Sheet4", "Two", "Three"]);
    let book = text_of(&mut wb, "xl/workbook.xml");
    assert!(
        book.contains("<sheet name=\"Sheet4\" sheetId=\"4\" r:id=\"rId"),
        "{book}"
    );
    let rels = text_of(&mut wb, "xl/_rels/workbook.xml.rels");
    assert!(rels.contains("Target=\"worksheets/sheet4.xml\""), "{rels}");
    let ct = text_of(&mut wb, "[Content_Types].xml");
    assert!(ct.contains("PartName=\"/xl/worksheets/sheet4.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\""), "{ct}");
    assert!(text_of(&mut wb, "xl/worksheets/sheet4.xml").contains("<sheetData/>"));
    let back = reopen(&mut wb);
    assert_eq!(names(&back), ["One", "Sheet4", "Two", "Three"]);
    // Formulas that name other sheets are untouched by an insert.
    assert_eq!(formula(&back, 2, 1, 1), "One!A1*2");
    assert!(wb.add_sheet(0, Some("two")).is_err());
    for bad in ["", "a/b", "x[1]", "'q", "q'", &"n".repeat(32), "History"] {
        assert!(wb.add_sheet(0, Some(bad)).is_err(), "{bad:?}");
    }
}

#[test]
fn renaming_a_sheet_rewrites_every_reference() {
    let mut wb = fixture("cross-sheet");
    wb.define_name(&open365_xlsx::NameSpec {
        name: "Link".to_owned(),
        formula: "One!$A$1".to_owned(),
        ..Default::default()
    })
    .expect("name");
    let recalc = wb.rename_sheet(0, "Base data").expect("renames");
    assert!(!recalc.is_empty());
    assert_eq!(names(&wb), ["Base data", "Two", "Three"]);
    assert_eq!(formula(&wb, 1, 1, 1), "'Base data'!A1*2");
    assert_eq!(formula(&wb, 2, 1, 1), "Two!A1+'Base data'!A1");
    assert_eq!(wb.defined_names()[0].formula, "'Base data'!$A$1");
    let book = text_of(&mut wb, "xl/workbook.xml");
    assert!(
        book.contains("<sheet name=\"Base data\" sheetId=\"1\""),
        "{book}"
    );
    assert!(wb.rename_sheet(0, "TWO").is_err());
    assert!(
        wb.rename_sheet(0, "Base data")
            .expect("same name")
            .is_empty()
    );
    reopen(&mut wb);
}

#[test]
fn renaming_follows_hyperlinks_and_chart_series() {
    let mut wb = fixture("chart");
    let old = wb.sheets()[0].name.clone();
    let chart_before = text_of(&mut wb, "xl/charts/chart1.xml");
    assert!(
        chart_before.contains(&format!("{old}!")),
        "the fixture chart refers to its sheet"
    );
    wb.set_hyperlink(
        0,
        &open365_xlsx::HyperlinkSpec {
            range: (9, 1, 9, 1),
            target: open365_xlsx::HyperlinkTarget::Location(format!("{old}!A1")),
            display: None,
            tooltip: None,
        },
    )
    .expect("link");
    wb.rename_sheet(0, "Sales 2020").expect("renames");
    let chart = text_of(&mut wb, "xl/charts/chart1.xml");
    assert!(
        chart.contains("'Sales 2020'!") && !chart.contains(&format!("<c:f>{old}!")),
        "{chart}"
    );
    assert_eq!(
        wb.hyperlinks(0)[0].target,
        open365_xlsx::HyperlinkTarget::Location("'Sales 2020'!A1".to_owned())
    );
}

#[test]
fn deleting_a_sheet_removes_its_parts_names_and_references() {
    let mut wb = fixture("cross-sheet");
    wb.define_name(&open365_xlsx::NameSpec {
        name: "OnTwo".to_owned(),
        scope: Some(1),
        formula: "Two!$A$1".to_owned(),
        ..Default::default()
    })
    .expect("scoped");
    wb.define_name(&open365_xlsx::NameSpec {
        name: "OnThree".to_owned(),
        scope: Some(2),
        formula: "Three!$A$1".to_owned(),
        ..Default::default()
    })
    .expect("scoped");
    wb.delete_sheet(1).expect("deletes");
    assert_eq!(names(&wb), ["One", "Three"]);
    assert!(!has_part(&mut wb, "xl/worksheets/sheet2.xml"));
    let ct = text_of(&mut wb, "[Content_Types].xml");
    assert!(!ct.contains("sheet2.xml"), "{ct}");
    let rels = text_of(&mut wb, "xl/_rels/workbook.xml.rels");
    assert!(!rels.contains("sheet2.xml"), "{rels}");
    // Three's formula pointed at Two and now reads #REF!; One pointed at Three and still does.
    assert_eq!(formula(&wb, 1, 1, 1), "Two!#REF!+One!A1");
    assert_eq!(formula(&wb, 0, 1, 2), "Three!A1+1");
    // The name scoped to Two went; the one scoped to Three moved to index 1.
    let kept: Vec<(String, Option<u32>)> = wb
        .defined_names()
        .iter()
        .map(|n| (n.name.clone(), n.local_sheet_id))
        .collect();
    assert_eq!(kept, vec![("OnThree".to_owned(), Some(1))]);
    reopen(&mut wb);
    assert!(wb.delete_sheet(7).is_err());
}

#[test]
fn the_last_visible_sheet_cannot_be_deleted_or_hidden() {
    let mut wb = fixture("formulas-basic");
    assert!(matches!(
        wb.delete_sheet(0),
        Err(XlsxError::UnsupportedEdit(_))
    ));
    assert!(matches!(
        wb.set_sheet_state(0, SheetState::Hidden),
        Err(XlsxError::UnsupportedEdit(_))
    ));
    wb.add_sheet(1, Some("Second")).expect("adds");
    wb.set_sheet_state(0, SheetState::Hidden)
        .expect("hides the first");
    assert!(wb.set_sheet_state(1, SheetState::VeryHidden).is_err());
    let book = text_of(&mut wb, "xl/workbook.xml");
    assert!(
        book.contains("<sheet name=\"Sheet1\" sheetId=\"1\" state=\"hidden\""),
        "{book}"
    );
    assert_eq!(
        wb.active_sheet(),
        1,
        "the active tab moves off the hidden sheet"
    );
    wb.set_sheet_state(0, SheetState::Visible).expect("unhides");
    assert!(!text_of(&mut wb, "xl/workbook.xml").contains("state="));
    let back = reopen(&mut wb);
    assert_eq!(back.sheets()[0].state, SheetState::Visible);
}

#[test]
fn moving_a_sheet_remaps_scoped_names_and_the_active_tab() {
    let mut wb = fixture("cross-sheet");
    wb.define_name(&open365_xlsx::NameSpec {
        name: "OnOne".to_owned(),
        scope: Some(0),
        formula: "One!$A$1".to_owned(),
        ..Default::default()
    })
    .expect("scoped");
    wb.move_sheet(0, 2).expect("moves");
    assert_eq!(names(&wb), ["Two", "Three", "One"]);
    assert_eq!(wb.defined_names()[0].local_sheet_id, Some(2));
    assert_eq!(
        wb.active_sheet(),
        1,
        "the active tab (Three) travels with its sheet"
    );
    let book = text_of(&mut wb, "xl/workbook.xml");
    assert!(
        book.find("name=\"Two\"").expect("two") < book.find("name=\"One\"").expect("one"),
        "{book}"
    );
    reopen(&mut wb);
    assert!(wb.move_sheet(0, 9).is_err());
}

#[test]
fn copying_a_sheet_copies_its_tables_and_retargets_self_references() {
    let mut wb = fixture("model-tables");
    let tables_before = wb.tables().count();
    let at = wb.copy_sheet(0, 1, None).expect("copies");
    assert_eq!(at, 1);
    assert_eq!(names(&wb), ["Data", "Data (2)", "Other"]);
    assert!(wb.tables().count() > tables_before);
    let copied: Vec<String> = wb
        .tables()
        .filter(|(i, _)| *i == 1)
        .map(|(_, t)| t.name.clone())
        .collect();
    assert!(copied.iter().all(|n| n.ends_with("_2")), "{copied:?}");
    let ids: Vec<u32> = wb.tables().map(|(_, t)| t.id).collect();
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(ids.len(), unique.len(), "table ids stay unique: {ids:?}");
    let back = reopen(&mut wb);
    assert_eq!(names(&back), ["Data", "Data (2)", "Other"]);
}

#[test]
fn copying_keeps_charts_notes_and_scoped_names_and_does_not_group_the_sheets() {
    let mut wb = fixture("chart");
    wb.copy_sheet(0, 1, Some("Copy")).expect("copies");
    assert_eq!(wb.sheet_charts(1).len(), wb.sheet_charts(0).len());
    let sheet2 = text_of(&mut wb, "xl/worksheets/sheet2.xml");
    assert!(
        !sheet2.contains("tabSelected"),
        "a copy must not be selected too: {sheet2}"
    );
    let back = reopen(&mut wb);
    assert_eq!(back.sheet_charts(1).len(), back.sheet_charts(0).len());

    let mut notes = fixture("comments-notes");
    notes.copy_sheet(0, 1, None).expect("copies notes");
    assert_eq!(notes.notes(1).len(), notes.notes(0).len());
    reopen(&mut notes);

    let mut scoped = fixture("defined-names");
    let sheet = scoped.sheets()[0].name.clone();
    scoped.copy_sheet(0, 1, None).expect("copies names");
    let local: Vec<(String, Option<u32>, String)> = scoped
        .defined_names()
        .iter()
        .filter(|n| n.local_sheet_id.is_some())
        .map(|n| (n.name.clone(), n.local_sheet_id, n.formula.clone()))
        .collect();
    assert_eq!(local.len(), 2, "{local:?}");
    assert!(
        local
            .iter()
            .any(|(_, id, f)| *id == Some(1) && f.contains(&format!("{sheet} (2)"))),
        "{local:?}"
    );
    reopen(&mut scoped);
}

#[test]
fn tab_colour_is_a_tabcolor_child_of_sheetpr() {
    let mut wb = fixture("formulas-basic");
    wb.set_tab_color(0, TabColor::Rgb(Rgb::new(255, 0, 0)))
        .expect("rgb");
    let sheet = text_of(&mut wb, "xl/worksheets/sheet1.xml");
    assert!(
        sheet.contains("<sheetPr><tabColor rgb=\"FFFF0000\"/></sheetPr><dimension"),
        "{sheet}"
    );
    assert_eq!(wb.tab_color(0), Some(TabColor::Rgb(Rgb::new(255, 0, 0))));
    wb.set_tab_color(
        0,
        TabColor::Theme {
            index: 4,
            tint: 0.4,
        },
    )
    .expect("theme");
    assert!(
        text_of(&mut wb, "xl/worksheets/sheet1.xml")
            .contains("<tabColor theme=\"4\" tint=\"0.4\"/>")
    );
    wb.set_tab_color(0, TabColor::None).expect("clear");
    assert_eq!(wb.tab_color(0), None);
    reopen(&mut wb);
}

fn grid() -> Workbook {
    let mut wb = open(&common::build(
        &common::sheet_xml("", "<sheetData/>", ""),
        &[],
    ));
    for r in 1..=5u32 {
        for c in 1..=3u32 {
            wb.set_cell_value(0, r, c, CellInput::Number(f64::from(r * 10 + c)))
                .expect("n");
        }
    }
    wb
}

fn num(wb: &Workbook, r: u32, c: u32) -> Option<f64> {
    match wb.cell(0, r, c).map(|x| &x.value) {
        Some(CellValue::Number(n)) => Some(*n),
        _ => None,
    }
}

#[test]
fn inserting_cells_shifts_down_only_the_selected_columns() {
    let mut wb = grid();
    wb.set_cell_formula(0, 8, 1, "SUM(A2:A5)", None)
        .expect("sum in band");
    wb.set_cell_formula(0, 8, 3, "SUM(A2:C2)", None)
        .expect("row range partly in band");
    wb.set_cell_formula(0, 9, 1, "B3+A3", None).expect("mixed");
    let recalc = wb
        .shift_cells(0, (3, 1, 4, 1), ShiftDirection::Down)
        .expect("inserts");
    assert_eq!(num(&wb, 2, 1), Some(21.0));
    assert_eq!(num(&wb, 3, 1), None);
    assert_eq!(num(&wb, 5, 1), Some(31.0), "A3 moved two rows down");
    assert_eq!(num(&wb, 3, 2), Some(32.0), "column B did not move");
    // The sum range straddles the insert point: it grows. The formula cell itself moves.
    assert_eq!(formula(&wb, 0, 10, 1), "SUM(A2:A7)");
    assert_eq!(
        formula(&wb, 0, 8, 3),
        "SUM(A2:C2)",
        "a range that leaves the band is not rewritten"
    );
    assert_eq!(formula(&wb, 0, 11, 1), "B3+A5");
    assert!(recalc.contains(&(0, 10, 1)));
    reopen(&mut wb);
}

#[test]
fn deleting_cells_shifts_up_and_kills_references_to_deleted_cells() {
    let mut wb = grid();
    wb.set_cell_formula(0, 7, 5, "A3", None)
        .expect("deleted cell");
    wb.set_cell_formula(0, 7, 6, "A5", None)
        .expect("moved cell");
    wb.set_cell_formula(0, 7, 7, "SUM(A2:A5)", None)
        .expect("shrinks");
    wb.shift_cells(0, (3, 1, 3, 1), ShiftDirection::Up)
        .expect("deletes");
    assert_eq!(num(&wb, 3, 1), Some(41.0));
    assert_eq!(num(&wb, 5, 1), None);
    assert_eq!(formula(&wb, 0, 7, 5), "#REF!");
    assert_eq!(formula(&wb, 0, 7, 6), "A4");
    assert_eq!(formula(&wb, 0, 7, 7), "SUM(A2:A4)");
    reopen(&mut wb);
}

#[test]
fn horizontal_shifts_move_cells_between_columns_of_the_chosen_rows() {
    let mut wb = grid();
    wb.shift_cells(0, (2, 1, 3, 1), ShiftDirection::Right)
        .expect("inserts");
    assert_eq!(num(&wb, 2, 2), Some(21.0));
    assert_eq!(num(&wb, 3, 4), Some(33.0));
    assert_eq!(num(&wb, 4, 1), Some(41.0), "row 4 was not selected");
    assert_eq!(num(&wb, 2, 1), None);
    wb.shift_cells(0, (2, 1, 3, 1), ShiftDirection::Left)
        .expect("deletes");
    assert_eq!(num(&wb, 2, 1), Some(21.0));
    assert_eq!(num(&wb, 3, 3), Some(33.0));
    reopen(&mut wb);
}

#[test]
fn shifts_that_excel_refuses_are_refused() {
    let mut wb = open(&common::build(
        &common::sheet_xml(
            "",
            "<sheetData><row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\"><v>2</v></c></row><row r=\"2\"><c r=\"A2\"><v>3</v></c></row></sheetData>",
            "<mergeCells count=\"1\"><mergeCell ref=\"A1:B1\"/></mergeCells>",
        ),
        &[],
    ));
    // Cutting a merged area in half.
    assert!(
        wb.shift_cells(0, (1, 1, 1, 1), ShiftDirection::Down)
            .is_err()
    );
    // Pushing cells off the grid.
    let mut full = grid();
    full.set_cell_value(0, 1_048_576, 1, CellInput::Number(1.0))
        .expect("last row");
    assert!(
        full.shift_cells(0, (1, 1, 1, 1), ShiftDirection::Down)
            .is_err()
    );
    // A table cut in half.
    let mut t = grid();
    t.set_cell_value(0, 1, 1, CellInput::Text("a".to_owned()))
        .expect("h");
    t.set_cell_value(0, 1, 2, CellInput::Text("b".to_owned()))
        .expect("h");
    t.create_table(
        0,
        &TableSpec {
            range: (1, 1, 3, 2),
            ..TableSpec::default()
        },
    )
    .expect("table");
    assert!(
        t.shift_cells(0, (2, 1, 2, 1), ShiftDirection::Down)
            .is_err()
    );
    assert!(
        t.shift_cells(0, (0, 1, 1, 1), ShiftDirection::Down)
            .is_err()
    );
}

#[test]
fn a_table_that_moves_whole_keeps_its_part_in_step() {
    let mut wb = grid();
    wb.set_cell_value(0, 1, 1, CellInput::Text("a".to_owned()))
        .expect("h");
    wb.set_cell_value(0, 1, 2, CellInput::Text("b".to_owned()))
        .expect("h");
    wb.create_table(
        0,
        &TableSpec {
            range: (1, 1, 3, 2),
            ..TableSpec::default()
        },
    )
    .expect("table");
    // Insert above the table across both of its columns: the table moves down one row.
    let mut wb = open(&common::build(
        &common::sheet_xml("", "<sheetData/>", ""),
        &[],
    ));
    for (r, c, t) in [(3, 1, "h1"), (3, 2, "h2")] {
        wb.set_cell_value(0, r, c, CellInput::Text(t.to_owned()))
            .expect("h");
    }
    wb.set_cell_value(0, 4, 1, CellInput::Number(1.0))
        .expect("d");
    wb.create_table(
        0,
        &TableSpec {
            range: (3, 1, 4, 2),
            ..TableSpec::default()
        },
    )
    .expect("table");
    wb.shift_cells(0, (1, 1, 1, 2), ShiftDirection::Down)
        .expect("insert above");
    let part = text_of(&mut wb, "xl/tables/table1.xml");
    assert!(
        part.contains("ref=\"A4:B5\"") && part.contains("<autoFilter ref=\"A4:B5\"/>"),
        "{part}"
    );
    assert_eq!(
        wb.table_by_name("Table1").and_then(|(_, t)| t.rect),
        Some((4, 1, 5, 2))
    );
    reopen(&mut wb);
}
