//! Worksheet function registry and shared argument helpers.

mod arith;
mod criteria;
mod datecalc;
mod datetime;
mod letters;
mod logical;
mod lookup;
mod math;
mod stats;
mod text;
mod textbyte;
mod textchar;
mod textfmt;
mod textsplit;

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
    ("SUMPRODUCT", Func::Res(stats::sumproduct)),
    ("SUMSQ", Func::Res(arith::sumsq)),
    ("CEILING", Func::Res(arith::ceiling)),
    ("CEILING.MATH", Func::Res(arith::ceiling_math)),
    ("FLOOR", Func::Res(arith::floor)),
    ("FLOOR.MATH", Func::Res(arith::floor_math)),
    ("MROUND", Func::Res(arith::mround)),
    ("TRUNC", Func::Res(arith::trunc)),
    ("SIGN", Func::Res(arith::sign)),
    ("EXP", Func::Res(arith::exp)),
    ("LN", Func::Res(arith::ln)),
    ("LOG", Func::Res(arith::log)),
    ("LOG10", Func::Res(arith::log10)),
    ("FACT", Func::Res(arith::fact)),
    ("COMBIN", Func::Res(arith::combin)),
    ("PERMUT", Func::Res(arith::permut)),
    ("GCD", Func::Res(arith::gcd)),
    ("LCM", Func::Res(arith::lcm)),
    ("QUOTIENT", Func::Res(arith::quotient)),
    ("EVEN", Func::Res(arith::even)),
    ("ODD", Func::Res(arith::odd)),
    ("RAND", Func::Res(arith::rand)),
    ("RANDBETWEEN", Func::Res(arith::randbetween)),
    ("SIN", Func::Res(arith::sin)),
    ("COS", Func::Res(arith::cos)),
    ("TAN", Func::Res(arith::tan)),
    ("ASIN", Func::Res(arith::asin)),
    ("ACOS", Func::Res(arith::acos)),
    ("ATAN", Func::Res(arith::atan)),
    ("ATAN2", Func::Res(arith::atan2)),
    ("DEGREES", Func::Res(arith::degrees)),
    ("RADIANS", Func::Res(arith::radians)),
    ("SINH", Func::Res(arith::sinh)),
    ("COSH", Func::Res(arith::cosh)),
    ("TANH", Func::Res(arith::tanh)),
    ("MEDIAN", Func::Res(stats::median)),
    ("MODE", Func::Res(stats::mode)),
    ("MODE.SNGL", Func::Res(stats::mode)),
    ("LARGE", Func::Res(stats::large)),
    ("SMALL", Func::Res(stats::small)),
    ("RANK", Func::Res(stats::rank)),
    ("RANK.EQ", Func::Res(stats::rank)),
    ("RANK.AVG", Func::Res(stats::rank_avg)),
    ("PERCENTILE", Func::Res(stats::percentile)),
    ("PERCENTILE.INC", Func::Res(stats::percentile)),
    ("PERCENTILE.EXC", Func::Res(stats::percentile_exc)),
    ("QUARTILE", Func::Res(stats::quartile)),
    ("QUARTILE.INC", Func::Res(stats::quartile)),
    ("QUARTILE.EXC", Func::Res(stats::quartile_exc)),
    ("STDEV", Func::Res(stats::stdev_s)),
    ("STDEV.S", Func::Res(stats::stdev_s)),
    ("STDEV.P", Func::Res(stats::stdev_p)),
    ("STDEVP", Func::Res(stats::stdev_p)),
    ("VAR", Func::Res(stats::var_s)),
    ("VAR.S", Func::Res(stats::var_s)),
    ("VAR.P", Func::Res(stats::var_p)),
    ("VARP", Func::Res(stats::var_p)),
    ("AVERAGEIFS", Func::Res(criteria::averageifs)),
    ("MAXIFS", Func::Res(criteria::maxifs)),
    ("MINIFS", Func::Res(criteria::minifs)),
    ("SUBTOTAL", Func::Res(stats::subtotal)),
    ("AGGREGATE", Func::Res(stats::aggregate)),
    ("AVERAGEA", Func::Res(stats::averagea)),
    ("MAXA", Func::Res(stats::maxa)),
    ("MINA", Func::Res(stats::mina)),
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
    ("NA", Func::Res(logical::na)),
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
    ("TEXT", Func::Res(textfmt::text)),
    ("FIXED", Func::Res(textfmt::fixed)),
    ("DOLLAR", Func::Res(textfmt::dollar)),
    ("NUMBERVALUE", Func::Res(textfmt::numbervalue)),
    ("VALUETOTEXT", Func::Res(textfmt::valuetotext)),
    ("ARRAYTOTEXT", Func::Res(textfmt::arraytotext)),
    ("CHAR", Func::Res(textchar::char_fn)),
    ("CODE", Func::Res(textchar::code)),
    ("UNICHAR", Func::Res(textchar::unichar)),
    ("UNICODE", Func::Res(textchar::unicode)),
    ("EXACT", Func::Res(textchar::exact)),
    ("PROPER", Func::Res(textchar::proper)),
    ("CLEAN", Func::Res(textchar::clean)),
    ("T", Func::Res(textchar::t)),
    ("N", Func::Res(textchar::n)),
    ("REPLACE", Func::Res(textbyte::replace)),
    ("REPLACEB", Func::Res(textbyte::replaceb)),
    ("LENB", Func::Res(textbyte::lenb)),
    ("LEFTB", Func::Res(text::left)),
    ("RIGHTB", Func::Res(text::right)),
    ("MIDB", Func::Res(textbyte::midb)),
    ("FINDB", Func::Res(textbyte::findb)),
    ("SEARCHB", Func::Res(textbyte::searchb)),
    ("TEXTBEFORE", Func::Res(textsplit::textbefore)),
    ("TEXTAFTER", Func::Res(textsplit::textafter)),
    ("TEXTSPLIT", Func::Res(textsplit::textsplit)),
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
    ("TIME", Func::Res(datecalc::time)),
    ("HOUR", Func::Res(datecalc::hour)),
    ("MINUTE", Func::Res(datecalc::minute)),
    ("SECOND", Func::Res(datecalc::second)),
    ("WEEKDAY", Func::Res(datecalc::weekday)),
    ("WEEKNUM", Func::Res(datecalc::weeknum)),
    ("ISOWEEKNUM", Func::Res(datecalc::isoweeknum)),
    ("EDATE", Func::Res(datecalc::edate)),
    ("EOMONTH", Func::Res(datecalc::eomonth)),
    ("DATEDIF", Func::Res(datecalc::datedif)),
    ("DAYS", Func::Res(datecalc::days)),
    ("DAYS360", Func::Res(datecalc::days360)),
    ("NETWORKDAYS", Func::Res(datecalc::networkdays)),
    ("NETWORKDAYS.INTL", Func::Res(datecalc::networkdays_intl)),
    ("WORKDAY", Func::Res(datecalc::workday)),
    ("WORKDAY.INTL", Func::Res(datecalc::workday_intl)),
    ("YEARFRAC", Func::Res(datecalc::yearfrac)),
    ("DATEVALUE", Func::Res(datecalc::datevalue)),
    ("TIMEVALUE", Func::Res(datecalc::timevalue)),
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
/// implicit intersection (the array parameters of `INDEX` and `XLOOKUP`, every parameter of `SUMPRODUCT`).
pub(crate) fn is_array_param(name: &str, index: usize) -> bool {
    if name.eq_ignore_ascii_case("INDEX") {
        index == 0
    } else if name.eq_ignore_ascii_case("XLOOKUP") {
        index == 1 || index == 2
    } else {
        name.eq_ignore_ascii_case("SUMPRODUCT")
    }
}

