//! The live dialog: widgets built from a [`Spec`], keyboard handling (Tab order, Enter, Esc, access
//! keys), a title bar that drags the dialog, list boxes, tab pages and the collapsible reference
//! field.

use std::collections::HashMap;

use open365_gfx::{Color, IconId};

use super::spec::{
    BUTTON_H, BUTTON_W, ButtonRole, FIELD_H, FOOTER_H, Kind, LIST_ROW_H, MARGIN, Spec, TAB_H,
    TITLE_H, Value, Values, split_access,
};
use crate::access::ak_id;
use crate::geom::{Point, Rect};
use crate::input::{Emit, Key};
use crate::textedit::TextEdit;
use crate::theme::Token;
use crate::tree::{Content, Look, Node, NodeId, ScrollState, TabItem, Widget};
use crate::ui::Ui;

/// Label prefix that marks the scrim of a dialog; the number is the dialog's serial.
const SCRIM_TAG: &str = "o365-dialog:";
/// Radio group numbers of dialogs start here so they never meet the ribbon's groups.
const GROUP_BASE: u32 = 0x7000_0000;
/// Width of a list box's scroll bar and of the reference button.
const BAR_W: f32 = 16.0;
const REF_BTN_W: f32 = 26.0;
/// Colour of the error line.
const ERROR_INK: Color = Color::rgba(196, 43, 28, 255);

/// Icons the dialog draws, supplied by the host application.
#[derive(Debug, Clone, Copy, Default)]
pub struct Icons {
    pub close: Option<IconId>,
    /// The reference field's collapse button.
    pub collapse: Option<IconId>,
}

/// What a handled event meant to the dialog's owner.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// A footer button (`ok`, `cancel`, `apply`, ...) or a button of the page was pressed. The
    /// title bar's close button and Esc press the footer's cancel button.
    Pressed(String),
    /// A control's value changed.
    Changed(String),
    /// A tab or wizard page was shown.
    Page(usize),
    /// A reference field collapsed (`true`) or expanded the dialog again.
    Ref { id: String, collapsed: bool },
}

/// The result of offering an event to the dialog.
#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    /// Not the dialog's event.
    Ignored,
    /// The dialog used the event.
    Consumed,
    Outcome(Outcome),
}

#[derive(Debug)]
struct Slot {
    main: NodeId,
    /// Other nodes of the control: caption lines, the reference button, list rows.
    extra: Vec<NodeId>,
}

#[derive(Debug)]
struct ListState {
    items: Vec<String>,
    selected: Option<usize>,
    top: usize,
    rows: Vec<NodeId>,
    bar: Option<NodeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    Control(usize),
    Row(usize, usize),
    Bar(usize),
    /// The invisible button behind a label's access key; focuses control `.0`.
    Proxy(usize),
    RefButton(usize),
    Footer(usize),
    Close,
    Strip,
    Title,
}

/// A modal dialog. Build it from a [`Spec`], [`Dialog::open`] it on a [`Ui`], feed it the
/// [`Emit`]s the `Ui` produces with [`Dialog::handle`] and read the answer with
/// [`Dialog::values`].
#[derive(Debug)]
pub struct Dialog {
    spec: Spec,
    serial: u64,
    icons: Icons,
    scrim: Option<NodeId>,
    panel: Option<NodeId>,
    title: Option<NodeId>,
    drag_bar: Option<NodeId>,
    close: Option<NodeId>,
    strip: Option<NodeId>,
    error: Option<NodeId>,
    slots: Vec<Slot>,
    footer: Vec<NodeId>,
    owners: HashMap<NodeId, Owner>,
    lists: HashMap<usize, ListState>,
    groups: HashMap<String, u32>,
    page: usize,
    /// Panel origin when the user dragged the dialog (or collapsed it from a dragged place).
    moved: Option<Point>,
    grab: Option<Point>,
    collapsed: Option<usize>,
    /// Where the user dragged the collapsed bar.
    collapsed_at: Option<Point>,
}

impl Dialog {
    /// A dialog that is not on screen yet. `serial` tells dialogs apart (see [`scrim_serial`]).
    pub fn new(spec: Spec, serial: u64) -> Self {
        Self {
            spec,
            serial,
            icons: Icons::default(),
            scrim: None,
            panel: None,
            title: None,
            drag_bar: None,
            close: None,
            strip: None,
            error: None,
            slots: Vec::new(),
            footer: Vec::new(),
            owners: HashMap::new(),
            lists: HashMap::new(),
            groups: HashMap::new(),
            page: 0,
            moved: None,
            grab: None,
            collapsed: None,
            collapsed_at: None,
        }
    }

    pub fn spec(&self) -> &Spec {
        &self.spec
    }

    pub fn serial(&self) -> u64 {
        self.serial
    }

    /// The panel node, `None` before [`Dialog::open`].
    pub fn panel(&self) -> Option<NodeId> {
        self.panel
    }

    pub fn page(&self) -> usize {
        self.page
    }

    /// True while the reference field has the dialog collapsed.
    pub fn is_collapsed(&self) -> bool {
        self.collapsed.is_some()
    }

    /// The node of control `id` (a list's panel, a reference field's text field).
    pub fn node(&self, id: &str) -> Option<NodeId> {
        self.index_of(id).map(|i| self.slots[i].main)
    }

    /// The node of footer button `id`.
    pub fn footer_node(&self, id: &str) -> Option<NodeId> {
        let j = self.spec.footer.iter().position(|b| b.id == id)?;
        self.footer.get(j).copied()
    }

    /// The title bar's close button.
    pub fn close_node(&self) -> Option<NodeId> {
        self.close
    }

    /// The strip of tabs.
    pub fn strip_node(&self) -> Option<NodeId> {
        self.strip
    }

    /// The bar that drags the dialog.
    pub fn title_bar(&self) -> Option<NodeId> {
        self.drag_bar
    }

    fn index_of(&self, id: &str) -> Option<usize> {
        if id.is_empty() {
            return None;
        }
        self.spec.controls.iter().position(|c| c.id == id)
    }

    // ---- building ----------------------------------------------------------------------------

