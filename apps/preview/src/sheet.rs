//! The Sheets preview's document: a workbook held by the command layer's [`Target`]. Every cell
//! edit is a [`Command`] (so it is audited, undoable and recalculated by the calc engine); this
//! type only adds the active cell, the scroll origin and the text a cell shows.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use open365_commands::{
    CellEntry, CellRef, Command, EffectClass, RangeRef, Target, cell_display_text,
    cell_source_text, grid_copy, parse_tsv, paste_rows,
};
use open365_xlsx::{CellValue, GridMetrics, SheetGeometry};

#[path = "shell/barfmt.rs"]
pub mod barfmt;
#[path = "../../sheets/src/blank.rs"]
mod blank;
#[path = "sheet_ext/mod.rs"]
pub mod sheet_ext;

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

/// Which neighbours move when cells are inserted or deleted (the Insert and Delete dialogs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellShift {
    Right,
    Down,
    Left,
    Up,
}

/// What a Paste Special puts in the destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteWhat {
    All,
    Formulas,
    Values,
    Formats,
}

/// The choices of the Paste Special dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasteSpecialOpts {
    pub what: PasteWhat,
    /// Blank source cells leave the destination as it is.
    pub skip_blanks: bool,
    pub transpose: bool,
}

/// A modal dialog over the grid. The choices live here so the dialog survives grid rebuilds.
#[derive(Debug, Clone, PartialEq)]
pub enum Dialog {
    /// Insert (`insert`) or Delete: `choice` 0 and 1 are the two shift directions (right/down,
    /// left/up), 2 the entire row and 3 the entire column.
    InsertDelete {
        insert: bool,
        choice: usize,
    },
    PasteSpecial(PasteSpecialOpts),
}

/// Popup and dialog state of the grid, kept with the model because the shell struct is owned
/// elsewhere. The widget nodes are rebuilt with the grid; `nodes` maps them to action names.
#[derive(Debug, Default)]
pub struct GridUi {
    pub dialog: Option<Dialog>,
    pub nodes: Vec<(open365_ui_core::NodeId, String)>,
    /// What the open context menu was opened for.
    pub menu_for: Option<MenuFor>,
    /// The last Insert and Delete dialog choices, which Excel offers again.
    pub last_shift: [Option<usize>; 2],
    /// What the open pop-up menu of the grid (a filter drop-down or a validation list) is for.
    pub pop: Option<GridPop>,
    /// The marks the grid draws, cached for one document revision (see [`SheetModel::grid_marks`]).
    pub marks: RefCell<Option<Rc<GridMarks>>>,
}

/// The target of a pop-up menu the grid opened from a button in a cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GridPop {
    /// The AutoFilter drop-down of a column; `anchor` is the logical point it opened at.
    Filter { col: u32, anchor: (i32, i32) },
    /// The in-cell validation list of a cell, with its items.
    Validation {
        row: u32,
        col: u32,
        items: Vec<String>,
    },
}

/// One outline group: the detail rows (or columns) `first..=last` at `level`; the summary is the
/// row (column) right after `last`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutlineGroup {
    pub level: u32,
    pub first: u32,
    pub last: u32,
    /// The detail is hidden (the button shows `+`).
    pub collapsed: bool,
}

/// What the grid draws besides cell text: AutoFilter buttons, outline groups, note and comment
/// markers. Built once per document revision and sheet.
#[derive(Debug, Clone, Default)]
pub struct GridMarks {
    /// The AutoFilter range and its filtered columns.
    pub filter: Option<(Rect4, Vec<u32>)>,
    pub row_groups: Vec<OutlineGroup>,
    pub col_groups: Vec<OutlineGroup>,
    pub row_levels: u32,
    pub col_levels: u32,
    pub notes: Vec<open365_commands::review_ops::Note>,
    pub threads: Vec<open365_commands::review_ops::Thread>,
    /// Last row and column of the used range (1 when the sheet is empty).
    pub used: (u32, u32),
    key: (u64, usize),
}

impl GridMarks {
    /// The note on a cell.
    pub fn note_at(&self, row: u32, col: u32) -> Option<&open365_commands::review_ops::Note> {
        self.notes.iter().find(|n| (n.row, n.col) == (row, col))
    }

    /// The comment thread on a cell.
    pub fn thread_at(&self, row: u32, col: u32) -> Option<&open365_commands::review_ops::Thread> {
        self.threads.iter().find(|t| (t.row, t.col) == (row, col))
    }
}

