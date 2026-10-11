//! The Sheets preview's document: a workbook held by the command layer's [`Target`]. Every cell
//! edit is a [`Command`] (so it is audited, undoable and recalculated by the calc engine); this
//! type only adds the active cell, the scroll origin and the text a cell shows.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use open365_commands::{
    CellEntry, CellRef, Command, EffectClass, Target, cell_display_text, cell_source_text,
    grid_copy, parse_tsv, paste_rows,
};
use open365_xlsx::{CellValue, GridMetrics, SheetGeometry};

#[path = "shell/barfmt.rs"]
pub mod barfmt;
#[path = "../../sheets/src/blank.rs"]
mod blank;

/// Last row and column of a worksheet.
pub const MAX_ROW: u32 = 1_048_576;
pub const MAX_COL: u32 = 16_384;

/// Where the clipboard text goes and comes from.
pub trait ClipboardIo {
    /// Puts `tsv` (plain text) and `html` on the clipboard.
    fn set(&mut self, tsv: &str, html: &str) -> Result<(), String>;
    /// The plain text on the clipboard.
    fn text(&mut self) -> Option<String>;
}

/// The operating system clipboard (arboard): CF_UNICODETEXT plus CF_HTML.
#[derive(Debug, Default)]
pub struct SystemClipboard;

impl ClipboardIo for SystemClipboard {
    fn set(&mut self, tsv: &str, html: &str) -> Result<(), String> {
        let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        cb.set().html(html, Some(tsv)).map_err(|e| e.to_string())
    }

    fn text(&mut self) -> Option<String> {
        arboard::Clipboard::new().ok()?.get_text().ok()
    }
}

/// An in-process clipboard; clones share the contents (tests keep one handle).
#[derive(Debug, Clone, Default)]
pub struct MemoryClipboard(pub Rc<RefCell<(String, String)>>);

impl MemoryClipboard {
    pub fn tsv(&self) -> String {
        self.0.borrow().0.clone()
    }

    pub fn html(&self) -> String {
        self.0.borrow().1.clone()
    }

    pub fn put_text(&self, text: &str) {
        *self.0.borrow_mut() = (text.to_owned(), String::new());
    }
}

impl ClipboardIo for MemoryClipboard {
    fn set(&mut self, tsv: &str, html: &str) -> Result<(), String> {
        *self.0.borrow_mut() = (tsv.to_owned(), html.to_owned());
        Ok(())
    }

    fn text(&mut self) -> Option<String> {
        let t = self.0.borrow().0.clone();
        (!t.is_empty()).then_some(t)
    }
}

/// Asks the person where to save: gets the suggested file name, returns the chosen path.
pub type SaveChooser = Box<dyn FnMut(&str) -> Option<PathBuf>>;

/// The native Save As dialog (rfd).
pub fn system_save_chooser() -> SaveChooser {
    Box::new(|name| {
        rfd::FileDialog::new()
            .set_title("Save As")
            .set_file_name(name)
            .add_filter("Excel workbook (*.xlsx)", &["xlsx"])
            .save_file()
    })
}

/// A pointer gesture in progress on the grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gesture {
    /// Dragging out a cell range.
    Select,
    /// Dragging across column headers from `anchor`.
    Cols { anchor: u32 },
    /// Dragging across row headers from `anchor`.
    Rows { anchor: u32 },
    /// Dragging a column edge: `px` is the live width in logical pixels.
    ResizeCol {
        col: u32,
        start_x: f32,
        start_px: u32,
        px: u32,
    },
    /// Dragging a row edge: `px` is the live height in logical pixels.
    ResizeRow {
        row: u32,
        start_y: f32,
        start_px: u32,
        px: u32,
    },
    /// Dragging the fill handle; `to` is the cell under the pointer.
    Fill { to: (u32, u32) },
}

/// What was copied inside this application (formulas shift on paste, a cut clears its source).
#[derive(Debug, Clone)]
struct Copied {
    origin: CellRef,
    tsv: String,
    rows: Vec<Vec<String>>,
    cut: Option<(u32, u32, u32, u32)>,
}

/// The open workbook, the active cell and the first visible cell (1-based row, column).
pub struct SheetModel {
    target: Target,
    pub sheet: usize,
    pub active: (u32, u32),
    /// The fixed and the moving corner of the selected range.
    pub anchor: (u32, u32),
    pub focus: (u32, u32),
    pub top_left: (u32, u32),
    pub status: String,
    pub gesture: Option<Gesture>,
    geo_cache: RefCell<Option<(u64, usize, GridGeometry)>>,
    clipboard: Box<dyn ClipboardIo>,
    copied: Option<Copied>,
    pending_paste: Option<(Command, Option<Rect4>)>,
    chooser: SaveChooser,
    locale: Arc<open365_calc::FormatLocale>,
}

impl std::fmt::Debug for SheetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SheetModel")
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

impl SheetModel {
    /// An empty workbook with one sheet.
    pub fn blank() -> Self {
        let target = Target::open_xlsx(&blank::xlsx()).expect("the built-in blank workbook opens");
        Self::with_target(target)
    }

    /// Opens `.xlsx`, `.xlsm`, `.xltx` or `.xltm` from `path`.
    pub fn open(path: &Path) -> Result<Self, String> {
        Target::open_path(path)
            .map(Self::with_target)
            .map_err(|e| e.to_string())
    }

    fn with_target(target: Target) -> Self {
        let mut model = Self::from_parts(target);
        model.clamp_scroll();
        model
    }

    fn from_parts(target: Target) -> Self {
        Self {
            target,
            sheet: 0,
            active: (1, 1),
            anchor: (1, 1),
            focus: (1, 1),
            top_left: (1, 1),
            status: String::new(),
            gesture: None,
            geo_cache: RefCell::new(None),
            clipboard: Box::new(SystemClipboard),
            copied: None,
            pending_paste: None,
            chooser: system_save_chooser(),
            locale: barfmt::default_locale(),
        }
    }

    /// Replaces the clipboard (tests use [`MemoryClipboard`]).
    pub fn set_clipboard(&mut self, clipboard: impl ClipboardIo + 'static) {
        self.clipboard = Box::new(clipboard);
    }

    /// Replaces the Save As dialog.
    pub fn set_save_chooser(&mut self, chooser: SaveChooser) {
        self.chooser = chooser;
    }

    /// Writes the workbook to `path` as XLSX.
    pub fn save(&mut self, path: &Path) -> Result<(), String> {
        let bytes = self.target.snapshot_bytes().map_err(|e| e.to_string())?;
        std::fs::write(path, bytes).map_err(|e| e.to_string())
    }

