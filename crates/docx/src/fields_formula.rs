//! The `=` (formula) field: Word's expression syntax and its decimal "scale" rule.
//!
//! Word shows a result with as many decimals as its most precise operand, at least two after a
//! division, and drops trailing zeros. [`Num`] carries that scale along the evaluation.

use crate::fields_number::{default_number_text, round_decimal};

/// A number with the scale Word would print it with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Num {
    pub value: f64,
    pub scale: u32,
}

impl Num {
    const fn int(value: f64) -> Self {
        Self { value, scale: 0 }
    }

    /// The default text of the result (no `\#` picture).
    #[must_use]
    pub fn text(self) -> String {
        default_number_text(self.value, self.scale)
    }
}

/// Values the formula can look up.
pub trait FormulaEnv {
    /// Numeric content of bookmark `name`.
    fn bookmark_value(&self, name: &str) -> Option<f64>;
    /// True when the bookmark exists, even if its text is not a number.
    fn bookmark_exists(&self, name: &str) -> bool;
    /// Numbers of a table range: `ABOVE`, `LEFT`, `BELOW`, `RIGHT` or `A1:B3` / `A1`.
    fn table_values(&self, reference: &str) -> Option<Vec<f64>>;
}

/// Evaluates `src`. The error is the text Word shows in the result (`!Zero Divide`...).
pub fn evaluate(src: &str, env: &dyn FormulaEnv) -> Result<Num, String> {
    let tokens = lex(src)?;
    let mut parser = Parser { tokens, at: 0, env };
    let value = parser.compare()?;
    match parser.peek() {
        None => Ok(value),
        Some(tok) => Err(match tok.kind {
            Kind::Num | Kind::Ident => "!Missing Operator".to_owned(),
            _ => format!("!Syntax Error, {}", tok.text),
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Num,
    Ident,
    Op,
    LParen,
    RParen,
    Comma,
    Str,
}

#[derive(Debug, Clone)]
struct Tok {
    kind: Kind,
    text: String,
    scale: u32,
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit()
            || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let mut scale = 0u32;
            if chars.get(i) == Some(&'.') {
                i += 1;
                let frac_start = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                scale = u32::try_from(i - frac_start).unwrap_or(0);
            }
            out.push(Tok {
                kind: Kind::Num,
                text: chars[start..i].iter().collect(),
                scale,
            });
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.')
            {
                i += 1;
            }
            // A cell range such as `A1:B3` is one word.
            if chars.get(i) == Some(&':')
                && chars.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic())
            {
                i += 1;
                while i < chars.len() && chars[i].is_ascii_alphanumeric() {
                    i += 1;
                }
            }
            out.push(Tok {
                kind: Kind::Ident,
                text: chars[start..i].iter().collect(),
                scale: 0,
            });
        } else if c == '"' {
            return Err("!Syntax Error, \"".to_owned());
        } else {
            let two: String = chars[i..].iter().take(2).collect();
            let (kind, text) = match two.as_str() {
                "<=" | ">=" | "<>" => (Kind::Op, two),
                _ => {
                    let kind = match c {
                        '(' => Kind::LParen,
                        ')' => Kind::RParen,
                        ',' | ';' => Kind::Comma,
                        '+' | '-' | '*' | '/' | '^' | '%' | '=' | '<' | '>' => Kind::Op,
                        _ => return Err(format!("!Syntax Error, {c}")),
                    };
                    (kind, c.to_string())
                }
            };
            i += text.chars().count();
            out.push(Tok {
                kind,
                text,
                scale: 0,
            });
        }
    }
    let _ = Kind::Str;
    Ok(out)
}

struct Parser<'a> {
    tokens: Vec<Tok>,
    at: usize,
    env: &'a dyn FormulaEnv,
}

