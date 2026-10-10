//! Worksheet function registry and shared argument helpers.

mod criteria;
mod datetime;
mod logical;
mod lookup;
mod math;
mod text;

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::eval::{Arg, EvalContext};
use crate::value::{ErrorKind, Value};

pub use criteria::{Criteria, wildcard_match};
pub use datetime::{serial_to_ymd, ymd_to_serial};

/// A registered function body.
#[derive(Debug, Clone, Copy)]
pub enum Func {
    /// Returns a plain value.
    Val(fn(&EvalContext<'_>, &[Arg]) -> Value),
    /// Returns a value or an error, so the body can use `?`.
    Res(fn(&EvalContext<'_>, &[Arg]) -> Result<Value, ErrorKind>),
    /// Returns an argument as-is, so references pass through (`IF`, `CHOOSE`).
    Arg(fn(&EvalContext<'_>, &[Arg]) -> Arg),
}

static TABLE: &[(&str, Func)] = &[
    // math and statistics
    ("SUM", Func::Res(math::sum)),
    ("AVERAGE", Func::Res(math::average)),
    ("MIN", Func::Res(math::min)),
    ("MAX", Func::Res(math::max)),
    ("COUNT", Func::Val(math::count)),
    ("COUNTA", Func::Val(math::counta)),
    ("COUNTBLANK", Func::Res(math::countblank)),
    ("PRODUCT", Func::Res(math::product)),
    ("ROUND", Func::Res(math::round)),
    ("ROUNDUP", Func::Res(math::roundup)),
    ("ROUNDDOWN", Func::Res(math::rounddown)),
    ("INT", Func::Res(math::int)),
    ("MOD", Func::Res(math::modulo)),
    ("ABS", Func::Res(math::abs)),
    ("SQRT", Func::Res(math::sqrt)),
    ("POWER", Func::Res(math::power)),
    ("PI", Func::Res(math::pi)),
    ("SUMIF", Func::Res(criteria::sumif)),
    ("COUNTIF", Func::Res(criteria::countif)),
    ("AVERAGEIF", Func::Res(criteria::averageif)),
    ("SUMIFS", Func::Res(criteria::sumifs)),
    ("COUNTIFS", Func::Res(criteria::countifs)),
    // logical and information
    ("IF", Func::Arg(logical::if_)),
    ("IFERROR", Func::Arg(logical::iferror)),
    ("IFNA", Func::Arg(logical::ifna)),
    ("AND", Func::Res(logical::and)),
    ("OR", Func::Res(logical::or)),
    ("NOT", Func::Res(logical::not)),
    ("TRUE", Func::Res(logical::true_fn)),
    ("FALSE", Func::Res(logical::false_fn)),
    ("XOR", Func::Res(logical::xor)),
    ("ISBLANK", Func::Res(logical::isblank)),
    ("ISNUMBER", Func::Res(logical::isnumber)),
    ("ISTEXT", Func::Res(logical::istext)),
    ("ISERROR", Func::Res(logical::iserror)),
    ("ISNA", Func::Res(logical::isna)),
    // text
    ("LEN", Func::Res(text::len)),
    ("LEFT", Func::Res(text::left)),
    ("RIGHT", Func::Res(text::right)),
    ("MID", Func::Res(text::mid)),
    ("UPPER", Func::Res(text::upper)),
    ("LOWER", Func::Res(text::lower)),
    ("TRIM", Func::Res(text::trim)),
    ("CONCATENATE", Func::Res(text::concatenate)),
    ("CONCAT", Func::Res(text::concat)),
    ("TEXTJOIN", Func::Res(text::textjoin)),
    ("FIND", Func::Res(text::find)),
    ("SEARCH", Func::Res(text::search)),
    ("SUBSTITUTE", Func::Res(text::substitute)),
    ("REPT", Func::Res(text::rept)),
    ("VALUE", Func::Res(text::value)),
    // lookup and reference
    ("VLOOKUP", Func::Res(lookup::vlookup)),
    ("HLOOKUP", Func::Res(lookup::hlookup)),
    ("INDEX", Func::Arg(lookup::index)),
    ("MATCH", Func::Res(lookup::match_)),
    ("XLOOKUP", Func::Res(lookup::xlookup)),
    ("CHOOSE", Func::Arg(lookup::choose)),
    // date and time
    ("DATE", Func::Res(datetime::date)),
    ("YEAR", Func::Res(datetime::year)),
    ("MONTH", Func::Res(datetime::month)),
    ("DAY", Func::Res(datetime::day)),
    ("TODAY", Func::Res(datetime::today)),
    ("NOW", Func::Res(datetime::now)),
];

/// Functions whose result changes without any precedent changing.
const VOLATILE: &[&str] = &["TODAY", "NOW", "RAND", "RANDBETWEEN", "OFFSET", "INDIRECT"];

fn table() -> &'static HashMap<&'static str, Func> {
    static MAP: OnceLock<HashMap<&'static str, Func>> = OnceLock::new();
    MAP.get_or_init(|| TABLE.iter().copied().collect())
}

/// Looks a function up by name (any case).
pub fn lookup(name: &str) -> Option<&'static Func> {
    let map = table();
    map.get(name)
        .or_else(|| map.get(name.to_ascii_uppercase().as_str()))
}

/// Parameters whose argument expressions Excel evaluates over whole ranges instead of taking the
/// implicit intersection (the array parameters of `INDEX` and `XLOOKUP`).
pub(crate) fn array_params(name: &str) -> &'static [usize] {
    if name.eq_ignore_ascii_case("INDEX") {
        &[0]
    } else if name.eq_ignore_ascii_case("XLOOKUP") {
        &[1, 2]
    } else {
        &[]
    }
}

/// All implemented function names, sorted.
pub fn function_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = TABLE.iter().map(|(n, _)| *n).collect();
    names.sort_unstable();
    names
}

/// True for functions that must be recalculated on every pass.
pub fn is_volatile(name: &str) -> bool {
    VOLATILE.iter().any(|v| v.eq_ignore_ascii_case(name))
}

// ---- shared argument helpers ---------------------------------------------------------

pub(crate) type Res<T> = Result<T, ErrorKind>;

/// Checks the argument count.
pub(crate) fn arity(args: &[Arg], min: usize, max: usize) -> Res<()> {
    if args.len() < min || args.len() > max {
        Err(ErrorKind::Value)
    } else {
        Ok(())
    }
}

/// Scalar value of an argument, propagating errors.
pub(crate) fn scalar(ctx: &EvalContext<'_>, arg: &Arg) -> Res<Value> {
    match ctx.scalar(arg) {
        Value::Error(e) => Err(e),
        v => Ok(v),
    }
}

pub(crate) fn num_arg(ctx: &EvalContext<'_>, arg: &Arg) -> Res<f64> {
    scalar(ctx, arg)?.to_number_in(ctx.date1904)
}

pub(crate) fn text_arg(ctx: &EvalContext<'_>, arg: &Arg) -> Res<String> {
    scalar(ctx, arg)?.to_text()
}

pub(crate) fn bool_arg(ctx: &EvalContext<'_>, arg: &Arg) -> Res<bool> {
    scalar(ctx, arg)?.to_bool()
}

/// Number argument truncated toward zero, as Excel does for integer parameters.
pub(crate) fn int_arg(ctx: &EvalContext<'_>, arg: &Arg) -> Res<i64> {
    let n = num_arg(ctx, arg)?.trunc();
    if n.abs() > 9.0e15 {
        return Err(ErrorKind::Num);
    }
    Ok(n as i64)
}

/// An optional argument: absent or omitted gives `None`.
pub(crate) fn optional(args: &[Arg], index: usize) -> Option<&Arg> {
    match args.get(index) {
        None | Some(Arg::Missing) => None,
        Some(a) => Some(a),
    }
}

/// Wraps a finite float as a number or `#NUM!`.
pub(crate) fn number(n: f64) -> Res<Value> {
    if n.is_finite() {
        Ok(Value::Number(n))
    } else {
        Err(ErrorKind::Num)
    }
}