    /// Where the workbook was opened from.
    /// Runs a `share_*` command: it never changes the workbook, its history or its saved state.
    pub fn apply_share(&mut self, command: Command) -> Result<open365_commands::Outcome, String> {
        if !command.is_share() {
            return Err("not a share command".to_owned());
        }
        self.target.apply(command).map_err(|e| e.to_string())
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.target.path().map(Path::to_path_buf)
    }

    pub fn is_dirty(&self) -> bool {
        self.target.is_dirty()
    }

    /// The cell as the formula bar shows it: `=formula` or the constant.
    pub fn source(&self, row: u32, col: u32) -> String {
        self.target
            .xlsx()
            .map(|wb| cell_source_text(wb, self.sheet, row, col))
            .unwrap_or_default()
    }

    /// The locale dates, times and percentages are written in (en-US until set).
    pub fn set_locale(&mut self, locale: Arc<open365_calc::FormatLocale>) {
        self.locale = locale;
    }

    pub fn locale(&self) -> Arc<open365_calc::FormatLocale> {
        Arc::clone(&self.locale)
    }

    /// The number format code of a cell.
    fn format_code(&self, row: u32, col: u32) -> String {
        let Some(wb) = self.target.xlsx() else {
            return String::new();
        };
        let style = wb
            .cell(self.sheet, row, col)
            .and_then(|c| c.style)
            .unwrap_or(0);
        let styles = wb.styles();
        let id = styles.num_fmt_id(style);
        if styles.custom_formats().contains_key(&id) {
            styles.format_code(id).to_owned()
        } else {
            open365_calc::numfmt::builtin_format_code(id)
                .map_or_else(|| styles.format_code(id).to_owned(), str::to_owned)
        }
    }

    /// The cell as the formula bar and the cell editor show it: like [`SheetModel::source`],
    /// but a date, a time or a percentage reads the way Excel writes it in the system locale.
    pub fn bar_text(&self, row: u32, col: u32) -> String {
        let raw = self.source(row, col);
        let Some(wb) = self.target.xlsx() else {
            return raw;
        };
        match wb
            .cell(self.sheet, row, col)
            .map(|c| (&c.value, c.formula.is_some()))
        {
            Some((CellValue::Number(n), false)) => {
                barfmt::number_text(*n, &self.format_code(row, col), wb.date1904(), &self.locale)
            }
            _ => raw,
        }
    }

    /// What `text` typed into a cell with a date, time or percent format stands for: the number.
    fn typed_number(&self, row: u32, col: u32, text: &str) -> Option<f64> {
        let wb = self.target.xlsx()?;
        match barfmt::classify(&self.format_code(row, col)) {
            barfmt::BarKind::DateTime => barfmt::parse_datetime(text, &self.locale, wb.date1904()),
            barfmt::BarKind::Percent => barfmt::parse_percent(text),
            barfmt::BarKind::Plain => None,
        }
    }

    /// The text the cell displays (value through its number format).
    pub fn display(&self, row: u32, col: u32) -> String {
        self.target
            .xlsx()
            .map(|wb| cell_display_text(wb, self.sheet, row, col))
            .unwrap_or_default()
    }

    /// Display text and whether it is a number (right aligned), `None` for an empty cell.
    pub fn shown(&self, row: u32, col: u32) -> Option<(String, bool)> {
        let wb = self.target.xlsx()?;
        let cell = wb.cell(self.sheet, row, col)?;
        if matches!(cell.value, CellValue::Empty) {
            return None;
        }
        let numeric = matches!(cell.value, CellValue::Number(_));
        Some((cell_display_text(wb, self.sheet, row, col), numeric))
    }

    /// Enters `text` into a cell as Excel does: `=` starts a formula, anything else is a number,
    /// a boolean or text. No command runs when the text equals the cell's current source.
    pub fn enter(&mut self, row: u32, col: u32, text: &str) -> Result<(), String> {
        if self.source(row, col) == text || self.bar_text(row, col) == text {
            return Ok(());
        }
        if !text.trim_start().starts_with('=')
            && let Some(n) = self.typed_number(row, col, text.trim())
        {
            return self.apply(Command::XlsxSetCellValue {
                sheet: self.sheet,
                row,
                col,
                value: CellEntry::Number(n),
            });
        }
        self.apply(Command::xlsx_enter(self.sheet, row, col, text))
    }

    /// Delete: empties the cell (its format stays).
    pub fn clear(&mut self, row: u32, col: u32) -> Result<(), String> {
        if self.source(row, col).is_empty() {
            return Ok(());
        }
        self.apply(Command::XlsxSetCellValue {
            sheet: self.sheet,
            row,
            col,
            value: CellEntry::Empty,
        })
    }

    fn apply(&mut self, command: Command) -> Result<(), String> {
        match self.target.apply(command) {
            Ok(_) => {
                self.status.clear();
                Ok(())
            }
            Err(e) => {
                self.status = e.to_string();
                Err(self.status.clone())
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        self.target.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.target.can_redo()
    }

    pub fn undo(&mut self) -> bool {
        self.target.undo().is_ok()
    }

    pub fn redo(&mut self) -> bool {
        self.target.redo().is_ok()
    }

    /// Moves the active cell by `(d_row, d_col)`, clamped to the sheet; the selection collapses.
    pub fn move_by(&mut self, d_row: i64, d_col: i64) {
        let r = (i64::from(self.active.0) + d_row).clamp(1, i64::from(MAX_ROW)) as u32;
        let c = (i64::from(self.active.1) + d_col).clamp(1, i64::from(MAX_COL)) as u32;
        self.select_cell((r, c));
    }

    /// Scrolls just enough that the active cell lies in a window of `rows` by `cols` cells.
    pub fn reveal_active(&mut self, rows: u32, cols: u32) {
        self.reveal(self.active, rows, cols);
    }

    /// Scrolls just enough that `cell` lies in a window of `rows` by `cols` cells.
    pub fn reveal(&mut self, cell: (u32, u32), rows: u32, cols: u32) {
        let (r, c) = cell;
        let (rows, cols) = (rows.max(1), cols.max(1));
        let (fr, fc) = self.frozen();
        // A cell in a frozen pane is always in view.
        if r > fr {
            if r < self.top_left.0 {
                self.top_left.0 = r;
            } else if r >= self.top_left.0 + rows {
                self.top_left.0 = r + 1 - rows;
            }
        }
        if c > fc {
            if c < self.top_left.1 {
                self.top_left.1 = c;
            } else if c >= self.top_left.1 + cols {
                self.top_left.1 = c + 1 - cols;
            }
        }
        self.clamp_scroll();
    }
}

/// Columns wider than this many logical pixels are refused (Excel's limit is 255 characters).
const MAX_COL_PX: u32 = 2302;
/// Rows taller than this many points are refused (Excel's limit).
const MAX_ROW_PT: f64 = 409.5;
/// Largest copy, in cells.
const MAX_COPY_CELLS: usize = 4_000_000;

/// The result of a paste request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paste {
    /// Written.
    Done,
    /// It would overwrite data: nothing changed until [`SheetModel::confirm_paste`].
    NeedsConfirm,
    /// The clipboard held no text.
    Empty,
}

type Rect4 = (u32, u32, u32, u32);

fn rect_of(a: (u32, u32), b: (u32, u32)) -> Rect4 {
    (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1))
}

