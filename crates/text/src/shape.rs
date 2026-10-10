use harfrust::{Feature, ShapeOptions, Tag, UnicodeBuffer};

use crate::{FaceId, FontChain, FontLibrary};

/// OpenType features applied to a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShapeFeatures {
    /// Pair kerning (`kern`).
    pub kerning: bool,
    /// Standard ligatures (`liga`, `clig`).
    pub ligatures: bool,
}

impl Default for ShapeFeatures {
    fn default() -> Self {
        Self {
            kerning: true,
            ligatures: true,
        }
    }
}

/// One shaped glyph. Distances are in font units of its face; `cluster` is a byte offset into
/// the shaped text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShapedGlyph {
    pub face: FaceId,
    pub glyph_id: u32,
    pub cluster: u32,
    pub x_advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
}

/// The shaped form of a piece of text in logical order (left-to-right only for now).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapedRun {
    pub glyphs: Vec<ShapedGlyph>,
}

impl ShapedRun {
    /// Total advance in points at `size_pt`.
    pub fn width_pt(&self, lib: &FontLibrary, size_pt: f32) -> f32 {
        self.glyphs
            .iter()
            .map(|g| g.x_advance as f32 * size_pt / f32::from(lib.face(g.face).units_per_em()))
            .sum()
    }
}

fn features(f: ShapeFeatures) -> Vec<Feature> {
    let flag = |on: bool| u32::from(on);
    vec![
        Feature::new(Tag::new(b"kern"), flag(f.kerning), ..),
        Feature::new(Tag::new(b"liga"), flag(f.ligatures), ..),
        Feature::new(Tag::new(b"clig"), flag(f.ligatures), ..),
    ]
}

fn shape_with(
    lib: &FontLibrary,
    face: FaceId,
    text: &str,
    base: u32,
    feats: &[Feature],
    out: &mut Vec<ShapedGlyph>,
) {
    let f = lib.face(face);
    let Some(font) = f.font_ref() else { return };
    let shaper = f.shaper_data().shaper(&font).build();
    let mut buf = UnicodeBuffer::new();
    buf.push_str(text);
    buf.guess_segment_properties();
    let shaped = shaper.shape(buf, ShapeOptions::new().features(feats));
    for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
        out.push(ShapedGlyph {
            face,
            glyph_id: info.glyph_id,
            cluster: base + info.cluster,
            x_advance: pos.x_advance,
            x_offset: pos.x_offset,
            y_offset: pos.y_offset,
        });
    }
}

/// Shapes `text` through `chain`: each maximal stretch of characters whose first covering face
/// is the same is shaped together. Characters no face covers use the first face's `.notdef`.
pub fn shape_run(
    lib: &FontLibrary,
    chain: &FontChain,
    text: &str,
    feats: ShapeFeatures,
) -> ShapedRun {
    let feats = features(feats);
    let mut glyphs = Vec::new();
    let mut seg_start = 0usize;
    let mut seg_face: Option<FaceId> = None;
    for (i, c) in text.char_indices() {
        // Neutral characters (spaces, controls) stay with the current face.
        let neutral = c.is_whitespace() || c.is_control();
        let face = if neutral && seg_face.is_some() {
            seg_face
        } else {
            chain
                .faces
                .iter()
                .copied()
                .find(|f| lib.face(*f).has_glyph(c))
                .or_else(|| chain.faces.first().copied())
        };
        if face != seg_face {
            if let Some(f) = seg_face {
                shape_with(
                    lib,
                    f,
                    &text[seg_start..i],
                    seg_start as u32,
                    &feats,
                    &mut glyphs,
                );
            }
            seg_start = i;
            seg_face = face;
        }
    }
    if let Some(f) = seg_face {
        shape_with(
            lib,
            f,
            &text[seg_start..],
            seg_start as u32,
            &feats,
            &mut glyphs,
        );
    }
    ShapedRun { glyphs }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FontStyle;

    fn lib_chain(family: &str) -> (FontLibrary, FontChain) {
        let lib = FontLibrary::bundled();
        let chain = lib.chain(family, FontStyle::default());
        (lib, chain)
    }

    #[test]
    fn width_scales_with_size() {
        let (lib, chain) = lib_chain("Calibri");
        let run = shape_run(&lib, &chain, "Hello world", ShapeFeatures::default());
        assert!(!run.glyphs.is_empty());
        let w11 = run.width_pt(&lib, 11.0);
        let w22 = run.width_pt(&lib, 22.0);
        assert!((w22 - 2.0 * w11).abs() < 1e-3);
        assert!(w11 > 30.0 && w11 < 80.0, "width {w11}");
    }

    #[test]
    fn clusters_are_byte_offsets() {
        let (lib, chain) = lib_chain("Cambria");
        let run = shape_run(&lib, &chain, "ab cd", ShapeFeatures::default());
        let clusters: Vec<u32> = run.glyphs.iter().map(|g| g.cluster).collect();
        assert_eq!(clusters, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn uncovered_characters_still_get_a_glyph() {
        // No bundled face has CJK glyphs, so the first face's .notdef stands in.
        let (lib, chain) = lib_chain("Calibri");
        let run = shape_run(&lib, &chain, "a\u{4e2d}b", ShapeFeatures::default());
        assert_eq!(run.glyphs.len(), 3);
    }
}
