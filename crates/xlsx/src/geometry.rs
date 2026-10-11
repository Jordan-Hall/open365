//! Read-only sheet geometry over the preserved worksheet XML: column widths, row heights,
//! merged cells and frozen panes, and the pixel conversion Excel applies to them.
//!
//! Nothing here touches the saved bytes; the accessors read the raw `cols`, `sheetFormatPr`,
//! `sheetViews`, `mergeCells` and `row` attributes that the model keeps verbatim.
//!
//! Pixel rules (ECMA-376 Part 1, 18.3.1.13 `col`, 18.3.1.81 `sheetFormatPr`), at 100% zoom:
//!
//! - A stored column `width` includes the cell padding. Its pixel width is
//!   `trunc(((256 * width + trunc(128 / mdw)) / 256) * mdw)`, `mdw` being the maximum digit
//!   width of the default font in pixels.
//! - Without `defaultColWidth` the default column is `baseColWidth * mdw + padding` pixels
//!   rounded up to a multiple of 8, with `padding = 2 * ceil(mdw / 4) + 1`.
//! - A row height in points is `trunc(pt * dpi / 72)` pixels. Checked against Excel for
//!   fractional heights (`tests/geometry.rs`); Excel never rounds a height up.

use crate::refs::parse_range;
use crate::sheet::{Rect, Row, Worksheet};
use crate::styles::Styles;
use crate::xml::{RawXml, XmlElement};

/// Pixels per inch of a 100% zoom, 100% scaling display.
pub const BASE_DPI: u32 = 96;

fn flag(value: Option<&str>) -> bool {
    matches!(value, Some("1" | "true"))
}

fn uint(value: Option<&str>) -> Option<u32> {
    value?.trim().parse().ok()
}

fn float(value: Option<&str>) -> Option<f64> {
    value?.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// A `<col>` entry: one width for the columns `min..=max` (1-based, inclusive).
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnSpec {
    pub min: u32,
    pub max: u32,
    /// Width in characters of the default font, padding included; `None` when absent.
    pub width: Option<f64>,
    pub custom_width: bool,
    pub hidden: bool,
    /// Default cell format (`style`) of the columns; `None` when absent.
    pub style: Option<u32>,
}

/// The `<sheetFormatPr>` defaults. `default_row_height` is in points.
#[derive(Debug, Clone, PartialEq)]
pub struct SheetFormat {
    pub base_col_width: u32,
    pub default_col_width: Option<f64>,
    pub default_row_height: f64,
    pub custom_height: bool,
    /// Rows are hidden unless a `row` element says otherwise.
    pub zero_height: bool,
}

impl Default for SheetFormat {
    fn default() -> Self {
        Self {
            base_col_width: 8,
            default_col_width: None,
            default_row_height: 15.0,
            custom_height: false,
            zero_height: false,
        }
    }
}

/// How a `<pane>` splits the sheet view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneState {
    Split,
    Frozen,
    FrozenSplit,
}

/// The first sheet view's `<pane>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Pane {
    /// Columns left of the split when frozen (a position in twips when split).
    pub x_split: f64,
    /// Rows above the split when frozen (a position in twips when split).
    pub y_split: f64,
    /// `(row, col)` of the top-left cell of the bottom-right pane.
    pub top_left: Option<(u32, u32)>,
    pub state: PaneState,
}

/// Rows and columns kept in place while the rest scrolls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrozenPanes {
    pub rows: u32,
    pub cols: u32,
}

impl Row {
    /// The `ht` attribute in points.
    pub fn height(&self) -> Option<f64> {
        float(self.attr("ht"))
    }

    /// The `customHeight` flag.
    pub fn custom_height(&self) -> bool {
        flag(self.attr("customHeight"))
    }

    /// The `hidden` flag.
    pub fn hidden(&self) -> bool {
        flag(self.attr("hidden"))
    }

    /// The row's default cell format (`s`), only when `customFormat` is set.
    pub fn default_style(&self) -> Option<u32> {
        if flag(self.attr("customFormat")) {
            uint(self.attr("s"))
        } else {
            None
        }
    }

    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs()
            .iter()
            .find(|a| a.ns.is_none() && a.name == name)
            .map(|a| a.value.as_str())
    }
}