fn bool_num(b: bool) -> Num {
    Num::int(if b { 1.0 } else { 0.0 })
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.at)
    }

    fn peek_op(&self, ops: &[&str]) -> Option<String> {
        self.peek()
            .filter(|t| t.kind == Kind::Op && ops.contains(&t.text.as_str()))
            .map(|t| t.text.clone())
    }

    fn compare(&mut self) -> Result<Num, String> {
        let mut left = self.add()?;
        while let Some(op) = self.peek_op(&["=", "<", ">", "<=", ">=", "<>"]) {
            self.at += 1;
            let right = self.add()?;
            let (a, b) = (left.value, right.value);
            left = bool_num(match op.as_str() {
                "=" => a == b,
                "<" => a < b,
                ">" => a > b,
                "<=" => a <= b,
                ">=" => a >= b,
                _ => a != b,
            });
        }
        Ok(left)
    }

    fn add(&mut self) -> Result<Num, String> {
        let mut left = self.mul()?;
        while let Some(op) = self.peek_op(&["+", "-"]) {
            self.at += 1;
            let right = self.mul()?;
            left = Num {
                value: if op == "+" {
                    left.value + right.value
                } else {
                    left.value - right.value
                },
                scale: left.scale.max(right.scale),
            };
        }
        Ok(left)
    }

    fn mul(&mut self) -> Result<Num, String> {
        let mut left = self.pow()?;
        while let Some(op) = self.peek_op(&["*", "/"]) {
            self.at += 1;
            let right = self.pow()?;
            left = if op == "*" {
                Num {
                    value: left.value * right.value,
                    scale: left.scale.max(right.scale),
                }
            } else {
                divide(left, right)?
            };
        }
        Ok(left)
    }

    fn pow(&mut self) -> Result<Num, String> {
        let base = self.unary()?;
        if self.peek_op(&["^"]).is_some() {
            self.at += 1;
            let exp = self.pow()?;
            let value = base.value.powf(exp.value);
            if !value.is_finite() {
                return Err("!Zero Divide".to_owned());
            }
            let scale = if exp.value < 0.0 {
                base.scale.max(exp.scale).max(2)
            } else {
                base.scale.max(exp.scale)
            };
            return Ok(Num { value, scale });
        }
        Ok(base)
    }

    fn unary(&mut self) -> Result<Num, String> {
        if self.peek_op(&["-"]).is_some() {
            self.at += 1;
            let inner = self.unary()?;
            return Ok(Num {
                value: -inner.value,
                scale: inner.scale,
            });
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Result<Num, String> {
        let mut value = self.primary()?;
        if self.peek_op(&["%"]).is_some() {
            self.at += 1;
            value = Num {
                value: value.value / 100.0,
                scale: value.scale + 2,
            };
        }
        Ok(value)
    }

    fn expect_close(&mut self) -> Result<(), String> {
        match self.peek() {
            None => Err("!Unexpected End of Formula".to_owned()),
            Some(t) if t.kind == Kind::RParen => {
                self.at += 1;
                Ok(())
            }
            Some(t) if matches!(t.kind, Kind::Num | Kind::Ident) => {
                Err("!Missing Operator".to_owned())
            }
            Some(t) => Err(format!("!Syntax Error, {}", t.text)),
        }
    }

    fn primary(&mut self) -> Result<Num, String> {
        let Some(tok) = self.peek().cloned() else {
            return Err("!Unexpected End of Formula".to_owned());
        };
        self.at += 1;
        match tok.kind {
            Kind::Num => {
                let value: f64 = tok
                    .text
                    .parse()
                    .map_err(|_| format!("!Syntax Error, {}", tok.text))?;
                Ok(Num {
                    value,
                    scale: tok.scale,
                })
            }
            Kind::LParen => {
                let value = self.compare()?;
                self.expect_close()?;
                Ok(value)
            }
            Kind::Ident => self.identifier(&tok.text),
            _ => Err(format!("!Syntax Error, {}", tok.text)),
        }
    }

    fn identifier(&mut self, name: &str) -> Result<Num, String> {
        let upper = name.to_ascii_uppercase();
        let is_call = self.peek().is_some_and(|t| t.kind == Kind::LParen);
        if !is_call {
            return match upper.as_str() {
                "TRUE" => Ok(Num::int(1.0)),
                "FALSE" => Ok(Num::int(0.0)),
                _ => match self.env.bookmark_value(name) {
                    Some(value) => Ok(Num {
                        value,
                        scale: decimals_of(value),
                    }),
                    None => Err(format!("!Undefined Bookmark, {upper}")),
                },
            };
        }
        if !is_function(&upper) {
            return Err(format!(
                "!Syntax Error, {}",
                self.peek().map_or("", |t| t.text.as_str())
            ));
        }
        self.at += 1; // (
        let mut args: Vec<Num> = Vec::new();
        let mut referenced: Vec<f64> = Vec::new();
        let mut defined_arg: Option<String> = None;
        if self.peek().is_some_and(|t| t.kind == Kind::RParen)
            && !matches!(upper.as_str(), "TRUE" | "FALSE")
        {
            return Err("!Syntax Error, )".to_owned());
        }
        loop {
            // A table reference (ABOVE, LEFT, A1:B2) or a plain expression.
            if let Some(tok) = self.peek().cloned()
                && tok.kind == Kind::Ident
                && let Some(values) = self.table_reference(&tok.text)
            {
                self.at += 1;
                // `A1:B2` arrives as ident ":" ident; consume a range tail.
                referenced.extend(values);
            } else if upper == "DEFINED" {
                if let Some(tok) = self.peek().cloned()
                    && tok.kind == Kind::Ident
                {
                    self.at += 1;
                    defined_arg = Some(tok.text);
                } else {
                    return Err("!Syntax Error, )".to_owned());
                }
            } else {
                args.push(self.compare()?);
            }
            match self.peek() {
                Some(t) if t.kind == Kind::Comma => self.at += 1,
                _ => break,
            }
        }
        self.expect_close()?;
        apply_function(&upper, &args, &referenced, defined_arg.as_deref(), self.env)
    }

    fn table_reference(&self, name: &str) -> Option<Vec<f64>> {
        let upper = name.to_ascii_uppercase();
        let direction = matches!(upper.as_str(), "ABOVE" | "LEFT" | "BELOW" | "RIGHT");
        let is_cell = |part: &str| {
            let letters = part.chars().take_while(char::is_ascii_alphabetic).count();
            letters > 0
                && letters < part.len()
                && part[letters..].chars().all(|c| c.is_ascii_digit())
        };
        let cell = upper.split(':').all(is_cell) && !self.env.bookmark_exists(name);
        if direction || cell {
            self.env.table_values(&upper)
        } else {
            None
        }
    }
}

fn decimals_of(value: f64) -> u32 {
    let text = format!("{value}");
    text.split_once('.')
        .map_or(0, |(_, f)| u32::try_from(f.len()).unwrap_or(0))
}

fn divide(a: Num, b: Num) -> Result<Num, String> {
    if b.value == 0.0 {
        return Err("!Zero Divide".to_owned());
    }
    Ok(Num {
        value: a.value / b.value,
        scale: a.scale.max(b.scale).max(2),
    })
}

fn is_function(name: &str) -> bool {
    matches!(
        name,
        "ABS"
            | "AND"
            | "AVERAGE"
            | "COUNT"
            | "DEFINED"
            | "FALSE"
            | "IF"
            | "INT"
            | "MAX"
            | "MIN"
            | "MOD"
            | "NOT"
            | "OR"
            | "PRODUCT"
            | "ROUND"
            | "SIGN"
            | "SUM"
            | "TRUE"
    )
}

fn apply_function(
    name: &str,
    args: &[Num],
    referenced: &[f64],
    defined: Option<&str>,
    env: &dyn FormulaEnv,
) -> Result<Num, String> {
    let scale = args
        .iter()
        .map(|a| a.scale)
        .chain(referenced.iter().map(|v| decimals_of(*v)))
        .max()
        .unwrap_or(0);
    let mut values: Vec<f64> = args.iter().map(|a| a.value).collect();
    values.extend_from_slice(referenced);
    let need = |n: usize| -> Result<(), String> {
        if args.len() == n {
            Ok(())
        } else {
            Err("!Syntax Error, )".to_owned())
        }
    };
    Ok(match name {
        "ABS" => {
            need(1)?;
            Num {
                value: args[0].value.abs(),
                scale: args[0].scale,
            }
        }
        "AND" => bool_num(values.len() >= 2 && values.iter().all(|v| *v != 0.0)),
        "OR" => bool_num(values.len() >= 2 && values.iter().any(|v| *v != 0.0)),
        "NOT" => {
            need(1)?;
            bool_num(args[0].value == 0.0)
        }
        "AVERAGE" => {
            if values.is_empty() {
                return Err("!Syntax Error, )".to_owned());
            }
            let sum: f64 = values.iter().sum();
            #[allow(clippy::cast_precision_loss)]
            let count = values.len() as f64;
            Num {
                value: sum / count,
                scale: scale.max(2),
            }
        }
        "COUNT" => {
            #[allow(clippy::cast_precision_loss)]
            let count = values.len() as f64;
            Num::int(count)
        }
        "DEFINED" => bool_num(defined.is_some_and(|n| env.bookmark_exists(n))),
        "TRUE" => Num::int(1.0),
        "FALSE" => Num::int(0.0),
        "IF" => {
            need(3)?;
            if args[0].value != 0.0 {
                args[1]
            } else {
                args[2]
            }
        }
        "INT" => {
            need(1)?;
            Num::int(args[0].value.trunc())
        }
        "MAX" => Num {
            value: values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            scale,
        },
        "MIN" => Num {
            value: values.iter().copied().fold(f64::INFINITY, f64::min),
            scale,
        },
        "MOD" => {
            need(2)?;
            if args[1].value == 0.0 {
                return Err(String::new());
            }
            Num {
                value: args[0].value % args[1].value,
                scale,
            }
        }
        "PRODUCT" => Num {
            value: values.iter().product(),
            scale,
        },
        "ROUND" => {
            need(2)?;
            let places = args[1].value.trunc();
            #[allow(clippy::cast_possible_truncation)]
            let places = places as i32;
            Num {
                value: round_decimal(args[0].value, places),
                scale: u32::try_from(places.max(0)).unwrap_or(0),
            }
        }
        "SIGN" => {
            need(1)?;
            Num::int(if args[0].value > 0.0 {
                1.0
            } else if args[0].value < 0.0 {
                -1.0
            } else {
                0.0
            })
        }
        "SUM" => Num {
            value: values.iter().sum(),
            scale,
        },
        _ => return Err(format!("!Syntax Error, {name}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoEnv;
    impl FormulaEnv for NoEnv {
        fn bookmark_value(&self, _: &str) -> Option<f64> {
            None
        }
        fn bookmark_exists(&self, _: &str) -> bool {
            false
        }
        fn table_values(&self, _: &str) -> Option<Vec<f64>> {
            None
        }
    }

    fn eval(s: &str) -> String {
        match evaluate(s, &NoEnv) {
            Ok(n) => n.text(),
            Err(e) => e,
        }
    }

    #[test]
    fn scales_follow_word() {
        assert_eq!(eval("1+2"), "3");
        assert_eq!(eval("10/3"), "3.33");
        assert_eq!(eval("2^0.5"), "1.4");
        assert_eq!(eval("1/3*1000"), "333.33");
        assert_eq!(eval("1.5*1.5"), "2.3");
        assert_eq!(eval("0.1^2"), "0.0");
        assert_eq!(eval("12.5%"), "0.125");
    }

    #[test]
    fn precedence_and_errors() {
        assert_eq!(eval("-2^2"), "4");
        assert_eq!(eval("2^3^2"), "512");
        assert_eq!(eval("1/0"), "!Zero Divide");
        assert_eq!(eval("2*(3"), "!Unexpected End of Formula");
        assert_eq!(eval("5 5"), "!Missing Operator");
    }
}
