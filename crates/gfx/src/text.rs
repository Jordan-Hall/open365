//! Text helpers on [`Assets`]: font fallback, metrics, advances, glyph-by-glyph layout and
//! path masks. Shaping with kerning and ligatures stays in `open365-text`; this layer covers UI
//! strings where DirectWrite-style natural advances are enough.

use skrifa::instance::{LocationRef, Size};
use skrifa::{FontRef, GlyphId, MetadataProvider};

use crate::assets::{Assets, FontId, GlyphPos, Placed};
use crate::path::Path;
use crate::raster::{RasterMode, rasterize};

/// Per-font geometry at one size, in pixels. Offsets are measured upward from the baseline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontMetricsPx {
    pub ascent: f32,
    /// Positive distance below the baseline.
    pub descent: f32,
    pub line_gap: f32,
    pub underline_offset: f32,
    pub underline_thickness: f32,
    pub strike_offset: f32,
    pub strike_thickness: f32,
}

/// Glyphs of one font inside a [`TextLayout`].
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub font: FontId,
    pub glyphs: Vec<GlyphPos>,
}

/// Options for [`Assets::layout_text`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextLayoutOptions {
    /// Round every advance to whole pixels (GDI-compatible spacing) instead of keeping the
    /// fractional advance.
    pub round_advances: bool,
}

/// Text placed glyph by glyph with font fallback. No kerning or ligatures: shape with
/// `open365-text` and use [`GlyphRun`](crate::GlyphRun) when those matter.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextLayout {
    pub runs: Vec<TextRun>,
    pub width: f32,
    /// Where the layout was placed: `[x, baseline]`.
    pub origin: [f32; 2],
    /// `(byte offset, x)` at every character boundary, including the end; x is relative to the
    /// layout origin.
    pub stops: Vec<(usize, f32)>,
}

impl TextLayout {
    /// The x of the boundary at `byte` (the last stop when beyond the text).
    pub fn x_at(&self, byte: usize) -> f32 {
        self.stops
            .iter()
            .find(|(b, _)| *b >= byte)
            .or(self.stops.last())
            .map_or(0.0, |s| s.1)
    }

    /// The byte offset of the boundary closest to `x`.
    pub fn byte_at(&self, x: f32) -> usize {
        self.stops
            .iter()
            .min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs()))
            .map_or(0, |s| s.0)
    }
}

impl Assets {
    /// Fonts tried, in order, for characters `font` has no glyph for.
    pub fn set_fallbacks(&mut self, font: FontId, fallbacks: Vec<FontId>) {
        self.fallbacks.insert(font, fallbacks);
    }

    pub fn fallbacks(&self, font: FontId) -> &[FontId] {
        self.fallbacks.get(&font).map_or(&[], Vec::as_slice)
    }

