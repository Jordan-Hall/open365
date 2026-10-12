//! Guide formulas: `op arg arg arg`, evaluated in definition order.
//!
//! Semantics follow the prose of the DrawingML guide-formula table: angles are 60000ths of
//! a degree, `*/` is `x*y/z`, `+-` is `x+y-z`, `+/` is `(x+y)/z`, `?:` is `x>0 ? y : z`,
//! and so on. Choices the prose leaves open (division by zero gives 0, `at2` is
//! normalised to `0..21 600 000`) are marked where they are made and are to be confirmed
//! against the Office oracle.

use std::collections::HashMap;

/// Angle units per degree.
pub const ANG: f64 = 60_000.0;
/// A full turn in angle units.
pub const FULL_TURN: f64 = 21_600_000.0;

/// A formula argument: a number or the name of a guide, adjust value or built-in.
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Lit(f64),
    Name(String),
}

impl Operand {
    /// Reads a literal when the text is numeric, otherwise a name.
    pub fn parse(s: &str) -> Self {
        let s = s.trim();
        match s.parse::<f64>() {
            Ok(v) if v.is_finite() => Self::Lit(v),
            _ => Self::Name(s.to_owned()),
        }
    }
}

/// Formula operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    MulDiv,
    AddSub,
    AddDiv,
    If,
    Abs,
    At2,
    Cat2,
    Cos,
    Max,
    Min,
    Mod,
    Pin,
    Sat2,
    Sin,
    Sqrt,
    Tan,
    Val,
}

impl Op {
    fn parse(s: &str) -> Option<(Self, usize)> {
        Some(match s {
            "*/" => (Self::MulDiv, 3),
            "+-" => (Self::AddSub, 3),
            "+/" => (Self::AddDiv, 3),
            "?:" => (Self::If, 3),
            "abs" => (Self::Abs, 1),
            "at2" => (Self::At2, 2),
            "cat2" => (Self::Cat2, 3),
            "cos" => (Self::Cos, 2),
            "max" => (Self::Max, 2),
            "min" => (Self::Min, 2),
            "mod" => (Self::Mod, 3),
            "pin" => (Self::Pin, 3),
            "sat2" => (Self::Sat2, 3),
            "sin" => (Self::Sin, 2),
            "sqrt" => (Self::Sqrt, 1),
            "tan" => (Self::Tan, 2),
            "val" => (Self::Val, 1),
            _ => return None,
        })
    }
}

/// A parsed formula.
#[derive(Debug, Clone, PartialEq)]
pub struct Formula {
    pub op: Op,
    pub args: Vec<Operand>,
}

impl Formula {
    /// Parses `"*/ w adj 100000"`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut it = s.split_whitespace();
        let name = it.next().ok_or("empty formula")?;
        let (op, n) = Op::parse(name).ok_or_else(|| format!("unknown operator `{name}`"))?;
        let args: Vec<Operand> = it.map(Operand::parse).collect();
        if args.len() != n {
            return Err(format!(
                "operator `{name}` takes {n} arguments, found {}",
                args.len()
            ));
        }
        Ok(Self { op, args })
    }

    /// Evaluates against `scope`.
    pub fn eval(&self, scope: &Scope) -> Result<f64, String> {
        let mut v = [0.0f64; 3];
        for (i, a) in self.args.iter().enumerate() {
            v[i] = scope.operand(a)?;
        }
        let [x, y, z] = v;
        let rad = |a: f64| (a / ANG).to_radians();
        let r = match self.op {
            Op::MulDiv => div(x * y, z),
            Op::AddSub => x + y - z,
            Op::AddDiv => div(x + y, z),
            Op::If => {
                if x > 0.0 {
                    y
                } else {
                    z
                }
            }
            Op::Abs => x.abs(),
            Op::At2 => atan2_units(y, x),
            Op::Cat2 => x * (z.atan2(y)).cos(),
            Op::Sat2 => x * (z.atan2(y)).sin(),
            Op::Cos => x * rad(y).cos(),
            Op::Sin => x * rad(y).sin(),
            Op::Tan => x * rad(y).tan(),
            Op::Max => x.max(y),
            Op::Min => x.min(y),
            Op::Mod => (x * x + y * y + z * z).sqrt(),
            Op::Pin => {
                if y < x {
                    x
                } else if y > z {
                    z
                } else {
                    y
                }
            }
            Op::Sqrt => {
                if x > 0.0 {
                    x.sqrt()
                } else {
                    0.0
                }
            }
            Op::Val => x,
        };
        if r.is_finite() {
            Ok(r)
        } else {
            Err("non-finite result".into())
        }
    }
}

