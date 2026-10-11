//! Editing the style sheet the way Excel does: a change to a cell derives a new `cellXfs/xf`
//! from the cell's current one, reusing an identical font, fill, border, number format and xf
//! when the sheet already has one. Everything is measured against Excel by
//! `tools/office/format-oracle.ps1`; `docs/architecture/xlsx-cell-formatting.md` lists the rules.
//!
//! The edit works on the raw `styles.xml` tree, so anything the model does not interpret
//! (extension lists, table styles, unknown attributes) survives.

use std::collections::BTreeSet;

use crate::XlsxError;
use crate::format::{
    BorderEdge, BorderStyle, Color, ColorSource, FillPattern, HorizontalAlignment, Underline,
    VerticalAlignment,
};
use crate::styles::{Styles, builtin_number_format};
use crate::xml::{Attr, RawXml, XmlElement};
use open365_locale::builtin_number_format_in;

/// A change to a cell's font. `None` leaves the property as it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FontPatch {
    pub name: Option<String>,
    /// Size in points.
    pub size: Option<f64>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<Underline>,
    pub strike: Option<bool>,
    pub color: Option<Color>,
}

/// A cell fill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FillSpec {
    /// No fill.
    None,
    /// A solid fill; the colour is the visible colour.
    Solid(Color),
    /// A pattern; `fg` is the pattern colour, `bg` the colour behind it.
    Pattern {
        pattern: FillPattern,
        fg: Option<Color>,
        bg: Option<Color>,
    },
}

/// A change to the edges of a cell's border. `Some` with [`BorderStyle::None`] removes the edge.
/// The two diagonals share one line in the file, so the last one set decides its style and colour.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BorderPatch {
    pub left: Option<BorderEdge>,
    pub right: Option<BorderEdge>,
    pub top: Option<BorderEdge>,
    pub bottom: Option<BorderEdge>,
    /// Diagonal from the top left to the bottom right.
    pub diagonal_down: Option<BorderEdge>,
    /// Diagonal from the bottom left to the top right.
    pub diagonal_up: Option<BorderEdge>,
    /// Mark the border as applied even when no edge changes (Excel does this to the cells inside a
    /// range it draws an outline around).
    pub touch: bool,
}

impl BorderPatch {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Text rotation as Excel's `Orientation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    /// -90 (down) to 90 (up) degrees.
    Degrees(i32),
    /// Stacked vertical text.
    Stacked,
}

/// A change to a cell's alignment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AlignmentPatch {
    pub horizontal: Option<HorizontalAlignment>,
    pub vertical: Option<VerticalAlignment>,
    pub wrap_text: Option<bool>,
    pub indent: Option<u32>,
    pub rotation: Option<Rotation>,
    pub shrink_to_fit: Option<bool>,
}

/// A change to a cell's protection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProtectionPatch {
    pub locked: Option<bool>,
    /// Hide the formula when the sheet is protected.
    pub hidden: Option<bool>,
}

/// A number format: a built-in id or a format code. A code equal to a built-in one resolves to
/// the built-in id, as Excel does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NumberFormat {
    Builtin(u32),
    Code(String),
}

/// Everything a single formatting action can change. `None` and empty fields are left alone.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FormatPatch {
    pub font: FontPatch,
    pub fill: Option<FillSpec>,
    pub border: BorderPatch,
    pub alignment: AlignmentPatch,
    pub number_format: Option<NumberFormat>,
    pub protection: ProtectionPatch,
}

// ---- tree helpers ---------------------------------------------------------------------------

const FONT_RANK: [&str; 15] = [
    "b",
    "i",
    "strike",
    "condense",
    "extend",
    "outline",
    "shadow",
    "u",
    "vertAlign",
    "sz",
    "color",
    "name",
    "family",
    "charset",
    "scheme",
];

fn font_rank(local: &str) -> usize {
    FONT_RANK
        .iter()
        .position(|n| *n == local)
        .unwrap_or(FONT_RANK.len())
}

const APPLY_ORDER: [&str; 6] = [
    "applyNumberFormat",
    "applyFont",
    "applyFill",
    "applyBorder",
    "applyAlignment",
    "applyProtection",
];

/// Canonical text of an element: attribute order does not matter and font children are in
/// schema order, so two fonts that mean the same compare equal.
pub(crate) fn canon(e: &XmlElement) -> String {
    let mut out = String::new();
    canon_into(e, &mut out);
    out
}

