//! Run properties (`w:rPr`, ECMA-376 Part 1 §17.3.2): a typed view over the common
//! properties that keeps every child, known or not, in its original position.

use crate::xml::{ElementHead, RawXml, XmlElement, is_w_ns};

/// Properties with typed accessors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunPropKind {
    Bold,
    Italic,
    Underline,
    Strike,
    Size,
    Color,
    Fonts,
    VertAlign,
    Highlight,
}

impl RunPropKind {
    fn local(self) -> &'static str {
        match self {
            Self::Bold => "b",
            Self::Italic => "i",
            Self::Underline => "u",
            Self::Strike => "strike",
            Self::Size => "sz",
            Self::Color => "color",
            Self::Fonts => "rFonts",
            Self::VertAlign => "vertAlign",
            Self::Highlight => "highlight",
        }
    }

    fn from_local(local: &str) -> Option<Self> {
        Some(match local {
            "b" => Self::Bold,
            "i" => Self::Italic,
            "u" => Self::Underline,
            "strike" => Self::Strike,
            "sz" => Self::Size,
            "color" => Self::Color,
            "rFonts" => Self::Fonts,
            "vertAlign" => Self::VertAlign,
            "highlight" => Self::Highlight,
            _ => return None,
        })
    }
}

/// One child of `w:rPr`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunPropItem {
    /// A property with a typed accessor; the element keeps all its attributes.
    Known(RunPropKind, XmlElement),
    /// Anything else (other properties, `rPrChange`, extension elements, comments, text).
    Raw(RawXml),
}

/// Position of a child in the schema sequence for `CT_RPr` / `CT_ParaRPr`. Used only to
/// place newly added properties; existing children are never reordered.
fn rank(item: &RunPropItem) -> u32 {
    let element = match item {
        RunPropItem::Known(_, e) => e,
        RunPropItem::Raw(RawXml::Element(e)) => e,
        RunPropItem::Raw(_) => return 0,
    };
    if !is_w_ns(element.head.ns.as_deref()) {
        return 500;
    }
    rank_of_local(element.head.local_name())
}

fn rank_of_local(local: &str) -> u32 {
    const ORDER: &[&str] = &[
        "rStyle",
        "rFonts",
        "b",
        "bCs",
        "i",
        "iCs",
        "caps",
        "smallCaps",
        "strike",
        "dstrike",
        "outline",
        "shadow",
        "emboss",
        "imprint",
        "noProof",
        "snapToGrid",
        "vanish",
        "webHidden",
        "color",
        "spacing",
        "w",
        "kern",
        "position",
        "sz",
        "szCs",
        "highlight",
        "u",
        "effect",
        "bdr",
        "shd",
        "fitText",
        "vertAlign",
        "rtl",
        "cs",
        "em",
        "lang",
        "eastAsianLayout",
        "specVanish",
        "oMath",
    ];
    match local {
        "ins" | "del" | "moveFrom" | "moveTo" => 0,
        "rPrChange" => 1000,
        other => ORDER
            .iter()
            .position(|n| *n == other)
            .map_or(500, |p| u32::try_from(p).unwrap_or(0) + 1),
    }
}

/// ST_OnOff: absent value means true.
fn on_off(value: Option<&str>) -> bool {
    !matches!(value, Some("0" | "false" | "off"))
}

/// The `w:rPr` of a run. `head` is `None` when the run has no `w:rPr` element.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunProps {
    pub(crate) head: Option<ElementHead>,
    pub(crate) items: Vec<RunPropItem>,
}

impl RunProps {
    pub(crate) fn from_element(element: XmlElement) -> Self {
        let items = element
            .children
            .into_iter()
            .map(|child| match child {
                RawXml::Element(e) => match classify(&e) {
                    Some(kind) => RunPropItem::Known(kind, e),
                    None => RunPropItem::Raw(RawXml::Element(e)),
                },
                other => RunPropItem::Raw(other),
            })
            .collect();
        Self {
            head: Some(element.head),
            items,
        }
    }

    /// True when the run carried a `w:rPr` element (possibly empty).
    pub fn is_present(&self) -> bool {
        self.head.is_some()
    }

    pub fn items(&self) -> &[RunPropItem] {
        &self.items
    }

    /// Children that have no typed accessor, in order.
    pub fn extras(&self) -> impl Iterator<Item = &RawXml> {
        self.items.iter().filter_map(|i| match i {
            RunPropItem::Raw(r) => Some(r),
            RunPropItem::Known(..) => None,
        })
    }

