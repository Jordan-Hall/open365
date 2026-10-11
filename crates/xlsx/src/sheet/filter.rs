//! AutoFilter: the `autoFilter` element and the evaluation of its criteria into hidden rows.
//!
//! The rules follow Excel's output, measured through COM (`tools/office/make-sortfilter-refs.ps1`,
//! `fixtures/reference/xlsx/sortfilter-filter/scenarios.json`):
//!
//! - A value list (`filters`) matches the text a cell shows (number format applied), ignoring case; `blank`
//!   matches blank cells. Date groups (`dateGroupItem`, also in Excel's `xlrd2` extension) match a date or time at
//!   the grouping's level (year, month, day, hour, minute, second).
//! - A custom filter compares numbers with numbers, text with text (locale collation) and never mixes the two;
//!   blank cells satisfy only "does not equal". Equality with a text that holds `*`, `?` or `~` is a wildcard match
//!   on the shown text. `<>` with a single space as the value means "is not blank".
//! - Top 10 counts the numeric cells of the column: the n-th largest (or smallest) value is the threshold, ties
//!   stay; for a percentage n is the share of that count rounded down, at least 1. Above and below average compare
//!   with the mean of the numeric cells.
//! - Dynamic date filters use today's date; weeks start on Sunday. Quarters and months (`Q1`, `M1`...) match any
//!   year.
//! - Colour filters compare the cell fill (or font colour) with the colour of the differential format `dxfId`.
//! - Columns combine with AND. Blank, text and error cells never satisfy a numeric filter.

use std::sync::Arc;

use open365_calc::FormatLocale;
use open365_calc::numfmt::format_value_in;
use open365_calc::value::{ErrorKind, Value, compare_numbers, parse_value_text_in};
use open365_locale::Locale;

use super::sort::{
    ColorKey, Kind, SortState, TextOrder, cell_fill_key, cell_font_key, dxf_key, kind_of,
};
use super::{Cell, CellValue, Rect, Worksheet, format_number};
use crate::XlsxError;
use crate::refs::parse_range;
use crate::structure::range_a1;
use crate::styles::Styles;
use crate::xml::{Attr, ElementHead, RawXml, XmlElement};

/// How a date group is matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateTimeGrouping {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
}

/// One `dateGroupItem`: every field down to `grouping` must match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateGroup {
    pub year: u32,
    pub month: Option<u32>,
    pub day: Option<u32>,
    pub hour: Option<u32>,
    pub minute: Option<u32>,
    pub second: Option<u32>,
    pub grouping: DateTimeGrouping,
}

/// A list of values (the check boxes of the filter menu).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ValuesFilter {
    /// The shown texts that pass.
    pub items: Vec<String>,
    /// "(Blanks)" is ticked.
    pub blank: bool,
    pub calendar_type: Option<String>,
    pub date_groups: Vec<DateGroup>,
    /// The groups were read from Excel's `extLst` (and are written back there, as raw XML).
    pub date_groups_in_ext: bool,
}

/// The comparison of a custom filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FilterOperator {
    #[default]
    Equal,
    LessThan,
    LessThanOrEqual,
    NotEqual,
    GreaterThanOrEqual,
    GreaterThan,
}

impl FilterOperator {
    fn parse(s: Option<&str>) -> Self {
        match s {
            Some("lessThan") => Self::LessThan,
            Some("lessThanOrEqual") => Self::LessThanOrEqual,
            Some("notEqual") => Self::NotEqual,
            Some("greaterThanOrEqual") => Self::GreaterThanOrEqual,
            Some("greaterThan") => Self::GreaterThan,
            _ => Self::Equal,
        }
    }

    fn name(self) -> Option<&'static str> {
        match self {
            Self::Equal => None,
            Self::LessThan => Some("lessThan"),
            Self::LessThanOrEqual => Some("lessThanOrEqual"),
            Self::NotEqual => Some("notEqual"),
            Self::GreaterThanOrEqual => Some("greaterThanOrEqual"),
            Self::GreaterThan => Some("greaterThan"),
        }
    }
}

/// One comparison of `customFilters`.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomFilter {
    pub operator: FilterOperator,
    pub value: String,
}

/// One or two comparisons joined by AND or OR.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CustomFilters {
    pub and: bool,
    pub filters: Vec<CustomFilter>,
}

/// "Top 10" and its variants.
#[derive(Debug, Clone, PartialEq)]
pub struct Top10 {
    pub top: bool,
    pub percent: bool,
    pub value: f64,
    /// The threshold Excel stored the last time it applied the filter.
    pub filter_value: Option<f64>,
}

