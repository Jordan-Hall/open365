//! Text functions. Positions and lengths count Unicode scalar values: Excel 365 gives `LEN("😀")` = 1 (measured).
//! Only the wildcard characters of `SEARCH`/`COUNTIF` see UTF-16 units: `"?"` does not match an emoji.

use super::criteria::wildcard_search;
use super::{Res, arity, bool_arg, int_arg, scalar, text_arg};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value, parse_value_text_in};

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

fn units(s: &str) -> Vec<char> {
    s.chars().collect()
}

fn from_units(u: &[char]) -> String {
    u.iter().collect()
}

pub(super) fn len(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    count(text_arg(ctx, &args[0])?.chars().count())
}

/// A non-negative count argument. An omitted argument counts as 0, an absent one as `default`.
fn count_arg(ctx: &EvalContext<'_>, args: &[Arg], index: usize, default: usize) -> Res<usize> {
    match args.get(index) {
        None => Ok(default),
        Some(a) => {
            let n = int_arg(ctx, a)?;
            usize::try_from(n).map_err(|_| ErrorKind::Value)
        }
    }
}

/// Excel rejects counts beyond the 32,767-character limit.
fn limit(n: usize) -> Res<usize> {
    if n > i32::MAX as usize {
        Err(ErrorKind::Value)
    } else {
        Ok(n)
    }
}

pub(super) fn left(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 2)?;
    let u = units(&text_arg(ctx, &args[0])?);
    let n = count_arg(ctx, args, 1, 1)?.min(u.len());
    Ok(Value::Text(from_units(&u[..n])))
}

pub(super) fn right(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 2)?;
    let u = units(&text_arg(ctx, &args[0])?);
    let n = count_arg(ctx, args, 1, 1)?.min(u.len());
    Ok(Value::Text(from_units(&u[u.len() - n..])))
}

pub(super) fn mid(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 3)?;
    let u = units(&text_arg(ctx, &args[0])?);
    let start = int_arg(ctx, &args[1])?;
    let n = limit(count_arg(ctx, args, 2, 0)?)?;
    if !(1..=i64::from(i32::MAX)).contains(&start) {
        return Err(ErrorKind::Value);
    }
    let from = (start as usize - 1).min(u.len());
    let to = from.saturating_add(n).min(u.len());
    Ok(Value::Text(from_units(&u[from..to])))
}

/// Excel maps characters one to one and only inside the Basic Multilingual Plane; a few characters whose
/// simple case mapping crosses scripts (`µ`, `ſ`, `ẞ`) keep their form.
pub(super) fn map_case(s: &str, upper: bool) -> String {
    let chars: Vec<char> = s.chars().collect();
    (0..chars.len())
        .map(|i| map_char(&chars, i, upper))
        .collect()
}

/// The case mapping of `chars[i]`; the neighbours matter for the Greek final sigma.
pub(super) fn map_char(chars: &[char], i: usize, upper: bool) -> char {
    let c = chars[i];
    if c.is_ascii() {
        return if upper {
            c.to_ascii_uppercase()
        } else {
            c.to_ascii_lowercase()
        };
    }
    // Greek capital sigma becomes the final form unless a letter or digit follows (a lone sigma is final too).
    if !upper && c == 'Σ' {
        return if chars.get(i + 1).is_some_and(|n| n.is_alphanumeric()) {
            'σ'
        } else {
            'ς'
        };
    }
    if c as u32 > 0xFFFF || matches!(c, 'µ' | 'ſ' | 'ẞ' | 'ß') {
        return c;
    }
    // Excel keeps the Kelvin, Ohm and Angstrom signs when lower-casing.
    if !upper && matches!(c, '\u{212A}' | '\u{2126}' | '\u{212B}') {
        return c;
    }
    if !upper && c == 'İ' {
        return 'i';
    }
    // Excel maps these two to the capital with a diaeresis only (the full mapping adds a tonos).
    if upper && matches!(c, 'ΐ' | 'ΰ') {
        return if c == 'ΐ' { 'Ϊ' } else { 'Ϋ' };
    }
    let mut mapped = if upper {
        c.to_uppercase().collect::<Vec<_>>()
    } else {
        c.to_lowercase().collect::<Vec<_>>()
    };
    if mapped.len() == 1 {
        mapped.remove(0)
    } else {
        c
    }
}

pub(super) fn upper(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    Ok(Value::Text(map_case(&text_arg(ctx, &args[0])?, true)))
}

pub(super) fn lower(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    Ok(Value::Text(map_case(&text_arg(ctx, &args[0])?, false)))
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
    // Excel cuts the result of CONCATENATE at the cell limit instead of failing.
    if out.chars().count() > MAX_TEXT {
        out = out.chars().take(MAX_TEXT).collect();
    }
    Ok(Value::Text(out))
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
    // An omitted ignore_empty behaves like TRUE.
    let ignore_empty = match &args[1] {
        Arg::Missing => true,
        a => bool_arg(ctx, a)?,
    };
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

/// Shared start-position handling for `FIND` and `SEARCH`; `max` is the largest valid 1-based start.
fn start_index(ctx: &EvalContext<'_>, args: &[Arg], max: usize) -> Res<usize> {
    let start = match args.get(2) {
        None => 1,
        Some(a) => int_arg(ctx, a)?,
    };
    if start < 1 || start as usize > max {
        return Err(ErrorKind::Value);
    }
    Ok(start as usize - 1)
}

pub(super) fn find(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 3)?;
    let needle = units(&text_arg(ctx, &args[0])?);
    let hay = units(&text_arg(ctx, &args[1])?);
    let start = start_index(ctx, args, hay.len() + 1)?;
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
    // Wildcards work on UTF-16 units; positions are reported in characters.
    let start16: usize = hay.chars().take(start).map(char::len_utf16).sum();
    let found16 = wildcard_search(&pattern, &hay, start16).ok_or(ErrorKind::Value)?;
    let mut offset = 0;
    for (i, c) in hay.chars().enumerate() {
        if offset >= found16 {
            return count(i + 1);
        }
        offset += c.len_utf16();
    }
    count(hay.chars().count() + 1)
}

pub(super) fn substitute(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 4)?;
    let s = text_arg(ctx, &args[0])?;
    let old = text_arg(ctx, &args[1])?;
    let new = text_arg(ctx, &args[2])?;
    let instance = match args.get(3) {
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
    let Some(n) = instance else {
        return text(s.replace(&old, &new));
    };
    // Occurrences are counted with overlap: the 2nd "aa" in "aaaa" starts at the 2nd character.
    let first_len = old.chars().next().map_or(1, char::len_utf8);
    let mut from = 0;
    let mut seen = 0;
    while let Some(pos) = s[from..].find(&old) {
        seen += 1;
        let at = from + pos;
        if seen == n {
            return text(format!("{}{}{}", &s[..at], new, &s[at + old.len()..]));
        }
        from = at + first_len;
    }
    Ok(Value::Text(s))
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
        Value::Text(t) => parse_value_text_in(&t, ctx.date1904)
            .map(Value::Number)
            .ok_or(ErrorKind::Value),
        _ => Err(ErrorKind::Value),
    }
}
