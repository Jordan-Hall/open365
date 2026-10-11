//! Column width and row height edits: `col` splitting and merging, attribute preservation,
//! undo snapshots, validation and the guarantee that only the edited sheet part changes.

mod common;

use open365_opc::{Limits, Package};
use open365_xlsx::{Workbook, XlsxError};

use common::{build, fixture, sheet_xml};

const SHEET1: &str = "xl/worksheets/sheet1.xml";

fn open(bytes: &[u8]) -> Workbook {
    Workbook::open(bytes, &Limits::default()).expect("opens")
}

fn sheet_text(wb: &mut Workbook) -> String {
    let bytes = wb.save().expect("saves");
    let pkg = Package::read(&bytes, &Limits::default()).expect("package");
    String::from_utf8(pkg.part(SHEET1).expect("sheet part").data().to_vec()).expect("utf-8")
}

/// The `<cols>` element of a saved sheet.
fn cols_of(text: &str) -> &str {
    let start = text.find("<cols>").expect("cols");
    let end = text.find("</cols>").expect("cols end") + "</cols>".len();
    &text[start..end]
}

fn workbook_with(before: &str, data: &str) -> Workbook {
    open(&build(&sheet_xml(before, data, ""), &[]))
}

#[test]
fn a_new_cols_element_is_created_before_sheet_data() {
    let mut wb = workbook_with(
        "",
        "<sheetData><row r=\"1\"><c r=\"A1\"/></row></sheetData>",
    );
    wb.set_column_width(0, 2, 3, 20.5).expect("sets");
    let text = sheet_text(&mut wb);
    assert!(text.contains(
        "<cols><col min=\"2\" max=\"3\" width=\"20.5\" customWidth=\"1\"/></cols><sheetData>"
    ));
    let cols = wb.sheet(0).expect("sheet").columns();
    assert_eq!(cols.len(), 1);
    assert_eq!(
        (cols[0].min, cols[0].max, cols[0].width),
        (2, 3, Some(20.5))
    );
    assert!(cols[0].custom_width);
}

#[test]
fn a_range_inside_a_col_splits_it_and_keeps_every_other_attribute() {
    let mut wb = workbook_with(
        "<cols><col min=\"1\" max=\"10\" width=\"9\" style=\"3\" hidden=\"1\" bestFit=\"1\" outlineLevel=\"2\"/></cols>",
        "<sheetData/>",
    );
    wb.set_column_width(0, 4, 5, 30.0).expect("sets");
    let text = sheet_text(&mut wb);
    assert_eq!(
        cols_of(&text),
        "<cols>\
<col min=\"1\" max=\"3\" width=\"9\" style=\"3\" hidden=\"1\" bestFit=\"1\" outlineLevel=\"2\"/>\
<col min=\"4\" max=\"5\" width=\"30\" style=\"3\" bestFit=\"1\" outlineLevel=\"2\" customWidth=\"1\"/>\
<col min=\"6\" max=\"10\" width=\"9\" style=\"3\" hidden=\"1\" bestFit=\"1\" outlineLevel=\"2\"/>\
</cols>"
    );
}

#[test]
fn a_range_spanning_several_cols_and_gaps_covers_every_column_once() {
    let mut wb = workbook_with(
        "<cols><col min=\"2\" max=\"3\" width=\"5\" style=\"1\"/><col min=\"6\" max=\"8\" width=\"7\"/></cols>",
        "<sheetData/>",
    );
    wb.set_column_width(0, 3, 7, 12.0).expect("sets");
    let cols = wb.sheet(0).expect("sheet").columns();
    let got: Vec<_> = cols
        .iter()
        .map(|c| (c.min, c.max, c.width, c.style))
        .collect();
    assert_eq!(
        got,
        vec![
            (2, 2, Some(5.0), Some(1)),
            (3, 3, Some(12.0), Some(1)),
            // The gap 4..=5 and the untouched-style part 6..=7 merge into one entry.
            (4, 7, Some(12.0), None),
            (8, 8, Some(7.0), None),
        ]
    );
}

