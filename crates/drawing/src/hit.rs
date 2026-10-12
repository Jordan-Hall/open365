//! Hit testing and handles. Only the bounding box exists so far; geometry hit tests,
//! resize/rotate handles and adjust handles arrive with M3.

use crate::geom::P;
use crate::model::Xfrm;
use crate::transform::{Affine, box_matrix, pt};

/// Axis-aligned bounds in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Bounds {
    pub fn width(&self) -> f64 {
        self.max_x - self.min_x
    }

    pub fn height(&self) -> f64 {
        self.max_y - self.min_y
    }

    /// True when `p` lies inside or on the edge.
    pub fn contains(&self, p: P) -> bool {
        p.x >= self.min_x && p.x <= self.max_x && p.y >= self.min_y && p.y <= self.max_y
    }
}

/// Bounds of the object's rotated, flipped box under `parent`.
pub fn bounding_box(x: &Xfrm, parent: &Affine) -> Bounds {
    let m = parent.mul(&box_matrix(x));
    let (w, h) = (pt(x.ext_cx), pt(x.ext_cy));
    let corners = [
        P::new(0.0, 0.0),
        P::new(w, 0.0),
        P::new(w, h),
        P::new(0.0, h),
    ]
    .map(|c| m.apply(c));
    let mut b = Bounds {
        min_x: f64::INFINITY,
        min_y: f64::INFINITY,
        max_x: f64::NEG_INFINITY,
        max_y: f64::NEG_INFINITY,
    };
    for c in corners {
        b.min_x = b.min_x.min(c.x);
        b.min_y = b.min_y.min(c.y);
        b.max_x = b.max_x.max(c.x);
        b.max_y = b.max_y.max(c.y);
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xf(x: i64, y: i64, cx: i64, cy: i64, rot: i64) -> Xfrm {
        Xfrm {
            off_x: x * 12_700,
            off_y: y * 12_700,
            ext_cx: cx * 12_700,
            ext_cy: cy * 12_700,
            rot,
            ..Xfrm::default()
        }
    }

    #[test]
    fn unrotated_bounds_are_the_box() {
        let b = bounding_box(&xf(10, 20, 100, 50, 0), &Affine::IDENTITY);
        assert_eq!(
            (b.min_x, b.min_y, b.max_x, b.max_y),
            (10.0, 20.0, 110.0, 70.0)
        );
        assert!(b.contains(P::new(110.0, 70.0)));
        assert!(!b.contains(P::new(111.0, 70.0)));
    }

    #[test]
    fn quarter_turn_swaps_width_and_height_about_the_centre() {
        let b = bounding_box(&xf(0, 0, 100, 50, 5_400_000), &Affine::IDENTITY);
        assert!((b.width() - 50.0).abs() < 1e-9 && (b.height() - 100.0).abs() < 1e-9);
        assert!((b.min_x - 25.0).abs() < 1e-9 && (b.min_y + 25.0).abs() < 1e-9);
    }

    #[test]
    fn forty_five_degrees_grows_the_bounds() {
        let b = bounding_box(&xf(0, 0, 100, 100, 2_700_000), &Affine::IDENTITY);
        let d = 100.0 * 2f64.sqrt();
        assert!((b.width() - d).abs() < 1e-9 && (b.height() - d).abs() < 1e-9);
    }

    #[test]
    fn parent_transform_applies() {
        let b = bounding_box(&xf(0, 0, 10, 10, 0), &Affine::translate(5.0, 7.0));
        assert_eq!((b.min_x, b.min_y), (5.0, 7.0));
    }
}
