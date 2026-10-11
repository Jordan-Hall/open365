//! Fits text rendering (advances, hinting, LCD filter, gamma) to what Office draws.
//!
//! Input is a `.raw` capture from `tools/office/capture-chrome.ps1 -TextProbes` (8-byte header,
//! then RGBA; kept in a scratch folder, never in the repository). Only the derived numbers are
//! kept.
//!
//! ```text
//! OPEN365_FIT_CAPTURE=<scratch>/probe/excel-textprobe.raw cargo run -p open365-gfx --example fit_text --release
//! ```
//!
//! The probe sheet holds black Segoe UI text on white (Office was in the dark theme, so the chrome
//! is dark but the grid and the text box are light surfaces):
//!
//! * text box, 11 pt (18.33 px at 125 %): drawn in the grayscale drawing layer;
//! * cells B2:B3, 11 pt, and B4:B5, 9 pt (15 px): ClearType.
//!
//! For every combination of hinting, advance rounding and LCD filter it aligns each line to the
//! capture (origin x in quarter pixels, integer baseline), fits the dark-ink gamma and reports the
//! RMS in levels.

use std::sync::Arc;

use open365_gfx::{
    Assets, Color, DrawList, GpuContext, Hinting, LcdFilter, Offscreen, RenderParams, Renderer,
    TextLayoutOptions, TextOptions,
};

struct Cap {
    w: usize,
    rgb: Vec<[u8; 3]>,
}

impl Cap {
    fn load(path: &str) -> Self {
        let raw = std::fs::read(path).expect("capture file");
        let w = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
        let rgb = raw[8..]
            .chunks_exact(4)
            .map(|p| [p[0], p[1], p[2]])
            .collect();
        Self { w, rgb }
    }

    fn at(&self, x: usize, y: usize) -> [u8; 3] {
        self.rgb[y * self.w + x]
    }
}

/// One line of text and the capture rectangle that holds it (padded ink box).
struct Line {
    text: &'static str,
    size_px: f32,
    light: bool,
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
}

/// Padded ink bounding box of dark pixels inside `search` = (x0, y0, x1, y1).
fn ink_box(
    cap: &Cap,
    search: (usize, usize, usize, usize),
    light: bool,
) -> Option<(usize, usize, usize, usize)> {
    let (mut l, mut t, mut r, mut b) = (usize::MAX, usize::MAX, 0, 0);
    for y in search.1..search.3 {
        for x in search.0..search.2 {
            if cap
                .at(x, y)
                .iter()
                .any(|&v| if light { v > 110 } else { v < 150 })
            {
                l = l.min(x);
                t = t.min(y);
                r = r.max(x);
                b = b.max(y);
            }
        }
    }
    (l != usize::MAX).then_some((
        l.saturating_sub(4),
        t.saturating_sub(4),
        r - l + 9,
        b - t + 9,
    ))
}

#[derive(Clone, Copy)]
struct Config {
    hinting: Hinting,
    round: bool,
    lcd: bool,
    filter: LcdFilter,
}

struct Rig {
    gpu: GpuContext,
    assets: Assets,
    font: open365_gfx::FontId,
    renderer: Renderer,
}

impl Rig {
    /// Renders `line` at origin `(dx, base)` in item-local pixels; returns RGBA.
    fn render(&mut self, line: &Line, c: Config, dx: f32, base: f32) -> Vec<u8> {
        self.assets.set_text_options(TextOptions {
            hinting: c.hinting,
            subpixel_bins: 4,
            lcd: c.lcd,
            lcd_filter: c.filter,
        });
        let mut list = DrawList::new(line.w as u32, line.h as u32);
        let lay = self.assets.layout_text(
            self.font,
            line.text,
            line.size_px,
            dx,
            base,
            TextLayoutOptions {
                round_advances: c.round,
            },
        );
        let ink = if line.light {
            Color::WHITE
        } else {
            Color::BLACK
        };
        list.text(
            &mut self.assets,
            self.font,
            &lay,
            line.size_px,
            ink,
            Default::default(),
        );
        let target = Offscreen::new(&self.gpu.device, line.w as u32, line.h as u32);
        self.renderer
            .render(
                &self.gpu.device,
                &self.gpu.queue,
                &mut self.assets,
                &list,
                target.view(),
                Some(if line.light {
                    Color::rgb(9, 9, 9)
                } else {
                    Color::WHITE
                }),
            )
            .expect("render");
        target
            .read_rgba(&self.gpu.device, &self.gpu.queue)
            .expect("readback")
    }
}

