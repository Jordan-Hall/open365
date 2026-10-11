//! Page flow: places laid-out paragraphs and tables onto pages with Word's rules for spacing,
//! widow and orphan control, keep-with-next, keep-lines, page breaks, sections, table row
//! splitting and repeated header rows.

use std::sync::Arc;

use crate::build::UnitLayout;
use crate::float::{Float, Frame, dynamic_lines};
use crate::furniture::{Furniture, NoteStore};
use crate::para::{AtomClass, LineBox, ParaLayout};
use crate::props::SectionProps;
use crate::table::{CellLayout, CellMeta, FlowLine, RowLayout, TableLayout};
use crate::{
    CellBox, Cluster, ClusterKind, GlyphRun, ImagePlacement, ImageSource, Line, NoteSlot, Page,
    PlacedGlyph, StoryKind,
};

fn cell_box(x: f64, y: f64, width: f64, height: f64, meta: CellMeta, table_base: usize) -> CellBox {
    CellBox {
        x,
        y,
        width,
        height,
        table: table_base + meta.table,
        row: meta.row,
        col: meta.col,
        col_span: meta.col_span,
        row_span: meta.row_span,
        header_row: meta.header_row,
        fill: meta.fill,
        borders: meta.borders,
        pad_top: meta.pad_top,
        pad_bottom: meta.pad_bottom,
    }
}

/// What pagination needs besides the items: section geometry, header and footer sizes, notes.
pub(crate) struct Env<'a> {
    pub sections: &'a [SectionProps],
    pub furniture: &'a Furniture,
    pub notes: &'a NoteStore,
}

const EPS: f64 = 1e-6;

/// A block of the main story in layout order: a laid-out unit and where it sits in the story.
#[derive(Debug, Clone)]
pub(crate) struct FlowItem {
    pub unit: Arc<UnitLayout>,
    pub section: usize,
    /// Document-order index of the unit's first paragraph.
    pub para_base: usize,
    /// Story offset of the unit's first character.
    pub story_base: usize,
    /// Document-order id of the unit's first table.
    pub table_base: usize,
    /// The story the unit belongs to (the main story, or the endnotes after it).
    pub story: StoryKind,
}

impl FlowItem {
    fn section(&self) -> usize {
        self.section
    }

    fn para(&self) -> Option<&ParaLayout> {
        self.unit.para()
    }

    fn table(&self) -> Option<&TableLayout> {
        self.unit.table.as_ref()
    }

    fn base(&self) -> Base {
        Base {
            para: self.para_base,
            story: self.story_base,
            table: self.table_base,
        }
    }

    /// What the next item sees of this one: space after, style id and contextual spacing.
    fn carry(&self) -> Carry {
        match self.para() {
            Some(p) => Carry {
                after: f64::from(p.props.space_after_pt),
                style: Some(p.style_id.clone()),
                contextual: p.flow.contextual_spacing,
            },
            None => Carry::default(),
        }
    }
}

/// Offsets that turn a unit's relative paragraph and story numbers into document ones.
#[derive(Clone, Copy)]
pub(crate) struct Base {
    pub para: usize,
    pub story: usize,
    pub table: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct Carry {
    after: f64,
    style: Option<Option<String>>,
    contextual: bool,
}

/// The paginator's state at the start of a flow item, enough to resume from there given the
/// pages before it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Checkpoint {
    /// Pages finished before the current one.
    pub pages_done: usize,
    /// Lines and cell boxes already on the current page.
    pub lines: usize,
    pub cells: usize,
    pub images: usize,
    pub floats: usize,
    /// Fingerprint of the floats' rectangles, for convergence.
    pub floats_sig: u64,
    pub y: f64,
    pub hard_start: bool,
    pub force_break: bool,
    pub section_index: usize,
    /// Footnotes on the current page.
    pub notes: usize,
}

struct PageBuild {
    number: usize,
    section: SectionProps,
    section_index: usize,
    lines: Vec<Line>,
    cells: Vec<CellBox>,
    images: Vec<ImagePlacement>,
    floats: Vec<Float>,
    first_in_section: bool,
    /// Top and bottom of the body area before footnotes take room.
    top: f64,
    frame_bottom: f64,
    slots: Vec<NoteSlot>,
    notes_h: f64,
    sep_h: f64,
    /// Cursor: top of the next line.
    y: f64,
    /// Space after the previous paragraph on this page, not yet applied.
    pending_after: f64,
    /// Style id of the previous paragraph on this page.
    prev_style: Option<Option<String>>,
    prev_contextual: bool,
    /// How this page began, which decides whether space before survives at its top.
    hard_start: bool,
}

impl PageBuild {
    fn new(
        number: usize,
        section_index: usize,
        section: SectionProps,
        hard_start: bool,
        first_in_section: bool,
        env: &Env<'_>,
    ) -> Self {
        let frame = env.furniture.frame(section_index, number, first_in_section);
        Self {
            number,
            section,
            section_index,
            lines: Vec::new(),
            cells: Vec::new(),
            images: Vec::new(),
            floats: Vec::new(),
            first_in_section,
            top: frame.top,
            frame_bottom: frame.bottom,
            slots: Vec::new(),
            notes_h: 0.0,
            sep_h: env.notes.sep_height(section_index),
            y: frame.top,
            pending_after: 0.0,
            prev_style: None,
            prev_contextual: false,
            hard_start,
        }
    }

    /// Where the body ends on this page: the frame's bottom less the footnote area.
    fn bottom(&self) -> f64 {
        if self.slots.is_empty() {
            self.frame_bottom
        } else {
            self.frame_bottom - self.notes_h - self.sep_h
        }
    }

    /// Puts lines `from..to` of footnote `id` on the page.
    fn add_slot(&mut self, slot: NoteSlot, env: &Env<'_>) {
        let h = env.notes.slot_height(self.section_index, slot);
        if h > 0.0
            && !self
                .slots
                .iter()
                .any(|s| s.id == slot.id && s.from == slot.from)
        {
            self.slots.push(slot);
            self.notes_h += h;
        }
    }