fn canon_into(e: &XmlElement, out: &mut String) {
    out.push('<');
    out.push_str(e.head.local_name());
    let mut attrs: Vec<(&str, &str)> = e
        .head
        .attrs
        .iter()
        .filter(|a| a.ns.is_none())
        .map(|a| (a.name.as_str(), a.value.as_str()))
        .collect();
    attrs.sort_unstable();
    for (n, v) in attrs {
        out.push(' ');
        out.push_str(n);
        out.push_str("=\"");
        out.push_str(v);
        out.push('"');
    }
    out.push('>');
    let mut kids: Vec<&XmlElement> = e.elements().collect();
    if e.head.local_name() == "font" {
        kids.sort_by_key(|k| font_rank(k.head.local_name()));
    }
    for k in kids {
        canon_into(k, out);
    }
    out.push_str("</");
    out.push_str(e.head.local_name());
    out.push('>');
}

pub(crate) fn attr_u32(e: &XmlElement, name: &str) -> Option<u32> {
    e.head.attr(name)?.trim().parse().ok()
}

pub(crate) fn el(tpl: &XmlElement, local: &str, attrs: &[(&str, &str)]) -> XmlElement {
    let mut e = tpl.sibling(local);
    for (n, v) in attrs {
        e.head.attrs.push(Attr::plain(n, v));
    }
    e
}

pub(crate) fn remove_child(e: &mut XmlElement, local: &str) {
    e.children
        .retain(|c| !c.as_element().is_some_and(|c| c.head.is_main(local)));
}

fn child_mut<'a>(e: &'a mut XmlElement, local: &str) -> Option<&'a mut XmlElement> {
    e.children
        .iter_mut()
        .filter_map(RawXml::as_element_mut)
        .find(|c| c.head.is_main(local))
}

/// Puts `child` into a font in schema order, replacing a child of the same name.
pub(crate) fn set_font_child(font: &mut XmlElement, child: XmlElement) {
    let local = child.head.local_name().to_owned();
    remove_child(font, &local);
    let rank = font_rank(&local);
    let at = font
        .children
        .iter()
        .position(|c| {
            c.as_element()
                .is_some_and(|c| font_rank(c.head.local_name()) > rank)
        })
        .unwrap_or(font.children.len());
    font.children.insert(at, RawXml::Element(child));
}

/// `count` of a list element, kept in step with its items.
pub(crate) fn fix_count(sec: &mut XmlElement) {
    let n = sec.elements().count();
    sec.head.set_attr("count", &n.to_string());
}

/// The index of the section `name` among the root's children, creating it in schema order.
pub(crate) fn section_index(root: &mut XmlElement, name: &str) -> usize {
    if let Some(i) = root
        .children
        .iter()
        .position(|c| c.as_element().is_some_and(|c| c.head.is_main(name)))
    {
        return i;
    }
    const ORDER: [&str; 12] = [
        "numFmts",
        "fonts",
        "fills",
        "borders",
        "cellStyleXfs",
        "cellXfs",
        "cellStyles",
        "dxfs",
        "tableStyles",
        "colors",
        "extLst",
        "",
    ];
    let rank = ORDER.iter().position(|n| *n == name).unwrap_or(0);
    let at = root
        .children
        .iter()
        .position(|c| {
            c.as_element().is_some_and(|c| {
                ORDER
                    .iter()
                    .position(|n| *n == c.head.local_name())
                    .is_some_and(|r| r > rank)
            })
        })
        .unwrap_or(root.children.len());
    let mut sec = root.sibling(name);
    sec.head.attrs.push(Attr::plain("count", "0"));
    root.children.insert(at, RawXml::Element(sec));
    at
}

pub(crate) fn section<'a>(root: &'a XmlElement, name: &str) -> Option<&'a XmlElement> {
    root.child_main(name)
}

/// Index of `item` among the items of `name`, adding it when no equal one exists.
pub(crate) fn intern(root: &mut XmlElement, name: &str, item: XmlElement) -> usize {
    let at = section_index(root, name);
    let Some(sec) = root.children[at].as_element_mut() else {
        return 0;
    };
    let key = canon(&item);
    if let Some(i) = sec.elements().position(|e| canon(e) == key) {
        return i;
    }
    let n = sec.elements().count();
    sec.children.push(RawXml::Element(item));
    fix_count(sec);
    n
}

pub(crate) fn item(root: &XmlElement, name: &str, index: usize) -> Option<XmlElement> {
    section(root, name)?.elements().nth(index).cloned()
}

// ---- colours -------------------------------------------------------------------------------

/// Excel stores a tint as a whole number of 1/32767 steps, truncated toward zero.
fn quantise_tint(tint: f64) -> f64 {
    let steps = (tint.clamp(-1.0, 1.0) * 32767.0).trunc();
    steps / 32767.0
}

