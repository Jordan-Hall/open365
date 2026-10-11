//! Painting: widgets become `open365-gfx` draw calls, in device pixels, from the snapped device
//! rectangles of the tree.

use open365_gfx::{Color, Decorations, DrawList, Path, Rect as GRect, TextLayoutOptions};

use crate::geom::{DeviceRect, Rect};
use crate::theme::Token;
use crate::tree::{
    Content, DROPDOWN_ARROW_W, MENU_PAD, MENU_ROW_H, MENU_SEP_H, MENU_TEXT_X, MenuState, Node,
    NodeId, SPINNER_BUTTON_W, Widget, tab_rects,
};
use crate::ui::Ui;

/// Which parts of the UI to paint. Tests use the partial layers to isolate geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaintLayers {
    pub backgrounds: bool,
    pub text: bool,
    /// A one pixel outline around every widget (and tab) in `debug_color`.
    pub debug_bounds: bool,
}

impl Default for PaintLayers {
    fn default() -> Self {
        Self {
            backgrounds: true,
            text: true,
            debug_bounds: false,
        }
    }
}

pub const DEBUG_COLOR: Color = Color::rgb(255, 0, 255);

impl Ui {
    fn dev(&self, r: Rect) -> DeviceRect {
        self.scale.snap(r)
    }

    fn color(&self, t: Token) -> Color {
        self.theme.color(t)
    }

    fn text_px(&self) -> f32 {
        self.text_size * self.scale.0
    }

    /// Width in device pixels of `text` at the UI text size.
    pub fn measure(&self, text: &str) -> f32 {
        self.measure_px(text, self.text_px())
    }

    /// Width in device pixels of `text` at `px` pixels.
    pub fn measure_px(&self, text: &str, px: f32) -> f32 {
        self.assets
            .layout_text(self.font, text, px, 0.0, 0.0, TextLayoutOptions::default())
            .width
    }

    /// Draws `text` inside `r`, `align` 0 left, 1 centre; returns the x it started at.
    fn put_text(
        &mut self,
        list: &mut DrawList,
        text: &str,
        r: DeviceRect,
        align: f32,
        color: Color,
        decor: Decorations,
    ) -> f32 {
        let px = self.text_px();
        self.put_text_px(list, text, r, align, color, decor, px)
    }

    #[allow(clippy::too_many_arguments)]
    fn put_text_px(
        &mut self,
        list: &mut DrawList,
        text: &str,
        r: DeviceRect,
        align: f32,
        color: Color,
        decor: Decorations,
        px: f32,
    ) -> f32 {
        if text.is_empty() {
            return r.x0 as f32;
        }
        let Some(m) = self.assets.font_metrics(self.font, px) else {
            return r.x0 as f32;
        };
        let w = self.measure_px(text, px);
        let x = (r.x0 as f32 + (r.width() as f32 - w) * align)
            .max(r.x0 as f32)
            .floor();
        let base =
            (r.y0 as f32 + (r.height() as f32 - (m.ascent + m.descent)) / 2.0 + m.ascent).round();
        let lay =
            self.assets
                .layout_text(self.font, text, px, x, base, TextLayoutOptions::default());
        list.push_clip(GRect::from_edges(
            r.x0 as f32,
            r.y0 as f32,
            r.x1 as f32,
            r.y1 as f32,
        ));
        list.text(&mut self.assets, self.font, &lay, px, color, decor);
        list.pop_clip();
        x
    }

    /// Splits `text` into at most two lines no wider than `max_px`: at a newline, else at the
    /// space nearest the middle when it does not fit.
    fn wrap_two(&self, text: &str, max_px: f32, px: f32) -> Vec<String> {
        if let Some((a, b)) = text.split_once('\n') {
            return vec![a.to_owned(), b.to_owned()];
        }
        if self.measure_px(text, px) <= max_px {
            return vec![text.to_owned()];
        }
        let mid = text.len() / 2;
        let cut = text
            .char_indices()
            .filter(|(_, c)| *c == ' ')
            .min_by_key(|(i, _)| i.abs_diff(mid))
            .map(|(i, _)| i);
        match cut {
            Some(i) => vec![text[..i].to_owned(), text[i + 1..].to_owned()],
            None => vec![text.to_owned()],
        }
    }

    fn put_icon(&mut self, list: &mut DrawList, node: &Node, at: DeviceRect, color: Color) {
        if let Some(icon) = node.look.icon {
            list.icon(&mut self.assets, icon, at.to_gfx(), color);
        }
    }

    fn face_px(&self, node: &Node) -> f32 {
        node.look
            .text_size
            .map_or_else(|| self.text_px(), |t| t * self.scale.0)
    }

