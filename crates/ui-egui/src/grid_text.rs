//! How a spreadsheet cell's text behaves in Excel, as pure functions the grid view drives:
//! General alignment by value type, number fitting (`####` and the shrinking of General
//! numbers), overflow into empty neighbours and fill/shrink helpers.
//!
//! Measurements are in device pixels (the real pixel grid the geometry is laid out on), so the
//! caller supplies a `measure` function that returns the width of a string in pixels with the
//! cell's font.

use egui::Color32;
use open365_xlsx::{CellValue, HorizontalAlignment, Rgb};

/// Pixels between a cell's edges and its text: 2 px each side plus the 1 px grid line, so a
/// column of `n` pixels holds text up to `n - 5` pixels wide (checked against Excel's `####`
/// decisions in `tests/grid_style.rs`).
pub(crate) const TEXT_INSET_PX: f32 = 5.0;

pub(crate) fn color32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

/// Whether the value is a number, an error or a boolean; those never overflow.
pub(crate) fn is_numeric(value: &CellValue) -> bool {
    matches!(value, CellValue::Number(_))
}

/// General alignment resolved by value type: numbers right, text left, booleans and errors
/// centred. Other alignments are returned unchanged.
pub(crate) fn effective_horizontal(
    h: HorizontalAlignment,
    value: &CellValue,
) -> HorizontalAlignment {
    if h != HorizontalAlignment::General {
        return h;
    }
    match value {
        CellValue::Number(_) => HorizontalAlignment::Right,
        CellValue::Bool(_) | CellValue::Error(_) => HorizontalAlignment::Center,
        _ => HorizontalAlignment::Left,
    }
}

/// Rounds half away from zero to `decimals` places, as Excel does, and prints without
/// trailing zeros. `-0` prints as `0`.
fn fixed(n: f64, decimals: usize) -> String {
    let scale = 10f64.powi(decimals as i32);
    let r = (n.abs() * scale + 0.5).floor() / scale;
    let mut s = format!("{r:.decimals$}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if n < 0.0 && s != "0" {
        s.insert(0, '-');
    }
    s
}

