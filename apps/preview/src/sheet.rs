//! The Sheets preview's document: a workbook held by the command layer's [`Target`]. Every cell
//! edit is a [`Command`] (so it is audited, undoable and recalculated by the calc engine); this
//! type only adds the active cell, the scroll origin and the text a cell shows.

use std::path::{Path, PathBuf};

use open365_commands::{CellEntry, Command, Target, cell_display_text, cell_source_text};
use open365_xlsx::CellValue;

#[path = "../../sheets/src/blank.rs"]
mod blank;

/// Last row and column of a worksheet.
pub const MAX_ROW: u32 = 1_048_576;
pub const MAX_COL: u32 = 16_384;

/// The open workbook, the active cell and the first visible cell (1-based row, column).
pub struct SheetModel {
    target: Target,
    pub sheet: usize,
    pub active: (u32, u32),
    pub top_left: (u32, u32),
    pub status: String,
}

impl std::fmt::Debug for SheetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SheetModel")
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

impl SheetModel {
    /// An empty workbook with one sheet.
    pub fn blank() -> Self {
        let target = Target::open_xlsx(&blank::xlsx()).expect("the built-in blank workbook opens");
        Self::with_target(target)
    }

    /// Opens `.xlsx`, `.xlsm`, `.xltx` or `.xltm` from `path`.
    pub fn open(path: &Path) -> Result<Self, String> {
        Target::open_path(path)
            .map(Self::with_target)
            .map_err(|e| e.to_string())
    }

    fn with_target(target: Target) -> Self {
        Self {
            target,
            sheet: 0,
            active: (1, 1),
            top_left: (1, 1),
            status: String::new(),
        }
    }

    /// Writes the workbook to `path` as XLSX.
    pub fn save(&mut self, path: &Path) -> Result<(), String> {
        let bytes = self.target.snapshot_bytes().map_err(|e| e.to_string())?;
        std::fs::write(path, bytes).map_err(|e| e.to_string())
    }

    /// Where the workbook was opened from.
    pub fn path(&self) -> Option<PathBuf> {
        self.target.path().map(Path::to_path_buf)
    }

    pub fn is_dirty(&self) -> bool {
        self.target.is_dirty()
    }

    /// The cell as the formula bar shows it: `=formula` or the constant.
    pub fn source(&self, row: u32, col: u32) -> String {
        self.target
            .xlsx()
            .map(|wb| cell_source_text(wb, self.sheet, row, col))
            .unwrap_or_default()
    }

    /// The text the cell displays (value through its number format).
    pub fn display(&self, row: u32, col: u32) -> String {
        self.target
            .xlsx()
            .map(|wb| cell_display_text(wb, self.sheet, row, col))
            .unwrap_or_default()
    }

    /// Display text and whether it is a number (right aligned), `None` for an empty cell.
    pub fn shown(&self, row: u32, col: u32) -> Option<(String, bool)> {
        let wb = self.target.xlsx()?;
        let cell = wb.cell(self.sheet, row, col)?;
        if matches!(cell.value, CellValue::Empty) {
            return None;
        }
        let numeric = matches!(cell.value, CellValue::Number(_));
        Some((cell_display_text(wb, self.sheet, row, col), numeric))
    }

    /// Enters `text` into a cell as Excel does: `=` starts a formula, anything else is a number,
    /// a boolean or text. No command runs when the text equals the cell's current source.
    pub fn enter(&mut self, row: u32, col: u32, text: &str) -> Result<(), String> {
        if self.source(row, col) == text {
            return Ok(());
        }
        self.apply(Command::xlsx_enter(self.sheet, row, col, text))
    }

    /// Delete: empties the cell (its format stays).
    pub fn clear(&mut self, row: u32, col: u32) -> Result<(), String> {
        if self.source(row, col).is_empty() {
            return Ok(());
        }
        self.apply(Command::XlsxSetCellValue {
            sheet: self.sheet,
            row,
            col,
            value: CellEntry::Empty,
        })
    }

