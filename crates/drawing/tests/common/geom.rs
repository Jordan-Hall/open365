//! Path commands, flattening and the deviation between two sets of paths.
//!
//! The deviation is the symmetric Hausdorff distance between the flattened outlines: every
//! point of one outline is within it of the other outline, and the other way round. It does
//! not depend on where a path starts, how a curve is split into cubics, or the order of
//! subpaths, which is what a comparison with Word's output needs. It does not compare
//! direction or the number of overlapping copies of a line.

use std::collections::HashMap;

/// A path command in absolute coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cmd {
    M(f64, f64),
    L(f64, f64),
    C(f64, f64, f64, f64, f64, f64),
    Z,
}

pub type Pt = (f64, f64);

const CURVE_STEPS: usize = 64;

/// Flattens subpaths into polylines. A closed subpath ends on its first point.
pub fn flatten(subs: &[Vec<Cmd>]) -> Vec<Vec<Pt>> {
    let mut out = Vec::new();
    for sub in subs {
        let mut poly: Vec<Pt> = Vec::new();
        let mut start = (0.0, 0.0);
        for c in sub {
            match *c {
                Cmd::M(x, y) => {
                    if poly.len() > 1 {
                        out.push(std::mem::take(&mut poly));
                    }
                    poly.clear();
                    poly.push((x, y));
                    start = (x, y);
                }
                Cmd::L(x, y) => poly.push((x, y)),
                Cmd::C(x1, y1, x2, y2, x, y) => {
                    let p0 = poly.last().copied().unwrap_or(start);
                    for i in 1..=CURVE_STEPS {
                        let t = i as f64 / CURVE_STEPS as f64;
                        let u = 1.0 - t;
                        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                        poly.push((
                            a * p0.0 + b * x1 + c * x2 + d * x,
                            a * p0.1 + b * y1 + c * y2 + d * y,
                        ));
                    }
                }
                Cmd::Z => poly.push(start),
            }
        }
        if poly.len() > 1 {
            out.push(poly);
        }
    }
    out
}

type Seg = (Pt, Pt);

fn segments(polys: &[Vec<Pt>]) -> Vec<Seg> {
    let mut out = Vec::new();
    for p in polys {
        if p.len() == 1 {
            out.push((p[0], p[0]));
        }
        for w in p.windows(2) {
            out.push((w[0], w[1]));
        }
    }
    // An outline of only zero-length segments is a point, which Word does not draw: it is
    // the same as no path.
    if out.iter().all(|(a, b)| a == b) {
        out.clear();
    }
    out
}

fn dist_to_segment(p: Pt, s: &Seg) -> f64 {
    let (a, b) = *s;
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    };
    (p.0 - (a.0 + t * dx)).hypot(p.1 - (a.1 + t * dy))
}

const CELL: f64 = 8.0;
/// Distances beyond this are reported as infinite.
pub const FAR: f64 = CELL;

struct Grid {
    segs: Vec<Seg>,
    cells: HashMap<(i64, i64), Vec<usize>>,
}

fn cell_of(v: f64) -> i64 {
    (v / CELL).floor() as i64
}

impl Grid {
    fn new(segs: Vec<Seg>) -> Self {
        let mut cells: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (i, (a, b)) in segs.iter().enumerate() {
            for cx in cell_of(a.0.min(b.0))..=cell_of(a.0.max(b.0)) {
                for cy in cell_of(a.1.min(b.1))..=cell_of(a.1.max(b.1)) {
                    cells.entry((cx, cy)).or_default().push(i);
                }
            }
        }
        Self { segs, cells }
    }

    /// Nearest distance from `p`, or infinity when nothing is within one cell.
    fn nearest(&self, p: Pt) -> f64 {
        let (cx, cy) = (cell_of(p.0), cell_of(p.1));
        let mut best = f64::INFINITY;
        for x in cx - 1..=cx + 1 {
            for y in cy - 1..=cy + 1 {
                if let Some(v) = self.cells.get(&(x, y)) {
                    for &i in v {
                        best = best.min(dist_to_segment(p, &self.segs[i]));
                    }
                }
            }
        }
        if best <= FAR { best } else { f64::INFINITY }
    }
}

/// The largest distance from a point of `a` to the outline `b`.
fn directed(a: &[Seg], b: &Grid) -> f64 {
    const STEP: f64 = 0.1;
    let mut worst: f64 = 0.0;
    for &(p, q) in a {
        let len = (q.0 - p.0).hypot(q.1 - p.1);
        let n = (len / STEP).ceil().max(1.0) as usize;
        for i in 0..=n {
            let t = i as f64 / n as f64;
            let d = b.nearest((p.0 + t * (q.0 - p.0), p.1 + t * (q.1 - p.1)));
            worst = worst.max(d);
            if worst.is_infinite() {
                return worst;
            }
        }
    }
    worst
}

/// Symmetric deviation between two sets of polylines, in the unit of the coordinates.
/// Both empty gives 0; one empty gives infinity.
pub fn deviation(a: &[Vec<Pt>], b: &[Vec<Pt>]) -> f64 {
    let (sa, sb) = (segments(a), segments(b));
    match (sa.is_empty(), sb.is_empty()) {
        (true, true) => return 0.0,
        (true, false) | (false, true) => return f64::INFINITY,
        _ => {}
    }
    let (ga, gb) = (Grid::new(sa.clone()), Grid::new(sb.clone()));
    directed(&sa, &gb).max(directed(&sb, &ga))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deviation_ignores_start_point_and_split() {
        let sq1 = vec![vec![
            Cmd::M(0.0, 0.0),
            Cmd::L(10.0, 0.0),
            Cmd::L(10.0, 10.0),
            Cmd::L(0.0, 10.0),
            Cmd::Z,
        ]];
        let sq2 = vec![vec![
            Cmd::M(10.0, 10.0),
            Cmd::L(0.0, 10.0),
            Cmd::L(0.0, 5.0),
            Cmd::L(0.0, 0.0),
            Cmd::L(10.0, 0.0),
            Cmd::Z,
        ]];
        assert!(deviation(&flatten(&sq1), &flatten(&sq2)) < 1e-9);
        let moved = vec![vec![
            Cmd::M(0.0, 0.3),
            Cmd::L(10.0, 0.3),
            Cmd::L(10.0, 10.0),
            Cmd::L(0.0, 10.0),
            Cmd::Z,
        ]];
        let d = deviation(&flatten(&sq1), &flatten(&moved));
        assert!((d - 0.3).abs() < 1e-6, "{d}");
        assert!(deviation(&flatten(&sq1), &[]).is_infinite());
    }
}