    fn add_note(&mut self, id: i64, env: &Env<'_>) {
        let to = env.notes.line_count(self.section_index, id);
        self.add_slot(NoteSlot { id, from: 0, to }, env);
    }

    fn has_content(&self) -> bool {
        !self.lines.is_empty() || !self.cells.is_empty()
    }

    /// Left edge of the text area.
    fn origin_x(&self) -> f64 {
        self.section.margin_left + self.section.gutter
    }

    fn finish(self, env: &Env<'_>) -> Page {
        let (notes, note_images) = env.notes.page_lines(
            self.section_index,
            &self.slots,
            self.section.margin_left + self.section.gutter,
            self.frame_bottom,
        );
        Page {
            number: self.number,
            section: self.section,
            lines: self.lines,
            cells: self.cells.into_iter().map(snap_cell).collect(),
            images: self.images,
            frames: Vec::new(),
            floats: self.floats,
            section_index: self.section_index,
            first_in_section: self.first_in_section,
            body_top: self.top,
            body_bottom: self.frame_bottom,
            note_slots: self.slots,
            notes,
            header: Vec::new(),
            footer: Vec::new(),
            header_images: Vec::new(),
            footer_images: Vec::new(),
            note_images,
            rules: Vec::new(),
        }
    }
}

/// How many of `heights` fit starting at `y0` above `bottom`.
/// Each item is a line's height and the part of it (spacing below the text) that need not fit.
fn lines_fitting(heights: impl Iterator<Item = (f64, f64)>, y0: f64, bottom: f64) -> usize {
    let mut y = y0;
    let mut k = 0;
    for (h, slack) in heights {
        if y + h - slack > bottom + EPS {
            break;
        }
        y += h;
        k += 1;
    }
    k
}

struct Placement {
    /// Lines of the paragraph's remaining range that go on this page.
    take: usize,
    /// The footnote of the last taken line that continues on the next page: its id and the
    /// number of its lines that stay on this page.
    split: Option<(i64, usize)>,
}

/// The footnote state of a page while a paragraph's lines are tried on it.
struct NotesAt<'a> {
    env: &'a Env<'a>,
    section: usize,
    /// The page's body bottom before footnotes, the footnotes already on it and their height.
    frame_bottom: f64,
    count: usize,
    height: f64,
    sep: f64,
}

/// How many of `para.lines[from..]` fit above the frame bottom once the footnotes their
/// references bring (and the separator, for the first) are placed.
fn lines_fitting_notes(
    para: &ParaLayout,
    lines: &[LineBox],
    y0: f64,
    n: &NotesAt<'_>,
) -> (usize, Option<(i64, usize)>) {
    let (mut y, mut nh, mut has, mut k) = (y0, n.height, n.count > 0, 0);
    for l in lines {
        y += l.lead;
        let mut add = 0.0;
        let mut any = false;
        let mut ids = Vec::new();
        for (a, id) in &para.foot_refs {
            if l.atoms.contains(a) {
                let h = n.env.notes.height(n.section, *id);
                any |= h > 0.0;
                add += h;
                ids.push(*id);
            }
        }
        let has2 = has || any;
        let total = nh + add + if has2 { n.sep } else { 0.0 };
        if y + l.height - l.slack + total > n.frame_bottom + EPS {
            // A single long footnote may start here and go on in the next page's notes.
            if let [id] = ids[..] {
                let lines = n.env.notes.line_count(n.section, id);
                let avail = n.frame_bottom - (y + l.height) - nh - if has { 0.0 } else { n.sep };
                let mut m = 0;
                let mut used = 0.0;
                while m < lines {
                    let h = n.env.notes.slot_height(
                        n.section,
                        NoteSlot {
                            id,
                            from: m,
                            to: m + 1,
                        },
                    );
                    if used + h > avail + EPS {
                        break;
                    }
                    used += h;
                    m += 1;
                }
                if lines - m == 1 && m > 2 {
                    m -= 1;
                }
                if m >= 2.min(lines) && m < lines {
                    return (k + 1, Some((id, m)));
                }
            }
            break;
        }
        y += l.height;
        nh += add;
        has = has2;
        k += 1;
    }
    (k, None)
}

/// Decides how many of `lines[from..]` to put on a page where `y0` is where the first would
/// go. `at_top` means nothing is on the page yet, so no rule may push everything away.
fn decide(
    para: &ParaLayout,
    lines: &[LineBox],
    first_segment: bool,
    y0: f64,
    bottom: f64,
    at_top: bool,
    notes: Option<&NotesAt<'_>>,
) -> Placement {
    let n = lines.len();
    let (raw, split) = match notes {
        Some(nt) => lines_fitting_notes(para, lines, y0, nt),
        None => (
            lines_fitting(
                lines.iter().map(|l| (l.lead + l.height, l.slack)),
                y0,
                bottom,
            ),
            None,
        ),
    };
    let mut k = raw;
    if k < n {
        if para.flow.keep_lines && first_segment && k > 0 {
            k = 0;
        } else if para.flow.widow_control && n >= 2 {
            if first_segment && k == 1 {
                k = 0;
            } else if n - k == 1 && k >= 1 {
                // One line would be left alone on the next page: pull one more over.
                k -= 1;
                if first_segment && k < 2 {
                    k = 0;
                }
            }
        }
        if k == 0 && at_top {
            k = raw.max(1);
        }
    } else if n > 0 && raw == 0 && at_top {
        k = 1;
    }
    // A pulled-back line count (widow control, keep rules) drops the split with its line.
    Placement {
        take: k,
        split: split.filter(|_| k == raw),
    }
}

