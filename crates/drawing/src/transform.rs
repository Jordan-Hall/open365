//! 2-D affine transforms in points (y grows downward, positive rotation is clockwise).

use crate::geom::P;
use crate::model::{EMU_PER_PT, Xfrm};

/// `x' = a x + c y + e`, `y' = b x + d y + f`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Affine {
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub const fn translate(x: f64, y: f64) -> Self {
        Self {
            e: x,
            f: y,
            ..Self::IDENTITY
        }
    }

    pub const fn scale(x: f64, y: f64) -> Self {
        Self {
            a: x,
            d: y,
            ..Self::IDENTITY
        }
    }

    /// Rotation by `deg` degrees, clockwise on a y-down page.
    pub fn rotate_deg(deg: f64) -> Self {
        let (s, c) = deg.to_radians().sin_cos();
        Self {
            a: c,
            b: s,
            c: -s,
            d: c,
            e: 0.0,
            f: 0.0,
        }
    }

    /// `self` applied after `rhs` (matrix product `self * rhs`).
    pub fn mul(&self, rhs: &Self) -> Self {
        Self {
            a: self.a * rhs.a + self.c * rhs.b,
            b: self.b * rhs.a + self.d * rhs.b,
            c: self.a * rhs.c + self.c * rhs.d,
            d: self.b * rhs.c + self.d * rhs.d,
            e: self.a * rhs.e + self.c * rhs.f + self.e,
            f: self.b * rhs.e + self.d * rhs.f + self.f,
        }
    }

    pub fn apply(&self, p: P) -> P {
        P::new(
            self.a * p.x + self.c * p.y + self.e,
            self.b * p.x + self.d * p.y + self.f,
        )
    }
}

/// EMU to points.
pub fn pt(emu: i64) -> f64 {
    emu as f64 / EMU_PER_PT
}

/// Maps the object's local box (`0..w`, `0..h` points) to its parent's space: move to
/// `off`, flip about the box centre, then rotate about the centre.
pub fn box_matrix(x: &Xfrm) -> Affine {
    let (ox, oy) = (pt(x.off_x), pt(x.off_y));
    let (w, h) = (pt(x.ext_cx), pt(x.ext_cy));
    let (cx, cy) = (ox + w / 2.0, oy + h / 2.0);
    let flip = Affine::scale(
        if x.flip_h { -1.0 } else { 1.0 },
        if x.flip_v { -1.0 } else { 1.0 },
    );
    Affine::translate(cx, cy)
        .mul(&Affine::rotate_deg(x.rot as f64 / 60_000.0))
        .mul(&flip)
        .mul(&Affine::translate(-cx, -cy))
        .mul(&Affine::translate(ox, oy))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: P, x: f64, y: f64) -> bool {
        (a.x - x).abs() < 1e-9 && (a.y - y).abs() < 1e-9
    }

    fn xf(off: (i64, i64), ext: (i64, i64), rot: i64, fh: bool, fv: bool) -> Xfrm {
        Xfrm {
            off_x: off.0,
            off_y: off.1,
            ext_cx: ext.0,
            ext_cy: ext.1,
            rot,
            flip_h: fh,
            flip_v: fv,
        }
    }

    #[test]
    fn plain_box_is_a_translation_to_points() {
        let m = box_matrix(&xf(
            (127_000, 254_000),
            (1_270_000, 635_000),
            0,
            false,
            false,
        ));
        assert!(near(m.apply(P::new(0.0, 0.0)), 10.0, 20.0));
        assert!(near(m.apply(P::new(100.0, 50.0)), 110.0, 70.0));
    }

    #[test]
    fn quarter_turn_rotates_clockwise_about_the_centre() {
        // 100x50 pt box at origin; centre (50, 25). Rotate 90: top-left goes to top-right.
        let m = box_matrix(&xf((0, 0), (1_270_000, 635_000), 5_400_000, false, false));
        assert!(near(m.apply(P::new(0.0, 0.0)), 75.0, -25.0));
        assert!(near(m.apply(P::new(100.0, 0.0)), 75.0, 75.0));
    }

    #[test]
    fn flip_mirrors_inside_the_box() {
        let m = box_matrix(&xf((0, 0), (1_270_000, 635_000), 0, true, false));
        assert!(near(m.apply(P::new(0.0, 0.0)), 100.0, 0.0));
        let m = box_matrix(&xf((0, 0), (1_270_000, 635_000), 0, false, true));
        assert!(near(m.apply(P::new(0.0, 0.0)), 0.0, 50.0));
    }

    #[test]
    fn flip_applies_before_rotation() {
        let m = box_matrix(&xf((0, 0), (1_270_000, 635_000), 5_400_000, true, false));
        // flip: (0,0)->(100,0); then rotate 90 about (50,25): (100,0)->(75, 75)
        assert!(near(m.apply(P::new(0.0, 0.0)), 75.0, 75.0));
    }
}
