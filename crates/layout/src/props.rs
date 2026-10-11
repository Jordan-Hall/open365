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

/// How a section starts, `w:type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SectionStart {
    NextPage,
    /// On the same page, below the previous section; the previous section's columns are
    /// balanced.
    Continuous,
    NextColumn,
    EvenPage,
    OddPage,
}

/// One text column: the offset of its left edge from the left edge of the text area, and its
/// width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ColumnGeom {
    pub x: f64,
    pub width: f64,
}

/// The columns of a section (`w:cols`) and how the section starts.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SectionCols {
    pub start: SectionStart,
    pub columns: Vec<ColumnGeom>,
    /// `w:sep`: a line between the columns.
    pub separator: bool,
}

impl SectionCols {
    pub(crate) fn from_section(s: &Section, text_width: f64) -> Self {
        use open365_docx::SectionType;
        let start = match s.kind {
            SectionType::NextPage => SectionStart::NextPage,
            SectionType::Continuous => SectionStart::Continuous,
            SectionType::NextColumn => SectionStart::NextColumn,
            SectionType::EvenPage => SectionStart::EvenPage,
            SectionType::OddPage => SectionStart::OddPage,
        };
        let c = &s.columns;
        let space = pt(c.space_twips);
        let mut columns = Vec::new();
        if !c.equal_width && !c.columns.is_empty() {
            // Explicit columns: each carries the space after it (the default space otherwise).
            let mut x = 0.0;
            for (i, col) in c.columns.iter().enumerate() {
                let width = pt(col.width_twips);
                columns.push(ColumnGeom { x, width });
                let gap = col.space_twips.map_or(space, pt);
                if i + 1 < c.columns.len() {
                    x += width + gap;
                }
            }
        } else {
            let n = c.count.max(1) as usize;
            let width = (text_width - space * (n - 1) as f64) / n as f64;
            for i in 0..n {
                columns.push(ColumnGeom {
                    x: i as f64 * (width + space),
                    width,
                });
            }
        }
        let separator = c.separator && columns.len() > 1;
        Self {
            start,
            columns,
            separator,
        }
    }

    pub(crate) fn is_multi(&self) -> bool {
        self.columns.len() > 1
    }

    /// Width of the first column, the width a unit is laid out at.
    pub(crate) fn first_width(&self) -> f64 {
        self.columns.first().map_or(0.0, |c| c.width)
    }
}