#[test]
fn neighbours_with_the_same_attributes_merge() {
    let mut wb = workbook_with(
        "<cols><col min=\"1\" max=\"2\" width=\"20\" customWidth=\"1\"/><col min=\"4\" max=\"5\" width=\"20\" customWidth=\"1\"/></cols>",
        "<sheetData/>",
    );
    wb.set_column_width(0, 3, 3, 20.0).expect("sets");
    let text = sheet_text(&mut wb);
    assert_eq!(
        cols_of(&text),
        "<cols><col min=\"1\" max=\"5\" width=\"20\" customWidth=\"1\"/></cols>"
    );
}

#[test]
fn cols_untouched_by_the_edit_are_left_alone() {
    let mut wb = workbook_with(
        "<cols><col min=\"1\" max=\"1\" width=\"9\"/><col min=\"2\" max=\"2\" width=\"9\"/><col min=\"9\" max=\"9\" width=\"4\"/></cols>",
        "<sheetData/>",
    );
    wb.set_column_width(0, 9, 9, 6.0).expect("sets");
    let text = sheet_text(&mut wb);
    assert_eq!(
        cols_of(&text),
        "<cols><col min=\"1\" max=\"1\" width=\"9\"/><col min=\"2\" max=\"2\" width=\"9\"/>\
<col min=\"9\" max=\"9\" width=\"6\" customWidth=\"1\"/></cols>"
    );
}

#[test]
fn restoring_the_snapshot_puts_the_cols_back() {
    let before = "<cols><col min=\"1\" max=\"4\" width=\"9\" style=\"2\"/></cols>";
    let mut wb = workbook_with(before, "<sheetData/>");
    let snap = wb.set_column_width(0, 2, 2, 40.0).expect("sets");
    assert_ne!(cols_of(&sheet_text(&mut wb)), before);
    wb.restore_columns(0, &snap).expect("restores");
    assert_eq!(cols_of(&sheet_text(&mut wb)), before);

    // No cols element at all: restoring removes the one the edit created.
    let mut wb = workbook_with("", "<sheetData/>");
    let snap = wb.set_column_width(0, 1, 1, 3.0).expect("sets");
    wb.restore_columns(0, &snap).expect("restores");
    assert!(!sheet_text(&mut wb).contains("<cols"));
}

#[test]
fn row_height_sets_ht_and_custom_height_and_keeps_the_rest() {
    let mut wb = workbook_with(
        "",
        "<sheetData><row r=\"1\" spans=\"1:2\" s=\"4\" customFormat=\"1\" hidden=\"1\" ht=\"15\" thickBot=\"1\"><c r=\"A1\"/></row></sheetData>",
    );
    wb.set_row_height(0, 1, 1, 33.75).expect("sets");
    let text = sheet_text(&mut wb);
    assert!(
        text.contains(
            "<row r=\"1\" spans=\"1:2\" s=\"4\" customFormat=\"1\" ht=\"33.75\" thickBot=\"1\" customHeight=\"1\">"
        ),
        "{text}"
    );
    let row = &wb.sheet(0).expect("sheet").rows()[0];
    assert_eq!(row.height(), Some(33.75));
    assert!(row.custom_height());
}

#[test]
fn row_height_creates_missing_rows_in_order_and_undo_removes_them() {
    let data =
        "<sheetData><row r=\"2\"><c r=\"A2\"/></row><row r=\"9\"><c r=\"A9\"/></row></sheetData>";
    let mut wb = workbook_with("", data);
    let snap = wb.set_row_height(0, 1, 3, 20.0).expect("sets");
    let rows: Vec<_> = wb
        .sheet(0)
        .expect("sheet")
        .rows()
        .iter()
        .map(|r| (r.r, r.height()))
        .collect();
    assert_eq!(
        rows,
        vec![(1, Some(20.0)), (2, Some(20.0)), (3, Some(20.0)), (9, None)]
    );
    wb.restore_row_heights(0, &snap).expect("restores");
    let rows: Vec<_> = wb
        .sheet(0)
        .expect("sheet")
        .rows()
        .iter()
        .map(|r| (r.r, r.height(), r.custom_height()))
        .collect();
    assert_eq!(rows, vec![(2, None, false), (9, None, false)]);
    assert!(sheet_text(&mut wb).contains(data));
}

