//! Typed table properties (ECMA-376 Part 1 §17.4): `w:tblPr`, `w:tblGrid`, `w:trPr`,
//! `w:tcPr` and the table-style conditional formatting (`w:tblStylePr`).
//!
//! The model in [`crate::model`] keeps tables as raw XML in place so that an unedited
//! document round-trips; the readers here interpret that XML on demand. Lengths are in
//! points.

use crate::model::{Block, Cell, Row, RowChild, Table, TableChild};
use crate::xml::{RawXml, XmlElement};
use crate::xmlq::{child, children, int, on_off, twips_attr, val};

/// A table or cell width (`w:tblW`, `w:tcW`, ...).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TableWidth {
    /// `auto`: sized by content.
    Auto,
    /// `nil`: no width.
    Nil,
    /// Points (`dxa`).
    Points(f64),
    /// Percent of the available width (`pct`, stored in 50ths of a percent or as `NN%`).
    Percent(f64),
}

impl TableWidth {
    fn from_element(e: &XmlElement) -> Option<Self> {
        let raw = e.head.attr_w("w");
        let kind = e.head.attr_w("type").unwrap_or("dxa");
        Some(match kind {
            "auto" => Self::Auto,
            "nil" => Self::Nil,
            "pct" => {
                let raw = raw?.trim();
                match raw.strip_suffix('%') {
                    Some(p) => Self::Percent(p.parse().ok()?),
                    None => Self::Percent(raw.parse::<f64>().ok()? / 50.0),
                }
            }
            _ => Self::Points(raw?.trim().parse::<f64>().ok()? / 20.0),
        })
    }

    /// Points, resolving a percentage against `available`; `None` for `auto` and `nil`.
    pub fn resolve(self, available: f64) -> Option<f64> {
        match self {
            Self::Points(p) => Some(p),
            Self::Percent(p) => Some(available * p / 100.0),
            Self::Auto | Self::Nil => None,
        }
    }
}

/// `w:tblLayout`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableLayoutType {
    Fixed,
    AutoFit,
}

/// Table alignment between the margins (`w:jc` of `w:tblPr`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableJustification {
    Left,
    Center,
    Right,
}

/// One border (`w:top`, `w:insideH`, ...).
#[derive(Debug, Clone, PartialEq)]
pub struct Border {
    /// `w:val`: `single`, `double`, `nil`, `none`...
    pub style: String,
    /// Line width in points (`w:sz` is in eighths of a point).
    pub width: f64,
    /// `w:space` in points.
    pub space: f64,
    pub color: Option<[u8; 3]>,
}

impl Border {
    pub(crate) fn from_element(e: &XmlElement) -> Self {
        Self {
            style: e.head.attr_w("val").unwrap_or("nil").to_owned(),
            width: int(e.head.attr_w("sz")).map_or(0.0, |s| s as f64 / 8.0),
            space: int(e.head.attr_w("space")).map_or(0.0, |s| s as f64),
            color: e.head.attr_w("color").and_then(parse_color),
        }
    }

    /// False for `nil`/`none`.
    pub fn is_visible(&self) -> bool {
        !matches!(self.style.as_str(), "nil" | "none")
    }
}

