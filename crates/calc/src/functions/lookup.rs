//! Lookup and reference functions: `VLOOKUP`, `HLOOKUP`, `INDEX`, `MATCH`, `LOOKUP`, `XLOOKUP`, `CHOOSE`.
//!
//! Exact matches compare numbers bit for bit (unlike `=`, which equates values equal to 15 digits).
//! An ascending approximate match (`MATCH` type 1, `VLOOKUP` without FALSE) is the binary search Excel
//! runs, so unsorted data gives Excel's answer, not the "right" one. `MATCH` type -1 scans forward: it
//! returns an exact hit at once, otherwise the last element before the first smaller one. Only elements of
//! the same type as the lookup value are considered (numbers never match text).

use std::cmp::Ordering;

use super::criteria::{has_wildcards, wildcard_match};
use super::{Res, arity, bool_arg, int_arg, num_arg, optional, scalar};
use crate::eval::{Arg, EvalContext, Grid, RangeRef};
use crate::value::{ErrorKind, Value, compare_text, compare_values};

fn class(v: &Value) -> u8 {
    match v {
        Value::Number(_) => 0,
        Value::Text(_) => 1,
        Value::Bool(_) => 2,
        Value::Error(_) => 3,
        Value::Empty | Value::Array(_) => 4,
    }
}

/// The value to look for; `MATCH` and `VLOOKUP` refuse text longer than 255 characters.
fn lookup_value(ctx: &EvalContext<'_>, arg: &Arg) -> Res<Value> {
    Ok(match scalar(ctx, arg)? {
        Value::Empty => Value::Number(0.0),
        Value::Text(t) if t.chars().count() > 255 => return Err(ErrorKind::Value),
        v => v,
    })
}

fn exact_eq(lookup: &Value, cell: &Value, wildcards: bool) -> bool {
    match (lookup, cell) {
        (Value::Number(a), Value::Number(b)) => a == b,
        (Value::Text(a), Value::Text(b)) => {
            if wildcards && has_wildcards(a) {
                wildcard_match(a, b)
            } else {
                compare_text(a, b) == Ordering::Equal
            }
        }
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Empty, Value::Empty) => true,
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

/// Excel's ascending approximate search: a binary search that keeps going right on equality, so duplicates
/// give the last one. Elements of another type than the lookup value are skipped.
fn find_last_le(lookup: &Value, n: usize, get: &dyn Fn(usize) -> Value) -> Option<usize> {
    let (mut lo, mut hi) = (0isize, n as isize - 1); // inclusive bounds
    let (mut equal, mut below) = (None, None);
    while lo <= hi {
        let probe = (lo + hi) / 2;
        // Step left past elements of another type.
        let mut mid = probe;
        while mid >= lo && class(&get(mid as usize)) != class(lookup) {
            mid -= 1;
        }
        if mid < lo {
            lo = probe + 1;
            continue;
        }
        match compare_exact(&get(mid as usize), lookup) {
            Ordering::Greater => hi = mid - 1,
            Ordering::Equal => {
                equal = Some(mid as usize);
                lo = mid + 1;
            }
            Ordering::Less => {
                below = Some(mid as usize);
                lo = mid + 1;
            }
        }
    }
    equal.or(below)
}

/// Ordering for lookups: numbers exactly, everything else as the comparison operators do.
fn compare_exact(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        _ => compare_values(a, b),
    }
}

/// `MATCH` type -1: an exact hit wins; otherwise the last element not smaller than `lookup`, scanning
/// forward until the first smaller one.
fn find_last_ge(lookup: &Value, n: usize, get: &dyn Fn(usize) -> Value) -> Option<usize> {
    let mut best = None;
    for i in 0..n {
        let v = get(i);
        if class(&v) != class(lookup) {
            continue;
        }
        match compare_exact(&v, lookup) {
            Ordering::Equal => return Some(i),
            Ordering::Less => break,
            Ordering::Greater => best = Some(i),
        }
    }
    best
}

fn table_grid<'a>(ctx: &'a EvalContext<'a>, arg: &'a Arg) -> Res<Grid<'a>> {
    match arg {
        Arg::Range(_) | Arg::Array(_) | Arg::Value(_) => ctx.grid(arg).ok_or(ErrorKind::Value),
        _ => Err(ErrorKind::Value),
    }
}

