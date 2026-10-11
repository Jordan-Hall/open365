//! [`UiAdapter`]: a [`UiApp`] that hosts an `open365-ui-core` [`Ui`].
//!
//! The adapter owns the `Ui`, translates window events with [`InputBridge`], installs the system
//! clipboard, keeps the `Ui` scale, size and platform settings in step with the window, drives
//! timers and redraws, publishes the AccessKit tree and forwards IME composition. What the
//! application decides stays in a small [`UiHandler`]: build the widget tree, react to widget
//! events and paint the document area.

use std::sync::Arc;

use accesskit::TreeUpdate;
use open365_gfx::{Assets, DrawList, TextOptions};
use open365_ui_core::geom::{Rect, Scale};
use open365_ui_core::{Emit, InputEvent, Ui};
use winit::event::WindowEvent;

use crate::app::{Cx, Frame, UiApp, UiEvent};
use crate::bridge::InputBridge;
use crate::clipboard::SystemClipboard;
use crate::ime::{ImeCursorArea, ImeEvent, ImeHandler};
use crate::settings::{SettingsWatcher, with_scheme};
use crate::theme::ColorScheme;

/// What the application provides to a [`UiAdapter`].
pub trait UiHandler: 'static {
    /// The window and GPU exist: register icons on `ui.assets` and build the widget tree.
    fn init(&mut self, ui: &mut Ui, cx: &mut Cx<'_>);

    /// The logical size (the root node's bounds) or the scale changed: lay the tree out again.
    fn resized(&mut self, _ui: &mut Ui, _cx: &mut Cx<'_>) {}

    /// Platform settings (theme, high contrast, caret) were re-read and `ui.theme` follows them.
    fn settings_changed(&mut self, _ui: &mut Ui, _cx: &mut Cx<'_>) {}

    /// Widget events collected while handling input.
    fn emitted(&mut self, ui: &mut Ui, cx: &mut Cx<'_>, events: Vec<Emit>);

    /// Sees every window event before the widgets do. Return true to keep it from them.
    fn window_event(&mut self, _ui: &mut Ui, _cx: &mut Cx<'_>, _event: &WindowEvent) -> bool {
        false
    }

    /// Draws under the widgets (window background, document area).
    fn paint_below(&mut self, _ui: &mut Ui, _list: &mut DrawList) {}

    /// Draws over the widgets.
    fn paint_above(&mut self, _ui: &mut Ui, _list: &mut DrawList) {}

    fn close_requested(&mut self, _ui: &mut Ui, _cx: &mut Cx<'_>) -> bool {
        true
    }
}

/// Font and text options for the adapter.
#[derive(Debug, Clone)]
pub struct AdapterConfig {
    /// Primary UI font (file bytes and face index).
    pub font: Arc<[u8]>,
    pub font_index: u32,
    /// Fonts tried for characters the primary font lacks.
    pub fallbacks: Vec<Arc<[u8]>>,
    pub text_options: Option<TextOptions>,
    /// Text size in logical units (9 pt Segoe UI is 12).
    pub text_size: f32,
    /// Use this app mode instead of the system's.
    pub force_scheme: Option<ColorScheme>,
}

impl AdapterConfig {
    pub fn new(font: Arc<[u8]>) -> Self {
        Self {
            font,
            font_index: 0,
            fallbacks: Vec::new(),
            text_options: None,
            text_size: 12.0,
            force_scheme: None,
        }
    }
}

/// Hosts a [`Ui`] behind [`UiApp`].
pub struct UiAdapter<H: UiHandler> {
    handler: H,
    config: AdapterConfig,
    ui: Option<Ui>,
    bridge: InputBridge,
    watcher: SettingsWatcher,
    clipboard_is_system: bool,
}

impl<H: UiHandler> std::fmt::Debug for UiAdapter<H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UiAdapter")
            .field("ui", &self.ui)
            .finish_non_exhaustive()
    }
}

impl<H: UiHandler> UiAdapter<H> {
    pub fn new(handler: H, config: AdapterConfig) -> Self {
        Self {
            handler,
            config,
            ui: None,
            bridge: InputBridge::default(),
            watcher: SettingsWatcher::new(open365_ui_core::PlatformSettings::default()),
            clipboard_is_system: false,
        }
    }

    pub fn handler(&self) -> &H {
        &self.handler
    }

    /// True once the operating system clipboard is in use.
    pub fn clipboard_is_system(&self) -> bool {
        self.clipboard_is_system
    }

    fn scheme(&self, cx: &Cx<'_>) -> ColorScheme {
        self.config.force_scheme.unwrap_or(cx.scheme)
    }

    /// Re-reads the platform settings; applies and reports them when they changed.
    fn refresh_settings(&mut self, cx: &mut Cx<'_>) {
        let scheme = self.scheme(cx);
        let Some(ui) = self.ui.as_mut() else { return };
        if let Some(fresh) = self.watcher.poll(scheme) {
            ui.set_settings(fresh);
            self.handler.settings_changed(ui, cx);
            cx.request_redraw();
        }
    }

    /// Size and scale of the window as the `Ui` should see them.
    fn sync_size(&mut self, cx: &mut Cx<'_>) -> bool {
        let Some(ui) = self.ui.as_mut() else {
            return false;
        };
        let scale = cx.scale_factor as f32;
        let phys = cx.window.inner_size();
        let bounds = Rect::new(
            0.0,
            0.0,
            phys.width as f32 / scale,
            phys.height as f32 / scale,
        );
        let same = ui.scale == Scale(scale)
            && ui
                .tree
                .get(ui.tree.root())
                .is_some_and(|r| r.bounds == bounds);
        if same {
            return false;
        }
        ui.scale = Scale(scale);
        let root = ui.tree.root();
        if let Some(r) = ui.tree.get_mut(root) {
            r.bounds = bounds;
        }
        self.bridge.set_scale(cx.scale_factor);
        self.handler.resized(ui, cx);
        cx.request_redraw();
        true
    }

    /// Feeds `inputs` to the `Ui`, hands the widget events to the handler and schedules what
    /// the `Ui` wants next (a redraw, a timer).
    fn pump(&mut self, cx: &mut Cx<'_>, inputs: Vec<InputEvent>) {
        let Some(ui) = self.ui.as_mut() else { return };
        // Timers and tooltips are measured on the host clock.
        let mut redraw = ui.handle(InputEvent::Tick(cx.now));
        for ev in inputs {
            redraw |= ui.handle(ev);
        }
        let events = ui.take_events();
        if !events.is_empty() {
            self.handler.emitted(ui, cx, events);
            redraw = true;
        }
        redraw |= ui.take_redraw();
        if redraw {
            cx.request_redraw();
        }
        if let Some(at) = ui.next_wake() {
            cx.wake_at(at);
        }
    }
}

impl<H: UiHandler> UiApp for UiAdapter<H> {
    fn init(&mut self, cx: &mut Cx<'_>, assets: &mut Assets) {
        let mut real = std::mem::replace(assets, Assets::new(16));
        if let Some(o) = self.config.text_options {
            real.set_text_options(o);
        }
        let Ok(font) = real.add_font(self.config.font.clone(), self.config.font_index) else {
            // The host cannot draw text without a font; keep the empty window instead of panicking.
            *assets = real;
            return;
        };
        let fallbacks = self
            .config
            .fallbacks
            .iter()
            .filter_map(|b| real.add_font(b.clone(), 0).ok())
            .collect();
        real.set_fallbacks(font, fallbacks);
        let scale = cx.scale_factor as f32;
        let phys = cx.window.inner_size();
        let bounds = Rect::new(
            0.0,
            0.0,
            phys.width as f32 / scale,
            phys.height as f32 / scale,
        );
        let mut ui = Ui::new(bounds, scale, real, font, cx.now);
        ui.text_size = self.config.text_size;
        let scheme = self.scheme(cx);
        let settings = with_scheme(open365_ui_core::PlatformSettings::read(), scheme);
        self.watcher = SettingsWatcher::new(settings.clone());
        ui.set_settings(settings);
        let clip = SystemClipboard::new();
        self.clipboard_is_system = clip.is_system();
        ui.set_clipboard(Box::new(clip));
        self.bridge.set_scale(cx.scale_factor);
        self.handler.init(&mut ui, cx);
        self.ui = Some(ui);
        cx.request_redraw();
    }

    fn event(&mut self, cx: &mut Cx<'_>, event: &UiEvent<'_>) {
        let mut inputs = Vec::new();
        match event {
            UiEvent::Window(w) => {
                let consumed = match self.ui.as_mut() {
                    Some(ui) => self.handler.window_event(ui, cx, w),
                    None => false,
                };
                match w {
                    WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                        self.sync_size(cx);
                    }
                    WindowEvent::Focused(true) => self.refresh_settings(cx),
                    _ => {}
                }
                if !consumed {
                    self.bridge.translate(w, &mut inputs);
                }
            }
            UiEvent::Theme(_) => self.refresh_settings(cx),
            UiEvent::Tick(_) => {}
        }
        self.pump(cx, inputs);
    }

    fn close_requested(&mut self, cx: &mut Cx<'_>) -> bool {
        match self.ui.as_mut() {
            Some(ui) => self.handler.close_requested(ui, cx),
            None => true,
        }
    }

    fn assets(&mut self) -> Option<&mut Assets> {
        self.ui.as_mut().map(|u| &mut u.assets)
    }

    fn paint(&mut self, cx: &mut Cx<'_>, frame: &mut Frame<'_>) {
        // A resize may have been queued behind this frame request.
        self.sync_size(cx);
        let Some(ui) = self.ui.as_mut() else { return };
        self.handler.paint_below(ui, frame.list);
        ui.paint(frame.list, open365_ui_core::PaintLayers::default());
        self.handler.paint_above(ui, frame.list);
        cx.update_accessibility(|| ui.accessibility_tree());
        if let Some(at) = ui.next_wake() {
            cx.wake_at(at);
        }
    }

    fn ime(&mut self) -> Option<&mut dyn ImeHandler> {
        if self.ui.is_some() { Some(self) } else { None }
    }

    fn accessibility_action(&mut self, cx: &mut Cx<'_>, request: accesskit::ActionRequest) {
        if let Some(ui) = self.ui.as_mut() {
            ui.accessibility_action(&request);
        }
        self.pump(cx, Vec::new());
    }

    fn accessibility_tree(&mut self, _cx: &mut Cx<'_>) -> Option<TreeUpdate> {
        self.ui.as_mut().map(Ui::accessibility_tree)
    }
}

