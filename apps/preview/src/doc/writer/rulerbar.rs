//! The horizontal ruler above the pages: the page's margins, centimetre ticks, the caret
//! paragraph's indent markers and tab stops, a tab selector in the corner, and dragging of
//! markers and tab stops. Geometry, hit testing and drag arithmetic are `ruler.rs` (shared with
//! the egui Writer, measured from Word); this file converts its units (`u`: one physical pixel
//! at 125 % scale and 100 % zoom) to device pixels, paints and sends edits as commands.

use open365_commands::Command;
use open365_docx::TabAlign;
use open365_gfx::{Cap, Color, DrawList, Join, Path, Rect as GRect, TextLayoutOptions};
use open365_ui_core::Ui;

use super::WriterPane;
use crate::doc::ruler::{
    self, BAND_U, DIGIT_Y, RulerEdit, RulerHit, RulerInfo, RulerTab, RulerTabStop, TICK_SMALL_Y,
    TICK_TALL_Y, TickKind, U_PER_PT, URect,
};
use crate::palette::Palette;

/// Ruler state of a pane.
#[derive(Debug, Default)]
pub struct RulerState {
    pub on: bool,
    /// The kind a click on the track adds (the corner button cycles it).
    pub corner: RulerTab,
    drag: Option<RulerHit>,
    /// The edit a drag in progress would make, shown live.
    preview: Option<RulerEdit>,
}

