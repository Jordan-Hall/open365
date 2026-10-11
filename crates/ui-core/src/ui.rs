//! [`Ui`]: the tree plus focus, pointer state, timers, key tips, IME and the clipboard, and the
//! routing of input events to widgets.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use open365_gfx::{Assets, FontId};

use crate::clipboard::{Clipboard, MemoryClipboard};
use crate::geom::{Point, Rect, Scale};
use crate::input::{Emit, ImeInput, InputEvent, Key, Modifiers, PointerButton};
use crate::keytips::{KeyTips, Tip, TipOutcome};
use crate::settings::PlatformSettings;
use crate::textedit::{Composition, TextEdit};
use crate::theme::Theme;
use crate::tree::{
    DROPDOWN_ARROW_W, MENU_PAD, MENU_ROW_H, MenuItem, MenuState, Node, NodeId, SPINNER_BUTTON_W,
    Tree, Widget,
};

/// How long the pointer rests on a node before its tooltip shows.
pub const TOOLTIP_DELAY: Duration = Duration::from_millis(500);
/// Two presses within this time and 4 px are a double click.
pub const CLICK_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimerKind {
    Tooltip(NodeId),
    Blink,
}

#[derive(Debug, Clone, Copy)]
struct Timer {
    at: Instant,
    kind: TimerKind,
}

/// A tooltip currently shown.
#[derive(Debug, Clone, PartialEq)]
pub struct TooltipState {
    pub target: NodeId,
    pub text: String,
    /// Where the pointer was when the tip appeared.
    pub at: Point,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum TipTarget {
    Tab(NodeId, usize),
    Node(NodeId),
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    node: NodeId,
    /// Offset of the grab inside the thumb, along the track.
    grab: f32,
}

/// The UI of one window.
pub struct Ui {
    pub tree: Tree,
    pub theme: Theme,
    pub settings: PlatformSettings,
    pub scale: Scale,
    pub assets: Assets,
    pub font: FontId,
    /// Text size in logical units (9 pt Segoe UI is 12).
    pub text_size: f32,
    pub keytips: KeyTips,
    pub(crate) focus: Option<NodeId>,
    pub(crate) focus_visible: bool,
    pub(crate) hover: Option<NodeId>,
    pub(crate) pressed: Option<NodeId>,
    capture: Option<NodeId>,
    drag: Option<Drag>,
    pub(crate) pointer: Point,
    timers: Vec<Timer>,
    events: Vec<Emit>,
    clipboard: Box<dyn Clipboard>,
    pub(crate) now: Instant,
    pub(crate) tooltip: Option<TooltipState>,
    pub(crate) ime_enabled: bool,
    window_focused: bool,
    tip_children: HashMap<String, Vec<Tip>>,
    tip_targets: HashMap<String, TipTarget>,
    tip_nodes: HashMap<String, NodeId>,
    pub(crate) redraw: bool,
    last_click: Option<(Instant, Point, u8)>,
    shift_down: bool,
}

impl std::fmt::Debug for Ui {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ui")
            .field("scale", &self.scale)
            .field("focus", &self.focus)
            .field("hover", &self.hover)
            .finish_non_exhaustive()
    }
}

fn editable(w: &Widget) -> Option<&TextEdit> {
    match w {
        Widget::TextField(e) | Widget::Combo { edit: e, .. } | Widget::Spinner { edit: e, .. } => {
            Some(e)
        }
        _ => None,
    }
}

fn editable_mut(w: &mut Widget) -> Option<&mut TextEdit> {
    match w {
        Widget::TextField(e) | Widget::Combo { edit: e, .. } | Widget::Spinner { edit: e, .. } => {
            Some(e)
        }
        _ => None,
    }
}

impl Ui {
    pub fn new(bounds: Rect, scale: f32, assets: Assets, font: FontId, now: Instant) -> Self {
        let settings = PlatformSettings::read();
        Self {
            tree: Tree::new(bounds),
            theme: Theme::for_settings(&settings),
            settings,
            scale: Scale(scale),
            assets,
            font,
            text_size: 12.0,
            keytips: KeyTips::default(),
            focus: None,
            focus_visible: false,
            hover: None,
            pressed: None,
            capture: None,
            drag: None,
            pointer: Point::default(),
            timers: Vec::new(),
            events: Vec::new(),
            clipboard: Box::new(MemoryClipboard::default()),
            now,
            tooltip: None,
            ime_enabled: false,
            window_focused: true,
            tip_children: HashMap::new(),
            tip_targets: HashMap::new(),
            tip_nodes: HashMap::new(),
            redraw: true,
            last_click: None,
            shift_down: false,
        }
    }

    /// Replaces the OS settings (and the theme that follows them).
    pub fn set_settings(&mut self, s: PlatformSettings) {
        self.theme = Theme::for_settings(&s);
        self.settings = s;
        self.redraw = true;
    }

    pub fn set_clipboard(&mut self, c: Box<dyn Clipboard>) {
        self.clipboard = c;
    }

    /// The last pointer position in logical units.
    pub fn pointer(&self) -> Point {
        self.pointer
    }

    pub fn focus(&self) -> Option<NodeId> {
        self.focus
    }

    pub fn hover(&self) -> Option<NodeId> {
        self.hover
    }

    pub fn pressed(&self) -> Option<NodeId> {
        self.pressed
    }

    pub fn captured(&self) -> Option<NodeId> {
        self.capture
    }

    pub fn tooltip(&self) -> Option<&TooltipState> {
        self.tooltip.as_ref()
    }

    /// Takes the events emitted since the last call.
    pub fn take_events(&mut self) -> Vec<Emit> {
        std::mem::take(&mut self.events)
    }

    /// True once since the last call when a frame is needed.
    pub fn take_redraw(&mut self) -> bool {
        std::mem::take(&mut self.redraw)
    }

    /// The earliest time something needs a [`InputEvent::Tick`].
    pub fn next_wake(&self) -> Option<Instant> {
        self.timers.iter().map(|t| t.at).min()
    }

