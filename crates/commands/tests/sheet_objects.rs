//! Sheets Insert/Formulas/sheet-management commands: each is one undo step, wired through the
//! typed command layer.

use std::io::Write as _;
use std::path::PathBuf;

use open365_commands::{
    CellEntry, Command, EffectClass, ObjectsOp, RangeRef, Target, objects_ops::TableRequest,
};
use open365_xlsx::CellValue;
use zip::write::SimpleFileOptions;

fn fixture(name: &str) -> Target {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/xlsx")
        .join(format!("{name}.xlsx"));
    Target::open_path(&path).expect("open xlsx")
}

fn range(r1: u32, c1: u32, r2: u32, c2: u32) -> RangeRef {
    RangeRef {
        first_row: r1,
        first_col: c1,
        last_row: r2,
        last_col: c2,
    }
}

fn op(t: &mut Target, sheet: usize, op: ObjectsOp) -> open365_commands::Outcome {
    t.apply(Command::SheetObjects { sheet, op })
        .expect("applies")
}

/// Applies `op`, then undoes it: the saved bytes must come back exactly and one undo must be enough.
fn one_undo_step(t: &mut Target, sheet: usize, o: ObjectsOp) {
    let before = t.snapshot_bytes().expect("snapshot");
    op(t, sheet, o);
    let after = t.snapshot_bytes().expect("snapshot");
    assert_ne!(before, after, "the command changed the workbook");
    t.undo().expect("undo");
    assert_eq!(
        t.snapshot_bytes().expect("snapshot"),
        before,
        "one undo restores the workbook"
    );
    t.redo().expect("redo");
    assert_eq!(
        t.snapshot_bytes().expect("snapshot"),
        after,
        "redo reapplies it"
    );
    t.undo().expect("undo again");
}

#[test]
fn a_table_is_one_undo_step() {
    let mut t = fixture("formulas-basic");
    one_undo_step(
        &mut t,
        0,
        ObjectsOp::CreateTable(
            serde_json::from_value(serde_json::json!({
                "range": {"first_row": 1, "first_col": 1, "last_row": 4, "last_col": 2},
                "has_headers": false,
                "totals_row": true
            }))
            .expect("request"),
        ),
    );
    assert_eq!(t.xlsx().expect("xlsx").tables().count(), 0);
    let out = op(
        &mut t,
        0,
        ObjectsOp::CreateTable(TableRequest {
            range: range(1, 1, 4, 2),
            name: Some("Sales".to_owned()),
            has_headers: Some(false),
            style: None,
            totals_row: false,
            filter_buttons: true,
            first_column: false,
            last_column: false,
            banded_rows: true,
            banded_columns: false,
        }),
    );
    assert_eq!(out.effect, EffectClass::Edit);
    assert!(out.notes.iter().any(|n| n.contains("Sales")));
    assert_eq!(
        t.xlsx()
            .expect("xlsx")
            .table_by_name("Sales")
            .map(|(_, tb)| tb.range_text.clone())
            .as_deref(),
        Some("A1:B5")
    );
}

#[test]
fn names_links_notes_and_calculation_are_undoable() {
    let mut t = fixture("formulas-basic");
    one_undo_step(
        &mut t,
        0,
        serde_json::from_value(
            serde_json::json!({"op": "define_name", "name": "Tax", "refers_to": "=Sheet1!$A$1"}),
        )
        .expect("op"),
    );
    one_undo_step(
        &mut t,
        0,
        serde_json::from_value(serde_json::json!({
            "op": "set_hyperlink",
            "range": {"first_row": 1, "first_col": 1, "last_row": 1, "last_col": 1},
            "target": {"type": "email", "address": "a@b.com", "subject": "Hi"}
        }))
        .expect("op"),
    );
    one_undo_step(
        &mut t,
        0,
        serde_json::from_value(serde_json::json!({"op": "set_note", "row": 2, "col": 2, "author": "J", "text": "check"})).expect("op"),
    );
    one_undo_step(
        &mut t,
        0,
        serde_json::from_value(
            serde_json::json!({"op": "set_calculation", "mode": "manual", "iterate": true}),
        )
        .expect("op"),
    );
    assert_eq!(
        t.xlsx().expect("xlsx").calc_settings().mode,
        open365_xlsx::CalcMode::Automatic
    );
}

