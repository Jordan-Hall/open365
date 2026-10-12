//! Border display against Excel's `Range.DisplayFormat` (`fixtures/reference/xlsx/styles-*`):
//! every colour form of a border edge (cell format and conditional-format `dxf`), and how the
//! file's styles survive a rewrite of one edge.

mod common;

use open365_opc::Limits;
use open365_xlsx::{BorderEdge, BorderPatch, BorderStyle, FormatPatch, Workbook};
use serde_json::Value;

use common::repo_root;

fn dir(name: &str) -> std::path::PathBuf {
    repo_root()
        .join("fixtures")
        .join("reference")
        .join("xlsx")
        .join(format!("styles-{name}"))
}

fn input(name: &str) -> Vec<u8> {
    std::fs::read(dir(name).join("input.xlsx")).expect("input.xlsx")
}

fn display(name: &str) -> Value {
    let bytes = std::fs::read(dir(name).join("displayformat.json")).expect("displayformat.json");
    serde_json::from_slice::<Value>(&bytes).expect("json")["cells"].clone()
}

fn line(style: BorderStyle) -> i64 {
    match style {
        BorderStyle::None => -4142,
        BorderStyle::Dashed | BorderStyle::MediumDashed => -4115,
        BorderStyle::Dotted => -4118,
        BorderStyle::Double => -4119,
        BorderStyle::DashDot | BorderStyle::MediumDashDot => 4,
        BorderStyle::SlantDashDot => 13,
        BorderStyle::DashDotDot | BorderStyle::MediumDashDotDot => 5,
        _ => 1,
    }
}

fn weight(style: BorderStyle) -> i64 {
    match style {
        BorderStyle::Hair => 1,
        BorderStyle::Medium
        | BorderStyle::MediumDashed
        | BorderStyle::MediumDashDot
        | BorderStyle::MediumDashDotDot
        | BorderStyle::SlantDashDot => -4138,
        BorderStyle::Thick | BorderStyle::Double => 4,
        _ => 2,
    }
}

