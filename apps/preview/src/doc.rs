//! Placeholder document areas drawn straight with `open365-gfx`: a Word-like page and an
//! Excel-like grid. They stand in until the canvas and grid of phase 3 are ported.

use open365_gfx::{Color, Decorations, DrawList, Rect, TextLayoutOptions};
use open365_ui_core::Ui;

// The Writer document pane (editable, paged) and the egui-free pieces of the egui Writer it
// shares by path so the paragraph model cannot drift.
#[path = "../../writer/src/blank.rs"]
pub mod blank;
#[path = "../../../crates/ui-egui/src/docmodel.rs"]
pub mod docmodel;
#[path = "../../../crates/ui-egui/src/parts.rs"]
mod parts;
#[path = "../../../crates/ui-egui/src/ruler.rs"]
pub mod ruler;
pub mod sheetchrome;
mod writer;

pub use writer::{CanvasAccess, Format, Nav, PaintStats, PathChooser, Toggle, WriterPane};

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
    "Open Write preview",
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

/// Dashed outline of a rectangle (the fill preview).
fn dashed(list: &mut DrawList, r: Rect, color: Color, dash: f32) {
    let mut x = r.x0;
    while x < r.x1 {
        let e = (x + dash).min(r.x1);
        list.fill_rect(Rect::from_edges(x, r.y0, e, r.y0 + 1.0), color);
        list.fill_rect(Rect::from_edges(x, r.y1 - 1.0, e, r.y1), color);
        x += dash * 2.0;
    }
    let mut y = r.y0;
    while y < r.y1 {
        let e = (y + dash).min(r.y1);
        list.fill_rect(Rect::from_edges(r.x0, y, r.x0 + 1.0, e), color);
        list.fill_rect(Rect::from_edges(r.x1 - 1.0, y, r.x1, e), color);
        y += dash * 2.0;
    }
}

/// Dashed outline of a rectangle whose dash runs start `phase` pixels before the corner
/// (marching ants). Each run is clipped to the rectangle.
fn dashed_phase(list: &mut DrawList, r: Rect, color: Color, dash: f32, phase: f32) {
    let mut x = r.x0 - phase;
    while x < r.x1 {
        let (s, e) = (x.max(r.x0), (x + dash).min(r.x1));
        if e > s {
            list.fill_rect(Rect::from_edges(s, r.y0, e, r.y0 + 1.0), color);
            list.fill_rect(Rect::from_edges(s, r.y1 - 1.0, e, r.y1), color);
        }
        x += dash * 2.0;
    }
    let mut y = r.y0 - phase;
    while y < r.y1 {
        let (s, e) = (y.max(r.y0), (y + dash).min(r.y1));
        if e > s {
            list.fill_rect(Rect::from_edges(r.x0, s, r.x0 + 1.0, e), color);
            list.fill_rect(Rect::from_edges(r.x1 - 1.0, s, r.x1, e), color);
        }
        y += dash * 2.0;
    }
}

