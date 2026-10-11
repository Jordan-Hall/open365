//! Word-compatible page layout (layer 3).
//!
//! [`layout_document`] turns an [`open365_docx::Document`] into pages, lines and positioned
//! glyph runs. The rules are measured against Microsoft Word 16.0 with
//! `tools/office/word-layout-oracle.ps1`; `tests/word_parity.rs` compares every line with the
//! recorded reference and a ratcheting floor file prevents regressions. See
//! `docs/architecture/word-layout.md` for what is measured and what is still open.
//!
//! Units are points; page coordinates have the origin at the top-left of the page and y grows
//! downwards. Character offsets are UTF-16 code units of the main story, counting a paragraph
//! mark, a tab and a break as one each, which is how Word's `Range.Start` counts.
//!
//! Tables are laid out with Word's column, row and page-splitting rules (`table.rs`,
//! `flow.rs`). Headers, footers (`furniture.rs`), footnotes and endnotes are laid out as Word
//! does; PAGE, NUMPAGES and SECTIONPAGES fields are recomputed in headers and footers, every
//! other field keeps its cached result. Inline pictures are boxes on the baseline that set the
//! line height; floating pictures that wrap top and bottom or square displace text; both are
//! reported in [`Page::images`]. Section columns (balancing, column breaks) are laid out; not supported yet (reported in [`Layout::notes`]):
//! right-to-left text, bidi and tight wrap polygons. Pictures in headers, footers and footnotes
//! are reported in [`Page::header_images`], [`Page::footer_images`] and [`Page::note_images`];
//! floating ones are positioned against the page but do not displace the text of those stories.
//!
//! Every glyph carries a [`Cluster`]: the characters it stands for (ligatures included), as
//! story offsets and as text of the paragraph ([`Line::cluster_text`]). Cells carry their table,
//! row and column ids and the borders and shading resolved for them. Lines carry the paragraph's
//! [`ParaRole`].

mod build;
mod cancel;
mod float;
mod flow;
mod frame;
mod furniture;
mod incremental;
mod metrics;
mod numbering;
mod para;
mod props;
mod rules;
mod table;
mod textbox;

use std::ops::Range;
use std::sync::Arc;

use open365_docx::{Block, Document, Paragraph};
use open365_text::{FaceId, FontLibrary, FontMatch};

pub use incremental::{IncrementalLayout, UpdateStats};
pub use metrics::LineMetricModel;
pub use props::{SectionProps, TabKind, TabLeader, TabStop};
pub use rules::LineBorder;

/// Knobs for layout. The defaults are the measured Word behaviour.
#[derive(Debug, Clone, Copy)]
pub struct LayoutOptions {
    pub metric_model: LineMetricModel,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            metric_model: LineMetricModel::Win,
        }
    }
}

/// A positioned glyph; `x` is in page coordinates (the glyph origin).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedGlyph {
    /// The face the glyph comes from: the run's face, or a fallback face that covers the
    /// character.
    pub face: FaceId,
    pub glyph_id: u32,
    pub x: f64,
    pub y_offset: f64,
    pub advance: f64,
    /// The source characters of the glyph.
    pub cluster: Cluster,
}

/// What a glyph's source characters are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterKind {
    /// Characters of the paragraph: `text_start..text_end` indexes [`Line::text`].
    Text,
    /// Characters of the list label: `text_start..text_end` indexes [`Line::label`]. The story
    /// range is empty.
    Label,
    /// A drawn rule with no characters of its own (the note separator).
    Other,
}

/// The characters a glyph was shaped from. A ligature is one glyph with a cluster of several
/// characters; when one cluster shapes to several glyphs, the first carries the text and the
/// others have an empty text range (and the same story range).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cluster {
    pub kind: ClusterKind,
    /// Story offsets (UTF-16 units) relative to [`Line::start`], so that moving a line moves its
    /// clusters.
    pub start: u32,
    pub end: u32,
    /// Byte range in [`Line::text`] (or [`Line::label`]).
    pub text_start: u32,
    pub text_end: u32,
}

/// What a paragraph is, for accessibility and tagged PDF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParaRole {
    #[default]
    Plain,
    /// A heading of level 1 to 6 (style `heading N`, through `basedOn`).
    Heading(u8),
    /// A numbered or bulleted paragraph.
    ListItem,
}

/// Consecutive glyphs of one face, size and style on a line.
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphRun {
    pub face: FaceId,
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub color: Option<[u8; 3]>,
    /// A note reference mark: drawn small and raised, on its own baseline in Word's PDF.
    pub superscript: bool,
    pub glyphs: Vec<PlacedGlyph>,
}

