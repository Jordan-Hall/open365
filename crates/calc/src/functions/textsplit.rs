//! `TEXTBEFORE`, `TEXTAFTER` and `TEXTSPLIT`.
//!
//! Behaviour measured against Excel 365:
//! * Delimiters are matched left to right without overlap (`TEXTBEFORE("aaaa","aa",2)` is `aa`); a negative
//!   instance scans from the end with the same rule. With several delimiters the longest one at a position wins.
//! * Case-insensitive matching lower-cases the text and every delimiter separately with the `LOWER` rules, which
//!   makes it asymmetric for the Greek sigma (`TEXTBEFORE("xΣy","σ",1,1)` finds it, `("xσy","Σ")` does not).
//! * An empty delimiter matches at the start (positive instance) or the end (negative instance).
//! * An instance beyond the text length is `#VALUE!`; one within it that is not found is `#N/A` (or the
//!   `if_not_found` value).

use super::text::map_case;
use super::textchar::trunc_arg;
use super::{Res, arity, bool_arg, optional, text_arg};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

const MAX_TEXT: usize = 32_767;

fn chars_of(s: &str, ignore_case: bool) -> Vec<char> {
    if ignore_case {
        map_case(s, false).chars().collect()
    } else {
        s.chars().collect()
    }
}

/// The delimiter strings of an argument (a single value, an array or a range).
fn delimiter_strings(ctx: &EvalContext<'_>, arg: &Arg) -> Res<Vec<String>> {
    ctx.cells(arg).map(|v| v.to_text()).collect()
}

/// A 0/1 option argument (numbers, booleans and numeric text); an omitted argument is `default`.
fn flag(ctx: &EvalContext<'_>, args: &[Arg], index: usize, default: bool) -> Res<bool> {
    match optional(args, index) {
        None => Ok(default),
        Some(a) => match trunc_arg(ctx, a)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ErrorKind::Value),
        },
    }
}

/// The first non-overlapping match of any delimiter at or after `from` within `hay[..to]`: `(start, length)`.
fn find_forward(
    hay: &[char],
    to: usize,
    delims: &[Vec<char>],
    from: usize,
) -> Option<(usize, usize)> {
    (from..=to).find_map(|i| {
        delims
            .iter()
            .filter(|d| !d.is_empty() && i + d.len() <= to && hay[i..i + d.len()] == d[..])
            .map(Vec::len)
            .max()
            .map(|l| (i, l))
    })
}

/// The last match of any delimiter ending at or before `upto`: `(start, length)`.
fn find_backward(hay: &[char], upto: usize, delims: &[Vec<char>]) -> Option<(usize, usize)> {
    (0..=upto).rev().find_map(|end| {
        delims
            .iter()
            .filter(|d| !d.is_empty() && d.len() <= end && hay[end - d.len()..end] == d[..])
            .map(Vec::len)
            .max()
            .map(|l| (end - l, l))
    })
}

fn before_after(ctx: &EvalContext<'_>, args: &[Arg], after: bool) -> Res<Value> {
    arity(args, 2, 6)?;
    let text = text_arg(ctx, &args[0])?;
    let instance = match optional(args, 2) {
        None => 1,
        Some(a) => trunc_arg(ctx, a)?,
    };
    let ignore_case = flag(ctx, args, 3, false)?;
    let match_end = flag(ctx, args, 4, false)?;
    let raw_delims = delimiter_strings(ctx, &args[1])?;
    let not_found = match optional(args, 5) {
        None => Value::Error(ErrorKind::NA),
        Some(a) => ctx.scalar(a),
    };
    if instance == 0 {
        return Err(ErrorKind::Value);
    }
    let orig: Vec<char> = text.chars().collect();
    let hay = chars_of(&text, ignore_case);
    let delims: Vec<Vec<char>> = raw_delims
        .iter()
        .map(|d| chars_of(d, ignore_case))
        .collect();
    let n = hay.len();
    let found: Option<(usize, usize)> = if delims.iter().any(Vec::is_empty) {
        Some(if instance > 0 { (0, 0) } else { (n, 0) })
    } else {
        let wanted = instance.unsigned_abs() as usize;
        if wanted > n {
            // An empty text with instance 1 is a plain "not found".
            if !(n == 0 && instance == 1 && !match_end) {
                return Err(ErrorKind::Value);
            }
            None
        } else if instance > 0 {
            let (mut pos, mut count) = (0, 0);
            let mut hit = None;
            while let Some((i, l)) = find_forward(&hay, n, &delims, pos) {
                count += 1;
                if count == wanted {
                    hit = Some((i, l));
                    break;
                }
                pos = i + l;
            }
            hit.or_else(|| (match_end && count + 1 == wanted).then_some((n, 0)))
        } else {
            let (mut end, mut count) = (n, 0);
            let mut hit = None;
            while let Some((i, l)) = find_backward(&hay, end, &delims) {
                count += 1;
                if count == wanted {
                    hit = Some((i, l));
                    break;
                }
                end = i;
            }
            hit.or_else(|| (match_end && count + 1 == wanted).then_some((0, 0)))
        }
    };
    match found {
        Some((i, l)) => {
            let part = if after { &orig[i + l..] } else { &orig[..i] };
            Ok(Value::Text(part.iter().collect()))
        }
        None => match not_found {
            Value::Error(e) => Err(e),
            v => Ok(Value::Text(v.to_text()?)),
        },
    }
}

