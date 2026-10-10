//! End-to-end tests of operators and worksheet functions through the engine.

use crate::ErrorKind::{DivZero, NA, Name, Null, Num, Ref, Value as ValueErr};
use crate::Value;
use crate::functions::function_names;
use crate::test_support::{
    CLOCK_SECONDS, assert_close, b, engine_with, err, eval, eval_full, eval_in, eval_with, n, t,
};

/// A1:A6 = 1, 2, "x", TRUE, (blank), 4
fn mixed() -> Vec<(&'static str, Value)> {
    vec![
        ("A1", n(1.0)),
        ("A2", n(2.0)),
        ("A3", t("x")),
        ("A4", b(true)),
        ("A6", n(4.0)),
    ]
}

/// Text in B1:B6, numbers in C1:C6.
fn fruit() -> Vec<(&'static str, Value)> {
    vec![
        ("B1", t("apple")),
        ("B2", t("Apricot")),
        ("B3", t("banana")),
        ("B4", t("apple")),
        ("B5", n(5.0)),
        ("B6", n(10.0)),
        ("C1", n(1.0)),
        ("C2", n(2.0)),
        ("C3", n(3.0)),
        ("C4", n(4.0)),
        ("C5", n(5.0)),
        ("C6", n(6.0)),
    ]
}

/// A1:C4 lookup table, D1:D4 descending, E1:H2 horizontal table.
fn table() -> Vec<(&'static str, Value)> {
    vec![
        ("A1", n(1.0)),
        ("A2", n(2.0)),
        ("A3", n(3.0)),
        ("A4", n(4.0)),
        ("B1", t("one")),
        ("B2", t("two")),
        ("B3", t("three")),
        ("B4", t("four")),
        ("C1", n(10.0)),
        ("C2", n(20.0)),
        ("C3", n(30.0)),
        ("C4", n(40.0)),
        ("D1", n(40.0)),
        ("D2", n(30.0)),
        ("D3", n(20.0)),
        ("D4", n(10.0)),
        ("E1", n(1.0)),
        ("F1", n(2.0)),
        ("G1", n(3.0)),
        ("H1", n(4.0)),
        ("E2", t("a")),
        ("F2", t("b")),
        ("G2", t("c")),
        ("H2", t("d")),
    ]
}

#[test]
fn every_required_function_is_registered() {
    let required = [
        "SUM",
        "AVERAGE",
        "MIN",
        "MAX",
        "COUNT",
        "COUNTA",
        "COUNTBLANK",
        "PRODUCT",
        "ROUND",
        "ROUNDUP",
        "ROUNDDOWN",
        "INT",
        "MOD",
        "ABS",
        "SQRT",
        "POWER",
        "IF",
        "IFERROR",
        "IFNA",
        "AND",
        "OR",
        "NOT",
        "XOR",
        "ISBLANK",
        "ISNUMBER",
        "ISTEXT",
        "ISERROR",
        "ISNA",
        "LEN",
        "LEFT",
        "RIGHT",
        "MID",
        "UPPER",
        "LOWER",
        "TRIM",
        "CONCATENATE",
        "CONCAT",
        "TEXTJOIN",
        "FIND",
        "SEARCH",
        "SUBSTITUTE",
        "REPT",
        "VALUE",
        "SUMIF",
        "COUNTIF",
        "AVERAGEIF",
        "SUMIFS",
        "COUNTIFS",
        "VLOOKUP",
        "HLOOKUP",
        "INDEX",
        "MATCH",
        "XLOOKUP",
        "CHOOSE",
        "DATE",
        "YEAR",
        "MONTH",
        "DAY",
        "TODAY",
        "NOW",
        "PI",
    ];
    let names = function_names();
    for name in required {
        assert!(names.contains(&name), "{name} is not registered");
    }
}

#[test]
fn function_names_are_case_insensitive() {
    assert_eq!(eval("sum(1,2)"), n(3.0));
    assert_eq!(eval("Sum(1,2)"), n(3.0));
    assert_eq!(eval("_xlfn.CONCAT(\"a\",\"b\")"), t("ab"));
    assert_eq!(eval("_xlfn.XLOOKUP(1,{1,2},{3,4})"), n(3.0));
}

// ---- operators -----------------------------------------------------------------------

#[test]
fn arithmetic_and_precedence() {
    assert_eq!(eval("1+2*3"), n(7.0));
    assert_eq!(eval("(1+2)*3"), n(9.0));
    assert_eq!(eval("10-4-3"), n(3.0));
    assert_eq!(eval("2^3^2"), n(64.0));
    assert_eq!(eval("-2^2"), n(4.0));
    assert_eq!(eval("-(2^2)"), n(-4.0));
    assert_eq!(eval("2^-1"), n(0.5));
    assert_eq!(eval("10/4"), n(2.5));
    assert_eq!(eval("5%"), n(0.05));
    assert_eq!(eval("-50%"), n(-0.5));
    assert_eq!(eval("50%*2"), n(1.0));
    assert_close(&eval("2+3%"), 2.03);
    assert_eq!(eval("1+1&1"), t("21"));
    assert_eq!(eval("2*3&\"x\""), t("6x"));
    assert_close(&eval("2^0.5"), 2f64.sqrt());
}

#[test]
fn arithmetic_errors() {
    assert_eq!(eval("1/0"), err(DivZero));
    assert_eq!(eval("0/0"), err(DivZero));
    assert_eq!(eval("0^0"), err(Num));
    assert_eq!(eval("0^-1"), err(DivZero));
    assert_eq!(
        eval("(-8)^(1/3)"),
        n(-1.999_999_999_999_999_8),
        "odd root, as Excel"
    );
    assert_eq!(eval("(-8)^0.5"), err(Num));
    assert_eq!(eval("10^308*10"), err(Num));
    assert_eq!(eval("\"abc\"+1"), err(ValueErr));
    assert_eq!(eval("-\"abc\""), err(ValueErr));
    assert_eq!(eval("\"abc\"%"), err(ValueErr));
}

#[test]
fn error_propagation_leftmost_wins() {
    assert_eq!(eval("#N/A+#DIV/0!"), err(NA));
    assert_eq!(eval("#DIV/0!+#N/A"), err(DivZero));
    assert_eq!(eval("1/0+#N/A"), err(DivZero));
    assert_eq!(eval("#N/A&\"x\""), err(NA));
    assert_eq!(eval("#REF!=1"), err(Ref));
    assert_eq!(eval("1=#NAME?"), err(Name));
    assert_eq!(eval("-#NUM!"), err(Num));
    assert_eq!(eval("SUM(1,#NULL!)"), err(Null));
}

