//! `REGEXTEST`, `REGEXEXTRACT` and `REGEXREPLACE` on the PCRE2-compatible engine (`textpcre.rs`, `textpcrevm.rs`).
//!
//! Behaviour measured against Excel 365:
//! * The text and pattern arguments are converted to text (`REGEXEXTRACT(12345,"\d{2}")` is `12`); an error
//!   argument propagates; a pattern PCRE2 rejects, or a match that exceeds the backtracking limits, is `#VALUE!`.
//! * `case_sensitivity` is 0 (default) or 1 (ignore case); anything else is `#VALUE!`.
//! * `REGEXEXTRACT` return modes: 0 the first match, 1 every match as a row, 2 the capture groups of the first
//!   match as a row (`#N/A` for a group that did not take part; `#VALUE!` when the pattern has no group).
//!   No match gives `#N/A`.
//! * `REGEXREPLACE` replaces every match, or the n-th (negative n counts from the end; a match that does not
//!   exist leaves the text unchanged). Matches follow `pcre2_substitute`: after an empty match the same place
//!   is retried for a non-empty one. In the replacement `$1`..`$99`, `${1}`, `${name}`, `$<name>`, `$0` and `$&`
//!   are groups, `` $` `` the text before the match, `$'` the text after, `$_` the whole text, `$$` a dollar
//!   sign, `${1:-default}` and `${1:+set:unset}` conditional text; a backslash escapes `\n \t \r \a \e \f`,
//!   `\xhh`, `\x{h..}`, `\o{..}`, a three-digit octal, `\1`..`\9` (groups), `\U \L \E \u \l` (case changes),
//!   and any non-alphanumeric character. Anything else after `$` or `\`, or a group that does not exist, is
//!   `#VALUE!`.

use super::textchar::trunc_arg;
use super::textpcre::{PcreError, Regex, compile};
use super::textpcrevm::Caps;
use super::{Res, arity, optional, text_arg};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

fn case_flag(ctx: &EvalContext<'_>, args: &[Arg], index: usize) -> Res<bool> {
    match optional(args, index) {
        None => Ok(false),
        Some(a) => match trunc_arg(ctx, a)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ErrorKind::Value),
        },
    }
}

fn build(pattern: &str, caseless: bool) -> Res<Regex> {
    compile(pattern, caseless).map_err(|PcreError| ErrorKind::Value)
}

fn slice(hay: &[char], span: (usize, usize)) -> String {
    hay[span.0.min(hay.len())..span.1.min(hay.len())]
        .iter()
        .collect()
}

pub(super) fn regextest(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 3)?;
    let text = text_arg(ctx, &args[0])?;
    let pattern = text_arg(ctx, &args[1])?;
    let caseless = case_flag(ctx, args, 2)?;
    let re = build(&pattern, caseless)?;
    let hay: Vec<char> = text.chars().collect();
    let found = re
        .exec(&hay, 0, false, false)
        .map_err(|_| ErrorKind::Value)?;
    Ok(Value::Bool(found.is_some()))
}

pub(super) fn regexextract(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 4)?;
    let text = text_arg(ctx, &args[0])?;
    let pattern = text_arg(ctx, &args[1])?;
    let mode = match optional(args, 2) {
        None => 0,
        Some(a) => trunc_arg(ctx, a)?,
    };
    if !(0..=2).contains(&mode) {
        return Err(ErrorKind::Value);
    }
    let caseless = case_flag(ctx, args, 3)?;
    let re = build(&pattern, caseless)?;
    let hay: Vec<char> = text.chars().collect();
    let abort = |_| ErrorKind::Value;
    match mode {
        0 => {
            let caps = re
                .exec(&hay, 0, false, false)
                .map_err(abort)?
                .ok_or(ErrorKind::NA)?;
            Ok(Value::Text(slice(&hay, caps[0].unwrap_or((0, 0)))))
        }
        1 => {
            let all = re.all_matches(&hay, usize::MAX).map_err(abort)?;
            let mut row: Vec<Value> = all
                .iter()
                .map(|c| Value::Text(slice(&hay, c[0].unwrap_or((0, 0)))))
                .collect();
            match row.len() {
                0 => Err(ErrorKind::NA),
                1 => Ok(row.remove(0)),
                _ => Ok(Value::Array(vec![row])),
            }
        }
        _ => {
            if re.ngroups == 0 {
                return Err(ErrorKind::Value);
            }
            let caps = re
                .exec(&hay, 0, false, false)
                .map_err(abort)?
                .ok_or(ErrorKind::NA)?;
            let row: Vec<Value> = caps[1..]
                .iter()
                .map(|c| match c {
                    Some(span) => Value::Text(slice(&hay, *span)),
                    None => Value::Error(ErrorKind::NA),
                })
                .collect();
            Ok(Value::Array(vec![row]))
        }
    }
}

