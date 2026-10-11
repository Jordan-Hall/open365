//! Worksheet function registry and shared argument helpers.

mod arith;
mod complex;
mod criteria;
mod database;
mod datecalc;
mod datetime;
mod dynarray;
mod engineering;
mod finance;
mod info;
mod letters;
mod logical;
mod lookup;
mod math;
mod mathx;
mod refs;
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
    /// Implemented by the evaluator (`LET`, `LAMBDA`, `MAP`, ...): it needs the unevaluated arguments or calls
    /// back into the evaluator. Registered so the function is known, not callable through the table.
    Lazy,
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
    ("ASINH", Func::Res(mathx::asinh)),
    ("ACOSH", Func::Res(mathx::acosh)),
    ("ATANH", Func::Res(mathx::atanh)),
    ("ACOT", Func::Res(mathx::acot)),
    ("ACOTH", Func::Res(mathx::acoth)),
    ("COT", Func::Res(mathx::cot)),
    ("COTH", Func::Res(mathx::coth)),
    ("CSC", Func::Res(mathx::csc)),
    ("CSCH", Func::Res(mathx::csch)),
    ("SEC", Func::Res(mathx::sec)),
    ("SECH", Func::Res(mathx::sech)),
    ("SQRTPI", Func::Res(mathx::sqrtpi)),
    ("FACTDOUBLE", Func::Res(mathx::factdouble)),
    ("SERIESSUM", Func::Res(mathx::seriessum)),
    ("BASE", Func::Res(mathx::base)),
    ("DECIMAL", Func::Res(mathx::decimal)),
    ("ARABIC", Func::Res(mathx::arabic)),
    ("ROMAN", Func::Res(mathx::roman)),
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
    // financial
    ("PV", Func::Res(finance::pv)),
    ("FV", Func::Res(finance::fv)),
    ("PMT", Func::Res(finance::pmt)),
    ("NPER", Func::Res(finance::nper)),
    ("IPMT", Func::Res(finance::ipmt)),
    ("PPMT", Func::Res(finance::ppmt)),
    ("CUMIPMT", Func::Res(finance::cumipmt)),
    ("CUMPRINC", Func::Res(finance::cumprinc)),
    ("RATE", Func::Res(finance::rate)),
    ("NPV", Func::Res(finance::npv)),
    ("XNPV", Func::Res(finance::xnpv)),
    ("IRR", Func::Res(finance::irr)),
    ("XIRR", Func::Res(finance::xirr)),
    ("MIRR", Func::Res(finance::mirr)),
    ("SLN", Func::Res(finance::sln)),
    ("SYD", Func::Res(finance::syd)),
    ("DB", Func::Res(finance::db)),
    ("DDB", Func::Res(finance::ddb)),
    ("VDB", Func::Res(finance::vdb)),
    ("EFFECT", Func::Res(finance::effect)),
    ("NOMINAL", Func::Res(finance::nominal)),
    ("DOLLARDE", Func::Res(finance::dollarde)),
    ("DOLLARFR", Func::Res(finance::dollarfr)),
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
    ("ISLOGICAL", Func::Res(logical::islogical)),
    ("ISNONTEXT", Func::Res(logical::isnontext)),
    ("ISERR", Func::Res(logical::iserr)),
    ("IFS", Func::Arg(logical::ifs)),
    ("SWITCH", Func::Arg(logical::switch)),
    ("ISEVEN", Func::Res(info::iseven)),
    ("ISODD", Func::Res(info::isodd)),
    ("ISREF", Func::Res(info::isref)),
    ("ISFORMULA", Func::Res(info::isformula)),
    ("FORMULATEXT", Func::Res(info::formulatext)),
    ("TYPE", Func::Res(info::type_)),
    ("ERROR.TYPE", Func::Res(info::error_type)),
    ("ROWS", Func::Res(refs::rows)),
    ("COLUMNS", Func::Res(refs::columns)),
    ("ROW", Func::Arg(refs::row)),
    ("COLUMN", Func::Arg(refs::column)),
    ("ADDRESS", Func::Res(refs::address)),
    ("OFFSET", Func::Arg(refs::offset)),
    ("INDIRECT", Func::Arg(refs::indirect)),
    ("LOOKUP", Func::Res(lookup::lookup)),
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
    ("XMATCH", Func::Res(lookup::xmatch)),
    ("CHOOSE", Func::Arg(lookup::choose)),
    ("LET", Func::Lazy),
    ("LAMBDA", Func::Lazy),
    ("MAP", Func::Lazy),
    ("REDUCE", Func::Lazy),
    ("SCAN", Func::Lazy),
    ("BYROW", Func::Lazy),
    ("BYCOL", Func::Lazy),
    ("MAKEARRAY", Func::Lazy),
    ("ISOMITTED", Func::Lazy),
    // engineering
    ("BIN2DEC", Func::Res(engineering::bin2dec)),
    ("BIN2OCT", Func::Res(engineering::bin2oct)),
    ("BIN2HEX", Func::Res(engineering::bin2hex)),
    ("OCT2BIN", Func::Res(engineering::oct2bin)),
    ("OCT2DEC", Func::Res(engineering::oct2dec)),
    ("OCT2HEX", Func::Res(engineering::oct2hex)),
    ("HEX2BIN", Func::Res(engineering::hex2bin)),
    ("HEX2DEC", Func::Res(engineering::hex2dec)),
    ("HEX2OCT", Func::Res(engineering::hex2oct)),
    ("DEC2BIN", Func::Res(engineering::dec2bin)),
    ("DEC2OCT", Func::Res(engineering::dec2oct)),
    ("DEC2HEX", Func::Res(engineering::dec2hex)),
    ("BITAND", Func::Res(engineering::bitand)),
    ("BITOR", Func::Res(engineering::bitor)),
    ("BITXOR", Func::Res(engineering::bitxor)),
    ("BITLSHIFT", Func::Res(engineering::bitlshift)),
    ("BITRSHIFT", Func::Res(engineering::bitrshift)),
    ("DELTA", Func::Res(engineering::delta)),
    ("GESTEP", Func::Res(engineering::gestep)),
    ("COMPLEX", Func::Res(complex::complex)),
    ("IMREAL", Func::Res(complex::imreal)),
    ("IMAGINARY", Func::Res(complex::imaginary)),
    ("IMARGUMENT", Func::Res(complex::imargument)),
    ("IMCONJUGATE", Func::Res(complex::imconjugate)),
    ("IMEXP", Func::Res(complex::imexp)),
    ("IMLN", Func::Res(complex::imln)),
    ("IMLOG10", Func::Res(complex::imlog10)),
    ("IMLOG2", Func::Res(complex::imlog2)),
    ("IMSIN", Func::Res(complex::imsin)),
    ("IMCOS", Func::Res(complex::imcos)),
    ("IMSINH", Func::Res(complex::imsinh)),
    ("IMCOSH", Func::Res(complex::imcosh)),
    ("IMSUB", Func::Res(complex::imsub)),
    ("IMDIV", Func::Res(complex::imdiv)),
    ("IMSUM", Func::Res(complex::imsum)),
    ("IMPRODUCT", Func::Res(complex::improduct)),
    ("SEQUENCE", Func::Res(dynarray::sequence)),
    ("TRANSPOSE", Func::Res(dynarray::transpose)),
    ("SORT", Func::Res(dynarray::sort)),
    ("SORTBY", Func::Res(dynarray::sortby)),
    ("UNIQUE", Func::Res(dynarray::unique)),
    ("FILTER", Func::Res(dynarray::filter)),
    ("TAKE", Func::Res(dynarray::take)),
    ("DROP", Func::Res(dynarray::drop)),
    ("CHOOSECOLS", Func::Res(dynarray::choosecols)),
    ("CHOOSEROWS", Func::Res(dynarray::chooserows)),
    ("EXPAND", Func::Res(dynarray::expand)),
    ("HSTACK", Func::Res(dynarray::hstack)),
    ("VSTACK", Func::Res(dynarray::vstack)),
    ("TOCOL", Func::Res(dynarray::tocol)),
    ("TOROW", Func::Res(dynarray::torow)),
    ("WRAPCOLS", Func::Res(dynarray::wrapcols)),
    ("WRAPROWS", Func::Res(dynarray::wraprows)),
    ("AREAS", Func::Res(dynarray::areas)),
    ("HYPERLINK", Func::Res(dynarray::hyperlink)),
    ("DSUM", Func::Res(database::dsum)),
    ("DAVERAGE", Func::Res(database::daverage)),
    ("DCOUNT", Func::Res(database::dcount)),
    ("DCOUNTA", Func::Res(database::dcounta)),
    ("DGET", Func::Res(database::dget)),
    ("DMAX", Func::Res(database::dmax)),
    ("DMIN", Func::Res(database::dmin)),
    ("DPRODUCT", Func::Res(database::dproduct)),
    ("DSTDEV", Func::Res(database::dstdev)),
    ("DSTDEVP", Func::Res(database::dstdevp)),
    ("DVAR", Func::Res(database::dvar)),
    ("DVARP", Func::Res(database::dvarp)),
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
    } else if name.eq_ignore_ascii_case("XLOOKUP") || name.eq_ignore_ascii_case("LOOKUP") {
        index == 1 || index == 2
    } else if name.eq_ignore_ascii_case("XMATCH") {
        index == 1
    } else if name.eq_ignore_ascii_case("ROWS") || name.eq_ignore_ascii_case("COLUMNS") {
        true
    } else {
        [
            "SUMPRODUCT",
            "ARRAYTOTEXT",
            "SORT",
            "SORTBY",
            "UNIQUE",
            "FILTER",
            "TAKE",
            "DROP",
            "CHOOSECOLS",
            "CHOOSEROWS",
            "EXPAND",
            "HSTACK",
            "VSTACK",
            "TOCOL",
            "TOROW",
            "WRAPCOLS",
            "WRAPROWS",
            "DSUM",
            "DAVERAGE",
            "DCOUNT",
            "DCOUNTA",
            "DGET",
            "DMAX",
            "DMIN",
            "DPRODUCT",
            "DSTDEV",
            "DSTDEVP",
            "DVAR",
            "DVARP",
        ]
        .iter()
        .any(|f| name.eq_ignore_ascii_case(f))
    }
}