/// The kinds of `dynamicFilter`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynamicKind {
    Null,
    AboveAverage,
    BelowAverage,
    Tomorrow,
    Today,
    Yesterday,
    NextWeek,
    ThisWeek,
    LastWeek,
    NextMonth,
    ThisMonth,
    LastMonth,
    NextQuarter,
    ThisQuarter,
    LastQuarter,
    NextYear,
    ThisYear,
    LastYear,
    YearToDate,
    /// `Q1` to `Q4`.
    Quarter(u8),
    /// `M1` to `M12`.
    Month(u8),
}

const DYNAMIC_NAMES: [(&str, DynamicKind); 19] = [
    ("null", DynamicKind::Null),
    ("aboveAverage", DynamicKind::AboveAverage),
    ("belowAverage", DynamicKind::BelowAverage),
    ("tomorrow", DynamicKind::Tomorrow),
    ("today", DynamicKind::Today),
    ("yesterday", DynamicKind::Yesterday),
    ("nextWeek", DynamicKind::NextWeek),
    ("thisWeek", DynamicKind::ThisWeek),
    ("lastWeek", DynamicKind::LastWeek),
    ("nextMonth", DynamicKind::NextMonth),
    ("thisMonth", DynamicKind::ThisMonth),
    ("lastMonth", DynamicKind::LastMonth),
    ("nextQuarter", DynamicKind::NextQuarter),
    ("thisQuarter", DynamicKind::ThisQuarter),
    ("lastQuarter", DynamicKind::LastQuarter),
    ("nextYear", DynamicKind::NextYear),
    ("thisYear", DynamicKind::ThisYear),
    ("lastYear", DynamicKind::LastYear),
    ("yearToDate", DynamicKind::YearToDate),
];

impl DynamicKind {
    fn parse(s: &str) -> Option<Self> {
        if let Some((_, k)) = DYNAMIC_NAMES.iter().find(|(n, _)| *n == s) {
            return Some(*k);
        }
        if let Some(n) = s.strip_prefix('Q').and_then(|n| n.parse::<u8>().ok())
            && (1..=4).contains(&n)
        {
            return Some(Self::Quarter(n));
        }
        if let Some(n) = s.strip_prefix('M').and_then(|n| n.parse::<u8>().ok())
            && (1..=12).contains(&n)
        {
            return Some(Self::Month(n));
        }
        None
    }

    fn name(self) -> String {
        match self {
            Self::Quarter(n) => format!("Q{n}"),
            Self::Month(n) => format!("M{n}"),
            other => DYNAMIC_NAMES
                .iter()
                .find(|(_, k)| *k == other)
                .map_or_else(|| "null".to_owned(), |(n, _)| (*n).to_owned()),
        }
    }
}

/// A `dynamicFilter`.
#[derive(Debug, Clone, PartialEq)]
pub struct DynamicFilter {
    pub kind: DynamicKind,
    pub value: Option<f64>,
    pub max_value: Option<f64>,
}

/// A `colorFilter`: `dxf_id` names the differential format holding the colour.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorFilter {
    pub dxf_id: Option<u32>,
    /// True for the cell fill, false for the font colour.
    pub cell_color: bool,
}

/// An `iconFilter` (read and written, not evaluated).
#[derive(Debug, Clone, PartialEq)]
pub struct IconFilter {
    pub icon_set: String,
    pub icon_id: Option<u32>,
}

/// The criteria of one column.
#[derive(Debug, Clone, PartialEq)]
pub enum FilterCriteria {
    Values(ValuesFilter),
    Custom(CustomFilters),
    Top10(Top10),
    Dynamic(DynamicFilter),
    Color(ColorFilter),
    Icon(IconFilter),
}

/// One `filterColumn`.
#[derive(Debug, Clone, PartialEq)]
pub struct FilterColumn {
    /// 0-based column within the filter range.
    pub col_id: u32,
    pub hidden_button: bool,
    pub show_button: bool,
    pub criteria: Option<FilterCriteria>,
    ext: Option<RawXml>,
}

impl FilterColumn {
    pub fn new(col_id: u32, criteria: FilterCriteria) -> Self {
        Self {
            col_id,
            hidden_button: false,
            show_button: true,
            criteria: Some(criteria),
            ext: None,
        }
    }
}

/// The `autoFilter` of a worksheet.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoFilter {
    /// The filtered cells including the header row.
    pub range: Rect,
    pub columns: Vec<FilterColumn>,
    pub sort_state: Option<SortState>,
    other_attrs: Vec<Attr>,
    ext: Vec<RawXml>,
}

fn malformed(reason: &str) -> XlsxError {
    XlsxError::Malformed {
        part: "worksheet".to_owned(),
        reason: reason.to_owned(),
    }
}

fn flag(head: &ElementHead, name: &str, default: bool) -> bool {
    match head.attr(name).map(str::trim) {
        Some("1" | "true") => true,
        Some("0" | "false") => false,
        _ => default,
    }
}

