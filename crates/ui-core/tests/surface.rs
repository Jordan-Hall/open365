//! The proof surface: the Excel Home ribbon tab row and the File button, built from the ui-shell
//! spec JSON, compared with the measured reference (`fixtures/reference/ui/excel-ribbon.json`,
//! taken at 125 % in a 1600 x 1000 window).
//!
//! Two comparisons: rectangles (exact at 125 %, derived at other scales) and pixels (the widget
//! tree's outlines against outlines drawn straight from the fixture rectangles).

use open365_gfx::{Color, Rect as GRect};
use open365_ui_core::chrome::{RowSpec, build_tab_row};
use open365_ui_core::harness::Harness;
use open365_ui_core::paint::DEBUG_COLOR;
use open365_ui_core::{PaintLayers, Scale};
use serde_json::Value;

fn reference() -> Value {
    let path = format!(
        "{}/../../fixtures/reference/ui/excel-ribbon.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("json")
}

fn rect(v: &Value) -> [i32; 4] {
    let a: Vec<i32> = v["rect"]
        .as_array()
        .expect("rect")
        .iter()
        .map(|n| n.as_i64().expect("int") as i32)
        .collect();
    [a[0], a[1], a[2], a[3]]
}

/// Fixture rectangles as `[x, y, w, h]` at 125 %: the File button, then the tabs.
fn reference_rects() -> Vec<[i32; 4]> {
    let r = reference();
    let file = r["chrome"]["elements"]
        .as_array()
        .expect("elements")
        .iter()
        .find(|e| e["id"] == "FileTabButton")
        .map(rect)
        .expect("file tab");
    let mut out = vec![file];
    out.extend(r["tabs"].as_array().expect("tabs").iter().map(rect));
    out
}

fn build(scale: f32) -> (Harness, open365_ui_core::chrome::TabRow) {
    let spec = RowSpec::sheets();
    let win = spec.window();
    let mut h = Harness::new(win.w, win.h, scale);
    let row = build_tab_row(&mut h.ui, &spec);
    (h, row)
}

#[test]
fn layout_matches_the_reference_exactly_at_125_percent() {
    let (mut h, _) = build(1.25);
    h.ui.tree.layout();
    let got: Vec<[i32; 4]> =
        h.ui.debug_rects()
            .iter()
            .map(|r| [r.x0, r.y0, r.width(), r.height()])
            .collect();
    let want = reference_rects();
    assert_eq!(got.len(), want.len(), "File button plus ten tabs");
    let mut worst = 0;
    for (g, w) in got.iter().zip(&want) {
        for k in 0..4 {
            worst = worst.max((g[k] - w[k]).abs());
        }
        assert_eq!(g, w, "rectangle differs from the fixture");
    }
    println!(
        "tab row at 125 %: {} rectangles, worst edge delta {worst} px",
        got.len()
    );
}

#[test]
fn other_scales_stay_within_one_pixel_of_the_scaled_reference() {
    for scale in [1.0_f32, 1.5, 1.75, 2.0] {
        let (mut h, _) = build(scale);
        h.ui.tree.layout();
        let k = scale / 1.25;
        let sc = Scale(scale);
        let mut worst = 0;
        for (g, w) in h.ui.debug_rects().iter().zip(reference_rects()) {
            let (x0, y0) = (sc.edge(w[0] as f32 / 1.25), sc.edge(w[1] as f32 / 1.25));
            let (x1, y1) = (
                sc.edge((w[0] + w[2]) as f32 / 1.25),
                sc.edge((w[1] + w[3]) as f32 / 1.25),
            );
            worst = worst
                .max((g.x0 - x0).abs())
                .max((g.y0 - y0).abs())
                .max((g.x1 - x1).abs())
                .max((g.y1 - y1).abs());
            // Against the plainly scaled fixture the edges may differ by rounding only.
            let plain = [
                (w[0] as f32 * k).round() as i32,
                (w[0] as f32 * k + w[2] as f32 * k).round() as i32,
            ];
            assert!(
                (g.x0 - plain[0]).abs() <= 1 && (g.x1 - plain[1]).abs() <= 1,
                "scale {scale}: {g:?} vs {w:?}"
            );
        }
        assert_eq!(worst, 0, "edges are round(u / 1.25 * scale)");
        println!("tab row at {scale}: all edges within 1 px of the scaled reference");
    }
}

#[test]
fn pixel_diff_against_the_fixture_rectangles_is_zero_at_125_percent() {
    let (mut h, _) = build(1.25);
    let Some(ours) = h.render(PaintLayers {
        backgrounds: false,
        text: false,
        debug_bounds: true,
    }) else {
        eprintln!("skipping pixel diff: no GPU adapter");
        return;
    };
    let clear = h.ui.theme.color(open365_ui_core::Token::Frame);
    let theirs = h
        .render_with(clear, |list| {
            for w in reference_rects() {
                list.rect_outline(
                    GRect::from_xywh(w[0] as f32, w[1] as f32, w[2] as f32, w[3] as f32),
                    1.0,
                    DEBUG_COLOR,
                );
            }
        })
        .expect("reference render");
    let d = ours.diff(&theirs, 0);
    println!(
        "pixel diff at 125 %: {} of {} pixels differ (max channel delta {}), outline pixels drawn: {}",
        d.mismatched,
        d.pixels,
        d.max_delta,
        ours.count(DEBUG_COLOR)
    );
    assert!(ours.count(DEBUG_COLOR) > 1000, "outlines were drawn");
    assert_eq!(d.mismatched, 0);
}

#[test]
fn pixel_diff_at_other_scales_against_snapped_reference_rectangles() {
    for scale in [1.0_f32, 1.5, 2.0] {
        let (mut h, _) = build(scale);
        let Some(ours) = h.render(PaintLayers {
            backgrounds: false,
            text: false,
            debug_bounds: true,
        }) else {
            eprintln!("skipping pixel diff: no GPU adapter");
            return;
        };
        let clear = h.ui.theme.color(open365_ui_core::Token::Frame);
        let sc = Scale(scale);
        let theirs = h
            .render_with(clear, |list| {
                for w in reference_rects() {
                    let (x0, y0) = (sc.edge(w[0] as f32 / 1.25), sc.edge(w[1] as f32 / 1.25));
                    let (x1, y1) = (
                        sc.edge((w[0] + w[2]) as f32 / 1.25),
                        sc.edge((w[1] + w[3]) as f32 / 1.25),
                    );
                    list.rect_outline(
                        GRect::from_edges(x0 as f32, y0 as f32, x1 as f32, y1 as f32),
                        1.0,
                        DEBUG_COLOR,
                    );
                }
            })
            .expect("reference render");
        let d = ours.diff(&theirs, 0);
        println!(
            "pixel diff at {scale}: {} of {} pixels differ",
            d.mismatched, d.pixels
        );
        assert_eq!(d.mismatched, 0, "scale {scale}");
    }
}

#[test]
fn the_full_row_paints_labels_and_the_active_tab_underline() {
    let (mut h, row) = build(1.25);
    let Some(img) = h.render(PaintLayers::default()) else {
        eprintln!("skipping render: no GPU adapter");
        return;
    };
    // The default settings are light mode: dark text on the light frame, accent underline under Home.
    let accent = h.ui.theme.color(open365_ui_core::Token::TabUnderline);
    assert!(img.count(accent) > 20, "underline of the selected tab");
    let ink = h.ui.theme.color(open365_ui_core::Token::TabText);
    assert!(img.count(ink) > 100, "label pixels");
    let r = h.ui.tree.device_rect(row.file, Scale(1.25));
    let label_ink =
        (r.y0..r.y1).any(|y| (r.x0..r.x1).any(|x| img.pixel(x as u32, y as u32)[0] < 200));
    assert!(label_ink, "the File label is drawn");
    let _ = Color::BLACK;
}
