//! Path segments and the DrawingML `arcTo` conversion.

use super::formula::ANG;

/// A point.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct P {
    pub x: f64,
    pub y: f64,
}

impl P {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// One path segment. Arcs are converted to cubic Beziers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seg {
    Move(P),
    Line(P),
    Quad(P, P),
    Cubic(P, P, P),
    Close,
}

impl Seg {
    /// End point, when the segment has one.
    pub fn end(&self) -> Option<P> {
        match *self {
            Self::Move(p) | Self::Line(p) | Self::Quad(_, p) | Self::Cubic(_, _, p) => Some(p),
            Self::Close => None,
        }
    }

    /// Applies `f` to every point.
    pub fn map(&self, f: &impl Fn(P) -> P) -> Self {
        match *self {
            Self::Move(p) => Self::Move(f(p)),
            Self::Line(p) => Self::Line(f(p)),
            Self::Quad(a, b) => Self::Quad(f(a), f(b)),
            Self::Cubic(a, b, c) => Self::Cubic(f(a), f(b), f(c)),
            Self::Close => Self::Close,
        }
    }
}

/// Visual angle to the ellipse parameter, continuous in `theta` (radians).
fn param_angle(wr: f64, hr: f64, theta: f64) -> f64 {
    // A flat ellipse (one radius zero) has no radius-line angle to convert; Word then uses
    // the visual angle as the parameter (measured on `blockArc` with the inner ellipse flat).
    if wr == 0.0 || hr == 0.0 {
        return theta;
    }
    let base = (wr * theta.sin()).atan2(hr * theta.cos());
    let turns = ((theta - base) / std::f64::consts::TAU).round();
    base + turns * std::f64::consts::TAU
}

