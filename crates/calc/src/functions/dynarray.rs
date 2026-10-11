//! Array-shaping functions: `SEQUENCE`, `TRANSPOSE`, `SORT`, `SORTBY`, `UNIQUE`, `FILTER`, `TAKE`, `DROP`,
//! `CHOOSECOLS`, `CHOOSEROWS`, `EXPAND`, `HSTACK`, `VSTACK`, `TOCOL`, `TOROW`, `WRAPCOLS`, `WRAPROWS`,
//! `AREAS` and `HYPERLINK`. Measured against Excel; see docs/testing/calc-conformance.md.

use std::cmp::Ordering;

use super::{Res, arity, bool_arg, optional, scalar};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value, compare_values};

type Rows = Vec<Vec<Value>>;

/// The cells of an argument as rows. A reference is read in full (clamped to the used area for whole columns).
fn rows_of(ctx: &EvalContext<'_>, arg: &Arg) -> Res<Rows> {
    match arg {
        Arg::Missing => Ok(vec![vec![Value::Empty]]),
        Arg::Value(Value::Error(e)) => Err(*e),
        Arg::Value(v) => Ok(vec![vec![v.clone()]]),
        Arg::Array(rows) => Ok(rows.clone()),
        Arg::Range(r) => {
            let (nr, nc) = (ctx.scan_rows(r), ctx.scan_cols(r));
            if nr == 0 || nc == 0 {
                return Ok(vec![vec![Value::Empty]]);
            }
            Ok((0..nr)
                .map(|dr| {
                    (0..nc)
                        .map(|dc| ctx.source.cell(&r.sheet, r.row1 + dr, r.col1 + dc))
                        .collect()
                })
                .collect())
        }
        Arg::Union(_) => Err(ErrorKind::Value),
        Arg::Refs(_) | Arg::Lambda(_) => Err(ErrorKind::Value),
    }
}

fn dims(rows: &Rows) -> (usize, usize) {
    (rows.len(), rows.first().map_or(0, Vec::len))
}

fn finish(rows: Rows) -> Res<Value> {
    if rows.is_empty() || rows[0].is_empty() {
        return Err(ErrorKind::Calc);
    }
    Ok(Value::Array(rows))
}

/// An integer parameter (truncated); text that is not a number is `#VALUE!`.
fn int_of(ctx: &EvalContext<'_>, arg: &Arg) -> Res<i64> {
    let n = scalar(ctx, arg)?.to_number_in(ctx.date1904)?.trunc();
    if n.abs() > 1e15 {
        return Err(ErrorKind::Value);
    }
    Ok(n as i64)
}

fn opt_int(ctx: &EvalContext<'_>, args: &[Arg], i: usize, default: i64) -> Res<i64> {
    match optional(args, i) {
        None => Ok(default),
        Some(a) => int_of(ctx, a),
    }
}

pub(super) fn sequence(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 4)?;
    let rows = int_of(ctx, &args[0])?;
    let cols = opt_int(ctx, args, 1, 1)?;
    let num = |i: usize, d: f64| -> Res<f64> {
        match optional(args, i) {
            None => Ok(d),
            Some(a) => scalar(ctx, a)?.to_number_in(ctx.date1904),
        }
    };
    let start = num(2, 1.0)?;
    let step = num(3, 1.0)?;
    if rows < 0 || cols < 0 || rows > 1_048_576 || cols > 16_384 {
        return Err(ErrorKind::Value);
    }
    if rows < 1 || cols < 1 {
        return Err(ErrorKind::Calc);
    }
    if rows.saturating_mul(cols) > 1 << 24 {
        return Err(ErrorKind::Num);
    }
    let mut out = Vec::new();
    for r in 0..rows {
        let mut row = Vec::new();
        for c in 0..cols {
            let k = (r * cols + c) as f64;
            row.push(Value::Number(start + k * step));
        }
        out.push(row);
    }
    Ok(Value::Array(out))
}

