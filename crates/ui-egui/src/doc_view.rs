//! Paged document view over a DOCX [`Target`].
//!
//! Layout is virtualised: only paragraphs on visible pages get a galley that is drawn, galleys
//! are cached per paragraph and dropped when the paragraph is edited, and paragraph heights
//! (which decide the page breaks) are measured in the background with a small time budget per
//! frame. Until a paragraph is measured its height is estimated from its length.
//!
//! All edits go through [`Target::apply`]; the view keeps only the caret, the selection and
//! the layout caches.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use egui::epaint::text::CharIndex;
use egui::output::IMEOutput;
use egui::text::{CCursor, LayoutJob, TextFormat};
use egui::{
    Align, Color32, Context, Event, EventFilter, FontId, Galley, IMEPurpose, Id, Key, Modifiers,
    Pos2, Rect, Sense, Stroke, Ui, UiBuilder, Vec2, accesskit, vec2,
};
use open365_commands::{Command, Outcome, Target};
use open365_docx::{Alignment, LineSpacing, RunPropsPatch};

use crate::a11y::{self, RunEntry};
use crate::docmodel::{PageSetup, ParaModel, SpanStyle, build_models};
use crate::fonts::FontBook;

pub use open365_docx::DocPos;

const PAGE_GAP: f32 = 16.0;
const SIDE_PAD: f32 = 16.0;
/// Screen points per document point at 100% zoom (96 dpi).
const POINT_SCALE: f32 = 96.0 / 72.0;
const MEASURE_BUDGET_MS: f64 = 4.0;

/// Which character toggle a button or shortcut flips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    Bold,
    Italic,
    Underline,
}

impl Toggle {
    fn test(self, style: &SpanStyle) -> bool {
        match self {
            Self::Bold => style.bold,
            Self::Italic => style.italic,
            Self::Underline => style.underline,
        }
    }

    fn patch(self, on: bool) -> RunPropsPatch {
        match self {
            Self::Bold => RunPropsPatch::bold(on),
            Self::Italic => RunPropsPatch {
                italic: Some(on),
                ..RunPropsPatch::default()
            },
            Self::Underline => RunPropsPatch {
                underline: Some(on),
                ..RunPropsPatch::default()
            },
        }
    }
}

/// Where a paragraph's text sits, in screen pixels relative to the text area.
#[derive(Debug, Clone, Copy)]
struct ParaGeom {
    /// Left edge of the wrapping area.
    x0: f32,
    /// Width of the wrapping area.
    wrap: f32,
    /// Extra space before the first line (first-line indent).
    leading: f32,
}

#[derive(Debug, Clone, Copy)]
struct PageSlice {
    first: usize,
    end: usize,
}

struct Cached {
    galley: Arc<Galley>,
    /// Same layout with only the faked-bold spans visible, drawn offset to fake a bold weight
    /// for fonts that have no bold face.
    bold_overlay: Option<Arc<Galley>>,
    preedit: Option<String>,
    last_used: u64,
}

impl std::fmt::Debug for Cached {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cached")
            .field("rows", &self.galley.rows.len())
            .finish()
    }
}

/// State of the document view.
#[derive(Debug)]
pub struct DocView {
    models: Vec<ParaModel>,
    page: PageSetup,
    model_revision: Option<u64>,
    heights: Vec<f32>,
    measured: Vec<bool>,
    measure_cursor: usize,
    layout_key: (u32, u32),
    pages: Vec<PageSlice>,
    para_y: Vec<f32>,
    paginate_dirty: bool,
    cache: HashMap<usize, Cached>,
    caret: DocPos,
    anchor: DocPos,
    preedit: Option<String>,
    goal_x: Option<f32>,
    /// Paragraphs drawn last frame.
    placed: Vec<usize>,
    runs: Vec<RunEntry>,
    scale: f32,
    scroll_to_caret: bool,
    frame: u64,
    dragging: bool,
    want_focus: bool,
    doc_id: Id,
    fonts: Rc<RefCell<FontBook>>,
    fonts_gen: u64,
    /// Character formatting to apply to the next typed text (a toggle with no selection).
    pending: RunPropsPatch,
    /// Message for the status bar, taken by the application each frame.
    pub status: Option<String>,
    /// Milliseconds spent laying out and painting the last frame.
    pub last_frame_ms: f32,
}

impl Default for DocView {
    fn default() -> Self {
        Self::new()
    }
}

impl DocView {
    pub fn new() -> Self {
        Self {
            models: Vec::new(),
            page: PageSetup::LETTER,
            model_revision: None,
            heights: Vec::new(),
            measured: Vec::new(),
            measure_cursor: 0,
            layout_key: (0, 0),
            pages: Vec::new(),
            para_y: Vec::new(),
            paginate_dirty: true,
            cache: HashMap::new(),
            caret: DocPos::default(),
            anchor: DocPos::default(),
            preedit: None,
            goal_x: None,
            placed: Vec::new(),
            runs: Vec::new(),
            scale: POINT_SCALE,
            scroll_to_caret: false,
            frame: 0,
            dragging: false,
            want_focus: true,
            doc_id: Id::new("open365-doc-view"),
            fonts: Rc::new(RefCell::new(FontBook::new())),
            fonts_gen: 0,
            pending: RunPropsPatch::default(),
            status: None,
            last_frame_ms: 0.0,
        }
    }

    /// Gives the document keyboard focus on the next frame (after a toolbar action).
    pub fn focus(&mut self) {
        self.want_focus = true;
    }

    pub fn id(&self) -> Id {
        self.doc_id
    }

    pub fn caret(&self) -> DocPos {
        self.caret
    }

    /// The selection as ordered (start, end) positions.
    pub fn selection(&self) -> (DocPos, DocPos) {
        if self.anchor <= self.caret {
            (self.anchor, self.caret)
        } else {
            (self.caret, self.anchor)
        }
    }

    pub fn has_selection(&self) -> bool {
        self.anchor != self.caret
    }

    pub fn paragraph_count(&self) -> usize {
        self.models.len()
    }

    pub fn page_count(&self) -> usize {
        self.pages.len().max(1)
    }

    pub fn page_of(&self, para: usize) -> usize {
        self.pages
            .iter()
            .position(|p| p.first <= para && para < p.end)
            .unwrap_or(0)
    }

    /// Places the caret (clamped) and clears the selection.
    pub fn set_caret(&mut self, pos: DocPos) {
        let pos = self.clamp(pos);
        self.caret = pos;
        self.anchor = pos;
        self.goal_x = None;
        self.scroll_to_caret = true;
        self.pending = RunPropsPatch::default();
    }

    /// Selects the half-open range between two positions.
    pub fn select(&mut self, anchor: DocPos, focus: DocPos) {
        self.anchor = self.clamp(anchor);
        self.caret = self.clamp(focus);
        self.goal_x = None;
        self.scroll_to_caret = true;
        self.pending = RunPropsPatch::default();
    }

