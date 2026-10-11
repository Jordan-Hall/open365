//! Fits `RenderParams` text gamma and contrast to glyph coverage that Office draws.
//!
//! Input is a raw BGRA dump of a 1600 x 1000 Excel window at 125 % (made with
//! `tools`-style PrintWindow capture kept in a scratch folder, never in the repo). Only the
//! derived numbers are kept: run
//!
//! ```text
//! OPEN365_FIT_CAPTURE=<scratch>/excel-window.bgra cargo run -p open365-gfx --example fit_text --release
//! ```
//!
//! Two sets are fitted at 120 dpi:
//!
//! * ribbon tab labels (Segoe UI 14 dip = 17.5 px), light ink on `#090909`, grayscale antialiasing (light ink parameters);
//! * cell text typed in Segoe UI 9 pt (15 px), black on white, ClearType (dark ink parameters, LCD path).

use std::sync::Arc;

use open365_gfx::{
    Assets, Color, DrawList, GpuContext, Hinting, Offscreen, RenderParams, Renderer,
    TextLayoutOptions, TextOptions,
};

const W: usize = 1600;
use std::sync::atomic::{AtomicU32, Ordering};

/// Pixel size of the text being fitted (bits of an f32).
static SIZE_BITS: AtomicU32 = AtomicU32::new(0);
static ROUND: AtomicU32 = AtomicU32::new(0);

fn size_px() -> f32 {
    f32::from_bits(SIZE_BITS.load(Ordering::Relaxed))
}

struct Cap {
    rgb: Vec<[u8; 3]>,
}

impl Cap {
    fn at(&self, x: usize, y: usize) -> [u8; 3] {
        self.rgb[y * W + x]
    }
}

struct Item {
    text: &'static str,
    /// Region in capture pixels.
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
}

fn render(
    gpu: &GpuContext,
    renderer: &mut Renderer,
    assets: &mut Assets,
    items: &[(Item, f32, f32)],
    font: open365_gfx::FontId,
    bg: Color,
    ink: Color,
) -> Vec<u8> {
    let rows: usize = items.iter().map(|(i, _, _)| i.h).sum();
    let width = items.iter().map(|(i, _, _)| i.w).max().unwrap_or(1);
    let mut list = DrawList::new(width as u32, rows as u32);
    let mut y = 0usize;
    for (it, dx, base) in items {
        list.push_clip(open365_gfx::Rect::from_xywh(
            0.0,
            y as f32,
            it.w as f32,
            it.h as f32,
        ));
        let lay = assets.layout_text(
            font,
            it.text,
            size_px(),
            *dx,
            y as f32 + *base,
            TextLayoutOptions {
                round_advances: ROUND.load(Ordering::Relaxed) == 1,
            },
        );
        list.text(assets, font, &lay, size_px(), ink, Default::default());
        list.pop_clip();
        y += it.h;
    }
    let target = Offscreen::new(&gpu.device, width as u32, rows as u32);
    renderer
        .render(
            &gpu.device,
            &gpu.queue,
            assets,
            &list,
            target.view(),
            Some(bg),
        )
        .expect("render");
    target.read_rgba(&gpu.device, &gpu.queue).expect("readback")
}

/// Sum of squared differences between the render and the capture over all items, channels `ch`.
fn error(cap: &Cap, items: &[(Item, f32, f32)], px: &[u8], width: usize, per_channel: bool) -> f64 {
    let mut e = 0.0;
    let mut y0 = 0usize;
    for (it, _, _) in items {
        for y in 0..it.h {
            for x in 0..it.w {
                let c = cap.at(it.x0 + x, it.y0 + y);
                let i = ((y0 + y) * width + x) * 4;
                if per_channel {
                    for k in 0..3 {
                        let d = f64::from(c[k]) - f64::from(px[i + k]);
                        e += d * d;
                    }
                } else {
                    let d = f64::from(c[0]) - f64::from(px[i]);
                    e += d * d;
                }
            }
        }
        y0 += it.h;
    }
    e
}

