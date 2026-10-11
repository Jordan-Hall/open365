//! Conditional formatting visuals the grid draws in Excel's style: data bars (gradient fill with
//! a solid edge) and icon sets (arrows, traffic lights, flags, stars, rating bars, quarters,
//! boxes), as vector shapes sized to the row. Pure drawing: the cell rectangle and the evaluated
//! effect come from the caller.

use std::f32::consts::{PI, TAU};

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, vec2};
use open365_commands::DataBarEffect;

/// Width in device pixels the icon takes at the left edge of a cell, margins included.
pub(crate) const ICON_SLOT_PX: f32 = 18.0;
/// Edge of the icon square in device pixels.
const ICON_PX: f32 = 13.0;

const RED: Color32 = Color32::from_rgb(0xE3, 0x4A, 0x3E);
const ORANGE: Color32 = Color32::from_rgb(0xF2, 0x8E, 0x2B);
const YELLOW: Color32 = Color32::from_rgb(0xF7, 0xC5, 0x2D);
const LIME: Color32 = Color32::from_rgb(0xA5, 0xC8, 0x3F);
const GREEN: Color32 = Color32::from_rgb(0x4D, 0xA8, 0x55);
const BLACK: Color32 = Color32::from_rgb(0x33, 0x33, 0x33);
const GRAY: Color32 = Color32::from_rgb(0x8C, 0x8C, 0x8C);
const BLUE: Color32 = Color32::from_rgb(0x4F, 0x81, 0xBD);

/// Draws the data bar of a cell: a gradient from the bar colour to near white, a solid edge in
/// the bar colour, inset from the cell like Excel's.
pub(crate) fn paint_data_bar(painter: &Painter, cell: Rect, bar: &DataBarEffect, ppp: f32) {
    let px = |v: f32| v / ppp;
    let span = (cell.width() - px(4.0)).max(0.0);
    let pct = |p: u32| (p.min(100) as f32) / 100.0;
    let (lo, hi) = (pct(bar.min_length), pct(bar.max_length));
    let fraction = bar.fraction.clamp(0.0, 1.0) as f32;
    let length = span * (lo + (hi - lo) * fraction);
    if length < px(1.0) || cell.height() < px(5.0) {
        return;
    }
    let rect = Rect::from_min_size(
        Pos2::new(cell.left() + px(2.0), cell.top() + px(2.0)),
        vec2(length, cell.height() - px(4.0)),
    );
    let c = Color32::from_rgb(bar.color.r, bar.color.g, bar.color.b);
    let light = mix(c, Color32::WHITE, 0.85);
    let mut mesh = Mesh::default();
    let v = |pos: Pos2, color: Color32| Vertex {
        pos,
        uv: WHITE_UV,
        color,
    };
    mesh.vertices = vec![
        v(rect.left_top(), c),
        v(rect.right_top(), light),
        v(rect.right_bottom(), light),
        v(rect.left_bottom(), c),
    ];
    mesh.indices = vec![0, 1, 2, 0, 2, 3];
    painter.add(Shape::mesh(mesh));
    painter.rect_stroke(rect, 0.0, Stroke::new(px(1.0), c), StrokeKind::Inside);
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// The colour of icon `index` of `count` in `set`.
pub(crate) fn icon_color(set: &str, index: usize, count: usize) -> Color32 {
    let rated = |gray: bool| -> Color32 { if gray { GRAY } else { BLUE } };
    if set.contains("Gray") {
        return mix(
            Color32::from_gray(0xB0),
            BLACK,
            index as f32 / count.max(2) as f32,
        );
    }
    if set.contains("RedToBlack") {
        return [BLACK, GRAY, Color32::from_rgb(0xE8, 0x9A, 0x9A), RED][index.min(3)];
    }
    if set.contains("Rating")
        || set.contains("Quarters")
        || set.contains("Boxes")
        || set.contains("Stars")
    {
        return if set.contains("Stars") {
            YELLOW
        } else {
            rated(set.contains("Quarters"))
        };
    }
    if set.contains("TrafficLights") && count == 4 {
        return [BLACK, RED, YELLOW, GREEN][index.min(3)];
    }
    let palette: &[Color32] = match count {
        3 => &[RED, YELLOW, GREEN],
        4 => &[RED, YELLOW, LIME, GREEN],
        _ => &[RED, ORANGE, YELLOW, LIME, GREEN],
    };
    palette[index.min(palette.len() - 1)]
}

/// Draws a filled triangle fan (convex or star-shaped) around `centre`.
fn fan(painter: &Painter, centre: Pos2, ring: &[Pos2], color: Color32, closed: bool) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(centre, color);
    for p in ring {
        mesh.colored_vertex(*p, color);
    }
    let last = if closed {
        ring.len()
    } else {
        ring.len().saturating_sub(1)
    };
    for i in 0..last {
        let next = (i + 1) % ring.len();
        mesh.add_triangle(0, i as u32 + 1, next as u32 + 1);
    }
    painter.add(Shape::mesh(mesh));
}

