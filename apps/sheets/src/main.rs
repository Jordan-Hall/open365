//! Open Sheet: `open-sheet [file.xlsx|.xltx|.xlsm|.xltm]`.

mod blank;
mod format_ui;

use std::path::{Path, PathBuf};

use egui::{Key, KeyboardShortcut, Modifiers, Ui};
use open365_commands::{Command, Target};
use open365_ui_egui::{ClipboardState, DocView, FindDialog, GridColors, GridView, Resize};
use open365_ui_shell::{AppKind, CommandState, MenuEntry, Mode, Shell, ShellHost, Theme, TileFont};
use open365_xlsx::Workbook;

use format_ui::{Action, FormatUi, Sel};

const EXTENSIONS: [&str; 4] = ["xlsx", "xltx", "xlsm", "xltm"];

/// Everything except the chrome: the workbook, its grid and the status line.
struct State {
    target: Option<Target>,
    grid: GridView,
    /// Only the Find dialog needs a document view; Sheets never shows one.
    doc: DocView,
    find: FindDialog,
    clipboard: ClipboardState,
    status: String,
    recent: Vec<PathBuf>,
    untitled: u32,
    /// Row height (true) or column width (false) dialog and its value.
    size_dialog: Option<(bool, f64)>,
    /// A question waiting for OK or Cancel (the merge warning) and the command OK runs.
    prompt: Option<(&'static str, Command)>,
    /// Name Box text while it has focus.
    name_text: String,
    /// The OS UI font, handed to every new grid.
    ui_font: Option<(&'static str, Vec<u8>)>,
    /// Enter (true) or Escape (false) pressed on the formula bar buttons.
    formula_action: Option<bool>,
    /// Memory of the Home tab's formatting controls (Format Painter, last colours, pop-ups).
    fmt: FormatUi,
    /// Clock for the Format Painter double click.
    clock: Clock,
}

/// Seconds since start for double-click timing; a test can freeze it so timing never depends on the wall clock.
struct Clock {
    started: std::time::Instant,
    frozen: Option<f64>,
}

impl Clock {
    fn new() -> Self {
        Self {
            started: std::time::Instant::now(),
            frozen: None,
        }
    }

    fn now(&self) -> f64 {
        self.frozen
            .unwrap_or_else(|| self.started.elapsed().as_secs_f64())
    }
}

fn columns_label(first: u32, last: u32) -> String {
    let a = open365_xlsx_cols(first);
    if first == last {
        format!("column {a}")
    } else {
        format!("columns {a}:{}", open365_xlsx_cols(last))
    }
}

fn open365_xlsx_cols(mut n: u32) -> String {
    let mut s = String::new();
    while n > 0 {
        n -= 1;
        s.insert(0, char::from(b'A' + (n % 26) as u8));
        n /= 26;
    }
    s
}

impl State {
    fn new() -> Self {
        Self {
            target: None,
            grid: GridView::new(),
            doc: DocView::new(),
            find: FindDialog::new(),
            clipboard: ClipboardState::default(),
            status: String::new(),
            recent: Vec::new(),
            untitled: 0,
            size_dialog: None,
            prompt: None,
            name_text: String::new(),
            ui_font: open365_ui_shell::fonts::system_ui_font(),
            formula_action: None,
            fmt: FormatUi::default(),
            clock: Clock::new(),
        }
    }

    /// The sheet, active cell and selected range the formatting controls act on.
    fn sel(&self) -> Sel {
        Sel {
            sheet: self.grid.sheet(),
            active: self.grid.active(),
            range: self.grid.selection(),
        }
    }

    fn workbook(&self) -> Option<&Workbook> {
        self.target.as_ref().and_then(Target::xlsx)
    }

    /// Runs a formatting control or menu entry through the command layer.
    fn run_format(&mut self, id: &str) {
        let sel = self.sel();
        let now = self.clock.now();
        let action = self
            .fmt
            .run(id, self.target.as_ref().and_then(Target::xlsx), sel, now);
        match action {
            Action::None => {}
            Action::Apply(commands) => commands.into_iter().for_each(|c| self.apply(c)),
            Action::Confirm { text, command } => self.prompt = Some((text, *command)),
        }
    }

    fn set_target(&mut self, t: Target) {
        self.target = Some(t);
        self.fmt.reset_document();
        self.grid = GridView::new();
        if let Some((name, bytes)) = &self.ui_font {
            self.grid.set_ui_font(name, bytes.clone());
        }
    }

    fn new_blank(&mut self) {
        match Target::open_xlsx(&blank::xlsx()) {
            Ok(t) => {
                self.untitled += 1;
                self.set_target(t);
                self.status = "New blank workbook".into();
            }
            Err(e) => self.status = format!("Could not create a workbook: {e}"),
        }
    }

    fn open_path(&mut self, path: &Path) {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let result = if ext == "xltx" || ext == "xltm" {
            std::fs::read(path)
                .map_err(|e| e.to_string())
                .and_then(|b| Target::open_template(&b, &ext).map_err(|e| e.to_string()))
        } else {
            Target::open_path(path).map_err(|e| e.to_string())
        };
        match result {
            Ok(t) => {
                self.set_target(t);
                self.recent.retain(|p| p != path);
                self.recent.insert(0, path.to_path_buf());
                self.status = format!("Opened {}", path.display());
            }
            Err(e) => self.status = format!("Could not open {}: {e}", path.display()),
        }
    }

    fn open_dialog(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .add_filter("Workbooks", &EXTENSIONS)
            .set_title("Open")
            .pick_file()
        {
            self.open_path(&p);
        }
    }

    fn save(&mut self) {
        let path = self
            .target
            .as_ref()
            .and_then(|t| t.path().map(Path::to_path_buf));
        match path {
            Some(p) => self.save_to(p),
            None => self.save_as(),
        }
    }

    fn save_as(&mut self) {
        if self.target.is_none() {
            return;
        }
        if let Some(p) = rfd::FileDialog::new()
            .add_filter("Workbook", &["xlsx"])
            .set_title("Save As")
            .save_file()
        {
            self.save_to(p);
        }
    }

    fn save_to(&mut self, path: PathBuf) {
        let Some(t) = &mut self.target else { return };
        self.status = match t.apply(Command::Save { path: path.clone() }) {
            Ok(out) => format!("Saved {} ({} bytes)", path.display(), out.bytes_written),
            Err(e) => format!("Save failed: {e}"),
        };
    }

    /// Format Painter: once the user has chosen a range with the mouse, copies the source formats
    /// onto it. A single use switches off; the locked one stays on until Escape.
    fn format_painter_tick(&mut self, ctx: &egui::Context, grid_rect: egui::Rect) {
        if !self.fmt.painter_active() {
            return;
        }
        let (pressed, released, pos) = ctx.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.pointer.latest_pos(),
            )
        });
        let inside = pos.is_some_and(|p| grid_rect.contains(p));
        if inside {
            ctx.set_cursor_icon(egui::CursorIcon::Copy);
        }
        if pressed && inside {
            self.fmt.painter_pressed = true;
        }
        if released
            && self.fmt.painter_pressed
            && let Some(command) = self
                .fmt
                .finish_paint(self.grid.sheet(), self.grid.selection())
        {
            self.apply(command);
        }
    }

    fn undo(&mut self) {
        if let Some(t) = &mut self.target {
            let r = self.doc.undo(t);
            self.grid.invalidate_layout();
            if let Err(e) = r {
                self.status = e.to_string();
            }
        }
    }

    fn redo(&mut self) {
        if let Some(t) = &mut self.target {
            let r = self.doc.redo(t);
            self.grid.invalidate_layout();
            if let Err(e) = r {
                self.status = e.to_string();
            }
        }
    }

    /// Applies one structure command to the selected rows or columns.
    fn structure(&mut self, id: &str) {
        let (r1, c1, r2, c2) = self.grid.selection();
        let sheet = self.grid.sheet();
        let (rows, cols) = (r2 - r1 + 1, c2 - c1 + 1);
        let command = match id {
            "cells.insert-rows" => Command::InsertRows {
                sheet,
                at: r1,
                count: rows,
            },
            "cells.insert-cols" => Command::InsertColumns {
                sheet,
                at: c1,
                count: cols,
            },
            "cells.delete-rows" => Command::DeleteRows {
                sheet,
                at: r1,
                count: rows,
            },
            "cells.delete-cols" => Command::DeleteColumns {
                sheet,
                at: c1,
                count: cols,
            },
            "cells.hide-rows" => Command::HideRows {
                sheet,
                first: r1,
                last: r2,
            },
            "cells.hide-cols" => Command::HideColumns {
                sheet,
                first: c1,
                last: c2,
            },
            "cells.unhide-rows" => Command::UnhideRows {
                sheet,
                first: r1,
                last: r2,
            },
            "cells.unhide-cols" => Command::UnhideColumns {
                sheet,
                first: c1,
                last: c2,
                default_width: None,
            },
            _ => return,
        };
        let Some(t) = &mut self.target else { return };
        self.status = match t.apply(command) {
            Ok(_) => {
                self.grid.invalidate_layout();
                format!(
                    "{id} done for {}",
                    if id.ends_with("rows") {
                        format!("rows {r1}:{r2}")
                    } else {
                        columns_label(c1, c2)
                    }
                )
            }
            Err(e) => format!("Command failed: {e}"),
        };
    }

    fn apply_size(&mut self, is_row: bool, value: f64) {
        let (r1, c1, r2, c2) = self.grid.selection();
        let sheet = self.grid.sheet();
        let command = if is_row {
            Command::SetRowHeight {
                sheet,
                first: r1,
                last: r2,
                points: value,
            }
        } else {
            Command::SetColumnWidth {
                sheet,
                first: c1,
                last: c2,
                width: value,
            }
        };
        self.apply(command);
    }

    fn apply(&mut self, command: Command) {
        let Some(t) = &mut self.target else { return };
        self.grid.autofit_after(&command);
        self.status = match t.apply(command) {
            Ok(_) => {
                self.grid.invalidate_layout();
                "Done".to_owned()
            }
            Err(e) => format!("Command failed: {e}"),
        };
    }

    /// Applies the column widths and row heights the grid queued from dragging headers.
    fn apply_resizes(&mut self) {
        // Measured fits of one frame are one undo step each (columns, rows).
        let mut fit_cols: Vec<(u32, f64)> = Vec::new();
        let mut fit_rows: Vec<(u32, f64)> = Vec::new();
        let mut fit_sheet = self.grid.sheet();
        for resize in self.grid.take_resizes() {
            let command = match resize {
                Resize::ColumnFit { sheet, col, width } => {
                    fit_sheet = sheet;
                    fit_cols.push((col, width));
                    continue;
                }
                Resize::RowFit { sheet, row, points } => {
                    fit_sheet = sheet;
                    fit_rows.push((row, points));
                    continue;
                }
                Resize::ColumnWidth {
                    sheet,
                    first,
                    last,
                    width,
                } => Command::SetColumnWidth {
                    sheet,
                    first,
                    last,
                    width,
                },
                Resize::RowHeight {
                    sheet,
                    first,
                    last,
                    points,
                } => Command::SetRowHeight {
                    sheet,
                    first,
                    last,
                    points,
                },
            };
            self.apply(command);
        }
        if !fit_cols.is_empty() {
            self.apply(Command::AutoFitColumns {
                sheet: fit_sheet,
                columns: fit_cols,
            });
        }
        if !fit_rows.is_empty() {
            self.apply(Command::AutoFitRows {
                sheet: fit_sheet,
                rows: fit_rows,
            });
        }
    }

    fn copy(&mut self) {
        if let Some(t) = &self.target {
            self.status = if self.clipboard.copy_grid(t, &self.grid) {
                "Copied".into()
            } else {
                "That range is too large to copy".into()
            };
        }
    }

    fn cut(&mut self) {
        if let Some(t) = &mut self.target {
            self.status = self.clipboard.cut_grid(t, &mut self.grid);
        }
    }

    fn paste(&mut self) {
        let text = arboard::Clipboard::new()
            .ok()
            .and_then(|mut c| c.get_text().ok());
        if let (Some(t), Some(text)) = (&mut self.target, text) {
            self.status = self.clipboard.paste_grid(t, &mut self.grid, &text);
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let take = |m: Modifiers, k: Key| {
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k)))
        };
        let c = Modifiers::COMMAND;
        if take(c, Key::N) {
            self.new_blank();
        }
        if take(c, Key::O) {
            self.open_dialog();
        }
        if take(c.plus(Modifiers::SHIFT), Key::S) {
            self.save_as();
        }
        if take(c, Key::S) {
            self.save();
        }
        if self.target.is_some() {
            if take(c, Key::F) {
                self.find.show_dialog(false);
            }
            if take(c, Key::H) {
                self.find.show_dialog(true);
            }
        }
        if !self.grid.is_editing() {
            if take(c.plus(Modifiers::SHIFT), Key::Z) || take(c, Key::Y) {
                self.redo();
            }
            if take(c, Key::Z) {
                self.undo();
            }
        }
        if self.fmt.painter_active()
            && !self.grid.is_editing()
            && take(Modifiers::NONE, Key::Escape)
        {
            self.fmt.cancel_painter();
        }
        if self.find.text_focused
            || self.grid.is_editing()
            || ctx.current_pass_index() != 0
            || self.target.is_none()
            || ctx.memory(|m| m.has_focus(egui::Id::new("open365-name-box")))
        {
            return;
        }
        for id in format_shortcuts(ctx) {
            self.run_format(&id);
        }
        let events: Vec<egui::Event> = ctx.input_mut(|i| {
            let mut taken = Vec::new();
            i.events.retain(|e| {
                let ours = matches!(
                    e,
                    egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_)
                );
                if ours {
                    taken.push(e.clone());
                }
                !ours
            });
            taken
        });
        for e in events {
            match e {
                egui::Event::Copy => self.copy(),
                egui::Event::Cut => self.cut(),
                egui::Event::Paste(text) => {
                    if let Some(t) = &mut self.target {
                        self.status = self.clipboard.paste_grid(t, &mut self.grid, &text);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Takes the Ctrl+B/I/U/5 and Ctrl+Shift number-format key presses out of this frame's input and
/// gives the control ids they run. Matching uses the key's position, so a layout that types
/// the symbol with Shift still works.
fn format_shortcuts(ctx: &egui::Context) -> Vec<String> {
    ctx.input_mut(|i| {
        let mut ids = Vec::new();
        i.events.retain(|e| {
            let egui::Event::Key {
                key,
                physical_key,
                pressed: true,
                modifiers,
                ..
            } = e
            else {
                return true;
            };
            match format_ui::shortcut_id(*key, *physical_key, *modifiers) {
                Some(id) => {
                    ids.push(id);
                    false
                }
                None => true,
            }
        });
        ids
    })
}

struct Host<'a> {
    s: &'a mut State,
    theme: Theme,
}

/// The grid's colours for a theme: Excel keeps cells white in both modes.
fn grid_colors(t: &Theme) -> GridColors {
    GridColors {
        text: t.cell_text,
        grid_line: t.grid_line,
        cell_bg: t.cell_bg,
        header_bg: t.grid_header,
        header_sel_bg: t.grid_header_selected,
        header_text: t.grid_header_text,
        header_sel_text: t.text,
        header_border: t.grid_header_border,
        accent: t.grid_accent,
    }
}

impl ShellHost for Host<'_> {
    fn state(&self, id: &str) -> CommandState {
        let t = self.s.target.as_ref();
        let has = t.is_some();
        let on = |b: bool| {
            if b {
                CommandState::ENABLED
            } else {
                CommandState::DISABLED
            }
        };
        if format_ui::handles(id) {
            return self.s.fmt.state(id, self.s.workbook(), self.s.sel());
        }
        match id {
            "file.new" | "file.open" => CommandState::ENABLED,
            "qat.filesave" | "file.save" | "file.save-as" | "file.close" => on(has),
            "qat.undo" => on(t.is_some_and(Target::can_undo)),
            "qat.redo" | "qat.redoorrepeat" => on(t.is_some_and(Target::can_redo)),
            "home.clipboard.paste" | "home.clipboard.cut" | "home.clipboard.copy" => on(has),
            "home.cells.insert"
            | "home.cells.delete"
            | "home.cells.format"
            | "home.editing.find-select" => on(has),
            "formula.cancel" | "formula.enter" => on(self.s.grid.is_editing()),
            "status.normal" => CommandState::checked(true),
            _ => CommandState::DISABLED,
        }
    }

    /// A menu, list, group pop-up, combo editor or the Backstage view closed: the grid takes the keyboard back.
    fn chrome_closed(&mut self) {
        self.s.grid.focus();
    }

    fn run(&mut self, id: &str, _value: Option<f64>) {
        let s = &mut *self.s;
        if let Some(i) = id
            .strip_prefix("file.open-recent#")
            .and_then(|i| i.parse::<usize>().ok())
        {
            if let Some(p) = s.recent.get(i).cloned() {
                s.open_path(&p);
            }
            return;
        }
        if format_ui::handles(id) {
            s.run_format(id);
            return;
        }
        match id {
            "file.new" => s.new_blank(),
            "file.open" => s.open_dialog(),
            "qat.filesave" | "file.save" => s.save(),
            "file.save-as" => s.save_as(),
            "file.close" => s.target = None,
            "qat.undo" => s.undo(),
            "qat.redo" | "qat.redoorrepeat" => s.redo(),
            "home.clipboard.paste" => s.paste(),
            "home.clipboard.cut" => s.cut(),
            "home.clipboard.copy" => s.copy(),
            "edit.find" => s.find.show_dialog(false),
            "edit.replace" => s.find.show_dialog(true),
            "cells.row-height" => {
                let h = s.target.as_ref().map_or(15.0, |_| 15.0);
                s.size_dialog = Some((true, h));
            }
            "cells.col-width" => s.size_dialog = Some((false, 8.43)),
            "cells.autofit-rows" => s.grid.autofit_selection(false),
            "cells.autofit-cols" => s.grid.autofit_selection(true),
            "sheet.select" => {
                if let Some(v) = _value {
                    s.grid.select_sheet(v as usize);
                }
            }
            "formula.enter" => s.formula_action = Some(true),
            "formula.cancel" => s.formula_action = Some(false),
            other if other.starts_with("cells.") => s.structure(other),
            _ => {}
        }
    }

    fn title(&self) -> String {
        match self.s.target.as_ref().and_then(|t| t.path()) {
            Some(p) => p
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            None if self.s.target.is_some() => format!("Book{}", self.s.untitled),
            None => open365_branding::SUITE.to_owned(),
        }
    }

    fn status_text(&self, id: &str) -> Option<String> {
        (id == "status.mode").then(|| "Ready".to_owned())
    }

    fn combo_commit(&mut self, id: &str, text: &str) -> Option<String> {
        self.s.fmt.combo_commit(id, text)
    }

    fn menu(&self, id: &str) -> Option<Vec<MenuEntry>> {
        if format_ui::handles(id) {
            let face = |name: &str, bold: bool, italic: bool| {
                let (family, fake_bold, fake_italic) = self.s.grid.preview_face(name, bold, italic);
                TileFont {
                    family,
                    fake_bold,
                    fake_italic,
                }
            };
            return self.s.fmt.menu(id, self.s.workbook(), self.s.sel(), &face);
        }
        let e = |i: &str, l: &str| MenuEntry::new(i, l, true);
        match id {
            "home.cells.insert" => Some(vec![
                e("cells.insert-rows", "Insert Sheet Rows"),
                e("cells.insert-cols", "Insert Sheet Columns"),
            ]),
            "home.cells.delete" => Some(vec![
                e("cells.delete-rows", "Delete Sheet Rows"),
                e("cells.delete-cols", "Delete Sheet Columns"),
            ]),
            "home.cells.format" => Some(vec![
                e("cells.row-height", "Row Height..."),
                e("cells.autofit-rows", "AutoFit Row Height"),
                e("cells.col-width", "Column Width..."),
                e("cells.autofit-cols", "AutoFit Column Width"),
                e("cells.hide-rows", "Hide Rows"),
                e("cells.hide-cols", "Hide Columns"),
                e("cells.unhide-rows", "Unhide Rows"),
                e("cells.unhide-cols", "Unhide Columns"),
            ]),
            "home.editing.find-select" => {
                Some(vec![e("edit.find", "Find"), e("edit.replace", "Replace")])
            }
            _ => None,
        }
    }

    fn value(&self, id: &str) -> Option<String> {
        if format_ui::handles(id) {
            return self.s.fmt.value(id, self.s.workbook(), self.s.sel());
        }
        match id {
            "sheet.active" => Some(self.s.grid.sheet().to_string()),
            "formula.name" => Some(self.s.grid.name_box_text()),
            _ => None,
        }
    }

    fn gallery(&self, id: &str) -> Vec<(String, String)> {
        if id == "home.styles.cell-styles" {
            return ["Normal", "Bad", "Good", "Neutral"]
                .map(|n| (n.to_owned(), n.to_owned()))
                .to_vec();
        }
        match (id, &self.s.target) {
            ("sheet.tabs", Some(t)) => self
                .s
                .grid
                .sheet_names(t)
                .into_iter()
                .map(|(i, n)| (i.to_string(), n))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn name_box_ui(&mut self, ui: &mut Ui) -> bool {
        let s = &mut *self.s;
        if s.target.is_none() {
            return false;
        }
        let id = egui::Id::new("open365-name-box");
        if !ui.memory(|m| m.has_focus(id)) {
            s.name_text = s.grid.name_box_text();
        }
        let resp = ui.add(
            egui::TextEdit::singleline(&mut s.name_text)
                .id(id)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .vertical_align(egui::Align::Center),
        );
        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            if s.grid.go_to(&s.name_text) {
                s.grid.focus();
            } else {
                s.status = format!("{} is not a valid reference", s.name_text);
            }
        }
        true
    }

    fn formula_ui(&mut self, ui: &mut Ui) -> bool {
        let s = &mut *self.s;
        match &mut s.target {
            Some(t) => {
                s.grid.formula_text_ui(ui, t);
                true
            }
            None => false,
        }
    }

    fn recent(&self) -> Vec<String> {
        self.s
            .recent
            .iter()
            .map(|p| p.display().to_string())
            .collect()
    }

    fn info(&self) -> Vec<(String, String)> {
        let Some(t) = &self.s.target else {
            return Vec::new();
        };
        vec![
            ("Name".into(), self.title()),
            (
                "Location".into(),
                t.path()
                    .map_or("Not saved yet".into(), |p| p.display().to_string()),
            ),
            (
                "Unsaved changes".into(),
                if t.is_dirty() { "Yes" } else { "No" }.into(),
            ),
        ]
    }

    fn central(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let s = &mut *self.s;
        s.shortcuts(&ctx);
        let grid_rect = ui.max_rect();
        match &mut s.target {
            Some(t) => {
                s.grid.set_colors(Some(grid_colors(&self.theme)));
                s.grid.show_grid(ui, t);
                s.format_painter_tick(&ctx, grid_rect);
            }
            None => {
                ui.vertical_centered(|ui| {
                    ui.add_space(80.0);
                    ui.heading(open365_branding::SHEET_NAME);
                    ui.label("Create or open a workbook to begin.");
                    if ui.button("New blank workbook").clicked() {
                        s.new_blank();
                    }
                    if ui.button("Open...").clicked() {
                        s.open_dialog();
                    }
                });
            }
        }
        if let Some(m) = s.grid.status.take() {
            s.status = m;
        }
        if let Some(action) = s.find.ui(&ctx, false)
            && let Some(t) = &mut s.target
        {
            s.status = s.find.run(action, t, &mut s.doc, &mut s.grid);
        }
        s.apply_resizes();
        if let Some(enter) = s.formula_action.take()
            && let Some(t) = &mut s.target
        {
            if enter {
                s.grid.commit_edit(&ctx, t, false);
            } else {
                s.grid.cancel_edit(&ctx);
            }
        }
        if let Some((text, command)) = s.prompt.take() {
            // None: still open; Some(true): OK; Some(false): Cancel.
            let mut answer = None;
            egui::Window::new(open365_branding::SHEET_NAME)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(&ctx, |ui| {
                    ui.label(text);
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            answer = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            answer = Some(false);
                        }
                    });
                });
            ctx.input(|i| {
                if i.key_pressed(egui::Key::Enter) {
                    answer = Some(true);
                } else if i.key_pressed(egui::Key::Escape) {
                    answer = Some(false);
                }
            });
            match answer {
                Some(true) => s.apply(command),
                Some(false) => {}
                None => s.prompt = Some((text, command)),
            }
        }
        if let Some((is_row, mut v)) = s.size_dialog {
            let mut open = true;
            let mut apply = false;
            egui::Window::new(if is_row { "Row Height" } else { "Column Width" })
                .open(&mut open)
                .collapsible(false)
                .show(&ctx, |ui| {
                    ui.add(egui::DragValue::new(&mut v).range(0.0..=409.5));
                    apply = ui.button("OK").clicked();
                });
            if apply {
                s.apply_size(is_row, v);
                open = false;
            }
            s.size_dialog = open.then_some((is_row, v));
        }
    }
}

