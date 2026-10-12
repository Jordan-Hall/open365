//! Writer Layout tab operations: page setup (margins, orientation, size, columns), breaks,
//! line numbers, hyphenation, paragraph indent and spacing. Each runs as one undo step.
//!
//! Sections are numbered as in [`Document::sections`]. Lengths are points; the page presets
//! the ribbon offers are in [`margin_presets`], [`paper_sizes`] and [`column_presets`].

use open365_docx::{
    ColumnSpec, DocPos, Document, LineNumberRestart, Orientation, SectionType, TextBreak,
};
use serde::{Deserialize, Serialize};

use crate::EffectClass;
use crate::error::CommandError;

/// How a section starts (Layout > Breaks > Section Breaks).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionStart {
    NextPage,
    Continuous,
    EvenPage,
    OddPage,
}

impl SectionStart {
    fn kind(self) -> SectionType {
        match self {
            Self::NextPage => SectionType::NextPage,
            Self::Continuous => SectionType::Continuous,
            Self::EvenPage => SectionType::EvenPage,
            Self::OddPage => SectionType::OddPage,
        }
    }
}

/// The breaks of Layout > Breaks > Page Breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BreakKind {
    Page,
    Column,
    TextWrapping,
}

/// When the line numbers restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Restart {
    Continuous,
    EachPage,
    EachSection,
}

/// Line numbering of a section (Layout > Line Numbers > Line Numbering Options).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LineNumbering {
    pub count_by: u32,
    pub start_at: u32,
    /// Distance from the text in points; `None` is Auto.
    pub from_text_pt: Option<f64>,
    pub restart: Restart,
}

impl Default for LineNumbering {
    fn default() -> Self {
        Self {
            count_by: 1,
            start_at: 1,
            from_text_pt: None,
            restart: Restart::Continuous,
        }
    }
}

/// What a [`crate::Command::DocxLayout`] does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum LayoutOp {
    /// Margins (points) of the sections; `gutter_pt` keeps the gutter when `None`.
    Margins {
        sections: Vec<usize>,
        top_pt: f64,
        bottom_pt: f64,
        left_pt: f64,
        right_pt: f64,
        gutter_pt: Option<f64>,
        /// Mirror margins (left and right become inside and outside); `None` leaves the setting.
        mirrored: Option<bool>,
    },
    Orientation {
        sections: Vec<usize>,
        landscape: bool,
    },
    /// Paper size in points; `code` is the printer paper code (none for custom sizes).
    PageSize {
        sections: Vec<usize>,
        width_pt: f64,
        height_pt: f64,
        code: Option<u32>,
    },
    /// Equal columns, or `widths_pt` (width, space after) for unequal ones.
    Columns {
        sections: Vec<usize>,
        count: u32,
        space_pt: f64,
        separator: Option<bool>,
        widths_pt: Option<Vec<(f64, f64)>>,
    },
    /// Line numbers of the sections; `None` turns them off.
    LineNumbers {
        sections: Vec<usize>,
        numbering: Option<LineNumbering>,
    },
    /// Suppress Line Numbers for paragraphs.
    SuppressLineNumbers { paras: Vec<usize>, on: bool },
    /// Page, column and text wrapping breaks at `pos`.
    Break { pos: DocPos, kind: BreakKind },
    /// Next Page, Continuous, Even Page and Odd Page section breaks at `pos`.
    SectionBreak { pos: DocPos, start: SectionStart },
    /// Automatic hyphenation and its options (zone and limit).
    Hyphenation {
        auto: bool,
        zone_pt: Option<f64>,
        limit: Option<u32>,
        hyphenate_caps: Option<bool>,
    },
    /// Indent Left and Right of paragraphs, points; `None` leaves a side.
    Indent {
        paras: Vec<usize>,
        left_pt: Option<f64>,
        right_pt: Option<f64>,
    },
    /// Spacing Before and After paragraphs, points; `None` leaves a side.
    Spacing {
        paras: Vec<usize>,
        before_pt: Option<f64>,
        after_pt: Option<f64>,
    },
}

impl LayoutOp {
    pub fn effect(&self) -> EffectClass {
        EffectClass::Edit
    }
}

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::Format(msg.into())
}

/// Points to twips, rejecting values that are not finite or are beyond 22 inches.
fn twips(pt: f64) -> Result<i32, CommandError> {
    if !pt.is_finite() || !(-1584.0..=1584.0).contains(&pt) {
        return Err(bad(format!("{pt} pt is not a usable length")));
    }
    Ok((pt * 20.0).round() as i32)
}

