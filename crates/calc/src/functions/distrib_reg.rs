//! Array statistics: `LINEST`/`LOGEST`/`TREND`/`GROWTH`, `FREQUENCY`, `PROB`, `PERCENTRANK` and `MODE.MULT`.
#![allow(
    clippy::approx_constant,
    clippy::excessive_precision,
    clippy::manual_range_contains,
    clippy::needless_range_loop
)]

use super::distrib::NUM;
use super::distribdd::{self as dd, Dd};
use super::{Res, arity, bool_arg, num_arg, number, optional};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

fn numbers(ctx: &EvalContext<'_>, arg: &Arg) -> Res<Vec<f64>> {
    let mut v = Vec::new();
    for x in ctx.cells(arg) {
        match x {
            Value::Number(n) => v.push(n),
            Value::Error(e) => return Err(e),
            _ => {}
        }
    }
    Ok(v)
}

fn column(values: Vec<Value>) -> Value {
    Value::Array(values.into_iter().map(|v| vec![v]).collect())
}

// ---- FREQUENCY, MODE.MULT, PROB ------------------------------------------------------------------

pub(super) fn frequency(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let data = numbers(ctx, &args[0])?;
    // Non-numeric bins are dropped, errors included (`FREQUENCY(x, {3,#N/A,8})` has two bins); an error in
    // the data fills every element.
    let bins: Vec<f64> = ctx
        .cells(&args[1])
        .filter_map(|v| match v {
            Value::Number(n) => Some(n),
            _ => None,
        })
        .collect();
    if bins.is_empty() {
        // Without a bin Excel still returns two elements: nothing at or below the (absent) bin and every value above.
        return Ok(column(vec![
            Value::Number(0.0),
            Value::Number(data.len() as f64),
        ]));
    }
    let mut counts = vec![0.0; bins.len() + 1];
    let mut sorted: Vec<f64> = bins.clone();
    sorted.sort_by(f64::total_cmp);
    sorted.dedup();
    for x in &data {
        // the first bin value that is >= x
        match sorted.iter().position(|b| x <= b) {
            Some(p) => {
                let bin = sorted[p];
                let slot = bins.iter().position(|b| *b == bin).unwrap_or(0);
                counts[slot] += 1.0;
            }
            None => counts[bins.len()] += 1.0,
        }
    }
    Ok(column(counts.into_iter().map(Value::Number).collect()))
}

pub(super) fn mode_mult(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    if args.is_empty() {
        return Err(ErrorKind::Value);
    }
    let mut all = Vec::new();
    for a in args {
        all.extend(numbers(ctx, a)?);
    }
    let mut counts: Vec<(f64, usize)> = Vec::new();
    for x in &all {
        match counts.iter_mut().find(|(v, _)| v == x) {
            Some(c) => c.1 += 1,
            None => counts.push((*x, 1)),
        }
    }
    let best = counts.iter().map(|c| c.1).max().unwrap_or(0);
    if best < 2 {
        return Err(ErrorKind::NA);
    }
    Ok(column(
        counts
            .into_iter()
            .filter(|c| c.1 == best)
            .map(|c| Value::Number(c.0))
            .collect(),
    ))
}

pub(super) fn prob(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 4)?;
    let xs: Vec<Value> = ctx.cells(&args[0]).collect();
    let ps: Vec<Value> = ctx.cells(&args[1]).collect();
    let lo = num_arg(ctx, &args[2])?;
    let hi = match optional(args, 3) {
        Some(a) => Some(num_arg(ctx, a)?),
        None => None,
    };
    if xs.len() != ps.len() {
        return Err(ErrorKind::NA);
    }
    let mut total = 0.0;
    let mut sum = 0.0;
    let upper = hi.unwrap_or(lo);
    for (x, p) in xs.iter().zip(&ps) {
        if let Value::Error(e) = x {
            return Err(*e);
        }
        if let Value::Error(e) = p {
            return Err(*e);
        }
        // A blank probability, like a zero one, drops the pair whatever the value is; a text or boolean
        // probability, or a non-number value beside a non-zero probability, is #NUM!. A probability is not
        // range-checked (`-0.1` is accepted); only their sum is.
        let p = match p {
            Value::Empty => continue,
            Value::Number(p) => *p,
            _ => return Err(NUM),
        };
        if p == 0.0 {
            continue;
        }
        let Value::Number(x) = x else {
            return Err(NUM);
        };
        total += p;
        let inside = if hi.is_some() {
            *x >= lo && *x <= upper
        } else {
            *x == lo
        };
        if inside {
            sum += p;
        }
    }
    // Measured on the double sum: 1 - 4.4E-16 up to 1 + 4.9E-15 is accepted, beyond that #NUM!.
    if total < 1.0 - 4.5e-16 || total > 1.0 + 5.0e-15 {
        return Err(NUM);
    }
    number(sum)
}

