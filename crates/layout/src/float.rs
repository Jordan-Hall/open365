//! Floating pictures: where Word puts an anchored drawing and how it displaces text.
//!
//! A picture that wraps top and bottom pushes every line that would overlap it below it. A
//! picture that wraps square (tight and through are treated alike: pictures are rectangles)
//! narrows the lines beside it; a line that cannot hold its first word beside the picture goes
//! below it. Rules measured with `fixtures/docx/layout-img-*`.

use open365_docx::{
    Align, Drawing, HorizontalRelativeFrom, Position, VerticalRelativeFrom, WrapSide, WrapType,
};

use std::sync::Arc;

use crate::ImagePlacement;
use crate::flow::Base;
use crate::para::{LineBox, ParaLayout};
use crate::props::SectionProps;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FloatKind {
    TopBottom,
    Square,
}

/// The rectangle text keeps out of: the picture grown by its effect extent and the wrapping
/// distances.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Float {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub kind: FloatKind,
    pub side: WrapSide,
    /// The drawing's own rectangle (no wrap distances), what `allowOverlap="0"` keeps apart.
    pub rect: [f64; 4],
    /// Outline a tight or through wrap follows, in page coordinates, with the wrap distances.
    pub outline: Option<Arc<Outline>>,
}

/// The wrap polygon of a tight or through wrap.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Outline {
    pub points: Vec<(f64, f64)>,
    pub dist_l: f64,
    pub dist_r: f64,
}

impl Float {
    /// Horizontal extent text keeps out of between `top` and `bottom`, wrap distances included;
    /// `None` when the outline does not reach that band.
    fn extent(&self, top: f64, bottom: f64) -> Option<(f64, f64)> {
        let Some(o) = &self.outline else {
            return Some((self.x0, self.x1));
        };
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        let n = o.points.len();
        let (top, bottom) = match std::env::var("VAR").as_deref() {
            Ok("B") => (bottom, bottom),
            Ok("C") => (top, top),
            _ => (top, bottom),
        };
        for i in 0..n {
            let (a, b) = (o.points[i], o.points[(i + 1) % n]);
            // The part of the edge inside the band.
            let (ya, yb) = (a.1.min(b.1), a.1.max(b.1));
            if yb < top || ya > bottom {
                continue;
            }
            let at = |y: f64| {
                if (b.1 - a.1).abs() < 1e-9 {
                    None
                } else {
                    Some(a.0 + (b.0 - a.0) * (y - a.1) / (b.1 - a.1))
                }
            };
            let (y0, y1) = (ya.max(top), yb.min(bottom));
            let (x0, x1) = match (at(y0), at(y1)) {
                (Some(p), Some(q)) => (p, q),
                _ => (a.0, b.0),
            };
            lo = lo.min(x0).min(x1);
            hi = hi.max(x0).max(x1);
        }
        (lo <= hi).then_some((lo - o.dist_l, hi + o.dist_r))
    }
}

/// Attributes of `wp:anchor` the typed drawing does not keep.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AnchorFlags {
    pub allow_overlap: bool,
    pub layout_in_cell: bool,
    /// `wp:wrapPolygon` points in its 21600-unit square.
    pub polygon: Option<Arc<[(f64, f64)]>>,
}

impl Default for AnchorFlags {
    fn default() -> Self {
        Self {
            allow_overlap: true,
            layout_in_cell: true,
            polygon: None,
        }
    }
}