fn root_child<'a>(nodes: &'a [RawXml], local: &str) -> Option<&'a XmlElement> {
    nodes
        .iter()
        .filter_map(RawXml::as_element)
        .find(|e| e.head.is_main(local))
}

impl Worksheet {
    fn find_root_child(&self, local: &str) -> Option<&XmlElement> {
        root_child(self.raw_before(), local).or_else(|| root_child(self.raw_after(), local))
    }

    /// The `<cols>` entries in source order. Entries without a valid `min`/`max` are skipped.
    pub fn columns(&self) -> Vec<ColumnSpec> {
        let Some(cols) = self.find_root_child("cols") else {
            return Vec::new();
        };
        cols.children
            .iter()
            .filter_map(RawXml::as_element)
            .filter(|e| e.head.is_main("col"))
            .filter_map(|e| {
                let min = uint(e.head.attr("min"))?.max(1);
                let max = uint(e.head.attr("max"))?.max(min);
                Some(ColumnSpec {
                    min,
                    max: max.min(crate::MAX_COL),
                    width: float(e.head.attr("width")),
                    custom_width: flag(e.head.attr("customWidth")),
                    hidden: flag(e.head.attr("hidden")),
                    style: uint(e.head.attr("style")),
                })
            })
            .collect()
    }

    /// `(min, max, style)` of every `<col>` with a default cell format, sorted by `min`.
    pub fn column_styles(&self) -> Vec<(u32, u32, u32)> {
        let mut out: Vec<_> = self
            .columns()
            .into_iter()
            .filter_map(|c| Some((c.min, c.max, c.style?)))
            .collect();
        out.sort_unstable();
        out
    }

    /// The cell format index that applies at `(row, col)`: the cell's own `s`, else the
    /// row's `s` when `customFormat` is set, else the column's `style` (from
    /// [`Self::column_styles`]), else 0.
    pub fn resolved_style(&self, row: u32, col: u32, column_styles: &[(u32, u32, u32)]) -> u32 {
        if let Some(s) = self.cell(row, col).and_then(|c| c.style) {
            return s;
        }
        let rows = self.rows();
        if let Ok(i) = rows.binary_search_by_key(&row, |r| r.r)
            && let Some(s) = rows[i].default_style()
        {
            return s;
        }
        let i = column_styles.partition_point(|c| c.1 < col);
        match column_styles.get(i) {
            Some(&(min, _, style)) if min <= col => style,
            _ => 0,
        }
    }

    /// The borders of the cell at `(row, col)` as Excel reports them: each edge is shared with
    /// the neighbouring cell's matching edge (see [`crate::format::shared_edge`]).
    pub fn resolved_border(
        &self,
        styles: &Styles,
        row: u32,
        col: u32,
        column_styles: &[(u32, u32, u32)],
    ) -> crate::format::Border {
        use crate::format::shared_edge;
        let own = |r: u32, c: u32| {
            styles
                .cell_format(self.resolved_style(r, c, column_styles))
                .border
        };
        let mut b = own(row, col);
        if col > 1 {
            b.left = shared_edge(own(row, col - 1).right, b.left);
        }
        if col < crate::MAX_COL {
            b.right = shared_edge(b.right, own(row, col + 1).left);
        }
        if row > 1 {
            b.top = shared_edge(own(row - 1, col).bottom, b.top);
        }
        if row < crate::MAX_ROW {
            b.bottom = shared_edge(b.bottom, own(row + 1, col).top);
        }
        b
    }

    /// The `<sheetFormatPr>` defaults, or the spec defaults when the element is absent.
    pub fn sheet_format(&self) -> SheetFormat {
        let mut out = SheetFormat::default();
        if let Some(e) = self.find_root_child("sheetFormatPr") {
            let head = &e.head;
            out.base_col_width = uint(head.attr("baseColWidth")).unwrap_or(8);
            out.default_col_width = float(head.attr("defaultColWidth"));
            if let Some(h) = float(head.attr("defaultRowHeight")) {
                out.default_row_height = h;
            }
            out.custom_height = flag(head.attr("customHeight"));
            out.zero_height = flag(head.attr("zeroHeight"));
        }
        out
    }