/// Functions whose parameters are all scalars: in an array context they are applied element by element.
pub(crate) fn is_elementwise(name: &str) -> bool {
    const NAMES: &[&str] = &[
        "ABS",
        "INT",
        "SQRT",
        "ISNUMBER",
        "ISTEXT",
        "ISBLANK",
        "ISERROR",
        "ISNA",
        "NOT",
        "LEN",
        "LEFT",
        "RIGHT",
        "MID",
        "UPPER",
        "LOWER",
        "TRIM",
        "ROUND",
        "ROUNDUP",
        "ROUNDDOWN",
        "MOD",
        "POWER",
        "SIGN",
        "EXP",
        "LN",
        "LOG",
        "LOG10",
        "SIN",
        "COS",
        "TAN",
        "ASIN",
        "ACOS",
        "ATAN",
        "ATAN2",
        "SINH",
        "COSH",
        "TANH",
        "DEGREES",
        "RADIANS",
        "CEILING",
        "FLOOR",
        "TRUNC",
        "MROUND",
        "FACT",
        "EVEN",
        "ODD",
        "QUOTIENT",
        "VALUE",
        "SUBSTITUTE",
        "REPT",
        "FIND",
        "SEARCH",
        "TEXT",
        "FIXED",
        "DOLLAR",
        "NUMBERVALUE",
        "CHAR",
        "CODE",
        "UNICHAR",
        "UNICODE",
        "EXACT",
        "PROPER",
        "CLEAN",
        "REPLACE",
        "REPLACEB",
        "LENB",
        "LEFTB",
        "RIGHTB",
        "MIDB",
        "FINDB",
        "SEARCHB",
        "YEAR",
        "MONTH",
        "DAY",
    ];
    NAMES.iter().any(|n| n.eq_ignore_ascii_case(name))
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
