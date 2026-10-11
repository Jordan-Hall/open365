//! Functions that turn values into text: `TEXT`, `FIXED`, `DOLLAR`, `VALUETOTEXT`, `ARRAYTOTEXT`, and the
//! locale-aware text to number conversion `NUMBERVALUE`.
//!
//! `DOLLAR` uses the en-US currency format (`$` and parentheses for negatives); the oracle machine is en-GB, so
//! the corpus normalises the currency symbol and leaves negative values out (see docs/testing/calc-conformance.md).

use super::textchar::trunc_arg;
use super::{Res, arity, bool_arg, optional, scalar, text_arg};
use crate::eval::{Arg, EvalContext};
use crate::numfmt;
use crate::value::{ErrorKind, Value, number_to_text, parse_number_text_in};

/// Longest text `TEXT` returns.
const MAX_TEXT_RESULT: usize = 255;
/// Excel's maximum cell text length.
const MAX_TEXT: usize = 32_767;

pub(super) fn text(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let value = match scalar(ctx, &args[0])? {
        Value::Text(t) => match parse_number_text_in(&t, ctx.date1904) {
            Some(n) => Value::Number(n),
            None => Value::Text(t),
        },
        Value::Empty => Value::Number(0.0),
        other => other,
    };
    let code = text_arg(ctx, &args[1])?;
    match numfmt::text_function_in(&value, &code, ctx.date1904, ctx.locale) {
        Ok(Some(s)) if s.encode_utf16().count() <= MAX_TEXT_RESULT => Ok(Value::Text(s)),
        _ => Err(ErrorKind::Value),
    }
}

/// The `decimals` argument of `FIXED` and `DOLLAR`: at most 127 places; negative values round left of the point.
fn decimals_arg(ctx: &EvalContext<'_>, args: &[Arg], default: i64) -> Res<i32> {
    let d = match optional(args, 1) {
        None => default,
        Some(a) => trunc_arg(ctx, a)?,
    };
    if d > 127 {
        return Err(ErrorKind::Value);
    }
    Ok(d.max(-400) as i32)
}

pub(super) fn fixed(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 3)?;
    let x = super::num_arg(ctx, &args[0])?;
    let decimals = decimals_arg(ctx, args, 2)?;
    let no_commas = match optional(args, 2) {
        None => false,
        Some(a) => bool_arg(ctx, a)?,
    };
    let (digits, negative) = numfmt::fixed_digits(x, decimals, !no_commas);
    Ok(Value::Text(if negative {
        format!("-{digits}")
    } else {
        digits
    }))
}

pub(super) fn dollar(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 2)?;
    let x = super::num_arg(ctx, &args[0])?;
    // A trailing comma (`DOLLAR(x,)`) means zero decimals; a missing argument means two.
    let default = if args.len() > 1 { 0 } else { 2 };
    let decimals = decimals_arg(ctx, args, default)?;
    let (digits, _) = numfmt::fixed_digits(x, decimals, true);
    let symbol = &ctx.locale.currency;
    Ok(Value::Text(if x < 0.0 && ctx.locale.currency_parens {
        format!("({symbol}{digits})")
    } else if x < 0.0 {
        format!("-{symbol}{digits}")
    } else {
        format!("{symbol}{digits}")
    }))
}

/// A separator argument of `NUMBERVALUE`: omitted gives the default, an empty text is an error.
fn separator(ctx: &EvalContext<'_>, arg: Option<&Arg>, default: &str) -> Res<String> {
    match arg {
        None => Ok(default.to_owned()),
        Some(a) => {
            let s = text_arg(ctx, a)?;
            if s.is_empty() {
                Err(ErrorKind::Value)
            } else {
                Ok(s)
            }
        }
    }
}

