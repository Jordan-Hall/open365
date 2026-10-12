//! Insert Function and Function Arguments: the function catalogue by category and the formula a
//! set of arguments produces. Pure data; the shell draws the dialog.

use open365_calc::functions::function_names;

/// Excel's function categories as the Formulas tab lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Financial,
    Logical,
    Text,
    DateTime,
    LookupReference,
    MathTrig,
    More,
}

const FINANCIAL: &[&str] = &[
    "ACCRINT",
    "AMORDEGRC",
    "AMORLINC",
    "COUPDAYBS",
    "COUPDAYS",
    "COUPDAYSNC",
    "COUPNCD",
    "COUPNUM",
    "COUPPCD",
    "CUMIPMT",
    "CUMPRINC",
    "DB",
    "DDB",
    "DISC",
    "DOLLARDE",
    "DOLLARFR",
    "DURATION",
    "EFFECT",
    "FV",
    "FVSCHEDULE",
    "INTRATE",
    "IPMT",
    "IRR",
    "ISPMT",
    "MDURATION",
    "MIRR",
    "NOMINAL",
    "NPER",
    "NPV",
    "ODDFPRICE",
    "PDURATION",
    "PMT",
    "PPMT",
    "PRICE",
    "PV",
    "RATE",
    "RECEIVED",
    "RRI",
    "SLN",
    "SYD",
    "TBILLEQ",
    "TBILLPRICE",
    "TBILLYIELD",
    "VDB",
    "XIRR",
    "XNPV",
    "YIELD",
    "YIELDDISC",
    "YIELDMAT",
];
const LOGICAL: &[&str] = &[
    "AND", "FALSE", "IF", "IFERROR", "IFNA", "IFS", "LAMBDA", "LET", "NOT", "OR", "SWITCH", "TRUE",
    "XOR",
];
const TEXT: &[&str] = &[
    "ARRAYTOTEXT",
    "ASC",
    "CHAR",
    "CLEAN",
    "CODE",
    "CONCAT",
    "CONCATENATE",
    "DOLLAR",
    "EXACT",
    "FIND",
    "FIXED",
    "LEFT",
    "LEN",
    "LOWER",
    "MID",
    "NUMBERVALUE",
    "PROPER",
    "REPLACE",
    "REPT",
    "RIGHT",
    "SEARCH",
    "SUBSTITUTE",
    "T",
    "TEXT",
    "TEXTAFTER",
    "TEXTBEFORE",
    "TEXTJOIN",
    "TEXTSPLIT",
    "TRIM",
    "UNICHAR",
    "UNICODE",
    "UPPER",
    "VALUE",
    "VALUETOTEXT",
];
const DATE_TIME: &[&str] = &[
    "DATE",
    "DATEDIF",
    "DATEVALUE",
    "DAY",
    "DAYS",
    "DAYS360",
    "EDATE",
    "EOMONTH",
    "HOUR",
    "ISOWEEKNUM",
    "MINUTE",
    "MONTH",
    "NETWORKDAYS",
    "NOW",
    "SECOND",
    "TIME",
    "TIMEVALUE",
    "TODAY",
    "WEEKDAY",
    "WEEKNUM",
    "WORKDAY",
    "YEAR",
    "YEARFRAC",
];
const LOOKUP: &[&str] = &[
    "ADDRESS",
    "AREAS",
    "CHOOSE",
    "CHOOSECOLS",
    "CHOOSEROWS",
    "COLUMN",
    "COLUMNS",
    "DROP",
    "EXPAND",
    "FILTER",
    "FORMULATEXT",
    "HLOOKUP",
    "HSTACK",
    "HYPERLINK",
    "INDEX",
    "INDIRECT",
    "LOOKUP",
    "MATCH",
    "OFFSET",
    "ROW",
    "ROWS",
    "SORT",
    "SORTBY",
    "TAKE",
    "TOCOL",
    "TOROW",
    "TRANSPOSE",
    "UNIQUE",
    "VLOOKUP",
    "VSTACK",
    "WRAPCOLS",
    "WRAPROWS",
    "XLOOKUP",
    "XMATCH",
];
const MATH: &[&str] = &[
    "ABS",
    "ACOS",
    "ACOSH",
    "ASIN",
    "ASINH",
    "ATAN",
    "ATAN2",
    "ATANH",
    "BASE",
    "CEILING",
    "COMBIN",
    "COS",
    "COSH",
    "DEGREES",
    "EVEN",
    "EXP",
    "FACT",
    "FACTDOUBLE",
    "FLOOR",
    "GCD",
    "INT",
    "LCM",
    "LN",
    "LOG",
    "LOG10",
    "MDETERM",
    "MINVERSE",
    "MMULT",
    "MOD",
    "MROUND",
    "ODD",
    "PI",
    "POWER",
    "PRODUCT",
    "QUOTIENT",
    "RADIANS",
    "RAND",
    "RANDARRAY",
    "RANDBETWEEN",
    "ROMAN",
    "ROUND",
    "ROUNDDOWN",
    "ROUNDUP",
    "SEQUENCE",
    "SIGN",
    "SIN",
    "SINH",
    "SQRT",
    "SQRTPI",
    "SUBTOTAL",
    "SUM",
    "SUMIF",
    "SUMIFS",
    "SUMPRODUCT",
    "SUMSQ",
    "TAN",
    "TANH",
    "TRUNC",
];

