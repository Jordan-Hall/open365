//! Virtualised spreadsheet grid over an XLSX [`Target`].
//!
//! Column widths, row heights, hidden rows and columns, merged cells and frozen panes come
//! from the worksheet XML through `open365_xlsx::SheetGeometry`, converted to pixels the way
//! Excel does at 100% zoom (maximum digit width rule). Only the rows and columns in the
//! viewport are drawn, found by binary search over the geometry and the sparse row list, so a
//! 50,000-row sheet costs the same per frame as a small one.
//!
//! The canvas is one scrolling surface. Frozen rows and columns are drawn at the viewport
//! edge, shifted by the scroll offset, in up to four regions (corner, frozen rows, frozen
//! columns, body), each clipped to its own rectangle.
//!
//! Selection is a range: the active cell is the anchor and `extent` the moving corner, grown
//! by Shift+click, Shift+arrows or a drag, and widened to whole merged cells. A click on a
//! column or row header, Ctrl+Space, Shift+Space and Ctrl+A select whole columns, whole rows
//! or the whole sheet: the active cell stays where it was, and [`Reach`] says how far the
//! range extends past the two corners. The active cell may sit anywhere inside the range:
//! Enter and Tab move it through the range. The sheet is the whole A1:XFD1048576 (positions are
//! summed in `f64`, so rows 20 million pixels down stay exact); Ctrl+arrow, Page Up and Page
//! Down, Home and End follow Excel. Header borders resize on drag and auto-fit on a double click,
//! as [`Resize`] requests. Dragging past the edge of the viewport scrolls. Cell edits go
//! through the formula bar and the command layer: text starting with `=` becomes
//! `XlsxSetCellFormula`, anything else `XlsxSetCellValue`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use egui::text::CCursor;
use egui::text::{LayoutJob, TextFormat};
use egui::text_edit::TextEditState;
use egui::{
    Align, Color32, Context, Event, EventFilter, FontId, Id, Key, Painter, PointerButton, Pos2,
    Rect, Sense, Stroke, TextEdit, Ui, UiBuilder, Vec2, accesskit, vec2,
};
use open365_calc::Value;
use open365_calc::numfmt::{builtin_format_code, format_value};
use open365_commands::{CellEntry, Command, Target};
use open365_xlsx::{
    Border, BorderEdge, BorderStyle, Cell, CellFormat, CellValue, Color, ColorSource, Fill,
    FillPattern, FrozenPanes, GridMetrics, HorizontalAlignment, MAX_COL, MAX_ROW, SheetGeometry,
    SheetKind, SheetState, Styles, Underline, VerticalAlignment, Workbook, Worksheet, col_letters,
    format_number,
};

use crate::a11y;
use crate::fonts::{Face, FontBook};
use crate::grid_text::{
    HintCache, HintedFace, NumberText, TEXT_INSET_PX, color32, effective_horizontal, fill_text,
    fit_number, is_numeric, line_px, overflow_room,
};

const ROW_HEADER_W: f32 = 54.0;
const COL_HEADER_H: f32 = 24.0;
/// How close (in points) the pointer must be to a header border to grab it.
const GRAB: f32 = 3.0;
/// Excel's largest column width in characters and row height in points.
const MAX_COL_WIDTH_CHARS: f64 = 255.0;
const MAX_ROW_HEIGHT_PT: f64 = 409.5;
/// Resize requests kept for the shell to take.
const MAX_PENDING_RESIZES: usize = 64;
/// DPI of a display with no scaling: one egui point is one pixel. The grid lays out at
/// `96 * pixels_per_point` so Excel's pixel rules hold on the real pixel grid.
const BASE_DPI: f32 = 96.0;
/// Largest number of cells Delete clears in one go.
const MAX_CLEAR_CELLS: usize = 10_000;

/// Inclusive cell range `(row1, col1, row2, col2)`.
type Span = (u32, u32, u32, u32);

fn contains(span: Span, row: u32, col: u32) -> bool {
    (span.0..=span.2).contains(&row) && (span.1..=span.3).contains(&col)
}

/// Geometry, merges and frozen panes of one sheet, built once per sheet.
#[derive(Debug)]
struct SheetLayout {
    geo: SheetGeometry,
    /// Sorted by `(row1, col1)`.
    merges: Vec<Span>,
    /// Tallest merge in rows; bounds the backwards scan in [`Self::merge_at`].
    max_span: u32,
    frozen: FrozenPanes,
    /// Device pixels per egui point; geometry is in device pixels.
    ppp: f32,
    /// `(min, max, style)` of columns with a default cell format.
    col_styles: Vec<(u32, u32, u32)>,
}

/// Layout DPI for a scale factor.
fn layout_dpi(ppp: f32) -> u32 {
    (BASE_DPI * ppp).round().max(1.0) as u32
}

impl SheetLayout {
    fn new(wb: &Workbook, sheet: usize, ppp: f32) -> Self {
        let metrics = GridMetrics::for_styles(wb.styles(), layout_dpi(ppp));
        let Some(ws) = wb.sheet(sheet) else {
            return Self {
                geo: SheetGeometry::uniform(metrics),
                merges: Vec::new(),
                max_span: 1,
                frozen: FrozenPanes::default(),
                ppp,
                col_styles: Vec::new(),
            };
        };
        let mut merges = ws.merged_cells();
        merges.sort_unstable();
        let max_span = merges.iter().map(|m| m.2 - m.0 + 1).max().unwrap_or(1);
        Self {
            geo: ws.geometry(metrics),
            merges,
            max_span,
            frozen: ws.frozen_panes().unwrap_or_default(),
            ppp,
            col_styles: ws.column_styles(),
        }
    }

    /// Left edge of column `col` in points from the sheet origin.
    fn x(&self, col: u32) -> f32 {
        self.geo.col_x(col) as f32 / self.ppp
    }

    /// Top edge of row `row` in points from the sheet origin.
    fn y(&self, row: u32) -> f32 {
        self.geo.row_y(row) as f32 / self.ppp
    }

    /// Width of column `col` in points.
    fn w(&self, col: u32) -> f32 {
        self.geo.col_px(col) as f32 / self.ppp
    }

    /// Height of row `row` in points.
    fn h(&self, row: u32) -> f32 {
        self.geo.row_px(row) as f32 / self.ppp
    }

    /// Left edge of column `col` in points, in `f64` (rows reach 21 million pixels, past what
    /// `f32` keeps to the pixel).
    fn x64(&self, col: u32) -> f64 {
        self.geo.col_x(col) as f64 / f64::from(self.ppp)
    }

    /// Top edge of row `row` in points, in `f64`.
    fn y64(&self, row: u32) -> f64 {
        self.geo.row_y(row) as f64 / f64::from(self.ppp)
    }

    /// Column at a point offset from the sheet origin.
    fn col_at(&self, x: f64, limit: u32) -> u32 {
        self.geo
            .col_at((x.max(0.0) * f64::from(self.ppp)) as u64, limit)
    }

    /// Row at a point offset from the sheet origin.
    fn row_at(&self, y: f64, limit: u32) -> u32 {
        self.geo
            .row_at((y.max(0.0) * f64::from(self.ppp)) as u64, limit)
    }

    /// Merges whose first row lies in `a..=b`.
    fn merges_starting_in(&self, a: u32, b: u32) -> &[Span] {
        let lo = self.merges.partition_point(|m| m.0 < a);
        let hi = self.merges.partition_point(|m| m.0 <= b);
        &self.merges[lo..hi.max(lo)]
    }

    /// The merge containing the cell.
    fn merge_at(&self, row: u32, col: u32) -> Option<Span> {
        let first = row.saturating_sub(self.max_span - 1);
        self.merges_starting_in(first, row)
            .iter()
            .copied()
            .find(|m| contains(*m, row, col))
    }

    /// Merges that overlap the given rows and columns.
    fn merges_overlapping(&self, rows: (u32, u32), cols: (u32, u32)) -> Vec<Span> {
        if rows.0 > rows.1 || cols.0 > cols.1 {
            return Vec::new();
        }
        self.merges_starting_in(rows.0.saturating_sub(self.max_span - 1), rows.1)
            .iter()
            .copied()
            .filter(|m| m.2 >= rows.0 && m.1 <= cols.1 && m.3 >= cols.0)
            .collect()
    }

    /// Widens `span` until no merge is cut by its edge.
    fn expand(&self, mut span: Span) -> Span {
        loop {
            let before = span;
            for m in self.merges_overlapping((span.0, span.2), (span.1, span.3)) {
                span = (
                    span.0.min(m.0),
                    span.1.min(m.1),
                    span.2.max(m.2),
                    span.3.max(m.3),
                );
            }
            if span == before {
                return span;
            }
        }
    }

    /// Next visible index from `from` in direction `dir` (`1` or `-1`), or `from` at the edge.
    fn next_visible(&self, rows: bool, from: u32, dir: i32, max: u32) -> u32 {
        let hidden = |i: u32| {
            if rows {
                self.geo.row_hidden(i)
            } else {
                self.geo.col_hidden(i)
            }
        };
        let valid = 1..=i64::from(max);
        let mut i = i64::from(from) + i64::from(dir);
        while valid.contains(&i) && hidden(i as u32) {
            i += i64::from(dir);
        }
        if valid.contains(&i) { i as u32 } else { from }
    }

    /// One keyboard step from `pos`: leaves a merge by its far edge and skips hidden lines.
    fn step(&self, pos: (u32, u32), d_row: i32, d_col: i32) -> (u32, u32) {
        let (mut r, mut c) = pos;
        if let Some(m) = self.merge_at(r, c) {
            if d_row > 0 {
                r = m.2;
            } else if d_row < 0 {
                r = m.0;
            }
            if d_col > 0 {
                c = m.3;
            } else if d_col < 0 {
                c = m.1;
            }
        }
        if d_row != 0 {
            r = self.next_visible(true, r, d_row.signum(), MAX_ROW);
        }
        if d_col != 0 {
            c = self.next_visible(false, c, d_col.signum(), MAX_COL);
        }
        (r, c)
    }
}

/// How far the selection extends past the active cell and the moving corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Reach {
    /// Just the rectangle between the two cells.
    #[default]
    Cells,
    /// Every row of the columns between them.
    Cols,
    /// Every column of the rows between them.
    Rows,
    /// The whole sheet.
    All,
}

/// A part of the column and row headers under the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Header {
    Col(u32),
    Row(u32),
    /// The corner square above the row numbers: selects the sheet.
    Corner,
}

/// The selection while the pointer changes it.
#[derive(Debug, Clone, Copy)]
struct Sel {
    active: (u32, u32),
    anchor: (u32, u32),
    extent: (u32, u32),
    dragging: bool,
    reach: Reach,
}

/// A column width or row height the user asked for, by dragging a header border or by
/// double-clicking it (auto-fit). Units are the file's: characters of the default font for a
/// column, points for a row. The shell applies it through a typed command.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Resize {
    ColumnWidth {
        sheet: usize,
        first: u32,
        last: u32,
        width: f64,
    },
    RowHeight {
        sheet: usize,
        first: u32,
        last: u32,
        points: f64,
    },
}

/// A header border being dragged.
#[derive(Debug, Clone, Copy)]
struct ResizeDrag {
    cols: bool,
    index: u32,
    /// Size in device pixels when the drag began.
    start_px: u32,
    /// Pointer coordinate along the resized axis when the drag began.
    origin: f32,
    /// Size in device pixels now.
    px: u32,
}

/// Step taken after an entry is committed or Enter/Tab is pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Enter; `true` for Shift+Enter.
    Enter(bool),
    /// Tab; `true` for Shift+Tab.
    Tab(bool),
}

/// Where cells sit on screen for one frame.
struct View<'a> {
    layout: &'a SheetLayout,
    /// Screen position of the viewport's top-left corner (where the column and row headers
    /// meet).
    view_min: Pos2,
    /// Scroll offset.
    scroll: Vec2,
}

