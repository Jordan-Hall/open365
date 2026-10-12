//! Sheets Insert and Formulas tabs, and sheet management: tables, defined names, hyperlinks, notes,
//! charts, pivot tables, calculation options, conditional formats, sheet operations, Insert/Delete
//! cells and Calculate Now/Sheet. Each is one undo step (the caller keeps a copy of the workbook).

use open365_calc::{ErrorKind, Value};
use open365_opc::{PartName, TargetMode};
use open365_xlsx::{
    CalcMode, CellError, CellIsOp, CellValue, CfKind, CfRuleSpec, CfSpec, Cfvo, CfvoKind,
    ChartAnchor, Color, ColorSource, DataBarAxis, DataBarDirection, DataBarExt, Dxf, DxfFill,
    DxfFont, FillPattern, HeaderChoice, HyperlinkSpec, HyperlinkTarget, NameSource, NameSpec,
    NoteSpec, PivotSpec, Rgb, SheetKind, SheetState, ShiftDirection, TabColor, TableSpec,
    TableStyleChoice, TimePeriod, Underline, Workbook,
};
use serde::{Deserialize, Serialize};

use super::review::{self, ReviewOp};
use crate::command::CellRef;
use crate::error::CommandError;
use crate::format::{ColorSpec, RangeRef};
use crate::xlsx_state::{XlsxState, build_engine};

fn yes() -> bool {
    true
}

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::Format(msg.into())
}

// ---- requests ---------------------------------------------------------------------------------

/// Insert > Table and Home > Format as Table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableRequest {
    pub range: RangeRef,
    /// `Table1`, `Table2` ... when absent.
    #[serde(default)]
    pub name: Option<String>,
    /// "My table has headers"; Excel's guess when absent.
    #[serde(default)]
    pub has_headers: Option<bool>,
    /// A table style name; `TableStyleMedium2` when absent, no style when empty.
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub totals_row: bool,
    #[serde(default = "yes")]
    pub filter_buttons: bool,
    #[serde(default)]
    pub first_column: bool,
    #[serde(default)]
    pub last_column: bool,
    #[serde(default = "yes")]
    pub banded_rows: bool,
    #[serde(default)]
    pub banded_columns: bool,
}

/// A defined name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NameRequest {
    pub name: String,
    /// The sheet the name is scoped to; the workbook when absent.
    #[serde(default)]
    pub scope: Option<usize>,
    /// Refers to, with or without the leading `=`.
    pub refers_to: String,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub hidden: bool,
}

/// Where a hyperlink points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LinkKind {
    Url {
        address: String,
    },
    Place {
        reference: String,
    },
    Email {
        address: String,
        #[serde(default)]
        subject: Option<String>,
    },
}

/// Insert > Link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HyperlinkRequest {
    pub range: RangeRef,
    pub target: LinkKind,
    /// Text to display. As in Excel it replaces what the first cell holds; without it an empty cell shows the
    /// address and a cell with content keeps it.
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub screen_tip: Option<String>,
}

/// Review > New Note.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoteRequest {
    pub row: u32,
    pub col: u32,
    pub author: String,
    pub text: String,
    /// Start with the author's name in bold (what New Note does).
    #[serde(default = "yes")]
    pub author_prefix: bool,
}

/// How a conditional-format threshold is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointKind {
    Num,
    Percent,
    Percentile,
    Min,
    Max,
    Formula,
    AutoMin,
    AutoMax,
}

/// One threshold of a colour scale, data bar or icon set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CfPoint {
    pub kind: PointKind,
    #[serde(default)]
    pub value: Option<String>,
    /// Greater-or-equal (icon sets): a value on the threshold counts as above it.
    #[serde(default = "yes")]
    pub gte: bool,
}

impl CfPoint {
    fn cfvo(&self) -> Cfvo {
        Cfvo {
            kind: match self.kind {
                PointKind::Num => CfvoKind::Num,
                PointKind::Percent => CfvoKind::Percent,
                PointKind::Percentile => CfvoKind::Percentile,
                PointKind::Min => CfvoKind::Min,
                PointKind::Max => CfvoKind::Max,
                PointKind::Formula => CfvoKind::Formula,
                PointKind::AutoMin => CfvoKind::AutoMin,
                PointKind::AutoMax => CfvoKind::AutoMax,
            },
            val: self.value.clone(),
            gte: self.gte,
        }
    }
}

