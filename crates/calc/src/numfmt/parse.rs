//! Parsing of Excel number format codes (en-US syntax) into sections of tokens.

use super::FormatError;

/// A comparison from a section condition such as `[<100]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Cond {
    pub op: Cmp,
    pub limit: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

impl Cond {
    pub fn matches(self, x: f64) -> bool {
        match self.op {
            Cmp::Lt => x < self.limit,
            Cmp::Le => x <= self.limit,
            Cmp::Gt => x > self.limit,
            Cmp::Ge => x >= self.limit,
            Cmp::Eq => x == self.limit,
            Cmp::Ne => x != self.limit,
        }
    }

    /// True when every number the condition accepts is negative.
    pub fn only_negative(self) -> bool {
        match self.op {
            Cmp::Lt => self.limit <= 0.0,
            Cmp::Le | Cmp::Eq => self.limit < 0.0,
            _ => false,
        }
    }
}

/// One element of a format section.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Tok {
    /// Text copied to the output.
    Lit(String),
    /// `_x`: a space as wide as `x`.
    Pad(char),
    /// `*x`: repeat `x` to fill the cell.
    Fill(char),
    /// `0`, `#` or `?`.
    Digit(char),
    Point,
    /// A comma before it is classified as a group separator, a scaling comma or a literal.
    Comma,
    Percent,
    /// `E+` or `E-`.
    Exp {
        plus: bool,
    },
    Slash,
    /// A constant denominator (`# ?/16`).
    FixedDen(u64),
    /// `@`.
    At,
    General,
    Year(usize),
    Month(usize),
    Day(usize),
    Hour(usize),
    Minute(usize),
    Second(usize),
    /// `AM/PM` (`short` false) or `A/P`; `upper` follows the case of the first letter.
    AmPm {
        short: bool,
        upper: bool,
    },
    /// Buddhist-era year (`b`, `bb`, `bbbb`).
    BuddhistYear(usize),
    /// Era name (`g` the Latin initial, `gg` the first character, `ggg` the full name): only the Japanese
    /// locale (`[$-411]`) has eras; elsewhere it is empty.
    Era(usize),
    /// Era year (`e`, `ee`): the Japanese era year under `[$-411]`, the four-digit year elsewhere.
    EraYear(usize),
    /// `[h]`, `[m]` or `[s]` with the number of letters.
    Elapsed(char, usize),
    /// Fractional seconds: `.0`, `.00`, `.000`.
    SubSec(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Number,
    Date,
    /// Contains `@` and nothing numeric.
    Text,
    General,
}

#[derive(Debug, Clone)]
pub(super) struct Section {
    pub toks: Vec<Tok>,
    pub cond: Option<Cond>,
    pub kind: Kind,
    /// Digit groups separated by thousands separators.
    pub grouping: bool,
    /// Trailing commas: each divides by 1000.
    pub scale: u32,
    pub percents: u32,
    pub has_ampm: bool,
    /// The language of a `[$-LCID]` prefix (calendar and number-shape bits removed): names in a date section.
    pub lcid: Option<u32>,
    /// `[DBNum1]` to `[DBNum3]`: digits are written with CJK numerals (only under the CJK language prefixes).
    pub dbnum: Option<u8>,
    /// The number-shape byte of a `[$-xxxxxxxx]` tag (bits 24 to 31): the digit set of numbers and dates.
    pub shape: u8,
    /// The calendar byte of the tag (bits 16 to 23), or the `B1` (Gregorian, 1) and `B2` (Hijri, 6) prefix.
    pub calendar: u8,
    /// The section uses the Thai date letters without a `[$-]` tag: Thai digits, names and Buddhist years.
    pub thai: bool,
}

/// A parsed format code.
#[derive(Debug, Clone)]
pub struct NumberFormat {
    pub(super) sections: Vec<Section>,
}

