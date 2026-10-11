//! Typed read access to `w:drawing` (ECMA-376 Part 1 section 20.4): the size, wrapping and
//! placement of an inline or anchored picture, and the image part it refers to.
//!
//! Nothing here is written back: the drawing stays raw XML in its run. Values are clamped to
//! what a page can hold so that hostile numbers cannot overflow later arithmetic.

use crate::xml::{ElementHead, RawXml, XmlElement};

/// English Metric Units per point.
pub const EMU_PER_POINT: f64 = 12700.0;
/// Largest magnitude of any length read from a drawing: about 1.6 km, far beyond a page.
const MAX_EMU: i64 = 2_000_000_000;

/// How text flows around a drawing (`wp:inline`, or the `wp:wrap*` child of `wp:anchor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrapType {
    /// `wp:inline`: the drawing is a character of the line.
    Inline,
    /// `wp:wrapSquare`.
    Square,
    /// `wp:wrapTight`.
    Tight,
    /// `wp:wrapThrough`.
    Through,
    /// `wp:wrapTopAndBottom`.
    TopAndBottom,
    /// `wp:wrapNone`: in front of or behind the text, which ignores it.
    None,
}

/// Which side of a square, tight or through wrapped drawing text may use (`wp:wrapText`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrapSide {
    BothSides,
    Left,
    Right,
    Largest,
}

/// `relativeFrom` of `wp:positionH`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HorizontalRelativeFrom {
    Margin,
    Page,
    Column,
    Character,
    LeftMargin,
    RightMargin,
    InsideMargin,
    OutsideMargin,
}

/// `relativeFrom` of `wp:positionV`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalRelativeFrom {
    Margin,
    Page,
    Paragraph,
    Line,
    TopMargin,
    BottomMargin,
    InsideMargin,
    OutsideMargin,
}

/// `wp:align` of a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
    Inside,
    Outside,
    Top,
    Bottom,
}

/// A position along one axis: an offset in EMU from the `relativeFrom` base, or an alignment
/// against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Offset(i64),
    Align(Align),
}

/// An inline or anchored drawing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drawing {
    /// True for `wp:anchor`, false for `wp:inline`.
    pub anchored: bool,
    /// `wp:extent` (`cx`, `cy`) in EMU.
    pub extent_emu: (i64, i64),
    /// `wp:effectExtent` (`l`, `t`, `r`, `b`) in EMU: room for effects such as borders and
    /// shadows outside the extent.
    pub effect_extent_emu: [i64; 4],
    pub wrap: WrapType,
    pub wrap_side: WrapSide,
    /// `distT`, `distB`, `distL`, `distR` in EMU (the wrap element's own values win over the
    /// anchor's).
    pub distance_emu: [i64; 4],
    /// Bounding box (`min x`, `min y`, `max x`, `max y`) of the `wp:wrapPolygon` of a tight or
    /// through wrap, in the polygon's 21600-unit square over the extent. `None` for other wraps
    /// and when the polygon is missing or empty.
    pub wrap_polygon: Option<[i64; 4]>,
    pub horizontal: (HorizontalRelativeFrom, Position),
    pub vertical: (VerticalRelativeFrom, Position),
    /// `behindDoc` of an anchor.
    pub behind_text: bool,
    /// `wp:docPr/@id`.
    pub id: Option<i64>,
    /// `wp:docPr/@name`.
    pub name: Option<String>,
    /// `wp:docPr/@descr`: the alternative text.
    pub description: Option<String>,
    /// `a:blip/@r:embed`: the relationship id of the image part.
    pub embed: Option<String>,
}

