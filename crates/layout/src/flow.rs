//! Page flow: places laid-out paragraphs onto pages with Word's rules for spacing, widow and
//! orphan control, keep-with-next, keep-lines, page breaks and sections.

use crate::para::{AtomClass, Context, LineBox, ParaLayout};
use crate::props::SectionProps;
use crate::{GlyphRun, Line, Page, PlacedGlyph};

const EPS: f64 = 1e-6;

struct PageBuild {
    number: usize,
    section: SectionProps,
    section_index: usize,
    lines: Vec<Line>,
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
        !self.lines.is_empty()
    }

    fn finish(self) -> Page {
        Page {
            number: self.number,
            section: self.section,
            lines: self.lines,
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
        if para.props.keep_lines && first_segment && k > 0 {
            k = 0;
        } else if para.extra.widow_control && n >= 2 {
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

/// Whether paragraph `p` (and, when it is kept with the next, its followers) can start on a
/// page whose cursor is at `y`.
fn can_start(
    paras: &[(ParaLayout, usize)],
    p: usize,
    y: f64,
    prev_after: f64,
    bottom: f64,
    depth: usize,
) -> bool {
    let (para, _) = &paras[p];
    let y0 = y + prev_after + para.props.space_before_pt as f64;
    let place = decide(para, 0, y0, bottom, false);
    if place.take == 0 {
        return false;
    }
    if para.props.keep_next && place.take == para.lines.len() && p + 1 < paras.len() && depth < 8 {
        let used: f64 = para.lines.iter().map(|l| l.height).sum();
        return can_start(
            paras,
            p + 1,
            y0 + used,
            para.props.space_after_pt as f64,
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

fn build_line(
    ctx: &Context<'_>,
    para: &ParaLayout,
    lb: &LineBox,
    section: &SectionProps,
    top: f64,
) -> Line {
    let left_origin = section.margin_left + section.gutter;
    let mut runs: Vec<GlyphRun> = Vec::new();
    let mut cur_style: Option<usize> = None;
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
                    glyph_id: g.glyph_id,
                    x: x + g.x_offset,
                    y_offset: g.y_offset,
                    advance: g.adv,
                });
                x += g.adv;
            }
        }
    }
    let _ = ctx;
    Line {
        paragraph: para.index,
        start: lb.start,
        end: lb.end,
        top,
        baseline: snap_baseline(top + lb.baseline_off),
        height: lb.height,
        left: left_origin + lb.left,
        last_x: left_origin + lb.last_x,
        runs,
    }
}

pub(crate) fn paginate(
    ctx: &Context<'_>,
    paras: &[(ParaLayout, usize)],
    sections: &[SectionProps],
) -> Vec<Page> {
    let mut done: Vec<Page> = Vec::new();
    let first_section = paras.first().map_or(0, |p| p.1);
    let mut page = PageBuild::new(
        1,
        first_section,
        sections.get(first_section).copied().unwrap_or_default(),
        true,
    );

    macro_rules! new_page {
        ($hard:expr, $section:expr) => {{
            let number = page.number + 1;
            let sec = $section;
            let next = PageBuild::new(
                number,
                sec,
                sections.get(sec).copied().unwrap_or_default(),
                $hard,
            );
            done.push(std::mem::replace(&mut page, next).finish());
        }};
    }

    let mut force_break = false;
    for (p, (para, section)) in paras.iter().enumerate() {
        let wants_break = force_break || para.extra.page_break_before;
        if *section != page.section_index || (wants_break && page.has_content()) {
            // Space before is dropped at the top of every page after the first, whether the page
            // began naturally, after a break character or at `pageBreakBefore`; a new section
            // keeps it (layout-pagination).
            new_page!(*section != page.section_index, *section);
        }
        force_break = false;

        let contextual_same = |pg: &PageBuild| {
            pg.prev_style
                .as_ref()
                .is_some_and(|s| *s == para.extra.style_id)
        };
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
                    if para.extra.contextual_spacing {
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
            let y0 = page.y + gap(&page);
            let at_top = !page.has_content();
            let mut place = decide(para, from, y0, page.bottom(), at_top);
            if first_segment
                && place.take == para.lines.len()
                && para.props.keep_next
                && !at_top
                && p + 1 < paras.len()
            {
                let used: f64 = para.lines.iter().map(|l| l.height).sum();
                let ok = can_start(
                    paras,
                    p + 1,
                    y0 + used,
                    f64::from(para.props.space_after_pt),
                    page.bottom(),
                    0,
                );
                if !ok {
                    place.take = 0;
                }
            }
            if place.take == 0 {
                // Nothing of this paragraph goes here: move to a fresh page and retry.
                new_page!(false, *section);
                continue;
            }
            let mut y = y0;
            let mut placed = 0usize;
            let mut broke = false;
            for lb in &para.lines[from..from + place.take] {
                let line = build_line(ctx, para, lb, &page.section, y);
                y += lb.height;
                page.lines.push(line);
                placed += 1;
                if lb.page_break_after {
                    broke = true;
                    break;
                }
            }
            page.y = y;
            from += placed;
            if from < n {
                new_page!(broke, *section);
            } else if broke {
                force_break = true;
            }
        }
        page.pending_after = f64::from(para.props.space_after_pt);
        page.prev_style = Some(para.extra.style_id.clone());
        page.prev_contextual = para.extra.contextual_spacing;
    }
    done.push(page.finish());
    done
}
