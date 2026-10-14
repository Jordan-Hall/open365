//! Review tab on the sheet model: spelling (crates/spell), the accessibility check, workbook
//! statistics, notes, and sheet and workbook protection. Edits are `ReviewOp` commands (one undo
//! step); the ribbon module `ribbon::sheets::review` maps control ids to these methods.

use open365_commands::review_ops::{self, Note, Password, SheetProtection, Thread};
use open365_commands::{BorderChange, CellFormatChange, Command, RangeRef, ReviewOp};
use open365_xlsx::CellValue;

use super::super::SheetModel;

/// A dialog the shell should open for a Review command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    Spelling,
    Accessibility,
    Statistics,
    ProtectSheet,
    ProtectWorkbook,
    /// A protected sheet needs its password before it can be unprotected.
    UnprotectSheet,
    UnprotectWorkbook,
}

/// What the Review tab keeps on the sheet model (none of it is in the file or the undo history).
#[derive(Debug, Default)]
pub struct ReviewState {
    request: Option<Request>,
    /// Words the user chose to ignore for the rest of the session.
    pub ignored: Vec<String>,
    /// The last spelling run.
    pub spelling: Vec<SpellIssue>,
    /// Show Comments: the comments pane is open.
    pub show_comments: bool,
}

impl ReviewState {
    /// The dialog the last command asked for, once.
    pub fn take_request(&mut self) -> Option<Request> {
        self.request.take()
    }

    pub fn request(&mut self, r: Request) {
        self.request = Some(r);
    }
}

/// A word the spelling check flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpellIssue {
    pub row: u32,
    pub col: u32,
    pub word: String,
    pub repeated: bool,
    pub suggestions: Vec<String>,
}

/// How serious an accessibility finding is, as the Accessibility pane groups them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Tip,
}

/// One accessibility finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessIssue {
    pub severity: Severity,
    pub rule: &'static str,
    /// Where: a sheet name and cell or range.
    pub place: String,
}

/// The numbers Workbook Statistics lists.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkbookStats {
    pub worksheets: usize,
    pub hidden_worksheets: usize,
    pub chart_sheets: usize,
    pub cells_with_data: usize,
    pub formulas: usize,
    pub tables: usize,
    pub charts: usize,
    pub merged_areas: usize,
    pub defined_names: usize,
    pub notes: usize,
}

impl WorkbookStats {
    /// The statistics as the dialog lists them, label and value.
    #[must_use]
    pub fn lines(&self) -> Vec<(&'static str, usize)> {
        vec![
            ("Worksheets", self.worksheets),
            ("Hidden worksheets", self.hidden_worksheets),
            ("Chart sheets", self.chart_sheets),
            ("Cells with data", self.cells_with_data),
            ("Formulas", self.formulas),
            ("Tables", self.tables),
            ("Charts", self.charts),
            ("Merged areas", self.merged_areas),
            ("Named ranges", self.defined_names),
            ("Notes", self.notes),
        ]
    }
}

/// What the command layer says for a wrong password, and Excel's full text for it.
const PASSWORD_WRONG: &str = "The password you supplied is not correct.";
pub const PASSWORD_WRONG_MESSAGE: &str = "The password you supplied is not correct. Verify that the CAPS LOCK key is off and be sure to use the correct capitalization.";

/// The message Excel shows when a protected sheet refuses an edit.
pub const PROTECTED_MESSAGE: &str = "The cell or chart you're trying to change is on a protected sheet. To make a change, unprotect the sheet. You might be requested to enter a password.";

fn user_name() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .ok()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "Author".to_owned())
}

impl SheetModel {
    fn review_cmd(&mut self, op: ReviewOp) -> Result<Vec<String>, String> {
        match self.target.apply(Command::ReviewOp {
            sheet: self.sheet,
            op,
        }) {
            Ok(o) => {
                self.status = o.notes.join(" ");
                Ok(o.notes)
            }
            Err(e) => {
                self.status = e.to_string();
                Err(self.status.clone())
            }
        }
    }

    // ---- protection ----------------------------------------------------------------------

    /// What the active sheet's protection allows, `None` when it is not protected.
    #[must_use]
    pub fn review_sheet_protection(&self) -> Option<SheetProtection> {
        review_ops::sheet_protection(self.target.xlsx()?, self.sheet)
    }

    /// Protect Sheet's OK. While the dialog that asks for the options is still to open (a bare
    /// click on the button left the request), nothing is protected: as in Excel, the dialog
    /// comes first.
    pub fn review_protect_sheet(
        &mut self,
        options: SheetProtection,
        password: Option<&str>,
    ) -> Result<(), String> {
        if self.ext_review.request == Some(Request::ProtectSheet) {
            return Ok(());
        }
        self.review_cmd(ReviewOp::ProtectSheet {
            options,
            password: password.map(Password::new),
        })
        .map(|_| ())
    }