/// Functions that take a reference: a literal reference to a missing sheet is still a reference for them.
pub(crate) fn takes_reference(name: &str) -> bool {
    [
        "ROW",
        "COLUMN",
        "ROWS",
        "COLUMNS",
        "ISREF",
        "ISFORMULA",
        "FORMULATEXT",
        "AREAS",
    ]
    .iter()
    .any(|n| n.eq_ignore_ascii_case(name))
}

/// Functions that return a whole array for a multi-cell reference only where Excel evaluates arrays.
pub(crate) fn returns_array_in_array_context(name: &str) -> bool {
    name.eq_ignore_ascii_case("ROW") || name.eq_ignore_ascii_case("COLUMN")
}

/// How a function applies to arrays passed for its scalar parameters: once per element, with `1xN`/`Nx1`
/// operands broadcast and mismatched sizes padded with `#N/A`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Lifting {
    /// Lift only where Excel evaluates array parameters (inside `SUMPRODUCT`, `INDEX`, ...). Elsewhere the
    /// function takes the first element of an array argument (`IFERROR`).
    pub array_only: bool,
    /// The parameters whose arrays make the call lift, when that is fewer than the scalar ones (`IF`,
    /// `CHOOSE`, `IFS`): with a scalar condition the chosen branch is returned whole.
    pub trigger: Option<fn(usize) -> bool>,
    /// Whether multi-cell ranges are expanded where Excel evaluates array parameters. False for functions
    /// that lift array values but take the implicit intersection of a range (`IF`, `IFS`, `ISEVEN`).
    pub expand_ranges: bool,
    /// Whether parameter `i` is a scalar parameter (lifted); the others (ranges, arrays) pass through.
    pub scalar_param: fn(usize) -> bool,
}

