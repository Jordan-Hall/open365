//! Placeholder document areas drawn straight with `open365-gfx`: a Word-like page and an
//! Excel-like grid. They stand in until the canvas and grid of phase 3 are ported.

use open365_gfx::{Color, Decorations, DrawList, Rect, TextLayoutOptions};
use open365_ui_core::Ui;

use crate::layout::PxRect;
use crate::palette::Palette;
use crate::spec::REF_SCALE;

fn gr(r: PxRect) -> Rect {
    Rect::from_edges(r.x as f32, r.y as f32, r.right() as f32, r.bottom() as f32)
}

/// Draws one line of text with its baseline at `base`.
pub fn text(ui: &mut Ui, list: &mut DrawList, s: &str, x: f32, base: f32, px: f32, color: Color) {
    let lay = ui
        .assets
        .layout_text(ui.font, s, px, x, base, TextLayoutOptions::default());
    list.text(
        &mut ui.assets,
        ui.font,
        &lay,
        px,
        color,
        Decorations::default(),
    );
}

fn width(ui: &Ui, s: &str, px: f32) -> f32 {
    ui.assets
        .layout_text(ui.font, s, px, 0.0, 0.0, TextLayoutOptions::default())
        .width
}

/// Baseline that centres a line of `px` text in `top..bottom`.
fn centred(ui: &Ui, top: f32, bottom: f32, px: f32) -> f32 {
    match ui.assets.font_metrics(ui.font, px) {
        Some(m) => (top + (bottom - top - (m.ascent + m.descent)) / 2.0 + m.ascent).round(),
        None => bottom,
    }
}

const BODY: [&str; 6] = [
    "Open365 Writer preview",
    "",
    "This page is a placeholder drawn with the gfx renderer. The ribbon, tabs, Backstage",
    "view and status bar around it are the real chrome on the new UI platform.",
    "",
    "Hover and press the ribbon, switch tabs, open drop-downs, press Alt for key tips,",
];

/// The Writer canvas: a page on the canvas colour.
pub fn paint_writer(
    ui: &mut Ui,
    list: &mut DrawList,
    area: PxRect,
    pal: &Palette,
    scale: f32,
    zoom: f32,
) {
    list.push_clip(gr(area));
    list.fill_rect(gr(area), pal.canvas);
    let k = scale * zoom;
    let (pw, ph) = ((816.0 * k).round(), (1056.0 * k).round());
    let x = (area.x as f32 + ((area.w as f32 - pw) / 2.0).max(8.0 * scale)).round();
    let mut y = (area.y as f32 + 12.0 * k).round();
    for page in 0..2 {
        let rect = Rect::from_edges(x, y, x + pw, y + ph);
        list.fill_rect(rect, pal.page);
        if let Some(b) = pal.page_border {
            list.rect_outline(rect, 1.0, b);
        }
        if page == 0 {
            let margin = (96.0 * k).round();
            let px = 14.67 * k;
            let lh = (px * 1.45).round();
            let mut base = y + margin + px;
            for (i, line) in BODY.iter().enumerate() {
                let size = if i == 0 { px * 1.8 } else { px };
                text(
                    ui,
                    list,
                    line,
                    x + margin,
                    base.round(),
                    size,
                    pal.page_text,
                );
                base += if i == 0 { lh * 1.6 } else { lh };
            }
            // Greeked paragraphs.
            let bar = pal.page_text.with_opacity(0.18);
            for row in 0..14 {
                let w = pw - 2.0 * margin - if row % 5 == 4 { 120.0 * k } else { 0.0 };
                let top = (base + row as f32 * lh - px * 0.5).round();
                list.fill_rect(
                    Rect::from_edges(x + margin, top, x + margin + w, top + (px * 0.55).round()),
                    bar,
                );
            }
        }
        y += ph + (12.0 * k).round();
    }
    list.pop_clip();
}

fn col_name(mut i: usize) -> String {
    let mut s = String::new();
    loop {
        s.insert(0, (b'A' + (i % 26) as u8) as char);
        if i < 26 {
            return s;
        }
        i = i / 26 - 1;
    }
}

/// Device-pixel geometry of the sheet grid (the same numbers paint and hit testing use).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridGeom {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub row_header_w: f32,
    pub col_header_h: f32,
    pub col_w: f32,
    pub row_h: f32,
}

impl GridGeom {
    pub fn new(area: PxRect, scale: f32, header: (i32, i32)) -> Self {
        let u = |v: i32| (v as f32 * scale / REF_SCALE).round();
        Self {
            x0: area.x as f32,
            y0: area.y as f32,
            x1: area.right() as f32,
            y1: area.bottom() as f32,
            row_header_w: u(header.0),
            col_header_h: u(header.1),
            col_w: u(80),
            row_h: u(25),
        }
    }

    /// Whole rows and columns that fit (at least one each).
    pub fn visible(&self) -> (u32, u32) {
        let rows = ((self.y1 - self.y0 - self.col_header_h) / self.row_h).floor();
        let cols = ((self.x1 - self.x0 - self.row_header_w) / self.col_w).floor();
        (rows.max(1.0) as u32, cols.max(1.0) as u32)
    }

    /// Cell rectangle `(left, top, right, bottom)` of 1-based `(row, col)` with `top_left` as
    /// the first visible cell; the right and bottom edges include the grid line.
    pub fn cell(&self, top_left: (u32, u32), row: u32, col: u32) -> (f32, f32, f32, f32) {
        let l = self.x0 + self.row_header_w + (col as f32 - top_left.1 as f32) * self.col_w;
        let t = self.y0 + self.col_header_h + (row as f32 - top_left.0 as f32) * self.row_h;
        (l, t, l + self.col_w, t + self.row_h)
    }