fn float(head: &ElementHead, name: &str) -> Option<f64> {
    head.attr(name)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
}

fn uint(head: &ElementHead, name: &str) -> Option<u32> {
    head.attr(name).and_then(|v| v.trim().parse().ok())
}

fn parse_group(el: &XmlElement) -> Option<DateGroup> {
    let h = &el.head;
    Some(DateGroup {
        year: uint(h, "year")?,
        month: uint(h, "month"),
        day: uint(h, "day"),
        hour: uint(h, "hour"),
        minute: uint(h, "minute"),
        second: uint(h, "second"),
        grouping: match h.attr("dateTimeGrouping")? {
            "year" => DateTimeGrouping::Year,
            "month" => DateTimeGrouping::Month,
            "day" => DateTimeGrouping::Day,
            "hour" => DateTimeGrouping::Hour,
            "minute" => DateTimeGrouping::Minute,
            "second" => DateTimeGrouping::Second,
            _ => return None,
        },
    })
}

/// `dateGroupItem`s anywhere below `el` (Excel keeps some under `extLst/ext/filterColumn/filters`).
fn collect_groups(el: &XmlElement, out: &mut Vec<DateGroup>) {
    for c in el.elements() {
        if c.head.local_name() == "dateGroupItem" {
            out.extend(parse_group(c));
        } else {
            collect_groups(c, out);
        }
    }
}

impl AutoFilter {
    /// A filter over `range` with no criteria.
    pub fn new(range: Rect) -> Self {
        Self {
            range,
            columns: Vec::new(),
            sort_state: None,
            other_attrs: Vec::new(),
            ext: Vec::new(),
        }
    }

    /// Reads an `autoFilter` element.
    pub fn from_element(el: &XmlElement) -> Result<Self, XlsxError> {
        let range = el
            .head
            .attr("ref")
            .and_then(parse_range)
            .ok_or_else(|| malformed("autoFilter without a valid ref"))?;
        let mut f = Self::new(range);
        f.other_attrs = el
            .head
            .attrs
            .iter()
            .filter(|a| a.ns.is_some() || a.name != "ref")
            .cloned()
            .collect();
        for child in &el.children {
            let RawXml::Element(c) = child else { continue };
            if c.head.is_main("filterColumn") {
                f.columns.push(Self::parse_column(c)?);
            } else if c.head.is_main("sortState") {
                f.sort_state = Some(SortState::from_element(c)?);
            } else if c.head.local_name() == "extLst" {
                f.ext.push(child.clone());
            }
        }
        Ok(f)
    }

    fn parse_column(c: &XmlElement) -> Result<FilterColumn, XlsxError> {
        let col_id =
            uint(&c.head, "colId").ok_or_else(|| malformed("filterColumn without colId"))?;
        let mut col = FilterColumn {
            col_id,
            hidden_button: flag(&c.head, "hiddenButton", false),
            show_button: flag(&c.head, "showButton", true),
            criteria: None,
            ext: None,
        };
        for child in &c.children {
            let RawXml::Element(e) = child else { continue };
            let h = &e.head;
            let parsed = match h.local_name() {
                "filters" if h.is_main("filters") => {
                    let mut v = ValuesFilter {
                        blank: flag(h, "blank", false),
                        calendar_type: h.attr("calendarType").map(str::to_owned),
                        ..ValuesFilter::default()
                    };
                    for it in e.elements() {
                        if it.head.is_main("filter") {
                            if let Some(val) = it.head.attr("val") {
                                v.items.push(val.to_owned());
                            }
                        } else if it.head.is_main("dateGroupItem") {
                            v.date_groups.extend(parse_group(it));
                        }
                    }
                    Some(FilterCriteria::Values(v))
                }
                "customFilters" if h.is_main("customFilters") => {
                    let filters = e
                        .elements()
                        .filter(|f| f.head.is_main("customFilter"))
                        .map(|f| CustomFilter {
                            operator: FilterOperator::parse(f.head.attr("operator")),
                            value: f.head.attr("val").unwrap_or_default().to_owned(),
                        })
                        .collect();
                    Some(FilterCriteria::Custom(CustomFilters {
                        and: flag(h, "and", false),
                        filters,
                    }))
                }
                "top10" if h.is_main("top10") => Some(FilterCriteria::Top10(Top10 {
                    top: flag(h, "top", true),
                    percent: flag(h, "percent", false),
                    value: float(h, "val").ok_or_else(|| malformed("top10 without val"))?,
                    filter_value: float(h, "filterVal"),
                })),
                "dynamicFilter" if h.is_main("dynamicFilter") => {
                    Some(FilterCriteria::Dynamic(DynamicFilter {
                        kind: DynamicKind::parse(h.attr("type").unwrap_or("null"))
                            .ok_or_else(|| malformed("unknown dynamicFilter type"))?,
                        value: float(h, "val"),
                        max_value: float(h, "maxVal"),
                    }))
                }
                "colorFilter" if h.is_main("colorFilter") => {
                    Some(FilterCriteria::Color(ColorFilter {
                        dxf_id: uint(h, "dxfId"),
                        cell_color: flag(h, "cellColor", true),
                    }))
                }
                "iconFilter" if h.is_main("iconFilter") => Some(FilterCriteria::Icon(IconFilter {
                    icon_set: h.attr("iconSet").unwrap_or_default().to_owned(),
                    icon_id: uint(h, "iconId"),
                })),
                "extLst" => {
                    let mut groups = Vec::new();
                    collect_groups(e, &mut groups);
                    if !groups.is_empty() {
                        match &mut col.criteria {
                            Some(FilterCriteria::Values(v)) => v.date_groups.extend(groups),
                            None => {
                                col.criteria = Some(FilterCriteria::Values(ValuesFilter {
                                    date_groups: groups,
                                    date_groups_in_ext: true,
                                    ..ValuesFilter::default()
                                }));
                            }
                            Some(_) => {}
                        }
                    }
                    col.ext = Some(child.clone());
                    None
                }
                _ => None,
            };
            if parsed.is_some() {
                col.criteria = parsed;
            }
        }
        Ok(col)
    }

