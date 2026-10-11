//! `FORECAST.ETS`, `FORECAST.ETS.CONFINT`, `FORECAST.ETS.SEASONALITY` and `FORECAST.ETS.STAT`.
//!
//! The model is additive error, additive trend, additive seasonality ("AAA"). Findings that were measured on Excel
//! 16.0 through `FORECAST.ETS.STAT` (see `docs/testing/calc-coverage.md`):
//!
//! * the recurrence is the error-correction form: `e = y - (l + b + s)`, `l += b + alpha*e`, `b += beta*e`,
//!   `s += gamma*e`, and the first one-step forecast equals the first observation (`l0 + b0 = y1`);
//! * the initial trend `b0` is a linear function of the observations that does not depend on the parameters: the
//!   least-squares slope for 3 and for 7 or more points, a fixed weight table for 4 to 6 points;
//! * the parameters are not the continuous optimum but lie on a lattice (`alpha` 0.002, 0.1, 0.126, 0.25, 0.251, 0.5,
//!   0.9 ..., `beta` 0.001 or 0.099, `gamma` 0.001, 0.125, 0.25, 0.749 or 0 up to 2.2E-16); this port minimises the
//!   squared one-step error over that lattice, which finds the same point for about a quarter of the series;
//! * MASE, SMAPE, MAE and RMSE are means over all n in-sample errors (the first is 0), MASE against the naive
//!   one-step error, the confidence interval at step 1 is `z * RMSE`.
//!
//! The seasonal start values, the seasonality detection and the interval for longer horizons are fitted models, not
//! reproductions (listed as known differences).

use std::cmp::Ordering;

use super::distrib::norm_s_inv_f;
use super::{Res, arity, int_arg, num_arg, number, optional};
use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

pub(super) fn takes_arrays(name: &str, index: usize) -> bool {
    match name.to_ascii_uppercase().as_str() {
        "FORECAST.ETS" | "FORECAST.ETS.CONFINT" => index == 1 || index == 2,
        "FORECAST.ETS.SEASONALITY" | "FORECAST.ETS.STAT" => index <= 1,
        _ => false,
    }
}

const ALPHAS: &[f64] = &[
    0.001, 0.002, 0.1, 0.126, 0.25, 0.251, 0.5, 0.749, 0.75, 0.751, 0.9,
];
const BETAS: &[f64] = &[0.001, 0.099];
const GAMMAS: &[f64] = &[f64::EPSILON, 0.001, 0.125, 0.25, 0.749];

/// The prepared, evenly spaced series.
struct Series {
    y: Vec<f64>,
    first: f64,
    step: f64,
}

/// Weights of the initial trend for series of 4 to 6 observations (numerators over 28, 140, 980).
fn trend_weights(n: usize) -> Option<(&'static [f64], f64)> {
    match n {
        4 => Some((&[-8.0, -3.0, 2.0, 9.0], 28.0)),
        5 => Some((&[-26.0, -13.0, -5.0, 13.0, 31.0], 140.0)),
        6 => Some((&[-90.0, -124.0, -53.0, 18.0, 89.0, 160.0], 980.0)),
        _ => None,
    }
}

fn ols_slope(y: &[f64]) -> f64 {
    let n = y.len() as f64;
    let xm = (n - 1.0) / 2.0;
    let ym = y.iter().sum::<f64>() / n;
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for (i, v) in y.iter().enumerate() {
        let dx = i as f64 - xm;
        sxy += dx * (v - ym);
        sxx += dx * dx;
    }
    if sxx == 0.0 { 0.0 } else { sxy / sxx }
}

fn initial_trend(y: &[f64]) -> f64 {
    match trend_weights(y.len()) {
        Some((w, d)) => w.iter().zip(y).map(|(a, b)| a * b).sum::<f64>() / d,
        None => ols_slope(y),
    }
}

/// Fit state after the last observation plus the in-sample errors.
struct Fit {
    alpha: f64,
    beta: f64,
    gamma: f64,
    level: f64,
    trend: f64,
    /// Seasonal components for the next `m` periods, starting with the next period.
    season: Vec<f64>,
    fitted: Vec<f64>,
    sse: f64,
}

