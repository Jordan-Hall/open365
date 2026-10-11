//! The shell: state, per-frame drawing and input of the application chrome.

// Drawing helpers share one frame context (ui, painter, host, layout, origin, scale, output).
#![allow(clippy::too_many_arguments)]

use egui::{
    Align2, Area, Color32, CornerRadius, FontId, Id, Key, Order, Painter, Pos2, Rect, Sense,
    Stroke, StrokeKind, Ui, UiBuilder, ViewportCommand, pos2, vec2,
};

use crate::host::{CommandState, MenuEntry, ShellHost};
use crate::icons::Icons;
use crate::layout::{self, Layout, LayoutInput, PlacedChrome, PlacedControl, PlacedGroup, PxRect};
use crate::menu::{self, MenuEvent, MenuState};
use crate::spec::{AppSpec, Chrome, Kind, REF_SCALE, Scale, Size};
use crate::theme::{AppKind, Mode, Theme, size};

/// Switches for parts a host may not be ready to use yet.
#[derive(Debug, Clone, Copy, Default)]
pub struct ShellOptions {
    /// Draw our own caption buttons and drag area (the window must have no native frame).
    pub custom_frame: bool,
    /// Draw the Name Box / formula bar strip (spreadsheet). Off while the grid still draws its own.
    pub formula_bar: bool,
    /// Draw the sheet tab bar (spreadsheet). Off while the grid still draws its own.
    pub sheet_tabs: bool,
}

/// Space between the edge of a group popup and the group inside it (points).
const POPUP_MARGIN: f32 = 4.0;

/// An editable combo box (font name, size, number format) with focus: what was typed and what it replaces.
struct ComboEdit {
    id: String,
    text: String,
    /// Characters typed so far (the rest of `text` is an auto-complete suggestion).
    typed: usize,
    focus: bool,
}

/// The chrome of one application.
pub struct Shell {
    spec: std::rc::Rc<AppSpec>,
    kind: AppKind,
    pub theme: Theme,
    pub options: ShellOptions,
    tab: usize,
    /// Ctrl+F1: the ribbon stays open (otherwise only the tabs show until one is clicked).
    pub pinned: bool,
    popup_tab: bool,
    /// How far the ribbon groups are scrolled left, in `u` (narrow windows).
    ribbon_scroll: i32,
    /// How far the tab strip is scrolled left (points) when the tabs overflow.
    tab_scroll: f32,
    /// Backstage page (index into the navigation list) when the File view is open.
    backstage: Option<usize>,
    icons: Icons,
    /// Spreadsheet formula bar expanded (measured height) or one line (derived).
    pub formula_expanded: bool,
    /// First sheet tab shown in the tab bar (scrolled with the arrows).
    sheet_first: usize,
    keytips: bool,
    /// Key tips typed so far, and the tab whose control tips are showing.
    keybuf: String,
    keytip_tab: Option<usize>,
    key_run: Option<String>,
    alt_clean: bool,
    menu: Option<MenuState>,
    edit: Option<ComboEdit>,
    /// Id of the collapsed group whose popup is open.
    group_popup: Option<String>,
    /// Developer aid: `OPEN365_SHOT=<file.ppm>` saves one frame after a few redraws and quits.
    shot: Option<(std::path::PathBuf, u32)>,
}

impl std::fmt::Debug for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shell")
            .field("app", &self.spec.app)
            .field("tab", &self.tab)
            .field("pinned", &self.pinned)
            .finish_non_exhaustive()
    }
}

/// The label of entry `id` anywhere in a menu tree.
fn label_of(entries: &[MenuEntry], id: &str) -> String {
    for e in entries {
        if e.id == id {
            return e.label.clone();
        }
        let l = label_of(&e.children, id);
        if !l.is_empty() {
            return l;
        }
    }
    String::new()
}

fn rect_of(origin: Pos2, ppp: f32, r: PxRect) -> Rect {
    Rect::from_min_size(
        origin + vec2(r.x as f32 / ppp, r.y as f32 / ppp),
        vec2(r.w as f32 / ppp, r.h as f32 / ppp),
    )
}

/// Resize zones along the edges of a frameless window (the native frame is gone, so the shell
/// provides what it gave).
fn resize_edges(ui: &Ui, full: Rect) {
    use egui::{CursorIcon, ResizeDirection as D};
    let ctx = ui.ctx();
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let b = 5.0;
    let (l, r, t, bt) = (full.left(), full.right(), full.top(), full.bottom());
    let zones = [
        (
            Rect::from_min_max(pos2(l + b, t), pos2(r - b, t + b)),
            D::North,
            CursorIcon::ResizeVertical,
        ),
        (
            Rect::from_min_max(pos2(l + b, bt - b), pos2(r - b, bt)),
            D::South,
            CursorIcon::ResizeVertical,
        ),
        (
            Rect::from_min_max(pos2(l, t + b), pos2(l + b, bt - b)),
            D::West,
            CursorIcon::ResizeHorizontal,
        ),
        (
            Rect::from_min_max(pos2(r - b, t + b), pos2(r, bt - b)),
            D::East,
            CursorIcon::ResizeHorizontal,
        ),
        (
            Rect::from_min_max(pos2(l, t), pos2(l + b, t + b)),
            D::NorthWest,
            CursorIcon::ResizeNwSe,
        ),
        (
            Rect::from_min_max(pos2(r - b, t), pos2(r, t + b)),
            D::NorthEast,
            CursorIcon::ResizeNeSw,
        ),
        (
            Rect::from_min_max(pos2(l, bt - b), pos2(l + b, bt)),
            D::SouthWest,
            CursorIcon::ResizeNeSw,
        ),
        (
            Rect::from_min_max(pos2(r - b, bt - b), pos2(r, bt)),
            D::SouthEast,
            CursorIcon::ResizeNwSe,
        ),
    ];
    for (i, (rect, dir, cursor)) in zones.into_iter().enumerate() {
        let resp = ui.interact(rect, Id::new(("resize-edge", i)), Sense::drag());
        if resp.hovered() || resp.dragged() {
            ctx.set_cursor_icon(cursor);
        }
        if resp.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
        }
    }
}

impl Shell {
    #[must_use]
    pub fn new(kind: AppKind, mode: Mode) -> Self {
        let spec = std::rc::Rc::new(match kind {
            AppKind::Writer => AppSpec::writer(),
            AppKind::Sheets => AppSpec::sheets(),
        });
        Self {
            spec,
            kind,
            theme: Theme::new(kind, mode),
            options: ShellOptions::default(),
            tab: 0,
            pinned: true,
            popup_tab: false,
            ribbon_scroll: 0,
            tab_scroll: 0.0,
            backstage: None,
            icons: Icons::default(),
            formula_expanded: true,
            sheet_first: 0,
            keytips: false,
            keybuf: String::new(),
            keytip_tab: None,
            key_run: None,
            alt_clean: false,
            menu: None,
            edit: None,
            group_popup: None,
            shot: std::env::var_os("OPEN365_SHOT").map(|p| (p.into(), 0)),
        }
    }

    #[must_use]
    pub fn spec(&self) -> &AppSpec {
        &self.spec
    }

    /// Switches between light and dark.
    pub fn set_mode(&mut self, mode: Mode) {
        self.theme = Theme::new(self.kind, mode);
    }

    /// Opens the File view (Backstage) at its Home page.
    pub fn open_backstage(&mut self) {
        self.backstage = Some(1);
    }

    #[must_use]
    pub fn backstage_open(&self) -> bool {
        self.backstage.is_some()
    }

    /// The combo box being edited: its control id and the text in its editor.
    #[must_use]
    pub fn combo_edit(&self) -> Option<(&str, &str)> {
        self.edit.as_ref().map(|e| (e.id.as_str(), e.text.as_str()))
    }

    /// How far the tab strip is scrolled (points), 0 when the tabs fit.
    #[must_use]
    pub fn tab_scroll_offset(&self) -> f32 {
        self.tab_scroll
    }

    /// How far the ribbon groups are scrolled (`u`), 0 when they fit.
    #[must_use]
    pub fn ribbon_scroll_offset(&self) -> i32 {
        self.ribbon_scroll
    }

    /// Id of the collapsed group whose popup is open.
    #[must_use]
    pub fn open_group_popup(&self) -> Option<&str> {
        self.group_popup.as_deref()
    }

