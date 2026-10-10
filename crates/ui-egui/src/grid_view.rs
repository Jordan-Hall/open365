//! Virtualised spreadsheet grid over an XLSX [`Target`].
//!
//! Only the rows and columns in the viewport are drawn, found by binary search over the sparse
//! row list, so a 50,000-row sheet costs the same per frame as a small one. Cell edits go
//! through the formula bar and the command layer: text starting with `=` becomes
//! `XlsxSetCellFormula`, anything else `XlsxSetCellValue`.

use std::time::Instant;

use egui::text::CCursor;
use egui::text_edit::TextEditState;
use egui::{
    Align2, Color32, Context, Event, EventFilter, FontId, Id, Key, Pos2, Rect, Sense, Stroke,
    TextEdit, Ui, UiBuilder, accesskit, vec2,
};
use open365_commands::{Command, Target};
use open365_xlsx::{CellValue, MAX_COL, MAX_ROW, SheetKind, SheetState, Workbook, col_letters};

use crate::a11y;
use crate::numfmt;

const ROW_HEADER_W: f32 = 54.0;
const COL_HEADER_H: f32 = 24.0;
const COL_W: f32 = 88.0;
const ROW_H: f32 = 22.0;
const MIN_ROWS: u32 = 200;
const MIN_COLS: u32 = 26;

/// State of the grid view.
#[derive(Debug)]
pub struct GridView {
    sheet: usize,
    active: (u32, u32),
    editing: bool,
    edit_text: String,
    scroll_to_active: bool,
    want_focus: bool,
    grid_id: Id,
    formula_id: Id,
    /// Cell the screen reader asked to focus, applied next frame.
    pending_focus: Option<(u32, u32)>,
    /// Widget id that holds keyboard focus and is the node id of the active cell.
    focus_id: Id,
    /// Set when this frame's key events were already used by the formula bar.
    keys_consumed: bool,
    /// Message for the status bar, taken by the application each frame.
    pub status: Option<String>,
    /// Milliseconds spent drawing the grid in the last frame.
    pub last_frame_ms: f32,
}

impl Default for GridView {
    fn default() -> Self {
        Self::new()
    }
}

impl GridView {
    pub fn new() -> Self {
        Self {
            sheet: 0,
            active: (1, 1),
            editing: false,
            edit_text: String::new(),
            scroll_to_active: false,
            want_focus: true,
            grid_id: Id::new("open365-grid-view"),
            formula_id: Id::new("open365-formula-bar"),
            pending_focus: None,
            focus_id: Id::new("open365-grid-active-cell"),
            keys_consumed: false,
            status: None,
            last_frame_ms: 0.0,
        }
    }

    pub fn id(&self) -> Id {
        self.grid_id
    }

    pub fn sheet(&self) -> usize {
        self.sheet
    }

    pub fn active(&self) -> (u32, u32) {
        self.active
    }

    pub fn is_editing(&self) -> bool {
        self.editing
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn set_active(&mut self, row: u32, col: u32) {
        self.active = (row.clamp(1, MAX_ROW), col.clamp(1, MAX_COL));
        self.scroll_to_active = true;
    }

    pub fn select_sheet(&mut self, sheet: usize) {
        if sheet != self.sheet {
            self.sheet = sheet;
            self.active = (1, 1);
            self.editing = false;
            self.scroll_to_active = true;
        }
    }

    /// The active cell always uses one stable widget id (egui drops focus from ids that did
    /// not exist last frame); other cells get ids derived from their address.
    fn cell_id(&self, row: u32, col: u32) -> Id {
        if (row, col) == self.active {
            return self.focus_id;
        }
        self.grid_id.with(("cell", row, col))
    }

    /// What the formula bar shows for a cell: `=formula`, or the stored value in full precision.
    pub fn source_text(wb: &Workbook, sheet: usize, row: u32, col: u32) -> String {
        if let Some(f) = wb.formula_text(sheet, row, col) {
            return format!("={f}");
        }
        match wb.cell(sheet, row, col).map(|c| &c.value) {
            None | Some(CellValue::Empty) => String::new(),
            Some(v) => numfmt::display_value(v, "General", wb.date1904()),
        }
    }

    pub fn start_edit(&mut self, ctx: &Context, initial: String) {
        self.editing = true;
        self.edit_text = initial;
        ctx.memory_mut(|m| m.request_focus(self.formula_id));
        let mut state = TextEditState::load(ctx, self.formula_id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(CCursor::new(
                self.edit_text.chars().count(),
            ))));
        state.store(ctx, self.formula_id);
    }

