//! Table cell shading, cell borders and paragraph border rules, painted from the layout's
//! `Page::cells` and `Page::rules` (the same geometry the egui Writer and the PDF export use).

use open365_gfx::{Color, DrawList, Rect as GRect};
use open365_layout::{BorderLine, BorderStyle};

use super::WriterPane;

fn rgb(c: [u8; 3]) -> Color {
    Color::rgb(c[0], c[1], c[2])
}

/// A dashed or dotted run along a strip of `thick` pixels, `on` pixels on and off.
fn dashes(list: &mut DrawList, r: GRect, horizontal: bool, on: f32, color: Color) {
    let on = on.max(1.0);
    let (start, end) = if horizontal {
        (r.x0, r.x1)
    } else {
        (r.y0, r.y1)
    };
    let mut at = start;
    while at < end {
        let to = (at + on).min(end);
        let piece = if horizontal {
            GRect::from_edges(at, r.y0, to, r.y1)
        } else {
            GRect::from_edges(r.x0, at, r.x1, to)
        };
        list.fill_rect(piece, color);
        at += on * 2.0;
    }
}

impl WriterPane {
    /// Rules, cell fills and cell borders of `page`. `origin` is the page's top left in device
    /// pixels; `view` is the visible band (top, bottom) used to skip what cannot be seen.
    pub(super) fn paint_cells(
        &mut self,
        list: &mut DrawList,
        page: usize,
        origin: (f32, f32),
        view: (f32, f32),
    ) {
        let s = self.s();
        let rect = |x: f64, y: f64, w: f64, h: f64| {
            let (x0, y0) = (origin.0 + x as f32 * s, origin.1 + y as f32 * s);
            GRect::from_edges(
                x0,
                y0,
                x0 + (w as f32 * s).max(1.0),
                y0 + (h as f32 * s).max(1.0),
            )
        };
        let visible = |r: GRect| r.y1 + 8.0 * s >= view.0 && r.y0 - 8.0 * s <= view.1;
        let (mut rules, mut fills, mut strips) = (0usize, 0usize, 0usize);
        let layout_page = &self.engine.layout().pages[page];
        for r in &layout_page.rules {
            let g = rect(r.x, r.y, r.width, r.height);
            if visible(g) {
                list.fill_rect(g, rgb(r.color));
                rules += 1;
            }
        }
        for cell in &layout_page.cells {
            let g = rect(cell.x, cell.y, cell.width, cell.height);
            if let (Some(fill), true) = (cell.fill, visible(g)) {
                list.fill_rect(g, rgb(fill));
                fills += 1;
            }
        }
        for cell in &layout_page.cells {
            let g = rect(cell.x, cell.y, cell.width, cell.height);
            if !visible(g) {
                continue;
            }
            let b = &cell.borders;
            let width = |l: &Option<BorderLine>| l.as_ref().map_or(0.0, |l| l.width);
            let (wl, wr, wt, wb) = (
                width(&b.left),
                width(&b.right),
                width(&b.top),
                width(&b.bottom),
            );
            let (x, y, w, h) = (cell.x, cell.y, cell.width, cell.height);
            // Strip rectangles in page points, corners included (as in the PDF export).
            let sides = [
                (&b.top, true, (x - wl, y - wt, w + wl + wr, wt)),
                (&b.bottom, true, (x - wl, y + h, w + wl + wr, wb)),
                (&b.left, false, (x - wl, y - wt, wl, h + wt + wb)),
                (&b.right, false, (x + w, y - wt, wr, h + wt + wb)),
            ];
            for (line, horizontal, (sx, sy, sw, sh)) in sides {
                let Some(line) = line else { continue };
                let c = rgb(line.color);
                let strip = rect(sx, sy, sw, sh);
                match line.style {
                    BorderStyle::Solid => list.fill_rect(strip, c),
                    BorderStyle::Double => {
                        let (a, z) = if horizontal {
                            let third = (strip.y1 - strip.y0) / 3.0;
                            (
                                GRect::from_edges(strip.x0, strip.y0, strip.x1, strip.y0 + third),
                                GRect::from_edges(strip.x0, strip.y1 - third, strip.x1, strip.y1),
                            )
                        } else {
                            let third = (strip.x1 - strip.x0) / 3.0;
                            (
                                GRect::from_edges(strip.x0, strip.y0, strip.x0 + third, strip.y1),
                                GRect::from_edges(strip.x1 - third, strip.y0, strip.x1, strip.y1),
                            )
                        };
                        list.fill_rect(a, c);
                        list.fill_rect(z, c);
                    }
                    BorderStyle::Dashed | BorderStyle::Dotted => {
                        let thick = (line.width as f32 * s).max(1.0);
                        let on = if line.style == BorderStyle::Dotted {
                            thick
                        } else {
                            thick * 3.0
                        };
                        dashes(list, strip, horizontal, on, c);
                    }
                }
                strips += 1;
            }
        }
        self.stats.rules += rules;
        self.stats.cell_fills += fills;
        self.stats.border_strips += strips;
    }
}
