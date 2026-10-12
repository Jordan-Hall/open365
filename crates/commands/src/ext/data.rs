//! The Data tab's commands for Sheets: Sort, AutoFilter, Advanced Filter, Text to Columns,
//! Remove Duplicates, Data Validation, Consolidate, Goal Seek and the outline (Group, Subtotal).
//! Each runs as one undo step (the caller keeps a copy of the workbook).

use open365_locale::Locale;
use open365_xlsx::{
    AutoFilter, CellValue, CustomFilter, CustomFilters, DataValidation, DataValidations,
    DynamicFilter, DynamicKind, ErrorStyle, FilterColumn, FilterCriteria, FilterOperator,
    HeaderMode, SortKey, SortOrientation, SortSpec, Top10, ValidationOperator, ValidationType,
    ValuesFilter, Workbook,
};
use serde::{Deserialize, Serialize};

use super::xmlpatch::{self, Rows};
use crate::command::{CellEntry, CellRef};
use crate::error::CommandError;
use crate::find::{cell_display_text, cell_source_text};
use crate::format::RangeRef;
use crate::xlsx_state::XlsxState;

/// Whether the first line of a sort range is a header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortHeader {
    Yes,
    No,
    /// Excel's guess from the cell types and formats of the first two lines.
    #[default]
    Guess,
}

/// One level of the Sort dialog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SortLevel {
    /// Absolute 1-based column (row for a left-to-right sort) holding the key.
    pub index: u32,
    #[serde(default)]
    pub descending: bool,
    /// Custom list order (the items in order); `None` is "Smallest to Largest" / "A to Z".
    #[serde(default)]
    pub custom_list: Option<Vec<String>>,
}

/// A Sort dialog result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SortRequest {
    pub range: RangeRef,
    #[serde(default)]
    pub header: SortHeader,
    /// "Sort left to right" (the keys are rows).
    #[serde(default)]
    pub left_to_right: bool,
    #[serde(default)]
    pub case_sensitive: bool,
    pub levels: Vec<SortLevel>,
}

/// A comparison of a custom filter or an Advanced Filter condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cmp {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

impl Cmp {
    fn operator(self) -> FilterOperator {
        match self {
            Self::Equal => FilterOperator::Equal,
            Self::NotEqual => FilterOperator::NotEqual,
            Self::Less => FilterOperator::LessThan,
            Self::LessOrEqual => FilterOperator::LessThanOrEqual,
            Self::Greater => FilterOperator::GreaterThan,
            Self::GreaterOrEqual => FilterOperator::GreaterThanOrEqual,
        }
    }

    fn validation(self) -> ValidationOperator {
        match self {
            Self::Equal => ValidationOperator::Equal,
            Self::NotEqual => ValidationOperator::NotEqual,
            Self::Less => ValidationOperator::LessThan,
            Self::LessOrEqual => ValidationOperator::LessThanOrEqual,
            Self::Greater => ValidationOperator::GreaterThan,
            Self::GreaterOrEqual => ValidationOperator::GreaterThanOrEqual,
        }
    }
}

/// What one AutoFilter drop-down applies to its column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ColumnFilter {
    /// The check boxes: the checked items (as shown) and whether (Blanks) is checked.
    Values {
        items: Vec<String>,
        blank: bool,
    },
    /// Custom AutoFilter: one or two comparisons joined by And or Or.
    Custom {
        first: (Cmp, String),
        second: Option<(Cmp, String)>,
        and: bool,
    },
    /// Top 10: the top or bottom `count` items or percent.
    Top10 {
        top: bool,
        percent: bool,
        count: f64,
    },
    AboveAverage,
    BelowAverage,
}

/// Where an Advanced Filter puts its result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum FilterAction {
    InPlace,
    CopyTo { row: u32, col: u32 },
}

/// How Text to Columns splits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum SplitMode {
    Delimited {
        #[serde(default)]
        tab: bool,
        #[serde(default)]
        semicolon: bool,
        #[serde(default)]
        comma: bool,
        #[serde(default)]
        space: bool,
        #[serde(default)]
        other: Option<char>,
        /// "Treat consecutive delimiters as one".
        #[serde(default)]
        consecutive: bool,
        /// Text qualifier; Excel's default is the double quote.
        #[serde(default = "quote")]
        qualifier: Option<char>,
    },
    /// Break lines: character offsets where a new column starts.
    Fixed { breaks: Vec<usize> },
}

fn quote() -> Option<char> {
    Some('"')
}

/// Step 3 of the Text to Columns wizard, per column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnFormat {
    #[default]
    General,
    Text,
    /// "Do not import column (skip)".
    Skip,
}

/// The kinds of the Data Validation "Allow" box (Any value is `SetValidation { rule: None }`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AllowKind {
    WholeNumber,
    Decimal,
    List,
    Date,
    Time,
    TextLength,
    Custom,
}

/// "Between" and "not between" besides the six comparisons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataCmp {
    Between,
    NotBetween,
    Cmp(Cmp),
}

/// The Error Alert tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorAlert {
    /// `stop`, `warning` or `information`.
    pub style: String,
    pub title: String,
    pub message: String,
}

/// A Data Validation dialog result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationRule {
    pub allow: AllowKind,
    pub compare: DataCmp,
    /// Minimum / value / source / formula (a leading `=` is dropped; a list literal is `a,b,c`).
    pub value1: String,
    #[serde(default)]
    pub value2: Option<String>,
    #[serde(default = "yes")]
    pub ignore_blank: bool,
    #[serde(default = "yes")]
    pub in_cell_dropdown: bool,
    /// Input Message tab: title and message, shown when the cell is selected.
    #[serde(default)]
    pub input_message: Option<(String, String)>,
    /// Error Alert tab; `None` is the box unchecked.
    #[serde(default)]
    pub error_alert: Option<ErrorAlert>,
}

fn yes() -> bool {
    true
}

/// The Consolidate dialog's Function box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggregate {
    Sum,
    Count,
    Average,
    Max,
    Min,
    Product,
    CountNums,
    StdDev,
    StdDevP,
    Var,
    VarP,
}

impl Aggregate {
    /// The `SUBTOTAL` function number (also Subtotal's "Use function").
    #[must_use]
    pub fn subtotal_code(self) -> u32 {
        match self {
            Self::Average => 1,
            Self::CountNums => 2,
            Self::Count => 3,
            Self::Max => 4,
            Self::Min => 5,
            Self::Product => 6,
            Self::StdDev => 7,
            Self::StdDevP => 8,
            Self::Sum => 9,
            Self::Var => 10,
            Self::VarP => 11,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Sum => "Total",
            Self::Count | Self::CountNums => "Count",
            Self::Average => "Average",
            Self::Max => "Max",
            Self::Min => "Min",
            Self::Product => "Product",
            Self::StdDev => "StdDev",
            Self::StdDevP => "StdDevp",
            Self::Var => "Var",
            Self::VarP => "Varp",
        }
    }

