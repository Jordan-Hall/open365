//! Paged document view over a DOCX [`Target`].
//!
//! Layout is virtualised: only paragraphs on visible pages get a galley that is drawn, galleys
//! are cached per paragraph and dropped when the paragraph is edited, and paragraph heights
//! (which decide the page breaks) are measured in the background with a small time budget per
//! frame. Until a paragraph is measured its height is estimated from its length.
//!
//! All edits go through [`Target::apply`]; the view keeps only the caret, the selection and
//! the layout caches.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use egui::epaint::text::CharIndex;
use egui::output::IMEOutput;
use egui::text::{CCursor, LayoutJob, TextFormat};
use egui::{
    Align, Color32, Context, Event, EventFilter, FontId, Galley, IMEPurpose, Id, Key, Modifiers,
    Pos2, Rect, Sense, Stroke, Ui, UiBuilder, Vec2, accesskit, vec2,
};
use open365_commands::{Command, Target};

use crate::a11y::{self, RunEntry};
use crate::docmodel::{PageSetup, ParaKind, ParaModel, SpanStyle, build_models};

const PAGE_GAP: f32 = 16.0;
const SIDE_PAD: f32 = 16.0;
/// Screen points per document point at 100% zoom (96 dpi).
const POINT_SCALE: f32 = 96.0 / 72.0;
const MEASURE_BUDGET_MS: f64 = 4.0;
const DEFAULT_SIZE_PT: f32 = 11.0;

/// A caret position: paragraph index and character offset within it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct DocPos {
    pub para: usize,
    pub offset: usize,
}

#[derive(Debug, Clone, Copy)]
struct PageSlice {
    first: usize,
    end: usize,
}