#[allow(clippy::too_many_arguments)]
fn fit(
    name: &str,
    cap: &Cap,
    gpu: &GpuContext,
    assets: &mut Assets,
    font: open365_gfx::FontId,
    regions: &[Item],
    bg: Color,
    ink: Color,
    lcd: bool,
    hinting: Hinting,
) {
    let mut renderer = Renderer::new(
        &gpu.device,
        open365_gfx::OFFSCREEN_FORMAT,
        assets.atlas_size(),
    );
    assets.set_text_options(TextOptions {
        hinting,
        subpixel_bins: 4,
        lcd,
    });
    let count = regions.len();
    let rebuild = |dx: &[f32], base: &[f32]| -> Vec<(Item, f32, f32)> {
        regions
            .iter()
            .enumerate()
            .map(|(i, r)| {
                (
                    Item {
                        text: r.text,
                        x0: r.x0,
                        y0: r.y0,
                        w: r.w,
                        h: r.h,
                    },
                    dx[i],
                    base[i],
                )
            })
            .collect()
    };
    // 1. Alignment per item with identity parameters.
    renderer.set_params(RenderParams::default());
    let mut dx = vec![0.0f32; count];
    let mut base = vec![0.0f32; count];
    for i in 0..count {
        let mut best = (f64::MAX, 0.0f32, 0.0f32);
        let mut try_at = |cx: f32, cy: f32, best: &mut (f64, f32, f32)| {
            let it = &regions[i];
            let one = vec![(
                Item {
                    text: it.text,
                    x0: it.x0,
                    y0: it.y0,
                    w: it.w,
                    h: it.h,
                },
                cx,
                cy,
            )];
            let px = render(gpu, &mut renderer, assets, &one, font, bg, ink);
            let e = error(cap, &one, &px, it.w, lcd);
            if e < best.0 {
                *best = (e, cx, cy);
            }
        };
        for bx in 0..regions[i].w {
            for by in 6..(regions[i].h as i32 - 2) {
                try_at(bx as f32, by as f32, &mut best);
            }
        }
        let (c0x, c0y) = (best.1, best.2);
        for fx in -4..=4 {
            try_at(c0x + fx as f32 * 0.25, c0y, &mut best);
        }
        dx[i] = best.1;
        base[i] = best.2;
    }
    let items = rebuild(&dx, &base);
    let width = items.iter().map(|(i, _, _)| i.w).max().unwrap_or(1);
    let pixels: usize = items.iter().map(|(i, _, _)| i.w * i.h).sum();
    let channels = if lcd { 3.0 } else { 1.0 };
    let rms = |e: f64| (e / (pixels as f64 * channels)).sqrt();
    let px = render(gpu, &mut renderer, assets, &items, font, bg, ink);
    let identity = rms(error(cap, &items, &px, width, lcd));
    // 2. Gamma and contrast grid.
    let mut best = (f64::MAX, 1.0f32, 0.0f32);
    for gi in 0..=24 {
        for ci in 0..=16 {
            let g = 0.6 + gi as f32 * 0.1;
            let c = -0.4 + ci as f32 * 0.1;
            let params = if ink.r > 128 {
                RenderParams {
                    text_gamma_light: g,
                    text_contrast_light: c,
                    ..RenderParams::default()
                }
            } else {
                RenderParams {
                    text_gamma: g,
                    text_contrast: c,
                    ..RenderParams::default()
                }
            };
            renderer.set_params(params);
            let px = render(gpu, &mut renderer, assets, &items, font, bg, ink);
            let e = error(cap, &items, &px, width, lcd);
            if e < best.0 {
                best = (e, g, c);
            }
        }
    }
    // Alignment-free check: total ink mass (sum of coverage) of the capture against the render.
    let ink_levels = |v: u8, b: u8, i: u8| -> f64 {
        (f64::from(v) - f64::from(b)) / (f64::from(i) - f64::from(b))
    };
    let cap_mass: f64 = {
        let mut m = 0.0;
        for it in &items {
            for y in 0..it.0.h {
                for x in 0..it.0.w {
                    let c = cap.at(it.0.x0 + x, it.0.y0 + y);
                    m += ink_levels(c[0], bg.r, ink.r)
                        + ink_levels(c[1], bg.g, ink.g)
                        + ink_levels(c[2], bg.b, ink.b);
                }
            }
        }
        m
    };
    let mut mass_fit = (f64::MAX, 1.0f32, 0.0f64);
    for gi in 0..=40 {
        let g = 0.5 + gi as f32 * 0.05;
        renderer.set_params(if ink.r > 128 {
            RenderParams {
                text_gamma_light: g,
                ..RenderParams::default()
            }
        } else {
            RenderParams {
                text_gamma: g,
                ..RenderParams::default()
            }
        });
        let px = render(gpu, &mut renderer, assets, &items, font, bg, ink);
        let mut m = 0.0;
        let mut y0 = 0usize;
        for it in &items {
            for y in 0..it.0.h {
                for x in 0..it.0.w {
                    let i = ((y0 + y) * width + x) * 4;
                    m += ink_levels(px[i], bg.r, ink.r)
                        + ink_levels(px[i + 1], bg.g, ink.g)
                        + ink_levels(px[i + 2], bg.b, ink.b);
                }
            }
            y0 += it.0.h;
        }
        let ratio = m / cap_mass;
        if (ratio - 1.0).abs() < mass_fit.0 {
            mass_fit = ((ratio - 1.0).abs(), g, ratio);
        }
    }
    println!(
        "  ink-mass match: gamma {:.2} (render/capture ink ratio {:.3})",
        mass_fit.1, mass_fit.2
    );
    println!(
        "{name} {hinting:?} round={}: items {count}, alignments dx {dx:?} baseline {base:?}\n  identity rms {identity:.2} levels -> fitted gamma {:.2} contrast {:.2} rms {:.2} levels",
        ROUND.load(Ordering::Relaxed),
        best.1,
        best.2,
        rms(best.0)
    );
}

