//! Hypothesis tests (`Z.TEST`, `T.TEST`, `F.TEST`, `CHISQ.TEST`), confidence intervals and a few small helpers
//! (`FISHER`, `FISHERINV`, `PERMUTATIONA`).

use super::distrib::{NUM, nums, out};
use super::distribdd::{self as dd, Dd};
use super::stats::variance_of;
use super::{Res, arity, num_arg, number, optional};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

pub(super) fn takes_arrays(name: &str, index: usize) -> bool {
    match name.to_ascii_uppercase().as_str() {
        // Z.TEST and PERCENTRANK take a range but, unlike the others, do not evaluate an expression over it as an
        // array: `Z.TEST(A1:A5*2,4)` is #VALUE! in Excel.
        "T.TEST" | "TTEST" | "F.TEST" | "FTEST" | "CHISQ.TEST" | "CHITEST" => index <= 1,
        "PROB" => index <= 2,
        "FREQUENCY" | "MODE.MULT" => true,
        "LINEST" | "LOGEST" => index <= 1,
        "TREND" | "GROWTH" => index <= 2,
        _ => false,
    }
}

/// The numbers of an array or reference: text, booleans and blanks are skipped, an error propagates.
fn numbers(ctx: &EvalContext<'_>, arg: &Arg) -> Res<Vec<f64>> {
    let mut v = Vec::new();
    for x in ctx.cells(arg) {
        match x {
            Value::Number(n) => v.push(n),
            Value::Error(e) => return Err(e),
            _ => {}
        }
    }
    Ok(v)
}

fn mean(x: &[f64]) -> f64 {
    let mut s = 0.0;
    for a in x {
        s += a;
    }
    s / x.len() as f64
}

fn var(x: &[f64]) -> Res<f64> {
    match variance_of(x, true, false)? {
        Value::Number(v) => Ok(v),
        _ => Err(ErrorKind::Num),
    }
}

pub(super) fn z_test(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 3)?;
    let x = numbers(ctx, &args[0])?;
    let mu = num_arg(ctx, &args[1])?;
    let sigma = match optional(args, 2) {
        Some(a) => Some(num_arg(ctx, a)?),
        None => None,
    };
    if x.is_empty() {
        return Err(ErrorKind::NA);
    }
    // One value cannot give a z statistic, even with a known sigma (`Z.TEST({5},2,1)` is #DIV/0!).
    if x.len() < 2 {
        return Err(ErrorKind::DivZero);
    }
    let n = x.len() as f64;
    let sigma = match sigma {
        Some(s) if s <= 0.0 => return Err(NUM),
        Some(s) => s,
        None => {
            let sd = var(&x)?.sqrt();
            if sd == 0.0 {
                return Err(ErrorKind::DivZero);
            }
            sd
        }
    };
    let z = (mean(&x) - mu) / (sigma / n.sqrt());
    number(dd::norm_cdf(Dd::from(z)).1.to_f64())
}

/// Student's t two-tail probability for a real `df`.
fn t_two_tail(t: f64, df: f64) -> Res<(f64, f64)> {
    let nu = Dd::from(df);
    let t2 = dd::prod(t, t);
    let den = nu.add(t2);
    let (two, _) =
        dd::beta_pq(nu.scale(0.5), Dd::from(0.5), nu.div(den), t2.div(den)).ok_or(NUM)?;
    Ok((two.to_f64(), two.scale(0.5).to_f64()))
}

pub(super) fn t_test(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 4, 4)?;
    let tails = num_arg(ctx, &args[2])?.trunc();
    let kind = num_arg(ctx, &args[3])?.trunc();
    let (a_cells, b_cells): (Vec<Value>, Vec<Value>) =
        (ctx.cells(&args[0]).collect(), ctx.cells(&args[1]).collect());
    for v in a_cells.iter().chain(&b_cells) {
        if let Value::Error(e) = v {
            return Err(*e);
        }
    }
    if !(tails == 1.0 || tails == 2.0) || !(1.0..=3.0).contains(&kind) {
        return Err(NUM);
    }
    let (t, df) = if kind == 1.0 {
        if a_cells.len() != b_cells.len() {
            return Err(ErrorKind::NA);
        }
        let d: Vec<f64> = a_cells
            .iter()
            .zip(&b_cells)
            .filter_map(|(p, q)| match (p, q) {
                (Value::Number(p), Value::Number(q)) => Some(p - q),
                _ => None,
            })
            .collect();
        if d.len() < 2 {
            return Err(ErrorKind::DivZero);
        }
        let n = d.len() as f64;
        let v = var(&d)?;
        if v == 0.0 {
            return Err(ErrorKind::DivZero);
        }
        (mean(&d) / (v / n).sqrt(), n - 1.0)
    } else {
        let a = numbers(ctx, &args[0])?;
        let b = numbers(ctx, &args[1])?;
        if a.len() < 2 || b.len() < 2 {
            return Err(ErrorKind::DivZero);
        }
        let (n1, n2) = (a.len() as f64, b.len() as f64);
        let (v1, v2) = (var(&a)?, var(&b)?);
        let dm = mean(&a) - mean(&b);
        if kind == 2.0 {
            let sp = ((n1 - 1.0) * v1 + (n2 - 1.0) * v2) / (n1 + n2 - 2.0);
            let se = (sp * (1.0 / n1 + 1.0 / n2)).sqrt();
            if se == 0.0 {
                return Err(ErrorKind::DivZero);
            }
            (dm / se, n1 + n2 - 2.0)
        } else {
            let (s1, s2) = (v1 / n1, v2 / n2);
            let se = (s1 + s2).sqrt();
            if se == 0.0 {
                return Err(ErrorKind::DivZero);
            }
            let df = (s1 + s2) * (s1 + s2) / (s1 * s1 / (n1 - 1.0) + s2 * s2 / (n2 - 1.0));
            (dm / se, df)
        }
    };
    let (two, one) = t_two_tail(t.abs(), df)?;
    number(if tails == 1.0 { one } else { two })
}

