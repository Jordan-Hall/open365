//! Editing: constants and formulas, the first-edit hygiene, shared-formula materialisation.

mod common;

use std::collections::HashSet;
use std::path::PathBuf;

use open365_opc::{IntegrityIssue, Limits, Package};
use open365_xlsx::{
    CellInput, CellValue, FormulaKind, HygieneAction, SheetKind, Workbook, XlsxError,
};

use common::{Node, fixture, fixtures, infoset, repo_root};

fn open(bytes: &[u8]) -> Workbook {
    Workbook::open(bytes, &Limits::default()).expect("opens")
}

fn package(bytes: &[u8]) -> Package {
    Package::read(bytes, &Limits::default()).expect("package")
}

fn out_dir() -> PathBuf {
    let dir = repo_root().join("target").join("xlsx-edited");
    std::fs::create_dir_all(&dir).expect("output dir");
    dir
}

/// `Override` part names of `[Content_Types].xml`, lower-cased.
fn overrides(pkg: &Package) -> Vec<String> {
    let part = pkg.part("[Content_Types].xml").expect("content types");
    let Node::Element { children, .. } = infoset(part.data()).expect("parses") else {
        panic!("root is an element");
    };
    children
        .iter()
        .filter_map(|c| match c {
            Node::Element { local, attrs, .. } if local == "Override" => attrs
                .get(&(String::new(), "PartName".to_owned()))
                .map(|n| n.to_ascii_lowercase()),
            _ => None,
        })
        .collect()
}

fn issues(pkg: &Package) -> Vec<IntegrityIssue> {
    pkg.check_integrity()
}

fn assert_content_types_consistent(name: &str, pkg: &Package) {
    let parts: HashSet<String> = pkg
        .parts()
        .map(|p| format!("/{}", p.name().as_str().to_ascii_lowercase()))
        .collect();
    for o in overrides(pkg) {
        assert!(parts.contains(&o), "{name}: override for missing part {o}");
    }
}

fn workbook_xml(pkg: &Package, wb: &Workbook) -> String {
    let part = pkg
        .part(wb.workbook_part().as_str())
        .expect("workbook part");
    String::from_utf8(part.data().to_vec()).expect("utf-8")
}

/// What every first edit must have done to the package.
fn assert_hygiene(name: &str, before: &Package, after: &Package, wb: &Workbook) {
    assert!(
        after.parts().all(|p| !p
            .name()
            .as_str()
            .to_ascii_lowercase()
            .ends_with("calcchain.xml")),
        "{name}: calcChain still present"
    );
    assert!(
        !overrides(after).iter().any(|o| o.contains("calcchain")),
        "{name}: calcChain override still present"
    );
    let rels = after
        .part("xl/_rels/workbook.xml.rels")
        .expect("workbook rels");
    assert!(
        !String::from_utf8_lossy(rels.data()).contains("calcChain"),
        "{name}: calcChain relationship still present"
    );
    let xml = workbook_xml(after, wb);
    assert!(xml.contains("fullCalcOnLoad=\"1\""), "{name}: {xml}");
    assert_content_types_consistent(name, after);
    let before_issues = issues(before);
    for issue in issues(after) {
        assert!(
            before_issues.contains(&issue),
            "{name}: new integrity issue {issue:?}"
        );
    }
}

fn value_of(wb: &Workbook, sheet: usize, row: u32, col: u32) -> CellValue {
    wb.cell(sheet, row, col)
        .map_or(CellValue::Empty, |c| c.value.clone())
}

