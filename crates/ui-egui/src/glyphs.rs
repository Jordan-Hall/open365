//! Glyph rasterisation for the document canvas.
//!
//! Layout positions glyphs by glyph id and font file, so the canvas draws exactly those glyphs
//! from exactly those files: outlines are read with skrifa, flattened and filled with an exact
//! area-coverage scanline rasteriser, and packed into shared alpha atlases (egui textures).
//! egui's own text path would re-shape the text with its own fonts and drift from layout.

use std::collections::HashMap;
use std::sync::Arc;

use egui::epaint::Mesh;
use egui::{Color32, ColorImage, Context, Pos2, Rect, TextureHandle, TextureId, TextureOptions};
use open365_text::{FaceId, FontLibrary, FontStyle};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, GlyphId, MetadataProvider};

const ATLAS_SIZE: usize = 1024;
const MAX_PAGES: usize = 6;
/// Fixed-point steps per pixel of the cached glyph size.
const SIZE_STEPS: f32 = 8.0;

/// The font files of a [`FontLibrary`], kept so outlines can be read from the same bytes
/// layout measured with.
pub struct FontSet {
    pub library: FontLibrary,
    data: Vec<Arc<[u8]>>,
}

macro_rules! bundled {
    ($file:literal) => {
        include_bytes!(concat!("../../../assets/fonts/", $file)) as &'static [u8]
    };
}

/// (family, bold, italic, bytes): the same files `FontLibrary::bundled` loads.
const BUNDLED: [(&str, bool, bool, &[u8]); 8] = [
    ("Carlito", false, false, bundled!("Carlito-Regular.ttf")),
    ("Carlito", true, false, bundled!("Carlito-Bold.ttf")),
    ("Carlito", false, true, bundled!("Carlito-Italic.ttf")),
    ("Carlito", true, true, bundled!("Carlito-BoldItalic.ttf")),
    ("Caladea", false, false, bundled!("Caladea-Regular.ttf")),
    ("Caladea", true, false, bundled!("Caladea-Bold.ttf")),
    ("Caladea", false, true, bundled!("Caladea-Italic.ttf")),
    ("Caladea", true, true, bundled!("Caladea-BoldItalic.ttf")),
];

impl std::fmt::Debug for FontSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontSet")
            .field("faces", &self.data.len())
            .finish()
    }
}

impl FontSet {
    /// The deterministic bundled set (Carlito, Caladea; Calibri and Cambria alias to them).
    pub fn bundled() -> Self {
        let mut library = FontLibrary::empty("Carlito");
        let mut data = Vec::new();
        for (family, bold, italic, bytes) in BUNDLED {
            let bytes: Arc<[u8]> = Arc::from(bytes);
            if library
                .add_bytes(Arc::clone(&bytes), 0, family, bold, italic)
                .is_ok()
            {
                data.push(bytes);
            }
        }
        library.add_alias("Calibri", "Carlito");
        library.add_alias("Cambria", "Caladea");
        Self { library, data }
    }

    fn font(&self, face: FaceId) -> Option<FontRef<'_>> {
        FontRef::from_index(self.data.get(face.index())?, 0).ok()
    }

    /// The face a family name resolves to, and its family name (for status messages).
    pub fn resolve(&self, family: &str, bold: bool, italic: bool) -> Option<FaceId> {
        self.library.resolve(family, FontStyle { bold, italic })
    }

    /// The glyph the face's cmap gives `c`, 0 when it has none.
    pub fn nominal_glyph(&self, face: FaceId, c: char) -> u32 {
        self.font(face)
            .and_then(|f| f.charmap().map(c))
            .map_or(0, |g| g.to_u32())
    }
}

/// Where a glyph's bitmap sits in an atlas page and relative to the glyph origin.
#[derive(Debug, Clone, Copy)]
struct Slot {
    page: usize,
    /// UV rectangle in the page.
    uv: Rect,
    /// Bitmap rectangle relative to the glyph origin, pixels, y down.
    offset: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    face: usize,
    glyph: u32,
    size: u32,
}

struct Page {
    texture: TextureHandle,
    cursor_x: usize,
    cursor_y: usize,
    row_h: usize,
}

/// Shared glyph atlas.
#[derive(Default)]
pub struct Atlas {
    pages: Vec<Page>,
    slots: HashMap<Key, Option<Slot>>,
    reset: bool,
}

