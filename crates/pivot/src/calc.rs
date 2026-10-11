//! Formulas of calculated fields (`cacheField/@formula`) and calculated items
//! (`calculatedItem/@formula`): a small evaluator for what Excel allows in them (constants,
//! field and item names, operators and the worksheet functions that take no cell references).
//!
//! A calculated field is evaluated over the *sums* of the fields it names, per output cell
//! (so `Revenue-Units*Price` at a total is the total revenue minus total units times the sum
//! of prices), which is what Excel does.

/// A parsed formula.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f64),
    Text(String),
    Bool(bool),
    /// A field name (or item name inside a calculated item).
    Name(String),
    /// `Field[item]` inside a calculated item formula.
    Item(String, String),
    Neg(Box<Expr>),
    Percent(Box<Expr>),
    Bin(char, Box<Expr>, Box<Expr>),
    /// `<=`, `>=`, `<>` use the codes `l`, `g`, `n`.
    Cmp(char, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

/// An evaluated value.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Num(f64),
    Text(String),
    Bool(bool),
    Err(&'static str),
}

impl Val {
    fn num(&self) -> Result<f64, &'static str> {
        match self {
            Self::Num(n) => Ok(*n),
            Self::Bool(b) => Ok(f64::from(u8::from(*b))),
            Self::Text(t) => t.trim().parse().map_err(|_| "#VALUE!"),
            Self::Err(e) => Err(e),
        }
    }
}

struct Parser<'a> {
    s: &'a [char],
    i: usize,
}

/// Parses a formula (a leading `=` is allowed).
pub fn parse(src: &str) -> Option<Expr> {
    let chars: Vec<char> = src.trim().trim_start_matches('=').chars().collect();
    let mut p = Parser { s: &chars, i: 0 };
    let e = p.compare()?;
    p.ws();
    (p.i == p.s.len()).then_some(e)
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(|c| c.is_whitespace()) {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.ws();
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn compare(&mut self) -> Option<Expr> {
        let l = self.concat()?;
        let Some(first) = self.peek() else {
            return Some(l);
        };
        let op = match (first, self.s.get(self.i + 1).copied()) {
            ('<', Some('=')) => {
                self.i += 2;
                'l'
            }
            ('>', Some('=')) => {
                self.i += 2;
                'g'
            }
            ('<', Some('>')) => {
                self.i += 2;
                'n'
            }
            (c @ ('<' | '>' | '='), _) => {
                self.i += 1;
                c
            }
            _ => return Some(l),
        };
        let r = self.concat()?;
        Some(Expr::Cmp(op, Box::new(l), Box::new(r)))
    }

    fn concat(&mut self) -> Option<Expr> {
        let mut l = self.sum()?;
        while self.eat('&') {
            l = Expr::Bin('&', Box::new(l), Box::new(self.sum()?));
        }
        Some(l)
    }

    fn sum(&mut self) -> Option<Expr> {
        let mut l = self.product()?;
        loop {
            let op = match self.peek() {
                Some(c @ ('+' | '-')) => c,
                _ => return Some(l),
            };
            self.i += 1;
            l = Expr::Bin(op, Box::new(l), Box::new(self.product()?));
        }
    }

    fn product(&mut self) -> Option<Expr> {
        let mut l = self.power()?;
        loop {
            let op = match self.peek() {
                Some(c @ ('*' | '/')) => c,
                _ => return Some(l),
            };
            self.i += 1;
            l = Expr::Bin(op, Box::new(l), Box::new(self.power()?));
        }
    }

    fn power(&mut self) -> Option<Expr> {
        let mut l = self.unary()?;
        while self.eat('^') {
            l = Expr::Bin('^', Box::new(l), Box::new(self.unary()?));
        }
        Some(l)
    }

    fn unary(&mut self) -> Option<Expr> {
        if self.eat('-') {
            return Some(Expr::Neg(Box::new(self.unary()?)));
        }
        if self.eat('+') {
            return self.unary();
        }
        let mut e = self.atom()?;
        while self.eat('%') {
            e = Expr::Percent(Box::new(e));
        }
        Some(e)
    }

    fn quoted(&mut self, q: char) -> Option<String> {
        let mut out = String::new();
        loop {
            let c = *self.s.get(self.i)?;
            self.i += 1;
            if c == q {
                if self.s.get(self.i) == Some(&q) {
                    out.push(q);
                    self.i += 1;
                } else {
                    return Some(out);
                }
            } else {
                out.push(c);
            }
        }
    }

    fn name(&mut self) -> Option<String> {
        if self.eat('\'') {
            return self.quoted('\'');
        }
        let start = self.i;
        while self
            .s
            .get(self.i)
            .is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '.'))
        {
            self.i += 1;
        }
        (self.i > start).then(|| self.s[start..self.i].iter().collect())
    }

    fn atom(&mut self) -> Option<Expr> {
        let c = self.peek()?;
        if c == '(' {
            self.i += 1;
            let e = self.compare()?;
            return self.eat(')').then_some(e);
        }
        if c == '"' {
            self.i += 1;
            return self.quoted('"').map(Expr::Text);
        }
        if c.is_ascii_digit() || c == '.' {
            let start = self.i;
            while self.s.get(self.i).is_some_and(|c| {
                c.is_ascii_digit()
                    || *c == '.'
                    || ((*c == 'e' || *c == 'E') && self.i > start)
                    || ((*c == '+' || *c == '-') && matches!(self.s[self.i - 1], 'e' | 'E'))
            }) {
                self.i += 1;
            }
            let t: String = self.s[start..self.i].iter().collect();
            return t.parse().ok().map(Expr::Num);
        }
        let name = self.name()?;
        if self.s.get(self.i) == Some(&'(') {
            self.i += 1;
            let mut args = Vec::new();
            if !self.eat(')') {
                loop {
                    args.push(self.compare()?);
                    if self.eat(')') {
                        break;
                    }
                    if !self.eat(',') {
                        return None;
                    }
                }
            }
            return Some(Expr::Call(name.to_ascii_uppercase(), args));
        }
        if self.s.get(self.i) == Some(&'[') {
            self.i += 1;
            let item = if self.eat('\'') {
                self.quoted('\'')?
            } else {
                let start = self.i;
                while self.s.get(self.i).is_some_and(|c| *c != ']') {
                    self.i += 1;
                }
                self.s[start..self.i].iter().collect()
            };
            return self.eat(']').then_some(Expr::Item(name, item));
        }
        match name.to_ascii_uppercase().as_str() {
            "TRUE" => Some(Expr::Bool(true)),
            "FALSE" => Some(Expr::Bool(false)),
            _ => Some(Expr::Name(name)),
        }
    }
}

