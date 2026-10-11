//! `LET`, `LAMBDA` and the functions that take a lambda (`MAP`, `REDUCE`, `SCAN`, `BYROW`, `BYCOL`,
//! `MAKEARRAY`), plus `ISOMITTED`.
//!
//! These need the unevaluated argument expressions (a `LET` name is not a value) and call back into the
//! evaluator, so they live beside it instead of in the function registry. A lambda is a closure: its
//! parameter names, its body and a snapshot of the `LET`/parameter names visible where it was written.
//!
//! Rules measured against Excel (Range.Formula, so no spill):
//! * `LET` values and bodies, and lambda bodies, are evaluated outside array mode. A name bound to an array
//!   that an operator or a scalar function consumes there is its first element (`LET(x,{1,2,3},x+1)` is 2);
//!   a computed array (`SEQUENCE(3)+1`) still lifts.
//! * A lambda must receive exactly the parameters it declares (optional ones, written `[y]`, may be left out);
//!   a mismatch is `#VALUE!`. A lambda that returns an array where `MAP`, `SCAN`, `BYROW`, `BYCOL` or
//!   `MAKEARRAY` need a single value makes the whole call `#CALC!`.
//! * `MAP` pads arrays of different sizes with `#N/A` and refuses a mix of scalars and arrays.
//! * A function name used as a value (`MAP(a,SUM)`, `LET(f,ABS,f(-1))`) is a lambda over that function.

use std::sync::Arc;

use super::{Arg, Evaluator, RangeRef, call_function};
use crate::ast::{Ast, to_formula_string};
use crate::functions;
use crate::value::{ErrorKind, Value};

/// Deepest chain of nested lambda calls before the call fails.
const MAX_LAMBDA_DEPTH: u32 = 256;
/// Largest `MAKEARRAY` result in cells that is attempted (a guard, not a measured Excel limit).
const MAX_CELLS: f64 = 4_194_304.0;

/// A function value: what `LAMBDA(..)` returns, or a bare function name such as `SUM`.
#[derive(Debug)]
pub struct Lambda {
    kind: Kind,
}

#[derive(Debug)]
enum Kind {
    Closure {
        params: Vec<Param>,
        body: Ast,
        env: Vec<(String, Arg)>,
    },
    Builtin(String),
}

#[derive(Debug)]
struct Param {
    name: String,
    optional: bool,
}

/// Two lambdas are equal only when they are the same value.
impl PartialEq for Lambda {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

impl Lambda {
    /// Number of parameters, if fixed (a function name accepts any number).
    fn arity(&self) -> Option<usize> {
        match &self.kind {
            Kind::Closure { params, .. } => Some(params.len()),
            Kind::Builtin(_) => None,
        }
    }
}

/// The result of calling a lambda where one value is needed.
fn single(arg: &Evaluator<'_, '_>, result: Arg) -> Result<Value, ErrorKind> {
    match result {
        Arg::Value(v) => Ok(v),
        Arg::Missing => Ok(Value::Empty),
        Arg::Array(rows) => match rows.as_slice() {
            [row] if row.len() == 1 => Ok(row[0].clone()),
            _ => Err(ErrorKind::Calc),
        },
        Arg::Range(r) if r.is_single_cell() => Ok(arg.ctx.scalar(&Arg::Range(r))),
        Arg::Range(_) | Arg::Union(_) | Arg::Refs(_) | Arg::Lambda(_) => Err(ErrorKind::Calc),
    }
}

/// A part of an array handed to a lambda: a single element is a plain value, not a 1x1 array.
fn one(rows: Vec<Vec<Value>>) -> Arg {
    match rows.as_slice() {
        [row] if row.len() == 1 => Arg::Value(row[0].clone()),
        _ => Arg::Array(rows),
    }
}

/// Dimensions and element access for the array arguments of `MAP`, `REDUCE`, `SCAN`, `BYROW`, `BYCOL`.
enum Source {
    Scalar(Arg),
    Range(RangeRef),
    Rows(Vec<Vec<Value>>),
}

impl Source {
    fn rows(&self) -> usize {
        match self {
            Source::Scalar(_) => 1,
            Source::Range(r) => r.rows() as usize,
            Source::Rows(rows) => rows.len(),
        }
    }

