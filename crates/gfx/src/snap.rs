//! Device-pixel snapping. Office draws chrome, gridlines and borders on whole device pixels;
//! everything that must match it exactly goes through these functions.

use crate::geom::Rect;

/// Rounds a coordinate to the nearest pixel edge (halves round away from zero).
pub fn snap_edge(v: f32) -> f32 {
    v.round()
}

/// Snaps every edge of `r` to the pixel grid. A non-empty rectangle never collapses: if both
/// edges round to the same line the far edge moves out by one pixel, so a 0.4 px rule still
/// paints a 1 px line like Office does.
pub fn snap_rect(r: Rect) -> Rect {
    let (x0, y0) = (snap_edge(r.x0), snap_edge(r.y0));
    let (mut x1, mut y1) = (snap_edge(r.x1), snap_edge(r.y1));
    if r.x1 > r.x0 && x1 <= x0 {
        x1 = x0 + 1.0;
    }
    if r.y1 > r.y0 && y1 <= y0 {
        y1 = y0 + 1.0;
    }
    Rect::from_edges(x0, y0, x1, y1)
}

/// The whole-pixel width a stroke of `width` device pixels is drawn with (at least one).
pub fn stroke_pixels(width: f32) -> u32 {
    if width.is_nan() {
        return 1;
    }
    (width.round().max(1.0)) as u32
}

/// Snaps a stroke centred on `center` to whole pixels and returns `(start, end)` edges.
/// An odd pixel width is centred on the pixel containing `center`; an even width is centred on
/// the nearest pixel edge, so edges always land on pixel boundaries and the stroke stays crisp.
pub fn snap_stroke(center: f32, width: f32) -> (f32, f32) {
    let w = stroke_pixels(width);
    let start = if w % 2 == 1 {
        center.floor() - ((w - 1) / 2) as f32
    } else {
        center.round() - (w / 2) as f32
    };
    (start, start + w as f32)
}

/// Splits a position into the integer pixel and a subpixel bin in `0..bins`, rounding to the
/// nearest bin. Used for glyph x positions: the glyph is rasterised at the bin offset and the quad
/// is placed on the integer pixel.
pub fn split_subpixel(x: f32, bins: u8) -> (i32, u8) {
    let bins = bins.max(1);
    let floor = x.floor();
    let frac = x - floor;
    let mut bin = (frac * f32::from(bins)).round() as i32;
    let mut whole = floor as i32;
    if bin >= i32::from(bins) {
        bin = 0;
        whole += 1;
    }
    (whole, bin as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_round_to_nearest_pixel() {
        assert_eq!(snap_edge(10.49), 10.0);
        assert_eq!(snap_edge(10.5), 11.0);
        assert_eq!(snap_edge(-0.4), 0.0);
    }

    #[test]
    fn thin_rect_keeps_one_pixel() {
        let r = snap_rect(Rect::from_xywh(10.2, 5.0, 0.4, 20.0));
        assert_eq!((r.x0, r.x1), (10.0, 11.0));
        assert_eq!((r.y0, r.y1), (5.0, 25.0));
    }

    #[test]
    fn empty_rect_stays_empty() {
        let r = snap_rect(Rect::from_xywh(10.2, 5.0, 0.0, 20.0));
        assert!(r.is_empty());
    }

    #[test]
    fn one_pixel_stroke_covers_the_pixel_under_its_centre() {
        assert_eq!(snap_stroke(10.0, 1.0), (10.0, 11.0));
        assert_eq!(snap_stroke(10.5, 1.0), (10.0, 11.0));
        assert_eq!(snap_stroke(10.99, 1.0), (10.0, 11.0));
    }

    #[test]
    fn even_stroke_centres_on_an_edge() {
        assert_eq!(snap_stroke(10.4, 2.0), (9.0, 11.0));
        assert_eq!(snap_stroke(10.6, 2.0), (10.0, 12.0));
    }

    #[test]
    fn odd_wide_stroke_is_centred_on_a_pixel() {
        assert_eq!(snap_stroke(10.3, 3.0), (9.0, 12.0));
    }

    #[test]
    fn sub_pixel_widths_round_up_to_one() {
        assert_eq!(stroke_pixels(0.2), 1);
        assert_eq!(stroke_pixels(1.4), 1);
        assert_eq!(stroke_pixels(1.5), 2);
        assert_eq!(stroke_pixels(f32::NAN), 1);
    }

    #[test]
    fn subpixel_bins_round_and_carry() {
        assert_eq!(split_subpixel(10.0, 4), (10, 0));
        assert_eq!(split_subpixel(10.3, 4), (10, 1));
        assert_eq!(split_subpixel(10.62, 4), (10, 2));
        assert_eq!(split_subpixel(10.9, 4), (11, 0));
        assert_eq!(split_subpixel(-0.2, 4), (-1, 3));
        assert_eq!(split_subpixel(5.4, 3), (5, 1));
    }
}
