//! Conditional formatting display, read-only: which rules hold for a cell and what they do to
//! its look.
//!
//! The model crate reads the rules (`<conditionalFormatting>`) and differential formats
//! (`<dxf>`); this module evaluates them against the workbook's values. Nothing is written
//! and no command is involved, so the effect class is `Read`. Rules whose condition is a
//! formula (`expression`, `cellIs` operands, text, blank, error and time-period rules) run
//! through the calculation engine in a scratch copy of the workbook, the same path as
//! [`crate::evaluate_formula`]; rules that compare a cell with its neighbours (top 10,
//! average, duplicates, scales, bars, icons) read the stored values of the block's cells.
//!
//! Rules apply in priority order. For each property (a font part, the fill, a border edge,
//! the number format, a bar, an icon) the first matching rule that sets it wins; a rule with
//! `stopIfTrue` ends the walk.

use std::cell::{OnceCell, RefCell};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

use open365_calc::eval::FixedClock;
use open365_calc::{EvalContext, Value, evaluate, parse_formula};
use open365_xlsx::{
    CellIsOp, CellValue, CfBlock, CfKind, CfRule, Cfvo, CfvoKind, Color, DataBarAxis,
    DataBarDirection, DataBarExt, Dxf, Rgb, TimePeriod, Workbook, cell_ref_string, shift_formula,
};

use crate::eval::FormulaEngine;
use crate::xlsx_state::to_calc_value;

/// A data bar for one cell, resolved to geometry: positions and lengths are fractions of the
/// bar span (the cell width less the bar's margins), counted from the cell's left edge.
#[derive(Debug, Clone, PartialEq)]
pub struct DataBarEffect {
    /// Where the value sits between the bar's minimum and maximum, `0.0..=1.0`.
    pub fraction: f64,
    /// Where the bar starts: the axis, `0.0..=1.0` from the left edge.
    pub axis: f64,
    /// How long the bar is, `0.0..=1.0` of the span; 0 draws nothing.
    pub length: f64,
    /// The bar grows leftwards from the axis (negative values, or right-to-left bars).
    pub grows_left: bool,
    /// The value is below zero.
    pub negative: bool,
    /// The fill, already the negative colour for a negative bar that has its own.
    pub color: Rgb,
    /// Gradient fill (a solid one otherwise).
    pub gradient: bool,
    /// The border colour, when the bar has a border.
    pub border: Option<Rgb>,
    /// The axis line, drawn at `axis` for ranges that hold negative values.
    pub axis_line: Option<Rgb>,
    pub show_value: bool,
}

/// An icon for one cell.
#[derive(Debug, Clone, PartialEq)]
pub struct IconEffect {
    /// The icon set name (`3Arrows`, `5Rating`, ...).
    pub set: String,
    /// Which icon of the set, 0 = the lowest band, after `reverse` is applied.
    pub index: usize,
    /// Icons in the set.
    pub count: usize,
    pub show_value: bool,
}

/// What conditional formatting does to one cell.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CfEffect {
    /// The merged differential format of the matching rules.
    pub dxf: Dxf,
    /// The fill a colour scale gives (wins over a lower-priority dxf fill).
    pub scale_fill: Option<Rgb>,
    pub data_bar: Option<DataBarEffect>,
    pub icon: Option<IconEffect>,
}

impl CfEffect {
    fn is_empty(&self) -> bool {
        self.dxf == Dxf::default()
            && self.scale_fill.is_none()
            && self.data_bar.is_none()
            && self.icon.is_none()
    }
}

/// A comparable cell value for duplicate detection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Number(u64),
    Text(String),
    Bool(bool),
    Error(String),
}