/// The story a line belongs to; the discriminants-as-codes are Word's `WdStoryType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StoryKind {
    #[default]
    Main,
    Footnotes,
    Endnotes,
    EvenHeader,
    PrimaryHeader,
    EvenFooter,
    PrimaryFooter,
    FirstHeader,
    FirstFooter,
    FootnoteSeparator,
    EndnoteSeparator,
    /// A text box (`wdTextFrameStory`): one story per box, offsets from 0; see [`Line::frame`].
    TextBox,
}

impl StoryKind {
    /// Word's `WdStoryType` value.
    pub fn word_code(self) -> u8 {
        match self {
            Self::Main => 1,
            Self::Footnotes => 2,
            Self::Endnotes => 3,
            Self::EvenHeader => 6,
            Self::PrimaryHeader => 7,
            Self::EvenFooter => 8,
            Self::PrimaryFooter => 9,
            Self::FirstHeader => 10,
            Self::FirstFooter => 11,
            Self::FootnoteSeparator => 12,
            Self::EndnoteSeparator => 15,
            Self::TextBox => 5,
        }
    }
}

/// One line of a story on a page (the main story unless [`Line::story`] says otherwise).
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// The story the offsets `start` and `end` count in.
    pub story: StoryKind,
    /// The text box the line is in (its [`FrameBox::id`]), for lines of [`StoryKind::TextBox`].
    pub frame: Option<usize>,
    /// Paragraph index in document order (tables included; in a text box, within the box).
    pub paragraph: usize,
    /// Story offset of the first character.
    pub start: usize,
    /// Story offset one past the last character.
    pub end: usize,
    /// Top of the line box.
    pub top: f64,
    /// Baseline.
    pub baseline: f64,
    pub height: f64,
    /// X of the first character's left edge.
    pub left: f64,
    /// X of the last character's left edge.
    pub last_x: f64,
    pub runs: Vec<GlyphRun>,
    /// Text of the paragraph the line belongs to (list label excluded, paragraph mark
    /// included); [`Cluster::text_start`] indexes it. Shared by the lines of a paragraph.
    pub text: Arc<str>,
    /// The list label text, on the first line of a numbered paragraph.
    pub label: Option<Arc<str>>,
    pub role: ParaRole,
    /// The paragraph's border box, when it has borders; [`Page::rules`] is derived from it.
    pub border: Option<LineBorder>,
}

impl Line {
    /// The story offsets (UTF-16 units) the glyph's cluster covers.
    pub fn cluster_story(&self, g: &PlacedGlyph) -> Range<usize> {
        self.start + g.cluster.start as usize..self.start + g.cluster.end as usize
    }

    /// The characters the glyph's cluster stands for; empty for a glyph that continues the
    /// previous glyph's cluster or has no characters.
    pub fn cluster_text(&self, g: &PlacedGlyph) -> &str {
        let c = g.cluster;
        let src: &str = match c.kind {
            ClusterKind::Text => &self.text,
            ClusterKind::Label => self.label.as_deref().unwrap_or(""),
            ClusterKind::Other => "",
        };
        src.get(c.text_start as usize..c.text_end as usize)
            .unwrap_or("")
    }

    /// X of the first drawn glyph's origin, or of the last character (the paragraph mark Word
    /// draws) when the line has no glyph, as a line of pictures only. A note reference mark is
    /// not counted: Word's PDF draws it as a row of its own.
    pub fn first_glyph_x(&self) -> f64 {
        self.runs
            .iter()
            .filter(|r| !r.superscript)
            .find_map(|r| r.glyphs.first())
            .map_or(self.last_x, |g| g.x)
    }
}

/// How a cell border is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderStyle {
    Solid,
    /// `double`, `triple` and the thin/thick pairs: two lines with a gap.
    Double,
    /// `dashed`, `dotDash` and the like.
    Dashed,
    Dotted,
}

/// A resolved border: the cell's own, else its table style's, else the table's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderLine {
    /// Line width in points.
    pub width: f64,
    pub color: [u8; 3],
    pub style: BorderStyle,
}

/// The four borders of a cell; `None` where nothing is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CellBorders {
    pub top: Option<BorderLine>,
    pub bottom: Option<BorderLine>,
    pub left: Option<BorderLine>,
    pub right: Option<BorderLine>,
}