    /// Puts the dialog on `ui`, on top of everything, and focuses its first control.
    pub fn open(&mut self, ui: &mut Ui, icons: Icons) {
        self.icons = icons;
        let root = ui.tree.root();
        let win = ui.tree.get(root).map_or(Rect::default(), |n| n.bounds);
        let scrim_look = Look {
            fill_rgb: Some(Color::rgba(0, 0, 0, 0)),
            still: true,
            ..Look::default()
        };
        let mut s = Node::new(Widget::Panel, Rect::new(0.0, 0.0, win.w, win.h))
            .with_look(scrim_look)
            .with_label(&format!("{SCRIM_TAG}{}", self.serial));
        s.clip_children = false;
        let scrim = ui.add(root, s);
        self.scrim = Some(scrim);
        ui.set_canvas_access(scrim, hidden_node(), Vec::new());

        let (w, h) = self.size();
        let panel = ui.add(
            scrim,
            Node::new(Widget::Panel, Rect::new(0.0, 0.0, w, h))
                .with_label(&self.spec.title)
                .with_look(Look {
                    fill: Some(Token::MenuBg),
                    border: Some(Token::MenuBorder),
                    still: true,
                    ..Look::default()
                }),
        );
        self.panel = Some(panel);

        // Title text under the drag bar, which sits under the close button.
        let title = ui.add(
            panel,
            Node::new(Widget::Label, Rect::new(0.0, 0.0, w - 46.0, TITLE_H))
                .with_label(&self.spec.title)
                .with_look(label_look(Token::Text, 12.0)),
        );
        self.title = Some(title);
        let bar = ui.add(
            panel,
            Node::new(
                Widget::Canvas {
                    caret: Rect::default(),
                },
                Rect::new(0.0, 0.0, w - 46.0, TITLE_H),
            )
            .with_label("Title bar"),
        );
        self.drag_bar = Some(bar);
        self.owners.insert(bar, Owner::Title);
        let mut close = Node::new(
            Widget::Button { toggled: None },
            Rect::new(w - 46.0, 0.0, 46.0, TITLE_H),
        )
        .with_label("Close")
        .with_look(Look {
            icon: self.icons.close,
            content: if self.icons.close.is_some() {
                Content::IconOnly
            } else {
                Content::TextOnly
            },
            icon_px: 12.0,
            ..Look::default()
        });
        close.intercept = vec![Key::Tab, Key::Escape];
        let close = ui.add(panel, close);
        self.close = Some(close);
        self.owners.insert(close, Owner::Close);

        if !self.spec.tabs.is_empty() {
            let tabs: Vec<TabItem> = self
                .spec
                .tabs
                .iter()
                .map(|t| {
                    let (text, _) = split_access(t);
                    TabItem::new(&text, 12.0 + text.chars().count() as f32 * 6.4)
                })
                .collect();
            let mut st = Node::new(
                Widget::TabStrip {
                    tabs,
                    selected: self
                        .spec
                        .start_page
                        .min(self.spec.tabs.len().saturating_sub(1)),
                    gap: 0.0,
                },
                Rect::new(MARGIN, TITLE_H + 4.0, self.spec.width - 2.0 * MARGIN, TAB_H),
            )
            .with_label("Tabs");
            st.intercept = vec![Key::Tab, Key::Escape, Key::Enter];
            let strip = ui.add(panel, st);
            self.strip = Some(strip);
            self.owners.insert(strip, Owner::Strip);
        }

        let groups = self.group_numbers();
        self.groups = groups;
        for i in 0..self.spec.controls.len() {
            let slot = self.build_control(ui, panel, i);
            self.slots.push(slot);
        }
        self.assign_access_keys(ui);

        for (j, b) in self.spec.footer.clone().iter().enumerate() {
            let (text, key) = split_access(&b.label);
            let default = b.role == ButtonRole::Default;
            let mut n = Node::new(
                Widget::Button { toggled: None },
                Rect::new(0.0, 0.0, BUTTON_W, BUTTON_H),
            )
            .with_label(&text)
            .with_look(Look {
                content: Content::TextOnly,
                fill: Some(if default { Token::Accent } else { Token::Field }),
                border: Some(if default {
                    Token::Accent
                } else {
                    Token::FieldBorder
                }),
                ink: default.then_some(Token::OnAccent),
                radius: 2.0,
                ..Look::default()
            });
            n.enabled = b.enabled;
            n.access_key = key;
            n.intercept = vec![Key::Tab, Key::Escape];
            let id = ui.add(panel, n);
            self.owners.insert(id, Owner::Footer(j));
            self.footer.push(id);
        }

        let err = ui.add(
            panel,
            Node::new(Widget::Label, Rect::new(MARGIN, 0.0, 10.0, 20.0)).with_look(Look {
                ink_rgb: Some(ERROR_INK),
                ..label_look(Token::Text, 0.0)
            }),
        );
        self.error = Some(err);

        self.page = self.spec.start_page;
        self.apply_layout(ui);
        self.apply_access(ui);
        self.refresh_pages(ui);
        let first = self
            .spec
            .focus
            .as_deref()
            .and_then(|id| self.focus_target(id))
            .or_else(|| self.tab_stops(ui).first().copied());
        if let Some(f) = first {
            self.focus_node(ui, f);
        }
    }

    /// Removes the dialog from the tree and leaves the focus to the caller.
    pub fn close(&mut self, ui: &mut Ui) {
        if let Some(s) = self.scrim.take() {
            ui.tree.remove(s);
        }
        self.panel = None;
        self.slots.clear();
        self.footer.clear();
        self.owners.clear();
        self.lists.clear();
        ui.set_focus(None);
    }

    pub fn is_open(&self, ui: &Ui) -> bool {
        self.scrim.is_some_and(|s| ui.tree.contains(s))
    }

    fn group_numbers(&self) -> HashMap<String, u32> {
        let mut out = HashMap::new();
        for c in &self.spec.controls {
            if let Kind::Radio { group, .. } = &c.kind {
                let n = out.len() as u32;
                out.entry(group.clone())
                    .or_insert(GROUP_BASE + (self.serial as u32 % 4096) * 64 + n);
            }
        }
        out
    }

    /// The size of the panel.
    fn size(&self) -> (f32, f32) {
        (self.spec.width, TITLE_H + self.spec.height + FOOTER_H)
    }

