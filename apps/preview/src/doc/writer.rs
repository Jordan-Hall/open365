//! The Writer document pane: a paged, editable view of a DOCX on the own UI platform.
//!
//! The model is the one the egui Writer uses and nothing else: `open365-docx` through the
//! `open365-commands` [`Target`] (every edit is a [`Command`], so undo, redo and the audit log
//! come with it), `open365-layout`'s incremental Word-parity pagination for pages, lines and glyph
//! positions, and `open365-text` for fonts. This file adds only what a pane needs: the caret and
//! selection, hit testing, scrolling, the clipboard seam, IME composition, painting through
//! `open365-gfx` and the AccessKit text subtree.
//!
//! Geometry is kept in canvas-local device pixels (origin at the top-left of the pane); a document
//! point is `scale * zoom * 96 / 72` device pixels.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use accesskit::{Action, Node as AkNode, NodeId as AkId, Role, TextPosition, TextSelection};
use open365_commands::{Command, Outcome, Target};
use open365_docx::{DocPos, RunPropsPatch, VertAlign};
use open365_gfx::{Color, DrawList, FontId, GlyphPos, GlyphRun, Rect as GRect};
use open365_layout::{ClusterKind, IncrementalLayout, LayoutOptions, Line, ParaRole};
use open365_text::{FaceId, FontLibrary, FontStyle, ShapeFeatures, shape_run};
use open365_ui_core::access::ak_id;
use open365_ui_core::{
    Clipboard, Emit, ImeInput, Key, MemoryClipboard, Modifiers, Node, NodeId, Rect, Ui, Widget,
};

mod cells;
mod ext_design;
mod ext_file;
mod ext_home;
mod ext_insert;
mod ext_layout;
mod ext_references;
mod ext_review;
mod ext_view;
mod findbar;
mod pictures;
mod replacebar;
mod rulerbar;

use super::docmodel::{ParaModel, build_models};
use crate::layout::PxRect;
use crate::palette::Palette;

/// Gap between pages in document points (Word at 100 %: 18 px at 125 % scale).
const PAGE_GAP_PT: f64 = 10.8;
/// Space above the first page in document points.
const PAGE_TOP_PT: f64 = 12.6;
/// Smallest margin left of a page that is wider than the pane, logical pixels.
const SIDE_PAD: f32 = 16.0;
/// Screen pixels per document point at 100 % zoom (96 dpi).
const POINT_SCALE: f32 = 96.0 / 72.0;
/// Width of the highlight past the end of a selected paragraph, document points.
const MARK_WIDTH_PT: f64 = 4.0;
/// Opacity of header and footer text while the body is edited.
const FURNITURE_DIM: f32 = 0.45;
/// Half period of the caret blink.
const BLINK: Duration = Duration::from_millis(530);
/// AccessKit stores word starts as `u8`, so one text run holds at most this many characters.
const MAX_CHARS_PER_RUN: usize = 255;

/// Which character toggle a shortcut flips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    Bold,
    Italic,
    Underline,
    Strike,
    /// Subscript (Ctrl+=).
    Sub,
    /// Superscript (Ctrl+Shift+=).
    Sup,
}

impl Toggle {
    /// The vertical alignment of Sub and Sup.
    fn vert(self) -> Option<VertAlign> {
        match self {
            Self::Sub => Some(VertAlign::Subscript),
            Self::Sup => Some(VertAlign::Superscript),
            _ => None,
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
            Self::Sub | Self::Sup => RunPropsPatch::vert_align(match (self.vert(), on) {
                (Some(v), true) => v,
                _ => VertAlign::Baseline,
            }),
        }
    }

    fn test(self, s: &super::docmodel::SpanStyle) -> bool {
        match self {
            Self::Bold => s.bold,
            Self::Italic => s.italic,
            Self::Underline => s.underline,
            Self::Strike => s.strike,
            Self::Sub | Self::Sup => false,
        }
    }
}

/// A formatting change from the ribbon, applied through the command layer.
#[derive(Debug, Clone, PartialEq)]
pub enum Format {
    Toggle(Toggle),
    /// Font size in half points.
    Size(u32),
    /// Grow (+) or shrink (-) the size by this many half points.
    SizeStep(i32),
    Font(String),
    Align(open365_docx::Alignment),
    /// Paragraph style id (`Heading1`, ...).
    Style(String),
}

impl Format {
    /// The change a ribbon control id (`home.font.bold`, `home.font.font-size#3`,
    /// `home.font.font=Arial`) stands for, if it is a formatting command.
    pub fn from_ribbon(id: &str) -> Option<Self> {
        use open365_docx::Alignment as A;
        if let Some((head, text)) = id.split_once('=') {
            let text = text.trim();
            return match head {
                "home.styles.style" => {
                    Self::style_by_name(text).map(|id| Self::Style(id.to_owned()))
                }
                "home.font.font" if !text.is_empty() => Some(Self::Font(text.to_owned())),
                "home.font.font-size" => text
                    .parse::<f32>()
                    .ok()
                    .filter(|v| (1.0..=1638.0).contains(v))
                    .map(|v| Self::Size((v * 2.0).round() as u32)),
                _ => None,
            };
        }
        if let Some((head, i)) = id.split_once('#') {
            let i: usize = i.parse().ok()?;
            let items = crate::menus::combo_items(head);
            let text = items.get(i)?;
            return Self::from_ribbon(&format!("{head}={text}"));
        }
        if let Some(name) = id.strip_prefix("home.styles.") {
            return Self::style_by_name(name).map(|id| Self::Style(id.to_owned()));
        }
        Some(match id {
            "home.font.bold" => Self::Toggle(Toggle::Bold),
            "home.font.italic" => Self::Toggle(Toggle::Italic),
            "home.font.underline" => Self::Toggle(Toggle::Underline),
            "home.font.strikethrough" => Self::Toggle(Toggle::Strike),
            "home.font.subscript" => Self::Toggle(Toggle::Sub),
            "home.font.superscript" => Self::Toggle(Toggle::Sup),
            "home.font.grow-font" => Self::SizeStep(2),
            "home.font.shrink-font" => Self::SizeStep(-2),
            "home.paragraph.align-left" => Self::Align(A::Left),
            "home.paragraph.centre" => Self::Align(A::Center),
            "home.paragraph.align-right" => Self::Align(A::Right),
            "home.paragraph.justify" => Self::Align(A::Justify),
            _ => return None,
        })
    }
}

impl Format {
    /// The style id for a style's display name or a ribbon slug (`Heading 1`, `heading-1`,
    /// `normal`, `title`, ...). Only the built-in names; a document's own styles are applied by
    /// id with [`Format::Style`].
    pub fn style_by_name(name: &str) -> Option<&'static str> {
        let key: String = name
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        Some(match key.as_str() {
            "normal" => "Normal",
            "title" => "Title",
            "subtitle" => "Subtitle",
            "heading1" => "Heading1",
            "heading2" => "Heading2",
            "heading3" => "Heading3",
            "heading4" => "Heading4",
            "nospacing" => "NoSpacing",
            "quote" => "Quote",
            "listparagraph" => "ListParagraph",
            _ => return None,
        })
    }

    /// Word's style shortcuts with Ctrl+Alt: 1, 2, 3 for Heading 1 to 3 (Ctrl+Shift+N is Normal).
    pub fn style_for_shortcut(c: char) -> Option<&'static str> {
        match c {
            '1' => Some("Heading1"),
            '2' => Some("Heading2"),
            '3' => Some("Heading3"),
            _ => None,
        }
    }
}

/// How far a run is raised above its line's baseline, in points: a note reference mark is the
/// base size scaled to 65 % and raised by a third of the base size, as Word does. (Layout does
/// not mark user-formatted superscript or subscript runs; they would join here.)
pub fn run_raise(run: &open365_layout::GlyphRun) -> f64 {
    if run.superscript {
        run.size / 0.65 * 0.33
    } else {
        0.0
    }
}

/// A caret movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

#[derive(Debug, Clone, Copy)]
struct LineRef {
    page: u32,
    line: u32,
}

/// Lookup tables over a layout: lines in reading order, which lines belong to a paragraph and
/// where each page sits in the scrolling canvas (document points).
#[derive(Debug, Default)]
struct LayoutIndex {
    refs: Vec<LineRef>,
    page_first: Vec<usize>,
    para_first: Vec<usize>,
    para_story: Vec<usize>,
    page_top: Vec<f64>,
    total_height: f64,
}

impl LayoutIndex {
    fn build(layout: &open365_layout::Layout, paras: usize) -> Self {
        let mut refs = Vec::new();
        let mut page_first = Vec::new();
        let mut page_top = Vec::new();
        let mut y = PAGE_TOP_PT;
        for (pi, page) in layout.pages.iter().enumerate() {
            page_first.push(refs.len());
            page_top.push(y);
            y += page.section.page_height + PAGE_GAP_PT;
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
        }
    }

    fn page_at(&self, y: f64) -> usize {
        self.page_top
            .partition_point(|t| *t <= y + PAGE_GAP_PT / 2.0)
            .saturating_sub(1)
    }
}

/// Character boundaries of one layout line in page coordinates.
#[derive(Debug, Clone)]
struct LineGeom {
    cs: usize,
    ce: usize,
    /// X (points) of the left edge of every character, plus the right edge of the last.
    xs: Vec<f64>,
}

/// What the last paint drew, summed over the frames since [`WriterPane::reset_stats`]. The
/// headless tests read it instead of pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PaintStats {
    /// Table cell fills.
    pub cell_fills: usize,
    /// Cell border strips (each side of each cell is one).
    pub border_strips: usize,
    /// Paragraph border rules.
    pub rules: usize,
    /// Pictures drawn.
    pub pictures: usize,
    /// Of those, floating pictures.
    pub floating_pictures: usize,
    /// Runs drawn raised or lowered (note references, superscript).
    pub raised_runs: usize,
    /// Markers and tab stops on the ruler.
    pub ruler_items: usize,
    /// Margin edges and table rows on the vertical ruler.
    pub vruler_items: usize,
    /// Highlighted find matches.
    pub find_marks: usize,
}