    fn find(&self, kind: RunPropKind) -> Option<&XmlElement> {
        self.items.iter().find_map(|i| match i {
            RunPropItem::Known(k, e) if *k == kind => Some(e),
            _ => None,
        })
    }

    fn toggle(&self, kind: RunPropKind) -> Option<bool> {
        self.find(kind).map(|e| on_off(e.head.attr_w("val")))
    }

    /// `Some(true)` for `<w:b/>`, `Some(false)` for `<w:b w:val="0"/>`, `None` when unset.
    pub fn bold(&self) -> Option<bool> {
        self.toggle(RunPropKind::Bold)
    }

    pub fn italic(&self) -> Option<bool> {
        self.toggle(RunPropKind::Italic)
    }

    pub fn strike(&self) -> Option<bool> {
        self.toggle(RunPropKind::Strike)
    }

    /// Underline style (`single`, `double`, ...); `single` when `w:val` is omitted.
    pub fn underline(&self) -> Option<&str> {
        self.find(RunPropKind::Underline)
            .map(|e| e.head.attr_w("val").unwrap_or("single"))
    }

    /// Font size in half-points.
    pub fn size_half_points(&self) -> Option<u32> {
        self.find(RunPropKind::Size)
            .and_then(|e| e.head.attr_w("val"))
            .and_then(|v| v.parse().ok())
    }

    /// Colour as written: hex `RRGGBB` or `auto`.
    pub fn color(&self) -> Option<&str> {
        self.find(RunPropKind::Color)
            .and_then(|e| e.head.attr_w("val"))
    }

    /// The `w:rFonts` element, with all its font slots as attributes.
    pub fn fonts(&self) -> Option<&XmlElement> {
        self.find(RunPropKind::Fonts)
    }

    /// `superscript`, `subscript` or `baseline`.
    pub fn vert_align(&self) -> Option<&str> {
        self.find(RunPropKind::VertAlign)
            .and_then(|e| e.head.attr_w("val"))
    }

    pub fn highlight(&self) -> Option<&str> {
        self.find(RunPropKind::Highlight)
            .and_then(|e| e.head.attr_w("val"))
    }

    /// Sets or clears (`None`) bold. `true` writes `<w:b/>`, `false` writes `w:val="0"`.
    pub fn set_bold(&mut self, value: Option<bool>, owner: &ElementHead) {
        self.set_toggle(RunPropKind::Bold, value, owner);
    }

    pub fn set_italic(&mut self, value: Option<bool>, owner: &ElementHead) {
        self.set_toggle(RunPropKind::Italic, value, owner);
    }

    pub fn set_strike(&mut self, value: Option<bool>, owner: &ElementHead) {
        self.set_toggle(RunPropKind::Strike, value, owner);
    }

    /// `owner` is the run's head, used to name a `w:rPr` or child created on demand.
    fn set_toggle(&mut self, kind: RunPropKind, value: Option<bool>, owner: &ElementHead) {
        let existing = self
            .items
            .iter()
            .position(|i| matches!(i, RunPropItem::Known(k, _) if *k == kind));
        let Some(value) = value else {
            if let Some(index) = existing {
                self.items.remove(index);
            }
            return;
        };
        if let Some(index) = existing
            && let RunPropItem::Known(_, element) = &mut self.items[index]
        {
            if value {
                element.head.remove_attr_w("val");
            } else {
                element.head.set_attr_w("val", "0");
            }
            return;
        }
        let head = self.head.get_or_insert_with(|| owner.sibling("rPr"));
        let mut element = XmlElement {
            head: head.sibling(kind.local()),
            children: Vec::new(),
        };
        if !value {
            element.head.set_attr_w("val", "0");
        }
        let new_rank = rank_of_local(kind.local());
        let index = self
            .items
            .iter()
            .position(|i| rank(i) > new_rank)
            .unwrap_or(self.items.len());
        self.items.insert(index, RunPropItem::Known(kind, element));
    }

    pub(crate) fn write(&self, out: &mut String) {
        let Some(head) = &self.head else { return };
        if self.items.is_empty() {
            crate::xml::write_head(head, true, out);
            return;
        }
        crate::xml::write_head(head, false, out);
        for item in &self.items {
            match item {
                RunPropItem::Known(_, e) => e.write(out),
                RunPropItem::Raw(r) => r.write(out),
            }
        }
        crate::xml::write_end(head, out);
    }
}

fn classify(element: &XmlElement) -> Option<RunPropKind> {
    if !is_w_ns(element.head.ns.as_deref()) {
        return None;
    }
    RunPropKind::from_local(element.head.local_name())
}
