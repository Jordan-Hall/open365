//! View tab state and operations of the sheets model: workbook view, zoom, Show options,
//! Freeze Panes, Split and window bookkeeping.
//!
//! Excel keeps none of this in the undo history, so none of it is a command. Gridlines, headings,
//! freeze and split are per worksheet; the rest belongs to the window. The state lives in the
//! model and is not yet written to the file's `<sheetViews>`.

use std::collections::BTreeMap;

use super::super::{GridGeometry, MAX_COL, MAX_ROW, SheetModel};

/// The three workbook views of the View tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Normal,
    PageBreakPreview,
    PageLayout,
}

/// The four Freeze Panes menu actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freeze {
    /// Rows above and columns left of the active cell.
    Panes,
    TopRow,
    FirstColumn,
    Unfreeze,
}

/// Per worksheet view settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SheetView {
    gridlines: bool,
    headings: bool,
    /// `(rows, cols)` the user froze; `None` keeps what the file says.
    freeze: Option<(u32, u32)>,
    /// `(row, col)` of the cell the window was split at.
    split: Option<(u32, u32)>,
}

impl Default for SheetView {
    fn default() -> Self {
        Self {
            gridlines: true,
            headings: true,
            freeze: None,
            split: None,
        }
    }
}

/// Everything the View tab changes.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewState {
    pub mode: ViewMode,
    pub formula_bar: bool,
    pub ruler: bool,
    pub navigation: bool,
    pub data_type_icons: bool,
    pub focus_cell: bool,
    /// Zoom in percent, 10 to 400.
    pub zoom: u32,
    /// Windows open on this workbook (`Book1:1`, `Book1:2`, ...).
    pub windows: u32,
    /// The window is hidden (View > Hide).
    pub hidden: bool,
    pub side_by_side: bool,
    pub synchronous_scrolling: bool,
    sheets: BTreeMap<usize, SheetView>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            mode: ViewMode::Normal,
            formula_bar: true,
            ruler: true,
            navigation: false,
            data_type_icons: true,
            focus_cell: false,
            zoom: 100,
            windows: 1,
            hidden: false,
            side_by_side: false,
            synchronous_scrolling: true,
            sheets: BTreeMap::new(),
        }
    }
}

impl ViewState {
    fn of(&self, sheet: usize) -> SheetView {
        self.sheets.get(&sheet).copied().unwrap_or_default()
    }

    fn of_mut(&mut self, sheet: usize) -> &mut SheetView {
        self.sheets.entry(sheet).or_default()
    }

    /// Whether the worksheet shows its gridlines (also Page Layout > Gridlines > View).
    #[must_use]
    pub fn gridlines(&self, sheet: usize) -> bool {
        self.of(sheet).gridlines
    }

    pub fn set_gridlines(&mut self, sheet: usize, on: bool) {
        self.of_mut(sheet).gridlines = on;
    }

    /// Whether the worksheet shows row and column headings (also Page Layout > Headings > View).
    #[must_use]
    pub fn headings(&self, sheet: usize) -> bool {
        self.of(sheet).headings
    }

    pub fn set_headings(&mut self, sheet: usize, on: bool) {
        self.of_mut(sheet).headings = on;
    }

    /// The cell the window of `sheet` is split at.
    #[must_use]
    pub fn split(&self, sheet: usize) -> Option<(u32, u32)> {
        self.of(sheet).split
    }

    /// The geometry with the frozen panes the user chose (the file's panes otherwise).
    pub(crate) fn apply_freeze(&self, sheet: usize, mut geo: GridGeometry) -> GridGeometry {
        if let Some(f) = self.sheets.get(&sheet).and_then(|s| s.freeze) {
            geo.frozen = f;
        }
        geo
    }

    /// The names the window title shows: `Book1:2` for the second window, empty for one.
    #[must_use]
    pub fn window_suffix(&self, n: u32) -> String {
        if self.windows > 1 {
            format!(":{n}")
        } else {
            String::new()
        }
    }
}

/// Smallest and largest zoom of the Zoom dialog.
pub const ZOOM_MIN: u32 = 10;
pub const ZOOM_MAX: u32 = 400;

impl SheetModel {
    /// Switches to Normal, Page Break Preview or Page Layout view.
    pub fn set_view_mode(&mut self, mode: ViewMode) {
        self.ext_view.mode = mode;
    }

    /// Sets the zoom (10 to 400 percent, clamped as the Zoom dialog does).
    pub fn set_zoom_percent(&mut self, percent: u32) {
        self.ext_view.zoom = percent.clamp(ZOOM_MIN, ZOOM_MAX);
    }

    /// Zoom to Selection: the largest zoom at which the selected range fills a window of
    /// `view_w` by `view_h` pixels (a single cell zooms to 400%).
    pub fn zoom_to_selection(&mut self, view_w: f32, view_h: f32) {
        let (r1, c1, r2, c2) = self.range();
        let geo = self.geometry();
        let w: f64 = (c1..=c2.min(c1.saturating_add(2000)))
            .map(|c| f64::from(geo.col_px(c)))
            .sum();
        let h: f64 = (r1..=r2.min(r1.saturating_add(20000)))
            .map(|r| f64::from(geo.row_px(r)))
            .sum();
        let by_w = f64::from(view_w) / w.max(1.0);
        let by_h = f64::from(view_h) / h.max(1.0);
        let z = (by_w.min(by_h) * 100.0).floor();
        self.set_zoom_percent(if z.is_finite() { z as u32 } else { ZOOM_MAX });
        // The selection starts at the window's top-left corner.
        let (fr, fc) = self.frozen();
        self.top_left = (r1.max(fr + 1), c1.max(fc + 1));
    }

    /// The rows and columns frozen at the top and left of the active sheet.
    #[must_use]
    pub fn is_frozen(&self) -> bool {
        self.frozen() != (0, 0)
    }

    /// Freeze Panes menu: freezes (or unfreezes) the active sheet as Excel does.
    pub fn freeze(&mut self, kind: Freeze) {
        let frozen = match kind {
            Freeze::Unfreeze => (0, 0),
            Freeze::TopRow => (1, 0),
            Freeze::FirstColumn => (0, 1),
            Freeze::Panes => {
                // The first menu item reads Unfreeze Panes while panes are frozen.
                if self.is_frozen() {
                    (0, 0)
                } else {
                    let (r, c) = self.active;
                    (
                        r.saturating_sub(1).min(MAX_ROW - 1),
                        c.saturating_sub(1).min(MAX_COL - 1),
                    )
                }
            }
        };
        let sheet = self.sheet;
        self.ext_view.of_mut(sheet).freeze = Some(frozen);
        if frozen != (0, 0) {
            // The panes show the first rows and columns; the rest scrolls beside them.
            self.top_left = (
                self.top_left.0.max(frozen.0 + 1),
                self.top_left.1.max(frozen.1 + 1),
            );
        }
        self.clamp_scroll();
    }

    /// Split: divides the window at the active cell, or removes the split.
    pub fn toggle_split(&mut self) {
        let sheet = self.sheet;
        let at = self.active;
        let v = self.ext_view.of_mut(sheet);
        v.split = if v.split.is_some() { None } else { Some(at) };
    }
}