/// Division where a zero divisor gives 0 (to confirm against the oracle).
fn div(n: f64, d: f64) -> f64 {
    if d == 0.0 { 0.0 } else { n / d }
}

/// `atan2(y, x)` in angle units, normalised to `0..FULL_TURN` (to confirm against the oracle).
fn atan2_units(y: f64, x: f64) -> f64 {
    let a = (y.atan2(x).to_degrees() * ANG).rem_euclid(FULL_TURN);
    if a >= FULL_TURN { 0.0 } else { a }
}

/// Named values visible to formulas: built-ins from the shape size, adjust values, guides.
#[derive(Debug, Clone)]
pub struct Scope {
    vars: HashMap<String, f64>,
}

impl Scope {
    /// A scope holding only the built-in guides for a `w` by `h` box.
    pub fn new(w: f64, h: f64) -> Self {
        let mut vars = HashMap::new();
        let mut put = |k: &str, v: f64| {
            vars.insert(k.to_owned(), v);
        };
        put("w", w);
        put("h", h);
        put("l", 0.0);
        put("t", 0.0);
        put("r", w);
        put("b", h);
        put("hc", w / 2.0);
        put("vc", h / 2.0);
        for d in [2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0, 32.0] {
            put(&format!("wd{d}"), w / d);
            put(&format!("hd{d}"), h / d);
        }
        let ss = w.min(h);
        put("ss", ss);
        put("ls", w.max(h));
        for d in [2.0, 4.0, 6.0, 8.0, 16.0, 32.0] {
            put(&format!("ssd{d}"), ss / d);
        }
        put("cd2", FULL_TURN / 2.0);
        put("cd4", FULL_TURN / 4.0);
        put("cd8", FULL_TURN / 8.0);
        put("3cd4", FULL_TURN * 0.75);
        put("3cd8", FULL_TURN * 3.0 / 8.0);
        put("5cd8", FULL_TURN * 5.0 / 8.0);
        put("7cd8", FULL_TURN * 7.0 / 8.0);
        Self { vars }
    }

    /// Defines or replaces a name.
    pub fn set(&mut self, name: &str, v: f64) {
        self.vars.insert(name.to_owned(), v);
    }

    /// Looks a name up.
    pub fn get(&self, name: &str) -> Option<f64> {
        self.vars.get(name).copied()
    }