    fn build_control(&mut self, ui: &mut Ui, panel: NodeId, i: usize) -> Slot {
        let c = self.spec.controls[i].clone();
        let (text, key) = c.caption();
        let r = c.rect.translate(0.0, TITLE_H);
        let mut extra = Vec::new();
        let mk = |ui: &mut Ui, w: Widget, rect: Rect, label: &str, look: Look| {
            ui.add(panel, Node::new(w, rect).with_label(label).with_look(look))
        };
        let main = match &c.kind {
            Kind::Label => {
                let lines: Vec<&str> = text.split('\n').collect();
                let lh = if lines.len() > 1 {
                    (r.h / lines.len() as f32).clamp(14.0, 16.0)
                } else {
                    r.h
                };
                let first = mk(
                    ui,
                    Widget::Label,
                    Rect::new(r.x, r.y, r.w, lh),
                    lines[0],
                    label_look(Token::Text, 0.0),
                );
                for (k, line) in lines.iter().enumerate().skip(1) {
                    extra.push(mk(
                        ui,
                        Widget::Label,
                        Rect::new(r.x, r.y + k as f32 * lh, r.w, lh),
                        line,
                        label_look(Token::Text, 0.0),
                    ));
                }
                first
            }
            Kind::Group => {
                let frame = mk(
                    ui,
                    Widget::Panel,
                    Rect::new(r.x, r.y + 8.0, r.w, (r.h - 8.0).max(1.0)),
                    "",
                    Look {
                        border: Some(Token::Separator),
                        still: true,
                        ..Look::default()
                    },
                );
                let cap_w = text.chars().count() as f32 * 6.4 + 8.0;
                let cap = mk(
                    ui,
                    Widget::Label,
                    Rect::new(r.x + 8.0, r.y, cap_w, 16.0),
                    &text,
                    Look {
                        fill: Some(Token::MenuBg),
                        ..label_look(Token::Text, 3.0)
                    },
                );
                extra.push(cap);
                frame
            }
            Kind::Text(v) => mk(
                ui,
                Widget::TextField(TextEdit::new(v)),
                r,
                &text,
                Look::default(),
            ),
            Kind::Spin {
                value,
                min,
                max,
                step,
            } => mk(
                ui,
                Widget::Spinner {
                    value: *value,
                    min: *min,
                    max: *max,
                    step: *step,
                    edit: TextEdit::new(&format_number(*value, *step)),
                },
                r,
                &text,
                Look::default(),
            ),
            Kind::Check(on) => mk(
                ui,
                Widget::Checkbox { checked: *on },
                r,
                &text,
                Look::default(),
            ),
            Kind::Radio { group, checked } => {
                let g = self.groups.get(group).copied().unwrap_or(GROUP_BASE);
                mk(
                    ui,
                    Widget::Radio {
                        checked: *checked,
                        group: g,
                    },
                    r,
                    &text,
                    Look::default(),
                )
            }
            Kind::Dropdown { items, selected } => mk(
                ui,
                Widget::Dropdown {
                    items: items.clone(),
                    selected: (*selected).min(items.len().saturating_sub(1)),
                    open: false,
                },
                r,
                &text,
                Look {
                    list_row: LIST_ROW_H,
                    ..Look::default()
                },
            ),
            Kind::Combo { items, text: t } => mk(
                ui,
                Widget::Combo {
                    edit: TextEdit::new(t),
                    items: items.clone(),
                    open: false,
                },
                r,
                &text,
                Look {
                    list_row: LIST_ROW_H,
                    ..Look::default()
                },
            ),
            Kind::List { items, selected } => {
                let frame = mk(
                    ui,
                    Widget::Panel,
                    r,
                    "",
                    Look {
                        fill: Some(Token::Field),
                        border: Some(Token::FieldBorder),
                        still: true,
                        ..Look::default()
                    },
                );
                if let Some(n) = ui.tree.get_mut(frame) {
                    n.clip_children = true;
                }
                let vis = ((r.h - 2.0) / LIST_ROW_H).floor().max(1.0) as usize;
                let mut rows = Vec::new();
                for k in 0..vis {
                    let mut n = Node::new(
                        Widget::Button {
                            toggled: Some(false),
                        },
                        Rect::new(1.0, 1.0 + k as f32 * LIST_ROW_H, r.w - 2.0, LIST_ROW_H),
                    )
                    .with_look(Look {
                        content: Content::TextOnly,
                        align: 0.0,
                        pad: 6.0,
                        ..Look::default()
                    });
                    n.intercept = vec![
                        Key::Tab,
                        Key::Escape,
                        Key::Enter,
                        Key::Up,
                        Key::Down,
                        Key::Home,
                        Key::End,
                        Key::PageUp,
                        Key::PageDown,
                    ];
                    n.label.clone_from(&text);
                    let id = ui.add(frame, n);
                    self.owners.insert(id, Owner::Row(i, k));
                    rows.push(id);
                }
                let bar = (items.len() > vis).then(|| {
                    let mut sc = ScrollState::new(true, (items.len() - vis) as f32, vis as f32);
                    sc.line = 1.0;
                    let id = ui.add(
                        frame,
                        Node::new(
                            Widget::ScrollBar(sc),
                            Rect::new(r.w - BAR_W - 1.0, 1.0, BAR_W, r.h - 2.0),
                        ),
                    );
                    self.owners.insert(id, Owner::Bar(i));
                    id
                });
                self.lists.insert(
                    i,
                    ListState {
                        items: items.clone(),
                        selected: selected.filter(|s| *s < items.len()),
                        top: 0,
                        rows,
                        bar,
                    },
                );
                frame
            }
            Kind::Ref(v) => {
                let field = mk(
                    ui,
                    Widget::TextField(TextEdit::new(v)),
                    Rect::new(r.x, r.y, r.w - REF_BTN_W, r.h),
                    &text,
                    Look::default(),
                );
                let mut b = Node::new(
                    Widget::Button { toggled: None },
                    Rect::new(r.x + r.w - REF_BTN_W, r.y, REF_BTN_W, r.h),
                )
                .with_label(&format!("Collapse dialog: {text}"))
                .with_look(Look {
                    icon: self.icons.collapse,
                    content: if self.icons.collapse.is_some() {
                        Content::IconOnly
                    } else {
                        Content::TextOnly
                    },
                    fill: Some(Token::Field),
                    border: Some(Token::FieldBorder),
                    ..Look::default()
                });
                b.intercept = vec![Key::Tab, Key::Escape];
                let b = ui.add(panel, b);
                self.owners.insert(b, Owner::RefButton(i));
                extra.push(b);
                field
            }
            Kind::Button => mk(
                ui,
                Widget::Button { toggled: None },
                r,
                &text,
                Look {
                    content: Content::TextOnly,
                    fill: Some(Token::Field),
                    border: Some(Token::FieldBorder),
                    radius: 2.0,
                    ..Look::default()
                },
            ),
        };
        self.owners.insert(main, Owner::Control(i));
        if let Some(n) = ui.tree.get_mut(main) {
            n.enabled = c.enabled;
            match c.kind {
                Kind::Label | Kind::Group | Kind::List { .. } => {}
                Kind::Button => {
                    n.access_key = key;
                    n.intercept = vec![Key::Tab, Key::Escape];
                }
                Kind::Dropdown { .. } | Kind::Combo { .. } => {
                    n.access_key = key;
                    n.intercept = vec![Key::Tab, Key::Escape, Key::Enter];
                }
                _ => {
                    n.access_key = key;
                    n.intercept = vec![Key::Tab, Key::Escape, Key::Enter];
                }
            }
        }
        if !c.enabled {
            for e in &extra {
                if let Some(n) = ui.tree.get_mut(*e) {
                    n.enabled = false;
                }
            }
        }
        Slot { main, extra }
    }

