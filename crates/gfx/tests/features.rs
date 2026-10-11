//! Paths, gradients, shadows, rounded and path clips, text decorations and font fallback. GPU
//! tests skip with a note when the machine has no adapter.

use std::sync::Arc;

use open365_gfx::{
    Assets, Cap, Color, Decorations, DrawList, FontId, GpuContext, Join, LinearGradient, Offscreen,
    Path, Rect, Renderer, TextLayoutOptions, pixel_at,
};
use open365_text::{FontLibrary, FontStyle};

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

fn font(assets: &mut Assets) -> FontId {
    let lib = FontLibrary::bundled();
    let id = lib
        .resolve("Carlito", FontStyle::default())
        .expect("bundled Carlito");
    let bytes: Arc<[u8]> = Arc::from(lib.face(id).bytes().to_vec());
    assets.add_font(bytes, lib.face(id).index()).expect("font")
}

#[test]
fn horizontal_gradient_interpolates_in_srgb() {
    let Some(mut rig) = rig() else { return };
    let px = rig.draw(64, 8, |l, _| {
        l.fill_gradient(
            Rect::from_xywh(0.0, 0.0, 64.0, 8.0),
            &LinearGradient::horizontal(0.0, 64.0, Color::BLACK, Color::WHITE),
        );
    });
    let mid = pixel_at(&px, 64, 32, 4);
    assert!((i32::from(mid[0]) - 132).abs() <= 6, "mid grey {mid:?}");
    assert!(pixel_at(&px, 64, 0, 4)[0] < 6);
    assert!(pixel_at(&px, 64, 63, 4)[0] > 250);
    assert!(pixel_at(&px, 64, 10, 4)[0] < pixel_at(&px, 64, 50, 4)[0]);
}

#[test]
fn multi_stop_vertical_gradient_hits_its_stops() {
    let Some(mut rig) = rig() else { return };
    let g = LinearGradient {
        start: [0.0, 0.0],
        end: [0.0, 40.0],
        stops: vec![
            (0.0, Color::rgb(255, 0, 0)),
            (0.5, Color::rgb(0, 255, 0)),
            (1.0, Color::rgb(0, 0, 255)),
        ],
    };
    let px = rig.draw(8, 40, |l, _| {
        l.fill_gradient(Rect::from_xywh(0.0, 0.0, 8.0, 40.0), &g)
    });
    let at20 = pixel_at(&px, 8, 4, 20);
    assert!(at20[1] > 235 && at20[0] < 20 && at20[2] < 20, "{at20:?}");
    let top = pixel_at(&px, 8, 4, 0);
    assert!(top[0] > 235 && top[1] < 25);
}

#[test]
fn shadow_is_dark_under_the_shape_and_fades_outside() {
    let Some(mut rig) = rig() else { return };
    let px = rig.draw(80, 80, |l, _| {
        l.shadow(
            Rect::from_xywh(20.0, 20.0, 40.0, 40.0),
            4.0,
            3.0,
            [0.0, 2.0],
            Color::rgba(0, 0, 0, 200),
        );
    });
    let inside = pixel_at(&px, 80, 40, 42)[0];
    let edge = pixel_at(&px, 80, 40, 62)[0];
    let far = pixel_at(&px, 80, 40, 75)[0];
    assert!((120..=136).contains(&inside), "inside {inside}");
    assert!(edge > inside && edge < 240, "edge {edge}");
    assert!(far > 250, "far {far}");
    // Monotonic fade below the shape.
    let col: Vec<u8> = (60..80).map(|y| pixel_at(&px, 80, 40, y)[0]).collect();
    assert!(col.windows(2).all(|w| w[0] <= w[1]), "{col:?}");
}

#[test]
fn rounded_clip_cuts_the_corners() {
    let Some(mut rig) = rig() else { return };
    let px = rig.draw(40, 40, |l, a| {
        l.push_clip_rrect(a, Rect::from_xywh(0.0, 0.0, 40.0, 40.0), 12.0);
        l.fill_rect(Rect::from_xywh(0.0, 0.0, 40.0, 40.0), Color::rgb(255, 0, 0));
        l.pop_clip();
    });
    assert_eq!(
        pixel_at(&px, 40, 0, 0),
        [255, 255, 255, 255],
        "corner is clipped"
    );
    assert_eq!(pixel_at(&px, 40, 20, 20), [255, 0, 0, 255]);
    assert_eq!(
        pixel_at(&px, 40, 20, 0),
        [255, 0, 0, 255],
        "straight edge is kept"
    );
}

#[test]
fn nested_path_clips_multiply() {
    let Some(mut rig) = rig() else { return };
    let px = rig.draw(60, 60, |l, a| {
        l.push_clip_path(a, &Path::rect(Rect::from_xywh(0.0, 0.0, 30.0, 60.0)));
        l.push_clip_path(a, &Path::ellipse(Rect::from_xywh(10.0, 10.0, 40.0, 40.0)));
        l.fill_rect(Rect::from_xywh(0.0, 0.0, 60.0, 60.0), Color::BLACK);
        l.pop_clip();
        l.pop_clip();
    });
    assert_eq!(pixel_at(&px, 60, 20, 30)[0], 0, "inside both");
    assert_eq!(pixel_at(&px, 60, 40, 30)[0], 255, "right of the rect clip");
    assert_eq!(pixel_at(&px, 60, 11, 11)[0], 255, "outside the ellipse");
}