fn all_params(_: usize) -> bool {
    true
}
fn only_second(i: usize) -> bool {
    i == 1
}
fn not_second(i: usize) -> bool {
    i != 1
}
fn only_first(i: usize) -> bool {
    i == 0
}
fn first_two(i: usize) -> bool {
    i < 2
}
fn first_three(i: usize) -> bool {
    i < 3
}
fn criteria_pairs(i: usize) -> bool {
    i >= 2 && i.is_multiple_of(2)
}
fn even_params(i: usize) -> bool {
    i.is_multiple_of(2)
}
fn criteria_odd(i: usize) -> bool {
    i % 2 == 1
}

const fn lift(scalar_param: fn(usize) -> bool) -> Lifting {
    Lifting {
        array_only: false,
        trigger: None,
        expand_ranges: true,
        scalar_param,
    }
}

const ALWAYS: Lifting = lift(all_params);

static LIFTING: &[(&str, Lifting)] = &[
    ("PV", ALWAYS),
    ("FV", ALWAYS),
    ("PMT", ALWAYS),
    ("NPER", ALWAYS),
    ("IPMT", ALWAYS),
    ("PPMT", ALWAYS),
    ("CUMIPMT", ALWAYS),
    ("CUMPRINC", ALWAYS),
    ("RATE", ALWAYS),
    ("SLN", ALWAYS),
    ("SYD", ALWAYS),
    ("DB", ALWAYS),
    ("DDB", ALWAYS),
    ("VDB", ALWAYS),
    ("EFFECT", ALWAYS),
    ("NOMINAL", ALWAYS),
    ("DOLLARDE", ALWAYS),
    ("DOLLARFR", ALWAYS),
    ("ABS", ALWAYS),
    ("INT", ALWAYS),
    ("SQRT", ALWAYS),
    ("ISNUMBER", ALWAYS),
    ("ISTEXT", ALWAYS),
    ("ISBLANK", ALWAYS),
    ("ISERROR", ALWAYS),
    ("ISNA", ALWAYS),
    ("NOT", ALWAYS),
    ("LEN", ALWAYS),
    ("LEFT", ALWAYS),
    ("RIGHT", ALWAYS),
    ("MID", ALWAYS),
    ("UPPER", ALWAYS),
    ("LOWER", ALWAYS),
    ("TRIM", ALWAYS),
    ("ROUND", ALWAYS),
    ("ROUNDUP", ALWAYS),
    ("ROUNDDOWN", ALWAYS),
    ("MOD", ALWAYS),
    ("POWER", ALWAYS),
    ("SIGN", ALWAYS),
    ("EXP", ALWAYS),
    ("LN", ALWAYS),
    ("LOG", ALWAYS),
    ("LOG10", ALWAYS),
    ("SIN", ALWAYS),
    ("COS", ALWAYS),
    ("TAN", ALWAYS),
    ("ASIN", ALWAYS),
    ("ACOS", ALWAYS),
    ("ATAN", ALWAYS),
    ("ATAN2", ALWAYS),
    ("SINH", ALWAYS),
    ("COSH", ALWAYS),
    ("TANH", ALWAYS),
    ("DEGREES", ALWAYS),
    ("RADIANS", ALWAYS),
    ("CEILING", ALWAYS),
    ("FLOOR", ALWAYS),
    ("TRUNC", ALWAYS),
    ("MROUND", ALWAYS),
    ("FACT", ALWAYS),
    ("ASINH", ALWAYS),
    ("ACOSH", ALWAYS),
    ("ATANH", ALWAYS),
    ("ACOT", ALWAYS),
    ("ACOTH", ALWAYS),
    ("COT", ALWAYS),
    ("COTH", ALWAYS),
    ("CSC", ALWAYS),
    ("CSCH", ALWAYS),
    ("SEC", ALWAYS),
    ("SECH", ALWAYS),
    ("SQRTPI", ALWAYS),
    ("FACTDOUBLE", ALWAYS),
    ("BASE", ALWAYS),
    ("DECIMAL", ALWAYS),
    ("ARABIC", ALWAYS),
    ("ROMAN", ALWAYS),
    ("EVEN", ALWAYS),
    ("ODD", ALWAYS),
    ("QUOTIENT", ALWAYS),
    ("VALUE", ALWAYS),
    ("SUBSTITUTE", ALWAYS),
    ("REPT", ALWAYS),
    ("FIND", ALWAYS),
    ("SEARCH", ALWAYS),
    ("TEXT", ALWAYS),
    ("FIXED", ALWAYS),
    ("DOLLAR", ALWAYS),
    ("NUMBERVALUE", ALWAYS),
    ("CHAR", ALWAYS),
    ("CODE", ALWAYS),
    ("UNICHAR", ALWAYS),
    ("UNICODE", ALWAYS),
    ("EXACT", ALWAYS),
    ("PROPER", ALWAYS),
    ("CLEAN", ALWAYS),
    ("REPLACE", ALWAYS),
    ("REPLACEB", ALWAYS),
    ("LENB", ALWAYS),
    ("LEFTB", ALWAYS),
    ("RIGHTB", ALWAYS),
    ("MIDB", ALWAYS),
    ("FINDB", ALWAYS),
    ("SEARCHB", ALWAYS),
    ("YEAR", ALWAYS),
    ("MONTH", ALWAYS),
    ("DAY", ALWAYS),
    (
        "IF",
        Lifting {
            trigger: Some(only_first),
            expand_ranges: false,
            ..ALWAYS
        },
    ),
    (
        "CHOOSE",
        Lifting {
            trigger: Some(only_first),
            expand_ranges: false,
            ..ALWAYS
        },
    ),
    (
        "IFS",
        Lifting {
            trigger: Some(even_params),
            expand_ranges: false,
            ..ALWAYS
        },
    ),
    (
        "SWITCH",
        Lifting {
            trigger: Some(only_first),
            expand_ranges: false,
            ..ALWAYS
        },
    ),
    (
        "ISEVEN",
        Lifting {
            expand_ranges: false,
            ..ALWAYS
        },
    ),
    (
        "ISODD",
        Lifting {
            expand_ranges: false,
            ..ALWAYS
        },
    ),
    (
        "N",
        Lifting {
            expand_ranges: false,
            ..ALWAYS
        },
    ),
    ("LOOKUP", lift(only_first)),
    (
        "IFERROR",
        Lifting {
            array_only: true,
            expand_ranges: false,
            ..ALWAYS
        },
    ),
    (
        "IFNA",
        Lifting {
            array_only: true,
            expand_ranges: false,
            ..ALWAYS
        },
    ),
    ("MATCH", lift(not_second)),
    ("XMATCH", lift(only_first)),
    ("SUMIF", lift(only_second)),
    ("COUNTIF", lift(only_second)),
    ("AVERAGEIF", lift(only_second)),
    ("SUMIFS", lift(criteria_pairs)),
    ("AVERAGEIFS", lift(criteria_pairs)),
    ("MAXIFS", lift(criteria_pairs)),
    ("MINIFS", lift(criteria_pairs)),
    ("COUNTIFS", lift(criteria_odd)),
    ("LARGE", lift(only_second)),
    ("SMALL", lift(only_second)),
    ("PERCENTILE", lift(only_second)),
    ("PERCENTILE.INC", lift(only_second)),
    ("PERCENTILE.EXC", lift(only_second)),
    ("QUARTILE", lift(only_second)),
    ("QUARTILE.INC", lift(only_second)),
    ("QUARTILE.EXC", lift(only_second)),
    ("RANK", lift(not_second)),
    ("RANK.EQ", lift(not_second)),
    ("RANK.AVG", lift(not_second)),
    ("SUBTOTAL", lift(only_first)),
    ("AGGREGATE", lift(first_two)),
    ("NETWORKDAYS", lift(first_two)),
    ("WORKDAY", lift(first_two)),
    ("NETWORKDAYS.INTL", lift(first_three)),
    ("WORKDAY.INTL", lift(first_three)),
    ("ISLOGICAL", ALWAYS),
    ("ISNONTEXT", ALWAYS),
    ("ISERR", ALWAYS),
    ("ERROR.TYPE", ALWAYS),
    ("DATE", ALWAYS),
    ("WEEKDAY", ALWAYS),
    ("EDATE", ALWAYS),
    ("EOMONTH", ALWAYS),
    ("DAYS", ALWAYS),
    ("HOUR", ALWAYS),
    ("MINUTE", ALWAYS),
    ("SECOND", ALWAYS),
    ("TIME", ALWAYS),
    ("DATEVALUE", ALWAYS),
    ("TIMEVALUE", ALWAYS),
    ("DATEDIF", ALWAYS),
    ("DAYS360", ALWAYS),
    ("YEARFRAC", ALWAYS),
    ("WEEKNUM", ALWAYS),
    ("ISOWEEKNUM", ALWAYS),
    ("COMBIN", ALWAYS),
    ("PERMUT", ALWAYS),
    ("CEILING.MATH", ALWAYS),
    ("FLOOR.MATH", ALWAYS),
    ("T", ALWAYS),
    ("CONCATENATE", ALWAYS),
    ("TEXTBEFORE", lift(only_first)),
    ("TEXTAFTER", lift(only_first)),
    ("VALUETOTEXT", ALWAYS),
    ("ADDRESS", ALWAYS),
    ("COMPLEX", ALWAYS),
    ("IMREAL", ALWAYS),
    ("IMAGINARY", ALWAYS),
    ("IMARGUMENT", ALWAYS),
    ("IMCONJUGATE", ALWAYS),
    ("IMEXP", ALWAYS),
    ("IMLN", ALWAYS),
    ("IMLOG10", ALWAYS),
    ("IMLOG2", ALWAYS),
    ("IMSIN", ALWAYS),
    ("IMCOS", ALWAYS),
    ("IMSINH", ALWAYS),
    ("IMCOSH", ALWAYS),
    ("IMSUB", ALWAYS),
    ("IMDIV", ALWAYS),
    ("BIN2DEC", ALWAYS),
    ("BIN2OCT", ALWAYS),
    ("BIN2HEX", ALWAYS),
    ("OCT2BIN", ALWAYS),
    ("OCT2DEC", ALWAYS),
    ("OCT2HEX", ALWAYS),
    ("HEX2BIN", ALWAYS),
    ("HEX2DEC", ALWAYS),
    ("HEX2OCT", ALWAYS),
    ("DEC2BIN", ALWAYS),
    ("DEC2OCT", ALWAYS),
    ("DEC2HEX", ALWAYS),
    ("BITAND", ALWAYS),
    ("BITOR", ALWAYS),
    ("BITXOR", ALWAYS),
    ("BITLSHIFT", ALWAYS),
    ("BITRSHIFT", ALWAYS),
    ("DELTA", ALWAYS),
    ("GESTEP", ALWAYS),
];