/// Groups of consecutive indices at or above each level, from `(index, level)` pairs sorted by
/// index. `hidden` says whether an index is hidden (a collapsed group).
fn outline_groups(levels: &[(u32, u32)], hidden: impl Fn(u32) -> bool) -> Vec<OutlineGroup> {
    let max = levels.iter().map(|l| l.1).max().unwrap_or(0).min(8);
    let mut out = Vec::new();
    for level in 1..=max {
        let mut run: Option<(u32, u32)> = None;
        for &(i, _) in levels.iter().filter(|l| l.1 >= level) {
            run = match run {
                Some((a, b)) if b + 1 == i => Some((a, i)),
                Some((a, b)) => {
                    out.push(OutlineGroup {
                        level,
                        first: a,
                        last: b,
                        collapsed: hidden(b),
                    });
                    Some((i, i))
                }
                None => Some((i, i)),
            };
        }
        if let Some((a, b)) = run {
            out.push(OutlineGroup {
                level,
                first: a,
                last: b,
                collapsed: hidden(b),
            });
        }
    }
    out
}

impl SheetModel {
    /// The revision of the document (it grows with every edit, undo and redo).
    pub fn revision(&self) -> u64 {
        self.target.revision()
    }

    /// The marks of the active sheet for the grid, rebuilt only when the document changed.
    pub fn grid_marks(&self) -> Rc<GridMarks> {
        let key = (self.target.revision(), self.sheet);
        if let Some(m) = self.ui.marks.borrow().as_ref()
            && m.key == key
        {
            return Rc::clone(m);
        }
        let geo = self.geometry();
        let mut marks = GridMarks {
            key,
            used: (1, 1),
            ..GridMarks::default()
        };
        if let Some(wb) = self.target.xlsx()
            && let Some(ws) = wb.sheet(self.sheet)
        {
            let mut rows: Vec<(u32, u32)> = ws
                .rows()
                .iter()
                .filter_map(|r| {
                    let level = r
                        .attrs()
                        .iter()
                        .find(|a| a.name == "outlineLevel")
                        .and_then(|a| a.value.parse::<u32>().ok())?;
                    (level > 0).then_some((r.r, level))
                })
                .collect();
            rows.sort_unstable();
            marks.row_levels = rows.iter().map(|r| r.1).max().unwrap_or(0).min(8);
            marks.row_groups = outline_groups(&rows, |r| geo.row_hidden(r));
            let xml = String::from_utf8_lossy(&ws.to_bytes()).into_owned();
            if let Some(start) = xml.find("<cols>") {
                let end = xml[start..]
                    .find("</cols>")
                    .map_or(xml.len(), |e| start + e);
                let attr = |tag: &str, name: &str| -> Option<u32> {
                    let key = format!(" {name}=\"");
                    let at = tag.find(&key)? + key.len();
                    tag[at..].split('"').next()?.parse().ok()
                };
                let mut cols: Vec<(u32, u32)> = Vec::new();
                for tag in xml[start..end].split("<col ").skip(1) {
                    let tag = format!(" {tag}");
                    let (Some(min), Some(max)) = (attr(&tag, "min"), attr(&tag, "max")) else {
                        continue;
                    };
                    let level = attr(&tag, "outlineLevel").unwrap_or(0);
                    if level > 0 {
                        cols.extend((min..=max.min(min + 2000)).map(|c| (c, level)));
                    }
                }
                cols.sort_unstable();
                marks.col_levels = cols.iter().map(|c| c.1).max().unwrap_or(0).min(8);
                marks.col_groups = outline_groups(&cols, |c| geo.col_hidden(c));
            }
            if let Some(u) = ws.used_range() {
                marks.used = (u.2.max(1), u.3.max(1));
            }
            marks.filter = ws.auto_filter().map(|f| {
                let cols = f
                    .columns
                    .iter()
                    .filter(|c| c.criteria.is_some())
                    .map(|c| f.range.1 + c.col_id)
                    .collect();
                (f.range, cols)
            });
        }
        marks.notes = self.review_notes();
        marks.threads = self.review_comments();
        let marks = Rc::new(marks);
        *self.ui.marks.borrow_mut() = Some(Rc::clone(&marks));
        marks
    }
}

