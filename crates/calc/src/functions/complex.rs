//! Complex number functions (`COMPLEX`, `IM*`). Operands are text such as `3+4i`.

use super::{Res, arity, num_arg, optional, text_arg};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value, number_to_text as general_text, parse_number_text};
use crate::xmath;

/// 15 significant digits, positional up to 21 characters (one more than the General conversion), else scientific.
fn number_to_text(n: f64) -> String {
    if n == 0.0 || !n.is_finite() {
        return general_text(n);
    }
    let sci = format!("{:.14e}", n.abs());
    let (mant, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let digits: String = mant.replace('.', "");
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let sign = if n < 0.0 { "-" } else { "" };
    let len = if exp >= 0 {
        (exp as usize + 1).max(digits.len() + 1)
    } else {
        digits.len() + (-exp) as usize + 1
    };
    if len > 21 {
        let frac = if digits.len() > 1 {
            format!(".{}", &digits[1..])
        } else {
            String::new()
        };
        return format!(
            "{sign}{}{frac}E{}{:02}",
            &digits[..1],
            if exp < 0 { '-' } else { '+' },
            exp.abs()
        );
    }
    if exp >= 0 {
        let int_len = exp as usize + 1;
        if digits.len() <= int_len {
            format!("{sign}{digits}{}", "0".repeat(int_len - digits.len()))
        } else {
            format!("{sign}{}.{}", &digits[..int_len], &digits[int_len..])
        }
    } else {
        format!("{sign}0.{}{digits}", "0".repeat((-exp - 1) as usize))
    }
}

fn sn(x: f64) -> f64 {
    xmath::sin_cos_tan(x).0
}
fn cs(x: f64) -> f64 {
    xmath::sin_cos_tan(x).1
}

#[derive(Debug, Clone, Copy)]
struct Cx {
    re: f64,
    im: f64,
    /// `'i'`, `'j'`, or `None` for a plain real number (no suffix seen).
    suffix: Option<char>,
}

/// An imaginary coefficient: any number of signs (with spaces between) in front of a number.
fn coefficient(s: &str) -> Option<f64> {
    if s == "+" || s.is_empty() {
        return Some(1.0);
    }
    if s == "-" {
        return Some(-1.0);
    }
    let mut neg = false;
    let mut rest = s;
    loop {
        rest = rest.trim_start_matches(' ');
        match rest.chars().next() {
            Some('+') => rest = &rest[1..],
            Some('-') => {
                neg = !neg;
                rest = &rest[1..];
            }
            _ => break,
        }
    }
    let n = parse_number_text(rest.trim_end_matches(' '))?;
    Some(if neg { -n } else { n })
}

fn parse(text: &str) -> Res<Cx> {
    let s = text.trim_matches(' ');
    if let Some(n) = parse_number_text(s) {
        return Ok(Cx {
            re: n,
            im: 0.0,
            suffix: None,
        });
    }
    let suffix = match s.chars().last() {
        Some(c @ ('i' | 'j')) => c,
        _ => return Err(ErrorKind::Num),
    };
    let body = &s[..s.len() - 1];
    let bytes = body.as_bytes();
    let mut split = None;
    for (i, &b) in bytes.iter().enumerate().skip(1) {
        if (b == b'+' || b == b'-') && !matches!(bytes[i - 1], b'e' | b'E') {
            split = Some(i);
            break;
        }
    }
    let (re, im) = match split {
        Some(i) => {
            let re = parse_number_text(body[..i].trim_matches(' ')).ok_or(ErrorKind::Num)?;
            (re, coefficient(&body[i..]).ok_or(ErrorKind::Num)?)
        }
        None => (
            0.0,
            coefficient(body.trim_matches(' ')).ok_or(ErrorKind::Num)?,
        ),
    };
    if !re.is_finite() || !im.is_finite() {
        return Err(ErrorKind::Num);
    }
    Ok(Cx {
        re,
        im,
        suffix: Some(suffix),
    })
}

fn operand(ctx: &EvalContext<'_>, arg: &Arg) -> Res<Cx> {
    if matches!(arg, Arg::Missing) {
        return Err(ErrorKind::NA);
    }
    if matches!(arg, Arg::Range(r) if !r.is_single_cell()) {
        return Err(ErrorKind::Value);
    }
    let text = match super::scalar(ctx, arg)? {
        Value::Bool(_) => return Err(ErrorKind::Value),
        Value::Number(n) => number_to_text(n),
        Value::Empty => String::new(),
        v => v.to_text()?,
    };
    parse(&text)
}

fn format(re: f64, im: f64, suffix: Option<char>) -> Res<Value> {
    if !re.is_finite() || !im.is_finite() {
        return Err(ErrorKind::Num);
    }
    let sfx = suffix.unwrap_or('i');
    let mut out = String::new();
    if re != 0.0 || im == 0.0 {
        out.push_str(&number_to_text(re));
    }
    if im != 0.0 {
        let c = if im == 1.0 {
            if out.is_empty() {
                String::new()
            } else {
                "+".to_owned()
            }
        } else if im == -1.0 {
            "-".to_owned()
        } else {
            let t = number_to_text(im);
            if im > 0.0 && !out.is_empty() {
                format!("+{t}")
            } else {
                t
            }
        };
        out.push_str(&c);
        out.push(sfx);
    }
    Ok(Value::Text(out))
}

fn out(z: Cx, re: f64, im: f64) -> Res<Value> {
    format(re, im, z.suffix)
}

pub(super) fn complex(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 3)?;
    let strict = |a: &Arg| -> Res<f64> {
        if matches!(a, Arg::Missing) {
            return Err(ErrorKind::NA);
        }
        if matches!(ctx.scalar(a), Value::Bool(_)) {
            return Err(ErrorKind::Value);
        }
        num_arg(ctx, a)
    };
    let re = strict(&args[0])?;
    let im = strict(&args[1])?;
    let suffix = match optional(args, 2) {
        None => 'i',
        Some(a) => match text_arg(ctx, a)?.as_str() {
            "i" | "" => 'i',
            "j" => 'j',
            _ => return Err(ErrorKind::Value),
        },
    };
    format(re, im, Some(suffix))
}

