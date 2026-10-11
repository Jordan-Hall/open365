//! Data validation against Excel: the `dataValidations` element Excel saved for each scenario of
//! `fixtures/reference/xlsx/sortfilter-validation/scenarios.json` is read, and every candidate value is checked
//! the way Excel's `Validation.Value` answered.

use std::path::PathBuf;

use open365_calc::{FormatLocale, Value as Calc};
use open365_opc::Limits;
use open365_xlsx::{
    DataValidation, DataValidations, ErrorStyle, ValidationContext, ValidationType, Workbook,
    WorkbookCells, parse_cell_ref, parse_xml,
};
use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load() -> (Workbook, Value) {
    let bytes =
        std::fs::read(root().join("fixtures/xlsx/sortfilter-validation.xlsx")).expect("fixture");
    let wb = Workbook::open(&bytes, &Limits::default()).expect("opens");
    let json = std::fs::read_to_string(
        root().join("fixtures/reference/xlsx/sortfilter-validation/scenarios.json"),
    )
    .expect("reference");
    (
        wb,
        serde_json::from_str(json.trim_start_matches('\u{feff}')).expect("json"),
    )
}

fn wrap(snippet: &str) -> Vec<u8> {
    format!(
        "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">{snippet}</worksheet>"
    )
    .into_bytes()
}

fn candidate(c: &Value) -> Calc {
    match c["type"].as_str() {
        Some("n") => Calc::Number(
            c["value"]
                .as_f64()
                .or_else(|| c["value"].as_str().and_then(|s| s.parse().ok()))
                .unwrap_or_else(|| panic!("number in {c}")),
        ),
        Some("s") => Calc::Text(c["value"].as_str().unwrap_or_default().to_owned()),
        Some("b") => Calc::Bool(c["value"] == true),
        _ => Calc::Empty,
    }
}

#[test]
fn validation_answers_match_excel() {
    let (wb, json) = load();
    let fmt = FormatLocale::en_us();
    let scenarios = json["scenarios"].as_array().expect("scenarios");
    let (mut asked, mut agreed, mut failures) = (0, 0, Vec::new());
    for sc in scenarios {
        let id = sc["id"].as_str().expect("id");
        let parsed = parse_xml(&wrap(sc["data_validations"].as_str().expect("xml"))).expect("xml");
        let set =
            DataValidations::from_element(parsed.root.child_main("dataValidations").expect("dv"));
        let rule = &set.items[0];
        let cells = WorkbookCells(&wb);
        let ctx = ValidationContext::new(&cells, id, &fmt);
        for c in sc["candidates"].as_array().expect("candidates") {
            let (row, col) = parse_cell_ref(c["cell"].as_str().expect("cell")).expect("cell");
            let got = rule.check(row, col, &candidate(c), &ctx).is_valid();
            asked += 1;
            if got == (c["valid"] == true) {
                agreed += 1;
            } else {
                eprintln!("{id} {c}: got {got}");
                failures.push(id.to_owned());
            }
        }
    }
    eprintln!("validation: {agreed}/{asked} answers match Excel");
    assert!(failures.is_empty(), "differs from Excel: {failures:?}");
}

#[test]
fn rules_round_trip_through_the_sheet() {
    let (mut wb, json) = load();
    for sc in json["scenarios"].as_array().expect("scenarios") {
        let parsed = parse_xml(&wrap(sc["data_validations"].as_str().expect("xml"))).expect("xml");
        let el = parsed.root.child_main("dataValidations").expect("dv");
        let set = DataValidations::from_element(el);
        let sheet = wb
            .sheet_index(sc["id"].as_str().expect("id"))
            .expect("sheet");
        wb.set_data_validations(sheet, Some(&set)).expect("sets");
        let again = wb
            .sheet(sheet)
            .expect("sheet")
            .data_validations()
            .expect("rules");
        assert_eq!(again, set, "{}", sc["id"]);
    }
    let bytes = wb.save().expect("saves");
    let wb2 = Workbook::open(&bytes, &Limits::default()).expect("reopens");
    let sheet = wb2.sheet_index("messages-stop").expect("sheet");
    let rules = wb2
        .sheet(sheet)
        .expect("sheet")
        .data_validations()
        .expect("rules");
    let rule: &DataValidation = &rules.items[0];
    assert_eq!(rule.kind, ValidationType::Whole);
    assert_eq!(rule.error_style, ErrorStyle::Stop);
    assert_eq!(rule.error.as_deref(), Some("Msg err & <more>"));
    assert_eq!(rule.prompt_title.as_deref(), Some("Title in"));
}

#[test]
fn the_workbook_checks_an_entry_against_the_rule_at_the_cell() {
    let (mut wb, _) = load();
    let sheet = wb.sheet_index("whole-between").expect("sheet");
    let mut rule = DataValidation::default();
    rule.kind = ValidationType::Whole;
    rule.formula1 = Some("1".to_owned());
    rule.formula2 = Some("10".to_owned());
    rule.sqref = vec![(1, 1, 3, 1)];
    let set = DataValidations {
        items: vec![rule],
        ..DataValidations::default()
    };
    wb.set_data_validations(sheet, Some(&set)).expect("sets");
    let fmt = FormatLocale::en_us();
    let clock = open365_calc::SystemClock;
    let check = |v: Calc, row| wb.validate_cell(sheet, row, 1, &v, &fmt, &clock);
    assert!(check(Calc::Number(5.0), 2).expect("rule").is_valid());
    assert!(!check(Calc::Number(50.0), 2).expect("rule").is_valid());
    assert!(check(Calc::Number(50.0), 9).is_none());
}