impl Expr {
    /// Field (or item) names the formula refers to, in order of appearance.
    pub fn names(&self, out: &mut Vec<String>) {
        match self {
            Self::Name(n) => out.push(n.clone()),
            Self::Neg(e) | Self::Percent(e) => e.names(out),
            Self::Bin(_, a, b) | Self::Cmp(_, a, b) => {
                a.names(out);
                b.names(out);
            }
            Self::Call(_, args) => args.iter().for_each(|a| a.names(out)),
            _ => {}
        }
    }
}

/// Evaluates `e`; `lookup` gives the value of a name.
pub fn eval(e: &Expr, lookup: &dyn Fn(&str) -> Val) -> Val {
    match e {
        Expr::Num(n) => Val::Num(*n),
        Expr::Text(t) => Val::Text(t.clone()),
        Expr::Bool(b) => Val::Bool(*b),
        Expr::Name(n) => lookup(n),
        Expr::Item(f, i) => lookup(&format!("{f}[{i}]")),
        Expr::Neg(a) => match eval(a, lookup).num() {
            Ok(n) => Val::Num(-n),
            Err(e) => Val::Err(e),
        },
        Expr::Percent(a) => match eval(a, lookup).num() {
            Ok(n) => Val::Num(n / 100.0),
            Err(e) => Val::Err(e),
        },
        Expr::Bin('&', a, b) => {
            let text = |v: Val| match v {
                Val::Text(t) => Ok(t),
                Val::Num(n) => {
                    let mut s = String::new();
                    open365_xlsx::format_number(n, &mut s);
                    Ok(s)
                }
                Val::Bool(b) => Ok(if b { "TRUE" } else { "FALSE" }.to_owned()),
                Val::Err(e) => Err(e),
            };
            match (text(eval(a, lookup)), text(eval(b, lookup))) {
                (Ok(x), Ok(y)) => Val::Text(x + &y),
                (Err(e), _) | (_, Err(e)) => Val::Err(e),
            }
        }
        Expr::Bin(op, a, b) => {
            let (x, y) = match (eval(a, lookup).num(), eval(b, lookup).num()) {
                (Ok(x), Ok(y)) => (x, y),
                (Err(e), _) | (_, Err(e)) => return Val::Err(e),
            };
            match op {
                '+' => Val::Num(x + y),
                '-' => Val::Num(x - y),
                '*' => Val::Num(x * y),
                '/' if y == 0.0 => Val::Err("#DIV/0!"),
                '/' => Val::Num(x / y),
                _ => {
                    let r = x.powf(y);
                    if r.is_finite() {
                        Val::Num(r)
                    } else {
                        Val::Err("#NUM!")
                    }
                }
            }
        }
        Expr::Cmp(op, a, b) => {
            let (x, y) = (eval(a, lookup), eval(b, lookup));
            if let Val::Err(e) = x {
                return Val::Err(e);
            }
            if let Val::Err(e) = y {
                return Val::Err(e);
            }
            let ord = match (&x, &y) {
                (Val::Text(p), Val::Text(q)) => p.to_lowercase().cmp(&q.to_lowercase()),
                (Val::Text(_), _) => std::cmp::Ordering::Greater,
                (_, Val::Text(_)) => std::cmp::Ordering::Less,
                _ => x
                    .num()
                    .unwrap_or(0.0)
                    .partial_cmp(&y.num().unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal),
            };
            use std::cmp::Ordering::{Equal, Greater, Less};
            Val::Bool(match op {
                '<' => ord == Less,
                '>' => ord == Greater,
                '=' => ord == Equal,
                'l' => ord != Greater,
                'g' => ord != Less,
                _ => ord != Equal,
            })
        }
        Expr::Call(name, args) => call(name, args, lookup),
    }
}

