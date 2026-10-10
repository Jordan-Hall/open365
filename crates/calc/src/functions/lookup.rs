//! Lookup and reference functions: `VLOOKUP`, `HLOOKUP`, `INDEX`, `MATCH`, `XLOOKUP`, `CHOOSE`.
//!
//! Approximate matches scan forward and stop at the first element larger than the lookup
//! value, which gives Excel's answer on sorted data. Only elements of the same type as the
//! lookup value are considered (numbers never match text).

use std::cmp::Ordering;

use super::criteria::{has_wildcards, wildcard_match};
use super::{Res, arity, bool_arg, int_arg, num_arg, optional, scalar};
use crate::eval::{Arg, EvalContext, Grid};
use crate::value::{ErrorKind, Value, compare_numbers, compare_text, compare_values};

fn class(v: &Value) -> u8 {
    match v {
        Value::Number(_) => 0,
        Value::Text(_) => 1,
        Value::Bool(_) => 2,
        Value::Error(_) => 3,
        Value::Empty | Value::Array(_) => 4,
    }
}

fn lookup_value(ctx: &EvalContext<'_>, arg: &Arg) -> Res<Value> {
    Ok(match scalar(ctx, arg)? {
        Value::Empty => Value::Number(0.0),
        v => v,
    })
}

fn exact_eq(lookup: &Value, cell: &Value, wildcards: bool) -> bool {
    match (lookup, cell) {
        (Value::Number(a), Value::Number(b)) => compare_numbers(*a, *b) == Ordering::Equal,
        (Value::Text(a), Value::Text(b)) => {
            if wildcards && has_wildcards(a) {
                wildcard_match(a, b)
            } else {
                compare_text(a, b) == Ordering::Equal
            }
        }
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Error(a), Value::Error(b)) => a == b,
        _ => false,
    }
}

fn find_exact(
    lookup: &Value,
    n: usize,
    get: &dyn Fn(usize) -> Value,
    reverse: bool,
    wildcards: bool,
) -> Option<usize> {
    let mut order: Box<dyn Iterator<Item = usize>> = if reverse {
        Box::new((0..n).rev())
    } else {
        Box::new(0..n)
    };
    order.find(|&i| exact_eq(lookup, &get(i), wildcards))
}

/// Last element not greater than `lookup` in an ascending sequence.
fn find_last_le(lookup: &Value, n: usize, get: &dyn Fn(usize) -> Value) -> Option<usize> {
    let mut best = None;
    for i in 0..n {
        let v = get(i);
        if class(&v) != class(lookup) {
            continue;
        }
        if compare_values(&v, lookup) == Ordering::Greater {
            break;
        }
        best = Some(i);
    }
    best
}

/// Last element not less than `lookup` in a descending sequence.
fn find_last_ge(lookup: &Value, n: usize, get: &dyn Fn(usize) -> Value) -> Option<usize> {
    let mut best = None;
    for i in 0..n {
        let v = get(i);
        if class(&v) != class(lookup) {
            continue;
        }
        if compare_values(&v, lookup) == Ordering::Less {
            break;
        }
        best = Some(i);
    }
    best
}

fn table_grid<'a>(ctx: &'a EvalContext<'a>, arg: &'a Arg) -> Res<Grid<'a>> {
    match arg {
        Arg::Range(_) | Arg::Array(_) | Arg::Value(_) => ctx.grid(arg).ok_or(ErrorKind::Value),
        _ => Err(ErrorKind::Value),
    }
}

fn blank_to_zero(v: Value) -> Value {
    if v == Value::Empty {
        Value::Number(0.0)
    } else {
        v
    }
}

fn table_lookup(ctx: &EvalContext<'_>, args: &[Arg], vertical: bool) -> Res<Value> {
    arity(args, 3, 4)?;
    let lookup = lookup_value(ctx, &args[0])?;
    let grid = table_grid(ctx, &args[1])?;
    let index = int_arg(ctx, &args[2])?;
    let approximate = match args.get(3) {
        None => true,
        Some(Arg::Missing) => false,
        Some(a) => bool_arg(ctx, a)?,
    };
    if index < 1 {
        return Err(ErrorKind::Value);
    }
    let (key_len, span) = if vertical {
        (grid.scan_rows(), grid.cols())
    } else {
        (grid.scan_cols(), grid.rows())
    };
    if index as usize > span {
        return Err(ErrorKind::Ref);
    }
    let get = |i: usize| {
        if vertical {
            grid.get(i, 0)
        } else {
            grid.get(0, i)
        }
    };
    let found = if approximate {
        find_last_le(&lookup, key_len, &get)
    } else {
        find_exact(&lookup, key_len, &get, false, true)
    };
    let pos = found.ok_or(ErrorKind::NA)?;
    let cell = if vertical {
        grid.get(pos, index as usize - 1)
    } else {
        grid.get(index as usize - 1, pos)
    };
    Ok(blank_to_zero(cell))
}

pub(super) fn vlookup(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    table_lookup(ctx, args, true)
}

pub(super) fn hlookup(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    table_lookup(ctx, args, false)
}

/// A one-dimensional view of a grid: `(is_vertical, length)`.
fn vector_shape(grid: &Grid<'_>) -> Res<(bool, usize)> {
    if grid.cols() == 1 {
        Ok((true, grid.scan_rows()))
    } else if grid.rows() == 1 {
        Ok((false, grid.scan_cols()))
    } else {
        Err(ErrorKind::NA)
    }
}

