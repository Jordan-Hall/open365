//! Excel templates (.xltx) and macro-enabled files (.xlsm, .xltm): kind detection,
//! instantiation and byte-identical preservation of the VBA project. Macros never run.
#![allow(clippy::unwrap_used)]

mod common;

use open365_opc::{ContentTypes, Limits, Package, PartName};
use open365_xlsx::{CellInput, HygieneAction, Workbook, WorkbookKind};

use common::{repo_root, sheet_xml, xlsx_with_sheet};

const CT_XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml";
const CT_XLTX: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.template.main+xml";
const CT_XLSM: &str = "application/vnd.ms-excel.sheet.macroEnabled.main+xml";
const CT_XLTM: &str = "application/vnd.ms-excel.template.macroEnabled.main+xml";

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(repo_root().join("fixtures").join("xlsx").join(name)).expect("fixture exists")
}

fn pkg(bytes: &[u8]) -> Package {
    Package::read(bytes, &Limits::default()).unwrap()
}

fn main_content_type(bytes: &[u8]) -> String {
    let package = pkg(bytes);
    let ct = ContentTypes::parse(package.part("[Content_Types].xml").unwrap().data()).unwrap();
    ct.content_type_of(&PartName::new("xl/workbook.xml").unwrap())
        .unwrap()
        .to_owned()
}

fn part(bytes: &[u8], name: &str) -> Option<Vec<u8>> {
    pkg(bytes).part(name).map(|p| p.data().to_vec())
}

fn with_main_type(bytes: &[u8], to: &str) -> Vec<u8> {
    let mut package = pkg(bytes);
    let ct = String::from_utf8(package.part("[Content_Types].xml").unwrap().data().to_vec())
        .unwrap()
        .replace(&main_content_type(bytes), to);
    package.set_part(
        PartName::new("[Content_Types].xml").unwrap(),
        ct.into_bytes(),
    );
    package.write().unwrap()
}

fn open(bytes: &[u8]) -> Workbook {
    Workbook::open(bytes, &Limits::default()).expect("opens")
}

fn base() -> Vec<u8> {
    xlsx_with_sheet(&sheet_xml("", "<sheetData/>", ""))
}

#[test]
fn plain_workbook_has_workbook_kind() {
    let mut wb = open(&base());
    assert_eq!(wb.kind(), WorkbookKind::Workbook);
    assert!(!wb.has_vba_project());
    assert!(!wb.instantiate_template().unwrap(), "not a template: no-op");
    assert!(wb.hygiene_log().is_empty());
}

#[test]
fn kind_is_read_from_every_main_content_type() {
    for (ct, kind) in [
        (CT_XLSX, WorkbookKind::Workbook),
        (CT_XLTX, WorkbookKind::Template),
        (CT_XLSM, WorkbookKind::MacroEnabledWorkbook),
        (CT_XLTM, WorkbookKind::MacroEnabledTemplate),
    ] {
        let wb = open(&with_main_type(&base(), ct));
        assert_eq!(wb.kind(), kind, "{ct}");
        assert_eq!(kind.content_type(), ct);
        assert_eq!(WorkbookKind::from_content_type(ct), Some(kind));
        assert!(wb.dropped().is_empty());
    }
    assert_eq!(WorkbookKind::from_content_type("text/plain"), None);
}

