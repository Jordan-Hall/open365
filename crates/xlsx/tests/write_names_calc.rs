//! Defined names (create, edit, delete, Create from Selection) and calculation options.

mod common;
mod write_support;

use open365_xlsx::{
    CalcMode, CalcSettings, CellInput, NameSource, NameSpec, Workbook, XlsxError,
    validate_defined_name,
};
use write_support::{fixture, open, reopen, text_of};

fn grid() -> Workbook {
    let mut wb = open(&common::build(
        &common::sheet_xml("", "<sheetData/>", ""),
        &[],
    ));
    for (r, c, t) in [(1, 1, "Qty"), (1, 2, "Price"), (2, 1, "x")] {
        wb.set_cell_value(0, r, c, CellInput::Text(t.to_owned()))
            .expect("text");
    }
    for (r, c, n) in [(2, 2, 5.0), (3, 2, 6.0), (3, 1, 1.0), (4, 1, 2.0)] {
        wb.set_cell_value(0, r, c, CellInput::Number(n))
            .expect("number");
    }
    wb
}

fn spec(name: &str, formula: &str) -> NameSpec {
    NameSpec {
        name: name.to_owned(),
        formula: formula.to_owned(),
        ..NameSpec::default()
    }
}

#[test]
fn workbook_and_sheet_scoped_names_are_written_sorted_like_excel() {
    let mut wb = fixture("formulas-basic");
    wb.define_name(&spec("Zeta", "Sheet1!$A$1"))
        .expect("workbook scope");
    wb.define_name(&NameSpec {
        scope: Some(0),
        comment: Some("note".to_owned()),
        ..spec("alpha", "Sheet1!$B$2:$B$3")
    })
    .expect("sheet scope");
    wb.define_name(&NameSpec {
        hidden: true,
        ..spec("Mid", "42")
    })
    .expect("constant");
    let book = text_of(&mut wb, "xl/workbook.xml");
    let a = book.find("name=\"alpha\"").expect("alpha");
    let m = book.find("name=\"Mid\"").expect("Mid");
    let z = book.find("name=\"Zeta\"").expect("Zeta");
    assert!(a < m && m < z, "{book}");
    assert!(book.contains("<definedName name=\"alpha\" comment=\"note\" localSheetId=\"0\">Sheet1!$B$2:$B$3</definedName>"), "{book}");
    assert!(
        book.contains("<definedName name=\"Mid\" hidden=\"1\">42</definedName>"),
        "{book}"
    );
    // definedNames sits between sheets and calcPr.
    let sheets_end = book.find("</sheets>").expect("sheets");
    let names = book.find("<definedNames").expect("names");
    assert!(sheets_end < names, "{book}");
    if let Some(calc) = book.find("<calcPr") {
        assert!(names < calc, "{book}");
    }
    let back = reopen(&mut wb);
    assert!(back.resolve_defined_name(0, "alpha").is_some());
    assert!(back.resolve_defined_name(0, "Zeta").is_some());
}

#[test]
fn names_are_validated_and_unique_per_scope() {
    for bad in ["", "1a", "A1", "XFD1048576", "R1C1", "R", "c", "a b", "a-b"] {
        assert!(validate_defined_name(bad).is_err(), "{bad:?}");
    }
    for good in ["_x", "Tax.Rate", r"\path", "ABC", "Sales_2020", "XFE1"] {
        assert!(validate_defined_name(good).is_ok(), "{good:?}");
    }
    let mut wb = fixture("formulas-basic");
    wb.define_name(&spec("Rate", "0.2")).expect("first");
    assert!(matches!(
        wb.define_name(&spec("RATE", "0.3")),
        Err(XlsxError::UnsupportedEdit(_))
    ));
    wb.define_name(&NameSpec {
        scope: Some(0),
        ..spec("Rate", "0.4")
    })
    .expect("same name, sheet scope");
    assert!(wb.define_name(&spec("Bad", "1+")).is_err());
}

#[test]
fn editing_a_name_renames_it_in_formulas_and_deleting_removes_it() {
    let mut wb = grid();
    wb.define_name(&spec("Price", "Sheet1!$B$2")).expect("name");
    wb.set_cell_formula(0, 6, 1, "Price*2", None)
        .expect("formula");
    let recalc = wb
        .edit_defined_name("Price", None, &spec("UnitPrice", "Sheet1!$B$3"))
        .expect("edit");
    assert_eq!(recalc, vec![(0, 6, 1)]);
    assert_eq!(wb.formula_text(0, 6, 1).as_deref(), Some("UnitPrice*2"));
    assert_eq!(
        wb.defined_names()
            .iter()
            .map(|n| (n.name.as_str(), n.formula.as_str()))
            .collect::<Vec<_>>(),
        vec![("UnitPrice", "Sheet1!$B$3")]
    );
    let scope_change = wb.edit_defined_name(
        "UnitPrice",
        None,
        &NameSpec {
            scope: Some(0),
            ..spec("UnitPrice", "1")
        },
    );
    assert!(scope_change.is_err());
    let gone = wb.delete_defined_name("UnitPrice", None).expect("delete");
    assert_eq!(gone.name, "UnitPrice");
    let book = text_of(&mut wb, "xl/workbook.xml");
    assert!(!book.contains("definedName"), "{book}");
    assert!(wb.delete_defined_name("UnitPrice", None).is_err());
}