    /// The element, named like `like` (same namespace and prefix).
    pub fn to_element(&self, like: &ElementHead) -> XmlElement {
        let mut el = XmlElement {
            head: like.sibling("autoFilter"),
            children: Vec::new(),
        };
        el.head.set_attr("ref", &range_a1(self.range));
        el.head.attrs.extend(self.other_attrs.iter().cloned());
        for col in &self.columns {
            el.children
                .push(RawXml::Element(Self::column_element(col, like)));
        }
        if let Some(s) = &self.sort_state {
            el.children.push(RawXml::Element(s.to_element(like)));
        }
        el.children.extend(self.ext.iter().cloned());
        el
    }

    fn column_element(col: &FilterColumn, like: &ElementHead) -> XmlElement {
        let mut c = XmlElement {
            head: like.sibling("filterColumn"),
            children: Vec::new(),
        };
        c.head.set_attr("colId", &col.col_id.to_string());
        if col.hidden_button {
            c.head.set_attr("hiddenButton", "1");
        }
        if !col.show_button {
            c.head.set_attr("showButton", "0");
        }
        let mut child = |name: &str, f: &dyn Fn(&mut XmlElement)| {
            let mut e = XmlElement {
                head: like.sibling(name),
                children: Vec::new(),
            };
            f(&mut e);
            c.children.push(RawXml::Element(e));
        };
        let num = |v: f64| {
            let mut s = String::new();
            format_number(v, &mut s);
            s
        };
        match &col.criteria {
            None => {}
            Some(FilterCriteria::Values(v))
                if !(v.date_groups_in_ext && v.items.is_empty() && !v.blank) =>
            {
                child("filters", &|e| {
                    if v.blank {
                        e.head.set_attr("blank", "1");
                    }
                    if let Some(t) = &v.calendar_type {
                        e.head.set_attr("calendarType", t);
                    }
                    for item in &v.items {
                        let mut f = e.sibling("filter");
                        f.head.set_attr("val", item);
                        e.children.push(RawXml::Element(f));
                    }
                    if !v.date_groups_in_ext {
                        for g in &v.date_groups {
                            let mut d = e.sibling("dateGroupItem");
                            d.head.set_attr("year", &g.year.to_string());
                            for (n, x) in [
                                ("month", g.month),
                                ("day", g.day),
                                ("hour", g.hour),
                                ("minute", g.minute),
                                ("second", g.second),
                            ] {
                                if let Some(x) = x {
                                    d.head.set_attr(n, &x.to_string());
                                }
                            }
                            d.head.set_attr(
                                "dateTimeGrouping",
                                match g.grouping {
                                    DateTimeGrouping::Year => "year",
                                    DateTimeGrouping::Month => "month",
                                    DateTimeGrouping::Day => "day",
                                    DateTimeGrouping::Hour => "hour",
                                    DateTimeGrouping::Minute => "minute",
                                    DateTimeGrouping::Second => "second",
                                },
                            );
                            e.children.push(RawXml::Element(d));
                        }
                    }
                });
            }
            Some(FilterCriteria::Values(_)) => {}
            Some(FilterCriteria::Custom(cf)) => child("customFilters", &|e| {
                if cf.and {
                    e.head.set_attr("and", "1");
                }
                for f in &cf.filters {
                    let mut x = e.sibling("customFilter");
                    if let Some(op) = f.operator.name() {
                        x.head.set_attr("operator", op);
                    }
                    x.head.set_attr("val", &f.value);
                    e.children.push(RawXml::Element(x));
                }
            }),
            Some(FilterCriteria::Top10(t)) => child("top10", &|e| {
                if !t.top {
                    e.head.set_attr("top", "0");
                }
                if t.percent {
                    e.head.set_attr("percent", "1");
                }
                e.head.set_attr("val", &num(t.value));
                if let Some(v) = t.filter_value {
                    e.head.set_attr("filterVal", &num(v));
                }
            }),
            Some(FilterCriteria::Dynamic(d)) => child("dynamicFilter", &|e| {
                e.head.set_attr("type", &d.kind.name());
                if let Some(v) = d.value {
                    e.head.set_attr("val", &num(v));
                }
                if let Some(v) = d.max_value {
                    e.head.set_attr("maxVal", &num(v));
                }
            }),
            Some(FilterCriteria::Color(cf)) => child("colorFilter", &|e| {
                if let Some(id) = cf.dxf_id {
                    e.head.set_attr("dxfId", &id.to_string());
                }
                if !cf.cell_color {
                    e.head.set_attr("cellColor", "0");
                }
            }),
            Some(FilterCriteria::Icon(i)) => child("iconFilter", &|e| {
                e.head.set_attr("iconSet", &i.icon_set);
                if let Some(id) = i.icon_id {
                    e.head.set_attr("iconId", &id.to_string());
                }
            }),
        }
        if let Some(x) = &col.ext {
            c.children.push(x.clone());
        }
        c
    }
}