impl std::fmt::Debug for Atlas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Atlas")
            .field("pages", &self.pages.len())
            .field("glyphs", &self.slots.len())
            .finish()
    }
}

/// Quads for one frame, one mesh per atlas page.
#[derive(Default)]
pub struct GlyphBatch {
    meshes: Vec<Option<Mesh>>,
}

impl GlyphBatch {
    pub fn into_meshes(self) -> impl Iterator<Item = Mesh> {
        self.meshes.into_iter().flatten()
    }
}

impl Atlas {
    /// Drops everything if a previous frame ran out of room. Call at the start of a frame.
    pub fn begin_frame(&mut self) {
        if std::mem::take(&mut self.reset) {
            self.pages.clear();
            self.slots.clear();
        }
    }

    /// Adds a quad for glyph `glyph` of `face` at pixel size `px`, origin `at` (physical pixels
    /// divided by `ppp`, i.e. in egui points), to `batch`.
    #[allow(clippy::too_many_arguments)]
    pub fn push(
        &mut self,
        ctx: &Context,
        fonts: &FontSet,
        batch: &mut GlyphBatch,
        face: FaceId,
        glyph: u32,
        px: f32,
        at: Pos2,
        ppp: f32,
        color: Color32,
    ) {
        let key = Key {
            face: face.index(),
            glyph,
            size: (px * SIZE_STEPS).round().max(1.0) as u32,
        };
        let slot = match self.slots.get(&key) {
            Some(s) => *s,
            None => {
                let s = self.rasterise(ctx, fonts, key);
                self.slots.insert(key, s);
                s
            }
        };
        let Some(slot) = slot else { return };
        let Some(page) = self.pages.get(slot.page) else {
            return;
        };
        // Snap the origin to a physical pixel so the bitmap is not resampled.
        let ox = (at.x * ppp).round() / ppp;
        let oy = (at.y * ppp).round() / ppp;
        let rect = Rect::from_min_max(
            Pos2::new(ox + slot.offset.min.x / ppp, oy + slot.offset.min.y / ppp),
            Pos2::new(ox + slot.offset.max.x / ppp, oy + slot.offset.max.y / ppp),
        );
        if batch.meshes.len() <= slot.page {
            batch.meshes.resize_with(slot.page + 1, || None);
        }
        let mesh = batch.meshes[slot.page]
            .get_or_insert_with(|| Mesh::with_texture(texture_id(&page.texture)));
        mesh.add_rect_with_uv(rect, slot.uv, color);
    }

    fn rasterise(&mut self, ctx: &Context, fonts: &FontSet, key: Key) -> Option<Slot> {
        let font = fonts.font(FaceId::from_index(key.face))?;
        let px = key.size as f32 / SIZE_STEPS;
        let outline = font.outline_glyphs().get(GlyphId::new(key.glyph))?;
        let mut pen = Collector::default();
        outline
            .draw(
                DrawSettings::unhinted(Size::new(px), LocationRef::default()),
                &mut pen,
            )
            .ok()?;
        let bitmap = pen.fill()?;
        let (w, h) = (bitmap.width, bitmap.height);
        if w + 2 > ATLAS_SIZE || h + 2 > ATLAS_SIZE {
            return None;
        }
        // Find room (shelf packing with a pixel of padding).
        let (page_index, x, y) = self.allocate(ctx, w + 1, h + 1)?;
        let pixels: Vec<Color32> = bitmap
            .coverage
            .iter()
            .map(|c| Color32::from_white_alpha(*c))
            .collect();
        let image = ColorImage::new([w, h], pixels);
        self.pages[page_index]
            .texture
            .set_partial([x, y], image, TextureOptions::LINEAR);
        let s = ATLAS_SIZE as f32;
        Some(Slot {
            page: page_index,
            uv: Rect::from_min_max(
                Pos2::new(x as f32 / s, y as f32 / s),
                Pos2::new((x + w) as f32 / s, (y + h) as f32 / s),
            ),
            offset: Rect::from_min_size(
                Pos2::new(bitmap.left as f32, bitmap.top as f32),
                egui::vec2(w as f32, h as f32),
            ),
        })
    }

