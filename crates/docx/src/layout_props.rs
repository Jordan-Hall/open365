//! Typed, style-resolved paragraph and run properties that page layout needs
//! (ECMA-376 Part 1 §17.3). Resolution order is the one [`crate::Styles`] uses:
//! document defaults, the style chain (`w:basedOn`), then direct formatting.

use crate::props::RunProps;
use crate::styles::EffectiveRun;
use crate::xml::{RawXml, XmlElement};

/// Alignment kind of a tab stop (`w:tab` `w:val`, ECMA-376 Part 1 §17.3.1.37, ST_TabJc).
/// `clear` is not a kind: it removes an inherited stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabKind {
    Left,
    Center,
    Right,
    Decimal,
    /// Draws a vertical rule; never moves text.
    Bar,
    /// List-number tab.
    Num,
}

/// Leader drawn in the gap a tab leaves (`w:tab` `w:leader`, ST_TabTlc). Default `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabLeader {
    None,
    Dot,
    Hyphen,
    Underscore,
    Heavy,
    MiddleDot,
}

/// A custom tab stop. `pos_twips` is measured from the paragraph's leading text edge and
/// may be negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabStop {
    pub pos_twips: i32,
    pub kind: TabKind,
    pub leader: TabLeader,
}

impl TabStop {
    pub fn pos_pt(&self) -> f32 {
        self.pos_twips as f32 / 20.0
    }
}

/// One `w:tab` child of `w:tabs`, before inheritance is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TabOp {
    Set(TabStop),
    Clear(i32),
}

pub(crate) fn tab_ops(tabs: &XmlElement) -> Vec<TabOp> {
    let mut out = Vec::new();
    for t in tabs.children.iter().filter_map(RawXml::as_element) {
        if !t.head.is_w("tab") {
            continue;
        }
        let Some(pos) = t
            .head
            .attr_w("pos")
            .and_then(|v| v.trim().parse::<i32>().ok())
        else {
            continue;
        };
        let kind = match t.head.attr_w("val").unwrap_or("left") {
            "clear" => {
                out.push(TabOp::Clear(pos));
                continue;
            }
            "center" => TabKind::Center,
            "right" | "end" => TabKind::Right,
            "decimal" => TabKind::Decimal,
            "bar" => TabKind::Bar,
            "num" => TabKind::Num,
            _ => TabKind::Left,
        };
        let leader = match t.head.attr_w("leader") {
            Some("dot") => TabLeader::Dot,
            Some("hyphen") => TabLeader::Hyphen,
            Some("underscore") => TabLeader::Underscore,
            Some("heavy") => TabLeader::Heavy,
            Some("middleDot") => TabLeader::MiddleDot,
            _ => TabLeader::None,
        };
        out.push(TabOp::Set(TabStop {
            pos_twips: pos,
            kind,
            leader,
        }));
    }
    out
}

/// Applies inherited-then-direct tab operations in order: a `set` replaces a stop at the
/// same position, a `clear` removes it. The result is sorted by position.
pub(crate) fn resolve_tabs(ops: &[TabOp]) -> Vec<TabStop> {
    let mut tabs: Vec<TabStop> = Vec::new();
    for op in ops {
        match *op {
            TabOp::Set(stop) => {
                tabs.retain(|t| t.pos_twips != stop.pos_twips);
                tabs.push(stop);
            }
            TabOp::Clear(pos) => tabs.retain(|t| t.pos_twips != pos),
        }
    }
    tabs.sort_by_key(|t| t.pos_twips);
    tabs
}

/// Paragraph properties for pagination and tabbing, after style resolution.
///
/// Absent-property defaults follow ECMA-376 Part 1: `widowControl` is on (§17.3.1.44),
/// every other flag is off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParagraphProps {
    /// `w:widowControl`, §17.3.1.44. Default true.
    pub widow_control: bool,
    /// `w:keepNext`, §17.3.1.14. Default false.
    pub keep_next: bool,
    /// `w:keepLines`, §17.3.1.15. Default false.
    pub keep_lines: bool,
    /// `w:pageBreakBefore`, §17.3.1.23. Default false.
    pub page_break_before: bool,
    /// `w:contextualSpacing`, §17.3.1.9. Default false.
    pub contextual_spacing: bool,
    /// `w:suppressAutoHyphens`, §17.3.1.34. Default false.
    pub suppress_auto_hyphens: bool,
    /// Effective custom tab stops (§17.3.1.38), sorted by position, `clear` applied.
    pub tabs: Vec<TabStop>,
}