#[derive(Debug)]
enum Stats {
    /// Nothing precomputed (formula rules).
    Plain,
    Top10 {
        /// A cell qualifies when it is `>=` (top) or `<=` (bottom) this value.
        threshold: Option<f64>,
        bottom: bool,
    },
    Average {
        /// Cutoff; a cell qualifies when it is beyond it (or on it with `equal`).
        cutoff: Option<f64>,
        above: bool,
        equal: bool,
    },
    Duplicates(HashMap<Key, u32>),
    Scale(Vec<(f64, Rgb)>),
    Bar {
        min: f64,
        max: f64,
    },
    Icons(Vec<f64>),
}

#[derive(Debug)]
struct Prepared {
    block: usize,
    rule: CfRule,
    stats: OnceCell<Stats>,
}

/// Memoised answers by cell.
type Memo = HashMap<(u32, u32), Option<Arc<CfEffect>>>;

/// The conditional formatting of one sheet, ready to ask about cells. Build it once per
/// workbook revision; answers are memoised. Not `Send` (the formula engine holds a clock).
#[derive(Debug)]
pub struct ConditionalFormats {
    sheet: usize,
    blocks: Vec<CfBlock>,
    rules: Vec<Prepared>,
    today: Option<f64>,
    engine: OnceCell<FormulaEngine>,
    memo: RefCell<Memo>,
}

impl ConditionalFormats {
    /// Reads the rules of `sheet`. `today` pins the date `TODAY()` and the time-period rules
    /// use as an Excel serial (tests); `None` uses the system clock.
    pub fn new(wb: &Workbook, sheet: usize, today: Option<f64>) -> Self {
        let blocks = wb
            .sheet(sheet)
            .map(|ws| ws.conditional_formats())
            .unwrap_or_default();
        let mut rules: Vec<Prepared> = blocks
            .iter()
            .enumerate()
            .flat_map(|(block, b)| {
                b.rules.iter().map(move |rule| Prepared {
                    block,
                    rule: rule.clone(),
                    stats: OnceCell::new(),
                })
            })
            .collect();
        rules.sort_by_key(|p| p.rule.priority);
        Self {
            sheet,
            blocks,
            rules,
            today,
            engine: OnceCell::new(),
            memo: RefCell::new(HashMap::new()),
        }
    }

