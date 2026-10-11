//! Cell editing in the Sheets preview: the grid canvas node, the in-cell editor, the formula bar
//! and the Name Box, all wired to the active cell of the [`SheetModel`]. The rules are Excel's:
//!
//! * select a cell and type: the text replaces the contents (Enter mode; the arrows, Tab and
//!   Enter commit and move);
//! * F2 or a double click edits in place with the existing contents (Edit mode; the arrows move
//!   the caret, Tab and Enter commit);
//! * the formula bar edits the active cell and commits with Enter;
//! * Esc cancels; Delete clears; Ctrl+Z and Ctrl+Y undo and redo; Ctrl+S saves.
//!
//! IME composition and the AccessKit tree come with the widgets: the canvas is a `Widget::Canvas`
//! (role Grid, IME allowed, candidate window at the active cell) and the editor a real
//! `Widget::TextField`.

use open365_ui_core::geom::Point;
use open365_ui_core::{ImeInput, Key, Modifiers};

use super::*;
use crate::doc::GridGeom;
use crate::sheet::{self, SheetModel};

/// The in-cell editor while a cell is being edited.
#[derive(Debug, Clone)]
pub(super) struct Editor {
    /// Enter mode (typed to replace): arrows commit and move. Edit mode (F2, double click): they
    /// move the caret.
    pub enter_mode: bool,
    pub text: String,
    pub node: Option<NodeId>,
}

const COMMIT_KEYS: [Key; 3] = [Key::Tab, Key::Enter, Key::Escape];
const ENTER_MODE_KEYS: [Key; 7] = [
    Key::Tab,
    Key::Enter,
    Key::Escape,
    Key::Up,
    Key::Down,
    Key::Left,
    Key::Right,
];

fn step(up: bool) -> i64 {
    if up { -1 } else { 1 }
}

impl Shell {
    pub fn sheet(&self) -> &SheetModel {
        &self.sheet
    }

    pub fn sheet_mut(&mut self) -> &mut SheetModel {
        &mut self.sheet
    }

    /// True while the in-cell editor is open.
    pub fn is_editing(&self) -> bool {
        self.editor.is_some()
    }

    /// The text of the open editor.
    pub fn editor_text(&self) -> Option<&str> {
        self.editor.as_ref().map(|e| e.text.as_str())
    }

    /// The text of the formula bar.
    pub fn formula_text(&self) -> &str {
        &self.formula
    }

    /// Replaces the workbook (a file given on the command line) and redraws.
    pub fn set_sheet(&mut self, ui: &mut Ui, model: SheetModel) {
        self.sheet = model;
        self.editor = None;
        self.sync_bars(ui);
        self.build_grid(ui);
    }

    pub(super) fn grid_geom(&self) -> GridGeom {
        let hdr = (
            self.spec.row_header_w.unwrap_or(34),
            self.spec.col_header_h.unwrap_or(24),
        );
        GridGeom::new(self.content, self.scale, hdr)
    }

    /// Rebuilds the canvas node (and the editor over the active cell) and puts the keyboard focus
    /// where the user is typing.
    pub(super) fn build_grid(&mut self, ui: &mut Ui) {
        let old = std::mem::take(&mut self.grid_ids);
        let in_bar = ui
            .focus()
            .is_some_and(|f| self.roles.formula == Some(f) || self.roles.name_box == Some(f));
        for id in old {
            ui.tree.remove(id);
        }
        self.canvas = None;
        if self.kind != AppKind::Sheets
            || self.backstage.is_some()
            || self.content.w <= 0
            || self.content.h <= 0
        {
            self.editor = None;
            return;
        }
        let (rows, cols) = self.grid_geom().visible();
        self.sheet.reveal_active(rows, cols);
        let g = self.grid_geom();
        let (l, t, r, b) = g.cell(
            self.sheet.top_left,
            self.sheet.active.0,
            self.sheet.active.1,
        );
        let root = ui.tree.root();
        let area = self.content;
        let caret = Rect::new(
            (l - area.x as f32) / self.scale,
            (t - area.y as f32) / self.scale,
            (r - l) / self.scale,
            (b - t) / self.scale,
        );
        let label = format!(
            "Sheet1 grid, cell {}",
            sheet::a1(self.sheet.active.0, self.sheet.active.1)
        );
        let canvas = self.add(
            ui,
            root,
            (0, 0),
            area,
            Widget::Canvas { caret },
            &label,
            Look::default(),
            Act::Inert,
        );
        self.grid_ids.push(canvas);
        self.canvas = Some(canvas);
        if let Some(ed) = self.editor.clone() {
            let cell = PxRect {
                x: l as i32,
                y: t as i32,
                w: (r - l) as i32,
                h: (b - t) as i32,
            };
            let node = self.add(
                ui,
                root,
                (0, 0),
                cell,
                Widget::TextField(TextEdit::new(&ed.text)),
                "Cell editor",
                Look::default(),
                Act::Inert,
            );
            if let Some(n) = ui.tree.get_mut(node) {
                n.intercept = if ed.enter_mode {
                    ENTER_MODE_KEYS.to_vec()
                } else {
                    COMMIT_KEYS.to_vec()
                };
            }
            self.grid_ids.push(node);
            if let Some(e) = self.editor.as_mut() {
                e.node = Some(node);
            }
            ui.set_focus(Some(node));
        } else if !in_bar {
            ui.set_focus(Some(canvas));
        }
    }