impl SheetModel {
    // ---- selection ---------------------------------------------------------------------

    /// The selected range `(first row, first col, last row, last col)`.
    pub fn range(&self) -> Rect4 {
        let (r1, c1, r2, c2) = rect_of(self.anchor, self.focus);
        let (r, c) = self.active;
        if (r1..=r2).contains(&r) && (c1..=c2).contains(&c) {
            (r1, c1, r2, c2)
        } else {
            (r, c, r, c)
        }
    }

    /// True when more than the active cell is selected.
    pub fn is_range(&self) -> bool {
        let (r1, c1, r2, c2) = self.range();
        (r1, c1) != (r2, c2)
    }

    pub fn select_cell(&mut self, cell: (u32, u32)) {
        self.active = cell;
        self.anchor = cell;
        self.focus = cell;
    }

    /// The corner that moves when the selection grows.
    fn moving_corner(&self) -> (u32, u32) {
        if rect_of(self.anchor, self.focus) == self.range() {
            self.focus
        } else {
            self.active
        }
    }

    /// Moves the moving corner (Shift+click, Shift+arrows, drag).
    pub fn extend_to(&mut self, cell: (u32, u32)) {
        if rect_of(self.anchor, self.focus) != self.range() {
            self.anchor = self.active;
        }
        self.focus = (cell.0.clamp(1, MAX_ROW), cell.1.clamp(1, MAX_COL));
    }

    /// Shift+arrow: moves the moving corner by one cell.
    pub fn extend_by(&mut self, d_row: i64, d_col: i64) {
        let f = self.moving_corner();
        let r = (i64::from(f.0) + d_row).clamp(1, i64::from(MAX_ROW)) as u32;
        let c = (i64::from(f.1) + d_col).clamp(1, i64::from(MAX_COL)) as u32;
        self.extend_to((r, c));
    }

    /// Ctrl+A: every cell of the sheet.
    pub fn select_all(&mut self) {
        self.anchor = (1, 1);
        self.focus = (MAX_ROW, MAX_COL);
    }

    /// Click or drag on column headers: whole columns `a..=b`. The active cell moves to the
    /// first visible row of column `a` unless `keep_active`.
    pub fn select_cols(&mut self, a: u32, b: u32, keep_active: bool) {
        if !keep_active {
            self.active = (self.top_left.0, a);
        }
        self.anchor = (1, a);
        self.focus = (MAX_ROW, b);
    }

    pub fn select_rows(&mut self, a: u32, b: u32, keep_active: bool) {
        if !keep_active {
            self.active = (a, self.top_left.1);
        }
        self.anchor = (a, 1);
        self.focus = (b, MAX_COL);
    }

    fn filled(&self, row: u32, col: u32) -> bool {
        self.target
            .xlsx()
            .and_then(|wb| wb.cell(self.sheet, row, col))
            .is_some_and(|c| !matches!(c.value, CellValue::Empty) || c.formula.is_some())
    }

    /// Where Ctrl+arrow takes `from`: the edge of the block of data, the start of the next
    /// block, or the edge of the sheet. `(d_row, d_col)` has one non-zero entry.
    pub fn jump(&self, from: (u32, u32), d_row: i64, d_col: i64) -> (u32, u32) {
        let used = self
            .target
            .xlsx()
            .and_then(|wb| wb.sheet(self.sheet))
            .and_then(|ws| ws.used_range());
        let vertical = d_row != 0;
        let step = if vertical {
            d_row.signum()
        } else {
            d_col.signum()
        };
        let (pos, limit) = if vertical {
            (from.0, if step > 0 { MAX_ROW } else { 1 })
        } else {
            (from.1, if step > 0 { MAX_COL } else { 1 })
        };
        let cell = |p: u32| if vertical { (p, from.1) } else { (from.0, p) };
        let edge = |p: u32| {
            let at = cell(p);
            self.filled(at.0, at.1)
        };
        // Past the last used row or column there is nothing: go to the edge.
        let beyond = |p: u32| match used {
            None => true,
            Some((r1, c1, r2, c2)) => {
                let (lo, hi) = if vertical { (r1, r2) } else { (c1, c2) };
                if step > 0 { p >= hi } else { p <= lo }
            }
        };
        let advance = |p: u32| (i64::from(p) + step) as u32;
        if pos == limit || step == 0 {
            return from;
        }
        let next = advance(pos);
        if edge(pos) && edge(next) {
            let mut p = next;
            while p != limit && edge(advance(p)) {
                p = advance(p);
            }
            return cell(p);
        }
        let mut p = next;
        loop {
            if edge(p) {
                return cell(p);
            }
            if p == limit || beyond(p) {
                return cell(limit);
            }
            p = advance(p);
        }
    }

    /// Ctrl+arrow and Ctrl+Shift+arrow.
    pub fn jump_selection(&mut self, d_row: i64, d_col: i64, extend: bool) {
        if extend {
            let to = self.jump(self.moving_corner(), d_row, d_col);
            self.extend_to(to);
        } else {
            let to = self.jump(self.active, d_row, d_col);
            self.select_cell(to);
        }
    }

    // ---- geometry and scrolling ----------------------------------------------------------

    /// Column widths, row heights and frozen panes of the active sheet, cached per revision.
    pub fn geometry(&self) -> GridGeometry {
        let key = (self.target.revision(), self.sheet);
        if let Some((rev, sheet, geo)) = self.geo_cache.borrow().as_ref()
            && (*rev, *sheet) == key
        {
            return geo.clone();
        }
        let (geo, frozen) = match self.target.xlsx() {
            Some(wb) => {
                let metrics = GridMetrics::for_styles(wb.styles(), 96);
                match wb.sheet(self.sheet) {
                    Some(ws) => (
                        ws.geometry(metrics),
                        ws.frozen_panes().map_or((0, 0), |f| (f.rows, f.cols)),
                    ),
                    None => (SheetGeometry::uniform(metrics), (0, 0)),
                }
            }
            None => (SheetGeometry::uniform(GridMetrics::new(7, 96)), (0, 0)),
        };
        let geo = GridGeometry {
            sheet: Arc::new(geo),
            frozen,
        };
        *self.geo_cache.borrow_mut() = Some((key.0, key.1, geo.clone()));
        geo
    }