impl AnchorFlags {
    /// Reads `allowOverlap` and `layoutInCell` of the first `wp:anchor` in `e`.
    pub(crate) fn read(e: &open365_docx::XmlElement) -> Self {
        fn find(e: &open365_docx::XmlElement) -> Option<&open365_docx::XmlElement> {
            if e.head.local_name() == "anchor" {
                return Some(e);
            }
            e.children
                .iter()
                .filter_map(open365_docx::RawXml::as_element)
                .find_map(find)
        }
        let flag = |e: &open365_docx::XmlElement, name: &str| {
            e.head
                .attrs
                .iter()
                .find(|a| a.ns.is_none() && a.name == name)
                .map(|a| matches!(a.value.trim(), "1" | "true" | "on"))
        };
        fn polygon(e: &open365_docx::XmlElement, out: &mut Vec<(f64, f64)>) {
            for c in e
                .children
                .iter()
                .filter_map(open365_docx::RawXml::as_element)
            {
                if matches!(c.head.local_name(), "start" | "lineTo") {
                    let num = |n: &str| {
                        c.head
                            .attrs
                            .iter()
                            .find(|a| a.ns.is_none() && a.name == n)
                            .and_then(|a| a.value.trim().parse::<f64>().ok())
                    };
                    if let (Some(x), Some(y)) = (num("x"), num("y")) {
                        out.push((x, y));
                    }
                } else {
                    polygon(c, out);
                }
            }
        }
        match find(e) {
            Some(a) => {
                let mut pts = Vec::new();
                for c in a
                    .children
                    .iter()
                    .filter_map(open365_docx::RawXml::as_element)
                {
                    if matches!(c.head.local_name(), "wrapTight" | "wrapThrough") {
                        polygon(c, &mut pts);
                    }
                }
                Self {
                    allow_overlap: flag(a, "allowOverlap").unwrap_or(true),
                    layout_in_cell: flag(a, "layoutInCell").unwrap_or(true),
                    polygon: (pts.len() >= 3).then(|| pts.into()),
                }
            }
            None => Self::default(),
        }
    }
}

/// A floating picture of a paragraph, placed.
#[derive(Debug, Clone)]
pub(crate) struct Anchored {
    /// Index of the object's atom.
    pub atom: usize,
    pub float: Option<Float>,
    pub image: ImagePlacement,
}

/// Page geometry the anchors are positioned against.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Frame {
    pub section: SectionProps,
    /// Left and right edge of the text column the paragraph is in (page coordinates).
    pub col: (f64, f64),
}

impl Frame {
    /// The frame of a section without columns: the column is the margin box.
    pub(crate) fn new(section: SectionProps) -> Self {
        Self {
            col: (
                section.margin_left + section.gutter,
                section.page_width - section.margin_right,
            ),
            section,
        }
    }
}

/// Side of the square the coordinates of a wrap polygon are in.
const POLYGON_UNITS: f64 = 21600.0;

/// Where the anchor character sits once its line is known: the left edge of the character and
/// the top of its line.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AnchorPoint {
    pub x: f64,
    pub line_top: f64,
}

/// The drawing in the `mc:Choice` of an `mc:AlternateContent`: a text box or any other shape.
/// (The typed model keeps only text boxes; a shape without text still wraps the body text and
/// counts as one character.)
pub(crate) fn alternate_drawing(e: &open365_docx::XmlElement) -> Option<Drawing> {
    if e.head.local_name() != "AlternateContent" {
        return None;
    }
    let choice = e
        .children
        .iter()
        .filter_map(open365_docx::RawXml::as_element)
        .find(|c| c.head.local_name() == "Choice")?;
    choice
        .children
        .iter()
        .filter_map(open365_docx::RawXml::as_element)
        .filter(|d| d.head.is_w("drawing"))
        .find_map(Drawing::from_element)
}

/// Whether a drawing's position depends on the line that holds its anchor.
fn needs_line(d: &Drawing) -> bool {
    d.anchored
        && (d.horizontal.0 == HorizontalRelativeFrom::Character
            || d.vertical.0 == VerticalRelativeFrom::Line)
}

fn horizontal_base(
    rel: HorizontalRelativeFrom,
    s: &SectionProps,
    col: (f64, f64),
    at: Option<AnchorPoint>,
) -> (f64, f64) {
    let margin = (s.margin_left + s.gutter, s.page_width - s.margin_right);
    match rel {
        HorizontalRelativeFrom::Page => (0.0, s.page_width),
        HorizontalRelativeFrom::LeftMargin | HorizontalRelativeFrom::InsideMargin => {
            (0.0, margin.0)
        }
        HorizontalRelativeFrom::RightMargin | HorizontalRelativeFrom::OutsideMargin => {
            (margin.1, s.page_width)
        }
        HorizontalRelativeFrom::Character => {
            at.map_or(margin, |a| (a.x, a.x + margin.1 - margin.0))
        }
        HorizontalRelativeFrom::Margin => margin,
        HorizontalRelativeFrom::Column => col,
    }
}

