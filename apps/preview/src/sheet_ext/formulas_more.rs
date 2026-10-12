//! Formulas tab, second half: Define Name, Use in Formula, Create from Selection, the Watch Window
//! list and Calculation Options / Calculate Now / Calculate Sheet.
//!
//! Calculation settings live in the workbook (`calcPr`), so they are saved with the file and a
//! change is one undo step. The watch list is view state: it is kept per thread (one Sheets model
//! per shell) until `FormulasState` can hold it; see the report for the field it should become.

use std::cell::RefCell;

use open365_commands::objects_ops::{CalcModeChoice, NameRequest};
use open365_commands::{Command, ObjectsOp, cell_display_text};

use crate::sheet::{Rect4, SheetModel, a1};

/// One row of the Watch Window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchRow {
    pub book: String,
    pub sheet: String,
    /// The defined name that refers to exactly this cell, or empty.
    pub name: String,
    pub cell: String,
    pub value: String,
    pub formula: String,
}

#[derive(Debug, Default, Clone)]
struct WatchState {
    visible: bool,
    /// `(sheet index, row, col)`.
    cells: Vec<(usize, u32, u32)>,
}

thread_local! {
    static WATCH: RefCell<WatchState> = RefCell::new(WatchState::default());
}

/// The calculation option a Calculation Options menu item names.
fn calc_choice(item: &str) -> Option<CalcModeChoice> {
    match item {
        "automatic" | "auto" => Some(CalcModeChoice::Automatic),
        "automatic-except-tables"
        | "automatic-except-data-tables"
        | "auto-except-tables"
        | "except-tables" => Some(CalcModeChoice::AutomaticExceptTables),
        "manual" => Some(CalcModeChoice::Manual),
        _ => None,
    }
}

/// `'My Sheet'!$A$1:$B$2`: an absolute reference as a defined name stores it.
fn absolute(sheet: &str, r: Rect4) -> String {
    let plain = !sheet.is_empty() && sheet.chars().all(|c| c.is_alphanumeric() || c == '_');
    let q = if plain {
        sheet.to_owned()
    } else {
        format!("'{}'", sheet.replace('\'', "''"))
    };
    let abs = |row, col| {
        let s = a1(row, col);
        let n = s.find(|c: char| c.is_ascii_digit()).unwrap_or(0);
        format!("${}${}", &s[..n], &s[n..])
    };
    if (r.0, r.1) == (r.2, r.3) {
        format!("{q}!{}", abs(r.0, r.1))
    } else {
        format!("{q}!{}:{}", abs(r.0, r.1), abs(r.2, r.3))
    }
}

/// A label as Excel turns it into a name: spaces and other illegal characters become `_`, a
/// leading digit or an A1-looking label gets a `_` in front.
fn name_from_label(label: &str) -> Option<String> {
    let t = label.trim();
    if t.is_empty() {
        return None;
    }
    let mut s: String = t
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '_' | '.' | '\\') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let first = s.chars().next()?;
    if !(first.is_alphabetic() || first == '_' || first == '\\') {
        s.insert(0, '_');
    }
    let upper = s.to_ascii_uppercase();
    let letters = upper.chars().take_while(char::is_ascii_alphabetic).count();
    let looks_like_cell = (1..=3).contains(&letters)
        && upper.len() > letters
        && upper[letters..].chars().all(|c| c.is_ascii_digit());
    if looks_like_cell || matches!(upper.as_str(), "R" | "C") {
        s.insert(0, '_');
    }
    Some(s)
}

/// Which edges of the selection hold the labels (Create Names from Selection).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LabelEdges {
    pub top: bool,
    pub left: bool,
    pub bottom: bool,
    pub right: bool,
}

impl LabelEdges {
    /// `top,left` style text (the value of the ribbon command).
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut e = Self::default();
        for p in text.split(',') {
            match p.trim().to_ascii_lowercase().as_str() {
                "top" | "top-row" => e.top = true,
                "left" | "left-column" => e.left = true,
                "bottom" | "bottom-row" => e.bottom = true,
                "right" | "right-column" => e.right = true,
                _ => {}
            }
        }
        e
    }
}