fn seasonal_start(y: &[f64], b0: f64, m: usize) -> Vec<f64> {
    if m < 2 {
        return Vec::new();
    }
    let ym = y.iter().sum::<f64>() / y.len() as f64;
    let xm = (y.len() as f64 - 1.0) / 2.0;
    let mut sum = vec![0.0; m];
    let mut cnt = vec![0.0; m];
    for (i, v) in y.iter().enumerate() {
        sum[i % m] += v - (ym + b0 * (i as f64 - xm));
        cnt[i % m] += 1.0;
    }
    let mut s: Vec<f64> = sum.iter().zip(&cnt).map(|(a, c)| a / c).collect();
    let mean = s.iter().sum::<f64>() / m as f64;
    for v in &mut s {
        *v -= mean;
    }
    s
}

fn run(y: &[f64], m: usize, alpha: f64, beta: f64, gamma: f64) -> Fit {
    let seasonal = m >= 2;
    let b0 = if seasonal {
        // Trend of the series without its seasonal pattern.
        let s = seasonal_start(y, initial_trend(y), m);
        let adj: Vec<f64> = y.iter().enumerate().map(|(i, v)| v - s[i % m]).collect();
        initial_trend(&adj)
    } else {
        initial_trend(y)
    };
    let mut season = seasonal_start(y, b0, m);
    let mut l = y[0] - b0 - season.first().copied().unwrap_or(0.0);
    let mut b = b0;
    let mut sse = 0.0;
    let mut fitted = Vec::with_capacity(y.len());
    for (i, yt) in y.iter().enumerate() {
        let s = if seasonal { season[i % m] } else { 0.0 };
        let f = l + b + s;
        let e = yt - f;
        fitted.push(f);
        sse += e * e;
        l += b + alpha * e;
        b += beta * e;
        if seasonal {
            season[i % m] += gamma * e;
        }
    }
    let n = y.len();
    let next: Vec<f64> = if seasonal {
        (0..m).map(|j| season[(n + j) % m]).collect()
    } else {
        Vec::new()
    };
    Fit {
        alpha,
        beta,
        gamma: if seasonal { gamma } else { f64::EPSILON },
        level: l,
        trend: b,
        season: next,
        fitted,
        sse,
    }
}

fn fit(y: &[f64], m: usize) -> Fit {
    // Excel does not search for three observations: it keeps its start values (measured on 8 of 8 series).
    if y.len() == 3 {
        return run(y, m, 0.1, 0.099, f64::EPSILON);
    }
    let mut best: Option<Fit> = None;
    let gammas: &[f64] = if m >= 2 { GAMMAS } else { &GAMMAS[..1] };
    for &a in ALPHAS {
        for &b in BETAS {
            for &g in gammas {
                let f = run(y, m, a, b, g);
                if best.as_ref().is_none_or(|x| f.sse < x.sse) {
                    best = Some(f);
                }
            }
        }
    }
    best.unwrap_or_else(|| run(y, m, 0.5, 0.001, f64::EPSILON))
}

/// Seasonal period by the autocorrelation of the detrended series (0 when there is no clear one).
fn detect(y: &[f64]) -> usize {
    let n = y.len();
    if n < 4 {
        return 0;
    }
    let b = ols_slope(y);
    let ym = y.iter().sum::<f64>() / n as f64;
    let xm = (n as f64 - 1.0) / 2.0;
    let r: Vec<f64> = y
        .iter()
        .enumerate()
        .map(|(i, v)| v - ym - b * (i as f64 - xm))
        .collect();
    let var: f64 = r.iter().map(|v| v * v).sum();
    if var == 0.0 {
        return 0;
    }
    let mut best = (0usize, 0.3f64);
    for lag in 2..=n / 2 {
        let c: f64 = (lag..n).map(|i| r[i] * r[i - lag]).sum::<f64>() / var;
        if c > best.1 {
            best = (lag, c);
        }
    }
    best.0
}