    /// No rule at all: nothing to evaluate.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The cell ranges any rule covers, `(row1, col1, row2, col2)`.
    pub fn ranges(&self) -> impl Iterator<Item = (u32, u32, u32, u32)> + '_ {
        self.blocks.iter().flat_map(|b| b.ranges.iter().copied())
    }

    /// What the rules do to cell (`row`, `col`); `None` when no rule applies. `wb` must be
    /// the workbook this was built from.
    pub fn effect(&self, wb: &Workbook, row: u32, col: u32) -> Option<Arc<CfEffect>> {
        if let Some(hit) = self.memo.borrow().get(&(row, col)) {
            return hit.clone();
        }
        let computed = self.compute(wb, row, col).map(Arc::new);
        self.memo.borrow_mut().insert((row, col), computed.clone());
        computed
    }

    fn compute(&self, wb: &Workbook, row: u32, col: u32) -> Option<CfEffect> {
        let styles = wb.styles();
        let mut out = CfEffect::default();
        let mut fill_taken = false;
        for prepared in &self.rules {
            let block = &self.blocks[prepared.block];
            if !block.contains(row, col) {
                continue;
            }
            let rule = &prepared.rule;
            let Some(hit) = self.rule_applies(wb, prepared, block, row, col) else {
                continue;
            };
            match hit {
                Hit::No => continue,
                Hit::Dxf => {
                    if let Some(dxf) = rule.dxf_id.and_then(|id| styles.dxf(id)) {
                        let mut dxf = dxf.clone();
                        if fill_taken {
                            dxf.fill = None;
                        } else if dxf.fill.is_some() {
                            fill_taken = true;
                        }
                        out.dxf.fill_missing_from(&dxf);
                    }
                }
                Hit::Fill(rgb) => {
                    if !fill_taken {
                        fill_taken = true;
                        out.scale_fill = Some(rgb);
                    }
                }
                Hit::Bar(bar) => {
                    out.data_bar.get_or_insert(bar);
                }
                Hit::Icon(icon) => {
                    out.icon.get_or_insert(icon);
                }
            }
            if rule.stop_if_true {
                break;
            }
        }
        (!out.is_empty()).then_some(out)
    }

    fn engine(&self, wb: &Workbook) -> &FormulaEngine {
        self.engine
            .get_or_init(|| FormulaEngine::build(wb, &|_, _, _| true))
    }

    /// Evaluates formula text written for the block's anchor, as if entered in (`row`, `col`).
    fn eval(&self, wb: &Workbook, block: &CfBlock, text: &str, row: u32, col: u32) -> Value {
        let (ar, ac) = block.anchor();
        let shifted = shift_formula(
            text.trim().trim_start_matches('='),
            i64::from(row) - i64::from(ar),
            i64::from(col) - i64::from(ac),
        );
        let Ok(ast) = parse_formula(&shifted) else {
            return Value::Error(open365_calc::ErrorKind::Value);
        };
        let Some(info) = wb.sheets().get(self.sheet) else {
            return Value::Error(open365_calc::ErrorKind::Ref);
        };
        let engine = self.engine(wb).engine();
        let clock = self.today.map(|serial| {
            let epoch = if wb.date1904() { 24107.0 } else { 25569.0 };
            FixedClock((serial - epoch) * 86400.0)
        });
        let mut ctx = EvalContext::new(engine, &info.name)
            .at_cell(row, col)
            .with_date1904(wb.date1904());
        if let Some(clock) = &clock {
            ctx.clock = clock;
        }
        evaluate(&ast, &ctx).top_left()
    }

    fn cell_value(wb: &Workbook, sheet: usize, row: u32, col: u32) -> CellValue {
        wb.cell(sheet, row, col)
            .map_or(CellValue::Empty, |c| c.value.clone())
    }

    /// The formula that decides a rule built on one, as stored or as Excel would write it.
    fn formula_for(&self, rule: &CfRule, block: &CfBlock) -> Option<String> {
        if let Some(f) = rule.formula.as_deref()
            && !f.trim().is_empty()
            && !matches!(rule.kind, CfKind::CellIs { .. })
        {
            return Some(f.to_owned());
        }
        let (ar, ac) = block.anchor();
        let tl = cell_ref_string(ar, ac);
        let quoted = |t: &str| format!("\"{}\"", t.replace('"', "\"\""));
        Some(match &rule.kind {
            CfKind::Expression => return None,
            CfKind::ContainsText(t) => format!("NOT(ISERROR(SEARCH({},{tl})))", quoted(t)),
            CfKind::NotContainsText(t) => format!("ISERROR(SEARCH({},{tl}))", quoted(t)),
            CfKind::BeginsWith(t) => format!("LEFT({tl},LEN({q}))={q}", q = quoted(t)),
            CfKind::EndsWith(t) => format!("RIGHT({tl},LEN({q}))={q}", q = quoted(t)),
            CfKind::ContainsBlanks => format!("LEN(TRIM({tl}))=0"),
            CfKind::NotContainsBlanks => format!("LEN(TRIM({tl}))>0"),
            CfKind::ContainsErrors => format!("ISERROR({tl})"),
            CfKind::NotContainsErrors => format!("NOT(ISERROR({tl}))"),
            CfKind::TimePeriod(p) => match p {
                TimePeriod::Today => format!("FLOOR({tl},1)=TODAY()"),
                TimePeriod::Yesterday => format!("FLOOR({tl},1)=TODAY()-1"),
                TimePeriod::Tomorrow => format!("FLOOR({tl},1)=TODAY()+1"),
                TimePeriod::Last7Days => {
                    format!("AND(TODAY()-FLOOR({tl},1)<=6,FLOOR({tl},1)<=TODAY())")
                }
                TimePeriod::ThisWeek => format!(
                    "AND(TODAY()-ROUNDDOWN({tl},0)<=WEEKDAY(TODAY())-1,ROUNDDOWN({tl},0)-TODAY()<=7-WEEKDAY(TODAY()))"
                ),
                TimePeriod::LastWeek => format!(
                    "AND(TODAY()-ROUNDDOWN({tl},0)>=(WEEKDAY(TODAY())),TODAY()-ROUNDDOWN({tl},0)<(WEEKDAY(TODAY())+7))"
                ),
                TimePeriod::NextWeek => format!(
                    "AND(ROUNDDOWN({tl},0)-TODAY()>(7-WEEKDAY(TODAY())),ROUNDDOWN({tl},0)-TODAY()<(15-WEEKDAY(TODAY())))"
                ),
                TimePeriod::ThisMonth => {
                    format!("AND(MONTH({tl})=MONTH(TODAY()),YEAR({tl})=YEAR(TODAY()))")
                }
                TimePeriod::LastMonth => format!(
                    "AND(MONTH({tl})=MONTH(EDATE(TODAY(),0-1)),YEAR({tl})=YEAR(EDATE(TODAY(),0-1)))"
                ),
                TimePeriod::NextMonth => format!(
                    "AND(MONTH({tl})=MONTH(EDATE(TODAY(),0+1)),YEAR({tl})=YEAR(EDATE(TODAY(),0+1)))"
                ),
            },
            _ => return None,
        })
    }

    fn rule_applies(
        &self,
        wb: &Workbook,
        prepared: &Prepared,
        block: &CfBlock,
        row: u32,
        col: u32,
    ) -> Option<Hit> {
        let rule = &prepared.rule;
        let value = Self::cell_value(wb, self.sheet, row, col);
        let yes = |b: bool| Some(if b { Hit::Dxf } else { Hit::No });
        match &rule.kind {
            CfKind::Unknown(_) => None,
            CfKind::Expression => {
                let f = rule.formula.as_deref()?;
                yes(truthy(&self.eval(wb, block, f, row, col)))
            }
            CfKind::CellIs { op, formulas } => {
                let cell = to_calc_value(&value);
                let operands: Vec<Value> = formulas
                    .iter()
                    .map(|f| self.eval(wb, block, f, row, col))
                    .collect();
                let a = operands.first()?;
                let ord = |x: &Value, y: &Value| compare(x, y);
                let holds = match op {
                    CellIsOp::LessThan => ord(&cell, a) == Ordering::Less,
                    CellIsOp::LessThanOrEqual => ord(&cell, a) != Ordering::Greater,
                    CellIsOp::Equal => ord(&cell, a) == Ordering::Equal,
                    CellIsOp::NotEqual => ord(&cell, a) != Ordering::Equal,
                    CellIsOp::GreaterThanOrEqual => ord(&cell, a) != Ordering::Less,
                    CellIsOp::GreaterThan => ord(&cell, a) == Ordering::Greater,
                    CellIsOp::Between | CellIsOp::NotBetween => {
                        let b = operands.get(1)?;
                        let inside =
                            ord(&cell, a) != Ordering::Less && ord(&cell, b) != Ordering::Greater;
                        inside == matches!(op, CellIsOp::Between)
                    }
                };
                yes(holds)
            }
            CfKind::ContainsText(_)
            | CfKind::NotContainsText(_)
            | CfKind::BeginsWith(_)
            | CfKind::EndsWith(_)
            | CfKind::ContainsBlanks
            | CfKind::NotContainsBlanks
            | CfKind::ContainsErrors
            | CfKind::NotContainsErrors
            | CfKind::TimePeriod(_) => {
                let f = self.formula_for(rule, block)?;
                yes(truthy(&self.eval(wb, block, &f, row, col)))
            }
            CfKind::Top10 { .. } | CfKind::AboveAverage { .. } => {
                let CellValue::Number(n) = value else {
                    return Some(Hit::No);
                };
                match self.stats(wb, prepared, block) {
                    Stats::Top10 { threshold, bottom } => {
                        let t = (*threshold)?;
                        yes(if *bottom { n <= t } else { n >= t })
                    }
                    Stats::Average {
                        cutoff,
                        above,
                        equal,
                    } => {
                        let c = (*cutoff)?;
                        yes(match (above, equal) {
                            (true, false) => n > c,
                            (true, true) => n >= c,
                            (false, false) => n < c,
                            (false, true) => n <= c,
                        })
                    }
                    _ => None,
                }
            }
            CfKind::DuplicateValues | CfKind::UniqueValues => {
                let key = key_of(&value)?;
                let Stats::Duplicates(counts) = self.stats(wb, prepared, block) else {
                    return None;
                };
                let n = counts.get(&key).copied().unwrap_or(0);
                yes(if matches!(rule.kind, CfKind::DuplicateValues) {
                    n > 1
                } else {
                    n == 1
                })
            }
            CfKind::ColorScale { .. } => {
                let CellValue::Number(n) = value else {
                    return Some(Hit::No);
                };
                let Stats::Scale(points) = self.stats(wb, prepared, block) else {
                    return None;
                };
                scale_color(points, n).map(Hit::Fill)
            }
            CfKind::DataBar {
                min_length,
                max_length,
                color,
                show_value,
                ext,
                ..
            } => {
                let CellValue::Number(n) = value else {
                    return Some(Hit::No);
                };
                let Stats::Bar { min, max } = self.stats(wb, prepared, block) else {
                    return None;
                };
                let styles = wb.styles();
                let positive = styles.resolve_color(color);
                let bar = match ext {
                    Some(ext) => {
                        bar_geometry(ext, n, *min, *max, positive, |c| styles.resolve_color(c))
                    }
                    None => legacy_bar(n, *min, *max, positive, *min_length, *max_length),
                };
                Some(Hit::Bar(DataBarEffect {
                    show_value: *show_value,
                    ..bar
                }))
            }
            CfKind::IconSet {
                name,
                cfvos,
                reverse,
                show_value,
            } => {
                let CellValue::Number(n) = value else {
                    return Some(Hit::No);
                };
                let Stats::Icons(thresholds) = self.stats(wb, prepared, block) else {
                    return None;
                };
                let count = thresholds.len().max(1);
                let mut index = 0;
                for (i, t) in thresholds.iter().enumerate() {
                    let gte = cfvos.get(i).is_none_or(|c| c.gte);
                    if i > 0 && (if gte { n >= *t } else { n > *t }) {
                        index = i;
                    }
                }
                if *reverse {
                    index = count - 1 - index;
                }
                Some(Hit::Icon(IconEffect {
                    set: name.clone(),
                    index,
                    count,
                    show_value: *show_value,
                }))
            }
        }
    }

    /// The numbers and keys of the block's cells, gathered for the statistics of one rule.
    fn gather(&self, wb: &Workbook, block: &CfBlock) -> Vec<CellValue> {
        let Some(ws) = wb.sheet(self.sheet) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for &(r1, c1, r2, c2) in &block.ranges {
            let rows = ws.rows();
            let start = rows.partition_point(|r| r.r < r1);
            for row in rows[start..].iter().take_while(|r| r.r <= r2) {
                let cs = row.cells.partition_point(|c| c.col < c1);
                for cell in row.cells[cs..].iter().take_while(|c| c.col <= c2) {
                    if !matches!(cell.value, CellValue::Empty) {
                        out.push(cell.value.clone());
                    }
                }
            }
        }
        out
    }

    fn stats<'a>(&'a self, wb: &Workbook, prepared: &'a Prepared, block: &CfBlock) -> &'a Stats {
        prepared.stats.get_or_init(|| {
            let values = self.gather(wb, block);
            let mut nums: Vec<f64> = values
                .iter()
                .filter_map(|v| match v {
                    CellValue::Number(n) if n.is_finite() => Some(*n),
                    _ => None,
                })
                .collect();
            nums.sort_by(f64::total_cmp);
            let (ar, ac) = block.anchor();
            let threshold_value = |c: &Cfvo, fallback: f64| -> f64 {
                let (min, max) = (nums.first().copied(), nums.last().copied());
                match c.kind {
                    CfvoKind::Min => min.unwrap_or(fallback),
                    CfvoKind::Max => max.unwrap_or(fallback),
                    CfvoKind::AutoMin => min.unwrap_or(fallback).min(0.0),
                    CfvoKind::AutoMax => max.unwrap_or(fallback).max(0.0),
                    CfvoKind::Num => parse_num(c.val.as_deref()).unwrap_or(fallback),
                    CfvoKind::Percent => {
                        let p = parse_num(c.val.as_deref()).unwrap_or(0.0);
                        let (lo, hi) = (min.unwrap_or(0.0), max.unwrap_or(0.0));
                        lo + (hi - lo) * p / 100.0
                    }
                    CfvoKind::Percentile => {
                        let p = parse_num(c.val.as_deref()).unwrap_or(0.0);
                        percentile(&nums, p / 100.0)
                    }
                    CfvoKind::Formula => c
                        .val
                        .as_deref()
                        .and_then(|f| match self.eval(wb, block, f, ar, ac) {
                            Value::Number(n) => Some(n),
                            _ => None,
                        })
                        .unwrap_or(fallback),
                }
            };
            match &prepared.rule.kind {
                CfKind::Top10 {
                    rank,
                    percent,
                    bottom,
                } => {
                    let n = nums.len();
                    let take = if *percent {
                        ((n as f64) * f64::from(*rank) / 100.0).floor() as usize
                    } else {
                        *rank as usize
                    }
                    .clamp(1, n.max(1));
                    let threshold = (n > 0).then(|| {
                        if *bottom {
                            nums[take - 1]
                        } else {
                            nums[n - take]
                        }
                    });
                    Stats::Top10 {
                        threshold,
                        bottom: *bottom,
                    }
                }
                CfKind::AboveAverage {
                    above,
                    equal,
                    std_dev,
                } => {
                    let n = nums.len();
                    let cutoff = (n > 0).then(|| {
                        let mean = nums.iter().sum::<f64>() / n as f64;
                        if *std_dev == 0 || n < 2 {
                            mean
                        } else {
                            let var = nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>()
                                / (n as f64 - 1.0);
                            let k = f64::from(*std_dev) * var.sqrt();
                            if *above { mean + k } else { mean - k }
                        }
                    });
                    Stats::Average {
                        cutoff,
                        above: *above,
                        equal: *equal,
                    }
                }
                CfKind::DuplicateValues | CfKind::UniqueValues => {
                    let mut counts: HashMap<Key, u32> = HashMap::new();
                    for v in &values {
                        if let Some(k) = key_of(v) {
                            *counts.entry(k).or_default() += 1;
                        }
                    }
                    Stats::Duplicates(counts)
                }
                CfKind::ColorScale { cfvos, colors } => {
                    let points: Vec<(f64, Rgb)> = cfvos
                        .iter()
                        .zip(colors)
                        .map(|(c, color)| {
                            (threshold_value(c, 0.0), wb.styles().resolve_color(color))
                        })
                        .collect();
                    Stats::Scale(points)
                }
                CfKind::DataBar { min, max, ext, .. } => {
                    // The Excel 2010 extension carries its own minimum and maximum (`autoMin`).
                    let (min, max) = ext
                        .as_ref()
                        .and_then(|x| x.min.as_ref().zip(x.max.as_ref()))
                        .unwrap_or((min, max));
                    Stats::Bar {
                        min: threshold_value(min, 0.0),
                        max: threshold_value(max, 0.0),
                    }
                }
                CfKind::IconSet { cfvos, .. } => {
                    Stats::Icons(cfvos.iter().map(|c| threshold_value(c, 0.0)).collect())
                }
                _ => Stats::Plain,
            }
        })
    }
}