fn vertical_base(
    rel: VerticalRelativeFrom,
    s: &SectionProps,
    para_top: f64,
    at: Option<AnchorPoint>,
) -> (f64, f64) {
    match rel {
        VerticalRelativeFrom::Page => (0.0, s.page_height),
        VerticalRelativeFrom::Margin
        | VerticalRelativeFrom::InsideMargin
        | VerticalRelativeFrom::OutsideMargin => (s.margin_top, s.page_height - s.margin_bottom),
        VerticalRelativeFrom::TopMargin => (0.0, s.margin_top),
        VerticalRelativeFrom::BottomMargin => (s.page_height - s.margin_bottom, s.page_height),
        VerticalRelativeFrom::Paragraph => (para_top, para_top),
        VerticalRelativeFrom::Line => {
            let top = at.map_or(para_top, |a| a.line_top);
            (top, top)
        }
    }
}

fn along(pos: Position, base: (f64, f64), size: f64, far_effect: f64) -> f64 {
    match pos {
        Position::Offset(e) => base.0 + e as f64 / open365_docx::EMU_PER_POINT,
        Position::Align(Align::Center) => base.0 + (base.1 - base.0 - size) / 2.0,
        Position::Align(Align::Right | Align::Outside | Align::Bottom) => {
            base.1 - size - far_effect
        }
        Position::Align(Align::Left | Align::Inside | Align::Top) => base.0,
    }
}

#[allow(clippy::too_many_arguments)]
/// Positions the anchored `d` of a paragraph whose first line sits at `para_top`.
pub(crate) fn place(
    d: &Drawing,
    atom: usize,
    frame: &Frame,
    para_top: f64,
    paragraph: usize,
    text_box: Option<crate::textbox::BoxRef>,
    at: Option<AnchorPoint>,
    polygon: Option<&Arc<[(f64, f64)]>>,
) -> Anchored {
    let s = &frame.section;
    let (w, h) = d.size_pt();
    let hb = horizontal_base(d.horizontal.0, s, frame.col, at);
    let vb = vertical_base(d.vertical.0, s, para_top, at);
    let [el, et, er, eb] = d.effect_extent_pt();
    let x = along(d.horizontal.1, hb, w, er);
    let y = along(d.vertical.1, vb, h, eb);
    let [dt, db, dl, dr] = d.distance_pt();
    let kind = match d.wrap {
        WrapType::TopAndBottom => Some(FloatKind::TopBottom),
        WrapType::Square | WrapType::Tight | WrapType::Through => Some(FloatKind::Square),
        WrapType::None | WrapType::Inline => None,
    };
    // A tight or through wrap follows its polygon (a rectangle for a picture): the polygon's
    // bounding box in the 21600-unit square over the extent, with no effect extent (measured).
    let outline: Option<Outline> = match (d.wrap, polygon) {
        (WrapType::Tight | WrapType::Through, Some(p)) => Some(Outline {
            points: p
                .iter()
                .map(|&(px, py)| (x + w * px / POLYGON_UNITS, y + h * py / POLYGON_UNITS))
                .collect(),
            dist_l: dl,
            dist_r: dr,
        }),
        _ => None,
    };
    let (bx0, by0, bx1, by1) = match (d.wrap, d.wrap_polygon) {
        _ if outline.is_some() => {
            let pts = outline.iter().flat_map(|o| o.points.iter().copied());
            let fold = |f: fn(f64, f64) -> f64, init: f64, k: fn((f64, f64)) -> f64| {
                pts.clone().map(k).fold(init, f)
            };
            (
                fold(f64::min, f64::INFINITY, |p| p.0),
                fold(f64::min, f64::INFINITY, |p| p.1),
                fold(f64::max, f64::NEG_INFINITY, |p| p.0),
                fold(f64::max, f64::NEG_INFINITY, |p| p.1),
            )
        }
        (WrapType::Tight | WrapType::Through, Some([x0, y0, x1, y1])) => (
            x + w * x0 as f64 / POLYGON_UNITS,
            y + h * y0 as f64 / POLYGON_UNITS,
            x + w * x1 as f64 / POLYGON_UNITS,
            y + h * y1 as f64 / POLYGON_UNITS,
        ),
        _ => (x - el, y - et, x + w + er, y + h + eb),
    };
    Anchored {
        atom,
        float: kind.map(|kind| Float {
            x0: bx0 - dl,
            y0: by0 - dt,
            x1: bx1 + dr,
            y1: by1 + db,
            kind,
            side: d.wrap_side,
            rect: [x, y, x + w, y + h],
            outline: outline.map(Arc::new),
        }),
        image: ImagePlacement {
            x,
            y,
            width: w,
            height: h,
            embed: d.embed.clone(),
            description: d.description.clone(),
            paragraph,
            anchored: true,
            behind_text: d.behind_text,
            source: crate::ImageSource::Main,
            text_box,
        },
    }
}

