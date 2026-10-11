//! Paragraph properties (`w:pPr`, ECMA-376 Part 1 §17.3.1): ordered insertion of the few
//! children the editor writes. Everything else in `w:pPr` stays untouched.

use serde::{Deserialize, Serialize};

use crate::document::Document;
use crate::error::DocxError;
use crate::layout_props::TabKind;
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

/// Alignment of a custom tab stop (`w:tab` `w:val`, ST_TabJc) that the editor can add. `clear`
/// is written by [`Document::clear_tab`] and has no variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabAlign {
    #[default]
    Left,
    Center,
    Right,
    Decimal,
    Bar,
}

impl TabAlign {
    /// The transitional `w:val` for this kind.
    pub fn as_val(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
            Self::Decimal => "decimal",
            Self::Bar => "bar",
        }
    }
}

/// Points to whole twips (1 pt = 20 twips); rejects NaN, infinities and absurd lengths.
fn to_twips(pt: f64) -> Result<i32, DocxError> {
    let twips = (pt * 20.0).round();
    if twips.is_finite() && twips.abs() <= f64::from(i32::MAX / 2) {
        #[allow(clippy::cast_possible_truncation)]
        Ok(twips as i32)
    } else {
        Err(DocxError::UnsupportedRange(format!(
            "{pt} pt is not a usable paragraph length"
        )))
    }
}

fn tab_pos(tab: &RawXml) -> Option<i32> {
    match tab {
        RawXml::Element(e) if e.head.is_w("tab") => e.head.attr_w("pos")?.trim().parse().ok(),
        _ => None,
    }
}

impl Paragraph {
    /// Writes `w:ind` with the three lengths in twips: `first_line` > 0 is `w:firstLine`, < 0 is
    /// `w:hanging`, 0 is an explicit `w:firstLine="0"` so a style's indent is overridden.
    /// Character-based attributes (`w:leftChars` ...) would win over these, so they are dropped.
    pub(crate) fn set_indents(&mut self, left: i32, first_line: i32, right: i32) {
        upsert_child(self.props_mut(), "ind", |e| {
            for stale in [
                "start",
                "end",
                "startChars",
                "endChars",
                "leftChars",
                "rightChars",
                "firstLineChars",
                "hangingChars",
                "firstLine",
                "hanging",
            ] {
                e.head.remove_attr_w(stale);
            }
            e.head.set_attr_w("left", &left.to_string());
            e.head.set_attr_w("right", &right.to_string());
            if first_line < 0 {
                e.head.set_attr_w("hanging", &(-first_line).to_string());
            } else {
                e.head.set_attr_w("firstLine", &first_line.to_string());
            }
        });
    }

    /// Sets a tab stop at `pos` twips with `val`, replacing any direct stop (or `clear`) at that
    /// position. Stops stay sorted by position.
    fn put_tab(&mut self, pos: i32, val: &str) {
        upsert_child(self.props_mut(), "tabs", |tabs| {
            tabs.children.retain(|c| tab_pos(c) != Some(pos));
            let mut tab = tabs.sibling("tab");
            tab.head.set_attr_w("val", val);
            tab.head.set_attr_w("pos", &pos.to_string());
            let at = tabs
                .children
                .iter()
                .position(|c| tab_pos(c).is_some_and(|p| p > pos))
                .unwrap_or(tabs.children.len());
            tabs.children.insert(at, RawXml::Element(tab));
        });
    }

    /// Removes the direct stop (or `clear`) at `pos`; an emptied `w:tabs` goes too.
    fn remove_tab(&mut self, pos: i32) {
        let Some(props) = &mut self.props else {
            return;
        };
        let mut empty = false;
        if let Some(RawXml::Element(tabs)) = props
            .children
            .iter_mut()
            .find(|c| matches!(c, RawXml::Element(e) if e.head.is_w("tabs")))
        {
            tabs.children.retain(|c| tab_pos(c) != Some(pos));
            empty = !tabs.children.iter().any(|c| tab_pos(c).is_some());
        }
        if empty {
            remove_child(props, "tabs");
        }
    }
}