/// What the pane publishes to AccessKit: the canvas node as a multi-line text input with the
/// selection, and its paragraph and text run descendants.
#[derive(Debug, Clone)]
pub struct CanvasAccess {
    /// The replacement for the canvas node (role, children, actions, text selection).
    pub node: AkNode,
    /// The paragraphs and runs under it.
    pub nodes: Vec<(AkId, AkNode)>,
}

/// How a pane asks for a save path when the document has none.
pub type PathChooser = fn() -> Option<PathBuf>;

fn choose_save_path() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Word document", &["docx"])
        .set_title("Save As")
        .save_file()
}

/// The editable document pane.
pub struct WriterPane {
    target: Target,
    models: Vec<ParaModel>,
    revision: Option<u64>,
    fonts: FontLibrary,
    system_fonts: bool,
    gfx_fonts: HashMap<usize, FontId>,
    engine: IncrementalLayout,
    index: LayoutIndex,
    caret: DocPos,
    anchor: DocPos,
    /// The caret sits at the end of a wrapped line rather than the start of the next.
    upstream: bool,
    goal_x: Option<f64>,
    pending: RunPropsPatch,
    preedit: Option<String>,
    /// Pane rectangle in device pixels, the UI scale and the zoom (1.0 is 100 %).
    area: PxRect,
    scale: f32,
    zoom: f32,
    /// Document y (points) at the top of the pane.
    scroll: f64,
    canvas: Option<NodeId>,
    dragging: bool,
    /// The vertical scroll bar's thumb is being dragged.
    bar_drag: bool,
    blink_epoch: Instant,
    clipboard: Box<dyn Clipboard>,
    chooser: PathChooser,
    /// Message for the status bar (the last failed command, the last save).
    pub status: Option<String>,
    pictures: pictures::Cache,
    ruler: rulerbar::RulerState,
    find: findbar::FindState,
    replace: replacebar::ReplaceState,
    stats: PaintStats,
    /// Horizontal scroll in device pixels (a page wider than the pane).
    scroll_x: f32,
    /// The zoom changed through the pane (Ctrl+wheel); the host reads it with
    /// [`WriterPane::take_zoom_change`].
    zoom_changed: bool,
}

impl std::fmt::Debug for WriterPane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriterPane")
            .field("paragraphs", &self.models.len())
            .field("caret", &self.caret)
            .finish_non_exhaustive()
    }
}

impl WriterPane {
    /// A pane over an open document.
    pub fn new(target: Target) -> Self {
        let mut p = Self {
            target,
            models: Vec::new(),
            revision: None,
            fonts: FontLibrary::bundled(),
            system_fonts: false,
            gfx_fonts: HashMap::new(),
            engine: IncrementalLayout::new(),
            index: LayoutIndex::default(),
            caret: DocPos::default(),
            anchor: DocPos::default(),
            upstream: false,
            goal_x: None,
            pending: RunPropsPatch::default(),
            preedit: None,
            area: PxRect {
                x: 0,
                y: 0,
                w: 1000,
                h: 700,
            },
            scale: 1.0,
            zoom: 1.0,
            scroll: 0.0,
            canvas: None,
            dragging: false,
            bar_drag: false,
            blink_epoch: Instant::now(),
            clipboard: Box::new(MemoryClipboard::default()),
            chooser: choose_save_path,
            status: None,
            pictures: pictures::Cache::default(),
            ruler: rulerbar::RulerState::default(),
            find: findbar::FindState::default(),
            replace: replacebar::ReplaceState::default(),
            stats: PaintStats::default(),
            scroll_x: 0.0,
            zoom_changed: false,
        };
        p.sync();
        p
    }

    /// A new blank document.
    pub fn blank() -> Self {
        let target = Target::open_docx(&super::blank::docx()).expect("the blank document opens");
        Self::new(target)
    }

    /// Opens a `.docx` from disk.
    ///
    /// # Errors
    /// The file cannot be read or is not a Word document.
    pub fn open(path: &Path) -> Result<Self, String> {
        Target::open_path(path)
            .map(Self::new)
            .map_err(|e| e.to_string())
    }

    /// Uses the Word fonts installed on this machine before the bundled stand-ins, so lines
    /// break where Word breaks them. Slow (it reads font files); the tests keep the bundled set.
    pub fn use_system_fonts(&mut self) {
        if self.system_fonts {
            return;
        }
        self.system_fonts = true;
        for family in [
            "Calibri",
            "Calibri Light",
            "Cambria",
            "Aptos",
            "Arial",
            "Times New Roman",
            "Segoe UI",
            "Symbol",
        ] {
            self.fonts.add_system_family(family);
        }
        self.gfx_fonts.clear();
        self.engine.invalidate();
        self.revision = None;
        self.sync();
    }

    /// Replaces the clipboard (the host passes the system one; tests a memory one).
    pub fn set_clipboard(&mut self, c: Box<dyn Clipboard>) {
        self.clipboard = c;
    }

    /// Replaces the Save As dialog.
    pub fn set_path_chooser(&mut self, c: PathChooser) {
        self.chooser = c;
    }

    // ---- state queries ----------------------------------------------------------------------

    pub fn caret(&self) -> DocPos {
        self.caret
    }

    pub fn anchor(&self) -> DocPos {
        self.anchor
    }

    /// The selection as (start, end), ordered.
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

    pub fn paragraph_text(&self, para: usize) -> Option<&str> {
        self.models.get(para).map(|m| m.text.as_str())
    }

