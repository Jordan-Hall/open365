//! The preview without a window: the same [`Shell`] and `Ui`, driven by simulated input and
//! rendered to an offscreen target. Tests and the pixel comparison use it.

use std::sync::Arc;
use std::time::Instant;

use open365_gfx::{
    Assets, Color, DrawList, GpuContext, Offscreen, RenderParams, Renderer, TextOptions,
};
use open365_ui_core::geom::{Point, Rect};
use open365_ui_core::{
    InputEvent, Key, Modifiers, PaintLayers, PlatformSettings, PointerButton, Ui,
};

use crate::AppKind;
use crate::fonts::{self, UiFonts};
use crate::shell::{Shell, WinAct};

/// A rendered frame: tightly packed sRGB RGBA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Snapshot {
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }
}

/// Options for [`Preview::with_options`].
#[derive(Debug, Clone)]
pub struct Options {
    pub dark: bool,
    pub fonts: UiFonts,
    pub text: TextOptions,
    pub params: RenderParams,
    /// Frame between the window edge and the pane, as in Office's own window (9, 1, 9, 9 at 125 %).
    pub inset: (i32, i32, i32, i32),
}

impl Options {
    pub fn new(dark: bool) -> Self {
        Self {
            dark,
            fonts: fonts::load(),
            text: TextOptions::default(),
            params: RenderParams::default(),
            inset: (0, 0, 0, 0),
        }
    }
}

/// A [`Shell`] on a `Ui` of fixed pixel size.
pub struct Preview {
    pub ui: Ui,
    pub shell: Shell,
    /// Window requests produced by the widget events so far.
    pub window_acts: Vec<WinAct>,
    gpu: Option<(GpuContext, Renderer)>,
    size: (u32, u32),
    params: RenderParams,
}

impl std::fmt::Debug for Preview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Preview")
            .field("size", &self.size)
            .field("gpu", &self.gpu.is_some())
            .finish_non_exhaustive()
    }
}

impl Preview {
    /// A window of `w` by `h` device pixels at `scale`, in the machine's fonts.
    pub fn new(kind: AppKind, dark: bool, w: u32, h: u32, scale: f32) -> Self {
        Self::with_options(kind, w, h, scale, Options::new(dark))
    }

    pub fn with_options(kind: AppKind, w: u32, h: u32, scale: f32, o: Options) -> Self {
        let mut assets = Assets::new(2048);
        assets.set_text_options(o.text);
        let font = assets
            .add_font(Arc::clone(&o.fonts.primary), o.fonts.index)
            .expect("UI font");
        let fallbacks = o
            .fonts
            .fallbacks
            .iter()
            .filter_map(|b| assets.add_font(Arc::clone(b), 0).ok())
            .collect();
        assets.set_fallbacks(font, fallbacks);
        let bounds = Rect::new(0.0, 0.0, w as f32 / scale, h as f32 / scale);
        let mut ui = Ui::new(bounds, scale, assets, font, Instant::now());
        ui.set_settings(PlatformSettings {
            apps_dark: o.dark,
            ..PlatformSettings::default()
        });
        let mut shell = Shell::new(kind, o.dark);
        shell.init(&mut ui);
        if o.inset != (0, 0, 0, 0) {
            shell.set_inset(&mut ui, o.inset);
        }
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
            shell,
            window_acts: Vec::new(),
            gpu,
            size: (w, h),
            params: o.params,
        }
    }

    pub fn has_gpu(&self) -> bool {
        self.gpu.is_some()
    }

    pub fn size_px(&self) -> (u32, u32) {
        self.size
    }

    pub fn scale(&self) -> f32 {
        self.ui.scale.0
    }

    /// Resizes the window to `w` by `h` device pixels and rebuilds the chrome.
    pub fn resize(&mut self, w: u32, h: u32) {
        self.size = (w, h);
        let s = self.ui.scale.0;
        let root = self.ui.tree.root();
        if let Some(r) = self.ui.tree.get_mut(root) {
            r.bounds = Rect::new(0.0, 0.0, w as f32 / s, h as f32 / s);
        }
        self.shell.resize(&mut self.ui);
    }

    fn drain(&mut self) {
        let events = self.ui.take_events();
        if !events.is_empty() {
            let acts = self.shell.handle(&mut self.ui, events);
            self.window_acts.extend(acts);
        }
    }

    /// Feeds one input event and routes the widget events to the shell.
    pub fn input(&mut self, ev: InputEvent) {
        self.ui.handle(ev);
        self.drain();
    }

    /// Pointer positions here are device pixels.
    pub fn move_to(&mut self, x: f32, y: f32) {
        let s = self.ui.scale.0;
        self.input(InputEvent::PointerMoved(Point::new(x / s, y / s)));
    }

    pub fn press(&mut self, x: f32, y: f32) {
        let s = self.ui.scale.0;
        self.input(InputEvent::PointerDown(
            PointerButton::Primary,
            Point::new(x / s, y / s),
        ));
    }

    pub fn release(&mut self, x: f32, y: f32) {
        let s = self.ui.scale.0;
        self.input(InputEvent::PointerUp(
            PointerButton::Primary,
            Point::new(x / s, y / s),
        ));
    }

    pub fn click(&mut self, x: f32, y: f32) {
        self.move_to(x, y);
        self.press(x, y);
        self.release(x, y);
    }

    pub fn key(&mut self, key: Key) {
        for pressed in [true, false] {
            self.input(InputEvent::Key {
                key,
                pressed,
                modifiers: Modifiers::default(),
            });
        }
    }

    pub fn alt_tap(&mut self) {
        let alt = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        self.input(InputEvent::Key {
            key: Key::Alt,
            pressed: true,
            modifiers: alt,
        });
        self.input(InputEvent::Key {
            key: Key::Alt,
            pressed: false,
            modifiers: Modifiers::default(),
        });
    }

    /// Builds the draw list of the current frame.
    pub fn draw_list(&mut self) -> DrawList {
        let (w, h) = self.size;
        let mut list = DrawList::new(w, h);
        self.ui.assets.begin_frame();
        self.shell.paint_below(&mut self.ui, &mut list);
        self.ui.paint(&mut list, PaintLayers::default());
        list
    }

    /// A screen reader action (as AccessKit delivers it), routed like input.
    pub fn accessibility_action(&mut self, request: &accesskit::ActionRequest) -> bool {
        let done = self.ui.accessibility_action(request);
        self.drain();
        done
    }

    /// Renders the frame offscreen; `None` without a GPU adapter.
    pub fn render(&mut self) -> Option<Snapshot> {
        let list = self.draw_list();
        let (w, h) = self.size;
        let (gpu, renderer) = self.gpu.as_mut()?;
        renderer.set_params(self.params);
        let target = Offscreen::new(&gpu.device, w, h);
        renderer
            .render(
                &gpu.device,
                &gpu.queue,
                &mut self.ui.assets,
                &list,
                target.view(),
                Some(Color::WHITE),
            )
            .ok()?;
        let rgba = target.read_rgba(&gpu.device, &gpu.queue).ok()?;
        Some(Snapshot {
            width: w,
            height: h,
            rgba,
        })
    }
}
