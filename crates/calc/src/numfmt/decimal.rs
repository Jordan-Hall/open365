//! Decimal digit strings of a double, as Excel's number formatting sees them (15 significant digits).

/// A non-negative decimal number: `0.DIGITS x 10^point`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Dec {
    /// Significant digits without leading or trailing zeros; empty for zero.
    pub digits: Vec<u8>,
    pub point: i32,
}

impl Dec {
    /// `abs(x)` rounded to 15 significant digits.
    pub fn from_f64(x: f64) -> Dec {
        let x = x.abs();
        if x == 0.0 || !x.is_finite() {
            return Dec {
                digits: Vec::new(),
                point: 0,
            };
        }
        let sci = format!("{x:.14e}");
        let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
        let exp: i32 = exp.parse().unwrap_or(0);
        let mut digits: Vec<u8> = mantissa
            .bytes()
            .filter(u8::is_ascii_digit)
            .map(|b| b - b'0')
            .collect();
        while digits.last() == Some(&0) {
            digits.pop();
        }
        Dec {
            digits,
            point: exp + 1,
        }
    }

    pub fn is_zero(&self) -> bool {
        self.digits.is_empty()
    }

    /// Multiplies by `10^n` (exact for a decimal).
    pub fn shift(&mut self, n: i32) {
        if !self.is_zero() {
            self.point += n;
        }
    }

    /// Rounds half away from zero to `decimals` places after the point.
    pub fn round_to(&mut self, decimals: usize) {
        self.round_at(decimals as i64);
    }

    /// Rounds half away from zero at `decimals` places after the point (negative: before it).
    pub fn round_at(&mut self, decimals: i64) {
        if self.is_zero() {
            return;
        }
        let keep = i64::from(self.point) + decimals;
        if keep < 0 {
            self.digits.clear();
            self.point = 0;
            return;
        }
        let keep = keep as usize;
        if keep >= self.digits.len() {
            return;
        }
        let up = self.digits[keep] >= 5;
        self.digits.truncate(keep);
        if up {
            let mut i = keep;
            loop {
                if i == 0 {
                    self.digits.insert(0, 1);
                    self.point += 1;
                    break;
                }
                i -= 1;
                if self.digits[i] == 9 {
                    self.digits[i] = 0;
                } else {
                    self.digits[i] += 1;
                    break;
                }
            }
        }
        while self.digits.last() == Some(&0) {
            self.digits.pop();
        }
        if self.digits.is_empty() {
            self.point = 0;
        }
    }

    /// Digits left of the point, without leading zeros (empty when the integer part is zero).
    pub fn int_digits(&self) -> String {
        if self.point <= 0 {
            return String::new();
        }
        (0..self.point as usize)
            .map(|i| char::from(b'0' + self.digits.get(i).copied().unwrap_or(0)))
            .collect()
    }

    /// The first `n` digits right of the point (zero padded).
    pub fn frac_digits(&self, n: usize) -> String {
        (0..n)
            .map(|j| {
                let idx = i64::from(self.point) + j as i64;
                let d = if idx < 0 {
                    0
                } else {
                    self.digits.get(idx as usize).copied().unwrap_or(0)
                };
                char::from(b'0' + d)
            })
            .collect()
    }

    /// Decimal exponent of the leading digit (`1234` is 3).
    pub fn exponent(&self) -> i32 {
        self.point - 1
    }

    pub fn to_f64(&self) -> f64 {
        if self.is_zero() {
            return 0.0;
        }
        let digits: String = self.digits.iter().map(|d| char::from(b'0' + d)).collect();
        format!("0.{digits}e{}", self.point).parse().unwrap_or(0.0)
    }
}

/// General format as shown in a cell of the reference column: at most 11 characters, 15 significant digits.
pub(super) fn general_text(x: f64) -> String {
    if x == 0.0 {
        return "0".to_owned();
    }
    let sign = if x < 0.0 { "-" } else { "" };
    let dec = Dec::from_f64(x);
    let e = dec.exponent();
    if let Some(s) = general_fixed(&dec, e) {
        return format!("{sign}{s}");
    }
    format!("{sign}{}", general_sci(&dec))
}

fn general_fixed(dec: &Dec, e: i32) -> Option<String> {
    if e >= 0 {
        let n = e as usize + 1;
        if n > 11 {
            return None;
        }
        let avail = 10usize.saturating_sub(n);
        let mut r = dec.clone();
        r.round_to(avail);
        let int = r.int_digits();
        if int.len() > 11 {
            return None;
        }
        let frac = r.frac_digits(avail);
        let frac = frac.trim_end_matches('0');
        let text = if frac.is_empty() {
            int
        } else {
            format!("{int}.{frac}")
        };
        (text.len() <= 11).then_some(text)
    } else {
        let z = (-e - 1) as usize;
        if z > 8 {
            return None;
        }
        let k = 9 - z;
        let mut r = dec.clone();
        if e < -4 {
            // Below 0.0001 the value is shown positionally only when it needs no rounding.
            if dec.digits.len() > k {
                return None;
            }
        }
        r.round_to(z + k);
        if r.is_zero() {
            return None;
        }
        let int = r.int_digits();
        let frac = r.frac_digits(z + k);
        let frac = frac.trim_end_matches('0');
        let int = if int.is_empty() { "0".to_owned() } else { int };
        Some(if frac.is_empty() {
            int
        } else {
            format!("{int}.{frac}")
        })
    }
}

fn general_sci(dec: &Dec) -> String {
    let mut e = dec.exponent();
    let mut r = dec.clone();
    // Mantissa decimals so that the whole text is 11 characters: d.ddddE+xx.
    let mut ndec = 11usize.saturating_sub(2 + 2 + exp_digits(e));
    r.round_to_sig(ndec + 1);
    if r.exponent() != e {
        e = r.exponent();
        ndec = 11usize.saturating_sub(2 + 2 + exp_digits(e));
        r.round_to_sig(ndec + 1);
    }
    let mut mant = String::new();
    mant.push(char::from(b'0' + r.digits.first().copied().unwrap_or(0)));
    let rest: String = r
        .digits
        .iter()
        .skip(1)
        .map(|d| char::from(b'0' + d))
        .collect();
    if !rest.is_empty() {
        mant.push('.');
        mant.push_str(&rest);
    }
    let sign = if e < 0 { '-' } else { '+' };
    format!("{mant}E{sign}{:02}", e.abs())
}

fn exp_digits(e: i32) -> usize {
    if e.abs() >= 100 { 3 } else { 2 }
}

impl Dec {
    /// Rounds half up to `n` significant digits.
    pub fn round_to_sig(&mut self, n: usize) {
        if self.is_zero() || n >= self.digits.len() {
            return;
        }
        self.round_at(n as i64 - i64::from(self.point));
    }
}
