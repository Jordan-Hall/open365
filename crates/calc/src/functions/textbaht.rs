//! `BAHTTEXT`: a number as Thai currency words.
//!
//! Measured against Excel (the function does not depend on the locale):
//! * The amount is rounded half up to two decimals from its 15 significant digits (`2.675` is 2.68 baht,
//!   `1.005` is 1 baht 1 satang, `0.995` is 1 baht). Text is converted to a number first.
//! * A negative amount starts with `ลบ`, also when it rounds to zero. Zero is `ศูนย์บาทถ้วน`; a whole amount ends
//!   with `ถ้วน`; an amount below one baht has no baht part.
//! * Groups of six digits are joined with `ล้าน` (so 10^12 is `หนึ่งล้านล้านบาทถ้วน`). A final 1 is `เอ็ด` when a
//!   digit sits before it in the same number or its group (11, 101, 1001, 1,000,001), otherwise `หนึ่ง`; 2 in
//!   the tens place is `ยี่`, 1 in the tens place is just `สิบ`.

use super::{Res, arity, num_arg};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

const DIGITS: [&str; 10] = [
    "ศูนย์",
    "หนึ่ง",
    "สอง",
    "สาม",
    "สี่",
    "ห้า",
    "หก",
    "เจ็ด",
    "แปด",
    "เก้า",
];
const PLACES: [&str; 6] = ["", "สิบ", "ร้อย", "พัน", "หมื่น", "แสน"];

/// Words for a group of at most six digits (most significant first); `higher` is true when more digits come
/// before the group, which turns a final 1 into `เอ็ด`.
fn group_words(digits: &[u8], higher: bool) -> String {
    let n = digits.len();
    let mut out = String::new();
    for (i, &d) in digits.iter().enumerate() {
        let pos = n - 1 - i;
        if d == 0 {
            continue;
        }
        match pos {
            0 => {
                if d == 1 && (higher || digits[..i].iter().any(|&x| x != 0)) {
                    out.push_str("เอ็ด");
                } else {
                    out.push_str(DIGITS[usize::from(d)]);
                }
            }
            1 => match d {
                1 => out.push_str("สิบ"),
                2 => out.push_str("ยี่สิบ"),
                _ => {
                    out.push_str(DIGITS[usize::from(d)]);
                    out.push_str("สิบ");
                }
            },
            _ => {
                out.push_str(DIGITS[usize::from(d)]);
                out.push_str(PLACES[pos]);
            }
        }
    }
    out
}

/// Words for a non-negative integer given as decimal digits without leading zeros (or a single 0).
fn integer_words(digits: &[u8], higher: bool) -> String {
    if digits.len() <= 6 {
        return group_words(digits, higher);
    }
    let split = digits.len() - 6;
    let mut out = integer_words(&digits[..split], higher);
    out.push_str("ล้าน");
    if digits[split..].iter().any(|&d| d != 0) {
        out.push_str(&group_words(&digits[split..], true));
    }
    out
}

/// The integer digits and the satang of `x` (non-negative, finite): `x` is cut to 15 significant digits, then
/// rounded half up to two decimals.
fn split_amount(x: f64) -> (Vec<u8>, u8) {
    // d.dddddddddddddde+X: 15 significant digits.
    let sci = format!("{x:.14e}");
    let (mant, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let mut sig: Vec<u8> = mant
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    let mut int: Vec<u8> = Vec::new();
    let mut frac: Vec<u8> = Vec::new();
    if exp >= 0 {
        let k = exp as usize + 1;
        while sig.len() < k {
            sig.push(0);
        }
        int.extend_from_slice(&sig[..k]);
        frac.extend_from_slice(&sig[k..]);
    } else {
        int.push(0);
        frac.extend(std::iter::repeat_n(0, (-exp - 1) as usize));
        frac.extend_from_slice(&sig);
    }
    while frac.len() < 3 {
        frac.push(0);
    }
    let mut satang = frac[0] * 10 + frac[1];
    if frac[2] >= 5 {
        satang += 1;
    }
    if satang >= 100 {
        satang -= 100;
        let mut i = int.len();
        loop {
            if i == 0 {
                int.insert(0, 1);
                break;
            }
            i -= 1;
            if int[i] == 9 {
                int[i] = 0;
            } else {
                int[i] += 1;
                break;
            }
        }
    }
    (int, satang)
}

pub(super) fn bahttext(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let x = num_arg(ctx, &args[0])?;
    if !x.is_finite() {
        return Err(ErrorKind::Value);
    }
    let (mut int, satang) = split_amount(x.abs());
    while int.len() > 1 && int[0] == 0 {
        int.remove(0);
    }
    let mut out = String::new();
    if x < 0.0 {
        out.push_str("ลบ");
    }
    let whole = int != [0];
    if whole || satang == 0 {
        if whole {
            out.push_str(&integer_words(&int, false));
        } else {
            out.push_str(DIGITS[0]);
        }
        out.push_str("บาท");
    }
    if satang == 0 {
        out.push_str("ถ้วน");
    } else {
        out.push_str(&group_words(&[satang / 10, satang % 10], false));
        out.push_str("สตางค์");
    }
    Ok(Value::Text(out))
}