#[test]
fn formulas_basic_edit_round_trips() {
    let bytes = fixture("formulas-basic");
    let before = package(&bytes);
    let mut wb = open(&bytes);
    let sst_before = wb.shared_strings().expect("sst").len();
    wb.set_cell_value(0, 1, 1, CellInput::Number(99.0)).unwrap();
    wb.set_cell_value(0, 7, 2, CellInput::Text("brand new".into()))
        .unwrap();
    wb.set_cell_value(0, 8, 2, CellInput::Text("item1".into()))
        .unwrap();
    wb.set_cell_formula(0, 1, 5, "=SUM(A1:A5)*2", Some(CellValue::Number(210.0)))
        .unwrap();
    wb.set_cell_formula(0, 2, 5, "B1&\"!\"", None).unwrap();
    let saved = wb.save().unwrap();
    std::fs::write(out_dir().join("formulas-basic-multi.xlsx"), &saved).unwrap();

    let after = package(&saved);
    let wb2 = open(&saved);
    assert_hygiene("formulas-basic", &before, &after, &wb2);
    assert!(wb2.dropped().is_empty());
    assert_eq!(value_of(&wb2, 0, 1, 1), CellValue::Number(99.0));
    assert_eq!(value_of(&wb2, 0, 7, 2).as_text(), Some("brand new"));
    // "item1" already existed as a plain item: reused, not appended.
    assert_eq!(value_of(&wb2, 0, 8, 2).as_text(), Some("item1"));
    assert_eq!(wb2.shared_strings().unwrap().len(), sst_before + 1);
    assert_eq!(wb2.formula_text(0, 1, 5).as_deref(), Some("SUM(A1:A5)*2"));
    assert_eq!(value_of(&wb2, 0, 1, 5), CellValue::Number(210.0));
    assert_eq!(wb2.formula_text(0, 2, 5).as_deref(), Some("B1&\"!\""));
    assert_eq!(value_of(&wb2, 0, 2, 5), CellValue::Empty);
    // The log says what was done.
    let log: Vec<String> = wb.hygiene_log().iter().map(ToString::to_string).collect();
    assert!(log.iter().any(|l| l.contains("calcChain.xml")), "{log:?}");
    assert!(wb.hygiene_log().contains(&HygieneAction::SetFullCalcOnLoad));
    assert!(
        wb.hygiene_log()
            .iter()
            .any(|a| matches!(a, HygieneAction::RemovedContentTypeOverride { .. }))
    );
    // Untouched parts are byte-identical.
    for part in before.parts() {
        let name = part.name().as_str();
        if matches!(
            name,
            "xl/workbook.xml"
                | "xl/worksheets/sheet1.xml"
                | "xl/sharedStrings.xml"
                | "xl/calcChain.xml"
                | "xl/_rels/workbook.xml.rels"
                | "[Content_Types].xml"
        ) {
            continue;
        }
        assert_eq!(
            after.part(name).map(|p| p.data()),
            Some(part.data()),
            "{name} changed"
        );
    }
}

#[test]
fn values_types_edit_round_trips() {
    let bytes = fixture("values-types");
    let before = package(&bytes);
    let mut wb = open(&bytes);
    wb.set_cell_value(0, 1, 1, CellInput::Number(-0.5)).unwrap();
    wb.set_cell_value(0, 2, 1, CellInput::Text("text value".into()))
        .unwrap();
    wb.set_cell_value(
        0,
        6,
        1,
        CellInput::Text("  padded\nline_x0041_ \u{1}".into()),
    )
    .unwrap();
    wb.set_cell_value(0, 3, 1, CellInput::Bool(false)).unwrap();
    wb.set_cell_formula(0, 7, 1, "A1*2", Some(CellValue::Number(-1.0)))
        .unwrap();
    wb.set_cell_formula(
        0,
        8,
        1,
        "UPPER(A2)",
        Some(CellValue::Str("TEXT VALUE".into())),
    )
    .unwrap();
    let saved = wb.save().unwrap();
    std::fs::write(out_dir().join("values-types-multi.xlsx"), &saved).unwrap();

    let after = package(&saved);
    let wb2 = open(&saved);
    assert_hygiene("values-types", &before, &after, &wb2);
    assert_eq!(value_of(&wb2, 0, 1, 1), CellValue::Number(-0.5));
    assert_eq!(value_of(&wb2, 0, 2, 1).as_text(), Some("text value"));
    assert_eq!(
        value_of(&wb2, 0, 6, 1).as_text(),
        Some("  padded\nline_x0041_ \u{1}")
    );
    assert_eq!(value_of(&wb2, 0, 3, 1), CellValue::Bool(false));
    assert_eq!(value_of(&wb2, 0, 8, 1).as_text(), Some("TEXT VALUE"));
    // Existing text was reused; uniqueCount only grew by the one new string.
    let sst =
        String::from_utf8(after.part("xl/sharedStrings.xml").unwrap().data().to_vec()).unwrap();
    assert!(sst.contains("uniqueCount=\"2\""), "{sst}");
    // Style of the date cell survives; the error cell A4 is untouched.
    assert!(matches!(value_of(&wb2, 0, 4, 1), CellValue::Error(_)));
    assert_eq!(wb2.number_format(0, 5, 1), r"yyyy\-mm\-dd");
}