/// How the text inserted from here on changes case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaseOp {
    Upper,
    Lower,
    End,
    NextUpper,
    NextLower,
}

#[derive(Debug, Clone)]
enum Part {
    Text(String),
    Group(usize),
    Prefix,
    Suffix,
    Subject,
    /// `${n:-default}`
    Default(usize, String),
    /// `${n:+set:unset}`
    Choice(usize, String, String),
    Case(CaseOp),
}

struct Template<'a> {
    s: Vec<char>,
    pos: usize,
    re: &'a Regex,
}

impl Template<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.pos).copied()
    }

    fn group_by_name(&self, name: &str) -> Option<usize> {
        self.re
            .names
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, i)| *i)
    }

    fn group_ref(&self, spec: &str) -> Res<usize> {
        let idx = if !spec.is_empty() && spec.bytes().all(|b| b.is_ascii_digit()) {
            spec.parse::<usize>().map_err(|_| ErrorKind::Value)?
        } else {
            self.group_by_name(spec).ok_or(ErrorKind::Value)?
        };
        if idx > self.re.ngroups {
            return Err(ErrorKind::Value);
        }
        Ok(idx)
    }

    fn parse(mut self) -> Res<Vec<Part>> {
        let mut parts = Vec::new();
        let mut text = String::new();
        let flush = |text: &mut String, parts: &mut Vec<Part>| {
            if !text.is_empty() {
                parts.push(Part::Text(std::mem::take(text)));
            }
        };
        while let Some(c) = self.peek() {
            self.pos += 1;
            match c {
                '$' => {
                    flush(&mut text, &mut parts);
                    let part = self.dollar()?;
                    parts.push(part);
                }
                '\\' => match self.backslash()? {
                    Esc::Char(ch) => text.push(ch),
                    Esc::Part(p) => {
                        flush(&mut text, &mut parts);
                        parts.push(p);
                    }
                },
                _ => text.push(c),
            }
        }
        flush(&mut text, &mut parts);
        Ok(parts)
    }

    fn dollar(&mut self) -> Res<Part> {
        let c = self.peek().ok_or(ErrorKind::Value)?;
        self.pos += 1;
        match c {
            '$' => Ok(Part::Text("$".into())),
            '&' => Ok(Part::Group(0)),
            '`' => Ok(Part::Prefix),
            '\'' => Ok(Part::Suffix),
            '_' => Ok(Part::Subject),
            '0'..='9' => {
                let mut spec = c.to_string();
                while let Some(d) = self.peek().filter(char::is_ascii_digit) {
                    spec.push(d);
                    self.pos += 1;
                }
                Ok(Part::Group(self.group_ref(&spec)?))
            }
            '<' => {
                let mut name = String::new();
                loop {
                    match self.peek() {
                        Some('>') => {
                            self.pos += 1;
                            break;
                        }
                        Some(ch) => {
                            name.push(ch);
                            self.pos += 1;
                        }
                        None => return Err(ErrorKind::Value),
                    }
                }
                Ok(Part::Group(self.group_ref(&name)?))
            }
            '{' => {
                let mut body = String::new();
                loop {
                    match self.peek() {
                        Some('}') => {
                            self.pos += 1;
                            break;
                        }
                        Some(ch) => {
                            body.push(ch);
                            self.pos += 1;
                        }
                        None => return Err(ErrorKind::Value),
                    }
                }
                if let Some((spec, default)) = body.split_once(":-") {
                    return Ok(Part::Default(self.group_ref(spec)?, default.to_owned()));
                }
                if let Some((spec, rest)) = body.split_once(":+") {
                    let (set, unset) = rest.split_once(':').unwrap_or((rest, ""));
                    return Ok(Part::Choice(
                        self.group_ref(spec)?,
                        set.to_owned(),
                        unset.to_owned(),
                    ));
                }
                Ok(Part::Group(self.group_ref(&body)?))
            }
            _ => Err(ErrorKind::Value),
        }
    }

    fn backslash(&mut self) -> Res<Esc> {
        let c = self.peek().ok_or(ErrorKind::Value)?;
        self.pos += 1;
        let ch = |c: char| Ok(Esc::Char(c));
        match c {
            'n' => ch('\n'),
            't' => ch('\t'),
            'r' => ch('\r'),
            'a' => ch('\u{7}'),
            'e' => ch('\u{1b}'),
            'f' => ch('\u{c}'),
            'U' => Ok(Esc::Part(Part::Case(CaseOp::Upper))),
            'L' => Ok(Esc::Part(Part::Case(CaseOp::Lower))),
            'E' => Ok(Esc::Part(Part::Case(CaseOp::End))),
            'u' => Ok(Esc::Part(Part::Case(CaseOp::NextUpper))),
            'l' => Ok(Esc::Part(Part::Case(CaseOp::NextLower))),
            'x' => {
                let mut v: u32 = 0;
                if self.peek() == Some('{') {
                    self.pos += 1;
                    let mut any = false;
                    while let Some(d) = self.peek().and_then(|c| c.to_digit(16)) {
                        v = v.saturating_mul(16).saturating_add(d);
                        any = true;
                        self.pos += 1;
                    }
                    if !any || self.peek() != Some('}') {
                        return Err(ErrorKind::Value);
                    }
                    self.pos += 1;
                } else {
                    for _ in 0..2 {
                        match self.peek().and_then(|c| c.to_digit(16)) {
                            Some(d) => {
                                v = v * 16 + d;
                                self.pos += 1;
                            }
                            None => break,
                        }
                    }
                }
                char::from_u32(v).map(Esc::Char).ok_or(ErrorKind::Value)
            }
            'o' => {
                if self.peek() != Some('{') {
                    return Err(ErrorKind::Value);
                }
                self.pos += 1;
                let mut v: u32 = 0;
                let mut any = false;
                while let Some(d) = self.peek().and_then(|c| c.to_digit(8)) {
                    v = v.saturating_mul(8).saturating_add(d);
                    any = true;
                    self.pos += 1;
                }
                if !any || self.peek() != Some('}') {
                    return Err(ErrorKind::Value);
                }
                self.pos += 1;
                char::from_u32(v).map(Esc::Char).ok_or(ErrorKind::Value)
            }
            '0'..='9' => {
                let mut run = c.to_string();
                while let Some(d) = self.peek().filter(char::is_ascii_digit) {
                    run.push(d);
                    self.pos += 1;
                }
                let octal3 =
                    run.len() >= 3 && run.bytes().take(3).all(|b| (b'0'..=b'7').contains(&b));
                if octal3 {
                    // Three octal digits are a character; any digits after them are literal.
                    let v = u32::from_str_radix(&run[..3], 8).map_err(|_| ErrorKind::Value)?;
                    self.pos -= run.len() - 3;
                    return char::from_u32(v).map(Esc::Char).ok_or(ErrorKind::Value);
                }
                if run.starts_with('0') {
                    return Err(ErrorKind::Value);
                }
                Ok(Esc::Part(Part::Group(self.group_ref(&run)?)))
            }
            c if c.is_ascii_alphanumeric() => Err(ErrorKind::Value),
            c => ch(c),
        }
    }
}

