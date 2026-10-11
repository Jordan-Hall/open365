//! The egui side of Word's rulers: layout of the document area with the bands, painting, and pointer handling.
//! Geometry, hit testing and drag arithmetic are in `ruler.rs` (pure, in `u`); this file only converts `u` to
//! points (1 u = 0.8 points) and draws.

use egui::epaint::{PathShape, TextShape};
use egui::{CursorIcon, FontId, Painter, StrokeKind};

use super::*;
use crate::ruler::{
    self, BAND_U, CORNER_U, DIGIT_Y, RulerEdit, RulerHit, RulerInfo, RulerTab, RulerTabStop,
    TICK_SMALL_Y, TICK_TALL_Y, TickKind, URect, VBAND_U,
};

/// Points per `u`.
const U: f32 = 0.8;

/// Colours of the rulers (the band is drawn in the canvas colour; digits and ticks use the page's automatic text colour).
#[derive(Debug, Clone, Copy)]
pub struct RulerColors {
    pub band: Color32,
    pub track: Color32,
    pub line: Color32,
    pub marker: Color32,
    pub marker_border: Color32,
    pub glyph: Color32,
}

impl Default for RulerColors {
    fn default() -> Self {
        Self {
            band: Color32::from_rgb(0x09, 0x09, 0x09),
            track: Color32::from_rgb(0x29, 0x29, 0x29),
            line: Color32::from_rgb(0x61, 0x61, 0x61),
            marker: Color32::from_rgb(0xbd, 0xbd, 0xbd),
            marker_border: Color32::from_rgb(0x61, 0x61, 0x61),
            glyph: Color32::from_rgb(0xbd, 0xbd, 0xbd),
        }
    }
}

/// Ruler state of a [`DocView`].
#[derive(Debug, Default)]
pub(super) struct RulerState {
    pub on: bool,
    pub colors: RulerColors,
    /// The kind a click on the track adds (the corner button cycles it).
    pub corner: RulerTab,
    drag: Option<RulerHit>,
}

/// The paragraph a ruler edit touches and the command that performs it.
/// The same edit aimed at paragraph `p`.
fn with_para(edit: &RulerEdit, p: usize) -> RulerEdit {
    let mut e = edit.clone();
    match &mut e {
        RulerEdit::SetIndents { para, .. }
        | RulerEdit::AddTab { para, .. }
        | RulerEdit::MoveTab { para, .. }
        | RulerEdit::ClearTab { para, .. } => *para = p,
    }
    e
}

fn ruler_command(edit: &RulerEdit) -> (usize, Command) {
    match *edit {
        RulerEdit::SetIndents {
            para,
            left_pt,
            first_line_pt,
            right_pt,
        } => (
            para,
            Command::DocxSetIndents {
                para,
                left_pt,
                first_line_pt,
                right_pt,
            },
        ),
        RulerEdit::AddTab { para, pos_pt, kind } => (
            para,
            Command::DocxAddTab {
                para,
                pos_pt,
                kind: match kind {
                    RulerTab::Left => TabAlign::Left,
                    RulerTab::Center => TabAlign::Center,
                    RulerTab::Right => TabAlign::Right,
                    RulerTab::Decimal => TabAlign::Decimal,
                    RulerTab::Bar => TabAlign::Bar,
                },
            },
        ),
        RulerEdit::MoveTab {
            para,
            from_pt,
            to_pt,
        } => (
            para,
            Command::DocxMoveTab {
                para,
                from_pt,
                to_pt,
            },
        ),
        RulerEdit::ClearTab { para, pos_pt } => (para, Command::DocxClearTab { para, pos_pt }),
    }
}

fn pt(v: f64) -> f32 {
    v as f32 * U
}

fn urect(origin: Pos2, r: &URect) -> Rect {
    Rect::from_min_max(
        Pos2::new(origin.x + pt(r.x0), origin.y + pt(r.y0)),
        Pos2::new(origin.x + pt(r.x1), origin.y + pt(r.y1)),
    )
}

