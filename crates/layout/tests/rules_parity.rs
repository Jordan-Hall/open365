//! Compares the paragraph border rules (`Page::rules`) with the filled rectangles Word draws.
//!
//! Reference: the `rules` array of `fixtures/reference/layout/<name>.json`, written by
//! `tools/office/word-layout-oracle.ps1 -Rules` from Word's 600 dpi PDF. Word cuts a box into
//! corner squares, bars and one segment per line; the engine reports whole bars, so rectangles
//! are compared as covered area: every reference rectangle must lie inside the engine's rules
//! and every engine rule inside the reference's, within `TOLERANCE` on each edge.
//!
//! `fixtures/reference/layout/rules-floor.toml` holds the matched counts; like the other floors
//! it only goes up (`UPDATE_PARITY_FLOOR=1` raises it). `PARITY_DETAIL=<fixture>` lists every
//! rectangle that has no partner.

mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;

use common::{Mode, library, root};
use open365_docx::Document;
use open365_layout::{LayoutOptions, layout_document};
use open365_opc::Limits;
use serde_json::Value;

/// Edge tolerance in points: the PDF's rounding leaves about 0.02 pt after the scale is undone.
const TOLERANCE: f64 = 0.05;
/// Word's PDF is 1.0004 times the nominal scale across and 1.0003 down (see `common`).
const SCALE_X: f64 = 1.0004;
const SCALE_Y: f64 = 1.0003;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Rect {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Rect {
    fn shrunk(self, by: f64) -> Self {
        let sx = by.min((self.x1 - self.x0) / 2.0);
        let sy = by.min((self.y1 - self.y0) / 2.0);
        Self {
            x0: self.x0 + sx,
            y0: self.y0 + sy,
            x1: self.x1 - sx,
            y1: self.y1 - sy,
        }
    }

    fn contains(self, o: Self) -> bool {
        o.x0 >= self.x0 - 1e-9
            && o.x1 <= self.x1 + 1e-9
            && o.y0 >= self.y0 - 1e-9
            && o.y1 <= self.y1 + 1e-9
    }
}

/// Whether `r` lies inside the union of `others` (cell-centre test on the compressed grid).
fn covered(r: Rect, others: &[Rect]) -> bool {
    if others.iter().any(|o| o.contains(r)) {
        return true;
    }
    let mut xs = vec![r.x0, r.x1];
    let mut ys = vec![r.y0, r.y1];
    for o in others {
        xs.extend([o.x0, o.x1].into_iter().filter(|v| *v > r.x0 && *v < r.x1));
        ys.extend([o.y0, o.y1].into_iter().filter(|v| *v > r.y0 && *v < r.y1));
    }
    xs.sort_by(f64::total_cmp);
    ys.sort_by(f64::total_cmp);
    for xw in xs.windows(2) {
        for yw in ys.windows(2) {
            if xw[1] - xw[0] < 1e-9 || yw[1] - yw[0] < 1e-9 {
                continue;
            }
            let (cx, cy) = ((xw[0] + xw[1]) / 2.0, (yw[0] + yw[1]) / 2.0);
            if !others
                .iter()
                .any(|o| cx >= o.x0 && cx <= o.x1 && cy >= o.y0 && cy <= o.y1)
            {
                return false;
            }
        }
    }
    true
}

/// Joins rectangles that share two edges and touch along the other two (within a hair), which
/// is how Word cuts one rule into per-line segments.
fn merge(rects: &mut Vec<Rect>) {
    const EPS: f64 = 0.03;
    loop {
        let mut joined = None;
        'outer: for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                let (a, b) = (rects[i], rects[j]);
                let same_x = (a.x0 - b.x0).abs() < EPS && (a.x1 - b.x1).abs() < EPS;
                let same_y = (a.y0 - b.y0).abs() < EPS && (a.y1 - b.y1).abs() < EPS;
                let touch_y = a.y0 <= b.y1 + EPS && b.y0 <= a.y1 + EPS;
                let touch_x = a.x0 <= b.x1 + EPS && b.x0 <= a.x1 + EPS;
                if (same_x && touch_y) || (same_y && touch_x) {
                    joined = Some((
                        i,
                        j,
                        Rect {
                            x0: a.x0.min(b.x0),
                            y0: a.y0.min(b.y0),
                            x1: a.x1.max(b.x1),
                            y1: a.y1.max(b.y1),
                        },
                    ));
                    break 'outer;
                }
            }
        }
        match joined {
            Some((i, j, r)) => {
                rects[i] = r;
                rects.swap_remove(j);
            }
            None => return,
        }
    }
}

struct Scored {
    matched: usize,
    total: usize,
    report: String,
}

