//! Numbering definitions (ECMA-376 Part 1 §17.9): `word/numbering.xml` as typed abstract
//! numberings, numbering instances and levels, plus the resolution of a paragraph's
//! `w:numPr` through the style chain.
//!
//! Counting (which number a paragraph shows) is left to the consumer; this module only
//! reads what the document states.

use std::collections::BTreeMap;

use crate::model::Paragraph;
use crate::ppr;
use crate::styles::Styles;
use crate::xml::{self, RawXml, XmlElement};
use crate::xmlq::{child, children, int, on_off, twips_attr, val};

/// Number of list levels (`w:ilvl` 0..=8).
pub const LEVELS: usize = 9;

/// What follows the label (`w:suff`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelSuffix {
    Tab,
    Space,
    Nothing,
}

/// Label alignment around the number position (`w:lvlJc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelJustification {
    Left,
    Center,
    Right,
}

/// Indents in points that a numbering level (or a paragraph) states; `None` means "not
/// stated".
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct IndentOverride {
    pub left: Option<f64>,
    pub right: Option<f64>,
    /// First-line offset: positive for `w:firstLine`, negative for `w:hanging`.
    pub first_line: Option<f64>,
}

impl IndentOverride {
    pub(crate) fn from_ind(e: &XmlElement) -> Self {
        let pt = |names: &[&str]| names.iter().find_map(|n| twips_attr(e, n));
        let hanging = pt(&["hanging"]);
        let first = pt(&["firstLine"]);
        Self {
            left: pt(&["left", "start"]),
            right: pt(&["right", "end"]),
            first_line: hanging.map(|h| -h).or(first),
        }
    }

    /// `self` wins where it states a value, `lower` fills the rest.
    pub fn over(self, lower: Self) -> Self {
        Self {
            left: self.left.or(lower.left),
            right: self.right.or(lower.right),
            first_line: self.first_line.or(lower.first_line),
        }
    }
}

/// Character formatting a level states for its label (`w:lvl/w:rPr`); `None` keeps the
/// paragraph mark's value.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LevelRun {
    pub font: Option<String>,
    pub size_half_points: Option<u32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub color: Option<[u8; 3]>,
}

fn parse_hex_color(v: &str) -> Option<[u8; 3]> {
    if v.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(v, 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

impl LevelRun {
    fn from_rpr(rpr: &XmlElement) -> Self {
        let mut out = Self::default();
        for e in rpr.children.iter().filter_map(RawXml::as_element) {
            match e.head.local_name() {
                "rFonts" if e.head.is_w("rFonts") => {
                    out.font = e
                        .head
                        .attr_w("ascii")
                        .or_else(|| e.head.attr_w("hAnsi"))
                        .map(str::to_owned);
                }
                "sz" if e.head.is_w("sz") => {
                    out.size_half_points =
                        int(e.head.attr_w("val")).and_then(|v| v.try_into().ok());
                }
                "b" if e.head.is_w("b") => out.bold = Some(on_off(e)),
                "i" if e.head.is_w("i") => out.italic = Some(on_off(e)),
                "u" if e.head.is_w("u") => {
                    out.underline = Some(e.head.attr_w("val") != Some("none"));
                }
                "strike" if e.head.is_w("strike") => out.strike = Some(on_off(e)),
                "color" if e.head.is_w("color") => {
                    out.color = e.head.attr_w("val").and_then(parse_hex_color);
                }
                _ => {}
            }
        }
        out
    }
}

/// One list level (`w:lvl`).
#[derive(Debug, Clone, PartialEq)]
pub struct NumLevel {
    /// `w:start`, default 1.
    pub start: i64,
    /// `w:numFmt` value, default `decimal`.
    pub num_fmt: String,
    /// `w:lvlText`.
    pub text: String,
    pub suffix: LevelSuffix,
    pub justification: LevelJustification,
    /// `w:isLgl`: higher levels in the label show as decimal.
    pub is_legal: bool,
    /// `w:lvlRestart`: `None` restarts after any higher level, `Some(0)` never, `Some(k)`
    /// only after levels above level `k`.
    pub restart: Option<usize>,
    /// `w:pStyle`: the paragraph style linked to this level.
    pub paragraph_style: Option<String>,
    /// `w:pPr/w:ind`.
    pub indent: IndentOverride,
    /// `w:rPr` of the label.
    pub run: LevelRun,
}

impl NumLevel {
    fn from_lvl(lvl: &XmlElement) -> Self {
        Self {
            start: int(val(lvl, "start")).unwrap_or(1),
            num_fmt: val(lvl, "numFmt").unwrap_or("decimal").to_owned(),
            text: val(lvl, "lvlText").unwrap_or_default().to_owned(),
            suffix: match val(lvl, "suff") {
                Some("space") => LevelSuffix::Space,
                Some("nothing") => LevelSuffix::Nothing,
                _ => LevelSuffix::Tab,
            },
            justification: match val(lvl, "lvlJc") {
                Some("right" | "end") => LevelJustification::Right,
                Some("center") => LevelJustification::Center,
                _ => LevelJustification::Left,
            },
            is_legal: child(lvl, "isLgl").is_some_and(on_off),
            restart: int(val(lvl, "lvlRestart")).and_then(|v| usize::try_from(v).ok()),
            paragraph_style: val(lvl, "pStyle").map(str::to_owned),
            indent: child(lvl, "pPr")
                .and_then(|p| child(p, "ind"))
                .map(IndentOverride::from_ind)
                .unwrap_or_default(),
            run: child(lvl, "rPr")
                .map(LevelRun::from_rpr)
                .unwrap_or_default(),
        }
    }

    /// True for a bullet level.
    pub fn is_bullet(&self) -> bool {
        self.num_fmt == "bullet"
    }
}

/// `w:abstractNum`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AbstractNum {
    pub id: i64,
    /// Always [`LEVELS`] entries; `None` for a level the definition does not declare.
    pub levels: Vec<Option<NumLevel>>,
    /// `w:numStyleLink`: this list only points at a numbering style.
    pub num_style_link: Option<String>,
    /// `w:styleLink`: the numbering style this list defines.
    pub style_link: Option<String>,
}

/// `w:lvlOverride`.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelOverride {
    pub ilvl: usize,
    /// `w:startOverride`.
    pub start_override: Option<i64>,
    /// A replacement `w:lvl`.
    pub level: Option<NumLevel>,
}

