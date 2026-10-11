//! Exact-area outline rasteriser. Contours are flattened to line segments, then signed
//! trapezoid areas are accumulated per pixel (the same idea as FreeType's smooth rasteriser and
//! font-rs). Coverage is analytic, not sampled, so a stem that lands on a pixel boundary gets
//! exactly 100% coverage, which hinted text depends on.

use skrifa::outline::OutlinePen;

/// Maximum distance, in pixels, between a curve and its flattened form.
const FLATNESS: f32 = 0.02;

/// Collects an outline (y up, pixel units) as line segments `[x0, y0, x1, y1]`.
#[derive(Debug, Default)]
pub struct Flattener {
    pub lines: Vec<[f32; 4]>,
    start: (f32, f32),
    cur: (f32, f32),
    open: bool,
}

impl Flattener {
    fn line(&mut self, x: f32, y: f32) {
        self.lines.push([self.cur.0, self.cur.1, x, y]);
        self.cur = (x, y);
    }

    fn close_contour(&mut self) {
        if self.open && self.cur != self.start {
            self.line(self.start.0, self.start.1);
        }
        self.cur = self.start;
        self.open = false;
    }
}

impl OutlinePen for Flattener {
    fn move_to(&mut self, x: f32, y: f32) {
        self.close_contour();
        self.start = (x, y);
        self.cur = (x, y);
        self.open = true;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.line(x, y);
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let (px, py) = self.cur;
        let ddx = px - 2.0 * cx0 + x;
        let ddy = py - 2.0 * cy0 + y;
        let dd = (ddx * ddx + ddy * ddy).sqrt();
        let n = ((dd / (4.0 * FLATNESS)).sqrt().ceil() as u32).clamp(1, 64);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let mt = 1.0 - t;
            let qx = mt * mt * px + 2.0 * mt * t * cx0 + t * t * x;
            let qy = mt * mt * py + 2.0 * mt * t * cy0 + t * t * y;
            self.line(qx, qy);
        }
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (px, py) = self.cur;
        let d1 = ((px - 2.0 * cx0 + cx1).powi(2) + (py - 2.0 * cy0 + cy1).powi(2)).sqrt();
        let d2 = ((cx0 - 2.0 * cx1 + x).powi(2) + (cy0 - 2.0 * cy1 + y).powi(2)).sqrt();
        let dd = d1.max(d2);
        let n = ((0.75 * dd / FLATNESS).sqrt().ceil() as u32).clamp(1, 64);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let mt = 1.0 - t;
            let a = mt * mt * mt;
            let b = 3.0 * mt * mt * t;
            let c = 3.0 * mt * t * t;
            let d = t * t * t;
            self.line(
                a * px + b * cx0 + c * cx1 + d * x,
                a * py + b * cy0 + c * cy1 + d * y,
            );
        }
    }

    fn close(&mut self) {
        self.close_contour();
    }
}

impl Flattener {
    /// Closes any open contour and returns the segments.
    pub fn finish(mut self) -> Vec<[f32; 4]> {
        self.close_contour();
        self.lines
    }
}

/// How a glyph bitmap is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RasterMode {
    /// One coverage byte per pixel.
    Gray,
    /// Three coverage bytes per pixel (R, G, B subpixels, left to right), FIR filtered.
    Lcd,
}

/// A rasterised glyph. `left` and `top` place the bitmap relative to the glyph origin:
/// column 0 is at `origin.x + left`, row 0 is at `origin.y - top` (y down).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    /// `width * height` bytes for [`RasterMode::Gray`], `width * height * 3` for Lcd.
    pub data: Vec<u8>,
}

/// The FIR filter applied across subpixels of an LCD glyph (taps sum to 256).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LcdFilter {
    /// FreeType's default 5-tap filter.
    #[default]
    Default,
    /// FreeType's light filter (3 equal taps): sharper, more colour fringe.
    Light,
    /// Wider and softer than the default.
    Soft,
    /// No filtering: each subpixel keeps its own coverage (strong colour fringes).
    None,
}

impl LcdFilter {
    /// The five taps, centre at index 2.
    pub const fn taps(self) -> [u32; 5] {
        match self {
            LcdFilter::Default => [0x08, 0x4D, 0x56, 0x4D, 0x08],
            LcdFilter::Light => [0x00, 0x55, 0x56, 0x55, 0x00],
            LcdFilter::Soft => [0x30, 0x48, 0x48, 0x48, 0x30],
            LcdFilter::None => [0x00, 0x00, 0x100, 0x00, 0x00],
        }
    }
}

