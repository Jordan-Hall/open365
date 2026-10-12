//! The dialog host: every dialog of both apps is a `ui-core` [`Dialog`] described by a [`Spec`]
//! and driven by a [`View`] that reads the model, reacts to changes and answers by calling the
//! model APIs or `run_command("<id>=<value>")`.
//!
//! Entry points (all on [`Shell`]):
//!
//! * [`Shell::open_dialog`] opens a [`DialogRequest`].
//! * [`Shell::poll_dialog_requests`] opens what the Sheets models asked for with
//!   `take_request()` (call it after a command was dispatched).
//! * [`Shell::try_open_dialog`] opens the dialog a bare ribbon command stands for (call it
//!   before dispatching; `true` means the command is consumed).
//! * [`Shell::dialog_handle`] takes the `Emit`s of the widget tree (the Sheets grid already
//!   routes them through `dialog_event`).
//!
//! The open dialogs live in a table on the `Shell` keyed by the serial in the scrim's label.

use std::cell::Cell;

use open365_ui_core::dialog::{Dialog, Icons, Outcome, Response, Spec, Values, scrim_serial};
use open365_ui_core::{Emit, NodeId, Ui};

use crate::AppKind;
use crate::dialogs::request::DialogRequest;
use crate::ribbon;
use crate::shell::Shell;

mod data;
mod format;
mod formulas;
mod insert;
mod insert_delete;
mod page;
mod paste_special;
mod review;
mod writer;

/// What a dialog does besides showing controls.
pub(super) trait View: std::fmt::Debug {
    /// The dialog is on screen: set the initial enabled states and texts that depend on others.
    fn opened(&mut self, _s: &mut Shell, _ui: &mut Ui, _d: &mut Dialog) {}

    /// A control changed, a page was shown or a custom button was pressed.
    fn event(&mut self, _s: &mut Shell, _ui: &mut Ui, _d: &mut Dialog, _o: &Outcome) {}

    /// OK or Apply. `Err` is shown in the dialog, which stays open.
    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String>;

    /// Cancel, Esc or the close button.
    fn cancel(&mut self, _s: &mut Shell, _ui: &mut Ui) {}
}

#[derive(Debug)]
pub(in crate::shell) struct Entry {
    dialog: Dialog,
    view: Box<dyn View>,
    /// The control that had the focus when the grid last rebuilt, restored afterwards.
    focus: Option<NodeId>,
}

thread_local! {
    static CLOSE: Cell<bool> = const { Cell::new(false) };
    static SERIAL: Cell<u64> = const { Cell::new(0) };
}

/// Asks the host to close the dialog whose view is handling an event (a button that finishes
/// the dialog without OK, such as Remove All Subtotals).
pub(super) fn request_close() {
    CLOSE.with(|c| c.set(true));
}

fn next_serial() -> u64 {
    SERIAL.with(|s| {
        s.set(s.get() + 1);
        s.get()
    })
}

/// Builds the spec and the view of a request, `None` when the request does not apply (no
/// workbook, no formula in the cell...).
fn make(s: &mut Shell, req: &DialogRequest) -> Option<(Spec, Box<dyn View>)> {
    use DialogRequest as R;
    match req {
        R::InsertDelete { insert } => Some(insert_delete::make(s, *insert)),
        R::PasteSpecial => Some(paste_special::make(s)),
        R::InsertFunction { .. }
        | R::FunctionArguments { .. }
        | R::NameManager
        | R::EvaluateFormula => formulas::make(s, req),
        R::Sort
        | R::AdvancedFilter
        | R::TextToColumns
        | R::RemoveDuplicates
        | R::DataValidation
        | R::Consolidate
        | R::GoalSeek
        | R::Group
        | R::Subtotal
        | R::CustomFilter { .. } => data::make(s, req),
        R::ProtectSheet | R::ProtectWorkbook => review::make(s, req),
        R::FormatCells { .. }
        | R::RowHeight
        | R::ColumnWidth
        | R::MergeConfirm { .. }
        | R::ValidationAlert { .. } => format::make(s, req),
        R::PageSetup { .. } | R::Zoom => page::make(s, req),
        R::CreateTable | R::CreatePivot | R::InsertLink | R::Symbol => insert::make(s, req),
        R::Writer(w) => writer::make(s, *w),
    }
}

impl Shell {
    /// True while a dialog is open.
    pub fn dialog_open(&self) -> bool {
        !self.dialogs.is_empty()
    }

    /// The icons the dialogs draw.
    fn dialog_icons(&self) -> Icons {
        Icons {
            close: self.icon("x"),
            collapse: self.icon("table-options"),
        }
    }