    fn clamp(&self, pos: DocPos) -> DocPos {
        if self.models.is_empty() {
            return DocPos::default();
        }
        let para = pos.para.min(self.models.len() - 1);
        DocPos {
            para,
            offset: pos.offset.min(self.models[para].char_len),
        }
    }

    /// Throws away everything derived from the document (after open, undo or redo).
    pub fn reset(&mut self) {
        self.model_revision = None;
        self.cache.clear();
    }

    /// A view that shares an application-wide font book.
    pub(crate) fn with_fonts(fonts: Rc<RefCell<FontBook>>) -> Self {
        Self {
            fonts,
            ..Self::new()
        }
    }

    /// Fonts the document uses that are not installed, with the bundled font drawn instead.
    pub fn font_substitutions(&self) -> Vec<(String, String)> {
        self.fonts.borrow().substitutions()
    }

    fn rebuild_all(&mut self, target: &Target) {
        let Some(doc) = target.docx() else {
            self.models.clear();
            return;
        };
        self.models = build_models(doc);
        self.page = PageSetup::from_document(doc);
        self.register_fonts(0..self.models.len());
        self.cache.clear();
        self.heights = (0..self.models.len()).map(|i| self.estimate(i)).collect();
        self.measured = vec![false; self.models.len()];
        self.measure_cursor = 0;
        self.paginate_dirty = true;
        self.caret = self.clamp(self.caret);
        self.anchor = self.clamp(self.anchor);
        self.model_revision = Some(target.revision());
    }

    /// Asks the font book for every face the paragraphs use, so they are registered together.
    fn register_fonts(&self, range: std::ops::Range<usize>) {
        let mut fonts = self.fonts.borrow_mut();
        for model in self.models.get(range).unwrap_or_default() {
            for style in model
                .spans
                .iter()
                .map(|s| &s.style)
                .chain(std::iter::once(&model.mark))
            {
                fonts.resolve(&style.font, style.bold, style.italic);
            }
        }
    }

    fn refresh_para(&mut self, target: &Target, para: usize) {
        let Some(doc) = target.docx() else { return };
        let Some(p) = doc.paragraph(para) else { return };
        if let Some(slot) = self.models.get_mut(para) {
            *slot = ParaModel::from_paragraph(p, doc.styles());
        }
        self.register_fonts(para..para + 1);
        self.cache.remove(&para);
        if let Some(m) = self.measured.get_mut(para) {
            *m = false;
        }
        let estimate = self.estimate(para);
        if let Some(h) = self.heights.get_mut(para) {
            *h = estimate;
        }
        self.measure_cursor = self.measure_cursor.min(para);
        self.paginate_dirty = true;
        self.model_revision = Some(target.revision());
    }

    /// After an edit replaced `old` paragraphs starting at `first` by `new` ones, rebuilds just
    /// those models. Falls back to a full rebuild when the counts do not add up.
    fn resync(&mut self, target: &Target, first: usize, old: usize, new: usize) {
        let Some(doc) = target.docx() else { return };
        let expected = (self.models.len() + new).checked_sub(old);
        if expected != Some(doc.paragraph_count()) || first + old > self.models.len() {
            self.rebuild_all(target);
            return;
        }
        let styles = doc.styles();
        let fresh: Vec<ParaModel> = (first..first + new)
            .filter_map(|i| doc.paragraph(i))
            .map(|p| ParaModel::from_paragraph(p, styles))
            .collect();
        if fresh.len() != new {
            self.rebuild_all(target);
            return;
        }
        self.models.splice(first..first + old, fresh);
        self.register_fonts(first..first + new);
        let estimates: Vec<f32> = (first..first + new).map(|i| self.estimate(i)).collect();
        self.heights.splice(first..first + old, estimates);
        self.measured
            .splice(first..first + old, std::iter::repeat_n(false, new));
        self.cache.clear();
        self.measure_cursor = self.measure_cursor.min(first);
        self.paginate_dirty = true;
        self.model_revision = Some(target.revision());
    }

    /// The largest font size in a paragraph, in points.
    fn max_size_pt(model: &ParaModel) -> f32 {
        model
            .spans
            .iter()
            .map(|s| s.style.size_pt())
            .fold(model.mark.size_pt(), f32::max)
            .max(1.0)
    }

    fn before_px(&self, model: &ParaModel) -> f32 {
        model.para.space_before_pt * self.scale
    }

    fn after_px(&self, model: &ParaModel) -> f32 {
        model.para.space_after_pt * self.scale
    }

    fn geom(&self, model: &ParaModel) -> ParaGeom {
        let body = self.page.body_width() * self.scale;
        let left = model.para.indent_left_pt * self.scale;
        let right = model.para.indent_right_pt * self.scale;
        let first = model.para.first_line_pt * self.scale;
        // A hanging indent starts the whole paragraph at the hanging position: egui cannot
        // indent only the continuation lines.
        let (x0, leading) = if first >= 0.0 {
            (left.max(0.0), first)
        } else {
            ((left + first).max(0.0), 0.0)
        };
        ParaGeom {
            x0,
            wrap: (body - x0 - right).max(50.0),
            leading,
        }
    }

    /// X of the galley anchor relative to the text area: the left edge, centre or right edge
    /// of the wrapping area depending on alignment.
    fn anchor_x(&self, model: &ParaModel) -> f32 {
        let g = self.geom(model);
        match model.para.alignment {
            Alignment::Center => g.x0 + g.wrap / 2.0,
            Alignment::Right => g.x0 + g.wrap,
            Alignment::Left | Alignment::Justify => g.x0,
        }
    }

    fn estimate(&self, idx: usize) -> f32 {
        let Some(model) = self.models.get(idx) else {
            return 0.0;
        };
        let size_pt = Self::max_size_pt(model);
        let size = size_pt * self.scale;
        let wrap = self.geom(model).wrap;
        let chars_per_line = (wrap / (size * 0.5)).max(1.0);
        let lines = ((model.char_len as f32 / chars_per_line).ceil() as usize).max(1)
            + model.text.matches('\n').count();
        let line = match model.para.line {
            LineSpacing::Multiple(m) => size * 1.22 * m,
            LineSpacing::AtLeast(pt) => (size * 1.22).max(pt * self.scale),
            LineSpacing::Exactly(pt) => pt * self.scale,
        };
        self.before_px(model) + lines as f32 * line + self.after_px(model)
    }