    fn allocate(&mut self, ctx: &Context, w: usize, h: usize) -> Option<(usize, usize, usize)> {
        let last = self.pages.len().wrapping_sub(1);
        if let Some(page) = self.pages.last_mut() {
            if page.cursor_x + w > ATLAS_SIZE {
                page.cursor_x = 0;
                page.cursor_y += page.row_h;
                page.row_h = 0;
            }
            if page.cursor_y + h <= ATLAS_SIZE && page.cursor_x + w <= ATLAS_SIZE {
                let at = (last, page.cursor_x, page.cursor_y);
                page.cursor_x += w;
                page.row_h = page.row_h.max(h);
                return Some(at);
            }
        }
        if self.pages.len() >= MAX_PAGES {
            // Out of room: draw nothing new this frame and start over at the next.
            self.reset = true;
            return None;
        }
        let blank = ColorImage::filled([ATLAS_SIZE, ATLAS_SIZE], Color32::TRANSPARENT);
        let texture = ctx.load_texture(
            format!("open365-glyph-atlas-{}", self.pages.len()),
            blank,
            TextureOptions::LINEAR,
        );
        self.pages.push(Page {
            texture,
            cursor_x: w,
            cursor_y: 0,
            row_h: h,
        });
        Some((self.pages.len() - 1, 0, 0))
    }
}

fn texture_id(t: &TextureHandle) -> TextureId {
    t.id()
}

struct Bitmap {
    width: usize,
    height: usize,
    /// Pixels left of the origin the bitmap starts.
    left: i32,
    /// Pixels the bitmap's top lies above the baseline, as a y-down offset (negative = above).
    top: i32,
    coverage: Vec<u8>,
}

/// Collects an outline as flattened line segments (y up, pixels).
#[derive(Default)]
struct Collector {
    lines: Vec<([f32; 2], [f32; 2])>,
    start: [f32; 2],
    at: [f32; 2],
}

impl Collector {
    fn line(&mut self, to: [f32; 2]) {
        self.lines.push((self.at, to));
        self.at = to;
    }

    fn steps(len: f32) -> usize {
        ((len / 1.5).ceil() as usize).clamp(2, 32)
    }

