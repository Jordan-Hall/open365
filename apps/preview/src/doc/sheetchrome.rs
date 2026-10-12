//! What the Sheets canvas draws besides cells, headings and grid lines: AutoFilter buttons, the
//! in-cell validation arrow, note and comment markers with their hover popup, the outline
//! gutter, formula-trace arrows, Page Break Preview and the Page Layout view.
//!
//! The geometry (button rectangles, gutter widths, page splits) is plain functions of the model
//! and the device scale, so painting and the pointer hit tests in `shell/grid.rs` agree. Pixel
//! sizes are Excel's at 96 dpi and are multiplied by the device scale.

use std::collections::BTreeSet;

use open365_gfx::{Color, DrawList, Path, Rect};
use open365_ui_core::Ui;

use super::{centred, gr, text, width};
use crate::layout::PxRect;
use crate::palette::Palette;
use crate::sheet::{Geo, GridMarks, OutlineGroup, SheetModel};
use crate::sheet_ext::page_layout::SheetPage;
use crate::sheet_ext::view::ViewMode;
use crate::spec::REF_SCALE;

/// Side of the AutoFilter and validation buttons.
const BUTTON: f32 = 17.0;
/// Width of one outline level in the gutter, and the side of a level button.
const LEVEL: f32 = 14.0;
const LEVEL_BUTTON: f32 = 12.0;
/// Leg of the note and comment marker triangle.
const MARKER: f32 = 6.0;

const NOTE_RED: Color = Color::rgb(0xff, 0x00, 0x00);
const COMMENT_PURPLE: Color = Color::rgb(0x87, 0x64, 0xb8);
const TRACE_BLUE: Color = Color::rgb(0x1e, 0x5f, 0xcc);
const TRACE_RED: Color = Color::rgb(0xe0, 0x1b, 0x1b);
const BREAK_BLUE: Color = Color::rgb(0x00, 0x4e, 0xc8);
const OUTSIDE_GREY: Color = Color::rgb(0xab, 0xab, 0xab);

fn dp(scale: f32, v: f32) -> f32 {
    (v * scale).round()
}

fn rect(l: f32, t: f32, r: f32, b: f32) -> Rect {
    Rect::from_edges(l, t, r, b)
}

fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r.x0 && x < r.x1 && y >= r.y0 && y < r.y1
}

/// Width and height of the outline gutters (one band per level, plus one for the level buttons'
/// extra "show all" button) (left of the row headings, above the column headings).
pub fn gutter(marks: &GridMarks, scale: f32) -> (f32, f32) {
    let size = |levels: u32| {
        if levels == 0 {
            0.0
        } else {
            dp(scale, LEVEL) * (levels + 1) as f32 + dp(scale, 2.0)
        }
    };
    (size(marks.row_levels), size(marks.col_levels))
}

/// The device geometry of the grid inside `area`: headings and the outline gutter decide where
/// the cells start. `header` is the heading size in logical units at 125 %.
pub fn sheet_geo(area: PxRect, header: (i32, i32), scale: f32, model: &SheetModel) -> Geo {
    let u = |v: i32| (v as f32 * scale / REF_SCALE).round();
    let marks = model.grid_marks();
    let (gw, gh) = gutter(&marks, scale);
    let (hw, hh) = if model.ext_view.headings(model.sheet) {
        (u(header.0), u(header.1))
    } else {
        (0.0, 0.0)
    };
    Geo::new(
        (
            area.x as f32 + gw,
            area.y as f32 + gh,
            area.right() as f32,
            area.bottom() as f32,
        ),
        (hw, hh),
        scale,
        model.geometry(),
        model.top_left,
    )
}

/// Cells whose row and column are on screen: `(row, col)` is drawn when both are in the lists.
struct Visible {
    rows: Vec<u32>,
    cols: Vec<u32>,
}

impl Visible {
    fn new(g: &Geo) -> Self {
        Self {
            rows: g.drawn_rows(),
            cols: g.drawn_cols(),
        }
    }

    fn has(&self, row: u32, col: u32) -> bool {
        self.rows.binary_search(&row).is_ok() && self.cols.binary_search(&col).is_ok()
    }
}

// ---- AutoFilter and validation buttons ------------------------------------------------------

/// The AutoFilter button in the header cell of `col`: flush with the bottom right of the cell.
pub fn filter_button_rect(g: &Geo, scale: f32, row: u32, col: u32) -> Rect {
    let (l, t, r, b) = g.cell(row, col);
    let side = dp(scale, BUTTON).min(b - t - 2.0).min(r - l - 2.0).max(4.0);
    rect(r - 1.0 - side, b - 1.0 - side, r - 1.0, b - 1.0)
}

/// The columns whose AutoFilter button is on screen, with its rectangle.
pub fn filter_buttons(g: &Geo, scale: f32, marks: &GridMarks) -> Vec<(u32, Rect)> {
    let Some((range, _)) = &marks.filter else {
        return Vec::new();
    };
    let vis = Visible::new(g);
    (range.1..=range.3)
        .filter(|&c| vis.has(range.0, c))
        .map(|c| (c, filter_button_rect(g, scale, range.0, c)))
        .collect()
}