#[test]
fn clip_state_does_not_leak_to_later_batches() {
    let Some(mut rig) = rig() else { return };
    let px = rig.draw(40, 20, |l, a| {
        l.push_clip_rrect(a, Rect::from_xywh(0.0, 0.0, 20.0, 20.0), 10.0);
        l.fill_rect(Rect::from_xywh(0.0, 0.0, 20.0, 20.0), Color::BLACK);
        l.pop_clip();
        l.fill_rect(Rect::from_xywh(20.0, 0.0, 20.0, 20.0), Color::BLACK);
    });
    assert_eq!(
        pixel_at(&px, 40, 20, 0)[0],
        0,
        "corner of the unclipped rect"
    );
    assert_eq!(pixel_at(&px, 40, 0, 0)[0], 255);
}

#[test]
fn filled_path_has_exact_edge_coverage() {
    let Some(mut rig) = rig() else { return };
    let px = rig.draw(20, 20, |l, a| {
        let mut p = Path::new();
        p.move_to(2.0, 2.0)
            .line_to(10.5, 2.0)
            .line_to(10.5, 10.0)
            .line_to(2.0, 10.0)
            .close();
        l.fill_path(a, &p, Color::BLACK);
    });
    assert_eq!(pixel_at(&px, 20, 5, 5)[0], 0);
    assert_eq!(pixel_at(&px, 20, 1, 5)[0], 255);
    // Half covered column blends to the linear-light midpoint.
    let half = pixel_at(&px, 20, 10, 5)[0];
    assert!((half as i32 - 188).abs() <= 3, "half {half}");
}

#[test]
fn stroked_path_draws_a_band() {
    let Some(mut rig) = rig() else { return };
    let px = rig.draw(30, 20, |l, a| {
        let mut p = Path::new();
        p.move_to(2.0, 10.0).line_to(28.0, 10.0);
        l.stroke_path(a, &p, 4.0, Join::Miter, Cap::Butt, Color::BLACK);
    });
    assert_eq!(pixel_at(&px, 30, 15, 8)[0], 0);
    assert_eq!(pixel_at(&px, 30, 15, 11)[0], 0);
    assert_eq!(pixel_at(&px, 30, 15, 7)[0], 255);
    assert_eq!(pixel_at(&px, 30, 15, 12)[0], 255);
}

#[test]
fn underline_and_strikethrough_add_ink_below_and_through_the_text() {
    let Some(mut rig) = rig() else { return };
    let mut count = |decor: Decorations| -> usize {
        let px = rig.draw(80, 30, |l, a| {
            let f = font(a);
            let lay = a.layout_text(f, "Hello", 16.0, 4.0, 20.0, TextLayoutOptions::default());
            l.text(a, f, &lay, 16.0, Color::BLACK, decor);
        });
        px.chunks_exact(4).filter(|p| p[0] < 128).count()
    };
    let plain = count(Decorations::default());
    let under = count(Decorations {
        underline: true,
        strikethrough: false,
    });
    let strike = count(Decorations {
        underline: false,
        strikethrough: true,
    });
    assert!(plain > 20);
    assert!(under > plain + 20, "{under} vs {plain}");
    assert!(strike > plain + 20, "{strike} vs {plain}");
}

#[test]
fn fallback_font_supplies_missing_glyphs() {
    let mut assets = Assets::new(256);
    let carlito = font(&mut assets);
    let lib = FontLibrary::bundled();
    let cal = lib
        .resolve("Caladea", FontStyle::default())
        .expect("Caladea");
    let bytes: Arc<[u8]> = Arc::from(lib.face(cal).bytes().to_vec());
    let caladea = assets.add_font(bytes, 0).expect("font");
    // A CJK character is in neither font: notdef from the primary.
    assert_eq!(assets.resolve_char(carlito, '\u{4e2d}'), (carlito, 0));
    // A Latin letter resolves to the primary without consulting fallbacks.
    assert_eq!(assets.resolve_char(carlito, 'a').0, carlito);
    assets.set_fallbacks(carlito, vec![caladea]);
    assert_eq!(assets.fallbacks(carlito), &[caladea]);
    let lay = assets.layout_text(carlito, "ab", 16.0, 0.0, 10.0, TextLayoutOptions::default());
    assert_eq!(lay.runs.len(), 1);
    assert_eq!(lay.stops.len(), 3);
    assert!(lay.width > 10.0);
    let rounded = assets.layout_text(
        carlito,
        "ab",
        16.0,
        0.0,
        10.0,
        TextLayoutOptions {
            round_advances: true,
        },
    );
    assert_eq!(rounded.width.fract(), 0.0);
    let m = assets.font_metrics(carlito, 16.0).expect("metrics");
    assert!(m.ascent > 10.0 && m.descent > 2.0 && m.underline_thickness > 0.0);
}