    /// The result over `values`; `nonblank` is the number of non-empty cells (Count).
    fn apply(self, values: &[f64], nonblank: usize) -> Option<f64> {
        let n = values.len() as f64;
        let mean = || values.iter().sum::<f64>() / n;
        let ss = |m: f64| values.iter().map(|v| (v - m) * (v - m)).sum::<f64>();
        match self {
            Self::Sum => Some(values.iter().sum()),
            Self::Count => Some(nonblank as f64),
            Self::CountNums => Some(n),
            Self::Average => (!values.is_empty()).then(mean),
            Self::Max => values.iter().copied().reduce(f64::max),
            Self::Min => values.iter().copied().reduce(f64::min),
            Self::Product => (!values.is_empty()).then(|| values.iter().product()),
            Self::StdDev => (values.len() > 1).then(|| (ss(mean()) / (n - 1.0)).sqrt()),
            Self::StdDevP => (!values.is_empty()).then(|| (ss(mean()) / n).sqrt()),
            Self::Var => (values.len() > 1).then(|| ss(mean()) / (n - 1.0)),
            Self::VarP => (!values.is_empty()).then(|| ss(mean()) / n),
        }
    }
}

/// One reference of the Consolidate dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRange {
    pub sheet: usize,
    pub range: RangeRef,
}

/// The Subtotal dialog result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubtotalRequest {
    /// The data, header line included.
    pub range: RangeRef,
    /// "At each change in": the column.
    pub at_change: u32,
    pub function: Aggregate,
    /// "Add subtotal to": the columns.
    pub columns: Vec<u32>,
    #[serde(default = "yes")]
    pub replace_current: bool,
    #[serde(default = "yes")]
    pub summary_below: bool,
}

/// Every Data tab edit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DataOp {
    Sort(SortRequest),
    /// Filter on over `range` (header line first), or off with `None`.
    AutoFilter {
        range: Option<RangeRef>,
    },
    /// Sets or clears (`None`) the criteria of one column (absolute 1-based) and filters.
    FilterColumn {
        column: u32,
        filter: Option<ColumnFilter>,
    },
    /// Data > Clear: removes every criterion and shows all rows; the arrows stay.
    ClearFilter,
    /// Data > Reapply.
    ReapplyFilter,
    AdvancedFilter {
        list: RangeRef,
        criteria: RangeRef,
        action: FilterAction,
        #[serde(default)]
        unique_only: bool,
    },
    TextToColumns {
        /// One column of cells.
        range: RangeRef,
        split: SplitMode,
        #[serde(default)]
        columns: Vec<ColumnFormat>,
        /// Destination top-left; the first cell of `range` when `None`.
        #[serde(default)]
        destination: Option<(u32, u32)>,
    },
    RemoveDuplicates {
        range: RangeRef,
        has_headers: bool,
        /// Absolute columns compared.
        columns: Vec<u32>,
    },
    /// Sets the rule of `range` (replacing others there) or, with `None`, clears it.
    SetValidation {
        range: RangeRef,
        rule: Option<ValidationRule>,
    },
    Consolidate {
        function: Aggregate,
        sources: Vec<SourceRange>,
        use_top_row: bool,
        use_left_column: bool,
        row: u32,
        col: u32,
    },
    /// What-If Analysis > Goal Seek.
    GoalSeek {
        set_row: u32,
        set_col: u32,
        to_value: f64,
        change_row: u32,
        change_col: u32,
    },
    /// Group rows `first..=last` (one more outline level).
    GroupRows {
        first: u32,
        last: u32,
    },
    UngroupRows {
        first: u32,
        last: u32,
    },
    /// Hide or show the detail rows `first..=last`; the summary row next to them is marked.
    ShowDetailRows {
        first: u32,
        last: u32,
        show: bool,
        summary_below: bool,
    },
    GroupCols {
        first: u32,
        last: u32,
    },
    UngroupCols {
        first: u32,
        last: u32,
    },
    /// Hide or show the detail columns `first..=last`; the summary column next to them is marked.
    ShowDetailCols {
        first: u32,
        last: u32,
        show: bool,
        summary_right: bool,
    },
    Subtotal(SubtotalRequest),
    RemoveSubtotals {
        range: RangeRef,
    },
}

fn today_serial() -> f64 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    (secs / 86_400) as f64 + 25_569.0
}

fn text(wb: &Workbook, sheet: usize, r: u32, c: u32) -> String {
    cell_display_text(wb, sheet, r, c)
}

fn number(wb: &Workbook, sheet: usize, r: u32, c: u32) -> Option<f64> {
    match wb.cell(sheet, r, c).map(|cell| &cell.value) {
        Some(CellValue::Number(n)) => Some(*n),
        _ => None,
    }
}

fn blank(wb: &Workbook, sheet: usize, r: u32, c: u32) -> bool {
    wb.cell(sheet, r, c)
        .is_none_or(|cell| matches!(cell.value, CellValue::Empty) && cell.formula.is_none())
}

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::Format(msg.into())
}

fn merge(into: &mut Vec<CellRef>, more: Vec<CellRef>) {
    for c in more {
        if !into.contains(&c) {
            into.push(c);
        }
    }
}

impl DataOp {
    /// Whether the operation overwrites or discards content that exists.
    pub(crate) fn destroys(&self) -> bool {
        matches!(
            self,
            Self::RemoveDuplicates { .. }
                | Self::RemoveSubtotals { .. }
                | Self::TextToColumns { .. }
                | Self::Consolidate { .. }
        )
    }
}

/// Runs `op` on `sheet`.
pub(crate) fn run(
    x: &mut XlsxState,
    sheet: usize,
    op: &DataOp,
    notes: &mut Vec<String>,
) -> Result<Vec<CellRef>, CommandError> {
    match op {
        DataOp::Sort(req) => sort(x, sheet, req),
        DataOp::AutoFilter { range } => auto_filter(x, sheet, *range),
        DataOp::FilterColumn { column, filter } => {
            filter_column(x, sheet, *column, filter.as_ref())
        }
        DataOp::ClearFilter => {
            let mut filter = x
                .workbook
                .sheet(sheet)
                .and_then(open365_xlsx::Worksheet::auto_filter)
                .ok_or_else(|| bad("the sheet has no filter"))?;
            filter.columns.clear();
            x.workbook.set_auto_filter(sheet, Some(&filter))?;
            reapply(x, sheet)
        }
        DataOp::ReapplyFilter => reapply(x, sheet),
        DataOp::AdvancedFilter {
            list,
            criteria,
            action,
            unique_only,
        } => advanced_filter(x, sheet, *list, *criteria, *action, *unique_only, notes),
        DataOp::TextToColumns {
            range,
            split,
            columns,
            destination,
        } => text_to_columns(x, sheet, *range, split, columns, *destination),
        DataOp::RemoveDuplicates {
            range,
            has_headers,
            columns,
        } => remove_duplicates(x, sheet, *range, *has_headers, columns, notes),
        DataOp::SetValidation { range, rule } => set_validation(x, sheet, *range, rule.as_ref()),
        DataOp::Consolidate {
            function,
            sources,
            use_top_row,
            use_left_column,
            row,
            col,
        } => consolidate(
            x,
            sheet,
            *function,
            sources,
            *use_top_row,
            *use_left_column,
            (*row, *col),
        ),
        DataOp::GoalSeek {
            set_row,
            set_col,
            to_value,
            change_row,
            change_col,
        } => goal_seek(
            x,
            sheet,
            (*set_row, *set_col),
            *to_value,
            (*change_row, *change_col),
            notes,
        ),
        DataOp::GroupRows { first, last } => outline(x, sheet, *first, *last, 1),
        DataOp::UngroupRows { first, last } => outline(x, sheet, *first, *last, -1),
        DataOp::ShowDetailRows {
            first,
            last,
            show,
            summary_below,
        } => show_detail(x, sheet, *first, *last, *show, *summary_below),
        DataOp::GroupCols { first, last } => outline_cols(x, sheet, *first, *last, 1),
        DataOp::UngroupCols { first, last } => outline_cols(x, sheet, *first, *last, -1),
        DataOp::ShowDetailCols {
            first,
            last,
            show,
            summary_right,
        } => show_detail_cols(x, sheet, *first, *last, *show, *summary_right),
        DataOp::Subtotal(req) => subtotal(x, sheet, req, notes),
        DataOp::RemoveSubtotals { range } => remove_subtotals(x, sheet, *range),
    }
}

