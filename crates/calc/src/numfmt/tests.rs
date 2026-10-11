use super::*;

fn n(v: f64, code: &str) -> String {
    format_value(&Value::Number(v), code, false)
}

#[test]
fn plain_digits() {
    assert_eq!(n(1234.5, "0.00"), "1234.50");
    assert_eq!(n(1234.5678, "#,##0.00"), "1,234.57");
    assert_eq!(n(0.5, "0%"), "50%");
    assert_eq!(n(2.675, "0.00"), "2.68");
}

#[test]
fn general_width() {
    assert_eq!(n(123_456_789_012.0, "General"), "1.23457E+11");
    assert_eq!(n(1.0 / 3.0, "General"), "0.333333333");
}

#[test]
fn builtin_ids() {
    assert_eq!(builtin_format_code(14), Some("m/d/yyyy"));
    assert_eq!(builtin_format_code(49), Some("@"));
    assert_eq!(builtin_format_code(30), None);
}

#[test]
fn cell_format_codes() {
    let c = |code: &str| cell_format_code(code, "£");
    assert_eq!(c("General"), "G");
    assert_eq!(c("0.00"), "F2");
    assert_eq!(c("#,##0"), ",0");
    assert_eq!(c("0.0%"), "P1");
    assert_eq!(c("0.00E+00"), "S2");
    assert_eq!(c("£#,##0.00_);[Red](£#,##0.00)"), "C2-");
    assert_eq!(c("(0)"), "F0()");
    assert_eq!(c("# ?/?"), "G");
    assert_eq!(c(";;;"), "H");
    assert_eq!(c("m/d/yy"), "D4");
    assert_eq!(c("dd/mm/yyyy"), "D1");
    assert_eq!(c("h:mm:ss AM/PM"), "D6");
    assert_eq!(c("yyyy-mm-dd"), "G");
    assert_eq!(c("[$-409]mmmm d, yyyy"), "D2");
}