    /// A label with an access key hands it to the next field in the dialog, as Windows does:
    /// Alt+key focuses that field (and selects its text). The key sits on an invisible,
    /// zero-sized button of the label's slot so it follows the label's page and is skipped by
    /// assistive technology.
    fn assign_access_keys(&mut self, ui: &mut Ui) {
        for i in 0..self.spec.controls.len() {
            let c = &self.spec.controls[i];
            if !matches!(c.kind, Kind::Label) {
                continue;
            }
            let (name, Some(key)) = c.caption() else {
                continue;
            };
            let page = c.page;
            let target = self.spec.controls[i + 1..].iter().position(|n| {
                !matches!(n.kind, Kind::Label | Kind::Group) && (n.page == page || n.page.is_none())
            });
            let Some(off) = target else { continue };
            let t = i + 1 + off;
            let name = name.trim_end_matches(':').to_owned();
            let main = self.slots[t].main;
            if let Some(n) = ui.tree.get_mut(main)
                && n.label.is_empty()
            {
                n.label.clone_from(&name);
            }
            if let Some(ls) = self.lists.get(&t) {
                for r in &ls.rows {
                    if let Some(n) = ui.tree.get_mut(*r) {
                        n.label.clone_from(&name);
                    }
                }
            }
            let Some(panel) = self.panel else { continue };
            let mut proxy = Node::new(
                Widget::Button { toggled: None },
                Rect::new(0.0, 0.0, 0.0, 0.0),
            )
            .with_label(&name);
            proxy.access_key = Some(key);
            let id = ui.add(panel, proxy);
            ui.set_canvas_access(id, hidden_node(), Vec::new());
            self.owners.insert(id, Owner::Proxy(t));
            self.slots[i].extra.push(id);
        }
    }

    /// Dialog role, modality and the title as its label for assistive technology.
    fn apply_access(&mut self, ui: &mut Ui) {
        let (Some(panel), Some(title)) = (self.panel, self.title) else {
            return;
        };
        let mut n = accesskit::Node::new(accesskit::Role::Dialog);
        n.set_modal();
        n.set_labelled_by(vec![ak_id(title)]);
        ui.set_canvas_access(panel, n, Vec::new());
    }

    // ---- layout -----------------------------------------------------------------------------

    fn window(ui: &Ui) -> Rect {
        let root = ui.tree.root();
        ui.tree.get(root).map_or(Rect::default(), |n| n.bounds)
    }

    /// Where the panel sits: where the user dragged it, else centred, always inside the window.
    fn panel_origin(&self, ui: &Ui, w: f32, h: f32) -> Point {
        let win = Self::window(ui);
        let p = self.moved.unwrap_or_else(|| {
            Point::new(((win.w - w) / 2.0).floor(), ((win.h - h) / 2.0).floor())
        });
        clamp_origin(p, w, win)
    }

    /// Positions every node from the spec; also the collapsed form of a reference field.
    fn apply_layout(&mut self, ui: &mut Ui) {
        let (Some(scrim), Some(panel)) = (self.scrim, self.panel) else {
            return;
        };
        let win = Self::window(ui);
        if let Some(n) = ui.tree.get_mut(scrim) {
            n.bounds = Rect::new(0.0, 0.0, win.w, win.h);
            n.visible = self.collapsed.is_none();
        }
        let (full_w, full_h) = self.size();
        let (w, h) = match self.collapsed {
            Some(_) => (full_w.min(380.0), TITLE_H + 2.0 * MARGIN + FIELD_H),
            None => (full_w, full_h),
        };
        let origin = if self.collapsed.is_some() {
            // The collapsed bar sits near the top, out of the way of the sheet's first rows.
            let p = self
                .collapsed_at
                .unwrap_or_else(|| Point::new(((win.w - w) / 2.0).floor(), 8.0));
            clamp_origin(p, w, win)
        } else {
            self.panel_origin(ui, w, h)
        };
        if let Some(n) = ui.tree.get_mut(panel) {
            n.bounds = Rect::new(origin.x, origin.y, w, h);
        }
        for id in [self.title, self.drag_bar].into_iter().flatten() {
            if let Some(n) = ui.tree.get_mut(id) {
                n.bounds = Rect::new(0.0, 0.0, w - 46.0, TITLE_H);
            }
        }
        if let Some(c) = self.close
            && let Some(n) = ui.tree.get_mut(c)
        {
            n.bounds = Rect::new(w - 46.0, 0.0, 46.0, TITLE_H);
        }
        if let Some(s) = self.strip
            && let Some(n) = ui.tree.get_mut(s)
        {
            n.bounds = Rect::new(MARGIN, TITLE_H + 4.0, w - 2.0 * MARGIN, TAB_H);
        }
        // Footer buttons, right aligned.
        let y = TITLE_H + self.spec.height + (FOOTER_H - BUTTON_H) / 2.0;
        let mut x = full_w - MARGIN;
        for id in self.footer.iter().rev() {
            x -= BUTTON_W;
            if let Some(n) = ui.tree.get_mut(*id) {
                n.bounds = Rect::new(x, y, BUTTON_W, BUTTON_H);
            }
            x -= 6.0;
        }
        if let Some(e) = self.error
            && let Some(n) = ui.tree.get_mut(e)
        {
            n.bounds = Rect::new(MARGIN, y, (x - MARGIN).max(40.0), BUTTON_H);
        }
        if let Some(i) = self.collapsed {
            let c = &self.spec.controls[i];
            let _ = c;
            let slot = &self.slots[i];
            if let Some(n) = ui.tree.get_mut(slot.main) {
                n.bounds = Rect::new(
                    MARGIN,
                    TITLE_H + MARGIN,
                    w - 2.0 * MARGIN - REF_BTN_W,
                    FIELD_H,
                );
            }
            if let Some(b) = slot.extra.first()
                && let Some(n) = ui.tree.get_mut(*b)
            {
                n.bounds = Rect::new(w - MARGIN - REF_BTN_W, TITLE_H + MARGIN, REF_BTN_W, FIELD_H);
            }
        } else {
            for (i, c) in self.spec.controls.iter().enumerate() {
                let r = c.rect.translate(0.0, TITLE_H);
                let slot = &self.slots[i];
                match &c.kind {
                    Kind::Ref(_) => {
                        if let Some(n) = ui.tree.get_mut(slot.main) {
                            n.bounds = Rect::new(r.x, r.y, r.w - REF_BTN_W, r.h);
                        }
                        if let Some(b) = slot.extra.first()
                            && let Some(n) = ui.tree.get_mut(*b)
                        {
                            n.bounds = Rect::new(r.x + r.w - REF_BTN_W, r.y, REF_BTN_W, r.h);
                        }
                    }
                    Kind::Label | Kind::Group => {}
                    _ => {
                        if let Some(n) = ui.tree.get_mut(slot.main) {
                            n.bounds = r;
                        }
                    }
                }
            }
        }
    }

    /// Recentres or clamps the dialog after the window changed size.
    pub fn relayout(&mut self, ui: &mut Ui) {
        self.apply_layout(ui);
    }

    /// Moves the dialog to the top of the stacking order (after other nodes were added to the
    /// root since it opened).
    pub fn raise(&self, ui: &mut Ui) {
        let Some(s) = self.scrim else { return };
        let root = ui.tree.root();
        if let Some(r) = ui.tree.get_mut(root)
            && r.children.last() != Some(&s)
            && let Some(pos) = r.children.iter().position(|c| *c == s)
        {
            r.children.remove(pos);
            r.children.push(s);
        }
    }

