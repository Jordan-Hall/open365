//! Paged document view over a DOCX [`Target`].
//!
//! Pages, lines and glyph positions come from `open365-layout`, the Word-parity layout engine,
//! so the screen shows Word's line and page breaks. The view draws exactly what layout
//! computed: page rectangles, then the positioned glyphs of each visible line, rasterised from
//! the same font files layout measured with (see `glyphs.rs`). Only pages that intersect the
//! viewport are painted, so a long document costs the same to scroll as a short one.
//!
//! Layout is rerun after every command (the edit helpers mark it dirty, the next navigation or
//! frame rebuilds it). The caret, the selection and hit-testing work on layout lines and glyph
//! positions. All edits go through [`Target::apply`]; the view keeps only the caret, the
//! selection and the layout.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use egui::output::IMEOutput;
use egui::{
    Color32, Context, Event, EventFilter, IMEPurpose, Id, Pos2, Rect, Sense, Shape, Stroke, Ui,
    UiBuilder, Vec2, accesskit, vec2,
};
use open365_commands::{Command, CommandError, Outcome, Target};
use open365_docx::{Alignment, RunPropsPatch, TabAlign};
use open365_layout::{
    BorderLine, BorderStyle, ClusterKind, IncrementalLayout, Layout, LayoutOptions, Line, ParaRole,
    PlacedGlyph,
};
use open365_text::{FaceId, FontStyle, ShapeFeatures, shape_run};

use self::glyphs::{Atlas, FontSet, GlyphBatch};
use self::keys::{KeyAction, Nav};
use crate::a11y::{self, RunEntry};
use crate::docmodel::{ParaModel, SpanStyle, build_models};

use crate::parts;

// The glyph atlas and the key map live next to this file; `lib.rs` need not know them.
#[path = "glyphs.rs"]
mod glyphs;
#[path = "keys.rs"]
mod keys;
#[path = "ruler_view.rs"]
mod ruler_view;

pub use self::ruler_view::RulerColors;

pub use open365_docx::DocPos;

/// Gap between pages, in document points. Word at 100 % zoom: 18 px at 125 % scale = 14.4 dips = 10.8 pt
/// (`fixtures/reference/ui/word-page.json`: first characters of pages 1 and 2 are 1421 px apart, a page is 1403.2 px).
const PAGE_GAP_PT: f64 = 10.8;
/// Space above the first page: 21 u (physical px at 125 %) = 16.8 points = 12.6 document points. Word's document area
/// starts at y 222 (the shell's content top) and the first page outline is at y 243; with the ruler on the band
/// (222..252) sits above the canvas and the outline is at 273, again 21 px under it (`word-page.json` firstChar).
const PAGE_TOP_PT: f64 = 12.6;
const SIDE_PAD: f32 = 16.0;
/// Screen points per document point at 100% zoom (96 dpi).
const POINT_SCALE: f32 = 96.0 / 72.0;
/// Width of the highlight past the end of a selected paragraph, document points.
const MARK_WIDTH_PT: f64 = 4.0;
/// Opacity of header and footer text and pictures: Word dims them while the body is edited.
const FURNITURE_DIM: f32 = 0.45;
/// Spacing of the accessibility ids of the picture groups (body, header, notes, footer).
const IMAGE_GROUP_STRIDE: usize = 1 << 28;

/// Which character toggle a button or shortcut flips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    Bold,
    Italic,
    Underline,
    Strike,
}

impl Toggle {
    fn test(self, style: &SpanStyle) -> bool {
        match self {
            Self::Bold => style.bold,
            Self::Italic => style.italic,
            Self::Underline => style.underline,
            Self::Strike => style.strike,
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
            Self::Strike => RunPropsPatch {
                strike: Some(on),
                ..RunPropsPatch::default()
            },
        }
    }
}

/// A line of the layout by page and index within the page.
#[derive(Debug, Clone, Copy)]
struct LineRef {
    page: u32,
    line: u32,
}

/// Lookup tables over a [`Layout`]: lines in reading order, which lines belong to a paragraph
/// and where each page sits in the scrolling canvas (document points).
#[derive(Debug, Default)]
struct LayoutIndex {
    refs: Vec<LineRef>,
    /// Index into `refs` of each page's first line.
    page_first: Vec<usize>,
    /// Index into `refs` of each paragraph's first line, one extra entry at the end.
    para_first: Vec<usize>,
    /// Story offset of each paragraph's first character.
    para_story: Vec<usize>,
    page_top: Vec<f64>,
    total_height: f64,
    max_width: f64,
}

impl LayoutIndex {
    fn build(layout: &Layout, paras: usize) -> Self {
        let mut refs = Vec::new();
        let mut page_first = Vec::new();
        let mut page_top = Vec::new();
        let mut y = PAGE_TOP_PT;
        let mut max_width = 0.0f64;
        for (pi, page) in layout.pages.iter().enumerate() {
            page_first.push(refs.len());
            page_top.push(y);
            y += page.section.page_height + PAGE_GAP_PT;
            max_width = max_width.max(page.section.page_width);
            for li in 0..page.lines.len() {
                refs.push(LineRef {
                    page: pi as u32,
                    line: li as u32,
                });
            }
        }
        let mut para_first = vec![usize::MAX; paras + 1];
        let mut story: Vec<Option<usize>> = vec![None; paras];
        for (ri, r) in refs.iter().enumerate() {
            let line = &layout.pages[r.page as usize].lines[r.line as usize];
            if line.paragraph < paras && para_first[line.paragraph] == usize::MAX {
                para_first[line.paragraph] = ri;
                story[line.paragraph] = Some(line.start);
            }
        }
        para_first[paras] = refs.len();
        for p in (0..paras).rev() {
            if para_first[p] == usize::MAX {
                para_first[p] = para_first[p + 1];
            }
        }
        let mut prev = 0usize;
        let para_story = story
            .into_iter()
            .map(|s| {
                prev = s.unwrap_or(prev);
                prev
            })
            .collect();
        Self {
            refs,
            page_first,
            para_first,
            para_story,
            page_top,
            total_height: y,
            max_width,
        }
    }

    /// The page whose slot (page plus the gap below it) contains document y `y`.
    fn page_at(&self, y: f64) -> usize {
        self.page_top
            .partition_point(|t| *t <= y + PAGE_GAP_PT / 2.0)
            .saturating_sub(1)
    }
}

/// Character boundaries of one layout line in page coordinates.
#[derive(Debug, Clone)]
struct LineGeom {
    /// Paragraph character offset of the first character of the line.
    cs: usize,
    /// One past the last character (the paragraph mark is not a character).
    ce: usize,
    /// X (document points) of the left edge of every character, plus the right edge of the
    /// last: `ce - cs + 1` entries.
    xs: Vec<f64>,
}

/// State of the document view.
#[derive(Debug)]
pub struct DocView {
    /// Page colour, automatic text colour and optional page outline (Word dark mode draws a dark
    /// page with an outline instead of the shadow). See [`DocView::set_page_colors`].
    page_fill: Color32,
    auto_text: Color32,
    page_border: Option<Color32>,
    models: Vec<ParaModel>,
    model_revision: Option<u64>,
    fonts: FontSet,
    atlas: Atlas,
    /// The layout, kept up to date incrementally as the document is edited.
    engine: IncrementalLayout,
    /// Whether installed system fonts are used before the bundled ones.
    system_fonts: bool,
    /// Families already looked up on the system, found or not.
    probed: HashSet<String>,
    index: LayoutIndex,
    layout_dirty: bool,
    /// The next model rebuild follows undo or redo: the layout cache stays, because the
    /// incremental engine finds the changed blocks by comparing them with what it laid out.
    after_history: bool,
    caret: DocPos,
    anchor: DocPos,
    /// The caret sits at the end of a wrapped line rather than the start of the next.
    upstream: bool,
    preedit: Option<String>,
    /// Horizontal goal of up/down movement, document points in page coordinates.
    goal_x: Option<f64>,
    runs: Vec<RunEntry>,
    /// The canvas of the last frame, for mapping points to the screen and back.
    canvas: Rect,
    /// Screen points per document point.
    scale: f32,
    /// Explicit zoom (1.0 is 96 dpi); `None` shrinks pages to fit narrow windows.
    zoom: Option<f32>,
    /// Height of the viewport in document points.
    view_h: f64,
    scroll_to_caret: bool,
    dragging: bool,
    want_focus: bool,
    /// Keyboard focus as of the last frame.
    focused: bool,
    /// Caret width in points and the length of its shown (and hidden) half cycle in seconds (`None`: no blink).
    caret_width: f32,
    blink_half: Option<f64>,
    /// Input time of the last keystroke, caret move or focus change: the blink cycle starts here.
    blink_epoch: f64,
    blink_sig: Option<(DocPos, DocPos, bool)>,
    /// The caret was drawn in the last frame.
    caret_shown: bool,
    doc_id: Id,
    /// Character formatting to apply to the next typed text (a toggle with no selection).
    pending: RunPropsPatch,
    /// Paragraphs whose lines were painted last frame.
    painted: Vec<usize>,
    /// Pages painted last frame (first, last), for the read-only regions of the tree.
    visible_pages: (usize, usize),
    /// Decoded pictures by relationship id (`None`: undecodable), valid for one revision.
    pictures: HashMap<String, Option<Picture>>,
    pictures_rev: Option<u64>,
    /// Message for the status bar, taken by the application each frame.
    pub status: Option<String>,
    /// Milliseconds spent on the last frame (layout, input, paint, accessibility).
    pub last_frame_ms: f32,
    /// Milliseconds the last relayout took.
    pub last_layout_ms: f32,
    /// Milliseconds spent building the visible pages' draw commands last frame.
    pub last_paint_ms: f32,
    /// Rulers (View > Ruler): visibility, colours, the tab kind the corner selects, and a drag in progress.
    ruler: ruler_view::RulerState,
}