#[test]
fn coercion_in_arithmetic() {
    assert_eq!(eval("\"5\"+1"), n(6.0));
    assert_eq!(eval("TRUE+1"), n(2.0));
    assert_eq!(eval("\"1,000\"*2"), n(2000.0));
    assert_eq!(eval("A1+1"), n(1.0), "blank is zero");
    assert_eq!(eval("+\"abc\""), t("abc"), "unary plus does not coerce");
    assert_eq!(eval("\"50%\"*2"), n(1.0));
}

#[test]
fn concatenation_uses_general_number_format() {
    assert_eq!(eval("0.1+0.2&\"\""), t("0.3"));
    assert_eq!(eval("1/3&\"\""), t("0.333333333333333"));
    assert_eq!(eval("1&2"), t("12"));
    assert_eq!(eval("TRUE&\"x\""), t("TRUEx"));
    assert_eq!(eval("A1&\"x\""), t("x"));
    assert_eq!(eval("1000000000000000&\"\""), t("1000000000000000"));
    assert_eq!(eval("1E20&\"\""), t("1E+20"));
    assert_eq!(eval("1E-10&\"\""), t("0.0000000001"));
    assert_eq!(eval("123456789012345&\"\""), t("123456789012345"));
    assert_eq!(eval("-2.5&\"\""), t("-2.5"));
}

#[test]
fn float_noise_is_hidden_like_excel() {
    assert_eq!(eval("0.1+0.2-0.3"), n(0.0));
    assert_eq!(eval("0.1+0.2=0.3"), b(true));
    assert_eq!(eval("1-0.9-0.1"), n(0.0));
}

#[test]
fn comparison_ordering() {
    assert_eq!(eval("1<2"), b(true));
    assert_eq!(eval("2<=2"), b(true));
    assert_eq!(eval("3>=4"), b(false));
    assert_eq!(eval("2<>3"), b(true));
    assert_eq!(eval("\"a\"=\"A\""), b(true));
    assert_eq!(eval("\"a\"<\"B\""), b(true));
    assert_eq!(eval("\"abc\"<\"abd\""), b(true));
    assert_eq!(eval("1<\"a\""), b(true), "numbers sort before text");
    assert_eq!(eval("\"a\"<TRUE"), b(true), "text sorts before booleans");
    assert_eq!(eval("TRUE>999999"), b(true));
    assert_eq!(eval("TRUE=1"), b(false));
    assert_eq!(eval("A1=0"), b(true));
    assert_eq!(eval("A1=\"\""), b(true));
    assert_eq!(eval("A1=FALSE"), b(true));
    assert_eq!(eval("\"1\"=1"), b(false));
}

#[test]
fn missing_names_functions_and_sheets() {
    assert_eq!(eval("FOO(1)"), err(Name));
    assert_eq!(eval("undefined_name+1"), err(Name));
    assert_eq!(eval("NoSuchSheet!A1"), err(Ref));
}

#[test]
fn array_constants_and_lifting() {
    assert_eq!(eval("SUM({1,2,3}*2)"), n(12.0));
    assert_eq!(eval("SUM({1,2;3,4})"), n(10.0));
    assert_eq!(eval("SUM({1,2,3}+{10;20})"), n(102.0));
    assert_eq!(eval("SUM(-{1,2})"), n(-3.0));
    assert_eq!(eval("SUM({1,\"a\",TRUE})"), n(1.0));
    assert_eq!(eval("{1,2;3,4}"), n(1.0), "cell shows the top-left element");
    assert_eq!(
        eval_full(&[], "{1,2;3,4}"),
        Value::Array(vec![vec![n(1.0), n(2.0)], vec![n(3.0), n(4.0)]])
    );
    let data = [("A1", n(1.0)), ("A2", n(2.0)), ("A3", n(3.0))];
    // A non-array formula intersects ranges inside operators; only INDEX/XLOOKUP array parameters see whole ranges.
    assert_eq!(eval_with(&data, "SUM(A1:A3*2)"), err(ValueErr));
    assert_eq!(eval_with(&data, "SUM(INDEX(A1:A3*2,0))"), n(12.0));
    assert_eq!(eval_with(&data, "SUM(INDEX((A1:A3>1)*1,0))"), n(2.0));
    assert_eq!(eval_with(&data, "SUM(IF(A1:A3>1,A1:A3,0))"), err(ValueErr));
}

#[test]
fn reference_operators() {
    let data = [
        ("A1", n(1.0)),
        ("B2", n(5.0)),
        ("B3", n(7.0)),
        ("C4", n(100.0)),
    ];
    assert_eq!(eval_with(&data, "SUM(A1:B3 B2:C4)"), n(12.0));
    assert_eq!(eval_with(&data, "SUM((A1,B2,B3))"), n(13.0));
    assert_eq!(eval_with(&data, "SUM((A1:A1,C4))"), n(101.0));
    assert_eq!(eval_with(&data, "SUM(A1:A1 C4:C4)"), err(Null));
    assert_eq!(
        eval_with(&data, "A1:B3"),
        err(ValueErr),
        "no implicit intersection here"
    );
}

#[test]
fn scalar_parameters_intersect_with_the_formula_cell() {
    let mut e = engine_with(&[("A5", t("hello")), ("A6", t("hi"))]);
    // The formula in B6 reads A5:A9: same row 6 intersects A6.
    e.set_formula("Sheet1", 6, 2, "LEN(A5:A9)").unwrap();
    // A formula in a row outside the range cannot intersect.
    e.set_formula("Sheet1", 20, 2, "LEN(A5:A9)").unwrap();
    e.recalc_dirty();
    assert_eq!(e.get("Sheet1", 6, 2), n(2.0));
    assert_eq!(e.get("Sheet1", 20, 2), err(ValueErr));
}

// ---- aggregation ---------------------------------------------------------------------

