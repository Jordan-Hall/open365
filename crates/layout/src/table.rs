//! Table layout: column widths, cell content, row heights. Page placement (splitting rows,
//! repeating header rows) is in `flow.rs`.
//!
//! The rules were measured against Word with the `layout-table-*` fixtures; see
//! `docs/architecture/word-layout.md`.

use open365_docx::{
    Block, Border, CellMargins, CellPosition, HeightRule, Shading, Table, TableJustification,
    TableProps, VerticalAlign,
};

use std::sync::Arc;

use open365_docx::Drawing;

use crate::build::Builder;
use crate::float::{Float, Frame, dynamic_lines};
use crate::flow::Base;
use crate::para::AtomClass;
use crate::textbox::BoxRef;
use crate::{BorderLine, BorderStyle, CellBorders, ImagePlacement};

/// The identity and look of a cell, as `CellBox` reports it. `table` counts the tables of the
/// unit; the paginator adds the unit's first table id.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CellMeta {
    pub table: usize,
    pub row: usize,
    pub col: usize,
    pub col_span: usize,
    pub row_span: usize,
    pub header_row: bool,
    pub fill: Option<[u8; 3]>,
    pub borders: CellBorders,
    /// Top and bottom cell margins (`w:tblCellMar`, `w:tcMar`).
    pub pad_top: f64,
    pub pad_bottom: f64,
}

/// The border drawn at a cell edge: the first level that states one decides.
fn resolve_border(
    a: &Option<Border>,
    b: &Option<Border>,
    t: &Option<Border>,
) -> Option<BorderLine> {
    let found = a.as_ref().or(b.as_ref()).or(t.as_ref())?;
    if !found.is_visible() || found.width <= 0.0 {
        return None;
    }
    let style = match found.style.as_str() {
        "double" | "triple" | "thinThickSmallGap" | "thickThinSmallGap" => BorderStyle::Double,
        "dotted" => BorderStyle::Dotted,
        "dashed" | "dotDash" | "dotDotDash" | "dashSmallGap" => BorderStyle::Dashed,
        _ => BorderStyle::Solid,
    };
    Some(BorderLine {
        width: found.width,
        color: found.color.unwrap_or([0, 0, 0]),
        style,
    })
}

/// A shading as a solid colour: `pctNN` patterns mix the foreground into the fill.
fn shading_fill(s: &Shading) -> Option<[u8; 3]> {
    let pattern = s.pattern.as_str();
    match pattern {
        "nil" | "none" => None,
        "solid" => s.color.or(s.fill),
        p => {
            if let Some(pct) = p.strip_prefix("pct").and_then(|n| n.parse::<f64>().ok()) {
                let t = (pct / 100.0).clamp(0.0, 1.0);
                let fg = s.color.unwrap_or([0, 0, 0]);
                let bg = s.fill.unwrap_or([255, 255, 255]);
                let mix =
                    |i: usize| (f64::from(fg[i]) * t + f64::from(bg[i]) * (1.0 - t)).round() as u8;
                Some([mix(0), mix(1), mix(2)])
            } else {
                s.fill
            }
        }
    }
}

/// Word's default cell margins when neither the table, its style nor the cell state one.
const DEFAULT_MARGIN_X: f64 = 5.4;
const DEFAULT_MARGIN_Y: f64 = 0.0;

/// One line of cell content, positioned inside the cell's content box.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FlowLine {
    /// Index into the paragraph arena.
    pub para: usize,
    pub line: usize,
    /// Left of the line's paragraph text area relative to the content box.
    pub dx: f64,
    /// Top of the line relative to the content box.
    pub dy: f64,
    pub height: f64,
    /// The paragraph continues the border box of the one before it, or the one after it does.
    pub join_prev: bool,
    pub join_next: bool,
}

/// A floating drawing anchored in a line of a cell.
#[derive(Debug, Clone)]
pub(crate) struct FlowImage {
    /// Index of the anchor's line in the cell's lines.
    pub line: usize,
    /// Top left corner relative to the content box (a drawing placed against the cell), or the
    /// anchor paragraph's top in `dy` (a drawing placed against the page).
    pub dx: f64,
    pub dy: f64,
    pub kind: ImageKind,
}