/// The AutoFilter column whose button is under the device point.
pub fn filter_button_at(
    g: &Geo,
    scale: f32,
    marks: &GridMarks,
    x: f32,
    y: f32,
) -> Option<(u32, Rect)> {
    let (range, _) = marks.filter.as_ref()?;
    let body = body_clip(g);
    if !inside(body, x, y) {
        return None;
    }
    filter_buttons(g, scale, marks).into_iter().find(|&(c, r)| {
        inside(r, x, y) && inside(rect(g.min_x(c), g.min_y(range.0), g.x1, g.y1), x, y)
    })
}

/// The in-cell validation arrow of the active cell: the full height of the cell, 17 px wide.
pub fn validation_button_rect(g: &Geo, scale: f32, cell: (u32, u32)) -> Rect {
    let (l, t, r, b) = g.cell(cell.0, cell.1);
    let w = dp(scale, BUTTON).min(r - l - 2.0).max(4.0);
    rect(r - 1.0 - w, t, r - 1.0, b - 1.0)
}

fn body_clip(g: &Geo) -> Rect {
    rect(g.x0 + g.row_header_w, g.y0 + g.col_header_h, g.x1, g.y1)
}

fn down_triangle(ui: &mut Ui, list: &mut DrawList, cx: f32, cy: f32, half: f32, color: Color) {
    let mut p = Path::new();
    p.move_to(cx - half, cy - half / 2.0)
        .line_to(cx + half, cy - half / 2.0)
        .line_to(cx, cy + half / 2.0 + 0.5)
        .close();
    list.fill_path(&mut ui.assets, &p, color);
}

/// A small funnel: the mark of a filtered column.
fn funnel(ui: &mut Ui, list: &mut DrawList, cx: f32, cy: f32, size: f32, color: Color) {
    let h = size / 2.0;
    let mut p = Path::new();
    p.move_to(cx - h, cy - h)
        .line_to(cx + h, cy - h)
        .line_to(cx + h * 0.2, cy)
        .line_to(cx + h * 0.2, cy + h)
        .line_to(cx - h * 0.2, cy + h * 0.6)
        .line_to(cx - h * 0.2, cy)
        .close();
    list.fill_path(&mut ui.assets, &p, color);
}

fn paint_filter_buttons(
    ui: &mut Ui,
    list: &mut DrawList,
    pal: &Palette,
    g: &Geo,
    scale: f32,
    marks: &GridMarks,
) {
    let Some((range, filtered)) = &marks.filter else {
        return;
    };
    for (c, r) in filter_buttons(g, scale, marks) {
        list.push_clip(rect(g.min_x(c), g.min_y(range.0), g.x1, g.y1));
        list.fill_rect(r, pal.grid_header);
        list.rect_outline(r, 1.0, pal.grid_header_border);
        let (cx, cy) = ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0);
        if filtered.contains(&c) {
            funnel(ui, list, cx, cy, dp(scale, 9.0), pal.grid_header_text);
        } else {
            down_triangle(ui, list, cx, cy, dp(scale, 3.5), pal.grid_header_text);
        }
        list.pop_clip();
    }
}

fn paint_validation_arrow(
    ui: &mut Ui,
    list: &mut DrawList,
    pal: &Palette,
    g: &Geo,
    scale: f32,
    model: &SheetModel,
) {
    let cell = model.active;
    if model.data_validation_list(cell.0, cell.1).is_empty() {
        return;
    }
    let r = validation_button_rect(g, scale, cell);
    list.push_clip(rect(g.min_x(cell.1), g.min_y(cell.0), g.x1, g.y1));
    list.fill_rect(r, pal.grid_header);
    list.rect_outline(r, 1.0, pal.grid_header_border);
    down_triangle(
        ui,
        list,
        (r.x0 + r.x1) / 2.0,
        (r.y0 + r.y1) / 2.0,
        dp(scale, 3.5),
        pal.grid_header_text,
    );
    list.pop_clip();
}

// ---- notes and comments ---------------------------------------------------------------------

fn corner(list: &mut DrawList, ui: &mut Ui, right: f32, top: f32, leg: f32, color: Color) {
    let mut p = Path::new();
    p.move_to(right - leg, top)
        .line_to(right, top)
        .line_to(right, top + leg)
        .close();
    list.fill_path(&mut ui.assets, &p, color);
}

fn paint_markers(ui: &mut Ui, list: &mut DrawList, g: &Geo, scale: f32, marks: &GridMarks) {
    let vis = Visible::new(g);
    let leg = dp(scale, MARKER);
    let cells = marks
        .notes
        .iter()
        .map(|n| (n.row, n.col, NOTE_RED))
        .chain(marks.threads.iter().map(|t| (t.row, t.col, COMMENT_PURPLE)));
    for (row, col, color) in cells {
        if !vis.has(row, col) {
            continue;
        }
        let (_, t, r, _) = g.cell(row, col);
        list.push_clip(rect(g.min_x(col), g.min_y(row), g.x1, g.y1));
        corner(list, ui, r - 1.0, t, leg, color);
        list.pop_clip();
    }
}

/// Greedy word wrap of `s` to `max` pixels at size `px`.
fn wrap(ui: &Ui, s: &str, px: f32, max: f32) -> Vec<String> {
    let mut out = Vec::new();
    for para in s.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let trial = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && width(ui, &trial, px) > max {
                out.push(std::mem::take(&mut line));
                line = word.to_owned();
            } else {
                line = trial;
            }
        }
        out.push(line);
    }
    out
}

