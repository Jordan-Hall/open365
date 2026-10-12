//! Conditional formatting: add, edit and delete rules as Excel writes them (`cfRule`, `dxf`, and the
//! Excel 2010 `x14` extension of data bars).

use super::{MAIN, esc};
use crate::XlsxError;
use crate::conditional::{
    CellIsOp, CfBlock, CfKind, Cfvo, CfvoKind, DataBarAxis, DataBarDirection, DataBarExt, Dxf,
    TimePeriod,
};
use crate::format::{Color, ColorSource, FillPattern, Underline};
use crate::refs::{MAX_COL, MAX_ROW, cell_ref_string, parse_range};
use crate::sheet::Rect;
use crate::structure::range_a1;
use crate::styles::Styles;
use crate::styles_edit::{
    NumberFormat, border_style_name, color_element, el, intern, pattern_name,
};
use crate::workbook::Workbook;
use crate::xml::{RawXml, XmlElement};

const X14_NS: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const XM_NS: &str = "http://schemas.microsoft.com/office/excel/2006/main";
const URI_CF_EXT: &str = "{78C0D931-6437-407d-A8EE-F0AAD7539E65}";
const URI_DATABAR_ID: &str = "{B025F937-C7B1-47D3-B67F-A62EFF666E3E}";

/// One rule to write.
#[derive(Debug, Clone, PartialEq)]
pub struct CfRuleSpec {
    pub kind: CfKind,
    /// The format the rule applies (font, fill, border, number format); ignored by colour scales,
    /// data bars and icon sets.
    pub format: Option<Dxf>,
    pub stop_if_true: bool,
    /// The formula of an [`CfKind::Expression`] rule, relative to the top-left cell of the first range.
    pub formula: Option<String>,
}

/// The cells and the rules of one `conditionalFormatting` element.
#[derive(Debug, Clone, PartialEq)]
pub struct CfSpec {
    pub ranges: Vec<Rect>,
    pub rules: Vec<CfRuleSpec>,
}

fn color_attrs(c: &Color) -> String {
    let mut s = match c.source {
        ColorSource::Auto => " auto=\"1\"".to_owned(),
        ColorSource::Rgb(rgb) => format!(" rgb=\"FF{:02X}{:02X}{:02X}\"", rgb.r, rgb.g, rgb.b),
        ColorSource::Theme(t) => format!(" theme=\"{t}\""),
        ColorSource::Indexed(i) => format!(" indexed=\"{i}\""),
    };
    if c.tint != 0.0 {
        s.push_str(&format!(" tint=\"{}\"", c.tint));
    }
    s
}

fn cfvo_text(tag: &str, v: &Cfvo, ext: bool) -> String {
    let ty = match v.kind {
        CfvoKind::Num => "num",
        CfvoKind::Percent => "percent",
        CfvoKind::Max => "max",
        CfvoKind::Min => "min",
        CfvoKind::Formula => "formula",
        CfvoKind::Percentile => "percentile",
        CfvoKind::AutoMin => "autoMin",
        CfvoKind::AutoMax => "autoMax",
    };
    let gte = if v.gte { "" } else { " gte=\"0\"" };
    match (&v.val, ext) {
        (Some(val), true) => format!(
            "<{tag} type=\"{ty}\"{gte}><xm:f>{}</xm:f></{tag}>",
            esc(val)
        ),
        (Some(val), false) => format!("<{tag} type=\"{ty}\" val=\"{}\"{gte}/>", esc(val)),
        (None, _) => format!("<{tag} type=\"{ty}\"{gte}/>"),
    }
}

fn time_formula(p: TimePeriod, a: &str) -> (&'static str, String) {
    match p {
        TimePeriod::Today => ("today", format!("FLOOR({a},1)=TODAY()")),
        TimePeriod::Yesterday => ("yesterday", format!("FLOOR({a},1)=TODAY()-1")),
        TimePeriod::Tomorrow => ("tomorrow", format!("FLOOR({a},1)=TODAY()+1")),
        TimePeriod::Last7Days => (
            "last7Days",
            format!("AND(TODAY()-FLOOR({a},1)<=6,FLOOR({a},1)<=TODAY())"),
        ),
        TimePeriod::ThisMonth => (
            "thisMonth",
            format!("AND(MONTH({a})=MONTH(TODAY()),YEAR({a})=YEAR(TODAY()))"),
        ),
        TimePeriod::LastMonth => (
            "lastMonth",
            format!("AND(MONTH({a})=MONTH(EDATE(TODAY(),0-1)),YEAR({a})=YEAR(EDATE(TODAY(),0-1)))"),
        ),
        TimePeriod::NextMonth => (
            "nextMonth",
            format!("AND(MONTH({a})=MONTH(EDATE(TODAY(),0+1)),YEAR({a})=YEAR(EDATE(TODAY(),0+1)))"),
        ),
        TimePeriod::ThisWeek => (
            "thisWeek",
            format!(
                "AND(TODAY()-ROUNDDOWN({a},0)<=WEEKDAY(TODAY())-1,ROUNDDOWN({a},0)-TODAY()<=7-WEEKDAY(TODAY()))"
            ),
        ),
        TimePeriod::LastWeek => (
            "lastWeek",
            format!(
                "AND(TODAY()-ROUNDDOWN({a},0)>=(WEEKDAY(TODAY())),TODAY()-ROUNDDOWN({a},0)<(WEEKDAY(TODAY())+7))"
            ),
        ),
        TimePeriod::NextWeek => (
            "nextWeek",
            format!(
                "AND(ROUNDDOWN({a},0)-TODAY()>(7-WEEKDAY(TODAY())),ROUNDDOWN({a},0)-TODAY()<(15-WEEKDAY(TODAY())))"
            ),
        ),
    }
}

