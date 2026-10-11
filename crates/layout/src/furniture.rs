//! Page furniture: headers and footers (ECMA-376 Part 1 §17.10) and the footnote area.
//!
//! Measured against Word (layout-hf-*, layout-fn-*):
//! * The header's first line sits at the header distance from the page top and stacks down;
//!   the footer is bottom aligned, its last line (and the space after it) ending at the footer
//!   distance above the page bottom.
//! * The body starts at the larger of the top margin and the bottom of the header (space after
//!   the last header paragraph included); it ends at the smaller of the bottom margin and the top
//!   of the footer.
//! * A header or footer kind a section does not name is inherited from the section before.
//!   The first page of a section with `titlePg` uses the first kind, even pages use the even
//!   kind when `evenAndOddHeaders` is set.
//! * The footnote area is bottom aligned to the body bottom: the separator paragraph, then the
//!   notes of the page's references in order. It takes room from the body.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use open365_docx::{Block, Document, HeaderFooterType, Inline, NoteKind, RawXml, RunContent};

use crate::build::Builder;
use crate::float::{self, Frame as FloatFrame};
use crate::flow::{Base, build_line, line_images};
use crate::numbering::ListLabel;
use crate::para::{Context, FieldEnv, NoteOrder, ParaLayout};
use crate::props::SectionProps;
use crate::rules::rules_of;
use crate::{ImagePlacement, ImageSource, Line, NoteSlot, Page, StoryKind};

/// Where the body goes on a page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Frame {
    pub top: f64,
    pub bottom: f64,
}

type Blocks = Option<Arc<Vec<Block>>>;

/// Header and footer stories of every section, and the heights they take.
#[derive(Debug, Default)]
pub(crate) struct Furniture {
    sections: Vec<SectionProps>,
    title_pg: Vec<bool>,
    even_odd: bool,
    /// `[section][footer][kind]`, kind 0 default, 1 first, 2 even; inheritance resolved.
    stories: Vec<[[Blocks; 3]; 2]>,
    /// The relationship id of each story's part, parallel to `stories`.
    rels: Vec<[[Option<String>; 3]; 2]>,
    heights: Vec<[[f64; 3]; 2]>,
}

fn kind_index(t: HeaderFooterType) -> usize {
    match t {
        HeaderFooterType::Default => 0,
        HeaderFooterType::First => 1,
        HeaderFooterType::Even => 2,
    }
}

/// Lays out `blocks` (paragraphs only) at `width`.
fn layout_blocks(ctx: &Context<'_>, blocks: &[Block], width: f64) -> Vec<ParaLayout> {
    let labels: [Option<ListLabel>; 0] = [];
    let mut b = Builder::new(*ctx, &labels, 0);
    for block in blocks {
        if let Block::Paragraph(p) = block {
            b.paragraph(p, width, None);
        }
    }
    b.arena
}

/// Lines placed in a furniture story, the inline pictures on them (with the index of their
/// line) and the height used.
struct Placed {
    lines: Vec<Line>,
    images: Vec<(usize, ImagePlacement)>,
    height: f64,
}

/// Places paragraphs one below the other from `top`: the lines and the height used, the last
/// paragraph's space after included.
fn place(
    paras: &[ParaLayout],
    origin_x: f64,
    top: f64,
    story: StoryKind,
    source: &ImageSource,
    section: Option<&SectionProps>,
) -> Placed {
    let mut y = top;
    let mut after = 0.0f64;
    let mut lines = Vec::new();
    let mut images = Vec::new();
    let base = Base {
        para: 0,
        story: 0,
        table: 0,
    };
    for (k, pl) in paras.iter().enumerate() {
        let joined_prev = k > 0 && paras[k - 1].joins(pl);
        let joined_next = paras.get(k + 1).is_some_and(|q| pl.joins(q));
        y += after.max(f64::from(pl.props.space_before_pt));
        y += if joined_prev {
            pl.between_extent()
        } else {
            pl.border_top()
        };
        // Floating pictures sit against the page; they do not displace the text of a furniture
        // story.
        if let Some(sp) = section {
            let frame = FloatFrame { section: *sp };
            for o in pl.objects.iter().filter(|o| !o.inline()) {
                let mut im = float::place(&o.drawing, o.atom, &frame, y, 0, None, None, None).image;
                im.source = source.clone();
                // A shape without a picture or text has nothing to draw here.
                if im.embed.is_some() || im.text_box.is_some() {
                    images.push((lines.len(), im));
                }
            }
        }
        for lb in &pl.lines {
            let mut line = build_line(pl, lb, origin_x, y, base);
            line.story = story;
            line.border = pl.line_border(lb, origin_x, joined_prev, joined_next);
            for mut im in line_images(pl, lb, origin_x, y, base) {
                im.source = source.clone();
                images.push((lines.len(), im));
            }
            lines.push(line);
            y += lb.height;
        }
        if !joined_next {
            y += pl.border_bottom();
        }
        after = f64::from(pl.props.space_after_pt);
    }
    Placed {
        lines,
        images,
        height: y + after - top,
    }
}

