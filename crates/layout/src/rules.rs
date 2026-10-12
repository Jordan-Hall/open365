//! Paragraph border rules: the filled rectangles Word draws for `w:pBdr` (see
//! `docs/architecture/word-layout.md`, "Paragraph border geometry").
//!
//! Layout positions the lines and leaves a [`LineBorder`] on every bordered line; [`rules_of`]
//! turns the lines of one story on one page into [`Rule`]s. Doing it after pagination keeps the
//! incremental layout simple: a rule is a pure function of the final lines.

use std::sync::Arc;

use open365_docx::{Border, ParagraphBorders};

use crate::{BorderStyle, Line, Rule, RuleSide};

/// Word's drawing grid in points: every rule edge lies on it (rounded to nearest, halves up) and
/// every bar is a whole number of units thick.
const UNIT: f64 = 0.24;
/// The side rules stand this far outside `w:space` (measured: 1.44 pt, six grid units).
const SIDE_GAP: f64 = 1.44;

/// Rounds a length to the drawing grid, nearest, halves up.
pub(crate) fn snap(v: f64) -> f64 {
    (v / UNIT + 1e-9).round() * UNIT
}

/// Whole grid units a bar of `pt` points covers; Word floors and never draws less than one.
fn units(pt: f64) -> f64 {
    (pt / UNIT + 1e-6).floor().max(1.0)
}

/// How one edge is drawn: bars from the outer edge inwards, as (offset, thickness) in grid units.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Edge {
    /// `w:sz` in points.
    pub width: f64,
    /// `w:space` in points.
    pub space: f64,
    pub color: [u8; 3],
    pub style: BorderStyle,
    pub bars: Vec<(f64, f64)>,
    /// Height the layout reserves for the edge, `w:space` excluded.
    pub extent: f64,
}

impl Edge {
    fn new(b: &Border) -> Self {
        let w = b.width;
        let thick = units(w);
        let (thin_small, thin_large) = (units(0.75), units(0.75));
        // A pair or triple: `pattern` lists the bars (true = thick) and `gap` is the gap in
        // units between them.
        let pair = |pattern: &[bool], thick: f64, thin: f64, gap: f64| {
            let mut off = 0.0;
            let mut out = Vec::new();
            for (i, is_thick) in pattern.iter().enumerate() {
                if i > 0 {
                    off += gap;
                }
                let t = if *is_thick { thick } else { thin };
                out.push((off, t));
                off += t;
            }
            out
        };
        let name = b.style.as_str();
        let (thin_medium, gap_medium) = (units(w / 2.0), units(w / 2.0));
        let thick_large = units(w.max(1.5));
        let gap_large = units(w);
        let (bars, style) = match name {
            "double" => (pair(&[true, true], thick, thick, thick), BorderStyle::Solid),
            "triple" => (
                pair(&[true, true, true], thick, thick, thick),
                BorderStyle::Solid,
            ),
            "thinThickSmallGap" => (
                pair(&[true, false], thick, thin_small, thin_small),
                BorderStyle::Solid,
            ),
            "thickThinSmallGap" => (
                pair(&[false, true], thick, thin_small, thin_small),
                BorderStyle::Solid,
            ),
            "thinThickThinSmallGap" => (
                pair(&[false, true, false], thick, thin_small, thin_small),
                BorderStyle::Solid,
            ),
            "thinThickMediumGap" => (
                pair(&[true, false], thick, thin_medium, gap_medium),
                BorderStyle::Solid,
            ),
            "thickThinMediumGap" => (
                pair(&[false, true], thick, thin_medium, gap_medium),
                BorderStyle::Solid,
            ),
            "thinThickThinMediumGap" => (
                pair(&[false, true, false], thick, thin_medium, gap_medium),
                BorderStyle::Solid,
            ),
            "thinThickLargeGap" => (
                pair(&[true, false], thick_large, thin_large, gap_large),
                BorderStyle::Solid,
            ),
            "thickThinLargeGap" => (
                pair(&[false, true], thick_large, thin_large, gap_large),
                BorderStyle::Solid,
            ),
            "thinThickThinLargeGap" => (
                pair(&[false, true, false], thick_large, thin_large, gap_large),
                BorderStyle::Solid,
            ),
            "dotted" => (vec![(0.0, thick)], BorderStyle::Dotted),
            "dashed" | "dashSmallGap" | "dotDash" | "dotDotDash" => {
                (vec![(0.0, thick)], BorderStyle::Dashed)
            }
            _ => (vec![(0.0, thick)], BorderStyle::Solid),
        };
        Self {
            width: w,
            space: b.space,
            color: b.color.unwrap_or([0, 0, 0]),
            style,
            bars,
            extent: crate::para::border_extent(b),
        }
    }

