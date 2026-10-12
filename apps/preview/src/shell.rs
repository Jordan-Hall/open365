//! The preview chrome: the title row, tab row, ribbon, formula strip, sheet tabs, status bar and
//! Backstage of Writer and Sheets, built as `open365-ui-core` widgets from the `ui-shell` spec
//! JSON. Positions come from the shared pure layout (`layout::compute`), so the responsive ribbon
//! rules are the ones the egui shell already replays against Office.
//!
//! The tree is rebuilt in two scopes: [`Shell::build_all`] (resize, ribbon pin, scale) and
//! [`Shell::build_body`] (ribbon tab, collapsed-group popup, Backstage, scrolling). Text typed in
//! the Name Box and the formula field lives in the shell, so it survives both.

use std::collections::HashMap;
use std::sync::Arc;

use open365_gfx::{DrawList, IconId, Rect as GRect};
use open365_ui_core::geom::{Point, Rect};
use open365_ui_core::{
    Content, Emit, InputEvent, Key, Look, MenuState, Node, NodeId, TabItem, TextEdit, Tip, Token,
    Ui, Widget,
};

use crate::layout::{self, LayoutInput, PlacedChrome, PlacedControl, PlacedGroup, PxRect};
use crate::menus;
use crate::palette::Palette;
use crate::ribbon::{self, RibbonCtx};
use crate::sheet::SheetModel;
use crate::spec::{AppSpec, Control, Kind, REF_SCALE, Scale as GroupScale, Size};
use crate::{AppKind, doc, icons};

mod grid;
mod share;

/// The ribbon control label size in points (`theme::size::LABEL` of the egui shell).
const RIBBON_LABEL_PT: f32 = 12.0;

/// What the window should do after a widget event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WinAct {
    Minimize,
    ToggleMaximize,
    Close,
    Title(String),
    /// Open this document in a new window (the host starts another instance on the file).
    OpenDocument(std::path::PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Act {
    Cmd(String),
    /// A button whose click opens the menu of the command.
    Menu(String),
    Tab,
    File,
    Caption(String),
    TitleBar,
    Nav(usize),
    Launcher(String),
    GroupButton(usize),
    ScrollRibbon,
    ToggleRibbon,
    NameBox,
    Formula,
    ExpandFormula,
    Zoom,
    ZoomStep(i32),
    ZoomLabel,
    Inert,
}

/// The widget that owns a window-sized overlay, to find it again.
#[derive(Debug, Default)]
struct Roles {
    name_box: Option<NodeId>,
    formula: Option<NodeId>,
    zoom_label: Option<NodeId>,
    strip: Option<NodeId>,
    group_popup: Option<NodeId>,
    menu: Option<NodeId>,
}

/// The chrome of one application.
pub struct Shell {
    pub kind: AppKind,
    spec: Arc<AppSpec>,
    pub pal: Palette,
    icons: HashMap<String, IconId>,
    scale: f32,
    win_px: (i32, i32),
    /// Left, top, right, bottom frame between the window edge and the Office pane (Office's own
    /// window has an invisible resize frame; the preview's borderless window has none).
    inset: (i32, i32, i32, i32),
    tab: usize,
    ribbon_open: bool,
    formula_expanded: bool,
    ribbon_scroll: i32,
    backstage: Option<usize>,
    group_popup: Option<usize>,
    name_box: String,
    formula: String,
    zoom: f32,
    note: String,
    acts: HashMap<NodeId, Act>,
    roles: Roles,
    static_ids: Vec<NodeId>,
    body_ids: Vec<NodeId>,
    /// Content rectangle of the document area (physical pixels).
    pub content: PxRect,
    pub maximized: bool,
    sheet: SheetModel,
    editor: Option<grid::Editor>,
    grid_ids: Vec<NodeId>,
    /// The Writer document (Writer only); `None` shows the placeholder page.
    writer: Option<doc::WriterPane>,
    refocus_doc: bool,
    /// Documents opened in new windows so far (names their temporary files).
    new_documents: u32,
    /// The operator the next Custom AutoFilter dialog preselects (index into its list).
    filter_op: usize,
    /// The open dialogs by scrim serial (see `shell::dialogs`).
    dialogs: HashMap<u64, grid::dialogs::Entry>,
    share: Option<share::ShareDialog>,
    share_o: (i32, i32),
    canvas: Option<NodeId>,
}

impl std::fmt::Debug for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shell")
            .field("kind", &self.kind)
            .field("tab", &self.tab)
            .finish_non_exhaustive()
    }
}

/// The menu items that carry an icon: the last dotted segment of the item id, and the Tabler
/// icon it shows. Every name must be in the icon table (see the `menu_icons_resolve` test).
pub(crate) const MENU_ICONS: &[(&str, &str)] = &[
    ("paste", "clipboard"),
    ("values", "numbers"),
    ("formatting", "paint"),
    ("link", "link"),
    ("special", "clipboard"),
    ("az", "sort-ascending-letters"),
    ("za", "sort-descending-letters"),
    ("filter", "filter"),
    ("invite", "users"),
    ("send", "mail-forward"),
    ("commands", "dots"),
    ("more", "dots"),
];

fn menu_icon_name(tail: &str) -> Option<&'static str> {
    MENU_ICONS.iter().find(|(k, _)| *k == tail).map(|(_, v)| *v)
}

const TOGGLE_IDS: [&str; 8] = [
    "bold",
    "italic",
    "underline",
    "wrap-text",
    "strikethrough",
    "subscript",
    "superscript",
    "format-painter",
];

impl Shell {
    pub fn new(kind: AppKind, dark: bool) -> Self {
        let spec = match kind {
            AppKind::Writer => AppSpec::writer(),
            AppKind::Sheets => AppSpec::sheets(),
        };
        Self {
            kind,
            spec: Arc::new(spec),
            pal: Palette::new(kind, dark),
            icons: HashMap::new(),
            scale: 1.25,
            win_px: (1600, 1000),
            inset: (0, 0, 0, 0),
            tab: 0,
            ribbon_open: true,
            formula_expanded: kind == AppKind::Sheets,
            ribbon_scroll: 0,
            backstage: None,
            group_popup: None,
            name_box: "A1".into(),
            formula: String::new(),
            zoom: 100.0,
            note: String::new(),
            acts: HashMap::new(),
            roles: Roles::default(),
            static_ids: Vec::new(),
            body_ids: Vec::new(),
            content: PxRect::default(),
            maximized: false,
            sheet: SheetModel::blank(),
            editor: None,
            grid_ids: Vec::new(),
            writer: None,
            refocus_doc: false,
            new_documents: 0,
            filter_op: 0,
            dialogs: HashMap::new(),
            share: None,
            share_o: (0, 0),
            canvas: None,
        }
    }

    pub fn spec(&self) -> &AppSpec {
        &self.spec
    }

    pub fn set_writer(&mut self, ui: &mut Ui, pane: doc::WriterPane) {
        self.writer = Some(pane);
        self.build_doc(ui);
        self.sync_history(ui);
    }

    pub fn writer(&self) -> Option<&doc::WriterPane> {
        self.writer.as_ref()
    }

    pub fn writer_mut(&mut self) -> Option<&mut doc::WriterPane> {
        self.writer.as_mut()
    }

    /// (Re)creates the document canvas node over the content area, or removes it under Backstage.
    pub(super) fn build_doc(&mut self, ui: &mut Ui) {
        let covered = self.backstage.is_some()
            || self.share.is_some()
            || self.content.w <= 0
            || self.content.h <= 0;
        let (area, scale, zoom, root) =
            (self.content, self.scale, self.zoom / 100.0, ui.tree.root());
        if let Some(p) = self.writer.as_mut() {
            if covered {
                p.release(ui);
            } else {
                p.build(ui, root, area, scale, zoom);
            }
        }
    }

    pub fn title(&self) -> String {
        if let Some(p) = self.writer.as_ref() {
            return p.title();
        }
        let doc = match self.kind {
            AppKind::Writer => "Document1 - Word",
            AppKind::Sheets => "Book1 - Excel",
        };
        if self.note.is_empty() {
            format!("{doc} (preview)")
        } else {
            format!("{doc} (preview) - {}", self.note)
        }
    }

    pub fn tab(&self) -> usize {
        self.tab
    }

    pub fn backstage_open(&self) -> bool {
        self.backstage.is_some()
    }

    pub fn select_tab(&mut self, ui: &mut Ui, i: usize) {
        if i < self.spec.tabs.len() && i != self.tab {
            self.tab = i;
            self.group_popup = None;
            self.ribbon_scroll = 0;
            self.build_body(ui);
        }
    }

    // ---- setup ----------------------------------------------------------------------------

    /// Registers the icons, applies the palette to the theme and builds the tree.
    pub fn init(&mut self, ui: &mut Ui) {
        self.icons = icons::register(&mut ui.assets);
        self.sync_size(ui);
        self.fit_labels(ui);
        self.apply_theme(ui);
        self.register_tip_levels(ui);
        self.build_all(ui);
    }