impl Default for ParagraphProps {
    fn default() -> Self {
        Self {
            widow_control: true,
            keep_next: false,
            keep_lines: false,
            page_break_before: false,
            contextual_spacing: false,
            suppress_auto_hyphens: false,
            tabs: Vec::new(),
        }
    }
}

/// OpenType ligature sets requested by `w14:ligatures` (MS-DOCX §2.6.2.4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ligatures {
    pub standard: bool,
    pub contextual: bool,
    pub historical: bool,
    pub discretionary: bool,
}

impl Ligatures {
    pub(crate) fn parse(value: &str) -> Self {
        match value {
            "all" => Self {
                standard: true,
                contextual: true,
                historical: true,
                discretionary: true,
            },
            other => Self {
                standard: other.contains("standard"),
                contextual: other.contains("ontextual"),
                historical: other.contains("istorical"),
                discretionary: other.contains("iscretionary"),
            },
        }
    }

    pub fn is_none(&self) -> bool {
        *self == Self::default()
    }
}

/// Character-level typography properties, after style resolution.
///
/// Absent-property defaults: no kerning, no ligatures, baseline position, no extra
/// spacing, 100 percent scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunLayoutProps {
    /// `w:kern`, §17.3.2.19: smallest font size (half-points) that is kerned; 0 is off.
    pub kern_half_points: u32,
    /// `w14:ligatures`; default none.
    pub ligatures: Ligatures,
    /// `w:position`, §17.3.2.24: raise (positive) or lower (negative), half-points.
    pub position_half_points: i32,
    /// `w:spacing`, §17.3.2.35: extra character spacing in twips (may be negative).
    pub spacing_twips: i32,
    /// `w:w`, §17.3.2.43: horizontal scale in percent (1..=600).
    pub scale_percent: u32,
}

impl Default for RunLayoutProps {
    fn default() -> Self {
        Self {
            kern_half_points: 0,
            ligatures: Ligatures::default(),
            position_half_points: 0,
            spacing_twips: 0,
            scale_percent: 100,
        }
    }
}

impl RunLayoutProps {
    pub fn position_pt(&self) -> f32 {
        self.position_half_points as f32 / 2.0
    }

    pub fn spacing_pt(&self) -> f32 {
        self.spacing_twips as f32 / 20.0
    }
}

/// The paragraph mark's own run properties (`w:pPr/w:rPr`, §17.3.1.29) after style
/// resolution: document defaults, paragraph style run properties, then the mark's direct
/// formatting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParagraphMark {
    pub run: EffectiveRun,
    pub layout: RunLayoutProps,
}

/// Run properties written in one place; `None` means "not set here".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LayoutFmt {
    pub kern: Option<u32>,
    pub ligatures: Option<Ligatures>,
    pub position: Option<i32>,
    pub spacing: Option<i32>,
    pub scale: Option<u32>,
}

impl LayoutFmt {
    pub(crate) fn from_props(props: &RunProps) -> Self {
        let int = |local: &str| {
            props
                .any_w_element(local)
                .and_then(|e| e.head.attr_w("val"))
                .and_then(|v| v.trim().parse::<i32>().ok())
        };
        Self {
            kern: int("kern").and_then(|v| u32::try_from(v).ok()),
            ligatures: props.ligatures_value().map(Ligatures::parse),
            position: int("position"),
            spacing: int("spacing"),
            scale: int("w").and_then(|v| u32::try_from(v).ok()),
        }
    }

    pub(crate) fn overlay(&mut self, other: &Self) {
        fn put<T: Copy>(slot: &mut Option<T>, value: Option<T>) {
            if value.is_some() {
                *slot = value;
            }
        }
        put(&mut self.kern, other.kern);
        put(&mut self.ligatures, other.ligatures);
        put(&mut self.position, other.position);
        put(&mut self.spacing, other.spacing);
        put(&mut self.scale, other.scale);
    }

    pub(crate) fn finish(self) -> RunLayoutProps {
        let d = RunLayoutProps::default();
        RunLayoutProps {
            kern_half_points: self.kern.unwrap_or(d.kern_half_points),
            ligatures: self.ligatures.unwrap_or(d.ligatures),
            position_half_points: self.position.unwrap_or(d.position_half_points),
            spacing_twips: self.spacing.unwrap_or(d.spacing_twips),
            scale_percent: self.scale.unwrap_or(d.scale_percent),
        }
    }
}