fn lifting_table() -> &'static HashMap<String, Lifting> {
    static MAP: OnceLock<HashMap<String, Lifting>> = OnceLock::new();
    MAP.get_or_init(|| LIFTING.iter().map(|(n, l)| ((*n).to_owned(), *l)).collect())
}

/// `OFFSET` and `INDIRECT` given an array argument return an array of references, one per element.
pub(crate) fn ref_lifting(name: &str) -> Option<&'static Lifting> {
    static OFFSET: Lifting = Lifting {
        array_only: false,
        trigger: None,
        expand_ranges: false,
        scalar_param: |i| i >= 1,
    };
    static INDIRECT: Lifting = Lifting {
        array_only: false,
        trigger: None,
        expand_ranges: false,
        scalar_param: |_| true,
    };
    if name.eq_ignore_ascii_case("OFFSET") {
        Some(&OFFSET)
    } else if name.eq_ignore_ascii_case("INDIRECT") {
        Some(&INDIRECT)
    } else {
        None
    }
}

/// What a function does with an array of references among its arguments (measured, corpus group `refarr`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefsMode {
    /// The function reads the array itself.
    Native,
    /// Runs once per element (`COUNTIF`, `SUMIF`, ...).
    LiftOver,
    /// The elements form a multi-area reference (`SUBTOTAL`).
    Union,
    /// `#VALUE!`
    Value,
    /// `#VALUE!` where Excel evaluates arrays, the first element elsewhere (scalar functions such as `ABS`).
    ValueLifted,
    /// `#N/A`
    NA,
    /// `TYPE` is 64, the type of an array.
    Type,
    /// `ISREF` is FALSE.
    NotRef,
    /// `ISNUMBER`, `ISERROR`, ...: an array of FALSE.
    FalseArray,
    /// Everything else sees the first element.
    First,
}