pub(super) fn transpose(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    // A reference is a scalar parameter here: it takes the implicit intersection.
    let rows = match &args[0] {
        Arg::Range(_) => vec![vec![scalar(ctx, &args[0])?]],
        a => rows_of(ctx, a)?,
    };
    let (nr, nc) = dims(&rows);
    let out = (0..nc)
        .map(|c| (0..nr).map(|r| rows[r][c].clone()).collect())
        .collect();
    finish(out)
}

fn rank(v: &Value) -> u8 {
    match v {
        Value::Number(_) => 0,
        Value::Text(_) => 1,
        Value::Bool(_) => 2,
        Value::Error(_) => 3,
        Value::Empty | Value::Array(_) => 4,
    }
}

fn sort_cmp(a: &Value, b: &Value, ascending: bool) -> Ordering {
    let (ra, rb) = (rank(a), rank(b));
    if ra != rb {
        // Blanks stay last in both directions.
        if ra == 4 || rb == 4 {
            return ra.cmp(&rb);
        }
        return if ascending { ra.cmp(&rb) } else { rb.cmp(&ra) };
    }
    if ra == 4 || ra == 3 {
        return Ordering::Equal;
    }
    let o = compare_values(a, b);
    if ascending { o } else { o.reverse() }
}

fn transposed(rows: &Rows) -> Rows {
    let (nr, nc) = dims(rows);
    (0..nc)
        .map(|c| (0..nr).map(|r| rows[r][c].clone()).collect())
        .collect()
}

pub(super) fn sort(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 4)?;
    let mut rows = rows_of(ctx, &args[0])?;
    let idx = opt_int(ctx, args, 1, 1)?;
    let order = opt_int(ctx, args, 2, 1)?;
    let by_col = match optional(args, 3) {
        None => false,
        Some(a) => bool_arg(ctx, a)?,
    };
    if order != 1 && order != -1 {
        return Err(ErrorKind::Value);
    }
    if by_col {
        rows = transposed(&rows);
    }
    let (_, nc) = dims(&rows);
    if idx < 1 || idx as usize > nc {
        return Err(ErrorKind::Value);
    }
    let k = idx as usize - 1;
    rows.sort_by(|a, b| sort_cmp(&a[k], &b[k], order == 1));
    if by_col {
        rows = transposed(&rows);
    }
    finish(rows)
}

pub(super) fn sortby(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    // Excel refuses a call with a key that has no order next to a further key (four arguments, six, ...).
    if args.len() < 2 || (args.len() >= 4 && args.len().is_multiple_of(2)) {
        return Err(ErrorKind::Value);
    }
    let rows = rows_of(ctx, &args[0])?;
    let (nr, nc) = dims(&rows);
    let mut keys: Vec<(Vec<Value>, bool)> = Vec::new();
    let mut by_col: Option<bool> = None;
    let mut i = 1;
    while i < args.len() {
        let key = rows_of(ctx, &args[i])?;
        let (kr, kc) = dims(&key);
        let col = if kc == 1 && kr == nr {
            false
        } else if kr == 1 && kc == nc {
            true
        } else {
            return Err(ErrorKind::Value);
        };
        if by_col.is_some_and(|b| b != col) {
            return Err(ErrorKind::Value);
        }
        by_col = Some(col);
        let order = match optional(args, i + 1) {
            None => 1,
            Some(a) => int_of(ctx, a)?,
        };
        if order != 1 && order != -1 {
            return Err(ErrorKind::Value);
        }
        keys.push((key.into_iter().flatten().collect(), order == 1));
        i += 2;
    }
    let n = if by_col == Some(true) { nc } else { nr };
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| {
        for (k, asc) in &keys {
            let o = sort_cmp(&k[a], &k[b], *asc);
            if o != Ordering::Equal {
                return o;
            }
        }
        Ordering::Equal
    });
    let out = if by_col == Some(true) {
        rows.iter()
            .map(|row| idx.iter().map(|&c| row[c].clone()).collect())
            .collect()
    } else {
        idx.iter().map(|&r| rows[r].clone()).collect()
    };
    finish(out)
}