    fn apply(&mut self, command: Command) -> Result<(), String> {
        match self.target.apply(command) {
            Ok(_) => {
                self.status.clear();
                Ok(())
            }
            Err(e) => {
                self.status = e.to_string();
                Err(self.status.clone())
            }
        }
    }

    pub fn undo(&mut self) -> bool {
        self.target.undo().is_ok()
    }

    pub fn redo(&mut self) -> bool {
        self.target.redo().is_ok()
    }

    /// Moves the active cell by `(d_row, d_col)`, clamped to the sheet.
    pub fn move_by(&mut self, d_row: i64, d_col: i64) {
        let r = (i64::from(self.active.0) + d_row).clamp(1, i64::from(MAX_ROW)) as u32;
        let c = (i64::from(self.active.1) + d_col).clamp(1, i64::from(MAX_COL)) as u32;
        self.active = (r, c);
    }

    /// Scrolls just enough that the active cell lies in a window of `rows` by `cols` cells.
    pub fn reveal_active(&mut self, rows: u32, cols: u32) {
        let (r, c) = self.active;
        let (rows, cols) = (rows.max(1), cols.max(1));
        if r < self.top_left.0 {
            self.top_left.0 = r;
        } else if r >= self.top_left.0 + rows {
            self.top_left.0 = r + 1 - rows;
        }
        if c < self.top_left.1 {
            self.top_left.1 = c;
        } else if c >= self.top_left.1 + cols {
            self.top_left.1 = c + 1 - cols;
        }
    }
}

/// `A1`, `c12` or `AA100` as `(row, col)`.
pub fn parse_a1(s: &str) -> Option<(u32, u32)> {
    let s = s.trim().replace('$', "");
    let split = s.find(|c: char| c.is_ascii_digit())?;
    let (letters, digits) = s.split_at(split);
    if letters.is_empty() || !letters.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let col = letters
        .to_ascii_uppercase()
        .bytes()
        .fold(0u32, |a, b| a * 26 + u32::from(b - b'A' + 1));
    let row: u32 = digits.parse().ok()?;
    ((1..=MAX_ROW).contains(&row) && (1..=MAX_COL).contains(&col)).then_some((row, col))
}

/// `(1, 1)` as `A1`.
pub fn a1(row: u32, col: u32) -> String {
    let mut n = col;
    let mut letters = String::new();
    while n > 0 {
        n -= 1;
        letters.insert(0, char::from(b'A' + (n % 26) as u8));
        n /= 26;
    }
    format!("{letters}{row}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a1_names_round_trip() {
        for (name, rc) in [
            ("A1", (1, 1)),
            ("Z9", (9, 26)),
            ("AA100", (100, 27)),
            ("XFD1048576", (1_048_576, 16_384)),
        ] {
            assert_eq!(parse_a1(name), Some(rc), "{name}");
            assert_eq!(a1(rc.0, rc.1), name);
        }
        assert_eq!(parse_a1("c12"), Some((12, 3)));
        assert_eq!(parse_a1("XFE1"), None);
        assert_eq!(parse_a1("12"), None);
        assert_eq!(parse_a1("A0"), None);
    }

    #[test]
    fn typed_values_and_a_formula_recalculate_through_the_command_layer() {
        let mut m = SheetModel::blank();
        m.enter(1, 1, "2").unwrap();
        m.enter(1, 2, "3").unwrap();
        m.enter(1, 3, "=A1+B1").unwrap();
        assert_eq!(m.display(1, 3), "5");
        assert_eq!(m.source(1, 3), "=A1+B1");
        m.enter(1, 1, "10").unwrap();
        assert_eq!(m.display(1, 3), "13");
        m.clear(1, 1).unwrap();
        assert_eq!(m.display(1, 3), "3");
        assert!(m.undo());
        assert_eq!(m.display(1, 3), "13");
    }
}