    /// Key tips of the command level below `target`: `tab:<i>` for the key-tipped tab `i` of a
    /// tab strip (only one strip should carry tips).
    pub fn set_tip_children(&mut self, target: &str, tips: Vec<Tip>) {
        self.tip_children.insert(target.to_owned(), tips);
    }

    /// Nodes that the command tips below a tab target (`target` as given to
    /// [`Ui::set_tip_children`]), so they can show their badge and be activated.
    pub fn set_tip_nodes(&mut self, nodes: HashMap<String, NodeId>) {
        self.tip_nodes = nodes;
        self.redraw = true;
    }

    pub fn add(&mut self, parent: NodeId, node: Node) -> NodeId {
        self.redraw = true;
        self.tree.add(parent, node)
    }

    /// Moves keyboard focus (no-op for nodes that cannot take it).
    pub fn set_focus(&mut self, id: Option<NodeId>) {
        if let Some(i) = id
            && !self
                .tree
                .get(i)
                .is_some_and(|n| n.widget.focusable() && n.enabled)
        {
            return;
        }
        if self.focus != id {
            self.focus = id;
            self.touch_focus();
            self.redraw = true;
        }
    }

    fn touch_focus(&mut self) {
        let now = self.now;
        let Some(f) = self.focus else {
            self.ime_enabled = false;
            return;
        };
        let blink = self.settings.caret_blink;
        let mut text = false;
        if let Some(e) = self
            .tree
            .get_mut(f)
            .and_then(|n| editable_mut(&mut n.widget))
        {
            e.last_activity = Some(now);
            text = true;
        }
        self.ime_enabled = text
            || self
                .tree
                .get(f)
                .is_some_and(|n| matches!(n.widget, Widget::Canvas { .. }));
        if text && let Some(b) = blink {
            self.schedule(now + b, TimerKind::Blink);
        }
    }

    fn schedule(&mut self, at: Instant, kind: TimerKind) {
        self.timers.retain(|t| t.kind != kind);
        self.timers.push(Timer { at, kind });
    }

    fn emit(&mut self, e: Emit) {
        self.events.push(e);
        self.redraw = true;
    }

    /// Feeds one input event. Returns true when the picture changed.
    pub fn handle(&mut self, ev: InputEvent) -> bool {
        self.redraw = false;
        match ev {
            InputEvent::Tick(now) => self.tick(now),
            InputEvent::PointerMoved(p) => self.pointer_moved(p),
            InputEvent::PointerLeft => {
                self.set_hover(None);
            }
            InputEvent::PointerDown(PointerButton::Primary, p) => self.pointer_down(p),
            InputEvent::PointerUp(PointerButton::Primary, p) => self.pointer_up(p),
            InputEvent::PointerDown(..) | InputEvent::PointerUp(..) => {}
            InputEvent::Wheel { delta_y } => self.wheel(delta_y),
            InputEvent::Key {
                key,
                pressed,
                modifiers,
            } => self.key(key, pressed, modifiers),
            InputEvent::Text(s) => self.text(&s),
            InputEvent::Ime(i) => self.ime(i),
            InputEvent::FocusChanged(f) => {
                self.window_focused = f;
                if !f {
                    self.keytips.cancel();
                    self.capture = None;
                    self.pressed = None;
                    self.close_popups(None);
                }
                self.redraw = true;
            }
        }
        self.redraw
    }

    // ---- timers -------------------------------------------------------------------------

    fn tick(&mut self, now: Instant) {
        self.now = now;
        let due: Vec<Timer> = self
            .timers
            .iter()
            .copied()
            .filter(|t| t.at <= now)
            .collect();
        self.timers.retain(|t| t.at > now);
        for t in due {
            match t.kind {
                TimerKind::Tooltip(n) => {
                    if self.hover == Some(n)
                        && let Some(text) = self.tree.get(n).and_then(|x| x.tooltip.clone())
                    {
                        self.tooltip = Some(TooltipState {
                            target: n,
                            text,
                            at: self.pointer,
                        });
                        self.redraw = true;
                    }
                }
                TimerKind::Blink => {
                    if self.window_focused
                        && let (Some(f), Some(b)) = (self.focus, self.settings.caret_blink)
                        && self
                            .tree
                            .get(f)
                            .is_some_and(|n| editable(&n.widget).is_some())
                    {
                        self.redraw = true;
                        self.schedule(now + b, TimerKind::Blink);
                    }
                }
            }
        }
    }

    // ---- pointer ------------------------------------------------------------------------

    fn set_hover(&mut self, h: Option<NodeId>) {
        if self.hover != h {
            self.hover = h;
            self.tooltip = None;
            self.timers
                .retain(|t| !matches!(t.kind, TimerKind::Tooltip(_)));
            if let Some(n) = h
                && self.tree.get(n).is_some_and(|x| x.tooltip.is_some())
            {
                self.schedule(self.now + TOOLTIP_DELAY, TimerKind::Tooltip(n));
            }
            self.redraw = true;
        }
    }

    fn pointer_moved(&mut self, p: Point) {
        self.pointer = p;
        if let Some(d) = self.drag {
            self.drag_scroll(d, p);
            return;
        }
        if let Some(c) = self.capture {
            // Text selection by dragging keeps extending while captured.
            self.drag_text(c, p);
            return;
        }
        let hit = self.tree.hit_test(p);
        self.set_hover(hit);
        if let Some(h) = hit {
            self.menu_hover(h, p);
            if self.tooltip.is_some() && self.tooltip.as_ref().map(|t| t.target) != Some(h) {
                self.tooltip = None;
            }
        }
    }

