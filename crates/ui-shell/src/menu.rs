//! Drop-down menus, colour pickers, tile galleries and combo lists, drawn the way Office draws them.
//!
//! Geometry is data measured from the reference application (`fixtures/reference/ui/excel-popups.json`, 125 %,
//! physical pixels = `u`): a 1 u frame plus 1 u padding, rows 36 u high, group headings 37 / 38 u, separators 3 u, colour cells
//! 30 u on a 34 u pitch (tints 30 u), gallery tiles with a 37 u heading and 10 u under each group, a 21 u scroll bar
//! when the list does not fit. [`plan`] turns entries into rectangles (pure, tested against the fixture);
//! [`show`] draws a menu with its open sub-menus and handles the pointer and the keyboard.

use egui::{
    Align2, Area, Color32, CornerRadius, FontFamily, FontId, Id, Key, Order, Painter, Pos2, Rect,
    Sense, Stroke, StrokeKind, pos2, vec2,
};

use crate::host::{EntryKind, MenuEntry};
use crate::icons::Icons;
use crate::spec::REF_SCALE;
use crate::theme::{Theme, size};

/// Frame plus padding on every side.
pub const FRAME: f32 = 2.0;
pub const ITEM_H: f32 = 36.0;
pub const HEADING_H: f32 = 37.0;
/// The "Gallery Filters" strip above Excel's colour pickers (measured: group 340 x 52, switch button
/// x 142, y 9, 193 x 35 in a 340 u popup).
pub const FILTER_H: f32 = 52.0;
const FILTER_SWITCH: [f32; 4] = [142.0, 9.0, 193.0, 35.0];
/// WCAG contrast a colour needs against the menu background to stay while the filter is on.
const HIGH_CONTRAST: f32 = 3.0;
pub const SEPARATOR_H: f32 = 3.0;
/// Colour cell and its pitch across a row.
pub const CELL: f32 = 30.0;
pub const CELL_PITCH: f32 = 34.0;
pub const SCROLLBAR_W: f32 = 21.0;
/// Space left of the label for the icon or check mark, and right of it for the sub-menu arrow.
const GUTTER: f32 = 44.0;
const RIGHT_PAD: f32 = 37.0;

/// Where a row and its cells sit, in `u` from the popup's top-left corner (before scrolling).
#[derive(Debug, Clone, PartialEq)]
pub struct RowPlan {
    pub index: usize,
    pub rect: [f32; 4],
    /// Swatch or tile cells: `[x, y, w, h]`.
    pub cells: Vec<[f32; 4]>,
}

/// A laid-out popup.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub width: f32,
    pub height: f32,
    pub content_height: f32,
    pub scrolls: bool,
    pub rows: Vec<RowPlan>,
}

/// Looks of the combo lists the reference measured (row height, minimum width, rows shown at most).
#[derive(Debug, Clone, Copy)]
pub struct ListLook {
    pub min_w: f32,
    pub max_rows: Option<u32>,
}

/// List metrics by control id (font name, size and number format lists).
#[must_use]
pub fn list_look(id: &str) -> Option<ListLook> {
    match id {
        "home.font.font" => Some(ListLook {
            min_w: 376.0,
            max_rows: None,
        }),
        "home.font.font-size" => Some(ListLook {
            min_w: 67.0,
            max_rows: Some(16),
        }),
        "home.number.number-format" => Some(ListLook {
            min_w: 237.0,
            max_rows: None,
        }),
        _ => None,
    }
}