enum Esc {
    Char(char),
    Part(Part),
}

/// Applies the case changes of `\U \L \E \u \l` to inserted text.
struct Out {
    text: String,
    mode: Option<CaseOp>,
    next: Option<CaseOp>,
}

impl Out {
    fn push(&mut self, s: &str) {
        for ch in s.chars() {
            let op = self.next.take().or(self.mode);
            match op {
                Some(CaseOp::Upper | CaseOp::NextUpper) => self.text.extend(ch.to_uppercase()),
                Some(CaseOp::Lower | CaseOp::NextLower) => self.text.extend(ch.to_lowercase()),
                _ => self.text.push(ch),
            }
        }
    }
}

fn render(parts: &[Part], hay: &[char], caps: &Caps, out: &mut Out) {
    let (ms, me) = caps[0].unwrap_or((0, 0));
    let group = |i: usize| caps.get(i).copied().flatten();
    for p in parts {
        match p {
            Part::Text(t) => out.push(t),
            Part::Group(i) => {
                if let Some(span) = group(*i) {
                    out.push(&slice(hay, span));
                }
            }
            Part::Prefix => out.push(&slice(hay, (0, ms))),
            Part::Suffix => out.push(&slice(hay, (me, hay.len()))),
            Part::Subject => out.push(&slice(hay, (0, hay.len()))),
            Part::Default(i, d) => match group(*i) {
                Some(span) => out.push(&slice(hay, span)),
                None => out.push(d),
            },
            Part::Choice(i, set, unset) => {
                if group(*i).is_some() {
                    out.push(set);
                } else {
                    out.push(unset);
                }
            }
            Part::Case(op) => match op {
                CaseOp::Upper | CaseOp::Lower => {
                    out.mode = Some(*op);
                    out.next = None;
                }
                CaseOp::End => {
                    out.mode = None;
                    out.next = None;
                }
                CaseOp::NextUpper | CaseOp::NextLower => out.next = Some(*op),
            },
        }
    }
}