    /// Layout job of a paragraph (and, when some text needs faked bold, a second job with only
    /// those spans visible).
    fn build_jobs(
        &self,
        ctx: &Context,
        idx: usize,
        preedit: Option<&str>,
    ) -> (LayoutJob, Option<LayoutJob>) {
        let model = &self.models[idx];
        let geom = self.geom(model);
        let mut job = LayoutJob::default();
        job.wrap.max_width = geom.wrap;
        job.halign = match model.para.alignment {
            Alignment::Center => Align::Center,
            Alignment::Right => Align::RIGHT,
            Alignment::Left | Alignment::Justify => Align::LEFT,
        };
        job.justify = model.para.alignment == Alignment::Justify;
        let mut overlay = LayoutJob::default();
        overlay.wrap.max_width = geom.wrap;
        overlay.halign = job.halign;
        overlay.justify = job.justify;
        let mut any_fake_bold = false;
        let mut first_section = true;

        // Walk the text in runs of equal style, inserting the IME preedit at the caret.
        let preedit_at = preedit.map(|_| self.caret.offset.min(model.char_len));
        let mut push = |text: &str, style: &SpanStyle, is_preedit: bool| {
            if text.is_empty() {
                return;
            }
            let face = self
                .fonts
                .borrow_mut()
                .resolve(&style.font, style.bold, style.italic);
            let fake_bold = style.bold && !face.real_bold;
            any_fake_bold |= fake_bold;
            let size = style.size_pt() * self.scale;
            let font_id = FontId::new(size, face.family);
            let color = style
                .color
                .map_or(Color32::BLACK, |[r, g, b]| Color32::from_rgb(r, g, b));
            let underline = if style.underline || is_preedit {
                Stroke::new(1.0, color)
            } else {
                Stroke::NONE
            };
            let strike = if style.strike {
                Stroke::new(1.0, color)
            } else {
                Stroke::NONE
            };
            let natural = ctx.fonts_mut(|f| f.row_height(&font_id));
            let line_height = match model.para.line {
                LineSpacing::Multiple(m) if (m - 1.0).abs() < 0.001 => None,
                LineSpacing::Multiple(m) => Some(natural * m),
                LineSpacing::AtLeast(pt) => Some(natural.max(pt * self.scale)),
                LineSpacing::Exactly(pt) => Some(pt * self.scale),
            };
            let format = |color: Color32| TextFormat {
                font_id: font_id.clone(),
                color,
                italics: style.italic && !face.real_italic,
                underline,
                strikethrough: strike,
                line_height,
                ..TextFormat::default()
            };
            let leading = if std::mem::take(&mut first_section) {
                geom.leading
            } else {
                0.0
            };
            job.append(text, leading, format(color));
            overlay.append(
                text,
                leading,
                format(if fake_bold {
                    color
                } else {
                    Color32::TRANSPARENT
                }),
            );
        };

        let mut buf = String::new();
        let mut buf_style = model.mark.clone();
        let mut span_i = 0usize;
        for (i, ch) in model.text.chars().enumerate() {
            if preedit_at == Some(i) {
                push(&buf, &buf_style, false);
                buf.clear();
                push(preedit.unwrap_or_default(), &buf_style, true);
            }
            while span_i < model.spans.len() && model.spans[span_i].end <= i {
                span_i += 1;
            }
            let style = match model.spans.get(span_i) {
                Some(s) if s.start <= i => s.style.clone(),
                _ => model.mark.clone(),
            };
            if style != buf_style {
                push(&buf, &buf_style, false);
                buf.clear();
                buf_style = style;
            }
            buf.push(ch);
        }
        push(&buf, &buf_style, false);
        if preedit_at == Some(model.char_len) {
            let style = model.spans.last().map_or(&model.mark, |s| &s.style);
            push(preedit.unwrap_or_default(), style, true);
        }
        if job.is_empty() {
            // An empty paragraph still needs a row of the right height.
            let style = &model.mark;
            let face = self
                .fonts
                .borrow_mut()
                .resolve(&style.font, style.bold, style.italic);
            let font_id = FontId::new(style.size_pt() * self.scale, face.family);
            job.append("", 0.0, TextFormat::simple(font_id, Color32::BLACK));
        }
        (job, any_fake_bold.then_some(overlay))
    }

    /// The cached galley of a paragraph, laying it out when needed.
    fn galley(&mut self, ctx: &Context, idx: usize) -> Arc<Galley> {
        let preedit = (self.caret.para == idx)
            .then(|| self.preedit.clone())
            .flatten();
        if let Some(c) = self.cache.get_mut(&idx)
            && c.preedit == preedit
        {
            c.last_used = self.frame;
            return Arc::clone(&c.galley);
        }
        let (job, overlay) = self.build_jobs(ctx, idx, preedit.as_deref());
        let galley = ctx.fonts_mut(|f| f.layout_job(job));
        let bold_overlay = overlay.map(|j| ctx.fonts_mut(|f| f.layout_job(j)));
        let model = &self.models[idx];
        let height = self.before_px(model)
            + galley.size().y.max(self.min_row_height(idx))
            + self.after_px(model);
        if let (Some(h), Some(m)) = (self.heights.get_mut(idx), self.measured.get_mut(idx)) {
            if (*h - height).abs() > 0.01 {
                self.paginate_dirty = true;
            }
            *h = height;
            *m = true;
        }
        self.cache.insert(
            idx,
            Cached {
                galley: Arc::clone(&galley),
                bold_overlay,
                preedit,
                last_used: self.frame,
            },
        );
        galley
    }

    fn min_row_height(&self, idx: usize) -> f32 {
        self.models
            .get(idx)
            .map_or(0.0, |m| Self::max_size_pt(m) * self.scale * 1.15)
    }

    /// Measures a few unmeasured paragraphs per frame so page breaks settle without a stall.
    fn measure_some(&mut self, ctx: &Context) -> bool {
        let start = Instant::now();
        while self.measure_cursor < self.models.len() {
            let i = self.measure_cursor;
            if !self.measured[i] {
                let (job, _) = self.build_jobs(ctx, i, None);
                let galley = ctx.fonts_mut(|f| f.layout_job(job));
                let model = &self.models[i];
                let height = self.before_px(model)
                    + galley.size().y.max(self.min_row_height(i))
                    + self.after_px(model);
                if (self.heights[i] - height).abs() > 0.01 {
                    self.paginate_dirty = true;
                }
                self.heights[i] = height;
                self.measured[i] = true;
            }
            self.measure_cursor += 1;
            if start.elapsed().as_secs_f64() * 1000.0 > MEASURE_BUDGET_MS {
                break;
            }
        }
        self.measure_cursor < self.models.len()
    }

    fn paginate(&mut self) {
        let body = self.page.body_height() * self.scale;
        self.pages.clear();
        self.para_y.clear();
        self.para_y.resize(self.models.len(), 0.0);
        let (mut start, mut y) = (0usize, 0.0f32);
        for i in 0..self.models.len() {
            let h = self.heights[i];
            if y > 0.0 && y + h > body {
                self.pages.push(PageSlice {
                    first: start,
                    end: i,
                });
                start = i;
                y = 0.0;
            }
            self.para_y[i] = y;
            y += h;
            if self.models[i].page_break {
                self.pages.push(PageSlice {
                    first: start,
                    end: i + 1,
                });
                start = i + 1;
                y = 0.0;
            }
        }
        if start < self.models.len() || self.pages.is_empty() {
            self.pages.push(PageSlice {
                first: start,
                end: self.models.len(),
            });
        }
        self.paginate_dirty = false;
    }

    // ----- geometry -----

    fn page_size_px(&self) -> Vec2 {
        vec2(self.page.width, self.page.height) * self.scale
    }

    fn page_top(&self, canvas_top: f32, page: usize) -> f32 {
        let size = self.page_size_px();
        canvas_top + PAGE_GAP + page as f32 * (size.y + PAGE_GAP)
    }