    /// Merged ranges `(row1, col1, row2, col2)` from `<mergeCells>`, in source order.
    /// Single-cell and unparsable entries are skipped.
    pub fn merged_cells(&self) -> Vec<Rect> {
        let Some(merges) = self.find_root_child("mergeCells") else {
            return Vec::new();
        };
        merges
            .children
            .iter()
            .filter_map(RawXml::as_element)
            .filter(|e| e.head.is_main("mergeCell"))
            .filter_map(|e| parse_range(e.head.attr("ref")?))
            .filter(|&(r1, c1, r2, c2)| (r1, c1) != (r2, c2))
            .collect()
    }

    /// The `<pane>` of the first `<sheetView>`.
    pub fn pane(&self) -> Option<Pane> {
        let view = root_child(&self.find_root_child("sheetViews")?.children, "sheetView")?;
        let pane = root_child(&view.children, "pane")?;
        let head = &pane.head;
        let state = match head.attr("state") {
            Some("frozen") => PaneState::Frozen,
            Some("frozenSplit") => PaneState::FrozenSplit,
            _ => PaneState::Split,
        };
        Some(Pane {
            x_split: float(head.attr("xSplit")).unwrap_or(0.0),
            y_split: float(head.attr("ySplit")).unwrap_or(0.0),
            top_left: head
                .attr("topLeftCell")
                .and_then(crate::refs::parse_cell_ref),
            state,
        })
    }

    /// Frozen rows and columns, when the first sheet view freezes any.
    pub fn frozen_panes(&self) -> Option<FrozenPanes> {
        let pane = self.pane()?;
        if pane.state == PaneState::Split {
            return None;
        }
        let count = |v: f64| if v >= 1.0 { v as u32 } else { 0 };
        let frozen = FrozenPanes {
            rows: count(pane.y_split).min(crate::MAX_ROW),
            cols: count(pane.x_split).min(crate::MAX_COL),
        };
        (frozen != FrozenPanes::default()).then_some(frozen)
    }
}

/// Horizontal advance of the digits 0 to 9 in em for fonts the oracle was checked against.
/// Unknown fonts use the Calibri value, which is close for most proportional UI fonts.
const DIGIT_EM: &[(&str, f64)] = &[
    ("calibri", 0.5068),
    ("aptos narrow", 0.4800),
    ("aptos", 0.5500),
    ("arial", 0.5560),
    ("segoe ui", 0.5500),
    ("times new roman", 0.5000),
    ("cambria", 0.5500),
    ("verdana", 0.6360),
    ("tahoma", 0.5460),
];

/// The default font (`fonts[0]` of the style sheet) as `(name, size in points)`.
pub fn default_font(styles: &Styles) -> (String, f64) {
    styles
        .default_font()
        .unwrap_or_else(|| ("Calibri".to_owned(), 11.0))
}

/// Pixel conversion for one display: the maximum digit width of the default font and the DPI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridMetrics {
    /// Maximum digit width of the default font in pixels (7 for Calibri 11 at 96 dpi).
    pub mdw: u32,
    /// Display pixels per inch.
    pub dpi: u32,
}

impl GridMetrics {
    pub fn new(mdw: u32, dpi: u32) -> Self {
        Self {
            mdw: mdw.max(1),
            dpi: dpi.max(1),
        }
    }

    /// Estimates the maximum digit width of `font` at `size_pt` on a `dpi` display. Verified
    /// against Excel for Aptos Narrow 11 at 120 dpi (9 px); other entries come from the font
    /// metrics and are unverified.
    pub fn for_font(font: &str, size_pt: f64, dpi: u32) -> Self {
        let name = font.to_ascii_lowercase();
        let em = DIGIT_EM
            .iter()
            .find(|(n, _)| *n == name)
            .map_or(0.5068, |(_, em)| *em);
        let size_px = size_pt * f64::from(dpi) / 72.0;
        Self::new((size_px * em).round() as u32, dpi)
    }

