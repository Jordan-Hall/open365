//! Hostile and malformed input: nothing may panic, hang or be silently half-read.

mod common;

use open365_opc::{Limits, Package, PartName};
use open365_xlsx::{CellInput, CellValue, Workbook, XlsxError};

use common::{build, sheet_xml, xlsx_with_sheet};

fn open(bytes: &[u8]) -> Result<Workbook, XlsxError> {
    Workbook::open(bytes, &Limits::default())
}

fn rows(inner: &str) -> String {
    sheet_xml("", &format!("<sheetData>{inner}</sheetData>"), "")
}

#[test]
fn doctype_is_rejected_everywhere() {
    let dtd = r#"<!DOCTYPE x [<!ENTITY e "boom">]>"#;
    let sheet = format!(
        "<?xml version=\"1.0\"?>{dtd}<worksheet xmlns=\"{}\"><sheetData/></worksheet>",
        common::MAIN
    );
    assert!(matches!(
        open(&xlsx_with_sheet(&sheet)),
        Err(XlsxError::DoctypeForbidden)
    ));
    // Inside the document too.
    let inside = format!(
        "<worksheet xmlns=\"{}\"><sheetData/>{dtd}</worksheet>",
        common::MAIN
    );
    assert!(open(&xlsx_with_sheet(&inside)).is_err());

    let workbook = format!(
        "{dtd}<workbook xmlns=\"{}\"><sheets/></workbook>",
        common::MAIN
    );
    assert!(matches!(
        open(&build(
            &rows(""),
            &[("xl/workbook.xml", workbook.as_bytes())]
        )),
        Err(XlsxError::DoctypeForbidden)
    ));

    let sst = format!("{dtd}<sst xmlns=\"{}\"/>", common::MAIN);
    let extra: [(&str, &[u8]); 2] = [
        ("xl/sharedStrings.xml", sst.as_bytes()),
        (
            "xl/_rels/workbook.xml.rels",
            br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#,
        ),
    ];
    assert!(matches!(
        open(&build(&rows(""), &extra)),
        Err(XlsxError::DoctypeForbidden)
    ));
}