fn parse_color(value: &str) -> Option<[u8; 3]> {
    if value.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(value, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// The borders of a table or cell. `None` means "not stated here".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BorderSet {
    pub top: Option<Border>,
    pub left: Option<Border>,
    pub bottom: Option<Border>,
    pub right: Option<Border>,
    pub inside_h: Option<Border>,
    pub inside_v: Option<Border>,
}

impl BorderSet {
    fn from_element(e: &XmlElement) -> Self {
        let get = |names: &[&str]| {
            names
                .iter()
                .find_map(|n| child(e, n))
                .map(Border::from_element)
        };
        Self {
            top: get(&["top"]),
            left: get(&["left", "start"]),
            bottom: get(&["bottom"]),
            right: get(&["right", "end"]),
            inside_h: get(&["insideH"]),
            inside_v: get(&["insideV"]),
        }
    }

    /// `self` wins where it states a border.
    pub fn over(self, lower: Self) -> Self {
        Self {
            top: self.top.or(lower.top),
            left: self.left.or(lower.left),
            bottom: self.bottom.or(lower.bottom),
            right: self.right.or(lower.right),
            inside_h: self.inside_h.or(lower.inside_h),
            inside_v: self.inside_v.or(lower.inside_v),
        }
    }
}

/// Cell margins (`w:tblCellMar`, `w:tcMar`). `None` means "not stated here".
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CellMargins {
    pub top: Option<f64>,
    pub left: Option<f64>,
    pub bottom: Option<f64>,
    pub right: Option<f64>,
}

impl CellMargins {
    fn from_element(e: &XmlElement) -> Self {
        let get = |names: &[&str]| {
            names
                .iter()
                .find_map(|n| child(e, n))
                .and_then(|m| match TableWidth::from_element(m) {
                    Some(TableWidth::Points(p)) => Some(p),
                    _ => None,
                })
        };
        Self {
            top: get(&["top"]),
            left: get(&["left", "start"]),
            bottom: get(&["bottom"]),
            right: get(&["right", "end"]),
        }
    }

    /// `self` wins where it states a margin.
    pub fn over(self, lower: Self) -> Self {
        Self {
            top: self.top.or(lower.top),
            left: self.left.or(lower.left),
            bottom: self.bottom.or(lower.bottom),
            right: self.right.or(lower.right),
        }
    }
}

/// Cell shading (`w:shd`).
#[derive(Debug, Clone, PartialEq)]
pub struct Shading {
    pub pattern: String,
    pub fill: Option<[u8; 3]>,
    pub color: Option<[u8; 3]>,
}

impl Shading {
    fn from_element(e: &XmlElement) -> Self {
        Self {
            pattern: e.head.attr_w("val").unwrap_or("clear").to_owned(),
            fill: e.head.attr_w("fill").and_then(parse_color),
            color: e.head.attr_w("color").and_then(parse_color),
        }
    }
}

/// `w:tblLook`: which conditional formats of the table style apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableLook {
    pub first_row: bool,
    pub last_row: bool,
    pub first_column: bool,
    pub last_column: bool,
    pub no_h_band: bool,
    pub no_v_band: bool,
}

impl Default for TableLook {
    /// The look Word writes for a new table: header row, first column, banded rows.
    fn default() -> Self {
        Self {
            first_row: true,
            last_row: false,
            first_column: true,
            last_column: false,
            no_h_band: false,
            no_v_band: true,
        }
    }
}

impl TableLook {
    fn from_element(e: &XmlElement) -> Self {
        let flag = |name: &str, bit: u32, mask: Option<u32>| match e.head.attr_w(name) {
            Some(v) => matches!(v, "1" | "true" | "on"),
            None => mask.is_some_and(|m| m & bit != 0),
        };
        let mask = e
            .head
            .attr_w("val")
            .and_then(|v| u32::from_str_radix(v.trim(), 16).ok());
        Self {
            first_row: flag("firstRow", 0x0020, mask),
            last_row: flag("lastRow", 0x0040, mask),
            first_column: flag("firstColumn", 0x0080, mask),
            last_column: flag("lastColumn", 0x0100, mask),
            no_h_band: flag("noHBand", 0x0200, mask),
            no_v_band: flag("noVBand", 0x0400, mask),
        }
    }
}

