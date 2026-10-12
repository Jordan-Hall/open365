//! Design tab support for the Writer pane: the commands behind the theme, style set, spacing,
//! page background and watermark controls, and the state their menus show.

use open365_commands::{Command, DesignOp};

use super::WriterPane;

impl WriterPane {
    /// Runs a Design operation as one undo step; false when the command failed.
    pub fn design_apply(&mut self, op: DesignOp) -> bool {
        self.run(Command::DocxDesign { op }).is_some()
    }

    /// The twelve colours of the document theme, when it has one.
    #[must_use]
    pub fn theme_colours(&self) -> Option<Vec<String>> {
        self.target.docx()?.theme_colours()
    }

    /// The page colour (`RRGGBB`), when the document has one.
    #[must_use]
    pub fn page_colour(&self) -> Option<String> {
        self.target.docx()?.page_colour()
    }

    /// The text of the watermark in the document headers, if any.
    #[must_use]
    pub fn watermark_text(&self) -> Option<String> {
        self.target.docx()?.watermark_text()
    }
}