impl Category {
    /// The ribbon control (last id segment) that opens this category.
    #[must_use]
    pub fn from_control(tail: &str) -> Option<Self> {
        Some(match tail {
            "financial" => Self::Financial,
            "logical" => Self::Logical,
            "text" => Self::Text,
            "date-time" => Self::DateTime,
            "lookup-reference" => Self::LookupReference,
            "maths-trig" => Self::MathTrig,
            "more-functions" => Self::More,
            _ => return None,
        })
    }

    /// The category name Insert Function shows.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Financial => "Financial",
            Self::Logical => "Logical",
            Self::Text => "Text",
            Self::DateTime => "Date & Time",
            Self::LookupReference => "Lookup & Reference",
            Self::MathTrig => "Math & Trig",
            Self::More => "Statistical, Engineering, Information and more",
        }
    }

    fn list(self) -> Option<&'static [&'static str]> {
        match self {
            Self::Financial => Some(FINANCIAL),
            Self::Logical => Some(LOGICAL),
            Self::Text => Some(TEXT),
            Self::DateTime => Some(DATE_TIME),
            Self::LookupReference => Some(LOOKUP),
            Self::MathTrig => Some(MATH),
            Self::More => None,
        }
    }
}

/// The category of function `name`.
#[must_use]
pub fn category_of(name: &str) -> Category {
    let up = name.to_ascii_uppercase();
    [
        Category::Financial,
        Category::Logical,
        Category::Text,
        Category::DateTime,
        Category::LookupReference,
        Category::MathTrig,
    ]
    .into_iter()
    .find(|c| c.list().is_some_and(|l| l.contains(&up.as_str())))
    .unwrap_or(Category::More)
}

/// The implemented functions of `category`, sorted.
#[must_use]
pub fn functions_in(category: Category) -> Vec<&'static str> {
    function_names()
        .into_iter()
        .filter(|n| category_of(n) == category)
        .collect()
}

/// True when the calculation engine implements `name`.
#[must_use]
pub fn is_function(name: &str) -> bool {
    function_names()
        .iter()
        .any(|n| n.eq_ignore_ascii_case(name))
}

/// Insert Function's search box: functions whose name contains `query` (case-insensitive),
/// optionally within one category.
#[must_use]
pub fn search(query: &str, category: Option<Category>) -> Vec<&'static str> {
    let q = query.trim().to_ascii_uppercase();
    function_names()
        .into_iter()
        .filter(|n| category.is_none_or(|c| category_of(n) == c))
        .filter(|n| n.contains(&q))
        .collect()
}

/// The formula Function Arguments writes for `name` with `args` (empty trailing arguments dropped).
#[must_use]
pub fn formula_for(name: &str, args: &[&str]) -> String {
    let mut args: Vec<&str> = args.iter().map(|a| a.trim()).collect();
    while args.last().is_some_and(|a| a.is_empty()) {
        args.pop();
    }
    format!("={}({})", name.to_ascii_uppercase(), args.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_and_search() {
        assert_eq!(category_of("sum"), Category::MathTrig);
        assert_eq!(category_of("IF"), Category::Logical);
        assert!(functions_in(Category::Logical).contains(&"IF"));
        assert!(search("vlook", None).contains(&"VLOOKUP"));
        assert_eq!(formula_for("sum", &["A1:A3", ""]), "=SUM(A1:A3)");
    }
}