/// Whether item `p` (and, when it is kept with the next, its followers) can start on a page
/// whose cursor is at `y`.
fn can_start(
    items: &[FlowItem],
    p: usize,
    y: f64,
    prev_after: f64,
    bottom: f64,
    depth: usize,
) -> bool {
    let section = items[p].section;
    let Some(para) = items[p].para() else {
        // A table starts with its first row's first line.
        let first = items[p]
            .table()
            .and_then(|t| t.rows.first())
            .map_or(0.0, |r| {
                r.cells
                    .iter()
                    .filter_map(|c| c.lines.first())
                    .map(|l| l.dy + l.height)
                    .fold(0.0, f64::max)
            });
        return y + prev_after + first <= bottom + EPS;
    };
    // A follower that starts its own page (page-break-before, a new section) is not kept with.
    if para.flow.page_break_before
        || p.checked_sub(1)
            .and_then(|q| items.get(q))
            .is_some_and(|i| i.section() != section)
    {
        return true;
    }
    // Space between paragraphs is the larger of space after and space before.
    let y0 = y + prev_after.max(f64::from(para.props.space_before_pt));
    let place = decide(para, &para.lines, true, y0, bottom, false, None);
    if place.take == 0 {
        return false;
    }
    if para.flow.keep_next && place.take == para.lines.len() && p + 1 < items.len() && depth < 8 {
        let used: f64 = para.lines.iter().map(|l| l.height).sum();
        return can_start(
            items,
            p + 1,
            y0 + used,
            f64::from(para.props.space_after_pt),
            bottom,
            depth + 1,
        );
    }
    true
}

/// Word positions baselines on a 1/300 inch grid (0.24 pt), rounding up: measured from the
/// 600 dpi PDF export (`tools/office/word-layout-oracle.ps1`) over every font size from 8 to
/// 72 pt, the line pitch averages the unrounded font height but each baseline lies on this grid.
const BASELINE_GRID: f64 = 0.24;

fn snap_baseline(y: f64) -> f64 {
    ((y - 1e-6) / BASELINE_GRID).ceil() * BASELINE_GRID
}

/// A cell rectangle as Word draws it: every edge lies on the same 0.24 pt grid as baselines, each
/// rounded to the nearest line, halves up (measured over 120-row probe tables and 500 cell
/// edges of the `layout-table-*` fixtures: the row boundaries are `round(exact_y / 0.24) * 0.24`,
/// so rows alternate 13.44 and 13.20 pt for a 13.43 pt line; the column edges `round(exact_x /
/// 0.24) * 0.24` in page coordinates, so equal twip widths come out 199.92 / 200.4 pt).
fn snap_cell(c: CellBox) -> CellBox {
    let snap = |v: f64| ((v / BASELINE_GRID) + 1e-9).round() * BASELINE_GRID;
    let (left, right) = (snap(c.x), snap(c.x + c.width));
    let (top, bottom) = (snap(c.y), snap(c.y + c.height));
    CellBox {
        x: left,
        width: right - left,
        y: top,
        height: bottom - top,
        ..c
    }
}

/// Builds the placed line `lb` of `para`, its text area starting at `origin_x` on the page.
pub(crate) fn build_line(
    para: &ParaLayout,
    lb: &LineBox,
    origin_x: f64,
    top: f64,
    base: Base,
) -> Line {
    let left_origin = origin_x;
    let mut runs: Vec<GlyphRun> = Vec::new();
    let mut cur_style: Option<usize> = None;
    let mut label_text: Option<std::sync::Arc<str>> = None;
    if lb.atoms.start == 0
        && let Some(label) = &para.label
    {
        let st = &para.styles[label.style];
        let mut x = left_origin + label.x;
        let glyphs = label
            .glyphs
            .iter()
            .enumerate()
            .map(|(k, g)| {
                // The text runs from this glyph's cluster to the next cluster; later glyphs of
                // one cluster carry no text.
                let first = k == 0 || label.glyphs[k - 1].cluster != g.cluster;
                let end = label.glyphs[k + 1..]
                    .iter()
                    .map(|n| n.cluster)
                    .find(|c| *c != g.cluster)
                    .unwrap_or(label.text.len() as u32);
                let placed = PlacedGlyph {
                    face: g.face,
                    glyph_id: g.glyph_id,
                    x: x + g.x_offset,
                    y_offset: g.y_offset,
                    advance: g.adv,
                    cluster: Cluster {
                        kind: ClusterKind::Label,
                        start: 0,
                        end: 0,
                        text_start: g.cluster,
                        text_end: if first { end } else { g.cluster },
                    },
                };
                x += g.adv;
                placed
            })
            .collect();
        label_text = Some(label.text.clone());
        runs.push(GlyphRun {
            face: st.face,
            size: st.size,
            bold: st.bold,
            italic: st.italic,
            underline: st.underline,
            strike: st.strike,
            color: st.color,
            superscript: false,
            glyphs,
        });
    }
    for (k, ai) in lb.atoms.clone().enumerate() {
        let atom = &para.atoms[ai];
        if !matches!(atom.class, AtomClass::Text | AtomClass::Space) {
            continue;
        }
        if cur_style != Some(atom.style) {
            let st = &para.styles[atom.style];
            runs.push(GlyphRun {
                face: st.face,
                size: st.size,
                bold: st.bold,
                italic: st.italic,
                underline: st.underline,
                strike: st.strike,
                color: st.color,
                superscript: st.superscript,
                glyphs: Vec::new(),
            });
            cur_style = Some(atom.style);
        }
        // The cluster: the atom's characters, and the story range up to the next atom.
        let text_end = para
            .atoms
            .get(ai + 1)
            .map_or(para.text.len(), |n| n.byte_start);
        let story = &para.fill.story_of;
        let rel = |u: usize| u.saturating_sub(lb.start) as u32;
        // Story offsets of atoms are not monotonic around a note's own mark.
        let (c_start, c_end) = (rel(story[ai]), rel(story[ai + 1]).max(rel(story[ai])));
        let mut x = left_origin + lb.xs[k];
        if let Some(run) = runs.last_mut() {
            for (gi, g) in para.glyphs[atom.glyphs.clone()].iter().enumerate() {
                run.glyphs.push(PlacedGlyph {
                    face: g.face,
                    glyph_id: g.glyph_id,
                    x: x + g.x_offset,
                    y_offset: g.y_offset,
                    advance: g.adv,
                    cluster: Cluster {
                        kind: ClusterKind::Text,
                        start: c_start,
                        end: c_end,
                        text_start: atom.byte_start as u32,
                        text_end: if gi == 0 {
                            text_end as u32
                        } else {
                            atom.byte_start as u32
                        },
                    },
                });
                x += g.adv;
            }
        }
    }
    if para.separator
        && let Some(st) = para.styles.first()
    {
        // The separator character is a rule Word draws 144 pt long; the paragraph mark after
        // it is the first glyph the PDF shows.
        runs.insert(
            0,
            GlyphRun {
                face: st.face,
                size: st.size,
                bold: false,
                italic: false,
                underline: false,
                strike: false,
                color: None,
                superscript: false,
                glyphs: vec![PlacedGlyph {
                    face: st.face,
                    glyph_id: 0,
                    x: left_origin + crate::para::SEPARATOR_WIDTH,
                    y_offset: 0.0,
                    advance: 0.0,
                    cluster: Cluster {
                        kind: ClusterKind::Other,
                        start: 0,
                        end: 0,
                        text_start: 0,
                        text_end: 0,
                    },
                }],
            },
        );
    }
    Line {
        story: StoryKind::Main,
        frame: None,
        paragraph: base.para + para.index,
        start: base.story + lb.start,
        end: base.story + lb.end,
        top,
        baseline: snap_baseline(top + lb.baseline_off),
        height: lb.height,
        left: left_origin + lb.left,
        last_x: left_origin + lb.last_x,
        runs,
        text: para.text.clone(),
        label: label_text,
        role: para.role,
        border: None,
    }
}