/// Table properties as written in one place (`w:tblPr`); `None` means "not stated".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TableProps {
    /// `w:tblStyle`.
    pub style_id: Option<String>,
    pub width: Option<TableWidth>,
    /// `w:tblInd` in points.
    pub indent: Option<f64>,
    pub justification: Option<TableJustification>,
    pub layout: Option<TableLayoutType>,
    pub cell_margins: CellMargins,
    pub borders: BorderSet,
    pub look: Option<TableLook>,
    /// `w:tblStyleRowBandSize`, `w:tblStyleColBandSize`.
    pub row_band_size: Option<u32>,
    pub col_band_size: Option<u32>,
    /// `w:tblCellSpacing` in points.
    pub cell_spacing: Option<f64>,
}

impl TableProps {
    pub(crate) fn from_element(e: &XmlElement) -> Self {
        Self {
            style_id: val(e, "tblStyle").map(str::to_owned),
            width: child(e, "tblW").and_then(TableWidth::from_element),
            indent: child(e, "tblInd")
                .or_else(|| child(e, "tblIndent"))
                .and_then(TableWidth::from_element)
                .and_then(|w| match w {
                    TableWidth::Points(p) => Some(p),
                    _ => None,
                }),
            justification: val(e, "jc").and_then(|v| match v {
                "center" => Some(TableJustification::Center),
                "right" | "end" => Some(TableJustification::Right),
                "left" | "start" => Some(TableJustification::Left),
                _ => None,
            }),
            layout: child(e, "tblLayout").map(|l| match l.head.attr_w("type") {
                Some("fixed") => TableLayoutType::Fixed,
                _ => TableLayoutType::AutoFit,
            }),
            cell_margins: child(e, "tblCellMar")
                .map(CellMargins::from_element)
                .unwrap_or_default(),
            borders: child(e, "tblBorders")
                .map(BorderSet::from_element)
                .unwrap_or_default(),
            look: child(e, "tblLook").map(TableLook::from_element),
            row_band_size: int(val(e, "tblStyleRowBandSize")).and_then(|v| u32::try_from(v).ok()),
            col_band_size: int(val(e, "tblStyleColBandSize")).and_then(|v| u32::try_from(v).ok()),
            cell_spacing: child(e, "tblCellSpacing")
                .and_then(TableWidth::from_element)
                .and_then(|w| match w {
                    TableWidth::Points(p) => Some(p),
                    _ => None,
                }),
        }
    }

    /// `self` wins where it states a value, `lower` fills the rest. The style id is
    /// always `self`'s.
    pub fn over(self, lower: Self) -> Self {
        Self {
            style_id: self.style_id.or(lower.style_id),
            width: self.width.or(lower.width),
            indent: self.indent.or(lower.indent),
            justification: self.justification.or(lower.justification),
            layout: self.layout.or(lower.layout),
            cell_margins: self.cell_margins.over(lower.cell_margins),
            borders: self.borders.over(lower.borders),
            look: self.look.or(lower.look),
            row_band_size: self.row_band_size.or(lower.row_band_size),
            col_band_size: self.col_band_size.or(lower.col_band_size),
            cell_spacing: self.cell_spacing.or(lower.cell_spacing),
        }
    }
}

/// `w:trHeight` rule (`w:hRule`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeightRule {
    /// The height value is ignored (`auto`).
    Auto,
    AtLeast,
    Exact,
}

/// Row properties (`w:trPr`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RowProps {
    /// `w:trHeight` value in points, with its rule. A height without `w:hRule` is
    /// "at least".
    pub height: Option<(f64, HeightRule)>,
    /// `w:cantSplit`.
    pub cant_split: bool,
    /// `w:tblHeader`: repeated at the top of every page the table continues on.
    pub is_header: bool,
    /// `w:gridBefore` / `w:gridAfter`: grid columns left empty at the row's start or end.
    pub grid_before: usize,
    pub grid_after: usize,
    pub width_before: Option<TableWidth>,
    pub width_after: Option<TableWidth>,
    pub justification: Option<TableJustification>,
    pub cell_spacing: Option<f64>,
}