/// The Sheets canvas: headers, grid lines with the sheet's own column widths and row heights,
/// cell values, the selected range, the active cell and the resize or fill preview.
pub fn paint_sheet(
    ui: &mut Ui,
    list: &mut DrawList,
    area: PxRect,
    pal: &Palette,
    scale: f32,
    header: (i32, i32),
    view: &SheetView<'_>,
) {
    use crate::sheet::Gesture;
    let model = view.model;
    if sheetchrome::paint_layout_view(ui, list, pal, area, scale, model) {
        return;
    }
    let g = sheetchrome::sheet_geo(area, header, scale, model);
    let headings = model.ext_view.headings(model.sheet);
    let gridlines = model.ext_view.gridlines(model.sheet);
    let (row_header_w, col_header_h) = (g.row_header_w, g.col_header_h);
    let (x0, y0, x1, y1) = (g.x0, g.y0, g.x1, g.y1);
    let (bx, by) = (x0 + row_header_w, y0 + col_header_h);
    list.push_clip(gr(area));
    list.fill_rect(gr(area), pal.cell_bg);
    list.fill_rect(Rect::from_edges(x0, y0, x1, by), pal.grid_header);
    list.fill_rect(Rect::from_edges(x0, y0, bx, y1), pal.grid_header);
    let px = 12.0 * scale;
    // The frozen columns and rows, then the scrolling ones up to one past the edge so a partly
    // visible one is drawn.
    let cols = g.drawn_cols();
    let rows = g.drawn_rows();
    let (r1, c1, r2, c2) = model.range();
    // Header highlight for every selected column and row.
    for &c in cols.iter().filter(|c| (c1..=c2).contains(c)) {
        let (l, r) = (g.col_left(c).max(g.min_x(c)), g.col_right(c));
        list.fill_rect(Rect::from_edges(l, y0, r, by), pal.grid_header_selected);
        list.fill_rect(Rect::from_edges(l, by - 2.0, r, by), pal.grid_accent);
    }
    for &r in rows.iter().filter(|r| (r1..=r2).contains(r)) {
        let (t, b) = (g.row_top(r).max(g.min_y(r)), g.row_bottom(r));
        list.fill_rect(Rect::from_edges(x0, t, bx, b), pal.grid_header_selected);
    }
    // Grid lines (the sheet's own View > Gridlines setting) and the heading borders.
    for &c in &cols {
        let x = g.col_right(c) - 1.0;
        if gridlines {
            list.fill_rect(Rect::from_edges(x, g.min_y(1), x + 1.0, y1), pal.grid_line);
        }
        list.fill_rect(Rect::from_edges(x, y0, x + 1.0, by), pal.grid_header_border);
    }
    for &r in &rows {
        let y = g.row_bottom(r) - 1.0;
        if gridlines {
            list.fill_rect(Rect::from_edges(g.min_x(1), y, x1, y + 1.0), pal.grid_line);
        }
        list.fill_rect(Rect::from_edges(x0, y, bx, y + 1.0), pal.grid_header_border);
    }
    for &c in cols.iter().filter(|_| headings) {
        let name = col_name(c as usize - 1);
        let w = width(ui, &name, px);
        let (left, right) = (g.col_left(c).max(g.min_x(c)), g.col_right(c));
        let base = centred(ui, y0, by, px);
        text(
            ui,
            list,
            &name,
            (left + (right - left - w) / 2.0).floor(),
            base,
            px,
            pal.grid_header_text,
        );
    }
    for &r in rows.iter().filter(|_| headings) {
        let name = r.to_string();
        let w = width(ui, &name, px);
        let (top, bottom) = (g.row_top(r).max(g.min_y(r)), g.row_bottom(r));
        let base = centred(ui, top, bottom, px);
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
    for &row in &rows {
        for &col in &cols {
            if view.editing && (row, col) == model.active {
                continue;
            }
            let Some((value, numeric)) = model.shown_for_view(row, col) else {
                continue;
            };
            let (left, top, right, bottom) = g.cell(row, col);
            let base = centred(ui, top, bottom, cell_px);
            let w = width(ui, &value, cell_px);
            let tx = if numeric {
                right - 4.0 * scale - w
            } else {
                left + 4.0 * scale
            };
            list.push_clip(Rect::from_edges(
                left.max(g.min_x(col)),
                top.max(g.min_y(row)),
                right - 1.0,
                bottom - 1.0,
            ));
            text(ui, list, &value, tx.floor(), base, cell_px, pal.cell_text);
            list.pop_clip();
        }
    }
    // The split of frozen panes: a thin dark line along the frozen rows and columns.
    let (fr, fc) = g.frozen();
    let split = pal.cell_text.with_opacity(0.55);
    if fc > 0 {
        let x = g.scroll_x0() - 1.0;
        list.fill_rect(Rect::from_edges(x, by, x + 1.0, y1), split);
    }
    if fr > 0 {
        let y = g.scroll_y0() - 1.0;
        list.fill_rect(Rect::from_edges(bx, y, x1, y + 1.0), split);
    }
    sheetchrome::paint_under(ui, list, pal, &g, scale, model);
    crate::sheet_ext::drawing::paint(ui, list, &g, model);
    // The selected range: a light wash over the body, the active cell left clear.
    list.push_clip(Rect::from_edges(bx, by, x1, y1));
    let (rl, rt, rr, rb) = g.range_rect((r1, c1, r2, c2));
    if (r1, c1) != (r2, c2) {
        let wash = pal.grid_accent.with_opacity(0.12);
        let (al, at, ar, ab) = g.cell(model.active.0, model.active.1);
        for (l, t, r, b) in [
            (rl, rt, rr, at),
            (rl, ab, rr, rb),
            (rl, at, al, ab),
            (ar, at, rr, ab),
        ] {
            if r > l && b > t {
                list.fill_rect(Rect::from_edges(l, t, r, b), wash);
            }
        }
    }
    let (al, at, ar, ab) = g.cell(model.active.0, model.active.1);
    list.rect_outline(
        Rect::from_edges(al - 1.0, at - 1.0, ar, ab),
        2.0,
        pal.grid_accent,
    );
    if (r1, c1) != (r2, c2) {
        list.rect_outline(
            Rect::from_edges(rl - 1.0, rt - 1.0, rr, rb),
            2.0,
            pal.grid_accent,
        );
    }
    // Marching ants: a dashed outline round the copied or cut range until Esc or a paste.
    if let Some(copied) = model.copied_range() {
        let (l, t, r, b) = g.range_rect(copied);
        let phase = crate::sheet::ants_phase(ui.anim_frame(), 4.0 * scale);
        dashed_phase(
            list,
            Rect::from_edges(l, t, r, b),
            pal.grid_accent,
            4.0 * scale,
            phase,
        );
    }
    // Markers, AutoFilter and validation buttons, formula trace arrows.
    sheetchrome::paint_over(ui, list, pal, &g, scale, model);
    // Fill handle at the bottom right corner of the range.
    let hs = (6.0 * scale / REF_SCALE).round().max(3.0);
    list.fill_rect(
        Rect::from_edges(
            rr - hs / 2.0 - 1.0,
            rb - hs / 2.0 - 1.0,
            rr + hs / 2.0 - 1.0,
            rb + hs / 2.0 - 1.0,
        ),
        pal.grid_accent,
    );
    // Previews of the gesture in progress.
    match model.gesture {
        Some(Gesture::ResizeCol { col, px: w, .. }) => {
            let x = g.col_left(col) + (w as f32 * scale).round() - 1.0;
            list.fill_rect(Rect::from_edges(x, y0, x + 1.0, y1), pal.cell_text);
        }
        Some(Gesture::ResizeRow { row, px: h, .. }) => {
            let y = g.row_top(row) + (h as f32 * scale).round() - 1.0;
            list.fill_rect(Rect::from_edges(x0, y, x1, y + 1.0), pal.cell_text);
        }
        Some(Gesture::Fill { to }) => {
            let (fr1, fc1, fr2, fc2) = (r1.min(to.0), c1.min(to.1), r2.max(to.0), c2.max(to.1));
            let (l, t, r, b) = g.range_rect((fr1, fc1, fr2, fc2));
            dashed(
                list,
                Rect::from_edges(l, t, r, b),
                pal.cell_text,
                4.0 * scale,
            );
        }
        _ => {}
    }
    list.pop_clip();
    // The outline gutter and the note or comment under the pointer sit over everything.
    sheetchrome::paint_gutter_and_hover(ui, list, pal, &g, scale, model);
    list.pop_clip();
}