/// The lines of `para` from atom `from_atom` on, broken and positioned around the floating
/// pictures `floats` (those already on the page, plus the paragraph's own), starting with the
/// first line's top at `y0`.
pub(crate) struct Dynamic {
    pub lines: Vec<LineBox>,
    /// The paragraph's own pictures from `from_atom` on.
    pub anchors: Vec<Anchored>,
}

pub(crate) fn dynamic_lines(
    para: &ParaLayout,
    from_atom: usize,
    y0: f64,
    anchor_top: Option<f64>,
    floats: &[Float],
    frame: &Frame,
    base: Base,
) -> Dynamic {
    let paragraph = base.para + para.index;
    let objects: Vec<&crate::para::ObjectBox> = para
        .objects
        .iter()
        .filter(|o| !o.inline() && o.atom >= from_atom)
        .collect();
    let flags: Vec<AnchorFlags> = objects.iter().map(|o| o.flags.clone()).collect();
    // Drawings placed against their anchor's line wait for it; they never move the lines above
    // (nor the anchor's own line, measured with `layout-float-posv-line`).
    let deferred: Vec<bool> = objects.iter().map(|o| needs_line(&o.drawing)).collect();
    let mut anchors: Vec<Anchored> = objects
        .iter()
        .map(|o| {
            let top = anchor_top.unwrap_or(y0);
            let mut a = place(
                &o.drawing,
                o.atom,
                frame,
                top,
                paragraph,
                o.text_box.clone(),
                None,
                o.flags.polygon.as_ref(),
            );
            if needs_line(&o.drawing) {
                a.float = None;
            }
            a
        })
        .collect();
    #[allow(clippy::needless_range_loop)]
    for i in 0..anchors.len() {
        if !deferred[i] {
            separate_one(&mut anchors, &flags, i, floats, frame.col);
        }
    }
    let mut all: Vec<Float> = floats.to_vec();
    all.extend(anchors.iter().filter_map(|a| a.float.clone()));
    let mut placed: Vec<bool> = deferred.iter().map(|d| !d).collect();

    let (area_l, area_r) = frame.col;
    // The paragraph was laid out at its unit's width; the column may be narrower or wider.
    let delta = para.fill.text_width - (area_r - area_l);
    let builder = para.builder();
    let n = para.atoms.len();
    let mut lines: Vec<LineBox> = Vec::new();
    let mut a0 = from_atom;
    let mut cursor = y0;
    while a0 < n {
        let first = a0 == 0;
        let mut y = cursor;
        let whole = (0.0, delta);
        let mut est = builder.fill_in(a0, first, whole).height;
        let mut row: Vec<LineBox> = Vec::new();
        for _ in 0..16 {
            let (top, bottom) = (y, y + est);
            let hits: Vec<&Float> = all
                .iter()
                .filter(|f| f.y0 < bottom - 1e-6 && f.y1 > top + 1e-6)
                .collect();
            // Top-and-bottom pictures push the line below them.
            if let Some(f) = hits
                .iter()
                .filter(|f| f.kind == FloatKind::TopBottom)
                .max_by(|a, b| a.y1.total_cmp(&b.y1))
            {
                y = f.y1;
                est = builder.fill_in(a0, first, whole).height;
                continue;
            }
            let squares: Vec<&Float> = hits
                .iter()
                .copied()
                .filter(|f| f.kind == FloatKind::Square)
                .collect();
            let spans = free_spans(&squares, area_l, area_r, (top, bottom));
            // One line per free span, left to right, all on the same row.
            row.clear();
            let mut at = a0;
            let narrowed = spans.len() != 1 || spans[0] != (area_l, area_r);
            for &(l, r) in &spans {
                if at >= n {
                    break;
                }
                let inset = ((l - area_l).max(0.0), (area_r - r).max(0.0) + delta);
                let seg_first = first && row.is_empty();
                let line = builder.fill_in(at, seg_first, inset);
                // The first word must fit in the span, or the span stays empty.
                if narrowed && r - l < line_min_width(para, &line) - 1e-6 {
                    continue;
                }
                at = line.atoms.end;
                row.push(line);
            }
            if row.is_empty() {
                // Nowhere beside the pictures: below them.
                let below = squares.iter().map(|f| f.y1).fold(y, f64::max);
                y = if below > y + 1e-6 { below } else { y + est };
                est = builder.fill_in(a0, first, whole).height;
                continue;
            }
            let h = row.iter().map(|l| l.height).fold(0.0, f64::max);
            if h > est + 1e-6 && narrowed {
                est = h;
                continue;
            }
            break;
        }
        if row.is_empty() {
            row.push(builder.fill_in(a0, first, whole));
        }
        let h = row.iter().map(|l| l.height).fold(0.0, f64::max);
        let base = row.iter().map(|l| l.baseline_off).fold(0.0, f64::max);
        let lead = y - cursor;
        for (k, o) in objects.iter().enumerate() {
            if placed[k] {
                continue;
            }
            let Some(line) = row.iter().find(|l| l.atoms.contains(&o.atom)) else {
                continue;
            };
            // Word measures from the character before the anchor (rule 9 of the research note).
            let prev = (o.atom - line.atoms.start).saturating_sub(1);
            let at = AnchorPoint {
                x: area_l + line.xs.get(prev).copied().unwrap_or(line.left),
                line_top: y,
            };
            let mut a = place(
                &o.drawing,
                o.atom,
                frame,
                y0,
                paragraph,
                o.text_box.clone(),
                Some(at),
                o.flags.polygon.as_ref(),
            );
            std::mem::swap(&mut anchors[k], &mut a);
            placed[k] = true;
            separate_one(&mut anchors, &flags, k, &all, frame.col);
            all.extend(anchors[k].float.clone());
        }
        for (i, mut line) in row.into_iter().enumerate() {
            line.height = h;
            line.baseline_off = base;
            line.lead = if i == 0 { lead } else { -h };
            a0 = line.atoms.end;
            lines.push(line);
        }
        cursor = y + h;
    }
    Dynamic { lines, anchors }
}