/// Scientific text with `mantissa_decimals` decimals and an exponent of at least two digits.
fn scientific(n: f64, mantissa_decimals: usize) -> String {
    let s = format!("{:.*e}", mantissa_decimals, n);
    let (mantissa, exp) = s.split_once('e').unwrap_or((&s, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let mut mantissa = mantissa.to_owned();
    if mantissa.contains('.') {
        while mantissa.ends_with('0') {
            mantissa.pop();
        }
        if mantissa.ends_with('.') {
            mantissa.pop();
        }
    }
    format!(
        "{mantissa}E{}{:02}",
        if exp < 0 { '-' } else { '+' },
        exp.abs()
    )
}

/// The text a number cell shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NumberText {
    Text(String),
    /// Does not fit: the cell shows as many `#` as fit.
    Hashes(String),
}

impl NumberText {
    #[cfg(test)]
    pub fn into_string(self) -> String {
        match self {
            Self::Text(s) | Self::Hashes(s) => s,
        }
    }
}

/// Fits the formatted number `full` into `avail` pixels.
///
/// A General number gives up decimals one by one (rounding half away from zero), then falls
/// back to scientific notation with fewer mantissa digits. Any other format that is too wide,
/// and a General number that cannot shrink enough, shows as `#` characters.
pub(crate) fn fit_number(
    n: f64,
    full: String,
    general: bool,
    avail: f32,
    measure: &mut dyn FnMut(&str) -> f32,
) -> NumberText {
    if measure(&full) <= avail {
        return NumberText::Text(full);
    }
    if general && n.is_finite() {
        let mut candidates: Vec<String> = Vec::new();
        if !full.contains('E') {
            candidates.extend((0..=14).rev().map(|d| fixed(n, d)));
        }
        candidates.extend((0..=5).rev().map(|m| scientific(n, m)));
        for c in candidates {
            if c.len() <= full.len() && measure(&c) <= avail {
                return NumberText::Text(c);
            }
        }
    }
    let one = measure("#").max(1.0);
    let count = ((avail / one).floor() as usize).max(1);
    NumberText::Hashes("#".repeat(count))
}

/// Room available beyond a cell for text that overflows: walks neighbours in `step` direction
/// (+1 or -1) while they are empty, adding their widths, until `need` is covered.
pub(crate) fn overflow_room(
    start: u32,
    step: i32,
    max_col: u32,
    need: f32,
    is_empty: &dyn Fn(u32) -> bool,
    width: &dyn Fn(u32) -> f32,
) -> f32 {
    let mut room = 0.0;
    let mut col = i64::from(start) + i64::from(step);
    let mut steps = 0;
    while room < need && (1..=i64::from(max_col)).contains(&col) && steps < 256 {
        let c = col as u32;
        if !is_empty(c) {
            break;
        }
        room += width(c);
        col += i64::from(step);
        steps += 1;
    }
    room
}

/// The text of a Fill-aligned cell: the string repeated as often as it fits.
pub(crate) fn fill_text(text: &str, avail: f32, measure: &mut dyn FnMut(&str) -> f32) -> String {
    let one = measure(text);
    if text.is_empty() || one <= 0.0 {
        return text.to_owned();
    }
    let count = ((avail / one).floor() as usize).max(1);
    text.repeat(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(s: &str) -> f32 {
        // Calibri 11 at 120 dpi: digits 9 px, '.' and '-' 5 px, 'E' and '+' 9 px, '#' 9 px.
        s.chars()
            .map(|c| {
                if matches!(c, '.' | '-' | ',') {
                    5.0
                } else {
                    9.0
                }
            })
            .sum()
    }

    #[test]
    fn general_numbers_give_up_decimals_then_use_scientific() {
        let mut m = measure;
        let fit = |n: f64, full: &str, avail: f32, m: &mut dyn FnMut(&str) -> f32| {
            fit_number(n, full.to_owned(), true, avail, m).into_string()
        };
        assert_eq!(fit(0.123456789, "0.123456789", 54.0, &mut m), "0.1235");
        assert_eq!(fit(12.5, "12.5", 27.0, &mut m), "13");
        assert_eq!(fit(-0.5, "-0.5", 27.0, &mut m), "-1");
        assert_eq!(fit(1234567.0, "1234567", 54.0, &mut m), "1E+06");
        assert_eq!(fit(1234567.0, "1234567", 36.0, &mut m), "####");
        assert_eq!(fit(0.000123, "0.000123", 27.0, &mut m), "0");
    }

    #[test]
    fn other_formats_show_hashes() {
        let mut m = measure;
        let t = fit_number(1234.5678, "1234.57".to_owned(), false, 36.0, &mut m);
        assert_eq!(t, NumberText::Hashes("####".to_owned()));
    }

    #[test]
    fn general_alignment_follows_the_value_type() {
        use HorizontalAlignment::{Center, Left, Right};
        let g = HorizontalAlignment::General;
        assert_eq!(effective_horizontal(g, &CellValue::Number(1.0)), Right);
        assert_eq!(effective_horizontal(g, &CellValue::Bool(true)), Center);
        assert_eq!(
            effective_horizontal(g, &CellValue::InlineStr("a".into())),
            Left
        );
        assert_eq!(effective_horizontal(Left, &CellValue::Number(1.0)), Left);
    }

    #[test]
    fn overflow_stops_at_the_first_non_empty_neighbour() {
        let empty = |c: u32| c != 4;
        let width = |_c: u32| 50.0;
        assert_eq!(overflow_room(1, 1, 10, 500.0, &empty, &width), 100.0);
        assert_eq!(overflow_room(1, 1, 10, 60.0, &|_| true, &width), 100.0);
        assert_eq!(overflow_room(3, -1, 10, 500.0, &|_| true, &width), 100.0);
    }
}
