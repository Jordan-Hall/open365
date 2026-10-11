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
            // Faces that set USE_TYPO_METRICS (fsSelection bit 7: Aptos, Cascadia, Bahnschrift,
            // Gabriola) get their line from the typographic metrics, the line gap above the
            // ascent. Measured with layout-face-typo (24 pt line pitch 29.3, 27.9, 28.8, 40.8 pt).
            Self::Win if m.use_typo_metrics => (m.typo_ascent + m.typo_line_gap, m.typo_descent),
            Self::Win => {
                // GDI's external leading: the part of the hhea line gap that the Windows
                // metrics do not already cover goes above the line (Arial and Times New
                // Roman gain 67 and 87 units, Calibri and Cambria none). Measured with
                // layout-img-line text and picture runs in six faces.
                let covered = (m.win_ascent + m.win_descent) - (m.hhea_ascent + m.hhea_descent);
                let external = (m.hhea_line_gap - covered).max(0);
                (m.win_ascent + external, m.win_descent)
            }
            Self::Hhea => (m.hhea_ascent + m.hhea_line_gap, m.hhea_descent),
        };
        (f64::from(a) * size / upem, f64::from(d) * size / upem)
    }
}

/// What the inline pictures on a line do to it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PictureLine {
    /// Tallest inline picture (its height above the baseline); zero when there is none.
    pub height: f64,
    /// Tallest font height (ascent plus descent) among the runs that hold a picture.
    pub run_h: f64,
    /// Ascent and height of the text on the line.
    pub text_asc: f64,
    pub text_h: f64,
}

/// Line height and baseline offset (from the line top) for the natural `asc`/`desc` of the
/// text under a spacing rule. Measured against Word (layout-img-line, layout-img-spacing):
///
/// * A picture's run sizes the line like a text run, except that the picture has no descent:
///   the line grows to the run's full font height by raising the baseline
///   (`run_h - desc`), the descent staying the text's. The run counts only for automatic
///   (multiple) spacing; "at least" and "exactly" ignore it.
/// * A multiple above one adds `(m - 1)` times the tallest font height below the baseline; a
///   multiple below one scales the ascent and the descent.
/// * "At least" puts the extra space above the baseline.
/// * "Exactly" puts the baseline at 80 % of the line, whatever the fonts and pictures.
pub(crate) fn line_height(
    asc: f64,
    desc: f64,
    rule: LineSpacing,
    _model: LineMetricModel,
    pic: PictureLine,
) -> (f64, f64) {
    match rule {
        LineSpacing::Multiple(m) => {
            let m = f64::from(m);
            let asc = asc.max(pic.height).max(pic.run_h - desc);
            let tallest = pic.text_h.max(pic.run_h);
            if m >= 1.0 {
                (asc + desc + (m - 1.0) * tallest, asc)
            } else {
                // A picture taller than the text keeps its height: only the text's own
                // ascent shrinks.
                let base = pic.text_asc.max(pic.run_h - desc);
                let asc = asc - (1.0 - m) * base;
                (asc + m * desc, asc)
            }
        }
        LineSpacing::AtLeast(x) => {
            let asc = asc.max(pic.height);
            let single = asc + desc;
            let h = single.max(f64::from(x));
            (h, asc + (h - single))
        }
        LineSpacing::Exactly(x) => {
            let h = f64::from(x);
            (h, 0.8 * h)
        }
    }
}