#[test]
fn workbook_without_shared_strings_gets_one() {
    let bytes = fixture("number-formats");
    let before = package(&bytes);
    assert!(before.part("xl/sharedStrings.xml").is_none());
    let mut wb = open(&bytes);
    wb.set_cell_value(0, 1, 2, CellInput::Text("label".into()))
        .unwrap();
    wb.set_cell_value(0, 2, 2, CellInput::Text("label".into()))
        .unwrap();
    let saved = wb.save().unwrap();
    std::fs::write(out_dir().join("number-formats-newsst.xlsx"), &saved).unwrap();
    let after = package(&saved);
    assert_hygiene("number-formats", &before, &after, &open(&saved));
    assert!(after.part("xl/sharedStrings.xml").is_some());
    assert!(overrides(&after).contains(&"/xl/sharedstrings.xml".to_owned()));
    let wb2 = open(&saved);
    assert_eq!(value_of(&wb2, 0, 1, 2).as_text(), Some("label"));
    assert_eq!(wb2.shared_strings().unwrap().len(), 1);
    assert!(
        wb.hygiene_log()
            .iter()
            .any(|a| matches!(a, HygieneAction::AddedPart { .. }))
    );
}

#[test]
fn editing_a_shared_formula_master_keeps_every_dependent() {
    let bytes = fixture("shared-formulas");
    let before = package(&bytes);
    let mut wb = open(&bytes);
    let expected: Vec<(u32, u32, String)> = wb
        .iter_cells(0)
        .filter(|(_, _, c)| c.formula.is_some())
        .map(|(r, c, _)| (r, c, wb.formula_text(0, r, c).unwrap()))
        .collect();
    assert_eq!(expected.len(), 100);
    // B1 is the master of group 0 (B1:B32).
    let master = wb
        .cell(0, 1, 2)
        .unwrap()
        .formula
        .as_deref()
        .unwrap()
        .clone();
    assert!(matches!(master.kind, FormulaKind::Shared { si: 0, .. }));
    wb.set_cell_value(0, 1, 2, CellInput::Number(7.0)).unwrap();
    let saved = wb.save().unwrap();
    std::fs::write(out_dir().join("shared-formulas-master.xlsx"), &saved).unwrap();

    let after = package(&saved);
    let wb2 = open(&saved);
    assert_hygiene("shared-formulas", &before, &after, &wb2);
    assert_eq!(value_of(&wb2, 0, 1, 2), CellValue::Number(7.0));
    assert!(wb2.cell(0, 1, 2).unwrap().formula.is_none());
    let mut normal = 0;
    for (r, c, text) in &expected {
        if (*r, *c) == (1, 2) {
            continue;
        }
        assert_eq!(
            wb2.formula_text(0, *r, *c).as_deref(),
            Some(text.as_str()),
            "{r},{c}"
        );
        let f = wb2.cell(0, *r, *c).unwrap().formula.as_deref().unwrap();
        // Group 0 (rows 2..=32) became normal formulas; the other groups are untouched.
        if *r <= 32 {
            assert_eq!(f.kind, FormulaKind::Normal, "B{r}");
            normal += 1;
        } else {
            assert!(matches!(f.kind, FormulaKind::Shared { .. }), "B{r}");
        }
        // Cached values were not touched.
        assert_eq!(
            value_of(&wb2, 0, *r, *c),
            value_of(&open(&bytes), 0, *r, *c),
            "B{r}"
        );
    }
    assert_eq!(normal, 31);
}