#[derive(Debug, Clone)]
pub(crate) enum ImageKind {
    /// Placed: `x` and `y` are set when the cell is drawn.
    Cell(ImagePlacement),
    /// `layoutInCell="0"` in a document of compatibility mode 14 or lower: positioned against the
    /// page when the cell is drawn.
    Page(PageDrawing),
}

#[derive(Debug, Clone)]
pub(crate) struct PageDrawing {
    pub drawing: Drawing,
    pub atom: usize,
    pub polygon: Option<Arc<[(f64, f64)]>>,
    pub text_box: Option<BoxRef>,
    /// Arena index of the anchor paragraph.
    pub paragraph: usize,
}

/// A nested cell's rectangle inside the content box.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FlowBox {
    pub dx: f64,
    pub dy: f64,
    pub w: f64,
    pub h: f64,
    pub meta: CellMeta,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Pad {
    pub top: f64,
    pub left: f64,
    pub bottom: f64,
    pub right: f64,
}

#[derive(Debug)]
pub(crate) struct CellLayout {
    /// Left edge relative to the table's left edge, and width.
    pub x: f64,
    pub w: f64,
    /// Rows covered; 0 for a cell covered by the one above.
    pub row_span: usize,
    pub pad: Pad,
    pub lines: Vec<FlowLine>,
    pub boxes: Vec<FlowBox>,
    pub images: Vec<FlowImage>,
    /// Height of the content including the space before the first and after the last paragraph.
    pub content_h: f64,
    /// Top of the content box relative to the table's top, vertical alignment applied.
    pub content_top: f64,
    pub valign: VerticalAlign,
    /// Half the border on the cell's left and right grid lines: the drawn rectangle is the
    /// cell between the borders, so it starts and ends this far inside `x` and `x + w`.
    pub inset_l: f64,
    pub inset_r: f64,
    pub meta: CellMeta,
    /// Widths of the borders on the cell's four sides, as resolved for the table.
    pub border_top: f64,
    border_bottom: f64,
    border_left: f64,
    border_right: f64,
}

#[derive(Debug)]
pub(crate) struct RowLayout {
    pub top: f64,
    /// Includes the border below the row and, for the first row, the one above the table.
    pub height: f64,
    /// Border above the row (the table's top border, on the first row only) and below it (the
    /// border between this row and the next, or the table's bottom border).
    pub above: f64,
    pub below: f64,
    pub cant_split: bool,
    pub is_header: bool,
    /// `w:hRule="exact"`: the content is clipped, never split.
    pub exact: bool,
    pub cells: Vec<CellLayout>,
}

/// What a floating table's position is measured from (`w:horzAnchor`, `w:vertAnchor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PosAnchor {
    /// The column (horizontal) or the anchor paragraph's top (vertical).
    Text,
    Margin,
    Page,
}

/// `w:tblpXSpec` and `w:tblpYSpec`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PosSpec {
    Start,
    Center,
    End,
    /// Vertical only: `inline` keeps the table in the flow.
    Inline,
}

/// A table out of the flow (`w:tblpPr`), positions in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TablePos {
    pub horz: PosAnchor,
    pub vert: PosAnchor,
    pub x: f64,
    pub y: f64,
    pub x_spec: Option<PosSpec>,
    pub y_spec: Option<PosSpec>,
    /// Text distances: left, top, right, bottom.
    pub dist: [f64; 4],
    /// `w:tblOverlap` is not `never`.
    pub overlap: bool,
}

