//! Document-order construction state: lays out every paragraph (body and table cells) with
//! consecutive indices, story offsets and list numbering.

use open365_docx::{Document, Paragraph, TableCellFormat};

use crate::numbering::{Labeler, NumberingState};
use crate::para::{Context, ParaLayout, layout_paragraph};

pub(crate) struct Builder<'a> {
    pub ctx: Context<'a>,
    /// Every laid-out paragraph, in document order.
    pub arena: Vec<ParaLayout>,
    /// Story offset of the next paragraph.
    pub story: usize,
    labeler: Option<Labeler<'a>>,
    numbers: NumberingState,
}

impl<'a> Builder<'a> {
    pub(crate) fn new(doc: &'a Document, ctx: Context<'a>) -> Self {
        Self {
            ctx,
            arena: Vec::new(),
            story: 0,
            labeler: Labeler::new(doc),
            numbers: NumberingState::default(),
        }
    }

    /// Lays out `para` at `width`; returns its index in [`Builder::arena`].
    pub(crate) fn paragraph(
        &mut self,
        para: &Paragraph,
        width: f64,
        table: Option<&TableCellFormat>,
    ) -> usize {
        let label = self
            .labeler
            .as_ref()
            .and_then(|l| l.label(&mut self.numbers, para));
        let ctx = Context { table, ..self.ctx };
        let index = self.arena.len();
        let pl = layout_paragraph(&ctx, para, index, self.story, width, label.as_ref());
        self.story = pl.story_end;
        self.arena.push(pl);
        index
    }
}