    /// `w:space` as a side rule stands off the text: whole grid units, rounded down (a 4 pt
    /// space is 3.84 pt; `p-pshd`).
    fn side_space(&self) -> f64 {
        (self.space / UNIT + 1e-6).floor() * UNIT
    }

    /// Thickness of all bars with their gaps, in points.
    fn thickness(&self) -> f64 {
        self.bars.last().map_or(0.0, |(o, t)| (o + t) * UNIT)
    }
}

/// The resolved borders of a paragraph.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct BorderSpec {
    pub top: Option<Edge>,
    pub bottom: Option<Edge>,
    pub left: Option<Edge>,
    pub right: Option<Edge>,
    pub between: Option<Edge>,
}

impl BorderSpec {
    /// `None` for a paragraph without a visible top, bottom, left, right or between border.
    pub(crate) fn from_docx(b: &ParagraphBorders) -> Option<Arc<Self>> {
        let edge = |e: &Option<Border>| {
            e.as_ref()
                .filter(|b| b.is_visible() && b.width > 0.0)
                .map(Edge::new)
        };
        let spec = Self {
            top: edge(&b.top),
            bottom: edge(&b.bottom),
            left: edge(&b.left),
            right: edge(&b.right),
            between: edge(&b.between),
        };
        (spec != Self::default()).then(|| Arc::new(spec))
    }
}

/// What a placed line knows about the border box it belongs to.
#[derive(Debug, Clone, PartialEq)]
pub struct LineBorder {
    pub(crate) spec: Arc<BorderSpec>,
    /// Left and right edges of the paragraph's text area (indents applied), page coordinates.
    pub(crate) left: f64,
    pub(crate) right: f64,
    /// The line is the paragraph's first or last line.
    pub(crate) first: bool,
    pub(crate) last: bool,
    /// The paragraph continues the box of the one before it, or the one after it does.
    pub(crate) join_prev: bool,
    pub(crate) join_next: bool,
}

/// Whether the lines `a` then `b` are in one border box (the same paragraph, or consecutive
/// paragraphs the layout joined).
pub(crate) fn same_box(a: &Line, b: &Line) -> bool {
    let (Some(x), Some(y)) = (&a.border, &b.border) else {
        return false;
    };
    if x.spec != y.spec || (x.left - y.left).abs() > 1e-6 || (x.right - y.right).abs() > 1e-6 {
        return false;
    }
    if a.paragraph == b.paragraph {
        return !y.first || !x.last;
    }
    x.last && y.first && x.join_next && y.join_prev
}

struct Out<'a> {
    rules: &'a mut Vec<Rule>,
    story: crate::StoryKind,
    paragraph: usize,
}

impl Out<'_> {
    fn push(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, e: &Edge, side: RuleSide) {
        if x1 - x0 < 1e-6 || y1 - y0 < 1e-6 {
            return;
        }
        self.rules.push(Rule {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
            color: e.color,
            style: e.style,
            side,
            story: self.story,
            paragraph: self.paragraph,
        });
    }
}