impl View<'_> {
    /// Screen x of the left edge of column `col`, with a frozen shift applied. The sum is done
    /// in `f64` so a position 20 million pixels down the sheet stays exact to the pixel.
    fn edge_x(&self, col: u32, shift: f32) -> f32 {
        (f64::from(self.view_min.x)
            + f64::from(ROW_HEADER_W)
            + self.layout.x64(col)
            + f64::from(shift)
            - f64::from(self.scroll.x)) as f32
    }

    /// Screen y of the top edge of row `row`, with a frozen shift applied.
    fn edge_y(&self, row: u32, shift: f32) -> f32 {
        (f64::from(self.view_min.y)
            + f64::from(COL_HEADER_H)
            + self.layout.y64(row)
            + f64::from(shift)
            - f64::from(self.scroll.y)) as f32
    }

    /// Cell range on screen, moved by `shift` (a frozen block follows the scroll).
    fn rect(&self, span: Span, shift: Vec2) -> Rect {
        Rect::from_min_max(
            Pos2::new(self.edge_x(span.1, shift.x), self.edge_y(span.0, shift.y)),
            Pos2::new(
                self.edge_x(span.3 + 1, shift.x),
                self.edge_y(span.2 + 1, shift.y),
            ),
        )
    }

    /// Shift that pins a frozen row or column to the viewport edge.
    fn shift_for(&self, row: u32, col: u32) -> Vec2 {
        let f = self.layout.frozen;
        vec2(
            if col <= f.cols { self.scroll.x } else { 0.0 },
            if row <= f.rows { self.scroll.y } else { 0.0 },
        )
    }

    /// On-screen rectangle of a cell, or of a merge anchored at `(span.0, span.1)`.
    fn screen_rect(&self, span: Span) -> Rect {
        self.rect(span, self.shift_for(span.0, span.1))
    }

    /// Width and height of the frozen block in pixels.
    fn frozen_size(&self) -> Vec2 {
        let f = self.layout.frozen;
        vec2(self.layout.x(f.cols + 1), self.layout.y(f.rows + 1))
    }

    /// Position along one axis on the sheet, with the frozen block pinned to the edge.
    fn along(local: f32, frozen: f32, scroll: f32) -> f64 {
        if local < frozen {
            f64::from(local.max(0.0))
        } else {
            f64::from(local) + f64::from(scroll)
        }
    }

    /// The header under a screen position, if any.
    fn header_at(&self, pos: Pos2, limits: (u32, u32)) -> Option<Header> {
        let local = pos - self.view_min;
        let fz = self.frozen_size();
        match (local.x < ROW_HEADER_W, local.y < COL_HEADER_H) {
            _ if local.x < 0.0 || local.y < 0.0 => None,
            (true, true) => Some(Header::Corner),
            (false, true) => {
                let x = Self::along(local.x - ROW_HEADER_W, fz.x, self.scroll.x);
                Some(Header::Col(self.layout.col_at(x, limits.1)))
            }
            (true, false) => {
                let y = Self::along(local.y - COL_HEADER_H, fz.y, self.scroll.y);
                Some(Header::Row(self.layout.row_at(y, limits.0)))
            }
            (false, false) => None,
        }
    }

    /// The header border within reach of a screen position: `Header::Col(c)` is the right
    /// edge of column `c`, `Header::Row(r)` the bottom edge of row `r`. Borders of hidden
    /// lines belong to the visible line before them.
    fn border_at(&self, pos: Pos2, limits: (u32, u32)) -> Option<Header> {
        let local = pos - self.view_min;
        let fz = self.frozen_size();
        let f = self.layout.frozen;
        if local.x < 0.0 || local.y < 0.0 {
            return None;
        }
        if local.y < COL_HEADER_H && local.x >= ROW_HEADER_W {
            let x = Self::along(local.x - ROW_HEADER_W, fz.x, self.scroll.x);
            let col = self.layout.col_at(x, limits.1);
            let shift = if col <= f.cols { self.scroll.x } else { 0.0 };
            if (pos.x - self.edge_x(col + 1, shift)).abs() <= GRAB
                && self.layout.geo.col_px(col) > 0
            {
                return Some(Header::Col(col));
            }
            if (pos.x - self.edge_x(col, shift)).abs() <= GRAB && col > 1 {
                let prev = self.layout.next_visible(false, col, -1, MAX_COL);
                if prev != col {
                    return Some(Header::Col(prev));
                }
            }
        } else if local.x < ROW_HEADER_W && local.y >= COL_HEADER_H {
            let y = Self::along(local.y - COL_HEADER_H, fz.y, self.scroll.y);
            let row = self.layout.row_at(y, limits.0);
            let shift = if row <= f.rows { self.scroll.y } else { 0.0 };
            if (pos.y - self.edge_y(row + 1, shift)).abs() <= GRAB
                && self.layout.geo.row_px(row) > 0
            {
                return Some(Header::Row(row));
            }
            if (pos.y - self.edge_y(row, shift)).abs() <= GRAB && row > 1 {
                let prev = self.layout.next_visible(true, row, -1, MAX_ROW);
                if prev != row {
                    return Some(Header::Row(prev));
                }
            }
        }
        None
    }

    /// The cell under a screen position; `clamp` keeps a position outside the cell area
    /// inside it (used while dragging).
    fn cell_at(&self, pos: Pos2, clamp: bool, limits: (u32, u32)) -> Option<(u32, u32)> {
        let local = pos - self.view_min - vec2(ROW_HEADER_W, COL_HEADER_H);
        if !clamp && (local.x < 0.0 || local.y < 0.0) {
            return None;
        }
        let fz = self.frozen_size();
        let sheet_x = Self::along(local.x, fz.x, self.scroll.x);
        let sheet_y = Self::along(local.y, fz.y, self.scroll.y);
        let col = self.layout.col_at(sheet_x, limits.1);
        let row = self.layout.row_at(sheet_y, limits.0);
        Some((row, col))
    }
}

/// One drawn part of the grid: a block of rows and columns, its shift and its clip.
#[derive(Clone, Copy)]
struct Region {
    rows: (u32, u32),
    cols: (u32, u32),
    shift: Vec2,
    clip: Rect,
}

/// Rows and columns that were visible in the last frame.
#[derive(Debug, Clone, Copy, Default)]
struct Visible {
    frozen_rows: u32,
    frozen_cols: u32,
    rows: (u32, u32),
    cols: (u32, u32),
    scroll: Vec2,
    view_min: Pos2,
    /// Size of the viewport, headers included.
    size: Vec2,
}

struct Palette {
    text: Color32,
    grid_line: Color32,
    header_bg: Color32,
    cell_bg: Color32,
    accent: Color32,
}

/// State of the grid view.
#[derive(Debug)]
pub struct GridView {
    sheet: usize,
    /// The cell that is edited; it can sit anywhere inside the selection (Enter and Tab move it
    /// through the selection).
    active: (u32, u32),
    /// The fixed corner of the selection (where it was started).
    anchor: (u32, u32),
    /// The moving corner of the selection.
    extent: (u32, u32),
    /// Whether the selection runs past the corners to whole columns, rows or the sheet.
    reach: Reach,
    dragging: bool,
    /// The last press was on a header (a double click there does not edit a cell).
    press_on_header: bool,
    /// Frame in which a press or move changed the selection. Egui drops focus from the cell
    /// widgets in that frame and the next (press, then click), so both ask again.
    refocus_frame: Option<u64>,
    editing: bool,
    edit_text: String,
    scroll_target: Option<(u32, u32)>,
    /// Scroll offset to jump to next frame (Page Up and Page Down).
    pending_scroll: Option<Vec2>,
    /// Column that Tab started a run of entries in; Enter goes back to it.
    return_col: Option<u32>,
    resize_drag: Option<ResizeDrag>,
    resizes: Vec<Resize>,
    want_focus: bool,
    grid_id: Id,
    formula_id: Id,
    /// Cell the screen reader asked to focus, applied next frame.
    pending_focus: Option<(u32, u32)>,
    /// Widget id that holds keyboard focus and is the node id of the active cell.
    focus_id: Id,
    /// Set when this frame's key events were already used by the formula bar.
    keys_consumed: bool,
    /// Geometry of the current sheet, built once per sheet.
    layout: Option<(usize, u32, Arc<SheetLayout>)>,
    visible: Visible,
    fonts: Rc<RefCell<FontBook>>,
    /// The application syncs a shared font book; a grid with its own syncs it itself.
    owns_fonts: bool,
    painted: RefCell<Vec<PaintedCell>>,
    /// Hinted glyph advances for the cell text decisions.
    hints: RefCell<HintCache>,
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

/// The format code of a cell style. Built-in ids that depend on the locale (14 is a short date)
/// use the calculation engine's table, which the engine formats; custom codes win.
fn format_code(wb: &Workbook, style: u32) -> &str {
    let styles = wb.styles();
    let id = styles.num_fmt_id(style);
    if styles.custom_formats().contains_key(&id) {
        return styles.format_code(id);
    }
    builtin_format_code(id).unwrap_or_else(|| styles.format_code(id))
}

/// The text shown in a cell: its value through the cell's number format.
fn display_text(wb: &Workbook, cell: &Cell) -> String {
    let code = format_code(wb, cell.style.unwrap_or(0));
    match &cell.value {
        CellValue::Empty => String::new(),
        CellValue::Number(n) => format_value(&Value::Number(*n), code, wb.date1904()),
        CellValue::Bool(true) => "TRUE".to_owned(),
        CellValue::Bool(false) => "FALSE".to_owned(),
        CellValue::Error(e) => e.code().to_owned(),
        other => {
            let text = other.as_text().unwrap_or_default();
            if code == "General" || code.is_empty() {
                text.to_owned()
            } else {
                format_value(&Value::Text(text.to_owned()), code, wb.date1904())
            }
        }
    }
}

/// The address of a range: `B2`, `B2:C4`, `B:D` for whole columns, `2:4` for whole rows.
fn range_label(span: Span) -> String {
    let whole_cols = span.0 == 1 && span.2 == MAX_ROW;
    let whole_rows = span.1 == 1 && span.3 == MAX_COL;
    if whole_cols && whole_rows {
        return format!("A:{}", col_letters(MAX_COL));
    }
    if whole_cols {
        return format!("{}:{}", col_letters(span.1), col_letters(span.3));
    }
    if whole_rows {
        return format!("{}:{}", span.0, span.2);
    }
    let a = format!("{}{}", col_letters(span.1), span.0);
    if (span.0, span.1) == (span.2, span.3) {
        a
    } else {
        format!("{a}:{}{}", col_letters(span.3), span.2)
    }
}

impl GridView {
    pub fn new() -> Self {
        Self {
            sheet: 0,
            active: (1, 1),
            anchor: (1, 1),
            extent: (1, 1),
            reach: Reach::Cells,
            dragging: false,
            press_on_header: false,
            refocus_frame: None,
            editing: false,
            edit_text: String::new(),
            scroll_target: None,
            pending_scroll: None,
            return_col: None,
            resize_drag: None,
            resizes: Vec::new(),
            want_focus: true,
            grid_id: Id::new("open365-grid-view"),
            formula_id: Id::new("open365-formula-bar"),
            pending_focus: None,
            focus_id: Id::new("open365-grid-active-cell"),
            keys_consumed: false,
            layout: None,
            visible: Visible::default(),
            fonts: Rc::new(RefCell::new(FontBook::new())),
            owns_fonts: true,
            painted: RefCell::new(Vec::new()),
            hints: RefCell::new(HintCache::default()),
            status: None,
            last_frame_ms: 0.0,
        }
    }

    /// A grid that draws with the application's shared font book (which the application syncs).
    pub(crate) fn with_fonts(fonts: Rc<RefCell<FontBook>>) -> Self {
        Self {
            fonts,
            owns_fonts: false,
            ..Self::new()
        }
    }

