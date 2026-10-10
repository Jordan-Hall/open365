//! Cell formatting against Excel: `fixtures/reference/xlsx/<name>/styles.json` holds, per cell,
//! the font, fill, border and alignment Excel reports through COM (colours as resolved BGR
//! integers). The accessors must give the same values, with no tolerance, and reading them
//! must not change the saved bytes.

mod common;

use open365_opc::Limits;
use open365_xlsx::{
    BorderStyle, FillPattern, HorizontalAlignment, Underline, VerticalAlignment, Workbook,
    parse_cell_ref,
};
use serde_json::Value;

use common::{fixture, repo_root};

fn styles_json(name: &str) -> Value {
    let path = repo_root()
        .join("fixtures")
        .join("reference")
        .join("xlsx")
        .join(name)
        .join("styles.json");
    serde_json::from_slice(&std::fs::read(&path).expect("styles.json exists"))
        .expect("styles.json parses")
}

fn excel_pattern(p: FillPattern) -> i64 {
    match p {
        FillPattern::None => -4142,
        FillPattern::Solid => 1,
        FillPattern::MediumGray => -4125,
        FillPattern::DarkGray => -4126,
        FillPattern::LightGray => -4124,
        FillPattern::Gray125 => 17,
        FillPattern::Gray0625 => 18,
        FillPattern::DarkHorizontal => -4128,
        FillPattern::DarkVertical => -4166,
        FillPattern::DarkDown => -4121,
        FillPattern::DarkUp => -4162,
        FillPattern::DarkGrid => 9,
        FillPattern::DarkTrellis => 10,
        FillPattern::LightHorizontal => 11,
        FillPattern::LightVertical => 12,
        FillPattern::LightDown => 13,
        FillPattern::LightUp => 14,
        FillPattern::LightGrid => 15,
        FillPattern::LightTrellis => 16,
    }
}

/// Excel's `(LineStyle, Weight)` for a border style.
fn excel_line(style: BorderStyle) -> (i64, i64) {
    match style {
        BorderStyle::None => (-4142, 0),
        BorderStyle::Hair => (1, 1),
        BorderStyle::Thin => (1, 2),
        BorderStyle::Medium => (1, -4138),
        BorderStyle::Thick => (1, 4),
        BorderStyle::Dashed => (-4115, 2),
        BorderStyle::Dotted => (-4118, 2),
        BorderStyle::Double => (-4119, 4),
        BorderStyle::DashDot => (4, 2),
        BorderStyle::DashDotDot => (5, 2),
        BorderStyle::MediumDashed => (-4115, -4138),
        BorderStyle::MediumDashDot => (4, -4138),
        BorderStyle::MediumDashDotDot => (5, -4138),
        BorderStyle::SlantDashDot => (13, -4138),
    }
}

fn excel_h(h: HorizontalAlignment) -> i64 {
    match h {
        HorizontalAlignment::General => 1,
        HorizontalAlignment::Left => -4131,
        HorizontalAlignment::Center => -4108,
        HorizontalAlignment::Right => -4152,
        HorizontalAlignment::Fill => 5,
        HorizontalAlignment::Justify => -4130,
        HorizontalAlignment::CenterContinuous => 7,
        HorizontalAlignment::Distributed => -4117,
    }
}

fn excel_v(v: VerticalAlignment) -> i64 {
    match v {
        VerticalAlignment::Top => -4160,
        VerticalAlignment::Center => -4108,
        VerticalAlignment::Bottom => -4107,
        VerticalAlignment::Justify => -4130,
        VerticalAlignment::Distributed => -4117,
    }
}

fn excel_underline(u: Underline) -> i64 {
    match u {
        Underline::None => -4142,
        Underline::Single => 2,
        Underline::Double => -4119,
        Underline::SingleAccounting => 4,
        Underline::DoubleAccounting => 5,
    }
}