    /// Icon and label of a button-like node inside `r`, per its [`crate::Look`].
    fn paint_face(
        &mut self,
        list: &mut DrawList,
        node: &Node,
        r: DeviceRect,
        ink: Color,
        layers: PaintLayers,
    ) {
        if !layers.text {
            return;
        }
        let look = node.look;
        let px = self.face_px(node);
        let ink = look
            .ink
            .filter(|_| node.enabled)
            .map_or(ink, |t| self.color(t));
        let mode = match look.content {
            Content::Auto => match (look.icon.is_some(), node.label.is_empty()) {
                (true, false) => Content::IconBeside,
                (true, true) => Content::IconOnly,
                _ => Content::TextOnly,
            },
            c => c,
        };
        let isz = self.scale.edge(look.icon_px).max(1);
        let icon_at = |x0: i32, y0: i32| DeviceRect {
            x0,
            y0,
            x1: x0 + isz,
            y1: y0 + isz,
        };
        let decor = Decorations::default();
        match mode {
            Content::IconOnly => {
                let x0 = r.x0 + (r.width() - isz) / 2;
                let y0 = r.y0 + (r.height() - isz) / 2;
                self.put_icon(list, node, icon_at(x0, y0), ink);
            }
            Content::IconBeside => {
                let gap = self.scale.edge(4.0);
                let tw = self.measure_px(&node.label, px).ceil() as i32;
                let chev = if look.chevron {
                    self.scale.edge(12.0)
                } else {
                    0
                };
                let total = isz + gap + tw + chev;
                let pad = self.scale.edge(look.pad);
                let room = (r.width() - 2 * pad - total).max(0);
                let x0 = r.x0 + pad + (room as f32 * look.align).floor() as i32;
                let y0 = r.y0 + (r.height() - isz) / 2;
                self.put_icon(list, node, icon_at(x0, y0), ink);
                let tr = DeviceRect {
                    x0: x0 + isz + gap,
                    ..r
                };
                self.put_text_px(list, &node.label, tr, 0.0, ink, decor, px);
                if look.chevron {
                    let cx = (x0 + isz + gap + tw) as f32 + self.scale.0 * 8.0;
                    self.chevron(list, cx, (r.y0 + r.y1) as f32 / 2.0, ink, false);
                }
            }
            Content::IconAbove => {
                let x0 = r.x0 + (r.width() - isz) / 2;
                let y0 = r.y0 + self.scale.edge(look.icon_top);
                self.put_icon(list, node, icon_at(x0, y0), ink);
                if node.label.is_empty() {
                    return;
                }
                let lines = self.wrap_two(&node.label, r.width() as f32 - 2.0, px);
                let Some(m) = self.assets.font_metrics(self.font, px) else {
                    return;
                };
                let lh = (m.ascent + m.descent).round() as i32;
                let mut y = y0 + isz + self.scale.edge(2.0);
                for l in &lines {
                    let lr = DeviceRect {
                        y0: y,
                        y1: y + lh,
                        ..r
                    };
                    self.put_text_px(list, l, lr, 0.5, ink, decor, px);
                    y += lh;
                }
                if look.chevron {
                    let last = lines.last().map_or("", String::as_str);
                    let w = self.measure_px(last, px);
                    let cx = r.x0 as f32 + r.width() as f32 / 2.0 + w / 2.0 + self.scale.0 * 7.0;
                    self.chevron(list, cx, (y - lh / 2) as f32 + 1.0, ink, false);
                }
            }
            Content::TextOnly | Content::Auto => {
                let pad = self.scale.edge(look.pad);
                let right = if look.chevron {
                    self.scale.edge(14.0)
                } else {
                    0
                };
                let tr = DeviceRect {
                    x0: r.x0 + pad,
                    x1: (r.x1 - pad - right).max(r.x0 + pad),
                    ..r
                };
                self.put_text_px(list, &node.label, tr, look.align, ink, decor, px);
                if look.chevron {
                    let cx = (r.x1 - pad) as f32 - self.scale.0 * 6.0;
                    self.chevron(list, cx, (r.y0 + r.y1) as f32 / 2.0, ink, false);
                }
            }
        }
    }

    fn chevron(&mut self, list: &mut DrawList, cx: f32, cy: f32, color: Color, up: bool) {
        let s = self.scale.0;
        let (w, h) = (3.5 * s, 2.0 * s);
        let dir = if up { -1.0 } else { 1.0 };
        let mut p = Path::new();
        p.move_to(cx - w, cy - h * dir / 2.0)
            .line_to(cx, cy + h * dir / 2.0)
            .line_to(cx + w, cy - h * dir / 2.0);
        list.stroke_path(
            &mut self.assets,
            &p,
            s.max(1.0),
            open365_gfx::Join::Miter,
            open365_gfx::Cap::Butt,
            color,
        );
    }