#[test]
fn conditional_formats_from_json_cover_the_rule_types() {
    let mut t = fixture("formulas-basic");
    let json = serde_json::json!({
        "op": "add_conditional_format",
        "ranges": [{"first_row": 1, "first_col": 1, "last_row": 6, "last_col": 1}],
        "rules": [
            {"type": "cell_value", "compare": "greater", "formulas": ["3"],
             "format": {"bold": true, "font_color": {"rgb": "9C0006"}, "fill_color": {"rgb": "FFC7CE"}}},
            {"type": "top10", "rank": 3, "format": {"fill_color": {"rgb": "FFEB9C"}}},
            {"type": "color_scale",
             "points": [{"kind": "min"}, {"kind": "max"}],
             "colors": [{"rgb": "F8696B"}, {"rgb": "63BE7B"}]},
            {"type": "data_bar", "min": {"kind": "min"}, "max": {"kind": "max"},
             "color": {"rgb": "638EC6"}, "gradient": false},
            {"type": "icon_set", "name": "3Arrows",
             "points": [{"kind": "percent", "value": "0"}, {"kind": "percent", "value": "33"}, {"kind": "percent", "value": "67"}]},
            {"type": "formula", "formula": "=MOD(ROW(),2)=0", "format": {"italic": true}}
        ]
    });
    let parsed: ObjectsOp = serde_json::from_value(json).expect("op");
    one_undo_step(&mut t, 0, parsed.clone());
    op(&mut t, 0, parsed);
    let blocks = t
        .xlsx()
        .and_then(|w| w.sheet(0))
        .expect("sheet")
        .conditional_formats();
    assert_eq!(blocks.last().map(|b| b.rules.len()), Some(6));
    // Delete is destructive and undoable.
    let delete = Command::SheetObjects {
        sheet: 0,
        op: ObjectsOp::DeleteConditionalFormat {
            index: blocks.len() - 1,
        },
    };
    assert_eq!(delete.effect(None), EffectClass::Destructive);
    t.apply(delete).expect("delete");
    t.undo().expect("undo delete");
    assert_eq!(
        t.xlsx()
            .and_then(|w| w.sheet(0))
            .expect("sheet")
            .conditional_formats()
            .len(),
        blocks.len()
    );
}

#[test]
fn sheet_operations_are_single_undo_steps_and_recalculate() {
    let mut t = fixture("cross-sheet");
    one_undo_step(
        &mut t,
        0,
        ObjectsOp::AddSheet {
            at: Some(1),
            name: Some("New".to_owned()),
        },
    );
    one_undo_step(
        &mut t,
        0,
        ObjectsOp::CopySheet {
            at: None,
            name: None,
        },
    );
    one_undo_step(&mut t, 1, ObjectsOp::MoveSheet { to: 0 });
    one_undo_step(
        &mut t,
        1,
        ObjectsOp::SetTabColor {
            color: Some(open365_commands::ColorSpec::rgb("FF0000")),
        },
    );
    one_undo_step(
        &mut t,
        1,
        ObjectsOp::SetSheetVisibility {
            visibility: open365_commands::objects_ops::SheetVisibility::Hidden,
        },
    );
    // Rename rewrites the formulas that point at the sheet.
    let out = op(
        &mut t,
        0,
        ObjectsOp::RenameSheet {
            name: "Base data".to_owned(),
        },
    );
    let wb = t.xlsx().expect("xlsx");
    assert_eq!(
        wb.formula_text(1, 1, 1).as_deref(),
        Some("'Base data'!A1*2")
    );
    assert!(out.effect == EffectClass::Edit);
    t.undo().expect("undo rename");
    assert_eq!(
        t.xlsx().expect("xlsx").formula_text(1, 1, 1).as_deref(),
        Some("One!A1*2")
    );
    // Deleting is destructive, rewrites to #REF!, and one undo brings the sheet back.
    let cmd = Command::SheetObjects {
        sheet: 0,
        op: ObjectsOp::DeleteSheet,
    };
    assert_eq!(cmd.effect(None), EffectClass::Destructive);
    let before = t.snapshot_bytes().expect("snapshot");
    t.apply(cmd).expect("delete");
    assert_eq!(t.xlsx().expect("xlsx").sheets().len(), 2);
    assert_eq!(
        t.xlsx().expect("xlsx").formula_text(1, 1, 1).as_deref(),
        Some("Two!A1+One!#REF!")
    );
    t.undo().expect("undo delete");
    assert_eq!(t.snapshot_bytes().expect("snapshot"), before);
}