/// The rectangle of a table cell (or a cell fragment, when its row continues on another
/// page) in page coordinates, with the cell's identity and look.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Document-order id of the table (a nested table follows its parent), counting the tables
    /// of the main story.
    pub table: usize,
    /// Row index in the table, and first grid column.
    pub row: usize,
    pub col: usize,
    pub col_span: usize,
    pub row_span: usize,
    /// The row is a header row (`w:tblHeader`).
    pub header_row: bool,
    /// Fill colour (shading resolved to a solid colour), `None` for no fill.
    pub fill: Option<[u8; 3]>,
    pub borders: CellBorders,
    /// Top and bottom cell margins in points. The box is the cell as laid out; Word's PDF clips
    /// the cell's content to the box inset by these (`layout-table-basic`).
    pub pad_top: f64,
    pub pad_bottom: f64,
}

/// Lines `from..to` of footnote `id`, placed on one page. A footnote that does not fit goes on
/// in the next page's first slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoteSlot {
    pub id: i64,
    pub from: usize,
    pub to: usize,
}

/// A picture on a page, in page coordinates (points, y down). The rectangle is the picture
/// itself: Word's effect extent and wrapping distances are not included.
#[derive(Debug, Clone, PartialEq)]
pub struct ImagePlacement {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Relationship id of the image part (`a:blip/@r:embed`); resolve it with
    /// `Document::image_part`.
    pub embed: Option<String>,
    /// Alternative text (`wp:docPr/@descr`).
    pub description: Option<String>,
    /// Document-order index of the paragraph the picture belongs to.
    pub paragraph: usize,
    /// True for a floating (anchored) picture, false for one in the line of text.
    pub anchored: bool,
    /// A floating picture drawn behind the text.
    pub behind_text: bool,
    /// Which part's relationships `embed` belongs to.
    pub source: ImageSource,
    /// The shape is a text box: [`Page::frames`] holds its text.
    pub(crate) text_box: Option<textbox::BoxRef>,
}

/// The package part whose relationship table resolves a picture's `embed` id.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ImageSource {
    /// The main document part (`Document::image_part`).
    #[default]
    Main,
    /// The header or footer part that relationship `rel_id` of the main part names.
    HeaderFooter(String),
    Footnotes,
    Endnotes,
}

/// A text box on a page: Word's clip rectangle of its text, and the lines in it (story
/// [`StoryKind::TextBox`], offsets from 0 in the box's own story).
#[derive(Debug, Clone, PartialEq)]
pub struct FrameBox {
    /// Index of the box in anchor order over the whole document (`StoryRanges(5)` order).
    pub id: usize,
    /// Document-order index of the paragraph the box is anchored in.
    pub paragraph: usize,
    /// The rectangle the text is clipped to, page coordinates (points, y down).
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub lines: Vec<Line>,
}

/// One page of the main story.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    /// 1-based page number.
    pub number: usize,
    pub section: SectionProps,
    pub lines: Vec<Line>,
    /// Table cell rectangles on this page, in drawing order.
    pub cells: Vec<CellBox>,
    /// Pictures on this page, in drawing order.
    pub images: Vec<ImagePlacement>,
    /// Text boxes on this page, in anchor order.
    pub frames: Vec<FrameBox>,
    /// The areas floating pictures keep text out of (see `float.rs`).
    pub(crate) floats: Vec<float::Float>,
    /// Index of the page's section.
    pub section_index: usize,
    /// Whether this is the first page of its section (`titlePg` pages).
    pub first_in_section: bool,
    /// Top and bottom of the body area: the margins, moved by a tall header or footer.
    pub body_top: f64,
    pub body_bottom: f64,
    /// The footnotes (or the parts of footnotes) placed on this page, in order.
    pub note_slots: Vec<NoteSlot>,
    /// The footnote separator and footnote lines, at the bottom of the body area.
    pub notes: Vec<Line>,
    /// Header lines, top of the page.
    pub header: Vec<Line>,
    /// Footer lines, bottom of the page.
    pub footer: Vec<Line>,
    /// Inline pictures of the header, footer and footnote lines, in page coordinates.
    pub header_images: Vec<ImagePlacement>,
    pub footer_images: Vec<ImagePlacement>,
    pub note_images: Vec<ImagePlacement>,
    /// Paragraph border rules of every story on the page (header, body, cells, notes, footer),
    /// in drawing order.
    pub rules: Vec<Rule>,
}