    /// Rows and columns frozen at the top and left of the sheet, as the file says.
    pub fn frozen(&self) -> (u32, u32) {
        self.geometry().frozen
    }

    /// Keeps the scroll origin out of the frozen panes: the first scrolling row and column are
    /// the ones after them.
    pub fn clamp_scroll(&mut self) {
        let (fr, fc) = self.frozen();
        self.top_left.0 = self.top_left.0.clamp(fr + 1, MAX_ROW);
        self.top_left.1 = self.top_left.1.clamp(fc + 1, MAX_COL);
    }

    /// Mouse wheel: `notches` positive scrolls up, three rows (or columns) per notch.
    pub fn scroll_wheel(&mut self, notches: f32, horizontal: bool) {
        let by = (-notches * 3.0).round() as i64;
        if horizontal {
            self.top_left.1 = (i64::from(self.top_left.1) + by).clamp(1, i64::from(MAX_COL)) as u32;
        } else {
            self.top_left.0 = (i64::from(self.top_left.0) + by).clamp(1, i64::from(MAX_ROW)) as u32;
        }
        self.clamp_scroll();
    }

    /// Sets the first visible row and column.
    pub fn scroll_to(&mut self, row: Option<u32>, col: Option<u32>) {
        if let Some(r) = row {
            self.top_left.0 = r.clamp(1, MAX_ROW);
        }
        if let Some(c) = col {
            self.top_left.1 = c.clamp(1, MAX_COL);
        }
        self.clamp_scroll();
    }

    /// Rows and columns the scroll bars span for a window showing `rows` by `cols`: the used
    /// range plus one window, and always the current position.
    pub fn scroll_extent(&self, rows: u32, cols: u32) -> (u32, u32) {
        let used = self
            .target
            .xlsx()
            .and_then(|wb| wb.sheet(self.sheet))
            .and_then(|ws| ws.used_range())
            .map_or((0, 0), |u| (u.2, u.3));
        (
            used.0
                .max(self.top_left.0 + rows)
                .saturating_add(rows)
                .min(MAX_ROW),
            used.1
                .max(self.top_left.1 + cols)
                .saturating_add(cols)
                .min(MAX_COL),
        )
    }

    // ---- sizes -----------------------------------------------------------------------------

    /// Resizes column `col` to `px` logical pixels. When `col` lies in a selection of whole
    /// columns, every selected column gets the width, as in Excel.
    pub fn set_col_px(&mut self, col: u32, px: u32) -> Result<(), String> {
        let m = self.geometry().metrics();
        let px = px.min(MAX_COL_PX);
        let (r1, c1, r2, c2) = self.range();
        let (first, last) = if r1 == 1 && r2 == MAX_ROW && (c1..=c2).contains(&col) {
            (c1, c2)
        } else {
            (col, col)
        };
        self.apply(Command::SetColumnWidth {
            sheet: self.sheet,
            first,
            last,
            width: width_for_px(m, px),
        })
    }

    /// Resizes row `row` to `px` logical pixels (the selection rule of [`Self::set_col_px`]).
    pub fn set_row_px(&mut self, row: u32, px: u32) -> Result<(), String> {
        let m = self.geometry().metrics();
        let (r1, c1, r2, c2) = self.range();
        let (first, last) = if c1 == 1 && c2 == MAX_COL && (r1..=r2).contains(&row) {
            (r1, r2)
        } else {
            (row, row)
        };
        self.apply(Command::SetRowHeight {
            sheet: self.sheet,
            first,
            last,
            points: points_for_px(m, px).min(MAX_ROW_PT),
        })
    }

    /// Double click on a column edge: the width that fits the widest text, measured by
    /// `measure` in logical pixels. An empty column keeps its width.
    pub fn autofit_col(&mut self, col: u32, measure: &dyn Fn(&str) -> f32) -> Result<(), String> {
        let m = self.geometry().metrics();
        let Some(wb) = self.target.xlsx() else {
            return Ok(());
        };
        let widest = wb
            .iter_cells(self.sheet)
            .filter(|(_, c, cell)| *c == col && !matches!(cell.value, CellValue::Empty))
            .map(|(r, c, _)| measure(&cell_display_text(wb, self.sheet, r, c)))
            .fold(None, |a: Option<f32>, w| Some(a.map_or(w, |a| a.max(w))));
        let Some(widest) = widest else {
            return Ok(());
        };
        let px = (widest.ceil() as u32 + m.padding() + 2).min(MAX_COL_PX);
        self.apply(Command::AutoFitColumns {
            sheet: self.sheet,
            columns: vec![(col, width_for_px(m, px))],
        })
    }

    /// Double click on a row edge: the row returns to the default height (every cell is one line).
    pub fn autofit_row(&mut self, row: u32) -> Result<(), String> {
        let points = self
            .target
            .xlsx()
            .and_then(|wb| wb.sheet(self.sheet))
            .map_or(15.0, |ws| ws.sheet_format().default_row_height);
        self.apply(Command::AutoFitRows {
            sheet: self.sheet,
            rows: vec![(row, points)],
        })
    }

    // ---- clipboard ---------------------------------------------------------------------------

    /// Ctrl+C (`cut` false) or Ctrl+X: TSV and HTML to the clipboard.
    pub fn copy(&mut self, cut: bool) -> Result<(), String> {
        let selected = self.range();
        let (r1, c1, mut r2, mut c2) = selected;
        let wb = self.target.xlsx().ok_or("no workbook")?;
        // Whole rows or columns copy only up to the data.
        match wb.sheet(self.sheet).and_then(|ws| ws.used_range()) {
            Some(u) => {
                r2 = r2.min(u.2.max(r1));
                c2 = c2.min(u.3.max(c1));
            }
            None => {
                r2 = r1;
                c2 = c1;
            }
        }
        let clip = grid_copy(wb, self.sheet, (r1, c1), (r2, c2), MAX_COPY_CELLS)
            .ok_or("the range is too large to copy")?;
        self.clipboard.set(&clip.tsv, &clip.html)?;
        self.copied = Some(Copied {
            origin: clip.origin,
            tsv: clip.tsv,
            rows: clip.rows,
            cut: cut.then_some((r1, c1, r2, c2)),
        });
        self.status.clear();
        Ok(())
    }

