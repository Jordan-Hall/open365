//! Section geometry for layout, taken from the typed `open365-docx` accessors
//! ([`open365_docx::Document::sections`]). Everything else layout needs (tab stops, keep and
//! widow flags, the paragraph mark, the default tab interval) comes straight from
//! `Styles::paragraph_flow_props`, `Styles::paragraph_mark_props` and `Document::settings`.

use open365_docx::Section;
pub use open365_docx::{TabKind, TabLeader, TabStop};

/// Page geometry of one section (`w:sectPr`), in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SectionProps {
    pub page_width: f64,
    pub page_height: f64,
    pub margin_top: f64,
    pub margin_right: f64,
    pub margin_bottom: f64,
    pub margin_left: f64,
    pub gutter: f64,
    /// Distance from the page top to the header (`w:pgMar w:header`).
    pub header_distance: f64,
    /// Distance from the page bottom to the footer (`w:pgMar w:footer`).
    pub footer_distance: f64,
}

impl Default for SectionProps {
    /// US Letter with one inch margins, the format's customary default.
    fn default() -> Self {
        Self::from(&Section::default())
    }
}

fn pt(twips: impl Into<f64>) -> f64 {
    twips.into() / 20.0
}

impl From<&Section> for SectionProps {
    fn from(s: &Section) -> Self {
        Self {
            page_width: pt(s.page.width_twips),
            page_height: pt(s.page.height_twips),
            margin_top: pt(s.margins.top_twips),
            margin_right: pt(s.margins.right_twips),
            margin_bottom: pt(s.margins.bottom_twips),
            margin_left: pt(s.margins.left_twips),
            gutter: pt(s.margins.gutter_twips),
            header_distance: pt(s.margins.header_twips),
            footer_distance: pt(s.margins.footer_twips),
        }
    }
}

impl SectionProps {
    /// Width of the text area.
    pub fn text_width(&self) -> f64 {
        self.page_width - self.margin_left - self.margin_right - self.gutter
    }
}