    /// Widens every large ribbon button to its label, as Office does ("Proofread"). The labels
    /// are measured in points at the ribbon label size ([`RIBBON_LABEL_PT`]), the unit
    /// `fit_large_labels` and the `ui-shell` `labels` test use, and the spec widths are in `u`
    /// at [`REF_SCALE`].
    fn fit_labels(&mut self, ui: &Ui) {
        let text_w = |s: &str| ui.measure_px(s, RIBBON_LABEL_PT * ui.scale.0) / ui.scale.0;
        layout::fit_large_labels(Arc::make_mut(&mut self.spec), text_w, REF_SCALE);
    }

    /// Takes the window size and scale from the `Ui` and rebuilds.
    pub fn resize(&mut self, ui: &mut Ui) {
        self.sync_size(ui);
        self.build_all(ui);
        self.dialog_resize(ui);
    }

    /// Sets the frame between the window edge and the pane (physical pixels) and rebuilds.
    pub fn set_inset(&mut self, ui: &mut Ui, inset: (i32, i32, i32, i32)) {
        self.inset = inset;
        self.build_all(ui);
    }

    pub fn settings_changed(&mut self, ui: &mut Ui) {
        self.pal = Palette::new(self.kind, ui.settings.apps_dark);
        self.apply_theme(ui);
        self.build_all(ui);
    }

    fn sync_size(&mut self, ui: &Ui) {
        self.scale = ui.scale.0;
        if let Some(r) = ui.tree.get(ui.tree.root()) {
            self.win_px = (
                (r.bounds.w * self.scale).round() as i32,
                (r.bounds.h * self.scale).round() as i32,
            );
        }
    }

    fn apply_theme(&mut self, ui: &mut Ui) {
        if !ui.settings.high_contrast {
            ui.theme.set(Token::Accent, self.pal.accent);
            ui.theme.set(Token::TabUnderline, self.pal.accent);
            ui.theme.set(Token::Frame, self.pal.frame);
        }
    }

    /// Registers, for every ribbon tab, the command-level key tips of its controls.
    fn register_tip_levels(&mut self, ui: &mut Ui) {
        for (ti, tab) in self.spec.tabs.iter().enumerate() {
            let mut tips = Vec::new();
            for g in &tab.groups {
                for c in g.controls() {
                    if let Some(k) = &c.key {
                        tips.push(Tip {
                            keys: k.to_ascii_uppercase(),
                            target: format!("cmd:{}", c.id),
                        });
                    }
                }
                if let Some(k) = &g.launcher_key {
                    tips.push(Tip {
                        keys: k.to_ascii_uppercase(),
                        target: format!("launcher:{}", g.id),
                    });
                }
                if let Some(k) = &g.popup_key {
                    tips.push(Tip {
                        keys: k.to_ascii_uppercase(),
                        target: format!("popup:{}", g.id),
                    });
                }
            }
            ui.set_tip_children(&format!("tab:{ti}"), tips);
        }
    }

    // ---- geometry helpers ------------------------------------------------------------------

    fn px(&self, u: i32) -> i32 {
        (u as f32 * self.scale / REF_SCALE).round() as i32
    }

    fn rect(&self, r: PxRect, origin: (i32, i32)) -> Rect {
        let s = self.scale;
        Rect::new(
            (r.x - origin.0) as f32 / s,
            (r.y - origin.1) as f32 / s,
            r.w as f32 / s,
            r.h as f32 / s,
        )
    }

    fn icon(&self, name: &str) -> Option<IconId> {
        self.icons.get(name).copied()
    }

