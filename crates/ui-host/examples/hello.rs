//! Smoke test for the host: a panel, a rounded button, an SVG icon, shaped text in Segoe UI (or
//! the bundled fallback), a blinking caret, IME composition and an AccessKit tree.
//!
//! Run with `cargo run -p open365-ui-host --example hello`. Type to edit; the window follows the
//! system light/dark mode, including the title bar.

use std::collections::HashMap;
use std::sync::Arc;

use open365_gfx::{Assets, Color, FontId, GlyphPos, GlyphRun, IconId, Rect};
use open365_text::{FaceId, FontLibrary, FontStyle, ShapeFeatures, shape_run};
use open365_ui_host::accesskit::{Node, NodeId, Rect as AkRect, Role, Tree, TreeId, TreeUpdate};
use open365_ui_host::winit::event::{ElementState, WindowEvent};
use open365_ui_host::winit::keyboard::{Key, NamedKey};
use open365_ui_host::{
    CaretBlink, ColorScheme, Cx, Frame, HostConfig, ImeCursorArea, ImeHandler, UiApp, UiEvent, run,
};

const ROOT: NodeId = NodeId(0);
const EDIT: NodeId = NodeId(1);

/// A stand-in file icon in the Lucide/Tabler stroke style; the real sets arrive as bytes later.
const ICON_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"
fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
<path d="M14 3v4a1 1 0 0 0 1 1h4"/><path d="M17 21H7a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h7l5 5v11a2 2 0 0 1-2 2z"/>
<path d="M9 13h6"/><path d="M9 17h6"/></svg>"#;

struct Palette {
    window: Color,
    panel: Color,
    border: Color,
    text: Color,
    accent: Color,
    accent_text: Color,
}

impl Palette {
    fn for_scheme(s: ColorScheme) -> Self {
        match s {
            ColorScheme::Light => Self {
                window: Color::rgb(243, 243, 243),
                panel: Color::WHITE,
                border: Color::rgb(209, 209, 209),
                text: Color::rgb(36, 36, 36),
                accent: Color::rgb(15, 108, 189),
                accent_text: Color::WHITE,
            },
            ColorScheme::Dark => Self {
                window: Color::rgb(32, 32, 32),
                panel: Color::rgb(41, 41, 41),
                border: Color::rgb(66, 66, 66),
                text: Color::rgb(255, 255, 255),
                accent: Color::rgb(71, 158, 245),
                accent_text: Color::rgb(0, 0, 0),
            },
        }
    }
}

struct Hello {
    lib: FontLibrary,
    fonts: HashMap<usize, FontId>,
    icon: Option<IconId>,
    text: String,
    preedit: String,
    blink: CaretBlink,
    caret: ImeCursorArea,
    scale: f64,
}

impl Hello {
    fn new() -> Self {
        let mut lib = FontLibrary::bundled();
        lib.add_system_family("Segoe UI");
        Self {
            lib,
            fonts: HashMap::new(),
            icon: None,
            text: "Hello, Open365".into(),
            preedit: String::new(),
            blink: CaretBlink::new(None, std::time::Instant::now()),
            caret: ImeCursorArea {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 16.0,
            },
            scale: 1.0,
        }
    }

    fn font_for(&mut self, assets: &mut Assets, face: FaceId) -> Option<FontId> {
        if let Some(f) = self.fonts.get(&face.index()) {
            return Some(*f);
        }
        let f = self.lib.face(face);
        let id = assets
            .add_font(Arc::from(f.bytes().to_vec()), f.index())
            .ok()?;
        self.fonts.insert(face.index(), id);
        Some(id)
    }

    /// Draws `text` with its left edge at `x` and top at `y` (device pixels); returns the end x.
    fn draw_text(
        &mut self,
        frame: &mut Frame<'_>,
        text: &str,
        x: f32,
        y: f32,
        size_px: f32,
        color: Color,
    ) -> f32 {
        let chain = self.lib.chain("Segoe UI", FontStyle::default());
        let run = shape_run(&self.lib, &chain, text, ShapeFeatures::default());
        let Some(first) = run.glyphs.first().map(|g| g.face) else {
            return x;
        };
        let baseline = y
            + (self.lib.face(first).metrics().hhea_ascent as f32 * size_px
                / f32::from(self.lib.face(first).units_per_em()))
            .round();
        let mut pen = x;
        // Group consecutive glyphs of the same face into one run each.
        let mut groups: Vec<(FaceId, Vec<GlyphPos>)> = Vec::new();
        for g in &run.glyphs {
            let upem = f32::from(self.lib.face(g.face).units_per_em());
            let pos = GlyphPos {
                id: g.glyph_id,
                x: pen + g.x_offset as f32 * size_px / upem,
                y: baseline,
            };
            pen += g.x_advance as f32 * size_px / upem;
            match groups.last_mut() {
                Some((f, v)) if *f == g.face => v.push(pos),
                _ => groups.push((g.face, vec![pos])),
            }
        }
        for (face, glyphs) in groups {
            if let Some(font) = self.font_for(frame.assets, face) {
                frame.list.glyph_run(
                    frame.assets,
                    &GlyphRun {
                        font,
                        size_px,
                        glyphs: &glyphs,
                    },
                    color,
                );
            }
        }
        pen
    }

    fn tree(&self, scale: f64) -> TreeUpdate {
        let mut root = Node::new(Role::Window);
        root.set_label("Open365 host demo");
        root.set_children(vec![EDIT]);
        let mut edit = Node::new(Role::TextInput);
        edit.set_label("Document text");
        edit.set_value(self.text.as_str());
        edit.set_bounds(AkRect {
            x0: 24.0 * scale,
            y0: 72.0 * scale,
            x1: 424.0 * scale,
            y1: 108.0 * scale,
        });
        TreeUpdate {
            nodes: vec![(ROOT, root), (EDIT, edit)],
            tree: Some(Tree::new(ROOT)),
            tree_id: TreeId::ROOT,
            focus: EDIT,
        }
    }
}