fn command_of(edit: &RulerEdit) -> Command {
    match *edit {
        RulerEdit::SetIndents {
            para,
            left_pt,
            first_line_pt,
            right_pt,
        } => Command::DocxSetIndents {
            para,
            left_pt,
            first_line_pt,
            right_pt,
        },
        RulerEdit::AddTab { para, pos_pt, kind } => Command::DocxAddTab {
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
        RulerEdit::MoveTab {
            para,
            from_pt,
            to_pt,
        } => Command::DocxMoveTab {
            para,
            from_pt,
            to_pt,
        },
        RulerEdit::ClearTab { para, pos_pt } => Command::DocxClearTab { para, pos_pt },
    }
}

fn retarget(edit: &RulerEdit, p: usize) -> RulerEdit {
    let mut e = edit.clone();
    match &mut e {
        RulerEdit::SetIndents { para, .. }
        | RulerEdit::AddTab { para, .. }
        | RulerEdit::MoveTab { para, .. }
        | RulerEdit::ClearTab { para, .. } => *para = p,
    }
    e
}

fn edit_para(edit: &RulerEdit) -> usize {
    match edit {
        RulerEdit::SetIndents { para, .. }
        | RulerEdit::AddTab { para, .. }
        | RulerEdit::MoveTab { para, .. }
        | RulerEdit::ClearTab { para, .. } => *para,
    }
}

impl WriterPane {
    /// Shows or hides the ruler (View > Ruler). It takes a band off the top of the pane.
    pub fn set_ruler(&mut self, on: bool) {
        self.ruler.on = on;
        self.clamp_scroll();
    }

    pub fn ruler(&self) -> bool {
        self.ruler.on
    }

    /// The tab kind the next click on the ruler adds.
    pub fn ruler_tab_kind(&self) -> RulerTab {
        self.ruler.corner
    }

    /// Device pixels per ruler unit.
    fn pxu(&self) -> f32 {
        self.scale / 1.25
    }

    /// Height of the ruler band in device pixels (0 when the ruler is off).
    pub(super) fn top_inset(&self) -> f32 {
        if self.ruler.on {
            (BAND_U as f32 * self.pxu()).round()
        } else {
            0.0
        }
    }

    /// Ruler units per document point at the current zoom.
    fn ruler_k(&self) -> f64 {
        U_PER_PT * f64::from(self.zoom)
    }

    /// What the ruler shows for the paragraph holding the caret: its page, indents and tabs.
    pub fn ruler_info(&self) -> Option<RulerInfo> {
        let para = self.caret.para;
        let model = self.models.get(para)?;
        let page = self
            .index
            .refs
            .get(self.index.para_first.get(para).copied()?)
            .map_or(0, |r| r.page as usize);
        let s = &self.engine.layout().pages.get(page)?.section;
        let mut info = RulerInfo {
            para,
            page,
            page_width_pt: s.page_width,
            page_height_pt: s.page_height,
            margin_left_pt: s.margin_left,
            margin_right_pt: s.margin_right,
            margin_top_pt: s.margin_top,
            margin_bottom_pt: s.margin_bottom,
            indent_left_pt: f64::from(model.para.indent_left_pt),
            first_line_pt: f64::from(model.para.first_line_pt),
            indent_right_pt: f64::from(model.para.indent_right_pt),
            tabs: Vec::new(),
        };
        if let Some(doc) = self.target.docx()
            && let (Some(p), Some(styles)) = (doc.paragraph(para), doc.styles())
        {
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

    /// Runs a ruler edit through the command layer: once for every paragraph of the selection
    /// when the edited paragraph is inside it (each its own undo step), else for that paragraph.
    /// False when no paragraph changed.
    pub fn apply_ruler_edit(&mut self, edit: &RulerEdit) -> bool {
        let para = edit_para(edit);
        let (a, b) = self.selection();
        let paras: Vec<usize> = if (a.para..=b.para).contains(&para) {
            (a.para..=b.para).collect()
        } else {
            vec![para]
        };
        let mut changed = 0;
        for p in paras {
            if self.run(command_of(&retarget(edit, p))).is_some() {
                changed += 1;
            }
        }
        if changed > 0 {
            self.status = None;
        }
        changed > 0
    }

    // ---- pointer -----------------------------------------------------------------------------

    /// Page left edge and band-relative position of a canvas-local device point, in ruler units.
    fn ruler_point(&self, info: &RulerInfo, x: f32, y: f32) -> (f64, f64) {
        let pxu = f64::from(self.pxu());
        (
            f64::from(x - self.page_x(info.page)) / pxu,
            f64::from(y) / pxu,
        )
    }

    /// A press at canvas-local device point (`x`, `y`) inside the band. Returns true when the
    /// ruler took it.
    pub(super) fn ruler_press(&mut self, x: f32, y: f32) -> bool {
        if !self.ruler.on || y >= self.top_inset() {
            return false;
        }
        let Some(info) = self.ruler_info() else {
            return true;
        };
        let k = self.ruler_k();
        let (ux, uy) = self.ruler_point(&info, x, y);
        // The corner selector sits at the left end of the band.
        if f64::from(x) / f64::from(self.pxu()) < ruler::CORNER_U {
            self.ruler.corner = self.ruler.corner.next();
            return true;
        }
        let h = ruler::horizontal(&info, k);
        match ruler::hit_test(&h, ux, uy) {
            Some(RulerHit::Track(tx)) => {
                if let Some(edit) = ruler::click_edit(&info, k, tx, self.ruler.corner) {
                    self.apply_ruler_edit(&edit);
                }
            }
            Some(hit @ (RulerHit::Marker(_) | RulerHit::Tab(_))) => self.ruler.drag = Some(hit),
            _ => {}
        }
        true
    }

    /// While a marker or tab stop is dragged: the live preview, and the edit on release.
    pub(super) fn ruler_drag(&mut self, pointer: Option<(f32, f32)>) -> bool {
        let Some(hit) = self.ruler.drag else {
            return false;
        };
        let Some(info) = self.ruler_info() else {
            self.ruler.drag = None;
            return true;
        };
        match pointer {
            Some((x, y)) => {
                let (ux, uy) = self.ruler_point(&info, x, y);
                self.ruler.preview = ruler::drag_edit(&info, self.ruler_k(), hit, ux, uy);
            }
            None => {
                self.ruler.drag = None;
                if let Some(edit) = self.ruler.preview.take() {
                    self.apply_ruler_edit(&edit);
                }
            }
        }
        true
    }

    // ---- painting ----------------------------------------------------------------------------

    pub(super) fn paint_ruler(&mut self, ui: &mut Ui, list: &mut DrawList, pal: &Palette) {
        if !self.ruler.on {
            return;
        }
        let Some(info) = self.ruler_info() else {
            return;
        };
        let shown = match &self.ruler.preview {
            Some(edit) if self.ruler.drag.is_some() => ruler::preview(&info, edit),
            _ => info.clone(),
        };
        let pxu = self.pxu();
        let (ox, oy) = (self.area.x as f32, self.area.y as f32);
        let band_h = self.top_inset();
        let band = GRect::from_edges(ox, oy, ox + self.area.w as f32, oy + band_h);
        list.push_clip(band);
        list.fill_rect(band, pal.canvas);
        let ink = pal.page_text;
        let origin = (ox + self.page_x(info.page), oy);
        let h = ruler::horizontal(&shown, self.ruler_k());
        let ur = |r: &URect| {
            GRect::from_edges(
                origin.0 + r.x0 as f32 * pxu,
                origin.1 + r.y0 as f32 * pxu,
                origin.0 + (r.x1 as f32 * pxu).max(r.x0 as f32 * pxu + 1.0),
                origin.1 + (r.y1 as f32 * pxu).max(r.y0 as f32 * pxu + 1.0),
            )
        };
        list.fill_rect(ur(&h.track), pal.page);
        let mut count = 0usize;
        for t in &h.ticks {
            let y = if t.kind == TickKind::Tall {
                TICK_TALL_Y
            } else {
                TICK_SMALL_Y
            };
            list.fill_rect(
                ur(&URect::new(t.at, t.at + 1.0, y.0, y.1)),
                ink.with_opacity(0.7),
            );
        }
        let px = 10.0 * pxu * 1.25;
        for d in &h.digits {
            let label = d.number.to_string();
            let w = ui
                .assets
                .layout_text(ui.font, &label, px, 0.0, 0.0, TextLayoutOptions::default())
                .width;
            let cx = origin.0 + (d.at as f32 + 0.5) * pxu;
            let base = origin.1 + ((DIGIT_Y.0 + DIGIT_Y.1) as f32 / 2.0 + 3.5) * pxu;
            crate::doc::text(
                ui,
                list,
                &label,
                (cx - w / 2.0).floor(),
                base.round(),
                px,
                ink,
            );
        }
        for g in &h.tabs {
            self.paint_tab_glyph(list, origin, pxu, g, ink);
            count += 1;
        }
        for m in [&h.right, &h.hanging, &h.first_line] {
            let mut path = Path::new();
            for (i, &(x, y)) in m.outline.iter().enumerate() {
                let (px, py) = (origin.0 + x as f32 * pxu, origin.1 + y as f32 * pxu);
                if i == 0 {
                    path.move_to(px, py);
                } else {
                    path.line_to(px, py);
                }
            }
            path.close();
            list.fill_path(&mut ui.assets, &path, pal.page);
            list.stroke_path(
                &mut ui.assets,
                &path,
                1.0,
                Join::Miter,
                Cap::Butt,
                ink.with_opacity(0.7),
            );
            count += 1;
        }
        if let Some(b) = &h.left_box.box_rect {
            let r = ur(b);
            list.fill_rect(r, pal.page);
            list.rect_outline(r, 1.0, ink.with_opacity(0.7));
            count += 1;
        }
        // The tab selector.
        let corner = GRect::from_edges(ox, oy, ox + ruler::CORNER_U as f32 * pxu, oy + band_h);
        list.fill_rect(corner, pal.canvas);
        let g = ruler::corner_glyph(self.ruler.corner);
        self.paint_tab_glyph(list, (ox, oy), pxu, &g, ink);
        list.pop_clip();
        self.stats.ruler_items += count;
    }

    fn paint_tab_glyph(
        &self,
        list: &mut DrawList,
        o: (f32, f32),
        pxu: f32,
        g: &ruler::TabGlyph,
        color: Color,
    ) {
        let r = |u: &URect| {
            GRect::from_edges(
                o.0 + u.x0 as f32 * pxu,
                o.1 + u.y0 as f32 * pxu,
                o.0 + (u.x1 as f32 * pxu).max(u.x0 as f32 * pxu + 1.0),
                o.1 + (u.y1 as f32 * pxu).max(u.y0 as f32 * pxu + 1.0),
            )
        };
        list.fill_rect(r(&g.stem), color);
        if g.kind != RulerTab::Bar {
            list.fill_rect(r(&g.foot), color);
        }
        if let Some(d) = &g.dot {
            list.fill_rect(r(d), color);
        }
    }
}