/// The target of a context menu: it decides the items and what they act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuFor {
    Cell,
    Row(u32),
    Col(u32),
    AutoFill,
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
    pending_paste: Option<(Command, Option<Rect4>, Option<Rect4>)>,
    /// The column where a run of Tab presses began: Enter returns there on the next row.
    pub tab_start: Option<u32>,
    /// A Ctrl+Space or Shift+Space just selected: its text event is not typed.
    pub swallow_space: bool,
    /// Ctrl was held at fill-handle release (set by the host: the UI core reports no modifiers there).
    pub fill_ctrl: bool,
    /// User-defined AutoFill lists (Excel's Custom Lists); the day and month names are built in.
    pub custom_lists: Vec<Vec<String>>,
    /// The last fill, so the AutoFill Options button can redo it another way.
    last_fill: Option<(Rect4, (u32, u32), usize)>,
    /// The range the last fill produced and how it filled: the floating AutoFill Options button
    /// stays until the next edit, undo or selection change.
    autofill: Option<(Rect4, FillMode)>,
    /// Dialogs and popups the grid shows (see [`GridUi`]).
    pub ui: GridUi,
    chooser: SaveChooser,
    locale: Arc<open365_calc::FormatLocale>,
    /// View tab state: view mode, zoom, gridlines, headings, freeze panes (not in the file yet).
    pub ext_view: sheet_ext::view::ViewState,
    /// Page Layout tab state: margins, orientation, paper size, print area, breaks, scaling.
    pub ext_page: sheet_ext::page_layout::PageState,
    /// File tab state: the Open dialog.
    pub ext_file: sheet_ext::file::FileState,
    /// Home tab state: the Format Painter's picked-up cell and the last border and colours used.
    pub ext_home: sheet_ext::home::HomeState,
    /// Formulas tab state: Show Formulas, audit arrows, recent functions, a pending dialog.
    pub ext_formulas: sheet_ext::formulas::FormulasState,
    /// Data tab state: the dialog the shell should open.
    pub ext_data: sheet_ext::data::DataState,
    /// Review tab state: the dialog to open, ignored spelling words, the last spelling run.
    pub ext_review: sheet_ext::review::ReviewState,
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
            tab_start: None,
            swallow_space: false,
            fill_ctrl: false,
            custom_lists: Vec::new(),
            last_fill: None,
            autofill: None,
            ui: GridUi::default(),
            chooser: system_save_chooser(),
            locale: barfmt::default_locale(),
            ext_view: sheet_ext::view::ViewState::default(),
            ext_page: sheet_ext::page_layout::PageState::default(),
            ext_file: sheet_ext::file::FileState::default(),
            ext_home: sheet_ext::home::HomeState::default(),
            ext_formulas: sheet_ext::formulas::FormulasState::default(),
            ext_data: sheet_ext::data::DataState::default(),
            ext_review: sheet_ext::review::ReviewState::default(),
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
        self.autofill = None;
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
        self.autofill = None;
        self.target.undo().is_ok()
    }

    pub fn redo(&mut self) -> bool {
        self.autofill = None;
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

/// How a fill-handle drag fills (the drag itself, with Ctrl, or an AutoFill Options choice).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillMode {
    /// A plain drag: series for dates, names and `Item1`; a lone number copies.
    Auto,
    /// A drag with Ctrl held: a lone number counts up, everything else copies.
    Ctrl,
    /// Copy Cells.
    Copy,
    /// Fill Series: a lone number counts up by one.
    Series,
    /// Fill Formatting Only.
    FormatsOnly,
    /// Fill Without Formatting.
    WithoutFormatting,
}

/// What Paste Special puts in the destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteKind {
    /// Values only: formulas become their results.
    Values,
    /// Formulas only, no formats.
    Formulas,
    /// Formats only.
    Formats,
    /// Rows become columns and columns rows.
    Transpose,
}

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

