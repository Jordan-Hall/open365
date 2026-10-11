//! Conditional formatting, read-only: `<conditionalFormatting>` blocks from a worksheet and
//! the `<dxf>` differential formats of the style sheet they point at.
//!
//! Nothing here writes. Both parts stay raw trees in the workbook model (ADR-001), so an
//! unedited sheet writes back byte for byte; this module only reads typed views of them.
//! Evaluating the rules against cell values is the commands crate's job (it owns the formula
//! engine); the model supplies what each rule says.

use crate::format::{BorderEdge, Color, FillPattern, Underline};
use crate::refs::{MAX_COL, MAX_ROW, col_number, parse_range};
use crate::sheet::{Rect, Worksheet};
use crate::xml::{RawXml, XmlElement};

/// A differential format (`dxf`): the parts of a cell's look a rule overrides. Every part is
/// optional; an absent part leaves the cell's own format alone.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Dxf {
    pub font: DxfFont,
    pub fill: Option<DxfFill>,
    pub border: DxfBorder,
    /// The dxf's own number format code, such as `mmmm\ \'yy`.
    pub num_fmt: Option<String>,
}

/// Font overrides of a [`Dxf`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DxfFont {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub strike: Option<bool>,
    pub underline: Option<Underline>,
    pub color: Option<Color>,
}

/// Fill override of a [`Dxf`]. In a dxf the colour of a solid fill is `bg` (Excel writes
/// `<patternFill><bgColor/></patternFill>` with no pattern type); `fg` is the pattern colour.
#[derive(Debug, Clone, PartialEq)]
pub struct DxfFill {
    pub pattern: FillPattern,
    pub fg: Option<Color>,
    pub bg: Option<Color>,
}

impl DxfFill {
    /// The colour a solid (or unspecified) pattern shows.
    pub fn solid_color(&self) -> Option<Color> {
        self.bg.or(self.fg)
    }
}

/// Border overrides of a [`Dxf`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DxfBorder {
    pub left: Option<BorderEdge>,
    pub right: Option<BorderEdge>,
    pub top: Option<BorderEdge>,
    pub bottom: Option<BorderEdge>,
}

impl Dxf {
    pub(crate) fn parse(e: &XmlElement, num_fmts: &dyn Fn(u32) -> Option<String>) -> Self {
        let mut d = Self::default();
        if let Some(f) = e.child_main("font") {
            let flag = |name: &str| {
                f.child_main(name)
                    .map(|c| !matches!(c.head.attr("val").map(str::trim), Some("0" | "false")))
            };
            d.font.bold = flag("b");
            d.font.italic = flag("i");
            d.font.strike = flag("strike");
            d.font.underline = f.child_main("u").map(|u| match u.head.attr("val") {
                None | Some("single") => Underline::Single,
                Some("double") => Underline::Double,
                Some("singleAccounting") => Underline::SingleAccounting,
                Some("doubleAccounting") => Underline::DoubleAccounting,
                _ => Underline::None,
            });
            d.font.color = f.child_main("color").map(Color::parse);
        }
        if let Some(p) = e
            .child_main("fill")
            .and_then(|f| f.child_main("patternFill"))
        {
            d.fill = Some(DxfFill {
                pattern: p
                    .head
                    .attr("patternType")
                    .map_or(FillPattern::Solid, FillPattern::parse),
                fg: p.child_main("fgColor").map(Color::parse),
                bg: p.child_main("bgColor").map(Color::parse),
            });
        }
        if let Some(b) = e.child_main("border") {
            for c in b.elements() {
                if c.head.attr("style").is_none() {
                    continue;
                }
                let edge = Some(BorderEdge::parse(c));
                match c.head.local_name() {
                    "left" | "start" => d.border.left = edge,
                    "right" | "end" => d.border.right = edge,
                    "top" => d.border.top = edge,
                    "bottom" => d.border.bottom = edge,
                    _ => {}
                }
            }
        }
        if let Some(n) = e.child_main("numFmt") {
            d.num_fmt = n.head.attr("formatCode").map(str::to_owned).or_else(|| {
                n.head
                    .attr("numFmtId")
                    .and_then(|v| v.trim().parse().ok())
                    .and_then(num_fmts)
            });
        }
        d
    }
}