fn truthy(v: &Val) -> Result<bool, &'static str> {
    match v {
        Val::Bool(b) => Ok(*b),
        Val::Num(n) => Ok(*n != 0.0),
        Val::Text(_) => Err("#VALUE!"),
        Val::Err(e) => Err(e),
    }
}

fn call(name: &str, args: &[Expr], lookup: &dyn Fn(&str) -> Val) -> Val {
    let vals: Vec<Val> = args.iter().map(|a| eval(a, lookup)).collect();
    if name == "IF" {
        let Some(c) = vals.first() else {
            return Val::Err("#VALUE!");
        };
        return match truthy(c) {
            Ok(true) => vals.get(1).cloned().unwrap_or(Val::Bool(true)),
            Ok(false) => vals.get(2).cloned().unwrap_or(Val::Bool(false)),
            Err(e) => Val::Err(e),
        };
    }
    if name == "IFERROR" {
        return match vals.first() {
            Some(Val::Err(_)) | None => vals.get(1).cloned().unwrap_or(Val::Num(0.0)),
            Some(v) => v.clone(),
        };
    }
    if matches!(name, "AND" | "OR" | "NOT") {
        let mut bs = Vec::new();
        for v in &vals {
            match truthy(v) {
                Ok(b) => bs.push(b),
                Err(e) => return Val::Err(e),
            }
        }
        return match name {
            "AND" => Val::Bool(bs.iter().all(|b| *b)),
            "OR" => Val::Bool(bs.iter().any(|b| *b)),
            _ => Val::Bool(bs.first().is_some_and(|b| !*b)),
        };
    }
    let mut nums = Vec::new();
    for v in &vals {
        match v.num() {
            Ok(n) => nums.push(n),
            Err(e) => return Val::Err(e),
        }
    }
    let n0 = nums.first().copied().unwrap_or(0.0);
    let n1 = nums.get(1).copied().unwrap_or(0.0);
    let round = |x: f64, digits: f64, mode: u8| -> f64 {
        let k = 10f64.powi(digits as i32);
        let y = x * k;
        let y = match mode {
            0 => y.abs().round().copysign(y),
            1 => y.abs().ceil().copysign(y),
            _ => y.abs().floor().copysign(y),
        };
        y / k
    };
    match name {
        "ABS" => Val::Num(n0.abs()),
        "SQRT" if n0 < 0.0 => Val::Err("#NUM!"),
        "SQRT" => Val::Num(n0.sqrt()),
        "INT" => Val::Num(n0.floor()),
        "ROUND" => Val::Num(round(n0, n1, 0)),
        "ROUNDUP" => Val::Num(round(n0, n1, 1)),
        "ROUNDDOWN" => Val::Num(round(n0, n1, 2)),
        "POWER" => Val::Num(n0.powf(n1)),
        "MOD" if n1 == 0.0 => Val::Err("#DIV/0!"),
        "MOD" => Val::Num(n0 - n1 * (n0 / n1).floor()),
        "SUM" => Val::Num(nums.iter().sum()),
        "MAX" => Val::Num(nums.iter().copied().fold(f64::MIN, f64::max)),
        "MIN" => Val::Num(nums.iter().copied().fold(f64::MAX, f64::min)),
        "AVERAGE" if nums.is_empty() => Val::Err("#DIV/0!"),
        "AVERAGE" => Val::Num(nums.iter().sum::<f64>() / nums.len() as f64),
        _ => Val::Err("#NAME?"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str, vars: &[(&str, f64)]) -> Val {
        let e = parse(src).expect("parses");
        eval(&e, &|n| {
            vars.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(n))
                .map_or(Val::Err("#NAME?"), |(_, v)| Val::Num(*v))
        })
    }

    #[test]
    fn arithmetic_and_precedence() {
        let v = [("Revenue", 100.0), ("Units", 4.0), ("Per unit", 3.0)];
        assert_eq!(run("=Revenue-Units*3", &v), Val::Num(88.0));
        assert_eq!(run("'Per unit'^2/Units", &v), Val::Num(2.25));
        assert_eq!(run("-2^2", &v), Val::Num(4.0));
        assert_eq!(run("Revenue/(Units-4)", &v), Val::Err("#DIV/0!"));
        assert_eq!(run("IF(Units>3,Revenue,0)+50%", &v), Val::Num(100.5));
        assert_eq!(run("ROUND(Revenue/3,1)", &v), Val::Num(33.3));
    }
}