impl DocView {
    /// Shows or hides the rulers (View > Ruler). Off by default in the view, on in Writer as in Word.
    pub fn set_ruler(&mut self, on: bool) {
        self.ruler.on = on;
    }

    #[must_use]
    pub fn ruler(&self) -> bool {
        self.ruler.on
    }

    pub fn set_ruler_colors(&mut self, colors: RulerColors) {
        self.ruler.colors = colors;
    }

    /// The kind of tab a click on the ruler adds.
    #[must_use]
    pub fn ruler_tab_kind(&self) -> RulerTab {
        self.ruler.corner
    }

    /// What the rulers show for the paragraph holding the caret: its page, indents and tab stops.
    #[must_use]
    pub fn ruler_info(&self, target: &Target) -> Option<RulerInfo> {
        let doc = target.docx()?;
        let para = self.caret.para;
        let model = self.models.get(para)?;
        let mut info = self.page_ruler_info(self.page_of(para))?;
        info.para = para;
        info.indent_left_pt = f64::from(model.para.indent_left_pt);
        info.indent_right_pt = f64::from(model.para.indent_right_pt);
        info.first_line_pt = f64::from(model.para.first_line_pt);
        if let (Some(p), Some(styles)) = (doc.paragraph(para), doc.styles()) {
            info.tabs = styles
                .paragraph_flow_props(p)
                .tabs
                .iter()
                .filter_map(|t| {
                    use open365_docx::TabKind as K;
                    let kind = match t.kind {
                        K::Left => RulerTab::Left,
                        K::Center => RulerTab::Center,
                        K::Right => RulerTab::Right,
                        K::Decimal => RulerTab::Decimal,
                        K::Bar => RulerTab::Bar,
                        K::Num => return None,
                    };
                    Some(RulerTabStop {
                        pos_pt: f64::from(t.pos_pt()),
                        kind,
                    })
                })
                .collect();
        }
        Some(info)
    }

    /// Page geometry of `page` with no paragraph attributes.
    fn page_ruler_info(&self, page: usize) -> Option<RulerInfo> {
        let s = &self.engine.layout().pages.get(page)?.section;
        Some(RulerInfo {
            para: 0,
            page,
            page_width_pt: s.page_width,
            page_height_pt: s.page_height,
            margin_left_pt: s.margin_left,
            margin_right_pt: s.margin_right,
            margin_top_pt: s.margin_top,
            margin_bottom_pt: s.margin_bottom,
            indent_left_pt: 0.0,
            first_line_pt: 0.0,
            indent_right_pt: 0.0,
            tabs: Vec::new(),
        })
    }

    /// Runs a ruler edit through the command layer (`DocxSetIndents`, `DocxAddTab`, `DocxMoveTab`,
    /// `DocxClearTab`), once for every paragraph of the selection when the edited paragraph is inside it (as
    /// Word does; each paragraph is its own undo step). Moving or clearing a tab only touches the paragraphs
    /// that have that stop. False, with the reason in [`DocView::status`], when no paragraph changed.
    pub fn apply_ruler_edit(&mut self, target: &mut Target, edit: &RulerEdit) -> bool {
        let (para, _) = ruler_command(edit);
        let selected = self.selected_paragraphs();
        let paras: Vec<usize> = if selected.contains(&para) {
            selected.collect()
        } else {
            vec![para]
        };
        let mut changed = 0;
        for p in paras {
            let (_, command) = ruler_command(&with_para(edit, p));
            if self.run(target, command).is_some() {
                self.refresh_para(target, p);
                changed += 1;
            }
        }
        if changed > 0 {
            self.status = None;
        }
        changed > 0
    }

    /// The document area with the horizontal band on top and the vertical band on the left. The canvas gets what is left,
    /// so the page moves down by one band height and right by half of the vertical band.
    pub(super) fn show_with_rulers(&mut self, ui: &mut Ui, target: &mut Target) -> bool {
        let rect = ui.max_rect();
        let band = Rect::from_min_size(rect.min, vec2(rect.width(), pt(BAND_U)));
        let vband = Rect::from_min_max(
            Pos2::new(rect.min.x, band.max.y),
            Pos2::new(rect.min.x + pt(VBAND_U), rect.max.y),
        );
        let rest = Rect::from_min_max(Pos2::new(vband.max.x, band.max.y), rect.max);
        let mut dragging = false;
        ui.scope_builder(UiBuilder::new().max_rect(rest), |ui| {
            dragging = self.show_canvas(ui, target);
        });
        self.paint_rulers(ui, target, band, vband);
        dragging
    }

