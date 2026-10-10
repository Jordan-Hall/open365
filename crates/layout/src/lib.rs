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
//! Not supported yet (reported in [`Layout::notes`]): tables are laid out as sequential
//! paragraphs, and images, fields, footnotes, headers, footers, columns,
//! right-to-left text and bidi are ignored.

mod flow;
mod metrics;
mod numbering;
mod para;
mod props;

use open365_docx::{Block, Document, Paragraph, Styles};
use open365_text::{FaceId, FontLibrary, FontMatch};

pub use metrics::LineMetricModel;
pub use props::{SectionProps, TabKind, TabLeader, TabStop};

use numbering::{Numbering, NumberingState};
use para::{Context, ParaLayout, layout_paragraph};

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
    pub glyphs: Vec<PlacedGlyph>,
}

/// One line of the main story on a page.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
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
    /// X of the first drawn glyph's origin, or of the line start when nothing is drawn.
    pub fn first_glyph_x(&self) -> f64 {
        self.runs
            .iter()
            .find_map(|r| r.glyphs.first())
            .map_or(self.left, |g| g.x)
    }
}

/// One page of the main story.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    /// 1-based page number.
    pub number: usize,
    pub section: SectionProps,
    pub lines: Vec<Line>,
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
    /// All lines in reading order.
    pub fn lines(&self) -> impl Iterator<Item = (usize, &Line)> {
        self.pages
            .iter()
            .flat_map(|p| p.lines.iter().map(move |l| (p.number, l)))
    }
}

struct Item<'a> {
    para: &'a Paragraph,
    /// Extra story characters after this paragraph (a table row end mark).
    extra_chars: usize,
}

/// Paragraphs in document order, the order `Document::sections` indexes them in.
fn collect_items<'a>(blocks: &'a [Block], items: &mut Vec<Item<'a>>, notes: &mut Vec<String>) {
    for block in blocks {
        match block {
            Block::Paragraph(p) => items.push(Item {
                para: p,
                extra_chars: 0,
            }),
            Block::Table(t) => {
                if !notes.iter().any(|n| n.starts_with("tables")) {
                    notes.push("tables are laid out as sequential paragraphs".to_owned());
                }
                for row in t.rows() {
                    for cell in row.cells() {
                        collect_items(&cell.blocks, items, notes);
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
    let mut items: Vec<Item<'_>> = Vec::new();
    collect_items(&doc.body().blocks, &mut items, &mut Vec::new());
    let numbering = Numbering::new(doc);
    let mut state = NumberingState::default();
    items
        .iter()
        .map(|i| {
            numbering
                .as_ref()
                .and_then(|n| n.label(&mut state, i.para))
                .map(|l| l.text)
        })
        .collect()
}

/// Lays out the main story of `doc` with `fonts`.
///
/// Returns an empty layout when `fonts` has no faces.
pub fn layout_document(doc: &Document, fonts: &FontLibrary, options: LayoutOptions) -> Layout {
    let mut layout = Layout::default();
    if fonts.is_empty() {
        layout.notes.push("font library is empty".to_owned());
        return layout;
    }
    let styles: Option<&Styles> = doc.styles();
    let ctx = Context {
        fonts,
        styles,
        default_tab: f64::from(doc.settings().default_tab_stop_twips) / 20.0,
        model: options.metric_model,
    };

    let mut items: Vec<Item<'_>> = Vec::new();
    collect_items(&doc.body().blocks, &mut items, &mut layout.notes);
    let doc_sections = doc.sections();
    let sections: Vec<SectionProps> = doc_sections.iter().map(SectionProps::from).collect();
    for (i, s) in doc_sections.iter().enumerate() {
        if s.columns.count > 1 && !layout.notes.iter().any(|n| n.starts_with("columns")) {
            layout
                .notes
                .push(format!("columns are ignored (section {i})"));
        }
    }

    let mut laid: Vec<(ParaLayout, usize)> = Vec::with_capacity(items.len());
    let mut story = 0usize;
    let mut section = 0usize;
    let numbering = Numbering::new(doc);
    let mut numbers = NumberingState::default();
    for (index, item) in items.iter().enumerate() {
        let width = sections[section].text_width();
        let label = numbering
            .as_ref()
            .and_then(|n| n.label(&mut numbers, item.para));
        let pl = layout_paragraph(&ctx, item.para, index, story, width, label.as_ref());
        story = pl.story_end + item.extra_chars;
        laid.push((pl, section));
        // The paragraph that carries a `w:sectPr` is the last of its section.
        if doc_sections[section].end_paragraph == Some(index) {
            section += 1;
        }
    }

    let mut requested: Vec<&str> = laid
        .iter()
        .flat_map(|(pl, _)| pl.styles.iter().map(|s| s.requested.as_str()))
        .collect();
    requested.sort_unstable();
    requested.dedup();
    layout.fonts = requested
        .into_iter()
        .map(|r| FontUse {
            requested: r.to_owned(),
            matched: fonts.match_kind(r),
        })
        .collect();

    layout.pages = flow::paginate(&ctx, &laid, &sections);
    layout
}
