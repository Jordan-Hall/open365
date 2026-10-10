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