/// How far the dashes of the copy outline have marched at animation frame `frame` (one device
/// pixel per 60 ms step, repeating every dash period). Paint the dashes starting `phase` pixels
/// before the corner.
pub fn ants_phase(frame: u64, dash: f32) -> f32 {
    let period = (dash * 2.0).round().max(1.0) as u64;
    (frame % period) as f32
}

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
        self.autofill = None;
        self.tab_start = None;
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
        self.autofill = None;
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
        self.autofill = None;
        self.anchor = (1, 1);
        self.focus = (MAX_ROW, MAX_COL);
    }

    /// Click or drag on column headers: whole columns `a..=b`. The active cell moves to the
    /// first visible row of column `a` unless `keep_active`.
    pub fn select_cols(&mut self, a: u32, b: u32, keep_active: bool) {
        self.autofill = None;
        if !keep_active {
            self.active = (self.top_left.0, a);
        }
        self.anchor = (1, a);
        self.focus = (MAX_ROW, b);
    }

    pub fn select_rows(&mut self, a: u32, b: u32, keep_active: bool) {
        self.autofill = None;
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
            return self.ext_view.apply_freeze(self.sheet, geo.clone());
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
        self.ext_view.apply_freeze(self.sheet, geo)
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
        let (rows, cut, fmt_src) = match &self.copied {
            Some(c) if c.tsv.replace("\r\n", "\n") == text.replace("\r\n", "\n") => (
                paste_rows(&c.rows, Some(c.origin), dest),
                c.cut,
                Some(c.rect()),
            ),
            _ => (paste_rows(&parse_tsv(&text), None, dest), None, None),
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
            self.pending_paste = Some((command, cut, fmt_src));
            return Ok(Paste::NeedsConfirm);
        }
        self.run_paste(command, cut, fmt_src)?;
        Ok(Paste::Done)
    }

    pub fn has_pending_paste(&self) -> bool {
        self.pending_paste.is_some()
    }

    /// The person agreed to overwrite.
    pub fn confirm_paste(&mut self) -> Result<(), String> {
        match self.pending_paste.take() {
            Some((command, cut, fmt)) => self.run_paste(command, cut, fmt),
            None => Ok(()),
        }
    }

    pub fn cancel_paste(&mut self) {
        self.pending_paste = None;
    }

    fn run_paste(
        &mut self,
        command: Command,
        cut: Option<Rect4>,
        fmt_src: Option<Rect4>,
    ) -> Result<(), String> {
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
        if let Some(src) = fmt_src {
            // Formats travel with the cells; a cut leaves its source unformatted. They go first
            // so one undo takes back the pasted values.
            self.copy_formats(src, (row, col, end.0, end.1), cut.is_some());
        }
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
        self.fill_with(to, FillMode::Auto)
    }

    /// [`Self::fill_to`] with an explicit [`FillMode`] (Ctrl drag, AutoFill Options).
    pub fn fill_with(&mut self, to: (u32, u32), mode: FillMode) -> Result<bool, String> {
        let (r1, c1, r2, c2) = self.range();
        let rows_out = to.0.saturating_sub(r2).max(r1.saturating_sub(to.0));
        let cols_out = to.1.saturating_sub(c2).max(c1.saturating_sub(to.1));
        if rows_out.max(cols_out) == 0 {
            return Ok(false);
        }
        let vertical = rows_out >= cols_out;
        let custom = self.custom_lists.clone();
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
                        mode,
                        &custom,
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
                        .map(|t| {
                            series_value(
                                &src,
                                i64::from(t) - i64::from(c1),
                                false,
                                date,
                                mode,
                                &custom,
                            )
                        })
                        .collect(),
                );
            }
            (block, r1, first)
        };
        let (br2, bc2) = (
            row0 + block.len() as u32 - 1,
            col0 + block.first().map_or(1, Vec::len) as u32 - 1,
        );
        // Formats first: one undo then takes back the values, the step the person sees.
        let mut steps = 0;
        if mode != FillMode::WithoutFormatting {
            let copied = self.apply(Command::CopyFormat {
                sheet: self.sheet,
                from: RangeRef {
                    first_row: r1,
                    first_col: c1,
                    last_row: r2,
                    last_col: c2,
                },
                to: RangeRef {
                    first_row: row0,
                    first_col: col0,
                    last_row: br2,
                    last_col: bc2,
                },
            });
            steps += usize::from(copied.is_ok());
        }
        if mode != FillMode::FormatsOnly {
            steps += 1;
            self.apply(Command::XlsxPaste {
                sheet: self.sheet,
                row: row0,
                col: col0,
                rows: block,
            })?;
        }
        self.last_fill = Some(((r1, c1, r2, c2), to, steps));
        if vertical {
            self.anchor = (r1.min(to.0), c1);
            self.focus = (r2.max(to.0), c2);
        } else {
            self.anchor = (r1, c1.min(to.1));
            self.focus = (r2, c2.max(to.1));
        }
        self.autofill = Some((self.range(), mode));
        Ok(true)
    }

    /// The AutoFill Options button: undoes the last fill and does it again as `mode`.
    pub fn refill(&mut self, mode: FillMode) -> Result<bool, String> {
        let Some(((r1, c1, r2, c2), to, steps)) = self.last_fill.take() else {
            return Ok(false);
        };
        for _ in 0..steps {
            self.undo();
        }
        self.anchor = (r1, c1);
        self.focus = (r2, c2);
        self.active = (r1, c1);
        self.fill_with(to, mode)
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

    // ---- paste special, copy marker -----------------------------------------------------------

    /// The copied or cut range of this sheet, the one the marching ants outline.
    pub fn copied_range(&self) -> Option<(u32, u32, u32, u32)> {
        let c = self.copied.as_ref()?;
        (c.origin.sheet == self.sheet).then(|| c.rect())
    }

    /// Esc: drops the marching ants. The clipboard keeps its text, but a later paste is a plain
    /// text paste (a cut no longer clears its source).
    pub fn clear_copied(&mut self) -> bool {
        self.copied.take().is_some()
    }

    fn copy_formats(&mut self, src: Rect4, to: Rect4, clear_source: bool) {
        let rr = |r: Rect4| RangeRef {
            first_row: r.0,
            first_col: r.1,
            last_row: r.2,
            last_col: r.3,
        };
        let _ = self.apply(Command::CopyFormat {
            sheet: self.sheet,
            from: rr(src),
            to: rr(to),
        });
        let overlap = src.0 <= to.2 && to.0 <= src.2 && src.1 <= to.3 && to.1 <= src.3;
        if clear_source && !overlap {
            let _ = self.apply(Command::ClearFormats {
                sheet: self.sheet,
                range: rr(src),
            });
        }
    }

    /// The value a cell holds, as text that enters back as the same value.
    fn value_text(&self, row: u32, col: u32) -> String {
        match self
            .target
            .xlsx()
            .and_then(|wb| wb.cell(self.sheet, row, col))
            .map(|c| &c.value)
        {
            Some(CellValue::Number(n)) => number_text(*n),
            Some(CellValue::Bool(b)) => if *b { "TRUE" } else { "FALSE" }.to_owned(),
            Some(CellValue::Empty) | None => String::new(),
            _ => self.display(row, col),
        }
    }

    /// True after a copy or cut made in this window (Paste Special needs it).
    pub fn has_copied(&self) -> bool {
        self.copied.is_some()
    }

    /// Esc: hides the AutoFill Options button. True when it was showing.
    pub fn dismiss_autofill(&mut self) -> bool {
        self.autofill.take().is_some()
    }

    /// The range of the last fill and how it filled, while the AutoFill Options button shows.
    pub fn autofill_button(&self) -> Option<(Rect4, FillMode)> {
        self.autofill
    }

    /// Paste Special from the last copy made in this window, at the top-left of the selection.
    pub fn paste_special(&mut self, kind: PasteKind) -> Result<Paste, String> {
        let opts = match kind {
            PasteKind::Values => (PasteWhat::Values, false),
            PasteKind::Formulas => (PasteWhat::Formulas, false),
            PasteKind::Formats => (PasteWhat::Formats, false),
            PasteKind::Transpose => (PasteWhat::All, true),
        };
        self.paste_special_with(PasteSpecialOpts {
            what: opts.0,
            skip_blanks: false,
            transpose: opts.1,
        })
    }

    /// Paste Special with the dialog's choices: what to paste, skipping blank source cells,
    /// transposing.
    pub fn paste_special_with(&mut self, o: PasteSpecialOpts) -> Result<Paste, String> {
        let Some(c) = self.copied.clone() else {
            return Ok(Paste::Empty);
        };
        let (r1, c1, ..) = self.range();
        let dest = CellRef {
            sheet: self.sheet,
            row: r1,
            col: c1,
        };
        let src = c.rect();
        let (h, w) = (src.2 - src.0 + 1, src.3 - src.1 + 1);
        let (dh, dw) = if o.transpose { (w, h) } else { (h, w) };
        let dest_rect = (
            r1,
            c1,
            (r1 + dh - 1).min(MAX_ROW),
            (c1 + dw - 1).min(MAX_COL),
        );
        let formats = matches!(o.what, PasteWhat::All | PasteWhat::Formats);
        if o.what == PasteWhat::Formats {
            self.copy_formats(src, dest_rect, false);
            self.select_pasted(dest_rect.0, dest_rect.1, dest_rect.2, dest_rect.3);
            return Ok(Paste::Done);
        }
        let mut rows: Vec<Vec<String>> = match o.what {
            PasteWhat::Values => (src.0..=src.2)
                .map(|r| (src.1..=src.3).map(|cc| self.value_text(r, cc)).collect())
                .collect(),
            _ => paste_rows(&c.rows, Some(c.origin), dest),
        };
        if o.transpose {
            rows = (0..w as usize)
                .map(|j| {
                    rows.iter()
                        .map(|row| row.get(j).cloned().unwrap_or_default())
                        .collect()
                })
                .collect();
        }
        if o.skip_blanks {
            // A blank source cell keeps what the destination holds.
            for (i, row) in rows.iter_mut().enumerate() {
                for (j, cell) in row.iter_mut().enumerate() {
                    if cell.is_empty() {
                        *cell = self.source(r1 + i as u32, c1 + j as u32);
                    }
                }
            }
        }
        if formats {
            self.copy_formats(src, dest_rect, false);
        }
        let (bh, bw) = (rows.len() as u32, rows.first().map_or(0, Vec::len) as u32);
        self.apply(Command::XlsxPaste {
            sheet: self.sheet,
            row: r1,
            col: c1,
            rows,
        })?;
        self.select_pasted(r1, c1, r1 + bh.max(1) - 1, c1 + bw.max(1) - 1);
        Ok(Paste::Done)
    }

    fn select_pasted(&mut self, r1: u32, c1: u32, r2: u32, c2: u32) {
        self.active = (r1, c1);
        self.anchor = (r1, c1);
        self.focus = (r2.min(MAX_ROW), c2.min(MAX_COL));
    }

    // ---- insert and delete ---------------------------------------------------------------------

    /// Insert Sheet Rows / Columns for the selection: as many as are selected, before the first.
    /// `rows` picks rows, else columns.
    pub fn insert_selected(&mut self, rows: bool) -> Result<(), String> {
        let (r1, c1, r2, c2) = self.range();
        let (at, count) = if rows {
            (r1, r2 - r1 + 1)
        } else {
            (c1, c2 - c1 + 1)
        };
        let sheet = self.sheet;
        self.apply(if rows {
            Command::InsertRows { sheet, at, count }
        } else {
            Command::InsertColumns { sheet, at, count }
        })
    }

    /// Delete Sheet Rows / Columns for the selection; what lies below (right) moves up (left).
    pub fn delete_selected(&mut self, rows: bool) -> Result<(), String> {
        let (r1, c1, r2, c2) = self.range();
        let (at, count) = if rows {
            (r1, r2 - r1 + 1)
        } else {
            (c1, c2 - c1 + 1)
        };
        let sheet = self.sheet;
        self.apply(if rows {
            Command::DeleteRows { sheet, at, count }
        } else {
            Command::DeleteColumns { sheet, at, count }
        })
    }

    /// Last row and column that hold data or formats on this sheet.
    fn used_end(&self) -> Option<(u32, u32)> {
        self.target
            .xlsx()?
            .sheet(self.sheet)?
            .used_range()
            .map(|u| (u.2, u.3))
    }

    /// Insert Cells (shift right or down): the selected block becomes empty and its neighbours
    /// make room. Formats move with the cells; the new cells take the format of the cells above
    /// (left of) them, as Excel's default. References in formulas are not rewritten.
    pub fn insert_cells(&mut self, shift: CellShift) -> Result<(), String> {
        self.shift_cells(shift, true)
    }

    /// Delete Cells (shift left or up): the neighbours close the gap.
    pub fn delete_cells(&mut self, shift: CellShift) -> Result<(), String> {
        self.shift_cells(shift, false)
    }

    fn shift_cells(&mut self, dir: CellShift, insert: bool) -> Result<(), String> {
        let (r1, c1, r2, c2) = self.range();
        let vertical = matches!(dir, CellShift::Down | CellShift::Up);
        let Some((ur, uc)) = self.used_end() else {
            return Ok(());
        };
        // Lines run along the shift axis; the band is the span across it.
        let (lo, hi, used, limit, b1, b2) = if vertical {
            (r1, r2, ur, MAX_ROW, c1, c2)
        } else {
            (c1, c2, uc, MAX_COL, r1, r2)
        };
        if used < lo {
            return Ok(());
        }
        let n = hi - lo + 1;
        let end = if insert { (used + n).min(limit) } else { used };
        let range = |a: u32, b: u32| {
            let (first, last) = (a.min(b), a.max(b));
            if vertical {
                RangeRef {
                    first_row: first,
                    first_col: b1,
                    last_row: last,
                    last_col: b2,
                }
            } else {
                RangeRef {
                    first_row: b1,
                    first_col: first,
                    last_row: b2,
                    last_col: last,
                }
            }
        };
        let at = |line: u32, across: u32| {
            if vertical {
                self.source(line, across)
            } else {
                self.source(across, line)
            }
        };
        let mut lines: Vec<Vec<String>> = Vec::new();
        for l in lo..=end {
            let from = if insert {
                l.checked_sub(n).filter(|f| *f >= lo)
            } else {
                Some(l + n).filter(|f| *f <= limit && *f <= used)
            };
            lines.push(
                (b1..=b2)
                    .map(|a| from.map_or_else(String::new, |f| at(f, a)))
                    .collect(),
            );
        }
        let rows: Vec<Vec<String>> = if vertical {
            lines
        } else {
            (0..(b2 - b1 + 1) as usize)
                .map(|a| lines.iter().map(|line| line[a].clone()).collect())
                .collect()
        };
        let sheet = self.sheet;
        if insert {
            if end >= lo + n {
                let _ = self.apply(Command::CopyFormat {
                    sheet,
                    from: range(lo, end - n),
                    to: range(lo + n, end),
                });
            }
            let vacated = range(lo, lo + n - 1);
            let _ = if lo > 1 {
                self.apply(Command::CopyFormat {
                    sheet,
                    from: range(lo - 1, lo - 1),
                    to: vacated,
                })
            } else {
                self.apply(Command::ClearFormats {
                    sheet,
                    range: vacated,
                })
            };
        } else if lo + n <= end {
            let _ = self.apply(Command::CopyFormat {
                sheet,
                from: range(lo + n, end),
                to: range(lo, end - n),
            });
            let _ = self.apply(Command::ClearFormats {
                sheet,
                range: range(end - n + 1, end),
            });
        } else {
            let _ = self.apply(Command::ClearFormats {
                sheet,
                range: range(lo, end),
            });
        }
        let (row, col) = if vertical { (lo, b1) } else { (b1, lo) };
        self.apply(Command::XlsxPaste {
            sheet,
            row,
            col,
            rows,
        })
    }

    /// Delete / Clear Contents: empties every cell of the selection (formats stay).
    pub fn clear_range(&mut self) -> Result<(), String> {
        let (r1, c1, r2, c2) = self.range();
        let Some((ur, uc)) = self.used_end() else {
            return Ok(());
        };
        let (r2, c2) = (r2.min(ur), c2.min(uc));
        if r1 > r2 || c1 > c2 {
            return Ok(());
        }
        if (r1, c1) == (r2, c2) {
            return self.clear(r1, c1);
        }
        let any = (r1..=r2).any(|r| (c1..=c2).any(|c| !self.source(r, c).is_empty()));
        if !any {
            return Ok(());
        }
        let blank = vec![vec![String::new(); (c2 - c1 + 1) as usize]; (r2 - r1 + 1) as usize];
        self.apply(Command::XlsxPaste {
            sheet: self.sheet,
            row: r1,
            col: c1,
            rows: blank,
        })
    }

    // ---- Tab and Enter ---------------------------------------------------------------------------

    /// Tab (`row_major` false) or Enter (`true`) inside a multi-cell selection moves the active
    /// cell through the selection (wrapping) and keeps the selection. `back` is Shift.
    pub fn advance_in_selection(&mut self, row_major: bool, back: bool) {
        let (r1, c1, r2, c2) = self.range();
        let (mut r, mut c) = self.active;
        // Enter walks down a column, then to the top of the next; Tab walks along a row.
        let (pos, lo, hi, other, olo, ohi) = if row_major {
            (&mut r, r1, r2, &mut c, c1, c2)
        } else {
            (&mut c, c1, c2, &mut r, r1, r2)
        };
        if back {
            if *pos > lo {
                *pos -= 1;
            } else {
                *pos = hi;
                *other = if *other > olo { *other - 1 } else { ohi };
            }
        } else if *pos < hi {
            *pos += 1;
        } else {
            *pos = lo;
            *other = if *other < ohi { *other + 1 } else { olo };
        }
        self.active = (r, c);
        self.tab_start = None;
    }

    /// Enter after a run of Tabs returns to the column where the run began, one row down.
    pub fn enter_after_tabs(&mut self, back: bool) {
        match self.tab_start.take() {
            Some(col) if !back => {
                let r = (self.active.0 + 1).min(MAX_ROW);
                self.select_cell((r, col));
            }
            _ => self.move_by(if back { -1 } else { 1 }, 0),
        }
    }

    /// Tab / Shift+Tab: moves along the row and remembers where the run began.
    pub fn tab_move(&mut self, back: bool) {
        let start = self.tab_start.unwrap_or(self.active.1);
        self.move_by(0, if back { -1 } else { 1 });
        self.tab_start = Some(start);
    }

    // ---- Ctrl+; and Ctrl+Shift+; -----------------------------------------------------------------

    /// Ctrl+; (`time` false) or Ctrl+Shift+; writes today's date (the current time) into the
    /// active cell as a constant: `yyyy-mm-dd` or `hh:mm`.
    pub fn enter_now(&mut self, time: bool) -> Result<(), String> {
        let (r, c) = self.active;
        let (serial, builtin) =
            now_serial(time, self.target.xlsx().is_some_and(|wb| wb.date1904()));
        let rr = RangeRef::cell(r, c);
        self.apply(Command::FormatCells {
            sheet: self.sheet,
            range: rr,
            change: Box::new(open365_commands::CellFormatChange {
                number_format: Some(open365_commands::NumberFormatSpec::Builtin(builtin)),
                ..Default::default()
            }),
        })?;
        self.apply(Command::XlsxSetCellValue {
            sheet: self.sheet,
            row: r,
            col: c,
            value: CellEntry::Number(serial),
        })
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
fn series_value(
    src: &[String],
    i: i64,
    vertical: bool,
    date: bool,
    mode: FillMode,
    custom: &[Vec<String>],
) -> String {
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
    let lone_number = len == 1 && split_number(item).is_some_and(|p| p.0.is_empty()) && !date;
    match mode {
        FillMode::Copy => return item.clone(),
        // Ctrl: a lone number counts up; anything else copies.
        FillMode::Ctrl => {
            return if lone_number {
                split_number(item).map_or_else(|| item.clone(), |p| number_text(p.1 + i as f64))
            } else {
                item.clone()
            };
        }
        FillMode::Series if lone_number => {
            return split_number(item)
                .map_or_else(|| item.clone(), |p| number_text(p.1 + i as f64));
        }
        _ => {}
    }
    if let Some(v) = name_series(src, i, custom) {
        return v;
    }
    let parts: Option<Vec<(String, f64)>> = src.iter().map(|s| split_number(s)).collect();
    if let Some(parts) = parts
        && parts.iter().all(|p| p.0 == parts[0].0)
    {
        let prefix = &parts[0].0;
        let ys: Vec<f64> = parts.iter().map(|p| p.1).collect();
        if date
            && prefix.is_empty()
            && ys.len() > 1
            && let Some(v) = month_series(&ys, i)
        {
            return number_text(v);
        }
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

/// Days since 1970-01-01 for a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date `(year, month, day)` of a day count since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Excel's serial number of a civil date (1900 system, valid from 1900-03-01).
fn serial_from_civil(y: i64, m: i64, d: i64) -> f64 {
    (days_from_civil(y, m, d) - days_from_civil(1899, 12, 30)) as f64
}

fn civil_from_serial(serial: f64) -> (i64, i64, i64) {
    civil_from_days(serial.floor() as i64 + days_from_civil(1899, 12, 30))
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Dates a whole number of months apart on the same day of the month continue by month (Excel's
/// Auto series unit); `None` when the dates are not month-spaced.
fn month_series(ys: &[f64], i: i64) -> Option<f64> {
    if ys.iter().any(|y| *y < 61.0 || y.fract() != 0.0) {
        return None;
    }
    let civil: Vec<_> = ys.iter().map(|y| civil_from_serial(*y)).collect();
    let months = |c: (i64, i64, i64)| c.0 * 12 + c.1 - 1;
    let step = months(civil[1]) - months(civil[0]);
    let steady = civil
        .windows(2)
        .all(|w| months(w[1]) - months(w[0]) == step);
    let same_day = civil.iter().all(|c| c.2 == civil[0].2);
    let days_regular = ys
        .windows(2)
        .all(|w| (w[1] - w[0] - (ys[1] - ys[0])).abs() < 1e-9);
    if step == 0 || !steady || !same_day || (days_regular && (ys[1] - ys[0]).abs() < 28.0) {
        return None;
    }
    let total = months(civil[0]) + step * i;
    let (y, m) = (total.div_euclid(12), total.rem_euclid(12) + 1);
    Some(serial_from_civil(y, m, civil[0].2.min(days_in_month(y, m))))
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
fn name_series(src: &[String], i: i64, custom: &[Vec<String>]) -> Option<String> {
    let built_in: [&[&str]; 4] = [&DAYS_LONG, &DAYS_SHORT, &MONTHS_LONG, &MONTHS_SHORT];
    let custom: Vec<Vec<&str>> = custom
        .iter()
        .map(|l| l.iter().map(String::as_str).collect())
        .collect();
    let lists = built_in
        .iter()
        .copied()
        .chain(custom.iter().map(Vec::as_slice));
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

impl Copied {
    fn rect(&self) -> Rect4 {
        let h = self.rows.len().max(1) as u32;
        let w = self.rows.iter().map(Vec::len).max().unwrap_or(1).max(1) as u32;
        (
            self.origin.row,
            self.origin.col,
            self.origin.row + h - 1,
            self.origin.col + w - 1,
        )
    }
}

/// Today's date as a serial number (format 14, m/d/yyyy) or the time of day as a fraction
/// (format 20, h:mm), from the system clock in the OS local time zone (as Excel's Ctrl+; does).
fn now_serial(time: bool, date1904: bool) -> (f64, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    now_serial_at(secs, open365_ui_core::utc_offset_minutes(), time, date1904)
}

/// [`now_serial`] for the instant `utc_secs` (Unix seconds) in a zone `offset_minutes` east of UTC.
fn now_serial_at(utc_secs: i64, offset_minutes: i32, time: bool, date1904: bool) -> (f64, u32) {
    let secs = utc_secs + i64::from(offset_minutes) * 60;
    let day = secs.div_euclid(86_400);
    let (y, m, d) = civil_from_days(day);
    let mut serial = serial_from_civil(y, m, d);
    if date1904 {
        serial -= 1462.0;
    }
    if time {
        let t = secs.rem_euclid(86_400) / 60 * 60;
        (t as f64 / 86_400.0, 20)
    } else {
        (serial, 14)
    }
}

#[cfg(test)]
mod fill_dates {
    use super::*;

    #[test]
    fn ctrl_semicolon_uses_the_local_zone() {
        // 2024-01-01 23:30 UTC is already 2024-01-02 04:00 in UTC+4:30 and still the 1st in UTC-5.
        let utc = 1_704_151_800;
        assert_eq!(now_serial_at(utc, 0, false, false), (45_292.0, 14));
        assert_eq!(now_serial_at(utc, 270, false, false), (45_293.0, 14));
        assert_eq!(now_serial_at(utc, -300, false, false), (45_292.0, 14));
        let (t, f) = now_serial_at(utc, 270, true, false);
        assert_eq!(f, 20);
        assert!((t - 4.0 / 24.0).abs() < 1e-9, "{t}");
        let (t, _) = now_serial_at(utc, -300, true, false);
        assert!((t - (18.0 + 30.0 / 60.0) / 24.0).abs() < 1e-9, "{t}");
    }

    #[test]
    fn civil_dates_round_trip_through_excel_serials() {
        assert_eq!(serial_from_civil(2024, 1, 1), 45292.0);
        assert_eq!(civil_from_serial(45292.0), (2024, 1, 1));
        assert_eq!(serial_from_civil(1900, 3, 1), 61.0);
    }

    #[test]
    fn dates_a_month_apart_continue_by_month_end_clamped() {
        let ys = [
            serial_from_civil(2024, 1, 31),
            serial_from_civil(2024, 2, 29),
        ];
        // Same day of month is required: 31 Jan then 29 Feb is not month-spaced.
        assert_eq!(month_series(&ys, 2), None);
        let ys = [
            serial_from_civil(2024, 1, 15),
            serial_from_civil(2024, 2, 15),
        ];
        assert_eq!(month_series(&ys, 2), Some(serial_from_civil(2024, 3, 15)));
        assert_eq!(month_series(&ys, -1), Some(serial_from_civil(2023, 12, 15)));
        // Weekly dates stay a day series.
        let ys = [serial_from_civil(2024, 1, 1), serial_from_civil(2024, 1, 8)];
        assert_eq!(month_series(&ys, 2), None);
    }
}