    /// Unprotect Sheet. Without a password, a sheet that has one asks for it in a dialog (the
    /// request is left for the shell) instead of failing, as Excel's button does.
    pub fn review_unprotect_sheet(&mut self, password: Option<&str>) -> Result<(), String> {
        let done = self
            .review_cmd(ReviewOp::UnprotectSheet {
                password: password.map(Password::new),
            })
            .map(|_| ());
        self.password_prompt(password.is_none(), Request::UnprotectSheet, done)
    }

    /// Turns a wrong-password failure into Excel's text, or into the prompt when none was given.
    fn password_prompt(
        &mut self,
        ask: bool,
        request: Request,
        done: Result<(), String>,
    ) -> Result<(), String> {
        match done {
            Err(e) if e.contains(PASSWORD_WRONG) && ask => {
                self.ext_review.request(request);
                self.status.clear();
                Ok(())
            }
            Err(e) if e.contains(PASSWORD_WRONG) => {
                self.status = PASSWORD_WRONG_MESSAGE.to_owned();
                Err(PASSWORD_WRONG_MESSAGE.to_owned())
            }
            other => other,
        }
    }

    /// Whether Protect Workbook (structure) is on.
    #[must_use]
    pub fn review_workbook_protected(&self) -> bool {
        self.target
            .xlsx()
            .is_some_and(review_ops::workbook_structure_locked)
    }

    /// Protect Workbook's OK; the dialog comes first, as for the sheet.
    pub fn review_protect_workbook(&mut self, password: Option<&str>) -> Result<(), String> {
        if self.ext_review.request == Some(Request::ProtectWorkbook) {
            return Ok(());
        }
        self.review_cmd(ReviewOp::ProtectWorkbook {
            password: password.map(Password::new),
        })
        .map(|_| ())
    }

    pub fn review_unprotect_workbook(&mut self, password: Option<&str>) -> Result<(), String> {
        let done = self
            .review_cmd(ReviewOp::UnprotectWorkbook {
                password: password.map(Password::new),
            })
            .map(|_| ());
        self.password_prompt(password.is_none(), Request::UnprotectWorkbook, done)
    }

    /// Whether a protected sheet refuses to edit the cell: protected and the cell is locked.
    #[must_use]
    pub fn review_cell_protected(&self, row: u32, col: u32) -> bool {
        let Some(wb) = self.target.xlsx() else {
            return false;
        };
        review_ops::sheet_protection(wb, self.sheet).is_some()
            && review_ops::cell_locked(wb, self.sheet, row, col)
            && !self.review_in_open_edit_range(row, col)
    }

    /// Typing into a cell, refused with Excel's message on a protected sheet when the cell is
    /// locked. Also applies the cell's data validation.
    pub fn review_enter(&mut self, row: u32, col: u32, text: &str) -> Result<(), String> {
        if self.review_cell_protected(row, col) {
            self.status = PROTECTED_MESSAGE.to_owned();
            return Err(PROTECTED_MESSAGE.to_owned());
        }
        self.data_enter(row, col, text)
    }

    // ---- notes ---------------------------------------------------------------------------

    /// The notes of the active sheet in reading order (row by row).
    #[must_use]
    pub fn review_notes(&self) -> Vec<Note> {
        let mut notes = self
            .target
            .xlsx()
            .map(|wb| review_ops::notes(wb, self.sheet))
            .unwrap_or_default();
        notes.sort_by_key(|n| (n.row, n.col));
        notes
    }

    /// The note of a cell.
    #[must_use]
    pub fn review_note_at(&self, row: u32, col: u32) -> Option<Note> {
        self.review_notes()
            .into_iter()
            .find(|n| (n.row, n.col) == (row, col))
    }

    /// New Note / Edit Note: sets the note text of the active cell.
    pub fn review_set_note(&mut self, text: &str) -> Result<(), String> {
        let (row, col) = self.active;
        let author = self
            .review_note_at(row, col)
            .map_or_else(user_name, |n| n.author);
        self.review_cmd(ReviewOp::SetNote(Note {
            row,
            col,
            author,
            text: text.to_owned(),
        }))
        .map(|_| ())
    }

    /// Delete Note on the active cell.
    pub fn review_delete_note(&mut self) -> Result<(), String> {
        let (row, col) = self.active;
        if self.review_note_at(row, col).is_none() {
            return Err("The cell has no note.".to_owned());
        }
        self.review_cmd(ReviewOp::DeleteNote { row, col })
            .map(|_| ())
    }