    fn fill(self) -> Option<Bitmap> {
        if self.lines.is_empty() {
            return None;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (a, b) in &self.lines {
            for p in [a, b] {
                x0 = x0.min(p[0]);
                x1 = x1.max(p[0]);
                y0 = y0.min(p[1]);
                y1 = y1.max(p[1]);
            }
        }
        let left = x0.floor() as i32 - 1;
        let bottom = y0.floor() as i32 - 1;
        let top = y1.ceil() as i32 + 1;
        let width = ((x1.ceil() as i32 + 1) - left).max(1) as usize;
        let height = (top - bottom).max(1) as usize;
        // Accumulation buffer with a spare column for the area spill.
        let stride = width + 2;
        let mut acc = vec![0.0f32; stride * height];
        for (a, b) in &self.lines {
            // Raster space: x right, y down from the top row.
            let p0 = [a[0] - left as f32, top as f32 - a[1]];
            let p1 = [b[0] - left as f32, top as f32 - b[1]];
            accumulate(&mut acc, stride, height, p0, p1);
        }
        let mut coverage = Vec::with_capacity(width * height);
        for row in acc.chunks(stride) {
            let mut sum = 0.0f32;
            for v in &row[..width] {
                sum += *v;
                coverage.push((sum.abs().min(1.0) * 255.0 + 0.5) as u8);
            }
        }
        Some(Bitmap {
            width,
            height,
            left,
            top: -top,
            coverage,
        })
    }
}

impl OutlinePen for Collector {
    fn move_to(&mut self, x: f32, y: f32) {
        self.start = [x, y];
        self.at = [x, y];
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.line([x, y]);
    }

    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let p0 = self.at;
        let len = dist(p0, [cx, cy]) + dist([cx, cy], [x, y]);
        let n = Self::steps(len);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            self.line([
                u * u * p0[0] + 2.0 * u * t * cx + t * t * x,
                u * u * p0[1] + 2.0 * u * t * cy + t * t * y,
            ]);
        }
    }

    fn curve_to(&mut self, c0x: f32, c0y: f32, c1x: f32, c1y: f32, x: f32, y: f32) {
        let p0 = self.at;
        let len = dist(p0, [c0x, c0y]) + dist([c0x, c0y], [c1x, c1y]) + dist([c1x, c1y], [x, y]);
        let n = Self::steps(len);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            self.line([
                u * u * u * p0[0] + 3.0 * u * u * t * c0x + 3.0 * u * t * t * c1x + t * t * t * x,
                u * u * u * p0[1] + 3.0 * u * u * t * c0y + 3.0 * u * t * t * c1y + t * t * t * y,
            ]);
        }
    }

    fn close(&mut self) {
        if self.at != self.start {
            self.line(self.start);
        }
    }
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Adds the signed area a segment covers to the accumulation buffer (the font-rs method).
fn accumulate(acc: &mut [f32], stride: usize, height: usize, p0: [f32; 2], p1: [f32; 2]) {
    if (p0[1] - p1[1]).abs() < f32::EPSILON {
        return;
    }
    let (dir, p0, p1) = if p0[1] < p1[1] {
        (1.0, p0, p1)
    } else {
        (-1.0, p1, p0)
    };
    let dxdy = (p1[0] - p0[0]) / (p1[1] - p0[1]);
    let mut x = p0[0];
    let mut y_start = p0[1].max(0.0);
    if p0[1] < 0.0 {
        x -= p0[1] * dxdy;
    }
    y_start = y_start.min(height as f32);
    let last = (p1[1].ceil().max(0.0) as usize).min(height);
    for y in (y_start as usize)..last {
        let line = y * stride;
        let dy = ((y + 1) as f32).min(p1[1]) - (y as f32).max(p0[1]);
        let xnext = x + dxdy * dy;
        let d = dy * dir;
        let (xa, xb) = if x < xnext { (x, xnext) } else { (xnext, x) };
        let xa_floor = xa.floor();
        let xai = xa_floor as i64;
        let xb_ceil = xb.ceil();
        let xbi = xb_ceil as i64;
        let put = |acc: &mut [f32], xi: i64, v: f32| {
            if xi >= 0 && (xi as usize) < stride {
                acc[line + xi as usize] += v;
            }
        };
        if xbi <= xai + 1 {
            let xmf = 0.5 * (x + xnext) - xa_floor;
            put(acc, xai, d - d * xmf);
            put(acc, xai + 1, d * xmf);
        } else {
            let s = 1.0 / (xb - xa);
            let xaf = xa - xa_floor;
            let a0 = 0.5 * s * (1.0 - xaf) * (1.0 - xaf);
            let xbf = xb - xb_ceil + 1.0;
            let am = 0.5 * s * xbf * xbf;
            put(acc, xai, d * a0);
            if xbi == xai + 2 {
                put(acc, xai + 1, d * (1.0 - a0 - am));
            } else {
                let a1 = s * (1.5 - xaf);
                put(acc, xai + 1, d * (a1 - a0));
                for xi in (xai + 2)..(xbi - 1) {
                    put(acc, xi, d * s);
                }
                let a2 = a1 + (xbi - xai - 3) as f32 * s;
                put(acc, xbi - 1, d * (1.0 - a2 - am));
            }
            put(acc, xbi, d * am);
        }
        x = xnext;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filled_square_has_full_coverage_inside() {
        let mut pen = Collector::default();
        pen.move_to(2.0, 2.0);
        pen.line_to(8.0, 2.0);
        pen.line_to(8.0, 8.0);
        pen.line_to(2.0, 8.0);
        pen.close();
        let bm = pen.fill().expect("bitmap");
        let total: f32 = bm.coverage.iter().map(|c| f32::from(*c) / 255.0).sum();
        assert!((total - 36.0).abs() < 0.5, "area {total}");
    }

    #[test]
    fn bundled_glyph_has_ink() {
        let fonts = FontSet::bundled();
        let face = fonts.resolve("Calibri", false, false).expect("face");
        let g = fonts.nominal_glyph(face, 'H');
        assert_ne!(g, 0);
        let font = fonts.font(face).expect("font");
        let mut pen = Collector::default();
        font.outline_glyphs()
            .get(GlyphId::new(g))
            .expect("outline")
            .draw(
                DrawSettings::unhinted(Size::new(24.0), LocationRef::default()),
                &mut pen,
            )
            .expect("draw");
        let bm = pen.fill().expect("bitmap");
        assert!(bm.coverage.iter().filter(|c| **c > 128).count() > 20);
    }
}
