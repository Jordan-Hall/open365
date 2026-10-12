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

use open365_docx::{
    Block, Document, HeaderFooterType, Inline, NoteKind, Paragraph, RawXml, RunContent, W_NS,
    parse_xml,
};

use crate::build::Builder;
use crate::float::{self, Frame as FloatFrame};
use crate::flow::{Base, build_line, line_images};
use crate::incremental::{REL_FOOTNOTES, related_links};
use crate::numbering::ListLabel;
use crate::para::{Context, FieldEnv, LineBox, NoteOrder, ParaLayout};
use crate::props::{SectionCols, SectionProps};
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
    /// External hyperlink targets of each header or footer part by its relationship id in the
    /// main part (the part's own relationship ids differ from the main part's).
    links: HashMap<String, HashMap<String, String>>,
    heights: Vec<[[f64; 3]; 2]>,
    /// Sections whose missing header and footer stories Word keeps as empty paragraphs.
    implied: Vec<bool>,
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
            let frame = FloatFrame::new(*sp);
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

/// An empty paragraph of the built-in header or footer style: what Word keeps in a header or
/// footer story the file does not define.
fn empty_story_paragraph(style: &str) -> Option<Paragraph> {
    let xml = format!("<w:p xmlns:w=\"{W_NS}\"><w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr></w:p>");
    let root = parse_xml(xml.as_bytes()).ok()?.root;
    let props = root.children.iter().find_map(|c| c.as_element().cloned());
    Some(Paragraph {
        head: root.head,
        props,
        content: Vec::new(),
    })
}

