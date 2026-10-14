//! The Print page of the File tab (Backstage): settings on the left, a live preview on the right.
//!
//! The page owns one subtree of the `Ui` and is built into a rectangle the shell gives it, in
//! logical units. The shell sends it every `Emit` it receives ([`PrintPage::handle`]) and the page
//! says whether the event was its own. Settings that change the layout (paper, orientation,
//! margins, scaling) reach the document through the [`PrintSource`] and re-render the pages; the
//! preview is drawn from that render, so it shows what the printer gets.

use std::collections::HashMap;

use open365_gfx::Color;
use open365_print::{
    InkItem, PER_SHEET, Paper, PrintSystem, Product, RangeMode, Scaling, Sides, margin_presets,
    papers,
};
use open365_ui_core::{Content, Emit, Look, Node, NodeId, Rect, TextEdit, Token, Ui, Widget};

use super::model::{CUSTOM_MARGINS, DOCUMENT_PAPER, PrintModel};
use super::source::PrintSource;

/// What a node of the page does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ctl {
    Print,
    Copies,
    Printer,
    Range,
    Pages,
    Sides,
    Collate,
    Orientation,
    Paper,
    Margins,
    Scaling,
    ScalePercent,
    PerSheet,
    Prev,
    Next,
}

const LEFT: f32 = 28.0;
const COL_W: f32 = 340.0;

/// The Print page of one document.
#[derive(Debug)]
pub struct PrintPage {
    pub model: PrintModel,
    root: Option<NodeId>,
    bounds: Rect,
    controls: HashMap<NodeId, Ctl>,
    /// Nodes that are rebuilt after every change (summary, notes, preview).
    dynamic: Vec<NodeId>,
    /// Ids behind the dropdowns whose items are not a fixed list.
    paper_ids: Vec<&'static str>,
    margin_ids: Vec<&'static str>,
    pages_field: Option<NodeId>,
}

impl PrintPage {
    /// Reads the document's page setup and the printers; builds nothing yet.
    pub fn open(source: &dyn PrintSource, system: &dyn PrintSystem) -> Self {
        Self {
            model: PrintModel::new(source, system),
            root: None,
            bounds: Rect::new(0.0, 0.0, 0.0, 0.0),
            controls: HashMap::new(),
            dynamic: Vec::new(),
            paper_ids: Vec::new(),
            margin_ids: Vec::new(),
            pages_field: None,
        }
    }

    /// The control a node is, if it belongs to this page.
    #[must_use]
    pub fn control(&self, node: NodeId) -> Option<Ctl> {
        self.controls.get(&node).copied()
    }

    /// The first node of a control (tests and key tips).
    #[must_use]
    pub fn node_of(&self, ctl: Ctl) -> Option<NodeId> {
        self.controls
            .iter()
            .find_map(|(n, c)| (*c == ctl).then_some(*n))
    }

    /// Removes the page from the tree.
    pub fn close(&mut self, ui: &mut Ui) {
        if let Some(r) = self.root.take() {
            ui.tree.remove(r);
        }
        self.controls.clear();
        self.dynamic.clear();
        self.pages_field = None;
    }

    /// Builds (or rebuilds) the page into `bounds` of `parent`.
    pub fn build(
        &mut self,
        ui: &mut Ui,
        parent: NodeId,
        bounds: Rect,
        source: &mut dyn PrintSource,
    ) {
        self.close(ui);
        self.bounds = bounds;
        self.model.refresh(source);
        let root = ui.add(
            parent,
            Node::new(Widget::Panel, bounds).with_look(Look {
                fill: Some(Token::Card),
                still: true,
                ..Look::default()
            }),
        );
        self.root = Some(root);
        self.build_settings(ui, root);
        self.build_dynamic(ui, root);
    }

    #[allow(clippy::too_many_arguments)]
    fn put(
        &mut self,
        ui: &mut Ui,
        parent: NodeId,
        r: Rect,
        widget: Widget,
        label: &str,
        look: Look,
        ctl: Option<Ctl>,
    ) -> NodeId {
        let id = ui.add(
            parent,
            Node::new(widget, r).with_label(label).with_look(look),
        );
        if let Some(c) = ctl {
            self.controls.insert(id, c);
        }
        id
    }