pub(super) fn regexreplace(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 5)?;
    let text = text_arg(ctx, &args[0])?;
    let pattern = text_arg(ctx, &args[1])?;
    let replacement = text_arg(ctx, &args[2])?;
    let occurrence = match optional(args, 3) {
        None => 0,
        Some(a) => trunc_arg(ctx, a)?,
    };
    let caseless = case_flag(ctx, args, 4)?;
    let re = build(&pattern, caseless)?;
    let hay: Vec<char> = text.chars().collect();
    let all = re
        .all_matches(&hay, usize::MAX)
        .map_err(|_| ErrorKind::Value)?;
    let chosen: Vec<&Caps> = if occurrence == 0 {
        all.iter().collect()
    } else {
        let n = all.len() as i64;
        let idx = if occurrence > 0 {
            occurrence - 1
        } else {
            n + occurrence
        };
        if (0..n).contains(&idx) {
            vec![&all[idx as usize]]
        } else {
            Vec::new()
        }
    };
    // The replacement is read only when there is a match to replace: a bad one is not an error otherwise.
    let parts = if chosen.is_empty() {
        Vec::new()
    } else {
        Template {
            s: replacement.chars().collect(),
            pos: 0,
            re: &re,
        }
        .parse()?
    };
    let mut out = Out {
        text: String::new(),
        mode: None,
        next: None,
    };
    let mut last = 0;
    for caps in chosen {
        let (s, e) = caps[0].unwrap_or((last, last));
        let s = s.max(last);
        out.text.push_str(&slice(&hay, (last, s)));
        // The case state does not carry over from one match to the next.
        out.mode = None;
        out.next = None;
        render(&parts, &hay, caps, &mut out);
        last = e.max(s);
    }
    out.text.push_str(&slice(&hay, (last, hay.len())));
    Ok(Value::Text(out.text))
}