    fn focus_ring(&self, list: &mut DrawList, r: DeviceRect, id: NodeId) {
        if self.focus == Some(id) && self.focus_visible {
            let inner = GRect::from_edges(
                (r.x0 + 1) as f32,
                (r.y0 + 1) as f32,
                (r.x1 - 1) as f32,
                (r.y1 - 1) as f32,
            );
            list.rect_outline(inner, 1.0, self.color(Token::FocusRing));
        }
    }

    /// Paints the whole UI into `list`.
    pub fn paint(&mut self, list: &mut DrawList, layers: PaintLayers) {
        self.tree.layout();
        let order = self.tree.paint_order();
        for id in &order {
            let clip = self.tree.clip_rect(*id).map(|r| self.dev(r));
            if let Some(c) = clip {
                list.push_clip(c.to_gfx());
            }
            self.paint_node(list, *id, layers);
            if clip.is_some() {
                list.pop_clip();
            }
        }
        // Popups and overlays come last so they sit above everything.
        for id in &order {
            self.paint_popup(list, *id, layers);
        }
        if layers.text {
            self.paint_keytips(list);
            self.paint_tooltip(list);
        }
        if layers.debug_bounds {
            for r in self.debug_rects() {
                list.rect_outline(r.to_gfx(), 1.0, DEBUG_COLOR);
            }
        }
    }

    /// Device rectangles of every widget and every tab, for geometry tests.
    pub fn debug_rects(&self) -> Vec<DeviceRect> {
        let mut out = Vec::new();
        for id in self.tree.paint_order() {
            if id == self.tree.root() {
                continue;
            }
            let Some(n) = self.tree.get(id) else { continue };
            match &n.widget {
                Widget::Panel | Widget::Tooltip => {}
                Widget::TabStrip { tabs, gap, .. } => {
                    let r = self.tree.abs_rect(id);
                    for t in tab_rects(crate::geom::Point::new(r.x, r.y), r.h, tabs, *gap) {
                        out.push(self.dev(t));
                    }
                }
                Widget::Menu(m) if !m.open => {}
                _ => out.push(self.tree.device_rect(id, self.scale)),
            }
        }
        out
    }