    // ---- pages --------------------------------------------------------------------------------

    /// Shows page `page` (a tab or a wizard step).
    pub fn set_page(&mut self, ui: &mut Ui, page: usize) {
        self.page = page;
        if let Some(s) = self.strip
            && let Some(Widget::TabStrip { selected, tabs, .. }) =
                ui.tree.get_mut(s).map(|n| &mut n.widget)
        {
            *selected = page.min(tabs.len().saturating_sub(1));
        }
        self.refresh_pages(ui);
        // The control that had the focus is on a page that is gone: the tab strip takes it.
        if let (Some(f), Some(s)) = (ui.focus(), self.strip)
            && !ui.tree.shown(f)
        {
            self.focus_node(ui, s);
        }
    }

    /// Shows or hides a control (and its helper nodes).
    pub fn set_visible(&mut self, ui: &mut Ui, id: &str, visible: bool) {
        if let Some(i) = self.index_of(id) {
            self.spec.controls[i].page = if visible {
                self.spec.controls[i].page
            } else {
                Some(usize::MAX)
            };
            self.refresh_pages(ui);
        }
    }

    fn refresh_pages(&mut self, ui: &mut Ui) {
        let shown: Vec<bool> = self
            .spec
            .controls
            .iter()
            .enumerate()
            .map(|(i, c)| match self.collapsed {
                Some(k) => k == i,
                None => c.page.is_none() || c.page == Some(self.page),
            })
            .collect();
        for (i, show) in shown.into_iter().enumerate() {
            let slot = &self.slots[i];
            for id in std::iter::once(&slot.main).chain(slot.extra.iter()) {
                if let Some(n) = ui.tree.get_mut(*id) {
                    n.visible = show;
                }
            }
            if show {
                self.sync_list_rows(ui, i);
            }
        }
        let show_chrome = self.collapsed.is_none();
        for id in self
            .footer
            .iter()
            .chain(self.strip.iter())
            .chain(self.error.iter())
        {
            if let Some(n) = ui.tree.get_mut(*id) {
                n.visible = show_chrome;
            }
        }
    }

    // ---- values -------------------------------------------------------------------------------

    /// Every control's current value.
    pub fn values(&self, ui: &Ui) -> Values {
        let mut v = Values::default();
        for (i, c) in self.spec.controls.iter().enumerate() {
            if c.id.is_empty() {
                continue;
            }
            let Some(n) = ui.tree.get(self.slots[i].main) else {
                continue;
            };
            match &n.widget {
                Widget::TextField(e) => v.insert(&c.id, Value::Text(e.text.clone())),
                Widget::Combo { edit, .. } => v.insert(&c.id, Value::Text(edit.text.clone())),
                Widget::Spinner {
                    edit,
                    value,
                    min,
                    max,
                    ..
                } => {
                    let parsed = edit.text.trim().parse::<f64>().unwrap_or(*value);
                    v.insert(&c.id, Value::Number(parsed.clamp(*min, *max)));
                }
                Widget::Checkbox { checked } => v.insert(&c.id, Value::Bool(*checked)),
                Widget::Radio { checked, .. } => {
                    v.insert(&c.id, Value::Bool(*checked));
                    if *checked && let Kind::Radio { group, .. } = &c.kind {
                        v.set_group(group, &c.id);
                    }
                }
                Widget::Dropdown {
                    selected, items, ..
                } => v.insert(
                    &c.id,
                    Value::Index((!items.is_empty()).then_some(*selected)),
                ),
                Widget::Panel => {
                    if let Some(ls) = self.lists.get(&i) {
                        v.insert(&c.id, Value::Index(ls.selected));
                    }
                }
                _ => {}
            }
        }
        v
    }

    /// The text of dropdown item `i` / list item `i` of control `id`.
    pub fn item(&self, ui: &Ui, id: &str, i: usize) -> Option<String> {
        let k = self.index_of(id)?;
        if let Some(ls) = self.lists.get(&k) {
            return ls.items.get(i).cloned();
        }
        match &ui.tree.get(self.slots[k].main)?.widget {
            Widget::Dropdown { items, .. } | Widget::Combo { items, .. } => items.get(i).cloned(),
            _ => None,
        }
    }

    /// Sets the text of a field, combo or reference field, or a label.
    pub fn set_text(&mut self, ui: &mut Ui, id: &str, text: &str) {
        let Some(i) = self.index_of(id) else { return };
        let main = self.slots[i].main;
        if let Some(n) = ui.tree.get_mut(main) {
            match &mut n.widget {
                Widget::TextField(e) | Widget::Combo { edit: e, .. } => {
                    e.text = text.to_owned();
                    e.caret = text.len();
                    e.anchor = text.len();
                }
                Widget::Spinner { edit, value, .. } => {
                    edit.text = text.to_owned();
                    edit.caret = text.len();
                    edit.anchor = text.len();
                    if let Ok(v) = text.trim().parse::<f64>() {
                        *value = v;
                    }
                }
                Widget::Label => n.label = text.to_owned(),
                _ => {}
            }
        }
        if matches!(self.spec.controls[i].kind, Kind::Label) {
            self.spec.controls[i].label = text.to_owned();
        }
    }

    pub fn set_number(&mut self, ui: &mut Ui, id: &str, v: f64) {
        let Some(i) = self.index_of(id) else { return };
        if let Some(n) = ui.tree.get_mut(self.slots[i].main)
            && let Widget::Spinner {
                value, edit, step, ..
            } = &mut n.widget
        {
            *value = v;
            edit.text = format_number(v, *step);
            edit.caret = edit.text.len();
            edit.anchor = edit.text.len();
        }
    }

    pub fn set_checked(&mut self, ui: &mut Ui, id: &str, on: bool) {
        let Some(i) = self.index_of(id) else { return };
        if let Some(n) = ui.tree.get_mut(self.slots[i].main) {
            match &mut n.widget {
                Widget::Checkbox { checked } | Widget::Radio { checked, .. } => *checked = on,
                _ => {}
            }
        }
    }

    /// Checks radio `id` and unchecks the others of its group.
    pub fn set_radio(&mut self, ui: &mut Ui, id: &str) {
        let Some(i) = self.index_of(id) else { return };
        let Kind::Radio { group, .. } = self.spec.controls[i].kind.clone() else {
            return;
        };
        for (k, c) in self.spec.controls.iter().enumerate() {
            if let Kind::Radio { group: g, .. } = &c.kind
                && *g == group
                && let Some(n) = ui.tree.get_mut(self.slots[k].main)
                && let Widget::Radio { checked, .. } = &mut n.widget
            {
                *checked = k == i;
            }
        }
    }