    /// Previous / Next Note: selects the neighbouring cell with a note, wrapping around.
    pub fn review_goto_note(&mut self, forward: bool) -> Result<(), String> {
        let notes = self.review_notes();
        if notes.is_empty() {
            return Err("There are no notes on this sheet.".to_owned());
        }
        let here = self.active;
        let pick = if forward {
            notes
                .iter()
                .find(|n| (n.row, n.col) > here)
                .or_else(|| notes.first())
        } else {
            notes
                .iter()
                .rev()
                .find(|n| (n.row, n.col) < here)
                .or_else(|| notes.last())
        };
        if let Some(n) = pick {
            self.select_cell((n.row, n.col));
        }
        Ok(())
    }

    // ---- threaded comments ---------------------------------------------------------------

    /// The threaded comments of the active sheet in reading order.
    #[must_use]
    pub fn review_comments(&self) -> Vec<Thread> {
        let mut all = self
            .target
            .xlsx()
            .map(|wb| review_ops::threads(wb, self.sheet))
            .unwrap_or_default();
        all.sort_by_key(|t| (t.row, t.col));
        all
    }

    /// The thread on a cell.
    #[must_use]
    pub fn review_comment_at(&self, row: u32, col: u32) -> Option<Thread> {
        self.review_comments()
            .into_iter()
            .find(|t| (t.row, t.col) == (row, col))
    }

    /// New Comment on the active cell; a reply when the cell already has a thread.
    pub fn review_add_comment(&mut self, text: &str) -> Result<(), String> {
        let (row, col) = self.active;
        self.review_cmd(ReviewOp::AddComment {
            row,
            col,
            author: user_name(),
            text: text.to_owned(),
        })
        .map(|_| ())
    }

    /// Delete on the Comments group: the thread of the active cell, else its note.
    pub fn review_delete_comment(&mut self) -> Result<(), String> {
        let (row, col) = self.active;
        if self.review_comment_at(row, col).is_some() {
            return self
                .review_cmd(ReviewOp::DeleteComment { row, col })
                .map(|_| ());
        }
        if self.review_note_at(row, col).is_some() {
            return self.review_delete_note();
        }
        Err("The cell has no comment or note.".to_owned())
    }

    /// Previous / Next Comment: selects the neighbouring cell with a thread, wrapping around.
    pub fn review_goto_comment(&mut self, forward: bool) -> Result<(), String> {
        let all = self.review_comments();
        if all.is_empty() {
            return Err("There are no comments on this sheet.".to_owned());
        }
        let here = self.active;
        let pick = if forward {
            all.iter()
                .find(|t| (t.row, t.col) > here)
                .or_else(|| all.first())
        } else {
            all.iter()
                .rev()
                .find(|t| (t.row, t.col) < here)
                .or_else(|| all.last())
        };
        if let Some(t) = pick {
            self.select_cell((t.row, t.col));
        }
        Ok(())
    }

    // ---- spelling ------------------------------------------------------------------------

    /// Spelling (F7): every misspelled word in the text constants of the active sheet, in
    /// reading order. Formulas are not checked, as in Excel.
    pub fn review_spell_check(&mut self, lang: &str) -> Vec<SpellIssue> {
        let proofing = open365_spell::Proofing::new();
        let mut issues = Vec::new();
        if let Some(wb) = self.target.xlsx() {
            let mut cells: Vec<(u32, u32, String)> = wb
                .iter_cells(self.sheet)
                .filter(|(_, _, c)| c.formula.is_none())
                .filter_map(|(r, c, cell)| match &cell.value {
                    CellValue::Empty | CellValue::Number(_) | CellValue::Bool(_) => None,
                    other => other.as_text().map(|t| (r, c, t.to_owned())),
                })
                .collect();
            cells.sort_by_key(|(r, c, _)| (*r, *c));
            for (row, col, text) in cells {
                let report = proofing.check_paragraph(&text, lang);
                for m in report.issues {
                    if self
                        .ext_review
                        .ignored
                        .iter()
                        .any(|w| w.eq_ignore_ascii_case(&m.word))
                    {
                        continue;
                    }
                    let repeated = matches!(m.kind, open365_spell::IssueKind::RepeatedWord);
                    let suggestions = if repeated {
                        Vec::new()
                    } else {
                        proofing.suggest(&m.word, lang)
                    };
                    issues.push(SpellIssue {
                        row,
                        col,
                        word: m.word,
                        repeated,
                        suggestions,
                    });
                }
            }
        }
        self.status = if issues.is_empty() {
            "Spell check complete. You're good to go!".to_owned()
        } else {
            format!("{} possible spelling mistakes", issues.len())
        };
        if let Some(first) = issues.first() {
            self.select_cell((first.row, first.col));
        }
        self.ext_review.spelling.clone_from(&issues);
        issues
    }