    /// Ctrl+V at the top-left of the selection. Pasting over data is destructive: the paste waits
    /// for [`Self::confirm_paste`] (or [`Self::cancel_paste`]) and changes nothing until then.
    pub fn paste(&mut self) -> Result<Paste, String> {
        let Some(text) = self.clipboard.text() else {
            return Ok(Paste::Empty);
        };
        let (r1, c1, ..) = self.range();
        let dest = CellRef {
            sheet: self.sheet,
            row: r1,
            col: c1,
        };
        let (rows, cut) = match &self.copied {
            Some(c) if c.tsv.replace("\r\n", "\n") == text.replace("\r\n", "\n") => {
                (paste_rows(&c.rows, Some(c.origin), dest), c.cut)
            }
            _ => (paste_rows(&parse_tsv(&text), None, dest), None),
        };
        if rows.is_empty() {
            return Ok(Paste::Empty);
        }
        let command = Command::XlsxPaste {
            sheet: self.sheet,
            row: r1,
            col: c1,
            rows,
        };
        if command.effect_in(&self.target) == EffectClass::Destructive {
            self.pending_paste = Some((command, cut));
            return Ok(Paste::NeedsConfirm);
        }
        self.run_paste(command, cut)?;
        Ok(Paste::Done)
    }

    pub fn has_pending_paste(&self) -> bool {
        self.pending_paste.is_some()
    }

    /// The person agreed to overwrite.
    pub fn confirm_paste(&mut self) -> Result<(), String> {
        match self.pending_paste.take() {
            Some((command, cut)) => self.run_paste(command, cut),
            None => Ok(()),
        }
    }

    pub fn cancel_paste(&mut self) {
        self.pending_paste = None;
    }

    fn run_paste(&mut self, command: Command, cut: Option<Rect4>) -> Result<(), String> {
        let Command::XlsxPaste {
            row, col, ref rows, ..
        } = command
        else {
            return Ok(());
        };
        let end = (
            row + rows.len().saturating_sub(1) as u32,
            col + rows
                .iter()
                .map(Vec::len)
                .max()
                .unwrap_or(1)
                .saturating_sub(1) as u32,
        );
        if let Some((r1, c1, r2, c2)) = cut {
            let blank = vec![vec![String::new(); (c2 - c1 + 1) as usize]; (r2 - r1 + 1) as usize];
            self.apply(Command::XlsxPaste {
                sheet: self.sheet,
                row: r1,
                col: c1,
                rows: blank,
            })?;
            self.copied = None;
        }
        self.apply(command)?;
        self.active = (row, col);
        self.anchor = (row, col);
        self.focus = (end.0.min(MAX_ROW), end.1.min(MAX_COL));
        Ok(())
    }

    // ---- fill ----------------------------------------------------------------------------------

    /// Drags the fill handle of the selection to `to`: a number series continues (a constant
    /// step, else a straight-line fit), `Item1` text counts up, formulas shift and other text
    /// repeats. One paste command, so one undo step. Returns whether cells changed.
    pub fn fill_to(&mut self, to: (u32, u32)) -> Result<bool, String> {
        let (r1, c1, r2, c2) = self.range();
        let rows_out = to.0.saturating_sub(r2).max(r1.saturating_sub(to.0));
        let cols_out = to.1.saturating_sub(c2).max(c1.saturating_sub(to.1));
        if rows_out.max(cols_out) == 0 {
            return Ok(false);
        }
        let vertical = rows_out >= cols_out;
        let (block, row0, col0) = if vertical {
            let (first, last) = if to.0 > r2 {
                (r2 + 1, to.0)
            } else {
                (to.0, r1 - 1)
            };
            let mut block = vec![Vec::new(); (last - first + 1) as usize];
            for c in c1..=c2 {
                let src: Vec<String> = (r1..=r2).map(|r| self.source(r, c)).collect();
                let date = self.is_date_cell(r1, c);
                for t in first..=last {
                    block[(t - first) as usize].push(series_value(
                        &src,
                        i64::from(t) - i64::from(r1),
                        true,
                        date,
                    ));
                }
            }
            (block, first, c1)
        } else {
            let (first, last) = if to.1 > c2 {
                (c2 + 1, to.1)
            } else {
                (to.1, c1 - 1)
            };
            let mut block = Vec::new();
            for r in r1..=r2 {
                let src: Vec<String> = (c1..=c2).map(|c| self.source(r, c)).collect();
                let date = self.is_date_cell(r, c1);
                block.push(
                    (first..=last)
                        .map(|t| series_value(&src, i64::from(t) - i64::from(c1), false, date))
                        .collect(),
                );
            }
            (block, r1, first)
        };
        self.apply(Command::XlsxPaste {
            sheet: self.sheet,
            row: row0,
            col: col0,
            rows: block,
        })?;
        if vertical {
            self.anchor = (r1.min(to.0), c1);
            self.focus = (r2.max(to.0), c2);
        } else {
            self.anchor = (r1, c1.min(to.1));
            self.focus = (r2, c2.max(to.1));
        }
        Ok(true)
    }

    /// Whether the cell holds a number that its format shows as a date or time (so a fill
    /// counts days instead of copying).
    fn is_date_cell(&self, row: u32, col: u32) -> bool {
        let src = self.source(row, col);
        let shown = self.display(row, col);
        src.trim().parse::<f64>().is_ok() && looks_like_date(&shown)
    }

    /// Ctrl+D (`down`) or Ctrl+R: copies the first row (column) of the selection into the rest;
    /// for a single row (column) it copies from the cell above (left). Formulas shift. One paste
    /// command, so one undo step. Returns whether cells changed.
    pub fn fill_copy(&mut self, down: bool) -> Result<bool, String> {
        let (r1, c1, r2, c2) = self.range();
        let (sr1, sc1) = if down {
            if r2 > r1 {
                (r1, c1)
            } else if r1 > 0 {
                (r1 - 1, c1)
            } else {
                return Ok(false);
            }
        } else if c2 > c1 {
            (r1, c1)
        } else if c1 > 0 {
            (r1, c1 - 1)
        } else {
            return Ok(false);
        };
        let (dest_r, dest_c) = if down {
            (if r2 > r1 { r1 + 1 } else { r1 }, c1)
        } else {
            (r1, if c2 > c1 { c1 + 1 } else { c1 })
        };
        let mut block: Vec<Vec<String>> = Vec::new();
        for r in dest_r..=r2 {
            let mut row = Vec::new();
            for c in dest_c..=c2 {
                let (src_r, src_c) = if down { (sr1, c) } else { (r, sc1) };
                let (dr, dc) = (
                    i64::from(r) - i64::from(src_r),
                    i64::from(c) - i64::from(src_c),
                );
                let text = self.source(src_r, src_c);
                row.push(if text.starts_with('=') {
                    open365_xlsx::shift_formula(&text, dr, dc)
                } else {
                    text
                });
            }
            block.push(row);
        }
        self.apply(Command::XlsxPaste {
            sheet: self.sheet,
            row: dest_r,
            col: dest_c,
            rows: block,
        })?;
        Ok(true)
    }

