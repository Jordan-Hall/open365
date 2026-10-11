//! The draw list: CPU tessellation of shapes into vertices, indices and clip-scoped batches.
//! Coordinates are device pixels. Building a list needs no GPU.

use bytemuck::{Pod, Zeroable};

use crate::assets::{Assets, FontId, GlyphRun, IconId, ImageId};
use crate::color::Color;
use crate::geom::{PixelRect, Rect};
use crate::path::{Cap, Join, Path};
use crate::raster::{RasterMode, rasterize};
use crate::snap::{snap_rect, snap_stroke, split_subpixel, stroke_pixels};
use crate::text::TextLayout;

/// Vertex shading modes; the values are shared with `shader.wgsl`.
pub mod mode {
    pub const SOLID: u32 = 0;
    pub const ROUND_RECT: u32 = 1;
    pub const IMAGE: u32 = 2;
    /// Grayscale glyph coverage from the mask atlas (text gamma applies).
    pub const GLYPH: u32 = 3;
    /// Per-channel glyph coverage from the LCD atlas.
    pub const GLYPH_LCD: u32 = 4;
    /// Icon coverage from the mask atlas (no text gamma).
    pub const ICON: u32 = 5;
    /// Vertex colours interpolate in sRGB (gradients).
    pub const GRADIENT: u32 = 6;
    /// Blurred rounded-box shadow; `params` is `[half_w, half_h, radius, sigma]`.
    pub const SHADOW: u32 = 7;
}

/// A coverage mask in the mask atlas that clips a batch (rounded and path clips). `x`, `y` is
/// the mask's origin in the atlas; `ox`, `oy` is the device pixel its first texel covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipMask {
    pub x: u32,
    pub y: u32,
    pub ox: i32,
    pub oy: i32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClipEntry {
    rect: PixelRect,
    mask: Option<ClipMask>,
}

/// A linear gradient between two points (device pixels) with colour stops at `t` in `0..=1`.
/// Interpolation is in sRGB, as Office draws gradients.
#[derive(Debug, Clone, PartialEq)]
pub struct LinearGradient {
    pub start: [f32; 2],
    pub end: [f32; 2],
    pub stops: Vec<(f32, Color)>,
}

impl LinearGradient {
    pub fn vertical(y0: f32, y1: f32, top: Color, bottom: Color) -> Self {
        Self {
            start: [0.0, y0],
            end: [0.0, y1],
            stops: vec![(0.0, top), (1.0, bottom)],
        }
    }

    pub fn horizontal(x0: f32, x1: f32, left: Color, right: Color) -> Self {
        Self {
            start: [x0, 0.0],
            end: [x1, 0.0],
            stops: vec![(0.0, left), (1.0, right)],
        }
    }

    /// Colour at `t`, clamped to the first and last stop.
    pub fn color_at(&self, t: f32) -> Color {
        let (Some(first), Some(last)) = (self.stops.first(), self.stops.last()) else {
            return Color::TRANSPARENT;
        };
        if t <= first.0 {
            return first.1;
        }
        for w in self.stops.windows(2) {
            let ((t0, c0), (t1, c1)) = (w[0], w[1]);
            if t <= t1 {
                let k = if t1 > t0 { (t - t0) / (t1 - t0) } else { 1.0 };
                let mix =
                    |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * k).round() as u8;
                return Color::rgba(
                    mix(c0.r, c1.r),
                    mix(c0.g, c1.g),
                    mix(c0.b, c1.b),
                    mix(c0.a, c1.a),
                );
            }
        }
        last.1
    }

    fn t_at(&self, p: [f32; 2]) -> f32 {
        let (dx, dy) = (self.end[0] - self.start[0], self.end[1] - self.start[1]);
        let len2 = dx * dx + dy * dy;
        if len2 <= 0.0 {
            return 0.0;
        }
        ((p[0] - self.start[0]) * dx + (p[1] - self.start[1]) * dy) / len2
    }
}

/// Text decorations drawn with a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Decorations {
    pub underline: bool,
    pub strikethrough: bool,
}

