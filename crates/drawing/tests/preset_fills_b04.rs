//! Fill check for batch b04: the unstroked and shaded parts of the presets against Word's output.
//!
//! `preset_probes.rs` compares stroked outlines only. The b04 probes were made with `-Fill`
//! (a grey 0xC0 fill), so Word's PDF also holds the filled faces: the lighter and darker faces of
//! `cube`, `can`, `bevel`, `foldedCorner`, the eyes of `smileyFace`, the pie of `arc`, and so on.
//! This test reads every filled path of a page together with its grey level, maps our fill modes
//! to the grey Word gives them (`norm` 0.75, `lighten` 0.85, `lightenLess` 0.80, `darken` 0.45,
//! `darkenLess` 0.60 for a base of 0.753), and compares the outlines per grey level within the same
//! 0.25 pt as the stroke check.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use common::geom::{Cmd, deviation, flatten};
use common::pdf::{Painted, Pdf, paint_ops};
use open365_drawing::geom::{PathFill, Seg, evaluate, presets};
use serde_json::Value;

const TOL_PT: f64 = 0.25;
const EMU_PER_PT: f64 = 12_700.0;
/// Largest area, in square points, of a face that may be present on one side only.
const SLIVER_PT2: f64 = 50.0;
const BATCH: &str = "b04";

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/reference/drawing/presets")
}

/// Grey level of a fill mode, as a key in hundredths, for the 0xC0 base fill of the probes.
fn fill_key(f: PathFill) -> Option<i32> {
    let base = 192.0_f64;
    let g = match f {
        PathFill::None => return None,
        PathFill::Norm => base,
        PathFill::Lighten => base + (255.0 - base) * 0.4,
        PathFill::LightenLess => base + (255.0 - base) * 0.2,
        PathFill::Darken => base * 0.6,
        PathFill::DarkenLess => base * 0.8,
    };
    Some((g / 255.0 * 100.0).round() as i32)
}

/// Splits a content stream at its `<gray> g` operators; each part carries the grey in force.
fn gray_parts(content: &[u8]) -> Vec<(f64, Vec<u8>)> {
    let s = String::from_utf8_lossy(content).into_owned();
    let base = s.as_ptr() as usize;
    let toks: Vec<(usize, &str)> = s
        .split_ascii_whitespace()
        .map(|t| (t.as_ptr() as usize - base, t))
        .collect();
    let mut parts = Vec::new();
    let (mut gray, mut start) = (0.0, 0);
    for i in 1..toks.len() {
        if toks[i].1 == "g"
            && let Ok(v) = toks[i - 1].1.parse::<f64>()
        {
            parts.push((gray, s.as_bytes()[start..toks[i - 1].0].to_vec()));
            gray = v;
            start = toks[i].0 + 1;
        }
    }
    parts.push((gray, s.as_bytes()[start..].to_vec()));
    parts
}