    fn para_origin(&self, canvas: Rect, para: usize) -> Pos2 {
        let page = self.page_of(para);
        let size = self.page_size_px();
        let x0 = canvas.min.x + ((canvas.width() - size.x) / 2.0).max(SIDE_PAD);
        let model = self.models.get(para);
        let anchor = model.map_or(0.0, |m| self.anchor_x(m));
        let before = model.map_or(0.0, |m| self.before_px(m));
        Pos2::new(
            x0 + self.page.margin_left * self.scale + anchor,
            self.page_top(canvas.min.y, page)
                + self.page.margin_top * self.scale
                + self.para_y.get(para).copied().unwrap_or(0.0)
                + before,
        )
    }

    fn hit_test(&mut self, ctx: &Context, canvas: Rect, pos: Pos2) -> DocPos {
        if self.models.is_empty() {
            return DocPos::default();
        }
        let size = self.page_size_px();
        let rel = pos.y - canvas.min.y - PAGE_GAP;
        let page = (rel / (size.y + PAGE_GAP)).floor().max(0.0) as usize;
        let page = page.min(self.pages.len().saturating_sub(1));
        let slice = self.pages[page];
        if slice.first >= slice.end {
            return DocPos::default();
        }
        let top = self.page_top(canvas.min.y, page) + self.page.margin_top * self.scale;
        let local_y = pos.y - top;
        let in_page = self.para_y[slice.first..slice.end].partition_point(|y| *y <= local_y);
        let para = (slice.first + in_page.saturating_sub(1)).min(slice.end - 1);
        let origin = self.para_origin(canvas, para);
        let galley = self.galley(ctx, para);
        let cursor = galley.cursor_from_pos(pos - origin);
        DocPos {
            para,
            offset: cursor.index.0.min(self.models[para].char_len),
        }
    }

    fn caret_rect(&mut self, ctx: &Context, canvas: Rect, pos: DocPos) -> Rect {
        let origin = self.para_origin(canvas, pos.para);
        let galley = self.galley(ctx, pos.para);
        let offset = pos.offset + self.preedit_len_before_caret(pos);
        let mut c = CCursor::new(CharIndex(offset));
        c.prefer_next_row = true;
        let r = galley.pos_from_cursor(c);
        Rect::from_min_max(
            Pos2::new(origin.x + r.min.x, origin.y + r.min.y),
            Pos2::new(origin.x + r.min.x + 1.5, origin.y + r.max.y),
        )
    }

    /// While composing, the preedit text sits at the caret and the caret draws after it.
    fn preedit_len_before_caret(&self, pos: DocPos) -> usize {
        if pos == self.caret {
            self.preedit.as_ref().map_or(0, |p| p.chars().count())
        } else {
            0
        }
    }

    // ----- edits (all through the command layer) -----

    /// Applies a command; on failure shows the reason and returns `None`.
    fn run(&mut self, target: &mut Target, command: Command) -> Option<Outcome> {
        match target.apply(command) {
            Ok(outcome) => {
                if !outcome.notes.is_empty() {
                    self.status = Some(format!("Removed {}", outcome.notes.join("; ")));
                }
                Some(outcome)
            }
            Err(e) => {
                self.status = Some(e.to_string());
                None
            }
        }
    }

    /// The selection clamped to the document, as (start, end).
    fn range(&self) -> (DocPos, DocPos) {
        let (a, b) = self.selection();
        (self.clamp(a), self.clamp(b))
    }

    /// Replaces the selection (or inserts at the caret) with `text`. `\n` in `text` starts a
    /// new paragraph; the caller has removed other control characters.
    fn replace_selection(&mut self, target: &mut Target, text: &str) {
        if self.models.is_empty() {
            return;
        }
        let (s, e) = self.range();
        let lines = text.matches('\n').count();
        let (end_para, end_offset) = match text.rsplit_once('\n') {
            Some((_, last)) => (s.para + lines, last.chars().count()),
            None => (s.para, s.offset + text.chars().count()),
        };
        let after = DocPos::new(end_para, end_offset);
        let ok = if s == e && lines == 0 {
            self.run(
                target,
                Command::DocxInsertText {
                    para: s.para,
                    offset: s.offset,
                    text: text.to_owned(),
                },
            )
            .is_some()
        } else {
            self.run(
                target,
                Command::DocxReplaceRange {
                    start: s,
                    end: e,
                    text: text.to_owned(),
                },
            )
            .is_some()
        };
        if !ok {
            return;
        }
        if s.para == e.para && lines == 0 {
            self.refresh_para(target, s.para);
        } else {
            self.resync(target, s.para, e.para - s.para + 1, 1 + lines);
        }
        self.apply_pending(target, s, after);
        self.set_caret(after);
        self.preedit = None;
    }

    /// Gives text just typed the formatting toggled with no selection.
    fn apply_pending(&mut self, target: &mut Target, start: DocPos, end: DocPos) {
        let patch = std::mem::take(&mut self.pending);
        if patch.is_empty() || start == end {
            return;
        }
        if self
            .run(target, Command::DocxSetRunProps { start, end, patch })
            .is_some()
        {
            for p in start.para..=end.para {
                self.refresh_para(target, p);
            }
        }
    }

    /// Inserts typed or committed text at the caret (or over the selection). Line breaks
    /// become spaces; use [`DocView::paste_text`] for multi-line text.
    pub fn insert_text(&mut self, target: &mut Target, text: &str) {
        let clean: String = text
            .chars()
            .map(|c| if matches!(c, '\n' | '\r') { ' ' } else { c })
            .filter(|c| !c.is_control() || *c == '\t')
            .collect();
        if !clean.is_empty() {
            self.replace_selection(target, &clean);
        }
    }

    /// Pastes plain text: line breaks start new paragraphs.
    pub fn paste_text(&mut self, target: &mut Target, text: &str) {
        let normalised = text.replace("\r\n", "\n").replace('\r', "\n");
        let clean: String = normalised
            .chars()
            .filter(|c| !c.is_control() || matches!(c, '\t' | '\n'))
            .collect();
        if !clean.is_empty() {
            self.replace_selection(target, &clean);
        }
    }

    /// Deletes the selection; true when something was deleted.
    pub fn delete_selection(&mut self, target: &mut Target) -> bool {
        if !self.has_selection() {
            return false;
        }
        let (s, e) = self.range();
        if self
            .run(target, Command::DocxDeleteRange { start: s, end: e })
            .is_none()
        {
            return false;
        }
        if s.para == e.para {
            self.refresh_para(target, s.para);
        } else {
            self.resync(target, s.para, e.para - s.para + 1, 1);
        }
        self.set_caret(s);
        true
    }

