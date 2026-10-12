//! Section page margins as one edit (ruler drag, Page Setup dialog): top, bottom, left, right,
//! gutter, and the header and footer distances of a `w:sectPr`'s `w:pgMar`.

use crate::DocxError;
use crate::document::Document;
use crate::ext_layout::upsert_sect;

/// New page margins for a section, in twips. `None` leaves a value as it is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MarginEdit {
    pub top: Option<i32>,
    pub bottom: Option<i32>,
    pub left: Option<i32>,
    pub right: Option<i32>,
    pub gutter: Option<i32>,
    /// Distance from the page top to the header.
    pub header: Option<i32>,
    /// Distance from the page bottom to the footer.
    pub footer: Option<i32>,
}

/// Largest margin, 22 inches.
const MAX_TWIPS: i32 = 31_680;

/// The smallest text area a margin change may leave, 1 point (Word refuses margins that leave
/// no text area).
const MIN_TEXT_TWIPS: i64 = 20;

impl Document {
    /// Sets the page margins of `section`; values not given keep their current (or default)
    /// value, and the whole `w:pgMar` is written so no attribute is missing.
    ///
    /// # Errors
    /// `section` does not exist, a value is negative or beyond 22 inches, or the margins would
    /// leave the page no text area.
    pub fn set_section_margins(
        &mut self,
        section: usize,
        edit: &MarginEdit,
    ) -> Result<(), DocxError> {
        let sections = self.sections();
        let Some(current) = sections.get(section) else {
            return Err(DocxError::UnsupportedRange(format!(
                "no section {section} (the document has {})",
                sections.len()
            )));
        };
        let m = current.margins;
        let new = [
            ("top", edit.top.unwrap_or(m.top_twips)),
            ("right", edit.right.unwrap_or(m.right_twips)),
            ("bottom", edit.bottom.unwrap_or(m.bottom_twips)),
            ("left", edit.left.unwrap_or(m.left_twips)),
            ("header", edit.header.unwrap_or(m.header_twips)),
            ("footer", edit.footer.unwrap_or(m.footer_twips)),
            ("gutter", edit.gutter.unwrap_or(m.gutter_twips)),
        ];
        for (name, v) in new {
            if !(0..=MAX_TWIPS).contains(&v) {
                return Err(DocxError::UnsupportedRange(format!(
                    "{name} margin of {v} twips"
                )));
            }
        }
        let get =
            |name: &str| i64::from(new.iter().find(|(n, _)| *n == name).map_or(0, |(_, v)| *v));
        let width =
            i64::from(current.page.width_twips) - get("left") - get("right") - get("gutter");
        let height = i64::from(current.page.height_twips) - get("top") - get("bottom");
        if width < MIN_TEXT_TWIPS || height < MIN_TEXT_TWIPS {
            return Err(DocxError::UnsupportedRange(
                "the margins leave no room for text on the page".to_owned(),
            ));
        }
        let sect = self.section_pr_mut(section)?;
        upsert_sect(sect, "pgMar", |e| {
            for (name, v) in new {
                e.head.set_attr_w(name, &v.to_string());
            }
        });
        self.edited();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_edit_changes_nothing() {
        assert_eq!(MarginEdit::default().top, None);
    }
}