    pub fn set_enabled(&mut self, ui: &mut Ui, id: &str, on: bool) {
        if let Some(i) = self.index_of(id) {
            self.spec.controls[i].enabled = on;
            let slot = &self.slots[i];
            for n in std::iter::once(&slot.main).chain(slot.extra.iter()) {
                if let Some(n) = ui.tree.get_mut(*n) {
                    n.enabled = on;
                }
            }
            if let Some(ls) = self.lists.get(&i) {
                for r in &ls.rows {
                    if let Some(n) = ui.tree.get_mut(*r) {
                        n.enabled = on;
                    }
                }
            }
        } else if let Some(j) = self.spec.footer.iter().position(|b| b.id == id) {
            self.spec.footer[j].enabled = on;
            if let Some(n) = ui.tree.get_mut(self.footer[j]) {
                n.enabled = on;
            }
        }
    }

    /// Replaces the items of a drop-down, combo or list box.
    pub fn set_items(
        &mut self,
        ui: &mut Ui,
        id: &str,
        items: Vec<String>,
        selected: Option<usize>,
    ) {
        let Some(i) = self.index_of(id) else { return };
        if self.lists.contains_key(&i) {
            let vis = self.lists.get(&i).map_or(1, |l| l.rows.len());
            let frame = self.slots[i].main;
            let r = self.spec.controls[i].rect;
            // The scroll bar exists only while the items overflow; rebuild it when that changes.
            if let Some(ls) = self.lists.get_mut(&i) {
                let had = ls.bar.is_some();
                let needs = items.len() > vis;
                if had != needs {
                    if let Some(b) = ls.bar.take() {
                        ui.tree.remove(b);
                        self.owners.remove(&b);
                    }
                    if needs {
                        let sc = {
                            let mut s = ScrollState::new(true, 0.0, vis as f32);
                            s.line = 1.0;
                            s
                        };
                        let id = ui.add(
                            frame,
                            Node::new(
                                Widget::ScrollBar(sc),
                                Rect::new(r.w - BAR_W - 1.0, 1.0, BAR_W, r.h - 2.0),
                            ),
                        );
                        self.owners.insert(id, Owner::Bar(i));
                        ls.bar = Some(id);
                    }
                }
                ls.selected = selected.filter(|s| *s < items.len());
                ls.items = items;
                ls.top = 0;
            }
            if let Some(s) = selected {
                self.ensure_visible(i, s);
            }
            self.sync_list_rows(ui, i);
            return;
        }
        if let Some(n) = ui.tree.get_mut(self.slots[i].main) {
            match &mut n.widget {
                Widget::Dropdown {
                    items: it,
                    selected: s,
                    ..
                } => {
                    *s = selected.unwrap_or(0).min(items.len().saturating_sub(1));
                    *it = items;
                }
                Widget::Combo { items: it, .. } => *it = items,
                _ => {}
            }
        }
    }

    /// Selects dropdown item `i`.
    pub fn set_choice(&mut self, ui: &mut Ui, id: &str, i: usize) {
        let Some(k) = self.index_of(id) else { return };
        if self.lists.contains_key(&k) {
            self.select_row(ui, k, i, false);
            return;
        }
        if let Some(n) = ui.tree.get_mut(self.slots[k].main)
            && let Widget::Dropdown {
                items, selected, ..
            } = &mut n.widget
        {
            *selected = i.min(items.len().saturating_sub(1));
        }
    }

    /// Shows `msg` beside the footer buttons (empty clears it).
    pub fn set_error(&mut self, ui: &mut Ui, msg: &str) {
        if let Some(e) = self.error
            && let Some(n) = ui.tree.get_mut(e)
        {
            n.label = msg.to_owned();
        }
    }

    /// The message shown beside the footer buttons.
    pub fn error(&self, ui: &Ui) -> String {
        self.error
            .and_then(|e| ui.tree.get(e))
            .map(|n| n.label.clone())
            .unwrap_or_default()
    }

    /// Moves the focus to control `id`.
    pub fn focus(&mut self, ui: &mut Ui, id: &str) {
        if let Some(n) = self.focus_target(id) {
            self.focus_node(ui, n);
        }
    }

    fn focus_target(&self, id: &str) -> Option<NodeId> {
        if let Some(j) = self.spec.footer.iter().position(|b| b.id == id) {
            return self.footer.get(j).copied();
        }
        let i = self.index_of(id)?;
        self.focus_target_index(i)
    }

    fn focus_target_index(&self, i: usize) -> Option<NodeId> {
        if let Some(ls) = self.lists.get(&i) {
            return self.row_stop(ls);
        }
        Some(self.slots[i].main)
    }

    fn focus_node(&self, ui: &mut Ui, n: NodeId) {
        ui.set_focus(Some(n));
        if let Some(node) = ui.tree.get_mut(n) {
            match &mut node.widget {
                Widget::TextField(e)
                | Widget::Spinner { edit: e, .. }
                | Widget::Combo { edit: e, .. } => {
                    e.select_all();
                }
                _ => {}
            }
        }
    }

    // ---- lists ----------------------------------------------------------------------------------

    /// The row that takes the Tab stop of a list: the selected one when visible, else the first.
    fn row_stop(&self, ls: &ListState) -> Option<NodeId> {
        let k = ls
            .selected
            .and_then(|s| s.checked_sub(ls.top))
            .filter(|k| *k < ls.rows.len())
            .unwrap_or(0);
        ls.rows.get(k).copied()
    }

    fn ensure_visible(&mut self, i: usize, sel: usize) {
        if let Some(ls) = self.lists.get_mut(&i) {
            let vis = ls.rows.len();
            if sel < ls.top {
                ls.top = sel;
            } else if sel >= ls.top + vis {
                ls.top = sel + 1 - vis;
            }
        }
    }

    fn sync_list_rows(&mut self, ui: &mut Ui, i: usize) {
        let Some(ls) = self.lists.get(&i) else { return };
        let list_visible = ui.tree.get(self.slots[i].main).is_some_and(|n| n.visible);
        for (k, row) in ls.rows.iter().enumerate() {
            let idx = ls.top + k;
            if let Some(n) = ui.tree.get_mut(*row) {
                match ls.items.get(idx) {
                    Some(t) => {
                        t.clone_into(&mut n.label);
                        n.visible = list_visible;
                        n.widget = Widget::Button {
                            toggled: Some(ls.selected == Some(idx)),
                        };
                    }
                    None => {
                        n.visible = false;
                        n.label.clear();
                    }
                }
            }
        }
        if let Some(b) = ls.bar
            && let Some(Widget::ScrollBar(sc)) = ui.tree.get_mut(b).map(|n| &mut n.widget)
        {
            sc.max = (ls.items.len().saturating_sub(ls.rows.len())) as f32;
            sc.value = ls.top as f32;
        }
    }

