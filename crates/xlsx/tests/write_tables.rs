//! Creating tables (Insert > Table, Format as Table).

mod common;
mod write_support;

use open365_xlsx::{
    CellInput, CellValue, HeaderChoice, TableSpec, TableStyleChoice, Workbook, XlsxError,
};
use write_support::{open, reopen, text_of};

fn blank() -> Workbook {
    open(&common::build(
        &common::sheet_xml("", "<sheetData/>", ""),
        &[],
    ))
}

fn put_text(wb: &mut Workbook, r: u32, c: u32, t: &str) {
    wb.set_cell_value(0, r, c, CellInput::Text(t.to_owned()))
        .expect("text");
}

fn put_num(wb: &mut Workbook, r: u32, c: u32, n: f64) {
    wb.set_cell_value(0, r, c, CellInput::Number(n))
        .expect("number");
}

fn people() -> Workbook {
    let mut wb = blank();
    put_text(&mut wb, 1, 1, "Name");
    put_text(&mut wb, 1, 2, "Qty");
    put_text(&mut wb, 2, 1, "Ann");
    put_num(&mut wb, 2, 2, 3.0);
    put_text(&mut wb, 3, 1, "Bob");
    put_num(&mut wb, 3, 2, 4.0);
    wb
}

#[test]
fn a_table_over_text_headers_is_written_like_excel() {
    let mut wb = people();
    let info = wb
        .create_table(
            0,
            &TableSpec {
                range: (1, 1, 3, 2),
                ..TableSpec::default()
            },
        )
        .expect("creates");
    assert_eq!(
        (info.name.as_str(), info.id, info.range),
        ("Table1", 1, (1, 1, 3, 2))
    );
    assert!(!info.header_inserted);
    let part = text_of(&mut wb, "xl/tables/table1.xml");
    assert!(part.contains(
        "<table xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" id=\"1\" name=\"Table1\" displayName=\"Table1\" ref=\"A1:B3\" totalsRowShown=\"0\"><autoFilter ref=\"A1:B3\"/><tableColumns count=\"2\"><tableColumn id=\"1\" name=\"Name\"/><tableColumn id=\"2\" name=\"Qty\"/></tableColumns><tableStyleInfo name=\"TableStyleMedium2\" showFirstColumn=\"0\" showLastColumn=\"0\" showRowStripes=\"1\" showColumnStripes=\"0\"/></table>"
    ), "{part}");
    let sheet = text_of(&mut wb, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("<tableParts count=\"1\"><tablePart xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" r:id=\"rId1\"/></tableParts>"), "{sheet}");
    let rels = text_of(&mut wb, "xl/worksheets/_rels/sheet1.xml.rels");
    assert!(
        rels.contains(
            "Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/table\""
        ) && rels.contains("Target=\"../tables/table1.xml\""),
        "{rels}"
    );
    let ct = text_of(&mut wb, "[Content_Types].xml");
    assert!(ct.contains("PartName=\"/xl/tables/table1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml\""), "{ct}");
    let back = reopen(&mut wb);
    let (sheet_index, table) = back.table_by_name("Table1").expect("table survives");
    assert_eq!((sheet_index, table.range_text.as_str()), (0, "A1:B3"));
}

#[test]
fn data_without_headers_gets_a_header_row_and_moves_down() {
    let mut wb = blank();
    put_num(&mut wb, 1, 1, 1.0);
    put_num(&mut wb, 1, 2, 2.0);
    put_num(&mut wb, 2, 1, 3.0);
    put_num(&mut wb, 2, 2, 4.0);
    wb.set_cell_formula(0, 5, 1, "SUM(A1:A2)", None)
        .expect("formula");
    let info = wb
        .create_table(
            0,
            &TableSpec {
                range: (1, 1, 2, 2),
                ..TableSpec::default()
            },
        )
        .expect("creates");
    assert!(info.header_inserted);
    assert_eq!(info.range, (1, 1, 3, 2));
    assert_eq!(
        wb.cell(0, 1, 1)
            .and_then(|c| c.value.as_text().map(str::to_owned))
            .as_deref(),
        Some("Column1")
    );
    assert_eq!(
        wb.cell(0, 1, 2)
            .and_then(|c| c.value.as_text().map(str::to_owned))
            .as_deref(),
        Some("Column2")
    );
    assert_eq!(
        wb.cell(0, 2, 1).map(|c| c.value.clone()),
        Some(CellValue::Number(1.0))
    );
    assert_eq!(
        wb.cell(0, 3, 2).map(|c| c.value.clone()),
        Some(CellValue::Number(4.0))
    );
    // The formula below follows the data it summed.
    assert_eq!(wb.formula_text(0, 6, 1).as_deref(), Some("SUM(A2:A3)"));
    reopen(&mut wb);
}