    /// Backspace (`word`: Ctrl+Backspace). Joins with the previous paragraph at its start.
    pub fn backspace(&mut self, target: &mut Target, word: bool, ctx: &Context) {
        let _ = ctx;
        if self.delete_selection(target) || self.models.is_empty() {
            return;
        }
        let caret = self.caret;
        let start = if caret.offset > 0 {
            DocPos::new(
                caret.para,
                if word {
                    self.word_left(caret).offset
                } else {
                    caret.offset - 1
                },
            )
        } else if caret.para > 0 {
            DocPos::new(caret.para - 1, self.models[caret.para - 1].char_len)
        } else {
            return;
        };
        self.delete_between(target, start, caret);
    }

    /// Delete key (`word`: Ctrl+Delete). Joins with the next paragraph at the end.
    pub fn delete_forward(&mut self, target: &mut Target, word: bool) {
        if self.delete_selection(target) || self.models.is_empty() {
            return;
        }
        let caret = self.caret;
        let len = self.models[caret.para].char_len;
        let end = if caret.offset < len {
            DocPos::new(
                caret.para,
                if word {
                    self.word_right(caret).offset.max(caret.offset + 1)
                } else {
                    caret.offset + 1
                },
            )
        } else if caret.para + 1 < self.models.len() {
            DocPos::new(caret.para + 1, 0)
        } else {
            return;
        };
        self.delete_between(target, caret, end);
    }

    fn delete_between(&mut self, target: &mut Target, start: DocPos, end: DocPos) {
        if self
            .run(target, Command::DocxDeleteRange { start, end })
            .is_none()
        {
            return;
        }
        if start.para == end.para {
            self.refresh_para(target, start.para);
        } else {
            self.resync(target, start.para, end.para - start.para + 1, 1);
        }
        self.set_caret(start);
    }

    /// Enter: splits the paragraph at the caret (replacing the selection). With `soft`
    /// (Shift+Enter) inserts a line break instead.
    pub fn enter(&mut self, target: &mut Target, soft: bool) {
        if self.models.is_empty() {
            return;
        }
        if soft {
            self.delete_selection(target);
            let at = self.caret;
            if self
                .run(
                    target,
                    Command::DocxInsertText {
                        para: at.para,
                        offset: at.offset,
                        text: "\n".to_owned(),
                    },
                )
                .is_some()
            {
                self.refresh_para(target, at.para);
                self.set_caret(DocPos::new(at.para, at.offset + 1));
            }
            return;
        }
        if self.has_selection() {
            self.replace_selection(target, "\n");
            return;
        }
        let at = self.clamp(self.caret);
        if self
            .run(target, Command::DocxSplitParagraph { pos: at })
            .is_some()
        {
            self.resync(target, at.para, 1, 2);
            self.set_caret(DocPos::new(at.para + 1, 0));
            self.pending = RunPropsPatch::default();
        }
    }

    /// Cuts the selection to the clipboard.
    pub fn cut(&mut self, target: &mut Target, ctx: &Context) {
        if self.has_selection() {
            ctx.copy_text(self.selected_text());
            self.delete_selection(target);
        }
    }

    /// Flips bold, italic or underline: over the selection when there is one (on unless the
    /// whole selection already has it), otherwise for the text typed next.
    pub fn toggle(&mut self, target: &mut Target, which: Toggle) {
        if self.models.is_empty() {
            return;
        }
        if !self.has_selection() {
            let current = self.models[self.caret.para].style_at(self.caret.offset);
            let on = match which {
                Toggle::Bold => self.pending.bold.unwrap_or(current.bold),
                Toggle::Italic => self.pending.italic.unwrap_or(current.italic),
                Toggle::Underline => self.pending.underline.unwrap_or(current.underline),
            };
            let flipped = which.patch(!on);
            match which {
                Toggle::Bold => self.pending.bold = flipped.bold,
                Toggle::Italic => self.pending.italic = flipped.italic,
                Toggle::Underline => self.pending.underline = flipped.underline,
            }
            return;
        }
        let (a, b) = self.range();
        let all = (a.para..=b.para).all(|p| {
            let len = self.models[p].char_len;
            let start = if p == a.para { a.offset } else { 0 };
            let end = if p == b.para { b.offset } else { len };
            start >= end || self.models[p].all(start, end, |s| which.test(s))
        });
        let command = Command::DocxSetRunProps {
            start: a,
            end: b,
            patch: which.patch(!all),
        };
        if self.run(target, command).is_some() {
            for p in a.para..=b.para {
                self.refresh_para(target, p);
            }
        }
    }

    pub fn toggle_bold(&mut self, target: &mut Target) {
        self.toggle(target, Toggle::Bold);
    }

    /// Style id of the paragraph holding the caret (`None` is the default style).
    pub fn caret_style_id(&self) -> Option<&str> {
        self.models
            .get(self.caret.para)
            .and_then(|m| m.style_id.as_deref())
    }

    /// Alignment of the paragraph holding the caret.
    pub fn caret_alignment(&self) -> Alignment {
        self.models
            .get(self.caret.para)
            .map_or(Alignment::Left, |m| m.para.alignment)
    }

    /// Character formatting at the caret, including a toggle waiting to be typed.
    pub fn caret_format(&self) -> SpanStyle {
        let mut style = self
            .models
            .get(self.caret.para)
            .map_or_else(SpanStyle::default, |m| {
                m.style_at(self.caret.offset).clone()
            });
        if let Some(v) = self.pending.bold {
            style.bold = v;
        }
        if let Some(v) = self.pending.italic {
            style.italic = v;
        }
        if let Some(v) = self.pending.underline {
            style.underline = v;
        }
        style
    }

    /// Paragraphs touched by the selection (or just the caret's).
    fn selected_paragraphs(&self) -> std::ops::RangeInclusive<usize> {
        let (a, b) = self.range();
        a.para..=b.para
    }

    /// Applies a paragraph style to the selected paragraphs.
    pub fn set_style(&mut self, target: &mut Target, style_id: &str) {
        for p in self.selected_paragraphs() {
            let command = Command::DocxSetParagraphStyle {
                para: p,
                style_id: style_id.to_owned(),
            };
            if self.run(target, command).is_none() {
                return;
            }
            self.refresh_para(target, p);
        }
    }

    /// Aligns the selected paragraphs.
    pub fn set_alignment(&mut self, target: &mut Target, alignment: Alignment) {
        for p in self.selected_paragraphs() {
            let command = Command::DocxSetAlignment { para: p, alignment };
            if self.run(target, command).is_none() {
                return;
            }
            self.refresh_para(target, p);
        }
    }

    pub fn selected_text(&self) -> String {
        let (a, b) = self.selection();
        let mut out = String::new();
        for p in a.para..=b.para.min(self.models.len().saturating_sub(1)) {
            let model = &self.models[p];
            let start = if p == a.para { a.offset } else { 0 };
            let end = if p == b.para {
                b.offset
            } else {
                model.char_len
            };
            out.extend(
                model
                    .text
                    .chars()
                    .skip(start)
                    .take(end.saturating_sub(start)),
            );
            if p != b.para {
                out.push('\n');
            }
        }
        out.replace('\u{2003}', "\t")
    }

    // ----- caret movement -----