/// 17 significant digits as Excel writes them; below 0.1 in magnitude it switches to the
/// `E-2` form.
fn tint_text(tint: f64) -> String {
    let sci = format!("{tint:.16e}");
    let Some((mantissa, exp)) = sci.split_once('e') else {
        return sci;
    };
    let exp: i32 = exp.parse().unwrap_or(0);
    let negative = mantissa.starts_with('-');
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let sign = if negative { "-" } else { "" };
    if exp == -1 {
        format!("{sign}0.{digits}")
    } else {
        let (first, rest) = digits.split_at(1.min(digits.len()));
        format!("{sign}{first}.{rest}E{exp}")
    }
}

/// A `color`-like element (`color`, `fgColor`, `bgColor`) for `c`. `auto_index` is what an
/// automatic colour is written as, or `None` to write nothing.
fn color_element(
    tpl: &XmlElement,
    local: &str,
    c: &Color,
    auto_index: Option<u32>,
) -> Option<XmlElement> {
    let mut e = tpl.sibling(local);
    match c.source {
        ColorSource::Auto => {
            let i = auto_index?;
            e.head.attrs.push(Attr::plain("indexed", &i.to_string()));
            return Some(e);
        }
        ColorSource::Rgb(rgb) => e.head.attrs.push(Attr::plain(
            "rgb",
            &format!("FF{:02X}{:02X}{:02X}", rgb.r, rgb.g, rgb.b),
        )),
        ColorSource::Theme(t) => e.head.attrs.push(Attr::plain("theme", &t.to_string())),
        ColorSource::Indexed(i) => e.head.attrs.push(Attr::plain("indexed", &i.to_string())),
    }
    if c.tint != 0.0 {
        let q = quantise_tint(c.tint);
        if q != 0.0 {
            e.head.attrs.push(Attr::plain("tint", &tint_text(q)));
        }
    }
    Some(e)
}

// ---- fonts, fills, borders -----------------------------------------------------------------

fn underline_element(tpl: &XmlElement, u: Underline) -> Option<XmlElement> {
    let val = match u {
        Underline::None => return None,
        Underline::Single => None,
        Underline::Double => Some("double"),
        Underline::SingleAccounting => Some("singleAccounting"),
        Underline::DoubleAccounting => Some("doubleAccounting"),
    };
    let mut e = tpl.sibling("u");
    if let Some(v) = val {
        e.head.attrs.push(Attr::plain("val", v));
    }
    Some(e)
}

fn apply_font_patch(font: &mut XmlElement, p: &FontPatch) {
    let tpl = font.sibling("x");
    let flag = |font: &mut XmlElement, local: &str, on: bool| {
        if on {
            set_font_child(font, tpl.sibling(local));
        } else {
            remove_child(font, local);
        }
    };
    if let Some(b) = p.bold {
        flag(font, "b", b);
    }
    if let Some(i) = p.italic {
        flag(font, "i", i);
    }
    if let Some(s) = p.strike {
        flag(font, "strike", s);
    }
    if let Some(u) = p.underline {
        remove_child(font, "u");
        if let Some(e) = underline_element(&tpl, u) {
            set_font_child(font, e);
        }
    }
    if let Some(size) = p.size {
        set_font_child(font, el(&tpl, "sz", &[("val", &format!("{size}"))]));
    }
    if let Some(c) = &p.color {
        remove_child(font, "color");
        if let Some(e) = color_element(&tpl, "color", c, None) {
            set_font_child(font, e);
        }
    }
    if let Some(name) = &p.name {
        let same = font
            .child_main("name")
            .and_then(|n| n.head.attr("val"))
            .is_some_and(|v| v == name);
        if !same {
            set_font_child(font, el(&tpl, "name", &[("val", name)]));
            // A theme font name (`scheme`) no longer applies to a font picked by name.
            remove_child(font, "scheme");
            if let Some(family) = font_family(name) {
                set_font_child(font, el(&tpl, "family", &[("val", family)]));
            }
        }
    }
}

/// The `family` Excel records for common fonts: 1 roman, 2 swiss, 3 modern.
fn font_family(name: &str) -> Option<&'static str> {
    const ROMAN: [&str; 7] = [
        "times new roman",
        "cambria",
        "georgia",
        "garamond",
        "book antiqua",
        "palatino linotype",
        "century",
    ];
    const MODERN: [&str; 3] = ["courier new", "consolas", "lucida console"];
    let n = name.to_ascii_lowercase();
    if ROMAN.contains(&n.as_str()) {
        Some("1")
    } else if MODERN.contains(&n.as_str()) {
        Some("3")
    } else {
        Some("2")
    }
}