/// The comparison of a "cell value" rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellCompare {
    Between,
    NotBetween,
    Equal,
    NotEqual,
    Greater,
    Less,
    GreaterOrEqual,
    LessOrEqual,
}

/// The period of a "dates occurring" rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatePeriod {
    Today,
    Yesterday,
    Tomorrow,
    Last7Days,
    ThisWeek,
    LastWeek,
    NextWeek,
    ThisMonth,
    LastMonth,
    NextMonth,
}

/// What a conditional-format rule tests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CfRuleKind {
    CellValue {
        compare: CellCompare,
        formulas: Vec<String>,
    },
    Formula {
        formula: String,
    },
    Top10 {
        rank: u32,
        #[serde(default)]
        percent: bool,
        #[serde(default)]
        bottom: bool,
    },
    AboveAverage {
        #[serde(default = "yes")]
        above: bool,
        #[serde(default)]
        include_equal: bool,
        #[serde(default)]
        std_dev: u32,
    },
    DuplicateValues,
    UniqueValues,
    ContainsText {
        text: String,
    },
    NotContainsText {
        text: String,
    },
    BeginsWith {
        text: String,
    },
    EndsWith {
        text: String,
    },
    ContainsBlanks,
    NotContainsBlanks,
    ContainsErrors,
    NotContainsErrors,
    DateOccurring {
        period: DatePeriod,
    },
    ColorScale {
        points: Vec<CfPoint>,
        colors: Vec<ColorSpec>,
    },
    DataBar {
        min: CfPoint,
        max: CfPoint,
        color: ColorSpec,
        #[serde(default = "yes")]
        show_value: bool,
        #[serde(default = "yes")]
        gradient: bool,
        #[serde(default)]
        border: bool,
        #[serde(default)]
        negative_color: Option<ColorSpec>,
        /// `automatic`, `midpoint` or `none`.
        #[serde(default)]
        axis: Option<String>,
        /// `context`, `left_to_right` or `right_to_left`.
        #[serde(default)]
        direction: Option<String>,
    },
    IconSet {
        name: String,
        points: Vec<CfPoint>,
        #[serde(default)]
        reverse: bool,
        #[serde(default = "yes")]
        show_value: bool,
    },
}

/// The look a rule applies.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CfFormat {
    #[serde(default)]
    pub bold: Option<bool>,
    #[serde(default)]
    pub italic: Option<bool>,
    #[serde(default)]
    pub underline: Option<bool>,
    #[serde(default)]
    pub strikethrough: Option<bool>,
    #[serde(default)]
    pub font_color: Option<ColorSpec>,
    #[serde(default)]
    pub fill_color: Option<ColorSpec>,
    #[serde(default)]
    pub number_format: Option<String>,
}

/// One rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CfRuleRequest {
    #[serde(flatten)]
    pub kind: CfRuleKind,
    #[serde(default)]
    pub format: Option<CfFormat>,
    #[serde(default)]
    pub stop_if_true: bool,
}

/// A conditional format: the cells and the rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CfRequest {
    pub ranges: Vec<RangeRef>,
    pub rules: Vec<CfRuleRequest>,
}

/// Formulas > Calculation Options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalcModeChoice {
    Automatic,
    AutomaticExceptTables,
    Manual,
}

/// Visibility of a sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SheetVisibility {
    Visible,
    Hidden,
    VeryHidden,
}

/// Which way Insert Cells and Delete Cells shift their neighbours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellShift {
    Right,
    Down,
    Left,
    Up,
}