/// The pictures that sit in the line `lb` of `para` (inline ones; floating pictures are placed
/// by the paginator).
pub(crate) fn line_images(
    para: &ParaLayout,
    lb: &LineBox,
    origin_x: f64,
    top: f64,
    base: Base,
) -> Vec<ImagePlacement> {
    let baseline = snap_baseline(top + lb.baseline_off);
    let mut out = Vec::new();
    for (k, ai) in lb.atoms.clone().enumerate() {
        let Some(o) = para.atoms[ai].object.and_then(|o| para.objects.get(o)) else {
            continue;
        };
        if !o.inline() {
            continue;
        }
        let (w, h) = o.drawing.size_pt();
        let [el, et, _, _] = o.drawing.effect_extent_pt();
        out.push(ImagePlacement {
            x: origin_x + lb.xs[k] + el,
            y: baseline - o.height + et,
            width: w,
            height: h,
            embed: o.drawing.embed.clone(),
            description: o.drawing.description.clone(),
            paragraph: base.para + para.index,
            anchored: false,
            behind_text: false,
            source: ImageSource::Main,
            text_box: o.text_box.clone(),
        });
    }
    out
}

/// A cell that spans several rows and is still being drawn.
struct ActiveSpan {
    start_row: usize,
    cell: usize,
    end_row: usize,
    /// Index into `PageBuild::cells` of the box on the current page, `None` on a new page
    /// until the next row creates it.
    box_idx: Option<usize>,
}

/// One source of lines for a row that is split: one of its cells, or a vertically merged cell
/// from above that ends in it.
struct RowPart<'t> {
    cell: &'t CellLayout,
    /// First line of the cell that belongs to the row.
    first: usize,
    /// Offset of the cell's content box below the row's top, for the first fragment.
    lead: f64,
    /// Index into the active spans for a merged cell.
    span: Option<usize>,
}

impl RowPart<'_> {
    /// Offset of line 0 of the cell below the fragment top when the fragment starts at line `at`.
    fn lead_at(&self, at: usize) -> f64 {
        if at <= self.first {
            self.lead
        } else {
            self.cell.pad.top - self.cell.lines.get(at).map_or(0.0, |l| l.dy)
        }
    }
}

struct Paginator<'a> {
    env: &'a Env<'a>,
    done: Vec<Page>,
    page: PageBuild,
    /// The table unit being placed and its offsets.
    cur: Option<(&'a UnitLayout, Base)>,
}