enum Hit {
    No,
    Dxf,
    Fill(Rgb),
    Bar(DataBarEffect),
    Icon(IconEffect),
}

/// Excel's red for negative bars that have no colour of their own.
const NEGATIVE_RED: Rgb = Rgb {
    r: 0xFF,
    g: 0,
    b: 0,
};

/// A 2007 data bar (no Excel 2010 extension): no axis, every bar grows from the left edge, the
/// lowest value at `min_length` percent and the highest at `max_length`.
fn legacy_bar(
    n: f64,
    min: f64,
    max: f64,
    color: Rgb,
    min_length: u32,
    max_length: u32,
) -> DataBarEffect {
    let fraction = if max > min {
        ((n - min) / (max - min)).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let pct = |p: u32| f64::from(p.min(100)) / 100.0;
    let (lo, hi) = (pct(min_length), pct(max_length));
    DataBarEffect {
        fraction,
        axis: 0.0,
        length: lo + (hi - lo) * fraction,
        grows_left: false,
        negative: n < 0.0,
        color,
        gradient: true,
        border: Some(color),
        axis_line: None,
        show_value: true,
    }
}

/// An Excel 2010 data bar, as Excel draws it (measured against Excel, see
/// `fixtures/reference/xlsx/grid-databar`). With `min` and `max` the range the value is placed
/// in: the automatic axis sits where zero falls in it (at the left edge when every value is
/// positive, the right edge when every value is negative) and a bar is as long as the value is
/// far from zero, or from the end of the range that has no zero; a midpoint axis is the middle
/// of the cell with both halves scaled to the larger absolute end; without an axis every bar
/// grows from the left in proportion to its place in the range.
fn bar_geometry(
    ext: &DataBarExt,
    n: f64,
    min: f64,
    max: f64,
    positive: Rgb,
    resolve: impl Fn(&Color) -> Rgb,
) -> DataBarEffect {
    let span = max - min;
    let v = n.clamp(min.min(max), max.max(min));
    let fraction = if span > 0.0 { (v - min) / span } else { 1.0 };
    let (mut axis, mut length, mut grows_left) = match ext.axis {
        DataBarAxis::None => (0.0, fraction, false),
        DataBarAxis::Midpoint => {
            let scale = min.abs().max(max.abs());
            let half = if scale > 0.0 {
                v.abs() / scale / 2.0
            } else {
                0.5
            };
            (0.5, half, v < 0.0)
        }
        DataBarAxis::Automatic if span > 0.0 => {
            let zero = 0.0_f64.clamp(min, max);
            ((zero - min) / span, (v - zero).abs() / span, v < zero)
        }
        DataBarAxis::Automatic => (0.0, 1.0, false),
    };
    let lo = f64::from(ext.min_length.min(100)) / 100.0;
    let hi = f64::from(ext.max_length.min(100)) / 100.0;
    length = if ext.axis == DataBarAxis::None {
        // The lowest value gets `lo`, the highest `hi`; a bar of no length stays empty.
        if length > 0.0 {
            lo + (hi - lo) * length
        } else {
            0.0
        }
    } else {
        length * hi
    };
    if ext.direction == DataBarDirection::RightToLeft {
        axis = 1.0 - axis;
        grows_left = !grows_left;
    }
    let negative = n < 0.0;
    let own = negative && !ext.negative_same_as_positive;
    let color = if own {
        ext.negative_fill.as_ref().map_or(NEGATIVE_RED, &resolve)
    } else {
        positive
    };
    let border = ext.border.then(|| {
        if negative && !ext.negative_border_same_as_positive {
            ext.negative_border_color.as_ref().map_or(color, &resolve)
        } else {
            ext.border_color.as_ref().map_or(positive, &resolve)
        }
    });
    let edge = axis <= 0.0 || axis >= 1.0;
    let axis_line = (ext.axis != DataBarAxis::None && min < 0.0 && !edge).then(|| {
        ext.axis_color
            .as_ref()
            .map_or(Rgb { r: 0, g: 0, b: 0 }, &resolve)
    });
    DataBarEffect {
        fraction,
        axis,
        length: length.clamp(0.0, 1.0),
        grows_left,
        negative,
        color,
        gradient: ext.gradient,
        border,
        axis_line,
        show_value: true,
    }
}

fn parse_num(v: Option<&str>) -> Option<f64> {
    v?.trim().parse().ok()
}

/// `PERCENTILE.INC` of ascending `sorted` at `k` in `0.0..=1.0`.
fn percentile(sorted: &[f64], k: f64) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return 0.0;
    }
    let pos = k.clamp(0.0, 1.0) * (n as f64 - 1.0);
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(n - 1);
    sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
}