/// Every Insert tab, Formulas tab and sheet-management edit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ObjectsOp {
    /// Adds a package part with its content type.
    AddPart {
        name: String,
        content_type: String,
        data: Vec<u8>,
    },
    /// Adds a relationship from a part (the package when `source` is absent).
    AddRelationship {
        #[serde(default)]
        source: Option<String>,
        rel_type: String,
        target: String,
        #[serde(default)]
        external: bool,
    },
    CreateTable(TableRequest),
    DefineName(NameRequest),
    /// Name Manager > Edit.
    EditName {
        name: String,
        #[serde(default)]
        scope: Option<usize>,
        to: NameRequest,
    },
    DeleteName {
        name: String,
        #[serde(default)]
        scope: Option<usize>,
    },
    /// Formulas > Create from Selection.
    CreateNamesFromSelection {
        range: RangeRef,
        #[serde(default)]
        top: bool,
        #[serde(default)]
        left: bool,
        #[serde(default)]
        bottom: bool,
        #[serde(default)]
        right: bool,
        #[serde(default)]
        replace: bool,
    },
    SetHyperlink(HyperlinkRequest),
    RemoveHyperlink {
        row: u32,
        col: u32,
    },
    SetNote(NoteRequest),
    DeleteNote {
        row: u32,
        col: u32,
    },
    /// Inserts a chart (a `chartSpace` part) over the cells of `range`.
    InsertChart {
        chart_xml: String,
        range: RangeRef,
        #[serde(default)]
        name: Option<String>,
    },
    /// Adds a pivot table from the parts the pivot crate produced.
    AddPivotTable {
        cache_definition: Vec<u8>,
        #[serde(default)]
        cache_records: Option<Vec<u8>>,
        table_definition: Vec<u8>,
    },
    SetCalculation {
        #[serde(default)]
        mode: Option<CalcModeChoice>,
        #[serde(default)]
        full_calc_on_load: Option<bool>,
        #[serde(default)]
        calc_on_save: Option<bool>,
        #[serde(default)]
        iterate: Option<bool>,
        #[serde(default)]
        iterate_count: Option<u32>,
        #[serde(default)]
        iterate_delta: Option<f64>,
        #[serde(default)]
        precision_as_displayed: Option<bool>,
    },
    AddConditionalFormat(CfRequest),
    ReplaceConditionalFormat {
        index: usize,
        format: CfRequest,
    },
    DeleteConditionalFormat {
        index: usize,
    },
    /// Clear Rules from Selected Cells.
    ClearConditionalFormats {
        range: RangeRef,
    },
    /// Inserts a blank sheet before `at` (the end when absent).
    AddSheet {
        #[serde(default)]
        at: Option<usize>,
        #[serde(default)]
        name: Option<String>,
    },
    DeleteSheet,
    RenameSheet {
        name: String,
    },
    /// Moves the sheet so it ends at index `to`.
    MoveSheet {
        to: usize,
    },
    CopySheet {
        #[serde(default)]
        at: Option<usize>,
        #[serde(default)]
        name: Option<String>,
    },
    SetTabColor {
        #[serde(default)]
        color: Option<ColorSpec>,
    },
    SetSheetVisibility {
        visibility: SheetVisibility,
    },
    /// Protect Sheet, Protect Workbook and their reverses.
    Protection(ReviewOp),
    InsertCells {
        range: RangeRef,
        shift: CellShift,
    },
    DeleteCells {
        range: RangeRef,
        shift: CellShift,
    },
    /// Formulas > Calculate Now: recalculates every formula.
    CalculateNow,
    /// Formulas > Calculate Sheet.
    CalculateSheet,
}

impl ObjectsOp {
    /// Whether the operation removes content that exists.
    pub(crate) fn destroys(&self) -> bool {
        matches!(
            self,
            Self::DeleteSheet
                | Self::DeleteName { .. }
                | Self::DeleteNote { .. }
                | Self::RemoveHyperlink { .. }
                | Self::DeleteConditionalFormat { .. }
                | Self::ClearConditionalFormats { .. }
                | Self::DeleteCells { .. }
                | Self::ReplaceConditionalFormat { .. }
        )
    }
}

// ---- conversions ------------------------------------------------------------------------------

fn color_of(spec: &ColorSpec) -> Result<Color, CommandError> {
    let source = if let Some(h) = &spec.rgb {
        let v = (h.len() == 6)
            .then(|| u32::from_str_radix(h, 16).ok())
            .flatten()
            .ok_or_else(|| bad(format!("{h:?} is not an RRGGBB colour")))?;
        ColorSource::Rgb(Rgb::new((v >> 16) as u8, (v >> 8) as u8, v as u8))
    } else if let Some(t) = spec.theme {
        ColorSource::Theme(t)
    } else if let Some(i) = spec.indexed {
        ColorSource::Indexed(i)
    } else {
        ColorSource::Auto
    };
    Ok(Color {
        source,
        tint: spec.tint.clamp(-1.0, 1.0),
    })
}