/// Total order whose equal classes are the cells `UNIQUE` treats as the same value.
fn cell_cmp(a: &Value, b: &Value) -> Ordering {
    rank(a).cmp(&rank(b)).then_with(|| match (a, b) {
        (Value::Error(x), Value::Error(y)) => (*x as u8).cmp(&(*y as u8)),
        (Value::Empty, Value::Empty) => Ordering::Equal,
        _ => compare_values(a, b),
    })
}

fn row_cmp(a: &[Value], b: &[Value]) -> Ordering {
    a.iter()
        .zip(b)
        .map(|(x, y)| cell_cmp(x, y))
        .find(|o| *o != Ordering::Equal)
        .unwrap_or(Ordering::Equal)
}

pub(super) fn unique(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 3)?;
    let mut rows = rows_of(ctx, &args[0])?;
    let by_col = match optional(args, 1) {
        None => false,
        Some(a) => bool_arg(ctx, a)?,
    };
    let once = match optional(args, 2) {
        None => false,
        Some(a) => bool_arg(ctx, a)?,
    };
    if by_col {
        rows = transposed(&rows);
    }
    // Sort the row indices (stable) and group neighbours that compare equal: n log n instead of the n^2 of
    // comparing every row with every earlier distinct row. The first row of a group is its lowest index.
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by(|&a, &b| row_cmp(&rows[a], &rows[b]));
    let mut groups: Vec<(usize, usize)> = Vec::new();
    let mut last: Option<usize> = None;
    for &i in &order {
        match (last, groups.last_mut()) {
            (Some(l), Some(g)) if row_cmp(&rows[l], &rows[i]) == Ordering::Equal => g.1 += 1,
            _ => groups.push((i, 1)),
        }
        last = Some(i);
    }
    groups.sort_unstable_by_key(|g| g.0);
    let mut out: Rows = groups
        .into_iter()
        .filter(|(_, n)| !once || *n == 1)
        .map(|(i, _)| rows[i].clone())
        .collect();
    if by_col {
        out = transposed(&out);
    }
    finish(out)
}

pub(super) fn filter(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 3)?;
    let rows = rows_of(ctx, &args[0])?;
    let inc = rows_of(ctx, &args[1])?;
    let (nr, nc) = dims(&rows);
    let (ir, ic) = dims(&inc);
    let by_row = if ic == 1 && ir == nr {
        true
    } else if ir == 1 && ic == nc {
        false
    } else {
        return Err(ErrorKind::Value);
    };
    let mut flags = Vec::new();
    for v in inc.into_iter().flatten() {
        flags.push(match v {
            Value::Error(e) => return Err(e),
            Value::Number(n) => n != 0.0,
            Value::Bool(b) => b,
            Value::Empty => false,
            Value::Text(_) | Value::Array(_) => return Err(ErrorKind::Value),
        });
    }
    let out: Rows = if by_row {
        rows.into_iter()
            .zip(flags)
            .filter(|(_, f)| *f)
            .map(|(r, _)| r)
            .collect()
    } else {
        rows.into_iter()
            .map(|row| {
                row.into_iter()
                    .zip(&flags)
                    .filter(|(_, f)| **f)
                    .map(|(v, _)| v)
                    .collect()
            })
            .collect()
    };
    if out.is_empty() || out[0].is_empty() {
        return match optional(args, 2) {
            Some(a) => {
                if matches!(a, Arg::Range(r) if !r.is_single_cell()) {
                    return Err(ErrorKind::Value);
                }
                let r = rows_of(ctx, a)?;
                Ok(Value::Array(r))
            }
            None => Err(ErrorKind::Calc),
        };
    }
    Ok(Value::Array(out))
}