/// Aggregation of duplicate timeline entries.
fn aggregate(mut v: Vec<f64>, how: i64) -> f64 {
    match how {
        2 | 3 => v.len() as f64,
        4 => v.iter().copied().fold(f64::MIN, f64::max),
        5 => {
            v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            let n = v.len();
            if n % 2 == 1 {
                v[n / 2]
            } else {
                (v[n / 2 - 1] + v[n / 2]) / 2.0
            }
        }
        6 => v.iter().copied().fold(f64::MAX, f64::min),
        7 => v.iter().sum(),
        _ => v.iter().sum::<f64>() / v.len() as f64,
    }
}

fn series(
    ctx: &EvalContext<'_>,
    values: &Arg,
    timeline: &Arg,
    completion: i64,
    aggregation: i64,
) -> Res<Series> {
    if !(0..=1).contains(&completion) || !(1..=7).contains(&aggregation) {
        return Err(ErrorKind::Num);
    }
    let vs: Vec<Value> = ctx.cells(values).collect();
    let ts: Vec<Value> = ctx.cells(timeline).collect();
    if vs.len() != ts.len() {
        return Err(ErrorKind::NA);
    }
    let mut pts: Vec<(f64, f64)> = Vec::with_capacity(vs.len());
    for (t, v) in ts.iter().zip(&vs) {
        match (t, v) {
            (Value::Error(e), _) | (_, Value::Error(e)) => return Err(*e),
            (Value::Number(t), Value::Number(v)) => pts.push((*t, *v)),
            _ => return Err(ErrorKind::Value),
        }
    }
    if pts.len() < 2 {
        return Err(ErrorKind::Value);
    }
    pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
    // Duplicates.
    let mut merged: Vec<(f64, f64)> = Vec::new();
    let mut i = 0;
    while i < pts.len() {
        let mut j = i;
        while j < pts.len() && pts[j].0 == pts[i].0 {
            j += 1;
        }
        let group: Vec<f64> = pts[i..j].iter().map(|p| p.1).collect();
        merged.push((pts[i].0, aggregate(group, aggregation)));
        i = j;
    }
    if merged.len() < 2 {
        return Err(ErrorKind::Value);
    }
    let step = merged
        .windows(2)
        .map(|w| w[1].0 - w[0].0)
        .fold(f64::MAX, f64::min);
    let first = merged[0].0;
    let slots = ((merged[merged.len() - 1].0 - first) / step).round() as usize + 1;
    if slots > 1 << 20 {
        return Err(ErrorKind::Num);
    }
    let mut y: Vec<Option<f64>> = vec![None; slots];
    for (t, v) in &merged {
        let pos = (t - first) / step;
        if (pos - pos.round()).abs() > 1e-9 {
            return Err(ErrorKind::Num);
        }
        y[pos.round() as usize] = Some(*v);
    }
    let mut out = Vec::with_capacity(slots);
    for k in 0..slots {
        match y[k] {
            Some(v) => out.push(v),
            None if completion == 0 => out.push(0.0),
            None => {
                let lo = (0..k).rev().find(|&q| y[q].is_some()).unwrap_or(0);
                let hi = (k + 1..slots)
                    .find(|&q| y[q].is_some())
                    .unwrap_or(slots - 1);
                let (a, b) = (y[lo].unwrap_or(0.0), y[hi].unwrap_or(0.0));
                out.push(a + (b - a) * (k - lo) as f64 / (hi - lo) as f64);
            }
        }
    }
    Ok(Series {
        y: out,
        first,
        step,
    })
}

fn seasonality_arg(ctx: &EvalContext<'_>, args: &[Arg], index: usize, s: &Series) -> Res<usize> {
    let m = match optional(args, index) {
        None => 1,
        Some(a) => int_arg(ctx, a)?,
    };
    match m {
        0 => Ok(0),
        1 => Ok(detect(&s.y)),
        2..=8760 if (m as usize) * 2 <= s.y.len() => Ok(m as usize),
        _ => Err(ErrorKind::Num),
    }
}

fn int_opt(ctx: &EvalContext<'_>, args: &[Arg], index: usize, default: i64) -> Res<i64> {
    match optional(args, index) {
        None => Ok(default),
        Some(a) => int_arg(ctx, a),
    }
}