fn fill_element(tpl: &XmlElement, spec: &FillSpec) -> XmlElement {
    let mut fill = tpl.sibling("fill");
    let (pattern, fg, bg) = match spec {
        FillSpec::None => ("none", None, None),
        FillSpec::Solid(c) => ("solid", Some(*c), Some(Color::AUTO)),
        FillSpec::Pattern { pattern, fg, bg } => (pattern_name(*pattern), *fg, *bg),
    };
    let mut pf = el(tpl, "patternFill", &[("patternType", pattern)]);
    if let Some(c) = fg
        && let Some(e) = color_element(tpl, "fgColor", &c, Some(64))
    {
        pf.children.push(RawXml::Element(e));
    }
    if let Some(c) = bg
        && let Some(e) = color_element(tpl, "bgColor", &c, Some(64))
    {
        pf.children.push(RawXml::Element(e));
    }
    fill.children.push(RawXml::Element(pf));
    fill
}

fn pattern_name(p: FillPattern) -> &'static str {
    match p {
        FillPattern::None => "none",
        FillPattern::Solid => "solid",
        FillPattern::MediumGray => "mediumGray",
        FillPattern::DarkGray => "darkGray",
        FillPattern::LightGray => "lightGray",
        FillPattern::DarkHorizontal => "darkHorizontal",
        FillPattern::DarkVertical => "darkVertical",
        FillPattern::DarkDown => "darkDown",
        FillPattern::DarkUp => "darkUp",
        FillPattern::DarkGrid => "darkGrid",
        FillPattern::DarkTrellis => "darkTrellis",
        FillPattern::LightHorizontal => "lightHorizontal",
        FillPattern::LightVertical => "lightVertical",
        FillPattern::LightDown => "lightDown",
        FillPattern::LightUp => "lightUp",
        FillPattern::LightGrid => "lightGrid",
        FillPattern::LightTrellis => "lightTrellis",
        FillPattern::Gray125 => "gray125",
        FillPattern::Gray0625 => "gray0625",
    }
}

fn border_style_name(s: BorderStyle) -> Option<&'static str> {
    Some(match s {
        BorderStyle::None => return None,
        BorderStyle::Thin => "thin",
        BorderStyle::Medium => "medium",
        BorderStyle::Dashed => "dashed",
        BorderStyle::Dotted => "dotted",
        BorderStyle::Thick => "thick",
        BorderStyle::Double => "double",
        BorderStyle::Hair => "hair",
        BorderStyle::MediumDashed => "mediumDashed",
        BorderStyle::DashDot => "dashDot",
        BorderStyle::MediumDashDot => "mediumDashDot",
        BorderStyle::DashDotDot => "dashDotDot",
        BorderStyle::MediumDashDotDot => "mediumDashDotDot",
        BorderStyle::SlantDashDot => "slantDashDot",
    })
}

fn edge_element(tpl: &XmlElement, local: &str, edge: &BorderEdge) -> XmlElement {
    let Some(style) = border_style_name(edge.style) else {
        return tpl.sibling(local);
    };
    let mut e = el(tpl, local, &[("style", style)]);
    if let Some(c) = color_element(tpl, "color", &edge.color, Some(64)) {
        e.children.push(RawXml::Element(c));
    }
    e
}

fn empty_border(tpl: &XmlElement) -> XmlElement {
    let mut b = tpl.sibling("border");
    for side in ["left", "right", "top", "bottom", "diagonal"] {
        b.children.push(RawXml::Element(tpl.sibling(side)));
    }
    b
}

fn set_edge(border: &mut XmlElement, local: &str, edge: &BorderEdge) {
    let tpl = border.sibling("x");
    let new = edge_element(&tpl, local, edge);
    if let Some(slot) = child_mut(border, local) {
        *slot = new;
        return;
    }
    // Missing edge: keep the schema order left, right, top, bottom, diagonal.
    const ORDER: [&str; 7] = ["start", "left", "end", "right", "top", "bottom", "diagonal"];
    let rank = |n: &str| ORDER.iter().position(|o| *o == n).unwrap_or(ORDER.len());
    let at = border
        .children
        .iter()
        .position(|c| {
            c.as_element()
                .is_some_and(|c| rank(c.head.local_name()) > rank(local))
        })
        .unwrap_or(border.children.len());
    border.children.insert(at, RawXml::Element(new));
}

