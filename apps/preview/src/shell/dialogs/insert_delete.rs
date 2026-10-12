//! The Insert and Delete dialogs of a partial selection (Ctrl+Shift+Plus, Ctrl+Minus, and the
//! cell context menu): shift cells right or down (left or up), or the entire rows or columns.

use open365_ui_core::Ui;
use open365_ui_core::dialog::{Dialog, Spec, Values};

use super::View;
use crate::dialogs::request::DialogRequest;
use crate::sheet::CellShift;
use crate::shell::Shell;

const INSERT: [&str; 4] = [
    "Shift cells r&ight",
    "Shift cells &down",
    "Entire &row",
    "Entire &column",
];
const DELETE: [&str; 4] = [
    "Shift cells &left",
    "Shift cells &up",
    "Entire &row",
    "Entire &column",
];

#[derive(Debug)]
struct InsertDelete {
    insert: bool,
}

impl Shell {
    /// Ctrl+Shift+Plus on cells: asks how to make room. Whole rows or columns insert at once.
    pub fn open_insert_dialog(&mut self, ui: &mut Ui) {
        self.open_insert_delete(ui, true);
    }

    /// Ctrl+Minus on cells.
    pub fn open_delete_dialog(&mut self, ui: &mut Ui) {
        self.open_insert_delete(ui, false);
    }

    fn open_insert_delete(&mut self, ui: &mut Ui, insert: bool) {
        let (r1, c1, r2, c2) = self.sheet.range();
        let whole_cols = r1 == 1 && r2 == crate::sheet::MAX_ROW;
        let whole_rows = c1 == 1 && c2 == crate::sheet::MAX_COL;
        if whole_rows || whole_cols {
            // Entire rows or columns are selected: no question to ask.
            self.insert_rows_cols(ui, whole_rows, insert);
            return;
        }
        self.open_dialog(ui, DialogRequest::InsertDelete { insert });
    }
}

pub(super) fn make(s: &mut Shell, insert: bool) -> (Spec, Box<dyn View>) {
    let title = if insert { "Insert" } else { "Delete" };
    let labels = if insert { INSERT } else { DELETE };
    // Excel remembers the last choice; the first time it offers shifting down (up).
    let choice = s.sheet.ui.last_shift[usize::from(!insert)].unwrap_or(1);
    let mut spec = Spec::new(title, 252.0, 124.0);
    spec.group(12.0, 8.0, 228.0, 108.0, title);
    for (i, label) in labels.iter().enumerate() {
        spec.radio(
            &format!("c{i}"),
            "choice",
            24.0,
            26.0 + 22.0 * i as f32,
            204.0,
            label,
            i == choice,
        );
    }
    spec.focus = Some(format!("c{choice}"));
    (spec, Box::new(InsertDelete { insert }))
}

impl View for InsertDelete {
    fn commit(
        &mut self,
        s: &mut Shell,
        _ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let choice = v
            .radio("choice")
            .strip_prefix('c')
            .and_then(|n| n.parse::<usize>().ok())
            .unwrap_or(1);
        s.sheet.ui.last_shift[usize::from(!self.insert)] = Some(choice);
        let _ = match (self.insert, choice) {
            (true, 0) => s.sheet.insert_cells(CellShift::Right),
            (true, 1) => s.sheet.insert_cells(CellShift::Down),
            (false, 0) => s.sheet.delete_cells(CellShift::Left),
            (false, 1) => s.sheet.delete_cells(CellShift::Up),
            (true, c) => s.sheet.insert_selected(c == 2),
            (false, c) => s.sheet.delete_selected(c == 2),
        };
        Ok(())
    }
}