/// The hover popup of a note (yellow) or a comment (white card) under the pointer.
fn paint_hover(
    ui: &mut Ui,
    list: &mut DrawList,
    pal: &Palette,
    g: &Geo,
    scale: f32,
    marks: &GridMarks,
) {
    let p = ui.pointer();
    let (x, y) = (p.x * scale, p.y * scale);
    if !inside(body_clip(g), x, y) || (marks.notes.is_empty() && marks.threads.is_empty()) {
        return;
    }
    let Some((row, col)) = g.hit(x, y) else {
        return;
    };
    let (lines, fill, border, ink): (Vec<String>, Color, Color, Color) =
        if let Some(t) = marks.thread_at(row, col) {
            let mut lines = Vec::new();
            for item in &t.items {
                lines.push(item.author.clone());
                lines.extend(item.text.lines().map(str::to_owned));
            }
            (
                lines,
                Color::WHITE,
                Color::rgb(0xc8, 0xc8, 0xc8),
                Color::rgb(0x24, 0x24, 0x24),
            )
        } else if let Some(n) = marks.note_at(row, col) {
            let mut lines = Vec::new();
            if !n.author.is_empty() && !n.text.starts_with(&n.author) {
                lines.push(format!("{}:", n.author));
            }
            lines.extend(n.text.lines().map(str::to_owned));
            (
                lines,
                Color::rgb(0xff, 0xff, 0xe1),
                Color::BLACK,
                Color::BLACK,
            )
        } else {
            return;
        };
    let px = dp(scale, 11.0);
    let max_w = dp(scale, 160.0);
    let wrapped: Vec<String> = lines.iter().flat_map(|l| wrap(ui, l, px, max_w)).collect();
    let line_h = (px * 1.3).round();
    let pad = dp(scale, 4.0);
    let w = wrapped
        .iter()
        .map(|l| width(ui, l, px))
        .fold(0.0f32, f32::max)
        + pad * 2.0;
    let h = line_h * wrapped.len() as f32 + pad * 2.0;
    let (_, t, r, _) = g.cell(row, col);
    let left = (r + dp(scale, 4.0)).min(g.x1 - w).max(g.x0);
    let top = t.min(g.y1 - h).max(g.y0);
    let box_ = rect(left, top, left + w, top + h);
    list.push_clip(rect(g.x0, g.y0, g.x1, g.y1));
    list.fill_rect(box_, fill);
    list.rect_outline(box_, 1.0, border);
    let metrics = ui.assets.font_metrics(ui.font, px);
    let ascent = metrics.map_or(px, |m| m.ascent);
    for (i, l) in wrapped.iter().enumerate() {
        let base = (top + pad + i as f32 * line_h + (line_h - px) / 2.0 + ascent).round();
        text(ui, list, l, (left + pad).floor(), base, px, ink);
    }
    let _ = pal;
    list.pop_clip();
}

// ---- trace arrows ---------------------------------------------------------------------------

fn dashed_line(
    list: &mut DrawList,
    a: [f32; 2],
    b: [f32; 2],
    w: f32,
    dash: f32,
    gap: f32,
    color: Color,
) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let mut s = 0.0;
    while s < len {
        let e = (s + dash).min(len);
        list.line(
            [a[0] + ux * s, a[1] + uy * s],
            [a[0] + ux * e, a[1] + uy * e],
            w,
            color,
        );
        s += dash + gap;
    }
}

fn arrow_head(
    ui: &mut Ui,
    list: &mut DrawList,
    from: [f32; 2],
    tip: [f32; 2],
    len: f32,
    color: Color,
) {
    let (dx, dy) = (tip[0] - from[0], tip[1] - from[1]);
    let d = (dx * dx + dy * dy).sqrt().max(1.0);
    let (ux, uy) = (dx / d, dy / d);
    let (bx, by) = (tip[0] - ux * len, tip[1] - uy * len);
    let half = len * 0.5;
    let mut p = Path::new();
    p.move_to(tip[0], tip[1])
        .line_to(bx - uy * half, by + ux * half)
        .line_to(bx + uy * half, by - ux * half)
        .close();
    list.fill_path(&mut ui.assets, &p, color);
}

/// Formula auditing: blue arrows with a dot at the source (red when the source is an error), a
/// blue frame round a source range, and a dashed black arrow from a sheet icon for a source on
/// another sheet.
fn paint_arrows(ui: &mut Ui, list: &mut DrawList, g: &Geo, scale: f32, model: &SheetModel) {
    let w = (scale / REF_SCALE).max(1.0);
    for a in &model.ext_formulas.arrows {
        let (tl, tt, tr, tb) = g.cell(a.to.0, a.to.1);
        let end = [tl + (tr - tl) * 0.25, tt + (tb - tt) * 0.5];
        if a.off_sheet {
            let start = [end[0] - dp(scale, 28.0), end[1]];
            let icon = rect(
                start[0] - dp(scale, 7.0),
                start[1] - dp(scale, 8.0),
                start[0] + dp(scale, 7.0),
                start[1] + dp(scale, 8.0),
            );
            list.fill_rect(icon, Color::WHITE);
            list.rect_outline(icon, 1.0, Color::BLACK);
            for k in 1..4 {
                let y = icon.y0 + (icon.y1 - icon.y0) * k as f32 / 4.0;
                list.fill_rect(
                    rect(icon.x0 + 1.0, y, icon.x1 - 1.0, y + 1.0),
                    Color::BLACK.with_opacity(0.6),
                );
            }
            let from = [icon.x1, start[1]];
            dashed_line(
                list,
                from,
                end,
                w,
                dp(scale, 4.0),
                dp(scale, 3.0),
                Color::BLACK,
            );
            arrow_head(ui, list, from, end, dp(scale, 7.0), Color::BLACK);
            continue;
        }
        let source_error = model
            .shown_for_view(a.from.0, a.from.1)
            .is_some_and(|(s, numeric)| !numeric && s.starts_with('#') && s.len() > 2);
        let color = if source_error { TRACE_RED } else { TRACE_BLUE };
        let (sl, st, sr, sb) = g.cell(a.from.0, a.from.1);
        if a.from.0 != a.from.2 || a.from.1 != a.from.3 {
            let (l, t, r, b) = g.range_rect(a.from);
            list.rect_outline(rect(l, t, r, b), w, color);
        }
        let start = [(sl + sr) * 0.5, (st + sb) * 0.5];
        list.line(start, end, w, color);
        let d = dp(scale, 2.5);
        list.fill_rect(
            rect(start[0] - d, start[1] - d, start[0] + d, start[1] + d),
            color,
        );
        arrow_head(ui, list, start, end, dp(scale, 7.0), color);
    }
}