#[test]
fn sum_distinguishes_direct_and_range_arguments() {
    let d = mixed();
    assert_eq!(eval_with(&d, "SUM(A1:A6)"), n(7.0));
    assert_eq!(eval_with(&d, "SUM(A1,A2)"), n(3.0));
    assert_eq!(
        eval_with(&d, "SUM(A3)"),
        n(0.0),
        "text in a reference is ignored"
    );
    assert_eq!(
        eval_with(&d, "SUM(A4)"),
        n(0.0),
        "booleans in a reference are ignored"
    );
    assert_eq!(
        eval("SUM(\"5\",1)"),
        n(6.0),
        "numeric text literal is coerced"
    );
    assert_eq!(eval("SUM(TRUE,1)"), n(2.0), "boolean literal is coerced");
    assert_eq!(eval("SUM(\"x\")"), err(ValueErr));
    assert_eq!(eval("SUM(1,)"), n(1.0));
    assert_eq!(eval("SUM(A1:A3)"), n(0.0));
}

#[test]
fn sum_propagates_errors_from_ranges() {
    let d = [("B1", err(DivZero)), ("B2", n(1.0))];
    assert_eq!(eval_with(&d, "SUM(B1:B2)"), err(DivZero));
    assert_eq!(eval_with(&d, "SUM(B2,B1)"), err(DivZero));
    assert_eq!(eval_with(&d, "MAX(B1:B2)"), err(DivZero));
}

#[test]
fn average_min_max_product() {
    let d = mixed();
    assert_close(&eval_with(&d, "AVERAGE(A1:A6)"), 7.0 / 3.0);
    assert_eq!(eval_with(&d, "AVERAGE(A3:A5)"), err(DivZero));
    assert_eq!(eval("AVERAGE(\"a\")"), err(ValueErr));
    assert_eq!(eval("AVERAGE(1,2,\"3\")"), n(2.0));
    assert_eq!(eval_with(&d, "MIN(A1:A6)"), n(1.0));
    assert_eq!(eval_with(&d, "MAX(A1:A6)"), n(4.0));
    assert_eq!(eval_with(&d, "MIN(A3:A5)"), n(0.0));
    assert_eq!(eval_with(&d, "MAX(A3:A5)"), n(0.0));
    assert_eq!(eval("MAX(TRUE,0)"), n(1.0));
    assert_eq!(eval("MIN(-1,-5,3)"), n(-5.0));
    assert_eq!(eval_with(&d, "PRODUCT(A1,A2,A6)"), n(8.0));
    assert_eq!(eval_with(&d, "PRODUCT(A1:A6)"), n(8.0));
    assert_eq!(eval_with(&d, "PRODUCT(A3:A5)"), n(0.0));
}

#[test]
fn counting_functions() {
    let d = mixed();
    assert_eq!(eval_with(&d, "COUNT(A1:A6)"), n(3.0));
    assert_eq!(eval_with(&d, "COUNT(A1:A6,1,\"2\",\"x\",TRUE)"), n(6.0));
    assert_eq!(eval_with(&d, "COUNTA(A1:A6)"), n(5.0));
    assert_eq!(eval_with(&d, "COUNTBLANK(A1:A6)"), n(1.0));
    assert_eq!(eval_with(&d, "COUNT(1/0)"), n(0.0));
    assert_eq!(eval("COUNTA(1/0,\"\",2)"), n(3.0));
    assert_eq!(eval("COUNTBLANK(5)"), err(ValueErr));
    let with_empty_text = [("A1", t("")), ("A2", n(1.0))];
    assert_eq!(eval_with(&with_empty_text, "COUNTBLANK(A1:A3)"), n(2.0));
    assert_eq!(eval_with(&with_empty_text, "COUNTA(A1:A3)"), n(2.0));
}

#[test]
fn whole_column_aggregates_only_scan_used_rows() {
    let d = [
        ("A1", n(1.0)),
        ("A2", n(2.0)),
        ("A50", n(3.0)),
        ("B1", n(100.0)),
    ];
    assert_eq!(eval_with(&d, "SUM(A:A)"), n(6.0));
    assert_eq!(eval_with(&d, "COUNT(A:A)"), n(3.0));
    assert_eq!(eval_with(&d, "SUM(A:B)"), n(106.0));
    assert_eq!(eval_with(&d, "SUM(1:1)"), n(101.0));
    assert_eq!(eval_with(&d, "COUNTBLANK(A:A)"), n(1_048_576.0 - 3.0));
}

// ---- rounding and math ---------------------------------------------------------------

#[test]
fn rounding_functions() {
    assert_eq!(eval("ROUND(2.5,0)"), n(3.0));
    assert_eq!(eval("ROUND(-2.5,0)"), n(-3.0));
    assert_eq!(eval("ROUND(0.5,0)"), n(1.0));
    assert_eq!(eval("ROUND(0.4,0)"), n(0.0));
    assert_eq!(eval("ROUND(2.675,2)"), n(2.68));
    assert_eq!(eval("ROUND(1.005,2)"), n(1.01));
    assert_eq!(eval("ROUND(1234.5678,-2)"), n(1200.0));
    assert_eq!(eval("ROUND(1250,-2)"), n(1300.0));
    assert_eq!(eval("ROUND(1.23456,3)"), n(1.235));
    assert_eq!(eval("ROUND(5,1)"), n(5.0));
    assert_eq!(eval("ROUND(0.001,1)"), n(0.0));
    assert_eq!(eval("ROUND(7.9,0.9)"), n(8.0), "digits are truncated");
    assert_eq!(eval("ROUND(\"x\",1)"), err(ValueErr));
    assert_eq!(eval("ROUNDUP(3.01,1)"), n(3.1));
    assert_eq!(eval("ROUNDUP(-3.01,1)"), n(-3.1));
    assert_eq!(eval("ROUNDUP(3,1)"), n(3.0));
    assert_eq!(eval("ROUNDUP(0.001,1)"), n(0.1));
    assert_eq!(eval("ROUNDUP(1234,-2)"), n(1300.0));
    assert_eq!(eval("ROUNDDOWN(3.99,1)"), n(3.9));
    assert_eq!(eval("ROUNDDOWN(-3.99,1)"), n(-3.9));
    assert_eq!(eval("ROUNDDOWN(1299,-2)"), n(1200.0));
    assert_eq!(eval("ROUNDDOWN(0.99,0)"), n(0.0));
    assert_eq!(eval("INT(-1.5)"), n(-2.0));
    assert_eq!(eval("INT(1.9)"), n(1.0));
    assert_eq!(eval("INT(5)"), n(5.0));
}

