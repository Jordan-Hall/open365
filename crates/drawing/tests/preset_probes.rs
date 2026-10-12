//! Clean-room preset check: our preset table against Word's own output.
//!
//! `tools/office/make-preset-probes.ps1` makes a .docx with one preset per page (sizes, adjust
//! values) and has Word export it to PDF; both, plus `<batch>.probes.json` and
//! `<batch>.handles.json`, live in `fixtures/reference/drawing/presets`. This test reads each
//! shape's stroked path out of the PDF, normalises it to the shape box (EMU, origin top left),
//! evaluates our preset at the probe's size and adjusts, and compares the two outlines within
//! `TOL_PT`. A preset missing from `presets::table()` is reported as missing, not failed.
//!
//! `cargo test -p open365-drawing --test preset_probes -- --nocapture` prints the report.
//! `... -- --ignored write_word_paths` rewrites `<batch>.paths.json` (Word's measured paths).

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use common::geom::{Cmd, deviation, flatten};
use common::pdf::Pdf;
use open365_drawing::geom::{Formula, Scope, Seg, ShapeDef, evaluate, presets};
use serde_json::{Value, json};

/// Largest allowed deviation between our outline and Word's, in points.
const TOL_PT: f64 = 0.25;
const EMU_PER_PT: f64 = 12_700.0;
const MANIFEST_COUNT: usize = 187;

/// Probe pages (preset, page kind) where Word's outline comes from degenerate arithmetic that
/// our guides do not reproduce (collapsed inner ellipses and zero-size heads of the circular
/// arrows). They are reported as known, not failed; a page here that starts to pass fails the
/// test so that the list cannot go stale. Listed in docs/architecture/drawing.md.
const KNOWN_UNMATCHED: &[(&str, &str)] = &[
    ("circularArrow", "adj-all-400000"),
    ("leftCircularArrow", "adj-all-0"),
    ("leftCircularArrow", "adj-all-30000"),
    ("leftCircularArrow", "adj-all-100000"),
    ("leftCircularArrow", "adj-all--50000"),
    ("leftCircularArrow", "adj-all-400000"),
    ("leftCircularArrow", "scan-adj5"),
    ("leftRightCircularArrow", "adj-all-0"),
    ("leftRightCircularArrow", "adj-all-100000"),
    ("leftRightCircularArrow", "adj-all--50000"),
    ("leftRightCircularArrow", "adj-all-400000"),
    ("leftRightCircularArrow", "scan-adj5"),
];

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/reference/drawing/presets")
}

struct Batch {
    name: String,
    probes: Value,
    handles: Option<Value>,
    pdf: Vec<u8>,
}

fn load_batches() -> Vec<Batch> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(dir()) else {
        return out;
    };
    let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    files.sort();
    for f in files {
        let Some(fname) = f.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(name) = fname.strip_suffix(".probes.json") else {
            continue;
        };
        let (Ok(probes), Ok(pdf)) = (fs::read(&f), fs::read(dir().join(format!("{name}.pdf"))))
        else {
            continue;
        };
        let handles = fs::read(dir().join(format!("{name}.handles.json")))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        out.push(Batch {
            name: name.to_owned(),
            probes: serde_json::from_slice(&probes).expect("probes.json is JSON"),
            handles,
            pdf,
        });
    }
    out
}

