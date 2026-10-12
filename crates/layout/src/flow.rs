//! Page flow: places laid-out paragraphs and tables onto pages with Word's rules for spacing,
//! widow and orphan control, keep-with-next, keep-lines, page breaks, sections, table row
//! splitting and repeated header rows.

use std::sync::Arc;

use crate::build::UnitLayout;
use crate::float::{Float, FloatKind, Frame, ahead_floats, dynamic_lines, place};
use crate::frame::{FrameSpec, FrameWrap};
use crate::furniture::{Furniture, NoteStore};
use crate::para::{AtomClass, LineBox, ParaLayout};
use crate::props::{SectionCols, SectionProps, SectionStart};
use crate::table::{
    CellLayout, CellMeta, FlowImage, FlowLine, ImageKind, PosAnchor, PosSpec, RowLayout,
    TableLayout, TablePos,
};
use crate::{
    CellBox, Cluster, ClusterKind, GlyphRun, ImagePlacement, ImageSource, Line, NoteSlot, Page,
    PlacedGlyph, StoryKind,
};
use open365_docx::WrapSide;

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
    /// Columns and start type of every section.
    pub cols: &'a [SectionCols],
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
    /// The last paragraph ended with a column break.
    pub force_column: bool,
    pub section_index: usize,
    /// The section the content being placed belongs to, and its current column.
    pub cur: usize,
    pub col: usize,
    /// Footnotes on the current page.
    pub notes: usize,
    /// Page and top of the next paragraph as laid out before its floats pushed this one.
    pub anchor_top: Option<(usize, f64)>,
}