fn quote(s: &str) -> String {
    s.replace('"', "\"\"")
}

fn op_name(op: CellIsOp) -> &'static str {
    match op {
        CellIsOp::LessThan => "lessThan",
        CellIsOp::LessThanOrEqual => "lessThanOrEqual",
        CellIsOp::Equal => "equal",
        CellIsOp::NotEqual => "notEqual",
        CellIsOp::GreaterThanOrEqual => "greaterThanOrEqual",
        CellIsOp::GreaterThan => "greaterThan",
        CellIsOp::Between => "between",
        CellIsOp::NotBetween => "notBetween",
    }
}

/// A GUID derived from `seed`, so replaying a command writes the same bytes.
fn guid(seed: &str) -> String {
    use std::hash::{Hash, Hasher};
    let part = |salt: u8| {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (salt, seed).hash(&mut h);
        h.finish()
    };
    let (a, b) = (part(1), part(2));
    format!(
        "{{{:08X}-{:04X}-4{:03X}-{:04X}-{:012X}}}",
        (a >> 32) as u32,
        (a >> 16) as u16,
        a as u16 & 0x0FFF,
        ((b >> 48) as u16 & 0x3FFF) | 0x8000,
        b & 0xFFFF_FFFF_FFFF
    )
}

/// The `cfRule` text, and the x14 data bar entry (`id`, `x14:cfRule` text) when it has one.
fn rule_text(
    spec: &CfRuleSpec,
    priority: i64,
    dxf_id: Option<u32>,
    anchor: &str,
    seed: &str,
) -> Result<(String, Option<(String, String)>), XlsxError> {
    let mut attrs = String::new();
    let dxf = dxf_id.map_or(String::new(), |d| format!(" dxfId=\"{d}\""));
    let stop = if spec.stop_if_true {
        " stopIfTrue=\"1\""
    } else {
        ""
    };
    let mut inner = String::new();
    let mut ext = None;
    let ty: &str;
    let uses_dxf = !matches!(
        spec.kind,
        CfKind::ColorScale { .. } | CfKind::DataBar { .. } | CfKind::IconSet { .. }
    );
    let formula = |f: &str| format!("<formula>{}</formula>", esc(f));
    match &spec.kind {
        CfKind::CellIs { op, formulas } => {
            ty = "cellIs";
            let need = if matches!(op, CellIsOp::Between | CellIsOp::NotBetween) {
                2
            } else {
                1
            };
            if formulas.len() != need {
                return Err(XlsxError::UnsupportedEdit(format!(
                    "a cell-value rule with this operator needs {need} formula(s)"
                )));
            }
            attrs.push_str(&format!(" operator=\"{}\"", op_name(*op)));
            for f in formulas {
                inner.push_str(&formula(f));
            }
        }
        CfKind::Expression => {
            ty = "expression";
            let f = spec
                .formula
                .as_deref()
                .filter(|f| !f.trim().is_empty())
                .ok_or_else(|| {
                    XlsxError::UnsupportedEdit("an expression rule needs a formula".to_owned())
                })?;
            inner.push_str(&formula(f.trim_start_matches('=')));
        }
        CfKind::Top10 {
            rank,
            percent,
            bottom,
        } => {
            ty = "top10";
            if *percent {
                attrs.push_str(" percent=\"1\"");
            }
            if *bottom {
                attrs.push_str(" bottom=\"1\"");
            }
            attrs.push_str(&format!(" rank=\"{rank}\""));
        }
        CfKind::AboveAverage {
            above,
            equal,
            std_dev,
        } => {
            ty = "aboveAverage";
            if !above {
                attrs.push_str(" aboveAverage=\"0\"");
            }
            if *equal {
                attrs.push_str(" equalAverage=\"1\"");
            }
            if *std_dev > 0 {
                attrs.push_str(&format!(" stdDev=\"{std_dev}\""));
            }
        }
        CfKind::DuplicateValues => ty = "duplicateValues",
        CfKind::UniqueValues => ty = "uniqueValues",
        CfKind::ContainsText(t) => {
            ty = "containsText";
            attrs.push_str(&format!(" operator=\"containsText\" text=\"{}\"", esc(t)));
            inner.push_str(&formula(&format!(
                "NOT(ISERROR(SEARCH(\"{}\",{anchor})))",
                quote(t)
            )));
        }
        CfKind::NotContainsText(t) => {
            ty = "notContainsText";
            attrs.push_str(&format!(" operator=\"notContains\" text=\"{}\"", esc(t)));
            inner.push_str(&formula(&format!(
                "ISERROR(SEARCH(\"{}\",{anchor}))",
                quote(t)
            )));
        }
        CfKind::BeginsWith(t) => {
            ty = "beginsWith";
            attrs.push_str(&format!(" operator=\"beginsWith\" text=\"{}\"", esc(t)));
            inner.push_str(&formula(&format!(
                "LEFT({anchor},LEN(\"{q}\"))=\"{q}\"",
                q = quote(t)
            )));
        }
        CfKind::EndsWith(t) => {
            ty = "endsWith";
            attrs.push_str(&format!(" operator=\"endsWith\" text=\"{}\"", esc(t)));
            inner.push_str(&formula(&format!(
                "RIGHT({anchor},LEN(\"{q}\"))=\"{q}\"",
                q = quote(t)
            )));
        }
        CfKind::ContainsBlanks => {
            ty = "containsBlanks";
            inner.push_str(&formula(&format!("LEN(TRIM({anchor}))=0")));
        }
        CfKind::NotContainsBlanks => {
            ty = "notContainsBlanks";
            inner.push_str(&formula(&format!("LEN(TRIM({anchor}))>0")));
        }
        CfKind::ContainsErrors => {
            ty = "containsErrors";
            inner.push_str(&formula(&format!("ISERROR({anchor})")));
        }
        CfKind::NotContainsErrors => {
            ty = "notContainsErrors";
            inner.push_str(&formula(&format!("NOT(ISERROR({anchor}))")));
        }
        CfKind::TimePeriod(p) => {
            ty = "timePeriod";
            let (name, f) = time_formula(*p, anchor);
            attrs.push_str(&format!(" timePeriod=\"{name}\""));
            inner.push_str(&formula(&f));
        }
        CfKind::ColorScale { cfvos, colors } => {
            ty = "colorScale";
            if !(cfvos.len() == 2 || cfvos.len() == 3) || colors.len() != cfvos.len() {
                return Err(XlsxError::UnsupportedEdit(
                    "a colour scale needs two or three thresholds with one colour each".to_owned(),
                ));
            }
            inner.push_str("<colorScale>");
            for v in cfvos {
                inner.push_str(&cfvo_text("cfvo", v, false));
            }
            for c in colors {
                inner.push_str(&format!("<color{}/>", color_attrs(c)));
            }
            inner.push_str("</colorScale>");
        }
        CfKind::DataBar {
            min,
            max,
            color,
            min_length,
            max_length,
            show_value,
            ext: bar,
        } => {
            ty = "dataBar";
            let mut head = String::from("<dataBar");
            if *min_length != 10 {
                head.push_str(&format!(" minLength=\"{min_length}\""));
            }
            if *max_length != 90 {
                head.push_str(&format!(" maxLength=\"{max_length}\""));
            }
            if !show_value {
                head.push_str(" showValue=\"0\"");
            }
            inner.push_str(&head);
            inner.push('>');
            inner.push_str(&cfvo_text("cfvo", min, false));
            inner.push_str(&cfvo_text("cfvo", max, false));
            inner.push_str(&format!("<color{}/>", color_attrs(color)));
            inner.push_str("</dataBar>");
            let id = guid(&format!("{seed}|{priority}"));
            inner.push_str(&format!(
                "<extLst><ext uri=\"{URI_DATABAR_ID}\" xmlns:x14=\"{X14_NS}\"><x14:id>{id}</x14:id></ext></extLst>"
            ));
            let bar = bar.clone().unwrap_or_else(default_bar);
            ext = Some((id.clone(), x14_bar(&bar, *color, &id)));
        }
        CfKind::IconSet {
            name,
            cfvos,
            reverse,
            show_value,
        } => {
            ty = "iconSet";
            if matches!(name.as_str(), "3Stars" | "3Triangles" | "5Boxes") {
                return Err(XlsxError::UnsupportedEdit(format!(
                    "the icon set {name} is only written by the Excel 2010 extension, which is not supported yet"
                )));
            }
            let expected: usize = name
                .chars()
                .next()
                .and_then(|c| c.to_digit(10))
                .map_or(0, |d| d as usize);
            if expected == 0 || cfvos.len() != expected {
                return Err(XlsxError::UnsupportedEdit(format!(
                    "the icon set {name} needs {expected} thresholds"
                )));
            }
            inner.push_str("<iconSet");
            if name != "3TrafficLights1" {
                inner.push_str(&format!(" iconSet=\"{}\"", esc(name)));
            }
            if !show_value {
                inner.push_str(" showValue=\"0\"");
            }
            if *reverse {
                inner.push_str(" reverse=\"1\"");
            }
            inner.push('>');
            for v in cfvos {
                inner.push_str(&cfvo_text("cfvo", v, false));
            }
            inner.push_str("</iconSet>");
        }
        CfKind::Unknown(t) => {
            return Err(XlsxError::UnsupportedEdit(format!(
                "unknown rule type {t:?}"
            )));
        }
    }
    let dxf = if uses_dxf { dxf } else { String::new() };
    let text =
        format!("<cfRule type=\"{ty}\"{dxf} priority=\"{priority}\"{stop}{attrs}>{inner}</cfRule>");
    Ok((text, ext))
}

