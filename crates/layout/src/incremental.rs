//! Incremental layout: after an edit only the changed blocks are laid out again and pagination
//! restarts a few blocks before the first change and stops as soon as it reaches the state the
//! previous layout had at the same block. The result equals a full layout.
//!
//! * A *unit* (one body paragraph or one table, [`UnitLayout`]) is laid out independently of
//!   the page. A unit is reused when its block is equal to the cached one and it was laid out at
//!   the same width with the same list labels. Equal blocks are found by comparing the new body
//!   with the cached one from both ends, so inserting or deleting paragraphs keeps the rest.
//! * Pagination records a [`Checkpoint`] before every block. A run restarts from the checkpoint
//!   [`LOOK_BEHIND`] blocks before the first changed block (keep-with-next decisions look that
//!   far ahead), and ends when its checkpoint equals the earlier run's: the earlier pages after
//!   that point are then reused, with their paragraph and story numbers shifted.
//!
//! The cache assumes the styles, numbering and fonts do not change between calls (the document
//! model has no edit that changes them). Call [`IncrementalLayout::invalidate`] when the
//! document is replaced or the font library gets new faces.

use std::sync::Arc;

use open365_docx::{Block, Document, NoteKind};
use open365_text::FontLibrary;

use crate::build::{UnitLayout, build_unit};
use crate::flow::{Checkpoint, Env, FlowItem, Previous, Resume, paginate};
use crate::furniture::{Furniture, NoteStore, note_order};
use crate::numbering::{Labeler, ListLabel, NumberingState};
use crate::para::{Context, FieldEnv};
use crate::{
    FontUse, Layout, LayoutOptions, LineMetricModel, Page, SectionProps, StoryKind, collect_items,
};

/// How many blocks before the first changed one pagination starts again. Keep-with-next looks
/// at most nine blocks ahead (`can_start` stops at depth eight).
const LOOK_BEHIND: usize = 10;

/// The cheap-to-compare settings the cache depends on. The styles, numbering and fonts are not
/// compared: [`IncrementalLayout::invalidate`] must be called when they change.
#[derive(Debug, Clone, PartialEq)]
struct Fingerprint {
    model: LineMetricModel,
    font_count: usize,
    default_tab: u64,
}

#[derive(Debug)]
struct State {
    fingerprint: Fingerprint,
    sections: Vec<SectionProps>,
    /// The body's paragraph and table blocks as laid out.
    blocks: Vec<Block>,
    items: Vec<FlowItem>,
    /// One per item plus one for the end of the story.
    checkpoints: Vec<Checkpoint>,
}

/// A layout that is updated in place after edits. See the module documentation.
#[derive(Debug, Default)]
pub struct IncrementalLayout {
    layout: Layout,
    state: Option<State>,
    labels: Vec<Option<ListLabel>>,
    stats: UpdateStats,
}

/// What the last [`IncrementalLayout::update`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UpdateStats {
    /// Blocks laid out again.
    pub blocks_laid_out: usize,
    /// Blocks in the body.
    pub blocks: usize,
    /// Blocks placed on pages again.
    pub blocks_paginated: usize,
    /// Pages built again.
    pub pages_built: usize,
    /// Pagination stopped early because it re-converged with the previous layout.
    pub converged: bool,
    /// Everything was laid out from scratch.
    pub full: bool,
}

/// The list label of every paragraph in document order (tables included).
pub(crate) fn all_labels(doc: &Document) -> Vec<Option<ListLabel>> {
    let Some(labeler) = Labeler::new(doc) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    collect_items(&doc.body().blocks, &mut items);
    let mut state = NumberingState::default();
    items
        .iter()
        .map(|i| labeler.label(&mut state, i.para))
        .collect()
}

fn fingerprint(doc: &Document, fonts: &FontLibrary, options: LayoutOptions) -> Fingerprint {
    Fingerprint {
        model: options.metric_model,
        font_count: fonts.len(),
        default_tab: f64::from(doc.settings().default_tab_stop_twips).to_bits(),
    }
}

impl IncrementalLayout {
    pub fn new() -> Self {
        Self::default()
    }

    /// The layout of the last [`update`](Self::update).
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// The list label text of paragraph `index` (document order, tables included).
    pub fn list_label(&self, index: usize) -> Option<&str> {
        self.labels
            .get(index)
            .and_then(Option::as_ref)
            .map(|l| l.text.as_str())
    }

