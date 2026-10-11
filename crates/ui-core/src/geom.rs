//! Logical geometry and per-scale snapping. Widgets are authored in logical units (1 unit = one
//! pixel at 100 %); device rectangles come from rounding edges, never sizes, so neighbours that
//! touch in logical space touch in device space at every scale.

/// A point in logical units.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A rectangle in logical units.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    pub fn translate(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        Rect::new(
            x,
            y,
            (self.right().min(o.right()) - x).max(0.0),
            (self.bottom().min(o.bottom()) - y).max(0.0),
        )
    }

    pub fn inset(&self, d: f32) -> Rect {
        Rect::new(
            self.x + d,
            self.y + d,
            (self.w - 2.0 * d).max(0.0),
            (self.h - 2.0 * d).max(0.0),
        )
    }

    pub fn center(&self) -> Point {
        Point::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// A device-pixel rectangle with integer edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeviceRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl DeviceRect {
    pub fn width(&self) -> i32 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> i32 {
        self.y1 - self.y0
    }

    pub fn to_gfx(self) -> open365_gfx::Rect {
        open365_gfx::Rect::from_edges(
            self.x0 as f32,
            self.y0 as f32,
            self.x1 as f32,
            self.y1 as f32,
        )
    }
}

/// Device pixels per logical unit (1.0, 1.25, 1.5, 2.0 ...).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale(pub f32);

impl Scale {
    /// Rounds a logical coordinate to a device edge. The small bias keeps values such as
    /// `70 / 1.25 * 1.25 = 69.99999` on the intended edge.
    pub fn edge(self, v: f32) -> i32 {
        (v * self.0 + 1e-3).round() as i32
    }

    /// Snaps a logical rectangle by its edges.
    pub fn snap(self, r: Rect) -> DeviceRect {
        DeviceRect {
            x0: self.edge(r.x),
            y0: self.edge(r.y),
            x1: self.edge(r.right()),
            y1: self.edge(r.bottom()),
        }
    }

    /// Device pixels to logical units.
    pub fn to_logical(self, device: f32) -> f32 {
        device / self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjacent_rects_share_an_edge_at_every_scale() {
        for s in [1.0, 1.25, 1.5, 1.75, 2.0, 2.5] {
            let sc = Scale(s);
            let a = Rect::new(10.3, 0.0, 33.7, 10.0);
            let b = Rect::new(a.right(), 0.0, 20.1, 10.0);
            assert_eq!(sc.snap(a).x1, sc.snap(b).x0, "scale {s}");
        }
    }

    #[test]
    fn spec_units_land_on_exact_pixels_at_the_reference_scale() {
        let sc = Scale(1.25);
        assert_eq!(sc.edge(82.0 / 1.25), 82);
        assert_eq!(sc.edge(152.0 / 1.25), 152);
        assert_eq!(
            sc.snap(Rect::new(
                14.0 / 1.25,
                61.0 / 1.25,
                68.0 / 1.25,
                38.0 / 1.25
            ))
            .width(),
            68
        );
    }

    #[test]
    fn contains_is_half_open() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(r.contains(Point::new(0.0, 0.0)));
        assert!(!r.contains(Point::new(10.0, 5.0)));
    }
}