impl Default for DocView {
    fn default() -> Self {
        Self::new()
    }
}

impl DocView {
    pub fn new() -> Self {
        Self {
            page_fill: Color32::WHITE,
            auto_text: Color32::BLACK,
            page_border: None,
            models: Vec::new(),
            model_revision: None,
            fonts: FontSet::bundled(),
            atlas: Atlas::default(),
            engine: IncrementalLayout::new(),
            system_fonts: false,
            probed: HashSet::new(),
            index: LayoutIndex::default(),
            layout_dirty: true,
            after_history: false,
            caret: DocPos::default(),
            anchor: DocPos::default(),
            upstream: false,
            preedit: None,
            goal_x: None,
            runs: Vec::new(),
            canvas: Rect::from_min_size(Pos2::ZERO, Vec2::ZERO),
            scale: POINT_SCALE,
            zoom: None,
            view_h: 700.0,
            scroll_to_caret: false,
            dragging: false,
            want_focus: true,
            focused: false,
            caret_width: 1.0,
            blink_half: Some(0.53),
            blink_epoch: 0.0,
            blink_sig: None,
            caret_shown: false,
            doc_id: Id::new("open365-doc-view"),
            pending: RunPropsPatch::default(),
            painted: Vec::new(),
            visible_pages: (0, 0),
            pictures: HashMap::new(),
            pictures_rev: None,
            status: None,
            last_frame_ms: 0.0,
            last_layout_ms: 0.0,
            last_paint_ms: 0.0,
            ruler: ruler_view::RulerState::default(),
        }
    }

    /// Sets the caret like Windows does: width in points and the blink time in seconds (`None`: INFINITE, no blink).
    pub fn set_caret_style(&mut self, width: f32, blink_secs: Option<f64>) {
        self.caret_width = width.max(0.5);
        self.blink_half = blink_secs;
    }

    /// The document holds the keyboard focus (as of the last frame).
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// The caret was drawn in the last frame (focused, no selection, in the shown half of the blink cycle).
    pub fn caret_shown(&self) -> bool {
        self.caret_shown
    }

