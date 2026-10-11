//! Word's line metrics: which font tables give ascent and descent, and how the line spacing
//! rules turn them into a line height and a baseline position.

use open365_docx::LineSpacing;
use open365_text::FaceMetrics;

/// Which vertical metrics a line is built from. Chosen by measurement against Word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineMetricModel {
    /// `OS/2` `usWinAscent` and `usWinDescent`.
    Win,
    /// `hhea` ascender, descender and line gap (gap added above the line).
    Hhea,
}

impl LineMetricModel {
    /// Ascent and descent in points for a face at `size` points.
    pub fn ascent_descent(self, m: &FaceMetrics, size: f64) -> (f64, f64) {
        let upem = f64::from(m.units_per_em);
        let (a, d) = match self {
            Self::Win => (m.win_ascent, m.win_descent),
            Self::Hhea => (m.hhea_ascent + m.hhea_line_gap, m.hhea_descent),
        };
        (f64::from(a) * size / upem, f64::from(d) * size / upem)
    }
}

/// What an inline picture does to a line: its height above the baseline (zero when there is
/// none) and the ascent and height of the text on the line.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PictureLine {
    pub height: f64,
    pub text_asc: f64,
    pub text_h: f64,
}

/// Line height and baseline offset (from the line top) for the natural `asc`/`desc` under
/// a spacing rule.
pub(crate) fn line_height(
    asc: f64,
    desc: f64,
    rule: LineSpacing,
    _model: LineMetricModel,
    pic: PictureLine,
) -> (f64, f64) {
    let single = asc + desc;
    let taller = pic.height > pic.text_asc;
    match rule {
        LineSpacing::Multiple(m) if taller => {
            // Measured (layout-img-inline): a picture is not stretched by the multiple, only
            // the text's own height is: the extra leading goes below the baseline.
            (single + (f64::from(m) - 1.0) * pic.text_h, asc)
        }
        LineSpacing::Multiple(m) => {
            // Extra leading goes below the baseline (measured: a 1.079 multiple leaves the
            // first baseline where single spacing puts it).
            let h = single * f64::from(m);
            (h, asc)
        }
        LineSpacing::Exactly(x) if taller => {
            // Measured (layout-img-inline, one point): the baseline sits at 80 % of the line.
            let h = f64::from(x);
            (h, 0.8 * h)
        }
        LineSpacing::AtLeast(x) => {
            let h = single.max(f64::from(x));
            (h, asc + (h - single))
        }
        LineSpacing::Exactly(x) => {
            let h = f64::from(x);
            (h, asc + (h - single))
        }
    }
}