    /// Metrics for the workbook's default font on a `dpi` display.
    pub fn for_styles(styles: &Styles, dpi: u32) -> Self {
        let (name, size) = default_font(styles);
        Self::for_font(&name, size, dpi)
    }

    /// Cell padding in pixels: `2 * ceil(mdw / 4) + 1`.
    pub fn padding(&self) -> u32 {
        2 * self.mdw.div_ceil(4) + 1
    }

    /// Pixel width of a stored column width.
    pub fn col_px(&self, width: f64) -> u32 {
        if width <= 0.0 {
            return 0;
        }
        let mdw = f64::from(self.mdw);
        let px = ((256.0 * width + (128 / self.mdw) as f64) / 256.0 * mdw).floor();
        px.clamp(0.0, 1_000_000.0) as u32
    }

    /// Pixel width of a column when the sheet gives no width.
    pub fn default_col_px(&self, format: &SheetFormat) -> u32 {
        match format.default_col_width {
            Some(w) => self.col_px(w),
            None => {
                let raw = format.base_col_width * self.mdw + self.padding();
                raw.div_ceil(8) * 8
            }
        }
    }

    /// Pixel height of a row height in points.
    pub fn row_px(&self, points: f64) -> u32 {
        if points <= 0.0 {
            return 0;
        }
        (points * f64::from(self.dpi) / 72.0)
            .floor()
            .clamp(0.0, 1_000_000.0) as u32
    }

    /// Points for a pixel count: `px * 72 / dpi`.
    pub fn px_to_pt(&self, px: u32) -> f64 {
        f64::from(px) * 72.0 / f64::from(self.dpi)
    }
}

/// One axis of sizes: a default plus runs that differ, with O(log n) offsets.
#[derive(Debug, Clone, Default)]
pub struct Axis {
    default: u32,
    /// Sorted, non-overlapping `(first, last, px)`, 1-based inclusive.
    runs: Vec<(u32, u32, u32)>,
    /// `before[i]`: sum over runs `0..i` of `(px - default) * len`.
    before: Vec<i64>,
}

impl Axis {
    fn new(default: u32, mut runs: Vec<(u32, u32, u32)>) -> Self {
        runs.sort_by_key(|r| (r.0, r.1));
        // A later entry that starts inside an earlier one clips it, as Excel reads them.
        let mut clean: Vec<(u32, u32, u32)> = Vec::with_capacity(runs.len());
        for run in runs {
            if let Some(last) = clean.last_mut()
                && run.0 <= last.1
            {
                if run.0 <= last.0 {
                    clean.pop();
                } else {
                    last.1 = run.0 - 1;
                }
            }
            clean.push(run);
        }
        let mut before = Vec::with_capacity(clean.len() + 1);
        let mut sum = 0i64;
        for &(a, b, px) in &clean {
            before.push(sum);
            sum += (i64::from(px) - i64::from(default)) * i64::from(b - a + 1);
        }
        before.push(sum);
        Self {
            default,
            runs: clean,
            before,
        }
    }

    /// Size in pixels of the 1-based index `i`.
    pub fn size(&self, i: u32) -> u32 {
        let k = self.runs.partition_point(|r| r.1 < i);
        match self.runs.get(k) {
            Some(&(a, _, px)) if a <= i => px,
            _ => self.default,
        }
    }

    /// Pixel offset of the start of the 1-based index `i` (index 1 starts at 0).
    pub fn offset(&self, i: u32) -> u64 {
        let i = i.max(1);
        let mut total = i64::from(i - 1) * i64::from(self.default);
        let k = self.runs.partition_point(|r| r.1 < i);
        total += self.before[k];
        if let Some(&(a, _, px)) = self.runs.get(k)
            && a < i
        {
            total += (i64::from(px) - i64::from(self.default)) * i64::from(i - a);
        }
        total.max(0) as u64
    }