struct SheetsApp {
    shell: Shell,
    state: State,
    title: String,
}

impl eframe::App for SheetsApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let mut host = Host {
            s: &mut self.state,
            theme: self.shell.theme,
        };
        self.shell.show(ui, &mut host);
        let title = open365_branding::window_title(&host.title(), open365_branding::SHEET_NAME);
        if title != self.title {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

/// The window and taskbar icon (the same art as the executable resource).
fn app_icon() -> egui::IconData {
    let png = include_bytes!("../../../assets/icons/open-sheet-256.png");
    match image::load_from_memory_with_format(png, image::ImageFormat::Png) {
        Ok(img) => {
            let rgba = img.into_rgba8();
            let (width, height) = rgba.dimensions();
            egui::IconData {
                rgba: rgba.into_raw(),
                width,
                height,
            }
        }
        Err(_) => egui::IconData::default(),
    }
}

fn main() -> eframe::Result {
    let initial = std::env::args_os().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(open365_branding::SHEET_NAME)
            .with_icon(app_icon())
            .with_decorations(false)
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([160.0, 160.0]),
        ..eframe::NativeOptions::default()
    };
    eframe::run_native(
        open365_branding::SHEET_NAME,
        options,
        Box::new(move |_cc| {
            // The grid owns egui's font table, so the OS UI font is handed to the grid.
            let mut state = State::new();
            match initial {
                Some(p) => state.open_path(&p),
                None => state.new_blank(),
            }
            let mut shell = Shell::new(AppKind::Sheets, Mode::system());
            shell.options.custom_frame = true;
            shell.options.formula_bar = true;
            shell.options.sheet_tabs = true;
            Ok(Box::new(SheetsApp {
                shell,
                state,
                title: String::new(),
            }))
        }),
    )
}

