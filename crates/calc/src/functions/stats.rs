//! Statistical functions: order statistics, ranking, spread and the `...A` variants.
//!
//! Like `SUM`, the list-style functions (`MEDIAN`, `MODE`, `STDEV`, ...) coerce direct arguments and ignore
//! text and booleans inside references. The array parameters (`LARGE`, `PERCENTILE`, `RANK`, ...) never coerce.

use std::cmp::Ordering;

use super::math::collect;
use super::{Res, arity, num_arg, number, optional};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value, parse_number_text_in};

fn sorted(mut nums: Vec<f64>) -> Vec<f64> {
    nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    nums
}

/// Numbers of an array or reference parameter: text, booleans and blanks are ignored, errors propagate.
fn array_numbers(ctx: &EvalContext<'_>, arg: &Arg) -> Res<Vec<f64>> {
    // A direct scalar is coerced (`LARGE("5",1)`, `SMALL(TRUE,1)`); references and arrays are not.
    if let Arg::Value(v) = arg {
        return Ok(vec![v.to_number_in(ctx.date1904)?]);
    }
    let mut out = Vec::new();
    for v in ctx.cells(arg) {
        match v {
            Value::Number(n) => out.push(n),
            Value::Error(e) => return Err(e),
            _ => {}
        }
    }
    Ok(out)
}

pub(super) fn median(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let nums = sorted(collect(ctx, args)?);
    let n = nums.len();
    if n == 0 {
        return Err(ErrorKind::Num);
    }
    number(if n % 2 == 1 {
        nums[n / 2]
    } else {
        (nums[n / 2 - 1] + nums[n / 2]) / 2.0
    })
}

pub(super) fn mode(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, usize::MAX)?;
    // Only numbers count, even among the direct arguments; text and booleans are skipped.
    let mut nums = Vec::new();
    for arg in args {
        for v in ctx.cells(arg) {
            match v {
                Value::Number(n) => nums.push(n),
                Value::Error(e) => return Err(e),
                _ => {}
            }
        }
    }
    mode_of(&nums)
}

fn mode_of(nums: &[f64]) -> Res<Value> {
    let mut best: Option<(usize, f64)> = None;
    for (i, &x) in nums.iter().enumerate() {
        let count = nums.iter().filter(|&&y| y == x).count();
        let first = nums.iter().position(|&y| y == x) == Some(i);
        if first && count > 1 && best.is_none_or(|(c, _)| count > c) {
            best = Some((count, x));
        }
    }
    best.map(|(_, x)| Value::Number(x)).ok_or(ErrorKind::NA)
}

/// 1-based ascending position selected by `k`: `SMALL` floors `k`, `LARGE` floors `n + 1 - k`.
fn order_position(ctx: &EvalContext<'_>, arg: &Arg, len: usize, largest: bool) -> Res<usize> {
    let k = num_arg(ctx, arg)?;
    let pos = if largest {
        (len as f64 + 1.0 - k).floor()
    } else {
        k.floor()
    };
    if pos < 1.0 || pos > len as f64 {
        return Err(ErrorKind::Num);
    }
    Ok(pos as usize)
}

pub(super) fn large(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let nums = sorted(array_numbers(ctx, &args[0])?);
    let pos = order_position(ctx, &args[1], nums.len(), true)?;
    Ok(Value::Number(nums[pos - 1]))
}

pub(super) fn small(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let nums = sorted(array_numbers(ctx, &args[0])?);
    let pos = order_position(ctx, &args[1], nums.len(), false)?;
    Ok(Value::Number(nums[pos - 1]))
}

#[derive(Clone, Copy, PartialEq)]
enum Ties {
    Top,
    Average,
}

fn rank_impl(ctx: &EvalContext<'_>, args: &[Arg], ties: Ties) -> Res<Value> {
    arity(args, 2, 3)?;
    let x = num_arg(ctx, &args[0])?;
    // The reference must be a range or array of numbers.
    if matches!(args[1], Arg::Value(_) | Arg::Missing | Arg::Array(_)) {
        return Err(ErrorKind::Value);
    }
    let nums = array_numbers(ctx, &args[1])?;
    let ascending = match optional(args, 2) {
        Some(a) => num_arg(ctx, a)? != 0.0,
        None => false,
    };
    let before = |y: f64| if ascending { y < x } else { y > x };
    let equal = nums.iter().filter(|&&y| y == x).count();
    if equal == 0 {
        return Err(ErrorKind::NA);
    }
    let above = nums.iter().filter(|&&y| before(y)).count() as f64;
    number(match ties {
        Ties::Top => above + 1.0,
        Ties::Average => above + 1.0 + (equal as f64 - 1.0) / 2.0,
    })
}

