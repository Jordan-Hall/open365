//! Text boxes (`wps:txbx`): the `w:txbxContent` of a shape is a story of its own, laid out at the
//! shape's width less the insets and placed inside the shape. Rules measured with
//! `fixtures/docx/layout-txbx-*` (see `docs/architecture/word-layout.md`).

use std::sync::Arc;

use open365_docx::{EMU_PER_POINT, TableCellFormat, TextBox, TextBoxAnchor};

use crate::build::Builder;
use crate::flow::{Base, build_line};
use crate::para::{Context, ParaLayout};
use crate::table::{FlowLine, Pad, layout_cell};
use crate::{FrameBox, ImagePlacement, Line, Page, StoryKind};

/// Width the lines of a `wrap="none"` box are laid out at: they never break.
const UNBOUNDED: f64 = 10_000.0;

/// The content of a text box laid out, relative to the content area (the shape inside its
/// insets).
#[derive(Debug)]
pub(crate) struct BoxLayout {
    arena: Vec<ParaLayout>,
    lines: Vec<FlowLine>,
    content_h: f64,
    /// Width of the widest line (the left edge of its last character).
    text_w: f64,
    shape: Arc<TextBox>,
    width: f64,
}

/// A shared, comparable handle on a laid-out text box, carried by the shape's placement.
#[derive(Debug, Clone)]
pub(crate) struct BoxRef(Arc<BoxLayout>);

impl PartialEq for BoxRef {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
            || (self.0.shape == other.0.shape && self.0.width.to_bits() == other.0.width.to_bits())
    }
}

impl BoxRef {
    /// Lays out `tb` in a shape `shape_w` wide.
    pub(crate) fn new(ctx: &Context<'_>, tb: &Arc<TextBox>, shape_w: f64) -> Self {
        let [l, _, r, _] = tb.insets_pt();
        let width = if tb.wrap {
            (shape_w - l - r).max(1.0)
        } else {
            UNBOUNDED
        };
        let mut builder = Builder::new(*ctx, &[], 0);
        let fmt = TableCellFormat::default();
        let content = layout_cell(&mut builder, &tb.blocks, width, Pad::default(), &fmt);
        let (lines, content_h) = (content.lines, content.height);
        let text_w = lines
            .iter()
            .map(|ln| ln.dx + builder.arena[ln.para].lines[ln.line].last_x)
            .fold(0.0, f64::max);
        Self(Arc::new(BoxLayout {
            arena: builder.arena,
            lines,
            content_h,
            text_w,
            shape: tb.clone(),
            width: shape_w,
        }))
    }

    /// The frame for the shape at `im`; the id is set by [`apply`].
    fn place(&self, im: &ImagePlacement, paragraph: usize) -> FrameBox {
        let b = &*self.0;
        let [l, t, r, bt] = b.shape.insets_pt();
        let ln = b.shape.line_emu as f64 / EMU_PER_POINT;
        let avail = (im.height - t - bt).max(0.0);
        let shift = match b.shape.anchor {
            TextBoxAnchor::Top => 0.0,
            TextBoxAnchor::Middle => ((avail - b.content_h) / 2.0).max(0.0),
            TextBoxAnchor::Bottom => (avail - b.content_h).max(0.0),
        };
        let base = Base {
            para: 0,
            story: 0,
            table: 0,
        };
        let lines: Vec<Line> = b
            .lines
            .iter()
            .map(|f| {
                let para = &b.arena[f.para];
                let mut line = build_line(
                    para,
                    &para.lines[f.line],
                    im.x + l + f.dx,
                    im.y + t + shift + f.dy,
                    base,
                );
                line.story = StoryKind::TextBox;
                line
            })
            .collect();
        let width = if b.shape.wrap {
            im.width
        } else {
            l + b.text_w + r
        };
        FrameBox {
            id: 0,
            paragraph,
            x: im.x + ln / 2.0,
            y: im.y + t + ln / 2.0,
            width: width - ln,
            height: (im.height - t - bt - ln).max(0.0),
            lines,
        }
    }
}

/// Builds the frames of every page from the text box shapes on it and numbers them in anchor
/// order (document order of the anchoring paragraphs), the order of Word's
/// `StoryRanges(wdTextFrameStory)`.
pub(crate) fn apply(pages: &mut [Page]) {
    let mut found: Vec<(usize, usize, FrameBox)> = Vec::new();
    for (pi, page) in pages.iter_mut().enumerate() {
        page.frames.clear();
        for im in &page.images {
            if let Some(bx) = &im.text_box {
                found.push((pi, found.len(), bx.place(im, im.paragraph)));
            }
        }
    }
    found.sort_by_key(|(_, seq, f)| (f.paragraph, *seq));
    for (id, (pi, _, mut frame)) in found.into_iter().enumerate() {
        frame.id = id;
        for line in &mut frame.lines {
            line.frame = Some(id);
        }
        pages[pi].frames.push(frame);
    }
}