// ---- outline gutter -------------------------------------------------------------------------

/// What a press in the outline gutter hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutlineHit {
    /// A row level button (`1` shows only the top level).
    RowLevel(u32),
    /// A column level button.
    ColLevel(u32),
    /// The `+` or `-` button of a row group (index into `row_groups`).
    RowGroup(usize),
    /// The button of a column group.
    ColGroup(usize),
}

fn gutter_origin(g: &Geo, marks: &GridMarks, scale: f32) -> (f32, f32) {
    let (gw, gh) = gutter(marks, scale);
    (g.x0 - gw, g.y0 - gh)
}

fn row_level_button(g: &Geo, marks: &GridMarks, scale: f32, level: u32) -> Rect {
    let (ox, _) = gutter_origin(g, marks, scale);
    let (p, s) = (dp(scale, LEVEL), dp(scale, LEVEL_BUTTON));
    let x = ox + p * (level - 1) as f32 + (p - s) / 2.0;
    let y = g.y0 + ((g.col_header_h - s) / 2.0).max(0.0);
    rect(x, y, x + s, y + s)
}

fn col_level_button(g: &Geo, marks: &GridMarks, scale: f32, level: u32) -> Rect {
    let (_, oy) = gutter_origin(g, marks, scale);
    let (p, s) = (dp(scale, LEVEL), dp(scale, LEVEL_BUTTON));
    let y = oy + p * (level - 1) as f32 + (p - s) / 2.0;
    let x = g.x0 + ((g.row_header_w - s) / 2.0).max(0.0);
    rect(x, y, x + s, y + s)
}

/// The `+`/`-` button of a row group: centred on the summary row under the detail, in the band
/// of the group's level. `None` when the summary row is off screen.
fn row_group_button(g: &Geo, marks: &GridMarks, scale: f32, grp: &OutlineGroup) -> Option<Rect> {
    let summary = grp.last + 1;
    let (ox, _) = gutter_origin(g, marks, scale);
    let (p, s) = (dp(scale, LEVEL), dp(scale, 9.0));
    let top = g.row_top(summary);
    let bottom = g.row_bottom(summary);
    if bottom < g.y0 + g.col_header_h || top > g.y1 {
        return None;
    }
    let cy = if bottom - top > s * 2.0 {
        top + s
    } else {
        (top + bottom) / 2.0
    };
    let cx = ox + p * (grp.level - 1) as f32 + p / 2.0;
    Some(rect(
        (cx - s / 2.0).round(),
        (cy - s / 2.0).round(),
        (cx + s / 2.0).round(),
        (cy + s / 2.0).round(),
    ))
}

fn col_group_button(g: &Geo, marks: &GridMarks, scale: f32, grp: &OutlineGroup) -> Option<Rect> {
    let summary = grp.last + 1;
    let (_, oy) = gutter_origin(g, marks, scale);
    let (p, s) = (dp(scale, LEVEL), dp(scale, 9.0));
    let left = g.col_left(summary);
    let right = g.col_right(summary);
    if right < g.x0 + g.row_header_w || left > g.x1 {
        return None;
    }
    let cx = if right - left > s * 2.0 {
        left + s
    } else {
        (left + right) / 2.0
    };
    let cy = oy + p * (grp.level - 1) as f32 + p / 2.0;
    Some(rect(
        (cx - s / 2.0).round(),
        (cy - s / 2.0).round(),
        (cx + s / 2.0).round(),
        (cy + s / 2.0).round(),
    ))
}

/// The outline control under a device point.
pub fn outline_hit(g: &Geo, marks: &GridMarks, scale: f32, x: f32, y: f32) -> Option<OutlineHit> {
    for level in (1..=marks.row_levels + 1).filter(|_| marks.row_levels > 0) {
        if inside(row_level_button(g, marks, scale, level), x, y) {
            return Some(OutlineHit::RowLevel(level));
        }
    }
    for level in (1..=marks.col_levels + 1).filter(|_| marks.col_levels > 0) {
        if inside(col_level_button(g, marks, scale, level), x, y) {
            return Some(OutlineHit::ColLevel(level));
        }
    }
    let (gw, gh) = gutter(marks, scale);
    let rows_area = rect(g.x0 - gw, g.y0 + g.col_header_h, g.x0, g.y1);
    if gw > 0.0 && inside(rows_area, x, y) {
        for (i, grp) in marks.row_groups.iter().enumerate() {
            if row_group_button(g, marks, scale, grp).is_some_and(|r| inside(r, x, y)) {
                return Some(OutlineHit::RowGroup(i));
            }
        }
    }
    let cols_area = rect(g.x0 + g.row_header_w, g.y0 - gh, g.x1, g.y0);
    if gh > 0.0 && inside(cols_area, x, y) {
        for (i, grp) in marks.col_groups.iter().enumerate() {
            if col_group_button(g, marks, scale, grp).is_some_and(|r| inside(r, x, y)) {
                return Some(OutlineHit::ColGroup(i));
            }
        }
    }
    None
}

