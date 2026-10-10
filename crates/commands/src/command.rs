use std::path::{Path, PathBuf};

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
    /// Removes content or files that cannot be recovered from undo.
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
    Save {
        path: PathBuf,
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
            Self::XlsxSetCellValue { .. } => "xlsx_set_cell_value",
            Self::XlsxSetCellFormula { .. } => "xlsx_set_cell_formula",
            Self::Save { .. } => "save",
        }
    }

    /// The effect class. Edits are `Edit`. `Save` is `Export` when it writes a path other
    /// than `current_path` (the file the document was opened from or last saved to, or
    /// none for an unsaved document), and `Edit` when it overwrites that file.
    pub fn effect(&self, current_path: Option<&Path>) -> EffectClass {
        match self {
            Self::Save { path } if current_path == Some(path.as_path()) => EffectClass::Edit,
            Self::Save { .. } => EffectClass::Export,
            _ => EffectClass::Edit,
        }
    }
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
