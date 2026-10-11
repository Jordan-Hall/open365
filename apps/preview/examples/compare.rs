//! Scores the preview chrome against an Office capture, per region.
//!
//! `cargo run -p open365-preview --release --example compare -- <sheets|writer> <capture.png> [dark|light] [diff.png]`
//!
//! The capture comes from `tools/office/capture-chrome.ps1` (1600 x 1000 at 125 %) and stays out of
//! the repository. With a fourth argument the absolute difference is written as a PNG.

use open365_preview::compare::{Frame, compare, format_scores};
use open365_preview::headless::Options;
use open365_preview::{AppKind, Preview};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let kind = match a.get(1).map(String::as_str) {
        Some("writer") => AppKind::Writer,
        _ => AppKind::Sheets,
    };
    let Some(cap_path) = a.get(2) else {
        eprintln!("usage: compare <sheets|writer> <capture.png> [dark|light] [diff.png]");
        return;
    };
    let dark = a.get(3).map(String::as_str) != Some("light");
    let img = image::open(cap_path).expect("capture png").to_rgba8();
    let capture = Frame {
        width: img.width(),
        height: img.height(),
        rgba: img.into_raw(),
    };
    let mut o = Options::new(dark);
    // Office's own window keeps an invisible frame around its pane (measured: pane at 9, 1).
    o.inset = (9, 1, 9, 9);
    if std::env::var_os("OPEN365_COMPARE_PLAIN").is_none() {
        o.text = open365_gfx::TextOptions::office(false);
        o.params = open365_gfx::RenderParams::office_fitted();
    }
    let mut p = Preview::with_options(kind, capture.width, capture.height, 1.25, o);
    let Some(snap) = p.render() else {
        eprintln!("no GPU adapter");
        return;
    };
    let frame = Frame {
        width: snap.width,
        height: snap.height,
        rgba: snap.rgba,
    };
    print!("{}", format_scores(&compare(kind, &frame, &capture)));
    if let Some(out) = a.get(4) {
        let mut diff = vec![255u8; frame.rgba.len()];
        for (i, d) in diff.chunks_mut(4).enumerate() {
            for (c, v) in d.iter_mut().enumerate().take(3) {
                *v = frame.rgba[i * 4 + c]
                    .abs_diff(capture.rgba[i * 4 + c])
                    .saturating_mul(3);
            }
        }
        image::save_buffer(
            format!("{out}.frame.png"),
            &frame.rgba,
            frame.width,
            frame.height,
            image::ColorType::Rgba8,
        )
        .expect("write frame");
        image::save_buffer(
            out,
            &diff,
            frame.width,
            frame.height,
            image::ColorType::Rgba8,
        )
        .expect("write diff");
    }
}