    /// The 1-based index whose span contains pixel `pos`; zero-size indexes are skipped.
    /// `limit` caps the result (the last index considered).
    pub fn index_at(&self, pos: u64, limit: u32) -> u32 {
        let (mut lo, mut hi) = (1u32, limit.max(1));
        while lo < hi {
            let mid = lo + (hi - lo).div_ceil(2);
            if self.offset(mid) <= pos {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        lo
    }
}

/// Resolved column widths and row heights of one worksheet in pixels.
#[derive(Debug, Clone)]
pub struct SheetGeometry {
    metrics: GridMetrics,
    cols: Axis,
    rows: Axis,
}

impl SheetGeometry {
    pub fn metrics(&self) -> GridMetrics {
        self.metrics
    }

    pub fn col_px(&self, col: u32) -> u32 {
        self.cols.size(col)
    }

    pub fn row_px(&self, row: u32) -> u32 {
        self.rows.size(row)
    }

    /// Left edge of `col` from the sheet origin, in pixels.
    pub fn col_x(&self, col: u32) -> u64 {
        self.cols.offset(col)
    }

    /// Top edge of `row` from the sheet origin, in pixels.
    pub fn row_y(&self, row: u32) -> u64 {
        self.rows.offset(row)
    }

    /// The column at pixel `x` (at most `limit`).
    pub fn col_at(&self, x: u64, limit: u32) -> u32 {
        self.cols.index_at(x, limit)
    }

    /// The row at pixel `y` (at most `limit`).
    pub fn row_at(&self, y: u64, limit: u32) -> u32 {
        self.rows.index_at(y, limit)
    }

    pub fn col_hidden(&self, col: u32) -> bool {
        self.col_px(col) == 0
    }

    pub fn row_hidden(&self, row: u32) -> bool {
        self.row_px(row) == 0
    }
}

impl Worksheet {
    /// Pixel geometry of the sheet for `metrics`.
    pub fn geometry(&self, metrics: GridMetrics) -> SheetGeometry {
        let format = self.sheet_format();
        let default_col = metrics.default_col_px(&format);
        let col_runs = self
            .columns()
            .into_iter()
            .map(|c| {
                let px = if c.hidden {
                    0
                } else {
                    c.width.map_or(default_col, |w| metrics.col_px(w))
                };
                (c.min, c.max, px)
            })
            .collect();
        let default_row = if format.zero_height {
            0
        } else {
            metrics.row_px(format.default_row_height)
        };
        let row_runs = self
            .rows()
            .iter()
            .filter_map(|row| {
                let px = if row.hidden() {
                    0
                } else if let Some(h) = row.height() {
                    metrics.row_px(h)
                } else if format.zero_height {
                    metrics.row_px(format.default_row_height)
                } else {
                    return None;
                };
                (px != default_row).then_some((row.r, row.r, px))
            })
            .collect();
        SheetGeometry {
            metrics,
            cols: Axis::new(default_col, col_runs),
            rows: Axis::new(default_row, row_runs),
        }
    }
}

impl SheetGeometry {
    /// Geometry of a sheet with no custom sizes (a chart sheet, or no sheet at all).
    pub fn uniform(metrics: GridMetrics) -> Self {
        let format = SheetFormat::default();
        Self {
            metrics,
            cols: Axis::new(metrics.default_col_px(&format), Vec::new()),
            rows: Axis::new(metrics.row_px(format.default_row_height), Vec::new()),
        }
    }
}

impl Worksheet {
    /// The stored width (characters, padding included) of a column the sheet gives no width:
    /// `defaultColWidth`, else the width `baseColWidth` characters take for `metrics`, kept to
    /// the 1/256 steps Excel stores. This is the width Excel writes when it unhides a column
    /// that was hidden without one (`fixtures/reference/xlsx/grid-hide`).
    pub fn default_column_width(&self, metrics: GridMetrics) -> f64 {
        let format = self.sheet_format();
        if let Some(w) = format.default_col_width {
            return w;
        }
        let px = metrics.default_col_px(&format);
        (f64::from(px) / f64::from(metrics.mdw) * 256.0).floor() / 256.0
    }
}