#[test]
fn replacing_a_formula_with_text_updates_counts() {
    let bytes = fixture("formulas-basic");
    let mut wb = open(&bytes);
    // B1 is a shared string ("item1"): replacing it by a number drops one reference.
    let count_before = sst_attr(&mut wb, "count");
    wb.set_cell_value(0, 1, 2, CellInput::Number(1.0)).unwrap();
    let count_after = sst_attr(&mut wb, "count");
    assert_eq!(count_after + 1, count_before);
    // D1 holds a formula; an empty input clears value and formula, keeping nothing behind.
    wb.set_cell_value(0, 1, 4, CellInput::Empty).unwrap();
    let saved = wb.save().unwrap();
    let wb2 = open(&saved);
    assert!(wb2.cell(0, 1, 4).is_none());
    assert!(wb2.cell(0, 1, 2).is_some());
}

fn sst_attr(wb: &mut Workbook, name: &str) -> u64 {
    let bytes = wb.save().expect("saves");
    let pkg = package(&bytes);
    let data = pkg.part("xl/sharedStrings.xml").expect("sst").data();
    let text = String::from_utf8(data.to_vec()).expect("utf-8");
    let needle = format!("{name}=\"");
    let at = text.find(&needle).expect("attribute") + needle.len();
    text[at..]
        .split('"')
        .next()
        .expect("value")
        .parse()
        .expect("number")
}

#[test]
fn a_failed_edit_changes_nothing() {
    let bytes = fixture("formulas-basic");
    let mut wb = open(&bytes);
    assert!(matches!(
        wb.set_cell_value(0, 1_048_577, 1, CellInput::Number(1.0)),
        Err(XlsxError::CellOutOfRange { .. })
    ));
    assert!(matches!(
        wb.set_cell_value(0, 1, 16_385, CellInput::Number(1.0)),
        Err(XlsxError::CellOutOfRange { .. })
    ));
    assert!(matches!(
        wb.set_cell_value(0, 0, 1, CellInput::Number(1.0)),
        Err(XlsxError::CellOutOfRange { .. })
    ));
    assert!(matches!(
        wb.set_cell_value(5, 1, 1, CellInput::Number(1.0)),
        Err(XlsxError::SheetOutOfRange { .. })
    ));
    assert!(matches!(
        wb.set_cell_value(0, 1, 1, CellInput::Number(f64::NAN)),
        Err(XlsxError::InvalidNumber(_))
    ));
    assert!(wb.set_cell_formula(0, 1, 1, "=", None).is_err());
    assert!(wb.set_cell_formula(0, 1, 1, "A1\u{1}", None).is_err());
    assert!(wb.hygiene_log().is_empty());
    assert!(!wb.is_dirty());
    // Clearing a cell that does not exist is a no-op, not an edit.
    wb.set_cell_value(0, 50, 50, CellInput::Empty).unwrap();
    assert!(!wb.is_dirty());
    let saved = wb.save().unwrap();
    let a = package(&bytes);
    let b = package(&saved);
    assert!(
        a.parts()
            .zip(b.parts())
            .all(|(x, y)| x.name() == y.name() && x.data() == y.data())
    );
}