impl TablePos {
    /// Reads `w:tblPr/w:tblpPr` of `table`.
    pub(crate) fn read(table: &Table) -> Option<Self> {
        use open365_docx::{RawXml, TableChild};
        let pr = table.children.iter().find_map(|c| match c {
            TableChild::Raw(RawXml::Element(e)) if e.head.is_w("tblPr") => Some(e),
            _ => None,
        })?;
        let el = |name: &str| {
            pr.children
                .iter()
                .filter_map(RawXml::as_element)
                .find(|e| e.head.is_w(name))
        };
        let p = el("tblpPr")?;
        let twips = |name: &str| {
            p.head
                .attr_w(name)
                .and_then(|v| v.trim().parse::<i64>().ok())
        };
        let pt = |name: &str| twips(name).map_or(0.0, |t| t as f64 / 20.0);
        // Word writes a position one twip too large (0 as 1, 60 pt as 1201, ...) and reads it
        // back without that twip: odd values lose it (layout-tblp-basic, layout-tblp-margin).
        let pos = |name: &str| twips(name).map_or(0.0, |t| (t - (t & 1)) as f64 / 20.0);
        // Word omits the default: `horzAnchor` text, `vertAnchor` margin (layout-tblp-margin).
        let anchor = |name: &str, default: PosAnchor| match p.head.attr_w(name) {
            Some("margin") => PosAnchor::Margin,
            Some("page") => PosAnchor::Page,
            Some("text") => PosAnchor::Text,
            _ => default,
        };
        let spec = |name: &str| {
            p.head.attr_w(name).and_then(|v| match v {
                "left" | "inside" | "top" => Some(PosSpec::Start),
                "center" => Some(PosSpec::Center),
                "right" | "outside" | "bottom" => Some(PosSpec::End),
                "inline" => Some(PosSpec::Inline),
                _ => None,
            })
        };
        Some(Self {
            horz: anchor("horzAnchor", PosAnchor::Text),
            vert: anchor("vertAnchor", PosAnchor::Margin),
            x: pos("tblpX"),
            y: pos("tblpY"),
            x_spec: spec("tblpXSpec"),
            y_spec: spec("tblpYSpec"),
            dist: [
                pt("leftFromText"),
                pt("topFromText"),
                pt("rightFromText"),
                pt("bottomFromText"),
            ],
            overlap: el("tblOverlap").and_then(|e| e.head.attr_w("val")) != Some("never"),
        })
    }
}

#[derive(Debug)]
pub(crate) struct TableLayout {
    /// Left edge relative to the text area.
    pub x: f64,
    /// Sum of the grid columns, and the widths of the borders on the left and right grid lines.
    pub width: f64,
    pub edge_l: f64,
    pub edge_r: f64,
    /// `w:tblpPr`: the table floats.
    pub pos: Option<TablePos>,
    pub rows: Vec<RowLayout>,
    /// Leading rows marked `w:tblHeader`.
    pub header_rows: usize,
}

impl TableLayout {
    pub(crate) fn height(&self) -> f64 {
        self.rows.last().map_or(0.0, |r| r.top + r.height)
    }

    /// Width of the table including its outer borders.
    pub(crate) fn outer_width(&self) -> f64 {
        self.width + (self.edge_l + self.edge_r) / 2.0
    }
}

/// Width of a border that is set: its line width when drawn, 0 for `nil`/`none`. `None` when
/// the level states no border at all.
fn border_width(b: &Option<Border>) -> Option<f64> {
    b.as_ref().map(crate::para::border_extent)
}

struct Prev {
    after: f64,
    contextual: bool,
    style: Option<String>,
}

/// What a cell holds, relative to its content box.
#[derive(Default)]
pub(crate) struct CellContent {
    pub lines: Vec<FlowLine>,
    pub boxes: Vec<FlowBox>,
    pub images: Vec<FlowImage>,
    pub height: f64,
}