fn each(
    sections: &[usize],
    mut f: impl FnMut(usize) -> Result<(), CommandError>,
) -> Result<(), CommandError> {
    if sections.is_empty() {
        return Err(bad("no section to change"));
    }
    sections.iter().try_for_each(|s| f(*s))
}

/// Runs `op` on `doc`.
pub fn apply(doc: &mut Document, op: &LayoutOp) -> Result<(), CommandError> {
    match op {
        LayoutOp::Margins {
            sections,
            top_pt,
            bottom_pt,
            left_pt,
            right_pt,
            gutter_pt,
            mirrored,
        } => {
            let gutter = gutter_pt.map(twips).transpose()?;
            let (t, b, l, r) = (
                twips(*top_pt)?,
                twips(*bottom_pt)?,
                twips(*left_pt)?,
                twips(*right_pt)?,
            );
            each(sections, |s| {
                Ok(doc.set_page_margins(s, t, b, l, r, gutter)?)
            })?;
            if let Some(m) = mirrored {
                doc.set_mirror_margins(*m)?;
            }
        }
        LayoutOp::Orientation {
            sections,
            landscape,
        } => {
            let o = if *landscape {
                Orientation::Landscape
            } else {
                Orientation::Portrait
            };
            each(sections, |s| Ok(doc.set_orientation(s, o)?))?;
        }
        LayoutOp::PageSize {
            sections,
            width_pt,
            height_pt,
            code,
        } => {
            let (w, h) = (twips(*width_pt)?, twips(*height_pt)?);
            each(sections, |s| {
                Ok(doc.set_page_size(s, i64::from(w), i64::from(h), *code)?)
            })?;
        }
        LayoutOp::Columns {
            sections,
            count,
            space_pt,
            separator,
            widths_pt,
        } => {
            let space = u32::try_from(twips(*space_pt)?.max(0)).unwrap_or(0);
            let widths: Option<Vec<ColumnSpec>> = match widths_pt {
                Some(list) => Some(
                    list.iter()
                        .map(|(w, s)| {
                            Ok(ColumnSpec {
                                width_twips: u32::try_from(twips(*w)?.max(0)).unwrap_or(0),
                                space_twips: u32::try_from(twips(*s)?.max(0)).unwrap_or(0),
                            })
                        })
                        .collect::<Result<_, CommandError>>()?,
                ),
                None => None,
            };
            each(sections, |s| {
                Ok(doc.set_columns(s, *count, space, *separator, widths.as_deref())?)
            })?;
        }
        LayoutOp::LineNumbers {
            sections,
            numbering,
        } => {
            let spec = match numbering {
                Some(n) => Some((
                    n.count_by.clamp(1, 100),
                    n.start_at,
                    n.from_text_pt
                        .map(|d| twips(d).map(|t| u32::try_from(t.max(0)).unwrap_or(0)))
                        .transpose()?,
                    match n.restart {
                        Restart::Continuous => LineNumberRestart::Continuous,
                        Restart::EachPage => LineNumberRestart::NewPage,
                        Restart::EachSection => LineNumberRestart::NewSection,
                    },
                )),
                None => None,
            };
            each(sections, |s| Ok(doc.set_line_numbering(s, spec)?))?;
        }
        LayoutOp::SuppressLineNumbers { paras, on } => {
            for p in paras {
                doc.set_suppress_line_numbers(*p, *on)?;
            }
        }
        LayoutOp::Break { pos, kind } => {
            let kind = match kind {
                BreakKind::Page => TextBreak::Page,
                BreakKind::Column => TextBreak::Column,
                BreakKind::TextWrapping => TextBreak::TextWrapping,
            };
            doc.insert_break(*pos, kind)?;
        }
        LayoutOp::SectionBreak { pos, start } => {
            doc.insert_section_break(*pos, start.kind())?;
        }
        LayoutOp::Hyphenation {
            auto,
            zone_pt,
            limit,
            hyphenate_caps,
        } => {
            let zone = zone_pt
                .map(|z| twips(z).map(|t| u32::try_from(t.max(0)).unwrap_or(0)))
                .transpose()?;
            doc.set_hyphenation(*auto, zone, *limit, *hyphenate_caps)?;
        }
        LayoutOp::Indent {
            paras,
            left_pt,
            right_pt,
        } => {
            let (l, r) = (
                left_pt.map(twips).transpose()?,
                right_pt.map(twips).transpose()?,
            );
            for p in paras {
                doc.set_para_indent(*p, l, r)?;
            }
        }
        LayoutOp::Spacing {
            paras,
            before_pt,
            after_pt,
        } => {
            let (b, a) = (
                before_pt.map(twips).transpose()?,
                after_pt.map(twips).transpose()?,
            );
            for p in paras {
                doc.set_para_spacing(*p, b, a)?;
            }
        }
    }
    Ok(())
}