    /// The cells drawn in the last frame, with the text as shown.
    pub fn painted(&self) -> Vec<PaintedCell> {
        self.painted.borrow().clone()
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

    /// The selected range `(row1, col1, row2, col2)`, widened to whole merged cells.
    pub fn selection(&self) -> (u32, u32, u32, u32) {
        let (a, e) = (self.anchor, self.extent);
        let (r1, c1, r2, c2) = (a.0.min(e.0), a.1.min(e.1), a.0.max(e.0), a.1.max(e.1));
        let raw = match self.reach {
            Reach::Cells => (r1, c1, r2, c2),
            Reach::Cols => (1, c1, MAX_ROW, c2),
            Reach::Rows => (r1, 1, r2, MAX_COL),
            Reach::All => (1, 1, MAX_ROW, MAX_COL),
        };
        self.layout
            .as_ref()
            .map_or(raw, |(_, _, layout)| layout.expand(raw))
    }

    /// The column widths and row heights the user asked for since the last call.
    pub fn take_resizes(&mut self) -> Vec<Resize> {
        std::mem::take(&mut self.resizes)
    }

    pub fn is_editing(&self) -> bool {
        self.editing
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn set_active(&mut self, row: u32, col: u32) {
        self.active = (row.clamp(1, MAX_ROW), col.clamp(1, MAX_COL));
        self.anchor = self.active;
        self.extent = self.active;
        self.reach = Reach::Cells;
        self.return_col = None;
        self.scroll_target = Some(self.active);
    }

    /// Selects whole columns `first..=last`. The active cell keeps its row, and its column if
    /// that is an end of the range (else it moves to `first`).
    pub fn select_columns(&mut self, first: u32, last: u32) {
        let (first, last) = (first.clamp(1, MAX_COL), last.clamp(1, MAX_COL));
        let (first, last) = (first.min(last), first.max(last));
        let (col, other) = Self::ends(self.active.1, first, last);
        self.active = (self.active.0, col);
        self.anchor = self.active;
        self.extent = (self.active.0, other);
        self.reach = Reach::Cols;
    }

    /// Selects whole rows `first..=last` (see [`GridView::select_columns`]).
    pub fn select_rows(&mut self, first: u32, last: u32) {
        let (first, last) = (first.clamp(1, MAX_ROW), last.clamp(1, MAX_ROW));
        let (first, last) = (first.min(last), first.max(last));
        let (row, other) = Self::ends(self.active.0, first, last);
        self.active = (row, self.active.1);
        self.anchor = self.active;
        self.extent = (other, self.active.1);
        self.reach = Reach::Rows;
    }

    /// The active index and the moving corner's index for a range `first..=last`: the active
    /// cell stays when it is at one end of the range.
    fn ends(active: u32, first: u32, last: u32) -> (u32, u32) {
        if active == last {
            (last, first)
        } else {
            (first, last)
        }
    }

    /// Selects the whole sheet; the active cell stays.
    pub fn select_all(&mut self) {
        self.anchor = self.active;
        self.extent = self.active;
        self.reach = Reach::All;
    }

    /// Selects the range from `anchor` (the active cell) to `corner`.
    pub fn select_range(&mut self, anchor: (u32, u32), corner: (u32, u32)) {
        self.set_active(anchor.0, anchor.1);
        self.extent = (corner.0.clamp(1, MAX_ROW), corner.1.clamp(1, MAX_COL));
        self.reach = Reach::Cells;
        self.scroll_target = Some(self.extent);
    }

    pub fn select_sheet(&mut self, sheet: usize) {
        if sheet != self.sheet {
            self.sheet = sheet;
            self.active = (1, 1);
            self.anchor = (1, 1);
            self.extent = (1, 1);
            self.reach = Reach::Cells;
            self.dragging = false;
            self.resize_drag = None;
            self.editing = false;
            self.layout = None;
            self.scroll_target = Some((1, 1));
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
            Some(CellValue::Number(n)) => {
                let mut out = String::new();
                format_number(*n, &mut out);
                out
            }
            Some(CellValue::Bool(b)) => if *b { "TRUE" } else { "FALSE" }.to_owned(),
            Some(CellValue::Error(e)) => e.code().to_owned(),
            Some(other) => other.as_text().unwrap_or_default().to_owned(),
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

    /// Focuses the active cell. Requesting focus on a widget that has it resets its key filter,
    /// so ask only when needed and set the filter again.
    fn focus_active(&self, ctx: &Context) {
        let id = self.cell_id(self.active.0, self.active.1);
        ctx.memory_mut(|m| {
            if !m.has_focus(id) {
                m.request_focus(id);
            }
            m.set_focus_lock_filter(
                id,
                EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: false,
                },
            );
        });
    }

    fn cancel_edit(&mut self, ctx: &Context) {
        self.editing = false;
        self.keys_consumed = true;
        self.focus_active(ctx);
    }

    /// Commits the formula bar text to the active cell through the command layer; `advance`
    /// then moves on as Enter does.
    pub fn commit_edit(&mut self, ctx: &Context, target: &mut Target, advance: bool) {
        self.commit(ctx, target, advance.then_some(Step::Enter(false)));
    }

    fn commit(&mut self, ctx: &Context, target: &mut Target, step: Option<Step>) {
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
        if let Some(step) = step {
            self.advance(step, true);
        }
        self.focus_active(ctx);
    }

    /// `pos` moved by `steps` cells (sign gives direction) along one axis.
    fn walk(&self, mut pos: (u32, u32), d_row: i32, d_col: i32) -> (u32, u32) {
        let layout = self.layout.as_ref().map(|(_, _, l)| l);
        for _ in 0..d_row.unsigned_abs().max(d_col.unsigned_abs()) {
            pos = match layout {
                Some(l) => l.step(pos, d_row.signum(), d_col.signum()),
                None => (
                    (i64::from(pos.0) + i64::from(d_row.signum())).clamp(1, i64::from(MAX_ROW))
                        as u32,
                    (i64::from(pos.1) + i64::from(d_col.signum())).clamp(1, i64::from(MAX_COL))
                        as u32,
                ),
            };
        }
        pos
    }

    /// The anchor cell of the merge `pos` sits in, or `pos` itself.
    fn anchor_of(&self, pos: (u32, u32)) -> (u32, u32) {
        self.layout
            .as_ref()
            .and_then(|(_, _, l)| l.merge_at(pos.0, pos.1))
            .map_or(pos, |m| (m.0, m.1))
    }

    fn move_active(&mut self, d_row: i32, d_col: i32) {
        let to = self.anchor_of(self.walk(self.active, d_row, d_col));
        self.set_active(to.0, to.1);
    }

    /// Whether the selection is only the active cell (or the merge it sits in).
    fn is_single(&self) -> bool {
        let (r, c) = self.active;
        let own = self
            .layout
            .as_ref()
            .and_then(|(_, _, l)| l.merge_at(r, c))
            .unwrap_or((r, c, r, c));
        self.selection() == own
    }

    /// Enter and Tab: with one cell selected they move the cell on; with a range selected
    /// they move the active cell through the range (Enter down a column, then to the next
    /// column; Tab along a row, then to the next row) and wrap from the last cell to the first,
    /// leaving the selection as it is. Hidden rows and columns are skipped.
    fn advance(&mut self, step: Step, from_edit: bool) {
        let remembered = self.return_col.take();
        let single = self.is_single();
        match step {
            Step::Enter(back) => {
                if from_edit
                    && !back
                    && single
                    && let Some(col) = remembered
                {
                    // Tab, Tab, Enter: back to the column the entries began in, one row down.
                    let to = self.anchor_of(self.walk((self.active.0, col), 1, 0));
                    self.set_active(to.0, to.1);
                    return;
                }
                self.cycle(true, back);
            }
            Step::Tab(back) => {
                let col = remembered.unwrap_or(self.active.1);
                self.cycle(false, back);
                if from_edit && !back && single {
                    self.return_col = Some(col);
                }
            }
        }
    }

    /// Moves the active cell through the selection (see [`GridView::advance`]); `down_first`
    /// is Enter.
    fn cycle(&mut self, down_first: bool, backward: bool) {
        let sel = self.selection();
        let layout = self.layout.as_ref().map(|(_, _, l)| Arc::clone(l));
        let Some(layout) = layout else {
            return;
        };
        if self.is_single() || !contains(sel, self.active.0, self.active.1) {
            let k = if backward { -1 } else { 1 };
            let (d_row, d_col) = if down_first { (k, 0) } else { (0, k) };
            self.move_active(d_row, d_col);
            return;
        }
        let dir = if backward { -1 } else { 1 };
        // Primary axis moves first (rows for Enter), the secondary one on wrap-around.
        let (p_rows, plo, phi, slo, shi) = if down_first {
            (true, sel.0, sel.2, sel.1, sel.3)
        } else {
            (false, sel.1, sel.3, sel.0, sel.2)
        };
        let max = |rows: bool| if rows { MAX_ROW } else { MAX_COL };
        let step_in = |rows: bool, i: u32, lo: u32, hi: u32| {
            let n = layout.next_visible(rows, i, dir, max(rows));
            (n != i && (lo..=hi).contains(&n)).then_some(n)
        };
        // First visible index on the way in.
        let edge = |rows: bool, lo: u32, hi: u32| {
            let from = if backward { hi } else { lo };
            let hidden = if rows {
                layout.geo.row_hidden(from)
            } else {
                layout.geo.col_hidden(from)
            };
            if hidden {
                step_in(rows, from, lo, hi).unwrap_or(from)
            } else {
                from
            }
        };
        let (mut p, mut s) = if down_first {
            self.active
        } else {
            (self.active.1, self.active.0)
        };
        for _ in 0..1024 {
            match step_in(p_rows, p, plo, phi) {
                Some(n) => p = n,
                None => {
                    p = edge(p_rows, plo, phi);
                    s = step_in(!p_rows, s, slo, shi).unwrap_or_else(|| edge(!p_rows, slo, shi));
                }
            }
            let cell = if down_first { (p, s) } else { (s, p) };
            // Cells covered by a merge are not stops; its anchor cell is.
            if layout
                .merge_at(cell.0, cell.1)
                .is_none_or(|m| (m.0, m.1) == cell)
            {
                self.active = cell;
                self.scroll_target = Some(cell);
                return;
            }
        }
    }

    /// Starts growing the selection from the active cell (Shift+movement). After Enter or Tab
    /// moved the active cell inside a range, the range is re-anchored there.
    fn begin_extend(&mut self) {
        if self.anchor != self.active {
            self.anchor = self.active;
            self.extent = self.active;
        }
    }

    /// Moves the selection's moving corner, keeping the active cell.
    /// Whole columns grow only sideways and whole rows only down; the whole sheet stays.
    fn extend(&mut self, d_row: i32, d_col: i32) {
        let (d_row, d_col) = match self.reach {
            Reach::Cells => (d_row, d_col),
            Reach::Cols => (0, d_col),
            Reach::Rows => (d_row, 0),
            Reach::All => (0, 0),
        };
        self.begin_extend();
        self.extent = self.walk(self.extent, d_row, d_col);
        self.scroll_target = Some(self.extent);
    }

    fn extend_to(&mut self, row: u32, col: u32) {
        self.begin_extend();
        let row = if matches!(self.reach, Reach::Cols | Reach::All) {
            self.extent.0
        } else {
            row
        };
        let col = if matches!(self.reach, Reach::Rows | Reach::All) {
            self.extent.1
        } else {
            col
        };
        self.extent = (row.clamp(1, MAX_ROW), col.clamp(1, MAX_COL));
        self.scroll_target = Some(self.extent);
    }

    /// The cell Ctrl+arrow reaches from `from` on this sheet (see [`data_edge`]): Excel's jump
    /// to the edge of the data. `d_row` or `d_col` is `-1` or `1`, the other `0`.
    pub fn ctrl_arrow_target(
        &self,
        target: &Target,
        from: (u32, u32),
        d_row: i32,
        d_col: i32,
    ) -> (u32, u32) {
        let ws = target.xlsx().and_then(|wb| wb.sheet(self.sheet));
        self.layout.as_ref().map_or(from, |(_, _, layout)| {
            data_edge(ws, layout, from, d_row, d_col)
        })
    }

    /// Ctrl+Home: the first cell below and right of the frozen panes.
    fn home_cell(&self) -> (u32, u32) {
        self.layout
            .as_ref()
            .map_or((1, 1), |(_, _, l)| (l.frozen.rows + 1, l.frozen.cols + 1))
    }

    /// Ctrl+End: the last cell of the used range.
    fn end_cell(&self, target: &Target) -> (u32, u32) {
        target
            .xlsx()
            .and_then(|wb| wb.sheet(self.sheet))
            .and_then(Worksheet::used_range)
            .map_or((1, 1), |(_, _, r2, c2)| (r2, c2))
    }

    /// Page Up and Page Down: the active cell and the window move by the rows that fit the
    /// window, so the cell keeps its place on screen. With `extend` the moving corner moves.
    fn page(&mut self, down: bool, extend: bool) {
        let Some((_, _, layout)) = self.layout.clone() else {
            return;
        };
        let vis = self.visible;
        let fz_y = f64::from(layout.y(layout.frozen.rows + 1));
        let height = f64::from((vis.size.y - COL_HEADER_H).max(0.0)) - fz_y;
        let top = layout.row_at(fz_y + f64::from(vis.scroll.y), MAX_ROW);
        let mut last = layout.row_at(layout.y64(top) + height, MAX_ROW);
        if layout.y64(last + 1) > layout.y64(top) + height + 0.01 {
            last = last.saturating_sub(1);
        }
        let last = last.max(top);
        let dy = layout.y64(last + 1) - layout.y64(top);
        let dy = if down { dy } else { -dy };
        let moved = |row: u32| {
            let y = (layout.y64(row) + dy).max(0.0);
            layout.row_at(y, MAX_ROW).clamp(1, MAX_ROW)
        };
        if extend {
            if matches!(self.reach, Reach::Cells | Reach::Rows) {
                self.begin_extend();
                self.extent.0 = moved(self.extent.0);
            }
        } else {
            let to = self.anchor_of((moved(self.active.0), self.active.1));
            self.set_active(to.0, to.1);
            self.scroll_target = None;
        }
        let scroll_y = (f64::from(vis.scroll.y) + dy).max(0.0) as f32;
        self.pending_scroll = Some(vec2(vis.scroll.x, scroll_y));
    }

    fn layout_for(&mut self, wb: &Workbook, ppp: f32) -> Arc<SheetLayout> {
        let dpi = layout_dpi(ppp);
        if let Some((sheet, cached_dpi, layout)) = &self.layout
            && *sheet == self.sheet
            && *cached_dpi == dpi
        {
            return Arc::clone(layout);
        }
        let layout = Arc::new(SheetLayout::new(wb, self.sheet, ppp));
        self.layout = Some((self.sheet, dpi, Arc::clone(&layout)));
        layout
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
        if self.owns_fonts {
            self.fonts.borrow_mut().sync(ui.ctx());
        }
        self.layout_for(wb, ui.ctx().pixels_per_point());
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
        let address = range_label(self.selection());
        let active_address = format!("{}{}", col_letters(col), row);
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
                        format!("Formula bar for {active_address}"),
                    )
                });
                if resp.changed() || (resp.gained_focus() && !self.editing) {
                    self.editing = true;
                }
                let (enter, tab, escape, shift) = ctx.input(|i| {
                    (
                        i.key_pressed(Key::Enter),
                        i.key_pressed(Key::Tab),
                        i.key_pressed(Key::Escape),
                        i.modifiers.shift,
                    )
                });
                if self.editing && escape && (resp.has_focus() || resp.lost_focus()) {
                    self.cancel_edit(&ctx);
                } else if self.editing && resp.lost_focus() && (enter || tab) {
                    let step = if enter {
                        Step::Enter(shift)
                    } else {
                        Step::Tab(shift)
                    };
                    self.commit(&ctx, target, Some(step));
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
        let layout = self.layout_for(wb, ctx.pixels_per_point());
        // The grid is the whole sheet, A1 to XFD1048576; only the visible part is built.
        let (rows_n, cols_n) = (MAX_ROW, MAX_COL);
        // Selecting any cell of a merge selects the merge, anchored at its top-left cell.
        self.active = self.anchor_of(self.active);
        self.anchor = self.anchor_of(self.anchor);
        let content = vec2(
            ROW_HEADER_W + layout.x(cols_n + 1),
            COL_HEADER_H + layout.y(rows_n + 1),
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
            self.handle_keys(&ctx, target);
        }
        let Some(wb) = target.xlsx() else { return };

        let pal = Palette {
            text: ui.visuals().text_color(),
            grid_line: ui.visuals().widgets.noninteractive.bg_stroke.color,
            header_bg: ui.visuals().faint_bg_color,
            cell_bg: ui.visuals().extreme_bg_color,
            accent: ui.visuals().selection.stroke.color,
        };
        let font = FontId::proportional(13.0);
        let active = self.active;
        let selection = self.selection();
        let scroll_target = self.scroll_target.take();
        let grid_id = self.grid_id;
        let ws = wb.sheet(sheet);

        // Where the sheet scrolls to this frame. The offset is set directly (not eased) so a
        // jump of a million rows lands at once.
        let view_size = ui.available_size();
        let mut want_scroll = self.pending_scroll.take();
        if let Some(cell) = scroll_target {
            let base = want_scroll.unwrap_or(self.visible.scroll);
            let to = scroll_to_cell(&layout, cell, base, view_size);
            if to != base || want_scroll.is_some() {
                want_scroll = Some(to);
            }
        }

        let mut sel = Sel {
            active: self.active,
            anchor: self.anchor,
            extent: self.extent,
            dragging: self.dragging,
            reach: self.reach,
        };
        let mut press_on_header = self.press_on_header;
        let mut resize_drag = self.resize_drag;
        let mut new_resizes: Vec<Resize> = Vec::new();
        let mut pressed = false;
        let mut double_clicked = false;
        let mut visible = Visible::default();

        let mut area = egui::ScrollArea::both()
            .auto_shrink([false, false])
            .scroll_source(egui::scroll_area::ScrollSource {
                drag: egui::scroll_area::DragScroll::Never,
                ..egui::scroll_area::ScrollSource::ALL
            });
        if let Some(offset) = want_scroll {
            area = area.scroll_offset(offset);
        }
        area.show_viewport(ui, |ui, viewport| {
            let (rect, _) = ui.allocate_exact_size(content, Sense::hover());
            let response = ui.interact(rect, grid_id, Sense::click_and_drag());
            let scroll = viewport.min.to_vec2();
            let clip_all = ui.clip_rect();
            let view_min = clip_all.min;
            let view = View {
                layout: &layout,
                view_min,
                scroll,
            };
            let frozen = layout.frozen;
            let fz = view.frozen_size();
            let painter = ui.painter_at(clip_all);

            // Visible rows and columns of the scrolling part.
            let x0 = (scroll.x + fz.x).max(0.0);
            let x1 = (scroll.x + viewport.width() - ROW_HEADER_W).max(0.0);
            let y0 = (scroll.y + fz.y).max(0.0);
            let y1 = (scroll.y + viewport.height() - COL_HEADER_H).max(0.0);
            let (x0, x1, y0, y1) = (f64::from(x0), f64::from(x1), f64::from(y0), f64::from(y1));
            let first_col = layout.col_at(x0, cols_n).max(frozen.cols + 1).min(cols_n);
            let last_col = layout.col_at(x1.max(x0), cols_n).max(first_col);
            let first_row = layout.row_at(y0, rows_n).max(frozen.rows + 1).min(rows_n);
            let last_row = layout.row_at(y1.max(y0), rows_n).max(first_row);
            visible = Visible {
                frozen_rows: frozen.rows.min(rows_n),
                frozen_cols: frozen.cols.min(cols_n),
                rows: (first_row, last_row),
                cols: (first_col, last_col),
                scroll,
                view_min,
                size: viewport.size(),
            };

            // Pointer: press selects (Shift extends), dragging grows the range.
            let events = ctx.input(|i| {
                i.events
                    .iter()
                    .filter(|e| matches!(e, Event::PointerButton { .. } | Event::PointerMoved(_)))
                    .cloned()
                    .collect::<Vec<_>>()
            });
            let limits = (rows_n, cols_n);
            let ppp = ctx.pixels_per_point();
            let own_layer = |pos: Pos2| {
                clip_all.contains(pos)
                    && ctx
                        .layer_id_at(pos)
                        .is_none_or(|layer| layer == ui.layer_id())
            };
            // Moves the dragged corner to the cell under `pos`, along the axes the
            // selection grows on.
            let drag_to = |sel: &mut Sel, pos: Pos2| {
                if let Some(cell) = view.cell_at(pos, true, limits) {
                    match sel.reach {
                        Reach::Cells => sel.extent = cell,
                        Reach::Cols => sel.extent.1 = cell.1,
                        Reach::Rows => sel.extent.0 = cell.0,
                        Reach::All => {}
                    }
                }
            };
            // The size a header border drag has reached with the pointer at `pos`.
            let drag_size = |d: &ResizeDrag, pos: Pos2| {
                let along = if d.cols { pos.x } else { pos.y };
                let max = if d.cols {
                    layout.geo.metrics().col_px(MAX_COL_WIDTH_CHARS)
                } else {
                    layout.geo.metrics().row_px(MAX_ROW_HEIGHT_PT)
                };
                let px = f64::from(d.start_px) + f64::from((along - d.origin) * ppp);
                (px.round().max(0.0) as u32).min(max)
            };
            for event in events {
                match event {
                    Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed: true,
                        modifiers,
                    } if own_layer(pos) => {
                        if let Some(border) = view.border_at(pos, limits) {
                            press_on_header = true;
                            resize_drag = match border {
                                Header::Col(c) => Some(ResizeDrag {
                                    cols: true,
                                    index: c,
                                    start_px: layout.geo.col_px(c),
                                    origin: pos.x,
                                    px: layout.geo.col_px(c),
                                }),
                                Header::Row(r) => Some(ResizeDrag {
                                    cols: false,
                                    index: r,
                                    start_px: layout.geo.row_px(r),
                                    origin: pos.y,
                                    px: layout.geo.row_px(r),
                                }),
                                Header::Corner => None,
                            };
                        } else if let Some(header) = view.header_at(pos, limits) {
                            pressed = true;
                            press_on_header = true;
                            // Like Excel: the active cell stays; Shift grows from it.
                            match header {
                                Header::Col(c) => {
                                    if !modifiers.shift {
                                        sel.active.1 = c;
                                    }
                                    sel.anchor = sel.active;
                                    sel.extent = (sel.active.0, c);
                                    sel.reach = Reach::Cols;
                                }
                                Header::Row(r) => {
                                    if !modifiers.shift {
                                        sel.active.0 = r;
                                    }
                                    sel.anchor = sel.active;
                                    sel.extent = (r, sel.active.1);
                                    sel.reach = Reach::Rows;
                                }
                                Header::Corner => {
                                    sel.anchor = sel.active;
                                    sel.extent = sel.active;
                                    sel.reach = Reach::All;
                                }
                            }
                            sel.dragging = header != Header::Corner;
                        } else if let Some(cell) = view.cell_at(pos, false, limits) {
                            pressed = true;
                            press_on_header = false;
                            sel.reach = Reach::Cells;
                            if modifiers.shift {
                                sel.anchor = sel.active;
                                sel.extent = cell;
                            } else {
                                let anchor =
                                    layout.merge_at(cell.0, cell.1).map_or(cell, |m| (m.0, m.1));
                                sel.active = anchor;
                                sel.anchor = anchor;
                                sel.extent = anchor;
                            }
                            sel.dragging = true;
                        }
                    }
                    Event::PointerButton {
                        button: PointerButton::Primary,
                        pressed: false,
                        pos,
                        ..
                    } => {
                        sel.dragging = false;
                        if let Some(d) = resize_drag.take() {
                            let px = drag_size(&d, pos);
                            if px != d.start_px {
                                new_resizes.push(resize_request(
                                    &layout, sheet, selection, sel.reach, &d, px,
                                ));
                            }
                        }
                    }
                    Event::PointerMoved(pos) if resize_drag.is_some() => {
                        if let Some(d) = resize_drag.as_mut() {
                            d.px = drag_size(d, pos);
                        }
                    }
                    Event::PointerMoved(pos) if sel.dragging => drag_to(&mut sel, pos),
                    _ => {}
                }
            }
            // While dragging, the pointer decides the corner every frame, also when it is
            // held still outside the viewport and the sheet scrolls under it.
            if sel.dragging
                && let Some(pos) = ctx.input(|i| i.pointer.latest_pos())
            {
                drag_to(&mut sel, pos);
                let data_min = view_min + vec2(ROW_HEADER_W + fz.x, COL_HEADER_H + fz.y);
                let beyond = |p: f32, lo: f32, hi: f32| {
                    if p < lo {
                        p - lo
                    } else if p > hi {
                        p - hi
                    } else {
                        0.0
                    }
                };
                let dx = beyond(pos.x, data_min.x, clip_all.right());
                let dy = beyond(pos.y, data_min.y, clip_all.bottom());
                let (dx, dy) = match sel.reach {
                    Reach::Cells => (dx, dy),
                    Reach::Cols => (dx, 0.0),
                    Reach::Rows => (0.0, dy),
                    Reach::All => (0.0, 0.0),
                };
                if dx != 0.0 || dy != 0.0 {
                    let dt = ctx.input(|i| i.stable_dt).clamp(0.001, 0.1);
                    // Points per second grow with the distance past the edge.
                    let speed = |d: f32| d.signum() * (d.abs() * 12.0).clamp(60.0, 2400.0) * dt;
                    let step = |d: f32| if d == 0.0 { 0.0 } else { speed(d) };
                    // Scrolling the content left shows columns to the right.
                    ui.scroll_with_delta(-vec2(step(dx), step(dy)));
                }
            }
            if let Some(d) = resize_drag.as_mut()
                && let Some(pos) = ctx.input(|i| i.pointer.latest_pos())
            {
                d.px = drag_size(d, pos);
            }
            // The pointer over a header border (or dragging one) is a resize cursor.
            let pointer = ctx.input(|i| i.pointer.latest_pos());
            let hover_border = pointer
                .filter(|p| own_layer(*p))
                .and_then(|p| view.border_at(p, limits));
            match (resize_drag, hover_border) {
                (Some(d), _) if d.cols => ctx.set_cursor_icon(egui::CursorIcon::ResizeColumn),
                (Some(_), _) => ctx.set_cursor_icon(egui::CursorIcon::ResizeRow),
                (None, Some(Header::Col(_))) => {
                    ctx.set_cursor_icon(egui::CursorIcon::ResizeColumn);
                }
                (None, Some(_)) => ctx.set_cursor_icon(egui::CursorIcon::ResizeRow),
                (None, None) => {}
            }
            double_clicked = response.double_clicked();
            let fit_border = if double_clicked {
                response
                    .interact_pointer_pos()
                    .and_then(|p| view.border_at(p, limits))
            } else {
                None
            };

            // Regions: body, frozen rows, frozen columns, corner.
            let body_min = view_min + vec2(ROW_HEADER_W + fz.x, COL_HEADER_H + fz.y);
            let strip_x = Rect::from_min_max(
                Pos2::new(body_min.x, clip_all.top()),
                Pos2::new(clip_all.right(), clip_all.bottom()),
            );
            let strip_y = Rect::from_min_max(
                Pos2::new(clip_all.left(), body_min.y),
                Pos2::new(clip_all.right(), clip_all.bottom()),
            );
            let frozen_x = Rect::from_min_size(
                Pos2::new(view_min.x + ROW_HEADER_W, clip_all.top()),
                vec2(fz.x, clip_all.height()),
            );
            let frozen_y = Rect::from_min_size(
                Pos2::new(clip_all.left(), view_min.y + COL_HEADER_H),
                vec2(clip_all.width(), fz.y),
            );
            let mut regions = vec![Region {
                rows: (first_row, last_row),
                cols: (first_col, last_col),
                shift: Vec2::ZERO,
                clip: strip_x.intersect(strip_y),
            }];
            if frozen.rows > 0 {
                regions.push(Region {
                    rows: (1, visible.frozen_rows),
                    cols: (first_col, last_col),
                    shift: vec2(0.0, scroll.y),
                    clip: strip_x.intersect(frozen_y),
                });
            }
            if frozen.cols > 0 {
                regions.push(Region {
                    rows: (first_row, last_row),
                    cols: (1, visible.frozen_cols),
                    shift: vec2(scroll.x, 0.0),
                    clip: frozen_x.intersect(strip_y),
                });
            }
            if frozen.rows > 0 && frozen.cols > 0 {
                regions.push(Region {
                    rows: (1, visible.frozen_rows),
                    cols: (1, visible.frozen_cols),
                    shift: scroll,
                    clip: frozen_x.intersect(frozen_y),
                });
            }
            self.painted.borrow_mut().clear();
            let paint = Paint {
                wb,
                ws,
                view: &view,
                pal: &pal,
                fonts: &self.fonts,
                log: &self.painted,
                hints: &self.hints,
                ppp,
            };
            if let Some(border) = fit_border {
                new_resizes.extend(autofit_request(
                    &painter, &paint, sheet, border, selection, sel.reach,
                ));
            }
            for region in &regions {
                paint_region(&painter, &paint, region, selection, active);
            }
            paint_headers(&painter, &view, clip_all, &visible, &pal, selection, &font);
            if let Some(d) = resize_drag {
                paint_resize_guide(&painter, &view, clip_all, &layout, &pal, &d, ppp);
            }

            // The active cell is always a focusable widget so focus and the screen
            // reader's cursor have a node to sit on.
            let ar = layout
                .merge_at(active.0, active.1)
                .unwrap_or((active.0, active.1, active.0, active.1));
            ui.interact(
                view.screen_rect(ar),
                active_id,
                Sense::focusable_noninteractive(),
            );
        });
        self.visible = visible;

        if pressed && self.editing {
            self.commit(&ctx, target, None);
        }
        let moved = sel.active != self.active;
        // The active cell stays inside the selection.
        let a = sel.anchor;
        let e = sel.extent;
        let raw = match sel.reach {
            Reach::Cells => (a.0.min(e.0), a.1.min(e.1), a.0.max(e.0), a.1.max(e.1)),
            Reach::Cols => (1, a.1.min(e.1), MAX_ROW, a.1.max(e.1)),
            Reach::Rows => (a.0.min(e.0), 1, a.0.max(e.0), MAX_COL),
            Reach::All => (1, 1, MAX_ROW, MAX_COL),
        };
        if !contains(layout.expand(raw), sel.active.0, sel.active.1) {
            sel.active = sel.anchor;
        }
        self.active = sel.active;
        self.anchor = sel.anchor;
        self.extent = sel.extent;
        self.dragging = sel.dragging;
        self.reach = sel.reach;
        self.resize_drag = resize_drag;
        self.press_on_header = press_on_header;
        if pressed || moved {
            self.return_col = None;
            self.refocus_frame = Some(ctx.cumulative_frame_nr());
        }
        for r in new_resizes {
            self.status = Some(resize_notice(&r));
            if self.resizes.len() >= MAX_PENDING_RESIZES {
                self.resizes.remove(0);
            }
            self.resizes.push(r);
        }
        if self.dragging || self.resize_drag.is_some() {
            ctx.request_repaint();
        }
        if double_clicked && !self.press_on_header {
            self.status = None;
            let (r, c) = self.active;
            let text = target
                .xlsx()
                .map(|wb| Self::source_text(wb, self.sheet, r, c))
                .unwrap_or_default();
            self.start_edit(&ctx, text);
        }

        self.publish_accessibility(ui, target, &layout, (rows_n, cols_n));
        // After the cell widgets exist: a press elsewhere makes egui drop focus from them.
        if self
            .refocus_frame
            .is_some_and(|f| ctx.cumulative_frame_nr() <= f + 1)
            && !self.editing
        {
            self.focus_active(&ctx);
        }
    }