/// Lays `entries` out. `text_w` measures a label in `u`; `max_h` is the tallest popup that fits (u).
#[must_use]
pub fn plan(entries: &[MenuEntry], min_w: f32, max_h: f32, text_w: &dyn Fn(&str) -> f32) -> Plan {
    let mut rows = Vec::with_capacity(entries.len());
    let mut y = FRAME;
    let mut width = 0.0_f32;
    let mut prev: Option<&EntryKind> = None;
    let mut prev_was_heading = false;
    for (index, e) in entries.iter().enumerate() {
        let (h, cells) = match &e.kind {
            EntryKind::Separator => (SEPARATOR_H, Vec::new()),
            EntryKind::Heading => {
                let h = match prev {
                    None | Some(EntryKind::Separator) => HEADING_H,
                    Some(_) => HEADING_H + 1.0,
                };
                width = width.max(FRAME * 2.0 + text_w(&e.label) + 20.0);
                (h, Vec::new())
            }
            EntryKind::Item => {
                let h = e
                    .height
                    .map_or(if e.detail.is_some() { 60.0 } else { ITEM_H }, |h| h as f32);
                let extra = if e.height.is_some() {
                    27.0
                } else {
                    GUTTER + RIGHT_PAD
                };
                let sc = e.shortcut.as_deref().map_or(0.0, |s| text_w(s) + 24.0);
                width = width.max(FRAME * 2.0 + extra + text_w(&e.label) + sc);
                (h, Vec::new())
            }
            EntryKind::Filter => {
                width = width.max(FRAME * 2.0 + FILTER_SWITCH[0] + FILTER_SWITCH[2]);
                (FILTER_H, Vec::new())
            }
            EntryKind::Swatches { rows: r, tight } => {
                let pad_top = if prev_was_heading { 3.0 } else { 1.0 };
                let cols = r.iter().map(Vec::len).max().unwrap_or(0) as f32;
                width = width.max(FRAME * 2.0 + CELL_PITCH * cols);
                let mut cells = Vec::new();
                for (ri, row) in r.iter().enumerate() {
                    for ci in 0..row.len() {
                        cells.push([
                            4.0 + CELL_PITCH * ci as f32,
                            y + pad_top + CELL * ri as f32,
                            CELL,
                            CELL,
                        ]);
                    }
                }
                let tail = if *tight { 14.0 } else { 12.0 };
                (pad_top + CELL * r.len() as f32 + tail, cells)
            }
            EntryKind::Tiles {
                columns,
                size: (tw, th),
                tiles,
            } => {
                let cols = (*columns).max(1);
                let (tw, th) = (*tw as f32, *th as f32);
                width = width.max(FRAME * 2.0 + tw * cols as f32);
                let mut cells = Vec::new();
                for k in 0..tiles.len() {
                    cells.push([
                        FRAME + tw * (k % cols) as f32,
                        y + th * (k / cols) as f32,
                        tw,
                        th,
                    ]);
                }
                (th * tiles.len().div_ceil(cols) as f32 + 10.0, cells)
            }
        };
        rows.push(RowPlan {
            index,
            rect: [FRAME, y, 0.0, h],
            cells,
        });
        y += h;
        prev_was_heading = e.kind == EntryKind::Heading;
        prev = Some(&e.kind);
    }
    let content_height = y + FRAME;
    let scrolls = content_height > max_h;
    let height = if scrolls { max_h } else { content_height };
    if scrolls {
        width += SCROLLBAR_W;
    }
    width = width.max(min_w);
    let inner = width - 2.0 * FRAME - if scrolls { SCROLLBAR_W } else { 0.0 };
    for r in &mut rows {
        r.rect[2] = inner;
    }
    Plan {
        width,
        height,
        content_height,
        scrolls,
        rows,
    }
}

/// What the user did with a menu this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEvent {
    None,
    /// An entry was chosen (its id); the menu closes.
    Run(String),
    /// Escape or a click outside.
    Close,
}

/// An open menu: the entry path to its sub-menus, the hot rows and the scroll offsets (one per level).
#[derive(Debug, Clone)]
pub struct MenuState {
    pub id: String,
    /// Top-left of the first popup in points.
    pub pos: Pos2,
    /// Width of the control that opened it (the list is at least this wide), in `u`.
    pub opener_w: f32,
    pub fresh: bool,
    /// Entry index of the open sub-menu at each level.
    pub path: Vec<usize>,
    pub hot: Vec<Option<usize>>,
    pub scroll: Vec<f32>,
    /// Combo list: the hot row follows the keyboard and the list scrolls to the checked entry once.
    pub list: bool,
    pub reveal: bool,
}