    fn paint_rulers(&mut self, ui: &mut Ui, target: &mut Target, band: Rect, vband: Rect) {
        let c = self.ruler.colors;
        let painter = ui.painter().clone();
        painter.rect_filled(band, 0.0, c.band);
        painter.rect_filled(vband, 0.0, c.band);
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(band.min.x, band.min.y + pt(BAND_U - 1.0)),
                vec2(band.width(), pt(1.0)),
            ),
            0.0,
            c.line,
        );
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(vband.min.x + pt(VBAND_U - 1.0), vband.min.y),
                vec2(pt(1.0), vband.height()),
            ),
            0.0,
            c.line,
        );
        let k = f64::from(self.scale) / f64::from(U);

        self.paint_vertical(&painter, vband, k);

        let corner = Rect::from_min_size(band.min, vec2(pt(CORNER_U), pt(BAND_U - 1.0)));
        self.paint_corner(ui, &painter, corner);

        let Some(info) = self.ruler_info(target) else {
            return;
        };
        let origin = self.page_origin(self.canvas, info.page);
        let track_band = Rect::from_min_max(
            Pos2::new(corner.max.x, band.min.y),
            Pos2::new(band.max.x, band.max.y),
        );
        let id = Id::new("open365-ruler-band");
        let resp = ui.interact(track_band, id, Sense::click_and_drag());
        let band_origin = Pos2::new(origin.x, band.min.y);
        let to_u = |p: Pos2| {
            (
                (f64::from(p.x - origin.x)) / f64::from(U),
                f64::from(p.y - band.min.y) / f64::from(U),
            )
        };

        let h0 = ruler::horizontal(&info, k);
        if resp.drag_started()
            && let Some(p) = ui.input(|i| i.pointer.press_origin())
        {
            let (x, y) = to_u(p);
            self.ruler.drag = ruler::hit_test(&h0, x, y);
        }
        let mut shown = info.clone();
        if resp.dragged()
            && let (Some(hit), Some(p)) = (self.ruler.drag, ui.input(|i| i.pointer.latest_pos()))
        {
            let (x, y) = to_u(p);
            if let Some(edit) = ruler::drag_edit(&info, k, hit, x, y) {
                shown = ruler::preview(&info, &edit);
            }
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        if resp.drag_stopped() {
            if let (Some(hit), Some(p)) =
                (self.ruler.drag.take(), ui.input(|i| i.pointer.latest_pos()))
            {
                let (x, y) = to_u(p);
                if let Some(edit) = ruler::drag_edit(&info, k, hit, x, y) {
                    self.apply_ruler_edit(target, &edit);
                }
            }
        } else if resp.clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            let (x, y) = to_u(p);
            if let Some(RulerHit::Track(x)) = ruler::hit_test(&h0, x, y)
                && let Some(edit) = ruler::click_edit(&info, k, x, self.ruler.corner)
            {
                self.apply_ruler_edit(target, &edit);
            }
        }
        if resp.hovered()
            && !resp.dragged()
            && let Some(p) = resp.hover_pos()
        {
            let (x, y) = to_u(p);
            if matches!(
                ruler::hit_test(&h0, x, y),
                Some(RulerHit::Marker(_) | RulerHit::Tab(_))
            ) {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            }
        }

        let h = ruler::horizontal(&shown, k);
        let clip = painter.with_clip_rect(track_band);
        self.paint_horizontal(&clip, band_origin, &h);
    }

    fn paint_horizontal(&self, p: &Painter, o: Pos2, h: &ruler::HRuler) {
        let c = self.ruler.colors;
        let ink = self.auto_text;
        p.rect_filled(urect(o, &h.track), 0.0, c.track);
        for t in &h.ticks {
            let y = if t.kind == TickKind::Tall {
                TICK_TALL_Y
            } else {
                TICK_SMALL_Y
            };
            p.rect_filled(urect(o, &URect::new(t.at, t.at + 1.0, y.0, y.1)), 0.0, ink);
        }
        let font = FontId::proportional(10.0);
        for d in &h.digits {
            p.text(
                Pos2::new(
                    o.x + pt(d.at + 0.5),
                    o.y + pt((DIGIT_Y.0 + DIGIT_Y.1) / 2.0),
                ),
                egui::Align2::CENTER_CENTER,
                d.number.to_string(),
                font.clone(),
                ink,
            );
        }
        for g in &h.tabs {
            self.paint_glyph(p, o, g);
        }
        for m in [&h.right, &h.hanging, &h.first_line] {
            let pts: Vec<Pos2> = m
                .outline
                .iter()
                .map(|&(x, y)| Pos2::new(o.x + pt(x), o.y + pt(y)))
                .collect();
            p.add(Shape::Path(PathShape::convex_polygon(
                pts,
                c.marker,
                Stroke::new(1.0, c.marker_border),
            )));
        }
        if let Some(b) = &h.left_box.box_rect {
            let r = urect(o, b);
            p.rect_filled(r, 0.0, c.marker);
            p.rect_stroke(
                r,
                0.0,
                Stroke::new(1.0, c.marker_border),
                StrokeKind::Inside,
            );
        }
    }

    fn paint_glyph(&self, p: &Painter, o: Pos2, g: &ruler::TabGlyph) {
        let col = self.ruler.colors.glyph;
        p.rect_filled(urect(o, &g.stem), 0.0, col);
        if g.kind != RulerTab::Bar {
            p.rect_filled(urect(o, &g.foot), 0.0, col);
        }
        if let Some(d) = &g.dot {
            p.rect_filled(urect(o, d), 0.0, col);
        }
    }

    /// The tab selector at the left end of the band; a click cycles the kind.
    fn paint_corner(&mut self, ui: &mut Ui, p: &Painter, corner: Rect) {
        let resp = ui.interact(corner, Id::new("open365-ruler-corner"), Sense::click());
        if resp.clicked() {
            self.ruler.corner = self.ruler.corner.next();
        }
        let g = ruler::corner_glyph(self.ruler.corner);
        self.paint_glyph(p, corner.min, &g);
    }

    /// The vertical ruler of every visible page: the horizontal one turned on its side, digits rotated.
    fn paint_vertical(&self, painter: &Painter, vband: Rect, k: f64) {
        let c = self.ruler.colors;
        let ink = self.auto_text;
        let p = painter.with_clip_rect(vband);
        let (first, last) = self.visible_pages;
        for page in first..=last {
            let Some(info) = self.page_ruler_info(page) else {
                continue;
            };
            let v = ruler::vertical(&info, k);
            let top = self.page_origin(self.canvas, page).y;
            let o = Pos2::new(vband.min.x, top);
            p.rect_filled(urect(o, &v.track), 0.0, c.track);
            for t in &v.ticks {
                let x = if t.kind == TickKind::Tall {
                    TICK_TALL_Y
                } else {
                    TICK_SMALL_Y
                };
                p.rect_filled(urect(o, &URect::new(x.0, x.1, t.at, t.at + 1.0)), 0.0, ink);
            }
            for d in &v.digits {
                let galley =
                    painter.layout_no_wrap(d.number.to_string(), FontId::proportional(10.0), ink);
                let cx = o.x + pt((DIGIT_Y.0 + DIGIT_Y.1) / 2.0);
                let cy = o.y + pt(d.at + 0.5);
                let pos = Pos2::new(cx - galley.size().y / 2.0, cy + galley.size().x / 2.0);
                p.add(Shape::Text(
                    TextShape::new(pos, galley, ink).with_angle(-std::f32::consts::FRAC_PI_2),
                ));
            }
        }
    }
}