impl Drawing {
    /// Reads a `w:drawing` element; `None` when it holds neither `wp:inline` nor `wp:anchor`.
    pub fn from_element(drawing: &XmlElement) -> Option<Self> {
        let container = drawing
            .children
            .iter()
            .filter_map(RawXml::as_element)
            .find(|e| is_wp(&e.head, "inline") || is_wp(&e.head, "anchor"))?;
        let anchored = is_wp(&container.head, "anchor");
        let mut out = Self {
            anchored,
            extent_emu: (0, 0),
            effect_extent_emu: [0; 4],
            wrap: WrapType::Inline,
            wrap_side: WrapSide::BothSides,
            distance_emu: [
                emu_attr(&container.head, "distT"),
                emu_attr(&container.head, "distB"),
                emu_attr(&container.head, "distL"),
                emu_attr(&container.head, "distR"),
            ],
            wrap_polygon: None,
            horizontal: (HorizontalRelativeFrom::Column, Position::Offset(0)),
            vertical: (VerticalRelativeFrom::Paragraph, Position::Offset(0)),
            behind_text: anchored && bool_attr(&container.head, "behindDoc"),
            id: None,
            name: None,
            description: None,
            embed: None,
        };
        if anchored {
            out.wrap = WrapType::None;
        }
        for child in container.children.iter().filter_map(RawXml::as_element) {
            let h = &child.head;
            match h.local_name() {
                "extent" if is_wp(h, "extent") => {
                    out.extent_emu = (emu_attr(h, "cx").max(0), emu_attr(h, "cy").max(0));
                }
                "effectExtent" if is_wp(h, "effectExtent") => {
                    out.effect_extent_emu = [
                        emu_attr(h, "l"),
                        emu_attr(h, "t"),
                        emu_attr(h, "r"),
                        emu_attr(h, "b"),
                    ];
                }
                "docPr" if is_wp(h, "docPr") => {
                    out.id = attr(h, "id").and_then(|v| v.trim().parse().ok());
                    out.name = attr(h, "name").map(str::to_owned);
                    out.description = attr(h, "descr").map(str::to_owned);
                }
                "positionH" if anchored && is_wp(h, "positionH") => {
                    let rel = match attr(h, "relativeFrom") {
                        Some("page") => HorizontalRelativeFrom::Page,
                        Some("margin") => HorizontalRelativeFrom::Margin,
                        Some("character") => HorizontalRelativeFrom::Character,
                        Some("leftMargin") => HorizontalRelativeFrom::LeftMargin,
                        Some("rightMargin") => HorizontalRelativeFrom::RightMargin,
                        Some("insideMargin") => HorizontalRelativeFrom::InsideMargin,
                        Some("outsideMargin") => HorizontalRelativeFrom::OutsideMargin,
                        _ => HorizontalRelativeFrom::Column,
                    };
                    out.horizontal = (rel, position(child));
                }
                "positionV" if anchored && is_wp(h, "positionV") => {
                    let rel = match attr(h, "relativeFrom") {
                        Some("page") => VerticalRelativeFrom::Page,
                        Some("margin") => VerticalRelativeFrom::Margin,
                        Some("line") => VerticalRelativeFrom::Line,
                        Some("topMargin") => VerticalRelativeFrom::TopMargin,
                        Some("bottomMargin") => VerticalRelativeFrom::BottomMargin,
                        Some("insideMargin") => VerticalRelativeFrom::InsideMargin,
                        Some("outsideMargin") => VerticalRelativeFrom::OutsideMargin,
                        _ => VerticalRelativeFrom::Paragraph,
                    };
                    out.vertical = (rel, position(child));
                }
                "wrapNone" | "wrapSquare" | "wrapTight" | "wrapThrough" | "wrapTopAndBottom"
                    if anchored && is_wp(h, h.local_name()) =>
                {
                    out.wrap = match h.local_name() {
                        "wrapSquare" => WrapType::Square,
                        "wrapTight" => WrapType::Tight,
                        "wrapThrough" => WrapType::Through,
                        "wrapTopAndBottom" => WrapType::TopAndBottom,
                        _ => WrapType::None,
                    };
                    for (i, key) in ["distT", "distB", "distL", "distR"].into_iter().enumerate() {
                        if attr(h, key).is_some() {
                            out.distance_emu[i] = emu_attr(h, key);
                        }
                    }
                    out.wrap_polygon = polygon_bounds(child);
                    out.wrap_side = match attr(h, "wrapText") {
                        Some("left") => WrapSide::Left,
                        Some("right") => WrapSide::Right,
                        Some("largest") => WrapSide::Largest,
                        _ => WrapSide::BothSides,
                    };
                }
                _ => {}
            }
        }
        out.embed = find_embed(container, 0);
        Some(out)
    }

    /// Width and height of the picture in points (`wp:extent`).
    pub fn size_pt(&self) -> (f64, f64) {
        (
            self.extent_emu.0 as f64 / EMU_PER_POINT,
            self.extent_emu.1 as f64 / EMU_PER_POINT,
        )
    }