#[test]
fn array_formula_cells_are_protected() {
    let sheet = common::sheet_xml(
        "",
        r#"<sheetData><row r="1"><c r="A1"><f t="array" ref="A1:A3">ROW(1:3)</f><v>1</v></c></row><row r="2"><c r="A2"><v>2</v></c></row></sheetData>"#,
        "",
    );
    let mut wb = open(&common::xlsx_with_sheet(&sheet));
    for (row, col) in [(1, 1), (2, 1), (3, 1)] {
        let err = wb
            .set_cell_value(0, row, col, CellInput::Number(1.0))
            .unwrap_err();
        assert!(matches!(err, XlsxError::UnsupportedEdit(_)), "{err}");
    }
    wb.set_cell_value(0, 1, 2, CellInput::Number(1.0)).unwrap();
    assert!(
        wb.hygiene_log()
            .iter()
            .any(|a| matches!(a, HygieneAction::SetFullCalcOnLoad))
    );
}

#[test]
fn dimension_and_spans_follow_new_cells() {
    let sheet = common::sheet_xml(
        r#"<dimension ref="A1:B2"/>"#,
        r#"<sheetData><row r="1" spans="1:2"><c r="A1"><v>1</v></c></row></sheetData>"#,
        "",
    );
    let mut wb = open(&common::xlsx_with_sheet(&sheet));
    wb.set_cell_value(0, 5, 4, CellInput::Number(1.0)).unwrap();
    wb.set_cell_value(0, 1, 6, CellInput::Number(1.0)).unwrap();
    let xml = String::from_utf8(wb.serialize_sheet(0).unwrap()).unwrap();
    assert!(xml.contains(r#"<dimension ref="A1:F5"/>"#), "{xml}");
    assert!(!xml.contains("spans"), "stale spans must go: {xml}");
    assert!(
        xml.contains(r#"<row r="5"><c r="D5"><v>1</v></c></row>"#),
        "{xml}"
    );
}

#[test]
fn every_fixture_survives_an_edit_and_oracle_inputs_are_written() {
    for (name, bytes) in fixtures() {
        let before = package(&bytes);
        let mut wb = open(&bytes);
        let sheet = wb
            .sheets()
            .iter()
            .position(|s| s.kind == SheetKind::Worksheet)
            .expect("a worksheet");
        let (_, _, last_row, _) = wb
            .sheet(sheet)
            .and_then(|s| s.used_range())
            .unwrap_or((1, 1, 1, 1));
        let row = last_row + 2;
        wb.set_cell_value(sheet, row, 1, CellInput::Number(12.5))
            .unwrap();
        wb.set_cell_value(sheet, row, 2, CellInput::Text(format!("edited {name}")))
            .unwrap();
        wb.set_cell_formula(sheet, row, 3, "A1+1", None).unwrap();
        wb.set_cell_formula(sheet, row + 1, 3, "1+2", Some(CellValue::Number(3.0)))
            .unwrap();
        let saved = wb.save().unwrap();
        std::fs::write(out_dir().join(format!("{name}.xlsx")), &saved).unwrap();

        let after = package(&saved);
        let wb2 = open(&saved);
        assert_hygiene(&name, &before, &after, &wb2);
        assert!(wb2.dropped().is_empty(), "{name}");
        assert_eq!(
            value_of(&wb2, sheet, row, 1),
            CellValue::Number(12.5),
            "{name}"
        );
        assert_eq!(
            value_of(&wb2, sheet, row, 2).as_text(),
            Some(format!("edited {name}").as_str()),
            "{name}"
        );
        assert_eq!(
            wb2.formula_text(sheet, row, 3).as_deref(),
            Some("A1+1"),
            "{name}"
        );
        assert_eq!(
            value_of(&wb2, sheet, row + 1, 3),
            CellValue::Number(3.0),
            "{name}"
        );
        // Everything that was there before is still there.
        for (r, c, cell) in open(&bytes).iter_cells(sheet) {
            assert_eq!(
                wb2.cell(sheet, r, c).map(|x| &x.value),
                Some(&cell.value),
                "{name} {r},{c}"
            );
        }
    }
}