/// `w:num`: a numbering instance.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Num {
    pub id: i64,
    pub abstract_id: i64,
    pub overrides: Vec<LevelOverride>,
}

impl Num {
    /// `w:startOverride` for `ilvl`.
    pub fn start_override(&self, ilvl: usize) -> Option<i64> {
        self.overrides
            .iter()
            .find(|o| o.ilvl == ilvl)
            .and_then(|o| o.start_override)
    }

    /// Replacement level for `ilvl`.
    pub fn level_override(&self, ilvl: usize) -> Option<&NumLevel> {
        self.overrides
            .iter()
            .find(|o| o.ilvl == ilvl)
            .and_then(|o| o.level.as_ref())
    }
}

/// `w:numPr` as written in one place; either part may be missing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NumPr {
    pub num_id: Option<i64>,
    pub ilvl: Option<usize>,
}

impl NumPr {
    pub(crate) fn from_element(num_pr: &XmlElement) -> Self {
        Self {
            num_id: int(val(num_pr, "numId")),
            ilvl: int(val(num_pr, "ilvl")).and_then(|i| usize::try_from(i).ok()),
        }
    }
}

/// The list a paragraph belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParagraphNumbering {
    pub num_id: i64,
    pub ilvl: usize,
}

/// A numbering instance with its abstract numbering resolved through `w:numStyleLink`.
#[derive(Debug, Clone, Copy)]
pub struct ResolvedNum<'a> {
    numbering: &'a Numbering,
    pub num: &'a Num,
    /// Id of the abstract numbering that carries the levels.
    pub abstract_id: i64,
}

impl<'a> ResolvedNum<'a> {
    /// The level `ilvl` of the instance: its own override, else the abstract definition.
    pub fn level(&self, ilvl: usize) -> Option<&'a NumLevel> {
        self.num.level_override(ilvl).or_else(|| {
            self.numbering
                .abstract_num(self.abstract_id)?
                .levels
                .get(ilvl)?
                .as_ref()
        })
    }
}

/// Numbering definitions of a document.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Numbering {
    abstracts: BTreeMap<i64, AbstractNum>,
    nums: BTreeMap<i64, Num>,
}

impl Numbering {
    /// Parses a numbering part. `None` when it is not XML.
    pub(crate) fn parse(bytes: &[u8]) -> Option<Self> {
        let parsed = xml::parse(bytes).ok()?;
        let mut out = Self::default();
        for e in parsed.root.children.iter().filter_map(RawXml::as_element) {
            if e.head.is_w("abstractNum") {
                let Some(id) = int(e.head.attr_w("abstractNumId")) else {
                    continue;
                };
                let mut levels: Vec<Option<NumLevel>> = vec![None; LEVELS];
                for lvl in children(e, "lvl") {
                    if let Some(i) = int(lvl.head.attr_w("ilvl"))
                        .and_then(|i| usize::try_from(i).ok())
                        .filter(|i| *i < LEVELS)
                    {
                        levels[i] = Some(NumLevel::from_lvl(lvl));
                    }
                }
                out.abstracts.insert(
                    id,
                    AbstractNum {
                        id,
                        levels,
                        num_style_link: val(e, "numStyleLink").map(str::to_owned),
                        style_link: val(e, "styleLink").map(str::to_owned),
                    },
                );
            } else if e.head.is_w("num") {
                let Some(id) = int(e.head.attr_w("numId")) else {
                    continue;
                };
                let overrides = children(e, "lvlOverride")
                    .filter_map(|o| {
                        let ilvl = int(o.head.attr_w("ilvl"))
                            .and_then(|i| usize::try_from(i).ok())
                            .filter(|i| *i < LEVELS)?;
                        Some(LevelOverride {
                            ilvl,
                            start_override: int(val(o, "startOverride")),
                            level: child(o, "lvl").map(NumLevel::from_lvl),
                        })
                    })
                    .collect();
                out.nums.insert(
                    id,
                    Num {
                        id,
                        abstract_id: int(val(e, "abstractNumId")).unwrap_or(-1),
                        overrides,
                    },
                );
            }
        }
        Some(out)
    }