pub(super) fn rank(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    rank_impl(ctx, args, Ties::Top)
}

pub(super) fn rank_avg(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    rank_impl(ctx, args, Ties::Average)
}

/// Linear interpolation between order statistics at the 1-based fractional position `pos`. Excel forms
/// the position first and takes its fractional part, so `PERCENTILE({1,2,3,4,10},0.9)` ends in `...98`.
fn interpolate(sorted: &[f64], pos: f64) -> f64 {
    let lo = pos.floor();
    let frac = pos - lo;
    let i = lo as usize;
    if frac == 0.0 || i >= sorted.len() {
        sorted[i.saturating_sub(1).min(sorted.len() - 1)]
    } else {
        sorted[i - 1] + frac * (sorted[i] - sorted[i - 1])
    }
}

fn percentile_inc_of(sorted: &[f64], k: f64) -> Res<f64> {
    if sorted.is_empty() || !(0.0..=1.0).contains(&k) {
        return Err(ErrorKind::Num);
    }
    Ok(interpolate(sorted, k * (sorted.len() - 1) as f64 + 1.0))
}

fn percentile_exc_of(sorted: &[f64], k: f64) -> Res<f64> {
    let n = sorted.len() as f64;
    if sorted.is_empty() || k < 1.0 / (n + 1.0) || k > n / (n + 1.0) {
        return Err(ErrorKind::Num);
    }
    Ok(interpolate(sorted, k * (n + 1.0)))
}

