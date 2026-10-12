//! Layout tab support for the Writer pane: which sections and paragraphs a ribbon change
//! applies to, and the commands that make it.

use open365_commands::{Command, LayoutOp};
use open365_docx::{DocPos, Section};

use super::WriterPane;

impl WriterPane {
    /// The sections the selection touches (the caret's section for a caret), as the Layout
    /// tab's Page Setup commands apply to them.
    #[must_use]
    pub fn layout_sections(&self) -> Vec<usize> {
        let Some(doc) = self.target.docx() else {
            return Vec::new();
        };
        let (a, b) = self.selection();
        (doc.section_of_paragraph(a.para)..=doc.section_of_paragraph(b.para)).collect()
    }

    /// The paragraphs of the selection.
    #[must_use]
    pub fn layout_paras(&self) -> Vec<usize> {
        let (a, b) = self.selection();
        (a.para..=b.para.min(self.models.len().saturating_sub(1))).collect()
    }

    /// The section the caret is in.
    #[must_use]
    pub fn caret_section(&self) -> Option<Section> {
        let doc = self.target.docx()?;
        let sections = doc.sections();
        sections
            .into_iter()
            .nth(doc.section_of_paragraph(self.caret.para))
    }

    /// The width of the text area of the caret's section in points (page width less margins
    /// and gutter): what the column presets divide.
    #[must_use]
    pub fn text_width_pt(&self) -> f64 {
        self.caret_section().map_or(468.0, |s| {
            let m = s.margins;
            f64::from(s.page.width_twips as i32 - m.left_twips - m.right_twips - m.gutter_twips)
                / 20.0
        })
    }

    /// Runs a Layout operation as one undo step; false (with the reason in [`WriterPane::status`])
    /// when the command failed.
    pub fn layout_apply(&mut self, op: LayoutOp) -> bool {
        self.run(Command::DocxLayout { op }).is_some()
    }

    /// Inserts a break at the caret (replacing the selection first, as typing does). Page,
    /// column and section breaks leave the caret at the start of the next paragraph.
    pub fn layout_break(&mut self, op: impl FnOnce(DocPos) -> LayoutOp, moves_on: bool) -> bool {
        if self.has_selection() {
            self.delete_selection();
        }
        let at = self.caret;
        if !self.layout_apply(op(at)) {
            return false;
        }
        self.set_caret(if moves_on {
            DocPos {
                para: at.para + 1,
                offset: 0,
            }
        } else {
            DocPos {
                para: at.para,
                offset: at.offset + 1,
            }
        });
        self.reveal_caret();
        true
    }

    /// Whether every paragraph of `paras` has Suppress Line Numbers.
    #[must_use]
    pub fn all_suppressed(&self, paras: &[usize]) -> bool {
        let Some(doc) = self.target.docx() else {
            return false;
        };
        !paras.is_empty()
            && paras.iter().all(|&i| {
                doc.paragraph(i).and_then(|p| p.props.as_ref()).is_some_and(|ppr| {
                    ppr.children.iter().any(|c| {
                        matches!(c, open365_docx::RawXml::Element(e) if e.head.is_w("suppressLineNumbers"))
                    })
                })
            })
    }

    /// Whether the document hyphenates automatically.
    #[must_use]
    pub fn hyphenation_on(&self) -> bool {
        self.target
            .docx()
            .is_some_and(|d| d.settings().auto_hyphenation)
    }

    /// The number of sections in the document.
    #[must_use]
    pub fn section_count(&self) -> usize {
        self.target.docx().map_or(1, |d| d.sections().len())
    }
}
