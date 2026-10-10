//! Character-level text functions: `CHAR`, `CODE`, `UNICHAR`, `UNICODE`, `EXACT`, `PROPER`, `CLEAN`.
//!
//! `CHAR` and `CODE` use the Windows-1252 code page, which is the system code page of an English-language
//! Excel (measured against Excel 365, en-GB). Characters without a Windows-1252 form give `?` (63) in `CODE`.

use super::letters::is_letter;
use super::text::map_char;
use super::{Res, arity, text_arg};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

/// Unicode value of the Windows-1252 bytes 0x80..=0x9F (bytes without a character map to themselves).
const CP1252_HIGH: [u32; 32] = [
    0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039,
    0x0152, 0x008D, 0x017D, 0x008F, 0x0090, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014,
    0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x009D, 0x017E, 0x0178,
];

/// A number argument truncated toward zero; values beyond the `i64` range saturate (callers range-check).
pub(super) fn trunc_arg(ctx: &EvalContext<'_>, arg: &Arg) -> Res<i64> {
    let n = super::num_arg(ctx, arg)?.trunc();
    Ok(n as i64)
}

pub(super) fn char_fn(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let n = trunc_arg(ctx, &args[0])?;
    if !(1..=255).contains(&n) {
        return Err(ErrorKind::Value);
    }
    let byte = n as u32;
    let u = if (0x80..=0x9F).contains(&byte) {
        CP1252_HIGH[(byte - 0x80) as usize]
    } else {
        byte
    };
    Ok(Value::Text(
        char::from_u32(u).map(String::from).unwrap_or_default(),
    ))
}

pub(super) fn code(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let s = text_arg(ctx, &args[0])?;
    let c = s.chars().next().ok_or(ErrorKind::Value)?;
    let u = u32::from(c);
    let byte = if u < 0x80 || (0xA0..=0xFF).contains(&u) || (0x80..=0x9F).contains(&u) {
        u
    } else if let Some(i) = CP1252_HIGH.iter().position(|&x| x == u) {
        0x80 + i as u32
    } else {
        u32::from(b'?')
    };
    Ok(Value::Number(f64::from(byte)))
}

pub(super) fn unichar(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let n = trunc_arg(ctx, &args[0])?;
    if !(1..=0x10FFFF).contains(&n) {
        return Err(ErrorKind::Value);
    }
    let u = n as u32;
    // Surrogates and the noncharacters (U+FDD0..U+FDEF and the last two code points of every plane) are #N/A.
    if (0xD800..=0xDFFF).contains(&u) || (0xFDD0..=0xFDEF).contains(&u) || (u & 0xFFFE) == 0xFFFE {
        return Err(ErrorKind::NA);
    }
    let c = char::from_u32(u).ok_or(ErrorKind::NA)?;
    Ok(Value::Text(c.to_string()))
}

pub(super) fn unicode(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let s = text_arg(ctx, &args[0])?;
    let c = s.chars().next().ok_or(ErrorKind::Value)?;
    Ok(Value::Number(f64::from(u32::from(c))))
}

pub(super) fn exact(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let a = text_arg(ctx, &args[0])?;
    let b = text_arg(ctx, &args[1])?;
    Ok(Value::Bool(a == b))
}

/// Capitalises the first letter of every run of letters and lower-cases the rest. Only BMP letters count as
/// letters, so a digit, mark, symbol or astral character starts a new word.
pub(super) fn proper(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let s = text_arg(ctx, &args[0])?;
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut in_word = false;
    for (i, &c) in chars.iter().enumerate() {
        // A word's first letter is lower-cased first: PROPER("İ") is "I" and PROPER("ǅ") is "Ǆ".
        let lowered = [map_char(&chars, i, false)];
        out.push(if in_word {
            lowered[0]
        } else {
            map_char(&lowered, 0, true)
        });
        in_word = is_letter(c);
    }
    Ok(Value::Text(out))
}

/// Removes the control characters U+0000..U+001F and the C1 controls U+0080..U+009F.
pub(super) fn clean(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let s = text_arg(ctx, &args[0])?;
    Ok(Value::Text(
        s.chars()
            .filter(|&c| !(c < ' ' || ('\u{80}'..='\u{9F}').contains(&c)))
            .collect(),
    ))
}

/// `T`: the value if it is text, otherwise empty text. A range or array gives its top-left cell.
pub(super) fn t(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    if let Arg::Refs(refs) = &args[0] {
        return Ok(Value::Array(
            refs.iter()
                .map(|row| {
                    row.iter()
                        .map(|e| match e {
                            Ok(r) => match ctx.source.cell(&r.sheet, r.row1, r.col1) {
                                v @ (Value::Text(_) | Value::Error(_)) => v,
                                _ => Value::Text(String::new()),
                            },
                            Err(e) => Value::Error(*e),
                        })
                        .collect()
                })
                .collect(),
        ));
    }
    match first_cell(ctx, &args[0])? {
        v @ Value::Text(_) => Ok(v),
        _ => Ok(Value::Text(String::new())),
    }
}

/// `N`: numbers as they are, `TRUE` as 1, everything else 0.
pub(super) fn n(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    // An array of references: the top-left cell of each element; an error element stays an error.
    if let Arg::Refs(refs) = &args[0] {
        let as_number = |v: Value| match v {
            Value::Number(x) => Value::Number(x),
            Value::Bool(true) => Value::Number(1.0),
            Value::Error(e) => Value::Error(e),
            _ => Value::Number(0.0),
        };
        return Ok(Value::Array(
            refs.iter()
                .map(|row| {
                    row.iter()
                        .map(|e| match e {
                            Ok(r) => as_number(ctx.source.cell(&r.sheet, r.row1, r.col1)),
                            Err(e) => Value::Error(*e),
                        })
                        .collect()
                })
                .collect(),
        ));
    }
    Ok(Value::Number(match first_cell(ctx, &args[0])? {
        Value::Number(x) => x,
        Value::Bool(true) => 1.0,
        _ => 0.0,
    }))
}

/// The top-left cell of a reference or array, errors propagating.
pub(super) fn first_cell(ctx: &EvalContext<'_>, arg: &Arg) -> Res<Value> {
    let v = match arg {
        Arg::Range(r) => ctx.source.cell(&r.sheet, r.row1, r.col1),
        Arg::Union(rs) => match rs.first() {
            Some(r) => ctx.source.cell(&r.sheet, r.row1, r.col1),
            None => Value::Error(ErrorKind::Value),
        },
        other => ctx.scalar(other),
    };
    match v {
        Value::Error(e) => Err(e),
        v => Ok(v),
    }
}