fn solid_fill(spec: &ColorSpec) -> Result<DxfFill, CommandError> {
    Ok(DxfFill {
        pattern: FillPattern::Solid,
        fg: None,
        bg: Some(color_of(spec)?),
    })
}

fn dxf_of(f: &CfFormat) -> Result<Dxf, CommandError> {
    Ok(Dxf {
        font: DxfFont {
            bold: f.bold,
            italic: f.italic,
            strike: f.strikethrough,
            underline: f.underline.map(|u| {
                if u {
                    Underline::Single
                } else {
                    Underline::None
                }
            }),
            color: f.font_color.as_ref().map(color_of).transpose()?,
        },
        fill: f.fill_color.as_ref().map(solid_fill).transpose()?,
        border: open365_xlsx::DxfBorder::default(),
        num_fmt: f.number_format.clone(),
    })
}

fn rule_of(r: &CfRuleRequest) -> Result<CfRuleSpec, CommandError> {
    let mut formula = None;
    let kind = match &r.kind {
        CfRuleKind::CellValue { compare, formulas } => CfKind::CellIs {
            op: match compare {
                CellCompare::Between => CellIsOp::Between,
                CellCompare::NotBetween => CellIsOp::NotBetween,
                CellCompare::Equal => CellIsOp::Equal,
                CellCompare::NotEqual => CellIsOp::NotEqual,
                CellCompare::Greater => CellIsOp::GreaterThan,
                CellCompare::Less => CellIsOp::LessThan,
                CellCompare::GreaterOrEqual => CellIsOp::GreaterThanOrEqual,
                CellCompare::LessOrEqual => CellIsOp::LessThanOrEqual,
            },
            formulas: formulas
                .iter()
                .map(|f| f.trim_start_matches('=').to_owned())
                .collect(),
        },
        CfRuleKind::Formula { formula: f } => {
            formula = Some(f.clone());
            CfKind::Expression
        }
        CfRuleKind::Top10 {
            rank,
            percent,
            bottom,
        } => CfKind::Top10 {
            rank: *rank,
            percent: *percent,
            bottom: *bottom,
        },
        CfRuleKind::AboveAverage {
            above,
            include_equal,
            std_dev,
        } => CfKind::AboveAverage {
            above: *above,
            equal: *include_equal,
            std_dev: *std_dev,
        },
        CfRuleKind::DuplicateValues => CfKind::DuplicateValues,
        CfRuleKind::UniqueValues => CfKind::UniqueValues,
        CfRuleKind::ContainsText { text } => CfKind::ContainsText(text.clone()),
        CfRuleKind::NotContainsText { text } => CfKind::NotContainsText(text.clone()),
        CfRuleKind::BeginsWith { text } => CfKind::BeginsWith(text.clone()),
        CfRuleKind::EndsWith { text } => CfKind::EndsWith(text.clone()),
        CfRuleKind::ContainsBlanks => CfKind::ContainsBlanks,
        CfRuleKind::NotContainsBlanks => CfKind::NotContainsBlanks,
        CfRuleKind::ContainsErrors => CfKind::ContainsErrors,
        CfRuleKind::NotContainsErrors => CfKind::NotContainsErrors,
        CfRuleKind::DateOccurring { period } => CfKind::TimePeriod(match period {
            DatePeriod::Today => TimePeriod::Today,
            DatePeriod::Yesterday => TimePeriod::Yesterday,
            DatePeriod::Tomorrow => TimePeriod::Tomorrow,
            DatePeriod::Last7Days => TimePeriod::Last7Days,
            DatePeriod::ThisWeek => TimePeriod::ThisWeek,
            DatePeriod::LastWeek => TimePeriod::LastWeek,
            DatePeriod::NextWeek => TimePeriod::NextWeek,
            DatePeriod::ThisMonth => TimePeriod::ThisMonth,
            DatePeriod::LastMonth => TimePeriod::LastMonth,
            DatePeriod::NextMonth => TimePeriod::NextMonth,
        }),
        CfRuleKind::ColorScale { points, colors } => CfKind::ColorScale {
            cfvos: points.iter().map(CfPoint::cfvo).collect(),
            colors: colors.iter().map(color_of).collect::<Result<_, _>>()?,
        },
        CfRuleKind::DataBar {
            min,
            max,
            color,
            show_value,
            gradient,
            border,
            negative_color,
            axis,
            direction,
        } => {
            let auto = |p: &CfPoint, lo: bool| match p.kind {
                PointKind::Min | PointKind::AutoMin if lo => Cfvo {
                    kind: CfvoKind::AutoMin,
                    val: None,
                    gte: true,
                },
                PointKind::Max | PointKind::AutoMax if !lo => Cfvo {
                    kind: CfvoKind::AutoMax,
                    val: None,
                    gte: true,
                },
                _ => p.cfvo(),
            };
            let ext = DataBarExt {
                min_length: 0,
                max_length: 100,
                gradient: *gradient,
                border: *border,
                border_color: None,
                negative_same_as_positive: false,
                negative_fill: negative_color.as_ref().map(color_of).transpose()?,
                negative_border_same_as_positive: true,
                negative_border_color: None,
                axis: match axis.as_deref() {
                    Some("midpoint") => DataBarAxis::Midpoint,
                    Some("none") => DataBarAxis::None,
                    None | Some("automatic") => DataBarAxis::Automatic,
                    Some(other) => return Err(bad(format!("unknown data bar axis {other:?}"))),
                },
                axis_color: None,
                direction: match direction.as_deref() {
                    Some("left_to_right") => DataBarDirection::LeftToRight,
                    Some("right_to_left") => DataBarDirection::RightToLeft,
                    None | Some("context") => DataBarDirection::Context,
                    Some(other) => {
                        return Err(bad(format!("unknown data bar direction {other:?}")));
                    }
                },
                min: Some(auto(min, true)),
                max: Some(auto(max, false)),
            };
            CfKind::DataBar {
                min: min.cfvo(),
                max: max.cfvo(),
                color: color_of(color)?,
                min_length: 10,
                max_length: 90,
                show_value: *show_value,
                ext: Some(ext),
            }
        }
        CfRuleKind::IconSet {
            name,
            points,
            reverse,
            show_value,
        } => CfKind::IconSet {
            name: name.clone(),
            cfvos: points.iter().map(CfPoint::cfvo).collect(),
            reverse: *reverse,
            show_value: *show_value,
        },
    };
    Ok(CfRuleSpec {
        kind,
        format: r.format.as_ref().map(dxf_of).transpose()?,
        stop_if_true: r.stop_if_true,
        formula,
    })
}

