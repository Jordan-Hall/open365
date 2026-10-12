//! View tab support for the Writer pane: the document view mode, the Show options, the zoom
//! presets and the window state.
//!
//! The pane struct has no field for this state yet, so it lives in a per-thread table (one
//! Writer window per thread). Moving it into a `WriterPane` field is a one-line change listed
//! in the ribbon report; the accessors below stay the same.

use std::cell::RefCell;

use super::{POINT_SCALE, SIDE_PAD, WriterPane};
use crate::ribbon::writer::view::ViewMode;

/// The View tab state of one window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewState {
    pub mode: ViewMode,
    pub gridlines: bool,
    pub nav_pane: bool,
    pub focus: bool,
    pub reader: bool,
    pub dark: bool,
    pub side_to_side: bool,
    pub split: bool,
    pub side_by_side: bool,
    pub sync_scroll: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            mode: ViewMode::PrintLayout,
            gridlines: false,
            nav_pane: false,
            focus: false,
            reader: false,
            dark: false,
            side_to_side: false,
            split: false,
            side_by_side: false,
            sync_scroll: true,
        }
    }
}

thread_local! {
    static STATE: RefCell<ViewState> = RefCell::new(ViewState::default());
}

impl WriterPane {
    /// The View tab state.
    #[must_use]
    pub fn view_state(&self) -> ViewState {
        STATE.with(|s| *s.borrow())
    }

    /// Changes the View tab state.
    pub fn edit_view_state(&mut self, f: impl FnOnce(&mut ViewState)) {
        STATE.with(|s| f(&mut s.borrow_mut()));
    }

    /// Sets the document view; Read Mode and the Immersive Reader are one switch.
    pub fn set_view_mode(&mut self, mode: ViewMode) {
        self.edit_view_state(|v| {
            v.mode = mode;
            v.reader = mode == ViewMode::Read;
        });
        self.status = Some(format!("{} view", mode.label()));
    }

    /// Size in points of the first page.
    fn zoom_page_pt(&self) -> Option<(f64, f64)> {
        let s = &self.engine.layout().pages.first()?.section;
        Some((s.page_width, s.page_height))
    }

    /// Zoom (percent) at which `pages_across` pages and their gaps fill the pane width.
    fn fit_width_percent(&self, pages_across: f64) -> Option<f32> {
        let (pw, _) = self.zoom_page_pt()?;
        let avail = f64::from(
            self.area.w as f32 - self.bar_w() - self.left_inset() - 2.0 * SIDE_PAD * self.scale,
        );
        let unit = f64::from(self.scale * POINT_SCALE);
        let pts = pw * pages_across + (pages_across - 1.0) * super::PAGE_GAP_PT;
        Some((avail / (pts * unit) * 100.0).floor() as f32)
    }

    /// View > Page Width: the page fills the pane width.
    pub fn zoom_page_width(&mut self) {
        if let Some(p) = self.fit_width_percent(1.0) {
            self.set_zoom_percent(p);
        }
    }

    /// View > One Page: a whole page fits in the pane.
    pub fn zoom_one_page(&mut self) {
        let (Some(w), Some((_, ph))) = (self.fit_width_percent(1.0), self.zoom_page_pt()) else {
            return;
        };
        let avail_h =
            f64::from(self.area.h as f32 - self.top_inset()) - 2.0 * f64::from(self.scale) * 12.0;
        let h = (avail_h / (ph * f64::from(self.scale * POINT_SCALE)) * 100.0).floor() as f32;
        self.set_zoom_percent(w.min(h));
        self.reveal_caret();
    }

    /// View > Multiple Pages: two pages side by side fit in the pane width.
    pub fn zoom_multiple_pages(&mut self) {
        if let Some(p) = self.fit_width_percent(2.0) {
            self.set_zoom_percent(p);
        }
    }
}