#[test]
fn xltx_fixture_opens_and_instantiates_to_a_plain_workbook() {
    let bytes = fixture("template-basic.xltx");
    let mut wb = open(&bytes);
    assert_eq!(wb.kind(), WorkbookKind::Template);
    assert!(wb.kind().is_template() && !wb.kind().is_macro_enabled());
    assert!(!wb.has_vba_project());
    assert!(wb.dropped().is_empty());

    let saved = open(&bytes).save().unwrap();
    assert_eq!(main_content_type(&saved), CT_XLTX);

    assert!(wb.instantiate_template().unwrap());
    assert_eq!(wb.kind(), WorkbookKind::Workbook);
    assert!(matches!(
        wb.hygiene_log().last(),
        Some(HygieneAction::ChangedContentType { from, to, .. }) if from == CT_XLTX && to == CT_XLSX
    ));
    assert!(!wb.instantiate_template().unwrap(), "second call no-op");
    assert_eq!(wb.hygiene_log().len(), 1);

    let out = wb.save().unwrap();
    assert_eq!(main_content_type(&out), CT_XLSX);
    assert_eq!(open(&out).kind(), WorkbookKind::Workbook);

    let before = pkg(&bytes);
    let after = pkg(&out);
    for p in before.parts() {
        let q = after.part(p.name().as_str()).expect("part kept");
        if p.name().as_str() != "[Content_Types].xml" {
            assert_eq!(p.data(), q.data(), "{}", p.name());
        }
    }
    write_for_oracle("template-basic-instantiated.xlsx", &out);
}

#[test]
fn xltm_fixture_keeps_the_vba_project_byte_identical() {
    let bytes = fixture("template-macro.xltm");
    let vba = part(&bytes, "xl/vbaProject.bin").expect("fixture has a VBA project");
    let mut wb = open(&bytes);
    assert_eq!(wb.kind(), WorkbookKind::MacroEnabledTemplate);
    assert!(wb.kind().is_macro_enabled());
    assert!(wb.has_vba_project());
    assert!(wb.dropped().is_empty());

    let saved = open(&bytes).save().unwrap();
    assert_eq!(main_content_type(&saved), CT_XLTM);
    assert_eq!(part(&saved, "xl/vbaProject.bin").unwrap(), vba);

    // Edited save: still preserved.
    wb.set_cell_value(0, 5, 1, CellInput::Number(7.0)).unwrap();
    let edited = wb.save().unwrap();
    assert_eq!(main_content_type(&edited), CT_XLTM);
    assert_eq!(part(&edited, "xl/vbaProject.bin").unwrap(), vba);
    write_for_oracle("template-macro-roundtrip.xltm", &edited);

    // Instantiated: macro-enabled workbook, VBA untouched.
    let mut wb = open(&bytes);
    assert!(wb.instantiate_template().unwrap());
    assert_eq!(wb.kind(), WorkbookKind::MacroEnabledWorkbook);
    assert!(matches!(
        wb.hygiene_log().last(),
        Some(HygieneAction::ChangedContentType { from, to, .. }) if from == CT_XLTM && to == CT_XLSM
    ));
    let out = wb.save().unwrap();
    assert_eq!(main_content_type(&out), CT_XLSM);
    assert_eq!(part(&out, "xl/vbaProject.bin").unwrap(), vba);
    assert_eq!(open(&out).kind(), WorkbookKind::MacroEnabledWorkbook);
    write_for_oracle("template-macro-instantiated.xlsm", &out);
}

#[test]
fn template_without_vba_part_instantiates_to_plain_even_for_xltm() {
    let mut wb = open(&with_main_type(&base(), CT_XLTM));
    assert!(!wb.has_vba_project());
    assert!(wb.instantiate_template().unwrap());
    assert_eq!(wb.kind(), WorkbookKind::Workbook);
}

#[test]
fn macro_enabled_workbook_is_not_a_template() {
    let mut wb = open(&with_main_type(&base(), CT_XLSM));
    assert!(!wb.kind().is_template());
    assert!(!wb.instantiate_template().unwrap());
    assert_eq!(wb.kind(), WorkbookKind::MacroEnabledWorkbook);
}

/// Saves `bytes` under `target/xlsx-template-out` so `office-oracle` can open them.
fn write_for_oracle(name: &str, bytes: &[u8]) {
    let dir = repo_root().join("target").join("xlsx-template-out");
    std::fs::create_dir_all(&dir).expect("output dir");
    std::fs::write(dir.join(name), bytes).expect("write output");
}