    /// Takes the formula bar and the Name Box from the active cell.
    pub(super) fn sync_bars(&mut self, ui: &mut Ui) {
        let (r, c) = self.sheet.active;
        self.name_box = sheet::a1(r, c);
        self.formula = self.sheet.source(r, c);
        for (node, text) in [
            (self.roles.formula, self.formula.clone()),
            (self.roles.name_box, self.name_box.clone()),
        ] {
            if let Some(n) = node.and_then(|n| ui.tree.get_mut(n)) {
                match &mut n.widget {
                    Widget::TextField(e) | Widget::Combo { edit: e, .. } => {
                        *e = TextEdit::new(&text);
                    }
                    _ => {}
                }
            }
        }
    }

    fn begin_edit(&mut self, ui: &mut Ui, replace: bool, enter_mode: bool) {
        if self.editor.is_some() {
            return;
        }
        let (r, c) = self.sheet.active;
        let text = if replace {
            String::new()
        } else {
            self.sheet.source(r, c)
        };
        self.editor = Some(Editor {
            enter_mode,
            text,
            node: None,
        });
        self.build_grid(ui);
    }

    /// Writes the editor text to the active cell and moves by `by` (rows, columns).
    fn commit_edit(&mut self, ui: &mut Ui, by: (i64, i64)) {
        if let Some(ed) = self.editor.take() {
            let (r, c) = self.sheet.active;
            let _ = self.sheet.enter(r, c, &ed.text);
        }
        self.move_active(ui, by);
    }

    fn cancel_edit(&mut self, ui: &mut Ui) {
        self.editor = None;
        self.sync_bars(ui);
        self.build_grid(ui);
        ui.set_focus(self.canvas);
    }

    pub(super) fn move_active(&mut self, ui: &mut Ui, by: (i64, i64)) {
        self.sheet.move_by(by.0, by.1);
        self.sync_bars(ui);
        self.build_grid(ui);
        ui.set_focus(self.canvas);
    }

    fn select(&mut self, ui: &mut Ui, cell: (u32, u32)) {
        self.sheet.active = cell;
        self.sync_bars(ui);
        self.build_grid(ui);
        ui.set_focus(self.canvas);
    }

    /// Handles the events of the grid, the editor, the formula bar and the Name Box. Returns true
    /// when the event was one of them.
    pub(super) fn grid_event(&mut self, ui: &mut Ui, ev: &Emit) -> bool {
        let editor_node = self.editor.as_ref().and_then(|e| e.node);
        match ev {
            Emit::CanvasPointer { at, clicks, .. } => {
                if self.editor.is_some() {
                    self.commit_edit(ui, (0, 0));
                }
                let x = self.content.x as f32 + at.x * self.scale;
                let y = self.content.y as f32 + at.y * self.scale;
                if let Some(cell) = self.grid_geom().hit(self.sheet.top_left, x, y) {
                    self.select(ui, cell);
                    if *clicks >= 2 {
                        self.begin_edit(ui, false, false);
                    }
                }
                true
            }
            Emit::CanvasText(n, s) if Some(*n) == self.canvas => {
                self.begin_edit(ui, true, true);
                ui.handle(InputEvent::Text(s.clone()));
                true
            }
            Emit::CanvasIme(n, i) if Some(*n) == self.canvas => {
                self.begin_edit(ui, true, true);
                ui.handle(InputEvent::Ime(ImeInput::Enabled));
                ui.handle(InputEvent::Ime(i.clone()));
                true
            }
            Emit::KeyPressed(n, key, m) => {
                if Some(*n) == self.canvas {
                    self.canvas_key(ui, *key, *m);
                    true
                } else if editor_node == Some(*n) {
                    self.editor_key(ui, *key, *m);
                    true
                } else if Some(*n) == self.roles.formula && *key == Key::Escape {
                    self.sync_bars(ui);
                    ui.set_focus(self.canvas);
                    true
                } else {
                    false
                }
            }
            Emit::TextChanged(n, t) if editor_node == Some(*n) => {
                if let Some(e) = self.editor.as_mut() {
                    e.text.clone_from(t);
                }
                self.formula.clone_from(t);
                if let Some(f) = self.roles.formula.and_then(|f| ui.tree.get_mut(f))
                    && let Widget::TextField(e) = &mut f.widget
                {
                    *e = TextEdit::new(t);
                }
                true
            }
            Emit::TextCommitted(n, t) if Some(*n) == self.roles.formula => {
                self.formula.clone_from(t);
                self.editor = None;
                let (r, c) = self.sheet.active;
                let _ = self.sheet.enter(r, c, t);
                self.move_active(ui, (1, 0));
                true
            }
            Emit::TextCommitted(n, t) if Some(*n) == self.roles.name_box => {
                match sheet::parse_a1(t) {
                    Some(cell) => self.select(ui, cell),
                    None => self.sync_bars(ui),
                }
                true
            }
            _ => false,
        }
    }

