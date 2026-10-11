//! The node arena, widget data, layout and hit testing.

use crate::geom::{Point, Rect, Scale};
use crate::textedit::TextEdit;

/// Generational handle into the arena; stale handles resolve to nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId {
    index: u32,
    generation: u32,
}

/// How a container places its children.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Layout {
    /// Children keep the `bounds` they were given (relative to the container).
    #[default]
    Absolute,
    Row {
        gap: f32,
        padding: f32,
    },
    Column {
        gap: f32,
        padding: f32,
    },
}

/// A size along one axis for children of a stack.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Extent {
    Fixed(f32),
    /// A share of the room left after fixed sizes and gaps.
    Flex(f32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub checked: bool,
    pub separator: bool,
    /// Access key letter shown underlined in the label.
    pub access: Option<char>,
    pub submenu: Vec<MenuItem>,
    /// Icon drawn in the cell left of the label.
    pub icon: Option<open365_gfx::IconId>,
    /// A group heading: not selectable, drawn as a caption above its group.
    pub heading: bool,
    /// A second, dimmer line under the label (a 60 u row).
    pub detail: Option<String>,
}

impl MenuItem {
    pub fn new(id: &str, label: &str) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            enabled: true,
            checked: false,
            separator: false,
            access: None,
            submenu: Vec::new(),
            icon: None,
            heading: false,
            detail: None,
        }
    }

    /// A group heading: Office puts one above each group of a gallery or menu.
    pub fn heading(label: &str) -> Self {
        Self {
            heading: true,
            enabled: false,
            ..Self::new("", label)
        }
    }

    pub fn with_detail(mut self, detail: &str) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Height of this row when it follows `prev` (`None` at the top).
    pub fn height_after(&self, prev: Option<&MenuItem>) -> f32 {
        if self.separator {
            MENU_SEP_H
        } else if self.heading {
            // 37 u at the top or after a separator, 38 u after an item.
            match prev {
                Some(p) if !p.separator => MENU_HEADING_H + 0.8,
                _ => MENU_HEADING_H,
            }
        } else if self.detail.is_some() {
            MENU_DETAIL_H
        } else {
            MENU_ROW_H
        }
    }

    pub fn separator() -> Self {
        Self {
            separator: true,
            ..Self::new("", "")
        }
    }

    pub fn with_icon(mut self, icon: Option<open365_gfx::IconId>) -> Self {
        self.icon = icon;
        self
    }

    pub fn with_submenu(mut self, items: Vec<MenuItem>) -> Self {
        self.submenu = items;
        self
    }
}

/// Menu geometry in logical units, measured on Excel's own menus at 125 % (device px / 1.25):
/// rows 36 px, a group separator 3 px (a 1 px line one pixel below the group top), 2 px of border
/// and inset on every side, text 44 px from the row edge (after a 20 px icon cell).
pub const MENU_ROW_H: f32 = 28.8;
pub const MENU_SEP_H: f32 = 2.4;
/// Group heading (37 u) and an item with a description line (60 u).
pub const MENU_HEADING_H: f32 = 29.6;
pub const MENU_DETAIL_H: f32 = 48.0;
/// Rows of the combo lists Office measures: font names 33 u, font sizes 30 u (menu rows are
/// [`MENU_ROW_H`]).
pub const LIST_ROW_FONT_H: f32 = 26.4;
pub const LIST_ROW_SIZE_H: f32 = 24.0;
pub const MENU_PAD: f32 = 1.6;
/// Left edge of the label in a menu row, from the row's own left edge.
pub const MENU_TEXT_X: f32 = 35.2;

#[derive(Debug, Clone, PartialEq)]
pub struct MenuState {
    pub items: Vec<MenuItem>,
    /// Highlighted item index per open level; `[1, 0]` is item 0 of the submenu of item 1.
    pub path: Vec<usize>,
    pub open: bool,
}

impl MenuState {
    pub fn new(items: Vec<MenuItem>) -> Self {
        Self {
            items,
            path: Vec::new(),
            open: false,
        }
    }

    /// Items shown at `level` (0 = root) following the highlighted path.
    pub fn level_items(&self, level: usize) -> Option<&[MenuItem]> {
        if self.path.len() < level {
            return None;
        }
        let mut items = self.items.as_slice();
        for &i in self.path.iter().take(level) {
            items = &items.get(i)?.submenu;
            if items.is_empty() {
                return None;
            }
        }
        Some(items)
    }