fn default_bar() -> DataBarExt {
    DataBarExt {
        min_length: 0,
        max_length: 100,
        gradient: true,
        border: false,
        border_color: None,
        negative_same_as_positive: false,
        negative_fill: None,
        negative_border_same_as_positive: true,
        negative_border_color: None,
        axis: DataBarAxis::Automatic,
        axis_color: None,
        direction: DataBarDirection::Context,
        min: Some(Cfvo {
            kind: CfvoKind::AutoMin,
            val: None,
            gte: true,
        }),
        max: Some(Cfvo {
            kind: CfvoKind::AutoMax,
            val: None,
            gte: true,
        }),
    }
}

/// The `x14:cfRule` of a data bar.
fn x14_bar(b: &DataBarExt, bar_color: Color, id: &str) -> String {
    let mut a = format!(
        "<x14:dataBar minLength=\"{}\" maxLength=\"{}\"",
        b.min_length, b.max_length
    );
    if !b.gradient {
        a.push_str(" gradient=\"0\"");
    }
    if b.border {
        a.push_str(" border=\"1\"");
    }
    if b.negative_same_as_positive {
        a.push_str(" negativeBarColorSameAsPositive=\"1\"");
    }
    if !b.negative_border_same_as_positive {
        a.push_str(" negativeBarBorderColorSameAsPositive=\"0\"");
    }
    match b.axis {
        DataBarAxis::Automatic => {}
        DataBarAxis::Midpoint => a.push_str(" axisPosition=\"middle\""),
        DataBarAxis::None => a.push_str(" axisPosition=\"none\""),
    }
    match b.direction {
        DataBarDirection::Context => {}
        DataBarDirection::LeftToRight => a.push_str(" direction=\"leftToRight\""),
        DataBarDirection::RightToLeft => a.push_str(" direction=\"rightToLeft\""),
    }
    a.push('>');
    let auto_min = Cfvo {
        kind: CfvoKind::AutoMin,
        val: None,
        gte: true,
    };
    let auto_max = Cfvo {
        kind: CfvoKind::AutoMax,
        val: None,
        gte: true,
    };
    a.push_str(&cfvo_text(
        "x14:cfvo",
        b.min.as_ref().unwrap_or(&auto_min),
        true,
    ));
    a.push_str(&cfvo_text(
        "x14:cfvo",
        b.max.as_ref().unwrap_or(&auto_max),
        true,
    ));
    if b.border {
        a.push_str(&format!(
            "<x14:borderColor{}/>",
            color_attrs(&b.border_color.unwrap_or(bar_color))
        ));
    }
    if !b.negative_same_as_positive {
        a.push_str(&format!(
            "<x14:negativeFillColor{}/>",
            color_attrs(&b.negative_fill.unwrap_or(rgb_red()))
        ));
    }
    if !b.negative_border_same_as_positive {
        a.push_str(&format!(
            "<x14:negativeBorderColor{}/>",
            color_attrs(&b.negative_border_color.unwrap_or(rgb_red()))
        ));
    }
    if b.axis != DataBarAxis::None {
        a.push_str(&format!(
            "<x14:axisColor{}/>",
            color_attrs(&b.axis_color.unwrap_or(Color {
                source: ColorSource::Rgb(crate::format::Rgb::BLACK),
                tint: 0.0
            }))
        ));
    }
    a.push_str("</x14:dataBar>");
    // Excel writes no priority on a data bar's x14:cfRule; the rule in the sheet carries it.
    format!("<x14:cfRule type=\"dataBar\" id=\"{id}\">{a}</x14:cfRule>")
}

