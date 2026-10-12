//! Fill check for the b07 and b08 presets (button icons, flow chart caps, tabs and so on).
//!
//! `preset_probes.rs` compares only the stroked outlines, so a shading part (`stroke="0"`, fill
//! only) is invisible to it. `tools/office/make-preset-probes.ps1 -Fill` writes the same presets
//! with a solid fill (fixtures `b07.fill.*` and `b08.fill.*`); here the filled sub-paths of every
//! page are compared with ours: the number of filled paths must be equal and the union of the
//! filled outlines must lie within 0.25 pt of Word's (symmetric Hausdorff distance, as in the
//! stroke comparison). The fill colours (darken, darkenLess, lightenLess) are not in the
//! comparison: the shades were read from the content stream by hand and are documented in the
//! header comments of `presets_b07.rs` and `presets_b08.rs`.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use common::geom::{Cmd, deviation, flatten};
use common::pdf::Pdf;
use open365_drawing::geom::{PathFill, Seg, ShapeDef, evaluate, presets};
use serde_json::Value;

const TOL_PT: f64 = 0.25;
const EMU_PER_PT: f64 = 12_700.0;

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/reference/drawing/presets")
}

fn filled(paint: &str) -> bool {
    matches!(paint, "f" | "F" | "f*" | "B" | "B*" | "b" | "b*")
}

/// Word's filled sub-paths of one page (EMU, box relative, y down) and the number of filled paths.
fn word_page(
    paths: &[common::pdf::Painted],
    origin_emu: f64,
    page_h_pt: f64,
) -> (Vec<Vec<Cmd>>, usize) {
    let o = origin_emu / EMU_PER_PT;
    let fx = |x: f64| (x - o) * EMU_PER_PT;
    let fy = |y: f64| (page_h_pt - y - o) * EMU_PER_PT;
    let mut subs = Vec::new();
    let mut count = 0;
    for p in paths.iter().filter(|p| filled(&p.paint)) {
        count += 1;
        for s in &p.subpaths {
            subs.push(
                s.iter()
                    .map(|c| match *c {
                        Cmd::M(x, y) => Cmd::M(fx(x), fy(y)),
                        Cmd::L(x, y) => Cmd::L(fx(x), fy(y)),
                        Cmd::C(a, b, c2, d, e, f) => {
                            Cmd::C(fx(a), fy(b), fx(c2), fy(d), fx(e), fy(f))
                        }
                        Cmd::Z => Cmd::Z,
                    })
                    .collect(),
            );
        }
    }
    (subs, count)
}

/// Our filled sub-paths (every path whose fill is not `none`) and the number of such paths.
fn our_page(def: &ShapeDef, adj: &[(String, String)], w: f64, h: f64) -> (Vec<Vec<Cmd>>, usize) {
    let ev = evaluate(def, adj, w, h);
    let mut subs: Vec<Vec<Cmd>> = Vec::new();
    let mut count = 0;
    for p in ev.paths.iter().filter(|p| p.fill != PathFill::None) {
        count += 1;
        let mut cur: Vec<Cmd> = Vec::new();
        let mut last = (0.0, 0.0);
        for s in &p.segs {
            match *s {
                Seg::Move(q) => {
                    if !cur.is_empty() {
                        subs.push(std::mem::take(&mut cur));
                    }
                    cur.push(Cmd::M(q.x, q.y));
                    last = (q.x, q.y);
                }
                Seg::Line(q) => {
                    cur.push(Cmd::L(q.x, q.y));
                    last = (q.x, q.y);
                }
                Seg::Quad(c, q) => {
                    let c1 = (
                        last.0 + 2.0 / 3.0 * (c.x - last.0),
                        last.1 + 2.0 / 3.0 * (c.y - last.1),
                    );
                    let c2 = (q.x + 2.0 / 3.0 * (c.x - q.x), q.y + 2.0 / 3.0 * (c.y - q.y));
                    cur.push(Cmd::C(c1.0, c1.1, c2.0, c2.1, q.x, q.y));
                    last = (q.x, q.y);
                }
                Seg::Cubic(a, b, q) => {
                    cur.push(Cmd::C(a.x, a.y, b.x, b.y, q.x, q.y));
                    last = (q.x, q.y);
                }
                Seg::Close => cur.push(Cmd::Z),
            }
        }
        if !cur.is_empty() {
            subs.push(cur);
        }
    }
    (subs, count)
}

fn adjust_overrides(page: &Value) -> Vec<(String, String)> {
    page["adj"]
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), format!("val {}", v.as_i64().unwrap_or(0))))
                .collect()
        })
        .unwrap_or_default()
}

fn to_pt(subs: &[Vec<Cmd>]) -> Vec<Vec<(f64, f64)>> {
    let k = 1.0 / EMU_PER_PT;
    let scaled: Vec<Vec<Cmd>> = subs
        .iter()
        .map(|s| {
            s.iter()
                .map(|c| match *c {
                    Cmd::M(x, y) => Cmd::M(x * k, y * k),
                    Cmd::L(x, y) => Cmd::L(x * k, y * k),
                    Cmd::C(a, b, c2, d, e, f) => Cmd::C(a * k, b * k, c2 * k, d * k, e * k, f * k),
                    Cmd::Z => Cmd::Z,
                })
                .collect()
        })
        .collect();
    flatten(&scaled)
}

#[test]
fn filled_parts_match_word_for_b07_and_b08() {
    let ours: BTreeMap<&str, ShapeDef> = presets::table().into_iter().collect();
    let mut failures = Vec::new();
    let mut checked = 0;
    for name in ["b07.fill", "b08.fill"] {
        let (Ok(probes), Ok(pdf)) = (
            fs::read(dir().join(format!("{name}.probes.json"))),
            fs::read(dir().join(format!("{name}.pdf"))),
        ) else {
            continue;
        };
        let probes: Value = serde_json::from_slice(&probes).expect("probes.json");
        let word = Pdf::parse(&pdf).page_paths();
        let origin = probes["origin_emu"].as_f64().unwrap_or(914_400.0);
        let page_h = probes["page_pt"][1].as_f64().unwrap_or(792.0);
        let pages = probes["pages"].as_array().expect("pages");
        assert_eq!(pages.len(), word.len(), "{name}: page count");
        for (pg, wp) in pages.iter().zip(&word) {
            let preset = pg["preset"].as_str().expect("preset");
            let Some(def) = ours.get(preset) else {
                continue;
            };
            let (w, h) = (
                pg["w_emu"].as_f64().unwrap_or(0.0),
                pg["h_emu"].as_f64().unwrap_or(0.0),
            );
            let (theirs, wn) = word_page(wp, origin, page_h);
            let (mine, on) = our_page(def, &adjust_overrides(pg), w, h);
            // Word drops the fill of an outline without area (the all-zero adjust pages), so
            // there is nothing to compare.
            if wn == 0 {
                continue;
            }
            checked += 1;
            if wn != on {
                failures.push(format!(
                    "{preset} {}: {on} filled paths (Word {wn})",
                    pg["kind"]
                ));
                continue;
            }
            let d = deviation(&to_pt(&mine), &to_pt(&theirs));
            if d.is_nan() || d > TOL_PT {
                failures.push(format!("{preset} {}: fill deviation {d:.3} pt", pg["kind"]));
            }
        }
    }
    assert!(checked > 0, "no fill fixtures found");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