impl SheetModel {
    fn sheet_name_here(&self) -> String {
        self.target
            .xlsx()
            .and_then(|wb| wb.sheets().get(self.sheet).map(|s| s.name.clone()))
            .unwrap_or_default()
    }

    fn formulas_op(&mut self, op: ObjectsOp) -> Result<(), String> {
        self.apply(Command::SheetObjects {
            sheet: self.sheet,
            op,
        })
    }

    /// Define Name: a workbook-scoped name for the selection. One undo step.
    ///
    /// # Errors
    /// The command layer's message (illegal name, name taken).
    pub fn define_name_for_selection(&mut self, name: &str) -> Result<(), String> {
        let refers_to = absolute(&self.sheet_name_here(), self.range());
        self.name_define(NameRequest {
            name: name.to_owned(),
            scope: None,
            refers_to,
            comment: None,
            hidden: false,
        })
    }

    /// The names the Use in Formula list offers, sorted.
    #[must_use]
    pub fn names_for_formula(&self) -> Vec<String> {
        let mut v: Vec<String> = self.name_rows().into_iter().map(|r| r.name).collect();
        v.dedup();
        v
    }

    /// Use in Formula > a name: starts `=name` in an empty active cell, appends the name to a
    /// formula already there; text and numbers are left alone with a status message.
    ///
    /// # Errors
    /// The command layer's message.
    pub fn use_name_in_formula(&mut self, name: &str) -> Result<(), String> {
        if !self.names_for_formula().iter().any(|n| n == name) {
            self.status = format!("There is no name {name}.");
            return Ok(());
        }
        let (r, c) = self.active;
        let src = self.source(r, c);
        let text = if src.is_empty() {
            format!("={name}")
        } else if src.starts_with('=') {
            format!("{src}{name}")
        } else {
            self.status = "Select a cell to start the formula in.".to_owned();
            return Ok(());
        };
        self.enter(r, c, &text)
    }

