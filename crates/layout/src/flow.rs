//! Page flow: places laid-out paragraphs and tables onto pages with Word's rules for spacing,
//! widow and orphan control, keep-with-next, keep-lines, page breaks, sections, table row
//! splitting and repeated header rows.

use std::sync::Arc;

use crate::build::UnitLayout;
use crate::para::{AtomClass, LineBox, ParaLayout};
use crate::props::SectionProps;
use crate::table::{CellLayout, FlowLine, RowLayout, TableLayout};
use crate::{CellBox, GlyphRun, Line, Page, PlacedGlyph};

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
struct Base {
    para: usize,
    story: usize,
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
    pub y: f64,
    pub hard_start: bool,
    pub force_break: bool,
    pub section_index: usize,
}

struct PageBuild {
    number: usize,
    section: SectionProps,
    section_index: usize,
    lines: Vec<Line>,
    cells: Vec<CellBox>,
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
    fn new(number: usize, section_index: usize, section: SectionProps, hard_start: bool) -> Self {
        Self {
            number,
            section,
            section_index,
            lines: Vec::new(),
            cells: Vec::new(),
            y: section.margin_top,
            pending_after: 0.0,
            prev_style: None,
            prev_contextual: false,
            hard_start,
        }
    }

    fn bottom(&self) -> f64 {
        self.section.page_height - self.section.margin_bottom
    }

    fn has_content(&self) -> bool {
        !self.lines.is_empty() || !self.cells.is_empty()
    }

    /// Left edge of the text area.
    fn origin_x(&self) -> f64 {
        self.section.margin_left + self.section.gutter
    }

    fn finish(self) -> Page {
        Page {
            number: self.number,
            section: self.section,
            lines: self.lines,
            cells: self.cells.into_iter().map(snap_cell).collect(),
        }
    }
}

