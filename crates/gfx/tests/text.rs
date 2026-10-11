//! Glyph, icon and image pipeline tests that need no GPU.

use std::sync::Arc;

use open365_gfx::atlas::AtlasSlot;
use open365_gfx::{Assets, Color, DrawList, FontId, GlyphPos, GlyphRun, Hinting, TextOptions};
use open365_text::{FontLibrary, FontStyle, ShapeFeatures, shape_run};

const SIZE_PX: f32 = 16.0;

fn font(assets: &mut Assets) -> FontId {
    let lib = FontLibrary::bundled();
    let id = lib
        .resolve("Carlito", FontStyle::default())
        .expect("bundled Carlito");
    let bytes: Arc<[u8]> = Arc::from(lib.face(id).bytes().to_vec());
    assets
        .add_font(bytes, lib.face(id).index())
        .expect("font accepted")
}

/// Shapes `text` with the text crate and returns glyph positions starting at `x`, baseline `y`.
fn layout(text: &str, size_px: f32, x: f32, y: f32) -> Vec<GlyphPos> {
    let lib = FontLibrary::bundled();
    let chain = lib.chain("Carlito", FontStyle::default());
    let run = shape_run(&lib, &chain, text, ShapeFeatures::default());
    let mut pen = x;
    run.glyphs
        .iter()
        .map(|g| {
            let pos = GlyphPos {
                id: g.glyph_id,
                x: pen + g.x_offset as f32 * size_px / f32::from(lib.face(g.face).units_per_em()),
                y,
            };
            pen += g.x_advance as f32 * size_px / f32::from(lib.face(g.face).units_per_em());
            pos
        })
        .collect()
}

fn glyph_id(text: &str) -> u32 {
    layout(text, SIZE_PX, 0.0, 0.0)[0].id
}

fn slot_pixels(assets: &Assets, slot: AtlasSlot) -> Vec<u8> {
    let mut out = Vec::new();
    for y in 0..slot.height {
        for x in 0..slot.width {
            out.extend_from_slice(assets.mask().pixel(slot.x + x, slot.y + y));
        }
    }
    out
}

#[test]
fn hinted_horizontals_have_near_full_coverage() {
    let mut assets = Assets::new(512);
    let f = font(&mut assets);
    let h = assets
        .glyph(f, glyph_id("H"), SIZE_PX, 0)
        .expect("H has an outline");
    let pixels = slot_pixels(&assets, h.slot);
    // Vertical-only hinting leaves the 1.4 px stem straddling columns (231 + 135), but the
    // snapped crossbar and serifs reach near-full coverage.
    assert!(
        pixels.iter().any(|&p| p >= 240),
        "hinted horizontals should fill whole pixels"
    );
    assert!(
        pixels.iter().any(|&p| p > 0 && p < 255),
        "curves and edges antialias"
    );
    assert!(h.top > 0 && h.top <= SIZE_PX as i32, "top {}", h.top);
}

#[test]
fn glyph_bitmaps_are_cached_per_bin() {
    let mut assets = Assets::new(512);
    let f = font(&mut assets);
    let id = glyph_id("l");
    let a = assets.glyph(f, id, SIZE_PX, 0).expect("glyph");
    let used = assets.mask().utilisation();
    let again = assets.glyph(f, id, SIZE_PX, 0).expect("glyph");
    assert_eq!(a, again);
    assert_eq!(
        assets.mask().utilisation(),
        used,
        "second lookup must not rasterise"
    );
    let shifted = assets.glyph(f, id, SIZE_PX, 2).expect("glyph");
    assert_ne!(a.slot, shifted.slot);
    assert_ne!(
        slot_pixels(&assets, a.slot),
        slot_pixels(&assets, shifted.slot)
    );
}

#[test]
fn subpixel_bins_spread_a_stem_across_columns() {
    let mut assets = Assets::new(512);
    assets.set_text_options(TextOptions {
        hinting: Hinting::None,
        subpixel_bins: 4,
        lcd: false,
    });
    let f = font(&mut assets);
    let id = glyph_id("l");
    let at = |assets: &mut Assets, bin| {
        let p = assets.glyph(f, id, SIZE_PX, bin).expect("glyph");
        let w = p.slot.width;
        let mid = p.slot.height / 2;
        (0..w)
            .map(|x| u32::from(assets.mask().pixel(p.slot.x + x, p.slot.y + mid)[0]))
            .sum::<u32>()
    };
    let (c0, c3) = (at(&mut assets, 0), at(&mut assets, 3));
    // Shifting by a fraction of a pixel preserves total ink within rounding.
    assert!(c0.abs_diff(c3) <= 6, "{c0} vs {c3}");
}