    fn select_row(&mut self, ui: &mut Ui, i: usize, idx: usize, focus: bool) -> bool {
        let Some(ls) = self.lists.get_mut(&i) else {
            return false;
        };
        if idx >= ls.items.len() {
            return false;
        }
        let changed = ls.selected != Some(idx);
        ls.selected = Some(idx);
        self.ensure_visible(i, idx);
        self.sync_list_rows(ui, i);
        if focus
            && let Some(ls) = self.lists.get(&i)
            && let Some(row) = ls.rows.get(idx.saturating_sub(ls.top))
        {
            ui.set_focus(Some(*row));
        }
        changed
    }

    // ---- focus ----------------------------------------------------------------------------------

    /// The nodes Tab visits, in order.
    pub fn tab_stops(&self, ui: &Ui) -> Vec<NodeId> {
        let mut out = Vec::new();
        if let Some(i) = self.collapsed {
            out.push(self.slots[i].main);
            out.extend(self.slots[i].extra.first().copied());
            return out;
        }
        out.extend(self.strip);
        for (i, c) in self.spec.controls.iter().enumerate() {
            let shown = c.page.is_none() || c.page == Some(self.page);
            if !shown || !c.enabled || matches!(c.kind, Kind::Label | Kind::Group) {
                continue;
            }
            let slot = &self.slots[i];
            match &c.kind {
                Kind::List { .. } => {
                    if let Some(ls) = self.lists.get(&i)
                        && let Some(r) = self.row_stop(ls)
                    {
                        out.push(r);
                    }
                }
                Kind::Radio { group, .. } => {
                    // One stop per group: the checked radio, or the first when none is checked.
                    let group_has_check = self.spec.controls.iter().enumerate().any(|(k, o)| {
                        matches!(&o.kind, Kind::Radio { group: g, .. } if g == group)
                            && self.radio_on(ui, k)
                    });
                    let mine = self.radio_on(ui, i);
                    let first = self
                        .spec
                        .controls
                        .iter()
                        .position(|o| {
                            matches!(&o.kind, Kind::Radio { group: g, .. } if g == group)
                                && o.enabled
                                && (o.page.is_none() || o.page == Some(self.page))
                        })
                        .is_some_and(|f| f == i);
                    if mine || (!group_has_check && first) {
                        out.push(slot.main);
                    }
                }
                Kind::Ref(_) => {
                    out.push(slot.main);
                    out.extend(slot.extra.first().copied());
                }
                _ => out.push(slot.main),
            }
        }
        for (j, b) in self.spec.footer.iter().enumerate() {
            if b.enabled {
                out.push(self.footer[j]);
            }
        }
        out
    }

    fn radio_on(&self, ui: &Ui, i: usize) -> bool {
        matches!(
            ui.tree.get(self.slots[i].main).map(|n| &n.widget),
            Some(Widget::Radio { checked: true, .. })
        )
    }
}

// ---- events ---------------------------------------------------------------------------------

impl Dialog {
    /// Offers one `Ui` event to the dialog.
    pub fn handle(&mut self, ui: &mut Ui, ev: &Emit) -> Response {
        if self.panel.is_none() {
            return Response::Ignored;
        }
        match ev {
            Emit::Clicked(n) => match self.owners.get(n).copied() {
                Some(o) => self.clicked(ui, o),
                None => Response::Ignored,
            },
            Emit::Toggled(n, _) => match self.owners.get(n).copied() {
                Some(Owner::Control(i)) => self.changed(i),
                Some(Owner::Row(i, k)) => {
                    let top = self.lists.get(&i).map_or(0, |l| l.top);
                    self.select_row(ui, i, top + k, false);
                    self.changed(i)
                }
                Some(_) => Response::Consumed,
                None => Response::Ignored,
            },
            Emit::Selected(n, _) | Emit::TextChanged(n, _) | Emit::ValueChanged(n, _) => {
                match self.owners.get(n).copied() {
                    Some(Owner::Control(i)) => self.changed(i),
                    Some(_) => Response::Consumed,
                    None => Response::Ignored,
                }
            }
            Emit::TextCommitted(n, _) => {
                if self.owners.contains_key(n) {
                    Response::Consumed
                } else {
                    Response::Ignored
                }
            }
            Emit::TabSelected(n, i) if Some(*n) == self.strip => {
                self.page = *i;
                self.refresh_pages(ui);
                Response::Outcome(Outcome::Page(*i))
            }
            Emit::ScrollChanged(n, v) => match self.owners.get(n).copied() {
                Some(Owner::Bar(i)) => {
                    if let Some(ls) = self.lists.get_mut(&i) {
                        let max = ls.items.len().saturating_sub(ls.rows.len());
                        ls.top = (v.round().max(0.0) as usize).min(max);
                    }
                    self.sync_list_rows(ui, i);
                    Response::Consumed
                }
                _ => Response::Ignored,
            },
            Emit::KeyPressed(n, key, m) => self.key(ui, *n, *key, m.shift, m.ctrl),
            Emit::CanvasPointer { node, at, .. } if Some(*node) == self.drag_bar => {
                self.grab = Some(*at);
                Response::Consumed
            }
            Emit::CanvasDrag { node, at, .. } if Some(*node) == self.drag_bar => {
                if let Some(g) = self.grab {
                    self.drag_by(ui, at.x - g.x, at.y - g.y);
                }
                Response::Consumed
            }
            Emit::CanvasRelease { node, .. } if Some(*node) == self.drag_bar => {
                self.grab = None;
                Response::Consumed
            }
            _ => Response::Ignored,
        }
    }

    fn changed(&self, i: usize) -> Response {
        match self.spec.controls.get(i) {
            Some(c) if !c.id.is_empty() => Response::Outcome(Outcome::Changed(c.id.clone())),
            _ => Response::Consumed,
        }
    }

    fn cancel_id(&self) -> Option<String> {
        self.spec
            .footer
            .iter()
            .find(|b| b.role == ButtonRole::Cancel)
            .map(|b| b.id.clone())
    }

    fn pressed_cancel(&self) -> Response {
        match self.cancel_id() {
            Some(id) => Response::Outcome(Outcome::Pressed(id)),
            None => Response::Consumed,
        }
    }

    fn pressed_default(&self) -> Response {
        match self
            .spec
            .footer
            .iter()
            .find(|b| b.role == ButtonRole::Default && b.enabled)
        {
            Some(b) => Response::Outcome(Outcome::Pressed(b.id.clone())),
            None => Response::Consumed,
        }
    }

    fn clicked(&mut self, ui: &mut Ui, o: Owner) -> Response {
        match o {
            Owner::Footer(j) => Response::Outcome(Outcome::Pressed(self.spec.footer[j].id.clone())),
            Owner::Close => self.pressed_cancel(),
            Owner::Control(i) if matches!(self.spec.controls[i].kind, Kind::Button) => {
                Response::Outcome(Outcome::Pressed(self.spec.controls[i].id.clone()))
            }
            Owner::RefButton(i) => self.toggle_ref(ui, i),
            Owner::Proxy(t) => {
                // Alt+key on a label: focus its field unless that is disabled or hidden.
                let id = self.spec.controls[t].id.clone();
                let usable = ui
                    .tree
                    .get(self.slots[t].main)
                    .is_some_and(|n| n.enabled && ui.tree.shown(self.slots[t].main));
                if usable && let Some(n) = self.focus_target_index(t) {
                    self.focus_node(ui, n);
                }
                let _ = id;
                Response::Consumed
            }
            _ => Response::Consumed,
        }
    }

