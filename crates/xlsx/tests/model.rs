//! Workbook-model reads: tables, defined names, legacy array formulas and shared
//! formulas, on Excel-made fixtures (`fixtures/xlsx/model-*`) and synthetic sheets.

mod common;

use open365_opc::{Limits, Package};
use open365_xlsx::{ArrayRole, Workbook};

use common::{build, fixture, sheet_xml};

fn open(bytes: &[u8]) -> Workbook {
    Workbook::open(bytes, &Limits::default()).expect("opens")
}

#[test]
fn model_fixtures_round_trip_byte_identical() {
    for name in ["model-tables", "model-arrays"] {
        let bytes = fixture(name);
        let original = Package::read(&bytes, &Limits::default()).expect("package");
        let mut wb = open(&bytes);
        let saved = wb.save().expect("saves");
        let again = Package::read(&saved, &Limits::default()).expect("package");
        let a: Vec<_> = original.parts().collect();
        let b: Vec<_> = again.parts().collect();
        assert_eq!(a.len(), b.len(), "{name}");
        for (x, y) in a.iter().zip(&b) {
            assert!(
                x.name() == y.name() && x.data() == y.data(),
                "{name}: {}",
                x.name()
            );
        }
    }
}

#[test]
fn tables_are_read_from_the_sheet_relationships() {
    let wb = open(&fixture("model-tables"));
    let all: Vec<_> = wb.tables().collect();
    assert_eq!(all.len(), 2);
    let (sheet, t) = wb.table_by_name("salestbl").expect("table");
    assert_eq!(sheet, 0);
    assert_eq!(
        (t.name.as_str(), t.display_name.as_str()),
        ("SalesTbl", "SalesTbl")
    );
    assert_eq!(t.range_text, "A1:D7");
    assert_eq!(t.rect, Some((1, 1, 7, 4)));
    assert_eq!((t.header_row_count, t.totals_row_count), (1, 1));
    assert_eq!(t.data_rect(), Some((2, 1, 6, 4)));
    assert_eq!(t.data_row_count(), Some(5));
    let names: Vec<_> = t.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Item", "Qty", "Price", "Total"]);
    assert_eq!(t.columns[1].totals_row_function.as_deref(), Some("sum"));
    assert_eq!(t.columns[0].totals_row_function, None);
    let total = &t.columns[3];
    assert_eq!(
        total.calculated_column_formula.as_deref(),
        Some("SalesTbl[[#This Row],[Qty]]*SalesTbl[[#This Row],[Price]]")
    );
    let style = t.style.as_ref().expect("style");
    assert_eq!(style.name.as_deref(), Some("TableStyleMedium2"));
    assert!(style.show_row_stripes && !style.show_column_stripes);
    assert_eq!(t.column("QTY").map(|(i, _)| i), Some(2));

    let (sheet, lookup) = wb.table_by_name("Lookup").expect("second table");
    assert_eq!(sheet, 1);
    assert_eq!((lookup.header_row_count, lookup.totals_row_count), (1, 0));
    let s = lookup.style.as_ref().expect("style");
    assert!(s.show_first_column && !s.show_row_stripes);

    let ws = wb.sheet(0).expect("sheet");
    assert_eq!(ws.tables().len(), 1);
    assert_eq!(ws.table_at(3, 2).map(|t| t.name.as_str()), Some("SalesTbl"));
    assert!(ws.table_at(9, 1).is_none());
    assert!(wb.table_by_name("nope").is_none());
}

#[test]
fn defined_names_carry_scope_hidden_and_comment() {
    let wb = open(&fixture("model-tables"));
    let names = wb.defined_names();
    assert_eq!(names.len(), 3);
    let local = names.iter().find(|n| n.name == "LocalBase").expect("local");
    assert_eq!(local.local_sheet_id, Some(1));
    assert!(local.is_sheet_scoped());
    assert_eq!(local.comment.as_deref(), Some("sheet scoped note"));
    assert_eq!(local.formula, "Other!$B$2");
    let hidden = names
        .iter()
        .find(|n| n.name == "SecretBase")
        .expect("hidden");
    assert!(hidden.hidden && hidden.comment.is_none() && hidden.local_sheet_id.is_none());
    assert_eq!(wb.sheet_defined_names(1).count(), 1);
    assert_eq!(wb.sheet_defined_names(0).count(), 0);
    // Sheet scope wins on its own sheet and is invisible from the other.
    assert_eq!(
        wb.resolve_defined_name(1, "localbase")
            .map(|n| n.formula.as_str()),
        Some("Other!$B$2")
    );
    assert!(wb.resolve_defined_name(0, "LocalBase").is_none());
    assert_eq!(
        wb.resolve_defined_name(0, "taxrate")
            .map(|n| n.formula.as_str()),
        Some("0.2")
    );
}