fn apply_border_patch(border: &mut XmlElement, p: &BorderPatch) {
    for (local, edge) in [
        ("left", p.left),
        ("right", p.right),
        ("top", p.top),
        ("bottom", p.bottom),
    ] {
        if let Some(e) = edge {
            set_edge(border, local, &e);
        }
    }
    let flag =
        |border: &XmlElement, name: &str| matches!(border.head.attr(name), Some("1" | "true"));
    let mut up = flag(border, "diagonalUp");
    let mut down = flag(border, "diagonalDown");
    let mut line: Option<BorderEdge> = None;
    if let Some(e) = p.diagonal_down {
        down = e.style != BorderStyle::None;
        line = Some(e);
    }
    if let Some(e) = p.diagonal_up {
        up = e.style != BorderStyle::None;
        // The shared line takes the style of whichever diagonal is still on.
        if line.is_none_or(|l| l.style == BorderStyle::None) || up {
            line = Some(e);
        }
    }
    if let Some(l) = line {
        let drawn = if up || down { l } else { BorderEdge::default() };
        set_edge(border, "diagonal", &drawn);
    }
    border.head.remove_attr("diagonalUp");
    border.head.remove_attr("diagonalDown");
    if up {
        border.head.attrs.push(Attr::plain("diagonalUp", "1"));
    }
    if down {
        border.head.attrs.push(Attr::plain("diagonalDown", "1"));
    }
}

// ---- alignment and protection ---------------------------------------------------------------

fn horizontal_name(h: HorizontalAlignment) -> Option<&'static str> {
    Some(match h {
        HorizontalAlignment::General => return None,
        HorizontalAlignment::Left => "left",
        HorizontalAlignment::Center => "center",
        HorizontalAlignment::Right => "right",
        HorizontalAlignment::Fill => "fill",
        HorizontalAlignment::Justify => "justify",
        HorizontalAlignment::CenterContinuous => "centerContinuous",
        HorizontalAlignment::Distributed => "distributed",
    })
}

fn vertical_name(v: VerticalAlignment) -> Option<&'static str> {
    Some(match v {
        VerticalAlignment::Bottom => return None,
        VerticalAlignment::Top => "top",
        VerticalAlignment::Center => "center",
        VerticalAlignment::Justify => "justify",
        VerticalAlignment::Distributed => "distributed",
    })
}

fn rotation_value(r: Rotation) -> u32 {
    match r {
        Rotation::Stacked => 255,
        Rotation::Degrees(d) => {
            let d = d.clamp(-90, 90);
            if d >= 0 { d as u32 } else { (90 - d) as u32 }
        }
    }
}

/// Order Excel writes `alignment` attributes in.
const ALIGN_ORDER: [&str; 9] = [
    "horizontal",
    "vertical",
    "textRotation",
    "wrapText",
    "indent",
    "relativeIndent",
    "justifyLastLine",
    "shrinkToFit",
    "readingOrder",
];

fn apply_alignment_patch(xf: &mut XmlElement, p: &AlignmentPatch) {
    let tpl = xf.sibling("x");
    let mut align = child_mut(xf, "alignment").map_or_else(
        || tpl.sibling("alignment"),
        |a| std::mem::replace(a, tpl.sibling("alignment")),
    );
    let set = |a: &mut XmlElement, name: &str, value: Option<String>| {
        a.head.remove_attr(name);
        if let Some(v) = value {
            a.head.attrs.push(Attr::plain(name, &v));
        }
    };
    if let Some(h) = p.horizontal {
        set(
            &mut align,
            "horizontal",
            horizontal_name(h).map(str::to_owned),
        );
    }
    if let Some(v) = p.vertical {
        set(&mut align, "vertical", vertical_name(v).map(str::to_owned));
    }
    if let Some(r) = p.rotation {
        let v = rotation_value(r);
        set(&mut align, "textRotation", (v != 0).then(|| v.to_string()));
    }
    if let Some(w) = p.wrap_text {
        set(&mut align, "wrapText", w.then(|| "1".to_owned()));
    }
    if let Some(i) = p.indent {
        set(&mut align, "indent", (i != 0).then(|| i.to_string()));
    }
    if let Some(s) = p.shrink_to_fit {
        set(&mut align, "shrinkToFit", s.then(|| "1".to_owned()));
    }
    // An indent only exists on left, right and distributed text; Excel turns anything else
    // into left aligned text.
    if p.indent.is_some_and(|i| i > 0) {
        let h = align.head.attr("horizontal");
        if !matches!(h, Some("left" | "right" | "distributed")) {
            set(&mut align, "horizontal", Some("left".to_owned()));
        }
    }
    align
        .head
        .attrs
        .sort_by_key(|a| ALIGN_ORDER.iter().position(|n| *n == a.name).unwrap_or(99));
    // Put the (possibly empty) alignment back as the first child, or drop it.
    remove_child(xf, "alignment");
    if !align.head.attrs.is_empty() {
        xf.children.insert(0, RawXml::Element(align));
    }
}