fn sse(cap: &Cap, line: &Line, px: &[u8], all: bool) -> f64 {
    let mut e = 0.0;
    for y in 0..line.h {
        for x in 0..line.w {
            let c = cap.at(line.x0 + x, line.y0 + y);
            let i = (y * line.w + x) * 4;
            for k in 0..(if all { 3 } else { 1 }) {
                let d = f64::from(c[k]) - f64::from(px[i + k]);
                e += d * d;
            }
        }
    }
    e
}

/// Best origin for one line under `c`, searching `dxs` x `bases`.
fn align(
    rig: &mut Rig,
    cap: &Cap,
    line: &Line,
    c: Config,
    dxs: &[f32],
    bases: &[f32],
) -> (f64, f32, f32) {
    let mut best = (f64::MAX, 0.0, 0.0);
    for &dx in dxs {
        for &b in bases {
            let px = rig.render(line, c, dx, b);
            let e = sse(cap, line, &px, c.lcd);
            if e < best.0 {
                best = (e, dx, b);
            }
        }
    }
    best
}

fn run_group(
    rig: &mut Rig,
    cap: &Cap,
    name: &str,
    lines: &[Line],
    lcd: bool,
    filters: &[LcdFilter],
) {
    println!("== {name}");
    let pixels: usize = lines.iter().map(|l| l.w * l.h).sum();
    let ch = if lcd { 3.0 } else { 1.0 };
    for hinting in [Hinting::None, Hinting::Light, Hinting::Normal] {
        for round in [false, true] {
            for &filter in filters {
                let c = Config {
                    hinting,
                    round,
                    lcd,
                    filter,
                };
                let mut total = 0.0;
                let mut origins = Vec::new();
                rig.renderer.set_params(RenderParams::default());
                for line in lines {
                    let coarse: Vec<f32> = (0..10).map(|v| v as f32).collect();
                    let bases: Vec<f32> = (6..line.h as i32 - 2).map(|v| v as f32).collect();
                    let (_, cx, cy) = align(rig, cap, line, c, &coarse, &bases);
                    let fine: Vec<f32> = (-4..=4).map(|v| cx + v as f32 * 0.25).collect();
                    let (e, fx, fy) = align(rig, cap, line, c, &fine, &[cy]);
                    total += e;
                    origins.push((fx, fy));
                }
                let identity = (total / (pixels as f64 * ch)).sqrt();
                // Gamma fit with the aligned origins.
                let mut best = (f64::MAX, 1.0f32);
                for gi in 0..=16 {
                    let g = 0.4 + gi as f32 * 0.2;
                    rig.renderer.set_params(if lines[0].light {
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
                    let mut e = 0.0;
                    for (line, (fx, fy)) in lines.iter().zip(&origins) {
                        let px = rig.render(line, c, *fx, *fy);
                        e += sse(cap, line, &px, lcd);
                    }
                    if e < best.0 {
                        best = (e, g);
                    }
                }
                println!(
                    "{hinting:<7?} round={:<5} filter={filter:<7?} identity {identity:6.2}  gamma {:.1} -> rms {:6.2}",
                    round,
                    best.1,
                    (best.0 / (pixels as f64 * ch)).sqrt()
                );
            }
        }
    }
}

fn main() {
    let cap = Cap::load(&std::env::var("OPEN365_FIT_CAPTURE").expect("set OPEN365_FIT_CAPTURE"));
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    let bytes = std::fs::read(format!("{windir}\\Fonts\\segoeui.ttf")).expect("Segoe UI");
    let gpu = GpuContext::headless().expect("adapter");
    let mut assets = Assets::new(1024);
    let font = assets.add_font(Arc::from(bytes), 0).expect("font");
    let renderer = Renderer::new(
        &gpu.device,
        open365_gfx::OFFSCREEN_FORMAT,
        assets.atlas_size(),
    );
    let mut rig = Rig {
        gpu,
        assets,
        font,
        renderer,
    };

    const L1: &str = "The quick brown fox jumps over the lazy dog 0123456789";
    const L2: &str = "Hamburgefonstiv, Rhythm and Blues";
    let mk = |cap: &Cap, text, size_px, area: (usize, usize, usize, usize)| {
        ink_box(cap, area, false).map(|(x0, y0, w, h)| Line {
            text,
            size_px,
            light: false,
            x0,
            y0,
            w,
            h,
        })
    };
    let size11 = 11.0 * 96.0 / 72.0 * 1.25;
    let size9 = 9.0 * 96.0 / 72.0 * 1.25;
    let tb: Vec<Line> = [
        mk(&cap, L1, size11, (715, 400, 1300, 424)),
        mk(&cap, L2, size11, (715, 425, 1300, 448)),
    ]
    .into_iter()
    .flatten()
    .collect();
    let c11: Vec<Line> = [
        mk(&cap, L1, 18.0, (125, 369, 640, 393)),
        mk(&cap, L1, 18.0, (125, 397, 640, 421)),
    ]
    .into_iter()
    .flatten()
    .collect();
    let c9: Vec<Line> = [
        mk(&cap, L1, size9, (125, 427, 640, 445)),
        mk(&cap, L1, size9, (125, 451, 640, 469)),
    ]
    .into_iter()
    .flatten()
    .collect();
    println!(
        "lines found: text box {}, cells 11 pt {}, cells 9 pt {}",
        tb.len(),
        c11.len(),
        c9.len()
    );
    run_group(
        &mut rig,
        &cap,
        "text box 11 pt, black on white, grayscale",
        &tb,
        false,
        &[LcdFilter::Default],
    );
    let filters = [
        LcdFilter::Default,
        LcdFilter::Light,
        LcdFilter::Soft,
        LcdFilter::None,
    ];
    run_group(
        &mut rig,
        &cap,
        "cells 11 pt, black on white, ClearType",
        &c11,
        true,
        &filters,
    );
    if let Ok(home) = std::env::var("OPEN365_FIT_HOME") {
        let home = Cap::load(&home);
        let tabs = [
            ("Insert", 160, 215),
            ("Draw", 225, 280),
            ("Page Layout", 288, 400),
            ("Formulas", 410, 495),
            ("Review", 562, 630),
            ("View", 640, 692),
        ];
        let size: f32 = std::env::var("OPEN365_TAB_PX")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(14.0 * 1.25);
        let lines: Vec<Line> = tabs
            .iter()
            .filter_map(|(t, a, b)| {
                ink_box(&home, (*a, 68, *b, 94), true).map(|(x0, y0, w, h)| Line {
                    text: t,
                    size_px: size,
                    light: true,
                    x0,
                    y0,
                    w,
                    h,
                })
            })
            .collect();
        println!("tab labels found: {}", lines.len());
        run_group(
            &mut rig,
            &home,
            "tab labels 14 dip, white on 090909, grayscale",
            &lines,
            false,
            &[LcdFilter::Default],
        );
    }
    run_group(
        &mut rig,
        &cap,
        "cells 9 pt, black on white, ClearType",
        &c9,
        true,
        &filters,
    );
}
