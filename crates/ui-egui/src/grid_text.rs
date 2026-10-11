//! How a spreadsheet cell's text behaves in Excel, as pure functions the grid view drives:
//! General alignment by value type, number fitting (`####` and the shrinking of General
//! numbers), overflow into empty neighbours and fill/shrink helpers.
//!
//! Measurements are in device pixels (the real pixel grid the geometry is laid out on), so the
//! caller supplies a `measure` function that returns the width of a string in pixels with the
//! cell's font.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use egui::{Color32, FontData};
use open365_xlsx::{CellValue, HorizontalAlignment, Rgb};

/// Pixels between a cell's edges and its text: 2 px each side plus the 1 px grid line. Number
/// text needs more room than this; see `number_inset_px` in the grid view.
pub(crate) const TEXT_INSET_PX: f32 = 5.0;

/// A font at one pixel size whose glyph advances come from the TrueType hinting program, as
/// GDI measures text at screen sizes (Excel decides `####`, General-number shrinking and
/// overflow with these widths). Advances are whole pixels; there is no kerning.
pub(crate) struct HintedFace {
    data: Arc<FontData>,
    hinter: skrifa::outline::HintingInstance,
    ppem: f32,
    advances: RefCell<HashMap<char, Option<f32>>>,
}

struct NullPen;

impl skrifa::outline::OutlinePen for NullPen {
    fn move_to(&mut self, _: f32, _: f32) {}
    fn line_to(&mut self, _: f32, _: f32) {}
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn close(&mut self) {}
}

impl HintedFace {
    /// `ppem` is the font height in whole pixels (GDI rounds `size * dpi / 72`).
    pub(crate) fn new(data: Arc<FontData>, ppem: u32) -> Option<Self> {
        use skrifa::MetadataProvider as _;
        use skrifa::instance::{LocationRef, Size};
        use skrifa::outline::{Engine, HintingInstance, HintingOptions, Target};
        let ppem = ppem.max(1) as f32;
        let font = skrifa::FontRef::from_index(&data.font, data.index).ok()?;
        let hinter = HintingInstance::new(
            &font.outline_glyphs(),
            Size::new(ppem),
            LocationRef::default(),
            HintingOptions {
                engine: Engine::Interpreter,
                target: Target::Mono,
            },
        )
        .ok()?;
        Some(Self {
            data,
            hinter,
            ppem,
            advances: RefCell::new(HashMap::new()),
        })
    }

    /// Hinted advance of `c` in pixels, or `None` when the font has no glyph for it.
    fn advance(&self, c: char) -> Option<f32> {
        if let Some(a) = self.advances.borrow().get(&c) {
            return *a;
        }
        let a = self.compute(c);
        self.advances.borrow_mut().insert(c, a);
        a
    }

    fn compute(&self, c: char) -> Option<f32> {
        use skrifa::MetadataProvider as _;
        use skrifa::instance::{LocationRef, Size};
        use skrifa::outline::DrawSettings;
        let font = skrifa::FontRef::from_index(&self.data.font, self.data.index).ok()?;
        let gid = font.charmap().map(c)?;
        let outlines = font.outline_glyphs();
        let hinted = outlines
            .get(gid)
            .and_then(|g| {
                g.draw(DrawSettings::hinted(&self.hinter, false), &mut NullPen)
                    .ok()
            })
            .and_then(|m| m.advance_width);
        hinted.or_else(|| {
            font.glyph_metrics(Size::new(self.ppem), LocationRef::default())
                .advance_width(gid)
                .map(f32::round)
        })
    }

    /// Width of `text` in pixels, or `None` when a character has no glyph in this font.
    pub(crate) fn width(&self, text: &str) -> Option<f32> {
        text.chars()
            .try_fold(0.0, |w, c| Some(w + self.advance(c)?))
    }
}

/// Height in pixels of one line of `data` at `ppem`, as GDI reports it: the Windows ascent and
/// descent scaled and rounded one by one.
pub(crate) fn line_px(data: &FontData, ppem: u32) -> Option<u32> {
    use skrifa::raw::TableProvider as _;
    let font = skrifa::FontRef::from_index(&data.font, data.index).ok()?;
    let os2 = font.os2().ok()?;
    let upm = f32::from(font.head().ok()?.units_per_em());
    let scale = ppem.max(1) as f32 / upm;
    let ascent = (f32::from(os2.us_win_ascent()) * scale).round();
    let descent = (f32::from(os2.us_win_descent()) * scale).round();
    Some((ascent + descent) as u32)
}

/// Hinted faces by font data and pixel size.
#[derive(Default)]
pub(crate) struct HintCache {
    faces: HashMap<(usize, u32), Option<Rc<HintedFace>>>,
}

impl std::fmt::Debug for HintCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HintCache")
            .field("faces", &self.faces.len())
            .finish()
    }
}
impl HintCache {
    pub(crate) fn face(&mut self, data: &Arc<FontData>, ppem: u32) -> Option<Rc<HintedFace>> {
        self.faces
            .entry((Arc::as_ptr(data) as usize, ppem))
            .or_insert_with(|| HintedFace::new(Arc::clone(data), ppem).map(Rc::new))
            .clone()
    }
}

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
/// trailing zeros. A negative number that rounds to zero prints as `-0`, as Excel shows it.
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
    if n < 0.0 {
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
    fn negative_numbers_that_round_to_zero_keep_the_sign() {
        let mut m = measure;
        let t = fit_number(-0.25, "-0.25".to_owned(), true, 24.0, &mut m);
        assert_eq!(t.into_string(), "-0");
    }

    #[test]
    fn hinted_advances_are_whole_pixels_from_the_font_program() {
        // Carlito is metric-compatible with Calibri; at 18 px GDI measures '-' 6, digits 9, '.' 5.
        let data = Arc::new(FontData::from_static(include_bytes!(
            "../../../assets/fonts/Carlito-Regular.ttf"
        )));
        let face = HintedFace::new(Arc::clone(&data), 18).expect("hinting instance");
        assert_eq!(face.width("-0.5"), Some(29.0));
        assert_eq!(face.width("-1"), Some(15.0));
        assert_eq!(face.width("\u{4e2d}"), None, "no glyph: caller falls back");
        let mut cache = HintCache::default();
        assert!(Rc::ptr_eq(
            &cache.face(&data, 18).unwrap(),
            &cache.face(&data, 18).unwrap()
        ));
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