fn apply_protection_patch(xf: &mut XmlElement, p: &ProtectionPatch) {
    let tpl = xf.sibling("x");
    let mut prot = child_mut(xf, "protection").map_or_else(
        || tpl.sibling("protection"),
        |a| std::mem::replace(a, tpl.sibling("protection")),
    );
    if let Some(l) = p.locked {
        prot.head.remove_attr("locked");
        if !l {
            prot.head.attrs.insert(0, Attr::plain("locked", "0"));
        }
    }
    if let Some(h) = p.hidden {
        prot.head.remove_attr("hidden");
        if h {
            prot.head.attrs.push(Attr::plain("hidden", "1"));
        }
    }
    remove_child(xf, "protection");
    if !prot.head.attrs.is_empty() {
        xf.children.push(RawXml::Element(prot));
    }
}

pub(crate) fn set_apply(xf: &mut XmlElement, name: &str) {
    xf.head.set_attr(name, "1");
    // Keep the apply flags in the order Excel writes them, after the id attributes.
    let (mut apply, other): (Vec<Attr>, Vec<Attr>) = std::mem::take(&mut xf.head.attrs)
        .into_iter()
        .partition(|a| a.ns.is_none() && APPLY_ORDER.contains(&a.name.as_str()));
    apply.sort_by_key(|a| APPLY_ORDER.iter().position(|n| *n == a.name).unwrap_or(99));
    xf.head.attrs = other;
    xf.head.attrs.extend(apply);
}

// ---- number formats --------------------------------------------------------------------------

/// The built-in id whose code is `code`, as Excel resolves a typed format. Under a locale
/// (`Styles::set_number_format_locale`) the codes are that locale's, so en-GB `dd/mm/yyyy` is
/// id 14 and `m/d/yyyy` stays a custom format; without one, the specification's codes.
fn builtin_id_for(code: &str, locale: Option<u32>) -> Option<u32> {
    if code == "General" {
        return Some(0);
    }
    if let Some(lcid) = locale {
        return (1..=49).find(|&id| builtin_number_format_in(id, lcid).as_deref() == Some(code));
    }
    if code == "m/d/yyyy" {
        return Some(14);
    }
    (1..=49).find(|&id| builtin_number_format(id) == Some(code))
}

// ---- the edit API on Styles ------------------------------------------------------------------

impl Styles {
    pub(crate) fn root_mut(&mut self) -> Result<&mut XmlElement, XlsxError> {
        self.parsed
            .as_mut()
            .map(|(_, r, _)| r)
            .ok_or_else(|| XlsxError::UnsupportedEdit("the workbook has no styles part".to_owned()))
    }

    pub(crate) fn root(&self) -> Result<&XmlElement, XlsxError> {
        self.parsed
            .as_ref()
            .map(|(_, r, _)| r)
            .ok_or_else(|| XlsxError::UnsupportedEdit("the workbook has no styles part".to_owned()))
    }

    /// The `numFmtId` for `fmt`, adding a custom format when needed.
    fn number_format_id(root: &mut XmlElement, fmt: &NumberFormat, locale: Option<u32>) -> u32 {
        let code = match fmt {
            NumberFormat::Builtin(id) => return *id,
            NumberFormat::Code(code) => code,
        };
        if let Some(id) = builtin_id_for(code, locale) {
            return id;
        }
        let normalised = crate::styles_numfmt::normalise_code(code);
        let code = &normalised;
        let at = section_index(root, "numFmts");
        let Some(sec) = root.children[at].as_element_mut() else {
            return 0;
        };
        let mut next = 164;
        for f in sec.elements() {
            if f.head.attr("formatCode") == Some(code.as_str()) {
                return attr_u32(f, "numFmtId").unwrap_or(0);
            }
            next = next.max(attr_u32(f, "numFmtId").map_or(0, |i| i + 1));
        }
        let tpl = sec.sibling("x");
        sec.children.push(RawXml::Element(el(
            &tpl,
            "numFmt",
            &[("numFmtId", &next.to_string()), ("formatCode", code)],
        )));
        fix_count(sec);
        next
    }

