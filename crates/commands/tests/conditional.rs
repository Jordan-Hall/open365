//! Conditional formatting display against Excel: `fixtures/reference/xlsx/grid-cf/displayformat.json`
//! holds, per cell of `fixtures/xlsx/grid-cf.xlsx`, what Excel shows through `Range.Text` and
//! `Range.DisplayFormat` (fill, font colour and style, edge borders; colours as BGR integers).
//! Every rule type Excel exposes there must match on every cell, with no tolerance. Data bars and
//! icon sets have no `DisplayFormat` equivalent; they are checked for their documented shape.

use std::path::PathBuf;

use open365_calc::Value;
use open365_calc::numfmt::format_value;
use open365_commands::{CfEffect, ConditionalFormats};
use open365_opc::Limits;
use open365_xlsx::{BorderEdge, BorderStyle, CellValue, Underline, Workbook, parse_cell_ref};
use serde_json::Value as Json;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn workbook() -> Workbook {
    let bytes = std::fs::read(root().join("fixtures/xlsx/grid-cf.xlsx")).expect("fixture");
    Workbook::open(&bytes, &Limits::default()).expect("opens")
}

fn reference() -> Json {
    let path = root().join("fixtures/reference/xlsx/grid-cf/displayformat.json");
    serde_json::from_slice(&std::fs::read(path).expect("reference")).expect("json")
}

fn shown_text(wb: &Workbook, fx: Option<&CfEffect>, row: u32, col: u32) -> String {
    let Some(cell) = wb.cell(0, row, col) else {
        return String::new();
    };
    let code = fx
        .and_then(|e| e.dxf.num_fmt.clone())
        .unwrap_or_else(|| wb.number_format(0, row, col).to_owned());
    match &cell.value {
        CellValue::Empty => String::new(),
        CellValue::Number(n) => format_value(&Value::Number(*n), &code, wb.date1904()),
        CellValue::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_owned(),
        CellValue::Error(e) => e.code().to_owned(),
        other => other.as_text().unwrap_or_default().to_owned(),
    }
}

fn edge_pair(wb: &Workbook, e: Option<BorderEdge>) -> (i64, i64) {
    match e {
        Some(e) if e.style != BorderStyle::None => {
            (1, i64::from(wb.styles().resolve_color(&e.color).bgr_u32()))
        }
        _ => (-4142, 0),
    }
}

fn border(wb: &Workbook, cf: &ConditionalFormats, row: u32, col: u32, side: char) -> (i64, i64) {
    let own = |r: u32, c: u32, s: char| -> Option<BorderEdge> {
        let fx = cf.effect(wb, r, c)?;
        match s {
            'l' => fx.dxf.border.left,
            'r' => fx.dxf.border.right,
            't' => fx.dxf.border.top,
            _ => fx.dxf.border.bottom,
        }
    };
    let (mine, theirs) = match side {
        'l' => (
            own(row, col, 'l'),
            col.checked_sub(1).and_then(|c| own(row, c, 'r')),
        ),
        'r' => (own(row, col, 'r'), own(row, col + 1, 'l')),
        't' => (
            own(row, col, 't'),
            row.checked_sub(1).and_then(|r| own(r, col, 'b')),
        ),
        _ => (own(row, col, 'b'), own(row + 1, col, 't')),
    };
    edge_pair(wb, mine.or(theirs))
}

/// Columns whose rule has no `DisplayFormat` equivalent (data bar, icon sets).
fn unobservable(col: u32) -> bool {
    matches!(col, 36..=38)
}