    /// Consumes the engine and returns the layout.
    pub fn into_layout(self) -> Layout {
        self.layout
    }

    pub fn stats(&self) -> UpdateStats {
        self.stats
    }

    /// Forgets the cache; the next update lays everything out. Call it when the document, its
    /// styles or numbering, or the font library changed.
    pub fn invalidate(&mut self) {
        self.state = None;
    }

    /// Brings the layout in line with `doc`, reusing what did not change.
    pub fn update(
        &mut self,
        doc: &Document,
        fonts: &FontLibrary,
        options: LayoutOptions,
    ) -> &Layout {
        let mut layout = Layout::default();
        let mut stats = UpdateStats::default();
        if fonts.is_empty() {
            layout.notes.push("font library is empty".to_owned());
            self.layout = layout;
            self.state = None;
            self.labels.clear();
            self.stats = stats;
            return &self.layout;
        }
        let order = note_order(doc);
        let ctx = Context {
            fonts,
            styles: doc.styles(),
            table: None,
            default_tab: f64::from(doc.settings().default_tab_stop_twips) / 20.0,
            model: options.metric_model,
            fields: FieldEnv::default(),
            order: Some(&order),
            own_mark: None,
        };
        let doc_sections = doc.sections();
        let sections: Vec<SectionProps> = doc_sections.iter().map(SectionProps::from).collect();
        let furniture = Furniture::new(doc, &doc_sections, &sections, &ctx);
        let note_store = NoteStore::new(doc, &sections, &order, &ctx);
        let env = Env {
            sections: &sections,
            furniture: &furniture,
            notes: &note_store,
        };
        for (i, s) in doc_sections.iter().enumerate() {
            if s.columns.count > 1 && !layout.notes.iter().any(|n| n.starts_with("columns")) {
                layout
                    .notes
                    .push(format!("columns are ignored (section {i})"));
            }
        }
        let labels = all_labels(doc);
        let mut blocks: Vec<&Block> = doc
            .body()
            .blocks
            .iter()
            .filter(|b| !matches!(b, Block::Raw(_)))
            .collect();
        let body_n = blocks.len();
        // Endnotes follow the body as further blocks of their own stories.
        let mut extra: Vec<(Block, StoryKind, Option<usize>)> = Vec::new();
        if !order.end.is_empty() {
            for note in doc.endnotes() {
                let (story, mark) = match note.kind {
                    NoteKind::Separator => (StoryKind::EndnoteSeparator, None),
                    NoteKind::Normal => match order.end.get(&note.id) {
                        Some(o) => (StoryKind::Endnotes, Some(*o)),
                        None => continue,
                    },
                    _ => continue,
                };
                for b in note.blocks {
                    if matches!(b, Block::Paragraph(_)) {
                        extra.push((b, story, mark));
                    }
                }
            }
        }
        blocks.extend(extra.iter().map(|e| &e.0));
        let fp = fingerprint(doc, fonts, options);
        let prev = self
            .state
            .take()
            .filter(|s| s.fingerprint == fp && s.sections == sections);
        stats.blocks = blocks.len();
        stats.full = prev.is_none();
        let mut prev = prev;
        let old_blocks: Vec<Block> = prev
            .as_mut()
            .map_or_else(Vec::new, |p| std::mem::take(&mut p.blocks));
        let (n, m) = (blocks.len(), old_blocks.len());

        // Equal blocks from both ends: the middle is what changed.
        let (mut pre, mut suf) = (0usize, 0usize);
        if prev.is_some() {
            while pre < n.min(m) && *blocks[pre] == old_blocks[pre] {
                pre += 1;
            }
            while suf < n.min(m) - pre && *blocks[n - 1 - suf] == old_blocks[m - 1 - suf] {
                suf += 1;
            }
        }
        let mut old_blocks: Vec<Option<Block>> = old_blocks.into_iter().map(Some).collect();

        let mut items: Vec<FlowItem> = Vec::with_capacity(n);
        let mut new_blocks: Vec<Block> = Vec::with_capacity(n);
        // Old index each item reuses, when its unit and section are unchanged.
        let mut same: Vec<Option<usize>> = Vec::with_capacity(n);
        let (mut section, mut para, mut story) = (0usize, 0usize, 0usize);
        let mut story_kind = StoryKind::Main;
        for (i, block) in blocks.iter().enumerate() {
            let (kind, mark) = if i < body_n {
                (StoryKind::Main, None)
            } else {
                (extra[i - body_n].1, extra[i - body_n].2)
            };
            if kind != story_kind {
                story_kind = kind;
                story = 0;
                section = sections.len().saturating_sub(1);
            }
            let sect = sections.get(section).copied().unwrap_or_default();
            let width = sect.text_width();
            let candidate = if i < pre {
                Some(i)
            } else if i >= n - suf {
                Some(m - (n - i))
            } else {
                None
            };
            let reuse = prev.as_ref().zip(candidate).and_then(|(p, o)| {
                let unit = &p.items[o].unit;
                (unit.width == width && unit.labels_match(labels.get(para..).unwrap_or(&[])))
                    .then_some(o)
            });
            let unit: Arc<UnitLayout> = match (&prev, reuse) {
                (Some(p), Some(o)) => Arc::clone(&p.items[o].unit),
                _ => {
                    stats.blocks_laid_out += 1;
                    let ctx = Context {
                        own_mark: mark.map(|o| (true, o)),
                        ..ctx
                    };
                    Arc::new(build_unit(block, ctx, &labels, para, width))
                }
            };
            same.push(reuse.filter(|o| {
                prev.as_ref()
                    .is_some_and(|p| p.items[*o].section == section)
            }));
            let count = unit.arena.len();
            let story_len = unit.story_len;
            items.push(FlowItem {
                unit,
                section,
                para_base: para,
                story_base: story,
                story: kind,
            });
            new_blocks.push(
                candidate
                    .and_then(|o| old_blocks.get_mut(o).and_then(Option::take))
                    .unwrap_or_else(|| (*block).clone()),
            );
            if i < body_n
                && matches!(block, Block::Paragraph(_))
                && doc_sections.get(section).and_then(|s| s.end_paragraph) == Some(para)
            {
                section += 1;
            }
            para += count;
            story += story_len;
        }
        drop(old_blocks);

        // The first item that differs, and where the unchanged tail starts.
        let first_dirty = same
            .iter()
            .enumerate()
            .position(|(i, s)| *s != Some(i))
            .unwrap_or(n);
        let mut tail = n;
        while tail > 0 && same[tail - 1].is_some_and(|o| o + n == tail - 1 + m) {
            tail -= 1;
        }

        let unchanged = prev
            .as_ref()
            .is_some_and(|_| first_dirty == n && n == m && tail == 0);
        if unchanged && let Some(p) = prev {
            // Nothing in the body changed; only the notes and fonts need refreshing.
            layout.pages = std::mem::take(&mut self.layout.pages);
            furniture.apply(&mut layout.pages, &ctx);
            layout.fonts = font_uses(&items, fonts);
            self.layout = layout;
            self.state = Some(State {
                blocks: new_blocks,
                items,
                ..p
            });
            self.labels = labels;
            self.stats = stats;
            return &self.layout;
        }

        let (pages, checkpoints) = match prev {
            None => {
                let out = paginate(&items, &env, None, None);
                stats.blocks_paginated = n;
                stats.pages_built = out.pages.len();
                (out.pages, out.checkpoints)
            }
            Some(p) => {
                let start = first_dirty.saturating_sub(LOOK_BEHIND);
                let ck = p.checkpoints[start];
                let mut old_pages = std::mem::take(&mut self.layout.pages);
                let mut rest = old_pages.split_off(ck.pages_done.min(old_pages.len()));
                let current = rest.first().map(|pg| Page {
                    lines: pg.lines[..ck.lines.min(pg.lines.len())].to_vec(),
                    cells: pg.cells[..ck.cells.min(pg.cells.len())].to_vec(),
                    note_slots: pg.note_slots[..ck.notes.min(pg.note_slots.len())].to_vec(),
                    notes: Vec::new(),
                    header: Vec::new(),
                    footer: Vec::new(),
                    ..pg.clone()
                });
                let Some(current) = current else {
                    // The cache and the layout disagree: start over.
                    let out = paginate(&items, &env, None, None);
                    stats.full = true;
                    stats.blocks_paginated = n;
                    stats.pages_built = out.pages.len();
                    layout.pages = out.pages;
                    furniture.apply(&mut layout.pages, &ctx);
                    layout.fonts = font_uses(&items, fonts);
                    self.layout = layout;
                    let checkpoints = out.checkpoints;
                    self.state = Some(State {
                        fingerprint: fp,
                        sections: sections.clone(),
                        blocks: new_blocks,
                        items,
                        checkpoints,
                    });
                    self.labels = labels;
                    self.stats = stats;
                    return &self.layout;
                };
                let resume = Resume {
                    item: start,
                    prior: old_pages,
                    page: current,
                    checkpoint: ck,
                    checkpoints: p.checkpoints[..start].to_vec(),
                };
                let previous = (tail < n).then_some(Previous {
                    items: &p.items,
                    checkpoints: &p.checkpoints,
                    first: tail,
                });
                let out = paginate(&items, &env, Some(resume), previous.as_ref());
                match (out.converged, out.open) {
                    (Some(c), Some(mut open)) => {
                        stats.converged = true;
                        stats.blocks_paginated = c - start;
                        let o = c + m - n;
                        let old_ck = p.checkpoints[o];
                        let first_page = ck.pages_done;
                        let at = old_ck.pages_done - first_page;
                        let mut tail_pages = rest.split_off((at + 1).min(rest.len()));
                        let mut old_page = rest.swap_remove(at.min(rest.len().saturating_sub(1)));
                        let d_para = items[c].para_base as isize - p.items[o].para_base as isize;
                        let d_story = items[c].story_base as isize - p.items[o].story_base as isize;
                        let d_page = out.pages.len() as isize - old_ck.pages_done as isize;
                        let mut moved_lines = old_page.lines.split_off(old_ck.lines);
                        let moved_cells = old_page.cells.split_off(old_ck.cells);
                        let moved_notes = old_page
                            .note_slots
                            .split_off(old_ck.notes.min(old_page.note_slots.len()));
                        shift_lines(&mut moved_lines, d_para, d_story);
                        open.lines.extend(moved_lines);
                        open.cells.extend(moved_cells);
                        open.note_slots.extend(moved_notes);
                        open.notes = note_store.page_lines(
                            open.section_index,
                            &open.note_slots,
                            open.section.margin_left + open.section.gutter,
                            open.body_bottom,
                        );
                        for pg in &mut tail_pages {
                            shift_lines(&mut pg.lines, d_para, d_story);
                            pg.number = pg.number.saturating_add_signed(d_page);
                        }
                        let out_pages_len = out.pages.len();
                        let mut pages = out.pages;
                        pages.push(open);
                        pages.extend(tail_pages);
                        stats.pages_built = out_pages_len + 1 - first_page;
                        let mut checkpoints = out.checkpoints;
                        checkpoints.extend(p.checkpoints[o..].iter().map(|c| Checkpoint {
                            pages_done: c.pages_done.saturating_add_signed(d_page),
                            ..*c
                        }));
                        (pages, checkpoints)
                    }
                    _ => {
                        stats.blocks_paginated = n - start;
                        stats.pages_built = out.pages.len().saturating_sub(ck.pages_done);
                        (out.pages, out.checkpoints)
                    }
                }
            }
        };

        layout.pages = pages;
        furniture.apply(&mut layout.pages, &ctx);
        layout.fonts = font_uses(&items, fonts);
        self.layout = layout;
        self.state = Some(State {
            fingerprint: fp,
            sections: sections.clone(),
            blocks: new_blocks,
            items,
            checkpoints,
        });
        self.labels = labels;
        self.stats = stats;
        &self.layout
    }
}

/// Moves the paragraph and story numbers of reused lines.
fn shift_lines(lines: &mut [crate::Line], d_para: isize, d_story: isize) {
    if d_para == 0 && d_story == 0 {
        return;
    }
    for l in lines {
        l.paragraph = l.paragraph.saturating_add_signed(d_para);
        l.start = l.start.saturating_add_signed(d_story);
        l.end = l.end.saturating_add_signed(d_story);
    }
}

/// Every family the units use, sorted by name, with how the library satisfies it.
fn font_uses(items: &[FlowItem], fonts: &FontLibrary) -> Vec<FontUse> {
    let mut requested: Vec<&str> = Vec::new();
    let mut last: Option<&UnitLayout> = None;
    for item in items {
        let unit = item.unit.as_ref();
        if last.is_some_and(|l| l.fonts == unit.fonts) {
            continue;
        }
        requested.extend(unit.fonts.iter().map(String::as_str));
        last = Some(unit);
    }
    requested.sort_unstable();
    requested.dedup();
    requested
        .into_iter()
        .map(|r| FontUse {
            requested: r.to_owned(),
            matched: fonts.match_kind(r),
        })
        .collect()
}