fn paint_level_button(
    ui: &mut Ui,
    list: &mut DrawList,
    pal: &Palette,
    r: Rect,
    label: &str,
    scale: f32,
) {
    list.fill_rect(r, pal.cell_bg);
    list.rect_outline(r, 1.0, pal.grid_header_text);
    let px = dp(scale, 9.0);
    let w = width(ui, label, px);
    let base = centred(ui, r.y0, r.y1, px);
    text(
        ui,
        list,
        label,
        (r.x0 + (r.x1 - r.x0 - w) / 2.0).floor(),
        base,
        px,
        pal.cell_text,
    );
}

fn paint_group_button(list: &mut DrawList, pal: &Palette, r: Rect, collapsed: bool) {
    list.fill_rect(r, pal.cell_bg);
    list.rect_outline(r, 1.0, pal.grid_header_text);
    let (cx, cy) = ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0);
    let arm = ((r.x1 - r.x0) / 2.0 - 2.0).max(1.0);
    list.fill_rect(
        rect(
            (cx - arm).round(),
            cy.floor(),
            (cx + arm).round(),
            cy.floor() + 1.0,
        ),
        pal.cell_text,
    );
    if collapsed {
        list.fill_rect(
            rect(
                cx.floor(),
                (cy - arm).round(),
                cx.floor() + 1.0,
                (cy + arm).round(),
            ),
            pal.cell_text,
        );
    }
}

fn paint_gutter(
    ui: &mut Ui,
    list: &mut DrawList,
    pal: &Palette,
    g: &Geo,
    scale: f32,
    marks: &GridMarks,
) {
    let (gw, gh) = gutter(marks, scale);
    if gw <= 0.0 && gh <= 0.0 {
        return;
    }
    let (ox, oy) = gutter_origin(g, marks, scale);
    let line = pal.grid_header_text;
    let p = dp(scale, LEVEL);
    // The bands behind the brackets.
    list.fill_rect(rect(ox, oy, g.x1, g.y0), pal.cell_bg);
    list.fill_rect(rect(ox, oy, g.x0, g.y1), pal.cell_bg);
    if gw > 0.0 {
        list.push_clip(rect(ox, g.y0 + g.col_header_h, g.x0, g.y1));
        for grp in &marks.row_groups {
            let cx = (ox + p * (grp.level - 1) as f32 + p / 2.0).floor();
            if !grp.collapsed {
                let (top, bottom) = (g.row_top(grp.first), g.row_bottom(grp.last));
                list.fill_rect(rect(cx, top + 1.0, cx + 1.0, bottom), line);
                list.fill_rect(rect(cx, bottom - 1.0, cx + dp(scale, 4.0), bottom), line);
            }
            if let Some(r) = row_group_button(g, marks, scale, grp) {
                paint_group_button(list, pal, r, grp.collapsed);
            }
        }
        list.pop_clip();
        for level in 1..=marks.row_levels + 1 {
            paint_level_button(
                ui,
                list,
                pal,
                row_level_button(g, marks, scale, level),
                &level.to_string(),
                scale,
            );
        }
    }
    if gh > 0.0 {
        list.push_clip(rect(g.x0 + g.row_header_w, oy, g.x1, g.y0));
        for grp in &marks.col_groups {
            let cy = (oy + p * (grp.level - 1) as f32 + p / 2.0).floor();
            if !grp.collapsed {
                let (left, right) = (g.col_left(grp.first), g.col_right(grp.last));
                list.fill_rect(rect(left + 1.0, cy, right, cy + 1.0), line);
                list.fill_rect(rect(right - 1.0, cy, right, cy + dp(scale, 4.0)), line);
            }
            if let Some(r) = col_group_button(g, marks, scale, grp) {
                paint_group_button(list, pal, r, grp.collapsed);
            }
        }
        list.pop_clip();
        for level in 1..=marks.col_levels + 1 {
            paint_level_button(
                ui,
                list,
                pal,
                col_level_button(g, marks, scale, level),
                &level.to_string(),
                scale,
            );
        }
    }
}

// ---- pages ----------------------------------------------------------------------------------

/// How the sheet splits into printed pages.
#[derive(Debug, Clone, PartialEq)]
pub struct Pages {
    /// The print area `(row1, col1, row2, col2)`.
    pub print: (u32, u32, u32, u32),
    /// First column of every page column, left to right.
    pub col_starts: Vec<u32>,
    /// First row of every page row, top to bottom.
    pub row_starts: Vec<u32>,
    /// Which starts come from manual breaks.
    pub manual_cols: BTreeSet<u32>,
    pub manual_rows: BTreeSet<u32>,
    /// Page size and margins in inches (landscape applied).
    pub page_in: (f64, f64),
    pub margins_in: (f64, f64, f64, f64),
    /// Print scale (1.0 is 100 %).
    pub scale: f64,
}

