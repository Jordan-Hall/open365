//! Comparison helpers for batches b05 and b06 (callouts, ribbons, scrolls): fills and page culling.
//!
//! The shared check (`tests/preset_probes.rs`) compares stroked outlines only. These two batches
//! need two more things, so they have their own helper and test (`tests/preset_probes_b05_b06.rs`):
//!
//! - Fill-only paths (`stroke="0"`: the shaded folds of ribbons and scrolls, the box of a
//!   borderless callout) are visible in the `*.fill` probe batches, where Word paints a grey fill
//!   (0.753). Word emits one fill operation per preset path; the grey level of each operation
//!   gives the shade of the path (0.604 / 0.753 = 0.80 for the `darkenLess` paths).
//! - Word drops an operation whose outline lies wholly outside the page (the line callouts and the
//!   cloud callout put points far outside the box at adjust 400000 and -50000) and draws nothing
//!   for a zero-length stroke at some positions. Both sides are filtered the same way before they
//!   are compared: subpaths wholly outside the page and zero-length subpaths are ignored.

use crate::common::geom::{Cmd, deviation, flatten};
use crate::common::pdf::Painted;
use open365_drawing::geom::{PathFill, Seg, ShapeDef, evaluate};

pub const EMU_PER_PT: f64 = 12_700.0;
/// Page size in points (US Letter, as the probe documents use) and the box origin on it.
const PAGE: (f64, f64) = (612.0, 792.0);
const ORIGIN: f64 = 72.0;

pub type Poly = Vec<(f64, f64)>;

/// One painted operation of a page: its subpaths in points relative to the shape box (y down).
#[derive(Debug, Clone)]
pub struct Op {
    pub subs: Vec<Poly>,
    /// Grey level for a fill operation.
    pub grey: Option<f64>,
}

fn to_box_pt(c: &Cmd) -> Cmd {
    let fx = |x: f64| x - ORIGIN;
    let fy = |y: f64| PAGE.1 - y - ORIGIN;
    match *c {
        Cmd::M(x, y) => Cmd::M(fx(x), fy(y)),
        Cmd::L(x, y) => Cmd::L(fx(x), fy(y)),
        Cmd::C(a, b, c2, d, e, f) => Cmd::C(fx(a), fy(b), fx(c2), fy(d), fx(e), fy(f)),
        Cmd::Z => Cmd::Z,
    }
}

fn is_fill_op(p: &str) -> bool {
    matches!(p, "f" | "F" | "f*" | "B" | "B*" | "b" | "b*")
}

/// Grey levels set before each fill operation of a page, in order (the content streams set the
/// colour with `<n> g` immediately before the paint operator).
fn fill_greys(content: &[u8]) -> Vec<f64> {
    let text = String::from_utf8_lossy(content);
    let mut out = Vec::new();
    let mut grey = 0.0;
    let mut prev: Option<&str> = None;
    for tok in text.split_whitespace() {
        match tok {
            "g" => {
                if let Some(v) = prev.and_then(|p| p.parse::<f64>().ok()) {
                    grey = v;
                }
            }
            "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" => out.push(grey),
            _ => {}
        }
        prev = Some(tok);
    }
    out
}

fn closed(mut s: Vec<Cmd>) -> Vec<Cmd> {
    if !matches!(s.last(), Some(Cmd::Z)) {
        s.push(Cmd::Z);
    }
    s
}

fn polys(subs: &[Vec<Cmd>]) -> Vec<Poly> {
    flatten(subs)
}

fn bbox(p: &Poly) -> (f64, f64, f64, f64) {
    let mut b = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &(x, y) in p {
        b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
    }
    b
}

/// Keeps the subpaths Word would paint: not wholly off the page, not a single point.
fn visible(p: &Poly) -> bool {
    let (x0, y0, x1, y1) = bbox(p);
    if (x1 - x0).abs() < 1e-6 && (y1 - y0).abs() < 1e-6 {
        return false;
    }
    x1 >= -ORIGIN && x0 <= PAGE.0 - ORIGIN && y1 >= -ORIGIN && y0 <= PAGE.1 - ORIGIN
}

fn keep_visible(subs: Vec<Poly>) -> Vec<Poly> {
    subs.into_iter().filter(visible).collect()
}

/// A fill with no area (all points on one line) paints nothing and Word emits no operation.
fn has_area(p: &Poly) -> bool {
    let (x0, y0, x1, y1) = bbox(p);
    (x1 - x0).abs() > 1e-6 && (y1 - y0).abs() > 1e-6
}

fn keep_filled(subs: Vec<Poly>) -> Vec<Poly> {
    subs.into_iter()
        .filter(|p| visible(p) && has_area(p))
        .collect()
}

/// Word's painted operations for one page: (stroke operations, fill operations).
pub fn word_ops(painted: &[Painted], content: &[u8]) -> (Vec<Op>, Vec<Op>) {
    let greys = fill_greys(content);
    let (mut strokes, mut fills) = (Vec::new(), Vec::new());
    let mut gi = 0;
    for p in painted {
        let subs: Vec<Vec<Cmd>> = p
            .subpaths
            .iter()
            .map(|s| s.iter().map(to_box_pt).collect())
            .collect();
        if is_fill_op(&p.paint) {
            let grey = greys.get(gi).copied();
            gi += 1;
            let sub_closed: Vec<Vec<Cmd>> = subs.iter().cloned().map(closed).collect();
            let v = keep_filled(polys(&sub_closed));
            if !v.is_empty() {
                fills.push(Op { subs: v, grey });
            }
        }
        if p.stroked() {
            let v = keep_visible(polys(&subs));
            if !v.is_empty() {
                strokes.push(Op {
                    subs: v,
                    grey: None,
                });
            }
        }
    }
    (strokes, fills)
}