#[test]
fn modulo_abs_sqrt_power_pi() {
    assert_eq!(eval("MOD(5,3)"), n(2.0));
    assert_eq!(eval("MOD(-5,3)"), n(1.0));
    assert_eq!(eval("MOD(5,-3)"), n(-1.0));
    assert_eq!(eval("MOD(5.5,2)"), n(1.5));
    assert_eq!(eval("MOD(6,3)"), n(0.0));
    assert_eq!(eval("MOD(5,0)"), err(DivZero));
    assert_eq!(eval("ABS(-3)"), n(3.0));
    assert_eq!(eval("ABS(\"-3\")"), n(3.0));
    assert_eq!(eval("SQRT(16)"), n(4.0));
    assert_eq!(eval("SQRT(-1)"), err(Num));
    assert_eq!(eval("POWER(2,10)"), n(1024.0));
    assert_eq!(eval("POWER(0,0)"), err(Num));
    assert_eq!(eval("POWER(4,0.5)"), n(2.0));
    assert_close(&eval("PI()"), std::f64::consts::PI);
    assert_eq!(eval("ABS(1,2)"), err(ValueErr), "wrong argument count");
}

// ---- logical -------------------------------------------------------------------------

#[test]
fn if_function() {
    assert_eq!(eval("IF(TRUE,1,2)"), n(1.0));
    assert_eq!(eval("IF(FALSE,1,2)"), n(2.0));
    assert_eq!(eval("IF(FALSE,1)"), b(false));
    assert_eq!(eval("IF(0,\"a\",\"b\")"), t("b"));
    assert_eq!(eval("IF(2,\"a\",\"b\")"), t("a"));
    assert_eq!(eval("IF(\"TRUE\",1,2)"), n(1.0));
    assert_eq!(eval("IF(\"x\",1,2)"), err(ValueErr));
    assert_eq!(eval("IF(#N/A,1,2)"), err(NA));
    assert_eq!(eval("IF(TRUE,,2)"), n(0.0));
    assert_eq!(eval("IF(FALSE,1,)"), n(0.0));
    assert_eq!(eval("IF(TRUE,1/0,2)"), err(DivZero));
    assert_eq!(eval("IF(1)"), err(ValueErr));
    let d = [("A1", n(7.0))];
    assert_eq!(eval_with(&d, "IF(TRUE,A1)"), n(7.0));
    assert_eq!(eval_with(&d, "IF(A1>5,\"big\",\"small\")"), t("big"));
    assert_eq!(
        eval_with(&d, "SUM(IF(TRUE,A1:A2))"),
        n(7.0),
        "references pass through IF"
    );
}

#[test]
fn iferror_and_ifna() {
    assert_eq!(eval("IFERROR(1/0,\"e\")"), t("e"));
    assert_eq!(eval("IFERROR(5,\"e\")"), n(5.0));
    assert_eq!(eval("IFERROR(#N/A,0)"), n(0.0));
    assert_eq!(eval("IFNA(#N/A,\"x\")"), t("x"));
    assert_eq!(eval("IFNA(1/0,\"x\")"), err(DivZero));
    assert_eq!(eval("IFNA(3,\"x\")"), n(3.0));
    let d = [("A1", n(1.0)), ("A2", err(DivZero)), ("A3", n(3.0))];
    // The first argument is a scalar parameter: with no intersection it is #VALUE!, which IFERROR catches.
    assert_eq!(eval_with(&d, "SUM(IFERROR(A1:A3,0))"), n(0.0));
    assert_eq!(eval_with(&d, "IFERROR(A2,\"bad\")"), t("bad"));
}

#[test]
fn and_or_xor_not() {
    assert_eq!(eval("AND(TRUE,1)"), b(true));
    assert_eq!(eval("AND(TRUE,0)"), b(false));
    assert_eq!(eval("OR(FALSE,0)"), b(false));
    assert_eq!(eval("OR(FALSE,2)"), b(true));
    assert_eq!(eval("XOR(TRUE,TRUE)"), b(false));
    assert_eq!(eval("XOR(TRUE,FALSE,TRUE,TRUE)"), b(true));
    assert_eq!(eval("NOT(TRUE)"), b(false));
    assert_eq!(eval("NOT(0)"), b(true));
    assert_eq!(eval("NOT(\"x\")"), err(ValueErr));
    assert_eq!(eval("AND(\"x\")"), err(ValueErr));
    assert_eq!(eval("AND(\"TRUE\")"), b(true));
    assert_eq!(eval("AND(1/0,TRUE)"), err(DivZero));
    let d = mixed();
    assert_eq!(eval_with(&d, "AND(A1:A2)"), b(true));
    assert_eq!(
        eval_with(&d, "AND(A3:A3)"),
        err(ValueErr),
        "text-only range has no logicals"
    );
    assert_eq!(
        eval_with(&d, "AND(A1:A6)"),
        b(true),
        "text and blanks in ranges are skipped"
    );
    assert_eq!(eval_with(&d, "OR(A5:A5)"), err(ValueErr));
}

#[test]
fn information_functions() {
    let mut d = mixed();
    d.push(("B1", err(DivZero)));
    d.push(("B2", err(NA)));
    assert_eq!(eval_with(&d, "ISBLANK(A5)"), b(true));
    assert_eq!(eval_with(&d, "ISBLANK(A1)"), b(false));
    assert_eq!(eval_with(&d, "ISBLANK(\"\")"), b(false));
    assert_eq!(eval_with(&d, "ISNUMBER(A1)"), b(true));
    assert_eq!(eval_with(&d, "ISNUMBER(\"1\")"), b(false));
    assert_eq!(eval_with(&d, "ISNUMBER(A3)"), b(false));
    assert_eq!(eval_with(&d, "ISTEXT(A3)"), b(true));
    assert_eq!(eval_with(&d, "ISTEXT(A1)"), b(false));
    assert_eq!(eval_with(&d, "ISERROR(1/0)"), b(true));
    assert_eq!(eval_with(&d, "ISERROR(1)"), b(false));
    assert_eq!(eval_with(&d, "ISERROR(B1)"), b(true));
    assert_eq!(eval_with(&d, "ISNA(B2)"), b(true));
    assert_eq!(eval_with(&d, "ISNA(B1)"), b(false));
    assert_eq!(eval_with(&d, "ISNA(1/0)"), b(false));
}

// ---- text ----------------------------------------------------------------------------

