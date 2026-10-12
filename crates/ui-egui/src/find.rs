//! The Find and Replace dialog (Ctrl+F, Ctrl+H) for both document kinds.
//!
//! Word documents search with Word's options (match case, whole words, wildcards); workbooks with
//! Excel's (within sheet or workbook, by rows or columns, formulas or values, entire cell).
//! Searching moves the selection; Replace All sends one command, so one undo reverts it.

use egui::{Context, Id, Window};
use open365_commands::{
    Command, DocPos, FindOptions, FindScope, LookIn, Target, XlsxFind, next_match, xlsx_find_all,
};

use crate::doc_view::DocView;
use crate::grid_view::GridView;

/// What the user asked of the dialog this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindAction {
    Next,
    Previous,
    ReplaceAll,
}

/// Dialog state: the text boxes and options of both kinds.
#[derive(Debug, Clone)]
pub struct FindDialog {
    pub open: bool,
    /// Show the replace box (Ctrl+H) as well as the find box.
    pub replace_mode: bool,
    pub find: String,
    pub replace: String,
    pub match_case: bool,
    pub whole_word: bool,
    pub wildcards: bool,
    pub match_entire_cell: bool,
    pub whole_workbook: bool,
    pub by_columns: bool,
    pub look_in_values: bool,
    /// A text box of the dialog has the keyboard.
    pub text_focused: bool,
    want_focus: bool,
}

impl Default for FindDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl FindDialog {
    pub fn new() -> Self {
        Self {
            open: false,
            replace_mode: false,
            find: String::new(),
            replace: String::new(),
            match_case: false,
            whole_word: false,
            wildcards: false,
            match_entire_cell: false,
            whole_workbook: false,
            by_columns: false,
            look_in_values: false,
            text_focused: false,
            want_focus: false,
        }
    }

    /// Opens the dialog; `replace` also shows the replace box.
    pub fn show_dialog(&mut self, replace: bool) {
        self.open = true;
        self.replace_mode = replace;
        self.want_focus = true;
    }

    fn docx_options(&self) -> FindOptions {
        FindOptions {
            pattern: self.find.clone(),
            match_case: self.match_case,
            whole_word: self.whole_word,
            wildcards: self.wildcards,
        }
    }

    fn xlsx_find(&self, grid: &GridView) -> XlsxFind {
        XlsxFind {
            text: self.find.clone(),
            scope: if self.whole_workbook {
                FindScope::Workbook
            } else {
                FindScope::Sheet {
                    sheet: grid.sheet(),
                }
            },
            by_columns: self.by_columns,
            look_in: if self.look_in_values {
                LookIn::Values
            } else {
                LookIn::Formulas
            },
            match_case: self.match_case,
            match_entire_cell: self.match_entire_cell,
        }
    }

