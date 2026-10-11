//! Renders through wgpu into an offscreen texture and checks exact pixels. Every test skips
//! (passes with a note) when the machine has no adapter, so CI without a GPU stays green.

use std::sync::Arc;

use open365_gfx::{
    Assets, Color, DrawList, GlyphPos, GlyphRun, GpuContext, Offscreen, Rect, Renderer,
    TextOptions, pixel_at,
};
use open365_text::{FontLibrary, FontStyle, ShapeFeatures, shape_run};

const WHITE: [u8; 4] = [255, 255, 255, 255];

struct Rig {
    gpu: GpuContext,
    renderer: Renderer,
    assets: Assets,
}

fn rig() -> Option<Rig> {
    let gpu = match GpuContext::headless() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("skipping GPU test: {e}");
            return None;
        }
    };
    let assets = Assets::new(512);
    let renderer = Renderer::new(
        &gpu.device,
        open365_gfx::OFFSCREEN_FORMAT,
        assets.atlas_size(),
    );
    Some(Rig {
        gpu,
        renderer,
        assets,
    })
}

impl Rig {
    fn draw(&mut self, w: u32, h: u32, build: impl FnOnce(&mut DrawList, &mut Assets)) -> Vec<u8> {
        let mut list = DrawList::new(w, h);
        build(&mut list, &mut self.assets);
        let target = Offscreen::new(&self.gpu.device, w, h);
        self.renderer
            .render(
                &self.gpu.device,
                &self.gpu.queue,
                &mut self.assets,
                &list,
                target.view(),
                Some(Color::WHITE),
            )
            .expect("render");
        target
            .read_rgba(&self.gpu.device, &self.gpu.queue)
            .expect("readback")
    }
}

fn px(img: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    pixel_at(img, w, x, y)
}

fn near(a: [u8; 4], b: [u8; 4], tol: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tol)
}

#[test]
fn snapped_rect_fills_exact_pixels() {
    let Some(mut rig) = rig() else { return };
    let img = rig.draw(16, 16, |l, _| {
        l.fill_rect(Rect::from_xywh(2.4, 3.0, 5.2, 4.0), Color::rgb(255, 0, 0));
    });
    // Edges round to x 2..8, y 3..7.
    assert_eq!(px(&img, 16, 2, 3), [255, 0, 0, 255]);
    assert_eq!(px(&img, 16, 7, 6), [255, 0, 0, 255]);
    assert_eq!(px(&img, 16, 1, 3), WHITE);
    assert_eq!(px(&img, 16, 8, 3), WHITE);
    assert_eq!(px(&img, 16, 2, 7), WHITE);
    assert_eq!(px(&img, 16, 2, 2), WHITE);
}

#[test]
fn colours_round_trip_exactly_through_the_srgb_target() {
    let Some(mut rig) = rig() else { return };
    let c = Color::rgb(12, 99, 200);
    let img = rig.draw(8, 8, |l, _| {
        l.fill_rect(Rect::from_xywh(0.0, 0.0, 8.0, 8.0), c)
    });
    assert!(
        near(px(&img, 8, 3, 3), [12, 99, 200, 255], 1),
        "{:?}",
        px(&img, 8, 3, 3)
    );
}

#[test]
fn half_transparent_black_over_white_blends_in_linear_light() {
    let Some(mut rig) = rig() else { return };
    let img = rig.draw(8, 8, |l, _| {
        l.fill_rect(
            Rect::from_xywh(0.0, 0.0, 8.0, 8.0),
            Color::rgba(0, 0, 0, 128),
        );
    });
    // 50.2% black over white is 0.498 linear light = sRGB 187.
    let p = px(&img, 8, 4, 4);
    assert!(near(p, [187, 187, 187, 255], 2), "{p:?}");
    assert!(p[0] > 128, "naive sRGB-space blending would give about 127");
}

#[test]
fn clip_limits_drawing_to_whole_pixels() {
    let Some(mut rig) = rig() else { return };
    let img = rig.draw(16, 16, |l, _| {
        l.push_clip(Rect::from_xywh(4.0, 4.0, 4.0, 4.0));
        l.fill_rect(Rect::from_xywh(0.0, 0.0, 16.0, 16.0), Color::BLACK);
        l.pop_clip();
    });
    assert_eq!(px(&img, 16, 4, 4), [0, 0, 0, 255]);
    assert_eq!(px(&img, 16, 7, 7), [0, 0, 0, 255]);
    assert_eq!(px(&img, 16, 3, 4), WHITE);
    assert_eq!(px(&img, 16, 8, 7), WHITE);
}

#[test]
fn hairlines_are_one_crisp_pixel() {
    let Some(mut rig) = rig() else { return };
    let img = rig.draw(16, 16, |l, _| {
        l.line([1.0, 5.7], [14.0, 5.7], 1.0, Color::BLACK);
        l.line([9.2, 7.0], [9.2, 15.0], 1.0, Color::BLACK);
    });
    for x in 1..14 {
        assert_eq!(px(&img, 16, x, 5), [0, 0, 0, 255], "x {x}");
        assert_eq!(px(&img, 16, x, 4), WHITE);
        assert_eq!(px(&img, 16, x, 6), WHITE);
    }
    assert_eq!(px(&img, 16, 9, 10), [0, 0, 0, 255]);
    assert_eq!(px(&img, 16, 8, 10), WHITE);
    assert_eq!(px(&img, 16, 10, 10), WHITE);
}