fn key_of(v: &CellValue) -> Option<Key> {
    Some(match v {
        CellValue::Empty => return None,
        CellValue::Number(n) => Key::Number(if *n == 0.0 { 0 } else { n.to_bits() }),
        CellValue::Bool(b) => Key::Bool(*b),
        CellValue::Error(e) => Key::Error(e.code().to_owned()),
        other => Key::Text(other.as_text()?.to_lowercase()),
    })
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => *n != 0.0,
        _ => false,
    }
}

/// Excel's comparison order: numbers, then text (case-insensitive), then booleans; a blank
/// cell compares as 0 against a number and as the empty text against text.
fn compare(a: &Value, b: &Value) -> Ordering {
    fn rank(v: &Value) -> u8 {
        match v {
            Value::Empty | Value::Number(_) => 0,
            Value::Text(_) => 1,
            Value::Bool(_) => 2,
            _ => 3,
        }
    }
    match (a, b) {
        (Value::Text(x), Value::Empty) => x.is_empty().then_some(Ordering::Equal),
        (Value::Empty, Value::Text(y)) => y.is_empty().then_some(Ordering::Equal),
        _ => None,
    }
    .unwrap_or_else(|| match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.total_cmp(y),
        (Value::Number(x), Value::Empty) => x.total_cmp(&0.0),
        (Value::Empty, Value::Number(y)) => 0.0f64.total_cmp(y),
        (Value::Empty, Value::Empty) => Ordering::Equal,
        (Value::Text(x), Value::Text(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    })
}

/// The colour of a scale at `n`: the first colour at or below the first threshold, the last at
/// or above the last, and a per-channel linear blend between the two surrounding points.
fn scale_color(points: &[(f64, Rgb)], n: f64) -> Option<Rgb> {
    let first = points.first()?;
    let last = points.last()?;
    if n <= first.0 {
        return Some(first.1);
    }
    if n >= last.0 {
        return Some(last.1);
    }
    let pair = points.windows(2).find(|w| n <= w[1].0)?;
    let ((x0, c0), (x1, c1)) = (pair[0], pair[1]);
    if x1 <= x0 {
        return Some(c1);
    }
    let t = (n - x0) / (x1 - x0);
    // Excel blends with a 16-bit fixed-point fraction and truncates the step toward the start
    // colour, whichever way the channel runs. This fit reproduces all 36 blended cells of
    // `fixtures/xlsx/grid-cf.xlsx` (plain f64 arithmetic misses 3 whole-number steps).
    let t = (t * 65536.0).round() / 65536.0;
    let mix = |a: u8, b: u8| {
        let step = (f64::from(b) - f64::from(a)) * t;
        (f64::from(a) + step.trunc()) as u8
    };
    Some(Rgb::new(mix(c0.r, c1.r), mix(c0.g, c1.g), mix(c0.b, c1.b)))
}
