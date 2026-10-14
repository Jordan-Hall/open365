//! DrawingML shapes (`wps:wsp`) for the PDF: preset geometry from the formulas of ECMA-376 Part 1
//! (20.1.9, `presetShapeDefinitions`), colours with the modifiers of 20.1.2.3 and the theme.
//!
//! A preset is built as paths in the shape's own coordinates (points, y down, origin at the
//! top left) from its width, height and adjust values. Word's PDF for each preset (see
//! `fixtures/reference/pdf/probes/p-shapes.pdf`) is the oracle `tests/shapes.rs` compares with.

use std::collections::HashMap;

use open365_docx::Document;

/// One segment of a path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seg {
    Move(f64, f64),
    Line(f64, f64),
    Cubic([f64; 6]),
    Close,
}

/// How a path is painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    /// The shape's fill colour.
    Norm,
    /// 80 % of it.
    Darken,
    /// 60 % of it.
    DarkenLess,
    /// 20 % towards white.
    Lighten,
    /// 40 % towards white.
    LightenLess,
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    pub segs: Vec<Seg>,
    pub fill: Fill,
    pub stroke: bool,
}

impl Path {
    fn new(fill: Fill, stroke: bool) -> Self {
        Self {
            segs: Vec::new(),
            fill,
            stroke,
        }
    }

    fn m(&mut self, x: f64, y: f64) -> &mut Self {
        self.segs.push(Seg::Move(x, y));
        self
    }

    fn l(&mut self, x: f64, y: f64) -> &mut Self {
        self.segs.push(Seg::Line(x, y));
        self
    }

    fn c(&mut self, a: [f64; 6]) -> &mut Self {
        self.segs.push(Seg::Cubic(a));
        self
    }

    fn z(&mut self) -> &mut Self {
        self.segs.push(Seg::Close);
        self
    }

    fn poly(&mut self, pts: &[(f64, f64)]) -> &mut Self {
        for (i, (x, y)) in pts.iter().enumerate() {
            if i == 0 {
                self.m(*x, *y);
            } else {
                self.l(*x, *y);
            }
        }
        self.z()
    }

    /// The current point.
    fn pen(&self) -> (f64, f64) {
        let mut at = (0.0, 0.0);
        let mut start = at;
        for s in &self.segs {
            match *s {
                Seg::Move(x, y) => {
                    at = (x, y);
                    start = at;
                }
                Seg::Line(x, y) => at = (x, y),
                Seg::Cubic(c) => at = (c[4], c[5]),
                Seg::Close => at = start,
            }
        }
        at
    }

    /// `arcTo`: an elliptical arc of radii `wr`, `hr` that starts at the pen, where the ellipse
    /// is at angle `st` (degrees, clockwise on the page), sweeping `sw` degrees. Angles are
    /// those of the direction from the centre (not the parametric ones), as in the
    /// specification. Quarter arcs are one cubic each.
    fn arc(&mut self, wr: f64, hr: f64, st: f64, sw: f64) -> &mut Self {
        let (px, py) = self.pen();
        let t0 = param(wr, hr, st);
        // The parametric end angle follows the visual one.
        let t1 = t0 + {
            let end = param(wr, hr, st + sw);
            let mut d = end - t0;
            // Keep the sweep's sign and size (a quarter turn stays a quarter turn).
            while sw > 0.0 && d < 0.0 {
                d += 360.0;
            }
            while sw < 0.0 && d > 0.0 {
                d -= 360.0;
            }
            while d.abs() > sw.abs() + 180.0 {
                d -= 360.0 * d.signum();
            }
            d
        };
        let cx = px - wr * t0.to_radians().cos();
        let cy = py - hr * t0.to_radians().sin();
        let steps = ((t1 - t0).abs() / 90.0 - 1e-9).ceil().max(1.0) as usize;
        let delta = (t1 - t0) / steps as f64;
        let k = 4.0 / 3.0 * (delta.to_radians() / 4.0).tan();
        for i in 0..steps {
            let a0 = (t0 + delta * i as f64).to_radians();
            let a1 = (t0 + delta * (i + 1) as f64).to_radians();
            let p = |a: f64| (cx + wr * a.cos(), cy + hr * a.sin());
            let (x0, y0) = p(a0);
            let (x3, y3) = p(a1);
            let d0 = (-wr * a0.sin(), hr * a0.cos());
            let d1 = (-wr * a1.sin(), hr * a1.cos());
            self.c([
                x0 + k * d0.0,
                y0 + k * d0.1,
                x3 - k * d1.0,
                y3 - k * d1.1,
                x3,
                y3,
            ]);
        }
        self
    }
}