impl Paginator<'_> {
    fn new_page(&mut self, hard: bool, section: usize) {
        let number = self.page.number + 1;
        let first = section != self.page.section_index;
        let mut next = PageBuild::new(
            number,
            section,
            self.env.sections.get(section).copied().unwrap_or_default(),
            hard,
            first,
            self.env,
        );
        // A footnote that went on past its page continues at the top of the next page's notes.
        if let Some(last) = self.page.slots.last().copied() {
            let total = self.env.notes.line_count(self.page.section_index, last.id);
            if last.to < total && section == self.page.section_index {
                let avail = next.frame_bottom - next.top - next.sep_h;
                let mut to = last.to;
                let mut used = 0.0;
                while to < total {
                    let h = self.env.notes.slot_height(
                        section,
                        NoteSlot {
                            id: last.id,
                            from: to,
                            to: to + 1,
                        },
                    );
                    if used + h > avail + EPS && to > last.to {
                        break;
                    }
                    used += h;
                    to += 1;
                }
                next.add_slot(
                    NoteSlot {
                        id: last.id,
                        from: last.to,
                        to,
                    },
                    self.env,
                );
            }
        }
        let done = std::mem::replace(&mut self.page, next).finish(self.env);
        self.done.push(done);
    }

    /// Emits the lines of `cell` that start within `[row.top, row.top + row.height)` of the
    /// table, with the row drawn at page offset `y`.
    fn emit_cell_lines(&mut self, table: &TableLayout, cell: &CellLayout, row: &RowLayout, y: f64) {
        let origin = self.page.origin_x() + table.x + cell.x + cell.pad.left;
        let (lo, hi) = (row.top - EPS, row.top + row.height - EPS);
        for ln in &cell.lines {
            let abs = cell.content_top + ln.dy;
            if abs < lo || abs >= hi {
                continue;
            }
            self.push_line(ln, origin, y + (abs - row.top));
        }
        for bx in &cell.boxes {
            let abs = cell.content_top + bx.dy;
            if abs < lo || abs >= hi {
                continue;
            }
            self.page.cells.push(cell_box(
                origin + bx.dx,
                y + (abs - row.top),
                bx.w,
                bx.h,
                bx.meta,
                self.cur.map_or(0, |c| c.1.table),
            ));
        }
    }

    fn push_line(&mut self, ln: &FlowLine, origin: f64, top: f64) {
        let Some((unit, base)) = self.cur else { return };
        let para = &unit.arena[ln.para];
        let lb = &para.lines[ln.line];
        let mut line = build_line(para, lb, origin + ln.dx, top, base);
        line.border = para.line_border(lb, origin + ln.dx, ln.join_prev, ln.join_next);
        self.page.lines.push(line);
        let images = line_images(para, lb, origin + ln.dx, top, base);
        self.page.images.extend(images);
    }

    /// Draws row `r` whole at `y`. `active` holds the vertically merged cells that cover it.
    fn emit_row(
        &mut self,
        table: &TableLayout,
        r: usize,
        y: f64,
        active: &mut Vec<ActiveSpan>,
        repeated: bool,
    ) {
        let row = &table.rows[r];
        let origin = self.page.origin_x() + table.x;
        // Merged cells from above continue (new box when the page changed).
        for s in active.iter_mut() {
            if s.start_row < r && r <= s.end_row {
                let cell = &table.rows[s.start_row].cells[s.cell];
                match s.box_idx {
                    Some(i) => {
                        let b = &mut self.page.cells[i];
                        b.height = y + row.height - row.below - b.y;
                    }
                    None => {
                        self.page.cells.push(cell_box(
                            origin + cell.x + cell.inset_l,
                            y,
                            cell.w - cell.inset_l - cell.inset_r,
                            row.height - row.below,
                            cell.meta,
                            self.cur.map_or(0, |c| c.1.table),
                        ));
                        s.box_idx = Some(self.page.cells.len() - 1);
                    }
                }
            }
        }
        for (ci, cell) in row.cells.iter().enumerate() {
            if cell.row_span == 0 {
                continue;
            }
            self.page.cells.push(cell_box(
                origin + cell.x + cell.inset_l,
                y + row.above,
                cell.w - cell.inset_l - cell.inset_r,
                row.height - row.above - row.below,
                cell.meta,
                self.cur.map_or(0, |c| c.1.table),
            ));
            if cell.row_span > 1 && !repeated {
                active.push(ActiveSpan {
                    start_row: r,
                    cell: ci,
                    end_row: (r + cell.row_span - 1).min(table.rows.len() - 1),
                    box_idx: Some(self.page.cells.len() - 1),
                });
            }
            self.emit_cell_lines(table, cell, row, y);
        }
        let spans: Vec<(usize, usize)> = active
            .iter()
            .filter(|s| s.start_row < r && r <= s.end_row)
            .map(|s| (s.start_row, s.cell))
            .collect();
        for (sr, ci) in spans {
            self.emit_cell_lines(table, &table.rows[sr].cells[ci], row, y);
        }
        active.retain(|s| s.end_row > r);
    }

    /// Whether row `r` may be split across pages: not exact, not `cantSplit`, no merge starting in
    /// it, and every merged cell above it ends in it (the extra height such a cell needs is the
    /// row's, so Word splits the row there; measured on `layout-table-pages`).
    fn splittable(table: &TableLayout, r: usize, active: &[ActiveSpan]) -> bool {
        let row = &table.rows[r];
        !row.cant_split
            && !row.exact
            && row.cells.iter().all(|c| c.row_span <= 1)
            && active
                .iter()
                .filter(|s| s.start_row < r && r <= s.end_row)
                .all(|s| s.end_row == r)
    }

    /// The cells whose lines make up row `r` when it is split: its own and the merged cells that
    /// end in it.
    fn row_parts<'t>(table: &'t TableLayout, r: usize, active: &[ActiveSpan]) -> Vec<RowPart<'t>> {
        let row = &table.rows[r];
        let mut parts: Vec<RowPart<'t>> = row
            .cells
            .iter()
            .filter(|c| c.row_span == 1)
            .map(|cell| RowPart {
                cell,
                first: 0,
                lead: cell.pad.top,
                span: None,
            })
            .collect();
        for (i, s) in active.iter().enumerate() {
            if s.start_row < r && r <= s.end_row {
                let cell = &table.rows[s.start_row].cells[s.cell];
                let first = cell
                    .lines
                    .iter()
                    .position(|l| cell.content_top + l.dy >= row.top - EPS)
                    .unwrap_or(cell.lines.len());
                parts.push(RowPart {
                    cell,
                    first,
                    lead: cell.content_top - row.top,
                    span: Some(i),
                });
            }
        }
        parts
    }

    /// Takes as many lines of every part of a row as fit in `avail`, starting at `resume`.
    /// Returns the new cursors, the fragment height and whether the row is finished.
    fn split_fragment(
        parts: &[RowPart<'_>],
        resume: &[usize],
        avail: f64,
    ) -> (Vec<usize>, f64, bool) {
        let mut next = resume.to_vec();
        let mut height = 0.0f64;
        let mut finished = true;
        for (pi, part) in parts.iter().enumerate() {
            let (cell, start) = (part.cell, resume[pi]);
            if start >= cell.lines.len() {
                continue;
            }
            let lead = part.lead_at(start);
            let mut k = start;
            let mut used = 0.0;
            while k < cell.lines.len() {
                let ln = &cell.lines[k];
                let bottom = lead + ln.dy + ln.height;
                if bottom + cell.pad.bottom > avail + EPS {
                    break;
                }
                used = bottom;
                k += 1;
            }
            if k == cell.lines.len() {
                used = used.max(lead + cell.content_h);
                if used + cell.pad.bottom > avail + EPS {
                    // The trailing space does not fit: leave the last line for the next page.
                    k = cell.lines.len().saturating_sub(1).max(start);
                    used = if k > start {
                        let last = &cell.lines[k - 1];
                        lead + last.dy + last.height
                    } else {
                        0.0
                    };
                }
            }
            if k < cell.lines.len() {
                finished = false;
            }
            next[pi] = k;
            if k > start {
                height = height.max(used + cell.pad.bottom);
            }
        }
        (next, height, finished)
    }

    /// Draws the lines `from..to` of every part of a row at `y`. After a page break only the
    /// parts with lines left get a box (Word draws no empty cell on a continuation page).
    #[allow(clippy::too_many_arguments)]
    fn emit_fragment(
        &mut self,
        table: &TableLayout,
        parts: &[RowPart<'_>],
        active: &mut [ActiveSpan],
        y: f64,
        from: &[usize],
        to: &[usize],
        height: f64,
        resumed: bool,
    ) {
        let origin = self.page.origin_x() + table.x;
        for (pi, part) in parts.iter().enumerate() {
            let cell = part.cell;
            let n = cell.lines.len();
            if !resumed || from[pi] < n {
                let table_id = self.cur.map_or(0, |c| c.1.table);
                let existing = part.span.and_then(|i| active[i].box_idx);
                if let Some(i) = existing {
                    let b = &mut self.page.cells[i];
                    b.height = y + height - b.y;
                } else {
                    self.page.cells.push(cell_box(
                        origin + cell.x + cell.inset_l,
                        y,
                        cell.w - cell.inset_l - cell.inset_r,
                        height,
                        cell.meta,
                        table_id,
                    ));
                    if let Some(i) = part.span {
                        active[i].box_idx = Some(self.page.cells.len() - 1);
                    }
                }
            }
            let lead = part.lead_at(from[pi]);
            for ln in &cell.lines[from[pi].min(n)..to[pi].min(n)] {
                self.push_line(ln, origin + cell.pad.left + cell.x, y + lead + ln.dy);
            }
        }
    }

    /// Places `table` starting where the page cursor is (`gap` below it).
    fn place_table(&mut self, table: &TableLayout, gap: f64) {
        let mut y = self.page.y + gap;
        let mut active: Vec<ActiveSpan> = Vec::new();
        let mut r = 0usize;
        let mut resume: Option<Vec<usize>> = None;
        while r < table.rows.len() {
            let row = &table.rows[r];
            let avail = self.page.bottom() - y;
            let at_top = !self.page.has_content();
            if let Some(cursors) = resume.clone() {
                let parts = Self::row_parts(table, r, &active);
                let (to, h, finished) = Self::split_fragment(&parts, &cursors, avail);
                if h > 0.0 || finished {
                    self.emit_fragment(table, &parts, &mut active, y, &cursors, &to, h, true);
                    y += h;
                    if finished {
                        resume = None;
                        active.retain(|s| s.end_row > r);
                        r += 1;
                        continue;
                    }
                    resume = Some(to);
                }
                self.page.y = y;
                self.break_in_table(table, r, &mut y, &mut active);
                continue;
            }
            if row.height <= avail + EPS || (at_top && !Self::splittable(table, r, &active)) {
                self.emit_row(table, r, y, &mut active, false);
                y += row.height;
                r += 1;
                continue;
            }
            if Self::splittable(table, r, &active) {
                let parts = Self::row_parts(table, r, &active);
                let zero: Vec<usize> = parts.iter().map(|p| p.first).collect();
                let (to, h, finished) = Self::split_fragment(&parts, &zero, avail);
                if h > 0.0 || (at_top && finished) {
                    self.emit_fragment(table, &parts, &mut active, y, &zero, &to, h, false);
                    y += h;
                    if finished {
                        active.retain(|s| s.end_row > r);
                        r += 1;
                        continue;
                    }
                    resume = Some(to);
                    self.page.y = y;
                    self.break_in_table(table, r, &mut y, &mut active);
                    continue;
                }
                if at_top {
                    // Not even one line fits on an empty page: take one anyway.
                    let to: Vec<usize> = parts
                        .iter()
                        .map(|p| (p.first + 1).min(p.cell.lines.len()).max(p.first))
                        .collect();
                    let h = row.height.min(avail.max(1.0));
                    self.emit_fragment(table, &parts, &mut active, y, &zero, &to, h, false);
                    y += h;
                    resume = Some(to);
                    self.page.y = y;
                    self.break_in_table(table, r, &mut y, &mut active);
                    continue;
                }
            }
            // Move the whole row to the next page.
            self.page.y = y;
            self.break_in_table(table, r, &mut y, &mut active);
        }
        self.page.y = y;
    }

    /// Starts a new page inside a table, repeating the header rows when row `r` is a body row.
    fn break_in_table(
        &mut self,
        table: &TableLayout,
        r: usize,
        y: &mut f64,
        active: &mut [ActiveSpan],
    ) {
        let section = self.page.section_index;
        self.new_page(false, section);
        for s in active.iter_mut() {
            s.box_idx = None;
        }
        *y = self.page.y;
        if table.header_rows > 0 && r >= table.header_rows {
            let mut none: Vec<ActiveSpan> = Vec::new();
            for h in 0..table.header_rows {
                self.emit_row(table, h, *y, &mut none, true);
                *y += table.rows[h].height;
            }
        }
    }
}

impl Checkpoint {
    /// Whether two states lead to the same pages from here on. The page count may differ.
    fn same_future(&self, other: &Self) -> bool {
        self.lines == other.lines
            && self.cells == other.cells
            && self.images == other.images
            && self.floats == other.floats
            && self.floats_sig == other.floats_sig
            && self.notes == other.notes
            && self.y == other.y
            && self.hard_start == other.hard_start
            && self.force_break == other.force_break
            && self.section_index == other.section_index
            && (self.pages_done > 0) == (other.pages_done > 0)
    }
}

/// Where a pagination run picks up: the pages before it and the state at its first item.
pub(crate) struct Resume {
    /// Index of the first item to place.
    pub item: usize,
    /// Pages finished before the current one.
    pub prior: Vec<Page>,
    /// The current page, cut back to the lines and boxes the checkpoint counts.
    pub page: Page,
    pub checkpoint: Checkpoint,
    /// Checkpoints of the items before `item`.
    pub checkpoints: Vec<Checkpoint>,
}

/// An earlier run whose pages may be reused once this run's state matches it again.
pub(crate) struct Previous<'a> {
    pub items: &'a [FlowItem],
    /// One per item plus one for the end.
    pub checkpoints: &'a [Checkpoint],
    /// Items from this index on are the earlier run's last items, unchanged.
    pub first: usize,
}

