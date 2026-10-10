//! Numeric sections: plain digits, scientific notation and fractions.

use super::Out;
use super::decimal::Dec;
use super::parse::{Section, Tok, fraction_shape};

/// True when the section prints the number with digit placeholders (otherwise the number is not shown).
pub(super) fn is_scientific(sec: &Section) -> bool {
    sec.toks.iter().any(|t| matches!(t, Tok::Exp { .. }))
}

/// Digits of one integer run with grouping; `kinds` are the placeholders left to right.
struct IntRun<'a> {
    digits: &'a str,
    kinds: Vec<char>,
    grouping: bool,
}

impl IntRun<'_> {
    /// Text for placeholder `j` (and, for `j == 0`, the digits left of all placeholders).
    fn piece(&self, j: usize) -> String {
        let n = self.kinds.len();
        let len = self.digits.len();
        let mut s = String::new();
        let put = |p: usize, kind: Option<char>, s: &mut String| {
            let ch = if p < len {
                Some(char::from(self.digits.as_bytes()[len - 1 - p]))
            } else {
                match kind {
                    Some('0') => Some('0'),
                    Some('?') => Some(' '),
                    _ => None,
                }
            };
            if let Some(c) = ch {
                s.push(c);
                if self.grouping && p > 0 && p.is_multiple_of(3) && c != ' ' {
                    s.push(',');
                }
            }
        };
        if j == 0 {
            for p in (n..len).rev() {
                put(p, None, &mut s);
            }
        }
        put(n - 1 - j, Some(self.kinds[j]), &mut s);
        s
    }
}

fn kinds_of(toks: &[Tok]) -> Vec<char> {
    toks.iter()
        .filter_map(|t| match t {
            Tok::Digit(c) => Some(*c),
            _ => None,
        })
        .collect()
}

/// Emits the digits of a number given the integer digits and the already rounded fraction digits.
fn emit_plain(sec: &Section, toks: &[Tok], int: &str, frac: &str, out: &mut Out) {
    let point = toks.iter().position(|t| matches!(t, Tok::Point));
    let int_kinds = kinds_of(&toks[..point.unwrap_or(toks.len())]);
    let frac_kinds = point.map_or_else(Vec::new, |p| kinds_of(&toks[p..]));
    let last_nz = frac.rfind(|c| c != '0');
    let run = IntRun {
        digits: int,
        kinds: int_kinds.clone(),
        grouping: sec.grouping,
    };
    let mut ij = 0;
    let mut fj = 0;
    let mut in_frac = false;
    for (idx, tok) in toks.iter().enumerate() {
        match tok {
            Tok::Digit(_) if !in_frac => {
                out.push(&run.piece(ij));
                ij += 1;
            }
            Tok::Digit(kind) => {
                let digit = frac.as_bytes().get(fj).map(|b| char::from(*b));
                let shown = last_nz.is_some_and(|l| fj <= l);
                match (kind, digit) {
                    ('0', Some(d)) => out.push(&d.to_string()),
                    ('0', None) => out.push("0"),
                    ('#', Some(d)) if shown => out.push(&d.to_string()),
                    ('?', Some(d)) if shown => out.push(&d.to_string()),
                    ('?', _) => out.push(" "),
                    _ => {}
                }
                fj += 1;
            }
            Tok::Point => {
                if int_kinds.is_empty() && !int.is_empty() {
                    let run0 = IntRun {
                        digits: int,
                        kinds: Vec::new(),
                        grouping: sec.grouping,
                    };
                    let mut s = String::new();
                    for p in (0..int.len()).rev() {
                        s.push(char::from(int.as_bytes()[int.len() - 1 - p]));
                        if run0.grouping && p > 0 && p.is_multiple_of(3) {
                            s.push(',');
                        }
                    }
                    out.push(&s);
                }
                in_frac = true;
                let _ = idx;
                out.push(".");
            }
            other => emit_common(other, out),
        }
    }
    if point.is_none() && int_kinds.is_empty() {
        // No placeholders at all: the number is not shown.
    }
    let _ = frac_kinds;
}

/// Tokens that print the same in every kind of numeric section.
fn emit_common(tok: &Tok, out: &mut Out) {
    match tok {
        Tok::Lit(s) => out.push(s),
        Tok::Pad(c) => out.pad(*c),
        Tok::Fill(c) => out.fill(*c),
        Tok::Percent => out.push("%"),
        Tok::Slash => out.push("/"),
        _ => {}
    }
}

fn scale_dec(sec: &Section, x: f64) -> Dec {
    let mut dec = Dec::from_f64(x);
    dec.shift(2 * sec.percents as i32 - 3 * sec.scale as i32);
    dec
}