    fn paint_node(&mut self, list: &mut DrawList, id: NodeId, layers: PaintLayers) {
        let Some(node) = self.tree.get(id).cloned() else {
            return;
        };
        let r = self.tree.device_rect(id, self.scale);
        let enabled = node.enabled;
        let ink = if enabled {
            node.look.ink_rgb.unwrap_or_else(|| self.color(Token::Text))
        } else {
            self.color(Token::TextDisabled)
        };
        let hovered = self.hover == Some(id);
        let pressed = self.pressed == Some(id) && hovered;
        let still = node.look.still;
        let state_fill = |s: &Self, base: Option<Color>| -> Option<Color> {
            if !enabled || still {
                None
            } else if pressed {
                Some(s.color(Token::Pressed))
            } else if hovered {
                Some(node.look.hover_rgb.unwrap_or_else(|| s.color(Token::Hover)))
            } else {
                base
            }
        };
        let gr = r.to_gfx();
        if layers.backgrounds && !matches!(node.widget, Widget::Menu(_)) {
            let rad = node.look.radius * self.scale.0;
            let fill_c = node.look.fill_rgb.or(node.look.fill.map(|t| self.color(t)));
            let border_c = node
                .look
                .border_rgb
                .or(node.look.border.map(|t| self.color(t)));
            if let Some(c) = fill_c {
                if rad > 0.0 {
                    list.fill_rrect(gr, rad, c);
                } else {
                    list.fill_rect(gr, c);
                }
            }
            if let Some(c) = border_c {
                if rad > 0.0 {
                    list.stroke_rrect(gr, rad, 1.0, c);
                } else {
                    list.rect_outline(gr, 1.0, c);
                }
            }
            if let Some((c, h)) = node.look.bar {
                let hp = self.scale.edge(h).max(1);
                list.fill_rect(
                    GRect::from_edges(r.x0 as f32, (r.y1 - hp) as f32, r.x1 as f32, r.y1 as f32),
                    c,
                );
            }
        }
        let radius = node.look.radius * self.scale.0;
        let fill = move |list: &mut DrawList, g: GRect, c: Color| {
            if radius > 0.0 {
                list.fill_rrect(g, radius, c);
            } else {
                list.fill_rect(g, c);
            }
        };
        match &node.widget {
            Widget::Panel | Widget::Tooltip | Widget::Menu(_) | Widget::Canvas { .. } => {}
            Widget::Label => {
                self.paint_face(list, &node, r, ink, layers);
            }
            Widget::Button { toggled } => {
                if layers.backgrounds {
                    let base = toggled.filter(|t| *t).map(|_| self.color(Token::Checked));
                    if let Some(c) = state_fill(self, base) {
                        fill(list, gr, c);
                    }
                }
                self.paint_face(list, &node, r, ink, layers);
                self.focus_ring(list, r, id);
            }
            Widget::SplitButton { arrow_w } => {
                let below = node.look.arrow_below;
                let ab = self.tree.abs_rect(id);
                let (main, arrow) = if below {
                    let y = self.scale.edge(ab.bottom() - arrow_w);
                    (DeviceRect { y1: y, ..r }, DeviceRect { y0: y, ..r })
                } else {
                    let x = self.scale.edge(ab.right() - arrow_w);
                    (DeviceRect { x1: x, ..r }, DeviceRect { x0: x, ..r })
                };
                if layers.backgrounds && enabled && !still && (hovered || pressed) {
                    // Each half lights up on its own, as in Office.
                    let in_arrow = arrow.to_gfx().contains_point(self.pointer_device());
                    let part = if in_arrow { arrow } else { main };
                    let tok = if pressed {
                        Token::Pressed
                    } else {
                        Token::Hover
                    };
                    fill(list, gr, self.color(Token::Hover).with_opacity(0.45));
                    fill(list, part.to_gfx(), self.color(tok));
                }
                if layers.text {
                    if below {
                        let mut face = node.clone();
                        face.label = String::new();
                        face.look.content = Content::IconOnly;
                        self.paint_face(list, &face, main, ink, layers);
                        let px = self.face_px(&node);
                        let lines = self.wrap_two(&node.label, r.width() as f32 - 2.0, px);
                        let lh = self
                            .assets
                            .font_metrics(self.font, px)
                            .map_or(14, |m| (m.ascent + m.descent).round() as i32);
                        let mut y = arrow.y0 + self.scale.edge(2.0);
                        for l in &lines {
                            let lr = DeviceRect {
                                y0: y,
                                y1: y + lh,
                                ..arrow
                            };
                            self.put_text_px(list, l, lr, 0.5, ink, Decorations::default(), px);
                            y += lh;
                        }
                        self.chevron(
                            list,
                            (arrow.x0 + arrow.x1) as f32 / 2.0,
                            y as f32 + self.scale.0 * 3.0,
                            ink,
                            false,
                        );
                    } else {
                        self.paint_face(list, &node, main, ink, layers);
                        self.chevron(
                            list,
                            (arrow.x0 + arrow.x1) as f32 / 2.0,
                            (r.y0 + r.y1) as f32 / 2.0,
                            ink,
                            false,
                        );
                    }
                }
                self.focus_ring(list, r, id);
            }
            Widget::Dropdown {
                items,
                selected,
                open,
            } => {
                self.paint_field(list, r, id, layers);
                if layers.text {
                    let text = items.get(*selected).map_or("", String::as_str);
                    let inner = DeviceRect {
                        x0: r.x0 + self.scale.edge(6.0),
                        x1: r.x1 - self.scale.edge(DROPDOWN_ARROW_W),
                        ..r
                    };
                    self.put_text(list, text, inner, 0.0, ink, Decorations::default());
                    self.chevron(
                        list,
                        r.x1 as f32 - self.scale.0 * DROPDOWN_ARROW_W / 2.0,
                        (r.y0 + r.y1) as f32 / 2.0,
                        ink,
                        *open,
                    );
                }
            }
            Widget::Combo { edit, open, .. } => {
                self.paint_field(list, r, id, layers);
                let inner = DeviceRect {
                    x0: r.x0 + self.scale.edge(4.0),
                    x1: r.x1 - self.scale.edge(DROPDOWN_ARROW_W),
                    ..r
                };
                self.paint_edit(list, id, edit, inner, ink, layers);
                if layers.text {
                    self.chevron(
                        list,
                        r.x1 as f32 - self.scale.0 * DROPDOWN_ARROW_W / 2.0,
                        (r.y0 + r.y1) as f32 / 2.0,
                        ink,
                        *open,
                    );
                }
            }
            Widget::TextField(edit) => {
                self.paint_field(list, r, id, layers);
                let inner = DeviceRect {
                    x0: r.x0 + self.scale.edge(4.0),
                    x1: r.x1 - self.scale.edge(4.0),
                    ..r
                };
                self.paint_edit(list, id, edit, inner, ink, layers);
            }
            Widget::Spinner { edit, .. } => {
                self.paint_field(list, r, id, layers);
                let bw = self.scale.edge(SPINNER_BUTTON_W);
                let inner = DeviceRect {
                    x0: r.x0 + self.scale.edge(4.0),
                    x1: r.x1 - bw,
                    ..r
                };
                self.paint_edit(list, id, edit, inner, ink, layers);
                if layers.text {
                    let cx = (r.x1 - bw / 2) as f32;
                    self.chevron(list, cx, r.y0 as f32 + r.height() as f32 * 0.28, ink, true);
                    self.chevron(list, cx, r.y0 as f32 + r.height() as f32 * 0.72, ink, false);
                }
            }
            Widget::Checkbox { checked } => {
                let b = self.scale.edge(16.0);
                let box_r = DeviceRect {
                    x0: r.x0,
                    y0: r.y0 + (r.height() - b) / 2,
                    x1: r.x0 + b,
                    y1: r.y0 + (r.height() - b) / 2 + b,
                };
                if layers.backgrounds {
                    if *checked {
                        list.fill_rrect(
                            box_r.to_gfx(),
                            2.0 * self.scale.0,
                            self.color(Token::Accent),
                        );
                    } else {
                        list.stroke_rrect(
                            box_r.to_gfx(),
                            2.0 * self.scale.0,
                            1.0,
                            self.color(Token::FieldBorder),
                        );
                    }
                }
                if *checked && layers.text {
                    let s = b as f32;
                    let mut p = Path::new();
                    p.move_to(box_r.x0 as f32 + s * 0.25, box_r.y0 as f32 + s * 0.52)
                        .line_to(box_r.x0 as f32 + s * 0.43, box_r.y0 as f32 + s * 0.70)
                        .line_to(box_r.x0 as f32 + s * 0.76, box_r.y0 as f32 + s * 0.32);
                    let on = self.color(Token::OnAccent);
                    list.stroke_path(
                        &mut self.assets,
                        &p,
                        1.5 * self.scale.0,
                        open365_gfx::Join::Miter,
                        open365_gfx::Cap::Butt,
                        on,
                    );
                }
                if layers.text {
                    let label = DeviceRect {
                        x0: box_r.x1 + self.scale.edge(8.0),
                        ..r
                    };
                    self.put_text(list, &node.label, label, 0.0, ink, Decorations::default());
                }
                self.focus_ring(list, r, id);
            }
            Widget::Radio { checked, .. } => {
                let b = self.scale.edge(16.0);
                let y0 = r.y0 + (r.height() - b) / 2;
                let circle =
                    GRect::from_edges(r.x0 as f32, y0 as f32, (r.x0 + b) as f32, (y0 + b) as f32);
                if layers.backgrounds {
                    let ring = Path::ellipse(circle);
                    let col = if *checked {
                        self.color(Token::Accent)
                    } else {
                        self.color(Token::FieldBorder)
                    };
                    list.stroke_path(
                        &mut self.assets,
                        &ring,
                        self.scale.0.max(1.0),
                        open365_gfx::Join::Miter,
                        open365_gfx::Cap::Butt,
                        col,
                    );
                    if *checked {
                        let dot = Path::ellipse(circle.inflate(-(b as f32) * 0.3));
                        list.fill_path(&mut self.assets, &dot, col);
                    }
                }
                if layers.text {
                    let label = DeviceRect {
                        x0: r.x0 + b + self.scale.edge(8.0),
                        ..r
                    };
                    self.put_text(list, &node.label, label, 0.0, ink, Decorations::default());
                }
                self.focus_ring(list, r, id);
            }
            Widget::Slider {
                value, min, max, ..
            } => {
                if layers.backgrounds {
                    let ar = self.tree.abs_rect(id);
                    let cy = ((r.y0 + r.y1) / 2) as f32;
                    let line = self.color(Token::TextDim);
                    list.fill_rect(
                        GRect::from_edges(r.x0 as f32, cy, r.x1 as f32, cy + 1.0),
                        line,
                    );
                    // Centre tick (100 %) and the thumb.
                    let mid = ((r.x0 + r.x1) / 2) as f32;
                    let s = self.scale.0;
                    list.fill_rect(
                        GRect::from_edges(mid, cy - 3.0 * s, mid + 1.0, cy + 4.0 * s),
                        line,
                    );
                    let room = (ar.w - crate::tree::SLIDER_THUMB_W).max(1.0);
                    let t = if max > min {
                        (value - min) / (max - min)
                    } else {
                        0.0
                    };
                    let thumb = Rect::new(
                        ar.x + t * room,
                        ar.y + (ar.h - crate::tree::SLIDER_THUMB_H) / 2.0,
                        crate::tree::SLIDER_THUMB_W,
                        crate::tree::SLIDER_THUMB_H,
                    );
                    let col = if self.pressed == Some(id) || hovered {
                        self.color(Token::Text)
                    } else {
                        self.color(Token::TextDim)
                    };
                    list.fill_rect(self.dev(thumb).to_gfx(), col);
                }
                self.focus_ring(list, r, id);
            }
            Widget::ScrollBar(s) => {
                if layers.backgrounds {
                    list.fill_rect(gr, self.color(Token::ScrollTrack));
                    let ar = self.tree.abs_rect(id);
                    let track = if s.vertical { ar.h } else { ar.w };
                    let (off, len) = s.thumb(track);
                    let pad = 2.0;
                    let thumb = if s.vertical {
                        Rect::new(ar.x + pad, ar.y + off, ar.w - 2.0 * pad, len)
                    } else {
                        Rect::new(ar.x + off, ar.y + pad, len, ar.h - 2.0 * pad)
                    };
                    let t = self.dev(thumb).to_gfx();
                    list.fill_rrect(
                        t,
                        (thumb.w.min(thumb.h) / 2.0) * self.scale.0,
                        self.color(Token::ScrollThumb),
                    );
                }
            }
            Widget::TabStrip {
                tabs,
                selected,
                gap,
            } => {
                let ar = self.tree.abs_rect(id);
                let tpx = self.face_px(&node);
                let rects = tab_rects(crate::geom::Point::new(ar.x, ar.y), ar.h, tabs, *gap);
                for (i, (t, tr)) in tabs.iter().zip(rects).enumerate() {
                    let dr = self.dev(tr);
                    let sel = i == *selected;
                    let col = if t.enabled {
                        self.color(Token::TabText)
                    } else {
                        self.color(Token::TextDisabled)
                    };
                    if layers.backgrounds
                        && self.hover == Some(id)
                        && dr.to_gfx().contains_point(self.pointer_device())
                        && t.enabled
                        && !sel
                    {
                        list.fill_rect(dr.to_gfx(), self.color(Token::Hover));
                    }
                    if layers.text {
                        let x = self.put_text_px(
                            list,
                            &t.label,
                            dr,
                            0.5,
                            col,
                            Decorations::default(),
                            tpx,
                        );
                        if sel && layers.backgrounds {
                            let w = self.measure_px(&t.label, tpx).ceil();
                            let h = self.scale.edge(3.0).max(2);
                            list.fill_rect(
                                GRect::from_edges(
                                    x,
                                    (dr.y1 - h - self.scale.edge(5.0)) as f32,
                                    x + w,
                                    (dr.y1 - self.scale.edge(5.0)) as f32,
                                ),
                                self.color(Token::TabUnderline),
                            );
                        }
                    }
                }
                self.focus_ring(list, r, id);
            }
        }
    }