fn rgb_red() -> Color {
    Color {
        source: ColorSource::Rgb(crate::format::Rgb::new(255, 0, 0)),
        tint: 0.0,
    }
}

impl Styles {
    /// Adds a differential format to `dxfs` (reusing an equal one) and returns its `dxfId`.
    pub(crate) fn add_dxf(&mut self, dxf: &Dxf) -> Result<u32, XlsxError> {
        let locale = self.num_fmt_locale;
        let (_, root, _) = self.parsed.as_mut().ok_or_else(|| {
            XlsxError::UnsupportedEdit("the workbook has no styles part".to_owned())
        })?;
        let tpl = root.clone();
        let mut d = el(&tpl, "dxf", &[]);
        let mut font = el(&tpl, "font", &[]);
        let flag = |name: &str, on: bool| {
            if on {
                el(&tpl, name, &[])
            } else {
                el(&tpl, name, &[("val", "0")])
            }
        };
        if let Some(b) = dxf.font.bold {
            font.children.push(RawXml::Element(flag("b", b)));
        }
        if let Some(i) = dxf.font.italic {
            font.children.push(RawXml::Element(flag("i", i)));
        }
        if let Some(s) = dxf.font.strike {
            font.children.push(RawXml::Element(flag("strike", s)));
        }
        if let Some(u) = dxf.font.underline {
            let e = match u {
                Underline::None => el(&tpl, "u", &[("val", "none")]),
                Underline::Single => el(&tpl, "u", &[]),
                Underline::Double => el(&tpl, "u", &[("val", "double")]),
                Underline::SingleAccounting => el(&tpl, "u", &[("val", "singleAccounting")]),
                Underline::DoubleAccounting => el(&tpl, "u", &[("val", "doubleAccounting")]),
            };
            font.children.push(RawXml::Element(e));
        }
        if let Some(c) = &dxf.font.color
            && let Some(e) = color_element(&tpl, "color", c, Some(1))
        {
            font.children.push(RawXml::Element(e));
        }
        if !font.children.is_empty() {
            d.children.push(RawXml::Element(font));
        }
        if let Some(code) = &dxf.num_fmt {
            let id = Self::number_format_id(root, &NumberFormat::Code(code.clone()), locale);
            d.children.push(RawXml::Element(el(
                &tpl,
                "numFmt",
                &[("numFmtId", &id.to_string()), ("formatCode", code)],
            )));
        }
        if let Some(f) = &dxf.fill {
            let mut fill = el(&tpl, "fill", &[]);
            let mut pf = if f.pattern == FillPattern::Solid && f.fg.is_none() {
                el(&tpl, "patternFill", &[])
            } else {
                el(
                    &tpl,
                    "patternFill",
                    &[("patternType", pattern_name(f.pattern))],
                )
            };
            if let Some(c) = &f.fg
                && let Some(e) = color_element(&tpl, "fgColor", c, Some(64))
            {
                pf.children.push(RawXml::Element(e));
            }
            if let Some(c) = &f.bg
                && let Some(e) = color_element(&tpl, "bgColor", c, Some(64))
            {
                pf.children.push(RawXml::Element(e));
            }
            fill.children.push(RawXml::Element(pf));
            d.children.push(RawXml::Element(fill));
        }
        let edges = [
            ("left", &dxf.border.left),
            ("right", &dxf.border.right),
            ("top", &dxf.border.top),
            ("bottom", &dxf.border.bottom),
        ];
        if edges.iter().any(|(_, e)| e.is_some()) {
            let mut border = el(&tpl, "border", &[]);
            for (name, edge) in edges {
                let Some(edge) = edge else { continue };
                let Some(style) = border_style_name(edge.style) else {
                    continue;
                };
                let mut e = el(&tpl, name, &[("style", style)]);
                if let Some(c) = color_element(&tpl, "color", &edge.color, Some(64)) {
                    e.children.push(RawXml::Element(c));
                }
                border.children.push(RawXml::Element(e));
            }
            d.children.push(RawXml::Element(border));
        }
        let before = root.child_main("dxfs").map_or(0, |s| s.elements().count());
        let idx = intern(root, "dxfs", d);
        if idx >= before {
            self.dxfs.push(dxf.clone());
        }
        self.reindex();
        u32::try_from(idx).map_err(|_| XlsxError::UnsupportedEdit("too many formats".to_owned()))
    }
}