/// Renders `x` (non-negative) with the digit placeholders of a plain numeric section.
pub(super) fn render_plain(sec: &Section, x: f64, out: &mut Out) -> bool {
    let toks = &sec.toks;
    let point = toks.iter().position(|t| matches!(t, Tok::Point));
    let n_frac = point.map_or(0, |p| kinds_of(&toks[p..]).len());
    let mut dec = scale_dec(sec, x);
    dec.round_to(n_frac);
    emit_plain(sec, toks, &dec.int_digits(), &dec.frac_digits(n_frac), out);
    !dec.is_zero()
}

/// Scientific notation.
pub(super) fn render_sci(sec: &Section, x: f64, out: &mut Out) -> bool {
    let toks = &sec.toks;
    let exp_at = toks
        .iter()
        .position(|t| matches!(t, Tok::Exp { .. }))
        .unwrap_or(toks.len());
    let plus = matches!(toks.get(exp_at), Some(Tok::Exp { plus: true }));
    let mant_toks = &toks[..exp_at];
    let point = mant_toks.iter().position(|t| matches!(t, Tok::Point));
    let n_int = kinds_of(&mant_toks[..point.unwrap_or(mant_toks.len())]).len();
    let n_frac = point.map_or(0, |p| kinds_of(&mant_toks[p..]).len());
    let exp_kinds: Vec<char> = toks
        .iter()
        .skip(exp_at + 1)
        .take_while(|t| matches!(t, Tok::Digit(_)))
        .map(|t| if let Tok::Digit(c) = t { *c } else { '0' })
        .collect();
    let dec = scale_dec(sec, x);
    let ni = n_int.max(1) as i32;
    let (mant, exp10) = if dec.is_zero() {
        (dec.clone(), 0)
    } else {
        let mut e10 = dec.exponent().div_euclid(ni) * ni;
        let mut m = dec.clone();
        m.shift(-e10);
        m.round_to(n_frac);
        if m.int_digits().len() as i32 > ni {
            e10 += ni;
            m = dec.clone();
            m.shift(-e10);
            m.round_to(n_frac);
        }
        (m, e10)
    };
    let zero = dec.is_zero();
    let mut int = mant.int_digits();
    if zero {
        int = "0".repeat(n_int.max(1));
    }
    let frac = mant.frac_digits(n_frac);
    emit_plain(sec, mant_toks, &int, &frac, out);
    out.push("E");
    if exp10 < 0 {
        out.push("-");
    } else if plus {
        out.push("+");
    }
    let digits = exp10.unsigned_abs().to_string();
    let run = IntRun {
        digits: &digits,
        kinds: exp_kinds.clone(),
        grouping: false,
    };
    if exp_kinds.is_empty() {
        out.push(&digits);
    } else {
        for j in 0..exp_kinds.len() {
            out.push(&run.piece(j));
        }
    }
    for tok in toks.iter().skip(exp_at + 1 + exp_kinds.len()) {
        emit_common(tok, out);
    }
    !zero
}

/// Best fraction for `x` with a denominator below `max_den`: the continued fraction is followed while the
/// denominator stays within the limit.
fn best_fraction(x: f64, max_den: u64) -> (u64, u64) {
    let max = max_den as f64;
    let (mut p2, mut p1, mut q2, mut q1) = (0.0f64, 1.0f64, 1.0f64, 0.0f64);
    let mut b = x;
    let (mut p, mut q) = (0.0f64, 0.0f64);
    while q1 < max {
        let a = b.floor();
        p = a * p1 + p2;
        q = a * q1 + q2;
        if b - a < 0.000_000_05 {
            break;
        }
        b = 1.0 / (b - a);
        p2 = p1;
        p1 = p;
        q2 = q1;
        q1 = q;
    }
    if q > max {
        if q1 > max {
            q = q2;
            p = p2;
        } else {
            q = q1;
            p = p1;
        }
    }
    (p as u64, q as u64)
}

