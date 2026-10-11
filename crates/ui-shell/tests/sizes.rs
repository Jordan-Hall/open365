//! Sizes the ribbon walk does not cover: the Sheets formula strip, the sheet tab bar and the
//! grid metrics, all against pixel scans and UIA rectangles of the reference (125 %, 1 px).
//!
//! The scans are `metrics.scan*` in `excel-ribbon.json`: colour runs along a line of the
//! oracle window (numbers only). A 1 px border of colour `666666` marks each box edge.

use open365_ui_shell::layout::{LayoutInput, PxRect, compute};
use open365_ui_shell::spec::AppSpec;
use serde_json::Value;

fn excel() -> Value {
    let path = format!(
        "{}/../../fixtures/reference/ui/excel-ribbon.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("reference file")).expect("json")
}

/// Positions (along the scan, plus its origin) of the 1 px runs with colour `hex`.
fn edges(m: &Value, key: &str, hex: &str, axis: usize) -> Vec<i32> {
    let origin = m[key]["origin"][axis].as_i64().expect("origin") as i32;
    m[key]["runs"]
        .as_array()
        .expect("runs")
        .iter()
        .filter(|r| r[2] == hex && r[1] == 1)
        .map(|r| origin + r[0].as_i64().expect("start") as i32)
        .collect()
}

fn layout_at_125(expanded: bool) -> (AppSpec, PxRect, PxRect, PxRect, PxRect) {
    let spec = AppSpec::sheets();
    let [px, py, _, _] = spec.pane;
    let lay = compute(
        &spec,
        &LayoutInput {
            scale: 1.25,
            window: (spec.ref_window[0], spec.ref_window[1]),
            inset: (px, py, px, px),
            tab: 0,
            ribbon_open: true,
            formula_expanded: expanded,
        },
    );
    let a = lay.name_box_outer.expect("name box");
    let b = lay.grip.expect("grip");
    let c = lay.fx_group.expect("fx group");
    let d = lay.formula_field.expect("field");
    (spec, a, b, c, d)
}

fn close(what: &str, ours: PxRect, theirs: PxRect, misses: &mut Vec<String>, total: &mut usize) {
    *total += 1;
    if ours.max_delta(&theirs) > 1 {
        misses.push(format!("{what}: ours {ours:?} reference {theirs:?}"));
    }
}

#[test]
fn formula_strip_matches_pixel_scans() {
    let r = excel();
    let m = &r["metrics"];
    let xs = edges(m, "scanRowY248", "666666", 0);
    // Borders at the left and right edge of: Name Box, fx group, field (x = 19, 145, 171, 277, 283, 1580).
    let find = |x: i32| xs.contains(&x);
    for x in [19, 145, 171, 277, 283, 1580] {
        assert!(find(x), "scan has a border at x={x}: {xs:?}");
    }
    let y60 = edges(m, "scanColX60", "666666", 1);
    let y900 = edges(m, "scanColX900", "666666", 1);
    assert_eq!((y60[0], y60[1]), (233, 262));
    assert_eq!((y900[0], y900[1]), (233, 310));
    let (_, name_box, _grip, fx, field) = layout_at_125(true);
    let (mut total, mut misses) = (0, Vec::new());
    let boxed = |x0: i32, x1: i32, y0: i32, y1: i32| PxRect {
        x: x0,
        y: y0,
        w: x1 - x0 + 1,
        h: y1 - y0 + 1,
    };
    close(
        "name box",
        name_box,
        boxed(xs[0], xs[1], y60[0], y60[1]),
        &mut misses,
        &mut total,
    );
    close(
        "fx group",
        fx,
        boxed(171, 277, y60[0], y60[1]),
        &mut misses,
        &mut total,
    );
    close(
        "formula field",
        field,
        boxed(283, 1580, y900[0], y900[1]),
        &mut misses,
        &mut total,
    );
    println!(
        "formula strip: {}/{total} within 1 px",
        total - misses.len()
    );
    assert!(misses.is_empty(), "{misses:#?}");
}

fn walk<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
    if v["id"] == "SheetTab" {
        out.push(v);
    }
    for c in v["children"].as_array().into_iter().flatten() {
        walk(c, out);
    }
}

#[test]
fn sheet_tab_bar_matches_uia() {
    let r = excel();
    let mut tabs = Vec::new();
    for p in r["panes"].as_array().expect("panes") {
        walk(p, &mut tabs);
    }
    assert_eq!(
        tabs.len(),
        4,
        "Scroll Left, Scroll Right, Add Sheet, Sheet1"
    );
    let (spec, ..) = layout_at_125(true);
    let [px, py, _, _] = spec.pane;
    let lay = compute(
        &spec,
        &LayoutInput {
            scale: 1.25,
            window: (spec.ref_window[0], spec.ref_window[1]),
            inset: (px, py, px, px),
            tab: 0,
            ribbon_open: true,
            formula_expanded: true,
        },
    );
    let (mut total, mut misses) = (0, Vec::new());
    for item in &lay.sheet_items {
        let name = item.chrome.ref_name.as_deref().unwrap_or_default();
        let want = tabs
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("reference has {name}"));
        let a: Vec<i32> = want["rect"]
            .as_array()
            .expect("rect")
            .iter()
            .map(|n| n.as_i64().expect("int") as i32)
            .collect();
        close(
            name,
            item.rect,
            PxRect {
                x: a[0],
                y: a[1],
                w: a[2],
                h: a[3],
            },
            &mut misses,
            &mut total,
        );
    }
    let bar = lay.sheet_bar.expect("sheet bar");
    close(
        "sheet bar",
        bar,
        PxRect {
            x: bar.x,
            y: 924,
            w: bar.w,
            h: 40,
        },
        &mut misses,
        &mut total,
    );
    println!("sheet tabs: {}/{total} within 1 px", total - misses.len());
    assert!(misses.is_empty(), "{misses:#?}");
}

#[test]
fn collapsed_formula_strip_is_shorter_and_grid_moves_up() {
    let spec = AppSpec::sheets();
    let [px, py, _, _] = spec.pane;
    let mk = |expanded| {
        compute(
            &spec,
            &LayoutInput {
                scale: 1.25,
                window: (spec.ref_window[0], spec.ref_window[1]),
                inset: (px, py, px, px),
                tab: 0,
                ribbon_open: true,
                formula_expanded: expanded,
            },
        )
    };
    let (e, c) = (mk(true), mk(false));
    assert_eq!(
        e.content.y, 318,
        "expanded: the grid starts at the measured y"
    );
    assert!(c.content.y < e.content.y);
    assert_eq!(c.formula_field.expect("field").h, 30);
}

/// Grid metrics: header and cell pitch from the pixel scans (units are pixels at 125 %).
#[test]
fn grid_metrics_are_recorded() {
    let r = excel();
    let m = &r["metrics"];
    // Row header: 9 .. 42 is the header, the 999999 border at x=42, cells start at 43.
    let xs = edges(m, "scanRowY500", "999999", 0);
    assert_eq!(xs, vec![42], "row header border");
    // Column pitch is 80 px (79 white + 1 line); row pitch 24 px (23 + 1).
    let cols = edges(m, "scanRowY500", "e0e0e0", 0);
    assert_eq!(cols[1] - cols[0], 80);
    let rows = edges(m, "scanColX300", "e0e0e0", 1);
    assert_eq!(rows[1] - rows[0], 24);
    // Column header: 0a0a0a from y=319 (pane at 318) to the border at 342.
    let hy = edges(m, "scanColX300", "999999", 1);
    assert_eq!(hy, vec![342], "column header border");
}