    fn app_name(&self) -> &'static str {
        match self.kind {
            AppKind::Writer => "Open365 Writer",
            AppKind::Sheets => "Open365 Sheets",
        }
    }

    /// Draws the whole window into `ui` and runs the document area through `host`.
    pub fn show(&mut self, ui: &mut Ui, host: &mut dyn ShellHost) {
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let full = ui.max_rect();
        let origin = full.min;
        let spec = std::rc::Rc::clone(&self.spec);
        let chrome_was_open = self.chrome_open();
        let base = LayoutInput {
            scale: ppp,
            window: (
                (full.width() * ppp).round() as i32,
                (full.height() * ppp).round() as i32,
            ),
            inset: (0, 0, 0, 0),
            tab: self.tab,
            ribbon_open: true,
            formula_expanded: self.formula_expanded,
        };
        self.input(&ctx, &spec, &base);
        self.screenshot(&ctx);
        let ribbon_open = self.pinned || self.popup_tab;
        let lay = layout::compute_scrolled(
            &spec,
            &LayoutInput {
                tab: self.tab,
                ribbon_open,
                formula_expanded: self.formula_expanded,
                ..base
            },
            self.ribbon_scroll,
        );
        ctx.set_visuals(self.theme.visuals());
        let painter = ui.painter().clone();
        let t = self.theme;
        // Frame behind the title and tab rows.
        let top = rect_of(
            origin,
            ppp,
            PxRect {
                h: lay.content.y,
                ..lay.window
            },
        );
        painter.rect_filled(top, 0.0, t.frame);
        painter.rect_filled(rect_of(origin, ppp, lay.status), 0.0, t.status);

        let mut run: Option<(String, Option<f64>)> = None;
        self.title_row(ui, &painter, host, &lay, origin, ppp, &mut run);
        self.tab_row(ui, &painter, &lay, origin, ppp);
        if ribbon_open {
            self.ribbon(ui, &painter, host, &lay, origin, ppp, &mut run);
        }
        if self.options.formula_bar {
            self.formula_strip(ui, &painter, host, &lay, origin, ppp, &mut run);
        }
        if self.options.sheet_tabs {
            self.sheet_bar(ui, &painter, host, &lay, origin, ppp, &mut run);
        }
        self.status_bar(ui, &painter, host, &lay, origin, ppp, &mut run);

        let content = rect_of(origin, ppp, lay.content);
        painter.rect_filled(content, 0.0, t.canvas);
        ui.scope_builder(UiBuilder::new().max_rect(content), |ui| {
            ui.set_clip_rect(content);
            host.central(ui);
            if self.popup_tab
                && !self.pinned
                && ui.input(|i| i.pointer.any_pressed())
                && ui
                    .input(|i| i.pointer.interact_pos())
                    .is_some_and(|p| content.contains(p))
            {
                self.popup_tab = false;
            }
        });

        if self.backstage.is_some() {
            self.backstage_view(&ctx, host, &lay, origin, ppp, &mut run);
        }
        if self.options.custom_frame {
            resize_edges(ui, full);
        }
        self.group_popup_view(&ctx, host, &spec, &lay, origin, ppp, &mut run);
        self.menu_popup(&ctx, host, &mut run);
        self.keytip_exec(host, &spec, &lay, origin, ppp, &mut run);
        self.keytip_badges(&ctx, &spec, &lay, origin, ppp);
        if let Some((id, v)) = run {
            host.run(&id, v);
        }
        if chrome_was_open && !self.chrome_open() {
            host.chrome_closed();
        }
    }

    /// A pop-up, combo editor or the Backstage view holds the keyboard.
    fn chrome_open(&self) -> bool {
        self.menu.is_some()
            || self.edit.is_some()
            || self.group_popup.is_some()
            || self.backstage.is_some()
    }

    /// Saves the rendered frame as a binary PPM (no image dependency), then closes the window.
    fn screenshot(&mut self, ctx: &egui::Context) {
        let Some((path, frames)) = &mut self.shot else {
            return;
        };
        ctx.request_repaint();
        *frames += 1;
        if *frames == 2
            && let Some((w, h)) = std::env::var("OPEN365_SHOT_SIZE")
                .ok()
                .and_then(|v| v.split_once('x').map(|(a, b)| (a.to_owned(), b.to_owned())))
                .and_then(|(a, b)| Some((a.parse::<f32>().ok()?, b.parse::<f32>().ok()?)))
        {
            let ppp = ctx.pixels_per_point();
            ctx.send_viewport_cmd(ViewportCommand::Maximized(false));
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(w / ppp, h / ppp)));
        }
        if *frames == 25 {
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(std::sync::Arc::clone(image)),
                _ => None,
            })
        });
        if let Some(img) = image {
            let [w, h] = img.size;
            let mut out = format!(
                "P6
{w} {h}
255
"
            )
            .into_bytes();
            for p in &img.pixels {
                out.extend_from_slice(&[p.r(), p.g(), p.b()]);
            }
            let _ = std::fs::write(&*path, out);
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    fn input(&mut self, ctx: &egui::Context, spec: &AppSpec, base: &LayoutInput) {
        let (alt, any_key, esc, f1) = ctx.input_mut(|i| {
            let f1 = i.consume_key(egui::Modifiers::COMMAND, Key::F1);
            let any = i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key { pressed: true, .. }
                        | egui::Event::PointerButton { pressed: true, .. }
                )
            });
            (i.modifiers.alt, any, i.key_pressed(Key::Escape), f1)
        });
        if f1 {
            self.pinned = !self.pinned;
            self.popup_tab = false;
        }
        if alt && !self.alt_clean && !any_key {
            self.alt_clean = true;
        } else if alt && any_key {
            self.alt_clean = false;
        } else if !alt && self.alt_clean {
            self.alt_clean = false;
            self.keytips = !self.keytips;
        }
        if esc && self.menu.is_some() {
            self.menu = None;
        } else if esc && self.group_popup.is_some() {
            self.group_popup = None;
            self.keybuf.clear();
        } else if esc {
            if !self.keybuf.is_empty() {
                self.keybuf.clear();
            } else if self.keytip_tab.is_some() {
                self.keytip_tab = None;
            } else {
                self.keytips = false;
            }
        }
        if !self.keytips {
            self.keytip_tab = None;
            self.keybuf.clear();
        } else {
            let typed: String = ctx.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::Text(s) => Some(s.to_uppercase()),
                        _ => None,
                    })
                    .collect()
            });
            for ch in typed.chars() {
                self.keybuf.push(ch);
                match self.keytip_tab {
                    None => self.key_at_tab_level(),
                    Some(tab) => self.key_at_tab_content(tab, spec, base),
                }
            }
        }
    }

    /// Key tips are typed on the tab row: `F` opens File, a tab key opens that tab's tips.
    fn key_at_tab_level(&mut self) {
        let buf = self.keybuf.clone();
        if buf == "F" {
            self.backstage = Some(1);
            self.keytips = false;
            self.keybuf.clear();
            return;
        }
        let hit = self
            .spec
            .tabs
            .iter()
            .position(|t| t.key.as_deref() == Some(buf.as_str()));
        if let Some(i) = hit {
            self.tab = i;
            self.group_popup = None;
            self.popup_tab = !self.pinned;
            self.keytip_tab = Some(i);
            self.keybuf.clear();
        } else if !self
            .spec
            .tabs
            .iter()
            .any(|t| t.key.as_deref().is_some_and(|k| k.starts_with(&buf)))
        {
            self.keybuf.clear();
        }
    }

    /// Key tips that answer on `tab` right now: (keys, action). An action is a control id,
    /// `launcher:<group>` or `popup:<group>` (a collapsed group button). While a popup is open only
    /// its controls answer.
    fn visible_tips(
        &self,
        tab: usize,
        spec: &AppSpec,
        base: &LayoutInput,
    ) -> Vec<(String, String)> {
        let lay = layout::compute(
            spec,
            &LayoutInput {
                tab,
                ribbon_open: true,
                ..*base
            },
        );
        let mut tips = Vec::new();
        let add = |tips: &mut Vec<(String, String)>, pg: &PlacedGroup<'_>| {
            for pc in &pg.controls {
                if let Some(k) = pc.control.key.as_deref() {
                    tips.push((k.to_owned(), pc.control.id.clone()));
                }
            }
            if let Some(k) = pg
                .group
                .launcher_key
                .as_deref()
                .filter(|_| pg.launcher.is_some())
            {
                tips.push((k.to_owned(), format!("launcher:{}", pg.group.id)));
            }
        };
        if let Some(gid) = &self.group_popup {
            if let Some(pg) = lay.groups.iter().find(|g| g.group.id == *gid) {
                let full = layout::popup_group(spec, pg.group, base.scale);
                add(&mut tips, &full);
            }
            return tips;
        }
        for pg in &lay.groups {
            if pg.scale == Scale::Collapsed {
                if let Some(k) = pg.group.popup_key.as_deref() {
                    tips.push((k.to_owned(), format!("popup:{}", pg.group.id)));
                }
            } else {
                add(&mut tips, pg);
            }
        }
        tips
    }

    /// Key tips of a tab's controls: an exact match runs the control, a prefix waits for more.
    fn key_at_tab_content(&mut self, tab: usize, spec: &AppSpec, base: &LayoutInput) {
        let buf = self.keybuf.clone();
        let mut exact = None;
        let mut prefix = false;
        for (k, action) in self.visible_tips(tab, spec, base) {
            if k == buf {
                exact = Some(action);
            } else if k.starts_with(&buf) {
                prefix = true;
            }
        }
        // A key that is also a prefix of a longer one waits only when nothing matches exactly.
        match exact {
            Some(action) if action.starts_with("popup:") => {
                self.group_popup = action.strip_prefix("popup:").map(str::to_owned);
                self.keybuf.clear();
            }
            Some(id) => {
                self.key_run = Some(id);
                self.keytips = false;
                self.group_popup = None;
                self.keybuf.clear();
            }
            None if !prefix => self.keybuf.clear(),
            None => {}
        }
    }

    /// Runs the control a completed key tip chose (opens its menu when it has one).
    fn keytip_exec(
        &mut self,
        host: &mut dyn ShellHost,
        spec: &AppSpec,
        lay: &Layout<'_>,
        origin: Pos2,
        ppp: f32,
        run: &mut Option<(String, Option<f64>)>,
    ) {
        let Some(id) = self.key_run.take() else {
            return;
        };
        for g in &lay.groups {
            let full = layout::popup_group(spec, g.group, ppp);
            let (controls, base) = if g.scale == Scale::Collapsed {
                // The key tip came from the popup: its controls sit below the group button.
                let at = rect_of(origin, ppp, g.rect).left_bottom();
                (&full.controls, at + vec2(POPUP_MARGIN, POPUP_MARGIN))
            } else {
                (&g.controls, origin)
            };
            for pc in controls {
                if pc.control.id == id {
                    let r = rect_of(base, ppp, pc.rect);
                    if host.menu(&id).is_some() {
                        self.open_menu_for(&id, r.left_bottom());
                    } else if host.state(&id).enabled {
                        *run = Some((id, None));
                    }
                    return;
                }
            }
        }
    }

    fn keytip_badges(
        &self,
        ctx: &egui::Context,
        spec: &AppSpec,
        lay: &Layout<'_>,
        origin: Pos2,
        ppp: f32,
    ) {
        if !self.keytips {
            return;
        }
        // Tips float above everything, including the group popup.
        let painter =
            ctx.layer_painter(egui::LayerId::new(Order::Tooltip, Id::new("shell-keytips")));
        let t = self.theme;
        let badge = |r: Rect, key: &str, shown: &str| {
            let c = pos2(r.center().x, r.bottom() - 4.0);
            let w = 8.0 + 7.0 * key.chars().count() as f32;
            let b = Rect::from_center_size(c, vec2(w.max(22.0), 16.0));
            painter.rect_filled(b, 3.0, t.text);
            painter.text(
                b.center(),
                Align2::CENTER_CENTER,
                shown,
                FontId::proportional(11.0),
                t.frame,
            );
        };
        let Some(_tab) = self.keytip_tab else {
            if let Some(f) = &lay.file_tab {
                badge(rect_of(origin, ppp, f.rect), "F", "F");
            }
            for (tab, r) in self.spec.tabs.iter().zip(&lay.tab_rects) {
                if let Some(k) = tab.key.as_deref().filter(|k| k.starts_with(&self.keybuf)) {
                    badge(rect_of(origin, ppp, *r), k, k);
                }
            }
            return;
        };
        let wanted = |k: &str| k.starts_with(&self.keybuf);
        let one = |pg: &PlacedGroup<'_>, at: Pos2| {
            for pc in &pg.controls {
                if let Some(k) = pc.control.key.as_deref().filter(|k| wanted(k)) {
                    badge(rect_of(at, ppp, pc.rect), k, k);
                }
            }
            if let (Some(l), Some(k)) = (pg.launcher, pg.group.launcher_key.as_deref())
                && wanted(k)
            {
                badge(rect_of(at, ppp, l), k, k);
            }
        };
        if let Some((_, content, full)) = self.popup_geom(spec, lay, origin, ppp) {
            one(&full, content);
            return;
        }
        for g in &lay.groups {
            if g.scale == Scale::Collapsed {
                if let Some(k) = g.group.popup_key.as_deref().filter(|k| wanted(k)) {
                    badge(rect_of(origin, ppp, g.rect), k, k);
                }
            } else {
                one(g, origin);
            }
        }
    }

    /// Frame, content origin and layout of the open group popup, when its group is still collapsed.
    fn popup_geom<'a>(
        &self,
        spec: &AppSpec,
        lay: &Layout<'a>,
        origin: Pos2,
        ppp: f32,
    ) -> Option<(Rect, Pos2, PlacedGroup<'a>)> {
        let gid = self.group_popup.as_deref()?;
        let pg = lay
            .groups
            .iter()
            .find(|g| g.group.id == gid && g.scale == Scale::Collapsed)?;
        let full = layout::popup_group(spec, pg.group, ppp);
        let size = vec2(
            full.rect.w as f32 / ppp + 2.0 * POPUP_MARGIN,
            full.rect.h as f32 / ppp + 2.0 * POPUP_MARGIN,
        );
        let anchor = rect_of(origin, ppp, pg.rect);
        let right = origin.x + lay.window.w as f32 / ppp;
        let x = anchor.left().min(right - size.x).max(origin.x);
        let min = pos2(x, anchor.bottom());
        Some((
            Rect::from_min_size(min, size),
            min + vec2(POPUP_MARGIN, POPUP_MARGIN),
            full,
        ))
    }

    /// The popup of a collapsed group: the full group, drawn over the document.
    fn group_popup_view(
        &mut self,
        ctx: &egui::Context,
        host: &mut dyn ShellHost,
        spec: &AppSpec,
        lay: &Layout<'_>,
        origin: Pos2,
        ppp: f32,
        run: &mut Option<(String, Option<f64>)>,
    ) {
        if self.group_popup.is_none() {
            return;
        }
        let Some((frame, content, full)) = self.popup_geom(spec, lay, origin, ppp) else {
            self.group_popup = None;
            return;
        };
        let anchor = lay
            .groups
            .iter()
            .find(|g| Some(g.group.id.as_str()) == self.group_popup.as_deref())
            .map(|g| rect_of(origin, ppp, g.rect));
        let t = self.theme;
        let ran_before = run.is_some();
        Area::new(Id::new("group-popup"))
            .order(Order::Foreground)
            .fixed_pos(frame.min)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(frame.size(), Sense::hover());
                let painter = ui.painter().clone();
                painter.add(
                    egui::Shadow {
                        offset: [0, 3],
                        blur: 10,
                        spread: 0,
                        color: Color32::from_black_alpha(70),
                    }
                    .as_shape(rect, CornerRadius::same(6)),
                );
                painter.rect_filled(rect, 6.0, t.card);
                painter.rect_stroke(
                    rect,
                    6.0,
                    Stroke::new(1.0, t.card_border),
                    StrokeKind::Inside,
                );
                let lr = rect_of(content, ppp, full.label);
                painter.text(
                    lr.center(),
                    Align2::CENTER_CENTER,
                    &full.group.label,
                    FontId::proportional(size::GROUP),
                    t.text,
                );
                for pc in &full.controls {
                    self.control(ui, &painter, host, pc, content, ppp, run);
                }
            });
        if !ran_before && run.is_some() {
            self.group_popup = None;
            return;
        }
        let (pressed, pos) = ctx.input(|i| (i.pointer.any_pressed(), i.pointer.interact_pos()));
        let inside =
            pos.is_some_and(|p| frame.contains(p) || anchor.is_some_and(|a| a.contains(p)));
        if pressed && !inside && self.menu.is_none() {
            self.group_popup = None;
        }
    }

    // ---- drawing helpers -------------------------------------------------------------------

    fn icon(&mut self, ui: &Ui, painter: &Painter, name: &str, rect: Rect, tint: Color32) {
        if !self.icons.paint(ui.ctx(), painter, name, rect, tint) {
            painter.rect_stroke(
                rect.shrink(1.0),
                3.0,
                Stroke::new(1.2, tint),
                StrokeKind::Inside,
            );
        }
    }

    fn glyph(
        &mut self,
        ui: &Ui,
        painter: &Painter,
        icon: Option<&str>,
        label: &str,
        rect: Rect,
        tint: Color32,
    ) {
        match icon {
            Some(n) => self.icon(ui, painter, n, rect, tint),
            None => {
                painter.rect_stroke(
                    rect.shrink(1.5),
                    3.0,
                    Stroke::new(1.2, tint),
                    StrokeKind::Inside,
                );
                let initial: String = label.chars().next().into_iter().collect();
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    initial,
                    FontId::proportional(rect.height() * 0.55),
                    tint,
                );
            }
        }
    }

    fn button_bg(&self, painter: &Painter, r: Rect, resp: &egui::Response, checked: bool) {
        let t = self.theme;
        let color = if resp.is_pointer_button_down_on() {
            t.pressed
        } else if checked {
            t.checked
        } else if resp.hovered() {
            t.hover
        } else {
            return;
        };
        painter.rect_filled(r, CornerRadius::same(4), color);
    }

    fn title_row(
        &mut self,
        ui: &mut Ui,
        painter: &Painter,
        host: &mut dyn ShellHost,
        lay: &Layout<'_>,
        origin: Pos2,
        ppp: f32,
        run: &mut Option<(String, Option<f64>)>,
    ) {
        let t = self.theme;
        let ctx = ui.ctx().clone();
        // Quick Access Toolbar.
        for p in &lay.qat {
            let r = rect_of(origin, ppp, p.rect);
            let c = p.chrome;
            if c.kind == Kind::Button && c.id.contains("autosave") {
                let k = 1.0 / REF_SCALE;
                painter.text(
                    pos2(r.left() + 8.0 * k, r.center().y),
                    Align2::LEFT_CENTER,
                    "AutoSave",
                    FontId::proportional(size::LABEL - 1.0),
                    t.text_dim,
                );
                // The switch: 54 x 22 u track, 7 u from the right edge, knob on the left (off).
                let track = Rect::from_center_size(
                    pos2(r.right() - (7.0 + 27.0) * k, r.center().y + 0.5 * k),
                    vec2(54.0 * k, 22.0 * k),
                );
                painter.rect_filled(track, track.height() / 2.0, t.card);
                painter.rect_stroke(
                    track,
                    track.height() / 2.0,
                    Stroke::new(1.0, t.field_border),
                    StrokeKind::Inside,
                );
                painter.circle_filled(
                    pos2(track.left() + 13.0 * k, track.center().y),
                    8.0 * k,
                    t.field_border,
                );
                continue;
            }
            let id = c.id.as_str();
            let st = host.state(id);
            let resp = ui.interact(
                r,
                Id::new(("qat", id)),
                if st.enabled {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            if st.enabled {
                self.button_bg(painter, r, &resp, false);
            }
            let ink = t.ink(st.enabled);
            let isize = 18.0;
            if c.icon.is_some() {
                let main_w = r.width() - c.arrow_w.map_or(0.0, |a| a as f32 / ppp * 1.25);
                let ir = Rect::from_center_size(
                    pos2(r.left() + main_w / 2.0, r.center().y),
                    vec2(isize, isize),
                );
                self.icon(ui, painter, c.icon.as_deref().unwrap_or(""), ir, ink);
                if c.arrow_w.is_some() {
                    let ar = Rect::from_center_size(
                        pos2(r.right() - 7.0, r.center().y),
                        vec2(10.0, 10.0),
                    );
                    self.icon(ui, painter, "chevron-down", ar, ink);
                }
            } else {
                let ar = Rect::from_center_size(r.center(), vec2(10.0, 10.0));
                self.icon(ui, painter, "chevron-down", ar, t.text_dim);
            }
            if resp.clicked() {
                *run = Some((id.to_owned(), None));
            }
        }
        // Title text.
        if let Some(tt) = &lay.title_text {
            let r = rect_of(origin, ppp, tt.rect);
            let name = host.title();
            painter.text(
                pos2(r.left() + 12.0, r.center().y),
                Align2::LEFT_CENTER,
                format!("{name}  -  {}", self.app_name()),
                FontId::proportional(size::TITLE),
                // An inactive window's title is dimmed, as Office's is.
                if ctx.input(|i| i.focused) {
                    t.text
                } else {
                    t.text_dim
                },
            );
            if self.options.custom_frame {
                let drag = ui.interact(r, Id::new("title-drag"), Sense::click_and_drag());
                if drag.drag_started() {
                    ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                }
                if drag.double_clicked() {
                    let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(ViewportCommand::Maximized(!max));
                }
            }
        }
        // Search box.
        if let Some(s) = lay.search.as_ref().filter(|_| lay.search_button) {
            let r = rect_of(origin, ppp, s.rect);
            let resp = ui.interact(r, Id::new("search-button"), Sense::click());
            self.button_bg(painter, r, &resp, false);
            let ir = Rect::from_center_size(r.center(), vec2(16.0, 16.0));
            self.icon(ui, painter, "search", ir, t.text_dim);
        } else if let Some(s) = &lay.search {
            let r = rect_of(origin, ppp, s.rect);
            painter.rect_filled(r, 4.0, t.search_fill);
            painter.rect_stroke(
                r,
                4.0,
                Stroke::new(1.0, t.search_border),
                StrokeKind::Inside,
            );
            let ir = Rect::from_center_size(pos2(r.left() + 20.0, r.center().y), vec2(16.0, 16.0));
            self.icon(ui, painter, "search", ir, t.text_dim);
            painter.text(
                pos2(r.left() + 40.0, r.center().y),
                Align2::LEFT_CENTER,
                "Search",
                FontId::proportional(size::TITLE),
                t.text_dim,
            );
        }
        // Caption buttons.
        if self.options.custom_frame {
            for p in &lay.title {
                if !p.chrome.id.starts_with("caption.") {
                    continue;
                }
                let r = rect_of(origin, ppp, p.rect);
                let which = p.chrome.id.trim_start_matches("caption.");
                let resp = ui.interact(r, Id::new(("caption", which)), Sense::click());
                let close = which == "close";
                if resp.hovered() {
                    painter.rect_filled(r, 0.0, if close { t.close_hover } else { t.hover });
                }
                let (name, cmd) = if close {
                    ("x", ViewportCommand::Close)
                } else if which == "minimise" {
                    ("minus", ViewportCommand::Minimized(true))
                } else {
                    let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ("square", ViewportCommand::Maximized(!max))
                };
                let ir = Rect::from_center_size(r.center(), vec2(14.0, 14.0));
                self.icon(ui, painter, name, ir, t.text);
                if resp.clicked() {
                    ctx.send_viewport_cmd(cmd);
                }
            }
        }
    }

    fn tab_row(
        &mut self,
        ui: &mut Ui,
        painter: &Painter,
        lay: &Layout<'_>,
        origin: Pos2,
        ppp: f32,
    ) {
        let t = self.theme;
        if let Some(f) = &lay.file_tab {
            let r = rect_of(origin, ppp, f.rect);
            let resp = ui.interact(r, Id::new("file-tab"), Sense::click());
            self.button_bg(painter, r, &resp, false);
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                "File",
                FontId::proportional(size::TAB),
                t.text,
            );
            if resp.clicked() {
                self.backstage = Some(1);
            }
        }
        if lay.tab_scroll.is_none() {
            self.tab_scroll = 0.0;
        }
        let step = 120.0 / REF_SCALE;
        let strip_left = rect_of(origin, ppp, lay.tab_rects[0]).left();
        let left_btn = (self.tab_scroll > 0.0).then(|| {
            let r0 = rect_of(origin, ppp, lay.tab_rects[0]);
            Rect::from_min_size(r0.min, vec2(30.0 / REF_SCALE, r0.height()))
        });
        let tab_clip = lay.tab_scroll.map(|b| {
            let mut c = rect_of(origin, ppp, lay.tab_rects[0]);
            c.set_right(rect_of(origin, ppp, b).left());
            c.set_left(strip_left + left_btn.map_or(0.0, |l| l.width()));
            c
        });
        // The furthest the strip can go: its last item meets the button.
        let content_right = lay
            .tab_rects
            .last()
            .map(|r| rect_of(origin, ppp, *r).right())
            .into_iter()
            .chain(
                lay.tab_row_right
                    .iter()
                    .filter(|p| !p.chrome.id.contains("display-options"))
                    .map(|p| rect_of(origin, ppp, p.rect).right()),
            )
            .fold(0.0_f32, f32::max);
        let max_tab_scroll = tab_clip.map_or(0.0, |c| (content_right - c.right()).max(0.0));
        self.tab_scroll = self.tab_scroll.min(max_tab_scroll);
        let scroll_dx = self.tab_scroll;
        let tab_painter = tab_clip.map_or_else(|| painter.clone(), |c| painter.with_clip_rect(c));
        let painter = &tab_painter;
        for i in 0..self.spec.tabs.len() {
            let r = rect_of(origin, ppp, lay.tab_rects[i]).translate(vec2(-scroll_dx, 0.0));
            if tab_clip.is_some_and(|c| !c.intersects(r)) {
                continue;
            }
            let hit = tab_clip.map_or(r, |c| r.intersect(c));
            let resp = ui.interact(hit, Id::new(("tab", i)), Sense::click());
            let sel = i == self.tab;
            if resp.hovered() && !sel {
                painter.rect_filled(r.shrink2(vec2(0.0, 3.0)), 4.0, t.hover);
            }
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                &self.spec.tabs[i].label,
                FontId::proportional(size::TAB),
                t.text,
            );
            if sel {
                // Office draws the selected tab semibold; a second pass half a pixel to the right
                // thickens the regular face without a second font file.
                painter.text(
                    r.center() + vec2(0.5 / ppp, 0.0),
                    Align2::CENTER_CENTER,
                    &self.spec.tabs[i].label,
                    FontId::proportional(size::TAB),
                    t.text,
                );
            }
            if sel {
                let y = r.bottom() - 3.5;
                let inset = 11.0 / REF_SCALE;
                painter.line_segment(
                    [pos2(r.left() + inset, y), pos2(r.right() - inset, y)],
                    Stroke::new(3.0 / ppp, t.accent),
                );
            }
            if resp.clicked() {
                self.tab = i;
                self.group_popup = None;
                self.popup_tab = !self.pinned;
            }
            if resp.double_clicked() {
                self.pinned = !self.pinned;
            }
        }
        let full_painter = ui.painter().clone();
        let strip_painter =
            tab_clip.map_or_else(|| full_painter.clone(), |c| full_painter.with_clip_rect(c));
        let painter = &full_painter;
        if let Some(b) = lay.tab_scroll {
            let r = rect_of(origin, ppp, b);
            let resp = ui.interact(r, Id::new("tab-scroll"), Sense::click());
            painter.rect_filled(r, 0.0, t.frame);
            self.button_bg(painter, r, &resp, false);
            self.icon(
                ui,
                painter,
                "chevron-right",
                Rect::from_center_size(r.center(), vec2(12.0, 12.0)),
                t.text,
            );
            if resp.clicked() {
                self.tab_scroll = (self.tab_scroll + step).min(max_tab_scroll);
            }
        }
        if let Some(r) = left_btn {
            let resp = ui.interact(r, Id::new("tab-scroll-left"), Sense::click());
            painter.rect_filled(r, 0.0, t.frame);
            self.button_bg(painter, r, &resp, false);
            self.icon(
                ui,
                painter,
                "chevron-left",
                Rect::from_center_size(r.center(), vec2(12.0, 12.0)),
                t.text,
            );
            if resp.clicked() {
                self.tab_scroll = (self.tab_scroll - step).max(0.0);
            }
        }
        for p in &lay.tab_row_right {
            let display = p.chrome.id.contains("display-options");
            if p.offscreen && tab_clip.is_none() {
                continue;
            }
            let mut r = rect_of(origin, ppp, p.rect);
            let painter = if display {
                &full_painter
            } else {
                &strip_painter
            };
            if !display {
                r = r.translate(vec2(-scroll_dx, 0.0));
                if tab_clip.is_some_and(|c| !c.intersects(r)) {
                    continue;
                }
            }
            if display {
                let resp = ui.interact(r, Id::new("ribbon-display"), Sense::click());
                self.button_bg(painter, r, &resp, false);
                let name = if self.pinned {
                    "chevron-up"
                } else {
                    "chevron-down"
                };
                self.icon(
                    ui,
                    painter,
                    name,
                    Rect::from_center_size(r.center(), vec2(12.0, 12.0)),
                    t.text,
                );
                if resp.clicked() {
                    self.pinned = !self.pinned;
                    self.popup_tab = false;
                }
                continue;
            }
            // Comments / Editing / Share: drawn as Office draws them, not wired yet.
            let id = p.chrome.id.as_str();
            let share = id == "share";
            if share {
                painter.rect_filled(r, 4.0, t.share);
            } else {
                painter.rect_stroke(r, 4.0, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
            }
            let ink = if share { t.on_accent } else { t.text };
            let icon = match id {
                "comments" => "message",
                "editing" => "pencil",
                _ => "share",
            };
            let font = FontId::proportional(size::LABEL + 1.0);
            if p.compact {
                self.icon(
                    ui,
                    painter,
                    icon,
                    Rect::from_center_size(r.center(), vec2(16.0, 16.0)),
                    ink,
                );
                continue;
            }
            let galley = painter.layout_no_wrap(p.chrome.label.clone(), font.clone(), ink);
            let content = 16.0 + 6.0 + galley.size().x + if id == "comments" { 0.0 } else { 14.0 };
            let x0 = r.center().x - content / 2.0;
            self.icon(
                ui,
                painter,
                icon,
                Rect::from_min_size(pos2(x0, r.center().y - 8.0), vec2(16.0, 16.0)),
                ink,
            );
            painter.text(
                pos2(x0 + 22.0, r.center().y),
                Align2::LEFT_CENTER,
                &p.chrome.label,
                font,
                ink,
            );
            if id != "comments" {
                self.icon(
                    ui,
                    painter,
                    "chevron-down",
                    Rect::from_center_size(
                        pos2(x0 + 22.0 + galley.size().x + 8.0, r.center().y),
                        vec2(10.0, 10.0),
                    ),
                    ink,
                );
            }
        }
    }

    fn ribbon(
        &mut self,
        ui: &mut Ui,
        painter: &Painter,
        host: &mut dyn ShellHost,
        lay: &Layout<'_>,
        origin: Pos2,
        ppp: f32,
        run: &mut Option<(String, Option<f64>)>,
    ) {
        let t = self.theme;
        let card = rect_of(origin, ppp, lay.card);
        painter.rect_filled(card, 8.0, t.card);
        // Groups that overflow scroll under the "Column right" button; they are clipped where it begins.
        let clipped = lay.ribbon_scroll.map(|sc| {
            let mut c = card;
            c.set_right(rect_of(origin, ppp, sc.button).left());
            if self.ribbon_scroll > 0 {
                c.set_left(c.left() + rect_of(origin, ppp, sc.button).width());
            }
            painter.with_clip_rect(c)
        });
        let group_painter = clipped.as_ref().unwrap_or(painter);
        let painter = group_painter;
        for g in &lay.groups {
            let gr = rect_of(origin, ppp, g.rect);
            painter.line_segment(
                [
                    pos2(gr.right(), gr.top() + 6.0),
                    pos2(gr.right(), gr.bottom() - 6.0),
                ],
                Stroke::new(1.0, t.separator),
            );
            if g.scale == Scale::Collapsed {
                self.group_button(ui, painter, g, origin, ppp);
                continue;
            }
            let lr = rect_of(origin, ppp, g.label);
            painter.text(
                lr.center(),
                Align2::CENTER_CENTER,
                &g.group.label,
                FontId::proportional(size::GROUP),
                t.text,
            );
            if let Some(l) = g.launcher {
                let r = rect_of(origin, ppp, l);
                let id = format!("{}.launcher", g.group.id);
                let resp = ui.interact(r, Id::new(&id), Sense::hover());
                self.button_bg(painter, r, &resp, false);
                let c = r.center();
                let s = Stroke::new(1.0, t.text_disabled);
                painter.line_segment([pos2(c.x - 3.0, c.y - 3.0), pos2(c.x + 3.0, c.y + 3.0)], s);
                painter.line_segment([pos2(c.x + 3.0, c.y), pos2(c.x + 3.0, c.y + 3.0)], s);
                painter.line_segment([pos2(c.x, c.y + 3.0), pos2(c.x + 3.0, c.y + 3.0)], s);
            }
            for pc in &g.controls {
                if let Some(sc) = lay.ribbon_scroll
                    && pc.rect.x > sc.button.x
                {
                    continue;
                }
                self.control(ui, painter, host, pc, origin, ppp, run);
            }
        }
        if let Some(sc) = lay.ribbon_scroll {
            let step = 120;
            let button = |this: &mut Self, rect: PxRect, icon: &str, id: &str, delta: i32| {
                let r = rect_of(origin, ppp, rect);
                let resp = ui.interact(r, Id::new(id), Sense::click());
                let pt = ui.painter().clone();
                pt.rect_filled(r, 0.0, this.theme.card);
                this.button_bg(&pt, r, &resp, false);
                let ink = this.theme.text;
                this.icon(
                    ui,
                    &pt,
                    icon,
                    Rect::from_center_size(r.center(), vec2(12.0, 12.0)),
                    ink,
                );
                if resp.clicked() {
                    this.ribbon_scroll = (this.ribbon_scroll + delta).clamp(0, sc.max);
                }
            };
            button(
                self,
                sc.button,
                "chevron-right",
                "ribbon-column-right",
                step,
            );
            if self.ribbon_scroll > 0 {
                let left = PxRect {
                    x: lay.card.x,
                    ..sc.button
                };
                button(self, left, "chevron-left", "ribbon-column-left", -step);
            }
        }
    }

    /// A collapsed group: one large button (icon, group name, arrow) that opens the popup.
    fn group_button(
        &mut self,
        ui: &mut Ui,
        painter: &Painter,
        g: &PlacedGroup<'_>,
        origin: Pos2,
        ppp: f32,
    ) {
        let t = self.theme;
        let r = rect_of(origin, ppp, g.button.unwrap_or(g.rect)).shrink2(vec2(1.0, 1.0));
        let open = self.group_popup.as_deref() == Some(g.group.id.as_str());
        let resp = ui.interact(r, Id::new(("group-button", &g.group.id)), Sense::click());
        self.button_bg(painter, r, &resp, open);
        let k = 1.0 / REF_SCALE;
        let ink = t.text;
        let ir = Rect::from_center_size(
            pos2(r.center().x, r.top() + 25.0 * k),
            vec2(36.0 * k, 36.0 * k),
        );
        self.glyph(
            ui,
            painter,
            g.group.icon.as_deref(),
            &g.group.label,
            ir,
            ink,
        );
        let label = &g.group.label;
        let font = FontId::proportional(size::LABEL);
        let words: Vec<&str> = label.split(' ').collect();
        let lines = if words.len() > 1 && label.len() > 8 {
            let half = words.len().div_ceil(2);
            vec![words[..half].join(" "), words[half..].join(" ")]
        } else {
            vec![label.clone()]
        };
        let last = lines.len() - 1;
        for (i, line) in lines.iter().enumerate() {
            let y = r.top() + (56.0 + 19.0 * i as f32) * k;
            painter.text(
                pos2(r.center().x, y),
                Align2::CENTER_CENTER,
                line,
                font.clone(),
                ink,
            );
            if i == last {
                self.icon(
                    ui,
                    painter,
                    "chevron-down",
                    Rect::from_center_size(pos2(r.center().x, y + 14.0 * k), vec2(9.0, 9.0)),
                    ink,
                );
            }
        }
        if resp.hovered() {
            resp.clone().on_hover_text(label);
        }
        if resp.clicked() {
            self.group_popup = if open { None } else { Some(g.group.id.clone()) };
            self.menu = None;
        }
    }

    fn open_menu_for(&mut self, id: &str, at: Pos2) {
        self.menu = Some(MenuState::new(id, at, 0.0, false));
    }

    /// Opens the list under an editable combo.
    fn open_list_for(&mut self, id: &str, r: Rect) {
        self.menu = Some(MenuState::new(
            id,
            r.left_bottom(),
            r.width() * REF_SCALE,
            true,
        ));
    }

    fn control(
        &mut self,
        ui: &mut Ui,
        painter: &Painter,
        host: &mut dyn ShellHost,
        pc: &PlacedControl<'_>,
        origin: Pos2,
        ppp: f32,
        run: &mut Option<(String, Option<f64>)>,
    ) {
        let t = self.theme;
        let c = pc.control;
        let r = rect_of(origin, ppp, pc.rect);
        let st: CommandState = host.state(&c.id);
        let has_menu = host.menu(&c.id).is_some();
        let live = st.enabled || has_menu;
        let ink = t.ink(live);
        let sense = if live { Sense::click() } else { Sense::hover() };
        // Dips per reference unit (`u` = pixels at the 125 % reference scale).
        let k = 1.0 / REF_SCALE;
        match c.kind {
            Kind::Combo if has_menu => {
                let fallback = c.value.clone().unwrap_or_default();
                self.combo(ui, painter, host, &c.id, r, &fallback, run);
            }
            Kind::Combo | Kind::Edit | Kind::Spinner | Kind::Checkbox => {
                let resp = ui.interact(r, Id::new(("ctl", &c.id)), sense);
                if c.kind == Kind::Checkbox {
                    let b = Rect::from_center_size(
                        pos2(r.left() + 10.0, r.center().y),
                        vec2(14.0, 14.0),
                    );
                    painter.rect_stroke(b, 2.0, Stroke::new(1.0, ink), StrokeKind::Inside);
                    if st.checked {
                        self.icon(ui, painter, "check", b, ink);
                    }
                    painter.text(
                        pos2(r.left() + 22.0, r.center().y),
                        Align2::LEFT_CENTER,
                        &c.label,
                        FontId::proportional(size::LABEL),
                        ink,
                    );
                } else {
                    painter.rect_filled(r, 3.0, t.field);
                    painter.rect_stroke(
                        r,
                        3.0,
                        Stroke::new(1.0, t.field_border),
                        StrokeKind::Inside,
                    );
                    let v = host
                        .value(&c.id)
                        .or_else(|| c.value.clone())
                        .unwrap_or_default();
                    painter.text(
                        pos2(r.left() + 6.0, r.center().y),
                        Align2::LEFT_CENTER,
                        v,
                        FontId::proportional(size::LABEL),
                        ink,
                    );
                    if c.kind == Kind::Combo {
                        let ar = Rect::from_center_size(
                            pos2(r.right() - 9.0, r.center().y),
                            vec2(10.0, 10.0),
                        );
                        self.icon(ui, painter, "chevron-down", ar, ink);
                    }
                }
                if resp.clicked() && has_menu {
                    self.open_menu_for(&c.id, r.left_bottom());
                }
            }
            Kind::Gallery => {
                painter.rect_filled(r, 4.0, t.field);
                painter.rect_stroke(r, 4.0, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
                let tiles = host.gallery(&c.id);
                let cur = host.value(&c.id).unwrap_or_default();
                let inner = Rect::from_min_max(
                    r.min + vec2(4.0, 4.0),
                    pos2(r.right() - 26.0, r.bottom() - 4.0),
                );
                // Cells are 135 x 70 u from (3, 6) u inside the control; the white tile sits 5 u inside
                // its cell, and the current or hovered cell gets a 1 u light ring over a lifted fill.
                let n = tiles.len().min(((c.w - 38) / 135).max(1) as usize);
                let _ = inner;
                for (i, (vid, label)) in tiles.iter().take(n).enumerate() {
                    let cell = Rect::from_min_size(
                        r.min + vec2((3.0 + 135.0 * i as f32) * k, 6.0 * k),
                        vec2(135.0 * k, 70.0 * k),
                    );
                    let tr = cell.shrink(5.0 * k);
                    let resp = ui.interact(cell, Id::new(("tile", &c.id, vid)), Sense::click());
                    let on = *vid == cur;
                    if on || resp.hovered() {
                        painter.rect_filled(cell, 0.0, t.gallery_cell);
                        painter.rect_stroke(
                            cell,
                            0.0,
                            Stroke::new(1.0 / ppp.max(1.0), t.gallery_ring),
                            StrokeKind::Inside,
                        );
                    }
                    painter.rect_filled(tr, 0.0, Color32::WHITE);
                    painter.text(
                        tr.center(),
                        Align2::CENTER_CENTER,
                        label,
                        FontId::proportional(if i == 2 {
                            size::LABEL + 8.0
                        } else {
                            size::LABEL + 1.0
                        }),
                        Color32::from_rgb(0x24, 0x24, 0x24),
                    );
                    if resp.clicked() {
                        *run = Some((format!("{}#{vid}", c.id), None));
                    }
                }
                let ar = Rect::from_min_max(pos2(r.right() - 24.0, r.top()), r.max);
                let resp = ui.interact(
                    ar,
                    Id::new(("galmore", &c.id)),
                    if has_menu {
                        Sense::click()
                    } else {
                        Sense::hover()
                    },
                );
                self.icon(
                    ui,
                    painter,
                    "chevron-down",
                    Rect::from_center_size(ar.center(), vec2(12.0, 12.0)),
                    t.text_dim,
                );
                if resp.clicked() {
                    self.open_menu_for(&c.id, ar.left_bottom());
                }
            }
            _ => {
                let large = c.size == Size::Large;
                let split = c.kind == Kind::Split;
                let main = rect_of(origin, ppp, pc.main);
                let arrow = rect_of(origin, ppp, pc.arrow);
                let resp_main = ui.interact(main, Id::new(("ctl", &c.id)), sense);
                let resp_arrow =
                    split.then(|| ui.interact(arrow, Id::new(("arrow", &c.id)), sense));
                if live {
                    self.button_bg(painter, main, &resp_main, st.checked);
                    if let Some(ra) = &resp_arrow {
                        self.button_bg(painter, arrow, ra, false);
                    }
                }
                let icon = c.icon.as_deref();
                if large {
                    let ir = Rect::from_center_size(
                        pos2(r.center().x, r.top() + 17.0 * k),
                        vec2(36.0 * k, 36.0 * k),
                    );
                    self.glyph(ui, painter, icon, &c.label, ir, ink);
                    let words: Vec<&str> = c.label.split(' ').collect();
                    let lines = if words.len() > 1 && c.label.len() > 7 {
                        let half = words.len() / 2;
                        vec![words[..half].join(" "), words[half..].join(" ")]
                    } else {
                        vec![c.label.clone()]
                    };
                    let font = FontId::proportional(size::LABEL);
                    let has_arrow = split || c.kind == Kind::Dropdown;
                    let two = lines.len() > 1;
                    for (i, line) in lines.iter().enumerate() {
                        let y = r.top() + if two { 49.0 + 19.0 * i as f32 } else { 50.5 } * k;
                        let last = i + 1 == lines.len();
                        if two && last && has_arrow {
                            // Office puts the arrow after the last line of a two-line label.
                            let galley = painter.layout_no_wrap(line.clone(), font.clone(), ink);
                            let w = galley.size().x + 12.0;
                            let x0 = r.center().x - w / 2.0;
                            painter.text(pos2(x0, y), Align2::LEFT_CENTER, line, font.clone(), ink);
                            self.icon(
                                ui,
                                painter,
                                "chevron-down",
                                Rect::from_center_size(
                                    pos2(x0 + galley.size().x + 7.0, y),
                                    vec2(9.0, 9.0),
                                ),
                                ink,
                            );
                        } else {
                            painter.text(
                                pos2(r.center().x, y),
                                Align2::CENTER_CENTER,
                                line,
                                font.clone(),
                                ink,
                            );
                        }
                    }
                    if has_arrow && !two {
                        self.icon(
                            ui,
                            painter,
                            "chevron-down",
                            Rect::from_center_size(
                                pos2(r.center().x, r.top() + 69.5 * k),
                                vec2(9.0, 9.0),
                            ),
                            ink,
                        );
                    }
                } else {
                    let icon_only = main.width() < 36.0 * k + 2.0 && c.kind != Kind::Dropdown
                        || c.w <= 50 && c.label.len() > 7;
                    let ir = Rect::from_min_size(
                        pos2(main.left() + 6.0, main.center().y - 8.0),
                        vec2(16.0, 16.0),
                    );
                    if icon_only && !(c.kind == Kind::Dropdown && c.w > 50) {
                        self.glyph(
                            ui,
                            painter,
                            icon,
                            &c.label,
                            Rect::from_center_size(main.center(), vec2(16.0, 16.0)),
                            ink,
                        );
                    } else {
                        self.glyph(ui, painter, icon, &c.label, ir, ink);
                        painter.text(
                            pos2(ir.right() + 6.0, main.center().y),
                            Align2::LEFT_CENTER,
                            &c.label,
                            FontId::proportional(size::LABEL),
                            ink,
                        );
                    }
                    if c.kind == Kind::Dropdown {
                        self.icon(
                            ui,
                            painter,
                            "chevron-down",
                            Rect::from_center_size(
                                pos2(r.right() - 9.0, r.center().y),
                                vec2(9.0, 9.0),
                            ),
                            ink,
                        );
                    }
                    if split {
                        self.icon(
                            ui,
                            painter,
                            "chevron-down",
                            Rect::from_center_size(arrow.center(), vec2(9.0, 9.0)),
                            ink,
                        );
                    }
                }
                if !c.label.is_empty() && resp_main.hovered() {
                    resp_main.clone().on_hover_text(&c.label);
                }
                if resp_main.clicked() {
                    if st.enabled && !(c.kind == Kind::Dropdown && has_menu) {
                        *run = Some((c.id.clone(), None));
                    } else if has_menu {
                        self.open_menu_for(&c.id, main.left_bottom());
                    }
                }
                if let Some(ra) = resp_arrow
                    && ra.clicked()
                    && has_menu
                {
                    self.open_menu_for(&c.id, arrow.left_bottom());
                }
            }
        }
    }

    /// An editable combo box: typed input with Office's inline auto-complete, Enter commits through
    /// [`ShellHost::combo_commit`], Escape cancels, the arrow (or Up / Down) opens the list.
    fn combo(
        &mut self,
        ui: &mut Ui,
        painter: &Painter,
        host: &mut dyn ShellHost,
        id: &str,
        r: Rect,
        fallback: &str,
        run: &mut Option<(String, Option<f64>)>,
    ) {
        let t = self.theme;
        let k = 1.0 / REF_SCALE;
        let editing = self.edit.as_ref().is_some_and(|e| e.id == id);
        let listing = self.menu.as_ref().is_some_and(|m| m.id == id);
        let border = if editing || listing {
            t.accent
        } else {
            t.field_border
        };
        painter.rect_filled(r, 3.0, t.field);
        painter.rect_stroke(r, 3.0, Stroke::new(1.0, border), StrokeKind::Inside);
        let arrow = Rect::from_min_max(
            pos2(r.right() - 17.0 * k, r.top() + k),
            pos2(r.right() - k, r.bottom() - k),
        );
        let text_r = Rect::from_min_max(
            pos2(r.left() + k, r.top() + 3.0 * k),
            pos2(arrow.left(), r.bottom() - 3.0 * k),
        );
        let ra = ui.interact(arrow, Id::new(("combo-arrow", id)), Sense::click());
        if ra.hovered() || listing {
            painter.rect_filled(arrow, 2.0, t.hover);
        }
        self.icon(
            ui,
            painter,
            "chevron-down",
            Rect::from_center_size(arrow.center(), vec2(10.0, 10.0)),
            t.text,
        );
        if ra.clicked() {
            if listing {
                self.menu = None;
            } else {
                self.open_list_for(id, r);
            }
        }
        let shown = host.value(id).unwrap_or_else(|| fallback.to_owned());
        if !editing {
            painter.text(
                pos2(text_r.left() + 4.0 * k, r.center().y),
                Align2::LEFT_CENTER,
                &shown,
                FontId::proportional(size::LABEL),
                t.text,
            );
            let rt = ui.interact(text_r, Id::new(("combo-text", id)), Sense::click());
            if rt.clicked() {
                self.edit = Some(ComboEdit {
                    id: id.to_owned(),
                    text: shown.clone(),
                    // The whole text is selected: the first typed character counts as growth, so it completes too.
                    typed: 0,
                    focus: true,
                });
            }
            if self.edit.as_ref().is_none_or(|e| e.id != id) {
                return;
            }
        }
        let Some(mut ed) = self.edit.take() else {
            return;
        };
        let eid = Id::new(("combo-edit", id));
        let mut text = ed.text.clone();
        let out = ui.put(
            text_r,
            egui::TextEdit::singleline(&mut text)
                .id(eid)
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::symmetric(4, 0))
                .font(FontId::proportional(size::LABEL))
                .text_color(t.text)
                .vertical_align(egui::Align::Center),
        );
        let n = ed.text.chars().count();
        if ed.focus {
            ui.memory_mut(|m| m.request_focus(eid));
            let mut st = egui::TextEdit::load_state(ui.ctx(), eid).unwrap_or_default();
            st.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(n),
            )));
            st.store(ui.ctx(), eid);
            ed.focus = false;
        }
        let list_hot = self
            .menu
            .as_ref()
            .is_some_and(|m| m.id == id && m.hot.first().is_some_and(Option::is_some));
        let (enter, esc, down, up) = ui.input(|i| {
            (
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Escape),
                i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::ArrowUp),
            )
        });
        if out.changed() {
            let count = text.chars().count();
            let grew = count > ed.typed;
            ed.typed = count;
            if grew && let Some(entries) = host.menu(id) {
                let lower = text.to_lowercase();
                let numeric = entries.iter().any(|e| e.label.parse::<f64>().is_ok());
                let hit = entries.iter().find(|e| {
                    e.kind == crate::host::EntryKind::Item
                        && !numeric
                        && e.label.to_lowercase().starts_with(&lower)
                        && e.label.chars().count() > count
                });
                if let Some(e) = hit {
                    text = e.label.clone();
                    let mut st = egui::TextEdit::load_state(ui.ctx(), eid).unwrap_or_default();
                    st.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(count),
                        egui::text::CCursor::new(text.chars().count()),
                    )));
                    st.store(ui.ctx(), eid);
                }
            }
            ed.text = text.clone();
        }
        if (down || up) && !listing {
            self.open_list_for(id, r);
        }
        if esc {
            ui.memory_mut(|m| m.surrender_focus(eid));
            self.menu = None;
        // A single-line egui TextEdit gives up its focus on Enter itself, so the editor has already lost it here.
        } else if enter && (out.has_focus() || out.lost_focus()) && !list_hot {
            if let Some(cmd) = host.combo_commit(id, &text) {
                *run = Some((cmd, None));
            }
            ui.memory_mut(|m| m.surrender_focus(eid));
            self.menu = None;
        } else if out.lost_focus() && !listing && !ra.hovered() {
            // Clicking elsewhere abandons the edit.
        } else {
            self.edit = Some(ed);
        }
    }

    fn menu_popup(
        &mut self,
        ctx: &egui::Context,
        host: &mut dyn ShellHost,
        run: &mut Option<(String, Option<f64>)>,
    ) {
        let Some(m) = &mut self.menu else { return };
        let entries: Vec<MenuEntry> = host.menu(&m.id).unwrap_or_default();
        let list = m.list;
        let combo = m.id.clone();
        match menu::show(ctx, m, &entries, &self.theme, &mut self.icons) {
            MenuEvent::None => {}
            MenuEvent::Run(id) => {
                let _ = (list, &combo, label_of(&entries, &id));
                *run = Some((id, None));
                self.menu = None;
                self.edit = None;
                self.group_popup = None;
            }
            MenuEvent::Close => self.menu = None,
        }
    }

    fn status_bar(
        &mut self,
        ui: &mut Ui,
        painter: &Painter,
        host: &mut dyn ShellHost,
        lay: &Layout<'_>,
        origin: Pos2,
        ppp: f32,
        run: &mut Option<(String, Option<f64>)>,
    ) {
        let t = self.theme;
        for p in &lay.status_items {
            let c: &Chrome = p.chrome;
            let r = rect_of(origin, ppp, p.rect);
            let id = c.id.as_str();
            let ink = t.text;
            match id {
                "status.grip" => {}
                "status.slider" | "status.zoom" => self.zoom_slider(ui, painter, host, p, r, run),
                "status.zoom-out" | "status.zoom-in" => {
                    let resp = ui.interact(r, Id::new(id), Sense::click());
                    self.button_bg(painter, r, &resp, false);
                    let n = if id.ends_with("out") { "minus" } else { "plus" };
                    self.icon(
                        ui,
                        painter,
                        n,
                        Rect::from_center_size(r.center(), vec2(12.0, 12.0)),
                        t.text_dim,
                    );
                    if resp.clicked() {
                        let z =
                            f64::from(host.zoom()) + if id.ends_with("out") { -0.1 } else { 0.1 };
                        *run = Some(("view.zoom".to_owned(), Some(z.clamp(0.1, 5.0))));
                    }
                }
                "status.zoom-percent" => {
                    let resp = ui.interact(r, Id::new(id), Sense::click());
                    self.button_bg(painter, r, &resp, false);
                    painter.text(
                        r.center(),
                        Align2::CENTER_CENTER,
                        format!("{}%", (host.zoom() * 100.0).round()),
                        FontId::proportional(size::LABEL),
                        ink,
                    );
                    if resp.clicked() {
                        *run = Some(("view.zoom".to_owned(), Some(1.0)));
                    }
                }
                "status.read"
                | "status.print-layout"
                | "status.web-layout"
                | "status.normal"
                | "status.page-layout"
                | "status.page-break"
                | "status.focus" => {
                    let st = host.state(id);
                    let resp = ui.interact(
                        r,
                        Id::new(id),
                        if st.enabled {
                            Sense::click()
                        } else {
                            Sense::hover()
                        },
                    );
                    self.button_bg(painter, r, &resp, st.checked);
                    let icon = match id {
                        "status.read" => "book-2",
                        "status.web-layout" => "world",
                        "status.normal" => "table",
                        "status.focus" => "layout-grid",
                        _ => "file-text",
                    };
                    let tint = t.ink(st.enabled);
                    if id == "status.focus" {
                        painter.text(
                            r.center(),
                            Align2::CENTER_CENTER,
                            "Focus",
                            FontId::proportional(size::LABEL),
                            tint,
                        );
                    } else {
                        self.icon(
                            ui,
                            painter,
                            icon,
                            Rect::from_center_size(r.center(), vec2(16.0, 16.0)),
                            tint,
                        );
                    }
                    if resp.clicked() {
                        *run = Some((id.to_owned(), None));
                    }
                }
                _ => {
                    let text = host.status_text(id).unwrap_or_else(|| {
                        // Language and accessibility texts come from the reference tree.
                        c.ref_name
                            .as_deref()
                            .and_then(|n| {
                                n.strip_prefix("Language ")
                                    .or_else(|| n.strip_prefix("Accessibility Checker "))
                            })
                            .map_or_else(|| c.label.clone(), str::to_owned)
                    });
                    painter.text(
                        pos2(r.left() + 7.0, r.center().y),
                        Align2::LEFT_CENTER,
                        text,
                        FontId::proportional(size::STATUS),
                        ink,
                    );
                }
            }
        }
    }

    fn zoom_slider(
        &mut self,
        ui: &mut Ui,
        painter: &Painter,
        host: &mut dyn ShellHost,
        _p: &PlacedChrome<'_>,
        r: Rect,
        run: &mut Option<(String, Option<f64>)>,
    ) {
        let t = self.theme;
        let resp = ui.interact(r, Id::new("zoom-slider"), Sense::click_and_drag());
        let track = r.shrink2(vec2(8.0, 0.0));
        let y = r.center().y;
        painter.line_segment(
            [pos2(track.left(), y), pos2(track.right(), y)],
            Stroke::new(1.0, t.text_dim),
        );
        painter.line_segment(
            [
                pos2(track.center().x, y - 4.0),
                pos2(track.center().x, y + 4.0),
            ],
            Stroke::new(1.0, t.text_dim),
        );
        // Office maps 10..100 % to the left half and 100..500 % to the right half.
        let z = host.zoom();
        let frac = if z <= 1.0 {
            (z - 0.1) / 0.9 * 0.5
        } else {
            0.5 + (z - 1.0) / 4.0 * 0.5
        };
        let x = track.left() + frac.clamp(0.0, 1.0) * track.width();
        painter.rect_filled(
            Rect::from_center_size(pos2(x, y), vec2(5.0, 14.0)),
            2.0,
            t.text,
        );
        if (resp.dragged() || resp.clicked())
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let f = ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0);
            let z = if f <= 0.5 {
                0.1 + f / 0.5 * 0.9
            } else {
                1.0 + (f - 0.5) / 0.5 * 4.0
            };
            let z = (f64::from(z) * 100.0).round() / 100.0;
            *run = Some(("view.zoom".to_owned(), Some(z)));
        }
    }

    fn formula_strip(
        &mut self,
        ui: &mut Ui,
        painter: &Painter,
        host: &mut dyn ShellHost,
        lay: &Layout<'_>,
        origin: Pos2,
        ppp: f32,
        run: &mut Option<(String, Option<f64>)>,
    ) {
        let t = self.theme;
        let border = Stroke::new(1.0, t.field_border);
        let px = 1.0 / ppp;
        // One reference unit (a pixel at 125 %) in points.
        let u = 1.0 / REF_SCALE;
        if let Some(l) = lay.formula_left {
            painter.rect_filled(rect_of(origin, ppp, l), 0.0, t.frame);
        }
        if let Some(b) = lay.formula_bar {
            painter.rect_filled(rect_of(origin, ppp, b), 0.0, t.frame);
        }
        if let Some(nb) = lay.name_box_outer {
            let r = rect_of(origin, ppp, nb);
            painter.rect_filled(r, 3.0, t.card);
            painter.rect_stroke(r, 3.0, border, StrokeKind::Inside);
            let inner = Rect::from_min_max(
                pos2(r.left() + 5.0, r.top() + px),
                pos2(r.right() - 22.0, r.bottom() - px),
            );
            let drawn = ui
                .scope_builder(UiBuilder::new().max_rect(inner), |ui| {
                    ui.set_clip_rect(inner);
                    host.name_box_ui(ui)
                })
                .inner;
            if !drawn {
                painter.text(
                    pos2(inner.left() + 1.0, r.center().y),
                    Align2::LEFT_CENTER,
                    host.value("formula.name").unwrap_or_default(),
                    FontId::proportional(size::LABEL + 1.0),
                    t.text,
                );
            }
            let ar = Rect::from_center_size(pos2(r.right() - 11.0, r.center().y), vec2(10.0, 10.0));
            self.icon(ui, painter, "chevron-down", ar, t.icon);
        }
        if let Some(g) = lay.grip {
            let r = rect_of(origin, ppp, g);
            self.icon(
                ui,
                painter,
                "dots-vertical",
                Rect::from_center_size(r.center(), vec2(12.0, 12.0)),
                t.text_dim,
            );
        }
        if let Some(fx) = lay.fx_group {
            let r = rect_of(origin, ppp, fx);
            painter.rect_filled(r, 3.0, t.card);
            painter.rect_stroke(r, 3.0, border, StrokeKind::Inside);
            // Icon centres measured from the group's left edge (units): cancel, enter, insert function.
            let items = [
                ("x", "formula.cancel", 15.0),
                ("check", "formula.enter", 43.0),
                ("math-function", "formula.insert-function", 84.0),
            ];
            for (icon, id, cx) in items {
                let c = pos2(r.left() + cx * u, r.center().y);
                let ir = Rect::from_center_size(c, vec2(22.0, 22.0));
                let st = host.state(id);
                let hit = Rect::from_center_size(c, vec2(24.0 * u * 1.25, r.height()));
                let resp = ui.interact(
                    hit,
                    Id::new(("fx", id)),
                    if st.enabled {
                        Sense::click()
                    } else {
                        Sense::hover()
                    },
                );
                if st.enabled {
                    self.button_bg(painter, hit, &resp, false);
                }
                // Office draws Cancel and Enter dim while no edit is running, and "fx" bright.
                let ink = if icon == "math-function" {
                    t.icon
                } else if st.enabled {
                    t.text
                } else {
                    t.icon_off
                };
                self.icon(ui, painter, icon, ir, ink);
                if resp.clicked() {
                    *run = Some((id.to_owned(), None));
                }
            }
            let ar = Rect::from_center_size(pos2(r.right() - 7.0, r.center().y), vec2(10.0, 10.0));
            self.icon(ui, painter, "chevron-down", ar, t.icon);
        }
        if let Some(f) = lay.formula_field {
            let r = rect_of(origin, ppp, f);
            painter.rect_filled(r, 3.0, t.card);
            painter.rect_stroke(r, 3.0, border, StrokeKind::Inside);
            let inner = Rect::from_min_max(
                pos2(r.left() + 6.0, r.top() + 2.0),
                pos2(r.right() - 26.0, r.bottom() - 2.0),
            );
            let drawn = ui
                .scope_builder(UiBuilder::new().max_rect(inner), |ui| {
                    ui.set_clip_rect(inner);
                    host.formula_ui(ui)
                })
                .inner;
            if !drawn {
                painter.text(
                    pos2(inner.left(), inner.top() + 9.0),
                    Align2::LEFT_CENTER,
                    host.value("formula.text").unwrap_or_default(),
                    FontId::proportional(size::LABEL + 1.0),
                    t.text,
                );
            }
            let cr =
                Rect::from_center_size(pos2(r.right() - 9.0, r.top() + 10.0), vec2(14.0, 14.0));
            let resp = ui.interact(cr.expand(4.0), Id::new("formula-expand"), Sense::click());
            self.button_bg(painter, cr.expand(4.0), &resp, false);
            self.icon(
                ui,
                painter,
                if self.formula_expanded {
                    "chevron-up"
                } else {
                    "chevron-down"
                },
                cr,
                t.text_dim,
            );
            if resp.clicked() {
                self.formula_expanded = !self.formula_expanded;
            }
        }
    }

    fn sheet_bar(
        &mut self,
        ui: &mut Ui,
        painter: &Painter,
        host: &mut dyn ShellHost,
        lay: &Layout<'_>,
        origin: Pos2,
        ppp: f32,
        run: &mut Option<(String, Option<f64>)>,
    ) {
        let t = self.theme;
        let Some(bar) = lay.sheet_bar else { return };
        let br = rect_of(origin, ppp, bar);
        painter.rect_filled(br, 0.0, t.frame);
        let slot = |name: &str| {
            lay.sheet_items
                .iter()
                .find(|i| i.chrome.ref_name.as_deref() == Some(name))
                .map(|i| rect_of(origin, ppp, i.rect))
        };
        let (Some(left), Some(right), Some(add)) =
            (slot("Scroll Left"), slot("Scroll Right"), slot("Add Sheet"))
        else {
            return;
        };
        // No room for a tab (a very narrow window): only the scroll and add buttons stay.
        let tabs_shown = slot("Sheet1").is_some();
        let first = slot("Sheet1").unwrap_or(add);
        let sheets = host.gallery("sheet.tabs");
        let active = host.value("sheet.active").unwrap_or_default();
        let font = FontId::proportional(size::TAB);
        let text_w = |s: &str| {
            painter
                .layout_no_wrap(s.to_owned(), font.clone(), t.text)
                .size()
                .x
        };
        // Excel's tab is 83 units wide for "Sheet1"; text widths vary, so keep that padding.
        // The first tab may be clipped by a narrow window, so take the padding from the spec width.
        let natural_w = self
            .spec
            .sheet_tabs
            .as_ref()
            .and_then(|s| {
                s.items
                    .iter()
                    .find(|c| c.ref_name.as_deref() == Some("Sheet1"))
            })
            .map_or(first.width(), |c| c.w as f32 * add.height() / 40.0);
        let pad = (natural_w - text_w("Sheet1")).max(8.0);
        let gap = if tabs_shown {
            (add.left() - first.right()).max(0.0)
        } else {
            0.0
        };
        // Tabs run from the first visible one; the tab area ends at the split handle.
        let limit = lay.sheet_split.map_or(br.left() + br.width() * 0.6, |r| {
            rect_of(origin, ppp, r).left()
        });
        let widths: Vec<f32> = sheets.iter().map(|(_, n)| text_w(n) + pad).collect();
        self.sheet_first = self.sheet_first.min(sheets.len().saturating_sub(1));
        let mut x = first.left();
        let mut rects = Vec::new();
        #[allow(clippy::needless_range_loop)]
        for i in self.sheet_first..sheets.len() {
            if !tabs_shown {
                break;
            }
            if x + widths[i] > limit && !rects.is_empty() {
                break;
            }
            rects.push((
                i,
                Rect::from_min_size(pos2(x, first.top()), vec2(widths[i], first.height())),
            ));
            x += widths[i];
        }
        let more = rects.last().is_some_and(|(i, _)| i + 1 < sheets.len());
        for (r, icon, enabled, delta) in [
            (left, "chevron-left", self.sheet_first > 0, -1_i32),
            (right, "chevron-right", more, 1_i32),
        ] {
            let resp = ui.interact(
                r,
                Id::new(("sheet-scroll", delta)),
                if enabled {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            if enabled {
                self.button_bg(painter, r, &resp, false);
            }
            self.icon(
                ui,
                painter,
                icon,
                Rect::from_center_size(r.center(), vec2(16.0, 16.0)),
                t.ink(enabled),
            );
            if resp.clicked() {
                self.sheet_first = (self.sheet_first as i32 + delta).max(0) as usize;
            }
        }
        for (i, r) in rects {
            let (vid, name) = &sheets[i];
            let on = *vid == active;
            let tab = Rect::from_min_max(
                pos2(r.left() + 2.5, r.top() + 2.0),
                pos2(r.right() - 2.5, r.bottom() - 3.0),
            );
            let resp = ui.interact(tab, Id::new(("sheet", vid)), Sense::click());
            if on {
                painter.rect_filled(tab, 2.0, t.tab_active);
                painter.line_segment(
                    [
                        pos2(tab.left() + 8.0, tab.bottom() - 1.5),
                        pos2(tab.right() - 8.0, tab.bottom() - 1.5),
                    ],
                    Stroke::new(2.5, t.grid_accent),
                );
            } else if resp.hovered() {
                painter.rect_filled(tab, 2.0, t.hover);
            }
            painter.text(
                tab.center(),
                Align2::CENTER_CENTER,
                name,
                font.clone(),
                if on { t.tab_active_text } else { t.text },
            );
            if resp.clicked() {
                *run = Some(("sheet.select".to_owned(), vid.parse::<f64>().ok()));
            }
        }
        // Add Sheet follows the last visible tab, with the measured gap.
        let add_r = Rect::from_min_size(pos2(x + gap, add.top()), add.size());
        let st = host.state("sheet.add");
        let resp = ui.interact(
            add_r,
            Id::new("sheet-add"),
            if st.enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        if st.enabled {
            self.button_bg(painter, add_r, &resp, false);
        }
        self.icon(
            ui,
            painter,
            "plus",
            Rect::from_center_size(add_r.center(), vec2(16.0, 16.0)),
            t.ink(st.enabled),
        );
        if resp.clicked() {
            *run = Some(("sheet.add".to_owned(), None));
        }
    }

    fn backstage_view(
        &mut self,
        ctx: &egui::Context,
        host: &mut dyn ShellHost,
        lay: &Layout<'_>,
        origin: Pos2,
        ppp: f32,
        run: &mut Option<(String, Option<f64>)>,
    ) {
        let t = self.theme;
        let k = ppp / REF_SCALE;
        let top = origin.y + lay.tab_rects.first().map_or(60.0, |r| r.y as f32 / ppp);
        let area = Rect::from_min_max(
            pos2(origin.x, top),
            pos2(
                origin.x + lay.window.w as f32 / ppp,
                origin.y + lay.window.h as f32 / ppp,
            ),
        );
        let nav = self.spec.backstage.clone();
        let nav_w = nav.nav_w as f32 / REF_SCALE;
        let first_y = nav.nav.first().map_or(0, |n| n.y);
        let mut close = false;
        let mut page = self.backstage.unwrap_or(1);
        Area::new(Id::new("backstage"))
            .order(Order::Foreground)
            .fixed_pos(area.min)
            .show(ctx, |ui| {
                let painter = ui.painter().clone();
                ui.allocate_rect(Rect::from_min_size(area.min, area.size()), Sense::click());
                painter.rect_filled(area, 0.0, t.backstage);
                painter.rect_filled(
                    Rect::from_min_size(area.min, vec2(nav_w, area.height())),
                    0.0,
                    t.backstage_nav,
                );
                for (i, n) in nav.nav.iter().enumerate() {
                    let y = area.top() + (n.y - first_y) as f32 / REF_SCALE;
                    let r = Rect::from_min_size(
                        pos2(area.left(), y),
                        vec2(nav_w, n.h as f32 / REF_SCALE),
                    );
                    let wired = !matches!(n.id.as_str(), "share" | "account" | "options");
                    let resp = ui.interact(
                        r,
                        Id::new(("bs", &n.id)),
                        if wired {
                            Sense::click()
                        } else {
                            Sense::hover()
                        },
                    );
                    if page == i && n.id != "back" {
                        painter.rect_filled(r, 0.0, t.hover);
                        painter.rect_filled(
                            Rect::from_min_size(r.min, vec2(3.0, r.height())),
                            0.0,
                            t.accent,
                        );
                    } else if resp.hovered() && wired {
                        painter.rect_filled(r, 0.0, t.hover);
                    }
                    let icon = match n.id.as_str() {
                        "back" => "arrow-left",
                        "home" => "home",
                        "new" => "file-plus",
                        "open" => "folder-open",
                        "share" => "share",
                        "info" => "info-circle",
                        "save" | "save-as" => "device-floppy",
                        "print" => "printer",
                        "export" => "file-export",
                        "close" => "door-exit",
                        "account" => "user-circle",
                        _ => "settings",
                    };
                    let tint = t.ink(wired);
                    self.icon(
                        ui,
                        &painter,
                        icon,
                        Rect::from_center_size(
                            pos2(r.left() + 28.0, r.center().y),
                            vec2(18.0, 18.0),
                        ),
                        tint,
                    );
                    if n.id != "back" {
                        painter.text(
                            pos2(r.left() + 56.0, r.center().y),
                            Align2::LEFT_CENTER,
                            &n.label,
                            FontId::proportional(size::NAV),
                            tint,
                        );
                    }
                    if resp.clicked() {
                        match n.id.as_str() {
                            "back" => close = true,
                            "home" | "new" | "open" | "info" => page = i,
                            other => {
                                *run = Some((format!("file.{other}"), None));
                                close = true;
                            }
                        }
                    }
                }
                let cx = area.left() + nav_w + 40.0 * k;
                let mut y = area.top() + 24.0;
                let id = nav.nav.get(page).map_or("home", |n| n.id.as_str());
                painter.text(
                    pos2(cx, y + 12.0),
                    Align2::LEFT_CENTER,
                    match id {
                        "new" => "New",
                        "open" => "Open",
                        "info" => "Info",
                        _ => "Home",
                    },
                    FontId::proportional(size::HEADING),
                    t.text,
                );
                y += 56.0;
                if matches!(id, "home" | "new") {
                    let r = Rect::from_min_size(pos2(cx, y), vec2(90.0, 115.0));
                    let resp = ui.interact(r, Id::new("bs-blank"), Sense::click());
                    painter.rect_filled(r, 0.0, Color32::WHITE);
                    painter.rect_stroke(
                        r,
                        0.0,
                        Stroke::new(if resp.hovered() { 2.0 } else { 1.0 }, t.accent),
                        StrokeKind::Inside,
                    );
                    let name = if self.kind == AppKind::Writer {
                        "Blank document"
                    } else {
                        "Blank workbook"
                    };
                    painter.text(
                        pos2(r.center().x, r.bottom() + 14.0),
                        Align2::CENTER_CENTER,
                        name,
                        FontId::proportional(size::LABEL + 1.0),
                        t.text,
                    );
                    if resp.clicked() {
                        *run = Some(("file.new".to_owned(), None));
                        close = true;
                    }
                    y += 150.0;
                }
                if matches!(id, "home" | "open") {
                    let b = Rect::from_min_size(pos2(cx, y), vec2(120.0, 32.0));
                    let resp = ui.interact(b, Id::new("bs-browse"), Sense::click());
                    painter.rect_filled(b, 4.0, if resp.hovered() { t.pressed } else { t.hover });
                    painter.text(
                        b.center(),
                        Align2::CENTER_CENTER,
                        "Browse...",
                        FontId::proportional(size::NAV),
                        t.text,
                    );
                    if resp.clicked() {
                        *run = Some(("file.open".to_owned(), None));
                        close = true;
                    }
                    y += 48.0;
                    painter.text(
                        pos2(cx, y),
                        Align2::LEFT_CENTER,
                        "Recent",
                        FontId::proportional(size::NAV + 2.0),
                        t.text,
                    );
                    y += 24.0;
                    for (i, path) in host.recent().iter().take(10).enumerate() {
                        let r =
                            Rect::from_min_size(pos2(cx, y), vec2(area.right() - cx - 40.0, 30.0));
                        let resp = ui.interact(r, Id::new(("recent", i)), Sense::click());
                        if resp.hovered() {
                            painter.rect_filled(r, 3.0, t.hover);
                        }
                        painter.text(
                            pos2(r.left() + 8.0, r.center().y),
                            Align2::LEFT_CENTER,
                            path,
                            FontId::proportional(size::LABEL + 1.0),
                            t.text,
                        );
                        if resp.clicked() {
                            *run = Some((format!("file.open-recent#{i}"), None));
                            close = true;
                        }
                        y += 32.0;
                    }
                }
                if id == "info" {
                    for (k, v) in host.info() {
                        painter.text(
                            pos2(cx, y),
                            Align2::LEFT_CENTER,
                            k,
                            FontId::proportional(size::NAV),
                            t.text_dim,
                        );
                        painter.text(
                            pos2(cx + 160.0, y),
                            Align2::LEFT_CENTER,
                            v,
                            FontId::proportional(size::NAV),
                            t.text,
                        );
                        y += 28.0;
                    }
                }
            });
        self.backstage = if close { None } else { Some(page) };
    }
}