#[test]
fn create_from_selection_names_columns_and_rows_from_their_labels() {
    let mut wb = grid();
    let made = wb
        .create_names_from_selection(
            0,
            (1, 1, 3, 2),
            NameSource {
                top: true,
                ..NameSource::default()
            },
            false,
        )
        .expect("top row");
    let got: Vec<(String, String)> = made
        .iter()
        .map(|n| (n.name.clone(), n.formula.clone()))
        .collect();
    assert_eq!(
        got,
        vec![
            ("Qty".to_owned(), "Sheet1!$A$2:$A$3".to_owned()),
            ("Price".to_owned(), "Sheet1!$B$2:$B$3".to_owned())
        ]
    );
    // A second run refuses unless it may replace.
    assert!(
        wb.create_names_from_selection(
            0,
            (1, 1, 3, 2),
            NameSource {
                top: true,
                ..NameSource::default()
            },
            false
        )
        .is_err()
    );
    wb.create_names_from_selection(
        0,
        (1, 1, 3, 2),
        NameSource {
            top: true,
            ..NameSource::default()
        },
        true,
    )
    .expect("replace");
    assert_eq!(wb.defined_names().len(), 2);
    // Left column: the label names the cells to its right.
    let mut wb = grid();
    wb.set_cell_value(0, 5, 1, CellInput::Text("Total row".to_owned()))
        .expect("label");
    wb.set_cell_value(0, 5, 2, CellInput::Number(1.0))
        .expect("n");
    let made = wb
        .create_names_from_selection(
            0,
            (5, 1, 5, 2),
            NameSource {
                left: true,
                ..NameSource::default()
            },
            false,
        )
        .expect("left");
    assert_eq!(made.len(), 1);
    assert_eq!(
        (made[0].name.as_str(), made[0].formula.as_str()),
        ("Total_row", "Sheet1!$B$5")
    );
    assert!(
        wb.create_names_from_selection(0, (1, 1, 2, 2), NameSource::default(), false)
            .is_err()
    );
}

#[test]
fn calculation_mode_and_flags_are_written_to_calcpr() {
    let mut wb = fixture("formulas-basic");
    assert_eq!(wb.calc_settings().mode, CalcMode::Automatic);
    wb.set_calc_mode(CalcMode::Manual).expect("manual");
    let book = text_of(&mut wb, "xl/workbook.xml");
    assert!(book.contains("calcMode=\"manual\""), "{book}");
    wb.set_calc_mode(CalcMode::AutomaticExceptTables)
        .expect("no tables");
    assert!(text_of(&mut wb, "xl/workbook.xml").contains("calcMode=\"autoNoTable\""));
    wb.set_calc_mode(CalcMode::Automatic).expect("auto");
    assert!(!text_of(&mut wb, "xl/workbook.xml").contains("calcMode"));
    wb.request_full_calc_on_load(true).expect("flag");
    assert!(text_of(&mut wb, "xl/workbook.xml").contains("fullCalcOnLoad=\"1\""));
    wb.request_full_calc_on_load(false).expect("clear");
    assert!(!text_of(&mut wb, "xl/workbook.xml").contains("fullCalcOnLoad"));
    let s = CalcSettings {
        iterate: true,
        iterate_count: 50,
        iterate_delta: 0.01,
        calc_on_save: false,
        full_precision: false,
        ..CalcSettings::default()
    };
    wb.set_calc_settings(&s).expect("iteration");
    let back = reopen(&mut wb);
    assert_eq!(back.calc_settings(), s);
    assert!(
        wb.set_calc_settings(&CalcSettings {
            iterate: true,
            iterate_count: 0,
            ..CalcSettings::default()
        })
        .is_err()
    );
}

#[test]
fn a_workbook_without_calcpr_gets_one_in_schema_order() {
    let mut wb = open(&common::build(
        &common::sheet_xml("", "<sheetData/>", ""),
        &[],
    ));
    wb.set_calc_mode(CalcMode::Manual).expect("manual");
    let book = text_of(&mut wb, "xl/workbook.xml");
    assert!(
        book.contains("<calcPr calcId=\"191029\" calcMode=\"manual\"/>"),
        "{book}"
    );
    assert!(
        book.find("</sheets>").expect("sheets") < book.find("<calcPr").expect("calc"),
        "{book}"
    );
}