/// How `name` treats an array of references.
pub(crate) fn refs_mode(name: &str) -> RefsMode {
    let is = |list: &[&str]| list.iter().any(|f| f.eq_ignore_ascii_case(name));
    if is(&[
        "SUMPRODUCT",
        "N",
        "T",
        "SUM",
        "MAX",
        "MIN",
        "AVERAGE",
        "PRODUCT",
        "COUNT",
        "COUNTA",
        "COUNTBLANK",
        "ROWS",
        "COLUMNS",
        "STDEV",
        "STDEV.S",
        "STDEV.P",
        "STDEVP",
        "VAR",
        "VAR.S",
        "VAR.P",
        "VARP",
    ]) {
        RefsMode::Native
    } else if is(&["COUNTIF", "COUNTIFS", "SUMIF", "AVERAGEIF"]) {
        RefsMode::LiftOver
    } else if is(&["SUBTOTAL"]) {
        RefsMode::Union
    } else if is(&["TYPE"]) {
        RefsMode::Type
    } else if is(&["ISREF"]) {
        RefsMode::NotRef
    } else if is(&[
        "ISNUMBER",
        "ISERROR",
        "ISERR",
        "ISBLANK",
        "ISTEXT",
        "ISNA",
        "ISLOGICAL",
        "ISNONTEXT",
    ]) {
        RefsMode::FalseArray
    } else if is(&["AND", "OR", "MATCH"]) {
        RefsMode::NA
    } else if is(&[
        "CONCAT",
        "TEXTJOIN",
        "LARGE",
        "SMALL",
        "MEDIAN",
        "AGGREGATE",
    ]) {
        RefsMode::Value
    } else if lifting(name).is_some()
        && !is(&[
            "IF", "IFS", "IFERROR", "IFNA", "CHOOSE", "SWITCH", "INDEX", "LOOKUP", "ISEVEN",
            "ISODD",
        ])
    {
        RefsMode::ValueLifted
    } else {
        RefsMode::First
    }
}

/// Functions that run once per element when a range parameter is an array of references.
pub(crate) fn lifts_over_refs(name: &str) -> bool {
    refs_mode(name) == RefsMode::LiftOver
}

/// How `name` lifts over arrays, if it is a function with scalar parameters.
pub(crate) fn lifting(name: &str) -> Option<&'static Lifting> {
    let map = lifting_table();
    map.get(name)
        .or_else(|| map.get(name.to_ascii_uppercase().as_str()))
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