/// The parametric angle of the point of an ellipse seen at visual angle `a` from its centre.
fn param(wr: f64, hr: f64, a: f64) -> f64 {
    if (wr - hr).abs() < 1e-9 {
        return a;
    }
    // Keep the turn count of `a`.
    let turns = (a / 360.0).floor();
    let r = a - turns * 360.0;
    let t = (wr * r.to_radians().sin())
        .atan2(hr * r.to_radians().cos())
        .to_degrees();
    let t = if t < 0.0 { t + 360.0 } else { t };
    // Snap the quarter turns.
    turns * 360.0 + t
}

/// A preset shape: its paths.
#[derive(Debug, Clone, PartialEq)]
pub struct Geometry {
    pub paths: Vec<Path>,
}

fn pin(lo: f64, v: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

/// Adjust values by name; a preset not given one uses its default.
pub struct Adj<'a>(pub &'a [(String, f64)]);

impl Adj<'_> {
    fn get(&self, name: &str, default: f64) -> f64 {
        self.0
            .iter()
            .find(|(n, _)| n == name)
            .map_or(default, |(_, v)| *v)
    }
}

fn single(p: Path) -> Option<Geometry> {
    Some(Geometry { paths: vec![p] })
}

fn norm() -> Path {
    Path::new(Fill::Norm, true)
}

fn polygon(pts: &[(f64, f64)]) -> Path {
    let mut p = norm();
    p.poly(pts);
    p
}

/// A star with `n` points: the outer vertices on the ellipse of radii `(rx, ry)` and the inner
/// ones on `(irx, iry)` between them. `first` is the page angle (degrees, clockwise from the
/// right) of the first outer vertex.
fn star(cx: f64, cy: f64, rx: f64, ry: f64, irx: f64, iry: f64, n: usize, first: f64) -> Path {
    let mut p = norm();
    let step = 360.0 / n as f64;
    for i in 0..2 * n {
        let a = (first + step / 2.0 * i as f64).to_radians();
        let (ax, ay) = if i % 2 == 0 { (rx, ry) } else { (irx, iry) };
        let (x, y) = (cx + ax * a.cos(), cy + ay * a.sin());
        if i == 0 {
            p.m(x, y);
        } else {
            p.l(x, y);
        }
    }
    p.z();
    p
}