fn main() {
    let path =
        std::env::var("OPEN365_FIT_CAPTURE").expect("set OPEN365_FIT_CAPTURE to the BGRA dump");
    let raw = std::fs::read(path).expect("capture file");
    let rgb = raw
        .chunks_exact(4)
        .map(|p| [p[2], p[1], p[0]])
        .collect::<Vec<_>>();
    let cap = Cap { rgb };
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    let bytes = std::fs::read(format!("{windir}\\Fonts\\segoeui.ttf")).expect("Segoe UI");
    let gpu = GpuContext::headless().expect("adapter");
    let mut assets = Assets::new(1024);
    let font = assets.add_font(Arc::from(bytes), 0).expect("font");

    // Ribbon tab labels are 14 dip = 17.5 px at 125 % (reference rectangles at 125 %: fixtures/reference/ui/excel-ribbon.json).
    let tabs = [
        ("Insert", 153, 66),
        ("Draw", 220, 63),
        ("Page Layout", 284, 118),
        ("Formulas", 403, 93),
        ("Data", 497, 59),
        ("Review", 557, 78),
        ("View", 636, 60),
    ];
    let regions: Vec<Item> = tabs
        .iter()
        .map(|(t, x, w)| Item {
            text: t,
            x0: *x,
            y0: 66,
            w: *w,
            h: 28,
        })
        .collect();
    SIZE_BITS.store(17.5f32.to_bits(), Ordering::Relaxed);
    for (h, r) in [(Hinting::Light, 0), (Hinting::Normal, 0)] {
        ROUND.store(r, Ordering::Relaxed);
        fit(
            "tab labels (light ink, grayscale)",
            &cap,
            &gpu,
            &mut assets,
            font,
            &regions,
            Color::rgb(9, 9, 9),
            Color::WHITE,
            false,
            h,
        );
    }

    // Cell text: rows below the header, column C. Find each row's ink bounding box.
    let words = [
        "Home",
        "Insert",
        "Review",
        "Formulas",
        "Layout",
        "Quick brown fox",
        "Segoe UI text",
        "Data Table",
    ];
    let mut cells = Vec::new();
    SIZE_BITS.store(15.0f32.to_bits(), Ordering::Relaxed);
    let mut y = 364usize;
    for word in words {
        let (mut top, mut bottom, mut left, mut right) = (usize::MAX, 0usize, usize::MAX, 0usize);
        for yy in y..y + 24 {
            for x in 200..330 {
                let p = cap.at(x, yy);
                if p.iter().any(|&v| v < 150) {
                    top = top.min(yy);
                    bottom = bottom.max(yy);
                    left = left.min(x);
                    right = right.max(x);
                }
            }
        }
        if top != usize::MAX {
            cells.push(Item {
                text: word,
                x0: left.saturating_sub(4),
                y0: top.saturating_sub(4),
                w: right - left + 9,
                h: bottom - top + 9,
            });
        }
        y += 24;
    }
    println!("cell rows found: {}", cells.len());
    for (h, r) in [(Hinting::Light, 0), (Hinting::Normal, 0)] {
        ROUND.store(r, Ordering::Relaxed);
        fit(
            "cell text (dark ink, LCD)",
            &cap,
            &gpu,
            &mut assets,
            font,
            &cells,
            Color::WHITE,
            Color::BLACK,
            true,
            h,
        );
    }
}