    fn cols(&self) -> usize {
        match self {
            Source::Scalar(_) => 1,
            Source::Range(r) => r.cols() as usize,
            Source::Rows(rows) => rows.first().map_or(0, Vec::len),
        }
    }

    /// The element handed to a lambda: a reference for a range, so `ISBLANK` and friends still see the cell.
    fn element(&self, row: usize, col: usize) -> Arg {
        match self {
            Source::Scalar(a) => a.clone(),
            Source::Range(r) => {
                let (row, col) = (r.row1 + row as u32, r.col1 + col as u32);
                Arg::Range(RangeRef::new(&r.sheet, row, col, row, col))
            }
            Source::Rows(rows) => Arg::Value(rows[row][col].clone()),
        }
    }

    /// Row `row` as one argument: a reference to the row of a range, else an array.
    fn row_arg(&self, row: usize) -> Arg {
        match self {
            Source::Scalar(a) => a.clone(),
            Source::Range(r) => {
                let line = r.row1 + row as u32;
                Arg::Range(RangeRef::new(&r.sheet, line, r.col1, line, r.col2))
            }
            Source::Rows(rows) => one(vec![rows[row].clone()]),
        }
    }

    fn col_arg(&self, col: usize) -> Arg {
        match self {
            Source::Scalar(a) => a.clone(),
            Source::Range(r) => {
                let line = r.col1 + col as u32;
                Arg::Range(RangeRef::new(&r.sheet, r.row1, line, r.row2, line))
            }
            Source::Rows(rows) => one(rows.iter().map(|row| vec![row[col].clone()]).collect()),
        }
    }
}

impl Evaluator<'_, '_> {
    /// The special forms. `None` when `name` is an ordinary function.
    pub(super) fn eval_special(&self, name: &str, args: &[Ast]) -> Option<Arg> {
        Some(match name.to_ascii_uppercase().as_str() {
            "LET" => self.eval_let(args),
            "LAMBDA" => self.eval_lambda(args),
            "MAP" => self.eval_map(args),
            "REDUCE" => self.eval_reduce(args, false),
            "SCAN" => self.eval_reduce(args, true),
            "BYROW" => self.eval_by(args, true),
            "BYCOL" => self.eval_by(args, false),
            "MAKEARRAY" => self.eval_makearray(args),
            "ISOMITTED" => self.eval_isomitted(args),
            _ => return None,
        })
    }

