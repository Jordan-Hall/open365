//! View tab, workbook side: Custom Views. A custom view is the zoom, the workbook view, the
//! gridlines, headings and frozen panes of every sheet, the formula bar and the position in the
//! active sheet under a name. Excel keeps them in the file (`customWorkbookViews` and
//! `customSheetViews`), so they are saved with it, and adding or deleting one is an undo step of
//! the workbook.
//!
//! Not included: the dialog's "Print settings" and "Hidden rows, columns and filter settings"
//! choices (a view always leaves both as they are).

use open365_commands::sheet_page_ops::{CustomView, CustomViewSheet};
use open365_commands::{Command, SheetPageOp, sheet_page_ops};

use super::super::SheetModel;
use super::view::{Freeze, ViewMode};

fn mode_name(m: ViewMode) -> &'static str {
    match m {
        ViewMode::Normal => "normal",
        ViewMode::PageBreakPreview => "pageBreakPreview",
        ViewMode::PageLayout => "pageLayout",
    }
}

fn mode_of(name: &str) -> ViewMode {
    match name {
        "pageBreakPreview" => ViewMode::PageBreakPreview,
        "pageLayout" => ViewMode::PageLayout,
        _ => ViewMode::Normal,
    }
}

impl SheetModel {
    /// The custom views of the workbook.
    #[must_use]
    pub fn view_custom_views(&self) -> Vec<CustomView> {
        self.target
            .xlsx()
            .map(sheet_page_ops::custom_views)
            .unwrap_or_default()
    }

    /// Custom Views > Add: saves the current view under `name`, replacing a view of that name.
    ///
    /// # Errors
    /// The name is empty or too long, or the document is not a workbook.
    pub fn view_add_custom_view(&mut self, name: &str) -> Result<(), String> {
        let count = self
            .target
            .xlsx()
            .map(|wb| wb.sheets().len())
            .ok_or_else(|| "Custom views need a workbook.".to_owned())?;
        let here = self.sheet;
        let mut sheets = Vec::with_capacity(count);
        for i in 0..count {
            self.sheet = i;
            let (fr, fc) = self.frozen();
            let current = i == here;
            sheets.push(CustomViewSheet {
                zoom: self.ext_view.zoom,
                view: mode_name(self.ext_view.mode).to_owned(),
                gridlines: self.ext_view.gridlines(i),
                headings: self.ext_view.headings(i),
                frozen_rows: fr,
                frozen_cols: fc,
                top_left: if current { self.top_left } else { (1, 1) },
                active: if current { self.active } else { (1, 1) },
            });
        }
        self.sheet = here;
        let view = CustomView {
            name: name.to_owned(),
            guid: String::new(),
            formula_bar: self.ext_view.formula_bar,
            active_sheet: here,
            sheets,
        };
        self.apply(Command::SheetPage {
            sheet: here,
            op: SheetPageOp::SaveCustomView(view),
        })
    }

    /// Custom Views > Show: applies the view called `name`.
    ///
    /// # Errors
    /// There is no such view.
    pub fn view_show_custom_view(&mut self, name: &str) -> Result<(), String> {
        let view = self
            .view_custom_views()
            .into_iter()
            .find(|v| v.name.eq_ignore_ascii_case(name.trim()))
            .ok_or_else(|| "There is no view with that name.".to_owned())?;
        if let Some(s) = view.sheets.first() {
            self.set_zoom_percent(s.zoom);
            self.set_view_mode(mode_of(&s.view));
        }
        self.ext_view.formula_bar = view.formula_bar;
        let here = self.sheet;
        for (i, s) in view.sheets.iter().enumerate() {
            self.ext_view.set_gridlines(i, s.gridlines);
            self.ext_view.set_headings(i, s.headings);
            self.sheet = i;
            if self.frozen() != (s.frozen_rows, s.frozen_cols) {
                if self.is_frozen() {
                    self.freeze(Freeze::Unfreeze);
                }
                if s.frozen_rows > 0 || s.frozen_cols > 0 {
                    self.active = (s.frozen_rows + 1, s.frozen_cols + 1);
                    self.freeze(Freeze::Panes);
                }
            }
        }
        self.sheet = if view.sheets.is_empty() {
            here
        } else {
            view.active_sheet.min(view.sheets.len() - 1)
        };
        if let Some(s) = view.sheets.get(self.sheet) {
            self.select_cell(s.active);
            self.top_left = s.top_left;
            self.clamp_scroll();
        }
        Ok(())
    }

    /// Custom Views > Delete.
    ///
    /// # Errors
    /// There is no such view.
    pub fn view_delete_custom_view(&mut self, name: &str) -> Result<(), String> {
        let sheet = self.sheet;
        self.apply(Command::SheetPage {
            sheet,
            op: SheetPageOp::DeleteCustomView {
                name: name.trim().to_owned(),
            },
        })
    }
}