    /// The cell format `base` with `patch` applied, as the index of an existing or new
    /// `cellXfs/xf`. Excel's rules: each group the patch touches is marked `apply*="1"` even
    /// when the value ends up equal to the default, and the xf is reused only when it is
    /// identical, flags included.
    pub(crate) fn derive_xf(&mut self, base: u32, patch: &FormatPatch) -> Result<u32, XlsxError> {
        let touches_font = patch.font != FontPatch::default();
        let touches_border = !patch.border.is_empty();
        let touches_align = patch.alignment != AlignmentPatch::default();
        let touches_prot = patch.protection != ProtectionPatch::default();
        let locale = self.num_fmt_locale;
        if !(touches_font
            || patch.fill.is_some()
            || touches_border
            || touches_align
            || patch.number_format.is_some()
            || touches_prot)
        {
            return Ok(base);
        }
        let root = self.root_mut()?;
        let mut xf = item(root, "cellXfs", base as usize)
            .or_else(|| item(root, "cellXfs", 0))
            .ok_or_else(|| {
                XlsxError::UnsupportedEdit("the style sheet has no cellXfs".to_owned())
            })?;
        if touches_font {
            let mut font = item(root, "fonts", attr_u32(&xf, "fontId").unwrap_or(0) as usize)
                .or_else(|| item(root, "fonts", 0))
                .ok_or_else(|| {
                    XlsxError::UnsupportedEdit("the style sheet has no fonts".to_owned())
                })?;
            apply_font_patch(&mut font, &patch.font);
            let id = intern(root, "fonts", font);
            xf.head.set_attr("fontId", &id.to_string());
            set_apply(&mut xf, "applyFont");
        }
        if let Some(fill) = &patch.fill {
            let tpl = xf.sibling("x");
            let id = intern(root, "fills", fill_element(&tpl, fill));
            xf.head.set_attr("fillId", &id.to_string());
            set_apply(&mut xf, "applyFill");
        }
        if touches_border {
            let mut border = item(
                root,
                "borders",
                attr_u32(&xf, "borderId").unwrap_or(0) as usize,
            )
            .unwrap_or_else(|| empty_border(&xf.sibling("x")));
            apply_border_patch(&mut border, &patch.border);
            let id = intern(root, "borders", border);
            xf.head.set_attr("borderId", &id.to_string());
            set_apply(&mut xf, "applyBorder");
        }
        if let Some(fmt) = &patch.number_format {
            let id = Self::number_format_id(root, fmt, locale);
            xf.head.set_attr("numFmtId", &id.to_string());
            set_apply(&mut xf, "applyNumberFormat");
        }
        if touches_align {
            apply_alignment_patch(&mut xf, &patch.alignment);
            set_apply(&mut xf, "applyAlignment");
        }
        if touches_prot {
            apply_protection_patch(&mut xf, &patch.protection);
            set_apply(&mut xf, "applyProtection");
        }
        let id = intern(root, "cellXfs", xf);
        self.reindex();
        Ok(id as u32)
    }

    /// The number of `cellXfs` entries, `fonts`, `fills`, `borders`: `(xfs, fonts, fills, borders)`.
    pub fn table_sizes(&self) -> (usize, usize, usize, usize) {
        let count = |name: &str| {
            self.root()
                .ok()
                .and_then(|r| section(r, name))
                .map_or(0, |s| s.elements().count())
        };
        (
            count("cellXfs"),
            count("fonts"),
            count("fills"),
            count("borders"),
        )
    }