    /// The whole text, paragraphs joined by `\n`.
    pub fn text(&self) -> String {
        self.models
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn page_count(&self) -> usize {
        self.engine.layout().pages.len()
    }

    pub fn is_dirty(&self) -> bool {
        self.target.is_dirty()
    }

    pub fn path(&self) -> Option<&Path> {
        self.target.path()
    }

    pub fn can_undo(&self) -> bool {
        self.target.can_undo()
    }

    /// Whether Redo has a change to bring back (the command layer's redo stack).
    pub fn can_redo(&self) -> bool {
        self.target.can_redo()
    }

    /// The IME composition text shown at the caret, if any.
    pub fn preedit(&self) -> Option<&str> {
        self.preedit.as_deref()
    }

    pub fn scroll(&self) -> f64 {
        self.scroll
    }

    pub fn canvas(&self) -> Option<NodeId> {
        self.canvas
    }

    /// File name for the title bar.
    pub fn title(&self) -> String {
        let name = self.target.path().and_then(Path::file_name).map_or_else(
            || "Document1".to_owned(),
            |n| n.to_string_lossy().into_owned(),
        );
        format!("{name} - Word")
    }

    // ---- model and layout sync ---------------------------------------------------------------

    /// Rebuilds the paragraph models and the layout after the document changed.
    fn sync(&mut self) {
        if self.revision == Some(self.target.revision()) {
            return;
        }
        let Some(doc) = self.target.docx() else {
            self.models.clear();
            return;
        };
        self.models = build_models(doc);
        self.engine
            .update(doc, &self.fonts, LayoutOptions::default());
        self.index = LayoutIndex::build(self.engine.layout(), self.models.len());
        self.revision = Some(self.target.revision());
        self.caret = self.clamp(self.caret);
        self.anchor = self.clamp(self.anchor);
        self.find_sync();
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

    fn touch(&mut self) {
        self.blink_epoch = Instant::now();
    }

    /// Runs a `share_*` command: it never changes the document, its history or its saved state.
    pub fn apply_share(&mut self, command: Command) -> Result<Outcome, String> {
        if !command.is_share() {
            return Err("not a share command".to_owned());
        }
        self.target.apply(command).map_err(|e| e.to_string())
    }

    fn run(&mut self, command: Command) -> Option<Outcome> {
        match self.target.apply(command) {
            Ok(o) => {
                self.sync();
                Some(o)
            }
            Err(e) => {
                self.status = Some(e.to_string());
                None
            }
        }
    }

    // ---- geometry ----------------------------------------------------------------------------

    /// Device pixels per document point.
    fn s(&self) -> f32 {
        self.scale * self.zoom * POINT_SCALE
    }

    fn view_h_pt(&self) -> f64 {
        f64::from((self.area.h as f32 - self.top_inset()).max(1.0)) / f64::from(self.s())
    }

    /// A page rectangle in canvas-local device pixels: (x, y, width, height).
    pub fn page_rect(&self, page: usize) -> Option<(f32, f32, f32, f32)> {
        let sec = self.engine.layout().pages.get(page)?.section;
        let s = self.s();
        Some((
            self.page_x(page),
            self.page_y(page),
            sec.page_width as f32 * s,
            sec.page_height as f32 * s,
        ))
    }

    /// The caret rectangle of `pos` in canvas-local device pixels: (x, y, height).
    pub fn point_of(&self, pos: DocPos) -> Option<(f32, f32, f32)> {
        let upstream = self.upstream && pos == self.caret;
        let (ri, x) = self.x_of(pos, upstream)?;
        let page = self.index.refs[ri].page as usize;
        let line = self.line(ri);
        let s = self.s();
        Some((
            self.page_x(page) + x as f32 * s,
            self.page_y(page) + line.top as f32 * s,
            line.height as f32 * s,
        ))
    }

    /// Positions the pane: its device rectangle, the UI scale and the zoom (1.0 is 100 %).
    pub fn set_view(&mut self, area: PxRect, scale: f32, zoom: f32) {
        self.area = area;
        self.scale = scale.max(0.1);
        self.zoom = zoom.max(0.1);
        self.clamp_scroll();
    }

    fn clamp_scroll(&mut self) {
        let max = (self.index.total_height - self.view_h_pt()).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max);
        let widest = self
            .engine
            .layout()
            .pages
            .iter()
            .map(|p| p.section.page_width)
            .fold(0.0f64, f64::max) as f32
            * self.s();
        let over = (widest + 2.0 * SIDE_PAD * self.scale + self.left_inset() - self.area.w as f32)
            .max(0.0);
        self.scroll_x = self.scroll_x.clamp(0.0, over);
    }

    // ---- zoom --------------------------------------------------------------------------------

    /// The zoom factor (1.0 is 100 %).
    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// The zoom in percent, as the status bar shows it.
    pub fn zoom_percent(&self) -> f32 {
        (self.zoom * 100.0).round()
    }

    /// Sets the zoom (the status-bar slider): 10 % to 500 %, the top of the view stays put.
    pub fn set_zoom(&mut self, factor: f32) {
        self.zoom_about(factor, self.top_inset());
    }

    /// Sets the zoom in percent.
    pub fn set_zoom_percent(&mut self, percent: f32) {
        self.set_zoom(percent / 100.0);
    }

    /// Sets the zoom keeping the document point under canvas-local device `y` where it is.
    pub fn zoom_about(&mut self, factor: f32, y: f32) {
        let factor = if factor.is_finite() {
            factor.clamp(0.1, 5.0)
        } else {
            1.0
        };
        if (factor - self.zoom).abs() < 1e-6 {
            return;
        }
        let inset = self.top_inset();
        let y = y.max(inset);
        let anchor = f64::from(y - inset) / f64::from(self.s()) + self.scroll;
        self.zoom = factor;
        self.scroll = anchor - f64::from(y - inset) / f64::from(self.s());
        self.clamp_scroll();
        self.zoom_changed = true;
    }

    /// Zooms by `steps` notches of 10 % about `y` (Ctrl+wheel).
    pub fn zoom_step(&mut self, steps: f32, y: f32) {
        let percent = (self.zoom * 100.0 + steps * 10.0).round();
        self.zoom_about(percent / 100.0, y);
    }

    /// The new zoom in percent when the pane changed it since the last call; the host moves the
    /// status-bar slider and passes the same zoom back to [`Self::build`].
    pub fn take_zoom_change(&mut self) -> Option<f32> {
        std::mem::take(&mut self.zoom_changed).then(|| self.zoom_percent())
    }

    /// Width of the vertical scroll bar in device pixels (17 px at 100 % in Word).
    fn bar_w(&self) -> f32 {
        (17.0 * self.scale / 1.25).round()
    }

    /// The scroll thumb in canvas-local device pixels: (top, height), when the document is
    /// taller than the pane.
    pub fn thumb(&self) -> Option<(f32, f32)> {
        let view = self.view_h_pt();
        let total = self.index.total_height;
        if total <= view + 0.5 {
            return None;
        }
        let track = self.area.h as f32;
        let h = ((view / total) as f32 * track)
            .max(self.bar_w() * 1.5)
            .min(track);
        let max = total - view;
        let top = (self.scroll / max) as f32 * (track - h);
        Some((top, h))
    }

    /// Whether a canvas-local point lies on the scroll bar.
    pub fn on_bar(&self, x: f32) -> bool {
        self.thumb().is_some() && x >= self.area.w as f32 - self.bar_w()
    }

    /// Moves the thumb's centre to canvas-local `y` and scrolls to match.
    fn scroll_to_bar(&mut self, y: f32) {
        let Some((_, h)) = self.thumb() else { return };
        let track = (self.area.h as f32 - h).max(1.0);
        let t = ((y - h / 2.0) / track).clamp(0.0, 1.0);
        let max = (self.index.total_height - self.view_h_pt()).max(0.0);
        self.scroll = f64::from(t) * max;
        self.clamp_scroll();
    }

    /// Scrolls by `pt` document points (positive: down).
    pub fn scroll_by(&mut self, pt: f64) {
        self.scroll += pt;
        self.clamp_scroll();
    }

    /// Left edge of `page` in canvas-local device pixels.
    fn page_x(&self, page: usize) -> f32 {
        let width = self
            .engine
            .layout()
            .pages
            .get(page)
            .map_or(612.0, |p| p.section.page_width) as f32
            * self.s();
        let left = self.left_inset();
        ((self.area.w as f32 - left - width) / 2.0).max(SIDE_PAD * self.scale) + left
            - self.scroll_x
    }

    /// Top of `page` in canvas-local device pixels.
    fn page_y(&self, page: usize) -> f32 {
        let top = self
            .index
            .page_top
            .get(page)
            .copied()
            .unwrap_or(PAGE_TOP_PT);
        ((top - self.scroll) * f64::from(self.s())) as f32 + self.top_inset()
    }

    /// The position nearest to canvas-local device point (`x`, `y`).
    pub fn hit(&self, x: f32, y: f32) -> DocPos {
        let pages = self.engine.layout().pages.len();
        if self.models.is_empty() || pages == 0 {
            return DocPos::default();
        }
        let s = f64::from(self.s());
        let gy = f64::from(y - self.top_inset()) / s + self.scroll;
        let page = self.index.page_at(gy).min(pages - 1);
        let px = f64::from(x - self.page_x(page)) / s;
        self.hit_page(page, px, gy - self.index.page_top[page])
    }

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

    /// Where each character of a line starts, from the layout's cluster map.
    fn line_geom(&self, ri: usize) -> LineGeom {
        let line = self.line(ri);
        let para = line.paragraph.min(self.models.len().saturating_sub(1));
        let (cs, ce) = self.char_range(para, line);
        let chars: Vec<char> = self
            .models
            .get(para)
            .map(|m| m.text.chars().skip(cs).take(ce - cs).collect())
            .unwrap_or_default();
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

    fn x_of(&self, pos: DocPos, upstream: bool) -> Option<(usize, f64)> {
        let ri = self.line_ref_for(pos, upstream)?;
        let geom = self.line_geom(ri);
        let i = pos.offset.saturating_sub(geom.cs).min(geom.ce - geom.cs);
        Some((ri, geom.xs[i]))
    }

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

    fn hit_page(&self, page: usize, x: f64, y: f64) -> DocPos {
        let Some(p) = self.engine.layout().pages.get(page) else {
            return DocPos::default();
        };
        if self.index.refs.is_empty() {
            return DocPos::default();
        }
        let first = self.index.page_first[page];
        let ri = if p.lines.is_empty() {
            first.saturating_sub(1)
        } else {
            let k = p.lines.partition_point(|l| l.top + l.height <= y);
            first + k.min(p.lines.len() - 1)
        };
        self.pos_on_line(ri.min(self.index.refs.len() - 1), x)
    }

    /// The caret's line: (page, x, top, height) in page points, with the preedit width added.
    fn caret_geom(&self, pos: DocPos) -> Option<(usize, f64, f64, f64)> {
        let upstream = self.upstream && pos == self.caret;
        let (ri, x) = self.x_of(pos, upstream)?;
        let line = self.line(ri);
        let dx = if pos == self.caret {
            self.preedit_shape().map_or(0.0, |(_, w, _)| w)
        } else {
            0.0
        };
        Some((
            self.index.refs[ri].page as usize,
            x + dx,
            line.top,
            line.height,
        ))
    }

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

    /// Glyphs of the IME preedit text: (glyph, face, x offset), total width and size in points.
    #[allow(clippy::type_complexity)]
    fn preedit_shape(&self) -> Option<(Vec<(u32, FaceId, f64)>, f64, f64)> {
        let text = self.preedit.as_deref()?;
        let model = self.models.get(self.caret.para)?;
        let style = model.style_at(self.caret.offset);
        let chain = self.fonts.chain(
            &style.font,
            FontStyle {
                bold: style.bold,
                italic: style.italic,
            },
        );
        let shaped = shape_run(&self.fonts, &chain, text, ShapeFeatures::default());
        let size = f64::from(style.size_pt());
        let mut x = 0.0;
        let mut out = Vec::new();
        for g in &shaped.glyphs {
            let upem = f64::from(self.fonts.face(g.face).units_per_em());
            out.push((g.glyph_id, g.face, x));
            x += f64::from(g.x_advance) * size / upem;
        }
        Some((out, x, size))
    }

    /// Scrolls so that the caret line is inside the pane.
    pub fn reveal_caret(&mut self) {
        let Some((page, _, top, height)) = self.caret_geom(self.caret) else {
            return;
        };
        let y = self.index.page_top[page] + top;
        let margin = 24.0 / f64::from(POINT_SCALE);
        if y - margin < self.scroll {
            self.scroll = y - margin;
        } else if y + height + margin > self.scroll + self.view_h_pt() {
            self.scroll = y + height + margin - self.view_h_pt();
        }
        self.clamp_scroll();
    }

    // ---- selection and navigation ------------------------------------------------------------

    pub fn set_caret(&mut self, pos: DocPos) {
        let pos = self.clamp(pos);
        self.caret = pos;
        self.anchor = pos;
        self.goal_x = None;
        self.upstream = false;
        self.pending = RunPropsPatch::default();
        self.touch();
    }

    pub fn select(&mut self, anchor: DocPos, focus: DocPos) {
        self.anchor = self.clamp(anchor);
        self.caret = self.clamp(focus);
        self.goal_x = None;
        self.upstream = false;
        self.pending = RunPropsPatch::default();
        self.touch();
    }

    pub fn select_all(&mut self) {
        if self.models.is_empty() {
            return;
        }
        let last = self.models.len() - 1;
        self.select(
            DocPos::default(),
            DocPos::new(last, self.models[last].char_len),
        );
    }

    fn is_word(c: char) -> bool {
        c.is_alphanumeric() || c == '_'
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
        while i > 0 && !Self::is_word(chars[i - 1]) {
            i -= 1;
        }
        while i > 0 && Self::is_word(chars[i - 1]) {
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
        while i < len && Self::is_word(chars[i]) {
            i += 1;
        }
        while i < len && !Self::is_word(chars[i]) {
            i += 1;
        }
        pos.offset = i;
        pos
    }

    /// Moves the caret (`select`: extend the selection, `big`: by word or to the document end).
    pub fn move_caret(&mut self, nav: Nav, select: bool, big: bool) {
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
        self.pending = RunPropsPatch::default();
        self.touch();
        self.reveal_caret();
    }

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
        let step = (self.view_h_pt() * 0.9).max(line.height);
        let gy = if down { here + step } else { here - step };
        // The view follows by the same distance, as in Word.
        self.scroll_by(if down { step } else { -step });
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

    /// The word around `pos` as a range.
    fn word_at(&self, pos: DocPos) -> (DocPos, DocPos) {
        let chars: Vec<char> = self.models[pos.para].text.chars().collect();
        let mut a = pos.offset.min(chars.len());
        let mut b = a;
        let on_word = chars.get(a).copied().is_some_and(Self::is_word);
        if on_word {
            while a > 0 && Self::is_word(chars[a - 1]) {
                a -= 1;
            }
            while b < chars.len() && Self::is_word(chars[b]) {
                b += 1;
            }
        }
        (DocPos::new(pos.para, a), DocPos::new(pos.para, b))
    }

    // ---- edits (all through the command layer) -----------------------------------------------

    /// Replaces the selection (or inserts at the caret) with `text`; `\n` starts a paragraph.
    fn replace_selection(&mut self, text: &str) {
        if self.models.is_empty() {
            return;
        }
        let (s, e) = self.selection();
        let lines = text.matches('\n').count();
        let (end_para, end_offset) = match text.rsplit_once('\n') {
            Some((_, last)) => (s.para + lines, last.chars().count()),
            None => (s.para, s.offset + text.chars().count()),
        };
        let after = DocPos::new(end_para, end_offset);
        let command = if s == e && lines == 0 {
            Command::DocxInsertText {
                para: s.para,
                offset: s.offset,
                text: text.to_owned(),
            }
        } else {
            Command::DocxReplaceRange {
                start: s,
                end: e,
                text: text.to_owned(),
            }
        };
        if self.run(command).is_none() {
            return;
        }
        let patch = std::mem::take(&mut self.pending);
        if !patch.is_empty() && s != after {
            self.run(Command::DocxSetRunProps {
                start: s,
                end: after,
                patch,
            });
        }
        self.set_caret(after);
        self.preedit = None;
        self.reveal_caret();
    }

    /// Typed or committed text; line breaks become spaces (use [`Self::paste_text`] for lines).
    pub fn insert_text(&mut self, text: &str) {
        let clean: String = text
            .chars()
            .map(|c| if matches!(c, '\n' | '\r') { ' ' } else { c })
            .filter(|c| !c.is_control() || *c == '\t')
            .collect();
        if !clean.is_empty() {
            self.replace_selection(&clean);
        }
    }

    /// Pastes plain text: line breaks start new paragraphs.
    pub fn paste_text(&mut self, text: &str) {
        let normalised = text.replace("\r\n", "\n").replace('\r', "\n");
        let clean: String = normalised
            .chars()
            .filter(|c| !c.is_control() || matches!(c, '\t' | '\n'))
            .collect();
        if !clean.is_empty() {
            self.replace_selection(&clean);
        }
    }

    pub fn delete_selection(&mut self) -> bool {
        if !self.has_selection() {
            return false;
        }
        let (s, e) = self.selection();
        if self
            .run(Command::DocxDeleteRange { start: s, end: e })
            .is_none()
        {
            return false;
        }
        self.set_caret(s);
        self.reveal_caret();
        true
    }

    fn delete_between(&mut self, start: DocPos, end: DocPos) {
        if self.run(Command::DocxDeleteRange { start, end }).is_some() {
            self.set_caret(start);
            self.reveal_caret();
        }
    }

    /// Backspace (`word`: Ctrl+Backspace). Joins with the previous paragraph at its start.
    pub fn backspace(&mut self, word: bool) {
        if self.delete_selection() || self.models.is_empty() {
            return;
        }
        let caret = self.caret;
        let start = if caret.offset > 0 {
            let offset = if word {
                self.word_left(caret).offset
            } else {
                caret.offset - 1
            };
            DocPos::new(caret.para, offset)
        } else if caret.para > 0 {
            DocPos::new(caret.para - 1, self.models[caret.para - 1].char_len)
        } else {
            return;
        };
        self.delete_between(start, caret);
    }

    /// Delete (`word`: Ctrl+Delete). Joins with the next paragraph at the end.
    pub fn delete_forward(&mut self, word: bool) {
        if self.delete_selection() || self.models.is_empty() {
            return;
        }
        let caret = self.caret;
        let len = self.models[caret.para].char_len;
        let end = if caret.offset < len {
            let offset = if word {
                self.word_right(caret).offset.max(caret.offset + 1)
            } else {
                caret.offset + 1
            };
            DocPos::new(caret.para, offset)
        } else if caret.para + 1 < self.models.len() {
            DocPos::new(caret.para + 1, 0)
        } else {
            return;
        };
        self.delete_between(caret, end);
    }

    /// Enter: a new paragraph (a line break with `soft`).
    pub fn enter(&mut self, soft: bool) {
        if self.models.is_empty() {
            return;
        }
        if soft {
            self.delete_selection();
            let at = self.caret;
            if self
                .run(Command::DocxInsertText {
                    para: at.para,
                    offset: at.offset,
                    text: "\n".to_owned(),
                })
                .is_some()
            {
                self.set_caret(DocPos::new(at.para, at.offset + 1));
                self.reveal_caret();
            }
            return;
        }
        if self.has_selection() {
            self.replace_selection("\n");
            return;
        }
        let at = self.clamp(self.caret);
        if self.run(Command::DocxSplitParagraph { pos: at }).is_some() {
            self.set_caret(DocPos::new(at.para + 1, 0));
            self.reveal_caret();
        }
    }

    /// Tab inserts a tab character.
    pub fn tab(&mut self) {
        self.insert_text("\t");
    }

    /// Flips bold, italic or underline over the selection, or for the text typed next.
    pub fn toggle(&mut self, which: Toggle) {
        if self.models.is_empty() {
            return;
        }
        if let Some(v) = which.vert() {
            self.toggle_vert(v);
            return;
        }
        if !self.has_selection() {
            let current = self.models[self.caret.para].style_at(self.caret.offset);
            let on = match which {
                Toggle::Bold => self.pending.bold.unwrap_or(current.bold),
                Toggle::Italic => self.pending.italic.unwrap_or(current.italic),
                Toggle::Underline => self.pending.underline.unwrap_or(current.underline),
                Toggle::Strike => self.pending.strike.unwrap_or(current.strike),
                Toggle::Sub | Toggle::Sup => false,
            };
            let flipped = which.patch(!on);
            match which {
                Toggle::Bold => self.pending.bold = flipped.bold,
                Toggle::Italic => self.pending.italic = flipped.italic,
                Toggle::Underline => self.pending.underline = flipped.underline,
                Toggle::Strike => self.pending.strike = flipped.strike,
                Toggle::Sub | Toggle::Sup => {}
            }
            return;
        }
        let (a, b) = self.selection();
        let all = (a.para..=b.para).all(|p| {
            let len = self.models[p].char_len;
            let start = if p == a.para { a.offset } else { 0 };
            let end = if p == b.para { b.offset } else { len };
            start >= end || self.models[p].all(start, end, |s| which.test(s))
        });
        self.run(Command::DocxSetRunProps {
            start: a,
            end: b,
            patch: which.patch(!all),
        });
    }

    /// The vertical alignment at the caret, effective through the style chain: the text typed next
    /// if a toggle is pending, else the character before the caret.
    fn caret_vert(&self) -> Option<VertAlign> {
        let c = self.caret;
        self.pending.vert_align.or_else(|| {
            self.models
                .get(c.para)
                .map(|m| m.style_at(c.offset).vert_align)
        })
    }

    /// Ctrl+= and Ctrl+Shift+=: sets `v`, or goes back to baseline when the text already has it.
    fn toggle_vert(&mut self, v: VertAlign) {
        if !self.has_selection() {
            let cur = self.caret_vert();
            self.pending.vert_align = Some(if cur == Some(v) {
                VertAlign::Baseline
            } else {
                v
            });
            return;
        }
        let (a, b) = self.selection();
        let all = (a.para..=b.para).all(|p| {
            let len = self.models[p].char_len;
            let start = if p == a.para { a.offset } else { 0 };
            let end = if p == b.para { b.offset } else { len };
            start >= end || self.models[p].all(start, end, |s| s.vert_align == v)
        });
        self.run(Command::DocxSetRunProps {
            start: a,
            end: b,
            patch: RunPropsPatch::vert_align(if all { VertAlign::Baseline } else { v }),
        });
    }

    /// Whether the character formatting at the caret has the toggle on (for button states).
    pub fn toggle_state(&self, which: Toggle) -> bool {
        if let Some(v) = which.vert() {
            return self.caret_vert() == Some(v);
        }
        let Some(m) = self.models.get(self.caret.para) else {
            return false;
        };
        let s = m.style_at(self.caret.offset);
        match which {
            Toggle::Bold => self.pending.bold.unwrap_or(s.bold),
            Toggle::Italic => self.pending.italic.unwrap_or(s.italic),
            Toggle::Underline => self.pending.underline.unwrap_or(s.underline),
            Toggle::Strike => self.pending.strike.unwrap_or(s.strike),
            Toggle::Sub | Toggle::Sup => false,
        }
    }

    /// Font name and size (points) at the caret, for the ribbon's combo boxes.
    pub fn font_state(&self) -> Option<(String, f32)> {
        let m = self.models.get(self.caret.para)?;
        let s = m.style_at(self.caret.offset);
        let font = self
            .pending
            .font
            .clone()
            .unwrap_or_else(|| s.font.to_string());
        let half = self.pending.size_half_points.unwrap_or(s.size_half_points);
        Some((font, half as f32 / 2.0))
    }

    /// Style id of the paragraph at the caret (`Normal` for the default style), for the ribbon's
    /// style gallery.
    pub fn style_state(&self) -> Option<String> {
        self.models
            .get(self.caret.para)
            .map(|m| m.style_id.clone().unwrap_or_else(|| "Normal".to_owned()))
    }

    /// Alignment of the paragraph at the caret.
    pub fn alignment_state(&self) -> Option<open365_docx::Alignment> {
        self.models.get(self.caret.para).map(|m| m.para.alignment)
    }

    /// Applies a ribbon formatting change to the selection (or, for a caret, to the text typed
    /// next). Every change is a command, so it is undoable and audited.
    pub fn format(&mut self, f: Format) {
        if self.models.is_empty() {
            return;
        }
        let (a, b) = self.selection();
        match f {
            Format::Toggle(t) => self.toggle(t),
            Format::Align(alignment) => {
                for para in a.para..=b.para {
                    self.run(Command::DocxSetAlignment { para, alignment });
                }
            }
            Format::Style(style_id) => {
                for para in a.para..=b.para {
                    self.run(Command::DocxSetParagraphStyle {
                        para,
                        style_id: style_id.clone(),
                    });
                }
            }
            Format::Size(_) | Format::SizeStep(_) | Format::Font(_) => {
                let current = self.models[self.caret.para]
                    .style_at(self.caret.offset)
                    .size_half_points;
                let patch = match f {
                    Format::Size(h) => RunPropsPatch {
                        size_half_points: Some(h),
                        ..RunPropsPatch::default()
                    },
                    Format::SizeStep(d) => RunPropsPatch {
                        size_half_points: Some((current as i32 + d).clamp(2, 3276) as u32),
                        ..RunPropsPatch::default()
                    },
                    Format::Font(name) => RunPropsPatch {
                        font: Some(name),
                        ..RunPropsPatch::default()
                    },
                    _ => return,
                };
                if self.has_selection() {
                    self.run(Command::DocxSetRunProps {
                        start: a,
                        end: b,
                        patch,
                    });
                } else {
                    if patch.size_half_points.is_some() {
                        self.pending.size_half_points = patch.size_half_points;
                    }
                    if patch.font.is_some() {
                        self.pending.font = patch.font;
                    }
                }
            }
        }
    }

    pub fn undo(&mut self) -> bool {
        let ok = self.target.undo().is_ok();
        self.after_history(ok)
    }

    pub fn redo(&mut self) -> bool {
        let ok = self.target.redo().is_ok();
        self.after_history(ok)
    }

    fn after_history(&mut self, ok: bool) -> bool {
        if ok {
            self.preedit = None;
            self.pending = RunPropsPatch::default();
            self.sync();
            self.caret = self.clamp(self.caret);
            self.anchor = self.caret;
            self.reveal_caret();
        }
        ok
    }

    // ---- clipboard ---------------------------------------------------------------------------

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

    pub fn copy(&mut self) {
        if self.has_selection() {
            let t = self.selected_text();
            self.clipboard.set_text(&t);
        }
    }

    pub fn cut(&mut self) {
        if self.has_selection() {
            self.copy();
            self.delete_selection();
        }
    }

    pub fn paste(&mut self) {
        if let Some(t) = self.clipboard.get_text() {
            self.paste_text(&t);
        }
    }

    // ---- saving ------------------------------------------------------------------------------

    /// Saves to `path` through the `Save` command.
    pub fn save_to(&mut self, path: PathBuf) -> bool {
        match self.target.apply(Command::Save { path: path.clone() }) {
            Ok(out) => {
                self.status = Some(format!(
                    "Saved {} ({} bytes)",
                    path.display(),
                    out.bytes_written
                ));
                true
            }
            Err(e) => {
                self.status = Some(format!("Save failed: {e}"));
                false
            }
        }
    }

    /// Ctrl+S: saves to the document's path, asking for one when it has none.
    pub fn save(&mut self) -> bool {
        match self.target.path().map(Path::to_path_buf) {
            Some(p) => self.save_to(p),
            None => self.save_as(),
        }
    }

    /// Save As: asks for a path with the chooser (the `rfd` dialog by default).
    pub fn save_as(&mut self) -> bool {
        match (self.chooser)() {
            Some(mut p) => {
                if p.extension().is_none() {
                    p.set_extension("docx");
                }
                self.save_to(p)
            }
            None => false,
        }
    }

    // ---- input routing -----------------------------------------------------------------------

    /// Creates the canvas node under `parent` over `area` (device pixels), replacing the old one,
    /// and gives it the keyboard focus. Returns the node.
    pub fn build(
        &mut self,
        ui: &mut Ui,
        parent: NodeId,
        area: PxRect,
        scale: f32,
        zoom: f32,
    ) -> NodeId {
        if let Some(old) = self.canvas.take() {
            ui.tree.remove(old);
        }
        self.set_view(area, scale, zoom);
        let node = Node::new(
            Widget::Canvas {
                caret: Rect::new(0.0, 0.0, 1.0, 16.0),
            },
            Rect::new(
                area.x as f32 / scale,
                area.y as f32 / scale,
                area.w as f32 / scale,
                area.h as f32 / scale,
            ),
        )
        .with_label("Document");
        let id = ui.add(parent, node);
        self.canvas = Some(id);
        self.update_node(ui);
        ui.set_focus(Some(id));
        id
    }

    /// Removes the canvas node (a Backstage view covers the pane).
    pub fn release(&mut self, ui: &mut Ui) {
        if let Some(old) = self.canvas.take() {
            ui.tree.remove(old);
        }
    }

    /// Keeps the canvas node's IME caret rectangle and name current.
    pub fn update_node(&mut self, ui: &mut Ui) {
        let Some(id) = self.canvas else { return };
        let s = self.s();
        let caret = self
            .caret_geom(self.caret)
            .map(|(page, x, top, h)| {
                Rect::new(
                    (self.page_x(page) + x as f32 * s) / self.scale,
                    (self.page_y(page) + top as f32 * s) / self.scale,
                    1.0,
                    (h as f32 * s) / self.scale,
                )
            })
            .unwrap_or_default();
        let label = format!("Document, {} paragraphs", self.models.len());
        if let Some(n) = ui.tree.get_mut(id) {
            n.widget = Widget::Canvas { caret };
            n.label = label;
        }
    }

    fn local(&self, ui: &Ui, node: NodeId, p: open365_ui_core::Point) -> (f32, f32) {
        let r = ui.tree.abs_rect(node);
        ((p.x - r.x) * self.scale, (p.y - r.y) * self.scale)
    }

    /// Extends the selection while the primary button is held after a press on the canvas. The
    /// host calls this every frame (the pointer position is `ui.pointer()`).
    pub fn drag(&mut self, ui: &mut Ui) {
        if let Some(node) = self.canvas {
            let pointer = (ui.captured() == Some(node)).then(|| self.local(ui, node, ui.pointer()));
            if self.ruler_drag(pointer) {
                self.update_node(ui);
                return;
            }
        }
        if self.bar_drag {
            let Some(node) = self.canvas else { return };
            if ui.captured() == Some(node) {
                let (_, y) = self.local(ui, node, ui.pointer());
                self.scroll_to_bar(y);
                self.update_node(ui);
            } else {
                self.bar_drag = false;
            }
            return;
        }
        if !self.dragging {
            return;
        }
        let Some(node) = self.canvas else { return };
        if ui.captured() != Some(node) {
            self.dragging = false;
            return;
        }
        let (x, y) = self.local(ui, node, ui.pointer());
        let pos = self.hit(x, y);
        if pos != self.caret {
            self.caret = pos;
            self.upstream = false;
            self.touch();
            // Dragging past the top or bottom edge scrolls.
            let h = self.area.h as f32;
            if y < 0.0 {
                self.scroll_by(-f64::from(-y / self.s()).min(40.0));
            } else if y > h {
                self.scroll_by(f64::from((y - h) / self.s()).min(40.0));
            }
            self.update_node(ui);
        }
    }

    /// Mouse wheel over the pane (`delta_y` positive: up, in notches).
    pub fn wheel(&mut self, delta_y: f32) {
        self.scroll_by(-f64::from(delta_y) * 3.0 * 14.0);
    }

    /// Mouse wheel with the modifier keys: Ctrl zooms about the pointer (`y`, canvas-local
    /// device pixels), Shift scrolls sideways, else it scrolls.
    pub fn wheel_with(&mut self, delta_y: f32, shift: bool, ctrl: bool, y: f32) {
        if ctrl {
            self.zoom_step(delta_y, y);
        } else if shift {
            self.scroll_x -= delta_y * 3.0 * 14.0 * self.s() / POINT_SCALE;
            self.clamp_scroll();
        } else {
            self.wheel(delta_y);
        }
    }

    /// Handles an event from the canvas. Returns true when it was one of the pane's.
    pub fn handle(&mut self, ui: &mut Ui, ev: &Emit) -> bool {
        let mine = |n: &NodeId| Some(*n) == self.canvas;
        let consumed = match ev {
            Emit::CanvasPointer {
                node,
                at,
                clicks,
                shift,
            } if mine(node) => {
                if self.replace_press(at.x * self.scale, at.y * self.scale) {
                    self.update_node(ui);
                    return true;
                }
                if self.ruler_press(at.x * self.scale, at.y * self.scale) {
                    self.update_node(ui);
                    return true;
                }
                if self.on_bar(at.x * self.scale) {
                    self.bar_drag = true;
                    self.scroll_to_bar(at.y * self.scale);
                    self.update_node(ui);
                    return true;
                }
                let pos = self.hit(at.x * self.scale, at.y * self.scale);
                match clicks {
                    0 | 1 => {
                        if *shift {
                            self.select(self.anchor, pos);
                        } else {
                            self.set_caret(pos);
                        }
                        self.dragging = true;
                    }
                    2 => {
                        let (a, b) = self.word_at(pos);
                        self.select(a, b);
                    }
                    _ => {
                        let len = self.models.get(pos.para).map_or(0, |m| m.char_len);
                        self.select(DocPos::new(pos.para, 0), DocPos::new(pos.para, len));
                    }
                }
                true
            }
            Emit::CanvasText(n, s) if mine(n) => {
                if self.find.open {
                    self.find_text(s);
                } else {
                    self.insert_text(s);
                }
                true
            }
            Emit::CanvasWheel {
                node,
                delta_y,
                shift,
                ctrl,
            } if mine(node) => {
                let y = (ui.pointer().y - ui.tree.abs_rect(*node).y) * self.scale;
                self.wheel_with(*delta_y, *shift, *ctrl, y);
                true
            }
            Emit::CanvasIme(n, i) if mine(n) => {
                self.ime(i);
                true
            }
            Emit::KeyPressed(n, key, m) if mine(n) => {
                self.key(*key, *m);
                true
            }
            _ => false,
        };
        if consumed {
            self.update_node(ui);
        }
        consumed
    }

    /// IME composition: the preedit text sits at the caret, the commit becomes typed text.
    pub fn ime(&mut self, i: &ImeInput) {
        match i {
            ImeInput::Enabled => {}
            ImeInput::Preedit { text, .. } => {
                if !text.is_empty() && self.has_selection() {
                    self.delete_selection();
                }
                self.preedit = (!text.is_empty()).then(|| text.clone());
                self.touch();
            }
            ImeInput::Commit(t) => {
                self.preedit = None;
                self.insert_text(t);
            }
            ImeInput::Disabled => self.preedit = None,
        }
    }

    /// A key the canvas does not take as text.
    pub fn key(&mut self, key: Key, m: Modifiers) {
        if self.find.open && self.find_key(key, m) {
            return;
        }
        match key {
            Key::Left => self.move_caret(Nav::Left, m.shift, m.ctrl),
            Key::Right => self.move_caret(Nav::Right, m.shift, m.ctrl),
            Key::Up => self.move_caret(Nav::Up, m.shift, m.ctrl),
            Key::Down => self.move_caret(Nav::Down, m.shift, m.ctrl),
            Key::Home => self.move_caret(Nav::Home, m.shift, m.ctrl),
            Key::End => self.move_caret(Nav::End, m.shift, m.ctrl),
            Key::PageUp => self.move_caret(Nav::PageUp, m.shift, false),
            Key::PageDown => self.move_caret(Nav::PageDown, m.shift, false),
            Key::Backspace => self.backspace(m.ctrl),
            Key::Delete => self.delete_forward(m.ctrl),
            Key::Enter => self.enter(m.shift),
            Key::Tab if !m.shift => self.tab(),
            Key::Char('=') if m.ctrl && !m.alt && !m.shift => self.toggle(Toggle::Sub),
            Key::Char('=' | '+') if m.ctrl && !m.alt => self.toggle(Toggle::Sup),
            Key::Char(c) if m.ctrl && !m.alt => match c.to_ascii_uppercase() {
                'A' => self.select_all(),
                'F' => self.find_open(),
                'H' => self.replace_open(),
                'B' => self.toggle(Toggle::Bold),
                'I' => self.toggle(Toggle::Italic),
                'U' => self.toggle(Toggle::Underline),
                'C' => self.copy(),
                'X' => self.cut(),
                'V' => self.paste(),
                'Z' => {
                    self.undo();
                }
                'Y' => {
                    self.redo();
                }
                'S' => {
                    if m.shift {
                        self.save_as();
                    } else {
                        self.save();
                    }
                }
                _ => {}
            },
            Key::Char(c) if m.ctrl && m.alt => {
                if let Some(style) = Format::style_for_shortcut(c) {
                    self.format(Format::Style(style.to_owned()));
                }
            }
            _ => {}
        }
    }

    // ---- painting ----------------------------------------------------------------------------

    /// What the paints since the last [`Self::reset_stats`] drew.
    pub fn stats(&self) -> PaintStats {
        self.stats
    }

    /// Pictures decoded and held for painting.
    pub fn cached_pictures(&self) -> usize {
        self.pictures.len()
    }

    pub fn reset_stats(&mut self) {
        self.stats = PaintStats::default();
    }

    /// The caret is in the shown half of its blink cycle at `now`.
    pub fn caret_on(&self, now: Instant) -> bool {
        let t = now.saturating_duration_since(self.blink_epoch);
        (t.as_millis() / BLINK.as_millis()).is_multiple_of(2)
    }

    fn gfx_font(&mut self, ui: &mut Ui, face: FaceId) -> Option<FontId> {
        if let Some(f) = self.gfx_fonts.get(&face.index()) {
            return Some(*f);
        }
        let f = self.fonts.try_face(face)?;
        let id = ui
            .assets
            .add_font(Arc::from(f.bytes().to_vec()), f.index())
            .ok()?;
        self.gfx_fonts.insert(face.index(), id);
        Some(id)
    }

    /// Draws the canvas colour, the pages, the selection, the text and the caret.
    pub fn paint(&mut self, ui: &mut Ui, list: &mut DrawList, pal: &Palette, now: Instant) {
        self.drag(ui);
        self.sync();
        let focused = self.canvas.is_some() && ui.focus() == self.canvas;
        let (ox, oy) = (self.area.x as f32, self.area.y as f32);
        let (w, h) = (self.area.w as f32, self.area.h as f32);
        let s = self.s();
        list.push_clip(GRect::from_edges(ox, oy, ox + w, oy + h));
        list.fill_rect(GRect::from_edges(ox, oy, ox + w, oy + h), pal.canvas);
        // The pages scroll under the ruler band, not through it.
        let view = (oy + self.top_inset(), oy + h);
        list.push_clip(GRect::from_edges(
            ox + self.left_inset(),
            view.0,
            ox + w,
            view.1,
        ));
        let pages = self.engine.layout().pages.len();
        let sel = self.selection();
        for page in 0..pages {
            let (px, py) = (ox + self.page_x(page), oy + self.page_y(page));
            let sec = self.engine.layout().pages[page].section;
            let (pw, ph) = (sec.page_width as f32 * s, sec.page_height as f32 * s);
            if py > oy + h || py + ph < oy {
                continue;
            }
            let rect =
                GRect::from_edges(px.round(), py.round(), (px + pw).round(), (py + ph).round());
            list.fill_rect(rect, pal.page);
            if let Some(b) = pal.page_border {
                list.rect_outline(rect, 1.0, b);
            }
            // No crop marks or text boundaries: Word's defaults have both off.
            self.paint_cells(list, page, (px, py), view);
            self.paint_pictures(ui, list, page, (px, py), view, pictures::Layer::Behind);
            self.paint_find_marks(list, page, (px, py), view);
            let first = self.index.page_first[page];
            let n = self.engine.layout().pages[page].lines.len();
            if self.has_selection() {
                for li in 0..n {
                    let line = &self.engine.layout().pages[page].lines[li];
                    let (top, bottom) = (
                        py + line.top as f32 * s,
                        py + (line.top + line.height) as f32 * s,
                    );
                    if bottom < oy || top > oy + h {
                        continue;
                    }
                    if let Some((x0, x1)) = self.selection_span(first + li, sel) {
                        list.fill_rect(
                            GRect::from_edges(px + x0 as f32 * s, top, px + x1 as f32 * s, bottom),
                            pal.accent.with_opacity(0.35),
                        );
                    }
                }
            }
            // While composing, the rest of the caret's line moves right to make room.
            let composing = self.preedit_shape();
            let shift = composing.as_ref().and_then(|(_, wd, _)| {
                let (ri, x) = self.x_of(self.caret, self.upstream)?;
                Some((ri, x, *wd))
            });
            for li in 0..n {
                let (top, bottom, baseline, dx) = {
                    let line = &self.engine.layout().pages[page].lines[li];
                    (
                        py + line.top as f32 * s,
                        py + (line.top + line.height) as f32 * s,
                        py + line.baseline as f32 * s,
                        shift
                            .filter(|(ri, _, _)| *ri == first + li)
                            .map(|(_, x, wd)| (x, wd)),
                    )
                };
                if bottom < oy || top > oy + h {
                    continue;
                }
                self.paint_line(ui, list, pal, (page, li), (px, baseline), s, dx, 1.0);
                if let (Some((glyphs, width, size)), Some((x, _))) = (&composing, dx) {
                    let px_size = *size as f32 * s;
                    let mut by_face: Vec<(FaceId, Vec<GlyphPos>)> = Vec::new();
                    for (gid, face, off) in glyphs {
                        let pos = GlyphPos {
                            id: *gid,
                            x: px + (x + *off) as f32 * s,
                            y: baseline,
                        };
                        match by_face.last_mut() {
                            Some((f, v)) if f == face => v.push(pos),
                            _ => by_face.push((*face, vec![pos])),
                        }
                    }
                    for (face, v) in by_face {
                        if let Some(font) = self.gfx_font(ui, face) {
                            list.glyph_run(
                                &mut ui.assets,
                                &GlyphRun {
                                    font,
                                    size_px: px_size,
                                    glyphs: &v,
                                },
                                pal.page_text,
                            );
                        }
                    }
                    let y = baseline + *size as f32 * 0.12 * s;
                    list.hline(
                        y,
                        px + x as f32 * s,
                        px + (x + *width) as f32 * s,
                        1.0,
                        pal.page_text,
                    );
                }
            }
            // Header, footnotes and footer are dimmed while the body is edited.
            for part in 0..3 {
                let count = {
                    let p = &self.engine.layout().pages[page];
                    [&p.header, &p.notes, &p.footer][part].len()
                };
                for li in 0..count {
                    let baseline = {
                        let p = &self.engine.layout().pages[page];
                        py + [&p.header, &p.notes, &p.footer][part][li].baseline as f32 * s
                    };
                    self.paint_furniture(ui, list, pal, (page, part, li), (px, baseline), s);
                }
            }
            self.paint_pictures(ui, list, page, (px, py), view, pictures::Layer::Front);
        }
        list.pop_clip();
        if focused
            && !self.has_selection()
            && self.caret_on(now)
            && let Some((page, x, top, height)) = self.caret_geom(self.caret)
        {
            let cx = ox + self.page_x(page) + x as f32 * s;
            let cy = oy + self.page_y(page) + top as f32 * s;
            let width = (self.scale).max(1.0).round();
            list.fill_rect(
                GRect::from_edges(
                    cx.round(),
                    cy.round(),
                    cx.round() + width,
                    (cy + height as f32 * s).round(),
                ),
                pal.page_text,
            );
        }
        self.paint_ruler(ui, list, pal);
        self.paint_vruler(ui, list, pal);
        self.paint_replace_box(ui, list, pal);
        self.paint_find_box(ui, list, pal);
        if let Some((top, th)) = self.thumb() {
            let bw = self.bar_w();
            let x1 = ox + w;
            list.fill_rect(GRect::from_edges(x1 - bw, oy, x1, oy + h), pal.canvas);
            let pad = (3.0 * self.scale / 1.25).round();
            list.fill_rect(
                GRect::from_edges(x1 - bw + pad, oy + top + pad, x1 - pad, oy + top + th - pad),
                pal.page_text
                    .with_opacity(if self.bar_drag { 0.6 } else { 0.35 }),
            );
        }
        list.pop_clip();
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_line(
        &mut self,
        ui: &mut Ui,
        list: &mut DrawList,
        pal: &Palette,
        at: (usize, usize),
        origin: (f32, f32),
        s: f32,
        dx: Option<(f64, f64)>,
        opacity: f32,
    ) {
        let line = self.engine.layout().pages[at.0].lines[at.1].clone();
        self.paint_line_data(ui, list, pal, &line, origin, s, dx, opacity);
    }

    fn paint_furniture(
        &mut self,
        ui: &mut Ui,
        list: &mut DrawList,
        pal: &Palette,
        at: (usize, usize, usize),
        origin: (f32, f32),
        s: f32,
    ) {
        let line = {
            let p = &self.engine.layout().pages[at.0];
            [&p.header, &p.notes, &p.footer][at.1][at.2].clone()
        };
        self.paint_line_data(ui, list, pal, &line, origin, s, None, FURNITURE_DIM);
    }

    /// Draws one line's glyph runs with their underline and strike-through. `origin` is the
    /// page's left edge and the line's baseline in device pixels.
    #[allow(clippy::too_many_arguments)]
    fn paint_line_data(
        &mut self,
        ui: &mut Ui,
        list: &mut DrawList,
        pal: &Palette,
        line: &Line,
        origin: (f32, f32),
        s: f32,
        dx: Option<(f64, f64)>,
        opacity: f32,
    ) {
        let px = origin.0;
        for run in &line.runs {
            let (Some(first), Some(last)) = (run.glyphs.first(), run.glyphs.last()) else {
                continue;
            };
            let color = run
                .color
                .map_or(pal.page_text, |[r, g, b]| Color::rgb(r, g, b));
            let color = if opacity < 1.0 {
                color.with_opacity(opacity)
            } else {
                color
            };
            let move_x = |x: f64| match dx {
                Some((at, w)) if x >= at - 1e-6 => x + w,
                _ => x,
            };
            let size_px = run.size as f32 * s;
            // A note reference is drawn raised on its own baseline.
            let raise = run_raise(run) as f32 * s;
            if raise != 0.0 {
                self.stats.raised_runs += 1;
            }
            let baseline = origin.1 - raise;
            let mut batch: Vec<GlyphPos> = Vec::new();
            let mut batch_face: Option<FaceId> = None;
            for g in &run.glyphs {
                if batch_face != Some(g.face) {
                    self.flush_glyphs(ui, list, batch_face, &batch, size_px, color);
                    batch.clear();
                    batch_face = Some(g.face);
                }
                batch.push(GlyphPos {
                    id: g.glyph_id,
                    x: px + move_x(g.x) as f32 * s,
                    y: baseline - g.y_offset as f32 * s,
                });
            }
            self.flush_glyphs(ui, list, batch_face, &batch, size_px, color);
            let x0 = px + move_x(first.x) as f32 * s;
            let x1 = px + (move_x(last.x) + last.advance) as f32 * s;
            let thickness = (run.size as f32 * 0.06 * s).max(1.0);
            if run.underline {
                list.hline(
                    baseline + run.size as f32 * 0.12 * s,
                    x0,
                    x1,
                    thickness,
                    color,
                );
            }
            if run.strike {
                list.hline(
                    baseline - run.size as f32 * 0.28 * s,
                    x0,
                    x1,
                    thickness,
                    color,
                );
            }
        }
    }

    fn flush_glyphs(
        &mut self,
        ui: &mut Ui,
        list: &mut DrawList,
        face: Option<FaceId>,
        glyphs: &[GlyphPos],
        size_px: f32,
        color: Color,
    ) {
        let Some(face) = face else { return };
        if glyphs.is_empty() {
            return;
        }
        if let Some(font) = self.gfx_font(ui, face) {
            list.glyph_run(
                &mut ui.assets,
                &GlyphRun {
                    font,
                    size_px,
                    glyphs,
                },
                color,
            );
        }
    }

    // ---- accessibility -----------------------------------------------------------------------

    fn run_id(para: usize, line: usize, chunk: usize) -> AkId {
        AkId(
            (1u64 << 60)
                | ((para as u64) << 24)
                | ((line as u64 & 0xFFFF) << 8)
                | (chunk as u64 & 0xFF),
        )
    }

    fn para_id(para: usize) -> AkId {
        AkId((1u64 << 59) | para as u64)
    }

    fn access_position(&self, pos: DocPos) -> TextPosition {
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
        let chunk = if column > 0 && column.is_multiple_of(MAX_CHARS_PER_RUN) {
            column / MAX_CHARS_PER_RUN - 1
        } else {
            column / MAX_CHARS_PER_RUN
        };
        TextPosition {
            node: Self::run_id(pos.para, line, chunk),
            character_index: column - chunk * MAX_CHARS_PER_RUN,
        }
    }

    /// The accessibility subtree of the pane: a multi-line text input holding the selection, with
    /// a paragraph (or heading, list item) per visible paragraph and a text run per line. The
    /// paragraphs holding the caret and the selection anchor are always included.
    pub fn access(&self) -> Option<CanvasAccess> {
        let canvas = self.canvas?;
        let s = self.s();
        let (ox, oy) = (self.area.x as f32, self.area.y as f32);
        let mut paras: Vec<usize> = Vec::new();
        for (ri, r) in self.index.refs.iter().enumerate() {
            let line = self.line(ri);
            let y = oy + self.page_y(r.page as usize) + line.top as f32 * s;
            if y + (line.height as f32 * s) >= oy
                && y <= oy + self.area.h as f32
                && paras.last() != Some(&line.paragraph)
            {
                paras.push(line.paragraph);
            }
        }
        for p in [self.caret.para, self.anchor.para] {
            if p < self.models.len() {
                paras.push(p);
            }
        }
        paras.sort_unstable();
        paras.dedup();
        let mut nodes: Vec<(AkId, AkNode)> = Vec::new();
        let mut children = Vec::new();
        for para in paras {
            let range = self.line_range_of(para);
            let mut run_ids = Vec::new();
            let mut bounds: Option<accesskit::Rect> = None;
            for (li, ri) in range.clone().enumerate() {
                let r = self.index.refs[ri];
                let line = self.line(ri);
                let geom = self.line_geom(ri);
                let chars: Vec<char> = self.models[para]
                    .text
                    .chars()
                    .skip(geom.cs)
                    .take(geom.ce - geom.cs)
                    .collect();
                let x0 = geom.xs[0];
                let left = ox + self.page_x(r.page as usize) + x0 as f32 * s;
                let top = oy + self.page_y(r.page as usize) + line.top as f32 * s;
                let rect = accesskit::Rect {
                    x0: f64::from(left),
                    y0: f64::from(top),
                    x1: f64::from(
                        ox + self.page_x(r.page as usize) + geom.xs[chars.len()] as f32 * s,
                    ),
                    y1: f64::from(top + line.height as f32 * s),
                };
                bounds = Some(bounds.map_or(rect, |b| b.union(rect)));
                let last_line = ri + 1 == range.end;
                let continuation = li > 0
                    && self.line(ri - 1).end == line.start
                    && self.models[para]
                        .text
                        .chars()
                        .nth(geom.cs.saturating_sub(1))
                        != Some('\n');
                let mut value = String::new();
                let mut lengths: Vec<u8> = Vec::new();
                let mut positions: Vec<f32> = geom.xs[..chars.len()]
                    .iter()
                    .map(|x| ((x - x0) as f32) * s)
                    .collect();
                let mut widths: Vec<f32> = geom
                    .xs
                    .windows(2)
                    .map(|w| ((w[1] - w[0]) as f32) * s)
                    .collect();
                let mut word_starts: Vec<usize> = Vec::new();
                let mut at_word_end = continuation;
                for ch in &chars {
                    let word = Self::is_word(*ch);
                    if word && at_word_end {
                        word_starts.push(lengths.len());
                    }
                    at_word_end = !word;
                    let before = value.len();
                    value.push(*ch);
                    lengths.push((value.len() - before) as u8);
                }
                if last_line {
                    value.push('\n');
                    lengths.push(1);
                    positions.push((rect.x1 - rect.x0) as f32);
                    widths.push(0.0);
                }
                let total = lengths.len();
                let chunks = total.div_ceil(MAX_CHARS_PER_RUN).max(1);
                let mut byte_start = 0usize;
                for chunk in 0..chunks {
                    let from = chunk * MAX_CHARS_PER_RUN;
                    let to = (from + MAX_CHARS_PER_RUN).min(total);
                    let bytes: usize = lengths[from..to].iter().map(|l| usize::from(*l)).sum();
                    let text = value[byte_start..byte_start + bytes].to_owned();
                    byte_start += bytes;
                    let mut node = AkNode::new(Role::TextRun);
                    node.set_value(text);
                    node.set_bounds(rect);
                    node.set_character_lengths(lengths[from..to].to_vec());
                    let base = positions.get(from).copied().unwrap_or(0.0);
                    node.set_character_positions(
                        positions[from.min(positions.len())..to.min(positions.len())]
                            .iter()
                            .map(|p| p - base)
                            .collect::<Vec<_>>(),
                    );
                    node.set_character_widths(
                        widths[from.min(widths.len())..to.min(widths.len())].to_vec(),
                    );
                    node.set_word_starts(
                        word_starts
                            .iter()
                            .filter(|w| **w >= from && **w < to)
                            .map(|w| (*w - from) as u8)
                            .collect::<Vec<_>>(),
                    );
                    let id = Self::run_id(para, li, chunk);
                    run_ids.push(id);
                    nodes.push((id, node));
                }
            }
            if run_ids.is_empty() {
                // An empty paragraph with no layout line yet still needs a run to hold the caret.
                let mut node = AkNode::new(Role::TextRun);
                node.set_value("\n");
                node.set_character_lengths(vec![1u8]);
                node.set_character_positions(vec![0.0f32]);
                node.set_character_widths(vec![0.0f32]);
                let id = Self::run_id(para, 0, 0);
                run_ids.push(id);
                nodes.push((id, node));
            }
            let role = if range.is_empty() {
                ParaRole::Plain
            } else {
                self.line(range.start).role
            };
            let mut p = AkNode::new(match role {
                ParaRole::Plain => Role::Paragraph,
                ParaRole::Heading(_) => Role::Heading,
                ParaRole::ListItem => Role::ListItem,
            });
            if let ParaRole::Heading(level) = role {
                p.set_level(usize::from(level));
            }
            if let Some(b) = bounds {
                p.set_bounds(b);
            }
            p.set_children(run_ids);
            nodes.push((Self::para_id(para), p));
            children.push(Self::para_id(para));
        }
        children.extend(self.access_shapes(&mut nodes));
        let mut doc = AkNode::new(Role::MultilineTextInput);
        doc.set_label(format!("Document, {} paragraphs", self.models.len()));
        doc.set_children(children);
        doc.add_action(Action::Focus);
        doc.add_action(Action::SetTextSelection);
        doc.add_action(Action::ReplaceSelectedText);
        if !self.models.is_empty() {
            doc.set_text_selection(TextSelection {
                anchor: self.access_position(self.anchor),
                focus: self.access_position(self.caret),
            });
        }
        let _ = ak_id(canvas);
        Some(CanvasAccess { node: doc, nodes })
    }

    /// Nodes for the pictures, shapes and text boxes in view: an image (or text box) with its
    /// alternative text (`wp:docPr/@descr`) as the name, as Word exposes them. A picture without
    /// alternative text is named "Picture". Returns the ids, in page order, for the document node.
    pub fn access_shapes(&self, nodes: &mut Vec<(AkId, AkNode)>) -> Vec<AkId> {
        let s = self.s();
        let (ox, oy) = (self.area.x as f32, self.area.y as f32);
        let mut ids = Vec::new();
        let rect = |page: usize, x: f64, y: f64, w: f64, h: f64| accesskit::Rect {
            x0: f64::from(ox + self.page_x(page)) + x * f64::from(s),
            y0: f64::from(oy + self.page_y(page)) + y * f64::from(s),
            x1: f64::from(ox + self.page_x(page)) + (x + w) * f64::from(s),
            y1: f64::from(oy + self.page_y(page)) + (y + h) * f64::from(s),
        };
        let (top, bottom) = (
            f64::from(oy + self.top_inset()),
            f64::from(oy) + f64::from(self.area.h),
        );
        for (pi, page) in self.engine.layout().pages.iter().enumerate() {
            let groups = [
                &page.images,
                &page.header_images,
                &page.note_images,
                &page.footer_images,
            ];
            for (gi, group) in groups.into_iter().enumerate() {
                for (ii, im) in group.iter().enumerate() {
                    let r = rect(pi, im.x, im.y, im.width, im.height);
                    if r.y1 < top || r.y0 > bottom {
                        continue;
                    }
                    let mut node = AkNode::new(Role::Image);
                    let alt = im
                        .description
                        .as_deref()
                        .map(str::trim)
                        .filter(|d| !d.is_empty());
                    node.set_label(alt.unwrap_or("Picture"));
                    if let Some(alt) = alt {
                        node.set_description(alt);
                    }
                    node.set_bounds(r);
                    let id = AkId(
                        (1u64 << 58)
                            | ((pi as u64) << 24)
                            | ((gi as u64) << 20)
                            | ii as u64 & 0xF_FFFF,
                    );
                    nodes.push((id, node));
                    ids.push(id);
                }
            }
            for (fi, f) in page.frames.iter().enumerate() {
                let r = rect(pi, f.x, f.y, f.width, f.height);
                if r.y1 < top || r.y0 > bottom {
                    continue;
                }
                let mut node = AkNode::new(Role::GenericContainer);
                node.set_label("Text Box");
                node.set_bounds(r);
                // The text inside the box: one text run per line, in reading order.
                let mut kids = Vec::new();
                let (mut para, mut consumed) = (None, 0usize);
                for (li, line) in f.lines.iter().enumerate() {
                    if para != Some(line.paragraph) {
                        para = Some(line.paragraph);
                        consumed = 0;
                    }
                    let (chars, xs) = Self::box_line_chars(line, consumed);
                    consumed += chars.len();
                    let (x0, x1) = (xs[0], xs[chars.len()]);
                    let mut run = AkNode::new(Role::TextRun);
                    let mut value: String = chars
                        .iter()
                        .map(|c| if matches!(c, '\r' | '\n') { '\n' } else { *c })
                        .collect();
                    if value.is_empty() {
                        value.push('\n');
                    }
                    let lengths: Vec<u8> = value.chars().map(|c| c.len_utf8() as u8).collect();
                    let n = lengths.len();
                    run.set_value(value);
                    run.set_bounds(rect(pi, x0, line.top, x1 - x0, line.height));
                    run.set_character_lengths(lengths);
                    run.set_character_positions(
                        (0..n)
                            .map(|i| (xs.get(i).copied().unwrap_or(x1) - x0) as f32 * s)
                            .collect::<Vec<_>>(),
                    );
                    run.set_character_widths(
                        (0..n)
                            .map(|i| {
                                (xs.get(i + 1).copied().unwrap_or(x1)
                                    - xs.get(i).copied().unwrap_or(x1))
                                    as f32
                                    * s
                            })
                            .collect::<Vec<_>>(),
                    );
                    let rid = AkId(
                        (1u64 << 56)
                            | ((pi as u64 & 0xFFF) << 44)
                            | ((fi as u64 & 0xFFF) << 32)
                            | li as u64 & 0xFFFF_FFFF,
                    );
                    kids.push(rid);
                    nodes.push((rid, run));
                }
                node.set_children(kids);
                let id = AkId((1u64 << 57) | ((pi as u64) << 24) | fi as u64 & 0xFF_FFFF);
                nodes.push((id, node));
                ids.push(id);
            }
        }
        ids
    }

    /// The characters of a text-box line (those after the first `skip` of its paragraph) and
    /// the x of each character's left edge plus the line's end, in page points.
    fn box_line_chars(line: &Line, skip: usize) -> (Vec<char>, Vec<f64>) {
        let units = line.end.saturating_sub(line.start);
        let (mut chars, mut u) = (Vec::new(), 0usize);
        for c in line.text.chars().skip(skip) {
            if u >= units {
                break;
            }
            u += c.len_utf16();
            chars.push(c);
        }
        let mut spans: Vec<(usize, usize, f64, f64)> = Vec::new();
        for g in line.runs.iter().flat_map(|r| &r.glyphs) {
            let r = line.cluster_story(g);
            if g.cluster.kind != ClusterKind::Text || r.is_empty() {
                continue;
            }
            match spans.last_mut() {
                Some(last) if last.0 == r.start && last.1 == r.end => {
                    last.3 = last.3.max(g.x + g.advance)
                }
                _ => spans.push((r.start, r.end, g.x, g.x + g.advance)),
            }
        }
        let mut xs = Vec::with_capacity(chars.len() + 1);
        let (mut pen, mut at, mut si) = (line.left, line.start, 0usize);
        for c in &chars {
            while si < spans.len() && spans[si].1 <= at {
                si += 1;
            }
            let n = c.len_utf16();
            match spans.get(si) {
                Some(&(start, end, x0, x1)) if start <= at => {
                    let w = (end - start) as f64;
                    xs.push(x0 + (x1 - x0) * (at - start) as f64 / w);
                    pen = x0 + (x1 - x0) * ((at + n).min(end) - start) as f64 / w;
                }
                _ => xs.push(pen),
            }
            at += n;
        }
        xs.push(pen);
        (chars, xs)
    }

    /// A screen reader's `SetTextSelection` or `ReplaceSelectedText` request.
    pub fn accessibility_action(&mut self, request: &accesskit::ActionRequest) -> bool {
        use accesskit::ActionData;
        match (request.action, &request.data) {
            (Action::SetTextSelection, Some(ActionData::SetTextSelection(sel))) => {
                match (
                    self.pos_from_access(&sel.anchor),
                    self.pos_from_access(&sel.focus),
                ) {
                    (Some(a), Some(f)) => {
                        self.select(a, f);
                        true
                    }
                    _ => false,
                }
            }
            (Action::ReplaceSelectedText, Some(ActionData::Value(text))) => {
                self.insert_text(text);
                true
            }
            _ => false,
        }
    }

    fn pos_from_access(&self, p: &TextPosition) -> Option<DocPos> {
        let id = p.node.0;
        if id >> 60 != 1 {
            return None;
        }
        let para = ((id >> 24) & 0xFFFF_FFFF) as usize;
        let line = ((id >> 8) & 0xFFFF) as usize;
        let chunk = (id & 0xFF) as usize;
        let range = self.line_range_of(para);
        let start = range
            .clone()
            .nth(line)
            .map_or(0, |ri| self.line_geom(ri).cs);
        Some(self.clamp(DocPos::new(
            para,
            start + chunk * MAX_CHARS_PER_RUN + p.character_index,
        )))
    }
}
