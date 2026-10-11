//! The application-facing API of the host: the [`UiApp`] trait and the contexts passed to it.

use std::time::{Duration, Instant};

use accesskit::{ActionRequest, TreeUpdate};
use open365_gfx::{Assets, DrawList};
use winit::event::WindowEvent;
use winit::window::Window;

use crate::ime::ImeHandler;
use crate::schedule::Scheduler;
use crate::theme::{ColorScheme, SystemMetrics};

/// An event delivered to [`UiApp::event`].
#[derive(Debug)]
pub enum UiEvent<'a> {
    /// A raw window event (pointer, keyboard, resize, focus, scale-factor change). IME events
    /// are not repeated here: they go to [`UiApp::ime`].
    Window(&'a WindowEvent),
    /// The system app mode changed (light or dark).
    Theme(ColorScheme),
    /// A wake-up requested with [`Cx::wake_after`] is due.
    Tick(Instant),
}

/// Handles to the window and scheduler, valid for one callback.
pub struct Cx<'a> {
    pub window: &'a Window,
    pub(crate) adapter: &'a mut accesskit_winit::Adapter,
    pub(crate) scheduler: &'a mut Scheduler,
    /// Time of this callback; use it instead of calling `Instant::now()` so tests stay deterministic.
    pub now: Instant,
    pub scheme: ColorScheme,
    /// Device pixels per logical pixel (1.0 = 96 dpi on Windows), per the monitor the window is on.
    pub scale_factor: f64,
    pub metrics: SystemMetrics,
}

impl std::fmt::Debug for Cx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cx")
            .field("scheme", &self.scheme)
            .field("scale_factor", &self.scale_factor)
            .finish_non_exhaustive()
    }
}

impl Cx<'_> {
    /// Schedule one frame.
    pub fn request_redraw(&mut self) {
        self.scheduler.request_redraw();
    }

    /// Deliver a [`UiEvent::Tick`] after `delay` (the earliest pending wake-up wins). The host
    /// sleeps until then; nothing runs in between.
    pub fn wake_after(&mut self, delay: Duration) {
        self.scheduler.wake_after(self.now, delay);
    }

    pub fn wake_at(&mut self, at: Instant) {
        self.scheduler.wake_at(at);
    }

    /// Pushes an accessibility tree update, built only if a screen reader is attached.
    pub fn update_accessibility(&mut self, build: impl FnOnce() -> TreeUpdate) {
        self.adapter.update_if_active(build);
    }

    /// Close the window and end the event loop after this callback (no `close_requested` call).
    pub fn request_exit(&mut self) {
        self.scheduler.request_exit();
    }

    pub fn set_title(&self, title: &str) {
        self.window.set_title(title);
    }
}

/// Drawing resources for one frame. Coordinates are device pixels.
#[derive(Debug)]
pub struct Frame<'a> {
    pub list: &'a mut DrawList,
    pub assets: &'a mut Assets,
    /// Target size in device pixels.
    pub size_px: (u32, u32),
}

/// An application hosted by [`crate::run`].
pub trait UiApp: 'static {
    /// Called once, after the window and GPU exist: register fonts, icons and images.
    fn init(&mut self, _cx: &mut Cx<'_>, _assets: &mut Assets) {}

    /// Called for every input, theme and timer event.
    fn event(&mut self, _cx: &mut Cx<'_>, _event: &UiEvent<'_>) {}

    /// Called when the user asks to close the window. Return false to keep it open.
    fn close_requested(&mut self, _cx: &mut Cx<'_>) -> bool {
        true
    }

    /// Atlases owned by the application. When this returns `Some`, the host renders with it
    /// instead of its own (the application then draws through the same [`Assets`] it uses for
    /// layout, as `UiAdapter` does with the `Ui`'s). `Frame::assets` is then unused.
    fn assets(&mut self) -> Option<&mut Assets> {
        None
    }

    /// Fill `frame.list`. Called only when a frame was requested.
    fn paint(&mut self, cx: &mut Cx<'_>, frame: &mut Frame<'_>);

    /// The focused text surface, if any, to receive IME composition.
    fn ime(&mut self) -> Option<&mut dyn ImeHandler> {
        None
    }

    /// The accessibility tree, requested when a screen reader first connects.
    fn accessibility_tree(&mut self, _cx: &mut Cx<'_>) -> Option<TreeUpdate> {
        None
    }

    /// A screen reader asked for an action (focus, click, scroll, set value).
    fn accessibility_action(&mut self, _cx: &mut Cx<'_>, _request: ActionRequest) {}
}
