//! `*IF` / `*IFS` functions and the shared criteria and wildcard matchers.
//!
//! Criteria are either a value (matches equal cells) or text with an optional comparison
//! operator prefix (`>5`, `<>a`, `>=2`, `=`). Text patterns support the wildcards `?`, `*` and
//! the escape `~`, and compare case-insensitively.

use std::cmp::Ordering;

use super::{Res, arity, number};
use crate::eval::{Arg, EvalContext, Grid};
use crate::value::{ErrorKind, Value, compare_numbers, compare_text, parse_number_text};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pat {
    Lit(char),
    Any,
    Star,
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn compile(pattern: &str) -> Vec<Pat> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '~' if matches!(chars.get(i + 1), Some('?' | '*' | '~')) => {
                out.push(Pat::Lit(fold(chars[i + 1])));
                i += 1;
            }
            '?' => out.push(Pat::Any),
            '*' => out.push(Pat::Star),
            c => out.push(Pat::Lit(fold(c))),
        }
        i += 1;
    }
    out
}

fn match_full(pat: &[Pat], text: &[char]) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if let Some(&token) = pat.get(p) {
            match token {
                Pat::Star => {
                    star = Some((p, t));
                    p += 1;
                    continue;
                }
                Pat::Any => {
                    p += 1;
                    t += 1;
                    continue;
                }
                Pat::Lit(c) if c == fold(text[t]) => {
                    p += 1;
                    t += 1;
                    continue;
                }
                Pat::Lit(_) => {}
            }
        }
        match star {
            Some((sp, st)) => {
                p = sp + 1;
                t = st + 1;
                star = Some((sp, st + 1));
            }
            None => return false,
        }
    }
    pat[p..].iter().all(|t| *t == Pat::Star)
}

/// Case-insensitive match of the whole text against a wildcard pattern.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let text: Vec<char> = text.chars().collect();
    match_full(&compile(pattern), &text)
}

/// Whether the text contains wildcard syntax.
pub(crate) fn has_wildcards(text: &str) -> bool {
    text.contains(['?', '*', '~'])
}