// ---- evaluation -----------------------------------------------------------------------------------------

/// What the evaluation needs besides the cells.
#[derive(Clone)]
pub struct FilterContext<'a> {
    /// Cell formats (for the shown text and the colours).
    pub styles: &'a Styles,
    /// Resolves `dxfId` (second argument: true for a font colour) when the differential formats are not in
    /// `styles` (a filter read from another workbook).
    pub dxf_color: Option<&'a dyn Fn(u32, bool) -> ColorKey>,
    /// Collation of text comparisons.
    pub locale: &'a Locale,
    /// Number and date formatting of the shown text.
    pub format_locale: Arc<FormatLocale>,
    /// Today as a date serial.
    pub today: f64,
    pub date1904: bool,
    /// First day of the week for the week filters, 0 = Sunday.
    pub first_weekday: u8,
}

impl<'a> FilterContext<'a> {
    pub fn new(styles: &'a Styles, locale: &'a Locale, today: f64) -> Self {
        Self {
            styles,
            dxf_color: None,
            locale,
            format_locale: Arc::new(FormatLocale::new(locale)),
            today,
            date1904: false,
            first_weekday: 0,
        }
    }
}

impl std::fmt::Debug for FilterContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilterContext")
            .field("today", &self.today)
            .finish_non_exhaustive()
    }
}

/// Which data rows pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterOutcome {
    pub hidden: Vec<u32>,
    pub visible: Vec<u32>,
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// `(year, month, day)` of a day serial, with Excel's 1900 leap-year quirk.
fn ymd(serial: i64, date1904: bool) -> (i64, u32, u32) {
    if date1904 {
        return civil_from_days(serial + days_from_civil(1904, 1, 1));
    }
    match serial {
        ..=59 => civil_from_days(serial + days_from_civil(1899, 12, 31)),
        60 => (1900, 2, 29),
        _ => civil_from_days(serial + days_from_civil(1899, 12, 30)),
    }
}

fn weekday(serial: i64) -> i64 {
    (serial - 1).rem_euclid(7)
}