fn percentile_args(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<(Vec<f64>, f64)> {
    arity(args, 2, 2)?;
    let nums = sorted(array_numbers(ctx, &args[0])?);
    let k = num_arg(ctx, &args[1])?;
    Ok((nums, k))
}

pub(super) fn percentile(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let (nums, k) = percentile_args(ctx, args)?;
    number(percentile_inc_of(&nums, k)?)
}

pub(super) fn percentile_exc(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let (nums, k) = percentile_args(ctx, args)?;
    number(percentile_exc_of(&nums, k)?)
}

/// The quartile number, truncated.
fn quart(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<(Vec<f64>, f64)> {
    let (nums, q) = percentile_args(ctx, args)?;
    Ok((nums, q.trunc()))
}

pub(super) fn quartile(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let (nums, q) = quart(ctx, args)?;
    if !(0.0..=4.0).contains(&q) {
        return Err(ErrorKind::Num);
    }
    number(percentile_inc_of(&nums, q / 4.0)?)
}

pub(super) fn quartile_exc(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let (nums, q) = quart(ctx, args)?;
    if !(1.0..=3.0).contains(&q) {
        return Err(ErrorKind::Num);
    }
    number(percentile_exc_of(&nums, q / 4.0)?)
}

/// Variance as Excel's last digits show it: the sample variance is `(Σx² - mean·Σx) / (n-1)` and the
/// population variance `(n·Σx² - (Σx)²) / n²`, both on plain left-to-right sums.
fn variance(ctx: &EvalContext<'_>, args: &[Arg], sample: bool, root: bool) -> Res<Value> {
    variance_of(&collect(ctx, args)?, sample, root)
}

pub(super) fn variance_of(nums: &[f64], sample: bool, root: bool) -> Res<Value> {
    let n = nums.len() as f64;
    if nums.len() < usize::from(sample) + 1 {
        return Err(ErrorKind::DivZero);
    }
    let (mut s, mut s2) = (0.0, 0.0);
    for x in nums {
        s += x;
        s2 += x * x;
    }
    if !s2.is_finite() {
        return Err(ErrorKind::Num);
    }
    let divisor = if sample { n - 1.0 } else { n };
    let mut v = if sample {
        (s2 - s / n * s) / divisor
    } else {
        (n * s2 - s * s) / (n * n)
    };
    // When the sum-of-squares form cancels almost everything, fall back to the two-pass form.
    if v < s2 * 1e-9 / divisor {
        let mean = s / n;
        let mut ss = 0.0;
        for x in nums {
            ss += (x - mean) * (x - mean);
        }
        v = ss / divisor;
    }
    number(if root { v.max(0.0).sqrt() } else { v.max(0.0) })
}

pub(super) fn stdev_s(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    variance(ctx, args, true, true)
}

pub(super) fn stdev_p(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    variance(ctx, args, false, true)
}

pub(super) fn var_s(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    variance(ctx, args, true, false)
}

pub(super) fn var_p(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    variance(ctx, args, false, false)
}

/// Values seen by `AVERAGEA`, `MAXA` and `MINA`: text in a reference counts as 0 and booleans as 1/0;
/// blanks are skipped. Direct arguments are coerced.
fn a_values(ctx: &EvalContext<'_>, args: &[Arg], text_as_zero: bool) -> Res<Vec<f64>> {
    arity(args, 1, usize::MAX)?;
    let mut out = Vec::new();
    for arg in args {
        match arg {
            Arg::Missing => out.push(0.0),
            Arg::Value(v) => out.push(match v {
                Value::Number(n) => *n,
                Value::Bool(b) => f64::from(u8::from(*b)),
                Value::Empty => 0.0,
                Value::Text(t) => parse_number_text_in(t, ctx.date1904).ok_or(ErrorKind::Value)?,
                Value::Error(e) => return Err(*e),
                Value::Array(_) => return Err(ErrorKind::Value),
            }),
            _ => {
                let literal_array = matches!(arg, Arg::Array(_));
                for v in ctx.cells(arg) {
                    match v {
                        Value::Number(n) => out.push(n),
                        Value::Bool(b) => out.push(f64::from(u8::from(b))),
                        // Text in a reference counts as 0; text in an array constant is ignored.
                        Value::Text(_) if literal_array && !text_as_zero => {}
                        Value::Text(_) => out.push(0.0),
                        Value::Error(e) => return Err(e),
                        Value::Empty | Value::Array(_) => {}
                    }
                }
            }
        }
    }
    Ok(out)
}

pub(super) fn averagea(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let v = a_values(ctx, args, true)?;
    if v.is_empty() {
        return Err(ErrorKind::DivZero);
    }
    number(v.iter().sum::<f64>() / v.len() as f64)
}

pub(super) fn maxa(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    Ok(Value::Number(
        a_values(ctx, args, false)?
            .into_iter()
            .reduce(f64::max)
            .unwrap_or(0.0),
    ))
}

pub(super) fn mina(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    Ok(Value::Number(
        a_values(ctx, args, false)?
            .into_iter()
            .reduce(f64::min)
            .unwrap_or(0.0),
    ))
}

pub(super) fn sumproduct(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, usize::MAX)?;
    let mut grids = Vec::new();
    for arg in args {
        grids.push(ctx.grid(arg).ok_or(ErrorKind::Value)?);
    }
    let (rows, cols) = (grids[0].rows(), grids[0].cols());
    if grids.iter().any(|g| g.rows() != rows || g.cols() != cols) {
        return Err(ErrorKind::Value);
    }
    let scan_rows = grids.iter().map(|g| g.scan_rows()).max().unwrap_or(0);
    let scan_cols = grids.iter().map(|g| g.scan_cols()).max().unwrap_or(0);
    let mut total = 0.0;
    for r in 0..scan_rows {
        for c in 0..scan_cols {
            let mut product = 1.0;
            for g in &grids {
                match g.get(r, c) {
                    Value::Number(n) => product *= n,
                    Value::Error(e) => return Err(e),
                    _ => product = 0.0,
                }
            }
            total += product;
        }
    }
    number(total)
}

// ---- SUBTOTAL and AGGREGATE ---------------------------------------------------------------------

/// What the reference arguments of `SUBTOTAL`/`AGGREGATE` yield.
struct RefValues {
    numbers: Vec<f64>,
    /// Non-blank cells (for COUNTA).
    non_blank: usize,
}

/// Reads the reference arguments. Cells holding a `SUBTOTAL`/`AGGREGATE` formula are skipped when
/// `skip_nested`; errors are skipped when `skip_errors` and otherwise returned.
fn reference_values(
    ctx: &EvalContext<'_>,
    args: &[Arg],
    skip_nested: bool,
    skip_errors: bool,
) -> Res<RefValues> {
    let mut out = RefValues {
        numbers: Vec::new(),
        non_blank: 0,
    };
    for arg in args {
        let ranges: Vec<&crate::eval::RangeRef> = match arg {
            Arg::Range(r) => vec![r],
            Arg::Union(rs) => rs.iter().collect(),
            Arg::Value(Value::Error(e)) => return Err(*e),
            _ => return Err(ErrorKind::Value),
        };
        for r in ranges {
            let (rows, cols) = (ctx.scan_rows(r), ctx.scan_cols(r));
            for dr in 0..rows {
                for dc in 0..cols {
                    let (row, col) = (r.row1 + dr, r.col1 + dc);
                    if skip_nested && ctx.source.is_subtotal(&r.sheet, row, col) {
                        continue;
                    }
                    match ctx.source.cell(&r.sheet, row, col) {
                        Value::Number(n) => {
                            out.numbers.push(n);
                            out.non_blank += 1;
                        }
                        Value::Error(e) => {
                            if !skip_errors {
                                return Err(e);
                            }
                        }
                        Value::Empty => {}
                        _ => out.non_blank += 1,
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Applies aggregate function number 1..=13 (`AVERAGE` ... `MODE.SNGL`, the numbering shared by both functions).
fn apply_aggregate(code: u32, values: &RefValues) -> Res<Value> {
    let nums = &values.numbers;
    match code {
        1 => {
            if nums.is_empty() {
                Err(ErrorKind::DivZero)
            } else {
                number(super::math::total(nums) / nums.len() as f64)
            }
        }
        2 => Ok(Value::Number(nums.len() as f64)),
        3 => Ok(Value::Number(values.non_blank as f64)),
        4 => Ok(Value::Number(
            nums.iter().copied().reduce(f64::max).unwrap_or(0.0),
        )),
        5 => Ok(Value::Number(
            nums.iter().copied().reduce(f64::min).unwrap_or(0.0),
        )),
        6 => Ok(Value::Number(if nums.is_empty() {
            0.0
        } else {
            nums.iter().product()
        })),
        7 => variance_of(nums, true, true),
        8 => variance_of(nums, false, true),
        9 => number(super::math::total(nums)),
        10 => variance_of(nums, true, false),
        11 => variance_of(nums, false, false),
        12 => {
            let s = sorted(nums.clone());
            if s.is_empty() {
                return Err(ErrorKind::Num);
            }
            let n = s.len();
            number(if n % 2 == 1 {
                s[n / 2]
            } else {
                (s[n / 2 - 1] + s[n / 2]) / 2.0
            })
        }
        _ => mode_of(nums),
    }
}

pub(super) fn subtotal(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, usize::MAX)?;
    let code = num_arg(ctx, &args[0])?.trunc() as i64;
    // 101..=111 ignore manually hidden rows; this engine has no hidden rows, so they equal 1..=11.
    let code = match code {
        1..=11 => code,
        101..=111 => code - 100,
        _ => return Err(ErrorKind::Value),
    };
    let values = reference_values(ctx, &args[1..], true, false)?;
    apply_aggregate(code as u32, &values)
}

pub(super) fn aggregate(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, usize::MAX)?;
    let code = num_arg(ctx, &args[0])?.trunc() as i64;
    let options = num_arg(ctx, &args[1])?.trunc();
    if !(1..=19).contains(&code) || !(0.0..=7.0).contains(&options) {
        return Err(ErrorKind::Value);
    }
    let options = options as u32;
    let skip_errors = matches!(options, 2 | 3 | 6 | 7);
    let skip_nested = options <= 3;
    if code <= 13 {
        let values = reference_values(ctx, &args[2..], skip_nested, skip_errors)?;
        return apply_aggregate(code as u32, &values);
    }
    // 14..=19 take an array and k.
    arity(args, 4, 4)?;
    let nums = match &args[2] {
        Arg::Range(_) | Arg::Union(_) => {
            reference_values(ctx, &args[2..3], skip_nested, skip_errors)?.numbers
        }
        Arg::Array(rows) => {
            let mut v = Vec::new();
            for x in rows.iter().flatten() {
                match x {
                    Value::Number(n) => v.push(*n),
                    Value::Error(e) if !skip_errors => return Err(*e),
                    _ => {}
                }
            }
            v
        }
        _ => return Err(ErrorKind::Value),
    };
    let k = num_arg(ctx, &args[3])?;
    let s = sorted(nums);
    let position = |pos: f64| -> Res<Value> {
        if pos < 1.0 || pos > s.len() as f64 {
            Err(ErrorKind::Num)
        } else {
            Ok(Value::Number(s[pos as usize - 1]))
        }
    };
    match code {
        14 => position((s.len() as f64 + 1.0 - k).floor()),
        15 => position(k.floor()),
        16 => number(percentile_inc_of(&s, k)?),
        17 => {
            let q = k.trunc();
            if !(0.0..=4.0).contains(&q) {
                return Err(ErrorKind::Num);
            }
            number(percentile_inc_of(&s, q / 4.0)?)
        }
        18 => number(percentile_exc_of(&s, k)?),
        _ => {
            let q = k.trunc();
            if !(1.0..=3.0).contains(&q) {
                return Err(ErrorKind::Num);
            }
            number(percentile_exc_of(&s, q / 4.0)?)
        }
    }
}