/// Which `count` of `n` items `TAKE`/`DROP` mean: positive from the start, negative from the end.
fn span(n: usize, count: i64, take: bool) -> Res<(usize, usize)> {
    let c = count.unsigned_abs() as usize;
    if take {
        if count == 0 {
            return Err(ErrorKind::Calc);
        }
        let c = c.min(n);
        Ok(if count > 0 { (0, c) } else { (n - c, n) })
    } else {
        let c = c.min(n);
        Ok(if count >= 0 { (c, n) } else { (0, n - c) })
    }
}

fn take_drop(ctx: &EvalContext<'_>, args: &[Arg], take: bool) -> Res<Value> {
    arity(args, 2, 3)?;
    let rows = rows_of(ctx, &args[0])?;
    let (nr, nc) = dims(&rows);
    let r = match optional(args, 1) {
        Some(a) => Some(int_of(ctx, a)?),
        None => None,
    };
    let c = match optional(args, 2) {
        Some(a) => Some(int_of(ctx, a)?),
        None => None,
    };
    let (r0, r1) = match r {
        Some(r) => span(nr, r, take)?,
        None => (0, nr),
    };
    let (c0, c1) = match c {
        Some(c) => span(nc, c, take)?,
        None => (0, nc),
    };
    let out: Rows = rows[r0..r1]
        .iter()
        .map(|row| row[c0..c1].to_vec())
        .collect();
    finish(out)
}

pub(super) fn take(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    take_drop(ctx, args, true)
}
pub(super) fn drop(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    take_drop(ctx, args, false)
}

fn choose(ctx: &EvalContext<'_>, args: &[Arg], cols: bool) -> Res<Value> {
    if args.len() < 2 {
        return Err(ErrorKind::Value);
    }
    let mut rows = rows_of(ctx, &args[0])?;
    if cols {
        rows = transposed(&rows);
    }
    let n = rows.len() as i64;
    let mut picked = Vec::new();
    for a in &args[1..] {
        let idx = match a {
            Arg::Range(_) => vec![vec![ctx.scalar(a)]],
            other => rows_of(ctx, other)?,
        };
        if idx.len() > 1 && idx[0].len() > 1 {
            return Err(ErrorKind::Value);
        }
        for v in idx.into_iter().flatten() {
            let k = match v {
                Value::Error(e) => return Err(e),
                v => v.to_number_in(ctx.date1904)?.trunc(),
            };
            let k = k as i64;
            if k == 0 || k.abs() > n {
                return Err(ErrorKind::Value);
            }
            let i = if k > 0 { k - 1 } else { n + k } as usize;
            picked.push(rows[i].clone());
        }
    }
    if cols {
        picked = transposed(&picked);
    }
    finish(picked)
}

pub(super) fn choosecols(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    choose(ctx, args, true)
}
pub(super) fn chooserows(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    choose(ctx, args, false)
}

pub(super) fn expand(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 4)?;
    let rows = rows_of(ctx, &args[0])?;
    let (nr, nc) = dims(&rows);
    let r = opt_int(ctx, args, 1, nr as i64)?;
    let c = opt_int(ctx, args, 2, nc as i64)?;
    let pad = match optional(args, 3) {
        None => Value::Error(ErrorKind::NA),
        Some(a) => ctx.scalar(a),
    };
    if r < nr as i64 || c < nc as i64 {
        return Err(ErrorKind::Value);
    }
    if r.saturating_mul(c) > 1 << 22 {
        return Err(ErrorKind::Value);
    }
    let out = (0..r as usize)
        .map(|i| {
            (0..c as usize)
                .map(|j| {
                    if i < nr && j < nc {
                        rows[i][j].clone()
                    } else {
                        pad.clone()
                    }
                })
                .collect()
        })
        .collect();
    finish(out)
}