impl Dxf {
    /// Fills every part this format leaves unset from `lower` (a lower-priority rule's
    /// format): the first rule to set a property keeps it.
    pub fn fill_missing_from(&mut self, lower: &Dxf) {
        fn keep<T: Clone>(slot: &mut Option<T>, other: &Option<T>) {
            if slot.is_none() {
                slot.clone_from(other);
            }
        }
        keep(&mut self.font.bold, &lower.font.bold);
        keep(&mut self.font.italic, &lower.font.italic);
        keep(&mut self.font.strike, &lower.font.strike);
        keep(&mut self.font.underline, &lower.font.underline);
        keep(&mut self.font.color, &lower.font.color);
        keep(&mut self.fill, &lower.fill);
        keep(&mut self.border.left, &lower.border.left);
        keep(&mut self.border.right, &lower.border.right);
        keep(&mut self.border.top, &lower.border.top);
        keep(&mut self.border.bottom, &lower.border.bottom);
        keep(&mut self.num_fmt, &lower.num_fmt);
    }
}

/// How a `cfvo` threshold is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfvoKind {
    Num,
    Percent,
    Max,
    Min,
    Formula,
    Percentile,
    /// `autoMin` (Excel 2010 data bars): the lowest value, or 0 when every value is positive.
    AutoMin,
    /// `autoMax` (Excel 2010 data bars): the highest value, or 0 when every value is negative.
    AutoMax,
}

/// A conditional-format value object (`cfvo`).
#[derive(Debug, Clone, PartialEq)]
pub struct Cfvo {
    pub kind: CfvoKind,
    /// The `val` attribute (a number, percentage or formula).
    pub val: Option<String>,
    /// Greater-or-equal (`gte`, default true): whether a value on the threshold is above it.
    pub gte: bool,
}

impl Cfvo {
    fn parse(e: &XmlElement) -> Self {
        let kind = match e.head.attr("type") {
            Some("num") => CfvoKind::Num,
            Some("percent") => CfvoKind::Percent,
            Some("max") => CfvoKind::Max,
            Some("formula") => CfvoKind::Formula,
            Some("percentile") => CfvoKind::Percentile,
            Some("autoMin") => CfvoKind::AutoMin,
            Some("autoMax") => CfvoKind::AutoMax,
            _ => CfvoKind::Min,
        };
        // The Excel 2010 form keeps the value in an `<xm:f>` child instead of `val`.
        let val = e.head.attr("val").map(str::to_owned).or_else(|| {
            e.elements()
                .find(|c| c.head.local_name() == "f")
                .map(XmlElement::own_text)
        });
        Self {
            kind,
            val,
            gte: !matches!(e.head.attr("gte").map(str::trim), Some("0" | "false")),
        }
    }
}

/// The `timePeriod` of a time-period rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimePeriod {
    Today,
    Yesterday,
    Tomorrow,
    Last7Days,
    ThisMonth,
    LastMonth,
    NextMonth,
    ThisWeek,
    LastWeek,
    NextWeek,
}

/// `operator` of a `cellIs` rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellIsOp {
    LessThan,
    LessThanOrEqual,
    Equal,
    NotEqual,
    GreaterThanOrEqual,
    GreaterThan,
    Between,
    NotBetween,
}

/// What a rule tests.
#[derive(Debug, Clone, PartialEq)]
pub enum CfKind {
    CellIs {
        op: CellIsOp,
        formulas: Vec<String>,
    },
    Expression,
    Top10 {
        rank: u32,
        percent: bool,
        bottom: bool,
    },
    AboveAverage {
        above: bool,
        equal: bool,
        std_dev: u32,
    },
    DuplicateValues,
    UniqueValues,
    ContainsText(String),
    NotContainsText(String),
    BeginsWith(String),
    EndsWith(String),
    ContainsBlanks,
    NotContainsBlanks,
    ContainsErrors,
    NotContainsErrors,
    TimePeriod(TimePeriod),
    ColorScale {
        cfvos: Vec<Cfvo>,
        colors: Vec<Color>,
    },
    DataBar {
        min: Cfvo,
        max: Cfvo,
        color: Color,
        min_length: u32,
        max_length: u32,
        show_value: bool,
        /// The Excel 2010 `x14:dataBar` extension of the rule, when the sheet has one.
        ext: Option<DataBarExt>,
    },
    IconSet {
        name: String,
        cfvos: Vec<Cfvo>,
        reverse: bool,
        show_value: bool,
    },
    /// A `type` this reader does not know; kept (and written back) but never evaluated.
    Unknown(String),
}