    // ---- saving ----------------------------------------------------------------------------------

    /// Asks where to save (the dialog), then saves there through the command layer; the workbook
    /// takes the new path. `Ok(None)` when the dialog was cancelled.
    pub fn save_as(&mut self) -> Result<Option<PathBuf>, String> {
        let name = self
            .path()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "Book1.xlsx".to_owned());
        let Some(mut path) = (self.chooser)(&name) else {
            return Ok(None);
        };
        if path.extension().is_none() {
            path.set_extension("xlsx");
        }
        self.apply(Command::Save { path: path.clone() })?;
        Ok(Some(path))
    }
}

/// The stored width (characters) whose pixel width is `px`.
fn width_for_px(m: GridMetrics, px: u32) -> f64 {
    if px == 0 {
        return 0.0;
    }
    let mdw = f64::from(m.mdw);
    let mut width = (f64::from(px) / mdw * 256.0).floor() / 256.0;
    for _ in 0..8 {
        if m.col_px(width) >= px {
            break;
        }
        width += 1.0 / 256.0;
    }
    width
}

/// The row height in points that is `px` pixels tall.
fn points_for_px(m: GridMetrics, px: u32) -> f64 {
    let mut points = m.px_to_pt(px);
    for _ in 0..8 {
        if m.row_px(points) >= px {
            break;
        }
        points += 0.01;
    }
    points
}

fn number_text(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let t = format!("{v:.10}");
        t.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

/// `Item12` as `("Item", 12.0)`; a plain number has the prefix `""`.
fn split_number(s: &str) -> Option<(String, f64)> {
    let t = s.trim();
    if let Ok(v) = t.parse::<f64>()
        && v.is_finite()
    {
        return Some((String::new(), v));
    }
    let end = t.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    if end == t.len() || end == 0 {
        return None;
    }
    let n: f64 = t[end..].parse().ok()?;
    Some((t[..end].to_owned(), n))
}

/// The value at source index `i` (outside `0..src.len()` it extends the series).
fn series_value(src: &[String], i: i64, vertical: bool, date: bool) -> String {
    let len = src.len() as i64;
    let k = i.rem_euclid(len) as usize;
    let offset = i - k as i64;
    let item = &src[k];
    if item.starts_with('=') {
        return if vertical {
            open365_xlsx::shift_formula(item, offset, 0)
        } else {
            open365_xlsx::shift_formula(item, 0, offset)
        };
    }
    if let Some(v) = name_series(src, i) {
        return v;
    }
    let parts: Option<Vec<(String, f64)>> = src.iter().map(|s| split_number(s)).collect();
    if let Some(parts) = parts
        && parts.iter().all(|p| p.0 == parts[0].0)
    {
        let prefix = &parts[0].0;
        let ys: Vec<f64> = parts.iter().map(|p| p.1).collect();
        let v = if ys.len() == 1 {
            // One number copies; one `Item1` counts up.
            if prefix.is_empty() && !date {
                ys[0]
            } else {
                ys[0] + i as f64
            }
        } else {
            let step = ys[1] - ys[0];
            if ys.windows(2).all(|w| (w[1] - w[0] - step).abs() < 1e-9) {
                ys[0] + step * i as f64
            } else {
                // Least squares line through the source points.
                let n = ys.len() as f64;
                let mx = (n - 1.0) / 2.0;
                let my = ys.iter().sum::<f64>() / n;
                let sxx: f64 = (0..ys.len()).map(|x| (x as f64 - mx).powi(2)).sum();
                let sxy: f64 = ys
                    .iter()
                    .enumerate()
                    .map(|(x, y)| (x as f64 - mx) * (y - my))
                    .sum();
                my + sxy / sxx * (i as f64 - mx)
            }
        };
        return format!("{prefix}{}", number_text(v));
    }
    item.clone()
}

const DAYS_LONG: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
const DAYS_SHORT: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS_LONG: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const MONTHS_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Weekday and month names continue as a cycle (`Mon, Tue` gives `Wed`; one name steps by one).
/// The case of the first item is kept.
fn name_series(src: &[String], i: i64) -> Option<String> {
    let lists: [&[&str]; 4] = [&DAYS_LONG, &DAYS_SHORT, &MONTHS_LONG, &MONTHS_SHORT];
    for list in lists {
        let pos = |s: &str| list.iter().position(|n| n.eq_ignore_ascii_case(s.trim()));
        let idx: Option<Vec<usize>> = src.iter().map(|s| pos(s)).collect();
        let Some(idx) = idx else { continue };
        let n = list.len() as i64;
        let step = if idx.len() > 1 {
            (idx[1] as i64 - idx[0] as i64).rem_euclid(n)
        } else {
            1
        };
        let at = (idx[0] as i64 + step * i).rem_euclid(n) as usize;
        let name = list[at];
        let first = src[0].trim();
        return Some(
            if first.chars().all(|c| !c.is_lowercase()) && first.len() > 1 {
                name.to_uppercase()
            } else if first.chars().next().is_some_and(char::is_lowercase) {
                name.to_lowercase()
            } else {
                name.to_owned()
            },
        );
    }
    None
}

/// Whether text a number format produced reads as a date or time (`2024-03-01`, `3/1/2024`,
/// `1-Mar-24`, `12:30`), as opposed to a plain, grouped or percent number.
fn looks_like_date(shown: &str) -> bool {
    let t = shown.trim();
    let seps = t.chars().filter(|c| matches!(c, '/' | '-')).count();
    let has_month = MONTHS_SHORT
        .iter()
        .any(|m| t.to_ascii_lowercase().contains(&m.to_ascii_lowercase()));
    let digits = t.chars().filter(char::is_ascii_digit).count();
    (seps >= 2 && digits >= 3) || (has_month && digits >= 1) || (t.contains(':') && digits >= 3)
}

/// What lies under a device-pixel point of the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    /// The select-all box where the headers meet.
    Corner,
    ColHeader(u32),
    /// Within a few pixels of the right edge of this column's header.
    ColEdge(u32),
    RowHeader(u32),
    RowEdge(u32),
    Cell(u32, u32),
    /// The fill handle of the selection.
    FillHandle,
    Outside,
}