#[test]
fn entities_that_are_not_predefined_are_errors() {
    let bad = rows(r#"<row r="1"><c r="A1" t="str"><f>&xxe;</f><v>1</v></c></row>"#);
    assert!(open(&xlsx_with_sheet(&bad)).is_err());
    let bad = rows(r#"<row r="1"><c r="A1" t="str"><v>&#xD800;</v></c></row>"#);
    assert!(open(&xlsx_with_sheet(&bad)).is_err());
}

#[test]
fn deep_nesting_is_an_error_not_a_stack_overflow() {
    let deep = format!("{}{}", "<x>".repeat(10_000), "</x>".repeat(10_000));
    // After sheetData (general path), inside a cell, and in the workbook.
    let after = sheet_xml("", "<sheetData/>", &deep);
    assert!(matches!(
        open(&xlsx_with_sheet(&after)),
        Err(XlsxError::DepthExceeded(_))
    ));
    let in_cell = rows(&format!(
        r#"<row r="1"><c r="A1"><extLst>{deep}</extLst></c></row>"#
    ));
    assert!(matches!(
        open(&xlsx_with_sheet(&in_cell)),
        Err(XlsxError::DepthExceeded(_))
    ));
    let before = sheet_xml(&deep, "<sheetData/>", "");
    assert!(matches!(
        open(&xlsx_with_sheet(&before)),
        Err(XlsxError::DepthExceeded(_))
    ));
}

#[test]
fn absurd_cell_references_are_errors() {
    for r in [
        "XFD1048577",
        "XFE1",
        "A0",
        "A",
        "1",
        "ZZZZ1",
        "A1B",
        "$A$1",
        "A-1",
        "",
        "A99999999999",
    ] {
        let sheet = rows(&format!(r#"<row r="1"><c r="{r}"><v>1</v></c></row>"#));
        assert!(open(&xlsx_with_sheet(&sheet)).is_err(), "cell ref {r:?}");
    }
    for row in ["0", "1048577", "-1", "x", "4294967296", ""] {
        let sheet = rows(&format!(r#"<row r="{row}"><c r="A1"><v>1</v></c></row>"#));
        assert!(open(&xlsx_with_sheet(&sheet)).is_err(), "row {row:?}");
    }
    // The extremes of the grid are fine.
    let sheet = rows(r#"<row r="1048576"><c r="XFD1048576"><v>1</v></c></row>"#);
    let wb = open(&xlsx_with_sheet(&sheet)).expect("last cell is valid");
    assert!(wb.cell(0, 1_048_576, 16_384).is_some());
}

#[test]
fn rows_and_cells_must_be_in_order_and_consistent() {
    let cases = [
        r#"<row r="2"><c r="A2"/></row><row r="1"><c r="A1"/></row>"#,
        r#"<row r="1"><c r="A1"/></row><row r="1"><c r="A1"/></row>"#,
        r#"<row r="1"><c r="B1"/><c r="A1"/></row>"#,
        r#"<row r="1"><c r="A1"/><c r="A1"/></row>"#,
        r#"<row r="1"><c r="A2"/></row>"#,
        r#"<row r="1"><c r="A1" t="zz"><v>1</v></c></row>"#,
        r#"<row r="1"><c r="A1"><v>notanumber</v></c></row>"#,
        r#"<row r="1"><c r="A1"><v>inf</v></c></row>"#,
        r#"<row r="1"><c r="A1" t="s"><v>5</v></c></row>"#,
        r#"<row r="1"><c r="A1" t="b"><v>maybe</v></c></row>"#,
        r#"<row r="1"><c r="A1"><f t="shared"/></c></row>"#,
        r#"<row r="1"><c r="A1">stray text</c></row>"#,
        r#"<row r="1">stray text</row>"#,
    ];
    for case in cases {
        let r = open(&xlsx_with_sheet(&rows(case)));
        assert!(r.is_err(), "{case}");
    }
}

#[test]
fn dimension_lies_are_ignored() {
    for dim in ["A1:A1", "garbage", "ZZ9999:A1", "", "A1:XFD1048576"] {
        let sheet = sheet_xml(
            &format!(r#"<dimension ref="{dim}"/>"#),
            r#"<sheetData><row r="3"><c r="C3"><v>1</v></c></row><row r="100"><c r="Z100"><v>2</v></c></row></sheetData>"#,
            "",
        );
        let mut wb = open(&xlsx_with_sheet(&sheet)).unwrap_or_else(|e| panic!("{dim}: {e}"));
        assert_eq!(
            wb.sheet(0).unwrap().used_range(),
            Some((3, 3, 100, 26)),
            "{dim}"
        );
        assert_eq!(wb.cell(0, 100, 26).unwrap().value, CellValue::Number(2.0));
        // Editing with a lying dimension still produces a valid reference.
        wb.set_cell_value(0, 1, 1, CellInput::Number(1.0)).unwrap();
        let xml = String::from_utf8(wb.serialize_sheet(0).unwrap()).unwrap();
        assert!(xml.contains("<dimension ref=\""), "{dim}: {xml}");
    }
}

#[test]
fn missing_and_wrong_parts_are_errors() {
    // No sheetData.
    let no_data = format!("<worksheet xmlns=\"{}\"/>", common::MAIN);
    assert!(open(&xlsx_with_sheet(&no_data)).is_err());
    // Not a worksheet.
    assert!(open(&xlsx_with_sheet("<html/>")).is_err());
    // Two sheetData elements.
    assert!(
        open(&xlsx_with_sheet(&sheet_xml(
            "",
            "<sheetData/><sheetData/>",
            ""
        )))
        .is_err()
    );
    // Truncated and non-UTF-8.
    assert!(open(&xlsx_with_sheet("<worksheet")).is_err());
    let mut package = Package::read(&xlsx_with_sheet(&rows("")), &Limits::default()).unwrap();
    package.set_part(
        PartName::new("xl/worksheets/sheet1.xml").unwrap(),
        vec![0xff, 0xfe, b'<'],
    );
    assert!(matches!(
        open(&package.write().unwrap()),
        Err(XlsxError::NotUtf8)
    ));
    // Relationship to a missing part.
    package.remove_part("xl/worksheets/sheet1.xml");
    assert!(matches!(
        open(&package.write().unwrap()),
        Err(XlsxError::MissingPart(_))
    ));
    // No workbook at all.
    package.remove_part("_rels/.rels");
    assert!(matches!(
        open(&package.write().unwrap()),
        Err(XlsxError::NoWorkbook)
    ));
    // Not a zip.
    assert!(open(b"PK nope").is_err());
}

#[test]
fn many_tiny_rows_do_not_blow_up() {
    let mut inner = String::new();
    for r in 1..=20_000u32 {
        inner.push_str(&format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c></row>"#));
    }
    let wb = open(&xlsx_with_sheet(&rows(&inner))).unwrap();
    assert_eq!(wb.iter_cells(0).count(), 20_000);
}

#[test]
fn unusual_but_valid_markup_takes_the_general_path_and_survives() {
    // Prefixed elements, comments, whitespace, single quotes, entities, extLst, inline strings.
    let pretty = "
  <!-- c -->
  <x:row r=\"1\" spans='1:3'>
   <x:c r=\"A1\" t=\"inlineStr\"><x:is><x:r><x:t>ri</x:t></x:r><x:r><x:t> ch</x:t></x:r></x:is></x:c>
   <x:c r=\"B1\" t=\"str\"><x:f>\"a&amp;b\"&amp;\"&lt;\"</x:f><x:v>a&amp;b&lt;</x:v></x:c>
   <x:c r=\"C1\" cm=\"1\"><x:v>1.50</x:v><x:extLst><x:ext uri=\"u\"/></x:extLst></x:c>
  </x:row>
 ";
    let compact = "<x:row r=\"1\" spans=\"1:3\"><x:c r=\"A1\" t=\"inlineStr\"><x:is><x:r><x:t>ri</x:t></x:r><x:r><x:t> ch</x:t></x:r></x:is></x:c><x:c r=\"B1\" t=\"str\"><x:f>\"a&amp;b\"&amp;\"&lt;\"</x:f><x:v>a&amp;b&lt;</x:v></x:c><x:c r=\"C1\" cm=\"1\"><x:v>1.50</x:v><x:extLst><x:ext uri=\"u\"/></x:extLst></x:c></x:row>";
    let wrap = |inner: &str| {
        format!(
            "<?xml version=\"1.0\"?>
<x:worksheet xmlns:x=\"{}\" xmlns:r=\"urn:r\"><x:sheetData>{inner}</x:sheetData></x:worksheet>
",
            common::MAIN
        )
    };
    let sheet = wrap(pretty);
    let wb = open(&xlsx_with_sheet(&sheet)).unwrap();
    let ws = wb.sheet(0).unwrap();
    assert_eq!(ws.cell(1, 1).unwrap().value.as_text(), Some("ri ch"));
    assert_eq!(ws.cell(1, 2).unwrap().value.as_text(), Some("a&b<"));
    assert_eq!(ws.formula_text(1, 2).as_deref(), Some("\"a&b\"&\"<\""));
    assert_eq!(ws.cell(1, 3).unwrap().value, CellValue::Number(1.5));
    // The raw text of `1.50` and the extension list come back out.
    let out = String::from_utf8(wb.serialize_sheet(0).unwrap()).unwrap();
    assert!(out.contains("<x:v>1.50</x:v>"), "{out}");
    assert!(
        out.contains("<x:extLst><x:ext uri=\"u\"/></x:extLst>"),
        "{out}"
    );
    assert!(out.contains("cm=\"1\""), "{out}");
    assert!(out.contains("<x:r><x:t>ri</x:t></x:r>"), "{out}");
    // Whitespace and comments between sheetData elements carry no meaning and are not kept.
    common::assert_infoset_eq(wrap(compact).as_bytes(), out.as_bytes()).expect("same infoset");
}