impl RowProps {
    fn from_element(e: &XmlElement) -> Self {
        let flag = |l: &str| child(e, l).is_some_and(on_off);
        let count = |l: &str| {
            int(val(e, l))
                .and_then(|v| usize::try_from(v).ok())
                .unwrap_or(0)
        };
        Self {
            height: child(e, "trHeight").and_then(|h| {
                let v = twips_attr(h, "val")?;
                let rule = match h.head.attr_w("hRule") {
                    Some("auto") => HeightRule::Auto,
                    Some("exact") => HeightRule::Exact,
                    _ => HeightRule::AtLeast,
                };
                Some((v, rule))
            }),
            cant_split: flag("cantSplit"),
            is_header: flag("tblHeader"),
            grid_before: count("gridBefore"),
            grid_after: count("gridAfter"),
            width_before: child(e, "wBefore").and_then(TableWidth::from_element),
            width_after: child(e, "wAfter").and_then(TableWidth::from_element),
            justification: val(e, "jc").and_then(|v| match v {
                "center" => Some(TableJustification::Center),
                "right" | "end" => Some(TableJustification::Right),
                "left" | "start" => Some(TableJustification::Left),
                _ => None,
            }),
            cell_spacing: child(e, "tblCellSpacing")
                .and_then(TableWidth::from_element)
                .and_then(|w| match w {
                    TableWidth::Points(p) => Some(p),
                    _ => None,
                }),
        }
    }
}

/// `w:vMerge`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerticalMerge {
    #[default]
    None,
    /// `restart`: the first cell of a vertical merge.
    Restart,
    /// `continue` (or a bare `w:vMerge`): covered by the cell above.
    Continue,
}

/// `w:vAlign`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerticalAlign {
    #[default]
    Top,
    Center,
    Bottom,
}

/// Cell properties (`w:tcPr`).
#[derive(Debug, Clone, PartialEq)]
pub struct CellProps {
    pub width: Option<TableWidth>,
    /// `w:gridSpan`, at least 1.
    pub grid_span: usize,
    pub v_merge: VerticalMerge,
    pub margins: CellMargins,
    pub v_align: Option<VerticalAlign>,
    pub borders: BorderSet,
    pub shading: Option<Shading>,
    /// `w:noWrap`.
    pub no_wrap: bool,
}

impl Default for CellProps {
    fn default() -> Self {
        Self {
            width: None,
            grid_span: 1,
            v_merge: VerticalMerge::None,
            margins: CellMargins::default(),
            v_align: None,
            borders: BorderSet::default(),
            shading: None,
            no_wrap: false,
        }
    }
}

impl CellProps {
    pub(crate) fn from_element(e: &XmlElement) -> Self {
        Self {
            width: child(e, "tcW").and_then(TableWidth::from_element),
            grid_span: int(val(e, "gridSpan"))
                .and_then(|v| usize::try_from(v).ok())
                .filter(|v| *v >= 1)
                .unwrap_or(1),
            v_merge: match child(e, "vMerge") {
                None => VerticalMerge::None,
                Some(m) => match m.head.attr_w("val") {
                    Some("restart") => VerticalMerge::Restart,
                    _ => VerticalMerge::Continue,
                },
            },
            margins: child(e, "tcMar")
                .map(CellMargins::from_element)
                .unwrap_or_default(),
            v_align: val(e, "vAlign").and_then(|v| match v {
                "top" => Some(VerticalAlign::Top),
                "center" => Some(VerticalAlign::Center),
                "bottom" => Some(VerticalAlign::Bottom),
                _ => None,
            }),
            borders: child(e, "tcBorders")
                .map(BorderSet::from_element)
                .unwrap_or_default(),
            shading: child(e, "shd").map(Shading::from_element),
            no_wrap: child(e, "noWrap").is_some_and(on_off),
        }
    }
}

impl Table {
    /// The `w:tblPr` element.
    fn tbl_pr(&self) -> Option<&XmlElement> {
        self.children.iter().find_map(|c| match c {
            TableChild::Raw(RawXml::Element(e)) if e.head.is_w("tblPr") => Some(e),
            _ => None,
        })
    }