/// The rules of the border boxes the `lines` of one story on one page form.
pub(crate) fn rules_of(lines: &[Line], rules: &mut Vec<Rule>) {
    let mut i = 0;
    while i < lines.len() {
        if lines[i].border.is_none() {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < lines.len() && same_box(&lines[j - 1], &lines[j]) {
            j += 1;
        }
        box_rules(&lines[i..j], rules);
        i = j;
    }
}

/// The edges of a border box that a fill inside it stops at (page coordinates, y down).
#[derive(Debug, Clone, Copy)]
pub(crate) struct BoxEdges {
    pub left_inner: f64,
    pub right_inner: f64,
    /// Inner edge of the top rule; `None` when the box has no top rule at its top.
    pub top_inner: Option<f64>,
    /// Inner edge of the bottom rule; `None` when the box has no bottom rule at its bottom.
    pub bottom_inner: Option<f64>,
}

/// The edges of the border box of `lines` (consecutive lines of one border group).
pub(crate) fn box_edges(lines: &[Line]) -> Option<BoxEdges> {
    let (first, last) = (lines.first()?, lines.last()?);
    let (fb, lb) = (first.border.as_ref()?, last.border.as_ref()?);
    let spec = &fb.spec;
    let side_in = |e: &Option<Edge>| e.as_ref().map_or(SIDE_GAP, |e| e.side_space() + SIDE_GAP);
    let top_edge = fb
        .first
        .then_some(())
        .and(spec.top.as_ref())
        .filter(|_| !fb.join_prev);
    let bottom_edge = lb
        .last
        .then_some(())
        .and(spec.bottom.as_ref())
        .filter(|_| !lb.join_next);
    Some(BoxEdges {
        left_inner: snap(fb.left - side_in(&spec.left)),
        right_inner: snap(fb.right + side_in(&spec.right)),
        top_inner: top_edge.map(|e| snap(first.top - e.space - e.extent) + snap_len(e.thickness())),
        bottom_inner: bottom_edge.map(|e| snap(last.top + last.height + e.space)),
    })
}

/// The rules of one box: consecutive lines of one border group on one page.
fn box_rules(lines: &[Line], rules: &mut Vec<Rule>) {
    let (first, last) = (&lines[0], &lines[lines.len() - 1]);
    let (Some(fb), Some(lb)) = (&first.border, &last.border) else {
        return;
    };
    let spec = &fb.spec;
    let mut out = Out {
        rules,
        story: first.story,
        paragraph: first.paragraph,
    };
    // Horizontal extent: the side rules' outer edges, or the bare gap when a side has no rule.
    let side_in = |e: &Option<Edge>| e.as_ref().map_or(SIDE_GAP, |e| e.side_space() + SIDE_GAP);
    let left_inner = snap(fb.left - side_in(&spec.left));
    let right_inner = snap(fb.right + side_in(&spec.right));
    let left_outer = left_inner - spec.left.as_ref().map_or(0.0, |e| snap_len(e.thickness()));
    let right_outer = right_inner + spec.right.as_ref().map_or(0.0, |e| snap_len(e.thickness()));

    let top_edge = fb
        .first
        .then_some(())
        .and(spec.top.as_ref())
        .filter(|_| !fb.join_prev);
    let bottom_edge = lb
        .last
        .then_some(())
        .and(spec.bottom.as_ref())
        .filter(|_| !lb.join_next);

    // Top and bottom of the box.
    let y_text_top = first.top;
    let top_outer = match top_edge {
        Some(e) => snap(y_text_top - e.space - e.extent),
        None => snap(y_text_top),
    };
    let bottom_outer_start = last.top + last.height;
    let bottom_inner = match bottom_edge {
        Some(e) => snap(bottom_outer_start + e.space),
        None => snap(bottom_outer_start),
    };
    let bottom_outer = bottom_inner + bottom_edge.map_or(0.0, |e| snap_len(e.thickness()));

    if let Some(e) = top_edge {
        for (off, t) in &e.bars {
            let y0 = top_outer + off * UNIT;
            out.push(left_outer, y0, right_outer, y0 + t * UNIT, e, RuleSide::Top);
        }
    }
    if let Some(e) = bottom_edge {
        for (off, t) in &e.bars {
            let y0 = bottom_inner + off * UNIT;
            out.push(
                left_outer,
                y0,
                right_outer,
                y0 + t * UNIT,
                e,
                RuleSide::Bottom,
            );
        }
    }
    // Between rules: at every paragraph boundary inside the box.
    for w in lines.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if a.paragraph == b.paragraph {
            continue;
        }
        let Some(e) = &fb.spec.between else { continue };
        let y_next = b.top;
        let y0 = snap(y_next - e.space - e.extent);
        for (off, t) in &e.bars {
            let y = y0 + off * UNIT;
            out.push(
                left_outer,
                y,
                right_outer,
                y + t * UNIT,
                e,
                RuleSide::Between,
            );
        }
    }
    // The side rules run the whole height of the box.
    if let Some(e) = &spec.left {
        for (off, t) in &e.bars {
            let x0 = left_inner - e.thickness() + off * UNIT;
            out.push(
                x0,
                top_outer,
                x0 + t * UNIT,
                bottom_outer,
                e,
                RuleSide::Left,
            );
        }
    }
    if let Some(e) = &spec.right {
        for (off, t) in &e.bars {
            let x0 = right_inner + off * UNIT;
            out.push(
                x0,
                top_outer,
                x0 + t * UNIT,
                bottom_outer,
                e,
                RuleSide::Right,
            );
        }
    }
}

/// A length rounded to whole grid units (bars are unit multiples already).
fn snap_len(v: f64) -> f64 {
    (v / UNIT + 1e-6).round() * UNIT
}