fn matches_group(g: &DateGroup, serial: f64, date1904: bool) -> bool {
    if !serial.is_finite() || serial < 0.0 {
        return false;
    }
    let ms = (serial * 86_400_000.0).round() as i64;
    let day = ms.div_euclid(86_400_000);
    let secs = ms.rem_euclid(86_400_000) / 1000;
    let (y, m, d) = ymd(day, date1904);
    let (h, mi, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    let want = |have: i64, w: Option<u32>| w.is_none_or(|w| i64::from(w) == have);
    let level = match g.grouping {
        DateTimeGrouping::Year => 0,
        DateTimeGrouping::Month => 1,
        DateTimeGrouping::Day => 2,
        DateTimeGrouping::Hour => 3,
        DateTimeGrouping::Minute => 4,
        DateTimeGrouping::Second => 5,
    };
    y == i64::from(g.year)
        && (level < 1 || want(i64::from(m), g.month))
        && (level < 2 || want(i64::from(d), g.day))
        && (level < 3 || want(h, g.hour))
        && (level < 4 || want(mi, g.minute))
        && (level < 5 || want(s, g.second))
}

fn dynamic_date_match(kind: DynamicKind, serial: f64, ctx: &FilterContext<'_>) -> bool {
    if !serial.is_finite() || serial < 0.0 {
        return false;
    }
    let d = serial.floor() as i64;
    let t = ctx.today.floor() as i64;
    let (dy, dm, _) = ymd(d, ctx.date1904);
    let (ty, tm, _) = ymd(t, ctx.date1904);
    let week_start = t - (weekday(t) - i64::from(ctx.first_weekday)).rem_euclid(7);
    let month = |y: i64, m: u32| y * 12 + i64::from(m) - 1;
    let quarter = |y: i64, m: u32| y * 4 + i64::from((m - 1) / 3);
    match kind {
        DynamicKind::Today => d == t,
        DynamicKind::Yesterday => d == t - 1,
        DynamicKind::Tomorrow => d == t + 1,
        DynamicKind::ThisWeek => (week_start..week_start + 7).contains(&d),
        DynamicKind::LastWeek => (week_start - 7..week_start).contains(&d),
        DynamicKind::NextWeek => (week_start + 7..week_start + 14).contains(&d),
        DynamicKind::ThisMonth => month(dy, dm) == month(ty, tm),
        DynamicKind::LastMonth => month(dy, dm) == month(ty, tm) - 1,
        DynamicKind::NextMonth => month(dy, dm) == month(ty, tm) + 1,
        DynamicKind::ThisQuarter => quarter(dy, dm) == quarter(ty, tm),
        DynamicKind::LastQuarter => quarter(dy, dm) == quarter(ty, tm) - 1,
        DynamicKind::NextQuarter => quarter(dy, dm) == quarter(ty, tm) + 1,
        DynamicKind::ThisYear => dy == ty,
        DynamicKind::LastYear => dy == ty - 1,
        DynamicKind::NextYear => dy == ty + 1,
        DynamicKind::YearToDate => dy == ty && d <= t,
        DynamicKind::Quarter(n) => (dm - 1) / 3 + 1 == u32::from(n),
        DynamicKind::Month(n) => dm == u32::from(n),
        DynamicKind::Null | DynamicKind::AboveAverage | DynamicKind::BelowAverage => true,
    }
}

/// Wildcard match (`*`, `?`, `~` escape) of lower-cased text.
fn wildcard(pattern: &[char], text: &[char]) -> bool {
    let (mut p, mut t) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        let step = match pattern.get(p) {
            Some('*') => {
                star = Some((p, t));
                p += 1;
                continue;
            }
            Some('?') => true,
            Some('~') if p + 1 < pattern.len() && pattern[p + 1] == text[t] => {
                p += 1;
                true
            }
            Some('~') if p + 1 < pattern.len() => false,
            Some(c) => *c == text[t],
            None => false,
        };
        if step {
            p += 1;
            t += 1;
        } else if let Some((sp, st)) = star {
            p = sp + 1;
            t = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    while pattern.get(p) == Some(&'*') {
        p += 1;
    }
    p == pattern.len()
}

fn plain_number(s: &str) -> bool {
    s.trim().parse::<f64>().is_ok()
}

fn has_wildcard(s: &str) -> bool {
    s.contains(['*', '?', '~'])
}

/// One cell as the filter sees it.
struct View<'a> {
    kind: Kind<'a>,
    /// The text the cell shows; empty for a blank cell.
    shown: String,
    cell: Option<&'a Cell>,
}

fn calc_value(cell: &Cell) -> Value {
    match &cell.value {
        CellValue::Empty => Value::Empty,
        CellValue::Number(n) => Value::Number(*n),
        CellValue::Bool(b) => Value::Bool(*b),
        CellValue::Error(e) => Value::Error(ErrorKind::parse(e.code()).unwrap_or(ErrorKind::Value)),
        other => Value::Text(other.as_text().unwrap_or_default().to_owned()),
    }
}

fn view<'a>(cell: Option<&'a Cell>, ctx: &FilterContext<'_>) -> View<'a> {
    let kind = kind_of(cell);
    let shown = match (cell, kind) {
        (None, _) | (Some(_), Kind::Blank) => String::new(),
        (Some(c), Kind::Text(t)) => {
            let code = ctx.styles.number_format(c.style.unwrap_or(0));
            if code == "General" {
                t.to_owned()
            } else {
                format_value_in(&calc_value(c), code, ctx.date1904, &ctx.format_locale)
            }
        }
        (Some(c), _) => {
            let code = ctx.styles.number_format(c.style.unwrap_or(0));
            format_value_in(&calc_value(c), code, ctx.date1904, &ctx.format_locale)
        }
    };
    View { kind, shown, cell }
}

fn parse_needle(val: &str, date1904: bool) -> Option<f64> {
    let v = val.trim();
    v.parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .or_else(|| parse_value_text_in(v, date1904))
}

enum Matcher {
    Values {
        items: Vec<String>,
        blank: bool,
        groups: Vec<DateGroup>,
    },
    Custom {
        and: bool,
        filters: Vec<(FilterOperator, String, Option<f64>)>,
    },
    /// Numeric threshold: pass numbers `>=` (top) or `<=` (bottom) the value.
    Threshold {
        value: f64,
        at_least: bool,
    },
    Average {
        mean: f64,
        above: bool,
    },
    Date(DynamicKind),
    Color {
        wanted: ColorKey,
        cell: bool,
    },
    /// Icon filters are not evaluated: every row passes.
    Open,
}

impl AutoFilter {
    /// Evaluates the filter against `ws`: the rows below the header row that pass every column.
    pub fn evaluate(
        &self,
        ws: &Worksheet,
        ctx: &FilterContext<'_>,
    ) -> Result<FilterOutcome, XlsxError> {
        let (r1, c1, r2, _c2) = self.range;
        let order = TextOrder::new(ctx.locale, super::sort::SortMethod::Default, false)?;
        let rows: Vec<u32> = (r1 + 1..=r2).collect();
        let mut active: Vec<(u32, Matcher)> = Vec::new();
        for col in &self.columns {
            let Some(criteria) = &col.criteria else {
                continue;
            };
            let c = c1 + col.col_id;
            let numbers = || -> Vec<f64> {
                rows.iter()
                    .filter_map(|&r| match kind_of(ws.cell(r, c)) {
                        Kind::Number(n) => Some(n),
                        _ => None,
                    })
                    .collect()
            };
            let m = match criteria {
                FilterCriteria::Values(v) => Matcher::Values {
                    items: v.items.iter().map(|s| s.to_lowercase()).collect(),
                    blank: v.blank,
                    groups: v.date_groups.clone(),
                },
                FilterCriteria::Custom(cf) => Matcher::Custom {
                    and: cf.and,
                    filters: cf
                        .filters
                        .iter()
                        .map(|f| {
                            (
                                f.operator,
                                f.value.clone(),
                                parse_needle(&f.value, ctx.date1904),
                            )
                        })
                        .collect(),
                },
                FilterCriteria::Top10(t) => {
                    let mut nums = numbers();
                    nums.sort_by(|a, b| a.total_cmp(b));
                    if t.top {
                        nums.reverse();
                    }
                    if nums.is_empty() {
                        Matcher::Open
                    } else {
                        let n = nums.len();
                        let k = if t.percent {
                            ((n as f64 * t.value / 100.0).floor() as usize).max(1)
                        } else {
                            (t.value as usize).max(1)
                        }
                        .min(n);
                        Matcher::Threshold {
                            value: nums[k - 1],
                            at_least: t.top,
                        }
                    }
                }
                FilterCriteria::Dynamic(d) => match d.kind {
                    DynamicKind::AboveAverage | DynamicKind::BelowAverage => {
                        let nums = numbers();
                        let mean = if nums.is_empty() {
                            0.0
                        } else {
                            nums.iter().sum::<f64>() / nums.len() as f64
                        };
                        Matcher::Average {
                            mean,
                            above: d.kind == DynamicKind::AboveAverage,
                        }
                    }
                    DynamicKind::Null => Matcher::Open,
                    kind => Matcher::Date(kind),
                },
                FilterCriteria::Color(cf) => Matcher::Color {
                    wanted: cf
                        .dxf_id
                        .map_or(ColorKey::Automatic, |id| match ctx.dxf_color {
                            Some(f) => f(id, !cf.cell_color),
                            None => dxf_key(ctx.styles, id, !cf.cell_color),
                        }),
                    cell: cf.cell_color,
                },
                FilterCriteria::Icon(_) => Matcher::Open,
            };
            active.push((c, m));
        }
        let mut hidden = Vec::new();
        let mut visible = Vec::new();
        'rows: for &r in &rows {
            for (c, m) in &active {
                let v = view(ws.cell(r, *c), ctx);
                if !matches(m, &v, ctx, &order) {
                    hidden.push(r);
                    continue 'rows;
                }
            }
            visible.push(r);
        }
        Ok(FilterOutcome { hidden, visible })
    }
}

