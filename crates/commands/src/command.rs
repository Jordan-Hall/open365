use std::path::{Path, PathBuf};

use open365_docx::{Alignment, DocPos, RunPropsPatch};
use serde::{Deserialize, Serialize};

use crate::CommandError;

/// What a command does to the world outside the in-memory document. A permission layer
/// (ADR-002) decides on this class before the command runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectClass {
    /// Reads document state; changes nothing.
    Read,
    /// Changes the open document in memory, or overwrites the file it was opened from.
    Edit,
    /// Removes existing content. Undo restores it within the session, but a permission layer
    /// should still treat it as the riskiest in-document class.
    Destructive,
    /// Sends data off this machine.
    Egress,
    /// Writes a new file outside the document's own path.
    Export,
}

/// A user-visible cell constant, independent of the file format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum CellEntry {
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
}

impl CellEntry {
    /// Interprets text typed into a cell: blank clears, `TRUE`/`FALSE` are booleans,
    /// a finite number is a number, anything else is text.
    pub fn from_user_text(text: &str) -> Self {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Self::Empty;
        }
        if trimmed.eq_ignore_ascii_case("true") {
            return Self::Bool(true);
        }
        if trimmed.eq_ignore_ascii_case("false") {
            return Self::Bool(false);
        }
        match trimmed.parse::<f64>() {
            Ok(n) if n.is_finite() => Self::Number(n),
            _ => Self::Text(text.to_owned()),
        }
    }
}

/// Every mutation the application can perform. Offsets are Unicode scalar values;
/// spreadsheet rows, columns are 1-based and sheets 0-based, as in the model crates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    DocxInsertText {
        para: usize,
        offset: usize,
        text: String,
    },
    /// Sets or clears bold on the half-open character range `start..end`.
    DocxSetBold {
        para: usize,
        start: usize,
        end: usize,
        bold: bool,
    },
    /// Deletes the characters between two positions, merging paragraphs across the range.
    DocxDeleteRange {
        start: DocPos,
        end: DocPos,
    },
    /// Enter: splits the paragraph at `pos`.
    DocxSplitParagraph {
        pos: DocPos,
    },
    /// Applies run-property changes over a character range (may span paragraphs).
    DocxSetRunProps {
        start: DocPos,
        end: DocPos,
        patch: RunPropsPatch,
    },
    /// `style_id` is a `w:styleId`; the default style removes `w:pStyle`.
    DocxSetParagraphStyle {
        para: usize,
        style_id: String,
    },
    DocxSetAlignment {
        para: usize,
        alignment: Alignment,
    },
    /// Typing over a selection or pasting: deletes the range, then inserts `text` there.
    /// `\n` in `text` starts a new paragraph.
    DocxReplaceRange {
        start: DocPos,
        end: DocPos,
        text: String,
    },
    /// Word's Replace All: every match of `find` becomes `replacement` (`^&` is the found text,
    /// `^t` a tab, `^l` a line break). Matches span runs; the replacement takes the formatting
    /// of the first matched run. One undo step.
    DocxReplaceAll {
        find: String,
        replacement: String,
        #[serde(default)]
        match_case: bool,
        #[serde(default)]
        whole_word: bool,
        #[serde(default)]
        wildcards: bool,
    },
    /// Excel's Replace All over the formula text (see [`crate::XlsxFind`]; `look_in` is
    /// ignored, replace always works on formulas). One undo step.
    XlsxReplaceAll {
        find: crate::XlsxFind,
        replacement: String,
    },
    /// Pastes a block of cell entries (typed text, `=` for formulas, empty to clear) with its
    /// top-left cell at `row`, `col`. One undo step.
    XlsxPaste {
        sheet: usize,
        row: u32,
        col: u32,
        rows: Vec<Vec<String>>,
    },
    XlsxSetCellValue {
        sheet: usize,
        row: u32,
        col: u32,
        value: CellEntry,
    },
    /// `formula` may start with `=`.
    XlsxSetCellFormula {
        sheet: usize,
        row: u32,
        col: u32,
        formula: String,
    },
    /// Sets the stored width of columns `first..=last` (1-based, inclusive) in characters of
    /// the default font, cell padding included: 0 up to 255 characters plus the padding (about 255.78), as
    /// Excel allows: its cap counts the padding (2302 px for 9 px digits).
    SetColumnWidth {
        sheet: usize,
        first: u32,
        last: u32,
        width: f64,
    },
    /// Sets the height of rows `first..=last` (1-based, inclusive) in points: 0..=409.5.
    SetRowHeight {
        sheet: usize,
        first: u32,
        last: u32,
        points: f64,
    },
    /// Inserts `count` blank rows before row `at` (1-based) of a worksheet, as Excel's Insert does:
    /// cells at and below move down, formulas anywhere in the workbook that point at the sheet are
    /// rewritten, ranges that span the insertion point grow.
    InsertRows {
        sheet: usize,
        at: u32,
        #[serde(default = "one")]
        count: u32,
    },
    /// Deletes rows `at..at+count`. References to deleted cells become `#REF!`; ranges shrink.
    DeleteRows {
        sheet: usize,
        at: u32,
        #[serde(default = "one")]
        count: u32,
    },
    /// Inserts `count` blank columns before column `at` (1-based).
    InsertColumns {
        sheet: usize,
        at: u32,
        #[serde(default = "one")]
        count: u32,
    },
    /// Deletes columns `at..at+count`.
    DeleteColumns {
        sheet: usize,
        at: u32,
        #[serde(default = "one")]
        count: u32,
    },
    /// Hides columns `first..=last`; their stored widths are kept (Excel's `Hidden = True`).
    HideColumns {
        sheet: usize,
        first: u32,
        last: u32,
    },
    /// Shows columns `first..=last`. A column without a width of its own gets
    /// `default_width` (stored characters), else the default for the workbook's default font
    /// at 96 dpi.
    UnhideColumns {
        sheet: usize,
        first: u32,
        last: u32,
        #[serde(default)]
        default_width: Option<f64>,
    },
    /// Hides rows `first..=last`; their stored heights are kept.
    HideRows {
        sheet: usize,
        first: u32,
        last: u32,
    },
    /// Shows the hidden rows of `first..=last`.
    UnhideRows {
        sheet: usize,
        first: u32,
        last: u32,
    },
    Save {
        path: PathBuf,
    },
    /// Lays the DOCX out and writes a PDF to `path` (effect `Export`). Without `overwrite` the
    /// file is created exclusively and an existing file is an error; the document, its undo
    /// history and its saved state are untouched. Metadata and fonts come from the target's
    /// [`crate::PdfOptions`].
    ExportPdf {
        path: PathBuf,
        #[serde(default)]
        overwrite: bool,
    },
}