#[test]
fn len_left_right_mid() {
    assert_eq!(eval("LEN(\"hello\")"), n(5.0));
    assert_eq!(eval("LEN(123)"), n(3.0));
    assert_eq!(eval("LEN(1.5)"), n(3.0));
    assert_eq!(eval("LEN(\"héllo\")"), n(5.0));
    assert_eq!(eval("LEN(A1)"), n(0.0));
    assert_eq!(eval("LEFT(\"hello\",2)"), t("he"));
    assert_eq!(eval("LEFT(\"hello\")"), t("h"));
    assert_eq!(eval("LEFT(\"hi\",10)"), t("hi"));
    assert_eq!(eval("LEFT(\"héllo\",2)"), t("hé"));
    assert_eq!(eval("LEFT(\"hi\",-1)"), err(ValueErr));
    assert_eq!(eval("RIGHT(\"hello\",3)"), t("llo"));
    assert_eq!(eval("RIGHT(\"hello\")"), t("o"));
    assert_eq!(eval("RIGHT(\"hi\",10)"), t("hi"));
    assert_eq!(eval("MID(\"hello\",2,3)"), t("ell"));
    assert_eq!(eval("MID(\"hello\",4,10)"), t("lo"));
    assert_eq!(eval("MID(\"hello\",10,2)"), t(""));
    assert_eq!(eval("MID(\"hello\",0,2)"), err(ValueErr));
    assert_eq!(eval("MID(\"hello\",2,-1)"), err(ValueErr));
    assert_eq!(eval("LEN(#N/A)"), err(NA));
}

#[test]
fn case_and_trim() {
    assert_eq!(eval("UPPER(\"aBc\")"), t("ABC"));
    assert_eq!(eval("LOWER(\"aBc\")"), t("abc"));
    assert_eq!(eval("TRIM(\"  a   b  \")"), t("a b"));
    assert_eq!(eval("TRIM(\"\")"), t(""));
    assert_eq!(eval("UPPER(1)"), t("1"));
}

#[test]
fn concatenation_functions() {
    assert_eq!(eval("CONCATENATE(\"a\",1,TRUE)"), t("a1TRUE"));
    assert_eq!(eval("CONCATENATE()"), err(ValueErr));
    let d = mixed();
    assert_eq!(eval_with(&d, "CONCAT(A1:A2,\"x\")"), t("12x"));
    assert_eq!(eval_with(&d, "CONCAT(A1:A6)"), t("12xTRUE4"));
    assert_eq!(eval("TEXTJOIN(\",\",TRUE,\"a\",\"\",\"b\")"), t("a,b"));
    assert_eq!(eval("TEXTJOIN(\",\",FALSE,\"a\",\"\",\"b\")"), t("a,,b"));
    assert_eq!(
        eval_with(&d, "TEXTJOIN(\"-\",TRUE,A1:A6)"),
        t("1-2-x-TRUE-4")
    );
    assert_eq!(eval_with(&d, "TEXTJOIN(\"-\",FALSE,A5:A6)"), t("-4"));
    assert_eq!(eval("TEXTJOIN(\",\",TRUE)"), err(ValueErr));
}

#[test]
fn find_and_search() {
    assert_eq!(eval("FIND(\"l\",\"hello\")"), n(3.0));
    assert_eq!(eval("FIND(\"L\",\"hello\")"), err(ValueErr));
    assert_eq!(eval("FIND(\"l\",\"hello\",4)"), n(4.0));
    assert_eq!(eval("FIND(\"z\",\"hello\")"), err(ValueErr));
    assert_eq!(eval("FIND(\"\",\"abc\")"), n(1.0));
    assert_eq!(eval("FIND(\"a\",\"abc\",0)"), err(ValueErr));
    assert_eq!(eval("FIND(\"a\",\"abc\",9)"), err(ValueErr));
    assert_eq!(eval("FIND(\"*\",\"a*b\")"), n(2.0), "FIND has no wildcards");
    assert_eq!(eval("SEARCH(\"L\",\"hello\")"), n(3.0));
    assert_eq!(eval("SEARCH(\"h*o\",\"say hello\")"), n(5.0));
    assert_eq!(eval("SEARCH(\"?e\",\"hello\")"), n(1.0));
    assert_eq!(eval("SEARCH(\"~?\",\"what?\")"), n(5.0));
    assert_eq!(eval("SEARCH(\"z\",\"hello\")"), err(ValueErr));
    assert_eq!(eval("SEARCH(\"l\",\"hello\",4)"), n(4.0));
    assert_eq!(eval("SEARCH(\"\",\"abc\")"), n(1.0));
}

#[test]
fn substitute_rept_value() {
    assert_eq!(eval("SUBSTITUTE(\"a-b-c\",\"-\",\"+\")"), t("a+b+c"));
    assert_eq!(eval("SUBSTITUTE(\"a-b-c\",\"-\",\"+\",2)"), t("a-b+c"));
    assert_eq!(eval("SUBSTITUTE(\"a-b-c\",\"-\",\"+\",5)"), t("a-b-c"));
    assert_eq!(eval("SUBSTITUTE(\"abc\",\"\",\"x\")"), t("abc"));
    assert_eq!(eval("SUBSTITUTE(\"Aa\",\"a\",\"x\")"), t("Ax"));
    assert_eq!(eval("SUBSTITUTE(\"aaa\",\"aa\",\"b\")"), t("ba"));
    assert_eq!(eval("SUBSTITUTE(\"abc\",\"b\",\"\",0)"), err(ValueErr));
    assert_eq!(eval("REPT(\"ab\",3)"), t("ababab"));
    assert_eq!(eval("REPT(\"a\",0)"), t(""));
    assert_eq!(eval("REPT(\"a\",-1)"), err(ValueErr));
    assert_eq!(eval("REPT(\"abc\",20000)"), err(ValueErr));
    assert_eq!(eval("VALUE(\"12.5\")"), n(12.5));
    assert_eq!(eval("VALUE(\"1,000\")"), n(1000.0));
    assert_eq!(eval("VALUE(\" 7 \")"), n(7.0));
    assert_eq!(eval("VALUE(\"abc\")"), err(ValueErr));
    assert_eq!(eval("VALUE(\"\")"), err(ValueErr));
    assert_eq!(eval("VALUE(5)"), n(5.0));
    assert_eq!(eval("VALUE(TRUE)"), err(ValueErr));
}

// ---- conditional aggregation ---------------------------------------------------------