/// Adds the signed area of one segment to the accumulation buffer.
fn accumulate(acc: &mut [f32], stride: usize, w: usize, h: usize, seg: [f32; 4]) {
    let [mut ax, mut ay, mut bx, mut by] = seg;
    if ay == by {
        return;
    }
    let dir = if ay < by {
        1.0
    } else {
        std::mem::swap(&mut ax, &mut bx);
        std::mem::swap(&mut ay, &mut by);
        -1.0
    };
    let dxdy = (bx - ax) / (by - ay);
    let mut x = ax;
    let mut y_start = ay;
    if y_start < 0.0 {
        x -= y_start * dxdy;
        y_start = 0.0;
    }
    let y_end = by.min(h as f32);
    if y_start >= y_end {
        return;
    }
    let max_x = w as f32;
    for y in (y_start as usize)..(y_end.ceil() as usize) {
        let row = y * stride;
        let dy = ((y + 1) as f32).min(y_end) - (y as f32).max(y_start);
        let xnext = x + dxdy * dy;
        let d = dy * dir;
        let (x0, x1) = if x < xnext { (x, xnext) } else { (xnext, x) };
        let x0 = x0.clamp(0.0, max_x);
        let x1 = x1.clamp(0.0, max_x);
        let x0floor = x0.floor();
        let x0i = x0floor as usize;
        let x1ceil = x1.ceil();
        let x1i = x1ceil as usize;
        if x1i <= x0i + 1 {
            let xmf = 0.5 * (x0 + x1) - x0floor;
            acc[row + x0i] += d - d * xmf;
            acc[row + x0i + 1] += d * xmf;
        } else {
            let s = 1.0 / (x1 - x0);
            let x0f = x0 - x0floor;
            let a0 = 0.5 * s * (1.0 - x0f) * (1.0 - x0f);
            let x1f = x1 - x1ceil + 1.0;
            let am = 0.5 * s * x1f * x1f;
            acc[row + x0i] += d * a0;
            if x1i == x0i + 2 {
                acc[row + x0i + 1] += d * (1.0 - a0 - am);
            } else {
                let a1 = s * (1.5 - x0f);
                acc[row + x0i + 1] += d * (a1 - a0);
                for xi in (x0i + 2)..(x1i - 1) {
                    acc[row + xi] += d * s;
                }
                let a2 = a1 + (x1i - x0i - 3) as f32 * s;
                acc[row + x1i - 1] += d * (1.0 - a2 - am);
            }
            acc[row + x1i] += d * am;
        }
        x = xnext;
    }
}

/// Rasterises `lines` (y up) after shifting them right by `x_offset` pixels. Returns `None` for
/// an empty outline such as a space.
pub fn rasterize(lines: &[[f32; 4]], x_offset: f32, mode: RasterMode) -> Option<Bitmap> {
    rasterize_with(lines, x_offset, mode, LcdFilter::Default)
}

