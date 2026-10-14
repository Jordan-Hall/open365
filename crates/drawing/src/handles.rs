//! Selection handles of a drawing object: where they are, which one a point hits, and the rectangle
//! that results from dragging one. All values are points in the space of the object's frame (for a
//! sheet: points from the corner of A1); rotation is degrees, clockwise, about the centre.
//!
//! Resizing keeps the opposite edge or corner fixed in the rotated frame, as every Office app does.

use crate::geom::P;

/// Distance of the rotation handle above the top edge, points. The handle sits on the object's
/// vertical axis; not yet checked against Excel (needs a pointer-driven oracle).
pub const ROTATE_OFFSET_PT: f64 = 18.0;
/// Smallest size a drag can make, points.
pub const MIN_SIZE_PT: f64 = 1.0;

/// An object frame: unrotated box plus rotation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub rot_deg: f64,
}

impl Frame {
    #[must_use]
    pub fn new(x: f64, y: f64, w: f64, h: f64, rot_deg: f64) -> Self {
        Self {
            x,
            y,
            w,
            h,
            rot_deg,
        }
    }

    #[must_use]
    pub fn centre(&self) -> P {
        P::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    fn sin_cos(&self) -> (f64, f64) {
        self.rot_deg.to_radians().sin_cos()
    }

    /// Frame-local offset `(lx, ly)` from the centre to the world point.
    fn local_of(&self, p: P) -> (f64, f64) {
        let c = self.centre();
        let (s, co) = self.sin_cos();
        let (dx, dy) = (p.x - c.x, p.y - c.y);
        (dx * co + dy * s, -dx * s + dy * co)
    }

    fn world_of(&self, lx: f64, ly: f64) -> P {
        let c = self.centre();
        let (s, co) = self.sin_cos();
        P::new(c.x + lx * co - ly * s, c.y + lx * s + ly * co)
    }

    /// True when `p` is inside the rotated box (edges included).
    #[must_use]
    pub fn contains(&self, p: P) -> bool {
        let (lx, ly) = self.local_of(p);
        lx.abs() <= self.w / 2.0 && ly.abs() <= self.h / 2.0
    }
}

/// A handle of the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Handle {
    NorthWest,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    Rotate,
}

impl Handle {
    /// The eight sizing handles, clockwise from the top left.
    pub const SIZING: [Self; 8] = [
        Self::NorthWest,
        Self::North,
        Self::NorthEast,
        Self::East,
        Self::SouthEast,
        Self::South,
        Self::SouthWest,
        Self::West,
    ];

    /// Which edges move: `(horizontal, vertical)` each -1, 0 or 1 (left/up is -1).
    fn dir(self) -> (f64, f64) {
        match self {
            Self::NorthWest => (-1.0, -1.0),
            Self::North => (0.0, -1.0),
            Self::NorthEast => (1.0, -1.0),
            Self::East => (1.0, 0.0),
            Self::SouthEast => (1.0, 1.0),
            Self::South => (0.0, 1.0),
            Self::SouthWest => (-1.0, 1.0),
            Self::West => (-1.0, 0.0),
            Self::Rotate => (0.0, 0.0),
        }
    }
}

/// Where a handle is, in world points.
#[must_use]
pub fn handle_point(f: &Frame, h: Handle) -> P {
    if h == Handle::Rotate {
        return f.world_of(0.0, -f.h / 2.0 - ROTATE_OFFSET_PT);
    }
    let (dx, dy) = h.dir();
    f.world_of(dx * f.w / 2.0, dy * f.h / 2.0)
}

/// The handle under `p` (within `tol` points), the rotation handle first, then corners, then edges.
/// `None` when the point is on no handle.
#[must_use]
pub fn hit_handle(f: &Frame, p: P, tol: f64) -> Option<Handle> {
    let near = |h: Handle| {
        let q = handle_point(f, h);
        (q.x - p.x).hypot(q.y - p.y) <= tol
    };
    if near(Handle::Rotate) {
        return Some(Handle::Rotate);
    }
    [
        Handle::NorthWest,
        Handle::NorthEast,
        Handle::SouthEast,
        Handle::SouthWest,
        Handle::North,
        Handle::East,
        Handle::South,
        Handle::West,
    ]
    .into_iter()
    .find(|h| near(*h))
}

/// The frame after dragging `handle` to the world point `to`. A corner handle with `keep_aspect`
/// keeps the width to height ratio. The rotation handle is [`rotation_to`].
#[must_use]
pub fn resize(f: &Frame, handle: Handle, to: P, keep_aspect: bool) -> Frame {
    let (dx, dy) = handle.dir();
    if handle == Handle::Rotate {
        return *f;
    }
    // The fixed point, local to the centre.
    let (ax, ay) = (-dx * f.w / 2.0, -dy * f.h / 2.0);
    let anchor = f.world_of(ax, ay);
    let probe = Frame {
        x: anchor.x,
        y: anchor.y,
        w: 0.0,
        h: 0.0,
        rot_deg: f.rot_deg,
    };
    let (qx, qy) = probe.local_of(to);
    let mut w = if dx == 0.0 {
        f.w
    } else {
        (dx * qx).max(MIN_SIZE_PT)
    };
    let mut h = if dy == 0.0 {
        f.h
    } else {
        (dy * qy).max(MIN_SIZE_PT)
    };
    if keep_aspect && dx != 0.0 && dy != 0.0 && f.w > 0.0 && f.h > 0.0 {
        let scale = (w / f.w).max(h / f.h);
        w = (f.w * scale).max(MIN_SIZE_PT);
        h = (f.h * scale).max(MIN_SIZE_PT);
    }
    // New centre: the fixed point plus half the new extent along the dragged directions.
    let local_centre_x = if dx == 0.0 { 0.0 } else { ax + dx * w / 2.0 };
    let local_centre_y = if dy == 0.0 { 0.0 } else { ay + dy * h / 2.0 };
    let c = f.world_of(local_centre_x, local_centre_y);
    Frame {
        x: c.x - w / 2.0,
        y: c.y - h / 2.0,
        w,
        h,
        rot_deg: f.rot_deg,
    }
}