impl Document {
    /// Ruler edit: left, first-line (negative: hanging) and right indent of paragraph `para`,
    /// in points, written as `w:ind`.
    ///
    /// # Errors
    /// `para` is out of range, or a length is not finite.
    pub fn set_indents(
        &mut self,
        para: usize,
        left_pt: f64,
        first_line_pt: f64,
        right_pt: f64,
    ) -> Result<(), DocxError> {
        let (left, first, right) = (
            to_twips(left_pt)?,
            to_twips(first_line_pt)?,
            to_twips(right_pt)?,
        );
        self.paragraph_mut(para)?.set_indents(left, first, right);
        self.edited();
        Ok(())
    }

    /// Adds (or retypes) the custom tab stop at `pos_pt` points from the left margin.
    ///
    /// # Errors
    /// `para` is out of range or the position is not finite.
    pub fn add_tab(&mut self, para: usize, pos_pt: f64, align: TabAlign) -> Result<(), DocxError> {
        let pos = to_twips(pos_pt)?;
        self.paragraph_mut(para)?.put_tab(pos, align.as_val());
        self.edited();
        Ok(())
    }

    /// Moves the stop at `from_pt` to `to_pt`, keeping its alignment. A stop inherited from the
    /// style is cleared at `from_pt` and set at `to_pt`.
    ///
    /// # Errors
    /// `para` is out of range, a position is not finite, or there is no stop at `from_pt`.
    pub fn move_tab(&mut self, para: usize, from_pt: f64, to_pt: f64) -> Result<(), DocxError> {
        let (from, to) = (to_twips(from_pt)?, to_twips(to_pt)?);
        let align = self.effective_tab_align(para, from)?;
        if from == to {
            return Ok(());
        }
        self.clear_tab_twips(para, from)?;
        self.paragraph_mut(para)?.put_tab(to, align.as_val());
        self.edited();
        Ok(())
    }

    /// Clears the stop at `pos_pt`: a direct stop is removed, one inherited from the style gets a
    /// `w:val="clear"` entry (as Word writes it).
    ///
    /// # Errors
    /// `para` is out of range, the position is not finite, or there is no stop there.
    pub fn clear_tab(&mut self, para: usize, pos_pt: f64) -> Result<(), DocxError> {
        let pos = to_twips(pos_pt)?;
        self.effective_tab_align(para, pos)?;
        self.clear_tab_twips(para, pos)?;
        self.edited();
        Ok(())
    }

    fn clear_tab_twips(&mut self, para: usize, pos: i32) -> Result<(), DocxError> {
        self.paragraph_mut(para)?.remove_tab(pos);
        if self.inherits_tab(para, pos) {
            self.paragraph_mut(para)?.put_tab(pos, "clear");
        }
        Ok(())
    }

    /// True when the effective stops (after the direct ones are taken out) still hold `pos`.
    fn inherits_tab(&self, para: usize, pos: i32) -> bool {
        self.paragraph(para)
            .zip(self.styles())
            .is_some_and(|(p, s)| {
                s.paragraph_flow_props(p)
                    .tabs
                    .iter()
                    .any(|t| t.pos_twips == pos)
            })
    }

    /// Alignment of the effective stop at `pos` twips.
    fn effective_tab_align(&self, para: usize, pos: i32) -> Result<TabAlign, DocxError> {
        let count = self.paragraph_count();
        let p = self
            .paragraph(para)
            .ok_or(DocxError::ParagraphOutOfRange { index: para, count })?;
        let tabs = self
            .styles()
            .map(|s| s.paragraph_flow_props(p).tabs)
            .unwrap_or_default();
        let stop = tabs.iter().find(|t| t.pos_twips == pos).ok_or_else(|| {
            DocxError::UnsupportedRange(format!("no tab stop at {} pt", f64::from(pos) / 20.0))
        })?;
        Ok(match stop.kind {
            TabKind::Left | TabKind::Num => TabAlign::Left,
            TabKind::Center => TabAlign::Center,
            TabKind::Right => TabAlign::Right,
            TabKind::Decimal => TabAlign::Decimal,
            TabKind::Bar => TabAlign::Bar,
        })
    }
}