    /// Drops every `cellXfs` entry nobody uses (index 0 stays), then the fonts, fills, borders
    /// and custom number formats no xf or cell style uses, as Excel does when it saves. Gives
    /// the new index of every old `cellXfs` index.
    pub(crate) fn compact(&mut self, used: &BTreeSet<u32>) -> Result<Vec<u32>, XlsxError> {
        let root = self.root_mut()?;
        let xfs: Vec<XmlElement> = section(root, "cellXfs")
            .map(|s| s.elements().cloned().collect())
            .unwrap_or_default();
        let mut remap = vec![0u32; xfs.len()];
        let mut kept: Vec<XmlElement> = Vec::new();
        for (i, xf) in xfs.into_iter().enumerate() {
            if i == 0 || used.contains(&(i as u32)) {
                remap[i] = kept.len() as u32;
                kept.push(xf);
            }
        }
        // Cell styles: Normal, those a kept xf is based on and those the file defines itself
        // stay; an unused built-in one goes, as it does when Excel saves.
        let all_style_xfs: Vec<XmlElement> = section(root, "cellStyleXfs")
            .map(|s| s.elements().cloned().collect())
            .unwrap_or_default();
        let cell_styles: Vec<XmlElement> = section(root, "cellStyles")
            .map(|s| s.elements().cloned().collect())
            .unwrap_or_default();
        let mut style_used: BTreeSet<u32> = BTreeSet::from([0]);
        for x in &kept {
            style_used.insert(attr_u32(x, "xfId").unwrap_or(0));
        }
        for cs in &cell_styles {
            let custom = cs.head.attr("builtinId").is_none()
                || matches!(cs.head.attr("customBuiltin"), Some("1" | "true"));
            if custom && let Some(id) = attr_u32(cs, "xfId") {
                style_used.insert(id);
            }
        }
        let mut style_map = vec![0u32; all_style_xfs.len()];
        let mut style_xfs: Vec<XmlElement> = Vec::new();
        for (i, x) in all_style_xfs.into_iter().enumerate() {
            if style_used.contains(&(i as u32)) {
                style_map[i] = style_xfs.len() as u32;
                style_xfs.push(x);
            }
        }
        let remap_style = |id: u32| style_map.get(id as usize).copied().unwrap_or(0);
        for x in &mut kept {
            if let Some(id) = attr_u32(x, "xfId") {
                x.head.set_attr("xfId", &remap_style(id).to_string());
            }
        }
        let cell_styles: Vec<XmlElement> = cell_styles
            .into_iter()
            .filter(|cs| attr_u32(cs, "xfId").is_some_and(|id| style_used.contains(&id)))
            .map(|mut cs| {
                if let Some(id) = attr_u32(&cs, "xfId") {
                    cs.head.set_attr("xfId", &remap_style(id).to_string());
                }
                cs
            })
            .collect();
        let referenced = |name: &str, keep: &[usize]| -> BTreeSet<usize> {
            let mut set: BTreeSet<usize> = keep.iter().copied().collect();
            for x in kept.iter().chain(style_xfs.iter()) {
                if let Some(i) = attr_u32(x, name) {
                    set.insert(i as usize);
                }
            }
            set
        };
        let font_ids = referenced("fontId", &[0]);
        let fill_ids = referenced("fillId", &[0, 1]);
        let border_ids = referenced("borderId", &[0]);
        let font_map = rebuild(root, "fonts", &font_ids);
        let fill_map = rebuild(root, "fills", &fill_ids);
        let border_map = rebuild(root, "borders", &border_ids);
        let remap_ids = |x: &mut XmlElement| {
            for (name, map) in [
                ("fontId", &font_map),
                ("fillId", &fill_map),
                ("borderId", &border_map),
            ] {
                if let Some(i) = attr_u32(x, name)
                    && let Some(n) = map.get(i as usize)
                {
                    x.head.set_attr(name, &n.to_string());
                }
            }
        };
        for x in &mut kept {
            remap_ids(x);
        }

        for x in &mut style_xfs {
            remap_ids(x);
        }
        // Custom number formats nobody references.
        let mut num_used: BTreeSet<u32> = BTreeSet::new();
        for x in kept.iter().chain(style_xfs.iter()) {
            if let Some(i) = attr_u32(x, "numFmtId") {
                num_used.insert(i);
            }
        }
        if let Some(dxfs) = section(root, "dxfs") {
            for d in dxfs.elements() {
                if let Some(nf) = d.child_main("numFmt")
                    && let Some(i) = attr_u32(nf, "numFmtId")
                {
                    num_used.insert(i);
                }
            }
        }
        if let Some(i) = root
            .children
            .iter()
            .position(|c| c.as_element().is_some_and(|c| c.head.is_main("numFmts")))
        {
            let mut drop_section = false;
            if let Some(sec) = root.children[i].as_element_mut() {
                sec.children.retain(|c| {
                    c.as_element().is_none_or(|f| {
                        attr_u32(f, "numFmtId").is_none_or(|id| num_used.contains(&id))
                    })
                });
                fix_count(sec);
                drop_section = sec.elements().next().is_none();
            }
            if drop_section {
                root.children.remove(i);
            }
        }
        replace_items(root, "cellXfs", kept);
        replace_items(root, "cellStyleXfs", style_xfs);
        replace_items(root, "cellStyles", cell_styles);
        self.reindex();
        Ok(remap)
    }
}

/// Keeps the items of `name` whose index is in `keep`; gives the new index of each old one.
fn rebuild(root: &mut XmlElement, name: &str, keep: &BTreeSet<usize>) -> Vec<usize> {
    let Some(sec) = root
        .children
        .iter_mut()
        .filter_map(RawXml::as_element_mut)
        .find(|e| e.head.is_main(name))
    else {
        return Vec::new();
    };
    let mut map = Vec::new();
    let mut next = 0;
    let mut index = 0;
    sec.children.retain(|c| {
        if c.as_element().is_none() {
            return true;
        }
        let keep_it = keep.contains(&index);
        map.push(next);
        if keep_it {
            next += 1;
        }
        index += 1;
        keep_it
    });
    fix_count(sec);
    map
}

fn replace_items(root: &mut XmlElement, name: &str, items: Vec<XmlElement>) {
    if let Some(sec) = root
        .children
        .iter_mut()
        .filter_map(RawXml::as_element_mut)
        .find(|e| e.head.is_main(name))
    {
        sec.children = items.into_iter().map(RawXml::Element).collect();
        fix_count(sec);
    }
}