/// One `cfRule`.
#[derive(Debug, Clone, PartialEq)]
pub struct CfRule {
    pub kind: CfKind,
    /// Lower numbers win; the sheet's rules apply in this order across all blocks.
    pub priority: i64,
    pub dxf_id: Option<u32>,
    pub stop_if_true: bool,
    /// The rule's first `formula` as stored (relative to the top-left cell of the block).
    pub formula: Option<String>,
    /// The `x14:id` that links the rule to its Excel 2010 extension (`x14:cfRule`), if any.
    pub ext_id: Option<String>,
}

/// One `conditionalFormatting` element: the cells (`sqref`) and the rules applying to them.
#[derive(Debug, Clone, PartialEq)]
pub struct CfBlock {
    pub ranges: Vec<Rect>,
    pub rules: Vec<CfRule>,
}

impl CfBlock {
    /// The cell the stored formulas are relative to: the top-left of the block's ranges.
    pub fn anchor(&self) -> (u32, u32) {
        let row = self.ranges.iter().map(|r| r.0).min().unwrap_or(1);
        let col = self.ranges.iter().map(|r| r.1).min().unwrap_or(1);
        (row, col)
    }

    pub fn contains(&self, row: u32, col: u32) -> bool {
        self.ranges
            .iter()
            .any(|r| row >= r.0 && row <= r.2 && col >= r.1 && col <= r.3)
    }
}

/// A `sqref` range, including whole-column (`B:D`) and whole-row (`2:4`) forms.
fn parse_sqref_range(s: &str) -> Option<Rect> {
    if let Some(r) = parse_range(s) {
        return Some(r);
    }
    let (a, b) = s.split_once(':').unwrap_or((s, s));
    if let (Some(c1), Some(c2)) = (col_number(a), col_number(b)) {
        return Some((1, c1.min(c2), MAX_ROW, c1.max(c2)));
    }
    if let (Ok(r1), Ok(r2)) = (a.parse::<u32>(), b.parse::<u32>())
        && r1 >= 1
        && r2 >= 1
        && r1 <= MAX_ROW
        && r2 <= MAX_ROW
    {
        return Some((r1.min(r2), 1, r1.max(r2), MAX_COL));
    }
    None
}

fn uint(e: &XmlElement, name: &str) -> Option<u32> {
    e.head.attr(name).and_then(|v| v.trim().parse().ok())
}

fn flag(e: &XmlElement, name: &str, default: bool) -> bool {
    match e.head.attr(name).map(str::trim) {
        Some("1" | "true") => true,
        Some("0" | "false") => false,
        _ => default,
    }
}