/// Converts `arcTo` from the current point `cur` into cubic segments.
///
/// `st` and `sw` are the start and swing angles in 60000ths of a degree; they are visual
/// angles of the ellipse (the angle of the radius line, not the parameter). The current
/// point is the point of the ellipse at `st`; the centre follows from that. Returns the
/// segments and the new current point. A zero swing, a negative radius or two zero radii yield
/// nothing. With one zero radius the arc is the flat curve Word draws, a line traced back and
/// forth along the other axis.
pub fn arc_to(cur: P, wr: f64, hr: f64, st: f64, sw: f64) -> (Vec<Seg>, P) {
    if wr < 0.0 || hr < 0.0 || (wr == 0.0 && hr == 0.0) || sw == 0.0 {
        return (Vec::new(), cur);
    }
    let th1 = (st / ANG).to_radians();
    let th2 = ((st + sw) / ANG).to_radians();
    let t1 = param_angle(wr, hr, th1);
    let mut t2 = param_angle(wr, hr, th2);
    // Keep the parametric sweep on the side and size of the requested swing.
    let tau = std::f64::consts::TAU;
    if sw > 0.0 {
        while t2 < t1 {
            t2 += tau;
        }
    } else {
        while t2 > t1 {
            t2 -= tau;
        }
    }
    let c = P::new(cur.x - wr * t1.cos(), cur.y - hr * t1.sin());
    let dt = t2 - t1;
    let n = (dt.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0);
    let step = dt / n;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let at = |t: f64| P::new(c.x + wr * t.cos(), c.y + hr * t.sin());
    let d = |t: f64| P::new(-wr * t.sin(), hr * t.cos());
    let mut out = Vec::new();
    let mut a = t1;
    let mut last = cur;
    for i in 0..n as usize {
        let b = if i + 1 == n as usize { t2 } else { a + step };
        let (pa, pb) = (at(a), at(b));
        let (da, db) = (d(a), d(b));
        let end = if i + 1 == n as usize { at(t2) } else { pb };
        out.push(Seg::Cubic(
            P::new(pa.x + k * da.x, pa.y + k * da.y),
            P::new(pb.x - k * db.x, pb.y - k * db.y),
            end,
        ));
        last = end;
        a = b;
    }
    (out, last)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: P, b: P) -> bool {
        (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6
    }

    #[test]
    fn quarter_circle_from_east_to_south() {
        // Start at (10, 0) on a circle of radius 10 centred on the origin, swing +90 degrees:
        // y grows downward, so the arc runs to (0, 10).
        let (segs, end) = arc_to(P::new(10.0, 0.0), 10.0, 10.0, 0.0, 5_400_000.0);
        assert_eq!(segs.len(), 1);
        assert!(close(end, P::new(0.0, 10.0)));
        // the midpoint of the Bezier lies on the circle
        let Seg::Cubic(a, b, c) = segs[0] else {
            panic!("cubic expected")
        };
        let m = P::new(
            (10.0 + 3.0 * a.x + 3.0 * b.x + c.x) / 8.0,
            (0.0 + 3.0 * a.y + 3.0 * b.y + c.y) / 8.0,
        );
        assert!((m.x.hypot(m.y) - 10.0).abs() < 1e-3);
    }

    #[test]
    fn full_circle_returns_to_start_and_stays_on_ellipse() {
        let (segs, end) = arc_to(P::new(30.0, 5.0), 30.0, 5.0, 0.0, 21_600_000.0);
        assert_eq!(segs.len(), 4);
        assert!(close(end, P::new(30.0, 5.0)));
        // centre is (0, 5)
        for s in &segs {
            let e = s.end().unwrap();
            let v = (e.x / 30.0).powi(2) + ((e.y - 5.0) / 5.0).powi(2);
            assert!((v - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn negative_swing_runs_counter_clockwise() {
        let (_, end) = arc_to(P::new(10.0, 0.0), 10.0, 10.0, 0.0, -5_400_000.0);
        assert!(close(end, P::new(0.0, -10.0)));
    }

    #[test]
    fn ellipse_uses_visual_angle() {
        // 45 degree visual angle on a 20x10 ellipse centred at origin is the point where
        // the radius line makes 45 degrees: x == y.
        let (_, end) = arc_to(P::new(20.0, 0.0), 20.0, 10.0, 0.0, 2_700_000.0);
        assert!((end.x - end.y * 1.0).abs() < 1e-6, "{end:?}");
        let (segs, _) = arc_to(P::new(20.0, 0.0), 20.0, 10.0, 0.0, 2_700_000.0);
        assert_eq!(segs.len(), 1);
    }

    #[test]
    fn degenerate_arcs_are_empty() {
        assert!(arc_to(P::new(1.0, 1.0), 0.0, 0.0, 0.0, 100.0).0.is_empty());
        assert!(arc_to(P::new(1.0, 1.0), -1.0, 5.0, 0.0, 100.0).0.is_empty());
        assert!(arc_to(P::new(1.0, 1.0), 5.0, 5.0, 0.0, 0.0).0.is_empty());
    }

    #[test]
    fn flat_ellipse_full_sweep_is_a_line_there_and_back() {
        // Radius 10 wide, 0 high, start at 90 degrees (visual angle used as the parameter, so
        // the centre is the current point), full negative turn: a flat curve on y = 3.
        let (segs, end) = arc_to(P::new(5.0, 3.0), 10.0, 0.0, 5_400_000.0, -21_600_000.0);
        assert_eq!(segs.len(), 4);
        assert!(close(end, P::new(5.0, 3.0)));
        for s in &segs {
            let Seg::Cubic(a, b, c) = *s else { panic!() };
            assert!([a, b, c].iter().all(|p| (p.y - 3.0).abs() < 1e-9));
        }
        let xs: Vec<f64> = segs.iter().map(|s| s.end().unwrap().x).collect();
        for (x, want) in xs.iter().zip([15.0, 5.0, -5.0, 5.0]) {
            assert!((x - want).abs() < 1e-9, "{xs:?}");
        }
    }
}