    /// Draws the dialog and reports the button pressed.
    pub fn ui(&mut self, ctx: &Context, is_docx: bool) -> Option<FindAction> {
        if !self.open {
            self.text_focused = false;
            return None;
        }
        let mut action = None;
        let mut open = true;
        let mut focused = false;
        Window::new(if self.replace_mode { "Replace" } else { "Find" })
            .id(Id::new("open365-find"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Find what:");
                    let r = ui.text_edit_singleline(&mut self.find);
                    if std::mem::take(&mut self.want_focus) {
                        r.request_focus();
                    }
                    focused |= r.has_focus();
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        action = Some(FindAction::Next);
                        r.request_focus();
                    }
                });
                if self.replace_mode {
                    ui.horizontal(|ui| {
                        ui.label("Replace with:");
                        let r = ui.text_edit_singleline(&mut self.replace);
                        focused |= r.has_focus();
                    });
                }
                ui.checkbox(&mut self.match_case, "Match case");
                if is_docx {
                    ui.checkbox(&mut self.whole_word, "Find whole words only");
                    ui.checkbox(&mut self.wildcards, "Use wildcards");
                } else {
                    ui.checkbox(&mut self.match_entire_cell, "Match entire cell contents");
                    ui.checkbox(&mut self.whole_workbook, "Within workbook (else sheet)");
                    ui.checkbox(&mut self.by_columns, "Search by columns (else rows)");
                    ui.checkbox(&mut self.look_in_values, "Look in values (else formulas)");
                }
                ui.horizontal(|ui| {
                    if ui.button("Find next").clicked() {
                        action = Some(FindAction::Next);
                    }
                    if ui.button("Find previous").clicked() {
                        action = Some(FindAction::Previous);
                    }
                    if self.replace_mode && ui.button("Replace all").clicked() {
                        action = Some(FindAction::ReplaceAll);
                    }
                });
            });
        self.text_focused = focused;
        if !open {
            self.open = false;
            self.text_focused = false;
        }
        action
    }

    /// Carries out `action` on the open document and returns the status line.
    pub fn run(
        &self,
        action: FindAction,
        target: &mut Target,
        doc: &mut DocView,
        grid: &mut GridView,
    ) -> String {
        if self.find.is_empty() {
            return "Type something to find".to_owned();
        }
        if target.docx().is_some() {
            self.run_docx(action, target, doc)
        } else {
            self.run_xlsx(action, target, grid)
        }
    }

    fn run_docx(&self, action: FindAction, target: &mut Target, doc: &mut DocView) -> String {
        let opts = self.docx_options();
        if action == FindAction::ReplaceAll {
            let command = Command::DocxReplaceAll {
                find: opts.pattern,
                replacement: self.replace.clone(),
                match_case: opts.match_case,
                whole_word: opts.whole_word,
                wildcards: opts.wildcards,
                clipboard: None,
            };
            return match target.apply(command) {
                Ok(out) => format!("{} replacements made", out.replaced),
                Err(e) => format!("Replace failed: {e}"),
            };
        }
        let matches = match target.docx().map(|d| d.find_all(&opts)) {
            Some(Ok(m)) => m,
            Some(Err(e)) => return format!("Find failed: {e}"),
            None => return String::new(),
        };
        if matches.is_empty() {
            return "No matches".to_owned();
        }
        let (start, _) = doc.selection();
        let key = |m: &open365_commands::DocMatch| (m.para, m.start);
        let from = (start.para, start.offset);
        let after = doc.has_selection();
        let pick = if action == FindAction::Next {
            matches
                .iter()
                .find(|m| if after { key(m) > from } else { key(m) >= from })
                .or(matches.first())
        } else {
            matches
                .iter()
                .rev()
                .find(|m| key(m) < (start.para, start.offset))
                .or(matches.last())
        };
        match pick {
            Some(m) => {
                doc.select(DocPos::new(m.para, m.start), DocPos::new(m.para, m.end));
                format!("{} matches", matches.len())
            }
            None => "No matches".to_owned(),
        }
    }

    fn run_xlsx(&self, action: FindAction, target: &mut Target, grid: &mut GridView) -> String {
        let find = self.xlsx_find(grid);
        if action == FindAction::ReplaceAll {
            let command = Command::XlsxReplaceAll {
                find,
                replacement: self.replace.clone(),
            };
            return match target.apply(command) {
                Ok(out) => {
                    grid.invalidate_layout();
                    format!(
                        "{} replacements made in {} cells",
                        out.replaced,
                        out.changed_cells.len()
                    )
                }
                Err(e) => format!("Replace failed: {e}"),
            };
        }
        let Some(wb) = target.xlsx() else {
            return String::new();
        };
        let hits = xlsx_find_all(wb, &find);
        let (row, col) = grid.active();
        let here = Some(open365_commands::CellRef {
            sheet: grid.sheet(),
            row,
            col,
        });
        match next_match(&hits, here, action == FindAction::Next, find.by_columns) {
            Some(hit) => {
                if hit.sheet != grid.sheet() {
                    grid.select_sheet(hit.sheet);
                }
                grid.set_active(hit.row, hit.col);
                format!("{} cells match", hits.len())
            }
            None => "No matches".to_owned(),
        }
    }
}