    /// Spelling > Change: replaces the first occurrence of the word in the cell.
    pub fn review_spell_change(
        &mut self,
        issue: &SpellIssue,
        replacement: &str,
    ) -> Result<(), String> {
        let current = self.source(issue.row, issue.col);
        let Some(at) = current.find(&issue.word) else {
            return Err("The word is no longer in the cell.".to_owned());
        };
        let mut next = String::with_capacity(current.len());
        next.push_str(&current[..at]);
        next.push_str(replacement);
        next.push_str(&current[at + issue.word.len()..]);
        self.review_enter(issue.row, issue.col, &next)
    }

    /// Spelling > Ignore All.
    pub fn review_spell_ignore_all(&mut self, word: &str) {
        if !self
            .ext_review
            .ignored
            .iter()
            .any(|w| w.eq_ignore_ascii_case(word))
        {
            self.ext_review.ignored.push(word.to_owned());
        }
    }

    // ---- accessibility -------------------------------------------------------------------

    /// Check Accessibility: findings grouped by severity (errors first).
    #[must_use]
    pub fn review_accessibility(&self) -> Vec<AccessIssue> {
        let Some(wb) = self.target.xlsx() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (i, info) in wb.sheets().iter().enumerate() {
            let name = &info.name;
            let default_name = name
                .strip_prefix("Sheet")
                .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
            if default_name {
                out.push(AccessIssue {
                    severity: Severity::Tip,
                    rule: "Default sheet name",
                    place: name.clone(),
                });
            }
            let Some(ws) = wb.sheet(i) else { continue };
            for t in ws.tables() {
                if t.header_row_count == 0 {
                    out.push(AccessIssue {
                        severity: Severity::Error,
                        rule: "No header row specified",
                        place: format!("{name}!{}", t.range_text),
                    });
                }
            }
            for (r1, c1, r2, c2) in ws.merged_cells() {
                out.push(AccessIssue {
                    severity: Severity::Warning,
                    rule: "Merged cells",
                    place: format!(
                        "{name}!{}:{}",
                        super::super::a1(r1, c1),
                        super::super::a1(r2, c2)
                    ),
                });
            }
        }
        out.sort_by_key(|i| match i.severity {
            Severity::Error => 0,
            Severity::Warning => 1,
            Severity::Tip => 2,
        });
        out
    }

    // ---- statistics ----------------------------------------------------------------------

    /// Workbook Statistics.
    #[must_use]
    pub fn review_statistics(&self) -> WorkbookStats {
        let Some(wb) = self.target.xlsx() else {
            return WorkbookStats::default();
        };
        let mut s = WorkbookStats::default();
        for (i, info) in wb.sheets().iter().enumerate() {
            if matches!(info.kind, open365_xlsx::SheetKind::Worksheet) {
                s.worksheets += 1;
                if !matches!(info.state, open365_xlsx::SheetState::Visible) {
                    s.hidden_worksheets += 1;
                }
                for (_, _, cell) in wb.iter_cells(i) {
                    if cell.formula.is_some() {
                        s.formulas += 1;
                    }
                    if cell.formula.is_some() || !matches!(cell.value, CellValue::Empty) {
                        s.cells_with_data += 1;
                    }
                }
                s.merged_areas += wb.sheet(i).map_or(0, |ws| ws.merged_cells().len());
                s.notes += review_ops::notes(wb, i).len();
            } else {
                s.chart_sheets += 1;
            }
        }
        s.tables = wb.tables().count();
        s.defined_names = wb.defined_names().len();
        s.charts = wb
            .package()
            .parts()
            .filter(|p| {
                let n = p.name().as_str();
                n.starts_with("xl/charts/chart") && n.ends_with(".xml")
            })
            .count();
        s
    }

    // ---- Format Cells dialog ---------------------------------------------------------------

    /// Applies what the Format Cells dialog set beyond the Home tab's buttons (strikethrough,
    /// shrink to fit, any text angle, per-edge border styles and colours) to the selection: the
    /// format change first, then the borders.
    ///
    /// # Errors
    /// The command layer's message.
    pub fn format_selection(
        &mut self,
        change: CellFormatChange,
        borders: Option<BorderChange>,
    ) -> Result<(), String> {
        let (first_row, first_col, last_row, last_col) = self.range();
        let range = RangeRef {
            first_row,
            first_col,
            last_row,
            last_col,
        };
        if change != CellFormatChange::default() {
            self.apply(Command::FormatCells {
                sheet: self.sheet,
                range,
                change: Box::new(change),
            })?;
        }
        if let Some(b) = borders {
            self.apply(Command::SetBorders {
                sheet: self.sheet,
                range,
                borders: Box::new(b),
            })?;
        }
        Ok(())
    }
}
