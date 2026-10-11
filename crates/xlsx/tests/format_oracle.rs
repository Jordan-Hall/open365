//! Cell formatting against Excel: every scenario in `fixtures/xlsx/format-scenarios.json` is applied
//! to `fixtures/xlsx/format-blank.xlsx` through the engine, saved, reopened and compared with what
//! Excel produced (`fixtures/reference/xlsx/format-<name>/expected.json`, recorded by
//! `tools/office/format-oracle.ps1`): the same number of fonts, fills, borders and cell formats, and for
//! every probe cell the same font, fill, border, number format, cell style and alignment/protection.

#![allow(clippy::unwrap_used, clippy::field_reassign_with_default)]

use std::path::PathBuf;

use open365_xlsx::{
    AlignmentPatch, BorderEdge, BorderSpec, BorderStyle, Color, ColorSource, FillPattern, FillSpec,
    FontPatch, FormatPatch, HorizontalAlignment, NumberFormat, ProtectionPatch, Rgb, Rotation,
    Underline, VerticalAlignment, Workbook,
};
use open365_xlsx::{CellInput, MAX_COL, MAX_ROW};
use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn col_number(s: &str) -> u32 {
    s.bytes().fold(0, |n, b| n * 26 + u32::from(b - b'A' + 1))
}

/// `A1`, `A1:B2`, `C:C`, `2:2` as `(row1, col1, row2, col2)`.
fn rect(text: &str) -> (u32, u32, u32, u32) {
    let (a, b) = text.split_once(':').unwrap_or((text, text));
    let split = |s: &str| {
        let letters: String = s.chars().take_while(char::is_ascii_alphabetic).collect();
        let digits: String = s.chars().skip(letters.len()).collect();
        (letters, digits)
    };
    let (la, da) = split(a);
    let (lb, db) = split(b);
    match (la.is_empty(), da.is_empty()) {
        (false, true) => (1, col_number(&la), MAX_ROW, col_number(&lb)),
        (true, false) => (da.parse().unwrap(), 1, db.parse().unwrap(), MAX_COL),
        _ => (
            da.parse().unwrap(),
            col_number(&la),
            db.parse().unwrap(),
            col_number(&lb),
        ),
    }
}

fn color(v: &Value) -> Color {
    if let Some(h) = v.get("rgb").and_then(Value::as_str) {
        let n = u32::from_str_radix(h, 16).unwrap();
        return Color {
            source: ColorSource::Rgb(Rgb::new((n >> 16) as u8, (n >> 8) as u8, n as u8)),
            tint: 0.0,
        };
    }
    if let Some(t) = v.get("theme").and_then(Value::as_u64) {
        return Color {
            source: ColorSource::Theme(t as u32),
            tint: v.get("tint").and_then(Value::as_f64).unwrap_or(0.0),
        };
    }
    Color::AUTO
}

fn border_style(s: &str) -> BorderStyle {
    match s {
        "thin" => BorderStyle::Thin,
        "medium" => BorderStyle::Medium,
        "dashed" => BorderStyle::Dashed,
        "dotted" => BorderStyle::Dotted,
        "thick" => BorderStyle::Thick,
        "double" => BorderStyle::Double,
        "hair" => BorderStyle::Hair,
        "mediumDashed" => BorderStyle::MediumDashed,
        "dashDot" => BorderStyle::DashDot,
        "mediumDashDot" => BorderStyle::MediumDashDot,
        "dashDotDot" => BorderStyle::DashDotDot,
        "mediumDashDotDot" => BorderStyle::MediumDashDotDot,
        "slantDashDot" => BorderStyle::SlantDashDot,
        _ => BorderStyle::None,
    }
}

