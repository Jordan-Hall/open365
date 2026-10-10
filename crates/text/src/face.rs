use std::sync::Arc;

use harfrust::ShaperData;
use skrifa::raw::TableProvider;
use skrifa::{FontRef, MetadataProvider};

use crate::FontError;

/// Vertical metrics of a face in font units, taken from the tables that Word's line
/// height is derived from (see `open365-layout` for which one it uses).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaceMetrics {
    pub units_per_em: u16,
    /// `hhea` ascender (positive up).
    pub hhea_ascent: i32,
    /// `hhea` descender as a positive distance below the baseline.
    pub hhea_descent: i32,
    pub hhea_line_gap: i32,
    /// `OS/2` `usWinAscent`.
    pub win_ascent: i32,
    /// `OS/2` `usWinDescent` (positive).
    pub win_descent: i32,
    /// `OS/2` `sTypoAscender`.
    pub typo_ascent: i32,
    /// `OS/2` `sTypoDescender` as a positive distance below the baseline.
    pub typo_descent: i32,
    pub typo_line_gap: i32,
    /// `OS/2` `fsSelection` bit 7.
    pub use_typo_metrics: bool,
}

/// One font file (or one face of a collection) with cached shaping data.
pub struct FontFace {
    data: Arc<[u8]>,
    index: u32,
    family: String,
    bold: bool,
    italic: bool,
    metrics: FaceMetrics,
    shaper_data: ShaperData,
}

impl std::fmt::Debug for FontFace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontFace")
            .field("family", &self.family)
            .field("bold", &self.bold)
            .field("italic", &self.italic)
            .field("index", &self.index)
            .field("bytes", &self.data.len())
            .finish_non_exhaustive()
    }
}

impl FontFace {
    /// Loads face `index` of the font file `data`. `family`, `bold` and `italic` describe
    /// the face for lookup; they are not read from the file so callers control naming.
    pub fn from_bytes(
        data: Arc<[u8]>,
        index: u32,
        family: &str,
        bold: bool,
        italic: bool,
    ) -> Result<Self, FontError> {
        let font =
            FontRef::from_index(&data, index).map_err(|e| FontError::Parse(e.to_string()))?;
        let head = font.head().map_err(|_| FontError::MissingTable("head"))?;
        let hhea = font.hhea().map_err(|_| FontError::MissingTable("hhea"))?;
        let upem = head.units_per_em();
        if upem == 0 {
            return Err(FontError::ZeroUnitsPerEm);
        }
        let os2 = font.os2().ok();
        let metrics = FaceMetrics {
            units_per_em: upem,
            hhea_ascent: i32::from(hhea.ascender().to_i16()),
            hhea_descent: -i32::from(hhea.descender().to_i16()),
            hhea_line_gap: i32::from(hhea.line_gap().to_i16()),
            win_ascent: os2.as_ref().map_or(0, |t| i32::from(t.us_win_ascent())),
            win_descent: os2.as_ref().map_or(0, |t| i32::from(t.us_win_descent())),
            typo_ascent: os2.as_ref().map_or(0, |t| i32::from(t.s_typo_ascender())),
            typo_descent: os2.as_ref().map_or(0, |t| -i32::from(t.s_typo_descender())),
            typo_line_gap: os2.as_ref().map_or(0, |t| i32::from(t.s_typo_line_gap())),
            use_typo_metrics: os2
                .as_ref()
                .is_some_and(|t| t.fs_selection().bits() & (1 << 7) != 0),
        };
        let shaper_data = ShaperData::new(&font);
        Ok(Self {
            data,
            index,
            family: family.to_owned(),
            bold,
            italic,
            metrics,
            shaper_data,
        })
    }

    pub fn family(&self) -> &str {
        &self.family
    }

    pub fn is_bold(&self) -> bool {
        self.bold
    }

    pub fn is_italic(&self) -> bool {
        self.italic
    }

    pub fn metrics(&self) -> &FaceMetrics {
        &self.metrics
    }

    pub fn units_per_em(&self) -> u16 {
        self.metrics.units_per_em
    }

    pub(crate) fn font_ref(&self) -> Option<FontRef<'_>> {
        FontRef::from_index(&self.data, self.index).ok()
    }

    pub(crate) fn shaper_data(&self) -> &ShaperData {
        &self.shaper_data
    }

    /// True when the font maps `c` to a real glyph (not `.notdef`).
    pub fn has_glyph(&self, c: char) -> bool {
        self.font_ref()
            .is_some_and(|f| f.charmap().map(c).is_some_and(|g| g.to_u32() != 0))
    }
}