    fn pointer_device(&self) -> (f32, f32) {
        (self.pointer.x * self.scale.0, self.pointer.y * self.scale.0)
    }

    fn paint_field(&mut self, list: &mut DrawList, r: DeviceRect, id: NodeId, layers: PaintLayers) {
        if !layers.backgrounds {
            return;
        }
        let focused = self.focus == Some(id);
        list.fill_rect(r.to_gfx(), self.color(Token::Field));
        list.rect_outline(r.to_gfx(), 1.0, self.color(Token::FieldBorder));
        if focused {
            let h = self.scale.edge(2.0).max(1) as f32;
            list.fill_rect(
                GRect::from_edges(r.x0 as f32, r.y1 as f32 - h, r.x1 as f32, r.y1 as f32),
                self.color(Token::Accent),
            );
        }
    }

    fn paint_edit(
        &mut self,
        list: &mut DrawList,
        id: NodeId,
        e: &crate::textedit::TextEdit,
        area: DeviceRect,
        ink: Color,
        layers: PaintLayers,
    ) {
        let px = self.text_px();
        let Some(m) = self.assets.font_metrics(self.font, px) else {
            return;
        };
        let base =
            (area.y0 as f32 + (area.height() as f32 - (m.ascent + m.descent)) / 2.0 + m.ascent)
                .round();
        // The composition is shown inline at the caret, underlined.
        let (a, b) = e.selection();
        let shown: String = match &e.composition {
            Some(c) => format!("{}{}{}", &e.text[..b], c.text, &e.text[b..]),
            None => e.text.clone(),
        };
        let lay = self.assets.layout_text(
            self.font,
            &shown,
            px,
            area.x0 as f32,
            base,
            TextLayoutOptions::default(),
        );
        list.push_clip(GRect::from_edges(
            area.x0 as f32,
            area.y0 as f32,
            area.x1 as f32,
            area.y1 as f32,
        ));
        if layers.backgrounds && a != b && e.composition.is_none() && self.focus == Some(id) {
            let (x0, x1) = (area.x0 as f32 + lay.x_at(a), area.x0 as f32 + lay.x_at(b));
            list.fill_rect(
                GRect::from_edges(x0, base - m.ascent, x1, base + m.descent),
                self.color(Token::Selection),
            );
        }
        if layers.text {
            list.text(
                &mut self.assets,
                self.font,
                &lay,
                px,
                ink,
                Decorations::default(),
            );
            if let Some(c) = &e.composition {
                let (x0, x1) = (
                    area.x0 as f32 + lay.x_at(b),
                    area.x0 as f32 + lay.x_at(b + c.text.len()),
                );
                list.text_decoration(
                    &self.assets,
                    self.font,
                    px,
                    x0,
                    x1,
                    base,
                    Decorations {
                        underline: true,
                        strikethrough: false,
                    },
                    ink,
                );
            }
            let caret_at = e.caret
                + e.composition
                    .as_ref()
                    .map_or(0, |c| c.cursor.map_or(c.text.len(), |cu| cu.1));
            let visible =
                self.focus == Some(id) && e.caret_visible(self.now, self.settings.caret_blink);
            if visible {
                let x = (area.x0 as f32 + lay.x_at(caret_at)).round();
                let w = (self.settings.caret_width as f32 * self.scale.0)
                    .round()
                    .max(1.0);
                list.fill_rect(
                    GRect::from_edges(x, base - m.ascent, x + w, base + m.descent),
                    self.color(Token::Caret),
                );
            }
        }
        list.pop_clip();
    }