#[test]
fn rounded_rect_cuts_its_corners_and_keeps_its_edges() {
    let Some(mut rig) = rig() else { return };
    let img = rig.draw(32, 32, |l, _| {
        l.fill_rrect(Rect::from_xywh(4.0, 4.0, 24.0, 24.0), 8.0, Color::BLACK);
    });
    assert_eq!(px(&img, 32, 16, 16), [0, 0, 0, 255]);
    assert_eq!(
        px(&img, 32, 4, 16),
        [0, 0, 0, 255],
        "straight edge is solid"
    );
    assert_eq!(px(&img, 32, 4, 4), WHITE, "corner pixel is outside the arc");
    let edge = px(&img, 32, 6, 6);
    assert!(
        edge[0] > 0 && edge[0] < 255,
        "arc edge antialiases: {edge:?}"
    );
}

#[test]
fn images_draw_one_texel_per_pixel() {
    let Some(mut rig) = rig() else { return };
    let rgba = [
        255, 0, 0, 255, 0, 255, 0, 255, //
        0, 0, 255, 255, 10, 20, 30, 255,
    ];
    let id = rig.assets.add_image(2, 2, &rgba).expect("image");
    let img = rig.draw(8, 8, |l, a| {
        l.image(a, id, Rect::from_xywh(3.0, 2.0, 2.0, 2.0), Color::WHITE);
    });
    assert!(near(px(&img, 8, 3, 2), [255, 0, 0, 255], 1));
    assert!(near(px(&img, 8, 4, 2), [0, 255, 0, 255], 1));
    assert!(near(px(&img, 8, 3, 3), [0, 0, 255, 255], 1));
    assert!(near(px(&img, 8, 4, 3), [10, 20, 30, 255], 1));
    assert_eq!(px(&img, 8, 2, 2), WHITE);
}

#[test]
fn icons_take_their_colour_from_the_draw_call() {
    let Some(mut rig) = rig() else { return };
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8" width="8" height="8">
        <rect x="0" y="0" width="8" height="4" fill="black"/></svg>"#;
    let icon = rig.assets.add_icon(svg).expect("svg");
    let img = rig.draw(16, 16, |l, a| {
        l.icon(
            a,
            icon,
            Rect::from_xywh(4.0, 4.0, 8.0, 8.0),
            Color::rgb(0, 0, 255),
        );
    });
    assert!(near(px(&img, 16, 6, 5), [0, 0, 255, 255], 1));
    assert_eq!(px(&img, 16, 6, 9), WHITE, "lower half of the icon is empty");
}

fn text_run(assets: &mut Assets, text: &str) -> (open365_gfx::FontId, Vec<GlyphPos>) {
    let lib = FontLibrary::bundled();
    let face = lib
        .resolve("Carlito", FontStyle::default())
        .expect("Carlito");
    let font = assets
        .add_font(
            Arc::from(lib.face(face).bytes().to_vec()),
            lib.face(face).index(),
        )
        .expect("font");
    let chain = lib.chain("Carlito", FontStyle::default());
    let run = shape_run(&lib, &chain, text, ShapeFeatures::default());
    let mut pen = 2.0;
    let glyphs = run
        .glyphs
        .iter()
        .map(|g| {
            let p = GlyphPos {
                id: g.glyph_id,
                x: pen,
                y: 18.0,
            };
            pen += g.x_advance as f32 * 16.0 / f32::from(lib.face(g.face).units_per_em());
            p
        })
        .collect();
    (font, glyphs)
}

#[test]
fn grayscale_text_is_dark_ink_on_white() {
    let Some(mut rig) = rig() else { return };
    let (font, glyphs) = text_run(&mut rig.assets, "Hill");
    let img = rig.draw(48, 24, |l, a| {
        l.glyph_run(
            a,
            &GlyphRun {
                font,
                size_px: 16.0,
                glyphs: &glyphs,
            },
            Color::BLACK,
        );
    });
    let mut black = 0;
    let mut grey = 0;
    for y in 0..24 {
        for x in 0..48 {
            let p = px(&img, 48, x, y);
            assert_eq!(p[0], p[1], "grayscale text has no colour fringe");
            assert_eq!(p[1], p[2]);
            if p[0] == 0 {
                black += 1;
            } else if p[0] < 255 {
                grey += 1;
            }
        }
    }
    assert!(black >= 8, "hinted stems reach full black ({black})");
    assert!(grey > 4, "edges antialias ({grey})");
}

#[test]
fn lcd_text_has_colour_fringes_and_uses_the_three_pass_path() {
    let Some(mut rig) = rig() else { return };
    rig.assets.set_text_options(TextOptions {
        lcd: true,
        ..TextOptions::default()
    });
    let (font, glyphs) = text_run(&mut rig.assets, "Hill");
    let img = rig.draw(48, 24, |l, a| {
        l.glyph_run(
            a,
            &GlyphRun {
                font,
                size_px: 16.0,
                glyphs: &glyphs,
            },
            Color::BLACK,
        );
        assert!(l.batches().iter().all(|b| b.lcd));
    });
    let mut fringed = 0;
    let mut dark = 0;
    for y in 0..24 {
        for x in 0..48 {
            let p = px(&img, 48, x, y);
            if p[0] != p[2] {
                fringed += 1;
            }
            // FIR filtering keeps a 1.5 px stem from reaching pure black, as with ClearType.
            if p[0] < 110 && p[1] < 110 && p[2] < 110 {
                dark += 1;
            }
        }
    }
    assert!(fringed > 4, "{fringed}");
    assert!(dark >= 4, "{dark}");
}