// ---- PERCENTRANK ---------------------------------------------------------------------------------

fn percentrank(ctx: &EvalContext<'_>, args: &[Arg], exclusive: bool) -> Res<Value> {
    arity(args, 2, 3)?;
    let mut a = numbers(ctx, &args[0])?;
    let x = num_arg(ctx, &args[1])?;
    let sig = match optional(args, 2) {
        Some(s) => num_arg(ctx, s)?.trunc(),
        None => 3.0,
    };
    if a.is_empty() {
        return Err(ErrorKind::NA);
    }
    if sig < 1.0 {
        return Err(NUM);
    }
    a.sort_by(f64::total_cmp);
    let n = a.len();
    if x < a[0] || x > a[n - 1] {
        return Err(ErrorKind::NA);
    }
    if n == 1 {
        // A single value ranks 1 in both variants (`PERCENTRANK.EXC({5},5)` is 1, not 1/2).
        return number(1.0);
    }
    let k = a.iter().rposition(|v| *v <= x).unwrap_or(0);
    let k = if a[k] == x {
        a.iter().position(|v| *v == x).unwrap_or(k)
    } else {
        k
    };
    let (pos, denom) = if a[k] == x {
        (k as f64, (n - 1) as f64)
    } else {
        (k as f64 + (x - a[k]) / (a[k + 1] - a[k]), (n - 1) as f64)
    };
    let rank = if exclusive {
        (pos + 1.0) / (n as f64 + 1.0)
    } else if n == 1 {
        1.0
    } else {
        pos / denom
    };
    let scale = 10f64.powf(sig.min(15.0));
    // Truncate after rounding to 15 significant digits: `PERCENTRANK(.1:.5, 0.15)` computes 0.12499999999999997
    // and Excel returns 0.125.
    number(round15(rank * scale).floor() / scale)
}

/// `x` rounded to 15 significant digits.
fn round15(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    let digits = 14 - x.abs().log10().floor() as i32;
    let m = 10f64.powi(digits.abs());
    if digits >= 0 {
        (x * m).round() / m
    } else {
        (x / m).round() * m
    }
}

pub(super) fn percentrank_inc(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    percentrank(ctx, args, false)
}

pub(super) fn percentrank_exc(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    percentrank(ctx, args, true)
}

// ---- regression ----------------------------------------------------------------------------------

struct Fit {
    /// Coefficients in the order x1..xk then the intercept (if fitted).
    coef: Vec<f64>,
    /// Columns found redundant.
    dropped: Vec<bool>,
    se: Vec<f64>,
    ssreg: f64,
    ssresid: f64,
    df: f64,
    n: usize,
    constant: bool,
}

/// Least squares by Householder QR on the columns `x1..xk` followed by the constant column.
fn least_squares(y: &[f64], xs: &[Vec<f64>], constant: bool) -> Fit {
    // Full-rank data keeps the natural column order (the last digits then agree with Excel more often); when a
    // column turns out redundant the fit is redone in Excel's pivoting order.
    let fit = least_squares_in(y, xs, constant, false);
    if fit.dropped.iter().any(|d| *d) {
        least_squares_in(y, xs, constant, true)
    } else {
        fit
    }
}

