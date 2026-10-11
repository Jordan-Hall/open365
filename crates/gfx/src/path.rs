//! Vector paths: the input of [`DrawList::fill_path`](crate::DrawList::fill_path), path clips and
//! strokes. A path is flattened to line segments and handed to the exact-area rasteriser, so
//! shapes get the same analytic coverage as glyphs.

use resvg::tiny_skia;
use skrifa::outline::OutlinePen;

use crate::geom::Rect;
use crate::raster::Flattener;

/// One path command, in device pixels (y down).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathCmd {
    MoveTo([f32; 2]),
    LineTo([f32; 2]),
    QuadTo([f32; 2], [f32; 2]),
    CubicTo([f32; 2], [f32; 2], [f32; 2]),
    Close,
}

/// A sequence of contours.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Path {
    cmds: Vec<PathCmd>,
}

/// Line joins for [`Path::stroked`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Join {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// Line caps for [`Path::stroked`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cap {
    #[default]
    Butt,
    Round,
    Square,
}

/// Circle-to-cubic constant (4/3 * (sqrt(2) - 1)).
const KAPPA: f32 = 0.552_284_8;

impl Path {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn commands(&self) -> &[PathCmd] {
        &self.cmds
    }

    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }

    pub fn move_to(&mut self, x: f32, y: f32) -> &mut Self {
        self.cmds.push(PathCmd::MoveTo([x, y]));
        self
    }

    pub fn line_to(&mut self, x: f32, y: f32) -> &mut Self {
        self.cmds.push(PathCmd::LineTo([x, y]));
        self
    }

    pub fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) -> &mut Self {
        self.cmds.push(PathCmd::QuadTo([cx, cy], [x, y]));
        self
    }

    pub fn cubic_to(&mut self, c0: [f32; 2], c1: [f32; 2], p: [f32; 2]) -> &mut Self {
        self.cmds.push(PathCmd::CubicTo(c0, c1, p));
        self
    }

    pub fn close(&mut self) -> &mut Self {
        self.cmds.push(PathCmd::Close);
        self
    }

    /// A closed rectangle.
    pub fn rect(r: Rect) -> Self {
        let mut p = Self::new();
        p.move_to(r.x0, r.y0)
            .line_to(r.x1, r.y0)
            .line_to(r.x1, r.y1)
            .line_to(r.x0, r.y1)
            .close();
        p
    }

    /// A closed rounded rectangle (radius clamped to half the shorter side).
    pub fn rounded_rect(r: Rect, radius: f32) -> Self {
        let rad = radius.clamp(0.0, r.width().min(r.height()) / 2.0);
        if rad <= 0.0 {
            return Self::rect(r);
        }
        let k = rad * KAPPA;
        let mut p = Self::new();
        p.move_to(r.x0 + rad, r.y0)
            .line_to(r.x1 - rad, r.y0)
            .cubic_to(
                [r.x1 - rad + k, r.y0],
                [r.x1, r.y0 + rad - k],
                [r.x1, r.y0 + rad],
            )
            .line_to(r.x1, r.y1 - rad)
            .cubic_to(
                [r.x1, r.y1 - rad + k],
                [r.x1 - rad + k, r.y1],
                [r.x1 - rad, r.y1],
            )
            .line_to(r.x0 + rad, r.y1)
            .cubic_to(
                [r.x0 + rad - k, r.y1],
                [r.x0, r.y1 - rad + k],
                [r.x0, r.y1 - rad],
            )
            .line_to(r.x0, r.y0 + rad)
            .cubic_to(
                [r.x0, r.y0 + rad - k],
                [r.x0 + rad - k, r.y0],
                [r.x0 + rad, r.y0],
            )
            .close();
        p
    }

    /// A closed ellipse inscribed in `r`.
    pub fn ellipse(r: Rect) -> Self {
        let (cx, cy) = ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0);
        let (rx, ry) = (r.width() / 2.0, r.height() / 2.0);
        let (kx, ky) = (rx * KAPPA, ry * KAPPA);
        let mut p = Self::new();
        p.move_to(cx + rx, cy)
            .cubic_to([cx + rx, cy + ky], [cx + kx, cy + ry], [cx, cy + ry])
            .cubic_to([cx - kx, cy + ry], [cx - rx, cy + ky], [cx - rx, cy])
            .cubic_to([cx - rx, cy - ky], [cx - kx, cy - ry], [cx, cy - ry])
            .cubic_to([cx + kx, cy - ry], [cx + rx, cy - ky], [cx + rx, cy])
            .close();
        p
    }

    /// The same path moved by `(dx, dy)`.
    pub fn translated(&self, dx: f32, dy: f32) -> Self {
        let mv = |p: [f32; 2]| [p[0] + dx, p[1] + dy];
        Self {
            cmds: self
                .cmds
                .iter()
                .map(|c| match *c {
                    PathCmd::MoveTo(p) => PathCmd::MoveTo(mv(p)),
                    PathCmd::LineTo(p) => PathCmd::LineTo(mv(p)),
                    PathCmd::QuadTo(a, b) => PathCmd::QuadTo(mv(a), mv(b)),
                    PathCmd::CubicTo(a, b, c) => PathCmd::CubicTo(mv(a), mv(b), mv(c)),
                    PathCmd::Close => PathCmd::Close,
                })
                .collect(),
        }
    }

    /// Flattened line segments `[x0, y0, x1, y1]` with y negated (the rasteriser works y up).
    pub(crate) fn lines_y_up(&self) -> Vec<[f32; 4]> {
        let mut f = Flattener::default();
        for c in &self.cmds {
            match *c {
                PathCmd::MoveTo(p) => f.move_to(p[0], -p[1]),
                PathCmd::LineTo(p) => f.line_to(p[0], -p[1]),
                PathCmd::QuadTo(a, b) => f.quad_to(a[0], -a[1], b[0], -b[1]),
                PathCmd::CubicTo(a, b, c) => {
                    f.curve_to(a[0], -a[1], b[0], -b[1], c[0], -c[1]);
                }
                PathCmd::Close => f.close(),
            }
        }
        f.finish()
    }

    /// Bounds of the flattened outline, or `None` for an empty path.
    pub fn bounds(&self) -> Option<Rect> {
        let lines = self.lines_y_up();
        if lines.is_empty() {
            return None;
        }
        let mut r = Rect::from_edges(f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for l in lines {
            for (x, y) in [(l[0], -l[1]), (l[2], -l[3])] {
                r.x0 = r.x0.min(x);
                r.x1 = r.x1.max(x);
                r.y0 = r.y0.min(y);
                r.y1 = r.y1.max(y);
            }
        }
        Some(r)
    }

    fn to_skia(&self) -> Option<tiny_skia::Path> {
        let mut b = tiny_skia::PathBuilder::new();
        for c in &self.cmds {
            match *c {
                PathCmd::MoveTo(p) => b.move_to(p[0], p[1]),
                PathCmd::LineTo(p) => b.line_to(p[0], p[1]),
                PathCmd::QuadTo(a, p) => b.quad_to(a[0], a[1], p[0], p[1]),
                PathCmd::CubicTo(a, c, p) => b.cubic_to(a[0], a[1], c[0], c[1], p[0], p[1]),
                PathCmd::Close => b.close(),
            }
        }
        b.finish()
    }

    /// The outline of this path stroked `width` pixels wide, as a fillable path.
    pub fn stroked(&self, width: f32, join: Join, cap: Cap) -> Option<Path> {
        let skia = self.to_skia()?;
        let stroke = tiny_skia::Stroke {
            width: width.max(0.01),
            line_join: match join {
                Join::Miter => tiny_skia::LineJoin::Miter,
                Join::Round => tiny_skia::LineJoin::Round,
                Join::Bevel => tiny_skia::LineJoin::Bevel,
            },
            line_cap: match cap {
                Cap::Butt => tiny_skia::LineCap::Butt,
                Cap::Round => tiny_skia::LineCap::Round,
                Cap::Square => tiny_skia::LineCap::Square,
            },
            ..tiny_skia::Stroke::default()
        };
        let out = skia.stroke(&stroke, 1.0)?;
        let mut p = Path::new();
        for seg in out.segments() {
            match seg {
                tiny_skia::PathSegment::MoveTo(a) => p.move_to(a.x, a.y),
                tiny_skia::PathSegment::LineTo(a) => p.line_to(a.x, a.y),
                tiny_skia::PathSegment::QuadTo(a, b) => p.quad_to(a.x, a.y, b.x, b.y),
                tiny_skia::PathSegment::CubicTo(a, b, c) => {
                    p.cubic_to([a.x, a.y], [b.x, b.y], [c.x, c.y])
                }
                tiny_skia::PathSegment::Close => p.close(),
            };
        }
        Some(p)
    }

    /// A stable hash of the geometry, for mask caching.
    pub(crate) fn key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for c in &self.cmds {
            match *c {
                PathCmd::MoveTo(p) => (0u8, p.map(f32::to_bits)).hash(&mut h),
                PathCmd::LineTo(p) => (1u8, p.map(f32::to_bits)).hash(&mut h),
                PathCmd::QuadTo(a, b) => {
                    (2u8, a.map(f32::to_bits), b.map(f32::to_bits)).hash(&mut h)
                }
                PathCmd::CubicTo(a, b, c) => (
                    3u8,
                    a.map(f32::to_bits),
                    b.map(f32::to_bits),
                    c.map(f32::to_bits),
                )
                    .hash(&mut h),
                PathCmd::Close => 4u8.hash(&mut h),
            }
        }
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_path_bounds_are_the_rect() {
        let b = Path::rect(Rect::from_xywh(2.0, 3.0, 10.0, 4.0))
            .bounds()
            .expect("bounds");
        assert_eq!(b, Rect::from_edges(2.0, 3.0, 12.0, 7.0));
    }

    #[test]
    fn rounded_rect_stays_inside_its_rect() {
        let r = Rect::from_xywh(0.0, 0.0, 20.0, 10.0);
        let b = Path::rounded_rect(r, 4.0).bounds().expect("bounds");
        assert!(b.x0 >= -0.01 && b.y0 >= -0.01 && b.x1 <= 20.01 && b.y1 <= 10.01);
        assert!(b.width() > 19.9 && b.height() > 9.9);
    }

    #[test]
    fn stroke_of_a_line_is_a_band() {
        let mut p = Path::new();
        p.move_to(0.0, 5.0).line_to(10.0, 5.0);
        let s = p.stroked(2.0, Join::Miter, Cap::Butt).expect("stroke");
        let b = s.bounds().expect("bounds");
        assert!(
            (b.y0 - 4.0).abs() < 0.01 && (b.y1 - 6.0).abs() < 0.01,
            "{b:?}"
        );
        assert!((b.x0 - 0.0).abs() < 0.01 && (b.x1 - 10.0).abs() < 0.01);
    }

    #[test]
    fn translation_moves_bounds() {
        let p = Path::ellipse(Rect::from_xywh(0.0, 0.0, 8.0, 8.0)).translated(5.0, 7.0);
        let b = p.bounds().expect("bounds");
        assert!((b.x0 - 5.0).abs() < 0.05 && (b.y0 - 7.0).abs() < 0.05);
    }

    #[test]
    fn equal_paths_share_a_key() {
        let a = Path::rect(Rect::from_xywh(0.0, 0.0, 3.0, 3.0));
        assert_eq!(a.key(), a.clone().key());
        assert_ne!(a.key(), a.translated(1.0, 0.0).key());
    }
}
