//! Icons: vendored Tabler SVGs (MIT) rasterised on demand and tinted at draw time.

use std::collections::HashMap;

use egui::{Color32, Context, Rect, TextureHandle, TextureOptions, pos2};
use resvg::{tiny_skia, usvg};

use crate::icon_table::ICONS;

/// A cache of rasterised icons keyed by (name, pixel size).
#[derive(Default)]
pub struct Icons {
    cache: HashMap<(String, u32), Option<TextureHandle>>,
}

impl std::fmt::Debug for Icons {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Icons")
            .field("cached", &self.cache.len())
            .finish()
    }
}

fn render(name: &str, px: u32) -> Option<egui::ColorImage> {
    let (_, svg) = ICONS.iter().find(|(n, _)| *n == name)?;
    // Icons are drawn white and tinted when painted, so one texture serves every colour.
    let svg = svg.replace("currentColor", "#ffffff");
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(px, px)?;
    let k = px as f32 / tree.size().width();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(k, k),
        &mut pixmap.as_mut(),
    );
    Some(egui::ColorImage::from_rgba_premultiplied(
        [px as usize, px as usize],
        pixmap.data(),
    ))
}

impl Icons {
    /// Draws icon `name` into `rect` tinted `tint`. Returns false when the icon is unknown.
    pub fn paint(
        &mut self,
        ctx: &Context,
        painter: &egui::Painter,
        name: &str,
        rect: Rect,
        tint: Color32,
    ) -> bool {
        let px = (rect.width() * ctx.pixels_per_point()).round().max(1.0) as u32;
        let entry = self.cache.entry((name.to_owned(), px)).or_insert_with(|| {
            render(name, px).map(|img| {
                ctx.load_texture(format!("icon-{name}-{px}"), img, TextureOptions::LINEAR)
            })
        });
        match entry {
            Some(tex) => {
                painter.image(
                    tex.id(),
                    rect,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    tint,
                );
                true
            }
            None => false,
        }
    }
}