fn least_squares_in(y: &[f64], xs: &[Vec<f64>], constant: bool, excel_order: bool) -> Fit {
    let n = y.len();
    let k = xs.len();
    let p = k + usize::from(constant);
    // Excel pivots in the order constant, last x column, ..., first x column: of two collinear columns the earlier
    // one is dropped (its coefficient reads 0), and a column that is constant is dropped in favour of the intercept.
    // `order[j]` is the coefficient slot (x1..xk, then the constant) of the j-th column processed.
    let mut order: Vec<usize> = Vec::with_capacity(p);
    if excel_order {
        if constant {
            order.push(k);
        }
        order.extend((0..k).rev());
    } else {
        order.extend(0..p);
    }
    let cols: Vec<Vec<f64>> = order
        .iter()
        .map(|&slot| {
            if slot == k {
                vec![1.0; n]
            } else {
                xs[slot].clone()
            }
        })
        .collect();
    let norms: Vec<f64> = cols
        .iter()
        .map(|c| c.iter().map(|v| v * v).sum::<f64>().sqrt())
        .collect();
    let mut qty = y.to_vec();
    let mut dropped = vec![false; p];
    let mut r = vec![vec![0.0; p]; p];
    let mut row = 0;
    let mut used: Vec<usize> = Vec::new();
    let mut a = cols.clone();
    for j in 0..p {
        if row >= n {
            dropped[j] = true;
            continue;
        }
        let norm: f64 = a[j][row..].iter().map(|v| v * v).sum::<f64>().sqrt();
        if norm <= 1e-12 * norms[j].max(f64::MIN_POSITIVE) || norm == 0.0 {
            dropped[j] = true;
            continue;
        }
        let alpha = if a[j][row] > 0.0 { -norm } else { norm };
        let mut v: Vec<f64> = a[j][row..].to_vec();
        v[0] -= alpha;
        let vnorm2: f64 = v.iter().map(|t| t * t).sum();
        let apply = |col: &mut [f64]| {
            let dotp: f64 = v.iter().zip(&col[row..]).map(|(a, b)| a * b).sum();
            let f = 2.0 * dotp / vnorm2;
            for (c, vi) in col[row..].iter_mut().zip(&v) {
                *c -= f * vi;
            }
        };
        for jj in j..p {
            if !dropped[jj] || jj == j {
                apply(&mut a[jj]);
            }
        }
        apply(&mut qty);
        used.push(j);
        for (i, &uj) in used.iter().enumerate() {
            let _ = (i, uj);
        }
        row += 1;
    }
    // R is the upper triangle of `a` restricted to the used columns
    let m = used.len();
    for (ri, &uj) in used.iter().enumerate() {
        for (ci, &uc) in used.iter().enumerate() {
            if ci >= ri {
                r[ri][ci] = a[uc][ri];
            }
        }
        let _ = uj;
    }
    let mut beta_u = vec![0.0; m];
    for i in (0..m).rev() {
        let mut s = qty[i];
        for j in i + 1..m {
            s -= r[i][j] * beta_u[j];
        }
        beta_u[i] = s / r[i][i];
    }
    let mut coef = vec![0.0; p];
    for (i, &uj) in used.iter().enumerate() {
        coef[uj] = beta_u[i];
    }
    let ssresid: f64 = qty[m..].iter().map(|v| v * v).sum();
    // inverse of R, for the standard errors
    let mut rinv = vec![vec![0.0; m]; m];
    for c in 0..m {
        for i in (0..m).rev() {
            let mut s = if i == c { 1.0 } else { 0.0 };
            for j in i + 1..m {
                s -= r[i][j] * rinv[j][c];
            }
            rinv[i][c] = s / r[i][i];
        }
    }
    let df = (n - m) as f64;
    let mse = if df > 0.0 { ssresid / df } else { 0.0 };
    let mut se = vec![0.0; p];
    for (i, &uj) in used.iter().enumerate() {
        let d: f64 = (0..m).map(|c| rinv[i][c] * rinv[i][c]).sum();
        se[uj] = (mse * d).sqrt();
    }
    let ybar = y.iter().sum::<f64>() / n as f64;
    let sst: f64 = if constant {
        y.iter().map(|v| (v - ybar) * (v - ybar)).sum()
    } else {
        y.iter().map(|v| v * v).sum()
    };
    // back to the coefficient slots
    let (mut coef_s, mut dropped_s, mut se_s) = (vec![0.0; p], vec![true; p], vec![0.0; p]);
    for (j, &slot) in order.iter().enumerate() {
        coef_s[slot] = coef[j];
        dropped_s[slot] = dropped[j];
        se_s[slot] = se[j];
    }
    Fit {
        coef: coef_s,
        dropped: dropped_s,
        se: se_s,
        ssreg: sst - ssresid,
        ssresid,
        df,
        n,
        constant,
    }
}