impl Pages {
    /// Page number of the page at page column `ci` and page row `ri` (down, then over).
    pub fn number(&self, ci: usize, ri: usize) -> usize {
        ci * self.row_starts.len() + ri + 1
    }

    /// The cells of page column `ci`: `(first, last)`.
    pub fn cols_of(&self, ci: usize) -> (u32, u32) {
        let first = self.col_starts[ci];
        (
            first,
            self.col_starts.get(ci + 1).map_or(self.print.3, |n| n - 1),
        )
    }

    /// The cells of page row `ri`.
    pub fn rows_of(&self, ri: usize) -> (u32, u32) {
        let first = self.row_starts[ri];
        (
            first,
            self.row_starts.get(ri + 1).map_or(self.print.2, |n| n - 1),
        )
    }
}

fn split(
    first: u32,
    last: u32,
    avail: f64,
    size: impl Fn(u32) -> f64,
    manual: &BTreeSet<u32>,
) -> Vec<u32> {
    let mut starts = vec![first];
    let mut acc = 0.0;
    for i in first..=last.min(first.saturating_add(20_000)) {
        let s = size(i);
        let force = i > *starts.last().unwrap_or(&first) && manual.contains(&i);
        if (acc + s > avail && i > *starts.last().unwrap_or(&first)) || force {
            if starts.len() >= 500 {
                break;
            }
            starts.push(i);
            acc = 0.0;
        }
        acc += s;
    }
    starts
}

/// The page split of the active sheet from its page setup.
pub fn pages(model: &SheetModel) -> Pages {
    let page: SheetPage = model.page();
    let marks = model.grid_marks();
    let geo = model.geometry();
    let print = page
        .print_areas
        .iter()
        .copied()
        .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)))
        .unwrap_or((1, 1, marks.used.0, marks.used.1));
    let (pw, ph) = page.paper.inches();
    let page_in = if page.landscape { (ph, pw) } else { (pw, ph) };
    let m = page.margins;
    let avail_w = (page_in.0 - m.left - m.right).max(0.5);
    let avail_h = (page_in.1 - m.top - m.bottom).max(0.5);
    let print_w: f64 = (print.1..=print.3.min(print.1 + 2000))
        .map(|c| f64::from(geo.col_px(c)) / 96.0)
        .sum();
    let print_h: f64 = (print.0..=print.2.min(print.0 + 20_000))
        .map(|r| f64::from(geo.row_px(r)) / 96.0)
        .sum();
    let scale = model.effective_scale(print_w, print_h, avail_w, avail_h);
    let (aw, ah) = (avail_w * 96.0 / scale, avail_h * 96.0 / scale);
    let col_starts = split(
        print.1,
        print.3,
        aw,
        |c| f64::from(geo.col_px(c)),
        &page.col_breaks,
    );
    let row_starts = split(
        print.0,
        print.2,
        ah,
        |r| f64::from(geo.row_px(r)),
        &page.row_breaks,
    );
    Pages {
        print,
        manual_cols: col_starts
            .iter()
            .copied()
            .filter(|c| page.col_breaks.contains(c))
            .collect(),
        manual_rows: row_starts
            .iter()
            .copied()
            .filter(|r| page.row_breaks.contains(r))
            .collect(),
        col_starts,
        row_starts,
        page_in,
        margins_in: (m.left, m.top, m.right, m.bottom),
        scale,
    }
}

fn paint_page_breaks(
    ui: &mut Ui,
    list: &mut DrawList,
    pal: &Palette,
    g: &Geo,
    scale: f32,
    model: &SheetModel,
) {
    let pg = pages(model);
    let body = body_clip(g);
    list.push_clip(body);
    let (r1, c1, r2, c2) = pg.print;
    let (l, t, r, b) = g.range_rect(pg.print);
    // Outside the print area the sheet is grey.
    for (x0, y0, x1, y1) in [
        (body.x0, body.y0, body.x1, t),
        (body.x0, b, body.x1, body.y1),
        (body.x0, t, l, b),
        (r, t, body.x1, b),
    ] {
        if x1 > x0 && y1 > y0 {
            list.fill_rect(rect(x0, y0, x1, y1), OUTSIDE_GREY.with_opacity(0.55));
        }
    }
    // Watermark page numbers, then the page lines.
    for ci in 0..pg.col_starts.len() {
        for ri in 0..pg.row_starts.len() {
            let ((fc, lc), (fr, lr)) = (pg.cols_of(ci), pg.rows_of(ri));
            let (pl, pt, pr, pb) = g.range_rect((fr, fc, lr, lc));
            let (vl, vt, vr, vb) = (
                pl.max(body.x0),
                pt.max(body.y0),
                pr.min(body.x1),
                pb.min(body.y1),
            );
            if vr <= vl || vb <= vt {
                continue;
            }
            let label = format!("Page {}", pg.number(ci, ri));
            let px = (((pr - pl).min(pb - pt)) / 3.5).clamp(dp(scale, 24.0), dp(scale, 120.0));
            let w = width(ui, &label, px);
            let base = centred(ui, (vt + vb) / 2.0 - px, (vt + vb) / 2.0 + px, px);
            let x = ((vl + vr - w) / 2.0).floor();
            text(
                ui,
                list,
                &label,
                x,
                base,
                px,
                Color::rgb(0x80, 0x80, 0x80).with_opacity(0.35),
            );
        }
    }
    let lw = (scale / REF_SCALE).max(1.0);
    list.rect_outline(rect(l, t, r, b), lw * 2.0, BREAK_BLUE);
    for &c in pg.col_starts.iter().skip(1) {
        let x = g.col_left(c);
        let manual = pg.manual_cols.contains(&c);
        let mut y = t;
        while y < b {
            let e = if manual {
                b
            } else {
                (y + dp(scale, 4.0)).min(b)
            };
            list.fill_rect(rect(x - 1.0, y, x + 1.0, e), BREAK_BLUE);
            y += if manual { b - t + 1.0 } else { dp(scale, 8.0) };
        }
    }
    for &rw in pg.row_starts.iter().skip(1) {
        let y = g.row_top(rw);
        let manual = pg.manual_rows.contains(&rw);
        let mut x = l;
        while x < r {
            let e = if manual {
                r
            } else {
                (x + dp(scale, 4.0)).min(r)
            };
            list.fill_rect(rect(x, y - 1.0, e, y + 1.0), BREAK_BLUE);
            x += if manual { r - l + 1.0 } else { dp(scale, 8.0) };
        }
    }
    let _ = (pal, r1, c1, r2, c2);
    list.pop_clip();
}