pub(crate) struct Paginated {
    /// Finished pages. When the run converged, the page in progress is in `open`.
    pub pages: Vec<Page>,
    pub open: Option<Page>,
    /// One per item placed or skipped over, plus the end when the run did not converge.
    pub checkpoints: Vec<Checkpoint>,
    /// The item at which the state matched the earlier run.
    pub converged: Option<usize>,
}

/// A fingerprint of the floats' rectangles.
fn floats_sig(floats: &[Float]) -> u64 {
    floats.iter().fold(0u64, |h, f| {
        [f.x0, f.y0, f.x1, f.y1]
            .iter()
            .fold(h, |h, v| h.rotate_left(7) ^ v.to_bits())
    })
}

fn checkpoint(pg: &Paginator<'_>, force_break: bool) -> Checkpoint {
    Checkpoint {
        pages_done: pg.done.len(),
        lines: pg.page.lines.len(),
        cells: pg.page.cells.len(),
        images: pg.page.images.len(),
        floats: pg.page.floats.len(),
        floats_sig: floats_sig(&pg.page.floats),
        y: pg.page.y,
        hard_start: pg.page.hard_start,
        force_break,
        section_index: pg.page.section_index,
        notes: pg.page.slots.len(),
    }
}

/// Places `items` on pages, from the start or from `resume`. With `previous` it stops as soon
/// as the state equals the earlier run's at the same item (see [`Paginated::converged`]).
pub(crate) fn paginate<'a>(
    items: &'a [FlowItem],
    env: &'a Env<'a>,
    resume: Option<Resume>,
    previous: Option<&Previous<'_>>,
) -> Paginated {
    let (start, mut checkpoints, mut pg, mut force_break) = match resume {
        Some(r) => {
            let carry = r
                .item
                .checked_sub(1)
                .map(|i| items[i].carry())
                .unwrap_or_default();
            let mut slots = r.page.note_slots;
            slots.truncate(r.checkpoint.notes);
            let section_index = r.checkpoint.section_index;
            let page = PageBuild {
                number: r.page.number,
                section: r.page.section,
                section_index,
                lines: r.page.lines,
                cells: r.page.cells,
                images: r.page.images,
                floats: r.page.floats,
                first_in_section: r.page.first_in_section,
                top: r.page.body_top,
                frame_bottom: r.page.body_bottom,
                notes_h: slots
                    .iter()
                    .map(|s| env.notes.slot_height(section_index, *s))
                    .sum(),
                sep_h: env.notes.sep_height(section_index),
                slots,
                y: r.checkpoint.y,
                pending_after: carry.after,
                prev_style: carry.style,
                prev_contextual: carry.contextual,
                hard_start: r.checkpoint.hard_start,
            };
            let pg = Paginator {
                env,
                done: r.prior,
                page,
                cur: None,
            };
            (r.item, r.checkpoints, pg, r.checkpoint.force_break)
        }
        None => {
            let first_section = items.first().map_or(0, FlowItem::section);
            let page = PageBuild::new(
                1,
                first_section,
                env.sections.get(first_section).copied().unwrap_or_default(),
                true,
                true,
                env,
            );
            let pg = Paginator {
                env,
                done: Vec::new(),
                page,
                cur: None,
            };
            (0, Vec::with_capacity(items.len() + 1), pg, false)
        }
    };

    for p in start..items.len() {
        if crate::cancel::cancelled() {
            break;
        }
        let ck = checkpoint(&pg, force_break);
        if let Some(prev) = previous
            && p >= prev.first
            && p > 0
        {
            let o = p + prev.items.len() - items.len();
            let same = prev.checkpoints.get(o).is_some_and(|c| ck.same_future(c))
                && Carry {
                    after: pg.page.pending_after,
                    style: pg.page.prev_style.clone(),
                    contextual: pg.page.prev_contextual,
                } == prev.items[o - 1].carry();
            if same {
                return Paginated {
                    pages: pg.done,
                    open: Some(pg.page.finish(env)),
                    checkpoints,
                    converged: Some(p),
                };
            }
        }
        checkpoints.push(ck);

        let item = &items[p];
        let section = item.section();
        let base = item.base();
        let (para, table) = (item.para(), item.table());
        let wants_break = force_break || para.is_some_and(|p| p.flow.page_break_before);
        if section != pg.page.section_index || (wants_break && pg.page.has_content()) {
            // Space before is dropped at the top of every page after the first, whether the page
            // began naturally, after a break character or at `pageBreakBefore`; a new section
            // keeps it (layout-pagination).
            pg.new_page(section != pg.page.section_index, section);
        }
        force_break = false;

        if let Some(table) = table {
            let gap = if pg.page.has_content() {
                pg.page.pending_after
            } else {
                0.0
            };
            pg.cur = Some((item.unit.as_ref(), base));
            pg.place_table(table, gap);
            pg.page.pending_after = 0.0;
            pg.page.prev_style = None;
            pg.page.prev_contextual = false;
            continue;
        }
        let Some(para) = para else { continue };

        let contextual_same =
            |pg: &PageBuild| pg.prev_style.as_ref().is_some_and(|s| *s == para.style_id);
        let has_anchor = para.objects.iter().any(|o| !o.inline());
        let mut from_atom = 0usize;
        let n_atoms = para.atoms.len();
        while from_atom < n_atoms {
            let first_segment = from_atom == 0;
            // Spacing above the first line of this paragraph.
            let gap = |pg: &PageBuild| -> f64 {
                if !first_segment {
                    return 0.0;
                }
                let mut after = pg.pending_after;
                let mut before = f64::from(para.props.space_before_pt);
                if contextual_same(pg) {
                    if pg.prev_contextual {
                        after = 0.0;
                    }
                    if para.flow.contextual_spacing {
                        before = 0.0;
                    }
                }
                if !pg.has_content() {
                    after = 0.0;
                    if !pg.hard_start && pg.number > 1 {
                        before = 0.0;
                    }
                }
                // Word does not add space after and space before; the larger one separates.
                after.max(before)
            };
            let at_top = !pg.page.has_content();
            // Paragraph borders (layout-pbdr-*): a group of paragraphs with identical borders has
            // its top edge above the first and its bottom edge below the last; a between rule
            // separates the paragraphs inside.
            let joined_prev =
                !at_top && p > 0 && items[p - 1].para().is_some_and(|q| q.joins(para));
            let joined_next = items
                .get(p + 1)
                .and_then(FlowItem::para)
                .is_some_and(|q| para.joins(q));
            let border_above = if !first_segment {
                0.0
            } else if joined_prev {
                para.between_extent()
            } else {
                para.border_top()
            };
            let y0 = pg.page.y + gap(&pg.page) + border_above;
            // Floating pictures change the lines: break them again around the pictures.
            let around = has_anchor || pg.page.floats.iter().any(|f| f.y1 > y0 - EPS);
            let dynamic = around.then(|| {
                let frame = Frame {
                    section: pg.page.section,
                };
                dynamic_lines(para, from_atom, y0, &pg.page.floats, &frame, base)
            });
            let lines: &[LineBox] = match &dynamic {
                Some(d) => &d.lines,
                None => {
                    let from = para.lines.partition_point(|l| l.atoms.start < from_atom);
                    &para.lines[from..]
                }
            };
            let notes_at = NotesAt {
                env,
                section: pg.page.section_index,
                frame_bottom: pg.page.frame_bottom,
                count: pg.page.slots.len(),
                height: pg.page.notes_h,
                sep: pg.page.sep_h,
            };
            let mut place = decide(
                para,
                lines,
                first_segment,
                y0,
                pg.page.bottom(),
                at_top,
                Some(&notes_at),
            );
            if first_segment
                && place.take == lines.len()
                && para.flow.keep_next
                && !at_top
                && p + 1 < items.len()
            {
                let used: f64 = lines.iter().map(|l| l.lead + l.height).sum();
                let ok = can_start(
                    items,
                    p + 1,
                    y0 + used,
                    f64::from(para.props.space_after_pt),
                    pg.page.bottom(),
                    0,
                );
                if !ok {
                    // The last line cannot sit with its follower here: Word moves as few lines
                    // as it may (never leaving a lone line when widow control is on) and
                    // only moves the whole paragraph when it cannot be split.
                    let n = lines.len();
                    let (min_head, min_tail) = if para.flow.widow_control {
                        (2, 2)
                    } else {
                        (1, 1)
                    };
                    let head = n.saturating_sub(min_tail);
                    place.take = if para.flow.keep_lines || head < min_head {
                        0
                    } else {
                        head
                    };
                }
            }
            if place.take == 0 {
                // Nothing of this paragraph goes here: move to a fresh page and retry.
                pg.new_page(false, section);
                continue;
            }
            let split = place.split;
            let mut y = y0;
            let mut placed = 0usize;
            let mut broke = false;
            let origin = pg.page.origin_x();
            for lb in &lines[..place.take] {
                y += lb.lead;
                let mut line = build_line(para, lb, origin, y, base);
                line.story = item.story;
                line.border = para.line_border(lb, origin, joined_prev, joined_next);
                let images = line_images(para, lb, origin, y, base);
                y += lb.height;
                pg.page.lines.push(line);
                pg.page.images.extend(images);
                for (a, id) in &para.foot_refs {
                    if lb.atoms.contains(a) {
                        match split {
                            Some((sid, m)) if sid == *id && placed + 1 == place.take => {
                                pg.page.add_slot(
                                    NoteSlot {
                                        id: sid,
                                        from: 0,
                                        to: m,
                                    },
                                    env,
                                );
                            }
                            _ => pg.page.add_note(*id, env),
                        }
                    }
                }
                placed += 1;
                if lb.page_break_after {
                    broke = true;
                    break;
                }
            }
            let end_atom = lines[placed - 1].atoms.end;
            if let Some(d) = &dynamic {
                for a in d.anchors.iter().filter(|a| a.atom < end_atom) {
                    pg.page.images.push(a.image.clone());
                    pg.page.floats.extend(a.float);
                }
            }
            if end_atom >= n_atoms && !joined_next {
                y += para.border_bottom();
            }
            pg.page.y = y;
            from_atom = end_atom;
            if from_atom < n_atoms {
                pg.new_page(broke, section);
            } else if broke {
                force_break = true;
            }
        }
        pg.page.pending_after = f64::from(para.props.space_after_pt);
        pg.page.prev_style = Some(para.style_id.clone());
        pg.page.prev_contextual = para.flow.contextual_spacing;
    }
    checkpoints.push(checkpoint(&pg, force_break));
    let last = pg.page.finish(env);
    pg.done.push(last);
    Paginated {
        pages: pg.done,
        open: None,
        checkpoints,
        converged: None,
    }
}