    fn move_caret(&mut self, ctx: &Context, key: Key, mods: Modifiers) {
        if self.models.is_empty() {
            return;
        }
        let mut pos = self.caret;
        let mut keep_goal = false;
        match key {
            Key::ArrowLeft => {
                if !mods.shift && self.has_selection() {
                    pos = self.selection().0;
                } else if mods.command {
                    pos = self.word_left(pos);
                } else if pos.offset > 0 {
                    pos.offset -= 1;
                } else if pos.para > 0 {
                    pos.para -= 1;
                    pos.offset = self.models[pos.para].char_len;
                }
            }
            Key::ArrowRight => {
                if !mods.shift && self.has_selection() {
                    pos = self.selection().1;
                } else if mods.command {
                    pos = self.word_right(pos);
                } else if pos.offset < self.models[pos.para].char_len {
                    pos.offset += 1;
                } else if pos.para + 1 < self.models.len() {
                    pos.para += 1;
                    pos.offset = 0;
                }
            }
            Key::Home => {
                pos = if mods.command {
                    DocPos::default()
                } else {
                    self.row_edge(ctx, pos, false)
                };
            }
            Key::End => {
                pos = if mods.command {
                    let last = self.models.len() - 1;
                    DocPos {
                        para: last,
                        offset: self.models[last].char_len,
                    }
                } else {
                    self.row_edge(ctx, pos, true)
                };
            }
            Key::ArrowUp | Key::ArrowDown | Key::PageUp | Key::PageDown => {
                let steps = if matches!(key, Key::PageUp | Key::PageDown) {
                    24
                } else {
                    1
                };
                let down = matches!(key, Key::ArrowDown | Key::PageDown);
                for _ in 0..steps {
                    pos = self.vertical(ctx, pos, down);
                }
                keep_goal = true;
            }
            _ => return,
        }
        let goal = self.goal_x;
        self.caret = self.clamp(pos);
        if !mods.shift {
            self.anchor = self.caret;
        }
        self.goal_x = if keep_goal { goal } else { None };
        self.scroll_to_caret = true;
        self.pending = RunPropsPatch::default();
    }

    fn word_left(&self, mut pos: DocPos) -> DocPos {
        if pos.offset == 0 {
            if pos.para > 0 {
                pos.para -= 1;
                pos.offset = self.models[pos.para].char_len;
            }
            return pos;
        }
        let chars: Vec<char> = self.models[pos.para].text.chars().collect();
        let mut i = pos.offset;
        while i > 0 && !a11y::is_word_char(chars[i - 1]) {
            i -= 1;
        }
        while i > 0 && a11y::is_word_char(chars[i - 1]) {
            i -= 1;
        }
        pos.offset = i;
        pos
    }

    fn word_right(&self, mut pos: DocPos) -> DocPos {
        let len = self.models[pos.para].char_len;
        if pos.offset >= len {
            if pos.para + 1 < self.models.len() {
                pos.para += 1;
                pos.offset = 0;
            }
            return pos;
        }
        let chars: Vec<char> = self.models[pos.para].text.chars().collect();
        let mut i = pos.offset;
        while i < len && a11y::is_word_char(chars[i]) {
            i += 1;
        }
        while i < len && !a11y::is_word_char(chars[i]) {
            i += 1;
        }
        pos.offset = i;
        pos
    }

    fn row_edge(&mut self, ctx: &Context, pos: DocPos, end: bool) -> DocPos {
        let galley = self.galley(ctx, pos.para);
        let cursor = CCursor::new(CharIndex(pos.offset));
        let moved = if end {
            let mut c = galley.cursor_end_of_row(&cursor);
            c.prefer_next_row = false;
            c
        } else {
            galley.cursor_begin_of_row(&cursor)
        };
        DocPos {
            para: pos.para,
            offset: moved.index.0.min(self.models[pos.para].char_len),
        }
    }

    fn vertical(&mut self, ctx: &Context, pos: DocPos, down: bool) -> DocPos {
        let galley = self.galley(ctx, pos.para);
        let mut c = CCursor::new(CharIndex(pos.offset));
        c.prefer_next_row = true;
        let layout = galley.layout_from_cursor(c);
        let here = galley.pos_from_layout_cursor(&layout);
        let x = *self.goal_x.get_or_insert(here.min.x);
        let last_row = galley.rows.len().saturating_sub(1);
        if down && layout.row < last_row {
            let row = &galley.rows[layout.row + 1];
            let cursor = galley.cursor_from_pos(vec2(x, row.rect().center().y));
            return DocPos {
                para: pos.para,
                offset: cursor.index.0.min(self.models[pos.para].char_len),
            };
        }
        if !down && layout.row > 0 {
            let row = &galley.rows[layout.row - 1];
            let cursor = galley.cursor_from_pos(vec2(x, row.rect().center().y));
            return DocPos {
                para: pos.para,
                offset: cursor.index.0.min(self.models[pos.para].char_len),
            };
        }
        let next = if down {
            if pos.para + 1 >= self.models.len() {
                return DocPos {
                    para: pos.para,
                    offset: self.models[pos.para].char_len,
                };
            }
            pos.para + 1
        } else {
            if pos.para == 0 {
                return DocPos { para: 0, offset: 0 };
            }
            pos.para - 1
        };
        let g = self.galley(ctx, next);
        let y = if down {
            g.rows.first().map_or(0.0, |r| r.rect().center().y)
        } else {
            g.rows.last().map_or(0.0, |r| r.rect().center().y)
        };
        let cursor = g.cursor_from_pos(vec2(x, y));
        DocPos {
            para: next,
            offset: cursor.index.0.min(self.models[next].char_len),
        }
    }

    // ----- the frame -----