fn stack(ctx: &EvalContext<'_>, args: &[Arg], horizontal: bool) -> Res<Value> {
    if args.is_empty() || args.iter().any(|a| matches!(a, Arg::Missing)) {
        return Err(ErrorKind::Value);
    }
    let mut parts: Vec<Rows> = Vec::new();
    for a in args {
        let mut p = rows_of(ctx, a)?;
        if horizontal {
            p = transposed(&p);
        }
        parts.push(p);
    }
    let width = parts.iter().map(|p| dims(p).1).max().unwrap_or(0);
    let mut out: Rows = Vec::new();
    for p in parts {
        for mut row in p {
            row.resize(width, Value::Error(ErrorKind::NA));
            out.push(row);
        }
    }
    if horizontal {
        out = transposed(&out);
    }
    finish(out)
}

pub(super) fn hstack(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    stack(ctx, args, true)
}
pub(super) fn vstack(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    stack(ctx, args, false)
}

fn flatten(ctx: &EvalContext<'_>, args: &[Arg], column: bool) -> Res<Value> {
    arity(args, 1, 3)?;
    let rows = rows_of(ctx, &args[0])?;
    let ignore = opt_int(ctx, args, 1, 0)?;
    if !(0..=3).contains(&ignore) {
        return Err(ErrorKind::Value);
    }
    let by_col = match optional(args, 2) {
        None => false,
        Some(a) => bool_arg(ctx, a)?,
    };
    let src = if by_col { transposed(&rows) } else { rows };
    let items: Vec<Value> = src
        .into_iter()
        .flatten()
        .filter(|v| match v {
            Value::Empty => ignore != 1 && ignore != 3,
            Value::Error(_) => ignore != 2 && ignore != 3,
            _ => true,
        })
        .collect();
    if items.is_empty() {
        return Err(ErrorKind::Calc);
    }
    if column {
        finish(items.into_iter().map(|v| vec![v]).collect())
    } else {
        finish(vec![items])
    }
}

pub(super) fn tocol(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    flatten(ctx, args, true)
}
pub(super) fn torow(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    flatten(ctx, args, false)
}

fn wrap(ctx: &EvalContext<'_>, args: &[Arg], by_rows: bool) -> Res<Value> {
    arity(args, 2, 3)?;
    let rows = rows_of(ctx, &args[0])?;
    let (nr, nc) = dims(&rows);
    if nr != 1 && nc != 1 {
        return Err(ErrorKind::Value);
    }
    let n = int_of(ctx, &args[1])?;
    if !(1..=1_048_576).contains(&n) {
        return Err(ErrorKind::Num);
    }
    let pad = match optional(args, 2) {
        None => Value::Error(ErrorKind::NA),
        Some(a) => ctx.scalar(a),
    };
    let items: Vec<Value> = rows.into_iter().flatten().collect();
    let n = (n as usize).min(items.len().max(1));
    let mut out: Rows = items
        .chunks(n)
        .map(|c| {
            let mut v = c.to_vec();
            v.resize(n, pad.clone());
            v
        })
        .collect();
    if !by_rows {
        out = transposed(&out);
    }
    finish(out)
}

pub(super) fn wrapcols(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    wrap(ctx, args, false)
}
pub(super) fn wraprows(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    wrap(ctx, args, true)
}

pub(super) fn areas(_ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    match &args[0] {
        Arg::Range(_) => Ok(Value::Number(1.0)),
        Arg::Union(rs) => Ok(Value::Number(rs.len() as f64)),
        Arg::Value(Value::Error(e)) => Err(*e),
        _ => Err(ErrorKind::Value),
    }
}

pub(super) fn hyperlink(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 2)?;
    let link = scalar(ctx, &args[0])?;
    match args.get(1) {
        None => Ok(Value::Text(link.to_text()?)),
        Some(Arg::Missing) => Ok(Value::Number(0.0)),
        Some(a) => Ok(match scalar(ctx, a)? {
            Value::Empty => Value::Number(0.0),
            v => v,
        }),
    }
}
