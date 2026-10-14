//! `cargo xtask icons`: renders `assets/icons/*.svg` to PNGs and multi-size `.ico` files.

use std::fs;
use std::path::{Path, PathBuf};

use resvg::{tiny_skia, usvg};

/// Sizes embedded in each `.ico` (16 to 256).
const ICO_SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
/// Application icons: SVG stem, output stem.
const APPS: [&str; 2] = ["open-write", "open-sheet"];

fn render(svg: &str, size: u32) -> Result<Vec<u8>, String> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).map_err(|e| e.to_string())?;
    let mut pixmap = tiny_skia::Pixmap::new(size, size).ok_or("pixmap")?;
    let scale = size as f32 / tree.size().width();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().map_err(|e| e.to_string())
}

/// An `.ico` whose entries are PNG images (valid since Windows Vista).
fn ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, png) in images {
        let dim = if *size >= 256 { 0 } else { *size as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0, 1, 0, 32, 0]);
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in images {
        out.extend_from_slice(png);
    }
    out
}

pub fn generate() -> Result<(), String> {
    let dir: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/icons");
    for stem in APPS {
        let svg = fs::read_to_string(dir.join(format!("{stem}.svg"))).map_err(|e| e.to_string())?;
        let mut images = Vec::new();
        for size in ICO_SIZES {
            let png = render(&svg, size)?;
            if matches!(size, 32 | 256) {
                fs::write(dir.join(format!("{stem}-{size}.png")), &png)
                    .map_err(|e| e.to_string())?;
            }
            images.push((size, png));
        }
        fs::write(dir.join(format!("{stem}.ico")), ico(&images)).map_err(|e| e.to_string())?;
        println!("wrote {stem}.ico and PNGs");
    }
    Ok(())
}