#[test]
fn every_border_colour_form_resolves_like_excel() {
    let wb = Workbook::open(&input("border-colors"), &Limits::default()).expect("opens");
    let want = display("border-colors");
    let ws = wb.sheet(0).expect("sheet");
    let styles = wb.styles();
    let cols = ws.column_styles();
    let mut bad = Vec::new();
    let mut n = 0;
    for i in 0..25u32 {
        let row = 2 + 2 * i;
        let b = ws.resolved_border(styles, row, 2, &cols);
        let got = [
            ("left", b.left),
            ("right", b.right),
            ("top", b.top),
            ("bottom", b.bottom),
            ("dd", b.diagonal),
        ];
        for (side, e) in got {
            n += 1;
            let w = &want[format!("B{row}")][side];
            let ours = styles.resolve_color(&e.color).bgr_u32();
            if w["c"].as_i64() != Some(i64::from(ours)) || w["ls"].as_i64() != Some(line(e.style)) {
                bad.push(format!(
                    "cell form {i} {side}: ours {ours} {:?}, Excel {w}",
                    e.style
                ));
            }
        }
        // The conditional format of E<row>.
        let d = styles.dxf(i).expect("dxf");
        let e = d.border.left.expect("dxf left");
        n += 1;
        let w = &want[format!("E{row}")]["left"];
        let ours = styles.resolve_color(&e.color).bgr_u32();
        if w["c"].as_i64() != Some(i64::from(ours)) {
            bad.push(format!("dxf form {i}: ours {ours}, Excel {w}"));
        }
    }
    assert!(
        bad.is_empty(),
        "{}/{n} differ:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// The `left` edge's colour element of every `<border>` in a styles part.
fn left_colours(styles: &str) -> Vec<String> {
    let borders = styles.split("<borders").nth(1).expect("borders");
    let borders = borders.split("</borders>").next().expect("end");
    borders
        .split("<border")
        .skip(1)
        .filter(|b| !b.starts_with('s'))
        .map(|b| {
            b.split("<left")
                .nth(1)
                .and_then(|l| l.split("</left>").next())
                .and_then(|l| l.split("<color").nth(1))
                .map(|c| c.split("/>").next().unwrap_or("").trim().to_owned())
                .unwrap_or_default()
        })
        .collect()
}

fn styles_part(bytes: &[u8]) -> String {
    let wb = Workbook::open(bytes, &Limits::default()).expect("opens");
    let part = wb.package().part("xl/styles.xml").expect("styles");
    String::from_utf8(part.data().to_vec()).expect("utf8")
}

/// Making the left edge of every probe cell thick keeps the colour element Excel writes for the
/// same colour in `saved/xl_styles.xml` (its own rewrite of the input): `auto="1"` stays, an
/// `indexed="64"` stays, an alpha byte becomes `FF`, a tint is written as Excel writes it.
#[test]
fn restyling_an_edge_writes_the_colour_element_excel_writes() {
    let mut wb = Workbook::open(&input("border-colors"), &Limits::default()).expect("opens");
    let excel = left_colours(
        &std::fs::read_to_string(dir("border-colors").join("saved").join("xl_styles.xml"))
            .expect("saved styles"),
    );
    let mut bad = Vec::new();
    // Form 22 is `auto` with a tint: only a hand-edited file has one.
    let forms: Vec<u32> = (0..25).filter(|f| *f != 22).collect();
    for &f in &forms {
        let row = 2 + 2 * f;
        let color = wb.cell_format(0, row, 2).border.left.color;
        let patch = FormatPatch {
            border: BorderPatch {
                left: Some(BorderEdge {
                    style: BorderStyle::Thick,
                    color,
                }),
                ..BorderPatch::default()
            },
            ..FormatPatch::default()
        };
        wb.format_range(0, (row, 2, row, 2), &patch)
            .expect("format");
    }
    let saved = wb.save().expect("saves");
    let ours = left_colours(&styles_part(&saved));
    let wb2 = Workbook::open(&saved, &Limits::default()).expect("reopens");
    let xf_borders = xf_border_ids(&styles_part(&saved));
    for &f in &forms {
        let id = xf_borders[wb2.cell_xf(0, 2 + 2 * f, 2) as usize];
        let got = ours[id].clone();
        // Excel's saved border `f + 1` is the same colour (the input's order).
        let want = excel[f as usize + 1].clone();
        if got != want {
            bad.push(format!("form {f}: ours {got:?}, Excel {want:?}"));
        }
    }
    assert!(
        bad.is_empty(),
        "{}",
        bad.join(
            "
"
        )
    );
}

fn xf_border_ids(styles: &str) -> Vec<usize> {
    let xfs = styles.split("<cellXfs").nth(1).expect("cellXfs");
    let xfs = xfs.split("</cellXfs>").next().expect("end");
    xfs.split("<xf ")
        .skip(1)
        .map(|x| {
            x.split("borderId=\"")
                .nth(1)
                .and_then(|s| s.split('"').next())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0)
        })
        .collect()
}

const STYLE_NAMES: [BorderStyle; 14] = [
    BorderStyle::None,
    BorderStyle::Thin,
    BorderStyle::Medium,
    BorderStyle::Dashed,
    BorderStyle::Dotted,
    BorderStyle::Thick,
    BorderStyle::Double,
    BorderStyle::Hair,
    BorderStyle::MediumDashed,
    BorderStyle::DashDot,
    BorderStyle::MediumDashDot,
    BorderStyle::DashDotDot,
    BorderStyle::MediumDashDotDot,
    BorderStyle::SlantDashDot,
];

/// All 14 x 14 pairs of line styles drawn on both sides of a shared edge, horizontal and
/// vertical: both cells show the edge Excel's `DisplayFormat` reports (the higher-ranked line,
/// and on a tie the right or lower cell's colour).
#[test]
fn a_shared_edge_shows_the_line_excel_shows() {
    let wb = Workbook::open(&input("border-conflicts"), &Limits::default()).expect("opens");
    let want = display("border-conflicts");
    let ws = wb.sheet(0).expect("sheet");
    let styles = wb.styles();
    let cols = ws.column_styles();
    let mut bad = Vec::new();
    let mut n = 0;
    let mut k = 0u32;
    for a in STYLE_NAMES {
        for b in STYLE_NAMES {
            let h = 2 + 2 * k;
            let v = 2 + 3 * k;
            k += 1;
            let cases = [
                (h, 2, "right", h, 3, "left", "H"),
                (v, 6, "bottom", v + 1, 6, "top", "V"),
            ];
            for (r1, c1, side1, r2, c2, side2, dir) in cases {
                let near = ws.resolved_border(styles, r1, c1, &cols);
                let far = ws.resolved_border(styles, r2, c2, &cols);
                let (e1, e2) = if dir == "H" {
                    (near.right, far.left)
                } else {
                    (near.bottom, far.top)
                };
                for (e, r, c, side) in [(e1, r1, c1, side1), (e2, r2, c2, side2)] {
                    n += 1;
                    let w = &want[format!("{}{r}", common_col(c))][side];
                    let ours = (
                        line(e.style),
                        weight(e.style),
                        i64::from(styles.resolve_color(&e.color).bgr_u32()),
                    );
                    let same = if e.style == BorderStyle::None {
                        w["ls"].as_i64() == Some(-4142)
                    } else {
                        w["ls"].as_i64() == Some(ours.0)
                            && w["w"].as_i64() == Some(ours.1)
                            && w["c"].as_i64() == Some(ours.2)
                    };
                    if !same {
                        bad.push(format!("{a:?}|{b:?} {dir} {side}: ours {e:?}, Excel {w}"));
                    }
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{}/{n} differ:\n{}",
        bad.len(),
        bad.iter().take(30).cloned().collect::<Vec<_>>().join("\n")
    );
}

fn common_col(c: u32) -> char {
    char::from(b'A' + (c - 1) as u8)
}

/// Diagonals: one `diagonal` line shown as `xlDiagonalUp` and/or `xlDiagonalDown` by the flags,
/// never shared with a neighbour, nothing when the flag is set without a line.
#[test]
fn diagonal_borders_follow_their_flags_like_excel() {
    let wb = Workbook::open(&input("border-diagonals"), &Limits::default()).expect("opens");
    let want = display("border-diagonals");
    let ws = wb.sheet(0).expect("sheet");
    let styles = wb.styles();
    let cols = ws.column_styles();
    let mut bad = Vec::new();
    let mut n = 0;
    let mut cells: Vec<(u32, u32)> = Vec::new();
    for k in 0..13u32 {
        for j in 0..4u32 {
            cells.push((2 + 2 * k, 2 + j));
        }
    }
    cells.extend([(2, 8), (4, 8), (2, 7), (2, 9)]);
    for (row, col) in cells {
        let b = ws.resolved_border(styles, row, col, &cols);
        let address = format!("{}{row}", common_col(col));
        for (side, e) in [("dd", b.diagonal_down_edge()), ("du", b.diagonal_up_edge())] {
            n += 1;
            let w = &want[address.as_str()][side];
            let same = if e.style == BorderStyle::None {
                w["ls"].as_i64() == Some(-4142)
            } else {
                w["ls"].as_i64() == Some(line(e.style))
                    && w["w"].as_i64() == Some(weight(e.style))
                    && w["c"].as_i64() == Some(i64::from(styles.resolve_color(&e.color).bgr_u32()))
            };
            if !same {
                bad.push(format!("{address} {side}: ours {e:?}, Excel {w}"));
            }
        }
        for (side, e) in [("left", b.left), ("right", b.right)] {
            n += 1;
            let w = &want[address.as_str()][side];
            if (e.style == BorderStyle::None) != (w["ls"].as_i64() == Some(-4142)) {
                bad.push(format!("{address} {side}: ours {e:?}, Excel {w}"));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{}/{n} differ:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

#[test]
fn set_cell_xfs_styles_many_cells_with_one_snapshot() {
    let mut wb = Workbook::open(&input("border-colors"), &Limits::default()).expect("opens");
    let before = wb
        .set_cell_xfs(0, &[(40, 9, 3), (41, 9, 4), (40, 9, 5)])
        .expect("applies");
    assert_eq!(wb.cell_xf(0, 40, 9), 5);
    assert_eq!(wb.cell_xf(0, 41, 9), 4);
    assert!(wb.set_cell_xfs(0, &[(1, 1, 9999)]).is_err());
    wb.restore_format(before).expect("restores");
    assert_eq!(wb.cell_xf(0, 40, 9), 0);
}

/// Conditional-format borders replace the cell's own edge, then compete on the shared edge by
/// rank like any other (all 7 x 7 pairs, CF against own and CF against CF, both directions).
#[test]
fn conditional_borders_show_what_excel_shows() {
    let wb = Workbook::open(&input("border-cf"), &Limits::default()).expect("opens");
    let want = display("border-cf");
    let ws = wb.sheet(0).expect("sheet");
    let styles = wb.styles();
    let cols = ws.column_styles();
    let blocks = ws.conditional_formats();
    // Every probe cell holds 1 and every rule is `= 1`: a rule applies where its block covers.
    let cf = |r: u32, c: u32| {
        blocks
            .iter()
            .filter(|b| b.contains(r, c))
            .flat_map(|b| b.rules.iter())
            .filter_map(|rule| rule.dxf_id.and_then(|id| styles.dxf(id)))
            .map(|d| d.border.clone())
            .next()
    };
    let mut bad = Vec::new();
    let mut n = 0;
    let probes = |k: u32| -> Vec<(u32, u32, &'static str)> {
        let h = 2 + 2 * k;
        let v = 2 + 3 * k;
        vec![
            (h, 2, "right"),
            (h, 3, "left"),
            (h, 5, "right"),
            (h, 6, "left"),
            (h, 8, "right"),
            (h, 9, "left"),
            (h, 18, "right"),
            (v, 12, "bottom"),
            (v + 1, 12, "top"),
            (v, 14, "bottom"),
            (v + 1, 14, "top"),
            (v, 16, "bottom"),
            (v + 1, 16, "top"),
        ]
    };
    for k in 0..49u32 {
        for (r, c, side) in probes(k) {
            let b = ws.display_border(styles, r, c, &cols, &cf);
            let e = match side {
                "right" => b.right,
                "left" => b.left,
                "top" => b.top,
                _ => b.bottom,
            };
            n += 1;
            let w = &want[format!("{}{r}", common_col(c))][side];
            let same = if e.style == BorderStyle::None {
                w["ls"].as_i64() == Some(-4142)
            } else {
                w["ls"].as_i64() == Some(line(e.style))
                    && w["w"].as_i64() == Some(weight(e.style))
                    && w["c"].as_i64() == Some(i64::from(styles.resolve_color(&e.color).bgr_u32()))
            };
            if !same {
                bad.push(format!(
                    "pair {k} {}{r} {side}: ours {e:?}, Excel {w}",
                    common_col(c)
                ));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{}/{n} differ:\n{}",
        bad.len(),
        bad.iter().take(20).cloned().collect::<Vec<_>>().join("\n")
    );
}