fn height_of(paras: &[ParaLayout]) -> f64 {
    place(paras, 0.0, 0.0, StoryKind::Main, &ImageSource::Main, None).height
}

impl Furniture {
    pub(crate) fn new(
        doc: &Document,
        doc_sections: &[open365_docx::Section],
        sections: &[SectionProps],
        ctx: &Context<'_>,
    ) -> Self {
        let mut stories: Vec<[[Blocks; 3]; 2]> = Vec::new();
        let mut current: [[Blocks; 3]; 2] = Default::default();
        let mut rels: Vec<[[Option<String>; 3]; 2]> = Vec::new();
        let mut current_rels: [[Option<String>; 3]; 2] = Default::default();
        for s in doc_sections {
            for r in &s.header_footer_refs {
                let blocks = doc.header_footer_blocks(&r.rel_id).map(Arc::new);
                current[usize::from(r.footer)][kind_index(r.kind)] = blocks;
                current_rels[usize::from(r.footer)][kind_index(r.kind)] = Some(r.rel_id.clone());
            }
            stories.push(current.clone());
            rels.push(current_rels.clone());
        }
        let mut heights = Vec::new();
        for (i, st) in stories.iter().enumerate() {
            let width = sections.get(i).map_or(0.0, SectionProps::text_width);
            let mut h = [[0.0; 3]; 2];
            for (f, row) in st.iter().enumerate() {
                for (k, blocks) in row.iter().enumerate() {
                    if let Some(blocks) = blocks {
                        h[f][k] = height_of(&layout_blocks(ctx, blocks, width));
                    }
                }
            }
            heights.push(h);
        }
        Self {
            sections: sections.to_vec(),
            title_pg: doc_sections.iter().map(|s| s.title_pg).collect(),
            even_odd: doc.settings().even_and_odd_headers,
            stories,
            rels,
            heights,
        }
    }

    fn kind(&self, section: usize, page: usize, first: bool) -> usize {
        if first && self.title_pg.get(section).copied().unwrap_or(false) {
            1
        } else if self.even_odd && page.is_multiple_of(2) {
            2
        } else {
            0
        }
    }

    /// The body area of page `page` (1-based), the `first`-in-section page or not.
    pub(crate) fn frame(&self, section: usize, page: usize, first: bool) -> Frame {
        let sp = self.sections.get(section).copied().unwrap_or_default();
        let (mut top, mut bottom) = (sp.margin_top, sp.page_height - sp.margin_bottom);
        if let Some(h) = self.heights.get(section) {
            let k = self.kind(section, page, first);
            if self.stories[section][0][k].is_some() {
                top = top.max(sp.header_distance + h[0][k]);
            }
            if self.stories[section][1][k].is_some() {
                bottom = bottom.min(sp.page_height - sp.footer_distance - h[1][k]);
            }
        }
        Frame { top, bottom }
    }