/// Word keeps an empty header and footer story for a section with a different first page even when
/// the file defines none (`titlePg` without references), and reserves their height.
fn stories_implied(doc_sections: &[open365_docx::Section], section: usize) -> bool {
    let Some(s) = doc_sections.get(section) else {
        return false;
    };
    let footers = s.header_footer_refs.iter().filter(|r| r.footer).count();
    let headers = s.header_footer_refs.len() - footers;
    (headers == 0 && footers >= 2) || (footers == 0 && headers >= 2)
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
        let mut links: HashMap<String, HashMap<String, String>> = HashMap::new();
        for s in doc_sections {
            for r in &s.header_footer_refs {
                links
                    .entry(r.rel_id.clone())
                    .or_insert_with(|| related_links(doc, |id, _| id == r.rel_id));
                let blocks = doc.header_footer_blocks(&r.rel_id).map(Arc::new);
                current[usize::from(r.footer)][kind_index(r.kind)] = blocks;
                current_rels[usize::from(r.footer)][kind_index(r.kind)] = Some(r.rel_id.clone());
            }
            stories.push(current.clone());
            rels.push(current_rels.clone());
        }
        let mut heights = Vec::new();
        let mut implied = Vec::new();
        for (i, st) in stories.iter().enumerate() {
            let width = sections.get(i).map_or(0.0, SectionProps::text_width);
            let mut h = [[0.0; 3]; 2];
            let implies = stories_implied(doc_sections, i);
            implied.push(implies);
            for (f, row) in st.iter().enumerate() {
                for (k, blocks) in row.iter().enumerate() {
                    if blocks.is_none() && implies {
                        let style = if f == 0 { "Header" } else { "Footer" };
                        if let Some(p) = empty_story_paragraph(style) {
                            h[f][k] = height_of(&layout_blocks(ctx, &[Block::Paragraph(p)], width));
                        }
                    }
                    if let Some(blocks) = blocks {
                        let ctx = Context {
                            links: rels[i][f][k].as_ref().and_then(|id| links.get(id)),
                            ..*ctx
                        };
                        h[f][k] = height_of(&layout_blocks(&ctx, blocks, width));
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
            links,
            heights,
            implied,
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
            if self.stories[section][0][k].is_some() || self.implied[section] {
                top = top.max(sp.header_distance + h[0][k]);
            }
            if self.stories[section][1][k].is_some() || self.implied[section] {
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
            let part_links = |footer: usize| {
                self.rels
                    .get(sec)
                    .and_then(|r| r[footer][k].as_ref())
                    .and_then(|id| self.links.get(id))
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
                let ctx = Context {
                    links: part_links(0),
                    ..ctx
                };
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
                let ctx = Context {
                    links: part_links(1),
                    ..ctx
                };
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
    /// Left edge of each note column relative to the text area (one entry for a one-column
    /// section).
    col_x: Vec<f64>,
    /// Width of each note column when the columns differ (empty otherwise): notes are set at
    /// each column's own width and flow down the first column, then the next (see
    /// [`NoteStore::flow`]).
    col_w: Vec<f64>,
}

/// A run of lines of one note paragraph in one note column.
struct Frag {
    /// Index into [`SectionNotes::paras`].
    para: usize,
    lines: Vec<LineBox>,
    col: usize,
    /// Distance from the top of the note area.
    y: f64,
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
        cols: &[SectionCols],
        order: &NoteOrder,
        ctx: &Context<'_>,
    ) -> Self {
        let notes = doc.footnotes();
        if notes.is_empty() {
            return Self::default();
        }
        let links = related_links(doc, |_, t| t == REL_FOOTNOTES);
        let ctx = Context {
            order: Some(order),
            links: Some(&links),
            ..*ctx
        };
        let sep_blocks: Vec<Block> = notes
            .iter()
            .find(|n| n.kind == NoteKind::Separator)
            .map(|n| n.blocks.clone())
            .unwrap_or_default();
        let mut out = Vec::new();
        for (si, sp) in sections.iter().enumerate() {
            // Measured (layout-cols-footnote): the footnotes of a section with equal columns are
            // set in columns of the section's width and balanced over them.
            // Unequal columns (layout-cols-fn-unequal): each column keeps its own width.
            let (col_x, col_w): (Vec<f64>, Vec<f64>) = match cols.get(si) {
                Some(c) if c.is_multi() => {
                    let equal = c
                        .columns
                        .iter()
                        .all(|g| (g.width - c.first_width()).abs() < 0.01);
                    (
                        c.columns.iter().map(|g| g.x).collect(),
                        if equal {
                            Vec::new()
                        } else {
                            c.columns.iter().map(|g| g.width).collect()
                        },
                    )
                }
                _ => (vec![0.0], Vec::new()),
            };
            let width = if col_x.len() > 1 {
                cols[si].first_width()
            } else {
                sp.text_width()
            };
            let sep = layout_blocks(&ctx, &sep_blocks, sp.text_width());
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
                col_x,
                col_w,
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

    /// The column (0-based) of each slot and the height of the note area: whole notes in order,
    /// split over the section's note columns so that the tallest column is as short as it can be.
    fn columns_of(&self, section: usize, slots: &[NoteSlot]) -> (Vec<usize>, f64) {
        let n = self
            .sections
            .get(section)
            .map_or(1, |s| s.col_x.len().max(1));
        let hs: Vec<f64> = slots
            .iter()
            .map(|sl| self.slot_height(section, *sl))
            .collect();
        let total: f64 = hs.iter().sum();
        if n == 1 {
            return (vec![0; hs.len()], total);
        }
        let pack = |limit: f64| {
            let (mut col, mut used) = (0usize, 0.0f64);
            let mut out = Vec::with_capacity(hs.len());
            for h in &hs {
                if used > 0.0 && used + h > limit + 1e-6 && col + 1 < n {
                    col += 1;
                    used = 0.0;
                }
                used += h;
                out.push((col, used));
            }
            out
        };
        let (mut lo, mut hi) = (hs.iter().copied().fold(0.0, f64::max), total);
        for _ in 0..40 {
            let mid = (lo + hi) / 2.0;
            let tallest = pack(mid).iter().map(|p| p.1).fold(0.0, f64::max);
            if tallest <= mid + 1e-6 {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        let packed = pack(hi);
        let height = packed.iter().map(|p| p.1).fold(0.0, f64::max);
        (packed.iter().map(|p| p.0).collect(), height)
    }

    /// Whether the section has footnotes at all.
    pub(crate) fn has_notes(&self, section: usize) -> bool {
        self.sections
            .get(section)
            .is_some_and(|s| !s.ranges.is_empty())
    }

    /// Whether the section's note columns differ in width.
    fn unequal(&self, section: usize) -> bool {
        self.sections
            .get(section)
            .is_some_and(|s| !s.col_w.is_empty())
    }

    /// Lines of `pl` from atom `at` to atom `end`, broken for a column `delta` narrower than
    /// the width the paragraph was laid out at.
    fn refill(pl: &ParaLayout, at: usize, end: usize, delta: f64) -> Vec<LineBox> {
        let b = pl.builder();
        let mut out = Vec::new();
        let mut a = at;
        while a < end {
            let l = b.fill_in(a, a == 0, (0.0, delta));
            if l.atoms.end <= a {
                break;
            }
            a = l.atoms.end;
            out.push(l);
        }
        out
    }

    /// Sets `slots` in a section whose note columns differ in width (measured on
    /// `layout-cols-fn-unequal`): the notes fill the first column down to `limit`, a note
    /// splits by line and its tail is broken again at the next column's width. Widow and orphan
    /// control keep two lines on each side of a split. `None` when the notes do not fit.
    fn flow(&self, section: usize, slots: &[NoteSlot], limit: f64) -> Option<(Vec<Frag>, f64)> {
        const EPS: f64 = 1e-6;
        let s = self.sections.get(section)?;
        let n = s.col_w.len();
        let (mut col, mut y, mut tallest) = (0usize, 0.0f64, 0.0f64);
        let mut frags = Vec::new();
        for slot in slots {
            let Some(r) = s.ranges.get(&slot.id) else {
                continue;
            };
            let (mut lo, mut prev_after) = (0usize, 0.0f64);
            for (pj, pl) in s.paras[r.clone()].iter().enumerate() {
                let first_flat = lo;
                lo += pl.lines.len();
                let before = f64::from(pl.props.space_before_pt);
                let after = f64::from(pl.props.space_after_pt);
                let (a, b) = (slot.from.max(first_flat), slot.to.min(lo));
                let mut sb = match (a == first_flat, pj) {
                    (false, _) => 0.0,
                    (true, 0) => before,
                    (true, _) => prev_after.max(before),
                };
                prev_after = after;
                if a >= b {
                    continue;
                }
                let post = if b == lo && pj + 1 == r.len() {
                    after
                } else {
                    0.0
                };
                let mut at = pl.lines[a - first_flat].atoms.start;
                let end = pl.lines[b - 1 - first_flat].atoms.end;
                loop {
                    let lines = Self::refill(pl, at, end, s.col_w[0] - s.col_w[col]);
                    if lines.is_empty() {
                        break;
                    }
                    let total = lines.len();
                    let mut k = 0;
                    let mut used = y + sb;
                    for (i, l) in lines.iter().enumerate() {
                        let extra = if i + 1 == total { post } else { 0.0 };
                        if used + l.height + extra > limit + EPS {
                            break;
                        }
                        used += l.height;
                        k += 1;
                    }
                    if k == total {
                        frags.push(Frag {
                            para: r.start + pj,
                            lines,
                            col,
                            y: y + sb,
                        });
                        y = used + post;
                        tallest = tallest.max(y);
                        break;
                    }
                    if col + 1 >= n {
                        return None;
                    }
                    let at_top = y == 0.0;
                    let head = at == 0;
                    if pl.flow.widow_control {
                        let rest = |k: usize| {
                            Self::refill(
                                pl,
                                lines[k].atoms.start,
                                end,
                                s.col_w[0] - s.col_w[col + 1],
                            )
                            .len()
                        };
                        if head && k == 1 {
                            k = 0;
                        } else if k >= 1 && rest(k) == 1 {
                            k -= 1;
                            if head && k < 2 {
                                k = 0;
                            }
                        }
                    }
                    if k == 0 && at_top {
                        k = 1;
                    }
                    if k > 0 {
                        tallest =
                            tallest.max(y + sb + lines[..k].iter().map(|l| l.height).sum::<f64>());
                        frags.push(Frag {
                            para: r.start + pj,
                            lines: lines[..k].to_vec(),
                            col,
                            y: y + sb,
                        });
                        at = lines[k].atoms.start;
                    }
                    col += 1;
                    y = 0.0;
                    sb = 0.0;
                }
            }
        }
        Some((frags, tallest))
    }

    /// The smallest note area height for which [`Self::flow`] fits `slots`, and the notes as
    /// they then lie.
    fn flow_min(&self, section: usize, slots: &[NoteSlot]) -> (Vec<Frag>, f64) {
        let total: f64 = slots.iter().map(|sl| self.slot_height(section, *sl)).sum();
        let (mut lo, mut hi) = (0.0f64, total + 1.0);
        let mut best = self.flow(section, slots, hi);
        for _ in 0..30 {
            let mid = (lo + hi) / 2.0;
            if let Some(f) = self.flow(section, slots, mid) {
                hi = mid;
                best = Some(f);
            } else {
                lo = mid;
            }
        }
        best.unwrap_or_default()
    }

    /// Height of the note area (separator excluded) for `slots`.
    pub(crate) fn area(&self, section: usize, slots: &[NoteSlot]) -> f64 {
        if self.unequal(section) {
            return self.flow_min(section, slots).1;
        }
        self.columns_of(section, slots).1
    }

    /// [`Self::page_lines`] for note columns of different widths.
    fn flow_lines(
        &self,
        s: &SectionNotes,
        section: usize,
        slots: &[NoteSlot],
        origin_x: f64,
        bottom: f64,
    ) -> (Vec<Line>, Vec<ImagePlacement>) {
        let (frags, area) = self.flow_min(section, slots);
        let mut y = bottom - (s.sep_h + area);
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
        let base = Base {
            para: 0,
            story: 0,
            table: 0,
        };
        for f in &frags {
            let pl = &s.paras[f.para];
            let x = origin_x + s.col_x.get(f.col).copied().unwrap_or(0.0);
            let mut top = y + f.y;
            for lb in &f.lines {
                let mut line = build_line(pl, lb, x, top, base);
                line.story = StoryKind::Footnotes;
                line.border = pl.line_border(lb, x, false, false);
                for mut im in line_images(pl, lb, x, top, base) {
                    im.source = src.clone();
                    images.push(im);
                }
                lines.push(line);
                top += lb.height;
            }
        }
        (lines, images)
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
        if self.unequal(section) {
            return self.flow_lines(s, section, slots, origin_x, bottom);
        }
        let (col_of, area) = self.columns_of(section, slots);
        let total: f64 = s.sep_h + area;
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
        let mut col_y = vec![y; s.col_x.len().max(1)];
        for (sl, col) in slots.iter().zip(col_of) {
            y = col_y[col];
            let Some(r) = s.ranges.get(&sl.id) else {
                continue;
            };
            let placed = place(
                &s.paras[r.clone()],
                origin_x + s.col_x.get(col).copied().unwrap_or(0.0),
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
            col_y[col] = y + self.slot_height(section, *sl);
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
