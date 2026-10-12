//! Writer page margins of one section as one undo step: the ruler drag and the Page Setup
//! dialog. Lengths are points; a side left out keeps its value.

use open365_docx::{Document, MarginEdit};
use serde::{Deserialize, Serialize};

use crate::EffectClass;
use crate::error::CommandError;

/// New margins for section `section` (numbered as in `Document::sections`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SectionMargins {
    pub section: usize,
    #[serde(default)]
    pub top_pt: Option<f64>,
    #[serde(default)]
    pub bottom_pt: Option<f64>,
    #[serde(default)]
    pub left_pt: Option<f64>,
    #[serde(default)]
    pub right_pt: Option<f64>,
    #[serde(default)]
    pub gutter_pt: Option<f64>,
    /// Distance from the page top to the header.
    #[serde(default)]
    pub header_pt: Option<f64>,
    /// Distance from the page bottom to the footer.
    #[serde(default)]
    pub footer_pt: Option<f64>,
}

impl SectionMargins {
    #[must_use]
    pub fn effect(&self) -> EffectClass {
        EffectClass::Edit
    }
}

/// Points to twips; rejects values that are not finite or beyond 22 inches.
fn twips(pt: Option<f64>) -> Result<Option<i32>, CommandError> {
    pt.map(|v| {
        if !v.is_finite() || !(0.0..=1584.0).contains(&v) {
            return Err(CommandError::Format(format!(
                "{v} pt is not a usable margin"
            )));
        }
        Ok((v * 20.0).round() as i32)
    })
    .transpose()
}

/// Runs the edit on `doc`.
///
/// # Errors
/// A length is not usable, the section does not exist, or the margins leave no text area;
/// the document is unchanged then.
pub fn apply(doc: &mut Document, op: &SectionMargins) -> Result<(), CommandError> {
    let edit = MarginEdit {
        top: twips(op.top_pt)?,
        bottom: twips(op.bottom_pt)?,
        left: twips(op.left_pt)?,
        right: twips(op.right_pt)?,
        gutter: twips(op.gutter_pt)?,
        header: twips(op.header_pt)?,
        footer: twips(op.footer_pt)?,
    };
    doc.set_section_margins(op.section, &edit)?;
    Ok(())
}
