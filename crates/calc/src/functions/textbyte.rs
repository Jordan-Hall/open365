//! `REPLACE` and the byte-oriented variants (`LENB`, `MIDB`, `FINDB`, `SEARCHB`, `REPLACEB`).
//!
//! In an Excel without a double-byte language (measured on en-GB) the byte functions count UTF-16 units:
//! `LENB("😀x")` is 3 and `MIDB` can cut a surrogate pair (the lone half shows as U+FFFD here). `LEFTB` and
//! `RIGHTB` behave exactly like `LEFT` and `RIGHT` (they count characters).

use super::criteria::wildcard_search;
use super::textchar::trunc_arg;
use super::{Res, arity, text_arg};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

/// Excel's maximum cell text length.
const MAX_TEXT: usize = 32_767;

fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn from_units(u: &[u16]) -> String {
    String::from_utf16_lossy(u)
}

fn joined(parts: [&str; 3]) -> Res<Value> {
    let s: String = parts.concat();
    if s.encode_utf16().count() > MAX_TEXT {
        Err(ErrorKind::Value)
    } else {
        Ok(Value::Text(s))
    }
}

/// `REPLACE` start and count arguments: 1..=32767 for the start, 0..=32767 for the count.
fn replace_args(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<(usize, usize)> {
    let start = trunc_arg(ctx, &args[1])?;
    let count = trunc_arg(ctx, &args[2])?;
    let max = MAX_TEXT as i64;
    if !(1..=max).contains(&start) || !(0..=max).contains(&count) {
        return Err(ErrorKind::Value);
    }
    Ok((start as usize - 1, count as usize))
}

pub(super) fn replace(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 4, 4)?;
    let old: Vec<char> = text_arg(ctx, &args[0])?.chars().collect();
    let (start, count) = replace_args(ctx, args)?;
    let new = text_arg(ctx, &args[3])?;
    let from = start.min(old.len());
    let to = from.saturating_add(count).min(old.len());
    let head: String = old[..from].iter().collect();
    let tail: String = old[to..].iter().collect();
    joined([&head, &new, &tail])
}

pub(super) fn replaceb(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 4, 4)?;
    let old = units(&text_arg(ctx, &args[0])?);
    let (start, count) = replace_args(ctx, args)?;
    let new = text_arg(ctx, &args[3])?;
    let from = start.min(old.len());
    let to = from.saturating_add(count).min(old.len());
    joined([&from_units(&old[..from]), &new, &from_units(&old[to..])])
}

pub(super) fn lenb(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    Ok(Value::Number(
        text_arg(ctx, &args[0])?.encode_utf16().count() as f64,
    ))
}

pub(super) fn midb(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 3)?;
    let u = units(&text_arg(ctx, &args[0])?);
    let start = trunc_arg(ctx, &args[1])?;
    let n = trunc_arg(ctx, &args[2])?;
    let max = i64::from(i32::MAX);
    if !(1..=max).contains(&start) || !(0..=max).contains(&n) {
        return Err(ErrorKind::Value);
    }
    let from = (start as usize - 1).min(u.len());
    let to = from.saturating_add(n as usize).min(u.len());
    Ok(Value::Text(from_units(&u[from..to])))
}

/// The 0-based start of `FINDB`/`SEARCHB`; `max` is the largest valid 1-based start.
fn start_unit(ctx: &EvalContext<'_>, args: &[Arg], max: usize) -> Res<usize> {
    let start = match args.get(2) {
        None => 1,
        Some(a) => trunc_arg(ctx, a)?,
    };
    if start < 1 || start as usize > max {
        return Err(ErrorKind::Value);
    }
    Ok(start as usize - 1)
}

pub(super) fn findb(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 3)?;
    let needle = units(&text_arg(ctx, &args[0])?);
    let hay = units(&text_arg(ctx, &args[1])?);
    let start = start_unit(ctx, args, hay.len() + 1)?;
    if needle.is_empty() {
        return Ok(Value::Number((start + 1) as f64));
    }
    if needle.len() > hay.len() {
        return Err(ErrorKind::Value);
    }
    (start..=hay.len() - needle.len())
        .find(|&i| hay[i..i + needle.len()] == needle[..])
        .map(|i| Value::Number((i + 1) as f64))
        .ok_or(ErrorKind::Value)
}

pub(super) fn searchb(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 3)?;
    let pattern = text_arg(ctx, &args[0])?;
    let hay = text_arg(ctx, &args[1])?;
    let start = start_unit(ctx, args, hay.encode_utf16().count())?;
    wildcard_search(&pattern, &hay, start)
        .map(|i| Value::Number((i + 1) as f64))
        .ok_or(ErrorKind::Value)
}