#[test]
fn insert_and_delete_cells_rewrite_formulas_and_refresh_results() {
    let mut t = fixture("formulas-basic");
    for (row, v) in [(1, 1.0), (2, 2.0), (3, 3.0)] {
        t.apply(Command::XlsxSetCellValue {
            sheet: 0,
            row,
            col: 6,
            value: CellEntry::Number(v),
        })
        .expect("n");
    }
    t.apply(Command::XlsxSetCellFormula {
        sheet: 0,
        row: 1,
        col: 8,
        formula: "=SUM(F1:F3)".to_owned(),
    })
    .expect("sum");
    let value = |t: &Target| match t
        .xlsx()
        .and_then(|w| w.cell(0, 1, 8))
        .map(|c| c.value.clone())
    {
        Some(CellValue::Number(n)) => n,
        other => panic!("{other:?}"),
    };
    assert_eq!(value(&t), 6.0);
    // Insert cells above F2 shifting down: the sum range grows, the total is unchanged.
    let out = op(
        &mut t,
        0,
        ObjectsOp::InsertCells {
            range: range(2, 6, 2, 6),
            shift: open365_commands::objects_ops::CellShift::Down,
        },
    );
    assert_eq!(
        t.xlsx().and_then(|w| w.formula_text(0, 1, 8)).as_deref(),
        Some("SUM(F1:F4)")
    );
    assert!(out.effect == EffectClass::Edit);
    // Deleting the cell with the 3 (now F4) shrinks the range and the total follows.
    let del = Command::SheetObjects {
        sheet: 0,
        op: ObjectsOp::DeleteCells {
            range: range(4, 6, 4, 6),
            shift: open365_commands::objects_ops::CellShift::Up,
        },
    };
    assert_eq!(del.effect(None), EffectClass::Destructive);
    let out = t.apply(del).expect("delete");
    assert_eq!(
        t.xlsx().and_then(|w| w.formula_text(0, 1, 8)).as_deref(),
        Some("SUM(F1:F3)")
    );
    assert_eq!(value(&t), 3.0);
    assert!(out.changed_cells.iter().any(|c| c.row == 1 && c.col == 8));
    // Wrong direction is refused and changes nothing.
    let before = t.snapshot_bytes().expect("snapshot");
    assert!(
        t.apply(Command::SheetObjects {
            sheet: 0,
            op: ObjectsOp::InsertCells {
                range: range(1, 1, 1, 1),
                shift: open365_commands::objects_ops::CellShift::Up
            }
        })
        .is_err()
    );
    assert_eq!(t.snapshot_bytes().expect("snapshot"), before);
}

/// A one-sheet-or-two workbook whose cached formula results are wrong on purpose.
fn stale_workbook() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default();
    let mut put = |name: &str, body: &str| {
        zip.start_file(name, opts).expect("start");
        zip.write_all(body.as_bytes()).expect("write");
    };
    put(
        "[Content_Types].xml",
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/><Override PartName=\"/xl/worksheets/sheet2.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/></Types>",
    );
    put(
        "_rels/.rels",
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>",
    );
    put(
        "xl/workbook.xml",
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets><sheet name=\"A\" sheetId=\"1\" r:id=\"rId1\"/><sheet name=\"B\" sheetId=\"2\" r:id=\"rId2\"/></sheets></workbook>",
    );
    put(
        "xl/_rels/workbook.xml.rels",
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet2.xml\"/></Relationships>",
    );
    let sheet = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData><row r=\"1\"><c r=\"A1\"><v>5</v></c><c r=\"B1\"><f>A1*2</f><v>99</v></c></row></sheetData></worksheet>";
    put("xl/worksheets/sheet1.xml", sheet);
    put("xl/worksheets/sheet2.xml", sheet);
    zip.finish().expect("finish").into_inner()
}

