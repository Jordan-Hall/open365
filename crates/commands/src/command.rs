use std::path::{Path, PathBuf};

use open365_docx::{Alignment, DocPos, RunPropsPatch};
use serde::{Deserialize, Serialize};

use crate::CommandError;
use crate::format::{BorderChange, CellFormatChange, RangeRef};
use crate::target::Target;
use open365_xlsx::CellValue;

/// What a command does to the world outside the in-memory document. A permission layer
/// (ADR-002) decides on this class before the command runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectClass {
    /// Reads document state; changes nothing.
    Read,
    /// Changes the open document in memory without removing existing content.
    Edit,
    /// Removes or overwrites existing content, or replaces the file the document was opened
    /// from. Undo restores in-memory content within the session, but a permission layer should
    /// still treat it as the riskiest in-document class.
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
    /// Ruler or Paragraph dialog: left, first-line (negative: hanging) and right indent of one
    /// paragraph, in points (`w:ind`).
    DocxSetIndents {
        para: usize,
        left_pt: f64,
        first_line_pt: f64,
        right_pt: f64,
    },
    /// Adds a custom tab stop `pos_pt` points from the left margin (`w:tabs`).
    DocxAddTab {
        para: usize,
        pos_pt: f64,
        #[serde(default)]
        kind: open365_docx::TabAlign,
    },
    /// Moves the tab stop at `from_pt` to `to_pt`, keeping its alignment.
    DocxMoveTab {
        para: usize,
        from_pt: f64,
        to_pt: f64,
    },
    /// Clears the tab stop at `pos_pt`.
    DocxClearTab {
        para: usize,
        pos_pt: f64,
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
    /// Stores measured auto-fit widths, `(column, stored width)` pairs (1-based columns, the unit of
    /// `SetColumnWidth`), as Excel's AutoFit Column Width does: the columns get `bestFit`. The
    /// caller measures the text; the command only records the result.
    AutoFitColumns {
        sheet: usize,
        columns: Vec<(u32, f64)>,
    },
    /// Stores measured auto-fit heights, `(row, points)` pairs (1-based rows, 0..=409.5), as Excel's
    /// AutoFit Row Height does and as it re-fits a row after wrapping or a font change: the rows
    /// lose their `customHeight` flag. The caller measures the text.
    AutoFitRows {
        sheet: usize,
        rows: Vec<(u32, f64)>,
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
    /// Sets font, fill, alignment, number format or protection on a range (Excel's Home tab
    /// buttons and Format Cells dialog). Whole rows or columns are formatted as rows or columns.
    FormatCells {
        sheet: usize,
        range: RangeRef,
        change: Box<CellFormatChange>,
    },
    /// The border menu: outline, inside lines, single edges and diagonals of a range.
    SetBorders {
        sheet: usize,
        range: RangeRef,
        borders: Box<BorderChange>,
    },
    /// Applies a named cell style (`Good`, `Heading 1`, `Percent`, `Comma`, `Currency`, ...).
    ApplyCellStyle {
        sheet: usize,
        range: RangeRef,
        style: String,
    },
    /// Clear Formats: back to the Normal style.
    ClearFormats {
        sheet: usize,
        range: RangeRef,
    },
    /// Format Painter: copies the formats of `from` onto `to`, tiling the source.
    CopyFormat {
        sheet: usize,
        from: RangeRef,
        to: RangeRef,
    },
    /// Increase (`delta` > 0) or Decrease Decimal. Nothing changes for text or a blank General cell.
    ChangeDecimals {
        sheet: usize,
        range: RangeRef,
        delta: i32,
    },
    /// Merge & Center, Merge Across, Merge Cells or Unmerge Cells on a range. Merging keeps only
    /// the upper-left value of each merged area and empties the other cells (the UI warns first, see
    /// [`crate::merge_discards`]). One undo step.
    MergeCells {
        sheet: usize,
        range: RangeRef,
        mode: crate::MergeMode,
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
            Self::DocxSetIndents { .. } => "docx_set_indents",
            Self::DocxAddTab { .. } => "docx_add_tab",
            Self::DocxMoveTab { .. } => "docx_move_tab",
            Self::DocxClearTab { .. } => "docx_clear_tab",
            Self::DocxReplaceRange { .. } => "docx_replace_range",
            Self::DocxReplaceAll { .. } => "docx_replace_all",
            Self::XlsxReplaceAll { .. } => "xlsx_replace_all",
            Self::XlsxPaste { .. } => "xlsx_paste",
            Self::XlsxSetCellValue { .. } => "xlsx_set_cell_value",
            Self::XlsxSetCellFormula { .. } => "xlsx_set_cell_formula",
            Self::SetColumnWidth { .. } => "set_column_width",
            Self::SetRowHeight { .. } => "set_row_height",
            Self::AutoFitColumns { .. } => "auto_fit_columns",
            Self::AutoFitRows { .. } => "auto_fit_rows",
            Self::InsertRows { .. } => "insert_rows",
            Self::DeleteRows { .. } => "delete_rows",
            Self::InsertColumns { .. } => "insert_columns",
            Self::DeleteColumns { .. } => "delete_columns",
            Self::HideColumns { .. } => "hide_columns",
            Self::UnhideColumns { .. } => "unhide_columns",
            Self::HideRows { .. } => "hide_rows",
            Self::UnhideRows { .. } => "unhide_rows",
            Self::FormatCells { .. } => "format_cells",
            Self::SetBorders { .. } => "set_borders",
            Self::ApplyCellStyle { .. } => "apply_cell_style",
            Self::ClearFormats { .. } => "clear_formats",
            Self::CopyFormat { .. } => "copy_format",
            Self::ChangeDecimals { .. } => "change_decimals",
            Self::MergeCells { .. } => "merge_cells",
            Self::Save { .. } => "save",
            Self::ExportPdf { .. } => "export_pdf",
        }
    }

    /// The effect class from the command alone. Formatting and insertion are `Edit`; deleting
    /// or replacing a non-empty range, and deleting rows or columns, is `Destructive`. `Save`
    /// is `Export` when it writes a path other than `current_path` (the file the document was
    /// opened from or last saved to, or none for an unsaved document), `Destructive` when it
    /// overwrites that file, and `Edit` when that file is gone. Writes whose effect depends on
    /// what the document holds now (a cell overwrite, a paste) are `Edit` here; use
    /// [`Command::effect_in`] to classify them against a target.
    pub fn effect(&self, current_path: Option<&Path>) -> EffectClass {
        // No `_` arm: a new command does not compile until it is classified here and in
        // `overwrites_content`.
        match self {
            Self::Save { path } if current_path == Some(path.as_path()) => {
                if path.exists() {
                    EffectClass::Destructive
                } else {
                    EffectClass::Edit
                }
            }
            Self::Save { .. } | Self::ExportPdf { .. } => EffectClass::Export,
            Self::DocxDeleteRange { start, end } | Self::DocxReplaceRange { start, end, .. } => {
                if start == end {
                    EffectClass::Edit
                } else {
                    EffectClass::Destructive
                }
            }
            Self::DocxReplaceAll { .. }
            | Self::XlsxReplaceAll { .. }
            | Self::DeleteRows { .. }
            | Self::DeleteColumns { .. } => EffectClass::Destructive,
            Self::DocxInsertText { .. }
            | Self::DocxSetBold { .. }
            | Self::DocxSplitParagraph { .. }
            | Self::DocxSetRunProps { .. }
            | Self::DocxSetParagraphStyle { .. }
            | Self::DocxSetAlignment { .. }
            | Self::DocxSetIndents { .. }
            | Self::DocxAddTab { .. }
            | Self::DocxMoveTab { .. }
            | Self::DocxClearTab { .. }
            | Self::XlsxPaste { .. }
            | Self::XlsxSetCellValue { .. }
            | Self::XlsxSetCellFormula { .. }
            | Self::SetColumnWidth { .. }
            | Self::SetRowHeight { .. }
            | Self::AutoFitColumns { .. }
            | Self::AutoFitRows { .. }
            | Self::InsertRows { .. }
            | Self::InsertColumns { .. }
            | Self::HideColumns { .. }
            | Self::UnhideColumns { .. }
            | Self::HideRows { .. }
            | Self::UnhideRows { .. }
            | Self::FormatCells { .. }
            | Self::SetBorders { .. }
            | Self::ApplyCellStyle { .. }
            | Self::ClearFormats { .. }
            | Self::CopyFormat { .. }
            | Self::ChangeDecimals { .. }
            | Self::MergeCells { .. } => EffectClass::Edit,
        }
    }

    /// The effect class by actual effect on `target`: [`Command::effect`], raised to
    /// `Destructive` when the command overwrites a non-empty cell (`xlsx_set_cell_value`,
    /// `xlsx_set_cell_formula`), pastes over non-empty cells (`xlsx_paste`) or clears formatting that exists
    /// (`clear_formats`), or a merge that would empty cells (`merge_cells`; a scan error counts as
    /// destructive). A permission layer must classify with this, against the state the
    /// command will run on (for a batch, the state after the earlier commands).
    pub fn effect_in(&self, target: &Target) -> EffectClass {
        let base = self.effect(target.path());
        if base == EffectClass::Edit && self.overwrites_content(target) {
            EffectClass::Destructive
        } else {
            base
        }
    }

    /// Whether an `Edit` command would, on `target`, overwrite or discard something that exists.
    /// Fails closed: a scan that errors counts as destructive. No `_` arm, as in `effect`.
    fn overwrites_content(&self, target: &Target) -> bool {
        let Some(wb) = target.xlsx() else {
            return false;
        };
        let filled = |sheet: usize, row: u32, col: u32| {
            wb.cell(sheet, row, col)
                .is_some_and(|c| !matches!(c.value, CellValue::Empty) || c.formula.is_some())
        };
        match self {
            // Any write over a filled cell, by value (an empty one included) or by formula,
            // replaces what is there. The agent surfaces classify with this; an interactive
            // UI, where the person at the keyboard is the authority, may use `effect` instead.
            Self::XlsxSetCellValue {
                sheet, row, col, ..
            }
            | Self::XlsxSetCellFormula {
                sheet, row, col, ..
            } => filled(*sheet, *row, *col),
            Self::XlsxPaste {
                sheet,
                row,
                col,
                rows,
            } => rows.iter().enumerate().any(|(dr, line)| {
                (0..line.len()).any(|dc| {
                    let (Some(r), Some(c)) = (
                        u32::try_from(dr).ok().and_then(|d| row.checked_add(d)),
                        u32::try_from(dc).ok().and_then(|d| col.checked_add(d)),
                    ) else {
                        return false;
                    };
                    filled(*sheet, r, c)
                })
            }),
            Self::ClearFormats { sheet, range } => {
                let (r1, c1, r2, c2) = range.rect();
                let styled = |c: &open365_xlsx::Cell| c.style.is_some_and(|s| s != 0);
                // Whole rows or columns may carry row or column formats of their own.
                if (r1 <= 1 && r2 >= open365_xlsx::MAX_ROW)
                    || (c1 <= 1 && c2 >= open365_xlsx::MAX_COL)
                {
                    return true;
                }
                wb.iter_cells(*sheet)
                    .any(|(r, c, cell)| r1 <= r && r <= r2 && c1 <= c && c <= c2 && styled(cell))
            }
            // Merging keeps only the upper-left value of each area and empties the rest.
            Self::MergeCells { sheet, range, mode } => {
                !matches!(crate::merge_discards(wb, *sheet, *range, *mode), Ok(v) if v.is_empty())
            }
            Self::DocxInsertText { .. }
            | Self::DocxSetBold { .. }
            | Self::DocxDeleteRange { .. }
            | Self::DocxSplitParagraph { .. }
            | Self::DocxSetRunProps { .. }
            | Self::DocxSetParagraphStyle { .. }
            | Self::DocxSetAlignment { .. }
            | Self::DocxSetIndents { .. }
            | Self::DocxAddTab { .. }
            | Self::DocxMoveTab { .. }
            | Self::DocxClearTab { .. }
            | Self::DocxReplaceRange { .. }
            | Self::DocxReplaceAll { .. }
            | Self::XlsxReplaceAll { .. }
            | Self::SetColumnWidth { .. }
            | Self::SetRowHeight { .. }
            | Self::AutoFitColumns { .. }
            | Self::AutoFitRows { .. }
            | Self::InsertRows { .. }
            | Self::DeleteRows { .. }
            | Self::InsertColumns { .. }
            | Self::DeleteColumns { .. }
            | Self::HideColumns { .. }
            | Self::UnhideColumns { .. }
            | Self::HideRows { .. }
            | Self::UnhideRows { .. }
            | Self::FormatCells { .. }
            | Self::SetBorders { .. }
            | Self::ApplyCellStyle { .. }
            | Self::CopyFormat { .. }
            | Self::ChangeDecimals { .. }
            | Self::Save { .. }
            | Self::ExportPdf { .. } => false,
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

#[cfg(test)]
mod effect_tests {
    use super::*;
    use crate::MergeMode;

    fn workbook() -> Target {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/xlsx/formulas-basic.xlsx");
        Target::open_path(&path).expect("open xlsx")
    }

    fn put(t: &mut Target, row: u32, col: u32, text: &str) {
        let c = Command::XlsxSetCellValue {
            sheet: 0,
            row,
            col,
            value: CellEntry::Text(text.into()),
        };
        t.apply(c).expect("set");
    }

    fn range(r1: u32, c1: u32, r2: u32, c2: u32) -> RangeRef {
        RangeRef {
            first_row: r1,
            first_col: c1,
            last_row: r2,
            last_col: c2,
        }
    }

    #[test]
    fn emptying_a_filled_cell_is_destructive_and_emptying_a_blank_one_is_not() {
        let mut t = workbook();
        put(&mut t, 200, 3, "data");
        let empty = |row, col| Command::XlsxSetCellValue {
            sheet: 0,
            row,
            col,
            value: CellEntry::Empty,
        };
        assert_eq!(empty(200, 3).effect_in(&t), EffectClass::Destructive);
        assert_eq!(empty(201, 3).effect_in(&t), EffectClass::Edit);
        // The static class cannot see the document.
        assert_eq!(empty(200, 3).effect(None), EffectClass::Edit);
    }

    #[test]
    fn a_value_or_formula_over_a_filled_cell_is_destructive() {
        let mut t = workbook();
        put(&mut t, 200, 3, "data");
        let value = |row, col| Command::XlsxSetCellValue {
            sheet: 0,
            row,
            col,
            value: CellEntry::Number(1.0),
        };
        let formula = |row, col| Command::XlsxSetCellFormula {
            sheet: 0,
            row,
            col,
            formula: "=1+1".into(),
        };
        assert_eq!(value(200, 3).effect_in(&t), EffectClass::Destructive);
        assert_eq!(formula(200, 3).effect_in(&t), EffectClass::Destructive);
        assert_eq!(value(201, 3).effect_in(&t), EffectClass::Edit);
        assert_eq!(formula(201, 3).effect_in(&t), EffectClass::Edit);
        // A formula cell counts as filled too.
        t.apply(formula(201, 3)).expect("formula");
        assert_eq!(value(201, 3).effect_in(&t), EffectClass::Destructive);
    }

    #[test]
    fn paste_is_destructive_only_over_data() {
        let mut t = workbook();
        put(&mut t, 200, 3, "data");
        let paste = |row, col| Command::XlsxPaste {
            sheet: 0,
            row,
            col,
            rows: vec![vec!["a".into(), "b".into()], vec!["c".into(), "d".into()]],
        };
        assert_eq!(paste(199, 2).effect_in(&t), EffectClass::Destructive);
        assert_eq!(paste(300, 2).effect_in(&t), EffectClass::Edit);
    }

    #[test]
    fn clear_formats_is_destructive_only_where_formats_exist() {
        let mut t = workbook();
        let clear = |r| Command::ClearFormats { sheet: 0, range: r };
        assert_eq!(
            clear(range(400, 1, 401, 2)).effect_in(&t),
            EffectClass::Edit
        );
        let bold = Command::ApplyCellStyle {
            sheet: 0,
            range: range(400, 1, 400, 1),
            style: "Good".into(),
        };
        t.apply(bold).expect("style");
        assert_eq!(
            clear(range(400, 1, 401, 2)).effect_in(&t),
            EffectClass::Destructive
        );
        // A whole column or row may hold formats of its own.
        assert_eq!(
            clear(range(1, 1, open365_xlsx::MAX_ROW, 1)).effect_in(&t),
            EffectClass::Destructive
        );
    }

    #[test]
    fn merging_that_would_discard_values_is_destructive() {
        let mut t = workbook();
        put(&mut t, 500, 1, "keep");
        let merge = |mode, r| Command::MergeCells {
            sheet: 0,
            range: r,
            mode,
        };
        // Only the upper-left value: nothing is lost.
        assert_eq!(
            merge(MergeMode::Cells, range(500, 1, 500, 3)).effect_in(&t),
            EffectClass::Edit
        );
        put(&mut t, 500, 2, "lost");
        assert_eq!(
            merge(MergeMode::Cells, range(500, 1, 500, 3)).effect_in(&t),
            EffectClass::Destructive
        );
        assert_eq!(
            merge(MergeMode::Across, range(500, 1, 500, 3)).effect_in(&t),
            EffectClass::Destructive
        );
        // Unmerging never removes a value.
        assert_eq!(
            merge(MergeMode::Unmerge, range(500, 1, 500, 3)).effect_in(&t),
            EffectClass::Edit
        );
        // A range the scan cannot read fails closed.
        assert_eq!(
            merge(MergeMode::Cells, range(500, 1, 500, 99_999)).effect_in(&t),
            EffectClass::Destructive
        );
    }

    #[test]
    fn saving_over_an_existing_file_is_destructive() {
        let t = workbook();
        let own = t.path().expect("path").to_path_buf();
        let save = Command::Save { path: own };
        assert_eq!(save.effect_in(&t), EffectClass::Destructive);
        let gone = Command::Save {
            path: std::env::temp_dir().join("open365-no-such-file.xlsx"),
        };
        assert_eq!(gone.effect_in(&t), EffectClass::Export);
    }
}