// ---- sort -------------------------------------------------------------------------------

/// Re-enters the formulas in `rect` so the calculation engine and the cached results follow
/// cells that moved.
fn refresh_formulas(
    x: &mut XlsxState,
    sheet: usize,
    rect: (u32, u32, u32, u32),
) -> Result<Vec<CellRef>, CommandError> {
    let wb = x.workbook.clone();
    x.restore_workbook(wb);
    let (r1, c1, r2, c2) = rect;
    let formulas: Vec<(u32, u32, String)> = x
        .workbook
        .iter_cells(sheet)
        .filter(|&(r, c, _)| (r1..=r2).contains(&r) && (c1..=c2).contains(&c))
        .filter_map(|(r, c, _)| x.workbook.formula_text(sheet, r, c).map(|f| (r, c, f)))
        .collect();
    let mut changed = Vec::new();
    for (r, c, f) in formulas {
        merge(&mut changed, x.set_formula(sheet, r, c, &f)?);
    }
    Ok(changed)
}

fn sort(x: &mut XlsxState, sheet: usize, req: &SortRequest) -> Result<Vec<CellRef>, CommandError> {
    let spec = SortSpec {
        range: (
            req.range.first_row,
            req.range.first_col,
            req.range.last_row,
            req.range.last_col,
        ),
        header: match req.header {
            SortHeader::Yes => HeaderMode::Yes,
            SortHeader::No => HeaderMode::No,
            SortHeader::Guess => HeaderMode::Guess,
        },
        orientation: if req.left_to_right {
            SortOrientation::LeftToRight
        } else {
            SortOrientation::TopToBottom
        },
        case_sensitive: req.case_sensitive,
        method: open365_xlsx::SortMethod::Default,
        keys: req
            .levels
            .iter()
            .map(|l| {
                let mut key = SortKey::value(l.index, l.descending);
                key.custom_list.clone_from(&l.custom_list);
                key
            })
            .collect(),
    };
    x.workbook.sort_range(sheet, &spec, &Locale::en_us())?;
    let mut changed = vec![CellRef {
        sheet,
        row: req.range.first_row,
        col: req.range.first_col,
    }];
    merge(&mut changed, refresh_formulas(x, sheet, spec.range)?);
    Ok(changed)
}

// ---- filter -----------------------------------------------------------------------------

fn reapply(x: &mut XlsxState, sheet: usize) -> Result<Vec<CellRef>, CommandError> {
    x.workbook
        .apply_auto_filter(sheet, today_serial(), &Locale::en_us())?;
    let wb = x.workbook.clone();
    x.restore_workbook(wb);
    Ok(Vec::new())
}

fn auto_filter(
    x: &mut XlsxState,
    sheet: usize,
    range: Option<RangeRef>,
) -> Result<Vec<CellRef>, CommandError> {
    match range {
        Some(r) => {
            if r.first_row >= r.last_row && r.first_col >= r.last_col && r.first_row == r.last_row {
                // One header cell with no data still filters, as in Excel.
            }
            let filter = AutoFilter::new((r.first_row, r.first_col, r.last_row, r.last_col));
            x.workbook.set_auto_filter(sheet, Some(&filter))?;
        }
        None => {
            if let Some(f) = x
                .workbook
                .sheet(sheet)
                .and_then(open365_xlsx::Worksheet::auto_filter)
            {
                x.workbook.set_auto_filter(sheet, None)?;
                x.workbook.unhide_rows(sheet, f.range.0, f.range.2)?;
            }
        }
    }
    Ok(Vec::new())
}

fn filter_column(
    x: &mut XlsxState,
    sheet: usize,
    column: u32,
    filter: Option<&ColumnFilter>,
) -> Result<Vec<CellRef>, CommandError> {
    let mut af = x
        .workbook
        .sheet(sheet)
        .and_then(open365_xlsx::Worksheet::auto_filter)
        .ok_or_else(|| bad("the sheet has no filter"))?;
    if column < af.range.1 || column > af.range.3 {
        return Err(bad("the column is outside the filter range"));
    }
    let col_id = column - af.range.1;
    af.columns.retain(|c| c.col_id != col_id);
    if let Some(f) = filter {
        let criteria = match f {
            ColumnFilter::Values { items, blank } => FilterCriteria::Values(ValuesFilter {
                items: items.clone(),
                blank: *blank,
                ..ValuesFilter::default()
            }),
            ColumnFilter::Custom { first, second, and } => {
                let mut filters = vec![CustomFilter {
                    operator: first.0.operator(),
                    value: first.1.clone(),
                }];
                if let Some(s) = second {
                    filters.push(CustomFilter {
                        operator: s.0.operator(),
                        value: s.1.clone(),
                    });
                }
                FilterCriteria::Custom(CustomFilters { and: *and, filters })
            }
            ColumnFilter::Top10 {
                top,
                percent,
                count,
            } => FilterCriteria::Top10(Top10 {
                top: *top,
                percent: *percent,
                value: *count,
                filter_value: None,
            }),
            ColumnFilter::AboveAverage => FilterCriteria::Dynamic(DynamicFilter {
                kind: DynamicKind::AboveAverage,
                value: None,
                max_value: None,
            }),
            ColumnFilter::BelowAverage => FilterCriteria::Dynamic(DynamicFilter {
                kind: DynamicKind::BelowAverage,
                value: None,
                max_value: None,
            }),
        };
        af.columns.push(FilterColumn::new(col_id, criteria));
        af.columns.sort_by_key(|c| c.col_id);
    }
    x.workbook.set_auto_filter(sheet, Some(&af))?;
    reapply(x, sheet)
}

