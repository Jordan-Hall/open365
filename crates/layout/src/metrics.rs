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

/// Line height and baseline offset (from the line top) for the natural `asc`/`desc` under
/// a spacing rule.
pub(crate) fn line_height(
    asc: f64,
    desc: f64,
    rule: LineSpacing,
    _model: LineMetricModel,
) -> (f64, f64) {
    let single = asc + desc;
    match rule {
        LineSpacing::Multiple(m) => {
            let h = single * f64::from(m);
            (h, asc + (h - single))
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