fn rect_text(ranges: &[Rect]) -> String {
    ranges
        .iter()
        .map(|r| range_a1(*r))
        .collect::<Vec<_>>()
        .join(" ")
}

fn subtract(a: Rect, cut: Rect) -> Vec<Rect> {
    let (r1, c1, r2, c2) = a;
    let (x1, y1, x2, y2) = cut;
    if x1 > r2 || x2 < r1 || y1 > c2 || y2 < c1 {
        return vec![a];
    }
    let mut out = Vec::new();
    if x1 > r1 {
        out.push((r1, c1, x1 - 1, c2));
    }
    if x2 < r2 {
        out.push((x2 + 1, c1, r2, c2));
    }
    let (mr1, mr2) = (r1.max(x1), r2.min(x2));
    if y1 > c1 {
        out.push((mr1, c1, mr2, y1 - 1));
    }
    if y2 < c2 {
        out.push((mr1, y2 + 1, mr2, c2));
    }
    out
}

impl Workbook {
    fn check_cf_spec(spec: &CfSpec) -> Result<(), XlsxError> {
        if spec.ranges.is_empty() || spec.rules.is_empty() {
            return Err(XlsxError::UnsupportedEdit(
                "a conditional format needs cells and at least one rule".to_owned(),
            ));
        }
        for &(r1, c1, r2, c2) in &spec.ranges {
            if r1 == 0 || c1 == 0 || r1 > r2 || c1 > c2 || r2 > MAX_ROW || c2 > MAX_COL {
                return Err(XlsxError::UnsupportedEdit(
                    "the conditional format range is outside the worksheet".to_owned(),
                ));
            }
        }
        Ok(())
    }