    fn cancel_edit(&mut self, ctx: &Context) {
        self.editing = false;
        self.keys_consumed = true;
        let id = self.cell_id(self.active.0, self.active.1);
        ctx.memory_mut(|m| m.request_focus(id));
    }

    /// Commits the formula bar text to the active cell through the command layer.
    pub fn commit_edit(&mut self, ctx: &Context, target: &mut Target, advance: bool) {
        let (row, col) = self.active;
        let unchanged = target
            .xlsx()
            .is_some_and(|wb| Self::source_text(wb, self.sheet, row, col) == self.edit_text);
        if !unchanged {
            let command = Command::xlsx_enter(self.sheet, row, col, &self.edit_text);
            if let Err(e) = target.apply(command) {
                self.status = Some(e.to_string());
                return;
            }
        }
        self.editing = false;
        self.keys_consumed = true;
        if advance {
            self.move_active(1, 0);
        }
        let id = self.cell_id(self.active.0, self.active.1);
        ctx.memory_mut(|m| m.request_focus(id));
    }

    fn move_active(&mut self, d_row: i64, d_col: i64) {
        let row = (i64::from(self.active.0) + d_row).clamp(1, i64::from(MAX_ROW)) as u32;
        let col = (i64::from(self.active.1) + d_col).clamp(1, i64::from(MAX_COL)) as u32;
        self.set_active(row, col);
    }

    fn sheet_extent(wb: &Workbook, sheet: usize) -> (u32, u32) {
        let (r, c) = wb
            .sheet(sheet)
            .and_then(|ws| ws.used_range())
            .map_or((0, 0), |(_, _, r2, c2)| (r2, c2));
        (
            (r + 50).clamp(MIN_ROWS, MAX_ROW),
            (c + 6).clamp(MIN_COLS, MAX_COL),
        )
    }

