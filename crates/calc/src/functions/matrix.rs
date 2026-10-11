//! Matrix functions (`MMULT`, `MDETERM`, `MINVERSE`, `MUNIT`) and the paired-array sums `SUMX2MY2`, `SUMX2PY2`,
//! `SUMXMY2`. Measured against Excel; see docs/testing/calc-coverage.md.

use super::{Res, arity, number, scalar};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

/// Parameters that Excel evaluates over whole arrays.
pub(super) fn takes_arrays(name: &str, _index: usize) -> bool {
    [
        "MMULT", "MDETERM", "MINVERSE", "SUMX2MY2", "SUMX2PY2", "SUMXMY2",
    ]
    .iter()
    .any(|f| f.eq_ignore_ascii_case(name))
}

type Mat = Vec<Vec<f64>>;

/// The cells of an argument as rows of values.
fn rows_of(ctx: &EvalContext<'_>, arg: &Arg) -> Res<Vec<Vec<Value>>> {
    match arg {
        Arg::Missing => Ok(vec![vec![Value::Empty]]),
        Arg::Value(Value::Error(e)) => Err(*e),
        Arg::Value(v) => Ok(vec![vec![v.clone()]]),
        Arg::Array(rows) => Ok(rows.clone()),
        Arg::Range(r) => Ok((0..r.rows())
            .map(|dr| {
                (0..r.cols())
                    .map(|dc| ctx.source.cell(&r.sheet, r.row1 + dr, r.col1 + dc))
                    .collect()
            })
            .collect()),
        _ => Err(ErrorKind::Value),
    }
}

/// A numeric matrix: any non-number is `#VALUE!`, an error cell is that error.
fn matrix(ctx: &EvalContext<'_>, arg: &Arg) -> Res<Mat> {
    let rows = rows_of(ctx, arg)?;
    let mut out = Vec::with_capacity(rows.len());
    let mut first_error = None;
    let mut bad = false;
    for row in &rows {
        let mut r = Vec::with_capacity(row.len());
        for v in row {
            match v {
                Value::Number(n) => r.push(*n),
                Value::Error(e) => {
                    first_error.get_or_insert(*e);
                    r.push(0.0);
                }
                _ => {
                    bad = true;
                    r.push(0.0);
                }
            }
        }
        out.push(r);
    }
    if bad {
        return Err(ErrorKind::Value);
    }
    if let Some(e) = first_error {
        return Err(e);
    }
    Ok(out)
}

fn to_value(m: &Mat) -> Res<Value> {
    if m.iter().flatten().any(|v| !v.is_finite()) {
        return Err(ErrorKind::Num);
    }
    Ok(Value::Array(
        m.iter()
            .map(|r| r.iter().map(|v| Value::Number(*v)).collect())
            .collect(),
    ))
}

#[allow(clippy::needless_range_loop)]
pub(super) fn mmult(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let a = matrix(ctx, &args[0])?;
    let b = matrix(ctx, &args[1])?;
    let (n, m) = (a.len(), a.first().map_or(0, Vec::len));
    let (m2, k) = (b.len(), b.first().map_or(0, Vec::len));
    if m != m2 {
        return Err(ErrorKind::Value);
    }
    let mut out = vec![vec![0.0; k]; n];
    for i in 0..n {
        for j in 0..k {
            let mut s = 0.0;
            for t in 0..m {
                s += a[i][t] * b[t][j];
            }
            out[i][j] = s;
        }
    }
    to_value(&out)
}