/// [`rasterize`] with a chosen LCD filter (ignored for grayscale).
pub fn rasterize_with(
    lines: &[[f32; 4]],
    x_offset: f32,
    mode: RasterMode,
    filter: LcdFilter,
) -> Option<Bitmap> {
    let fir = filter.taps();
    if lines.is_empty() {
        return None;
    }
    let (mut min_x, mut max_x) = (f32::MAX, f32::MIN);
    let (mut min_y, mut max_y) = (f32::MAX, f32::MIN);
    for l in lines {
        for (x, y) in [(l[0] + x_offset, l[1]), (l[2] + x_offset, l[3])] {
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
    }
    if !(min_x.is_finite() && max_x.is_finite() && min_y.is_finite() && max_y.is_finite()) {
        return None;
    }
    // LCD filtering spreads coverage two subpixels either way, so give it a pixel of margin.
    let pad = i32::from(mode == RasterMode::Lcd);
    let left = min_x.floor() as i32 - pad;
    let right = max_x.ceil() as i32 + pad;
    let top = max_y.ceil() as i32;
    let bottom = min_y.floor() as i32;
    let (w, h) = ((right - left) as usize, (top - bottom) as usize);
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return None;
    }
    let xs = if mode == RasterMode::Lcd { 3.0 } else { 1.0 };
    let ow = w * xs as usize;
    let stride = ow + 2;
    let mut acc = vec![0.0f32; stride * h + 2];
    for l in lines {
        let seg = [
            (l[0] + x_offset - left as f32) * xs,
            top as f32 - l[1],
            (l[2] + x_offset - left as f32) * xs,
            top as f32 - l[3],
        ];
        accumulate(&mut acc, stride, ow, h, seg);
    }
    let mut data = Vec::with_capacity(w * h * if xs == 3.0 { 3 } else { 1 });
    let mut cover = vec![0.0f32; ow];
    for y in 0..h {
        let mut sum = 0.0f32;
        for (x, c) in cover.iter_mut().enumerate() {
            sum += acc[y * stride + x];
            *c = sum.abs().min(1.0);
        }
        match mode {
            RasterMode::Gray => {
                data.extend(cover.iter().map(|c| (c * 255.0).round() as u8));
            }
            RasterMode::Lcd => {
                for px in 0..w {
                    for ch in 0..3usize {
                        let centre = (px * 3 + ch) as isize;
                        let mut v = 0.0f32;
                        for (k, wt) in fir.iter().enumerate() {
                            let i = centre + k as isize - 2;
                            if i >= 0 && (i as usize) < ow {
                                v += cover[i as usize] * *wt as f32;
                            }
                        }
                        data.push((v / 256.0 * 255.0).round().min(255.0) as u8);
                    }
                }
            }
        }
    }
    Some(Bitmap {
        width: w as u32,
        height: h as u32,
        left,
        top,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<[f32; 4]> {
        vec![
            [x0, y0, x1, y0],
            [x1, y0, x1, y1],
            [x1, y1, x0, y1],
            [x0, y1, x0, y0],
        ]
    }

    #[test]
    fn half_pixel_edges_give_half_coverage() {
        let b = rasterize(&rect(0.5, 0.0, 2.5, 2.0), 0.0, RasterMode::Gray).expect("bitmap");
        assert_eq!((b.width, b.height, b.left, b.top), (3, 2, 0, 2));
        assert_eq!(b.data, vec![128, 255, 128, 128, 255, 128]);
    }

    #[test]
    fn pixel_aligned_stem_is_fully_covered_with_no_fringe() {
        let b = rasterize(&rect(1.0, 0.0, 3.0, 4.0), 0.0, RasterMode::Gray).expect("bitmap");
        assert_eq!((b.left, b.width), (1, 2));
        assert!(b.data.iter().all(|&v| v == 255));
    }

    #[test]
    fn winding_direction_does_not_change_coverage() {
        let fwd = rect(0.25, 0.0, 1.75, 1.0);
        let rev: Vec<[f32; 4]> = fwd.iter().rev().map(|l| [l[2], l[3], l[0], l[1]]).collect();
        assert_eq!(
            rasterize(&fwd, 0.0, RasterMode::Gray),
            rasterize(&rev, 0.0, RasterMode::Gray)
        );
    }

    #[test]
    fn triangle_coverage_sums_to_its_area() {
        let tri = vec![
            [0.0, 0.0, 8.0, 0.0],
            [8.0, 0.0, 0.0, 8.0],
            [0.0, 8.0, 0.0, 0.0],
        ];
        let b = rasterize(&tri, 0.0, RasterMode::Gray).expect("bitmap");
        let sum: f32 = b.data.iter().map(|&v| f32::from(v) / 255.0).sum();
        assert!((sum - 32.0).abs() < 0.3, "{sum}");
    }

    #[test]
    fn subpixel_offset_moves_coverage() {
        let b = rasterize(&rect(0.0, 0.0, 1.0, 1.0), 0.25, RasterMode::Gray).expect("bitmap");
        assert_eq!(b.data, vec![191, 64]);
    }

    #[test]
    fn lcd_interior_is_solid_and_edges_are_filtered() {
        let b = rasterize(&rect(2.0, 0.0, 8.0, 1.0), 0.0, RasterMode::Lcd).expect("bitmap");
        assert_eq!(b.left, 1);
        assert_eq!(b.data.len(), (b.width * 3) as usize);
        // Pixel 4 (index 3 in the bitmap) is deep inside the stem.
        assert_eq!(&b.data[9..12], &[255, 255, 255]);
        // The left margin pixel only sees the filter tail, the first stem pixel is a ramp.
        let first = &b.data[3..6];
        assert!(first[0] < first[1] && first[1] < first[2], "{first:?}");
    }

    #[test]
    fn empty_outline_has_no_bitmap() {
        assert!(rasterize(&[], 0.0, RasterMode::Gray).is_none());
    }

    #[test]
    fn curves_flatten_within_tolerance() {
        let mut f = Flattener::default();
        f.move_to(0.0, 0.0);
        f.quad_to(5.0, 10.0, 10.0, 0.0);
        f.close();
        let lines = f.finish();
        assert!(lines.len() >= 8);
        // The apex of the quadratic is (5, 5); the flattened curve must come within tolerance.
        let top = lines.iter().map(|l| l[1].max(l[3])).fold(0.0, f32::max);
        assert!((top - 5.0).abs() < 0.05, "{top}");
    }
}