/// Word's stroked subpaths for one page, in EMU relative to the shape box (y down).
fn word_page(paths: &[common::pdf::Painted], origin_emu: f64, page_h_pt: f64) -> Vec<Vec<Cmd>> {
    let o = origin_emu / EMU_PER_PT;
    let fx = |x: f64| (x - o) * EMU_PER_PT;
    let fy = |y: f64| (page_h_pt - y - o) * EMU_PER_PT;
    let mut subs = Vec::new();
    for p in paths.iter().filter(|p| p.stroked()) {
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
    subs
}

/// Our stroked subpaths for a probe, in EMU.
fn our_page(def: &ShapeDef, adj: &[(String, String)], w: f64, h: f64) -> Vec<Vec<Cmd>> {
    let ev = evaluate(def, adj, w, h);
    let mut subs: Vec<Vec<Cmd>> = Vec::new();
    for p in ev.paths.iter().filter(|p| p.stroke) {
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
                    let (c1, c2) = (
                        (
                            last.0 + 2.0 / 3.0 * (c.x - last.0),
                            last.1 + 2.0 / 3.0 * (c.y - last.1),
                        ),
                        (q.x + 2.0 / 3.0 * (c.x - q.x), q.y + 2.0 / 3.0 * (c.y - q.y)),
                    );
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
    subs
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

/// Keeps the subpaths Word would paint: not a dot (zero length or tiny), and not wholly off the page.
/// Word's PDF drops operations that lie wholly off the page and some, but not all, zero-length
/// strokes, so both sides are filtered the same way. `subs` are in EMU relative to the shape box.
fn painted_only(subs: Vec<Vec<Cmd>>, origin_emu: f64, page_pt: (f64, f64)) -> Vec<Vec<Cmd>> {
    let (lo_x, hi_x) = (-origin_emu, page_pt.0 * EMU_PER_PT - origin_emu);
    let (lo_y, hi_y) = (-origin_emu, page_pt.1 * EMU_PER_PT - origin_emu);
    subs.into_iter()
        .filter(|s| {
            let mut b = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            let mut add = |x: f64, y: f64| {
                b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
            };
            for c in s {
                match *c {
                    Cmd::M(x, y) | Cmd::L(x, y) => add(x, y),
                    Cmd::C(a, bb, c2, d, e, f) => {
                        add(a, bb);
                        add(c2, d);
                        add(e, f);
                    }
                    Cmd::Z => {}
                }
            }
            // A dot smaller than 0.05 pt is inside the tolerance anyway.
            let point =
                (b.2 - b.0).abs() < 0.05 * EMU_PER_PT && (b.3 - b.1).abs() < 0.05 * EMU_PER_PT;
            !point && b.2 >= lo_x && b.0 <= hi_x && b.3 >= lo_y && b.1 <= hi_y
        })
        .collect()
}

#[derive(Default)]
struct Outcome {
    pages: usize,
    failed_pages: usize,
    known_pages: usize,
    stale_known: usize,
    worst: f64,
    worst_kind: String,
    handle_note: Option<String>,
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

/// Our adjust defaults against the counts and defaults Word reports through its object model.
fn handle_check(def: &ShapeDef, handle: &Value) -> Option<String> {
    let want = handle["count"].as_u64()? as usize;
    if def.adjust.len() != want {
        return Some(format!("adjust count {} (Word {want})", def.adjust.len()));
    }
    let scope = Scope::new(914_400.0, 914_400.0);
    for (i, (g, w)) in def
        .adjust
        .iter()
        .zip(handle["values"].as_array()?)
        .enumerate()
    {
        let got = g.fmla.eval(&scope).ok()?;
        let w = w.as_f64()?;
        let is_angle = w.abs() >= 2.0;
        let want_raw = if is_angle {
            w * 60_000.0
        } else {
            w * 100_000.0
        };
        // Word reports an angle in -180..180 degrees; the default may be the same angle
        // written as 0..360.
        let mut diff = (got - want_raw).abs();
        if is_angle {
            let turn = 21_600_000.0;
            diff = (got - want_raw).rem_euclid(turn);
            diff = diff.min(turn - diff);
        }
        if diff > 1.0 {
            return Some(format!("adjust {} default {got} (Word {want_raw})", i + 1));
        }
    }
    None
}

fn table() -> BTreeMap<&'static str, ShapeDef> {
    presets::table().into_iter().collect()
}

fn run_batch(
    b: &Batch,
    ours: &BTreeMap<&'static str, ShapeDef>,
) -> (BTreeMap<String, Option<Outcome>>, Vec<String>) {
    let word = Pdf::parse(&b.pdf).page_paths();
    let pages = b.probes["pages"].as_array().expect("pages");
    assert_eq!(
        word.len(),
        pages.len(),
        "{}: PDF page count differs from probes.json",
        b.name
    );
    let origin = b.probes["origin_emu"].as_f64().unwrap_or(914_400.0);
    let page_h = b.probes["page_pt"][1].as_f64().unwrap_or(792.0);
    let page_w = b.probes["page_pt"][0].as_f64().unwrap_or(612.0);
    let mut res: BTreeMap<String, Option<Outcome>> = BTreeMap::new();
    let mut lines = Vec::new();
    for (pg, wp) in pages.iter().zip(&word) {
        let name = pg["preset"].as_str().expect("preset").to_owned();
        let Some(def) = ours.get(name.as_str()) else {
            res.entry(name).or_insert(None);
            continue;
        };
        let entry = res.entry(name.clone()).or_insert_with(|| {
            let mut o = Outcome::default();
            if let Some(h) = b.handles.as_ref().map(|h| &h["presets"][name.as_str()]) {
                o.handle_note = handle_check(def, h);
            }
            Some(o)
        });
        let Some(o) = entry.as_mut() else { continue };
        let (w, h) = (
            pg["w_emu"].as_f64().unwrap_or(0.0),
            pg["h_emu"].as_f64().unwrap_or(0.0),
        );
        let page = (page_w, page_h);
        let mine = painted_only(our_page(def, &adjust_overrides(pg), w, h), origin, page);
        let theirs = painted_only(word_page(wp, origin, page_h), origin, page);
        let d = deviation(&to_pt(&mine), &to_pt(&theirs));
        o.pages += 1;
        if d > o.worst || o.worst_kind.is_empty() {
            o.worst = d;
            o.worst_kind = pg["kind"].as_str().unwrap_or("?").to_owned();
        }
        let kind = pg["kind"].as_str().unwrap_or("?");
        let known = KNOWN_UNMATCHED.contains(&(name.as_str(), kind));
        if known && !(d > TOL_PT || d.is_nan()) {
            o.stale_known += 1;
            lines.push(format!(
                "  {name} page {} {kind} now matches: remove it from KNOWN_UNMATCHED",
                pg["page"]
            ));
        }
        if d > TOL_PT || d.is_nan() {
            if known {
                o.known_pages += 1;
            } else {
                o.failed_pages += 1;
            }
            lines.push(format!(
                "  {}{name} page {} {} ({:.1}x{:.1} pt, adj {}): deviation {}",
                if known { "[known] " } else { "" },
                pg["page"],
                pg["kind"].as_str().unwrap_or("?"),
                w / EMU_PER_PT,
                h / EMU_PER_PT,
                pg["adj"],
                if d.is_finite() {
                    format!("{d:.3} pt")
                } else {
                    "more than 8 pt (or an outline is missing)".into()
                }
            ));
        }
    }
    (res, lines)
}

#[test]
fn table_matches_word_within_tolerance() {
    let ours = table();
    let batches = load_batches();
    assert!(
        !batches.is_empty(),
        "no probe batches in {}",
        dir().display()
    );
    let (mut probed, mut pass, mut fail, mut missing, mut known) = (0, 0, 0, 0, 0);
    let mut bad = Vec::new();
    // A preset probed in several batches (the pilot repeats five) counts once, in its first batch.
    let mut seen = std::collections::BTreeSet::new();
    println!("preset                         status   pages  worst (pt)  worst page kind");
    for b in &batches {
        let (res, lines) = run_batch(b, &ours);
        for (name, o) in &res {
            let first = seen.insert(name.clone());
            probed += usize::from(first);
            match o {
                None => {
                    missing += usize::from(first);
                    println!("{name:<30} missing");
                }
                Some(o) => {
                    let ok = o.failed_pages == 0 && o.handle_note.is_none() && o.stale_known == 0;
                    if ok {
                        pass += usize::from(first);
                        known += usize::from(first && o.known_pages > 0);
                    } else {
                        fail += usize::from(first);
                        bad.push(name.clone());
                    }
                    println!(
                        "{name:<30} {:<8} {:>2}/{:<2}   {:>9}   {}{}",
                        if !ok {
                            "FAIL"
                        } else if o.known_pages > 0 {
                            "known"
                        } else {
                            "pass"
                        },
                        o.pages - o.failed_pages - o.known_pages,
                        o.pages,
                        if o.worst.is_finite() {
                            format!("{:.3}", o.worst)
                        } else {
                            "inf".into()
                        },
                        o.worst_kind,
                        o.handle_note
                            .as_ref()
                            .map(|n| format!("  handles: {n}"))
                            .unwrap_or_default()
                    );
                }
            }
        }
        for l in lines {
            println!("{l}");
        }
    }
    let present = pass + fail;
    let exact = pass - known;
    println!(
        "batches {}: probed {probed}, pass {pass} (of which {known} with known-unmatched pages, so {exact} match every page), fail {fail}, missing {missing}; coverage of probed presets {:.1}% ({present}/{probed}), of all {MANIFEST_COUNT} presets {:.1}% ({}/{MANIFEST_COUNT}); tolerance {TOL_PT} pt",
        batches
            .iter()
            .map(|b| b.name.as_str())
            .collect::<Vec<_>>()
            .join(","),
        100.0 * present as f64 / probed.max(1) as f64,
        100.0 * ours.len() as f64 / MANIFEST_COUNT as f64,
        ours.len()
    );
    assert!(bad.is_empty(), "presets off Word's output: {bad:?}");
}

#[test]
fn every_table_entry_parses_and_is_named_in_the_manifest() {
    let manifest: Value =
        serde_json::from_slice(&fs::read(dir().join("manifest.json")).expect("manifest.json"))
            .expect("JSON");
    let mut known: Vec<String> = Vec::new();
    for names in manifest["batches"].as_object().expect("batches").values() {
        known.extend(
            names
                .as_array()
                .expect("array")
                .iter()
                .filter_map(|n| n.as_str().map(str::to_owned)),
        );
    }
    assert_eq!(known.len(), MANIFEST_COUNT);
    let t = presets::table();
    assert!(!t.is_empty());
    for (name, def) in &t {
        assert!(
            known.iter().any(|k| k == name),
            "{name} is not an ST_ShapeType name in the manifest"
        );
        let e = evaluate(def, &[], 1_828_800.0, 914_400.0);
        assert!(e.diagnostics.is_empty(), "{name}: {:?}", e.diagnostics);
    }
}

#[test]
fn manifest_batches_are_a_partition_of_the_187_names() {
    let manifest: Value =
        serde_json::from_slice(&fs::read(dir().join("manifest.json")).expect("manifest.json"))
            .expect("JSON");
    let mut all: Vec<&str> = manifest["batches"]
        .as_object()
        .expect("batches")
        .values()
        .flat_map(|v| {
            v.as_array()
                .expect("array")
                .iter()
                .filter_map(Value::as_str)
        })
        .collect();
    let n = all.len();
    all.sort_unstable();
    all.dedup();
    assert_eq!((n, all.len()), (MANIFEST_COUNT, MANIFEST_COUNT));
    for p in manifest["pilot"].as_array().expect("pilot") {
        assert!(all.contains(&p.as_str().expect("name")));
    }
}

fn cmd_json(c: &Cmd) -> Value {
    let r = |v: f64| (v * 10.0).round() / 10.0;
    match *c {
        Cmd::M(x, y) => json!(["M", r(x), r(y)]),
        Cmd::L(x, y) => json!(["L", r(x), r(y)]),
        Cmd::C(a, b, c2, d, e, f) => json!(["C", r(a), r(b), r(c2), r(d), r(e), r(f)]),
        Cmd::Z => json!(["Z"]),
    }
}

/// Writes Word's measured paths (EMU, relative to the shape box, y down) next to each batch's PDF,
/// so a preset can be authored from numbers without reading the PDF.
#[test]
#[ignore = "rewrites fixtures; run on purpose"]
fn write_word_paths() {
    for b in load_batches() {
        let word = Pdf::parse(&b.pdf).page_paths();
        let origin = b.probes["origin_emu"].as_f64().unwrap_or(914_400.0);
        let page_h = b.probes["page_pt"][1].as_f64().unwrap_or(792.0);
        let pages: Vec<Value> = b.probes["pages"]
            .as_array()
            .expect("pages")
            .iter()
            .zip(&word)
            .map(|(pg, wp)| {
                let subs = word_page(wp, origin, page_h);
                json!({
                    "page": pg["page"], "preset": pg["preset"], "kind": pg["kind"],
                    "w_emu": pg["w_emu"], "h_emu": pg["h_emu"], "adj": pg["adj"],
                    "subpaths": subs.iter().map(|s| s.iter().map(cmd_json).collect::<Vec<_>>()).collect::<Vec<_>>(),
                })
            })
            .collect();
        let doc = json!({
            "batch": b.name,
            "note": "Word's stroked paths, EMU relative to the shape box (top left origin, y down), from the batch PDF",
            "pages": pages,
        });
        fs::write(
            dir().join(format!("{}.paths.json", b.name)),
            serde_json::to_vec(&doc).expect("json"),
        )
        .expect("write");
    }
}

/// The harness must be able to fail: a wrong shape and a wrong adjust default are both caught.
#[test]
fn harness_detects_a_wrong_preset() {
    let batches = load_batches();
    let b = batches
        .iter()
        .find(|b| b.name == "pilot")
        .expect("pilot batch");
    let mut wrong = BTreeMap::new();
    wrong.insert("roundRect", ShapeDef::rect());
    let (res, lines) = run_batch(b, &wrong);
    let o = res["roundRect"].as_ref().expect("probed");
    assert!(
        o.failed_pages > 0 && !lines.is_empty(),
        "a rectangle must not pass as roundRect"
    );
    // A rectangle is right for `rect` pages, so those pass.
    let mut right = BTreeMap::new();
    right.insert("rect", ShapeDef::rect());
    let o = run_batch(b, &right)
        .0
        .remove("rect")
        .flatten()
        .expect("probed");
    assert_eq!(o.failed_pages, 0);
    // Word reports 0.16667 for roundRect: an adjust default of 20000 is flagged.
    let mut def = presets::table()
        .into_iter()
        .find(|(n, _)| *n == "roundRect")
        .expect("roundRect")
        .1;
    def.adjust[0].fmla = Formula::parse("val 20000").expect("parse");
    let h = &b.handles.as_ref().expect("handles")["presets"]["roundRect"];
    assert!(handle_check(&def, h).is_some());
}

#[test]
fn rect_sanity_formula_parse() {
    // The harness evaluates `val` formulas for defaults: make sure the helper path works.
    let f = Formula::parse("val 16667").expect("parse");
    assert_eq!(f.eval(&Scope::new(1.0, 1.0)).expect("eval"), 16667.0);
}