/// Word's filled subpaths of one page by grey key, in points relative to the shape box (y down).
fn word_fills(content: &[u8], origin_pt: f64, page_h_pt: f64) -> BTreeMap<i32, Vec<Vec<Cmd>>> {
    let mut out: BTreeMap<i32, Vec<Vec<Cmd>>> = BTreeMap::new();
    for (gray, part) in gray_parts(content) {
        let key = (gray * 100.0).round() as i32;
        let painted: Vec<Painted> = paint_ops(&part);
        for p in painted
            .iter()
            .filter(|p| matches!(p.paint.as_str(), "f" | "f*" | "F"))
        {
            for s in &p.subpaths {
                let fx = |x: f64| x - origin_pt;
                let fy = |y: f64| page_h_pt - y - origin_pt;
                out.entry(key).or_default().push(
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
    }
    out
}

/// Our filled subpaths by grey key, in points relative to the shape box.
fn our_fills(
    def: &open365_drawing::geom::ShapeDef,
    adj: &[(String, String)],
    w: f64,
    h: f64,
) -> BTreeMap<i32, Vec<Vec<Cmd>>> {
    let k = 1.0 / EMU_PER_PT;
    let ev = evaluate(def, adj, w, h);
    let mut out: BTreeMap<i32, Vec<Vec<Cmd>>> = BTreeMap::new();
    for p in &ev.paths {
        let Some(key) = fill_key(p.fill) else {
            continue;
        };
        let mut cur: Vec<Cmd> = Vec::new();
        let mut last = (0.0, 0.0);
        let mut flush = |cur: &mut Vec<Cmd>| {
            if !cur.is_empty() {
                out.entry(key).or_default().push(std::mem::take(cur));
            }
        };
        for s in &p.segs {
            match *s {
                Seg::Move(q) => {
                    flush(&mut cur);
                    cur.push(Cmd::M(q.x * k, q.y * k));
                    last = (q.x * k, q.y * k);
                }
                Seg::Line(q) => {
                    cur.push(Cmd::L(q.x * k, q.y * k));
                    last = (q.x * k, q.y * k);
                }
                Seg::Quad(c, q) => {
                    let (c, q) = ((c.x * k, c.y * k), (q.x * k, q.y * k));
                    cur.push(Cmd::C(
                        last.0 + 2.0 / 3.0 * (c.0 - last.0),
                        last.1 + 2.0 / 3.0 * (c.1 - last.1),
                        q.0 + 2.0 / 3.0 * (c.0 - q.0),
                        q.1 + 2.0 / 3.0 * (c.1 - q.1),
                        q.0,
                        q.1,
                    ));
                    last = q;
                }
                Seg::Cubic(a, b, q) => {
                    cur.push(Cmd::C(a.x * k, a.y * k, b.x * k, b.y * k, q.x * k, q.y * k));
                    last = (q.x * k, q.y * k);
                }
                Seg::Close => cur.push(Cmd::Z),
            }
        }
        flush(&mut cur);
    }
    out
}

/// Area of flattened polygons (shoelace, each polygon on its own), in square points.
fn area(polys: &[Vec<(f64, f64)>]) -> f64 {
    polys
        .iter()
        .map(|p| {
            p.windows(2)
                .map(|w| w[0].0 * w[1].1 - w[1].0 * w[0].1)
                .sum::<f64>()
                .abs()
                / 2.0
        })
        .sum()
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

#[test]
fn fills_match_word_b04() {
    let (Ok(probes), Ok(pdf)) = (
        fs::read(dir().join(format!("{BATCH}.probes.json"))),
        fs::read(dir().join(format!("{BATCH}.pdf"))),
    ) else {
        panic!("no {BATCH} probes in {}", dir().display());
    };
    let probes: Value = serde_json::from_slice(&probes).expect("probes.json");
    let pdf = Pdf::parse(&pdf);
    let ids = pdf.page_ids();
    let pages = probes["pages"].as_array().expect("pages");
    assert_eq!(
        ids.len(),
        pages.len(),
        "PDF page count differs from probes.json"
    );
    let table: BTreeMap<&str, _> = presets::table().into_iter().collect();
    let origin = probes["origin_emu"].as_f64().unwrap_or(914_400.0) / EMU_PER_PT;
    let page_h = probes["page_pt"][1].as_f64().unwrap_or(792.0);

    let mut worst: BTreeMap<String, (f64, String)> = BTreeMap::new();
    let mut checked = BTreeSet::new();
    for (pg, id) in pages.iter().zip(&ids) {
        let name = pg["preset"].as_str().expect("preset");
        let Some(def) = table.get(name) else { continue };
        checked.insert(name.to_owned());
        let (w, h) = (
            pg["w_emu"].as_f64().unwrap_or(0.0),
            pg["h_emu"].as_f64().unwrap_or(0.0),
        );
        let theirs = word_fills(&pdf.page_content(*id), origin, page_h);
        let mine = our_fills(def, &adjust_overrides(pg), w, h);
        let keys: BTreeSet<i32> = theirs.keys().chain(mine.keys()).copied().collect();
        for key in keys {
            // the grey 0 (text) and white parts are not shape fills
            if key <= 0 || key >= 100 {
                continue;
            }
            let a = flatten(mine.get(&key).map(Vec::as_slice).unwrap_or(&[]));
            let b = flatten(theirs.get(&key).map(Vec::as_slice).unwrap_or(&[]));
            // A face squashed to a sliver (a probe with adjust 0) may be left out by either side.
            if (a.is_empty() && area(&b) < SLIVER_PT2) || (b.is_empty() && area(&a) < SLIVER_PT2) {
                continue;
            }
            let d = if a.is_empty() && b.is_empty() {
                0.0
            } else {
                deviation(&a, &b)
            };
            let d = if d.is_nan() { f64::INFINITY } else { d };
            let e = worst.entry(name.to_owned()).or_insert((0.0, String::new()));
            if d > e.0 {
                *e = (
                    d,
                    format!(
                        "page {} {} grey {key} (ours {} outlines, Word {})",
                        pg["page"],
                        pg["kind"].as_str().unwrap_or("?"),
                        a.len(),
                        b.len()
                    ),
                );
            }
        }
    }
    let mut bad = Vec::new();
    for name in &checked {
        let (d, at) = worst.get(name).cloned().unwrap_or_default();
        println!("{name:<24} fills worst {d:.3} pt {at}");
        if d > TOL_PT || d.is_nan() {
            bad.push(name.clone());
        }
    }
    assert!(!checked.is_empty(), "no b04 preset in the table");
    assert!(bad.is_empty(), "fills off Word's output: {bad:?}");
}