fn eq_match(needle: &str, num: Option<f64>, v: &View<'_>) -> bool {
    // Blank cells, empty text and text of spaces all count as blank.
    let blankish = matches!(v.kind, Kind::Blank) || v.shown.trim().is_empty();
    if needle.is_empty() {
        return blankish;
    }
    if blankish {
        return false;
    }
    let shown = v.shown.to_lowercase();
    if has_wildcard(needle) {
        let p: Vec<char> = needle.to_lowercase().chars().collect();
        let t: Vec<char> = shown.chars().collect();
        return wildcard(&p, &t);
    }
    match (num, v.kind) {
        (Some(n), Kind::Number(x)) => compare_numbers(x, n) == std::cmp::Ordering::Equal,
        (Some(_), Kind::Text(_)) => false,
        _ => shown == needle.to_lowercase(),
    }
}

fn custom_match(
    op: FilterOperator,
    needle: &str,
    num: Option<f64>,
    v: &View<'_>,
    order: &TextOrder,
) -> bool {
    use std::cmp::Ordering::{Equal, Greater, Less};
    match op {
        FilterOperator::Equal => eq_match(needle, num, v),
        FilterOperator::NotEqual => {
            if needle == " " {
                return !(matches!(v.kind, Kind::Blank) || v.shown.trim().is_empty());
            }
            !eq_match(needle, num, v)
        }
        _ => {
            let ord = match (num, v.kind) {
                (Some(n), Kind::Number(x)) => compare_numbers(x, n),
                (None, Kind::Text(t)) => order.compare_text(t, needle),
                _ => return false,
            };
            match op {
                FilterOperator::LessThan => ord == Less,
                FilterOperator::LessThanOrEqual => ord != Greater,
                FilterOperator::GreaterThan => ord == Greater,
                FilterOperator::GreaterThanOrEqual => ord != Less,
                _ => ord == Equal,
            }
        }
    }
}