impl<H: UiHandler> ImeHandler for UiAdapter<H> {
    fn wants_ime(&self) -> bool {
        self.ui.as_ref().is_some_and(Ui::wants_ime)
    }

    fn cursor_area(&self) -> Option<ImeCursorArea> {
        let ui = self.ui.as_ref()?;
        let r = ui.ime_cursor_area()?;
        let s = f64::from(ui.scale.0);
        Some(ImeCursorArea {
            x: f64::from(r.x0) / s,
            y: f64::from(r.y0) / s,
            width: f64::from(r.width()) / s,
            height: f64::from(r.height()) / s,
        })
    }

    fn ime_enabled(&mut self) {
        if let Some(ui) = self.ui.as_mut() {
            ui.handle(InputEvent::Ime(open365_ui_core::ImeInput::Enabled));
        }
    }

    fn preedit(&mut self, text: &str, cursor: Option<(usize, usize)>) {
        if let Some(ui) = self.ui.as_mut() {
            ui.handle(InputEvent::Ime(open365_ui_core::ImeInput::Preedit {
                text: text.to_owned(),
                cursor,
            }));
        }
    }

    fn commit(&mut self, text: &str) {
        if let Some(ui) = self.ui.as_mut() {
            ui.handle(InputEvent::Ime(open365_ui_core::ImeInput::Commit(
                text.to_owned(),
            )));
        }
    }

    fn ime_disabled(&mut self) {
        if let Some(ui) = self.ui.as_mut() {
            ui.handle(InputEvent::Ime(open365_ui_core::ImeInput::Disabled));
        }
    }
}

/// Maps an [`ImeEvent`] to a core input (used by tests and custom hosts).
pub fn ime_input(e: &ImeEvent) -> open365_ui_core::ImeInput {
    match e {
        ImeEvent::Enabled => open365_ui_core::ImeInput::Enabled,
        ImeEvent::Preedit { text, cursor } => open365_ui_core::ImeInput::Preedit {
            text: text.clone(),
            cursor: *cursor,
        },
        ImeEvent::Commit(t) => open365_ui_core::ImeInput::Commit(t.clone()),
        ImeEvent::Disabled => open365_ui_core::ImeInput::Disabled,
    }
}