fn unary(ctx: &EvalContext<'_>, args: &[Arg], f: fn(Cx) -> Res<Value>) -> Res<Value> {
    arity(args, 1, 1)?;
    f(operand(ctx, &args[0])?)
}

pub(super) fn imreal(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |z| Ok(Value::Number(z.re)))
}
pub(super) fn imaginary(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |z| Ok(Value::Number(z.im)))
}
pub(super) fn imargument(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |z| {
        if z.re == 0.0 && z.im == 0.0 {
            return Err(ErrorKind::DivZero);
        }
        Ok(Value::Number(xmath::atan2(z.im, z.re)))
    })
}
pub(super) fn imconjugate(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |z| out(z, z.re, -z.im))
}

fn polar(z: Cx) -> (f64, f64) {
    (z.re.hypot(z.im), xmath::atan2(z.im, z.re))
}

fn cdiv(a: (f64, f64), b: (f64, f64)) -> Res<(f64, f64)> {
    let d = b.0 * b.0 + b.1 * b.1;
    if d == 0.0 {
        return Err(ErrorKind::Num);
    }
    Ok(((a.0 * b.0 + a.1 * b.1) / d, (a.1 * b.0 - a.0 * b.1) / d))
}

pub(super) fn imexp(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |z| {
        let e = xmath::exp(z.re);
        out(z, e * cs(z.im), e * sn(z.im))
    })
}
fn ln_parts(z: Cx) -> Res<(f64, f64)> {
    if z.re == 0.0 && z.im == 0.0 {
        return Err(ErrorKind::Num);
    }
    let (r, t) = polar(z);
    Ok((xmath::ln(r), t))
}
pub(super) fn imln(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |z| {
        let (a, b) = ln_parts(z)?;
        out(z, a, b)
    })
}
pub(super) fn imlog10(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |z| {
        let (a, b) = ln_parts(z)?;
        out(z, a / std::f64::consts::LN_10, b / std::f64::consts::LN_10)
    })
}
pub(super) fn imlog2(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    unary(ctx, args, |z| {
        let (a, b) = ln_parts(z)?;
        out(z, a / std::f64::consts::LN_2, b / std::f64::consts::LN_2)
    })
}