    fn paint_popup(&mut self, list: &mut DrawList, id: NodeId, layers: PaintLayers) {
        if !layers.backgrounds {
            return;
        }
        let Some(node) = self.tree.get(id).cloned() else {
            return;
        };
        let ar = self.tree.abs_rect(id);
        match &node.widget {
            Widget::Dropdown {
                items, open: true, ..
            }
            | Widget::Combo {
                items, open: true, ..
            } => {
                let sel = selected_of(&node.widget);
                let panel = Rect::new(
                    ar.x,
                    ar.bottom(),
                    ar.w,
                    items.len() as f32 * MENU_ROW_H + MENU_PAD * 2.0,
                );
                let rows: Vec<(&str, bool, bool, bool, bool)> = items
                    .iter()
                    .enumerate()
                    .map(|(i, s)| (s.as_str(), Some(i) == sel, false, false, false))
                    .collect();
                self.paint_panel(list, panel, &rows);
            }
            Widget::Menu(m) if m.open => {
                let mut x = ar.x;
                let mut y = ar.y;
                let mut level = 0;
                while let Some(items) = m.level_items(level) {
                    let panel = Rect::new(x, y, ar.w, MenuState::panel_height(items));
                    let hot = m.path.get(level).copied();
                    let rows: Vec<(&str, bool, bool, bool, bool)> = items
                        .iter()
                        .enumerate()
                        .map(|(i, it)| {
                            (
                                if it.separator { "-" } else { it.label.as_str() },
                                hot == Some(i),
                                it.checked,
                                !it.submenu.is_empty(),
                                !it.enabled,
                            )
                        })
                        .collect();
                    self.paint_panel(list, panel, &rows);
                    let Some(&sel) = m.path.get(level) else { break };
                    y += MenuState::row_top(items, sel);
                    x += ar.w;
                    level += 1;
                }
            }
            _ => {}
        }
    }

