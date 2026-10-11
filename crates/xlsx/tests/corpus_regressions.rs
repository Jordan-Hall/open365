//! Regressions found by the template-corpus round trip (`cargo xtask corpus-roundtrip`).
//! Each input is a minimal package built here; no corpus content is copied.
#![allow(clippy::unwrap_used)]

mod common;

use open365_opc::{Limits, Package};
use open365_xlsx::{CellInput, Workbook};

use common::{sheet_xml, xlsx_with_sheet};

fn sheet_part(bytes: &[u8]) -> String {
    let p = Package::read(bytes, &Limits::default()).unwrap();
    String::from_utf8(p.part("xl/worksheets/sheet1.xml").unwrap().data().to_vec()).unwrap()
}

/// A formula that returns text with edge spaces is cached as `<v xml:space="preserve"> x</v>`.
/// The attribute used to land in the dropped-content log, which blocked saving.
#[test]
fn v_element_attributes_are_preserved_not_dropped() {
    let cell = r#"<c r="A1" t="str"><f>" "&amp;"x"</f><v xml:space="preserve"> x </v></c>"#;
    let bytes = xlsx_with_sheet(&sheet_xml(
        "",
        &format!(r#"<sheetData><row r="1">{cell}</row></sheetData>"#),
        "",
    ));
    let mut wb = Workbook::open(&bytes, &Limits::default()).unwrap();
    assert!(wb.dropped().is_empty(), "{:?}", wb.dropped());
    let saved = wb.save().unwrap();
    assert!(sheet_part(&saved).contains(cell), "{}", sheet_part(&saved));

    // The attribute survives an edit elsewhere in the sheet.
    let mut wb = Workbook::open(&bytes, &Limits::default()).unwrap();
    wb.set_cell_value(0, 2, 1, CellInput::Number(1.0)).unwrap();
    assert!(sheet_part(&wb.save().unwrap()).contains(cell));

    // Replacing the cell's content removes the attribute with the old value.
    let mut wb = Workbook::open(&bytes, &Limits::default()).unwrap();
    wb.set_cell_value(0, 1, 1, CellInput::Number(5.0)).unwrap();
    let out = sheet_part(&wb.save().unwrap());
    assert!(!out.contains("xml:space"), "{out}");
}