fn score(name: &str) -> Option<Scored> {
    let root = root();
    let ref_path = root.join(format!("fixtures/reference/layout/{name}.json"));
    let json: Value = serde_json::from_slice(&fs::read(&ref_path).ok()?).ok()?;
    let rules = json.get("rules")?.as_array()?;
    let bytes = fs::read(root.join(format!("fixtures/docx/{name}.docx"))).ok()?;
    let doc = Document::open(&bytes, &Limits::default()).ok()?;
    let fonts = library(Mode::Bundled);
    let layout = layout_document(&doc, &fonts, LayoutOptions::default());

    let mut pages: BTreeMap<u64, (Vec<Rect>, Vec<Rect>)> = BTreeMap::new();
    for r in rules {
        let f = |k: &str| r[k].as_f64().unwrap_or(0.0);
        let rect = Rect {
            x0: f("x") / SCALE_X,
            y0: f("y") / SCALE_Y,
            x1: (f("x") + f("w")) / SCALE_X,
            y1: (f("y") + f("h")) / SCALE_Y,
        };
        let page = r["page"].as_u64().unwrap_or(1);
        let slot = &mut pages.entry(page).or_default().0;
        if !slot.contains(&rect) {
            slot.push(rect);
        }
    }
    for p in &layout.pages {
        let slot = &mut pages.entry(p.number as u64).or_default().1;
        for r in &p.rules {
            slot.push(Rect {
                x0: r.x,
                y0: r.y,
                x1: r.x + r.width,
                y1: r.y + r.height,
            });
        }
    }
    for (want, got) in pages.values_mut() {
        merge(want);
        merge(got);
    }
    let mut res = Scored {
        matched: 0,
        total: 0,
        report: String::new(),
    };
    for (page, (want, got)) in &pages {
        for w in want {
            res.total += 1;
            if covered(w.shrunk(TOLERANCE), got) {
                res.matched += 1;
            } else {
                let _ = writeln!(
                    res.report,
                    "FAIL ref    p{page} x {:>7.2} y {:>7.2} w {:>6.2} h {:>7.2}",
                    w.x0,
                    w.y0,
                    w.x1 - w.x0,
                    w.y1 - w.y0
                );
            }
        }
        for g in got {
            res.total += 1;
            if covered(g.shrunk(TOLERANCE), want) {
                res.matched += 1;
            } else {
                let _ = writeln!(
                    res.report,
                    "FAIL engine p{page} x {:>7.2} y {:>7.2} w {:>6.2} h {:>7.2}",
                    g.x0,
                    g.y0,
                    g.x1 - g.x0,
                    g.y1 - g.y0
                );
            }
        }
    }
    Some(res)
}

fn floor_path() -> std::path::PathBuf {
    root().join("fixtures/reference/layout/rules-floor.toml")
}

#[test]
fn border_rules_match_word_at_least_as_well_as_the_floor() {
    let detail = std::env::var("PARITY_DETAIL").ok();
    let update = std::env::var_os("UPDATE_PARITY_FLOOR").is_some();
    let mut floors: BTreeMap<String, usize> = BTreeMap::new();
    if let Ok(text) = fs::read_to_string(floor_path()) {
        let table: toml::Table = text.parse().expect("parse rules-floor.toml");
        for (k, v) in &table {
            floors.insert(k.clone(), v["matched"].as_integer().unwrap_or(0) as usize);
        }
    }
    let dir = root().join("fixtures/reference/layout");
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("reference dir")
        .filter_map(Result::ok)
        .filter_map(|e| {
            let p = e.path();
            (p.extension()? == "json").then(|| p.file_stem()?.to_str().map(str::to_owned))?
        })
        .filter(|n| !n.ends_with(".lists"))
        .collect();
    names.sort();
    let mut failures = Vec::new();
    let mut summary =
        String::from("\nBorder rule parity (matched / rectangles, both directions)\n");
    for name in &names {
        if std::env::var("PARITY_ONLY").is_ok_and(|o| o != *name) {
            continue;
        }
        let Some(r) = score(name) else { continue };
        let _ = writeln!(summary, "{name:<24} {:>4}/{:<4}", r.matched, r.total);
        if detail.as_deref().is_some_and(|d| d == name || d == "all") {
            println!("--- {name}\n{}", r.report);
        }
        let floor = floors.get(name).copied();
        match floor {
            None if !update => failures.push(format!(
                "{name}: no entry in rules-floor.toml (matched {}/{})",
                r.matched, r.total
            )),
            Some(f) if r.matched < f => {
                failures.push(format!("{name}: matched {}, floor is {f}", r.matched));
            }
            _ => {}
        }
        let keep = floor.unwrap_or(0).max(r.matched);
        floors.insert(name.to_string(), keep);
        let _ = (r.total,);
    }
    println!("{summary}");
    if update {
        let mut out = String::from(
            "# Ratchet for crates/layout/tests/rules_parity.rs: border rectangles that match Word.\n# Raise with UPDATE_PARITY_FLOOR=1; never lower.\n",
        );
        for (k, v) in &floors {
            let _ = write!(out, "\n[{k}]\nmatched = {v}\n");
        }
        fs::write(floor_path(), out).expect("write rules floor");
    }
    assert!(
        failures.is_empty(),
        "rule parity regressions:\n{}",
        failures.join("\n")
    );
}
