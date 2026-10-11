//! Construction of layout units: a body paragraph or a whole table, each laid out on its own
//! with story offsets and paragraph indices relative to the unit, so a unit can be cached and
//! reused wherever the same block appears again at the same width.

use open365_docx::{Block, Paragraph, TableCellFormat};

use crate::frame::{FramePr, FrameSpec, border_pads};
use crate::numbering::ListLabel;
use crate::para::{AtomClass, Context, ParaLayout, layout_paragraph};
use crate::table::{TableLayout, layout_table};

/// Document-order construction state of one unit.
pub(crate) struct Builder<'a> {
    pub ctx: Context<'a>,
    /// Every laid-out paragraph of the unit, in document order.
    pub arena: Vec<ParaLayout>,
    /// Story offset of the next paragraph, relative to the start of the unit.
    pub story: usize,
    /// List labels of the whole document, by paragraph in document order.
    labels: &'a [Option<ListLabel>],
    /// Document-order index of the next paragraph.
    next: usize,
    /// Tables started so far in this unit (the next table's local id).
    pub tables: usize,
}

impl<'a> Builder<'a> {
    pub(crate) fn new(
        ctx: Context<'a>,
        labels: &'a [Option<ListLabel>],
        first_paragraph: usize,
    ) -> Self {
        Self {
            ctx,
            arena: Vec::new(),
            story: 0,
            labels,
            next: first_paragraph,
            tables: 0,
        }
    }

    /// Lays out `para` at `width`; returns its index in [`Builder::arena`].
    pub(crate) fn paragraph(
        &mut self,
        para: &Paragraph,
        width: f64,
        table: Option<&TableCellFormat>,
    ) -> usize {
        let labels = self.labels;
        let label = labels.get(self.next).and_then(Option::as_ref);
        self.next += 1;
        let ctx = Context { table, ..self.ctx };
        let index = self.arena.len();
        let pl = layout_paragraph(&ctx, para, index, self.story, width, label);
        self.story = pl.story_end;
        self.arena.push(pl);
        index
    }

    /// Lays out the paragraph of a frame in its text box, whose width is `w:w` or, without one,
    /// the text's natural width up to `width` (a drop cap is as wide as its letter).
    pub(crate) fn frame_paragraph(
        &mut self,
        para: &Paragraph,
        pr: FramePr,
        width: f64,
    ) -> FrameSpec {
        let (next, story) = (self.next, self.story);
        let fixed = pr.width.map(|w| w.max(1.0));
        let index = self.paragraph(para, fixed.unwrap_or(width), None);
        let pad = border_pads(para, &self.arena[index].flow.borders);
        if let Some(w) = fixed {
            return FrameSpec { pr, box_w: w, pad };
        }
        let box_w = natural_width(&self.arena[index], width);
        if box_w >= width - 0.01 {
            return FrameSpec {
                pr,
                box_w: width,
                pad,
            };
        }
        // Laid out again at the natural width (a hair more, so that no word wraps).
        self.arena.pop();
        (self.next, self.story) = (next, story);
        self.paragraph(para, box_w + 0.01, None);
        FrameSpec { pr, box_w, pad }
    }
}

/// The right edge of the widest line of `pl`: the width an automatic-width frame takes.
fn natural_width(pl: &ParaLayout, width: f64) -> f64 {
    // Text that wraps fills the column; only forced breaks leave shorter lines.
    let wrapped = pl.lines.split_last().is_some_and(|(_, head)| {
        head.iter().any(|lb| {
            !pl.atoms
                .get(lb.atoms.end.wrapping_sub(1))
                .is_some_and(|a| matches!(a.class, AtomClass::LineBreak | AtomClass::PageBreak))
        })
    });
    if wrapped {
        return width;
    }
    pl.lines
        .iter()
        .map(|lb| {
            lb.atoms
                .clone()
                .enumerate()
                .filter(|(_, ai)| {
                    matches!(pl.atoms[*ai].class, AtomClass::Text | AtomClass::Object)
                })
                .map(|(k, ai)| lb.xs.get(k).copied().unwrap_or(lb.left) + pl.atoms[ai].adv)
                .fold(lb.left, f64::max)
        })
        .fold(0.0, f64::max)
}

/// A body paragraph or a table, laid out independently of the page.
#[derive(Debug)]
pub(crate) struct UnitLayout {
    /// Paragraphs of the unit (one for a body paragraph, every cell paragraph of a table).
    pub arena: Vec<ParaLayout>,
    pub table: Option<TableLayout>,
    /// Characters of the story the unit covers (row end marks included).
    pub story_len: usize,
    /// Tables in the unit, nested ones included.
    pub tables: usize,
    /// The list labels the unit was laid out with.
    pub labels: Vec<Option<ListLabel>>,
    /// Requested font families, sorted and unique.
    pub fonts: Vec<String>,
    /// Width the unit was laid out at.
    pub width: f64,
    /// The unit is a paragraph with a floating frame (`w:framePr`), laid out in its text box.
    pub frame: Option<FrameSpec>,
}

impl UnitLayout {
    /// Whether the unit was laid out with exactly these labels (`labels` starts at the unit's
    /// first paragraph).
    pub(crate) fn labels_match(&self, labels: &[Option<ListLabel>]) -> bool {
        (0..self.arena.len()).all(|i| {
            self.labels.get(i).and_then(Option::as_ref) == labels.get(i).and_then(Option::as_ref)
        })
    }

    pub(crate) fn para(&self) -> Option<&ParaLayout> {
        if self.table.is_some() {
            None
        } else {
            self.arena.first()
        }
    }
}

/// Lays out `block` (a paragraph or a table) whose first paragraph has document-order index
/// `first_paragraph`.
pub(crate) fn build_unit(
    block: &Block,
    ctx: Context<'_>,
    labels: &[Option<ListLabel>],
    first_paragraph: usize,
    width: f64,
) -> UnitLayout {
    let mut b = Builder::new(ctx, labels, first_paragraph);
    let mut frame = None;
    let table = match block {
        Block::Paragraph(p) => {
            match FramePr::read(p) {
                Some(pr) => frame = Some(b.frame_paragraph(p, pr, width)),
                None => {
                    b.paragraph(p, width, None);
                }
            }
            None
        }
        Block::Table(t) => Some(layout_table(&mut b, t, width)),
        Block::Sdt(_) | Block::Raw(_) => None,
    };
    let story_len = b.story;
    let tables = b.tables;
    let arena = b.arena;
    let mut fonts: Vec<String> = arena
        .iter()
        .flat_map(|pl| pl.styles.iter().map(|s| s.requested.clone()))
        .collect();
    fonts.sort_unstable();
    fonts.dedup();
    let end = (first_paragraph + arena.len()).min(labels.len());
    let start = first_paragraph.min(end);
    let used = &labels[start..end];
    UnitLayout {
        arena,
        table,
        story_len,
        tables,
        labels: if used.iter().any(Option::is_some) {
            used.to_vec()
        } else {
            Vec::new()
        },
        fonts,
        width,
        frame,
    }
}