    pub(super) fn local(&self, name: &str) -> Option<Arg> {
        let locals = self.locals.borrow();
        locals
            .iter()
            .rev()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, a)| a.clone())
    }

    /// An array bound to a name and consumed by an operator or scalar function outside array mode is its
    /// first element; ranges keep their implicit intersection and computed arrays still lift.
    pub(super) fn scalarize(&self, ast: &Ast, arg: Arg) -> Arg {
        if let (Ast::Name(n), Arg::Array(rows)) = (ast, &arg)
            && (rows.len() > 1 || rows.first().is_some_and(|r| r.len() > 1))
            && self.local(n).is_some()
        {
            return Arg::from_value(self.ctx.scalar(&arg));
        }
        arg
    }

    /// Arguments that are names bound to arrays, for a function evaluated outside array mode: the function
    /// sees their first element, unless another argument is a computed array that makes the call lift over
    /// all of them (`CHOOSE({1,2},x,x)`). `TYPE` and `TRANSPOSE` take the first element in any case.
    pub(super) fn scalarize_names(
        &self,
        name: &str,
        asts: &[Ast],
        mut evaluated: Vec<Arg>,
        lifting: Option<&crate::functions::Lifting>,
        array: bool,
        expand: bool,
    ) -> Vec<Arg> {
        let is_type = name.eq_ignore_ascii_case("TYPE");
        let always = name.eq_ignore_ascii_case("TRANSPOSE") || is_type && !array;
        if !always && (lifting.is_none() || array && expand) {
            return evaluated;
        }
        let is_array = |a: &Arg| matches!(a, Arg::Array(rows) if rows.len() > 1 || rows.first().is_some_and(|r| r.len() > 1));
        let is_local = |ast: &Ast| match ast {
            Ast::Name(n) => self.local(n).is_some(),
            // `TYPE(LET(x,{1,2},x))` sees the first element of what `LET` returns.
            Ast::Invoke { .. } => is_type,
            Ast::Func { name, .. } => is_type && name.eq_ignore_ascii_case("LET"),
            _ => false,
        };
        let others_lift = !always
            && lifting.is_some_and(|l| {
                evaluated.iter().zip(asts).enumerate().any(|(i, (a, ast))| {
                    (l.trigger.unwrap_or(l.scalar_param))(i) && is_array(a) && !is_local(ast)
                })
            });
        if others_lift {
            return evaluated;
        }
        for (i, (a, ast)) in evaluated.iter_mut().zip(asts).enumerate() {
            let scalar_param = always || lifting.is_some_and(|l| (l.scalar_param)(i));
            if scalar_param && is_local(ast) && is_array(a) {
                *a = Arg::from_value(self.ctx.scalar(a));
            }
        }
        evaluated
    }

    /// An expression used as a value: a bare function name is a function value.
    pub(super) fn eval_value(&self, ast: &Ast) -> Arg {
        if let Ast::Name(n) = ast
            && self.local(n).is_none()
            && self.ctx.source.name(n).is_none()
            && functions::lookup(n).is_some()
        {
            return Arg::Lambda(Arc::new(Lambda {
                kind: Kind::Builtin(n.to_ascii_uppercase()),
            }));
        }
        self.eval_in(ast, false)
    }

    fn with_locals<T>(&self, locals: Vec<(String, Arg)>, f: impl FnOnce() -> T) -> T {
        let saved = self.locals.replace(locals);
        let out = f();
        self.locals.replace(saved);
        out
    }

    fn eval_let(&self, args: &[Ast]) -> Arg {
        if args.len() < 3 || args.len().is_multiple_of(2) {
            return Arg::error(ErrorKind::Value);
        }
        let mark = self.locals.borrow().len();
        let mut seen: Vec<String> = Vec::new();
        let (pairs, body) = args.split_at(args.len() - 1);
        let mut failure = None;
        for pair in pairs.chunks(2) {
            let Some(name) = name_of(&pair[0]) else {
                failure = Some(ErrorKind::Value);
                break;
            };
            if seen.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                failure = Some(ErrorKind::Value);
                break;
            }
            let value = self.eval_value(&pair[1]);
            seen.push(name.clone());
            self.locals.borrow_mut().push((name, value));
        }
        let result = match failure {
            Some(e) => Arg::error(e),
            None => self.eval_in(&body[0], false),
        };
        self.locals.borrow_mut().truncate(mark);
        result
    }

    fn eval_lambda(&self, args: &[Ast]) -> Arg {
        let Some((body, params)) = args.split_last() else {
            return Arg::error(ErrorKind::Value);
        };
        let mut out: Vec<Param> = Vec::new();
        for p in params {
            let Some(name) = name_of(p) else {
                return Arg::error(ErrorKind::Value);
            };
            let (name, optional) = match name.strip_prefix('[').and_then(|n| n.strip_suffix(']')) {
                Some(inner) => (inner.to_owned(), true),
                None => (name, false),
            };
            if out.iter().any(|q| q.name.eq_ignore_ascii_case(&name)) {
                return Arg::error(ErrorKind::Value);
            }
            out.push(Param { name, optional });
        }
        Arg::Lambda(Arc::new(Lambda {
            kind: Kind::Closure {
                params: out,
                body: body.clone(),
                env: self.locals.borrow().clone(),
            },
        }))
    }

    fn eval_isomitted(&self, args: &[Ast]) -> Arg {
        let [arg] = args else {
            return Arg::error(ErrorKind::Value);
        };
        match arg {
            Ast::Name(n) => match self.local(n) {
                Some(Arg::Missing) => Arg::Value(Value::Bool(true)),
                _ => Arg::Value(Value::Bool(false)),
            },
            _ => Arg::Value(Value::Bool(false)),
        }
    }

    /// The arguments of a call of a lambda value: each outside array mode, an omitted one stays omitted.
    pub(super) fn call_args(&self, args: &[Ast]) -> Vec<Arg> {
        args.iter().map(|a| self.eval_value(a)).collect()
    }

    /// Calls a lambda value.
    pub(super) fn invoke(&self, callee: &Arg, args: Vec<Arg>) -> Arg {
        let lambda = match callee {
            Arg::Lambda(l) => Arc::clone(l),
            Arg::Value(Value::Error(e)) => return Arg::error(*e),
            _ => return Arg::error(ErrorKind::Value),
        };
        self.invoke_lambda(&lambda, args)
    }

    fn invoke_lambda(&self, lambda: &Arc<Lambda>, args: Vec<Arg>) -> Arg {
        match &lambda.kind {
            Kind::Builtin(name) => {
                let Some(func) = functions::lookup(name) else {
                    return Arg::error(ErrorKind::Name);
                };
                call_function(self.ctx, func, &args)
            }
            Kind::Closure { params, body, env } => {
                if args.len() > params.len() {
                    return Arg::error(ErrorKind::Value);
                }
                let mut locals = env.clone();
                for (i, p) in params.iter().enumerate() {
                    let value = match args.get(i) {
                        Some(a) => a.clone(),
                        None if p.optional => Arg::Missing,
                        None => return Arg::error(ErrorKind::Value),
                    };
                    locals.push((p.name.clone(), value));
                }
                let depth = self.lambda_depth.get();
                if depth >= MAX_LAMBDA_DEPTH {
                    return Arg::error(ErrorKind::Num);
                }
                self.lambda_depth.set(depth + 1);
                let result = self.with_locals(locals, || self.eval_in(body, false));
                self.lambda_depth.set(depth);
                result
            }
        }
    }

    /// The lambda argument of `MAP` and friends, checked against the number of values it will receive.
    fn lambda_arg(&self, ast: &Ast, wanted: usize) -> Result<Arc<Lambda>, ErrorKind> {
        match self.eval_value(ast) {
            Arg::Lambda(l) => {
                if l.arity().is_some_and(|n| n != wanted) {
                    return Err(ErrorKind::Value);
                }
                Ok(l)
            }
            Arg::Value(Value::Error(e)) => Err(e),
            _ => Err(ErrorKind::Value),
        }
    }

    /// An array argument. An error value that is the whole argument is the error of the call.
    fn source(&self, ast: &Ast) -> Result<Source, ErrorKind> {
        match self.eval_in(ast, true) {
            Arg::Range(r) => Ok(Source::Range(r)),
            Arg::Array(rows) => Ok(Source::Rows(rows)),
            Arg::Value(Value::Error(e)) => Err(e),
            Arg::Missing => Ok(Source::Scalar(Arg::Missing)),
            other @ Arg::Value(_) => Ok(Source::Scalar(other)),
            Arg::Lambda(_) => Err(ErrorKind::Calc),
            Arg::Union(_) | Arg::Refs(_) => Err(ErrorKind::Value),
        }
    }

    fn eval_map(&self, args: &[Ast]) -> Arg {
        let Some((last, arrays)) = args.split_last() else {
            return Arg::error(ErrorKind::Value);
        };
        if arrays.is_empty() {
            return Arg::error(ErrorKind::Value);
        }
        let mut sources = Vec::with_capacity(arrays.len());
        let mut error = None;
        for a in arrays {
            match self.source(a) {
                Ok(s) => sources.push(s),
                Err(e) => {
                    error.get_or_insert(e);
                    sources.push(Source::Scalar(Arg::error(e)));
                }
            }
        }
        let lambda = match self.lambda_arg(last, sources.len()) {
            Ok(l) => l,
            Err(e) => return Arg::error(e),
        };
        let scalars = sources
            .iter()
            .filter(|s| matches!(s, Source::Scalar(_)))
            .count();
        // A single scalar argument is mapped as a 1x1 array. With several arguments a scalar is `#VALUE!`
        // (even when all are scalars), and an error value as the only argument is the result.
        if let Some(e) = error
            && (scalars == sources.len() || sources.len() == 1)
        {
            return Arg::error(e);
        }
        if sources.len() > 1 && scalars != 0 {
            return Arg::error(ErrorKind::Value);
        }
        if let Some(e) = error {
            return Arg::error(e);
        }
        let rows = sources.iter().map(Source::rows).max().unwrap_or(1);
        let cols = sources.iter().map(Source::cols).max().unwrap_or(1);
        let mut out = Vec::with_capacity(rows);
        for r in 0..rows {
            let mut line = Vec::with_capacity(cols);
            for c in 0..cols {
                if sources.iter().any(|s| r >= s.rows() || c >= s.cols()) {
                    line.push(Value::Error(ErrorKind::NA));
                    continue;
                }
                let call: Vec<Arg> = sources.iter().map(|s| s.element(r, c)).collect();
                match single(self, self.invoke_lambda(&lambda, call)) {
                    Ok(v) => line.push(v),
                    Err(e) => return Arg::error(e),
                }
            }
            out.push(line);
        }
        Arg::Array(out)
    }

    /// `REDUCE(initial, array, lambda)` and `SCAN(initial, array, lambda)`.
    fn eval_reduce(&self, args: &[Ast], scan: bool) -> Arg {
        let [initial, array, lambda] = args else {
            return Arg::error(ErrorKind::Value);
        };
        let init = self.eval_value(initial);
        let source = self.source(array);
        let lambda = match self.lambda_arg(lambda, 2) {
            Ok(l) => l,
            Err(e) => return Arg::error(e),
        };
        let source = match source {
            Ok(s) => s,
            Err(e) => return Arg::error(e),
        };
        let (rows, cols) = (source.rows(), source.cols());
        let mut acc = init;
        let mut out = Vec::with_capacity(rows);
        for r in 0..rows {
            let mut line = Vec::with_capacity(cols);
            for c in 0..cols {
                acc = self.invoke_lambda(&lambda, vec![acc, source.element(r, c)]);
                if scan {
                    match single(self, acc.clone()) {
                        Ok(v) => {
                            acc = Arg::Value(v.clone());
                            line.push(v);
                        }
                        Err(e) => return Arg::error(e),
                    }
                }
            }
            out.push(line);
        }
        if scan {
            return Arg::Array(out);
        }
        match acc {
            Arg::Lambda(_) => Arg::error(ErrorKind::Calc),
            other => other,
        }
    }

    /// `BYROW(array, lambda)` and `BYCOL(array, lambda)`.
    fn eval_by(&self, args: &[Ast], by_row: bool) -> Arg {
        let [array, lambda] = args else {
            // Without the lambda the call is incomplete, not malformed.
            return Arg::error(if args.len() == 1 {
                ErrorKind::Calc
            } else {
                ErrorKind::Value
            });
        };
        let source = self.source(array);
        let lambda = match self.lambda_arg(lambda, 1) {
            Ok(l) => l,
            Err(e) => return Arg::error(e),
        };
        let source = match source {
            Ok(s) => s,
            Err(e) => return Arg::error(e),
        };
        let n = if by_row { source.rows() } else { source.cols() };
        let mut values = Vec::with_capacity(n);
        for i in 0..n {
            let part = if by_row {
                source.row_arg(i)
            } else {
                source.col_arg(i)
            };
            match single(self, self.invoke_lambda(&lambda, vec![part])) {
                Ok(v) => values.push(v),
                Err(e) => return Arg::error(e),
            }
        }
        if by_row {
            Arg::Array(values.into_iter().map(|v| vec![v]).collect())
        } else {
            Arg::Array(vec![values])
        }
    }

    fn eval_makearray(&self, args: &[Ast]) -> Arg {
        let [rows, cols, lambda] = args else {
            return Arg::error(ErrorKind::Value);
        };
        let dimension = |ast: &Ast| -> Result<f64, ErrorKind> {
            let v = self.ctx.scalar(&self.eval_in(ast, false));
            let n = v.to_number_in(self.ctx.date1904)?;
            if n < 1.0 || n.fract() != 0.0 {
                return Err(ErrorKind::Value);
            }
            Ok(n)
        };
        let rows = dimension(rows);
        let cols = dimension(cols);
        let lambda = match self.lambda_arg(lambda, 2) {
            Ok(l) => l,
            Err(e) => return Arg::error(e),
        };
        let (rows, cols) = match (rows, cols) {
            (Ok(r), Ok(c)) => (r, c),
            (Err(e), _) | (_, Err(e)) => return Arg::error(e),
        };
        if rows * cols > MAX_CELLS {
            return Arg::error(ErrorKind::Calc);
        }
        let (rows, cols) = (rows as usize, cols as usize);
        let mut out = Vec::with_capacity(rows);
        for r in 1..=rows {
            let mut line = Vec::with_capacity(cols);
            for c in 1..=cols {
                let call = vec![
                    Arg::Value(Value::Number(r as f64)),
                    Arg::Value(Value::Number(c as f64)),
                ];
                match single(self, self.invoke_lambda(&lambda, call)) {
                    Ok(v) => line.push(v),
                    Err(e) => return Arg::error(e),
                }
            }
            out.push(line);
        }
        Arg::Array(out)
    }

    /// What the functions that look at their argument's type say about a lambda value, where that differs
    /// from treating it as an error: `ISERROR(f)` is FALSE and `TYPE(f)` is 128.
    pub(super) fn lambda_answer(name: &str, args: &[Arg]) -> Option<Arg> {
        let boolean = |b: bool| Some(Arg::Value(Value::Bool(b)));
        match name.to_ascii_uppercase().as_str() {
            "ISNUMBER" | "ISTEXT" | "ISERROR" | "ISERR" | "ISNA" | "ISBLANK" | "ISLOGICAL"
            | "ISREF" => boolean(false),
            "ISNONTEXT" => boolean(true),
            "TYPE" => Some(Arg::Value(Value::Number(128.0))),
            "ARRAYTOTEXT" | "VALUETOTEXT" => Some(Arg::Value(Value::Text("\u{3bb}".to_owned()))),
            "ERROR.TYPE" => Some(Arg::error(ErrorKind::NA)),
            // These hand their first argument through.
            "INDEX" | "TRANSPOSE" | "IFERROR" | "IFNA" => match args.first() {
                Some(first @ Arg::Lambda(_)) => Some(first.clone()),
                _ => None,
            },
            _ => None,
        }
    }
}

/// The name a `LET` or `LAMBDA` argument declares.
fn name_of(ast: &Ast) -> Option<String> {
    match ast {
        // A period is not allowed in a `LET` or `LAMBDA` name.
        Ast::Name(n) if n.contains('.') => None,
        Ast::Name(n) => Some(n.clone()),
        Ast::Ref(_) => Some(to_formula_string(ast)),
        _ => None,
    }
}