fn pattern(s: &str) -> FillPattern {
    match s {
        "mediumGray" => FillPattern::MediumGray,
        "darkGray" => FillPattern::DarkGray,
        "lightGray" => FillPattern::LightGray,
        "darkHorizontal" => FillPattern::DarkHorizontal,
        "darkVertical" => FillPattern::DarkVertical,
        "darkDown" => FillPattern::DarkDown,
        "darkUp" => FillPattern::DarkUp,
        "darkGrid" => FillPattern::DarkGrid,
        "darkTrellis" => FillPattern::DarkTrellis,
        "lightHorizontal" => FillPattern::LightHorizontal,
        "lightVertical" => FillPattern::LightVertical,
        "lightDown" => FillPattern::LightDown,
        "lightUp" => FillPattern::LightUp,
        "lightGrid" => FillPattern::LightGrid,
        "lightTrellis" => FillPattern::LightTrellis,
        "gray125" => FillPattern::Gray125,
        "gray0625" => FillPattern::Gray0625,
        _ => FillPattern::None,
    }
}

fn e<T>(r: Result<T, open365_xlsx::XlsxError>) -> Result<(), String> {
    r.map(|_| ()).map_err(|e| e.to_string())
}

fn apply_op(wb: &mut Workbook, op: &Value) -> Result<(), String> {
    let kind = op["op"].as_str().unwrap_or_default();
    let range = |key: &str| rect(op[key].as_str().unwrap_or("A1"));
    match kind {
        "font" => {
            let mut p = FontPatch::default();
            p.name = op["name"].as_str().map(str::to_owned);
            p.size = op["size"].as_f64();
            p.bold = op["bold"].as_bool();
            p.italic = op["italic"].as_bool();
            p.strike = op["strike"].as_bool();
            p.color = op.get("color").map(color);
            p.underline = op["underline"].as_str().map(|u| match u {
                "single" => Underline::Single,
                "double" => Underline::Double,
                "singleAccounting" => Underline::SingleAccounting,
                "doubleAccounting" => Underline::DoubleAccounting,
                _ => Underline::None,
            });
            e(wb.format_range(
                0,
                range("range"),
                &FormatPatch {
                    font: p,
                    ..FormatPatch::default()
                },
            ))
        }
        "fill" => {
            let name = op["pattern"].as_str().unwrap_or("none");
            let fg = op.get("fg").map(color);
            let bg = op.get("bg").map(color);
            let spec = match name {
                "none" => FillSpec::None,
                "solid" => FillSpec::Solid(fg.unwrap_or(Color::AUTO)),
                other => FillSpec::Pattern {
                    pattern: pattern(other),
                    fg,
                    bg,
                },
            };
            e(wb.format_range(
                0,
                range("range"),
                &FormatPatch {
                    fill: Some(spec),
                    ..FormatPatch::default()
                },
            ))
        }
        "border" => {
            let edge = BorderEdge {
                style: border_style(op["style"].as_str().unwrap_or("none")),
                color: op.get("color").map_or(Color::AUTO, color),
            };
            let mut spec = BorderSpec::default();
            for side in op["sides"].as_array().into_iter().flatten() {
                match side.as_str().unwrap_or_default() {
                    "outline" => {
                        spec = BorderSpec {
                            left: Some(edge),
                            right: Some(edge),
                            top: Some(edge),
                            bottom: Some(edge),
                            ..spec
                        }
                    }
                    "all" => {
                        spec = BorderSpec {
                            inside_horizontal: Some(edge),
                            inside_vertical: Some(edge),
                            left: Some(edge),
                            right: Some(edge),
                            top: Some(edge),
                            bottom: Some(edge),
                            ..spec
                        }
                    }
                    "left" => spec.left = Some(edge),
                    "right" => spec.right = Some(edge),
                    "top" => spec.top = Some(edge),
                    "bottom" => spec.bottom = Some(edge),
                    "insideHorizontal" => spec.inside_horizontal = Some(edge),
                    "insideVertical" => spec.inside_vertical = Some(edge),
                    "diagonalDown" => spec.diagonal_down = Some(edge),
                    "diagonalUp" => spec.diagonal_up = Some(edge),
                    other => return Err(format!("unknown side {other}")),
                }
            }
            e(wb.set_borders(0, range("range"), &spec))
        }
        "align" => {
            let mut p = AlignmentPatch::default();
            p.horizontal = op["horizontal"].as_str().map(|h| match h {
                "left" => HorizontalAlignment::Left,
                "center" => HorizontalAlignment::Center,
                "right" => HorizontalAlignment::Right,
                "fill" => HorizontalAlignment::Fill,
                "justify" => HorizontalAlignment::Justify,
                "centerContinuous" => HorizontalAlignment::CenterContinuous,
                "distributed" => HorizontalAlignment::Distributed,
                _ => HorizontalAlignment::General,
            });
            p.vertical = op["vertical"].as_str().map(|v| match v {
                "top" => VerticalAlignment::Top,
                "center" => VerticalAlignment::Center,
                "justify" => VerticalAlignment::Justify,
                "distributed" => VerticalAlignment::Distributed,
                _ => VerticalAlignment::Bottom,
            });
            p.wrap_text = op["wrap"].as_bool();
            p.shrink_to_fit = op["shrink"].as_bool();
            p.indent = op["indent"].as_u64().map(|v| v as u32);
            p.rotation = match &op["rotation"] {
                Value::String(_) => Some(Rotation::Stacked),
                Value::Number(n) => n.as_i64().map(|d| Rotation::Degrees(d as i32)),
                _ => None,
            };
            e(wb.format_range(
                0,
                range("range"),
                &FormatPatch {
                    alignment: p,
                    ..FormatPatch::default()
                },
            ))
        }
        "numfmt" => e(wb.format_range(
            0,
            range("range"),
            &FormatPatch {
                number_format: Some(NumberFormat::Code(
                    op["code"].as_str().unwrap_or("General").to_owned(),
                )),
                ..FormatPatch::default()
            },
        )),
        "protect" => e(wb.format_range(
            0,
            range("range"),
            &FormatPatch {
                protection: ProtectionPatch {
                    locked: op["locked"].as_bool(),
                    hidden: op["hidden"].as_bool(),
                },
                ..FormatPatch::default()
            },
        )),
        "style" => {
            e(wb.apply_cell_style(0, range("range"), op["name"].as_str().unwrap_or("Normal")))
        }
        "percent" => e(wb.apply_cell_style(0, range("range"), "Percent")),
        "comma" => e(wb.apply_cell_style(0, range("range"), "Comma")),
        "accounting" => e(wb.apply_cell_style(0, range("range"), "Currency")),
        "decimals" => {
            e(wb.change_decimals(0, range("range"), op["delta"].as_i64().unwrap_or(1) as i32))
        }
        "clear_formats" => e(wb.clear_formats(0, range("range"))),
        "painter" => e(wb.copy_formats(0, range("from"), range("to"))),
        "value" => {
            let (r, c, ..) = range("range");
            e(wb.set_cell_value(
                0,
                r,
                c,
                CellInput::Number(op["value"].as_f64().unwrap_or(0.0)),
            ))
        }
        "row_height" => {
            let (r1, _, r2, _) = range("range");
            e(wb.set_row_height(0, r1, r2, op["points"].as_f64().unwrap_or(15.0)))
        }
        "col_width" => {
            let (_, c1, _, c2) = range("range");
            e(wb.set_column_width(0, c1, c2, op["width"].as_f64().unwrap_or(8.43) + 0.77734375))
        }
        other => Err(format!("unknown op {other}")),
    }
}