fn table_lookup(ctx: &EvalContext<'_>, args: &[Arg], vertical: bool) -> Res<Value> {
    arity(args, 3, 4)?;
    let lookup = lookup_value(ctx, &args[0])?;
    if matches!(args[1], Arg::Value(_)) {
        return Err(ErrorKind::NA);
    }
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
    Ok(cell)
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
    if matches!(args[1], Arg::Value(_)) {
        return Err(ErrorKind::NA);
    }
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

/// `LOOKUP(value, vector, [result_vector])`: an ascending approximate search. With a result vector the match
/// position indexes it (a single cell counts as a row, and the vector is not clipped to its own size). Without
/// one, `vector` is an array: a wide one is searched along its first row and answers from its last row, any
/// other along its first column and answers from its last column.
pub(super) fn lookup(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 3)?;
    let wanted = lookup_value(ctx, &args[0])?;
    let grid = table_grid(ctx, &args[1])?;
    let wide = grid.cols() > grid.rows();
    let n = if wide {
        grid.scan_cols()
    } else {
        grid.scan_rows()
    };
    let get = |i: usize| if wide { grid.get(0, i) } else { grid.get(i, 0) };
    let pos = find_last_le(&wanted, n, &get).ok_or(ErrorKind::NA)?;
    let Some(result) = args.get(2) else {
        return Ok(if wide {
            grid.get(grid.rows() - 1, pos)
        } else {
            grid.get(pos, grid.cols() - 1)
        });
    };
    let result = table_grid(ctx, result)?;
    let (rows, cols) = (result.rows(), result.cols());
    if rows > 1 && cols > 1 {
        return Err(ErrorKind::NA);
    }
    let vertical = cols == 1 && rows > 1;
    if let Arg::Range(r) = result_range(args) {
        let (row, col) = if vertical {
            (r.row1 + pos as u32, r.col1)
        } else {
            (r.row1, r.col1 + pos as u32)
        };
        if row > crate::ast::MAX_ROW || col > crate::ast::MAX_COL {
            return Err(ErrorKind::NA);
        }
        return Ok(ctx.source.cell(&r.sheet, row, col));
    }
    if vertical {
        if pos < rows {
            Ok(result.get(pos, 0))
        } else {
            Err(ErrorKind::NA)
        }
    } else if pos < cols {
        Ok(result.get(0, pos))
    } else {
        Err(ErrorKind::NA)
    }
}

fn result_range(args: &[Arg]) -> &Arg {
    &args[2]
}

pub(super) fn index(ctx: &EvalContext<'_>, args: &[Arg]) -> Arg {
    match index_inner(ctx, args) {
        Ok(arg) => arg,
        Err(e) => Arg::error(e),
    }
}