struct Cached {
    galley: Arc<Galley>,
    /// Same layout with only the bold spans visible, drawn offset to fake a bold weight (the
    /// bundled fonts have no bold face).
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
            status: None,
            last_frame_ms: 0.0,
        }
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
    }

    /// Selects the half-open range between two positions.
    pub fn select(&mut self, anchor: DocPos, focus: DocPos) {
        self.anchor = self.clamp(anchor);
        self.caret = self.clamp(focus);
        self.goal_x = None;
        self.scroll_to_caret = true;
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

    fn rebuild_all(&mut self, target: &Target) {
        let Some(doc) = target.docx() else {
            self.models.clear();
            return;
        };
        self.models = build_models(doc);
        self.page = PageSetup::from_document(doc);
        self.cache.clear();
        self.heights = (0..self.models.len()).map(|i| self.estimate(i)).collect();
        self.measured = vec![false; self.models.len()];
        self.measure_cursor = 0;
        self.paginate_dirty = true;
        self.caret = self.clamp(self.caret);
        self.anchor = self.clamp(self.anchor);
        self.model_revision = Some(target.revision());
    }

    fn refresh_para(&mut self, target: &Target, para: usize) {
        let Some(p) = target.docx().and_then(|d| d.paragraph(para)) else {
            return;
        };
        if let Some(slot) = self.models.get_mut(para) {
            *slot = ParaModel::from_paragraph(p);
        }
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

    fn font_size_pt(model: &ParaModel, style: &SpanStyle) -> f32 {
        if let Some(half) = style.size_half_points {
            return half as f32 / 2.0;
        }
        match model.kind {
            ParaKind::Title => 26.0,
            ParaKind::Heading(1) => 16.0,
            ParaKind::Heading(2) => 13.0,
            ParaKind::Heading(_) => 12.0,
            ParaKind::Normal => DEFAULT_SIZE_PT,
        }
    }

    fn space_after(&self, model: &ParaModel) -> f32 {
        let pt = match model.kind {
            ParaKind::Normal => 6.0,
            _ => 8.0,
        };
        pt * self.scale
    }

    fn estimate(&self, idx: usize) -> f32 {
        let Some(model) = self.models.get(idx) else {
            return 0.0;
        };
        let size = Self::font_size_pt(model, &SpanStyle::default()) * self.scale;
        let wrap = (self.page.body_width() * self.scale).max(50.0);
        let chars_per_line = (wrap / (size * 0.5)).max(1.0);
        let lines = ((model.char_len as f32 / chars_per_line).ceil() as usize).max(1)
            + model.text.matches('\n').count();
        lines as f32 * size * 1.3 + self.space_after(model)
    }

    fn build_jobs(&self, idx: usize, preedit: Option<&str>) -> (LayoutJob, Option<LayoutJob>) {
        let model = &self.models[idx];
        let wrap = (self.page.body_width() * self.scale).max(50.0);
        let mut job = LayoutJob::default();
        job.wrap.max_width = wrap;
        let mut overlay = LayoutJob::default();
        overlay.wrap.max_width = wrap;
        let mut any_bold = false;

        // Walk the text in runs of equal style, inserting the IME preedit at the caret.
        let preedit_at = preedit.map(|_| self.caret.offset.min(model.char_len));
        let heading_bold = !matches!(model.kind, ParaKind::Normal);
        let mut push = |text: &str, style: SpanStyle, is_preedit: bool| {
            if text.is_empty() {
                return;
            }
            let bold = style.bold || heading_bold;
            any_bold |= bold;
            let size = Self::font_size_pt(model, &style) * self.scale;
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
            let format = |color: Color32| TextFormat {
                font_id: FontId::proportional(size),
                color,
                italics: style.italic,
                underline,
                strikethrough: strike,
                ..TextFormat::default()
            };
            job.append(text, 0.0, format(color));
            overlay.append(
                text,
                0.0,
                format(if bold { color } else { Color32::TRANSPARENT }),
            );
        };

        let mut buf = String::new();
        let mut buf_style = SpanStyle::default();
        let mut span_i = 0usize;
        for (i, ch) in model.text.chars().enumerate() {
            if preedit_at == Some(i) {
                push(&buf, buf_style, false);
                buf.clear();
                push(preedit.unwrap_or_default(), buf_style, true);
            }
            while span_i < model.spans.len() && model.spans[span_i].end <= i {
                span_i += 1;
            }
            let style = match model.spans.get(span_i) {
                Some(s) if s.start <= i => s.style,
                _ => SpanStyle::default(),
            };
            if style != buf_style {
                push(&buf, buf_style, false);
                buf.clear();
                buf_style = style;
            }
            buf.push(ch);
        }
        push(&buf, buf_style, false);
        if preedit_at == Some(model.char_len) {
            push(preedit.unwrap_or_default(), buf_style, true);
        }
        if job.is_empty() {
            // An empty paragraph still needs a row of the right height.
            let size = Self::font_size_pt(model, &SpanStyle::default()) * self.scale;
            job.append(
                "",
                0.0,
                TextFormat::simple(FontId::proportional(size), Color32::BLACK),
            );
        }
        (job, any_bold.then_some(overlay))
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
        let (job, overlay) = self.build_jobs(idx, preedit.as_deref());
        let galley = ctx.fonts_mut(|f| f.layout_job(job));
        let bold_overlay = overlay.map(|j| ctx.fonts_mut(|f| f.layout_job(j)));
        let height =
            galley.size().y.max(self.min_row_height(idx)) + self.space_after(&self.models[idx]);
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
        self.models.get(idx).map_or(0.0, |m| {
            Self::font_size_pt(m, &SpanStyle::default()) * self.scale * 1.2
        })
    }

    /// Measures a few unmeasured paragraphs per frame so page breaks settle without a stall.
    fn measure_some(&mut self, ctx: &Context) -> bool {
        let start = Instant::now();
        while self.measure_cursor < self.models.len() {
            let i = self.measure_cursor;
            if !self.measured[i] {
                let (job, _) = self.build_jobs(i, None);
                let galley = ctx.fonts_mut(|f| f.layout_job(job));
                let height =
                    galley.size().y.max(self.min_row_height(i)) + self.space_after(&self.models[i]);
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
        Pos2::new(
            x0 + self.page.margin_left * self.scale,
            self.page_top(canvas.min.y, page)
                + self.page.margin_top * self.scale
                + self.para_y.get(para).copied().unwrap_or(0.0),
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

    fn run(&mut self, target: &mut Target, command: Command) -> bool {
        match target.apply(command) {
            Ok(_) => true,
            Err(e) => {
                self.status = Some(e.to_string());
                false
            }
        }
    }

    /// Inserts typed, pasted or committed text at the caret. The command layer has no delete
    /// yet, so typing over a selection collapses it to its end and inserts there.
    pub fn insert_text(&mut self, target: &mut Target, text: &str) {
        let clean: String = text
            .chars()
            .map(|c| if matches!(c, '\n' | '\r') { ' ' } else { c })
            .filter(|c| !c.is_control() || *c == '\t')
            .collect();
        if clean.is_empty() || self.models.is_empty() {
            return;
        }
        let at = if self.has_selection() {
            self.selection().1
        } else {
            self.caret
        };
        let at = self.clamp(at);
        let added = clean.chars().count();
        let ok = self.run(
            target,
            Command::DocxInsertText {
                para: at.para,
                offset: at.offset,
                text: clean,
            },
        );
        if ok {
            self.refresh_para(target, at.para);
            let pos = DocPos {
                para: at.para,
                offset: at.offset + added,
            };
            self.set_caret(pos);
        }
    }

    /// Toggles bold over the selection: bold unless all of it is already bold.
    pub fn toggle_bold(&mut self, target: &mut Target) {
        if !self.has_selection() {
            self.status = Some("Select text to make it bold".to_owned());
            return;
        }
        let (a, b) = self.selection();
        let range_of = |p: usize, models: &[ParaModel]| {
            let len = models[p].char_len;
            let start = if p == a.para { a.offset } else { 0 };
            let end = if p == b.para { b.offset } else { len };
            (start, end.max(start))
        };
        let all_bold = (a.para..=b.para).all(|p| {
            let (s, e) = range_of(p, &self.models);
            s == e || self.models[p].is_bold(s, e)
        });
        let bold = !all_bold;
        for p in a.para..=b.para {
            let (start, end) = range_of(p, &self.models);
            if start == end {
                continue;
            }
            if self.run(
                target,
                Command::DocxSetBold {
                    para: p,
                    start,
                    end,
                    bold,
                },
            ) {
                self.refresh_para(target, p);
            }
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
        if self.model_revision != Some(target.revision()) {
            self.rebuild_all(target);
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
        let mut want_more = more;

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
                    self.insert_text(target, &t);
                }
                Event::Copy | Event::Cut => {
                    if self.has_selection() {
                        let text = self.selected_text();
                        ctx.copy_text(text);
                    }
                    if matches!(event, Event::Cut) {
                        self.status = Some("Cut is not available yet (no delete command)".into());
                    }
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
                        Key::Backspace | Key::Delete | Key::Enter if self.preedit.is_none() => {
                            self.status = Some(
                                "Deleting and splitting paragraphs need a docx delete API (not yet available)"
                                    .into(),
                            );
                        }
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
        let mut paras: Vec<usize> = self.placed.clone();
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