fn parse_rule(e: &XmlElement) -> CfRule {
    let formulas: Vec<String> = e
        .elements()
        .filter(|c| c.head.is_main("formula"))
        .map(XmlElement::own_text)
        .collect();
    let text = || e.head.attr("text").unwrap_or_default().to_owned();
    let cfvos = |parent: &XmlElement| -> Vec<Cfvo> {
        parent
            .elements()
            .filter(|c| c.head.is_main("cfvo"))
            .map(Cfvo::parse)
            .collect()
    };
    let kind = match e.head.attr("type").unwrap_or_default() {
        "cellIs" => {
            let op = match e.head.attr("operator") {
                Some("lessThan") => Some(CellIsOp::LessThan),
                Some("lessThanOrEqual") => Some(CellIsOp::LessThanOrEqual),
                Some("equal") => Some(CellIsOp::Equal),
                Some("notEqual") => Some(CellIsOp::NotEqual),
                Some("greaterThanOrEqual") => Some(CellIsOp::GreaterThanOrEqual),
                Some("greaterThan") => Some(CellIsOp::GreaterThan),
                Some("between") => Some(CellIsOp::Between),
                Some("notBetween") => Some(CellIsOp::NotBetween),
                _ => None,
            };
            match op {
                Some(op) => CfKind::CellIs {
                    op,
                    formulas: formulas.clone(),
                },
                None => CfKind::Unknown("cellIs".to_owned()),
            }
        }
        "expression" => CfKind::Expression,
        "top10" => CfKind::Top10 {
            rank: uint(e, "rank").unwrap_or(10),
            percent: flag(e, "percent", false),
            bottom: flag(e, "bottom", false),
        },
        "aboveAverage" => CfKind::AboveAverage {
            above: flag(e, "aboveAverage", true),
            equal: flag(e, "equalAverage", false),
            std_dev: uint(e, "stdDev").unwrap_or(0),
        },
        "duplicateValues" => CfKind::DuplicateValues,
        "uniqueValues" => CfKind::UniqueValues,
        "containsText" => CfKind::ContainsText(text()),
        "notContainsText" => CfKind::NotContainsText(text()),
        "beginsWith" => CfKind::BeginsWith(text()),
        "endsWith" => CfKind::EndsWith(text()),
        "containsBlanks" => CfKind::ContainsBlanks,
        "notContainsBlanks" => CfKind::NotContainsBlanks,
        "containsErrors" => CfKind::ContainsErrors,
        "notContainsErrors" => CfKind::NotContainsErrors,
        "timePeriod" => match e.head.attr("timePeriod") {
            Some("today") => CfKind::TimePeriod(TimePeriod::Today),
            Some("yesterday") => CfKind::TimePeriod(TimePeriod::Yesterday),
            Some("tomorrow") => CfKind::TimePeriod(TimePeriod::Tomorrow),
            Some("last7Days") => CfKind::TimePeriod(TimePeriod::Last7Days),
            Some("thisMonth") => CfKind::TimePeriod(TimePeriod::ThisMonth),
            Some("lastMonth") => CfKind::TimePeriod(TimePeriod::LastMonth),
            Some("nextMonth") => CfKind::TimePeriod(TimePeriod::NextMonth),
            Some("thisWeek") => CfKind::TimePeriod(TimePeriod::ThisWeek),
            Some("lastWeek") => CfKind::TimePeriod(TimePeriod::LastWeek),
            Some("nextWeek") => CfKind::TimePeriod(TimePeriod::NextWeek),
            _ => CfKind::Unknown("timePeriod".to_owned()),
        },
        "colorScale" => e
            .child_main("colorScale")
            .map(|cs| CfKind::ColorScale {
                cfvos: cfvos(cs),
                colors: cs
                    .elements()
                    .filter(|c| c.head.is_main("color"))
                    .map(Color::parse)
                    .collect(),
            })
            .unwrap_or_else(|| CfKind::Unknown("colorScale".to_owned())),
        "dataBar" => e
            .child_main("dataBar")
            .and_then(|db| {
                let v = cfvos(db);
                Some(CfKind::DataBar {
                    min: v.first()?.clone(),
                    max: v.get(1)?.clone(),
                    color: db.child_main("color").map_or(Color::AUTO, Color::parse),
                    min_length: uint(db, "minLength").unwrap_or(10),
                    max_length: uint(db, "maxLength").unwrap_or(90),
                    show_value: flag(db, "showValue", true),
                    ext: None,
                })
            })
            .unwrap_or_else(|| CfKind::Unknown("dataBar".to_owned())),
        "iconSet" => e
            .child_main("iconSet")
            .map(|is| CfKind::IconSet {
                name: is
                    .head
                    .attr("iconSet")
                    .unwrap_or("3TrafficLights1")
                    .to_owned(),
                cfvos: cfvos(is),
                reverse: flag(is, "reverse", false),
                show_value: flag(is, "showValue", true),
            })
            .unwrap_or_else(|| CfKind::Unknown("iconSet".to_owned())),
        other => CfKind::Unknown(other.to_owned()),
    };
    CfRule {
        kind,
        priority: e
            .head
            .attr("priority")
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(i64::MAX),
        dxf_id: uint(e, "dxfId"),
        stop_if_true: flag(e, "stopIfTrue", false),
        formula: formulas.into_iter().next(),
        ext_id: ext_id(e),
    }
}

/// The `x14:id` inside the rule's `extLst`.
fn ext_id(rule: &XmlElement) -> Option<String> {
    rule.elements()
        .filter(|c| c.head.is_main("extLst"))
        .flat_map(XmlElement::elements)
        .flat_map(XmlElement::elements)
        .find(|c| c.head.local_name() == "id")
        .map(|c| c.own_text().trim().to_owned())
        .filter(|id| !id.is_empty())
}

/// Where the axis of an Excel 2010 data bar sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DataBarAxis {
    /// Where zero falls between the minimum and the maximum (the default).
    #[default]
    Automatic,
    /// The middle of the cell.
    Midpoint,
    /// No axis: every bar grows from the left (or right) edge.
    None,
}

/// Which way the positive bars of an Excel 2010 data bar grow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DataBarDirection {
    #[default]
    Context,
    LeftToRight,
    RightToLeft,
}