/// One Advanced Filter / criteria-range condition against a cell.
fn criterion_matches(criterion: &str, shown: &str, numeric: Option<f64>) -> bool {
    let c = criterion.trim();
    let (op, operand) = ["<=", ">=", "<>", "<", ">", "="]
        .iter()
        .find_map(|p| c.strip_prefix(p).map(|rest| (*p, rest)))
        .unwrap_or(("", c));
    let operand = operand.trim_start();
    if operand.is_empty() {
        return match op {
            "=" => shown.is_empty(),
            "<>" => !shown.is_empty(),
            _ => false,
        };
    }
    if let Ok(n) = operand.parse::<f64>() {
        let Some(v) = numeric else {
            return op == "<>";
        };
        return match op {
            "<" => v < n,
            "<=" => v <= n,
            ">" => v > n,
            ">=" => v >= n,
            "<>" => (v - n).abs() > f64::EPSILON * n.abs().max(1.0),
            _ => (v - n).abs() <= f64::EPSILON * n.abs().max(1.0),
        };
    }
    let (s, o) = (shown.to_lowercase(), operand.to_lowercase());
    match op {
        "<" => !s.is_empty() && s < o,
        "<=" => !s.is_empty() && s <= o,
        ">" => !s.is_empty() && s > o,
        ">=" => !s.is_empty() && s >= o,
        "<>" => !wildcard(&o, &s, false),
        "=" => wildcard(&o, &s, true),
        // No operator: "begins with" (a number criterion was handled above).
        _ => wildcard(&o, &s, false),
    }
}

