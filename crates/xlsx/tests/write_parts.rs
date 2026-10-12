//! Adding parts and relationships keeps `[Content_Types].xml` and the relationship parts in step.

mod write_support;

use open365_opc::{PartName, TargetMode};
use open365_xlsx::XlsxError;
use write_support::{fixture, has_part, reopen, text_of};

#[test]
fn a_new_part_gets_an_override_and_a_relationship() {
    let mut wb = fixture("formulas-basic");
    let wb_part = wb.workbook_part().clone();
    wb.add_part(
        "xl/custom/data.xml",
        "application/vnd.example.data+xml",
        b"<d/>".to_vec(),
    )
    .expect("adds");
    let rid = wb
        .add_relationship(
            Some(&wb_part),
            "http://example.com/rel/data",
            "custom/data.xml",
            TargetMode::Internal,
        )
        .expect("relates");
    assert!(rid.starts_with("rId"));
    let ct = text_of(&mut wb, "[Content_Types].xml");
    assert!(ct.contains(
        "<Override PartName=\"/xl/custom/data.xml\" ContentType=\"application/vnd.example.data+xml\"/>"
    ), "{ct}");
    let rels = text_of(&mut wb, "xl/_rels/workbook.xml.rels");
    assert!(rels.contains(&format!("Id=\"{rid}\"")) && rels.contains("Target=\"custom/data.xml\""));
    reopen(&mut wb);
}

#[test]
fn a_vml_part_uses_the_extension_default_like_excel() {
    let mut wb = fixture("formulas-basic");
    wb.add_part(
        "xl/drawings/vmlDrawing9.vml",
        open365_xlsx::content_types::VML_DRAWING,
        b"<xml/>".to_vec(),
    )
    .expect("adds");
    let ct = text_of(&mut wb, "[Content_Types].xml");
    assert!(ct.contains("<Default Extension=\"vml\" ContentType=\"application/vnd.openxmlformats-officedocument.vmlDrawing\"/>"), "{ct}");
    assert!(
        !ct.contains("PartName=\"/xl/drawings/vmlDrawing9.vml\""),
        "{ct}"
    );
    reopen(&mut wb);
}

#[test]
fn external_relationships_are_marked_and_duplicates_refused() {
    let mut wb = fixture("formulas-basic");
    let sheet = wb.sheet_part(0).cloned().expect("sheet part");
    let rid = wb
        .add_relationship(
            Some(&sheet),
            open365_xlsx::relationship_types::HYPERLINK,
            "https://example.com/",
            TargetMode::External,
        )
        .expect("relates");
    let rels = text_of(&mut wb, "xl/worksheets/_rels/sheet1.xml.rels");
    assert!(
        rels.contains(&format!("Id=\"{rid}\"")) && rels.contains("TargetMode=\"External\""),
        "{rels}"
    );
    wb.add_part("xl/x.xml", "application/x+xml", Vec::new())
        .expect("adds");
    let again = wb.add_part("xl/x.xml", "application/x+xml", Vec::new());
    assert!(matches!(again, Err(XlsxError::UnsupportedEdit(_))));
    let missing = wb.add_relationship(
        Some(&PartName::new("xl/nothing.xml").expect("name")),
        "t",
        "x",
        TargetMode::Internal,
    );
    assert!(matches!(missing, Err(XlsxError::MissingPart(_))));
    assert!(has_part(&mut wb, "xl/x.xml"));
}

#[test]
fn adding_a_part_requests_a_recalculation_like_any_first_edit() {
    let mut wb = fixture("formulas-basic");
    wb.add_part("xl/y.xml", "application/y+xml", Vec::new())
        .expect("adds");
    let book = text_of(&mut wb, "xl/workbook.xml");
    assert!(book.contains("fullCalcOnLoad=\"1\""), "{book}");
}