/// How many of `heights` fit starting at `y0` above `bottom`.
fn lines_fitting(heights: impl Iterator<Item = f64>, y0: f64, bottom: f64) -> usize {
    let mut y = y0;
    let mut k = 0;
    for h in heights {
        if y + h > bottom + EPS {
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
}

/// Decides how many of `lines[from..]` to put on a page where `y0` is where the first would
/// go. `at_top` means nothing is on the page yet, so no rule may push everything away.
fn decide(para: &ParaLayout, from: usize, y0: f64, bottom: f64, at_top: bool) -> Placement {
    let lines = &para.lines[from..];
    let n = lines.len();
    let raw = lines_fitting(lines.iter().map(|l| l.height), y0, bottom);
    let mut k = raw;
    if k < n {
        let first_segment = from == 0;
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
    Placement { take: k }
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
    let place = decide(para, 0, y0, bottom, false);
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

/// A cell rectangle as Word draws it: the top and bottom edges lie on the same 0.24 pt grid as
/// baselines, each rounded to the nearest line (measured over 120-row probe tables: the row
/// boundaries are `round(exact_y / 0.24) * 0.24`, so rows alternate 13.44 and 13.20 pt for a
/// 13.43 pt line). Left and right edges are not snapped (columns are exact twips).
fn snap_cell(c: CellBox) -> CellBox {
    let top = (c.y / BASELINE_GRID).round() * BASELINE_GRID;
    let bottom = ((c.y + c.height) / BASELINE_GRID).round() * BASELINE_GRID;
    CellBox {
        y: top,
        height: bottom - top,
        ..c
    }
}

/// Builds the placed line `lb` of `para`, its text area starting at `origin_x` on the page.
fn build_line(para: &ParaLayout, lb: &LineBox, origin_x: f64, top: f64, base: Base) -> Line {
    let left_origin = origin_x;
    let mut runs: Vec<GlyphRun> = Vec::new();
    let mut cur_style: Option<usize> = None;
    if lb.atoms.start == 0
        && let Some(label) = &para.label
    {
        let st = &para.styles[label.style];
        let mut x = left_origin + label.x;
        let glyphs = label
            .glyphs
            .iter()
            .map(|g| {
                let placed = PlacedGlyph {
                    face: g.face,
                    glyph_id: g.glyph_id,
                    x: x + g.x_offset,
                    y_offset: g.y_offset,
                    advance: g.adv,
                };
                x += g.adv;
                placed
            })
            .collect();
        runs.push(GlyphRun {
            face: st.face,
            size: st.size,
            bold: st.bold,
            italic: st.italic,
            underline: st.underline,
            strike: st.strike,
            color: st.color,
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
                glyphs: Vec::new(),
            });
            cur_style = Some(atom.style);
        }
        let mut x = left_origin + lb.xs[k];
        if let Some(run) = runs.last_mut() {
            for g in &para.glyphs[atom.glyphs.clone()] {
                run.glyphs.push(PlacedGlyph {
                    face: g.face,
                    glyph_id: g.glyph_id,
                    x: x + g.x_offset,
                    y_offset: g.y_offset,
                    advance: g.adv,
                });
                x += g.adv;
            }
        }
    }
    Line {
        paragraph: base.para + para.index,
        start: base.story + lb.start,
        end: base.story + lb.end,
        top,
        baseline: snap_baseline(top + lb.baseline_off),
        height: lb.height,
        left: left_origin + lb.left,
        last_x: left_origin + lb.last_x,
        runs,
    }
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

struct Paginator<'a> {
    sections: &'a [SectionProps],
    done: Vec<Page>,
    page: PageBuild,
    /// The table unit being placed and its offsets.
    cur: Option<(&'a UnitLayout, Base)>,
}

impl Paginator<'_> {
    fn new_page(&mut self, hard: bool, section: usize) {
        let number = self.page.number + 1;
        let next = PageBuild::new(
            number,
            section,
            self.sections.get(section).copied().unwrap_or_default(),
            hard,
        );
        self.done
            .push(std::mem::replace(&mut self.page, next).finish());
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
            self.page.cells.push(CellBox {
                x: origin + bx.dx,
                y: y + (abs - row.top),
                width: bx.w,
                height: bx.h,
            });
        }
    }

    fn push_line(&mut self, ln: &FlowLine, origin: f64, top: f64) {
        let Some((unit, base)) = self.cur else { return };
        let para = &unit.arena[ln.para];
        let line = build_line(para, &para.lines[ln.line], origin + ln.dx, top, base);
        self.page.lines.push(line);
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
                        self.page.cells.push(CellBox {
                            x: origin + cell.x + cell.inset_l,
                            y,
                            width: cell.w - cell.inset_l - cell.inset_r,
                            height: row.height - row.below,
                        });
                        s.box_idx = Some(self.page.cells.len() - 1);
                    }
                }
            }
        }
        for (ci, cell) in row.cells.iter().enumerate() {
            if cell.row_span == 0 {
                continue;
            }
            self.page.cells.push(CellBox {
                x: origin + cell.x + cell.inset_l,
                y: y + row.above,
                width: cell.w - cell.inset_l - cell.inset_r,
                height: row.height - row.above - row.below,
            });
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

    /// Whether row `r` may be split across pages: free of merges, not exact, not `cantSplit`.
    fn splittable(table: &TableLayout, r: usize, active: &[ActiveSpan]) -> bool {
        let row = &table.rows[r];
        !row.cant_split
            && !row.exact
            && row.cells.iter().all(|c| c.row_span == 1)
            && !active.iter().any(|s| s.start_row < r && r <= s.end_row)
    }

    /// Takes as many lines of every cell of row `r` as fit in `avail`, starting at `resume`.
    /// Returns the new cursors, the fragment height and whether the row is finished.
    fn split_fragment(
        table: &TableLayout,
        r: usize,
        resume: &[usize],
        avail: f64,
    ) -> (Vec<usize>, f64, bool) {
        let row = &table.rows[r];
        let mut next = resume.to_vec();
        let mut height = 0.0f64;
        let mut finished = true;
        for (ci, cell) in row.cells.iter().enumerate() {
            let start = resume[ci];
            if start >= cell.lines.len() {
                continue;
            }
            let base = if start == 0 {
                0.0
            } else {
                cell.lines[start].dy
            };
            let mut k = start;
            let mut used = 0.0;
            while k < cell.lines.len() {
                let ln = &cell.lines[k];
                let bottom = ln.dy + ln.height - base;
                if cell.pad.top + bottom + cell.pad.bottom > avail + EPS {
                    break;
                }
                used = bottom;
                k += 1;
            }
            if k == cell.lines.len() {
                used = used.max(cell.content_h - base);
                if cell.pad.top + used + cell.pad.bottom > avail + EPS {
                    // The trailing space does not fit: leave the last line for the next page.
                    k = cell.lines.len().saturating_sub(1).max(start);
                    used = cell.lines[k.saturating_sub(1).max(start)].dy - base;
                }
            }
            if k < cell.lines.len() {
                finished = false;
            }
            next[ci] = k;
            if k > start {
                height = height.max(cell.pad.top + used + cell.pad.bottom);
            }
        }
        (next, height, finished)
    }

    fn emit_fragment(
        &mut self,
        table: &TableLayout,
        r: usize,
        y: f64,
        from: &[usize],
        to: &[usize],
        height: f64,
    ) {
        let row = &table.rows[r];
        let origin = self.page.origin_x() + table.x;
        for (ci, cell) in row.cells.iter().enumerate() {
            self.page.cells.push(CellBox {
                x: origin + cell.x + cell.inset_l,
                y,
                width: cell.w - cell.inset_l - cell.inset_r,
                height,
            });
            let base = if from[ci] == 0 {
                0.0
            } else {
                cell.lines.get(from[ci]).map_or(0.0, |l| l.dy)
            };
            for ln in &cell.lines[from[ci].min(cell.lines.len())..to[ci].min(cell.lines.len())] {
                let top = y + cell.pad.top + (ln.dy - base);
                self.push_line(ln, origin + cell.pad.left + cell.x, top);
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
                let (to, h, finished) = Self::split_fragment(table, r, &cursors, avail);
                if h > 0.0 || finished {
                    self.emit_fragment(table, r, y, &cursors, &to, h);
                    y += h;
                    if finished {
                        resume = None;
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
                let zero = vec![0usize; row.cells.len()];
                let (to, h, finished) = Self::split_fragment(table, r, &zero, avail);
                if h > 0.0 || (at_top && finished) {
                    self.emit_fragment(table, r, y, &zero, &to, h);
                    y += h;
                    if finished {
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
                    let to: Vec<usize> = row
                        .cells
                        .iter()
                        .map(|c| usize::from(!c.lines.is_empty()))
                        .collect();
                    let h = row.height.min(avail.max(1.0));
                    self.emit_fragment(table, r, y, &zero, &to, h);
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

fn checkpoint(pg: &Paginator<'_>, force_break: bool) -> Checkpoint {
    Checkpoint {
        pages_done: pg.done.len(),
        lines: pg.page.lines.len(),
        cells: pg.page.cells.len(),
        y: pg.page.y,
        hard_start: pg.page.hard_start,
        force_break,
        section_index: pg.page.section_index,
    }
}

/// Places `items` on pages, from the start or from `resume`. With `previous` it stops as soon
/// as the state equals the earlier run's at the same item (see [`Paginated::converged`]).
pub(crate) fn paginate<'a>(
    items: &'a [FlowItem],
    sections: &'a [SectionProps],
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
            let page = PageBuild {
                number: r.page.number,
                section: r.page.section,
                section_index: r.checkpoint.section_index,
                lines: r.page.lines,
                cells: r.page.cells,
                y: r.checkpoint.y,
                pending_after: carry.after,
                prev_style: carry.style,
                prev_contextual: carry.contextual,
                hard_start: r.checkpoint.hard_start,
            };
            let pg = Paginator {
                sections,
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
                sections.get(first_section).copied().unwrap_or_default(),
                true,
            );
            let pg = Paginator {
                sections,
                done: Vec::new(),
                page,
                cur: None,
            };
            (0, Vec::with_capacity(items.len() + 1), pg, false)
        }
    };

    for p in start..items.len() {
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
                    open: Some(pg.page.finish()),
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
        let mut from = 0usize;
        let n = para.lines.len();
        while from < n {
            let first_segment = from == 0;
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
            let y0 = pg.page.y + gap(&pg.page);
            let at_top = !pg.page.has_content();
            let mut place = decide(para, from, y0, pg.page.bottom(), at_top);
            if first_segment
                && place.take == para.lines.len()
                && para.flow.keep_next
                && !at_top
                && p + 1 < items.len()
            {
                let used: f64 = para.lines.iter().map(|l| l.height).sum();
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
                    let n = para.lines.len();
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
            let mut y = y0;
            let mut placed = 0usize;
            let mut broke = false;
            let origin = pg.page.origin_x();
            for lb in &para.lines[from..from + place.take] {
                let line = build_line(para, lb, origin, y, base);
                y += lb.height;
                pg.page.lines.push(line);
                placed += 1;
                if lb.page_break_after {
                    broke = true;
                    break;
                }
            }
            pg.page.y = y;
            from += placed;
            if from < n {
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
    pg.done.push(pg.page.finish());
    Paginated {
        pages: pg.done,
        open: None,
        checkpoints,
        converged: None,
    }
}