// ---- Page Layout view -----------------------------------------------------------------------

/// One page of the Page Layout view on screen.
#[derive(Debug, Clone, Copy)]
pub struct Tile {
    /// The page, margins included.
    pub page: Rect,
    /// Where the first cell starts.
    pub content: Rect,
    pub rows: (u32, u32),
    pub cols: (u32, u32),
    pub number: usize,
    /// Device pixels per logical (96 dpi) cell pixel.
    pub k: f32,
}

/// The page tiles in `area`, starting at the page that holds the first visible cell.
pub fn layout_tiles(model: &SheetModel, area: PxRect, scale: f32) -> Vec<Tile> {
    let pg = pages(model);
    let (pw, ph) = (
        pg.page_in.0 as f32 * 96.0 * scale,
        pg.page_in.1 as f32 * 96.0 * scale,
    );
    let gap = dp(scale, 10.0);
    let (ox, oy) = (area.x as f32 + gap, area.y as f32 + gap);
    let (top, left) = model.top_left;
    let ri0 = pg.row_starts.iter().rposition(|&r| r <= top).unwrap_or(0);
    let ci0 = pg.col_starts.iter().rposition(|&c| c <= left).unwrap_or(0);
    let k = scale * pg.scale as f32;
    let mut out = Vec::new();
    for ci in ci0..pg.col_starts.len() {
        let x = ox + (ci - ci0) as f32 * (pw + gap);
        if x >= area.right() as f32 {
            break;
        }
        for ri in ri0..pg.row_starts.len() {
            let y = oy + (ri - ri0) as f32 * (ph + gap);
            if y >= area.bottom() as f32 {
                break;
            }
            let (ml, mt) = (
                pg.margins_in.0 as f32 * 96.0 * scale,
                pg.margins_in.1 as f32 * 96.0 * scale,
            );
            let (mr, mb) = (
                pg.margins_in.2 as f32 * 96.0 * scale,
                pg.margins_in.3 as f32 * 96.0 * scale,
            );
            out.push(Tile {
                page: rect(x, y, x + pw, y + ph),
                content: rect(x + ml, y + mt, x + pw - mr, y + ph - mb),
                rows: pg.rows_of(ri),
                cols: pg.cols_of(ci),
                number: pg.number(ci, ri),
                k,
            });
        }
    }
    out
}

impl Tile {
    /// Device rectangle of a cell inside this tile, `None` for a cell on another page.
    pub fn cell(&self, geo: &crate::sheet::GridGeometry, row: u32, col: u32) -> Option<Rect> {
        if row < self.rows.0 || row > self.rows.1 || col < self.cols.0 || col > self.cols.1 {
            return None;
        }
        let x = self.content.x0 + (geo.col_x(col) - geo.col_x(self.cols.0)) as f32 * self.k;
        let y = self.content.y0 + (geo.row_y(row) - geo.row_y(self.rows.0)) as f32 * self.k;
        Some(rect(
            x,
            y,
            x + geo.col_px(col) as f32 * self.k,
            y + geo.row_px(row) as f32 * self.k,
        ))
    }
}

/// The cell under a device point in the Page Layout view.
pub fn layout_cell_at(
    model: &SheetModel,
    area: PxRect,
    scale: f32,
    x: f32,
    y: f32,
) -> Option<(u32, u32)> {
    let geo = model.geometry();
    for tile in layout_tiles(model, area, scale) {
        if !inside(tile.content, x, y) {
            continue;
        }
        let px = ((x - tile.content.x0) / tile.k) as u64 + geo.col_x(tile.cols.0);
        let py = ((y - tile.content.y0) / tile.k) as u64 + geo.row_y(tile.rows.0);
        let col = geo.col_at(px, tile.cols.1).clamp(tile.cols.0, tile.cols.1);
        let row = geo.row_at(py, tile.rows.1).clamp(tile.rows.0, tile.rows.1);
        return Some((row, col));
    }
    None
}

/// Draws one cell's text in `r` (right-aligned numbers, left-aligned text), clipped to the cell.
/// Size, padding and clip of the text [`cell_text`] draws.
#[derive(Debug, Clone, Copy)]
pub struct TextStyle {
    pub px: f32,
    pub pad: f32,
    pub clip: Rect,
}