    /// Shows the formula bar, the grid and the sheet tabs.
    pub fn show(&mut self, ui: &mut Ui, target: &mut Target) {
        let started = Instant::now();
        let Some(wb) = target.xlsx() else {
            ui.label("No spreadsheet is open.");
            return;
        };
        if self.sheet >= wb.sheets().len() {
            self.sheet = 0;
        }
        if wb.sheet(self.sheet).is_none()
            && let Some(first) = wb
                .sheets()
                .iter()
                .position(|s| s.kind == SheetKind::Worksheet)
        {
            self.sheet = first;
        }
        if let Some((row, col)) = self.pending_focus.take() {
            self.set_active(row, col);
        }

        self.formula_bar(ui, target);
        let tabs_changed = self.sheet_tabs(ui, target);
        let _ = tabs_changed;

        egui::CentralPanel::default_margins()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.grid(ui, target));
        self.last_frame_ms = started.elapsed().as_secs_f32() * 1000.0;
    }

    fn formula_bar(&mut self, ui: &mut Ui, target: &mut Target) {
        let ctx = ui.ctx().clone();
        let (row, col) = self.active;
        if !self.editing
            && let Some(wb) = target.xlsx()
        {
            self.edit_text = Self::source_text(wb, self.sheet, row, col);
        }
        let address = format!("{}{}", col_letters(col), row);
        egui::Panel::top("open365-formula-panel").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [64.0, 22.0],
                    egui::Label::new(egui::RichText::new(&address).strong()),
                );
                ui.label("fx");
                let edit = TextEdit::singleline(&mut self.edit_text)
                    .id(self.formula_id)
                    .desired_width(f32::INFINITY)
                    .font(FontId::monospace(13.0))
                    .hint_text("Value or =formula");
                let resp = ui.add(edit);
                resp.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::TextEdit,
                        true,
                        format!("Formula bar for {address}"),
                    )
                });
                if resp.changed() || (resp.gained_focus() && !self.editing) {
                    self.editing = true;
                }
                let (enter, tab, escape) = ctx.input(|i| {
                    (
                        i.key_pressed(Key::Enter),
                        i.key_pressed(Key::Tab),
                        i.key_pressed(Key::Escape),
                    )
                });
                if self.editing && escape && (resp.has_focus() || resp.lost_focus()) {
                    self.cancel_edit(&ctx);
                } else if self.editing && resp.lost_focus() && (enter || tab) {
                    self.commit_edit(&ctx, target, enter);
                }
            });
        });
    }

    fn sheet_tabs(&mut self, ui: &mut Ui, target: &Target) -> bool {
        let mut switched = false;
        let Some(wb) = target.xlsx() else {
            return false;
        };
        egui::Panel::bottom("open365-sheet-tabs").show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, info) in wb.sheets().iter().enumerate() {
                    if info.kind != SheetKind::Worksheet || info.state != SheetState::Visible {
                        continue;
                    }
                    if ui.selectable_label(i == self.sheet, &info.name).clicked() {
                        self.select_sheet(i);
                        switched = true;
                    }
                }
            });
        });
        switched
    }

    fn grid(&mut self, ui: &mut Ui, target: &mut Target) {
        let ctx = ui.ctx().clone();
        let sheet = self.sheet;
        let Some(wb) = target.xlsx() else { return };
        let (rows_n, cols_n) = Self::sheet_extent(wb, sheet);
        let content = vec2(
            ROW_HEADER_W + cols_n as f32 * COL_W,
            COL_HEADER_H + rows_n as f32 * ROW_H,
        );

        let active_id = self.focus_id;
        let formula_focused = ctx.memory(|m| m.has_focus(self.formula_id));
        if self.want_focus && !formula_focused {
            self.want_focus = false;
            ctx.memory_mut(|m| m.request_focus(active_id));
        }
        let grid_focused = ctx.memory(|m| m.has_focus(active_id));
        ctx.memory_mut(|m| {
            m.set_focus_lock_filter(
                active_id,
                EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: false,
                },
            );
        });

        if grid_focused
            && !self.editing
            && !std::mem::take(&mut self.keys_consumed)
            && ctx.current_pass_index() == 0
        {
            self.handle_keys(&ctx, target, rows_n, cols_n);
        }
        let Some(wb) = target.xlsx() else { return };

        let text_color = ui.visuals().text_color();
        let grid_line = ui.visuals().widgets.noninteractive.bg_stroke.color;
        let header_bg = ui.visuals().faint_bg_color;
        let cell_bg = ui.visuals().extreme_bg_color;
        let accent = ui.visuals().selection.stroke.color;
        let date1904 = wb.date1904();
        let font = FontId::proportional(13.0);
        let active = self.active;
        let scroll_to_active = std::mem::take(&mut self.scroll_to_active);

        let mut clicked_cell: Option<(u32, u32)> = None;
        let mut visible_rows = (1u32, 0u32);
        let mut visible_cols = (1u32, 0u32);
        let mut canvas = Rect::NOTHING;

        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show_viewport(ui, |ui, viewport| {
                let (rect, _) = ui.allocate_exact_size(content, Sense::hover());
                canvas = rect;
                let response = ui.interact(rect, self.grid_id, Sense::click_and_drag());
                let origin = rect.min;
                let painter = ui.painter_at(ui.clip_rect());

                let first_row = (((viewport.min.y - COL_HEADER_H) / ROW_H).floor().max(0.0) as u32
                    + 1)
                .min(rows_n);
                let last_row = ((((viewport.max.y - COL_HEADER_H) / ROW_H).ceil().max(0.0)) as u32
                    + 1)
                .min(rows_n);
                let first_col = (((viewport.min.x - ROW_HEADER_W) / COL_W).floor().max(0.0) as u32
                    + 1)
                .min(cols_n);
                let last_col = ((((viewport.max.x - ROW_HEADER_W) / COL_W).ceil().max(0.0)) as u32
                    + 1)
                .min(cols_n);
                visible_rows = (first_row, last_row);
                visible_cols = (first_col, last_col);

                let cell_rect = |r: u32, c: u32| {
                    Rect::from_min_size(
                        Pos2::new(
                            origin.x + ROW_HEADER_W + (c - 1) as f32 * COL_W,
                            origin.y + COL_HEADER_H + (r - 1) as f32 * ROW_H,
                        ),
                        vec2(COL_W, ROW_H),
                    )
                };

                // Cell area background and grid lines.
                let area = Rect::from_min_max(
                    cell_rect(first_row, first_col).min,
                    cell_rect(last_row, last_col).max,
                );
                painter.rect_filled(area, 0.0, cell_bg);
                let line = Stroke::new(1.0, grid_line);
                for r in first_row..=last_row + 1 {
                    let y = origin.y + COL_HEADER_H + (r - 1) as f32 * ROW_H;
                    painter.hline(area.x_range(), y, line);
                }
                for c in first_col..=last_col + 1 {
                    let x = origin.x + ROW_HEADER_W + (c - 1) as f32 * COL_W;
                    painter.vline(x, area.y_range(), line);
                }

                // Cell text, found by binary search over the sparse rows.
                if let Some(ws) = wb.sheet(sheet) {
                    let rows = ws.rows();
                    let start = rows.partition_point(|r| r.r < first_row);
                    for row in rows[start..].iter().take_while(|r| r.r <= last_row) {
                        let cstart = row.cells.partition_point(|c| c.col < first_col);
                        for cell in row.cells[cstart..].iter().take_while(|c| c.col <= last_col) {
                            let text = numfmt::display_value(
                                &cell.value,
                                wb.styles().number_format(cell.style.unwrap_or(0)),
                                date1904,
                            );
                            if text.is_empty() {
                                continue;
                            }
                            let r = cell_rect(row.r, cell.col);
                            let clipped = painter.with_clip_rect(r.shrink2(vec2(1.0, 0.0)));
                            let (anchor, pos, color) = match &cell.value {
                                CellValue::Number(_) => (
                                    Align2::RIGHT_CENTER,
                                    Pos2::new(r.right() - 4.0, r.center().y),
                                    text_color,
                                ),
                                CellValue::Bool(_) => {
                                    (Align2::CENTER_CENTER, r.center(), text_color)
                                }
                                CellValue::Error(_) => (
                                    Align2::CENTER_CENTER,
                                    r.center(),
                                    Color32::from_rgb(200, 60, 60),
                                ),
                                _ => (
                                    Align2::LEFT_CENTER,
                                    Pos2::new(r.left() + 4.0, r.center().y),
                                    text_color,
                                ),
                            };
                            clipped.text(pos, anchor, text, font.clone(), color);
                        }
                    }
                }

                // Active cell outline.
                if (first_row..=last_row).contains(&active.0)
                    && (first_col..=last_col).contains(&active.1)
                {
                    painter.rect_stroke(
                        cell_rect(active.0, active.1),
                        0.0,
                        Stroke::new(2.0, accent),
                        egui::StrokeKind::Inside,
                    );
                }

                // Sticky headers, drawn over the cells at the viewport edge.
                let view_min = origin + viewport.min.to_vec2();
                let header_stroke = Stroke::new(1.0, grid_line);
                let col_header = Rect::from_min_size(
                    Pos2::new(origin.x + ROW_HEADER_W + 0.0, view_min.y),
                    vec2(content.x - ROW_HEADER_W, COL_HEADER_H),
                );
                painter.rect_filled(
                    Rect::from_min_size(view_min, vec2(viewport.width(), COL_HEADER_H)),
                    0.0,
                    header_bg,
                );
                painter.rect_filled(
                    Rect::from_min_size(view_min, vec2(ROW_HEADER_W, viewport.height())),
                    0.0,
                    header_bg,
                );
                for c in first_col..=last_col {
                    let r = Rect::from_min_size(
                        Pos2::new(origin.x + ROW_HEADER_W + (c - 1) as f32 * COL_W, view_min.y),
                        vec2(COL_W, COL_HEADER_H),
                    );
                    if r.intersects(col_header) {
                        painter.vline(r.left(), r.y_range(), header_stroke);
                        painter.text(
                            r.center(),
                            Align2::CENTER_CENTER,
                            col_letters(c),
                            font.clone(),
                            if c == active.1 { accent } else { text_color },
                        );
                    }
                }
                for r in first_row..=last_row {
                    let rr = Rect::from_min_size(
                        Pos2::new(view_min.x, origin.y + COL_HEADER_H + (r - 1) as f32 * ROW_H),
                        vec2(ROW_HEADER_W, ROW_H),
                    );
                    painter.hline(rr.x_range(), rr.top(), header_stroke);
                    painter.text(
                        rr.center(),
                        Align2::CENTER_CENTER,
                        r.to_string(),
                        font.clone(),
                        if r == active.0 { accent } else { text_color },
                    );
                }
                painter.hline(
                    view_min.x..=view_min.x + viewport.width(),
                    view_min.y + COL_HEADER_H,
                    header_stroke,
                );
                painter.vline(
                    view_min.x + ROW_HEADER_W,
                    view_min.y..=view_min.y + viewport.height(),
                    header_stroke,
                );

                // Pointer: click selects a cell, double click edits it.
                if response.clicked() || response.double_clicked() {
                    if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
                        let local = pos - origin;
                        if local.x >= ROW_HEADER_W
                            && local.y >= COL_HEADER_H
                            && local.x >= viewport.min.x + ROW_HEADER_W
                            && local.y >= viewport.min.y + COL_HEADER_H
                        {
                            let c = ((local.x - ROW_HEADER_W) / COL_W).floor() as u32 + 1;
                            let r = ((local.y - COL_HEADER_H) / ROW_H).floor() as u32 + 1;
                            if (1..=rows_n).contains(&r) && (1..=cols_n).contains(&c) {
                                clicked_cell = Some((r, c));
                            }
                        }
                    }
                    response.request_focus();
                }
                if response.double_clicked() {
                    // Handled after the borrow of the workbook ends.
                    self.status = None;
                }

                if scroll_to_active {
                    ui.scroll_to_rect(cell_rect(active.0, active.1).expand(2.0), None);
                }

                // The active cell is always a focusable widget so focus and the screen
                // reader's cursor have a node to sit on.
                let ar = cell_rect(active.0, active.1);
                ui.interact(ar, active_id, Sense::focusable_noninteractive());
            });

        let double = ctx.input(|i| {
            i.pointer
                .button_double_clicked(egui::PointerButton::Primary)
        });
        if let Some((r, c)) = clicked_cell {
            if self.editing {
                self.commit_edit(&ctx, target, false);
            }
            self.active = (r, c);
            let id = self.cell_id(r, c);
            ctx.memory_mut(|m| m.request_focus(id));
            if double {
                let text = target
                    .xlsx()
                    .map(|wb| Self::source_text(wb, self.sheet, r, c))
                    .unwrap_or_default();
                self.start_edit(&ctx, text);
            }
        }

        self.publish_accessibility(
            ui,
            target,
            canvas,
            visible_rows,
            visible_cols,
            (rows_n, cols_n),
        );
    }

    fn handle_keys(&mut self, ctx: &Context, target: &mut Target, rows_n: u32, cols_n: u32) {
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            match event {
                Event::Text(t) => {
                    let first: String = t.chars().filter(|c| !c.is_control()).collect();
                    if !first.is_empty() {
                        self.start_edit(ctx, first);
                    }
                }
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    let page = 20;
                    match key {
                        Key::ArrowUp if modifiers.command => self.set_active(1, self.active.1),
                        Key::ArrowDown if modifiers.command => {
                            self.set_active(rows_n, self.active.1)
                        }
                        Key::ArrowLeft if modifiers.command => self.set_active(self.active.0, 1),
                        Key::ArrowRight if modifiers.command => {
                            self.set_active(self.active.0, cols_n);
                        }
                        Key::ArrowUp => self.move_active(-1, 0),
                        Key::ArrowDown => self.move_active(1, 0),
                        Key::ArrowLeft => self.move_active(0, -1),
                        Key::ArrowRight => self.move_active(0, 1),
                        Key::Enter => self.move_active(if modifiers.shift { -1 } else { 1 }, 0),
                        Key::Tab => self.move_active(0, if modifiers.shift { -1 } else { 1 }),
                        Key::PageUp => self.move_active(-page, 0),
                        Key::PageDown => self.move_active(page, 0),
                        Key::Home if modifiers.command => self.set_active(1, 1),
                        Key::Home => self.set_active(self.active.0, 1),
                        Key::F2 => {
                            let text = target
                                .xlsx()
                                .map(|wb| {
                                    Self::source_text(wb, self.sheet, self.active.0, self.active.1)
                                })
                                .unwrap_or_default();
                            self.start_edit(ctx, text);
                        }
                        Key::Delete | Key::Backspace => {
                            let (row, col) = self.active;
                            let command = Command::XlsxSetCellValue {
                                sheet: self.sheet,
                                row,
                                col,
                                value: open365_commands::CellEntry::Empty,
                            };
                            if let Err(e) = target.apply(command) {
                                self.status = Some(e.to_string());
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        // Keep focus on whichever cell is now active.
        let id = self.cell_id(self.active.0, self.active.1);
        if !self.editing {
            ctx.memory_mut(|m| m.request_focus(id));
        }
    }

    /// Publishes Grid, Row and Cell nodes for the visible range plus the active cell.
    fn publish_accessibility(
        &mut self,
        ui: &mut Ui,
        target: &Target,
        canvas: Rect,
        rows: (u32, u32),
        cols: (u32, u32),
        extent: (u32, u32),
    ) {
        let ctx = ui.ctx().clone();
        if !a11y::is_enabled(&ctx, self.grid_id) {
            return;
        }
        let Some(wb) = target.xlsx() else { return };
        let sheet_name = wb
            .sheets()
            .get(self.sheet)
            .map_or_else(String::new, |s| s.name.clone());
        let viewport = ui.clip_rect();
        let origin = canvas.min;
        let cell_rect = |r: u32, c: u32| {
            Rect::from_min_size(
                Pos2::new(
                    origin.x + ROW_HEADER_W + (c - 1) as f32 * COL_W,
                    origin.y + COL_HEADER_H + (r - 1) as f32 * ROW_H,
                ),
                vec2(COL_W, ROW_H),
            )
        };

        let mut row_list: Vec<u32> = (rows.0..=rows.1).collect();
        if !row_list.contains(&self.active.0) {
            row_list.push(self.active.0);
            row_list.sort_unstable();
        }
        let date1904 = wb.date1904();
        // A screen reader moves the cursor by asking to focus a cell node.
        let focus_actions = ctx.input(|i| {
            i.events
                .iter()
                .any(|e| matches!(e, Event::AccessKitActionRequest(_)))
        });

        let mut rows_ui = ui.new_child(
            UiBuilder::new()
                .id_salt("grid-a11y-rows")
                .max_rect(viewport)
                .accessibility_parent(self.grid_id),
        );
        let grid_id = self.grid_id;
        let mut focus_request = None;
        for r in row_list {
            let row_id = grid_id.with(("row", r));
            let row_bounds = Rect::from_min_max(cell_rect(r, 1).min, cell_rect(r, extent.1).max);
            rows_ui.interact(row_bounds, row_id, Sense::hover());
            rows_ui.ctx().accesskit_node_builder(row_id, |b| {
                b.set_role(accesskit::Role::Row);
                b.set_row_index((r - 1) as usize);
                b.set_bounds(a11y::to_ak_rect(row_bounds));
            });
            let cells_ui = rows_ui.new_child(
                UiBuilder::new()
                    .id_salt(("grid-a11y-cells", r))
                    .max_rect(row_bounds)
                    .accessibility_parent(row_id),
            );
            let mut col_list: Vec<u32> = (cols.0..=cols.1).collect();
            if r == self.active.0 && !col_list.contains(&self.active.1) {
                col_list.push(self.active.1);
                col_list.sort_unstable();
            }
            for c in col_list {
                let cell_id = self.cell_id(r, c);
                let rect = cell_rect(r, c);
                // Only the active cell is focusable, so egui's arrow-key focus navigation cannot
                // jump between cell widgets.
                let sense = if (r, c) == self.active {
                    Sense::focusable_noninteractive()
                } else {
                    Sense::hover()
                };
                cells_ui.interact(rect, cell_id, sense);
                if focus_actions
                    && (r, c) != self.active
                    && ctx.input(|i| {
                        i.has_accesskit_action_request(cell_id, accesskit::Action::Focus)
                    })
                {
                    focus_request = Some((r, c));
                }
                let (content, is_formula) = match wb.cell(self.sheet, r, c) {
                    Some(cell) => (
                        numfmt::display_value(
                            &cell.value,
                            wb.number_format(self.sheet, r, c),
                            date1904,
                        ),
                        cell.formula.is_some(),
                    ),
                    None => (String::new(), false),
                };
                let address = format!("{}{}", col_letters(c), r);
                let formula = is_formula
                    .then(|| wb.formula_text(self.sheet, r, c))
                    .flatten();
                cells_ui.ctx().accesskit_node_builder(cell_id, |b| {
                    b.set_role(accesskit::Role::Cell);
                    b.set_row_index((r - 1) as usize);
                    b.set_column_index((c - 1) as usize);
                    b.set_bounds(a11y::to_ak_rect(rect));
                    b.set_label(if content.is_empty() {
                        "blank".to_owned()
                    } else {
                        content.clone()
                    });
                    b.set_value(content);
                    b.set_description(match formula {
                        Some(f) => format!("{address}, formula ={f}"),
                        None => address,
                    });
                });
            }
        }
        if let Some(cell) = focus_request {
            self.pending_focus = Some(cell);
            ctx.request_repaint();
        }
        let label = format!("{sheet_name}, spreadsheet grid");
        ctx.accesskit_node_builder(grid_id, |b| {
            b.set_role(accesskit::Role::Grid);
            b.set_label(label);
            b.set_row_count(extent.0 as usize);
            b.set_column_count(extent.1 as usize);
            b.set_bounds(a11y::to_ak_rect(viewport));
        });
    }
}