impl MenuState {
    #[must_use]
    pub fn new(id: &str, pos: Pos2, opener_w: f32, list: bool) -> Self {
        Self {
            id: id.to_owned(),
            pos,
            opener_w,
            fresh: true,
            path: Vec::new(),
            hot: vec![None],
            scroll: vec![0.0],
            list,
            reveal: list,
        }
    }
}

fn entries_at<'a>(root: &'a [MenuEntry], path: &[usize], depth: usize) -> &'a [MenuEntry] {
    let mut cur = root;
    for i in &path[..depth] {
        cur = &cur[*i].children;
    }
    cur
}

fn interactive(e: &MenuEntry) -> bool {
    e.kind == EntryKind::Item && e.enabled
}

/// WCAG contrast ratio of a colour against the menu background.
fn contrast(c: [u8; 3], bg: Color32) -> f32 {
    let lum = |r: u8, g: u8, b: u8| {
        let f = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
    };
    let (a, b) = (lum(c[0], c[1], c[2]), lum(bg.r(), bg.g(), bg.b()));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn darker(c: [u8; 3]) -> Color32 {
    let f = |v: u8| (f32::from(v) * 0.8) as u8;
    Color32::from_rgb(f(c[0]), f(c[1]), f(c[2]))
}

const K: f32 = 1.0 / REF_SCALE;

fn rect_u(origin: Pos2, r: [f32; 4], dy: f32) -> Rect {
    Rect::from_min_size(
        origin + vec2(r[0] * K, (r[1] - dy) * K),
        vec2(r[2] * K, r[3] * K),
    )
}

/// Draws the menu, its open sub-menus, and reacts to pointer and keys.
pub fn show(
    ctx: &egui::Context,
    state: &mut MenuState,
    root: &[MenuEntry],
    theme: &Theme,
    icons: &mut Icons,
) -> MenuEvent {
    let mut event = MenuEvent::None;
    let area_rect = ctx.content_rect();
    let measure = ctx.layer_painter(egui::LayerId::background());
    let font = FontId::proportional(size::LABEL);
    let text_w = |s: &str| {
        measure
            .layout_no_wrap(s.to_owned(), font.clone(), Color32::WHITE)
            .size()
            .x
            * REF_SCALE
    };
    let mut inside = false;
    let mut parent_rect: Option<Rect> = None;
    let mut depth = 0;
    // Keyboard first, on the deepest level.
    let keys = ctx.input(|i| {
        (
            i.key_pressed(Key::ArrowDown),
            i.key_pressed(Key::ArrowUp),
            i.key_pressed(Key::Enter),
            i.key_pressed(Key::ArrowRight),
            i.key_pressed(Key::ArrowLeft),
        )
    });
    let mut keep_levels = state.path.len() + 1;
    while depth < keep_levels {
        let entries = entries_at(root, &state.path, depth);
        if entries.is_empty() {
            break;
        }
        while state.hot.len() <= depth {
            state.hot.push(None);
            state.scroll.push(0.0);
        }
        let list = if depth == 0 && state.list {
            list_look(&state.id)
        } else {
            None
        };
        let min_w =
            list.map_or(0.0, |l| l.min_w)
                .max(if depth == 0 { state.opener_w } else { 0.0 });
        let mut pos = match parent_rect {
            None => state.pos,
            Some(pr) => pos2(pr.right() - 2.0 * K, pr.top() - FRAME * K),
        };
        let row_h = entries
            .iter()
            .find(|e| e.kind == EntryKind::Item)
            .and_then(|e| e.height)
            .map_or(ITEM_H, |h| h as f32);
        let room_below = (area_rect.bottom() - 8.0 - pos.y) * REF_SCALE;
        let max_rows_h = list
            .and_then(|l| l.max_rows)
            .map(|n| n as f32 * row_h + 12.0);
        let mut max_h = room_below;
        if let Some(m) = max_rows_h {
            max_h = max_h.min(m);
        }
        let mut p = plan(entries, min_w, max_h.max(3.0 * ITEM_H), &text_w);
        // Not enough room below: open upwards when that gives more room, else slide up.
        if pos.y + p.height * K > area_rect.bottom() - 4.0 && depth > 0 {
            pos.y = (area_rect.bottom() - 4.0 - p.height * K).max(area_rect.top());
        }
        if pos.x + p.width * K > area_rect.right() {
            pos.x = match parent_rect {
                Some(pr) => pr.left() + 2.0 * K - p.width * K,
                None => (area_rect.right() - p.width * K).max(0.0),
            };
        }
        let max_scroll = (p.content_height - p.height).max(0.0);
        if state.reveal && depth == 0 {
            if let Some(r) = p
                .rows
                .iter()
                .find(|r| entries[r.index].checked && entries[r.index].kind == EntryKind::Item)
            {
                state.scroll[0] =
                    (r.rect[1] - p.height / 2.0 + r.rect[3] / 2.0).clamp(0.0, max_scroll);
            }
            state.reveal = false;
        }
        let pointer = ctx.input(|i| i.pointer.hover_pos());
        let popup_rect = Rect::from_min_size(pos, vec2(p.width * K, p.height * K));
        if p.scrolls && pointer.is_some_and(|q| popup_rect.contains(q)) {
            let wheel = ctx.input(|i| i.smooth_scroll_delta.y);
            state.scroll[depth] -= wheel * REF_SCALE;
        }
        state.scroll[depth] = state.scroll[depth].clamp(0.0, max_scroll);
        let dy = state.scroll[depth];
        let view_w = p.width - if p.scrolls { SCROLLBAR_W } else { 0.0 };
        let mut sub_open: Option<(usize, Rect)> = None;
        let mut hot_now: Option<usize> = None;
        let resp = Area::new(Id::new(("shell-menu", depth)))
            .order(Order::Foreground)
            .fixed_pos(pos)
            .interactable(true)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(popup_rect.size(), Sense::hover());
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 0.0, theme.menu_bg);
                painter.rect_stroke(
                    rect,
                    0.0,
                    Stroke::new(K, theme.menu_border),
                    StrokeKind::Inside,
                );
                let view = Rect::from_min_max(
                    rect.min + vec2(FRAME * K, FRAME * K),
                    rect.max
                        - vec2(
                            (FRAME + if p.scrolls { SCROLLBAR_W } else { 0.0 }) * K,
                            FRAME * K,
                        ),
                );
                let clip = painter.with_clip_rect(view);
                for rp in &p.rows {
                    let e = &entries[rp.index];
                    let rr = rect_u(rect.min, rp.rect, dy);
                    if rr.bottom() < view.top() || rr.top() > view.bottom() {
                        continue;
                    }
                    match &e.kind {
                        EntryKind::Separator => {
                            let y = rr.center().y;
                            clip.line_segment(
                                [pos2(rr.left() + GUTTER * K * 0.5, y), pos2(rr.right(), y)],
                                Stroke::new(K, theme.separator),
                            );
                        }
                        EntryKind::Heading => {
                            clip.text(
                                pos2(rr.left() + 10.0 * K, rr.bottom() - 15.0 * K),
                                Align2::LEFT_CENTER,
                                &e.label,
                                FontId::proportional(size::LABEL),
                                theme.text_dim,
                            );
                        }
                        EntryKind::Item => {
                            let vis = rr.intersect(view);
                            let r = ui.interact(
                                vis,
                                Id::new(("mrow", depth, rp.index)),
                                if e.enabled {
                                    Sense::click()
                                } else {
                                    Sense::hover()
                                },
                            );
                            let hot =
                                (r.hovered() && e.enabled) || state.hot[depth] == Some(rp.index);
                            if r.hovered() && e.enabled {
                                hot_now = Some(rp.index);
                            }
                            let open_sub =
                                !e.children.is_empty() && state.path.get(depth) == Some(&rp.index);
                            if hot || open_sub {
                                clip.rect_filled(rr, CornerRadius::ZERO, theme.menu_hot);
                            }
                            let ink = theme.ink(e.enabled);
                            let list_row = e.height.is_some();
                            if e.checked && !list_row {
                                let b = Rect::from_center_size(
                                    pos2(rr.left() + 20.0 * K, rr.center().y),
                                    vec2(28.0 * K, 28.0 * K),
                                );
                                clip.rect_filled(b, CornerRadius::same(2), theme.checked);
                                icons.paint(
                                    ctx,
                                    &clip,
                                    e.icon.as_deref().unwrap_or("check"),
                                    Rect::from_center_size(b.center(), vec2(16.0 * K, 16.0 * K)),
                                    ink,
                                );
                            } else if let Some(name) = &e.icon {
                                icons.paint(
                                    ctx,
                                    &clip,
                                    name,
                                    Rect::from_center_size(
                                        pos2(rr.left() + 20.0 * K, rr.center().y),
                                        vec2(16.0 * K, 16.0 * K),
                                    ),
                                    ink,
                                );
                            }
                            if e.checked && list_row {
                                clip.rect_filled(rr, CornerRadius::ZERO, theme.checked);
                            }
                            let (x, align) = if e.centred {
                                (rr.center().x, Align2::CENTER_CENTER)
                            } else if list_row {
                                (rr.left() + 13.0 * K, Align2::LEFT_CENTER)
                            } else {
                                (rr.left() + GUTTER * K, Align2::LEFT_CENTER)
                            };
                            let two = e.detail.is_some();
                            let ty = if two {
                                rr.top() + 21.0 * K
                            } else {
                                rr.center().y
                            };
                            clip.text(
                                pos2(x, ty),
                                align,
                                &e.label,
                                FontId::proportional(size::LABEL),
                                ink,
                            );
                            if let Some(d) = &e.detail {
                                clip.text(
                                    pos2(x, rr.top() + 42.0 * K),
                                    align,
                                    d,
                                    FontId::proportional(size::LABEL - 1.0),
                                    theme.text_dim,
                                );
                            }
                            if let Some(s) = &e.shortcut {
                                clip.text(
                                    pos2(rr.right() - 14.0 * K, ty),
                                    Align2::RIGHT_CENTER,
                                    s,
                                    FontId::proportional(size::LABEL),
                                    theme.text_dim,
                                );
                            }
                            if !e.children.is_empty() {
                                icons.paint(
                                    ctx,
                                    &clip,
                                    "chevron-right",
                                    Rect::from_center_size(
                                        pos2(rr.right() - 18.0 * K, rr.center().y),
                                        vec2(12.0 * K, 12.0 * K),
                                    ),
                                    ink,
                                );
                                if open_sub || (r.hovered() && e.enabled) {
                                    sub_open = Some((rp.index, rr));
                                }
                            }
                            if r.clicked() && e.enabled {
                                if e.children.is_empty() {
                                    event = MenuEvent::Run(e.id.clone());
                                } else {
                                    sub_open = Some((rp.index, rr));
                                }
                            }
                        }
                        EntryKind::Filter => {
                            let sw = rect_u(
                                rect.min,
                                [
                                    FILTER_SWITCH[0],
                                    rp.rect[1] + FILTER_SWITCH[1] - FRAME,
                                    FILTER_SWITCH[2],
                                    FILTER_SWITCH[3],
                                ],
                                dy,
                            );
                            let r = ui.interact(sw, Id::new(("mfilter", depth)), Sense::click());
                            let key = Id::new("shell-menu-high-contrast");
                            let mut on = ctx.data(|d| d.get_temp::<bool>(key)).unwrap_or(false);
                            if r.clicked() {
                                on = !on;
                                ctx.data_mut(|d| d.insert_temp(key, on));
                            }
                            if r.hovered() {
                                clip.rect_filled(sw, CornerRadius::ZERO, theme.menu_hot);
                            }
                            clip.text(
                                pos2(sw.left() + 10.0 * K, sw.center().y),
                                Align2::LEFT_CENTER,
                                &e.label,
                                FontId::proportional(size::LABEL),
                                theme.text,
                            );
                            // The switch: a 36 x 18 u pill, knob left (off) or right (on). Derived.
                            let pill = Rect::from_center_size(
                                pos2(sw.right() - 28.0 * K, sw.center().y),
                                vec2(36.0 * K, 18.0 * K),
                            );
                            let ink = if on { theme.text } else { theme.text_dim };
                            clip.rect_stroke(
                                pill,
                                CornerRadius::same(255),
                                Stroke::new(K, ink),
                                StrokeKind::Inside,
                            );
                            let knob_x = if on {
                                pill.right() - 9.0 * K
                            } else {
                                pill.left() + 9.0 * K
                            };
                            clip.circle_filled(pos2(knob_x, pill.center().y), 5.0 * K, ink);
                        }
                        EntryKind::Swatches { rows, .. } => {
                            let high = ctx
                                .data(|d| d.get_temp::<bool>(Id::new("shell-menu-high-contrast")))
                                .unwrap_or(false);
                            let mut k = 0;
                            for row in rows {
                                for sw in row {
                                    let c = rp.cells[k];
                                    k += 1;
                                    if high && contrast(sw.rgb, theme.menu_bg) < HIGH_CONTRAST {
                                        continue;
                                    }
                                    let cr = rect_u(rect.min, c, dy);
                                    if cr.bottom() < view.top() || cr.top() > view.bottom() {
                                        continue;
                                    }
                                    let r = ui.interact(
                                        cr.intersect(view),
                                        Id::new(("msw", depth, &sw.id)),
                                        Sense::click(),
                                    );
                                    r.clone().on_hover_text(&sw.name);
                                    let ring = if sw.checked {
                                        theme.text
                                    } else if r.hovered() {
                                        theme.text_dim
                                    } else {
                                        theme.menu_ring
                                    };
                                    clip.rect_stroke(
                                        cr,
                                        CornerRadius::ZERO,
                                        Stroke::new(if sw.checked { 2.0 * K } else { K }, ring),
                                        StrokeKind::Inside,
                                    );
                                    let inner = cr.shrink(2.0 * K);
                                    clip.rect_filled(
                                        inner,
                                        CornerRadius::ZERO,
                                        Color32::from_rgb(sw.rgb[0], sw.rgb[1], sw.rgb[2]),
                                    );
                                    clip.rect_stroke(
                                        inner,
                                        CornerRadius::ZERO,
                                        Stroke::new(K, darker(sw.rgb)),
                                        StrokeKind::Inside,
                                    );
                                    if r.clicked() {
                                        event = MenuEvent::Run(sw.id.clone());
                                    }
                                }
                            }
                        }
                        EntryKind::Tiles { tiles, .. } => {
                            for (k, t) in tiles.iter().enumerate() {
                                let cr = rect_u(rect.min, rp.cells[k], dy);
                                if cr.bottom() < view.top() || cr.top() > view.bottom() {
                                    continue;
                                }
                                let r = ui.interact(
                                    cr.intersect(view),
                                    Id::new(("mtile", depth, &t.id)),
                                    Sense::click(),
                                );
                                r.clone().on_hover_text(&t.label);
                                let fill = t
                                    .fill
                                    .map_or(theme.field, |c| Color32::from_rgb(c[0], c[1], c[2]));
                                let ink = t
                                    .ink
                                    .map_or(theme.text, |c| Color32::from_rgb(c[0], c[1], c[2]));
                                if r.hovered() || t.checked {
                                    clip.rect_filled(cr, CornerRadius::ZERO, theme.gallery_cell);
                                }
                                // The sample cell: 4 u inside the tile on every side (28 u of the
                                // 36 u row, measured; the horizontal inset is derived), then the
                                // style's real fill, borders and text size.
                                let inset = cr.shrink(4.0 * K);
                                clip.rect_filled(inset, CornerRadius::ZERO, fill);
                                let line = |a: Pos2, b: Pos2, e: ([u8; 3], u8)| {
                                    clip.line_segment(
                                        [a, b],
                                        Stroke::new(
                                            f32::from(e.1) * K,
                                            Color32::from_rgb(e.0[0], e.0[1], e.0[2]),
                                        ),
                                    );
                                };
                                if let Some(e) = t.borders[0] {
                                    let x = inset.left() + f32::from(e.1) * K / 2.0;
                                    line(pos2(x, inset.top()), pos2(x, inset.bottom()), e);
                                }
                                if let Some(e) = t.borders[1] {
                                    let x = inset.right() - f32::from(e.1) * K / 2.0;
                                    line(pos2(x, inset.top()), pos2(x, inset.bottom()), e);
                                }
                                if let Some(e) = t.borders[2] {
                                    let y = inset.top() + f32::from(e.1) * K / 2.0;
                                    line(pos2(inset.left(), y), pos2(inset.right(), y), e);
                                }
                                if let Some(e) = t.borders[3] {
                                    let y = inset.bottom() - f32::from(e.1) * K / 2.0;
                                    line(pos2(inset.left(), y), pos2(inset.right(), y), e);
                                }
                                let px = (size::LABEL * t.text_pct.clamp(50, 250) as f32 / 100.0)
                                    .min(size::LABEL * 2.0);
                                let family = t
                                    .font
                                    .as_ref()
                                    .map_or(FontFamily::Proportional, |f| f.family.clone());
                                let fake_bold = t.font.as_ref().is_some_and(|f| f.fake_bold);
                                let fake_italic = t.font.as_ref().is_some_and(|f| f.fake_italic);
                                let mut job = egui::text::LayoutJob::default();
                                job.append(
                                    &t.label,
                                    0.0,
                                    egui::TextFormat {
                                        font_id: FontId::new(px, family),
                                        color: ink,
                                        italics: fake_italic,
                                        ..Default::default()
                                    },
                                );
                                let galley = clip.layout_job(job);
                                let at = inset.center() - galley.size() / 2.0;
                                if fake_bold {
                                    clip.galley(at + vec2(K, 0.0), galley.clone(), ink);
                                }
                                clip.galley(at, galley, ink);
                                if r.hovered() || t.checked {
                                    clip.rect_stroke(
                                        cr,
                                        CornerRadius::ZERO,
                                        Stroke::new(K, theme.gallery_ring),
                                        StrokeKind::Inside,
                                    );
                                }
                                if r.clicked() {
                                    event = MenuEvent::Run(t.id.clone());
                                }
                            }
                        }
                    }
                }
                if p.scrolls {
                    scrollbar(
                        ui,
                        &painter,
                        theme,
                        rect,
                        &p,
                        &mut state.scroll[depth],
                        depth,
                    );
                }
                let _ = view_w;
            });
        inside |= resp
            .response
            .rect
            .contains(pointer.unwrap_or(Pos2::new(-1.0, -1.0)));
        if let Some(h) = hot_now {
            state.hot[depth] = Some(h);
        }
        // Keyboard on the deepest level.
        if depth + 1 == keep_levels {
            let order: Vec<usize> = entries
                .iter()
                .enumerate()
                .filter(|(_, e)| interactive(e))
                .map(|(i, _)| i)
                .collect();
            let cur = state.hot[depth].and_then(|h| order.iter().position(|o| *o == h));
            let mut moved = false;
            if keys.0 && !order.is_empty() {
                state.hot[depth] = Some(order[cur.map_or(0, |c| (c + 1).min(order.len() - 1))]);
                moved = true;
            } else if keys.1 && !order.is_empty() {
                state.hot[depth] =
                    Some(order[cur.map_or(order.len() - 1, |c| c.saturating_sub(1))]);
                moved = true;
            }
            if moved && let Some(h) = state.hot[depth] {
                let row = &p.rows[h];
                if row.rect[1] < dy {
                    state.scroll[depth] = row.rect[1] - FRAME;
                } else if row.rect[1] + row.rect[3] > dy + p.height {
                    state.scroll[depth] = row.rect[1] + row.rect[3] - p.height + FRAME;
                }
            }
            if keys.2
                && let Some(h) = state.hot[depth]
            {
                let e = &entries[h];
                if e.children.is_empty() {
                    event = MenuEvent::Run(e.id.clone());
                } else {
                    state.path.push(h);
                    state.hot.push(Some(0));
                    state.scroll.push(0.0);
                    keep_levels += 1;
                }
            } else if keys.3
                && let Some(h) = state.hot[depth]
                && !entries[h].children.is_empty()
            {
                state.path.push(h);
                state
                    .hot
                    .push(entries[h].children.iter().position(interactive));
                state.scroll.push(0.0);
                keep_levels += 1;
            } else if keys.4 && depth > 0 {
                state.path.truncate(depth - 1);
                state.hot.truncate(depth);
                state.scroll.truncate(depth);
                break;
            }
        }
        // Sub-menu opened by hover or click; a different hot row at this level closes it.
        match sub_open {
            Some((i, rr)) if state.path.get(depth) != Some(&i) => {
                state.path.truncate(depth);
                state.path.push(i);
                state.hot.truncate(depth + 1);
                state.scroll.truncate(depth + 1);
                keep_levels = depth + 2;
                parent_rect = Some(rr);
                depth += 1;
                continue;
            }
            Some((_, rr)) => parent_rect = Some(rr),
            None => {
                if state.path.len() > depth && hot_now.is_some_and(|h| state.path[depth] != h) {
                    state.path.truncate(depth);
                    state.hot.truncate(depth + 1);
                    state.scroll.truncate(depth + 1);
                    keep_levels = depth + 1;
                } else if let Some(rr) = state
                    .path
                    .get(depth)
                    .and_then(|i| p.rows.get(*i))
                    .map(|r| rect_u(popup_rect.min, r.rect, dy))
                {
                    parent_rect = Some(rr);
                }
            }
        }
        let _ = &mut p;
        depth += 1;
    }
    let (pressed, esc) = ctx.input(|i| (i.pointer.any_pressed(), i.key_pressed(Key::Escape)));
    if matches!(event, MenuEvent::None) && (esc || (pressed && !inside && !state.fresh)) {
        event = MenuEvent::Close;
    }
    state.fresh = false;
    event
}