/// The Excel 2010 `x14:dataBar` extension: what the 2007 `dataBar` element cannot say. Read from
/// the sheet's `extLst`; the raw tree keeps it byte for byte, so it survives a save.
#[derive(Debug, Clone, PartialEq)]
pub struct DataBarExt {
    /// Bar length at the lowest and highest value, in percent of the cell width.
    pub min_length: u32,
    pub max_length: u32,
    /// Gradient fill (default) or solid.
    pub gradient: bool,
    /// Draw a border around the bar, in `border_color` (the bar colour when absent).
    pub border: bool,
    pub border_color: Option<Color>,
    /// Negative bars use the positive bar's colour.
    pub negative_same_as_positive: bool,
    /// Fill of negative bars; Excel's default is red when absent.
    pub negative_fill: Option<Color>,
    pub negative_border_same_as_positive: bool,
    pub negative_border_color: Option<Color>,
    pub axis: DataBarAxis,
    pub axis_color: Option<Color>,
    pub direction: DataBarDirection,
    /// The extension's own minimum and maximum (`autoMin`, `autoMax`, numbers, ...).
    pub min: Option<Cfvo>,
    pub max: Option<Cfvo>,
}

impl DataBarExt {
    fn parse(db: &XmlElement) -> Self {
        let local = |name: &str| db.elements().find(|c| c.head.local_name() == name);
        let cfvos: Vec<Cfvo> = db
            .elements()
            .filter(|c| c.head.local_name() == "cfvo")
            .map(Cfvo::parse)
            .collect();
        Self {
            min_length: uint(db, "minLength").unwrap_or(0),
            max_length: uint(db, "maxLength").unwrap_or(100),
            gradient: flag(db, "gradient", true),
            border: flag(db, "border", false),
            border_color: local("borderColor").map(Color::parse),
            negative_same_as_positive: flag(db, "negativeBarColorSameAsPositive", false),
            negative_fill: local("negativeFillColor").map(Color::parse),
            negative_border_same_as_positive: flag(
                db,
                "negativeBarBorderColorSameAsPositive",
                true,
            ),
            negative_border_color: local("negativeBorderColor").map(Color::parse),
            axis: match db.head.attr("axisPosition") {
                Some("middle") => DataBarAxis::Midpoint,
                Some("none") => DataBarAxis::None,
                _ => DataBarAxis::Automatic,
            },
            axis_color: local("axisColor").map(Color::parse),
            direction: match db.head.attr("direction") {
                Some("leftToRight") => DataBarDirection::LeftToRight,
                Some("rightToLeft") => DataBarDirection::RightToLeft,
                _ => DataBarDirection::Context,
            },
            min: cfvos.first().cloned(),
            max: cfvos.get(1).cloned(),
        }
    }
}

/// The data bar extensions of a sheet's `extLst`, by the id the `cfRule` points at.
fn data_bar_exts(after: &[RawXml]) -> Vec<(String, DataBarExt)> {
    let mut out = Vec::new();
    for ext_lst in after
        .iter()
        .filter_map(RawXml::as_element)
        .filter(|e| e.head.is_main("extLst"))
    {
        let rules = ext_lst
            .elements()
            .flat_map(XmlElement::elements)
            .filter(|e| e.head.local_name() == "conditionalFormattings")
            .flat_map(XmlElement::elements)
            .filter(|e| e.head.local_name() == "conditionalFormatting")
            .flat_map(XmlElement::elements)
            .filter(|e| e.head.local_name() == "cfRule");
        for rule in rules {
            let id = rule.head.attr("id").map(str::to_owned);
            let bar = rule.elements().find(|c| c.head.local_name() == "dataBar");
            if let (Some(id), Some(bar)) = (id, bar) {
                out.push((id, DataBarExt::parse(bar)));
            }
        }
    }
    out
}

/// Parses a `conditionalFormatting` element; `None` when it has no usable range.
pub(crate) fn parse_block(e: &XmlElement) -> Option<CfBlock> {
    let ranges: Vec<Rect> = e
        .head
        .attr("sqref")?
        .split_whitespace()
        .filter_map(parse_sqref_range)
        .collect();
    if ranges.is_empty() {
        return None;
    }
    let rules = e
        .elements()
        .filter(|c| c.head.is_main("cfRule"))
        .map(parse_rule)
        .collect();
    Some(CfBlock { ranges, rules })
}

