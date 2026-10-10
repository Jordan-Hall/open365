//! Table layout: column widths, cell content, row heights. Page placement (splitting rows,
//! repeating header rows) is in `flow.rs`.
//!
//! The rules were measured against Word with the `layout-table-*` fixtures; see
//! `docs/architecture/word-layout.md`.

use open365_docx::{
    Block, Cell, CellMargins, CellPosition, HeightRule, Table, TableJustification, TableProps,
    VerticalAlign,
};

use crate::build::Builder;

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
}

/// A nested cell's rectangle inside the content box.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FlowBox {
    pub dx: f64,
    pub dy: f64,
    pub w: f64,
    pub h: f64,
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
    /// Height of the content including the space before the first and after the last paragraph.
    pub content_h: f64,
    /// Top of the content box relative to the table's top, vertical alignment applied.
    pub content_top: f64,
    pub valign: VerticalAlign,
}

#[derive(Debug)]
pub(crate) struct RowLayout {
    pub top: f64,
    pub height: f64,
    pub cant_split: bool,
    pub is_header: bool,
    /// `w:hRule="exact"`: the content is clipped, never split.
    pub exact: bool,
    pub cells: Vec<CellLayout>,
}

#[derive(Debug)]
pub(crate) struct TableLayout {
    /// Left edge relative to the text area.
    pub x: f64,
    pub rows: Vec<RowLayout>,
    /// Leading rows marked `w:tblHeader`.
    pub header_rows: usize,
}

impl TableLayout {
    pub(crate) fn height(&self) -> f64 {
        self.rows.last().map_or(0.0, |r| r.top + r.height)
    }
}

struct Prev {
    after: f64,
    contextual: bool,
    style: Option<String>,
}

/// Content of a cell laid out in a box `content_w` wide.
fn layout_cell(
    b: &mut Builder<'_>,
    cell: &Cell,
    content_w: f64,
    fmt: &open365_docx::TableCellFormat,
) -> (Vec<FlowLine>, Vec<FlowBox>, f64) {
    let mut lines = Vec::new();
    let mut boxes = Vec::new();
    let mut y = 0.0;
    let mut prev: Option<Prev> = None;
    for block in &cell.blocks {
        match block {
            Block::Paragraph(p) => {
                let idx = b.paragraph(p, content_w, Some(fmt));
                let pl = &b.arena[idx];
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
                for (i, lb) in pl.lines.iter().enumerate() {
                    lines.push(FlowLine {
                        para: idx,
                        line: i,
                        dx: 0.0,
                        dy: y,
                        height: lb.height,
                    });
                    y += lb.height;
                }
                prev = Some(Prev {
                    after: f64::from(pl.props.space_after_pt),
                    contextual: pl.flow.contextual_spacing,
                    style: pl.style_id.clone(),
                });
            }
            Block::Table(t) => {
                y += prev.take().map_or(0.0, |p| p.after);
                let nested = layout_table(b, t, content_w);
                let (l, bx, h) = nested.flatten(0.0, y);
                lines.extend(l);
                boxes.extend(bx);
                y += h;
            }
            Block::Raw(_) => {}
        }
    }
    y += prev.map_or(0.0, |p| p.after);
    (lines, boxes, y)
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
    pub(crate) fn flatten(&self, dx: f64, dy: f64) -> (Vec<FlowLine>, Vec<FlowBox>, f64) {
        let mut lines = Vec::new();
        let mut boxes = Vec::new();
        for row in &self.rows {
            for cell in &row.cells {
                if cell.row_span == 0 {
                    continue;
                }
                let h = self.span_height(row, cell);
                boxes.push(FlowBox {
                    dx: dx + self.x + cell.x,
                    dy: dy + row.top,
                    w: cell.w,
                    h,
                });
                for bx in &cell.boxes {
                    boxes.push(FlowBox {
                        dx: bx.dx + dx + self.x + cell.x + cell.pad.left,
                        dy: bx.dy + dy + cell.content_top,
                        ..*bx
                    });
                }
                for ln in &cell.lines {
                    lines.push(FlowLine {
                        dx: ln.dx + dx + self.x + cell.x + cell.pad.left,
                        dy: ln.dy + dy + cell.content_top,
                        ..*ln
                    });
                }
            }
        }
        (lines, boxes, self.height())
    }

    /// Height of the box `cell` (which starts in `row`) covers.
    pub(crate) fn span_height(&self, row: &RowLayout, cell: &CellLayout) -> f64 {
        let first = self
            .rows
            .iter()
            .position(|r| std::ptr::eq(r, row))
            .unwrap_or(0);
        self.rows[first..(first + cell.row_span.max(1)).min(self.rows.len())]
            .iter()
            .map(|r| r.height)
            .sum()
    }
}

/// Lays out `table` in a text area `avail` wide.
pub(crate) fn layout_table(b: &mut Builder<'_>, table: &Table, avail: f64) -> TableLayout {
    let view = table.view();
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
    let x = match jc {
        Some(TableJustification::Center) => (avail - width) / 2.0,
        Some(TableJustification::Right) => avail - width,
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
            let (lines, boxes, content_h) = layout_cell(b, c.cell, content_w, &fmt);
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
                content_h,
                content_top: 0.0,
                valign,
            });
        }
        // The row end mark is one more character of the story.
        b.story += 1;
        rows.push(RowLayout {
            top: 0.0,
            height: 0.0,
            cant_split: row.props.cant_split,
            is_header: row.props.is_header,
            exact: matches!(row.props.height, Some((_, HeightRule::Exact))),
            cells,
        });
    }

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
        row.height = match view.rows[r].props.height {
            Some((h, HeightRule::Exact)) => h,
            Some((h, HeightRule::AtLeast)) => natural.max(h),
            _ => natural,
        };
    }
    for r in 0..rows.len() {
        for c in 0..rows[r].cells.len() {
            let span = rows[r].cells[c].row_span;
            if span > 1 {
                let last = (r + span).min(rows.len()) - 1;
                let total: f64 = rows[r..=last].iter().map(|x| x.height).sum();
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
            let outer = rows[last].top + rows[last].height - rows[r].top;
            let row_top = rows[r].top;
            let cell = &mut rows[r].cells[c];
            cell.content_top = row_top + cell.pad.top + cell.valign_shift(outer);
        }
    }
    let header_rows = rows.iter().take_while(|r| r.is_header).count();
    TableLayout {
        x,
        rows,
        header_rows,
    }
}