#[test]
fn sumif() {
    let d = fruit();
    assert_eq!(eval_with(&d, "SUMIF(B1:B6,\"apple\",C1:C6)"), n(5.0));
    assert_eq!(eval_with(&d, "SUMIF(B1:B6,\"ap*\",C1:C6)"), n(7.0));
    assert_eq!(eval_with(&d, "SUMIF(B1:B6,\"<>apple\",C1:C6)"), n(16.0));
    assert_eq!(eval_with(&d, "SUMIF(C1:C6,\">3\")"), n(15.0));
    assert_eq!(eval_with(&d, "SUMIF(C1:C6,\">=3\",C1:C6)"), n(18.0));
    assert_eq!(eval_with(&d, "SUMIF(B1:B6,5,C1:C6)"), n(5.0));
    assert_eq!(eval_with(&d, "SUMIF(B1:B6,\">5\",C1:C6)"), n(6.0));
    assert_eq!(eval_with(&d, "SUMIF(C1:C6,\">\"&C3)"), n(15.0));
    assert_eq!(eval_with(&d, "SUMIF(B:B,\"apple\",C:C)"), n(5.0));
    assert_eq!(eval_with(&d, "SUMIF(1,\"a\")"), err(ValueErr));
}

#[test]
fn sumif_propagates_errors_in_matched_cells_only() {
    let mut d = fruit();
    d.push(("C3", err(DivZero)));
    assert_eq!(eval_with(&d, "SUMIF(B1:B6,\"apple\",C1:C6)"), n(5.0));
    assert_eq!(eval_with(&d, "SUMIF(B1:B6,\"banana\",C1:C6)"), err(DivZero));
}

#[test]
fn countif() {
    let d = fruit();
    assert_eq!(eval_with(&d, "COUNTIF(B1:B6,\"apple\")"), n(2.0));
    assert_eq!(eval_with(&d, "COUNTIF(B1:B6,\"APPLE\")"), n(2.0));
    assert_eq!(eval_with(&d, "COUNTIF(B1:B6,\"*an*\")"), n(1.0));
    assert_eq!(eval_with(&d, "COUNTIF(B1:B6,\"?????\")"), n(2.0));
    assert_eq!(eval_with(&d, "COUNTIF(B1:B6,\"a*e\")"), n(2.0));
    assert_eq!(eval_with(&d, "COUNTIF(B1:B6,5)"), n(1.0));
    assert_eq!(eval_with(&d, "COUNTIF(C1:C6,\">2\")"), n(4.0));
    assert_eq!(eval_with(&d, "COUNTIF(C1:C6,\"<>3\")"), n(5.0));
    assert_eq!(eval_with(&d, "COUNTIF(C1:C6,\"<>\")"), n(6.0));
    assert_eq!(eval_with(&d, "COUNTIF(D1:D3,\"\")"), n(3.0));
    assert_eq!(eval_with(&d, "COUNTIF(D1:D3,\"=\")"), n(3.0));
    assert_eq!(eval_with(&d, "COUNTIF(B1:B6,\"\")"), n(0.0));
    assert_eq!(eval_with(&d, "COUNTIF(B1:B6,\"apple\",1)"), err(ValueErr));
    let tilde = [("A1", t("a*b")), ("A2", t("axb"))];
    assert_eq!(eval_with(&tilde, "COUNTIF(A1:A2,\"a~*b\")"), n(1.0));
    assert_eq!(eval_with(&tilde, "COUNTIF(A1:A2,\"a*b\")"), n(2.0));
    let errors = [("A1", err(NA)), ("A2", err(DivZero)), ("A3", n(1.0))];
    assert_eq!(eval_with(&errors, "COUNTIF(A1:A3,\"#N/A\")"), n(1.0));
    assert_eq!(eval_with(&errors, "COUNTIF(A1:A3,A1)"), n(1.0));
}

#[test]
fn averageif() {
    let d = fruit();
    assert_eq!(eval_with(&d, "AVERAGEIF(C1:C6,\">2\")"), n(4.5));
    assert_eq!(eval_with(&d, "AVERAGEIF(C1:C6,\">100\")"), err(DivZero));
    assert_eq!(eval_with(&d, "AVERAGEIF(B1:B6,\"apple\",C1:C6)"), n(2.5));
}

#[test]
fn sumifs_and_countifs() {
    let d = fruit();
    assert_eq!(
        eval_with(&d, "SUMIFS(C1:C6,B1:B6,\"apple\",C1:C6,\">1\")"),
        n(4.0)
    );
    assert_eq!(eval_with(&d, "SUMIFS(C1:C6,B1:B6,\"ap*\")"), n(7.0));
    assert_eq!(eval_with(&d, "SUMIFS(C1:C6,B1:B5,\"a*\")"), err(ValueErr));
    assert_eq!(eval_with(&d, "SUMIFS(C1:C6,B1:B6)"), err(ValueErr));
    assert_eq!(
        eval_with(&d, "COUNTIFS(B1:B6,\"apple\",C1:C6,\">1\")"),
        n(1.0)
    );
    assert_eq!(eval_with(&d, "COUNTIFS(B1:B6,\"apple\")"), n(2.0));
    assert_eq!(eval_with(&d, "COUNTIFS(B1:B6)"), err(ValueErr));
    assert_eq!(
        eval_with(&d, "COUNTIFS(B1:B6,\"apple\",C1:C5,\">1\")"),
        err(ValueErr)
    );
    assert_eq!(
        eval_with(&d, "COUNTIFS(C1:C6,\">=2\",C1:C6,\"<=4\")"),
        n(3.0)
    );
}

// ---- lookup --------------------------------------------------------------------------

#[test]
fn vlookup() {
    let d = table();
    assert_eq!(eval_with(&d, "VLOOKUP(2,A1:C4,2,FALSE)"), t("two"));
    assert_eq!(eval_with(&d, "VLOOKUP(2,A1:C4,2,0)"), t("two"));
    assert_eq!(eval_with(&d, "VLOOKUP(2.5,A1:C4,3)"), n(20.0));
    assert_eq!(eval_with(&d, "VLOOKUP(2.5,A1:C4,3,TRUE)"), n(20.0));
    assert_eq!(eval_with(&d, "VLOOKUP(99,A1:C4,3)"), n(40.0));
    assert_eq!(eval_with(&d, "VLOOKUP(0,A1:C4,2)"), err(NA));
    assert_eq!(eval_with(&d, "VLOOKUP(2.5,A1:C4,2,FALSE)"), err(NA));
    assert_eq!(eval_with(&d, "VLOOKUP(2,A1:C4,5,FALSE)"), err(Ref));
    assert_eq!(eval_with(&d, "VLOOKUP(2,A1:C4,0,FALSE)"), err(ValueErr));
    assert_eq!(eval_with(&d, "VLOOKUP(\"TWO\",B1:C4,2,FALSE)"), n(20.0));
    assert_eq!(eval_with(&d, "VLOOKUP(\"t*\",B1:C4,2,FALSE)"), n(20.0));
    assert_eq!(
        eval_with(&d, "VLOOKUP(\"two\",A1:C4,1,FALSE)"),
        err(NA),
        "type mismatch"
    );
    assert_eq!(eval_with(&d, "VLOOKUP(4,A:C,3,FALSE)"), n(40.0));
    assert_eq!(eval_with(&d, "VLOOKUP(#N/A,A1:C4,2,FALSE)"), err(NA));
    assert_eq!(
        eval_with(&d, "VLOOKUP(2,{1,\"a\";2,\"b\"},2,FALSE)"),
        t("b")
    );
    let blank_result = [("A1", n(1.0)), ("B1", Value::Empty)];
    assert_eq!(eval_with(&blank_result, "VLOOKUP(1,A1:B1,2,FALSE)"), n(0.0));
}