    fn canvas_key(&mut self, ui: &mut Ui, key: Key, m: Modifiers) {
        match key {
            Key::Up => self.move_active(ui, (-1, 0)),
            Key::Down => self.move_active(ui, (1, 0)),
            Key::Left => self.move_active(ui, (0, -1)),
            Key::Right => self.move_active(ui, (0, 1)),
            Key::Tab => self.move_active(ui, (0, step(m.shift))),
            Key::Enter => self.move_active(ui, (step(m.shift), 0)),
            Key::PageDown => {
                let (rows, _) = self.grid_geom().visible();
                self.move_active(ui, (i64::from(rows), 0));
            }
            Key::PageUp => {
                let (rows, _) = self.grid_geom().visible();
                self.move_active(ui, (-i64::from(rows), 0));
            }
            Key::Home => {
                let (r, c) = self.sheet.active;
                let rows = if m.ctrl { i64::from(r) - 1 } else { 0 };
                self.move_active(ui, (-rows, 1 - i64::from(c)));
            }
            Key::F2 => self.begin_edit(ui, false, false),
            Key::Delete => {
                let (r, c) = self.sheet.active;
                let _ = self.sheet.clear(r, c);
                self.sync_bars(ui);
            }
            Key::Backspace => {
                let (r, c) = self.sheet.active;
                let _ = self.sheet.clear(r, c);
                self.begin_edit(ui, true, true);
            }
            Key::Char('Z') if m.ctrl => {
                self.sheet.undo();
                self.sync_bars(ui);
            }
            Key::Char('Y') if m.ctrl => {
                self.sheet.redo();
                self.sync_bars(ui);
            }
            Key::Char('S') if m.ctrl => self.save(),
            _ => {}
        }
    }

    fn editor_key(&mut self, ui: &mut Ui, key: Key, m: Modifiers) {
        match key {
            Key::Escape => self.cancel_edit(ui),
            Key::Enter => self.commit_edit(ui, (step(m.shift), 0)),
            Key::Tab => self.commit_edit(ui, (0, step(m.shift))),
            Key::Up => self.commit_edit(ui, (-1, 0)),
            Key::Down => self.commit_edit(ui, (1, 0)),
            Key::Left => self.commit_edit(ui, (0, -1)),
            Key::Right => self.commit_edit(ui, (0, 1)),
            _ => {}
        }
    }

    /// Ctrl+S: saves to the workbook's own path, or `Book1.xlsx` in the working directory.
    pub fn save(&mut self) {
        let path = self
            .sheet
            .path()
            .unwrap_or_else(|| std::path::PathBuf::from("Book1.xlsx"));
        self.note = match self.sheet.save(&path) {
            Ok(()) => format!("saved {}", path.display()),
            Err(e) => format!("save failed: {e}"),
        };
    }

    /// Centre of cell `(row, col)` in device pixels (visible cells only), for tests and drivers.
    pub fn cell_centre(&self, row: u32, col: u32) -> Point {
        let (l, t, r, b) = self.grid_geom().cell(self.sheet.top_left, row, col);
        Point::new((l + r) / 2.0, (t + b) / 2.0)
    }
}