#[cfg(test)]
mod format_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// Both applications take the mode from the same source (Windows' app mode, as Office does).
    #[test]
    fn theme_follows_the_system_mode() {
        let shell = Shell::new(AppKind::Sheets, Mode::system());
        assert_eq!(shell.theme.mode, Mode::system());
    }

    /// Sheets as `main` builds it: shell and state on a headless window.
    fn app_harness() -> egui_kittest::Harness<'static, (Shell, State)> {
        let mut state = State::new();
        state.new_blank();
        let mut shell = Shell::new(AppKind::Sheets, Mode::system());
        shell.options.formula_bar = true;
        shell.options.sheet_tabs = true;
        egui_kittest::Harness::builder()
            .with_size(egui::vec2(1600.0, 900.0))
            .build_ui_state(
                |ui, (shell, state): &mut (Shell, State)| {
                    let mut host = Host {
                        s: state,
                        theme: shell.theme,
                    };
                    shell.show(ui, &mut host);
                },
                (shell, state),
            )
    }

    /// Like Writer: when a menu, list or the Backstage view closes the grid gets the keyboard back.
    #[test]
    fn closing_chrome_returns_focus_to_the_grid() {
        let mut h = app_harness();
        h.run();
        h.ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
        h.run();
        assert!(!h.state().1.grid.is_focused(&h.ctx));
        let theme = h.state().0.theme;
        let mut host = Host {
            s: &mut h.state_mut().1,
            theme,
        };
        host.chrome_closed();
        h.run();
        assert!(h.state().1.grid.is_focused(&h.ctx));
    }

    #[test]
    fn blank_workbook_opens_and_saves() {
        let mut t = Target::open_xlsx(&blank::xlsx()).expect("blank xlsx opens");
        assert!(t.xlsx().is_some());
        assert!(t.snapshot_bytes().is_ok());
    }
}