    /// Row heights of `items`, top to bottom.
    pub fn heights(items: &[MenuItem]) -> impl Iterator<Item = f32> + '_ {
        items
            .iter()
            .enumerate()
            .map(|(i, it)| it.height_after(i.checked_sub(1).map(|p| &items[p])))
    }

    /// Height of a panel showing `items`.
    pub fn panel_height(items: &[MenuItem]) -> f32 {
        MENU_PAD * 2.0 + Self::heights(items).sum::<f32>()
    }

    /// Top of row `index` inside its panel.
    pub fn row_top(items: &[MenuItem], index: usize) -> f32 {
        MENU_PAD + Self::heights(items).take(index).sum::<f32>()
    }

    /// The row under panel-relative `y`.
    pub fn row_at(items: &[MenuItem], y: f32) -> Option<usize> {
        let mut top = MENU_PAD;
        for (i, h) in Self::heights(items).enumerate() {
            if y >= top && y < top + h {
                return Some(i);
            }
            top += h;
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollState {
    pub vertical: bool,
    pub value: f32,
    /// Largest value (content minus viewport).
    pub max: f32,
    /// Viewport length in content units.
    pub page: f32,
    pub line: f32,
}

impl ScrollState {
    pub fn new(vertical: bool, max: f32, page: f32) -> Self {
        Self {
            vertical,
            value: 0.0,
            max,
            page,
            line: 16.0,
        }
    }

    /// Thumb `(offset, length)` along a track of `track` logical units.
    pub fn thumb(&self, track: f32) -> (f32, f32) {
        let total = self.max + self.page;
        if total <= 0.0 || self.max <= 0.0 {
            return (0.0, track);
        }
        let len = (track * self.page / total).max(16.0).min(track);
        let off = (track - len) * (self.value / self.max).clamp(0.0, 1.0);
        (off, len)
    }

    pub fn set(&mut self, v: f32) -> bool {
        let n = v.clamp(0.0, self.max.max(0.0));
        let changed = (n - self.value).abs() > f32::EPSILON;
        self.value = n;
        changed
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabItem {
    pub label: String,
    pub width: f32,
    pub enabled: bool,
    /// Key tip after Alt (`H`, `JI`).
    pub key_tip: Option<String>,
}

pub const SPINNER_BUTTON_W: f32 = 18.0;
/// Width of a combo's arrow button: 16 px at 125 % (Office's "Open" button inside the combo).
pub const DROPDOWN_ARROW_W: f32 = 12.8;

#[derive(Debug, Clone, PartialEq)]
pub enum Widget {
    Panel,
    Label,
    /// A push button; `Some(state)` makes it a toggle button.
    Button {
        toggled: Option<bool>,
    },
    /// A button with a menu arrow of `arrow_w` on its right.
    SplitButton {
        arrow_w: f32,
    },
    Dropdown {
        items: Vec<String>,
        selected: usize,
        open: bool,
    },
    Combo {
        edit: TextEdit,
        items: Vec<String>,
        open: bool,
    },
    Menu(MenuState),
    ScrollBar(ScrollState),
    TabStrip {
        tabs: Vec<TabItem>,
        selected: usize,
        gap: f32,
    },
    Tooltip,
    TextField(TextEdit),
    Checkbox {
        checked: bool,
    },
    Radio {
        checked: bool,
        group: u32,
    },
    Spinner {
        value: f64,
        min: f64,
        max: f64,
        step: f64,
        edit: TextEdit,
    },
    /// A horizontal slider (zoom): a hairline track, a centre tick and a thumb.
    Slider {
        value: f32,
        min: f32,
        max: f32,
        step: f32,
    },
    /// An application-drawn surface (the sheet grid) that takes focus and hands raw keys, typed
    /// text, IME input and pointer presses to the application as `Emit::Canvas*` and
    /// `Emit::KeyPressed`. `caret` is the rectangle (relative to the node, logical units) the IME
    /// candidate window should sit beside.
    Canvas {
        caret: Rect,
    },
}

/// Slider thumb size in logical units (6 x 14 px at 125 %).
pub const SLIDER_THUMB_W: f32 = 4.8;
pub const SLIDER_THUMB_H: f32 = 11.2;

impl Widget {
    /// Slider value for a pointer at `x` over a track spanning `left..left + width`.
    pub fn slider_value(min: f32, max: f32, step: f32, left: f32, width: f32, x: f32) -> f32 {
        let room = (width - SLIDER_THUMB_W).max(1.0);
        let t = ((x - left - SLIDER_THUMB_W / 2.0) / room).clamp(0.0, 1.0);
        let raw = min + t * (max - min);
        if step > 0.0 {
            (min + ((raw - min) / step).round() * step).clamp(min, max)
        } else {
            raw
        }
    }
}

impl Widget {
    pub fn focusable(&self) -> bool {
        !matches!(self, Widget::Panel | Widget::Label | Widget::Tooltip)
    }
}

/// How a button-like node arranges its icon and label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Content {
    /// Icon beside the label when both exist, otherwise whichever exists.
    #[default]
    Auto,
    /// Large ribbon button: icon on top, label (up to two lines) below.
    IconAbove,
    IconBeside,
    IconOnly,
    TextOnly,
}

/// Row height of an open Dropdown or Combo list.
pub fn list_row_h(look: &Look) -> f32 {
    if look.list_row > 0.0 {
        look.list_row
    } else {
        MENU_ROW_H
    }
}

/// Appearance beyond what the widget kind implies. All fields are optional so existing code
/// keeps its look.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub icon: Option<open365_gfx::IconId>,
    pub content: Content,
    /// Icon edge in logical units.
    pub icon_px: f32,
    /// Distance of the icon from the top edge for [`Content::IconAbove`], logical units.
    pub icon_top: f32,
    /// A split button's arrow part sits below the face (its `arrow_w` is then a height).
    pub arrow_below: bool,
    /// Background and one pixel border, drawn before the widget.
    pub fill: Option<crate::theme::Token>,
    pub border: Option<crate::theme::Token>,
    /// Corner radius of `fill` and hover fills, logical units.
    pub radius: f32,
    /// Text colour token instead of the widget default.
    pub ink: Option<crate::theme::Token>,
    /// Text size in logical units instead of the UI size.
    pub text_size: Option<f32>,
    /// 0 left, 0.5 centre, 1 right (labels and text-only buttons).
    pub align: f32,
    /// No hover or press fill (display-only buttons).
    pub still: bool,
    /// Left and right padding of aligned text, logical units.
    pub pad: f32,
    /// Direct colours that win over the tokens (application palettes such as Word's page).
    pub fill_rgb: Option<open365_gfx::Color>,
    pub border_rgb: Option<open365_gfx::Color>,
    pub ink_rgb: Option<open365_gfx::Color>,
    pub hover_rgb: Option<open365_gfx::Color>,
    /// A bar along the bottom edge: colour and height in logical units (active sheet tab).
    pub bar: Option<(open365_gfx::Color, f32)>,
    /// A drop-down chevron after the label (menu buttons).
    pub chevron: bool,
    /// A label followed by a pill switch drawn in our own style (`Some(on)`), as the AutoSave
    /// switch of the title bar.
    pub switch: Option<bool>,
    /// Row height of the list a Dropdown or Combo opens, logical units; 0 is [`MENU_ROW_H`].
    pub list_row: f32,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            icon: None,
            content: Content::Auto,
            icon_px: 16.0,
            icon_top: 4.0,
            arrow_below: false,
            fill: None,
            border: None,
            radius: 0.0,
            ink: None,
            text_size: None,
            align: 0.5,
            still: false,
            pad: 4.0,
            fill_rgb: None,
            border_rgb: None,
            ink_rgb: None,
            hover_rgb: None,
            bar: None,
            chevron: false,
            switch: None,
            list_row: 0.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    /// Logical rectangle relative to the parent's origin.
    pub bounds: Rect,
    pub layout: Layout,
    pub width: Extent,
    pub height: Extent,
    pub widget: Widget,
    pub label: String,
    pub tooltip: Option<String>,
    pub enabled: bool,
    pub visible: bool,
    /// Alt+letter accelerator inside a dialog or menu (underlined in the label).
    pub access_key: Option<char>,
    /// Alt key tip shown on the ribbon (`H`, `FP`).
    pub key_tip: Option<String>,
    pub clip_children: bool,
    /// Keys the focused node hands to the application as `Emit::KeyPressed` instead of handling
    /// them itself (a cell editor gives Tab, Enter, Escape and the arrows to the grid).
    pub intercept: Vec<crate::input::Key>,
    pub look: Look,
}

impl Node {
    pub fn new(widget: Widget, bounds: Rect) -> Self {
        Self {
            parent: None,
            children: Vec::new(),
            bounds,
            layout: Layout::Absolute,
            width: Extent::Fixed(bounds.w),
            height: Extent::Fixed(bounds.h),
            widget,
            label: String::new(),
            tooltip: None,
            enabled: true,
            visible: true,
            access_key: None,
            key_tip: None,
            clip_children: false,
            intercept: Vec::new(),
            look: Look::default(),
        }
    }

    pub fn with_look(mut self, look: Look) -> Self {
        self.look = look;
        self
    }

    pub fn with_label(mut self, label: &str) -> Self {
        self.label = label.into();
        self
    }
}

#[derive(Debug)]
struct Slot {
    generation: u32,
    node: Option<Node>,
}

/// The arena. Index zero is always the root.
#[derive(Debug)]
pub struct Tree {
    slots: Vec<Slot>,
    free: Vec<u32>,
    root: NodeId,
}

impl NodeId {
    /// A stable 64 bit value for external ids (AccessKit).
    pub fn as_u64(self) -> u64 {
        (u64::from(self.generation) << 32) | u64::from(self.index)
    }
}

impl Tree {
    pub fn new(bounds: Rect) -> Self {
        let mut t = Self {
            slots: Vec::new(),
            free: Vec::new(),
            root: NodeId {
                index: 0,
                generation: 0,
            },
        };
        t.root = t.alloc(Node::new(Widget::Panel, bounds));
        t
    }

    pub fn root(&self) -> NodeId {
        self.root
    }

    fn alloc(&mut self, node: Node) -> NodeId {
        if let Some(i) = self.free.pop() {
            let s = &mut self.slots[i as usize];
            s.node = Some(node);
            NodeId {
                index: i,
                generation: s.generation,
            }
        } else {
            self.slots.push(Slot {
                generation: 0,
                node: Some(node),
            });
            NodeId {
                index: (self.slots.len() - 1) as u32,
                generation: 0,
            }
        }
    }

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        let s = self.slots.get(id.index as usize)?;
        if s.generation == id.generation {
            s.node.as_ref()
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        let s = self.slots.get_mut(id.index as usize)?;
        if s.generation == id.generation {
            s.node.as_mut()
        } else {
            None
        }
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.get(id).is_some()
    }

    /// Adds `node` as the last child of `parent` (the top of the z-order).
    pub fn add(&mut self, parent: NodeId, mut node: Node) -> NodeId {
        node.parent = Some(parent);
        let id = self.alloc(node);
        if let Some(p) = self.get_mut(parent) {
            p.children.push(id);
        }
        id
    }

    /// Removes a node and its subtree; the handles become stale.
    pub fn remove(&mut self, id: NodeId) {
        if id == self.root {
            return;
        }
        if let Some(p) = self.get(id).and_then(|n| n.parent)
            && let Some(pn) = self.get_mut(p)
        {
            pn.children.retain(|c| *c != id);
        }
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            let Some(s) = self.slots.get_mut(n.index as usize) else {
                continue;
            };
            if s.generation != n.generation {
                continue;
            }
            if let Some(node) = s.node.take() {
                stack.extend(node.children);
            }
            s.generation += 1;
            self.free.push(n.index);
        }
    }

    /// Nodes in paint order (parents before children, later siblings on top).
    pub fn paint_order(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![self.root];
        while let Some(id) = stack.pop() {
            let Some(n) = self.get(id) else { continue };
            if !n.visible {
                continue;
            }
            out.push(id);
            stack.extend(n.children.iter().rev().copied());
        }
        out
    }

    /// Absolute logical origin of a node.
    pub fn origin(&self, id: NodeId) -> Point {
        let mut p = Point::default();
        let mut cur = Some(id);
        while let Some(c) = cur {
            let Some(n) = self.get(c) else { break };
            p.x += n.bounds.x;
            p.y += n.bounds.y;
            cur = n.parent;
        }
        p
    }

    /// Absolute logical rectangle of a node.
    pub fn abs_rect(&self, id: NodeId) -> Rect {
        let o = self.origin(id);
        self.get(id).map_or(Rect::default(), |n| {
            Rect::new(o.x, o.y, n.bounds.w, n.bounds.h)
        })
    }

    /// True when the node and all ancestors are visible.
    pub fn shown(&self, id: NodeId) -> bool {
        let mut cur = Some(id);
        while let Some(c) = cur {
            let Some(n) = self.get(c) else { return false };
            if !n.visible {
                return false;
            }
            cur = n.parent;
        }
        true
    }

    /// The area in which pointer events reach a node: its rectangle, plus open menu panels and
    /// open drop-down lists.
    pub fn hit_rects(&self, id: NodeId) -> Vec<Rect> {
        let r = self.abs_rect(id);
        let Some(n) = self.get(id) else { return vec![] };
        let mut out = vec![r];
        match &n.widget {
            Widget::Menu(m) if m.open => {
                out.clear();
                let mut x = r.x;
                let mut y = r.y;
                let mut level = 0;
                while let Some(items) = m.level_items(level) {
                    out.push(Rect::new(x, y, r.w, MenuState::panel_height(items)));
                    let Some(&sel) = m.path.get(level) else { break };
                    y += MenuState::row_top(items, sel);
                    x += r.w;
                    level += 1;
                }
            }
            Widget::Dropdown {
                items, open: true, ..
            }
            | Widget::Combo {
                items, open: true, ..
            } => {
                out.push(Rect::new(
                    r.x,
                    r.bottom(),
                    r.w,
                    items.len() as f32 * list_row_h(&n.look) + MENU_PAD * 2.0,
                ));
            }
            _ => {}
        }
        out
    }

    /// The topmost visible node under `p`.
    pub fn hit_test(&self, p: Point) -> Option<NodeId> {
        let order = self.paint_order();
        for &id in order.iter().rev() {
            if id == self.root {
                continue;
            }
            let Some(n) = self.get(id) else { continue };
            if matches!(n.widget, Widget::Menu(MenuState { open: false, .. })) {
                continue;
            }
            if self.clipped_out(id, p) {
                continue;
            }
            if self.hit_rects(id).iter().any(|r| r.contains(p)) {
                return Some(id);
            }
        }
        None
    }

    /// The intersection of the rectangles of the clipping ancestors of `id`, if any.
    pub fn clip_rect(&self, id: NodeId) -> Option<Rect> {
        let mut out: Option<Rect> = None;
        let mut cur = self.get(id).and_then(|n| n.parent);
        while let Some(c) = cur {
            let n = self.get(c)?;
            if n.clip_children {
                let r = self.abs_rect(c);
                out = Some(out.map_or(r, |o| o.intersect(&r)));
            }
            cur = n.parent;
        }
        out
    }

    fn clipped_out(&self, id: NodeId, p: Point) -> bool {
        let mut cur = self.get(id).and_then(|n| n.parent);
        while let Some(c) = cur {
            let Some(n) = self.get(c) else { return false };
            if n.clip_children && !self.abs_rect(c).contains(p) {
                return true;
            }
            cur = n.parent;
        }
        false
    }

    /// Arranges the children of stack containers, top down. Absolute containers keep their
    /// children's bounds.
    pub fn layout(&mut self) {
        let mut stack = vec![self.root];
        while let Some(id) = stack.pop() {
            self.arrange(id);
            if let Some(n) = self.get(id) {
                stack.extend(n.children.iter().copied());
            }
        }
    }

    fn arrange(&mut self, id: NodeId) {
        let Some(n) = self.get(id) else { return };
        let (layout, bounds, kids) = (n.layout, n.bounds, n.children.clone());
        let (horizontal, gap, pad) = match layout {
            Layout::Absolute => return,
            Layout::Row { gap, padding } => (true, gap, padding),
            Layout::Column { gap, padding } => (false, gap, padding),
        };
        let visible: Vec<NodeId> = kids
            .into_iter()
            .filter(|k| self.get(*k).is_some_and(|n| n.visible))
            .collect();
        let main_total = if horizontal { bounds.w } else { bounds.h } - 2.0 * pad;
        let gaps = gap * visible.len().saturating_sub(1) as f32;
        let ext = |n: &Node| if horizontal { n.width } else { n.height };
        let fixed: f32 = visible
            .iter()
            .filter_map(|k| self.get(*k))
            .map(|n| {
                if let Extent::Fixed(v) = ext(n) {
                    v
                } else {
                    0.0
                }
            })
            .sum();
        let flex: f32 = visible
            .iter()
            .filter_map(|k| self.get(*k))
            .map(|n| if let Extent::Flex(f) = ext(n) { f } else { 0.0 })
            .sum();
        let room = (main_total - fixed - gaps).max(0.0);
        let mut pos = pad;
        for k in visible {
            let Some(n) = self.get_mut(k) else { continue };
            let main = match ext(n) {
                Extent::Fixed(v) => v,
                Extent::Flex(f) => {
                    if flex > 0.0 {
                        room * f / flex
                    } else {
                        0.0
                    }
                }
            };
            let cross_ext = if horizontal { n.height } else { n.width };
            let cross_total = if horizontal { bounds.h } else { bounds.w } - 2.0 * pad;
            let cross = match cross_ext {
                Extent::Fixed(v) => v,
                Extent::Flex(_) => cross_total,
            };
            n.bounds = if horizontal {
                Rect::new(pos, pad, main, cross)
            } else {
                Rect::new(pad, pos, cross, main)
            };
            pos += main + gap;
        }
    }

    /// Device rectangle of a node at `scale`, snapped by edges from absolute logical coordinates.
    pub fn device_rect(&self, id: NodeId, scale: Scale) -> crate::geom::DeviceRect {
        scale.snap(self.abs_rect(id))
    }

    /// Focusable, visible, enabled nodes in tab order (tree order).
    pub fn tab_order(&self) -> Vec<NodeId> {
        self.paint_order()
            .into_iter()
            .filter(|id| {
                self.get(*id).is_some_and(|n| {
                    n.widget.focusable() && n.enabled && !matches!(n.widget, Widget::Menu(_))
                })
            })
            .collect()
    }
}

impl TabItem {
    pub fn new(label: &str, width: f32) -> Self {
        Self {
            label: label.into(),
            width,
            enabled: true,
            key_tip: None,
        }
    }
}

/// Rectangles of the tabs of a strip whose origin is `origin`.
pub fn tab_rects(origin: Point, h: f32, tabs: &[TabItem], gap: f32) -> Vec<Rect> {
    let mut x = origin.x;
    tabs.iter()
        .map(|t| {
            let r = Rect::new(x, origin.y, t.width, h);
            x += t.width + gap;
            r
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button(x: f32, y: f32, w: f32, h: f32) -> Node {
        Node::new(Widget::Button { toggled: None }, Rect::new(x, y, w, h))
    }

    #[test]
    fn menu_row_heights_follow_the_measured_spec() {
        // 36 u item, 37 u heading at the top, 38 u after an item, 60 u with a description,
        // 3 u separator (125 %, logical = u / 1.25).
        let items = vec![
            MenuItem::heading("Group"),
            MenuItem::new("a", "A"),
            MenuItem::heading("Next"),
            MenuItem::new("b", "B").with_detail("Describes B"),
            MenuItem::separator(),
            MenuItem::heading("After rule"),
        ];
        let h: Vec<f32> = MenuState::heights(&items).collect();
        let u: Vec<f32> = h.iter().map(|v| (v * 1.25).round()).collect();
        assert_eq!(u, vec![37.0, 36.0, 38.0, 60.0, 3.0, 37.0]);
        assert!(
            (MenuState::panel_height(&items) - (MENU_PAD * 2.0 + h.iter().sum::<f32>())).abs()
                < 1e-4
        );
        assert_eq!(MenuState::row_at(&items, MENU_PAD + 1.0), Some(0));
        assert!(!items[0].enabled && items[0].heading);
        assert_eq!((LIST_ROW_FONT_H * 1.25).round(), 33.0);
        assert_eq!((LIST_ROW_SIZE_H * 1.25).round(), 30.0);
        assert_eq!((MENU_ROW_H * 1.25).round(), 36.0);
    }

    #[test]
    fn stale_handles_resolve_to_nothing() {
        let mut t = Tree::new(Rect::new(0.0, 0.0, 100.0, 100.0));
        let a = t.add(t.root(), button(0.0, 0.0, 10.0, 10.0));
        t.remove(a);
        assert!(t.get(a).is_none());
        let b = t.add(t.root(), button(0.0, 0.0, 10.0, 10.0));
        assert!(
            t.get(a).is_none(),
            "a reused slot must not revive the old handle"
        );
        assert!(t.get(b).is_some());
    }

    #[test]
    fn hit_test_returns_the_topmost_node() {
        let mut t = Tree::new(Rect::new(0.0, 0.0, 100.0, 100.0));
        let under = t.add(t.root(), button(0.0, 0.0, 50.0, 50.0));
        let over = t.add(t.root(), button(25.0, 25.0, 50.0, 50.0));
        assert_eq!(t.hit_test(Point::new(10.0, 10.0)), Some(under));
        assert_eq!(t.hit_test(Point::new(30.0, 30.0)), Some(over));
        assert_eq!(t.hit_test(Point::new(90.0, 90.0)), None);
    }

    #[test]
    fn children_use_parent_relative_bounds() {
        let mut t = Tree::new(Rect::new(0.0, 0.0, 200.0, 200.0));
        let panel = t.add(
            t.root(),
            Node::new(Widget::Panel, Rect::new(50.0, 40.0, 100.0, 100.0)),
        );
        let b = t.add(panel, button(10.0, 10.0, 20.0, 20.0));
        assert_eq!(t.abs_rect(b), Rect::new(60.0, 50.0, 20.0, 20.0));
        assert_eq!(t.hit_test(Point::new(65.0, 55.0)), Some(b));
    }

    #[test]
    fn row_layout_splits_flex_room() {
        let mut t = Tree::new(Rect::new(0.0, 0.0, 200.0, 30.0));
        t.get_mut(t.root()).expect("root").layout = Layout::Row {
            gap: 10.0,
            padding: 5.0,
        };
        let a = t.add(t.root(), button(0.0, 0.0, 40.0, 20.0));
        let mut flex = button(0.0, 0.0, 0.0, 20.0);
        flex.width = Extent::Flex(1.0);
        let b = t.add(t.root(), flex);
        t.layout();
        assert_eq!(t.abs_rect(a), Rect::new(5.0, 5.0, 40.0, 20.0));
        assert_eq!(t.abs_rect(b), Rect::new(55.0, 5.0, 140.0, 20.0));
    }

    #[test]
    fn invisible_nodes_are_not_hit_and_clip_applies() {
        let mut t = Tree::new(Rect::new(0.0, 0.0, 100.0, 100.0));
        let a = t.add(t.root(), button(0.0, 0.0, 50.0, 50.0));
        t.get_mut(a).expect("a").visible = false;
        assert_eq!(t.hit_test(Point::new(10.0, 10.0)), None);
        let panel = t.add(
            t.root(),
            Node::new(Widget::Panel, Rect::new(0.0, 0.0, 20.0, 20.0)),
        );
        t.get_mut(panel).expect("panel").clip_children = true;
        let _inside = t.add(panel, button(0.0, 0.0, 60.0, 60.0));
        assert!(t.hit_test(Point::new(10.0, 10.0)).is_some());
        assert_eq!(
            t.hit_test(Point::new(40.0, 40.0)),
            None,
            "clipped by the panel"
        );
    }

    #[test]
    fn menu_levels_follow_the_highlight_path() {
        let items = vec![
            MenuItem::new("a", "A"),
            MenuItem::new("b", "B")
                .with_submenu(vec![MenuItem::new("b1", "B1"), MenuItem::new("b2", "B2")]),
        ];
        let mut m = MenuState::new(items);
        assert_eq!(m.level_items(0).map(<[_]>::len), Some(2));
        assert!(m.level_items(1).is_none());
        m.path = vec![1];
        assert_eq!(m.level_items(1).map(<[_]>::len), Some(2));
        assert_eq!(
            MenuState::row_at(m.level_items(0).expect("root"), MENU_PAD + MENU_ROW_H + 1.0),
            Some(1)
        );
    }

    #[test]
    fn thumb_scales_with_the_page() {
        let s = ScrollState::new(true, 900.0, 100.0);
        let (off, len) = s.thumb(200.0);
        assert_eq!(off, 0.0);
        assert!((len - 20.0).abs() < 0.01);
        let mut s2 = s;
        s2.set(900.0);
        assert!((s2.thumb(200.0).0 - 180.0).abs() < 0.01);
    }
}
