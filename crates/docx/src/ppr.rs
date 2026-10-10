//! Paragraph properties (`w:pPr`, ECMA-376 Part 1 §17.3.1): ordered insertion of the few
//! children the editor writes. Everything else in `w:pPr` stays untouched.

use serde::{Deserialize, Serialize};

use crate::model::Paragraph;
use crate::xml::{ElementHead, RawXml, XmlElement, is_w_ns};

/// Paragraph alignment (`w:jc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

impl Alignment {
    /// The transitional `w:val` for this alignment.
    pub fn as_val(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
            Self::Justify => "both",
        }
    }

    /// Reads a `w:jc` value (transitional and strict spellings); unknown values give `None`.
    pub fn from_val(value: &str) -> Option<Self> {
        Some(match value {
            "left" | "start" => Self::Left,
            "center" => Self::Center,
            "right" | "end" => Self::Right,
            "both" | "justify" | "distribute" | "thaiDistribute" | "lowKashida"
            | "mediumKashida" | "highKashida" => Self::Justify,
            _ => return None,
        })
    }
}

/// `CT_PPrBase` followed by the `CT_PPr` additions, in schema order.
const ORDER: &[&str] = &[
    "pStyle",
    "keepNext",
    "keepLines",
    "pageBreakBefore",
    "framePr",
    "widowControl",
    "numPr",
    "suppressLineNumbers",
    "pBdr",
    "shd",
    "tabs",
    "suppressAutoHyphens",
    "kinsoku",
    "wordWrap",
    "overflowPunct",
    "topLinePunct",
    "autoSpaceDE",
    "autoSpaceDN",
    "bidi",
    "adjustRightInd",
    "snapToGrid",
    "spacing",
    "ind",
    "contextualSpacing",
    "mirrorIndents",
    "suppressOverlap",
    "jc",
    "textDirection",
    "textAlignment",
    "textboxTightWrap",
    "outlineLvl",
    "divId",
    "cnfStyle",
    "rPr",
    "sectPr",
    "pPrChange",
];

fn rank(child: &RawXml) -> usize {
    match child {
        RawXml::Element(e) if is_w_ns(e.head.ns.as_deref()) => ORDER
            .iter()
            .position(|n| *n == e.head.local_name())
            .unwrap_or(ORDER.len()),
        RawXml::Element(_) => ORDER.len(),
        _ => 0,
    }
}

/// The child `w:local` of a `w:pPr`.
pub(crate) fn child<'a>(props: &'a XmlElement, local: &str) -> Option<&'a XmlElement> {
    props.children.iter().find_map(|c| match c {
        RawXml::Element(e) if e.head.is_w(local) => Some(e),
        _ => None,
    })
}

/// Removes the child `w:local`; true when something was removed.
pub(crate) fn remove_child(props: &mut XmlElement, local: &str) -> bool {
    let before = props.children.len();
    props
        .children
        .retain(|c| !matches!(c, RawXml::Element(e) if e.head.is_w(local)));
    props.children.len() != before
}

/// Finds or creates `w:local` in schema position and edits it.
pub(crate) fn upsert_child(
    props: &mut XmlElement,
    local: &str,
    edit: impl FnOnce(&mut XmlElement),
) {
    if let Some(RawXml::Element(e)) = props
        .children
        .iter_mut()
        .find(|c| matches!(c, RawXml::Element(e) if e.head.is_w(local)))
    {
        edit(e);
        return;
    }
    let mut element = props.sibling(local);
    edit(&mut element);
    let new_rank = ORDER
        .iter()
        .position(|n| *n == local)
        .unwrap_or(ORDER.len());
    let at = props
        .children
        .iter()
        .position(|c| rank(c) > new_rank)
        .unwrap_or(props.children.len());
    props.children.insert(at, RawXml::Element(element));
}

impl Paragraph {
    /// The `w:pPr`, created (empty) when the paragraph has none.
    pub(crate) fn props_mut(&mut self) -> &mut XmlElement {
        let head: ElementHead = self.head.sibling("pPr");
        self.props.get_or_insert_with(|| XmlElement {
            head,
            children: Vec::new(),
        })
    }

    /// Style id from `w:pStyle`, `None` when the paragraph uses the default style.
    pub fn style_id(&self) -> Option<&str> {
        let props = self.props.as_ref()?;
        child(props, "pStyle")?.head.attr_w("val")
    }

    /// Alignment set directly on the paragraph (not inherited from its style).
    pub fn direct_alignment(&self) -> Option<Alignment> {
        let props = self.props.as_ref()?;
        Alignment::from_val(child(props, "jc")?.head.attr_w("val")?)
    }

    /// Sets (`Some`) or removes (`None`) `w:pStyle`.
    pub(crate) fn set_style_id(&mut self, style: Option<&str>) {
        match style {
            Some(id) => upsert_child(self.props_mut(), "pStyle", |e| {
                e.head.set_attr_w("val", id);
            }),
            None => {
                if let Some(props) = &mut self.props {
                    remove_child(props, "pStyle");
                }
            }
        }
    }

    pub(crate) fn set_alignment(&mut self, alignment: Alignment) {
        upsert_child(self.props_mut(), "jc", |e| {
            e.head.set_attr_w("val", alignment.as_val());
        });
    }
}