    fn toggle_ref(&mut self, ui: &mut Ui, i: usize) -> Response {
        let id = self.spec.controls[i].id.clone();
        let collapsed = self.collapsed.is_none();
        self.collapsed = collapsed.then_some(i);
        self.apply_layout(ui);
        self.refresh_pages(ui);
        self.raise(ui);
        let field = self.slots[i].main;
        if collapsed {
            self.focus_node(ui, field);
        } else {
            let btn = self.slots[i].extra.first().copied().unwrap_or(field);
            self.focus_node(ui, btn);
        }
        Response::Outcome(Outcome::Ref { id, collapsed })
    }

    fn popup_open(&self, ui: &Ui, n: NodeId) -> bool {
        matches!(
            ui.tree.get(n).map(|x| &x.widget),
            Some(Widget::Dropdown { open: true, .. } | Widget::Combo { open: true, .. })
        )
    }

    fn close_popup(&self, ui: &mut Ui, n: NodeId) {
        if let Some(node) = ui.tree.get_mut(n) {
            match &mut node.widget {
                Widget::Dropdown { open, .. } | Widget::Combo { open, .. } => *open = false,
                _ => {}
            }
        }
    }

    fn key(&mut self, ui: &mut Ui, n: NodeId, key: Key, shift: bool, ctrl: bool) -> Response {
        let owner = self.owners.get(&n).copied();
        if owner.is_none() {
            // A key for the sheet while the reference field is collapsed: Enter or Esc expand.
            if let Some(i) = self.collapsed
                && matches!(key, Key::Enter | Key::Escape)
            {
                return self.toggle_ref(ui, i);
            }
            return Response::Ignored;
        }
        match key {
            Key::Tab if ctrl && !self.spec.tabs.is_empty() => {
                let count = self.spec.tabs.len();
                let page = if shift {
                    (self.page + count - 1) % count
                } else {
                    (self.page + 1) % count
                };
                self.set_page(ui, page);
                Response::Outcome(Outcome::Page(page))
            }
            Key::Tab => {
                self.focus_move(ui, shift);
                Response::Consumed
            }
            Key::Escape | Key::Enter if self.popup_open(ui, n) => {
                self.close_popup(ui, n);
                Response::Consumed
            }
            Key::Escape | Key::Enter if self.collapsed.is_some() => {
                let i = self.collapsed.unwrap_or(0);
                self.toggle_ref(ui, i)
            }
            Key::Escape => self.pressed_cancel(),
            Key::Enter => self.pressed_default(),
            Key::Up | Key::Down | Key::Home | Key::End | Key::PageUp | Key::PageDown => {
                if let Some(Owner::Row(i, k)) = owner {
                    self.row_key(ui, i, k, key)
                } else {
                    Response::Consumed
                }
            }
            _ => Response::Consumed,
        }
    }

    fn row_key(&mut self, ui: &mut Ui, i: usize, k: usize, key: Key) -> Response {
        let Some(ls) = self.lists.get(&i) else {
            return Response::Consumed;
        };
        let count = ls.items.len();
        if count == 0 {
            return Response::Consumed;
        }
        let cur = ls.selected.unwrap_or(ls.top + k);
        let page = ls.rows.len().max(1);
        let to = match key {
            Key::Up => cur.saturating_sub(1),
            Key::Down => (cur + 1).min(count - 1),
            Key::Home => 0,
            Key::End => count - 1,
            Key::PageUp => cur.saturating_sub(page),
            _ => (cur + page).min(count - 1),
        };
        self.select_row(ui, i, to, true);
        self.changed(i)
    }

    fn stop_of(&self, f: NodeId) -> NodeId {
        match self.owners.get(&f) {
            Some(Owner::Row(i, _)) => self
                .lists
                .get(i)
                .and_then(|l| self.row_stop(l))
                .unwrap_or(f),
            _ => f,
        }
    }

    fn focus_move(&self, ui: &mut Ui, back: bool) {
        let stops = self.tab_stops(ui);
        if stops.is_empty() {
            return;
        }
        let cur = ui
            .focus()
            .map(|f| self.stop_of(f))
            .and_then(|f| stops.iter().position(|s| *s == f));
        let next = match (cur, back) {
            (None, false) => 0,
            (None, true) => stops.len() - 1,
            (Some(i), false) => (i + 1) % stops.len(),
            (Some(i), true) => (i + stops.len() - 1) % stops.len(),
        };
        self.focus_node(ui, stops[next]);
    }

    fn drag_by(&mut self, ui: &mut Ui, dx: f32, dy: f32) {
        let Some(panel) = self.panel else { return };
        let win = Self::window(ui);
        let Some(n) = ui.tree.get_mut(panel) else {
            return;
        };
        let p = clamp_origin(
            Point::new(n.bounds.x + dx, n.bounds.y + dy),
            n.bounds.w,
            win,
        );
        n.bounds.x = p.x;
        n.bounds.y = p.y;
        if self.collapsed.is_some() {
            self.collapsed_at = Some(p);
        } else {
            self.moved = Some(p);
        }
    }
}

fn clamp_origin(p: Point, w: f32, win: Rect) -> Point {
    let min_x = -(w - 80.0);
    let max_x = (win.w - 80.0).max(min_x);
    let max_y = (win.h - TITLE_H).max(0.0);
    Point::new(p.x.clamp(min_x, max_x), p.y.clamp(0.0, max_y))
}

fn label_look(ink: Token, pad: f32) -> Look {
    Look {
        content: Content::TextOnly,
        align: 0.0,
        pad,
        ink: Some(ink),
        still: true,
        ..Look::default()
    }
}

/// An AccessKit node that screen readers skip (the scrim).
fn hidden_node() -> accesskit::Node {
    let mut n = accesskit::Node::new(accesskit::Role::GenericContainer);
    n.set_hidden();
    n
}

/// A number as a spin box shows it: no decimals for whole numbers.
pub fn format_number(v: f64, _step: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        let s = format!("{v:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

/// The serial of the dialog that is open on `ui` (the topmost one), if any.
pub fn scrim_serial(ui: &Ui) -> Option<u64> {
    let root = ui.tree.get(ui.tree.root())?;
    root.children.iter().rev().find_map(|c| {
        ui.tree
            .get(*c)
            .and_then(|n| n.label.strip_prefix(SCRIM_TAG))
            .and_then(|s| s.parse().ok())
    })
}