#[test]
fn hlookup() {
    let d = table();
    assert_eq!(eval_with(&d, "HLOOKUP(3,E1:H2,2,FALSE)"), t("c"));
    assert_eq!(eval_with(&d, "HLOOKUP(3.5,E1:H2,2)"), t("c"));
    assert_eq!(eval_with(&d, "HLOOKUP(0,E1:H2,2)"), err(NA));
    assert_eq!(eval_with(&d, "HLOOKUP(3,E1:H2,3,FALSE)"), err(Ref));
}

#[test]
fn index() {
    let d = table();
    assert_eq!(eval_with(&d, "INDEX(A1:C4,2,3)"), n(20.0));
    assert_eq!(eval_with(&d, "INDEX(A1:A4,3)"), n(3.0));
    assert_eq!(eval_with(&d, "INDEX(E1:H1,3)"), n(3.0));
    assert_eq!(eval_with(&d, "INDEX(A1:C4,5,1)"), err(Ref));
    assert_eq!(eval_with(&d, "INDEX(A1:C4,1,4)"), err(Ref));
    assert_eq!(eval_with(&d, "INDEX(A1:C4,-1,1)"), err(ValueErr));
    assert_eq!(eval_with(&d, "SUM(INDEX(A1:C4,0,3))"), n(100.0));
    assert_eq!(eval_with(&d, "SUM(INDEX(A1:C4,2,0))"), n(22.0));
    assert_eq!(eval_with(&d, "INDEX({1,2;3,4},2,1)"), n(3.0));
    assert_eq!(eval_with(&d, "INDEX(A:A,3)"), n(3.0));
    assert_eq!(
        eval_with(&d, "INDEX(A:A,500000)"),
        n(0.0),
        "a blank cell shows as 0"
    );
}

#[test]
fn match_function() {
    let d = table();
    assert_eq!(eval_with(&d, "MATCH(3,A1:A4,0)"), n(3.0));
    assert_eq!(eval_with(&d, "MATCH(3.5,A1:A4,1)"), n(3.0));
    assert_eq!(eval_with(&d, "MATCH(3.5,A1:A4)"), n(3.0));
    assert_eq!(eval_with(&d, "MATCH(5,A1:A4,1)"), n(4.0));
    assert_eq!(eval_with(&d, "MATCH(0,A1:A4,1)"), err(NA));
    assert_eq!(eval_with(&d, "MATCH(9,A1:A4,0)"), err(NA));
    assert_eq!(eval_with(&d, "MATCH(\"three\",B1:B4,0)"), n(3.0));
    assert_eq!(eval_with(&d, "MATCH(\"THREE\",B1:B4,0)"), n(3.0));
    assert_eq!(eval_with(&d, "MATCH(\"t*\",B1:B4,0)"), n(2.0));
    assert_eq!(eval_with(&d, "MATCH(\"one\",A1:C1,0)"), n(2.0));
    assert_eq!(eval_with(&d, "MATCH(25,D1:D4,-1)"), n(2.0));
    assert_eq!(eval_with(&d, "MATCH(30,D1:D4,-1)"), n(2.0));
    assert_eq!(eval_with(&d, "MATCH(1,A1:C4,0)"), err(NA));
}

#[test]
fn xlookup() {
    let d = table();
    assert_eq!(eval_with(&d, "XLOOKUP(3,A1:A4,B1:B4)"), t("three"));
    assert_eq!(eval_with(&d, "XLOOKUP(9,A1:A4,B1:B4,\"none\")"), t("none"));
    assert_eq!(eval_with(&d, "XLOOKUP(9,A1:A4,B1:B4)"), err(NA));
    assert_eq!(eval_with(&d, "XLOOKUP(2.5,A1:A4,B1:B4,,-1)"), t("two"));
    assert_eq!(eval_with(&d, "XLOOKUP(2.5,A1:A4,B1:B4,,1)"), t("three"));
    assert_eq!(
        eval_with(&d, "XLOOKUP(2,A1:A4,B1:B4,,-1)"),
        t("two"),
        "exact beats nearest"
    );
    assert_eq!(eval_with(&d, "XLOOKUP(\"t*\",B1:B4,A1:A4,,2)"), n(2.0));
    assert_eq!(eval_with(&d, "XLOOKUP(\"t*\",B1:B4,A1:A4)"), err(NA));
    assert_eq!(eval_with(&d, "XLOOKUP(3,E1:H1,E2:H2)"), t("c"));
    assert_eq!(
        eval_with(&d, "XLOOKUP(1,{1;2;1},{\"a\";\"b\";\"c\"},,0,-1)"),
        t("c")
    );
    assert_eq!(
        eval_with(&d, "XLOOKUP(1,{1;2;1},{\"a\";\"b\";\"c\"})"),
        t("a")
    );
    assert_eq!(eval_with(&d, "XLOOKUP(1,A1:A4,B1:B3)"), err(ValueErr));
    assert_eq!(eval_with(&d, "XLOOKUP(1,A1:C4,B1:B4)"), err(ValueErr));
    assert_eq!(
        eval_full(&d, "XLOOKUP(3,A1:A4,B1:C4)"),
        Value::Array(vec![vec![t("three"), n(30.0)]])
    );
}