// ---- reading the saved file the way the oracle reads Excel's ---------------------------------

struct Parsed {
    fonts: Vec<String>,
    fills: Vec<String>,
    borders: Vec<String>,
    xfs: Vec<open365_xlsx::XmlElement>,
    style_xfs: usize,
    cell_styles: Vec<open365_xlsx::XmlElement>,
    num_fmts: Vec<(u32, String)>,
}

fn text(e: &open365_xlsx::XmlElement) -> String {
    let mut s = String::new();
    e.write(&mut s);
    s
}

fn parse_styles(bytes: &[u8]) -> Parsed {
    let doc = open365_xlsx::parse_xml(bytes).unwrap();
    let list = |name: &str| -> Vec<open365_xlsx::XmlElement> {
        doc.root
            .elements()
            .find(|e| e.head.local_name() == name)
            .map(|s| s.elements().cloned().collect())
            .unwrap_or_default()
    };
    Parsed {
        fonts: list("fonts").iter().map(text).collect(),
        fills: list("fills").iter().map(text).collect(),
        borders: list("borders").iter().map(text).collect(),
        xfs: list("cellXfs"),
        style_xfs: list("cellStyleXfs").len(),
        cell_styles: list("cellStyles"),
        num_fmts: list("numFmts")
            .iter()
            .map(|n| {
                (
                    n.head
                        .attr("numFmtId")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0),
                    n.head.attr("formatCode").unwrap_or_default().to_owned(),
                )
            })
            .collect(),
    }
}

