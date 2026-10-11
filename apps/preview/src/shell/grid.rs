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
use crate::sheet::{self, Geo, Gesture, Paste, SheetModel, Zone};
use open365_ui_core::ScrollState;

/// The in-cell editor while a cell is being edited.
#[derive(Debug, Clone)]
pub(super) struct Editor {
    /// Enter mode (typed to replace): arrows commit and move. Edit mode (F2, double click): they
    /// move the caret.
    pub enter_mode: bool,
    pub text: String,
    pub node: Option<NodeId>,
}

/// Labels of the scroll bars; `grid_event` tells them apart by label.
const V_BAR: &str = "Vertical scroll bar";
const H_BAR: &str = "Horizontal scroll bar";
/// Scroll bar thickness in logical pixels.
const SCROLL_BAR_W: f32 = 14.0;

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
    pub fn set_sheet(&mut self, ui: &mut Ui, mut model: SheetModel) {
        model.set_locale(self.sheet.locale());
        self.sheet = model;
        self.editor = None;
        self.sync_bars(ui);
        self.build_grid(ui);
        self.sync_history(ui);
    }

    /// Sets the locale of dates, times and percentages in the formula bar and the editor.
    pub fn set_locale(&mut self, ui: &mut Ui, locale: std::sync::Arc<open365_calc::FormatLocale>) {
        self.sheet.set_locale(locale);
        self.sync_bars(ui);
    }

    /// The grid geometry in device pixels, with the sheet's own column widths and row heights.
    pub(super) fn grid_geom(&self) -> Geo {
        let u = |v: i32| (v as f32 * self.scale / crate::spec::REF_SCALE).round();
        let hdr = (
            u(self.spec.row_header_w.unwrap_or(34)),
            u(self.spec.col_header_h.unwrap_or(24)),
        );
        let a = self.content;
        Geo::new(
            (a.x as f32, a.y as f32, a.right() as f32, a.bottom() as f32),
            hdr,
            self.scale,
            self.sheet.geometry(),
            self.sheet.top_left,
        )
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
        let (l, t, r, b) = g.cell(self.sheet.active.0, self.sheet.active.1);
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
        self.add_scroll_bars(ui, root, rows, cols);
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

    /// The two scroll bars along the right and bottom edges of the grid body.
    fn add_scroll_bars(&mut self, ui: &mut Ui, root: NodeId, rows: u32, cols: u32) {
        let (total_rows, total_cols) = self.sheet.scroll_extent(rows, cols);
        let t = ((SCROLL_BAR_W * self.scale).round() as i32).max(8);
        let hdr_h = self.grid_geom().col_header_h as i32;
        let hdr_w = self.grid_geom().row_header_w as i32;
        let a = self.content;
        let (top, left) = (self.sheet.top_left.0, self.sheet.top_left.1);
        // The bars span the scrolling rows and columns only: the frozen ones never move.
        let (fr, fc) = self.sheet.frozen();
        let mut vertical = ScrollState::new(
            true,
            total_rows.saturating_sub(rows + fr) as f32,
            rows as f32,
        );
        vertical.line = 1.0;
        vertical.value = (top - 1 - fr) as f32;
        let mut horizontal = ScrollState::new(
            false,
            total_cols.saturating_sub(cols + fc) as f32,
            cols as f32,
        );
        horizontal.line = 1.0;
        horizontal.value = (left - 1 - fc) as f32;
        let bars = [
            (
                Widget::ScrollBar(vertical),
                V_BAR,
                PxRect {
                    x: a.right() - t,
                    y: a.y + hdr_h,
                    w: t,
                    h: (a.h - hdr_h - t).max(1),
                },
            ),
            (
                Widget::ScrollBar(horizontal),
                H_BAR,
                PxRect {
                    x: a.x + hdr_w,
                    y: a.bottom() - t,
                    w: (a.w - hdr_w - t).max(1),
                    h: t,
                },
            ),
        ];
        for (widget, label, rect) in bars {
            let id = self.add(
                ui,
                root,
                (0, 0),
                rect,
                widget,
                label,
                Look::default(),
                Act::Inert,
            );
            self.grid_ids.push(id);
        }
    }

    /// Takes the formula bar and the Name Box from the active cell (a range shows as `A1:C3`).
    pub(super) fn sync_bars(&mut self, ui: &mut Ui) {
        let (r, c) = self.sheet.active;
        self.name_box = if self.sheet.is_range() {
            let (r1, c1, r2, c2) = self.sheet.range();
            format!("{}:{}", sheet::a1(r1, c1), sheet::a1(r2, c2))
        } else {
            sheet::a1(r, c)
        };
        self.formula = self.sheet.bar_text(r, c);
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
            self.sheet.bar_text(r, c)
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
        self.sheet.select_cell(cell);
        self.sync_bars(ui);
        self.build_grid(ui);
        ui.set_focus(self.canvas);
    }

    /// Handles the events of the grid, the editor, the formula bar and the Name Box. Returns true
    /// when the event was one of them.
    pub(super) fn grid_event(&mut self, ui: &mut Ui, ev: &Emit) -> bool {
        let editor_node = self.editor.as_ref().and_then(|e| e.node);
        match ev {
            Emit::CanvasPointer {
                at, clicks, shift, ..
            } => {
                if self.editor.is_some() {
                    self.commit_edit(ui, (0, 0));
                }
                let x = self.content.x as f32 + at.x * self.scale;
                let y = self.content.y as f32 + at.y * self.scale;
                self.grid_press(ui, x, y, *clicks, *shift);
                true
            }
            Emit::CanvasDrag { node, at } if Some(*node) == self.canvas => {
                let x = self.content.x as f32 + at.x * self.scale;
                let y = self.content.y as f32 + at.y * self.scale;
                self.grid_drag(ui, x, y);
                true
            }
            Emit::CanvasRelease { node, .. } if Some(*node) == self.canvas => {
                self.grid_release(ui);
                true
            }
            Emit::CanvasWheel {
                node,
                delta_y,
                shift,
                ..
            } if Some(*node) == self.canvas => {
                self.grid_wheel(ui, *delta_y, *shift);
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
            Emit::ScrollChanged(n, v) if self.scroll_bar(ui, *n).is_some() => {
                let vertical = self.scroll_bar(ui, *n) == Some(true);
                let (fr, fc) = self.sheet.frozen();
                let at = v.round().max(0.0) as u32 + 1 + if vertical { fr } else { fc };
                self.grid_scroll_to(ui, vertical.then_some(at), (!vertical).then_some(at));
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

    /// Arrow keys: plain moves, Shift extends the range, Ctrl jumps to the edge of the data.
    fn arrow(&mut self, ui: &mut Ui, d: (i64, i64), m: Modifiers) {
        match (m.ctrl, m.shift) {
            (false, false) => self.move_active(ui, d),
            (false, true) => {
                self.sheet.extend_by(d.0, d.1);
                self.after_extend(ui);
            }
            (true, shift) => {
                self.sheet.jump_selection(d.0, d.1, shift);
                self.after_extend(ui);
            }
        }
    }

    /// The selection changed by key: scrolls its moving corner into view and updates the bars.
    fn after_extend(&mut self, ui: &mut Ui) {
        let (rows, cols) = self.grid_geom().visible();
        let corner = if self.sheet.is_range() {
            self.sheet.focus
        } else {
            self.sheet.active
        };
        self.sheet.reveal(corner, rows, cols);
        self.sync_bars(ui);
        self.build_grid(ui);
        ui.set_focus(self.canvas);
    }

    /// The selection changed without scrolling: refresh the bars.
    fn selection_changed(&mut self, ui: &mut Ui) {
        self.sync_bars(ui);
        ui.set_focus(self.canvas);
    }

    fn canvas_key(&mut self, ui: &mut Ui, key: Key, m: Modifiers) {
        if self.sheet.has_pending_paste() {
            // The paste would overwrite data: Enter or Y replaces, Esc or N keeps the cells.
            match key {
                Key::Enter | Key::Char('Y') => {
                    self.note = match self.sheet.confirm_paste() {
                        Ok(()) => "pasted".to_owned(),
                        Err(e) => format!("paste failed: {e}"),
                    };
                    self.sync_bars(ui);
                }
                Key::Escape | Key::Char('N') => {
                    self.sheet.cancel_paste();
                    self.note = "paste cancelled".to_owned();
                }
                _ => {}
            }
            return;
        }
        match key {
            Key::Up => self.arrow(ui, (-1, 0), m),
            Key::Down => self.arrow(ui, (1, 0), m),
            Key::Left => self.arrow(ui, (0, -1), m),
            Key::Right => self.arrow(ui, (0, 1), m),
            Key::Char('A') if m.ctrl => {
                self.sheet.select_all();
                self.selection_changed(ui);
            }
            Key::Char('C') if m.ctrl => self.copy_cells(false),
            Key::Char('X') if m.ctrl => self.copy_cells(true),
            Key::Char('V') if m.ctrl => self.paste_cells(ui),
            Key::Char(c @ ('D' | 'R')) if m.ctrl => {
                self.note = match self.sheet.fill_copy(c == 'D') {
                    Ok(true) => if c == 'D' {
                        "filled down"
                    } else {
                        "filled right"
                    }
                    .to_owned(),
                    Ok(false) => "nothing to fill".to_owned(),
                    Err(e) => format!("fill failed: {e}"),
                };
                self.sync_bars(ui);
                self.build_grid(ui);
            }
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
                // Ctrl+Home lands below and right of the frozen panes, as in Excel.
                let (r, c) = self.sheet.active;
                let (fr, fc) = self.sheet.frozen();
                if m.ctrl {
                    self.move_active(
                        ui,
                        (
                            i64::from(fr) + 1 - i64::from(r),
                            i64::from(fc) + 1 - i64::from(c),
                        ),
                    );
                } else {
                    self.move_active(ui, (0, 1 - i64::from(c)));
                }
            }
            Key::F2 => self.begin_edit(ui, false, false),
            Key::F12 => self.save_as(),
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
            Key::Char('Z') if m.ctrl => self.history(ui, true),
            Key::Char('Y') if m.ctrl => self.history(ui, false),
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

    /// Ctrl+S: saves to the workbook's own path; an unnamed workbook opens Save As.
    pub fn save(&mut self) {
        let Some(path) = self.sheet.path() else {
            self.save_as();
            return;
        };
        self.note = match self.sheet.save(&path) {
            Ok(()) => format!("saved {}", path.display()),
            Err(e) => format!("save failed: {e}"),
        };
    }

    /// F12: asks where to save, then saves there.
    pub fn save_as(&mut self) {
        self.note = match self.sheet.save_as() {
            Ok(Some(path)) => format!("saved {}", path.display()),
            Ok(None) => "save cancelled".to_owned(),
            Err(e) => format!("save failed: {e}"),
        };
    }

    fn copy_cells(&mut self, cut: bool) {
        self.note = match self.sheet.copy(cut) {
            Ok(()) if cut => "cut".to_owned(),
            Ok(()) => "copied".to_owned(),
            Err(e) => format!("copy failed: {e}"),
        };
    }

    fn paste_cells(&mut self, ui: &mut Ui) {
        self.note = match self.sheet.paste() {
            Ok(Paste::Done) => "pasted".to_owned(),
            Ok(Paste::NeedsConfirm) => {
                "Paste over existing data? Enter replaces it, Esc keeps it".to_owned()
            }
            Ok(Paste::Empty) => "nothing to paste".to_owned(),
            Err(e) => format!("paste failed: {e}"),
        };
        self.sync_bars(ui);
        self.build_grid(ui);
    }

    /// True while a paste waits for the person to confirm overwriting data.
    pub fn paste_pending(&self) -> bool {
        self.sheet.has_pending_paste()
    }

    /// Centre of cell `(row, col)` in device pixels (visible cells only), for tests and drivers.
    pub fn cell_centre(&self, row: u32, col: u32) -> Point {
        let (l, t, r, b) = self.grid_geom().cell(row, col);
        Point::new((l + r) / 2.0, (t + b) / 2.0)
    }

    /// Centre of column header `col` in device pixels.
    pub fn col_header_centre(&self, col: u32) -> Point {
        let g = self.grid_geom();
        let (l, _, r, _) = g.cell(self.sheet.top_left.0, col);
        Point::new((l + r) / 2.0, g.y0 + g.col_header_h / 2.0)
    }

    /// Centre of row header `row` in device pixels.
    pub fn row_header_centre(&self, row: u32) -> Point {
        let g = self.grid_geom();
        let (_, t, _, b) = g.cell(row, self.sheet.top_left.1);
        Point::new(g.x0 + g.row_header_w / 2.0, (t + b) / 2.0)
    }

    /// The point on the right edge of column `col`'s header (where a drag resizes it).
    pub fn col_edge_point(&self, col: u32) -> Point {
        let g = self.grid_geom();
        let (_, _, r, _) = g.cell(self.sheet.top_left.0, col);
        Point::new(r - 1.0, g.y0 + g.col_header_h / 2.0)
    }

    /// The point on the bottom edge of row `row`'s header.
    pub fn row_edge_point(&self, row: u32) -> Point {
        let g = self.grid_geom();
        let (_, _, _, b) = g.cell(row, self.sheet.top_left.1);
        Point::new(g.x0 + g.row_header_w / 2.0, b - 1.0)
    }

    /// The fill handle of the selection in device pixels.
    pub fn fill_handle_point(&self) -> Point {
        let g = self.grid_geom();
        let (_, _, r, b) = g.range_rect(self.sheet.range());
        Point::new(r - 1.0, b - 1.0)
    }

    /// Whether this node is one of the grid's scroll bars: `Some(true)` for the vertical one.
    fn scroll_bar(&self, ui: &Ui, id: NodeId) -> Option<bool> {
        match ui.tree.get(id).map(|n| n.label.as_str()) {
            Some(V_BAR) => Some(true),
            Some(H_BAR) => Some(false),
            _ => None,
        }
    }

    /// Scroll bar change: the first visible row or column.
    pub fn grid_scroll_to(&mut self, ui: &mut Ui, row: Option<u32>, col: Option<u32>) {
        self.sheet.scroll_to(row, col);
        if self.editor.is_some() {
            self.build_grid(ui);
        }
    }

    /// Mouse wheel over the grid: `notches` positive scrolls up (`sideways` scrolls across).
    pub fn grid_wheel(&mut self, ui: &mut Ui, notches: f32, sideways: bool) {
        self.sheet.scroll_wheel(notches, sideways);
        if self.editor.is_some() {
            self.build_grid(ui);
        }
        self.refresh_scroll_bars(ui);
    }

    /// Moves the scroll bar thumbs to the sheet's scroll position.
    fn refresh_scroll_bars(&mut self, ui: &mut Ui) {
        let (top, left) = self.sheet.top_left;
        let (fr, fc) = self.sheet.frozen();
        for id in self.grid_ids.clone() {
            let which = self.scroll_bar(ui, id);
            if let Some(n) = ui.tree.get_mut(id)
                && let Widget::ScrollBar(s) = &mut n.widget
            {
                s.value = match which {
                    Some(true) => (top - 1 - fr) as f32,
                    _ => (left - 1 - fc) as f32,
                };
            }
        }
    }

    /// A primary press on the grid at device point `(x, y)`.
    pub fn grid_press(&mut self, ui: &mut Ui, x: f32, y: f32, clicks: u8, shift: bool) {
        let geo = self.grid_geom();
        let zone = geo.zone(x, y, Some(self.sheet.range()));
        self.sheet.gesture = None;
        match zone {
            Zone::Corner => {
                self.sheet.select_all();
                self.selection_changed(ui);
            }
            Zone::ColHeader(c) => {
                let from = if shift { self.sheet.anchor.1 } else { c };
                self.sheet.select_cols(from, c, shift);
                self.sheet.gesture = Some(Gesture::Cols { anchor: from });
                self.selection_changed(ui);
            }
            Zone::RowHeader(r) => {
                let from = if shift { self.sheet.anchor.0 } else { r };
                self.sheet.select_rows(from, r, shift);
                self.sheet.gesture = Some(Gesture::Rows { anchor: from });
                self.selection_changed(ui);
            }
            Zone::ColEdge(c) if clicks >= 2 => {
                let px = 12.0 * self.scale;
                let scale = self.scale;
                let measure = |t: &str| {
                    ui.assets
                        .layout_text(ui.font, t, px, 0.0, 0.0, Default::default())
                        .width
                        / scale
                };
                let _ = self.sheet.autofit_col(c, &measure);
                self.build_grid(ui);
            }
            Zone::ColEdge(c) => {
                let px = self.sheet.geometry().col_px(c);
                self.sheet.gesture = Some(Gesture::ResizeCol {
                    col: c,
                    start_x: x,
                    start_px: px,
                    px,
                });
            }
            Zone::RowEdge(r) if clicks >= 2 => {
                let _ = self.sheet.autofit_row(r);
                self.build_grid(ui);
            }
            Zone::RowEdge(r) => {
                let px = self.sheet.geometry().row_px(r);
                self.sheet.gesture = Some(Gesture::ResizeRow {
                    row: r,
                    start_y: y,
                    start_px: px,
                    px,
                });
            }
            Zone::FillHandle => {
                let (_, _, r2, c2) = self.sheet.range();
                self.sheet.gesture = Some(Gesture::Fill { to: (r2, c2) });
            }
            Zone::Cell(r, c) => {
                if shift {
                    self.sheet.extend_to((r, c));
                    self.sheet.gesture = Some(Gesture::Select);
                    self.selection_changed(ui);
                } else {
                    self.select(ui, (r, c));
                    if clicks >= 2 {
                        self.begin_edit(ui, false, false);
                    } else {
                        self.sheet.gesture = Some(Gesture::Select);
                    }
                }
            }
            Zone::Outside => {}
        }
    }

    /// The pointer moved to device point `(x, y)` with the primary button held.
    pub fn grid_drag(&mut self, ui: &mut Ui, x: f32, y: f32) {
        let Some(gesture) = self.sheet.gesture else {
            return;
        };
        let geo = self.grid_geom();
        let scale = self.scale;
        match gesture {
            Gesture::Select => {
                // A drag beyond the edge scrolls one step per move.
                let lo_x = geo.x0 + geo.row_header_w;
                let lo_y = geo.y0 + geo.col_header_h;
                let dr = i64::from(y >= geo.y1) - i64::from(y < lo_y);
                let dc = i64::from(x >= geo.x1) - i64::from(x < lo_x);
                let mut geo = geo;
                if dr != 0 || dc != 0 {
                    let (t, l) = self.sheet.top_left;
                    self.sheet.scroll_to(
                        Some((i64::from(t) + dr).max(1) as u32),
                        Some((i64::from(l) + dc).max(1) as u32),
                    );
                    self.refresh_scroll_bars(ui);
                    geo = self.grid_geom();
                }
                self.sheet.extend_to(geo.hit_clamped(x, y));
                self.sync_bars(ui);
            }
            Gesture::Cols { anchor } => {
                let (_, c) = geo.hit_clamped(x, y);
                self.sheet.select_cols(anchor, c, true);
                self.sync_bars(ui);
            }
            Gesture::Rows { anchor } => {
                let (r, _) = geo.hit_clamped(x, y);
                self.sheet.select_rows(anchor, r, true);
                self.sync_bars(ui);
            }
            Gesture::ResizeCol {
                col,
                start_x,
                start_px,
                ..
            } => {
                let px = (start_px as f32 + (x - start_x) / scale).round().max(0.0) as u32;
                self.sheet.gesture = Some(Gesture::ResizeCol {
                    col,
                    start_x,
                    start_px,
                    px,
                });
            }
            Gesture::ResizeRow {
                row,
                start_y,
                start_px,
                ..
            } => {
                let px = (start_px as f32 + (y - start_y) / scale).round().max(0.0) as u32;
                self.sheet.gesture = Some(Gesture::ResizeRow {
                    row,
                    start_y,
                    start_px,
                    px,
                });
            }
            Gesture::Fill { .. } => {
                let to = geo.hit_clamped(x, y);
                self.sheet.gesture = Some(Gesture::Fill { to });
            }
        }
    }

    /// The primary button went up: a resize or fill takes effect now, as one undo step.
    pub fn grid_release(&mut self, ui: &mut Ui) {
        let Some(gesture) = self.sheet.gesture.take() else {
            return;
        };
        match gesture {
            Gesture::ResizeCol {
                col, start_px, px, ..
            } if px != start_px => {
                let _ = self.sheet.set_col_px(col, px);
            }
            Gesture::ResizeRow {
                row, start_px, px, ..
            } if px != start_px => {
                let _ = self.sheet.set_row_px(row, px);
            }
            Gesture::Fill { to } => {
                let _ = self.sheet.fill_to(to);
            }
            _ => {}
        }
        self.sync_bars(ui);
        self.build_grid(ui);
    }
}