/// The geometry of preset `name` for a `w` by `h` box, or `None` for a preset not known here.
#[allow(clippy::too_many_lines)]
pub fn preset(name: &str, adj: &Adj<'_>, w: f64, h: f64) -> Option<Geometry> {
    let ss = w.min(h);
    let (hc, vc) = (w / 2.0, h / 2.0);
    let (wd2, hd2) = (w / 2.0, h / 2.0);
    let (l, t, r, b) = (0.0, 0.0, w, h);
    match name {
        "rect" | "flowChartProcess" => single(polygon(&[(l, t), (r, t), (r, b), (l, b)])),
        "roundRect" => {
            let a = pin(0.0, adj.get("adj", 16667.0), 50000.0);
            let x1 = ss * a / 100000.0;
            let (x2, y2) = (r - x1, b - x1);
            let mut p = norm();
            p.m(l, x1)
                .arc(x1, x1, 180.0, 90.0)
                .l(x2, t)
                .arc(x1, x1, 270.0, 90.0)
                .l(r, y2)
                .arc(x1, x1, 0.0, 90.0)
                .l(x1, b)
                .arc(x1, x1, 90.0, 90.0)
                .z();
            single(p)
        }
        "ellipse" => {
            let mut p = norm();
            p.m(l, vc)
                .arc(wd2, hd2, 180.0, 90.0)
                .arc(wd2, hd2, 270.0, 90.0)
                .arc(wd2, hd2, 0.0, 90.0)
                .arc(wd2, hd2, 90.0, 90.0)
                .z();
            single(p)
        }
        "triangle" => {
            let x2 = w * adj.get("adj", 50000.0) / 100000.0;
            single(polygon(&[(l, b), (x2, t), (r, b)]))
        }
        "rtTriangle" => single(polygon(&[(l, b), (l, t), (r, b)])),
        "diamond" | "flowChartDecision" => {
            single(polygon(&[(l, vc), (hc, t), (r, vc), (hc, b)]))
        }
        "parallelogram" => {
            let max = 100000.0 * w / ss;
            let a = pin(0.0, adj.get("adj", 25000.0), max);
            let x2 = ss * a / 100000.0;
            single(polygon(&[(l, b), (x2, t), (r, t), (r - x2, b)]))
        }
        "trapezoid" => {
            let max = 50000.0 * w / ss;
            let a = pin(0.0, adj.get("adj", 25000.0), max);
            let x2 = ss * a / 100000.0;
            single(polygon(&[(l, b), (x2, t), (r - x2, t), (r, b)]))
        }
        "pentagon" => {
            let (hf, vf) = (1.05146, 1.10557);
            let (swd2, shd2, svc) = (wd2 * hf, hd2 * vf, vc * vf);
            let (c18, s18) = (18f64.to_radians().cos(), 18f64.to_radians().sin());
            let (c306, s306) = (306f64.to_radians().cos(), 306f64.to_radians().sin());
            let (dx1, dx2, dy1, dy2) = (swd2 * c18, swd2 * c306, shd2 * s18, shd2 * s306);
            let (y1, y2) = (svc - dy1, svc - dy2);
            single(polygon(&[
                (hc - dx1, y1),
                (hc, t),
                (hc + dx1, y1),
                (hc + dx2, y2),
                (hc - dx2, y2),
            ]))
        }
        "hexagon" => {
            let max = 50000.0 * w / ss;
            let a = pin(0.0, adj.get("adj", 25000.0), max);
            let vf = adj.get("vf", 115470.0);
            let shd2 = hd2 * vf / 100000.0;
            let x1 = ss * a / 100000.0;
            let dy1 = shd2 * 60f64.to_radians().sin();
            let (y1, y2) = (vc - dy1, vc + dy1);
            single(polygon(&[
                (l, vc),
                (x1, y1),
                (r - x1, y1),
                (r, vc),
                (r - x1, y2),
                (x1, y2),
            ]))
        }
        "octagon" => {
            let a = pin(0.0, adj.get("adj", 29289.0), 50000.0);
            let x1 = ss * a / 100000.0;
            let (x2, y2) = (r - x1, b - x1);
            single(polygon(&[
                (l, x1),
                (x1, t),
                (x2, t),
                (r, x1),
                (r, y2),
                (x2, b),
                (x1, b),
                (l, y2),
            ]))
        }
        "plus" => {
            let a = pin(0.0, adj.get("adj", 25000.0), 50000.0);
            let x1 = ss * a / 100000.0;
            let (x2, y2) = (r - x1, b - x1);
            single(polygon(&[
                (l, x1),
                (x1, x1),
                (x1, t),
                (x2, t),
                (x2, x1),
                (r, x1),
                (r, y2),
                (x2, y2),
                (x2, b),
                (x1, b),
                (x1, y2),
                (l, y2),
            ]))
        }
        "star4" => {
            let a = pin(0.0, adj.get("adj", 12500.0), 50000.0);
            single(star(hc, vc, wd2, hd2, wd2 * a / 50000.0, hd2 * a / 50000.0, 4, 180.0))
        }
        "star5" => {
            let a = pin(0.0, adj.get("adj", 19098.0), 50000.0);
            let (hf, vf) = (1.05146, 1.10557);
            let (swd2, shd2, svc) = (wd2 * hf, hd2 * vf, vc * vf);
            single(star(
                hc,
                svc,
                swd2,
                shd2,
                swd2 * a / 50000.0,
                shd2 * a / 50000.0,
                5,
                198.0,
            ))
        }
        "star6" => {
            let a = pin(0.0, adj.get("adj", 28868.0), 50000.0);
            let hf = 1.1547;
            let swd2 = wd2 * hf;
            single(star(
                hc,
                vc,
                swd2,
                hd2,
                swd2 * a / 50000.0,
                hd2 * a / 50000.0,
                6,
                210.0,
            ))
        }
        "star8" => {
            let a = pin(0.0, adj.get("adj", 37500.0), 50000.0);
            single(star(hc, vc, wd2, hd2, wd2 * a / 50000.0, hd2 * a / 50000.0, 8, 180.0))
        }
        "rightArrow" | "leftArrow" => {
            let max2 = 100000.0 * w / ss;
            let a1 = pin(0.0, adj.get("adj1", 50000.0), 100000.0);
            let a2 = pin(0.0, adj.get("adj2", 50000.0), max2);
            let dx1 = ss * a2 / 100000.0;
            let dy1 = h * a1 / 200000.0;
            let (y1, y2) = (vc - dy1, vc + dy1);
            if name == "rightArrow" {
                let x1 = r - dx1;
                single(polygon(&[
                    (l, y1),
                    (x1, y1),
                    (x1, t),
                    (r, vc),
                    (x1, b),
                    (x1, y2),
                    (l, y2),
                ]))
            } else {
                let x1 = l + dx1;
                single(polygon(&[
                    (l, vc),
                    (x1, t),
                    (x1, y1),
                    (r, y1),
                    (r, y2),
                    (x1, y2),
                    (x1, b),
                ]))
            }
        }
        "upArrow" | "downArrow" => {
            let max2 = 100000.0 * h / ss;
            let a1 = pin(0.0, adj.get("adj1", 50000.0), 100000.0);
            let a2 = pin(0.0, adj.get("adj2", 50000.0), max2);
            let dy2 = ss * a2 / 100000.0;
            let dx1 = w * a1 / 200000.0;
            let (x1, x2) = (hc - dx1, hc + dx1);
            if name == "upArrow" {
                let y2 = t + dy2;
                single(polygon(&[
                    (l, y2),
                    (hc, t),
                    (r, y2),
                    (x2, y2),
                    (x2, b),
                    (x1, b),
                    (x1, y2),
                ]))
            } else {
                let y2 = b - dy2;
                single(polygon(&[
                    (x1, t),
                    (x2, t),
                    (x2, y2),
                    (r, y2),
                    (hc, b),
                    (l, y2),
                    (x1, y2),
                ]))
            }
        }
        "leftRightArrow" => {
            let max2 = 50000.0 * w / ss;
            let a1 = pin(0.0, adj.get("adj1", 50000.0), 100000.0);
            let a2 = pin(0.0, adj.get("adj2", 50000.0), max2);
            let x1 = ss * a2 / 100000.0;
            let x2 = r - x1;
            let dy1 = h * a1 / 200000.0;
            let (y1, y2) = (vc - dy1, vc + dy1);
            single(polygon(&[
                (l, vc),
                (x1, t),
                (x1, y1),
                (x2, y1),
                (x2, t),
                (r, vc),
                (x2, b),
                (x2, y2),
                (x1, y2),
                (x1, b),
            ]))
        }
        "chevron" => {
            let max = 100000.0 * w / ss;
            let a = pin(0.0, adj.get("adj", 50000.0), max);
            let x1 = ss * a / 100000.0;
            let x2 = r - x1;
            single(polygon(&[(l, t), (x2, t), (r, vc), (x2, b), (l, b), (x1, vc)]))
        }
        "homePlate" => {
            let max = 100000.0 * w / ss;
            let a = pin(0.0, adj.get("adj", 50000.0), max);
            let x1 = r - ss * a / 100000.0;
            single(polygon(&[(l, t), (x1, t), (r, vc), (x1, b), (l, b)]))
        }
        "line" | "straightConnector1" => {
            let mut p = Path::new(Fill::None, true);
            p.m(l, t).l(r, b);
            single(p)
        }
        "flowChartTerminator" => {
            let (rx, ry) = (w * 3475.0 / 21600.0, hd2);
            let mut p = norm();
            p.m(rx, t)
                .l(r - rx, t)
                .arc(rx, ry, 270.0, 90.0)
                .arc(rx, ry, 0.0, 90.0)
                .l(rx, b)
                .arc(rx, ry, 90.0, 90.0)
                .arc(rx, ry, 180.0, 90.0)
                .z();
            single(p)
        }
        "flowChartDocument" => {
            let (sx, sy) = (w / 21600.0, h / 21600.0);
            let mut p = norm();
            p.m(l, t)
                .l(r, t)
                .l(r, 17322.0 * sy)
                .c([
                    10800.0 * sx,
                    17322.0 * sy,
                    10800.0 * sx,
                    23922.0 * sy,
                    l,
                    20172.0 * sy,
                ])
                .z();
            single(p)
        }
        "donut" => {
            let a = pin(0.0, adj.get("adj", 25000.0), 50000.0);
            let dr = ss * a / 100000.0;
            let (iw, ih) = (wd2 - dr, hd2 - dr);
            let mut p = norm();
            p.m(l, vc)
                .arc(wd2, hd2, 180.0, 90.0)
                .arc(wd2, hd2, 270.0, 90.0)
                .arc(wd2, hd2, 0.0, 90.0)
                .arc(wd2, hd2, 90.0, 90.0)
                .z();
            p.m(dr, vc)
                .arc(iw, ih, 180.0, -90.0)
                .arc(iw, ih, 90.0, -90.0)
                .arc(iw, ih, 0.0, -90.0)
                .arc(iw, ih, 270.0, -90.0)
                .z();
            single(p)
        }
        "heart" => {
            let dx1 = w * 49.0 / 48.0;
            let dx2 = w * 10.0 / 48.0;
            let (x1, x2, x3, x4) = (hc - dx1, hc - dx2, hc + dx2, hc + dx1);
            let y1 = t - h / 3.0;
            let mut p = norm();
            p.m(hc, h / 4.0)
                .c([x3, y1, x4, h / 3.0, hc, b])
                .c([x1, h / 3.0, x2, y1, hc, h / 4.0])
                .z();
            single(p)
        }
        "lightningBolt" => {
            let (sx, sy) = (w / 21600.0, h / 21600.0);
            let pts: Vec<(f64, f64)> = [
                (8472.0, 0.0),
                (12860.0, 6080.0),
                (11050.0, 6797.0),
                (16577.0, 12007.0),
                (14767.0, 12877.0),
                (21600.0, 21600.0),
                (10012.0, 14915.0),
                (12222.0, 13987.0),
                (5022.0, 9705.0),
                (7602.0, 8382.0),
                (0.0, 3890.0),
            ]
            .iter()
            .map(|(x, y)| (x * sx, y * sy))
            .collect();
            single(polygon(&pts))
        }
        "wedgeRectCallout" => {
            let dx_pos = w * adj.get("adj1", -20833.0) / 100000.0;
            let dy_pos = h * adj.get("adj2", 62500.0) / 100000.0;
            let (x_pos, y_pos) = (hc + dx_pos, vc + dy_pos);
            let dq = dx_pos * h / w;
            let dz = dy_pos.abs() - dq.abs();
            let (xg1, xg2) = if dx_pos > 0.0 { (7.0, 10.0) } else { (2.0, 5.0) };
            let (x1, x2) = (w * xg1 / 12.0, w * xg2 / 12.0);
            let (yg1, yg2) = if dy_pos > 0.0 { (7.0, 10.0) } else { (2.0, 5.0) };
            let (y1, y2) = (h * yg1 / 12.0, h * yg2 / 12.0);
            let t1 = if dx_pos > 0.0 { l } else { x_pos };
            let xl = if dz > 0.0 { l } else { t1 };
            let t2 = if dy_pos > 0.0 { x1 } else { x_pos };
            let xt = if dz > 0.0 { t2 } else { x1 };
            let t3 = if dx_pos > 0.0 { x_pos } else { r };
            let xr = if dz > 0.0 { r } else { t3 };
            let t4 = if dy_pos > 0.0 { x_pos } else { x1 };
            let xb = if dz > 0.0 { t4 } else { x1 };
            let t5 = if dx_pos > 0.0 { y1 } else { y_pos };
            let yl = if dz > 0.0 { y1 } else { t5 };
            let t6 = if dy_pos > 0.0 { t } else { y_pos };
            let yt = if dz > 0.0 { t6 } else { t };
            let t7 = if dx_pos > 0.0 { y_pos } else { y1 };
            let yr = if dz > 0.0 { y1 } else { t7 };
            let t8 = if dy_pos > 0.0 { y_pos } else { b };
            let yb = if dz > 0.0 { t8 } else { b };
            single(polygon(&[
                (l, t),
                (x1, t),
                (xt, yt),
                (x2, t),
                (r, t),
                (r, y1),
                (xr, yr),
                (r, y2),
                (r, b),
                (x2, b),
                (xb, yb),
                (x1, b),
                (l, b),
                (l, y2),
                (xl, yl),
                (l, y1),
            ]))
        }
        "can" => {
            let max = 50000.0 * h / ss;
            let a = pin(0.0, adj.get("adj", 25000.0), max);
            let y1 = ss * a / 200000.0;
            let y3 = b - y1;
            let mut body = Path::new(Fill::Norm, false);
            body.m(l, y1)
                .arc(wd2, y1, 180.0, -90.0)
                .arc(wd2, y1, 90.0, -90.0)
                .l(r, y3)
                .arc(wd2, y1, 0.0, 90.0)
                .arc(wd2, y1, 90.0, 90.0)
                .z();
            let mut top = Path::new(Fill::Lighten, false);
            top.m(l, y1)
                .arc(wd2, y1, 180.0, 90.0)
                .arc(wd2, y1, 270.0, 90.0)
                .arc(wd2, y1, 0.0, 90.0)
                .arc(wd2, y1, 90.0, 90.0)
                .z();
            let mut line = Path::new(Fill::None, true);
            line.m(r, y1)
                .arc(wd2, y1, 0.0, 90.0)
                .arc(wd2, y1, 90.0, 90.0)
                .arc(wd2, y1, 180.0, 90.0)
                .arc(wd2, y1, 270.0, 90.0)
                .l(l, y3)
                .arc(wd2, y1, 180.0, -180.0)
                .l(r, y1);
            Some(Geometry {
                paths: vec![body, top, line],
            })
        }
        "cube" => {
            let a = pin(0.0, adj.get("adj", 25000.0), 100000.0);
            let y1 = ss * a / 100000.0;
            let (y4, y2, x4) = (b - y1, y1 / 2.0, r - y1);
            let y3 = (y1 + b) / 2.0;
            let x2 = x4 / 2.0 + 0.0;
            let _ = (y2, y3, x2);
            let mut front = Path::new(Fill::Norm, false);
            front.poly(&[(l, y1), (x4, y1), (x4, b), (l, b)]);
            let mut side = Path::new(Fill::Darken, false);
            side.poly(&[(x4, y1), (r, t), (r, y4), (x4, b)]);
            let mut top = Path::new(Fill::Lighten, false);
            top.poly(&[(l, y1), (y1, t), (r, t), (x4, y1)]);
            let mut line = Path::new(Fill::None, true);
            line.poly(&[(l, y1), (y1, t), (r, t), (r, y4), (x4, b), (l, b)]);
            line.m(l, y1).l(x4, y1).l(r, t);
            line.m(x4, y1).l(x4, b);
            Some(Geometry {
                paths: vec![front, side, top, line],
            })
        }
        "blockArc" => {
            let st = adj.get("adj1", 10800000.0) / 60000.0;
            let en = adj.get("adj2", 0.0) / 60000.0;
            let a3 = pin(0.0, adj.get("adj3", 25000.0), 50000.0);
            let dr = ss * a3 / 100000.0;
            let (iw, ih) = (wd2 - dr, hd2 - dr);
            let mut sw = en - st;
            if sw < 0.0 {
                sw += 360.0;
            }
            let pt = |rx: f64, ry: f64, a: f64| {
                let t = param(rx, ry, a).to_radians();
                (hc + rx * t.cos(), vc + ry * t.sin())
            };
            let (sx, sy) = pt(wd2, hd2, st);
            let (ex, ey) = pt(iw, ih, en);
            let mut p = norm();
            p.m(sx, sy)
                .arc(wd2, hd2, st, sw)
                .l(ex, ey)
                .arc(iw, ih, en, -sw)
                .z();
            single(p)
        }
        "arc" => {
            let st = adj.get("adj1", 16200000.0) / 60000.0;
            let en = adj.get("adj2", 0.0) / 60000.0;
            let mut sw = en - st;
            if sw < 0.0 {
                sw += 360.0;
            }
            let pt = |a: f64| {
                let t = param(wd2, hd2, a).to_radians();
                (hc + wd2 * t.cos(), vc + hd2 * t.sin())
            };
            let (sx, sy) = pt(st);
            let mut fill = Path::new(Fill::Norm, false);
            fill.m(sx, sy).arc(wd2, hd2, st, sw).l(hc, vc).z();
            let mut line = Path::new(Fill::None, true);
            line.m(sx, sy).arc(wd2, hd2, st, sw);
            Some(Geometry {
                paths: vec![fill, line],
            })
        }
        _ => None,
    }
}