#[test]
fn legacy_array_formulas_expose_master_range_and_members() {
    let wb = open(&fixture("model-arrays"));
    let ws = wb.sheet(0).expect("sheet");
    let arrays = ws.array_formulas();
    assert_eq!(arrays.len(), 3);
    assert_eq!(arrays[0].master, (4, 4));
    assert_eq!(arrays[0].range, (4, 4, 4, 6));
    assert_eq!(arrays[0].text, "A1:C1*2");
    assert!(!arrays[0].dynamic);
    assert_eq!(arrays[1].range, (6, 1, 8, 1));
    assert_eq!(arrays[2].range, (10, 2, 10, 2));
    assert_eq!(ws.array_role(4, 4), Some(ArrayRole::Master));
    assert_eq!(
        ws.array_role(4, 6),
        Some(ArrayRole::Member { master: (4, 4) })
    );
    assert_eq!(
        ws.array_role(8, 1),
        Some(ArrayRole::Member { master: (6, 1) })
    );
    assert_eq!(ws.array_role(5, 4), None);
    assert_eq!(ws.formula_text(4, 4).as_deref(), Some("A1:C1*2"));
    assert_eq!(ws.formula_text(4, 5), None);
}

#[test]
fn empty_f_members_of_an_array_have_no_formula_text() {
    let data = r#"<sheetData><row r="4"><c r="D4"><f t="array" ref="D4:F4">A1:C1*2</f><v>2</v></c><c r="E4"><f/><v>4</v></c><c r="F4"><f></f><v>6</v></c><c r="G4"><f/><v>1</v></c></row></sheetData>"#;
    let bytes = build(&sheet_xml("", data, ""), &[]);
    let mut wb = open(&bytes);
    let ws = wb.sheet(0).expect("sheet");
    assert_eq!(ws.formula_text(4, 4).as_deref(), Some("A1:C1*2"));
    assert_eq!(ws.formula_text(4, 5), None);
    assert_eq!(ws.formula_text(4, 6), None);
    assert_eq!(wb.formula_text(0, 4, 6), None);
    assert_eq!(
        ws.array_role(4, 5),
        Some(ArrayRole::Member { master: (4, 4) })
    );
    // Outside the array an empty f is untouched.
    assert_eq!(ws.formula_text(4, 7).as_deref(), Some(""));
    // Round trip keeps the empty f elements.
    let saved = wb.save().expect("saves");
    let pkg = Package::read(&saved, &Limits::default()).expect("package");
    let text = String::from_utf8(
        pkg.part("/xl/worksheets/sheet1.xml")
            .or_else(|| pkg.part("xl/worksheets/sheet1.xml"))
            .expect("sheet")
            .data()
            .to_vec(),
    )
    .expect("utf-8");
    assert!(text.contains("<c r=\"E4\"><f/><v>4</v></c>"), "{text}");
}

#[test]
fn shared_formulas_expose_master_and_offsets() {
    let wb = open(&fixture("model-arrays"));
    let ws = wb.sheet(0).expect("sheet");
    let m = ws.shared_formula(12, 2).expect("master");
    assert!(m.is_master);
    assert_eq!((m.master, m.master_text.as_str()), ((12, 2), "A12*2+1"));
    assert_eq!(m.master_range, Some((12, 2, 17, 2)));
    assert_eq!((m.row_offset, m.col_offset), (0, 0));
    let d = ws.shared_formula(15, 2).expect("dependent");
    assert!(!d.is_master);
    assert_eq!((d.si, d.master), (m.si, (12, 2)));
    assert_eq!((d.row_offset, d.col_offset), (3, 0));
    assert_eq!(ws.formula_text(15, 2).as_deref(), Some("A15*2+1"));
    assert!(ws.shared_formula(4, 4).is_none());
}