    /// The cell under device point `(x, y)`, or `None` over the headers or outside the grid.
    pub fn hit(&self, top_left: (u32, u32), x: f32, y: f32) -> Option<(u32, u32)> {
        let (gx, gy) = (
            x - self.x0 - self.row_header_w,
            y - self.y0 - self.col_header_h,
        );
        if gx < 0.0 || gy < 0.0 || x >= self.x1 || y >= self.y1 {
            return None;
        }
        Some((
            top_left.0 + (gy / self.row_h) as u32,
            top_left.1 + (gx / self.col_w) as u32,
        ))
    }
}

/// What the Sheets canvas shows besides the grid itself.
#[derive(Debug)]
pub struct SheetView<'a> {
    pub model: &'a crate::sheet::SheetModel,
    /// The active cell is covered by the in-cell editor: draw no text in it.
    pub editing: bool,
}

/// The Sheets canvas: headers, grid lines, cell values and the active cell.
pub fn paint_sheet(
    ui: &mut Ui,
    list: &mut DrawList,
    area: PxRect,
    pal: &Palette,
    scale: f32,
    header: (i32, i32),
    view: &SheetView<'_>,
) {
    let g = GridGeom::new(area, scale, header);
    let model = view.model;
    let top_left = model.top_left;
    let (active_row, active_col) = model.active;
    let (row_header_w, col_header_h, col_w, row_h) =
        (g.row_header_w, g.col_header_h, g.col_w, g.row_h);
    list.push_clip(gr(area));
    list.fill_rect(gr(area), pal.cell_bg);
    let (x0, y0, x1, y1) = (g.x0, g.y0, g.x1, g.y1);
    list.fill_rect(
        Rect::from_edges(x0, y0, x1, y0 + col_header_h),
        pal.grid_header,
    );
    list.fill_rect(
        Rect::from_edges(x0, y0, x0 + row_header_w, y1),
        pal.grid_header,
    );
    let px = 12.0 * scale;
    let cols = ((x1 - x0 - row_header_w) / col_w).ceil() as usize + 1;
    let rows = ((y1 - y0 - col_header_h) / row_h).ceil() as usize + 1;
    // Active cell: header highlight.
    let (al, at, ar, ab) = g.cell(top_left, active_row, active_col);
    list.fill_rect(
        Rect::from_edges(al, y0, ar, y0 + col_header_h),
        pal.grid_header_selected,
    );
    list.fill_rect(
        Rect::from_edges(x0, at, x0 + row_header_w, ab),
        pal.grid_header_selected,
    );
    list.fill_rect(
        Rect::from_edges(al, y0 + col_header_h - 2.0, ar, y0 + col_header_h),
        pal.grid_accent,
    );
    // Grid lines.
    for c in 0..=cols {
        let x = x0 + row_header_w + c as f32 * col_w - 1.0;
        list.fill_rect(
            Rect::from_edges(x, y0 + col_header_h, x + 1.0, y1),
            pal.grid_line,
        );
        list.fill_rect(
            Rect::from_edges(x, y0, x + 1.0, y0 + col_header_h),
            pal.grid_header_border,
        );
    }
    for r in 0..=rows {
        let y = y0 + col_header_h + r as f32 * row_h - 1.0;
        list.fill_rect(
            Rect::from_edges(x0 + row_header_w, y, x1, y + 1.0),
            pal.grid_line,
        );
        list.fill_rect(
            Rect::from_edges(x0, y, x0 + row_header_w, y + 1.0),
            pal.grid_header_border,
        );
    }
    for c in 0..cols {
        let name = col_name(top_left.1 as usize - 1 + c);
        let w = width(ui, &name, px);
        let left = x0 + row_header_w + c as f32 * col_w;
        let base = centred(ui, y0, y0 + col_header_h, px);
        text(
            ui,
            list,
            &name,
            (left + (col_w - w) / 2.0).floor(),
            base,
            px,
            pal.grid_header_text,
        );
    }
    for r in 0..rows {
        let name = (top_left.0 as usize + r).to_string();
        let w = width(ui, &name, px);
        let top = y0 + col_header_h + r as f32 * row_h;
        let base = centred(ui, top, top + row_h, px);
        text(
            ui,
            list,
            &name,
            (x0 + (row_header_w - w) / 2.0).floor(),
            base,
            px,
            pal.grid_header_text,
        );
    }
    // Cell values, clipped to the grid body and to their own cell.
    let cell_px = 15.0 * scale / 1.25;
    for r in 0..rows {
        for c in 0..cols {
            let (row, col) = (top_left.0 + r as u32, top_left.1 + c as u32);
            if view.editing && (row, col) == model.active {
                continue;
            }
            let Some((value, numeric)) = model.shown(row, col) else {
                continue;
            };
            let (left, top, right, bottom) = g.cell(top_left, row, col);
            let base = centred(ui, top, bottom, cell_px);
            let w = width(ui, &value, cell_px);
            let tx = if numeric {
                right - 4.0 * scale - w
            } else {
                left + 4.0 * scale
            };
            list.push_clip(Rect::from_edges(
                left.max(x0 + row_header_w),
                top.max(y0 + col_header_h),
                right - 1.0,
                bottom - 1.0,
            ));
            text(ui, list, &value, tx.floor(), base, cell_px, pal.cell_text);
            list.pop_clip();
        }
    }
    // Active cell outline.
    let sel = Rect::from_edges(al - 1.0, at - 1.0, ar, ab);
    list.rect_outline(sel, 2.0, pal.grid_accent);
    list.pop_clip();
}