    /// Value of an operand.
    pub fn operand(&self, o: &Operand) -> Result<f64, String> {
        match o {
            Operand::Lit(v) => Ok(*v),
            Operand::Name(n) => self.get(n).ok_or_else(|| format!("unresolved name `{n}`")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str) -> f64 {
        let mut sc = Scope::new(1000.0, 500.0);
        sc.set("a", 25000.0);
        Formula::parse(s).unwrap().eval(&sc).unwrap()
    }

    #[test]
    fn arithmetic_operators() {
        assert_eq!(ev("*/ w a 100000"), 250.0);
        assert_eq!(ev("+- w h 100"), 1400.0);
        assert_eq!(ev("+/ w h 3"), 500.0);
        assert_eq!(ev("?: 1 7 9"), 7.0);
        assert_eq!(ev("?: 0 7 9"), 9.0);
        assert_eq!(ev("?: -3 7 9"), 9.0);
        assert_eq!(ev("abs -5"), 5.0);
        assert_eq!(ev("max 3 8"), 8.0);
        assert_eq!(ev("min 3 8"), 3.0);
        assert_eq!(ev("mod 3 4 12"), 13.0);
        assert_eq!(ev("sqrt 49"), 7.0);
        assert_eq!(ev("sqrt -4"), 0.0);
        assert_eq!(ev("val 12"), 12.0);
    }

    #[test]
    fn pin_clamps_middle_argument() {
        assert_eq!(ev("pin 0 -5 10"), 0.0);
        assert_eq!(ev("pin 0 5 10"), 5.0);
        assert_eq!(ev("pin 0 50 10"), 10.0);
    }

    #[test]
    fn trig_in_sixty_thousandths_of_a_degree() {
        assert!((ev("cos 100 0") - 100.0).abs() < 1e-9);
        assert!((ev("cos 100 10800000") + 100.0).abs() < 1e-9);
        assert!((ev("sin 100 5400000") - 100.0).abs() < 1e-9);
        assert!((ev("tan 100 2700000") - 100.0).abs() < 1e-9);
        assert!((ev("at2 1 1") - 2_700_000.0).abs() < 1e-6);
        assert!((ev("at2 0 1") - 5_400_000.0).abs() < 1e-6);
        assert!((ev("at2 -1 0") - 10_800_000.0).abs() < 1e-6);
        assert!((ev("at2 0 -1") - 16_200_000.0).abs() < 1e-6);
        assert!((ev("cat2 10 3 4") - 6.0).abs() < 1e-9);
        assert!((ev("sat2 10 3 4") - 8.0).abs() < 1e-9);
    }

    #[test]
    fn zero_divisor_is_zero() {
        assert_eq!(ev("*/ w a 0"), 0.0);
        assert_eq!(ev("+/ w a 0"), 0.0);
    }

    #[test]
    fn builtins() {
        let sc = Scope::new(1200.0, 600.0);
        for (n, v) in [
            ("w", 1200.0),
            ("h", 600.0),
            ("l", 0.0),
            ("t", 0.0),
            ("r", 1200.0),
            ("b", 600.0),
            ("hc", 600.0),
            ("vc", 300.0),
            ("wd2", 600.0),
            ("wd3", 400.0),
            ("wd4", 300.0),
            ("wd5", 240.0),
            ("wd6", 200.0),
            ("wd8", 150.0),
            ("wd10", 120.0),
            ("wd12", 100.0),
            ("wd32", 37.5),
            ("hd2", 300.0),
            ("hd3", 200.0),
            ("hd4", 150.0),
            ("hd5", 120.0),
            ("hd6", 100.0),
            ("hd8", 75.0),
            ("ss", 600.0),
            ("ls", 1200.0),
            ("ssd2", 300.0),
            ("ssd4", 150.0),
            ("ssd6", 100.0),
            ("ssd8", 75.0),
            ("ssd16", 37.5),
            ("ssd32", 18.75),
            ("cd2", 10_800_000.0),
            ("cd4", 5_400_000.0),
            ("cd8", 2_700_000.0),
            ("3cd4", 16_200_000.0),
            ("3cd8", 8_100_000.0),
            ("5cd8", 13_500_000.0),
            ("7cd8", 18_900_000.0),
        ] {
            assert_eq!(sc.get(n), Some(v), "{n}");
        }
    }

    #[test]
    fn parse_errors() {
        assert!(Formula::parse("").is_err());
        assert!(Formula::parse("nope 1 2").is_err());
        assert!(Formula::parse("*/ 1 2").is_err());
        let sc = Scope::new(1.0, 1.0);
        assert!(Formula::parse("val zzz").unwrap().eval(&sc).is_err());
    }
}