#[test]
fn every_rule_type_matches_excel_display() {
    let wb = workbook();
    let reference = reference();
    let today = reference["today"].as_f64().expect("today");
    let cf = ConditionalFormats::new(&wb, 0, Some(today));
    let mut compared = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for cell in reference["cells"].as_array().expect("cells") {
        let at = cell["ref"].as_str().expect("ref");
        let (row, col) = parse_cell_ref(at).expect("address");
        let fx = cf.effect(&wb, row, col);
        let fxr = fx.as_deref();
        let mut bad = Vec::new();

        let text = shown_text(&wb, fxr, row, col);
        if text != cell["text"].as_str().unwrap() {
            bad.push(format!("text {text:?} != {:?}", cell["text"]));
        }
        if unobservable(col) {
            compared += 1;
            if !bad.is_empty() {
                failures.push(format!("{at}: {}", bad.join("; ")));
            }
            continue;
        }
        let fill = fxr.and_then(|e| {
            e.scale_fill.map(|c| c.bgr_u32()).or_else(|| {
                e.dxf
                    .fill
                    .as_ref()
                    .and_then(|f| f.solid_color())
                    .map(|c| wb.styles().resolve_color(&c).bgr_u32())
            })
        });
        let want_fill = (cell["pattern"].as_i64().unwrap() != -4142)
            .then(|| u32::try_from(cell["fill"].as_i64().unwrap()).unwrap());
        if fill != want_fill {
            bad.push(format!("fill {fill:?} != {want_fill:?}"));
        }
        let font = fxr
            .and_then(|e| e.dxf.font.color)
            .map_or(0, |c| i64::from(wb.styles().resolve_color(&c).bgr_u32()));
        if font != cell["font_color"].as_i64().unwrap() {
            bad.push(format!("font colour {font} != {}", cell["font_color"]));
        }
        let flag = |get: fn(&CfEffect) -> Option<bool>| fxr.and_then(get).unwrap_or(false);
        for (name, got) in [
            ("bold", flag(|e| e.dxf.font.bold)),
            ("italic", flag(|e| e.dxf.font.italic)),
            ("strike", flag(|e| e.dxf.font.strike)),
        ] {
            if got != cell[name].as_bool().unwrap() {
                bad.push(format!("{name} {got} != {}", cell[name]));
            }
        }
        let underline = match fxr.and_then(|e| e.dxf.font.underline) {
            Some(Underline::Single) => 2,
            Some(Underline::Double) => -4119,
            _ => -4142,
        };
        if underline != cell["underline"].as_i64().unwrap() {
            bad.push(format!("underline {underline} != {}", cell["underline"]));
        }
        for (key, side) in [("left", 'l'), ("top", 't'), ("bottom", 'b'), ("right", 'r')] {
            let got = border(&wb, &cf, row, col, side);
            let want = (
                cell["borders"][key][0].as_i64().unwrap(),
                cell["borders"][key][1].as_i64().unwrap(),
            );
            if got != want {
                bad.push(format!("{key} border {got:?} != {want:?}"));
            }
        }
        compared += 1;
        if !bad.is_empty() {
            failures.push(format!("{at}: {}", bad.join("; ")));
        }
    }
    assert!(compared >= 500, "only {compared} cells compared");
    assert!(
        failures.is_empty(),
        "{} of {compared} cells differ from Excel:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn data_bars_and_icons_follow_the_documented_shape() {
    let wb = workbook();
    let cf = ConditionalFormats::new(&wb, 0, None);
    // Column AJ (36): data bar over 3..99; the lowest value has the shortest bar, the highest the longest.
    let low = cf.effect(&wb, 5, 36).expect("bar on 3");
    let high = cf.effect(&wb, 9, 36).expect("bar on 99");
    let (low, high) = (
        low.data_bar.clone().unwrap(),
        high.data_bar.clone().unwrap(),
    );
    assert_eq!(low.fraction, 0.0);
    assert_eq!(high.fraction, 1.0);
    // Column AK (37): 3Arrows with percent thresholds 0, 33, 67.
    let icon = |row| cf.effect(&wb, row, 37).unwrap().icon.clone().unwrap();
    assert_eq!(icon(5).index, 0, "3 is in the lowest band");
    assert_eq!(icon(9).index, 2, "99 is in the highest band");
    assert_eq!(icon(5).count, 3);
    assert_eq!(
        cf.effect(&wb, 9, 38).unwrap().icon.clone().unwrap().count,
        5
    );
}

#[test]
fn a_sheet_without_rules_has_no_effects() {
    let bytes = std::fs::read(root().join("fixtures/xlsx/formulas-basic.xlsx")).expect("fixture");
    let wb = Workbook::open(&bytes, &Limits::default()).unwrap();
    let cf = ConditionalFormats::new(&wb, 0, None);
    assert!(cf.is_empty());
    assert!(cf.effect(&wb, 1, 1).is_none());
}