    /// Fills the header and footer of every page; PAGE, NUMPAGES and SECTIONPAGES take the
    /// values of the finished layout.
    pub(crate) fn apply(&self, pages: &mut [Page], ctx: &Context<'_>) {
        let total = pages.len();
        let mut per_section: HashMap<usize, usize> = HashMap::new();
        for p in pages.iter() {
            *per_section.entry(p.section_index).or_default() += 1;
        }
        for page in pages.iter_mut() {
            let sec = page.section_index;
            let Some(stories) = self.stories.get(sec) else {
                continue;
            };
            let sp = page.section;
            let k = self.kind(sec, page.number, page.first_in_section);
            let env = FieldEnv {
                page: Some(page.number),
                num_pages: total,
                section_pages: per_section.get(&sec).copied().unwrap_or(1),
                section: sec,
            };
            let ctx = Context {
                fields: env,
                ..*ctx
            };
            let origin = sp.margin_left + sp.gutter;
            let width = sp.text_width();
            page.header.clear();
            page.footer.clear();
            page.header_images.clear();
            page.footer_images.clear();
            let source = |footer: usize| {
                self.rels
                    .get(sec)
                    .and_then(|r| r[footer][k].clone())
                    .map_or(ImageSource::Main, ImageSource::HeaderFooter)
            };
            if let Some(blocks) = &stories[0][k] {
                let paras = layout_blocks(&ctx, blocks, width);
                let story = [
                    StoryKind::PrimaryHeader,
                    StoryKind::FirstHeader,
                    StoryKind::EvenHeader,
                ][k];
                let placed = place(
                    &paras,
                    origin,
                    sp.header_distance,
                    story,
                    &source(0),
                    Some(&sp),
                );
                page.header = placed.lines;
                page.header_images = placed.images.into_iter().map(|i| i.1).collect();
            }
            if let Some(blocks) = &stories[1][k] {
                let paras = layout_blocks(&ctx, blocks, width);
                let story = [
                    StoryKind::PrimaryFooter,
                    StoryKind::FirstFooter,
                    StoryKind::EvenFooter,
                ][k];
                let h = height_of(&paras);
                let placed = place(
                    &paras,
                    origin,
                    sp.page_height - sp.footer_distance - h,
                    story,
                    &source(1),
                    Some(&sp),
                );
                page.footer = placed.lines;
                page.footer_images = placed.images.into_iter().map(|i| i.1).collect();
            }
        }
        for page in pages.iter_mut() {
            page.rules.clear();
            let mut rules = Vec::new();
            rules_of(&page.header, &mut rules);
            rules_of(&page.lines, &mut rules);
            rules_of(&page.notes, &mut rules);
            rules_of(&page.footer, &mut rules);
            page.rules = rules;
        }
    }
}

/// The footnotes of one section width, laid out.
#[derive(Debug, Default)]
struct SectionNotes {
    props: SectionProps,
    sep: Vec<ParaLayout>,
    sep_h: f64,
    paras: Vec<ParaLayout>,
    ranges: HashMap<i64, Range<usize>>,
    heights: HashMap<i64, f64>,
    /// Height of every line of a note, flattened over its paragraphs.
    line_heights: HashMap<i64, Vec<f64>>,
}

/// Footnote layouts for every section.
#[derive(Debug, Default)]
pub(crate) struct NoteStore {
    sections: Vec<SectionNotes>,
}

impl NoteStore {
    pub(crate) fn new(
        doc: &Document,
        sections: &[SectionProps],
        order: &NoteOrder,
        ctx: &Context<'_>,
    ) -> Self {
        let notes = doc.footnotes();
        if notes.is_empty() {
            return Self::default();
        }
        let ctx = Context {
            order: Some(order),
            ..*ctx
        };
        let sep_blocks: Vec<Block> = notes
            .iter()
            .find(|n| n.kind == NoteKind::Separator)
            .map(|n| n.blocks.clone())
            .unwrap_or_default();
        let mut out = Vec::new();
        for sp in sections {
            let width = sp.text_width();
            let sep = layout_blocks(&ctx, &sep_blocks, width);
            let sep_h = height_of(&sep);
            let labels: [Option<ListLabel>; 0] = [];
            let mut b = Builder::new(ctx, &labels, 0);
            let mut ranges = HashMap::new();
            for (i, note) in notes.iter().enumerate() {
                if note.kind != NoteKind::Normal {
                    continue;
                }
                let ordinal = order.foot.get(&note.id).copied().unwrap_or(i);
                b.ctx.own_mark = Some((false, ordinal));
                let start = b.arena.len();
                for block in &note.blocks {
                    if let Block::Paragraph(p) = block {
                        b.paragraph(p, width, None);
                    }
                }
                ranges.insert(note.id, start..b.arena.len());
            }
            let paras = b.arena;
            let heights = ranges
                .iter()
                .map(|(id, r)| (*id, height_of(&paras[r.clone()])))
                .collect();
            let line_heights = ranges
                .iter()
                .map(|(id, r)| {
                    let hs = paras[r.clone()]
                        .iter()
                        .flat_map(|p| p.lines.iter().map(|l| l.height))
                        .collect();
                    (*id, hs)
                })
                .collect();
            out.push(SectionNotes {
                props: *sp,
                sep,
                sep_h,
                paras,
                ranges,
                heights,
                line_heights,
            });
        }
        Self { sections: out }
    }