/// Content of a cell laid out in a box `content_w` wide, with `pad` around it.
pub(crate) fn layout_cell(
    b: &mut Builder<'_>,
    blocks: &[Block],
    content_w: f64,
    pad: Pad,
    fmt: &open365_docx::TableCellFormat,
) -> CellContent {
    let mut lines: Vec<FlowLine> = Vec::new();
    let mut boxes = Vec::new();
    let mut images: Vec<FlowImage> = Vec::new();
    // Floats of the cell's drawings in cell coordinates (origin: the cell's outer top left).
    let mut floats: Vec<Float> = Vec::new();
    let frame = Frame::cell(pad.left, pad.right, content_w);
    let mut y = 0.0;
    let mut prev: Option<Prev> = None;
    // The previous paragraph of the run being stacked (arena index) and the bottom border it
    // has not added yet: a following paragraph with the same borders takes it into its box.
    let mut run_prev: Option<usize> = None;
    let mut pending_bottom = 0.0;
    let count = blocks.len();
    for (bi, block) in blocks.iter().enumerate() {
        match block {
            Block::Paragraph(p) => {
                let idx = b.paragraph(p, content_w, Some(fmt));
                let pl = &b.arena[idx];
                // Measured (layout-table-flow): the empty paragraph that must follow a nested
                // table at the end of a cell takes no height.
                let after_table = bi > 0 && matches!(blocks[bi - 1], Block::Table(_));
                if after_table
                    && bi + 1 == count
                    && pl.atoms.iter().all(|a| a.class == AtomClass::Mark)
                {
                    continue;
                }
                let joined = run_prev.is_some_and(|q| b.arena[q].joins(pl));
                if !joined {
                    y += std::mem::take(&mut pending_bottom);
                }
                let mut before = f64::from(pl.props.space_before_pt);
                let mut after_prev = prev.as_ref().map_or(0.0, |p| p.after);
                if let Some(pv) = &prev
                    && pv.style == pl.style_id
                {
                    if pv.contextual {
                        after_prev = 0.0;
                    }
                    if pl.flow.contextual_spacing {
                        before = 0.0;
                    }
                }
                y += after_prev.max(before);
                y += if joined {
                    pl.between_extent()
                } else {
                    pl.border_top()
                };
                // Floating drawings anchored here, or still reaching down from an earlier
                // paragraph of the cell, break the lines again (layout-float-incell).
                let anchored = pl.objects.iter().any(|o| !o.inline() && !o.page_anchored);
                let reached = floats.iter().any(|f| f.y1 > y + pad.top + 1e-6);
                let mut dynamic = None;
                if anchored || reached {
                    let d = dynamic_lines(
                        pl,
                        0,
                        y + pad.top,
                        None,
                        &floats,
                        &frame,
                        Base {
                            para: 0,
                            story: 0,
                            table: 0,
                        },
                    );
                    dynamic = Some(d);
                }
                let para_top = y;
                let first_line = lines.len();
                if let Some(d) = dynamic {
                    b.arena[idx].lines = d.lines;
                    let pl = &b.arena[idx];
                    for (i, lb) in pl.lines.iter().enumerate() {
                        y += lb.lead;
                        lines.push(FlowLine {
                            para: idx,
                            line: i,
                            dx: 0.0,
                            dy: y,
                            height: lb.height,
                            join_prev: joined,
                            join_next: false,
                        });
                        y += lb.height;
                    }
                    for a in d.anchors {
                        let line = pl
                            .lines
                            .iter()
                            .position(|l| l.atoms.contains(&a.atom))
                            .map_or(first_line, |k| first_line + k);
                        floats.extend(a.float);
                        let mut image = a.image;
                        image.paragraph = idx;
                        images.push(FlowImage {
                            line,
                            dx: image.x - pad.left,
                            dy: image.y - pad.top,
                            kind: ImageKind::Cell(image),
                        });
                    }
                } else {
                    for (i, lb) in pl.lines.iter().enumerate() {
                        lines.push(FlowLine {
                            para: idx,
                            line: i,
                            dx: 0.0,
                            dy: y,
                            height: lb.height,
                            join_prev: joined,
                            join_next: false,
                        });
                        y += lb.height;
                    }
                }
                let pl = &b.arena[idx];
                for o in pl.objects.iter().filter(|o| o.page_anchored) {
                    let line = pl
                        .lines
                        .iter()
                        .position(|l| l.atoms.contains(&o.atom))
                        .map_or(first_line, |k| first_line + k);
                    images.push(FlowImage {
                        line,
                        dx: 0.0,
                        dy: para_top,
                        kind: ImageKind::Page(PageDrawing {
                            drawing: o.drawing.clone(),
                            atom: o.atom,
                            polygon: o.flags.polygon.clone(),
                            text_box: o.text_box.clone(),
                            paragraph: idx,
                        }),
                    });
                }
                if joined && let Some(q) = run_prev {
                    for l in lines.iter_mut().filter(|l| l.para == q) {
                        l.join_next = true;
                    }
                }
                pending_bottom = pl.border_bottom();
                run_prev = Some(idx);
                prev = Some(Prev {
                    after: f64::from(pl.props.space_after_pt),
                    contextual: pl.flow.contextual_spacing,
                    style: pl.style_id.clone(),
                });
            }
            Block::Table(t) => {
                y += std::mem::take(&mut pending_bottom);
                run_prev = None;
                y += prev.take().map_or(0.0, |p| p.after);
                let nested = layout_table(b, t, content_w);
                let inner = nested.flatten(0.0, y);
                let base = lines.len();
                lines.extend(inner.lines);
                boxes.extend(inner.boxes);
                images.extend(inner.images.into_iter().map(|i| FlowImage {
                    line: i.line + base,
                    ..i
                }));
                y += inner.height;
            }
            Block::Sdt(_) | Block::Raw(_) => {}
        }
    }
    y += pending_bottom;
    y += prev.map_or(0.0, |p| p.after);
    CellContent {
        lines,
        boxes,
        images,
        height: y,
    }
}