    /// Shows the view and handles its input. Returns true while background measurement still
    /// needs more frames.
    pub fn show(&mut self, ui: &mut Ui, target: &mut Target) -> bool {
        let started = Instant::now();
        self.frame += 1;
        let ctx = ui.ctx().clone();
        self.fonts.borrow_mut().sync(&ctx);
        if self.model_revision != Some(target.revision()) {
            self.rebuild_all(target);
        }
        let fonts_gen = self.fonts.borrow().generation();
        if fonts_gen != self.fonts_gen {
            // New families became usable: every layout made so far used stand-in fonts.
            self.fonts_gen = fonts_gen;
            self.cache.clear();
            self.heights = (0..self.models.len()).map(|i| self.estimate(i)).collect();
            self.measured = vec![false; self.models.len()];
            self.measure_cursor = 0;
            self.paginate_dirty = true;
        }
        let avail = ui.available_size_before_wrap();
        let fit = ((avail.x - 2.0 * SIDE_PAD) / self.page.width).clamp(0.3, POINT_SCALE);
        let scale = fit.min(POINT_SCALE);
        let key = (scale.to_bits(), self.page.body_width().to_bits());
        if key != self.layout_key || (self.scale - scale).abs() > f32::EPSILON {
            self.layout_key = key;
            self.scale = scale;
            self.cache.clear();
            self.heights = (0..self.models.len()).map(|i| self.estimate(i)).collect();
            self.measured = vec![false; self.models.len()];
            self.measure_cursor = 0;
            self.paginate_dirty = true;
        }
        let more = self.measure_some(&ctx);
        if self.paginate_dirty {
            self.paginate();
        }

        let page_px = self.page_size_px();
        let total_h = PAGE_GAP + self.pages.len() as f32 * (page_px.y + PAGE_GAP);
        let mut want_more = more || !self.fonts.borrow().is_ready();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_viewport(ui, |ui, viewport| {
                let width = ui.available_width().max(page_px.x + 2.0 * SIDE_PAD);
                let (canvas, _) = ui.allocate_exact_size(vec2(width, total_h), Sense::hover());
                let response = ui.interact(canvas, self.doc_id, Sense::click_and_drag());
                ui.memory_mut(|m| {
                    m.set_focus_lock_filter(
                        self.doc_id,
                        EventFilter {
                            tab: false,
                            horizontal_arrows: true,
                            vertical_arrows: true,
                            escape: false,
                        },
                    );
                });
                if std::mem::take(&mut self.want_focus) {
                    response.request_focus();
                }

                self.handle_pointer(&ctx, &response, canvas);
                let focused = response.has_focus() || ui.memory(|m| m.has_focus(self.doc_id));
                // egui may run a frame as several passes with the same input events, so
                // events are acted on in the first pass only.
                if ctx.current_pass_index() == 0 {
                    if focused {
                        self.handle_keys(&ctx, target);
                    }
                    self.handle_access_requests(&ctx, target);
                }
                // Edits above may have changed the paragraph list within this frame.
                if self.paginate_dirty {
                    self.paginate();
                }

                // Visible pages.
                let ph = page_px.y + PAGE_GAP;
                let first_page = (((viewport.min.y - PAGE_GAP) / ph).floor().max(0.0)) as usize;
                let last_page =
                    (((viewport.max.y) / ph).floor().max(0.0) as usize).min(self.pages.len() - 1);
                let clip = ui.clip_rect();
                let painter = ui.painter().clone();
                let sel = self.selection();
                let has_sel = self.has_selection();
                let x0 = canvas.min.x + ((canvas.width() - page_px.x) / 2.0).max(SIDE_PAD);
                self.placed.clear();
                let mut placed_new: Vec<usize> = Vec::new();

                for page in first_page..=last_page.max(first_page).min(self.pages.len() - 1) {
                    let top = self.page_top(canvas.min.y, page);
                    let page_rect = Rect::from_min_size(Pos2::new(x0, top), page_px);
                    if !page_rect.intersects(clip) {
                        continue;
                    }
                    painter.rect_filled(
                        page_rect.translate(vec2(2.0, 3.0)),
                        0.0,
                        Color32::from_black_alpha(40),
                    );
                    painter.rect_filled(page_rect, 0.0, Color32::WHITE);
                    let slice = self.pages[page];
                    for para in slice.first..slice.end {
                        let origin = self.para_origin(canvas, para);
                        let galley = self.galley(&ctx, para);
                        let height = galley.size().y;
                        if origin.y > clip.max.y || origin.y + height < clip.min.y {
                            continue;
                        }
                        if has_sel && para >= sel.0.para && para <= sel.1.para {
                            let start = if para == sel.0.para { sel.0.offset } else { 0 };
                            let end = if para == sel.1.para {
                                sel.1.offset
                            } else {
                                self.models[para].char_len
                            };
                            let extend = para != sel.1.para;
                            for r in selection_rects(&galley, start, end, extend) {
                                painter.rect_filled(
                                    r.translate(origin.to_vec2()),
                                    0.0,
                                    ui.visuals().selection.bg_fill,
                                );
                            }
                        }
                        painter.galley(origin, Arc::clone(&galley), Color32::BLACK);
                        if let Some(c) = self.cache.get(&para)
                            && let Some(overlay) = &c.bold_overlay
                        {
                            painter.galley(
                                origin + vec2(0.7, 0.0),
                                Arc::clone(overlay),
                                Color32::BLACK,
                            );
                        }
                        placed_new.push(para);
                    }
                }
                self.placed = placed_new;

                // Caret.
                let caret_rect = self.caret_rect(&ctx, canvas, self.caret);
                if focused && !has_sel {
                    painter.rect_filled(caret_rect, 0.0, Color32::BLACK);
                }
                if self.scroll_to_caret {
                    self.scroll_to_caret = false;
                    ui.scroll_to_rect(caret_rect.expand2(vec2(0.0, 24.0)), None);
                }

                // IME: tell the platform where the composition window belongs.
                if focused {
                    let to_global = ui
                        .ctx()
                        .layer_transform_to_global(ui.layer_id())
                        .unwrap_or_default();
                    ui.output_mut(|o| {
                        o.ime = Some(IMEOutput {
                            purpose: IMEPurpose::Normal,
                            rect: to_global * ui.clip_rect(),
                            cursor_rect: to_global * caret_rect,
                            should_interrupt_composition: false,
                        });
                    });
                }

                self.publish_accessibility(ui, canvas, focused);
                want_more |= self.dragging;
            });

        if self.cache.len() > 400 {
            let frame = self.frame;
            self.cache.retain(|_, c| frame - c.last_used < 120);
        }
        self.last_frame_ms = started.elapsed().as_secs_f32() * 1000.0;
        want_more
    }

    fn handle_pointer(&mut self, ctx: &Context, response: &egui::Response, canvas: Rect) {
        let pos = ctx.input(|i| i.pointer.interact_pos());
        let shift = ctx.input(|i| i.modifiers.shift);
        if response.drag_started() {
            if let Some(origin) = ctx.input(|i| i.pointer.press_origin()) {
                let hit = self.hit_test(ctx, canvas, origin);
                if shift {
                    self.caret = hit;
                } else {
                    self.set_caret(hit);
                }
                self.dragging = true;
            }
            response.request_focus();
        }
        if response.dragged()
            && let Some(pos) = pos
        {
            self.caret = self.hit_test(ctx, canvas, pos);
            self.preedit = None;
        }
        if response.drag_stopped() {
            self.dragging = false;
        }
        if response.double_clicked()
            && let Some(pos) = pos
        {
            let hit = self.hit_test(ctx, canvas, pos);
            let start = self.word_left(DocPos {
                para: hit.para,
                offset: (hit.offset + 1).min(self.models[hit.para].char_len),
            });
            let end = self.word_right(start);
            self.select(
                start,
                DocPos {
                    para: hit.para,
                    offset: end.offset.max(start.offset),
                },
            );
            response.request_focus();
        } else if response.clicked()
            && let Some(pos) = pos
        {
            let hit = self.hit_test(ctx, canvas, pos);
            if shift {
                self.caret = hit;
            } else {
                self.set_caret(hit);
            }
            self.preedit = None;
            response.request_focus();
        }
    }