fn cf_spec(req: &CfRequest) -> Result<CfSpec, CommandError> {
    Ok(CfSpec {
        ranges: req.ranges.iter().map(|r| r.rect()).collect(),
        rules: req.rules.iter().map(rule_of).collect::<Result<_, _>>()?,
    })
}

fn name_spec(n: &NameRequest) -> NameSpec {
    NameSpec {
        name: n.name.clone(),
        scope: n.scope,
        formula: n.refers_to.trim_start_matches('=').to_owned(),
        comment: n.comment.clone(),
        hidden: n.hidden,
    }
}

fn to_cell_value(v: &Value) -> CellValue {
    match v.top_left() {
        Value::Empty => CellValue::Number(0.0),
        Value::Number(n) if n.is_finite() => CellValue::Number(n),
        Value::Number(_) => CellValue::Error(CellError::Num),
        Value::Text(t) => CellValue::Str(t.into()),
        Value::Bool(b) => CellValue::Bool(b),
        Value::Error(e) => CellValue::Error(match e {
            ErrorKind::Null => CellError::Null,
            ErrorKind::DivZero => CellError::Div0,
            ErrorKind::Value => CellError::Value,
            ErrorKind::Ref => CellError::Ref,
            ErrorKind::Name => CellError::Name,
            ErrorKind::Num => CellError::Num,
            ErrorKind::NA => CellError::NA,
            ErrorKind::GettingData => CellError::GettingData,
            other => CellError::Other(other.as_str().into()),
        }),
        Value::Array(_) => CellValue::Error(CellError::Value),
    }
}

// ---- running ----------------------------------------------------------------------------------