fn horizon(s: &Series, target: f64) -> Res<f64> {
    let h = (target - s.first) / s.step - (s.y.len() as f64 - 1.0);
    if target < s.first {
        return Err(ErrorKind::Num);
    }
    Ok(h)
}

fn point(f: &Fit, m: usize, n: usize, h: f64) -> f64 {
    if h <= 0.0 {
        let i = (n as f64 - 1.0 + h).round().max(0.0) as usize;
        return f.fitted[i.min(n - 1)];
    }
    let hi = h.round().max(1.0) as usize;
    let s = if m >= 2 { f.season[(hi - 1) % m] } else { 0.0 };
    f.level + h * f.trend + s
}

fn rmse(f: &Fit, n: usize) -> f64 {
    (f.sse / n as f64).sqrt()
}

pub(super) fn forecast_ets(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 6)?;
    let target = num_arg(ctx, &args[0])?;
    let s = series(
        ctx,
        &args[1],
        &args[2],
        int_opt(ctx, args, 4, 1)?,
        int_opt(ctx, args, 5, 1)?,
    )?;
    let m = seasonality_arg(ctx, args, 3, &s)?;
    let f = fit(&s.y, m);
    number(point(&f, m, s.y.len(), horizon(&s, target)?))
}

pub(super) fn forecast_ets_confint(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 7)?;
    let target = num_arg(ctx, &args[0])?;
    let level = match optional(args, 3) {
        None => 0.95,
        Some(a) => num_arg(ctx, a)?,
    };
    if level <= 0.0 || level >= 1.0 {
        return Err(ErrorKind::Num);
    }
    let s = series(
        ctx,
        &args[1],
        &args[2],
        int_opt(ctx, args, 5, 1)?,
        int_opt(ctx, args, 6, 1)?,
    )?;
    let m = seasonality_arg(ctx, args, 4, &s)?;
    let f = fit(&s.y, m);
    let h = horizon(&s, target)?;
    let steps = h.round().max(1.0) as usize;
    let mut var = 1.0;
    for j in 1..steps {
        let mut c = f.alpha + j as f64 * f.beta;
        if m >= 2 && j % m == 0 {
            c += f.gamma;
        }
        var += c * c;
    }
    let z = norm_s_inv_f(1.0 - (1.0 - level) / 2.0)?;
    number(z * rmse(&f, s.y.len()) * var.sqrt())
}

pub(super) fn forecast_ets_seasonality(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 4)?;
    let s = series(
        ctx,
        &args[0],
        &args[1],
        int_opt(ctx, args, 2, 1)?,
        int_opt(ctx, args, 3, 1)?,
    )?;
    number(detect(&s.y) as f64)
}

pub(super) fn forecast_ets_stat(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 3, 6)?;
    let kind = int_arg(ctx, &args[2])?;
    if !(1..=8).contains(&kind) {
        return Err(ErrorKind::Num);
    }
    let s = series(
        ctx,
        &args[0],
        &args[1],
        int_opt(ctx, args, 4, 1)?,
        int_opt(ctx, args, 5, 1)?,
    )?;
    let m = seasonality_arg(ctx, args, 3, &s)?;
    let f = fit(&s.y, m);
    let n = s.y.len();
    let err = |i: usize| s.y[i] - f.fitted[i];
    let mae = (0..n).map(|i| err(i).abs()).sum::<f64>() / n as f64;
    match kind {
        1 => number(f.alpha),
        2 => number(f.beta),
        3 => number(f.gamma),
        4 => {
            let naive = (1..n).map(|i| (s.y[i] - s.y[i - 1]).abs()).sum::<f64>() / (n - 1) as f64;
            if naive == 0.0 {
                return Err(ErrorKind::DivZero);
            }
            number(mae / naive)
        }
        5 => {
            let total: f64 = (0..n)
                .map(|i| {
                    let d = s.y[i].abs() + f.fitted[i].abs();
                    if d == 0.0 {
                        0.0
                    } else {
                        2.0 * err(i).abs() / d
                    }
                })
                .sum();
            number(total / n as f64)
        }
        6 => number(mae),
        7 => number(rmse(&f, n)),
        _ => number(s.step),
    }
}