    /// A popup panel with shadow, border and rows `(label, hot, checked, submenu, disabled)`;
    /// the label `-` is a separator.
    fn paint_panel(
        &mut self,
        list: &mut DrawList,
        panel: Rect,
        rows: &[(&str, bool, bool, bool, bool)],
    ) {
        let r = self.dev(panel);
        let s = self.scale.0;
        list.shadow(
            r.to_gfx(),
            2.0 * s,
            4.0 * s,
            [0.0, 2.0 * s],
            self.color(Token::Shadow),
        );
        list.fill_rect(r.to_gfx(), self.color(Token::MenuBg));
        list.rect_outline(r.to_gfx(), 1.0, self.color(Token::MenuBorder));
        let mut y = panel.y + MENU_PAD;
        for (label, hot, checked, sub, disabled) in rows {
            if *label == "-" {
                // 1 px line, one device pixel below the separator's top.
                let line = self.dev(Rect::new(panel.x, y + 0.8, panel.w, 0.8));
                list.fill_rect(line.to_gfx(), self.color(Token::Separator));
                y += MENU_SEP_H;
                continue;
            }
            let row = self.dev(Rect::new(
                panel.x + MENU_PAD,
                y,
                panel.w - MENU_PAD * 2.0,
                MENU_ROW_H,
            ));
            if *hot && !*disabled {
                list.fill_rect(row.to_gfx(), self.color(Token::MenuHot));
            }
            let ink = if *disabled {
                self.color(Token::TextDisabled)
            } else {
                self.color(Token::Text)
            };
            let text_r = DeviceRect {
                x0: row.x0 + self.scale.edge(MENU_TEXT_X),
                ..row
            };
            self.put_text(list, label, text_r, 0.0, ink, Decorations::default());
            if *checked {
                let mut p = Path::new();
                let (cx, cy) = (
                    (row.x0 + self.scale.edge(14.0)) as f32,
                    (row.y0 + row.y1) as f32 / 2.0,
                );
                p.move_to(cx - 4.0 * s, cy)
                    .line_to(cx - 1.0 * s, cy + 3.0 * s)
                    .line_to(cx + 4.0 * s, cy - 3.0 * s);
                list.stroke_path(
                    &mut self.assets,
                    &p,
                    1.5 * s,
                    open365_gfx::Join::Miter,
                    open365_gfx::Cap::Butt,
                    ink,
                );
            }
            if *sub {
                let mut p = Path::new();
                let (cx, cy) = (row.x1 as f32 - 12.0 * s, (row.y0 + row.y1) as f32 / 2.0);
                p.move_to(cx - 1.5 * s, cy - 3.5 * s)
                    .line_to(cx + 1.5 * s, cy)
                    .line_to(cx - 1.5 * s, cy + 3.5 * s);
                list.stroke_path(
                    &mut self.assets,
                    &p,
                    s.max(1.0),
                    open365_gfx::Join::Miter,
                    open365_gfx::Cap::Butt,
                    ink,
                );
            }
            y += MENU_ROW_H;
        }
    }