/// Colours.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Paint {
    pub rgb: [u8; 3],
    /// 0..1
    pub alpha: f64,
}

/// What the theme gives a shape.
#[derive(Debug, Clone)]
pub struct Theme {
    colours: HashMap<&'static str, [u8; 3]>,
    /// Widths in points of the theme's line styles, 1-based by `lnRef/@idx`.
    pub line_widths: Vec<f64>,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            colours: HashMap::new(),
            line_widths: vec![0.5, 1.0, 1.5],
        }
    }
}

const SLOTS: [&str; 12] = [
    "dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6",
    "hlink", "folHlink",
];

pub fn hex(v: &str) -> Option<[u8; 3]> {
    if v.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(v, 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

impl Theme {
    pub fn from_doc(doc: &Document) -> Self {
        let mut t = Self::default();
        if let Some(cols) = doc.theme_colours() {
            for (slot, c) in SLOTS.iter().zip(cols) {
                if let Some(rgb) = hex(&c) {
                    t.colours.insert(slot, rgb);
                }
            }
        }
        if let Some(xml) = theme_xml(doc)
            && let Some(start) = xml.find("<a:lnStyleLst>")
        {
            let end = xml[start..].find("</a:lnStyleLst>").map_or(xml.len(), |e| start + e);
            let mut widths = Vec::new();
            let mut rest = &xml[start..end];
            while let Some(i) = rest.find("<a:ln ") {
                rest = &rest[i + 6..];
                let tag = &rest[..rest.find('>').unwrap_or(rest.len())];
                if let Some(w) = tag
                    .split_whitespace()
                    .find_map(|a| a.strip_prefix("w=\""))
                    .and_then(|v| v.trim_end_matches('"').parse::<f64>().ok())
                {
                    widths.push(w / 12700.0);
                }
            }
            if !widths.is_empty() {
                t.line_widths = widths;
            }
        }
        t
    }

    /// A scheme colour name (`accent1`, `tx1`, ...).
    pub fn scheme(&self, name: &str) -> Option<[u8; 3]> {
        let slot = match name {
            "tx1" => "dk1",
            "bg1" => "lt1",
            "tx2" => "dk2",
            "bg2" => "lt2",
            other => other,
        };
        SLOTS
            .iter()
            .find(|s| **s == slot)
            .and_then(|s| self.colours.get(s).copied())
    }
}

fn theme_xml(doc: &Document) -> Option<String> {
    use open365_opc::{Relationships, TargetMode, rels_part_name, resolve_target};
    let main = doc.main_part();
    let rels = Relationships::parse(doc.package().part(rels_part_name(Some(main)).as_str())?.data())
        .ok()?;
    let rel = rels
        .iter()
        .find(|r| r.target_mode == TargetMode::Internal && r.rel_type.ends_with("/theme"))?;
    let name = resolve_target(Some(main), &rel.target).ok()?;
    let data = doc.package().part(name.as_str())?.data();
    String::from_utf8(data.to_vec()).ok()
}

fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f64) -> f64 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn to_hsl([r, g, b]: [f64; 3]) -> (f64, f64, f64) {
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let l = (mx + mn) / 2.0;
    if (mx - mn).abs() < 1e-12 {
        return (0.0, 0.0, l);
    }
    let d = mx - mn;
    let s = if l > 0.5 { d / (2.0 - mx - mn) } else { d / (mx + mn) };
    let h = if (mx - r).abs() < 1e-12 {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if (mx - g).abs() < 1e-12 {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> [f64; 3] {
    if s.abs() < 1e-12 {
        return [l, l, l];
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0)]
}

/// Applies colour modifier `name` with `v` (a fraction: 60000 is 0.6) to `c` (0..1 RGB).
fn modify(c: [f64; 3], alpha: &mut f64, name: &str, v: f64) -> [f64; 3] {
    let hsl = |c: [f64; 3], f: &dyn Fn(f64, f64, f64) -> (f64, f64, f64)| {
        let (h, s, l) = to_hsl(c);
        let (h, s, l) = f(h, s, l);
        from_hsl(h.rem_euclid(1.0), s.clamp(0.0, 1.0), l.clamp(0.0, 1.0))
    };
    match name {
        "lumMod" => hsl(c, &|h, s, l| (h, s, l * v)),
        "lumOff" => hsl(c, &|h, s, l| (h, s, l + v)),
        "lum" => hsl(c, &|h, s, _| (h, s, v)),
        "satMod" => hsl(c, &|h, s, l| (h, s * v, l)),
        "satOff" => hsl(c, &|h, s, l| (h, s + v, l)),
        "sat" => hsl(c, &|h, _, l| (h, v, l)),
        "hueMod" => hsl(c, &|h, s, l| (h * v, s, l)),
        "tint" => c.map(|x| linear_to_srgb(srgb_to_linear(x) * v + (1.0 - v))),
        "shade" => c.map(|x| linear_to_srgb(srgb_to_linear(x) * v)),
        "alpha" => {
            *alpha = v;
            c
        }
        "alphaMod" => {
            *alpha *= v;
            c
        }
        "alphaOff" => {
            *alpha += v;
            c
        }
        "comp" => hsl(c, &|h, s, l| (h + 0.5, s, l)),
        "inv" => c.map(|x| 1.0 - x),
        "gray" => {
            let y = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
            [y, y, y]
        }
        _ => c,
    }
}

fn preset_colour(name: &str) -> Option<[u8; 3]> {
    Some(match name {
        "black" => [0, 0, 0],
        "white" => [255, 255, 255],
        "red" => [255, 0, 0],
        "green" => [0, 128, 0],
        "blue" => [0, 0, 255],
        "yellow" => [255, 255, 0],
        "cyan" | "aqua" => [0, 255, 255],
        "magenta" | "fuchsia" => [255, 0, 255],
        "gray" | "grey" => [128, 128, 128],
        "orange" => [255, 165, 0],
        "silver" => [192, 192, 192],
        "lime" => [0, 255, 0],
        "navy" => [0, 0, 128],
        "maroon" => [128, 0, 0],
        "purple" => [128, 0, 128],
        "teal" => [0, 128, 128],
        "olive" => [128, 128, 0],
        _ => return None,
    })
}

/// A colour element (`a:srgbClr`, `a:schemeClr`, `a:sysClr`, `a:prstClr`) with its modifiers.
pub fn paint(
    name: &str,
    attrs: &dyn Fn(&str) -> Option<String>,
    mods: &[(String, f64)],
    theme: &Theme,
) -> Option<Paint> {
    let rgb = match name {
        "srgbClr" => hex(&attrs("val")?)?,
        "schemeClr" => theme.scheme(&attrs("val")?)?,
        "sysClr" => match attrs("lastClr") {
            Some(v) => hex(&v)?,
            None => match attrs("val")?.as_str() {
                "windowText" => [0, 0, 0],
                "window" => [255, 255, 255],
                _ => return None,
            },
        },
        "prstClr" => preset_colour(&attrs("val")?)?,
        _ => return None,
    };
    let mut c = rgb.map(|v| f64::from(v) / 255.0);
    let mut alpha = 1.0;
    for (m, v) in mods {
        c = modify(c, &mut alpha, m, *v);
    }
    Some(Paint {
        rgb: c.map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8),
        alpha: alpha.clamp(0.0, 1.0),
    })
}

/// The fill colour of a path with fill mode `fill` over base colour `c`.
pub fn shade(c: [u8; 3], fill: Fill) -> [u8; 3] {
    let f = |v: u8| -> u8 {
        let v = f64::from(v);
        let out = match fill {
            Fill::Norm | Fill::None => v,
            Fill::Darken => (v * 0.8).round(),
            Fill::DarkenLess => (v * 0.6).round(),
            Fill::Lighten => (v + (255.0 - v) * 0.2 - 1e-4).floor(),
            Fill::LightenLess => (v + (255.0 - v) * 0.4 - 1e-4).floor(),
        };
        out.clamp(0.0, 255.0) as u8
    };
    c.map(f)
}

/// Dash pattern of `a:prstDash` in multiples of the line width.
pub fn dash(name: &str) -> Option<&'static [f64]> {
    Some(match name {
        "dot" => &[1.0, 3.0],
        "dash" => &[4.0, 3.0],
        "lgDash" => &[8.0, 3.0],
        "dashDot" => &[4.0, 3.0, 1.0, 3.0],
        "lgDashDot" => &[8.0, 3.0, 1.0, 3.0],
        "lgDashDotDot" => &[8.0, 3.0, 1.0, 3.0, 1.0, 3.0],
        "sysDash" => &[3.0, 1.0],
        "sysDot" => &[1.0, 1.0],
        "sysDashDot" => &[3.0, 1.0, 1.0, 1.0],
        "sysDashDotDot" => &[3.0, 1.0, 1.0, 1.0, 1.0, 1.0],
        _ => return None,
    })
}