fn index_inner(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Arg> {
    arity(args, 2, 4)?;
    if args.len() == 4 && int_arg(ctx, &args[3])? != 1 {
        return Err(ErrorKind::Ref);
    }
    if let Arg::Value(Value::Error(e)) = &args[0] {
        return Err(*e);
    }
    let grid = table_grid(ctx, &args[0])?;
    let (rows, cols) = (grid.rows(), grid.cols());
    let first = int_arg(ctx, &args[1])?;
    // An empty second index counts as 0 (the whole row).
    let second = args.get(2).map(|a| int_arg(ctx, a)).transpose()?;
    let (r, c) = match second {
        Some(c) => (first, c),
        None if rows == 1 => (1, first),
        None if cols == 1 => (first, 1),
        // An array constant or computed array with one index gives the whole row.
        None if !matches!(args[0], Arg::Range(_)) => (first, 0),
        None => return Err(ErrorKind::Ref),
    };
    if r < 0 || c < 0 {
        return Err(ErrorKind::Value);
    }
    let (r, c) = (r as usize, c as usize);
    if r > rows || c > cols {
        return Err(ErrorKind::Ref);
    }
    if let Arg::Range(range) = &args[0] {
        // A reference in, a reference out: blank cells stay blank and `A1:INDEX(...)` works.
        let (row1, row2) = if r == 0 {
            (range.row1, range.row2)
        } else {
            let row = range.row1 + r as u32 - 1;
            (row, row)
        };
        let (col1, col2) = if c == 0 {
            (range.col1, range.col2)
        } else {
            let col = range.col1 + c as u32 - 1;
            (col, col)
        };
        return Ok(Arg::Range(RangeRef::new(
            &range.sheet,
            row1,
            col1,
            row2,
            col2,
        )));
    }
    Ok(Arg::from_value(match (r, c) {
        (r, c) if r > 0 && c > 0 => grid.get(r - 1, c - 1),
        (0, c) if c > 0 => Value::Array((0..rows).map(|i| vec![grid.get(i, c - 1)]).collect()),
        (r, 0) if r > 0 => Value::Array(vec![(0..cols).map(|j| grid.get(r - 1, j)).collect()]),
        _ => Value::Array(
            (0..rows)
                .map(|i| (0..cols).map(|j| grid.get(i, j)).collect())
                .collect(),
        ),
    }))
}

/// The `match_mode` and `search_mode` arguments of `XLOOKUP` and `XMATCH` (arguments 4 and 5 of
/// `XLOOKUP`, 3 and 4 of `XMATCH`), checked.
fn lookup_modes(ctx: &EvalContext<'_>, args: &[Arg], first: usize) -> Res<(i64, i64)> {
    // The mode arguments are rounded down (`-1.5` is `-2`, which is invalid), not truncated.
    let mode = |a: &Arg| -> Res<i64> { Ok(num_arg(ctx, a)?.floor() as i64) };
    let match_mode = match optional(args, first) {
        None => 0,
        Some(a) => mode(a)?,
    };
    let search_mode = match optional(args, first + 1) {
        None => 1,
        Some(a) => mode(a)?,
    };
    if !(-1..=2).contains(&match_mode) || !matches!(search_mode, -2 | -1 | 1 | 2) {
        return Err(ErrorKind::Value);
    }
    // Wildcard matching cannot be combined with a binary search.
    if match_mode == 2 && search_mode.abs() == 2 {
        return Err(ErrorKind::Value);
    }
    Ok((match_mode, search_mode))
}

/// The position (0-based) of `lookup` in the vector `keys`, if it is there.
fn locate(
    lookup: &Value,
    keys: &Grid<'_>,
    vertical: bool,
    n: usize,
    match_mode: i64,
    search_mode: i64,
) -> Option<usize> {
    let get = |i: usize| {
        if vertical {
            keys.get(i, 0)
        } else {
            keys.get(0, i)
        }
    };
    let reverse = search_mode < 0;
    let mut found = if search_mode.abs() == 2 {
        binary_search(lookup, n, &get, reverse, match_mode)
    } else {
        let mut f = find_exact(lookup, n, &get, reverse, match_mode == 2);
        if f.is_none() && (match_mode == 1 || match_mode == -1) {
            f = nearest(lookup, n, &get, reverse, match_mode == 1);
        }
        f
    };
    let full_len = if vertical { keys.rows() } else { keys.cols() };
    if found.is_none() && *lookup == Value::Empty && n < full_len {
        // The blank cells past the used area of a whole-column reference.
        found = Some(n);
    }
    found
}

pub(super) fn xlookup(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 6)?;
    // A blank lookup value matches blank cells (not zero).
    let lookup = scalar(ctx, &args[0])?;
    let keys = table_grid(ctx, &args[1])?;
    let results = table_grid(ctx, &args[2])?;
    let (match_mode, search_mode) = lookup_modes(ctx, args, 4)?;
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
    let Some(i) = locate(&lookup, &keys, vertical, n, match_mode, search_mode) else {
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

/// `XMATCH(lookup, array, [match_mode], [search_mode])`: the position of the match, counted from 1.
pub(super) fn xmatch(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 4)?;
    let lookup = scalar(ctx, &args[0])?;
    let keys = table_grid(ctx, &args[1])?;
    let (match_mode, search_mode) = lookup_modes(ctx, args, 2)?;
    let (vertical, n) = vector_shape(&keys).map_err(|_| ErrorKind::Value)?;
    match locate(&lookup, &keys, vertical, n, match_mode, search_mode) {
        Some(i) => Ok(Value::Number((i + 1) as f64)),
        None => Err(ErrorKind::NA),
    }
}

/// `XLOOKUP` with `search_mode` 2 (ascending data) or -2 (descending data): a plain binary search that
/// trusts the order, so unsorted data gives Excel's answer. The nearest-match candidate is the last probe on
/// the smaller (`match_mode` -1) or larger (1) side.
fn binary_search(
    lookup: &Value,
    n: usize,
    get: &dyn Fn(usize) -> Value,
    descending: bool,
    match_mode: i64,
) -> Option<usize> {
    let (mut lo, mut hi) = (0isize, n as isize - 1);
    let mut candidate = None;
    let mut equal = None;
    while lo <= hi {
        let mid = (lo + hi) / 2;
        let v = get(mid as usize);
        let ord = if class(&v) == class(lookup) {
            compare_exact(&v, lookup)
        } else {
            Ordering::Less
        };
        let ord = if descending { ord.reverse() } else { ord };
        match ord {
            // Among equal keys the search keeps going to the first one (ascending) or last one (descending).
            Ordering::Equal => {
                equal = Some(mid as usize);
                if descending {
                    lo = mid + 1;
                } else {
                    hi = mid - 1;
                }
            }
            Ordering::Less => {
                if match_mode == -1 && !descending || match_mode == 1 && descending {
                    candidate = Some(mid as usize);
                }
                lo = mid + 1;
            }
            Ordering::Greater => {
                if match_mode == 1 && !descending || match_mode == -1 && descending {
                    candidate = Some(mid as usize);
                }
                hi = mid - 1;
            }
        }
    }
    equal.or(if match_mode == 0 { None } else { candidate })
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
        if class(&v) != class(lookup) || compare_exact(&v, lookup) != wanted {
            continue;
        }
        let better = match &best {
            None => true,
            Some((_, b)) => compare_exact(&v, b) == wanted.reverse(),
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
    // An array of indexes picks one value per element.
    if let Arg::Array(rows) = &args[0] {
        let pick = |v: &Value| -> Value {
            match v.to_number_in(ctx.date1904).map(f64::trunc) {
                Ok(n) if n >= 1.0 && n < args.len() as f64 => ctx.scalar(&args[n as usize]),
                Ok(_) => Value::Error(ErrorKind::Value),
                Err(e) => Value::Error(e),
            }
        };
        return Arg::Array(rows.iter().map(|r| r.iter().map(pick).collect()).collect());
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