    fn font_ref(&self, font: FontId) -> Option<FontRef<'_>> {
        let e = self.fonts.get(font.0 as usize)?;
        FontRef::from_index(&e.bytes, e.index).ok()
    }

    /// True when `font` itself (without fallbacks) maps `c` to a glyph.
    pub fn has_glyph(&self, font: FontId, c: char) -> bool {
        self.font_ref(font)
            .is_some_and(|f| f.charmap().map(c).is_some())
    }

    /// The font (`font` or one of its fallbacks) and glyph id for `c`. Characters nobody covers
    /// come back as `font`'s glyph 0 (`.notdef`).
    pub fn resolve_char(&self, font: FontId, c: char) -> (FontId, u32) {
        for f in std::iter::once(font).chain(self.fallbacks(font).iter().copied()) {
            if let Some(g) = self.font_ref(f).and_then(|r| r.charmap().map(c)) {
                return (f, g.to_u32());
            }
        }
        (font, 0)
    }

    /// Linear advance of a glyph in pixels at `size_px`.
    pub fn advance(&self, font: FontId, glyph: u32, size_px: f32) -> f32 {
        self.font_ref(font)
            .and_then(|f| {
                f.glyph_metrics(Size::new(size_px), LocationRef::default())
                    .advance_width(GlyphId::new(glyph))
            })
            .unwrap_or(0.0)
    }

    /// Ascent, descent and decoration geometry of `font` at `size_px`.
    pub fn font_metrics(&self, font: FontId, size_px: f32) -> Option<FontMetricsPx> {
        let f = self.font_ref(font)?;
        let m = f.metrics(Size::new(size_px), LocationRef::default());
        let (uo, ut) = m.underline.map_or((-0.1 * size_px, 0.05 * size_px), |d| {
            (d.offset, d.thickness)
        });
        let (so, st) = m
            .strikeout
            .map_or((0.3 * size_px, 0.05 * size_px), |d| (d.offset, d.thickness));
        Some(FontMetricsPx {
            ascent: m.ascent,
            descent: -m.descent,
            line_gap: m.leading,
            underline_offset: uo,
            underline_thickness: ut,
            strike_offset: so,
            strike_thickness: st,
        })
    }

    /// Places `text` starting at `(x, baseline)` with fallback fonts.
    pub fn layout_text(
        &self,
        font: FontId,
        text: &str,
        size_px: f32,
        x: f32,
        baseline: f32,
        options: TextLayoutOptions,
    ) -> TextLayout {
        let mut out = TextLayout {
            origin: [x, baseline],
            ..TextLayout::default()
        };
        let mut pen = 0.0f32;
        for (i, c) in text.char_indices() {
            out.stops.push((i, pen));
            if c.is_control() {
                continue;
            }
            let (f, g) = self.resolve_char(font, c);
            if g != 0 || !c.is_whitespace() {
                if out.runs.last().is_none_or(|r| r.font != f) {
                    out.runs.push(TextRun {
                        font: f,
                        glyphs: Vec::new(),
                    });
                }
                if let Some(r) = out.runs.last_mut() {
                    r.glyphs.push(GlyphPos {
                        id: g,
                        x: x + pen,
                        y: baseline,
                    });
                }
            }
            let adv = self.advance(f, g, size_px);
            pen += if options.round_advances {
                adv.round()
            } else {
                adv
            };
        }
        out.stops.push((text.len(), pen));
        out.width = pen;
        out
    }

    /// Rasterises (or fetches) `path` as a coverage mask in device pixels.
    pub fn path_mask(&mut self, path: &Path) -> Option<Placed> {
        let key = path.key();
        if let Some(hit) = self.path_masks.get(&key) {
            return *hit;
        }
        let bitmap = rasterize(&path.lines_y_up(), 0.0, RasterMode::Gray);
        let placed = match bitmap {
            Some(bm) => match self.mask.insert(bm.width, bm.height, &bm.data) {
                Some(slot) => Some(Placed {
                    slot,
                    left: bm.left,
                    top: bm.top,
                }),
                None => {
                    self.overflowed = true;
                    self.reset_glyph_caches();
                    None
                }
            },
            None => None,
        };
        self.path_masks.insert(key, placed);
        placed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_font_resolves_to_notdef() {
        let a = Assets::new(64);
        assert_eq!(a.resolve_char(FontId(0), 'a'), (FontId(0), 0));
        assert_eq!(a.advance(FontId(0), 1, 12.0), 0.0);
        assert!(a.font_metrics(FontId(0), 12.0).is_none());
    }

    #[test]
    fn layout_stops_map_between_x_and_bytes() {
        let l = TextLayout {
            runs: vec![],
            width: 6.0,
            origin: [0.0, 0.0],
            stops: vec![(0, 0.0), (1, 3.0), (2, 6.0)],
        };
        assert_eq!(l.x_at(1), 3.0);
        assert_eq!(l.x_at(99), 6.0);
        assert_eq!(l.byte_at(4.0), 1);
        assert_eq!(l.byte_at(5.0), 2);
    }
}