fn matches(m: &Matcher, v: &View<'_>, ctx: &FilterContext<'_>, order: &TextOrder) -> bool {
    match m {
        Matcher::Open => true,
        Matcher::Values {
            items,
            blank,
            groups,
        } => {
            if matches!(v.kind, Kind::Blank) {
                return *blank;
            }
            if v.shown.trim().is_empty() {
                return *blank;
            }
            if let Kind::Number(n) = v.kind
                && groups.iter().any(|g| matches_group(g, n, ctx.date1904))
            {
                return true;
            }
            let shown = v.shown.to_lowercase();
            // Text that reads as a number is not the number the menu entry stands for.
            let text_number = matches!(v.kind, Kind::Text(_)) && plain_number(&shown);
            items
                .iter()
                .any(|i| *i == shown && !(text_number && plain_number(i)))
        }
        Matcher::Custom { and, filters } => {
            let mut results = filters
                .iter()
                .map(|(op, needle, num)| custom_match(*op, needle, *num, v, order));
            if *and {
                results.all(|r| r)
            } else {
                results.any(|r| r)
            }
        }
        Matcher::Threshold { value, at_least } => match v.kind {
            Kind::Number(n) => {
                let o = compare_numbers(n, *value);
                if *at_least {
                    o != std::cmp::Ordering::Less
                } else {
                    o != std::cmp::Ordering::Greater
                }
            }
            _ => false,
        },
        Matcher::Average { mean, above } => match v.kind {
            Kind::Number(n) => {
                if *above {
                    n > *mean
                } else {
                    n < *mean
                }
            }
            _ => false,
        },
        Matcher::Date(kind) => match v.kind {
            Kind::Number(n) => dynamic_date_match(*kind, n, ctx),
            _ => false,
        },
        Matcher::Color { wanted, cell } => {
            let xf = v.cell.and_then(|c| c.style);
            let have = if *cell {
                cell_fill_key(ctx.styles, xf)
            } else {
                cell_font_key(ctx.styles, xf)
            };
            have == *wanted
        }
    }
}

impl Worksheet {
    /// The worksheet-level `autoFilter`, when there is one.
    pub fn auto_filter(&self) -> Option<AutoFilter> {
        AutoFilter::from_element(self.root_element("autoFilter")?).ok()
    }

    /// Sets or removes the worksheet-level `autoFilter`. This does not change which rows are hidden; see
    /// [`Worksheet::apply_auto_filter`].
    pub fn set_auto_filter(&mut self, filter: Option<&AutoFilter>) {
        let el = filter.map(|f| f.to_element(&self.sheet_data_head));
        self.put_root_element("autoFilter", el);
    }

    /// Hides the rows of `filter` that do not pass and shows the ones that do (Excel's "Reapply"). Rows outside
    /// the filter range are left alone.
    pub fn apply_auto_filter(
        &mut self,
        filter: &AutoFilter,
        ctx: &FilterContext<'_>,
    ) -> Result<FilterOutcome, XlsxError> {
        let outcome = filter.evaluate(self, ctx)?;
        let shown = |ws: &mut Self, rows: &[u32], hide: bool| -> Result<(), XlsxError> {
            let mut i = 0;
            while i < rows.len() {
                let mut j = i;
                while j + 1 < rows.len() && rows[j + 1] == rows[j] + 1 {
                    j += 1;
                }
                if hide {
                    ws.hide_rows(rows[i], rows[j])?;
                } else {
                    ws.unhide_rows(rows[i], rows[j])?;
                }
                i = j + 1;
            }
            Ok(())
        };
        shown(self, &outcome.visible, false)?;
        shown(self, &outcome.hidden, true)?;
        Ok(outcome)
    }
}