/// Splits at `;` outside quotes, escapes and brackets.
pub(super) fn split_sections(code: &str) -> Result<Vec<&str>, FormatError> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut chars = code.char_indices().peekable();
    let mut in_quote = false;
    let mut in_bracket = false;
    while let Some((i, c)) = chars.next() {
        match c {
            '"' if !in_bracket => in_quote = !in_quote,
            '\\' | '_' | '*' if !in_quote && !in_bracket && chars.next().is_none() => {
                return Err(FormatError::new("dangling escape"));
            }
            '[' if !in_quote => in_bracket = true,
            ']' if !in_quote => in_bracket = false,
            ';' if !in_quote && !in_bracket => {
                parts.push(&code[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if in_quote || in_bracket {
        return Err(FormatError::new("unterminated quote or bracket"));
    }
    parts.push(&code[start..]);
    Ok(parts)
}

fn parse_condition(body: &str) -> Option<Cond> {
    let (op, rest) = if let Some(r) = body.strip_prefix("<=") {
        (Cmp::Le, r)
    } else if let Some(r) = body.strip_prefix(">=") {
        (Cmp::Ge, r)
    } else if let Some(r) = body.strip_prefix("<>") {
        (Cmp::Ne, r)
    } else if let Some(r) = body.strip_prefix('<') {
        (Cmp::Lt, r)
    } else if let Some(r) = body.strip_prefix('>') {
        (Cmp::Gt, r)
    } else if let Some(r) = body.strip_prefix('=') {
        (Cmp::Eq, r)
    } else {
        return None;
    };
    let limit: f64 = rest.trim().parse().ok()?;
    limit.is_finite().then_some(Cond { op, limit })
}

const COLOR_NAMES: [&str; 8] = [
    "black", "blue", "cyan", "green", "magenta", "red", "white", "yellow",
];

pub(super) fn is_color(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    if COLOR_NAMES.contains(&lower.as_str()) {
        return true;
    }
    lower
        .strip_prefix("color")
        .map(str::trim_start)
        .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|n| n.parse::<u32>().ok())
        .is_some_and(|n| (1..=56).contains(&n))
        && lower.starts_with("color ")
}

/// Text of a `[$symbol-locale]` bracket. `None` for locale-only or system date/time codes handled elsewhere.
fn currency_text(body: &str) -> String {
    let inner = &body[1..];
    match inner.rfind('-') {
        Some(i) => inner[..i].to_owned(),
        None => inner.to_owned(),
    }
}

struct Tokenizer<'a> {
    chars: Vec<char>,
    pos: usize,
    out: Vec<Tok>,
    cond: Option<Cond>,
    lcid: Option<u32>,
    dbnum: Option<u8>,
    shape: u8,
    calendar: u8,
    thai: bool,
    _src: &'a str,
}

impl<'a> Tokenizer<'a> {
    fn new(src: &'a str) -> Self {
        Tokenizer {
            chars: src.chars().collect(),
            pos: 0,
            out: Vec::new(),
            cond: None,
            lcid: None,
            dbnum: None,
            shape: 0,
            calendar: 0,
            thai: false,
            _src: src,
        }
    }

    fn peek(&self, off: usize) -> Option<char> {
        self.chars.get(self.pos + off).copied()
    }

    fn lit(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        if let Some(Tok::Lit(prev)) = self.out.last_mut() {
            prev.push_str(s);
        } else {
            self.out.push(Tok::Lit(s.to_owned()));
        }
    }

    fn run_of(&mut self, c: char) -> usize {
        let lc = c.to_ascii_lowercase();
        let mut n = 0;
        while self.peek(0).is_some_and(|x| x.to_ascii_lowercase() == lc) {
            n += 1;
            self.pos += 1;
        }
        n
    }

    fn matches_ci(&self, word: &str) -> bool {
        word.chars()
            .enumerate()
            .all(|(i, w)| self.peek(i).is_some_and(|c| c.to_ascii_lowercase() == w))
    }

    fn tokenize(&mut self) -> Result<(), FormatError> {
        while let Some(c) = self.peek(0) {
            match c {
                '"' => {
                    self.pos += 1;
                    let mut s = String::new();
                    loop {
                        match self.peek(0) {
                            Some('"') => {
                                self.pos += 1;
                                break;
                            }
                            Some(ch) => {
                                s.push(ch);
                                self.pos += 1;
                            }
                            None => return Err(FormatError::new("unterminated quote")),
                        }
                    }
                    self.lit(&s);
                }
                '\\' => {
                    let ch = self
                        .peek(1)
                        .ok_or_else(|| FormatError::new("dangling escape"))?;
                    self.pos += 2;
                    self.lit(&ch.to_string());
                }
                '_' => {
                    let ch = self
                        .peek(1)
                        .ok_or_else(|| FormatError::new("dangling pad"))?;
                    self.pos += 2;
                    self.out.push(Tok::Pad(ch));
                }
                '*' => {
                    let ch = self
                        .peek(1)
                        .ok_or_else(|| FormatError::new("dangling fill"))?;
                    self.pos += 2;
                    self.out.push(Tok::Fill(ch));
                }
                '[' => self.bracket()?,
                '0' | '#' | '?' => {
                    self.pos += 1;
                    self.out.push(Tok::Digit(c));
                }
                '1'..='9' => {
                    if matches!(self.out.last(), Some(Tok::Slash)) {
                        let mut n: u64 = 0;
                        while let Some(d) = self.peek(0).and_then(|x| x.to_digit(10)) {
                            n = n.saturating_mul(10).saturating_add(u64::from(d));
                            self.pos += 1;
                        }
                        self.out.push(Tok::FixedDen(n));
                    } else {
                        self.pos += 1;
                        self.lit(&c.to_string());
                    }
                }
                '.' => {
                    self.pos += 1;
                    self.out.push(Tok::Point);
                }
                ',' => {
                    self.pos += 1;
                    self.out.push(Tok::Comma);
                }
                '%' => {
                    self.pos += 1;
                    self.out.push(Tok::Percent);
                }
                '/' => {
                    self.pos += 1;
                    self.out.push(Tok::Slash);
                }
                '@' => {
                    self.pos += 1;
                    self.out.push(Tok::At);
                }
                'E' | 'e' if matches!(self.peek(1), Some('+' | '-')) => {
                    let plus = self.peek(1) == Some('+');
                    if c == 'e' {
                        return Err(FormatError::new("lowercase exponent"));
                    }
                    if !self.peek(2).is_some_and(|d| matches!(d, '0' | '#' | '?')) {
                        return Err(FormatError::new("exponent without digits"));
                    }
                    self.pos += 2;
                    self.out.push(Tok::Exp { plus });
                }
                'G' | 'g' if self.matches_ci("general") => {
                    self.pos += 7;
                    self.out.push(Tok::General);
                }
                'y' | 'Y' => {
                    let n = self.run_of(c);
                    self.out.push(Tok::Year(n));
                }
                'm' | 'M' => {
                    let n = self.run_of(c);
                    self.out.push(Tok::Month(n));
                }
                'd' | 'D' => {
                    let n = self.run_of(c);
                    self.out.push(Tok::Day(n));
                }
                'h' | 'H' => {
                    let n = self.run_of(c);
                    self.out.push(Tok::Hour(n));
                }
                's' | 'S' => {
                    let n = self.run_of(c);
                    self.out.push(Tok::Second(n));
                }
                'a' | 'A' => {
                    if self.matches_ci("am/pm") {
                        self.pos += 5;
                        self.out.push(Tok::AmPm {
                            short: false,
                            upper: c == 'A',
                        });
                    } else if self.matches_ci("a/p") {
                        self.pos += 3;
                        self.out.push(Tok::AmPm {
                            short: true,
                            upper: c == 'A',
                        });
                    } else if self.peek(1).is_some_and(|x| x.eq_ignore_ascii_case(&'a'))
                        && self.peek(2).is_some_and(|x| x.eq_ignore_ascii_case(&'a'))
                    {
                        // `aaa` and `aaaa` are the weekday name; the locale decides how it differs from
                        // `ddd` and `dddd` (identical in English).
                        let n = self.run_of(c);
                        self.out.push(Tok::Day(if n == 3 { 3 } else { 4 }));
                    } else {
                        // One or two `a` are plain letters.
                        self.pos += 1;
                        self.lit(&c.to_string());
                    }
                }
                ' ' | '$' | '-' | '+' | '(' | ')' | ':' | '!' | '^' | '&' | '\'' | '~' | '{'
                | '}' | '<' | '>' | '=' => {
                    self.pos += 1;
                    self.lit(&c.to_string());
                }
                'e' | 'E' => {
                    let n = self.run_of(c);
                    self.out.push(Tok::EraYear(n));
                }
                'b' | 'B'
                    if self.out.is_empty() && self.peek(1).is_some_and(|d| d.is_ascii_digit()) =>
                {
                    // `B1` selects the Gregorian calendar and `B2` the Hijri one; Excel refuses the other digits.
                    self.calendar = match self.peek(1) {
                        Some('1') => 1,
                        Some('2') => 6,
                        _ => return Err(FormatError::new("unknown calendar prefix")),
                    };
                    self.pos += 2;
                }
                'b' | 'B' => {
                    let n = self.run_of(c);
                    self.out.push(Tok::BuddhistYear(n));
                }
                // The Thai date letters: day, month, year (Buddhist), hour, minute, second.
                '\u{0e27}' | '\u{0e14}' | '\u{0e1b}' | '\u{0e0a}' | '\u{0e19}' | '\u{0e17}' => {
                    let n = self.run_of(c);
                    self.thai = true;
                    self.out.push(match c {
                        '\u{0e27}' => Tok::Day(n),
                        '\u{0e14}' => Tok::Month(n),
                        '\u{0e1b}' => Tok::BuddhistYear(if n <= 2 { 2 } else { 4 }),
                        '\u{0e0a}' => Tok::Hour(n),
                        '\u{0e19}' => Tok::Minute(n),
                        _ => Tok::Second(n),
                    });
                }
                'g' | 'G' => {
                    let n = self.run_of(c);
                    self.out.push(Tok::Era(n));
                }
                c if c.is_ascii_alphabetic() && !matches!(c.to_ascii_lowercase(), 'n') => {
                    self.pos += 1;
                    self.lit(&c.to_string());
                }
                c if !c.is_ascii() => {
                    self.pos += 1;
                    self.lit(&c.to_string());
                }
                _ => return Err(FormatError::new("unexpected character")),
            }
        }
        Ok(())
    }

    fn bracket(&mut self) -> Result<(), FormatError> {
        let start = self.pos + 1;
        let end = self.chars[start..]
            .iter()
            .position(|&c| c == ']')
            .map(|i| start + i)
            .ok_or_else(|| FormatError::new("unterminated bracket"))?;
        let body: String = self.chars[start..end].iter().collect();
        self.pos = end + 1;
        if body.eq_ignore_ascii_case("$-F800") || body.eq_ignore_ascii_case("$-F400") {
            // The system long date or time, in its en-US form.
            self.out.clear();
            if body.eq_ignore_ascii_case("$-F800") {
                self.out.extend([
                    Tok::Day(4),
                    Tok::Lit(", ".into()),
                    Tok::Month(4),
                    Tok::Lit(" ".into()),
                    Tok::Day(2),
                    Tok::Lit(", ".into()),
                    Tok::Year(4),
                ]);
            } else {
                self.out.extend([
                    Tok::Hour(1),
                    Tok::Lit(":".into()),
                    Tok::Minute(2),
                    Tok::Lit(":".into()),
                    Tok::Second(2),
                    Tok::Lit(" ".into()),
                    Tok::AmPm {
                        short: false,
                        upper: true,
                    },
                ]);
            }
            self.pos = self.chars.len();
            return Ok(());
        }
        if body.starts_with('$') {
            if let Some((_, tail)) = body.rsplit_once('-')
                && let Ok(code) = u32::from_str_radix(tail, 16)
            {
                self.lcid = Some(code & 0xFFFF);
                self.shape = ((code >> 24) & 0xFF) as u8;
                // A tag without a calendar byte keeps the calendar of a `B1`/`B2` prefix (measured: `B2[$-409]` is Hijri).
                let calendar = ((code >> 16) & 0xFF) as u8;
                if calendar != 0 {
                    self.calendar = calendar;
                }
            }
            let text = currency_text(&body);
            self.lit(&text);
            return Ok(());
        }
        if body.starts_with('<') || body.starts_with('>') || body.starts_with('=') {
            if self.cond.is_some() {
                return Err(FormatError::new("two conditions"));
            }
            self.cond =
                Some(parse_condition(&body).ok_or_else(|| FormatError::new("bad condition"))?);
            return Ok(());
        }
        if is_color(&body) {
            return Ok(());
        }
        let lower = body.to_ascii_lowercase();
        if let Some(n) = lower.strip_prefix("dbnum") {
            // Only 1 to 4 exist; 4 changes nothing in the measured locales.
            return match n {
                "1" | "2" | "3" => {
                    self.dbnum = n.parse().ok();
                    Ok(())
                }
                "4" => Ok(()),
                _ => Err(FormatError::new("unknown DBNum code")),
            };
        }
        let letter = lower.chars().next().unwrap_or(' ');
        if matches!(letter, 'h' | 'm' | 's') && lower.chars().all(|c| c == letter) {
            self.out.push(Tok::Elapsed(letter, lower.len()));
            return Ok(());
        }
        Err(FormatError::new("unknown bracket code"))
    }
}

fn classify(toks: &[Tok]) -> Kind {
    if toks.iter().any(|t| matches!(t, Tok::General)) {
        return Kind::General;
    }
    let date = toks.iter().any(|t| {
        matches!(
            t,
            Tok::Year(_)
                | Tok::Month(_)
                | Tok::Day(_)
                | Tok::Hour(_)
                | Tok::Minute(_)
                | Tok::Second(_)
                | Tok::AmPm { .. }
                | Tok::Elapsed(..)
                | Tok::BuddhistYear(_)
                | Tok::Era(_)
                | Tok::EraYear(_)
        )
    });
    if date {
        return Kind::Date;
    }
    let numeric = toks
        .iter()
        .any(|t| matches!(t, Tok::Digit(_) | Tok::Exp { .. } | Tok::FixedDen(_)));
    if !numeric && toks.iter().any(|t| matches!(t, Tok::At)) {
        return Kind::Text;
    }
    Kind::Number
}

/// Resolves comma candidates (`\u{1}` literals) into grouping or scaling, and `m` into minutes.
fn finish_section(
    mut toks: Vec<Tok>,
    cond: Option<Cond>,
    lcid: Option<u32>,
    dbnum: Option<u8>,
    (shape, calendar, thai): (u8, u8, bool),
) -> Result<Section, FormatError> {
    let kind = classify(&toks);
    let mut grouping = false;
    let mut scale = 0;
    if kind == Kind::Date {
        if toks
            .iter()
            .any(|t| matches!(t, Tok::Digit(_) | Tok::Exp { .. }))
        {
            return Err(FormatError::new("digit placeholder in a date format"));
        }
        for t in &mut toks {
            match t {
                Tok::Point => *t = Tok::Lit(".".into()),
                Tok::Comma => *t = Tok::Lit(",".into()),
                Tok::Percent => *t = Tok::Lit("%".into()),
                Tok::Slash => *t = Tok::Lit("/".into()),
                Tok::FixedDen(n) => *t = Tok::Lit(n.to_string()),
                _ => {}
            }
        }
        // Month versus minute, and fractional seconds.
        let n = toks.len();
        for i in 0..n {
            if let Tok::Month(k) = toks[i]
                && k <= 2
            {
                let prev = toks[..i].iter().rev().find(|t| is_time_or_date(t));
                let next = toks[i + 1..].iter().find(|t| is_time_or_date(t));
                let after_hour = matches!(
                    prev,
                    Some(
                        Tok::Hour(_) | Tok::Elapsed('h', _) | Tok::Second(_) | Tok::Elapsed('s', _)
                    )
                );
                let before_sec = matches!(next, Some(Tok::Second(_) | Tok::Elapsed('s', _)));
                if after_hour || before_sec {
                    toks[i] = Tok::Minute(k);
                }
            }
        }
    } else if kind != Kind::General {
        // Comma candidates.
        let mut i = 0;
        let mut prev_digit = false;
        let mut pending: Vec<usize> = Vec::new();
        while i < toks.len() {
            match &toks[i] {
                Tok::Digit(_) => {
                    if !pending.is_empty() {
                        // commas between digit placeholders group
                        grouping = true;
                        for &p in &pending {
                            toks[p] = Tok::Lit(String::new());
                        }
                        pending.clear();
                    }
                    prev_digit = true;
                }
                Tok::Comma if prev_digit => pending.push(i),
                Tok::Comma => toks[i] = Tok::Lit(",".into()),
                _ => {
                    // A non-digit follows the commas: they scale.
                    for &p in &pending {
                        scale += 1;
                        toks[p] = Tok::Lit(String::new());
                    }
                    pending.clear();
                    if !matches!(&toks[i], Tok::Pad(_) | Tok::Point) {
                        prev_digit = false;
                    }
                }
            }
            i += 1;
        }
        for &p in &pending {
            scale += 1;
            toks[p] = Tok::Lit(String::new());
        }
        // Extra decimal points are literal.
        let mut seen_point = false;
        for t in &mut toks {
            if matches!(t, Tok::Point) {
                if seen_point {
                    *t = Tok::Lit(".".into());
                }
                seen_point = true;
            }
        }
        // A slash needs digit placeholders on both sides; a colon cannot sit in front of one.
        let has_fraction = fraction_shape(&toks);
        if !has_fraction && toks.iter().any(|t| matches!(t, Tok::Slash)) {
            return Err(FormatError::new("slash outside a fraction"));
        }
        for w in toks.windows(2) {
            if matches!(&w[0], Tok::Lit(s) if s.ends_with(':')) && matches!(w[1], Tok::Digit(_)) {
                return Err(FormatError::new("colon before a digit placeholder"));
            }
        }
    } else {
        for t in &mut toks {
            if matches!(t, Tok::Comma) {
                *t = Tok::Lit(",".into());
            }
        }
        if toks.iter().any(|t| {
            matches!(
                t,
                Tok::Digit(_) | Tok::Point | Tok::Percent | Tok::Exp { .. } | Tok::At
            )
        }) {
            return Err(FormatError::new("General mixed with digits"));
        }
    }
    // Merge adjacent literals and drop empties.
    let mut merged: Vec<Tok> = Vec::with_capacity(toks.len());
    for t in toks {
        match t {
            Tok::Lit(s) if s.is_empty() => {}
            Tok::Lit(s) => {
                if let Some(Tok::Lit(prev)) = merged.last_mut() {
                    prev.push_str(&s);
                } else {
                    merged.push(Tok::Lit(s));
                }
            }
            t => merged.push(t),
        }
    }
    let percents = merged.iter().filter(|t| matches!(t, Tok::Percent)).count() as u32;
    let has_ampm = merged.iter().any(|t| matches!(t, Tok::AmPm { .. }));
    Ok(Section {
        toks: merged,
        cond,
        kind,
        grouping,
        scale,
        percents,
        has_ampm,
        lcid,
        dbnum,
        shape,
        calendar,
        thai: thai && lcid.is_none(),
    })
}

fn is_time_or_date(t: &Tok) -> bool {
    matches!(
        t,
        Tok::Year(_)
            | Tok::Month(_)
            | Tok::Day(_)
            | Tok::Hour(_)
            | Tok::Minute(_)
            | Tok::Second(_)
            | Tok::Elapsed(..)
            | Tok::BuddhistYear(_)
    )
}

/// True when the tokens contain digit placeholders, a slash and then more digits or a constant denominator.
pub(super) fn fraction_shape(toks: &[Tok]) -> bool {
    let Some(slash) = toks.iter().position(|t| matches!(t, Tok::Slash)) else {
        return false;
    };
    let before = toks[..slash].iter().any(|t| matches!(t, Tok::Digit(_)));
    let after = toks[slash + 1..]
        .iter()
        .any(|t| matches!(t, Tok::Digit(_) | Tok::FixedDen(_)));
    before && after
}

impl NumberFormat {
    /// Parses a format code.
    pub fn parse(code: &str) -> Result<NumberFormat, FormatError> {
        let parts = split_sections(code)?;
        if parts.len() > 4 {
            return Err(FormatError::new("more than four sections"));
        }
        let mut sections = Vec::with_capacity(parts.len());
        for part in parts {
            let mut tk = Tokenizer::new(part);
            tk.tokenize()?;
            let cond = tk.cond;
            let lcid = tk.lcid;
            let dbnum = tk.dbnum;
            let extra = (tk.shape, tk.calendar, tk.thai);
            let mut toks = tk.out;
            // Fractional seconds are recognised here, where the token order is known.
            let mut i = 0;
            while i + 1 < toks.len() {
                if matches!(toks[i], Tok::Second(_) | Tok::Elapsed('s', _))
                    && matches!(toks[i + 1], Tok::Point)
                {
                    let mut n = 0;
                    while matches!(toks.get(i + 2 + n), Some(Tok::Digit('0'))) {
                        n += 1;
                    }
                    if n > 0 {
                        toks.splice(i + 1..i + 2 + n, [Tok::SubSec(n)]);
                    }
                }
                i += 1;
            }
            sections.push(finish_section(toks, cond, lcid, dbnum, extra)?);
        }
        Ok(NumberFormat { sections })
    }
}