pub(super) fn match_(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 3)?;
    let lookup = lookup_value(ctx, &args[0])?;
    let grid = table_grid(ctx, &args[1])?;
    let match_type = match optional(args, 2) {
        None => 1.0,
        Some(a) => num_arg(ctx, a)?,
    };
    let (vertical, n) = vector_shape(&grid)?;
    let get = |i: usize| {
        if vertical {
            grid.get(i, 0)
        } else {
            grid.get(0, i)
        }
    };
    let found = if match_type == 0.0 {
        find_exact(&lookup, n, &get, false, true)
    } else if match_type > 0.0 {
        find_last_le(&lookup, n, &get)
    } else {
        find_last_ge(&lookup, n, &get)
    };
    found
        .map(|i| Value::Number((i + 1) as f64))
        .ok_or(ErrorKind::NA)
}

pub(super) fn index(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 3)?;
    let grid = table_grid(ctx, &args[0])?;
    let (rows, cols) = (grid.rows(), grid.cols());
    let first = int_arg(ctx, &args[1])?;
    let second = optional(args, 2).map(|a| int_arg(ctx, a)).transpose()?;
    let (r, c) = match second {
        Some(c) => (first, c),
        None if rows == 1 => (1, first),
        None if cols == 1 => (first, 1),
        None => (first, 0),
    };
    if r < 0 || c < 0 {
        return Err(ErrorKind::Value);
    }
    let (r, c) = (r as usize, c as usize);
    if r > rows || c > cols {
        return Err(ErrorKind::Ref);
    }
    let row_span = if matches!(args[0], Arg::Range(_)) {
        grid.scan_rows()
    } else {
        rows
    };
    Ok(match (r, c) {
        (r, c) if r > 0 && c > 0 => grid.get(r - 1, c - 1),
        (0, c) if c > 0 => Value::Array((0..row_span).map(|i| vec![grid.get(i, c - 1)]).collect()),
        (r, 0) if r > 0 => Value::Array(vec![(0..cols).map(|j| grid.get(r - 1, j)).collect()]),
        _ => Value::Array(
            (0..row_span)
                .map(|i| (0..cols).map(|j| grid.get(i, j)).collect())
                .collect(),
        ),
    })
}

pub(super) fn xlookup(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 6)?;
    let lookup = lookup_value(ctx, &args[0])?;
    let keys = table_grid(ctx, &args[1])?;
    let results = table_grid(ctx, &args[2])?;
    let match_mode = match optional(args, 4) {
        None => 0,
        Some(a) => int_arg(ctx, a)?,
    };
    let search_mode = match optional(args, 5) {
        None => 1,
        Some(a) => int_arg(ctx, a)?,
    };
    if !(-1..=2).contains(&match_mode) || !matches!(search_mode, -2 | -1 | 1 | 2) {
        return Err(ErrorKind::Value);
    }
    let (vertical, n) = vector_shape(&keys).map_err(|_| ErrorKind::Value)?;
    let expected = if vertical { keys.rows() } else { keys.cols() };
    let actual = if vertical {
        results.rows()
    } else {
        results.cols()
    };
    if expected != actual {
        return Err(ErrorKind::Value);
    }
    let get = |i: usize| {
        if vertical {
            keys.get(i, 0)
        } else {
            keys.get(0, i)
        }
    };
    let reverse = search_mode < 0;
    let mut found = find_exact(&lookup, n, &get, reverse, match_mode == 2);
    if found.is_none() && (match_mode == 1 || match_mode == -1) {
        found = nearest(&lookup, n, &get, reverse, match_mode == 1);
    }
    let Some(i) = found else {
        return match optional(args, 3) {
            Some(fallback) => Ok(ctx.scalar(fallback)),
            None => Err(ErrorKind::NA),
        };
    };
    Ok(if vertical {
        if results.cols() == 1 {
            results.get(i, 0)
        } else {
            Value::Array(vec![
                (0..results.cols()).map(|c| results.get(i, c)).collect(),
            ])
        }
    } else if results.rows() == 1 {
        results.get(0, i)
    } else {
        Value::Array(
            (0..results.rows())
                .map(|r| vec![results.get(r, i)])
                .collect(),
        )
    })
}

/// Closest smaller (or larger) element of the same type, first in search order on ties.
fn nearest(
    lookup: &Value,
    n: usize,
    get: &dyn Fn(usize) -> Value,
    reverse: bool,
    larger: bool,
) -> Option<usize> {
    let wanted = if larger {
        Ordering::Greater
    } else {
        Ordering::Less
    };
    let mut best: Option<(usize, Value)> = None;
    let order: Vec<usize> = if reverse {
        (0..n).rev().collect()
    } else {
        (0..n).collect()
    };
    for i in order {
        let v = get(i);
        if class(&v) != class(lookup) || compare_values(&v, lookup) != wanted {
            continue;
        }
        let better = match &best {
            None => true,
            Some((_, b)) => compare_values(&v, b) == wanted.reverse(),
        };
        if better {
            best = Some((i, v));
        }
    }
    best.map(|(i, _)| i)
}

pub(super) fn choose(ctx: &EvalContext<'_>, args: &[Arg]) -> Arg {
    if args.len() < 2 {
        return Arg::error(ErrorKind::Value);
    }
    let index = match num_arg(ctx, &args[0]) {
        Ok(n) => n.trunc(),
        Err(e) => return Arg::error(e),
    };
    if index < 1.0 || index >= args.len() as f64 {
        return Arg::error(ErrorKind::Value);
    }
    match &args[index as usize] {
        Arg::Missing => Arg::Value(Value::Empty),
        other => other.clone(),
    }
}