    pub fn abstract_num(&self, id: i64) -> Option<&AbstractNum> {
        self.abstracts.get(&id)
    }

    pub fn num(&self, id: i64) -> Option<&Num> {
        self.nums.get(&id)
    }

    /// All abstract numberings by id.
    pub fn abstract_nums(&self) -> impl Iterator<Item = &AbstractNum> {
        self.abstracts.values()
    }

    /// All numbering instances by id.
    pub fn nums(&self) -> impl Iterator<Item = &Num> {
        self.nums.values()
    }

    /// Follows `w:numStyleLink`: a list that only points at a numbering style uses the
    /// abstract numbering of the instance that style names in its own `w:numPr`.
    fn resolve_abstract(&self, styles: Option<&Styles>, mut id: i64) -> Option<i64> {
        for _ in 0..8 {
            let a = self.abstracts.get(&id)?;
            let Some(link) = a.num_style_link.as_deref() else {
                return Some(id);
            };
            let num_id = styles?.style_num_pr(link)?.num_id;
            let next = self.nums.get(&num_id)?.abstract_id;
            if next == id {
                return Some(id);
            }
            id = next;
        }
        None
    }

    /// The numbering instance `num_id` with its abstract numbering resolved.
    pub fn resolve(&self, styles: Option<&Styles>, num_id: i64) -> Option<ResolvedNum<'_>> {
        let num = self.nums.get(&num_id)?;
        let abstract_id = self.resolve_abstract(styles, num.abstract_id)?;
        Some(ResolvedNum {
            numbering: self,
            num,
            abstract_id,
        })
    }

    /// Level whose `w:pStyle` names `style`, for a style that links to a list without a level.
    fn level_for_style(&self, styles: Option<&Styles>, num_id: i64, style: &str) -> Option<usize> {
        let resolved = self.resolve(styles, num_id)?;
        self.abstracts
            .get(&resolved.abstract_id)?
            .levels
            .iter()
            .position(|l| {
                l.as_ref()
                    .is_some_and(|l| l.paragraph_style.as_deref() == Some(style))
            })
    }

    /// The numbering instance and level `para` belongs to: its own `w:numPr`, else the
    /// one its paragraph style (or a style it is based on) assigns. `None` for a paragraph
    /// that is not numbered (including `numId` 0).
    pub fn paragraph_numbering(
        &self,
        styles: Option<&Styles>,
        para: &Paragraph,
    ) -> Option<ParagraphNumbering> {
        let direct = para.direct_num_pr();
        let from_style = styles.and_then(|s| {
            let id = para.style_id().or_else(|| s.default_paragraph_style())?;
            s.style_num_pr(id)
        });
        let (num_id, ilvl) = match (direct, from_style) {
            (
                Some(NumPr {
                    num_id: Some(n),
                    ilvl,
                }),
                _,
            ) => (n, ilvl),
            // Direct numPr without numId keeps the style's list at the stated level.
            (Some(NumPr { num_id: None, ilvl }), Some(s)) => (s.num_id, ilvl),
            (Some(NumPr { num_id: None, .. }), None) | (None, None) => return None,
            (None, Some(s)) => (
                s.num_id,
                s.ilvl
                    .or_else(|| self.level_for_style(styles, s.num_id, &s.holder)),
            ),
        };
        if num_id <= 0 {
            return None;
        }
        Some(ParagraphNumbering {
            num_id,
            ilvl: ilvl.unwrap_or(0).min(LEVELS - 1),
        })
    }
}

/// `numId` and level a paragraph style (or one it is based on) assigns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleNumPr {
    pub num_id: i64,
    pub ilvl: Option<usize>,
    /// The style in the chain that holds the `w:numPr`.
    pub holder: String,
}

impl Paragraph {
    /// The paragraph's own `w:pPr/w:numPr`.
    pub fn direct_num_pr(&self) -> Option<NumPr> {
        let props = self.props.as_ref()?;
        ppr::child(props, "numPr").map(NumPr::from_element)
    }

    /// The paragraph's own `w:pPr/w:ind`, in points.
    pub fn direct_indent(&self) -> IndentOverride {
        self.props
            .as_ref()
            .and_then(|p| ppr::child(p, "ind"))
            .map(IndentOverride::from_ind)
            .unwrap_or_default()
    }
}