pub(super) fn textbefore(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    before_after(ctx, args, false)
}

pub(super) fn textafter(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    before_after(ctx, args, true)
}

/// Splits `hay[from..to]` at every delimiter match; the pieces are `(start, end)` ranges.
fn split_range(hay: &[char], from: usize, to: usize, delims: &[Vec<char>]) -> Vec<(usize, usize)> {
    let mut pieces = Vec::new();
    let mut pos = from;
    while let Some((i, l)) = find_forward(hay, to, delims, pos) {
        pieces.push((pos, i));
        pos = i + l;
    }
    pieces.push((pos, to));
    pieces
}

/// A delimiter argument for `TEXTSPLIT`: none when omitted, an error when any delimiter is empty.
fn split_delims(
    ctx: &EvalContext<'_>,
    args: &[Arg],
    index: usize,
    ignore_case: bool,
) -> Res<Option<Vec<Vec<char>>>> {
    let Some(arg) = optional(args, index) else {
        return Ok(None);
    };
    let strings = delimiter_strings(ctx, arg)?;
    if strings.is_empty() || strings.iter().any(String::is_empty) {
        return Err(ErrorKind::Value);
    }
    Ok(Some(
        strings.iter().map(|d| chars_of(d, ignore_case)).collect(),
    ))
}

pub(super) fn textsplit(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 6)?;
    let text = text_arg(ctx, &args[0])?;
    let ignore_empty = match optional(args, 3) {
        None => false,
        Some(a) => bool_arg(ctx, a)?,
    };
    let ignore_case = flag(ctx, args, 4, false)?;
    let pad = match optional(args, 5) {
        None => Value::Error(ErrorKind::NA),
        Some(a) => ctx.scalar(a),
    };
    let col_delims = split_delims(ctx, args, 1, ignore_case)?;
    let row_delims = split_delims(ctx, args, 2, ignore_case)?;
    if (col_delims.is_none() && row_delims.is_none()) || text.is_empty() {
        return Err(ErrorKind::Value);
    }
    let orig: Vec<char> = text.chars().collect();
    let hay = chars_of(&text, ignore_case);
    let n = hay.len();
    let rows = match &row_delims {
        Some(d) => split_range(&hay, 0, n, d),
        None => vec![(0, n)],
    };
    let mut table: Vec<Vec<Value>> = Vec::new();
    for (from, to) in rows {
        let pieces = match &col_delims {
            Some(d) => split_range(&hay, from, to, d),
            None => vec![(from, to)],
        };
        let row: Vec<Value> = pieces
            .into_iter()
            .filter(|&(a, b)| !(ignore_empty && a == b))
            .map(|(a, b)| Value::Text(orig[a..b].iter().collect()))
            .collect();
        if !(ignore_empty && row.is_empty()) {
            table.push(row);
        }
    }
    if table.is_empty() {
        return Err(ErrorKind::Value);
    }
    let width = table.iter().map(Vec::len).max().unwrap_or(0);
    let pad = match pad {
        Value::Array(_) => Value::Error(ErrorKind::Value),
        other => other,
    };
    for row in &mut table {
        row.resize(width, pad.clone());
    }
    if table.len().saturating_mul(width) > MAX_TEXT * 32 {
        return Err(ErrorKind::Value);
    }
    Ok(Value::Array(table))
}