/// The parts of `l..r` left of text beside the square floats `fs`, left to right.
fn free_spans(fs: &[&Float], l: f64, r: f64, band: (f64, f64)) -> Vec<(f64, f64)> {
    let mut spans = vec![(l, r)];
    for f in fs {
        let Some((fx0, fx1)) = f.extent(band.0, band.1) else {
            continue;
        };
        let mut next: Vec<(f64, f64)> = Vec::new();
        let mut cut: Vec<(f64, f64)> = Vec::new();
        for &(a, b) in &spans {
            if fx1 <= a || fx0 >= b {
                next.push((a, b));
                continue;
            }
            let left = (a, fx0.min(b));
            let right = (fx1.max(a), b);
            let (keep_l, keep_r) = match f.side {
                WrapSide::Left => (true, false),
                WrapSide::Right => (false, true),
                WrapSide::BothSides | WrapSide::Largest => (true, true),
            };
            if keep_l && left.1 - left.0 > 1e-6 {
                cut.push(left);
            }
            if keep_r && right.1 - right.0 > 1e-6 {
                cut.push(right);
            }
        }
        if f.side == WrapSide::Largest && cut.len() > 1 {
            let widest = cut
                .iter()
                .copied()
                .max_by(|a, b| (a.1 - a.0).total_cmp(&(b.1 - b.0)))
                .unwrap_or((l, r));
            cut = vec![widest];
        }
        next.extend(cut);
        next.sort_by(|a, b| a.0.total_cmp(&b.0));
        spans = next;
    }
    spans
}