impl Command {
    /// The command for text typed into a grid cell: `=...` is a formula, anything else a
    /// constant (see [`CellEntry::from_user_text`]).
    pub fn xlsx_enter(sheet: usize, row: u32, col: u32, text: &str) -> Self {
        if text.trim_start().starts_with('=') {
            Self::XlsxSetCellFormula {
                sheet,
                row,
                col,
                formula: text.trim_start().to_owned(),
            }
        } else {
            Self::XlsxSetCellValue {
                sheet,
                row,
                col,
                value: CellEntry::from_user_text(text),
            }
        }
    }

    /// Short stable name, as used in the serialised form and in error messages.
    pub fn name(&self) -> &'static str {
        match self {
            Self::DocxInsertText { .. } => "docx_insert_text",
            Self::DocxSetBold { .. } => "docx_set_bold",
            Self::DocxDeleteRange { .. } => "docx_delete_range",
            Self::DocxSplitParagraph { .. } => "docx_split_paragraph",
            Self::DocxSetRunProps { .. } => "docx_set_run_props",
            Self::DocxSetParagraphStyle { .. } => "docx_set_paragraph_style",
            Self::DocxSetAlignment { .. } => "docx_set_alignment",
            Self::DocxReplaceRange { .. } => "docx_replace_range",
            Self::DocxReplaceAll { .. } => "docx_replace_all",
            Self::XlsxReplaceAll { .. } => "xlsx_replace_all",
            Self::XlsxPaste { .. } => "xlsx_paste",
            Self::XlsxSetCellValue { .. } => "xlsx_set_cell_value",
            Self::XlsxSetCellFormula { .. } => "xlsx_set_cell_formula",
            Self::SetColumnWidth { .. } => "set_column_width",
            Self::SetRowHeight { .. } => "set_row_height",
            Self::InsertRows { .. } => "insert_rows",
            Self::DeleteRows { .. } => "delete_rows",
            Self::InsertColumns { .. } => "insert_columns",
            Self::DeleteColumns { .. } => "delete_columns",
            Self::HideColumns { .. } => "hide_columns",
            Self::UnhideColumns { .. } => "unhide_columns",
            Self::HideRows { .. } => "hide_rows",
            Self::UnhideRows { .. } => "unhide_rows",
            Self::Save { .. } => "save",
            Self::ExportPdf { .. } => "export_pdf",
        }
    }

    /// The effect class. Formatting and insertion are `Edit`; deleting or replacing a
    /// non-empty range, and deleting rows or columns, is `Destructive`. `Save` is `Export` when it writes a path other
    /// than `current_path` (the file the document was opened from or last saved to, or
    /// none for an unsaved document), and `Edit` when it overwrites that file.
    pub fn effect(&self, current_path: Option<&Path>) -> EffectClass {
        match self {
            Self::Save { path } if current_path == Some(path.as_path()) => EffectClass::Edit,
            Self::Save { .. } | Self::ExportPdf { .. } => EffectClass::Export,
            Self::DocxDeleteRange { start, end } | Self::DocxReplaceRange { start, end, .. }
                if start != end =>
            {
                EffectClass::Destructive
            }
            Self::DocxReplaceAll { .. } | Self::XlsxReplaceAll { .. } => EffectClass::Destructive,
            Self::DeleteRows { .. } | Self::DeleteColumns { .. } => EffectClass::Destructive,
            _ => EffectClass::Edit,
        }
    }
}

fn one() -> u32 {
    1
}

/// A spreadsheet cell address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CellRef {
    pub sheet: usize,
    pub row: u32,
    pub col: u32,
}

/// What an applied command did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub effect: EffectClass,
    /// Cells whose stored value or formula changed, including recalculated dependents.
    pub changed_cells: Vec<CellRef>,
    /// Set by `Save`: where the file was written.
    pub saved_to: Option<PathBuf>,
    pub bytes_written: usize,
    /// Set by `ExportPdf`: the number of pages written.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pages: usize,
    /// Content the edit removed besides plain text (drawings, fields, section breaks).
    #[serde(default)]
    pub notes: Vec<String>,
    /// Set by the replace-all commands: how many replacements were made.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub replaced: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// The audit log of applied commands, in order. Replaying it on a fresh target reproduces
/// the document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CommandLog(pub Vec<Command>);

impl CommandLog {
    pub fn to_json(&self) -> Result<String, CommandError> {
        serde_json::to_string_pretty(self).map_err(|e| CommandError::Log(e.to_string()))
    }

    pub fn from_json(json: &str) -> Result<Self, CommandError> {
        serde_json::from_str(json).map_err(|e| CommandError::Log(e.to_string()))
    }

    pub fn commands(&self) -> &[Command] {
        &self.0
    }
}