#[test]
fn a_totals_row_labels_the_first_column_and_sums_the_last() {
    let mut wb = people();
    let info = wb
        .create_table(
            0,
            &TableSpec {
                range: (1, 1, 3, 2),
                totals_row: true,
                ..TableSpec::default()
            },
        )
        .expect("creates");
    assert_eq!(info.range, (1, 1, 4, 2));
    let part = text_of(&mut wb, "xl/tables/table1.xml");
    assert!(
        part.contains("ref=\"A1:B4\" totalsRowCount=\"1\">"),
        "{part}"
    );
    assert!(part.contains("<autoFilter ref=\"A1:B3\"/>"), "{part}");
    assert!(part.contains("<tableColumn id=\"1\" name=\"Name\" totalsRowLabel=\"Total\"/><tableColumn id=\"2\" name=\"Qty\" totalsRowFunction=\"sum\"/>"), "{part}");
    assert_eq!(
        wb.cell(0, 4, 1)
            .and_then(|c| c.value.as_text().map(str::to_owned))
            .as_deref(),
        Some("Total")
    );
    assert_eq!(
        wb.formula_text(0, 4, 2).as_deref(),
        Some("SUBTOTAL(109,Table1[Qty])")
    );
    let back = reopen(&mut wb);
    assert_eq!(
        back.table_by_name("Table1").and_then(|(_, t)| t.rect),
        Some((1, 1, 4, 2))
    );
}

#[test]
fn styles_names_and_ids_follow_excel() {
    let mut wb = people();
    put_text(&mut wb, 6, 1, "k");
    put_text(&mut wb, 7, 1, "k");
    put_num(&mut wb, 6, 2, 1.0);
    put_num(&mut wb, 7, 2, 2.0);
    wb.create_table(
        0,
        &TableSpec {
            range: (1, 1, 3, 2),
            style: TableStyleChoice::None,
            auto_filter: false,
            ..TableSpec::default()
        },
    )
    .expect("first");
    let second = wb
        .create_table(
            0,
            &TableSpec {
                range: (6, 1, 7, 2),
                headers: HeaderChoice::No,
                style: TableStyleChoice::Named("TableStyleLight9".to_owned()),
                ..TableSpec::default()
            },
        )
        .expect("second");
    assert_eq!((second.name.as_str(), second.id), ("Table2", 2));
    let first_part = text_of(&mut wb, "xl/tables/table1.xml");
    assert!(!first_part.contains("autoFilter"), "{first_part}");
    assert!(first_part.contains("<tableStyleInfo showFirstColumn=\"0\" showLastColumn=\"0\" showRowStripes=\"1\" showColumnStripes=\"0\"/>"), "{first_part}");
    let sheet = text_of(&mut wb, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("<tableParts count=\"2\">"), "{sheet}");
    let second_part = text_of(&mut wb, "xl/tables/table2.xml");
    assert!(
        second_part.contains("name=\"TableStyleLight9\""),
        "{second_part}"
    );
    reopen(&mut wb);
}

#[test]
fn headers_are_made_unique_and_text() {
    let mut wb = blank();
    put_text(&mut wb, 1, 1, "A");
    put_text(&mut wb, 1, 2, "a");
    put_num(&mut wb, 1, 3, 2020.0);
    put_text(&mut wb, 2, 1, "x");
    wb.create_table(
        0,
        &TableSpec {
            range: (1, 1, 2, 3),
            headers: HeaderChoice::Yes,
            ..TableSpec::default()
        },
    )
    .expect("creates");
    let part = text_of(&mut wb, "xl/tables/table1.xml");
    assert!(
        part.contains(
            "name=\"A\"/><tableColumn id=\"2\" name=\"a2\"/><tableColumn id=\"3\" name=\"2020\"/>"
        ),
        "{part}"
    );
    assert_eq!(
        wb.cell(0, 1, 3)
            .and_then(|c| c.value.as_text().map(str::to_owned))
            .as_deref(),
        Some("2020")
    );
}

#[test]
fn overlaps_bad_names_and_merges_are_refused() {
    let mut wb = people();
    wb.create_table(
        0,
        &TableSpec {
            range: (1, 1, 3, 2),
            ..TableSpec::default()
        },
    )
    .expect("first");
    let overlap = wb.create_table(
        0,
        &TableSpec {
            range: (3, 2, 5, 3),
            ..TableSpec::default()
        },
    );
    assert!(matches!(overlap, Err(XlsxError::UnsupportedEdit(_))));
    for bad in ["", "1abc", "A1", "has space", "Table1"] {
        let r = wb.create_table(
            0,
            &TableSpec {
                range: (10, 1, 12, 2),
                name: Some(bad.to_owned()),
                headers: HeaderChoice::No,
                ..TableSpec::default()
            },
        );
        assert!(r.is_err(), "{bad:?} should be refused");
    }
    let off = wb.create_table(
        0,
        &TableSpec {
            range: (0, 1, 3, 2),
            ..TableSpec::default()
        },
    );
    assert!(off.is_err());
}