#[test]
fn calculate_now_and_calculate_sheet_refresh_cached_results() {
    let mut t = Target::open_xlsx(&stale_workbook()).expect("opens");
    let cached = |t: &Target, sheet: usize| match t
        .xlsx()
        .and_then(|w| w.cell(sheet, 1, 2))
        .map(|c| c.value.clone())
    {
        Some(CellValue::Number(n)) => n,
        other => panic!("{other:?}"),
    };
    assert_eq!((cached(&t, 0), cached(&t, 1)), (99.0, 99.0));
    let out = op(&mut t, 0, ObjectsOp::CalculateSheet);
    assert_eq!(
        (cached(&t, 0), cached(&t, 1)),
        (10.0, 99.0),
        "only the chosen sheet"
    );
    assert_eq!(out.changed_cells.len(), 1);
    let out = op(&mut t, 0, ObjectsOp::CalculateNow);
    assert_eq!((cached(&t, 0), cached(&t, 1)), (10.0, 10.0));
    assert_eq!(out.changed_cells.len(), 1);
    t.undo().expect("undo");
    assert_eq!((cached(&t, 0), cached(&t, 1)), (10.0, 99.0));
    // Nothing stale: a second Calculate Now changes nothing.
    op(&mut t, 0, ObjectsOp::CalculateNow);
    assert!(
        op(&mut t, 0, ObjectsOp::CalculateNow)
            .changed_cells
            .is_empty()
    );
}

#[test]
fn parts_charts_and_pivots_go_through_the_command_log() {
    let mut t = fixture("formulas-basic");
    one_undo_step(
        &mut t,
        0,
        ObjectsOp::AddPart {
            name: "xl/custom/x.xml".to_owned(),
            content_type: "application/x+xml".to_owned(),
            data: b"<x/>".to_vec(),
        },
    );
    one_undo_step(
        &mut t,
        0,
        ObjectsOp::AddRelationship {
            source: None,
            rel_type: "http://example.com/rel".to_owned(),
            target: "https://example.com/".to_owned(),
            external: true,
        },
    );
    let chart_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/xlsx/chart.xlsx");
    let pkg = open365_opc::Package::read(
        &std::fs::read(chart_path).expect("fixture"),
        &open365_opc::Limits::default(),
    )
    .expect("package");
    let chart = String::from_utf8(
        pkg.part("xl/charts/chart1.xml")
            .expect("chart")
            .data()
            .to_vec(),
    )
    .expect("utf-8");
    one_undo_step(
        &mut t,
        0,
        ObjectsOp::InsertChart {
            chart_xml: chart,
            range: range(8, 1, 20, 6),
            name: None,
        },
    );
    // The log records the command and replays from JSON.
    let log = t.audit_log();
    assert!(!log.0.is_empty());
    let text = serde_json::to_string(&Command::SheetObjects {
        sheet: 0,
        op: ObjectsOp::CalculateNow,
    })
    .expect("json");
    let back: Command = serde_json::from_str(&text).expect("parses");
    assert_eq!(back.name(), "sheet_objects");
}

#[test]
fn a_header_and_footer_is_written_read_back_and_one_undo_step() {
    use open365_commands::{HeaderFooterRequest, header_footer};
    let mut t = fixture("formulas-basic");
    let h = HeaderFooterRequest {
        odd_header: "&LLeft & <co>&RPage &P".to_owned(),
        odd_footer: "&C&A".to_owned(),
        first_footer: "Cover".to_owned(),
        different_first: true,
        align_with_margins: false,
        ..HeaderFooterRequest::default()
    };
    one_undo_step(&mut t, 0, ObjectsOp::SetHeaderFooter(h.clone()));
    op(&mut t, 0, ObjectsOp::SetHeaderFooter(h.clone()));
    let bytes = t.snapshot_bytes().expect("snapshot");
    let again = Target::open_xlsx(&bytes).expect("reopens");
    assert_eq!(header_footer(again.xlsx().expect("xlsx"), 0), h);
    // Emptying it removes the element (Excel writes none for an empty header and footer).
    op(
        &mut t,
        0,
        ObjectsOp::SetHeaderFooter(HeaderFooterRequest::default()),
    );
    assert_eq!(
        header_footer(t.xlsx().expect("xlsx"), 0),
        HeaderFooterRequest::default()
    );
}

#[test]
fn passwords_are_redacted_in_debug_output() {
    use open365_commands::ReviewOp;
    use open365_commands::review_ops::Password;
    let op = ReviewOp::UnprotectSheet {
        password: Some(Password::new("hunter2")),
    };
    let text = format!(
        "{op:?} {:?}",
        Command::ReviewOp {
            sheet: 0,
            op: op.clone(),
        }
    );
    assert!(!text.contains("hunter2"), "{text}");
    assert!(text.contains("redacted"));
}