/// Column widths and row heights of a sheet in logical pixels (96 dpi) with the rows and
/// columns its file freezes; dereferences to the widths and heights.
#[derive(Debug, Clone)]
pub struct GridGeometry {
    sheet: Arc<SheetGeometry>,
    /// Frozen rows and columns.
    pub frozen: (u32, u32),
}

impl std::ops::Deref for GridGeometry {
    type Target = SheetGeometry;

    fn deref(&self) -> &SheetGeometry {
        &self.sheet
    }
}

/// Device-pixel geometry of the sheet grid with the sheet's own column widths and row heights:
/// what painting and hit testing share.
#[derive(Debug, Clone)]
pub struct Geo {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub row_header_w: f32,
    pub col_header_h: f32,
    pub scale: f32,
    pub top_left: (u32, u32),
    geo: GridGeometry,
}

impl Geo {
    /// `area` is `(x, y, right, bottom)` and `header` the header size, both in device pixels.
    pub fn new(
        area: (f32, f32, f32, f32),
        header: (f32, f32),
        scale: f32,
        geo: GridGeometry,
        top_left: (u32, u32),
    ) -> Self {
        Self {
            x0: area.0,
            y0: area.1,
            x1: area.2,
            y1: area.3,
            row_header_w: header.0,
            col_header_h: header.1,
            scale,
            top_left,
            geo,
        }
    }

    fn dev(&self, logical: i64) -> f32 {
        (logical as f64 * f64::from(self.scale)).round() as f32
    }

    /// Rows and columns frozen at the top and left.
    pub fn frozen(&self) -> (u32, u32) {
        self.geo.frozen
    }

    /// Width in device pixels of the frozen columns (0 without a freeze).
    pub fn frozen_w(&self) -> f32 {
        let fc = self.geo.frozen.1;
        self.dev(self.geo.col_x(fc + 1) as i64 - self.geo.col_x(1) as i64)
    }

    /// Height in device pixels of the frozen rows.
    pub fn frozen_h(&self) -> f32 {
        let fr = self.geo.frozen.0;
        self.dev(self.geo.row_y(fr + 1) as i64 - self.geo.row_y(1) as i64)
    }

    /// Left edge of the scrolling area (right of the frozen columns).
    pub fn scroll_x0(&self) -> f32 {
        self.x0 + self.row_header_w + self.frozen_w()
    }

    /// Top edge of the scrolling area (below the frozen rows).
    pub fn scroll_y0(&self) -> f32 {
        self.y0 + self.col_header_h + self.frozen_h()
    }

    /// Left edge of `col`: a frozen column sits at its place in the sheet, any other follows
    /// the scroll position (and slides under the frozen columns).
    pub fn col_left(&self, col: u32) -> f32 {
        if col <= self.geo.frozen.1 {
            let d = self.geo.col_x(col) as i64 - self.geo.col_x(1) as i64;
            self.x0 + self.row_header_w + self.dev(d)
        } else {
            self.scrolled_left(col)
        }
    }

    fn scrolled_left(&self, col: u32) -> f32 {
        let d = self.geo.col_x(col) as i64 - self.geo.col_x(self.top_left.1) as i64;
        self.scroll_x0() + self.dev(d)
    }

    /// Right edge of `col` (the left edge of the next column, except at the frozen split).
    pub fn col_right(&self, col: u32) -> f32 {
        if col < self.geo.frozen.1 {
            self.col_left(col + 1)
        } else if col == self.geo.frozen.1 && col > 0 {
            self.scroll_x0()
        } else {
            self.scrolled_left(col + 1)
        }
    }

    /// Top edge of `row`.
    pub fn row_top(&self, row: u32) -> f32 {
        if row <= self.geo.frozen.0 {
            let d = self.geo.row_y(row) as i64 - self.geo.row_y(1) as i64;
            self.y0 + self.col_header_h + self.dev(d)
        } else {
            self.scrolled_top(row)
        }
    }

    fn scrolled_top(&self, row: u32) -> f32 {
        let d = self.geo.row_y(row) as i64 - self.geo.row_y(self.top_left.0) as i64;
        self.scroll_y0() + self.dev(d)
    }

    /// Bottom edge of `row`.
    pub fn row_bottom(&self, row: u32) -> f32 {
        if row < self.geo.frozen.0 {
            self.row_top(row + 1)
        } else if row == self.geo.frozen.0 && row > 0 {
            self.scroll_y0()
        } else {
            self.scrolled_top(row + 1)
        }
    }

    /// Smallest x a cell of `col` is drawn at: scrolled columns are clipped at the frozen split.
    pub fn min_x(&self, col: u32) -> f32 {
        if col <= self.geo.frozen.1 {
            self.x0 + self.row_header_w
        } else {
            self.scroll_x0()
        }
    }

    /// Smallest y a cell of `row` is drawn at.
    pub fn min_y(&self, row: u32) -> f32 {
        if row <= self.geo.frozen.0 {
            self.y0 + self.col_header_h
        } else {
            self.scroll_y0()
        }
    }

    /// The columns to draw: the frozen ones, then the scrolling ones that reach into view up
    /// to the right edge (one past, so a partly visible column is drawn).
    pub fn drawn_cols(&self) -> Vec<u32> {
        let mut cols: Vec<u32> = (1..=self.geo.frozen.1).collect();
        let mut c = self.top_left.1.max(self.geo.frozen.1 + 1);
        while cols.len() < 5000 && self.scrolled_left(c) < self.x1 && c <= MAX_COL {
            if self.scrolled_left(c + 1) > self.scroll_x0() {
                cols.push(c);
            }
            c += 1;
        }
        cols
    }

    /// The rows to draw, as [`Geo::drawn_cols`] does for columns.
    pub fn drawn_rows(&self) -> Vec<u32> {
        let mut rows: Vec<u32> = (1..=self.geo.frozen.0).collect();
        let mut r = self.top_left.0.max(self.geo.frozen.0 + 1);
        while rows.len() < 5000 && self.scrolled_top(r) < self.y1 && r <= MAX_ROW {
            if self.scrolled_top(r + 1) > self.scroll_y0() {
                rows.push(r);
            }
            r += 1;
        }
        rows
    }

    /// `(left, top, right, bottom)` of a cell; the edges include the grid line.
    pub fn cell(&self, row: u32, col: u32) -> (f32, f32, f32, f32) {
        (
            self.col_left(col),
            self.row_top(row),
            self.col_right(col),
            self.row_bottom(row),
        )
    }