/// Points of an arc around `c`, from `from` to `to` radians (0 = up, clockwise).
fn arc(c: Pos2, r: f32, from: f32, to: f32) -> Vec<Pos2> {
    let steps = (((to - from).abs() / TAU) * 32.0).ceil().max(2.0) as usize;
    (0..=steps)
        .map(|i| {
            let a = from + (to - from) * i as f32 / steps as f32;
            c + vec2(a.sin(), -a.cos()) * r
        })
        .collect()
}

/// Draws icon `index` (0 = lowest band) of `count` in `set` at the left edge of `cell`.
pub(crate) fn paint_icon(
    painter: &Painter,
    cell: Rect,
    set: &str,
    index: usize,
    count: usize,
    ppp: f32,
) {
    let px = |v: f32| v / ppp;
    let size = px(ICON_PX).min(cell.height() - px(2.0)).max(px(4.0));
    let c = Pos2::new(cell.left() + px(2.0) + size / 2.0, cell.center().y);
    let r = size / 2.0;
    let color = icon_color(set, index, count);
    let top = count.saturating_sub(1).max(1);
    let level = index as f32 / top as f32;
    let outline = Stroke::new(px(1.0), mix(color, BLACK, 0.35));
    if set.contains("Arrows") || set.contains("Triangles") {
        // Direction: 0 = down .. 1 = up, in steps of 45 degrees where the set has them.
        let angle = if set.contains("Triangles") {
            [PI, 0.0, 0.0][index.min(2)]
        } else {
            // Rotation from "up" clockwise: up 0, up-right 45, right 90, down-right 135, down 180.
            (180.0 - level * 180.0).to_radians()
        };
        let flat = set.contains("Triangles") && index == 1;
        if flat {
            let bar = Rect::from_center_size(c, vec2(size, size * 0.3));
            painter.rect_filled(bar, 0.0, color);
            return;
        }
        let rot = |p: Pos2| {
            let d = p - c;
            c + vec2(
                d.x * angle.cos() - d.y * angle.sin(),
                d.x * angle.sin() + d.y * angle.cos(),
            )
        };
        let pts: Vec<Pos2> = if set.contains("Triangles") {
            vec![
                c + vec2(0.0, -r),
                c + vec2(r, r * 0.8),
                c + vec2(-r, r * 0.8),
            ]
        } else {
            vec![
                c + vec2(0.0, -r),
                c + vec2(r, 0.0),
                c + vec2(r * 0.4, 0.0),
                c + vec2(r * 0.4, r),
                c + vec2(-r * 0.4, r),
                c + vec2(-r * 0.4, 0.0),
                c + vec2(-r, 0.0),
            ]
        };
        let rotated: Vec<Pos2> = pts.into_iter().map(rot).collect();
        // Arrow heads and shafts are convex pieces; the fan from the centre covers them.
        fan(painter, c, &rotated, color, true);
        return;
    }
    if set.contains("Stars") {
        let ring: Vec<Pos2> = (0..10)
            .map(|i| {
                let a = i as f32 * PI / 5.0;
                let rr = if i % 2 == 0 { r } else { r * 0.45 };
                c + vec2(a.sin(), -a.cos()) * rr
            })
            .collect();
        let full = index + 1 >= count || (index * 2 + 1 >= count && count > 1);
        painter.add(Shape::closed_line(ring.clone(), Stroke::new(px(1.0), GRAY)));
        if full {
            fan(painter, c, &ring, color, true);
        } else if index > 0 {
            let half: Vec<Pos2> = ring.iter().copied().filter(|p| p.x <= c.x + 0.01).collect();
            if half.len() > 2 {
                fan(painter, c, &half, color, true);
            }
        }
        return;
    }
    if set.contains("Quarters") {
        painter.circle_stroke(c, r, Stroke::new(px(1.0), BLACK));
        // The lowest band is an empty circle, the highest a full one.
        let quarters = index.min(4);
        if quarters > 0 {
            fan(
                painter,
                c,
                &arc(c, r, 0.0, quarters as f32 * TAU / 4.0),
                BLACK,
                false,
            );
        }
        return;
    }
    if set.contains("Rating") {
        // Four or five ascending bars; as many as the band, the rest grey.
        let bars = count.max(2);
        let bw = size / (bars as f32 * 1.6);
        for b in 0..bars {
            let h = size * (b + 1) as f32 / bars as f32;
            let x = c.x - r + b as f32 * bw * 1.6;
            let rect = Rect::from_min_max(Pos2::new(x, c.y + r - h), Pos2::new(x + bw, c.y + r));
            painter.rect_filled(
                rect,
                0.0,
                if b <= index {
                    color
                } else {
                    Color32::from_gray(0xC8)
                },
            );
        }
        return;
    }
    if set.contains("Boxes") {
        let side = size * 0.8;
        let rect = Rect::from_center_size(c, vec2(side, side));
        painter.rect_stroke(rect, 0.0, Stroke::new(px(1.0), BLACK), StrokeKind::Inside);
        let h = side * index as f32 / top as f32;
        if h > 0.0 {
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(rect.left(), rect.bottom() - h),
                    rect.right_bottom(),
                ),
                0.0,
                BLACK,
            );
        }
        return;
    }
    if set.contains("Flags") {
        let pole = Stroke::new(px(1.0), BLACK);
        painter.line_segment([c + vec2(-r * 0.7, -r), c + vec2(-r * 0.7, r)], pole);
        let flag = [
            c + vec2(-r * 0.7, -r),
            c + vec2(r, -r * 0.5),
            c + vec2(-r * 0.7, 0.0),
        ];
        fan(painter, c + vec2(0.0, -r * 0.5), &flag, color, true);
        return;
    }
    if set.contains("Signs") {
        // Red diamond, yellow triangle, green circle.
        match index {
            0 => fan(
                painter,
                c,
                &[
                    c + vec2(0.0, -r),
                    c + vec2(r, 0.0),
                    c + vec2(0.0, r),
                    c + vec2(-r, 0.0),
                ],
                color,
                true,
            ),
            1 => fan(
                painter,
                c,
                &[
                    c + vec2(0.0, -r),
                    c + vec2(r, r * 0.8),
                    c + vec2(-r, r * 0.8),
                ],
                color,
                true,
            ),
            _ => {
                painter.circle_filled(c, r, color);
            }
        }
        return;
    }
    // Traffic lights, symbols and the rest: a disc, with a mark for the symbol sets.
    painter.circle_filled(c, r, color);
    if set.contains("TrafficLights1") || set.contains("TrafficLights2") {
        painter.circle_stroke(c, r, outline);
    }
    if set.contains("Symbols") {
        let white = Stroke::new(px(1.5), Color32::WHITE);
        match (index, count) {
            (i, n) if i + 1 == n => {
                painter.line_segment(
                    [c + vec2(-r * 0.5, 0.0), c + vec2(-r * 0.1, r * 0.4)],
                    white,
                );
                painter.line_segment(
                    [c + vec2(-r * 0.1, r * 0.4), c + vec2(r * 0.5, -r * 0.4)],
                    white,
                );
            }
            (0, _) => {
                painter.line_segment(
                    [c + vec2(-r * 0.4, -r * 0.4), c + vec2(r * 0.4, r * 0.4)],
                    white,
                );
                painter.line_segment(
                    [c + vec2(r * 0.4, -r * 0.4), c + vec2(-r * 0.4, r * 0.4)],
                    white,
                );
            }
            _ => {
                painter.line_segment([c + vec2(0.0, -r * 0.5), c + vec2(0.0, r * 0.1)], white);
                painter.circle_filled(c + vec2(0.0, r * 0.45), px(1.0), Color32::WHITE);
            }
        }
    }
}