    /// Create from Selection: names from the labels on the chosen edges. A label becomes a name
    /// for the cells it heads; labels that cannot be made into a legal name or already name
    /// something are skipped. Returns how many names were created.
    pub fn create_names_from_selection(&mut self, edges: LabelEdges) -> usize {
        let (r1, c1, r2, c2) = self.range();
        let sheet = self.sheet_name_here();
        let mut taken = self.names_for_formula();
        let (dr1, dr2) = (r1 + u32::from(edges.top), r2 - u32::from(edges.bottom));
        let (dc1, dc2) = (c1 + u32::from(edges.left), c2 - u32::from(edges.right));
        // (label cell, data rect)
        let mut jobs: Vec<((u32, u32), Rect4)> = Vec::new();
        if dr1 <= dr2 {
            for c in dc1..=dc2 {
                if edges.top {
                    jobs.push(((r1, c), (dr1, c, dr2, c)));
                }
                if edges.bottom {
                    jobs.push(((r2, c), (dr1, c, dr2, c)));
                }
            }
        }
        if dc1 <= dc2 {
            for r in dr1..=dr2 {
                if edges.left {
                    jobs.push(((r, c1), (r, dc1, r, dc2)));
                }
                if edges.right {
                    jobs.push(((r, c2), (r, dc1, r, dc2)));
                }
            }
        }
        let mut made = 0;
        for ((lr, lc), data) in jobs {
            let Some(name) = name_from_label(&self.display(lr, lc)) else {
                continue;
            };
            if taken.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                continue;
            }
            let req = NameRequest {
                name: name.clone(),
                scope: None,
                refers_to: absolute(&sheet, data),
                comment: None,
                hidden: false,
            };
            if self.name_define(req).is_ok() {
                taken.push(name);
                made += 1;
            }
        }
        self.status = format!("{made} name(s) created.");
        made
    }

    // ---- Calculation ---------------------------------------------------------------------

    /// Calculation Options: `automatic`, `automatic-except-tables` or `manual`.
    ///
    /// # Errors
    /// The command layer's message; an unknown option.
    pub fn set_calculation_mode(&mut self, item: &str) -> Result<(), String> {
        let mode = calc_choice(item).ok_or_else(|| format!("unknown calculation option {item}"))?;
        self.formulas_op(ObjectsOp::SetCalculation {
            mode: Some(mode),
            full_calc_on_load: None,
            calc_on_save: None,
            iterate: None,
            iterate_count: None,
            iterate_delta: None,
            precision_as_displayed: None,
        })
    }

    /// The workbook's calculation mode.
    #[must_use]
    pub fn calculation_mode(&self) -> open365_xlsx::CalcMode {
        self.target
            .xlsx()
            .map(|wb| wb.calc_settings().mode)
            .unwrap_or_default()
    }

    /// Calculate Now (F9): every formula in the workbook.
    ///
    /// # Errors
    /// The command layer's message.
    pub fn calculate_now(&mut self) -> Result<(), String> {
        self.formulas_op(ObjectsOp::CalculateNow)
    }

    /// Calculate Sheet (Shift+F9): the active sheet.
    ///
    /// # Errors
    /// The command layer's message.
    pub fn calculate_sheet(&mut self) -> Result<(), String> {
        self.formulas_op(ObjectsOp::CalculateSheet)
    }

    // ---- Watch Window ----------------------------------------------------------------------

    /// Whether the Watch Window is shown.
    #[must_use]
    pub fn watch_visible(&self) -> bool {
        WATCH.with(|w| w.borrow().visible)
    }

    /// Shows or hides the Watch Window; returns the new state.
    pub fn toggle_watch_window(&mut self) -> bool {
        WATCH.with(|w| {
            let mut w = w.borrow_mut();
            w.visible = !w.visible;
            w.visible
        })
    }

    /// Add Watch: every cell of the selection (a cell is watched once).
    pub fn watch_add_selection(&mut self) {
        let (r1, c1, r2, c2) = self.range();
        let sheet = self.sheet;
        WATCH.with(|w| {
            let mut w = w.borrow_mut();
            w.visible = true;
            for r in r1..=r2 {
                for c in c1..=c2 {
                    if !w.cells.contains(&(sheet, r, c)) {
                        w.cells.push((sheet, r, c));
                    }
                }
            }
        });
    }

    /// Delete Watch: the rows at `indices` of the list.
    pub fn watch_delete(&mut self, indices: &[usize]) {
        WATCH.with(|w| {
            let mut w = w.borrow_mut();
            let mut i = 0;
            w.cells.retain(|_| {
                let keep = !indices.contains(&i);
                i += 1;
                keep
            });
        });
    }

    /// The Watch Window rows with their current values.
    #[must_use]
    pub fn watch_rows(&self) -> Vec<WatchRow> {
        let cells = WATCH.with(|w| w.borrow().cells.clone());
        let Some(wb) = self.target.xlsx() else {
            return Vec::new();
        };
        let book = self
            .path()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
        let book = book.unwrap_or_else(|| "Book1".to_owned());
        cells
            .into_iter()
            .filter_map(|(s, r, c)| {
                let sheet = wb.sheets().get(s)?.name.clone();
                let target = absolute(&sheet, (r, c, r, c));
                let name = wb
                    .defined_names()
                    .iter()
                    .find(|n| !n.hidden && n.formula.replace(' ', "") == target)
                    .map(|n| n.name.clone())
                    .unwrap_or_default();
                let formula = wb
                    .formula_text(s, r, c)
                    .map(|f| format!("={f}"))
                    .unwrap_or_default();
                Some(WatchRow {
                    book: book.clone(),
                    sheet,
                    name,
                    cell: a1(r, c),
                    value: cell_display_text(wb, s, r, c),
                    formula,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_become_legal_names() {
        assert_eq!(name_from_label("Sales Q1").as_deref(), Some("Sales_Q1"));
        assert_eq!(name_from_label("2024").as_deref(), Some("_2024"));
        assert_eq!(name_from_label("A1").as_deref(), Some("_A1"));
        assert_eq!(name_from_label("  "), None);
    }

    #[test]
    fn absolute_quotes_sheet_names() {
        assert_eq!(absolute("Sheet1", (1, 1, 3, 2)), "Sheet1!$A$1:$B$3");
        assert_eq!(absolute("My Sheet", (2, 3, 2, 3)), "'My Sheet'!$C$2");
    }
}