    /// Opens `req` over the window; false when it does not apply. Any dialog already open is
    /// cancelled first.
    pub fn open_dialog(&mut self, ui: &mut Ui, req: DialogRequest) -> bool {
        self.close_dialog(ui);
        let Some((spec, mut view)) = make(self, &req) else {
            return false;
        };
        self.group_popup = None;
        debug_assert!(
            spec.access_conflicts().is_empty(),
            "{}: access keys used twice: {:?}",
            spec.title,
            spec.access_conflicts()
        );
        let serial = next_serial();
        let mut dialog = Dialog::new(spec, serial);
        dialog.open(ui, self.dialog_icons());
        view.opened(self, ui, &mut dialog);
        self.dialogs.insert(
            serial,
            Entry {
                focus: ui.focus(),
                dialog,
                view,
            },
        );
        true
    }

    /// Cancels the open dialog (Esc) and returns the focus to the document.
    pub fn close_dialog(&mut self, ui: &mut Ui) {
        let Some(serial) = scrim_serial(ui) else {
            return;
        };
        if let Some(mut e) = self.dialogs.remove(&serial) {
            e.view.cancel(self, ui);
            e.dialog.close(ui);
        }
        self.dialog_closed(ui, false);
    }

    /// Opens the dialog the Sheets models asked for (`take_request()` of the formulas, data and
    /// review state). True when one opened.
    pub fn poll_dialog_requests(&mut self, ui: &mut Ui) -> bool {
        if self.kind != AppKind::Sheets {
            return false;
        }
        let req = self
            .sheet
            .take_request()
            .map(DialogRequest::from)
            .or_else(|| self.sheet.ext_data.take_request().map(DialogRequest::from))
            .or_else(|| {
                // Spelling, accessibility and statistics have their own panes: leave those.
                let r = self.sheet.ext_review.take_request()?;
                let dialog = DialogRequest::from_review(r);
                if dialog.is_none() {
                    self.sheet.ext_review.request(r);
                }
                dialog
            });
        match req {
            Some(r) => self.open_dialog(ui, r),
            None => false,
        }
    }

    /// Opens the dialog ribbon command `id` stands for when it is sent without a value. True
    /// when the command was consumed (the caller must not dispatch it).
    pub fn try_open_dialog(&mut self, ui: &mut Ui, id: &str) -> bool {
        let writer = self.kind == AppKind::Writer;
        if let Some(req) = DialogRequest::for_command(id, writer) {
            return self.open_dialog(ui, req);
        }
        if !writer && let Some(req) = format::merge_request(self, id) {
            return self.open_dialog(ui, req);
        }
        false
    }

    /// Runs a ribbon command without going through the dialog hooks.
    pub(super) fn run_id(&mut self, ui: &mut Ui, id: &str) -> ribbon::Handled {
        let mut out = Vec::new();
        let handled = self.dispatch_command(ui, id, &mut out);
        self.apply_writer_zoom(ui);
        self.sync_writer_state(ui);
        handled
    }

    /// Runs `commands` in order; the first model error stops the run.
    pub(super) fn run_ids(&mut self, ui: &mut Ui, commands: &[String]) {
        for c in commands {
            self.run_id(ui, c);
        }
    }

    /// Raises the dialog over nodes added after it opened and gives back the control to focus.
    pub(in crate::shell) fn build_dialog(&mut self, ui: &mut Ui) -> Option<NodeId> {
        let serial = scrim_serial(ui)?;
        let e = self.dialogs.remove(&serial)?;
        e.dialog.raise(ui);
        let focus = e.focus.filter(|f| ui.tree.contains(*f));
        self.dialogs.insert(serial, e);
        focus
    }

    /// The window changed size: recentre the dialog.
    pub fn dialog_resize(&mut self, ui: &mut Ui) {
        let Some(serial) = scrim_serial(ui) else {
            return;
        };
        if let Some(mut e) = self.dialogs.remove(&serial) {
            e.dialog.relayout(ui);
            self.dialogs.insert(serial, e);
        }
    }

    /// Handles an event of the dialog's controls. True when the dialog used the event.
    pub(super) fn dialog_event(&mut self, ui: &mut Ui, ev: &Emit) -> bool {
        self.dialog_handle(ui, ev)
    }