/// The rotation (degrees, 0 up to 360) that points the frame's top at the world point `to`. With `snap`
/// the angle snaps to multiples of 15 degrees (Shift held).
#[must_use]
pub fn rotation_to(f: &Frame, to: P, snap: bool) -> f64 {
    let c = f.centre();
    let mut deg = (to.x - c.x).atan2(-(to.y - c.y)).to_degrees();
    if snap {
        deg = (deg / 15.0).round() * 15.0;
    }
    deg.rem_euclid(360.0)
}

/// The frame moved by `(dx, dy)` points.
#[must_use]
pub fn translate(f: &Frame, dx: f64, dy: f64) -> Frame {
    Frame {
        x: f.x + dx,
        y: f.y + dy,
        ..*f
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn unrotated_handles_sit_on_the_box() {
        let f = Frame::new(10.0, 20.0, 100.0, 50.0, 0.0);
        let nw = handle_point(&f, Handle::NorthWest);
        let se = handle_point(&f, Handle::SouthEast);
        assert!(close(nw.x, 10.0) && close(nw.y, 20.0));
        assert!(close(se.x, 110.0) && close(se.y, 70.0));
        let r = handle_point(&f, Handle::Rotate);
        assert!(close(r.x, 60.0) && close(r.y, 20.0 - ROTATE_OFFSET_PT));
        assert_eq!(hit_handle(&f, P::new(60.0, 2.5), 3.0), Some(Handle::Rotate));
        assert_eq!(
            hit_handle(&f, P::new(111.0, 71.0), 3.0),
            Some(Handle::SouthEast)
        );
        assert_eq!(hit_handle(&f, P::new(60.0, 45.0), 3.0), None);
    }

    #[test]
    fn dragging_a_corner_keeps_the_opposite_corner() {
        let f = Frame::new(10.0, 20.0, 100.0, 50.0, 0.0);
        let g = resize(&f, Handle::SouthEast, P::new(210.0, 120.0), false);
        assert!(close(g.x, 10.0) && close(g.y, 20.0) && close(g.w, 200.0) && close(g.h, 100.0));
        let g = resize(&f, Handle::NorthWest, P::new(0.0, 0.0), false);
        assert!(close(g.x, 0.0) && close(g.y, 0.0) && close(g.w, 110.0) && close(g.h, 70.0));
    }

    #[test]
    fn dragging_an_edge_moves_only_that_edge() {
        let f = Frame::new(10.0, 20.0, 100.0, 50.0, 0.0);
        let g = resize(&f, Handle::East, P::new(160.0, 999.0), false);
        assert!(close(g.x, 10.0) && close(g.y, 20.0) && close(g.w, 150.0) && close(g.h, 50.0));
        let g = resize(&f, Handle::North, P::new(-5.0, 0.0), false);
        assert!(close(g.y, 0.0) && close(g.h, 70.0) && close(g.w, 100.0));
    }

    #[test]
    fn rotated_resize_fixes_the_opposite_corner_in_the_world() {
        let f = Frame::new(0.0, 0.0, 100.0, 40.0, 30.0);
        let fixed = handle_point(&f, Handle::NorthWest);
        let drag = handle_point(&f, Handle::SouthEast);
        let g = resize(
            &f,
            Handle::SouthEast,
            P::new(drag.x + 10.0, drag.y + 5.0),
            false,
        );
        let still = handle_point(&g, Handle::NorthWest);
        assert!(close(fixed.x, still.x) && close(fixed.y, still.y));
        assert!(g.w > f.w - 1e-9 && close(g.rot_deg, 30.0));
    }

    #[test]
    fn aspect_lock_scales_both_sides() {
        let f = Frame::new(0.0, 0.0, 100.0, 50.0, 0.0);
        let g = resize(&f, Handle::SouthEast, P::new(200.0, 60.0), true);
        assert!(close(g.w / g.h, 2.0) && close(g.w, 200.0));
    }

    #[test]
    fn size_never_collapses() {
        let f = Frame::new(0.0, 0.0, 100.0, 50.0, 0.0);
        let g = resize(&f, Handle::SouthEast, P::new(-50.0, -50.0), false);
        assert!(close(g.w, MIN_SIZE_PT) && close(g.h, MIN_SIZE_PT));
    }

    #[test]
    fn rotation_points_the_top_at_the_pointer() {
        let f = Frame::new(0.0, 0.0, 100.0, 100.0, 0.0);
        assert!(close(rotation_to(&f, P::new(50.0, -100.0), false), 0.0));
        assert!(close(rotation_to(&f, P::new(200.0, 50.0), false), 90.0));
        assert!(close(rotation_to(&f, P::new(50.0, 200.0), false), 180.0));
        assert!(close(rotation_to(&f, P::new(-100.0, 50.0), false), 270.0));
        assert!(close(
            rotation_to(&f, P::new(110.0, -20.0), true) % 15.0,
            0.0
        ));
    }

    #[test]
    fn contains_follows_rotation() {
        let f = Frame::new(0.0, 0.0, 100.0, 10.0, 90.0);
        // A 100x10 bar turned a quarter turn is 10 wide and 100 tall about (50, 5).
        assert!(f.contains(P::new(50.0, 50.0)));
        assert!(!f.contains(P::new(95.0, 5.0)));
    }
}
