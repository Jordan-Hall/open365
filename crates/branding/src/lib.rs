//! Single source of truth for Open365 user-visible product names.
//!
//! Every window title, dialog, backstage page, installer string and CLI help
//! text takes its product name from here. Interoperability wording that names
//! a third-party file format lives in the `FORMAT_*` constants.

/// The suite name.
pub const SUITE: &str = "Open365";
/// The word processor.
pub const WRITE_NAME: &str = "Open Write";
/// The spreadsheet application.
pub const SHEET_NAME: &str = "Open Sheet";
/// The planned communication product.
pub const TEAMS_NAME: &str = "Open Teams";

/// Windows executable names (without extension).
pub const WRITE_EXE: &str = "open-write";
/// Windows executable name for the spreadsheet app (without extension).
pub const SHEET_EXE: &str = "open-sheet";

/// Untitled document default window title.
pub const WRITE_UNTITLED: &str = "Document1";
/// Untitled workbook default window title.
pub const SHEET_UNTITLED: &str = "Book1";

/// File-type description for `.docx` family filters.
pub const FORMAT_DOCX: &str = "Document (*.docx)";
/// File-type description for `.xlsx` family filters.
pub const FORMAT_XLSX: &str = "Workbook (*.xlsx)";
/// Generic description for a document file.
pub const FORMAT_DOCUMENT: &str = "Document";
/// Generic description for a workbook file.
pub const FORMAT_WORKBOOK: &str = "Workbook";

/// Window title with the suite app name, e.g. `Report.docx - Open Write`.
#[must_use]
pub fn window_title(file: &str, app: &str) -> String {
    format!("{file} - {app}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_do_not_use_third_party_marks() {
        for s in [SUITE, WRITE_NAME, SHEET_NAME, TEAMS_NAME] {
            assert!(!s.contains("Word") && !s.contains("Excel") && !s.contains("Office"));
        }
    }

    #[test]
    fn title_format() {
        assert_eq!(window_title("a.docx", WRITE_NAME), "a.docx - Open Write");
    }
}