impl CellLayout {
    /// Offset of the content box below the cell's top margin edge for a box `outer_h` high.
    fn valign_shift(&self, outer_h: f64) -> f64 {
        let free = (outer_h - self.pad.top - self.pad.bottom - self.content_h).max(0.0);
        match self.valign {
            VerticalAlign::Top => 0.0,
            VerticalAlign::Center => free / 2.0,
            VerticalAlign::Bottom => free,
        }
    }
}

impl TableLayout {
    /// Lines and boxes of this table as content of an enclosing cell, offset by `(dx, dy)`.
    pub(crate) fn flatten(&self, dx: f64, dy: f64) -> CellContent {
        let mut lines: Vec<FlowLine> = Vec::new();
        let mut boxes = Vec::new();
        let mut images: Vec<FlowImage> = Vec::new();
        for row in &self.rows {
            for cell in &row.cells {
                if cell.row_span == 0 {
                    continue;
                }
                let h = self.span_height(row, cell);
                boxes.push(FlowBox {
                    dx: dx + self.x + cell.x + cell.inset_l,
                    dy: dy + row.top + row.above,
                    w: cell.w - cell.inset_l - cell.inset_r,
                    h,
                    meta: cell.meta,
                });
                for bx in &cell.boxes {
                    boxes.push(FlowBox {
                        dx: bx.dx + dx + self.x + cell.x + cell.pad.left,
                        dy: bx.dy + dy + cell.content_top,
                        ..*bx
                    });
                }
                let first = lines.len();
                for ln in &cell.lines {
                    lines.push(FlowLine {
                        dx: ln.dx + dx + self.x + cell.x + cell.pad.left,
                        dy: ln.dy + dy + cell.content_top,
                        ..*ln
                    });
                }
                for im in &cell.images {
                    images.push(FlowImage {
                        line: im.line + first,
                        dx: im.dx + dx + self.x + cell.x + cell.pad.left,
                        dy: im.dy + dy + cell.content_top,
                        kind: im.kind.clone(),
                    });
                }
            }
        }
        CellContent {
            lines,
            boxes,
            images,
            height: self.height(),
        }
    }

    /// Height of the box `cell` (which starts in `row`) covers.
    pub(crate) fn span_height(&self, row: &RowLayout, cell: &CellLayout) -> f64 {
        let first = self
            .rows
            .iter()
            .position(|r| std::ptr::eq(r, row))
            .unwrap_or(0);
        let covered = &self.rows[first..(first + cell.row_span.max(1)).min(self.rows.len())];
        let total: f64 = covered.iter().map(|r| r.height).sum();
        total - covered.first().map_or(0.0, |r| r.above) - covered.last().map_or(0.0, |r| r.below)
    }
}