    /// `wp:effectExtent` as left, top, right, bottom in points.
    pub fn effect_extent_pt(&self) -> [f64; 4] {
        self.effect_extent_emu.map(|v| v as f64 / EMU_PER_POINT)
    }

    /// `distT`, `distB`, `distL`, `distR` in points.
    pub fn distance_pt(&self) -> [f64; 4] {
        self.distance_emu.map(|v| v as f64 / EMU_PER_POINT)
    }
}

/// The image part a relationship id of the main part points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePart<'a> {
    /// Package part name, such as `/word/media/image1.png`.
    pub name: String,
    /// The content type the package declares for the part (empty when it declares none).
    pub content_type: String,
    pub bytes: &'a [u8],
}

/// Bounding box of the points of a `wp:wrapTight` or `wp:wrapThrough` polygon, clamped to a
/// generous multiple of the 21600 square.
fn polygon_bounds(wrap: &XmlElement) -> Option<[i64; 4]> {
    let poly = wrap
        .children
        .iter()
        .filter_map(RawXml::as_element)
        .find(|e| is_wp(&e.head, "wrapPolygon"))?;
    let mut b: Option<[i64; 4]> = None;
    for p in poly
        .children
        .iter()
        .filter_map(RawXml::as_element)
        .filter(|e| is_wp(&e.head, "start") || is_wp(&e.head, "lineTo"))
    {
        let get = |k: &str| {
            attr(&p.head, k)
                .and_then(|v| v.trim().parse::<i64>().ok())
                .map(|v| v.clamp(-1_000_000, 1_000_000))
        };
        let (Some(x), Some(y)) = (get("x"), get("y")) else {
            continue;
        };
        b = Some(match b {
            None => [x, y, x, y],
            Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
        });
    }
    b.filter(|b| b[2] > b[0] && b[3] > b[1])
}

fn is_wp(head: &ElementHead, local: &str) -> bool {
    head.local_name() == local
        && matches!(
            head.ns.as_deref(),
            Some(
                "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
                    | "http://purl.oclc.org/ooxml/drawingml/wordprocessingDrawing"
            )
        )
}

/// An unprefixed attribute.
fn attr<'a>(head: &'a ElementHead, name: &str) -> Option<&'a str> {
    head.attrs
        .iter()
        .find(|a| a.ns.is_none() && a.name == name)
        .map(|a| a.value.as_str())
}

fn emu_attr(head: &ElementHead, name: &str) -> i64 {
    attr(head, name)
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(0)
        .clamp(-MAX_EMU, MAX_EMU)
}

fn bool_attr(head: &ElementHead, name: &str) -> bool {
    matches!(attr(head, name).map(str::trim), Some("1" | "true" | "on"))
}

fn text_of(e: &XmlElement) -> String {
    e.children
        .iter()
        .filter_map(|c| match c {
            RawXml::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect()
}

fn position(e: &XmlElement) -> Position {
    for c in e.children.iter().filter_map(RawXml::as_element) {
        if is_wp(&c.head, "posOffset") {
            let v = text_of(c).trim().parse::<i64>().unwrap_or(0);
            return Position::Offset(v.clamp(-MAX_EMU, MAX_EMU));
        }
        if is_wp(&c.head, "align") {
            let a = match text_of(c).trim() {
                "right" => Align::Right,
                "center" => Align::Center,
                "inside" => Align::Inside,
                "outside" => Align::Outside,
                "top" => Align::Top,
                "bottom" => Align::Bottom,
                _ => Align::Left,
            };
            return Position::Align(a);
        }
    }
    Position::Offset(0)
}

/// The first `a:blip` embed relationship id below `e`. Depth is capped: the parser already
/// limits nesting, this keeps the search cheap.
fn find_embed(e: &XmlElement, depth: usize) -> Option<String> {
    if depth > 16 {
        return None;
    }
    for c in e.children.iter().filter_map(RawXml::as_element) {
        if c.head.local_name() == "blip" {
            return c
                .head
                .attrs
                .iter()
                .find(|a| a.local_name() == "embed")
                .map(|a| a.value.clone());
        }
        if let Some(found) = find_embed(c, depth + 1) {
            return Some(found);
        }
    }
    None
}
