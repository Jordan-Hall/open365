//! The headless test harness that replaces `egui_kittest` during the migration: a [`Ui`] with a
//! deterministic font and settings, simulated pointer and keyboard, rendering to an offscreen
//! `open365-gfx` target, pixel queries, image diffs and AccessKit queries.

use std::sync::Arc;
use std::time::{Duration, Instant};

use accesskit::{Node, NodeId as AkId, Role, TreeUpdate};
use open365_gfx::{Assets, Color, DrawList, GpuContext, Offscreen, RenderParams, Renderer};

use crate::geom::{Point, Rect};
use crate::input::{ImeInput, InputEvent, Key, Modifiers, PointerButton};
use crate::paint::PaintLayers;
use crate::settings::PlatformSettings;
use crate::theme::Token;
use crate::tree::NodeId;
use crate::ui::Ui;

const CARLITO: &[u8] = include_bytes!("../../../assets/fonts/Carlito-Regular.ttf");

/// A rendered frame, tightly packed sRGB RGBA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// How two images differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Diff {
    pub pixels: usize,
    /// Pixels whose largest channel difference exceeds `tolerance`.
    pub mismatched: usize,
    pub max_delta: u8,
}

impl Image {
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }

    /// Compares with `other` (same size); differences up to `tolerance` per channel are ignored.
    pub fn diff(&self, other: &Image, tolerance: u8) -> Diff {
        assert_eq!(
            (self.width, self.height),
            (other.width, other.height),
            "image sizes differ"
        );
        let mut d = Diff {
            pixels: (self.width * self.height) as usize,
            ..Diff::default()
        };
        for (a, b) in self.rgba.chunks_exact(4).zip(other.rgba.chunks_exact(4)) {
            let m = (0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap_or(0);
            d.max_delta = d.max_delta.max(m);
            if m > tolerance {
                d.mismatched += 1;
            }
        }
        d
    }

    /// Number of pixels equal to `color` (alpha ignored).
    pub fn count(&self, color: Color) -> usize {
        self.rgba
            .chunks_exact(4)
            .filter(|p| p[0] == color.r && p[1] == color.g && p[2] == color.b)
            .count()
    }
}

/// A [`Ui`] plus a virtual clock and, when the machine has a GPU adapter, an offscreen renderer.
pub struct Harness {
    pub ui: Ui,
    gpu: Option<(GpuContext, Renderer)>,
    size: (u32, u32),
    start: Instant,
    elapsed: Duration,
}

impl std::fmt::Debug for Harness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Harness")
            .field("size", &self.size)
            .field("gpu", &self.gpu.is_some())
            .finish()
    }
}

impl Harness {
    /// A window of `w` by `h` logical units at `scale`, with the bundled Carlito font and the
    /// default (not the machine's) platform settings, so tests are deterministic.
    pub fn new(w: f32, h: f32, scale: f32) -> Self {
        let mut assets = Assets::new(2048);
        let font = assets
            .add_font(Arc::from(CARLITO), 0)
            .expect("bundled font");
        let start = Instant::now();
        let mut ui = Ui::new(Rect::new(0.0, 0.0, w, h), scale, assets, font, start);
        ui.set_settings(PlatformSettings::default());
        let size = ((w * scale).round() as u32, (h * scale).round() as u32);
        let gpu = GpuContext::headless().ok().map(|g| {
            let r = Renderer::new(
                &g.device,
                open365_gfx::OFFSCREEN_FORMAT,
                ui.assets.atlas_size(),
            );
            (g, r)
        });
        Self {
            ui,
            gpu,
            size,
            start,
            elapsed: Duration::ZERO,
        }
    }

    pub fn has_gpu(&self) -> bool {
        self.gpu.is_some()
    }

    pub fn size_px(&self) -> (u32, u32) {
        self.size
    }

    pub fn now(&self) -> Instant {
        self.start + self.elapsed
    }

    /// Moves the virtual clock forward and fires timers.
    pub fn advance(&mut self, d: Duration) {
        self.elapsed += d;
        let now = self.now();
        self.ui.handle(InputEvent::Tick(now));
    }

    pub fn move_to(&mut self, x: f32, y: f32) {
        self.ui.handle(InputEvent::PointerMoved(Point::new(x, y)));
    }

    pub fn press(&mut self, x: f32, y: f32) {
        self.ui.handle(InputEvent::PointerDown(
            PointerButton::Primary,
            Point::new(x, y),
        ));
    }

    pub fn release(&mut self, x: f32, y: f32) {
        self.ui.handle(InputEvent::PointerUp(
            PointerButton::Primary,
            Point::new(x, y),
        ));
    }

