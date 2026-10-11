//! Pictures: inline and floating images from the layout's `Page::images` (and the header, footer
//! and footnote groups), decoded from the document package and drawn through the gfx image atlas.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use open365_gfx::{Color, DrawList, ImageId, Rect as GRect};

use super::WriterPane;
use crate::doc::parts;

/// Largest picture side decoded, in pixels, and the most memory a decode may take.
const MAX_PICTURE_SIDE: u32 = 8192;
const MAX_PICTURE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PICTURE_INPUT: usize = 64 * 1024 * 1024;
/// Pictures are scaled down to this side before they go to the atlas.
const ATLAS_SIDE: u32 = 1024;

/// Decoded pictures by part and relationship id, with a fingerprint of the bytes they came from
/// so an edited picture is decoded again and an unchanged one is not.
#[derive(Debug, Default)]
pub struct Cache {
    map: HashMap<String, (u64, Option<ImageId>)>,
}

impl Cache {
    /// Pictures decoded so far.
    pub fn len(&self) -> usize {
        self.map.len()
    }
}

fn fingerprint(bytes: &[u8]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.len().hash(&mut h);
    bytes[..bytes.len().min(2048)].hash(&mut h);
    bytes[bytes.len().saturating_sub(2048)..].hash(&mut h);
    h.finish()
}

/// Decodes a PNG, JPEG, GIF or BMP picture from untrusted bytes, scaled down to `max_side`.
/// Oversized or malformed input gives `None`; nothing here panics.
pub fn decode_picture(bytes: &[u8], max_side: u32) -> Option<(u32, u32, Vec<u8>)> {
    if bytes.len() > MAX_PICTURE_INPUT {
        return None;
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_PICTURE_SIDE);
    limits.max_image_height = Some(MAX_PICTURE_SIDE);
    limits.max_alloc = Some(MAX_PICTURE_BYTES);
    reader.limits(limits);
    let mut rgba = reader.decode().ok()?.to_rgba8();
    let longest = rgba.width().max(rgba.height());
    if max_side > 0 && longest > max_side {
        let k = f64::from(max_side) / f64::from(longest);
        let w = ((f64::from(rgba.width()) * k) as u32).max(1);
        let h = ((f64::from(rgba.height()) * k) as u32).max(1);
        rgba = image::imageops::resize(&rgba, w, h, image::imageops::FilterType::Triangle);
    }
    Some((rgba.width(), rgba.height(), rgba.into_raw()))
}

/// Which pictures of a page to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Layer {
    /// Floating pictures set behind the text.
    Behind,
    /// Everything else (inline, in front of the text, header, footer, notes).
    Front,
}

impl WriterPane {
    /// Draws the pictures of `page`. Header and footer pictures are dimmed like their text.
    pub(super) fn paint_pictures(
        &mut self,
        ui: &mut open365_ui_core::Ui,
        list: &mut DrawList,
        page: usize,
        origin: (f32, f32),
        view: (f32, f32),
        layer: Layer,
    ) {
        let s = self.s();
        let p = &self.engine.layout().pages[page];
        let groups = [
            (p.images.clone(), 1.0f32),
            (p.header_images.clone(), super::FURNITURE_DIM),
            (p.note_images.clone(), 1.0),
            (p.footer_images.clone(), super::FURNITURE_DIM),
        ];
        for (images, dim) in &groups {
            for im in images {
                let behind = im.anchored && im.behind_text;
                if behind != (layer == Layer::Behind) {
                    continue;
                }
                let x0 = origin.0 + im.x as f32 * s;
                let y0 = origin.1 + im.y as f32 * s;
                let dest =
                    GRect::from_edges(x0, y0, x0 + im.width as f32 * s, y0 + im.height as f32 * s);
                if dest.y1 < view.0 || dest.y0 > view.1 {
                    continue;
                }
                let Some(embed) = &im.embed else { continue };
                let key = parts::cache_key(&im.source, embed);
                let Some(bytes) = self
                    .target
                    .docx()
                    .and_then(|d| parts::image_bytes(d, &im.source, embed))
                else {
                    continue;
                };
                let print = fingerprint(bytes);
                let stale = self.pictures.map.get(&key).is_none_or(|(f, _)| *f != print);
                if stale {
                    let id = decode_picture(bytes, ATLAS_SIDE)
                        .and_then(|(w, h, px)| ui.assets.add_image(w, h, &px).ok());
                    self.pictures.map.insert(key.clone(), (print, id));
                }
                if let Some((_, Some(id))) = self.pictures.map.get(&key) {
                    let tint = Color::rgb(255, 255, 255).with_opacity(*dim);
                    list.image(&ui.assets, *id, dest, tint);
                    self.stats.pictures += 1;
                    self.stats.floating_pictures += usize::from(im.anchored);
                }
            }
        }
    }
}