    fn clear_selection(&mut self, target: &mut Target) {
        let (r1, c1, r2, c2) = self.selection();
        let mut cells: Vec<(u32, u32)> = target
            .xlsx()
            .and_then(|wb| wb.sheet(self.sheet))
            .map(|ws| {
                let rows = ws.rows();
                let start = rows.partition_point(|r| r.r < r1);
                rows[start..]
                    .iter()
                    .take_while(|r| r.r <= r2)
                    .flat_map(|r| {
                        r.cells
                            .iter()
                            .filter(|c| (c1..=c2).contains(&c.col))
                            .filter(|c| !matches!(c.value, CellValue::Empty) || c.formula.is_some())
                            .map(|c| (r.r, c.col))
                    })
                    .take(MAX_CLEAR_CELLS)
                    .collect()
            })
            .unwrap_or_default();
        if cells.is_empty() {
            cells.push(self.active);
        }
        for (row, col) in cells {
            let command = Command::XlsxSetCellValue {
                sheet: self.sheet,
                row,
                col,
                value: CellEntry::Empty,
            };
            if let Err(e) = target.apply(command) {
                self.status = Some(e.to_string());
                return;
            }
        }
    }

    fn handle_keys(&mut self, ctx: &Context, target: &mut Target) {
        let events = ctx.input(|i| i.events.clone());
        // Shift+Space also sends a space as text, which must not start an edit.
        let mut swallow_space = false;
        for event in events {
            match event {
                Event::Text(t) if swallow_space && t == " " => swallow_space = false,
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
                    let shift = modifiers.shift;
                    let ctrl = modifiers.command;
                    match key {
                        Key::Space if modifiers.ctrl => {
                            let sel = self.selection();
                            self.select_columns(sel.1, sel.3);
                        }
                        Key::Space if shift => {
                            swallow_space = true;
                            let sel = self.selection();
                            self.select_rows(sel.0, sel.2);
                        }
                        Key::A if ctrl => self.select_all(),
                        Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight => {
                            let (d_row, d_col) = match key {
                                Key::ArrowUp => (-1, 0),
                                Key::ArrowDown => (1, 0),
                                Key::ArrowLeft => (0, -1),
                                _ => (0, 1),
                            };
                            match (ctrl, shift) {
                                // Ctrl+Shift+arrow: the moving corner to the edge of the data.
                                (true, true) => {
                                    let to =
                                        self.ctrl_arrow_target(target, self.extent, d_row, d_col);
                                    self.extend_to(to.0, to.1);
                                }
                                // Ctrl+arrow: the active cell to the edge of the data.
                                (true, false) => {
                                    let from = self.active;
                                    let to = self.ctrl_arrow_target(target, from, d_row, d_col);
                                    let to = self.anchor_of(to);
                                    self.set_active(to.0, to.1);
                                }
                                (false, true) => self.extend(d_row, d_col),
                                (false, false) => self.move_active(d_row, d_col),
                            }
                        }
                        Key::Enter => self.advance(Step::Enter(shift), false),
                        Key::Tab => self.advance(Step::Tab(shift), false),
                        Key::PageUp => self.page(false, shift),
                        Key::PageDown => self.page(true, shift),
                        Key::Home => match (ctrl, shift) {
                            (true, true) => {
                                let (r, c) = self.home_cell();
                                self.extend_to(r, c);
                            }
                            (true, false) => {
                                let (r, c) = self.home_cell();
                                self.set_active(r, c);
                            }
                            (false, true) => self.extend_to(self.extent.0, 1),
                            (false, false) => self.set_active(self.active.0, 1),
                        },
                        Key::End if ctrl => {
                            let (r, c) = self.end_cell(target);
                            if shift {
                                self.extend_to(r, c);
                            } else {
                                self.set_active(r, c);
                            }
                        }
                        Key::F2 => {
                            let text = target
                                .xlsx()
                                .map(|wb| {
                                    Self::source_text(wb, self.sheet, self.active.0, self.active.1)
                                })
                                .unwrap_or_default();
                            self.start_edit(ctx, text);
                        }
                        Key::Delete | Key::Backspace => self.clear_selection(target),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        // Keep focus on whichever cell is now active.
        if !self.editing {
            self.focus_active(ctx);
        }
    }

    /// Publishes Grid, Row and Cell nodes for the visible range plus the active cell. A merged
    /// cell is one Cell node with row and column spans; the cells it covers are not published.
    fn publish_accessibility(
        &mut self,
        ui: &mut Ui,
        target: &Target,
        layout: &SheetLayout,
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
        let vis = self.visible;
        let view = View {
            layout,
            view_min: vis.view_min,
            scroll: vis.scroll,
        };
        let selection = self.selection();
        let axis = |frozen: u32,
                    scrolled: (u32, u32),
                    active: Option<u32>,
                    hidden: &dyn Fn(u32) -> bool| {
            let mut list: Vec<u32> = (1..=frozen).chain(scrolled.0..=scrolled.1).collect();
            list.extend(active);
            list.sort_unstable();
            list.dedup();
            list.retain(|i| !hidden(*i));
            list
        };
        let row_list = axis(vis.frozen_rows, vis.rows, Some(self.active.0), &|r| {
            layout.geo.row_hidden(r)
        });
        let sheet = self.sheet;
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
            let row_bounds = view.screen_rect((r, 1, r, extent.1));
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
            let active_col = (r == self.active.0).then_some(self.active.1);
            let col_list = axis(vis.frozen_cols, vis.cols, active_col, &|c| {
                layout.geo.col_hidden(c)
            });
            for c in col_list {
                let merge = layout.merge_at(r, c);
                if merge.is_some_and(|m| (m.0, m.1) != (r, c)) {
                    continue;
                }
                let span = merge.unwrap_or((r, c, r, c));
                let cell_id = self.cell_id(r, c);
                let rect = view.screen_rect(span);
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
                let cell = wb.cell(sheet, r, c);
                let content = cell.map(|cell| display_text(wb, cell)).unwrap_or_default();
                let formula = cell
                    .is_some_and(|cell| cell.formula.is_some())
                    .then(|| wb.formula_text(sheet, r, c))
                    .flatten();
                let address = format!("{}{}", col_letters(c), r);
                let selected = contains(selection, r, c);
                cells_ui.ctx().accesskit_node_builder(cell_id, |b| {
                    b.set_role(accesskit::Role::Cell);
                    b.set_row_index((r - 1) as usize);
                    b.set_column_index((c - 1) as usize);
                    if span.2 > span.0 {
                        b.set_row_span((span.2 - span.0 + 1) as usize);
                    }
                    if span.3 > span.1 {
                        b.set_column_span((span.3 - span.1 + 1) as usize);
                    }
                    if selected {
                        b.set_selected(true);
                    }
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

/// The window position (scroll offset) that brings `cell` into view from `scroll`, moving as
/// little as possible: a cell above or left of the window goes to its edge, one below or right
/// of it to the opposite edge. Frozen rows and columns never scroll. All sums are in `f64`.
fn scroll_to_cell(layout: &SheetLayout, cell: (u32, u32), scroll: Vec2, view: Vec2) -> Vec2 {
    let f = layout.frozen;
    let span = layout
        .merge_at(cell.0, cell.1)
        .unwrap_or((cell.0, cell.1, cell.0, cell.1));
    let fz = (
        f64::from(layout.x(f.cols + 1)),
        f64::from(layout.y(f.rows + 1)),
    );
    let axis = |frozen: bool, lo: f64, hi: f64, now: f32, fz: f64, room: f64| {
        if frozen {
            return now;
        }
        let mut out = f64::from(now);
        if hi > out + room {
            out = hi - room;
        }
        if lo < out + fz {
            out = lo - fz;
        }
        out.max(0.0) as f32
    };
    vec2(
        axis(
            cell.1 <= f.cols,
            layout.x64(span.1),
            layout.x64(span.3 + 1),
            scroll.x,
            fz.0,
            f64::from((view.x - ROW_HEADER_W).max(0.0)),
        ),
        axis(
            cell.0 <= f.rows,
            layout.y64(span.0),
            layout.y64(span.2 + 1),
            scroll.y,
            fz.1,
            f64::from((view.y - COL_HEADER_H).max(0.0)),
        ),
    )
}

/// Whether a cell holds anything: a value or a formula (a styled blank does not count).
fn is_filled(cell: &Cell) -> bool {
    !matches!(cell.value, CellValue::Empty) || cell.formula.is_some()
}

/// Where Ctrl+arrow goes from `from`, as Excel's `Range.End` does, measured in
/// `fixtures/reference/xlsx/grid-nav`:
///
/// - hidden rows and columns do not exist for the jump (their content does not count and the
///   jump never stops on one);
/// - from a filled cell whose next cell is also filled, the jump ends at the last cell of that
///   run of filled cells;
/// - otherwise it ends at the next filled cell, or at the edge of the sheet when there is none;
/// - at the edge of the sheet it stays;
/// - merged cells are ordinary cells (only the top-left one holds a value).
fn data_edge(
    ws: Option<&Worksheet>,
    layout: &SheetLayout,
    from: (u32, u32),
    d_row: i32,
    d_col: i32,
) -> (u32, u32) {
    let Some(ws) = ws else { return from };
    let vertical = d_row != 0;
    let dir = if vertical {
        d_row.signum()
    } else {
        d_col.signum()
    };
    let (line, start) = if vertical {
        (from.1, from.0)
    } else {
        (from.0, from.1)
    };
    let max = if vertical { MAX_ROW } else { MAX_COL };
    let hidden = |i: u32| {
        if vertical {
            layout.geo.row_hidden(i)
        } else {
            layout.geo.col_hidden(i)
        }
    };
    let rows = ws.rows();
    // Cells of row `r` are looked up by column; of column `line`, by row.
    let filled = |i: u32| {
        if vertical {
            ws.cell(i, line)
        } else {
            ws.cell(line, i)
        }
        .is_some_and(is_filled)
    };
    // The first visible filled index beyond `from` in the direction of travel.
    let next_filled = |at: u32| -> Option<u32> {
        if vertical {
            let in_col = |row: &open365_xlsx::Row| {
                row.cells
                    .binary_search_by_key(&line, |c| c.col)
                    .ok()
                    .and_then(|k| row.cells.get(k))
                    .is_some_and(is_filled)
            };
            if dir > 0 {
                let k = rows.partition_point(|r| r.r <= at);
                rows[k..]
                    .iter()
                    .find(|r| !layout.geo.row_hidden(r.r) && in_col(r))
                    .map(|r| r.r)
            } else {
                let k = rows.partition_point(|r| r.r < at);
                rows[..k]
                    .iter()
                    .rev()
                    .find(|r| !layout.geo.row_hidden(r.r) && in_col(r))
                    .map(|r| r.r)
            }
        } else {
            let k = rows.binary_search_by_key(&line, |r| r.r).ok()?;
            let cells = &rows[k].cells;
            let ok = |c: &&Cell| !layout.geo.col_hidden(c.col) && is_filled(c);
            if dir > 0 {
                let k = cells.partition_point(|c| c.col <= at);
                cells[k..].iter().find(ok).map(|c| c.col)
            } else {
                let k = cells.partition_point(|c| c.col < at);
                cells[..k].iter().rev().find(ok).map(|c| c.col)
            }
        }
    };
    let step = |i: u32| layout.next_visible(vertical, i, dir, max);
    let next = step(start);
    let to = if next == start {
        start
    } else if filled(start) && filled(next) {
        let mut last = next;
        loop {
            let n = step(last);
            if n == last || !filled(n) || hidden(n) {
                break last;
            }
            last = n;
        }
    } else {
        next_filled(start).unwrap_or(if dir > 0 { max } else { 1 })
    };
    if vertical { (to, from.1) } else { (from.0, to) }
}

/// The request for a header border dragged to `px` device pixels. Dragging a border inside a
/// selection of whole columns (rows) resizes all of them, as Excel does.
fn resize_request(
    layout: &SheetLayout,
    sheet: usize,
    selection: Span,
    reach: Reach,
    drag: &ResizeDrag,
    px: u32,
) -> Resize {
    let metrics = layout.geo.metrics();
    if drag.cols {
        let whole = matches!(reach, Reach::Cols | Reach::All)
            && (selection.1..=selection.3).contains(&drag.index);
        let (first, last) = if whole {
            (selection.1, selection.3)
        } else {
            (drag.index, drag.index)
        };
        Resize::ColumnWidth {
            sheet,
            first,
            last,
            width: column_width_for_px(metrics, px),
        }
    } else {
        let whole = matches!(reach, Reach::Rows | Reach::All)
            && (selection.0..=selection.2).contains(&drag.index);
        let (first, last) = if whole {
            (selection.0, selection.2)
        } else {
            (drag.index, drag.index)
        };
        Resize::RowHeight {
            sheet,
            first,
            last,
            points: row_points_for_px(metrics, px),
        }
    }
}

/// The stored column width (characters, cell padding included) that is `px` pixels wide.
fn column_width_for_px(metrics: GridMetrics, px: u32) -> f64 {
    if px == 0 {
        return 0.0;
    }
    let mdw = f64::from(metrics.mdw);
    let mut width = (f64::from(px) / mdw * 256.0).floor() / 256.0;
    for _ in 0..8 {
        if metrics.col_px(width) >= px {
            break;
        }
        width += 1.0 / 256.0;
    }
    width
}

/// The row height in points that is `px` pixels tall.
fn row_points_for_px(metrics: GridMetrics, px: u32) -> f64 {
    let mut points = metrics.px_to_pt(px);
    for _ in 0..8 {
        if metrics.row_px(points) >= px {
            break;
        }
        points += 0.01;
    }
    points
}

/// Pixels of padding a column adds around its widest cell content, as Excel's auto-fit does:
/// the cell padding of [`number_inset_px`] and two more (measured at 120 dpi, where it is 9).
fn fit_padding_px(ppp: f32) -> f32 {
    number_inset_px(ppp) + 2.0
}

/// The widest content, in pixels, each column of `first..=last` holds (columns with no
/// content are absent). Merged cells do not count, as in Excel.
fn content_widths(
    painter: &Painter,
    paint: &Paint<'_>,
    first: u32,
    last: u32,
) -> std::collections::BTreeMap<u32, f32> {
    let mut widest = std::collections::BTreeMap::new();
    let Some(ws) = paint.ws else { return widest };
    let layout = paint.view.layout;
    let styles = paint.wb.styles();
    for row in ws.rows() {
        let start = row.cells.partition_point(|c| c.col < first);
        for cell in row.cells[start..].iter().take_while(|c| c.col <= last) {
            if matches!(cell.value, CellValue::Empty) || layout.merge_at(row.r, cell.col).is_some()
            {
                continue;
            }
            let fmt = styles.cell_format(cell.style.unwrap_or(0));
            let w = content_px(painter, paint, cell, fmt);
            let entry = widest.entry(cell.col).or_insert(0.0f32);
            *entry = entry.max(w);
        }
    }
    widest
}

/// Pixels the content of one cell needs on one line.
fn content_px(painter: &Painter, paint: &Paint<'_>, cell: &Cell, fmt: &CellFormat) -> f32 {
    let cf = cell_font(paint, &fmt.font);
    let hinted = cf.hinted.as_deref();
    let ppp = paint.ppp;
    let mut text = display_text(paint.wb, cell);
    let mut general_number = false;
    if let CellValue::Number(n) = &cell.value {
        let code = format_code(paint.wb, cell.style.unwrap_or(0));
        let general = code == "General" || code.is_empty();
        general_number = general;
        if general && text.chars().count() > 11 {
            // General shows at most 11 characters, in scientific notation if it must.
            let one = text_px(painter, hinted, "0", &cf.id, ppp);
            let mut measure = |s: &str| text_px(painter, hinted, s, &cf.id, ppp);
            text = fit_number(*n, text, true, one * 11.0, &mut measure).into_string();
        }
    }
    let align = &fmt.alignment;
    let mut width = match align.rotation_degrees() {
        // Rotated up or down: the text's height is its width on the row.
        Some(90 | -90) => line_height_px(paint, &cf).unwrap_or(cf.ppem) as f32,
        // Wrapped text needs its longest run between spaces (a line break is not a break
        // point for this: Excel measures the run across it with the break itself zero-wide).
        _ if align.wrap_text && !is_numeric(&cell.value) => text
            .replace('\n', "")
            .split(' ')
            .map(|word| text_px(painter, hinted, word, &cf.id, ppp))
            .fold(0.0, f32::max),
        _ => text
            .split('\n')
            .map(|line| {
                // General counts the decimal point as wide as a digit.
                if general_number && line.contains('.') {
                    text_px(painter, hinted, &line.replace('.', "0"), &cf.id, ppp)
                } else {
                    text_px(painter, hinted, line, &cf.id, ppp)
                }
            })
            .fold(0.0, f32::max),
    };
    if fmt.font.italic {
        // Slanted glyphs overhang their advance.
        width += (cf.ppem as f32 * 0.4).round();
    }
    if matches!(
        align.horizontal,
        HorizontalAlignment::Left | HorizontalAlignment::Right | HorizontalAlignment::Distributed
    ) {
        let space = text_px(painter, hinted, " ", &cf.id, ppp);
        width += align.indent as f32 * 3.0 * space;
    }
    width
}

/// Height in pixels of one line of text in `cf`, as a row holds it.
fn line_height_px(paint: &Paint<'_>, cf: &CellFont) -> Option<u32> {
    let data = paint.fonts.borrow().font_data(&cf.face.family)?;
    line_px(&data, cf.ppem)
}

/// Requests for auto-fitting the border `border` was double-clicked on.
fn autofit_request(
    painter: &Painter,
    paint: &Paint<'_>,
    sheet: usize,
    border: Header,
    selection: Span,
    reach: Reach,
) -> Vec<Resize> {
    match border {
        Header::Col(c) => {
            let whole = matches!(reach, Reach::Cols | Reach::All)
                && (selection.1..=selection.3).contains(&c);
            let (first, last) = if whole {
                (selection.1, selection.3)
            } else {
                (c, c)
            };
            fit_columns(painter, paint, sheet, first, last, c)
        }
        Header::Row(r) => {
            let whole = matches!(reach, Reach::Rows | Reach::All)
                && (selection.0..=selection.2).contains(&r);
            let (first, last) = if whole {
                (selection.0, selection.2)
            } else {
                (r, r)
            };
            fit_rows(painter, paint, sheet, first, last, r)
        }
        Header::Corner => Vec::new(),
    }
}

/// Auto-fit requests for columns `first..=last`: each column with content gets the width of
/// its widest cell; `clicked`, if it has none, gets the sheet's default width, as in Excel.
fn fit_columns(
    painter: &Painter,
    paint: &Paint<'_>,
    sheet: usize,
    first: u32,
    last: u32,
    clicked: u32,
) -> Vec<Resize> {
    let layout = paint.view.layout;
    let metrics = layout.geo.metrics();
    let mut widths: Vec<(u32, u32)> = content_widths(painter, paint, first, last)
        .into_iter()
        .map(|(col, w)| {
            let px = (w + fit_padding_px(paint.ppp)).ceil() as u32;
            (col, px.min(metrics.col_px(MAX_COL_WIDTH_CHARS)))
        })
        .collect();
    if !widths.iter().any(|(c, _)| *c == clicked) {
        let format = paint.ws.map(Worksheet::sheet_format).unwrap_or_default();
        widths.push((clicked, metrics.default_col_px(&format)));
    }
    widths
        .into_iter()
        .map(|(col, px)| Resize::ColumnWidth {
            sheet,
            first: col,
            last: col,
            width: column_width_for_px(metrics, px),
        })
        .collect()
}

/// Auto-fit requests for rows `first..=last`: the tallest line of any cell with content
/// times its line count, never less than a row of the default font.
fn fit_rows(
    painter: &Painter,
    paint: &Paint<'_>,
    sheet: usize,
    first: u32,
    last: u32,
    clicked: u32,
) -> Vec<Resize> {
    let layout = paint.view.layout;
    let metrics = layout.geo.metrics();
    let styles = paint.wb.styles();
    let default_px = default_row_px(paint);
    let mut out = Vec::new();
    let mut seen_clicked = false;
    if let Some(ws) = paint.ws {
        let rows = ws.rows();
        let start = rows.partition_point(|r| r.r < first);
        for row in rows[start..].iter().take_while(|r| r.r <= last) {
            seen_clicked |= row.r == clicked;
            let mut px = default_px;
            for cell in &row.cells {
                if matches!(cell.value, CellValue::Empty)
                    || layout
                        .merge_at(row.r, cell.col)
                        .is_some_and(|m| m.0 != m.2 || m.1 != m.3)
                {
                    continue;
                }
                let fmt = styles.cell_format(cell.style.unwrap_or(0));
                px = px.max(cell_height_px(painter, paint, layout, row.r, cell, fmt));
            }
            out.push((row.r, px));
        }
    }
    if !seen_clicked {
        out.push((clicked, default_px));
    }
    out.into_iter()
        .map(|(row, px)| Resize::RowHeight {
            sheet,
            first: row,
            last: row,
            points: row_points_for_px(metrics, px),
        })
        .collect()
}

/// Pixel height of a row of the workbook's default font.
fn default_row_px(paint: &Paint<'_>) -> u32 {
    let font = paint.wb.styles().cell_format(0).font.clone();
    let cf = cell_font(paint, &font);
    line_height_px(paint, &cf).map_or(20, row_px_for_line)
}

/// A row that holds one line of `line` pixels: GDI's text height plus the row's own padding,
/// which grows with the font (measured against Excel auto-fit at 120 dpi).
fn row_px_for_line(line: u32) -> u32 {
    line + 2 + line / 36
}

/// Pixel height a cell asks of its row: its line count times a line of its font.
fn cell_height_px(
    painter: &Painter,
    paint: &Paint<'_>,
    layout: &SheetLayout,
    row: u32,
    cell: &Cell,
    fmt: &CellFormat,
) -> u32 {
    let cf = cell_font(paint, &fmt.font);
    let line = line_height_px(paint, &cf).map_or(20, row_px_for_line);
    let text = display_text(paint.wb, cell);
    let wrap = fmt.alignment.wrap_text && !is_numeric(&cell.value);
    let lines = if wrap {
        let width = (layout.w(cell.col) * paint.ppp).round() - fit_padding_px(paint.ppp);
        wrapped_lines(painter, &cf, &text, width.max(1.0) / paint.ppp)
    } else {
        1
    };
    let _ = row;
    line * lines as u32
}

/// Lines a text takes when wrapped to `width` points.
fn wrapped_lines(painter: &Painter, cf: &CellFont, text: &str, width: f32) -> usize {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: cf.id.clone(),
            ..TextFormat::default()
        },
    );
    painter.layout_job(job).rows.len().max(1)
}

/// The text Excel shows while a border is dragged: `Width: 8.43 (64 pixels)`.
fn resize_label(metrics: GridMetrics, cols: bool, px: u32) -> String {
    if cols {
        let chars = if px == 0 {
            0.0
        } else {
            (f64::from(px.saturating_sub(metrics.padding())) / f64::from(metrics.mdw) * 100.0 + 0.5)
                .floor()
                / 100.0
        };
        format!("Width: {chars:.2} ({px} pixels)")
    } else {
        format!("Height: {:.2} ({px} pixels)", metrics.px_to_pt(px))
    }
}

fn resize_notice(r: &Resize) -> String {
    match *r {
        Resize::ColumnWidth {
            first, last, width, ..
        } => {
            let cols = if first == last {
                format!("column {}", col_letters(first))
            } else {
                format!("columns {}:{}", col_letters(first), col_letters(last))
            };
            format!(
                "Width of {cols} set to {width:.2}: applying it needs the SetColumnWidth command"
            )
        }
        Resize::RowHeight {
            first,
            last,
            points,
            ..
        } => {
            let rows = if first == last {
                format!("row {first}")
            } else {
                format!("rows {first}:{last}")
            };
            format!(
                "Height of {rows} set to {points:.2} pt: applying it needs the SetRowHeight command"
            )
        }
    }
}

/// The guide line and size label drawn while a header border is dragged.
fn paint_resize_guide(
    painter: &Painter,
    view: &View<'_>,
    clip_all: Rect,
    layout: &SheetLayout,
    pal: &Palette,
    drag: &ResizeDrag,
    ppp: f32,
) {
    let f = layout.frozen;
    let size = drag.px as f32 / ppp;
    let stroke = Stroke::new(1.0, pal.accent);
    let metrics = layout.geo.metrics();
    let label = resize_label(metrics, drag.cols, drag.px);
    let at = if drag.cols {
        let shift = if drag.index <= f.cols {
            view.scroll.x
        } else {
            0.0
        };
        let x = view.edge_x(drag.index, shift) + size;
        painter.vline(x, clip_all.y_range(), stroke);
        Pos2::new(x + 6.0, clip_all.top() + COL_HEADER_H + 4.0)
    } else {
        let shift = if drag.index <= f.rows {
            view.scroll.y
        } else {
            0.0
        };
        let y = view.edge_y(drag.index, shift) + size;
        painter.hline(clip_all.x_range(), y, stroke);
        Pos2::new(clip_all.left() + ROW_HEADER_W + 6.0, y + 6.0)
    };
    let galley = painter.layout_no_wrap(label, FontId::proportional(12.0), pal.text);
    let r = Rect::from_min_size(at, galley.size()).expand(3.0);
    painter.rect_filled(r, 2.0, pal.cell_bg);
    painter.rect_stroke(r, 2.0, stroke, egui::StrokeKind::Inside);
    painter.galley(at, galley, pal.text);
}

/// What the grid drew for one cell: the text after fitting (`####`, shrunk General numbers,
/// fill repeats), the space it was clipped to and the box it occupies. Kept for the last
/// frame so tests and diagnostics can check text behaviour without reading pixels.
#[derive(Debug, Clone)]
pub struct PaintedCell {
    pub row: u32,
    pub col: u32,
    /// The text as drawn; a wrapped cell holds all its lines joined with `\n`.
    pub text: String,
    /// The text did not fit and `#` characters are shown.
    pub hashes: bool,
    /// Where the text may be drawn: the cell, widened over empty neighbours when it overflows.
    pub clip: Rect,
    /// The cell's own rectangle.
    pub cell: Rect,
    /// The rectangle the text occupies (before clipping).
    pub bounds: Rect,
    /// The alignment after resolving General.
    pub horizontal: HorizontalAlignment,
    pub lines: usize,
}

/// Everything a painting pass needs besides the painter.
struct Paint<'a> {
    wb: &'a Workbook,
    ws: Option<&'a Worksheet>,
    view: &'a View<'a>,
    pal: &'a Palette,
    fonts: &'a RefCell<FontBook>,
    log: &'a RefCell<Vec<PaintedCell>>,
    hints: &'a RefCell<HintCache>,
    ppp: f32,
}

/// A cell to draw: its rectangle (the whole merge for a merged cell) and format.
struct Item<'a> {
    row: u32,
    col: u32,
    /// Last column and row of the merge, or the cell itself.
    end: (u32, u32),
    rect: Rect,
    cell: &'a Cell,
    fmt: &'a CellFormat,
    merged: bool,
}

/// Mixes the pattern colour into the background for a patterned fill.
fn pattern_density(p: FillPattern) -> f32 {
    match p {
        FillPattern::Solid => 1.0,
        FillPattern::DarkGray => 0.75,
        FillPattern::MediumGray => 0.5,
        FillPattern::LightGray => 0.25,
        FillPattern::Gray125 => 0.125,
        FillPattern::Gray0625 => 0.0625,
        FillPattern::DarkHorizontal
        | FillPattern::DarkVertical
        | FillPattern::DarkDown
        | FillPattern::DarkUp
        | FillPattern::DarkGrid
        | FillPattern::DarkTrellis => 0.5,
        FillPattern::LightHorizontal
        | FillPattern::LightVertical
        | FillPattern::LightDown
        | FillPattern::LightUp
        | FillPattern::LightGrid
        | FillPattern::LightTrellis => 0.25,
        FillPattern::None => 0.0,
    }
}

fn fill_color(styles: &Styles, fill: &Fill) -> Option<Color32> {
    if fill.pattern == FillPattern::None {
        return None;
    }
    let fg = color32(styles.resolve_color(&fill.fg));
    if fill.pattern == FillPattern::Solid {
        return Some(fg);
    }
    let bg = color32(styles.resolve_color(&fill.bg));
    Some(Color32::from_rgb(
        mix(bg.r(), fg.r(), pattern_density(fill.pattern)),
        mix(bg.g(), fg.g(), pattern_density(fill.pattern)),
        mix(bg.b(), fg.b(), pattern_density(fill.pattern)),
    ))
}

fn mix(a: u8, b: u8, t: f32) -> u8 {
    (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8
}

/// The colour of text: automatic and theme "dark 1" follow the application's text colour so the
/// grid stays readable in a dark theme; every other colour is drawn as the file says.
fn text_color(styles: &Styles, c: &Color, pal: &Palette) -> Color32 {
    match c.source {
        ColorSource::Auto => pal.text,
        ColorSource::Theme(1) if c.tint == 0.0 => pal.text,
        _ => color32(styles.resolve_color(c)),
    }
}

/// Draws one region: background, grid lines, fills, text, borders and the selection.
fn paint_region(
    painter: &Painter,
    paint: &Paint<'_>,
    region: &Region,
    selection: Span,
    active: (u32, u32),
) {
    let (wb, ws, view, pal) = (paint.wb, paint.ws, paint.view, paint.pal);
    if region.rows.0 > region.rows.1 || region.cols.0 > region.cols.1 || !region.clip.is_positive()
    {
        return;
    }
    let p = painter.with_clip_rect(region.clip.intersect(painter.clip_rect()));
    let layout = view.layout;
    let geo = &layout.geo;
    let styles = wb.styles();
    let block = (region.rows.0, region.cols.0, region.rows.1, region.cols.1);
    let area = view.rect(block, region.shift);
    p.rect_filled(area, 0.0, pal.cell_bg);
    let line = Stroke::new(1.0 / paint.ppp, pal.grid_line);
    for r in region.rows.0..=region.rows.1 + 1 {
        let y = view.edge_y(r, region.shift.y) - 0.5 / paint.ppp;
        p.hline(area.x_range(), y, line);
    }
    for c in region.cols.0..=region.cols.1 + 1 {
        let x = view.edge_x(c, region.shift.x) - 0.5 / paint.ppp;
        p.vline(x, area.y_range(), line);
    }

    // The cells to draw: stored cells in the block, merges as one cell.
    let merges = layout.merges_overlapping(region.rows, region.cols);
    let mut items: Vec<Item<'_>> = Vec::new();
    if let Some(ws) = ws {
        let rows = ws.rows();
        let start = rows.partition_point(|r| r.r < region.rows.0);
        for row in rows[start..].iter().take_while(|r| r.r <= region.rows.1) {
            if geo.row_hidden(row.r) {
                continue;
            }
            let cstart = row.cells.partition_point(|c| c.col < region.cols.0);
            for cell in row.cells[cstart..]
                .iter()
                .take_while(|c| c.col <= region.cols.1)
            {
                if geo.col_hidden(cell.col) || layout.merge_at(row.r, cell.col).is_some() {
                    continue;
                }
                items.push(Item {
                    row: row.r,
                    col: cell.col,
                    end: (row.r, cell.col),
                    rect: view.rect((row.r, cell.col, row.r, cell.col), region.shift),
                    cell,
                    fmt: styles.cell_format(cell.style.unwrap_or(0)),
                    merged: false,
                });
            }
        }
        for m in &merges {
            if let Some(cell) = ws.cell(m.0, m.1) {
                items.push(Item {
                    row: m.0,
                    col: m.1,
                    end: (m.2, m.3),
                    rect: view.rect(*m, region.shift),
                    cell,
                    fmt: styles.cell_format(cell.style.unwrap_or(0)),
                    merged: true,
                });
            }
        }
    }
    // A merged cell is one cell: it covers the grid lines inside it.
    for m in &merges {
        let r = view.rect(*m, region.shift);
        if r.is_positive() {
            p.rect_filled(r, 0.0, pal.cell_bg);
        }
    }
    for item in &items {
        if let Some(color) = fill_color(styles, &item.fmt.fill) {
            p.rect_filled(item.rect, 0.0, color);
        }
    }
    for m in &merges {
        let r = view.rect(*m, region.shift);
        let has_fill = ws.and_then(|ws| ws.cell(m.0, m.1)).is_some_and(|c| {
            fill_color(styles, &styles.cell_format(c.style.unwrap_or(0)).fill).is_some()
        });
        if r.is_positive() && !has_fill {
            p.rect_stroke(r, 0.0, line, egui::StrokeKind::Inside);
        }
    }
    for item in &items {
        paint_cell_text(&p, paint, item, region);
    }
    if let Some(ws) = ws {
        for item in &items {
            paint_cell_borders(&p, paint, ws, item);
        }
    }

    // Selection: tinted range, outlined, with the active cell (or its merge) outlined too.
    let overlaps = selection.0 <= region.rows.1
        && selection.2 >= region.rows.0
        && selection.1 <= region.cols.1
        && selection.3 >= region.cols.0;
    if overlaps {
        let r = view.rect(selection, region.shift);
        let multi = (selection.0, selection.1) != (selection.2, selection.3);
        if multi {
            p.rect_filled(r, 0.0, pal.accent.gamma_multiply(0.15));
            p.rect_stroke(
                r,
                0.0,
                Stroke::new(2.0, pal.accent),
                egui::StrokeKind::Inside,
            );
        }
        if contains(
            (region.rows.0, region.cols.0, region.rows.1, region.cols.1),
            active.0,
            active.1,
        ) {
            let span = layout
                .merge_at(active.0, active.1)
                .unwrap_or((active.0, active.1, active.0, active.1));
            let ar = view.rect(span, region.shift);
            p.rect_stroke(
                ar,
                0.0,
                Stroke::new(2.0, pal.accent),
                egui::StrokeKind::Inside,
            );
        }
    }
}

/// A cell's font as the grid draws and measures it.
struct CellFont {
    face: Face,
    id: FontId,
    /// Font size in egui points at 100% zoom.
    base_size: f32,
    /// GDI font height: whole pixels at the display dpi.
    ppem: u32,
    /// Advances from the font's hinting program at `ppem`.
    hinted: Option<Rc<HintedFace>>,
}

fn cell_font(paint: &Paint<'_>, font: &open365_xlsx::Font) -> CellFont {
    let face = paint
        .fonts
        .borrow_mut()
        .resolve(&font.name, font.bold, font.italic);
    // Excel sizes are points at 72 per inch; egui points are 1/96 inch at ppp 1.
    let base_size = (font.size * 96.0 / 72.0) as f32;
    let id = FontId::new(base_size, face.family.clone());
    let ppem = (font.size * f64::from(BASE_DPI * paint.ppp) / 72.0)
        .round()
        .max(1.0) as u32;
    let hinted = {
        let data = paint.fonts.borrow().font_data(&face.family);
        data.and_then(|d| paint.hints.borrow_mut().face(&d, ppem))
    };
    CellFont {
        face,
        id,
        base_size,
        ppem,
        hinted,
    }
}

/// Pixels of a column that never hold number text: Excel's cell padding, 5 px at 96 dpi
/// growing with the display dpi (7 px at 120 dpi, probed with `Range.Text` over column
/// widths), so a column of `n` pixels fits a number up to `n - 7` pixels wide there.
fn number_inset_px(ppp: f32) -> f32 {
    (5.0 * ppp - 0.001).ceil()
}

/// Width of `text` in device pixels with `font`. Excel measures with GDI's hinted advances at
/// the display dpi, so `hinted` (the font's TrueType hinting at the pixel size) decides when it
/// has every glyph; otherwise the sum of egui's glyph advances, each rounded to whole pixels.
fn text_px(
    painter: &Painter,
    hinted: Option<&HintedFace>,
    text: &str,
    font: &FontId,
    ppp: f32,
) -> f32 {
    if let Some(w) = hinted.and_then(|h| h.width(text)) {
        return w;
    }
    let galley = painter.layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE);
    galley.rows.first().map_or(0.0, |row| {
        row.glyphs
            .iter()
            .map(|g| (g.advance_width * ppp).round())
            .sum()
    })
}

/// The text of a cell as shown, and whether it is a row of `#`.
fn shown_text(
    painter: &Painter,
    paint: &Paint<'_>,
    item: &Item<'_>,
    font: &FontId,
    hinted: Option<&HintedFace>,
    avail_px: f32,
) -> (String, bool) {
    let wb = paint.wb;
    let full = display_text(wb, item.cell);
    match &item.cell.value {
        CellValue::Number(n) => {
            let code = format_code(wb, item.cell.style.unwrap_or(0));
            let general = code == "General" || code.is_empty();
            // General counts the decimal point as wide as a digit when it decides what fits.
            let mut measure = |s: &str| {
                if general && s.contains('.') {
                    text_px(painter, hinted, &s.replace('.', "0"), font, paint.ppp)
                } else {
                    text_px(painter, hinted, s, font, paint.ppp)
                }
            };
            match fit_number(*n, full, general, avail_px, &mut measure) {
                NumberText::Text(t) => (t, false),
                NumberText::Hashes(t) => (t, true),
            }
        }
        CellValue::Error(_) => {
            if text_px(painter, hinted, &full, font, paint.ppp) <= avail_px {
                (full, false)
            } else {
                let one = text_px(painter, hinted, "#", font, paint.ppp).max(1.0);
                ("#".repeat(((avail_px / one).floor() as usize).max(1)), true)
            }
        }
        _ => (full, false),
    }
}

fn paint_cell_text(painter: &Painter, paint: &Paint<'_>, item: &Item<'_>, region: &Region) {
    let r = item.rect;
    if !r.is_positive() || matches!(item.cell.value, CellValue::Empty) {
        return;
    }
    let (wb, view, pal, ppp) = (paint.wb, paint.view, paint.pal, paint.ppp);
    let layout = view.layout;
    let styles = wb.styles();
    let fmt = item.fmt;
    let font = &fmt.font;
    let CellFont {
        face,
        id: mut font_id,
        base_size,
        hinted,
        ..
    } = cell_font(paint, font);
    let color = text_color(styles, &font.color, pal);
    let px = |v: f32| v / ppp;
    let hinted = hinted.as_deref();
    let avail_px = (r.width() * ppp).round() - number_inset_px(ppp);
    let (text, hashes) = shown_text(painter, paint, item, &font_id, hinted, avail_px);
    if text.is_empty() {
        return;
    }
    let is_text = !is_numeric(&item.cell.value) && !matches!(item.cell.value, CellValue::Error(_));
    let align = &fmt.alignment;
    let h = effective_horizontal(align.horizontal, &item.cell.value);
    let wrap = align.wrap_text && is_text && !hashes;
    let rotation = if wrap {
        Some(0)
    } else {
        align.rotation_degrees()
    };

    // Indent: three space widths per step, on the aligned side.
    let space = text_px(painter, hinted, " ", &font_id, ppp) / ppp;
    let indent = align.indent as f32 * 3.0 * space;
    let mut inner = Rect::from_min_max(
        Pos2::new(r.left() + px(2.0), r.top()),
        Pos2::new(r.right() - px(2.0) - px(1.0), r.bottom()),
    );
    match h {
        HorizontalAlignment::Left | HorizontalAlignment::Distributed => inner.min.x += indent,
        HorizontalAlignment::Right => inner.max.x -= indent,
        _ => {}
    }
    let mut text = text;
    if h == HorizontalAlignment::Fill && is_text {
        let mut measure = |s: &str| text_px(painter, hinted, s, &font_id, ppp);
        text = fill_text(&text, inner.width() * ppp, &mut measure);
    }

    let format = TextFormat {
        font_id: font_id.clone(),
        color,
        underline: if font.underline == Underline::None {
            Stroke::NONE
        } else {
            Stroke::new(1.0 / ppp, color)
        },
        strikethrough: if font.strike {
            Stroke::new(1.0 / ppp, color)
        } else {
            Stroke::NONE
        },
        ..TextFormat::default()
    };
    let mut job = LayoutJob::default();
    job.wrap.max_width = if wrap {
        inner.width().max(1.0)
    } else {
        f32::INFINITY
    };
    if rotation.is_none() {
        // Stacked vertical text: one character per line.
        text = text
            .chars()
            .map(String::from)
            .collect::<Vec<_>>()
            .join("\n");
    }
    job.halign = match h {
        HorizontalAlignment::Right => Align::Max,
        HorizontalAlignment::Center | HorizontalAlignment::CenterContinuous => Align::Center,
        _ => Align::Min,
    };
    job.justify = wrap
        && matches!(
            h,
            HorizontalAlignment::Justify | HorizontalAlignment::Distributed
        );
    job.append(&text, 0.0, format.clone());
    let mut galley = painter.layout_job(job.clone());

    // Shrink to fit: scale the font down until one line fits.
    if align.shrink_to_fit && !wrap && galley.size().x > inner.width() && galley.size().x > 0.0 {
        let scale = (inner.width() / galley.size().x).clamp(0.1, 1.0);
        font_id = FontId::new(base_size * scale, face.family.clone());
        let mut shrunk = TextFormat {
            font_id: font_id.clone(),
            ..format.clone()
        };
        shrunk.color = color;
        let mut j = LayoutJob::default();
        j.wrap.max_width = f32::INFINITY;
        j.halign = job.halign;
        j.append(&text, 0.0, shrunk);
        galley = painter.layout_job(j);
    }

    let size = galley.size();
    let cell_rect = r;
    // Where the text may spread: over empty neighbours when it overflows its cell.
    let mut clip = cell_rect;
    let mut area = inner;
    // Overflow is decided with GDI's hinted width of a single line, as Excel does.
    let text_w = match (hinted, rotation, galley.rows.len()) {
        (Some(h), Some(0), 1) => h.width(galley.text()).map_or(size.x, |w| w / ppp),
        _ => size.x,
    };
    let overflows = text_w > inner.width() + 0.01;
    let can_overflow =
        is_text && !wrap && !align.shrink_to_fit && !item.merged && rotation == Some(0);
    if h == HorizontalAlignment::CenterContinuous {
        // Centre across the following empty cells that are also centred across selection.
        if let Some(ws) = paint.ws {
            let mut last = item.end.1;
            while last < MAX_COL {
                let next = last + 1;
                let blank = ws
                    .cell(item.row, next)
                    .is_none_or(|c| matches!(c.value, CellValue::Empty) && c.formula.is_none());
                let st = ws.resolved_style(item.row, next, &layout.col_styles);
                if blank
                    && styles.cell_format(st).alignment.horizontal
                        == HorizontalAlignment::CenterContinuous
                {
                    last = next;
                } else {
                    break;
                }
            }
            if last > item.end.1 {
                let extra = layout.x(last + 1) - layout.x(item.end.1 + 1);
                clip.max.x += extra;
                area.max.x += extra;
            }
        }
    } else if can_overflow
        && overflows
        && let Some(ws) = paint.ws
    {
        let is_empty = |c: u32| {
            ws.cell(item.row, c)
                .is_none_or(|x| matches!(x.value, CellValue::Empty) && x.formula.is_none())
                && layout.merge_at(item.row, c).is_none()
        };
        let width = |c: u32| layout.w(c);
        let need = text_w - inner.width() + px(TEXT_INSET_PX);
        match h {
            HorizontalAlignment::Right => {
                let room = overflow_room(item.col, -1, MAX_COL, need, &is_empty, &width);
                clip.min.x -= room.min(need);
                area.min.x -= room.min(need);
            }
            HorizontalAlignment::Center => {
                let each = need / 2.0;
                let l = overflow_room(item.col, -1, MAX_COL, each, &is_empty, &width).min(each);
                let rr = overflow_room(item.end.1, 1, MAX_COL, each, &is_empty, &width).min(each);
                clip.min.x -= l;
                clip.max.x += rr;
                area.min.x -= l;
                area.max.x += rr;
            }
            _ => {
                let room = overflow_room(item.end.1, 1, MAX_COL, need, &is_empty, &width);
                clip.max.x += room.min(need);
                area.max.x += room.min(need);
            }
        }
    }

    // Position.
    let pad_y = px(1.0);
    let y = match align.vertical {
        VerticalAlignment::Top | VerticalAlignment::Justify => r.top() + pad_y,
        VerticalAlignment::Center | VerticalAlignment::Distributed => r.center().y - size.y / 2.0,
        VerticalAlignment::Bottom => r.bottom() - size.y - pad_y,
    };
    let x = match h {
        HorizontalAlignment::Right => area.right() - size.x,
        HorizontalAlignment::Center | HorizontalAlignment::CenterContinuous => {
            if overflows && !can_overflow && h == HorizontalAlignment::Center {
                inner.left()
            } else {
                area.center().x - size.x / 2.0
            }
        }
        _ => area.left(),
    };
    let wrap_x = if wrap { area.left() } else { x };
    let pos = Pos2::new(wrap_x, y);
    let clipped = painter.with_clip_rect(clip.intersect(painter.clip_rect()));
    let bounds = Rect::from_min_size(pos, size);
    match rotation {
        Some(0) => {
            clipped.galley(pos, galley.clone(), color);
            if font.bold && !face.real_bold {
                clipped.galley(pos + vec2(px(1.0), 0.0), galley.clone(), color);
            }
        }
        deg => {
            // Rotated (or stacked) text is centred in the cell.
            let top_left = r.center() - size / 2.0;
            let angle = -(deg.unwrap_or(0) as f32).to_radians();
            let shape = egui::epaint::TextShape::new(top_left, galley.clone(), color)
                .with_angle_and_anchor(angle, egui::Align2::CENTER_CENTER);
            painter
                .with_clip_rect(cell_rect.intersect(painter.clip_rect()))
                .add(shape);
        }
    }
    paint.log.borrow_mut().push(PaintedCell {
        row: item.row,
        col: item.col,
        text: galley.text().to_owned(),
        hashes,
        clip,
        cell: cell_rect,
        bounds,
        horizontal: h,
        lines: galley.rows.len().max(1),
    });
    let _ = region;
}

/// Pixel dash pattern `(on, off)` lengths for a border style, empty for a solid line.
fn dash_pattern(style: BorderStyle) -> &'static [f32] {
    match style {
        BorderStyle::Dashed => &[3.0, 1.0],
        BorderStyle::MediumDashed => &[3.0, 1.0],
        BorderStyle::Dotted | BorderStyle::Hair => &[1.0, 1.0],
        BorderStyle::DashDot | BorderStyle::MediumDashDot | BorderStyle::SlantDashDot => {
            &[3.0, 1.0, 1.0, 1.0]
        }
        BorderStyle::DashDotDot | BorderStyle::MediumDashDotDot => &[3.0, 1.0, 1.0, 1.0, 1.0, 1.0],
        _ => &[],
    }
}

/// Draws one border edge along the boundary between two cells. `horizontal` edges run along x
/// from `from` to `to` at the boundary `at`; vertical ones run along y. Positions are points.
#[allow(clippy::too_many_arguments)]
fn draw_edge(
    painter: &Painter,
    horizontal: bool,
    at: f32,
    from: f32,
    to: f32,
    edge: &BorderEdge,
    color: Color32,
    ppp: f32,
) {
    let width = edge.style.width_px();
    if width == 0 || to <= from {
        return;
    }
    // A line of `width` px covers the pixels [at - ceil(w/2), at + floor(w/2)), so a 1 px line
    // sits on the grid line pixel just before the boundary.
    let lines: &[(f32, f32)] = if edge.style == BorderStyle::Double {
        &[(2.0, 1.0), (0.0, 1.0)]
    } else {
        &[(width.div_ceil(2) as f32, width as f32)]
    };
    let pattern = dash_pattern(edge.style);
    for &(back, thickness) in lines {
        let center = at - (back - thickness / 2.0) / ppp;
        let stroke = Stroke::new(thickness / ppp, color);
        let seg = |a: f32, b: f32| {
            if horizontal {
                [Pos2::new(a, center), Pos2::new(b, center)]
            } else {
                [Pos2::new(center, a), Pos2::new(center, b)]
            }
        };
        if pattern.is_empty() {
            painter.line_segment(seg(from, to), stroke);
        } else {
            let mut pos = from;
            let mut i = 0;
            let mut guard = 0;
            while pos < to && guard < 4096 {
                let len = pattern[i % pattern.len()] / ppp;
                if i % 2 == 0 {
                    painter.line_segment(seg(pos, (pos + len).min(to)), stroke);
                }
                pos += len;
                i += 1;
                guard += 1;
            }
        }
    }
}

fn paint_cell_borders(painter: &Painter, paint: &Paint<'_>, ws: &Worksheet, item: &Item<'_>) {
    let styles = paint.wb.styles();
    let col_styles = &paint.view.layout.col_styles;
    let b = if item.merged {
        let tl = ws.resolved_border(styles, item.row, item.col, col_styles);
        let tr = ws.resolved_border(styles, item.row, item.end.1, col_styles);
        let bl = ws.resolved_border(styles, item.end.0, item.col, col_styles);
        Border {
            left: tl.left,
            top: tl.top,
            right: tr.right,
            bottom: bl.bottom,
            ..tl
        }
    } else {
        ws.resolved_border(styles, item.row, item.col, col_styles)
    };
    let r = item.rect;
    let ppp = paint.ppp;
    let colour = |e: &BorderEdge| color32(styles.resolve_color(&e.color));
    if b.top.style != BorderStyle::None {
        draw_edge(
            painter,
            true,
            r.top(),
            r.left(),
            r.right(),
            &b.top,
            colour(&b.top),
            ppp,
        );
    }
    if b.bottom.style != BorderStyle::None {
        draw_edge(
            painter,
            true,
            r.bottom(),
            r.left(),
            r.right(),
            &b.bottom,
            colour(&b.bottom),
            ppp,
        );
    }
    if b.left.style != BorderStyle::None {
        draw_edge(
            painter,
            false,
            r.left(),
            r.top(),
            r.bottom(),
            &b.left,
            colour(&b.left),
            ppp,
        );
    }
    if b.right.style != BorderStyle::None {
        draw_edge(
            painter,
            false,
            r.right(),
            r.top(),
            r.bottom(),
            &b.right,
            colour(&b.right),
            ppp,
        );
    }
}

/// Sticky column and row headers; frozen ones stay put, the rest scroll under them.
fn paint_headers(
    painter: &Painter,
    view: &View<'_>,
    clip_all: Rect,
    vis: &Visible,
    pal: &Palette,
    selection: Span,
    font: &FontId,
) {
    let layout = view.layout;
    let view_min = view.view_min;
    let fz = view.frozen_size();
    let stroke = Stroke::new(1.0, pal.grid_line);

    painter.rect_filled(
        Rect::from_min_size(view_min, vec2(clip_all.width(), COL_HEADER_H)),
        0.0,
        pal.header_bg,
    );
    painter.rect_filled(
        Rect::from_min_size(view_min, vec2(ROW_HEADER_W, clip_all.height())),
        0.0,
        pal.header_bg,
    );

    let col_groups = [
        (
            (1, vis.frozen_cols),
            view.scroll.x,
            view_min.x + ROW_HEADER_W,
            view_min.x + ROW_HEADER_W + fz.x,
        ),
        (
            vis.cols,
            0.0,
            view_min.x + ROW_HEADER_W + fz.x,
            clip_all.right(),
        ),
    ];
    for (cols, shift, left, right) in col_groups {
        if cols.0 > cols.1 || right <= left {
            continue;
        }
        let p = painter.with_clip_rect(
            Rect::from_min_max(
                Pos2::new(left, view_min.y),
                Pos2::new(right, view_min.y + COL_HEADER_H),
            )
            .intersect(clip_all),
        );
        for c in cols.0..=cols.1 {
            let w = layout.w(c);
            if w <= 0.0 {
                continue;
            }
            let x = view.edge_x(c, shift);
            let r = Rect::from_min_size(Pos2::new(x, view_min.y), vec2(w, COL_HEADER_H));
            let selected = (selection.1..=selection.3).contains(&c);
            if selected {
                p.rect_filled(r, 0.0, pal.accent.gamma_multiply(0.2));
            }
            p.vline(r.left(), r.y_range(), stroke);
            p.with_clip_rect(r.intersect(p.clip_rect())).text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                col_letters(c),
                font.clone(),
                if selected { pal.accent } else { pal.text },
            );
        }
    }
    let row_groups = [
        (
            (1, vis.frozen_rows),
            view.scroll.y,
            view_min.y + COL_HEADER_H,
            view_min.y + COL_HEADER_H + fz.y,
        ),
        (
            vis.rows,
            0.0,
            view_min.y + COL_HEADER_H + fz.y,
            clip_all.bottom(),
        ),
    ];
    for (rows, shift, top, bottom) in row_groups {
        if rows.0 > rows.1 || bottom <= top {
            continue;
        }
        let p = painter.with_clip_rect(
            Rect::from_min_max(
                Pos2::new(view_min.x, top),
                Pos2::new(view_min.x + ROW_HEADER_W, bottom),
            )
            .intersect(clip_all),
        );
        for r in rows.0..=rows.1 {
            let h = layout.h(r);
            if h <= 0.0 {
                continue;
            }
            let y = view.edge_y(r, shift);
            let rr = Rect::from_min_size(Pos2::new(view_min.x, y), vec2(ROW_HEADER_W, h));
            let selected = (selection.0..=selection.2).contains(&r);
            if selected {
                p.rect_filled(rr, 0.0, pal.accent.gamma_multiply(0.2));
            }
            p.hline(rr.x_range(), rr.top(), stroke);
            p.with_clip_rect(rr.intersect(p.clip_rect())).text(
                rr.center(),
                egui::Align2::CENTER_CENTER,
                r.to_string(),
                font.clone(),
                if selected { pal.accent } else { pal.text },
            );
        }
    }

    painter.hline(
        view_min.x..=clip_all.right(),
        view_min.y + COL_HEADER_H,
        stroke,
    );
    painter.vline(
        view_min.x + ROW_HEADER_W,
        view_min.y..=clip_all.bottom(),
        stroke,
    );
    // Frozen panes get a heavier divider.
    let divider = Stroke::new(2.0, pal.grid_line);
    if vis.frozen_cols > 0 {
        painter.vline(
            view_min.x + ROW_HEADER_W + fz.x,
            view_min.y..=clip_all.bottom(),
            divider,
        );
    }
    if vis.frozen_rows > 0 {
        painter.hline(
            view_min.x..=clip_all.right(),
            view_min.y + COL_HEADER_H + fz.y,
            divider,
        );
    }
}