/// Rebuilds the calculation engine from the edited workbook and writes every changed formula result
/// back as a cached value (`only` limits it to one sheet). Returns the cells that changed.
fn refresh_values(x: &mut XlsxState, only: Option<usize>) -> Result<Vec<CellRef>, CommandError> {
    let engine = build_engine(&x.workbook, &|_, _, _| true);
    let mut updates: Vec<(CellRef, CellValue)> = Vec::new();
    for (i, info) in x.workbook.sheets().iter().enumerate() {
        if info.kind != SheetKind::Worksheet || only.is_some_and(|s| s != i) {
            continue;
        }
        for (row, col, cell) in x.workbook.iter_cells(i) {
            if cell.formula.is_none() {
                continue;
            }
            let fresh = to_cell_value(&engine.get(&info.name, row, col));
            if cell.value != fresh {
                updates.push((CellRef { sheet: i, row, col }, fresh));
            }
        }
    }
    let mut changed = Vec::new();
    for (at, value) in updates {
        x.workbook
            .set_cached_value(at.sheet, at.row, at.col, value)?;
        changed.push(at);
    }
    reset_engine(x);
    Ok(changed)
}

/// Drops the calculation engine; it is rebuilt from the workbook when next needed.
fn reset_engine(x: &mut XlsxState) {
    let wb: Workbook = x.workbook.clone();
    x.restore_workbook(wb);
}

fn positions(list: Vec<(usize, u32, u32)>) -> Vec<CellRef> {
    list.into_iter()
        .map(|(sheet, row, col)| CellRef { sheet, row, col })
        .collect()
}

fn merge(into: &mut Vec<CellRef>, more: Vec<CellRef>) {
    for c in more {
        if !into.contains(&c) {
            into.push(c);
        }
    }
}

fn note_changed(
    x: &mut XlsxState,
    sheet: usize,
    recalc: Vec<(usize, u32, u32)>,
) -> Result<Vec<CellRef>, CommandError> {
    let mut changed = positions(recalc);
    merge(&mut changed, refresh_values(x, None)?);
    let _ = sheet;
    Ok(changed)
}