/// The wrapping floats of `next` that reach above `limit`, placed for a paragraph top `top`.
pub(crate) fn ahead_floats(next: &ParaLayout, frame: &Frame, top: f64, limit: f64) -> Vec<Float> {
    next.objects
        .iter()
        .filter(|o| !o.inline())
        .filter_map(|o| {
            place(
                &o.drawing,
                o.atom,
                frame,
                top,
                0,
                None,
                None,
                o.flags.polygon.as_ref(),
            )
            .float
        })
        .filter(|f| f.y0 < limit - 1e-6)
        .collect()
}

/// `allowOverlap="0"`: drawing `i` moves right until its left edge meets the right edge of an
/// earlier drawing it covers (left of it when there is no room), measured with
/// `layout-float-collide*`.
fn separate_one(
    anchors: &mut [Anchored],
    flags: &[AnchorFlags],
    i: usize,
    page: &[Float],
    col: (f64, f64),
) {
    let (area_l, area_r) = col;
    if flags.get(i).is_none_or(|f| f.allow_overlap) || anchors[i].float.is_none() {
        return;
    }
    for _ in 0..8 {
        let Some(me) = anchors[i].float.clone() else {
            break;
        };
        let hit = page
            .iter()
            .cloned()
            .chain(anchors[..i].iter().filter_map(|a| a.float.clone()))
            .find(|o| {
                me.rect[0] < o.rect[2] - 1e-6
                    && me.rect[2] > o.rect[0] + 1e-6
                    && me.rect[1] < o.rect[3] - 1e-6
                    && me.rect[3] > o.rect[1] + 1e-6
            });
        let Some(o) = hit else { break };
        let w = me.rect[2] - me.rect[0];
        let mut nx = o.rect[2];
        if nx + w > area_r + 1e-6 && o.rect[0] - w >= area_l - 1e-6 {
            nx = o.rect[0] - w;
        }
        let dx = nx - me.rect[0];
        if dx.abs() < 1e-9 {
            break;
        }
        let a = &mut anchors[i];
        a.image.x += dx;
        if let Some(f) = &mut a.float {
            f.x0 += dx;
            f.x1 += dx;
            f.rect[0] += dx;
            f.rect[2] += dx;
            if let Some(o) = &f.outline {
                let mut o = (**o).clone();
                for p in &mut o.points {
                    p.0 += dx;
                }
                f.outline = Some(Arc::new(o));
            }
        }
    }
}

/// Width of the first box of a line: what must fit beside a picture.
fn line_min_width(para: &ParaLayout, line: &LineBox) -> f64 {
    let a = &para.atoms[line.atoms.start];
    let mut w = 0.0;
    for atom in &para.atoms[line.atoms.start..] {
        if matches!(
            atom.class,
            crate::para::AtomClass::Space
                | crate::para::AtomClass::Tab
                | crate::para::AtomClass::Mark
                | crate::para::AtomClass::LineBreak
                | crate::para::AtomClass::PageBreak
        ) && atom.byte_start != a.byte_start
        {
            break;
        }
        w += atom.adv;
    }
    w
}