    /// Table properties written directly on the table.
    pub fn properties(&self) -> TableProps {
        self.tbl_pr()
            .map(TableProps::from_element)
            .unwrap_or_default()
    }

    /// Grid column widths in points (`w:tblGrid`).
    pub fn grid(&self) -> Vec<f64> {
        self.children
            .iter()
            .find_map(|c| match c {
                TableChild::Raw(RawXml::Element(e)) if e.head.is_w("tblGrid") => Some(e),
                _ => None,
            })
            .map(|g| {
                children(g, "gridCol")
                    .map(|c| twips_attr(c, "w").unwrap_or(0.0))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Rows with their properties and cells placed on the grid.
    pub fn view(&self) -> TableView<'_> {
        let mut rows: Vec<RowView<'_>> = self
            .rows()
            .map(|row| {
                let props = row.properties();
                let mut col = props.grid_before;
                let cells = row
                    .cells()
                    .map(|cell| {
                        let p = cell.properties();
                        let view = CellView {
                            cell,
                            col,
                            span: p.grid_span,
                            row_span: 1,
                            props: p,
                        };
                        col += view.span;
                        view
                    })
                    .collect();
                RowView {
                    row,
                    props,
                    exceptions: row.property_exceptions(),
                    cells,
                }
            })
            .collect();
        // Vertical merges: a restart cell spans the following rows' continue cells that sit
        // at the same grid column; those cells are covered (`row_span` 0).
        for r in 0..rows.len() {
            for c in 0..rows[r].cells.len() {
                if rows[r].cells[c].props.v_merge == VerticalMerge::Continue
                    && r > 0
                    && rows[r - 1]
                        .cells
                        .iter()
                        .any(|up| up.col == rows[r].cells[c].col)
                {
                    rows[r].cells[c].row_span = 0;
                }
            }
        }
        for r in 0..rows.len() {
            for c in 0..rows[r].cells.len() {
                if rows[r].cells[c].row_span == 0 {
                    continue;
                }
                let col = rows[r].cells[c].col;
                let mut span = 1;
                while r + span < rows.len()
                    && rows[r + span]
                        .cells
                        .iter()
                        .any(|x| x.col == col && x.row_span == 0)
                {
                    span += 1;
                }
                rows[r].cells[c].row_span = span;
            }
        }
        TableView {
            table: self,
            props: self.properties(),
            grid: self.grid(),
            rows,
        }
    }
}

impl Row {
    /// Row properties (`w:trPr`).
    pub fn properties(&self) -> RowProps {
        self.children
            .iter()
            .find_map(|c| match c {
                RowChild::Raw(RawXml::Element(e)) if e.head.is_w("trPr") => Some(e),
                _ => None,
            })
            .map(RowProps::from_element)
            .unwrap_or_default()
    }

    /// Table property exceptions of this row (`w:tblPrEx`).
    pub fn property_exceptions(&self) -> Option<TableProps> {
        self.children.iter().find_map(|c| match c {
            RowChild::Raw(RawXml::Element(e)) if e.head.is_w("tblPrEx") => {
                Some(TableProps::from_element(e))
            }
            _ => None,
        })
    }
}

impl Cell {
    /// Cell properties (`w:tcPr`).
    pub fn properties(&self) -> CellProps {
        self.blocks
            .iter()
            .find_map(|b| match b {
                Block::Raw(RawXml::Element(e)) if e.head.is_w("tcPr") => Some(e),
                _ => None,
            })
            .map(CellProps::from_element)
            .unwrap_or_default()
    }
}

/// A table with properties resolved from its own XML (style conditions are applied by
/// [`crate::Styles`]).
#[derive(Debug, Clone)]
pub struct TableView<'a> {
    pub table: &'a Table,
    pub props: TableProps,
    /// Grid column widths in points.
    pub grid: Vec<f64>,
    pub rows: Vec<RowView<'a>>,
}

/// One row of a [`TableView`].
#[derive(Debug, Clone)]
pub struct RowView<'a> {
    pub row: &'a Row,
    pub props: RowProps,
    /// `w:tblPrEx`.
    pub exceptions: Option<TableProps>,
    pub cells: Vec<CellView<'a>>,
}