/// One vertex. `uv` is atlas texels for sampled modes and the offset from the shape centre for
/// [`mode::ROUND_RECT`]; `params` is `[half_w, half_h, radius, border_width]` for rounded rects.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    /// sRGB, straight alpha.
    pub color: [u8; 4],
    pub mode: u32,
    pub params: [f32; 4],
}

/// A run of indices drawn under one scissor rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Batch {
    pub clip: PixelRect,
    /// Extra coverage mask for rounded and path clips.
    pub mask: Option<ClipMask>,
    /// Drawn with the three-pass LCD pipeline.
    pub lcd: bool,
    pub first_index: u32,
    pub index_count: u32,
}

/// Vertices, indices and batches for one frame.
#[derive(Debug, Clone)]
pub struct DrawList {
    size: (u32, u32),
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    batches: Vec<Batch>,
    clips: Vec<ClipEntry>,
}

impl DrawList {
    /// A list for a `width` by `height` pixel target.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            size: (width, height),
            vertices: Vec::new(),
            indices: Vec::new(),
            batches: Vec::new(),
            clips: vec![ClipEntry {
                rect: PixelRect::new(0, 0, width, height),
                mask: None,
            }],
        }
    }

    /// Empties the list for reuse, keeping its allocations.
    pub fn reset(&mut self, width: u32, height: u32) {
        self.size = (width, height);
        self.vertices.clear();
        self.indices.clear();
        self.batches.clear();
        self.clips.clear();
        self.clips.push(ClipEntry {
            rect: PixelRect::new(0, 0, width, height),
            mask: None,
        });
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn vertices(&self) -> &[Vertex] {
        &self.vertices
    }

    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    pub fn batches(&self) -> &[Batch] {
        &self.batches
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// The scissor rectangle currently in force.
    pub fn clip(&self) -> PixelRect {
        self.clips.last().map(|c| c.rect).unwrap_or_default()
    }

    /// The coverage mask of the innermost rounded or path clip, if any.
    pub fn clip_mask(&self) -> Option<ClipMask> {
        self.clips.last().and_then(|c| c.mask)
    }

    /// Intersects the current clip with `rect` (rounded to whole pixels).
    pub fn push_clip(&mut self, rect: Rect) {
        let r = snap_rect(rect);
        let clamp = |v: f32| v.max(0.0) as u32;
        let new = PixelRect::new(clamp(r.x0), clamp(r.y0), clamp(r.x1), clamp(r.y1));
        let merged = self.clip().intersect(&new);
        self.clips.push(ClipEntry {
            rect: merged,
            mask: self.clip_mask(),
        });
    }

    /// Clips to a rounded rectangle (edges snapped to pixels, corners antialiased).
    pub fn push_clip_rrect(&mut self, assets: &mut Assets, rect: Rect, radius: f32) {
        let r = snap_rect(rect);
        self.push_clip_path(assets, &Path::rounded_rect(r, radius));
    }

    /// Clips to `path` (device pixels). Nested path clips multiply their coverage.
    pub fn push_clip_path(&mut self, assets: &mut Assets, path: &Path) {
        let parent = self.clips.last().copied().unwrap_or(ClipEntry {
            rect: PixelRect::default(),
            mask: None,
        });
        let empty = ClipEntry {
            rect: PixelRect::default(),
            mask: None,
        };
        let Some(bm) = rasterize(&path.lines_y_up(), 0.0, RasterMode::Gray) else {
            self.clips.push(empty);
            return;
        };
        let (ox, oy) = (bm.left, -bm.top);
        let clamp = |v: i32| v.max(0) as u32;
        let bounds = PixelRect::new(
            clamp(ox),
            clamp(oy),
            clamp(ox + bm.width as i32),
            clamp(oy + bm.height as i32),
        );
        let rect = parent.rect.intersect(&bounds);
        let mut data = bm.data;
        if let Some(old) = parent.mask {
            for y in 0..bm.height {
                for x in 0..bm.width {
                    let (dx, dy) = (ox + x as i32 - old.ox, oy + y as i32 - old.oy);
                    let o = if dx >= 0 && dy >= 0 && (dx as u32) < old.w && (dy as u32) < old.h {
                        u32::from(assets.mask().pixel(old.x + dx as u32, old.y + dy as u32)[0])
                    } else {
                        0
                    };
                    let i = (y * bm.width + x) as usize;
                    data[i] = ((u32::from(data[i]) * o + 127) / 255) as u8;
                }
            }
        }
        let Some(slot) = assets.mask_atlas().insert(bm.width, bm.height, &data) else {
            assets.overflowed = true;
            assets.reset_glyph_caches();
            self.clips.push(empty);
            return;
        };
        self.clips.push(ClipEntry {
            rect,
            mask: Some(ClipMask {
                x: slot.x,
                y: slot.y,
                ox,
                oy,
                w: bm.width,
                h: bm.height,
            }),
        });
    }

    /// Restores the previous clip. The base clip (the whole target) is never popped.
    pub fn pop_clip(&mut self) {
        if self.clips.len() > 1 {
            self.clips.pop();
        }
    }

    pub fn clip_depth(&self) -> usize {
        self.clips.len() - 1
    }

    fn push_quad(
        &mut self,
        corners: [[f32; 2]; 4],
        uvs: [[f32; 2]; 4],
        color: Color,
        mode: u32,
        params: [f32; 4],
        lcd: bool,
    ) {
        self.push_quad_c(corners, uvs, [color; 4], mode, params, lcd);
    }

    fn push_quad_c(
        &mut self,
        corners: [[f32; 2]; 4],
        uvs: [[f32; 2]; 4],
        colors: [Color; 4],
        mode: u32,
        params: [f32; 4],
        lcd: bool,
    ) {
        let clip = self.clip();
        let mask = self.clip_mask();
        if clip.is_empty() {
            return;
        }
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for c in corners {
            min_x = min_x.min(c[0]);
            max_x = max_x.max(c[0]);
            min_y = min_y.min(c[1]);
            max_y = max_y.max(c[1]);
        }
        if !clip.overlaps(&Rect::from_edges(min_x, min_y, max_x, max_y)) {
            return;
        }
        let base = self.vertices.len() as u32;
        for ((pos, uv), color) in corners.into_iter().zip(uvs).zip(colors) {
            self.vertices.push(Vertex {
                pos,
                uv,
                color: color.to_array(),
                mode,
                params,
            });
        }
        let first = self.indices.len() as u32;
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        match self.batches.last_mut() {
            Some(b) if b.clip == clip && b.mask == mask && b.lcd == lcd => b.index_count += 6,
            _ => self.batches.push(Batch {
                clip,
                mask,
                lcd,
                first_index: first,
                index_count: 6,
            }),
        }
    }

    fn quad_for(r: Rect) -> [[f32; 2]; 4] {
        [[r.x0, r.y0], [r.x1, r.y0], [r.x1, r.y1], [r.x0, r.y1]]
    }

    /// Fills `rect` after snapping its edges to the pixel grid (no antialiasing fringe).
    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        let r = snap_rect(rect);
        if r.is_empty() || color.a == 0 {
            return;
        }
        self.push_quad(
            Self::quad_for(r),
            [[0.0; 2]; 4],
            color,
            mode::SOLID,
            [0.0; 4],
            false,
        );
    }

    /// A border drawn inside `rect`, `width` pixels thick (rounded to whole pixels, at least one).
    pub fn rect_outline(&mut self, rect: Rect, width: f32, color: Color) {
        let r = snap_rect(rect);
        let w = (stroke_pixels(width) as f32)
            .min(r.width() / 2.0)
            .min(r.height() / 2.0)
            .max(0.0);
        if r.is_empty() || w <= 0.0 {
            return;
        }
        self.fill_rect(Rect::from_edges(r.x0, r.y0, r.x1, r.y0 + w), color);
        self.fill_rect(Rect::from_edges(r.x0, r.y1 - w, r.x1, r.y1), color);
        self.fill_rect(Rect::from_edges(r.x0, r.y0 + w, r.x0 + w, r.y1 - w), color);
        self.fill_rect(Rect::from_edges(r.x1 - w, r.y0 + w, r.x1, r.y1 - w), color);
    }

    /// A horizontal rule centred on `y`, from `x0` to `x1`, snapped to whole pixels.
    pub fn hline(&mut self, y: f32, x0: f32, x1: f32, width: f32, color: Color) {
        let (a, b) = snap_stroke(y, width);
        self.fill_rect(Rect::from_edges(x0.min(x1), a, x0.max(x1), b), color);
    }

    /// A vertical rule centred on `x`, from `y0` to `y1`, snapped to whole pixels.
    pub fn vline(&mut self, x: f32, y0: f32, y1: f32, width: f32, color: Color) {
        let (a, b) = snap_stroke(x, width);
        self.fill_rect(Rect::from_edges(a, y0.min(y1), b, y0.max(y1)), color);
    }

    /// A line segment. Axis-aligned lines snap exactly like [`hline`](Self::hline) and
    /// [`vline`](Self::vline); other angles are antialiased and not snapped.
    pub fn line(&mut self, p0: [f32; 2], p1: [f32; 2], width: f32, color: Color) {
        const EPS: f32 = 1e-4;
        if (p0[1] - p1[1]).abs() < EPS {
            self.hline(p0[1], p0[0], p1[0], width, color);
        } else if (p0[0] - p1[0]).abs() < EPS {
            self.vline(p0[0], p0[1], p1[1], width, color);
        } else {
            self.diagonal_line(p0, p1, width.max(1.0), color);
        }
    }

    fn diagonal_line(&mut self, p0: [f32; 2], p1: [f32; 2], width: f32, color: Color) {
        let (dx, dy) = (p1[0] - p0[0], p1[1] - p0[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len <= 0.0 || color.a == 0 {
            return;
        }
        let (ux, uy) = (dx / len, dy / len);
        let (vx, vy) = (-uy, ux);
        let (hl, hw) = (len / 2.0, width / 2.0);
        let (cx, cy) = ((p0[0] + p1[0]) / 2.0, (p0[1] + p1[1]) / 2.0);
        // One pixel of margin on every side holds the antialiased fringe.
        let (el, ew) = (hl + 1.0, hw + 1.0);
        let corner = |a: f32, b: f32| [cx + ux * a + vx * b, cy + uy * a + vy * b];
        self.push_quad(
            [
                corner(-el, -ew),
                corner(el, -ew),
                corner(el, ew),
                corner(-el, ew),
            ],
            [[-el, -ew], [el, -ew], [el, ew], [-el, ew]],
            color,
            mode::ROUND_RECT,
            [hl, hw, 0.0, 0.0],
            false,
        );
    }

    fn rrect(&mut self, rect: Rect, radius: f32, border: f32, color: Color) {
        let r = snap_rect(rect);
        if r.is_empty() || color.a == 0 {
            return;
        }
        let (hw, hh) = (r.width() / 2.0, r.height() / 2.0);
        let radius = radius.clamp(0.0, hw.min(hh));
        let (cx, cy) = (r.x0 + hw, r.y0 + hh);
        let e = r.inflate(1.0);
        self.push_quad(
            Self::quad_for(e),
            [
                [e.x0 - cx, e.y0 - cy],
                [e.x1 - cx, e.y0 - cy],
                [e.x1 - cx, e.y1 - cy],
                [e.x0 - cx, e.y1 - cy],
            ],
            color,
            mode::ROUND_RECT,
            [hw, hh, radius, border],
            false,
        );
    }

    /// A filled rounded rectangle; edges snap to pixels, corners are antialiased.
    pub fn fill_rrect(&mut self, rect: Rect, radius: f32, color: Color) {
        self.rrect(rect, radius, 0.0, color);
    }

    /// A rounded-rectangle border drawn inside `rect`.
    pub fn stroke_rrect(&mut self, rect: Rect, radius: f32, width: f32, color: Color) {
        self.rrect(rect, radius, stroke_pixels(width) as f32, color);
    }

    /// Draws a registered image into `dest`. Without scaling and with an integer `dest`
    /// origin every texel maps to exactly one pixel. `tint` multiplies the image (white keeps it).
    pub fn image(&mut self, assets: &Assets, id: ImageId, dest: Rect, tint: Color) {
        let Some(slot) = assets.image(id) else { return };
        let (x, y) = (slot.x as f32, slot.y as f32);
        let (w, h) = (slot.width as f32, slot.height as f32);
        self.push_quad(
            Self::quad_for(dest),
            [[x, y], [x + w, y], [x + w, y + h], [x, y + h]],
            tint,
            mode::IMAGE,
            [0.0; 4],
            false,
        );
    }

    /// Draws an SVG icon, rasterised for the pixel size of `dest` (rounded) and tinted `color`.
    pub fn icon(&mut self, assets: &mut Assets, id: IconId, dest: Rect, color: Color) {
        let r = snap_rect(dest);
        if r.is_empty() {
            return;
        }
        let (w, h) = (r.width() as u32, r.height() as u32);
        let Some(p) = assets.icon_mask(id, w, h) else {
            return;
        };
        let (x, y) = (p.slot.x as f32, p.slot.y as f32);
        self.push_quad(
            Self::quad_for(r),
            [
                [x, y],
                [x + w as f32, y],
                [x + w as f32, y + h as f32],
                [x, y + h as f32],
            ],
            color,
            mode::ICON,
            [0.0; 4],
            false,
        );
    }

    /// Fills `path` (device pixels) with exact-area antialiasing.
    pub fn fill_path(&mut self, assets: &mut Assets, path: &Path, color: Color) {
        if color.a == 0 {
            return;
        }
        let Some(p) = assets.path_mask(path) else {
            return;
        };
        let (x0, y0) = (p.left as f32, -(p.top as f32));
        let (w, h) = (p.slot.width as f32, p.slot.height as f32);
        let (u, v) = (p.slot.x as f32, p.slot.y as f32);
        self.push_quad(
            Self::quad_for(Rect::from_xywh(x0, y0, w, h)),
            [[u, v], [u + w, v], [u + w, v + h], [u, v + h]],
            color,
            mode::ICON,
            [0.0; 4],
            false,
        );
    }

    /// Strokes `path` with a line `width` pixels wide.
    pub fn stroke_path(
        &mut self,
        assets: &mut Assets,
        path: &Path,
        width: f32,
        join: Join,
        cap: Cap,
        color: Color,
    ) {
        if let Some(outline) = path.stroked(width, join, cap) {
            self.fill_path(assets, &outline, color);
        }
    }

    /// Fills `rect` (snapped) with a linear gradient. Horizontal and vertical gradients take any
    /// number of stops; other angles are exact for the two end colours when the gradient spans
    /// the rectangle.
    pub fn fill_gradient(&mut self, rect: Rect, g: &LinearGradient) {
        let r = snap_rect(rect);
        if r.is_empty() || g.stops.is_empty() {
            return;
        }
        const EPS: f32 = 1e-4;
        let (dx, dy) = (g.end[0] - g.start[0], g.end[1] - g.start[1]);
        let horizontal = dy.abs() < EPS && dx.abs() >= EPS;
        let vertical = dx.abs() < EPS && dy.abs() >= EPS;
        if horizontal || vertical {
            let (lo, hi, p0, p1) = if horizontal {
                (r.x0, r.x1, g.start[0], g.end[0])
            } else {
                (r.y0, r.y1, g.start[1], g.end[1])
            };
            let mut cuts: Vec<f32> = vec![lo, hi];
            for (t, _) in &g.stops {
                let pos = p0 + t * (p1 - p0);
                if pos > lo && pos < hi {
                    cuts.push(pos);
                }
            }
            cuts.sort_by(f32::total_cmp);
            let color_at = |pos: f32| g.color_at((pos - p0) / (p1 - p0));
            for w in cuts.windows(2) {
                let (a, b) = (w[0], w[1]);
                if b <= a {
                    continue;
                }
                let (ca, cb) = (color_at(a), color_at(b));
                let (corners, colors) = if horizontal {
                    (
                        [[a, r.y0], [b, r.y0], [b, r.y1], [a, r.y1]],
                        [ca, cb, cb, ca],
                    )
                } else {
                    (
                        [[r.x0, a], [r.x1, a], [r.x1, b], [r.x0, b]],
                        [ca, ca, cb, cb],
                    )
                };
                self.push_quad_c(
                    corners,
                    [[0.0; 2]; 4],
                    colors,
                    mode::GRADIENT,
                    [0.0; 4],
                    false,
                );
            }
        } else {
            let corners = Self::quad_for(r);
            let colors = corners.map(|c| g.color_at(g.t_at(c)));
            self.push_quad_c(
                corners,
                [[0.0; 2]; 4],
                colors,
                mode::GRADIENT,
                [0.0; 4],
                false,
            );
        }
    }

    /// A soft shadow of a rounded rectangle: the shape of `rect` moved by `offset`, blurred with
    /// a Gaussian of standard deviation `sigma` pixels. Draw it before the shape it falls behind.
    pub fn shadow(&mut self, rect: Rect, radius: f32, sigma: f32, offset: [f32; 2], color: Color) {
        let r = snap_rect(rect);
        if r.is_empty() || color.a == 0 || sigma <= 0.0 {
            return;
        }
        let r = Rect::from_edges(
            r.x0 + offset[0],
            r.y0 + offset[1],
            r.x1 + offset[0],
            r.y1 + offset[1],
        );
        let (hw, hh) = (r.width() / 2.0, r.height() / 2.0);
        let radius = radius.clamp(0.0, hw.min(hh));
        let (cx, cy) = (r.x0 + hw, r.y0 + hh);
        let e = r.inflate((3.0 * sigma).ceil());
        self.push_quad(
            Self::quad_for(e),
            [
                [e.x0 - cx, e.y0 - cy],
                [e.x1 - cx, e.y0 - cy],
                [e.x1 - cx, e.y1 - cy],
                [e.x0 - cx, e.y1 - cy],
            ],
            color,
            mode::SHADOW,
            [hw, hh, radius, sigma],
            false,
        );
    }

    /// An underline or strike-through from `x0` to `x1` for text of `font` at `size_px` whose
    /// baseline is `baseline`. Position and thickness come from the font; thickness is at least
    /// one whole pixel.
    #[allow(clippy::too_many_arguments)]
    pub fn text_decoration(
        &mut self,
        assets: &Assets,
        font: FontId,
        size_px: f32,
        x0: f32,
        x1: f32,
        baseline: f32,
        kind: Decorations,
        color: Color,
    ) {
        let Some(m) = assets.font_metrics(font, size_px) else {
            return;
        };
        if kind.underline {
            let y = baseline - m.underline_offset;
            self.hline(y, x0, x1, m.underline_thickness.max(1.0), color);
        }
        if kind.strikethrough {
            let y = baseline - m.strike_offset;
            self.hline(y, x0, x1, m.strike_thickness.max(1.0), color);
        }
    }

    /// Draws a laid-out string (all runs, with fallback fonts) and its decorations.
    pub fn text(
        &mut self,
        assets: &mut Assets,
        font: FontId,
        layout: &TextLayout,
        size_px: f32,
        color: Color,
        decor: Decorations,
    ) {
        for run in &layout.runs {
            self.glyph_run(
                assets,
                &GlyphRun {
                    font: run.font,
                    size_px,
                    glyphs: &run.glyphs,
                },
                color,
            );
        }
        if decor != Decorations::default() && layout.width > 0.0 {
            let [x, baseline] = layout.origin;
            self.text_decoration(
                assets,
                font,
                size_px,
                x,
                x + layout.width,
                baseline,
                decor,
                color,
            );
        }
    }

    /// Draws glyphs. Each glyph's x position is split into a whole pixel and a subpixel bin, the
    /// baseline y rounds to a whole pixel, and the bitmap quad lands on whole pixels.
    pub fn glyph_run(&mut self, assets: &mut Assets, run: &GlyphRun<'_>, color: Color) {
        if color.a == 0 {
            return;
        }
        let options = assets.text_options();
        for g in run.glyphs {
            let (whole, bin) = split_subpixel(g.x, options.subpixel_bins);
            let Some(p) = assets.glyph(run.font, g.id, run.size_px, bin) else {
                continue;
            };
            let x0 = (whole + p.left) as f32;
            let y0 = g.y.round() - p.top as f32;
            let (w, h) = (p.slot.width as f32, p.slot.height as f32);
            let (u, v) = (p.slot.x as f32, p.slot.y as f32);
            self.push_quad(
                Self::quad_for(Rect::from_xywh(x0, y0, w, h)),
                [[u, v], [u + w, v], [u + w, v + h], [u, v + h]],
                color,
                if options.lcd {
                    mode::GLYPH_LCD
                } else {
                    mode::GLYPH
                },
                [0.0; 4],
                options.lcd,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color = Color::rgb(255, 0, 0);

    fn bounds(list: &DrawList) -> Rect {
        let mut r = Rect::from_edges(f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for v in list.vertices() {
            r.x0 = r.x0.min(v.pos[0]);
            r.y0 = r.y0.min(v.pos[1]);
            r.x1 = r.x1.max(v.pos[0]);
            r.y1 = r.y1.max(v.pos[1]);
        }
        r
    }

    #[test]
    fn fill_rect_emits_one_snapped_quad() {
        let mut l = DrawList::new(100, 100);
        l.fill_rect(Rect::from_xywh(10.4, 20.6, 30.2, 10.0), RED);
        assert_eq!(l.vertices().len(), 4);
        assert_eq!(l.indices(), &[0, 1, 2, 0, 2, 3]);
        assert_eq!(bounds(&l), Rect::from_edges(10.0, 21.0, 41.0, 31.0));
    }

    #[test]
    fn transparent_and_empty_fills_emit_nothing() {
        let mut l = DrawList::new(10, 10);
        l.fill_rect(Rect::from_xywh(0.0, 0.0, 5.0, 5.0), Color::TRANSPARENT);
        l.fill_rect(Rect::from_xywh(2.0, 2.0, 0.0, 5.0), RED);
        assert!(l.is_empty());
    }

    #[test]
    fn hairline_is_one_full_pixel_row() {
        let mut l = DrawList::new(100, 100);
        l.line([5.0, 10.3], [50.0, 10.3], 1.0, RED);
        assert_eq!(bounds(&l), Rect::from_edges(5.0, 10.0, 50.0, 11.0));
        let mut l = DrawList::new(100, 100);
        l.line([12.7, 5.0], [12.7, 50.0], 1.0, RED);
        assert_eq!(bounds(&l), Rect::from_edges(12.0, 5.0, 13.0, 50.0));
    }

    #[test]
    fn two_pixel_line_sits_on_a_pixel_edge() {
        let mut l = DrawList::new(100, 100);
        l.hline(10.2, 0.0, 20.0, 2.0, RED);
        assert_eq!(bounds(&l), Rect::from_edges(0.0, 9.0, 20.0, 11.0));
    }

    #[test]
    fn diagonal_line_uses_the_antialiased_shape_path() {
        let mut l = DrawList::new(100, 100);
        l.line([0.0, 0.0], [30.0, 40.0], 2.0, RED);
        assert_eq!(l.vertices().len(), 4);
        assert!(l.vertices().iter().all(|v| v.mode == mode::ROUND_RECT));
        assert_eq!(l.vertices()[0].params[0], 25.0);
    }

    #[test]
    fn outline_is_four_inside_rects() {
        let mut l = DrawList::new(100, 100);
        l.rect_outline(Rect::from_xywh(10.0, 10.0, 20.0, 10.0), 1.0, RED);
        assert_eq!(l.vertices().len(), 16);
        assert_eq!(bounds(&l), Rect::from_edges(10.0, 10.0, 30.0, 20.0));
    }

    #[test]
    fn rounded_rect_clamps_radius_and_carries_shape_params() {
        let mut l = DrawList::new(100, 100);
        l.fill_rrect(Rect::from_xywh(10.0, 10.0, 20.0, 10.0), 50.0, RED);
        let v = l.vertices()[0];
        assert_eq!(v.mode, mode::ROUND_RECT);
        assert_eq!(v.params, [10.0, 5.0, 5.0, 0.0]);
        // The quad grows by one pixel for the antialiasing fringe; uv is relative to the centre.
        assert_eq!(v.pos, [9.0, 9.0]);
        assert_eq!(v.uv, [-11.0, -6.0]);
    }

    #[test]
    fn stroke_rrect_records_border_width() {
        let mut l = DrawList::new(100, 100);
        l.stroke_rrect(Rect::from_xywh(0.0, 0.0, 30.0, 30.0), 3.0, 1.2, RED);
        assert_eq!(l.vertices()[0].params[3], 1.0);
    }

    #[test]
    fn clip_stack_intersects_and_restores() {
        let mut l = DrawList::new(100, 100);
        l.push_clip(Rect::from_xywh(10.0, 10.0, 50.0, 50.0));
        l.push_clip(Rect::from_xywh(40.0, 0.0, 100.0, 30.0));
        assert_eq!(l.clip(), PixelRect::new(40, 10, 60, 30));
        l.pop_clip();
        assert_eq!(l.clip(), PixelRect::new(10, 10, 60, 60));
        l.pop_clip();
        l.pop_clip();
        assert_eq!(l.clip(), PixelRect::new(0, 0, 100, 100));
        assert_eq!(l.clip_depth(), 0);
    }

    #[test]
    fn negative_clip_edges_clamp_to_the_target() {
        let mut l = DrawList::new(50, 50);
        l.push_clip(Rect::from_edges(-20.0, -5.0, 10.0, 200.0));
        assert_eq!(l.clip(), PixelRect::new(0, 0, 10, 50));
    }

    #[test]
    fn shapes_outside_the_clip_are_culled() {
        let mut l = DrawList::new(100, 100);
        l.push_clip(Rect::from_xywh(0.0, 0.0, 10.0, 10.0));
        l.fill_rect(Rect::from_xywh(20.0, 20.0, 5.0, 5.0), RED);
        assert!(l.is_empty());
        l.fill_rect(Rect::from_xywh(5.0, 5.0, 20.0, 20.0), RED);
        assert_eq!(l.batches().len(), 1);
    }

    #[test]
    fn batches_split_on_clip_change_and_merge_otherwise() {
        let mut l = DrawList::new(100, 100);
        l.fill_rect(Rect::from_xywh(0.0, 0.0, 5.0, 5.0), RED);
        l.fill_rect(Rect::from_xywh(10.0, 0.0, 5.0, 5.0), RED);
        l.push_clip(Rect::from_xywh(0.0, 0.0, 50.0, 50.0));
        l.fill_rect(Rect::from_xywh(20.0, 0.0, 5.0, 5.0), RED);
        l.pop_clip();
        l.fill_rect(Rect::from_xywh(30.0, 0.0, 5.0, 5.0), RED);
        let b = l.batches();
        assert_eq!(b.len(), 3);
        assert_eq!((b[0].first_index, b[0].index_count), (0, 12));
        assert_eq!((b[1].first_index, b[1].index_count), (12, 6));
        assert_eq!((b[2].first_index, b[2].index_count), (18, 6));
    }

    #[test]
    fn reset_clears_everything() {
        let mut l = DrawList::new(10, 10);
        l.push_clip(Rect::from_xywh(0.0, 0.0, 5.0, 5.0));
        l.fill_rect(Rect::from_xywh(0.0, 0.0, 5.0, 5.0), RED);
        l.reset(20, 20);
        assert!(l.is_empty());
        assert_eq!(l.clip(), PixelRect::new(0, 0, 20, 20));
    }

    #[test]
    fn vertex_layout_is_tightly_packed() {
        assert_eq!(std::mem::size_of::<Vertex>(), 40);
    }
}