#[test]
fn hinting_changes_the_bitmap() {
    let mut off = Assets::new(512);
    off.set_text_options(TextOptions {
        hinting: Hinting::None,
        ..TextOptions::default()
    });
    let mut on = Assets::new(512);
    let (fo, fn_) = (font(&mut off), font(&mut on));
    let mut differs = false;
    for g in layout("Hamburgefonts", SIZE_PX, 0.0, 0.0) {
        let a = off.glyph(fo, g.id, SIZE_PX, 0).expect("glyph");
        let b = on.glyph(fn_, g.id, SIZE_PX, 0).expect("glyph");
        differs |= slot_pixels(&off, a.slot) != slot_pixels(&on, b.slot) || a.top != b.top;
    }
    assert!(
        differs,
        "hinted and unhinted rasterisation should differ somewhere"
    );
}

#[test]
fn lcd_glyphs_have_unequal_channels_at_edges() {
    let mut assets = Assets::new(512);
    assets.set_text_options(TextOptions {
        lcd: true,
        ..TextOptions::default()
    });
    let f = font(&mut assets);
    let p = assets.glyph(f, glyph_id("l"), SIZE_PX, 0).expect("glyph");
    let mut unequal = false;
    for y in 0..p.slot.height {
        for x in 0..p.slot.width {
            let px = assets.lcd().pixel(p.slot.x + x, p.slot.y + y);
            unequal |= px[0] != px[2];
        }
    }
    assert!(unequal, "LCD filtering should colour the stem edges");
    assert_eq!(
        assets.mask().utilisation(),
        0.0,
        "LCD glyphs stay out of the gray atlas"
    );
}

#[test]
fn glyph_run_places_quads_on_whole_pixels() {
    let mut assets = Assets::new(512);
    let f = font(&mut assets);
    let glyphs = layout("Rect 10", SIZE_PX, 10.3, 40.4);
    let mut list = DrawList::new(200, 100);
    list.glyph_run(
        &mut assets,
        &GlyphRun {
            font: f,
            size_px: SIZE_PX,
            glyphs: &glyphs,
        },
        Color::BLACK,
    );
    // The space produces no quad.
    assert_eq!(list.vertices().len(), 4 * 6);
    for v in list.vertices() {
        assert_eq!(v.pos[0].fract(), 0.0);
        assert_eq!(v.pos[1].fract(), 0.0);
    }
}

#[test]
fn full_atlas_clears_and_reports_overflow() {
    let mut assets = Assets::new(64);
    let f = font(&mut assets);
    let mut overflowed = false;
    for g in layout("Hamburgefonstiv ABCDEFGHIJKLMNOP", 30.0, 0.0, 0.0) {
        assets.glyph(f, g.id, 30.0, 0);
        overflowed |= assets.take_overflow();
    }
    assert!(overflowed);
    assert!(assets.generation() >= 1);
}

#[test]
fn icon_is_rasterised_as_a_coverage_mask_at_the_requested_size() {
    let mut assets = Assets::new(256);
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24">
        <rect x="6" y="6" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"/></svg>"#;
    let icon = assets.add_icon(svg).expect("valid svg");
    let p = assets.icon_mask(icon, 48, 48).expect("rasterised");
    assert_eq!((p.slot.width, p.slot.height), (48, 48));
    // 2x scale: the stroke band covers x 10..14 on the 48 px mask.
    let at = |x, y| assets.mask().pixel(p.slot.x + x, p.slot.y + y)[0];
    assert_eq!(at(12, 24), 255);
    assert_eq!(at(24, 24), 0, "inside the square is empty");
    assert_eq!(at(2, 2), 0);
    assert!(assets.add_icon(b"not svg").is_err());
}

#[test]
fn images_must_match_their_byte_length() {
    let mut assets = Assets::new(64);
    assert!(assets.add_image(2, 2, &[0; 15]).is_err());
    let id = assets
        .add_image(2, 1, &[1, 2, 3, 255, 4, 5, 6, 255])
        .expect("image");
    let slot = assets.image(id).expect("registered");
    assert_eq!(assets.color().pixel(slot.x + 1, slot.y), &[4, 5, 6, 255]);
}

#[test]
fn bad_font_data_is_rejected() {
    let mut assets = Assets::new(64);
    assert!(assets.add_font(Arc::from(vec![0u8; 16]), 0).is_err());
}