impl Page {
    /// The rectangles (`[x, y, right, bottom]`) around floating tables: the table grown by its
    /// left and right text distances (not the top and bottom ones, measured). Word reports each
    /// as one more clip rectangle.
    pub fn table_wrap_zones(&self) -> Vec<[f64; 4]> {
        self.floats
            .iter()
            .filter(|f| f.table || f.frame)
            .map(|f| {
                let s = flow::snap_edge;
                [s(f.x0), s(f.rect[1]), s(f.x1), s(f.rect[3])]
            })
            .collect()
    }
}

/// The edge of a paragraph's border box a [`Rule`] belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleSide {
    Top,
    Bottom,
    Left,
    Right,
    /// The rule between two paragraphs of one box (`w:between`).
    Between,
}

/// A filled rectangle of a paragraph border, in page coordinates (points, y down). A composite
/// style (`double`, `triple`, thin-thick pairs) is several rules; a dotted or dashed style is
/// one rule the renderer may dash. Edges lie on Word's 0.24 pt drawing grid.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub color: [u8; 3],
    pub style: BorderStyle,
    pub side: RuleSide,
    pub story: StoryKind,
    /// Document-order index of the first paragraph of the box.
    pub paragraph: usize,
}

/// A font family the document asked for and how the library satisfied it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontUse {
    pub requested: String,
    pub matched: FontMatch,
}

/// The laid-out document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    pub pages: Vec<Page>,
    /// Every family the main story's text and paragraph marks use, sorted by name.
    pub fonts: Vec<FontUse>,
    /// Features present in the document that layout does not model yet.
    pub notes: Vec<String>,
}

impl Layout {
    /// All body lines in reading order.
    pub fn lines(&self) -> impl Iterator<Item = (usize, &Line)> {
        self.pages
            .iter()
            .flat_map(|p| p.lines.iter().map(move |l| (p.number, l)))
    }

    /// Every line of every story on every page: header, body, notes, footer.
    pub fn all_lines(&self) -> impl Iterator<Item = (usize, &Line)> {
        self.pages.iter().flat_map(|p| {
            p.header
                .iter()
                .chain(&p.lines)
                .chain(p.frames.iter().flat_map(|f| f.lines.iter()))
                .chain(&p.notes)
                .chain(&p.footer)
                .map(move |l| (p.number, l))
        })
    }
}

struct Item<'a> {
    para: &'a Paragraph,
    /// Extra story characters after this paragraph (a table row end mark).
    extra_chars: usize,
}

/// Paragraphs in document order, the order `Document::sections` indexes them in.
fn collect_items<'a>(blocks: &'a [Block], items: &mut Vec<Item<'a>>) {
    for block in blocks {
        match block {
            Block::Paragraph(p) => items.push(Item {
                para: p,
                extra_chars: 0,
            }),
            Block::Table(t) => {
                for row in t.rows() {
                    for cell in row.cells() {
                        collect_items(&cell.blocks, items);
                    }
                    if let Some(last) = items.last_mut() {
                        last.extra_chars += 1;
                    }
                }
            }
            Block::Sdt(_) | Block::Raw(_) => {}
        }
    }
}

/// The list label of every paragraph in document order (tables included), `None` for a
/// paragraph that is not numbered. Bullets carry their symbol character as written in the
/// numbering definition (often a private-use code point of the Symbol font).
pub fn list_labels(doc: &Document) -> Vec<Option<String>> {
    let mut labels: Vec<Option<String>> = incremental::all_labels(doc)
        .into_iter()
        .map(|l| l.map(|l| l.text))
        .collect();
    if labels.is_empty() {
        // No numbering part: every paragraph is unnumbered.
        labels.resize(doc.paragraph_count(), None);
    }
    labels
}

/// Lays out the main story of `doc` with `fonts`.
///
/// Returns an empty layout when `fonts` has no faces. To keep a layout up to date while the
/// document is edited use [`IncrementalLayout`], which gives the same result faster.
pub fn layout_document(doc: &Document, fonts: &FontLibrary, options: LayoutOptions) -> Layout {
    let mut layout = IncrementalLayout::new();
    layout.update(doc, fonts, options);
    layout.into_layout()
}

/// As [`layout_document`], but stops soon after `cancel` is set and returns `None`. The flag is
/// looked at once per block and once per item placed on a page, so a cancelled run ends within
/// one block's layout. Nothing partial is returned.
pub fn layout_document_cancellable(
    doc: &Document,
    fonts: &FontLibrary,
    options: LayoutOptions,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Option<Layout> {
    let _scope = cancel::Scope::enter(cancel);
    let mut layout = IncrementalLayout::new();
    layout.update(doc, fonts, options);
    if cancel::cancelled() {
        return None;
    }
    Some(layout.into_layout())
}
