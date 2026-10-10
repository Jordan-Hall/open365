//! Run properties (`w:rPr`, ECMA-376 Part 1 §17.3.2): a typed view over the common
//! properties that keeps every child, known or not, in its original position.

use serde::{Deserialize, Serialize};

use crate::xml::{ElementHead, RawXml, XmlElement, is_w_ns};

/// A set of run-property changes. `None` leaves a property alone; `Some` sets it, so
/// `bold: Some(false)` writes an explicit `w:val="0"` that overrides the style.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunPropsPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    /// `true` is a single underline, `false` writes `w:u w:val="none"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strike: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_half_points: Option<u32>,
    /// `RRGGBB` hex or `auto`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Font family name, applied to the ASCII, high-ANSI, complex-script and East Asian slots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
}

impl RunPropsPatch {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn bold(value: bool) -> Self {
        Self {
            bold: Some(value),
            ..Self::default()
        }
    }
}

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
    Style,
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
            Self::Style => "rStyle",
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
            "rStyle" => Self::Style,
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

    /// Character style id (`w:rStyle`).
    pub fn style_id(&self) -> Option<&str> {
        self.find(RunPropKind::Style)
            .and_then(|e| e.head.attr_w("val"))
    }

    /// Removes the property `kind`, if present.
    fn remove(&mut self, kind: RunPropKind) {
        self.items
            .retain(|i| !matches!(i, RunPropItem::Known(k, _) if *k == kind));
    }

    /// Finds or creates the element for `kind` (placed in schema order) and edits it.
    /// `owner` is the run's head, used to name a `w:rPr` or child created on demand.
    fn upsert(
        &mut self,
        kind: RunPropKind,
        owner: &ElementHead,
        edit: impl FnOnce(&mut XmlElement),
    ) {
        if let Some(RunPropItem::Known(_, element)) = self
            .items
            .iter_mut()
            .find(|i| matches!(i, RunPropItem::Known(k, _) if *k == kind))
        {
            edit(element);
            return;
        }
        let head = self.head.get_or_insert_with(|| owner.sibling("rPr"));
        let mut element = XmlElement {
            head: head.sibling(kind.local()),
            children: Vec::new(),
        };
        edit(&mut element);
        let new_rank = rank_of_local(kind.local());
        let index = self
            .items
            .iter()
            .position(|i| rank(i) > new_rank)
            .unwrap_or(self.items.len());
        self.items.insert(index, RunPropItem::Known(kind, element));
    }

    fn set_toggle(&mut self, kind: RunPropKind, value: Option<bool>, owner: &ElementHead) {
        let Some(value) = value else {
            self.remove(kind);
            return;
        };
        self.upsert(kind, owner, |element| {
            if value {
                element.head.remove_attr_w("val");
            } else {
                element.head.set_attr_w("val", "0");
            }
        });
    }

    /// Sets the underline: `true` keeps an existing style or writes `single`, `false`
    /// writes `none`.
    pub fn set_underline(&mut self, value: bool, owner: &ElementHead) {
        self.upsert(RunPropKind::Underline, owner, |element| {
            let current = element.head.attr_w("val").map(str::to_owned);
            let new = match (value, current.as_deref()) {
                (true, Some(style)) if style != "none" => style,
                (true, _) => "single",
                (false, _) => "none",
            };
            element.head.set_attr_w("val", new);
        });
    }

    pub fn set_size_half_points(&mut self, half_points: u32, owner: &ElementHead) {
        self.upsert(RunPropKind::Size, owner, |element| {
            element.head.set_attr_w("val", &half_points.to_string());
        });
    }

    /// `RRGGBB` or `auto`; theme colour attributes are dropped because they would win.
    pub fn set_color(&mut self, color: &str, owner: &ElementHead) {
        self.upsert(RunPropKind::Color, owner, |element| {
            element.head.set_attr_w("val", color);
            for theme in ["themeColor", "themeTint", "themeShade"] {
                element.head.remove_attr_w(theme);
            }
        });
    }

    /// Sets the font name in every slot and drops theme font references.
    pub fn set_font(&mut self, name: &str, owner: &ElementHead) {
        self.upsert(RunPropKind::Fonts, owner, |element| {
            for slot in ["ascii", "hAnsi", "cs", "eastAsia"] {
                element.head.set_attr_w(slot, name);
            }
            for theme in ["asciiTheme", "hAnsiTheme", "cstheme", "eastAsiaTheme"] {
                element.head.remove_attr_w(theme);
            }
        });
    }

    /// Applies every property set in `patch`.
    pub fn apply(&mut self, patch: &RunPropsPatch, owner: &ElementHead) {
        if let Some(v) = patch.bold {
            self.set_bold(Some(v), owner);
        }
        if let Some(v) = patch.italic {
            self.set_italic(Some(v), owner);
        }
        if let Some(v) = patch.underline {
            self.set_underline(v, owner);
        }
        if let Some(v) = patch.strike {
            self.set_strike(Some(v), owner);
        }
        if let Some(v) = patch.size_half_points {
            self.set_size_half_points(v, owner);
        }
        if let Some(v) = &patch.color {
            self.set_color(v, owner);
        }
        if let Some(v) = &patch.font {
            self.set_font(v, owner);
        }
    }

    /// The `w:rPr` element for these properties, `None` when the run has none.
    pub(crate) fn to_element(&self) -> Option<XmlElement> {
        let head = self.head.clone()?;
        let children = self
            .items
            .iter()
            .map(|item| match item {
                RunPropItem::Known(_, e) => RawXml::Element(e.clone()),
                RunPropItem::Raw(r) => r.clone(),
            })
            .collect();
        Some(XmlElement { head, children })
    }

    /// A copy without revision-tracking children (`w:ins`, `w:del`, `w:rPrChange`...), which
    /// describe one specific change and must not be copied to other text.
    pub(crate) fn without_tracking(&self) -> Self {
        let mut copy = self.clone();
        copy.items.retain(|item| match item {
            RunPropItem::Raw(RawXml::Element(e)) if is_w_ns(e.head.ns.as_deref()) => !matches!(
                e.head.local_name(),
                "ins" | "del" | "moveFrom" | "moveTo" | "rPrChange"
            ),
            _ => true,
        });
        copy
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