    /// Builds the block element and the x14 entries for `spec`, giving the rules `priorities`.
    fn build_cf_block(
        &mut self,
        sheet: usize,
        spec: &CfSpec,
        priorities: &[i64],
    ) -> Result<(XmlElement, Vec<(String, String)>), XlsxError> {
        let first = spec
            .ranges
            .iter()
            .fold((u32::MAX, u32::MAX), |a, r| (a.0.min(r.0), a.1.min(r.1)));
        let anchor = cell_ref_string(first.0, first.1);
        let existing = self.cf_priorities(sheet).len();
        let mut body = String::new();
        let mut exts = Vec::new();
        for (i, rule) in spec.rules.iter().enumerate() {
            let dxf_id = match &rule.format {
                Some(d)
                    if !matches!(
                        rule.kind,
                        CfKind::ColorScale { .. } | CfKind::DataBar { .. } | CfKind::IconSet { .. }
                    ) =>
                {
                    self.styles_dirty = true;
                    Some(self.styles.add_dxf(d)?)
                }
                _ => None,
            };
            let (text, ext) = rule_text(
                rule,
                priorities[i],
                dxf_id,
                &anchor,
                &format!("{sheet}|{}|{i}|{existing}", rect_text(&spec.ranges)),
            )?;
            body.push_str(&text);
            if let Some((id, x)) = ext {
                exts.push((id, x));
            }
        }
        let ws = self.worksheet_mut(sheet)?;
        let el = ws.fragment(&format!(
            "<conditionalFormatting xmlns=\"{MAIN}\" sqref=\"{}\">{body}</conditionalFormatting>",
            rect_text(&spec.ranges)
        ))?;
        Ok((el, exts))
    }