/// Fractions such as `# ?/?` or `?/16`.
pub(super) fn render_fraction(sec: &Section, x: f64, out: &mut Out) -> bool {
    let toks = &sec.toks;
    let slash = toks
        .iter()
        .position(|t| matches!(t, Tok::Slash))
        .unwrap_or(0);
    // Digit groups before the slash, split by non-digit tokens: the last group is the numerator.
    let mut groups: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < slash {
        if matches!(toks[i], Tok::Digit(_)) {
            let start = i;
            while i < slash && matches!(toks[i], Tok::Digit(_)) {
                i += 1;
            }
            groups.push((start, i));
        } else {
            i += 1;
        }
    }
    let (num_start, num_end) = groups.last().copied().unwrap_or((slash, slash));
    let int_group = (groups.len() >= 2).then(|| groups[groups.len() - 2]);
    let den_end = {
        let mut j = slash + 1;
        match toks.get(j) {
            Some(Tok::FixedDen(_)) => j += 1,
            _ => {
                while matches!(toks.get(j), Some(Tok::Digit(_))) {
                    j += 1;
                }
            }
        }
        j
    };
    let den_toks = &toks[slash + 1..den_end];
    let fixed_den = match den_toks.first() {
        Some(Tok::FixedDen(n)) => Some(*n),
        _ => None,
    };
    let den_kinds = kinds_of(den_toks);
    let num_kinds = kinds_of(&toks[num_start..num_end]);
    let dec = {
        let mut d = Dec::from_f64(x);
        d.shift(2 * sec.percents as i32 - 3 * sec.scale as i32);
        d
    };
    let xv = dec.to_f64();
    let (mut whole, frac_value) = if int_group.is_some() {
        (xv.floor(), xv - xv.floor())
    } else {
        (0.0, xv)
    };
    let (mut p, q) = match fixed_den {
        Some(q) => {
            let p = (frac_value * q as f64 + 0.5).floor() as u64;
            (p, q)
        }
        None => {
            let max_den = 10u64.pow(den_kinds.len().clamp(1, 9) as u32) - 1;
            best_fraction(frac_value, max_den)
        }
    };
    let mut q = q;
    if int_group.is_some() && p >= q && q > 0 {
        whole += (p / q) as f64;
        p %= q;
    }
    if int_group.is_some() && p == 0 {
        q = fixed_den.unwrap_or(1);
    }
    let blank_fraction = int_group.is_some() && p == 0;
    let first_kind = num_kinds.first().copied().unwrap_or('?');
    let int_str = if int_group.is_some() {
        let mut d = Dec::from_f64(whole);
        d.round_to(0);
        d.int_digits()
    } else {
        String::new()
    };
    let int_kinds = int_group.map_or_else(Vec::new, |(a, b)| kinds_of(&toks[a..b]));
    let int_run = IntRun {
        digits: &int_str,
        kinds: int_kinds.clone(),
        grouping: sec.grouping,
    };
    let first_block = int_group.map_or(num_start, |(a, _)| a);
    for (idx, tok) in toks.iter().enumerate() {
        if idx < first_block {
            emit_common(tok, out);
            continue;
        }
        if let Some((a, b)) = int_group
            && idx >= a
            && idx < b
        {
            let j = idx - a;
            // A whole number of zero shows "0" only when the fraction is blank too.
            if int_str.is_empty() && blank_fraction {
                out.push(&int_run_zero(&int_kinds, j));
            } else {
                out.push(&int_run.piece(j));
            }
            continue;
        }
        if idx >= den_end {
            emit_common(tok, out);
            continue;
        }
        // Between the integer group and the end of the denominator.
        if int_group.is_some() && first_kind == '#' && int_str.is_empty() && idx < num_start {
            // `# #/#` with a zero whole part shows the bare fraction.
            continue;
        }
        if blank_fraction {
            match first_kind {
                '#' => {}
                '?' => match tok {
                    Tok::Digit(_) | Tok::Slash | Tok::Lit(_) | Tok::FixedDen(_) => {
                        let width = match tok {
                            Tok::Lit(s) => s.chars().count(),
                            Tok::FixedDen(n) => n.to_string().len(),
                            _ => 1,
                        };
                        out.push(&" ".repeat(width));
                    }
                    other => emit_common(other, out),
                },
                _ => match tok {
                    Tok::Digit(_) if idx < slash => {
                        out.push(&format!("{:0>w$}", "0", w = 1));
                    }
                    Tok::Digit(_) => {}
                    Tok::FixedDen(_) => out.push(&q.to_string()),
                    other => emit_common(other, out),
                },
            }
            if first_kind != '#' && first_kind != '?' && idx + 1 == den_end && fixed_den.is_none() {
                out.push(&q.to_string());
            }
            continue;
        }
        if idx >= num_start && idx < num_end {
            if idx == num_start {
                out.push(&padded(p, &num_kinds, false));
            }
            continue;
        }
        if idx > slash && fixed_den.is_none() && idx < den_end {
            if idx == slash + 1 {
                out.push(&padded(q, &den_kinds, true));
            }
            continue;
        }
        if idx > slash && fixed_den.is_some() {
            out.push(&q.to_string());
            continue;
        }
        emit_common(tok, out);
    }
    whole > 0.0 || p > 0
}

fn int_run_zero(kinds: &[char], j: usize) -> String {
    if j + 1 == kinds.len() {
        "0".into()
    } else {
        String::new()
    }
}

/// Pads `n` to the placeholder count: numerators right-aligned, denominators left-aligned.
fn padded(n: u64, kinds: &[char], left_align: bool) -> String {
    let s = n.to_string();
    if s.len() >= kinds.len() {
        return s;
    }
    let missing = kinds.len() - s.len();
    let fill_kind = if left_align {
        kinds[kinds.len() - missing..].first()
    } else {
        kinds[..missing].first()
    };
    let fill = match fill_kind {
        Some('0') => "0".repeat(missing),
        Some('?') => " ".repeat(missing),
        _ => String::new(),
    };
    if left_align {
        format!("{s}{fill}")
    } else {
        format!("{fill}{s}")
    }
}

/// True for sections laid out as a fraction.
pub(super) fn is_fraction(sec: &Section) -> bool {
    fraction_shape(&sec.toks)
}