/// Our evaluated paths split into subpaths (EMU converted to points): returns, for each path,
/// (fill mode, stroke flag, subpaths as commands).
fn our_paths(
    def: &ShapeDef,
    adj: &[(String, String)],
    w: f64,
    h: f64,
) -> Vec<(PathFill, bool, Vec<Vec<Cmd>>)> {
    let ev = evaluate(def, adj, w, h);
    let k = 1.0 / EMU_PER_PT;
    let mut out = Vec::new();
    for p in &ev.paths {
        let mut subs: Vec<Vec<Cmd>> = Vec::new();
        let mut cur: Vec<Cmd> = Vec::new();
        let mut last = (0.0, 0.0);
        for s in &p.segs {
            match *s {
                Seg::Move(q) => {
                    if !cur.is_empty() {
                        subs.push(std::mem::take(&mut cur));
                    }
                    cur.push(Cmd::M(q.x * k, q.y * k));
                    last = (q.x * k, q.y * k);
                }
                Seg::Line(q) => {
                    cur.push(Cmd::L(q.x * k, q.y * k));
                    last = (q.x * k, q.y * k);
                }
                Seg::Quad(c, q) => {
                    let (cx, cy, qx, qy) = (c.x * k, c.y * k, q.x * k, q.y * k);
                    cur.push(Cmd::C(
                        last.0 + 2.0 / 3.0 * (cx - last.0),
                        last.1 + 2.0 / 3.0 * (cy - last.1),
                        qx + 2.0 / 3.0 * (cx - qx),
                        qy + 2.0 / 3.0 * (cy - qy),
                        qx,
                        qy,
                    ));
                    last = (qx, qy);
                }
                Seg::Cubic(a, b, q) => {
                    cur.push(Cmd::C(a.x * k, a.y * k, b.x * k, b.y * k, q.x * k, q.y * k));
                    last = (q.x * k, q.y * k);
                }
                Seg::Close => cur.push(Cmd::Z),
            }
        }
        if !cur.is_empty() {
            subs.push(cur);
        }
        out.push((p.fill, p.stroke, subs));
    }
    out
}

/// Our (stroke operations, fill operations) for a probe, filtered like Word's.
pub fn our_ops(def: &ShapeDef, adj: &[(String, String)], w: f64, h: f64) -> (Vec<Op>, Vec<Op>) {
    let (mut strokes, mut fills) = (Vec::new(), Vec::new());
    for (fill, stroke, subs) in our_paths(def, adj, w, h) {
        if fill != PathFill::None {
            let c: Vec<Vec<Cmd>> = subs.iter().cloned().map(closed).collect();
            let v = keep_filled(polys(&c));
            if !v.is_empty() {
                let grey = match fill {
                    PathFill::Norm => 1.0,
                    PathFill::DarkenLess => 0.80,
                    PathFill::Darken => 0.60,
                    PathFill::Lighten => 1.20,
                    PathFill::LightenLess => 1.10,
                    PathFill::None => 1.0,
                };
                fills.push(Op {
                    subs: v,
                    grey: Some(grey),
                });
            }
        }
        if stroke {
            let v = keep_visible(polys(&subs));
            if !v.is_empty() {
                strokes.push(Op {
                    subs: v,
                    grey: None,
                });
            }
        }
    }
    (strokes, fills)
}

fn merged(ops: &[Op]) -> Vec<Poly> {
    ops.iter().flat_map(|o| o.subs.iter().cloned()).collect()
}

/// Stroke deviation in points (all stroke operations merged; infinity when one side is empty).
pub fn stroke_deviation(ours: &[Op], theirs: &[Op]) -> f64 {
    deviation(&merged(ours), &merged(theirs))
}

/// Fill deviation: operations paired in order, the worst pair counts. Also checks the shade: the
/// ratio of each fill operation's grey to the first one's against what the path's fill mode
/// should give (within 0.02). Returns (deviation, shade problem).
pub fn fill_deviation(ours: &[Op], theirs: &[Op]) -> (f64, Option<String>) {
    if ours.len() != theirs.len() {
        return (
            f64::INFINITY,
            Some(format!(
                "{} fill operations (Word {})",
                ours.len(),
                theirs.len()
            )),
        );
    }
    let mut worst: f64 = 0.0;
    let mut shade = None;
    let base = theirs.first().and_then(|o| o.grey).unwrap_or(1.0);
    for (a, b) in ours.iter().zip(theirs) {
        worst = worst.max(deviation(&a.subs, &b.subs));
        if let (Some(ga), Some(gb)) = (a.grey, b.grey) {
            let ratio = gb / base;
            if (ratio - ga).abs() > 0.02 {
                shade = Some(format!("shade {ratio:.3} (ours {ga:.3})"));
            }
        }
    }
    (worst, shade)
}