/// Runs `op` on `sheet`.
pub(crate) fn run(
    x: &mut XlsxState,
    sheet: usize,
    op: &ObjectsOp,
    notes: &mut Vec<String>,
) -> Result<Vec<CellRef>, CommandError> {
    match op {
        ObjectsOp::AddPart {
            name,
            content_type,
            data,
        } => {
            x.workbook.add_part(name, content_type, data.clone())?;
            Ok(Vec::new())
        }
        ObjectsOp::AddRelationship {
            source,
            rel_type,
            target,
            external,
        } => {
            let src = source
                .as_deref()
                .map(PartName::new)
                .transpose()
                .map_err(|e| bad(e.to_string()))?;
            let mode = if *external {
                TargetMode::External
            } else {
                TargetMode::Internal
            };
            let id = x
                .workbook
                .add_relationship(src.as_ref(), rel_type, target, mode)?;
            notes.push(format!("relationship {id}"));
            Ok(Vec::new())
        }
        ObjectsOp::CreateTable(req) => {
            let spec = TableSpec {
                range: req.range.rect(),
                name: req.name.clone(),
                headers: match req.has_headers {
                    Some(true) => HeaderChoice::Yes,
                    Some(false) => HeaderChoice::No,
                    None => HeaderChoice::Guess,
                },
                style: match req.style.as_deref() {
                    None => TableStyleChoice::Default,
                    Some("") => TableStyleChoice::None,
                    Some(s) => TableStyleChoice::Named(s.to_owned()),
                },
                totals_row: req.totals_row,
                auto_filter: req.filter_buttons,
                first_column: req.first_column,
                last_column: req.last_column,
                row_stripes: req.banded_rows,
                column_stripes: req.banded_columns,
            };
            let info = x.workbook.create_table(sheet, &spec)?;
            notes.push(format!("table {}", info.name));
            note_changed(x, sheet, info.recalc)
        }
        ObjectsOp::DefineName(n) => {
            x.workbook.define_name(&name_spec(n))?;
            reset_engine(x);
            Ok(Vec::new())
        }
        ObjectsOp::EditName { name, scope, to } => {
            let recalc = x.workbook.edit_defined_name(name, *scope, &name_spec(to))?;
            note_changed(x, sheet, recalc)
        }
        ObjectsOp::DeleteName { name, scope } => {
            x.workbook.delete_defined_name(name, *scope)?;
            refresh_values(x, None)
        }
        ObjectsOp::CreateNamesFromSelection {
            range,
            top,
            left,
            bottom,
            right,
            replace,
        } => {
            let made = x.workbook.create_names_from_selection(
                sheet,
                range.rect(),
                NameSource {
                    top: *top,
                    left: *left,
                    bottom: *bottom,
                    right: *right,
                },
                *replace,
            )?;
            notes.push(format!("{} name(s) created", made.len()));
            reset_engine(x);
            Ok(Vec::new())
        }
        ObjectsOp::SetHyperlink(h) => {
            x.workbook.set_hyperlink(
                sheet,
                &HyperlinkSpec {
                    range: h.range.rect(),
                    target: match &h.target {
                        LinkKind::Url { address } => HyperlinkTarget::Url(address.clone()),
                        LinkKind::Place { reference } => {
                            HyperlinkTarget::Location(reference.clone())
                        }
                        LinkKind::Email { address, subject } => HyperlinkTarget::Email {
                            address: address.clone(),
                            subject: subject.clone(),
                        },
                    },
                    display: h.text.clone(),
                    tooltip: h.screen_tip.clone(),
                },
            )?;
            Ok(Vec::new())
        }
        ObjectsOp::RemoveHyperlink { row, col } => {
            if !x.workbook.remove_hyperlink(sheet, *row, *col)? {
                return Err(bad("the cell has no hyperlink"));
            }
            Ok(Vec::new())
        }
        ObjectsOp::SetNote(n) => {
            x.workbook.set_note(
                sheet,
                &NoteSpec {
                    row: n.row,
                    col: n.col,
                    author: n.author.clone(),
                    text: n.text.clone(),
                    author_prefix: n.author_prefix,
                },
            )?;
            Ok(Vec::new())
        }
        ObjectsOp::DeleteNote { row, col } => {
            if !x.workbook.delete_note(sheet, *row, *col)? {
                return Err(bad("the cell has no note"));
            }
            Ok(Vec::new())
        }
        ObjectsOp::InsertChart {
            chart_xml,
            range,
            name,
        } => {
            let made = x.workbook.insert_chart(
                sheet,
                chart_xml.as_bytes(),
                &ChartAnchor {
                    range: range.rect(),
                    name: name.clone(),
                },
            )?;
            notes.push(format!("chart {}", made.name));
            Ok(Vec::new())
        }
        ObjectsOp::AddPivotTable {
            cache_definition,
            cache_records,
            table_definition,
        } => {
            let made = x.workbook.add_pivot_table(
                sheet,
                &PivotSpec {
                    cache_definition: cache_definition.clone(),
                    cache_records: cache_records.clone(),
                    table_definition: table_definition.clone(),
                },
            )?;
            notes.push(format!("pivot cache {}", made.cache_id));
            Ok(Vec::new())
        }
        ObjectsOp::SetCalculation {
            mode,
            full_calc_on_load,
            calc_on_save,
            iterate,
            iterate_count,
            iterate_delta,
            precision_as_displayed,
        } => {
            let mut s = x.workbook.calc_settings();
            if let Some(m) = mode {
                s.mode = match m {
                    CalcModeChoice::Automatic => CalcMode::Automatic,
                    CalcModeChoice::AutomaticExceptTables => CalcMode::AutomaticExceptTables,
                    CalcModeChoice::Manual => CalcMode::Manual,
                };
            }
            if let Some(v) = full_calc_on_load {
                s.full_calc_on_load = *v;
            }
            if let Some(v) = calc_on_save {
                s.calc_on_save = *v;
            }
            if let Some(v) = iterate {
                s.iterate = *v;
            }
            if let Some(v) = iterate_count {
                s.iterate_count = *v;
            }
            if let Some(v) = iterate_delta {
                s.iterate_delta = *v;
            }
            if let Some(v) = precision_as_displayed {
                s.full_precision = !*v;
            }
            x.workbook.set_calc_settings(&s)?;
            Ok(Vec::new())
        }
        ObjectsOp::AddConditionalFormat(req) => {
            x.workbook.add_conditional_format(sheet, &cf_spec(req)?)?;
            Ok(Vec::new())
        }
        ObjectsOp::ReplaceConditionalFormat { index, format } => {
            x.workbook
                .replace_conditional_format(sheet, *index, &cf_spec(format)?)?;
            Ok(Vec::new())
        }
        ObjectsOp::DeleteConditionalFormat { index } => {
            x.workbook.delete_conditional_format(sheet, *index)?;
            Ok(Vec::new())
        }
        ObjectsOp::ClearConditionalFormats { range } => {
            let n = x
                .workbook
                .clear_conditional_formats_in(sheet, range.rect())?;
            if n == 0 {
                return Err(bad("no conditional format applies to the selected cells"));
            }
            Ok(Vec::new())
        }
        ObjectsOp::AddSheet { at, name } => {
            let at = at.unwrap_or_else(|| x.workbook.sheets().len());
            x.workbook.add_sheet(at, name.as_deref())?;
            reset_engine(x);
            Ok(Vec::new())
        }
        ObjectsOp::DeleteSheet => {
            let recalc = x.workbook.delete_sheet(sheet)?;
            note_changed(x, sheet, recalc)
        }
        ObjectsOp::RenameSheet { name } => {
            let recalc = x.workbook.rename_sheet(sheet, name)?;
            note_changed(x, sheet, recalc)
        }
        ObjectsOp::MoveSheet { to } => {
            x.workbook.move_sheet(sheet, *to)?;
            reset_engine(x);
            Ok(Vec::new())
        }
        ObjectsOp::CopySheet { at, name } => {
            let at = at.unwrap_or(sheet + 1);
            x.workbook.copy_sheet(sheet, at, name.as_deref())?;
            refresh_values(x, None)
        }
        ObjectsOp::SetTabColor { color } => {
            let tab = match color {
                None => TabColor::None,
                Some(c) => match color_of(c)?.source {
                    ColorSource::Rgb(rgb) => TabColor::Rgb(rgb),
                    ColorSource::Theme(index) => TabColor::Theme {
                        index,
                        tint: c.tint,
                    },
                    ColorSource::Indexed(i) => TabColor::Indexed(i),
                    ColorSource::Auto => TabColor::None,
                },
            };
            x.workbook.set_tab_color(sheet, tab)?;
            Ok(Vec::new())
        }
        ObjectsOp::SetSheetVisibility { visibility } => {
            x.workbook.set_sheet_state(
                sheet,
                match visibility {
                    SheetVisibility::Visible => SheetState::Visible,
                    SheetVisibility::Hidden => SheetState::Hidden,
                    SheetVisibility::VeryHidden => SheetState::VeryHidden,
                },
            )?;
            Ok(Vec::new())
        }
        ObjectsOp::Protection(op) => {
            if !matches!(
                op,
                ReviewOp::ProtectSheet { .. }
                    | ReviewOp::UnprotectSheet { .. }
                    | ReviewOp::ProtectWorkbook { .. }
                    | ReviewOp::UnprotectWorkbook { .. }
            ) {
                return Err(bad("only protection operations belong here"));
            }
            review::run(x, sheet, op, notes)
        }
        ObjectsOp::InsertCells { range, shift } => {
            let dir = match shift {
                CellShift::Right => ShiftDirection::Right,
                CellShift::Down => ShiftDirection::Down,
                _ => return Err(bad("Insert Cells shifts right or down")),
            };
            let recalc = x.workbook.shift_cells(sheet, range.rect(), dir)?;
            note_changed(x, sheet, recalc)
        }
        ObjectsOp::DeleteCells { range, shift } => {
            let dir = match shift {
                CellShift::Left => ShiftDirection::Left,
                CellShift::Up => ShiftDirection::Up,
                _ => return Err(bad("Delete Cells shifts left or up")),
            };
            let recalc = x.workbook.shift_cells(sheet, range.rect(), dir)?;
            note_changed(x, sheet, recalc)
        }
        ObjectsOp::CalculateNow => refresh_values(x, None),
        ObjectsOp::CalculateSheet => refresh_values(x, Some(sheet)),
    }
}