    fn pointer_down(&mut self, p: Point) {
        self.pointer = p;
        self.keytips.cancel();
        self.tooltip = None;
        let hit = self.tree.hit_test(p);
        self.close_popups(hit);
        let Some(h) = hit else {
            self.pressed = None;
            self.capture = None;
            return;
        };
        self.hover = Some(h);
        self.pressed = Some(h);
        self.capture = Some(h);
        self.redraw = true;
        if !self.tree.get(h).is_some_and(|n| n.enabled) {
            return;
        }
        if self
            .tree
            .get(h)
            .is_some_and(|n| n.widget.focusable() && !matches!(n.widget, Widget::Menu(_)))
        {
            self.focus_visible = false;
            self.set_focus(Some(h));
        }
        let rect = self.tree.abs_rect(h);
        let Some(node) = self.tree.get(h) else { return };
        match node.widget.clone() {
            Widget::TextField(_) | Widget::Combo { .. } | Widget::Spinner { .. } => {
                let in_field = match &node.widget {
                    Widget::Combo { .. } => p.x < rect.right() - DROPDOWN_ARROW_W,
                    Widget::Spinner { .. } => p.x < rect.right() - SPINNER_BUTTON_W,
                    _ => true,
                };
                if in_field {
                    let byte = self.byte_at(h, p.x - rect.x - 4.0);
                    let now = self.now;
                    if let Some(e) = self
                        .tree
                        .get_mut(h)
                        .and_then(|n| editable_mut(&mut n.widget))
                    {
                        e.set_caret(byte, false);
                        e.last_activity = Some(now);
                    }
                }
            }
            Widget::Slider { .. } => {
                self.drag = Some(Drag { node: h, grab: 0.0 });
                self.set_slider_at(h, p.x);
            }
            Widget::Canvas { .. } => {
                let now = self.now;
                let clicks = match self.last_click {
                    Some((t, q, n))
                        if now.saturating_duration_since(t) < CLICK_INTERVAL
                            && (q.x - p.x).abs() <= 4.0
                            && (q.y - p.y).abs() <= 4.0 =>
                    {
                        n.saturating_add(1)
                    }
                    _ => 1,
                };
                self.last_click = Some((now, p, clicks));
                let shift = self.shift_down;
                self.emit(Emit::CanvasPointer {
                    node: h,
                    at: Point::new(p.x - rect.x, p.y - rect.y),
                    clicks,
                    shift,
                });
            }
            Widget::ScrollBar(s) => {
                let (track, pos) = if s.vertical {
                    (rect.h, p.y - rect.y)
                } else {
                    (rect.w, p.x - rect.x)
                };
                let (off, len) = s.thumb(track);
                if pos >= off && pos <= off + len {
                    self.drag = Some(Drag {
                        node: h,
                        grab: pos - off,
                    });
                } else {
                    let dir = if pos < off { -1.0 } else { 1.0 };
                    self.scroll_by(h, dir * s.page);
                }
            }
            _ => {}
        }
    }

    fn pointer_up(&mut self, p: Point) {
        self.pointer = p;
        self.drag = None;
        let pressed = self.pressed.take();
        self.capture = None;
        self.redraw = true;
        let hit = self.tree.hit_test(p);
        if let (Some(a), Some(b)) = (pressed, hit)
            && a == b
            && self.tree.get(a).is_some_and(|n| n.enabled)
        {
            self.click(a, p);
        }
        self.set_hover(hit);
    }

    /// Activates a node as a pointer click at `p`.
    fn click(&mut self, id: NodeId, p: Point) {
        let rect = self.tree.abs_rect(id);
        let Some(node) = self.tree.get(id) else {
            return;
        };
        match node.widget.clone() {
            Widget::Button { .. } | Widget::Checkbox { .. } | Widget::Radio { .. } => {
                self.activate(id)
            }
            Widget::SplitButton { arrow_w } => {
                let in_arrow = if node.look.arrow_below {
                    p.y >= rect.bottom() - arrow_w
                } else {
                    p.x >= rect.right() - arrow_w
                };
                if in_arrow {
                    self.emit(Emit::MenuRequested(id));
                } else {
                    self.emit(Emit::Clicked(id));
                }
            }
            Widget::Dropdown { items, open, .. } => {
                if open && p.y >= rect.bottom() {
                    if let Some(i) = self.list_row(&rect, p, items.len()) {
                        self.select_item(id, i);
                    }
                    self.set_open(id, false);
                } else {
                    self.set_open(id, !open);
                }
            }
            Widget::Combo { items, open, .. } => {
                if open && p.y >= rect.bottom() {
                    if let Some(i) = self.list_row(&rect, p, items.len()) {
                        self.select_item(id, i);
                    }
                    self.set_open(id, false);
                } else if p.x >= rect.right() - DROPDOWN_ARROW_W {
                    self.set_open(id, !open);
                }
            }
            Widget::Menu(m) => self.menu_click(id, &m, p),
            Widget::TabStrip {
                tabs,
                gap,
                selected,
            } => {
                let rects = crate::tree::tab_rects(Point::new(rect.x, rect.y), rect.h, &tabs, gap);
                if let Some(i) = rects.iter().position(|r| r.contains(p))
                    && tabs[i].enabled
                    && i != selected
                {
                    self.select_tab(id, i);
                }
            }
            Widget::Spinner { .. } => {
                if p.x >= rect.right() - SPINNER_BUTTON_W {
                    let dir = if p.y < rect.y + rect.h / 2.0 {
                        1.0
                    } else {
                        -1.0
                    };
                    self.spin(id, dir);
                }
            }
            _ => {}
        }
    }

    fn list_row(&self, rect: &Rect, p: Point, n: usize) -> Option<usize> {
        let y = p.y - rect.bottom() - MENU_PAD;
        (y >= 0.0)
            .then(|| (y / MENU_ROW_H) as usize)
            .filter(|i| *i < n)
    }

    /// Toggles, checks or presses a button-like widget and reports it.
    fn activate(&mut self, id: NodeId) {
        let Some(node) = self.tree.get_mut(id) else {
            return;
        };
        match &mut node.widget {
            Widget::Button { toggled: Some(t) } => {
                *t = !*t;
                let v = *t;
                self.emit(Emit::Toggled(id, v));
            }
            Widget::Button { toggled: None } => self.emit(Emit::Clicked(id)),
            Widget::Checkbox { checked } => {
                *checked = !*checked;
                let v = *checked;
                self.emit(Emit::Toggled(id, v));
            }
            Widget::Radio { checked, group } => {
                if !*checked {
                    *checked = true;
                    let g = *group;
                    self.emit(Emit::Toggled(id, true));
                    let others: Vec<NodeId> = self
                        .tree
                        .paint_order()
                        .into_iter()
                        .filter(|o| *o != id)
                        .filter(|o| matches!(self.tree.get(*o).map(|n| &n.widget), Some(Widget::Radio { group, .. }) if *group == g))
                        .collect();
                    for o in others {
                        if let Some(Widget::Radio { checked, .. }) =
                            self.tree.get_mut(o).map(|n| &mut n.widget)
                            && *checked
                        {
                            *checked = false;
                            self.emit(Emit::Toggled(o, false));
                        }
                    }
                }
            }
            Widget::SplitButton { .. } => self.emit(Emit::Clicked(id)),
            _ => {}
        }
    }