/// Tints compare to 9 places: Excel writes 17 significant digits.
fn norm(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("tint=\"") {
        out.push_str(&rest[..i + 6]);
        rest = &rest[i + 6..];
        let end = rest.find('"').unwrap_or(rest.len());
        let v: f64 = rest[..end].parse().unwrap_or(0.0);
        out.push_str(&format!("{v:.9}"));
        rest = &rest[end..];
    }
    out.push_str(rest);
    out.replace("<xf />", "<xf/>")
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;")
}

/// The canonical format of xf `i`, as `Canon-Xf` in the oracle script builds it.
fn canon(p: &Parsed, i: usize) -> Value {
    let xf = &p.xfs[i];
    let id = |n: &str| -> usize { xf.head.attr(n).and_then(|v| v.parse().ok()).unwrap_or(0) };
    let num_fmt_id = id("numFmtId") as u32;
    let num = match p.num_fmts.iter().find(|(n, _)| *n == num_fmt_id) {
        Some((_, code)) if num_fmt_id >= 164 => format!("custom:{}", escape_attr(code)),
        _ => format!("id:{num_fmt_id}"),
    };
    let style = p
        .cell_styles
        .iter()
        .find(|c| c.head.attr("xfId").and_then(|v| v.parse::<usize>().ok()) == Some(id("xfId")))
        .map(|c| match c.head.attr("builtinId") {
            Some(b) => format!("builtin:{b}"),
            None => c.head.attr("name").unwrap_or_default().to_owned(),
        });
    let mut stripped = xf.clone();
    for a in ["fontId", "fillId", "borderId", "numFmtId", "xfId"] {
        stripped.head.remove_attr(a);
    }
    serde_json::json!({
        "font": p.fonts.get(id("fontId")),
        "fill": p.fills.get(id("fillId")),
        "border": p.borders.get(id("borderId")),
        "numFmt": num,
        "style": style,
        "xf": text(&stripped),
    })
}

fn probes(spec: &str) -> Vec<(String, u32, u32)> {
    let mut out = Vec::new();
    for part in spec.split(',') {
        let (r1, c1, r2, c2) = rect(part.trim());
        for r in r1..=r2 {
            for c in c1..=c2 {
                out.push((open365_xlsx::cell_ref_string(r, c), r, c));
            }
        }
    }
    out
}

struct Outcome {
    name: String,
    problems: Vec<String>,
}