fn scrollbar(
    ui: &egui::Ui,
    painter: &Painter,
    theme: &Theme,
    rect: Rect,
    p: &Plan,
    scroll: &mut f32,
    depth: usize,
) {
    let track = Rect::from_min_max(
        pos2(
            rect.right() - (FRAME + SCROLLBAR_W) * K,
            rect.top() + FRAME * K,
        ),
        pos2(rect.right() - FRAME * K, rect.bottom() - FRAME * K),
    );
    painter.rect_filled(track, 0.0, theme.scroll_track);
    let max_scroll = (p.content_height - p.height).max(1.0);
    let ratio = (p.height / p.content_height).clamp(0.1, 1.0);
    let thumb_h = (track.height() * ratio).max(24.0 * K);
    let free = track.height() - thumb_h;
    let top = track.top() + free * (*scroll / max_scroll);
    let thumb = Rect::from_min_size(
        pos2(track.left() + 6.0 * K, top),
        vec2(SCROLLBAR_W * K - 12.0 * K, thumb_h),
    );
    let r = ui.interact(
        thumb.expand2(vec2(4.0 * K, 0.0)),
        Id::new(("mscroll", depth)),
        Sense::drag(),
    );
    if r.dragged() && free > 0.0 {
        *scroll = (*scroll + r.drag_delta().y / free * max_scroll).clamp(0.0, max_scroll);
    }
    painter.rect_filled(thumb, CornerRadius::same(2), theme.scroll_thumb);
}