    fn paint_keytips(&mut self, list: &mut DrawList) {
        if !self.keytips.is_showing() {
            return;
        }
        for (keys, node, tab) in self.visible_tips() {
            let ar = self.tree.abs_rect(node);
            let anchor = match (tab, self.tree.get(node).map(|n| n.widget.clone())) {
                (Some(i), Some(Widget::TabStrip { tabs, gap, .. })) => {
                    tab_rects(crate::geom::Point::new(ar.x, ar.y), ar.h, &tabs, gap)
                        .get(i)
                        .copied()
                        .unwrap_or(ar)
                }
                _ => ar,
            };
            let w = self.measure(&keys).ceil() / self.scale.0 + 8.0;
            let badge = Rect::new(
                anchor.x + (anchor.w - w) / 2.0,
                anchor.bottom() - 10.0,
                w,
                18.0,
            );
            let d = self.dev(badge);
            list.fill_rect(d.to_gfx(), self.color(Token::KeytipBg));
            list.rect_outline(d.to_gfx(), 1.0, self.color(Token::KeytipBorder));
            let ink = self.color(Token::KeytipText);
            self.put_text(list, &keys, d, 0.5, ink, Decorations::default());
        }
    }

    fn paint_tooltip(&mut self, list: &mut DrawList) {
        let Some(t) = self.tooltip.clone() else {
            return;
        };
        let w = self.measure(&t.text).ceil() / self.scale.0 + 16.0;
        let r = self.dev(Rect::new(t.at.x + 12.0, t.at.y + 20.0, w, 26.0));
        let s = self.scale.0;
        list.shadow(
            r.to_gfx(),
            2.0 * s,
            3.0 * s,
            [0.0, 2.0 * s],
            self.color(Token::Shadow),
        );
        list.fill_rect(r.to_gfx(), self.color(Token::TooltipBg));
        list.rect_outline(r.to_gfx(), 1.0, self.color(Token::TooltipBorder));
        let ink = self.color(Token::TooltipText);
        self.put_text(list, &t.text, r, 0.5, ink, Decorations::default());
    }
}

fn selected_of(w: &Widget) -> Option<usize> {
    match w {
        Widget::Dropdown { selected, .. } => Some(*selected),
        Widget::Combo { edit, items, .. } => items.iter().position(|i| *i == edit.text),
        _ => None,
    }
}

trait ContainsPoint {
    fn contains_point(&self, p: (f32, f32)) -> bool;
}

impl ContainsPoint for GRect {
    fn contains_point(&self, p: (f32, f32)) -> bool {
        p.0 >= self.x0 && p.0 < self.x1 && p.1 >= self.y0 && p.1 < self.y1
    }
}