fn square(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Mat> {
    arity(args, 1, 1)?;
    let a = matrix(ctx, &args[0])?;
    let n = a.len();
    if n == 0 || a.iter().any(|r| r.len() != n) {
        return Err(ErrorKind::Value);
    }
    Ok(a)
}

/// LU decomposition with partial pivoting (in place). Returns the row permutation (`perm[i]` is the original row
/// now at position `i`), or `None` when a pivot is exactly zero.
#[allow(clippy::needless_range_loop)]
fn lu(a: &mut Mat) -> Option<Vec<usize>> {
    let n = a.len();
    let mut perm: Vec<usize> = (0..n).collect();
    for c in 0..n {
        let mut p = c;
        for r in c + 1..n {
            if a[r][c].abs() > a[p][c].abs() {
                p = r;
            }
        }
        if a[p][c] == 0.0 {
            return None;
        }
        if p != c {
            a.swap(p, c);
            perm.swap(p, c);
        }
        for r in c + 1..n {
            let f = a[r][c] / a[c][c];
            a[r][c] = f;
            for k in c + 1..n {
                a[r][k] -= f * a[c][k];
            }
        }
    }
    Some(perm)
}

/// Sign of a permutation.
fn parity(perm: &[usize]) -> f64 {
    let mut seen = vec![false; perm.len()];
    let mut sign = 1.0;
    for start in 0..perm.len() {
        if seen[start] {
            continue;
        }
        let (mut j, mut len) = (start, 0);
        while !seen[j] {
            seen[j] = true;
            j = perm[j];
            len += 1;
        }
        if len % 2 == 0 {
            sign = -sign;
        }
    }
    sign
}

pub(super) fn mdeterm(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let mut a = square(ctx, args)?;
    let n = a.len();
    let Some(perm) = lu(&mut a) else {
        return Ok(Value::Number(0.0));
    };
    let mut det = parity(&perm);
    for (i, row) in a.iter().enumerate().take(n) {
        det *= row[i];
    }
    number(det)
}

#[allow(clippy::needless_range_loop)]
pub(super) fn minverse(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let mut a = square(ctx, args)?;
    let n = a.len();
    let perm = lu(&mut a).ok_or(ErrorKind::Num)?;
    // Solves A x = e_j for each unit vector with the LU factors (bit-identical to Excel on 147 random matrices).
    let mut inv = vec![vec![0.0; n]; n];
    for col in 0..n {
        let mut y = vec![0.0; n];
        for i in 0..n {
            let mut s = if perm[i] == col { 1.0 } else { 0.0 };
            for k in 0..i {
                s -= a[i][k] * y[k];
            }
            y[i] = s;
        }
        let mut x = vec![0.0; n];
        for i in (0..n).rev() {
            let mut s = y[i];
            for k in i + 1..n {
                s -= a[i][k] * x[k];
            }
            x[i] = s / a[i][i];
        }
        for i in 0..n {
            inv[i][col] = x[i];
        }
    }
    to_value(&inv)
}

pub(super) fn munit(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let n = scalar(ctx, &args[0])?.to_number_in(ctx.date1904)?.trunc();
    if n < 1.0 {
        return Err(ErrorKind::Value);
    }
    if n * n > 4.0e6 {
        return Err(ErrorKind::Value);
    }
    let n = n as usize;
    Ok(Value::Array(
        (0..n)
            .map(|i| {
                (0..n)
                    .map(|j| Value::Number(if i == j { 1.0 } else { 0.0 }))
                    .collect()
            })
            .collect(),
    ))
}

/// Sums `f(x, y)` over the pairs of two arrays with the same number of cells (paired in row-major order); a pair
/// with a non-number on either side is skipped. An array without any number is `#DIV/0!`.
fn pair_sum(ctx: &EvalContext<'_>, args: &[Arg], f: fn(f64, f64) -> f64) -> Res<Value> {
    arity(args, 2, 2)?;
    // A blank or omitted single value is `#VALUE!`.
    for arg in args {
        let blank = match arg {
            Arg::Missing => true,
            Arg::Range(r) if r.is_single_cell() => {
                scalar(ctx, arg).is_ok_and(|v| v == Value::Empty)
            }
            _ => false,
        };
        if blank {
            return Err(ErrorKind::Value);
        }
    }
    let a: Vec<Value> = rows_of(ctx, &args[0])?.into_iter().flatten().collect();
    let b: Vec<Value> = rows_of(ctx, &args[1])?.into_iter().flatten().collect();
    if a.len() != b.len() {
        return Err(ErrorKind::NA);
    }
    let mut total = 0.0;
    for (x, y) in a.iter().zip(&b) {
        match (x, y) {
            (Value::Error(e), _) | (_, Value::Error(e)) => return Err(*e),
            (Value::Number(x), Value::Number(y)) => total += f(*x, *y),
            _ => {}
        }
    }
    let has_number = |v: &[Value]| v.iter().any(|x| matches!(x, Value::Number(_)));
    if !has_number(&a) || !has_number(&b) {
        return Err(ErrorKind::DivZero);
    }
    number(total)
}

pub(super) fn sumx2my2(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    pair_sum(ctx, args, |x, y| x * x - y * y)
}
pub(super) fn sumx2py2(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    pair_sum(ctx, args, |x, y| x * x + y * y)
}
pub(super) fn sumxmy2(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    pair_sum(ctx, args, |x, y| (x - y) * (x - y))
}