    fn compute<'a>(&self, spec: &'a AppSpec) -> layout::Layout<'a> {
        layout::compute_scrolled(
            spec,
            &LayoutInput {
                scale: self.scale,
                window: self.win_px,
                inset: self.inset,
                tab: self.tab,
                ribbon_open: self.ribbon_open,
                formula_expanded: self.formula_expanded,
            },
            self.ribbon_scroll,
        )
    }

    // ---- node helpers ----------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        ui: &mut Ui,
        parent: NodeId,
        origin: (i32, i32),
        r: PxRect,
        widget: Widget,
        label: &str,
        look: Look,
        act: Act,
    ) -> NodeId {
        let b = self.rect(r, origin);
        let mut n = Node::new(widget, b).with_label(label).with_look(look);
        if !label.is_empty() && !matches!(n.widget, Widget::Panel | Widget::Label) {
            n.tooltip = Some(label.to_owned());
        }
        let id = ui.add(parent, n);
        self.acts.insert(id, act);
        id
    }

    fn panel(
        &mut self,
        ui: &mut Ui,
        parent: NodeId,
        origin: (i32, i32),
        r: PxRect,
        look: Look,
    ) -> NodeId {
        self.add(ui, parent, origin, r, Widget::Panel, "", look, Act::Inert)
    }

    fn clear(&mut self, ui: &mut Ui, ids: Vec<NodeId>) {
        for id in ids {
            ui.tree.remove(id);
        }
    }

    // ---- full build ------------------------------------------------------------------------

    pub fn build_all(&mut self, ui: &mut Ui) {
        let focus_role = ui.focus().and_then(|f| {
            if Some(f) == self.roles.formula {
                Some(true)
            } else if Some(f) == self.roles.name_box {
                Some(false)
            } else {
                None
            }
        });
        let old: Vec<NodeId> = std::mem::take(&mut self.static_ids);
        let old_body: Vec<NodeId> = std::mem::take(&mut self.body_ids);
        self.clear(ui, old);
        self.clear(ui, old_body);
        self.acts.clear();
        self.roles = Roles::default();
        let spec = self.spec.clone();
        let lay = self.compute(&spec);
        self.content = lay.content;
        self.build_static(ui, &lay);
        self.build_body_with(ui, &lay);
        match focus_role {
            Some(true) => ui.set_focus(self.roles.formula),
            Some(false) => ui.set_focus(self.roles.name_box),
            None => {}
        }
        self.sync_bars(ui);
        self.build_grid(ui);
        self.build_doc(ui);
        self.sync_history(ui);
        self.refresh_hover(ui);
        self.build_dialog(ui);
    }

    pub fn build_body(&mut self, ui: &mut Ui) {
        let old = std::mem::take(&mut self.body_ids);
        self.clear(ui, old);
        let spec = self.spec.clone();
        let lay = self.compute(&spec);
        self.build_body_with(ui, &lay);
        self.sync_bars(ui);
        self.build_grid(ui);
        self.build_doc(ui);
        self.refresh_hover(ui);
        self.build_dialog(ui);
    }

    fn refresh_hover(&self, ui: &mut Ui) {
        let p = ui.pointer();
        ui.handle(InputEvent::PointerMoved(p));
    }

    fn static_look(&self) -> Look {
        Look {
            still: true,
            ..Look::default()
        }
    }

    fn build_static(&mut self, ui: &mut Ui, lay: &layout::Layout<'_>) {
        let root = ui.tree.root();
        let o = (0, 0);
        let keep = |s: &mut Self, id: NodeId| {
            s.static_ids.push(id);
        };

        // Fills behind the controls.
        if lay.card.h > 0 {
            let p = self.panel(
                ui,
                root,
                o,
                lay.card,
                Look {
                    fill: Some(Token::Card),
                    border: Some(Token::CardBorder),
                    radius: 4.0,
                    still: true,
                    ..Look::default()
                },
            );
            keep(self, p);
        }
        let status_fill = Look {
            fill_rgb: Some(self.pal.status),
            still: true,
            ..Look::default()
        };
        let p = self.panel(ui, root, o, lay.status, status_fill);
        keep(self, p);
        let show_fbar = self.kind != AppKind::Sheets || self.sheet.ext_view.formula_bar;
        if show_fbar && let (Some(l), Some(b)) = (lay.formula_left, lay.formula_bar) {
            let x = l.x.min(b.x);
            let y = l.y.min(b.y);
            let r = PxRect {
                x,
                y,
                w: b.right().max(l.right()) - x,
                h: b.bottom().max(l.bottom()) - y,
            };
            let p = self.panel(
                ui,
                root,
                o,
                r,
                Look {
                    fill: Some(Token::Card),
                    still: true,
                    ..Look::default()
                },
            );
            keep(self, p);
        }
        if let Some(bar) = lay.sheet_bar {
            let p = self.panel(ui, root, o, bar, status_fill);
            keep(self, p);
        }

        // Title row.
        for pc in &lay.title {
            let id = self.title_item(ui, root, pc);
            keep(self, id);
        }
        let mut qat_n = 0;
        for pc in &lay.qat {
            qat_n += 1;
            let id = self.qat_item(ui, root, pc, qat_n);
            keep(self, id);
        }
        if let Some(t) = &lay.title_text {
            let label = self
                .title()
                .split(" (preview)")
                .next()
                .unwrap_or("")
                .to_owned();
            let id = self.add(
                ui,
                root,
                o,
                t.rect,
                Widget::Label,
                &label,
                Look {
                    content: Content::TextOnly,
                    align: 0.0,
                    pad: 0.0,
                    ..Look::default()
                },
                Act::TitleBar,
            );
            keep(self, id);
        }
        if let Some(s) = &lay.search {
            let look = Look {
                icon: self.icon("search"),
                content: if lay.search_button {
                    Content::IconOnly
                } else {
                    Content::IconBeside
                },
                fill_rgb: Some(self.pal.search_fill),
                border_rgb: Some(self.pal.search_border),
                radius: 4.0,
                align: 0.0,
                pad: 10.0,
                ink: Some(Token::TextDim),
                ..Look::default()
            };
            let id = self.add(
                ui,
                root,
                o,
                s.rect,
                Widget::Button { toggled: None },
                "Search",
                look,
                Act::Cmd("search".into()),
            );
            keep(self, id);
        }

        // Tab row.
        if let Some(f) = &lay.file_tab {
            let id = self.add(
                ui,
                root,
                o,
                f.rect,
                Widget::Button { toggled: None },
                "File",
                Look {
                    content: Content::TextOnly,
                    text_size: Some(14.0),
                    ..Look::default()
                },
                Act::File,
            );
            if let Some(n) = ui.tree.get_mut(id) {
                n.key_tip = Some("F".into());
            }
            keep(self, id);
        }
        self.build_tab_strip(ui, lay);
        if let Some(sc) = lay.tab_scroll {
            let id = self.add(
                ui,
                root,
                o,
                sc,
                Widget::Button { toggled: None },
                "Scroll tabs",
                Look {
                    icon: self.icon("chevron-right-thin"),
                    content: Content::IconOnly,
                    icon_px: 10.0,
                    ..Look::default()
                },
                Act::Inert,
            );
            keep(self, id);
        }
        for pc in &lay.tab_row_right {
            if pc.offscreen {
                continue;
            }
            let id = self.tab_row_item(ui, root, pc);
            keep(self, id);
        }

        // Formula strip.
        if show_fbar && let Some(nb) = lay.name_box_outer {
            let edit = TextEdit::new(&self.name_box);
            let id = self.add(
                ui,
                root,
                o,
                nb,
                Widget::Combo {
                    edit,
                    items: vec!["A1".into(), "Print_Area".into()],
                    open: false,
                },
                "Name Box",
                Look::default(),
                Act::NameBox,
            );
            self.roles.name_box = Some(id);
            keep(self, id);
        }
        if show_fbar && let Some(g) = lay.grip {
            let id = self.add(
                ui,
                root,
                o,
                g,
                Widget::Button { toggled: None },
                "",
                Look {
                    icon: self.icon("dots-vertical"),
                    content: Content::IconOnly,
                    icon_px: 12.0,
                    still: true,
                    ink: Some(Token::TextDim),
                    ..Look::default()
                },
                Act::Inert,
            );
            keep(self, id);
        }
        if show_fbar && let Some(fx) = lay.fx_group {
            let third = fx.w / 3;
            let parts = [
                ("Cancel", "x", 0),
                ("Enter", "check", 1),
                ("Insert Function", "math-function", 2),
            ];
            for (label, icon, i) in parts {
                let r = PxRect {
                    x: fx.x + third * i,
                    w: if i == 2 { fx.w - 2 * third } else { third },
                    ..fx
                };
                let id = self.add(
                    ui,
                    root,
                    o,
                    r,
                    Widget::Button { toggled: None },
                    label,
                    Look {
                        icon: self.icon(icon),
                        content: Content::IconOnly,
                        icon_px: 14.0,
                        ink: if i < 2 { Some(Token::TextDim) } else { None },
                        ..Look::default()
                    },
                    Act::Cmd(format!(
                        "formula.{}",
                        label.to_lowercase().replace(' ', "-")
                    )),
                );
                keep(self, id);
            }
        }
        if show_fbar && let Some(ff) = lay.formula_field {
            let id = self.add(
                ui,
                root,
                o,
                ff,
                Widget::TextField(TextEdit::new(&self.formula)),
                "Formula Bar",
                Look::default(),
                Act::Formula,
            );
            self.roles.formula = Some(id);
            keep(self, id);
            let btn = PxRect {
                x: ff.right() - self.px(26),
                y: ff.y + self.px(2),
                w: self.px(24),
                h: self.px(24),
            };
            let id = self.add(
                ui,
                root,
                o,
                btn,
                Widget::Button { toggled: None },
                if self.formula_expanded {
                    "Collapse Formula Bar"
                } else {
                    "Expand Formula Bar"
                },
                Look {
                    icon: self.icon(if self.formula_expanded {
                        "chevron-up"
                    } else {
                        "chevron-down"
                    }),
                    content: Content::IconOnly,
                    icon_px: 12.0,
                    ..Look::default()
                },
                Act::ExpandFormula,
            );
            keep(self, id);
        }

        // Sheet tabs.
        for pc in &lay.sheet_items {
            let id = self.sheet_item(ui, root, pc);
            keep(self, id);
        }

        // Status bar.
        for pc in &lay.status_items {
            let id = self.status_item(ui, root, pc);
            keep(self, id);
        }
    }

    fn title_item(&mut self, ui: &mut Ui, root: NodeId, pc: &PlacedChrome<'_>) -> NodeId {
        let c = pc.chrome;
        match c.kind {
            Kind::Caption => {
                let (icon, tail) = match c.id.as_str() {
                    "caption.minimise" => ("cap-min", "minimise"),
                    "caption.maximise" => ("cap-max", "maximise"),
                    "caption.restore-down" => ("cap-restore", "maximise"),
                    _ => ("cap-close", "close"),
                };
                let icon = if tail == "maximise" && self.maximized {
                    "cap-restore"
                } else {
                    icon
                };
                let look = Look {
                    icon: self.icon(icon),
                    content: Content::IconOnly,
                    icon_px: 10.0,
                    hover_rgb: (tail == "close").then_some(self.pal.close_hover),
                    ..Look::default()
                };
                self.add(
                    ui,
                    root,
                    (0, 0),
                    pc.rect,
                    Widget::Button { toggled: None },
                    &c.label,
                    look,
                    Act::Caption(tail.into()),
                )
            }
            _ => self.add(
                ui,
                root,
                (0, 0),
                pc.rect,
                Widget::Button { toggled: None },
                "Account",
                Look {
                    icon: self.icon("user-circle"),
                    content: Content::IconOnly,
                    icon_px: 24.0,
                    ..Look::default()
                },
                Act::Inert,
            ),
        }
    }

    fn qat_item(&mut self, ui: &mut Ui, root: NodeId, pc: &PlacedChrome<'_>, n: usize) -> NodeId {
        let c = pc.chrome;
        let tip = if n <= 9 { Some(n.to_string()) } else { None };
        let id = match c.kind {
            Kind::Split => {
                let aw = c.arrow_w.unwrap_or(16);
                let look = Look {
                    icon: c.icon.as_deref().and_then(|i| self.icon(i)),
                    content: Content::IconOnly,
                    ..Look::default()
                };
                let id = self.add(
                    ui,
                    root,
                    (0, 0),
                    pc.rect,
                    Widget::SplitButton {
                        arrow_w: aw as f32 / REF_SCALE,
                    },
                    &c.label,
                    look,
                    Act::Cmd(c.id.clone()),
                );
                if !c.enabled
                    && let Some(n) = ui.tree.get_mut(id)
                {
                    n.enabled = false;
                }
                id
            }
            Kind::Dropdown => self.add(
                ui,
                root,
                (0, 0),
                pc.rect,
                Widget::Button { toggled: None },
                &c.label,
                Look {
                    icon: self.icon("chevron-down"),
                    content: Content::IconOnly,
                    icon_px: 12.0,
                    ..Look::default()
                },
                Act::Menu(c.id.clone()),
            ),
            _ => {
                let autosave = c.id.contains("autosave");
                let label = if autosave {
                    "AutoSave"
                } else {
                    c.label.as_str()
                };
                self.add(
                    ui,
                    root,
                    (0, 0),
                    pc.rect,
                    Widget::Button { toggled: None },
                    label,
                    Look {
                        icon: c.icon.as_deref().and_then(|i| self.icon(i)),
                        content: if c.icon.is_some() {
                            Content::IconOnly
                        } else {
                            Content::TextOnly
                        },
                        switch: autosave.then_some(false),
                        ..Look::default()
                    },
                    Act::Cmd(c.id.clone()),
                )
            }
        };
        if let Some(t) = tip
            && let Some(n) = ui.tree.get_mut(id)
        {
            n.key_tip = Some(t);
        }
        id
    }

    fn build_tab_strip(&mut self, ui: &mut Ui, lay: &layout::Layout<'_>) {
        let shown: Vec<usize> = (0..lay.tab_rects.len())
            .filter(|i| !lay.tab_offscreen.get(*i).copied().unwrap_or(false))
            .collect();
        let (Some(&first), Some(&last)) = (shown.first(), shown.last()) else {
            return;
        };
        let s = self.scale;
        let spec = self.spec.clone();
        let tabs: Vec<TabItem> = shown
            .iter()
            .map(|&i| {
                let r = lay.tab_rects[i];
                let w = match shown
                    .iter()
                    .position(|x| *x == i)
                    .and_then(|p| shown.get(p + 1))
                {
                    Some(&next) => (lay.tab_rects[next].x - r.x) as f32 / s,
                    None => r.w as f32 / s,
                };
                let mut t = TabItem::new(&spec.tabs[i].label, w);
                t.key_tip = spec.tabs[i].key.clone();
                t
            })
            .collect();
        let r0 = lay.tab_rects[first];
        let rl = lay.tab_rects[last];
        let bounds = PxRect {
            x: r0.x,
            y: r0.y,
            w: rl.right() - r0.x,
            h: r0.h,
        };
        let root = ui.tree.root();
        let id = self.add(
            ui,
            root,
            (0, 0),
            bounds,
            Widget::TabStrip {
                tabs,
                selected: self.tab.min(shown.len() - 1),
                gap: 0.0,
            },
            "Ribbon tabs",
            Look {
                text_size: Some(14.0),
                ..Look::default()
            },
            Act::Tab,
        );
        self.roles.strip = Some(id);
        self.static_ids.push(id);
    }

    fn tab_row_item(&mut self, ui: &mut Ui, root: NodeId, pc: &PlacedChrome<'_>) -> NodeId {
        let c = pc.chrome;
        let compact = pc.compact;
        if c.id.contains("display-options") {
            return self.add(
                ui,
                root,
                (0, 0),
                pc.rect,
                Widget::Button { toggled: None },
                "Ribbon Display Options",
                Look {
                    icon: self.icon(if self.ribbon_open {
                        "chevron-up"
                    } else {
                        "chevron-down"
                    }),
                    content: Content::IconOnly,
                    icon_px: 12.0,
                    ..Look::default()
                },
                Act::ToggleRibbon,
            );
        }
        match c.kind {
            Kind::Combo => self.add(
                ui,
                root,
                (0, 0),
                pc.rect,
                Widget::Dropdown {
                    items: menus::combo_items("editing"),
                    selected: 0,
                    open: false,
                },
                &c.label,
                Look::default(),
                Act::Cmd(c.id.clone()),
            ),
            _ if c.id == "share" => {
                let look = Look {
                    icon: self.icon("share"),
                    content: if compact {
                        Content::IconOnly
                    } else {
                        Content::IconBeside
                    },
                    fill_rgb: Some(self.pal.share),
                    ink_rgb: Some(self.pal.on_accent),
                    hover_rgb: Some(self.pal.accent),
                    radius: 4.0,
                    chevron: !compact,
                    ..Look::default()
                };
                self.add(
                    ui,
                    root,
                    (0, 0),
                    pc.rect,
                    Widget::Button { toggled: None },
                    &c.label,
                    look,
                    Act::Menu("share".into()),
                )
            }
            _ => {
                let look = Look {
                    icon: self.icon("message"),
                    content: if compact {
                        Content::IconOnly
                    } else {
                        Content::IconBeside
                    },
                    border: Some(Token::FieldBorder),
                    radius: 4.0,
                    ..Look::default()
                };
                self.add(
                    ui,
                    root,
                    (0, 0),
                    pc.rect,
                    Widget::Button { toggled: None },
                    &c.label,
                    look,
                    Act::Cmd(c.id.clone()),
                )
            }
        }
    }

    fn sheet_item(&mut self, ui: &mut Ui, root: NodeId, pc: &PlacedChrome<'_>) -> NodeId {
        let c = pc.chrome;
        let (icon, active) = match c.id.as_str() {
            "sheettab.scroll-left" => (Some("chevron-left"), false),
            "sheettab.scroll-right" => (Some("chevron-right-thin"), false),
            "sheettab.add-sheet" => (Some("plus"), false),
            _ => (None, true),
        };
        let look = if active {
            Look {
                content: Content::TextOnly,
                fill_rgb: Some(self.pal.tab_active),
                ink_rgb: Some(self.pal.tab_active_text),
                hover_rgb: Some(self.pal.tab_active),
                bar: Some((self.pal.grid_accent, 2.0)),
                ..Look::default()
            }
        } else {
            Look {
                icon: icon.and_then(|i| self.icon(i)),
                content: Content::IconOnly,
                icon_px: if icon == Some("plus") { 14.0 } else { 10.0 },
                ..Look::default()
            }
        };
        let id = self.add(
            ui,
            root,
            (0, 0),
            pc.rect,
            Widget::Button { toggled: None },
            if active { "Sheet1" } else { &c.label },
            look,
            Act::Cmd(c.id.clone()),
        );
        if !c.enabled
            && let Some(n) = ui.tree.get_mut(id)
        {
            n.enabled = false;
        }
        id
    }

    fn status_text(&self, id: &str) -> Option<&'static str> {
        Some(match id {
            "status.mode" => "Ready",
            "status.accessibility" => "Accessibility: Good to go",
            "status.page" => "Page 1 of 1",
            "status.words" => "0 words",
            "status.language" => "English (United Kingdom)",
            "status.focus" => "Focus",
            _ => return None,
        })
    }

    fn status_item(&mut self, ui: &mut Ui, root: NodeId, pc: &PlacedChrome<'_>) -> NodeId {
        let c = pc.chrome;
        let status_px = Some(10.8);
        if c.kind == Kind::Slider {
            let id = self.add(
                ui,
                root,
                (0, 0),
                pc.rect,
                Widget::Slider {
                    value: self.zoom,
                    min: 10.0,
                    max: 400.0,
                    step: 10.0,
                },
                "Zoom",
                Look::default(),
                Act::Zoom,
            );
            return id;
        }
        let icon = match c.id.as_str() {
            "status.normal" => Some("layout-grid"),
            "status.page-layout" | "status.print-layout" => Some("file"),
            "status.page-break" | "status.web-layout" => Some("world"),
            "status.read" => Some("book"),
            "status.zoom-out" => Some("minus"),
            "status.zoom-in" => Some("plus"),
            "status.grip" => None,
            _ => None,
        };
        let text = self
            .status_text(&c.id)
            .map(str::to_owned)
            .or_else(|| (c.id == "status.zoom-percent").then(|| format!("{}%", self.zoom as i32)));
        let act = match c.id.as_str() {
            "status.zoom-out" => Act::ZoomStep(-10),
            "status.zoom-in" => Act::ZoomStep(10),
            "status.zoom-percent" => Act::ZoomLabel,
            _ => Act::Cmd(c.id.clone()),
        };
        let (label, content) = match (&text, icon) {
            (Some(t), _) => (t.clone(), Content::TextOnly),
            (None, Some(_)) => (c.label.clone(), Content::IconOnly),
            (None, None) => (String::new(), Content::TextOnly),
        };
        let id = self.add(
            ui,
            root,
            (0, 0),
            pc.rect,
            Widget::Button { toggled: None },
            &label,
            Look {
                icon: icon.and_then(|i| self.icon(i)),
                content,
                icon_px: 14.0,
                text_size: status_px,
                still: c.id == "status.grip",
                ..Look::default()
            },
            act,
        );
        if c.id == "status.zoom-percent" {
            self.roles.zoom_label = Some(id);
        }
        id
    }

    // ---- ribbon body and overlays ------------------------------------------------------------

    fn build_body_with(&mut self, ui: &mut Ui, lay: &layout::Layout<'_>) {
        let root = ui.tree.root();
        let mut nodes: HashMap<String, NodeId> = HashMap::new();
        self.roles.group_popup = None;
        if self.ribbon_open && lay.lower.h > 0 {
            let clip_w = lay
                .ribbon_scroll
                .map_or(lay.lower.w, |s| (s.button.x - lay.lower.x).max(0));
            let clip_rect = PxRect {
                w: clip_w,
                ..lay.lower
            };
            let clip = self.panel(ui, root, (0, 0), clip_rect, self.static_look());
            if let Some(n) = ui.tree.get_mut(clip) {
                n.clip_children = true;
            }
            self.body_ids.push(clip);
            let origin = (clip_rect.x, clip_rect.y);
            for (gi, pg) in lay.groups.iter().enumerate() {
                self.build_group(ui, clip, origin, gi, pg, &mut nodes);
                // The hairline between groups.
                let sep = PxRect {
                    x: pg.rect.right() - 1,
                    y: pg.rect.y + self.px(4),
                    w: 1,
                    h: pg.rect.h - self.px(8),
                };
                let id = self.panel(
                    ui,
                    clip,
                    origin,
                    sep,
                    Look {
                        fill: Some(Token::Separator),
                        still: true,
                        ..Look::default()
                    },
                );
                let _ = id;
            }
            if let Some(sc) = lay.ribbon_scroll {
                let id = self.add(
                    ui,
                    root,
                    (0, 0),
                    sc.button,
                    Widget::Button { toggled: None },
                    "Scroll ribbon",
                    Look {
                        icon: self.icon("chevron-right-thin"),
                        content: Content::IconOnly,
                        icon_px: 10.0,
                        ..Look::default()
                    },
                    Act::ScrollRibbon,
                );
                self.body_ids.push(id);
            }
        }
        // Overlays, topmost last: the collapsed-group popup, Backstage, then the shared menu.
        if let Some(gi) = self.group_popup
            && let Some(pg) = lay.groups.get(gi)
            && let Some(btn) = pg.button
        {
            self.build_group_popup(ui, pg, btn, &mut nodes);
        }
        if let Some(sel) = self.backstage {
            self.build_backstage(ui, sel);
        }
        if self.share.is_some() {
            self.build_share(ui);
        }
        let menu = Node::new(
            Widget::Menu(MenuState::new(Vec::new())),
            Rect::new(0.0, 0.0, 240.0, 0.0),
        );
        let mid = ui.add(root, menu);
        if let Some(n) = ui.tree.get_mut(mid) {
            n.visible = false;
        }
        self.roles.menu = Some(mid);
        self.body_ids.push(mid);
        ui.set_tip_nodes(nodes);
    }

    fn group_scale_is_small(g: &PlacedGroup<'_>) -> bool {
        g.scale == GroupScale::Small
    }

    fn build_group(
        &mut self,
        ui: &mut Ui,
        parent: NodeId,
        origin: (i32, i32),
        gi: usize,
        pg: &PlacedGroup<'_>,
        nodes: &mut HashMap<String, NodeId>,
    ) {
        let g = pg.group;
        if let Some(btn) = pg.button {
            let id = self.add(
                ui,
                parent,
                origin,
                btn,
                Widget::Button { toggled: None },
                &g.label,
                Look {
                    icon: g.icon.as_deref().and_then(|i| self.icon(i)),
                    content: Content::IconAbove,
                    icon_px: 32.0,
                    chevron: true,
                    ..Look::default()
                },
                Act::GroupButton(gi),
            );
            nodes.insert(format!("popup:{}", g.id), id);
            return;
        }
        for pc in &pg.controls {
            self.add_control(
                ui,
                parent,
                origin,
                pc,
                Self::group_scale_is_small(pg),
                nodes,
            );
        }
        let lab = self.add(
            ui,
            parent,
            origin,
            pg.label,
            Widget::Label,
            &g.label,
            Look {
                content: Content::TextOnly,
                ink: Some(Token::TextDim),
                text_size: Some(11.0),
                ..Look::default()
            },
            Act::Inert,
        );
        let _ = lab;
        if let Some(l) = pg.launcher {
            let id = self.add(
                ui,
                parent,
                origin,
                l,
                Widget::Button { toggled: None },
                &format!("{} dialog launcher", g.label),
                Look {
                    icon: self.icon("launcher"),
                    content: Content::IconOnly,
                    icon_px: 8.0,
                    ink: Some(Token::TextDim),
                    ..Look::default()
                },
                Act::Launcher(g.id.clone()),
            );
            nodes.insert(format!("launcher:{}", g.id), id);
        }
    }

    fn build_group_popup(
        &mut self,
        ui: &mut Ui,
        pg: &PlacedGroup<'_>,
        btn: PxRect,
        nodes: &mut HashMap<String, NodeId>,
    ) {
        let spec = self.spec.clone();
        let full = layout::popup_group(&spec, pg.group, self.scale);
        let at = PxRect {
            x: btn.x.min((self.win_px.0 - full.rect.w).max(0)),
            y: btn.bottom(),
            w: full.rect.w,
            h: full.rect.h,
        };
        let root = ui.tree.root();
        let panel = self.panel(
            ui,
            root,
            (0, 0),
            at,
            Look {
                fill: Some(Token::MenuBg),
                border: Some(Token::MenuBorder),
                ..Look::default()
            },
        );
        self.roles.group_popup = Some(panel);
        self.body_ids.push(panel);
        for pc in &full.controls {
            let mut shifted = pc.clone();
            for r in [&mut shifted.rect, &mut shifted.main, &mut shifted.arrow] {
                r.x += at.x;
                r.y += at.y;
            }
            self.add_control(ui, panel, (at.x, at.y), &shifted, false, nodes);
        }
        self.add(
            ui,
            panel,
            (at.x, at.y),
            PxRect {
                x: at.x + full.label.x,
                y: at.y + full.label.y,
                ..full.label
            },
            Widget::Label,
            &pg.group.label,
            Look {
                content: Content::TextOnly,
                ink: Some(Token::TextDim),
                text_size: Some(11.0),
                ..Look::default()
            },
            Act::Inert,
        );
    }

    fn control_look(&self, c: &Control, small_scale: bool) -> Look {
        let large = c.size == Size::Large;
        let icon = c.icon.as_deref().and_then(|n| self.icon(n));
        let content = if large {
            Content::IconAbove
        } else if icon.is_none() {
            Content::TextOnly
        } else if c.w <= 44 || small_scale {
            Content::IconOnly
        } else {
            Content::IconBeside
        };
        Look {
            icon,
            content,
            icon_px: if large { 32.0 } else { 16.0 },
            align: if content == Content::IconBeside {
                0.0
            } else {
                0.5
            },
            ..Look::default()
        }
    }

    fn add_control(
        &mut self,
        ui: &mut Ui,
        parent: NodeId,
        origin: (i32, i32),
        pc: &PlacedControl<'_>,
        small_scale: bool,
        nodes: &mut HashMap<String, NodeId>,
    ) {
        let c = pc.control;
        let tail = c.id.rsplit('.').next().unwrap_or(&c.id).to_owned();
        let act = Act::Cmd(c.id.clone());
        let s = self.scale;
        let mut look = self.control_look(c, small_scale);
        let id = match c.kind {
            Kind::Split => {
                let large = c.size == Size::Large;
                look.arrow_below = large;
                let arrow = if large { pc.arrow.h } else { pc.arrow.w } as f32 / s;
                if !large && look.content == Content::IconBeside && pc.main.w < self.px(44) {
                    look.content = Content::IconOnly;
                }
                self.add(
                    ui,
                    parent,
                    origin,
                    pc.rect,
                    Widget::SplitButton { arrow_w: arrow },
                    &c.label,
                    look,
                    act,
                )
            }
            Kind::Dropdown => match &c.value {
                Some(v) if c.size != Size::Large => {
                    let shown = if tail == "number-format" {
                        "General"
                    } else {
                        v
                    };
                    self.add(
                        ui,
                        parent,
                        origin,
                        pc.rect,
                        Widget::Dropdown {
                            items: vec![
                                shown.to_owned(),
                                format!("{shown} 2"),
                                format!("{shown} 3"),
                            ],
                            selected: 0,
                            open: false,
                        },
                        &c.label,
                        Look::default(),
                        act,
                    )
                }
                _ => {
                    look.chevron = true;
                    // Office shows the label only when icon, label and chevron fit; Fill, Clear and
                    // Orientation (50 u wide) are icon and chevron only.
                    // Measured at the size the face paints at, so a label that is shown is whole.
                    let label_w = ui.measure(&c.label);
                    let chrome_w = (self.px(4 * 2 + 16 + 4 + 12)) as f32;
                    let fits = label_w + chrome_w <= pc.rect.w as f32;
                    if look.content == Content::IconBeside && !fits {
                        look.content = Content::IconOnly;
                    } else if look.content == Content::IconOnly && pc.rect.w >= self.px(40) && fits
                    {
                        look.content = Content::IconBeside;
                    }
                    self.add(
                        ui,
                        parent,
                        origin,
                        pc.rect,
                        Widget::Button { toggled: None },
                        &c.label,
                        look,
                        Act::Menu(c.id.clone()),
                    )
                }
            },
            Kind::Combo => {
                let value = match tail.as_str() {
                    "font" => "Calibri".to_owned(),
                    "font-size" => "11".to_owned(),
                    _ => c.value.clone().unwrap_or_default(),
                };
                self.add(
                    ui,
                    parent,
                    origin,
                    pc.rect,
                    Widget::Combo {
                        edit: TextEdit::new(&value),
                        items: menus::combo_items(&c.id),
                        open: false,
                    },
                    &c.label,
                    Look {
                        // Office's lists: font names 33 u rows, sizes 30 u (menu rows 36 u).
                        list_row: match tail.as_str() {
                            "font" => open365_ui_core::LIST_ROW_FONT_H,
                            "font-size" => open365_ui_core::LIST_ROW_SIZE_H,
                            _ => 0.0,
                        },
                        ..Look::default()
                    },
                    act,
                )
            }
            Kind::Spinner => self.add(
                ui,
                parent,
                origin,
                pc.rect,
                Widget::Spinner {
                    value: 1.0,
                    min: 0.0,
                    max: 100.0,
                    step: 0.5,
                    edit: TextEdit::new("1"),
                },
                &c.label,
                Look::default(),
                act,
            ),
            Kind::Edit => self.add(
                ui,
                parent,
                origin,
                pc.rect,
                Widget::TextField(TextEdit::new("")),
                &c.label,
                Look::default(),
                act,
            ),
            Kind::Checkbox => self.add(
                ui,
                parent,
                origin,
                pc.rect,
                Widget::Checkbox { checked: false },
                &c.label,
                Look::default(),
                act,
            ),
            Kind::Gallery => {
                let id = self.add(
                    ui,
                    parent,
                    origin,
                    pc.rect,
                    Widget::Panel,
                    &c.label,
                    Look {
                        fill: Some(Token::Field),
                        border: Some(Token::FieldBorder),
                        still: true,
                        ..Look::default()
                    },
                    Act::Inert,
                );
                self.gallery_cells(ui, id, origin, pc.rect, &c.id);
                id
            }
            _ => {
                let toggled = (TOGGLE_IDS.contains(&tail.as_str())
                    || (self.kind == AppKind::Writer
                        && matches!(
                            tail.as_str(),
                            "align-left" | "centre" | "align-right" | "justify"
                        )))
                .then_some(false);
                self.add(
                    ui,
                    parent,
                    origin,
                    pc.rect,
                    Widget::Button { toggled },
                    &c.label,
                    look,
                    act,
                )
            }
        };
        if !c.enabled
            && let Some(n) = ui.tree.get_mut(id)
        {
            n.enabled = false;
        }
        nodes.insert(format!("cmd:{}", c.id), id);
    }

    /// Sample cells of a gallery (styles, themes) and its scroll strip.
    fn gallery_cells(
        &mut self,
        ui: &mut Ui,
        gallery: NodeId,
        origin: (i32, i32),
        r: PxRect,
        gallery_id: &str,
    ) {
        let strip = self.px(18);
        let cells = 4;
        let cell_w = ((r.w - strip - self.px(4)) / cells).max(1);
        // The Writer style gallery applies the built-in styles; other galleries are samples.
        const STYLES: [(&str, &str); 4] = [
            ("normal", "Normal"),
            ("no-spacing", "No Spacing"),
            ("heading-1", "Heading 1"),
            ("heading-2", "Heading 2"),
        ];
        let styles =
            self.kind == AppKind::Writer && (gallery_id.ends_with("styles.gallery") || r.w < 700);
        for i in 0..cells {
            let cell = PxRect {
                x: r.x + self.px(2) + i * cell_w,
                y: r.y + self.px(2),
                w: cell_w - self.px(2),
                h: r.h - self.px(4),
            };
            let style = styles.then(|| STYLES[i as usize]);
            let id = self.add(
                ui,
                gallery,
                origin,
                cell,
                Widget::Button { toggled: None },
                "AaBbCc",
                Look {
                    content: Content::TextOnly,
                    ..Look::default()
                },
                match style {
                    Some((slug, _)) => Act::Cmd(format!("home.styles.{slug}")),
                    None => Act::Inert,
                },
            );
            if let (Some((_, name)), Some(n)) = (style, ui.tree.get_mut(id)) {
                n.tooltip = Some(name.to_owned());
            }
        }
        let third = (r.h - self.px(4)) / 3;
        for (i, icon) in ["chevron-up", "chevron-down", "dots"].iter().enumerate() {
            let b = PxRect {
                x: r.right() - strip - self.px(1),
                y: r.y + self.px(2) + third * i as i32,
                w: strip,
                h: third,
            };
            self.add(
                ui,
                gallery,
                origin,
                b,
                Widget::Button { toggled: None },
                "",
                Look {
                    icon: self.icon(icon),
                    content: Content::IconOnly,
                    icon_px: 10.0,
                    ..Look::default()
                },
                Act::Inert,
            );
        }
    }

    // ---- Backstage -------------------------------------------------------------------------

    fn build_backstage(&mut self, ui: &mut Ui, sel: usize) {
        let spec = self.spec.clone();
        let root = ui.tree.root();
        let top = self.px(spec.file_tab.y);
        let area = PxRect {
            x: 0,
            y: top,
            w: self.win_px.0,
            h: self.win_px.1 - top,
        };
        let view = self.panel(
            ui,
            root,
            (0, 0),
            area,
            Look {
                fill_rgb: Some(self.pal.backstage),
                still: true,
                ..Look::default()
            },
        );
        self.body_ids.push(view);
        let o = (0, 0);
        let nav_w = self
            .px(spec.backstage.nav_x * 2 + spec.backstage.nav_w)
            .min(self.win_px.0);
        let nav = PxRect { w: nav_w, ..area };
        self.panel(
            ui,
            view,
            o,
            nav,
            Look {
                fill_rgb: Some(self.pal.backstage_nav),
                still: true,
                ..Look::default()
            },
        );
        for (i, item) in spec.backstage.nav.iter().enumerate() {
            let r = PxRect {
                x: self.px(spec.backstage.nav_x),
                y: self.px(item.y),
                w: self.px(item.w),
                h: self.px(item.h),
            };
            let icon = match item.id.as_str() {
                "back" => "arrow-left",
                "home" => "home",
                "new" => "file-plus",
                "open" => "folder-open",
                "info" => "info-circle",
                "save" => "device-floppy",
                "saveas" | "save-as" => "file-export",
                "print" => "printer",
                "share" => "share",
                "export" => "file-download",
                "close" => "door-exit",
                "account" => "user-circle",
                "options" => "settings",
                _ => "file",
            };
            self.add(
                ui,
                view,
                o,
                r,
                Widget::Button { toggled: None },
                &item.label,
                Look {
                    icon: self.icon(icon),
                    content: Content::IconBeside,
                    icon_px: 18.0,
                    align: 0.0,
                    pad: 12.0,
                    text_size: Some(14.0),
                    fill: (i == sel && item.id != "back").then_some(Token::Checked),
                    ..Look::default()
                },
                Act::Nav(i),
            );
        }
        let label = spec
            .backstage
            .nav
            .get(sel)
            .map_or("Home", |n| n.label.as_str())
            .to_owned();
        let heading = PxRect {
            x: nav_w + self.px(40),
            y: top + self.px(40),
            w: self.px(600),
            h: self.px(44),
        };
        self.add(
            ui,
            view,
            o,
            heading,
            Widget::Label,
            &label,
            Look {
                content: Content::TextOnly,
                align: 0.0,
                pad: 0.0,
                text_size: Some(22.0),
                ..Look::default()
            },
            Act::Inert,
        );
        let blank = match self.kind {
            AppKind::Writer => "Blank document",
            AppKind::Sheets => "Blank workbook",
        };
        for (i, name) in [blank, "Take a tour", "Recent: Quarterly report"]
            .iter()
            .enumerate()
        {
            let tile = PxRect {
                x: nav_w + self.px(40) + i as i32 * self.px(190),
                y: top + self.px(110),
                w: self.px(170),
                h: self.px(140),
            };
            self.add(
                ui,
                view,
                o,
                tile,
                Widget::Button { toggled: None },
                name,
                Look {
                    icon: self.icon(if i == 0 { "file-plus" } else { "file-text" }),
                    content: Content::IconAbove,
                    icon_px: 40.0,
                    icon_top: 22.0,
                    border: Some(Token::FieldBorder),
                    ..Look::default()
                },
                Act::Cmd(format!("backstage.{i}")),
            );
        }
    }

    // ---- menus -----------------------------------------------------------------------------

    /// Menu icons (Tabler) by the tail of the command id.
    fn give_icons(&self, items: &mut [open365_ui_core::MenuItem]) {
        for it in items {
            let tail = it.id.rsplit('.').next().unwrap_or(&it.id);
            let name = menu_icon_name(tail);
            if it.icon.is_none() {
                it.icon = name.and_then(|n| self.icon(n));
            }
            self.give_icons(&mut it.submenu);
        }
    }

    fn open_menu(&mut self, ui: &mut Ui, owner: NodeId, id: &str) {
        let Some(menu) = self.roles.menu else { return };
        let label = ui
            .tree
            .get(owner)
            .map(|n| n.label.clone())
            .unwrap_or_default();
        let mut items = if id == "home.styles.cell-styles" {
            self.sheet
                .home_style_names()
                .iter()
                .map(|n| open365_ui_core::MenuItem::new(&format!("{id}.{n}"), n))
                .collect()
        } else {
            menus::items_for(id, &label)
        };
        self.give_icons(&mut items);
        let widest = items
            .iter()
            .map(|i| ui.measure(&i.label) / ui.scale.0)
            .fold(0.0f32, f32::max);
        let width = (widest + 56.0).max(160.0);
        let height = MenuState::panel_height(&items);
        let r = ui.tree.abs_rect(owner);
        let win = Point::new(
            self.win_px.0 as f32 / self.scale,
            self.win_px.1 as f32 / self.scale,
        );
        let x = r.x.min((win.x - width * 2.0).max(0.0));
        let y = if r.bottom() + height > win.y {
            (r.y - height).max(0.0)
        } else {
            r.bottom()
        };
        if let Some(n) = ui.tree.get_mut(menu) {
            n.widget = Widget::Menu(MenuState::new(items));
            n.bounds.w = width;
            n.bounds.h = height;
        }
        ui.open_menu(menu, Point::new(x, y));
    }

    fn close_overlays(&mut self, ui: &mut Ui) -> bool {
        let mut closed = false;
        if self.group_popup.take().is_some() {
            closed = true;
        }
        if self.backstage.take().is_some() {
            closed = true;
        }
        if self.close_share() {
            closed = true;
        }
        if closed {
            self.build_body(ui);
        }
        closed
    }

    /// Escape: closes the group popup or Backstage. False when an open menu or list owns it.
    pub fn escape(&mut self, ui: &mut Ui) -> bool {
        let menu_open = self
            .roles
            .menu
            .and_then(|m| ui.tree.get(m))
            .is_some_and(|n| matches!(&n.widget, Widget::Menu(m) if m.open));
        if menu_open || ui.keytips.is_showing() {
            return false;
        }
        self.close_overlays(ui)
    }

    /// True when a pointer press at `p` (logical) should close the group popup.
    pub fn press_outside_popup(&self, ui: &Ui, p: Point) -> bool {
        let Some(panel) = self.roles.group_popup else {
            return false;
        };
        match ui.tree.hit_test(p) {
            None => true,
            Some(mut n) => loop {
                if n == panel {
                    return false;
                }
                match ui.tree.get(n).and_then(|x| x.parent) {
                    Some(p) => n = p,
                    None => return true,
                }
            },
        }
    }

    pub fn close_group_popup(&mut self, ui: &mut Ui) {
        if self.group_popup.take().is_some() {
            self.build_body(ui);
        }
    }

    /// True when the title row is under `p` and no control is: dragging there moves the window.
    pub fn is_drag_area(&self, ui: &Ui, p: Point) -> bool {
        let title_h = self.px(self.spec.file_tab.y) as f32 / self.scale;
        if p.y >= title_h {
            return false;
        }
        match ui.tree.hit_test(p) {
            None => true,
            Some(n) => matches!(
                self.acts.get(&n),
                Some(Act::TitleBar) | Some(Act::Inert) | None
            ),
        }
    }

    // ---- events ----------------------------------------------------------------------------

    pub fn handle(&mut self, ui: &mut Ui, events: Vec<Emit>) -> Vec<WinAct> {
        let mut out = Vec::new();
        for ev in events {
            if self.dialog_handle(ui, &ev) {
                continue;
            }
            if self.kind == AppKind::Sheets && self.grid_event(ui, &ev) {
                continue;
            }
            if let Emit::KeyPressed(n, Key::Char(c), m) = &ev
                && m.ctrl
                && matches!(c.to_ascii_uppercase(), 'Z' | 'Y')
                && self.writer.as_ref().is_some_and(|p| p.canvas() == Some(*n))
            {
                self.history(ui, c.eq_ignore_ascii_case(&'Z'));
                continue;
            }
            if let Emit::KeyPressed(_, Key::Char(c), m) = &ev
                && m.ctrl
                && !m.alt
                && !m.shift
                && c.eq_ignore_ascii_case(&'f')
                && let Some(p) = self.writer.as_mut()
            {
                p.find_open();
                self.refocus_doc = true;
                continue;
            }
            if let Some(p) = self.writer.as_mut() {
                let consumed = match &ev {
                    Emit::CanvasAction(n, req) if Some(*n) == p.canvas() => {
                        p.accessibility_action(req);
                        true
                    }
                    _ => p.handle(ui, &ev),
                };
                if let Some(z) = p.take_zoom_change() {
                    self.set_zoom(ui, z);
                }
                if consumed {
                    continue;
                }
            }
            match ev {
                Emit::TabSelected(_, i) => {
                    if !self.ribbon_open {
                        self.ribbon_open = true;
                        self.tab = i;
                        self.build_all(ui);
                    } else {
                        self.select_tab(ui, i);
                    }
                }
                Emit::Clicked(n) | Emit::Toggled(n, _) | Emit::KeyTip(n) => {
                    self.activate(ui, n, &mut out);
                }
                Emit::MenuRequested(n) => {
                    if let Some(Act::Cmd(id) | Act::Menu(id)) = self.acts.get(&n).cloned() {
                        if id == "share" {
                            self.open_share(ui);
                            continue;
                        }
                        self.open_menu(ui, n, &id);
                    }
                }
                Emit::MenuCommand(_, id) if id.starts_with("share.") => self.open_share(ui),
                Emit::MenuCommand(_, id) => self.run(ui, &id, &mut out),
                Emit::TextChanged(n, t) => {
                    if Some(n) == self.roles.formula {
                        self.formula = t;
                    } else if Some(n) == self.roles.name_box {
                        self.name_box = t;
                    }
                }
                Emit::TextCommitted(n, t) => {
                    if Some(n) == self.roles.name_box {
                        self.name_box = t.clone();
                        self.run(ui, &format!("namebox.{t}"), &mut out);
                    } else if Some(n) == self.roles.formula {
                        self.formula = t;
                        self.run(ui, "formula.enter", &mut out);
                    } else if let Some(Act::Cmd(id)) = self.acts.get(&n).cloned() {
                        self.run(ui, &format!("{id}={t}"), &mut out);
                    }
                }
                Emit::ValueChanged(n, v) => {
                    if matches!(self.acts.get(&n), Some(Act::Zoom)) {
                        self.set_zoom(ui, v as f32);
                    }
                }
                Emit::Selected(n, i) => {
                    if let Some(Act::Cmd(id)) = self.acts.get(&n) {
                        let id = format!("{id}#{i}");
                        self.run(ui, &id, &mut out);
                    }
                }
                Emit::ScrollChanged(..)
                | Emit::KeyPressed(..)
                | Emit::CanvasText(..)
                | Emit::CanvasIme(..)
                | Emit::CanvasWheel { .. }
                | Emit::CanvasDrag { .. }
                | Emit::CanvasButton { .. }
                | Emit::CanvasRelease { .. }
                | Emit::CanvasAction(..)
                | Emit::CanvasPointer { .. } => {}
            }
        }
        self.sync_writer_state(ui);
        self.sync_history(ui);
        out
    }

    /// Undo (`true`) or redo through the command history, in both apps.
    pub fn history(&mut self, ui: &mut Ui, undo: bool) {
        if self.kind == AppKind::Sheets {
            let ok = if undo {
                self.sheet.undo()
            } else {
                self.sheet.redo()
            };
            if ok {
                self.editor = None;
                self.sync_bars(ui);
                self.build_grid(ui);
            }
        } else if let Some(p) = self.writer.as_mut() {
            if undo {
                p.undo();
            } else {
                p.redo();
            }
            self.refocus_doc = true;
        }
        self.sync_history(ui);
    }

    /// Whether Undo and Redo have something to do, as the command history reports it.
    pub fn history_state(&mut self) -> (bool, bool) {
        match self.kind {
            AppKind::Sheets => (self.sheet.can_undo(), self.sheet.can_redo()),
            AppKind::Writer => match self.writer.as_ref() {
                Some(p) => (p.can_undo(), p.can_redo()),
                None => (false, false),
            },
        }
    }

    /// Greys the Undo and Redo buttons of the quick access toolbar when there is nothing to do.
    pub fn sync_history(&mut self, ui: &mut Ui) {
        let (undo, redo) = self.history_state();
        for (node, act) in &self.acts {
            let Act::Cmd(id) = act else { continue };
            let on = match id.as_str() {
                "qat.undo" => undo,
                "qat.redo" => redo,
                _ => continue,
            };
            if let Some(n) = ui.tree.get_mut(*node) {
                n.enabled = on;
            }
        }
    }

    /// Opens the documents a command produced (Finish & Merge, Labels) in a window of their own,
    /// as Word does: each is written to a temporary `.docx` and the host is asked to open it.
    fn open_new_documents(&mut self, out: &mut Vec<WinAct>) {
        let Some(p) = self.writer.as_mut() else {
            return;
        };
        for (n, bytes) in p.take_new_documents().into_iter().enumerate() {
            let path = std::env::temp_dir().join(format!(
                "open365-merge-{}-{}-{n}.docx",
                std::process::id(),
                self.new_documents
            ));
            self.new_documents += 1;
            match std::fs::write(&path, bytes) {
                Ok(()) => out.push(WinAct::OpenDocument(path)),
                Err(e) => p.status = Some(format!("Cannot create the new document: {e}")),
            }
        }
    }

    /// Enables the Mailings controls the way Word does: Envelopes, Labels and Start Mail Merge
    /// always; Select Recipients once a merge started; the rest once it has recipients.
    fn sync_mailings(&mut self, ui: &mut Ui) {
        let Some(p) = self.writer.as_ref() else {
            return;
        };
        let (started, recipients) = (p.mail_started(), p.mail_has_recipients());
        for (node, act) in &self.acts {
            let Act::Cmd(id) = act else { continue };
            if !id.starts_with("mailings.") {
                continue;
            }
            let on = match id.as_str() {
                "mailings.create.envelopes"
                | "mailings.create.labels"
                | "mailings.start-mail-merge.start-mail-merge" => true,
                "mailings.start-mail-merge.select-recipients" => started,
                _ => recipients,
            };
            if let Some(n) = ui.tree.get_mut(*node) {
                n.enabled = on;
            }
        }
    }

    /// Reflects the caret's formatting in the ribbon (toggle buttons, font and size boxes) and
    /// returns the keyboard focus to the document after a ribbon command.
    pub fn sync_writer_state(&mut self, ui: &mut Ui) {
        self.sync_mailings(ui);
        let Some(p) = self.writer.as_ref() else {
            return;
        };
        use doc::Toggle;
        let font = p.font_state();
        let align = p.alignment_state();
        let flags = [
            ("home.font.bold", p.toggle_state(Toggle::Bold)),
            ("home.font.italic", p.toggle_state(Toggle::Italic)),
            ("home.font.underline", p.toggle_state(Toggle::Underline)),
            ("home.font.strikethrough", p.toggle_state(Toggle::Strike)),
        ];
        let aligns = [
            ("home.paragraph.align-left", open365_docx::Alignment::Left),
            ("home.paragraph.centre", open365_docx::Alignment::Center),
            ("home.paragraph.align-right", open365_docx::Alignment::Right),
            ("home.paragraph.justify", open365_docx::Alignment::Justify),
        ];
        let ruler = p.ruler();
        let focused = ui.focus();
        for (node, act) in &self.acts {
            let Act::Cmd(id) = act else { continue };
            let Some(n) = ui.tree.get_mut(*node) else {
                continue;
            };
            match &mut n.widget {
                Widget::Checkbox { checked } if id == "view.show.ruler" => *checked = ruler,
                Widget::Button { toggled: Some(t) } => {
                    if let Some((_, v)) = flags.iter().find(|(k, _)| k == id) {
                        *t = *v;
                    } else if let Some((_, a)) = aligns.iter().find(|(k, _)| k == id) {
                        *t = align == Some(*a);
                    }
                }
                Widget::Combo { edit, .. } if focused != Some(*node) => {
                    if let Some((name, size)) = &font {
                        let text = match id.as_str() {
                            "home.font.font" => Some(name.clone()),
                            "home.font.font-size" => Some(if size.fract() == 0.0 {
                                format!("{}", *size as i32)
                            } else {
                                format!("{size}")
                            }),
                            _ => None,
                        };
                        if let Some(t) = text
                            && edit.text != t
                        {
                            edit.text = t;
                        }
                    }
                }
                _ => {}
            }
        }
        if std::mem::take(&mut self.refocus_doc)
            && let Some(c) = self.writer.as_ref().and_then(doc::WriterPane::canvas)
        {
            ui.set_focus(Some(c));
        }
    }

    fn set_zoom(&mut self, ui: &mut Ui, z: f32) {
        self.zoom = z.clamp(10.0, 400.0);
        self.build_doc(ui);
        if let Some(n) = self.roles.zoom_label.and_then(|id| ui.tree.get_mut(id)) {
            n.label = format!("{}%", self.zoom as i32);
        }
        for id in self
            .acts
            .iter()
            .filter_map(|(k, a)| (*a == Act::Zoom).then_some(*k))
        {
            if let Some(Widget::Slider { value, .. }) = ui.tree.get_mut(id).map(|n| &mut n.widget) {
                *value = self.zoom;
            }
        }
    }

    fn run(&mut self, ui: &mut Ui, id: &str, out: &mut Vec<WinAct>) {
        if self.try_open_dialog(ui, id) {
            self.note = id.to_owned();
            out.push(WinAct::Title(self.title()));
            return;
        }
        self.dispatch_command(ui, id, out);
        self.poll_dialog_requests(ui);
        self.open_new_documents(out);
        self.apply_writer_zoom(ui);
        self.note = id.to_owned();
        out.push(WinAct::Title(self.title()));
    }

    /// Routes a ribbon command to the module of its app and tab (see [`ribbon`]).
    fn dispatch_command(
        &mut self,
        ui: &mut Ui,
        id: &str,
        out: &mut Vec<WinAct>,
    ) -> ribbon::Handled {
        let mut ctx = RibbonCtx {
            kind: self.kind,
            ui,
            writer: self.writer.as_mut(),
            sheet: &mut self.sheet,
            refocus_doc: &mut self.refocus_doc,
            out,
        };
        ribbon::dispatch(&mut ctx, id)
    }

    /// Gives the Sheets model the OS file choosers (the app calls this; tests leave them unset
    /// so no native dialog opens).
    pub fn install_sheet_choosers(&mut self) {
        let s = &mut self.sheet;
        s.ext_page.chooser = Some(crate::sheet::sheet_ext::page_layout::system_picture_chooser);
        s.ext_file.open_chooser = Some(crate::sheet::sheet_ext::file::system_open_chooser);
        s.ext_file.pdf_chooser = Some(crate::sheet::sheet_ext::file::system_pdf_chooser);
    }

    /// Moves the status-bar zoom to a zoom the last command set on the Writer pane (View tab),
    /// so the next `build_doc` does not reset it.
    fn apply_writer_zoom(&mut self, ui: &mut Ui) {
        if let Some(z) = self
            .writer
            .as_mut()
            .and_then(doc::WriterPane::take_zoom_change)
        {
            self.set_zoom(ui, z);
        } else if self.kind == AppKind::Sheets {
            let z = self.sheet.ext_view.zoom as f32;
            if (z - self.zoom).abs() > f32::EPSILON {
                self.set_zoom(ui, z);
            }
        }
    }

    /// Runs ribbon command `id` as a click on its control would; whether a ribbon module knew it.
    pub fn run_command(&mut self, ui: &mut Ui, id: &str) -> ribbon::Handled {
        if self.try_open_dialog(ui, id) {
            return ribbon::Handled::Yes;
        }
        let mut out = Vec::new();
        let handled = self.dispatch_command(ui, id, &mut out);
        self.poll_dialog_requests(ui);
        self.apply_writer_zoom(ui);
        self.sync_writer_state(ui);
        handled
    }

    fn activate(&mut self, ui: &mut Ui, n: NodeId, out: &mut Vec<WinAct>) {
        let Some(act) = self.acts.get(&n).cloned() else {
            return;
        };
        match act {
            Act::Cmd(id) if id.starts_with("share.dialog.") => {
                if self.share_action(ui, n, &id) {
                    self.close_share();
                    self.build_body(ui);
                }
            }
            Act::Menu(id) if id == "share" => self.open_share(ui),
            Act::Cmd(id) if id == "qat.undo" || id == "qat.redo" || id == "qat.redoorrepeat" => {
                self.history(ui, id == "qat.undo");
                out.push(WinAct::Title(self.title()));
            }
            Act::Cmd(id) => {
                if self.group_popup.is_some() {
                    self.group_popup = None;
                    self.build_body(ui);
                }
                if let Some(rest) = id.strip_prefix("backstage.") {
                    let _ = rest;
                }
                self.run(ui, &id, out);
            }
            Act::Menu(id) => self.open_menu(ui, n, &id),
            Act::File => {
                self.backstage = Some(1.min(self.spec.backstage.nav.len().saturating_sub(1)));
                self.group_popup = None;
                self.build_body(ui);
            }
            Act::Caption(c) => out.push(match c.as_str() {
                "minimise" => WinAct::Minimize,
                "maximise" => WinAct::ToggleMaximize,
                _ => WinAct::Close,
            }),
            Act::Nav(i) => {
                let id = self.spec.backstage.nav.get(i).map(|n| n.id.clone());
                match id.as_deref() {
                    Some("back") => {
                        self.backstage = None;
                    }
                    Some(
                        page @ ("new" | "open" | "save" | "save-as" | "print" | "export" | "close"),
                    ) => {
                        let cmd = format!("file.{page}");
                        let handled = self.dispatch_command(ui, &cmd, out);
                        self.apply_writer_zoom(ui);
                        if handled == ribbon::Handled::Yes {
                            self.backstage = None;
                            self.sync_writer_state(ui);
                            out.push(WinAct::Title(self.title()));
                        } else if page == "close" {
                            out.push(WinAct::Close);
                        } else {
                            self.backstage = Some(i);
                        }
                    }
                    Some(_) => self.backstage = Some(i),
                    None => {}
                }
                self.build_body(ui);
            }
            Act::Launcher(g) => self.run(ui, &format!("{g}.launcher"), out),
            Act::GroupButton(i) => {
                self.group_popup = if self.group_popup == Some(i) {
                    None
                } else {
                    Some(i)
                };
                self.build_body(ui);
            }
            Act::ScrollRibbon => {
                self.ribbon_scroll += 200;
                self.build_body(ui);
            }
            Act::ToggleRibbon => {
                self.ribbon_open = !self.ribbon_open;
                self.group_popup = None;
                self.build_all(ui);
            }
            Act::ExpandFormula => {
                self.formula_expanded = !self.formula_expanded;
                self.build_all(ui);
            }
            Act::ZoomStep(d) => {
                let z = self.zoom + d as f32;
                self.set_zoom(ui, z);
            }
            Act::ZoomLabel => self.set_zoom(ui, 100.0),
            Act::Tab | Act::TitleBar | Act::NameBox | Act::Formula | Act::Zoom | Act::Inert => {}
        }
    }

    // ---- painting --------------------------------------------------------------------------

    /// The window background and the document area, under the widgets.
    pub fn paint_below(&mut self, ui: &mut Ui, list: &mut DrawList) {
        let (w, h) = list.size();
        list.fill_rect(
            GRect::from_edges(0.0, 0.0, w as f32, h as f32),
            self.pal.frame,
        );
        if self.backstage.is_some() || self.content.w <= 0 || self.content.h <= 0 {
            return;
        }
        match self.kind {
            AppKind::Writer => match self.writer.as_mut() {
                Some(p) => {
                    p.paint(ui, list, &self.pal, std::time::Instant::now());
                    if let (Some(id), Some(a)) = (p.canvas(), p.access()) {
                        ui.set_canvas_access(id, a.node, a.nodes);
                    }
                }
                None => doc::paint_writer(
                    ui,
                    list,
                    self.content,
                    &self.pal,
                    self.scale,
                    self.zoom / 100.0,
                ),
            },
            AppKind::Sheets => {
                let hdr = (
                    self.spec.row_header_w.unwrap_or(34),
                    self.spec.col_header_h.unwrap_or(24),
                );
                let view = doc::SheetView {
                    model: &self.sheet,
                    editing: self.editor.is_some(),
                };
                doc::paint_sheet(ui, list, self.content, &self.pal, self.scale, hdr, &view);
            }
        }
    }

    /// Text of the Name Box, for tests.
    pub fn name_box_text(&self) -> &str {
        &self.name_box
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    pub fn ribbon_scroll(&self) -> i32 {
        self.ribbon_scroll
    }

    /// Every chrome item the layout places for the current state, as `(id, rect)`; used by tests
    /// that compare the widget tree with the layout.
    pub fn placed_controls(&self) -> Vec<(String, PxRect)> {
        let spec = self.spec.clone();
        let lay = self.compute(&spec);
        let mut v: Vec<(String, PxRect)> = Vec::new();
        for g in &lay.groups {
            for c in &g.controls {
                v.push((c.control.id.clone(), c.rect));
            }
        }
        v
    }
}