pub(super) fn f_test(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let a = numbers(ctx, &args[0])?;
    let b = numbers(ctx, &args[1])?;
    if a.len() < 2 || b.len() < 2 {
        return Err(ErrorKind::DivZero);
    }
    let (v1, v2) = (var(&a)?, var(&b)?);
    if v1 == 0.0 || v2 == 0.0 {
        return Err(ErrorKind::DivZero);
    }
    let (d1, d2) = ((a.len() - 1) as f64, (b.len() - 1) as f64);
    let f = v1 / v2;
    let n = dd::prod(d1, f);
    let den = n.add_f(d2);
    let (lo, hi) = dd::beta_pq(
        Dd::from(d1).scale(0.5),
        Dd::from(d2).scale(0.5),
        n.div(den),
        Dd::from(d2).div(den),
    )
    .ok_or(NUM)?;
    let p = if lo.lt(hi) { lo } else { hi }.scale(2.0);
    number(p.to_f64().min(1.0))
}

pub(super) fn chisq_test(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 2)?;
    let (Some(ga), Some(gb)) = (ctx.grid(&args[0]), ctx.grid(&args[1])) else {
        return Err(ErrorKind::Value);
    };
    let (rows, cols) = (ga.rows(), ga.cols());
    // Only the number of cells has to agree; the two arrays are paired in row-major order
    // (`CHISQ.TEST({1,2,3},{1;2;3})` is 1) and the degrees of freedom come from the shape of the first.
    if rows * cols != gb.rows() * gb.cols() {
        return Err(ErrorKind::NA);
    }
    let pairs = (0..rows * cols).map(|i| {
        (
            ga.get(i / cols, i % cols),
            gb.get(i / gb.cols(), i % gb.cols()),
        )
    });
    let mut chi = 0.0;
    let mut count = 0;
    for (a, e) in pairs {
        if let Value::Error(x) = a {
            return Err(x);
        }
        if let Value::Error(x) = e {
            return Err(x);
        }
        if let (Value::Number(a), Value::Number(e)) = (a, e) {
            if e == 0.0 {
                return Err(ErrorKind::DivZero);
            }
            if e < 0.0 {
                return Err(NUM);
            }
            chi += (a - e) * (a - e) / e;
            count += 1;
        }
    }
    let df = if rows == 1 {
        cols as f64 - 1.0
    } else if cols == 1 {
        rows as f64 - 1.0
    } else {
        (rows as f64 - 1.0) * (cols as f64 - 1.0)
    };
    if df < 1.0 {
        return Err(ErrorKind::NA);
    }
    if count == 0 {
        return Err(ErrorKind::DivZero);
    }
    if !chi.is_finite() {
        return Err(NUM);
    }
    let (_, q) = dd::gamma_pq(Dd::from(df / 2.0), Dd::from(chi / 2.0)).ok_or(NUM)?;
    number(q.to_f64())
}

// ---- confidence ----------------------------------------------------------------------------------

pub(super) fn confidence_norm(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [alpha, sd, n] = nums::<3>(ctx, args)?;
    let n = n.trunc();
    if !(alpha > 0.0 && alpha < 1.0) || sd <= 0.0 || n < 1.0 {
        return Err(NUM);
    }
    let z = super::distrib::norm_s_inv_f(1.0 - alpha / 2.0)?;
    out(z * (sd / n.sqrt()))
}

pub(super) fn confidence_t(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [alpha, sd, n] = nums::<3>(ctx, args)?;
    let n = n.trunc();
    if !(alpha > 0.0 && alpha < 1.0) || sd <= 0.0 || n < 1.0 {
        return Err(NUM);
    }
    if n == 1.0 {
        return Err(ErrorKind::DivZero);
    }
    let t = super::distrib::t_inv_2t_f(alpha, n - 1.0)?;
    out(t * (sd / n.sqrt()))
}

// ---- small helpers -------------------------------------------------------------------------------

pub(super) fn fisher(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [x] = nums::<1>(ctx, args)?;
    if !(x > -1.0 && x < 1.0) {
        return Err(NUM);
    }
    let r = dd::sum(1.0, x).div(dd::sum(1.0, -x));
    number(dd::ln_p(r).scale(0.5).to_f64())
}

pub(super) fn fisherinv(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [y] = nums::<1>(ctx, args)?;
    if y.abs() > 350.0 {
        return out(y.signum());
    }
    let e = dd::exp(Dd::from(y).scale(2.0));
    number(e.sub_f(1.0).div(e.add_f(1.0)).to_f64())
}

pub(super) fn permutationa(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    let [n, k] = nums::<2>(ctx, args)?;
    let (n, k) = (n.trunc(), k.trunc());
    if n < 0.0 || k < 0.0 {
        return Err(NUM);
    }
    if n == 0.0 {
        return out(if k == 0.0 { 1.0 } else { 0.0 });
    }
    // n^k by repeated squaring in double-double
    let mut result = Dd::from(1.0);
    let mut base = Dd::from(n);
    let mut e = k as u64;
    while e > 0 {
        if e & 1 == 1 {
            result = result.mul(base);
        }
        e >>= 1;
        if e > 0 {
            base = base.square();
            if !base.hi.is_finite() {
                return Err(NUM);
            }
        }
    }
    number(result.to_f64())
}