#[test]
fn choose() {
    assert_eq!(eval("CHOOSE(2,\"a\",\"b\",\"c\")"), t("b"));
    assert_eq!(eval("CHOOSE(2.9,\"a\",\"b\",\"c\")"), t("b"));
    assert_eq!(eval("CHOOSE(0,\"a\",\"b\")"), err(ValueErr));
    assert_eq!(eval("CHOOSE(4,\"a\",\"b\",\"c\")"), err(ValueErr));
    assert_eq!(eval("CHOOSE(1)"), err(ValueErr));
    let d = [
        ("A1", n(1.0)),
        ("A2", n(2.0)),
        ("C1", n(10.0)),
        ("C2", n(20.0)),
    ];
    assert_eq!(eval_with(&d, "SUM(CHOOSE(1,A1:A2,C1:C2))"), n(3.0));
    assert_eq!(eval_with(&d, "SUM(CHOOSE(2,A1:A2,C1:C2))"), n(30.0));
}

// ---- dates ---------------------------------------------------------------------------

#[test]
fn date_construction() {
    assert_eq!(eval("DATE(2024,2,29)"), n(45351.0));
    assert_eq!(eval("DATE(2000,1,1)"), n(36526.0));
    assert_eq!(eval("DATE(2024,13,1)"), eval("DATE(2025,1,1)"));
    assert_eq!(eval("DATE(2024,1,32)"), eval("DATE(2024,2,1)"));
    assert_eq!(eval("DATE(2024,0,1)"), eval("DATE(2023,12,1)"));
    assert_eq!(eval("DATE(24,1,1)"), eval("DATE(1924,1,1)"));
    assert_eq!(eval("DATE(2024.9,1.9,1.9)"), eval("DATE(2024,1,1)"));
    assert_eq!(eval("DATE(-1,1,1)"), err(Num));
    assert_eq!(eval("DATE(10000,1,1)"), err(Num));
    assert_eq!(eval("DATE(\"x\",1,1)"), err(ValueErr));
    assert_eq!(eval("DATE(2024,3,1)-DATE(2024,2,1)"), n(29.0));
}

#[test]
fn the_1900_leap_year_bug_is_reproduced() {
    assert_eq!(eval("DATE(1900,1,1)"), n(1.0));
    assert_eq!(eval("DATE(1900,2,28)"), n(59.0));
    assert_eq!(eval("DATE(1900,2,29)"), n(60.0));
    assert_eq!(eval("DATE(1900,3,1)"), n(61.0));
    assert_eq!(eval("YEAR(60)"), n(1900.0));
    assert_eq!(eval("MONTH(60)"), n(2.0));
    assert_eq!(eval("DAY(60)"), n(29.0));
    assert_eq!(eval("MONTH(61)"), n(3.0));
    assert_eq!(eval("DAY(61)"), n(1.0));
    assert_eq!(eval("MONTH(59)"), n(2.0));
    assert_eq!(eval("DAY(59)"), n(28.0));
    assert_eq!(eval("DAY(0)"), n(0.0));
    assert_eq!(eval("YEAR(0)"), n(1900.0));
}

#[test]
fn date_parts() {
    assert_eq!(eval("YEAR(45351)"), n(2024.0));
    assert_eq!(eval("MONTH(45351)"), n(2.0));
    assert_eq!(eval("DAY(45351)"), n(29.0));
    assert_eq!(eval("DAY(45351.99)"), n(29.0));
    assert_eq!(eval("YEAR(DATE(2031,7,4))"), n(2031.0));
    assert_eq!(eval("YEAR(-1)"), err(Num));
    assert_eq!(eval("YEAR(3000000)"), err(Num));
    assert_eq!(eval("YEAR(\"x\")"), err(ValueErr));
}

#[test]
fn the_1904_date_system() {
    let mut e = engine_with(&[]);
    e.set_date1904(true);
    assert_eq!(eval_in(&mut e, "DATE(1904,1,1)"), n(0.0));
    assert_eq!(eval_in(&mut e, "DATE(1904,1,2)"), n(1.0));
    assert_eq!(eval_in(&mut e, "DATE(2024,2,29)"), n(45351.0 - 1462.0));
    assert_eq!(eval_in(&mut e, "YEAR(0)"), n(1904.0));
    assert_eq!(eval_in(&mut e, "DAY(1)"), n(2.0));
    assert_eq!(eval_in(&mut e, "DATE(1900,1,1)"), err(Num));
    assert_eq!(
        eval_in(&mut e, "YEAR(60)"),
        n(1904.0),
        "no fake leap day in 1904"
    );
}

#[test]
fn today_and_now_use_the_injected_clock() {
    // 1_700_000_000 s since the epoch is 2023-11-14 22:13:20.
    assert_eq!(eval("TODAY()"), n(45244.0));
    assert_eq!(eval("YEAR(TODAY())"), n(2023.0));
    assert_eq!(eval("MONTH(TODAY())"), n(11.0));
    assert_eq!(eval("DAY(TODAY())"), n(14.0));
    let expected = 45244.0 + (CLOCK_SECONDS % 86_400.0) / 86_400.0;
    assert_close(&eval("NOW()"), expected);
    assert_eq!(eval("INT(NOW())"), eval("TODAY()"));
    let mut e = engine_with(&[]);
    e.set_date1904(true);
    assert_eq!(eval_in(&mut e, "TODAY()"), n(45244.0 - 1462.0));
}

#[test]
fn dollar_uses_the_en_us_currency_style() {
    assert_eq!(eval("=DOLLAR(1234.567)"), t("$1,234.57"));
    assert_eq!(eval("=DOLLAR(1234.567,-2)"), t("$1,200"));
    // Microsoft's documented example; the en-GB oracle cannot confirm negatives.
    assert_eq!(eval("=DOLLAR(-1234.567,-2)"), t("($1,200)"));
}

#[test]
fn text_functions_added_with_the_text_batch() {
    assert_eq!(eval("=TEXTBEFORE(\"a-b-c\",\"-\",-1)"), t("a-b"));
    assert_eq!(eval("=TEXTAFTER(\"aaaa\",\"aa\",-2)"), t("aa"));
    assert_eq!(
        eval("=TEXTJOIN(\"|\",FALSE,TEXTSPLIT(\"a,b;c\",\",\",\";\",,,\"-\"))"),
        t("a|b|c|-")
    );
    assert_eq!(eval("=PROPER(\"2nd it's\")"), t("2Nd It'S"));
    assert_eq!(eval("=NUMBERVALUE(\"3%%\")"), n(0.0003));
    assert_eq!(eval("=UNICHAR(55296)"), err(NA));
    assert_eq!(eval("=LENB(\"😀x\")"), n(3.0));
    assert_eq!(eval("=FIXED(1234.5678,-2)"), t("1,200"));
}