    fn handle_keys(&mut self, ctx: &Context, target: &mut Target) {
        let events = ctx.input(|i| i.events.clone());
        let mut typed = String::new();
        for event in events {
            match event {
                Event::Text(t) => typed.push_str(&t),
                Event::Paste(t) => {
                    self.flush_typed(target, &mut typed);
                    self.paste_text(target, &t);
                }
                Event::Copy if self.has_selection() => ctx.copy_text(self.selected_text()),
                Event::Cut => {
                    self.flush_typed(target, &mut typed);
                    self.cut(target, ctx);
                }
                Event::Ime(ime) => match ime {
                    egui::ImeEvent::Preedit { text, .. } => {
                        self.preedit = (!text.is_empty()).then_some(text);
                        self.cache.remove(&self.caret.para);
                    }
                    egui::ImeEvent::Commit(text) => {
                        self.preedit = None;
                        self.cache.remove(&self.caret.para);
                        self.flush_typed(target, &mut typed);
                        self.insert_text(target, &text);
                    }
                    _ => {}
                },
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    self.flush_typed(target, &mut typed);
                    match key {
                        Key::ArrowLeft
                        | Key::ArrowRight
                        | Key::ArrowUp
                        | Key::ArrowDown
                        | Key::Home
                        | Key::End
                        | Key::PageUp
                        | Key::PageDown => self.move_caret(ctx, key, modifiers),
                        Key::A if modifiers.command => {
                            if let Some(last) = self.models.len().checked_sub(1) {
                                let end = DocPos {
                                    para: last,
                                    offset: self.models[last].char_len,
                                };
                                self.select(DocPos::default(), end);
                            }
                        }
                        Key::Backspace if self.preedit.is_none() => {
                            self.backspace(target, modifiers.command, ctx);
                        }
                        Key::Delete if self.preedit.is_none() => {
                            self.delete_forward(target, modifiers.command);
                        }
                        Key::Enter if self.preedit.is_none() => self.enter(target, modifiers.shift),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        self.flush_typed(target, &mut typed);
    }

    fn flush_typed(&mut self, target: &mut Target, typed: &mut String) {
        if !typed.is_empty() {
            let text = std::mem::take(typed);
            self.insert_text(target, &text);
        }
    }

    /// Screen-reader requests: moving the selection and dictation.
    fn handle_access_requests(&mut self, ctx: &Context, target: &mut Target) {
        let requests: Vec<accesskit::ActionRequest> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    Event::AccessKitActionRequest(r)
                        if r.target_node == self.doc_id.accesskit_id() =>
                    {
                        Some(r.clone())
                    }
                    _ => None,
                })
                .collect()
        });
        for request in requests {
            match (request.action, request.data) {
                (
                    accesskit::Action::SetTextSelection,
                    Some(accesskit::ActionData::SetTextSelection(sel)),
                ) => {
                    if let (Some(a), Some(f)) = (
                        self.pos_from_access(&sel.anchor),
                        self.pos_from_access(&sel.focus),
                    ) {
                        self.select(a, f);
                    }
                }
                (
                    accesskit::Action::ReplaceSelectedText,
                    Some(accesskit::ActionData::Value(text)),
                ) => self.insert_text(target, &text),
                _ => {}
            }
        }
    }

    fn pos_from_access(&self, p: &accesskit::TextPosition) -> Option<DocPos> {
        let run = self.runs.iter().find(|r| r.node == p.node)?;
        Some(self.clamp(DocPos {
            para: run.para,
            offset: run.start + p.character_index,
        }))
    }

    fn publish_accessibility(&mut self, ui: &mut Ui, canvas: Rect, focused: bool) {
        let ctx = ui.ctx().clone();
        if !a11y::is_enabled(&ctx, self.doc_id) {
            return;
        }
        let viewport = ui.clip_rect();
        let doc_id = self.doc_id;
        // Always expose the paragraphs holding the caret and selection ends so the text
        // selection resolves to published runs even when they are off screen.
        let count = self.models.len();
        let mut paras: Vec<usize> = self.placed.iter().copied().filter(|p| *p < count).collect();
        for p in [self.caret.para, self.anchor.para] {
            if !paras.contains(&p) && p < self.models.len() {
                paras.push(p);
            }
        }
        paras.sort_unstable();

        let mut run_ui = ui.new_child(
            UiBuilder::new()
                .id_salt("doc-a11y")
                .max_rect(viewport)
                .accessibility_parent(doc_id),
        );
        let mut entries = Vec::new();
        let mut galleys: HashMap<usize, Arc<Galley>> = HashMap::new();
        for para in paras {
            let galley = self.galley(&ctx, para);
            let origin = self.para_origin(canvas, para);
            entries.extend(a11y::publish_paragraph(
                &mut run_ui,
                doc_id,
                para,
                &galley,
                origin,
                None,
            ));
            galleys.insert(para, galley);
        }
        self.runs = entries;

        let selection = match (
            galleys.get(&self.anchor.para),
            galleys.get(&self.caret.para),
        ) {
            (Some(ag), Some(cg)) => Some(accesskit::TextSelection {
                anchor: a11y::text_position(doc_id, self.anchor.para, ag, self.anchor.offset),
                focus: a11y::text_position(doc_id, self.caret.para, cg, self.caret.offset),
            }),
            _ => None,
        };
        let title = format!("Document, {} paragraphs", self.models.len());
        ctx.accesskit_node_builder(doc_id, |b| {
            b.set_role(accesskit::Role::MultilineTextInput);
            b.set_label(title);
            b.set_bounds(a11y::to_ak_rect(viewport));
            b.add_action(accesskit::Action::Focus);
            b.add_action(accesskit::Action::SetTextSelection);
            b.add_action(accesskit::Action::ReplaceSelectedText);
            if let Some(sel) = selection {
                b.set_text_selection(sel);
            }
        });
        let _ = (focused, Align::Min);
    }

    /// Text of a paragraph as laid out (for tests and the status bar).
    pub fn paragraph_text(&self, para: usize) -> Option<&str> {
        self.models.get(para).map(|m| m.text.as_str())
    }
}

/// Highlight rectangles (galley-relative) for the characters `start..end` of a paragraph.
/// With `extend`, the last row's highlight is widened to show that the selection continues.
fn selection_rects(galley: &Galley, start: usize, end: usize, extend: bool) -> Vec<Rect> {
    let mut out = Vec::new();
    let mut idx = 0usize;
    let last = galley.rows.len().saturating_sub(1);
    for (i, row) in galley.rows.iter().enumerate() {
        let n = row.char_count_excluding_newline().0;
        let row_end = idx + n;
        let s = start.max(idx);
        let e = end.min(row_end);
        let continues = extend && i == last;
        if s < e || (continues && s <= row_end && end > idx.saturating_sub(0) && start <= row_end) {
            let x0 = row.pos.x + row.x_offset(CharIndex(s.saturating_sub(idx).min(n)));
            let mut x1 = row.pos.x + row.x_offset(CharIndex(e.saturating_sub(idx).min(n)));
            if continues {
                x1 += 6.0;
            }
            if x1 > x0 {
                out.push(Rect::from_min_max(
                    Pos2::new(x0, row.rect().min.y),
                    Pos2::new(x1, row.rect().max.y),
                ));
            }
        }
        idx += row.char_count_including_newline().0;
    }
    out
}