    /// Routes one widget event to the open dialog; true when it was the dialog's. Call this for
    /// every event before the shell's own handling (the Sheets grid does through
    /// `dialog_event`, the Writer needs a call at the top of `Shell::handle`).
    pub fn dialog_handle(&mut self, ui: &mut Ui, ev: &Emit) -> bool {
        let Some(serial) = scrim_serial(ui) else {
            return false;
        };
        let Some(mut e) = self.dialogs.remove(&serial) else {
            return false;
        };
        e.dialog.raise(ui);
        let resp = e.dialog.handle(ui, ev);
        let consumed = !matches!(resp, Response::Ignored);
        if let Some(f) = ui.focus()
            && ui.tree.shown(f)
            && consumed
        {
            e.focus = Some(f);
        }
        let mut close = None;
        if let Response::Outcome(o) = resp {
            close = self.dialog_outcome(ui, &mut e, &o);
        }
        match close {
            Some(applied) => {
                e.dialog.close(ui);
                self.dialog_closed(ui, applied);
                if let Some(next) = formulas::take_queued() {
                    self.open_dialog(ui, next);
                }
            }
            None => {
                self.dialogs.insert(serial, e);
            }
        }
        consumed
    }

    /// `Some(applied)` when the dialog is finished.
    fn dialog_outcome(&mut self, ui: &mut Ui, e: &mut Entry, o: &Outcome) -> Option<bool> {
        let Outcome::Pressed(id) = o else {
            e.view.event(self, ui, &mut e.dialog, o);
            return CLOSE.with(Cell::take).then_some(false);
        };
        match id.as_str() {
            "ok" | "apply" => {
                let v = e.dialog.values(ui);
                match e.view.commit(self, ui, &mut e.dialog, &v) {
                    Ok(()) => {
                        e.dialog.set_error(ui, "");
                        (id == "ok").then_some(true)
                    }
                    Err(msg) => {
                        e.dialog.set_error(ui, &msg);
                        None
                    }
                }
            }
            "cancel" => {
                e.view.cancel(self, ui);
                Some(false)
            }
            _ => {
                e.view.event(self, ui, &mut e.dialog, o);
                CLOSE.with(Cell::take).then_some(true)
            }
        }
    }

    /// After a dialog closed: rebuild what it changed and give the document the focus.
    fn dialog_closed(&mut self, ui: &mut Ui, _applied: bool) {
        match self.kind {
            AppKind::Sheets => {
                self.sheet.ui.nodes.clear();
                self.sync_bars(ui);
                self.build_grid(ui);
                ui.set_focus(self.canvas);
            }
            AppKind::Writer => {
                self.sync_writer_state(ui);
                let canvas = self
                    .writer
                    .as_ref()
                    .and_then(crate::doc::WriterPane::canvas);
                ui.set_focus(canvas);
            }
        }
    }
}

// ---- helpers the views share ---------------------------------------------------------------

/// A 1-based column number as letters.
pub(super) fn col_letters(c: u32) -> String {
    crate::sheet::a1(1, c).trim_end_matches('1').to_owned()
}

/// `A1` or `A1:B2` from a range.
pub(super) fn range_text(r: (u32, u32, u32, u32)) -> String {
    if r.0 == r.2 && r.1 == r.3 {
        crate::sheet::a1(r.0, r.1)
    } else {
        format!(
            "{}:{}",
            crate::sheet::a1(r.0, r.1),
            crate::sheet::a1(r.2, r.3)
        )
    }
}

/// `$A$1:$B$2` as Excel's reference boxes show a selection.
pub(super) fn abs_range_text(r: (u32, u32, u32, u32)) -> String {
    let cell = |row: u32, col: u32| format!("${}${row}", col_letters(col));
    if r.0 == r.2 && r.1 == r.3 {
        cell(r.0, r.1)
    } else {
        format!("{}:{}", cell(r.0, r.1), cell(r.2, r.3))
    }
}

/// Parses a range or cell typed in a reference box (`$A$1:$B$2`, `a1`).
pub(super) fn parse_ref(text: &str) -> Option<(u32, u32, u32, u32)> {
    let t = text.trim().replace('$', "");
    if t.is_empty() {
        return None;
    }
    if let Some(r) = open365_xlsx::parse_range(&t) {
        return Some(r);
    }
    crate::sheet::parse_a1(&t).map(|(r, c)| (r, c, r, c))
}

/// The selection as a reference-box text.
pub(super) fn selection_text(s: &Shell) -> String {
    abs_range_text(s.sheet.range())
}

/// Writes the current selection into reference field `id` after the dialog expanded again.
pub(super) fn pick_selection(s: &Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
    if let Outcome::Ref {
        id,
        collapsed: false,
    } = o
    {
        d.set_text(ui, id, &selection_text(s));
    }
}

/// Breaks `text` into at most `max` lines of about `cols` characters, at spaces.
pub(super) fn wrap(text: &str, cols: usize, max: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > cols {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines.truncate(max);
    lines
}