    fn cf_priorities(&self, sheet: usize) -> Vec<i64> {
        self.sheets[sheet]
            .data
            .as_ref()
            .map(|ws| {
                ws.conditional_formats()
                    .iter()
                    .flat_map(|b| b.rules.iter().map(|r| r.priority))
                    .filter(|p| *p != i64::MAX)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Adds a conditional format. Its rules take the top priorities (1, 2 ...), pushing the existing
    /// rules down, as Excel does.
    ///
    /// # Errors
    /// The sheet is not a worksheet, a range is outside the grid, a rule is incomplete, or the rule
    /// kind cannot be written (see [`CfKind::Unknown`] and the three icon sets that need the Excel 2010
    /// extension).
    pub fn add_conditional_format(
        &mut self,
        sheet: usize,
        spec: &CfSpec,
    ) -> Result<usize, XlsxError> {
        Self::check_cf_spec(spec)?;
        self.worksheet_mut(sheet)?;
        self.edited()?;
        let n = i64::try_from(spec.rules.len()).unwrap_or(1);
        let priorities: Vec<i64> = (1..=n).collect();
        let (block, exts) = self.build_cf_block(sheet, spec, &priorities)?;
        let ws = self.worksheet_mut(sheet)?;
        for e in ws.root_elements_mut() {
            bump_priorities(e, n);
        }
        let index = ws.conditional_formats().len();
        ws.insert_root_element("conditionalFormatting", block);
        add_x14(ws, &spec.ranges, &exts)?;
        self.sheets[sheet].dirty = true;
        Ok(index)
    }

    /// Replaces the block `index` (in the order of [`Worksheet::conditional_formats`]) with `spec`; the
    /// rules keep the priorities of the ones they replace.
    ///
    /// # Errors
    /// As [`Workbook::add_conditional_format`]; also when `index` is out of range.
    pub fn replace_conditional_format(
        &mut self,
        sheet: usize,
        index: usize,
        spec: &CfSpec,
    ) -> Result<(), XlsxError> {
        Self::check_cf_spec(spec)?;
        let old: CfBlock = self
            .worksheet_mut(sheet)?
            .conditional_formats()
            .get(index)
            .cloned()
            .ok_or_else(|| XlsxError::UnsupportedEdit("no such conditional format".to_owned()))?;
        self.edited()?;
        let mut priorities: Vec<i64> = old.rules.iter().map(|r| r.priority).collect();
        let mut top = self.cf_priorities(sheet).into_iter().max().unwrap_or(0);
        while priorities.len() < spec.rules.len() {
            top += 1;
            priorities.push(top);
        }
        let (block, exts) = self.build_cf_block(sheet, spec, &priorities)?;
        let ws = self.worksheet_mut(sheet)?;
        for r in &old.rules {
            if let Some(id) = &r.ext_id {
                remove_x14(ws, id);
            }
        }
        let at = nth_block_position(ws, index)
            .ok_or_else(|| XlsxError::UnsupportedEdit("no such conditional format".to_owned()))?;
        ws.after_mut()[at] = RawXml::Element(block);
        add_x14(ws, &spec.ranges, &exts)?;
        self.sheets[sheet].dirty = true;
        Ok(())
    }

    /// Deletes the block `index` with its rules.
    ///
    /// # Errors
    /// The sheet is not a worksheet or `index` is out of range.
    pub fn delete_conditional_format(
        &mut self,
        sheet: usize,
        index: usize,
    ) -> Result<CfBlock, XlsxError> {
        let old: CfBlock = self
            .worksheet_mut(sheet)?
            .conditional_formats()
            .get(index)
            .cloned()
            .ok_or_else(|| XlsxError::UnsupportedEdit("no such conditional format".to_owned()))?;
        self.edited()?;
        let ws = self.worksheet_mut(sheet)?;
        for r in &old.rules {
            if let Some(id) = &r.ext_id {
                remove_x14(ws, id);
            }
        }
        if let Some(at) = nth_block_position(ws, index) {
            ws.after_mut().remove(at);
        }
        self.sheets[sheet].dirty = true;
        Ok(old)
    }

    /// Clear Rules from Selected Cells: removes `area` from every block; a block left with no cells is
    /// deleted. Returns how many blocks changed.
    ///
    /// # Errors
    /// The sheet is not a worksheet.
    pub fn clear_conditional_formats_in(
        &mut self,
        sheet: usize,
        area: Rect,
    ) -> Result<usize, XlsxError> {
        let blocks = self.worksheet_mut(sheet)?.conditional_formats();
        let hit: Vec<usize> = blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                b.ranges
                    .iter()
                    .any(|r| r.0 <= area.2 && area.0 <= r.2 && r.1 <= area.3 && area.1 <= r.3)
            })
            .map(|(i, _)| i)
            .collect();
        if hit.is_empty() {
            return Ok(0);
        }
        self.edited()?;
        let ws = self.worksheet_mut(sheet)?;
        for &i in hit.iter().rev() {
            let b = &blocks[i];
            let kept: Vec<Rect> = b.ranges.iter().flat_map(|r| subtract(*r, area)).collect();
            let Some(at) = nth_block_position(ws, i) else {
                continue;
            };
            if kept.is_empty() {
                for r in &b.rules {
                    if let Some(id) = &r.ext_id {
                        remove_x14(ws, id);
                    }
                }
                ws.after_mut().remove(at);
            } else if let Some(e) = ws.after_mut()[at].as_element_mut() {
                e.head.set_attr("sqref", &rect_text(&kept));
            }
        }
        self.sheets[sheet].dirty = true;
        Ok(hit.len())
    }
}

/// Position in `after` of the `index`-th block that parses (the order of `conditional_formats`).
fn nth_block_position(ws: &mut crate::sheet::Worksheet, index: usize) -> Option<usize> {
    ws.after_mut()
        .iter()
        .enumerate()
        .filter(|(_, n)| {
            n.as_element().is_some_and(|e| {
                e.head.is_main("conditionalFormatting")
                    && e.head.attr("sqref").is_some_and(|s| {
                        s.split_whitespace()
                            .any(|t| parse_range(t).is_some() || !t.is_empty())
                    })
            })
        })
        .map(|(i, _)| i)
        .nth(index)
}

fn bump_priorities(e: &mut XmlElement, n: i64) {
    let is_rule = e.head.local_name() == "cfRule";
    if is_rule
        && let Some(p) = e
            .head
            .attr("priority")
            .and_then(|v| v.trim().parse::<i64>().ok())
    {
        e.head.set_attr("priority", &(p + n).to_string());
    }
    for c in &mut e.children {
        if let RawXml::Element(x) = c {
            bump_priorities(x, n);
        }
    }
}

/// Adds the data bar entries to the sheet's `extLst` (creating the extension, as Excel does).
fn add_x14(
    ws: &mut crate::sheet::Worksheet,
    ranges: &[Rect],
    exts: &[(String, String)],
) -> Result<(), XlsxError> {
    if exts.is_empty() {
        return Ok(());
    }
    let sqref = rect_text(ranges);
    let body: String = exts.iter().map(|(_, x)| x.as_str()).collect();
    let mut cf = ws.fragment(&format!(
        "<x14:conditionalFormatting xmlns=\"{MAIN}\" xmlns:x14=\"{X14_NS}\" xmlns:xm=\"{XM_NS}\">{body}<xm:sqref>{sqref}</xm:sqref></x14:conditionalFormatting>"
    ))?;
    cf.head
        .attrs
        .retain(|a| a.name != "xmlns" && a.name != "xmlns:x14");
    // Find or create extLst / ext / x14:conditionalFormattings.
    if ws.root_element("extLst").is_none() {
        let el = ws.fragment(&format!("<extLst xmlns=\"{MAIN}\"/>"))?;
        ws.put_root_element("extLst", Some(el));
    }
    let Some(ext_lst) = ws
        .after_mut()
        .iter_mut()
        .find_map(|n| n.as_element_mut().filter(|e| e.head.is_main("extLst")))
    else {
        return Ok(());
    };
    let pos = ext_lst.children.iter().position(|c| {
        c.as_element()
            .is_some_and(|e| e.head.attr("uri") == Some(URI_CF_EXT))
    });
    let pos = match pos {
        Some(p) => p,
        None => {
            let mut ext = crate::sheet::parse_fragment(
                &format!(
                    "<ext xmlns=\"{MAIN}\" uri=\"{URI_CF_EXT}\" xmlns:x14=\"{X14_NS}\"><x14:conditionalFormattings/></ext>"
                ),
                &ext_lst.head,
            )?;
            ext.head.attrs.retain(|a| a.name != "xmlns");
            // The conditional-format extension goes first, as Excel writes it.
            ext_lst.children.insert(0, RawXml::Element(ext));
            0
        }
    };
    if let Some(RawXml::Element(ext)) = ext_lst.children.get_mut(pos) {
        match ext.children.iter_mut().find_map(|c| {
            c.as_element_mut()
                .filter(|e| e.head.local_name() == "conditionalFormattings")
        }) {
            Some(list) => list.children.push(RawXml::Element(cf)),
            None => {
                let mut list = ext.sibling("conditionalFormattings");
                list.children.push(RawXml::Element(cf));
                ext.children.push(RawXml::Element(list));
            }
        }
    }
    Ok(())
}

/// Removes the `x14:cfRule` with `id` (and the containers it leaves empty).
fn remove_x14(ws: &mut crate::sheet::Worksheet, id: &str) {
    let Some(ext_lst) = ws
        .after_mut()
        .iter_mut()
        .find_map(|n| n.as_element_mut().filter(|e| e.head.is_main("extLst")))
    else {
        return;
    };
    for ext in ext_lst
        .children
        .iter_mut()
        .filter_map(RawXml::as_element_mut)
    {
        for list in ext
            .children
            .iter_mut()
            .filter_map(RawXml::as_element_mut)
            .filter(|e| e.head.local_name() == "conditionalFormattings")
        {
            for cf in list.children.iter_mut().filter_map(RawXml::as_element_mut) {
                cf.children.retain(|c| {
                    c.as_element().is_none_or(|r| {
                        !(r.head.local_name() == "cfRule" && r.head.attr("id") == Some(id))
                    })
                });
            }
            list.children.retain(|c| {
                c.as_element()
                    .is_none_or(|cf| cf.elements().any(|r| r.head.local_name() == "cfRule"))
            });
        }
        ext.children.retain(|c| {
            c.as_element().is_none_or(|l| {
                l.head.local_name() != "conditionalFormattings" || l.elements().next().is_some()
            })
        });
    }
    ext_lst.children.retain(|c| {
        c.as_element()
            .is_none_or(|e| e.head.attr("uri") != Some(URI_CF_EXT) || e.elements().next().is_some())
    });
    let empty = ext_lst.elements().next().is_none();
    if empty {
        ws.put_root_element("extLst", None);
    }
}
