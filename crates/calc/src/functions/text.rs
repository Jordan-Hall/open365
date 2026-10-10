//! Text functions. Positions and lengths count Unicode scalar values (Excel counts UTF-16
//! code units, which differs only for characters outside the Basic Multilingual Plane).

use super::criteria::wildcard_search;
use super::{Res, arity, bool_arg, int_arg, optional, scalar, text_arg};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value, parse_number_text};

/// Excel's maximum cell text length.
const MAX_TEXT: usize = 32_767;

fn text(s: String) -> Res<Value> {
    if s.chars().count() > MAX_TEXT {
        Err(ErrorKind::Value)
    } else {
        Ok(Value::Text(s))
    }
}

fn count(n: usize) -> Res<Value> {
    Ok(Value::Number(n as f64))
}

pub(super) fn len(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    count(text_arg(ctx, &args[0])?.chars().count())
}

/// Optional non-negative character count, defaulting to `default`.
fn count_arg(ctx: &EvalContext<'_>, args: &[Arg], index: usize, default: usize) -> Res<usize> {
    match optional(args, index) {
        None => Ok(default),
        Some(a) => {
            let n = int_arg(ctx, a)?;
            usize::try_from(n).map_err(|_| ErrorKind::Value)
        }
    }
}

pub(super) fn left(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 2)?;
    let s = text_arg(ctx, &args[0])?;
    let n = count_arg(ctx, args, 1, 1)?;
    Ok(Value::Text(s.chars().take(n).collect()))
}

pub(super) fn right(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 2)?;
    let s = text_arg(ctx, &args[0])?;
    let n = count_arg(ctx, args, 1, 1)?;
    let total = s.chars().count();
    Ok(Value::Text(
        s.chars().skip(total.saturating_sub(n)).collect(),
    ))
}

pub(super) fn mid(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 3)?;
    let s = text_arg(ctx, &args[0])?;
    let start = int_arg(ctx, &args[1])?;
    let n = count_arg(ctx, args, 2, 0)?;
    if start < 1 {
        return Err(ErrorKind::Value);
    }
    Ok(Value::Text(
        s.chars().skip(start as usize - 1).take(n).collect(),
    ))
}

pub(super) fn upper(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    Ok(Value::Text(text_arg(ctx, &args[0])?.to_uppercase()))
}

pub(super) fn lower(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    Ok(Value::Text(text_arg(ctx, &args[0])?.to_lowercase()))
}

/// Removes leading and trailing spaces and collapses inner runs of spaces to one.
pub(super) fn trim(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let s = text_arg(ctx, &args[0])?;
    Ok(Value::Text(
        s.split(' ')
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    ))
}

pub(super) fn concatenate(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    if args.is_empty() {
        return Err(ErrorKind::Value);
    }
    let mut out = String::new();
    for a in args {
        out.push_str(&text_arg(ctx, a)?);
    }
    text(out)
}

pub(super) fn concat(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    if args.is_empty() {
        return Err(ErrorKind::Value);
    }
    let mut out = String::new();
    for a in args {
        for v in ctx.cells(a) {
            out.push_str(&v.to_text()?);
        }
    }
    text(out)
}

pub(super) fn textjoin(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    if args.len() < 3 {
        return Err(ErrorKind::Value);
    }
    let delimiter = text_arg(ctx, &args[0])?;
    let ignore_empty = bool_arg(ctx, &args[1])?;
    let mut parts = Vec::new();
    for a in &args[2..] {
        for v in ctx.cells(a) {
            let t = v.to_text()?;
            if !(ignore_empty && t.is_empty()) {
                parts.push(t);
            }
        }
    }
    text(parts.join(&delimiter))
}

/// Shared start-position handling for `FIND` and `SEARCH`.
fn start_index(ctx: &EvalContext<'_>, args: &[Arg], haystack_len: usize) -> Res<usize> {
    let start = match optional(args, 2) {
        None => 1,
        Some(a) => int_arg(ctx, a)?,
    };
    if start < 1 || start as usize > haystack_len.max(1) {
        return Err(ErrorKind::Value);
    }
    Ok(start as usize - 1)
}

pub(super) fn find(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 3)?;
    let needle: Vec<char> = text_arg(ctx, &args[0])?.chars().collect();
    let hay: Vec<char> = text_arg(ctx, &args[1])?.chars().collect();
    let start = start_index(ctx, args, hay.len())?;
    if needle.is_empty() {
        return count(start + 1);
    }
    if needle.len() > hay.len() {
        return Err(ErrorKind::Value);
    }
    (start..=hay.len() - needle.len())
        .find(|&i| hay[i..i + needle.len()] == needle[..])
        .map_or(Err(ErrorKind::Value), |i| count(i + 1))
}

pub(super) fn search(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 3)?;
    let pattern = text_arg(ctx, &args[0])?;
    let hay = text_arg(ctx, &args[1])?;
    let start = start_index(ctx, args, hay.chars().count())?;
    wildcard_search(&pattern, &hay, start).map_or(Err(ErrorKind::Value), |i| count(i + 1))
}

pub(super) fn substitute(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 4)?;
    let s = text_arg(ctx, &args[0])?;
    let old = text_arg(ctx, &args[1])?;
    let new = text_arg(ctx, &args[2])?;
    let instance = match optional(args, 3) {
        None => None,
        Some(a) => {
            let n = int_arg(ctx, a)?;
            if n < 1 {
                return Err(ErrorKind::Value);
            }
            Some(n as usize)
        }
    };
    if old.is_empty() {
        return Ok(Value::Text(s));
    }
    let mut out = String::new();
    let mut rest = s.as_str();
    let mut seen = 0usize;
    while let Some(pos) = rest.find(&old) {
        seen += 1;
        out.push_str(&rest[..pos]);
        if instance.is_none_or(|n| n == seen) {
            out.push_str(&new);
        } else {
            out.push_str(&old);
        }
        rest = &rest[pos + old.len()..];
    }
    out.push_str(rest);
    text(out)
}

pub(super) fn rept(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let s = text_arg(ctx, &args[0])?;
    let n = int_arg(ctx, &args[1])?;
    if n < 0 {
        return Err(ErrorKind::Value);
    }
    if (s.chars().count() as u64).saturating_mul(n as u64) > MAX_TEXT as u64 {
        return Err(ErrorKind::Value);
    }
    Ok(Value::Text(s.repeat(n as usize)))
}

pub(super) fn value(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    match scalar(ctx, &args[0])? {
        Value::Number(n) => Ok(Value::Number(n)),
        Value::Empty => Ok(Value::Number(0.0)),
        Value::Text(t) => parse_number_text(&t)
            .map(Value::Number)
            .ok_or(ErrorKind::Value),
        _ => Err(ErrorKind::Value),
    }
}