impl UiApp for Hello {
    fn init(&mut self, cx: &mut Cx<'_>, assets: &mut Assets) {
        self.icon = assets.add_icon(ICON_SVG).ok();
        self.blink = CaretBlink::new(cx.metrics.caret_blink, cx.now);
        self.scale = cx.scale_factor;
    }

    fn event(&mut self, cx: &mut Cx<'_>, event: &UiEvent<'_>) {
        match event {
            UiEvent::Tick(_) | UiEvent::Theme(_) => cx.request_redraw(),
            UiEvent::Window(WindowEvent::KeyboardInput { event: key, .. })
                if key.state == ElementState::Pressed =>
            {
                match &key.logical_key {
                    Key::Named(NamedKey::Backspace) => {
                        self.text.pop();
                    }
                    _ => {
                        if let Some(t) = key
                            .text
                            .as_ref()
                            .filter(|t| t.chars().all(|c| !c.is_control()))
                        {
                            self.text.push_str(t);
                        }
                    }
                }
                self.blink.restart(cx.now);
                let scale = cx.scale_factor;
                cx.update_accessibility(|| self.tree(scale));
                cx.request_redraw();
            }
            UiEvent::Window(WindowEvent::ScaleFactorChanged { scale_factor, .. }) => {
                self.scale = *scale_factor;
            }
            _ => {}
        }
    }

    fn paint(&mut self, cx: &mut Cx<'_>, frame: &mut Frame<'_>) {
        let s = cx.scale_factor as f32;
        let pal = Palette::for_scheme(cx.scheme);
        let (w, h) = (frame.size_px.0 as f32, frame.size_px.1 as f32);
        frame
            .list
            .fill_rect(Rect::from_xywh(0.0, 0.0, w, h), pal.window);

        // Text box: a 1 px border drawn inside the panel.
        let panel = Rect::from_xywh(24.0 * s, 72.0 * s, 400.0 * s, 36.0 * s);
        frame.list.fill_rect(panel, pal.panel);
        frame.list.rect_outline(panel, 1.0, pal.border);

        let size_px = (12.0 * s).round();
        let text_x = panel.x0 + 8.0 * s;
        let text_y = panel.y0 + ((panel.height() - size_px * 1.33) / 2.0).round();
        let text = self.text.clone();
        let mut end = self.draw_text(frame, &text, text_x, text_y, size_px, pal.text);
        if !self.preedit.is_empty() {
            let pre = self.preedit.clone();
            let start = end;
            end = self.draw_text(frame, &pre, end, text_y, size_px, pal.text);
            frame
                .list
                .hline(text_y + size_px * 1.3, start, end, 1.0, pal.text);
        }

        // Caret.
        if self.blink.visible(cx.now) {
            let cw = cx.metrics.caret_width_px as f32;
            frame.list.fill_rect(
                Rect::from_xywh(end.round(), text_y, cw, (size_px * 1.3).round()),
                pal.text,
            );
        }
        if let Some(at) = self.blink.next_toggle(cx.now) {
            cx.wake_at(at);
        }
        self.caret = ImeCursorArea {
            x: f64::from(end) / cx.scale_factor,
            y: f64::from(text_y) / cx.scale_factor,
            width: 1.0,
            height: f64::from(size_px * 1.3) / cx.scale_factor,
        };

        // Button with an icon and a label.
        let button = Rect::from_xywh(24.0 * s, 124.0 * s, 132.0 * s, 32.0 * s);
        frame.list.fill_rrect(button, 4.0 * s, pal.accent);
        if let Some(icon) = self.icon {
            let side = (16.0 * s).round();
            frame.list.icon(
                frame.assets,
                icon,
                Rect::from_xywh(
                    button.x0 + 10.0 * s,
                    button.y0 + (button.height() - side) / 2.0,
                    side,
                    side,
                ),
                pal.accent_text,
            );
        }
        self.draw_text(
            frame,
            "New document",
            button.x0 + 34.0 * s,
            button.y0 + ((button.height() - size_px * 1.33) / 2.0).round(),
            size_px,
            pal.accent_text,
        );
        // A hairline under the header, one device pixel at any scale.
        frame
            .list
            .hline(56.0 * s, 24.0 * s, w - 24.0 * s, 1.0, pal.border);
        self.draw_text(
            frame,
            "Open365 host",
            24.0 * s,
            24.0 * s,
            (20.0 * s).round(),
            pal.text,
        );
    }

    fn ime(&mut self) -> Option<&mut dyn ImeHandler> {
        Some(self)
    }

    fn accessibility_tree(&mut self, cx: &mut Cx<'_>) -> Option<TreeUpdate> {
        Some(self.tree(cx.scale_factor))
    }
}

impl ImeHandler for Hello {
    fn wants_ime(&self) -> bool {
        true
    }

    fn cursor_area(&self) -> Option<ImeCursorArea> {
        Some(self.caret)
    }

    fn preedit(&mut self, text: &str, _cursor: Option<(usize, usize)>) {
        self.preedit = text.to_owned();
    }

    fn commit(&mut self, text: &str) {
        self.text.push_str(text);
        self.preedit.clear();
    }

    fn ime_disabled(&mut self) {
        self.preedit.clear();
    }
}

fn main() {
    let config = HostConfig {
        title: "Open365 host demo".into(),
        inner_size: (640.0, 360.0),
        ..HostConfig::default()
    };
    if let Err(e) = run(config, Hello::new()) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