/// Reads `known_y` and `known_x` into a response vector and variable columns.
fn read_xy(
    ctx: &EvalContext<'_>,
    y_arg: &Arg,
    x_arg: Option<&Arg>,
) -> Res<(Vec<f64>, Vec<Vec<f64>>, bool)> {
    let gy = ctx.grid(y_arg).ok_or(ErrorKind::Value)?;
    let (yr, yc) = (gy.rows(), gy.cols());
    let y_is_row = yr == 1 && yc > 1;
    if yr > 1 && yc > 1 {
        return Err(ErrorKind::Ref);
    }
    let mut y = Vec::new();
    for r in 0..yr {
        for c in 0..yc {
            match gy.get(r, c) {
                Value::Number(v) => y.push(v),
                // an error cell is #VALUE! here, not the error itself
                _ => return Err(ErrorKind::Value),
            }
        }
    }
    let n = y.len();
    let xs = match x_arg {
        None => vec![(1..=n).map(|i| i as f64).collect::<Vec<f64>>()],
        Some(xa) => {
            let gx = ctx.grid(xa).ok_or(ErrorKind::Value)?;
            let (xr, xc) = (gx.rows(), gx.cols());
            let by_cols = if y_is_row { false } else { xr == n };
            let by_rows = y_is_row && xc == n;
            if !by_cols && !by_rows {
                return Err(ErrorKind::Ref);
            }
            let mut cols: Vec<Vec<f64>> = Vec::new();
            let (vars, len) = if by_cols { (xc, xr) } else { (xr, xc) };
            for v in 0..vars {
                let mut col = Vec::with_capacity(len);
                for i in 0..len {
                    let cell = if by_cols { gx.get(i, v) } else { gx.get(v, i) };
                    match cell {
                        Value::Number(t) => col.push(t),
                        _ => return Err(ErrorKind::Value),
                    }
                }
                cols.push(col);
            }
            cols
        }
    };
    Ok((y, xs, y_is_row))
}

fn regression(ctx: &EvalContext<'_>, args: &[Arg], log: bool) -> Res<Value> {
    arity(args, 1, 4)?;
    let x_arg = optional(args, 1);
    let constant = match optional(args, 2) {
        Some(a) => bool_arg(ctx, a)?,
        None => true,
    };
    let stats = match optional(args, 3) {
        Some(a) => bool_arg(ctx, a)?,
        None => false,
    };
    let (mut y, xs, _) = read_xy(ctx, &args[0], x_arg)?;
    if y.is_empty() || xs.iter().any(|c| c.len() != y.len()) {
        return Err(ErrorKind::Ref);
    }
    if log {
        if y.iter().any(|v| *v <= 0.0) {
            return Err(NUM);
        }
        for v in &mut y {
            *v = dd::ln_p(Dd::from(*v)).to_f64();
        }
    }
    let k = xs.len();
    let fit = least_squares(&y, &xs, constant);
    let width = k + 1;
    let blank = Value::Error(ErrorKind::NA);
    let mut rows: Vec<Vec<Value>> = Vec::new();
    let conv = |c: f64| if log { dd::exp_f64(Dd::from(c)) } else { c };
    let mut first = Vec::new();
    for j in (0..k).rev() {
        first.push(Value::Number(if fit.dropped[j] {
            conv(0.0)
        } else {
            conv(fit.coef[j])
        }));
    }
    first.push(if constant {
        Value::Number(conv(fit.coef[k]))
    } else {
        Value::Number(conv(0.0))
    });
    rows.push(first);
    if stats {
        let mut se = Vec::new();
        for j in (0..k).rev() {
            se.push(Value::Number(if fit.dropped[j] { 0.0 } else { fit.se[j] }));
        }
        se.push(if constant {
            Value::Number(fit.se[k])
        } else {
            blank.clone()
        });
        rows.push(se);
        let sey = if fit.df > 0.0 {
            (fit.ssresid / fit.df).sqrt()
        } else {
            0.0
        };
        // A perfect fit (or one point per parameter) has r2 = 1, a standard error of 0 and no F statistic (#NUM!).
        let total = fit.ssreg + fit.ssresid;
        let r2 = if total > 0.0 { fit.ssreg / total } else { 1.0 };
        let mut r3 = vec![Value::Number(r2), Value::Number(sey)];
        let mut r4 = vec![Value::Number(f64::NAN), Value::Number(fit.df)];
        let mut r5 = vec![Value::Number(fit.ssreg), Value::Number(fit.ssresid)];
        let kk = (fit.n as f64 - fit.df - f64::from(u8::from(fit.constant))).max(0.0);
        let f = (fit.ssreg / kk) / (fit.ssresid / fit.df);
        r4[0] = if f.is_finite() {
            Value::Number(f)
        } else {
            Value::Error(NUM)
        };
        while r3.len() < width {
            r3.push(blank.clone());
            r4.push(blank.clone());
            r5.push(blank.clone());
        }
        rows.push(r3);
        rows.push(r4);
        rows.push(r5);
    }
    Ok(Value::Array(rows))
}