fn sin_c(z: Cx) -> (f64, f64) {
    (sn(z.re) * xmath::cosh(z.im), cs(z.re) * xmath::sinh(z.im))
}
fn cos_c(z: Cx) -> (f64, f64) {
    (
        cs(z.re) * xmath::cosh(z.im),
        -(sn(z.re) * xmath::sinh(z.im)),
    )
}
fn sinh_c(z: Cx) -> (f64, f64) {
    (xmath::sinh(z.re) * cs(z.im), xmath::cosh(z.re) * sn(z.im))
}
fn cosh_c(z: Cx) -> (f64, f64) {
    (xmath::cosh(z.re) * cs(z.im), xmath::sinh(z.re) * sn(z.im))
}

macro_rules! cfn {
    ($name:ident, |$z:ident| $body:expr) => {
        pub(super) fn $name(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
            unary(ctx, args, |$z| {
                let (a, b) = $body?;
                out($z, a, b)
            })
        }
    };
}
cfn!(imsin, |z| Ok::<_, ErrorKind>(sin_c(z)));
cfn!(imcos, |z| Ok::<_, ErrorKind>(cos_c(z)));
cfn!(imsinh, |z| Ok::<_, ErrorKind>(sinh_c(z)));
cfn!(imcosh, |z| Ok::<_, ErrorKind>(cosh_c(z)));

/// Combines the suffix of two operands: both must agree where both have one.
fn join_suffix(a: Cx, b: Cx) -> Res<Option<char>> {
    match (a.suffix, b.suffix) {
        (Some(x), Some(y)) if x != y => Err(ErrorKind::Value),
        (Some(x), _) | (_, Some(x)) => Ok(Some(x)),
        _ => Ok(None),
    }
}

type Pair = (f64, f64);

fn binary(
    ctx: &EvalContext<'_>,
    args: &[Arg],
    f: fn(Pair, Pair) -> Res<Pair>,
    mismatch: ErrorKind,
) -> Res<Value> {
    arity(args, 2, 2)?;
    let a = operand(ctx, &args[0])?;
    let b = operand(ctx, &args[1])?;
    let suffix = join_suffix(a, b).map_err(|_| mismatch)?;
    let (re, im) = f((a.re, a.im), (b.re, b.im))?;
    format(re, im, suffix)
}

pub(super) fn imsub(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    binary(ctx, args, |a, b| Ok((a.0 - b.0, a.1 - b.1)), ErrorKind::Num)
}
pub(super) fn imdiv(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    // Mixed suffixes are #NUM! here (#VALUE! for the other functions).
    binary(ctx, args, cdiv, ErrorKind::Num)
}

fn fold(ctx: &EvalContext<'_>, args: &[Arg], start: Pair, f: fn(Pair, Pair) -> Pair) -> Res<Value> {
    if args.is_empty() {
        return Err(ErrorKind::Value);
    }
    let mut acc = Cx {
        re: start.0,
        im: start.1,
        suffix: None,
    };
    for arg in args {
        let items: Vec<Value> = match arg {
            Arg::Range(_) | Arg::Array(_) | Arg::Union(_) => ctx.cells(arg).collect(),
            Arg::Missing => continue,
            other => vec![ctx.scalar(other)],
        };
        for v in items {
            let z = match v {
                Value::Error(e) => return Err(e),
                Value::Bool(_) => return Err(ErrorKind::Value),
                Value::Number(n) => parse(&number_to_text(n))?,
                Value::Empty => parse("")?,
                other => parse(&other.to_text()?)?,
            };
            let suffix = join_suffix(acc, z)?;
            let (re, im) = f((acc.re, acc.im), (z.re, z.im));
            acc = Cx { re, im, suffix };
        }
    }
    if args.iter().all(|a| matches!(a, Arg::Missing)) {
        return Err(ErrorKind::NA);
    }
    format(acc.re, acc.im, acc.suffix)
}

pub(super) fn imsum(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    fold(ctx, args, (0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1))
}
pub(super) fn improduct(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    fold(ctx, args, (1.0, 0.0), |a, b| {
        (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
    })
}