    /// Move, press and release at the centre of `id`.
    pub fn click_node(&mut self, id: NodeId) {
        let c = self.ui.tree.abs_rect(id).center();
        self.click(c.x, c.y);
    }

    pub fn click(&mut self, x: f32, y: f32) {
        self.move_to(x, y);
        self.press(x, y);
        self.release(x, y);
    }

    pub fn key(&mut self, key: Key) {
        self.key_with(key, Modifiers::default());
    }

    pub fn key_with(&mut self, key: Key, modifiers: Modifiers) {
        self.ui.handle(InputEvent::Key {
            key,
            pressed: true,
            modifiers,
        });
        self.ui.handle(InputEvent::Key {
            key,
            pressed: false,
            modifiers,
        });
    }

    pub fn alt_tap(&mut self) {
        let m = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        self.ui.handle(InputEvent::Key {
            key: Key::Alt,
            pressed: true,
            modifiers: m,
        });
        self.ui.handle(InputEvent::Key {
            key: Key::Alt,
            pressed: false,
            modifiers: Modifiers::default(),
        });
    }

    pub fn type_text(&mut self, s: &str) {
        self.ui.handle(InputEvent::Text(s.to_owned()));
    }

    pub fn ime(&mut self, i: ImeInput) {
        self.ui.handle(InputEvent::Ime(i));
    }

    /// The first node with this label.
    pub fn find(&self, label: &str) -> Option<NodeId> {
        self.ui
            .tree
            .paint_order()
            .into_iter()
            .find(|id| self.ui.tree.get(*id).is_some_and(|n| n.label == label))
    }

    /// The accessibility tree as it would be sent to a screen reader.
    pub fn access(&mut self) -> TreeUpdate {
        self.ui.accessibility_tree()
    }

    /// Renders the UI into an offscreen target. `None` when the machine has no GPU adapter.
    pub fn render(&mut self, layers: PaintLayers) -> Option<Image> {
        let (w, h) = self.size;
        let mut list = DrawList::new(w, h);
        self.ui.assets.begin_frame();
        self.ui.paint(&mut list, layers);
        let (gpu, renderer) = self.gpu.as_mut()?;
        renderer.set_params(RenderParams::default());
        let target = Offscreen::new(&gpu.device, w, h);
        let clear = self.ui.theme.color(Token::Frame);
        renderer
            .render(
                &gpu.device,
                &gpu.queue,
                &mut self.ui.assets,
                &list,
                target.view(),
                Some(clear),
            )
            .ok()?;
        let rgba = target.read_rgba(&gpu.device, &gpu.queue).ok()?;
        Some(Image {
            width: w,
            height: h,
            rgba,
        })
    }

    /// Renders an arbitrary draw list built by `build` over a cleared target (for reference
    /// images in tests).
    pub fn render_with(
        &mut self,
        clear: Color,
        build: impl FnOnce(&mut DrawList),
    ) -> Option<Image> {
        let (w, h) = self.size;
        let mut list = DrawList::new(w, h);
        build(&mut list);
        let (gpu, renderer) = self.gpu.as_mut()?;
        let target = Offscreen::new(&gpu.device, w, h);
        renderer
            .render(
                &gpu.device,
                &gpu.queue,
                &mut self.ui.assets,
                &list,
                target.view(),
                Some(clear),
            )
            .ok()?;
        let rgba = target.read_rgba(&gpu.device, &gpu.queue).ok()?;
        Some(Image {
            width: w,
            height: h,
            rgba,
        })
    }
}

/// Finds a node of `update` by role and label.
pub fn find_access<'a>(
    update: &'a TreeUpdate,
    role: Role,
    label: &str,
) -> Option<(AkId, &'a Node)> {
    update
        .nodes
        .iter()
        .find(|(_, n)| n.role() == role && n.label() == Some(label))
        .map(|(id, n)| (*id, n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_images_have_no_diff() {
        let a = Image {
            width: 2,
            height: 1,
            rgba: vec![1, 2, 3, 255, 4, 5, 6, 255],
        };
        assert_eq!(
            a.diff(&a, 0),
            Diff {
                pixels: 2,
                mismatched: 0,
                max_delta: 0
            }
        );
        let mut b = a.clone();
        b.rgba[0] = 10;
        let d = a.diff(&b, 3);
        assert_eq!((d.mismatched, d.max_delta), (1, 9));
        assert_eq!(a.diff(&b, 9).mismatched, 0);
    }

    #[test]
    fn virtual_clock_drives_timers() {
        let mut h = Harness::new(100.0, 50.0, 1.0);
        let t0 = h.now();
        h.advance(Duration::from_millis(250));
        assert_eq!(h.now() - t0, Duration::from_millis(250));
    }
}