pub(super) fn numbervalue(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 3)?;
    let text = text_arg(ctx, &args[0])?;
    let decimal = separator(ctx, optional(args, 1), ".")?
        .chars()
        .next()
        .ok_or(ErrorKind::Value)?;
    // The default group separator moves to "." when the decimal separator is ",".
    let default_group = if decimal == ',' { "." } else { "," };
    let groups: Vec<char> = separator(ctx, optional(args, 2), default_group)?
        .chars()
        .collect();
    if groups.contains(&decimal) {
        return Err(ErrorKind::Value);
    }
    // Blanks (space, tab, line breaks) are ignored anywhere; group separators only left of the decimal one.
    let mut cleaned = String::with_capacity(text.len());
    let mut seen_decimal = false;
    for c in text.chars() {
        if matches!(c, ' ' | '\t' | '\n' | '\r') {
            continue;
        }
        if c == decimal {
            if seen_decimal {
                return Err(ErrorKind::Value);
            }
            seen_decimal = true;
            cleaned.push('.');
        } else if groups.contains(&c) {
            if seen_decimal {
                return Err(ErrorKind::Value);
            }
        } else {
            cleaned.push(c);
        }
    }
    // Percent signs on either end each divide by 100.
    let trimmed = cleaned.trim_start_matches('%');
    let lead = cleaned.len() - trimmed.len();
    let core = trimmed.trim_end_matches('%');
    let percents = lead + (trimmed.len() - core.len());
    let mut n = if core.is_empty() {
        0.0
    } else if core.contains('$') {
        return Err(ErrorKind::Value);
    } else {
        parse_number_text_in(core, ctx.date1904).ok_or(ErrorKind::Value)?
    };
    for _ in 0..percents {
        n /= 100.0;
    }
    super::number(n)
}

/// The text of one value for `VALUETOTEXT` and `ARRAYTOTEXT`; `strict` quotes text.
fn value_text(v: &Value, strict: bool) -> String {
    match v {
        Value::Empty => String::new(),
        Value::Number(n) => number_to_text(*n),
        Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_owned(),
        Value::Error(e) => e.as_str().to_owned(),
        Value::Text(t) if strict => format!("\"{}\"", t.replace('"', "\"\"")),
        Value::Text(t) => t.clone(),
        Value::Array(_) => ErrorKind::Value.as_str().to_owned(),
    }
}

/// The format argument of `VALUETOTEXT` and `ARRAYTOTEXT`: 0 concise, 1 strict.
fn format_arg(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<bool> {
    match optional(args, 1) {
        None => Ok(false),
        Some(a) => match trunc_arg(ctx, a)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ErrorKind::Value),
        },
    }
}

fn limited(s: String) -> Res<Value> {
    // Excel answers `#CALC!`, not `#VALUE!`, for a result longer than a cell can hold.
    if s.encode_utf16().count() > MAX_TEXT {
        Err(ErrorKind::Calc)
    } else {
        Ok(Value::Text(s))
    }
}

pub(super) fn valuetotext(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 2)?;
    let strict = format_arg(ctx, args)?;
    // Errors are shown as text, including the `#VALUE!` of a range that cannot be intersected.
    let v = ctx.scalar(&args[0]);
    Ok(Value::Text(value_text(&v, strict)))
}

pub(super) fn arraytotext(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 2)?;
    let strict = format_arg(ctx, args)?;
    let rows: Vec<Vec<Value>> = match &args[0] {
        Arg::Missing | Arg::Union(_) | Arg::Refs(_) | Arg::Lambda(_) => {
            return Err(ErrorKind::Value);
        }
        Arg::Value(v) => vec![vec![v.clone()]],
        Arg::Array(rows) => rows.clone(),
        Arg::Range(_) => {
            let grid = ctx.grid(&args[0]).ok_or(ErrorKind::Value)?;
            (0..grid.scan_rows())
                .map(|r| (0..grid.scan_cols()).map(|c| grid.get(r, c)).collect())
                .collect()
        }
    };
    let single = rows.len() == 1 && rows[0].len() == 1;
    if single {
        match &rows[0][0] {
            // `ARRAYTOTEXT(#N/A)` is the error, `ARRAYTOTEXT({#N/A},1)` the text `{#N/A}`.
            Value::Error(e) if !matches!(args[0], Arg::Array(_)) => return Err(*e),
            Value::Empty if matches!(args[0], Arg::Range(_)) => return Err(ErrorKind::Value),
            _ => {}
        }
    }
    if strict {
        let body: Vec<String> = rows
            .iter()
            .map(|r| {
                r.iter()
                    .map(|v| value_text(v, true))
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .collect();
        limited(format!("{{{}}}", body.join(";")))
    } else {
        let items: Vec<String> = rows
            .iter()
            .flat_map(|r| r.iter().map(|v| value_text(v, false)))
            .collect();
        limited(items.join(", "))
    }
}