/// `*` and `?` wildcards; `whole` demands the pattern match all of `s`, else only a prefix.
fn wildcard(pattern: &str, s: &str, whole: bool) -> bool {
    fn go(p: &[char], s: &[char], whole: bool) -> bool {
        match p.split_first() {
            None => !whole || s.is_empty(),
            Some(('*', rest)) => (0..=s.len()).any(|i| go(rest, &s[i..], whole)),
            Some(('?', rest)) => !s.is_empty() && go(rest, &s[1..], whole),
            Some((c, rest)) => s.first() == Some(c) && go(rest, &s[1..], whole),
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = s.chars().collect();
    go(&p, &s, whole)
}

fn advanced_filter(
    x: &mut XlsxState,
    sheet: usize,
    list: RangeRef,
    criteria: RangeRef,
    action: FilterAction,
    unique_only: bool,
    notes: &mut Vec<String>,
) -> Result<Vec<CellRef>, CommandError> {
    if list.last_row <= list.first_row && criteria.last_row < criteria.first_row {
        return Err(bad("the list range needs a header row"));
    }
    let wb = &x.workbook;
    let headers: Vec<String> = (list.first_col..=list.last_col)
        .map(|c| text(wb, sheet, list.first_row, c).to_lowercase())
        .collect();
    // Column of the list for each criteria column (None: no such header).
    let crit_cols: Vec<(u32, Option<u32>)> = (criteria.first_col..=criteria.last_col)
        .map(|cc| {
            let h = text(wb, sheet, criteria.first_row, cc).to_lowercase();
            let at = (!h.is_empty())
                .then(|| headers.iter().position(|x| *x == h))
                .flatten();
            (cc, at.map(|i| list.first_col + i as u32))
        })
        .collect();
    let crit_rows: Vec<u32> = (criteria.first_row + 1..=criteria.last_row).collect();
    let mut matching = Vec::new();
    let mut seen: Vec<Vec<String>> = Vec::new();
    for r in list.first_row + 1..=list.last_row {
        let ok = crit_rows.is_empty()
            || crit_rows.iter().any(|&cr| {
                crit_cols.iter().all(|&(cc, lc)| {
                    let crit = text(wb, sheet, cr, cc);
                    if crit.trim().is_empty() {
                        return true;
                    }
                    let Some(lc) = lc else {
                        return false;
                    };
                    criterion_matches(&crit, &text(wb, sheet, r, lc), number(wb, sheet, r, lc))
                })
            });
        if !ok {
            continue;
        }
        if unique_only {
            let key: Vec<String> = (list.first_col..=list.last_col)
                .map(|c| text(wb, sheet, r, c).to_lowercase())
                .collect();
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
        }
        matching.push(r);
    }
    match action {
        FilterAction::InPlace => {
            let all: Vec<u32> = (list.first_row + 1..=list.last_row).collect();
            for r in all {
                if matching.contains(&r) {
                    x.workbook.unhide_rows(sheet, r, r)?;
                } else {
                    x.workbook.hide_rows(sheet, r, r)?;
                }
            }
            let wb = x.workbook.clone();
            x.restore_workbook(wb);
            notes.push(format!(
                "{} of {} records found",
                matching.len(),
                list.last_row - list.first_row
            ));
            Ok(Vec::new())
        }
        FilterAction::CopyTo { row, col } => {
            let mut changed = Vec::new();
            let mut lines: Vec<Vec<String>> = vec![
                (list.first_col..=list.last_col)
                    .map(|c| cell_source_text(&x.workbook, sheet, list.first_row, c))
                    .collect(),
            ];
            for &r in &matching {
                lines.push(
                    (list.first_col..=list.last_col)
                        .map(|c| cell_source_text(&x.workbook, sheet, r, c))
                        .collect(),
                );
            }
            for (dr, line) in lines.iter().enumerate() {
                for (dc, t) in line.iter().enumerate() {
                    merge(
                        &mut changed,
                        x.enter(sheet, row + dr as u32, col + dc as u32, t)?,
                    );
                }
            }
            notes.push(format!("{} records copied", matching.len()));
            Ok(changed)
        }
    }
}

// ---- text to columns --------------------------------------------------------------------

fn split_line(line: &str, split: &SplitMode) -> Vec<String> {
    match split {
        SplitMode::Fixed { breaks } => {
            let chars: Vec<char> = line.chars().collect();
            let mut cuts: Vec<usize> = breaks.iter().copied().filter(|&b| b > 0).collect();
            cuts.sort_unstable();
            cuts.dedup();
            let mut out = Vec::new();
            let mut at = 0;
            for cut in cuts {
                let cut = cut.min(chars.len());
                if cut > at {
                    out.push(chars[at..cut].iter().collect::<String>());
                    at = cut;
                } else if cut == at && at > 0 {
                    continue;
                }
            }
            out.push(chars[at.min(chars.len())..].iter().collect());
            out.iter().map(|s| s.trim_end().to_owned()).collect()
        }
        SplitMode::Delimited {
            tab,
            semicolon,
            comma,
            space,
            other,
            consecutive,
            qualifier,
        } => {
            let is_delim = |c: char| {
                (*tab && c == '\t')
                    || (*semicolon && c == ';')
                    || (*comma && c == ',')
                    || (*space && c == ' ')
                    || *other == Some(c)
            };
            let chars: Vec<char> = line.chars().collect();
            let mut out = Vec::new();
            let mut cur = String::new();
            let mut i = 0;
            let mut field_start = true;
            while i < chars.len() {
                let c = chars[i];
                if field_start && Some(c) == *qualifier {
                    // A qualified field runs to the closing qualifier; a doubled one is literal.
                    let q = c;
                    i += 1;
                    while i < chars.len() {
                        if chars[i] == q {
                            if chars.get(i + 1) == Some(&q) {
                                cur.push(q);
                                i += 2;
                                continue;
                            }
                            i += 1;
                            break;
                        }
                        cur.push(chars[i]);
                        i += 1;
                    }
                    field_start = false;
                    continue;
                }
                if is_delim(c) {
                    out.push(std::mem::take(&mut cur));
                    i += 1;
                    if *consecutive {
                        while i < chars.len() && is_delim(chars[i]) {
                            i += 1;
                        }
                    }
                    field_start = true;
                    continue;
                }
                cur.push(c);
                field_start = false;
                i += 1;
            }
            out.push(cur);
            out
        }
    }
}

fn text_to_columns(
    x: &mut XlsxState,
    sheet: usize,
    range: RangeRef,
    split: &SplitMode,
    formats: &[ColumnFormat],
    destination: Option<(u32, u32)>,
) -> Result<Vec<CellRef>, CommandError> {
    if range.first_col != range.last_col {
        return Err(bad("select a single column of data"));
    }
    let (dest_row, dest_col) = destination.unwrap_or((range.first_row, range.first_col));
    let mut parts: Vec<Vec<String>> = Vec::new();
    for r in range.first_row..=range.last_row {
        parts.push(split_line(
            &text(&x.workbook, sheet, r, range.first_col),
            split,
        ));
    }
    let mut changed = Vec::new();
    for (i, line) in parts.iter().enumerate() {
        let row = dest_row + i as u32;
        let mut col = dest_col;
        for (k, piece) in line.iter().enumerate() {
            let format = formats.get(k).copied().unwrap_or_default();
            if format == ColumnFormat::Skip {
                continue;
            }
            let entry = match format {
                ColumnFormat::Text => CellEntry::Text(piece.clone()),
                _ => CellEntry::from_user_text(piece),
            };
            if piece.is_empty() {
                merge(
                    &mut changed,
                    x.set_value(sheet, row, col, &CellEntry::Empty)?,
                );
            } else {
                merge(&mut changed, x.set_value(sheet, row, col, &entry)?);
            }
            col += 1;
        }
    }
    Ok(changed)
}

// ---- remove duplicates ------------------------------------------------------------------

fn remove_duplicates(
    x: &mut XlsxState,
    sheet: usize,
    range: RangeRef,
    has_headers: bool,
    columns: &[u32],
    notes: &mut Vec<String>,
) -> Result<Vec<CellRef>, CommandError> {
    let first = range.first_row + u32::from(has_headers);
    if first > range.last_row || columns.is_empty() {
        return Err(bad("nothing to compare"));
    }
    if columns
        .iter()
        .any(|c| *c < range.first_col || *c > range.last_col)
    {
        return Err(bad("a column is outside the range"));
    }
    let wb = &x.workbook;
    let mut seen: Vec<Vec<String>> = Vec::new();
    let mut keep: Vec<u32> = Vec::new();
    for r in first..=range.last_row {
        let key: Vec<String> = columns
            .iter()
            .map(|&c| text(wb, sheet, r, c).to_lowercase())
            .collect();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        keep.push(r);
    }
    let total = range.last_row - first + 1;
    let removed = total as usize - keep.len();
    if removed > 0 {
        let rows: Vec<Vec<String>> = keep
            .iter()
            .map(|&r| {
                (range.first_col..=range.last_col)
                    .map(|c| cell_source_text(wb, sheet, r, c))
                    .collect()
            })
            .collect();
        let mut changed = Vec::new();
        for (i, line) in rows.iter().enumerate() {
            for (k, t) in line.iter().enumerate() {
                merge(
                    &mut changed,
                    x.enter(sheet, first + i as u32, range.first_col + k as u32, t)?,
                );
            }
        }
        for r in first + keep.len() as u32..=range.last_row {
            for c in range.first_col..=range.last_col {
                if !blank(&x.workbook, sheet, r, c) {
                    merge(&mut changed, x.set_value(sheet, r, c, &CellEntry::Empty)?);
                }
            }
        }
        notes.push(format!(
            "{removed} duplicate values found and removed; {} unique values remain.",
            keep.len()
        ));
        return Ok(changed);
    }
    notes.push(format!(
        "No duplicate values found; {} unique values remain.",
        keep.len()
    ));
    Ok(Vec::new())
}

// ---- data validation --------------------------------------------------------------------

type Rect4 = (u32, u32, u32, u32);

/// `a` minus `b`, as up to four rectangles.
fn subtract(a: Rect4, b: Rect4) -> Vec<Rect4> {
    let (r1, c1, r2, c2) = a;
    let (br1, bc1, br2, bc2) = b;
    if br1 > r2 || br2 < r1 || bc1 > c2 || bc2 < c1 {
        return vec![a];
    }
    let mut out = Vec::new();
    if br1 > r1 {
        out.push((r1, c1, br1 - 1, c2));
    }
    if br2 < r2 {
        out.push((br2 + 1, c1, r2, c2));
    }
    let (mr1, mr2) = (r1.max(br1), r2.min(br2));
    if bc1 > c1 {
        out.push((mr1, c1, mr2, bc1 - 1));
    }
    if bc2 < c2 {
        out.push((mr1, bc2 + 1, mr2, c2));
    }
    out
}

#[allow(clippy::field_reassign_with_default)]
fn set_validation(
    x: &mut XlsxState,
    sheet: usize,
    range: RangeRef,
    rule: Option<&ValidationRule>,
) -> Result<Vec<CellRef>, CommandError> {
    let target: Rect4 = (
        range.first_row,
        range.first_col,
        range.last_row,
        range.last_col,
    );
    let mut all = x
        .workbook
        .sheet(sheet)
        .and_then(open365_xlsx::Worksheet::data_validations)
        .unwrap_or_default();
    let mut items = Vec::new();
    for mut item in std::mem::take(&mut all.items) {
        let kept: Vec<Rect4> = item
            .sqref
            .iter()
            .flat_map(|&r| subtract(r, target))
            .collect();
        if !kept.is_empty() {
            item.sqref = kept;
            items.push(item);
        }
    }
    if let Some(rule) = rule {
        let kind = match rule.allow {
            AllowKind::WholeNumber => ValidationType::Whole,
            AllowKind::Decimal => ValidationType::Decimal,
            AllowKind::List => ValidationType::List,
            AllowKind::Date => ValidationType::Date,
            AllowKind::Time => ValidationType::Time,
            AllowKind::TextLength => ValidationType::TextLength,
            AllowKind::Custom => ValidationType::Custom,
        };
        let operator = match rule.compare {
            DataCmp::Between => ValidationOperator::Between,
            DataCmp::NotBetween => ValidationOperator::NotBetween,
            DataCmp::Cmp(c) => c.validation(),
        };
        let two = matches!(rule.compare, DataCmp::Between | DataCmp::NotBetween)
            && !matches!(rule.allow, AllowKind::List | AllowKind::Custom);
        let strip = |s: &str| s.trim().strip_prefix('=').unwrap_or(s.trim()).to_owned();
        let formula1 = if rule.allow == AllowKind::List && !rule.value1.trim().starts_with('=') {
            format!("\"{}\"", rule.value1)
        } else {
            strip(&rule.value1)
        };
        let mut dv = DataValidation::default();
        dv.kind = kind;
        dv.operator = operator;
        dv.allow_blank = rule.ignore_blank;
        dv.hide_drop_down = !rule.in_cell_dropdown;
        dv.show_input_message = rule.input_message.is_some();
        dv.show_error_message = rule.error_alert.is_some();
        dv.prompt_title = rule.input_message.as_ref().map(|m| m.0.clone());
        dv.prompt = rule.input_message.as_ref().map(|m| m.1.clone());
        dv.error_style = match rule.error_alert.as_ref().map(|e| e.style.as_str()) {
            Some("warning") => ErrorStyle::Warning,
            Some("information") => ErrorStyle::Information,
            _ => ErrorStyle::Stop,
        };
        dv.error_title = rule.error_alert.as_ref().map(|e| e.title.clone());
        dv.error = rule.error_alert.as_ref().map(|e| e.message.clone());
        dv.sqref = vec![target];
        dv.formula1 = Some(formula1);
        dv.formula2 = two.then(|| strip(rule.value2.as_deref().unwrap_or("")));
        items.push(dv);
    }
    all.items = items;
    let set: Option<&DataValidations> =
        (!all.items.is_empty() || !all.ext_items.is_empty()).then_some(&all);
    x.workbook.set_data_validations(sheet, set)?;
    Ok(Vec::new())
}

// ---- consolidate ------------------------------------------------------------------------

fn consolidate(
    x: &mut XlsxState,
    sheet: usize,
    function: Aggregate,
    sources: &[SourceRange],
    use_top: bool,
    use_left: bool,
    dest: (u32, u32),
) -> Result<Vec<CellRef>, CommandError> {
    if sources.is_empty() {
        return Err(bad("add at least one reference"));
    }
    // (row key, col key) -> numbers, count of non-empty cells.
    let mut row_keys: Vec<String> = Vec::new();
    let mut col_keys: Vec<String> = Vec::new();
    let mut cells: Vec<((usize, usize), Vec<f64>, usize)> = Vec::new();
    let wb = &x.workbook;
    for src in sources {
        let r = src.range;
        let (r0, c0) = (
            r.first_row + u32::from(use_top),
            r.first_col + u32::from(use_left),
        );
        for row in r0..=r.last_row {
            let rk = if use_left {
                text(wb, src.sheet, row, r.first_col)
            } else {
                format!("#{}", row - r0)
            };
            if use_left && rk.is_empty() {
                continue;
            }
            let ri = row_keys.iter().position(|k| *k == rk).unwrap_or_else(|| {
                row_keys.push(rk);
                row_keys.len() - 1
            });
            for col in c0..=r.last_col {
                let ck = if use_top {
                    text(wb, src.sheet, r.first_row, col)
                } else {
                    format!("#{}", col - c0)
                };
                if use_top && ck.is_empty() {
                    continue;
                }
                let ci = col_keys.iter().position(|k| *k == ck).unwrap_or_else(|| {
                    col_keys.push(ck);
                    col_keys.len() - 1
                });
                let at = cells.iter().position(|(k, ..)| *k == (ri, ci));
                let at = at.unwrap_or_else(|| {
                    cells.push(((ri, ci), Vec::new(), 0));
                    cells.len() - 1
                });
                if let Some(n) = number(wb, src.sheet, row, col) {
                    cells[at].1.push(n);
                }
                if !blank(wb, src.sheet, row, col) {
                    cells[at].2 += 1;
                }
            }
        }
    }
    let (row_off, col_off) = (u32::from(use_top), u32::from(use_left));
    let mut changed = Vec::new();
    if use_top {
        for (i, k) in col_keys.iter().enumerate() {
            merge(
                &mut changed,
                x.set_value(
                    sheet,
                    dest.0,
                    dest.1 + col_off + i as u32,
                    &CellEntry::Text(k.clone()),
                )?,
            );
        }
    }
    if use_left {
        for (i, k) in row_keys.iter().enumerate() {
            merge(
                &mut changed,
                x.set_value(
                    sheet,
                    dest.0 + row_off + i as u32,
                    dest.1,
                    &CellEntry::Text(k.clone()),
                )?,
            );
        }
    }
    for ((ri, ci), values, nonblank) in &cells {
        if let Some(v) = function.apply(values, *nonblank) {
            merge(
                &mut changed,
                x.set_value(
                    sheet,
                    dest.0 + row_off + *ri as u32,
                    dest.1 + col_off + *ci as u32,
                    &CellEntry::Number(v),
                )?,
            );
        }
    }
    Ok(changed)
}

// ---- goal seek --------------------------------------------------------------------------

fn goal_seek(
    x: &mut XlsxState,
    sheet: usize,
    set: (u32, u32),
    target: f64,
    change: (u32, u32),
    notes: &mut Vec<String>,
) -> Result<Vec<CellRef>, CommandError> {
    if x.workbook.formula_text(sheet, set.0, set.1).is_none() {
        return Err(bad("Cell must contain a formula."));
    }
    if x.workbook.formula_text(sheet, change.0, change.1).is_some() {
        return Err(bad("Cell must contain a value, not a formula."));
    }
    let mut changed = Vec::new();
    let eval =
        |x: &mut XlsxState, v: f64, changed: &mut Vec<CellRef>| -> Result<f64, CommandError> {
            merge(
                changed,
                x.set_value(sheet, change.0, change.1, &CellEntry::Number(v))?,
            );
            number(&x.workbook, sheet, set.0, set.1)
                .ok_or_else(|| bad("The target cell does not give a number."))
        };
    let x0 = number(&x.workbook, sheet, change.0, change.1).unwrap_or(0.0);
    let mut a = x0;
    let mut fa = eval(x, a, &mut changed)? - target;
    let tol = 1e-9 * target.abs().max(1.0);
    let mut b = if a == 0.0 { 1.0 } else { a * 1.1 };
    let mut found = fa.abs() <= tol;
    let mut fb = fa;
    let mut iterations = 0;
    while !found && iterations < 200 {
        iterations += 1;
        fb = eval(x, b, &mut changed)? - target;
        if fb.abs() <= tol {
            a = b;
            found = true;
            break;
        }
        let slope = (fb - fa) / (b - a);
        let next = if slope.is_finite() && slope != 0.0 {
            b - fb / slope
        } else {
            b + (b - a).abs().max(1.0)
        };
        a = b;
        fa = fb;
        b = next;
        if !b.is_finite() {
            break;
        }
    }
    if !found && fb.abs() > 0.001 {
        return Err(bad("Goal Seeking may not have found a solution."));
    }
    let value = if found { a } else { b };
    let reached = eval(x, value, &mut changed)?;
    notes.push(format!(
        "Goal Seeking with Cell {} found a solution. Target value: {target} Current value: {reached}",
        open365_xlsx::cell_ref_string(set.0, set.1)
    ));
    Ok(changed)
}

// ---- outline ----------------------------------------------------------------------------

fn edit_rows(
    x: &mut XlsxState,
    sheet: usize,
    f: impl FnOnce(&mut Rows),
    summary_below: Option<bool>,
) -> Result<(), CommandError> {
    let part = xmlpatch::sheet_part_name(&x.workbook, sheet)?;
    let wb = xmlpatch::rewrite(&x.workbook, |pkg| {
        let xml = xmlpatch::part_text(pkg, &part)?;
        let mut rows = Rows::parse(&xml).ok_or_else(|| bad("the worksheet has no sheetData"))?;
        f(&mut rows);
        let mut out = rows.render();
        xmlpatch::set_outline_level_row(&mut out, rows.max_level());
        if let Some(below) = summary_below {
            set_summary_below(&mut out, below);
        }
        xmlpatch::set_part_text(pkg, &part, out)
    })?;
    x.restore_workbook(wb);
    Ok(())
}

fn set_summary_below(xml: &mut String, below: bool) {
    if below {
        return;
    }
    let pr = "<outlinePr summaryBelow=\"0\"/>";
    if let Some(at) = xmlpatch::find_open(xml, "sheetPr") {
        let end = xml[at..].find('>').map_or(0, |i| at + i + 1);
        if xml[at..end].ends_with("/>") {
            xml.replace_range(end - 2..end, &format!(">{pr}</sheetPr>"));
        } else if xmlpatch::find_open(xml, "outlinePr").is_none() {
            let child = ["pageSetUpPr"]
                .iter()
                .filter_map(|n| xmlpatch::find_open(xml, n))
                .min();
            xml.insert_str(child.unwrap_or(end), pr);
        }
    } else if let Some(ws) = xml.find("<worksheet") {
        let end = ws + xml[ws..].find('>').map_or(0, |i| i + 1);
        xml.insert_str(end, &format!("<sheetPr>{pr}</sheetPr>"));
    }
}

fn outline(
    x: &mut XlsxState,
    sheet: usize,
    first: u32,
    last: u32,
    delta: i32,
) -> Result<Vec<CellRef>, CommandError> {
    if first == 0 || last < first || last > open365_xlsx::MAX_ROW {
        return Err(bad("invalid rows"));
    }
    if delta > 0 && x_max_level(&x.workbook, sheet, first, last) >= 7 {
        return Err(bad("A group can have at most 8 levels."));
    }
    edit_rows(
        x,
        sheet,
        |rows| {
            for r in first..=last {
                let level = rows.level(r) as i32 + delta;
                if delta < 0 && rows.level(r) == 0 {
                    continue;
                }
                let level = level.clamp(0, 7);
                let tag = rows.row_mut(r);
                xmlpatch::set_attr(
                    tag,
                    "outlineLevel",
                    (level > 0).then(|| level.to_string()).as_deref(),
                );
                if level == 0 {
                    xmlpatch::set_attr(tag, "hidden", None);
                    xmlpatch::set_attr(tag, "collapsed", None);
                }
            }
        },
        None,
    )?;
    Ok(Vec::new())
}

fn x_max_level(wb: &Workbook, sheet: usize, first: u32, last: u32) -> u32 {
    wb.sheet(sheet).map_or(0, |ws| {
        ws.rows()
            .iter()
            .filter(|r| (first..=last).contains(&r.r))
            .filter_map(|r| {
                r.attrs()
                    .iter()
                    .find(|a| a.name == "outlineLevel")
                    .and_then(|a| a.value.parse().ok())
            })
            .max()
            .unwrap_or(0)
    })
}

fn show_detail(
    x: &mut XlsxState,
    sheet: usize,
    first: u32,
    last: u32,
    show: bool,
    summary_below: bool,
) -> Result<Vec<CellRef>, CommandError> {
    edit_rows(
        x,
        sheet,
        |rows| {
            for r in first..=last {
                let tag = rows.row_mut(r);
                xmlpatch::set_attr(tag, "hidden", (!show).then_some("1"));
            }
            let summary = if summary_below {
                last + 1
            } else {
                first.saturating_sub(1)
            };
            if summary > 0 {
                let tag = rows.row_mut(summary);
                xmlpatch::set_attr(tag, "collapsed", (!show).then_some("1"));
            }
        },
        None,
    )?;
    Ok(Vec::new())
}

#[allow(clippy::type_complexity)]
fn edit_cols(
    x: &mut XlsxState,
    sheet: usize,
    first: u32,
    last: u32,
    f: &dyn Fn(&mut String),
    marker: Option<(u32, &dyn Fn(&mut String))>,
) -> Result<(), CommandError> {
    let part = xmlpatch::sheet_part_name(&x.workbook, sheet)?;
    let wb = xmlpatch::rewrite(&x.workbook, |pkg| {
        let mut xml = xmlpatch::part_text(pkg, &part)?;
        let (range, mut entries) = xmlpatch::parse_cols(&xml);
        xmlpatch::adjust_cols(&mut entries, first, last, f);
        if let Some((col, g)) = marker {
            xmlpatch::adjust_cols(&mut entries, col, col, g);
        }
        let level = xmlpatch::max_col_level(&entries);
        xmlpatch::write_cols(&mut xml, range, &entries);
        xmlpatch::set_outline_level(&mut xml, "outlineLevelCol", level);
        xmlpatch::set_part_text(pkg, &part, xml)
    })?;
    x.restore_workbook(wb);
    Ok(())
}

fn outline_cols(
    x: &mut XlsxState,
    sheet: usize,
    first: u32,
    last: u32,
    delta: i32,
) -> Result<Vec<CellRef>, CommandError> {
    if first == 0 || last < first || last > open365_xlsx::MAX_COL {
        return Err(bad("invalid columns"));
    }
    edit_cols(
        x,
        sheet,
        first,
        last,
        &|tag| {
            let now: i32 = xmlpatch::attr(tag, "outlineLevel")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let level = (now + delta).clamp(0, 7);
            xmlpatch::set_attr(
                tag,
                "outlineLevel",
                (level > 0).then(|| level.to_string()).as_deref(),
            );
            if level == 0 {
                xmlpatch::set_attr(tag, "hidden", None);
                xmlpatch::set_attr(tag, "collapsed", None);
            }
        },
        None,
    )?;
    Ok(Vec::new())
}

fn show_detail_cols(
    x: &mut XlsxState,
    sheet: usize,
    first: u32,
    last: u32,
    show: bool,
    summary_right: bool,
) -> Result<Vec<CellRef>, CommandError> {
    let summary = if summary_right {
        last + 1
    } else {
        first.saturating_sub(1)
    };
    let collapsed = move |tag: &mut String| {
        xmlpatch::set_attr(tag, "collapsed", (!show).then_some("1"));
    };
    edit_cols(
        x,
        sheet,
        first,
        last,
        &|tag| xmlpatch::set_attr(tag, "hidden", (!show).then_some("1")),
        (summary > 0).then_some((summary, &collapsed as &dyn Fn(&mut String))),
    )?;
    Ok(Vec::new())
}

// ---- subtotal ---------------------------------------------------------------------------

fn insert_row(x: &mut XlsxState, sheet: usize, at: u32) -> Result<(), CommandError> {
    let (s, edit) = crate::structure::edit_of(&crate::command::Command::InsertRows {
        sheet,
        at,
        count: 1,
    })
    .ok_or_else(|| bad("cannot insert a row"))?;
    x.edit_structure(s, edit)?;
    Ok(())
}

fn delete_row(x: &mut XlsxState, sheet: usize, at: u32) -> Result<(), CommandError> {
    let (s, edit) = crate::structure::edit_of(&crate::command::Command::DeleteRows {
        sheet,
        at,
        count: 1,
    })
    .ok_or_else(|| bad("cannot delete a row"))?;
    x.edit_structure(s, edit)?;
    Ok(())
}

fn is_subtotal_row(wb: &Workbook, sheet: usize, row: u32, c1: u32, c2: u32) -> bool {
    (c1..=c2).any(|c| {
        wb.formula_text(sheet, row, c)
            .is_some_and(|f| f.trim_start().to_ascii_uppercase().starts_with("SUBTOTAL("))
    })
}

fn remove_subtotals(
    x: &mut XlsxState,
    sheet: usize,
    range: RangeRef,
) -> Result<Vec<CellRef>, CommandError> {
    let mut last = range.last_row;
    for r in (range.first_row..=range.last_row).rev() {
        if is_subtotal_row(&x.workbook, sheet, r, range.first_col, range.last_col) {
            delete_row(x, sheet, r)?;
            last -= 1;
        }
    }
    let (first, last) = (range.first_row, last.max(range.first_row));
    // The whole sheet's outline of these rows goes.
    edit_rows(
        x,
        sheet,
        |rows| {
            for (r, tag) in &mut rows.rows {
                if *r >= first && *r <= last + 64 {
                    xmlpatch::set_attr(tag, "outlineLevel", None);
                    xmlpatch::set_attr(tag, "collapsed", None);
                }
            }
        },
        None,
    )?;
    Ok(vec![CellRef {
        sheet,
        row: first,
        col: range.first_col,
    }])
}

fn subtotal(
    x: &mut XlsxState,
    sheet: usize,
    req: &SubtotalRequest,
    notes: &mut Vec<String>,
) -> Result<Vec<CellRef>, CommandError> {
    let mut range = req.range;
    if req.replace_current {
        remove_subtotals(x, sheet, range)?;
        // Rows were deleted: recompute the last row from the remaining data.
        let mut r = range.last_row;
        while r > range.first_row
            && (range.first_col..=range.last_col).all(|c| blank(&x.workbook, sheet, r, c))
        {
            r -= 1;
        }
        range.last_row = r;
    }
    if req.columns.is_empty() {
        return Err(bad("choose a column to subtotal"));
    }
    let key_col = req.at_change;
    let (first, last) = (range.first_row + 1, range.last_row);
    if last < first {
        return Err(bad("no data to subtotal"));
    }
    // Groups of equal key (text as shown, ignoring case), in order.
    let mut groups: Vec<(u32, u32, String)> = Vec::new();
    for r in first..=last {
        let k = text(&x.workbook, sheet, r, key_col);
        match groups.last_mut() {
            Some((_, end, key)) if key.to_lowercase() == k.to_lowercase() => *end = r,
            _ => groups.push((r, r, k)),
        }
    }
    let label = req.function.label();
    let code = req.function.subtotal_code();
    let cols_a1 = |c: u32| open365_xlsx::col_letters(c);
    let mut changed = Vec::new();
    let mut shift = 0u32;
    // Grand total first (bottom or top), then groups bottom to top so earlier rows stay put.
    if req.summary_below {
        insert_row(x, sheet, last + 1)?;
        let grand_at = last + 1;
        for &c in &req.columns {
            let f = format!("=SUBTOTAL({code},{0}{first}:{0}{1})", cols_a1(c), last);
            merge(&mut changed, x.enter(sheet, grand_at, c, &f)?);
        }
        merge(
            &mut changed,
            x.enter(sheet, grand_at, key_col, &format!("Grand {label}"))?,
        );
    }
    for (s, e, key) in groups.iter().rev() {
        let (s, e) = (*s, *e);
        let name = if label == "Total" {
            format!("{key} Total")
        } else {
            format!("{key} {label}")
        };
        if req.summary_below {
            insert_row(x, sheet, e + 1)?;
            for &c in &req.columns {
                let f = format!("=SUBTOTAL({code},{0}{s}:{0}{e})", cols_a1(c));
                merge(&mut changed, x.enter(sheet, e + 1, c, &f)?);
            }
            merge(&mut changed, x.enter(sheet, e + 1, key_col, &name)?);
        } else {
            insert_row(x, sheet, s)?;
            for &c in &req.columns {
                let f = format!("=SUBTOTAL({code},{0}{1}:{0}{2})", cols_a1(c), s + 1, e + 1);
                merge(&mut changed, x.enter(sheet, s, c, &f)?);
            }
            merge(&mut changed, x.enter(sheet, s, key_col, &name)?);
        }
        shift += 1;
    }
    if !req.summary_below {
        // Grand total directly under the header.
        insert_row(x, sheet, first)?;
        let end = last + shift;
        for &c in &req.columns {
            let f = format!(
                "=SUBTOTAL({code},{0}{1}:{0}{2})",
                cols_a1(c),
                first + 1,
                end + 1
            );
            merge(&mut changed, x.enter(sheet, first, c, &f)?);
        }
        merge(
            &mut changed,
            x.enter(sheet, first, key_col, &format!("Grand {label}"))?,
        );
    }
    // Outline: detail rows 2, subtotal rows 1, grand total 0.
    let n = groups.len() as u32;
    let total_last = last + n + 1;
    let (subtotal_set, grand_at): (Vec<u32>, u32) = if req.summary_below {
        (
            groups
                .iter()
                .enumerate()
                .map(|(i, g)| g.1 + 1 + i as u32)
                .collect(),
            last + n + 1,
        )
    } else {
        (
            groups
                .iter()
                .enumerate()
                .map(|(i, g)| g.0 + i as u32 + 1)
                .collect(),
            first,
        )
    };
    let below = req.summary_below;
    edit_rows(
        x,
        sheet,
        |rows| {
            for r in first..=total_last {
                if r == grand_at {
                    continue;
                }
                let level = if subtotal_set.contains(&r) { "1" } else { "2" };
                let tag = rows.row_mut(r);
                xmlpatch::set_attr(tag, "outlineLevel", Some(level));
            }
        },
        Some(below),
    )?;
    notes.push(format!("{} subtotal rows added", groups.len() + 1));
    Ok(changed)
}