#[test]
fn resolved_formatting_equals_excel() {
    let bytes = fixture("grid-styles");
    let wb = Workbook::open(&bytes, &Limits::default()).expect("opens");
    let json = styles_json("grid-styles");
    let styles = wb.styles();
    let (mut matched, mut total) = (0usize, 0usize);
    let mut failures: Vec<String> = Vec::new();
    for sheet in json["sheets"].as_array().expect("sheets") {
        let sheet_name = sheet["name"].as_str().expect("name");
        let index = wb
            .sheets()
            .iter()
            .position(|s| s.name == sheet_name)
            .expect("sheet exists");
        let ws = wb.sheet(index).expect("worksheet");
        let col_styles = ws.column_styles();
        for (address, want) in sheet["cells"].as_object().expect("cells") {
            let (row, col) = parse_cell_ref(address).expect("address");
            let xf = ws.resolved_style(row, col, &col_styles);
            let f = styles.cell_format(xf);
            let mut check = |what: &str, got: Value, expect: &Value| {
                total += 1;
                let same = match (got.as_f64(), expect.as_f64()) {
                    (Some(a), Some(b)) => a == b,
                    _ => &got == expect,
                };
                if same {
                    matched += 1;
                } else {
                    failures.push(format!(
                        "{sheet_name}!{address} {what}: ours {got}, Excel {expect}"
                    ));
                }
            };
            let bgr = |c: &open365_xlsx::Color| Value::from(styles.resolve_color(c).bgr_u32());

            // Font
            let wf = &want["font"];
            check("font.name", Value::from(f.font.name.clone()), &wf["name"]);
            check("font.size", Value::from(f.font.size), &wf["size"]);
            check("font.bold", Value::from(f.font.bold), &wf["bold"]);
            check("font.italic", Value::from(f.font.italic), &wf["italic"]);
            check(
                "font.underline",
                Value::from(excel_underline(f.font.underline)),
                &wf["underline"],
            );
            check("font.strike", Value::from(f.font.strike), &wf["strike"]);
            check("font.color", bgr(&f.font.color), &wf["color"]);

            // Fill. Excel's Interior.Color is the visible background: the foreground colour of
            // a solid fill, the background colour behind a pattern; PatternColor is the pattern
            // (foreground) colour and is only meaningful for a real pattern.
            let wfill = &want["fill"];
            let pattern = excel_pattern(f.fill.pattern);
            check("fill.pattern", Value::from(pattern), &wfill["pattern"]);
            if pattern != -4142 {
                if f.fill.pattern == FillPattern::Solid {
                    check("fill.color", bgr(&f.fill.fg), &wfill["color"]);
                } else {
                    check("fill.color", bgr(&f.fill.bg), &wfill["color"]);
                    check(
                        "fill.pattern_color",
                        bgr(&f.fill.fg),
                        &wfill["pattern_color"],
                    );
                }
            }

            // Borders: left, right, top, bottom.
            let rb = ws.resolved_border(styles, row, col, &col_styles);
            let edges = [rb.left, rb.right, rb.top, rb.bottom];
            for (i, edge) in edges.iter().enumerate() {
                let w = &want["borders_lrtb"][i];
                let (ls, weight) = excel_line(edge.style);
                check(
                    &format!("border[{i}].style"),
                    Value::from(vec![ls, weight]),
                    &Value::from(vec![w[0].clone(), w[1].clone()]),
                );
                if edge.style != BorderStyle::None {
                    check(&format!("border[{i}].color"), bgr(&edge.color), &w[2]);
                }
            }

            // Alignment
            let wa = &want["align"];
            check(
                "align.h",
                Value::from(excel_h(f.alignment.horizontal)),
                &wa["h"],
            );
            check(
                "align.v",
                Value::from(excel_v(f.alignment.vertical)),
                &wa["v"],
            );
            check(
                "align.wrap",
                Value::from(f.alignment.wrap_text),
                &wa["wrap"],
            );
            check(
                "align.indent",
                Value::from(f.alignment.indent),
                &wa["indent"],
            );
            let orientation = match f.alignment.rotation_degrees() {
                Some(0) => -4128,
                Some(90) => -4171,
                Some(-90) => -4170,
                Some(d) => i64::from(d),
                None => -4166,
            };
            check(
                "align.orientation",
                Value::from(orientation),
                &wa["orientation"],
            );
            check(
                "align.shrink",
                Value::from(f.alignment.shrink_to_fit),
                &wa["shrink"],
            );
        }
    }
    assert!(total > 5000, "compared only {total} values");
    assert!(
        failures.is_empty(),
        "style parity {matched}/{total}; first differences:\n{}",
        failures
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    eprintln!("style parity: {matched}/{total}");
}

#[test]
fn reading_formatting_does_not_change_the_saved_bytes() {
    let bytes = fixture("grid-styles");
    let mut wb = Workbook::open(&bytes, &Limits::default()).expect("opens");
    let styles = wb.styles();
    for xf in 0..styles.cell_xf_count() as u32 {
        let _ = styles.resolve_color(&styles.cell_format(xf).font.color);
    }
    let saved = wb.save().expect("saves");
    let again = Workbook::open(&saved, &Limits::default()).expect("reopens");
    let mut checked = 0;
    for part in wb.package().parts() {
        let name = part.name().as_str().to_ascii_lowercase();
        if name.ends_with("styles.xml") || name.contains("/theme/") {
            let after = again.package().part(part.name().as_str()).expect("part");
            assert_eq!(part.data(), after.data(), "{name}");
            checked += 1;
        }
    }
    assert_eq!(checked, 2);
}

#[test]
fn theme_colours_come_from_theme1() {
    let wb = Workbook::open(&fixture("grid-styles"), &Limits::default()).expect("opens");
    let theme = wb.styles().theme();
    // The fixture was made by a current Excel: Office theme, accent1 156082, dark 2 0E2841.
    assert_eq!(theme.color(4).map(|c| c.rgb_u32()), Some(0x156082));
    assert_eq!(theme.color(3).map(|c| c.rgb_u32()), Some(0x0E2841));
    assert_eq!(theme.color(0).map(|c| c.rgb_u32()), Some(0xFFFFFF));
    assert_eq!(theme.color(1).map(|c| c.rgb_u32()), Some(0x000000));
    assert_eq!(theme.color(12), None);
}