#[test]
fn undo_restores_an_earlier_height() {
    let mut wb = workbook_with(
        "",
        "<sheetData><row r=\"1\" ht=\"18\" customHeight=\"1\"><c r=\"A1\"/></row></sheetData>",
    );
    let snap = wb.set_row_height(0, 1, 1, 50.0).expect("sets");
    wb.restore_row_heights(0, &snap).expect("restores");
    let row = &wb.sheet(0).expect("sheet").rows()[0];
    assert_eq!(row.height(), Some(18.0));
    assert!(row.custom_height());
}

#[test]
fn limits_and_ranges_are_validated_without_changing_anything() {
    let mut wb = workbook_with("", "<sheetData/>");
    let before = sheet_text(&mut wb);
    // The cap is 255 characters plus the cell padding (see `Workbook::max_column_width`).
    let over = wb.max_column_width() + 0.5;
    for width in [-0.5, over, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            wb.set_column_width(0, 1, 1, width),
            Err(XlsxError::UnsupportedEdit(_))
        ));
    }
    for points in [-1.0, 409.6, f64::NAN] {
        assert!(wb.set_row_height(0, 1, 1, points).is_err());
    }
    assert!(wb.set_column_width(0, 0, 1, 5.0).is_err());
    assert!(wb.set_column_width(0, 3, 2, 5.0).is_err());
    assert!(wb.set_column_width(0, 1, 16_385, 5.0).is_err());
    assert!(wb.set_row_height(0, 0, 1, 5.0).is_err());
    assert!(wb.set_row_height(0, 1, 1_048_577, 5.0).is_err());
    // Every row at once is the sheet default (Excel's select-all), not an error.
    assert!(wb.set_row_height(0, 1, 1_048_576, 5.0).is_ok());
    assert!(wb.set_row_height(0, 1, 20_000, 5.0).is_err());
    assert!(matches!(
        wb.set_column_width(3, 1, 1, 5.0),
        Err(XlsxError::SheetOutOfRange { .. })
    ));
    // The extremes are allowed.
    wb.set_column_width(0, 1, 1, 0.0).expect("0");
    wb.set_column_width(0, 1, 1, 255.0).expect("255");
    wb.set_row_height(0, 1, 1, 0.0).expect("0");
    wb.set_row_height(0, 1, 1, 409.5).expect("409.5");
    assert_ne!(sheet_text(&mut wb), before);
}

/// ADR-001: resizing changes the edited sheet part and nothing else in the package.
#[test]
fn only_the_edited_sheet_part_changes() {
    for name in ["grid-geometry", "formulas-basic", "cross-sheet"] {
        let original = fixture(name);
        let mut wb = open(&original);
        wb.set_column_width(0, 2, 3, 17.25).expect("column");
        wb.set_row_height(0, 2, 2, 31.5).expect("row");
        let saved = wb.save().expect("saves");
        let a = Package::read(&original, &Limits::default()).expect("package");
        let b = Package::read(&saved, &Limits::default()).expect("package");
        let names: Vec<String> = a.parts().map(|p| p.name().as_str().to_owned()).collect();
        let saved_names: Vec<String> = b.parts().map(|p| p.name().as_str().to_owned()).collect();
        assert_eq!(names, saved_names, "{name}: part list");
        let changed: Vec<&String> = names
            .iter()
            .filter(|n| a.part(n).map(|p| p.data()) != b.part(n).map(|p| p.data()))
            .collect();
        assert_eq!(changed.len(), 1, "{name}: {changed:?}");
        assert!(
            changed[0].starts_with("xl/worksheets/"),
            "{name}: {changed:?}"
        );
    }
}