    fn label(
        &mut self,
        ui: &mut Ui,
        parent: NodeId,
        r: Rect,
        text: &str,
        size: f32,
        ink: Token,
    ) -> NodeId {
        self.put(
            ui,
            parent,
            r,
            Widget::Label,
            text,
            Look {
                text_size: Some(size),
                ink: Some(ink),
                content: Content::TextOnly,
                still: true,
                ..Look::default()
            },
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn dropdown(
        &mut self,
        ui: &mut Ui,
        parent: NodeId,
        y: f32,
        name: &str,
        items: Vec<String>,
        selected: usize,
        ctl: Ctl,
    ) -> NodeId {
        self.put(
            ui,
            parent,
            Rect::new(LEFT, y, COL_W, 32.0),
            Widget::Dropdown {
                items,
                selected,
                open: false,
            },
            name,
            Look {
                fill: Some(Token::Field),
                border: Some(Token::FieldBorder),
                ..Look::default()
            },
            Some(ctl),
        )
    }

    fn build_settings(&mut self, ui: &mut Ui, root: NodeId) {
        let product = self.model.product;
        let s = self.model.settings.clone();
        self.label(
            ui,
            root,
            Rect::new(LEFT, 14.0, COL_W, 40.0),
            "Print",
            26.0,
            Token::Text,
        );

        // Print button, copies.
        self.put(
            ui,
            root,
            Rect::new(LEFT, 64.0, 96.0, 64.0),
            Widget::Button { toggled: None },
            "Print",
            Look {
                fill: Some(Token::Accent),
                ink: Some(Token::OnAccent),
                content: Content::TextOnly,
                text_size: Some(16.0),
                align: 0.5,
                radius: 3.0,
                ..Look::default()
            },
            Some(Ctl::Print),
        );
        self.label(
            ui,
            root,
            Rect::new(140.0, 66.0, 70.0, 30.0),
            "Copies:",
            14.0,
            Token::Text,
        );
        self.put(
            ui,
            root,
            Rect::new(212.0, 66.0, 90.0, 30.0),
            Widget::Spinner {
                value: f64::from(s.copies),
                min: 1.0,
                max: 32_767.0,
                step: 1.0,
                edit: TextEdit::new(&s.copies.to_string()),
            },
            "Copies",
            Look::default(),
            Some(Ctl::Copies),
        );

        // Printer.
        self.label(
            ui,
            root,
            Rect::new(LEFT, 146.0, COL_W, 24.0),
            "Printer",
            15.0,
            Token::Text,
        );
        let names: Vec<String> = if self.model.printers.is_empty() {
            vec!["No printers installed".to_owned()]
        } else {
            self.model
                .printers
                .iter()
                .map(|p| {
                    if p.is_default {
                        format!("{} (default)", p.name)
                    } else {
                        p.name.clone()
                    }
                })
                .collect()
        };
        let chosen = self
            .model
            .printers
            .iter()
            .position(|p| Some(&p.system_name) == s.printer.as_ref())
            .unwrap_or(0);
        let d = self.dropdown(ui, root, 172.0, "Printer", names, chosen, Ctl::Printer);
        if self.model.printers.is_empty()
            && let Some(n) = ui.tree.get_mut(d)
        {
            n.enabled = false;
        }

        // Settings.
        self.label(
            ui,
            root,
            Rect::new(LEFT, 220.0, COL_W, 24.0),
            "Settings",
            15.0,
            Token::Text,
        );
        let rows = 8.0;
        let pitch = ((self.bounds.h - 250.0 - 60.0) / rows).clamp(30.0, 38.0);
        let mut y = 248.0;
        let mut next = || {
            let v = y;
            y += pitch;
            v
        };

        let range = match s.range_mode {
            RangeMode::All => 0,
            RangeMode::CurrentPage => 1,
            RangeMode::Custom => 2,
        };
        self.dropdown(
            ui,
            root,
            next(),
            "Pages to print",
            vec![
                "Print All Pages".into(),
                "Print Current Page".into(),
                "Custom Print".into(),
            ],
            range,
            Ctl::Range,
        );

        let yy = next();
        self.label(
            ui,
            root,
            Rect::new(LEFT, yy, 56.0, 30.0),
            "Pages:",
            14.0,
            Token::Text,
        );
        let field = self.put(
            ui,
            root,
            Rect::new(LEFT + 60.0, yy, COL_W - 60.0, 30.0),
            Widget::TextField(TextEdit::new(&s.range_text)),
            "Pages",
            Look::default(),
            Some(Ctl::Pages),
        );
        if s.range_mode != RangeMode::Custom
            && let Some(n) = ui.tree.get_mut(field)
        {
            n.enabled = false;
        }
        self.pages_field = Some(field);

        let sides = match s.sides {
            Sides::OneSided => 0,
            Sides::TwoSidedLongEdge => 1,
            Sides::TwoSidedShortEdge => 2,
        };
        self.dropdown(
            ui,
            root,
            next(),
            "Sides",
            vec![
                "Print One Sided".into(),
                "Print on Both Sides, Flip on long edge".into(),
                "Print on Both Sides, Flip on short edge".into(),
            ],
            sides,
            Ctl::Sides,
        );
        self.dropdown(
            ui,
            root,
            next(),
            "Collation",
            vec!["Collated".into(), "Uncollated".into()],
            usize::from(!s.collate),
            Ctl::Collate,
        );
        self.dropdown(
            ui,
            root,
            next(),
            "Orientation",
            vec![
                "Portrait Orientation".into(),
                "Landscape Orientation".into(),
            ],
            usize::from(s.landscape),
            Ctl::Orientation,
        );

        // Paper.
        let mut ids: Vec<&'static str> = Vec::new();
        let mut items: Vec<String> = Vec::new();
        if s.paper_id == DOCUMENT_PAPER {
            ids.push(DOCUMENT_PAPER);
            items.push("Document size".to_owned());
        }
        for p in papers() {
            ids.push(p.id);
            items.push(paper_label(p));
        }
        let sel = ids.iter().position(|i| *i == s.paper_id).unwrap_or(0);
        self.paper_ids = ids;
        self.dropdown(ui, root, next(), "Paper size", items, sel, Ctl::Paper);

        // Margins.
        let mut ids: Vec<&'static str> = Vec::new();
        let mut items: Vec<String> = Vec::new();
        if s.margins_id == CUSTOM_MARGINS {
            ids.push(CUSTOM_MARGINS);
            items.push("Custom Margins".to_owned());
        }
        for m in margin_presets(product) {
            ids.push(m.id);
            items.push(format!("{} Margins", m.label));
        }
        let sel = ids.iter().position(|i| *i == s.margins_id).unwrap_or(0);
        self.margin_ids = ids;
        self.dropdown(ui, root, next(), "Margins", items, sel, Ctl::Margins);

        match product {
            Product::Sheets => {
                let sel = match s.scaling {
                    Scaling::None => 0,
                    Scaling::FitSheet => 1,
                    Scaling::FitColumns => 2,
                    Scaling::FitRows => 3,
                    Scaling::Custom(_) => 4,
                };
                self.dropdown(
                    ui,
                    root,
                    next(),
                    "Scaling",
                    vec![
                        "No Scaling".into(),
                        "Fit Sheet on One Page".into(),
                        "Fit All Columns on One Page".into(),
                        "Fit All Rows on One Page".into(),
                        "Custom Scaling Options".into(),
                    ],
                    sel,
                    Ctl::Scaling,
                );
                if let Scaling::Custom(p) = s.scaling {
                    let yy = next();
                    self.label(
                        ui,
                        root,
                        Rect::new(LEFT, yy, 120.0, 30.0),
                        "Scale (%):",
                        14.0,
                        Token::Text,
                    );
                    self.put(
                        ui,
                        root,
                        Rect::new(LEFT + 124.0, yy, 100.0, 30.0),
                        Widget::Spinner {
                            value: f64::from(p),
                            min: 10.0,
                            max: 400.0,
                            step: 5.0,
                            edit: TextEdit::new(&p.to_string()),
                        },
                        "Scale",
                        Look::default(),
                        Some(Ctl::ScalePercent),
                    );
                }
            }
            Product::Writer => {
                let items = PER_SHEET
                    .iter()
                    .map(|n| format!("{n} Page{} Per Sheet", if *n == 1 { "" } else { "s" }))
                    .collect();
                let sel = PER_SHEET
                    .iter()
                    .position(|n| *n == s.per_sheet)
                    .unwrap_or(0);
                self.dropdown(
                    ui,
                    root,
                    next(),
                    "Pages per sheet",
                    items,
                    sel,
                    Ctl::PerSheet,
                );
            }
        }
    }

    /// The parts that change with every edit: summary, printer note, message and the preview.
    fn build_dynamic(&mut self, ui: &mut Ui, root: NodeId) {
        for id in std::mem::take(&mut self.dynamic) {
            ui.tree.remove(id);
        }
        self.controls.retain(|id, _| ui.tree.contains(*id));
        let ok = self.model.pdf_error.is_none() && self.model.selected_pages().is_ok();
        if let Some(n) = self.node_of(Ctl::Print).and_then(|id| ui.tree.get_mut(id)) {
            n.enabled = ok && self.model.settings.printer.is_some();
        }
        if let Some(f) = self.pages_field.and_then(|id| ui.tree.get_mut(id)) {
            f.enabled = self.model.settings.range_mode == RangeMode::Custom;
        }
        let mut made = Vec::new();
        let summary = self.model.summary();
        let ink = if ok { Token::TextDim } else { Token::Text };
        made.push(self.label(
            ui,
            root,
            Rect::new(140.0, 100.0, 228.0, 26.0),
            &summary,
            12.0,
            ink,
        ));
        if let Some(note) = self.model.printer_note() {
            made.push(self.label(
                ui,
                root,
                Rect::new(LEFT, 206.0, COL_W, 16.0),
                &note,
                11.0,
                Token::TextDim,
            ));
        }
        if let Some(msg) = self.model.message.clone() {
            let y = (self.bounds.h - 40.0).max(0.0);
            made.push(self.label(
                ui,
                root,
                Rect::new(LEFT, y, COL_W, 24.0),
                &msg,
                12.0,
                Token::TextDim,
            ));
        }
        made.extend(self.build_preview(ui, root));
        self.dynamic = made;
    }

    fn build_preview(&mut self, ui: &mut Ui, root: NodeId) -> Vec<NodeId> {
        let mut made = Vec::new();
        let x0 = LEFT + COL_W + LEFT;
        let area = Rect::new(
            x0,
            16.0,
            (self.bounds.w - x0 - LEFT).max(40.0),
            (self.bounds.h - 16.0 - 64.0).max(40.0),
        );
        let back = self.put(
            ui,
            root,
            area,
            Widget::Panel,
            "",
            Look {
                fill: Some(Token::Frame),
                still: true,
                ..Look::default()
            },
            None,
        );
        made.push(back);
        let sheets = self.model.sheets();
        let total = sheets.len();
        if self.model.pdf().is_none() {
            let text = self
                .model
                .pdf_error
                .clone()
                .unwrap_or_else(|| "Nothing to preview.".to_owned());
            made.push(self.label(
                ui,
                back,
                Rect::new(16.0, 16.0, area.w - 32.0, 28.0),
                &text,
                14.0,
                Token::Text,
            ));
            return made;
        }
        if sheets.is_empty() {
            let text = self.model.summary();
            made.push(self.label(
                ui,
                back,
                Rect::new(16.0, 16.0, area.w - 32.0, 28.0),
                &text,
                14.0,
                Token::Text,
            ));
            return made;
        }
        let at = self.model.viewing.min(total - 1);
        let (sw, sh) = self.model.sheet_size();
        let k = ((area.w - 40.0) / sw as f32)
            .min((area.h - 40.0) / sh as f32)
            .max(0.05);
        let k = f64::from(k);
        let (pw, ph) = (sw * k, sh * k);
        let sheet = self.put(
            ui,
            back,
            Rect::new(
                ((f64::from(area.w) - pw) / 2.0) as f32,
                ((f64::from(area.h) - ph) / 2.0) as f32,
                pw as f32,
                ph as f32,
            ),
            Widget::Panel,
            &format!("Page {} of {total}", at + 1),
            Look {
                fill_rgb: Some(Color::rgba(255, 255, 255, 255)),
                border_rgb: Some(Color::rgba(190, 190, 190, 255)),
                still: true,
                ..Look::default()
            },
            None,
        );
        if let Some(n) = ui.tree.get_mut(sheet) {
            n.clip_children = true;
        }
        // Read everything the marks need before the tree is touched.
        let marks: Vec<(open365_print::Placement, open365_print::Ink)> = self
            .model
            .pdf()
            .map(|pdf| {
                pdf.placements(&sheets[at], (sw, sh), self.model.settings.per_sheet)
                    .into_iter()
                    .filter_map(|pl| pdf.ink(pl.source).map(|ink| (pl, ink)))
                    .collect()
            })
            .unwrap_or_default();
        let mut drawn = 0usize;
        for (pl, ink) in marks {
            // Page space (y up) to the displayed sheet (y down).
            let map = |x: f64, y: f64, w: f64, h: f64| {
                let left = (pl.dx + x * pl.scale) * k;
                let top = (sh - (pl.dy + (y + h) * pl.scale)) * k;
                Rect::new(
                    left as f32,
                    top as f32,
                    (w * pl.scale * k) as f32,
                    (h * pl.scale * k) as f32,
                )
            };
            for item in &ink.items {
                if drawn >= 1_800 {
                    break;
                }
                let (r, g) = match *item {
                    InkItem::Text { x, y, w, h, gray } => {
                        let v = (u16::from(gray) + 90).min(200) as u8;
                        (map(x, y - h * 0.2, w, h), Color::rgba(v, v, v, 255))
                    }
                    InkItem::Rect { x, y, w, h, gray } => {
                        (map(x, y, w, h), Color::rgba(gray, gray, gray, 255))
                    }
                    InkItem::Picture { x, y, w, h } => {
                        (map(x, y, w, h), Color::rgba(215, 215, 215, 255))
                    }
                    InkItem::Line {
                        x1,
                        y1,
                        x2,
                        y2,
                        width,
                        gray,
                    } => {
                        let (x, y) = (x1.min(x2), y1.min(y2));
                        let (w, h) = ((x1 - x2).abs().max(width), (y1 - y2).abs().max(width));
                        (map(x, y, w, h), Color::rgba(gray, gray, gray, 255))
                    }
                };
                let r = Rect::new(r.x, r.y, r.w.max(1.0), r.h.max(1.0));
                self.put(
                    ui,
                    sheet,
                    r,
                    Widget::Panel,
                    "",
                    Look {
                        fill_rgb: Some(g),
                        still: true,
                        ..Look::default()
                    },
                    None,
                );
                drawn += 1;
            }
        }

        // Page navigation under the preview.
        let ny = area.y + area.h + 14.0;
        let cx = area.x + area.w / 2.0;
        let nav = Look {
            content: Content::TextOnly,
            align: 0.5,
            text_size: Some(13.0),
            ..Look::default()
        };
        let prev = self.put(
            ui,
            root,
            Rect::new(cx - 190.0, ny, 110.0, 30.0),
            Widget::Button { toggled: None },
            "Previous page",
            nav,
            Some(Ctl::Prev),
        );
        let next = self.put(
            ui,
            root,
            Rect::new(cx + 80.0, ny, 110.0, 30.0),
            Widget::Button { toggled: None },
            "Next page",
            nav,
            Some(Ctl::Next),
        );
        if let Some(n) = ui.tree.get_mut(prev) {
            n.enabled = at > 0;
        }
        if let Some(n) = ui.tree.get_mut(next) {
            n.enabled = at + 1 < total;
        }
        made.push(prev);
        made.push(next);
        let pos = self.label(
            ui,
            root,
            Rect::new(cx - 70.0, ny, 140.0, 30.0),
            &format!("{} of {total}", at + 1),
            14.0,
            Token::Text,
        );
        if let Some(n) = ui.tree.get_mut(pos) {
            n.look.align = 0.5;
        }
        made.push(pos);
        made
    }

    /// Handles one event. Returns true when the event belonged to this page.
    pub fn handle(
        &mut self,
        ui: &mut Ui,
        emit: &Emit,
        source: &mut dyn PrintSource,
        system: &mut dyn PrintSystem,
    ) -> bool {
        let Some(root) = self.root else { return false };
        let node = match emit {
            Emit::Clicked(n)
            | Emit::Selected(n, _)
            | Emit::ValueChanged(n, _)
            | Emit::TextChanged(n, _)
            | Emit::TextCommitted(n, _) => *n,
            _ => return false,
        };
        let Some(ctl) = self.control(node) else {
            return false;
        };
        let mut rebuild_all = false;
        match (ctl, emit) {
            (Ctl::Print, Emit::Clicked(_)) => {
                let _ = self.model.print(system);
            }
            (Ctl::Prev, Emit::Clicked(_)) => self.model.step(-1),
            (Ctl::Next, Emit::Clicked(_)) => self.model.step(1),
            (Ctl::Copies, Emit::ValueChanged(_, v)) => self.model.settings.copies = *v as u32,
            (Ctl::Printer, Emit::Selected(_, i)) => {
                if let Some(p) = self.model.printers.get(*i) {
                    self.model.settings.printer = Some(p.system_name.clone());
                }
            }
            (Ctl::Range, Emit::Selected(_, i)) => {
                let mode = [RangeMode::All, RangeMode::CurrentPage, RangeMode::Custom]
                    .get(*i)
                    .copied()
                    .unwrap_or_default();
                self.model.set_range_mode(mode);
            }
            (Ctl::Pages, Emit::TextChanged(_, t) | Emit::TextCommitted(_, t)) => {
                self.model.set_range_text(t);
            }
            (Ctl::Sides, Emit::Selected(_, i)) => {
                self.model.settings.sides = [
                    Sides::OneSided,
                    Sides::TwoSidedLongEdge,
                    Sides::TwoSidedShortEdge,
                ]
                .get(*i)
                .copied()
                .unwrap_or_default();
            }
            (Ctl::Collate, Emit::Selected(_, i)) => self.model.settings.collate = *i == 0,
            (Ctl::Orientation, Emit::Selected(_, i)) => {
                self.model.set_landscape(source, *i == 1);
            }
            (Ctl::Paper, Emit::Selected(_, i)) => {
                if let Some(id) = self.paper_ids.get(*i).copied()
                    && id != DOCUMENT_PAPER
                {
                    self.model.set_paper(source, id);
                }
            }
            (Ctl::Margins, Emit::Selected(_, i)) => {
                if let Some(id) = self.margin_ids.get(*i).copied()
                    && id != CUSTOM_MARGINS
                {
                    self.model.set_margins(source, id);
                }
            }
            (Ctl::Scaling, Emit::Selected(_, i)) => {
                let keep = match self.model.settings.scaling {
                    Scaling::Custom(p) => p,
                    _ => 100,
                };
                let scaling = [
                    Scaling::None,
                    Scaling::FitSheet,
                    Scaling::FitColumns,
                    Scaling::FitRows,
                    Scaling::Custom(keep),
                ]
                .get(*i)
                .copied()
                .unwrap_or_default();
                let was_custom = matches!(self.model.settings.scaling, Scaling::Custom(_));
                rebuild_all = was_custom != matches!(scaling, Scaling::Custom(_));
                self.model.set_scaling(source, scaling);
            }
            (Ctl::ScalePercent, Emit::ValueChanged(_, v)) => {
                self.model.set_scaling(source, Scaling::Custom(*v as u32));
            }
            (Ctl::PerSheet, Emit::Selected(_, i)) => {
                if let Some(n) = PER_SHEET.get(*i) {
                    self.model.set_per_sheet(*n);
                }
            }
            _ => return true,
        }
        self.model.refresh(source);
        if rebuild_all {
            let parent = ui.tree.get(root).and_then(|n| n.parent);
            if let Some(p) = parent {
                let bounds = self.bounds;
                self.build(ui, p, bounds, source);
            }
        } else {
            self.build_dynamic(ui, root);
        }
        true
    }
}

/// The label of a paper size: its name and measures.
fn paper_label(p: &Paper) -> String {
    if p.ipp_media.starts_with("na_") {
        format!(
            "{} ({} x {} in)",
            p.label,
            trim(p.width_pt / 72.0),
            trim(p.height_pt / 72.0)
        )
    } else {
        format!(
            "{} ({} x {} cm)",
            p.label,
            trim(p.width_pt / 72.0 * 2.54),
            trim(p.height_pt / 72.0 * 2.54)
        )
    }
}

fn trim(v: f64) -> String {
    let s = format!("{v:.1}");
    s.trim_end_matches(".0").to_owned()
}