    /// Gives the document keyboard focus on the next frame (after a toolbar action).
    /// Sets the page fill, the colour of text with the automatic colour (and the caret) and the page
    /// outline. Defaults: white page, black text, shadow.
    pub fn set_page_colors(&mut self, fill: Color32, auto_text: Color32, border: Option<Color32>) {
        self.page_fill = fill;
        self.auto_text = auto_text;
        self.page_border = border;
    }

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
        self.engine.layout().pages.len().max(1)
    }

    /// The page (0-based) a paragraph starts on.
    pub fn page_of(&self, para: usize) -> usize {
        self.index
            .para_first
            .get(para)
            .and_then(|ri| self.index.refs.get(*ri))
            .map_or(0, |r| r.page as usize)
    }

    /// The laid-out document the view draws.
    /// Writes the document as a tagged PDF from the layout on screen (the print path, ADR-004):
    /// the same pages, glyph positions and fonts the view draws.
    pub fn export_pdf(
        &mut self,
        target: &Target,
        meta: &open365_pdf::PdfMetadata,
    ) -> Result<open365_pdf::PdfOutput, String> {
        self.ensure_layout(target);
        let doc = target
            .docx()
            .ok_or_else(|| "only Word documents can be exported to PDF".to_owned())?;
        open365_pdf::export_pdf(doc, self.engine.layout(), &self.fonts.library, meta)
            .map_err(|e| e.to_string())
    }

    pub fn layout(&self) -> &Layout {
        self.engine.layout()
    }

    /// How many paragraphs had lines painted last frame (the visible ones).
    pub fn painted_paragraph_count(&self) -> usize {
        self.painted.len()
    }

    /// Sets the zoom (1.0 is 96 dpi, `None` fits pages to the window width at most 100%).
    pub fn set_zoom(&mut self, zoom: Option<f32>) {
        self.zoom = zoom.map(|z| z.clamp(0.25, 4.0));
    }

    pub fn zoom(&self) -> Option<f32> {
        self.zoom
    }

    /// Screen points per document point used by the last frame.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Places the caret (clamped) and clears the selection.
    pub fn set_caret(&mut self, pos: DocPos) {
        let pos = self.clamp(pos);
        self.caret = pos;
        self.anchor = pos;
        self.goal_x = None;
        self.upstream = false;
        self.scroll_to_caret = true;
        self.pending = RunPropsPatch::default();
    }

    /// Selects the half-open range between two positions.
    pub fn select(&mut self, anchor: DocPos, focus: DocPos) {
        self.anchor = self.clamp(anchor);
        self.caret = self.clamp(focus);
        self.goal_x = None;
        self.upstream = false;
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

    /// Throws away everything derived from the document (after open or a replaced document).
    pub fn reset(&mut self) {
        self.model_revision = None;
        self.after_history = false;
        self.engine.invalidate();
    }

    /// Undoes the last change. The layout is updated incrementally on the next frame: only the
    /// blocks the undo changed are laid out again.
    pub fn undo(&mut self, target: &mut Target) -> Result<Outcome, CommandError> {
        let outcome = target.undo()?;
        self.history_applied();
        Ok(outcome)
    }

    /// Redoes the last undone change (see [`DocView::undo`]).
    pub fn redo(&mut self, target: &mut Target) -> Result<Outcome, CommandError> {
        let outcome = target.redo()?;
        self.history_applied();
        Ok(outcome)
    }

    fn history_applied(&mut self) {
        self.model_revision = None;
        self.after_history = true;
        self.preedit = None;
        self.pending = RunPropsPatch::default();
    }

    /// What the last layout update did (blocks laid out again, whether it was a full pass).
    pub fn layout_stats(&self) -> open365_layout::UpdateStats {
        self.engine.stats()
    }

    /// Uses the fonts installed on this machine before the bundled ones, so the layout on
    /// screen is Word's whenever the document's fonts are installed. Loads the system fonts
    /// now (it takes a moment). The default, and what the tests use, is the bundled set only.
    pub fn use_system_fonts(&mut self) {
        if self.system_fonts {
            return;
        }
        self.system_fonts = true;
        self.fonts = FontSet::system();
        self.probed = glyphs::SYSTEM_FAMILIES
            .iter()
            .map(|f| (*f).to_owned())
            .collect();
        self.engine.invalidate();
        self.layout_dirty = true;
    }

    /// Looks up, once, the document's fonts that are not loaded yet among the installed ones.
    /// True when faces were added.
    fn probe_system_fonts(&mut self) -> bool {
        if !self.system_fonts {
            return false;
        }
        let missing: Vec<String> = self
            .engine
            .layout()
            .fonts
            .iter()
            .filter(|f| {
                f.matched != open365_text::FontMatch::Exact && !self.probed.contains(&f.requested)
            })
            .map(|f| f.requested.clone())
            .collect();
        if missing.is_empty() {
            return false;
        }
        self.probed.extend(missing.iter().cloned());
        let names: Vec<&str> = missing.iter().map(String::as_str).collect();
        self.fonts.add_system_families(&names) > 0
    }

    /// Fonts the document asks for that the bundled set stands in for, as (requested, used).
    pub fn font_substitutions(&self) -> Vec<(String, String)> {
        self.engine
            .layout()
            .fonts
            .iter()
            .filter(|f| f.matched != open365_text::FontMatch::Exact)
            .map(|f| {
                let used = self
                    .fonts
                    .resolve(&f.requested, false, false)
                    .map_or_else(String::new, |id| {
                        self.fonts.library.face(id).family().to_owned()
                    });
                (f.requested.clone(), used)
            })
            .collect()
    }

    fn rebuild_all(&mut self, target: &Target) {
        let Some(doc) = target.docx() else {
            self.models.clear();
            return;
        };
        self.models = build_models(doc);
        // The document changed in a way the edit helpers did not follow. After undo or redo
        // the incremental layout still diffs against its cache; for anything else (a replaced
        // document) nothing cached can be trusted.
        if !std::mem::take(&mut self.after_history) {
            self.engine.invalidate();
        }
        self.caret = self.clamp(self.caret);
        self.anchor = self.clamp(self.anchor);
        self.layout_dirty = true;
        self.model_revision = Some(target.revision());
    }

    fn refresh_para(&mut self, target: &Target, para: usize) {
        let Some(doc) = target.docx() else { return };
        let Some(p) = doc.paragraph(para) else { return };
        if let Some(slot) = self.models.get_mut(para) {
            *slot = ParaModel::from_paragraph(p, doc.styles());
        }
        self.layout_dirty = true;
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
        self.layout_dirty = true;
        self.model_revision = Some(target.revision());
    }

    /// Lays the document out again when an edit invalidated the layout.
    fn ensure_layout(&mut self, target: &Target) {
        let started = Instant::now();
        if self.model_revision != Some(target.revision()) {
            self.rebuild_all(target);
        }
        if !self.layout_dirty {
            return;
        }
        if let Some(doc) = target.docx() {
            let options = LayoutOptions::default();
            self.engine.update(doc, &self.fonts.library, options);
            if self.probe_system_fonts() {
                // New faces came in: lay everything out again with them.
                self.engine.invalidate();
                self.engine.update(doc, &self.fonts.library, options);
            }
        } else {
            self.engine = IncrementalLayout::new();
        }
        self.index = LayoutIndex::build(self.engine.layout(), self.models.len());
        self.layout_dirty = false;
        self.last_layout_ms = started.elapsed().as_secs_f32() * 1000.0;
    }

    // ----- layout queries -----

    fn line(&self, ri: usize) -> &Line {
        let r = self.index.refs[ri];
        &self.engine.layout().pages[r.page as usize].lines[r.line as usize]
    }

    fn line_range_of(&self, para: usize) -> std::ops::Range<usize> {
        let first = self.index.para_first.get(para).copied().unwrap_or(0);
        let end = self
            .index
            .para_first
            .get(para + 1)
            .copied()
            .unwrap_or(first);
        first..end.max(first)
    }

    /// Story offset (UTF-16 units) of a position.
    fn story_of(&self, pos: DocPos) -> usize {
        let Some(model) = self.models.get(pos.para) else {
            return 0;
        };
        let base = self.index.para_story.get(pos.para).copied().unwrap_or(0);
        base + model
            .text
            .chars()
            .take(pos.offset)
            .map(char::len_utf16)
            .sum::<usize>()
    }

    /// Characters of the paragraph that sit on `line`: (first, one past the last).
    fn char_range(&self, para: usize, line: &Line) -> (usize, usize) {
        let Some(model) = self.models.get(para) else {
            return (0, 0);
        };
        let base = self.index.para_story.get(para).copied().unwrap_or(0);
        let from = line.start.saturating_sub(base);
        let to = line.end.saturating_sub(base);
        let (mut cs, mut ce, mut found) = (model.char_len, model.char_len, false);
        let mut units = 0usize;
        for (i, c) in model.text.chars().enumerate() {
            if !found && units >= from {
                cs = i;
                found = true;
            }
            if units >= to {
                ce = i;
                break;
            }
            units += c.len_utf16();
        }
        (cs, ce.max(cs))
    }

    /// The reading-order line index that holds `pos`.
    fn line_ref_for(&self, pos: DocPos, upstream: bool) -> Option<usize> {
        if self.index.refs.is_empty() {
            return None;
        }
        let range = self.line_range_of(pos.para);
        let fallback = range.start.min(self.index.refs.len() - 1);
        if range.is_empty() {
            return Some(fallback);
        }
        let story = self.story_of(pos);
        if upstream {
            let hit = range
                .clone()
                .find(|ri| self.line(*ri).start <= story && story <= self.line(*ri).end);
            return Some(hit.unwrap_or(fallback));
        }
        let hit = range.rev().find(|ri| self.line(*ri).start <= story);
        Some(hit.unwrap_or(fallback))
    }

    /// Where each character of a line starts, from the layout's cluster map: every glyph says
    /// which story range it stands for, so a tab (no glyph) falls back to the pen and a
    /// ligature (one glyph, several characters) is shared out by UTF-16 units.
    fn line_geom(&self, ri: usize) -> LineGeom {
        let line = self.line(ri);
        let para = line.paragraph.min(self.models.len().saturating_sub(1));
        let (cs, ce) = self.char_range(para, line);
        let chars: Vec<char> = self
            .models
            .get(para)
            .map(|m| m.text.chars().skip(cs).take(ce - cs).collect())
            .unwrap_or_default();
        // Clusters in story order: (start, end, left x, right x). Glyphs of one cluster merge.
        let mut spans: Vec<(usize, usize, f64, f64)> = Vec::new();
        for g in line.runs.iter().flat_map(|r| &r.glyphs) {
            if g.cluster.kind != ClusterKind::Text {
                continue;
            }
            let r = line.cluster_story(g);
            if r.is_empty() {
                continue;
            }
            match spans.last_mut() {
                Some(last) if last.0 == r.start && last.1 == r.end => {
                    last.3 = last.3.max(g.x + g.advance);
                }
                _ => spans.push((r.start, r.end, g.x, g.x + g.advance)),
            }
        }
        let mut xs = Vec::with_capacity(chars.len() + 1);
        let mut pen = line.left;
        let (mut u, mut si) = (line.start, 0usize);
        for c in &chars {
            let n = c.len_utf16();
            while si < spans.len() && spans[si].1 <= u {
                si += 1;
            }
            match spans.get(si) {
                Some(&(start, end, x0, x1)) if start <= u => {
                    let w = (end - start) as f64;
                    xs.push(x0 + (x1 - x0) * (u - start) as f64 / w);
                    pen = x0 + (x1 - x0) * ((u + n).min(end) - start) as f64 / w;
                }
                _ => xs.push(pen),
            }
            u += n;
        }
        xs.push(pen);
        LineGeom { cs, ce, xs }
    }

    /// X (document points, page coordinates) of a position on its line.
    fn x_of(&self, pos: DocPos, upstream: bool) -> Option<(usize, f64)> {
        let ri = self.line_ref_for(pos, upstream)?;
        let geom = self.line_geom(ri);
        let i = pos.offset.saturating_sub(geom.cs).min(geom.ce - geom.cs);
        Some((ri, geom.xs[i]))
    }

    /// The position on line `ri` nearest to page x `x`.
    fn pos_on_line(&self, ri: usize, x: f64) -> DocPos {
        let line = self.line(ri);
        let para = line.paragraph.min(self.models.len().saturating_sub(1));
        let geom = self.line_geom(ri);
        let n = geom.ce - geom.cs;
        let mut best = 0usize;
        let mut best_d = f64::MAX;
        for (i, edge) in geom.xs.iter().enumerate() {
            let d = (edge - x).abs();
            if d < best_d {
                best = i;
                best_d = d;
            }
        }
        // The end of a wrapped line is the start of the next one: stop before it.
        let wraps = ri + 1 < self.line_range_of(para).end;
        if wraps && n > 0 {
            best = best.min(n - 1);
        }
        DocPos {
            para,
            offset: geom.cs + best,
        }
    }

    /// The position nearest to document point (`x`, `y`) on `page` (`y` from the page top).
    fn hit_page(&self, page: usize, x: f64, y: f64) -> DocPos {
        let Some(p) = self.engine.layout().pages.get(page) else {
            return DocPos::default();
        };
        let first = self.index.page_first[page];
        let ri = if p.lines.is_empty() {
            // A page with no lines: the nearest line before it, else the first line.
            first.saturating_sub(1)
        } else {
            let k = p.lines.partition_point(|l| l.top + l.height <= y);
            first + k.min(p.lines.len() - 1)
        };
        if self.index.refs.is_empty() {
            return DocPos::default();
        }
        self.pos_on_line(ri.min(self.index.refs.len() - 1), x)
    }

    /// Screen position of the top-left corner of `page` on a canvas.
    fn page_origin(&self, canvas: Rect, page: usize) -> Pos2 {
        let width = self
            .engine
            .layout()
            .pages
            .get(page)
            .map_or(612.0, |p| p.section.page_width) as f32
            * self.scale;
        Pos2::new(
            canvas.min.x + ((canvas.width() - width) / 2.0).max(SIDE_PAD),
            canvas.min.y
                + self
                    .index
                    .page_top
                    .get(page)
                    .copied()
                    .unwrap_or(PAGE_TOP_PT) as f32
                    * self.scale,
        )
    }

    fn hit_test(&self, canvas: Rect, pos: Pos2) -> DocPos {
        if self.models.is_empty() || self.engine.layout().pages.is_empty() {
            return DocPos::default();
        }
        let gy = f64::from((pos.y - canvas.min.y) / self.scale);
        let page = self
            .index
            .page_at(gy)
            .min(self.engine.layout().pages.len() - 1);
        let origin = self.page_origin(canvas, page);
        let x = f64::from((pos.x - origin.x) / self.scale);
        let y = gy - self.index.page_top[page];
        self.hit_page(page, x, y)
    }

    /// The caret's line and geometry: (page, x, top, height) in page document points.
    fn caret_geom(&self, pos: DocPos) -> Option<(usize, f64, f64, f64)> {
        let upstream = self.upstream && pos == self.caret;
        let (ri, x) = self.x_of(pos, upstream)?;
        let line = self.line(ri);
        let preedit_dx = if pos == self.caret {
            self.preedit_shape().map_or(0.0, |(_, w, _)| w)
        } else {
            0.0
        };
        Some((
            self.index.refs[ri].page as usize,
            x + preedit_dx,
            line.top,
            line.height,
        ))
    }

    /// Highlight extent of a selection on line `ri`, page x range in document points.
    fn selection_span(&self, ri: usize, sel: (DocPos, DocPos)) -> Option<(f64, f64)> {
        let line = self.line(ri);
        let para = line.paragraph;
        if para < sel.0.para || para > sel.1.para {
            return None;
        }
        let model = self.models.get(para)?;
        let geom = self.line_geom(ri);
        let start = if para == sel.0.para { sel.0.offset } else { 0 };
        let (end, past_mark) = if para == sel.1.para {
            (sel.1.offset, false)
        } else {
            (model.char_len, true)
        };
        let last_line = ri + 1 == self.line_range_of(para).end;
        let s = start.max(geom.cs);
        let e = end.min(geom.ce);
        let mark = past_mark && last_line;
        if s >= e && !mark {
            return None;
        }
        let x0 = geom.xs[s.min(geom.ce).saturating_sub(geom.cs)];
        let mut x1 = geom.xs[e.max(s).saturating_sub(geom.cs).min(geom.ce - geom.cs)];
        if mark {
            x1 += MARK_WIDTH_PT;
        }
        (x1 > x0).then_some((x0, x1))
    }

    /// The glyphs of the IME preedit text at the caret: (glyph, face, x offset in points),
    /// the total width and the size in points.
    #[allow(clippy::type_complexity)]
    fn preedit_shape(&self) -> Option<(Vec<(u32, FaceId, f64)>, f64, f64)> {
        let text = self.preedit.as_deref()?;
        let model = self.models.get(self.caret.para)?;
        let style = model.style_at(self.caret.offset);
        let chain = self.fonts.library.chain(
            &style.font,
            FontStyle {
                bold: style.bold,
                italic: style.italic,
            },
        );
        let shaped = shape_run(&self.fonts.library, &chain, text, ShapeFeatures::default());
        let size = f64::from(style.size_pt());
        let mut x = 0.0;
        let mut out = Vec::new();
        for g in &shaped.glyphs {
            let upem = f64::from(self.fonts.library.face(g.face).units_per_em());
            out.push((g.glyph_id, g.face, x));
            x += f64::from(g.x_advance) * size / upem;
        }
        Some((out, x, size))
    }

    /// The published lines of a paragraph for the accessibility tree, in screen points.
    fn a11y_lines(&self, para: usize, canvas: Rect) -> Vec<a11y::LineData> {
        let range = self.line_range_of(para);
        let mut out = Vec::new();
        for ri in range.clone() {
            let line = self.line(ri);
            let page = self.index.refs[ri].page as usize;
            let origin = self.page_origin(canvas, page);
            let geom = self.line_geom(ri);
            let chars: Vec<char> = self.models[para]
                .text
                .chars()
                .skip(geom.cs)
                .take(geom.ce - geom.cs)
                .collect();
            let x0 = geom.xs[0];
            let s = f64::from(self.scale);
            let positions = geom.xs[..chars.len()]
                .iter()
                .map(|x| ((x - x0) * s) as f32)
                .collect();
            let widths = geom
                .xs
                .windows(2)
                .map(|w| ((w[1] - w[0]) * s) as f32)
                .collect();
            let rect = Rect::from_min_max(
                Pos2::new(origin.x + (x0 * s) as f32, origin.y + (line.top * s) as f32),
                Pos2::new(
                    origin.x + (geom.xs[chars.len()] * s) as f32,
                    origin.y + ((line.top + line.height) * s) as f32,
                ),
            );
            let continuation = ri > range.start
                && self.line(ri - 1).end == line.start
                && self.models[para]
                    .text
                    .chars()
                    .nth(geom.cs.saturating_sub(1))
                    != Some('\n');
            out.push(a11y::LineData {
                rect,
                start: geom.cs,
                chars,
                positions,
                widths,
                continuation,
                last: ri + 1 == range.end,
            });
        }
        if out.is_empty() {
            out.push(a11y::LineData {
                rect: Rect::from_min_size(canvas.min, Vec2::ZERO),
                start: 0,
                chars: Vec::new(),
                positions: Vec::new(),
                widths: Vec::new(),
                continuation: false,
                last: true,
            });
        }
        out
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
                Toggle::Strike => self.pending.strike.unwrap_or(current.strike),
            };
            let flipped = which.patch(!on);
            match which {
                Toggle::Bold => self.pending.bold = flipped.bold,
                Toggle::Italic => self.pending.italic = flipped.italic,
                Toggle::Underline => self.pending.underline = flipped.underline,
                Toggle::Strike => self.pending.strike = flipped.strike,
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

    /// Applies character formatting (font name, size, colour...) over the selection, or to the
    /// text typed next when there is none.
    pub fn set_run_props(&mut self, target: &mut Target, patch: RunPropsPatch) {
        if self.models.is_empty() || patch.is_empty() {
            return;
        }
        if !self.has_selection() {
            let p = &mut self.pending;
            p.bold = patch.bold.or(p.bold);
            p.italic = patch.italic.or(p.italic);
            p.underline = patch.underline.or(p.underline);
            p.strike = patch.strike.or(p.strike);
            p.size_half_points = patch.size_half_points.or(p.size_half_points);
            p.color = patch.color.or_else(|| p.color.take());
            p.font = patch.font.or_else(|| p.font.take());
            return;
        }
        let (a, b) = self.range();
        let command = Command::DocxSetRunProps {
            start: a,
            end: b,
            patch,
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
        if let Some(v) = self.pending.strike {
            style.strike = v;
        }
        if let Some(v) = self.pending.size_half_points {
            style.size_half_points = v;
        }
        if let Some(v) = &self.pending.font {
            style.font = std::sync::Arc::from(v.as_str());
            style.theme_font = None;
        }
        if let Some(v) = &self.pending.color {
            style.color = parse_hex_color(v);
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

    // ----- caret movement -----

    fn move_caret(&mut self, nav: Nav, select: bool, big: bool) {
        if self.models.is_empty() {
            return;
        }
        let mut pos = self.caret;
        let mut keep_goal = false;
        let mut upstream = false;
        match nav {
            Nav::Left => {
                if !select && self.has_selection() {
                    pos = self.selection().0;
                } else if big {
                    pos = self.word_left(pos);
                } else if pos.offset > 0 {
                    pos.offset -= 1;
                } else if pos.para > 0 {
                    pos.para -= 1;
                    pos.offset = self.models[pos.para].char_len;
                }
            }
            Nav::Right => {
                if !select && self.has_selection() {
                    pos = self.selection().1;
                } else if big {
                    pos = self.word_right(pos);
                } else if pos.offset < self.models[pos.para].char_len {
                    pos.offset += 1;
                } else if pos.para + 1 < self.models.len() {
                    pos.para += 1;
                    pos.offset = 0;
                }
            }
            Nav::Home => {
                pos = if big {
                    DocPos::default()
                } else if let Some(ri) = self.line_ref_for(pos, self.upstream) {
                    DocPos::new(self.line(ri).paragraph, self.line_geom(ri).cs)
                } else {
                    pos
                };
            }
            Nav::End => {
                if big {
                    let last = self.models.len() - 1;
                    pos = DocPos::new(last, self.models[last].char_len);
                } else if let Some(ri) = self.line_ref_for(pos, self.upstream) {
                    let para = self.line(ri).paragraph;
                    let geom = self.line_geom(ri);
                    let wraps = ri + 1 < self.line_range_of(para).end;
                    let ends_in_space = geom.ce > geom.cs
                        && self.models[para].text.chars().nth(geom.ce - 1) == Some(' ');
                    let offset = if wraps && ends_in_space {
                        geom.ce - 1
                    } else {
                        // At the very end of a wrapped line the caret stays on that line.
                        upstream = wraps;
                        geom.ce
                    };
                    pos = DocPos::new(para, offset);
                }
            }
            Nav::Up | Nav::Down => {
                pos = self.vertical(pos, nav == Nav::Down);
                keep_goal = true;
            }
            Nav::PageUp | Nav::PageDown => {
                pos = self.page_move(pos, nav == Nav::PageDown);
                keep_goal = true;
            }
        }
        let goal = self.goal_x;
        self.caret = self.clamp(pos);
        if !select {
            self.anchor = self.caret;
        }
        self.goal_x = if keep_goal { goal } else { None };
        self.upstream = upstream;
        self.scroll_to_caret = true;
        self.pending = RunPropsPatch::default();
    }

    /// One layout line up or down, keeping the horizontal goal.
    fn vertical(&mut self, pos: DocPos, down: bool) -> DocPos {
        let Some((ri, x)) = self.x_of(pos, self.upstream) else {
            return pos;
        };
        let gx = *self.goal_x.get_or_insert(x);
        let target = if down {
            Some(ri + 1)
        } else {
            ri.checked_sub(1)
        };
        match target {
            Some(t) if t < self.index.refs.len() => self.pos_on_line(t, gx),
            _ if down => {
                let last = self.models.len() - 1;
                DocPos::new(last, self.models[last].char_len)
            }
            _ => DocPos::default(),
        }
    }

    /// About one viewport up or down.
    fn page_move(&mut self, pos: DocPos, down: bool) -> DocPos {
        let Some((ri, x)) = self.x_of(pos, self.upstream) else {
            return pos;
        };
        let gx = *self.goal_x.get_or_insert(x);
        let line = self.line(ri);
        let page = self.index.refs[ri].page as usize;
        let here = self.index.page_top[page] + line.top + line.height / 2.0;
        let step = (self.view_h * 0.9).max(line.height);
        let gy = if down { here + step } else { here - step };
        if gy < 0.0 {
            return DocPos::default();
        }
        if gy > self.index.total_height {
            let last = self.models.len() - 1;
            return DocPos::new(last, self.models[last].char_len);
        }
        let target = self
            .index
            .page_at(gy)
            .min(self.engine.layout().pages.len() - 1);
        self.hit_page(target, gx, gy - self.index.page_top[target])
    }

    // ----- the frame -----

    /// Shows the view and handles its input. Returns true while another frame is wanted.
    pub fn show(&mut self, ui: &mut Ui, target: &mut Target) -> bool {
        if self.ruler.on {
            self.show_with_rulers(ui, target)
        } else {
            self.show_canvas(ui, target)
        }
    }

    /// The scrolling canvas of pages (the whole document area when the rulers are off).
    fn show_canvas(&mut self, ui: &mut Ui, target: &mut Target) -> bool {
        let started = Instant::now();
        let ctx = ui.ctx().clone();
        self.atlas.begin_frame();
        self.ensure_layout(target);

        let zoom_delta = ctx.input(|i| i.zoom_delta());
        if (zoom_delta - 1.0).abs() > f32::EPSILON && ui.rect_contains_pointer(ui.max_rect()) {
            let current = self.zoom.unwrap_or(self.scale / POINT_SCALE);
            self.set_zoom(Some(current * zoom_delta));
        }
        let max_width = self.index.max_width.max(72.0) as f32;
        self.scale = match self.zoom {
            Some(z) => POINT_SCALE * z,
            None => {
                let avail = ui.available_size_before_wrap();
                ((avail.x - 2.0 * SIDE_PAD) / max_width).clamp(0.3, POINT_SCALE)
            }
        };
        let scale = self.scale;
        let total_h = self.index.total_height.max(PAGE_GAP_PT * 2.0 + 792.0) as f32 * scale;

        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show_viewport(ui, |ui, _viewport| {
                let width = ui.available_width().max(max_width * scale + 2.0 * SIDE_PAD);
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
                    // Focus takes effect next frame; make sure there is one.
                    ctx.request_repaint();
                }
                self.view_h = f64::from(ui.clip_rect().height() / scale);

                self.handle_pointer(&ctx, &response, canvas);
                let focused = response.has_focus() || ui.memory(|m| m.has_focus(self.doc_id));
                self.focused = focused;
                // egui may run a frame as several passes with the same input events, so
                // events are acted on in the first pass only.
                if ctx.current_pass_index() == 0 {
                    if focused {
                        self.handle_keys(&ctx, target);
                    }
                    self.handle_access_requests(&ctx, target);
                }
                // Edits above changed the model within this frame.
                self.ensure_layout(target);

                let clip = ui.clip_rect();
                let painter = ui.painter().clone();
                let paint_started = Instant::now();
                let top_pt = f64::from((clip.min.y - canvas.min.y) / scale);
                let bottom_pt = f64::from((clip.max.y - canvas.min.y) / scale);
                let (first, last) = if self.engine.layout().pages.is_empty() {
                    (0, 0)
                } else {
                    (
                        self.index.page_at(top_pt),
                        self.index
                            .page_at(bottom_pt)
                            .min(self.engine.layout().pages.len() - 1),
                    )
                };
                self.painted.clear();
                self.visible_pages = (first, last.max(first));
                if self.engine.layout().pages.is_empty() {
                    let origin = self.page_origin(canvas, 0);
                    let size = vec2(612.0, 792.0) * scale;
                    painter.rect_filled(Rect::from_min_size(origin, size), 0.0, self.page_fill);
                } else {
                    let fill = ui.visuals().selection.bg_fill;
                    for page in first..=last.max(first) {
                        self.paint_page(&ctx, &painter, canvas, page, clip, fill);
                    }
                    for page in first..=last.max(first) {
                        self.paint_pictures(ui, &painter, target, canvas, page, clip);
                    }
                }

                // Caret.
                let caret_rect = self.caret_rect(canvas);
                self.paint_caret(&ctx, &painter, caret_rect, focused);
                self.last_paint_ms = paint_started.elapsed().as_secs_f32() * 1000.0;
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

                self.publish_accessibility(ui, canvas);
                self.canvas = canvas;
            });

        self.last_frame_ms = started.elapsed().as_secs_f32() * 1000.0;
        self.dragging
    }

    /// Screen rectangle of the caret.
    fn caret_rect(&self, canvas: Rect) -> Rect {
        let Some((page, x, top, height)) = self.caret_geom(self.caret) else {
            return Rect::from_min_size(canvas.min, vec2(1.5, 16.0));
        };
        let origin = self.page_origin(canvas, page);
        let s = self.scale;
        Rect::from_min_size(
            Pos2::new(origin.x + x as f32 * s, origin.y + top as f32 * s),
            vec2(self.caret_width, (height as f32 * s).max(4.0)),
        )
    }

    /// Draws the caret in the shown half of the blink cycle. The cycle restarts on every keystroke, caret move, click and
    /// focus change; the caret is hidden while the window is inactive. Repaints are requested for the next toggle only.
    fn paint_caret(&mut self, ctx: &Context, painter: &egui::Painter, rect: Rect, focused: bool) {
        let now = ctx.input(|i| i.time);
        let active = focused && ctx.input(|i| i.focused);
        let restart = ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    Event::Key { pressed: true, .. }
                        | Event::Text(_)
                        | Event::Ime(_)
                        | Event::Paste(_)
                        | Event::PointerButton { pressed: true, .. }
                )
            })
        });
        let sig = (self.caret, self.anchor, active);
        if restart || self.blink_sig != Some(sig) {
            self.blink_epoch = now;
            self.blink_sig = Some(sig);
        }
        let (on, wake) = crate::caret::blink_phase(now - self.blink_epoch, self.blink_half);
        let wanted = active && !self.has_selection();
        self.caret_shown = wanted && on;
        if self.caret_shown {
            painter.rect_filled(rect, 0.0, self.auto_text);
        }
        if wanted && let Some(w) = wake {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(w));
        }
    }

    /// Screen position of a point in page coordinates (document points) as of the last frame.
    pub fn page_point_to_screen(&self, page: usize, x: f64, y: f64) -> Pos2 {
        let origin = self.page_origin(self.canvas, page);
        Pos2::new(
            origin.x + x as f32 * self.scale,
            origin.y + y as f32 * self.scale,
        )
    }

    /// The caret as (page, x, top, height) in document points of its page.
    pub fn caret_point(&self) -> Option<(usize, f64, f64, f64)> {
        self.caret_geom(self.caret)
    }

    /// The position nearest to a screen point as of the last frame.
    pub fn position_at(&self, screen: Pos2) -> DocPos {
        self.hit_test(self.canvas, screen)
    }

    /// Draws the pictures of `page` at their layout positions and publishes an image node for
    /// each: the body's first, then those of the header, the footnotes and the footer (the
    /// header and footer dimmed, as in Word's print layout). Pictures that cannot be decoded
    /// are left blank (a grey frame).
    fn paint_pictures(
        &mut self,
        ui: &mut Ui,
        painter: &egui::Painter,
        target: &Target,
        canvas: Rect,
        page: usize,
        clip: Rect,
    ) {
        let rev = target.revision();
        if self.pictures_rev != Some(rev) {
            self.pictures.clear();
            self.pictures_rev = Some(rev);
        }
        let origin = self.page_origin(canvas, page);
        let s = self.scale;
        let max_side = ui.ctx().input(|i| i.max_texture_side) as u32;
        let p = &self.engine.layout().pages[page];
        let groups = [
            (p.images.clone(), 1.0f32),
            (p.header_images.clone(), FURNITURE_DIM),
            (p.note_images.clone(), 1.0),
            (p.footer_images.clone(), FURNITURE_DIM),
        ];
        for (group, (images, dim)) in groups.iter().enumerate() {
            let tint = Color32::WHITE.gamma_multiply(*dim);
            for (i, im) in images.iter().enumerate() {
                let rect = Rect::from_min_size(
                    origin + vec2(im.x as f32, im.y as f32) * s,
                    vec2(im.width as f32, im.height as f32) * s,
                );
                if !rect.intersects(clip) {
                    continue;
                }
                if let Some(embed) = &im.embed {
                    let key = parts::cache_key(&im.source, embed);
                    if !self.pictures.contains_key(&key) {
                        let tex = target
                            .docx()
                            .and_then(|d| parts::image_bytes(d, &im.source, embed))
                            .and_then(|bytes| decode_picture(bytes, max_side))
                            .map(|img| {
                                Picture(ui.ctx().load_texture(
                                    format!("pic-{key}"),
                                    img,
                                    Default::default(),
                                ))
                            });
                        self.pictures.insert(key.clone(), tex);
                    }
                    if let Some(Some(tex)) = self.pictures.get(&key) {
                        painter.image(
                            tex.0.id(),
                            rect,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            tint,
                        );
                    }
                    // A picture that has not loaded draws nothing (no placeholder frame).
                }
                a11y::publish_image(
                    ui,
                    self.doc_id,
                    group * IMAGE_GROUP_STRIDE + page * 4096 + i,
                    rect,
                    im.description.as_deref(),
                );
            }
        }
    }

    /// Draws the lines of a page's header, footnote area and footer. Header and footer are
    /// dimmed, as Word does in print layout, because they are not edited here; the footnotes
    /// are drawn in full.
    fn paint_furniture_text(
        &mut self,
        ctx: &Context,
        painter: &egui::Painter,
        origin: Pos2,
        page: usize,
        clip: Rect,
    ) {
        let s = self.scale;
        let ppp = ctx.pixels_per_point();
        let p = &self.engine.layout().pages[page];
        let fonts = &self.fonts;
        let atlas = &mut self.atlas;
        let mut batch = GlyphBatch::default();
        let mut decorations: Vec<(Pos2, Pos2, Color32, f32)> = Vec::new();
        type Lines<'a> = Box<dyn Iterator<Item = &'a open365_layout::Line> + 'a>;
        let stories: [(Lines<'_>, f32); 4] = [
            (Box::new(p.header.iter()), FURNITURE_DIM),
            // Text boxes are drawn in full, like the body, but are not part of the caret model.
            (Box::new(p.frames.iter().flat_map(|f| f.lines.iter())), 1.0),
            (Box::new(p.notes.iter()), 1.0),
            (Box::new(p.footer.iter()), FURNITURE_DIM),
        ];
        for (lines, dim) in stories {
            for line in lines {
                let top = origin.y + line.top as f32 * s;
                if top > clip.max.y || top + line.height as f32 * s < clip.min.y {
                    continue;
                }
                let base_y = origin.y + line.baseline as f32 * s;
                for run in &line.runs {
                    let (Some(first_g), Some(last_g)) = (run.glyphs.first(), run.glyphs.last())
                    else {
                        continue;
                    };
                    let px = run.size as f32 * s * ppp;
                    let color = run
                        .color
                        .map_or(self.auto_text, |[r, g, b]| Color32::from_rgb(r, g, b))
                        .gamma_multiply(dim);
                    for g in &run.glyphs {
                        let at =
                            Pos2::new(origin.x + g.x as f32 * s, base_y - g.y_offset as f32 * s);
                        atlas.push(
                            ctx, fonts, &mut batch, g.face, g.glyph_id, px, at, ppp, color,
                        );
                    }
                    let x0 = origin.x + first_g.x as f32 * s;
                    let x1 = origin.x + (last_g.x + last_g.advance) as f32 * s;
                    let thickness = (run.size as f32 * 0.06 * s).max(1.0 / ppp);
                    if run.underline {
                        let y = base_y + run.size as f32 * 0.12 * s;
                        decorations.push((Pos2::new(x0, y), Pos2::new(x1, y), color, thickness));
                    }
                    if run.strike {
                        let y = base_y - run.size as f32 * 0.28 * s;
                        decorations.push((Pos2::new(x0, y), Pos2::new(x1, y), color, thickness));
                    }
                }
            }
        }
        for mesh in batch.into_meshes() {
            painter.add(Shape::mesh(mesh));
        }
        for (a, b, color, width) in decorations {
            painter.line_segment([a, b], Stroke::new(width, color));
        }
    }

    /// Cell shading and borders from the layout's cell boxes (`CellBox::fill`, `borders`).
    fn paint_cells(&self, painter: &egui::Painter, origin: Pos2, page: usize, clip: Rect) {
        let s = self.scale;
        let at = |x: f64, y: f64| origin + vec2(x as f32, y as f32) * s;
        let color = |c: [u8; 3]| Color32::from_rgb(c[0], c[1], c[2]);
        let rect_of = |x: f64, y: f64, w: f64, h: f64| {
            Rect::from_min_size(at(x, y), vec2(w as f32, h as f32) * s)
        };
        for r in &self.engine.layout().pages[page].rules {
            let rect = rect_of(r.x, r.y, r.width, r.height);
            if rect.expand(8.0 * s).intersects(clip) {
                painter.rect_filled(rect, 0.0, color(r.color));
            }
        }
        let cells = &self.engine.layout().pages[page].cells;
        for cell in cells {
            let bounds = rect_of(cell.x, cell.y, cell.width, cell.height).expand(8.0 * s);
            if !bounds.intersects(clip) {
                continue;
            }
            if let Some(fill) = cell.fill {
                painter.rect_filled(
                    rect_of(cell.x, cell.y, cell.width, cell.height),
                    0.0,
                    color(fill),
                );
            }
        }
        for cell in cells {
            let bounds = rect_of(cell.x, cell.y, cell.width, cell.height).expand(8.0 * s);
            if !bounds.intersects(clip) {
                continue;
            }
            let b = &cell.borders;
            let width = |l: &Option<BorderLine>| l.as_ref().map_or(0.0, |l| l.width);
            let (wl, wr, wt, wb) = (
                width(&b.left),
                width(&b.right),
                width(&b.top),
                width(&b.bottom),
            );
            let (x, y, w, h) = (cell.x, cell.y, cell.width, cell.height);
            // Strip rectangles in page points, corners included (as in the PDF export).
            let strips = [
                (&b.top, true, (x - wl, y - wt, w + wl + wr, wt)),
                (&b.bottom, true, (x - wl, y + h, w + wl + wr, wb)),
                (&b.left, false, (x - wl, y - wt, wl, h + wt + wb)),
                (&b.right, false, (x + w, y - wt, wr, h + wt + wb)),
            ];
            for (line, horizontal, (sx, sy, sw, sh)) in strips {
                let Some(line) = line else { continue };
                let c = color(line.color);
                match line.style {
                    BorderStyle::Solid => {
                        painter.rect_filled(rect_of(sx, sy, sw, sh), 0.0, c);
                    }
                    BorderStyle::Double => {
                        let third = if horizontal { sh / 3.0 } else { sw / 3.0 };
                        let (a, z) = if horizontal {
                            (
                                rect_of(sx, sy, sw, third),
                                rect_of(sx, sy + 2.0 * third, sw, third),
                            )
                        } else {
                            (
                                rect_of(sx, sy, third, sh),
                                rect_of(sx + 2.0 * third, sy, third, sh),
                            )
                        };
                        painter.rect_filled(a, 0.0, c);
                        painter.rect_filled(z, 0.0, c);
                    }
                    BorderStyle::Dashed | BorderStyle::Dotted => {
                        let thick = line.width as f32 * s;
                        let on = if line.style == BorderStyle::Dotted {
                            thick
                        } else {
                            thick * 3.0
                        };
                        let (p0, p1) = if horizontal {
                            (at(sx, sy + sh / 2.0), at(sx + sw, sy + sh / 2.0))
                        } else {
                            (at(sx + sw / 2.0, sy), at(sx + sw / 2.0, sy + sh))
                        };
                        painter.extend(Shape::dashed_line(
                            &[p0, p1],
                            Stroke::new(thick, c),
                            on,
                            on,
                        ));
                    }
                }
            }
        }
    }

    fn paint_page(
        &mut self,
        ctx: &Context,
        painter: &egui::Painter,
        canvas: Rect,
        page: usize,
        clip: Rect,
        sel_fill: Color32,
    ) {
        let s = self.scale;
        let ppp = ctx.pixels_per_point();
        let origin = self.page_origin(canvas, page);
        let section = self.engine.layout().pages[page].section;
        let size = vec2(section.page_width as f32, section.page_height as f32) * s;
        let rect = Rect::from_min_size(origin, size);
        match self.page_border {
            Some(border) => {
                painter.rect_filled(rect, 0.0, self.page_fill);
                painter.rect_stroke(
                    rect,
                    0.0,
                    Stroke::new(1.0, border),
                    egui::StrokeKind::Outside,
                );
            }
            None => {
                painter.rect_filled(
                    rect.translate(vec2(2.0, 3.0)),
                    0.0,
                    Color32::from_black_alpha(40),
                );
                painter.rect_filled(rect, 0.0, self.page_fill);
            }
        }
        if !rect.intersects(clip) {
            return;
        }
        // No crop marks or text boundaries: Word's defaults ("Show crop marks" and "Show text
        // boundaries" in Options > Advanced) are both off.
        self.paint_cells(painter, origin, page, clip);

        let first_ri = self.index.page_first[page];
        let line_count = self.engine.layout().pages[page].lines.len();
        let (lo, hi) = {
            let lines = &self.engine.layout().pages[page].lines;
            (
                lines.partition_point(|l| origin.y + ((l.top + l.height) as f32) * s < clip.min.y),
                lines.partition_point(|l| origin.y + (l.top as f32) * s <= clip.max.y),
            )
        };
        let hi = hi.min(line_count);

        if self.has_selection() {
            let sel = self.selection();
            for li in lo..hi {
                if let Some((x0, x1)) = self.selection_span(first_ri + li, sel) {
                    let line = &self.engine.layout().pages[page].lines[li];
                    painter.rect_filled(
                        Rect::from_min_max(
                            Pos2::new(origin.x + x0 as f32 * s, origin.y + line.top as f32 * s),
                            Pos2::new(
                                origin.x + x1 as f32 * s,
                                origin.y + (line.top + line.height) as f32 * s,
                            ),
                        ),
                        0.0,
                        sel_fill,
                    );
                }
            }
        }

        // While composing, the rest of the caret's line moves right to make room.
        let composing = self.preedit_shape();
        let shift = composing.as_ref().and_then(|(_, w, _)| {
            let (ri, x) = self.x_of(self.caret, self.upstream)?;
            Some((ri, x, *w))
        });

        let fonts = &self.fonts;
        let atlas = &mut self.atlas;
        let mut batch = GlyphBatch::default();
        let mut decorations: Vec<(Pos2, Pos2, Color32, f32)> = Vec::new();
        let mut last_para = usize::MAX;
        let mut painted = Vec::new();
        for li in lo..hi {
            let line = &self.engine.layout().pages[page].lines[li];
            if line.paragraph != last_para {
                last_para = line.paragraph;
                painted.push(last_para);
            }
            let dx = shift
                .filter(|(ri, _, _)| *ri == first_ri + li)
                .map(|(_, x, w)| (x, w));
            let base_y = origin.y + line.baseline as f32 * s;
            for run in &line.runs {
                let (Some(first_g), Some(last_g)) = (run.glyphs.first(), run.glyphs.last()) else {
                    continue;
                };
                let px = run.size as f32 * s * ppp;
                let color = run
                    .color
                    .map_or(self.auto_text, |[r, g, b]| Color32::from_rgb(r, g, b));
                let move_x = |g: &PlacedGlyph| match dx {
                    Some((x, w)) if g.x >= x - 1e-6 => g.x + w,
                    _ => g.x,
                };
                for g in &run.glyphs {
                    let at = Pos2::new(
                        origin.x + move_x(g) as f32 * s,
                        base_y - g.y_offset as f32 * s,
                    );
                    atlas.push(
                        ctx, fonts, &mut batch, g.face, g.glyph_id, px, at, ppp, color,
                    );
                }
                if run.underline || run.strike {
                    let x0 = origin.x + move_x(first_g) as f32 * s;
                    let x1 = origin.x + (move_x(last_g) + last_g.advance) as f32 * s;
                    let thickness = (run.size as f32 * 0.06 * s).max(1.0 / ppp);
                    if run.underline {
                        let y = base_y + run.size as f32 * 0.12 * s;
                        decorations.push((Pos2::new(x0, y), Pos2::new(x1, y), color, thickness));
                    }
                    if run.strike {
                        let y = base_y - run.size as f32 * 0.28 * s;
                        decorations.push((Pos2::new(x0, y), Pos2::new(x1, y), color, thickness));
                    }
                }
            }
            // The IME preedit text, underlined, in the gap that was made for it.
            if let (Some((glyphs, width, size)), Some((x, _))) = (&composing, dx) {
                let color = self.auto_text;
                let px = *size as f32 * s * ppp;
                for (glyph, face, off) in glyphs {
                    let at = Pos2::new(origin.x + (x + off) as f32 * s, base_y);
                    atlas.push(ctx, fonts, &mut batch, *face, *glyph, px, at, ppp, color);
                }
                let y = base_y + *size as f32 * 0.12 * s;
                decorations.push((
                    Pos2::new(origin.x + x as f32 * s, y),
                    Pos2::new(origin.x + (x + width) as f32 * s, y),
                    color,
                    1.0,
                ));
            }
        }
        for mesh in batch.into_meshes() {
            painter.add(Shape::mesh(mesh));
        }
        for (a, b, color, width) in decorations {
            painter.line_segment([a, b], Stroke::new(width, color));
        }
        self.painted.extend(painted);
        self.paint_furniture_text(ctx, painter, origin, page, clip);
    }

    fn handle_pointer(&mut self, ctx: &Context, response: &egui::Response, canvas: Rect) {
        let pos = ctx.input(|i| i.pointer.interact_pos());
        let shift = ctx.input(|i| i.modifiers.shift);
        if response.drag_started() {
            if let Some(origin) = ctx.input(|i| i.pointer.press_origin()) {
                let hit = self.hit_test(canvas, origin);
                if shift {
                    self.caret = hit;
                    self.upstream = false;
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
            self.caret = self.hit_test(canvas, pos);
            self.upstream = false;
            self.preedit = None;
        }
        if response.drag_stopped() {
            self.dragging = false;
        }
        if response.double_clicked()
            && let Some(pos) = pos
        {
            let hit = self.hit_test(canvas, pos);
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
            let hit = self.hit_test(canvas, pos);
            if shift {
                self.caret = hit;
                self.upstream = false;
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
                    }
                    egui::ImeEvent::Commit(text) => {
                        self.preedit = None;
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
                    self.ensure_layout(target);
                    let composing = self.preedit.is_some();
                    match self::keys::classify(key, modifiers) {
                        Some(KeyAction::Move { nav, select, big }) => {
                            self.move_caret(nav, select, big);
                        }
                        Some(KeyAction::SelectAll) => {
                            if let Some(last) = self.models.len().checked_sub(1) {
                                let end = DocPos::new(last, self.models[last].char_len);
                                self.select(DocPos::default(), end);
                            }
                        }
                        Some(KeyAction::Backspace { word }) if !composing => {
                            self.backspace(target, word, ctx);
                        }
                        Some(KeyAction::Delete { word }) if !composing => {
                            self.delete_forward(target, word);
                        }
                        Some(KeyAction::Enter { soft }) if !composing => self.enter(target, soft),
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

    /// The AccessKit position of a document position: the run of its layout line.
    fn access_position(&self, pos: DocPos) -> accesskit::TextPosition {
        let upstream = self.upstream && pos == self.caret;
        let range = self.line_range_of(pos.para);
        let (line, column) = match self.line_ref_for(pos, upstream) {
            Some(ri) if !range.is_empty() => {
                let geom = self.line_geom(ri);
                (
                    ri - range.start,
                    pos.offset
                        .saturating_sub(geom.cs)
                        .min(geom.ce - geom.cs + 1),
                )
            }
            _ => (0, pos.offset),
        };
        a11y::text_position(self.doc_id, pos.para, line, column)
    }

    fn publish_accessibility(&mut self, ui: &mut Ui, canvas: Rect) {
        let ctx = ui.ctx().clone();
        if !a11y::is_enabled(&ctx, self.doc_id) {
            return;
        }
        let viewport = ui.clip_rect();
        let doc_id = self.doc_id;
        // Always expose the paragraphs holding the caret and selection ends so the text
        // selection resolves to published runs even when they are off screen.
        let count = self.models.len();
        let mut paras: Vec<usize> = self
            .painted
            .iter()
            .copied()
            .filter(|p| *p < count)
            .collect();
        for p in [self.caret.para, self.anchor.para] {
            if !paras.contains(&p) && p < count {
                paras.push(p);
            }
        }
        paras.sort_unstable();
        paras.dedup();

        let mut run_ui = ui.new_child(
            UiBuilder::new()
                .id_salt("doc-a11y")
                .max_rect(viewport)
                .accessibility_parent(doc_id),
        );
        let mut entries = Vec::new();
        for para in paras {
            let lines = self.a11y_lines(para, canvas);
            let label = self.engine.list_label(para).map(list_label_text);
            let range = self.line_range_of(para);
            let role = if range.is_empty() {
                ParaRole::Plain
            } else {
                self.line(range.start).role
            };
            entries.extend(a11y::publish_paragraph(
                &mut run_ui,
                doc_id,
                para,
                &lines,
                label.as_deref(),
                role,
            ));
        }
        self.runs = entries;
        self.publish_furniture_regions(&mut run_ui, canvas);

        let selection = (!self.models.is_empty()).then(|| accesskit::TextSelection {
            anchor: self.access_position(self.anchor),
            focus: self.access_position(self.caret),
        });
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
    }

    /// Header, footnote and footer lines of the visible pages as read-only regions.
    fn publish_furniture_regions(&self, ui: &mut Ui, canvas: Rect) {
        let layout = self.engine.layout();
        let (first, last) = self.visible_pages;
        let s = f64::from(self.scale);
        for page in first..=last {
            let Some(p) = layout.pages.get(page) else {
                continue;
            };
            let origin = self.page_origin(canvas, page);
            let line_data = |lines: &[Line]| -> Vec<(Rect, String)> {
                lines
                    .iter()
                    .filter_map(|l| {
                        let text = line_text(l);
                        if text.trim().is_empty() {
                            return None;
                        }
                        let x1 = l
                            .runs
                            .iter()
                            .flat_map(|r| &r.glyphs)
                            .fold(l.left, |m, g| m.max(g.x + g.advance));
                        let min = Pos2::new(
                            origin.x + (l.left * s) as f32,
                            origin.y + (l.top * s) as f32,
                        );
                        let max = Pos2::new(
                            origin.x + (x1 * s) as f32,
                            origin.y + ((l.top + l.height) * s) as f32,
                        );
                        Some((Rect::from_min_max(min, max), text))
                    })
                    .collect()
            };
            for (lines, tag, role, label) in [
                (&p.header, "header", accesskit::Role::Header, "Page header"),
                (&p.notes, "notes", accesskit::Role::Group, "Footnotes"),
                (&p.footer, "footer", accesskit::Role::Footer, "Page footer"),
            ] {
                let data = line_data(lines);
                if !data.is_empty() {
                    a11y::publish_region(ui, self.doc_id, (page, tag), role, label, &data);
                }
            }
        }
    }

    /// Text of a paragraph as laid out (for tests and the status bar).
    pub fn paragraph_text(&self, para: usize) -> Option<&str> {
        self.models.get(para).map(|m| m.text.as_str())
    }
}

/// The text a laid-out line shows, from its glyph clusters (list label included).
fn line_text(line: &Line) -> String {
    line.runs
        .iter()
        .flat_map(|r| &r.glyphs)
        .map(|g| line.cluster_text(g))
        .collect::<String>()
        .replace(['\u{2003}', '\r', '\n'], " ")
}

/// A list label for a screen reader: bullets are private-use symbol characters, spoken as a
/// bullet.
fn list_label_text(label: &str) -> String {
    label
        .chars()
        .map(|c| {
            if ('\u{e000}'..='\u{f8ff}').contains(&c) {
                '\u{2022}'
            } else {
                c
            }
        })
        .collect()
}

/// A decoded picture on the GPU.
struct Picture(egui::TextureHandle);

impl std::fmt::Debug for Picture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Picture")
    }
}

/// Largest picture side decoded, in pixels, and the most memory a decode may take.
const MAX_PICTURE_SIDE: u32 = 8192;
const MAX_PICTURE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PICTURE_INPUT: usize = 64 * 1024 * 1024;

/// Decodes a PNG, JPEG, GIF or BMP picture from untrusted bytes, scaled down to fit a GPU
/// texture of `max_side` pixels. Oversized or malformed input gives `None`; nothing here panics.
fn decode_picture(bytes: &[u8], max_side: u32) -> Option<egui::ColorImage> {
    if bytes.len() > MAX_PICTURE_INPUT {
        return None;
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_PICTURE_SIDE);
    limits.max_image_height = Some(MAX_PICTURE_SIDE);
    limits.max_alloc = Some(MAX_PICTURE_BYTES);
    reader.limits(limits);
    let mut rgba = reader.decode().ok()?.to_rgba8();
    let longest = rgba.width().max(rgba.height());
    if max_side > 0 && longest > max_side {
        let k = f64::from(max_side) / f64::from(longest);
        let w = ((f64::from(rgba.width()) * k) as u32).max(1);
        let h = ((f64::from(rgba.height()) * k) as u32).max(1);
        rgba = image::imageops::resize(&rgba, w, h, image::imageops::FilterType::Triangle);
    }
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(
        size,
        rgba.as_raw(),
    ))
}

/// `RRGGBB` (or `auto`) to a colour; `None` for automatic or malformed input.
fn parse_hex_color(v: &str) -> Option<[u8; 3]> {
    if v.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(v, 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

#[cfg(test)]
mod page_geometry {
    use super::{PAGE_GAP_PT, PAGE_TOP_PT, POINT_SCALE};

    /// Word at 100 % on the reference machine (125 % scale): the first page starts 21 px under the canvas top and
    /// consecutive pages are 1421 px apart (A4 page 841.9 pt = 1403.2 px), within 1 px.
    #[test]
    fn page_top_and_pitch_match_word() {
        let px = |pt: f64| pt * f64::from(POINT_SCALE) * 1.25;
        assert!(
            (px(PAGE_TOP_PT) - 21.0).abs() <= 1.0,
            "top {}",
            px(PAGE_TOP_PT)
        );
        let pitch = px(841.9 + PAGE_GAP_PT);
        assert!((pitch - 1421.0).abs() <= 1.0, "pitch {pitch}");
    }
}
