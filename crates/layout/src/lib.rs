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
//! paragraphs, and images, numbering, fields, footnotes, headers, footers, columns,
//! right-to-left text and bidi are ignored.

mod flow;
mod metrics;
mod para;
mod props;

use open365_docx::{Block, Document, Paragraph, RawXml, Styles};
use open365_text::{FaceId, FontLibrary};

pub use metrics::LineMetricModel;
pub use props::{SectionProps, TabKind, TabLeader, TabStop};

use para::{Context, ParaLayout, layout_paragraph};
use props::{DocExtras, paragraph_section};

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

/// One page of the main story.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    /// 1-based page number.
    pub number: usize,
    pub section: SectionProps,
    pub lines: Vec<Line>,
}

/// The laid-out document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    pub pages: Vec<Page>,
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
    section: usize,
    /// Extra story characters after this paragraph (a table row end mark).
    extra_chars: usize,
}

fn collect_items<'a>(
    blocks: &'a [Block],
    section: &mut usize,
    sections: &mut Vec<Option<SectionProps>>,
    items: &mut Vec<Item<'a>>,
    notes: &mut Vec<String>,
    in_table: bool,
) {
    for block in blocks {
        match block {
            Block::Paragraph(p) => {
                items.push(Item {
                    para: p,
                    section: *section,
                    extra_chars: 0,
                });
                if !in_table && let Some(props) = paragraph_section(p) {
                    sections[*section] = Some(props);
                    *section += 1;
                    sections.push(None);
                }
            }
            Block::Table(t) => {
                if !notes.iter().any(|n| n.starts_with("tables")) {
                    notes.push("tables are laid out as sequential paragraphs".to_owned());
                }
                for row in t.rows() {
                    for cell in row.cells() {
                        collect_items(&cell.blocks, section, sections, items, notes, true);
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

fn body_section(doc: &Document) -> Option<SectionProps> {
    doc.body().blocks.iter().rev().find_map(|b| match b {
        Block::Raw(RawXml::Element(e)) if e.head.is_w("sectPr") => {
            Some(SectionProps::from_element(e))
        }
        _ => None,
    })
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
    let extras = DocExtras::new(doc);
    let styles: Option<&Styles> = doc.styles();
    let ctx = Context {
        fonts,
        styles,
        extras: &extras,
        model: options.metric_model,
    };

    let mut items: Vec<Item<'_>> = Vec::new();
    let mut sections: Vec<Option<SectionProps>> = vec![None];
    let mut section = 0usize;
    collect_items(
        &doc.body().blocks,
        &mut section,
        &mut sections,
        &mut items,
        &mut layout.notes,
        false,
    );
    let last = sections.len() - 1;
    if sections[last].is_none() {
        sections[last] = Some(body_section(doc).unwrap_or_default());
    }
    let sections: Vec<SectionProps> = sections
        .into_iter()
        .map(std::option::Option::unwrap_or_default)
        .collect();

    let mut laid: Vec<(ParaLayout, usize)> = Vec::with_capacity(items.len());
    let mut story = 0usize;
    for (index, item) in items.iter().enumerate() {
        let width = sections[item.section].text_width();
        let pl = layout_paragraph(&ctx, item.para, index, story, width);
        story = pl.story_end + item.extra_chars;
        laid.push((pl, item.section));
    }

    layout.pages = flow::paginate(&ctx, &laid, &sections);
    layout
}