impl Worksheet {
    /// The sheet's `conditionalFormatting` blocks in source order (read from the preserved
    /// tree; the sheet is not changed).
    pub fn conditional_formats(&self) -> Vec<CfBlock> {
        let mut blocks: Vec<CfBlock> = self
            .raw_after()
            .iter()
            .filter_map(RawXml::as_element)
            .filter(|e| e.head.is_main("conditionalFormatting"))
            .filter_map(parse_block)
            .collect();
        let exts = data_bar_exts(self.raw_after());
        if !exts.is_empty() {
            for rule in blocks.iter_mut().flat_map(|b| b.rules.iter_mut()) {
                let found = rule
                    .ext_id
                    .as_ref()
                    .and_then(|id| exts.iter().find(|(i, _)| i == id));
                if let (Some((_, ext)), CfKind::DataBar { ext: slot, .. }) = (found, &mut rule.kind)
                {
                    *slot = Some(ext.clone());
                }
            }
        }
        blocks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml;

    fn element(src: &str) -> XmlElement {
        xml::parse(src.as_bytes()).unwrap().root
    }

    const NS: &str = "xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"";

    #[test]
    fn a_block_with_two_ranges_and_rules_parses() {
        let e = element(&format!(
            r#"<conditionalFormatting {NS} sqref="A2:A5 C:C"><cfRule type="cellIs" dxfId="3" priority="2" operator="between"><formula>1</formula><formula>9</formula></cfRule><cfRule type="top10" priority="1" rank="3" bottom="1"/></conditionalFormatting>"#
        ));
        let b = parse_block(&e).unwrap();
        assert_eq!(b.ranges, vec![(2, 1, 5, 1), (1, 3, MAX_ROW, 3)]);
        assert_eq!(b.rules.len(), 2);
        assert_eq!(b.rules[0].dxf_id, Some(3));
        assert!(matches!(
            &b.rules[0].kind,
            CfKind::CellIs { op: CellIsOp::Between, formulas } if formulas == &["1", "9"]
        ));
        assert_eq!(
            b.rules[1].kind,
            CfKind::Top10 {
                rank: 3,
                percent: false,
                bottom: true
            }
        );
        assert_eq!(b.anchor(), (1, 1));
    }

    #[test]
    fn a_dxf_reads_font_fill_border_and_number_format() {
        let e = element(&format!(
            r#"<dxf {NS}><font><b/><i val="0"/><color rgb="FF9C0006"/></font><numFmt numFmtId="166" formatCode="mmmm\ \'yy"/><fill><patternFill><bgColor rgb="FFFFC7CE"/></patternFill></fill><border><left style="thin"><color auto="1"/></left><right/></border></dxf>"#
        ));
        let d = Dxf::parse(&e, &|_| None);
        assert_eq!(d.font.bold, Some(true));
        assert_eq!(d.font.italic, Some(false));
        assert_eq!(d.num_fmt.as_deref(), Some("mmmm\\ \\'yy"));
        assert!(d.fill.unwrap().bg.is_some());
        assert!(d.border.left.is_some());
        assert!(d.border.right.is_none());
    }

    #[test]
    fn an_x14_data_bar_extension_is_read() {
        let e = element(&format!(
            r#"<dataBar {NS} xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main" minLength="0" maxLength="100" gradient="0" axisPosition="middle" direction="rightToLeft"><x14:cfvo type="autoMin"/><x14:cfvo type="num"><xm:f>50</xm:f></x14:cfvo><x14:negativeFillColor rgb="FFFF0000"/><x14:axisColor rgb="FF000000"/></dataBar>"#
        ));
        let x = DataBarExt::parse(&e);
        assert_eq!((x.min_length, x.max_length), (0, 100));
        assert!(!x.gradient && !x.border);
        assert!(!x.negative_same_as_positive);
        assert_eq!(x.axis, DataBarAxis::Midpoint);
        assert_eq!(x.direction, DataBarDirection::RightToLeft);
        assert_eq!(x.min.unwrap().kind, CfvoKind::AutoMin);
        let max = x.max.unwrap();
        assert_eq!((max.kind, max.val.as_deref()), (CfvoKind::Num, Some("50")));
        assert!(x.negative_fill.is_some() && x.axis_color.is_some());
    }

    #[test]
    fn a_rule_names_its_extension_id() {
        let e = element(&format!(
            r#"<cfRule {NS} xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" type="dataBar" priority="1"><dataBar><cfvo type="min"/><cfvo type="max"/><color rgb="FF638EC6"/></dataBar><extLst><ext uri="{{B025F937-C7B1-47D3-B67F-A62EFF666E3E}}"><x14:id>{{AB-12}}</x14:id></ext></extLst></cfRule>"#
        ));
        assert_eq!(parse_rule(&e).ext_id.as_deref(), Some("{AB-12}"));
    }
}