/// Lays out `table` in a text area `avail` wide.
pub(crate) fn layout_table(b: &mut Builder<'_>, table: &Table, avail: f64) -> TableLayout {
    let view = table.view();
    let table_id = b.tables;
    b.tables += 1;
    let styles = b.ctx.styles;
    let props: TableProps = match styles {
        Some(s) => s.table_props(&view.props),
        None => view.props.clone(),
    };

    // Grid columns: `w:tblGrid`, extended from the cell widths when a row is wider.
    let mut grid = view.grid.clone();
    let columns = view
        .rows
        .iter()
        .map(|r| {
            r.cells
                .last()
                .map_or(r.props.grid_before, |c| c.col + c.span)
        })
        .max()
        .unwrap_or(0)
        .max(grid.len());
    if grid.len() < columns {
        for row in &view.rows {
            for c in &row.cells {
                if c.span == 1 && c.col >= grid.len() {
                    grid.resize(c.col + 1, 0.0);
                    grid[c.col] = c
                        .props
                        .width
                        .and_then(|w| w.resolve(avail))
                        .unwrap_or(avail / columns.max(1) as f64);
                }
            }
        }
        grid.resize(columns, avail / columns.max(1) as f64);
    }
    let mut col_x = Vec::with_capacity(grid.len() + 1);
    let mut acc = 0.0;
    for w in &grid {
        col_x.push(acc);
        acc += w;
    }
    col_x.push(acc);
    let width = acc;
    let jc = view
        .rows
        .first()
        .and_then(|r| r.props.justification)
        .or(props.justification);
    let left_aligned = !matches!(
        jc,
        Some(TableJustification::Center | TableJustification::Right)
    );
    let x = match jc {
        Some(TableJustification::Center) => (avail - width) / 2.0,
        Some(TableJustification::Right) => avail - width,
        // Word 2010 and earlier measure the indent to the text of the first cell: the left grid
        // line lies one cell margin left of it (layout-float-incell-compat14).
        _ if b.ctx.compat < 15 => {
            props.indent.unwrap_or(0.0) - props.cell_margins.left.unwrap_or(DEFAULT_MARGIN_X)
        }
        _ => props.indent.unwrap_or(0.0),
    };

    let default_margins = CellMargins {
        top: Some(DEFAULT_MARGIN_Y),
        left: Some(DEFAULT_MARGIN_X),
        bottom: Some(DEFAULT_MARGIN_Y),
        right: Some(DEFAULT_MARGIN_X),
    };
    let nrows = view.rows.len();
    let mut rows: Vec<RowLayout> = Vec::with_capacity(nrows);
    for (r, row) in view.rows.iter().enumerate() {
        let header_row = row.props.is_header;
        let row_margins = row
            .exceptions
            .as_ref()
            .map_or(props.cell_margins, |e| {
                e.cell_margins.over(props.cell_margins)
            })
            .over(default_margins);
        let mut cells = Vec::with_capacity(row.cells.len());
        for c in &row.cells {
            let end = (c.col + c.span).min(col_x.len() - 1);
            let start = c.col.min(end);
            let cell_x = col_x[start];
            let cell_w = col_x[end] - cell_x;
            let m = c.props.margins.over(row_margins);
            let pad = Pad {
                top: m.top.unwrap_or(DEFAULT_MARGIN_Y),
                left: m.left.unwrap_or(DEFAULT_MARGIN_X),
                bottom: m.bottom.unwrap_or(DEFAULT_MARGIN_Y),
                right: m.right.unwrap_or(DEFAULT_MARGIN_X),
            };
            let fmt = styles
                .map(|s| {
                    s.table_cell_format(
                        &props,
                        CellPosition {
                            row: r,
                            column: c.col,
                            rows: nrows,
                            columns: grid.len(),
                        },
                    )
                })
                .unwrap_or_default();
            let content_w = (cell_w - pad.left - pad.right).max(1.0);
            let CellContent {
                lines,
                boxes,
                images,
                height: content_h,
            } = layout_cell(b, &c.cell.blocks, content_w, pad, &fmt);
            // Borders: the cell's own, then its table style's, then the table's. A style's
            // cell borders name its region's outer edges (`left`, `right`) and the edges between
            // its cells (`insideV`): a header row that states `insideV` nil has no vertical
            // borders between its cells (layout-table-flow). Top and bottom apply to every
            // cell of a row region.
            let last_col = c.col + c.span >= grid.len();
            let last_row = r + c.row_span.max(1) >= nrows;
            let (own, sty, tb) = (&c.props.borders, &fmt.cell.borders, &props.borders);
            let (sty_top, sty_bottom) = (&sty.top, &sty.bottom);
            let sty_left = if c.col == 0 { &sty.left } else { &sty.inside_v };
            let sty_right = if last_col { &sty.right } else { &sty.inside_v };
            let t_top = if r == 0 { &tb.top } else { &tb.inside_h };
            let t_bottom = if last_row { &tb.bottom } else { &tb.inside_h };
            let t_left = if c.col == 0 { &tb.left } else { &tb.inside_v };
            let t_right = if last_col { &tb.right } else { &tb.inside_v };
            let pick = |a: &Option<Border>, b: &Option<Border>, t: &Option<Border>| {
                border_width(a)
                    .or_else(|| border_width(b))
                    .or_else(|| border_width(t))
                    .unwrap_or(0.0)
            };
            let border_top = pick(&own.top, sty_top, t_top);
            let border_bottom = pick(&own.bottom, sty_bottom, t_bottom);
            let border_left = pick(&own.left, sty_left, t_left);
            let border_right = pick(&own.right, sty_right, t_right);
            let top = resolve_border(&own.top, sty_top, t_top);
            let bottom = resolve_border(&own.bottom, sty_bottom, t_bottom);
            let left = resolve_border(&own.left, sty_left, t_left);
            let right = resolve_border(&own.right, sty_right, t_right);
            let fill = c
                .props
                .shading
                .as_ref()
                .or(fmt.cell.shading.as_ref())
                .and_then(shading_fill);
            let meta = CellMeta {
                table: table_id,
                row: r,
                col: c.col,
                col_span: c.span,
                row_span: c.row_span,
                header_row,
                fill,
                pad_top: pad.top,
                pad_bottom: pad.bottom,
                borders: CellBorders {
                    top,
                    bottom,
                    left,
                    right,
                },
            };
            let valign = c
                .props
                .v_align
                .or(fmt.cell.v_align)
                .unwrap_or(VerticalAlign::Top);
            cells.push(CellLayout {
                x: cell_x,
                w: cell_w,
                row_span: c.row_span,
                pad,
                lines,
                boxes,
                images,
                content_h,
                content_top: 0.0,
                valign,
                inset_l: 0.0,
                inset_r: 0.0,
                meta,
                border_top,
                border_bottom,
                border_left,
                border_right,
            });
        }
        // Measured (corpus): the vertical padding is the row's. Every cell takes the largest top
        // and bottom padding any cell of the row states, for its content position and its clip.
        let (top_pad, bottom_pad) = cells
            .iter()
            .fold((0.0f64, 0.0f64), |(t, bm), c: &CellLayout| {
                (t.max(c.pad.top), bm.max(c.pad.bottom))
            });
        for c in &mut cells {
            c.pad.top = top_pad;
            c.pad.bottom = bottom_pad;
            c.meta.pad_top = top_pad;
            c.meta.pad_bottom = bottom_pad;
        }
        // The row end mark is one more character of the story.
        b.story += 1;
        rows.push(RowLayout {
            top: 0.0,
            height: 0.0,
            above: 0.0,
            below: 0.0,
            cant_split: row.props.cant_split,
            is_header: row.props.is_header,
            exact: matches!(row.props.height, Some((_, HeightRule::Exact))),
            cells,
        });
    }

    // Border geometry. Word puts the grid line in the middle of each border and draws the cell
    // between the borders; vertically a row's height includes the whole border below it and the
    // table's top border sits above the first row (measured with 0.25 to 6 pt borders).
    let mut vlines = vec![0.0f64; col_x.len()];
    let mut hlines = vec![0.0f64; nrows + 1];
    let last_line = vlines.len() - 1;
    for (r, row) in rows.iter().enumerate() {
        for (cv, cell) in view.rows[r].cells.iter().zip(&row.cells) {
            let (a, e) = (cv.col.min(last_line), (cv.col + cv.span).min(last_line));
            vlines[a] = vlines[a].max(cell.border_left);
            vlines[e] = vlines[e].max(cell.border_right);
            if cv.row_span > 0 {
                hlines[r] = hlines[r].max(cell.border_top);
                let end = (r + cv.row_span).min(nrows);
                hlines[end] = hlines[end].max(cell.border_bottom);
                if cv.row_span > 1 {
                    hlines[r + 1] = hlines[r + 1].max(cell.border_bottom);
                }
            }
        }
    }
    for (r, row) in rows.iter_mut().enumerate() {
        row.above = if r == 0 { hlines[0] } else { 0.0 };
        row.below = hlines[r + 1];
        // A cell is drawn between the borders of its own row: the grid line carries the wider
        // of the borders that meet there in this row (layout-table-flow: a header row without
        // vertical borders between its cells has no inset there, the rows below do).
        let mut row_lines = vec![0.0f64; vlines.len()];
        for (cv, cell) in view.rows[r].cells.iter().zip(&row.cells) {
            let (a, e) = (cv.col.min(last_line), (cv.col + cv.span).min(last_line));
            row_lines[a] = row_lines[a].max(cell.border_left);
            row_lines[e] = row_lines[e].max(cell.border_right);
        }
        for (cv, cell) in view.rows[r].cells.iter().zip(row.cells.iter_mut()) {
            cell.inset_l = row_lines[cv.col.min(last_line)] / 2.0;
            cell.inset_r = row_lines[(cv.col + cv.span).min(last_line)] / 2.0;
        }
    }
    // Compatibility mode 15 puts the left border's outer edge on the indent; earlier modes put the
    // grid line one cell margin left of it, whatever the border (text origin 72.0, cells 66.96).
    let x = if left_aligned && b.ctx.compat < 15 {
        x
    } else {
        x + vlines[0] / 2.0
    };

    // Row heights: the tallest single-row cell, then the row height rule, then the extra
    // that vertically merged cells need goes to the last row they cover.
    let need = |c: &CellLayout| c.pad.top + c.content_h + c.pad.bottom;
    for (r, row) in rows.iter_mut().enumerate() {
        let natural = row
            .cells
            .iter()
            .filter(|c| c.row_span == 1)
            .map(need)
            .fold(0.0, f64::max);
        let content = match view.rows[r].props.height {
            // Measured (layout-trh-pad, layout-trh-asym, layout-trh-small): an exact row is its
            // height plus the bottom padding; Word's clip rectangle runs from the top padding to
            // the exact height, and is absent when the height does not exceed the top padding.
            Some((h, HeightRule::Exact)) => row
                .cells
                .iter()
                .filter(|c| c.row_span == 1)
                .map(|c| h + c.pad.bottom)
                .fold(h, f64::max),
            Some((h, HeightRule::AtLeast)) => row
                .cells
                .iter()
                .filter(|c| c.row_span == 1)
                .map(|c| c.pad.top + c.content_h.max(h) + c.pad.bottom)
                .fold(natural.max(h), f64::max),
            _ => natural,
        };
        row.height = row.above + content + row.below;
    }
    for r in 0..rows.len() {
        for c in 0..rows[r].cells.len() {
            let span = rows[r].cells[c].row_span;
            if span > 1 {
                let last = (r + span).min(rows.len()) - 1;
                let total: f64 = rows[r..=last].iter().map(|x| x.height).sum::<f64>()
                    - rows[r].above
                    - rows[last].below;
                let extra = need(&rows[r].cells[c]) - total;
                if extra > 0.0 && !rows[last].exact {
                    rows[last].height += extra;
                }
            }
        }
    }
    let mut top = 0.0;
    for row in &mut rows {
        row.top = top;
        top += row.height;
    }
    // Content boxes, now that every row height is known.
    for r in 0..rows.len() {
        for c in 0..rows[r].cells.len() {
            let span = rows[r].cells[c].row_span;
            if span == 0 {
                continue;
            }
            let last = (r + span).min(rows.len()) - 1;
            let row_top = rows[r].top + rows[r].above;
            let outer = rows[last].top + rows[last].height - rows[last].below - row_top;
            // Measured (corpus): a vertically merged cell takes the bottom padding of the row
            // it ends in; its clip and its centring both stop that far above the cell's end.
            let end_pad = rows[last]
                .cells
                .iter()
                .fold(0.0f64, |m, x| m.max(x.pad.bottom));
            let cell = &mut rows[r].cells[c];
            if span > 1 {
                cell.pad.bottom = end_pad;
                cell.meta.pad_bottom = end_pad;
            }
            cell.content_top = row_top + cell.pad.top + cell.valign_shift(outer);
        }
    }
    let header_rows = rows.iter().take_while(|r| r.is_header).count();
    TableLayout {
        x,
        width,
        edge_l: vlines[0],
        edge_r: vlines[last_line],
        pos: TablePos::read(table),
        rows,
        header_rows,
    }
}