#[derive(Clone)]
struct PageBuild {
    number: usize,
    /// Geometry of the page's first section (what the page reports).
    section: SectionProps,
    section_index: usize,
    /// The section the content being placed belongs to (a continuous section can start in the
    /// middle of the page), its geometry and its current column.
    cur: usize,
    geom: SectionProps,
    col: usize,
    /// Left edge of the column, from the left edge of the text area, and its width.
    col_x: f64,
    col_w: f64,
    /// Top of the current column region: the top of the body, or where a continuous section
    /// started.
    col_top: f64,
    /// Lines and cell boxes on the page when the current column began.
    col_mark: (usize, usize),
    /// The column region starts below content of the page (a continuous section).
    inherited: bool,
    /// Where each finished column of the current region ended: the cursor and the space after
    /// its last paragraph.
    col_ends: Vec<(f64, f64)>,
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
    /// Page number and top of the next paragraph before the floats it anchors pushed the
    /// paragraph above it; Word places those floats from this top.
    anchor_top: Option<(usize, f64)>,
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
        let (col_x, col_w) = env.cols.get(section_index).map_or_else(
            || (0.0, section.text_width()),
            |c| c.columns.first().map_or((0.0, 0.0), |g| (g.x, g.width)),
        );
        Self {
            number,
            section,
            section_index,
            cur: section_index,
            geom: section,
            col: 0,
            col_x,
            col_w,
            col_top: frame.top,
            col_mark: (0, 0),
            inherited: false,
            col_ends: Vec::new(),
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
            anchor_top: None,
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
            self.notes_h = env.notes.area(self.section_index, &self.slots);
        }
    }

    fn add_note(&mut self, id: i64, env: &Env<'_>) {
        let to = env.notes.line_count(self.section_index, id);
        self.add_slot(NoteSlot { id, from: 0, to }, env);
    }

    /// Whether the current column holds something, or lies below something on the page.
    fn has_content(&self) -> bool {
        self.inherited || self.lines.len() > self.col_mark.0 || self.cells.len() > self.col_mark.1
    }

    /// Whether anything is on the page.
    fn page_has_content(&self) -> bool {
        !self.lines.is_empty() || !self.cells.is_empty()
    }

    /// Left edge of the current column.
    fn origin_x(&self) -> f64 {
        self.geom.margin_left + self.geom.gutter + self.col_x
    }

    /// Left and right edge of the current column, page coordinates.
    fn col_span(&self) -> (f64, f64) {
        let l = self.origin_x();
        (l, l + self.col_w)
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
    slots: &'a [NoteSlot],
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
    let mut slots = n.slots.to_vec();
    let (mut y, mut nh, mut has, mut k) = (
        y0,
        n.env.notes.area(n.section, &slots),
        !slots.is_empty(),
        0,
    );
    for l in lines {
        y += l.lead;
        let mut any = false;
        let mut ids = Vec::new();
        let mut with = slots.clone();
        for (a, id) in &para.foot_refs {
            if l.atoms.contains(a) {
                let to = n.env.notes.line_count(n.section, *id);
                any |= n.env.notes.height(n.section, *id) > 0.0;
                with.push(NoteSlot {
                    id: *id,
                    from: 0,
                    to,
                });
                ids.push(*id);
            }
        }
        let add = n.env.notes.area(n.section, &with) - nh;
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
        slots = with;
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
    if items[p].unit.frame.is_some() {
        return true;
    }
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
/// An edge on that grid.
pub(crate) fn snap_edge(v: f64) -> f64 {
    ((v / BASELINE_GRID) + 1e-9).round() * BASELINE_GRID
}

fn snap_cell(c: CellBox) -> CellBox {
    let snap = snap_edge;
    let (left, right) = (snap(c.x), snap(c.x + c.width));
    let (top, bottom) = (snap(c.y), snap(c.y + c.height));
    // Word snaps the clip rectangle (the box inset by the padding), not the padding: a 2 pt
    // padding is no multiple of 0.24 pt (`layout-trh-asym`).
    let clip_top = snap(c.y + c.pad_top);
    let clip_bottom = snap(c.y + c.height - c.pad_bottom);
    CellBox {
        x: left,
        width: right - left,
        y: top,
        height: bottom - top,
        pad_top: clip_top - top,
        pad_bottom: bottom - clip_bottom,
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
            vert_align: open365_docx::VertAlign::Baseline,
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
                vert_align: st.vert_align,
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
    for span in &lb.leaders {
        let style = para.atoms[span.atom].style;
        let Some(lg) = para
            .leader_glyphs
            .iter()
            .find(|g| g.style == style && g.ch == span.ch)
        else {
            continue;
        };
        // Leader glyphs sit on a grid of one pitch from the left edge of the text area, from
        // the tab's start to the last one that ends by the tab's target (`layout-tabs`).
        let st = &para.styles[style];
        let mut glyphs = Vec::new();
        let mut k = ((span.x0 - 0.1) / lg.pitch).ceil().max(0.0);
        loop {
            let gx = k * lg.pitch;
            if gx + lg.pitch > span.x1 + 0.05 {
                break;
            }
            glyphs.push(PlacedGlyph {
                face: lg.face,
                glyph_id: lg.glyph_id,
                x: left_origin + gx,
                y_offset: 0.0,
                advance: lg.pitch,
                cluster: Cluster {
                    kind: ClusterKind::Other,
                    start: 0,
                    end: 0,
                    text_start: u32::from(span.ch),
                    text_end: u32::from(span.ch),
                },
            });
            k += 1.0;
        }
        if !glyphs.is_empty() {
            runs.push(GlyphRun {
                face: lg.face,
                size: st.size,
                bold: st.bold,
                italic: st.italic,
                underline: false,
                strike: false,
                color: st.color,
                superscript: st.superscript,
                vert_align: open365_docx::VertAlign::Baseline,
                glyphs,
            });
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
                vert_align: open365_docx::VertAlign::Baseline,
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

/// The state of the page when the first content of a column region was about to be placed,
/// and the item and atom it was placed from: what balancing starts over from.
struct Region {
    item: usize,
    from_atom: usize,
    page: PageBuild,
}

struct Paginator<'a> {
    env: &'a Env<'a>,
    done: Vec<Page>,
    page: PageBuild,
    /// The table unit being placed and its offsets.
    cur: Option<(&'a UnitLayout, Base)>,
    region: Option<Region>,
    /// A balancing trial: running out of columns is recorded in `overflow` instead of starting
    /// a new page.
    trial: bool,
    overflow: bool,
    /// Page x of the grid's left line while a floating table is being drawn.
    float_origin: Option<f64>,
}

/// Spacing above the first line of `para` when it follows what is on `pg`.
fn first_gap(pg: &PageBuild, para: &ParaLayout) -> f64 {
    if para.section_mark {
        return 0.0;
    }
    let contextual_same = pg.prev_style.as_ref().is_some_and(|s| *s == para.style_id);
    let mut after = pg.pending_after;
    let mut before = f64::from(para.props.space_before_pt);
    if contextual_same {
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
}

/// How many columns the section has (one for a section without columns).
fn column_count(env: &Env<'_>, section: usize) -> usize {
    env.cols.get(section).map_or(1, |c| c.columns.len().max(1))
}

impl<'a> Paginator<'a> {
    fn start(env: &'a Env<'a>, page: PageBuild) -> Self {
        Self {
            env,
            done: Vec::new(),
            page,
            cur: None,
            region: None,
            trial: false,
            overflow: false,
            float_origin: None,
        }
    }

    /// Page x of the left grid line of `table`.
    fn table_origin(&self, table: &TableLayout) -> f64 {
        self.float_origin
            .unwrap_or_else(|| self.page.origin_x() + table.x)
    }

    /// Sets the current column of the current section.
    fn set_column(&mut self, col: usize) {
        let g = self
            .env
            .cols
            .get(self.page.cur)
            .and_then(|c| c.columns.get(col).or(c.columns.first()))
            .copied();
        self.page.col = col;
        match g {
            Some(g) => {
                self.page.col_x = g.x;
                self.page.col_w = g.width;
            }
            None => {
                self.page.col_x = 0.0;
                self.page.col_w = self.page.geom.text_width();
            }
        }
    }

    /// Moves on to the next column of the section, or starts a new page after the last one.
    fn next_column_or_page(&mut self, hard: bool, section: usize) {
        if section == self.page.cur && self.page.col + 1 < column_count(self.env, section) {
            self.next_column();
        } else if self.trial {
            self.overflow = true;
        } else {
            self.new_page(hard, section);
        }
    }

    fn next_column(&mut self) {
        let pg = &mut self.page;
        pg.col_ends.push((pg.y, pg.pending_after));
        pg.col_mark = (pg.lines.len(), pg.cells.len());
        pg.inherited = false;
        pg.y = pg.col_top;
        pg.pending_after = 0.0;
        pg.prev_style = None;
        pg.prev_contextual = false;
        pg.hard_start = false;
        pg.anchor_top = None;
        let col = pg.col + 1;
        self.set_column(col);
    }

    fn new_page(&mut self, hard: bool, section: usize) {
        if self.trial {
            self.overflow = true;
            return;
        }
        self.region = None;
        let mut number = self.page.number + 1;
        let first = section != self.page.cur;
        let props = self.env.sections.get(section).copied().unwrap_or_default();
        // An odd-page or even-page section starts on a page of that parity: Word inserts a
        // blank page when the next one has the wrong one.
        let blank = first
            && match self.env.cols.get(section).map(|c| c.start) {
                Some(SectionStart::OddPage) => number.is_multiple_of(2),
                Some(SectionStart::EvenPage) => !number.is_multiple_of(2),
                _ => false,
            };
        if blank {
            let page = PageBuild::new(number, section, props, true, true, self.env);
            let done = std::mem::replace(&mut self.page, page).finish(self.env);
            self.done.push(done);
            number += 1;
        }
        let mut next = PageBuild::new(number, section, props, hard, first && !blank, self.env);
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
        let origin = self.table_origin(table) + cell.x + cell.pad.left;
        let (lo, hi) = (row.top - EPS, row.top + row.height - EPS);
        for ln in &cell.lines {
            let abs = cell.content_top + ln.dy;
            if abs < lo || abs >= hi {
                continue;
            }
            self.push_line(ln, origin, y + (abs - row.top));
        }
        let content_y = y + (cell.content_top - row.top);
        for im in &cell.images {
            let abs = cell.content_top + cell.lines.get(im.line).map_or(0.0, |l| l.dy);
            if abs >= lo && abs < hi {
                self.push_cell_image(im, cell, origin, content_y);
            }
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

    /// Draws a floating drawing of a cell. `origin` is the left of the content box and
    /// `content_y` its top on the page.
    fn push_cell_image(&mut self, im: &FlowImage, cell: &CellLayout, origin: f64, content_y: f64) {
        let Some((_, base)) = self.cur else { return };
        match &im.kind {
            ImageKind::Cell(p) => {
                let mut p = p.clone();
                p.x = origin + im.dx;
                p.y = content_y + im.dy;
                p.paragraph += base.para;
                self.page.images.push(p);
            }
            ImageKind::Page(d) => {
                // Word counts the paragraph from the outer edge of the cell's top border
                // (layout-float-incell-compat14).
                let frame = Frame {
                    section: self.page.geom,
                    col: self.page.col_span(),
                };
                let a = place(
                    &d.drawing,
                    d.atom,
                    &frame,
                    content_y + im.dy - cell.border_top,
                    base.para + d.paragraph,
                    d.text_box.clone(),
                    None,
                    d.polygon.as_ref(),
                );
                self.page.images.push(a.image);
                self.page.floats.extend(a.float);
            }
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
        let origin = self.table_origin(table);
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
            for im in &cell.images {
                if (from[pi].min(n)..to[pi].min(n)).contains(&im.line) {
                    self.push_cell_image(im, cell, origin + cell.pad.left + cell.x, y + lead);
                }
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
            if self.overflow {
                break;
            }
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
                let (to, mut h, finished) = Self::split_fragment(&parts, &zero, avail);
                if at_top && finished && !row.exact {
                    // Measured (layout-trh-tall): a row taller than the page whose content fits
                    // is clamped to the page, not split.
                    h = h.max(row.height.min(avail));
                }
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
    /// The position of `table` when it leaves the flow. A table with no room for text beside
    /// it (as wide as the column with its distances) stays in the flow: its anchor paragraph
    /// goes below it either way, and the corpus' full-width layout tables match that way
    /// (`7e53ee0d`: 16 lines in the flow, 9 floating).
    fn floating(&self, table: &TableLayout) -> Option<TablePos> {
        table.pos.filter(|p| {
            p.y_spec != Some(PosSpec::Inline)
                && table.outer_width() + p.dist[0] + p.dist[2] < self.page.col_w - 1.0
        })
    }

    /// Where a table with `w:tblpPr` goes: its outer left edge and its top, for an anchor
    /// paragraph whose top is `anchor_top`, clear of the earlier tables in `floats`.
    fn table_spot(
        &self,
        table: &TableLayout,
        pos: &TablePos,
        anchor_top: f64,
        floats: &[Float],
    ) -> (f64, f64) {
        let g = self.page.geom;
        let (w, h) = (table.outer_width(), table.height());
        let along = |spec: Option<PosSpec>, base: (f64, f64), off: f64, size: f64| match spec {
            Some(PosSpec::Start) => base.0,
            Some(PosSpec::Center) => base.0 + (base.1 - base.0 - size) / 2.0,
            Some(PosSpec::End) => base.1 - size,
            _ => base.0 + off,
        };
        let hbase = match pos.horz {
            PosAnchor::Text => self.page.col_span(),
            PosAnchor::Margin => (g.margin_left + g.gutter, g.page_width - g.margin_right),
            PosAnchor::Page => (0.0, g.page_width),
        };
        let vbase = match pos.vert {
            PosAnchor::Text => (anchor_top, anchor_top),
            PosAnchor::Margin => (g.margin_top, g.page_height - g.margin_bottom),
            PosAnchor::Page => (0.0, g.page_height),
        };
        let mut left = along(pos.x_spec, hbase, pos.x, w);
        let top = along(pos.y_spec, vbase, pos.y, h);
        // `tblOverlap="never"`: a table that covers an earlier one moves right until its text
        // area meets the earlier one's (layout-tblp-never: 9 pt either side).
        for _ in 0..8 {
            let hit = floats.iter().find(|f| {
                f.table
                    && f.rect[1] < top + h - EPS
                    && f.rect[3] > top + EPS
                    && f.x0 < left + w - EPS
                    && f.x1 > left + EPS
            });
            match hit {
                Some(f) if !pos.overlap || f.no_overlap => left = f.x1 + pos.dist[0],
                _ => break,
            }
        }
        (left, top)
    }

    /// The area text keeps out of around the `h` points of `table` that stand at `(left, top)`.
    fn table_float(table: &TableLayout, pos: &TablePos, left: f64, top: f64, h: f64) -> Float {
        let w = table.outer_width();
        let [dl, dt, dr, db] = pos.dist;
        Float {
            x0: left - dl,
            y0: top - dt,
            x1: left + w + dr,
            y1: top + h + db,
            kind: FloatKind::Square,
            side: WrapSide::BothSides,
            rect: [left, top, left + w, top + h],
            outline: None,
            table: true,
            frame: false,
            no_overlap: !pos.overlap,
        }
    }

    /// Draws a table with `w:tblpPr` where its position says and makes the text of the
    /// following paragraphs wrap around it. The table leaves the flow: the cursor stays, and
    /// the next paragraph (its anchor) starts where it would have without the table.
    fn place_float_table(
        &mut self,
        items: &'a [FlowItem],
        p: usize,
        table: &TableLayout,
        pos: &TablePos,
    ) {
        let anchor = items[p + 1..]
            .iter()
            .find(|i| !self.leaves_flow(i))
            .and_then(FlowItem::para);
        let anchor_top = match self.page.anchor_top {
            Some((page, top)) if page == self.page.number => top,
            _ => self.page.y + anchor.map_or(0.0, |a| first_gap(&self.page, a) + a.border_top()),
        };
        let (left, top) = self.table_spot(table, pos, anchor_top, &self.page.floats);
        // A table that runs past the bottom margin breaks between rows (layout-tblp-bottom):
        // the rows that fit stay, the rest start the next page at the top margin, and the
        // anchor paragraph moves there with them (its text wraps beside the second part).
        let bottom = self.page.bottom();
        let fit = table
            .rows
            .iter()
            .take_while(|r| top + r.top + r.height <= bottom + EPS)
            .count();
        let crossing = table.rows[..fit.min(table.rows.len())]
            .iter()
            .enumerate()
            .any(|(r, row)| row.cells.iter().any(|c| r + c.row_span > fit));
        let split = (1..table.rows.len()).contains(&fit)
            && !crossing
            && !self.trial
            && table.height() - table.rows[fit].top <= self.page.bottom() - self.page.col_top;
        let end = if split { fit } else { table.rows.len() };
        self.emit_float_rows(table, pos, left, top, 0..end);
        if split {
            let section = self.page.cur;
            self.next_column_or_page(false, section);
            if self.overflow {
                return;
            }
            let top = self.page.y;
            self.emit_float_rows(table, pos, left, top, fit..table.rows.len());
        }
    }

    /// Draws rows `rows` of a floating table with the first at `top`, and makes the text keep
    /// out of them.
    fn emit_float_rows(
        &mut self,
        table: &TableLayout,
        pos: &TablePos,
        left: f64,
        top: f64,
        rows: std::ops::Range<usize>,
    ) {
        let before = self.page.cells.len();
        self.float_origin = Some(left + table.edge_l / 2.0);
        let mut active: Vec<ActiveSpan> = Vec::new();
        let (first, last) = (&table.rows[rows.start], &table.rows[rows.end - 1]);
        // A part that continues the table on a later page starts with the table's top border again.
        let lead = if rows.start > 0 {
            table.rows[0].above
        } else {
            0.0
        };
        let (base, h) = (first.top - lead, last.top + last.height - first.top + lead);
        for r in rows {
            self.emit_row(table, r, top + table.rows[r].top - base, &mut active, false);
        }
        self.float_origin = None;
        // The cells do not make the column non-empty: the anchor paragraph is still its first.
        self.page.col_mark.1 += self.page.cells.len() - before;
        self.page
            .floats
            .push(Self::table_float(table, pos, left, top, h));
    }

    /// Whether `item` leaves the flow: a floating table or a paragraph frame.
    fn leaves_flow(&self, item: &FlowItem) -> bool {
        item.unit.frame.is_some() || item.table().is_some_and(|t| self.floating(t).is_some())
    }

    /// Where a frame goes: the left edge of its text box and the top of its clip rectangle, for
    /// an anchor paragraph whose top is `anchor_top`.
    fn frame_spot(&self, g: &FrameGeom, anchor_top: f64) -> (f64, f64) {
        let pr = &g.spec.pr;
        let geo = self.page.geom;
        let w = g.spec.box_w;
        let along = |spec: Option<PosSpec>, base: (f64, f64), off: f64, size: f64| match spec {
            Some(PosSpec::Start) => base.0,
            Some(PosSpec::Center) => base.0 + (base.1 - base.0 - size) / 2.0,
            Some(PosSpec::End) => base.1 - size,
            _ => base.0 + off,
        };
        let hbase = match pr.horz {
            PosAnchor::Text => self.page.col_span(),
            PosAnchor::Margin => (
                geo.margin_left + geo.gutter,
                geo.page_width - geo.margin_right,
            ),
            PosAnchor::Page => (0.0, geo.page_width),
        };
        let vbase = match pr.vert {
            PosAnchor::Text => (anchor_top, anchor_top),
            PosAnchor::Margin => (geo.margin_top, geo.page_height - geo.margin_bottom),
            PosAnchor::Page => (0.0, geo.page_height),
        };
        let left = if pr.drop_cap == Some(true) {
            // A drop cap in the margin hangs left of the text.
            self.page.col_span().0 - w - g.gr
        } else {
            along(pr.x_spec, hbase, pr.x, w)
        };
        (left, along(pr.y_spec, vbase, pr.y, g.height))
    }

    /// The area text keeps out of around a frame whose text box starts at `left`: the clip
    /// rectangle horizontally, the clip rectangle grown by `vSpace` vertically.
    fn frame_float(g: &FrameGeom, left: f64, top: f64) -> Float {
        let pr = &g.spec.pr;
        let (cl, cr) = (left - g.gl, left + g.spec.box_w + g.gr);
        Float {
            x0: cl,
            y0: top - pr.v_space,
            x1: cr,
            y1: top + g.height + pr.v_space,
            kind: match pr.wrap {
                FrameWrap::Around => FloatKind::Square,
                FrameWrap::Clear => FloatKind::TopBottom,
            },
            side: WrapSide::BothSides,
            rect: [cl, top, cr, top + g.height],
            outline: None,
            table: false,
            frame: true,
            no_overlap: false,
        }
    }

    /// Draws the frame that starts at item `p` (with the paragraphs after it that share its
    /// properties) where its position says and makes the text of the following paragraphs wrap
    /// around it. The frame leaves the flow like a floating table.
    fn place_frame_run(&mut self, items: &'a [FlowItem], p: usize) {
        let q = frame_end(items, p);
        let Some(g) = frame_geom(items, p, q) else {
            return;
        };
        let anchor = items[q..]
            .iter()
            .find(|i| !self.leaves_flow(i))
            .and_then(FlowItem::para);
        let anchor_top = match self.page.anchor_top {
            Some((page, top)) if page == self.page.number => top,
            _ => self.page.y + anchor.map_or(0.0, |a| first_gap(&self.page, a) + a.border_top()),
        };
        let (left, top) = self.frame_spot(&g, anchor_top);
        let before = self.page.lines.len();
        for fl in &g.lines {
            let item = &items[fl.item];
            let Some(para) = item.para() else { continue };
            let lb = &para.lines[fl.line];
            let base = item.base();
            let y = top + fl.dy;
            let mut line = build_line(para, lb, left, y, base);
            line.story = item.story;
            line.border = para.line_border(lb, left, fl.join_prev, fl.join_next);
            let images = line_images(para, lb, left, y, base);
            self.page.lines.push(line);
            self.page.images.extend(images);
        }
        // The frame's lines do not make the column non-empty: the anchor paragraph is still
        // its first.
        self.page.col_mark.0 += self.page.lines.len() - before;
        self.page.floats.push(Self::frame_float(&g, left, top));
    }

    fn break_in_table(
        &mut self,
        table: &TableLayout,
        r: usize,
        y: &mut f64,
        active: &mut [ActiveSpan],
    ) {
        let section = self.page.cur;
        self.next_column_or_page(false, section);
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

/// One line of a frame: its item, its index in the paragraph and its top below the clip
/// rectangle's top.
struct FrameLine {
    item: usize,
    line: usize,
    dy: f64,
    join_prev: bool,
    join_next: bool,
}

/// The size of a frame and where its lines stand inside it.
struct FrameGeom {
    spec: FrameSpec,
    lines: Vec<FrameLine>,
    /// How far the clip rectangle reaches left and right of the text box: `hSpace`, and for a
    /// side with a border its distance, its width and 1.5 pt.
    gl: f64,
    gr: f64,
    /// Height of the clip rectangle.
    height: f64,
}

/// The item after the last paragraph of the frame that starts at item `p`: consecutive
/// paragraphs with the same properties share a frame, a drop cap stands alone.
fn frame_end(items: &[FlowItem], p: usize) -> usize {
    let Some(spec) = items[p].unit.frame else {
        return p + 1;
    };
    let mut q = p + 1;
    if spec.pr.drop_cap.is_none() {
        while items.get(q).is_some_and(|i| {
            i.section == items[p].section && i.unit.frame.is_some_and(|f| f.pr == spec.pr)
        }) {
            q += 1;
        }
    }
    q
}

/// Whether item `p` is a frame paragraph that belongs to the frame of the item before it.
fn frame_continues(items: &[FlowItem], p: usize) -> bool {
    p > 0 && items[p].unit.frame.is_some() && frame_end(items, p - 1) > p
}

/// Stacks the paragraphs `p..q` of a frame: spacing, borders and line tops relative to the
/// clip rectangle, whose top is where the first border begins.
fn frame_geom(items: &[FlowItem], p: usize, q: usize) -> Option<FrameGeom> {
    let spec = items[p].unit.frame?;
    let mut lines = Vec::new();
    let mut y = 0.0;
    let mut after = 0.0_f64;
    let mut prev: Option<&ParaLayout> = None;
    let (mut gl, mut gr) = (spec.pr.h_space, spec.pr.h_space);
    for i in p..q {
        let Some(para) = items[i].para() else {
            continue;
        };
        let joined_prev = prev.is_some_and(|q| q.joins(para));
        let joined_next = i + 1 < q
            && items
                .get(i + 1)
                .and_then(FlowItem::para)
                .is_some_and(|n| para.joins(n));
        if prev.is_none() {
            gl += spec.pad[0];
            gr += spec.pad[2];
            y += f64::from(para.props.space_before_pt);
        } else {
            y += after.max(f64::from(para.props.space_before_pt));
        }
        y += if joined_prev {
            para.between_extent()
        } else if prev.is_none() {
            spec.pad[1]
        } else {
            para.border_top()
        };
        for (li, lb) in para.lines.iter().enumerate() {
            y += lb.lead;
            lines.push(FrameLine {
                item: i,
                line: li,
                dy: y,
                join_prev: joined_prev,
                join_next: joined_next,
            });
            y += lb.height;
        }
        if !joined_next {
            y += if i + 1 == q {
                spec.pad[3]
            } else {
                para.border_bottom()
            };
        }
        after = f64::from(para.props.space_after_pt);
        prev = Some(para);
    }
    let height = match spec.pr.height {
        Some(h) if spec.pr.exact => h,
        Some(h) => h.max(y),
        None => y,
    };
    Some(FrameGeom {
        spec,
        lines,
        gl,
        gr,
        height,
    })
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
            && self.anchor_top == other.anchor_top
            && self.force_break == other.force_break
            && self.force_column == other.force_column
            && self.section_index == other.section_index
            && self.cur == other.cur
            && self.col == other.col
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

/// A break the last paragraph ended with, which the next block must honour.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Force {
    page: bool,
    column: bool,
}

fn checkpoint(pg: &Paginator<'_>, force: Force) -> Checkpoint {
    Checkpoint {
        pages_done: pg.done.len(),
        lines: pg.page.lines.len(),
        cells: pg.page.cells.len(),
        images: pg.page.images.len(),
        floats: pg.page.floats.len(),
        floats_sig: floats_sig(&pg.page.floats),
        y: pg.page.y,
        hard_start: pg.page.hard_start,
        force_break: force.page,
        force_column: force.column,
        section_index: pg.page.section_index,
        cur: pg.page.cur,
        col: pg.page.col,
        notes: pg.page.slots.len(),
        anchor_top: pg.page.anchor_top,
    }
}

impl<'a> Paginator<'a> {
    /// Remembers the page as it is before the first content of a column region is placed.
    fn snapshot(&mut self, items: &'a [FlowItem], item: usize, from_atom: usize) {
        if self.trial || self.region.is_some() {
            return;
        }
        if column_count(self.env, self.page.cur) > 1 && self.page.col == 0 {
            self.seed_notes(items, item, from_atom);
            self.region = Some(Region {
                item,
                from_atom,
                page: self.page.clone(),
            });
        }
    }

    /// The footnote area of a page is as tall as the notes of every reference on the page, those
    /// in later columns included, and it limits the body of the first column too (measured on
    /// `layout-cols-fn-unequal`: the notes of references in the second column keep the first
    /// column's text above the separator). The notes the region's page will hold are found by
    /// placing it once and are on the page from the start.
    fn seed_notes(&mut self, items: &'a [FlowItem], item: usize, from_atom: usize) {
        let env = self.env;
        if !env.notes.has_notes(self.page.section_index) {
            return;
        }
        for _ in 0..3 {
            let mut tp = Paginator::start(env, self.page.clone());
            tp.trial = true;
            let mut force = Force::default();
            for q in item..items.len() {
                if q > item {
                    tp.begin_item(items, q, &mut force);
                }
                tp.place_item(items, q, if q == item { from_atom } else { 0 }, &mut force);
                if tp.overflow {
                    break;
                }
            }
            if tp.page.slots == self.page.slots {
                break;
            }
            self.page.slots = tp.page.slots;
            self.page.notes_h = env.notes.area(self.page.section_index, &self.page.slots);
        }
    }

    /// Starts item `p` in its section: a new section begins where its type says, and a page or
    /// column break the previous paragraph ended with, or `pageBreakBefore`, is honoured.
    fn begin_item(&mut self, items: &'a [FlowItem], p: usize, force: &mut Force) {
        let item = &items[p];
        let section = item.section();
        let wants_break = force.page || item.para().is_some_and(|q| q.flow.page_break_before);
        let changed = section != self.page.cur;
        if changed {
            self.start_section(items, p, section);
        }
        if wants_break && self.page.page_has_content() {
            self.new_page(false, section);
        } else if force.column && !changed {
            self.next_column_or_page(false, section);
        }
        *force = Force::default();
    }

    /// Starts `section`, the section of item `p`, which differs from the current one.
    fn start_section(&mut self, items: &'a [FlowItem], p: usize, section: usize) {
        let start = self
            .env
            .cols
            .get(section)
            .map_or(SectionStart::NextPage, |c| c.start);
        let new = self.env.sections.get(section).copied().unwrap_or_default();
        let old = self.page.geom;
        let same_size = (old.page_width - new.page_width).abs() < 0.01
            && (old.page_height - new.page_height).abs() < 0.01;
        match start {
            SectionStart::Continuous if same_size => self.continue_section(items, p, section),
            SectionStart::NextColumn
                if same_size && self.page.col + 1 < column_count(self.env, self.page.cur) =>
            {
                self.next_column();
                self.page.cur = section;
                self.page.geom = new;
                let col = self.page.col;
                self.set_column(col);
                self.region = None;
            }
            _ => self.new_page(true, section),
        }
    }

    /// A continuous section starts on the page below the previous one, whose columns are
    /// balanced first.
    fn continue_section(&mut self, items: &'a [FlowItem], p: usize, section: usize) {
        if column_count(self.env, self.page.cur) > 1 {
            self.balance(items, p);
        }
        self.region = None;
        let props = self.env.sections.get(section).copied().unwrap_or_default();
        let pg = &mut self.page;
        pg.cur = section;
        pg.geom = props;
        pg.col_top = pg.y + pg.pending_after;
        pg.col_mark = (pg.lines.len(), pg.cells.len());
        pg.inherited = pg.page_has_content();
        pg.col_ends.clear();
        pg.anchor_top = None;
        self.set_column(0);
    }

    /// Word balances the columns of a section that a continuous section break ends: the
    /// content of the section's last page is spread over its columns at the smallest height
    /// that holds it. Found by placing that content again with a lower bottom.
    fn balance(&mut self, items: &'a [FlowItem], upto: usize) {
        let Some(r) = self.region.take() else { return };
        if self.trial {
            return;
        }
        let real_bottom = r.page.frame_bottom;
        let room = real_bottom - r.page.col_top;
        let env = self.env;
        let run = |h: f64| -> Option<PageBuild> {
            let mut page = r.page.clone();
            page.frame_bottom = page.col_top + h;
            let mut tp = Paginator::start(env, page);
            tp.trial = true;
            let mut force = Force::default();
            for q in r.item..upto {
                if q > r.item {
                    tp.begin_item(items, q, &mut force);
                }
                tp.place_item(
                    items,
                    q,
                    if q == r.item { r.from_atom } else { 0 },
                    &mut force,
                );
                if tp.overflow {
                    return None;
                }
            }
            Some(tp.page)
        };
        let Some(full) = run(room) else { return };
        // Word aims at an equal share of the content height per column and lets a column take
        // a line that starts before that share is used up (layout-cols-balance: 10 lines and 2
        // paragraph gaps over two columns give 6 lines in the first, not 5). When the rest
        // does not fit in the remaining columns the height grows until it does.
        let top = r.page.col_top;
        // The space after the last paragraph of a full column is not in its height, but it
        // separates the paragraphs in one column.
        let used: f64 = full.col_ends.iter().map(|e| e.0 - top + e.1).sum::<f64>() + (full.y - top);
        let n = column_count(env, full.cur) as f64;
        let added = &full.lines[r.page.lines.len().min(full.lines.len())..];
        let lax = if added.is_empty() {
            0.0
        } else {
            added.iter().map(|l| l.height).sum::<f64>() / added.len() as f64 - 0.01
        };
        let start = (used / n + lax).min(room);
        let mut best = full;
        if let Some(page) = run(start) {
            best = page;
        } else {
            let (mut lo, mut hi) = (start, room);
            for _ in 0..18 {
                let mid = (lo + hi) / 2.0;
                match run(mid) {
                    Some(page) => {
                        hi = mid;
                        best = page;
                    }
                    None => lo = mid,
                }
            }
        }
        best.frame_bottom = real_bottom;
        // The next section starts below the tallest column, including the space after its
        // last paragraph.
        best.col_ends.push((best.y, best.pending_after));
        let (y, after) = best
            .col_ends
            .iter()
            .copied()
            .max_by(|a, b| (a.0 + a.1).total_cmp(&(b.0 + b.1)))
            .unwrap_or((best.y, best.pending_after));
        best.y = y;
        best.pending_after = after;
        self.page = best;
    }

    /// Places block `p` of the main story, from atom `from_atom0` of its paragraph.
    fn place_item(
        &mut self,
        items: &'a [FlowItem],
        p: usize,
        from_atom0: usize,
        force: &mut Force,
    ) {
        let env = self.env;
        let item = &items[p];
        let section = item.section();
        let base = item.base();
        let (para, table) = (item.para(), item.table());

        if item.unit.frame.is_some() {
            // A frame leaves the flow; the first paragraph of a run places all of it.
            if !frame_continues(items, p) {
                self.snapshot(items, p, 0);
                self.place_frame_run(items, p);
            }
            return;
        }
        if let Some(table) = table {
            if let Some(pos) = self.floating(table) {
                self.cur = Some((item.unit.as_ref(), base));
                self.snapshot(items, p, 0);
                self.place_float_table(items, p, table, &pos);
                return;
            }
            let gap = if self.page.has_content() {
                self.page.pending_after
            } else {
                0.0
            };
            self.cur = Some((item.unit.as_ref(), base));
            self.snapshot(items, p, 0);
            self.page.anchor_top = None;
            self.place_table(table, gap);
            self.page.pending_after = 0.0;
            self.page.prev_style = None;
            self.page.prev_contextual = false;
            return;
        }
        let Some(para) = para else { return };

        let has_anchor = para.objects.iter().any(|o| !o.inline());
        let mut from_atom = from_atom0;
        let n_atoms = para.atoms.len();
        while from_atom < n_atoms {
            if self.overflow {
                return;
            }
            self.snapshot(items, p, from_atom);
            let first_segment = from_atom == 0;
            // Spacing above the first line of this paragraph.
            let gap = |pg: &PageBuild| -> f64 {
                if first_segment {
                    first_gap(pg, para)
                } else {
                    0.0
                }
            };
            let at_top = !self.page.has_content();
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
            let y0 = self.page.y + gap(&self.page) + border_above;
            // Floating pictures change the lines: break them again around the pictures.
            // A continuation that starts inside a line of the plain layout (the lines above broke
            // around a picture) must be broken again too. So must a paragraph that was laid
            // out at another width than the column it lands in (unequal columns).
            let mid_line = from_atom > 0 && !para.lines.iter().any(|l| l.atoms.start == from_atom);
            let narrow = (para.fill.text_width - self.page.col_w).abs() > 1e-6;
            let around = has_anchor
                || mid_line
                || narrow
                || self.page.floats.iter().any(|f| f.y1 > y0 - EPS);
            let frame = Frame {
                section: self.page.geom,
                col: self.page.col_span(),
            };
            let anchor_top = if first_segment {
                self.page
                    .anchor_top
                    .filter(|(page, _)| *page == self.page.number)
                    .map(|(_, top)| top)
            } else {
                None
            };
            let mut dynamic = around.then(|| {
                dynamic_lines(
                    para,
                    from_atom,
                    y0,
                    anchor_top,
                    &self.page.floats,
                    &frame,
                    base,
                )
            });
            // Floats of the next paragraph reach back over the lines of this one: Word places
            // them from the next paragraph's top as laid out without them, then pushes the
            // lines above (word-float-placement.md, rule 3).
            let mut next_top = None;
            // Floating tables in front of the next paragraph, and that paragraph.
            let mut q = p + 1;
            while items
                .get(q)
                .is_some_and(|i| self.leaves_flow(i) && i.section() == section)
            {
                q += 1;
            }
            let tables = p + 1..q;
            if let Some(next) = items.get(q).and_then(FlowItem::para)
                && (next.objects.iter().any(|o| !o.inline()) || !tables.is_empty())
                && !next.flow.page_break_before
                && items[q].section() == section
            {
                let lines: &[LineBox] = match &dynamic {
                    Some(d) => &d.lines,
                    None => {
                        let from = para.lines.partition_point(|l| l.atoms.start < from_atom);
                        &para.lines[from..]
                    }
                };
                let used: f64 = lines.iter().map(|l| l.lead + l.height).sum();
                let top = y0
                    + used
                    + f64::from(para.props.space_after_pt)
                        .max(f64::from(next.props.space_before_pt));
                let mut ahead = ahead_floats(next, &frame, top, y0 + used);
                let mut placed: Vec<Float> = self.page.floats.clone();
                for t in tables {
                    if let Some(tl) = items[t].table()
                        && let Some(pos) = self.floating(tl)
                    {
                        let (left, ttop) = self.table_spot(tl, &pos, top, &placed);
                        let f = Self::table_float(tl, &pos, left, ttop, tl.height());
                        placed.push(f.clone());
                        if f.y0 < y0 + used - 1e-6 {
                            ahead.push(f);
                        }
                    } else if !frame_continues(items, t)
                        && let Some(g) = frame_geom(items, t, frame_end(items, t))
                    {
                        let (left, ftop) = self.frame_spot(&g, top);
                        let f = Self::frame_float(&g, left, ftop);
                        placed.push(f.clone());
                        if f.y0 < y0 + used - 1e-6 {
                            ahead.push(f);
                        }
                    }
                }
                if !ahead.is_empty() {
                    let mut all = self.page.floats.clone();
                    all.extend(ahead);
                    let alt = dynamic_lines(para, from_atom, y0, anchor_top, &all, &frame, base);
                    let fit = |ls: &[LineBox]| {
                        decide(
                            para,
                            ls,
                            first_segment,
                            y0,
                            self.page.bottom(),
                            at_top,
                            None,
                        )
                        .take
                            == ls.len()
                    };
                    if fit(lines) && fit(&alt.lines) {
                        next_top = Some(top);
                        dynamic = Some(alt);
                    }
                }
            }
            let lines: &[LineBox] = match &dynamic {
                Some(d) => &d.lines,
                None => {
                    let from = para.lines.partition_point(|l| l.atoms.start < from_atom);
                    &para.lines[from..]
                }
            };
            let notes_at = NotesAt {
                env,
                section: self.page.section_index,
                frame_bottom: self.page.frame_bottom,
                slots: &self.page.slots,
                sep: self.page.sep_h,
            };
            let mut place = decide(
                para,
                lines,
                first_segment,
                y0,
                self.page.bottom(),
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
                    self.page.bottom(),
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
            if place.take > 0
                && !at_top
                && first_segment
                && dynamic.as_ref().is_some_and(|d| {
                    let end = lines[place.take - 1].atoms.end;
                    d.anchors.iter().any(|a| a.overflows && a.atom < end)
                })
            {
                // A wrapping float that cannot stand below its anchor on this page takes the
                // whole paragraph to the next one.
                place.take = 0;
            }
            if place.take == 0 {
                // Nothing of this paragraph goes here: move to the next column or a fresh page
                // and retry.
                self.next_column_or_page(false, section);
                continue;
            }
            let split = place.split;
            let mut y = y0;
            let mut placed = 0usize;
            let mut broke = false;
            let mut column_break = false;
            let origin = self.page.origin_x();
            for lb in &lines[..place.take] {
                y += lb.lead;
                let mut line = build_line(para, lb, origin, y, base);
                line.story = item.story;
                line.border = para.line_border(lb, origin, joined_prev, joined_next);
                let images = line_images(para, lb, origin, y, base);
                y += lb.height;
                self.page.lines.push(line);
                self.page.images.extend(images);
                for (a, id) in &para.foot_refs {
                    if lb.atoms.contains(a) {
                        match split {
                            Some((sid, m)) if sid == *id && placed + 1 == place.take => {
                                self.page.add_slot(
                                    NoteSlot {
                                        id: sid,
                                        from: 0,
                                        to: m,
                                    },
                                    env,
                                );
                            }
                            _ => self.page.add_note(*id, env),
                        }
                    }
                }
                placed += 1;
                if lb.page_break_after {
                    broke = true;
                    column_break = lb.column_break_after && column_count(env, self.page.cur) > 1;
                    break;
                }
            }
            let end_atom = lines[placed - 1].atoms.end;
            if let Some(d) = &dynamic {
                for a in d.anchors.iter().filter(|a| a.atom < end_atom) {
                    self.page.images.push(a.image.clone());
                    self.page.floats.extend(a.float.clone());
                }
            }
            if end_atom >= n_atoms && !joined_next {
                y += para.border_bottom();
            }
            self.page.y = y;
            self.page.anchor_top = next_top
                .filter(|_| end_atom >= n_atoms)
                .map(|top| (self.page.number, top));
            from_atom = end_atom;
            if from_atom < n_atoms {
                // The paragraph goes on in the next column: no space after it here.
                self.page.pending_after = 0.0;
                if broke && !column_break {
                    self.new_page(true, section);
                } else {
                    self.next_column_or_page(false, section);
                }
            } else if broke {
                if column_break {
                    force.column = true;
                } else {
                    force.page = true;
                }
            }
        }
        if !para.section_mark {
            self.page.pending_after = f64::from(para.props.space_after_pt);
            self.page.prev_style = Some(para.style_id.clone());
            self.page.prev_contextual = para.flow.contextual_spacing;
        }
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
    let (start, mut checkpoints, mut pg, mut force) = match resume {
        Some(r) => {
            let carry = r
                .item
                .checked_sub(1)
                .map(|i| items[i].carry())
                .unwrap_or_default();
            let mut slots = r.page.note_slots;
            slots.truncate(r.checkpoint.notes);
            let section_index = r.checkpoint.section_index;
            let (col_x, col_w) = env.cols.get(section_index).map_or_else(
                || (0.0, r.page.section.text_width()),
                |c| c.columns.first().map_or((0.0, 0.0), |g| (g.x, g.width)),
            );
            let page = PageBuild {
                number: r.page.number,
                section: r.page.section,
                section_index,
                cur: section_index,
                geom: r.page.section,
                col: 0,
                col_x,
                col_w,
                col_top: r.page.body_top,
                col_mark: (0, 0),
                inherited: false,
                col_ends: Vec::new(),
                lines: r.page.lines,
                cells: r.page.cells,
                images: r.page.images,
                floats: r.page.floats,
                first_in_section: r.page.first_in_section,
                top: r.page.body_top,
                frame_bottom: r.page.body_bottom,
                notes_h: env.notes.area(section_index, &slots),
                sep_h: env.notes.sep_height(section_index),
                slots,
                y: r.checkpoint.y,
                pending_after: carry.after,
                prev_style: carry.style,
                prev_contextual: carry.contextual,
                hard_start: r.checkpoint.hard_start,
                anchor_top: r.checkpoint.anchor_top,
            };
            let mut pg = Paginator::start(env, page);
            pg.done = r.prior;
            let force = Force {
                page: r.checkpoint.force_break,
                column: r.checkpoint.force_column,
            };
            (r.item, r.checkpoints, pg, force)
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
            let pg = Paginator::start(env, page);
            (0, Vec::with_capacity(items.len() + 1), pg, Force::default())
        }
    };

    for p in start..items.len() {
        if crate::cancel::cancelled() {
            break;
        }
        let ck = checkpoint(&pg, force);
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

        pg.begin_item(items, p, &mut force);
        pg.place_item(items, p, 0, &mut force);
    }
    checkpoints.push(checkpoint(&pg, force));
    let last = pg.page.finish(env);
    pg.done.push(last);
    Paginated {
        pages: pg.done,
        open: None,
        checkpoints,
        converged: None,
    }
}