fn run_scenario(sc: &Value) -> Option<Outcome> {
    let name = sc["name"].as_str()?.to_owned();
    let expected_path = root().join(format!(
        "fixtures/reference/xlsx/format-{name}/expected.json"
    ));
    let expected: Value = serde_json::from_slice(
        std::fs::read(&expected_path)
            .ok()?
            .strip_prefix(b"\xEF\xBB\xBF")
            .map(<[u8]>::to_vec)
            .unwrap_or_else(|| std::fs::read(&expected_path).unwrap_or_default())
            .as_slice(),
    )
    .ok()?;
    if !expected["error"].is_null() {
        return None;
    }
    let mut problems = Vec::new();
    let blank = std::fs::read(root().join("fixtures/xlsx/format-blank.xlsx")).unwrap();
    let mut wb = Workbook::open(&blank, &open365_opc::Limits::default()).unwrap();
    for op in sc["ops"].as_array().into_iter().flatten() {
        if let Err(e) = apply_op(&mut wb, op) {
            problems.push(format!("op {} failed: {e}", op["op"]));
        }
    }
    let saved = wb.save().unwrap();
    let wb2 = Workbook::open(&saved, &open365_opc::Limits::default()).unwrap();
    let mine = parse_styles(&wb2.serialize_styles().unwrap());
    let counts = &expected["counts"];
    for (key, have) in [
        ("fonts", mine.fonts.len()),
        ("fills", mine.fills.len()),
        ("borders", mine.borders.len()),
        ("cellXfs", mine.xfs.len()),
        ("cellStyleXfs", mine.style_xfs),
        ("cellStyles", mine.cell_styles.len()),
        ("numFmts", mine.num_fmts.len()),
    ] {
        let mut want = counts[key].as_u64().unwrap_or(0) as usize;
        if key == "numFmts" {
            // Excel writes the locale's text for built-in ids 41 to 44 into numFmts; the engine does not.
            want -= expected["tables"]["numFmts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|n| {
                    n.as_str().is_some_and(|n| {
                        ["41", "42", "43", "44"]
                            .iter()
                            .any(|id| n.starts_with(&format!("<numFmt numFmtId=\"{id}\"")))
                    })
                })
                .count();
        }
        if have != want {
            problems.push(format!("{key}: engine {have}, Excel {want}"));
        }
    }
    let probe = sc["probe"].as_str().unwrap_or("A1:F8");
    for (addr, r, c) in probes(probe) {
        let xf = wb2.cell_xf(0, r, c) as usize;
        let have = canon(&mine, xf.min(mine.xfs.len() - 1));
        let want = &expected["cells"][&addr]["canon"];
        for key in ["font", "fill", "border", "numFmt", "style", "xf"] {
            let h = norm(have[key].as_str().unwrap_or(""));
            let w = norm(want[key].as_str().unwrap_or(""));
            if h != w {
                problems.push(format!("{addr} {key}:\n    engine {h}\n    Excel  {w}"));
            }
        }
    }
    Some(Outcome { name, problems })
}

#[test]
fn engine_matches_excel_on_every_scenario() {
    let spec: Value = serde_json::from_slice(
        &std::fs::read(root().join("fixtures/xlsx/format-scenarios.json")).unwrap(),
    )
    .unwrap();
    let only = std::env::var("FORMAT_ONLY").ok();
    let mut compared = 0;
    let mut matched = 0;
    let mut failures = Vec::new();
    let (mut cells_total, mut cells_bad) = (0usize, 0usize);
    for sc in spec["scenarios"].as_array().unwrap() {
        if only
            .as_deref()
            .is_some_and(|o| sc["name"].as_str() != Some(o))
        {
            continue;
        }
        let Some(out) = run_scenario(sc) else {
            continue;
        };
        compared += 1;
        let probe_cells = probes(sc["probe"].as_str().unwrap_or("A1:F8")).len();
        cells_total += probe_cells;
        let bad_cells = out
            .problems
            .iter()
            .filter(|p| {
                p.contains(" font:")
                    || p.contains(" fill:")
                    || p.contains(" border:")
                    || p.contains(" numFmt:")
                    || p.contains(" style:")
                    || p.contains(" xf:")
            })
            .map(|p| p.split_whitespace().next().unwrap_or_default().to_owned())
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        cells_bad += bad_cells;
        if out.problems.is_empty() {
            matched += 1;
        } else {
            failures.push(format!("{}:\n  {}", out.name, out.problems.join("\n  ")));
        }
    }
    eprintln!(
        "format oracle: {matched}/{compared} scenarios match Excel; {} of {cells_total} probed cells match",
        cells_total - cells_bad
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
