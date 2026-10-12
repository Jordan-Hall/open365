//! Worksheet drawing objects against Excel: `fixtures/reference/xlsx/drawing-*/drawing.json` (written
//! by `tools/office/drawing-oracle.ps1`) holds `Shape.Left/Top/Width/Height` in points as Excel
//! reports them. Every object rectangle must match within one pixel at 100 % zoom.

mod common;

use open365_opc::Limits;
use open365_xlsx::{GridMetrics, Workbook};
use serde_json::Value;

use common::{fixture, repo_root};

const FIXTURES: [&str; 4] = [
    "drawing-basic",
    "drawing-editas",
    "drawing-hidden",
    "drawing-anchors",
];

fn reference(name: &str) -> Value {
    let path = repo_root()
        .join("fixtures/reference/xlsx")
        .join(name)
        .join("drawing.json");
    serde_json::from_slice(&std::fs::read(&path).expect("drawing.json exists"))
        .expect("drawing.json parses")
}

/// `(sheet, name, [dx, dy, dw, dh])` in points of our rectangle minus Excel's, for every object.
fn compare(name: &str) -> (Vec<(String, String, [f64; 4])>, f64) {
    let json = reference(name);
    let dpi = json["dpi"].as_u64().expect("dpi") as u32;
    let wb = Workbook::open(&fixture(name), &Limits::default()).expect("opens");
    let metrics = GridMetrics::for_styles(wb.styles(), dpi);
    let mut rows = Vec::new();
    for (i, sheet) in json["sheets"]
        .as_array()
        .expect("sheets")
        .iter()
        .enumerate()
    {
        let ws = wb.sheet(i).expect("sheet");
        let objects = wb.sheet_drawing_objects(i);
        let shapes = sheet["shapes"].as_array().expect("shapes");
        assert_eq!(
            objects.len(),
            shapes.len(),
            "{name}: object count on sheet {i}"
        );
        for (obj, shape) in objects.iter().zip(shapes) {
            let ours = obj.anchor.rect_px_pt(ws, metrics);
            let f = |k: &str| shape[k].as_f64().expect(k);
            assert_eq!(obj.object.name, shape["name"].as_str().expect("name"));
            rows.push((
                sheet["name"].as_str().expect("sheet name").to_owned(),
                obj.object.name.clone(),
                [
                    ours.x - f("left"),
                    ours.y - f("top"),
                    ours.width - f("width"),
                    ours.height - f("height"),
                ],
            ));
        }
    }
    (rows, 72.0 / f64::from(dpi))
}

#[test]
fn object_rects_match_excel_within_one_pixel() {
    let mut worst = 0.0_f64;
    let mut failures = Vec::new();
    for name in FIXTURES {
        let (rows, px) = compare(name);
        for (sheet, obj, d) in rows {
            let m = d.iter().fold(0.0_f64, |a, v| a.max(v.abs()));
            worst = worst.max(m);
            eprintln!(
                "{name:16} {sheet:10} {obj:14} dx {:7.2} dy {:7.2} dw {:7.2} dh {:7.2}",
                d[0], d[1], d[2], d[3]
            );
            if m > px {
                failures.push(format!("{name}/{sheet}/{obj}: {d:?} (1 px = {px:.3} pt)"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    eprintln!("worst deviation {worst:.3} pt");
}

#[test]
fn objects_carry_their_kind_and_resolve_their_parts() {
    use open365_drawing::{Kind, model::Geometry};
    let wb = Workbook::open(&fixture("drawing-basic"), &Limits::default()).expect("opens");
    let objects = wb.sheet_drawing_objects(0);
    assert_eq!(objects.len(), 4);
    assert!(objects.windows(2).all(|w| w[0].z < w[1].z));
    let Kind::Picture(pic) = &objects[0].object.kind else {
        panic!("picture first: {:?}", objects[0].object.kind)
    };
    let embed = pic.embed.as_deref().expect("embed");
    let image = objects[0].target(embed).expect("image relationship");
    assert!(image.as_str().starts_with("xl/media/"), "{image}");
    let bytes = wb
        .package()
        .part(image.as_str())
        .expect("image part")
        .data();
    assert_eq!(&bytes[1..4], b"PNG");
    let Kind::Shape(shape) = &objects[1].object.kind else {
        panic!("shape second")
    };
    assert!(matches!(&shape.geometry, Geometry::Preset { name, .. } if name == "rect"));
    assert!(shape.style.is_some(), "Excel writes a style reference");
    let Kind::Chart { rid } = &objects[2].object.kind else {
        panic!("chart third")
    };
    let chart = objects[2].target(rid).expect("chart relationship");
    assert_eq!(chart.as_str(), "xl/charts/chart1.xml");
    // The raw fragment is kept for the round trip.
    assert!(objects[1].object.raw.as_str().starts_with("<xdr:sp"));
}

#[test]
fn anchor_kinds_follow_the_drawing_part() {
    use open365_xlsx::Anchor;
    let wb = Workbook::open(&fixture("drawing-anchors"), &Limits::default()).expect("opens");
    let kinds: Vec<_> = wb
        .sheet_drawing_objects(0)
        .iter()
        .map(|o| {
            matches!(o.anchor, Anchor::OneCell { .. }) as u8
                + matches!(o.anchor, Anchor::Absolute { .. }) as u8 * 2
        })
        .collect();
    assert_eq!(kinds, [1, 2, 2, 2]);
}

#[test]
fn the_workbook_theme_reaches_the_drawing_layer() {
    use open365_drawing::paint::{Rgb, SchemeColor, Theme};
    let wb = Workbook::open(&fixture("drawing-basic"), &Limits::default()).expect("opens");
    // The Office 2023 theme this Excel writes; the colour comes from the file, not the default.
    let theme = wb.drawing_theme();
    assert_eq!(
        theme.scheme_color(SchemeColor::Accent1),
        Rgb(0x15, 0x60, 0x82)
    );
    assert!(!theme.lines.is_empty() && !theme.fills.is_empty());
}