    fn select_item(&mut self, id: NodeId, i: usize) {
        let mut text = None;
        if let Some(n) = self.tree.get_mut(id) {
            match &mut n.widget {
                Widget::Dropdown {
                    items, selected, ..
                } if i < items.len() => *selected = i,
                Widget::Combo { items, edit, .. } if i < items.len() => {
                    let t = items[i].clone();
                    edit.text.clone_from(&t);
                    edit.select_all();
                    edit.caret = t.len();
                    edit.anchor = t.len();
                    text = Some(t);
                }
                _ => return,
            }
        }
        self.emit(Emit::Selected(id, i));
        if let Some(t) = text {
            self.emit(Emit::TextChanged(id, t));
        }
    }

    fn set_open(&mut self, id: NodeId, open: bool) {
        if let Some(n) = self.tree.get_mut(id) {
            match &mut n.widget {
                Widget::Dropdown { open: o, .. } | Widget::Combo { open: o, .. } => *o = open,
                _ => {}
            }
        }
        self.redraw = true;
    }

    fn close_popups(&mut self, keep: Option<NodeId>) {
        let ids: Vec<NodeId> = self.tree.paint_order();
        for id in ids {
            if Some(id) == keep {
                continue;
            }
            if let Some(n) = self.tree.get_mut(id) {
                match &mut n.widget {
                    Widget::Dropdown { open, .. } | Widget::Combo { open, .. } => {
                        if *open {
                            *open = false;
                            self.redraw = true;
                        }
                    }
                    Widget::Menu(m) => {
                        if m.open {
                            m.open = false;
                            m.path.clear();
                            self.redraw = true;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Opens a menu node at an absolute logical position.
    pub fn open_menu(&mut self, id: NodeId, at: Point) {
        self.close_popups(Some(id));
        if let Some(n) = self.tree.get_mut(id) {
            let parent_origin = n.parent;
            if let Widget::Menu(m) = &mut n.widget {
                m.open = true;
                m.path = vec![];
                n.visible = true;
                n.bounds.x = at.x;
                n.bounds.y = at.y;
                let _ = parent_origin;
            }
        }
        // The menu sits in the root's coordinate space, so absolute equals relative there.
        self.focus = Some(id);
        self.redraw = true;
    }

    fn menu_hover(&mut self, id: NodeId, p: Point) {
        let rect = self.tree.abs_rect(id);
        let Some(Widget::Menu(m)) = self.tree.get(id).map(|n| &n.widget) else {
            return;
        };
        if !m.open {
            return;
        }
        let Some((level, row)) = Self::menu_row_at(m, &rect, p) else {
            return;
        };
        let mut path: Vec<usize> = m.path.iter().take(level).copied().collect();
        let items = m.level_items(level).unwrap_or(&[]);
        if items.get(row).is_some_and(|i| !i.separator && i.enabled) {
            path.push(row);
            if let Some(Widget::Menu(m)) = self.tree.get_mut(id).map(|n| &mut n.widget)
                && m.path != path
            {
                m.path = path;
                self.redraw = true;
            }
        }
    }

    fn menu_row_at(m: &MenuState, rect: &Rect, p: Point) -> Option<(usize, usize)> {
        let mut x = rect.x;
        let mut y = rect.y;
        let mut level = 0;
        let mut found = None;
        while let Some(items) = m.level_items(level) {
            let panel = Rect::new(x, y, rect.w, MenuState::panel_height(items));
            if panel.contains(p) {
                found = MenuState::row_at(items, p.y - y).map(|r| (level, r));
            }
            let Some(&sel) = m.path.get(level) else { break };
            y += MenuState::row_top(items, sel);
            x += rect.w;
            level += 1;
        }
        found
    }

    fn menu_click(&mut self, id: NodeId, m: &MenuState, p: Point) {
        let rect = self.tree.abs_rect(id);
        let Some((level, row)) = Self::menu_row_at(m, &rect, p) else {
            return;
        };
        let Some(item) = m.level_items(level).and_then(|i| i.get(row)).cloned() else {
            return;
        };
        if item.separator || !item.enabled {
            return;
        }
        if !item.submenu.is_empty() {
            let mut path: Vec<usize> = m.path.iter().take(level).copied().collect();
            path.push(row);
            if let Some(Widget::Menu(mm)) = self.tree.get_mut(id).map(|n| &mut n.widget) {
                mm.path = path;
            }
            return;
        }
        self.close_popups(None);
        self.emit(Emit::MenuCommand(id, item.id));
    }

    /// Moves a slider's value to the pointer at logical `x` and reports a change.
    fn set_slider_at(&mut self, id: NodeId, x: f32) {
        let rect = self.tree.abs_rect(id);
        let mut out = None;
        if let Some(Widget::Slider {
            value,
            min,
            max,
            step,
        }) = self.tree.get_mut(id).map(|n| &mut n.widget)
        {
            let v = Widget::slider_value(*min, *max, *step, rect.x, rect.w, x);
            if (v - *value).abs() > f32::EPSILON {
                *value = v;
                out = Some(v);
            }
        }
        if let Some(v) = out {
            self.emit(Emit::ValueChanged(id, f64::from(v)));
        }
    }

    fn step_slider(&mut self, id: NodeId, dir: f32) {
        let mut out = None;
        if let Some(Widget::Slider {
            value,
            min,
            max,
            step,
        }) = self.tree.get_mut(id).map(|n| &mut n.widget)
        {
            let v = (*value + dir * step.max(f32::EPSILON)).clamp(*min, *max);
            if (v - *value).abs() > f32::EPSILON {
                *value = v;
                out = Some(v);
            }
        }
        if let Some(v) = out {
            self.emit(Emit::ValueChanged(id, f64::from(v)));
        }
    }

    fn drag_scroll(&mut self, d: Drag, p: Point) {
        if matches!(
            self.tree.get(d.node).map(|n| &n.widget),
            Some(Widget::Slider { .. })
        ) {
            self.set_slider_at(d.node, p.x);
            return;
        }
        let rect = self.tree.abs_rect(d.node);
        let Some(Widget::ScrollBar(s)) = self.tree.get(d.node).map(|n| n.widget.clone()) else {
            return;
        };
        let (track, pos) = if s.vertical {
            (rect.h, p.y - rect.y)
        } else {
            (rect.w, p.x - rect.x)
        };
        let (_, len) = s.thumb(track);
        let room = (track - len).max(1.0);
        let v = ((pos - d.grab) / room).clamp(0.0, 1.0) * s.max;
        self.set_scroll(d.node, v);
    }

    fn set_scroll(&mut self, id: NodeId, v: f32) {
        let mut out = None;
        if let Some(Widget::ScrollBar(s)) = self.tree.get_mut(id).map(|n| &mut n.widget)
            && s.set(v)
        {
            out = Some(s.value);
        }
        if let Some(v) = out {
            self.emit(Emit::ScrollChanged(id, v));
        }
    }

    fn scroll_by(&mut self, id: NodeId, delta: f32) {
        if let Some(Widget::ScrollBar(s)) = self.tree.get(id).map(|n| n.widget.clone()) {
            self.set_scroll(id, s.value + delta);
        }
    }

    fn wheel(&mut self, delta_y: f32) {
        if let Some(h) = self.hover
            && let Some(Widget::ScrollBar(s)) = self.tree.get(h).map(|n| n.widget.clone())
        {
            // One notch (positive up) scrolls three lines.
            self.scroll_by(h, -delta_y * s.line * 3.0);
        }
    }

    fn byte_at(&self, id: NodeId, x: f32) -> usize {
        let Some(e) = self.tree.get(id).and_then(|n| editable(&n.widget)) else {
            return 0;
        };
        let lay = self.assets.layout_text(
            self.font,
            &e.text,
            self.text_size * self.scale.0,
            0.0,
            0.0,
            open365_gfx::TextLayoutOptions::default(),
        );
        lay.byte_at(x * self.scale.0)
    }

    fn drag_text(&mut self, id: NodeId, p: Point) {
        let rect = self.tree.abs_rect(id);
        if self
            .tree
            .get(id)
            .and_then(|n| editable(&n.widget))
            .is_some()
        {
            let byte = self.byte_at(id, p.x - rect.x - 4.0);
            if let Some(e) = self
                .tree
                .get_mut(id)
                .and_then(|n| editable_mut(&mut n.widget))
            {
                e.set_caret(byte, true);
                self.redraw = true;
            }
        }
    }

    // ---- keyboard ------------------------------------------------------------------------

    fn root_tips(&mut self) -> Vec<Tip> {
        self.tip_targets.clear();
        let mut tips = Vec::new();
        for id in self.tree.paint_order() {
            let Some(n) = self.tree.get(id) else { continue };
            if let Some(k) = &n.key_tip {
                let target = format!("node:{}", tips.len());
                self.tip_targets.insert(target.clone(), TipTarget::Node(id));
                tips.push(Tip {
                    keys: k.to_ascii_uppercase(),
                    target,
                });
            }
            if let Widget::TabStrip { tabs, .. } = &n.widget {
                for (i, t) in tabs.iter().enumerate() {
                    if let (Some(k), true) = (&t.key_tip, t.enabled) {
                        let target = format!("tab:{i}");
                        self.tip_targets
                            .insert(target.clone(), TipTarget::Tab(id, i));
                        tips.push(Tip {
                            keys: k.to_ascii_uppercase(),
                            target,
                        });
                    }
                }
            }
        }
        tips
    }

    /// The tips now shown, with the node (and tab index) each belongs to.
    pub fn visible_tips(&self) -> Vec<(String, NodeId, Option<usize>)> {
        self.keytips
            .visible()
            .filter_map(|t| match self.tip_targets.get(&t.target) {
                Some(TipTarget::Node(n)) => Some((t.keys.clone(), *n, None)),
                Some(TipTarget::Tab(n, i)) => Some((t.keys.clone(), *n, Some(*i))),
                None => self
                    .tip_nodes
                    .get(&t.target)
                    .filter(|n| self.tree.contains(**n))
                    .map(|n| (t.keys.clone(), *n, None)),
            })
            .collect()
    }

    fn apply_tip(&mut self, out: TipOutcome) {
        match out {
            TipOutcome::None => {}
            TipOutcome::Changed => self.redraw = true,
            TipOutcome::Activate(target) => {
                self.redraw = true;
                match self.tip_targets.get(&target).copied() {
                    Some(TipTarget::Tab(n, i)) => self.select_tab(n, i),
                    Some(TipTarget::Node(n)) => {
                        self.activate(n);
                        self.emit(Emit::KeyTip(n));
                    }
                    None => self.emit_command_tip(&target),
                }
            }
        }
    }

    /// A tip below a tab was completed: run its node like a click, or open it.
    fn emit_command_tip(&mut self, target: &str) {
        let Some(n) = self.tip_nodes.get(target).copied() else {
            return;
        };
        let widget = self.tree.get(n).map(|x| x.widget.clone());
        match widget {
            Some(Widget::Dropdown { open, .. } | Widget::Combo { open, .. }) => {
                self.set_focus(Some(n));
                self.set_open(n, !open);
            }
            Some(Widget::TextField(_)) => self.set_focus(Some(n)),
            Some(Widget::SplitButton { .. }) => self.emit(Emit::Clicked(n)),
            _ => self.activate(n),
        }
        self.emit(Emit::KeyTip(n));
    }

    fn select_tab(&mut self, id: NodeId, i: usize) {
        if let Some(Widget::TabStrip { selected, tabs, .. }) =
            self.tree.get_mut(id).map(|n| &mut n.widget)
            && i < tabs.len()
        {
            *selected = i;
            self.emit(Emit::TabSelected(id, i));
        }
    }

    fn key(&mut self, key: Key, pressed: bool, m: Modifiers) {
        if key == Key::Shift {
            self.shift_down = pressed;
            return;
        }
        // Alt tips.
        if key == Key::Alt {
            if pressed {
                let out = self.keytips.alt_pressed(false);
                self.apply_tip(out);
            } else {
                let tips = self.root_tips();
                let out = self.keytips.alt_released(tips);
                self.apply_tip(out);
            }
            return;
        }
        if !pressed {
            return;
        }
        if m.alt {
            self.keytips.other_key_with_alt();
        }
        if self.keytips.is_showing() {
            self.tip_key(key);
            return;
        }
        if m.alt
            && let Key::Char(c) = key
            && self.access_key(c)
        {
            return;
        }
        if let Some(f) = self.focus
            && self.tree.get(f).is_some_and(|n| {
                n.enabled
                    && match &n.widget {
                        Widget::Canvas { .. } => !matches!(key, Key::Char(_)) || m.ctrl,
                        _ => n.intercept.contains(&key),
                    }
            })
        {
            self.redraw = true;
            self.emit(Emit::KeyPressed(f, key, m));
            return;
        }
        if key == Key::Tab {
            self.focus_move(m.shift);
            return;
        }
        let Some(f) = self.focus else { return };
        if !self.tree.get(f).is_some_and(|n| n.enabled) {
            return;
        }
        self.focus_visible = true;
        self.redraw = true;
        let widget = self.tree.get(f).map(|n| n.widget.clone());
        match widget {
            Some(Widget::Menu(_)) => self.menu_key(f, key),
            Some(Widget::Button { .. } | Widget::Checkbox { .. }) => {
                if matches!(key, Key::Space | Key::Enter) {
                    self.activate(f);
                }
            }
            Some(Widget::Radio { group, .. }) => match key {
                Key::Space | Key::Enter => self.activate(f),
                Key::Left | Key::Up => self.radio_step(f, group, -1),
                Key::Right | Key::Down => self.radio_step(f, group, 1),
                _ => {}
            },
            Some(Widget::SplitButton { .. }) => match key {
                Key::Space | Key::Enter => self.activate(f),
                Key::Down if m.alt => self.emit(Emit::MenuRequested(f)),
                _ => {}
            },
            Some(Widget::Dropdown {
                items,
                selected,
                open,
            }) => match key {
                Key::Down if m.alt => self.set_open(f, !open),
                Key::Space | Key::F10 => self.set_open(f, !open),
                Key::Enter | Key::Escape => self.set_open(f, false),
                Key::Down if selected + 1 < items.len() => self.select_item(f, selected + 1),
                Key::Up if selected > 0 => self.select_item(f, selected - 1),
                Key::Home if !items.is_empty() => self.select_item(f, 0),
                Key::End if !items.is_empty() => self.select_item(f, items.len() - 1),
                _ => {}
            },
            Some(Widget::Combo { items, open, edit }) => match key {
                Key::Down if m.alt => self.set_open(f, !open),
                Key::Escape => self.set_open(f, false),
                Key::Enter => {
                    self.set_open(f, false);
                    let t = edit.text.clone();
                    self.emit(Emit::TextCommitted(f, t));
                }
                Key::Down | Key::Up if !items.is_empty() => {
                    let cur = items.iter().position(|s| *s == edit.text);
                    let next = match (key, cur) {
                        (Key::Down, Some(i)) => (i + 1).min(items.len() - 1),
                        (Key::Down, None) => 0,
                        (_, Some(i)) => i.saturating_sub(1),
                        (_, None) => 0,
                    };
                    self.select_item(f, next);
                }
                _ => self.edit_key(f, key, m),
            },
            Some(Widget::TextField(_)) => {
                if key == Key::Enter {
                    let t = self
                        .tree
                        .get(f)
                        .and_then(|n| editable(&n.widget))
                        .map(|e| e.text.clone());
                    if let Some(t) = t {
                        self.emit(Emit::TextCommitted(f, t));
                    }
                } else {
                    self.edit_key(f, key, m);
                }
            }
            Some(Widget::Spinner { .. }) => match key {
                Key::Up => self.spin(f, 1.0),
                Key::Down => self.spin(f, -1.0),
                Key::Enter => self.commit_spinner(f),
                _ => self.edit_key(f, key, m),
            },
            Some(Widget::TabStrip { tabs, selected, .. }) => {
                let enabled: Vec<usize> = (0..tabs.len()).filter(|i| tabs[*i].enabled).collect();
                let pos = enabled.iter().position(|i| *i == selected).unwrap_or(0);
                let to = match key {
                    Key::Left if pos > 0 => Some(enabled[pos - 1]),
                    Key::Right if pos + 1 < enabled.len() => Some(enabled[pos + 1]),
                    Key::Home => enabled.first().copied(),
                    Key::End => enabled.last().copied(),
                    _ => None,
                };
                if let Some(i) = to
                    && i != selected
                {
                    self.select_tab(f, i);
                }
            }
            Some(Widget::Slider { .. }) => match key {
                Key::Left | Key::Down => self.step_slider(f, -1.0),
                Key::Right | Key::Up => self.step_slider(f, 1.0),
                _ => {}
            },
            Some(Widget::ScrollBar(s)) => {
                let (small, page) = (s.line, s.page);
                match key {
                    Key::Up | Key::Left => self.scroll_by(f, -small),
                    Key::Down | Key::Right => self.scroll_by(f, small),
                    Key::PageUp => self.scroll_by(f, -page),
                    Key::PageDown => self.scroll_by(f, page),
                    Key::Home => self.set_scroll(f, 0.0),
                    Key::End => self.set_scroll(f, s.max),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn tip_key(&mut self, key: Key) {
        let children = self.tip_children.clone();
        let lookup = move |t: &str| children.get(t).cloned().unwrap_or_default();
        let out = match key {
            Key::Escape => self.keytips.escape(),
            Key::Char(c) if c.is_ascii_alphanumeric() => self.keytips.letter(c, &lookup),
            _ => self.keytips.cancel(),
        };
        if let Some(target) = self.keytips.take_descended()
            && let Some(TipTarget::Tab(n, i)) = self.tip_targets.get(&target).copied()
        {
            self.select_tab(n, i);
        }
        self.apply_tip(out);
    }

    fn access_key(&mut self, c: char) -> bool {
        let c = c.to_ascii_lowercase();
        let hit = self.tree.paint_order().into_iter().find(|id| {
            self.tree.get(*id).is_some_and(|n| {
                n.enabled && n.access_key.is_some_and(|k| k.to_ascii_lowercase() == c)
            })
        });
        let Some(id) = hit else { return false };
        if self
            .tree
            .get(id)
            .is_some_and(|n| editable(&n.widget).is_some())
        {
            self.set_focus(Some(id));
        } else {
            self.activate(id);
            self.set_focus(Some(id));
        }
        true
    }

    fn focus_move(&mut self, back: bool) {
        let order = self.tree.tab_order();
        if order.is_empty() {
            return;
        }
        let cur = self.focus.and_then(|f| order.iter().position(|o| *o == f));
        let next = match (cur, back) {
            (None, false) => 0,
            (None, true) => order.len() - 1,
            (Some(i), false) => (i + 1) % order.len(),
            (Some(i), true) => (i + order.len() - 1) % order.len(),
        };
        self.focus_visible = true;
        self.focus = None;
        self.set_focus(Some(order[next]));
        self.redraw = true;
    }

    fn radio_step(&mut self, id: NodeId, group: u32, dir: i32) {
        let members: Vec<NodeId> = self
            .tree
            .paint_order()
            .into_iter()
            .filter(|o| matches!(self.tree.get(*o).map(|n| (&n.widget, n.enabled)), Some((Widget::Radio { group: g, .. }, true)) if *g == group))
            .collect();
        if let Some(i) = members.iter().position(|m| *m == id) {
            let next = (i as i32 + dir).rem_euclid(members.len() as i32) as usize;
            self.set_focus(Some(members[next]));
            self.activate(members[next]);
        }
    }

    fn menu_key(&mut self, id: NodeId, key: Key) {
        let Some(Widget::Menu(m)) = self.tree.get(id).map(|n| n.widget.clone()) else {
            return;
        };
        if !m.open {
            return;
        }
        let mut path = m.path.clone();
        let level = path.len().saturating_sub(1);
        let depth_items = |p: &[usize]| {
            let mut mm = m.clone();
            mm.path = p.to_vec();
            mm
        };
        let cur_items = depth_items(&path)
            .level_items(level)
            .map(<[MenuItem]>::to_vec)
            .unwrap_or_else(|| m.items.clone());
        let step = |from: Option<usize>, dir: i32| -> Option<usize> {
            let n = cur_items.len() as i32;
            let mut i = from.map_or(if dir > 0 { -1 } else { n }, |v| v as i32);
            for _ in 0..n {
                i = (i + dir).rem_euclid(n);
                if !cur_items[i as usize].separator && cur_items[i as usize].enabled {
                    return Some(i as usize);
                }
            }
            None
        };
        match key {
            Key::Down | Key::Up => {
                let dir = if key == Key::Down { 1 } else { -1 };
                let from = if path.is_empty() {
                    None
                } else {
                    path.last().copied()
                };
                if let Some(i) = step(from, dir) {
                    if path.is_empty() {
                        path.push(i);
                    } else if let Some(l) = path.last_mut() {
                        *l = i;
                    }
                }
            }
            Key::Right => {
                if let Some(&sel) = path.last()
                    && cur_items.get(sel).is_some_and(|i| !i.submenu.is_empty())
                {
                    path.push(0);
                }
            }
            Key::Left => {
                if path.len() > 1 {
                    path.pop();
                }
            }
            Key::Escape => {
                if path.len() > 1 {
                    path.pop();
                } else {
                    self.close_popups(None);
                    return;
                }
            }
            Key::Enter | Key::Space => {
                if let Some(&sel) = path.last()
                    && let Some(item) = cur_items.get(sel)
                {
                    if item.submenu.is_empty() {
                        let id_str = item.id.clone();
                        self.close_popups(None);
                        self.emit(Emit::MenuCommand(id, id_str));
                        return;
                    }
                    path.push(0);
                }
            }
            Key::Char(c) => {
                let c = c.to_ascii_lowercase();
                if let Some(i) = cur_items.iter().position(|it| {
                    it.access.is_some_and(|a| a.to_ascii_lowercase() == c) && it.enabled
                }) {
                    if cur_items[i].submenu.is_empty() {
                        let id_str = cur_items[i].id.clone();
                        self.close_popups(None);
                        self.emit(Emit::MenuCommand(id, id_str));
                        return;
                    }
                    if let Some(l) = path.last_mut() {
                        *l = i;
                    } else {
                        path.push(i);
                    }
                    path.push(0);
                }
            }
            _ => {}
        }
        if let Some(Widget::Menu(mm)) = self.tree.get_mut(id).map(|n| &mut n.widget) {
            mm.path = path;
        }
        self.redraw = true;
    }

    fn spin(&mut self, id: NodeId, dir: f64) {
        let mut out = None;
        let now = self.now;
        if let Some(Widget::Spinner {
            value,
            min,
            max,
            step,
            edit,
        }) = self.tree.get_mut(id).map(|n| &mut n.widget)
        {
            let parsed = edit.text.trim().parse::<f64>().unwrap_or(*value);
            let v = (parsed + dir * *step).clamp(*min, *max);
            *value = v;
            edit.text = format_value(v, *step);
            edit.set_caret(edit.text.len(), false);
            edit.last_activity = Some(now);
            out = Some(v);
        }
        if let Some(v) = out {
            self.emit(Emit::ValueChanged(id, v));
        }
    }

    fn commit_spinner(&mut self, id: NodeId) {
        let mut out = None;
        if let Some(Widget::Spinner {
            value,
            min,
            max,
            step,
            edit,
        }) = self.tree.get_mut(id).map(|n| &mut n.widget)
        {
            let v = edit
                .text
                .trim()
                .parse::<f64>()
                .unwrap_or(*value)
                .clamp(*min, *max);
            *value = v;
            edit.text = format_value(v, *step);
            out = Some(v);
        }
        if let Some(v) = out {
            self.emit(Emit::ValueChanged(id, v));
        }
    }

    fn edit_key(&mut self, id: NodeId, key: Key, m: Modifiers) {
        let now = self.now;
        if m.ctrl {
            match key {
                Key::Char('C') | Key::Char('X') => {
                    let sel = self
                        .tree
                        .get(id)
                        .and_then(|n| editable(&n.widget))
                        .map(|e| e.selected_text().to_owned());
                    if let Some(s) = sel.filter(|s| !s.is_empty()) {
                        self.clipboard.set_text(&s);
                        if key == Key::Char('X') {
                            self.edit_apply(id, |e| {
                                e.delete_selection();
                                true
                            });
                        }
                    }
                    return;
                }
                Key::Char('V') => {
                    if let Some(t) = self.clipboard.get_text() {
                        let t: String = t.chars().filter(|c| !c.is_control()).collect();
                        self.edit_apply(id, |e| {
                            e.insert(&t);
                            true
                        });
                    }
                    return;
                }
                _ => {}
            }
        }
        let (shift, ctrl) = (m.shift, m.ctrl);
        self.edit_apply(id, |e| {
            e.last_activity = Some(now);
            e.key(key, shift, ctrl)
        });
    }

    fn edit_apply(&mut self, id: NodeId, f: impl FnOnce(&mut TextEdit) -> bool) {
        let now = self.now;
        let mut changed = None;
        if let Some(e) = self
            .tree
            .get_mut(id)
            .and_then(|n| editable_mut(&mut n.widget))
        {
            let was = e.text.clone();
            f(e);
            e.last_activity = Some(now);
            if e.text != was {
                changed = Some(e.text.clone());
            }
        }
        self.redraw = true;
        if let Some(t) = changed {
            self.emit(Emit::TextChanged(id, t));
        }
    }

    /// Performs an action a screen reader asked for on a widget: focus, click (the same path as a
    /// pointer click at the widget's centre), set a text value (then committed as Enter would),
    /// and increment or decrement a spinner or slider. Returns true when the request named a
    /// live, enabled widget and an action it supports.
    pub fn accessibility_action(&mut self, request: &accesskit::ActionRequest) -> bool {
        use accesskit::{Action, ActionData};
        let Some(id) = self
            .tree
            .paint_order()
            .into_iter()
            .find(|i| crate::access::ak_id(*i) == request.target_node)
        else {
            return false;
        };
        let Some(node) = self.tree.get(id) else {
            return false;
        };
        if !node.enabled {
            return false;
        }
        let widget = node.widget.clone();
        self.redraw = true;
        match request.action {
            Action::Focus => {
                self.set_focus(Some(id));
                true
            }
            Action::Click => {
                if widget.focusable() {
                    self.set_focus(Some(id));
                }
                let c = self.tree.abs_rect(id).center();
                self.click(id, c);
                true
            }
            Action::SetValue => {
                let Some(ActionData::Value(v)) = &request.data else {
                    return false;
                };
                if editable(&widget).is_none() {
                    return false;
                }
                let text = v.to_string();
                self.set_focus(Some(id));
                self.edit_apply(id, |e| {
                    *e = TextEdit::new(&text);
                    true
                });
                self.emit(Emit::TextCommitted(id, text));
                true
            }
            Action::Increment | Action::Decrement => {
                let dir = if request.action == Action::Increment {
                    1.0
                } else {
                    -1.0
                };
                match widget {
                    Widget::Spinner { .. } => self.spin(id, dir),
                    Widget::Slider { .. } => self.step_slider(id, dir as f32),
                    _ => return false,
                }
                true
            }
            _ => false,
        }
    }

    fn text(&mut self, s: &str) {
        let Some(f) = self.focus else { return };
        if self
            .tree
            .get(f)
            .is_some_and(|n| matches!(n.widget, Widget::Canvas { .. }))
        {
            let clean: String = s.chars().filter(|c| !c.is_control()).collect();
            if !clean.is_empty() {
                self.emit(Emit::CanvasText(f, clean));
            }
            return;
        }
        let clean: String = s.chars().filter(|c| !c.is_control()).collect();
        if clean.is_empty() || self.tree.get(f).and_then(|n| editable(&n.widget)).is_none() {
            return;
        }
        self.edit_apply(f, |e| {
            e.composition = None;
            e.insert(&clean);
            true
        });
    }

    fn ime(&mut self, i: ImeInput) {
        let Some(f) = self.focus else { return };
        if self
            .tree
            .get(f)
            .is_some_and(|n| matches!(n.widget, Widget::Canvas { .. }))
        {
            match i {
                ImeInput::Enabled => self.ime_enabled = true,
                ImeInput::Disabled => self.ime_enabled = false,
                other => self.emit(Emit::CanvasIme(f, other)),
            }
            return;
        }
        match i {
            ImeInput::Enabled => self.ime_enabled = true,
            ImeInput::Disabled => {
                self.ime_enabled = false;
                self.edit_apply(f, |e| {
                    e.composition = None;
                    false
                });
            }
            ImeInput::Preedit { text, cursor } => self.edit_apply(f, |e| {
                e.composition = if text.is_empty() {
                    None
                } else {
                    Some(Composition { text, cursor })
                };
                false
            }),
            ImeInput::Commit(t) => self.edit_apply(f, |e| {
                e.composition = None;
                e.insert(&t);
                true
            }),
        }
    }
}

fn format_value(v: f64, step: f64) -> String {
    if step.fract() == 0.0 && v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_values_format_without_noise() {
        assert_eq!(format_value(11.0, 1.0), "11");
        assert_eq!(format_value(10.5, 0.5), "10.5");
        assert_eq!(format_value(2.0, 0.5), "2");
    }
}