/// One cell of a [`RowView`], placed on the grid.
#[derive(Debug, Clone)]
pub struct CellView<'a> {
    pub cell: &'a Cell,
    pub props: CellProps,
    /// First grid column.
    pub col: usize,
    /// Grid columns covered.
    pub span: usize,
    /// Rows covered when this cell starts a vertical merge (or stands alone: 1); 0 when the
    /// cell is covered by the one above.
    pub row_span: usize,
}

/// Position of a cell in its table, for conditional formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellPosition {
    pub row: usize,
    pub column: usize,
    pub rows: usize,
    pub columns: usize,
}

/// Conditions a table style can format (`w:tblStylePr` `w:type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableStyleCondition {
    WholeTable,
    FirstRow,
    LastRow,
    FirstColumn,
    LastColumn,
    Band1Vert,
    Band2Vert,
    Band1Horz,
    Band2Horz,
    NeCell,
    NwCell,
    SeCell,
    SwCell,
}

impl TableStyleCondition {
    pub(crate) fn parse(v: &str) -> Option<Self> {
        Some(match v {
            "wholeTable" => Self::WholeTable,
            "firstRow" => Self::FirstRow,
            "lastRow" => Self::LastRow,
            "firstCol" => Self::FirstColumn,
            "lastCol" => Self::LastColumn,
            "band1Vert" => Self::Band1Vert,
            "band2Vert" => Self::Band2Vert,
            "band1Horz" => Self::Band1Horz,
            "band2Horz" => Self::Band2Horz,
            "neCell" => Self::NeCell,
            "nwCell" => Self::NwCell,
            "seCell" => Self::SeCell,
            "swCell" => Self::SwCell,
            _ => return None,
        })
    }

    /// The conditions that format a cell, in application order (later ones win), for a
    /// table with `look` (ECMA-376 Part 1 §17.7.6).
    pub fn applying(look: TableLook, band: (u32, u32), pos: CellPosition) -> Vec<Self> {
        use TableStyleCondition::*;
        let mut out = vec![WholeTable];
        let first_row = look.first_row && pos.row == 0;
        let last_row = look.last_row && pos.rows > 0 && pos.row + 1 == pos.rows;
        let first_col = look.first_column && pos.column == 0;
        let last_col = look.last_column && pos.columns > 0 && pos.column + 1 == pos.columns;
        // Banding counts rows and columns after the header row / first column.
        if !look.no_h_band && !first_row && !last_row {
            let skip = usize::from(look.first_row);
            let size = band.0.max(1) as usize;
            out.push(if ((pos.row - skip) / size).is_multiple_of(2) {
                Band1Horz
            } else {
                Band2Horz
            });
        }
        if !look.no_v_band && !first_col && !last_col {
            let skip = usize::from(look.first_column);
            let size = band.1.max(1) as usize;
            out.push(if ((pos.column - skip) / size).is_multiple_of(2) {
                Band1Vert
            } else {
                Band2Vert
            });
        }
        if first_col {
            out.push(FirstColumn);
        }
        if last_col {
            out.push(LastColumn);
        }
        if first_row {
            out.push(FirstRow);
        }
        if last_row {
            out.push(LastRow);
        }
        if first_row && first_col {
            out.push(NwCell);
        }
        if first_row && last_col {
            out.push(NeCell);
        }
        if last_row && first_col {
            out.push(SwCell);
        }
        if last_row && last_col {
            out.push(SeCell);
        }
        out
    }
}