/// Margins in points: top, bottom, left, right, and whether they mirror.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarginPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub top: f64,
    pub bottom: f64,
    pub left: f64,
    pub right: f64,
    pub mirrored: bool,
}

/// The Margins gallery: Normal, Narrow, Moderate, Wide and Mirrored. Inside and outside of
/// Mirrored are stored as left and right.
#[must_use]
pub fn margin_presets() -> &'static [MarginPreset] {
    &MARGINS
}

const MARGINS: [MarginPreset; 5] = {
    const fn p(
        id: &'static str,
        label: &'static str,
        t: f64,
        b: f64,
        l: f64,
        r: f64,
        m: bool,
    ) -> MarginPreset {
        MarginPreset {
            id,
            label,
            top: t,
            bottom: b,
            left: l,
            right: r,
            mirrored: m,
        }
    }
    [
        p("normal", "Normal", 72.0, 72.0, 72.0, 72.0, false),
        p("narrow", "Narrow", 36.0, 36.0, 36.0, 36.0, false),
        p("moderate", "Moderate", 72.0, 72.0, 54.0, 54.0, false),
        p("wide", "Wide", 72.0, 72.0, 144.0, 144.0, false),
        p("mirrored", "Mirrored", 72.0, 72.0, 90.0, 72.0, true),
    ]
};

/// A paper size: points and the printer paper code Word writes (`w:code`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaperSize {
    pub id: &'static str,
    pub label: &'static str,
    pub width: f64,
    pub height: f64,
    pub code: u32,
}

/// The Size gallery.
#[must_use]
pub fn paper_sizes() -> &'static [PaperSize] {
    &SIZES
}

const SIZES: [PaperSize; 15] = {
    const fn p(id: &'static str, label: &'static str, w: f64, h: f64, code: u32) -> PaperSize {
        PaperSize {
            id,
            label,
            width: w,
            height: h,
            code,
        }
    }
    [
        p("letter", "Letter", 612.0, 792.0, 1),
        p("tabloid", "Tabloid", 792.0, 1224.0, 3),
        p("legal", "Legal", 612.0, 1008.0, 5),
        p("statement", "Statement", 396.0, 612.0, 6),
        p("executive", "Executive", 522.0, 756.0, 7),
        p("a3", "A3", 841.9, 1190.55, 8),
        p("a4", "A4", 595.3, 841.9, 9),
        p("a5", "A5", 419.55, 595.3, 11),
        p("b4", "B4 (JIS)", 728.5, 1031.8, 12),
        p("b5", "B5 (JIS)", 515.9, 728.5, 13),
        p("envelope-10", "Envelope #10", 297.0, 684.0, 20),
        p("envelope-dl", "Envelope DL", 311.8, 623.6, 27),
        p("envelope-c5", "Envelope C5", 459.25, 649.1, 28),
        p("envelope-b5", "Envelope B5", 498.9, 708.65, 34),
        p("envelope-monarch", "Envelope Monarch", 279.0, 540.0, 37),
    ]
};

/// The columns of the Columns gallery for a text width of `text_width_pt`: the number, the
/// space and the widths (`None` for equal columns).
#[must_use]
#[allow(clippy::type_complexity)]
pub fn column_preset(id: &str, text_width_pt: f64) -> Option<(u32, f64, Option<Vec<(f64, f64)>>)> {
    let space = 36.0;
    let usable = (text_width_pt - space).max(1.0);
    match id {
        "one" => Some((1, space, None)),
        "two" => Some((2, space, None)),
        "three" => Some((3, space, None)),
        // Left: a narrow first column (one third), Right: a narrow second column.
        "left" => Some((
            2,
            space,
            Some(vec![(usable / 3.0, space), (usable * 2.0 / 3.0, 0.0)]),
        )),
        "right" => Some((
            2,
            space,
            Some(vec![(usable * 2.0 / 3.0, space), (usable / 3.0, 0.0)]),
        )),
        _ => None,
    }
}