pub fn cell_text(
    ui: &mut Ui,
    list: &mut DrawList,
    pal: &Palette,
    model: &SheetModel,
    (row, col): (u32, u32),
    r: Rect,
    style: TextStyle,
) {
    let TextStyle { px, pad, clip } = style;
    let Some((value, numeric)) = model.shown_for_view(row, col) else {
        return;
    };
    let base = centred(ui, r.y0, r.y1, px);
    let w = width(ui, &value, px);
    let tx = if numeric { r.x1 - pad - w } else { r.x0 + pad };
    let c = clip.intersect(&rect(r.x0, r.y0, r.x1 - 1.0, r.y1 - 1.0));
    if c.is_empty() {
        return;
    }
    list.push_clip(c);
    text(ui, list, &value, tx.floor(), base, px, pal.cell_text);
    list.pop_clip();
}

fn paint_page_layout(
    ui: &mut Ui,
    list: &mut DrawList,
    pal: &Palette,
    area: PxRect,
    scale: f32,
    model: &SheetModel,
) {
    list.push_clip(gr(area));
    list.fill_rect(gr(area), pal.canvas);
    let geo = model.geometry();
    let gridlines = model.ext_view.gridlines(model.sheet);
    for tile in layout_tiles(model, area, scale) {
        list.fill_rect(tile.page, Color::WHITE);
        list.rect_outline(tile.page, 1.0, pal.grid_header_border);
        // The margins: header and footer boxes and a dotted guide round the content.
        let hint = dp(scale, 10.0);
        for (label, y) in [
            ("Click to add header", tile.page.y0 + dp(scale, 12.0)),
            ("Click to add footer", tile.page.y1 - dp(scale, 12.0) - hint),
        ] {
            let w = width(ui, label, hint);
            let base = centred(ui, y, y + hint, hint);
            let x = ((tile.page.x0 + tile.page.x1 - w) / 2.0).floor();
            text(ui, list, label, x, base, hint, Color::rgb(0x8c, 0x8c, 0x8c));
        }
        let guide = Color::rgb(0xc8, 0xc8, 0xc8);
        list.rect_outline(tile.content.inflate(1.0), 1.0, guide);
        list.push_clip(tile.content.intersect(&gr(area)));
        let k = tile.k;
        let px = 12.0 * k;
        for row in tile.rows.0..=tile.rows.1 {
            for col in tile.cols.0..=tile.cols.1 {
                let Some(r) = tile.cell(&geo, row, col) else {
                    continue;
                };
                if r.y0 > f32::from(u16::MAX) || r.y1 < tile.content.y0 || r.x1 < tile.content.x0 {
                    continue;
                }
                if r.y0 > tile.content.y1 || r.x0 > tile.content.x1 {
                    continue;
                }
                if gridlines {
                    list.fill_rect(rect(r.x1 - 1.0, r.y0, r.x1, r.y1), pal.grid_line);
                    list.fill_rect(rect(r.x0, r.y1 - 1.0, r.x1, r.y1), pal.grid_line);
                }
                cell_text(
                    ui,
                    list,
                    pal,
                    model,
                    (row, col),
                    r,
                    TextStyle {
                        px,
                        pad: 4.0 * k,
                        clip: tile.content,
                    },
                );
            }
        }
        if let Some(a) = tile.cell(&geo, model.active.0, model.active.1) {
            list.rect_outline(
                rect(a.x0 - 1.0, a.y0 - 1.0, a.x1, a.y1),
                2.0,
                pal.grid_accent,
            );
        }
        list.pop_clip();
    }
    list.pop_clip();
}

// ---- entry point ----------------------------------------------------------------------------

/// What to draw over the cells, in Excel's order: Page Break Preview below the selection, then
/// the markers, buttons and arrows. Called with the body clip set up by the caller.
pub(super) fn paint_under(
    ui: &mut Ui,
    list: &mut DrawList,
    pal: &Palette,
    g: &Geo,
    scale: f32,
    model: &SheetModel,
) {
    if model.ext_view.mode == ViewMode::PageBreakPreview {
        paint_page_breaks(ui, list, pal, g, scale, model);
    }
}

pub(super) fn paint_over(
    ui: &mut Ui,
    list: &mut DrawList,
    pal: &Palette,
    g: &Geo,
    scale: f32,
    model: &SheetModel,
) {
    let marks = model.grid_marks();
    list.push_clip(body_clip(g));
    paint_markers(ui, list, g, scale, &marks);
    paint_filter_buttons(ui, list, pal, g, scale, &marks);
    paint_validation_arrow(ui, list, pal, g, scale, model);
    paint_arrows(ui, list, g, scale, model);
    list.pop_clip();
}

pub(super) fn paint_gutter_and_hover(
    ui: &mut Ui,
    list: &mut DrawList,
    pal: &Palette,
    g: &Geo,
    scale: f32,
    model: &SheetModel,
) {
    let marks = model.grid_marks();
    paint_gutter(ui, list, pal, g, scale, &marks);
    paint_hover(ui, list, pal, g, scale, &marks);
}

/// Paints the Page Layout view in place of the grid; `false` when the view is another mode.
pub(super) fn paint_layout_view(
    ui: &mut Ui,
    list: &mut DrawList,
    pal: &Palette,
    area: PxRect,
    scale: f32,
    model: &SheetModel,
) -> bool {
    if model.ext_view.mode != ViewMode::PageLayout {
        return false;
    }
    paint_page_layout(ui, list, pal, area, scale, model);
    true
}
