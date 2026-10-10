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
//! other field keeps its cached result. Not supported yet (reported in [`Layout::notes`]):
//! images, columns, right-to-left text and bidi are ignored.

mod build;
mod flow;
mod furniture;
mod incremental;
mod metrics;
mod numbering;
mod para;
mod props;
mod table;

use open365_docx::{Block, Document, Paragraph};
use open365_text::{FaceId, FontLibrary, FontMatch};

pub use incremental::{IncrementalLayout, UpdateStats};
pub use metrics::LineMetricModel;
pub use props::{SectionProps, TabKind, TabLeader, TabStop};

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
        }
    }
}

/// One line of a story on a page (the main story unless [`Line::story`] says otherwise).
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// The story the offsets `start` and `end` count in.
    pub story: StoryKind,
    /// Paragraph index in document order (tables included).
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
}

impl Line {
    /// X of the first drawn glyph's origin, or of the line start when nothing is drawn. A note
    /// reference mark is not counted: Word's PDF draws it as a row of its own.
    pub fn first_glyph_x(&self) -> f64 {
        self.runs
            .iter()
            .filter(|r| !r.superscript)
            .find_map(|r| r.glyphs.first())
            .map_or(self.left, |g| g.x)
    }
}

/// The rectangle of a table cell (or a cell fragment, when its row continues on another
/// page) in page coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Lines `from..to` of footnote `id`, placed on one page. A footnote that does not fit goes on
/// in the next page's first slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoteSlot {
    pub id: i64,
    pub from: usize,
    pub to: usize,
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
            Block::Raw(_) => {}
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