/// Character index of the first substring (at or after `start`) matching the pattern.
pub(crate) fn wildcard_search(pattern: &str, text: &str, start: usize) -> Option<usize> {
    let mut pat = compile(pattern);
    pat.push(Pat::Star);
    let text: Vec<char> = text.chars().collect();
    (start..=text.len()).find(|&i| match_full(&pat, &text[i..]))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    fn test(self, ord: Ordering) -> bool {
        match self {
            CmpOp::Eq => ord == Ordering::Equal,
            CmpOp::Ne => ord != Ordering::Equal,
            CmpOp::Lt => ord == Ordering::Less,
            CmpOp::Le => ord != Ordering::Greater,
            CmpOp::Gt => ord == Ordering::Greater,
            CmpOp::Ge => ord != Ordering::Less,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Operand {
    Number(f64),
    Bool(bool),
    Error(ErrorKind),
    Text(String),
}

/// A parsed `*IF` criterion.
#[derive(Debug, Clone, PartialEq)]
pub struct Criteria {
    op: CmpOp,
    operand: Operand,
    /// An operator was written (matters for `"="` versus `""`).
    explicit: bool,
}

impl Criteria {
    /// Builds a criterion from a cell value or literal.
    pub fn from_value(v: &Value) -> Criteria {
        let operand = match v {
            Value::Number(n) => Operand::Number(*n),
            Value::Bool(b) => Operand::Bool(*b),
            Value::Error(e) => Operand::Error(*e),
            Value::Empty => Operand::Number(0.0),
            Value::Text(t) => return Criteria::parse(t),
            Value::Array(_) => return Criteria::from_value(&v.top_left()),
        };
        Criteria {
            op: CmpOp::Eq,
            operand,
            explicit: false,
        }
    }

    /// Parses criterion text such as `">5"`, `"<>a"`, `"a*"` or `"="`.
    pub fn parse(text: &str) -> Criteria {
        let (op, rest, explicit) = [
            (">=", CmpOp::Ge),
            ("<=", CmpOp::Le),
            ("<>", CmpOp::Ne),
            (">", CmpOp::Gt),
            ("<", CmpOp::Lt),
            ("=", CmpOp::Eq),
        ]
        .iter()
        .find_map(|(prefix, op)| text.strip_prefix(prefix).map(|r| (*op, r, true)))
        .unwrap_or((CmpOp::Eq, text, false));
        let operand = if rest.is_empty() {
            Operand::Text(String::new())
        } else if let Some(n) = parse_number_text(rest) {
            Operand::Number(n)
        } else if rest.eq_ignore_ascii_case("TRUE") {
            Operand::Bool(true)
        } else if rest.eq_ignore_ascii_case("FALSE") {
            Operand::Bool(false)
        } else if let Some(e) = ErrorKind::parse(rest) {
            Operand::Error(e)
        } else {
            Operand::Text(rest.to_owned())
        };
        Criteria {
            op,
            operand,
            explicit,
        }
    }

    /// Whether a cell value satisfies the criterion.
    pub fn matches(&self, cell: &Value) -> bool {
        let negated = self.op == CmpOp::Ne;
        match &self.operand {
            Operand::Number(n) => match cell {
                Value::Number(c) => self.op.test(compare_numbers(*c, *n)),
                Value::Text(t) if matches!(self.op, CmpOp::Eq | CmpOp::Ne) => {
                    match parse_number_text(t) {
                        Some(c) => self.op.test(compare_numbers(c, *n)),
                        None => negated,
                    }
                }
                _ => negated,
            },
            Operand::Bool(b) => match cell {
                Value::Bool(c) => self.op.test(c.cmp(b)),
                _ => negated,
            },
            Operand::Error(e) => match cell {
                Value::Error(c) => match self.op {
                    CmpOp::Eq => c == e,
                    CmpOp::Ne => c != e,
                    _ => false,
                },
                _ => negated,
            },
            Operand::Text(pattern) if pattern.is_empty() => match self.op {
                CmpOp::Eq if self.explicit => matches!(cell, Value::Empty),
                CmpOp::Eq => {
                    matches!(cell, Value::Empty) || matches!(cell, Value::Text(t) if t.is_empty())
                }
                CmpOp::Ne => !matches!(cell, Value::Empty),
                _ => false,
            },
            Operand::Text(pattern) => match (self.op, cell) {
                (CmpOp::Eq, Value::Text(t)) => wildcard_match(pattern, t),
                (CmpOp::Eq, _) => false,
                (CmpOp::Ne, Value::Text(t)) => !wildcard_match(pattern, t),
                (CmpOp::Ne, _) => true,
                (op, Value::Text(t)) => op.test(compare_text(t, pattern)),
                _ => false,
            },
        }
    }
}

fn range_grid<'a>(ctx: &'a EvalContext<'a>, arg: &'a Arg) -> Res<Grid<'a>> {
    match arg {
        Arg::Range(_) | Arg::Array(_) => ctx.grid(arg).ok_or(ErrorKind::Value),
        _ => Err(ErrorKind::Value),
    }
}

fn criterion(ctx: &EvalContext<'_>, arg: &Arg) -> Criteria {
    Criteria::from_value(&ctx.scalar(arg))
}

pub(super) fn countif(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    countifs(ctx, args)
}

pub(super) fn countifs(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    if args.len() < 2 || !args.len().is_multiple_of(2) {
        return Err(ErrorKind::Value);
    }
    let pairs = criteria_pairs(ctx, args)?;
    let mut count = 0u64;
    for_each_match(&pairs, |_, _| count += 1);
    Ok(Value::Number(count as f64))
}

/// Criteria ranges with their criteria; all ranges must share one shape.
fn criteria_pairs<'a>(ctx: &'a EvalContext<'a>, args: &'a [Arg]) -> Res<Vec<(Grid<'a>, Criteria)>> {
    let mut out: Vec<(Grid<'a>, Criteria)> = Vec::new();
    for pair in args.chunks(2) {
        let grid = range_grid(ctx, &pair[0])?;
        if let Some((first, _)) = out.first()
            && (first.rows() != grid.rows() || first.cols() != grid.cols())
        {
            return Err(ErrorKind::Value);
        }
        out.push((grid, criterion(ctx, &pair[1])));
    }
    Ok(out)
}

/// Calls `f(row, col)` for every position where all criteria match.
fn for_each_match(pairs: &[(Grid<'_>, Criteria)], mut f: impl FnMut(usize, usize)) {
    let rows = pairs.iter().map(|(g, _)| g.scan_rows()).max().unwrap_or(0);
    let cols = pairs.iter().map(|(g, _)| g.scan_cols()).max().unwrap_or(0);
    for r in 0..rows {
        for c in 0..cols {
            if pairs.iter().all(|(g, crit)| crit.matches(&g.get(r, c))) {
                f(r, c);
            }
        }
    }
}

/// Sums or averages numeric cells of `values` at positions matching the criteria.
fn conditional_total(
    ctx: &EvalContext<'_>,
    values: &Arg,
    criteria_args: &[Arg],
) -> Res<(f64, u64)> {
    let pairs = criteria_pairs(ctx, criteria_args)?;
    let value_grid = range_grid(ctx, values)?;
    if let Some((first, _)) = pairs.first()
        && (first.rows() != value_grid.rows() || first.cols() != value_grid.cols())
        && !matches!(values, Arg::Range(_))
    {
        return Err(ErrorKind::Value);
    }
    let (mut total, mut n) = (0.0, 0u64);
    let mut failure = None;
    for_each_match(&pairs, |r, c| match value_grid.get(r, c) {
        Value::Number(x) => {
            total += x;
            n += 1;
        }
        Value::Error(e) => failure = failure.or(Some(e)),
        _ => {}
    });
    match failure {
        Some(e) => Err(e),
        None => Ok((total, n)),
    }
}

pub(super) fn sumif(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 3)?;
    let sum_range = args
        .get(2)
        .filter(|a| !matches!(a, Arg::Missing))
        .unwrap_or(&args[0]);
    let (total, _) = conditional_total(ctx, sum_range, &args[..2])?;
    number(total)
}

pub(super) fn averageif(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 3)?;
    let avg_range = args
        .get(2)
        .filter(|a| !matches!(a, Arg::Missing))
        .unwrap_or(&args[0]);
    let (total, n) = conditional_total(ctx, avg_range, &args[..2])?;
    if n == 0 {
        return Err(ErrorKind::DivZero);
    }
    number(total / n as f64)
}

pub(super) fn sumifs(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    if args.len() < 3 || args.len().is_multiple_of(2) {
        return Err(ErrorKind::Value);
    }
    let sum_grid = range_grid(ctx, &args[0])?;
    let first = range_grid(ctx, &args[1])?;
    if sum_grid.rows() != first.rows() || sum_grid.cols() != first.cols() {
        return Err(ErrorKind::Value);
    }
    let (total, _) = conditional_total(ctx, &args[0], &args[1..])?;
    number(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(criterion: &str, cell: impl Into<Value>) -> bool {
        Criteria::parse(criterion).matches(&cell.into())
    }

    #[test]
    fn wildcards() {
        assert!(wildcard_match("a*", "Apple"));
        assert!(wildcard_match("*e", "Apple"));
        assert!(wildcard_match("a?ple", "APPLE"));
        assert!(!wildcard_match("a?ple", "aple"));
        assert!(wildcard_match("*", ""));
        assert!(wildcard_match("a*b*c", "a--b--c"));
        assert!(!wildcard_match("a*b*c", "a--c--b"));
        assert!(wildcard_match("~*", "*"));
        assert!(!wildcard_match("~*", "x"));
        assert!(wildcard_match("a~?b", "a?b"));
        assert!(!wildcard_match("a~?b", "axb"));
        assert!(wildcard_match("a~~b", "a~b"));
        assert!(wildcard_match("~x", "~x"));
        assert!(wildcard_match("", ""));
        assert!(!wildcard_match("", "a"));
    }

    #[test]
    fn wildcard_search_finds_first_position() {
        assert_eq!(wildcard_search("b*d", "abcd", 0), Some(1));
        assert_eq!(wildcard_search("c", "abcabc", 3), Some(5));
        assert_eq!(wildcard_search("z", "abc", 0), None);
        assert_eq!(wildcard_search("", "abc", 1), Some(1));
        assert_eq!(wildcard_search("?c", "abc", 0), Some(1));
    }

    #[test]
    fn numeric_comparisons() {
        assert!(m(">5", 6.0));
        assert!(!m(">5", 5.0));
        assert!(m(">=5", 5.0));
        assert!(m("<5", 4.0));
        assert!(m("<=5", 5.0));
        assert!(m("=5", 5.0));
        assert!(m("<>5", 6.0));
        assert!(!m("<>5", 5.0));
        assert!(m("<>5", "text"));
        assert!(m("<>5", Value::Empty));
        assert!(!m(">5", "10"));
        assert!(!m(">5", Value::Empty));
        assert!(m("5", 5.0));
        assert!(m("5", "5"), "numeric criteria also match numeric text");
        assert!(!m("5", "five"));
        assert!(!m("=0", Value::Empty));
        assert!(m("-1.5", -1.5));
        assert!(m("50%", 0.5));
    }

    #[test]
    fn text_comparisons() {
        assert!(m("a", "A"));
        assert!(m("=a", "A"));
        assert!(!m("a", "ab"));
        assert!(m("a*", "Apple"));
        assert!(m("<>a", "b"));
        assert!(!m("<>a", "A"));
        assert!(m("<>a", 3.0));
        assert!(m("<>a", Value::Empty));
        assert!(m(">b", "c"));
        assert!(!m(">b", "a"));
        assert!(!m(">b", 3.0));
        assert!(!m("a*", 3.0));
        assert!(m("?", "x"));
        assert!(!m("?", "xy"));
    }

    #[test]
    fn blank_criteria() {
        assert!(m("=", Value::Empty));
        assert!(!m("=", ""));
        assert!(m("", Value::Empty));
        assert!(m("", ""));
        assert!(!m("", "x"));
        assert!(m("<>", "x"));
        assert!(m("<>", 0.0));
        assert!(!m("<>", Value::Empty));
    }

    #[test]
    fn bool_and_error_criteria() {
        assert!(m("TRUE", true));
        assert!(!m("TRUE", false));
        assert!(!m("TRUE", 1.0));
        assert!(m("<>TRUE", false));
        assert!(m("#N/A", ErrorKind::NA));
        assert!(!m("#N/A", ErrorKind::Ref));
        assert!(m("<>#N/A", ErrorKind::Ref));
    }

    #[test]
    fn from_value_literals() {
        assert!(Criteria::from_value(&Value::Number(2.0)).matches(&Value::Number(2.0)));
        assert!(Criteria::from_value(&Value::Bool(true)).matches(&Value::Bool(true)));
        assert!(
            Criteria::from_value(&Value::Error(ErrorKind::NA))
                .matches(&Value::Error(ErrorKind::NA))
        );
        assert!(Criteria::from_value(&Value::Empty).matches(&Value::Number(0.0)));
        assert!(Criteria::from_value(&Value::from(">1")).matches(&Value::Number(2.0)));
    }
}
