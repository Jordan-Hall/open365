//! Renders the preview chrome offscreen to a PNG for a look at the result.
//!
//! `cargo run -p open365-preview --example snapshot -- <writer|sheets> <out.png> [w] [h] [scale] [dark|light]`

use open365_preview::{AppKind, Preview};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let kind = match a.get(1).map(String::as_str) {
        Some("writer") => AppKind::Writer,
        _ => AppKind::Sheets,
    };
    let out = a.get(2).cloned().unwrap_or_else(|| "snapshot.png".into());
    let w: u32 = a.get(3).and_then(|v| v.parse().ok()).unwrap_or(1600);
    let h: u32 = a.get(4).and_then(|v| v.parse().ok()).unwrap_or(1000);
    let scale: f32 = a.get(5).and_then(|v| v.parse().ok()).unwrap_or(1.25);
    let dark = a.get(6).map(String::as_str) != Some("light");
    let mut p = Preview::new(kind, dark, w, h, scale);
    let Some(img) = p.render() else {
        eprintln!("no GPU adapter");
        return;
    };
    image::save_buffer(
        &out,
        &img.rgba,
        img.width,
        img.height,
        image::ColorType::Rgba8,
    )
    .expect("write png");
    println!("wrote {out} {}x{}", img.width, img.height);
}