    /// Rectangle of a range `(r1, c1, r2, c2)`; the caller clips it to the body.
    pub fn range_rect(&self, r: Rect4) -> (f32, f32, f32, f32) {
        (
            self.col_left(r.1),
            self.row_top(r.0),
            self.col_right(r.3),
            self.row_bottom(r.2),
        )
    }

    /// Whole scrolling rows and columns that fit beside the frozen panes (at least one each).
    pub fn visible(&self) -> (u32, u32) {
        let body_w = self.x1 - self.scroll_x0();
        let body_h = self.y1 - self.scroll_y0();
        let (r0, c0) = (
            self.scrolled_top(self.top_left.0),
            self.scrolled_left(self.top_left.1),
        );
        let (mut rows, mut cols) = (0u32, 0u32);
        while rows < 5000 && self.scrolled_top(self.top_left.0 + rows + 1) - r0 <= body_h {
            rows += 1;
        }
        while cols < 5000 && self.scrolled_left(self.top_left.1 + cols + 1) - c0 <= body_w {
            cols += 1;
        }
        (rows.max(1), cols.max(1))
    }

    fn col_at(&self, x: f32) -> u32 {
        let rel = x - self.x0 - self.row_header_w;
        let fw = self.frozen_w();
        let px = if rel < fw {
            self.geo.col_x(1) as f64 + f64::from(rel) / f64::from(self.scale)
        } else {
            self.geo.col_x(self.top_left.1) as f64 + f64::from(rel - fw) / f64::from(self.scale)
        };
        self.geo.col_at(px.max(0.0) as u64, MAX_COL)
    }

    fn row_at(&self, y: f32) -> u32 {
        let rel = y - self.y0 - self.col_header_h;
        let fh = self.frozen_h();
        let px = if rel < fh {
            self.geo.row_y(1) as f64 + f64::from(rel) / f64::from(self.scale)
        } else {
            self.geo.row_y(self.top_left.0) as f64 + f64::from(rel - fh) / f64::from(self.scale)
        };
        self.geo.row_at(px.max(0.0) as u64, MAX_ROW)
    }

    /// The cell under `(x, y)`, or `None` over the headers or outside the grid.
    pub fn hit(&self, x: f32, y: f32) -> Option<(u32, u32)> {
        match self.zone(x, y, None) {
            Zone::Cell(r, c) => Some((r, c)),
            _ => None,
        }
    }

    /// The nearest cell to `(x, y)`: a drag past the edge stays on the edge cell.
    pub fn hit_clamped(&self, x: f32, y: f32) -> (u32, u32) {
        let lo_x = self.x0 + self.row_header_w;
        let lo_y = self.y0 + self.col_header_h;
        let x = x.clamp(lo_x, (self.x1 - 1.0).max(lo_x));
        let y = y.clamp(lo_y, (self.y1 - 1.0).max(lo_y));
        (self.row_at(y), self.col_at(x))
    }

    /// Classifies a point. `selection` enables the fill handle at its bottom-right corner.
    pub fn zone(&self, x: f32, y: f32, selection: Option<Rect4>) -> Zone {
        if x < self.x0 || y < self.y0 || x >= self.x1 || y >= self.y1 {
            return Zone::Outside;
        }
        let grab = (4.0 * self.scale / 1.25).max(3.0);
        let in_col_hdr = y < self.y0 + self.col_header_h;
        let in_row_hdr = x < self.x0 + self.row_header_w;
        if in_col_hdr && in_row_hdr {
            return Zone::Corner;
        }
        if in_col_hdr {
            let c = self.col_at(x);
            if (x - self.col_right(c)).abs() <= grab {
                return Zone::ColEdge(c);
            }
            if c > 1 && (x - self.col_left(c)).abs() <= grab {
                return Zone::ColEdge(c - 1);
            }
            return Zone::ColHeader(c);
        }
        if in_row_hdr {
            let r = self.row_at(y);
            if (y - self.row_bottom(r)).abs() <= grab {
                return Zone::RowEdge(r);
            }
            if r > 1 && (y - self.row_top(r)).abs() <= grab {
                return Zone::RowEdge(r - 1);
            }
            return Zone::RowHeader(r);
        }
        if let Some(sel) = selection {
            let (_, _, right, bottom) = self.range_rect(sel);
            if (x - right).abs() <= grab && (y - bottom).abs() <= grab {
                return Zone::FillHandle;
            }
        }
        Zone::Cell(self.row_at(y), self.col_at(x))
    }
}

/// `A1`, `c12` or `AA100` as `(row, col)`.
pub fn parse_a1(s: &str) -> Option<(u32, u32)> {
    let s = s.trim().replace('$', "");
    let split = s.find(|c: char| c.is_ascii_digit())?;
    let (letters, digits) = s.split_at(split);
    if letters.is_empty() || !letters.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let col = letters
        .to_ascii_uppercase()
        .bytes()
        .fold(0u32, |a, b| a * 26 + u32::from(b - b'A' + 1));
    let row: u32 = digits.parse().ok()?;
    ((1..=MAX_ROW).contains(&row) && (1..=MAX_COL).contains(&col)).then_some((row, col))
}

/// `(1, 1)` as `A1`.
pub fn a1(row: u32, col: u32) -> String {
    let mut n = col;
    let mut letters = String::new();
    while n > 0 {
        n -= 1;
        letters.insert(0, char::from(b'A' + (n % 26) as u8));
        n /= 26;
    }
    format!("{letters}{row}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a1_names_round_trip() {
        for (name, rc) in [
            ("A1", (1, 1)),
            ("Z9", (9, 26)),
            ("AA100", (100, 27)),
            ("XFD1048576", (1_048_576, 16_384)),
        ] {
            assert_eq!(parse_a1(name), Some(rc), "{name}");
            assert_eq!(a1(rc.0, rc.1), name);
        }
        assert_eq!(parse_a1("c12"), Some((12, 3)));
        assert_eq!(parse_a1("XFE1"), None);
        assert_eq!(parse_a1("12"), None);
        assert_eq!(parse_a1("A0"), None);
    }

    #[test]
    fn typed_values_and_a_formula_recalculate_through_the_command_layer() {
        let mut m = SheetModel::blank();
        m.enter(1, 1, "2").unwrap();
        m.enter(1, 2, "3").unwrap();
        m.enter(1, 3, "=A1+B1").unwrap();
        assert_eq!(m.display(1, 3), "5");
        assert_eq!(m.source(1, 3), "=A1+B1");
        m.enter(1, 1, "10").unwrap();
        assert_eq!(m.display(1, 3), "13");
        m.clear(1, 1).unwrap();
        assert_eq!(m.display(1, 3), "3");
        assert!(m.undo());
        assert_eq!(m.display(1, 3), "13");
    }
}