pub(super) fn linest(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    regression(ctx, args, false)
}

pub(super) fn logest(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    regression(ctx, args, true)
}

fn trend_like(ctx: &EvalContext<'_>, args: &[Arg], log: bool) -> Res<Value> {
    arity(args, 1, 4)?;
    let x_arg = optional(args, 1);
    let constant = match optional(args, 3) {
        Some(a) => bool_arg(ctx, a)?,
        None => true,
    };
    let (mut y, xs, y_is_row) = read_xy(ctx, &args[0], x_arg)?;
    if log {
        if y.iter().any(|v| *v <= 0.0) {
            return Err(NUM);
        }
        for v in &mut y {
            *v = dd::ln_p(Dd::from(*v)).to_f64();
        }
    }
    let k = xs.len();
    let fit = least_squares(&y, &xs, constant);
    // new x values
    let (new_rows, shape): (Vec<Vec<f64>>, (usize, usize)) = match optional(args, 2) {
        None => {
            let m = xs[0].len();
            (
                (0..m).map(|i| xs.iter().map(|c| c[i]).collect()).collect(),
                if y_is_row { (1, m) } else { (m, 1) },
            )
        }
        Some(a) => {
            let g = ctx.grid(a).ok_or(ErrorKind::Value)?;
            let (r, c) = (g.rows(), g.cols());
            let mut cells = vec![vec![0.0; c]; r];
            for (i, row) in cells.iter_mut().enumerate() {
                for (j, slot) in row.iter_mut().enumerate() {
                    *slot = match g.get(i, j) {
                        Value::Number(v) => v,
                        _ => return Err(ErrorKind::Value),
                    };
                }
            }
            if k == 1 {
                (cells.iter().flatten().map(|v| vec![*v]).collect(), (r, c))
            } else if y_is_row {
                if r != k {
                    return Err(ErrorKind::Ref);
                }
                (
                    (0..c)
                        .map(|j| (0..k).map(|i| cells[i][j]).collect())
                        .collect(),
                    (1, c),
                )
            } else {
                if c != k {
                    return Err(ErrorKind::Ref);
                }
                (cells, (r, 1))
            }
        }
    };
    let mut vals = Vec::with_capacity(new_rows.len());
    for row in &new_rows {
        let mut s = 0.0;
        for j in 0..k {
            if !fit.dropped[j] {
                s += fit.coef[j] * row[j];
            }
        }
        if constant {
            s += fit.coef[k];
        }
        vals.push(Value::Number(if log {
            dd::exp_f64(Dd::from(s))
        } else {
            s
        }));
    }
    let (r, c) = shape;
    let mut it = vals.into_iter();
    let rows: Vec<Vec<Value>> = (0..r)
        .map(|_| {
            (0..c)
                .map(|_| it.next().unwrap_or(Value::Error(ErrorKind::NA)))
                .collect()
        })
        .collect();
    Ok(Value::Array(rows))
}

pub(super) fn trend(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    trend_like(ctx, args, false)
}

pub(super) fn growth(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    trend_like(ctx, args, true)
}
