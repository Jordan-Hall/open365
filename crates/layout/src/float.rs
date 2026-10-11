//! Floating pictures: where Word puts an anchored drawing and how it displaces text.
//!
//! A picture that wraps top and bottom pushes every line that would overlap it below it. A
//! picture that wraps square (tight and through are treated alike: pictures are rectangles)
//! narrows the lines beside it; a line that cannot hold its first word beside the picture goes
//! below it. Rules measured with `fixtures/docx/layout-img-*`.

use open365_docx::{
    Align, Drawing, HorizontalRelativeFrom, Position, VerticalRelativeFrom, WrapSide, WrapType,
};

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
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Float {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub kind: FloatKind,
    pub side: WrapSide,
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
}

/// Side of the square the coordinates of a wrap polygon are in.
const POLYGON_UNITS: f64 = 21600.0;

fn horizontal_base(rel: HorizontalRelativeFrom, s: &SectionProps) -> (f64, f64) {
    let margin = (s.margin_left + s.gutter, s.page_width - s.margin_right);
    match rel {
        HorizontalRelativeFrom::Page => (0.0, s.page_width),
        HorizontalRelativeFrom::LeftMargin | HorizontalRelativeFrom::InsideMargin => {
            (0.0, margin.0)
        }
        HorizontalRelativeFrom::RightMargin | HorizontalRelativeFrom::OutsideMargin => {
            (margin.1, s.page_width)
        }
        HorizontalRelativeFrom::Margin
        | HorizontalRelativeFrom::Column
        | HorizontalRelativeFrom::Character => margin,
    }
}

fn vertical_base(rel: VerticalRelativeFrom, s: &SectionProps, para_top: f64) -> (f64, f64) {
    match rel {
        VerticalRelativeFrom::Page => (0.0, s.page_height),
        VerticalRelativeFrom::Margin
        | VerticalRelativeFrom::InsideMargin
        | VerticalRelativeFrom::OutsideMargin => (s.margin_top, s.page_height - s.margin_bottom),
        VerticalRelativeFrom::TopMargin => (0.0, s.margin_top),
        VerticalRelativeFrom::BottomMargin => (s.page_height - s.margin_bottom, s.page_height),
        VerticalRelativeFrom::Paragraph | VerticalRelativeFrom::Line => (para_top, para_top),
    }
}

fn along(pos: Position, base: (f64, f64), size: f64) -> f64 {
    match pos {
        Position::Offset(e) => base.0 + e as f64 / open365_docx::EMU_PER_POINT,
        Position::Align(Align::Center) => base.0 + (base.1 - base.0 - size) / 2.0,
        Position::Align(Align::Right | Align::Outside | Align::Bottom) => base.1 - size,
        Position::Align(Align::Left | Align::Inside | Align::Top) => base.0,
    }
}

/// Positions the anchored `d` of a paragraph whose first line sits at `para_top`.
pub(crate) fn place(
    d: &Drawing,
    atom: usize,
    frame: &Frame,
    para_top: f64,
    paragraph: usize,
    text_box: Option<crate::textbox::BoxRef>,
) -> Anchored {
    let s = &frame.section;
    let (w, h) = d.size_pt();
    let hb = horizontal_base(d.horizontal.0, s);
    let vb = vertical_base(d.vertical.0, s, para_top);
    let x = along(d.horizontal.1, hb, w);
    let y = along(d.vertical.1, vb, h);
    let [el, et, er, eb] = d.effect_extent_pt();
    let [dt, db, dl, dr] = d.distance_pt();
    let kind = match d.wrap {
        WrapType::TopAndBottom => Some(FloatKind::TopBottom),
        WrapType::Square | WrapType::Tight | WrapType::Through => Some(FloatKind::Square),
        WrapType::None | WrapType::Inline => None,
    };
    // A tight or through wrap follows its polygon (a rectangle for a picture): the polygon's
    // bounding box in the 21600-unit square over the extent, with no effect extent (measured).
    let (bx0, by0, bx1, by1) = match (d.wrap, d.wrap_polygon) {
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
    floats: &[Float],
    frame: &Frame,
    base: Base,
) -> Dynamic {
    let paragraph = base.para + para.index;
    let anchors: Vec<Anchored> = para
        .objects
        .iter()
        .filter(|o| !o.inline() && o.atom >= from_atom)
        .map(|o| place(&o.drawing, o.atom, frame, y0, paragraph, o.text_box.clone()))
        .collect();
    let mut all: Vec<Float> = floats.to_vec();
    all.extend(anchors.iter().filter_map(|a| a.float));

    let s = &frame.section;
    let (area_l, area_r) = (s.margin_left + s.gutter, s.page_width - s.margin_right);
    let builder = para.builder();
    let n = para.atoms.len();
    let mut lines: Vec<LineBox> = Vec::new();
    let mut a0 = from_atom;
    let mut cursor = y0;
    while a0 < n {
        let first = a0 == 0;
        let mut y = cursor;
        let mut inset = (0.0f64, 0.0f64);
        let mut line = builder.fill_in(a0, first, inset);
        for _ in 0..12 {
            let (top, bottom) = (y, y + line.height);
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
                inset = (0.0, 0.0);
                line = builder.fill_in(a0, first, inset);
                continue;
            }
            // Square pictures leave a strip of the text area.
            let (mut l, mut r) = (area_l, area_r);
            for f in hits.iter().filter(|f| f.kind == FloatKind::Square) {
                match f.side {
                    WrapSide::Left => r = r.min(f.x0),
                    WrapSide::Right => l = l.max(f.x1),
                    WrapSide::BothSides | WrapSide::Largest => {
                        if f.x0 - l >= r - f.x1 {
                            r = r.min(f.x0);
                        } else {
                            l = l.max(f.x1);
                        }
                    }
                }
            }
            let want = ((l - area_l).max(0.0), (area_r - r).max(0.0));
            let narrowed = want.0 > 0.0 || want.1 > 0.0;
            if narrowed {
                // The first word must fit beside the picture, or the line goes below it.
                let first_adv = line_min_width(para, &line);
                if r - l < first_adv - 1e-6 {
                    let below = hits
                        .iter()
                        .filter(|f| f.kind == FloatKind::Square)
                        .map(|f| f.y1)
                        .fold(y, f64::max);
                    y = below;
                    inset = (0.0, 0.0);
                    line = builder.fill_in(a0, first, inset);
                    continue;
                }
            }
            if want == inset {
                break;
            }
            inset = want;
            line = builder.fill_in(a0, first, inset);
        }
        let lead = y - cursor;
        cursor = y + line.height;
        line.lead = lead;
        a0 = line.atoms.end;
        lines.push(line);
    }
    Dynamic { lines, anchors }
}

/// Width of the first box of a line: what must fit beside a picture.
fn line_min_width(para: &ParaLayout, line: &LineBox) -> f64 {
    let a = &para.atoms[line.atoms.start];
    let mut w = 0.0;
    for atom in &para.atoms[line.atoms.start..line.atoms.end] {
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