    /// Height of footnote `id` in a section of the store, 0 for an unknown note.
    pub(crate) fn height(&self, section: usize, id: i64) -> f64 {
        self.sections
            .get(section)
            .and_then(|s| s.heights.get(&id))
            .copied()
            .unwrap_or(0.0)
    }

    pub(crate) fn sep_height(&self, section: usize) -> f64 {
        self.sections.get(section).map_or(0.0, |s| s.sep_h)
    }

    /// Number of lines of footnote `id`.
    pub(crate) fn line_count(&self, section: usize, id: i64) -> usize {
        self.sections
            .get(section)
            .and_then(|s| s.line_heights.get(&id))
            .map_or(0, Vec::len)
    }

    /// Height of lines `from..to` of footnote `id`; the whole note's height (spacing between
    /// its paragraphs included) when the range is all of it.
    pub(crate) fn slot_height(&self, section: usize, slot: NoteSlot) -> f64 {
        let n = self.line_count(section, slot.id);
        if slot.from == 0 && slot.to >= n {
            return self.height(section, slot.id);
        }
        self.sections
            .get(section)
            .and_then(|s| s.line_heights.get(&slot.id))
            .map_or(0.0, |hs| hs[slot.from.min(n)..slot.to.min(n)].iter().sum())
    }

    /// The separator and note lines of a page whose body area ends at `bottom`.
    pub(crate) fn page_lines(
        &self,
        section: usize,
        slots: &[NoteSlot],
        origin_x: f64,
        bottom: f64,
    ) -> (Vec<Line>, Vec<ImagePlacement>) {
        let Some(s) = self.sections.get(section) else {
            return (Vec::new(), Vec::new());
        };
        if slots.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let total: f64 = s.sep_h
            + slots
                .iter()
                .map(|sl| self.slot_height(section, *sl))
                .sum::<f64>();
        let mut y = bottom - total;
        let src = ImageSource::Footnotes;
        let sep = place(
            &s.sep,
            origin_x,
            y,
            StoryKind::FootnoteSeparator,
            &src,
            Some(&s.props),
        );
        y += sep.height;
        let mut lines = sep.lines;
        let mut images: Vec<ImagePlacement> = sep.images.into_iter().map(|i| i.1).collect();
        for sl in slots {
            let Some(r) = s.ranges.get(&sl.id) else {
                continue;
            };
            let placed = place(
                &s.paras[r.clone()],
                origin_x,
                0.0,
                StoryKind::Footnotes,
                &src,
                Some(&s.props),
            );
            let (mut l, mut ims) = (placed.lines, placed.images);
            let n = self.line_count(section, sl.id);
            let shift;
            if sl.from > 0 || sl.to < n {
                shift = y - l.get(sl.from).map_or(0.0, |x| x.top);
                let (from, to) = (sl.from.min(n), sl.to.min(n));
                l = l.drain(from..to).collect();
                ims.retain(|(li, _)| (from..to).contains(li));
            } else {
                shift = y;
            }
            for line in &mut l {
                line.top += shift;
                line.baseline += shift;
            }
            images.extend(ims.into_iter().map(|(_, mut im)| {
                im.y += shift;
                im
            }));
            y += self.slot_height(section, *sl);
            lines.extend(l);
        }
        (lines, images)
    }
}

/// Ordinals of the footnote and endnote references in the body, in document order.
pub(crate) fn note_order(doc: &Document) -> NoteOrder {
    fn scan(content: &[Inline], order: &mut NoteOrder) {
        for inline in content {
            match inline {
                Inline::Run(r) => {
                    for c in &r.content {
                        if let RunContent::Raw(RawXml::Element(e)) = c
                            && let Some(id) =
                                e.head.attr_w("id").and_then(|v| v.trim().parse().ok())
                        {
                            if e.head.is_w("footnoteReference") {
                                let n = order.foot.len() + 1;
                                order.foot.entry(id).or_insert(n);
                            } else if e.head.is_w("endnoteReference") {
                                let n = order.end.len() + 1;
                                order.end.entry(id).or_insert(n);
                            }
                        }
                    }
                }
                Inline::Group(g) => scan(&g.content, order),
                Inline::Raw(_) => {}
            }
        }
    }
    let mut order = NoteOrder::default();
    let mut items = Vec::new();
    crate::collect_items(&doc.body().blocks, &mut items);
    for item in items {
        scan(&item.para.content, &mut order);
    }
    order
}
